# RFC 0006: Configuration/Display Profiles — Round 3 Review

**Review Round:** 3 of 4
**Focus:** Cross-RFC Consistency and Integration
**Issue:** rig-5vq.33
**Date:** 2026-03-22
**Reviewer:** Beads worker agent
**Doctrine files reviewed:** security.md, privacy.md
**Related RFCs reviewed:** RFC 0005 (Session Protocol), RFC 0008 (Lease Governance), RFC 0009 (Policy Arbitration)

---

## Doctrinal Alignment: 4/5

Round 1 and Round 2 established doctrinal alignment. No regressions found in this round. The following doctrinal commitments remain correctly implemented:

- "Declarative, file-based, human-readable, validated at load time" (architecture.md §Configuration model)
- Two production built-in profiles (`full-display`, `mobile`) plus `headless` for CI (architecture.md §Display profiles)
- Quiet-hours and interruption class gating (privacy.md §Interruption classes)
- Zone geometry adapts per display profile (presence.md §Zone geometry)
- Degradation ladder ordered lightest-first (failure.md §Degradation axes)
- Additive capability grants, revocable, per-session (security.md §Capability scopes)
- "Fail closed" privacy default (`default_classification = "private"`) (privacy.md §Content classification)
- Human override always available: `show_override_controls` chrome field (security.md §Human override)

Score unchanged from round 2.

---

## Technical Robustness: 4/5

No new technical regressions. Prior rounds fixed the structural defects (typed `ConfigErrorCode` enum, conflict validation for `extends`, boolean capability escalation validation, threshold ordering). The RFC is technically solid for its scope.

Score unchanged from round 2.

---

## Cross-RFC Consistency: 4/5 (after fixes)

### Summary

Four inconsistencies found. Three are MUST-FIX (two blocking implementation divergence, one blocking schema correctness). One is SHOULD-FIX (cross-reference documentation gap). All four are fixed in this PR.

---

## Actionable Findings

### [MUST-FIX 1] `redaction_style` in `[chrome]` section must be removed

**Location:** RFC 0006 §2.8 `[chrome]` TOML example block (line ~348 before this PR)

**Problem:** `redaction_style` appeared in two places in RFC 0006:
1. `[chrome]` section (§2.8) — with only three valid values: `"pattern"`, `"agent_name"`, `"icon"`
2. `[privacy]` section (§7.1) — with four correct values: `"pattern"`, `"agent_name"`, `"icon"`, `"blank"`

RFC 0009 §3.2 ("Policy Ownership: Resolved Conflicts") explicitly resolves this conflict:

> **Resolution:** `redaction_style` is a **privacy policy field**. It belongs exclusively in `[privacy]`. The `[chrome]` entry is a duplication error introduced during review and must be removed.
> **Required change to RFC 0006:** Remove `redaction_style` from the `[chrome]` section (§2.8). The authoritative field is in `[privacy]`.

Implementations reading RFC 0006 in isolation would see `[chrome].redaction_style` and `[privacy].redaction_style` and disagree on which governs. The `[chrome]` version also has a stale value list (three values vs. the correct four in `[privacy]`), which would cause validator drift.

**RFC 0009 §9 cross-reference table** states explicitly: "The `[chrome].redaction_style` field is removed."

**Fix applied:** Removed `redaction_style` from the `[chrome]` TOML block in §2.8. Added a clarifying note stating that `ChromeConfig` must not contain this field, pointing to §7.1 and RFC 0009 §3.2.

**Rationale:** Redaction is a rendering expression of a privacy decision (privacy.md §"Redaction behavior"). Chrome configuration governs display structure (tab bar, indicators). These are different concerns with different owners.

---

### [MUST-FIX 2] Cross-references missing RFC 0005, RFC 0008, RFC 0009

**Location:** RFC 0006 Cross-References section (after the Review History block)

**Problem:** RFC 0006's cross-reference list did not include RFC 0005, RFC 0008, or RFC 0009. These RFCs postdate the original RFC 0006 draft but have significant coupling:

