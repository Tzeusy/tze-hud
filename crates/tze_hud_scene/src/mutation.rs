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
    /// Publish content to a zone.
    PublishToZone {
        zone_name: String,
        content: ZoneContent,
        publish_token: ZonePublishToken,
        /// For MergeByKey contention: the key under which content is stored.
        merge_key: Option<String>,
    },
    /// Clear all active publishes for a zone.
    ClearZone {
        zone_name: String,
        publish_token: ZonePublishToken,
    },
    /// Create a new sync group.
    CreateSyncGroup {
        /// Optional human-readable label (max 128 UTF-8 bytes).
        name: Option<String>,
        /// Namespace creating this group (typically the agent namespace).
        owner_namespace: String,
        /// Commit policy: AllOrDefer or AvailableMembers.
        commit_policy: SyncCommitPolicy,
        /// Max deferral frames before force-commit (AllOrDefer only).
        max_deferrals: u32,
    },
    /// Delete a sync group by ID. All member tiles are released automatically.
    DeleteSyncGroup {
        group_id: SceneId,
    },
    /// Add a tile to a sync group. Replaces any previous group membership.
    JoinSyncGroup {
        tile_id: SceneId,
        group_id: SceneId,
    },
    /// Remove a tile from its current sync group. No-op if not in a group.
    LeaveSyncGroup {
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
    /// True if the lease is at the soft budget warning threshold (80%).
    /// The batch was still applied, but the caller should notify the agent.
    pub budget_warning: bool,
}

