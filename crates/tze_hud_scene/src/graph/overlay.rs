use crate::types::*;
use crate::validation::ValidationError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Transient per-frame state owned by the runtime and compositor layers.
///
/// These fields are all `#[serde(skip)]` — they are never serialized and carry
/// no durable scene-model semantics.  They are grouped here so that
/// [`SceneGraph`]'s public surface is not polluted with render-scratch details.
///
/// Owned by the [`SceneGraph`] as `pub overlay: RuntimeOverlayState`.  The
/// compositor populates hit regions each frame; the input processor reads and
/// clears them; the runtime drains pending SVG assets before the render loop.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuntimeOverlayState {
    /// Runtime-managed zone interaction hit regions.
    ///
    /// Populated by the compositor each frame during `render_zone_content`.
    /// Contains the display-space bounds of dismiss (×) buttons and action
    /// buttons for visible notification slots.
    ///
    /// The hit-test pipeline checks this list after tile testing when the
    /// result would otherwise be `Passthrough`, producing a
    /// [`HitResult::ZoneInteraction`] result for zone-owned affordances.
    ///
    /// Ephemeral: skipped during serialization.  Cleared by the compositor
    /// at the start of each frame before zone geometry is recomputed.
    #[serde(skip, default)]
    pub zone_hit_regions: Vec<ZoneHitRegion>,
    /// Runtime-managed chrome drag-handle hit regions.
    ///
    /// Populated by the compositor each frame from currently visible tiles,
    /// active zones, and active widgets. These are runtime-internal affordances
    /// and are never serialized or exposed over agent-facing scene mutations.
    #[serde(skip, default)]
    pub drag_handle_hit_regions: Vec<DragHandleHitRegion>,
    /// Local-first hover/press state keyed by drag-handle interaction id.
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub drag_handle_states: HashMap<String, DragHandleLocalState>,
    /// Set of element_ids that are currently being actively dragged.
    ///
    /// Written by the input processor on DragActivated and cleared on
    /// DragReleased / DragCancelled. Read by the compositor to apply v1-
    /// compatible visual feedback (z-order boost, opacity, 2px highlight border).
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub drag_active_elements: HashSet<SceneId>,
    /// Chrome-layer context menu shown on right-click / short-tap of a drag handle.
    ///
    /// `Some` while the menu is visible; `None` otherwise.  The compositor
    /// renders this overlay and populates `reset_button_rect` for hit-testing.
    /// Auto-dismissed by the windowed runtime after 3 s, or on click-outside.
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub drag_handle_context_menu: Option<crate::types::DragHandleContextMenuState>,
    /// Runtime-registered widget SVG assets awaiting compositor registration.
    ///
    /// Producers (session/MCP runtime registration paths) enqueue validated SVGs
    /// here. Consumers (windowed/headless render loops) drain and register them
    /// with the compositor-side widget renderer before rendering.
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub pending_widget_svg_assets: Vec<(String, String, Vec<u8>)>,
    /// Runtime-owned scroll configs for tiles that opt into local-first scroll.
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub tile_scroll_configs: HashMap<SceneId, TileScrollConfig>,
    /// Runtime-owned local scroll offsets per tile.
    ///
    /// Offsets are absolute from the tile content origin and are applied by
    /// the compositor and hit-testing paths for immediate local feedback.
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub tile_scroll_offsets: HashMap<SceneId, (f32, f32)>,
    /// Displayed (smoothed/lagged) scroll offsets per tile, published by the
    /// compositor each frame while windowed scroll smoothing is active.
    ///
    /// During an in-flight smoothed scroll the renderer draws tile content at
    /// the *displayed* offset from its `ScrollSmoother`, which lags the
    /// authoritative target in [`tile_scroll_offsets`](Self::tile_scroll_offsets).
    /// The hit-test path consults this map (via
    /// [`SceneGraph::effective_tile_scroll_offset_local`]) so pointer queries
    /// map against the same rows the operator actually sees mid-animation
    /// (hud-3lynp).
    ///
    /// Empty in headless/snap mode (smoothing disabled): hit-testing then falls
    /// back to the authoritative offset, leaving deterministic tests unchanged.
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub displayed_tile_scroll_offsets: HashMap<SceneId, (f32, f32)>,
    /// Follow-tail anchor state per tile (used by truncation-mode selection).
    ///
    /// `true` = tile viewport is at the tail of its content stream (use
    /// tail-anchored truncation); `false` = scrolled-back (use head-anchored
    /// truncation, spec task 3.3 append-stability guarantee).
    ///
    /// Defaults to `true` for newly registered tiles (new tiles start at tail).
    /// Set by the runtime layer when `FollowTailAnchor` transitions; read by
    /// the compositor's truncation-mode selection in `prime_truncation_cache`.
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub tile_follow_tail_at_tail: HashMap<SceneId, bool>,
    /// Runtime-owned lifecycle-affordance accents per tile.
    ///
    /// Set by the [`SceneMutation::SetTileLifecycleAccent`] apply path (driven by
    /// the coalescible StateStream `SetTileLifecycleAccent` wire mutation) and
    /// read by the compositor, which paints a token-colored left-edge bar.
    ///
    /// Lives here, keyed by tile id, rather than as a scene node so it survives
    /// `SetTileRoot`/`PublishToTile` content republishes (which replace the whole
    /// node tree). This is what keeps lifecycle-visible portals coalescible —
    /// the accent is never re-added as a per-republish `AddNode` (hud-m48i0).
    ///
    /// [`SceneMutation::SetTileLifecycleAccent`]: crate::mutation::SceneMutation::SetTileLifecycleAccent
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub tile_lifecycle_accents: HashMap<SceneId, LifecycleAccent>,
    /// Tile IDs removed since the last runtime drain.
    ///
    /// Populated by [`SceneGraph::remove_tile_and_nodes`] on every tile
    /// deletion; drained by the windowed runtime in `about_to_wait` via
    /// [`SceneGraph::drain_removed_tile_ids`].  Allows the runtime to eagerly
    /// prune per-tile state that cannot live inside the scene graph due to
    /// crate-layer constraints (e.g. `portal_resize_states` in `windowed.rs`,
    /// which holds a `tze_hud_input` type and would create a circular
    /// dependency if moved here).
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub recently_removed_tile_ids: Vec<SceneId>,
    /// Accepted MutationBatch ids applied since the last present drain (hud-91uu6).
    ///
    /// Populated by [`SceneGraph::apply_batch`] on every accepted (applied) batch,
    /// in application order. Drained by the render loop at frame present via
    /// [`SceneGraph::drain_present_ack_batch_ids`], which stamps them with the
    /// presented frame number + present wall-clock and emits a `FramePresented`
    /// event to TELEMETRY_FRAMES subscribers. This is the batch → on-screen-present
    /// correlation primitive: a batch whose commit changed the scene is reflected
    /// by the next composited frame, so recording batch_ids here and draining at
    /// present pairs each batch with the frame that carried it.
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub present_ack_pending_batch_ids: Vec<SceneId>,
    /// Tile IDs whose geometry the viewer has taken control of via a whole-portal
    /// move or resize gesture (hud-lyqun).
    ///
    /// While a tile is in this set the runtime holds geometry authority over it:
    /// the owning adapter's [`SceneMutation::UpdateTileBounds`] republishes become
    /// a bounds no-op (see [`SceneGraph::update_tile_bounds`]), so an adapter that
    /// re-emits its stale client-side layout on the next content publish or drag
    /// can no longer stomp individual members and fracture the portal. Content
    /// mutations (`SetTileRoot`/`PublishToTile`, `AddNode`, accents, input mode)
    /// are unaffected — they apply within the viewer-defined geometry, per the
    /// text-stream-portals resize contract ("the owning adapter … MUST NOT veto
    /// or reposition the surface"; adapter content "SHALL apply within the
    /// gesture-defined geometry").
    ///
    /// Viewer-driven resize/drag write `tile.bounds` directly and therefore
    /// bypass the lock; only adapter-originated `update_tile_bounds` calls are
    /// gated. Cleared per tile by the reset-geometry affordance and on tile
    /// removal.
    ///
    /// [`SceneMutation::UpdateTileBounds`]: crate::mutation::SceneMutation::UpdateTileBounds
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub viewer_geometry_locked: HashSet<SceneId>,

    /// Viewer-local per-tile font-scale multiplier for whole-portal resize text
    /// scaling (hud-ovjxu.1, spec §Portal Resize Text Scaling).
    ///
    /// Accumulates the portal's WIDTH ratio across resize steps (grow ⇒ >1,
    /// shrink ⇒ <1). The compositor multiplies each text node's published
    /// `font_size_px` by this factor at text-collection time and clamps the
    /// result to the token-defined legible min/max — so the text grows and
    /// shrinks with the portal without ever mutating the adapter-published node
    /// content. This is the durable, viewer-local design required by the spec
    /// ("font scaling ... MUST NOT alter the adapter-published content"): a
    /// content republish carries the base font, and the multiplier re-applies at
    /// render, so an adapter update can never reset the viewer's zoom (contrast a
    /// stored-font mutation, which a republish would stomp — the reason the
    /// [`Self::viewer_geometry_locked`] lock exists for bounds).
    ///
    /// Absent entry ⇒ 1.0 (no scaling). Cleared on tile removal and by the
    /// reset-geometry affordance alongside the geometry lock.
    ///
    /// Ephemeral: skipped during serialization.
    #[serde(skip, default)]
    pub tile_font_scale: HashMap<SceneId, f32>,
}

