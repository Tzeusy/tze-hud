# Review: RFC 0002 Runtime Kernel — Round 4 of 4
## Final Hardening and Quantitative Verification

**Issue:** rig-5vq.18
**Date:** 2026-03-22
**RFC reviewed:** about/law-and-lore/rfcs/0002-runtime-kernel.md
**Doctrine read:** architecture.md, failure.md, security.md, validation.md, v1.md
**RFCs cross-checked:** RFC 0001, RFC 0003, RFC 0005, RFC 0007, RFC 0008, RFC 0009

---

## Scores

| Dimension | Score | Rationale |
|---|---|---|
| Doctrinal Alignment | 5 | All doctrine principles faithfully expressed. All prior round MUST-FIX items (T-1 through T-9) remain correctly applied. No regressions. Frame sovereignty, local feedback first, arrival≠presentation, and the degradation doctrine are all quantitatively traceable to doctrine passages. |
| Technical Robustness | 4 | Three new issues found in round 4 (T-10, T-11, T-12); all applied directly to the RFC in this round. Post-fix the struct is complete, the channel topology is complete, and all cross-references are correct. One SHOULD-FIX (S-1 measurement endpoint) also applied. |
| Cross-RFC Consistency | 5 | Round 3 brought RFC 0002 into full alignment with RFCs 0005, 0008, 0009. Round 4 corrects one residual wrong RFC reference (T-12) and confirms no new cross-RFC drift. All shared types, enums, error codes, and ID schemes remain consistent. |

All dimensions are at or above 4. All MUST-FIX items have been applied directly to the RFC.

---

## Doctrine Files Reviewed

### validation.md
- Per-frame telemetry field list in §Layer 3 ("throughput: tiles, draw calls, texture uploads, coalesced updates, dropped ephemerals") confirms that `draw_call_count` and `mutation_count_this_frame` are load-bearing fields, not optional. Their absence from Appendix A was a MUST-FIX (T-10).
- Split latency budgets (input_to_local_ack, input_to_scene_commit, input_to_next_present) are faithfully reproduced in §9 Quantitative Requirements. The measurement point for `input_to_next_present` was imprecise — fixed (S-1).
- Hardware-normalized performance requirement is correctly noted in §9 preamble.
- Five validation layers are correctly cited in §8.5.
- DR-V2 through DR-V6 are all explicitly satisfied in the Design Requirements table (§0).

### architecture.md
- Screen sovereignty is fully expressed: compositor owns GPU context, scene state, input stream, and window surface.
- Three-plane protocol architecture is correctly represented: gRPC (resident control), MCP (compatibility), WebRTC (media, post-v1).
- Click-through overlay model (§7.2) is complete and accurate for all four platform targets.
- Session model ("one primary stream per agent") is consistent with §4.1 handshake and RFC 0005.

### failure.md
- Degradation ladder maps correctly to doctrine's six axes (§6.2, with v1-scope notes for deferred axes).
- Agent crash/slow/noisy/misbehave modes are all handled: §4.2 (session limits), §5.2 (budget enforcement, revocation), §1.4/§7.3 (GPU failure).
- "What the user always sees" guarantee is preserved: input_to_local_ack < 4ms, tab switching, human override controls always visible, chrome layer never depends on agents.

### security.md
- Resource governance (warning → throttle → revoke) exactly reproduced in §5.2.
- Trust gradient (guest vs. resident) reflected in §4.2 session limits (guest sessions always succeed).
- Capability scopes, revocability, isolation — all faithfully expressed.

### v1.md
- All v1 compositor deliverables are addressed: headless mode, overlay/fullscreen, three platforms, lease model, 60fps, split latency budgets, five validation layers.
- Post-v1 deferrals (media, WebRTC, audio-first, stream reduction) are correctly annotated in §6.2 and §8.

---

## Findings Applied in This Round

### MUST-FIX (all addressed)

**T-10: `TelemetryRecord` struct in Appendix A was missing `draw_call_count`, `mutation_count_this_frame`, and `timing_record` fields**

