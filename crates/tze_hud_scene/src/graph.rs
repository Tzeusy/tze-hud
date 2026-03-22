//! Scene graph: the core data structure holding all tabs, tiles, nodes, leases.
//! Pure data — no GPU, no async, no I/O.

use crate::clock::{Clock, SystemClock};
use crate::types::*;
use crate::validation::ValidationError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Returns a `SystemClock` wrapped in `Arc<dyn Clock>`.
/// Used as the serde default for the `clock` field so that deserialized
/// graphs behave like freshly constructed ones.
fn default_clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock)
}

/// The root scene graph.
///
/// Time-dependent operations (lease grant, tab creation timestamps, expiry
/// checks) are routed through the injected [`Clock`].  Use
/// [`SceneGraph::new`] for production code — it installs a [`SystemClock`].
/// Use [`SceneGraph::new_with_clock`] in tests to inject a [`TestClock`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneGraph {
    /// Clock used for all `now_millis()` calls inside the graph.
    /// Skipped during serialization; restored to `SystemClock` on
    /// deserialization.
    #[serde(skip, default = "default_clock")]
    clock: Arc<dyn Clock>,
    /// All tabs, keyed by ID.
    pub tabs: HashMap<SceneId, Tab>,
    /// The currently active tab.
    pub active_tab: Option<SceneId>,
    /// All tiles, keyed by ID.
    pub tiles: HashMap<SceneId, Tile>,
    /// All nodes, keyed by ID.
    pub nodes: HashMap<SceneId, Node>,
    /// Active leases, keyed by ID.
    pub leases: HashMap<SceneId, Lease>,
    /// Hit region local state, keyed by node ID.
    pub hit_region_states: HashMap<SceneId, HitRegionLocalState>,
    /// Zone registry.
    pub zone_registry: ZoneRegistry,
    /// Sync groups, keyed by ID.
    pub sync_groups: HashMap<SceneId, SyncGroup>,
    /// Display area (the viewport dimensions).
    pub display_area: Rect,
    /// Monotonic version counter, incremented on every mutation.
    pub version: u64,
}

impl SceneGraph {
    /// Create a new empty scene graph using the real system clock.
    pub fn new(width: f32, height: f32) -> Self {
        Self::new_with_clock(width, height, Arc::new(SystemClock))
    }

