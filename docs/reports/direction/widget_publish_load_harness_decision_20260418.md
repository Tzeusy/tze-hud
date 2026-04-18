# Rust Widget Publish Load Harness — Decision

Date: 2026-04-18
Issue: `hud-b642` (parent epic: `hud-h6jq`)
Gate: decide reinstate vs. withdraw
Input: `docs/reports/direction/widget_publish_load_harness_rollback_rationale_20260418.md` (investigation memo from `hud-iwzd`)

## Decision

**REINSTATE** the `rust-widget-publish-load-harness` OpenSpec change on v1.

One-line justification: the investigation memo demonstrates that commit `0da44b9` was an undifferentiated bulk revert, not a doctrine-driven rescope — main still carries the contract drift it created (WidgetPublishResult missing `request_sequence` while every sibling result message on main carries it), and the Python `/user-test-performance` replacement openly self-declares it cannot honestly measure per-request publish RTT.

## Supporting rationale

The memo's evidence is uncontested and has been independently cross-checked against main:

1. **Contract drift is active.** `crates/tze_hud_protocol/proto/session.proto` defines `WidgetPublishResult` (lines 589–594) with only `accepted`, `widget_name`, and `error` — no `request_sequence`. Every structurally comparable ack message on the same proto carries `request_sequence` as field 1: `ZonePublishResult` (line 564), `WidgetAssetRegisterResult` (line 614), `ResourceUploadAccepted` (line 655), `ResourceStored` (line 673), `ResourceErrorResponse` (line 699), `EmitSceneEventResult` (line 897). `WidgetPublishResult` is the odd one out, and the asymmetry is the exact contract shape the harness was built against.
2. **OpenSpec is in direct contradiction with itself.** `openspec/specs/session-protocol/spec.md` (authoritative for main) describes `WidgetPublishResult` without `request_sequence`. `openspec/changes/rust-widget-publish-load-harness/specs/session-protocol/spec.md` MODIFIES that requirement to require `request_sequence`. Leaving both on main is strictly worse than resolving the contradiction in either direction — reinstatement resolves it with one restored proto field; withdrawal requires editing two specs, a proto comment block, AGENTS.md, and `/user-test-performance` doctrine.
3. **AGENTS.md already flags the harness-shaped hole.** Line 253 of `AGENTS.md` still reads: *"Honest resident widget-publish benchmarking is currently blocked by contract drift: RFC 0005 expects `WidgetPublishResult.request_sequence`, but `crates/tze_hud_protocol/proto/session.proto`, runtime handling, and active OpenSpec omit it; repair that seam before trusting per-request RTT numbers."* The note treats the gap as a live bug.
4. **The replacement admits the gap.** `.claude/skills/user-test-performance/scripts/grpc_widget_publish_perf.py` (lines 6–7): *"Proto v1 WidgetPublishResult is not correlated by request sequence, so per-request RTT is not reported."* The Python perf path is strictly less capable than the deleted Rust harness; it is not a supersession, it is a regression with explicit scope acknowledgement.
5. **V1 doctrine still supports the harness.** `about/heart-and-soul/v1.md` §V1 must prove #4 locks in hardware-normalized latency budgets (input_to_scene_commit p99 < 50ms) as shipping properties. `about/heart-and-soul/validation.md` treats Layer 4 benchmark artifacts as first-class. Nothing in surviving v1 doctrine scopes out per-request publish RTT — in fact v1 explicitly scopes it in.
6. **No documented argument for withdrawal exists.** The rationale memo exhaustively searched bead close reasons, PR reviews, direction reports, and doctrine edits from 2026-04-08 through 2026-04-18. Every deletion signal is negative space (absence of discussion), and no surviving artifact on main argues that Rust-native harness benchmarking should be withdrawn from v1.

The "withdraw" alternative is strictly more expensive and strictly less honest: it requires rewriting the OpenSpec change's session-protocol delta, scrubbing the AGENTS.md drift note, updating `/user-test-performance`'s scope claims, and accepting that v1 will ship without honest per-request widget-publish RTT measurement despite doctrine requiring it. There is no commensurate benefit.

## Execution routing under `hud-h6jq`

With this decision, the execution branch of the epic is fixed:

| Bead | Title | New status | Reason |
|---|---|---|---|
| `hud-as4t` | Reinstate rust-widget-publish-load-harness implementation (conditional) | **ACTIVATE** (unblock; keep `open`, consider bumping to P1 to match epic urgency) | Decision is REINSTATE. This is the execution bead. |
| `hud-7wjx` | Withdraw OpenSpec change rust-widget-publish-load-harness (conditional) | **CLOSE as `not-applicable`** | Decision is REINSTATE; withdrawal branch is moot. |
| `hud-dg53` | Reconcile spec-to-code (gen-1) for rust-widget-publish-load-harness rollback resolution | **KEEP OPEN, still blocked on `hud-as4t`** | Terminal reconciliation bead runs after the activated execution bead, regardless of branch. |

`hud-as4t` already names the reinstate surface (proto `request_sequence`, example crate, `tze_hud_telemetry::publish_load`, targets TOML, gRPC wire-up, OpenSpec archival, closeout report) and the forward-adaptation workflow; no scope edits are required by this decision.

## Explicitly out of scope for the reinstate branch

Per the memo §4, the reinstate bead (`hud-as4t`) should NOT attempt to un-revert everything `0da44b9` co-deleted. Each of the following is a separate investigation/decision thread and must not be bundled into harness reinstatement:

- `CapabilityAuditRecord` message + `capability_audit_records` proto fields (from `hud-c8ra`, PR #417).
- `scripts/epic-report-scaffold.sh` + its tests (from `hud-dpd0`, PR #425), even though AGENTS.md still points at it.
- `openspec/changes/v2-embodied-media-presence/{beads-graph,execution-plan,reconciliation,final-reconciliation}.md` (may be a deliberate v2-defer, may be collateral).
- The four deleted policy-wiring reconciliation memos under `docs/reconciliations/` — some of their intent landed via doctrine edits, so this is likely partial-intent rather than pure accident, but it is separate scope from the harness.

If any of those four deserve reinstatement, file a sibling epic (not under `hud-h6jq`).

## Acceptance of this decision

1. This memo exists at `docs/reports/direction/widget_publish_load_harness_decision_20260418.md` with a single unambiguous outcome (REINSTATE) — acceptance criterion 1 of `hud-b642`.
2. The epic `hud-h6jq` description will be updated by the decision bead to record this outcome and unblock downstream execution — acceptance criterion 2.
3. `hud-as4t` remains `open` (activated); `hud-7wjx` is closed `not-applicable`; `hud-dg53` remains `open` — acceptance criterion 3.
4. No code or spec changes are made by `hud-b642` itself — acceptance criterion 4. Code restoration is `hud-as4t`'s job.
