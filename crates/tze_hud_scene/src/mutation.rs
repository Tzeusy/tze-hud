//! Mutation batch operations for atomic scene changes.
//!
//! # Transaction Validation Pipeline (RFC 0001 §3.2, §3.3)
//!
//! Every [`MutationBatch`] passes through five ordered validation stages before
//! any mutation is applied to the live scene graph:
//!
//! | Stage | Check | Early exit |
//! |-------|-------|-----------|
//! | 1 | **Lease check** — lease exists and is Active | Yes |
//! | 2 | **Budget check** — batch fits within lease resource budget | Yes |
//! | 3 | **Bounds check** — geometry is valid (positive dimensions, finite values) | Per-mutation |
//! | 4 | **Type check** — mutation references are consistent; mutation type is legal | Per-mutation |
//! | 5 | **Invariant check** — post-mutation simulation: no cycles, no z-order conflicts | Post-apply |
//!
//! Stages run in this order. A failure at any stage produces a [`BatchRejected`] and
//! no mutations are applied (all-or-nothing).
//!
//! # Batch size limit (RFC 0001 §3.1)
//!
//! The maximum batch size is [`MAX_BATCH_SIZE`] (1000 mutations).  Batches
//! exceeding this limit are rejected with [`ValidationError::BatchSizeExceeded`]
//! before any stage runs.
//!
//! # Agent namespace (RFC 0001 §3.1)
//!
//! `agent_namespace` MUST be derived from the authenticated session context.
//! It is carried in the batch struct for in-process callers but the gRPC layer
//! MUST overwrite it from the authenticated principal before calling
//! [`SceneGraph::apply_batch`].

use crate::types::*;
use crate::graph::SceneGraph;
use crate::validation::{BatchRejected, ValidationError};
#[cfg(test)]
use crate::validation::ValidationErrorCode;
use serde::{Deserialize, Serialize};

/// Maximum number of mutations in a single batch (RFC 0001 §3.1).
pub const MAX_BATCH_SIZE: usize = 1_000;

/// An atomic batch of scene mutations from an agent.
///
/// # Wire contract
/// - `batch_id` is a UUIDv7 `SceneId`.
/// - `agent_namespace` is filled by the runtime from the authenticated session;
///   client-supplied values MUST be ignored by the gRPC layer.
/// - `mutations` are applied in order, atomically (all-or-nothing).
/// - Optional `timing_hints` carry `present_at_wall_us` and `expires_at_wall_us`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MutationBatch {
    pub batch_id: SceneId,
    pub agent_namespace: String,
    pub mutations: Vec<SceneMutation>,
    /// Optional timing hints from the agent.
    pub timing_hints: Option<TimingHints>,
    /// Lease ID for this batch. Required for lease/budget validation.
    /// If absent, lease validation is skipped (use with care in tests only).
    pub lease_id: Option<SceneId>,
}

/// Optional timing hints from the agent (RFC 0005).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimingHints {
    /// Wall-clock time (microseconds since epoch) at which the batch should be presented.
    pub present_at_wall_us: Option<u64>,
    /// Wall-clock time (microseconds since epoch) at which the batch expires.
    pub expires_at_wall_us: Option<u64>,
}

