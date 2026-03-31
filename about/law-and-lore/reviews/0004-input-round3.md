# RFC 0004 — Input Model: Round 3 Review

**Round:** 3 of 4
**Focus:** Cross-RFC consistency and integration
**Reviewer:** rig-5vq.25
**Date:** 2026-03-22
**Issue:** rig-5vq.25

---

## Context

Round 3 reviews cross-RFC coherence following the merge of:

- **PR #43** — SceneId unification across all 7 RFCs (RFC 0004 §9.1 now uses `SceneId tile_id` / `SceneId node_id`)
- **PR #44** — RFC 0004 dependency header corrected (RFC 0003 = "Timing Model"; added RFC 0008)
- **PR #42** — Capability vocabulary unified between RFC 0005 and RFC 0006
- **PR #37** — Clock-domain naming convention established in RFC 0005 (`_wall_us`, `_mono_us` suffixes)

The review reads the RFC as it exists now on branch `agent/rig-5vq.25` (includes all prior round fixes).

---

## Doctrinal Alignment Score: 4/5

The RFC faithfully implements the core doctrinal commitments from `presence.md` and `architecture.md`:

- Local feedback first (§6): Local state update in the same frame as input event; rollback on agent rejection. Traceable to `presence.md §Interaction` ("clicking through a cloud roundtrip" invariant).
- Runtime arbitrates (§3): Agents receive named gesture events; the runtime runs all recognizer state machines. Traceable to `presence.md §Gesture arbitration`.
- Screen is sovereign (§7.2): Chrome layer always wins hit-testing. Traceable to `architecture.md §Screen is sovereign`.
- LLMs never sit in the frame loop (§6.1, §6.2): Event routing is asynchronous; local state is compositor-owned.
- Focus is per-tile (§1.1): Consistent with `presence.md §Focus`.
- Accessibility is first-class (§5): Platform a11y bridge specified for all three OS targets.

**Minor gap (non-blocking):** The scroll-deferred situation in §11.2 is flagged as a pre-implementation blocker with an action item but no deadline. `failure.md §"What the user always sees"` lists scroll responsiveness as an invariant; this should remain under pressure.

---

## Technical Robustness Score: 4/5

Round 2 resolved the major technical issues (gesture threshold, IME focus-loss ordering, transactional-event drop contradiction, `SessionMessage` naming). The RFC is architecturally sound.

Remaining technical issue found in this round:

**Duplicate §4.6 section heading** (structural error introduced when §4.5 split was added in Round 2): two sections are labeled `### 4.6`. One is "IME Candidate List Rendering" and the other is "Input Method Support". The second should be `### 4.7`.

---

## Cross-RFC Consistency Score: 3/5

Three inconsistencies found after the round-2 and post-round-2 PRs. Two are MUST-FIX; one is SHOULD-FIX.

---

## Actionable Findings

### [MUST-FIX] §12 RFC Dependency Map is stale and incomplete

**Location:** §12 RFC Dependency Map

**Problem:** Two errors:
1. The entry for RFC 0003 reads "RFC 0003 (Lease Model)" — the wrong name. RFC 0003 is the Timing Model. This was corrected in the `**Depends on:**` header by PR #44 but the §12 body was not updated.
2. RFC 0005 (Session Protocol) is actively referenced in §8.3 and §8.3.1 but is absent from the dependency map. RFC 0008 (Lease & Resource Governance) was added to the `**Depends on:**` header but also absent from the §12 body.

**Fix:** Update §12 to rename RFC 0003 entry and add RFC 0005 and RFC 0008 entries.

**Rationale:** The dependency map is the implementor's reference for cross-RFC wiring. A wrong RFC name and two missing entries are functional misinformation.

---

### [MUST-FIX] §8.3 Note incorrectly describes RFC 0005 field 34 type

**Location:** §8.3, the block-quoted note

**Problem:** The note states:
> RFC 0005 §2.2 currently defines `input_event` as a single `InputEnvelope`.

This is incorrect. RFC 0005 §2.2 field 34 has type `InputEvent` (imported from `scene_service.proto`), not `InputEnvelope`. `InputEnvelope` is defined in `input.proto` (this RFC). The note's intent is valid — RFC 0005 needs to change field 34's type from `InputEvent` to `EventBatch` to support batching — but the description of the current state is wrong and will mislead implementors.

Additionally, §7.1 of RFC 0005 uses the term `InputMessage` (e.g., "the runtime inspects the `InputMessage.event` oneof variant") which matches neither `InputEvent` nor `InputEnvelope`/`EventBatch`. This three-way name confusion (`InputEvent` in RFC 0005 proto, `InputMessage` in RFC 0005 narrative, `EventBatch`/`InputEnvelope` in RFC 0004 proto) is a cross-RFC consistency failure.

**Fix:** Correct the note to accurately describe the current RFC 0005 state: field 34 has type `InputEvent` (from `scene_service.proto`). The note should specify that `InputEvent` must be replaced with `EventBatch` (defined in `input.proto`) for batching support. This clarifies both the current state and the required change.

