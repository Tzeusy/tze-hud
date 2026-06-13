# Epic Report: Audit 2026-06-12 remediation: shape & doc refresh tranche

**Epic ID**: `hud-w0jfp`
**Date**: 2026-06-14
**Status**: 8/9 children closed (open — this report bead hud-w0jfp.9)
**Priority**: 2
**Source audit**: `docs/audits/20260612_project_review.md` §4, §5.13, §8 risk 8, §9 Q3/M5, §10

---

## 1. Overview

This epic closed the shape and documentation gaps surfaced by the 2026-06-12 full project review. Seven parallel implementation beads addressed every finding in the audit's §4 (shape findings 1–7), §5.13 (AGENTS.md), §8 risk 8 (doc estate accretion), §9 Q3/M5 quick wins, and §10 required shape work, with a mandatory change-tier reconciliation run on each normative artifact. The gen-1 reconciliation bead (hud-w0jfp.8) closed CLEAN: all seven siblings verified against the working tree, zero in-scope corrections required.

The tranche delivered: a corrected v1.md scope aligned to the text-stream-portal pivot; a fully refreshed topology inventory including the previously omitted `tze_hud_projection` crate; consistent deferral stamping on RFCs 0014/0018 and the parked mobile crates; two fixed `Purpose: TBD` spec headers plus an `Implementation:` source-reference convention backfilled across seven families; a regenerated AGENTS.md with a single coherent tracker block; a `docs/README.md` normative/episodic index; amended engineering-bar review mechanics with a production-call-site review standard; and a reconciled openspec change ledger (projection change archived, portal tasks.md checkboxes aligned to landed PRs).

No code was changed. All work is confined to normative documentation, specs, and the openspec change ledger.

---

## 2. Before / After Shape Scan

### BEFORE (2026-06-12 audit baseline — `docs/audits/20260612_project_review.md` §1)

```
Shape assessment: SHAPED — Full structure present, authored content incomplete in 2 pillars

Pillar 1: Doctrine         AUTHORED (14 files)
Pillar 2: Design Contracts MIXED — scaffold remnants (RFC 0004:258 <TBD> field number,
                                   RFC 0014 §4.2 TBD; RFCs 0014/0018 lack deferral banners)
Pillar 3: Capability Specs MIXED — 2 promoted specs with literal 'Purpose: TBD' headers;
                                   50/106 specs lack source references
Pillar 4: Topology         AUTHORED — but stale (omits tze_hud_projection; documents
                                      deferred v2 media subsystems as extant)
Pillar 5: Eng. Standards   AUTHORED (engineering-bar.md, 175 lines)

Known contradictions: RFC count wrong in 3 of 4 authoritative places (README 11,
components.md 13-but-lists-12, CLAUDE.md 13, filesystem 14); crate count (README 15,
actual 16+app); v1.md ships-list predates portal pivot; failure.md E25 pointer collides
with RFC 0014.

Scaffold-remnant flags:
  - openspec/specs/drag-to-reposition/spec.md:4   — "Purpose: TBD - synced from ..."
  - openspec/specs/element-identity-store/spec.md:4 — "Purpose: TBD - synced from ..."
  - RFC 0014 §4.2 TBD (stale post-RFC-0018)
```

### AFTER (2026-06-14 shape scan — run post-reconciliation close)

