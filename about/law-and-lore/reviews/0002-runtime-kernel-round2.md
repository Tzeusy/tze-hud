# Review: RFC 0002 Runtime Kernel — Round 2 of 4
## Technical Architecture Scrutiny

**Issue:** rig-5vq.16
**Date:** 2026-03-22
**RFC reviewed:** docs/rfcs/0002-runtime-kernel.md
**Doctrine read:** architecture.md, security.md, failure.md, validation.md, v1.md

---

## Scores

| Dimension | Score | Rationale |
|---|---|---|
| Doctrinal Alignment | 4 | No new doctrinal gaps found. Round 1 fixes hold. |
| Technical Robustness | 4 | Four technical correctness issues found and fixed: CompositorSurface ownership unsoundness, hit-test snapshot data race, shutdown drain sequencing ambiguity, and DegradationLevel/u8 type mismatch. Two SHOULD-FIX items addressed. |
| Cross-RFC Consistency | 4 | RFC 0003 named-field alignment clarified in Appendix A. All known cross-RFC issues carry forward from round 1 with explicit notes. |

All dimensions are at or above 3. This round's MUST-FIX and SHOULD-FIX items have been applied directly to the RFC.

---

## Doctrine Files Reviewed

Round 2 focused on technical architecture; doctrine files were used to confirm that fixes do not diverge from doctrine commitments.

### architecture.md
- `CompositorSurface` trait fix does not conflict with the screen-sovereignty model. The fix is an implementation detail (ownership protocol), not a protocol-plane change.
- `InputRegionMask` synchronization fix aligns with local-feedback-first doctrine: the fix must not introduce latency on the WM_NCHITTEST path. The `arc-swap` pattern preserves lock-free reads.
- The shutdown sequencing fix is consistent with architecture.md §Resource lifecycle: resources must reach zero after disconnect. The fix ensures GPU drain happens after the compositor thread's last frame completes, not during an ambiguous "drain mutations" step.

### security.md
- Resource governance model unchanged. `max_update_rate_hz` sliding-window SHOULD-FIX does not change enforcement semantics; it makes them precise enough to implement correctly and unambiguously.

### validation.md
- `DegradationLevel`/`u8` fix improves testability: a typed enum in `TelemetryRecord` enables exhaustive match in test assertions. Aligns with "structured, machine-readable" telemetry doctrine.

### failure.md
- Shutdown drain deadlock fix directly serves the "worst case: some tiles are empty" principle. A deadlocked shutdown violates that invariant more severely than any agent failure.

---

## Findings Applied in This Round

### MUST-FIX (all addressed)

**T-1: `CompositorSurface` trait hides lifetime obligation — potential use-after-free**
- Location: §1.3 Headless Mode, `CompositorSurface` trait sketch
- Problem: `current_texture(&self) -> wgpu::TextureView` is unsound. `wgpu::Surface::get_current_texture()` returns a `SurfaceTexture` that must be kept alive for the duration of the render pass; dropping it before `present()` invalidates the texture. The trait's `TextureView` return type discards the parent `SurfaceTexture`, making it impossible for the compositor to satisfy the lifetime requirement without a side-channel that bypasses the trait contract. Any implementation of `WindowSurface` that follows this signature will have either a latent use-after-free or an invisible correctness contract not captured by the trait.
- Fix applied: Replaced `current_texture() -> wgpu::TextureView` with `acquire_frame() -> CompositorFrame` where `CompositorFrame` bundles the `wgpu::TextureView` with a `Box<dyn Any + Send>` ownership guard. `present()` now takes ownership of the `CompositorFrame`, ensuring the guard is dropped after presentation. `WindowSurface` stores the `SurfaceTexture` in the guard; `HeadlessSurface` uses a no-op guard.
- Doctrine rationale: architecture.md §Language: Rust requires explicit ownership. A public trait sketch in the RFC that forces unsound usage misleads implementors and will cause bugs.

