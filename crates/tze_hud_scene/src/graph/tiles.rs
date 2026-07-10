use super::*;

impl SceneGraph {
    // ─── Tile operations ─────────────────────────────────────────────────

    /// Create a tile. This is the unchecked form used internally for scene construction.
    ///
    /// For agent-facing operations use [`create_tile_checked`] which enforces:
    /// - Lease active + `CreateTiles` + `ModifyOwnTiles` capabilities
    /// - Per-tab tile count limit (1024)
    /// - Bounds positive-size and within-display-area
    /// - z_order < ZONE_TILE_Z_MIN
    pub fn create_tile(
        &mut self,
        tab_id: SceneId,
        namespace: &str,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
    ) -> Result<SceneId, ValidationError> {
        self.create_tile_impl(tab_id, namespace, lease_id, bounds, z_order, false)
    }

    /// Create a tile with full spec-compliant validation including capability checks.
    ///
    /// RFC 0001 §2.3, §3.1, §3.3: requires active lease, `create_tiles`, and
    /// `modify_own_tiles` capabilities. Enforces per-tab tile limit, bounds invariants,
    /// and z_order zone-band reservation.
    pub fn create_tile_checked(
        &mut self,
        tab_id: SceneId,
        namespace: &str,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
    ) -> Result<SceneId, ValidationError> {
        self.create_tile_impl(tab_id, namespace, lease_id, bounds, z_order, true)
    }

    fn create_tile_impl(
        &mut self,
        tab_id: SceneId,
        namespace: &str,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
        enforce_capabilities: bool,
    ) -> Result<SceneId, ValidationError> {
        // Validate tab exists
        if !self.tabs.contains_key(&tab_id) {
            return Err(ValidationError::TabNotFound { id: tab_id });
        }

        if enforce_capabilities {
            // Lease must be active and have create_tiles + modify_own_tiles
            self.require_active_lease(lease_id)?;
            self.require_capability(lease_id, Capability::CreateTiles)?;
            self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

            // Namespace isolation: the caller's namespace must match the lease's namespace.
            // This prevents an agent from creating tiles in another agent's namespace
            // using their own (valid) lease. RFC 0001 §1.2.
            let lease_namespace = self
                .leases
                .get(&lease_id)
                .map(|l| l.namespace.as_str())
                .unwrap_or("");
            if namespace != lease_namespace {
                return Err(ValidationError::NamespaceMismatch {
                    tile_id: lease_id, // use lease_id as context; tile not created yet
                    tile_namespace: lease_namespace.to_string(),
                    agent_namespace: namespace.to_string(),
                });
            }
        } else {
            // Validate lease exists at minimum
            if !self.leases.contains_key(&lease_id) {
                return Err(ValidationError::LeaseNotFound { id: lease_id });
            }
        }

        // Per-tab tile count limit (RFC 0001 §2.1: max 1024 tiles per tab)
        let tiles_in_tab = self.tiles.values().filter(|t| t.tab_id == tab_id).count();
        if tiles_in_tab >= MAX_TILES_PER_TAB {
            return Err(ValidationError::BudgetExceeded {
                resource: format!("tiles_per_tab (limit {MAX_TILES_PER_TAB})"),
            });
        }

        // Bounds: width and height must be > 0 (RFC 0001 §2.3)
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Err(ValidationError::BoundsOutOfRange {
                reason: format!(
                    "tile bounds width ({}) and height ({}) must be > 0.0",
                    bounds.width, bounds.height
                ),
            });
        }

        // Bounds must be fully within the tab display area (RFC 0001 §2.3)
        if !bounds.is_within(&self.display_area) {
            return Err(ValidationError::BoundsOutOfRange {
                reason: format!(
                    "tile bounds ({},{} {}×{}) are not fully within display area ({},{} {}×{})",
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                    self.display_area.x,
                    self.display_area.y,
                    self.display_area.width,
                    self.display_area.height,
                ),
            });
        }

        // z_order must be < ZONE_TILE_Z_MIN for agent-owned tiles (RFC 0001 §2.3)
        if z_order >= ZONE_TILE_Z_MIN {
            return Err(ValidationError::InvalidField {
                field: "z_order".into(),
                reason: format!(
                    "z_order 0x{z_order:08X} is >= ZONE_TILE_Z_MIN (0x{ZONE_TILE_Z_MIN:08X}); reserved for runtime zone tiles"
                ),
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
                visual_hint: crate::lease::TileVisualHint::None,
            },
        );
        self.version += 1;
        Ok(id)
    }

