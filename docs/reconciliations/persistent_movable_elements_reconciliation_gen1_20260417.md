# Persistent Movable Elements Reconciliation (gen-1)

Date: 2026-04-17
Issue: `hud-bs2q.8`
Epic: `hud-bs2q` (Persistent movable elements)

## Inputs Audited

- `openspec/changes/persistent-movable-elements/specs/element-identity-store/spec.md`
- `openspec/changes/persistent-movable-elements/specs/drag-to-reposition/spec.md`
- `openspec/changes/persistent-movable-elements/design.md`
- `openspec/changes/persistent-movable-elements/proposal.md`
- `about/legends-and-lore/rfcs/0001-scene-contract.md` (§ PublishToTileMutation, § override pipeline)
- `about/legends-and-lore/rfcs/0004-input.md` (§3.0 chrome carve-out, § drag-handle timing)
- `crates/tze_hud_scene/src/element_store.rs`
- `crates/tze_hud_runtime/src/element_store.rs`
- `crates/tze_hud_input/src/drag.rs`
- `crates/tze_hud_input/src/lib.rs`
- `crates/tze_hud_compositor/src/renderer.rs`
- `crates/tze_hud_protocol/src/session_server.rs`
- `crates/tze_hud_protocol/proto/types.proto`
- `crates/tze_hud_protocol/proto/events.proto`
- `crates/tze_hud_protocol/proto/session.proto`
- `crates/tze_hud_scene/src/types.rs`
- `crates/tze_hud_scene/src/graph.rs`
- `crates/tze_hud_scene/src/mutation.rs`
- `crates/tze_hud_runtime/src/windowed.rs`
- `tests/integration/drag_reposition.rs`
- `tests/integration/movable_elements_e2e.rs`

## Scope Note

The two spec files each have an `## ADDED Requirements` heading — these are new requirements introduced by the `persistent-movable-elements` change (not amendments to existing requirement sets). Both spec files total **4 explicitly numbered requirements** (R1–R4 below) with **9 scenarios**. The design.md contributes an additional **5 implementation decisions** (D1–D5) that have direct code obligations. All 9 items are tracked in the matrix below.

---

## Requirement-to-Bead Coverage Matrix

