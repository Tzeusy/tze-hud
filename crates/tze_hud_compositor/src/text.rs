//! Text rasterization layer for the compositor.
//!
//! Integrates [glyphon](https://github.com/grovesNL/glyphon) (cosmic-text +
//! etagere atlas) into the tze_hud render pipeline.
//!
//! # Responsibilities
//!
//! - Maintain a single `FontSystem` and `SwashCache` for the compositor
//!   lifetime.
//! - Provide `render_text_areas` which runs a glyphon text pass against an
//!   existing `RenderPassDescriptor`-compatible texture view (LoadOp::Load so
//!   prior geometry is preserved).
//! - Map `TextMarkdownNode` + zone `StreamText` content into `TextArea` slices.
//!
//! # Design constraints
//!
//! - **No Markdown parsing in hot path.** CommonMark rendering is limited to
//!   plain text for v1-MVP; `#`-headed lines are treated as bold weight, `*...*`
//!   as italic style.  Full Markdown parsing is deferred.
//! - **No per-frame atlas trim.** `atlas.trim()` is called once per frame after
//!   the text pass to reclaim evicted glyph slabs.
//! - **glyphon `Buffer` allocation per text item per frame.** This avoids
//!   statefulness between frames at the cost of per-frame layout; acceptable
//!   for v1 because Stage 6 budget is 4 ms p99 and glyphon layout is fast.
//!
//! # Overflow modes
//!
//! - `Clip` — the `TextBounds` rectangle hard-clips rendered glyphs.
//! - `Ellipsis` — glyphon does not natively support trailing ellipsis; we
//!   approximate it by setting the cosmic-text `Wrap::Word` and truncating the
//!   visible line count to fit the bounds, appending "…" if truncation occurs.
//!   Full ellipsis support is deferred to post-MVP.

use std::collections::{BTreeMap, HashSet};

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use tze_hud_scene::types::{
    FontFamily, RenderingPolicy, Rgba, TextAlign, TextMarkdownNode, TextOverflow,
};
use wgpu::{Device, MultisampleState, Queue};

// ─── TextRasterizer ───────────────────────────────────────────────────────────

/// Compositor-owned glyphon state.
///
/// Created once per `Compositor` via [`TextRasterizer::new`]. Holds the
/// font system, glyph atlas, and renderer. Not `Send` — must stay on the
/// compositor thread.
pub struct TextRasterizer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    /// Content-addressed IDs of agent-uploaded fonts already loaded into
    /// `font_system`.  Used to skip redundant `load_font_data` calls (which
    /// would add duplicate entries to fontdb).
    ///
    /// The ID is the raw 32-byte BLAKE3 digest (`ResourceId` wire form) of
    /// the font bytes, matching the key used by `tze_hud_resource::FontBytesStore`.
    loaded_font_ids: HashSet<[u8; 32]>,
}

impl TextRasterizer {
    /// Create a text rasterizer targeting the given surface format.
    ///
    /// This must be called after the wgpu `Device` and `Queue` are available
    /// (i.e. after `Compositor::new_headless` or `new_windowed`).
    pub fn new(device: &Device, queue: &Queue, format: wgpu::TextureFormat) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer = TextRenderer::new(&mut atlas, device, MultisampleState::default(), None);

