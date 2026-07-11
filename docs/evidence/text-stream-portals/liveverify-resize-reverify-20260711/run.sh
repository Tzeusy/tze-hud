#!/usr/bin/env bash
# Reproduce the live OS-injection whole-portal resize RE-VERIFY run against a
# build containing #1129 (fix: whole-portal resize wired on the first-class
# surface path). This REUSES the proven hud-egn13 harness verbatim (the driver
# here is an identical copy of
# ../liveverify-resize-injection-20260711/resize_injection_driver.py, including
# the H1 hidden-console injection fix); only --outdir differs.
#
# Prereqs (never committed): a reachable autonomous hud-windows VM with the
# fresh tze_hud.exe deployed in DEFAULT config (no TZE_HUD_RESIDENT_GRPC_PORTAL
# bridge flag), the resident gRPC port reachable, and SSH access for the
# OS-input injector. Resolve host + PSK via the user-test VM env helper:
#
#   eval "$(<repo>/.claude/skills/user-test/scripts/hud_vm_env.sh)"
#   export TZE_HUD_PSK="$HUD_MCP_PSK"
#   export TZE_HUD_GRPC_TARGET="$TZE_HUD_TEST_HOST:50051"
#
# Then, from this directory:
#   ./run.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

: "${TZE_HUD_GRPC_TARGET:?set TZE_HUD_GRPC_TARGET=<vm-ip>:50051}"
: "${TZE_HUD_PSK:?set TZE_HUD_PSK to the resident gRPC PSK}"
: "${TZE_HUD_TEST_HOST:?set TZE_HUD_TEST_HOST for the OS-injection SSH path}"

python3 "$HERE/resize_injection_driver.py" \
  --target "$TZE_HUD_GRPC_TARGET" \
  --agent-id "${AGENT_ID:-agent-alpha}" \
  --tab-width "${SCENE_W:-1280}" --tab-height "${SCENE_H:-800}" \
  --win-host "$TZE_HUD_TEST_HOST" \
  --admin-user "${ADMIN_USER:-admin-user}" \
  --outdir "$HERE" \
  "$@"
