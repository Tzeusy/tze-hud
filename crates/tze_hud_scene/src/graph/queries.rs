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

    // ─── Portal frame / header-band resolution (hud-643dv) ───────────────
    //
    // Shared structural portal-group resolution used by BOTH the compositor
    // (to place the header-band drag handle on the frame tile) and the runtime
    // (`resolve_portal_group`, which delegates its anchor pick here so there is
    // a single source of truth for "which tile is the frame").  The rule mirrors
    // #984/#986: the largest-area tile sharing a lease is the frame/anchor; ties
    // break to the lowest id for determinism.

    /// The frame/anchor tile of the portal group that owns `tile_id`: the
    /// largest-area tile sharing its lease (ties broken by lowest id).
    ///
    /// A single-tile lease resolves to itself (a degenerate one-member group).
    /// Returns `None` only if `tile_id` does not exist.
    pub fn portal_anchor_tile(&self, tile_id: SceneId) -> Option<SceneId> {
        let seed = self.tiles.get(&tile_id)?;
        let lease_id = seed.lease_id;
        let mut anchor_id = tile_id;
        let mut anchor_area = seed.bounds.width * seed.bounds.height;
        for (id, tile) in self.tiles.iter() {
            if tile.lease_id != lease_id {
                continue;
            }
            let area = tile.bounds.width * tile.bounds.height;
            if area > anchor_area || (area == anchor_area && *id < anchor_id) {
                anchor_area = area;
                anchor_id = *id;
            }
        }
        Some(anchor_id)
    }

    /// The VISIBLE frame/anchor tile of the portal group that owns `tile_id`:
    /// the largest-area member that is NOT input-passthrough (ties → lowest id).
    ///
    /// The header drag band must live on the interactive frame, never on an
    /// invisible passthrough sibling that shares the frame's bounds — the live
    /// exemplar creates a capture-backstop tile with IDENTICAL bounds to the
    /// frame, and a plain largest-area pick tie-breaks onto whichever was created
    /// first (the backstop, via monotonic ids), stranding the band on an
    /// invisible tile whose node tree lacks the header controls (hud-cpjqe).
    /// Falls back to [`Self::portal_anchor_tile`] only if every member is
    /// passthrough (degenerate — should not happen for a real portal).
    pub fn portal_band_anchor_tile(&self, tile_id: SceneId) -> Option<SceneId> {
        let seed = self.tiles.get(&tile_id)?;
        let lease_id = seed.lease_id;
        let mut anchor: Option<SceneId> = None;
        let mut anchor_area = f32::NEG_INFINITY;
        for (id, tile) in self.tiles.iter() {
            if tile.lease_id != lease_id || tile.input_mode == InputMode::Passthrough {
                continue;
            }
            let area = tile.bounds.width * tile.bounds.height;
            if area > anchor_area || (area == anchor_area && anchor.is_none_or(|a| *id < a)) {
                anchor_area = area;
                anchor = Some(*id);
            }
        }
        anchor.or_else(|| self.portal_anchor_tile(tile_id))
    }

    /// `(anchor_tile_id, header_band_rect)` for every text-stream portal frame on
    /// the scene.
    ///
    /// The band is resolved off the **first-class portal surface** whenever one is
    /// declared for a tile: the declared `Header` part carries real surface-local
    /// bounds (even when its backing `node` is empty), so hit-testing keys off the
    /// surface descriptor rather than inferring a top strip from raw sibling tiles
    /// (hud-m4xay F5 — finishing the promotion's renderer/interaction residual).
    ///
    /// For portals that have NOT declared a surface (the retained raw-tile escape
    /// hatch), it falls back to the legacy heuristic: a "portal" is any lease group
    /// with at least one scrollable member (the same gate
    /// `translate_portal_group_on_drag` uses), a single scrollable tile qualifies
    /// as a degenerate one-member portal, and the band is the top `band_h` strip of
    /// the frame/anchor tile — the visible frame, never an equal-bounds passthrough
    /// capture-backstop (hud-cpjqe). Anchors already covered by a declared surface
    /// are not double-emitted.
    ///
    /// Used by the compositor to emit the header-band drag handle (hud-643dv).
    /// Returned sorted by anchor id so handle ordering is deterministic.
    pub fn portal_header_band_anchors(&self, band_h: f32) -> Vec<(SceneId, Rect)> {
        let mut out: Vec<(SceneId, Rect)> = Vec::new();
        let mut covered: std::collections::HashSet<SceneId> = std::collections::HashSet::new();

        // Preferred source: the declared surface's `Header` part. Its bounds are
        // surface-local, so convert to absolute via the host tile origin and clamp
        // to the tile so a malformed part can never over-extend the band.
        for (tile_id, surface) in self.overlay.portal_surfaces.iter() {
            let Some(tile) = self.tiles.get(tile_id) else {
                continue;
            };
            let Some(header) = surface
                .parts
                .iter()
                .find(|p| p.kind == PortalPartKind::Header)
            else {
                continue;
            };
            if let Some(band) = header_band_rect(tile.bounds, header.bounds) {
                out.push((*tile_id, band));
                covered.insert(*tile_id);
            }
        }

        // Fallback: raw-tile heuristic for portals without a declared surface.
        let mut anchors: std::collections::HashSet<SceneId> = std::collections::HashSet::new();
        for (id, _tile) in self.tiles.iter() {
            // Only scrollable surfaces seed a portal group.
            if self.tile_scroll_config(*id).is_none() {
                continue;
            }
            // Anchor on the VISIBLE frame, not an equal-bounds passthrough
            // capture-backstop (hud-cpjqe).
            if let Some(anchor) = self.portal_band_anchor_tile(*id) {
                if !covered.contains(&anchor) {
                    anchors.insert(anchor);
                }
            }
        }
        for anchor in anchors {
            if let Some(tile) = self.tiles.get(&anchor) {
                // Clamp the band to the tile so a tiny frame never over-extends.
                let h = band_h.min(tile.bounds.height).max(1.0);
                out.push((
                    anchor,
                    Rect::new(tile.bounds.x, tile.bounds.y, tile.bounds.width, h),
                ));
            }
        }

        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
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
                // Header-band handles (hud-643dv) span the whole top strip of a
                // portal frame and legitimately overlap interactive controls
                // (minimize, reply). Windows-titlebar semantics: a control on the
                // band wins; the band drags only empty header space. So a band
                // yields to any `accepts_pointer` HitRegionNode under the point,
                // letting the normal tile/node walk below return that control.
                // Legacy grips keep their original chrome-priority (they never
                // overlap controls), so this only affects band handles.
                if region.is_header_band && self.header_band_yields_to_node(region, x, y) {
                    break;
                }
                return HitResult::ZoneInteraction {
                    zone_name: "__chrome_drag_handle__".to_string(),
                    published_at_wall_us: 0,
                    publisher_namespace: "runtime".to_string(),
                    interaction_id: region.interaction_id.clone(),
                    kind: ZoneInteractionKind::DragHandle {
                        element_id: region.element_id,
                        element_kind: region.element_kind,
                        is_header_band: region.is_header_band,
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

    /// True when a header-band drag handle should yield the point to an
    /// interactive control beneath it (hud-643dv).
    ///
    /// Probes the band's frame tile (`region.element_id`) for an `accepts_pointer`
    /// HitRegionNode under `(x, y)` and yields ONLY when that node's bounds fit
    /// **inside the band rect** (small epsilon). This is true Windows-titlebar
    /// semantics: a titlebar button (e.g. minimize, which fits within the header
    /// strip) beats the drag, but the client area does not reach into the
    /// titlebar.
    ///
    /// The containment gate is essential, not cosmetic: the #981/#987 projection
    /// portal publishes its composer hit-region spanning the WHOLE tile
    /// (`x:0,y:0,w,h`) for click-anywhere-to-focus. On a single-tile projection
    /// portal that region overlaps the band at every point; a blanket "yield to
    /// any pointer node" rule would kill drag on exactly the surface live sessions
    /// use. A full-tile region is taller than the band, so it is not contained and
    /// the band keeps the drag; only header-sized controls yield.
    fn header_band_yields_to_node(&self, region: &DragHandleHitRegion, x: f32, y: f32) -> bool {
        let Some(tile) = self.tiles.get(&region.element_id) else {
            return false;
        };
        let Some(root_id) = tile.root_node else {
            return false;
        };
        let (scroll_x, scroll_y) = self.effective_tile_scroll_offset_local(tile.id);
        let local_x = x - tile.bounds.x + scroll_x;
        let local_y = y - tile.bounds.y + scroll_y;
        let Some(node_id) = self.hit_test_node(root_id, local_x, local_y) else {
            return false;
        };
        // Resolve the hit node's bounds and map them into display space to compare
        // against the (display-space) band rect.
        let Some(node_local) = self.nodes.get(&node_id).and_then(|n| match &n.data {
            NodeData::HitRegion(hr) => Some(hr.bounds),
            _ => None,
        }) else {
            return false;
        };
        let node_disp = Rect::new(
            tile.bounds.x - scroll_x + node_local.x,
            tile.bounds.y - scroll_y + node_local.y,
            node_local.width,
            node_local.height,
        );
        // Yield only when the control fits within the band (titlebar buttons win;
        // full-tile / client-area regions do not).
        const EPS: f32 = 0.5;
        let band = &region.bounds;
        node_disp.x >= band.x - EPS
            && node_disp.y >= band.y - EPS
            && node_disp.x + node_disp.width <= band.x + band.width + EPS
            && node_disp.y + node_disp.height <= band.y + band.height + EPS
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

/// Absolute header-band rect from a portal surface's `Header` part.
///
/// `part` bounds are surface-local (relative to the host `tile`); this translates
/// them to absolute display coordinates and returns the true rectangle
/// intersection with the tile, so the band can never exceed the frame on ANY
/// edge. `validate_structure` requires only non-negative extents (not a
/// non-negative origin), so a malformed `Header` part with a negative local
/// `x`/`y` is clamped to the tile origin here rather than starting the band
/// above/left of the frame. Returns `None` for a degenerate (sub-pixel) band —
/// keeping the drag handle off zero-area headers.
fn header_band_rect(tile: Rect, part: Rect) -> Option<Rect> {
    // Intersect the absolute part rect with the tile rect on all four edges.
    let x0 = (tile.x + part.x).max(tile.x);
    let y0 = (tile.y + part.y).max(tile.y);
    let x1 = (tile.x + part.x + part.width).min(tile.x + tile.width);
    let y1 = (tile.y + part.y + part.height).min(tile.y + tile.height);
    let w = x1 - x0;
    let h = y1 - y0;
    if w < 1.0 || h < 1.0 {
        return None;
    }
    Some(Rect::new(x0, y0, w, h))
}
