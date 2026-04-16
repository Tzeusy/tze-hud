# RFC 0006: Configuration/Display Profiles — Round 4 Review

**Review Round:** 4 of 4
**Focus:** Final hardening and quantitative verification
**Issue:** rig-5vq.34
**Date:** 2026-03-22
**Reviewer:** Beads worker agent
**Doctrine files reviewed:** architecture.md, v1.md, failure.md, validation.md
**Related RFCs reviewed:** RFC 0002 (Runtime Kernel), RFC 0005 (Session Protocol), RFC 0008 (Lease Governance)

---

## Doctrinal Alignment: 4/5

No doctrinal regressions from prior rounds. The RFC faithfully implements all doctrine commitments established in rounds 1–3. Key doctrinal alignments confirmed:

- "Screen is sovereign": config as input to the runtime, never the other way; runtime enforces profiles against all agent requests (§3.1: "It shapes what the runtime permits — not what agents can request, but what the compositor will grant and enforce.")
- "Configuration is declarative, file-based, human-readable, validated at load time" (architecture.md §Configuration model) — fully satisfied by TOML format, schemars export, and structured errors with `hint` field
- "Arrival time ≠ presentation time" — config carries no timing semantics to violate this
- Headless profile for CI (v1.md §Headless mode is fully functional) — implemented as third built-in
- Degradation ladder ordered lightest-first (failure.md §Degradation axes) — enforced by `CONFIG_DEGRADATION_THRESHOLD_ORDER`
- "Human can always override" — `show_override_controls` chrome field with default `true`

Score unchanged from round 3. No architectural deficiencies remain.

---

## Technical Robustness: 4/5 (after fixes)

Three MUST-FIX items identified. All are cases where the RFC text makes a requirement but no error code or validation rule enforces it — precisely the class of ambiguity a final-round review must catch. An implementer reading the RFC could easily miss these, leading to inconsistent runtime behavior.

---

## Cross-RFC Consistency: 5/5

No new cross-RFC inconsistencies. All issues from round 3 are resolved. The cross-references section now correctly cites RFC 0005, RFC 0008, and RFC 0009. Capability vocabulary is aligned. The `reconnect_grace_secs` equivalence is documented. No drift found against RFC 0002, RFC 0004, or the session protocol.

Score raised from 4/5 to 5/5: all previously-identified consistency gaps are now closed, no new gaps found.

---

## Actionable Findings

### [MUST-FIX 1] `CONFIG_NO_TABS` error code missing — "at least one `[[tabs]]`" has no enforced error

**Location:** §2.1 (top-level structure), §2.4 validation rules, §10 `ConfigErrorCode` enum, Summary of Validation Error Codes

**Problem:** §2.1 states explicitly: "Each section is optional except `[runtime]` and at least one `[[tabs]]` entry." However:

1. §2.4 lists validation rules for tab content but has no rule for the case where the `tabs` array is empty.
2. The `ConfigErrorCode` enum has no `ConfigNoTabs` variant.
3. The Summary of Validation Error Codes table has no `CONFIG_NO_TABS` row.

An implementer following this RFC will see the "at least one `[[tabs]]`" requirement in §2.1 and must invent an error code to enforce it. This is the exact type of ambiguity the final round is supposed to eliminate. The runtime should catch this at config load time with a structured error, not at scene construction time with a different error.

**Fix applied:**
- Added `CONFIG_NO_TABS` to §2.4 validation rules with description: "Empty `tabs` array → `CONFIG_NO_TABS`. Hint: add at least one `[[tabs]]` entry."
- Added `ConfigNoTabs` variant to the `ConfigErrorCode` Rust enum (§10)
- Added `CONFIG_NO_TABS` row to the Summary of Validation Error Codes table

**Rationale:** The RFC guarantees structured errors for all configuration failures. This guarantee is hollow if a foundational structural requirement has no error code. An LLM generating a config file that accidentally omits `[[tabs]]` must receive a `CONFIG_NO_TABS` error with a clear hint, not a runtime panic.

---