- **RFC 0009** directly modifies RFC 0006 (mandates removal of `[chrome].redaction_style`, establishes that `[privacy].quiet_hours` and `[privacy].max_interruptions_*` configure the arbitration stack). An implementer reading RFC 0006 without knowing about RFC 0009 would implement the wrong behavior.
- **RFC 0005** is the wire-format authority for capability identifier names (§7.1 canonical capability table). RFC 0006 §6.3 copies these names; if they diverge, session handshake capability checking will fail.
- **RFC 0008** adds two capabilities (`lease:priority:<N>`) and establishes the reconnect grace period value that RFC 0006 §2.2 `reconnect_grace_secs` configures.

**Fix applied:** Added RFC 0005, RFC 0008, and RFC 0009 to the Cross-References section with precise interaction descriptions for each.

---

### [MUST-FIX 3] Capability identifier table (§6.3) missing `read_telemetry` and `lease:priority:<N>`

**Location:** RFC 0006 §6.3 capability table

**Problem:** Two capabilities defined in other RFCs were absent from the configuration-side capability grant table:

1. **`read_telemetry`** — defined in RFC 0005 §7.1: required for an agent to subscribe to `telemetry_frames` events (runtime performance samples). Without this capability in the config table, operators have no way to grant telemetry access to pre-registered agents without consulting RFC 0005.

2. **`lease:priority:<N>`** — defined in RFC 0008 §2.1: permits an agent to request lease priority N or lower. RFC 0008 §2.1 states: "A runtime administrator configures which agents receive elevated priority capabilities." The configuration system is the exact mechanism for this, but the table did not list the capability.

Additionally, the table lacked a note explaining the vocabulary discrepancy: RFC 0001 diagrams use uppercase/colon-format names (`CREATE_TILE`, `zone:publish:subtitle`) for illustration. Implementers might treat these as the canonical strings and fail session handshake.

**Fix applied:**
- Added `read_telemetry` row with description and RFC 0005 §7.1 citation
- Added `lease:priority:<N>` row with RFC 0008 §2.1 citation and priority range description
- Added column "Authoritative RFC" to the capability table
- Added a note at the top of §6.3 clarifying RFC 0001 uppercase names are illustration-only; RFC 0005 lowercase underscore forms are canonical per PR #42

---

### [SHOULD-FIX 4] `reconnect_grace_secs` lacks cross-reference to RFC 0005 and RFC 0008

**Location:** RFC 0006 §2.2 `[runtime]` section, `reconnect_grace_secs` comment

**Problem:** RFC 0006 §2.2 defines `reconnect_grace_secs = 30` (seconds). RFC 0005 §8 defines `reconnect_grace_period_ms` with default 30,000ms. RFC 0008 §3.3 references this as the "orphan grace period" and cites RFC 0005 §8 as the authoritative location. These are the same semantic parameter with different names and units in different documents. Without an explicit cross-reference, implementers may:
- Configure a different value in RFC 0006 without knowing it conflicts with RFC 0005's internal default
- Misunderstand whether the seconds-vs-milliseconds distinction reflects different code paths

**Fix applied:** Added inline comment to `reconnect_grace_secs` noting the equivalence: "= 30,000ms; matches RFC 0005 §8 `reconnect_grace_period_ms` default; RFC 0008 §3.3 calls this the 'orphan grace period'".

---

## Scores Summary

| Dimension | Round 1 | Round 2 | Round 3 |
|-----------|---------|---------|---------|
| Doctrinal Alignment | 4/5 | 4/5 | 4/5 |
| Technical Robustness | 3/5 → 4/5 | 4/5 | 4/5 |
| Cross-RFC Consistency | 4/5 | 4/5 | 4/5 |

All scores ≥ 3. Round 3 complete.

---

## Changes Made

- `about/legends-and-lore/rfcs/0006-configuration.md`: Added RFC 0005, RFC 0008, RFC 0009 to Cross-References section
- `about/legends-and-lore/rfcs/0006-configuration.md`: Removed `redaction_style` from `[chrome]` section (§2.8); added clarifying note per RFC 0009 §3.2
- `about/legends-and-lore/rfcs/0006-configuration.md`: Added `read_telemetry` and `lease:priority:<N>` to capability identifier table (§6.3); added authoritative RFC column; added vocabulary note
- `about/legends-and-lore/rfcs/0006-configuration.md`: Added cross-reference note to `reconnect_grace_secs` field comment
- `about/legends-and-lore/rfcs/0006-configuration.md`: Added Round 3 review history entry
- `docs/reviews/0006-configuration-round3.md`: This review document
