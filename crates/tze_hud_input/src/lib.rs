//! # tze_hud_input
//!
//! Input pipeline for tze_hud. Processes pointer events, performs hit-testing,
//! updates local feedback state (hover/pressed), and dispatches events to agents.
//! Local feedback happens synchronously in < 4ms — no agent roundtrip.

use tze_hud_scene::{SceneId, NodeData};
use tze_hud_scene::graph::SceneGraph;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Raw pointer input event from the OS.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PointerEvent {
    pub x: f32,
    pub y: f32,
    pub kind: PointerEventKind,
    /// Monotonic timestamp (microseconds since process start).
    #[serde(skip)]
    pub timestamp: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerEventKind {
    Move,
    Down,
    Up,
}

/// Result of processing a pointer event — what changed locally.
#[derive(Clone, Debug)]
pub struct InputResult {
    /// The tile and node that were hit (if any).
    pub hit: Option<(SceneId, SceneId)>,
    /// The interaction_id of the hit region (if a HitRegionNode was hit).
    pub interaction_id: Option<String>,
    /// Whether this was an activation (press then release on the same hit region).
    pub activated: bool,
    /// Time taken for local acknowledgement (microseconds).
    pub local_ack_us: u64,
    /// Time taken for hit-test (microseconds).
    pub hit_test_us: u64,
    /// Agent dispatch descriptor, if an event should be forwarded to an agent.
    pub dispatch: Option<AgentDispatch>,
}

/// Information needed to dispatch this input event to the owning agent.
///
/// Callers (e.g. the runtime kernel) use this to call into the protocol layer
/// without the input crate needing a direct dependency on tze_hud_protocol.
#[derive(Clone, Debug)]
pub struct AgentDispatch {
    /// Namespace (agent name) of the tile owner.
    pub namespace: String,
    pub tile_id: SceneId,
    pub node_id: SceneId,
    pub interaction_id: String,
    /// Pointer position in tile-local coordinates.
    pub local_x: f32,
    pub local_y: f32,
    /// Pointer position in display-space coordinates.
    pub display_x: f32,
    pub display_y: f32,
    pub kind: AgentDispatchKind,
}

/// Which type of input event to deliver to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentDispatchKind {
    PointerMove,
    PointerDown,
    PointerUp,
    PointerEnter,
    PointerLeave,
    /// Activation: press + release on the same hit region.
    Activated,
}

/// The input processor. Tracks state across events for local feedback.
pub struct InputProcessor {
    /// Currently hovered node.
    current_hover: Option<(SceneId, SceneId)>, // (tile_id, node_id)
    /// Currently pressed node.
    current_press: Option<(SceneId, SceneId)>, // (tile_id, node_id)
}

impl InputProcessor {
    pub fn new() -> Self {
        Self {
            current_hover: None,
            current_press: None,
        }
    }

