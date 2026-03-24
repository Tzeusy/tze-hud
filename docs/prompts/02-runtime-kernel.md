# Epic 2: Runtime Kernel + Compositor

> **Dependencies:** Epic 0 (test infrastructure), Epic 1 (scene graph types)
> **Depended on by:** Epics 5, 6, 11 (input needs frame pipeline, session needs runtime, shell needs compositor)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
> **Secondary specs:** `validation-framework/spec.md` (Layer 1, Layer 3)

## Prompt

Create a `/beads-writer` epic for **runtime kernel and compositor** — the sovereign single-process execution model that owns the frame pipeline, thread architecture, degradation ladder, and headless rendering path.

### Context

The runtime kernel is the hot path. It owns an 8-stage frame pipeline, explicit thread roles, bounded channels, admission control, and a 5-level degradation ladder. The existing crate `crates/tze_hud_runtime/` has a partial implementation with `HeadlessRuntime`, `budget.rs` (991 lines, 23 tests), and frame rendering. `crates/tze_hud_compositor/` owns wgpu-based rendering with `HeadlessSurface` for pixel readback. Epic 0 provides budget assertion tests (5 existing) and Layer 1 pixel tests (3 existing) that this epic must satisfy.

### Epic structure

Create an epic with **5 implementation beads**:

#### 1. 8-stage frame pipeline (depends on Epic 1 scene hierarchy)
Implement the staged frame pipeline per `runtime-kernel/spec.md` Requirement: Frame Pipeline.
- Stages: input drain → scene commit → layout → render encode → GPU submit → present → telemetry → idle
- Per-stage budget tracking with `FrameTelemetry` emission
- Total pipeline p99 < 16.6ms (normalized to reference hardware) at 60fps
- **Acceptance:** `test_frame_time_p99_within_budget()` from Epic 0 passes. Per-stage telemetry emitted for every frame. Stage ordering is enforced.
- **Spec refs:** `runtime-kernel/spec.md` Requirement: Frame Pipeline, Requirement: Frame Time Budget

#### 2. Degradation ladder (depends on #1)
Implement the 5-level degradation system per `runtime-kernel/spec.md` Requirement: Degradation Ladder.
- Level 0 Normal → Level 1 Coalesce → Level 2 ReduceTextureQuality → Level 3 DisableTransparency → Level 4 ShedTiles → Level 5 Emergency
- Trigger: frame_time_p95 > 14ms over 10-frame rolling window
- Recovery: frame_time_p95 < 12ms sustained over 30-frame window
- Tile shedding sorts by (lease_priority ASC, z_order DESC)
- **Acceptance:** Budget enforcement tests from `budget.rs` all pass. Degradation trigger/recovery thresholds verified. Shedding order matches spec.
- **Spec refs:** `runtime-kernel/spec.md` Requirement: Degradation Ladder, Requirement: Frame-Time Guardian

#### 3. Bounded channels and thread architecture (depends on #1)
Implement inter-thread communication per `runtime-kernel/spec.md` Requirement: Thread Architecture.
- Main thread (elevated priority), compositor thread, telemetry thread, network thread
- All channels bounded — no unbounded queues
- Ring buffers for drop-oldest semantics (InputEvent, SceneLocalPatch, etc.)
- Backpressure channels for transactional messages (capacity 256, never dropped)
- Coalesce-key channels for state-stream messages (capacity 512)
- **Acceptance:** No unbounded allocations under sustained load. Backpressure channels never drop transactional messages. Channel capacity assertions pass.
- **Spec refs:** `runtime-kernel/spec.md` Requirement: Thread Architecture, Requirement: Bounded Channels

#### 4. Headless mode parity (depends on #1, #3)
Ensure headless mode is fully functional per `runtime-kernel/spec.md` Requirement: Headless Mode.
- Software GPU fallback when available
- Full frame pipeline, pixel readback, telemetry, gRPC session server
- No window, no display server dependency
- CI-suitable: deterministic output for same scene input
- **Acceptance:** All Layer 1 pixel readback tests from Epic 0 pass in headless mode. All Layer 3 budget tests pass. `cargo test` succeeds with no display server.
- **Spec refs:** `runtime-kernel/spec.md` Requirement: Headless Mode, `validation-framework/spec.md` Requirement: Headless Execution

#### 5. Admission control and resource budgets (depends on #1, #2)
Implement per-agent resource enforcement per `runtime-kernel/spec.md` Requirement: Admission Control.
- Per-session resource limits: max_tiles, max_texture_bytes, max_update_rate_hz, max_nodes_per_tile, max_active_leases
- Three-tier enforcement: Warning → Throttle → Revocation
- Session memory overhead < 64KB per agent (exclusive of content)
- **Acceptance:** Budget enforcement tests in `budget.rs` all pass. Three-tier ladder transitions verified. Resource limit violations produce correct structured errors.
- **Spec refs:** `runtime-kernel/spec.md` Requirement: Admission Control, Requirement: Per-Agent Resource Budgets

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `runtime-kernel/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference the exact spec scenarios
3. **Acceptance criteria** — which Epic 0 budget/pixel tests must pass
4. **Crate/file location** — `crates/tze_hud_runtime/` and `crates/tze_hud_compositor/`
5. **Performance gates** — specific p99 latency and throughput targets from the spec

### Dependency chain

```
Epic 1 ──→ #1 Frame Pipeline ──→ #2 Degradation Ladder ──→ #5 Admission Control
                              ──→ #3 Channels/Threads ──→ #4 Headless Parity
```
