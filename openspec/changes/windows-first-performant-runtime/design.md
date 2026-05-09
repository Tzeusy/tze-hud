# Windows-First Performant Runtime — Design Notes

## 1. Performance bar (proposed; to be calibrated against current behavior before locking)

The shipping Windows runtime should hit, on a documented reference machine, the following targets sustained across a 60-minute soak with three concurrent resident agents publishing scene + widget + zone updates:

| Property | Target | Source / rationale |
|---|---|---|
| Frame time p99 | ≤ 8.3 ms (120 Hz capable) | Stricter than the v1 `≤ 16.6 ms` (60 Hz) bar — Windows is the only target now, so the budget can lift |
| Frame time p99.9 | ≤ 16.6 ms | No frame should drop below 60 Hz |
| input_to_local_ack p99 | ≤ 2 ms | Halved from v1 (4 ms) — local-first feedback is the single most felt latency on a HUD |
| input_to_scene_commit p99 | ≤ 25 ms | Halved from v1 (50 ms) |
| input_to_next_present p99 | ≤ 16.6 ms | Halved from v1 (33 ms) |
| Widget SVG re-rasterization (512×512) | ≤ 1 ms p99 | Halved from v1 (2 ms) |
| Transparent-overlay composite cost vs fullscreen | ≤ +0.5 ms p99 added | Per-pixel transparency must not double frame cost |
| Idle CPU (overlay mode, no agents) | ≤ 1% on a single core | Background HUD must be free |
| Idle GPU (overlay mode, no agents) | ≤ 0.5% device utilization | Same |
| Memory growth (60-min soak) | ≤ 5 MB total drift | Leaks are correctness bugs |

These numbers are **proposals** until benched against current behavior. The first work bead under this change calibrates a reference machine, runs the existing benchmarks, and reports the gap; only then are budgets locked in `craft-and-care/engineering-bar.md`.

## 2. Reference hardware

Define one canonical Windows reference machine (CPU, GPU, RAM, display refresh rate, OS build) and treat it as the calibration anchor for every benchmark in this change. Any number quoted in spec or doctrine without a reference-hardware tag is rejected.

## 3. Scope cuts vs v1.md

What v1 already says we ship continues to ship. What this refocus removes from active scope:

- All media (WebRTC, GStreamer, video decode, AV, recording, cloud-relay) — already deferred in `v1.md`; tighten to `[deferred indefinitely]`.
- All multi-device profiles (mobile, glasses, upstream precomposition) — already deferred in `v1.md`; same tightening.
- Embodied presence level — was scheduled as v2 phase 2; removed from any near-term roadmap.

What v1 does **not** explicitly cut but the refocus puts on the back burner:

- macOS and Linux desktop as deployment lanes (CI correctness only).
- Cross-machine deploy/validate flow as a forward investment (existing automation kept as-is, no expansion).

## 4. What we double down on

Areas where the refocus admits aggressive investment, since the surface area shrunk:

- **Frame-pacing on Windows.** D3D12 swapchain, DWM composition behavior in overlay mode, present interval tuning, frame-time histograms.
- **Transparent-overlay performance.** The Windows path uses `WS_EX_NOREDIRECTIONBITMAP` + Vulkan + premultiplied alpha (per existing repo memory). Profile and tune this lane specifically; it is the headline differentiator vs. fullscreen mode.
- **Widget rasterization and texture caching.** resvg pipeline, per-instance texture lifetime, parameter-binding fast path.
- **Scene-graph mutation throughput.** Atomic batch commits, gRPC bidi stream pipelining, MCP-to-runtime conversion cost.
- **Compositor allocation discipline.** Zero allocations on the hot path, bounded ring buffers everywhere, profiled allocator.
- **Diagnostics.** Per-frame structured telemetry that an LLM can read and act on without rendering.

## 5. Sequencing

1. **Calibrate.** Stand up a reference Windows machine (or claim an existing one), run the current benchmarks, write a baseline report under `docs/reports/`.
2. **Lock budgets.** Once the gap between current and target is measured, lock budgets in `craft-and-care/engineering-bar.md` and add CI gates.
3. **Profile-and-fix loops.** One bead per identified bottleneck — frame pacing, overlay composite, widget raster, etc. — each measured against the locked budget.
4. **Soak.** 60-minute multi-agent soak with no leaks, no regressions, no jitter excursions.
5. **Tag a release.** A versioned Windows artifact with the perf report attached.

Phases 1–4 are bead-decomposed; phase 5 is a milestone bead.

## 6. Cooperative HUD projection — interaction with this refocus

The cooperative-hud-projection change stays active because its purpose (let an already-running LLM session project into a HUD running on the same Windows box) is single-device. However:

- **No new "different-runtime-bridge" capability** is proposed beyond what is already specified.
- The shipped projection daemon and `resident-projection` gRPC adapter remain in place. Pending beads (`hud-ggntn.7`, `hud-ggntn.10`, `hud-ggntn.11`) are completed under existing scope.
- Any future evolution of cooperative projection that adds multi-machine, multi-LLM orchestration semantics is **deferred** until after this change archives.

## 7. What this change does NOT propose

- No new RFCs. No new doctrine files. The doctrine surface this change touches is by way of *deferral markers*, not new documents.
- No reversion of shipped code. Projection adapter, scene graph, lease system, widget pipeline all remain.
- No specific implementation decisions about renderer internals — those land per-bead with profile data attached.
