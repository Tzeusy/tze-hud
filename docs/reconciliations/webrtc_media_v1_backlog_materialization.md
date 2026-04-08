# WebRTC/Media V1 Backlog Materialization (hud-nn9d.2)

## Purpose

Materialize follow-on backlog from `docs/reconciliations/webrtc_media_v1_direction_report.md` into a low-churn, spec-first bead plan that a coordinator can apply without reinterpretation.

Companion machine-readable payload: `docs/reconciliations/webrtc_media_v1_backlog_materialization.proposed_beads.json`.

## Source-of-Truth Inputs

- Direction report: `docs/reconciliations/webrtc_media_v1_direction_report.md`
- Epic prompt: `docs/reconciliations/webrtc_media_v1_epic_prompt.md`
- Existing parent/children: `hud-nn9d` (epic), `hud-nn9d.3` (human report), `hud-nn9d.4` (reconciliation)

## Ordering Rules

1. No runtime media implementation bead starts before media contracts are written and reviewed.
2. Contract beads are serialized to avoid protocol/runtime churn from unstable scope.
3. Implementation beads are gated by both human review (`hud-nn9d.3`) and reconciliation (`hud-nn9d.4`).
4. Deferred scope is tracked explicitly as blocked follow-on beads, not silent TODOs.

## Proposed Bead Set (Coordinator-Apply)

### Phase A: Spec-first contract tranche (create now)

| Local ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| WM-S1 | Spec: media/WebRTC post-v1 capability contract (single-ingress slice) | task | 1 | discovered-from:hud-nn9d.2 | Direction report says this is the hard blocker for all decomposition. |
| WM-S2 | Spec: session-protocol media signaling delta (compat + failure semantics) | task | 1 | WM-S1 | Protocol contract must stabilize before runtime wiring. |
| WM-S3 | Spec: runtime-kernel media activation gate and budgets | task | 1 | WM-S1, WM-S2 | Prevent ad-hoc media-thread enablement and budget drift. |
| WM-S4 | Spec: validation-framework media rehearsal scenarios and pass/fail thresholds | task | 1 | WM-S1, WM-S2, WM-S3 | Defines measurable acceptance before implementation beads execute. |
| WM-S5 | Docs/spec alignment: architecture-v1 phased contract wording for media deferment | task | 2 | WM-S1 | Removes doctrine ambiguity called out in direction report blockers. |
| WM-S6 | Docs alignment: README media/WebRTC claims vs v1/post-v1 scope | task | 2 | WM-S1, WM-S5 | Closes public-claim drift where README can imply active v1 WebRTC/media support. |

### Phase B: Implementation tranche (create now, blocked)

| Local ID | Title | Type | Priority | Depends on | Notes |
|---|---|---|---|---|---|
| WM-I1 | Implement protocol/media conversion for first-slice signaling path | feature | 1 | WM-S2, hud-nn9d.3, hud-nn9d.4 | Keep embodied/bidirectional semantics deferred. |
| WM-I2 | Implement runtime media activation gate (`off` by default) + budget hooks | feature | 1 | WM-S3, WM-I1, hud-nn9d.3, hud-nn9d.4 | No media pool unless gate preconditions are met. |
| WM-I3 | Implement single inbound `VideoSurfaceRef` fixed-zone render path (no audio) | feature | 1 | WM-S1, WM-I2, hud-nn9d.3, hud-nn9d.4 | Smallest credible post-v1 slice from direction report. |
| WM-I4 | Validation and benchmark implementation for media first-slice gates | task | 1 | WM-S4, WM-I3, hud-nn9d.3, hud-nn9d.4 | Makes readiness evidence explicit in CI/integration flows. |
| WM-I5 | Security/privacy hardening for media ingress path and operator controls | task | 1 | WM-S1, WM-S2, WM-I2, hud-nn9d.3, hud-nn9d.4 | Required before enabling media in non-dev environments. |

### Phase C: Deferred scope markers (create as blocked/deferred)

| Local ID | Title | Type | Priority | Depends on | Deferred reason |
|---|---|---|---|---|---|
| WM-D1 | Deferred: bidirectional AV/WebRTC session negotiation and embodied presence | feature | 3 | WM-I4, WM-I5 | Explicitly rejected for current tranche; too much churn/risk. |
| WM-D2 | Deferred: audio routing/mixing policy engine | feature | 3 | WM-I4, WM-I5 | Premature until video ingress contract is stable and measured. |
| WM-D3 | Deferred: multi-feed compositing and adaptive bitrate orchestration | feature | 4 | WM-I4, WM-I5 | High complexity; out of smallest-credible-slice scope. |

## Dependency Graph (Low-Churn Execution)

1. `WM-S1`
2. `WM-S2`, `WM-S3`
3. `WM-S4`, `WM-S5`, `WM-S6`
4. `hud-nn9d.3` (human report bead captures spec outcomes + proposed deltas)
5. `hud-nn9d.4` (reconciliation verifies coverage and no scope creep)
6. `WM-I1` -> `WM-I2` -> `WM-I3` -> `WM-I4` (+ `WM-I5` in parallel with `WM-I2/WM-I3` once contract is stable)
7. `WM-D1`, `WM-D2`, `WM-D3` remain blocked/deferred

