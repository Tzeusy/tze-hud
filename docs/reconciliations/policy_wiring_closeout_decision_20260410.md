# Policy Wiring V1 Closeout Decision (hud-s98v.3)

Date: 2026-04-10  
Issue: `hud-s98v.3`  
Scope: choose the honest v1 closeout path after the mutation-path pilot and telemetry evidence.

## Decision

[Observed] Choose `keep-v1-bounded`.

[Inferred] v1 closes policy wiring with a bounded mutation-admission path plus explicit telemetry conformance; it does not claim unified frame/event hot-path policy wiring.

## Evidence Reviewed

1. [Observed] Mutation-path policy admission is wired in protocol/session handling while runtime remains mutable authority owner:
   - `crates/tze_hud_protocol/src/session_server.rs`
   - `docs/reconciliations/policy_wiring_seam_contract.md`
2. [Observed] Latency budget contract is explicit and CI-visible:
   - `crates/tze_hud_telemetry/src/validation.rs` (`POLICY_MUTATION_EVAL_BUDGET_US = 50`, `evaluate_policy_mutation_latency_conformance`)
3. [Observed] Protocol-side policy admission reporting captures outcomes, per-level diagnostics, and latency conformance summaries:
   - `crates/tze_hud_protocol/src/session_server.rs` (`PolicyAdmissionReport`, conformance logging paths)
4. [Observed] Targeted verification for the pilot evidence paths passes:
   - `cargo test -p tze_hud_telemetry mutation_path_latency_conformance`
   - `cargo test -p tze_hud_protocol test_policy_admission_report_`

## Why This Is The Honest V1 Path

1. [Observed] Doctrine and spec already separate v1 runtime authority from target unified policy wiring.
2. [Observed] The mutation pilot is implemented and instrumented with a hard p99 budget contract.
3. [Inferred] This is sufficient to keep bounded policy wiring in v1 without implying broader hot-path unification that is not implemented.
4. [Observed] There is no evidence requiring immediate v1 scope shrink to remove the mutation pilot itself.

## Explicit Non-Claims (v1)

1. [Observed] v1 does not claim full seven-level unified arbitration wiring across frame/event hot paths.
2. [Observed] v1 does not move mutable runtime authority into `tze_hud_policy`.
3. [Observed] Unified hot-path policy wiring remains v2 follow-on work.

## Follow-On Expectations

1. [Observed] Reconciliation work (`hud-s98v.5`) should verify retained v1 MUST claims map to code/tests and explicitly defer non-retained target-state claims.
2. [Observed] The shrink-path bead (`hud-s98v.4`) is not required for this decision path unless later evidence invalidates bounded-pilot retention.
