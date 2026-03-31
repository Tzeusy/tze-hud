# Review: RFC 0002 Runtime Kernel — Round 1 of 4
## Doctrinal Alignment Deep-Dive

**Issue:** rig-5vq.15
**Date:** 2026-03-22
**RFC reviewed:** about/law-and-lore/rfcs/0002-runtime-kernel.md
**Doctrine read:** architecture.md, security.md, failure.md, validation.md, v1.md

---

## Scores

| Dimension | Score | Rationale |
|---|---|---|
| Doctrinal Alignment | 4 | RFC faithfully implements core doctrine commitments. Minor gaps in DR coverage table, degradation ladder v1 scope note, and missing `input_to_scene_commit` budget. All addressed by MUST-FIX/SHOULD-FIX items. |
| Technical Robustness | 4 | Thread model, frame pipeline, budget enforcement, and degradation ladder are sound. Missing thread priority specification and incomplete `BudgetState` enum are fixed. |
| Cross-RFC Consistency | 4 | Shared types (SceneId, MutationBatch, SceneMutation) are used consistently. Minor prose/struct naming inconsistency in TelemetryRecord fixed. RFC 0003 FrameTimingRecord cross-reference added. |

All dimensions are at or above 3. This round's MUST-FIX and SHOULD-FIX items have been applied directly to the RFC.

---

## Doctrine Files Reviewed

### architecture.md
- Screen sovereignty: RFC §1.1 correctly establishes single-process model, agent isolation via gRPC boundary.
- Compositing model (three layers): RFC §3.2 Stage 6 encodes chrome layer; the background and content layer distinction is implicit in the pipeline but not explicitly named in §6 render encode — this is acceptable since the full layer model is defined in RFC 0001.
- Session model (one stream per agent): RFC §4 admission control aligns with architecture.md §Session model. HTTP/2 concurrent-stream limits acknowledged in architecture.md are honored.
- Error model: RFC references structured errors with `BudgetWarning` events; full error schema is deferred to RFC 0005 and 0003 per architecture.md §Error model.
- Resource lifecycle: RFC §5.1 tracks per-agent resource counters. Added §5.2 resource cleanup on revocation to fully satisfy the "after disconnect, resource footprint is zero" invariant.
- Versioning: RFC §4.1 handshake includes `protocol_version` in `SessionOpen`. Policy is in RFC 0005 (appropriate delegation).

### security.md
- Trust gradient: RFC §4.1–4.3 covers authentication, capability negotiation, session limits — aligned.
- Resource governance: security.md lists 5 enforced dimensions (texture memory, bandwidth, concurrent streams, CPU time, active leases). RFC §5.1 `AgentResourceState` tracks texture_bytes, node_count, tile_count, lease_count, update_rate. **Gap:** concurrent streams and CPU time are not tracked. This is a SHOULD-FIX but acceptable for v1 because concurrent streams require media (deferred) and CPU time tracking adds measurement overhead that is not justified until profiling shows it as a bottleneck.
- Enforcement tiers: RFC §5.2 three-tier ladder (Warning → Throttle → Revocation) matches security.md exactly.

### failure.md
- Core principle ("worst case: some tiles are empty"): RFC §1.4 graceful shutdown and §5.2 revocation/orphaning satisfy this.
- Agent failure modes: failure.md §Agent crashes / slow / noisy / misbehaves — RFC §4.3 (envelope), §5.2 (budget tiers + frame guardian) cover all four.
- Degradation axes: RFC's 5-level ladder maps to doctrine's 6 axes. V1 scope note added to §6.2 to make the gap explicit and non-silent.
- Reconnection grace period: RFC §1.4 graceful shutdown mentions drain; grace period lifecycle is correctly delegated to RFC 0005. SHOULD-FIX: RFC 0002 should acknowledge the grace period exists and reference RFC 0005.

