# Degradation Cadence and Production Wiring Seam

Date: 2026-07-16

Issue: `hud-cpj4v`

Source finding: `hud-48s45` F7

Status: **decision required before behavior code**

## Purpose

Define the smallest missing contract needed to wire measured production frame
telemetry into the runtime-owned degradation controller and then into compositor
quality, shedding, observability, and session notices. This is Windows-runtime
efficiency reconciliation. It does not add a smart-glasses, VR, 90 Hz, or 120 Hz
implementation or validation lane.

The production consumer is currently absent:

```text
presented FrameTelemetry
        X
runtime::DegradationController::record_frame
        X
explicit runtime -> compositor policy mapping
        X
FrameTelemetry current level + transition event + DegradationNotice
```

The X marks are real missing call sites, not inferred behavior. Repository-wide
search finds `DegradationController` only in its definition, tests, and public
re-export. `Compositor::degradation_level` is initialized to `Nominal` and read by
widget interpolation, but production runtime code never changes it. There is also
a second, independently named `tze_hud_scene::DegradationTracker` with its own
frame windows, transition reset semantics, shedding indices, and
`tze_hud_scene::DegradationLevel`; it too has no production consumer. The
compositor field uses that scene-side level type, while the runtime controller
returns `tze_hud_runtime::DegradationLevel`. A production implementation must not
run both trackers or cast between their enums by ordinal.

## Why Implementation Is Blocked

The fixed 60 Hz contract is implementable, but the requested cadence-aware
contract is not defined. Choosing a formula in code would create a new normative
budget without an approved source.

| Surface | Current authority | Conflict or omission |
|---|---|---|
| Entry threshold | RFC 0002 section 6.1 and `runtime-kernel` OpenSpec: p95 > 14 ms | No rule derives the threshold from a presentation period. At a shorter period, 14 ms can be later than the missed-frame boundary. |
| Recovery threshold | RFC 0002 section 6.3 and OpenSpec: p95 < 12 ms | No cadence-derived recovery threshold or headroom ratio is specified. |
| Entry window | 10 frames, described as about 166 ms at 60 fps | It is unclear whether frame count or elapsed time is normative when cadence changes. |
| Recovery window | 30 frames, described as 500 ms at 60 fps | It is unclear whether frame count or elapsed time is normative when cadence changes. |
| Sampling | Evaluate after every frame | The production idle gate skips build, encode, present, and telemetry for unchanged scenes. No contract says whether an idle interval is a clean sample, no sample, or a recovery deadline. A controller wired only to presented telemetry can remain degraded forever after the scene becomes quiescent. |
| Measurement boundary | `FrameTelemetry::frame_time_us` is documented as Stage 1 start through Stage 7 completion | The generic pipeline and headless path time that boundary, but the windowed path starts `frame_start` inside the compositor loop; main-thread Stage 1/2 work is not timed by that clock. The follow-on must either carry the real Stage 1 start across the thread boundary or define and approve a distinct compositor-workload trigger. It must not silently treat the current windowed value as proven end-to-end Stage 1-7 time. |
| Transition application | Response within one frame | The contract does not state whether telemetry for frame N reports the policy applied to N or the policy selected for N+1. |
| Decision authority | Runtime-owned degradation | `tze_hud_runtime::DegradationController` and `tze_hud_scene::DegradationTracker` independently implement thresholds, windows, resets, and shedding. Neither is wired to production, and their level names/types differ. One must become the sole transition authority; the other must be removed or reduced to a pure helper with no independent frame history. |
| Quality semantics | Six named levels | `Compositor::degradation_level` currently affects widget transition interpolation only. Level 2 texture scaling, Level 3 transparency simplification, and Level 4/5 priority shedding lack one production compositor policy object and complete render-path consumers. |
| Level 1 ownership | RFC 0002: coalesce outbound state-stream `SceneEvent` fan-out only | This is a runtime/session event-lane action, not a compositor render-quality action. Inbound `MutationBatch` atomicity is unchanged. |
| Protocol mapping | RFC 0005 `DegradationNotice` has seven media-era values | Runtime-kernel has six compositor levels. The exact mapping and affected-capability vocabulary for Level 2/3/5 are not specified. |
| Notice backpressure | RFC 0002: `DegradationNotice` is transactional and the compositor blocks when its bounded lane is full | The current `HudSessionImpl` uses `tokio::sync::broadcast`; a lagging receiver returns `Lagged(n)` and the implementation continues after losing notices. `broadcast_degradation` also has no production caller. This is not the RFC-defined transactional lane. The follow-on must route notices through a bounded never-drop path with defined backpressure, or amend the ownership/channel contract. |
| New-session state | Active sessions receive transition notices | The current service stores a level and comments that new sessions can observe it, but no production handshake path reads that field. The RFC does not define whether or when a newly joining or reconnecting session receives a current-level notice relative to `SessionEstablished` and snapshot delivery. |

