# liveverify — resident-gRPC bridge routing (2026-07-11, hud-rw8eo)

Live on-device evidence that the **hud-hfuxy / PR #1046** production wiring routes MCP
`portal_projection_*` projections onto the **resident-gRPC bridge**
(`PortalTransport::ResidentGrpcBridge`) when the runtime runs with
`--resident-grpc-portal` (== `TZE_HUD_RESIDENT_GRPC_PORTAL=1`), instead of the in-process
direct-scene arm — run against the autonomous `windows-vm.example` HUD testhost on a fresh
current-`main` (`cae4b302`) binary.

**Headline result:** with the bridge enabled, a projection attached through the ordinary MCP
facade is materialised by the bridge — its portal tile is owned by agent `resident-grpc-portal`
— whereas the identical attach with the bridge disabled is materialised in-process under
`tze_hud_portal_driver`. Streaming and clean detach/teardown work over the bridged transport.
See **`VERDICTS.md`** for the per-check table, the A/B control, and caveats.

## Contents

| Path | What |
|------|------|
| `VERDICTS.md` | Per-check PASS/PARTIAL/N-A verdicts, the enabled-bridge log proof, and the in-process↔bridged namespace A/B |
| `bridge_routing_driver.py` | The evidence harness (MCP `portal_projection_*` + a gRPC observer snapshot; extracts the portal tile's owning namespace) |
| `run.sh` | Reproduce the run |
| `logs/bridge-startup.log` | Runtime startup log showing the bridge **enabling + connecting** (`resident gRPC portal bridge connected`) |
| `logs/exe.sha256` | sha256 of the deployed binary (current-`main` build) |
| `logs/timeline.json` | Machine timeline of every phase (attach, publishes, snapshots, paste, detach) |
| `logs/verdicts.json` | Machine-readable per-check verdicts |
| `snapshots/00-control-in-process.json` | **Control** (bridge OFF): portal tile `namespace = tze_hud_portal_driver` |
| `snapshots/01-bridged-baseline.json` | Bridge ON, after attach+publish: portal tile `namespace = resident-grpc-portal` |
| `snapshots/02-bridged-streamed.json` | After 4 streamed transcript units — still bridged, surface `Active` |
| `snapshots/03-bridged-unread-bursts.json` | After rapid publishes (unread-count plumbing exercised) |
| `snapshots/99-detached-clean.json` | After detach — `0` tiles, `0` portal_surfaces (clean teardown) |

**No screenshots are committed.** Visual capture is unreliable on this software-GPU Proxmox VM
(GDI capture does not composite the transparent Vulkan overlay). The authoritative parity source
is the `SceneSnapshot.tiles[].namespace` tile owner captured here, cross-checked against an A/B
control.

## How routing is proven

The wire deliberately excludes the transport (WM-S2b snapshot exclusion) and runtime tracing is
stdout-only (discarded by the Scheduled-Task deployment). But the bridge materialises the portal
tile from its OWN loopback `HudSession` as agent `resident-grpc-portal`, so the tile's
`namespace` reads back the materialiser. In-process rendering paints under
`tze_hud_portal_driver` and can never produce a `resident-grpc-portal` tile — so the namespace is
an unambiguous routing fingerprint, and the A/B control (`00` vs `01`) isolates the bridge flag
as the sole cause of the flip.

## Hygiene

Placeholders only: `windows-vm.example`, `proxmox-host.example`, `hud-user`/`admin-user`,
`agent-alpha`/`resident-grpc-portal`. No PSK, no real IPs/hostnames (loopback `127.0.0.1` is the
bridge self-target, intentional), no owner tokens, no full-desktop frames. Host restored to its
default bridge-disabled config; helper files/tasks removed; scene left empty.
