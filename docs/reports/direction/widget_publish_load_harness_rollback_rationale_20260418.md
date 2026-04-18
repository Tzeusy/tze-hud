# Rust Widget Publish Load Harness Rollback Rationale

Date: 2026-04-18
Issue: `hud-iwzd` (parent epic: `hud-h6jq`)
Scope: investigate the rationale behind commit `0da44b9` (`Diff reconciliation`, 2026-04-10 20:39:24 +0800), which deleted the shipped rust-widget-publish-load-harness implementation.
Status: investigation complete; recommendation issued.

## 1. Most likely rationale

**Most likely rationale: undifferentiated bulk revert of a staging/WIP branch to a clean baseline, *not* a doctrine-driven rescope.**

The commit rolls back far more than the harness. In a single non-merge commit on `main`, Tzeusy reverted at least five independent in-flight workstreams that had already been PR-reviewed and merged:

1. `hud-bm9i.*` — Rust publish-load harness epic (PRs #409, #411, #415, #420, #421, #418, #422, #423, #424).
2. `hud-c8ra` — structured CapabilityAuditRecord (PR #417).
3. `hud-dpd0` — in-repo `scripts/epic-report-scaffold.sh` target (PR #425).
4. `hud-s98v.*` — policy wiring v1 closeout reconciliation + scope-shrink memos (4 deleted reconciliation reports under `docs/reconciliations/`).
5. `v2-embodied-media-presence` — deleted `beads-graph.md`, `execution-plan.md`, `reconciliation.md`, `final-reconciliation.md` from `openspec/changes/v2-embodied-media-presence/` while keeping the proposal/design/tasks/spec skeleton.

Simultaneously it *added* new presence-card exemplar material, RFC 0011 session-resource upload planning artifacts, and the new `/user-test-performance` skill scaffold. The commit message is the bare string "Diff reconciliation" with no body, no bead reference, no mention of deprecation or supersession.

Three concrete signals point to bulk-revert rather than targeted scope cut:

1. **No surviving doctrine supersedes the harness.** The `v1-mvp-standards/specs/session-protocol/spec.md` delta that ships on `main` still omits `WidgetPublishResult.request_sequence` (line 751), but the **parallel OpenSpec change** `openspec/changes/rust-widget-publish-load-harness/specs/session-protocol/spec.md` still MANDATES `request_sequence` as a MUST. Those two spec deltas are in direct contradiction on `main` today.
2. **The replacement code is self-contradictory.** `/user-test-performance/scripts/grpc_widget_publish_perf.py` on `main` contains the explicit comment *"Proto v1 WidgetPublishResult is not correlated by request sequence, so per-request RTT is not reported"* — exactly the honest-measurement gap the harness was built to close. The new skill acknowledges the gap while the now-deleted harness had already fixed it.
3. **Bead close reasons cite completion, not supersession.** `bd show hud-bm9i hud-bm9i.6 hud-bm9i.5 hud-31c5 hud-zjlh hud-lh9k` all report close reasons of the form "PR #N merged after review" or "All scoped child beads completed and merged". None cite rescope, withdrawal, or supersession. `hud-bm9i` was closed at 2026-04-10 04:41 UTC — 8 hours before `0da44b9` was authored at 12:39 UTC. The epic was declared done and then its artifacts were deleted with no accompanying bead action.

Secondary rationales can be ruled out:

- **Not a scope cut aligned with v1 doctrine.** The harness itself was already narrow (gRPC widget publish only, one primary target, MCP/zones/tiles explicitly out of scope) and directly satisfied the "validation is first-class" doctrine in `about/heart-and-soul/validation.md`. The only substantive doctrine edit was the *removal* of the `policy_mutation_eval_p99` budget line from `validation.md`, which aligns with the parallel `policy_wiring_v1_scope_shrink_20260410.md` deletion, not with harness rescoping.
- **Not a redundancy cleanup.** The Python `grpc_widget_publish_perf.py` restored to `main` is strictly less capable than the deleted Rust harness (no per-request RTT) and explicitly says so in its own docstring.
- **Not doctrine-accidental drift.** `AGENTS.md` was retained and updated with notes about the new user-test-performance CSV ownership and an explicit call-out: *"Honest resident widget-publish benchmarking is currently blocked by contract drift: RFC 0005 expects `WidgetPublishResult.request_sequence`, but `crates/tze_hud_protocol/proto/session.proto`, runtime handling, and active OpenSpec omit it; repair that seam before trusting per-request RTT numbers."* That note describes the exact hole the harness filled.

The most parsimonious explanation: `0da44b9` represents an operator resetting their working tree to a known-good pre-rescoping baseline (possibly from a branch where the rescope conversation was still open) and force-landing it over `main`, **inadvertently taking shipped work with it**. This is consistent with the stripped status line in the restored direction report ("Planned and locally validated; not pushed") replacing the previous ("Planned and locally validated; captured in repo via PR") — the restored file reads like a local copy that never learned it had already shipped.

## 2. Evidence citations

| Claim | Evidence |
|---|---|
| Commit deletes 116 files / ~5.4k lines added / ~5.5k lines removed in a single non-merge commit | `git show 0da44b9 --stat` |
| `WidgetPublishResult.request_sequence` removed from proto | `git show 0da44b9 -- crates/tze_hud_protocol/proto/session.proto`, field 5 deleted |
| `CapabilityAuditRecord` message + `capability_audit_records` fields deleted | same proto diff, lines 303-330 reverted |
| Example crate `examples/widget_publish_load_harness/` (1181 lines) removed, plus Cargo workspace entry | `examples/widget_publish_load_harness/{Cargo.toml,src/main.rs}` D; `Cargo.toml` member list -1 |
| `crates/tze_hud_telemetry/src/publish_load.rs` (357 lines) + `validation.rs` (122 lines) + `tests/publish_load_artifact.rs` (173 lines) removed | commit name-status list |
| Epic report + component-flow diagrams deleted | `docs/reports/hud-bm9i-rust-widget-publish-load-harness.md`, `docs/reports/diagrams/hud-bm9i-component-flow.{mmd,svg}` D |
| Direction report rewritten from "captured in repo via PR" to "not pushed" and issue ID stripped | `git show 0da44b9 -- docs/reconciliations/rust_widget_publish_load_harness_direction_20260410.md` header diff |
| All 9 `hud-bm9i.*` / `hud-zjlh` / `hud-31c5` / `hud-sljv` / `hud-6bkd` / `hud-lh9k` PRs MERGED before the commit | `gh pr list --state all --search "publish-load harness"` (#409 merged 2026-04-09 18:57 UTC; #424 merged 2026-04-10 04:18 UTC; `0da44b9` authored 2026-04-10 12:39 UTC) |
| Replacement Python script openly admits per-request RTT is no longer reported | `.claude/skills/user-test-performance/scripts/grpc_widget_publish_perf.py` docstring on `main` |
| OpenSpec change still mandates the protocol contract the proto now omits | `openspec/changes/rust-widget-publish-load-harness/specs/session-protocol/spec.md` vs. `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L751` |
| Surviving `AGENTS.md` note flags exactly the contract drift the revert created | AGENTS.md "Honest resident widget-publish benchmarking is currently blocked by contract drift..." |
| No sibling report supersedes the harness | Checked `presence_card_direction_report_20260410.md`, `policy_wiring_completion_direction_20260409.md`, `session_resource_upload_rfc0011_direction_report_20260410.md`, `policy_wiring_v1_scope_shrink_20260410.md` (pre-delete) — none reference the harness |

## 3. Other rescoping acts bundled into `0da44b9`

The commit is a grab-bag, not a coherent rescope. It simultaneously:

- **Policy wiring scope-shrink (intended)**: deleted `docs/reconciliations/policy_wiring_v1_scope_shrink_20260410.md`, `policy_wiring_closeout_decision_20260410.md`, `policy_wiring_closeout_reconciliation_20260410.md`, `policy_wiring_final_human_signoff_20260410.md`, `policy_arbitration_telemetry_rescope_20260410.md`. Some matching doctrine edits (removal of `policy_mutation_eval_p99` from `validation.md`, AGENTS.md note updates) are retained — so at least the intent to shrink v1 policy-wiring claims is visible in the final state, even if the supporting reports were deleted.
- **Harness revert (likely unintended)**: as documented in section 1. No corresponding doctrine change exists. The OpenSpec planning folder and direction report are left stranded.
- **CapabilityAuditRecord revert (likely unintended)**: proto `capability_audit_records` fields removed from `SessionEstablished` and `CapabilityNotice`, plus the `CapabilityAuditKind` enum and `CapabilityAuditRecord` message. No doctrine edit supports this; `hud-c8ra` (PR #417) is not referenced anywhere in the surviving tree.
- **Epic-report scaffold revert (likely unintended)**: `scripts/epic-report-scaffold.sh` + tests removed. AGENTS.md note about it was retained ("Epic report scaffolding lives at `scripts/epic-report-scaffold.sh`"), which now points at nothing.
- **v2 embodied media planning artifacts removed (uncertain)**: `openspec/changes/v2-embodied-media-presence/{beads-graph,execution-plan,reconciliation,final-reconciliation}.md` deleted while proposal/design/tasks/spec kept. This could be an intentional "defer v2 planning detail until v1 closes" move, but also fits the bulk-revert pattern.
- **New presence-card + RFC 0011 planning material added (intended)**: `docs/reconciliations/presence_card_direction_report_20260410.md`, `session_resource_upload_rfc0011_direction_report_20260410.md`, and the associated backlog materialization JSON files land in this commit. The `/user-test-performance` skill scaffold (scripts, scenarios, vendored proto stubs) is also a net-new addition.

Summary: the commit mixes **one deliberate rescoping act** (policy-wiring scope shrink) and **three-to-four accidental regressions** (harness, capability audit, epic scaffold, v2 planning detail) with **one forward step** (presence-card + RFC 0011 + user-test-performance skill).

## 4. Recommendation

**Recommendation: REINSTATE the harness in whatever smallest honest form still matches shipped v1 scope.**

Justification:

1. **The replacement has a known, self-declared honesty gap.** Main's `grpc_widget_publish_perf.py` explicitly cannot produce per-request RTT. The whole *reason* the harness exists — honest per-request latency on the hot publish path — is still needed, still blocked by the same contract seam, and still called out in `AGENTS.md` as a live drift.
2. **The cost of reinstatement is low.** Every child bead was merged, reviewed, and closed. The work still exists in git history (`git show 0da44b9^:...`). A reinstatement epic does not need to rediscover the design; it can cherry-pick or revert-the-revert for the harness slice while leaving other co-reverted items (CapabilityAuditRecord, v2 planning detail) to be re-landed on their own schedules.
3. **Withdrawal is more expensive than reinstatement.** A clean withdrawal requires: archiving the OpenSpec change, removing the `WidgetPublishResult.request_sequence` requirement from its session-protocol delta, scrubbing AGENTS.md notes, removing the direction report's implicit promise, and reconciling `/user-test-performance` to own the "no per-request RTT" limitation permanently. That is more editorial surface than reinstating the deleted code.
4. **Doctrine still supports the harness.** `about/heart-and-soul/validation.md` treats Layer 4 benchmark artifacts as first-class; no surviving doctrine file says the Rust harness is out of v1 scope; `AGENTS.md` still flags the gap it fills.
5. **No one argued for withdrawal.** There is no commit, PR review, bead close reason, direction report, or doctrine edit on `main` that articulates a reason to not ship the harness. All deletion signal is negative space — absence, not argument.

Reinstatement should be scoped conservatively:

- Restore `WidgetPublishResult.request_sequence` (proto field 5) and the correlated runtime path; re-align `v1-mvp-standards/specs/session-protocol/spec.md#L751` with the `rust-widget-publish-load-harness/specs/session-protocol/spec.md` delta.
- Restore `examples/widget_publish_load_harness/` and `crates/tze_hud_telemetry/src/publish_load.rs` + artifact test.
- Re-route `/user-test-performance` gRPC widget runs through the Rust binary (undoing the regression note in `grpc_widget_publish_perf.py`).
- Advance the OpenSpec change through reconciliation and archive.
- Produce a new gen-1 reconciliation (replacing the deleted `rust_widget_publish_load_harness_reconciliation_gen1_20260410.md`) and a new closeout report at `docs/reports/`.
- **Out of scope for reinstatement**: CapabilityAuditRecord, `scripts/epic-report-scaffold.sh`, v2 planning artifacts — each of those was also co-reverted and each deserves its own investigation bead rather than being bundled into harness reinstatement.

If the decision gate instead chooses **withdraw**, the minimum honest exit is: archive `openspec/changes/rust-widget-publish-load-harness/` with `--reason="withdrawn-post-0da44b9"`, remove the `request_sequence` requirement from its session-protocol delta, remove the "contract drift" note from `AGENTS.md` or rewrite it to say the drift is permanent, and update `/user-test-performance`'s doctrine to acknowledge that per-request RTT is not a v1 deliverable. That withdrawal path is viable but strictly worse than reinstatement given the asymmetry above.
