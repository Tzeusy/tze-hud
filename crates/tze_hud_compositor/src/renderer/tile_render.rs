//! Tile and node rendering methods for the compositor.
//!
//! Moved from `renderer/mod.rs` (the "Drag-boost helpers" + tile/node
//! rendering cluster, formerly ~L7900–8213 at plan date) by Step R-9 of the
//! renderer module split (hud-fgryk).  No logic was changed; `pub(super)`
//! visibility was added to the methods called from sibling modules or from
//! `mod.rs`:
//!
//! - `effective_tile_z_order` — called by tests in `mod.rs`
//! - `effective_tile_opacity` — called by tests in `mod.rs`
//! - `sort_tiles_with_drag_boost` — called by `render_frame*` in `mod.rs`
//! - `tile_background_color` — called by `render_frame*` in `mod.rs`
//! - `render_composer_overlay` — called by `render_frame*` in `mod.rs`
//! - `render_node` — called by `render_frame*` in `mod.rs`
//! - `collect_composer_text_item` — called by `collect_text_items` in `text.rs`
//!
//! ## Methods in this file
//!
//! - `effective_tile_z_order` — effective sort key for a tile, with drag boost.
//! - `effective_tile_opacity` — effective opacity for a tile, with drag boost.
//! - `sort_tiles_with_drag_boost` — re-sort tiles by effective z-order.
//! - `tile_background_color` — background fill color for a tile from design tokens.
//! - `render_composer_overlay` — geometry for the local composer echo overlay.
//! - `tile_contains_composer_node` — search tile subtree for a composer HitRegion.
//! - `node_tree_contains` — depth-first search for a node in a subtree.
//! - `collect_composer_text_item` — build a TextItem for the composer draft text.
//! - `render_node` — render a node and its children within a tile.

use std::sync::Arc;

use tze_hud_input::{DRAG_OPACITY_BOOST, DRAG_Z_ORDER_BOOST};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;

use crate::pipeline::{RectVertex, rect_vertices};

use super::Compositor;
use super::draw_cmds::{TexturedDrawCmd, compute_fit_mode};
use super::image_cache::{
    ComposerLayout, caret_visible_at, composer_display_text_blink, composer_region_fit_lines,
    composer_scroll_offset, composer_vertical_line_offset, composer_visible_line_count,
};
use super::token_colors::{
    ComposerOverlayTokens, TILE_BG_DEFAULT, TILE_BG_STATIC_IMAGE, TILE_BG_TEXT_MARKDOWN,
    linear_to_srgb, resolve_composer_overlay_tokens, resolve_focus_ring_tokens,
    resolve_tile_bg_token, resolve_viewer_echo_tokens,
};

/// Horizontal inset (physical px) between the composer region edge and the draft
/// text, on both the left and right.  Shared by [`Compositor::collect_composer_text_item`]
/// (where it positions the draft and its clip) and [`Compositor::prime_composer_scroll_offset`]
/// (where it defines the caret-follow window and keep-visible margin), so the two
/// stay in lockstep.  Matches the composer strip's visual padding.
const COMPOSER_TEXT_MARGIN: f32 = 6.0;

/// Compute divider rectangles for transcript turn separators (hud-nx7yq.4).
///
/// Pure geometry: for each thematic-break byte offset in `breaks`, count the
/// newlines in `plain[..offset]` to find the break's blank-line index, then place
/// a `thickness`-tall full-width rule centred vertically within that line. The
/// origin (`origin_x`, `origin_y`) is the node's top-left in display space (tile
/// and node bounds, minus scroll); `width` is the node width. Returns
/// absolute-space `Rect`s the caller clips to the tile and fills with the token
/// divider color.
///
/// Kept free-standing (no `self`, no GPU) so the line-counting / centring math is
/// unit-testable without a headless compositor.
pub(super) fn transcript_separator_rects(
    plain: &str,
    breaks: &[usize],
    origin_x: f32,
    origin_y: f32,
    width: f32,
    line_height: f32,
    thickness: f32,
) -> Vec<Rect> {
    let w = width.max(0.0);
    if w <= 0.0 || thickness <= 0.0 {
        return Vec::new();
    }
    let mut rects = Vec::with_capacity(breaks.len());
    for &offset in breaks {
        let clamped = offset.min(plain.len());
        // Line index of the divider's (blank) line = newlines before it.
        let lines_before = plain[..clamped].chars().filter(|&c| c == '\n').count();
        // Centre the rule within its line, then offset up by half its thickness.
        let center_y = (lines_before as f32 + 0.5) * line_height;
        let y = origin_y + center_y - thickness / 2.0;
        rects.push(Rect::new(origin_x, y, w, thickness));
    }
    rects
}

impl Compositor {
    // ─── Drag-boost helpers ───────────────────────────────────────────────────

    /// Return the effective sort key for a tile, applying `DRAG_Z_ORDER_BOOST`
    /// when the tile is in the `Activated` drag phase.
    ///
    /// The boost raises the dragged tile above its peers in painter's-algorithm
    /// order (back-to-front).  `saturating_add` prevents wraparound for tiles
    /// already near `u32::MAX`.
    ///
    /// Per `tze_hud_input::drag::DRAG_Z_ORDER_BOOST` (0x1000).
    pub(super) fn effective_tile_z_order(tile: &Tile, scene: &SceneGraph) -> u32 {
        if scene.is_drag_active(tile.id) {
            tile.z_order.saturating_add(DRAG_Z_ORDER_BOOST)
        } else {
            tile.z_order
        }
    }

    /// Return the effective opacity for a tile, applying `DRAG_OPACITY_BOOST`
    /// (clamped to 1.0) when the tile is in the `Activated` drag phase.
    ///
    /// `DRAG_OPACITY_BOOST` is currently 1.0 (no visible change), but is applied
    /// faithfully so future changes to the constant take effect without a code
    /// change in the compositor.
    ///
    /// Per `tze_hud_input::drag::DRAG_OPACITY_BOOST`.
    pub(super) fn effective_tile_opacity(tile: &Tile, scene: &SceneGraph) -> f32 {
        if scene.is_drag_active(tile.id) {
            (tile.opacity * DRAG_OPACITY_BOOST).min(1.0)
        } else {
            tile.opacity
        }
    }

