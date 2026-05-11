#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPLAY_SCRIPT="$SCRIPT_DIR/live-replay.sh"

WIN_HOST="${WIN_HOST:-tzehouse-windows.parrot-hen.ts.net}"
MAX_POLLS="${MAX_POLLS:-30}"
POLL_INTERVAL_S="${POLL_INTERVAL_S:-60}"
PROBE_TIMEOUT_S="${PROBE_TIMEOUT_S:-12}"
LOG_DIR="${LOG_DIR:-$SCRIPT_DIR}"

log() {
  printf '[external-agent-projection-authority-watch] %s\n' "$*"
}

if ! [[ "$MAX_POLLS" =~ ^[0-9]+$ ]] || [[ "$MAX_POLLS" -lt 1 ]]; then
  log "BLOCKED: MAX_POLLS must be a positive integer"
  exit 2
fi

if ! [[ "$POLL_INTERVAL_S" =~ ^[0-9]+$ ]]; then
  log "BLOCKED: POLL_INTERVAL_S must be a non-negative integer"
  exit 2
fi

mkdir -p "$LOG_DIR"

for poll in $(seq 1 "$MAX_POLLS"); do
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  log "poll $poll/$MAX_POLLS at $stamp for $WIN_HOST"
  tailscale status --json \
    | jq -r '.Peer[] | select(.DNSName=="'"$WIN_HOST"'.") | {Online,LastSeen,TailscaleIPs,DNSName}' \
    || true

  if timeout "${PROBE_TIMEOUT_S}s" tailscale ping --c 1 "$WIN_HOST"; then
    log "reachable; running live replay"
    replay_log="$LOG_DIR/live-replay-${stamp}.log"
    set +e
    "$REPLAY_SCRIPT" 2>&1 | tee "$replay_log"
    replay_code="${PIPESTATUS[0]}"
    set -e
    log "live replay exit_code=$replay_code log=$replay_log"
    exit "$replay_code"
  fi

  log "not reachable"
  if [[ "$poll" -lt "$MAX_POLLS" ]]; then
    sleep "$POLL_INTERVAL_S"
  fi
done

log "BLOCKED: $WIN_HOST did not become reachable after $MAX_POLLS polls"
exit 20