```
=== Project Shape Scan ===
Root: /home/tze/gt/tze_hud/mayor/rig

## Pillar 1: Doctrine (WHY)
  [FOUND] about/heart-and-soul/ (14 markdown files)
  Content: AUTHORED — no scaffold markers detected in markdown files
    - vision.md (91 lines)    - v1.md (193 lines)
    - architecture.md (334)   - security.md (105)
    - failure.md (126 lines)  - development.md (189)
    - validation.md (231)     - README.md (52)

## Pillar 2: Design Contracts (HOW)
  [FOUND] about/legends-and-lore/ (58 markdown files)
  Content: MIXED — some authored content, some scaffold/template content remains
    - Contracts: 14 documents in about/legends-and-lore/rfcs/ (15 authored)
    - Reviews: 15 review documents
    (Residual MIXED flag: RFC 0004:258 <TBD> field number is an out-of-scope
     placeholder, not addressed by this tranche — no deferral-banner or Purpose-TBD
     flags remain in scope items)

## Pillar 3: Capability Specs (WHAT)
  [FOUND] openspec/ (200 markdown files)
  Content: MIXED — some authored content, some scaffold/template content remains
  Specs with source references: 58/108
    (Purpose-TBD flags cleared; source-ref coverage gap is a convention-rollout
     backlog item, not this tranche — see follow-up hud-5jbra.8 / filed note)

## Pillar 4: Topology (WHERE)
  [FOUND] about/lay-and-land/ (7 markdown files)
  Content: AUTHORED — no scaffold markers detected in markdown files
    - components.md (211 lines)   - data-flow.md (471)   - README.md (26)

## Pillar 5: Engineering Standards (WHO WE ARE WHEN WE BUILD)
  [FOUND] about/craft-and-care/ (2 markdown files)
  Content: AUTHORED — no scaffold markers detected in markdown files
    - engineering-bar.md (192 lines)   - README.md (19)

## Shape Summary
  Pillars present: 5/5
  Local skills installed: 5/5
  Pillars needing authoring: 2/5 (Pillar 2 RFC remnant; Pillar 3 source-ref gap)
  Local skill templates still uncustomized: 0/5
  Assessment: SHAPED — Full structure present, authored content still incomplete
               in 2 pillars (residual, out-of-scope of this tranche)
```

**Scaffold-remnant flags cleared**: YES — both `Purpose: TBD` headers (drag-to-reposition and element-identity-store) confirmed absent. Reconciliation bead hud-w0jfp.8 close reason: "tranche RECONCILES CLEAN — all 7 siblings verified, zero in-scope corrections; 1 follow-up filed for convention-rollout backlog."

---

## 3. Implementation Chronicle

### hud-w0jfp.1 — Amend v1.md: touch posture, macOS lane criterion, introspection scope, portal surface

