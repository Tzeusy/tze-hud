#!/usr/bin/env bash
# Reproduce the live OS-injection whole-portal resize evidence run
# (§6b.7 live phase from hud-5jbra.9 / bead hud-egn13).
#
# Prereqs (never committed): a reachable autonomous hud-windows VM
# (windows-vm.example) with the current-main tze_hud.exe deployed, the resident
# gRPC port reachable, and SSH access for the OS-input injector. Resolve host +
# PSK via the user-test VM env helper (it self-heals the VM if needed):
#
#   eval "$(<repo>/.claude/skills/user-test/scripts/hud_vm_env.sh)"
#   export TZE_HUD_PSK="$HUD_MCP_PSK"                 # resident gRPC PSK == MCP PSK
#   export TZE_HUD_GRPC_TARGET="$TZE_HUD_TEST_HOST:50051"
#   export TZE_HUD_TEST_HOST                          # for the OS-injection SSH path
#
# Then, from this directory:
#   ./run.sh
#
# The driver drives the resident-gRPC first-class PortalSurface, injects real OS
# pointer + keyboard events via the interactive scheduled-task path (with the
# injector console HIDDEN so events reach the HUD overlay instead of the
# injector's own window), and writes snapshots/, logs/timeline.json, and
# logs/verdicts_computed.json. No screenshots are captured (see VERDICTS.md
# §caveat — GDI capture does not composite the transparent Vulkan overlay on the
# software-GPU VM; the authoritative evidence is the gRPC SceneSnapshot bounds).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

: "${TZE_HUD_GRPC_TARGET:?set TZE_HUD_GRPC_TARGET=<vm-ip>:50051}"
: "${TZE_HUD_PSK:?set TZE_HUD_PSK to the resident gRPC PSK}"
: "${TZE_HUD_TEST_HOST:?set TZE_HUD_TEST_HOST for the OS-injection SSH path}"

# agent-id defaults to agent-alpha in the driver — the config-registered agent
# granted tile capabilities on the deployed host. Override via AGENT_ID only if
# the target host registers a different agent.
python3 "$HERE/resize_injection_driver.py" \
  --target "$TZE_HUD_GRPC_TARGET" \
  --agent-id "${AGENT_ID:-agent-alpha}" \
  --tab-width "${SCENE_W:-1280}" --tab-height "${SCENE_H:-800}" \
  --win-host "$TZE_HUD_TEST_HOST" \
  "$@"
