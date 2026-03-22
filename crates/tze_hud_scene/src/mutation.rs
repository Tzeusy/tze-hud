//! Mutation batch operations for atomic scene changes.

use crate::types::*;
use crate::graph::SceneGraph;
use crate::validation::ValidationError;
use serde::{Deserialize, Serialize};

/// An atomic batch of scene mutations from an agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MutationBatch {
    pub batch_id: SceneId,
    pub agent_namespace: String,
    pub mutations: Vec<SceneMutation>,
}

/// Individual scene mutations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SceneMutation {
    CreateTab {
        name: String,
        display_order: u32,
    },
    SwitchActiveTab {
        tab_id: SceneId,
    },
    CreateTile {
        tab_id: SceneId,
        namespace: String,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
    },
    SetTileRoot {
        tile_id: SceneId,
        node: Node,
    },
    AddNode {
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
    },
    UpdateTileBounds {
        tile_id: SceneId,
        bounds: Rect,
    },
    DeleteTile {
        tile_id: SceneId,
    },
}

/// Result of applying a mutation batch.
#[derive(Clone, Debug)]
pub struct MutationResult {
    pub batch_id: SceneId,
    pub applied: bool,
    pub created_ids: Vec<SceneId>,
    pub error: Option<ValidationError>,
}

impl SceneGraph {
    /// Apply a mutation batch atomically. All-or-nothing.
    pub fn apply_batch(&mut self, batch: &MutationBatch) -> MutationResult {
        // Clone the scene for rollback on failure
        let snapshot = self.clone();
        let mut created_ids = Vec::new();

        for mutation in &batch.mutations {
            match self.apply_single_mutation(mutation, &batch.agent_namespace) {
                Ok(ids) => created_ids.extend(ids),
                Err(e) => {
                    // Rollback
                    *self = snapshot;
                    return MutationResult {
                        batch_id: batch.batch_id,
                        applied: false,
                        created_ids: vec![],
                        error: Some(e),
                    };
                }
            }
        }

        MutationResult {
            batch_id: batch.batch_id,
            applied: true,
            created_ids,
            error: None,
        }
    }

    fn apply_single_mutation(
        &mut self,
        mutation: &SceneMutation,
        _namespace: &str,
    ) -> Result<Vec<SceneId>, ValidationError> {
        match mutation {
            SceneMutation::CreateTab { name, display_order } => {
                let id = self.create_tab(name, *display_order)?;
                Ok(vec![id])
            }
            SceneMutation::SwitchActiveTab { tab_id } => {
                self.switch_active_tab(*tab_id)?;
                Ok(vec![])
            }
            SceneMutation::CreateTile {
                tab_id,
                namespace,
                lease_id,
                bounds,
                z_order,
            } => {
                let id = self.create_tile(*tab_id, namespace, *lease_id, *bounds, *z_order)?;
                Ok(vec![id])
            }
            SceneMutation::SetTileRoot { tile_id, node } => {
                self.set_tile_root(*tile_id, node.clone())?;
                Ok(vec![node.id])
            }
            SceneMutation::AddNode {
                tile_id,
                parent_id,
                node,
            } => {
                self.add_node_to_tile(*tile_id, *parent_id, node.clone())?;
                Ok(vec![node.id])
            }
            SceneMutation::UpdateTileBounds { tile_id, bounds } => {
                let tile = self
                    .tiles
                    .get_mut(tile_id)
                    .ok_or(ValidationError::TileNotFound { id: *tile_id })?;
                if bounds.width <= 0.0 || bounds.height <= 0.0 {
                    return Err(ValidationError::InvalidField {
                        field: "bounds".into(),
                        reason: "width and height must be > 0".into(),
                    });
                }
                tile.bounds = *bounds;
                self.version += 1;
                Ok(vec![])
            }
            SceneMutation::DeleteTile { tile_id } => {
                if !self.tiles.contains_key(tile_id) {
                    return Err(ValidationError::TileNotFound { id: *tile_id });
                }
                self.remove_tile_and_nodes(*tile_id);
                self.version += 1;
                Ok(vec![])
            }
        }
    }

    // remove_tile_and_nodes and remove_node_tree are defined in graph.rs as pub(crate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mutation_batch_apply() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![
                SceneMutation::CreateTile {
                    tab_id,
                    namespace: "agent".to_string(),
                    lease_id,
                    bounds: Rect::new(10.0, 10.0, 200.0, 150.0),
                    z_order: 1,
                },
            ],
        };

        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert_eq!(result.created_ids.len(), 1);
        assert_eq!(scene.tile_count(), 1);
    }

    #[test]
    fn test_mutation_batch_rollback_on_failure() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![
                SceneMutation::CreateTile {
                    tab_id,
                    namespace: "agent".to_string(),
                    lease_id,
                    bounds: Rect::new(10.0, 10.0, 200.0, 150.0),
                    z_order: 1,
                },
                // This should fail — invalid bounds
                SceneMutation::CreateTile {
                    tab_id,
                    namespace: "agent".to_string(),
                    lease_id,
                    bounds: Rect::new(10.0, 10.0, 0.0, 0.0), // invalid
                    z_order: 2,
                },
            ],
        };

        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        // Entire batch rolled back — no tiles created
        assert_eq!(scene.tile_count(), 0);
    }
}
