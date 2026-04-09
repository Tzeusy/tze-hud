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
| OpenSpec (policy/runtime/session) | Covered | `openspec/.../policy-arbitration/spec.md:11-22`, `openspec/.../policy-arbitration/spec.md:217-230`, `openspec/.../runtime-kernel/spec.md:59-73`, `openspec/.../session-protocol/spec.md:616-636` |
| Runtime/Policy/Protocol code | Covered | `crates/tze_hud_runtime/src/lib.rs:12-26`, `crates/tze_hud_runtime/src/budget.rs:18-35`, `crates/tze_hud_policy/src/lib.rs:31-46`, `crates/tze_hud_protocol/src/session_server.rs:2844-3004`, `crates/tze_hud_protocol/src/session_server.rs:3455-3527` |
| Scene-side policy contract | Covered | `crates/tze_hud_scene/src/policy/mod.rs:1-7`, `crates/tze_hud_scene/src/policy/mod.rs:170-204` |
| Lease governance RFC seam | Covered | `about/law-and-lore/rfcs/0008-lease-governance.md:101-109` |

## Epic Acceptance-Criteria Reconciliation

| Epic criterion | Reconciliation result | Status |
|---|---|---|
| 1. Direction report identifies contradictions and tractable path | Contradictions and phased tractable path are explicit in both baseline and Gen-2 direction reports. | Pass |
| 2. Names exact seams runtime-owned vs policy-owned | Gen-2 report adds explicit runtime/policy/scene/session ownership matrix and seam requirements. | Pass |
| 3. Follow-on implementation/spec beads created with dependencies and acceptance criteria | `hud-iq2x.5`/`.6`/`.7`/`.8` exist and map to governance-first seams; remaining pilot/telemetry/scope/signoff beads are still not instantiated as tracker items. | Partial |
| 4. Human-readable report summarizes path, risks, and stop/do-not-do-yet items | Human signoff report plus Gen-2 report explicitly provide path, risks, and anti-goals. | Pass |
| 5. Final reconciliation verifies coverage | This document performs the final evidence and criterion reconciliation against current repo and bead state. | Pass |

## Current Execution Readiness

### Ready/Active sequence

1. `hud-iq2x.5` (lease scope fix) is active but blocked on PR review flow.
2. `hud-iq2x.6` (spec reconciliation) is active but blocked on PR review flow.
3. `hud-iq2x.7` (seam contract) and `hud-iq2x.8` (capability escalation semantics) are open and represent the correct next non-implementation governance steps.

### Still missing for full decomposition closure

1. Mutation-path pilot implementation bead.
2. Pilot telemetry + latency conformance bead.
3. Event/frame scope decision bead.
4. Post-pilot reconciliation bead.
5. Final human signoff closure bead.

## Conclusion

[Observed] Direction artifacts are now coherent with doctrine/spec/code evidence and explicitly model the three policy surfaces.

[Observed] The decomposition is only partially materialized in tracker state: governance-first beads are present, but the downstream pilot/telemetry/scope/reconciliation/signoff beads still require creation by the coordinator.

[Inferred] Closing `hud-iq2x` should wait for those remaining follow-on beads to be instantiated and linked after `.5`-`.8` governance work lands.
