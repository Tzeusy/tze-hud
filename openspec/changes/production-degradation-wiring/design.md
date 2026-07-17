## Context

`tze_hud_runtime::DegradationController` exists but is not called by either production frame loop. The compositor exposes only a scene-owned level enum, the scene crate also contains a second stateful tracker, `FrameTelemetry::frame_time_us` does not have the claimed Stage-1 provenance in the windowed loop, and the session server uses lag-dropping broadcast for a transactional message. PR #1182 documented those seams; owner decision `hud-o5snv` approved Option A.

The implementation must preserve runtime sovereignty, unchanged authoritative scene/lease state, the idle no-present contract, stable scene identities, and the Windows-only execution boundary.

## Goals / Non-Goals

**Goals:**

- Make one runtime controller own frame history and transitions.
- Derive immutable thresholds and elapsed windows from the effective Windows cadence while reproducing 14/12 ms and 10/30 presentation opportunities at 60 Hz.
- Apply each transition to the next frame through one explicit compositor policy whose suppression inputs are atomically snapshotted.
- Make degradation telemetry and session notice delivery machine-verifiable and lossless.

**Non-Goals:**

- Adding a 90/120 Hz, glasses, VR, stereo, media-stream, or device-validation lane.
- Mutating tiles, leases, or agent capabilities as a render-degradation action.
- Running `tze_hud_policy` or the scene-side stateful tracker in the frame loop.
- Synthesizing idle frames to recover.

## Decisions

### Authoritative cadence and immutable envelope

The effective cadence is the validated runtime `target_fps` after CLI/config profile resolution. A known monitor refresh caps that value; an unknown refresh does not invent a cap. Measured present intervals are observability only and never mutate the startup envelope.

For integer presentation period `P = floor(1_000_000 / effective_fps)` microseconds:

- entry threshold `E = min(14_000, ceil(21 * P / 25))`;
- recovery threshold `R = min(12_000, ceil(18 * P / 25))`;
- entry duration `10 * P`, minimum 10 presented samples;
- recovery duration `30 * P`, minimum 30 presented samples.

Thus 60 Hz yields exactly 14,000 us, 12,000 us, about 166 ms, and about 500 ms. The absolute 14/12 ms ceilings remain in force at slower cadences; shorter periods derive thresholds that remain ahead of the presentation deadline. Arithmetic is checked integer arithmetic and the envelope is frozen at startup.

Alternatives considered:

- Carry a main-thread Stage-1 timestamp into every compositor frame. Rejected for this change because the event and compositor clocks do not share a one-to-one frame boundary, and coupling them would enlarge a hot cross-thread seam.
- Adapt thresholds from measured vsync intervals. Rejected because jitter would mutate policy authority at runtime and make deterministic tests impossible.

### Degradation workload metric

Add `FrameTelemetry::degradation_work_time_us`, measured from the beginning of active runtime/compositor work (before Stage 3/resize/scene work) through successful Stage 7 completion. Both headless and windowed production paths populate the same field. `frame_time_us` keeps its existing Stage-1-to-Stage-7 meaning and is not silently reinterpreted.

The controller consumes only `degradation_work_time_us`. Samples are `(completed_at_mono, duration_us)`. Nearest-rank p95 is computed over samples inside the applicable elapsed window. The first sample covers one cadence period ending at its completion; eligibility requires both elapsed duration and the minimum sample count. Samples reset after every transition, so one burst cannot cascade multiple levels.

### Quiescent recovery

The frame scheduler reports quiescence only when the same predicate that suppresses rendering proves there is no scene/geometry change, animation, publication expiry, reveal, scroll, composer caret, resize, capture, benchmark, or other scheduled presentation deadline. No zero-time telemetry record or synthetic present is emitted. From the first proven-quiescent instant, the controller may recover one level after each full recovery duration. Active work clears quiescence and recovery never advances degradation.

### Atomic compositor policy

The runtime maps its six levels exhaustively to one compositor policy value. While holding the scene lock for frame N+1, it snapshots the chosen level plus each active tile's stable `SceneId`, lease priority, and z-order, computes the suppression set, and builds all scene-free draw inputs from that policy. Any scene version or geometry epoch change causes recomputation. Chrome is outside the suppression set.

Level 1 affects outbound state-stream fan-out only. Levels 2-5 affect rendering: large image textures use the configured lower-resolution cache, transparency/transition work is disabled, Level 4 suppresses the least-important quartile, and Level 5 preserves chrome plus one highest-priority tile. Restoration changes only the policy.

### Transactional notice hub and ordering

Replace degradation broadcast with a project-owned hub containing a bounded per-session queue. Publication blocks on a full live queue, removes closed subscribers, and cannot return success after dropping a notice. After `SessionEstablished` or accepted `SessionResumeResult`, the server sends `SceneSnapshot`, then atomically subscribes and obtains the current mapped notice, sends that current state, and only then drains later transitions. The subscribe/current operation and publication share one registry lock, preventing a transition gap.

### Append-only protocol mapping

Keep values 0-7 unchanged and append `TEXTURE_QUALITY_REDUCED=8` and `EMERGENCY_RENDERING=9`. Runtime mappings are exact and exhaustive. `DEGRADATION_LEVEL_UNSPECIFIED=0` is rejected/default-blocked by policy conversion. Render-only transitions carry an empty `affected_capabilities` list.

## Risks / Trade-offs

- [Bounded notice backpressure can stall the compositor behind an unresponsive session] -> This is the existing transactional contract; queue capacity and structured stall telemetry make the fault visible, and closed subscribers are pruned.
- [Changing image quality can invalidate GPU caches] -> Cache invalidation occurs only on level transitions, never per frame; raw content-addressed bytes remain authoritative.
- [A stale suppression set could hide the wrong tile] -> The set is stable-`SceneId` based and built from the same locked scene version/geometry epoch as all frame inputs.
- [Quiescence could recover while hidden work remains] -> Only the canonical no-render predicate can report quiescence; any deadline or mutation clears it.

## Migration Plan

1. Land strict-valid OpenSpec/RFC amendments and append protocol values.
2. Add deterministic controller, policy, telemetry, and notice-hub tests that fail against the unwired baseline.
3. Wire headless and windowed production consumers and prove all call sites by repository search.
4. Run focused crates, integration, full workspace check/clippy/tests, and release-mode sustained-load evidence.

Rollback is a normal commit revert: existing numeric protocol values remain compatible, and older receivers treat appended enum values as unknown rather than misreading an existing value.

## Open Questions

None. Owner decision `hud-o5snv` authorizes this contract.
