# RFC 0001 Scene Contract — Round 3 Review: Cross-RFC Consistency and Integration

**Reviewer:** rig-5vq.13 agent worker
**Date:** 2026-03-22
**Focus:** Cross-RFC coherence — shared types, capability names, data model alignment, contradictory requirements
**Doctrine files read:** presence.md, architecture.md, security.md
**RFCs consulted:** 0001, 0002, 0003, 0005, 0006, 0007, 0008, 0009

---

## Doctrinal Alignment: 4/5

No new doctrinal regressions found. Round 1 and Round 2 fixes held. The RFC correctly models all doctrine-mandated structures. Score unchanged from round 2.

One doctrinal gap was found and fixed (see Finding R3-1): presence.md §"Zone anatomy" explicitly lists "privacy classification" as a first-class field on zone publications. RFC 0001's `PublishToZoneMutation` was missing this field. This is a doctrinal commitment silently dropped — publications have no mechanism to declare their content classification.

---

## Technical Robustness: 4/5

No new technical regressions found. Score unchanged from round 2.

---

## Cross-RFC Consistency: 4/5

This round focused on cross-RFC coherence. Since round 2, RFC 0008 (Lease Governance) and RFC 0009 (Policy & Arbitration) have landed on main and introduced new integration requirements. The following findings were identified:

### Finding R3-1 [MUST-FIX → Fixed]

**Location:** RFC 0001 §2.5 `PublishToZone` Rust enum variant, §7.1 `PublishToZoneMutation` proto message
**Problem:** `PublishToZoneMutation` has no `content_classification` field. RFC 0009 §2.3 (Privacy/Viewer Gate, Step 3) specifies that the privacy gate "compares the content's declared `VisibilityClassification` against the current `ViewerClass`." This is a per-publication field that agents must supply. Without it, every zone publication is forced to inherit the zone's default classification — agents cannot publish content with a more restrictive classification than the zone default, which defeats the per-publication privacy model described in presence.md §"Zone anatomy":
> "Publication. One publish event into a zone instance: content payload, TTL, key (for merge-by-key zones), priority, **privacy classification**, and optional stream/session identity."

**Fix applied:** Added `ContentClassification` enum (Public, Household, Private, Sensitive) to §2.5 and §7.1. Added `content_classification: Option<ContentClassification>` to the Rust `PublishToZone` enum variant and `content_classification: ContentClassification` (field 6, UNSPECIFIED = inherit zone default) to `PublishToZoneMutation` proto. Added rationale comment referencing presence.md and RFC 0009 §2.3.

**RFC 0009 cross-reference:** RFC 0009 §2.3 "Zone ceiling rule" states `effective_classification = max(agent_declared_classification, zone_default_classification)`. The new field makes this rule implementable.

---

### Finding R3-2 [MUST-FIX → Fixed]

**Location:** RFC 0001 §3.3 lease check, §9.4 ID Namespace Isolation diagram
**Problem:** RFC 0001 used inconsistent capability identifiers:
- `WRITE_SCENE` in §3.3 lease check
- `CREATE_TILE`, `WRITE_SCENE`, `zone:publish:subtitle` (colon-separated) in §9.4 diagram

RFC 0006 §6.3 defines the authoritative capability naming scheme as `snake_case` with colon-separated sub-scopes for zone grants:
- `create_tiles` (not `CREATE_TILE`)
- `modify_own_tiles` (not `WRITE_SCENE`)
- `zone_publish:<zone_name>` (not `zone:publish:<zone_name>`)

RFC 0008 §1.2 capability table also uses `CREATE_TILE` and `WRITE_SCENE` in examples, but RFC 0008 §11 identifies this as a clarification target, not a naming authority.

RFC 0009 introduced a third variant using `kebab-case` (`zone-publish:`, `create-tiles`, `subscribe-scene-events`) which has also been corrected in RFC 0009 as part of this review round.

**Why this matters:** Config validation (`[agents.capabilities]` in RFC 0006), capability grant audit logs (RFC 0009 §7.3), and capability check matrices (RFC 0009 §2.2) all use the same capability strings. If RFCs use different naming conventions for the same capability, implementations will produce bugs where valid capabilities are rejected at the wrong layer.

**Fix applied:**
- RFC 0001 §3.3: replaced `WRITE_SCENE` with `modify_own_tiles`; added normative note establishing RFC 0006 §6.3 as the canonical naming authority
- RFC 0001 §9.4 diagram: replaced `CREATE_TILE`, `WRITE_SCENE`, `zone:publish:subtitle` with `create_tiles`, `modify_own_tiles`, `zone_publish:subtitle`
- RFC 0009 §2.2 capability check matrix: replaced all kebab-case names with canonical `snake_case` names; added convention note
- RFC 0009 §7.3 audit log examples: replaced `zone-publish:notification` with `zone_publish:notification`

---

### Finding R3-3 [SHOULD-FIX → Fixed]

