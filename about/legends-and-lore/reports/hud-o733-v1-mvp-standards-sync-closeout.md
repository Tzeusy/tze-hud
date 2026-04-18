# hud-o733 Closeout: v1-mvp-standards Sync, Reconciliation, and Archive

**Epic**: hud-o733 — v1-mvp-standards: sync, reconcile, and define archive strategy
**Closeout bead**: hud-d9il (terminal reconciliation + archive execution)
**Date**: 2026-04-18
**Outcome**: All 12 capability specs synced, 3 drifted specs reconciled, change archived under Model A (one-shot).

---

## 1. What Was Synced

The `v1-mvp-standards` OpenSpec change created 12 capability specs covering the complete v1 subsystem surface. All 12 were synced from `openspec/changes/v1-mvp-standards/specs/` into the authoritative `openspec/specs/` directory via dedicated sync beads, each landing as a merged PR on main.

| Capability | Bead | PR | Type |
|---|---|---|---|
| configuration | hud-cs50 | #467 | First-time sync |
| timing-model | hud-etry | #464 | First-time sync |
| validation-framework | hud-h3eb | #469 | First-time sync |
| session-protocol | hud-65i8 | #468 | First-time sync |
| scene-events | hud-hc86 | #470 | First-time sync |
| runtime-kernel | hud-2z4b | #471 | First-time sync |
| resource-store | hud-ksnl | #473 | First-time sync |
| lease-governance | hud-0agx | #474 | First-time sync |
| policy-arbitration | hud-7f7m | #472 | First-time sync |
| scene-graph | hud-66gj | #463 | Reconciliation (drifted) |
| input-model | hud-3daf | #465 | Reconciliation (drifted) |
| system-shell | hud-xttl | #462 | Reconciliation (drifted) |

All 12 sync/reconciliation beads are closed. All PRs are merged to main.

---

## 2. Drifted Specs Reconciled

Three capability specs had pre-existing content under `openspec/specs/` from the earlier `text-stream-portals` change archive. These required content-merge care rather than a straight copy-in.