| ID | Requirement | Primary implementing bead(s) | Status | Evidence |
|---|---|---|---|---|
| R1 | `element-identity-store` :: Publish-To-Tile Contract Is Additive And Lease-Gated | `hud-bs2q.3` | Covered | `PublishToTileMutation` coexists with `SetTileRootMutation` in the protobuf oneof at field 10: `crates/tze_hud_protocol/proto/types.proto:186`; runtime dispatch paths are separate branches in `handle_mutation_batch`: `crates/tze_hud_protocol/src/session_server.rs:3371` and `:3847`; lease/capability validation flows through `update_tile_bounds` → `get_tile_lease_checked` + `require_active_lease` + `require_capability(ModifyOwnTiles)`: `crates/tze_hud_scene/src/graph.rs:1719–1730`; element-not-found rejection: `crates/tze_hud_protocol/src/session_server.rs:3388`; tests: `test_publish_to_tile_by_element_id_applies_override_and_updates_timestamp` (:7375), `test_publish_to_tile_by_element_id_rejects_invalid_node_even_with_bounds` (:7541), `test_publish_to_tile_by_element_id_returns_element_not_found` (:7669). |
| R2 | `element-identity-store` :: Element Store Deletion Is Post-v1 | `hud-bs2q.2` | Covered | `ElementStore.entries` is a `HashMap` with no `remove` method exposed on the public API; only `reset_geometry_override` (which clears the override field, not the entry) is provided: `crates/tze_hud_scene/src/element_store.rs:117`; no deletion call sites exist in the codebase; monotonic growth is the only path. |
| R3 | `drag-to-reposition` :: V1-Compatible Drag Visual Feedback | `hud-bs2q.4`, `hud-bs2q.5` | Covered | Constants: `DRAG_Z_ORDER_BOOST = 0x1000`, `DRAG_HIGHLIGHT_BORDER_PX = 2.0`, `DRAG_OPACITY_BOOST = 1.0` (immediate, not animated): `crates/tze_hud_input/src/drag.rs:53–61`; compositor applies z-order boost, 2px highlight border quads, and opacity-active state on `drag_active_elements`: `crates/tze_hud_compositor/src/renderer.rs:458,5695`; comment explicitly cites spec prohibition on drop shadows/scale pulses: `:455,5702`; `drag_accumulation_progress()` returns `(elapsed_ms / threshold_ms).clamp(0.0, 1.0)` — state-derived, not a timer: `crates/tze_hud_input/src/drag.rs:154`; test: `drag_visual_feedback_applied_during_active_drag` in `crates/tze_hud_compositor/src/renderer.rs:13119`. |
| R4 | `drag-to-reposition` :: Reset Gesture Is Conflict-Free On Touch/Mobile Input | `hud-zc7f` | **Partially covered (GAP-1)** | Right-click on drag handle shows context menu anchored at cursor, with "Reset to default" button and 3-second auto-dismiss: `crates/tze_hud_runtime/src/windowed.rs` (`handle_right_click_on_drag_handle`, `tick_context_menu_auto_dismiss`); `DragHandleContextMenuState` struct: `crates/tze_hud_scene/src/types.rs`; reset call path: `crates/tze_hud_protocol/src/session_server.rs:1717`; long-press flow remains dedicated to drag activation via `DragPhase::Activated`: `crates/tze_hud_input/src/lib.rs:1180`. **GAP:** The spec requires a *short tap* (not right-click) on the drag handle to reveal the reset affordance — the v1 desktop path uses right-click, which is appropriate for pointer devices, but the E2E tests for reset (Tests 3 and 4 in `tests/integration/movable_elements_e2e.rs:379,402`) remain `#[ignore]` as `unimplemented!()` stubs even though `hud-zc7f` has merged. Reset path is not exercised by any active passing test. |
| D1 | Design §2: Chrome drag handles use compositor-internal interaction path; pointer 250ms hold, touch 1000ms hold | `hud-bs2q.4`, `hud-bs2q.5` | Covered | `LONG_PRESS_POINTER_THRESHOLD_MS = 250`, `LONG_PRESS_TOUCH_THRESHOLD_MS = 1000`: `crates/tze_hud_input/src/drag.rs:38–40`; device-type routing in `InputProcessor::process_drag_event` uses threshold from device context: `crates/tze_hud_input/src/lib.rs:1117`; RFC 0004 §3.0 chrome carve-out documented. |
| D2 | Design §3: Capture timing carve-out — runtime chrome may acquire/release at any event phase | `hud-bs2q.4` | Covered | RFC 0004 §5.1 amended with explicit carve-out note; compositor-internal drag state machine operates outside agent gesture pipeline. |
| D3 | Design §7: RFC 0001 transaction pipeline gains `Runtime Override Application` stage before atomic commit | `hud-bs2q.3` | Covered | `resolve_tile_bounds_with_override` applies user geometry override before passing `UpdateTileBounds`/`SetTileRoot` mutations to the scene's atomic `apply_batch`: `crates/tze_hud_protocol/src/session_server.rs:756,3395`; override is applied in the conversion phase, results committed atomically; test: `test_publish_to_tile_by_element_id_applies_override_and_updates_timestamp` validates override geometry wins: `:7375`. |
| D4 | Design §8: Factual correction — ZoneDefinition carries `SceneId` already; identity persistence extended to `ZoneInstance` and `WidgetInstance` | `hud-bs2q.2`, `hud-bs2q.3` | Covered | `ElementType` enum covers `Zone`, `Widget`, `Tile`: `crates/tze_hud_scene/src/element_store.rs:18`; `reconcile_scene_ids` re-uses stored IDs for zone, widget instance, and tile entries across restarts: `crates/tze_hud_runtime/src/element_store.rs:87`; `bootstrap_round_trip_reuses_ids_across_restart`: `:383`. |
| D5 | Design §6: `PublishToTileMutation` resolves element identity, applies runtime override, and `last_published_at` is touched on success | `hud-bs2q.3` | Covered | Element store entry's `last_published_at` is updated via `touch_element_store_entry_by_id` after successful publish: `crates/tze_hud_protocol/src/session_server.rs:3432`; test asserts timestamp advances: `:7535`. |

---

## E2E Test Coverage Matrix