**PR**: [#712](https://github.com/Tzeusy/tze-hud/pull/712) (merged 2026-06-12; commit `eceaa5ba`)
**Normative artifact**: `about/heart-and-soul/v1.md`

The audit identified four ships-claims in v1.md contradicted by current code reality or project intent post-portal-pivot:

1. **Touch input (v1.md:89)** — zero touch event paths existed in the runtime (doc comment only at `windowed.rs:318`). Amended to explicitly defer touch to post-v1 with an open bead citation.
2. **macOS headless CI lane (v1.md:185)** — CI runs ubuntu+windows only; the 2026-05-09 refocus implies amending the criterion, not building the lane. Criterion narrowed to the two running lanes.
3. **CLI/gRPC introspection surface (v1.md:161)** — partially realized via MCP `list_scene`/`list_zones`; scope narrowed to what exists; remaining build-out explicitly scheduled by open beads.
4. **Text-stream portals** — the dominant shipped surface (~40 of last 50 commits, RFC 0013, phase-1 change) was absent from the ships-list. Acknowledged as in-scope and linked to RFC 0013 and the openspec phase-1 change.

Change-tier reconciliation recorded per the th-projects protocol.

---

### hud-w0jfp.2 — Refresh topology + counts: components.md, README, CLAUDE.md; note a11y/media parked status

**PR**: [#734](https://github.com/Tzeusy/tze-hud/pull/734) (merged 2026-06-12)
**Normative artifacts**: `about/lay-and-land/components.md`, `README.md`, `CLAUDE.md`

Pre-tranche state: components.md:7 claimed "Core crates (13)", omitting `tze_hud_projection` entirely (49 tests, its own spec family). RFC/crate/LOC counts were wrong in 3 of 4 authoritative places. Media subsystem section presented deferred v2 components (decode worker pool, cpal audio routing) as if extant.

Delivered:
- All 16 crates + app listed with status labels (active / parked / unwired).
- `tze_hud_a11y` parked/zero-consumer status recorded.
- Media subsystem section re-labeled as deferred-design vs shipped-exception (windows-media-ingress path).
- RFC count (14), crate count (16+app), and LOC (~285k) consistent across README.md, components.md, and CLAUDE.md.

Change-tier reconciliation recorded.

---

### hud-w0jfp.3 — Deferral stamping: RFC 0014/0018 banners, mobile crate roots, failure.md E25 pointer fix

**PR**: [#736](https://github.com/Tzeusy/tze-hud/pull/736) (merged 2026-06-12)
**Normative artifacts**: `about/legends-and-lore/rfcs/rfc-0014-*.md`, `rfc-0018-*.md`, `tze_hud_media_apple/src/lib.rs`, `tze_hud_media_android/src/lib.rs`, `about/heart-and-soul/failure.md`

Pre-tranche state: RFC 0014 (media plane wire protocol) and RFC 0018 (WHIP signaling) carried no deferral banner despite v1.md:11–13 declaring media/mobile "deferred indefinitely". The openspec media specs already carried banners — the inconsistency was between RFC files and openspec. `tze_hud_media_apple` (1,321 LOC) and `tze_hud_media_android` (703 LOC) had plain workspace member status. `failure.md` deferred its E25 10-step degradation ladder to "RFC 0014 (forthcoming)" — a stale pointer that now collided with the existing media RFC 0014 title.

Delivered:
- RFC 0014 and RFC 0018 carry consistent deferral banners.
- Both mobile crate `lib.rs` roots state parked status and revival condition.
- `failure.md` E25 pointer updated: no longer references RFC 0014; v1 6-level mapping stated inline with citation to `degradation.rs`.

Change-tier reconciliation recorded.

---

### hud-w0jfp.4 — Spec hygiene: fix Purpose TBD headers; adopt Implementation source-ref convention + backfill 7 families

**PR**: [#732](https://github.com/Tzeusy/tze-hud/pull/732) (merged 2026-06-12)
**Normative artifacts**: `openspec/specs/drag-to-reposition/spec.md`, `openspec/specs/element-identity-store/spec.md`, `openspec/SPEC-FORMAT.md`, 7 spec families

Pre-tranche state: two promoted specs carried literal scaffold text `"Purpose: TBD - synced from persistent-movable-elements change [hud-mu38]"`. Separately, 50/106 specs lacked source references (convention gap, not staleness in the majority of cases).

Delivered:
- Both `Purpose: TBD` headers replaced with real purpose statements.
- `Implementation:` source-reference convention documented in `openspec/SPEC-FORMAT.md`.
- Seven audit-sampled families backfilled with verified real paths: scene-graph, widget-system, text-stream-portals, cooperative-hud-projection, publish-load-harness, element-identity-store, drag-to-reposition.

Verified via `openspec validate --specs` (36/39 pass; 3 pre-existing failures = hud-hzdn5, unrelated to this bead). Change-tier reconciliation recorded.

---

### hud-w0jfp.5 — Curate agent doc estate: regenerate AGENTS.md + add docs/ index

**PRs**: [#738](https://github.com/Tzeusy/tze-hud/pull/738) (merged 2026-06-12) + [#777](https://github.com/Tzeusy/tze-hud/pull/777) (merged 2026-06-13)
**Normative artifacts**: `AGENTS.md`, `docs/README.md`

Pre-tranche state: AGENTS.md (66 churn commits) contained two overlapping generated tracker blocks with contradictory sync mechanics, referenced nonexistent `docs/QUICKSTART.md`, and held ~100 flat "Notes to self" entries without thematic grouping. The `docs/` directory (129+ markdown files, 11 MB of evidence) had no index separating normative reference from episodic artifacts.

Delivered (PR #738 + cleanup PR #777):
- Single coherent tracker block; dead QUICKSTART link removed.
- Notes themed into subsections; zero entries lost (entry count verified before and after).
- `docs/README.md` index present with explicit normative/episodic split and archive policy.

Change-tier reconciliation recorded.

---

### hud-w0jfp.6 — Amend engineering-bar review mechanics + add production-call-site review standard

**PR**: [#754](https://github.com/Tzeusy/tze-hud/pull/754) (merged 2026-06-13)
**Normative artifacts**: `about/craft-and-care/engineering-bar.md`, `about/heart-and-soul/development.md`, PR-reviewer worker prompt

Pre-tranche state: `engineering-bar.md` §4 and `development.md:42` required "approval present" among merge conditions. Branch protection has `required_pull_request_reviews: null` and GitHub cannot approve self-authored PRs on a single-account agent fleet. The contradiction was acknowledged at `AGENTS.md:310` but the standards doc remained as written — the audit scored this as a low-tier finding (category 15) that must be resolved in writing.

Delivered:
- §4 amended to match achievable practice: required status checks + adversarial re-review as the operative merge gate.
- `development.md:42` made consistent.
- Production-call-site review standard added: "wire X" beads require grep-verified production call-sites before close; perf beads require empirical re-run of named payloads with timing assertions. Addresses the recurring "landed-but-not-live" delivery pattern (audit §8 risk 3).
- PR-reviewer worker prompt updated to encode the adversarial checklist.

Change-tier reconciliation recorded.

---

### hud-w0jfp.7 — OpenSpec change-ledger hygiene: archive projection change + reconcile portal tasks.md

**PR**: [#752](https://github.com/Tzeusy/tze-hud/pull/752) (merged 2026-06-13)
**Normative artifacts**: `openspec/changes/external-agent-projection-authority/` (archived), `openspec/changes/text-stream-portal-phase1/tasks.md`

Pre-tranche state: `openspec/changes/external-agent-projection-authority` was 18/18 tasks complete with live Windows evidence but remained unarchived. `openspec/changes/text-stream-portal-phase1/tasks.md` showed 2/53 checkboxes ticked while many covered PRs had landed (#683/#690/#693/#705/#707 and more) — the stale ledger contradicted the project's authoritative beads record.

Delivered:
- `external-agent-projection-authority` archived per the openspec archive flow.
- Portal `tasks.md` checkboxes reconciled to landed PRs (each entry cites a PR or bead ref; no checkbox marked done without a verifiable landed PR/bead).
- `windows-first-performant-runtime/tasks.md` untouched (scope boundary respected; owned by hud-iygbd).

Change-tier reconciliation recorded.

---

### hud-w0jfp.8 — Reconcile spec-to-code (gen-1) for shape & doc refresh tranche

**Status**: closed (2026-06-13)
**Assignee**: Opus (coordinator role)

Gen-1 reconciliation pass comparing the audit source findings (§4, §5.13, §10) against the documentation edits delivered by siblings .1–.7. Workflow: epic + sibling re-read → audit delivered edits → finding-to-bead checklist → shape scan re-run → reconciliation signoff.

**Outcome**: RECONCILES CLEAN — all 7 siblings verified against the working tree, zero in-scope corrections needed. Shape scan confirmed scaffold-remnant / Purpose-TBD flags cleared. One follow-up filed for the convention-rollout backlog (source-ref coverage gap across remaining 50 specs — not a correctness issue in this tranche).

Close reason (verbatim): "Gen-1 reconciliation (Opus): tranche RECONCILES CLEAN — all 7 siblings verified against tree, zero in-scope corrections needed. 1 follow-up filed for convention-rollout backlog."

---

## 4. Escalations

None. All seven implementation beads merged without escalation. The one follow-up item (backlog convention rollout for remaining spec source-refs) was correctly triaged as out-of-scope for this tranche and filed as a separate bead.

---

## 5. Subsequent Work

### Open follow-up (backlog)

The convention-rollout gap (50/108 specs still lacking `Implementation:` source-references after the 7-family backfill) was identified by the gen-1 reconciliation and filed as a backlog item. This is a convention propagation wave, not a correctness gap — the promoted specs are correctly authored, the source-ref convention is now documented in `openspec/SPEC-FORMAT.md`, and the backfill can proceed incrementally as specs are next touched.

The terminal portal reconciliation (tasks.md terminal sync, full openspec portal ledger close) remains with hud-5jbra.8.

### Deferred decisions

| Decision | Context | Revisit when |
|----------|---------|-------------|
| Source-ref backfill rollout across remaining 58 specs | Convention now documented; gap is coverage, not correctness | Next spec-hygiene wave or organic spec touch |
| RFC 0004:258 `<TBD>` field number | Out of scope for this tranche; pre-existing placeholder | RFC 0004 revision or protocol boundary change |

---

## 6. Risks & Notes for Reviewer

No new risks introduced; this epic was documentation-only.

The gen-1 reconciliation passed CLEAN, but a reviewer should spot-check two areas:

1. **hud-w0jfp.2 count consistency** — `README.md`, `components.md`, and `CLAUDE.md` should all read 14 RFCs / 16 crates + app / ~285k LOC Rust. A mismatch here would mean a partial merge.
2. **hud-w0jfp.3 RFC deferral banners** — RFC 0014 and RFC 0018 in `about/legends-and-lore/rfcs/` should carry the same deferral banner format used in the openspec media specs.

---

## Appendix A. Child Bead Summary

| Bead ID | Title | Status | PR | Merged |
|---------|-------|--------|----|--------|
| hud-w0jfp.1 | Amend v1.md: touch, macOS CI, introspection, portal scope | closed | [#712](https://github.com/Tzeusy/tze-hud/pull/712) | 2026-06-12 |
| hud-w0jfp.2 | Refresh topology + counts: components.md, README, CLAUDE.md | closed | [#734](https://github.com/Tzeusy/tze-hud/pull/734) | 2026-06-12 |
| hud-w0jfp.3 | Deferral stamping: RFC 0014/0018, mobile roots, failure.md E25 | closed | [#736](https://github.com/Tzeusy/tze-hud/pull/736) | 2026-06-12 |
| hud-w0jfp.4 | Spec hygiene: Purpose TBD fix + Implementation source-ref convention | closed | [#732](https://github.com/Tzeusy/tze-hud/pull/732) | 2026-06-12 |
| hud-w0jfp.5 | Curate agent doc estate: AGENTS.md + docs/README.md index | closed | [#738](https://github.com/Tzeusy/tze-hud/pull/738) + [#777](https://github.com/Tzeusy/tze-hud/pull/777) | 2026-06-12 / 2026-06-13 |
| hud-w0jfp.6 | Amend engineering-bar review mechanics + call-site standard | closed | [#754](https://github.com/Tzeusy/tze-hud/pull/754) | 2026-06-13 |
| hud-w0jfp.7 | OpenSpec ledger hygiene: archive projection, reconcile portal tasks | closed | [#752](https://github.com/Tzeusy/tze-hud/pull/752) | 2026-06-13 |
| hud-w0jfp.8 | Gen-1 reconciliation | closed | — | 2026-06-13 |
| hud-w0jfp.9 | Generate epic report (this document) | in_progress | — | — |

---

## Appendix B. Commits Referencing This Epic

```
17f01476 docs: refresh AGENTS.md + docs/README.md post-PR#738 [hud-w0jfp.5] (#777)
3f1c97a8 docs(engineering-bar): amend review mechanics + add production call-site standard [hud-w0jfp.6] (#754)
2a0a9f9e docs: archive external-agent-projection-authority; reconcile portal tasks.md ledger [hud-w0jfp.7] (#752)
2489f117 docs: regenerate AGENTS.md + add docs/README.md index [hud-w0jfp.5] (#738)
56683bd6 docs: deferral banners on RFC 0014/0018, parked notices on mobile crate roots, fix failure.md E25 pointer [hud-w0jfp.3] (#736)
0c75f1b1 docs(topology): refresh crate counts + inventory; add projection, media stubs, parked status [hud-w0jfp.2] (#734)
2868523f docs(specs): fix Purpose TBD headers; add Implementation source-ref convention + backfill 7 families [hud-w0jfp.4] (#732)
eceaa5ba docs(v1): amend touch posture, macOS lane criterion, introspection scope; acknowledge text-stream portals [hud-w0jfp.1] (#712)
a6404f33 docs: project-direction plan from 2026-06-12 audit — 32 beads across hud-1aswu/hud-w0jfp/hud-3qpgv + closeout wiring
```

---

## Appendix C. Source Audit Reference

`docs/audits/20260612_project_review.md` — findings mapped to this epic:

- §4 shape findings 1–7 → hud-w0jfp.1 through hud-w0jfp.7 (one-to-one coverage)
- §5.13 AGENTS.md contradictions → hud-w0jfp.5
- §8 risk 8 (doc estate accretion) → hud-w0jfp.5
- §9 Q3 / M5 quick wins (doc-refresh tranche) → all siblings
- §10 required shape work → hud-w0jfp.1, .2, .3, .4