use super::SceneGraph;

impl SceneGraph {
    /// Queue a runtime-registered widget SVG asset for compositor registration.
    pub fn enqueue_widget_svg_asset(
        &mut self,
        widget_type_id: &str,
        svg_filename: &str,
        svg_bytes: Vec<u8>,
    ) {
        self.overlay.pending_widget_svg_assets.push((
            widget_type_id.to_string(),
            svg_filename.to_string(),
            svg_bytes,
        ));
    }

    /// Drain pending runtime widget SVG assets.
    pub fn drain_pending_widget_svg_assets(&mut self) -> Vec<(String, String, Vec<u8>)> {
        self.overlay.pending_widget_svg_assets.drain(..).collect()
    }

    /// Drain the list of tile IDs removed since the last call.
    ///
    /// Populated by [`SceneGraph::remove_tile_and_nodes`].  The windowed
    /// runtime calls this in `about_to_wait` (inside
    /// `prune_portal_resize_states`) to eagerly prune per-tile state held
    /// outside the scene graph (e.g. `portal_resize_states`).
    pub fn drain_removed_tile_ids(&mut self) -> Vec<SceneId> {
        self.overlay.recently_removed_tile_ids.drain(..).collect()
    }

    /// Record an accepted MutationBatch id as pending on-screen present (hud-91uu6).
    ///
    /// Called by [`SceneGraph::apply_batch`] for every batch that applied. The id
    /// is drained at the next frame present by [`Self::drain_present_ack_batch_ids`]
    /// and paired with the presented frame number + present wall-clock to form a
    /// `FramePresented` acknowledgment.
    pub fn record_present_ack_batch(&mut self, batch_id: SceneId) {
        self.overlay.present_ack_pending_batch_ids.push(batch_id);
    }

