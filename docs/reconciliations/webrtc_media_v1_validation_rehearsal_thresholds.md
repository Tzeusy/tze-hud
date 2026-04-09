# WebRTC/Media V1 Validation Rehearsal Scenarios + Thresholds Contract (WM-S4)

Date: 2026-04-09
Issue: `hud-nn9d.13`
Parent epic: `hud-nn9d`
Depends on: `hud-nn9d.6` (WM-S1), `hud-nn9d.8` (WM-S2b), `hud-nn9d.9` (WM-S2c), `hud-nn9d.10` (WM-S3), `hud-nn9d.11` (WM-S3b), `hud-nn9d.12` (WM-S3c)

## Purpose

Define the validation contract for the bounded post-v1 media ingress slice:
required rehearsal scenes, quantitative pass/fail thresholds, explicit
headless-vs-real-decode strategy, privacy/operator test coverage, and CI-visible
result outputs.

This contract is normative for validation readiness only. It does not redefine
signaling/schema, zone transport policy, activation gates, privacy policy, or
compositor render semantics already specified by WM-S2b, WM-S2c, WM-S3, WM-S3b,
and WM-S3c.

## Inputs And Existing Constraints

1. Validation framework defines five layers and existing visual/performance
   thresholds, including SSIM thresholds (`0.995` layout, `0.99` media
   composition), and CI artifact requirements
   (`openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md:54`,
   `:119`).
2. Runtime/media activation gate and teardown requirements are already defined:
   bounded to one stream, deny under pressure, teardown within one compositor
   frame (`docs/reconciliations/webrtc_media_v1_runtime_activation_gate_budgets.md:74`).
3. Privacy/operator admission precedence and observability requirements are
   defined in WM-S3b
   (`openspec/specs/media-webrtc-privacy-operator-policy/spec.md`).
4. Compositor render-state and fallback behavior are defined in WM-S3c
   (`docs/reconciliations/webrtc_media_v1_compositor_videosurfaceref_contract.md`).

## Validation Strategy Decision: Dual-Lane Rehearsal

Bounded media ingress validation MUST run in two explicit lanes:

1. **Lane A: Headless synthetic-media lane (CI merge gate, mandatory)**
- Runs on software or headless-capable backends (llvmpipe/WARP/Metal CI).
- Uses deterministic synthetic media sources; no live network WebRTC peer needed.
- Produces deterministic pass/fail signals for contract invariants.

2. **Lane B: Real-decode lane (nightly or pre-release rehearsal, mandatory for tranche signoff)**
- Runs with real decode stack and transport handshake on designated runners.
- Validates decode-path behavior, stalls, and resource pressure not exercised by synthetic lane.
- Failures block signoff for implementation readiness but do not replace Lane A CI gating.

No implementation bead may claim bounded-ingress readiness unless Lane A passes
and Lane B has an executed result for the same contract version.

## Required Rehearsal Scene Matrix

The bounded-ingress validation matrix MUST include, at minimum, these scenes:

1. `media_single_stream_nominal`
- Purpose: baseline admit + present + steady render in approved zone.

2. `media_second_stream_rejected`
- Purpose: enforce one-stream hard limit.

3. `media_timing_present_and_expiry`
- Purpose: no-early-present and expiry cut-off semantics.

4. `media_lease_revocation_teardown`
- Purpose: lease revoke triggers deterministic teardown.

5. `media_operator_disable_teardown`
- Purpose: operator override takes precedence and tears down active stream.

6. `media_privacy_policy_denial`
- Purpose: missing/insufficient classification or viewer ceiling mismatch denies admission.

7. `media_degradation_level4_forced_teardown`
- Purpose: degradation coupling forces teardown and denies re-admission until recovery.

8. `media_zone_transport_or_identity_rejection`
- Purpose: reject ingress outside approved media zone/transport contract.

9. `media_reconnect_epoch_reconcile`
- Purpose: snapshot-first reconnect behavior preserves declarative state but requires epoch reconcile/fresh open.

Lane A MUST run scenes 1-9 with synthetic sources.
Lane B MUST run scenes 1, 3, 4, 5, 7, and 9 with real decode enabled.

## Quantitative Pass/Fail Thresholds