**Location:** RFC 0001 §2.3 `ResourceBudget` Rust struct, §7.1 `message ResourceBudget`
**Problem:** RFC 0001 and RFC 0008 both define a `ResourceBudget` struct with the same name but different fields serving different purposes:
- **RFC 0001 `ResourceBudget`** (3 fields): `texture_bytes`, `update_rate_hz`, `max_nodes` — per-tile limits embedded in `Tile`, enforced during mutation validation
- **RFC 0008 `ResourceBudget`** (7 fields): `texture_bytes_per_tile`, `max_nodes_per_tile`, `update_rate_hz`, `max_tiles`, `texture_bytes_total`, `max_active_leases`, `max_concurrent_streams` — lease-level budget in LeaseRequest/LeaseResponse, governing aggregate session limits

Note also different field names: RFC 0001 uses `texture_bytes` / `max_nodes` while RFC 0008 uses `texture_bytes_per_tile` / `max_nodes_per_tile` for the same concepts.

These are in different proto packages (`tze_hud.scene.v1` vs `tze.lease.v1`) but the identical struct name and overlapping fields will cause implementors to conflate them, potentially leading to struct misuse (embedding the 7-field lease budget in `Tile` structs or using the 3-field per-tile budget in LeaseRequest).

**Fix applied:** Added explicit doc-comment to both the Rust struct and proto `message ResourceBudget` in RFC 0001 §2.3 and §7.1 explaining the two-budget design, their distinct packages, their relationship, and the prohibition on conflation.

---

### Finding R3-4 [MUST-FIX → Fixed, in RFC 0007]

**Location:** RFC 0007 §4.2 "Dismiss All / Safe Mode" step 1
**Problem:** RFC 0007 §4.2 still says "All active leases are **revoked** simultaneously." RFC 0008 §3.4 (DR-LG7) is the authoritative resolution of the revoke/suspend contradiction between RFC 0007 §4.2 and §5.2:

> "Safe mode **suspends** leases; it does not revoke them. RFC 0007 §4.2's phrase 'All active leases are revoked' is incorrect."

RFC 0008 §11 ("Cross-RFC Errata") explicitly mandates that RFC 0007 §4.2 be updated. This update was not applied to RFC 0007 in round 2. As a result, the errata mandate existed in RFC 0008 but the incorrect text remained in RFC 0007.

**Fix applied (in RFC 0007):** Updated step 1 to "All active leases are **suspended** simultaneously" with full RFC 0008 §3.3/§3.4 cross-reference. Added authoritative behavior note. RFC 0008 §11 errata mandate is now satisfied.

---

### Finding R3-5 [MUST-FIX → Fixed, in RFC 0006]

**Location:** RFC 0006 §2.8 `[chrome]` section
**Problem:** RFC 0006 §2.8 still has `redaction_style = "pattern"` in the `[chrome]` section. RFC 0009 §3.2 explicitly identifies this as a duplication error and mandates its removal. RFC 0009 §9 Cross-RFC Interaction Table also states "The `[chrome].redaction_style` field is removed." The field was not removed from RFC 0006.

**Fix applied (in RFC 0006):** Removed `redaction_style = "pattern"` from the `[chrome]` section TOML block. Replaced with a normative comment explaining the removal and pointing to `[privacy].redaction_style` as the authoritative field. Added Round 3 review entry recording the change.

---

## Actionable Findings Summary

| # | Severity | Location (RFC/section) | Finding | Status |
|---|----------|------------------------|---------|--------|
| R3-1 | MUST-FIX | RFC 0001 §2.5, §7.1 | `PublishToZoneMutation` missing `content_classification` field; presence.md and RFC 0009 §2.3 require per-publication privacy classification | Fixed |
| R3-2 | MUST-FIX | RFC 0001 §3.3, §9.4; RFC 0009 §2.2, §7.3 | Capability names inconsistent across RFCs; RFC 0009 used kebab-case; RFC 0001 used SCREAMING_SNAKE_CASE; RFC 0006 §6.3 defines authoritative snake_case | Fixed |
| R3-3 | SHOULD-FIX | RFC 0001 §2.3, §7.1 | Two `ResourceBudget` structs with identical names across RFC 0001 and RFC 0008 create implementor confusion; no documentation of split | Fixed |
| R3-4 | MUST-FIX | RFC 0007 §4.2 | "All active leases are revoked" contradicts RFC 0008 §3.4 (DR-LG7); errata mandate in RFC 0008 §11 was not applied | Fixed |
| R3-5 | MUST-FIX | RFC 0006 §2.8 | `redaction_style` still present in `[chrome]` section; RFC 0009 §3.2 mandates removal | Fixed |

---

## Overall Scores

| Dimension | Score | Rationale |
|-----------|-------|-----------|
| Doctrinal Alignment | **4/5** | No regressions; minor gap fixed (content_classification). Prior rounds' fixes held. |
| Technical Robustness | **4/5** | No regressions. Prior rounds' fixes held. |
| Cross-RFC Consistency | **4/5** | Five cross-RFC inconsistencies found and fixed. No remaining known blockers. |

All dimensions ≥ 3. Round 3 is complete.

---

*Review round 3 complete. All MUST-FIX and SHOULD-FIX items addressed. No dimension scored below 3. Ready for Round 4 (Final Hardening).*
