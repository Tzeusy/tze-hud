#!/usr/bin/env bash
# quickstart.sh — one-command first-run bootstrap for the tze_hud text-stream portal.
#
# Goal: get a fresh user from "I cloned the repo" to "a Claude/Codex session is
# projecting onto my screen" with a single command and zero tribal knowledge.
#
# What it does (all idempotent):
#   1. Locates (or, with --build, builds) the canonical `tze_hud` binary.
#   2. Generates a minimal, valid `tze_hud.toml` if none exists (portal-primary:
#      just [runtime] + a default [[tabs]] — the portal renders into the Main tab
#      with the runtime's built-in zero-config placement/size/token defaults).
#   3. Generates a strong random PSK if none is set, and persists it to
#      `tze_hud.psk` (chmod 600) so re-runs are stable.
#   4. Prints the ATTACH INFO block: the MCP endpoint URL, the resident-principal
#      rule, and a ready-to-paste MCP `settings.json` snippet — the discovery
#      surface the runtime does not (yet) print on stdout itself.
#   5. Launches the runtime (unless --print-attach-info / --no-launch).
#
# Doctrine: cooperative opt-in projection; the screen-sovereign runtime owns the
# pixels. This script only wires up config + credentials + discovery; the LLM
# session still explicitly opts in via the `hud-projection` skill.
#
# Usage:
#   scripts/quickstart.sh                      # scaffold + launch (fullscreen)
#   scripts/quickstart.sh --window-mode overlay
#   scripts/quickstart.sh --print-attach-info  # scaffold + print attach block, do NOT launch
#   scripts/quickstart.sh --build              # cargo build --bin tze_hud --release first
#
# Options:
#   --config <path>         Config file to use/create   (default: ./tze_hud.toml)
#   --psk <key>             Use this PSK instead of generating/reading tze_hud.psk
#   --psk-file <path>       Where to persist the generated PSK (default: ./tze_hud.psk)
#   --window-mode <mode>    fullscreen | overlay         (default: fullscreen)
#   --mcp-port <port>       MCP HTTP listen port          (default: 9090)
#   --grpc-port <port>      gRPC listen port; 0 disables  (default: 50051)
#   --host <host>           Host shown in the attach URL  (default: 127.0.0.1)
#   --bin <path>            Explicit tze_hud binary path
#   --build                 Build the binary before launching
#   --print-attach-info     Scaffold + print attach block, then exit (no launch)
#   --no-launch             Alias for --print-attach-info
#   -h, --help              Show this help
#
# Exit codes: 0 ok · 1 usage · 2 binary not found · 3 build failed

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

CONFIG_PATH="tze_hud.toml"
PSK=""
PSK_FILE="tze_hud.psk"
WINDOW_MODE="fullscreen"
MCP_PORT="9090"
GRPC_PORT="50051"
HOST="127.0.0.1"
BIN_PATH=""
DO_BUILD=0
LAUNCH=1

usage() { sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --config)           CONFIG_PATH="${2:?--config requires a path}";       shift 2 ;;
    --psk)              PSK="${2:?--psk requires a key}";                    shift 2 ;;
    --psk-file)         PSK_FILE="${2:?--psk-file requires a path}";         shift 2 ;;
    --window-mode)      WINDOW_MODE="${2:?--window-mode requires a value}";  shift 2 ;;
    --mcp-port)         MCP_PORT="${2:?--mcp-port requires a value}";        shift 2 ;;
    --grpc-port)        GRPC_PORT="${2:?--grpc-port requires a value}";      shift 2 ;;
    --host)             HOST="${2:?--host requires a value}";                shift 2 ;;
    --bin)              BIN_PATH="${2:?--bin requires a path}";              shift 2 ;;
    --build)            DO_BUILD=1;                                          shift ;;
    --print-attach-info|--no-launch) LAUNCH=0;                              shift ;;
    -h|--help)          usage; exit 0 ;;
    *) echo "quickstart: unknown argument: $1 (see --help)" >&2; exit 1 ;;
  esac
done

info() { printf '\033[1;36m[quickstart]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[quickstart]\033[0m %s\n' "$*" >&2; }

# ── 1. Locate or build the binary ─────────────────────────────────────────────
if [[ -z "$BIN_PATH" ]]; then
  for candidate in \
    "${REPO_ROOT}/target/release/tze_hud" \
    "${REPO_ROOT}/target/debug/tze_hud"; do
    if [[ -x "$candidate" ]]; then BIN_PATH="$candidate"; break; fi
  done
fi

if [[ "$DO_BUILD" == "1" || -z "$BIN_PATH" ]]; then
  if [[ "$DO_BUILD" == "1" ]]; then
    info "Building canonical binary: cargo build --bin tze_hud --release"
    ( cd "$REPO_ROOT" && cargo build --bin tze_hud --release ) || { echo "quickstart: build failed" >&2; exit 3; }
    BIN_PATH="${REPO_ROOT}/target/release/tze_hud"
  else
    warn "No prebuilt tze_hud binary found under target/{release,debug}/."
    warn "Build it first:  cargo build --bin tze_hud --release"
    warn "or re-run:       scripts/quickstart.sh --build"
    exit 2
  fi
