use super::*;

impl SceneGraph {
    // ─── Queries ─────────────────────────────────────────────────────────

    /// Get all tiles on the active tab, sorted by z_order (back to front).
    pub fn visible_tiles(&self) -> Vec<&Tile> {
        let active = match self.active_tab {
            Some(id) => id,
            None => return vec![],
        };
        let mut tiles: Vec<&Tile> = self.tiles.values().filter(|t| t.tab_id == active).collect();
        tiles.sort_by_key(|t| t.z_order);
        tiles
    }

    /// Map a 2D display-coordinate point to the deepest interactive element.
    ///
    /// Traversal order (per scene-graph/spec.md §Requirement: Hit-Testing Contract,
    /// RFC 0001 §5.1-5.2, and input-model/spec.md lines 263-274):
    ///
    /// 1. **Chrome layer first** — tiles whose lease has priority 0 are checked
    ///    before any content-layer tile, regardless of z-order.  The first
    ///    non-passthrough chrome tile whose bounds contain the point wins and
    ///    returns [`HitResult::Chrome`].
    /// 2. **Content layer tiles by z-order descending** — remaining (non-chrome)
    ///    tiles sorted highest z-order first.  Passthrough tiles are skipped.
    /// 3. **Within each tile, reverse tree order** — node children visited
    ///    last-first (last sibling = front-most); depth-first.  Only
    ///    [`NodeData::HitRegion`] nodes with `accepts_pointer = true` qualify.
    ///
    /// # Return value
    /// - [`HitResult::Chrome`]   — chrome-layer tile/node absorbed the point.
    /// - [`HitResult::NodeHit`]  — a `HitRegionNode` within a content tile matched.
    /// - [`HitResult::TileHit`]  — the tile absorbed the point but no node matched.
    /// - [`HitResult::Passthrough`] — only passthrough tiles at this coordinate.
    ///
    /// Returns [`HitResult::Passthrough`] when no tile covers the point.
    ///
    /// # Performance
    /// Pure geometry — no GPU involvement.  Target: < 100 µs for 50 tiles
    /// (scene-graph/spec.md line 267, RFC 0001 §10).
    pub fn hit_test(&self, x: f32, y: f32) -> HitResult {
        // ── Chrome drag-handle hit regions (global, chrome-priority) ────────
        for region in &self.overlay.drag_handle_hit_regions {
            if region.hit_region.accepts_pointer && region.bounds.contains_point(x, y) {
                return HitResult::ZoneInteraction {
                    zone_name: "__chrome_drag_handle__".to_string(),
                    published_at_wall_us: 0,
                    publisher_namespace: "runtime".to_string(),
                    interaction_id: region.interaction_id.clone(),
                    kind: ZoneInteractionKind::DragHandle {
                        element_id: region.element_id,
                        element_kind: region.element_kind,
                    },
                };
            }
        }

        // ── Zone hit regions check (global, not tab-specific) ────────────────
        // These are runtime-managed zone hit regions (dismiss/action buttons on
        // notification slots). They are populated by the compositor each frame and
        // do not require agent-owned tiles or an active tab. Check them first so
        // they are always available regardless of active_tab state.
        for region in &self.overlay.zone_hit_regions {
            if region.bounds.contains_point(x, y) {
                return HitResult::ZoneInteraction {
                    zone_name: region.zone_name.clone(),
                    published_at_wall_us: region.published_at_wall_us,
                    publisher_namespace: region.publisher_namespace.clone(),
                    interaction_id: region.interaction_id.clone(),
                    kind: region.kind.clone(),
                };
            }
        }

        let Some(active) = self.active_tab else {
            return HitResult::Passthrough;
        };

        // Gather all tiles on the active tab that cover the point.
        // Partition into chrome (priority-0 lease) and content.
        let mut chrome_tiles: Vec<&Tile> = Vec::new();
        let mut content_tiles: Vec<&Tile> = Vec::new();

        for tile in self.tiles.values().filter(|t| t.tab_id == active) {
            if !tile.bounds.contains_point(x, y) {
                continue;
            }
            let is_chrome = self
                .leases
                .get(&tile.lease_id)
                .map(|l| l.priority == 0)
                .unwrap_or(false);
            if is_chrome {
                chrome_tiles.push(tile);
            } else {
                content_tiles.push(tile);
            }
        }

        // ── Phase 1: Chrome layer ────────────────────────────────────────
        // Sort chrome tiles highest z-order first; passthrough chrome tiles
        // do NOT block (they are skipped), but a non-passthrough chrome tile
        // wins immediately.
        chrome_tiles.sort_by(|a, b| b.z_order.cmp(&a.z_order));
        for tile in &chrome_tiles {
            if tile.input_mode == InputMode::Passthrough {
                continue;
            }
            // Chrome tile absorbs the hit.  If it has a HitRegionNode, report
            // its node_id as the element_id for richer routing; otherwise use
            // the tile id.
            // Use the displayed (smoothed/lagged) offset when a scroll
            // animation is in flight so pointer mapping matches the rows the
            // renderer drew; falls back to the authoritative offset otherwise
            // (hud-3lynp).
            let (scroll_x, scroll_y) = self.effective_tile_scroll_offset_local(tile.id);
            let local_x = x - tile.bounds.x + scroll_x;
            let local_y = y - tile.bounds.y + scroll_y;
            let element_id = tile
                .root_node
                .and_then(|root| self.hit_test_node(root, local_x, local_y))
                .unwrap_or(tile.id);
            return HitResult::Chrome { element_id };
        }

        // ── Phase 2: Content layer tiles (z-order descending) ────────────
        content_tiles.sort_by(|a, b| b.z_order.cmp(&a.z_order));
        for tile in &content_tiles {
            if tile.input_mode == InputMode::Passthrough {
                continue; // Skip passthrough tiles per spec.
            }
            // Displayed (smoothed/lagged) offset during an in-flight scroll
            // animation; authoritative offset otherwise (hud-3lynp).
            let (scroll_x, scroll_y) = self.effective_tile_scroll_offset_local(tile.id);
            let local_x = x - tile.bounds.x + scroll_x;
            let local_y = y - tile.bounds.y + scroll_y;

            // ── Phase 3: Within the tile — reverse tree order ────────────
            if let Some(root_id) = tile.root_node {
                if let Some(node_id) = self.hit_test_node(root_id, local_x, local_y) {
                    // Retrieve interaction_id from the node (it must be HitRegionNode).
                    let interaction_id = self
                        .nodes
                        .get(&node_id)
                        .and_then(|n| {
                            if let NodeData::HitRegion(hr) = &n.data {
                                Some(hr.interaction_id.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    return HitResult::NodeHit {
                        tile_id: tile.id,
                        node_id,
                        interaction_id,
                    };
                }
            }

            // Tile absorbed the point but no HitRegionNode matched.
            return HitResult::TileHit { tile_id: tile.id };
        }

        // Only passthrough tiles covered the point, or no tiles at all.
        HitResult::Passthrough
    }

    /// Update `HitRegionLocalState` for the given point.
    ///
    /// Called by the input pipeline (Stage 2) immediately after hit-testing to
    /// provide local visual feedback without waiting for the owning agent.
    /// Sets `hovered = true` on the newly-hit node and `hovered = false` on the
    /// previous hover node (if it changed).
    ///
    /// `prev_hover` — the node that was previously hovered (cleared on transition).
    /// `result`     — the current hit-test result.
    ///
    /// Returns the newly-hovered node ID (if any) for the caller to track.
    pub fn update_hover_state(
        &mut self,
        prev_hover: Option<SceneId>,
        result: &HitResult,
    ) -> Option<SceneId> {
        // Clear old hover.
        if let Some(old_id) = prev_hover {
            if let Some(state) = self.hit_region_states.get_mut(&old_id) {
                state.hovered = false;
            }
        }
        // Set new hover.  Use entry().or_insert_with() so that HitRegionNodes
        // inserted directly into `self.nodes` (e.g. in multi-node trees whose
        // children were not routed through `set_tile_root`) still get their
        // local state initialised on first hit rather than silently failing.

        if let HitResult::NodeHit { node_id, .. } = result {
            let state = self
                .hit_region_states
                .entry(*node_id)
                .or_insert_with(|| HitRegionLocalState::new(*node_id));
            state.hovered = true;
            Some(*node_id)
        } else {
            None
        }
    }

    /// Update pressed state for a node.
    ///
    /// Call with `pressed = true` on PointerDown and `pressed = false` on
    /// PointerUp / capture release.  No-op if the node has no local state entry.
    pub fn update_pressed_state(&mut self, node_id: SceneId, pressed: bool) {
        if let Some(state) = self.hit_region_states.get_mut(&node_id) {
            state.pressed = pressed;
        }
    }

    /// Update focused state for a node.
    ///
    /// The focus state machine is owned by the input epic; this helper allows
    /// the compositor to reflect focus changes into local state without a full
    /// state-machine transition.
    pub fn update_focused_state(&mut self, node_id: SceneId, focused: bool) {
        if let Some(state) = self.hit_region_states.get_mut(&node_id) {
            state.focused = focused;
        }
    }

    /// Set hover state for a chrome drag handle interaction id.
    pub fn set_drag_handle_hovered(&mut self, interaction_id: &str, hovered: bool) {
        if let Some(state) = self.overlay.drag_handle_states.get_mut(interaction_id) {
            state.hovered = hovered;
        } else {
            self.overlay
                .drag_handle_states
                .entry(interaction_id.to_string())
                .or_default()
                .hovered = hovered;
        }
    }

    /// Mark an element as actively being dragged (show visual feedback).
    pub fn set_drag_active(&mut self, element_id: SceneId) {
        self.overlay.drag_active_elements.insert(element_id);
    }

    /// Clear the active drag mark for an element.
    pub fn clear_drag_active(&mut self, element_id: SceneId) {
        self.overlay.drag_active_elements.remove(&element_id);
    }

    /// Returns `true` if the element is currently being dragged.
    pub fn is_drag_active(&self, element_id: SceneId) -> bool {
        self.overlay.drag_active_elements.contains(&element_id)
    }

    /// Set pressed state for a chrome drag handle interaction id.
    pub fn set_drag_handle_pressed(&mut self, interaction_id: &str, pressed: bool) {
        if let Some(state) = self.overlay.drag_handle_states.get_mut(interaction_id) {
            state.pressed = pressed;
        } else {
            self.overlay
                .drag_handle_states
                .entry(interaction_id.to_string())
                .or_default()
                .pressed = pressed;
        }
    }

    pub(super) fn hit_test_node(&self, node_id: SceneId, x: f32, y: f32) -> Option<SceneId> {
        let mut visited = HashSet::new();
        self.hit_test_node_inner(node_id, x, y, &mut visited)
    }

    fn hit_test_node_inner(
        &self,
        node_id: SceneId,
        x: f32,
        y: f32,
        visited: &mut HashSet<SceneId>,
    ) -> Option<SceneId> {
        if !visited.insert(node_id) {
            // Cycle detected — skip this node to avoid infinite recursion.
            #[cfg(debug_assertions)]
            eprintln!(
                "[tze_hud_scene] cycle detected in node graph at {node_id:?} during hit_test_node"
            );
            return None;
        }
        let node = self.nodes.get(&node_id)?;

        // Check children in reverse order (last child = front-most) — depth first.
        for child_id in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_node_inner(*child_id, x, y, visited) {
                return Some(hit);
            }
        }

        // Check this node — only HitRegionNode with accepts_pointer qualifies.
        match &node.data {
            NodeData::HitRegion(hr) if hr.accepts_pointer && hr.bounds.contains_point(x, y) => {
                Some(node_id)
            }
            _ => None,
        }
    }

    /// Returns `true` if `target_id` is reachable from `root_id` in the node graph.
    ///
    /// Uses a visited set to guard against cycles — if a cycle is detected, traversal
    /// returns early rather than recursing indefinitely.
    pub(crate) fn is_node_in_subtree(&self, root_id: SceneId, target_id: SceneId) -> bool {
        let mut visited = HashSet::new();
        self.is_node_in_subtree_inner(root_id, target_id, &mut visited)
    }

    fn is_node_in_subtree_inner(
        &self,
        node_id: SceneId,
        target_id: SceneId,
        visited: &mut HashSet<SceneId>,
    ) -> bool {
        if !visited.insert(node_id) {
            // Cycle detected — stop traversal.
            #[cfg(debug_assertions)]
            eprintln!(
                "[tze_hud_scene] cycle detected in node graph at {node_id:?} during is_node_in_subtree"
            );
            return false;
        }
        if node_id == target_id {
            return true;
        }
        match self.nodes.get(&node_id) {
            Some(node) => node
                .children
                .iter()
                .any(|c| self.is_node_in_subtree_inner(*c, target_id, visited)),
            None => false,
        }
    }
}