    /// Create a new empty scene graph with an injected clock.
    ///
    /// Prefer this constructor in tests so that time-dependent behaviour
    /// (lease expiry, timestamps) is fully deterministic.
    pub fn new_with_clock(width: f32, height: f32, clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            tabs: HashMap::new(),
            active_tab: None,
            tiles: HashMap::new(),
            nodes: HashMap::new(),
            leases: HashMap::new(),
            hit_region_states: HashMap::new(),
            zone_registry: ZoneRegistry::new(),
            sync_groups: HashMap::new(),
            display_area: Rect::new(0.0, 0.0, width, height),
            version: 0,
        }
    }

    // ─── Tab operations ──────────────────────────────────────────────────

    pub fn create_tab(&mut self, name: &str, display_order: u32) -> Result<SceneId, ValidationError> {
        if name.is_empty() {
            return Err(ValidationError::InvalidField {
                field: "name".into(),
                reason: "tab name must be non-empty".into(),
            });
        }
        // Check display_order uniqueness
        if self.tabs.values().any(|t| t.display_order == display_order) {
            return Err(ValidationError::DuplicateDisplayOrder { order: display_order });
        }
        let id = SceneId::new();
        let now_ms = self.clock.now_millis();
        self.tabs.insert(
            id,
            Tab {
                id,
                name: name.to_string(),
                display_order,
                created_at_ms: now_ms,
            },
        );
        if self.active_tab.is_none() {
            self.active_tab = Some(id);
        }
        self.version += 1;
        Ok(id)
    }

    pub fn switch_active_tab(&mut self, tab_id: SceneId) -> Result<(), ValidationError> {
        if !self.tabs.contains_key(&tab_id) {
            return Err(ValidationError::TabNotFound { id: tab_id });
        }
        self.active_tab = Some(tab_id);
        self.version += 1;
        Ok(())
    }

    // ─── Lease operations ────────────────────────────────────────────────

    pub fn grant_lease(
        &mut self,
        namespace: &str,
        ttl_ms: u64,
        capabilities: Vec<Capability>,
    ) -> SceneId {
        let id = SceneId::new();
        let now_ms = self.clock.now_millis();
        self.leases.insert(
            id,
            Lease {
                id,
                namespace: namespace.to_string(),
                granted_at_ms: now_ms,
                ttl_ms,
                capabilities,
                resource_budget: ResourceBudget::default(),
            },
        );
        self.version += 1;
        id
    }

    pub fn revoke_lease(&mut self, lease_id: SceneId) -> Result<(), ValidationError> {
        if self.leases.remove(&lease_id).is_none() {
            return Err(ValidationError::LeaseNotFound { id: lease_id });
        }
        // Remove all tiles associated with this lease
        let orphaned_tiles: Vec<SceneId> = self
            .tiles
            .values()
            .filter(|t| t.lease_id == lease_id)
            .map(|t| t.id)
            .collect();
        for tile_id in orphaned_tiles {
            self.remove_tile_and_nodes(tile_id);
        }
        self.version += 1;
        Ok(())
    }

    pub fn renew_lease(&mut self, lease_id: SceneId, new_ttl_ms: u64) -> Result<(), ValidationError> {
        let lease = self
            .leases
            .get_mut(&lease_id)
            .ok_or(ValidationError::LeaseNotFound { id: lease_id })?;
        lease.granted_at_ms = self.clock.now_millis();
        lease.ttl_ms = new_ttl_ms;
        self.version += 1;
        Ok(())
    }

    /// Expire all leases past their TTL. Returns IDs of expired leases.
    pub fn expire_leases(&mut self) -> Vec<SceneId> {
        let now = self.clock.now_millis();
        let expired: Vec<SceneId> = self
            .leases
            .values()
            .filter(|l| l.is_expired(now))
            .map(|l| l.id)
            .collect();
        for id in &expired {
            // Remove tiles owned by this lease
            let orphaned_tiles: Vec<SceneId> = self
                .tiles
                .values()
                .filter(|t| t.lease_id == *id)
                .map(|t| t.id)
                .collect();
            for tile_id in orphaned_tiles {
                self.remove_tile_and_nodes(tile_id);
            }
            self.leases.remove(id);
        }
        if !expired.is_empty() {
            self.version += 1;
        }
        expired
    }

    // ─── Tile operations ─────────────────────────────────────────────────

    pub fn create_tile(
        &mut self,
        tab_id: SceneId,
        namespace: &str,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
    ) -> Result<SceneId, ValidationError> {
        // Validate tab exists
        if !self.tabs.contains_key(&tab_id) {
            return Err(ValidationError::TabNotFound { id: tab_id });
        }
        // Validate lease exists
        if !self.leases.contains_key(&lease_id) {
            return Err(ValidationError::LeaseNotFound { id: lease_id });
        }
        // Validate bounds
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Err(ValidationError::InvalidField {
                field: "bounds".into(),
                reason: "width and height must be > 0".into(),
            });
        }

        let id = SceneId::new();
        self.tiles.insert(
            id,
            Tile {
                id,
                tab_id,
                namespace: namespace.to_string(),
                lease_id,
                bounds,
                z_order,
                opacity: 1.0,
                input_mode: InputMode::Capture,
                sync_group: None,
                present_at: None,
                expires_at: None,
                resource_budget: ResourceBudget::default(),
                root_node: None,
            },
        );
        self.version += 1;
        Ok(id)
    }

    pub fn set_tile_root(&mut self, tile_id: SceneId, node: Node) -> Result<(), ValidationError> {
        // Get old root first, then release the borrow
        let old_root = {
            let tile = self
                .tiles
                .get(&tile_id)
                .ok_or(ValidationError::TileNotFound { id: tile_id })?;
            tile.root_node
        };

        // Remove old root and its subtree if present
        if let Some(old_root_id) = old_root {
            self.remove_node_tree(old_root_id);
        }

        let node_id = node.id;

        // Initialize hit region local state if applicable
        if let NodeData::HitRegion(_) = &node.data {
            self.hit_region_states
                .insert(node_id, HitRegionLocalState::new(node_id));
        }

        // Insert the node and all children recursively
        self.insert_node_tree(&node);

        // Set the new root on the tile
        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.root_node = Some(node_id);

        self.version += 1;
        Ok(())
    }

    pub fn add_node_to_tile(
        &mut self,
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
    ) -> Result<(), ValidationError> {
        if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }

        let node_id = node.id;

        // If parent specified, add as child
        if let Some(pid) = parent_id {
            let parent = self
                .nodes
                .get_mut(&pid)
                .ok_or(ValidationError::NodeNotFound { id: pid })?;
            parent.children.push(node_id);
        } else {
            // Set as root if no root exists
            let tile = self.tiles.get_mut(&tile_id).unwrap();
            if tile.root_node.is_none() {
                tile.root_node = Some(node_id);
            }
        }

        // Track hit region state
        if let NodeData::HitRegion(_) = &node.data {
            self.hit_region_states
                .insert(node_id, HitRegionLocalState::new(node_id));
        }

        self.insert_node_tree(&node);
        self.version += 1;
        Ok(())
    }

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
        let created_at_us = now_micros();
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
        if !self.sync_groups.contains_key(&group_id) {
            return Err(ValidationError::SyncGroupNotFound { id: group_id });
        }
        // Release all member tiles from the group
        for tile in self.tiles.values_mut() {
            if tile.sync_group == Some(group_id) {
                tile.sync_group = None;
            }
        }
        self.sync_groups.remove(&group_id);
        self.version += 1;
        Ok(())
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
        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.sync_group = Some(group_id);

        // Add to the group's member set
        self.sync_groups
            .get_mut(&group_id)
            .unwrap()
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
            let tile = self.tiles.get_mut(&tile_id).unwrap();
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
    pub fn evaluate_sync_group_commit(
        &mut self,
        group_id: SceneId,
        tiles_with_pending: &std::collections::BTreeSet<SceneId>,
    ) -> Result<SyncGroupCommitDecision, ValidationError> {
        let group = self
            .sync_groups
            .get(&group_id)
            .ok_or(ValidationError::SyncGroupNotFound { id: group_id })?;

        match group.commit_policy {
            SyncCommitPolicy::AvailableMembers => {
                // Apply whatever is ready — never defers
                let ready: Vec<SceneId> = group
                    .members
                    .iter()
                    .filter(|id| tiles_with_pending.contains(id))
                    .copied()
                    .collect();
                Ok(SyncGroupCommitDecision::Commit { tiles: ready })
            }
            SyncCommitPolicy::AllOrDefer => {
                let all_ready = group.members.iter().all(|id| tiles_with_pending.contains(id));
                if all_ready {
                    // Reset deferral counter and commit all members
                    let tiles: Vec<SceneId> = group.members.iter().copied().collect();
                    self.sync_groups.get_mut(&group_id).unwrap().deferral_count = 0;
                    Ok(SyncGroupCommitDecision::Commit { tiles })
                } else if group.deferral_count < group.max_deferrals {
                    // Defer: increment counter
                    self.sync_groups.get_mut(&group_id).unwrap().deferral_count += 1;
                    Ok(SyncGroupCommitDecision::Defer)
                } else {
                    // Force-commit with available members after exhausting deferrals
                    let tiles: Vec<SceneId> = group
                        .members
                        .iter()
                        .filter(|id| tiles_with_pending.contains(id))
                        .copied()
                        .collect();
                    self.sync_groups.get_mut(&group_id).unwrap().deferral_count = 0;
                    Ok(SyncGroupCommitDecision::ForceCommit { tiles })
                }
            }
        }
    }

    /// Return the number of sync groups in the scene.
    pub fn sync_group_count(&self) -> usize {
        self.sync_groups.len()
    }

    // ─── Node tree helpers ───────────────────────────────────────────────

    fn insert_node_tree(&mut self, node: &Node) {
        // Insert children first (depth-first)
        for child_id in &node.children {
            // Children should already be in the node or will be added separately
            // For the vertical slice, nodes are self-contained with their children
            let _ = child_id;
        }
        self.nodes.insert(node.id, node.clone());
    }

    pub(crate) fn remove_node_tree(&mut self, node_id: SceneId) {
        if let Some(node) = self.nodes.remove(&node_id) {
            for child_id in &node.children {
                self.remove_node_tree(*child_id);
            }
        }
        self.hit_region_states.remove(&node_id);
    }

    pub(crate) fn remove_tile_and_nodes(&mut self, tile_id: SceneId) {
        if let Some(tile) = self.tiles.remove(&tile_id)
            && let Some(root_id) = tile.root_node
        {
            self.remove_node_tree(root_id);
        }
    }

    // ─── Queries ─────────────────────────────────────────────────────────

    /// Get all tiles on the active tab, sorted by z_order (back to front).
    pub fn visible_tiles(&self) -> Vec<&Tile> {
        let active = match self.active_tab {
            Some(id) => id,
            None => return vec![],
        };
        let mut tiles: Vec<&Tile> = self
            .tiles
            .values()
            .filter(|t| t.tab_id == active)
            .collect();
        tiles.sort_by_key(|t| t.z_order);
        tiles
    }

    /// Find the node at a given point, returning (tile_id, node_id) for hit-test.
    /// Searches front-to-back (highest z_order first).
    pub fn hit_test(&self, x: f32, y: f32) -> Option<(SceneId, SceneId)> {
        let active = self.active_tab?;
        let mut tiles: Vec<&Tile> = self
            .tiles
            .values()
            .filter(|t| t.tab_id == active && t.input_mode != InputMode::Passthrough)
            .collect();
        // Sort highest z_order first for front-to-back traversal
        tiles.sort_by(|a, b| b.z_order.cmp(&a.z_order));

        for tile in tiles {
            // Transform point to tile-local coordinates
            let local_x = x - tile.bounds.x;
            let local_y = y - tile.bounds.y;

            if !tile.bounds.contains_point(x, y) {
                continue;
            }

            // Check hit regions within this tile (depth-first, front-to-back)
            if let Some(root_id) = tile.root_node
                && let Some(node_id) = self.hit_test_node(root_id, local_x, local_y)
            {
                return Some((tile.id, node_id));
            }

            // If the tile itself was hit (but no specific node), return tile-level hit
            return Some((tile.id, tile.id));
        }
        None
    }

    fn hit_test_node(&self, node_id: SceneId, x: f32, y: f32) -> Option<SceneId> {
        let node = self.nodes.get(&node_id)?;

        // Check children in reverse order (last child = front-most)
        for child_id in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_node(*child_id, x, y) {
                return Some(hit);
            }
        }

        // Check this node
        match &node.data {
            NodeData::HitRegion(hr) if hr.accepts_pointer && hr.bounds.contains_point(x, y) => {
                Some(node_id)
            }
            _ => None,
        }
    }

    // ─── Zone operations ─────────────────────────────────────────────────

    /// Register a zone definition in the zone registry.
    pub fn register_zone(&mut self, zone: ZoneDefinition) {
        self.zone_registry.register(zone);
        self.version += 1;
    }

    /// Unregister a zone by name. Returns the removed definition if found.
    pub fn unregister_zone(&mut self, name: &str) -> Option<ZoneDefinition> {
        let removed = self.zone_registry.unregister(name);
        if removed.is_some() {
            self.version += 1;
        }
        removed
    }

    /// Publish content to a zone. Applies contention policy.
    ///
    /// Token validation is out-of-scope for the pure scene graph layer;
    /// callers (e.g., the gRPC server) must validate the token before calling this.
    pub fn publish_to_zone(
        &mut self,
        zone_name: &str,
        content: ZoneContent,
        publisher_namespace: &str,
        merge_key: Option<String>,
    ) -> Result<(), ValidationError> {
        // Check zone exists and content type is accepted
        let (contention_policy, max_publishers, accepted) = {
            let zone = self
                .zone_registry
                .get_by_name(zone_name)
                .ok_or_else(|| ValidationError::ZoneNotFound { name: zone_name.to_string() })?;
            let accepted = Self::content_media_type(&content)
                .map(|mt| zone.accepted_media_types.contains(&mt))
                .unwrap_or(true);
            (zone.contention_policy, zone.max_publishers, accepted)
        };

        if !accepted {
            return Err(ValidationError::ZoneMediaTypeMismatch {
                zone: zone_name.to_string(),
            });
        }

        let now_ms = self.clock.now_millis();
        let record = ZonePublishRecord {
            zone_name: zone_name.to_string(),
            publisher_namespace: publisher_namespace.to_string(),
            content,
            published_at_ms: now_ms,
            merge_key: merge_key.clone(),
        };

        let publishes = self
            .zone_registry
            .active_publishes
            .entry(zone_name.to_string())
            .or_default();

        match contention_policy {
            ContentionPolicy::LatestWins => {
                // Replace all with the single new record
                *publishes = vec![record];
            }
            ContentionPolicy::Replace => {
                // Single occupant: evict current and replace
                *publishes = vec![record];
            }
            ContentionPolicy::Stack { max_depth } => {
                // Check publisher count limit
                let publisher_count = publishes
                    .iter()
                    .filter(|r| r.publisher_namespace == publisher_namespace)
                    .count() as u32;
                if publisher_count >= max_publishers {
                    return Err(ValidationError::ZoneMaxPublishersReached {
                        zone: zone_name.to_string(),
                        max: max_publishers,
                    });
                }
                publishes.push(record);
                // Trim oldest if stack exceeds max_depth
                let max = max_depth as usize;
                if publishes.len() > max {
                    let excess = publishes.len() - max;
                    publishes.drain(0..excess);
                }
            }
            ContentionPolicy::MergeByKey { max_keys } => {
                let key = merge_key.clone().unwrap_or_default();
                // Replace existing entry with same key
                if let Some(pos) = publishes.iter().position(|r| {
                    r.merge_key.as_deref().unwrap_or("") == key.as_str()
                }) {
                    publishes[pos] = record;
                } else {
                    // Check key count limit
                    if publishes.len() >= max_keys as usize {
                        return Err(ValidationError::ZoneMaxKeysReached {
                            zone: zone_name.to_string(),
                            max: max_keys as u32,
                        });
                    }
                    publishes.push(record);
                }
            }
        }

        self.version += 1;
        Ok(())
    }

    /// Clear all active publishes for a zone.
    pub fn clear_zone(&mut self, zone_name: &str) -> Result<(), ValidationError> {
        if !self.zone_registry.zones.contains_key(zone_name) {
            return Err(ValidationError::ZoneNotFound { name: zone_name.to_string() });
        }
        self.zone_registry.active_publishes.remove(zone_name);
        self.version += 1;
        Ok(())
    }

    /// Map ZoneContent to its ZoneMediaType, if deterministic.
    fn content_media_type(content: &ZoneContent) -> Option<ZoneMediaType> {
        match content {
            ZoneContent::StreamText(_) => Some(ZoneMediaType::StreamText),
            ZoneContent::Notification(_) => Some(ZoneMediaType::ShortTextWithIcon),
            ZoneContent::StatusBar(_) => Some(ZoneMediaType::KeyValuePairs),
            ZoneContent::SolidColor(_) => Some(ZoneMediaType::SolidColor),
        }
    }

    // ─── Queries ─────────────────────────────────────────────────────────

    /// Snapshot the entire scene graph as JSON.
    pub fn snapshot_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a scene graph from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Count total nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Count total tiles in the graph.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
}


fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Decision returned by `SceneGraph::evaluate_sync_group_commit`.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncGroupCommitDecision {
    /// Commit the listed tiles' pending mutations this frame.
    Commit { tiles: Vec<SceneId> },
    /// Defer the entire group to the next frame (AllOrDefer policy).
    Defer,
    /// Force-commit with the listed tiles after exhausting max_deferrals.
    /// The compositor should emit a `sync_group_force_commit` telemetry event.
    ForceCommit { tiles: Vec<SceneId> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    /// Convenience: build a SceneGraph backed by a TestClock starting at t=1000ms.
    fn scene_with_test_clock() -> (SceneGraph, TestClock) {
        let clock = TestClock::new(1_000);
        let scene =
            SceneGraph::new_with_clock(1920.0, 1080.0, Arc::new(clock.clone()));
        (scene, clock)
    }

    #[test]
    fn test_create_scene_with_tab_and_tiles() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        // Create a tab
        let tab_id = scene.create_tab("Main", 0).unwrap();
        assert_eq!(scene.active_tab, Some(tab_id));

        // Grant a lease
        let lease_id = scene.grant_lease(
            "test-agent",
            60_000,
            vec![Capability::CreateTile, Capability::CreateNode],
        );

        // Create two tiles
        let tile1_id = scene
            .create_tile(tab_id, "test-agent", lease_id, Rect::new(10.0, 10.0, 400.0, 300.0), 1)
            .unwrap();

        let tile2_id = scene
            .create_tile(tab_id, "test-agent", lease_id, Rect::new(420.0, 10.0, 400.0, 300.0), 2)
            .unwrap();

        assert_eq!(scene.tile_count(), 2);

        // Add nodes
        let text_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "Hello, tze_hud!".to_string(),
                bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
                font_size_px: 24.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: Some(Rgba::new(0.1, 0.1, 0.2, 1.0)),
                alignment: TextAlign::Center,
                overflow: TextOverflow::Clip,
            }),
        };
        scene.set_tile_root(tile1_id, text_node).unwrap();

        let hit_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(50.0, 50.0, 200.0, 100.0),
                interaction_id: "btn-click".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
            }),
        };
        scene.set_tile_root(tile2_id, hit_node.clone()).unwrap();

        assert_eq!(scene.node_count(), 2);
        assert!(scene.hit_region_states.contains_key(&hit_node.id));
    }

    #[test]
    fn test_hit_test() {
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
                interaction_id: "btn".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
            }),
        };
        scene.set_tile_root(tile_id, hit_node).unwrap();

        // Hit the hit region (tile at 100,100; region at 50,50 within tile = 150,150 global)
        let result = scene.hit_test(200.0, 180.0);
        assert_eq!(result, Some((tile_id, hr_node_id)));

        // Miss the hit region but hit the tile
        let result = scene.hit_test(110.0, 110.0);
        assert_eq!(result, Some((tile_id, tile_id)));

        // Miss everything
        let result = scene.hit_test(10.0, 10.0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_snapshot_roundtrip() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        let json = scene.snapshot_json().unwrap();
        let restored = SceneGraph::from_json(&json).unwrap();

        assert_eq!(scene.tile_count(), restored.tile_count());
        assert_eq!(scene.active_tab, restored.active_tab);
        assert_eq!(scene.version, restored.version);
    }

    #[test]
    fn test_lease_expiry() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // Grant a lease with a 500 ms TTL.
        // Clock is at t=1000; lease expires at t=1500.
        let lease_id = scene.grant_lease("test", 500, vec![Capability::CreateTile]);
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        assert_eq!(scene.tile_count(), 1);

        // Before expiry: clock still at t=1000, lease lives.
        let expired = scene.expire_leases();
        assert_eq!(expired.len(), 0);
        assert_eq!(scene.tile_count(), 1);

        // Advance past the TTL.
        clock.advance(501);
        let expired = scene.expire_leases();
        assert_eq!(expired.len(), 1);
        assert_eq!(scene.tile_count(), 0);
    }

    #[test]
    fn test_tab_created_at_uses_clock() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        assert_eq!(scene.tabs[&tab_id].created_at_ms, 1_000);

        // Advancing the clock does NOT retroactively change existing timestamps.
        clock.advance(100);
        assert_eq!(scene.tabs[&tab_id].created_at_ms, 1_000);
    }

    #[test]
    fn test_renew_lease_uses_clock() {
        let (mut scene, clock) = scene_with_test_clock();
        // Clock at t=1000.
        let lease_id = scene.grant_lease("test", 5_000, vec![]);
        assert_eq!(scene.leases[&lease_id].granted_at_ms, 1_000);

        // Advance clock then renew.
        clock.advance(2_000);
        scene.renew_lease(lease_id, 10_000).unwrap();
        assert_eq!(scene.leases[&lease_id].granted_at_ms, 3_000);
        assert_eq!(scene.leases[&lease_id].ttl_ms, 10_000);
    }

    #[test]
    fn test_lease_revocation_cleans_tiles() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(200.0, 0.0, 100.0, 100.0), 2)
            .unwrap();

        assert_eq!(scene.tile_count(), 2);
        scene.revoke_lease(lease_id).unwrap();
        assert_eq!(scene.tile_count(), 0);
        assert!(scene.leases.is_empty());
    }

    #[test]
    fn test_visible_tiles_sorted_by_z_order() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        scene.create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 5).unwrap();
        scene.create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1).unwrap();
        scene.create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 3).unwrap();

        let visible = scene.visible_tiles();
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].z_order, 1);
        assert_eq!(visible[1].z_order, 3);
        assert_eq!(visible[2].z_order, 5);
    }

    // ─── Zone tests ───────────────────────────────────────────────────────

    fn make_subtitle_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_string(),
            description: "Subtitle overlay".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 48.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 2,
            transport_constraint: None,
            auto_clear_ms: None,
        }
    }

    fn make_notification_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "notifications".to_string(),
            description: "Notification stack".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 3 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: Some(5_000),
        }
    }

    fn make_status_bar_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_string(),
            description: "Status bar".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 8 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
        }
    }

    fn dummy_token() -> ZonePublishToken {
        ZonePublishToken { token: vec![0xDE, 0xAD, 0xBE, 0xEF] }
    }

    #[test]
    fn test_zone_register_unregister() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone = make_subtitle_zone();

        scene.register_zone(zone.clone());
        assert!(scene.zone_registry.get_by_name("subtitle").is_some());

        let removed = scene.unregister_zone("subtitle");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "subtitle");
        assert!(scene.zone_registry.get_by_name("subtitle").is_none());
    }

    #[test]
    fn test_zone_query_by_name() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.register_zone(make_notification_zone());

        let zone = scene.zone_registry.get_by_name("subtitle").unwrap();
        assert_eq!(zone.name, "subtitle");
        assert!(scene.zone_registry.get_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_zone_query_by_media_type() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.register_zone(make_notification_zone());

        let stream_text_zones = scene.zone_registry.zones_accepting(ZoneMediaType::StreamText);
        assert_eq!(stream_text_zones.len(), 1);
        assert_eq!(stream_text_zones[0].name, "subtitle");

        let notif_zones = scene.zone_registry.zones_accepting(ZoneMediaType::ShortTextWithIcon);
        assert_eq!(notif_zones.len(), 1);
        assert_eq!(notif_zones[0].name, "notifications");
    }

    #[test]
    fn test_default_zones_populated() {
        let registry = ZoneRegistry::with_defaults();
        assert!(registry.get_by_name("status-bar").is_some());
        assert!(registry.get_by_name("notification-area").is_some());
        assert!(registry.get_by_name("subtitle").is_some());
    }

    #[test]
    fn test_zone_publish_not_found() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result = scene.publish_to_zone(
            "nonexistent",
            ZoneContent::StreamText("hello".to_string()),
            "agent",
            None,
        );
        assert!(matches!(result, Err(ValidationError::ZoneNotFound { .. })));
    }

    #[test]
    fn test_zone_publish_media_type_mismatch() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone()); // accepts StreamText only

        let result = scene.publish_to_zone(
            "subtitle",
            ZoneContent::Notification(NotificationPayload {
                text: "Hello".to_string(),
                icon: "".to_string(),
                urgency: 1,
            }),
            "agent",
            None,
        );
        assert!(matches!(result, Err(ValidationError::ZoneMediaTypeMismatch { .. })));
    }

    #[test]
    fn test_contention_latest_wins() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        scene.publish_to_zone("subtitle", ZoneContent::StreamText("first".to_string()), "a1", None).unwrap();
        scene.publish_to_zone("subtitle", ZoneContent::StreamText("second".to_string()), "a2", None).unwrap();

        let publishes = scene.zone_registry.active_for_zone("subtitle");
        assert_eq!(publishes.len(), 1);
        assert_eq!(publishes[0].content, ZoneContent::StreamText("second".to_string()));
        assert_eq!(publishes[0].publisher_namespace, "a2");
    }

    #[test]
    fn test_contention_stack() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_notification_zone()); // Stack { max_depth: 3 }

        let notification = |text: &str| ZoneContent::Notification(NotificationPayload {
            text: text.to_string(),
            icon: "".to_string(),
            urgency: 1,
        });

        scene.publish_to_zone("notifications", notification("msg1"), "a1", None).unwrap();
        scene.publish_to_zone("notifications", notification("msg2"), "a2", None).unwrap();
        scene.publish_to_zone("notifications", notification("msg3"), "a3", None).unwrap();

        let publishes = scene.zone_registry.active_for_zone("notifications");
        assert_eq!(publishes.len(), 3);

        // 4th publish should trim the oldest
        scene.publish_to_zone("notifications", notification("msg4"), "a4", None).unwrap();
        let publishes = scene.zone_registry.active_for_zone("notifications");
        assert_eq!(publishes.len(), 3);
        // Oldest (msg1) should be gone, newest (msg4) at end
        if let ZoneContent::Notification(n) = &publishes[0].content {
            assert_eq!(n.text, "msg2");
        } else {
            panic!("expected Notification");
        }
        if let ZoneContent::Notification(n) = &publishes[2].content {
            assert_eq!(n.text, "msg4");
        } else {
            panic!("expected Notification");
        }
    }

    // ─── Sync Group Tests ────────────────────────────────────────────────

    fn make_scene_with_tiles(count: usize) -> (SceneGraph, SceneId, Vec<SceneId>) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);
        let mut tile_ids = Vec::new();
        for i in 0..count {
            let tile_id = scene
                .create_tile(
                    tab_id,
                    "agent",
                    lease_id,
                    Rect::new(i as f32 * 110.0, 0.0, 100.0, 100.0),
                    i as u32,
                )
                .unwrap();
            tile_ids.push(tile_id);
        }
        (scene, tab_id, tile_ids)
    }

    #[test]
    fn test_create_sync_group() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);

        let group_id = scene
            .create_sync_group(
                Some("test-group".to_string()),
                "agent",
                SyncCommitPolicy::AllOrDefer,
                3,
            )
            .unwrap();

        assert_eq!(scene.sync_group_count(), 1);
        let group = scene.sync_groups.get(&group_id).unwrap();
        assert_eq!(group.name, Some("test-group".to_string()));
        assert_eq!(group.owner_namespace, "agent");
        assert_eq!(group.commit_policy, SyncCommitPolicy::AllOrDefer);
        assert_eq!(group.max_deferrals, 3);
        assert!(group.members.is_empty());
    }

    #[test]
    fn test_delete_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);

        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();

        // Join both tiles
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Deleting the group should release tiles
        scene.delete_sync_group(group_id).unwrap();
        assert_eq!(scene.sync_group_count(), 0);

        // Tiles should have no sync_group reference
        assert_eq!(scene.tiles[&tiles[0]].sync_group, None);
        assert_eq!(scene.tiles[&tiles[1]].sync_group, None);
    }

    #[test]
    fn test_delete_nonexistent_sync_group_errors() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);
        let fake_id = SceneId::new();
        let result = scene.delete_sync_group(fake_id);
        assert!(matches!(result, Err(ValidationError::SyncGroupNotFound { .. })));
    }

    #[test]
    fn test_join_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();

        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        assert_eq!(scene.sync_groups[&group_id].members.len(), 2);
        assert_eq!(scene.tiles[&tiles[0]].sync_group, Some(group_id));
        assert_eq!(scene.tiles[&tiles[1]].sync_group, Some(group_id));
    }

    #[test]
    fn test_join_replaces_old_group_membership() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        let group_a = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();
        let group_b = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();

        scene.join_sync_group(tiles[0], group_a).unwrap();
        // Now join a different group — should leave group_a automatically
        scene.join_sync_group(tiles[0], group_b).unwrap();

        assert!(!scene.sync_groups[&group_a].members.contains(&tiles[0]));
        assert!(scene.sync_groups[&group_b].members.contains(&tiles[0]));
        assert_eq!(scene.tiles[&tiles[0]].sync_group, Some(group_b));
    }

    #[test]
    fn test_leave_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();

        scene.join_sync_group(tiles[0], group_id).unwrap();
        assert!(scene.sync_groups[&group_id].members.contains(&tiles[0]));

        scene.leave_sync_group(tiles[0]).unwrap();
        assert!(!scene.sync_groups[&group_id].members.contains(&tiles[0]));
        assert_eq!(scene.tiles[&tiles[0]].sync_group, None);
        // Group still exists after tile leaves
        assert_eq!(scene.sync_group_count(), 1);
    }

    #[test]
    fn test_leave_when_not_in_group_is_noop() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        // No group created — tile has no sync_group; leave should succeed silently
        let result = scene.leave_sync_group(tiles[0]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_available_members_commit_policy() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has a pending mutation
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();

        // AvailableMembers: commit whatever is ready, no deferral
        match decision {
            SyncGroupCommitDecision::Commit { tiles: committed } => {
                assert_eq!(committed, vec![tiles[0]]);
            }
            other => panic!("Expected Commit, got {:?}", other),
        }
    }

    #[test]
    fn test_contention_merge_by_key() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_status_bar_zone()); // MergeByKey { max_keys: 8 }

        let kv = |k: &str, v: &str| {
            let mut entries = std::collections::HashMap::new();
            entries.insert(k.to_string(), v.to_string());
            ZoneContent::StatusBar(StatusBarPayload { entries })
        };

        // Publish with different keys
        scene.publish_to_zone("status-bar", kv("clock", "12:00"), "a1", Some("clock".to_string())).unwrap();
        scene.publish_to_zone("status-bar", kv("battery", "80%"), "a2", Some("battery".to_string())).unwrap();

        let publishes = scene.zone_registry.active_for_zone("status-bar");
        assert_eq!(publishes.len(), 2);

        // Update existing key "clock"
        scene.publish_to_zone("status-bar", kv("clock", "12:01"), "a1", Some("clock".to_string())).unwrap();
        let publishes = scene.zone_registry.active_for_zone("status-bar");
        assert_eq!(publishes.len(), 2); // Still 2 (clock replaced, battery retained)
        let clock = publishes.iter().find(|r| r.merge_key.as_deref() == Some("clock")).unwrap();
        if let ZoneContent::StatusBar(sb) = &clock.content {
            assert_eq!(sb.entries["clock"], "12:01");
        } else {
            panic!("expected StatusBar");
        }
    }

    #[test]
    fn test_contention_replace() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone = ZoneDefinition {
            id: SceneId::new(),
            name: "pip".to_string(),
            description: "Picture in picture".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.80,
                y_pct: 0.80,
                width_pct: 0.18,
                height_pct: 0.18,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
        };
        scene.register_zone(zone);

        scene.publish_to_zone("pip", ZoneContent::SolidColor(Rgba::WHITE), "a1", None).unwrap();
        scene.publish_to_zone("pip", ZoneContent::SolidColor(Rgba::BLACK), "a2", None).unwrap();

        let publishes = scene.zone_registry.active_for_zone("pip");
        assert_eq!(publishes.len(), 1);
        assert_eq!(publishes[0].publisher_namespace, "a2");
    }

    #[test]
    fn test_clear_zone() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        scene.publish_to_zone("subtitle", ZoneContent::StreamText("hello".to_string()), "a1", None).unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 1);

        scene.clear_zone("subtitle").unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 0);
    }

    #[test]
    fn test_clear_zone_not_found() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result = scene.clear_zone("nonexistent");
        assert!(matches!(result, Err(ValidationError::ZoneNotFound { .. })));
    }

    #[test]
    fn test_zone_registry_snapshot() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.publish_to_zone("subtitle", ZoneContent::StreamText("hi".to_string()), "a1", None).unwrap();

        let snap = scene.zone_registry.snapshot();
        assert_eq!(snap.zones.len(), 1);
        assert_eq!(snap.active_publishes.len(), 1);
        assert_eq!(snap.active_publishes[0].zone_name, "subtitle");
    }

    #[test]
    fn test_zone_publish_via_mutation_batch() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        use crate::mutation::{MutationBatch, SceneMutation};

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![
                SceneMutation::PublishToZone {
                    zone_name: "subtitle".to_string(),
                    content: ZoneContent::StreamText("batch publish".to_string()),
                    publish_token: dummy_token(),
                    merge_key: None,
                },
            ],
        };

        let result = scene.apply_batch(&batch);
        assert!(result.applied, "batch should be applied");
        let publishes = scene.zone_registry.active_for_zone("subtitle");
        assert_eq!(publishes.len(), 1);
        assert_eq!(publishes[0].content, ZoneContent::StreamText("batch publish".to_string()));
    }

    #[test]
    fn test_clear_zone_via_mutation_batch() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.publish_to_zone("subtitle", ZoneContent::StreamText("hello".to_string()), "a1", None).unwrap();

        use crate::mutation::{MutationBatch, SceneMutation};

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![
                SceneMutation::ClearZone {
                    zone_name: "subtitle".to_string(),
                    publish_token: dummy_token(),
                },
            ],
        };

        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 0);
    }

    #[test]
    fn test_all_or_defer_commits_when_all_ready() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);
        pending.insert(tiles[1]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();

        // All members ready → Commit
        match decision {
            SyncGroupCommitDecision::Commit { tiles: committed } => {
                assert_eq!(committed.len(), 2);
            }
            other => panic!("Expected Commit, got {:?}", other),
        }
        // Deferral counter should be reset to 0
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 0);
    }

    #[test]
    fn test_all_or_defer_defers_when_incomplete() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has a pending mutation
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 1);

        // Second deferral
        let decision2 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision2, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 2);

        // Third deferral
        let decision3 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision3, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 3);
    }

    #[test]
    fn test_all_or_defer_force_commits_after_max_deferrals() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        // max_deferrals = 2
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 2)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has pending mutations — tile[1] is always missing
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        // Frame 1: deferral_count goes 0 → 1
        let d1 = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
        assert_eq!(d1, SyncGroupCommitDecision::Defer);

        // Frame 2: deferral_count goes 1 → 2
        let d2 = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
        assert_eq!(d2, SyncGroupCommitDecision::Defer);

        // Frame 3: deferral_count == max_deferrals (2) → force commit
        let d3 = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
        match d3 {
            SyncGroupCommitDecision::ForceCommit { tiles: committed } => {
                // Only tile[0] should be committed (tile[1] has no pending)
                assert_eq!(committed, vec![tiles[0]]);
            }
            other => panic!("Expected ForceCommit, got {:?}", other),
        }
        // Deferral counter reset after force-commit
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 0);
    }

    #[test]
    fn test_sync_group_namespace_limit() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);

        // Create 16 sync groups (the namespace limit)
        for i in 0..SceneGraph::MAX_SYNC_GROUPS_PER_NAMESPACE {
            scene
                .create_sync_group(
                    Some(format!("group-{}", i)),
                    "agent",
                    SyncCommitPolicy::AllOrDefer,
                    3,
                )
                .unwrap();
        }
        assert_eq!(scene.sync_group_count(), SceneGraph::MAX_SYNC_GROUPS_PER_NAMESPACE);

        // 17th should fail
        let result =
            scene.create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3);
        assert!(matches!(result, Err(ValidationError::SyncGroupLimitExceeded { .. })));

        // A different namespace can still create groups
        let other_group = scene
            .create_sync_group(None, "other-agent", SyncCommitPolicy::AllOrDefer, 3);
        assert!(other_group.is_ok());
    }
}
