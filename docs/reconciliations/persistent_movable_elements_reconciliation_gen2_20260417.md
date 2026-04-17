# Persistent Movable Elements Reconciliation (gen-2)

Date: 2026-04-17
Issue: `hud-brfc`
Epic: `hud-bs2q` (Persistent movable elements)
Preceding gen: [`persistent_movable_elements_reconciliation_gen1_20260417.md`](persistent_movable_elements_reconciliation_gen1_20260417.md)

---

## Purpose

Verify the two gaps identified in gen-1 are closed and all v1-scope requirements are
satisfied. Gen-1 left R4 partially covered (GAP-1: reset E2E tests as stubs) and the
delta specs un-promoted (GAP-2). Both have since landed.

---

## Delta From Gen-1

| Gap | Bead | PR | Status |
|-----|------|----|--------|
| GAP-1: Tests 3+4 `#[ignore]` stubs; reset path untested | `hud-hym1` (closed via `hud-2k88`) | #457 (`bdad9de`) | **CLOSED** |
| GAP-2: Delta specs not promoted to `openspec/specs/` | `hud-mu38` | #458 (`9a279cf`) | **CLOSED** |

---

## Gap Verification

### GAP-1 — Tests 3 and 4

File: `tests/integration/movable_elements_e2e.rs`

- **Test 3** (`reset_position_clears_user_override_and_restores_agent_bounds`, line 376):
  Present, no `#[ignore]`, no `unimplemented!()`. Exercises `ElementStore::reset_geometry_override`
  and verifies the post-reset `resolve_geometry_override_chain` returns the agent-requested bounds.
- **Test 4** (`zone_reset_falls_back_to_config_override_not_default_policy`, line 458):
  Present, no `#[ignore]`, no `unimplemented!()`. Exercises `reset_geometry_override` on a Zone
  entry, then verifies the chain returns the config-level override rather than the default policy.
- `grep -n "ignore\|unimplemented"` on the file returns empty — confirmed clean.

Both tests exercise `reset_geometry_override` (defined in `crates/tze_hud_scene/src/element_store.rs`),
which is the API surface wired by `hud-zc7f`. The reset path is now actively tested.

### GAP-1 — Short-Tap Gesture Scope

Gen-1 flagged that the spec (`drag-to-reposition/spec.md` § "Reset Gesture") specifies a **short
tap** on the drag handle to reveal the reset affordance, while the implementation uses a **right-click
context menu**. This is resolved by v1 scope:

- `about/heart-and-soul/v1.md §Mobile`: "No mobile build target. V1 is desktop/server only
  (Linux, Windows, macOS)."
- RFC 0004 §3.0: v1 gesture set for pointer devices is `Tap`, `DoubleTap`, `ContextMenu`
  (right-click); `LongPress`, `Drag`, and full gesture pipeline are V1-reserved.
- RFC 0004 §3.0 chrome carve-out: drag handles operate via compositor-internal state machine.
- The right-click → `ContextMenuEvent` → "Reset to default" context menu with 3-second auto-dismiss
  (`crates/tze_hud_runtime/src/windowed.rs`: `handle_right_click_on_drag_handle`,
  `tick_context_menu_auto_dismiss`) satisfies the v1 reset affordance requirement for pointer
  devices. The touch-tap path is post-v1 by doctrinal v1-scope boundary.

The `hud-hym1` close reason confirms: "Short-tap gesture for touch/mobile is post-v1 scope per
hud-bs2q.1 RFC amendment (v1 is desktop-only)."

**R4 verdict for v1:** Fully satisfied. Desktop right-click affordance with 3-second auto-dismiss
meets the non-conflict and auto-dismiss criteria. Touch tap is outside v1 scope.

### GAP-2 — Delta Specs Promoted

Both specs now exist at canonical paths:

- `openspec/specs/drag-to-reposition/spec.md` — contains requirements `V1-Compatible Drag Visual
  Feedback` and `Reset Gesture Is Conflict-Free On Touch/Mobile Input` with all scenarios.
- `openspec/specs/element-identity-store/spec.md` — contains requirements `Publish-To-Tile Contract
  Is Additive And Lease-Gated` and `Element Store Deletion Is Post-v1` with all scenarios.

Both files carry correct v1-mandatory scope annotations. Content matches the change specs under
`openspec/changes/persistent-movable-elements/specs/`.

---

## Full Requirement Status (gen-2)

All 9 items (R1–R4, D1–D5) from gen-1 retain their `Covered` status. No regressions found.

| ID | Requirement | v1 Status |
|----|-------------|-----------|
| R1 | Publish-To-Tile Contract additive and lease-gated | Covered (unchanged) |
| R2 | Element Store Deletion Is Post-v1 | Covered (unchanged) |
| R3 | V1-Compatible Drag Visual Feedback | Covered (unchanged) |
| R4 | Reset Gesture conflict-free (v1 pointer path) | **Now fully covered** — Tests 3+4 active |
| D1 | Chrome drag handles: 250ms pointer / 1000ms touch threshold | Covered (unchanged) |
| D2 | Capture timing carve-out for chrome chrome drag handles | Covered (unchanged) |
| D3 | RFC 0001 override stage before atomic commit | Covered (unchanged) |
| D4 | ZoneInstance and WidgetInstance gain persistent SceneId | Covered (unchanged) |
| D5 | `last_published_at` touched on `PublishToTileMutation` success | Covered (unchanged) |

---

## E2E Test Coverage (gen-2)

| Test | Name | Status |
|------|------|--------|
| 1 | `cross_session_persistence_preserves_user_geometry_override` | ACTIVE |
| 2 | `element_discovery_by_namespace_returns_correct_scene_id_with_override_preserved` | ACTIVE |
| 3 | `reset_position_clears_user_override_and_restores_agent_bounds` | **ACTIVE** (was IGNORED) |
| 4 | `zone_reset_falls_back_to_config_override_not_default_policy` | **ACTIVE** (was IGNORED) |
| 5a | `display_resolution_change_preserves_relative_center_position` | ACTIVE |
| 5b | `display_resolution_double_center_example_from_spec` | ACTIVE |
| 6 | `agent_receives_element_repositioned_event_with_old_and_new_geometry` | ACTIVE |

All 7 E2E tests active. No stubs. No ignored tests.

---

## Post-v1 Deferred Items (not gaps)

The following are intentionally out of v1 scope and require no follow-on before archive:

- **Short-tap reset affordance on touch/mobile.** Touch input path for drag handle reset is
  post-v1 by v1.md §Mobile scope boundary. The schema (`DragHandleContextMenuState`,
  `ContextMenuEvent`) is defined and stable; the touch trigger activates post-v1 without
  protocol changes.
- **Explicit element store deletion.** R2 / `element-identity-store` requirement explicitly
  defers deletion to a post-v1 layout management surface. No action needed.

---

## Verdict

**Ready to archive.**

All 9 v1-scope requirements and design decisions are fully covered by shipped, tested code.
All 7 E2E integration tests are active with no stubs. Both delta specs are promoted to the
authoritative `openspec/specs/` tree. The two deferred items (touch-tap and store deletion)
are explicitly post-v1 by doctrine and require no action before v1.

The `persistent-movable-elements` openspec change is ready for archival.

---

## Coordinator Follow-On Proposals

No new gap beads. The following archival action is recommended:

1. Run `/opsx:archive persistent-movable-elements` (or equivalent) to mark the openspec change
   as archived. This is the final step in the epic lifecycle.
2. Close epic `hud-bs2q` with reason: "All v1-scope requirements covered; gen-2 reconciliation
   confirmed; openspec change ready to archive."
