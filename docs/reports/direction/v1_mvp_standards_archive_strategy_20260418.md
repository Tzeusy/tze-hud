# v1-mvp-standards Archive Strategy — Decision

Date: 2026-04-18
Issue: `hud-e3yf` (parent epic: `hud-o733`)
Gate: decide one-shot archive (Model A) vs. living standards (Model B)
Inputs:
- `openspec/changes/v1-mvp-standards/` (proposal.md, design.md, tasks.md, specs/)
- `openspec/specs/{12 capabilities}/spec.md` (post-sync canonical set)
- `openspec/changes/archive/2026-04-18-session-resource-upload-rfc0011/` (sister pattern)
- Sync history via beads hud-etry, hud-h3eb, hud-cs50, hud-65i8, hud-hc86, hud-2z4b, hud-7f7m, hud-ksnl, hud-0agx, hud-xttl, hud-66gj, hud-3daf
- Git log of commits touching `openspec/changes/v1-mvp-standards/` on `origin/main`

## Decision

**MODEL A (one-shot change; archive after terminal reconciliation).**

One-line justification: all 12 capability specs already live authoritatively under `openspec/specs/` via 12 merged sync PRs; keeping the change open only preserves a stale mirror and a 0/118-checkbox task list while new deltas already route around it to the canonical specs.

## Supporting rationale

Five independent signals converge on Model A. None of them survive serious contact with Model B.

