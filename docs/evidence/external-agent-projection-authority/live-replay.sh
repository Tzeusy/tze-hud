#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

WIN_HOST="${WIN_HOST:-tzehouse-windows.parrot-hen.ts.net}"
HUD_USER="${HUD_USER:-hudbot}"
CONTROL_USER="${CONTROL_USER:-tzeus}"
SSH_KEY="${SSH_KEY:-$HOME/.ssh/ecdsa_home}"
TASK_NAME="${TASK_NAME:-TzeHudOverlay}"
HUD_EXE="${HUD_EXE:-C:\\tze_hud\\tze_hud.exe}"
HUD_CONFIG="${HUD_CONFIG:-C:\\tze_hud\\tze_hud.toml}"
MCP_HTTP_URL="${MCP_HTTP_URL:-http://$WIN_HOST:9090/mcp}"
GRPC_TARGET="${GRPC_TARGET:-$WIN_HOST:50051}"
PSK_ENV="${PSK_ENV:-TZE_HUD_PSK}"
PROBE_TIMEOUT_S="${PROBE_TIMEOUT_S:-12}"
SSH_CONNECT_TIMEOUT_S="${SSH_CONNECT_TIMEOUT_S:-8}"
STARTUP_WAIT_S="${STARTUP_WAIT_S:-5}"
RECREATE_TASK_ON_START="${RECREATE_TASK_ON_START:-0}"
TASK_START_TIME="${TASK_START_TIME:-23:59}"

ZONE_MESSAGES="$SCRIPT_DIR/replay-zone-messages.json"
WIDGET_MESSAGES="$SCRIPT_DIR/replay-widget-messages.json"
PORTAL_TRANSCRIPT="$SCRIPT_DIR/live-portal-transcript.json"

log() {
  printf '[external-agent-projection-authority] %s\n' "$*"
}

fail() {
  local code="$1"
  shift
  log "BLOCKED: $*"
  exit "$code"
}

ssh_probe() {
  local user="$1"
  timeout "${PROBE_TIMEOUT_S}s" \
    ssh -o BatchMode=yes \
      -o IdentitiesOnly=yes \
      -o ConnectTimeout="${SSH_CONNECT_TIMEOUT_S}" \
      -i "$SSH_KEY" \
      "$user@$WIN_HOST" \
      whoami
}

tcp_probe() {
  local port="$1"
  timeout "${PROBE_TIMEOUT_S}s" bash -lc "exec 3<>/dev/tcp/$WIN_HOST/$port"
}

run_control_ssh() {
  local command="$1"
  timeout "${PROBE_TIMEOUT_S}s" \
    ssh -o BatchMode=yes \
      -o IdentitiesOnly=yes \
      -o ConnectTimeout="${SSH_CONNECT_TIMEOUT_S}" \
      -i "$SSH_KEY" \
      "$CONTROL_USER@$WIN_HOST" \
      "$command"
}

run_control_powershell_stdin() {
  timeout "${PROBE_TIMEOUT_S}s" \
    ssh -o BatchMode=yes \
      -o IdentitiesOnly=yes \
      -o ConnectTimeout="${SSH_CONNECT_TIMEOUT_S}" \
      -i "$SSH_KEY" \
      "$CONTROL_USER@$WIN_HOST" \
      powershell -NoProfile -NonInteractive -Command -
}

ps_single_quote() {
  local value="${1//\'/\'\'}"
  printf "'%s'" "$value"
}

resolve_psk_env() {
  if [[ -z "${!PSK_ENV:-}" ]]; then
    if [[ "$PSK_ENV" != "MCP_TEST_PSK" && -n "${MCP_TEST_PSK:-}" ]]; then
      export "$PSK_ENV=${MCP_TEST_PSK}"
    else
      fail 13 "required PSK environment variable $PSK_ENV is not set; MCP_TEST_PSK may be used as a fallback"
    fi
  fi
  export MCP_TEST_PSK="${!PSK_ENV}"
}

register_overlay_task_with_psk() {
  local psk_value="${!PSK_ENV}"
  local psk_literal
  local exe_literal
  local config_literal
  local task_literal
  local start_time_literal
  psk_literal="$(ps_single_quote "$psk_value")"
  exe_literal="$(ps_single_quote "$HUD_EXE")"
  config_literal="$(ps_single_quote "$HUD_CONFIG")"
  task_literal="$(ps_single_quote "$TASK_NAME")"
  start_time_literal="$(ps_single_quote "$TASK_START_TIME")"

  {
    printf '$taskName = %s\n' "$task_literal"
    printf '$exe = %s\n' "$exe_literal"
    printf '$config = %s\n' "$config_literal"
    printf '$psk = %s\n' "$psk_literal"
    printf '$startTime = %s\n' "$start_time_literal"
    cat <<'POWERSHELL'
$taskRun = "`"$exe`" --config `"$config`" --window-mode overlay --grpc-port 50051 --mcp-port 9090 --psk `"$psk`""
& schtasks /Create /F /TN $taskName /SC ONCE /ST $startTime /IT /RL HIGHEST /TR $taskRun | Out-Null
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}
POWERSHELL
  } | run_control_powershell_stdin
}

cd "$REPO_ROOT"

log "checking Tailscale reachability for $WIN_HOST"
timeout "${PROBE_TIMEOUT_S}s" tailscale ping --c 1 "$WIN_HOST" \
  || fail 10 "tailscale ping failed for $WIN_HOST"

log "checking SSH for $HUD_USER and $CONTROL_USER"
ssh_probe "$HUD_USER" >/dev/null \
  || fail 11 "SSH failed for $HUD_USER@$WIN_HOST with $SSH_KEY"
ssh_probe "$CONTROL_USER" >/dev/null \
  || fail 11 "SSH failed for $CONTROL_USER@$WIN_HOST with $SSH_KEY"

resolve_psk_env

if ! tcp_probe 9090 >/dev/null 2>&1 || ! tcp_probe 50051 >/dev/null 2>&1; then
  log "MCP/gRPC ports are not both reachable; starting $TASK_NAME"
  if [[ "$RECREATE_TASK_ON_START" == "1" ]]; then
    log "recreating $TASK_NAME with non-default PSK from $PSK_ENV before launch"
    register_overlay_task_with_psk \
      || fail 12 "failed to register scheduled task $TASK_NAME"
  fi
  run_control_ssh "schtasks /Run /TN $TASK_NAME" \
    || fail 12 "failed to start scheduled task $TASK_NAME"
  sleep "$STARTUP_WAIT_S"
fi

log "checking MCP :9090 and gRPC :50051"
tcp_probe 9090 >/dev/null \
  || fail 12 "MCP port 9090 is not reachable after $TASK_NAME start attempt"
tcp_probe 50051 >/dev/null \
  || fail 12 "gRPC port 50051 is not reachable after $TASK_NAME start attempt"

log "publishing zone replay through $MCP_HTTP_URL"
python3 .claude/skills/user-test/scripts/publish_zone_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file "$ZONE_MESSAGES" \
  --list-zones

log "publishing widget replay through $MCP_HTTP_URL"
python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url "$MCP_HTTP_URL" \
  --psk-env MCP_TEST_PSK \
  --messages-file "$WIDGET_MESSAGES" \
  --list-widgets \
  --cleanup-on-exit

log "running portal composer smoke through $GRPC_TARGET"
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target "$GRPC_TARGET" \
  --psk-env "$PSK_ENV" \
  --agent-id projection:agent-question \
  --phases composer-smoke \
  --transcript-out "$PORTAL_TRANSCRIPT"

log "live replay complete; portal transcript: $PORTAL_TRANSCRIPT"
