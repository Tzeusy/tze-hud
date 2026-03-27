# Reconciliation Status

This document is the entry point for understanding the current spec-to-code coverage baseline.

## Current Baseline: Gen-4 (2026-03-27)

**Document:** [reconciliation-gen4.md](reconciliation-gen4.md)
**Issue:** hud-leji
**Date:** 2026-03-27

### Summary

Gen-4 is the authoritative reconciliation snapshot as of 2026-03-27. It is the **v1-MVP
final closure** snapshot, covering all 7 deliverables of the hud-kibj ship-readiness epic.

**Coverage at Gen-4 close:**

| Status    | Count | Percentage |
|-----------|-------|------------|
| FULL      | 54    | 93%        |
| PARTIAL   | 0     | 0%         |
| RFC-ONLY  | 4     | 7%         |
| ABSENT    | 0     | 0%         |

Key closures since Gen-3:
- All 3 PARTIAL items resolved (Sec2 capability revocation, V1 Layer 1 colour assertions, T1 per-frame correctness fields)
- Production config committed and exercised by CI
- Closure-grade CI workflow (9 quality gates on push/PR to main)
- Canonical vocabulary enforced with CI lint script
- Governance authority boundaries documented
- MCP zone conformance test coverage complete
- Historical reconciliation docs labeled

`cargo check --workspace` passes with zero errors.

RFC-ONLY items (I2, Pl1, Pl2, Pl3) are explicitly deferred to v1.1 with justification
in reconciliation-gen4.md §4. None block the v1 thesis.

---

## Historical Documents

These snapshots are preserved for reference but have been superseded by Gen-4:

| Document | Date | Coverage |
|----------|------|---------|
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