        Self {
            font_system,
            swash_cache,
            viewport,
            atlas,
            renderer,
            loaded_font_ids: HashSet::new(),
        }
    }

    /// Load raw font bytes (TTF or OTF) into glyphon's `FontSystem`.
    ///
    /// After this call the font is available for text layout via glyphon's
    /// automatic family detection.  Subsequent `TextItem`s whose `font_family`
    /// resolves to the family embedded in these bytes will use them.
    ///
    /// # Parameters
    ///
    /// - `resource_id` — the 32-byte BLAKE3 content hash of `data`
    ///   (matches `ResourceId::as_bytes()` from `tze_hud_resource`).
    ///   Used to deduplicate: calling this with the same `resource_id` twice
    ///   is a no-op after the first call.
    /// - `data` — raw TTF or OTF bytes.
    ///
    /// # Thread safety
    ///
    /// `TextRasterizer` is `!Send` — this must be called from the compositor
    /// thread only (same thread that calls `prepare_text_items` and
    /// `render_text_pass`).
    pub fn load_font_bytes(&mut self, resource_id: [u8; 32], data: &[u8]) {
        if self.loaded_font_ids.contains(&resource_id) {
            tracing::debug!(
                resource_id = %format_resource_id(&resource_id),
                "font already loaded — skipping duplicate load_font_data"
            );
            return;
        }

        self.font_system.db_mut().load_font_data(data.to_vec());

        self.loaded_font_ids.insert(resource_id);

        tracing::info!(
            resource_id = %format_resource_id(&resource_id),
            bytes = data.len(),
            "agent-uploaded font loaded into FontSystem"
        );
    }

    /// Number of agent-uploaded fonts currently loaded into the `FontSystem`.
    #[inline]
    pub fn loaded_font_count(&self) -> usize {
        self.loaded_font_ids.len()
    }

    /// Returns `true` if the font identified by `resource_id` has already been
    /// loaded into the `FontSystem`.
    #[inline]
    pub fn has_font(&self, resource_id: &[u8; 32]) -> bool {
        self.loaded_font_ids.contains(resource_id)
    }

    /// Total number of font faces visible to glyphon's `FontSystem`.
    #[inline]
    pub fn font_face_count(&self) -> usize {
        self.font_system.db().faces().count()
    }

    /// Update the viewport resolution before each frame.
    ///
    /// Must be called once per frame before `render_text_pass`.
    pub fn update_viewport(&mut self, queue: &Queue, width: u32, height: u32) {
        self.viewport.update(queue, Resolution { width, height });
    }

    /// Prepare text areas for the upcoming render pass.
    ///
    /// Collects all `TextItem`s, builds glyphon `Buffer`s, and calls
    /// `renderer.prepare`. Must be called after `update_viewport` and before
    /// `render_text_pass`.
    ///
    /// When a `TextItem` has `outline_color` and `outline_width > 0`, the text
    /// is rendered 9 times: once at each of the 8 cardinal+diagonal pixel offsets
    /// in `outline_color`, then once more in the fill `color` on top.
    ///
    /// Returns `Ok(())` on success, or a string on glyphon error (non-fatal —
    /// the frame continues with missing text rather than a crash).
    pub fn prepare_text_items(
        &mut self,
        device: &Device,
        queue: &Queue,
        items: &[TextItem],
    ) -> Result<(), String> {
        // 8-direction offsets for outline rendering (cardinal + diagonal).
        const OUTLINE_DIRS: [(f32, f32); 8] = [
            (-1.0, 0.0),
            (1.0, 0.0),
            (0.0, -1.0),
            (0.0, 1.0),
            (-1.0, -1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (1.0, 1.0),
        ];

        // Each item with outline produces 9 TextAreas (8 outline + 1 fill).
        // Items without outline produce 1 TextArea each.
        // We build all Buffers first, then construct TextArea references.

        // Phase 1: build one Buffer per item (shared by all outline + fill passes).
        let buffers: Vec<Buffer> = items
            .iter()
            .map(|item| {
                let line_height = item.font_size_px * 1.4;
                let mut buf = Buffer::new(
                    &mut self.font_system,
                    Metrics::new(item.font_size_px, line_height),
                );
                buf.set_size(
                    &mut self.font_system,
                    Some(item.bounds_width),
                    Some(item.bounds_height),
                );
                buf.set_wrap(&mut self.font_system, Wrap::Word);
                let family = match item.font_family {
                    FontFamily::SystemSansSerif => Family::SansSerif,
                    FontFamily::SystemMonospace => Family::Monospace,
                    FontFamily::SystemSerif => Family::Serif,
                };
                // Map CSS-style weight (100–900) to glyphon Weight.
                // Clamp to [100, 900]; Weight(0) would select arbitrary fallback fonts.
                let weight = Weight(item.font_weight.clamp(100, 900));
                let base_attrs = Attrs::new().family(family).weight(weight);

                if item.color_runs.is_empty() {
                    // Fast path: no inline runs — use uniform base color.
                    buf.set_text(
                        &mut self.font_system,
                        &item.text,
                        base_attrs,
                        Shaping::Basic,
                    );
                } else {
                    // Single-pass styled path: build (text_slice, Attrs) pairs and
                    // call set_rich_text once.  This avoids the double-shape that
                    // occurred when set_text created BufferLines and set_attrs_list
                    // then invalidated their shaping state.
                    //
                    // color_runs are sorted, non-overlapping byte ranges.  We walk
                    // the text left-to-right, emitting an unstyled span for any gap
                    // before a run, then a colored span for the run itself.  Text
                    // after the last run (if any) is emitted as a final unstyled span.
                    //
                    // `default_color` on TextArea acts as the fallback for glyphs
                    // without a color_opt set, so base_attrs carries no color_opt and
                    // run attrs carry explicit Color values.
                    let spans = color_run_spans(&item.text, &item.color_runs, base_attrs);
                    buf.set_rich_text(
                        &mut self.font_system,
                        spans,
                        base_attrs,
                        Shaping::Basic,
                    );
                }

                // Apply text alignment to all lines in the buffer.
                let ct_align = match item.alignment {
                    TextAlign::Start => glyphon::cosmic_text::Align::Left,
                    TextAlign::Center => glyphon::cosmic_text::Align::Center,
                    TextAlign::End => glyphon::cosmic_text::Align::End,
                };
                for line in buf.lines.iter_mut() {
                    line.set_align(Some(ct_align));
                }
                buf.shape_until_scroll(&mut self.font_system, false);
                buf
            })
            .collect();

        // Phase 2: build TextArea list.
        // For outlined items, we emit 8 outline passes then 1 fill pass.
        // For non-outlined items, we emit 1 fill pass.
        // Because TextArea borrows the buffer, all buffers must outlive this Vec.
        let mut text_areas: Vec<TextArea<'_>> = Vec::with_capacity(items.len() * 9);

        for (item, buf) in items.iter().zip(buffers.iter()) {
            let fill_color = item.color;
            let bounds = TextBounds {
                left: item.pixel_x as i32,
                top: item.pixel_y as i32,
                right: (item.pixel_x + item.bounds_width) as i32,
                bottom: (item.pixel_y + item.bounds_height) as i32,
            };

            // Outline passes (only when outline is active).
            if let (Some(oc), Some(ow)) = (item.outline_color, item.outline_width) {
                if ow > 0.0 {
                    for (dx, dy) in &OUTLINE_DIRS {
                        let offset = ow;
                        // Offset bounds to match shifted position.
                        let shifted_bounds = TextBounds {
                            left: (item.pixel_x + dx * offset) as i32,
                            top: (item.pixel_y + dy * offset) as i32,
                            right: (item.pixel_x + dx * offset + item.bounds_width) as i32,
                            bottom: (item.pixel_y + dy * offset + item.bounds_height) as i32,
                        };
                        text_areas.push(TextArea {
                            buffer: buf,
                            left: item.pixel_x + dx * offset,
                            top: item.pixel_y + dy * offset,
                            scale: 1.0,
                            bounds: shifted_bounds,
                            default_color: Color::rgba(oc[0], oc[1], oc[2], oc[3]),
                            custom_glyphs: &[],
                        });
                    }
                }
            }

            // Fill pass (always last so it renders on top of outline).
            text_areas.push(TextArea {
                buffer: buf,
                left: item.pixel_x,
                top: item.pixel_y,
                scale: 1.0,
                bounds,
                default_color: Color::rgba(
                    fill_color[0],
                    fill_color[1],
                    fill_color[2],
                    fill_color[3],
                ),
                custom_glyphs: &[],
            });
        }

        self.renderer
            .prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .map_err(|e| format!("glyphon prepare: {e:?}"))
    }

    /// Record the glyphon text pass into `render_pass`.
    ///
    /// The render pass must have been begun with `LoadOp::Load` so that prior
    /// geometry (rects, backgrounds) is preserved under the text.
    pub fn render_text_pass<'rp>(
        &'rp self,
        render_pass: &mut wgpu::RenderPass<'rp>,
    ) -> Result<(), String> {
        self.renderer
            .render(&self.atlas, &self.viewport, render_pass)
            .map_err(|e| format!("glyphon render: {e:?}"))
    }

    /// Trim the atlas after the frame is presented.
    ///
    /// Reclaims memory for glyphs that were not used in the last frame.
    pub fn trim_atlas(&mut self) {
        self.atlas.trim();
    }
}

// ─── TextItem ─────────────────────────────────────────────────────────────────

/// A single text rendering request — one TextMarkdownNode or one zone publish.
#[derive(Debug, Clone)]
pub struct TextItem {
    /// The text content to render (plain text for v1; Markdown stripped).
    pub text: String,
    /// Left edge in physical pixels (absolute, not tile-relative).
    pub pixel_x: f32,
    /// Top edge in physical pixels (absolute, not tile-relative).
    pub pixel_y: f32,
    /// Available width for word-wrap and clip.
    pub bounds_width: f32,
    /// Available height for clip.
    pub bounds_height: f32,
    /// Font size in pixels.
    pub font_size_px: f32,
    /// Font family selection.
    pub font_family: FontFamily,
    /// Font weight (CSS-style: 100–900); 400 = regular, 700 = bold.
    ///
    /// Mapped to `glyphon::Weight` at rasterization time.
    pub font_weight: u16,
    /// Text color as sRGB u8 bytes: [r, g, b, a].
    pub color: [u8; 4],
    /// Alignment (used for cosmic-text alignment).
    pub alignment: TextAlign,
    /// Overflow mode.
    pub overflow: TextOverflow,
    /// Outline color for 8-direction text outline; None = no outline.
    pub outline_color: Option<[u8; 4]>,
    /// Outline stroke width in pixels; None or 0.0 = no outline.
    pub outline_width: Option<f32>,
    /// Opacity multiplier (0.0–1.0) from zone animation state; 1.0 = fully opaque.
    pub opacity: f32,
    /// Inline color runs (byte offsets into `text`, which is the **post-strip**
    /// content).  Empty = use `color` for the entire text.
    ///
    /// Offsets are remapped from raw-content byte positions by
    /// `TextItem::from_text_markdown_node` when `color_runs` are present.
    /// Zone-derived `TextItem`s always carry an empty slice (no run support yet).
    pub color_runs: Box<[ColorRunItem]>,
}