### Functional thresholds (hard pass/fail in Lane A and Lane B where applicable)

1. **Concurrent-stream limit**
- Exactly 1 admitted stream maximum; any second concurrent open MUST be denied.

2. **Timing conformance**
- No frame may be presented before `present_at_wall_us`.
- No frame may be presented at/after `expires_at_wall_us`.

3. **Teardown latency**
- Lease revoke, operator disable, and level-4/5 degradation events MUST remove
  active media visibility within 1 compositor frame (target 16.6ms at 60fps).

4. **Policy precedence**
- Disabled policy or operator-disable state MUST deny admission before transport/decode checks.

5. **Zone/transport containment**
- Any target outside approved zone contract MUST be denied with deterministic
  contract/policy reason.

### Visual thresholds (Layer 2)

1. Layout/regression scenes: SSIM >= `0.995`.
2. Media composition scenes: SSIM >= `0.99`.
3. Synthetic headless pixel tolerances: +/-1 for solid fills, +/-2 for blend
   channels (software GPU tolerance).

### Performance thresholds (Layer 3)

1. Normalized `frame_time_p99 < 16.6ms` for nominal media rehearsal scenes.
2. `input_to_next_present_p99 < 33ms` on interactive rehearsal scenes.
3. `input_to_scene_commit_p99 < 50ms` for local agent-driven control flows.
4. Zero lease violations and zero budget-overrun correctness violations in the
   rehearsal session aggregate.

Performance thresholds above are valid pass/fail only when calibration factors
are available per validation-framework contract. If calibration is unavailable,
performance outputs MUST be marked `uncalibrated` and treated as warning status.

## Privacy/Operator Test Contract

WM-S4 validation MUST include explicit policy tests:

1. classification missing -> denied,
2. viewer/privacy ceiling mismatch -> denied,
3. operator disable while active -> teardown within one frame,
4. operator re-enable does not auto-resume prior stream,
5. disabled enablement policy short-circuits admission before transport checks.

Each case MUST emit a reason-coded structured decision event and MUST NOT leak
raw media payload bytes in telemetry artifacts.

## CI-Visible Outputs Contract

Every WM-S4 run MUST produce machine-readable outputs that can gate CI:

1. `media_ingress_validation_summary.json`
- Contains contract version, lane, scenario list, per-scenario status
  (`pass`/`fail`/`warn-uncalibrated`), metrics, and reason codes.

2. `media_ingress_validation.junit.xml`
- One test case per rehearsal scenario with deterministic failure messages.

3. Layer-4 artifacts under `test_results/{timestamp}-{branch}/`
- At least: per-scene rendered/golden/diff assets (when applicable),
  telemetry JSON, and explanation markdown.

4. CI step verdict rule
- `fail` status in any mandatory scenario => CI fail.
- only `pass` + optional `warn-uncalibrated` in designated performance fields
  => CI pass-with-warning annotation.

## Contract Validation Scenarios (Normative)

1. **Headless lane provides deterministic gate signal**
- **WHEN** Lane A executes on CI runners
- **THEN** all required scenes produce machine-verifiable pass/fail outputs without physical GPU requirements

2. **Real-decode lane is explicit and non-optional for signoff**
- **WHEN** bounded-ingress tranche seeks implementation-readiness signoff
- **THEN** Lane B results for the current contract revision MUST be present and auditable

3. **Privacy/operator coverage is first-class, not optional**
- **WHEN** WM-S4 suite executes
- **THEN** privacy denials and operator override scenarios MUST run as mandatory checks

4. **CI verdict derives from structured outputs**
- **WHEN** the validation run completes
- **THEN** CI pass/fail state MUST be computed from summary/JUnit artifacts rather than ad hoc log parsing

## Acceptance Traceability (`hud-nn9d.13`)

1. Validation scenes and thresholds are specified:
- fulfilled by the required scene matrix and quantitative threshold sections.
2. Headless-vs-real-decode strategy is explicit:
- fulfilled by dual-lane strategy with mandatory lane responsibilities.
3. Privacy/operator tests are in the contract:
- fulfilled by dedicated privacy/operator test contract section.
4. CI-visible pass/fail outputs are defined:
- fulfilled by required summary/JUnit/artifact outputs and verdict rule.
