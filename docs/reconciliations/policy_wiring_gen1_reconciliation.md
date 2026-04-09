# Policy Wiring Direction Outputs — Gen-1 Reconciliation

**Issue:** `hud-iq2x.4`  
**Audit date:** 2026-04-08  
**Scope:** Verify that cited policy requirements and contradictions from the direction pass are either (a) addressed in artifacts, or (b) backed by explicit follow-on backlog.
**Coordinator source-of-truth:** `policy_wiring_execution_backlog.md`

## Inputs Audited

- `docs/reconciliations/policy_wiring_direction_report.md` (`hud-iq2x.1`)
- `docs/reconciliations/policy_wiring_execution_backlog.md` (`hud-iq2x.2`)
- `docs/reconciliations/policy_wiring_execution_backlog.proposed_beads.json` (`hud-iq2x.2`)
- `docs/reconciliations/policy_wiring_human_signoff_report.md` (`hud-iq2x.3`)
- `docs/reconciliations/policy_wiring_epic_prompt.md`
- `bd show hud-iq2x --json` (epic/children state)

## Requirement Coverage Matrix

| Direction requirement (`hud-iq2x.1`) | Artifact coverage | Status | Reconciliation result |
|---|---|---|---|
| R1: Runtime sovereignty over leases/capabilities | Direction + signoff both enforce runtime-owned state and seam-first wiring | Partial | **Addressed via follow-on** (`PW-01`, `PW-02`) |
| R2: Seven-level arbitration stack governs runtime decisions | Direction backlog defines staged wiring: mutation first, event/frame later | Unmet | **Addressed via follow-on** (`PW-03`, `PW-05`) |
| R3: Mutation/event/frame latency budgets must hold | Backlog includes telemetry + latency conformance gate | Partial | **Addressed via follow-on** (`PW-04`, `PW-05`) |
| R4: Mid-session capability escalation must be policy-validated | Direction identifies policy-source gap, but no dedicated backlog item exists | Partial | **Missing explicit bead** |
| R5: Dynamic policy rules deferred beyond v1 | Direction + signoff both mark as anti-goal/deferred | Met | **Addressed in outputs** |
| R6: Single-source runtime-vs-policy authority boundary | Backlog includes seam contract + ownership matrix before code | Partial | **Addressed via follow-on** (`PW-02`) |
| R7: Use epic prompt brief (`policy_wiring_epic_prompt.md`) | Prompt file exists in tree; missing-file claim has been resolved | Met | **Addressed and documented as historical in direction report** |

## Contradiction Coverage Matrix

| Cited contradiction (`hud-iq2x.1`) | Current state | Coverage result |
|---|---|---|
| Spec says full stack MUST run, runtime says policy crate not wired | Captured in direction + signoff; execution backlog plans spec reconciliation before wiring | **Addressed via `PW-01` + `PW-03`/`PW-05`** |
| Spec implies centralized stack ownership; code has distributed runtime ownership | Explicitly handled by seam-contract step | **Addressed via `PW-02`** |
| Epic prompt file missing | File now exists (`docs/reconciliations/policy_wiring_epic_prompt.md`) | **Resolved in repo; flagged historical in direction report** |

## Findings

1. Direction outputs are coherent and largely complete, but follow-on work is not yet instantiated as real beads under `hud-iq2x` (only documented as `PW-*` proposals).  
2. Requirement R4 (capability-escalation policy validation semantics) is identified as a gap in direction analysis, but missing as a dedicated execution bead.  
3. The "missing epic prompt file" contradiction has been resolved in repository state and is now retained as historical context in the direction report.

## Proposed Follow-On Beads (Coordinator To Apply)

Because worker lifecycle mutation is out of scope, these are proposed for coordinator creation:

1. **Materialize policy-wiring execution backlog proposals into real beads (`PW-01`..`PW-07`)**
   - Suggested type/priority: `task`, `P1`
   - Suggested deps: `discovered-from:hud-iq2x.4`
   - Rationale: Epic acceptance requires concrete follow-on beads, not proposal-only artifacts.

2. **Add explicit capability-escalation policy-validation bead**
   - Suggested title: `Define and implement capability escalation policy source semantics`
   - Suggested type/priority: `task`, `P1`
   - Suggested deps: `PW-01`, `PW-02` (or discovered-from `hud-iq2x.4` if `PW-*` not yet created)
   - Rationale: Closes R4 gap and prevents escalation semantics from being implied but unverified.

3. **Patch direction-report historical contradiction note**
   - Suggested title: `Annotate policy_wiring_direction_report with resolved prompt-file status`
   - Suggested type/priority: `docs`, `P2`
   - Suggested deps: `discovered-from:hud-iq2x.4`
   - Rationale: Keeps reconciliation artifacts internally consistent for future readers. (Completed by this coordinated follow-up.)

## Close-Reason Summary Text (For Coordinator)

`Gen-1 reconciliation complete: all cited policy contradictions are either covered in direction/signoff artifacts or mapped to explicit follow-on backlog, with two missing items identified for coordinator action (instantiate PW-01..PW-07 as real beads; add dedicated capability-escalation policy-validation bead).`