- **Location:** Appendix A `TelemetryRecord` struct; §3.2 Stage 8 prose.
- **Problem:** §3.2 Stage 8 prose listed `TelemetryRecord` fields including `draw_call_count` and `mutation_count_this_frame`. Appendix A's Rust struct did not include these fields. Additionally, the Appendix A comment said "embedded here as timing_record" but the struct had no `timing_record` field. This created a direct contradiction between the spec prose and the normative type sketch — an implementer following Appendix A would produce a struct incompatible with the monitoring expectations in §3.2 and with validation.md §Layer 3 ("throughput: ... draw calls").
- **Fix applied:**
  - Appendix A struct now includes `draw_call_count: u32`, `mutation_count_this_frame: u32`, and `timing_record: Option<FrameTimingRecord>` with inline `Option<>` semantics explained (populated once RFC 0003 schema is finalized).
  - Added `FrameTimingRecord` opaque placeholder struct in Appendix A with a comment pointing to RFC 0003 as authoritative.
  - §3.2 Stage 8 field list updated to include all Appendix A fields (degradation_level, shed_count, timing_record) that were present in the struct but absent from the prose description.
- **Doctrine rationale:** validation.md §Layer 3 requires per-frame throughput metrics. draw_call_count is the primary GPU throughput indicator. An implementer who ships without it cannot measure GPU bottlenecks.

---

**T-11: `SceneLocalPatch` channel missing from §2.6 Channel Topology table**

- **Location:** §2.6 Channel Topology table; §3.2 Stage 2.
- **Problem:** Stage 2 (Local Feedback) produces a `SceneLocalPatch` and explicitly states it is "forwarded to the compositor thread." The §2.6 Channel Topology table listed five channels but not the `SceneLocalPatch` channel. An implementer reading §2.6 as the authoritative channel list would have no information about the type, capacity, backpressure behavior, or drop policy for this message. This left the main→compositor local feedback path unspecified in the topology — a gap that would likely produce a blocking mutex or ad-hoc shared state in the implementation.
- **Fix applied:**
  - Added `SceneLocalPatch (main → compositor)` row to §2.6 table: ring buffer, capacity 64, oldest dropped (latest hit-state wins).
  - Added `SceneLocalPatch` Rust struct sketch with `changes: Vec<(SceneId, LocalStateFlags)>` and `LocalStateFlags { pressed, hovered }`.
  - Updated §2.6 implementation note to include `SceneLocalPatch` in the list of drop-oldest channels requiring ring-buffer implementation.
  - Explained that the compositor thread drains `SceneLocalPatch` at the start of Stage 3 and applies local state patches without a full commit cycle.
- **Doctrine rationale:** "Local feedback first" and "input_to_local_ack p99 < 4ms" are core doctrine invariants. Without a specified channel for `SceneLocalPatch`, an implementer cannot implement the local feedback contract correctly.

---

**T-12: §1.1 incorrectly referenced "RFC 0003" as the gRPC control plane**

- **Location:** §1.1 Single-Process Model, second paragraph, final sentence.
- **Problem:** The sentence read: "Agents interact exclusively through the gRPC control plane (RFC 0003)." RFC 0003 is the Timing RFC, not the Session/Protocol RFC. The gRPC resident control plane is defined in RFC 0005 (Session Protocol). An implementer reading §1.1 would look to RFC 0003 for gRPC protocol definitions and find timing semantics instead.
- **Fix applied:** Sentence rewritten to name both planes correctly: "Agents interact through the gRPC resident control plane (RFC 0005) and the MCP compatibility plane; the Timing RFC (RFC 0003) defines timing semantics for payloads on both planes."
- **Doctrine rationale:** An incorrect cross-reference at a high-visibility location (the first substantive paragraph of the RFC) creates immediate confusion for any implementer.

---

### SHOULD-FIX (addressed)

**S-1: `input_to_next_present` measurement point in §9 specified "Stage 7 end" but present() is called after Stage 7**

- **Location:** §9 Quantitative Requirements table, `input_to_next_present` row.
- **Problem:** The measurement point was listed as "Stage 1 start → Stage 7 end." However, Stage 7 is "GPU Submit" — it signals `FrameReadySignal` to the main thread, which then calls `surface.present()`. The actual measurement endpoint for `input_to_next_present` (per validation.md §Layer 3: "time from input event to the next rendered frame containing the committed scene change appearing on screen") is the main thread's `present()` call returning, not the GPU submit. On a slow vsync path the gap between Stage 7 end and present() completion could be significant. Stating "Stage 7 end" as the endpoint understates the metric and would cause an implementer to place the measurement probe in the wrong location.
- **Fix applied:** Measurement point updated to: "Stage 1 start → main thread `surface.present()` returns (after Stage 7 signals FrameReadySignal)."
- **Doctrine rationale:** validation.md §Layer 3 defines input_to_next_present as the time to the frame "appearing on screen." That is the surface.present() call, not the GPU submit.