## Coordinator-Ready Bead Create Payloads (Do Not Execute Here)

Use these as direct `bd create` inputs from coordinator context. Replace local IDs in dependency arguments with created bead IDs.

```bash
bd create "Spec: media/WebRTC post-v1 capability contract (single-ingress slice)" \
  --type task --priority 1 \
  --description "Author normative media/WebRTC capability spec for first post-v1 slice. Include timing model, transport boundaries, lease/budget coupling, security/privacy assumptions, explicit non-goals, and measurable acceptance scenarios." \
  --deps discovered-from:hud-nn9d.2 --json

bd create "Spec: session-protocol media signaling delta (compat + failure semantics)" \
  --type task --priority 1 \
  --description "Define protocol field/message deltas for first media signaling path with backward compatibility, downgrade/fallback behavior, and error semantics." \
  --deps blocks:<WM-S1-ID> --json

bd create "Spec: runtime-kernel media activation gate and budgets" \
  --type task --priority 1 \
  --description "Define runtime media worker activation criteria, default-off behavior, explicit budgets, degradation policy coupling, and prerequisite validation evidence." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2-ID> --json

bd create "Spec: validation-framework media rehearsal scenarios and pass/fail thresholds" \
  --type task --priority 1 \
  --description "Specify validation scenes and benchmark thresholds required before enabling first media slice implementation." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2-ID> --deps blocks:<WM-S3-ID> --json

bd create "Docs/spec alignment: architecture-v1 phased contract wording for media deferment" \
  --type task --priority 2 \
  --description "Align architecture/v1 wording so media vision and v1 deferment are explicitly phased and non-contradictory." \
  --deps blocks:<WM-S1-ID> --json

bd create "Docs alignment: README media/WebRTC claims vs v1/post-v1 scope" \
  --type task --priority 2 \
  --description "Align README claims about protocol planes and media so v1 boundaries (no live media/WebRTC in v1) are explicit and consistent with doctrine/specs while preserving post-v1 direction." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S5-ID> --json

bd create "Implement protocol/media conversion for first-slice signaling path" \
  --type feature --priority 1 \
  --description "Implement protocol conversion and compatibility behavior for first-slice media signaling only; keep embodied and bidirectional AV out of scope." \
  --deps blocks:<WM-S2-ID> --deps blocks:hud-nn9d.3 --deps blocks:hud-nn9d.4 --json

bd create "Implement runtime media activation gate (off by default) + budget hooks" \
  --type feature --priority 1 \
  --description "Wire runtime activation gate and budget enforcement hooks for media path while preserving default-off behavior." \
  --deps blocks:<WM-S3-ID> --deps blocks:<WM-I1-ID> --deps blocks:hud-nn9d.3 --deps blocks:hud-nn9d.4 --json

bd create "Implement single inbound VideoSurfaceRef fixed-zone render path (no audio)" \
  --type feature --priority 1 \
  --description "Implement minimal one-way video ingest/render slice constrained to fixed-zone policy; no audio and no bidirectional call semantics." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-I2-ID> --deps blocks:hud-nn9d.3 --deps blocks:hud-nn9d.4 --json

bd create "Validation and benchmark implementation for media first-slice gates" \
  --type task --priority 1 \
  --description "Implement validation scenarios and benchmark checks defined in validation spec bead, including CI-visible pass/fail outputs." \
  --deps blocks:<WM-S4-ID> --deps blocks:<WM-I3-ID> --deps blocks:hud-nn9d.3 --deps blocks:hud-nn9d.4 --json

bd create "Security/privacy hardening for media ingress path and operator controls" \
  --type task --priority 1 \
  --description "Implement media-ingress threat mitigations, auth/privacy controls, and operator-facing guardrails required before non-dev enablement." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2-ID> --deps blocks:<WM-I2-ID> --deps blocks:hud-nn9d.3 --deps blocks:hud-nn9d.4 --json

bd create "Deferred: bidirectional AV/WebRTC session negotiation and embodied presence" \
  --type feature --priority 3 \
  --description "Deferred scope marker: bidirectional AV signaling/session semantics and embodied presence. Do not start before first-slice validation and hardening are complete." \
  --deps blocks:<WM-I4-ID> --deps blocks:<WM-I5-ID> --json

bd create "Deferred: audio routing/mixing policy engine" \
  --type feature --priority 3 \
  --description "Deferred scope marker: audio policy/routing engine post first-slice media stabilization." \
  --deps blocks:<WM-I4-ID> --deps blocks:<WM-I5-ID> --json

bd create "Deferred: multi-feed compositing and adaptive bitrate orchestration" \
  --type feature --priority 4 \
  --description "Deferred scope marker: multi-stream and ABR complexity, intentionally out of smallest-credible-slice scope." \
  --deps blocks:<WM-I4-ID> --deps blocks:<WM-I5-ID> --json
```

## Acceptance Traceability

- Target 1 (explicit dependencies): Satisfied by phase tables + create payload dependency wiring.
- Target 2 (spec-writing first): Satisfied by ordered `WM-S*` tranche gating all `WM-I*` items.
- Target 3 (low-churn + deferred markers): Satisfied by serialized contract -> review -> implementation flow and explicit `WM-D*` deferred beads.