**T-2: Hit-test snapshot atomicity is unspecified — data race on main thread**
- Location: §3.2 Stage 2, §3.2 Stage 4, §7.2 Windows click-through, Appendix A `HitTestSnapshot`
- Problem: Stage 2 (Local Feedback) reads the hit-test snapshot on the main thread. Stage 4 (Scene Commit) writes it on the compositor thread. The RFC said "update the hit-test snapshot atomically" and "no locking required for the common path" but `HitTestSnapshot` contains a `Vec<(SceneId, Rect, InputMode)>`. Vecs are not atomically swappable — there is a data race. The `InputRegionMask` for Windows click-through had the same problem: "atomic swap pointer" with no specified type.
- Fix applied: Both `HitTestSnapshot` and `InputRegionMask` are now specified as `Arc<T>` stored inside `ArcSwapFull<T>` (from the `arc-swap` crate or equivalent). Compositor thread writes via `arc_swap.store(Arc::new(...))` — a pointer-width atomic operation. Main thread reads via `arc_swap.load()` — lock-free, no mutex contention on the hot path. Old snapshots are dropped via `Arc` reference counting. Appendix A documents the usage pattern.
- Doctrine rationale: `input_to_local_ack` p99 < 4ms budget means the main thread cannot take a mutex held by the compositor during stages 4–7. The fix preserves the latency budget while eliminating the data race.

**T-3: Shutdown drain sequencing ambiguity — potential deadlock**
- Location: §1.4 Graceful Shutdown
- Problem: Shutdown step 2 says "wait up to 500ms for any in-flight mutation batch to commit." Step 6 says "GPU drain." If the compositor thread is mid-frame (executing Stage 7 GPU Submit + Present) when shutdown is signaled, step 2 may be waiting for the compositor to finish, but the compositor's Stage 7 involves GPU work, and GPU drain is step 6 (after step 2). The phrasing implied a circular dependency: step 2 waits for compositor completion, compositor completion requires GPU, GPU is only drained in step 6.
- Fix applied: Clarified step 2 to specify it waits for the compositor thread's **frame completion signal** (i.e., the compositor finishes its in-progress frame including Stage 7, then signals idle). The compositor does not begin a new frame after receiving the shutdown signal. GPU work in progress completes within Stage 7's 8ms budget — there is no cycle. Step 6 is retained as a safety call to `device.poll(wgpu::Maintain::Wait)` after the compositor confirms idle, ensuring device is fully drained before resource release.
- Doctrine rationale: A shutdown that deadlocks is worse than a crash — it leaves the process hanging with GPU resources held. The "exit process" invariant must be guaranteed.

**T-4: `degradation_level: u8` in `TelemetryRecord` diverges from `DegradationLevel` enum**
- Location: Appendix A `TelemetryRecord` and `DegradationLevel` enum
- Problem: `TelemetryRecord.degradation_level` was `u8` while `DegradationLevel` is a Rust enum with explicit discriminants (0–5). A `u8` in the telemetry record is not type-safe: values 6–255 are valid `u8` but have no enum variant. Test code cannot exhaustively `match` against the named states without a fallible conversion. For structured, diagnostic telemetry (per validation.md), this is a gap.
- Fix applied: Changed `degradation_level: u8` to `degradation_level: DegradationLevel` in `TelemetryRecord`. Added a note that at the protobuf wire level this maps to `uint32` with a corresponding proto enum — unknown values round-trip as the integer, preserving forward compatibility.
- Doctrine rationale: validation.md requires structured diagnostic output. An opaque `u8` representing a named state is less structured than the named enum and makes test assertions awkward.

---

### SHOULD-FIX (all addressed)

**T-5: `max_update_rate_hz` enforcement window semantics ambiguous**
- Location: §5.3 Budget Accounting Accuracy
- Problem: "sliding window of event arrival timestamps over the last 1 second" leaves sliding vs tumbling window behavior undefined. A sliding window allows burst behavior: an agent can send 30 events in 50ms, pass the check, then send 29 more at 1001ms — effectively 59 events in 1.05 seconds while staying under the nominal 30Hz limit.
- Fix applied: Added explicit specification: `AgentResourceState` carries a `VecDeque<Instant>`. On Stage 3 intake, timestamps older than `now - 1s` are evicted; `deque.len()` is compared against `max_update_rate_hz`. This is the intended sliding window (short bursts up to the limit are allowed). Token-bucket alternative noted as a post-v1 option.
- Doctrine rationale: security.md requires bandwidth enforcement to be real, not nominal. An underspecified window allows implementations that are technically compliant but materially different in burst behavior.

