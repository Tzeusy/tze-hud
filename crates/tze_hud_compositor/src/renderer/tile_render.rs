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
use super::image_cache::composer_display_text;
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
        // Base opacity from drag state (scene-level tile.opacity × drag boost).
        let base_opacity = Self::effective_tile_opacity(tile, scene);
        // §6.3: multiply portal tile animation opacity for scrollable (portal) tiles.
        let opacity = (base_opacity * self.portal_tile_anim_opacity(tile.id)).clamp(0.0, 1.0);
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

    /// Emit geometry for the local composer echo overlay.
    ///
    /// Called from the tile loop (after the content pass) when
    /// `self.local_composer` is `Some` and the state's `node_id` belongs to a
    /// `HitRegionNode` with `accepts_composer_input = true` inside the given
    /// tile.  If no matching node is found in the tile the call is a no-op.
    ///
    /// Emits:
    /// 1. A background fill rect for the composer strip (bottom
    ///    `font_size_px * 1.6` pixels of the tile).
    /// 2. When at_capacity, a 2px left-edge accent rect using the at-capacity
    ///    color so the user has a visual signal that no further input is
    ///    accepted.  The text pass (see `collect_composer_text_item`) renders
    ///    the draft text on top.
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

        // Only render inside the tile that owns the focused composer node.
        if !self.tile_contains_composer_node(tile, scene, cs.node_id) {
            return;
        }

        let strip_h = (tokens.font_size_px * 1.6).max(20.0);
        let strip_y = tile.bounds.y + tile.bounds.height - strip_h;
        let strip_y = strip_y.max(tile.bounds.y); // never above tile top

        // Background fill.
        let bg_color = [tokens.bg_r, tokens.bg_g, tokens.bg_b, tokens.bg_a];
        vertices.extend_from_slice(&rect_vertices(
            tile.bounds.x,
            strip_y,
            tile.bounds.width,
            strip_h,
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
                tile.bounds.x,
                strip_y,
                2.0,
                strip_h,
                sw,
                sh,
                accent,
            ));
        }
    }

    /// Return `true` when `node_id` is a `HitRegionNode` with
    /// `accepts_composer_input = true` that lives somewhere in the subtree
    /// rooted at `tile.root_node`.
    ///
    /// The search is bounded by the tile's node tree; the common case (one root
    /// node with a few children) terminates in O(n) with n typically ≤ 10.
    fn tile_contains_composer_node(
        &self,
        tile: &Tile,
        scene: &SceneGraph,
        target: SceneId,
    ) -> bool {
        let Some(root_id) = tile.root_node else {
            return false;
        };
        self.node_tree_contains(root_id, scene, target)
    }

    /// Depth-first search for `target` in the node sub-tree rooted at `node_id`.
    #[allow(clippy::only_used_in_recursion)]
    fn node_tree_contains(&self, node_id: SceneId, scene: &SceneGraph, target: SceneId) -> bool {
        if node_id == target {
            // Verify the node is indeed a composer-capable HitRegion.
            return scene.nodes.get(&node_id).is_some_and(|n| match &n.data {
                NodeData::HitRegion(hr) => hr.accepts_composer_input,
                _ => false,
            });
        }
        if let Some(node) = scene.nodes.get(&node_id) {
            for child in &node.children {
                if self.node_tree_contains(*child, scene, target) {
                    return true;
                }
            }
        }
        false
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
        if !self.tile_contains_composer_node(tile, scene, cs.node_id) {
            return None;
        }

        // Insert the caret glyph at the cursor byte offset.
        // composer_display_text handles OOB and non-char-boundary offsets safely.
        let display_text = composer_display_text(&cs.text, cs.cursor_byte);

        let strip_h = (tokens.font_size_px * 1.6).max(20.0);
        let strip_y = (tile.bounds.y + tile.bounds.height - strip_h).max(tile.bounds.y);
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

        let bw = (tile.bounds.width - text_margin * 2.0).max(1.0);
        let bh = (strip_h - text_margin).max(1.0);

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
            pixel_x: tile.bounds.x + text_margin,
            pixel_y: strip_y + text_margin * 0.5,
            bounds_width: bw,
            bounds_height: bh,
            clip_pixel_x: tile.bounds.x + text_margin,
            clip_pixel_y: strip_y,
            clip_bounds_width: bw.max(1.0),
            clip_bounds_height: strip_h.max(1.0),
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
        let (scroll_x, scroll_y) = scene.tile_scroll_offset_local(tile.id);

        match &node.data {
            NodeData::SolidColor(sc) => {
                if !sc.radius.is_some_and(|r| r > 0.0) {
                    let verts = rect_vertices(
                        tile.bounds.x + sc.bounds.x - scroll_x,
                        tile.bounds.y + sc.bounds.y - scroll_y,
                        sc.bounds.width,
                        sc.bounds.height,
                        sw,
                        sh,
                        self.gpu_color(sc.color),
                    );
                    vertices.extend_from_slice(&verts);
                }
            }
            NodeData::TextMarkdown(tm) => {
                if self.text_rasterizer.is_some() {
                    if let Some(bg) = tm.background {
                        let verts = rect_vertices(
                            tile.bounds.x + tm.bounds.x - scroll_x,
                            tile.bounds.y + tm.bounds.y - scroll_y,
                            tm.bounds.width,
                            tm.bounds.height,
                            sw,
                            sh,
                            self.gpu_color(bg),
                        );
                        vertices.extend_from_slice(&verts);
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
                        if let Some(key) = self.node_key_cache.get(&node_id) {
                            if let Some(parsed) = self.markdown_cache.get_by_key(key) {
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
                                        let verts = rect_vertices(
                                            panel_x,
                                            panel_y,
                                            panel_w,
                                            panel_height,
                                            sw,
                                            sh,
                                            self.gpu_color(code_bg),
                                        );
                                        vertices.extend_from_slice(&verts);
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Fallback when glyphon is unavailable: preserve the old
                    // placeholder treatment so text tiles remain visible.
                    let bg = tm.background.unwrap_or(TILE_BG_TEXT_MARKDOWN);
                    let verts = rect_vertices(
                        tile.bounds.x + tm.bounds.x - scroll_x,
                        tile.bounds.y + tm.bounds.y - scroll_y,
                        tm.bounds.width,
                        tm.bounds.height,
                        sw,
                        sh,
                        self.gpu_color(bg),
                    );
                    vertices.extend_from_slice(&verts);

                    let text_margin = 8.0;
                    if tm.bounds.width > text_margin * 2.0 && tm.bounds.height > text_margin * 2.0 {
                        let verts = rect_vertices(
                            tile.bounds.x + tm.bounds.x + text_margin - scroll_x,
                            tile.bounds.y + tm.bounds.y + text_margin - scroll_y,
                            tm.bounds.width - text_margin * 2.0,
                            (tm.font_size_px * 1.2).min(tm.bounds.height - text_margin * 2.0),
                            sw,
                            sh,
                            self.gpu_color(tm.color),
                        );
                        vertices.extend_from_slice(&verts);
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
                    let verts = rect_vertices(
                        tile.bounds.x + hr.bounds.x - scroll_x,
                        tile.bounds.y + hr.bounds.y - scroll_y,
                        hr.bounds.width,
                        hr.bounds.height,
                        sw,
                        sh,
                        self.gpu_color_raw(color),
                    );
                    vertices.extend_from_slice(&verts);
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
                    textured_cmds.push(TexturedDrawCmd {
                        resource_id: img.resource_id,
                        x: dx,
                        y: dy,
                        w: dw,
                        h: dh,
                        uv_rect,
                        tint: [1.0, 1.0, 1.0, Self::effective_tile_opacity(tile, scene)],
                    });
                } else {
                    // Fallback: warm-gray placeholder when bytes not registered.
                    let outer_color = [0.55_f32, 0.50, 0.45, 1.0];
                    let verts = rect_vertices(
                        tile.bounds.x + img.bounds.x - scroll_x,
                        tile.bounds.y + img.bounds.y - scroll_y,
                        img.bounds.width,
                        img.bounds.height,
                        sw,
                        sh,
                        self.gpu_color_raw(outer_color),
                    );
                    vertices.extend_from_slice(&verts);

                    let margin = 4.0_f32;
                    if img.bounds.width > margin * 2.0 && img.bounds.height > margin * 2.0 {
                        let accent_color = [0.75_f32, 0.70, 0.65, 1.0];
                        let verts = rect_vertices(
                            tile.bounds.x + img.bounds.x + margin - scroll_x,
                            tile.bounds.y + img.bounds.y + margin - scroll_y,
                            img.bounds.width - margin * 2.0,
                            img.bounds.height - margin * 2.0,
                            sw,
                            sh,
                            self.gpu_color_raw(accent_color),
                        );
                        vertices.extend_from_slice(&verts);
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
