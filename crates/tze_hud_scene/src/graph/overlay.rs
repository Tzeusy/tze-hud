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
    }

    /// Get the registered local-first scroll config for a tile.
    pub fn tile_scroll_config(&self, tile_id: SceneId) -> Option<TileScrollConfig> {
        self.overlay.tile_scroll_configs.get(&tile_id).copied()
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
    pub fn tile_scroll_offset_local(&self, tile_id: SceneId) -> (f32, f32) {
        self.overlay
            .tile_scroll_offsets
            .get(&tile_id)
            .copied()
            .unwrap_or((0.0, 0.0))
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