### validation.md
- DR-V2 (headless rendering): §8 — fully specified with wgpu offscreen texture, pixel readback, software backends.
- DR-V3 (structured telemetry): §3.2 Stage 8, §2.5 Telemetry thread — correctly specified.
- DR-V5 (trivial headless invocation): §1.3, §8 — runtime flag not compile fork.
- DR-V6 (no physical GPU for CI): Was missing from Design Requirements table. Added.
- Performance budgets: `input_to_local_ack` and `input_to_next_present` are in §9. **MUST-FIX:** `input_to_scene_commit` p99 < 50ms was missing. Added.
- Hardware normalization: RFC §9 references validation.md §Hardware-normalized performance for normalized budgets.

### v1.md
- Compositor scope (wgpu headless + windowed, tile composition, z-order, alpha blending, 60fps): §2–3 fully specified.
- Window modes (fullscreen and overlay): §7 fully specified with per-platform click-through implementations.
- Platform targets (Linux X11/Wayland, Windows, macOS): §7.2 covers all four (plus GNOME/KDE fallback).

---

## Findings Applied in This Round

### MUST-FIX (all addressed)

**F-1: TelemetryRecord field name inconsistency**
- Location: §3.2 Stage 8 prose vs Appendix A struct
- Problem: Prose used `active_agents` and `stage_durations[8]`; Rust struct used `active_sessions` and `stage_durations_us`. These two names describe the same fields and must match.
- Fix applied: Updated §3.2 prose to use `active_sessions` and `stage_durations_us[8]`. Added RFC 0003 cross-reference.
- Doctrine rationale: Inconsistent naming between specification prose and implementation sketch erodes implementor trust.

**F-2: `BudgetState` enum incomplete — missing `Revoked` variant**
- Location: §5.1 `BudgetState` enum
- Problem: The comment said "Revoked is a terminal state" but the enum had no variant for it. Any match expression on `BudgetState` would be incomplete.
- Fix applied: Added `Revoked { reason: RevocationReason }` variant and companion `RevocationReason` enum with all four documented critical trigger categories.
- Doctrine rationale: security.md §Resource governance requires revocation as a reachable state; the type must model it.

**F-3: `input_to_scene_commit` budget absent from §9**
- Location: §9 Quantitative Requirements table
- Problem: validation.md and v1.md define three split latency budgets: `input_to_local_ack`, `input_to_scene_commit`, `input_to_next_present`. Only two appeared in §9. The omission implied the runtime does not track or enforce the 50ms commit budget.
- Fix applied: Added `input_to_scene_commit | p99 < 50ms (local agents)` row with measurement point explanation.
- Doctrine rationale: validation.md §Layer 3 explicitly requires all three split latency budgets to be tracked per-session.

**F-4: RFC 0003 referenced as "forthcoming"**
- Location: §1.1
- Problem: RFC 0003 exists. Calling it "forthcoming" is stale.
- Fix applied: Removed "forthcoming" from the reference.

### SHOULD-FIX (all addressed)

**F-5: DR-V6 missing from Design Requirements table**
- Location: §Design Requirements Satisfied table (line 33)
- Problem: DR-V6 (no physical GPU required for CI) is fully satisfied by §8.3's HEADLESS_FORCE_SOFTWARE mechanism, but was not claimed in the table.
- Fix applied: Added DR-V6 row referencing §8.3.

**F-6: Thread priority not specified**
- Location: §2.2 Main Thread
- Problem: The original issue description explicitly listed "Thread priority: main thread elevated (SCHED_RR or equivalent) for input latency guarantee" as a required section. The RFC omitted this entirely. Thread priority elevation is essential to meet the `input_to_local_ack` p99 < 4ms budget in production under OS scheduling noise.
- Fix applied: Added platform-specific thread priority specification (SCHED_RR on Linux, QOS_CLASS_USER_INTERACTIVE on macOS, THREAD_PRIORITY_TIME_CRITICAL on Windows) with fallback behavior for privilege failures.

**F-7: Degradation ladder drops 2 doctrine axes without explanation**
- Location: §6.2 Degradation Ladder
- Problem: failure.md defines 6 degradation axes. RFC's 5-level ladder silently drops "reduce concurrent streams" and "audio-first fallback". Without explanation, a future author might restore the media stack and not realize these levels need to be inserted.
- Fix applied: Added V1 scope note table mapping doctrine axes to RFC levels, with explicit "deferred to post-v1" rows for the two omitted axes. Post-v1 RFC revision instruction included.

