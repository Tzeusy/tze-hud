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

    /// Scale a locked tile's freshly-published root node tree so its tile-local
    /// extent tracks the tile's viewer-defined content bounds (hud-rpmwt).
    ///
    /// Once the viewer takes geometry authority over a portal member via a
    /// whole-portal resize (`is_viewer_geometry_locked`), the tile-bounds lock
    /// (hud-lyqun) already ignores adapter-originated `UpdateTileBounds`. But a
    /// content republish replaces the whole node tree via [`set_tile_root`], and
    /// the compositor wraps `TextMarkdownNode` text to `node.bounds.width` and
    /// clips to `tile.bounds`. An adapter that republishes its stale attach-time
    /// (config) node geometry would therefore re-home the transcript back to the
    /// old wrap width — overflowing the resized pane *unclipped* and wrapping at
    /// the stale width (the exact hud-rpmwt live repro). Reconciling the new
    /// root's tile-local extent to the tile bounds — the same scaling
    /// `scale_tile_node_tree` applies at resize time — makes the tile the single
    /// wrap-width authority: text re-wraps and clips to the resized pane
    /// regardless of the adapter's published node bounds.
    ///
    /// Node bounds are tile-local and the portal content node fills its tile, so
    /// the target extent is the tile's own `width`/`height`. Scaling (not
    /// clamping) preserves any child node's relative layout, matching the resize
    /// path. A no-op when the tile is not viewer-locked, when the root already
    /// tracks the tile, or when either extent is degenerate.
    ///
    /// [`set_tile_root`]: SceneGraph::set_tile_root
    pub(super) fn reconcile_locked_root_to_tile_bounds(&mut self, tile_id: SceneId) {
        if !self.is_viewer_geometry_locked(tile_id) {
            return;
        }
        let Some((tile_bounds, root_id)) = self
            .tiles
            .get(&tile_id)
            .and_then(|t| t.root_node.map(|r| (t.bounds, r)))
        else {
            return;
        };
        let Some(root_bounds) = self.nodes.get(&root_id).map(|n| n.data.bounds()) else {
            return;
        };
        if root_bounds.width <= 0.0 || root_bounds.height <= 0.0 {
            return;
        }
        let r_w = tile_bounds.width / root_bounds.width;
        let r_h = tile_bounds.height / root_bounds.height;
        // Adapter already published resize-aware bounds → nothing to reconcile.
        // Guards against needless float churn and a tree walk on the common path.
        if (r_w - 1.0).abs() < f32::EPSILON && (r_h - 1.0).abs() < f32::EPSILON {
            return;
        }
        // Collect the subtree ids with an immutable walk first, then mutate —
        // mirrors `scale_tile_node_tree` in the runtime resize path so the two
        // node-scaling sites stay behaviourally identical. The tree is small
        // (≤ MAX_NODES_PER_TILE) and republish is a coalesced state-stream event,
        // not a hot loop.
        let mut stack = vec![root_id];
        let mut ids = Vec::new();
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            ids.push(id);
            stack.extend(node.children.iter().copied());
        }
        for id in ids {
            if let Some(node) = self.nodes.get_mut(&id) {
                let b = node.data.bounds_mut();
                b.x *= r_w;
                b.y *= r_h;
                b.width *= r_w;
                b.height *= r_h;
            }
        }
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
        self.overlay.tile_unread_counts.remove(&tile_id);
        self.overlay.tile_lifecycle_accents.remove(&tile_id);
        self.overlay.portal_surfaces.remove(&tile_id);
        self.overlay.drag_active_elements.remove(&tile_id);
        self.overlay.viewer_geometry_locked.remove(&tile_id);
        self.overlay.tile_font_scale.remove(&tile_id);
        // Notify the windowed runtime so it can eagerly prune per-tile state
        // that cannot live in the scene graph (e.g. `portal_resize_states`).
        // Drained by `SceneGraph::drain_removed_tile_ids` in `about_to_wait`.
        self.overlay.recently_removed_tile_ids.push(tile_id);
    }
}
