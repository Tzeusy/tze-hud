#!/usr/bin/env bash
# Reproduce the live resident-gRPC bridge-routing evidence run (hud-rw8eo).
#
# Prereqs (never committed): a reachable autonomous hud-windows VM, admin SSH,
# and a current-main tze_hud.exe built for x86_64-pc-windows-gnu.
#
# 1) Build the exe:
#      cargo build --release --target x86_64-pc-windows-gnu -p tze_hud_app
#
# 2) Resolve host + PSK (self-heals the VM); capture ONCE to avoid the env
#    helper's self-heal re-launching the DEFAULT fullscreen task under you:
#      .claude/skills/user-test/scripts/hud_vm_env.sh > /tmp/bridge-env.sh
#      source /tmp/bridge-env.sh   # exports TZE_HUD_TEST_HOST, HUD_MCP_URL, HUD_MCP_PSK
#
# 3) On the host, the runtime MUST run with the bridge ENABLED and the bridge
#    agent registered. The cleanest non-invasive path (used here):
#      - write a bridge config = canonical tze_hud.toml + a
#        `[agents.registered.resident-grpc-portal]` grant
#        (capabilities: create_tiles, modify_own_tiles, access_input_events);
#      - launch tze_hud.exe EXE-DIRECT (interactive Scheduled Task, NOT a
#        cmd/powershell wrapper — a wrapper runs non-interactively and cannot
#        create a Vulkan surface) with:
#          --window-mode fullscreen --config <bridge.toml> --bind-all-interfaces --resident-grpc-portal
#        Reusing the known-good interactive TzeHudFullscreen task's principal
#        (userid=<admin-user>, logontype=Interactive, runlevel=Highest) is the
#        reliable way to get an interactive, rendering instance over SSH.
#      - restore the default (bridge-disabled) config + args afterward.
#
# 4) Run the driver (attach/publish/snapshot/detach over MCP + a gRPC observer):
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
: "${TZE_HUD_TEST_HOST:?source /tmp/bridge-env.sh first}"
: "${HUD_MCP_URL:?}"
: "${HUD_MCP_PSK:?}"

python3 "$HERE/bridge_routing_driver.py" \
  --host "$TZE_HUD_TEST_HOST" \
  --mcp-url "$HUD_MCP_URL" \
  --psk "$HUD_MCP_PSK" \
  --agent-id "${AGENT_ID:-agent-alpha}" \
  --projection-id "${PROJECTION_ID:-bridge-routing-live}" \
  "$@"

# For the in-process A/B control, restore the default bridge-disabled config,
# restart the runtime, and re-run an attach — the portal tile namespace will be
# `tze_hud_portal_driver` instead of `resident-grpc-portal`.