    /// Drain the batch_ids applied since the last present (hud-91uu6).
    ///
    /// Returns them in application order. The render loop calls this once per
    /// presented frame; a non-empty result is stamped with the frame number and
    /// present wall-clock and emitted as a `FramePresented` event to
    /// TELEMETRY_FRAMES subscribers.
    pub fn drain_present_ack_batch_ids(&mut self) -> Vec<SceneId> {
        self.overlay
            .present_ack_pending_batch_ids
            .drain(..)
            .collect()
    }

    /// Take viewer geometry authority over a tile (hud-lyqun).
    ///
    /// Called for every member of a portal group when the viewer moves or
    /// resizes the whole portal, so a subsequent adapter `UpdateTileBounds`
    /// republish cannot reposition the member and fracture the group.
    pub fn lock_viewer_geometry(&mut self, tile_id: SceneId) {
        self.overlay.viewer_geometry_locked.insert(tile_id);
    }

    /// Release viewer geometry authority over a tile, restoring adapter control
    /// of its bounds. Called by the reset-geometry affordance and on tile
    /// removal.
    pub fn unlock_viewer_geometry(&mut self, tile_id: SceneId) {
        self.overlay.viewer_geometry_locked.remove(&tile_id);
    }

    /// The viewer-local font-scale multiplier for `tile_id` (hud-ovjxu.1).
    /// Returns `1.0` when no scaling has been applied (the common case).
    pub fn tile_font_scale(&self, tile_id: SceneId) -> f32 {
        self.overlay
            .tile_font_scale
            .get(&tile_id)
            .copied()
            .unwrap_or(1.0)
    }