1. **Sync is complete.** All 12 v1 subsystem specs have landed canonically on `origin/main`:
   - `configuration` (hud-cs50, PR #467), `timing-model` (hud-etry, PR #464), `validation-framework` (hud-h3eb, PR #469), `session-protocol` (hud-65i8, PR #468), `scene-events` (hud-hc86, PR #470), `runtime-kernel` (hud-2z4b, PR #471), `resource-store` (hud-ksnl, PR #473), `lease-governance` (hud-0agx, PR #474), `policy-arbitration` (hud-7f7m), plus `scene-graph`/`input-model`/`system-shell` reconciled via hud-66gj, hud-3daf, hud-xttl.
   - Every sync bead closed with a merged PR. The original goal stated in `openspec/changes/v1-mvp-standards/proposal.md` — "Creates 12 new capability specs covering every v1 subsystem" — is satisfied in `openspec/specs/`, not in the change directory.

2. **The "living" evidence is weaker than the briefing suggests.** The hud-bs2q amendment cited as living-document behavior touched `about/heart-and-soul/v1.md`, RFCs 0001/0004, and the `persistent-movable-elements` change directory — it did NOT edit `openspec/changes/v1-mvp-standards/`. The amendments that DID land in the change dir post-creation (hud-lyl7 resident-upload sync, hud-q2uz text-stream-portals dedupe, hud-u77s MUST/SHALL normalization, hud-1jd5/hud-za9y telemetry rescoping, the hud-iq2x.* policy closeout series) were all **pre-sync drift repair to clean the change before its specs landed canonically**. After the sync beads executed, that amendment stream has effectively stopped targeting the change dir — new deltas (`persistent-movable-elements`, `widget-system`, `component-shape-language`) modify canonical specs, not `v1-mvp-standards/specs/`.

3. **Sister pattern is unambiguous.** `session-resource-upload-rfc0011` (hud-lyl7) followed exactly this arc: synthesized spec deltas, synced them into v1-mvp-standards + canonical specs, then archived to `openspec/changes/archive/2026-04-18-session-resource-upload-rfc0011/` with a reconciliation memo (`reconciliation-hud-ooj1.1.md`). Seventeen other archived changes in `openspec/changes/archive/` follow the same one-shot model. There is no precedent in this repo for a perpetually-open "living standards" change; Model B would require inventing a new governance pattern with no existing tooling support.

4. **Keeping the change open creates active harm.**
   - **Two sources of truth.** Any future amendment must choose between `openspec/changes/v1-mvp-standards/specs/<cap>/spec.md` (the change mirror) and `openspec/specs/<cap>/spec.md` (the canonical). Reviewers and LLM implementers cannot know which is authoritative without runtime ceremony. The bc86dd4 "fold post-merge reviewer fixes onto timing-model and scene-graph specs" commit already had to edit both to stay coherent — that maintenance burden compounds.
   - **Stale task list.** `tasks.md` has 0 of 118 checkboxes checked despite substantial v1 implementation being landed (visible via closed epics hud-lviq, hud-ooj1, hud-bs2q, hud-bm9i, hud-t98e, hud-7yaf, hud-iq2x, hud-iwzd, and others). The catalog bead `hud-cek0` exists precisely because this task list is governance dead weight. A one-shot archive retires the catalog along with the change; Model B would demand keeping it fresh in perpetuity.
   - **OpenSpec tooling assumption.** `openspec validate <change> --strict` and `/opsx:archive` both assume changes are transient. Living changes would fight the tooling rather than ride it.

5. **Doctrine supports Model A.** `about/heart-and-soul/v1.md` is the load-bearing scope document and is itself the living artifact — RFCs amend it, deltas amend it, closed epics amend it. Capability specs under `openspec/specs/` are the second living layer, amended via scoped delta changes per normal OpenSpec workflow. There is no gap that a perpetually-open `v1-mvp-standards` change fills — v1.md already performs the living-scope role, and canonical specs already perform the living-normative-contract role.

The only argument surfaced for Model B is the observation that the change dir has received amendments over time. That observation is correct but mis-framed: amendments landed in the change dir when that was the authoritative location (before sync). Now that canonical specs are authoritative, new amendments should route there. Archiving the change cements this routing decision; keeping it open re-opens the ambiguity.

## Next actions (Model A execution)

These actions belong to beads already in the `hud-o733` tree. This memo does not execute any of them; it unblocks `hud-d9il` and clarifies `hud-cek0`'s scope.

| Bead | Action under Model A | Status |
|---|---|---|
| `hud-cek0` (task catalog) | **KEEP OPEN, rescope to terminal.** Since the change will archive, the catalog's job is to record a final disposition of each of the 118 tasks (done / tracked-elsewhere / deferred) so archival captures truth. No line-by-line bead filing for items already done or tracked under existing epics. Outcome: a one-shot annotated `tasks.md` delivered to main, then the change archives. | open, P2 |
| `hud-d9il` (terminal reconciliation) | **UNBLOCK once `hud-cek0` closes.** Executes `/opsx:archive v1-mvp-standards` (or the equivalent OpenSpec invocation), confirms the change lands under `openspec/changes/archive/2026-NN-NN-v1-mvp-standards/`, diffs the 12 archived specs against canonical to prove parity, runs `openspec validate --strict`, writes the epic closeout report at `about/legends-and-lore/reports/hud-o733-v1-mvp-standards-sync-closeout.md`, and closes `hud-o733`. | open, P1 |
| `hud-o733` (epic) | **KEEP OPEN** until `hud-d9il` closes. This memo does not close it. | open, P1 |
| Future v1 spec amendments | Route to `openspec/specs/<capability>/spec.md` directly via scoped delta changes (e.g., the pattern used by `persistent-movable-elements` and `widget-system`). Do NOT amend `openspec/changes/v1-mvp-standards/` further; treat it as frozen pending archive. | policy |

## Explicitly out of scope for this decision

- **Implementation of the 118 tasks.** Those are tracked via sibling epics (hud-lviq, hud-ooj1, hud-bs2q, etc.) and will continue on their own beads. The decision here does not re-open any implementation scope.
- **Archive ordering.** Whether `hud-cek0` must close before `hud-d9il` archives (strict), or whether `hud-d9il` can archive a tasks.md carrying unchecked items (lenient), is a `hud-d9il` workflow decision. This memo recommends strict ordering (catalog first, archive second) to preserve a clean audit trail, but does not mandate it.
- **Archive date naming.** `/opsx:archive` will stamp the directory with the execution date; we do not pre-commit to 2026-04-18.
- **Regeneration.** If a future v1 spec refactor warrants a new comprehensive pass, it should be filed as a fresh OpenSpec change (e.g., `v1-mvp-standards-v2` or capability-scoped), not by reopening this one.

## Acceptance of this decision

1. This memo exists at `docs/reports/direction/v1_mvp_standards_archive_strategy_20260418.md` with a single unambiguous outcome (Model A: one-shot, archive) — satisfies acceptance criterion "Decision memo exists".
2. The next-action list above names `hud-cek0` (rescope) and `hud-d9il` (unblock to archive) as the concrete follow-ups — satisfies "next-action list".
3. Epic `hud-o733` description is updated separately (by this bead's final step) to record Model A as the governance decision, unblocking `hud-d9il` — satisfies "unambiguous outcome".
4. No code, spec, or change-directory edits are made by `hud-e3yf` itself. Archive execution is `hud-d9il`'s job; task catalog is `hud-cek0`'s job.
