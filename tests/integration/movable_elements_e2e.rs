//! E2E integration tests for persistent movable elements.
//!
//! Tests the cross-session persistence, element discovery and reuse,
//! display resolution change invariance, and agent notification contracts
//! for user-dragged elements.
//!
//! All tests are headless Layer 0-1: no display server or GPU required.
//!
//! Source: openspec/changes/persistent-movable-elements/
//!
//! Test status overview:
//!   Test 1 — Cross-session persistence:          ACTIVE
//!   Test 2 — Element discovery and reuse:        ACTIVE
//!   Test 3 — Reset fallback chain:               IGNORED (hud-zc7f not merged)
//!   Test 4 — Zone with config override + reset:  IGNORED (hud-zc7f not merged)
//!   Test 5 — Display resolution change:          ACTIVE
//!   Test 6 — Agent notification:                 IGNORED (ElementRepositionedEvent not yet wired)

use tze_hud_input::{
    DeviceDragState, DragEventOutcome, DragHandleElementKind, DragPhase, InputProcessor,
    PointerEvent, PointerEventKind,
};
use tze_hud_scene::{
    ElementStore, ElementStoreEntry, ElementType, GeometryPolicy, Rect, SceneId,
    geometry_policy_to_absolute_rect, rect_to_relative_geometry_policy,
};

// ── Display constants ──────────────────────────────────────────────────────────

const DISPLAY_W: f32 = 1920.0;
const DISPLAY_H: f32 = 1080.0;

// Element bounds used across tests
const ELEMENT_X: f32 = 200.0;
const ELEMENT_Y: f32 = 150.0;
const ELEMENT_W: f32 = 400.0;
const ELEMENT_H: f32 = 200.0;

// Drag handle sits above the element center
const HANDLE_X: f32 = ELEMENT_X + ELEMENT_W / 2.0 - 12.0;
const HANDLE_Y: f32 = ELEMENT_Y - 4.0;
const HANDLE_W: f32 = 24.0;
const HANDLE_H: f32 = 8.0;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn element_bounds() -> Rect {
    Rect::new(ELEMENT_X, ELEMENT_Y, ELEMENT_W, ELEMENT_H)
}

fn handle_center() -> (f32, f32) {
    (HANDLE_X + HANDLE_W / 2.0, HANDLE_Y + HANDLE_H / 2.0)
}

fn pointer_up(x: f32, y: f32) -> PointerEvent {
    PointerEvent {
        x,
        y,
        kind: PointerEventKind::Up,
        device_id: 0,
        timestamp: None,
    }
}

fn pointer_move(x: f32, y: f32) -> PointerEvent {
    PointerEvent {
        x,
        y,
        kind: PointerEventKind::Move,
        device_id: 0,
        timestamp: None,
    }
}

/// Build an InputProcessor that is in the Activated phase and ready to drag.
///
/// Uses a 1ms hold threshold so tests can activate immediately without
/// real-time sleeping.
fn build_activated_processor(element_id: SceneId) -> InputProcessor {
    let (cx, cy) = handle_center();
    let mut ip = InputProcessor::new();
    // Seed with a 1ms-threshold DeviceDragState that has already been activated.
    let mut state = DeviceDragState::new(
        "drag-handle:tile".to_string(),
        element_id,
        DragHandleElementKind::Tile,
        cx,
        cy,
        1, // 1ms threshold
    );
    // Manually advance phase to Activated so we can immediately produce Moved/Released.
    state.phase = DragPhase::Activated;
    ip.drag_states.insert(0, state);
    ip
}

