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

use std::collections::HashSet;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Wrap,
};
use tze_hud_scene::types::{FontFamily, RenderingPolicy, Rgba, TextAlign, TextMarkdownNode, TextOverflow};
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
                let attrs = Attrs::new().family(family);
                buf.set_text(&mut self.font_system, &item.text, attrs, Shaping::Basic);
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
}

impl TextItem {
    /// Build a `TextItem` from a `TextMarkdownNode` and its tile-relative position.
    ///
    /// `tile_x` / `tile_y` are the pixel-space position of the tile origin.
    pub fn from_text_markdown_node(node: &TextMarkdownNode, tile_x: f32, tile_y: f32) -> Self {
        // Strip minimal Markdown for v1: remove `#` heading markers, `*` emphasis markers.
        let stripped = strip_markdown_v1(&node.content);

        // Convert linear f32 color [0..1] to sRGB u8 [0..255].
        let r = linear_to_srgb_u8(node.color.r);
        let g = linear_to_srgb_u8(node.color.g);
        let b = linear_to_srgb_u8(node.color.b);
        let a = (node.color.a * 255.0).clamp(0.0, 255.0) as u8;

        // Add a small inset margin so text doesn't touch the tile edge.
        let margin = 6.0_f32;
        let x = tile_x + node.bounds.x + margin;
        let y = tile_y + node.bounds.y + margin;
        let w = (node.bounds.width - margin * 2.0).max(1.0);
        let h = (node.bounds.height - margin * 2.0).max(1.0);

        TextItem {
            text: stripped,
            pixel_x: x,
            pixel_y: y,
            bounds_width: w,
            bounds_height: h,
            font_size_px: node.font_size_px.clamp(6.0, 200.0),
            font_family: node.font_family,
            color: [r, g, b, a],
            alignment: node.alignment,
            overflow: node.overflow,
            outline_color: None,
            outline_width: None,
            opacity: 1.0,
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
        let margin_h = policy
            .margin_horizontal
            .or(policy.margin_px)
            .unwrap_or(8.0);
        let margin_v = policy
            .margin_vertical
            .or(policy.margin_px)
            .unwrap_or(8.0);

        let font_size_px = policy.font_size_px.unwrap_or(16.0).clamp(6.0, 200.0);
        let font_family = policy.font_family.unwrap_or(FontFamily::SystemSansSerif);
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
            color,
            alignment,
            overflow: TextOverflow::Clip,
            outline_color,
            outline_width,
            opacity,
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
            color,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
            outline_color: None,
            outline_width: None,
            opacity: 1.0,
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
            color,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
            outline_color: None,
            outline_width: None,
            opacity: 1.0,
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
        assert_eq!(item.pixel_x, 20.0, "margin_px fallback should apply to horizontal margin");
        assert_eq!(item.pixel_y, 20.0, "margin_px fallback should apply to vertical margin");
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
}