---

## Quantitative Budget Verification

This section confirms that the performance budgets in §9 are internally consistent and achievable.

### Stage budget arithmetic

| Stages | Budget | Thread |
|--------|--------|--------|
| Stage 1 + Stage 2 | 1ms combined | Main |
| Stage 3 | 1ms | Compositor |
| Stage 4 | 1ms | Compositor |
| Stage 5 | 1ms | Compositor |
| Stage 6 | 4ms | Compositor |
| Stage 7 | 8ms | Compositor |
| Stage 8 | 0ms (non-blocking on compositor) | Telemetry |
| **Total** | **16ms** | |

At 60Hz the frame budget is 16.67ms. The sum of stage budgets is exactly 16ms, leaving 666μs headroom. This is tight but honest — the RFC correctly calls out that Stage 7 at 8ms is not fully under software control and the frame-time guardian exists to handle overruns.

**Verdict:** Arithmetic is sound. The budgets are consistent with the 16.6ms total frame budget and with each other.

### Latency budget cross-check

- `input_to_local_ack` p99 < 4ms: Stages 1+2 combined = p99 < 1ms. Substantial headroom. Achievable.
- `input_to_next_present` p99 < 33ms: Two frames at 60Hz = 33.3ms. The input event arrives at Stage 1 of frame N; the rendered result appears at Stage 7 of frame N (same frame if the input arrives before the frame budget expires) or frame N+1 (if it arrives late). Two-frame budget is the correct target. Achievable.
- `input_to_scene_commit` p99 < 50ms: Includes network round-trip (agent receives event, processes, sends mutation). For loopback gRPC this is well within budget. For remote agents with network latency the budget is generous. Achievable for local agents.
- `Mutation to next present` p99 < 33ms: MutationBatch enqueue → Stage 7 end. Same two-frame window. Achievable.

**Verdict:** All three split latency budgets are consistent with doctrine (validation.md §Layer 3) and with each other.

### Thread safety guarantees

The RFC specifies three concurrency-sensitive paths:

1. **HitTestSnapshot** (`ArcSwapFull<HitTestSnapshot>`): Main thread reads via `arc_swap.load()` (atomic pointer read, no lock). Compositor thread writes via `arc_swap.store()` (atomic pointer write). No mutex; no data race; no torn state. Correct.
2. **InputRegionMask** (`ArcSwapFull<InputRegionMask>`): Same pattern as HitTestSnapshot for WM_NCHITTEST handler. Correct.
3. **GPU Device/Queue ownership**: Compositor thread owns `wgpu::Device` and `wgpu::Queue`. Main thread holds surface handle only for `present()`. Exclusive ownership is stated clearly and enforced by the channel model. Correct.

**Verdict:** Thread safety guarantees are provable by construction from the ownership model and ArcSwap semantics. No data races identified.

### Resource cleanup completeness

On agent revocation (§5.2), the post-revocation resource footprint must be zero:
1. `BudgetState` moves to `Revoked`.
2. All active leases enqueue `LeaseRevocationEvent`.
3. All agent-owned tiles marked orphaned.
4. After post-revocation delay (default 100ms): textures and node data freed, reference counts reach zero.
5. `AgentResourceState` removed from per-agent table.

This procedure is verified by the `disconnect_reclaim_multiagent` test scene (validation.md test corpus). The post-revocation delay of 100ms is long enough for `LeaseRevocationEvent` fan-out but prevents the resource release from racing with the notification delivery.

**Verdict:** Resource cleanup is complete and verifiable.

### Frame-time guardian correctness

Guardian checks at Stage 5 (Layout Resolve) when cumulative Stages 3–5 time exceeds 3ms. Budget breakdown to that check point:
- Stage 3: p99 < 1ms
- Stage 4: p99 < 1ms
- Stage 5: p99 < 1ms
- Total Stages 3–5: p99 < 3ms

The guardian triggers exactly at the expected boundary. Work shedding in Stage 6 can reduce Stage 6 time from 4ms to < 1ms for low-priority tiles, buying back headroom for Stage 7. The logic is sound.