The idle ambiguity is observable even at the current Windows 60 Hz target, so
limiting the implementation to 60 Hz does not eliminate every correctness gap.

## Invariants Already Decided

The follow-on implementation must preserve these existing contracts regardless
of the option selected below:

1. The runtime owns degradation decisions. Agents cannot initiate them and the
   `tze_hud_policy` crate is not inserted into the frame loop.
2. Exactly one object owns frame-history and level transitions. The runtime
   controller is the proposed authority. The scene crate may expose pure
   priority/suppression helpers, but must not retain an independent degradation
   level or sample window in the production path.
3. Measurement uses the production `FrameTelemetry::frame_time_us` schema only
   after the windowed producer is proven to span Stage 1 start through Stage 7
   completion. Carry the authoritative start timestamp across threads or amend
   the trigger contract explicitly; synthetic counters and an undocumented
   compositor-loop proxy are not acceptable.
4. A transition selected from frame N can affect frame N+1 at the earliest; the
   telemetry record for N reports the policy actually applied while rendering N.
5. Level 1 changes outbound state-stream event fan-out only. Levels 2-5 change
   render policy only. No level changes authoritative scene or lease state;
   shedding does not delete tiles or revoke leases.
6. Chrome and human override controls remain renderable at every level.
7. Each evaluation can move exactly one level and entry/recovery retain a
   hysteresis band. Whether samples reset after a transition, and therefore when
   another level becomes eligible, is part of the decision below.
8. Every presented frame carries the active degradation level in the same
   machine-readable telemetry schema used by production and CI. Each transition
   also emits a structured tracing event with previous level, new level,
   direction, triggering p95, sample duration/count, and target cadence.
9. Session notices are transactional and active sessions receive each approved
   transition mapping. New-session/reconnect ordering must be decided explicitly;
   it is not asserted by this seam.
10. No implementation in this lane creates device targets, stereo/multiview
   surfaces, or 90/120 Hz test lanes. A generic formula applied to an existing
   Windows `--fps` value is not evidence of device support, and no new cadence is
   claimed validated unless an authorized Windows validation lane covers it.

## Proposed Production Seam

Once the decision below is approved, ownership should be wired as follows:

| Step | Owner | Contract |
|---|---|---|
| 1. Measure | `tze_hud_runtime::windowed` and `HeadlessRuntime` | Complete a presented frame and populate `FrameTelemetry` from the real Stage 1-7 path, including a cross-thread Stage 1 start for windowed production or an approved replacement metric. |
| 2. Evaluate | `tze_hud_runtime::DegradationController` | Consume the completed measurement plus the immutable cadence/budget envelope. Return at most one transition. Retire the scene-side tracker's independent frame history from production. |
| 3. Select next policy | runtime compositor loop | Exhaustively map the runtime level into one project-owned compositor policy for frame N+1. No independent level trackers and no numeric enum casts. |
| 4. Apply Level 1 | runtime/session event lane | Coalesce outbound state-stream fan-out by the approved ratio; never coalesce inbound mutation batches or transactional events. |
| 5. Apply Levels 2-5 | `tze_hud_compositor` | Apply texture quality, transparency, and tile visibility as render-only decisions. |
| 6. Observe | telemetry + `tracing` | Record the policy applied to each frame and a complete transition event. |
| 7. Notify | protocol session authority | Enqueue the approved mapping through a bounded never-drop transactional path and preserve RFC-defined backpressure unless an amendment authorizes another handoff. Do not reuse the current lossy broadcast as-is. |

The internal runtime-to-compositor mapping is mechanically determined by the
six RFC 0002 rungs and should be encoded as an exhaustive match:

| Runtime controller | Compositor policy | Production action |
|---|---|---|
| `Normal` | `Nominal` | Full quality |
| `Coalesce` | `Minor` | No compositor quality change; Level 1 acts only on outbound state-stream fan-out |
| `ReduceTextureQuality` | `Moderate` | Level 2 texture-quality policy |
| `DisableTransparency` | `Significant` | Level 3 transparency/transition simplification |
| `ShedTiles` | `ShedTiles` | Level 4 priority-ordered render suppression |
| `Emergency` | `Emergency` | Level 5 chrome plus highest-priority tile |

This internal mapping is separate from the unresolved seven-value
`DegradationNotice` mapping. The protocol values include media-era rungs that do
not correspond one-for-one with the v1 compositor ladder.

