# WebRTC/Media V1 Reconciliation: Corrected Contract + Backlog Coverage (WM-G2)

Date: 2026-04-09
Issue: `hud-nn9d.17`
Parent epic: `hud-nn9d`
Depends on: `hud-nn9d.16` (corrected contract tranche human signoff)

## Purpose

Verify that the corrected media contract tranche closes the missing
protocol/schema/config/compositor seams identified in WM-S0, and verify that
implementation-bead creation remains blocked until this reconciliation gate
closes.

This is a reconciliation artifact. It does not authorize runtime media
implementation in v1 and does not alter default-off doctrine boundaries.

## Inputs

- `docs/reconciliations/webrtc_media_v1_seam_inventory.md` (WM-S0 seam map)
- `docs/reconciliations/webrtc_media_v1_backlog_materialization.md` (corrected
  dependency/order contract)
- `docs/reconciliations/webrtc_media_v1_human_signoff_report.md` (`hud-nn9d.16`)
- `docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md` (WM-S2b)
- `openspec/specs/media-webrtc-bounded-ingress/spec.md` (+ WM-S2c details)
- `docs/reconciliations/webrtc_media_v1_runtime_activation_gate_budgets.md` (WM-S3)
- `openspec/specs/media-webrtc-privacy-operator-policy/spec.md` (WM-S3b)
- `docs/reconciliations/webrtc_media_v1_compositor_videosurfaceref_contract.md` (WM-S3c)
- `docs/reconciliations/webrtc_media_v1_validation_rehearsal_thresholds.md` (WM-S4)
- Beads evidence snapshot from `bd show hud-nn9d --json` and `bd list --json`
  (run in this worktree on 2026-04-09)

## Seam Reconciliation Matrix (Missing Seams -> Corrected Contract Coverage)

| Missing seam from WM-S0 | Corrected contract closure | Evidence | Reconciliation verdict |
|---|---|---|---|
| Protocol/schema parity and snapshot semantics (`WM-S0-2`) | Closed by WM-S2b (`hud-nn9d.8`) | `webrtc_media_v1_protocol_schema_snapshot_deltas.md` defines envelope fields, `ZonePublish` parity fields, reconnect/snapshot rules, and backward-compat constraints | Closed at contract layer |
| Zone transport/layer/reconnect semantics (`WM-S0-3`) | Closed by WM-S2c (`hud-nn9d.9`) + WM-S3c (`hud-nn9d.12`) | Bounded-ingress spec now carries explicit fixed-zone class + transport/layer constraints; compositor contract constrains render attachment/confinement | Closed at contract layer |
| Config/profile + activation gate ambiguity (`WM-S0-4` + `WM-S0-8`) | Closed by WM-S3 (`hud-nn9d.10`) + WM-S3b (`hud-nn9d.11`) | Activation prerequisites + quantitative budgets + default-off no-enable guardrails; explicit enablement/override precedence and observability | Closed at contract layer |
| Compositor behavior gap (`WM-S0-5`) | Closed by WM-S3c (`hud-nn9d.12`) | Runtime-owned texture lifecycle, present/expiry semantics, degradation/fallback states, strict non-audio behavior | Closed at contract layer |
| Validation-rehearsal contract gap (`WM-S0-6`) | Closed by WM-S4 (`hud-nn9d.13`) | Explicit rehearsal scene matrix, dual-lane headless vs real-decode strategy, quantitative thresholds, CI-visible outputs | Closed at contract layer |

## Hidden Assumption Reconciliation

This pass converts prior hidden assumptions into explicit contract or explicit
deferral:

1. **Assumption:** media signaling shape could be deferred until coding.
- **Now explicit:** WM-S2a resolved signaling shape; WM-S2b codified wire fields.

2. **Assumption:** scene-level media types implied protocol/snapshot parity.
- **Now explicit:** WM-S2b codifies parity and rejects malformed timing contracts.

3. **Assumption:** activation could be enabled by ad hoc config/feature flags.
- **Now explicit:** WM-S3 requires one deterministic gate and forbids out-of-gate
  enablement.

4. **Assumption:** household privacy/operator constraints could be post-hoc.
- **Now explicit:** WM-S3b makes policy/override precedence a hard admission
  contract.

5. **Assumption:** schema acceptance implied render behavior.
- **Now explicit:** WM-S3c defines render-state transitions and teardown/fallback
  behavior.

6. **Assumption:** validation could be generic and inferred from existing CI.
- **Now explicit:** WM-S4 defines bounded-ingress rehearsals, thresholds, and
  CI-visible verdict outputs.

7. **Deferred explicitly (not hidden):** bidirectional AV, audio policy, and
  multi-feed orchestration remain outside this tranche (`WM-D*` markers in
  backlog materialization).

## Backlog-Gate Coverage Check

Corrected backlog ordering requires:

1. complete corrected contract tranche (`WM-S1`..`WM-S6`),
2. renewed human signoff (`WM-G1` = `hud-nn9d.16`),
3. corrected-contract reconciliation (`WM-G2` = `hud-nn9d.17`),
4. only then implementation tranche creation (`WM-I*`).

Evidence snapshot (2026-04-09):

- `bd show hud-nn9d --json` reports `epic_closed_children = 16` of 17 with only
  `hud-nn9d.17` still in progress.
- Query for implementation-tranche beads in tracker via `bd list --json` returns
  no matching `WM-I*`/implementation media-ingress beads in current tracker state.

Reconciliation verdict:

- [Observed] Implementation-bead creation has remained blocked through this gate.
- [Observed] No hidden implementation track was introduced before WM-G2 closure.

## Acceptance Traceability (`hud-nn9d.17`)

1. Missing protocol/schema/config/compositor seams are reconciled:
- fulfilled by seam matrix above with explicit contract artifacts and closure
  verdict per seam.

2. Hidden assumptions converted to explicit work or deferrals:
- fulfilled by Hidden Assumption Reconciliation section and explicit `WM-D*`
  deferred-scope mapping.

3. Implementation-bead creation remains blocked until reconciliation closes:
- fulfilled by backlog-gate evidence and no-`WM-I*` tracker query result.

## Final Reconciliation Verdict

The corrected media contract tranche is now internally reconciled against the
missing seam set that motivated the corrected plan. Contract coverage is
sufficiently explicit to prevent silent scope creep, and implementation-bead
creation has remained correctly blocked through WM-G2.

After `hud-nn9d.17` closes, implementation-bead materialization may proceed only
per corrected ordering in `webrtc_media_v1_backlog_materialization.md`, with
default-off and bounded-slice constraints preserved.
