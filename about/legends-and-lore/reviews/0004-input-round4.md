# RFC 0004 — Input Model: Round 4 Review

**Round:** 4 of 4
**Focus:** Final hardening and quantitative verification
**Reviewer:** rig-5vq.26
**Date:** 2026-03-22
**Issue:** rig-5vq.26

---

## Context

Round 4 is the final review. It focuses on shipping readiness: quantitative completeness, zero-ambiguity for implementors, and verified internal consistency. Prior rounds resolved:

- **Round 1** — doctrinal alignment, design requirements table, headless testability
- **Round 2** — gesture threshold consistency, IME ordering, session naming alignment, transactional-event backpressure
- **Round 3** — SceneId type unification, clock-domain naming, duplicate §4.6, stale §12 dependency map, RFC 0005 field type correction

This review reads the RFC as merged on `main` (after PR #53 from round 3).

---

## Doctrinal Alignment Score: 5/5

The RFC is now fully aligned with doctrine:

- `presence.md §Interaction` — local-first feedback (§6), runtime arbitrates gesture disputes (§3.5), focus is per-tile (§1.1). All faithfully implemented.
- `architecture.md §Screen is sovereign` — chrome layer wins hit-testing (§7.2). Correct.
- `validation.md §latency budgets` — All three split latencies (input_to_local_ack, input_to_scene_commit, input_to_next_present) are in the DR table with the correct quantitative values from validation.md §3. DR-I11 covers headless testability.
- `failure.md §What the user always sees` — local feedback invariant (< 4ms) is directly mapped to DR-I1.
- `presence.md §IME, text input, and accessibility` — all three are addressed with platform-specific implementations.

No doctrinal violations found. Prior round fixes are solid. The scroll deferral (§11.2) correctly identifies the constraint from failure.md and adds an implementation gate. This is appropriate framing.

---

## Technical Robustness Score: 3/5

Three technical issues found, two MUST-FIX and one SHOULD-FIX. The first two block an implementor from producing a correct implementation.

### Issue 1 (MUST-FIX): `GestureBeganEvent` / `GestureCancelledEvent` referenced in §3.6 but not defined in proto

§3.6 states:
> "the runtime dispatches `GestureBeganEvent` to the owning agent"
> "All other recognizers for the same event sequence receive `GestureCancelledEvent` internally"

Neither `GestureBeganEvent` nor `GestureCancelledEvent` exists in §9.1 or anywhere in the proto schema. The `GestureEvent` message uses a `oneof gesture` with phase enums (DragGesture.Phase.BEGAN/ENDED, PinchGesture.Phase.BEGAN/ENDED, LongPressGesture.Phase.BEGAN/ENDED) — these are the actual mechanism for signaling gesture phases, not separate event types.

An implementor reading §3.6 would write code looking for `GestureBeganEvent`/`GestureCancelledEvent` in the event stream and find nothing. The §3.6 narrative must be rewritten to match the actual proto design.

### Issue 2 (MUST-FIX): `ContextMenu` listed as a gesture in §3.2 but dispatched as `ContextMenuEvent`, not as `GestureEvent`

§3.2 Supported Gestures lists `ContextMenu` in the gesture table alongside Tap, DoubleTap, LongPress, Drag, Pinch, and Swipe. An implementor reading §3.2 would expect a `ContextMenuGesture` oneof case in `GestureEvent`.

But §9.1 defines `InputEnvelope.context_menu` as a standalone `ContextMenuEvent` (field 8), not a case inside `GestureEvent`. The `GestureEvent` oneof has no ContextMenu entry (fields 10–15 cover Tap, DoubleTap, LongPress, Drag, Pinch, Swipe — no ContextMenu).

This is actually the correct design (ContextMenu is a terminal point event, not a phased gesture), but the inconsistency between the gesture table and the dispatch model is unresolved and will mislead implementors.

### Issue 3 (SHOULD-FIX): `SwipeGesture` has no recognition threshold specification

§3.4 defines the Tap recognizer state machine quantitatively (`< 150ms`, `< 10px`). The Drag recognizer is implicitly defined by the `> 10px` failure condition in the Tap machine. But the Swipe recognizer has no state machine or threshold definition anywhere.

§3.2 describes Swipe as "1-finger quick flick" but the proto `SwipeGesture` message carries a `velocity` field without specifying the minimum velocity that distinguishes Swipe from Drag. An implementor must guess.

---

## Cross-RFC Consistency Score: 4/5

One remaining cross-RFC inconsistency (SHOULD-FIX):

### Issue 4 (SHOULD-FIX): §3.3 gesture pipeline diagram shows only 4 recognizers but §3.2 defines 7 gesture types

The §3.3 pipeline diagram shows 4 recognizer boxes: Tap, LongPress, Drag, Pinch. But §3.2 defines 7 gestures: Tap, DoubleTap, LongPress, Drag, Pinch, Swipe, ContextMenu. The diagram omits DoubleTap, Swipe, and ContextMenu recognizers.

While ContextMenu may not have a standalone recognizer (it triggers from LongPress on touch, from right-click on pointer), DoubleTap and Swipe clearly require recognizer state machines. The diagram inconsistency will cause implementors to miss these recognizers.

---

## Actionable Findings

### [MUST-FIX] §3.6 uses non-existent event type names

**Location:** §3.6 Gesture Cancellation

**Problem:** The narrative refers to `GestureBeganEvent` and `GestureCancelledEvent` as if they are dispatched message types. Neither exists in the proto schema. Phased gestures (Drag, Pinch, LongPress) use the `Phase` enum within `GestureEvent`; gesture cancellation is signaled by `GestureEvent { drag { phase: CANCELLED } }` etc. Point gestures (Tap, DoubleTap, Swipe) have no "cancellation" — they are either recognized or not.

**Fix:** Replace the narrative in §3.6 to correctly describe the actual dispatch model:
- Winner is announced via `GestureEvent` with the appropriate phased gesture (BEGAN/CHANGED/ENDED) or a terminal point gesture (Tap, DoubleTap, Swipe).
- Losing recognizers result in the agents of tiles that received pointer events getting `PointerCancelEvent` (field 9 in `InputEnvelope`).
- There are no `GestureBeganEvent` or `GestureCancelledEvent` message types.

**Rationale:** An implementor coding to §3.6 as written would search for non-existent message types, wasting time and possibly implementing a parallel incorrect event system.

---

### [MUST-FIX] §3.2 lists `ContextMenu` as a gesture but it is dispatched as `ContextMenuEvent`

**Location:** §3.2 Supported Gestures table

**Problem:** The gesture table lists `ContextMenu` alongside the other gestures, implying it goes through the gesture recognizer pipeline and is dispatched as a `GestureEvent`. It does not. `ContextMenuEvent` is a standalone `InputEnvelope` variant (field 8), handled outside the `GestureEvent` oneof.

**Fix:** Add a note below the §3.2 table clarifying that `ContextMenu` is dispatched as a standalone `ContextMenuEvent` (not as `GestureEvent`), and is therefore not in the gesture recognizer pipeline or priority list in §3.5. Remove `ContextMenu` from the §3.5 conflict resolution priority list, or add a note explaining it does not conflict with other gestures (it triggers only after the other recognizers have failed or committed).

**Rationale:** An implementor building the gesture pipeline from §3.2 would add a ContextMenu recognizer to the fanout and wire it to `GestureEvent`, then spend time debugging why `InputEnvelope.gesture` never carries a ContextMenu event.

---

### [SHOULD-FIX] `SwipeGesture` has no recognition threshold — velocity floor is unspecified

**Location:** §3.4 Recognizer State Machines; §9.1 `SwipeGesture`

**Problem:** The Tap recognizer has a complete state machine with two quantitative thresholds (`< 150ms`, `< 10px`). The Drag recognizer is defined implicitly (> 10px movement). Swipe has no state machine definition in §3.4 and no velocity threshold. The `SwipeGesture.velocity` field carries the measured velocity but nowhere is the minimum velocity defined for a swipe to be recognized.

Without this, the implementor must choose a threshold. If both Swipe and Drag can be triggered by the same motion (fast move vs slow move), the differentiation rule must be specified.

**Fix:** Add a Swipe recognizer state machine to §3.4 specifying:
- Minimum velocity threshold for recognition (e.g., ≥ 400 px/s)
- Maximum duration window (e.g., < 300ms from touchdown to lift)
- Relationship to Drag recognizer (if movement exceeds 10px but velocity is below threshold, Drag wins; if velocity is above threshold on lift, Swipe wins and Drag is cancelled)

**Rationale:** Without this, two implementations could make wildly different threshold choices, producing inconsistent gesture behavior across builds.

---

### [SHOULD-FIX] §3.3 pipeline diagram omits DoubleTap and Swipe recognizers

**Location:** §3.3 Gesture Recognizer Pipeline diagram

**Problem:** The ASCII pipeline diagram shows only 4 recognizer boxes: Tap, LongPress, Drag, Pinch. DoubleTap and Swipe are absent. DoubleTap is a distinct gesture requiring its own recognizer (it must wait for the second tap and check inter-tap timing), and Swipe requires a recognizer state machine tracking velocity.

**Fix:** Add DoubleTap and Swipe boxes to the pipeline diagram. Note in the diagram (or a paragraph below it) that ContextMenu is handled directly from raw events (right-click or LongPress result) rather than through a parallel recognizer, and explain how the LongPress recognizer's recognition result triggers a ContextMenuEvent.

**Rationale:** The pipeline diagram is the implementor's primary architectural guide for the gesture system. A diagram showing 4 of 7 gesture types will cause the missing recognizers to be forgotten during implementation.

---

## Summary of Applied Fixes

All MUST-FIX and SHOULD-FIX items applied to `about/legends-and-lore/rfcs/0004-input.md`:

1. **§3.6** — rewrote to remove `GestureBeganEvent`/`GestureCancelledEvent` references; narrative now correctly describes `GestureEvent` with Phase enum for phased gestures and `PointerCancelEvent` for losers.
2. **§3.2** — added note below gesture table clarifying ContextMenu dispatch as standalone `ContextMenuEvent`; updated §3.5 to remove ContextMenu from gesture priority list and add a note about its dispatch.
3. **§3.4** — added Swipe recognizer state machine with quantitative velocity threshold (≥ 400 px/s), duration window (< 300ms), and Drag/Swipe disambiguation rule.
4. **§3.3** — added DoubleTap and Swipe recognizer boxes to the pipeline diagram; added ContextMenu handling note.
5. **Review Changelog** — updated Round 4 entry in the RFC's changelog table.