The compositor policy should be an explicit value rather than scattered
comparisons against a public enum field. A Level 4/5 suppression set must be
computed under the frame N+1 scene lock from that frame's lease priorities and
z-orders, then carried in the scene-free frame build. It must be bound to the
scene version/geometry epoch used for that build, not cached across mutations.
The set must use stable tile `SceneId`s, not positional indices such as those
stored by the current scene-side tracker. Priority, z-order, tile identity, and
the chosen policy level must be snapshotted atomically; a tile created, removed,
re-leased, or reordered after that snapshot must force recomputation before it
can be rendered under the policy.
This gives grep-verifiable production consumers, keeps priority sorting out of
every node/text/widget traversal, and prevents a stale suppression decision.

## Decision Options

### Option A — Cadence-derived immutable budget envelope (recommended)

Create an immutable runtime budget envelope at startup from the selected display
profile and effective Windows presentation cadence. The contract must define:

- the cadence authority and precedence among resolved display profile, CLI/env
  `--fps`, monitor refresh, and measured presentation timing;
- the exact entry threshold as a function of presentation period;
- the exact recovery threshold and hysteresis band;
- entry and recovery windows as elapsed monotonic durations, including minimum
  sample counts so a few slow frames cannot overrepresent a long idle interval;
- the p95 algorithm for sparse/time-window samples and the post-transition
  sample-reset/re-eligibility rule;
- the idle/quiescent recovery rule;
- which absolute latency ceilings remain independent of cadence; and
- the six-level runtime-to-session-notice mapping.

The existing 60 Hz values must be fixed calibration points: the formula evaluates
to 14 ms entry, 12 ms recovery, about 166 ms entry duration, and 500 ms recovery
duration at 60 Hz. Code should be implemented only after the ratios/margins and
idle rule are normative in an OpenSpec delta plus RFC 0002 amendment.

Advantages: one operational authority, honest non-default `--fps` behavior,
time-correct hysteresis, and direct alignment with F6/F7. Cost: requires a small
contract decision before implementation. The formula may apply generically to
existing Windows cadence configuration, but this change neither adds nor claims
validation for a 90/120 Hz device lane.

### Option B — Lock v1 production degradation to 60 Hz

Make 60 Hz a validated startup invariant for this policy and reject startup (or
the unsupported non-60-Hz configuration) at other effective cadences until a
future spec defines their budgets. Do not silently run with automatic
degradation disabled: the ladder is v1-mandatory and such a runtime would claim
a protection it does not provide. Keep 14/12 ms and 10/30 presented-frame
windows, and add a specific idle recovery rule, transition sample-reset rule,
transactional notice mapping, and new-session/reconnect ordering rule. Option B
does not need a new threshold formula or time-window p95 algorithm.

Advantages: smallest contract change and exactly preserves RFC 0002 numbers.
Cost: conflicts with the existing configurable `--fps` surface unless that
surface is narrowed; it postpones rather than resolves cadence-derived budgets.

### Option C — Keep fixed thresholds at every configured cadence

Wire the existing controller unchanged and apply 14/12 ms plus 10/30 frames for
all `target_fps` values.

Rejected. This makes `--fps` change the real duration of hysteresis and allows a
threshold later than the presentation deadline at sufficiently high cadence. It
would encode the desktop-headroom assumption identified by F7.

### Session-current-state ordering choices

The approved option must also choose how a session learns the already-active
level. The safe default is to send the current mapped notice after the
`SceneSnapshot` on both new handshake and accepted resume, before ordinary
incremental events. This preserves RFC 0005's `SessionEstablished`/resume-result
then snapshot ordering and gives the agent a coherent baseline before notices.
Adding a current-level field to `SessionEstablished` and `SessionResumeResult` is
also coherent but requires a protocol amendment. Omitting current state until a
later transition is rejected because a session joining during Level 4/5 would
have no machine-readable indication of the policy already affecting its tiles.

### Runtime-to-protocol mapping choices

The current seven-value enum cannot truthfully represent all six v1 compositor
rungs. `MEDIA_QUALITY_REDUCED` is not an exact name for static texture scaling,
and `AUDIO_ONLY_FALLBACK` is false for Level 5, which still renders chrome plus
one tile. The decision must select one of these protocol amendments:

1. **Append exact enum values (recommended).** Preserve all existing numeric
   values and append compositor-specific values for texture-quality reduction
   and emergency rendering. Map Normal, Coalesce, DisableTransparency, and
   ShedTiles to their existing exact semantic values; map Levels 2 and 5 to the
   new appended values. This keeps one primary discriminant and makes the wire
   transition lossless, at the cost of an enum/schema update.
2. **Add an explicit runtime-level/action field.** Keep the legacy seven-value
   enum as a compatibility summary and add an append-only numeric runtime level
   plus typed action codes. This is more verbose and creates two fields that
   receivers must reconcile, but preserves old enum behavior.

