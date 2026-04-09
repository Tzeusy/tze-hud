# Policy Wiring Completion Direction (Gen-3)

Date: 2026-04-09
Scope: `/project-direction` completion package for policy wiring after governance-seam repair

## Executive Verdict

[Observed] Recent policy work fixed the live lease/session authority seam, aligned mid-session capability escalation semantics, and reconciled the runtime/policy/scene ownership model. See `crates/tze_hud_protocol/src/session_server.rs`, `docs/reconciliations/policy_wiring_seam_contract.md`, and `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`.

[Observed] The bounded mutation-path pilot is now wired in protocol admission (`crates/tze_hud_protocol/src/session_server.rs`) while runtime frame/event hot paths remain runtime-owned and outside policy evaluation (`crates/tze_hud_runtime/src/lib.rs`).

[Inferred] Honest "completion" no longer means "finish full seven-level policy wiring everywhere in v1." It means:

1. either land a bounded mutation-path pilot that preserves runtime sovereignty and proves latency/telemetry fitness,
2. or explicitly shrink the remaining v1 policy claims and defer unified policy wiring to v2,
3. then reconcile the resulting truth and close the program.

## Evidence Baseline

1. [Observed] V1 runtime authority is currently split across runtime, protocol/session, and scene surfaces, not centralized in `tze_hud_policy`.
2. [Observed] The live authority bug for lease scope is fixed; lease requests outside session-granted capabilities are denied.
3. [Observed] `CapabilityRequest` semantics are now explicit and all-or-nothing against configured policy scope.
4. [Observed] The seam contract now defines `tze_hud_runtime` as the mutable authority owner, `tze_hud_policy` as the pure evaluator target, and `tze_hud_scene::policy` as transitional.
5. [Observed] Mutation-path telemetry budget conformance is now encoded and test-backed (`crates/tze_hud_telemetry/src/validation.rs`, `crates/tze_hud_protocol/src/session_server.rs`).
6. [Unknown] End-to-end hardware-specific p99 behavior for all runtime workloads still depends on live telemetry sampling, not unit tests alone.

## What Completion Means Now

### Recommended completion target

[Inferred] Treat policy completion as **bounded v1 closeout**, not as a mandate to widen v1 until every target-state arbitration claim is live.

That bounded closeout has four conditions:

1. Runtime-owned authority remains canonical in v1.
2. A mutation-path pilot is the only admissible new policy wiring in v1.
3. Event/frame hot-path wiring stays out of v1 unless mutation telemetry proves it is both low-churn and budget-safe.
4. If the mutation pilot is rejected or fails budgets, remaining target-state policy claims must be downgraded or deferred explicitly rather than implied.

### Not completion

[Observed] These do **not** qualify as honest completion:

1. Claiming that full seven-level hot-path arbitration is already active in v1.
2. Reopening frame/event wiring before mutation-path telemetry exists.
3. Treating `tze_hud_scene::policy` as a second mutable authority.
4. Adding a big-bang policy facade that duplicates budget, attention, freeze, or lease state machines.

## Remaining Workstreams

### Workstream A: Mutation-path pilot

Goal: wire `tze_hud_policy` into mutation admission only, with snapshot-in / decision-out semantics and runtime-owned side-effect execution.

Depends on:

1. Completed seam contract and capability semantics
2. Current runtime/policy/spec alignment

Stop conditions:

1. If pilot requires moving mutable ownership into `tze_hud_policy`, stop.
2. If latency budget cannot be met with bounded instrumentation, stop and route to scope-shrink.

### Workstream B: Telemetry and latency proof

Goal: make mutation-path policy evaluation observable and prove whether it fits the repo's latency/validation doctrine.

Depends on:

1. Mutation-path pilot landing behind a clear contract boundary

Required outputs:

1. Per-outcome counts
2. Per-level diagnostics
3. Budget percentile reporting suitable for CI

Implementation note (hud-s98v.2):

1. `crates/tze_hud_protocol/src/session_server.rs` emits mutation-path admission telemetry logs for live and queued paths, including per-outcome counters, per-level diagnostic summaries, and structured latency conformance payloads.
2. `crates/tze_hud_telemetry/src/validation.rs` defines `policy_mutation_eval_p99` conformance (`POLICY_MUTATION_EVAL_BUDGET_US = 50`) and unit-test-backed pass/fail/no-samples outcomes for CI-visible budget evidence.

### Workstream C: V1 scope decision

Goal: explicitly choose one of:

1. `keep-v1-bounded`: mutation pilot is enough; frame/event remain deferred
2. `shrink-v1-claims`: even mutation-path policy wiring is not a v1 requirement

[Inferred] This decision is the real closeout gate for the policy program.

Decision outcome (hud-s98v.3, 2026-04-10):

