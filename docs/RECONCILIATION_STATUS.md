# Reconciliation Status

This document is the entry point for understanding the current spec-to-code coverage baseline.

## Current Baseline: Gen-3 (2026-03-26)

**Document:** [reconciliation-gen3.md](reconciliation-gen3.md)
**Issue:** hud-nsyt.5
**Date:** 2026-03-26

### Summary

Gen-3 is the authoritative reconciliation snapshot as of 2026-03-26. It covers:

- All P1 divergence closures from the hud-nsyt epic (hud-nsyt.1, .2, .3)
- 243 commits and 60+ feature PRs merged since Gen-2 (2026-03-22)

**Coverage at Gen-3 close:**

| Status    | Count | Percentage |
|-----------|-------|------------|
| FULL      | 52    | 81%        |
| PARTIAL   | 5     | 8%         |
| RFC-ONLY  | 6     | 9%         |
| ABSENT    | 0     | 0%         |

Key closures since Gen-2:
- All three RFC-ONLY window modes (W1, W2, W3) are now FULL
- Entire failure-handling section (F1–F4) is now FULL
- Configuration governance section introduced with two FULL rows
- V4 ABSENT item closed

`cargo check --workspace` passes with zero errors.

---

## Historical Documents

These snapshots are preserved for reference but have been superseded by Gen-3:

| Document | Date | Coverage |
|----------|------|---------|
| [reconciliation-gen1.md](reconciliation-gen1.md) | 2026-03-22 | Gen-1 baseline |
| [reconciliation-gen2.md](reconciliation-gen2.md) | 2026-03-22 | 32 FULL (57%), 13 PARTIAL (23%), 9 RFC-ONLY (16%), 1 ABSENT (2%) |
| [reconciliation-nsyt-gen1.md](reconciliation-nsyt-gen1.md) | 2026-03-26 | hud-nsyt epic vs. sibling bead deliverables audit |

Each historical document has been labeled with a HISTORICAL header at the top.

---

## How to Update

When a new reconciliation is completed, update this file to:
1. Change "Current Baseline" to point to the new document
2. Move the previous current baseline to the Historical Documents table