    /// Re-sort a slice of tile references by effective z-order (back to front),
    /// applying `DRAG_Z_ORDER_BOOST` to the dragged tile's sort key.
    ///
    /// This does not mutate the scene; it returns a new owned `Vec<&Tile>` with
    /// the drag-boosted ordering.  The original `z_order` fields are unchanged.
    pub(super) fn sort_tiles_with_drag_boost<'a>(
        tiles: Vec<&'a Tile>,
        scene: &SceneGraph,
    ) -> Vec<&'a Tile> {
        let mut sorted = tiles;
        sorted.sort_by_key(|t| Self::effective_tile_z_order(t, scene));
        sorted
    }

    pub(super) fn clip_rect_to_tile(tile: &Tile, rect: Rect) -> Option<Rect> {
        let left = rect.x.max(tile.bounds.x);
        let top = rect.y.max(tile.bounds.y);
        let right = (rect.x + rect.width).min(tile.bounds.x + tile.bounds.width);
        let bottom = (rect.y + rect.height).min(tile.bounds.y + tile.bounds.height);
        if right <= left || bottom <= top {
            return None;
        }
        Some(Rect::new(left, top, right - left, bottom - top))
    }

    fn append_clipped_rect_vertices(
        tile: &Tile,
        rect: Rect,
        sw: f32,
        sh: f32,
        color: [f32; 4],
        vertices: &mut Vec<RectVertex>,
    ) {
        let Some(clipped) = Self::clip_rect_to_tile(tile, rect) else {
            return;
        };
        vertices.extend_from_slice(&rect_vertices(
            clipped.x,
            clipped.y,
            clipped.width,
            clipped.height,
            sw,
            sh,
            color,
        ));
    }

    /// Four edge rectangles forming a `width`-px focus ring stroked *inside* the
    /// edges of `region` (top, bottom, left, right).
    ///
    /// Drawing inward (rather than outward) keeps the ring within the focused
    /// region's own bounds, so it never bleeds past the owning tile and is
    /// clipped consistently with the region it decorates. The width is clamped to
    /// half the smaller dimension so a thin region cannot produce inverted rects.
    /// Returns empty edges (zero-size rects are dropped by
    /// `append_clipped_rect_vertices`) when `region` has no area.
    fn focus_ring_edge_rects(region: Rect, width: f32) -> [Rect; 4] {
        let w = width
            .max(0.0)
            .min(region.width / 2.0)
            .min(region.height / 2.0);
        if w <= 0.0 || region.width <= 0.0 || region.height <= 0.0 {
            return [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
        }
        [
            // Top edge.
            Rect::new(region.x, region.y, region.width, w),
            // Bottom edge.
            Rect::new(region.x, region.y + region.height - w, region.width, w),
            // Left edge.
            Rect::new(region.x, region.y, w, region.height),
            // Right edge.
            Rect::new(region.x + region.width - w, region.y, w, region.height),
        ]
    }

    /// Emit the keyboard focus ring for the current focus owner into `vertices`,
    /// for the chrome-layer pass drawn above all agent content (hud-k6yvb).
    ///
    /// Reads the runtime-plumbed [`focus_ring::FocusRingOwner`] (drained into
    /// `self.focus_ring_owner`). Draws nothing when focus is cleared, on a
    /// non-active tab, or the owning tile is gone. The ring covers BOTH owner
    /// kinds a keyboard user can land on:
    /// - **node** focus → a ring around the node's display-space bounds (with the
    ///   tile's scroll offset applied so it tracks the visibly-rendered node);
    /// - **tile-level** focus (a non-passthrough tile with no focusable nodes) →
    ///   a ring around the whole tile.
    ///
    /// Token-driven color/width; clipped to the owning tile so a scrolled-off
    /// node's ring never bleeds outside the portal. Overlay-safe (same
    /// `gpu_color_raw` flat-rect path as the hover/press tints).
    pub(super) fn append_focus_ring_vertices(
        &self,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        sw: f32,
        sh: f32,
    ) {
        let Some(owner) = self.focus_ring_owner else {
            return;
        };
        // Only the active tab's focus draws a ring (focus is per-tab).
        if scene.active_tab != Some(owner.tab_id) {
            return;
        }
        let Some(tile) = scene.tiles.get(&owner.tile_id) else {
            return;
        };

        let region = match owner.node_id {
            Some(node_id) => {
                let Some(node) = scene.nodes.get(&node_id) else {
                    return;
                };
                let NodeData::HitRegion(hr) = &node.data else {
                    return;
                };
                let (scroll_x, scroll_y) = self.display_tile_scroll_offset(scene, owner.tile_id);
                Rect::new(
                    tile.bounds.x + hr.bounds.x - scroll_x,
                    tile.bounds.y + hr.bounds.y - scroll_y,
                    hr.bounds.width,
                    hr.bounds.height,
                )
            }
            None => tile.bounds,
        };

        let ring = resolve_focus_ring_tokens(&self.token_map);
        let ring_color = self.gpu_color_raw(ring.color);
        for edge in Self::focus_ring_edge_rects(region, ring.width_px) {
            Self::append_clipped_rect_vertices(tile, edge, sw, sh, ring_color, vertices);
        }
    }

    fn clipped_textured_rect(
        tile: &Tile,
        rect: Rect,
        uv_rect: [f32; 4],
    ) -> Option<(Rect, [f32; 4])> {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return None;
        }
        let clipped = Self::clip_rect_to_tile(tile, rect)?;
        if clipped.x == rect.x
            && clipped.y == rect.y
            && clipped.width == rect.width
            && clipped.height == rect.height
        {
            return Some((rect, uv_rect));
        }
        let left_frac = ((clipped.x - rect.x) / rect.width).clamp(0.0, 1.0);
        let right_frac = ((clipped.x + clipped.width - rect.x) / rect.width).clamp(0.0, 1.0);
        let top_frac = ((clipped.y - rect.y) / rect.height).clamp(0.0, 1.0);
        let bottom_frac = ((clipped.y + clipped.height - rect.y) / rect.height).clamp(0.0, 1.0);

        let [u0, v0, u1, v1] = uv_rect;
        let du = u1 - u0;
        let dv = v1 - v0;
        Some((
            clipped,
            [
                u0 + du * left_frac,
                v0 + dv * top_frac,
                u0 + du * right_frac,
                v0 + dv * bottom_frac,
            ],
        ))
    }

    /// Effective per-frame render opacity for a tile: scene-level tile opacity
    /// (with drag boost) multiplied by the §6.3 portal-transition fade opacity
    /// for scrollable (portal) tiles, clamped to `[0, 1]`.
    ///
    /// This is the single source of truth for "how faded is the whole tile this
    /// frame". It MUST be applied uniformly to every element that composes the
    /// tile's opaque body — both the flat tile backdrop ([`tile_background_color`])
    /// and the content-node backgrounds painted in [`render_node`] — so the tile
    /// backdrop fades as a single unit. (The text pass fades on the portal-fade
    /// component alone via `portal_tile_anim_opacity`, which for a portal — where
    /// `tile.opacity` stays 1 — equals this value.) A backdrop site that omits it
    /// paints at full opacity while its neighbours fade, leaving that region
    /// see-through relative to the rest of the tile on any fade or geometry change
    /// that exposes it (hud-w41ef).
    pub(super) fn tile_effective_opacity(&self, tile: &Tile, scene: &SceneGraph) -> f32 {
        // Base opacity from drag state (scene-level tile.opacity × drag boost).
        let base_opacity = Self::effective_tile_opacity(tile, scene);
        // §6.3: multiply portal tile animation opacity for scrollable (portal) tiles.
        (base_opacity * self.portal_tile_anim_opacity(tile.id)).clamp(0.0, 1.0)
    }

    /// Determine the background fill color for a tile based on its root content.
    ///
    /// Colors are resolved from design tokens (`color.tile.background.*`) with
    /// documented fallback constants — no naked color literals here.  The opacity
    /// channel is derived from the tile's effective opacity (drag state) multiplied
    /// by the portal tile animation opacity (§6.3 transition tokens) when the tile
    /// has a registered `TileScrollConfig`.
    ///
    /// Returns `None` when the tile's rounded root node should be solely
    /// responsible for its own backdrop shape.
    pub(super) fn tile_background_color(
        &self,
        tile: &Tile,
        scene: &SceneGraph,
    ) -> Option<[f32; 4]> {
        let opacity = self.tile_effective_opacity(tile, scene);
        if let Some(root_id) = tile.root_node
            && let Some(node) = scene.nodes.get(&root_id)
        {
            match &node.data {
                NodeData::SolidColor(sc) => {
                    if sc.radius.is_some_and(|r| r > 0.0) {
                        return None;
                    }
                    // Apply combined opacity to the solid color alpha.
                    let mut c = sc.color.to_array();
                    c[3] *= opacity;
                    return Some(c);
                }
                NodeData::TextMarkdown(tm) => {
                    if let Some(bg) = &tm.background {
                        // Apply combined opacity to the token-supplied background alpha.
                        let mut c = bg.to_array();
                        c[3] *= opacity;
                        return Some(c);
                    }
                    let c = resolve_tile_bg_token(
                        &self.token_map,
                        "color.tile.background.text_markdown",
                        TILE_BG_TEXT_MARKDOWN,
                    );
                    return Some([c.r, c.g, c.b, opacity]);
                }
                NodeData::HitRegion(_) => {
                    // HitRegion is an invisible interaction primitive. Its visible
                    // footprint is supplied by sibling content nodes; the runtime
                    // only paints local-feedback tints on hover/press (RFC 0004 §6.5).
                    return None;
                }
                NodeData::StaticImage(_) => {
                    let c = resolve_tile_bg_token(
                        &self.token_map,
                        "color.tile.background.static_image",
                        TILE_BG_STATIC_IMAGE,
                    );
                    return Some([c.r, c.g, c.b, opacity]);
                }
            }
        }
        let c = resolve_tile_bg_token(
            &self.token_map,
            "color.tile.background.default",
            TILE_BG_DEFAULT,
        );
        Some([c.r, c.g, c.b, opacity])
    }

    /// Compute the lifecycle-affordance accent bar geometry+color for a tile, or
    /// `None` when nothing should paint.
    ///
    /// Pure (no GPU / no `self`) so it is unit-testable. The bar hugs the tile's
    /// left edge and spans the full tile height; its width is the token-resolved
    /// `accent.width_px` clamped to `[0, tile width]` so an out-of-range token
    /// value can never push it past the tile edge. The returned color is the
    /// token-resolved accent color (no literal visual value here, §6.1) with the
    /// caller-supplied combined tile opacity folded into the alpha channel, so the
    /// accent fades with the tile (matching the tile background).
    ///
    /// Returns `None` for a zero/negative width or non-positive opacity (nothing
    /// visible to draw). Source: hud-m48i0.
    pub(super) fn lifecycle_accent_bar_geom(
        tile_bounds: Rect,
        accent: LifecycleAccent,
        opacity: f32,
    ) -> Option<(f32, [f32; 4])> {
        // Sanitize the dynamic upper bound: f32::clamp panics when min > max, so a
        // negative (or NaN) tile width would otherwise crash the frame loop. `.max(0.0)`
        // pins it to a valid `[0, _]` range before clamping the token-resolved width.
        let bar_w = accent.width_px.clamp(0.0, tile_bounds.width.max(0.0));
        if bar_w <= 0.0 || opacity <= 0.0 {
            return None;
        }
        let mut color = accent.color.to_array();
        color[3] *= opacity;
        Some((bar_w, color))
    }

    /// Emit geometry for the local composer echo overlay.
    ///
    /// Called from the tile loop (after the content pass) when
    /// `self.local_composer` is `Some` and the state's `node_id` belongs to a
    /// `HitRegionNode` with `accepts_composer_input = true` inside the given
    /// tile.  If no matching node is found in the tile the call is a no-op.
    ///
    /// Emits:
    /// 1. A background fill rect for the composer hit-region.
    /// 2. When at_capacity, a 2px left-edge accent rect using the at-capacity
    ///    color so the user has a visual signal that no further input is
    ///    accepted.  The text pass (see `collect_composer_text_item`) renders
    ///    the draft text on top in the same hit-region.
    ///
    /// Geometry is clamped to the tile bounds to avoid overdraw outside the portal.
    pub(super) fn render_composer_overlay(
        &self,
        tile: &Tile,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        sw: f32,
        sh: f32,
        tokens: &ComposerOverlayTokens,
    ) {
        let Some(ref cs) = self.local_composer else {
            return;
        };

        let Some(region) = Self::composer_region_bounds(tile, scene, cs.node_id) else {
            return;
        };
        // Confine the composer chrome to the input box so it does not paint over
        // the whole portal (hud-2zsbf). The box grows upward with the wrapped-line
        // count primed into `composer_layout` (hud-nx7yq.1); `visible_lines == 1`
        // keeps the single-line strip. Kept in lockstep with
        // `collect_composer_text_item`, which anchors the draft to the same box.
        let line_height_multiplier =
            crate::markdown::MarkdownTokens::default().line_height_multiplier;
        let input_box = Self::composer_input_box(
            region,
            tokens.font_size_px,
            line_height_multiplier,
            self.composer_layout.visible_lines,
        );

        // Background fill.
        let bg_color = [tokens.bg_r, tokens.bg_g, tokens.bg_b, tokens.bg_a];
        vertices.extend_from_slice(&rect_vertices(
            input_box.x,
            input_box.y,
            input_box.width,
            input_box.height,
            sw,
            sh,
            bg_color,
        ));

        // At-capacity left-edge accent (2px wide, full box height).
        if cs.at_capacity {
            let accent = [
                tokens.at_capacity_r,
                tokens.at_capacity_g,
                tokens.at_capacity_b,
                tokens.at_capacity_a,
            ];
            vertices.extend_from_slice(&rect_vertices(
                input_box.x,
                input_box.y,
                2.0,
                input_box.height,
                sw,
                sh,
                accent,
            ));
        }
    }

    /// Return display-space bounds for the focused composer HitRegion inside a
    /// tile, clamped to the tile bounds.
    ///
    /// Local composer echo belongs to the composer region, not the bottom edge
    /// of the containing tile.  This keeps runtime-local feedback aligned with
    /// agent-rendered composer chrome in raw-tile portals.
    fn composer_region_bounds(tile: &Tile, scene: &SceneGraph, target: SceneId) -> Option<Rect> {
        let root_id = tile.root_node?;
        let local = Self::node_tree_composer_bounds(root_id, scene, target)?;

        let left = local.x.clamp(0.0, tile.bounds.width);
        let top = local.y.clamp(0.0, tile.bounds.height);
        let right = (local.x + local.width).clamp(0.0, tile.bounds.width);
        let bottom = (local.y + local.height).clamp(0.0, tile.bounds.height);
        if right <= left || bottom <= top {
            return None;
        }

        Some(Rect::new(
            tile.bounds.x + left,
            tile.bounds.y + top,
            right - left,
            bottom - top,
        ))
    }

    /// Confine the rendered composer echo to a single input-line strip at the
    /// BOTTOM of the composer region (hud-2zsbf).
    ///
    /// The composer HitRegion published by the projection authority spans the
    /// WHOLE portal (`resident_grpc::local_bounds_for_state` → `x:0, y:0, w, h`)
    /// so a pointer-down anywhere in the portal focuses the composer (hud-v4k1h).
    /// That full-portal region is the correct *pointer/focus* target, but using
    /// it verbatim as the echo's layout+clip box laid the single unwrapped draft
    /// line across the entire portal width at the portal's TOP — reading live as
    /// "the draft extends forever past the composer box". The clip was effective;
    /// the box was simply the whole portal.
    ///
    /// This derives the actual composer *input box* from that region: one text
    /// line tall (font line-height plus symmetric vertical padding equal to the
    /// horizontal text margin), pinned to the bottom edge where a chat-style
    /// composer belongs. Width and x are unchanged, so horizontal caret-follow is
    /// unaffected; only the vertical placement + clip height shrink to one line.
    /// For a composer region that is already ~one line tall (the intended
    /// promotion-era structured composer node) the strip equals the region, so
    /// behaviour there is unchanged.
    /// Composer input box for `visible_lines` wrapped text lines, pinned to the
    /// BOTTOM of the region and grown UPWARD (hud-nx7yq.1).
    ///
    /// Height is `visible_lines` text lines plus symmetric vertical padding (equal
    /// to the horizontal text margin), clamped to the region height.
    /// `visible_lines == 1.0` reproduces the single input-line strip (hud-2zsbf)
    /// exactly, so single-line behaviour is unchanged.
    ///
    /// Growth is viewer-local: the box extends upward over the transcript, which
    /// yields the space by occlusion; the portal's outer geometry is untouched.
    pub(super) fn composer_input_box(
        region: Rect,
        font_size_px: f32,
        line_height_multiplier: f32,
        visible_lines: f32,
    ) -> Rect {
        let line_height = (font_size_px * line_height_multiplier).max(1.0);
        let lines = visible_lines.max(1.0);
        let box_height = (line_height * lines + COMPOSER_TEXT_MARGIN * 2.0)
            .min(region.height)
            .max(1.0);
        let box_y = region.y + (region.height - box_height).max(0.0);
        Rect::new(region.x, box_y, region.width, box_height)
    }

    /// Depth-first search for `target` in the node sub-tree rooted at `node_id`.
    fn node_tree_composer_bounds(
        node_id: SceneId,
        scene: &SceneGraph,
        target: SceneId,
    ) -> Option<Rect> {
        if node_id == target {
            // Verify the node is indeed a composer-capable HitRegion.
            return scene.nodes.get(&node_id).and_then(|n| match &n.data {
                NodeData::HitRegion(hr) if hr.accepts_composer_input => Some(hr.bounds),
                _ => None,
            });
        }
        if let Some(node) = scene.nodes.get(&node_id) {
            for child in &node.children {
                if let Some(bounds) = Self::node_tree_composer_bounds(*child, scene, target) {
                    return Some(bounds);
                }
            }
        }
        None
    }

    /// Recompute the active composer's per-frame layout into `self.composer_layout`,
    /// ready for the following `collect_text_items` pass (hud-zlfi4 single-line
    /// caret-follow + hud-nx7yq.1 multi-line wrap / upward growth / vertical scroll).
    ///
    /// Must be called once per frame BEFORE `collect_text_items` (it measures the
    /// draft against the composer font via the mutable text rasterizer, which the
    /// immutable collect path cannot do).  When no composer is active, the text
    /// rasterizer is missing, or the composer region cannot be located, the layout
    /// resets to [`ComposerLayout::default`] (single line, no scroll).
    ///
    /// Two profiles, selected by the `portal.composer.max_lines` token:
    /// - **max_lines == 1** — single-line profile: measure the caret x and pin a
    ///   horizontal scroll offset so the caret stays visible (hud-zlfi4).
    /// - **max_lines > 1** — multi-line profile: wrap-measure the draft to the box
    ///   width, size the box to grow upward to at most `max_lines`, and pin a
    ///   vertical scroll once the draft exceeds that bound.
    ///
    /// The measured window / margin / wrap width here mirror exactly the geometry
    /// `collect_composer_text_item` uses to place the draft. Local presentation
    /// state — no adapter round trip.
    pub(crate) fn prime_composer_scroll_offset(&mut self, scene: &SceneGraph) {
        self.composer_layout = ComposerLayout::default();
        // Clear the reverse visual-layout channel by default (hud-21o6x); only the
        // multi-line branch below republishes a fresh layout. Every early return
        // (no composer / no rasterizer / no region / single-line) leaves it `None`,
        // so the input layer falls back to hard-newline vertical movement.
        if let Ok(mut guard) = self.composer_visual_layout.lock() {
            *guard = None;
        }

        // Gather the immutable inputs first (draft text, caret, region geometry,
        // font size) so the mutable text-rasterizer borrow below does not overlap
        // the `&self` reads.
        let Some(cs) = self.local_composer.as_ref() else {
            return;
        };
        if self.text_rasterizer.is_none() {
            return;
        }

        // Locate the composer region the same way collect_composer_text_item does:
        // the first visible tile whose subtree contains the focused composer node.
        let mut region: Option<Rect> = None;
        for tile in &Self::sort_tiles_with_drag_boost(scene.visible_tiles(), scene) {
            if let Some(r) = Self::composer_region_bounds(tile, scene, cs.node_id) {
                region = Some(r);
                break;
            }
        }
        let Some(region) = region else {
            return;
        };

        let tokens = resolve_composer_overlay_tokens(&self.token_map);
        let font_size_px = tokens.font_size_px;
        let max_lines = tokens.max_lines.max(1);
        // Visible text window = region interior width (region width minus the left
        // and right text margins).  This is the same `bw` collect uses.
        let window_width = (region.width - COMPOSER_TEXT_MARGIN * 2.0).max(1.0);
        let line_height_multiplier =
            crate::markdown::MarkdownTokens::default().line_height_multiplier;
        let line_height = (font_size_px * line_height_multiplier).max(1.0);

        // Own the measurement inputs, then drop the `&self` borrow before the
        // mutable rasterizer borrow.
        let text = cs.text.clone();
        let cursor_byte = cs.cursor_byte;

        let Some(tr) = self.text_rasterizer.as_mut() else {
            return;
        };

        if max_lines <= 1 {
            // ── Single-line profile (hud-zlfi4): horizontal caret-follow. ──
            let (caret_x, content_width) =
                tr.measure_composer_caret(&text, cursor_byte, font_size_px, line_height_multiplier);
            self.composer_layout = ComposerLayout {
                wrap: false,
                h_scroll_px: composer_scroll_offset(
                    caret_x,
                    content_width,
                    window_width,
                    COMPOSER_TEXT_MARGIN,
                ),
                content_width,
                visible_lines: 1.0,
                total_lines: 1.0,
                vscroll_px: 0.0,
            };
            return;
        }

        // ── Multi-line profile (hud-nx7yq.1): wrap, grow upward, vscroll. ──
        // Measure the DISPLAY string (caret glyph inserted) so the measured wrap
        // matches the rendered wrap and the box never clips the caret line.
        let display = composer_display_text_blink(&text, cursor_byte, true);
        let (total_lines, caret_line) = tr.measure_composer_wrapped(
            &display,
            cursor_byte,
            window_width,
            font_size_px,
            line_height_multiplier,
        );
        // Bound growth AND vertical scroll to what the composer REGION actually
        // fits (hud-nottc), not just the `max_lines` token. A short composer pane
        // (e.g. the exemplar's top input strip, ~2 lines) would otherwise grow the
        // box past its bounds while the scroll math — keyed on `max_lines` — left
        // the caret line clipped outside the visible box. `composer_input_box`
        // clamps the box height to the region too, so this keeps visible_lines,
        // the box, and vscroll mutually consistent and the caret always in view.
        let region_fit_lines =
            composer_region_fit_lines(region.height, line_height, COMPOSER_TEXT_MARGIN);
        let effective_max_lines = (max_lines as usize).min(region_fit_lines).max(1);
        let visible_lines = composer_visible_line_count(total_lines, effective_max_lines);
        let first_visible =
            composer_vertical_line_offset(caret_line, total_lines, effective_max_lines);
        self.composer_layout = ComposerLayout {
            wrap: true,
            h_scroll_px: 0.0,
            content_width: 0.0,
            visible_lines: visible_lines as f32,
            total_lines: total_lines as f32,
            vscroll_px: first_visible as f32 * line_height,
        };

        // Publish the wrapped VISUAL-LINE layout for the input thread's soft-wrap
        // vertical caret movement (hud-21o6x). Measured on the RAW draft (byte
        // space the caret uses), same wrap width as the render, so the input layer
        // maps caret byte ↔ visual row ↔ pixel x.
        let visual_layout = tr.measure_composer_visual_layout(
            &text,
            window_width,
            font_size_px,
            line_height_multiplier,
        );
        if let Ok(mut guard) = self.composer_visual_layout.lock() {
            *guard = Some(visual_layout);
        }
    }

    /// Build a [`TextItem`] for the local composer echo draft text.
    ///
    /// Returns `None` when:
    /// - `self.local_composer` is absent (no active composer).
    /// - `self.text_rasterizer` is absent (text path not initialised).
    /// - The tile does not contain the focused composer node.
    ///
    /// The caret character (`▌`, U+258C, LEFT HALF BLOCK) is inserted at the
    /// cursor byte position in the draft text so glyphon renders it as part of
    /// the normal text flow.  This avoids a separate GPU draw call for the
    /// caret while giving a visually correct block-caret appearance.
    ///
    /// When a selection is active (`cursor_byte != selection_anchor`), a
    /// [`crate::text::StyledRunItem`] with `background_color` set to
    /// `tokens.selection_bg` is emitted covering the selected byte range in the
    /// display string.  The text pipeline's `compute_inline_backdrop_quads` then
    /// renders a highlight quad behind the selected characters using glyph-level
    /// geometry — no separate geometry pass is required.
    ///
    /// ### Byte-offset accounting for the inserted caret glyph
    ///
    /// `▌` (U+258C) is 3 UTF-8 bytes.  It is inserted at `cursor_byte` in the
    /// display string, shifting all bytes after that point by +3.  The
    /// selection range `[sel_start, sel_end]` in the *original* text becomes:
    ///
    /// - Case `cursor_byte <= selection_anchor` (cursor at or before anchor):
    ///   - `display_sel_start = cursor_byte` (▌ is the first selected char)
    ///   - `display_sel_end   = selection_anchor + 3`
    /// - Case `cursor_byte > selection_anchor` (cursor after anchor):
    ///   - `display_sel_start = selection_anchor` (unshifted, before ▌)
    ///   - `display_sel_end   = cursor_byte + 3` (▌ is after the selection)
    ///
    /// The selection byte range is display-string byte offsets, so the highlight
    /// spans correctly across wrapped lines in the multi-line profile (the text
    /// pipeline positions each glyph on its own line); the offset math above is
    /// unaffected by wrapping. The run sets `fill_line_width` so the text pipeline
    /// draws the standard multi-line text-selection shape (hud-scgyw): partial
    /// first/last line, full-width interior lines.
    pub(super) fn collect_composer_text_item(
        &self,
        tile: &Tile,
        scene: &SceneGraph,
        sw: f32,
        sh: f32,
        tokens: &ComposerOverlayTokens,
    ) -> Option<crate::text::TextItem> {
        let cs = self.local_composer.as_ref()?;
        self.text_rasterizer.as_ref()?;
        let region = Self::composer_region_bounds(tile, scene, cs.node_id)?;
        let layout = self.composer_layout;
        // The full HitRegion spans the whole portal (click-anywhere-to-focus,
        // hud-v4k1h); confine the rendered draft to the input box at its bottom
        // edge so it does not stretch across the portal (hud-2zsbf). The box grows
        // upward with the wrapped-line count (hud-nx7yq.1); `visible_lines == 1`
        // reproduces the single input-line strip.
        let input_box = Self::composer_input_box(
            region,
            tokens.font_size_px,
            crate::markdown::MarkdownTokens::default().line_height_multiplier,
            layout.visible_lines,
        );

        // Insert the caret glyph at the cursor byte offset, gated by the blink
        // phase.  composer_display_text_blink handles OOB and non-char-boundary
        // offsets safely.  The blink phase is derived from the runtime's own
        // clock (compositor thread) — the model never sits in this loop.
        //
        // While a selection is active the caret marks the moving selection edge,
        // so it stays solid (no blink); the selection-offset math below relies on
        // the caret glyph being present.
        let has_selection = cs.selection_anchor != cs.cursor_byte;
        let caret_visible =
            has_selection || caret_visible_at(self.composer_caret_blink_start.elapsed());
        let display_text = composer_display_text_blink(&cs.text, cs.cursor_byte, caret_visible);

        let text_margin = COMPOSER_TEXT_MARGIN;

        // Horizontal caret-follow (hud-zlfi4, single-line profile only): shift the
        // draft LEFT by the per-frame scroll offset primed in
        // `prime_composer_scroll_offset` so the caret stays visible once the draft
        // is wider than the box.  Only the draft `pixel_x` moves; the clip stays
        // pinned to the box interior, so overflowing text is clipped (never painted
        // outside the box) and the selection run — byte-anchored relative to
        // `pixel_x` — scrolls with the text.  `0.0` in the multi-line profile,
        // which wraps instead of sliding horizontally.
        let scroll_offset = layout.h_scroll_px;

        // Convert linear-sRGB floats → sRGB u8 for TextItem (matches rgba_to_srgb_u8
        // in text.rs: RGB channels go through the sRGB transfer curve; alpha is linear).
        let to_srgb_u8 = |v: f32| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
        let to_alpha_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        let text_color = [
            to_srgb_u8(tokens.text_r),
            to_srgb_u8(tokens.text_g),
            to_srgb_u8(tokens.text_b),
            to_alpha_u8(tokens.text_a),
        ];

        let bw = (region.width - text_margin * 2.0).max(1.0);
        let line_height = (tokens.font_size_px
            * crate::markdown::MarkdownTokens::default().line_height_multiplier)
            .max(1.0);

        let _ = sw; // retained for API symmetry with other collect helpers
        let _ = sh;

        // Build a selection-highlight styled run when a non-empty selection
        // exists.  The run covers the selected characters in the *display*
        // string (which has the 3-byte ▌ inserted at `cursor_byte`).
        //
        // `cursor_byte` and `selection_anchor` are agent-provided and are
        // clamped to valid char boundaries by composer_display_text / the
        // ComposerDraft invariants; we still guard with `min(text.len())` here
        // so a stale snapshot with an out-of-range anchor cannot panic.
        let caret_utf8_len = '▌'.len_utf8(); // 3
        let styled_runs: Box<[crate::text::StyledRunItem]> = {
            let anchor = cs.selection_anchor.min(cs.text.len());
            let cursor = cs.cursor_byte.min(cs.text.len());
            if anchor != cursor {
                // Map original-text offsets to display-string offsets.
                let (display_sel_start, display_sel_end) = if cursor <= anchor {
                    // ▌ is inserted at `cursor` (= start of selection in display).
                    (cursor, anchor + caret_utf8_len)
                } else {
                    // ▌ is inserted at `cursor` (after selection end in original).
                    (anchor, cursor + caret_utf8_len)
                };
                // Clamp to display_text bounds (defensive).
                let display_len = display_text.len();
                let sel_start = display_sel_start.min(display_len);
                let sel_end = display_sel_end.min(display_len);
                if sel_start < sel_end {
                    Box::new([crate::text::StyledRunItem {
                        start_byte: sel_start,
                        end_byte: sel_end,
                        weight: None,
                        italic: false,
                        monospace: false,
                        color: None,
                        background_color: Some(tokens.selection_bg),
                        size_scale: None,
                        // Standard multi-line text-selection shape: full-width
                        // interior lines when the selection wraps (hud-scgyw).
                        fill_line_width: true,
                    }])
                } else {
                    Box::new([])
                }
            } else {
                Box::new([])
            }
        };

        // Per-profile text layout:
        // - Single-line (hud-zlfi4): lay the draft on ONE unwrapped line — layout
        //   width is the wider of the box and the measured content width plus one
        //   em of slack for the caret glyph, so word-wrap never triggers and the
        //   draft slides horizontally + clips instead of wrapping.
        // - Multi-line (hud-nx7yq.1): layout width is the box interior so glyphon
        //   word-wraps within it; layout height is the FULL wrapped-content height
        //   so every line lays out, and the box then clips + vertically scrolls it.
        let (layout_width, bounds_height) = if layout.wrap {
            let content_height = (layout.total_lines * line_height + text_margin * 2.0).max(1.0);
            (bw, content_height)
        } else {
            // One-line interior height — identical to the pre-multiline strip.
            let one_line = (input_box.height - text_margin * 2.0).max(1.0);
            (bw.max(layout.content_width + tokens.font_size_px), one_line)
        };

        Some(crate::text::TextItem {
            text: Arc::from(display_text.as_str()),
            // Shift the draft left by the horizontal caret-follow offset (0 in the
            // multi-line profile).
            pixel_x: region.x + text_margin - scroll_offset,
            // Shift the draft up by the vertical scroll offset (0 in the single-line
            // profile and until the multi-line draft exceeds the max line count).
            pixel_y: input_box.y + text_margin - layout.vscroll_px,
            bounds_width: layout_width,
            bounds_height,
            // Clip stays pinned to the input box interior so scrolled-off text
            // (horizontally or vertically) is clipped at the box edge, never painted
            // outside it. The box grows upward with the wrapped-line count (hud-nx7yq.1).
            clip_pixel_x: region.x + text_margin,
            clip_pixel_y: input_box.y,
            clip_bounds_width: bw.max(1.0),
            clip_bounds_height: input_box.height.max(1.0),
            font_size_px: tokens.font_size_px,
            font_family: tze_hud_scene::types::FontFamily::SystemSansSerif,
            font_weight: 400,
            color: text_color,
            alignment: tze_hud_scene::types::TextAlign::Start,
            overflow: tze_hud_scene::types::TextOverflow::Clip,
            outline_color: None,
            outline_width: None,
            opacity: 1.0,
            color_runs: Box::new([]),
            styled_runs,
            line_height_multiplier: crate::markdown::MarkdownTokens::default()
                .line_height_multiplier,
            viewport: crate::overflow::TruncationViewport::HeadAnchored,
        })
    }

    /// Depth-first search for the first composer-input `HitRegionNode` reachable
    /// from the tile root (a `HitRegionNode` with `accepts_composer_input`).
    ///
    /// Used to place the viewer reply echo relative to the composer even when no
    /// draft is active (post-submit `local_composer` is `None`), so history lines
    /// can render above the composer strip.
    fn find_composer_node_in_tile(tile: &Tile, scene: &SceneGraph) -> Option<SceneId> {
        fn dfs(node_id: SceneId, scene: &SceneGraph) -> Option<SceneId> {
            let node = scene.nodes.get(&node_id)?;
            if let NodeData::HitRegion(hr) = &node.data {
                if hr.accepts_composer_input {
                    return Some(node_id);
                }
            }
            for child in &node.children {
                if let Some(found) = dfs(*child, scene) {
                    return Some(found);
                }
            }
            None
        }
        dfs(tile.root_node?, scene)
    }

    /// The zone width available for viewer-echo text inside `region` (the
    /// composer region), i.e. the region interior minus the horizontal text
    /// margins.  Shared by the prime (wrap measurement) and collect (render) so
    /// the measured line count and the rendered wrap agree.
    fn viewer_echo_zone_width(region: Rect) -> f32 {
        (region.width - COMPOSER_TEXT_MARGIN * 2.0).max(1.0)
    }

    /// The retained viewer-echo entries for `tile` joined oldest-first with `\n`,
    /// so the rendered block reads top (oldest) to bottom (newest) and embedded
    /// newlines from Ctrl+Enter drafts (#992) break as their own lines (hud-pncm3).
    fn viewer_echo_joined_text(&self, tile_id: SceneId) -> Option<String> {
        let entries = self.viewer_echoes.entries_for(tile_id)?;
        Some(
            entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Measure the wrapped visual-line count of each tile's viewer-echo history
    /// once per frame (hud-pncm3), storing it in `viewer_echo_line_counts`.
    ///
    /// Word-wrap measurement needs the `&mut` text rasterizer (font metrics),
    /// which the `&self` `collect_viewer_echo_text_items` cannot reach — so it is
    /// primed here, mirroring `prime_composer_scroll_offset`. Must run once per
    /// frame BEFORE the text pass. Off the transcript hot path: it runs only when
    /// echoes exist and over a bounded (`MAX_VIEWER_ECHO_ENTRIES`) history.
    pub(crate) fn prime_viewer_echo_layout(&mut self, scene: &SceneGraph) {
        self.viewer_echo_line_counts.clear();
        if self.viewer_echoes.is_empty() || self.text_rasterizer.is_none() {
            return;
        }
        let lhm = crate::markdown::MarkdownTokens::default().line_height_multiplier;
        let echo_font = resolve_viewer_echo_tokens(&self.token_map).font_size_px;

        // Gather (tile, zone_width, joined_text) under &self first, then measure
        // under &mut self.text_rasterizer — the two borrows do not overlap.
        let mut jobs: Vec<(SceneId, f32, String)> = Vec::new();
        for tile in scene.visible_tiles() {
            let Some(composer_node) = Self::find_composer_node_in_tile(tile, scene) else {
                continue;
            };
            let Some(region) = Self::composer_region_bounds(tile, scene, composer_node) else {
                continue;
            };
            let Some(joined) = self.viewer_echo_joined_text(tile.id) else {
                continue;
            };
            jobs.push((tile.id, Self::viewer_echo_zone_width(region), joined));
        }

        let mut counts: Vec<(SceneId, usize)> = Vec::with_capacity(jobs.len());
        if let Some(tr) = self.text_rasterizer.as_mut() {
            for (tile_id, zone_width, joined) in &jobs {
                let (total_lines, _) =
                    tr.measure_composer_wrapped(joined, 0, *zone_width, echo_font, lhm);
                counts.push((*tile_id, total_lines.max(1)));
            }
        }
        for (tile_id, count) in counts {
            self.viewer_echo_line_counts.insert(tile_id, count);
        }
    }

    /// Collect kind-distinct `TextItem`s for the runtime-authored viewer reply
    /// echo (hud-nx7yq.3), as a single wrapped block bottom-aligned to the top of
    /// the LIVE composer input box — newest reply nearest the composer. Anchoring
    /// to the current `visible_lines`-aware box (not the fixed single-line strip)
    /// keeps the echo history riding above a growing multi-line draft rather than
    /// colliding with it (hud-xgtuf). The block word-wraps to the zone width and
    /// honors embedded newlines (hud-pncm3); the oldest lines clip first when the
    /// history would exceed the band above the composer box.
    ///
    /// Returns an empty vec when the tile has no retained viewer echoes, no
    /// composer node to anchor to, or the text rasterizer is unavailable. Lines
    /// that would land above the composer region are dropped (bounded window).
    /// Each line fades with the tile's effective opacity so the echo redacts /
    /// hides in lockstep with the surface it belongs to.
    pub(super) fn collect_viewer_echo_text_items(
        &self,
        tile: &Tile,
        scene: &SceneGraph,
        sw: f32,
        sh: f32,
        tokens: &super::token_colors::ViewerEchoTokens,
    ) -> Vec<crate::text::TextItem> {
        let _ = (sw, sh); // retained for API symmetry with sibling collect helpers
        let mut items = Vec::new();
        if self.text_rasterizer.is_none() {
            return items;
        }
        let Some(joined) = self.viewer_echo_joined_text(tile.id) else {
            return items;
        };
        let Some(composer_node) = Self::find_composer_node_in_tile(tile, scene) else {
            return items;
        };
        let Some(region) = Self::composer_region_bounds(tile, scene, composer_node) else {
            return items;
        };

        let line_height_multiplier =
            crate::markdown::MarkdownTokens::default().line_height_multiplier;
        // Anchor the history block to the TOP of the LIVE composer input box —
        // the same `visible_lines`-aware box the draft render uses (hud-xgtuf).
        // Post-submit the box rests at one line (composer_layout resets to
        // default each frame), but while the viewer types a multi-line draft the
        // box grows upward; anchoring here keeps the echo riding above the live
        // draft instead of the fixed single-line position it would otherwise grow
        // into. The box is measured with the COMPOSER font (matching the draft box
        // exactly), while the echo lines use their own font below.
        let composer_font_size_px = resolve_composer_overlay_tokens(&self.token_map).font_size_px;
        let draft_box = Self::composer_input_box(
            region,
            composer_font_size_px,
            line_height_multiplier,
            self.composer_layout.visible_lines,
        );
        let line_h = (tokens.font_size_px * line_height_multiplier).max(1.0);
        let margin = COMPOSER_TEXT_MARGIN;
        let opacity = self.tile_effective_opacity(tile, scene);
        let zone_width = Self::viewer_echo_zone_width(region);

        // The band available for history: from the region top down to the box top.
        let band_top = region.y;
        let band_height = draft_box.y - band_top;
        if band_height <= 0.0 {
            return items;
        }

        // Total WRAPPED visual-line count of the joined history: primed
        // (wrap-accurate) or, absent a prime this frame, the logical `\n`-split
        // count so embedded newlines still lay out one-line-per-break (hud-pncm3).
        let total_lines = self
            .viewer_echo_line_counts
            .get(&tile.id)
            .copied()
            .unwrap_or_else(|| joined.split('\n').count())
            .max(1);
        let block_height = (total_lines as f32 * line_h).max(line_h);

        // Bottom-align the block so the NEWEST reply sits just above the composer
        // box; the block grows upward. When the history is taller than the band,
        // the top (oldest) lines fall above `band_top` and are clipped by the
        // scissor below — the newest replies stay visible and the bound holds.
        let block_top = draft_box.y - block_height;

        items.push(crate::text::TextItem {
            text: Arc::from(joined.as_str()),
            pixel_x: region.x + margin,
            pixel_y: block_top,
            // Wrap to the zone width (Wrap::Word in the render path) so long
            // replies wrap instead of overflowing, and embedded `\n`s break.
            bounds_width: zone_width,
            bounds_height: block_height,
            // Scissor to the band between the region top and the live box top:
            // oldest lines clip first; the block never intrudes into the box.
            clip_pixel_x: region.x + margin,
            clip_pixel_y: band_top,
            clip_bounds_width: zone_width,
            clip_bounds_height: band_height,
            font_size_px: tokens.font_size_px,
            font_family: tze_hud_scene::types::FontFamily::SystemSansSerif,
            font_weight: 400,
            color: tokens.color,
            alignment: tze_hud_scene::types::TextAlign::Start,
            overflow: tze_hud_scene::types::TextOverflow::Clip,
            outline_color: None,
            outline_width: None,
            opacity,
            color_runs: Box::new([]),
            styled_runs: Box::new([]),
            line_height_multiplier,
            viewport: crate::overflow::TruncationViewport::HeadAnchored,
        });
        items
    }

    /// Render a node and its children within a tile.
    // Lint suppressed deliberately: `render_node` is a recursive tree walk.
    // `too_many_arguments` — the args are the node id plus the four distinct
    // output/scene buffers and two surface dimensions threaded unchanged through
    // every recursion; a context struct would add indirection on a hot path
    // without reducing the real fan-out. `only_used_in_recursion` — `tile`,
    // `sw`, and `sh` are forwarded to child calls, which is the intended shape.
    #[allow(clippy::only_used_in_recursion, clippy::too_many_arguments)]
    pub(super) fn render_node(
        &self,
        node_id: SceneId,
        tile: &Tile,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        textured_cmds: &mut Vec<TexturedDrawCmd>,
        sw: f32,
        sh: f32,
    ) {
        let node = match scene.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };
        let (scroll_x, scroll_y) = self.display_tile_scroll_offset(scene, tile.id);
        // §6.3 fade: the whole tile (backdrop + content backgrounds + text) must
        // fade as one unit. `tile_background_color` and `collect_text_items`
        // already apply this; the content-node backgrounds below MUST match, or a
        // faded/resized tile shows the content background at full opacity while
        // the flat backdrop around it goes see-through (hud-w41ef).
        let tile_opacity = self.tile_effective_opacity(tile, scene);

        match &node.data {
            NodeData::SolidColor(sc) => {
                if !sc.radius.is_some_and(|r| r > 0.0) {
                    Self::append_clipped_rect_vertices(
                        tile,
                        Rect::new(
                            tile.bounds.x + sc.bounds.x - scroll_x,
                            tile.bounds.y + sc.bounds.y - scroll_y,
                            sc.bounds.width,
                            sc.bounds.height,
                        ),
                        sw,
                        sh,
                        self.gpu_color(Rgba {
                            a: sc.color.a * tile_opacity,
                            ..sc.color
                        }),
                        vertices,
                    );
                }
            }
            NodeData::TextMarkdown(tm) => {
                if self.text_rasterizer.is_some() {
                    if let Some(bg) = tm.background {
                        Self::append_clipped_rect_vertices(
                            tile,
                            Rect::new(
                                tile.bounds.x + tm.bounds.x - scroll_x,
                                tile.bounds.y + tm.bounds.y - scroll_y,
                                tm.bounds.width,
                                tm.bounds.height,
                            ),
                            sw,
                            sh,
                            self.gpu_color(Rgba {
                                a: bg.a * tile_opacity,
                                ..bg
                            }),
                            vertices,
                        );
                    }
                    // Code panel backdrop quads: emitted behind fenced/indented code
                    // blocks when the `color.code.background` design token is set.
                    //
                    // Phase-1 geometry approximation: glyph pixel positions are not
                    // available in `render_node` (the geometry pass runs before the
                    // text rasterizer), so block panels are positioned by counting
                    // lines in the parsed plain text up to the panel byte range.
                    // Inline panel pixel-exact geometry is deferred to Phase 2.
                    if let Some(code_bg) = self.markdown_tokens.code_background {
                        // Load the current snapshot lock-free (hud-33qo7); pinned
                        // by the Arc for the lookup + panel emit below.
                        let markdown_cache = self.markdown_cache();
                        if let Some(key) = self.node_key_cache.get(&node_id) {
                            if let Some(parsed) = markdown_cache.get_by_key(key) {
                                let line_height = tm.font_size_px * 1.4;
                                let panel_margin_x = 4.0_f32;
                                let panel_pad_y = 2.0_f32;
                                let plain = parsed.plain_text.as_ref();
                                for panel in &parsed.code_panels {
                                    use crate::markdown::CodePanelKind;
                                    if !matches!(panel.kind, CodePanelKind::Block) {
                                        // Inline panels require glyph-level layout;
                                        // deferred to Phase 2.
                                        continue;
                                    }
                                    let clamped_start = panel.start_byte.min(plain.len());
                                    let clamped_end = panel.end_byte.min(plain.len());
                                    if clamped_start >= clamped_end
                                        || !plain.is_char_boundary(clamped_start)
                                        || !plain.is_char_boundary(clamped_end)
                                    {
                                        continue;
                                    }
                                    // Count lines before the panel to get y-offset,
                                    // and lines within the panel to get height.
                                    // Use `.lines().count()` (not newline counting) so that a
                                    // code block without a trailing newline (e.g. "a\nb") is
                                    // correctly measured as 2 lines rather than 1.
                                    let lines_before = plain[..clamped_start]
                                        .chars()
                                        .filter(|&c| c == '\n')
                                        .count();
                                    let lines_in_panel =
                                        plain[clamped_start..clamped_end].lines().count().max(1);
                                    let panel_y_offset =
                                        lines_before as f32 * line_height - panel_pad_y;
                                    let panel_height =
                                        lines_in_panel as f32 * line_height + panel_pad_y * 2.0;
                                    let panel_x =
                                        tile.bounds.x + tm.bounds.x + panel_margin_x - scroll_x;
                                    let panel_y =
                                        tile.bounds.y + tm.bounds.y + panel_y_offset - scroll_y;
                                    let panel_w = (tm.bounds.width - panel_margin_x * 2.0).max(0.0);
                                    if panel_w > 0.0 && panel_height > 0.0 {
                                        Self::append_clipped_rect_vertices(
                                            tile,
                                            Rect::new(panel_x, panel_y, panel_w, panel_height),
                                            sw,
                                            sh,
                                            self.gpu_color(code_bg),
                                            vertices,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Transcript turn separators: thin token-styled divider quads on
                    // thematic-break (`---`) lines (hud-nx7yq.4). Same line-counted
                    // geometry approximation as code panels. Content-free geometry —
                    // no text, so nothing is revealed under redaction (the transcript
                    // units are zeroed upstream when redacted, removing the breaks).
                    if let Some(sep_color) = self.markdown_tokens.separator_color {
                        let markdown_cache = self.markdown_cache();
                        if let Some(key) = self.node_key_cache.get(&node_id) {
                            if let Some(parsed) = markdown_cache.get_by_key(key) {
                                let line_height = tm.font_size_px * 1.4;
                                let thickness =
                                    self.markdown_tokens.separator_thickness_px.max(1.0);
                                let divider_color = self.gpu_color(Rgba {
                                    a: sep_color.a * tile_opacity,
                                    ..sep_color
                                });
                                let rects = transcript_separator_rects(
                                    parsed.plain_text.as_ref(),
                                    &parsed.thematic_breaks,
                                    tile.bounds.x + tm.bounds.x - scroll_x,
                                    tile.bounds.y + tm.bounds.y - scroll_y,
                                    tm.bounds.width,
                                    line_height,
                                    thickness,
                                );
                                for rect in rects {
                                    Self::append_clipped_rect_vertices(
                                        tile,
                                        rect,
                                        sw,
                                        sh,
                                        divider_color,
                                        vertices,
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // Fallback when glyphon is unavailable: preserve the old
                    // placeholder treatment so text tiles remain visible.
                    let bg = tm.background.unwrap_or(TILE_BG_TEXT_MARKDOWN);
                    Self::append_clipped_rect_vertices(
                        tile,
                        Rect::new(
                            tile.bounds.x + tm.bounds.x - scroll_x,
                            tile.bounds.y + tm.bounds.y - scroll_y,
                            tm.bounds.width,
                            tm.bounds.height,
                        ),
                        sw,
                        sh,
                        self.gpu_color(Rgba {
                            a: bg.a * tile_opacity,
                            ..bg
                        }),
                        vertices,
                    );

                    let text_margin = 8.0;
                    if tm.bounds.width > text_margin * 2.0 && tm.bounds.height > text_margin * 2.0 {
                        Self::append_clipped_rect_vertices(
                            tile,
                            Rect::new(
                                tile.bounds.x + tm.bounds.x + text_margin - scroll_x,
                                tile.bounds.y + tm.bounds.y + text_margin - scroll_y,
                                tm.bounds.width - text_margin * 2.0,
                                (tm.font_size_px * 1.2).min(tm.bounds.height - text_margin * 2.0),
                            ),
                            sw,
                            sh,
                            self.gpu_color(Rgba {
                                a: tm.color.a * tile_opacity,
                                ..tm.color
                            }),
                            vertices,
                        );
                    }
                }
            }
            NodeData::HitRegion(hr) => {
                // HitRegion is an invisible interaction primitive. Default (no
                // state) paints nothing so siblings show through. Hover + press
                // emit sibling-over tints per RFC 0004 §6.5:
                //   hover:   add 0.1 white overlay (lightening)
                //   pressed: multiply by 0.85 (~15% darken; approximated with
                //            a black overlay at 0.15 alpha against the sibling)
                // Per-node overrides come from `local_style.hover_tint` /
                // `pressed_tint` when set.
                let state = scene.hit_region_states.get(&node_id);
                let tint = match state {
                    Some(s) if s.pressed => Some(
                        hr.local_style
                            .pressed_tint
                            .map(|c| c.to_array())
                            .unwrap_or([0.0, 0.0, 0.0, 0.15]),
                    ),
                    Some(s) if s.hovered => Some(
                        hr.local_style
                            .hover_tint
                            .map(|c| c.to_array())
                            .unwrap_or([1.0, 1.0, 1.0, 0.1]),
                    ),
                    _ => None,
                };
                if let Some(color) = tint {
                    Self::append_clipped_rect_vertices(
                        tile,
                        Rect::new(
                            tile.bounds.x + hr.bounds.x - scroll_x,
                            tile.bounds.y + hr.bounds.y - scroll_y,
                            hr.bounds.width,
                            hr.bounds.height,
                        ),
                        sw,
                        sh,
                        self.gpu_color_raw(color),
                        vertices,
                    );
                }

                // The keyboard focus ring is NOT drawn here: it moved to the
                // chrome-layer pass (`append_focus_ring_vertices`, hud-k6yvb) so it
                // renders above all agent content (input-model §416) and covers
                // tile-level / composer-less focus owners the per-node scene state
                // cannot express.
            }
            NodeData::StaticImage(img) => {
                // If a GPU texture is cached for this resource, emit a textured
                // draw command with fit-mode UV calculations.
                if let Some(entry) = self.image_texture_cache.get(&img.resource_id) {
                    let (dx, dy, dw, dh, uv_rect) = compute_fit_mode(
                        img.fit_mode,
                        tile.bounds.x + img.bounds.x - scroll_x,
                        tile.bounds.y + img.bounds.y - scroll_y,
                        img.bounds.width,
                        img.bounds.height,
                        entry.width,
                        entry.height,
                    );
                    if let Some((clipped, clipped_uv)) =
                        Self::clipped_textured_rect(tile, Rect::new(dx, dy, dw, dh), uv_rect)
                    {
                        textured_cmds.push(TexturedDrawCmd {
                            resource_id: img.resource_id,
                            x: clipped.x,
                            y: clipped.y,
                            w: clipped.width,
                            h: clipped.height,
                            uv_rect: clipped_uv,
                            // Fade the image with the whole tile (drag + §6.3 portal
                            // transition), matching the tile backdrop/text so a
                            // faded/resized tile stays uniform (hud-b0x0m).
                            tint: [1.0, 1.0, 1.0, tile_opacity],
                        });
                    }
                } else {
                    // Fallback: warm-gray placeholder when bytes not registered.
                    let outer_color = [0.55_f32, 0.50, 0.45, tile_opacity];
                    Self::append_clipped_rect_vertices(
                        tile,
                        Rect::new(
                            tile.bounds.x + img.bounds.x - scroll_x,
                            tile.bounds.y + img.bounds.y - scroll_y,
                            img.bounds.width,
                            img.bounds.height,
                        ),
                        sw,
                        sh,
                        self.gpu_color_raw(outer_color),
                        vertices,
                    );

                    let margin = 4.0_f32;
                    if img.bounds.width > margin * 2.0 && img.bounds.height > margin * 2.0 {
                        let accent_color = [0.75_f32, 0.70, 0.65, tile_opacity];
                        Self::append_clipped_rect_vertices(
                            tile,
                            Rect::new(
                                tile.bounds.x + img.bounds.x + margin - scroll_x,
                                tile.bounds.y + img.bounds.y + margin - scroll_y,
                                img.bounds.width - margin * 2.0,
                                img.bounds.height - margin * 2.0,
                            ),
                            sw,
                            sh,
                            self.gpu_color_raw(accent_color),
                            vertices,
                        );
                    }
                }
            }
        }

        // Render children
        for child_id in &node.children {
            self.render_node(*child_id, tile, scene, vertices, textured_cmds, sw, sh);
        }
    }
}

#[cfg(test)]
mod lifecycle_accent_tests {
    use super::*;

    fn rect(w: f32, h: f32) -> Rect {
        Rect::new(0.0, 0.0, w, h)
    }

    /// The accent bar passes the token color through unchanged and clamps its
    /// width to the tile, folding tile opacity into the alpha (hud-m48i0).
    #[test]
    fn accent_bar_clamps_width_and_folds_opacity() {
        let accent = LifecycleAccent {
            color: Rgba::new(0.2, 0.4, 0.6, 1.0),
            width_px: 4.0,
        };
        let (w, color) =
            Compositor::lifecycle_accent_bar_geom(rect(200.0, 150.0), accent, 0.5).unwrap();
        assert_eq!(w, 4.0, "in-range width is used as-is");
        assert_eq!(
            [color[0], color[1], color[2]],
            [0.2, 0.4, 0.6],
            "color passes through"
        );
        assert!(
            (color[3] - 0.5).abs() < 1e-6,
            "tile opacity folds into alpha"
        );

        // Oversized token width is clamped to the tile width.
        let wide = LifecycleAccent {
            color: Rgba::WHITE,
            width_px: 9999.0,
        };
        let (w, _) = Compositor::lifecycle_accent_bar_geom(rect(12.0, 80.0), wide, 1.0).unwrap();
        assert_eq!(w, 12.0, "width is clamped to the tile edge");
    }

    /// Zero width or zero opacity paints nothing.
    #[test]
    fn accent_bar_none_when_invisible() {
        let zero_w = LifecycleAccent {
            color: Rgba::WHITE,
            width_px: 0.0,
        };
        assert!(Compositor::lifecycle_accent_bar_geom(rect(100.0, 50.0), zero_w, 1.0).is_none());
        let some = LifecycleAccent {
            color: Rgba::WHITE,
            width_px: 4.0,
        };
        assert!(Compositor::lifecycle_accent_bar_geom(rect(100.0, 50.0), some, 0.0).is_none());
    }
}
