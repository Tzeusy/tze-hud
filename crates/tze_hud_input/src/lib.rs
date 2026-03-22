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
}

/// The input processor. Tracks state across events for local feedback.
pub struct InputProcessor {
    /// Currently hovered node.
    current_hover: Option<SceneId>,
    /// Currently pressed node.
    current_press: Option<SceneId>,
}

impl InputProcessor {
    pub fn new() -> Self {
        Self {
            current_hover: None,
            current_press: None,
        }
    }

    /// Process a pointer event against the scene graph.
    /// Updates hit-region local state for immediate visual feedback.
    /// Returns the result including timing measurements.
    pub fn process(&mut self, event: &PointerEvent, scene: &mut SceneGraph) -> InputResult {
        let start = Instant::now();

        // Hit test
        let hit_start = Instant::now();
        let hit = scene.hit_test(event.x, event.y);
        let hit_test_us = hit_start.elapsed().as_micros() as u64;

        let mut interaction_id = None;
        let mut activated = false;

        // Determine which node (if any) is a hit region
        let hit_node_id = hit.and_then(|(_, node_id)| {
            scene.nodes.get(&node_id).and_then(|node| {
                if let NodeData::HitRegion(hr) = &node.data {
                    interaction_id = Some(hr.interaction_id.clone());
                    Some(node_id)
                } else {
                    None
                }
            })
        });

        // Update hover state
        if self.current_hover != hit_node_id {
            // Un-hover the old node
            if let Some(old_id) = self.current_hover {
                if let Some(state) = scene.hit_region_states.get_mut(&old_id) {
                    state.hovered = false;
                }
            }
            // Hover the new node
            if let Some(new_id) = hit_node_id {
                if let Some(state) = scene.hit_region_states.get_mut(&new_id) {
                    state.hovered = true;
                }
            }
            self.current_hover = hit_node_id;
        }

        // Handle press/release
        match event.kind {
            PointerEventKind::Down => {
                if let Some(node_id) = hit_node_id {
                    if let Some(state) = scene.hit_region_states.get_mut(&node_id) {
                        state.pressed = true;
                    }
                    self.current_press = Some(node_id);
                }
            }
            PointerEventKind::Up => {
                if let Some(pressed_id) = self.current_press.take() {
                    if let Some(state) = scene.hit_region_states.get_mut(&pressed_id) {
                        state.pressed = false;
                    }
                    // Activation: press and release on the same node
                    if hit_node_id == Some(pressed_id) {
                        activated = true;
                    }
                }
            }
            PointerEventKind::Move => {}
        }

        let local_ack_us = start.elapsed().as_micros() as u64;

        InputResult {
            hit,
            interaction_id,
            activated,
            local_ack_us,
            hit_test_us,
        }
    }
}

impl Default for InputProcessor {
    fn default() -> Self {
        Self::new()
    }
}

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
}
