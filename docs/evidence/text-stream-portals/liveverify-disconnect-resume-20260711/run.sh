#!/usr/bin/env bash
# Reproduce the live disconnect->stale->reconnect->resume evidence run
# (portal-disconnect-resume-ux §5.1 / hud-om69w).
#
# Prereqs (never committed): a reachable autonomous hud-windows VM with the
# current-main tze_hud.exe deployed, and the resident gRPC port tunnelled/reachable.
# Resolve host + PSK via the user-test VM env helper:
#
#   eval "$(<repo>/.claude/skills/user-test/scripts/hud_vm_env.sh)"
#   export TZE_HUD_PSK="$HUD_MCP_PSK"                 # resident gRPC PSK == MCP PSK
#   export TZE_HUD_GRPC_TARGET="$TZE_HUD_TEST_HOST:50051"
#   export TZE_HUD_TEST_HOST                          # for the screenshot SSH path
#
# Then, from this directory:
#   ./run.sh
#
# The driver writes snapshots/, logs/timeline.json, and (with --screenshots)
# full-desktop PNGs that MUST be cropped by crop_portal_region.py before commit.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

: "${TZE_HUD_GRPC_TARGET:?set TZE_HUD_GRPC_TARGET=<vm-ip>:50051}"
: "${TZE_HUD_PSK:?set TZE_HUD_PSK to the resident gRPC PSK}"

python3 "$HERE/disconnect_resume_driver.py" \
  --target "$TZE_HUD_GRPC_TARGET" \
  --agent-id liveverify-disc-resume \
  --tab-width "${SCENE_W:-1280}" --tab-height "${SCENE_H:-800}" \
  --detect-wait-s "${DETECT_WAIT_S:-20}" \
  --settle-s "${SETTLE_S:-3}" \
  ${SCREENSHOTS:+--screenshots --win-host "$TZE_HUD_TEST_HOST"} \
  "$@"

if [[ -n "${SCREENSHOTS:-}" ]]; then
  echo "cropping full-desktop captures to the portal region…"
  python3 "$HERE/crop_portal_region.py"
fi
