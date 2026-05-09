## Why

The project has accumulated multi-device, multi-runtime, and media-plane scope ahead of the single-device runtime being honed to its full performance ceiling. The v2 program (`openspec/changes/_deferred/v2-embodied-media-presence/`) was a 12–17 month commitment to mobile-first, glasses-ready, embodied-presence, WebRTC, GStreamer, recording, cloud-relay, agent-to-agent media — all spawned before the Windows runtime had been pushed to its limits.

The decision (2026-05-09) is to invert that order. **Build the most performant Rust HUD runtime humanly possible on Windows first**, then port. Multi-device design returns only as a port from a known-good single-device baseline, not as a forecast.

This change records the refocus, sets the new active scope boundary, and supersedes the multi-device promises in `v1.md`, `v2.md`, `mobile.md`, and `media-doctrine.md` (each of which carries a top-of-file deferral block as of this date).

## What Changes

- **Platform scope tightens to Windows only** as the first-class shipping target. macOS and Linux desktop remain compile/CI correctness targets but are no longer deployment lanes; mobile and glasses are deferred indefinitely (`mobile.md`).
- **Media plane is removed from active scope.** WebRTC, GStreamer integration, bounded media ingress, recording, cloud-relay, bidirectional AV, voice synthesis, agent-to-agent media — all parked. The associated specs (`media-webrtc-bounded-ingress`, `media-webrtc-privacy-operator-policy`) and doctrine (`media-doctrine.md`) carry deferral markers.
- **Embodied presence is removed from active scope.** Guest and resident only; the third presence level is parked along with `identity-and-roles` and `identity-portability` proposals.
- **The 44 `hud-ora8.*` v2 beads were closed** with reason "scope creep past single-device Windows; deferred indefinitely." Tracking parent: `hud-9wljr`.
- **A concrete Windows-runtime performance bar is established** (see `design.md` §1) with measured budgets for frame time, input latency, scene-commit latency, widget rasterization, transparent-overlay composite cost, and resource utilization at idle and under multi-agent load.
- **Cooperative HUD projection** (`hud-ggntn.*`) stays in scope: it is single-device and partially shipped. Outstanding beads (`hud-ggntn.7/.10/.11`) remain open.
- **Cross-machine validation** (`cross-machine-runtime-validation` spec) is reduced to "the existing Linux-builds-Windows automation we currently use" with no new requirements; deferral block added.

## Capabilities

### New Capabilities

- None *yet*. This change is a strategic refocus + performance-bar reset. Capability-level deltas (e.g., a `windows-runtime-performance-budget` capability or amendments to `runtime-kernel` and `validation-framework`) will be drafted as concrete work proceeds and recorded in this change's `specs/` folder before archive.

### Modified Capabilities

- `runtime-kernel`, `runtime-app-binary`, `validation-framework`, `widget-system`, `scene-graph`: candidates for tightening once the performance bar is exercised against current behavior. Specific deltas TBD.

## Impact

- **Doctrine surface narrowed.** `mobile.md`, `v2.md`, `media-doctrine.md` are marked deferred. `v1.md` replaces its V2 Program section with a Single-Windows Refocus block and converts `[superseded by V2: <phase>]` markers to `[deferred indefinitely]`.
- **Spec surface narrowed.** Three specs carry deferral blocks: `media-webrtc-bounded-ingress`, `media-webrtc-privacy-operator-policy`, `cross-machine-runtime-validation`. Other specs (`scene-graph`, `lease-governance`, `widget-system`, etc.) remain active and the source of truth for current implementation work.
- **Bead surface narrowed.** v2 closed (44 beads); cooperative-projection retained (3 beads); new beads will spawn from this change under `hud-9wljr`.
- **Implementation surface unchanged.** No code is reverted. The shipped projection adapter, scene graph, compositor, gRPC + MCP planes, lease system, widget pipeline, and Windows transparent-overlay path all remain. The refocus is about what is admitted as *next* work, not about what already shipped.
- **Reversibility.** Doctrine and specs are marked, not deleted. The deferred openspec change directory is preserved at `openspec/changes/_deferred/`. Any future multi-device program can build from these starting points without re-discovery.
