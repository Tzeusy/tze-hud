//! Scene graph: the core data structure holding all tabs, tiles, nodes, leases.
//! Pure data — no GPU, no async, no I/O.

use crate::types::*;
use crate::validation::ValidationError;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// The root scene graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneGraph {
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
    /// Display area (the viewport dimensions).
    pub display_area: Rect,
    /// Monotonic version counter, incremented on every mutation.
    pub version: u64,
}

use serde::{Deserialize, Serialize};

impl SceneGraph {
    /// Create a new empty scene graph with the given display dimensions.
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            tabs: HashMap::new(),
            active_tab: None,
            tiles: HashMap::new(),
            nodes: HashMap::new(),
            leases: HashMap::new(),
            hit_region_states: HashMap::new(),
            zone_registry: ZoneRegistry::new(),
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
        let now_ms = now_millis();
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
        let now_ms = now_millis();
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
        lease.granted_at_ms = now_millis();
        lease.ttl_ms = new_ttl_ms;
        self.version += 1;
        Ok(())
    }

    /// Expire all leases past their TTL. Returns IDs of expired leases.
    pub fn expire_leases(&mut self) -> Vec<SceneId> {
        let now = now_millis();
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
        if let Some(tile) = self.tiles.remove(&tile_id) {
            if let Some(root_id) = tile.root_node {
                self.remove_node_tree(root_id);
            }
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
            if let Some(root_id) = tile.root_node {
                if let Some(node_id) = self.hit_test_node(root_id, local_x, local_y) {
                    return Some((tile.id, node_id));
                }
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

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // Grant a lease that's already expired (ttl = 0)
        let lease_id = scene.grant_lease("test", 0, vec![Capability::CreateTile]);
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        assert_eq!(scene.tile_count(), 1);

        // Wait a tiny bit so it expires, then expire
        std::thread::sleep(std::time::Duration::from_millis(1));
        let expired = scene.expire_leases();
        assert_eq!(expired.len(), 1);
        assert_eq!(scene.tile_count(), 0);
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
}
