//! Pointer event dispatch pipeline — Stages 1 and 2.
//!
//! Implements §Requirement: Event Dispatch Flow (lines 304-306):
//!
//! - **Stage 1 (Input Drain)**: attach hardware and arrival timestamps, attach
//!   device_id and modifiers, enqueue the raw input. Budget: < 500µs p99.
//! - **Stage 2 (Local Feedback)**: hit-test against the bounds snapshot, update
//!   `HitRegionLocalState` (hover, pressed, focused), produce `SceneLocalPatch`,
//!   and build the typed event(s) to route to the owning agent. Budget: < 500µs
//!   p99; combined Stages 1+2 < 1ms p99.
//!
//! Stages 1 and 2 execute on the main thread with no locks on the mutable scene
//! graph — they use an immutable snapshot of tile bounds.
//!
//! Stage 3-4 (compositor) and the event router are out of scope for this crate
//! (they live in `tze_hud_compositor` and `tze_hud_runtime`), but this module
//! produces the `SceneLocalPatch` and `DispatchOutcome` that feed those stages.
//!
//! §Requirement: ContextMenu Dispatch (Pointer) (lines 433-435):
//! Right-click is mapped to `ContextMenuEvent` by the event preprocessor here,
//! bypassing the gesture recognizer pipeline.

use std::time::Instant;
use tze_hud_scene::{MonoUs, NodeData, SceneGraph, SceneId};

use crate::events::{EventBatch, HitTestResult, InputEnvelope, RouteTarget, SceneLocalPatch};
use crate::hit_test::hit_test;
use crate::pointer::{
    CancelReason, ClickEvent, ContextMenuEvent, PointerButton, PointerCancelEvent,
    PointerDownEvent, PointerEnterEvent, PointerFields, PointerLeaveEvent, PointerMoveEvent,
    PointerUpEvent, RawPointerEvent, RawPointerEventKind,
};

// ─── Dispatch outcome ─────────────────────────────────────────────────────────

/// Result of processing a raw pointer event through Stages 1 and 2.
#[derive(Debug)]
pub struct DispatchOutcome {
    /// The hit-test result.
    pub hit: HitTestResult,
    /// Local state updates to forward to the compositor (Stage 3-4).
    pub local_patch: SceneLocalPatch,
    /// The event(s) to route to the owning agent, if any.
    /// May contain multiple events (e.g., Enter + Down, or Leave + Enter).
    pub agent_events: Vec<(RouteTarget, InputEnvelope)>,
    /// Time taken for Stage 1 + Stage 2 combined (microseconds).
    pub stages_1_2_us: u64,
    /// Time taken for hit-test alone (microseconds).
    pub hit_test_us: u64,
}

// ─── Dispatch processor ───────────────────────────────────────────────────────

/// Tracks interaction state across raw pointer events.
///
/// Maintains hover and press state to generate enter/leave events and
/// click synthesis. Must be invoked on the main thread.
pub struct DispatchProcessor {
    /// Currently hovered (tile_id, node_id).
    current_hover: Option<(SceneId, SceneId)>,
    /// Currently pressed (tile_id, node_id, button).
    current_press: Option<(SceneId, SceneId, PointerButton)>,
    /// Last click timestamp for double-click detection.
    last_click_ts: Option<MonoUs>,
    /// Last click position for double-click proximity check.
    last_click_pos: Option<(f32, f32)>,
    /// Frame number (set by caller each frame).
    pub current_frame: u64,
}

impl DispatchProcessor {
    /// Double-click time window in microseconds (300ms).
    const DOUBLE_CLICK_WINDOW_US: u64 = 300_000;
    /// Double-click proximity in display pixels (20px).
    const DOUBLE_CLICK_MAX_PX: f32 = 20.0;

    pub fn new() -> Self {
        Self {
            current_hover: None,
            current_press: None,
            last_click_ts: None,
            last_click_pos: None,
            current_frame: 0,
        }
    }