    /// Update the bounds of a tile.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles` capability.
    /// Bounds must be positive and within the display area.
    pub fn update_tile_bounds(
        &mut self,
        tile_id: SceneId,
        bounds: Rect,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        // Viewer geometry authority (hud-lyqun): once the viewer has moved or
        // resized this tile as part of a whole-portal gesture, the adapter no
        // longer controls its bounds. Silently accept the mutation but leave the
        // bounds untouched — the adapter's content updates still apply within the
        // viewer-defined geometry, but its stale client-side layout can never
        // reposition the member and fracture the portal group. Viewer-driven
        // resize/drag write `tile.bounds` directly and so are not affected by
        // this gate; only adapter-originated `UpdateTileBounds` reaches here.
        if self.is_viewer_geometry_locked(tile_id) {
            return Ok(());
        }

        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Err(ValidationError::BoundsOutOfRange {
                reason: format!(
                    "tile bounds width ({}) and height ({}) must be > 0.0",
                    bounds.width, bounds.height
                ),
            });
        }
        if !bounds.is_within(&self.display_area) {
            return Err(ValidationError::BoundsOutOfRange {
                reason: format!(
                    "tile bounds ({},{} {}×{}) are not fully within display area",
                    bounds.x, bounds.y, bounds.width, bounds.height
                ),
            });
        }