    /// Set the viewer-local font-scale multiplier for `tile_id` (hud-ovjxu.1).
    ///
    /// A factor of exactly `1.0` removes the entry (back to the default) so the
    /// map only holds tiles the viewer has actually zoomed. Non-finite or
    /// non-positive factors are ignored (defensive — a resize ratio is always a
    /// positive finite number).
    pub fn set_tile_font_scale(&mut self, tile_id: SceneId, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        if (factor - 1.0).abs() < f32::EPSILON {
            self.overlay.tile_font_scale.remove(&tile_id);
        } else {
            self.overlay.tile_font_scale.insert(tile_id, factor);
        }
    }

    /// Clear any viewer-local font scaling for `tile_id`, restoring the
    /// adapter-published font size. Called by the reset-geometry affordance and
    /// on tile removal, alongside [`Self::unlock_viewer_geometry`].
    pub fn clear_tile_font_scale(&mut self, tile_id: SceneId) {
        self.overlay.tile_font_scale.remove(&tile_id);
    }

    /// Whether the viewer holds geometry authority over `tile_id` — i.e. adapter
    /// bounds republishes are currently suppressed for it.
    pub fn is_viewer_geometry_locked(&self, tile_id: SceneId) -> bool {
        self.overlay.viewer_geometry_locked.contains(&tile_id)
    }

