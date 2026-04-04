# Reconciliation Status

This document is the entry point for understanding the current spec-to-code coverage baseline.

## Current Baseline: Gen-5 (2026-04-04)

**Document:** [reconciliation-gen5.md](reconciliation-gen5.md)
**Issue:** hud-a1va
**Date:** 2026-04-04

### Summary

Gen-5 is a **post-MVP feature expansion snapshot** covering 174 commits since gen-4.
It documents the widget system, component shape language (RFC 0012), 10 exemplar components,
runtime app binary, MCP stress testing, input capture, and resource ref-count tracking.

**Key changes since Gen-4:**
- Widget system implemented (5 delta specs) with 2 P1 gaps remaining (ClearWidget, TTL expiry)
- Component shape language (RFC 0012) fully implemented
- 10 exemplar components with integration tests and user-test scenarios
- Session-protocol openspec refreshed to match actual 4-file proto layout
- RFC count in law-and-lore corrected to 12

**Open P1 gaps:**
- ClearWidgetMutation not wired (hud-jliz)
- Widget TTL expiry not enforced (hud-2c5g)
- Config contract app/spec alignment decision needed (hud-gxny)

**Coverage (estimated):**

| Status    | Percentage | Notes |
|-----------|------------|-------|
| FULL      | ~90%       | Gen-4 baseline + post-MVP features |
| PARTIAL   | ~5%        | Widget system P1 gaps |
| RFC-ONLY  | ~5%        | I2, Pl1-Pl3 (unchanged from gen-4) |
| ABSENT    | 0%         | — |

`cargo check --workspace` passes with zero errors.

---

## Previous Baseline: Gen-4 (2026-03-27)

**Document:** [reconciliation-gen4.md](reconciliation-gen4.md)
**Issue:** hud-leji
**Date:** 2026-03-27

Gen-4 was the **v1-MVP final closure** snapshot covering all 7 deliverables of the
hud-kibj ship-readiness epic. 54 FULL (93%), 0 PARTIAL, 4 RFC-ONLY (7%), 0 ABSENT.

---

## Historical Documents

These snapshots are preserved for reference:

| Document | Date | Coverage |
|----------|------|---------|
| [reconciliation-gen4.md](reconciliation-gen4.md) | 2026-03-27 | 54 FULL (93%), 0 PARTIAL (0%), 4 RFC-ONLY (7%), 0 ABSENT (0%) — v1-MVP closure |
| [reconciliation-gen3.md](reconciliation-gen3.md) | 2026-03-26 | 51 FULL (88%), 3 PARTIAL (5%), 4 RFC-ONLY (7%), 0 ABSENT (0%) |
| [reconciliation-gen1.md](reconciliation-gen1.md) | 2026-03-22 | Gen-1 baseline |
| [reconciliation-gen2.md](reconciliation-gen2.md) | 2026-03-22 | 32 FULL (57%), 13 PARTIAL (23%), 9 RFC-ONLY (16%), 1 ABSENT (2%) |
| [reconciliation-nsyt-gen1.md](reconciliation-nsyt-gen1.md) | 2026-03-26 | hud-nsyt epic vs. sibling bead deliverables audit |

Each historical document has been labeled with a HISTORICAL header at the top.

---

## How to Update

When a new reconciliation is completed, update this file to:
1. Change "Current Baseline" to point to the new document
2. Move the previous current baseline to the Historical Documents table