/// Individual scene mutations (v1 set per RFC 0001 §3.1).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SceneMutation {
    // ── Tab mutations ─────────────────────────────────────────────────────
    // NOTE: Tab mutations require the `manage_tabs` capability per RFC 0001
    // §2.2, §3.3. However, `SceneMutation` variants do not carry a `lease_id`
    // field, so capability enforcement at the batch-apply layer must be done
    // by the transport/session layer (gRPC handler) before calling
    // `apply_batch`. The scene graph's `create_tab_with_lease` /
    // `delete_tab_with_lease` / etc. checked variants are available for
    // direct callers that have a lease in scope.
    //
    // Tab mutations in `apply_single_mutation` call the unchecked graph
    // methods; the gRPC layer is responsible for verifying `manage_tabs`
    // before dispatching the batch.
    /// Create a new tab. RFC 0001 §2.2.
    CreateTab {
        name: String,
        display_order: u32,
    },
    /// Delete a tab and all its tiles. RFC 0001 §2.2.
    DeleteTab {
        tab_id: SceneId,
    },
    /// Rename a tab. RFC 0001 §2.2.
    RenameTab {
        tab_id: SceneId,
        new_name: String,
    },
    /// Change the display_order of a tab. RFC 0001 §2.2.
    ReorderTab {
        tab_id: SceneId,
        new_order: u32,
    },
    /// Switch the active tab. RFC 0001 §2.2.
    SwitchActiveTab {
        tab_id: SceneId,
    },
    // ── Tile mutations (require create_tiles / modify_own_tiles) ──────────
    /// Create a new tile. Requires `create_tiles` + `modify_own_tiles`. RFC 0001 §2.3.
    CreateTile {
        tab_id: SceneId,
        namespace: String,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
    },
    /// Update tile bounds. RFC 0001 §2.3.
    UpdateTileBounds {
        tile_id: SceneId,
        bounds: Rect,
    },
    /// Update tile z-order. RFC 0001 §2.3.
    UpdateTileZOrder {
        tile_id: SceneId,
        z_order: u32,
    },
    /// Update tile opacity (must be in [0.0, 1.0]). RFC 0001 §2.3.
    UpdateTileOpacity {
        tile_id: SceneId,
        opacity: f32,
    },
    /// Update tile input mode. RFC 0001 §2.3.
    UpdateTileInputMode {
        tile_id: SceneId,
        input_mode: InputMode,
    },
    /// Update tile sync group membership. RFC 0001 §2.3.
    UpdateTileSyncGroup {
        tile_id: SceneId,
        sync_group: Option<SceneId>,
    },
    /// Update tile expiry timestamp. RFC 0001 §2.3.
    UpdateTileExpiry {
        tile_id: SceneId,
        expires_at: Option<u64>,
    },
    /// Delete a tile and all its nodes. RFC 0001 §2.3.
    DeleteTile {
        tile_id: SceneId,
    },
    // ── Node mutations ────────────────────────────────────────────────────
    SetTileRoot {
        tile_id: SceneId,
        node: Node,
    },
    AddNode {
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
    },
    // ── Zone mutations ────────────────────────────────────────────────────
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
    // ── Sync group mutations ──────────────────────────────────────────────
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

impl SceneMutation {
    /// Return the human-readable type name for structured error responses.
    pub fn type_name(&self) -> &'static str {
        match self {
            SceneMutation::CreateTab { .. } => "CreateTab",
            SceneMutation::DeleteTab { .. } => "DeleteTab",
            SceneMutation::RenameTab { .. } => "RenameTab",
            SceneMutation::ReorderTab { .. } => "ReorderTab",
            SceneMutation::SwitchActiveTab { .. } => "SwitchActiveTab",
            SceneMutation::CreateTile { .. } => "CreateTile",
            SceneMutation::UpdateTileBounds { .. } => "UpdateTileBounds",
            SceneMutation::UpdateTileZOrder { .. } => "UpdateTileZOrder",
            SceneMutation::UpdateTileOpacity { .. } => "UpdateTileOpacity",
            SceneMutation::UpdateTileInputMode { .. } => "UpdateTileInputMode",
            SceneMutation::UpdateTileSyncGroup { .. } => "UpdateTileSyncGroup",
            SceneMutation::UpdateTileExpiry { .. } => "UpdateTileExpiry",
            SceneMutation::DeleteTile { .. } => "DeleteTile",
            SceneMutation::SetTileRoot { .. } => "SetTileRoot",
            SceneMutation::AddNode { .. } => "AddNode",
            SceneMutation::PublishToZone { .. } => "PublishToZone",
            SceneMutation::ClearZone { .. } => "ClearZone",
            SceneMutation::CreateSyncGroup { .. } => "CreateSyncGroup",
            SceneMutation::DeleteSyncGroup { .. } => "DeleteSyncGroup",
            SceneMutation::JoinSyncGroup { .. } => "JoinSyncGroup",
            SceneMutation::LeaveSyncGroup { .. } => "LeaveSyncGroup",
        }
    }
}

/// Result of applying a mutation batch.
#[derive(Clone, Debug)]
pub struct MutationResult {
    pub batch_id: SceneId,
    pub applied: bool,
    pub created_ids: Vec<SceneId>,
    pub error: Option<ValidationError>,
    /// Structured rejection response (RFC 0001 §3.4). Present when `applied == false`.
    pub rejection: Option<BatchRejected>,
    /// True if the lease is at the soft budget warning threshold (80%).
    /// The batch was still applied, but the caller should notify the agent.
    pub budget_warning: bool,
    /// Monotonically increasing sequence number assigned when this batch was committed.
    /// `None` if `applied == false`.
    pub sequence_number: Option<u64>,
}