Reusing `MEDIA_QUALITY_REDUCED` for Level 2 and
`AUDIO_ONLY_FALLBACK`/`SHEDDING_TILES` for Level 5 without another exact field is
rejected: it is either semantically false or collapses two observable rungs.
Likewise, `affected_capabilities` must not be filled with invented action names
such as `texture_quality` or `tile_visibility`; those are not capability grants.
For the v1 render-only ladder the safe default is an empty capability list unless
an actual granted capability is narrowed. The level/action field carries the
render-policy effect, while the human-readable reason remains explanatory only.

## Required Decision

Approve Option A or B.

Option A must provide:

1. Cadence authority/precedence and entry threshold formula.
2. Recovery threshold formula and hysteresis band.
3. Time-window semantics, p95 calculation, minimum samples, and
   post-transition sample reset/re-eligibility.
4. Idle/quiescent recovery behavior:
   - treat elapsed quiescence as clean and recover on deadlines;
   - render bounded recovery probes; or
   - another explicit rule.
5. Runtime level to `DegradationNotice` value/capability mapping.
6. Transactional notice handoff/backpressure and new-session/reconnect ordering,
   including replacement of the current lag-dropping broadcast behavior.

Option B must provide only the 60 Hz startup/configuration restriction plus
items 4-6 and the 10/30-frame post-transition sample-reset/re-eligibility rule.

For item 5, the default recommendation is the append-only exact enum option and
an empty `affected_capabilities` list for render-only transitions. If neither
protocol amendment is approved, the safe default is to leave production wiring
blocked rather than publish a misleading degradation value.

Default recommendation: **Option A**, with quiescent elapsed time eligible for
recovery only after the runtime proves no animation, publication expiry,
reveal, scroll, composer-caret, resize, or other scheduled render deadline is
pending; do not synthesize zero-time frames. A long quiescent interval may
recover at most one level per approved recovery duration. Keep absolute
input-to-local-ack and
input-to-scene-commit ceilings unchanged. The exact cadence margins remain an
owner/spec decision and are intentionally not proposed as implementation facts
here.

## Follow-On Acceptance Evidence

The implementation bead unblocked by this decision must produce all of the
following evidence:

1. `rg` output proving production calls to controller evaluation, compositor
   policy application, telemetry level emission, transition tracing, and session
   notice publication, plus proof that no second stateful degradation tracker is
   active in the frame path.
2. Deterministic pure tests with injected cadence/time covering transient spike,
   sustained entry, one-level transition, hysteresis, quiescent recovery, full
   Level 5 recovery, and approved post-transition re-eligibility.
3. Behavior tests proving the compositor consumes the policy: widget transition
   simplification, texture-quality action, transparency action, priority-ordered
   Level 4 shedding, Level 5 chrome plus one tile, and restoration without scene
   or lease mutation.
4. Telemetry serialization tests proving per-frame active level and structured
   transition fields, including backward-compatible defaults.
5. Protocol tests proving queue-full transactional behavior, active-session
   delivery without `Lagged` loss, and the approved new-session/reconnect
   ordering and mapping.
6. A bounded production-path exercise that feeds real windowed/headless
   `FrameTelemetry` through the controller and asserts N-to-N+1 policy ordering,
   transition latency, quiescent recovery, and headless/windowed semantic parity.
   It must prove the windowed trigger measurement begins at the approved Stage 1
   boundary (or at the explicitly amended replacement boundary), not merely at
   the compositor-loop iteration start.
   The named sustained-load payload must also run in release mode with timing
   assertions and structured output.
7. Full `cargo check --workspace`, `cargo clippy --workspace --all-targets --
   -D warnings`, runtime/compositor focused tests with the headless GPU mutex, and
   the separate integration package.

## Traceability

- Doctrine: `about/heart-and-soul/efficiency.md` (idle cost,
  change-proportional work, designed degradation)
- Doctrine: `about/heart-and-soul/failure.md` (runtime-owned ordered ladder)
- Doctrine: `about/heart-and-soul/validation.md` DR-V3 and Layer 3
- RFC: `about/legends-and-lore/rfcs/0002-runtime-kernel.md` sections 6.1-6.4
- RFC: `about/legends-and-lore/rfcs/0005-session-protocol.md` sections 1.3,
  2.5, 3.4, 5.1, and 6.4-6.5
- OpenSpec: `openspec/specs/runtime-kernel/spec.md` requirements Degradation
  Ladder, Trigger, Hysteresis, and Tile Shedding Order
- OpenSpec: `openspec/specs/validation-framework/spec.md` requirements Layer 3
  and DR-V3 Structured Telemetry
- OpenSpec: `openspec/specs/session-protocol/spec.md` requirement
  DegradationNotice Delivery
- Finding: `docs/reports/hud-48s45_desktop_headroom_assumption_audit_20260716.md`
  F6-F7
