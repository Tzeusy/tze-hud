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
use super::image_cache::{caret_visible_at, composer_display_text_blink};
use super::token_colors::{
    ComposerOverlayTokens, TILE_BG_DEFAULT, TILE_BG_STATIC_IMAGE, TILE_BG_TEXT_MARKDOWN,
    linear_to_srgb, resolve_tile_bg_token,
};

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

        // Background fill.
        let bg_color = [tokens.bg_r, tokens.bg_g, tokens.bg_b, tokens.bg_a];
        vertices.extend_from_slice(&rect_vertices(
            region.x,
            region.y,
            region.width,
            region.height,
            sw,
            sh,
            bg_color,
        ));

        // At-capacity left-edge accent (2px wide, full strip height).
        if cs.at_capacity {
            let accent = [
                tokens.at_capacity_r,
                tokens.at_capacity_g,
                tokens.at_capacity_b,
                tokens.at_capacity_a,
            ];
            vertices.extend_from_slice(&rect_vertices(
                region.x,
                region.y,
                2.0,
                region.height,
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
    /// NOTE: single-line selection only.  Multi-line composer is out of scope
    /// for v1 (the composer strip is always one line); a follow-up bead covers
    /// multi-line layouts when they land.
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

        let text_margin = 6.0;

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
        let bh = (region.height - text_margin * 2.0).max(1.0);

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
                    }])
                } else {
                    Box::new([])
                }
            } else {
                Box::new([])
            }
        };

        Some(crate::text::TextItem {
            text: Arc::from(display_text.as_str()),
            pixel_x: region.x + text_margin,
            pixel_y: region.y + text_margin,
            bounds_width: bw,
            bounds_height: bh,
            clip_pixel_x: region.x + text_margin,
            clip_pixel_y: region.y,
            clip_bounds_width: bw.max(1.0),
            clip_bounds_height: region.height.max(1.0),
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
                        self.gpu_color(sc.color),
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
                            self.gpu_color(tm.color),
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
                            tint: [1.0, 1.0, 1.0, Self::effective_tile_opacity(tile, scene)],
                        });
                    }
                } else {
                    // Fallback: warm-gray placeholder when bytes not registered.
                    let outer_color = [0.55_f32, 0.50, 0.45, 1.0];
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
                        let accent_color = [0.75_f32, 0.70, 0.65, 1.0];
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