        let tile = self
            .tiles
            .get_mut(&tile_id)
            .expect("tile_id existence verified by get_tile_lease_checked");
        tile.bounds = bounds;
        self.version += 1;
        Ok(())
    }

    /// Update the z-order of a tile.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles`.
    /// z_order must be < ZONE_TILE_Z_MIN.
    pub fn update_tile_z_order(
        &mut self,
        tile_id: SceneId,
        z_order: u32,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        if z_order >= ZONE_TILE_Z_MIN {
            return Err(ValidationError::InvalidField {
                field: "z_order".into(),
                reason: format!(
                    "z_order 0x{z_order:08X} is >= ZONE_TILE_Z_MIN (0x{ZONE_TILE_Z_MIN:08X}); reserved for runtime zone tiles"
                ),
            });
        }

        let tile = self
            .tiles
            .get_mut(&tile_id)
            .expect("tile_id existence verified by get_tile_lease_checked");
        tile.z_order = z_order;
        self.version += 1;
        Ok(())
    }

    /// Update the opacity of a tile.
    ///
    /// RFC 0001 §2.3: opacity must be in [0.0, 1.0]. Requires active lease + `ModifyOwnTiles`.
    pub fn update_tile_opacity(
        &mut self,
        tile_id: SceneId,
        opacity: f32,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        if !(0.0..=1.0).contains(&opacity) {
            return Err(ValidationError::InvalidField {
                field: "opacity".into(),
                reason: format!("opacity {opacity} is not in [0.0, 1.0]"),
            });
        }

        let tile = self
            .tiles
            .get_mut(&tile_id)
            .expect("tile_id existence verified by get_tile_lease_checked");
        tile.opacity = opacity;
        self.version += 1;
        Ok(())
    }

    /// Update the input mode of a tile.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles`.
    pub fn update_tile_input_mode(
        &mut self,
        tile_id: SceneId,
        input_mode: InputMode,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        let tile = self
            .tiles
            .get_mut(&tile_id)
            .expect("tile_id existence verified by get_tile_lease_checked");
        tile.input_mode = input_mode;
        self.version += 1;
        Ok(())
    }

    /// Update the expiry timestamp of a tile.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles`.
    pub fn update_tile_expiry(
        &mut self,
        tile_id: SceneId,
        expires_at: Option<u64>,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        let tile = self
            .tiles
            .get_mut(&tile_id)
            .expect("tile_id existence verified by get_tile_lease_checked");
        tile.expires_at = expires_at;
        self.version += 1;
        Ok(())
    }

    /// Delete a tile and all its nodes.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles`. Namespace isolation enforced.
    pub fn delete_tile(
        &mut self,
        tile_id: SceneId,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        // Leave sync group before removing the tile to avoid dangling member entries.
        let _ = self.leave_sync_group(tile_id);
        self.remove_tile_and_nodes(tile_id);
        self.version += 1;
        Ok(())
    }

    /// Get the lease ID for a tile, enforcing namespace isolation.
    ///
    /// Returns `NamespaceMismatch` if the tile belongs to a different namespace.
    /// Returns `TileNotFound` if the tile does not exist.
    fn get_tile_lease_checked(
        &self,
        tile_id: SceneId,
        agent_namespace: &str,
    ) -> Result<SceneId, ValidationError> {
        let tile = self
            .tiles
            .get(&tile_id)
            .ok_or(ValidationError::TileNotFound { id: tile_id })?;
        if tile.namespace != agent_namespace {
            return Err(ValidationError::NamespaceMismatch {
                tile_id,
                tile_namespace: tile.namespace.clone(),
                agent_namespace: agent_namespace.to_string(),
            });
        }
        Ok(tile.lease_id)
    }

    pub fn set_tile_root(&mut self, tile_id: SceneId, node: Node) -> Result<(), ValidationError> {
        self.set_tile_root_impl(tile_id, node, None)
    }

    /// Set tile root with full capability and node-count enforcement.
    pub fn set_tile_root_checked(
        &mut self,
        tile_id: SceneId,
        node: Node,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        self.set_tile_root_impl(tile_id, node, Some(agent_namespace))
    }

    fn set_tile_root_impl(
        &mut self,
        tile_id: SceneId,
        node: Node,
        agent_namespace: Option<&str>,
    ) -> Result<(), ValidationError> {
        if let Some(ns) = agent_namespace {
            let lease_id = self.get_tile_lease_checked(tile_id, ns)?;
            self.require_active_lease(lease_id)?;
            self.require_capability(lease_id, Capability::ModifyOwnTiles)?;
        }

        // Check for duplicate node ID (scene-globally unique per RFC 0001 §2.1)
        if self.nodes.contains_key(&node.id) {
            return Err(ValidationError::DuplicateId { id: node.id });
        }

        // Validate node data constraints (e.g. TextMarkdownNode content size limit)
        if let Some(err) = validate_text_markdown_node_data(&node.data) {
            return Err(err);
        }

        // Enforce resource registration for agent-submitted StaticImageNode mutations.
        // Same gate as add_node_to_tile_impl; see that function's comment for spec refs.
        if agent_namespace.is_some() {
            if let NodeData::StaticImage(ref si) = node.data {
                if !self.registered_resources.contains_key(&si.resource_id) {
                    return Err(ValidationError::ResourceNotFound { id: si.resource_id });
                }
            }
        }

        // Node count limit: SetTileRoot replaces the whole tree.
        // Count nodes in the incoming tree (simple count; children are flat in our model).
        let incoming_count = self.count_node_tree_deep(&node);
        if incoming_count > MAX_NODES_PER_TILE {
            return Err(ValidationError::NodeCountExceeded {
                tile_id,
                current: incoming_count,
                limit: MAX_NODES_PER_TILE,
            });
        }

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
        let tile = self
            .tiles
            .get_mut(&tile_id)
            .expect("tile_id existence verified earlier in set_tile_root_impl");
        tile.root_node = Some(node_id);

        // Replacing the root subtree removes the previous node tree, so any portal
        // surface part still pointing at a removed node would dangle. Prune those
        // stale refs back to `None` (the adapter re-binds on its next
        // SetPortalSurface) so consumers never resolve a stale SceneId
        // (hud-tc153 review P2).
        self.revalidate_portal_surface_part_nodes(tile_id);

        self.version += 1;
        Ok(())
    }

    pub fn add_node_to_tile(
        &mut self,
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
    ) -> Result<(), ValidationError> {
        self.add_node_to_tile_impl(tile_id, parent_id, node, None)
    }

    /// Add a node to a tile with full spec-compliant validation.
    pub fn add_node_to_tile_checked(
        &mut self,
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        self.add_node_to_tile_impl(tile_id, parent_id, node, Some(agent_namespace))
    }

    fn add_node_to_tile_impl(
        &mut self,
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
        agent_namespace: Option<&str>,
    ) -> Result<(), ValidationError> {
        if let Some(ns) = agent_namespace {
            let lease_id = self.get_tile_lease_checked(tile_id, ns)?;
            self.require_active_lease(lease_id)?;
            self.require_capability(lease_id, Capability::ModifyOwnTiles)?;
        } else if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }

        // Check for duplicate node ID (RFC 0001 §2.1: NodeIds must be scene-globally unique)
        if self.nodes.contains_key(&node.id) {
            return Err(ValidationError::DuplicateId { id: node.id });
        }

        // Validate node data constraints (e.g. TextMarkdownNode content size limit)
        if let Some(err) = validate_text_markdown_node_data(&node.data) {
            return Err(err);
        }

        // Enforce resource registration for agent-submitted StaticImageNode mutations.
        //
        // Per spec resource-store/spec.md §Requirement: Resource Upload Before Tile
        // Creation: "Any agent-submitted tile mutation that references a ResourceId not
        // present in the resource store MUST be rejected."
        //
        // Only enforced for agent-submitted paths (agent_namespace.is_some()).
        // Internal/test paths (unchecked variants, snapshot restore) bypass this gate.
        if agent_namespace.is_some() {
            if let NodeData::StaticImage(ref si) = node.data {
                if !self.registered_resources.contains_key(&si.resource_id) {
                    return Err(ValidationError::ResourceNotFound { id: si.resource_id });
                }
            }
        }

        // Enforce per-tile node count limit (RFC 0001 §2.1: max 64 nodes)
        let current_count = self.count_nodes_in_tile(
            self.tiles
                .get(&tile_id)
                .expect("tile_id existence verified above in add_node_to_tile_impl"),
        ) as usize;
        if current_count >= MAX_NODES_PER_TILE {
            return Err(ValidationError::NodeCountExceeded {
                tile_id,
                current: current_count,
                limit: MAX_NODES_PER_TILE,
            });
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
            let tile = self
                .tiles
                .get_mut(&tile_id)
                .expect("tile_id existence verified above in add_node_to_tile_impl");
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

    /// Atomically replace the `data` of an existing node (unchecked form).
    ///
    /// The node must already exist in the scene graph and belong to `tile_id`.
    /// The replacement `data` discriminant must match the existing node's discriminant.
    pub fn update_node_content(
        &mut self,
        tile_id: SceneId,
        node_id: SceneId,
        data: NodeData,
    ) -> Result<(), ValidationError> {
        self.update_node_content_impl(tile_id, node_id, data, None)
    }

    /// Atomically replace the `data` of an existing node (checked form).
    ///
    /// Enforces namespace isolation (`agent_namespace` must match the tile's namespace)
    /// and the `ModifyOwnTiles` capability, then delegates to `update_node_content_impl`.
    pub fn update_node_content_checked(
        &mut self,
        tile_id: SceneId,
        node_id: SceneId,
        data: NodeData,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        self.update_node_content_impl(tile_id, node_id, data, Some(agent_namespace))
    }

    fn update_node_content_impl(
        &mut self,
        tile_id: SceneId,
        node_id: SceneId,
        mut data: NodeData,
        agent_namespace: Option<&str>,
    ) -> Result<(), ValidationError> {
        // Stage 4: Lease + capability check (when namespace is provided).
        if let Some(ns) = agent_namespace {
            let lease_id = self.get_tile_lease_checked(tile_id, ns)?;
            self.require_active_lease(lease_id)?;
            self.require_capability(lease_id, Capability::ModifyOwnTiles)?;
        } else if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }

        // Stage 4: Node must exist in the scene graph.
        {
            let existing = self
                .nodes
                .get(&node_id)
                .ok_or(ValidationError::NodeNotFound { id: node_id })?;

            // Stage 4: Node must be reachable from this tile's root.
            let tile = self
                .tiles
                .get(&tile_id)
                .expect("tile_id existence verified above in update_node_content_impl");
            let root = tile
                .root_node
                .ok_or(ValidationError::NodeNotFound { id: node_id })?;
            if !self.is_node_in_subtree(root, node_id) {
                return Err(ValidationError::InvalidField {
                    field: "node_id".into(),
                    reason: format!("node {node_id} does not belong to tile {tile_id}"),
                });
            }

            // Stage 4: Type discriminant must match.
            let type_matches = matches!(
                (&existing.data, &data),
                (NodeData::TextMarkdown(_), NodeData::TextMarkdown(_))
                    | (NodeData::SolidColor(_), NodeData::SolidColor(_))
                    | (NodeData::HitRegion(_), NodeData::HitRegion(_))
                    | (NodeData::StaticImage(_), NodeData::StaticImage(_))
            );
            if !type_matches {
                return Err(ValidationError::InvalidField {
                    field: "data".into(),
                    reason: format!(
                        "cannot change node type: existing node {node_id} has a different variant"
                    ),
                });
            }
        }

        // Content constraints (e.g. markdown byte limit).
        if let Some(err) = validate_text_markdown_node_data(&data) {
            return Err(err);
        }

        // Enforce resource registration for agent-submitted StaticImage content updates.
        //
        // Per spec resource-store/spec.md §Requirement: Resource Upload Before Tile
        // Creation: "Any agent-submitted tile mutation that references a ResourceId not
        // present in the resource store MUST be rejected."
        //
        // This gate closes the bypass where an agent could swap a StaticImageNode to an
        // unregistered resource_id via UpdateNodeContent while passing the add/set_root
        // checks.  Only applied for agent-submitted paths (agent_namespace.is_some()).
        if agent_namespace.is_some() {
            if let NodeData::StaticImage(ref si) = data {
                if !self.registered_resources.contains_key(&si.resource_id) {
                    return Err(ValidationError::ResourceNotFound { id: si.resource_id });
                }
            }
        }

        // Budget re-accounting for StaticImage replacement.
        //
        // Proto ingest always sets `decoded_bytes = 0` on inbound `StaticImageNode`
        // payloads because `decoded_bytes` is runtime-owned metadata that the client
        // must not supply (see `convert.rs`).  If we blindly wrote the incoming zero
        // into the graph the texture-budget tracking in `sum_texture_bytes` /
        // `lease_resource_usage` would under-report actual GPU memory usage after
        // the replacement.
        //
        // Preservation rule:
        //   • Same resource_id AND incoming decoded_bytes == 0 → preserve the
        //     stored decoded_bytes (the image content has not changed; the stored
        //     value is authoritative for budget accounting).
        //   • resource_id changed OR incoming decoded_bytes > 0 → use the incoming
        //     value.  The caller (session server or test) is responsible for
        //     populating decoded_bytes from the resource store when the resource
        //     changes.
        {
            let node = self
                .nodes
                .get_mut(&node_id)
                .expect("node_id existence verified in Stage 4 above");
            if let (NodeData::StaticImage(old_si), NodeData::StaticImage(new_si)) =
                (&node.data, &mut data)
            {
                if new_si.resource_id == old_si.resource_id && new_si.decoded_bytes == 0 {
                    new_si.decoded_bytes = old_si.decoded_bytes;
                }
            }
        }

        // Resource ref-count maintenance for StaticImage content swaps.
        //
        // Extract the old resource_id before re-borrowing mutably, then update
        // ref counts and finally apply the data swap.  The borrow checker requires
        // that the immutable borrow of `node.data` (to read old_id) ends before
        // the mutable borrows of `self` (for dec/inc_resource_ref) begin.
        //
        // This correctly handles:
        //   1. Same resource_id → net zero change; no update needed.
        //   2. Different resource_id → old loses a ref, new gains one.
        let old_resource_id = if let NodeData::StaticImage(ref old_si) = self.nodes[&node_id].data {
            Some(old_si.resource_id)
        } else {
            None
        };
        if let (Some(old_id), NodeData::StaticImage(new_si)) = (old_resource_id, &data) {
            let new_id = new_si.resource_id;
            if old_id != new_id {
                self.dec_resource_ref(&old_id);
                self.inc_resource_ref(new_id);
            }
            // If resource_id is unchanged, ref count is unchanged.
        }

        // Apply the update — replace data in-place, preserving id and children.
        let node = self
            .nodes
            .get_mut(&node_id)
            .expect("node_id existence verified in Stage 4 above");
        node.data = data;
        self.version += 1;
        Ok(())
    }
}

// ─── Helper for TextMarkdownNode content size validation ──────────────────────

/// Validate a TextMarkdownNode's content size.
///
/// Returns `Some(ValidationError)` if the content exceeds `MAX_MARKDOWN_BYTES`.
/// Used by `set_tile_root_impl` when strict content validation is needed.
pub fn validate_text_markdown_node_data(data: &NodeData) -> Option<ValidationError> {
    if let NodeData::TextMarkdown(tm) = data {
        if tm.content.len() > MAX_MARKDOWN_BYTES {
            return Some(ValidationError::InvalidField {
                field: "content".into(),
                reason: format!(
                    "TextMarkdownNode content exceeds {} UTF-8 bytes (got {})",
                    MAX_MARKDOWN_BYTES,
                    tm.content.len()
                ),
            });
        }
    }
    None
}
