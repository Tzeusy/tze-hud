use super::*;

impl SceneGraph {
    // ─── Sync group operations ───────────────────────────────────────────

    /// Maximum sync groups per agent namespace (RFC 0003 §2.5).
    pub const MAX_SYNC_GROUPS_PER_NAMESPACE: usize = 16;

    /// Maximum tiles per sync group (RFC 0003 §2.5).
    pub const MAX_MEMBERS_PER_SYNC_GROUP: usize = 64;

    /// Create a new sync group. Returns the new sync group ID.
    pub fn create_sync_group(
        &mut self,
        name: Option<String>,
        owner_namespace: &str,
        commit_policy: SyncCommitPolicy,
        max_deferrals: u32,
    ) -> Result<SceneId, ValidationError> {
        // Enforce per-namespace limit (RFC 0003 §2.5)
        let existing_count = self
            .sync_groups
            .values()
            .filter(|sg| sg.owner_namespace == owner_namespace)
            .count();
        if existing_count >= Self::MAX_SYNC_GROUPS_PER_NAMESPACE {
            return Err(ValidationError::SyncGroupLimitExceeded {
                limit: Self::MAX_SYNC_GROUPS_PER_NAMESPACE,
            });
        }

        let id = SceneId::new();
        let created_at_us = super::now_micros();
        self.sync_groups.insert(
            id,
            SyncGroup::new(
                id,
                name,
                owner_namespace.to_string(),
                commit_policy,
                max_deferrals,
                created_at_us,
            ),
        );
        self.version += 1;
        Ok(id)
    }

    /// Delete a sync group. All member tiles are automatically released.
    pub fn delete_sync_group(&mut self, group_id: SceneId) -> Result<(), ValidationError> {
        if let Some(group) = self.sync_groups.remove(&group_id) {
            // Release only the tiles that are members of this group.
            // Iterating the member set is O(k) where k = member count, not O(n tiles).
            for tile_id in group.members {
                if let Some(tile) = self.tiles.get_mut(&tile_id) {
                    tile.sync_group = None;
                }
            }
            self.version += 1;
            Ok(())
        } else {
            Err(ValidationError::SyncGroupNotFound { id: group_id })
        }
    }

    /// Add a tile to a sync group.
    ///
    /// A tile may belong to at most one sync group (RFC 0003 §2.3). Joining
    /// replaces any previous group membership.
    pub fn join_sync_group(
        &mut self,
        tile_id: SceneId,
        group_id: SceneId,
    ) -> Result<(), ValidationError> {
        if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }
        if !self.sync_groups.contains_key(&group_id) {
            return Err(ValidationError::SyncGroupNotFound { id: group_id });
        }

        // Enforce member limit
        let member_count = self
            .sync_groups
            .get(&group_id)
            .map(|sg| sg.members.len())
            .unwrap_or(0);
        // Only enforce if tile is not already in this group
        let already_member = self
            .sync_groups
            .get(&group_id)
            .map(|sg| sg.members.contains(&tile_id))
            .unwrap_or(false);
        if !already_member && member_count >= Self::MAX_MEMBERS_PER_SYNC_GROUP {
            return Err(ValidationError::SyncGroupMemberLimitExceeded {
                limit: Self::MAX_MEMBERS_PER_SYNC_GROUP,
            });
        }

        // If tile is currently in a different group, remove it from that group first
        let current_group = self.tiles.get(&tile_id).and_then(|t| t.sync_group);
        if let Some(old_group_id) = current_group
            && old_group_id != group_id
            && let Some(old_group) = self.sync_groups.get_mut(&old_group_id)
        {
            old_group.members.remove(&tile_id);
        }

        // Update tile's sync_group reference
        let tile = self
            .tiles
            .get_mut(&tile_id)
            .expect("tile_id existence verified by get_tile_lease_checked");
        tile.sync_group = Some(group_id);

        // Add to the group's member set
        self.sync_groups
            .get_mut(&group_id)
            .expect("group_id existence verified at function entry")
            .members
            .insert(tile_id);