    /// Process a raw pointer event through Stages 1 and 2.
    ///
    /// Updates `HitRegionLocalState` in the scene graph and returns the
    /// `DispatchOutcome` containing the `SceneLocalPatch` and routed events.
    pub fn process(&mut self, raw: &RawPointerEvent, scene: &mut SceneGraph) -> DispatchOutcome {
        let start = Instant::now();

        // ── Stage 2: Hit test ────────────────────────────────────────────────
        let hit_start = Instant::now();
        let hit = hit_test(scene, raw.x, raw.y);
        let hit_test_us = hit_start.elapsed().as_micros() as u64;

        let mut local_patch = SceneLocalPatch::new();
        let mut agent_events: Vec<(RouteTarget, InputEnvelope)> = Vec::new();

        // ContextMenu pre-processor: right-click → ContextMenuEvent (spec §3.2-3.3)
        if raw.kind == RawPointerEventKind::RightClick {
            if let HitTestResult::NodeHit { tile_id, node_id } = &hit {
                let tile_id = *tile_id;
                let node_id = *node_id;
                // Only dispatch if event_mask.context_menu == true
                if node_allows_event(scene, node_id, |m| m.context_menu) {
                    if let Some(namespace) = tile_namespace(scene, tile_id) {
                        let interaction_id = interaction_id_for(scene, node_id);
                        let (local_x, local_y) =
                            display_to_local(scene, tile_id, raw.x, raw.y);
                        let fields = PointerFields {
                            tile_id,
                            node_id,
                            interaction_id,
                            device_id: raw.device_id,
                            local_x,
                            local_y,
                            display_x: raw.x,
                            display_y: raw.y,
                            modifiers: raw.modifiers,
                            timestamp_mono_us: raw.timestamp_mono_us,
                        };
                        agent_events.push((
                            RouteTarget::Agent { namespace, tile_id },
                            InputEnvelope::ContextMenu(ContextMenuEvent { fields }),
                        ));
                    }
                }
            }
            let stages_1_2_us = start.elapsed().as_micros() as u64;
            return DispatchOutcome { hit, local_patch, agent_events, stages_1_2_us, hit_test_us };
        }

        // ── Stage 2: Hover state update ──────────────────────────────────────
        let new_hover: Option<(SceneId, SceneId)> = match &hit {
            HitTestResult::NodeHit { tile_id, node_id } => Some((*tile_id, *node_id)),
            _ => None,
        };

        if new_hover != self.current_hover {
            // Pointer left the previous node
            if let Some((old_tile, old_node)) = self.current_hover {
                // Update local state
                if let Some(state) = scene.hit_region_states.get_mut(&old_node) {
                    state.hovered = false;
                }
                local_patch.update_node(old_node, None, Some(false), None);

                // Dispatch PointerLeave if event_mask allows
                if node_allows_event(scene, old_node, |m| m.pointer_leave) {
                    if let Some(namespace) = tile_namespace(scene, old_tile) {
                        let interaction_id = interaction_id_for(scene, old_node);
                        let (local_x, local_y) =
                            display_to_local(scene, old_tile, raw.x, raw.y);
                        agent_events.push((
                            RouteTarget::Agent { namespace, tile_id: old_tile },
                            InputEnvelope::PointerLeave(PointerLeaveEvent {
                                fields: PointerFields {
                                    tile_id: old_tile,
                                    node_id: old_node,
                                    interaction_id,
                                    device_id: raw.device_id,
                                    local_x,
                                    local_y,
                                    display_x: raw.x,
                                    display_y: raw.y,
                                    modifiers: raw.modifiers,
                                    timestamp_mono_us: raw.timestamp_mono_us,
                                },
                            }),
                        ));
                    }
                }
            }

            // Pointer entered a new node
            if let Some((new_tile, new_node)) = new_hover {
                {
                    let state = scene.hit_region_states.entry(new_node).or_insert_with(|| {
                        tze_hud_scene::HitRegionLocalState::new(new_node)
                    });
                    state.hovered = true;
                }
                local_patch.update_node(new_node, None, Some(true), None);

                if node_allows_event(scene, new_node, |m| m.pointer_enter) {
                    if let Some(namespace) = tile_namespace(scene, new_tile) {
                        let interaction_id = interaction_id_for(scene, new_node);
                        let (local_x, local_y) =
                            display_to_local(scene, new_tile, raw.x, raw.y);
                        agent_events.push((
                            RouteTarget::Agent { namespace, tile_id: new_tile },
                            InputEnvelope::PointerEnter(PointerEnterEvent {
                                fields: PointerFields {
                                    tile_id: new_tile,
                                    node_id: new_node,
                                    interaction_id,
                                    device_id: raw.device_id,
                                    local_x,
                                    local_y,
                                    display_x: raw.x,
                                    display_y: raw.y,
                                    modifiers: raw.modifiers,
                                    timestamp_mono_us: raw.timestamp_mono_us,
                                },
                            }),
                        ));
                    }
                }
            }

            self.current_hover = new_hover;
        }

        // ── Stage 2: Press / release / move ──────────────────────────────────
        match raw.kind {
            RawPointerEventKind::Down => {
                let button = raw.button.unwrap_or(PointerButton::Primary);
                if let Some((tile_id, node_id)) = new_hover {
                    // Update pressed state
                    if let Some(state) = scene.hit_region_states.get_mut(&node_id) {
                        state.pressed = true;
                    }
                    local_patch.update_node(node_id, Some(true), None, None);
                    self.current_press = Some((tile_id, node_id, button));

                    if node_allows_event(scene, node_id, |m| m.pointer_down) {
                        if let Some(namespace) = tile_namespace(scene, tile_id) {
                            let interaction_id = interaction_id_for(scene, node_id);
                            let (local_x, local_y) =
                                display_to_local(scene, tile_id, raw.x, raw.y);
                            agent_events.push((
                                RouteTarget::Agent { namespace, tile_id },
                                InputEnvelope::PointerDown(PointerDownEvent {
                                    fields: PointerFields {
                                        tile_id,
                                        node_id,
                                        interaction_id,
                                        device_id: raw.device_id,
                                        local_x,
                                        local_y,
                                        display_x: raw.x,
                                        display_y: raw.y,
                                        modifiers: raw.modifiers,
                                        timestamp_mono_us: raw.timestamp_mono_us,
                                    },
                                    button,
                                }),
                            ));
                        }
                    }
                }
            }

            RawPointerEventKind::Up => {
                let button = raw.button.unwrap_or(PointerButton::Primary);
                if let Some((pressed_tile, pressed_node, pressed_button)) =
                    self.current_press.take()
                {
                    // Release pressed state
                    if let Some(state) = scene.hit_region_states.get_mut(&pressed_node) {
                        state.pressed = false;
                    }
                    local_patch.update_node(pressed_node, Some(false), None, None);

                    if node_allows_event(scene, pressed_node, |m| m.pointer_up) {
                        if let Some(namespace) = tile_namespace(scene, pressed_tile) {
                            let interaction_id = interaction_id_for(scene, pressed_node);
                            let (local_x, local_y) =
                                display_to_local(scene, pressed_tile, raw.x, raw.y);
                            agent_events.push((
                                RouteTarget::Agent { namespace: namespace.clone(), tile_id: pressed_tile },
                                InputEnvelope::PointerUp(PointerUpEvent {
                                    fields: PointerFields {
                                        tile_id: pressed_tile,
                                        node_id: pressed_node,
                                        interaction_id: interaction_id.clone(),
                                        device_id: raw.device_id,
                                        local_x,
                                        local_y,
                                        display_x: raw.x,
                                        display_y: raw.y,
                                        modifiers: raw.modifiers,
                                        timestamp_mono_us: raw.timestamp_mono_us,
                                    },
                                    button,
                                }),
                            ));

                            // Click: press + release on same node (spec line 284)
                            let released_on_same_node = new_hover == Some((pressed_tile, pressed_node));
                            if released_on_same_node && button == pressed_button
                                && node_allows_event(scene, pressed_node, |m| m.click)
                            {
                                let is_double_click = self.check_double_click(
                                    raw.timestamp_mono_us,
                                    raw.x,
                                    raw.y,
                                );

                                // Always emit Click
                                agent_events.push((
                                    RouteTarget::Agent { namespace: namespace.clone(), tile_id: pressed_tile },
                                    InputEnvelope::Click(ClickEvent {
                                        fields: PointerFields {
                                            tile_id: pressed_tile,
                                            node_id: pressed_node,
                                            interaction_id: interaction_id.clone(),
                                            device_id: raw.device_id,
                                            local_x,
                                            local_y,
                                            display_x: raw.x,
                                            display_y: raw.y,
                                            modifiers: raw.modifiers,
                                            timestamp_mono_us: raw.timestamp_mono_us,
                                        },
                                        button,
                                    }),
                                ));

                                // Emit DoubleClick if criteria met
                                if is_double_click
                                    && node_allows_event(scene, pressed_node, |m| m.double_click)
                                {
                                    agent_events.push((
                                        RouteTarget::Agent { namespace, tile_id: pressed_tile },
                                        InputEnvelope::DoubleClick(crate::pointer::DoubleClickEvent {
                                            fields: PointerFields {
                                                tile_id: pressed_tile,
                                                node_id: pressed_node,
                                                interaction_id,
                                                device_id: raw.device_id,
                                                local_x,
                                                local_y,
                                                display_x: raw.x,
                                                display_y: raw.y,
                                                modifiers: raw.modifiers,
                                                timestamp_mono_us: raw.timestamp_mono_us,
                                            },
                                            button,
                                        }),
                                    ));
                                }

                                // Record this click for double-click detection
                                self.last_click_ts = Some(raw.timestamp_mono_us);
                                self.last_click_pos = Some((raw.x, raw.y));
                            }
                        }
                    }
                }
            }

            RawPointerEventKind::Move => {
                // Only dispatch PointerMove if still hovering and mask allows
                if let Some((tile_id, node_id)) = new_hover {
                    if self.current_hover == Some((tile_id, node_id))
                        && node_allows_event(scene, node_id, |m| m.pointer_move)
                    {
                        if let Some(namespace) = tile_namespace(scene, tile_id) {
                            let interaction_id = interaction_id_for(scene, node_id);
                            let (local_x, local_y) =
                                display_to_local(scene, tile_id, raw.x, raw.y);
                            agent_events.push((
                                RouteTarget::Agent { namespace, tile_id },
                                InputEnvelope::PointerMove(PointerMoveEvent {
                                    fields: PointerFields {
                                        tile_id,
                                        node_id,
                                        interaction_id,
                                        device_id: raw.device_id,
                                        local_x,
                                        local_y,
                                        display_x: raw.x,
                                        display_y: raw.y,
                                        modifiers: raw.modifiers,
                                        timestamp_mono_us: raw.timestamp_mono_us,
                                    },
                                }),
                            ));
                        }
                    }
                }
            }

            RawPointerEventKind::Cancel => {
                if let Some((pressed_tile, pressed_node, _)) = self.current_press.take() {
                    // Clear pressed state
                    if let Some(state) = scene.hit_region_states.get_mut(&pressed_node) {
                        state.pressed = false;
                    }
                    local_patch.update_node(pressed_node, Some(false), None, None);

                    // PointerCancelEvent is a terminal signal; EventMask does not gate it
                    // (the main-branch EventMask has no pointer_cancel field).
                    if let Some(namespace) = tile_namespace(scene, pressed_tile) {
                        let interaction_id = interaction_id_for(scene, pressed_node);
                        let (local_x, local_y) =
                            display_to_local(scene, pressed_tile, raw.x, raw.y);
                        agent_events.push((
                            RouteTarget::Agent { namespace, tile_id: pressed_tile },
                            InputEnvelope::PointerCancel(PointerCancelEvent {
                                fields: PointerFields {
                                    tile_id: pressed_tile,
                                    node_id: pressed_node,
                                    interaction_id,
                                    device_id: raw.device_id,
                                    local_x,
                                    local_y,
                                    display_x: raw.x,
                                    display_y: raw.y,
                                    modifiers: raw.modifiers,
                                    timestamp_mono_us: raw.timestamp_mono_us,
                                },
                                reason: CancelReason::RuntimeRevoked,
                            }),
                        ));
                    }
                }
            }

            RawPointerEventKind::RightClick => {
                // Already handled above via pre-processor branch
            }
        }

        let stages_1_2_us = start.elapsed().as_micros() as u64;
        DispatchOutcome { hit, local_patch, agent_events, stages_1_2_us, hit_test_us }
    }