---

## Cross-RFC Consistency — Round 4 Check

No new RFCs have landed since round 3. This section confirms no drift in the RFCs that were already checked.

| RFC | Section | Status |
|-----|---------|--------|
| RFC 0001 | §4 Mutation Pipeline, §7 SceneSnapshot | Consistent — `SceneMutation`, `SceneId`, `MutationBatch` usage unchanged |
| RFC 0003 | §FrameTimingRecord | Consistent — `timing_record: Option<FrameTimingRecord>` with deferral note correct |
| RFC 0005 | §1.4 grace period, §4.2 session resumption, §5.2–5.3 batch deduplication | Consistent — RFC 0005 is now correctly cited as the gRPC control plane reference (T-12 fixed) |
| RFC 0007 | §5.1 safe mode CRITICAL_ERROR | Consistent — §1.4 and §7.3 both reference RFC 0007 §5.1 correctly |
| RFC 0008 | §2.2 sort order, §3.3 Level 5 not SUSPENDED, §4.3 budget ladder | Consistent — T-8 and T-9 fixes from round 3 remain correct |
| RFC 0009 | §5 GPU failure resolution, §5.2/5.3 required changes | Consistent — T-7 fix from round 3 remains correct |

---

## Implementer Readiness Assessment

This section assesses whether an implementer would have zero unanswered questions after reading RFC 0002.

| Area | Verdict | Notes |
|------|---------|-------|
| Process architecture | Ready | Single-process model, entry point initialization order, shutdown sequence all specified |
| Thread model | Ready | Four threads, responsibilities, ownership all specified |
| Channel topology | Ready (post-T-11) | All 6 channels now specified with type, capacity, drop policy |
| Frame pipeline | Ready | All 8 stages specified with thread assignment and budget |
| Admission control | Ready | Handshake sequence, session limits, hot-connect all specified |
| Budget enforcement | Ready | Three-tier ladder, per-agent tracking, revocation procedure all specified |
| Degradation policy | Ready | Five levels, trigger/recovery thresholds, hysteresis all specified |
| Window surface management | Ready | Fullscreen and overlay modes, click-through per platform, surface lifecycle |
| Headless mode | Ready | Surface type, pixel readback, software backend, frame pacing all specified |
| Quantitative requirements | Ready (post-S-1) | All budgets stated with measurement points; arithmetic verified |
| Rust types | Ready (post-T-10) | Key types in Appendix A are complete and consistent with prose |
| Cross-references | Ready (post-T-12) | RFC references corrected; all cited section numbers verified |

---

## Open Questions (Carried Forward)

The following items from prior rounds remain open and are not addressed in this round:

- **C-3:** `EventNotification` ring buffer can drop critical revocation events under overflow. Still acceptable for v1; post-v1 consideration.
- **C-4:** Frame-time guardian threshold arithmetic is tight at the boundary under OS scheduling noise. Deferred.
- **C-5:** `HeadlessSurface` pixel format `Rgba8UnormSrgb` may differ from windowed path `Bgra8UnormSrgb`; Layer 1 test pixel readback helpers should normalize. Deferred.

---

## Summary

RFC 0002 is implementation-ready after this final round. Three MUST-FIX issues were found and resolved:

1. **T-10:** `TelemetryRecord` struct in Appendix A was missing `draw_call_count`, `mutation_count_this_frame`, and `timing_record` fields that §3.2 Stage 8 prose referenced. Struct and `FrameTimingRecord` placeholder type added.
2. **T-11:** `SceneLocalPatch` channel (main → compositor, for local feedback state from Stage 2) was absent from the §2.6 Channel Topology table. Channel row added with type sketch, capacity (64), drop policy (oldest dropped), and inline struct definition.
3. **T-12:** §1.1 incorrectly named "RFC 0003" as the gRPC control plane. Corrected to RFC 0005 (Session Protocol) with RFC 0003 (Timing) cited for its actual role.

One SHOULD-FIX was also applied:

4. **S-1:** `input_to_next_present` measurement endpoint in §9 said "Stage 7 end" rather than the correct endpoint: the main thread's `surface.present()` call returning.

**Final scores: Doctrinal Alignment 5/5, Technical Robustness 4/5, Cross-RFC Consistency 5/5.** All dimensions meet or exceed the round-4 target of ≥4.
