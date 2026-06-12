# Project Direction: audit 2026-06-12 follow-ups → beads work plan

**Date**: 2026-06-12 | **Mode**: full direction analysis consuming a fresh `/project-review` packet (`docs/audits/20260612_project_review.md`, reviewed at HEAD `ba7a3139`)
**Reconciliation**: Phase 1 doctrine check verify-tier (consumed, not edited); Phase 2 verify-tier (no new OpenSpec changeset — every item traces to an existing doctrine/spec/engineering-bar section or is itself spec/doc maintenance); Phase 3 graph reviewed pre-creation by an independent verify-tier pass (verdict APPROVE-WITH-CHANGES; 2 MAJOR cross-dep corrections + 1 restored dropped item + 7 minor fixes applied) and mechanically validated post-creation (ready-set, dep wiring, acyclicity).

## Executive summary

tze_hud's real direction is a single-Windows, performance-proven presence runtime whose current delivery surface is text-stream portals; the doctrine machinery (leases, budgets, planes) is implemented and the remaining risk lives in enforcement infrastructure, not product substance. The audit verdict was **Healthy but fragile**: average 4.0/5, with the fragility concentrated in safety nets (auth bypass on default binds, dead perf lane, flake in the blocking gate, zero release tags, convention-only review).

This plan converts the audit's confirmed findings into 32 new beads across three new epics plus six closeout/standalone beads wired into the existing refocus epic — after a dedup pass against the live backlog that **removed one entire proposed workstream** (the portal host chain is already fully tracked: `hud-2iup7` → `hud-be6ee` → `hud-ttq97`/`hud-endkj` etc.) and **corrected a stale audit assumption** (the 60-minute three-agent soak already ran and closed as `hud-nfl7n` on 2026-05-12; release is blocked on its follow-ups `hud-i0jdz` and `hud-po7iz`, not a missing soak — the windows-first `tasks.md` checkbox was the stale artifact, which is itself finding §8-risk-4).

Highest-priority next work: (1) the enforcement-machinery epic `hud-1aswu` — LocalSocket auth fix and CI-gate hardening are small and risk-dominant; (2) `hud-w0jfp.1` (v1.md amendments) and `hud-olxxd` (validation-ops execution), because both now gate the closeout reconcile `hud-iygbd` and therefore the first release tag; (3) the existing portal host chain, which this plan deliberately does not touch.

## The graph (created 2026-06-12, all labeled `audit-20260612`)

### Epic `hud-1aswu` — enforcement machinery: security + CI trust (P1)
| Bead | Title | P |
|---|---|---|
| hud-1aswu.1 | Reject LocalSocket credential for non-loopback peers; default-bind loopback (security.md:19; auth.rs:91-95; windowed.rs:4024/4302) | 1 |
| hud-1aswu.2 | Require integration suite in branch protection + strict up-to-date checks | 1 |
| hud-1aswu.3 | Quarantine calibrated wall-clock perf assertions out of blocking test-unit lane | 1 |
| hud-1aswu.4 | Restore D18 real-decode lane: runner + scheduled-lane alerting + bar annotation | 1 |
| hud-1aswu.5 | cargo-deny CI job (advisories + GPL-3.0 licenses) | 2 |
| hud-1aswu.6 → .7 | gen-1 reconciliation → epic report | 1 |

### Epic `hud-w0jfp` — shape & doc refresh tranche (P2; every child edits normative artifacts → change-tier reconciliation required per child)
| Bead | Title | P |
|---|---|---|
| hud-w0jfp.1 | Amend v1.md (touch, macOS lane, introspection, portal surface) — **gates hud-iygbd** | 1 |
| hud-w0jfp.2 | Refresh topology + counts (components.md/README/CLAUDE.md; a11y/media parked status) | 2 |
| hud-w0jfp.3 | Deferral stamping: RFC 0014/0018, mobile crate roots, failure.md E25 pointer fix | 2 |
| hud-w0jfp.4 | Spec hygiene: Purpose-TBD fixes + `Implementation:` convention + 7-family backfill | 2 |
| hud-w0jfp.5 | Regenerate AGENTS.md + docs/ index (normative/episodic split) | 2 |
| hud-w0jfp.6 | Amend engineering-bar review mechanics + production-call-site review standard | 2 |
| hud-w0jfp.7 | OpenSpec ledger hygiene: archive projection change + reconcile portal tasks.md (windows-first ledger stays owned by hud-iygbd) | 2 |
| hud-w0jfp.8 → .9 | gen-1 reconciliation (incl. shape-scan re-run) → epic report | 2 |