/// A single resolved color run for `TextItem` rendering, with byte offsets
/// into the **post-strip** `text` string.
#[derive(Debug, Clone)]
pub struct ColorRunItem {
    /// Inclusive byte offset into `TextItem::text`.
    pub start_byte: usize,
    /// Exclusive byte offset into `TextItem::text`.
    pub end_byte: usize,
    /// sRGB u8 color: [r, g, b, a].
    pub color: [u8; 4],
}

impl TextItem {
    /// Build a `TextItem` from a `TextMarkdownNode` and its tile-relative position.
    ///
    /// `tile_x` / `tile_y` are the pixel-space position of the tile origin.
    ///
    /// When `node.color_runs` is non-empty, Markdown stripping is skipped and
    /// the raw content is used as-is, so byte offsets in the runs are preserved
    /// exactly.  Callers that supply `color_runs` must pre-strip Markdown before
    /// populating `content`, or accept that markup characters appear in the output.
    pub fn from_text_markdown_node(node: &TextMarkdownNode, tile_x: f32, tile_y: f32) -> Self {
        // When color_runs are present, skip Markdown stripping so that the raw
        // byte offsets in the runs remain valid against `text`.
        let text = if node.color_runs.is_empty() {
            strip_markdown_v1(&node.content)
        } else {
            node.content.clone()
        };

        // Convert linear f32 color [0..1] to sRGB u8 [0..255].
        let r = linear_to_srgb_u8(node.color.r);
        let g = linear_to_srgb_u8(node.color.g);
        let b = linear_to_srgb_u8(node.color.b);
        let a = (node.color.a * 255.0).clamp(0.0, 255.0) as u8;

        // Add a size-aware inset margin so large text boxes get breathing room
        // without collapsing compact HUD labels like Presence Card rows.
        let font_size_px = node.font_size_px.clamp(6.0, 200.0);
        let line_height = font_size_px * 1.4;
        let margin_x = (node.bounds.width * 0.08).clamp(1.0, 6.0);
        let target_margin_y = (node.bounds.height * 0.20).clamp(1.0, 6.0);
        let max_margin_y = ((node.bounds.height - line_height).max(0.0) / 2.0).min(6.0);
        let margin_y = target_margin_y.min(max_margin_y);
        let x = tile_x + node.bounds.x + margin_x;
        let y = tile_y + node.bounds.y + margin_y;
        let w = (node.bounds.width - margin_x * 2.0).max(1.0);
        let h = (node.bounds.height - margin_y * 2.0).max(1.0);

        // Convert scene TextColorRun (raw content offsets) to ColorRunItem
        // (text/post-strip offsets).  Since we skip stripping when runs are
        // present, the offsets map 1:1 to positions in `text`.
        let color_runs: Box<[ColorRunItem]> = node
            .color_runs
            .iter()
            .filter_map(|run| {
                // Clamp to actual text length to guard against stale runs after
                // any future content truncation.
                let start = (run.start_byte as usize).min(text.len());
                let end = (run.end_byte as usize).min(text.len());
                if start >= end {
                    return None;
                }
                // Only emit the run if both boundaries are valid UTF-8 positions.
                if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                    return None;
                }
                Some(ColorRunItem {
                    start_byte: start,
                    end_byte: end,
                    color: rgba_to_srgb_u8(run.color),
                })
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        TextItem {
            text,
            pixel_x: x,
            pixel_y: y,
            bounds_width: w,
            bounds_height: h,
            font_size_px,
            font_family: node.font_family,
            font_weight: 400, // TextMarkdownNode does not carry weight; default regular.
            color: [r, g, b, a],
            alignment: node.alignment,
            overflow: node.overflow,
            outline_color: None,
            outline_width: None,
            opacity: 1.0,
            color_runs,
        }
    }

    /// Build a `TextItem` for zone text content driven by a [`RenderingPolicy`].
    ///
    /// This is the primary factory method for zone rendering — replaces the
    /// old `from_zone_stream_text` / `from_zone_notification` hardcoded-color
    /// variants.  All visual properties are read from `policy`; no hardcoded
    /// colors or font choices.
    ///
    /// `x`, `y`, `w`, `h` are the zone geometry in physical pixels.
    /// `opacity` is the current zone animation opacity (1.0 = fully opaque).
    ///
    /// [`RenderingPolicy`]: tze_hud_scene::types::RenderingPolicy
    pub fn from_zone_policy(
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        policy: &RenderingPolicy,
        opacity: f32,
    ) -> Self {
        // Margin: prefer margin_horizontal/margin_vertical; fall back to margin_px;
        // then fall back to 8px.  Per spec §Extended RenderingPolicy: margin_horizontal
        // "overrides margin_px for the horizontal axis. When None, falls back to margin_px."
        let margin_h = policy.margin_horizontal.or(policy.margin_px).unwrap_or(8.0);
        let margin_v = policy.margin_vertical.or(policy.margin_px).unwrap_or(8.0);

        let font_size_px = policy.font_size_px.unwrap_or(16.0).clamp(6.0, 200.0);
        let font_family = policy.font_family.unwrap_or(FontFamily::SystemSansSerif);
        // font_weight: use policy value clamped to CSS weight range [100, 900]; default 400.
        let font_weight = policy.font_weight.unwrap_or(400).clamp(100, 900);
        let alignment = policy.text_align.unwrap_or(TextAlign::Start);

        // text_color: policy.text_color if present, else white.
        let color = policy
            .text_color
            .map(rgba_to_srgb_u8)
            .unwrap_or([255, 255, 255, 220]);

        // Apply opacity to the fill color alpha.
        let color = apply_opacity_to_color(color, opacity);

        // Outline: propagate only when outline_width > 0.
        let (outline_color, outline_width) = match (policy.outline_color, policy.outline_width) {
            (Some(oc), Some(ow)) if ow > 0.0 => {
                let oc_srgb = apply_opacity_to_color(rgba_to_srgb_u8(oc), opacity);
                (Some(oc_srgb), Some(ow))
            }
            _ => (None, None),
        };

        TextItem {
            text: text.to_owned(),
            pixel_x: x + margin_h,
            pixel_y: y + margin_v,
            bounds_width: (w - margin_h * 2.0).max(1.0),
            bounds_height: (h - margin_v * 2.0).max(1.0),
            font_size_px,
            font_family,
            font_weight,
            color,
            alignment,
            overflow: policy.overflow.unwrap_or(TextOverflow::Clip),
            outline_color,
            outline_width,
            opacity,
            color_runs: Box::default(),
        }
    }

    /// Build a `TextItem` for zone `StreamText` content.
    ///
    /// `x`, `y`, `w`, `h` are the zone geometry in physical pixels.
    ///
    /// # Deprecation note
    ///
    /// Prefer [`TextItem::from_zone_policy`] which reads all visual properties
    /// from `RenderingPolicy`.  This method retains explicit parameters for
    /// callers that do not yet have a policy (e.g. benchmarks).
    pub fn from_zone_stream_text(
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        font_size_px: f32,
        color: [u8; 4],
    ) -> Self {
        let margin = 8.0_f32;
        TextItem {
            text: text.to_owned(),
            pixel_x: x + margin,
            pixel_y: y + margin,
            bounds_width: (w - margin * 2.0).max(1.0),
            bounds_height: (h - margin * 2.0).max(1.0),
            font_size_px: font_size_px.clamp(6.0, 200.0),
            font_family: FontFamily::SystemSansSerif,
            font_weight: 400,
            color,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
            outline_color: None,
            outline_width: None,
            opacity: 1.0,
            color_runs: Box::default(),
        }
    }

    /// Build a `TextItem` for zone `ShortTextWithIcon` / `Notification` content.
    ///
    /// For v1, only the `text` field of [`NotificationPayload`] is rendered. Icon
    /// rendering is stubbed — there is no texture pipeline yet.
    ///
    /// `x`, `y`, `w`, `h` are the zone geometry in physical pixels.
    ///
    /// [`NotificationPayload`]: tze_hud_scene::types::NotificationPayload
    pub fn from_zone_notification(
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        font_size_px: f32,
        color: [u8; 4],
    ) -> Self {
        let margin = 8.0_f32;
        TextItem {
            text: text.to_owned(),
            pixel_x: x + margin,
            pixel_y: y + margin,
            bounds_width: (w - margin * 2.0).max(1.0),
            bounds_height: (h - margin * 2.0).max(1.0),
            font_size_px: font_size_px.clamp(6.0, 200.0),
            font_family: FontFamily::SystemSansSerif,
            font_weight: 400,
            color,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
            outline_color: None,
            outline_width: None,
            opacity: 1.0,
            color_runs: Box::default(),
        }
    }
}

// ─── Color helpers ────────────────────────────────────────────────────────────

/// Convert an `Rgba` (linear f32) to sRGB u8 `[r, g, b, a]`.
///
/// The alpha channel is passed through directly (0..1 → 0..255) rather than
/// being gamma-encoded, which matches glyphon's expected color space.
pub fn rgba_to_srgb_u8(c: Rgba) -> [u8; 4] {
    [
        linear_to_srgb_u8(c.r),
        linear_to_srgb_u8(c.g),
        linear_to_srgb_u8(c.b),
        (c.a * 255.0).clamp(0.0, 255.0) as u8,
    ]
}

/// Multiply the alpha channel of an sRGB u8 color by `opacity`.
pub fn apply_opacity_to_color(color: [u8; 4], opacity: f32) -> [u8; 4] {
    let a = (color[3] as f32 * opacity.clamp(0.0, 1.0)).clamp(0.0, 255.0) as u8;
    [color[0], color[1], color[2], a]
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Canonicalize a slice of [`ColorRunItem`]s into a sorted, non-overlapping
/// sequence using **last-writer-wins** overlap semantics.
///
/// # Semantics
///
/// For any byte position covered by multiple runs, the run with the **highest
/// original index** (i.e. the last run in the input slice whose range covers
/// that byte) wins.
///
/// # Algorithm
///
/// Uses a sweep-line over all interval endpoints:
///
/// 1. Clamp each run to `text_len`; drop degenerate (start ≥ end) runs.
/// 2. Emit two events per run — a START and an END — keyed by byte position.
///    At equal positions, END events sort before START events so adjacent
///    runs are handled correctly.
/// 3. Walk events left-to-right, maintaining a `BTreeMap<orig_idx, color>`
///    of currently-active runs.  Before each event, emit a span from the
///    active run with the **highest key** (= highest original index).
/// 4. Merge adjacent output spans with identical colors to minimize span count.
///
/// Out-of-bounds or zero-length runs are silently dropped.
fn canonicalize_color_runs_impl(runs: &[ColorRunItem], text_len: usize) -> Vec<ColorRunItem> {
    if runs.is_empty() {
        return Vec::new();
    }

    // (position, is_start, orig_idx, color)
    // is_start=false (END) sorts before is_start=true (START) at equal positions,
    // so a run ending exactly where another begins does not overlap.
    let mut events: Vec<(usize, bool, usize, [u8; 4])> = Vec::with_capacity(runs.len() * 2);
    for (i, r) in runs.iter().enumerate() {
        let s = r.start_byte.min(text_len);
        let e = r.end_byte.min(text_len);
        if s < e {
            events.push((s, true, i, r.color));
            events.push((e, false, i, r.color));
        }
    }

    // Sort: primary by position, secondary END < START (false < true).
    events.sort_by_key(|&(pos, is_start, _, _)| (pos, is_start));

    // active: orig_idx → color, for currently open runs.
    let mut active: BTreeMap<usize, [u8; 4]> = BTreeMap::new();
    // raw output segments before merging: (start, end, color).
    let mut raw: Vec<(usize, usize, [u8; 4])> = Vec::new();
    let mut last_pos = 0usize;

    for (pos, is_start, idx, color) in events {
        if pos > last_pos {
            // Emit segment for the active run with the highest original index,
            // which is the last-writer per the semantics contract.
            if let Some((&_, &active_color)) = active.iter().next_back() {
                raw.push((last_pos, pos, active_color));
            }
        }
        if is_start {
            active.insert(idx, color);
        } else {
            active.remove(&idx);
        }
        last_pos = pos;
    }

    // Merge adjacent segments with the same color to minimize span count.
    let mut out: Vec<ColorRunItem> = Vec::with_capacity(raw.len());
    for (s, e, c) in raw {
        if let Some(last) = out.last_mut() {
            if last.end_byte == s && last.color == c {
                last.end_byte = e;
                continue;
            }
        }
        out.push(ColorRunItem { start_byte: s, end_byte: e, color: c });
    }
    out
}

/// Build `(text_slice, Attrs)` pairs for [`Buffer::set_rich_text`] from a set
/// of [`ColorRunItem`]s and a base `Attrs`.
///
/// Runs need not be sorted or non-overlapping — this function canonicalizes
/// them using last-writer-wins semantics before building spans:
///
/// - Runs are sorted by `start_byte`.
/// - When two runs overlap, the **later run** (higher index in the original
///   slice) wins on the intersection.  The earlier run is split: its prefix
///   before the overlap is kept; the overlap itself is replaced by the later
///   run's color; any suffix of the earlier run beyond the later run's end
///   resumes with the earlier run's color.
///
/// Segments of `text` not covered by any run are emitted with `base_attrs`
/// (no color override, so `TextArea::default_color` applies at render time).
/// Segments covered by a run are emitted with `base_attrs` plus an explicit
/// `Color` set from the run.
///
/// Out-of-bounds run byte offsets are clamped to `text.len()`.
pub(crate) fn color_run_spans<'t, 'a>(
    text: &'t str,
    runs: &[ColorRunItem],
    base_attrs: Attrs<'a>,
) -> Vec<(&'t str, Attrs<'a>)> {
    let canonical = canonicalize_color_runs_impl(runs, text.len());

    let mut spans: Vec<(&'t str, Attrs<'a>)> = Vec::with_capacity(canonical.len() * 2 + 1);
    let mut cursor = 0usize;

    for run in &canonical {
        let run_start = run.start_byte;
        let run_end = run.end_byte;
        // run_start < run_end guaranteed by canonicalize_color_runs_impl.
        // Both are ≤ text.len() (clamped during canonicalization).

        // Emit unstyled gap before this run (if any).
        if cursor < run_start {
            spans.push((&text[cursor..run_start], base_attrs));
        }

        // Emit the colored run.
        let run_attrs = base_attrs.color(Color::rgba(
            run.color[0],
            run.color[1],
            run.color[2],
            run.color[3],
        ));
        spans.push((&text[run_start..run_end], run_attrs));

        cursor = run_end;
    }

    // Emit any trailing unstyled text after the last run.
    if cursor < text.len() {
        spans.push((&text[cursor..], base_attrs));
    }

    // If no spans were emitted (all runs were empty/invalid), fall back to
    // the full text with base_attrs so the buffer is never left empty.
    if spans.is_empty() {
        spans.push((text, base_attrs));
    }

    spans
}

/// Format a 32-byte resource ID as a lowercase hex string for logging.
pub(crate) fn format_resource_id(id: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in id {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Minimal Markdown strip for v1: removes `#` heading prefixes and `*` emphasis
/// markers. Does not parse nested Markdown, code blocks, or links.
pub fn strip_markdown_v1(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        let stripped = line.trim_start_matches('#').trim_start();
        let stripped = stripped.replace('*', "");
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&stripped);
    }
    out
}

/// Convert a linear light value [0..1] to an sRGB u8 [0..255].
///
/// Uses the piecewise IEC 61966-2-1 formula (same as wgpu's `Rgba8UnormSrgb`
/// conversion).
fn linear_to_srgb_u8(linear: f32) -> u8 {
    let clamped = linear.clamp(0.0, 1.0);
    let srgb = if clamped <= 0.003_130_8 {
        clamped * 12.92
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0 + 0.5) as u8
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_markdown_removes_heading_markers() {
        let md = "# Hello\n## World\nPlain text";
        let stripped = strip_markdown_v1(md);
        assert_eq!(stripped, "Hello\nWorld\nPlain text");
    }

    #[test]
    fn strip_markdown_removes_emphasis() {
        let md = "Hello *world*";
        let stripped = strip_markdown_v1(md);
        assert_eq!(stripped, "Hello world");
    }

    #[test]
    fn strip_markdown_empty_input() {
        assert_eq!(strip_markdown_v1(""), "");
    }

    #[test]
    fn linear_to_srgb_u8_black() {
        assert_eq!(linear_to_srgb_u8(0.0), 0);
    }

    #[test]
    fn linear_to_srgb_u8_white() {
        assert_eq!(linear_to_srgb_u8(1.0), 255);
    }

    #[test]
    fn linear_to_srgb_u8_midpoint() {
        // Linear 0.5 → sRGB ~0.735 → ~187 u8
        let v = linear_to_srgb_u8(0.5);
        assert!(v > 180 && v < 200, "midpoint sRGB: {v}");
    }

    #[test]
    fn text_item_from_zone_stream_text_insets_margin() {
        let item = TextItem::from_zone_stream_text(
            "hello",
            100.0,
            200.0,
            400.0,
            100.0,
            14.0,
            [255, 255, 255, 255],
        );
        // margin = 8px on each side
        assert_eq!(item.pixel_x, 108.0);
        assert_eq!(item.pixel_y, 208.0);
        assert_eq!(item.bounds_width, 384.0);
        assert_eq!(item.bounds_height, 84.0);
        // New fields default correctly.
        assert!(item.outline_color.is_none());
        assert!(item.outline_width.is_none());
        assert_eq!(item.opacity, 1.0);
    }

    #[test]
    fn text_item_from_zone_notification_insets_margin_and_uses_text() {
        let item = TextItem::from_zone_notification(
            "Alert: ready",
            50.0,
            10.0,
            300.0,
            60.0,
            18.0,
            [255, 255, 255, 220],
        );
        // margin = 8px on each side
        assert_eq!(item.pixel_x, 58.0);
        assert_eq!(item.pixel_y, 18.0);
        assert_eq!(item.bounds_width, 284.0);
        assert_eq!(item.bounds_height, 44.0);
        assert_eq!(item.text, "Alert: ready");
        assert_eq!(item.font_size_px, 18.0);
        assert_eq!(item.color, [255, 255, 255, 220]);
    }

    #[test]
    fn text_item_from_text_markdown_node_insets_margin() {
        use tze_hud_scene::types::{Rect, Rgba, TextMarkdownNode};
        let node = TextMarkdownNode {
            content: "# Hello\n*world*".to_owned(),
            bounds: Rect::new(10.0, 20.0, 200.0, 80.0),
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            color: Rgba::new(1.0, 1.0, 1.0, 1.0),
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
            color_runs: Box::default(),
        };
        let item = TextItem::from_text_markdown_node(&node, 50.0, 50.0);
        // tile_x=50, tile_y=50, node.bounds.x=10, node.bounds.y=20, margin=6
        assert_eq!(item.pixel_x, 66.0);
        assert_eq!(item.pixel_y, 76.0);
        assert_eq!(item.text, "Hello\nworld");
        // New fields default correctly.
        assert!(item.outline_color.is_none());
        assert!(item.outline_width.is_none());
        assert_eq!(item.opacity, 1.0);
    }

    #[test]
    fn text_item_from_small_text_markdown_node_does_not_collapse_height() {
        use tze_hud_scene::types::{Rect, Rgba, TextMarkdownNode};
        let node = TextMarkdownNode {
            content: "RESIDENT AGENT".to_owned(),
            bounds: Rect::new(96.0, 18.0, 152.0, 12.0),
            font_size_px: 11.0,
            font_family: FontFamily::SystemSansSerif,
            color: Rgba::new(1.0, 1.0, 1.0, 1.0),
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
            color_runs: Box::default(),
        };
        let item = TextItem::from_text_markdown_node(&node, 0.0, 0.0);
        assert!(
            item.bounds_height >= 11.0,
            "small presence-card labels must retain height close to their font size for live glyph rendering; got {}",
            item.bounds_height
        );
        assert!(
            item.pixel_y < 22.0,
            "small presence-card labels should not be pushed too far down by inset margin; got {}",
            item.pixel_y
        );
    }

    // ── from_zone_policy tests [hud-sc0a.8] ──────────────────────────────────

    #[test]
    fn from_zone_policy_reads_font_size_and_color() {
        use tze_hud_scene::types::{RenderingPolicy, Rgba};
        let policy = RenderingPolicy {
            font_size_px: Some(24.0),
            text_color: Some(Rgba::new(1.0, 0.0, 0.0, 1.0)), // red
            ..Default::default()
        };
        let item = TextItem::from_zone_policy("hello", 0.0, 0.0, 300.0, 80.0, &policy, 1.0);
        assert_eq!(item.font_size_px, 24.0);
        // Red channel (R=1.0 linear → 255 sRGB).
        assert_eq!(item.color[0], 255, "R should be max (red)");
        assert_eq!(item.color[1], 0, "G should be 0");
    }

    #[test]
    fn from_zone_policy_outline_fields_propagated() {
        use tze_hud_scene::types::{RenderingPolicy, Rgba};
        let policy = RenderingPolicy {
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(2.0),
            ..Default::default()
        };
        let item = TextItem::from_zone_policy("outlined", 0.0, 0.0, 300.0, 80.0, &policy, 1.0);
        assert!(item.outline_color.is_some(), "outline_color should be Some");
        assert_eq!(item.outline_width.unwrap(), 2.0);
    }

    #[test]
    fn from_zone_policy_outline_width_zero_suppresses_outline() {
        use tze_hud_scene::types::{RenderingPolicy, Rgba};
        let policy = RenderingPolicy {
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(0.0), // zero = no outline
            ..Default::default()
        };
        let item = TextItem::from_zone_policy("no_outline", 0.0, 0.0, 300.0, 80.0, &policy, 1.0);
        assert!(
            item.outline_color.is_none(),
            "outline_color should be None when outline_width=0"
        );
        assert!(item.outline_width.is_none());
    }

    #[test]
    fn from_zone_policy_opacity_applied_to_alpha() {
        use tze_hud_scene::types::{RenderingPolicy, Rgba};
        let policy = RenderingPolicy {
            text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
            ..Default::default()
        };
        // opacity=0.5 should halve the alpha.
        let item = TextItem::from_zone_policy("faded", 0.0, 0.0, 300.0, 80.0, &policy, 0.5);
        let alpha = item.color[3];
        // 255 * 0.5 = 127 (±2 for rounding).
        assert!(
            (alpha as i32 - 127).abs() <= 2,
            "opacity=0.5 should halve alpha (got {alpha})"
        );
    }

    #[test]
    fn from_zone_policy_default_margins_used_when_none() {
        use tze_hud_scene::types::RenderingPolicy;
        let policy = RenderingPolicy::default(); // margin_horizontal/vertical = None
        let item = TextItem::from_zone_policy("text", 100.0, 200.0, 400.0, 100.0, &policy, 1.0);
        // Default margin = 8px on each side.
        assert_eq!(item.pixel_x, 108.0, "default margin_h = 8.0");
        assert_eq!(item.pixel_y, 208.0, "default margin_v = 8.0");
    }

    #[test]
    fn rgba_to_srgb_u8_black_and_white() {
        use tze_hud_scene::types::Rgba;
        let black = rgba_to_srgb_u8(Rgba::BLACK);
        assert_eq!(black, [0, 0, 0, 255]);
        let white = rgba_to_srgb_u8(Rgba::WHITE);
        assert_eq!(white, [255, 255, 255, 255]);
    }

    #[test]
    fn from_zone_policy_margin_px_fallback() {
        // When margin_horizontal/vertical are None but margin_px is set,
        // margin_px should be used (spec §Extended RenderingPolicy).
        use tze_hud_scene::types::RenderingPolicy;
        let policy = RenderingPolicy {
            margin_px: Some(20.0),
            // margin_horizontal/margin_vertical intentionally None
            ..Default::default()
        };
        let item = TextItem::from_zone_policy("text", 0.0, 0.0, 400.0, 100.0, &policy, 1.0);
        assert_eq!(
            item.pixel_x, 20.0,
            "margin_px fallback should apply to horizontal margin"
        );
        assert_eq!(
            item.pixel_y, 20.0,
            "margin_px fallback should apply to vertical margin"
        );
    }

    #[test]
    fn apply_opacity_to_color_halves_alpha() {
        let color = [200u8, 100, 50, 200];
        let result = apply_opacity_to_color(color, 0.5);
        assert_eq!(result[0], 200, "RGB channels unchanged");
        assert_eq!(result[1], 100);
        assert_eq!(result[2], 50);
        // 200 * 0.5 = 100.
        assert_eq!(result[3], 100, "alpha halved");
    }

    // ── font_weight tests [hud-w3o6.2] ───────────────────────────────────────

    /// font_weight=700 is propagated from RenderingPolicy to TextItem.
    ///
    /// Acceptance criterion §Alert-Banner Heading Typography:
    ///   "typography.heading.weight (700/bold)"
    #[test]
    fn from_zone_policy_font_weight_bold_propagated() {
        use tze_hud_scene::types::RenderingPolicy;
        let policy = RenderingPolicy {
            font_weight: Some(700),
            ..Default::default()
        };
        let item = TextItem::from_zone_policy("bold text", 0.0, 0.0, 300.0, 60.0, &policy, 1.0);
        assert_eq!(
            item.font_weight, 700,
            "font_weight=700 (bold) must be propagated from RenderingPolicy to TextItem"
        );
    }

    /// font_weight defaults to 400 (regular) when not set in RenderingPolicy.
    #[test]
    fn from_zone_policy_font_weight_defaults_to_regular() {
        use tze_hud_scene::types::RenderingPolicy;
        let policy = RenderingPolicy::default(); // font_weight = None
        let item = TextItem::from_zone_policy("regular text", 0.0, 0.0, 300.0, 60.0, &policy, 1.0);
        assert_eq!(
            item.font_weight, 400,
            "font_weight must default to 400 (regular) when not set"
        );
    }

    /// from_text_markdown_node defaults font_weight to 400 (regular).
    #[test]
    fn from_text_markdown_node_font_weight_defaults_to_regular() {
        use tze_hud_scene::types::{Rect, Rgba, TextMarkdownNode};
        let node = TextMarkdownNode {
            content: "Plain text".to_owned(),
            bounds: Rect::new(0.0, 0.0, 200.0, 60.0),
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            color: Rgba::WHITE,
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
            color_runs: Box::default(),
        };
        let item = TextItem::from_text_markdown_node(&node, 0.0, 0.0);
        assert_eq!(
            item.font_weight, 400,
            "from_text_markdown_node must default font_weight to 400"
        );
    }

    /// from_zone_policy defaults overflow to Clip when policy.overflow is None.
    #[test]
    fn from_zone_policy_overflow_defaults_to_clip() {
        let policy = RenderingPolicy::default();
        let item = TextItem::from_zone_policy("text", 0.0, 0.0, 200.0, 40.0, &policy, 1.0);
        assert_eq!(
            item.overflow,
            TextOverflow::Clip,
            "overflow should default to Clip when policy.overflow is None"
        );
    }

    /// from_zone_policy propagates Ellipsis overflow from policy.
    #[test]
    fn from_zone_policy_overflow_ellipsis_propagated() {
        let mut policy = RenderingPolicy::default();
        policy.overflow = Some(TextOverflow::Ellipsis);
        let item = TextItem::from_zone_policy("text", 0.0, 0.0, 200.0, 40.0, &policy, 1.0);
        assert_eq!(
            item.overflow,
            TextOverflow::Ellipsis,
            "overflow should be Ellipsis when policy.overflow is Some(Ellipsis)"
        );
    }

    /// from_zone_policy propagates explicit Clip overflow from policy.
    #[test]
    fn from_zone_policy_overflow_clip_explicit_propagated() {
        let mut policy = RenderingPolicy::default();
        policy.overflow = Some(TextOverflow::Clip);
        let item = TextItem::from_zone_policy("text", 0.0, 0.0, 200.0, 40.0, &policy, 1.0);
        assert_eq!(
            item.overflow,
            TextOverflow::Clip,
            "overflow should be Clip when policy.overflow is Some(Clip)"
        );
    }

    // ── color_run_spans single-pass tests [hud-9pmd] ─────────────────────────

    /// Single colored run covering the entire text produces one span with color.
    #[test]
    fn color_run_spans_single_run_full_text() {
        let text = "hello";
        let runs = [ColorRunItem {
            start_byte: 0,
            end_byte: 5,
            color: [255, 0, 0, 255],
        }];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);
        assert_eq!(spans.len(), 1, "one run covering full text → one span");
        assert_eq!(spans[0].0, "hello");
        // The span must carry an explicit color (not base_attrs which has no color_opt).
        assert!(
            spans[0].1.color_opt.is_some(),
            "run span must have a color_opt set"
        );
    }

    /// Two runs with a gap in between produce three spans: gap, run, run.
    #[test]
    fn color_run_spans_two_runs_with_gap() {
        let text = "hello world";
        // "hello" → red, " " → unstyled gap, "world" → blue
        let runs = [
            ColorRunItem {
                start_byte: 0,
                end_byte: 5,
                color: [255, 0, 0, 255],
            },
            ColorRunItem {
                start_byte: 6,
                end_byte: 11,
                color: [0, 0, 255, 255],
            },
        ];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);
        // Expect: [("hello", red), (" ", base), ("world", blue)]
        assert_eq!(spans.len(), 3, "two runs with gap → three spans");
        assert_eq!(spans[0].0, "hello");
        assert!(spans[0].1.color_opt.is_some(), "first run must have color");
        assert_eq!(spans[1].0, " ", "gap between runs must be unstyled");
        assert!(spans[1].1.color_opt.is_none(), "gap span must use base_attrs (no color_opt)");
        assert_eq!(spans[2].0, "world");
        assert!(spans[2].1.color_opt.is_some(), "second run must have color");
    }