| Test | Name | Status | Bead |
|---|---|---|---|
| 1 | `cross_session_persistence_preserves_user_geometry_override` | ACTIVE | `hud-bs2q.3`, `hud-bs2q.5` |
| 2 | `element_discovery_by_namespace_returns_correct_scene_id_with_override_preserved` | ACTIVE | `hud-bs2q.2`, `hud-bs2q.3` |
| 3 | `reset_position_clears_user_override_and_restores_agent_bounds` | **IGNORED** (stub — GAP-1) | `hud-zc7f` |
| 4 | `zone_reset_falls_back_to_config_override_not_default_policy` | **IGNORED** (stub — GAP-1) | `hud-zc7f` |
| 5a | `display_resolution_change_preserves_relative_center_position` | ACTIVE | `hud-bs2q.3` |
| 5b | `display_resolution_double_center_example_from_spec` | ACTIVE | `hud-bs2q.3` |
| 6 | `agent_receives_element_repositioned_event_with_old_and_new_geometry` | ACTIVE | `hud-bs2q.6`, `hud-zc7f` |

---

## Gaps Requiring Follow-On Beads

### GAP-1 (test coverage): Reset-to-default E2E tests remain `#[ignore]` stubs

`hud-zc7f` has merged and `reset_element_geometry` is implemented in
`crates/tze_hud_protocol/src/session_server.rs:1717`. The context menu entry
point is wired in `crates/tze_hud_runtime/src/windowed.rs`. However, E2E Tests
3 and 4 in `tests/integration/movable_elements_e2e.rs:379,402` are still
`#[ignore = "hud-zc7f (reset UI) not yet merged"]` and contain only
`unimplemented!()` bodies. These tests need to be implemented against the now-
landed API and un-ignored.

Additionally, the spec states the reset affordance is triggered by a **short tap**
on the drag handle (for touch/mobile). The current implementation uses a
**right-click** context menu (desktop path). This is spec-compliant for pointer
devices, but the touch tap → affordance path is unimplemented and untested.

### GAP-2 (spec sync): Delta specs not synced to v1-mvp-standards

`openspec/changes/persistent-movable-elements/specs/drag-to-reposition/spec.md`
and `.../element-identity-store/spec.md` have not been promoted into the
authoritative `openspec/specs/` or `openspec/changes/v1-mvp-standards/` trees.
Neither spec directory contains any `drag-to-reposition` or
`element-identity-store` entries.

---

## Coverage Verdict

1. **8 of 9 requirements/design decisions are fully covered** with shipped, tested
   code: R1, R2, R3, D1, D2, D3, D4, D5.
2. **1 requirement is partially covered (GAP-1):** R4 (reset gesture) — the reset
   path is implemented but two E2E tests remain as `unimplemented!()` stubs and the
   touch-specific tap → affordance path is absent.
3. **1 spec-sync gap (GAP-2):** the delta specs have not been promoted into the
   authoritative spec store.
4. Full epic closeout is **not yet justified** — GAP-1 and GAP-2 must close before
   the epic is marked done.
5. A gen-2 reconciliation should be filed to verify after gap beads land.

---

## Coordinator Follow-On Proposals

The worker cannot mutate bead lifecycle state. Materialize the following follow-on
beads under epic `hud-bs2q`:

1. `title`: `Implement and un-ignore reset-to-default E2E tests (Tests 3 and 4)`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-bs2q.8`
   `rationale`: `Close GAP-1. hud-zc7f merged but Tests 3 and 4 in tests/integration/movable_elements_e2e.rs remain #[ignore] stubs with unimplemented!() bodies. Tests must be implemented against reset_element_geometry (session_server.rs:1717) and un-ignored. Also verify touch-tap → affordance path or document desktop-only scope explicitly.`

2. `title`: `Sync persistent-movable-elements delta specs into v1-mvp-standards`
   `type`: `task`
   `priority`: `2`
   `depends_on`: `discovered-from:hud-bs2q.8`
   `rationale`: `Close GAP-2. The drag-to-reposition and element-identity-store specs from openspec/changes/persistent-movable-elements/ have not been promoted into the authoritative openspec/specs/ or v1-mvp-standards trees. Run /opsx:sync or equivalent after GAP-1 closes and full coverage is confirmed.`

3. `title`: `Reconcile spec-to-code (gen-2) for persistent movable elements`
   `type`: `task`
   `priority`: `2`
   `depends_on`: `discovered-from:hud-bs2q.8`
   `rationale`: `After gap beads close, run a gen-2 pass to confirm full coverage, un-ignore the E2E tests, and archive the persistent-movable-elements openspec change.`