    /// Register local-first scroll config for a tile.
    pub fn register_tile_scroll_config(
        &mut self,
        tile_id: SceneId,
        config: TileScrollConfig,
    ) -> Result<(), ValidationError> {
        if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }
        self.overlay.tile_scroll_configs.insert(tile_id, config);
        self.overlay
            .tile_scroll_offsets
            .entry(tile_id)
            .or_insert((0.0, 0.0));
        Ok(())
    }

    /// Remove local-first scroll config and offset state for a tile.
    pub fn clear_tile_scroll_config(&mut self, tile_id: SceneId) {
        self.overlay.tile_scroll_configs.remove(&tile_id);
        self.overlay.tile_scroll_offsets.remove(&tile_id);
        self.overlay.displayed_tile_scroll_offsets.remove(&tile_id);
    }

    /// Get the registered local-first scroll config for a tile.
    pub fn tile_scroll_config(&self, tile_id: SceneId) -> Option<TileScrollConfig> {
        self.overlay.tile_scroll_configs.get(&tile_id).copied()
    }

    /// Set the lifecycle-affordance accent for a tile (latest-wins; coalescible).
    ///
    /// Bumps `scene.version` only when the stored accent actually changes, so an
    /// accent-only transition re-arms the #943 idle present-gate (which repaints
    /// on `scene.version != last_rendered`) and the new color paints even when no
    /// content mutation co-travels in the batch. A redundant re-publish of the
    /// same accent leaves the version untouched, keeping a steady-state portal
    /// idle.
    pub fn set_tile_lifecycle_accent(
        &mut self,
        tile_id: SceneId,
        accent: LifecycleAccent,
    ) -> Result<(), ValidationError> {
        if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }
        if self.overlay.tile_lifecycle_accents.get(&tile_id) != Some(&accent) {
            self.overlay.tile_lifecycle_accents.insert(tile_id, accent);
            self.version += 1;
        }
        Ok(())
    }

    /// Clear the lifecycle-affordance accent for a tile (no-op if unset).
    ///
    /// Bumps `scene.version` only when an accent was actually present, so a
    /// redaction-CLEAR transition re-arms the present-gate and the stale accent
    /// is removed on screen; a clear with nothing stored stays idle.
    pub fn clear_tile_lifecycle_accent(&mut self, tile_id: SceneId) {
        if self
            .overlay
            .tile_lifecycle_accents
            .remove(&tile_id)
            .is_some()
        {
            self.version += 1;
        }
    }

    /// Get the lifecycle-affordance accent for a tile, if any.
    pub fn tile_lifecycle_accent(&self, tile_id: SceneId) -> Option<LifecycleAccent> {
        self.overlay.tile_lifecycle_accents.get(&tile_id).copied()
    }

    /// Set the tile-local scroll offset used by runtime local feedback.
    pub fn set_tile_scroll_offset_local(
        &mut self,
        tile_id: SceneId,
        offset_x: f32,
        offset_y: f32,
    ) -> Result<(), ValidationError> {
        if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }
        self.overlay
            .tile_scroll_offsets
            .insert(tile_id, (offset_x, offset_y));
        Ok(())
    }

    /// Get the current runtime-local scroll offset for a tile.
    ///
    /// This is the *authoritative* offset — the scroll target the runtime
    /// committed. During a smoothed scroll animation the renderer draws at the
    /// lagged displayed offset instead; use
    /// [`effective_tile_scroll_offset_local`](Self::effective_tile_scroll_offset_local)
    /// for pointer mapping so input and render agree.
    pub fn tile_scroll_offset_local(&self, tile_id: SceneId) -> (f32, f32) {
        self.overlay
            .tile_scroll_offsets
            .get(&tile_id)
            .copied()
            .unwrap_or((0.0, 0.0))
    }

    /// Publish the displayed (smoothed/lagged) scroll offset for a tile.
    ///
    /// Called by the compositor once per frame, after advancing the per-tile
    /// `ScrollSmoother`s, so the hit-test path can map pointer coordinates
    /// against the same offset the renderer drew with (hud-3lynp). Only invoked
    /// while windowed scroll smoothing is active.
    pub fn set_displayed_tile_scroll_offset(
        &mut self,
        tile_id: SceneId,
        offset_x: f32,
        offset_y: f32,
    ) {
        self.overlay
            .displayed_tile_scroll_offsets
            .insert(tile_id, (offset_x, offset_y));
    }

    /// Drop all published displayed scroll offsets.
    ///
    /// Called by the compositor when scroll smoothing is disabled (headless /
    /// snap) so hit-testing falls back to the authoritative offset.
    pub fn clear_displayed_tile_scroll_offsets(&mut self) {
        self.overlay.displayed_tile_scroll_offsets.clear();
    }

    /// Retain only the displayed scroll offsets whose tile satisfies `keep`.
    ///
    /// Lets the compositor prune overrides for tiles that no longer have an
    /// active smoother (no longer scrollable / removed) each frame.
    pub fn retain_displayed_tile_scroll_offsets<F: FnMut(SceneId) -> bool>(&mut self, mut keep: F) {
        self.overlay
            .displayed_tile_scroll_offsets
            .retain(|id, _| keep(*id));
    }

    /// Get the *effective* tile-local scroll offset for pointer mapping.
    ///
    /// Returns the displayed (smoothed/lagged) offset when the compositor has
    /// published one for this tile (i.e. a smoothed scroll is in flight);
    /// otherwise falls back to the authoritative
    /// [`tile_scroll_offset_local`](Self::tile_scroll_offset_local). This is the
    /// offset the hit-test path uses so input and render agree during animation
    /// (hud-3lynp).
    pub fn effective_tile_scroll_offset_local(&self, tile_id: SceneId) -> (f32, f32) {
        self.overlay
            .displayed_tile_scroll_offsets
            .get(&tile_id)
            .copied()
            .unwrap_or_else(|| self.tile_scroll_offset_local(tile_id))
    }

    // ─── Follow-tail anchor ───────────────────────────────────────────────

    /// Record whether a tile's viewport is at the tail of its content stream.
    ///
    /// Called by the runtime input layer whenever `FollowTailAnchor` transitions
    /// (typically after `ScrollState::notify_content_appended` or
    /// `ScrollState::apply_user_scroll`).
    ///
    /// `at_tail = true` means the tile uses tail-anchored truncation (newest
    /// content always visible).  `at_tail = false` means the tile uses
    /// head-anchored truncation (scrolled-back viewport, spec task 3.3
    /// append-stability guarantee).
    ///
    /// No-op if the tile does not exist (tiles without scroll state always
    /// behave as head-anchored by default).
    pub fn set_tile_follow_tail_at_tail(&mut self, tile_id: SceneId, at_tail: bool) {
        if self.tiles.contains_key(&tile_id) {
            self.overlay
                .tile_follow_tail_at_tail
                .insert(tile_id, at_tail);
        }
    }

    /// Return whether a tile's viewport is currently at the tail.
    ///
    /// Returns `true` (at-tail, tail-anchored mode) when the tile has been
    /// explicitly registered via [`set_tile_follow_tail_at_tail`] with `true`.
    ///
    /// Returns `false` (head-anchored mode) when:
    /// - the tile has been scrolled back, or
    /// - the tile has never been registered (non-scrollable or newly created
    ///   tiles that have not yet received a content-append or scroll event).
    ///
    /// Non-scrollable static-text tiles must default to `false` so that
    /// `TextOverflow::Ellipsis` shows the beginning of the text, not the end.
    pub fn tile_follow_tail_at_tail(&self, tile_id: SceneId) -> bool {
        self.overlay
            .tile_follow_tail_at_tail
            .get(&tile_id)
            .copied()
            .unwrap_or(false)
    }
}
