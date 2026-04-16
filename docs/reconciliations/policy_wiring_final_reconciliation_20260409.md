# Policy Wiring Final Reconciliation (Gen-2)

Date: 2026-04-09
Issue: `hud-iq2x`
Scope: Verify policy-wiring direction outputs against required evidence set and epic acceptance criteria.

## Inputs Audited

- `docs/reconciliations/policy_wiring_epic_prompt.md`
- `docs/reconciliations/policy_wiring_direction_report.md`
- `docs/reconciliations/policy_wiring_execution_backlog.md`
- `docs/reconciliations/policy_wiring_human_signoff_report.md`
- `docs/reconciliations/policy_wiring_gen1_reconciliation.md`
- `docs/reconciliations/policy_wiring_direction_report_20260409.md`
- `bd show hud-iq2x --json`

## Evidence Set Verification

| Required evidence family | Coverage | Key references |
|---|---|---|
| Doctrine (`heart-and-soul`) | Covered | `about/heart-and-soul/v1.md:11-21`, `about/heart-and-soul/architecture.md:1-17`, `about/heart-and-soul/privacy.md:1-20`, `about/heart-and-soul/attention.md:1-16`, `about/heart-and-soul/validation.md:1-20` |
| OpenSpec (policy/runtime/session) | Covered | `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:11-22`, `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:217-230`, `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:59-73`, `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:616-636` |
| Runtime/Policy/Protocol code | Covered | `crates/tze_hud_runtime/src/lib.rs:12-26`, `crates/tze_hud_runtime/src/budget.rs:18-35`, `crates/tze_hud_policy/src/lib.rs:31-46`, `crates/tze_hud_protocol/src/session_server.rs:2844-3004`, `crates/tze_hud_protocol/src/session_server.rs:3455-3527` |
| Scene-side policy contract | Covered | `crates/tze_hud_scene/src/policy/mod.rs:1-7`, `crates/tze_hud_scene/src/policy/mod.rs:170-204` |
| Lease governance RFC seam | Covered | `about/legends-and-lore/rfcs/0008-lease-governance.md:101-109` |

## Epic Acceptance-Criteria Reconciliation

| Epic criterion | Reconciliation result | Status |
|---|---|---|
| 1. Direction report identifies contradictions and tractable path | Contradictions and phased tractable path are explicit in both baseline and Gen-2 direction reports. | Pass |
| 2. Names exact seams runtime-owned vs policy-owned | Gen-2 report adds explicit runtime/policy/scene/session ownership matrix and seam requirements. | Pass |
| 3. Follow-on implementation/spec beads created with dependencies and acceptance criteria | `hud-iq2x.5`/`.6`/`.7`/`.8` exist and map to governance-first seams; scope-decision (`hud-ew7a`) and reconciliation/signoff (`hud-p48t`, `hud-5fb1`) are instantiated, while pilot/telemetry beads remain pending. | Partial |
| 4. Human-readable report summarizes path, risks, and stop/do-not-do-yet items | Human signoff report plus Gen-2 report explicitly provide path, risks, and anti-goals. | Pass |
| 5. Final reconciliation verifies coverage | This document performs the final evidence and criterion reconciliation against current repo and bead state. | Pass |

## Current Execution Readiness

### Ready/Active sequence

1. `hud-iq2x.5` (lease scope fix) is closed.
2. `hud-iq2x.6` (spec reconciliation) is closed.
3. `hud-iq2x.7` (seam contract) is blocked on PR review flow.
4. `hud-iq2x.8` (capability escalation semantics) is closed.
5. `hud-ew7a` (event/frame scope decision after telemetry) is instantiated and `in_progress`.

### Still missing for full decomposition closure

1. Mutation-path pilot implementation bead.
2. Pilot telemetry + latency conformance bead.

### Post-pilot reconciliation now instantiated

- `hud-p48t` now tracks the post-pilot reconciliation bead.
- `hud-p48t` remains dependency-gated on mutation pilot, telemetry, scope decision (`hud-ew7a`), and spec reconciliation completion.

### Closure bead now instantiated

- `hud-5fb1` now tracks the final human signoff closure artifact for the policy-wiring program.
- `hud-5fb1` should remain dependency-gated on the post-pilot reconciliation work before epic closure.

## Conclusion

[Observed] Direction artifacts are now coherent with doctrine/spec/code evidence and explicitly model the three policy surfaces.

[Observed] The decomposition is only partially materialized in tracker state: governance-first beads, scope-decision (`hud-ew7a`), post-pilot reconciliation (`hud-p48t`), and final closure signoff (`hud-5fb1`) are present, but the downstream pilot/telemetry beads still require creation by the coordinator.

[Inferred] Closing `hud-iq2x` should wait for those remaining follow-on beads to be instantiated and linked, then for `hud-p48t` and `hud-5fb1` closure work to complete after those prerequisites land.
