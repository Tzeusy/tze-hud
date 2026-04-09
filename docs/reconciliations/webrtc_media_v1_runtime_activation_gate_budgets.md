# WebRTC/Media V1 Runtime Activation Gate + Budgets Contract (WM-S3)

Date: 2026-04-09
Issue: `hud-nn9d.10`
Parent epic: `hud-nn9d`
Depends on: `hud-nn9d.6` (WM-S1), `hud-nn9d.7` (WM-S2a), `hud-nn9d.8` (WM-S2b)

## Purpose

Define the single normative gate that controls when post-v1 bounded media ingress
is allowed to activate at runtime, and the quantitative budgets that keep that
path bounded.

This contract is normative for:

1. activation prerequisites and default-off behavior,
2. measurable runtime admission and teardown budgets,
3. coupling between degradation state and media ingress behavior,
4. implementation guardrails that forbid out-of-gate enablement.

This contract does not replace privacy/operator policy details (WM-S3b),
compositor rendering semantics (WM-S3c), or validation suite thresholds (WM-S4).

## Inputs And Existing Constraints

1. v1 doctrine keeps media/WebRTC deferred and default-off
   (`about/heart-and-soul/v1.md:120`).
2. Runtime kernel keeps media worker pool deferred in v1 and defines
   `DecodedFrameReady` channel capacity as 4 per stream
   (`openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:383`).
3. Lease budget schema carries `max_concurrent_streams`, with v1 default `0`
   (`openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md:161`).
4. Runtime degradation trigger/recovery windows are already specified:
   trigger at `frame_time_p95 > 14ms` over 10 frames, recover at
   `frame_time_p95 < 12ms` over 30 frames
   (`openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:247`,
   `:260`).
5. WM-S2b defines post-v1 signaling/schema additions but does not define
   activation budgets (`docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md`).

## Gate Decision: Explicit Default-Off With All-Prereqs Admission

Runtime media ingress MUST remain disabled by default. Activation is allowed only
through a single deterministic gate that requires all prerequisites below.

Any missing prerequisite MUST deny activation and MUST leave worker-pool spawn,
decode, and ingress admission disabled.

## Activation Prerequisites (All MUST hold)

1. **Contract readiness gate**
- WM-S1, WM-S2a, and WM-S2b artifacts are approved and available in the runtime's
  contract set.
2. **Protocol/version gate**
- Session negotiation advertises the WM-S2b media message envelope support.
3. **Capability + lease gate**
- Session capability grant includes media-ingress authority for target scope.
- Active lease budget for the publisher has `max_concurrent_streams >= 1`.
4. **Config/profile gate**
- Resolved display/runtime profile allows media (`max_media_streams >= 1`).
- Explicit operator/runtime enablement for media ingress is `enabled`
  (default remains disabled).
5. **Performance headroom gate**
- At activation check time, runtime is not in severe degradation:
  `degradation_level <= 1`.
- Latest 30-frame recovery window satisfies `frame_time_p95 < 12ms`.
6. **Observability gate**
- Runtime can emit activation/admission/teardown telemetry for media ingress
  decisions and budget outcomes.

## Quantitative Budget Contract

For the bounded ingress slice, runtime MUST enforce all numeric limits below:

1. **Concurrent stream hard limit**
- Effective active stream limit is:
  `min(1, lease.max_concurrent_streams, profile.max_media_streams)`.
- Therefore bounded slice admits at most **1** active inbound stream.
2. **Per-stream frame queue bound**
- `DecodedFrameReady` queue depth is **4** per active stream.
3. **Admission under pressure**
- New media ingress admission MUST be denied when the runtime overbudget trigger
  is active (`frame_time_p95 > 14ms` over rolling 10-frame window).
4. **Teardown latency**
- On lease revoke, operator disable, or budget/degradation forced teardown,
  active media presentation MUST stop within **1 compositor frame**
  (target 16.6ms at 60fps).
5. **Re-enable hysteresis**
- After forced teardown from budget/degradation pressure, re-admission MUST stay
  denied until recovery window is satisfied (`frame_time_p95 < 12ms` over
  rolling 30-frame window) and degradation level is back to `<= 1`.

## Degradation Coupling Contract

Media ingress behavior MUST couple to the runtime degradation ladder:

1. **Levels 0-1**: media ingress may run if other gates pass.
2. **Levels 2-3**: media ingress remains admitted but MUST be marked degraded
   and runtime MAY apply quality reduction/drop policy.
3. **Levels 4-5**: media ingress MUST be forcibly torn down and new admissions
   MUST be denied until recovery criteria are met.

This preserves lease-state semantics (leases remain valid unless explicitly
revoked) while ensuring media decode/render load cannot bypass degradation
safety.

## No-Enable-Outside-Gate Rule (Implementation Guardrail)

1. Media worker pool spawn MUST occur only after the activation gate returns
   allow=true.
2. Any path that attempts media ingress open while gate=false MUST return a
   deterministic structured denial and MUST NOT create decode workers.
3. Feature flags, test harnesses, and direct runtime entry points MUST all route
   through the same gate evaluation path.
4. In v1-default config (`max_concurrent_streams = 0`, media disabled), gate
   result MUST be deny.

## Contract Validation Scenarios (Normative)

1. **Default boot is media-disabled**
- **WHEN** runtime starts with v1 defaults
- **THEN** media worker pool is not spawned and media ingress open requests are denied

2. **Missing prerequisite denies activation**
- **WHEN** any single prerequisite gate above is false
- **THEN** activation is denied with deterministic reason metadata and media remains disabled

3. **Performance headroom failure denies admission**
- **WHEN** runtime has `frame_time_p95 > 14ms` over the current 10-frame window
- **THEN** new media ingress admission is denied even if capability and config gates are true

4. **Bounded slice rejects second stream**
- **WHEN** one inbound stream is active and another open request arrives
- **THEN** second request is rejected by concurrent-stream hard limit

5. **Level-4 degradation tears down active media**
- **WHEN** degradation level advances to 4 with active media ingress
- **THEN** media presentation is torn down within one compositor frame
- **AND** new admissions remain denied until recovery window passes

6. **Operator disable overrides active stream**
- **WHEN** operator disable is set while stream is active
- **THEN** stream is torn down within one compositor frame and gate remains deny until explicitly re-enabled

## Acceptance Traceability (`hud-nn9d.10`)

1. Activation criteria are measurable:
- fulfilled by explicit prerequisite list with numeric degradation windows.
2. Default-off behavior remains explicit:
- fulfilled by gate decision and v1-default denial rule.
3. Quantitative budgets and degradation coupling are specified:
- fulfilled by hard-limit table and level-coupling contract.
4. No implementation may enable media outside this gate:
- fulfilled by guardrails requiring one shared gate path for all media enablement.