1. [Observed] `keep-v1-bounded` is selected.
2. [Observed] The bounded mutation-path pilot plus conformance harness is retained as the v1 closeout floor.
3. [Observed] Frame/event unified hot-path policy wiring remains explicitly deferred to v2.
4. [Observed] The detailed decision record is captured in `docs/reconciliations/policy_wiring_closeout_decision_20260410.md`.

### Workstream D: Final reconciliation and signoff

Goal: map every retained v1 MUST claim to code/tests or downgrade it before closure.

Depends on:

1. Workstream B
2. Workstream C

## Proposed Bead Graph

### Epic

- `Finish policy wiring honestly after governance reconciliation`

### Children

1. `Implement bounded mutation-path policy pilot`
2. `Add policy-path telemetry and latency conformance harness`
3. `Decide v1 closeout path for policy wiring after pilot evidence`
4. `If needed, shrink residual v1 policy claims to match shipped truth`
5. `Reconcile policy-wiring closeout against code/spec/doctrine`
6. `Publish final human signoff for policy-wiring closeout`

### Dependencies

1. `2` depends on `1`
2. `3` depends on `2`
3. `4` depends on `3` and should execute only if `3` chooses the shrink path
4. `5` depends on `3` and on `4` if shrink path is chosen
5. `6` depends on `5`

## Exact `bd` Commands

```bash
EPIC=$(bd create --title "Finish policy wiring honestly after governance reconciliation" \
  --type epic --priority 1 \
  --description "Drive the policy program to an honest completion state after governance-seam repair. Complete either a bounded mutation-path pilot with telemetry-backed closeout, or explicitly shrink residual v1 policy claims and defer unified hot-path wiring to v2. Evidence baseline: crates/tze_hud_runtime/src/lib.rs, crates/tze_hud_protocol/src/session_server.rs, docs/reconciliations/policy_wiring_seam_contract.md, openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md." \
  --json | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"id\"])')

MUT=$(bd create --title "Implement bounded mutation-path policy pilot" \
  --type feature --priority 1 --parent "$EPIC" \
  --description "Wire tze_hud_policy into mutation admission only. Runtime remains sole mutable authority owner; evaluators stay pure (PolicyContext -> ArbitrationOutcome). Covers docs/reconciliations/policy_wiring_seam_contract.md and openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md." \
  --json | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"id\"])')

TEL=$(bd create --title "Add policy-path telemetry and latency conformance harness" \
  --type task --priority 1 --parent "$EPIC" \
  --description "Add CI-visible telemetry, diagnostics, and latency assertions for the mutation-path policy pilot. Covers about/heart-and-soul/validation.md and openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md." \
  --json | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"id\"])')

DECIDE=$(bd create --title "Decide v1 closeout path for policy wiring after pilot evidence" \
  --type task --priority 1 --parent "$EPIC" \
  --description "Use mutation-path telemetry and latency evidence to choose the honest v1 closeout path: keep bounded mutation-only policy wiring, or shrink residual v1 policy claims and defer unified hot-path wiring to v2." \
  --json | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"id\"])')

SHRINK=$(bd create --title "If needed, shrink residual v1 policy claims to match shipped truth" \
  --type task --priority 1 --parent "$EPIC" \
  --description "If the closeout decision rejects broader v1 policy wiring, update remaining policy specs/docs so v1 claims match shipped runtime truth and explicitly defer unified policy hot-path work to v2." \
  --json | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"id\"])')

RECON=$(bd create --title "Reconcile policy-wiring closeout against code/spec/doctrine" \
  --type task --priority 1 --parent "$EPIC" \
  --description "Deep-dive reconciliation of retained policy claims against code, tests, and doctrine after the closeout decision. Create follow-on fix beads if any retained claim lacks evidence." \
  --json | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"id\"])')

SIGNOFF=$(bd create --title "Publish final human signoff for policy-wiring closeout" \
  --type task --priority 1 --parent "$EPIC" \
  --description "Publish the final human-readable signoff artifact for the bounded policy closeout path, including explicit notes on what v1 does and does not claim after closure." \
  --json | python3 -c 'import sys,json; print(json.load(sys.stdin)[\"id\"])')

bd dep add "$TEL" "$MUT"
bd dep add "$DECIDE" "$TEL"
bd dep add "$SHRINK" "$DECIDE"
bd dep add "$RECON" "$DECIDE"
bd dep add "$RECON" "$SHRINK"
bd dep add "$SIGNOFF" "$RECON"
```

## Changed-File Recommendations

1. Create this document as the Gen-3 completion report.
2. Keep `docs/reconciliations/policy_wiring_execution_backlog.md` as historical context; do not overwrite it with the completion package.
3. If the shrink path is chosen, update:
   - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
   - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
   - any remaining policy direction docs that still imply broader v1 hot-path wiring