    /// A run followed by trailing unstyled text produces run + trailing span.
    #[test]
    fn color_run_spans_trailing_unstyled() {
        let text = "hello world";
        let runs = [ColorRunItem {
            start_byte: 0,
            end_byte: 5,
            color: [255, 0, 0, 255],
        }];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);
        assert_eq!(spans.len(), 2, "run + trailing unstyled → two spans");
        assert_eq!(spans[0].0, "hello");
        assert_eq!(spans[1].0, " world");
        assert!(spans[1].1.color_opt.is_none(), "trailing span must use base_attrs");
    }

    /// Empty runs slice falls back to a single full-text base-attrs span.
    #[test]
    fn color_run_spans_empty_runs_fallback() {
        let text = "no color";
        let spans = color_run_spans(text, &[], Attrs::new());
        assert_eq!(spans.len(), 1, "no runs → single fallback span");
        assert_eq!(spans[0].0, "no color");
        assert!(spans[0].1.color_opt.is_none(), "fallback span must use base_attrs");
    }

    /// Out-of-bounds run end is clamped; no panic.
    #[test]
    fn color_run_spans_clamped_out_of_bounds_run() {
        let text = "hi";
        let runs = [ColorRunItem {
            start_byte: 0,
            end_byte: 999, // way beyond text length
            color: [0, 255, 0, 255],
        }];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);
        // Should produce one span covering the full text with color.
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].0, "hi");
        assert!(spans[0].1.color_opt.is_some());
    }

    /// Degenerate run (start >= end) is skipped; remaining text is returned unstyled.
    #[test]
    fn color_run_spans_degenerate_run_skipped() {
        let text = "text";
        let runs = [ColorRunItem {
            start_byte: 2,
            end_byte: 2, // zero-length: should be skipped
            color: [255, 0, 0, 255],
        }];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);
        // Zero-length run → no gap before it (cursor=0, run_start=2), and run is skipped.
        // cursor stays at 0, so trailing text "text" is emitted as unstyled.
        // But the fallback at the end also covers this — spans should be non-empty.
        assert!(!spans.is_empty(), "degenerate run must not produce empty spans");
        let combined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(combined, "text", "all text must be covered even with degenerate run");
    }

    /// color_run_spans produces correct slices for multi-byte UTF-8 characters.
    #[test]
    fn color_run_spans_multibyte_utf8() {
        // "é" is 2 bytes (0xC3 0xA9), "world" is 5 bytes → total 7 bytes
        let text = "éworld";
        let runs = [ColorRunItem {
            start_byte: 0,
            end_byte: 2, // "é"
            color: [255, 0, 0, 255],
        }];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].0, "é");
        assert_eq!(spans[1].0, "world");
    }

    // ── Overlap / out-of-order tests [hud-qu8k4] ─────────────────────────────

    /// Two overlapping runs: later run wins on the intersection.
    ///
    /// Input: red 0..10, blue 5..15 on "0123456789abcde" (15 bytes).
    /// Expected: red 0..5, blue 5..15.
    #[test]
    fn color_run_spans_two_runs_overlap_later_wins() {
        let text = "0123456789abcde"; // 15 ASCII bytes
        let red = [255u8, 0, 0, 255];
        let blue = [0u8, 0, 255, 255];
        let runs = [
            ColorRunItem { start_byte: 0, end_byte: 10, color: red },
            ColorRunItem { start_byte: 5, end_byte: 15, color: blue },
        ];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);

        // Reconstruct text coverage.
        let combined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(combined, text, "all text must be covered");

        // First span: "01234" (bytes 0..5) — red.
        assert_eq!(spans[0].0, &text[0..5], "red prefix should be 0..5");
        assert!(spans[0].1.color_opt.is_some(), "first span must be colored");
        let red_color = spans[0].1.color_opt.unwrap();
        assert_eq!((red_color.r(), red_color.g(), red_color.b()), (255, 0, 0),
            "first span should be red");

        // Second span: "56789abcde" (bytes 5..15) — blue (later run wins intersection).
        assert_eq!(spans[1].0, &text[5..15], "blue span should cover 5..15");
        assert!(spans[1].1.color_opt.is_some(), "second span must be colored");
        let blue_color = spans[1].1.color_opt.unwrap();
        assert_eq!((blue_color.r(), blue_color.g(), blue_color.b()), (0, 0, 255),
            "second span should be blue (later-writer-wins)");

        assert_eq!(spans.len(), 2, "no trailing unstyled text expected");
    }

    /// Three runs: middle run overlaps both outer runs; middle (later) wins intersection.
    ///
    /// Input: red 0..8, green 4..12, blue 10..15 on 15-byte ASCII.
    /// Expected: red 0..4, green 4..12, blue 12..15.
    #[test]
    fn color_run_spans_three_runs_middle_overlaps_outer() {
        let text = "0123456789abcde"; // 15 ASCII bytes
        let red = [255u8, 0, 0, 255];
        let green = [0u8, 255, 0, 255];
        let blue = [0u8, 0, 255, 255];
        // red: 0..8, green: 4..12 (overlaps red and blue), blue: 10..15
        let runs = [
            ColorRunItem { start_byte: 0, end_byte: 8, color: red },
            ColorRunItem { start_byte: 4, end_byte: 12, color: green },
            ColorRunItem { start_byte: 10, end_byte: 15, color: blue },
        ];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);

        let combined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(combined, text, "all 15 bytes must be covered");

        // Verify the color at the start of each expected segment.
        // Expected layout: red[0..4], green[4..12], blue[12..15].
        // (green wins 4..12; blue wins 10..15 — within green's range 10..12 is disputed;
        //  blue is the latest run touching 10..12, so blue wins.)
        // Actually: blue(10..15) is later than green(4..12), so blue wins 10..12 too.
        // Expected: red[0..4], green[4..10], blue[10..15].
        let segment_texts: Vec<&str> = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(segment_texts[0], &text[0..4], "segment 0 = red[0..4]");
        assert_eq!(segment_texts[1], &text[4..10], "segment 1 = green[4..10]");
        assert_eq!(segment_texts[2], &text[10..15], "segment 2 = blue[10..15]");

        let c0 = spans[0].1.color_opt.unwrap();
        assert_eq!((c0.r(), c0.g(), c0.b()), (255, 0, 0), "seg0 = red");
        let c1 = spans[1].1.color_opt.unwrap();
        assert_eq!((c1.r(), c1.g(), c1.b()), (0, 255, 0), "seg1 = green");
        let c2 = spans[2].1.color_opt.unwrap();
        assert_eq!((c2.r(), c2.g(), c2.b()), (0, 0, 255), "seg2 = blue");
    }

    /// Unsorted input (run B before run A by start_byte) produces the same
    /// output as the equivalent sorted input.
    #[test]
    fn color_run_spans_unsorted_input_same_as_sorted() {
        let text = "hello world"; // 11 bytes
        let red = [255u8, 0, 0, 255];
        let blue = [0u8, 0, 255, 255];

        // Sorted order: red 0..5, blue 6..11.
        let sorted_runs = [
            ColorRunItem { start_byte: 0, end_byte: 5, color: red },
            ColorRunItem { start_byte: 6, end_byte: 11, color: blue },
        ];
        // Reversed order: same runs but blue listed first.
        let unsorted_runs = [
            ColorRunItem { start_byte: 6, end_byte: 11, color: blue },
            ColorRunItem { start_byte: 0, end_byte: 5, color: red },
        ];

        let base = Attrs::new();
        let sorted_spans = color_run_spans(text, &sorted_runs, base);
        let unsorted_spans = color_run_spans(text, &unsorted_runs, base);

        // Text coverage must be identical.
        let sorted_text: String = sorted_spans.iter().map(|(s, _)| *s).collect();
        let unsorted_text: String = unsorted_spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(sorted_text, unsorted_text, "text coverage must match");

        // Span count must match.
        assert_eq!(sorted_spans.len(), unsorted_spans.len(), "span count must match");

        // Each span's slice and color must match.
        for i in 0..sorted_spans.len() {
            assert_eq!(sorted_spans[i].0, unsorted_spans[i].0,
                "span[{i}] text slice must match");
            assert_eq!(sorted_spans[i].1.color_opt, unsorted_spans[i].1.color_opt,
                "span[{i}] color must match");
        }
    }

    /// Adjacent (touching) non-overlapping runs are preserved as separate spans.
    #[test]
    fn color_run_spans_adjacent_non_overlapping() {
        let text = "abcdef"; // 6 bytes
        let red = [255u8, 0, 0, 255];
        let blue = [0u8, 0, 255, 255];
        // run A: 0..3 ("abc"), run B: 3..6 ("def") — adjacent, no overlap.
        let runs = [
            ColorRunItem { start_byte: 0, end_byte: 3, color: red },
            ColorRunItem { start_byte: 3, end_byte: 6, color: blue },
        ];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);

        let combined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(combined, text, "full text must be covered");
        assert_eq!(spans.len(), 2, "two adjacent runs → two spans (no gap)");
        assert_eq!(spans[0].0, "abc");
        assert_eq!(spans[1].0, "def");

        let c0 = spans[0].1.color_opt.unwrap();
        assert_eq!((c0.r(), c0.g(), c0.b()), (255, 0, 0), "first span = red");
        let c1 = spans[1].1.color_opt.unwrap();
        assert_eq!((c1.r(), c1.g(), c1.b()), (0, 0, 255), "second span = blue");
    }

    /// Fully nested run: inner run (later) wins; outer run retains prefix and
    /// suffix around the inner region.
    ///
    /// Input: red 0..12, blue 4..8 on "0123456789ab" (12 bytes).
    /// Expected: red 0..4, blue 4..8, red 8..12.
    #[test]
    fn color_run_spans_nested_inner_wins() {
        let text = "0123456789ab"; // 12 ASCII bytes
        let red = [255u8, 0, 0, 255];
        let blue = [0u8, 0, 255, 255];
        let runs = [
            ColorRunItem { start_byte: 0, end_byte: 12, color: red },
            ColorRunItem { start_byte: 4, end_byte: 8, color: blue }, // nested inside red
        ];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);

        let combined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(combined, text, "all 12 bytes must be covered");

        assert_eq!(spans.len(), 3, "nested inner run → prefix + inner + suffix");
        assert_eq!(spans[0].0, &text[0..4]);
        assert_eq!(spans[1].0, &text[4..8]);
        assert_eq!(spans[2].0, &text[8..12]);

        let c0 = spans[0].1.color_opt.unwrap();
        assert_eq!((c0.r(), c0.g(), c0.b()), (255, 0, 0), "prefix = red");
        let c1 = spans[1].1.color_opt.unwrap();
        assert_eq!((c1.r(), c1.g(), c1.b()), (0, 0, 255), "inner = blue (later wins)");
        let c2 = spans[2].1.color_opt.unwrap();
        assert_eq!((c2.r(), c2.g(), c2.b()), (255, 0, 0), "suffix = red (resumed)");
    }

    /// Higher original-index run wins even when it starts earlier than the
    /// lower-index run.
    ///
    /// Input: blue(index 0) = 5..15, red(index 1) = 0..10 on 15-byte ASCII.
    /// Red has the higher original index so it wins bytes 5..10.
    /// Expected: red 0..10, blue 10..15.
    #[test]
    fn color_run_spans_higher_index_earlier_start_wins() {
        let text = "0123456789abcde"; // 15 ASCII bytes
        let red = [255u8, 0, 0, 255];
        let blue = [0u8, 0, 255, 255];
        // blue is index 0, red is index 1 — red (higher index) must win the overlap.
        let runs = [
            ColorRunItem { start_byte: 5, end_byte: 15, color: blue },
            ColorRunItem { start_byte: 0, end_byte: 10, color: red },
        ];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);

        let combined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(combined, text, "all 15 bytes must be covered");

        // Expected: red[0..10], blue[10..15].
        assert_eq!(spans.len(), 2, "higher-index-wins overlap → two spans");
        assert_eq!(spans[0].0, &text[0..10], "span 0 = red 0..10");
        assert_eq!(spans[1].0, &text[10..15], "span 1 = blue 10..15");

        let c0 = spans[0].1.color_opt.unwrap();
        assert_eq!((c0.r(), c0.g(), c0.b()), (255, 0, 0), "red (higher index) wins 0..10");
        let c1 = spans[1].1.color_opt.unwrap();
        assert_eq!((c1.r(), c1.g(), c1.b()), (0, 0, 255), "blue covers unclaimed 10..15");
    }

    /// Zero-length and out-of-bounds runs are silently dropped.
    #[test]
    fn color_run_spans_degenerate_runs_dropped() {
        let text = "hello"; // 5 bytes
        let red = [255u8, 0, 0, 255];
        let runs = [
            // zero-length: start == end
            ColorRunItem { start_byte: 2, end_byte: 2, color: red },
            // out-of-bounds: extends past text.len()
            ColorRunItem { start_byte: 3, end_byte: 100, color: red },
            // valid run
            ColorRunItem { start_byte: 0, end_byte: 3, color: red },
        ];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);

        let combined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(combined, text, "all text must be covered");

        // Degenerate run at 2..2 is dropped.
        // Out-of-bounds 3..100 is clamped to 3..5 and still applies.
        // Valid 0..3 covers "hel". Clamped 3..5 covers "lo".
        // Both are red, and they're adjacent — so they should merge into one span.
        assert_eq!(spans.len(), 1, "adjacent same-color spans should merge");
        let c0 = spans[0].1.color_opt.unwrap();
        assert_eq!((c0.r(), c0.g(), c0.b()), (255, 0, 0));
    }
}