    /// Process a pointer event against the scene graph.
    ///
    /// Updates hit-region local state for immediate visual feedback.
    /// Returns the result including timing measurements and an optional
    /// `AgentDispatch` descriptor for forwarding the event to the owning agent.
    pub fn process(&mut self, event: &PointerEvent, scene: &mut SceneGraph) -> InputResult {
        let start = Instant::now();

        // ── Stage 2: Hit test ─────────────────────────────────────────────
        let hit_start = Instant::now();
        let hit = scene.hit_test(event.x, event.y);
        let hit_test_us = hit_start.elapsed().as_micros() as u64;

        let mut interaction_id: Option<String> = None;
        let mut activated = false;
        let mut dispatch: Option<AgentDispatch> = None;

        // Determine the hit node (must be a HitRegionNode to produce interaction_id)
        let hit_node_id: Option<SceneId> = hit.and_then(|(_, node_id)| {
            scene.nodes.get(&node_id).and_then(|node| {
                if let NodeData::HitRegion(hr) = &node.data {
                    interaction_id = Some(hr.interaction_id.clone());
                    Some(node_id)
                } else {
                    None
                }
            })
        });

        let hit_tile_id: Option<SceneId> = hit.map(|(tile_id, _)| tile_id);

        // ── Stage 2: Update hover state ───────────────────────────────────
        let prev_hover_node = self.current_hover.map(|(_, n)| n);
        let new_hover_node = hit_node_id;

        if prev_hover_node != new_hover_node {
            // Un-hover the old node
            if let Some(old_id) = prev_hover_node {
                if let Some(state) = scene.hit_region_states.get_mut(&old_id) {
                    state.hovered = false;
                }
                // Dispatch pointer_leave to previous owning agent
                if let Some((old_tile_id, _)) = self.current_hover {
                    if let Some(namespace) = tile_namespace(scene, old_tile_id) {
                        let leave_interaction_id = scene
                            .nodes
                            .get(&old_id)
                            .and_then(|n| {
                                if let NodeData::HitRegion(hr) = &n.data {
                                    Some(hr.interaction_id.clone())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default();
                        dispatch = Some(AgentDispatch {
                            namespace,
                            tile_id: old_tile_id,
                            node_id: old_id,
                            interaction_id: leave_interaction_id,
                            local_x: 0.0,
                            local_y: 0.0,
                            display_x: event.x,
                            display_y: event.y,
                            kind: AgentDispatchKind::PointerLeave,
                        });
                    }
                }
            }
            // Hover the new node
            if let Some(new_id) = new_hover_node {
                if let Some(state) = scene.hit_region_states.get_mut(&new_id) {
                    state.hovered = true;
                }
                // Dispatch pointer_enter to new owning agent (overwrites leave above —
                // enter takes priority; the caller can queue both if needed)
                if let Some(tile_id) = hit_tile_id {
                    if let Some(namespace) = tile_namespace(scene, tile_id) {
                        let (local_x, local_y) = display_to_local(scene, tile_id, event.x, event.y);
                        dispatch = Some(AgentDispatch {
                            namespace,
                            tile_id,
                            node_id: new_id,
                            interaction_id: interaction_id.clone().unwrap_or_default(),
                            local_x,
                            local_y,
                            display_x: event.x,
                            display_y: event.y,
                            kind: AgentDispatchKind::PointerEnter,
                        });
                    }
                }
            }
            self.current_hover = hit_tile_id.and_then(|t| hit_node_id.map(|n| (t, n)));
        }

        // ── Stage 2: Handle press/release ─────────────────────────────────
        match event.kind {
            PointerEventKind::Down => {
                if let (Some(tile_id), Some(node_id)) = (hit_tile_id, hit_node_id) {
                    if let Some(state) = scene.hit_region_states.get_mut(&node_id) {
                        state.pressed = true;
                    }
                    self.current_press = Some((tile_id, node_id));
                    if let Some(namespace) = tile_namespace(scene, tile_id) {
                        let (local_x, local_y) = display_to_local(scene, tile_id, event.x, event.y);
                        dispatch = Some(AgentDispatch {
                            namespace,
                            tile_id,
                            node_id,
                            interaction_id: interaction_id.clone().unwrap_or_default(),
                            local_x,
                            local_y,
                            display_x: event.x,
                            display_y: event.y,
                            kind: AgentDispatchKind::PointerDown,
                        });
                    }
                }
            }
            PointerEventKind::Up => {
                if let Some((pressed_tile_id, pressed_node_id)) = self.current_press.take() {
                    if let Some(state) = scene.hit_region_states.get_mut(&pressed_node_id) {
                        state.pressed = false;
                    }
                    // Activation: press and release on the same node
                    if hit_node_id == Some(pressed_node_id) {
                        activated = true;
                        if let Some(namespace) = tile_namespace(scene, pressed_tile_id) {
                            let (local_x, local_y) =
                                display_to_local(scene, pressed_tile_id, event.x, event.y);
                            dispatch = Some(AgentDispatch {
                                namespace,
                                tile_id: pressed_tile_id,
                                node_id: pressed_node_id,
                                interaction_id: interaction_id.clone().unwrap_or_default(),
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                kind: AgentDispatchKind::Activated,
                            });
                        }
                    } else {
                        // Released outside the pressed node — dispatch pointer_up
                        if let Some(namespace) = tile_namespace(scene, pressed_tile_id) {
                            let (local_x, local_y) =
                                display_to_local(scene, pressed_tile_id, event.x, event.y);
                            let up_interaction_id = scene
                                .nodes
                                .get(&pressed_node_id)
                                .and_then(|n| {
                                    if let NodeData::HitRegion(hr) = &n.data {
                                        Some(hr.interaction_id.clone())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_default();
                            dispatch = Some(AgentDispatch {
                                namespace,
                                tile_id: pressed_tile_id,
                                node_id: pressed_node_id,
                                interaction_id: up_interaction_id,
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                kind: AgentDispatchKind::PointerUp,
                            });
                        }
                    }
                }
            }
            PointerEventKind::Move => {
                // If hovering, dispatch pointer_move (overrides enter/leave set above)
                if let (Some(tile_id), Some(node_id)) = (hit_tile_id, hit_node_id) {
                    if prev_hover_node == new_hover_node {
                        // Already hovering this node — plain move
                        if let Some(namespace) = tile_namespace(scene, tile_id) {
                            let (local_x, local_y) =
                                display_to_local(scene, tile_id, event.x, event.y);
                            dispatch = Some(AgentDispatch {
                                namespace,
                                tile_id,
                                node_id,
                                interaction_id: interaction_id.clone().unwrap_or_default(),
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                kind: AgentDispatchKind::PointerMove,
                            });
                        }
                    }
                    // If prev != new (handled above by enter/leave dispatch), no extra move
                }
            }
        }

        let local_ack_us = start.elapsed().as_micros() as u64;

        InputResult {
            hit,
            interaction_id,
            activated,
            local_ack_us,
            hit_test_us,
            dispatch,
        }
    }
}

impl Default for InputProcessor {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Get the namespace (agent name) of the tile owner, or None if tile not found.
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

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::*;

    fn setup_scene_with_hit_region() -> (SceneGraph, SceneId, SceneId) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        let tile_id = scene
            .create_tile(tab_id, "test", lease_id, Rect::new(100.0, 100.0, 400.0, 300.0), 1)
            .unwrap();

        let hr_node_id = SceneId::new();
        let hit_node = Node {
            id: hr_node_id,
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(50.0, 50.0, 200.0, 100.0),
                interaction_id: "test-button".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
            }),
        };
        scene.set_tile_root(tile_id, hit_node).unwrap();

        (scene, tile_id, hr_node_id)
    }

    #[test]
    fn test_hover_state_updates() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Move pointer over hit region (tile at 100,100; region at 50,50 within)
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(result.hit.is_some());
        assert_eq!(result.interaction_id, Some("test-button".to_string()));
        assert!(scene.hit_region_states[&hr_node_id].hovered);

        // Move pointer away
        let result = processor.process(
            &PointerEvent {
                x: 10.0,
                y: 10.0,
                kind: PointerEventKind::Move,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(result.hit.is_none());
        assert!(!scene.hit_region_states[&hr_node_id].hovered);
    }

    #[test]
    fn test_press_and_activate() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Press on hit region
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(scene.hit_region_states[&hr_node_id].pressed);
        assert!(!result.activated);

        // Release on hit region — should activate
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Up,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(!scene.hit_region_states[&hr_node_id].pressed);
        assert!(result.activated);
        assert_eq!(result.interaction_id, Some("test-button".to_string()));
    }

    #[test]
    fn test_local_ack_under_4ms() {
        let (mut scene, _, _) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                timestamp: None,
            },
            &mut scene,
        );

        // local_ack should be well under 4ms (4000 us) for a simple scene
        assert!(
            result.local_ack_us < 4000,
            "local_ack_us was {}us, budget is 4000us",
            result.local_ack_us
        );
        // hit_test should be under 100us
        assert!(
            result.hit_test_us < 100,
            "hit_test_us was {}us, budget is 100us",
            result.hit_test_us
        );
    }

    // ── New tests for AgentDispatch ─────────────────────────────────────

    #[test]
    fn test_dispatch_pointer_enter_on_first_hover() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // First move onto the hit region — should produce PointerEnter dispatch
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                timestamp: None,
            },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("expected AgentDispatch on enter");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerEnter);
        assert_eq!(dispatch.namespace, "test");
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert_eq!(dispatch.interaction_id, "test-button");
        // local coords: display(200,180) - tile_origin(100,100) = (100,80)
        assert!((dispatch.local_x - 100.0).abs() < 0.01);
        assert!((dispatch.local_y - 80.0).abs() < 0.01);
    }

    #[test]
    fn test_dispatch_pointer_leave_on_exit() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Enter
        processor.process(
            &PointerEvent { x: 200.0, y: 180.0, kind: PointerEventKind::Move, timestamp: None },
            &mut scene,
        );

        // Leave
        let result = processor.process(
            &PointerEvent { x: 10.0, y: 10.0, kind: PointerEventKind::Move, timestamp: None },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("expected AgentDispatch on leave");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerLeave);
        assert_eq!(dispatch.namespace, "test");
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert_eq!(dispatch.interaction_id, "test-button");
    }

    #[test]
    fn test_dispatch_pointer_down() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        let result = processor.process(
            &PointerEvent { x: 200.0, y: 180.0, kind: PointerEventKind::Down, timestamp: None },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("expected AgentDispatch on down");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerDown);
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
    }

    #[test]
    fn test_dispatch_activated_on_press_release() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Down
        processor.process(
            &PointerEvent { x: 200.0, y: 180.0, kind: PointerEventKind::Down, timestamp: None },
            &mut scene,
        );

        // Up on same node — Activated
        let result = processor.process(
            &PointerEvent { x: 200.0, y: 180.0, kind: PointerEventKind::Up, timestamp: None },
            &mut scene,
        );

        assert!(result.activated);
        let dispatch = result.dispatch.expect("expected AgentDispatch on activation");
        assert_eq!(dispatch.kind, AgentDispatchKind::Activated);
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert_eq!(dispatch.interaction_id, "test-button");
    }

    #[test]
    fn test_dispatch_pointer_up_outside_pressed_node() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Press inside hit region
        processor.process(
            &PointerEvent { x: 200.0, y: 180.0, kind: PointerEventKind::Down, timestamp: None },
            &mut scene,
        );

        // Release outside — PointerUp (not Activated)
        let result = processor.process(
            &PointerEvent { x: 10.0, y: 10.0, kind: PointerEventKind::Up, timestamp: None },
            &mut scene,
        );

        assert!(!result.activated);
        let dispatch = result.dispatch.expect("expected AgentDispatch on up-outside");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerUp);
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert_eq!(dispatch.interaction_id, "test-button");
    }

    #[test]
    fn test_no_dispatch_when_no_hit() {
        let (mut scene, _, _) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        let result = processor.process(
            &PointerEvent { x: 5.0, y: 5.0, kind: PointerEventKind::Move, timestamp: None },
            &mut scene,
        );

        assert!(result.hit.is_none());
        assert!(result.dispatch.is_none());
    }

    #[test]
    fn test_dispatch_move_while_hovering() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Enter
        processor.process(
            &PointerEvent { x: 200.0, y: 180.0, kind: PointerEventKind::Move, timestamp: None },
            &mut scene,
        );

        // Move within the same hit region — PointerMove
        let result = processor.process(
            &PointerEvent { x: 210.0, y: 185.0, kind: PointerEventKind::Move, timestamp: None },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("expected AgentDispatch on move");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerMove);
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert!((dispatch.local_x - 110.0).abs() < 0.01);
        assert!((dispatch.local_y - 85.0).abs() < 0.01);
    }
}