fi
if [[ ! -x "$BIN_PATH" ]]; then
  warn "tze_hud binary not found or not executable: ${BIN_PATH}"
  warn "Build it:  cargo build --bin tze_hud --release   (or pass a valid --bin <path>)"
  exit 2
fi
info "Binary: ${BIN_PATH}"

# ── 2. Scaffold a minimal, portal-primary config if absent ────────────────────
if [[ ! -f "$CONFIG_PATH" ]]; then
  info "Writing minimal portal-primary config: ${CONFIG_PATH}"
  cat > "$CONFIG_PATH" <<'TOML'
# tze_hud — minimal portal-primary config (generated by scripts/quickstart.sh)
#
# This is the smallest valid config: [runtime] + one default [[tabs]].
# A text-stream portal renders into the Main tab using the runtime's built-in
# zero-config placement, size, and design-token defaults — no widget wiring
# needed for LLM-session projection. Add [[tabs.widgets]] later if you also
# want gauges/status widgets (see app/tze_hud_app/config/production.toml).

[runtime]
profile = "full-display"

[[tabs]]
name        = "Main"
default_tab = true
TOML
else
  info "Using existing config: ${CONFIG_PATH}"
fi

# ── 3. Resolve / generate the PSK ─────────────────────────────────────────────
gen_psk() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 24
  else
    # Fallback: 48 hex chars from the kernel CSPRNG.
    LC_ALL=C tr -dc 'a-f0-9' < /dev/urandom | head -c 48
  fi
}

if [[ -z "$PSK" ]]; then
  if [[ -n "${TZE_HUD_PSK:-}" ]]; then
    PSK="$TZE_HUD_PSK"
    info "Using PSK from TZE_HUD_PSK environment variable."
  elif [[ -f "$PSK_FILE" ]]; then
    PSK="$(tr -d '[:space:]' < "$PSK_FILE")"
    info "Using PSK from ${PSK_FILE}."
  else
    PSK="$(gen_psk)"
    ( umask 077; printf '%s\n' "$PSK" > "$PSK_FILE" )
    info "Generated a strong PSK and stored it (chmod 600) in ${PSK_FILE}."
  fi
fi

if [[ "$PSK" == "tze-hud-key" ]]; then
  warn "PSK is the trivial default 'tze-hud-key' — strict startup will reject it."
  warn "Pass --psk <strong-key> or delete tze_hud.psk to regenerate."
fi

# The resident principal MUST equal the PSK: the runtime mints the resident_mcp
# capability (which reaches the portal_projection_* tools) only for a caller whose
# bearer matches BOTH the configured principal AND the PSK (constant-time).
export TZE_HUD_PSK="$PSK"
export TZE_HUD_MCP_RESIDENT_PRINCIPAL="$PSK"

MCP_URL="http://${HOST}:${MCP_PORT}/mcp"

# ── 4. Print the ATTACH INFO discovery block ──────────────────────────────────
cat <<BANNER

────────────────────────────────────────────────────────────────────────────
 tze_hud — ATTACH INFO  (point your LLM session's MCP client here)
────────────────────────────────────────────────────────────────────────────
 MCP endpoint : ${MCP_URL}
 Auth bearer  : <your PSK>   (stored in ${PSK_FILE})
 Resident env : TZE_HUD_MCP_RESIDENT_PRINCIPAL = <your PSK>   (already exported)

 The runtime grants the portal_projection_* tools only to a caller that presents
 the PSK as BOTH the resident principal and the MCP bearer. This script already
 exported TZE_HUD_PSK and TZE_HUD_MCP_RESIDENT_PRINCIPAL for the launch below.

 MCP client config (e.g. .mcp.json / settings.json), with your PSK substituted:

   {
     "mcpServers": {
       "tze-hud-runtime": {
         "type": "url",
         "url": "${MCP_URL}",
         "headers": { "Authorization": "Bearer <your PSK>" }
       }
     }
   }

 Then, in the LLM session, invoke the hud-projection skill and 'attach' — see
 docs/QUICKSTART.md for the copy-paste attach walkthrough.
────────────────────────────────────────────────────────────────────────────

BANNER

if [[ "$LAUNCH" == "0" ]]; then
  info "--print-attach-info set; not launching. Start the runtime yourself with:"
  echo "  TZE_HUD_PSK=<psk> TZE_HUD_MCP_RESIDENT_PRINCIPAL=<psk> \\"
  echo "    ${BIN_PATH} --config ${CONFIG_PATH} --window-mode ${WINDOW_MODE} --mcp-port ${MCP_PORT} --grpc-port ${GRPC_PORT}"
  exit 0
fi

# ── 5. Launch the runtime ─────────────────────────────────────────────────────
info "Launching runtime (Ctrl-C to stop)…"
exec "$BIN_PATH" \
  --config "$CONFIG_PATH" \
  --window-mode "$WINDOW_MODE" \
  --mcp-port "$MCP_PORT" \
  --grpc-port "$GRPC_PORT" \
  --psk "$PSK"
