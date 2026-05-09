# Tasks — Windows-First Performant Runtime

Each task below corresponds to one or more `bd` beads under epic `hud-9wljr`. Bead spawning happens after this change is reviewed and acknowledged; this file is the source of truth for what gets spawned.

## 1. Refocus bookkeeping (this session)

- [x] 1.1 Close all 44 `hud-ora8.*` v2 beads with deferral reason
- [x] 1.2 Move `openspec/changes/v2-embodied-media-presence/` to `openspec/changes/_deferred/`
- [x] 1.3 Add deferral block to `v2.md`, `mobile.md`, `media-doctrine.md`
- [x] 1.4 Update `v1.md` — replace V2 Program section with Single-Windows Refocus, convert `[superseded by V2: <phase>]` markers to `[deferred indefinitely]`
- [x] 1.5 Add deferral block to `media-webrtc-bounded-ingress`, `media-webrtc-privacy-operator-policy`, `cross-machine-runtime-validation` specs
- [x] 1.6 Create `openspec/changes/windows-first-performant-runtime/` (this change)
- [ ] 1.7 Spawn parent epic + first calibration bead under `hud-9wljr`
- [ ] 1.8 Commit and push (mandatory landing protocol)

## 2. Performance baseline (next)

- [ ] 2.1 Identify or stand up a reference Windows machine; record CPU, GPU, RAM, display, OS build in `docs/reports/`
- [ ] 2.2 Run existing benches (`examples/widget_publish_load_harness`, scene-graph mutation throughput, frame-time histograms, overlay composite cost) on the reference machine
- [ ] 2.3 Write a baseline report under `docs/reports/windows_perf_baseline_2026-05.md` covering: frame time p50/p99/p99.9, input latency triple, widget raster cost, overlay composite cost, idle CPU/GPU/memory, multi-agent soak behavior
- [ ] 2.4 Compare baseline to the proposed targets in `design.md` §1; identify the top three gaps and their suspected causes

## 3. Lock budgets

- [ ] 3.1 Update `about/craft-and-care/engineering-bar.md` §2 with the calibrated Windows-only performance budgets and reference-hardware tag
- [ ] 3.2 Add CI gates in the existing benchmark harness that fail PRs which regress past the locked budgets
- [ ] 3.3 Document the reference-hardware procurement / claim path

## 4. Profile-and-fix

For each gap identified in §2.4, one bead per bottleneck:

- [ ] 4.1 Frame-pacing fixes (swapchain, present interval, DWM composition behavior)
- [ ] 4.2 Transparent-overlay composite cost (Vulkan path, premultiplied alpha verification, redundant blits)
- [ ] 4.3 Widget rasterization cost (resvg fast path, texture cache hit rate, parameter-bind allocations)
- [ ] 4.4 Scene-graph mutation throughput (gRPC bidi pipelining, batch commit cost, MCP→gRPC conversion)
- [ ] 4.5 Hot-path allocations (allocator profile, ring-buffer audit, SmallVec/ArrayVec usage)
- [ ] 4.6 Idle cost (overlay mode at zero agents — every milliwatt counts)

Each bead must close with a before/after measurement against the locked budget.

## 5. Soak and release

- [ ] 5.1 60-minute multi-agent soak with three concurrent resident agents publishing scenes/widgets/zones; verify no leaks, no regressions, no jitter excursions
- [ ] 5.2 Reconcile spec-to-code under the active spec set (post-deferral); generate closeout report under `docs/reports/`
- [ ] 5.3 Tag a Windows release artifact with attached perf report
- [ ] 5.4 Archive this change

## 6. Cooperative HUD projection completion (parallel)

Tracked under `hud-ggntn.*`, not duplicated as beads here. This change asserts that those beads are in scope for the single-Windows refocus and should complete on their own track.

- [ ] 6.1 Resolve `hud-ggntn.7` (gen-1 reconciliation) and `hud-abwjw` (validation host reachability) blockers
- [ ] 6.2 Complete `hud-ggntn.10` (live Windows governance validation) and `hud-ggntn.11` (gen-2 reconcile)
- [ ] 6.3 Archive cooperative-hud-projection change after closeout report