**scene-graph** (hud-66gj, PR #463): The 14-line text-stream-portals stub (covering SceneId stability, PortalNode registration, and a `text-stream-portals` activation event requirement) was merged into the 367-line v1-mvp-standards content. The union spec is 401 lines. No text-stream-portals requirements were dropped.

**input-model** (hud-3daf, PR #465): The 19-line text-stream-portals stub (covering portal interaction reuse of the local-first input model) was folded into the v1-mvp-standards content. Preserved verbatim as a dedicated `### Requirement: Text Stream Portal Interaction Reuses Local-First Input` section with scenario. The pointer capture requirement received a clarification distinguishing agent-requested capture semantics (PointerDownEvent only) from runtime-owned chrome interactions (may acquire at other phases). PR #465 squash-merged reviewer fixes.

**system-shell** (hud-xttl, PR #462): The text-stream-portals stub (covering the chrome/portal boundary — portals remain outside chrome, portal shell override semantics) was preserved as two distinct requirements. The boundary between system shell authority and text stream portal surfaces is explicitly documented. Shell override scenarios apply to portal tiles under the same unconditional rules as any content-layer tile.

---

## 3. Drift Analysis: Change Dir vs. Canonical Specs

At archive time, all 12 canonical specs diverge from their counterparts in the change directory. All observed drift is expected and correct:

**Structural transformations (all specs)**: Section title `# <Title> Specification` normalized to `# <capability> Specification`; `## ADDED Requirements` resolved to `## Requirements`; Purpose sections populated (from `TBD` or absent); horizontal rule separators added between requirement blocks.

**Content additions (subset of specs)**: Several canonical specs received additional requirements from other changes that touched those specs after the sync PRs landed. Specifically:
- `configuration`: Design token configuration section added from `component-shape-language` change (PR in `f53bbdd` bulk-archive commit); mobile profile normative language refined (`MUST NOT be implemented` vs. `exercised post-v1 only`); `tab_switch_on_event` regex constraint clarified; `SILENT` quiet-hours behavior made more precise.
- `resource-store`: IMAGE_SVG validation semantics clarified (5 raster/font + 1 vector, not 6 uniform types); zone publish restriction on IMAGE_SVG explicit; three new scenarios added for SVG validation edge cases.
- `policy-arbitration`: Attention budget language normalized (`shed` vs. `discarded` for LOW; `non-SILENT mutations` coalesced); scenario MUST language strengthened.
- `runtime-kernel`: Comma formatting in numeric ranges (5,000ms); Spec-First Handoff Notes section dropped from canonical (those were pre-sync implementation guidance, not normative requirements).
- `session-protocol`: MutationProto oneof count updated from 9 to 10 variants (PublishToTileMutation added); widget type list expanded.

None of this drift indicates reconciliation gaps. All differences reflect forward progress on canonical specs after the sync PRs landed. The v1-mvp-standards change directory was frozen as a source of truth the moment each sync PR merged; amendments since then correctly targeted `openspec/specs/` directly.

**No unexpected drift was found. No follow-up beads required for drift.**

---

## 4. Validation Results

Before archive:
- All 12 canonical capability specs: **PASS** (`openspec validate <cap> --strict`)
- `v1-mvp-standards` change: 6 errors in deferred-requirement descriptions (pre-existing — "Stylus and Pressure Input (Post-v1)", "Dynamic Policy Rules", "Incremental Diff (Deferred)", "VideoSurfaceRef and WebRtcRequired (Deferred)", "Zone Occupancy Query API (Deferred)", "Full TypeScript Inspector"). These were non-normative markers documenting post-v1 deferrals; their absence from canonical specs confirms they were intentionally not synced.

After archive:
- `openspec validate --all --strict`: 34 passed, 3 failed (37 items). The 3 remaining failures are pre-existing and unrelated to v1-mvp-standards: `spec/component-shape-language`, `spec/exemplar-status-bar`, `change/mcp-stress-testing`.
- `change/v1-mvp-standards` no longer appears in validation results (correctly).

---

## 5. Task Catalog Disposition (hud-cek0)

The 118 unchecked tasks in `openspec/changes/v1-mvp-standards/tasks.md` were given terminal dispositions via hud-cek0 (PR #481, merged to main).

- **DONE: 109** — Checked off with inline attribution to Rust crate files and closed beads. Implementation exists in `crates/tze_hud_*` for all DONE items.
- **DEFERRED: 9** — Items where the framework exists but full CI validation gates are not yet confirmed:
  - Layer 3 benchmark binary with JSON emission (12.5)
  - 25-scene test registry (12.7 — 3 scenes exist; 25-scene target not met)
  - Record/replay trace infrastructure (12.9 — trace.rs exists; full replay not verified)
  - Soak/leak harness 5% tolerance gate (12.10)
  - Capability name convergence audit (13.1 — spec fixed by hud-of3; runtime audit ongoing)
  - MCP guest/resident enforcement audit (13.2)
  - Protobuf field-by-field audit against session-protocol spec (13.3)
  - Full cross-spec integration test at 60fps (13.4)
  - Quantitative budget validation on reference hardware (13.5)

The 9 deferred items carry rationale annotations in tasks.md. No new beads were required — all deferred items are either tracked under existing epics or represent CI gate completion work that can be addressed as implementation matures.

---

## 6. Archive Decision and Execution

**Decision (hud-e3yf)**: Model A — one-shot archive. Decision memo at `docs/reports/direction/v1_mvp_standards_archive_strategy_20260418.md`.

**Rationale summary**: All 12 specs authoritatively live under `openspec/specs/`; the change directory was a stale mirror after sync PRs merged. Keeping it open creates two sources of truth. The sister pattern (`session-resource-upload-rfc0011`) follows the same one-shot arc. No precedent for living standards changes in this repo; tooling assumes transient changes. Model B was rejected on all five independent signals evaluated in the decision memo.

**Execution**: `mv openspec/changes/v1-mvp-standards openspec/changes/archive/2026-04-18-v1-mvp-standards` (executed by hud-d9il on 2026-04-18). Change contents preserved in full at archive location.

**Archive contents**:
```
openspec/changes/archive/2026-04-18-v1-mvp-standards/
  proposal.md
  design.md
  tasks.md  (109 checked, 9 deferred)
  specs/
    configuration/spec.md
    input-model/spec.md
    lease-governance/spec.md
    policy-arbitration/spec.md
    resource-store/spec.md
    runtime-kernel/spec.md
    scene-events/spec.md
    scene-graph/spec.md
    session-protocol/spec.md
    system-shell/spec.md
    timing-model/spec.md
    validation-framework/spec.md
```

---

## 7. Acceptance Criteria Verification

| Criterion | Status |
|---|---|
| All 12 canonical specs match v1-mvp-standards content (with documented exceptions for 3 drifted specs) | SATISFIED |
| `openspec validate --strict` passes for all 12 v1 capability specs | SATISFIED — all 12 pass |
| Archive decision (Model A) executed | SATISFIED — change moved to `openspec/changes/archive/2026-04-18-v1-mvp-standards/` |
| Closeout report written | SATISFIED — this document |
| Epic hud-o733 closed | SATISFIED — closed by hud-d9il as final act |

---

## 8. Bead Trail

| Bead | Role | Status | PR |
|---|---|---|---|
| hud-o733 | Epic | closed (by hud-d9il) | — |
| hud-e3yf | Archive strategy decision | closed | — (decision memo) |
| hud-cek0 | Task catalog terminal disposition | closed | #481 |
| hud-d9il | Terminal reconciliation + archive | closed by coordinator | this PR |
| hud-cs50 | configuration sync | closed | #467 |
| hud-etry | timing-model sync | closed | #464 |
| hud-h3eb | validation-framework sync | closed | #469 |
| hud-65i8 | session-protocol sync | closed | #468 |
| hud-hc86 | scene-events sync | closed | #470 |
| hud-2z4b | runtime-kernel sync | closed | #471 |
| hud-ksnl | resource-store sync | closed | #473 |
| hud-0agx | lease-governance sync | closed | #474 |
| hud-7f7m | policy-arbitration sync | closed | #472 |
| hud-66gj | scene-graph reconciliation | closed | #463 |
| hud-3daf | input-model reconciliation | closed | #465 |
| hud-xttl | system-shell reconciliation | closed | #462 |

**Total PRs landed**: 12 + 1 (task catalog) = 13 PRs on main.

---

## 9. Forward Policy

Future v1 spec amendments MUST route to `openspec/specs/<capability>/spec.md` directly via scoped delta changes. The pattern established by `persistent-movable-elements` and `widget-system` is the reference. Do NOT amend `openspec/changes/archive/2026-04-18-v1-mvp-standards/`; it is a frozen audit artifact.

The 9 deferred task items from tasks.md are monitoring responsibilities for ongoing implementation epics. They do not require new beads unless a gate failure is confirmed.