### Epic `hud-3qpgv` — code hygiene chore wave (P3, fleet fill-in; no report bead)
hud-3qpgv.1 ResourceBudget unification · .2 try_lock miss telemetry (feeds the deferred double-buffer evaluation) · .3 mechanical polish wave (unwrap→expect, clippy-allow justifications, subtle swap, SAFETY backfill) · .4 projection_authority tracing · .5 dev harness (toolchain/justfile/lints) · .6 merged-branch cleanup (312) · .7 gen-1 reconciliation.

### Closeout beads (children of existing refocus epic `hud-9wljr`) + standalones
| Bead | Title | P | Gated on |
|---|---|---|---|
| hud-9wljr.8 | Release mechanics: retro-tag perf baseline, CHANGELOG, --version SHA, deploy provenance | 2 | — (unblocked) |
| hud-9wljr.9 | Cut first release tag for windows-first closeout | 1 | hud-i0jdz, hud-po7iz, hud-iygbd, hud-9wljr.8 |
| hud-9wljr.10 | Tile background colors → design tokens (closes the last v1 ships-claim violation) | 2 | — |
| hud-olxxd | Execute validation-operations-standalone + sync validation-framework specs — **gates hud-iygbd** | 1 | — |
| hud-se14n | Plan section-banner splits of session_server.rs/renderer.rs (before phase-4 proto fields) | 3 | — |
| hud-hj7ut | Decision: tze_hud_policy fate (wire / extract+freeze / park) — owner decision | 3 | — |

### Cross-wiring into existing beads
`hud-iygbd` (closeout spec-to-code reconcile) now additionally depends on `hud-w0jfp.1` and `hud-olxxd` (per audit §10: v1.md amendments and validation-framework sync must precede an honest closeout). Verified post-creation: `hud-iygbd` open deps = {hud-i0jdz, hud-po7iz, hud-w0jfp.1, hud-olxxd}; graph acyclic; 24 of the new beads immediately ready; recons/report/release-tag correctly blocked.

## Dedup decisions (no duplicate work)
- **Skipped entirely**: portal host chain (8+ live beads), 3-agent soak (hud-nfl7n closed with evidence), closeout reconcile (hud-iygbd exists), soak coverage gap (hud-po7iz), overlay composite (hud-i0jdz), beads backup (hud-qdeh8).
- **Re-pointed**: windows-first perf-deep-dive checkbox staleness belongs to hud-iygbd's own ledger deliverable, not a new bead (verify-pass finding 1; the original draft had a partial duplicate, removed).
- **Distinct-not-duplicate**: hud-an467 (safe-mode try_lock fail-open) vs hud-3qpgv.2 (frame-loop miss telemetry); hud-ora8.1.28 (deferred D18 glass-to-glass procurement) vs hud-1aswu.4 (runner restoration).

## Do not do yet
| Item | Reason | Revisit when |
|---|---|---|
| TLS/mTLS full transport hardening | Loopback fix (hud-1aswu.1) suffices for v1's local/tailnet threat model | Remote agents / cloud-relay (session.proto fields 80-99) get scheduled |
| Scene double-buffer/snapshot rework | Needs miss-rate data first | hud-3qpgv.2 close reason reports counters |
| Mobile/media crates, RFC 0014/0018 implementation, v2 change | Refocus (v1.md:9-17) | Owner reopens v2 |
| macOS CI lane build-out | Doctrine criterion being amended instead (hud-w0jfp.1) | Owner decides macOS matters for v1 |
| Second reference host / benchmark trend gate | Owner/hardware decision; M-effort | After release tag, if lane outages recur |
| Zero-production-caller *lint* automation | Review-standard form lands via hud-w0jfp.6 first | If the landed-but-not-live pattern recurs after hud-w0jfp.6 |

---

## Conclusion

**Real direction**: a single-Windows, evidence-gated presence runtime whose near-term product is text-stream portals and whose near-term obligation is closing out windows-first with its first-ever release tag.

**Work on next**: (1) `hud-1aswu.1`/`.2`/`.3` — small, risk-dominant enforcement fixes (two already claimed by the fleet); (2) `hud-w0jfp.1` + `hud-olxxd` — the two new gates on closeout; (3) keep the existing portal host chain (`hud-2iup7` → `hud-be6ee`) as the active feature critical path.

**Stop pretending**: that v1.md's touch/macOS/introspection claims are shipping as written (amend them — hud-w0jfp.1); that the D18 media bar is enforced while its lane is dead (hud-1aswu.4); and that the project has a release process before its first tag exists (hud-9wljr.8/.9).

Execution ownership now passes to the beads coordinator (`/beads-orchestration`). This document is a planning record; beads are the source of truth.
