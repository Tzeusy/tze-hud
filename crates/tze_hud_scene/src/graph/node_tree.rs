use super::SceneGraph;
use crate::types::*;

impl SceneGraph {
    // ─── Node tree helpers ───────────────────────────────────────────────

    pub(super) fn insert_node_tree(&mut self, node: &Node) {
        // Insert children first (depth-first)
        for child_id in &node.children {
            // Children should already be in the node or will be added separately
            // For the vertical slice, nodes are self-contained with their children
            let _ = child_id;
        }
        // Increment the resource ref count if this node references an image resource.
        if let NodeData::StaticImage(ref si) = node.data {
            self.inc_resource_ref(si.resource_id);
        }
        self.nodes.insert(node.id, node.clone());
    }

    pub(crate) fn remove_node_tree(&mut self, node_id: SceneId) {
        if let Some(node) = self.nodes.remove(&node_id) {
            // Decrement the resource ref count if this node referenced an image resource.
            if let NodeData::StaticImage(ref si) = node.data {
                self.dec_resource_ref(&si.resource_id);
            }
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
        self.overlay.tile_scroll_configs.remove(&tile_id);
        self.overlay.tile_scroll_offsets.remove(&tile_id);
        self.overlay.tile_follow_tail_at_tail.remove(&tile_id);
        self.overlay.tile_lifecycle_accents.remove(&tile_id);
        self.overlay.drag_active_elements.remove(&tile_id);
        self.overlay.viewer_geometry_locked.remove(&tile_id);
        // Notify the windowed runtime so it can eagerly prune per-tile state
        // that cannot live in the scene graph (e.g. `portal_resize_states`).
        // Drained by `SceneGraph::drain_removed_tile_ids` in `about_to_wait`.
        self.overlay.recently_removed_tile_ids.push(tile_id);
    }
}