### [MUST-FIX 2] `reserved_*_fraction` layout fields have no range validation or error code

**Location:** §5.3 Layout Constraints table

**Problem:** The four `reserved_*_fraction` fields (`reserved_bottom_fraction`, `reserved_top_fraction`, `reserved_left_fraction`, `reserved_right_fraction`) are documented as `float (0.0–1.0)` in the table, but:

1. No validation rule in §5.3 enforces the [0.0, 1.0] range.
2. No validation rule checks that the sum of horizontal reserved fractions (`reserved_left_fraction + reserved_right_fraction`) and vertical reserved fractions (`reserved_top_fraction + reserved_bottom_fraction`) each remain below 1.0 — a config with `reserved_top_fraction = 0.6` and `reserved_bottom_fraction = 0.6` would claim 120% of display height for reserved areas, leaving no space for agent tiles.
3. No error code exists for either failure case.

An implementer reading §5.3 has no error code to emit when a value is `1.5` or when reserved fractions sum to `1.2`. They must silently clamp, panic, or invent their own error. The RFC's contract requires explicit error codes for all validation failures.

**Fix applied:**
- Added validation rules to §5.3: each `reserved_*_fraction` must be in [0.0, 1.0]; the sum `reserved_top_fraction + reserved_bottom_fraction` must be < 1.0 (strictly — otherwise zero vertical space remains for tiles); the sum `reserved_left_fraction + reserved_right_fraction` must be < 1.0. Violation → `CONFIG_INVALID_RESERVED_FRACTION`.
- Added `ConfigInvalidReservedFraction` variant to the `ConfigErrorCode` enum (§10)
- Added `CONFIG_INVALID_RESERVED_FRACTION` row to the Summary of Validation Error Codes table

**Rationale:** The 0.0–1.0 annotation in the table is a documentation convention; without a validation rule backed by an error code, it is aspirational rather than contractual. An LLM implementer building the validator needs a concrete error to emit.

---

### [MUST-FIX 3] Agent budget overrides have no cross-profile validation rule

**Location:** §6.2, §6.4, agent budget sub-tables (`[agents.registered.<name>.budgets]`, `[agents.dynamic_policy.default_budgets]`)

**Problem:** The `doorbell_agent` example in §6.2 sets `max_texture_mb = 512` — a value exceeding the built-in `mobile` profile's total texture budget of `256 MiB`. The RFC provides no rule governing whether per-agent budget overrides are validated against the active profile's total budget.

This creates two ambiguous behaviors for an implementer:
1. Should the validator reject an agent budget `max_texture_mb` that exceeds the profile's `max_texture_mb` total?
2. Should the validator reject an agent budget `max_tiles` that exceeds the profile's `max_tiles` total?

Without explicit rules, an implementer may silently accept overrides that exceed the profile ceiling, leading to runtime behavior where a single agent claims more than the entire profile's budget. The RFC must state the intended semantics.

Note: the `doorbell_agent` example comment says "Camera feed budget (post-v1)" — this is a legitimate post-v1 intent, but the example value should not exceed the profile ceiling in an unaddressed way.