**Rationale:** Implementors reading this note will search for an `InputEnvelope` field in RFC 0005 and find nothing. The three-way naming confusion is a real integration risk.

---

### [MUST-FIX] Duplicate §4.6 section number

**Location:** §4 (IME section)

**Problem:** Two sections share the heading `### 4.6`:
- Line 457: `### 4.6 IME Candidate List Rendering`
- Line 463: `### 4.6 Input Method Support`

The second one should be `### 4.7 Input Method Support`. This is a structural error that breaks cross-references and navigation.

**Fix:** Renumber the second `### 4.6` to `### 4.7`.

---

### [SHOULD-FIX] §1.4 narrative uses standalone enums that don't match §9.1 proto nested enums

**Location:** §1.4 (Focus Events) narrative proto snippets

**Problem:** The §1.4 narrative snippet declares top-level standalone enums `FocusSource` and `FocusLostReason`, but §9.1 proto defines these as nested enums inside their respective messages (`FocusGainedEvent.Source` and `FocusLostEvent.Reason`). The standalone enums are also inconsistently named:

- §1.4: `enum FocusSource { CLICK = 0; TAB_KEY = 1; PROGRAMMATIC = 2; }` (standalone, used as `FocusSource source = 3`)
- §9.1: `enum Source { CLICK = 0; TAB_KEY = 1; PROGRAMMATIC = 2; }` inside `FocusGainedEvent` (nested, used as `Source source = 3`)

Same pattern for `FocusLostReason` (standalone in §1.4) vs `FocusLostEvent.Reason` (nested in §9.1).

For capture events: `CaptureReleaseReason` (standalone in §2.3 narrative) vs `CaptureReleasedEvent.Reason` (nested in §9.1).

**Fix:** Update §1.4 and §2.3 narrative proto snippets to use nested enum syntax, matching §9.1. This makes narrative examples implementation-faithful.

**Rationale:** When narrative examples show different naming than the actual proto, implementors following the narrative (natural reading order) will produce incorrect code.

---

### [SHOULD-FIX] Input event `timestamp_us` fields don't follow clock-domain naming convention

**Location:** All input event proto messages (§7.3, §7.4, §9.1)

**Problem:** RFC 0003 established that all clock domains should be identifiable from field names alone. RFC 0005 (PR #37) implemented this with `_wall_us` / `_mono_us` suffixes. RFC 0004's input event proto messages all use bare `timestamp_us` fields.

RFC 0002 §3.2 defines the internal `InputEvent` struct as having `timestamp_hw` (hardware timestamp from OS event) and `timestamp_arrival` (monotonic). The proto messages in RFC 0004 only carry `timestamp_us` described as "hardware timestamp" — but the clock domain is unspecified in the field name.

The hardware timestamp from winit is effectively in the OS monotonic domain (or a device-specific monotonic domain). It should be named `timestamp_hw_us` to distinguish it from wall-clock and compositor-monotonic timestamps.

**Fix:** Rename `timestamp_us` to `timestamp_hw_us` in all input event proto messages (§7.3 pointer events, §7.4 keyboard events, gesture events, IME events) and update §9.1 accordingly. Add a note clarifying this is the OS hardware timestamp (monotonic domain) per RFC 0003 §1.1. Non-event timestamps (e.g., `EventBatch.batch_ts_us`) should similarly be specified.

**Rationale:** Consistent clock-domain naming was established as a MUST-FIX in prior rounds for RFC 0005. RFC 0004 should follow the same convention.

---

### [CONSIDER] §8.3.1 dependency note should reference RFC 0005 fields by current revision state

**Location:** §8.3.1, the comment block

**Problem:** The comment block specifies "fields 26–29" and "fields 39–40" for the proposed `SessionMessage` extensions. However RFC 0005's current proto already uses fields 39 and 40 for `SubscriptionChangeResult` and `ZonePublishResult` respectively (added in later rounds). The field numbers proposed in RFC 0004 §8.3.1 conflict with already-allocated RFC 0005 fields.

**Fix:** Update the field number suggestions in §8.3.1 to use unallocated field numbers. RFC 0005 §9.2 field registry shows fields 50–99 are reserved for future use — fields 50–53 (agent→runtime) and 54–55 (runtime→agent) would be appropriate.

---

## Summary of Applied Fixes

All MUST-FIX items are applied to `about/law-and-lore/rfcs/0004-input.md`:

1. §12 RFC Dependency Map: corrected "Lease Model" → "Timing Model" for RFC 0003; added RFC 0005 and RFC 0008 entries
2. §8.3 Note: corrected description of RFC 0005 field 34 type from `InputEnvelope` to `InputEvent`
3. §4.6 duplicate: renumbered second §4.6 to §4.7

SHOULD-FIX items also applied:

4. §1.4 and §2.3 narrative enum snippets updated to use nested syntax matching §9.1
5. `timestamp_us` → `timestamp_hw_us` across all input event messages
6. Updated changelog entry in the Review Changelog table

CONSIDER item (§8.3.1 field number conflicts) noted in changelog but not changed — requires RFC 0005 coordination.