impl MutationResult {
    fn rejected_with_error(
        batch_id: SceneId,
        rejection: BatchRejected,
        error: ValidationError,
    ) -> Self {
        Self {
            batch_id,
            applied: false,
            created_ids: vec![],
            error: Some(error),
            rejection: Some(rejection),
            budget_warning: false,
            sequence_number: None,
        }
    }
}

impl SceneGraph {
    /// Apply a mutation batch atomically per the five-stage validation pipeline.
    ///
    /// # Pipeline stages (RFC 0001 §3.2)
    ///
    /// 1. **Batch size check** — reject if > [`MAX_BATCH_SIZE`] mutations.
    /// 2. **Stage 1: Lease check** — all referenced leases must be Active.
    ///    Uses `batch.lease_id` if set, else discovers lease_ids from `CreateTile`
    ///    mutations and tile lookups. Expired lease is caught here before budget.
    /// 3. **Stage 2: Budget check** — projected resource usage fits within budget.
    /// 4. **Stage 3: Bounds check** — bounds have positive width/height, finite coords.
    /// 5. **Stage 4: Type check** — referenced tabs/tiles/nodes/groups exist.
    /// 6. **Stage 5: Invariant check (post-mutation simulation)** — apply to a clone
    ///    and verify no cycles, no z-order conflicts, no broken internal references.
    ///
    /// On any failure the live graph is untouched. The returned [`MutationResult`]
    /// carries a structured [`BatchRejected`] with per-mutation diagnostics.
    pub fn apply_batch(&mut self, batch: &MutationBatch) -> MutationResult {
        // ── Batch size limit ───────────────────────────────────────────────
        if batch.mutations.len() > MAX_BATCH_SIZE {
            let err = ValidationError::BatchSizeExceeded {
                max: MAX_BATCH_SIZE,
                got: batch.mutations.len(),
            };
            let rejection = BatchRejected::batch_level(batch.batch_id, "batch", &err);
            return MutationResult::rejected_with_error(batch.batch_id, rejection, err);
        }

        // ── Stage 1: Lease check ──────────────────────────────────────────
        // Collect all lease IDs referenced by this batch.
        let mut lease_ids: Vec<SceneId> = Vec::new();

        // Prefer the explicit batch-level lease_id.
        if let Some(lid) = batch.lease_id {
            lease_ids.push(lid);
        }

        // Also harvest any lease IDs embedded in CreateTile mutations.
        for mutation in &batch.mutations {
            if let SceneMutation::CreateTile { lease_id, .. } = mutation {
                if !lease_ids.contains(lease_id) {
                    lease_ids.push(*lease_id);
                }
            }
        }

        // Deduplicate
        lease_ids.sort();
        lease_ids.dedup();

        // Check each referenced lease: must exist and be Active.
        for (idx, mutation) in batch.mutations.iter().enumerate() {
            let maybe_lease_id = Self::lease_id_for_mutation(mutation);
            if let Some(lease_id) = maybe_lease_id {
                if let Some(lease) = self.leases.get(&lease_id) {
                    if !lease.is_mutations_allowed() {
                        // Stage 1 failure: lease is not Active
                        let err = if lease.is_expired(self.now_millis()) {
                            ValidationError::LeaseExpired { id: lease_id }
                        } else {
                            ValidationError::InvalidField {
                                field: "lease_state".into(),
                                reason: format!(
                                    "lease {} is in {:?} state; mutations require Active state",
                                    lease_id, lease.state,
                                ),
                            }
                        };
                        let rejection = BatchRejected::single(
                            batch.batch_id,
                            idx,
                            mutation.type_name(),
                            &err,
                        );
                        return MutationResult::rejected_with_error(
                            batch.batch_id,
                            rejection,
                            err,
                        );
                    }
                } else {
                    let err = ValidationError::LeaseNotFound { id: lease_id };
                    let rejection = BatchRejected::single(
                        batch.batch_id,
                        idx,
                        mutation.type_name(),
                        &err,
                    );
                    return MutationResult::rejected_with_error(batch.batch_id, rejection, err);
                }
            }
        }

        // ── Stage 2: Budget check ─────────────────────────────────────────
        let mut budget_warning = false;
        for lid in &lease_ids {
            if let Err(budget_err) = self.check_budget(lid, batch) {
                let err = ValidationError::BudgetExceeded {
                    resource: format!("{}", budget_err),
                };
                let rejection = BatchRejected::batch_level(batch.batch_id, "batch", &err);
                return MutationResult::rejected_with_error(batch.batch_id, rejection, err);
            }
            if self.is_lease_budget_warning(lid) {
                budget_warning = true;
            }
        }

        // ── Stages 3 + 4 + 5: Apply to a snapshot, collect per-mutation errors ──
        // Clone the scene for rollback and for the post-mutation invariant check.
        let snapshot = self.clone();
        let mut created_ids = Vec::new();

        for (idx, mutation) in batch.mutations.iter().enumerate() {
            // Stage 3: Bounds check (in-line in apply_single_mutation via bounds validation)
            // Stage 4: Type check (in-line — references validated by apply_single_mutation)
            match self.apply_single_mutation(mutation, &batch.agent_namespace) {
                Ok(ids) => created_ids.extend(ids),
                Err(e) => {
                    // Rollback to snapshot
                    *self = snapshot;
                    let rejection = BatchRejected::single(
                        batch.batch_id,
                        idx,
                        mutation.type_name(),
                        &e,
                    );
                    return MutationResult::rejected_with_error(batch.batch_id, rejection, e);
                }
            }
        }

        // ── Stage 5: Post-mutation invariant check ────────────────────────
        if let Err(e) = self.check_post_mutation_invariants(batch) {
            // Rollback: the invariant check found a violation
            *self = snapshot;
            let rejection = BatchRejected::batch_level(batch.batch_id, "batch", &e);
            return MutationResult::rejected_with_error(batch.batch_id, rejection, e);
        }

        // ── Commit ────────────────────────────────────────────────────────
        // Assign a monotonically increasing sequence number.
        let seq = self.next_sequence_number();

        // Re-check budget warning after application (usage may have changed).
        for lid in &lease_ids {
            if self.is_lease_budget_warning(lid) {
                budget_warning = true;
            }
        }

        MutationResult {
            batch_id: batch.batch_id,
            applied: true,
            created_ids,
            error: None,
            rejection: None,
            budget_warning,
            sequence_number: Some(seq),
        }
    }

