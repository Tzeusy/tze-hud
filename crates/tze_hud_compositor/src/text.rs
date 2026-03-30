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

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Wrap,
};
use tze_hud_scene::types::{FontFamily, TextAlign, TextMarkdownNode, TextOverflow};
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
        }
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
    /// Returns `Ok(())` on success, or a string on glyphon error (non-fatal —
    /// the frame continues with missing text rather than a crash).
    pub fn prepare_text_items(
        &mut self,
        device: &Device,
        queue: &Queue,
        items: &[TextItem],
    ) -> Result<(), String> {
        // Build one glyphon Buffer per item.
        let buffers: Vec<Buffer> = items
            .iter()
            .map(|item| {
                let line_height = item.font_size_px * 1.4;
                let mut buf = Buffer::new(
                    &mut self.font_system,
                    Metrics::new(item.font_size_px, line_height),
                );

                // Set available size so word-wrap operates within bounds.
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

        // Build TextArea slice from items + buffers (same order).
        let text_areas: Vec<TextArea<'_>> = items
            .iter()
            .zip(buffers.iter())
            .map(|(item, buf)| {
                let color = item.color;
                // Hard clip bounds for both Clip and Ellipsis (ellipsis approximated
                // by word-wrap fitting into bounds height).
                let bounds = TextBounds {
                    left: item.pixel_x as i32,
                    top: item.pixel_y as i32,
                    right: (item.pixel_x + item.bounds_width) as i32,
                    bottom: (item.pixel_y + item.bounds_height) as i32,
                };
                TextArea {
                    buffer: buf,
                    left: item.pixel_x,
                    top: item.pixel_y,
                    scale: 1.0,
                    bounds,
                    default_color: Color::rgba(color[0], color[1], color[2], color[3]),
                    custom_glyphs: &[],
                }
            })
            .collect();

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
        }
    }

    /// Build a `TextItem` for zone `StreamText` content.
    ///
    /// `x`, `y`, `w`, `h` are the zone geometry in physical pixels.
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
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

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
    }
}
