use super::*;

impl SceneGraph {
    // ─── Budget enforcement ─────────────────────────────────────────────

    /// Get current resource usage for a lease.
    pub fn lease_resource_usage(&self, lease_id: &SceneId) -> ResourceUsage {
        let mut usage = ResourceUsage::default();
        for tile in self.tiles.values().filter(|t| t.lease_id == *lease_id) {
            usage.tiles += 1;
            // Count nodes in this tile
            let node_count = self.count_nodes_in_tile(tile);
            usage.nodes_per_tile.insert(tile.id, node_count);
            // Sum texture bytes for static image nodes in this tile
            if let Some(root_id) = tile.root_node {
                usage.texture_bytes += self.sum_texture_bytes(root_id);
            }
        }
        usage
    }

    /// Check if a mutation batch would exceed the lease's resource budget.
    ///
    /// Returns Ok(()) if within budget, or Err with the specific violation.
    pub fn check_budget(
        &self,
        lease_id: &SceneId,
        batch: &crate::mutation::MutationBatch,
    ) -> Result<(), BudgetError> {
        let lease = match self.leases.get(lease_id) {
            Some(l) => l,
            None => return Ok(()), // No lease = no budget to check
        };
        let budget = &lease.resource_budget;
        let usage = self.lease_resource_usage(lease_id);

        // Count new tiles in batch
        let new_tiles: u32 = batch
            .mutations
            .iter()
            .filter(|m| matches!(m, crate::mutation::SceneMutation::CreateTile { .. }))
            .count() as u32;

        if new_tiles > 0 {
            let projected = usage.tiles as u64 + new_tiles as u64;
            if projected > budget.max_tiles as u64 {
                return Err(BudgetError {
                    resource: "tiles".to_string(),
                    current: usage.tiles as u64,
                    limit: budget.max_tiles as u64,
                    requested: new_tiles as u64,
                });
            }
        }

        // Running projected texture total for the batch.  We accumulate deltas
        // across SetTileRoot and UpdateNodeContent mutations so that a batch
        // with multiple texture swaps is evaluated against the cumulative
        // projected usage, not independently against the initial snapshot.
        let mut projected_tex = usage.texture_bytes;

        // Count new nodes per tile (AddNode / SetTileRoot)
        for mutation in &batch.mutations {
            match mutation {
                crate::mutation::SceneMutation::AddNode { tile_id, node, .. } => {
                    let current = usage.nodes_per_tile.get(tile_id).copied().unwrap_or(0);
                    let new_count = Self::fresh_batch_node_count(node);
                    let projected = current as u64 + new_count as u64;
                    if projected > budget.max_nodes_per_tile as u64 {
                        return Err(BudgetError {
                            resource: "nodes_per_tile".to_string(),
                            current: current as u64,
                            limit: budget.max_nodes_per_tile as u64,
                            requested: new_count as u64,
                        });
                    }
                }
                crate::mutation::SceneMutation::SetTileRoot {
                    tile_id,
                    node,
                    descendants,
                } => {
                    // SetTileRoot replaces the entire tree, so count new tree size.
                    // With an inline subtree (hud-ga4md) the root plus every fresh
                    // descendant counts against the per-tile node budget.
                    let new_count =
                        Self::fresh_batch_node_count(node) as u64 + descendants.len() as u64;
                    if new_count > budget.max_nodes_per_tile as u64 {
                        return Err(BudgetError {
                            resource: "nodes_per_tile".to_string(),
                            current: 0,
                            limit: budget.max_nodes_per_tile as u64,
                            requested: new_count,
                        });
                    }
                    // Check texture bytes in new tree against the running projected total.
                    // Sum the root and every inline descendant's texture bytes.
                    let new_tex = Self::count_texture_bytes_in_node(node)
                        + descendants
                            .iter()
                            .map(Self::count_texture_bytes_in_node)
                            .sum::<u64>();
                    let old_tile_tex = self
                        .tiles
                        .get(tile_id)
                        .and_then(|t| t.root_node)
                        .map(|r| self.sum_texture_bytes(r))
                        .unwrap_or(0);
                    let other_tex = projected_tex.saturating_sub(old_tile_tex);
                    if other_tex.saturating_add(new_tex) > budget.max_texture_bytes {
                        return Err(BudgetError {
                            resource: "texture_bytes".to_string(),
                            current: other_tex,
                            limit: budget.max_texture_bytes,
                            requested: new_tex,
                        });
                    }
                    // Advance the running projected total for subsequent mutations.
                    projected_tex = other_tex.saturating_add(new_tex);
                }
                crate::mutation::SceneMutation::UpdateNodeContent {
                    node_id,
                    data: NodeData::StaticImage(new_si),
                    ..
                } => {
                    // UpdateNodeContent on a StaticImage node swaps the texture.
                    // Compute the old texture bytes for this specific node (if any),
                    // subtract them from the running projected total, and check
                    // whether the replacement fits within the budget.
                    //
                    // If the resource_id is unchanged and decoded_bytes == 0, the
                    // preservation logic in update_node_content_impl will restore the
                    // stored value — so the net texture delta is zero and no budget
                    // violation can occur.  If decoded_bytes > 0, the caller has
                    // supplied a concrete new size and we must validate it.
                    let new_tex = new_si.decoded_bytes;
                    if new_tex > 0 {
                        let old_tex = self
                            .nodes
                            .get(node_id)
                            .map(|n| match &n.data {
                                NodeData::StaticImage(si) => si.decoded_bytes,
                                _ => 0,
                            })
                            .unwrap_or(0);
                        let other_tex = projected_tex.saturating_sub(old_tex);
                        if other_tex.saturating_add(new_tex) > budget.max_texture_bytes {
                            return Err(BudgetError {
                                resource: "texture_bytes".to_string(),
                                current: other_tex,
                                limit: budget.max_texture_bytes,
                                requested: new_tex,
                            });
                        }
                        // Advance the running projected total for subsequent mutations.
                        projected_tex = other_tex.saturating_add(new_tex);
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Check if a lease is at the soft budget warning threshold (80%).
    pub fn is_lease_budget_warning(&self, lease_id: &SceneId) -> bool {
        let lease = match self.leases.get(lease_id) {
            Some(l) => l,
            None => return false,
        };
        let usage = self.lease_resource_usage(lease_id);
        let budget = &lease.resource_budget;

        let tile_pct = usage.tiles as f64 / budget.max_tiles.max(1) as f64;
        let tex_pct = usage.texture_bytes as f64 / budget.max_texture_bytes.max(1) as f64;

        tile_pct >= Self::BUDGET_SOFT_LIMIT_PCT || tex_pct >= Self::BUDGET_SOFT_LIMIT_PCT
    }

    /// Count nodes in a tile by walking the root node tree.
    pub(super) fn count_nodes_in_tile(&self, tile: &Tile) -> u32 {
        match tile.root_node {
            Some(root_id) => self.count_node_subtree(root_id),
            None => 0,
        }
    }

    pub(crate) fn count_node_subtree(&self, node_id: SceneId) -> u32 {
        let mut visited = HashSet::new();
        self.count_node_subtree_inner(node_id, &mut visited)
    }

    fn count_node_subtree_inner(&self, node_id: SceneId, visited: &mut HashSet<SceneId>) -> u32 {
        if !visited.insert(node_id) {
            // Cycle detected — skip this node to avoid infinite recursion.
            #[cfg(debug_assertions)]
            eprintln!(
                "[tze_hud_scene] cycle detected in node graph at {node_id:?} during count_node_subtree"
            );
            return 0;
        }
        match self.nodes.get(&node_id) {
            Some(node) => {
                1 + node
                    .children
                    .iter()
                    .map(|c| self.count_node_subtree_inner(*c, visited))
                    .sum::<u32>()
            }
            None => 0,
        }
    }

    pub(super) fn sum_texture_bytes(&self, node_id: SceneId) -> u64 {
        let mut visited = HashSet::new();
        self.sum_texture_bytes_inner(node_id, &mut visited)
    }

    fn sum_texture_bytes_inner(&self, node_id: SceneId, visited: &mut HashSet<SceneId>) -> u64 {
        if !visited.insert(node_id) {
            // Cycle detected — skip this node to avoid infinite recursion.
            #[cfg(debug_assertions)]
            eprintln!(
                "[tze_hud_scene] cycle detected in node graph at {node_id:?} during sum_texture_bytes"
            );
            return 0;
        }
        match self.nodes.get(&node_id) {
            Some(node) => {
                let self_bytes = match &node.data {
                    NodeData::StaticImage(img) => img.decoded_bytes,
                    _ => 0,
                };
                self_bytes
                    + node
                        .children
                        .iter()
                        .map(|c| self.sum_texture_bytes_inner(*c, visited))
                        .sum::<u64>()
            }
            None => 0,
        }
    }

    /// Number of nodes a not-yet-inserted node contributes to its tile when
    /// submitted as part of a fresh mutation batch.
    ///
    /// This is **always 1** by construction, not a tree walk: in the current
    /// node model a `Node`'s children are `SceneId` references, and in a fresh
    /// batch each child arrives as its own separate `AddNode`/`SetTileRoot`
    /// mutation. The incoming node therefore adds exactly itself; its children
    /// are budgeted when their own mutations are processed.
    ///
    /// Contrast with [`SceneGraph::count_node_tree_deep`], which *does* walk
    /// children that already exist in the graph (used by `set_tile_root_impl`
    /// to catch re-attachment of a persisted subtree).
    fn fresh_batch_node_count(_node: &Node) -> u32 {
        1
    }

    /// Count the incoming node plus any of its children that are already in the graph.
    ///
    /// Used by `set_tile_root_impl` to validate the post-insert node count before
    /// replacing the tile root. The incoming `node` is counted as 1, and any of its
    /// `children` SceneIds that already exist in `self.nodes` are recursively counted.
    ///
    /// For a brand-new node with no pre-existing children, this returns 1 (correct).
    /// For a node whose `children` already reference persisted nodes (e.g., re-attaching
    /// an existing subtree), this returns the full subtree size, preventing the node
    /// count limit from being bypassed.
    pub(super) fn count_node_tree_deep(&self, node: &Node) -> usize {
        1 + node
            .children
            .iter()
            .map(|child_id| self.count_node_subtree(*child_id) as usize)
            .sum::<usize>()
    }

    /// Count texture bytes in a node (not yet inserted into the graph).
    fn count_texture_bytes_in_node(node: &Node) -> u64 {
        match &node.data {
            NodeData::StaticImage(img) => img.decoded_bytes,
            _ => 0,
        }
    }

    /// Budget-driven revocation: transitions all non-terminal session leases to
    /// REVOKED, clears tiles, clears zone publications.
    ///
    /// Spec §Post-Revocation Resource Cleanup (lines 253–260):
    /// - Bypasses the grace period entirely.
    /// - Caller is responsible for sending `LeaseResponse{revoke_reason=BUDGET_POLICY}`
    ///   and then waiting `POST_REVOCATION_FREE_DELAY_MS` before calling
    ///   `finalize_budget_revocation`.
    ///
    /// Returns the cleanup spec (containing the free delay) for each revoked lease.
    pub fn initiate_budget_revocation(
        &mut self,
        session_namespace: &str,
    ) -> Vec<crate::lease::cleanup::PostRevocationCleanupSpec> {
        use crate::lease::cleanup::{PostRevocationCleanupSpec, RevocationKind};
        let now_ms = self.clock.now_millis();

        // Collect all non-terminal leases for this namespace.
        let to_revoke: Vec<SceneId> = self
            .leases
            .values()
            .filter(|l| l.namespace == session_namespace && !l.state.is_terminal())
            .map(|l| l.id)
            .collect();

        let mut specs = Vec::new();
        for lease_id in to_revoke {
            // Transition to REVOKED (bypasses grace — no orphan path).
            if let Some(lease) = self.leases.get_mut(&lease_id) {
                lease.state = LeaseState::Revoked;
            }
            // Tiles will be freed after the 100ms delay by finalize_budget_revocation.
            // Budget revocation bypasses the orphan/disconnection path, so tiles
            // do not receive a DisconnectionBadge — they are simply marked for
            // pending removal (visual_hint remains None; compositor will not render
            // them once removed by finalize_budget_revocation).

            // Clear zone and widget publications immediately on REVOKED transition.
            // Spec §Requirement: Lease Revocation Clears Zone Publications
            // (lines 235–242): zone pubs must be cleared when lease is REVOKED/EXPIRED.
            // Widget publications are similarly cleared immediately.
            // Tile/node resources are deferred by the 100ms delay; zone/widget pubs are not.
            if let Some(lease) = self.leases.get(&lease_id) {
                let ns = lease.namespace.clone();
                self.clear_zone_publications_for_namespace(&ns);
                self.clear_widget_publications_for_namespace(&ns);
            }
            specs.push(PostRevocationCleanupSpec::new(
                lease_id,
                session_namespace,
                RevocationKind::BudgetPolicy,
                now_ms,
            ));
        }

        if !specs.is_empty() {
            self.version += 1;
        }
        specs
    }

    /// Finalize budget revocation: remove tiles and zone publications for
    /// all leases in the cleanup specs that are ready to free.
    ///
    /// Must be called after `POST_REVOCATION_FREE_DELAY_MS` has elapsed
    /// (spec line 254: "free all resources after a 100ms delay").
    ///
    /// Returns the number of specs that were finalized.
    pub fn finalize_budget_revocation(
        &mut self,
        specs: &[crate::lease::cleanup::PostRevocationCleanupSpec],
        now_ms: u64,
    ) -> usize {
        let mut finalized = 0;
        for spec in specs {
            if spec.is_ready_to_free(now_ms) {
                // Remove tiles
                let tile_ids: Vec<SceneId> = self
                    .tiles
                    .values()
                    .filter(|t| t.lease_id == spec.lease_id)
                    .map(|t| t.id)
                    .collect();
                // Leave sync groups before removing tiles to avoid dangling member entries.
                for tid in &tile_ids {
                    let _ = self.leave_sync_group(*tid);
                }
                for tid in tile_ids {
                    self.remove_tile_and_nodes(tid);
                }
                // Clear zone and widget publications
                self.clear_zone_publications_for_namespace(&spec.session_namespace);
                self.clear_widget_publications_for_namespace(&spec.session_namespace);
                finalized += 1;
            }
        }
        if finalized > 0 {
            self.version += 1;
        }
        finalized
    }
}