    /// Extract the lease_id directly embedded in a mutation, if applicable.
    fn lease_id_for_mutation(mutation: &SceneMutation) -> Option<SceneId> {
        match mutation {
            SceneMutation::CreateTile { lease_id, .. } => Some(*lease_id),
            _ => None,
        }
    }

    /// Check post-mutation invariants on the (already mutated) working graph.
    ///
    /// Stage 5: verifies:
    /// 1. No cycles in node trees.
    /// 2. No exclusive z-order conflicts among non-passthrough tiles on the same tab.
    fn check_post_mutation_invariants(
        &self,
        _batch: &MutationBatch,
    ) -> Result<(), ValidationError> {
        // 5a: Cycle detection in node trees
        // Walk each tile's root node tree and ensure no node appears twice.
        for tile in self.tiles.values() {
            if let Some(root_id) = tile.root_node {
                let mut visited = std::collections::HashSet::new();
                if let Err(cycle_node) = self.detect_cycle(root_id, &mut visited) {
                    return Err(ValidationError::CycleDetected { node_id: cycle_node });
                }
            }
        }

        // 5b: Z-order conflict detection
        // Group tiles by tab. Within each tab, detect non-passthrough tiles that share
        // a z_order AND have overlapping bounds.
        let mut tab_tiles: std::collections::HashMap<SceneId, Vec<&Tile>> =
            std::collections::HashMap::new();
        for tile in self.tiles.values() {
            tab_tiles.entry(tile.tab_id).or_default().push(tile);
        }

        for tiles in tab_tiles.values() {
            // O(n²) is fine for the max tile count (64 per lease, 64 leases = 4096 max,
            // but in practice batches are small). If this becomes a bottleneck we can
            // bucket by z_order first.
            for i in 0..tiles.len() {
                for j in (i + 1)..tiles.len() {
                    let a = tiles[i];
                    let b = tiles[j];
                    if a.z_order == b.z_order
                        && a.input_mode != InputMode::Passthrough
                        && b.input_mode != InputMode::Passthrough
                        && a.bounds.intersects(&b.bounds)
                    {
                        return Err(ValidationError::ZOrderConflict {
                            tile_a: a.id,
                            tile_b: b.id,
                            z_order: a.z_order,
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// DFS cycle detection for a node subtree.
    ///
    /// Returns `Ok(())` if the subtree is acyclic, or `Err(node_id)` identifying the
    /// node that creates the cycle.
    ///
    /// # Algorithm
    ///
    /// `visited` tracks the current DFS path (nodes on the active recursion stack).
    /// A node is removed from `visited` when the recursion backtracks from it, so
    /// shared child nodes (valid in a DAG) are not incorrectly flagged as cycles.
    /// Only true back-edges (node encountered while still on the active path) are
    /// rejected as cycles.
    fn detect_cycle(
        &self,
        node_id: SceneId,
        visited: &mut std::collections::HashSet<SceneId>,
    ) -> Result<(), SceneId> {
        if !visited.insert(node_id) {
            // node_id is already on the active DFS path → back-edge → cycle
            return Err(node_id);
        }
        if let Some(node) = self.nodes.get(&node_id) {
            for &child_id in &node.children {
                self.detect_cycle(child_id, visited)?;
            }
        }
        // Backtrack: remove from path so sibling branches can share this node
        // without being falsely flagged as cycles.
        visited.remove(&node_id);
        Ok(())
    }

    fn apply_single_mutation(
        &mut self,
        mutation: &SceneMutation,
        namespace: &str,
    ) -> Result<Vec<SceneId>, ValidationError> {
        match mutation {
            // ── Tab mutations ─────────────────────────────────────────────────
            SceneMutation::CreateTab { name, display_order } => {
                let id = self.create_tab(name, *display_order)?;
                Ok(vec![id])
            }
            SceneMutation::DeleteTab { tab_id } => {
                self.delete_tab(*tab_id)?;
                Ok(vec![])
            }
            SceneMutation::RenameTab { tab_id, new_name } => {
                self.rename_tab(*tab_id, new_name)?;
                Ok(vec![])
            }
            SceneMutation::ReorderTab { tab_id, new_order } => {
                self.reorder_tab(*tab_id, *new_order)?;
                Ok(vec![])
            }
            SceneMutation::SwitchActiveTab { tab_id } => {
                self.switch_active_tab(*tab_id)?;
                Ok(vec![])
            }
            // ── Tile mutations ────────────────────────────────────────────────
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
            SceneMutation::UpdateTileBounds { tile_id, bounds } => {
                // Route through the checked method to enforce namespace isolation,
                // lease/capability checks, and the within-display-area invariant.
                self.update_tile_bounds(*tile_id, *bounds, namespace)?;
                Ok(vec![])
            }
            SceneMutation::UpdateTileZOrder { tile_id, z_order } => {
                self.update_tile_z_order(*tile_id, *z_order, namespace)?;
                Ok(vec![])
            }
            SceneMutation::UpdateTileOpacity { tile_id, opacity } => {
                self.update_tile_opacity(*tile_id, *opacity, namespace)?;
                Ok(vec![])
            }
            SceneMutation::UpdateTileInputMode { tile_id, input_mode } => {
                self.update_tile_input_mode(*tile_id, *input_mode, namespace)?;
                Ok(vec![])
            }
            SceneMutation::UpdateTileSyncGroup { tile_id, sync_group } => {
                if let Some(group_id) = sync_group {
                    self.join_sync_group(*tile_id, *group_id)?;
                } else {
                    // Clear the sync group
                    let _ = self.leave_sync_group(*tile_id);
                }
                Ok(vec![])
            }
            SceneMutation::UpdateTileExpiry { tile_id, expires_at } => {
                self.update_tile_expiry(*tile_id, *expires_at, namespace)?;
                Ok(vec![])
            }
            SceneMutation::DeleteTile { tile_id } => {
                // Use the checked delete which enforces namespace isolation and capabilities.
                self.delete_tile(*tile_id, namespace)?;
                Ok(vec![])
            }
            // ── Node mutations ────────────────────────────────────────────────
            SceneMutation::SetTileRoot { tile_id, node } => {
                // Use checked variant to enforce namespace isolation and ModifyOwnTiles capability.
                self.set_tile_root_checked(*tile_id, node.clone(), namespace)?;
                Ok(vec![node.id])
            }
            SceneMutation::AddNode {
                tile_id,
                parent_id,
                node,
            } => {
                // Use checked variant to enforce namespace isolation and ModifyOwnTiles capability.
                self.add_node_to_tile_checked(*tile_id, *parent_id, node.clone(), namespace)?;
                Ok(vec![node.id])
            }
            // ── Zone mutations ────────────────────────────────────────────────
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
            // ── Sync group mutations ──────────────────────────────────────────
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

    fn make_batch(agent: &str, mutations: Vec<SceneMutation>) -> MutationBatch {
        MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: agent.to_string(),
            mutations,
            timing_hints: None,
            lease_id: None,
        }
    }

    fn make_batch_with_lease(
        agent: &str,
        lease_id: SceneId,
        mutations: Vec<SceneMutation>,
    ) -> MutationBatch {
        MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: agent.to_string(),
            mutations,
            timing_hints: None,
            lease_id: Some(lease_id),
        }
    }

    #[test]
    fn test_mutation_batch_apply() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

        let batch = make_batch("agent", vec![
            SceneMutation::CreateTile {
                tab_id,
                namespace: "agent".to_string(),
                lease_id,
                bounds: Rect::new(10.0, 10.0, 200.0, 150.0),
                z_order: 1,
            },
        ]);

        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert_eq!(result.created_ids.len(), 1);
        assert_eq!(scene.tile_count(), 1);
        assert!(result.sequence_number.is_some());
    }

    #[test]
    fn test_mutation_batch_rollback_on_failure() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

        let batch = make_batch("agent", vec![
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
        ]);

        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        // Entire batch rolled back — no tiles created
        assert_eq!(scene.tile_count(), 0);
        // Structured rejection must be present
        assert!(result.rejection.is_some());
        let rej = result.rejection.unwrap();
        assert_eq!(rej.errors[0].mutation_index, 1);
        // Zero-size bounds is a BoundsInvalid violation (width/height must be > 0.0)
        assert_eq!(rej.errors[0].code, ValidationErrorCode::BoundsInvalid);
    }

    #[test]
    fn test_batch_size_exceeded() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

        // Build a batch with 1001 mutations
        let mutations: Vec<SceneMutation> = (0..=1000)
            .map(|z| SceneMutation::CreateTile {
                tab_id,
                namespace: "agent".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
                z_order: z as u32,
            })
            .collect();

        let batch = make_batch("agent", mutations);
        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        let rej = result.rejection.unwrap();
        assert_eq!(rej.primary_code(), Some(ValidationErrorCode::BatchSizeExceeded));
        // No tiles created
        assert_eq!(scene.tile_count(), 0);
    }

    #[test]
    fn test_lease_check_before_budget_check() {
        // Stage 1 (lease check) must fire before Stage 2 (budget check).
        // We set an expired lease; the rejection must be LeaseExpired / LeaseInvalidState,
        // not BudgetExceeded.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // Grant a lease with a 1ms TTL, then immediately expire it
        let lease_id = scene.grant_lease("agent", 1, vec![Capability::CreateTile]);
        // Advance the clock past TTL by expiring leases (simulated by direct state manipulation)
        scene.leases.get_mut(&lease_id).unwrap().state =
            crate::types::LeaseState::Expired;

        let batch = make_batch("agent", vec![
            SceneMutation::CreateTile {
                tab_id,
                namespace: "agent".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            },
        ]);

        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        let rej = result.rejection.unwrap();
        let code = rej.primary_code().unwrap();
        // Must be a lease-stage error, not a budget error
        assert!(
            matches!(code, ValidationErrorCode::LeaseInvalidState | ValidationErrorCode::LeaseExpired | ValidationErrorCode::LeaseNotFound),
            "expected lease-stage error, got {:?}", code
        );
    }

    #[test]
    fn test_sequence_numbers_monotonically_increasing() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

        let mut prev_seq = 0u64;
        for z in 1..=5u32 {
            let batch = make_batch("agent", vec![
                SceneMutation::CreateTile {
                    tab_id,
                    namespace: "agent".to_string(),
                    lease_id,
                    bounds: Rect::new(z as f32 * 10.0, 0.0, 50.0, 50.0),
                    z_order: z,
                },
            ]);
            let result = scene.apply_batch(&batch);
            assert!(result.applied, "batch {z} failed");
            let seq = result.sequence_number.unwrap();
            assert!(seq > prev_seq, "sequence {seq} not strictly greater than {prev_seq}");
            prev_seq = seq;
        }
    }

    #[test]
    fn test_z_order_conflict_detected() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

        // Create first tile at z_order=1 with bounds [0,0,200,200]
        let b1 = make_batch("agent", vec![
            SceneMutation::CreateTile {
                tab_id,
                namespace: "agent".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
                z_order: 1,
            },
        ]);
        let r1 = scene.apply_batch(&b1);
        assert!(r1.applied);

        // Try to create a second tile at same z_order=1 with overlapping bounds
        let b2 = make_batch("agent", vec![
            SceneMutation::CreateTile {
                tab_id,
                namespace: "agent".to_string(),
                lease_id,
                bounds: Rect::new(100.0, 100.0, 200.0, 200.0), // overlaps first tile
                z_order: 1, // same z_order
            },
        ]);
        let r2 = scene.apply_batch(&b2);
        assert!(!r2.applied, "should reject z-order conflict");
        let rej = r2.rejection.unwrap();
        assert_eq!(rej.primary_code(), Some(ValidationErrorCode::ZOrderConflict));
    }

    #[test]
    fn test_mutation_create_and_delete_sync_group() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        // Create a sync group via mutation batch
        let create_batch = make_batch("agent", vec![SceneMutation::CreateSyncGroup {
            name: Some("my-group".to_string()),
            owner_namespace: "agent".to_string(),
            commit_policy: SyncCommitPolicy::AllOrDefer,
            max_deferrals: 3,
        }]);
        let result = scene.apply_batch(&create_batch);
        assert!(result.applied);
        assert_eq!(result.created_ids.len(), 1);
        let group_id = result.created_ids[0];
        assert_eq!(scene.sync_group_count(), 1);

        // Delete via mutation batch
        let delete_batch = make_batch("agent", vec![
            SceneMutation::DeleteSyncGroup { group_id },
        ]);
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
        let batch = make_batch("agent", vec![SceneMutation::CreateSyncGroup {
            name: None,
            owner_namespace: "agent".to_string(),
            commit_policy: SyncCommitPolicy::AvailableMembers,
            max_deferrals: 0,
        }]);
        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        let group_id = result.created_ids[0];

        // Join tile to group
        let join_batch = make_batch("agent", vec![
            SceneMutation::JoinSyncGroup { tile_id, group_id },
        ]);
        let result = scene.apply_batch(&join_batch);
        assert!(result.applied);
        assert_eq!(scene.tiles[&tile_id].sync_group, Some(group_id));
        assert!(scene.sync_groups[&group_id].members.contains(&tile_id));

        // Leave sync group
        let leave_batch = make_batch("agent", vec![
            SceneMutation::LeaveSyncGroup { tile_id },
        ]);
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
        let batch = make_batch("agent", vec![SceneMutation::JoinSyncGroup {
            tile_id,
            group_id: nonexistent_group,
        }]);
        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        assert!(result.rejection.is_some());
        // Tile should remain without a sync group
        assert_eq!(scene.tiles[&tile_id].sync_group, None);
    }

    #[test]
    fn test_structured_error_has_required_fields() {
        // Verify structured rejection includes mutation_index, code, message, context.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

        let batch = make_batch("agent", vec![
            SceneMutation::CreateTile {
                tab_id,
                namespace: "agent".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            },
            // Second mutation fails with invalid bounds
            SceneMutation::UpdateTileBounds {
                tile_id: SceneId::new(), // non-existent tile
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            },
        ]);

        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        let rej = result.rejection.unwrap();
        let err = &rej.errors[0];
        assert_eq!(err.mutation_index, 1);
        assert_eq!(err.mutation_type, "UpdateTileBounds");
        assert!(!err.message.is_empty());
        // Context must be a JSON object
        assert!(err.context.is_object());
    }
}