        self.version += 1;
        Ok(())
    }

    /// Remove a tile from its sync group.
    ///
    /// Removes the tile from whatever group it currently belongs to.
    /// If the tile is not in any group, this is a no-op (returns Ok).
    /// If the group becomes empty after the last member leaves it is **not**
    /// automatically destroyed — destruction is explicit (RFC 0003 §2.3).
    pub fn leave_sync_group(&mut self, tile_id: SceneId) -> Result<(), ValidationError> {
        if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }
        let current_group = self.tiles.get(&tile_id).and_then(|t| t.sync_group);
        if let Some(group_id) = current_group {
            if let Some(group) = self.sync_groups.get_mut(&group_id) {
                group.members.remove(&tile_id);
            }
            let tile = self
                .tiles
                .get_mut(&tile_id)
                .expect("tile_id existence checked at function entry");
            tile.sync_group = None;
        }
        self.version += 1;
        Ok(())
    }

    /// Evaluate a sync group's commit policy for a given set of tiles that
    /// have pending mutations this frame.
    ///
    /// Returns a `SyncGroupCommitDecision` describing whether to commit,
    /// defer, or force-commit the group.
    ///
    /// This is called by the compositor at Stage 4 (Scene Commit).
    ///
    /// # Correctness invariant
    ///
    /// `deferral_count` MUST only increment when at least one member has a
    /// pending mutation AND at least one member is absent. When the group is
    /// idle (zero pending mutations), the counter MUST NOT change.
    /// Spec: timing-model/spec.md lines 159, 167–169.
    pub fn evaluate_sync_group_commit(
        &mut self,
        group_id: SceneId,
        tiles_with_pending: &std::collections::BTreeSet<SceneId>,
    ) -> Result<SyncGroupCommitDecision, ValidationError> {
        use crate::timing::sync_commit::{CommitDecision, apply_decision, evaluate_commit};

        let group = self
            .sync_groups
            .get(&group_id)
            .ok_or(ValidationError::SyncGroupNotFound { id: group_id })?;

        let decision = evaluate_commit(group, tiles_with_pending);

        // Translate CommitDecision → SyncGroupCommitDecision and apply state
        // changes to the group.
        let result = match &decision {
            CommitDecision::Commit { tiles } => SyncGroupCommitDecision::Commit {
                tiles: tiles.clone(),
            },
            CommitDecision::Defer => SyncGroupCommitDecision::Defer,
            CommitDecision::ForceCommit {
                committed_tiles, ..
            } => SyncGroupCommitDecision::ForceCommit {
                tiles: committed_tiles.clone(),
            },
        };

        // Apply state mutation (deferral_count update) to the group.
        let group_mut = self
            .sync_groups
            .get_mut(&group_id)
            .expect("group_id existence verified by ok_or above");
        apply_decision(group_mut, &decision);

        Ok(result)
    }

    /// Add a tile to a sync group with an ownership check.
    ///
    /// The `agent_namespace` must match both the tile's namespace and the
    /// sync group's `owner_namespace`. This enforces the spec rule that an
    /// agent MUST NOT place another agent's tiles into a sync group.
    ///
    /// Spec: timing-model/spec.md lines 188–189.
    pub fn join_sync_group_checked(
        &mut self,
        tile_id: SceneId,
        group_id: SceneId,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        use crate::timing::sync_group::check_sync_group_ownership;

        let tile = self
            .tiles
            .get(&tile_id)
            .ok_or(ValidationError::TileNotFound { id: tile_id })?;
        let group = self
            .sync_groups
            .get(&group_id)
            .ok_or(ValidationError::SyncGroupNotFound { id: group_id })?;

        check_sync_group_ownership(agent_namespace, &tile.namespace, &group.owner_namespace)
            .map_err(|reason| ValidationError::SyncGroupOwnershipViolation { reason })?;

        self.join_sync_group(tile_id, group_id)
    }

    /// Return the number of sync groups in the scene.
    pub fn sync_group_count(&self) -> usize {
        self.sync_groups.len()
    }
}