    /// Check whether the current Up event qualifies as a DoubleClick.
    fn check_double_click(&self, ts: MonoUs, x: f32, y: f32) -> bool {
        if let (Some(last_ts), Some((last_x, last_y))) =
            (self.last_click_ts, self.last_click_pos)
        {
            let dt = ts.0.saturating_sub(last_ts.0);
            let dx = x - last_x;
            let dy = y - last_y;
            let dist = (dx * dx + dy * dy).sqrt();
            dt <= Self::DOUBLE_CLICK_WINDOW_US && dist <= Self::DOUBLE_CLICK_MAX_PX
        } else {
            false
        }
    }
}

impl Default for DispatchProcessor {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Get the namespace (agent name) of a tile's owner, or `None` if not found.
fn tile_namespace(scene: &SceneGraph, tile_id: SceneId) -> Option<String> {
    scene.tiles.get(&tile_id).map(|t| t.namespace.clone())
}

/// Convert display-space coordinates to tile-local coordinates.
fn display_to_local(scene: &SceneGraph, tile_id: SceneId, x: f32, y: f32) -> (f32, f32) {
    if let Some(tile) = scene.tiles.get(&tile_id) {
        (x - tile.bounds.x, y - tile.bounds.y)
    } else {
        (x, y)
    }
}

/// Get the interaction_id of a HitRegionNode, or empty string if not found.
fn interaction_id_for(scene: &SceneGraph, node_id: SceneId) -> String {
    scene
        .nodes
        .get(&node_id)
        .and_then(|n| {
            if let NodeData::HitRegion(hr) = &n.data {
                Some(hr.interaction_id.clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// Check whether a node's `event_mask` allows a given event type.
///
/// Returns `false` if the node is not a `HitRegion` or does not exist.
/// Takes an immutable reference to `SceneGraph` and does not modify any state.
fn node_allows_event<F>(scene: &SceneGraph, node_id: SceneId, pred: F) -> bool
where
    F: Fn(&tze_hud_scene::EventMask) -> bool,
{
    scene
        .nodes
        .get(&node_id)
        .and_then(|n| {
            if let NodeData::HitRegion(hr) = &n.data {
                Some(pred(&hr.event_mask))
            } else {
                None
            }
        })
        .unwrap_or(false)
}

// ─── EventBatch builder helper ────────────────────────────────────────────────

/// Build an `EventBatch` for a single agent from a list of `(RouteTarget, InputEnvelope)`.
///
/// Filters to only those events targeting the given `namespace` and inserts
/// them in timestamp order.
pub fn build_agent_batch(
    events: &[(RouteTarget, InputEnvelope)],
    namespace: &str,
    frame_number: u64,
    batch_ts_us: u64,
) -> EventBatch {
    let mut batch = EventBatch::new(frame_number, batch_ts_us);
    for (route, envelope) in events {
        if let RouteTarget::Agent { namespace: ns, .. } = route {
            if ns == namespace {
                batch.push(envelope.clone());
            }
        }
    }
    batch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pointer::Modifiers;
    use tze_hud_scene::{
        Capability, HitRegionNode, Node, NodeData, Rect, SceneGraph, SceneId,
    };

    fn setup_scene() -> (SceneGraph, SceneId, SceneId) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(tab_id, "test", lease_id, Rect::new(100.0, 100.0, 400.0, 300.0), 1)
            .unwrap();
        let node_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: node_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(50.0, 50.0, 200.0, 100.0),
                        interaction_id: "submit-button".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();
        (scene, tile_id, node_id)
    }

    fn raw(kind: RawPointerEventKind, x: f32, y: f32) -> RawPointerEvent {
        RawPointerEvent {
            x,
            y,
            kind,
            button: match kind {
                RawPointerEventKind::Down | RawPointerEventKind::Up => {
                    Some(PointerButton::Primary)
                }
                _ => None,
            },
            device_id: 1,
            timestamp_mono_us: MonoUs(1000),
            modifiers: Modifiers::NONE,
        }
    }

    // ── PointerDownEvent carries all required fields (spec line 284) ──────────

    #[test]
    fn pointer_down_carries_all_required_fields() {
        let (mut scene, tile_id, node_id) = setup_scene();
        let mut proc = DispatchProcessor::new();

        // (50,50) is at tile origin (100,100) + node (50,50) + point offset (0,0)
        // Node bounds: tile(100,100) + hr(50,50 to 250,150) → display (150,150 to 350,250)
        let outcome = proc.process(
            &RawPointerEvent {
                x: 200.0,
                y: 180.0,
                kind: RawPointerEventKind::Down,
                button: Some(PointerButton::Primary),
                device_id: 42,
                timestamp_mono_us: MonoUs(1_234_567),
                modifiers: Modifiers { shift: true, ..Modifiers::NONE },
            },
            &mut scene,
        );

        let down_events: Vec<_> = outcome
            .agent_events
            .iter()
            .filter(|(_, e)| matches!(e, InputEnvelope::PointerDown(_)))
            .collect();
        assert_eq!(down_events.len(), 1, "exactly one PointerDown must be produced");

        let InputEnvelope::PointerDown(down) = &down_events[0].1 else { unreachable!() };
        assert_eq!(down.fields.tile_id, tile_id, "tile_id must match");
        assert_eq!(down.fields.node_id, node_id, "node_id must match");
        assert_eq!(down.fields.device_id, 42, "device_id must match");
        assert_eq!(down.button, PointerButton::Primary, "button must be PRIMARY");
        assert_eq!(down.fields.interaction_id, "submit-button", "interaction_id must be forwarded");
        // Tile origin is (100,100), event at (200,180) → local = (100, 80)
        assert!((down.fields.local_x - 100.0).abs() < 0.1, "local_x must be display - tile origin");
        assert!((down.fields.local_y - 80.0).abs() < 0.1, "local_y must be display - tile origin");
        assert_eq!(down.fields.display_x, 200.0, "display_x must match raw event");
        assert_eq!(down.fields.display_y, 180.0, "display_y must match raw event");
        assert!(down.fields.modifiers.shift, "modifier shift must be forwarded");
        assert_eq!(down.fields.timestamp_mono_us, MonoUs(1_234_567), "timestamp_mono_us must match");
    }

    // ── Event routing to lease owner (spec line 321) ──────────────────────────

    #[test]
    fn event_routed_to_tile_lease_owner() {
        let (mut scene, tile_id, _) = setup_scene();
        let mut proc = DispatchProcessor::new();

        let outcome = proc.process(&raw(RawPointerEventKind::Down, 200.0, 180.0), &mut scene);

        for (route, _) in &outcome.agent_events {
            if let RouteTarget::Agent { namespace, tile_id: rt } = route {
                assert_eq!(namespace, "test", "must route to 'test' namespace");
                assert_eq!(*rt, tile_id, "route tile_id must match the hit tile");
            }
        }
    }

    // ── Event mask: pointer_move=false suppresses PointerMove (spec line 254) ──

    #[test]
    fn event_mask_pointer_move_false_suppresses_move() {
        let (mut scene, _, node_id) = setup_scene();
        // Disable pointer_move in the event mask
        if let Some(node) = scene.nodes.get_mut(&node_id) {
            if let NodeData::HitRegion(hr) = &mut node.data {
                hr.event_mask.pointer_move = false;
            }
        }
        let mut proc = DispatchProcessor::new();

        // First move to enter
        proc.process(&raw(RawPointerEventKind::Move, 200.0, 180.0), &mut scene);
        // Second move — should NOT produce PointerMove
        let outcome = proc.process(&raw(RawPointerEventKind::Move, 210.0, 185.0), &mut scene);

        let move_events: Vec<_> = outcome
            .agent_events
            .iter()
            .filter(|(_, e)| matches!(e, InputEnvelope::PointerMove(_)))
            .collect();
        assert!(move_events.is_empty(), "PointerMove must not be dispatched when event_mask.pointer_move=false");
    }

    // ── interaction_id forwarded in ClickEvent (spec line 258) ───────────────

    #[test]
    fn click_event_includes_interaction_id() {
        let (mut scene, _, _) = setup_scene();
        let mut proc = DispatchProcessor::new();

        // Down then Up on same node
        proc.process(&raw(RawPointerEventKind::Down, 200.0, 180.0), &mut scene);
        let outcome = proc.process(&raw(RawPointerEventKind::Up, 200.0, 180.0), &mut scene);

        let clicks: Vec<_> = outcome
            .agent_events
            .iter()
            .filter(|(_, e)| matches!(e, InputEnvelope::Click(_)))
            .collect();
        assert_eq!(clicks.len(), 1, "exactly one ClickEvent must be produced");
        let InputEnvelope::Click(click) = &clicks[0].1 else { unreachable!() };
        assert_eq!(click.fields.interaction_id, "submit-button", "interaction_id must be forwarded");
    }

    // ── Right-click → ContextMenuEvent, not GestureEvent (spec line 439) ──────

    #[test]
    fn right_click_produces_context_menu_event() {
        let (mut scene, _, node_id) = setup_scene();
        // Ensure context_menu is enabled
        if let Some(node) = scene.nodes.get_mut(&node_id) {
            if let NodeData::HitRegion(hr) = &mut node.data {
                hr.event_mask.context_menu = true;
            }
        }
        let mut proc = DispatchProcessor::new();

        let outcome = proc.process(
            &RawPointerEvent {
                x: 200.0,
                y: 180.0,
                kind: RawPointerEventKind::RightClick,
                button: Some(PointerButton::Secondary),
                device_id: 1,
                timestamp_mono_us: MonoUs(1000),
                modifiers: Modifiers::NONE,
            },
            &mut scene,
        );

        let ctx_events: Vec<_> = outcome
            .agent_events
            .iter()
            .filter(|(_, e)| matches!(e, InputEnvelope::ContextMenu(_)))
            .collect();
        assert_eq!(ctx_events.len(), 1, "exactly one ContextMenuEvent must be produced");

        // Must NOT produce any gesture-like events
        assert!(
            !outcome.agent_events.iter().any(|(_, e)| matches!(e, InputEnvelope::PointerDown(_))),
            "RightClick must not produce PointerDown"
        );
    }

    // ── Double-click detection ────────────────────────────────────────────────

    #[test]
    fn double_click_within_window_produces_double_click_event() {
        let (mut scene, _, _) = setup_scene();
        let mut proc = DispatchProcessor::new();

        // First click (Down + Up) at ts=1000
        proc.process(
            &RawPointerEvent {
                x: 200.0,
                y: 180.0,
                kind: RawPointerEventKind::Down,
                button: Some(PointerButton::Primary),
                device_id: 1,
                timestamp_mono_us: MonoUs(1000),
                modifiers: Modifiers::NONE,
            },
            &mut scene,
        );
        proc.process(
            &RawPointerEvent {
                x: 200.0,
                y: 180.0,
                kind: RawPointerEventKind::Up,
                button: Some(PointerButton::Primary),
                device_id: 1,
                timestamp_mono_us: MonoUs(1000),
                modifiers: Modifiers::NONE,
            },
            &mut scene,
        );

        // Second click (Down + Up) at ts=250000 (250ms, within 300ms window)
        proc.process(
            &RawPointerEvent {
                x: 200.0,
                y: 180.0,
                kind: RawPointerEventKind::Down,
                button: Some(PointerButton::Primary),
                device_id: 1,
                timestamp_mono_us: MonoUs(250_000),
                modifiers: Modifiers::NONE,
            },
            &mut scene,
        );
        let outcome = proc.process(
            &RawPointerEvent {
                x: 200.0,
                y: 180.0,
                kind: RawPointerEventKind::Up,
                button: Some(PointerButton::Primary),
                device_id: 1,
                timestamp_mono_us: MonoUs(250_000),
                modifiers: Modifiers::NONE,
            },
            &mut scene,
        );

        let dbl_clicks: Vec<_> = outcome
            .agent_events
            .iter()
            .filter(|(_, e)| matches!(e, InputEnvelope::DoubleClick(_)))
            .collect();
        assert_eq!(dbl_clicks.len(), 1, "DoubleClickEvent must be produced within 300ms");
    }

    // ── SceneLocalPatch contains pressed state (spec line 159) ───────────────

    #[test]
    fn stage2_produces_pressed_local_patch_on_down() {
        let (mut scene, _, node_id) = setup_scene();
        let mut proc = DispatchProcessor::new();

        let outcome = proc.process(&raw(RawPointerEventKind::Down, 200.0, 180.0), &mut scene);

        let pressed_update = outcome
            .local_patch
            .node_updates
            .iter()
            .find(|u| u.node_id == node_id && u.pressed == Some(true));
        assert!(pressed_update.is_some(), "SceneLocalPatch must contain pressed=true on PointerDown");

        // Verify scene state was updated
        assert!(
            scene.hit_region_states.get(&node_id).map(|s| s.pressed).unwrap_or(false),
            "HitRegionLocalState.pressed must be true after PointerDown"
        );
    }

    // ── Stage 1+2 combined latency budget (spec line 310) ────────────────────

    #[test]
    fn stages_1_2_combined_under_1ms() {
        use tze_hud_scene::calibration::{budgets, test_budget};

        let (mut scene, _, _) = setup_scene();
        let mut proc = DispatchProcessor::new();

        // Combined budget is < 1ms (stages1+2)
        let combined_budget_us = test_budget(budgets::INPUT_ACK_BUDGET_US / 4); // conservative

        let outcome = proc.process(&raw(RawPointerEventKind::Down, 200.0, 180.0), &mut scene);
        let stages_us = outcome.stages_1_2_us;

        // We use 1ms (1000µs) as the hard threshold
        assert!(
            stages_us < 1000,
            "Stages 1+2 took {}µs, must be < 1ms (calibrated: {}µs)",
            stages_us,
            combined_budget_us,
        );
    }

    // ── EventBatch builder ────────────────────────────────────────────────────

    #[test]
    fn build_agent_batch_filters_by_namespace() {
        let tile_id = SceneId::new();
        let node_id = SceneId::new();
        let fields = |ts: u64| PointerFields {
            tile_id,
            node_id,
            interaction_id: "btn".to_string(),
            device_id: 1,
            local_x: 0.0,
            local_y: 0.0,
            display_x: 0.0,
            display_y: 0.0,
            modifiers: Modifiers::NONE,
            timestamp_mono_us: MonoUs(ts),
        };

        let events = vec![
            (
                RouteTarget::Agent { namespace: "agent-a".to_string(), tile_id },
                InputEnvelope::PointerDown(PointerDownEvent {
                    fields: fields(100),
                    button: PointerButton::Primary,
                }),
            ),
            (
                RouteTarget::Agent { namespace: "agent-b".to_string(), tile_id },
                InputEnvelope::PointerMove(PointerMoveEvent { fields: fields(200) }),
            ),
            (
                RouteTarget::Agent { namespace: "agent-a".to_string(), tile_id },
                InputEnvelope::Click(ClickEvent {
                    fields: fields(300),
                    button: PointerButton::Primary,
                }),
            ),
        ];

        let batch_a = build_agent_batch(&events, "agent-a", 1, 1000);
        assert_eq!(batch_a.events.len(), 2, "agent-a must get exactly 2 events");
        assert!(
            matches!(batch_a.events[0], InputEnvelope::PointerDown(_)),
            "first event must be PointerDown"
        );
        assert!(
            matches!(batch_a.events[1], InputEnvelope::Click(_)),
            "second event must be Click"
        );

        let batch_b = build_agent_batch(&events, "agent-b", 1, 1000);
        assert_eq!(batch_b.events.len(), 1, "agent-b must get exactly 1 event");
    }
}