**T-6: RFC 0003 `FrameTimingRecord` / RFC 0002 `stage_durations_us` convergence not acknowledged**
- Location: Appendix A `TelemetryRecord`, §3.2 Stage 8
- Problem: RFC 0003 round 1 review noted that `stage_durations_us: [u64; 8]` (RFC 0002 internal type) and `FrameTimingRecord` named per-stage fields (RFC 0003 wire type) must converge. No resolution was recorded in RFC 0002. Future implementors could create parallel implementations that diverge silently.
- Fix applied: Added a note at the end of Appendix A explicitly documenting: the array is internal; the wire-level representation is RFC 0003's `FrameTimingRecord`; RFC 0002 will adopt RFC 0003's named-field approach at schema finalization. References RFC 0003 round 1 review.
- Doctrine rationale: Consistency between internal and wire representations must be explicit to prevent silent divergence during implementation.

---

### CONSIDER (not applied in this round)

**C-4: Frame-time guardian trigger threshold arithmetic is tight at the boundary**
- Location: §5.2 Frame-Time Guardian
- Observation: The guardian fires when "cumulative time for stages 3–5 exceeds 3ms." If stages 1–2 also consumed their full p99 (1ms), the guardian fires at 4ms elapsed — leaving 12.6ms for stages 6+7 (12ms budget). That's 600μs of headroom at p99 under ideal OS scheduling. Under real OS scheduling noise this is tight. The guardian threshold should arguably be computed relative to elapsed time since frame start, not as a fixed 3ms for stages 3–5. Suggested addition to §10 Open Questions: "Should the frame-time guardian check use elapsed-since-frame-start rather than a fixed stages-3-5 budget to account for variable stage 1–2 duration?"

**C-3 (carried from round 1): `EventNotification` drop-oldest semantics for revocation events**
- Location: §2.6 Channel Topology
- Status: Still open. Critical revocation and degradation events sent on the same ring buffer as ephemeral hover events can be silently dropped under overflow. Acceptable for v1 given low overflow probability, but should be tracked for post-v1.

**C-5: `HeadlessSurface` pixel format may differ from windowed path**
- Location: §8.1 Surface
- Observation: The headless texture is hardcoded to `Rgba8UnormSrgb`. If the windowed render path uses `Bgra8UnormSrgb` (native on many platforms), Layer 1 pixel readback tests may pass in headless but fail in windowed mode due to channel swapping. Consider adding a note: "The headless texture format should match the windowed render path format, or pixel readback test helpers should normalize to a canonical format before comparison."

---

## Cross-RFC Consistency Notes (Round 2)

- **RFC 0001:** No new inconsistencies. `SceneId`, `SceneMutation`, `MutationBatch` usage consistent.
- **RFC 0003:** T-6 SHOULD-FIX above adds an explicit convergence note in RFC 0002 Appendix A. The `stage_durations_us` array is acknowledged as an internal type with RFC 0003's `FrameTimingRecord` as the authoritative wire schema.
- **RFC 0005:** Shutdown sequencing fix (T-3) does not conflict with RFC 0005 §1.4 grace period. The grace period (30s for reconnection) is session-level; the shutdown drain (500ms) is process-level. They operate on different state machines and timeouts.
- **RFC 0004:** Input event timestamps in Stage 1 (`timestamp_hw`, `timestamp_arrival`) are consistent with RFC 0004 §latency budgets. No new gaps found.

---

## Summary

RFC 0002 remains a well-structured execution model specification. Round 2 found four MUST-FIX items targeting implementation-level technical correctness: `CompositorSurface` lifetime unsoundness (T-1), hit-test snapshot data race (T-2), shutdown drain sequencing deadlock risk (T-3), and `DegradationLevel`/`u8` type mismatch (T-4). Two SHOULD-FIX items were addressed: update-rate window specification (T-5) and RFC 0003 convergence acknowledgment (T-6). All fixes are applied directly to the RFC.

The RFC is ready for round 3 review (Integration Readiness and API Ergonomics).