impl SceneGraph {
    /// Apply a mutation batch atomically. All-or-nothing.
    ///
    /// Before applying mutations, checks:
    /// 1. That all referenced leases are in Active state (mutations allowed).
    /// 2. That the batch would not exceed any lease's resource budget.
    ///
    /// If budget soft limit (80%) is reached, the batch is still applied
    /// but the `budget_warning` flag is set on the result.
    pub fn apply_batch(&mut self, batch: &MutationBatch) -> MutationResult {
        // Check lease state for mutations that reference a lease
        for mutation in &batch.mutations {
            if let Some(lease_id) = Self::lease_id_for_mutation(mutation) {
                if let Some(lease) = self.leases.get(&lease_id) {
                    if !lease.is_mutations_allowed() {
                        return MutationResult {
                            batch_id: batch.batch_id,
                            applied: false,
                            created_ids: vec![],
                            error: Some(ValidationError::InvalidField {
                                field: "lease_state".into(),
                                reason: format!(
                                    "lease {} is in {:?} state; mutations require Active state",
                                    lease_id, lease.state,
                                ),
                            }),
                            budget_warning: false,
                        };
                    }
                }
            }
        }

        // Check budget for lease-bound mutations.
        // Find the lease_id from the batch (from CreateTile mutations or tile lookups).
        let lease_ids: Vec<SceneId> = batch
            .mutations
            .iter()
            .filter_map(|m| Self::lease_id_for_mutation(m))
            .collect();

        // Deduplicate
        let mut unique_lease_ids: Vec<SceneId> = lease_ids.clone();
        unique_lease_ids.sort();
        unique_lease_ids.dedup();

        let mut budget_warning = false;
        for lid in &unique_lease_ids {
            if let Err(budget_err) = self.check_budget(lid, batch) {
                return MutationResult {
                    batch_id: batch.batch_id,
                    applied: false,
                    created_ids: vec![],
                    error: Some(ValidationError::BudgetExceeded {
                        resource: format!("{}", budget_err),
                    }),
                    budget_warning: false,
                };
            }
            if self.is_lease_budget_warning(lid) {
                budget_warning = true;
            }
        }

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
                        budget_warning: false,
                    };
                }
            }
        }

        // Re-check budget warning after application (usage may have changed)
        for lid in &unique_lease_ids {
            if self.is_lease_budget_warning(lid) {
                budget_warning = true;
            }
        }

        MutationResult {
            batch_id: batch.batch_id,
            applied: true,
            created_ids,
            error: None,
            budget_warning,
        }
    }

    /// Extract the lease_id referenced by a mutation, if applicable.
    fn lease_id_for_mutation(mutation: &SceneMutation) -> Option<SceneId> {
        match mutation {
            SceneMutation::CreateTile { lease_id, .. } => Some(*lease_id),
            // For tile-modifying mutations, the lease is on the tile itself;
            // we would need the tile to look it up. Budget enforcement for
            // these is handled via the tile's lease_id at check_budget time.
            _ => None,
        }
    }

    fn apply_single_mutation(
        &mut self,
        mutation: &SceneMutation,
        namespace: &str,
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
            SceneMutation::PublishToZone {
                zone_name,
                content,
                publish_token: _publish_token, // token validated by the gRPC layer
                merge_key,
            } => {
                self.publish_to_zone(zone_name, content.clone(), namespace, merge_key.clone())?;
                Ok(vec![])
            }
            SceneMutation::ClearZone {
                zone_name,
                publish_token: _publish_token, // token validated by the gRPC layer
            } => {
                self.clear_zone(zone_name)?;
                Ok(vec![])
            }
            SceneMutation::CreateSyncGroup {
                name,
                owner_namespace,
                commit_policy,
                max_deferrals,
            } => {
                let id = self.create_sync_group(
                    name.clone(),
                    owner_namespace,
                    *commit_policy,
                    *max_deferrals,
                )?;
                Ok(vec![id])
            }
            SceneMutation::DeleteSyncGroup { group_id } => {
                self.delete_sync_group(*group_id)?;
                Ok(vec![])
            }
            SceneMutation::JoinSyncGroup { tile_id, group_id } => {
                self.join_sync_group(*tile_id, *group_id)?;
                Ok(vec![])
            }
            SceneMutation::LeaveSyncGroup { tile_id } => {
                self.leave_sync_group(*tile_id)?;
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

    #[test]
    fn test_mutation_create_and_delete_sync_group() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        // Create a sync group via mutation batch
        let create_batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::CreateSyncGroup {
                name: Some("my-group".to_string()),
                owner_namespace: "agent".to_string(),
                commit_policy: SyncCommitPolicy::AllOrDefer,
                max_deferrals: 3,
            }],
        };
        let result = scene.apply_batch(&create_batch);
        assert!(result.applied);
        assert_eq!(result.created_ids.len(), 1);
        let group_id = result.created_ids[0];
        assert_eq!(scene.sync_group_count(), 1);

        // Delete via mutation batch
        let delete_batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::DeleteSyncGroup { group_id }],
        };
        let result = scene.apply_batch(&delete_batch);
        assert!(result.applied);
        assert_eq!(scene.sync_group_count(), 0);
    }

    #[test]
    fn test_mutation_join_leave_sync_group() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        // Create group and join tile in one batch
        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::CreateSyncGroup {
                name: None,
                owner_namespace: "agent".to_string(),
                commit_policy: SyncCommitPolicy::AvailableMembers,
                max_deferrals: 0,
            }],
        };
        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        let group_id = result.created_ids[0];

        // Join tile to group
        let join_batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::JoinSyncGroup { tile_id, group_id }],
        };
        let result = scene.apply_batch(&join_batch);
        assert!(result.applied);
        assert_eq!(scene.tiles[&tile_id].sync_group, Some(group_id));
        assert!(scene.sync_groups[&group_id].members.contains(&tile_id));

        // Leave sync group
        let leave_batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::LeaveSyncGroup { tile_id }],
        };
        let result = scene.apply_batch(&leave_batch);
        assert!(result.applied);
        assert_eq!(scene.tiles[&tile_id].sync_group, None);
        assert!(!scene.sync_groups[&group_id].members.contains(&tile_id));
    }

    #[test]
    fn test_mutation_batch_rollback_on_bad_sync_group_join() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        let nonexistent_group = SceneId::new();

        // Batch that tries to join a non-existent group — should fail and rollback
        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::JoinSyncGroup {
                tile_id,
                group_id: nonexistent_group,
            }],
        };
        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        assert!(result.error.is_some());
        // Tile should remain without a sync group
        assert_eq!(scene.tiles[&tile_id].sync_group, None);
    }
}