/// Drive the element from Activated to Released, returning `(final_x, final_y)`.
///
/// Moves the element to the target `(new_x, new_y)` then releases. Snapping
/// and clamping may adjust the final position slightly.
fn drag_to_then_release(
    ip: &mut InputProcessor,
    element_id: SceneId,
    new_x: f32,
    new_y: f32,
) -> (f32, f32) {
    let grab_off_x = ip.drag_states[&0].grab_offset_x;
    let grab_off_y = ip.drag_states[&0].grab_offset_y;
    let ptr_x = new_x + grab_off_x;
    let ptr_y = new_y + grab_off_y;

    // Move to new position
    ip.process_drag_handle_pointer(
        &pointer_move(ptr_x, ptr_y),
        "drag-handle:tile",
        element_id,
        DragHandleElementKind::Tile,
        element_bounds(),
        DISPLAY_W,
        DISPLAY_H,
    );

    // Release
    let released = ip.process_drag_handle_pointer(
        &pointer_up(ptr_x, ptr_y),
        "drag-handle:tile",
        element_id,
        DragHandleElementKind::Tile,
        element_bounds(),
        DISPLAY_W,
        DISPLAY_H,
    );

    match released {
        DragEventOutcome::Released {
            final_x, final_y, ..
        } => (final_x, final_y),
        other => panic!("expected Released outcome, got {other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Test 1 — Cross-session persistence
//
// Agent creates tile → user drags tile to new position → runtime restarts
// (ElementStore is serialized and deserialized) → agent reconnects (new scene) →
// tile appears at user-overridden position (geometry_override preserved across
// the TOML round-trip).
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn cross_session_persistence_preserves_user_geometry_override() {
    // ── Session A: agent creates tile and user drags it ───────────────────────
    let tile_namespace = "agent-persistent-tile";
    let tile_id = SceneId::new();

    // User drags the element to a new position.
    let mut ip = build_activated_processor(tile_id);
    let target_x = 600.0_f32;
    let target_y = 400.0_f32;
    let (final_x, final_y) = drag_to_then_release(&mut ip, tile_id, target_x, target_y);

    // Persist the drag geometry into an ElementStore entry.
    let mut store = ElementStore::default();
    store.entries.insert(
        tile_id,
        ElementStoreEntry {
            element_type: ElementType::Tile,
            namespace: tile_namespace.to_string(),
            created_at: 1_000,
            last_published_at: 2_000,
            geometry_override: None,
        },
    );

    InputProcessor::persist_drag_geometry(
        &mut store,
        ElementType::Tile,
        tile_namespace,
        final_x,
        final_y,
        ELEMENT_W,
        ELEMENT_H,
        DISPLAY_W,
        DISPLAY_H,
    );

    // Verify geometry_override was written.
    let entry_a = store.entries.get(&tile_id).expect("entry must exist");
    let override_a = entry_a
        .geometry_override
        .expect("geometry_override must be set after drag");

    // ── Persist → TOML → reload (simulate runtime restart) ───────────────────
    let toml_str = store
        .to_toml_string()
        .expect("ElementStore TOML serialization must succeed");
    let reloaded_store =
        ElementStore::from_toml_str(&toml_str).expect("TOML deserialization must succeed");

    // ── Session B: verify the override survived the round-trip ───────────────
    let entry_b = reloaded_store
        .entries
        .get(&tile_id)
        .expect("entry must survive restart");
    assert_eq!(
        entry_b.namespace, tile_namespace,
        "namespace must survive TOML round-trip"
    );
    assert_eq!(
        entry_b.element_type,
        ElementType::Tile,
        "element_type must survive TOML round-trip"
    );

    let override_b = entry_b
        .geometry_override
        .expect("geometry_override must survive runtime restart");

    // Override must be identical after round-trip.
    assert_eq!(
        override_b, override_a,
        "geometry_override must be byte-identical after TOML serialization → deserialization"
    );

    // The override must reflect the user-dragged position: x_pct and y_pct
    // should match final_x/DISPLAY_W and final_y/DISPLAY_H within tolerance.
    if let GeometryPolicy::Relative {
        x_pct,
        y_pct,
        width_pct,
        height_pct,
    } = override_b
    {
        let expected_x_pct = final_x / DISPLAY_W;
        let expected_y_pct = final_y / DISPLAY_H;
        assert!(
            (x_pct - expected_x_pct).abs() < 1e-4,
            "persisted x_pct ({x_pct}) must match final_x/DISPLAY_W ({expected_x_pct})"
        );
        assert!(
            (y_pct - expected_y_pct).abs() < 1e-4,
            "persisted y_pct ({y_pct}) must match final_y/DISPLAY_H ({expected_y_pct})"
        );
        assert!(
            (width_pct - ELEMENT_W / DISPLAY_W).abs() < 1e-4,
            "persisted width_pct must match element_w/display_w"
        );
        assert!(
            (height_pct - ELEMENT_H / DISPLAY_H).abs() < 1e-4,
            "persisted height_pct must match element_h/display_h"
        );
    } else {
        panic!("expected Relative geometry policy, got {override_b:?}");
    }

    // Session B: agent-requested bounds (the tile's Rect) must be SUPERSEDED by the
    // user override. Resolve using the four-tier chain:
    //   user_override > agent_requested > config_override > default.
    let agent_requested = rect_to_relative_geometry_policy(
        Rect::new(ELEMENT_X, ELEMENT_Y, ELEMENT_W, ELEMENT_H),
        DISPLAY_W,
        DISPLAY_H,
    );
    let resolved = tze_hud_scene::resolve_geometry_override_chain(
        Some(override_b),
        Some(agent_requested),
        None,
        None,
    )
    .expect("chain must resolve to the user override");

    assert_eq!(
        resolved, override_b,
        "user override must win over agent-requested geometry in the resolution chain"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Test 2 — Element discovery and reuse
//
// Agent creates tile (gets SceneId) → disconnects → reconnects → queries the
// ElementStore by namespace filter → finds the same SceneId → user-override is
// preserved (geometry_override unchanged by reconnect).
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn element_discovery_by_namespace_returns_correct_scene_id_with_override_preserved() {
    let tile_namespace = "agent-discoverable-tile";
    let tile_id = SceneId::new();

    // Simulate Session A: tile is created, user drags it, geometry is persisted.
    let user_override = GeometryPolicy::Relative {
        x_pct: 0.35,
        y_pct: 0.25,
        width_pct: 0.20,
        height_pct: 0.15,
    };

    let mut store = ElementStore::default();
    store.entries.insert(
        tile_id,
        ElementStoreEntry {
            element_type: ElementType::Tile,
            namespace: tile_namespace.to_string(),
            created_at: 5_000,
            last_published_at: 6_000,
            geometry_override: Some(user_override),
        },
    );

    // Also add an unrelated entry to ensure namespace filtering is scoped.
    let other_id = SceneId::new();
    store.entries.insert(
        other_id,
        ElementStoreEntry {
            element_type: ElementType::Tile,
            namespace: "other-agent-tile".to_string(),
            created_at: 5_001,
            last_published_at: 6_001,
            geometry_override: None,
        },
    );

    // ── Simulate runtime restart: persist → reload ────────────────────────────
    let toml_str = store.to_toml_string().expect("serialize must succeed");
    let reloaded = ElementStore::from_toml_str(&toml_str).expect("deserialize must succeed");

    // ── Session B: agent reconnects and queries by namespace ──────────────────
    // find_id_by_type_namespace simulates the ListElements(namespace_filter="agent-discoverable-")
    // lookup the runtime performs when reconnecting.
    let found_id = reloaded
        .find_id_by_type_namespace(ElementType::Tile, tile_namespace)
        .expect("element must be discoverable by (type, namespace) after restart");

    assert_eq!(
        found_id, tile_id,
        "discovered element_id must match the original tile_id from Session A"
    );

    // Namespace filter must not return unrelated entries.
    let other_found = reloaded.find_id_by_type_namespace(ElementType::Tile, "other-agent-tile");
    assert!(
        other_found.is_some(),
        "other-agent-tile must also be findable (not silently dropped)"
    );
    assert_ne!(
        other_found.unwrap(),
        tile_id,
        "other-agent-tile must not be confused with agent-discoverable-tile"
    );

    // User override must be intact — reconnect must NOT clear geometry_override.
    let reloaded_entry = reloaded
        .entries
        .get(&found_id)
        .expect("entry must exist for found_id");
    let reloaded_override = reloaded_entry
        .geometry_override
        .expect("user geometry_override must survive reconnect");

    assert_eq!(
        reloaded_override, user_override,
        "geometry_override must be identical after reconnect (user position preserved)"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Test 3 — Reset fallback chain (DEFERRED — hud-zc7f not yet merged)
//
// User drags element → user resets position → element returns to
// agent-requested bounds (geometry_override cleared).
//
// Requires: reset-position API from hud-zc7f (ResetElementPosition mutation /
// clear_geometry_override RPC). Ship when hud-zc7f lands on main.
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
#[ignore = "hud-zc7f (reset UI) not yet merged; enable after ElementStore.clear_geometry_override is wired"]
fn reset_position_clears_user_override_and_restores_agent_bounds() {
    // Prereq: reset-position API from hud-zc7f must be available.
    // Expected flow:
    //   1. Set geometry_override on an ElementStore entry (simulate post-drag).
    //   2. User triggers reset (calls clear_geometry_override / ResetElementPosition).
    //   3. geometry_override is None on the entry.
    //   4. resolve_geometry_override_chain returns the agent-requested bounds.
    unimplemented!("enable after hud-zc7f lands: clear_geometry_override API required");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Test 4 — Zone with config override (DEFERRED — hud-zc7f not yet merged)
//
// Zone with config-level geometry_override → user drags → user resets →
// zone returns to config override (NOT default geometry policy).
//
// Requires: reset-position API from hud-zc7f. After reset, the chain
// user_override=None, agent_requested=None, config_override=Some(X) must
// return X (not None / default).
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
#[ignore = "hud-zc7f (reset UI) not yet merged; enable after ElementStore.clear_geometry_override and zone reset path are wired"]
fn zone_reset_falls_back_to_config_override_not_default_policy() {
    // Prereq: reset-position API from hud-zc7f must be available.
    // Expected flow:
    //   1. Zone has config_override=Some(GeometryPolicy::Relative { x=0.1, … }).
    //   2. User drags → geometry_override set.
    //   3. User resets → geometry_override cleared (user_override=None).
    //   4. resolve_geometry_override_chain(None, None, Some(config_override), Some(default))
    //      returns config_override — NOT default.
    unimplemented!("enable after hud-zc7f lands: zone reset must resolve to config_override");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Test 5 — Display resolution change
//
// User drags element to center of 1920×1080 (x_pct=0.5, y_pct=0.5) → display
// changes to 3840×2160 → element renders at (1920, 1080) — still center.
//
// The geometry_override is stored as relative percentages, so the element
// tracks the same proportional position regardless of display resolution.
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn display_resolution_change_preserves_relative_center_position() {
    // ── Original display: 1920×1080 ───────────────────────────────────────────
    let original_w = 1920.0_f32;
    let original_h = 1080.0_f32;

    // Element size (constant, absolute pixels — same physical component size).
    let elem_w = 384.0_f32; // 20% of 1920
    let elem_h = 216.0_f32; // 20% of 1080

    // User drags element to center of original display.
    // Center means top-left is at (display/2 - width/2, display/2 - height/2).
    let center_x = (original_w - elem_w) / 2.0; // 768.0
    let center_y = (original_h - elem_h) / 2.0; // 432.0

    // Persist the geometry override as a relative policy.
    let mut store = ElementStore::default();
    let tile_id = SceneId::new();
    store.entries.insert(
        tile_id,
        ElementStoreEntry {
            element_type: ElementType::Tile,
            namespace: "resolution-test-tile".to_string(),
            created_at: 1,
            last_published_at: 1,
            geometry_override: None,
        },
    );

    InputProcessor::persist_drag_geometry(
        &mut store,
        ElementType::Tile,
        "resolution-test-tile",
        center_x,
        center_y,
        elem_w,
        elem_h,
        original_w,
        original_h,
    );

    let entry = store.entries.get(&tile_id).expect("entry must exist");
    let override_policy = entry
        .geometry_override
        .expect("geometry_override must be set");

    // Verify the override was stored as x_pct=0.4, y_pct=0.4 (center_x/W = 768/1920 = 0.4).
    if let GeometryPolicy::Relative { x_pct, y_pct, .. } = override_policy {
        let expected_x_pct = center_x / original_w;
        let expected_y_pct = center_y / original_h;
        assert!(
            (x_pct - expected_x_pct).abs() < 1e-4,
            "x_pct must be center_x/original_w = {expected_x_pct}, got {x_pct}"
        );
        assert!(
            (y_pct - expected_y_pct).abs() < 1e-4,
            "y_pct must be center_y/original_h = {expected_y_pct}, got {y_pct}"
        );
    } else {
        panic!("expected Relative geometry policy, got {override_policy:?}");
    }

    // ── Display changes to 3840×2160 (2× HiDPI) ──────────────────────────────
    let new_w = 3840.0_f32;
    let new_h = 2160.0_f32;

    // Apply the same relative override policy to the new display size.
    let new_rect = geometry_policy_to_absolute_rect(override_policy, new_w, new_h);

    // Element top-left should now be at (1920, 1080) — the center of 3840×2160.
    let expected_new_x = center_x / original_w * new_w; // 768/1920 * 3840 = 1536
    let expected_new_y = center_y / original_h * new_h; // 432/1080 * 2160 = 864

    assert!(
        (new_rect.x - expected_new_x).abs() < 1.0,
        "at 3840×2160, element x must be {expected_new_x}px (same relative position), got {}",
        new_rect.x
    );
    assert!(
        (new_rect.y - expected_new_y).abs() < 1.0,
        "at 3840×2160, element y must be {expected_new_y}px (same relative position), got {}",
        new_rect.y
    );

    // Width and height must also scale proportionally.
    let expected_new_w = elem_w / original_w * new_w; // 384/1920 * 3840 = 768
    let expected_new_h = elem_h / original_h * new_h; // 216/1080 * 2160 = 432
    assert!(
        (new_rect.width - expected_new_w).abs() < 1.0,
        "element width must scale to {expected_new_w} at new resolution, got {}",
        new_rect.width
    );
    assert!(
        (new_rect.height - expected_new_h).abs() < 1.0,
        "element height must scale to {expected_new_h} at new resolution, got {}",
        new_rect.height
    );

    // Confirm proportional center: x_pct and y_pct of the result on the new display
    // must equal the original percentages.
    let result_policy = rect_to_relative_geometry_policy(new_rect, new_w, new_h);
    if let GeometryPolicy::Relative {
        x_pct: rx,
        y_pct: ry,
        ..
    } = result_policy
    {
        if let GeometryPolicy::Relative {
            x_pct: ox,
            y_pct: oy,
            ..
        } = override_policy
        {
            assert!(
                (rx - ox).abs() < 1e-4,
                "x_pct must be invariant under resolution change: expected {ox}, got {rx}"
            );
            assert!(
                (ry - oy).abs() < 1e-4,
                "y_pct must be invariant under resolution change: expected {oy}, got {ry}"
            );
        }
    }
}

/// Variant: element at exact 50% center of 1920×1080 renders at (1920, 1080) on 3840×2160.
///
/// This is the canonical example from the spec: x_pct=0.5, y_pct=0.5 on a
/// display that doubles in each dimension maps to (new_w/2, new_h/2).
#[test]
fn display_resolution_double_center_example_from_spec() {
    // Store a 50% / 50% relative override directly (no drag needed).
    let override_policy = GeometryPolicy::Relative {
        x_pct: 0.5,
        y_pct: 0.5,
        width_pct: 0.2,
        height_pct: 0.2,
    };

    let original_w = 1920.0_f32;
    let original_h = 1080.0_f32;

    // On the original display, this places the element at (960, 540).
    let orig_rect = geometry_policy_to_absolute_rect(override_policy, original_w, original_h);
    assert!(
        (orig_rect.x - 960.0).abs() < 0.5,
        "x on 1920×1080 must be 960 (50%), got {}",
        orig_rect.x
    );
    assert!(
        (orig_rect.y - 540.0).abs() < 0.5,
        "y on 1920×1080 must be 540 (50%), got {}",
        orig_rect.y
    );

    // On a 3840×2160 display the same percentages place the element at (1920, 1080).
    let new_w = 3840.0_f32;
    let new_h = 2160.0_f32;
    let new_rect = geometry_policy_to_absolute_rect(override_policy, new_w, new_h);

    assert!(
        (new_rect.x - 1920.0).abs() < 0.5,
        "x on 3840×2160 must be 1920 (50%), got {}",
        new_rect.x
    );
    assert!(
        (new_rect.y - 1080.0).abs() < 0.5,
        "y on 3840×2160 must be 1080 (50%), got {}",
        new_rect.y
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Test 6 — Agent notification (DEFERRED — ElementRepositionedEvent not wired)
//
// Agent creates tile → user drags → agent receives ElementRepositionedEvent
// with correct old_geometry and new_geometry.
//
// Requires: ElementRepositionedEvent in the gRPC ServerMessage stream and the
// session_server wiring that emits it on DragEventOutcome::Released.
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
#[ignore = "ElementRepositionedEvent not yet implemented; enable after the gRPC event emission is wired in session_server.rs"]
fn agent_receives_element_repositioned_event_with_old_and_new_geometry() {
    // Prereqs:
    //   - ElementRepositionedEvent added to ServerMessage::payload oneof in proto/session.proto
    //   - session_server.rs emits the event when DragEventOutcome::Released is processed
    //   - Event carries: element_id, old_geometry (agent-requested Rect as Relative),
    //     new_geometry (user-overridden Relative from geometry_override)
    //
    // Expected flow:
    //   1. Agent connects, creates tile at agent_bounds.
    //   2. User long-press drags tile to new position (DragEventOutcome::Released emitted).
    //   3. Runtime persists geometry to ElementStore.
    //   4. Agent receives ElementRepositionedEvent {
    //          element_id: tile_id,
    //          old_geometry: rect_to_relative(agent_bounds, display),
    //          new_geometry: geometry_override (user position),
    //      }.
    //   5. Test verifies both geometry fields are correct.
    unimplemented!(
        "enable after ElementRepositionedEvent is added to proto/session.proto and \
         emitted in session_server.rs on DragEventOutcome::Released"
    );
}