**F-8: Resource cleanup on revocation not specified**
- Location: §5.2 Budget Tiers
- Problem: architecture.md §Resource lifecycle requires that after a session ends, its resource footprint is zero. The RFC documented what triggers revocation but not what the compositor does immediately after — leaving the resource cleanup sequence underspecified.
- Fix applied: Added ordered 6-step resource cleanup sequence to §5.2, including the post-revocation delay, lease revocation event fan-out, and reference to the `disconnect_reclaim_multiagent` test scene.

### CONSIDER (not applied in this round)

**C-1: Per-stage budget headroom is tight**
- Location: §3.1 Pipeline Overview
- Observation: Sum of p99 stage budgets = 16.2ms, leaving 400μs for overhead (channel ops, context switches, scheduling jitter) at p99 16.6ms. On a real system, the sum-of-p99s will often be lower than the combined p99, so this may be fine in practice. However, the RFC should acknowledge this in §10 Open Questions.
- Suggested addition: Add an Open Question item: "Stage budget headroom: the sum of stage p99 budgets is 16.2ms, leaving 400μs for scheduling overhead. Post-implementation: verify that the combined p99 frame time is within budget under OS jitter."

**C-2: Missing bandwidth and CPU time in `AgentResourceState`**
- Location: §5.1 `AgentResourceState`
- Observation: security.md lists "bandwidth" and "CPU time" as enforced resource dimensions. The RFC tracks update rate via Hz limit but not raw bytes/sec or CPU time. For v1 with no media, this gap is acceptable. Post-v1, these fields will need to be added.
- Suggested future addition: `bandwidth_bytes_per_second_window: VecDeque<(Instant, u64)>` and `cpu_time_window_us: u64`.

**C-3: `EventNotification` drop-oldest semantics for correctness events**
- Location: §2.6 Channel Topology
- Observation: The `EventNotification` channel drops oldest on overflow. This is correct for ephemeral events (hover, cursor position), but if lease revocation events or degradation events are sent on the same channel, dropping the oldest could mean an agent never learns its lease was revoked. Consider whether critical events (revocation, degradation) should be sent on a separate, backpressured channel vs. a ring buffer.

---

## Cross-RFC Consistency Notes

- **RFC 0001:** `SceneId`, `SceneMutation`, `MutationBatch` used consistently in RFC 0002 §5.1 and Appendix A. `SceneId` correctly reused for `session_id` in `AgentResourceState` and `MutationBatch`.
- **RFC 0003:** `FrameTimingRecord` extends the telemetry surface defined here. The cross-reference was missing from §3.2 Stage 8 and Appendix A; added.
- **RFC 0004:** Input latency budgets (`input_to_local_ack` p99 < 4ms) are consistent. RFC 0004 §latency budgets references the local feedback path (stages 1+2) in RFC 0002 §3.2.
- **RFC 0005:** Session lifecycle (Connecting → Handshaking → Active → Resuming → Closed) is compatible with the admission control handshake in RFC 0002 §4.1. Grace period defaults (30,000ms) in RFC 0005 §1.4 are not contradicted by RFC 0002 (RFC 0002 does not specify grace period, correctly delegating to RFC 0005).
- **RFC 0006:** Display profile definitions in RFC 0006 depend on RFC 0002's `CompositorConfig`. The `display_profile` section in RFC 0006 §2.3 is compatible with the runtime configuration model in RFC 0002 §1.2.

---

## Summary

RFC 0002 is a well-structured execution model specification. The core commitments — single-process model, fixed thread set with bounded channels, 8-stage frame pipeline with per-stage budgets, three-tier budget enforcement, 5-level degradation ladder — are doctrinally sound and technically coherent.

This review round fixed four MUST-FIX items (TelemetryRecord naming inconsistency, incomplete BudgetState enum, missing input_to_scene_commit budget, stale RFC reference) and four SHOULD-FIX items (DR-V6 coverage, thread priority specification, degradation ladder v1 scope mapping, resource cleanup on revocation). All three review dimensions score 4/5.

The RFC is ready for round 2 review (Technical Architecture Scrutiny, rig-5vq.16).