**Fix applied:**
- Added validation rules to §6.2: per-agent `max_tiles` may not exceed the active profile's `max_tiles`; per-agent `max_texture_mb` may not exceed the active profile's `max_texture_mb`. Violation → `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE` with a hint identifying the field and the profile ceiling.
- Updated the `doorbell_agent` example to use `max_texture_mb = 128` (a value below the `full-display` profile's ceiling of 2048 MiB) with a comment explaining the post-v1 camera budget intent.
- Added `ConfigAgentBudgetExceedsProfile` variant to the `ConfigErrorCode` enum (§10)
- Added `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE` row to the Summary of Validation Error Codes table

**Rationale:** Per-agent budgets are sub-allocations within the profile's total budget. An agent that claims more than the entire profile budget is misconfigured, not just under-provisioned. The runtime will enforce this at runtime anyway (RFC 0002 §4.3 budget enforcement); catching it at config load time with a structured error is strictly better.

---

### [SHOULD-FIX 4] `tab_switch_on_event` unknown names accepted silently — WARN behavior not specified

**Location:** §5.4 Tab Switching Policy; Review History Round 2 [CONSIDER] entry

**Problem:** Round 2 noted (as a [CONSIDER]): "`tab_switch_on_event` unknown names are silently accepted at load time; a `WARN` log entry is recommended for names not in the built-in event registry."

That finding was marked CONSIDER and carried forward without implementing any RFC text. For round 4 (final hardening), this gap is material: an implementer reading §5.4 sees no specification for what happens when `tab_switch_on_event` names an event that doesn't exist in the built-in event registry. Silently ignoring it (tab never auto-switches) is likely but not documented.

**Fix applied:**
- Added a validation note to §5.4: "If `tab_switch_on_event` names an event not found in the built-in event registry (see RFC 0004 §scene-level events), the runtime accepts the configuration and emits a `WARN` log entry at startup: `tab_switch_on_event 'X' in tab 'Y': event name not recognized; tab will never auto-switch until this event is registered`. This is not a hard error because custom events may be registered at runtime in post-v1; treating it as an error would block forward compatibility."

**Rationale:** Documenting "accept with WARN" is a behavioral contract, not decoration. Without it, two implementations will disagree: one will reject the config (breaking forward compatibility), another will silently drop the tab switch (making debugging impossible).

---

### [SHOULD-FIX 5] `DisplayProfileConfig` Rust struct comment omits `headless` from valid built-in list

**Location:** §10 Rust Types, `DisplayProfileConfig.extends` field doc comment

**Problem:** The doc comment reads: "Must name a built-in profile (`full-display` or `mobile`)." This is incorrect — `headless` is also a built-in profile (§3.4). The omission is a documentation error that could mislead an implementer into accepting `extends = "headless"` (which is correctly rejected at validation time by §3.6), but for the wrong reason: they might think `headless` is not a built-in when in fact it is — just one that cannot be used as a base for extension.

**Fix applied:**
- Updated the doc comment to: "Must name a built-in profile (`full-display` or `mobile`). The `headless` built-in profile is recognized but may not be used as an `extends` base — see `CONFIG_HEADLESS_NOT_EXTENDABLE`."

**Rationale:** The comment is part of the formal specification (it generates rustdoc and is exported via schemars). Implementers reading it to understand the validate-extends logic will be confused by the incomplete built-in list.

---

## Scores Summary

| Dimension | Round 1 | Round 2 | Round 3 | Round 4 |
|-----------|---------|---------|---------|---------|
| Doctrinal Alignment | 4/5 | 4/5 | 4/5 | 4/5 |
| Technical Robustness | 3/5 → 4/5 | 4/5 | 4/5 | 4/5 |
| Cross-RFC Consistency | 4/5 | 4/5 | 4/5 | 5/5 |

All scores ≥ 4. All dimensions ≥ 3. Round 4 complete.

---

## Changes Made

- `about/legends-and-lore/rfcs/0006-configuration.md`: Added `CONFIG_NO_TABS` validation rule to §2.4; added `CONFIG_NO_TABS` error code to §10 enum and summary table
- `about/legends-and-lore/rfcs/0006-configuration.md`: Added `CONFIG_INVALID_RESERVED_FRACTION` validation rules to §5.3; added to §10 enum and summary table
- `about/legends-and-lore/rfcs/0006-configuration.md`: Added `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE` validation rules to §6.2; corrected `doorbell_agent` `max_texture_mb` example; added to §10 enum and summary table
- `about/legends-and-lore/rfcs/0006-configuration.md`: Added `tab_switch_on_event` WARN behavior to §5.4
- `about/legends-and-lore/rfcs/0006-configuration.md`: Fixed `DisplayProfileConfig.extends` doc comment in §10 to include `headless` with correct usage note
- `about/legends-and-lore/rfcs/0006-configuration.md`: Added Round 4 review history entry
- `about/legends-and-lore/reviews/0006-configuration-round4.md`: This review document
