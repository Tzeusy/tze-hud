//! Vertical-flow layout resolution for stacked child nodes (hud-txkbh).
//!
//! The compositor plots every node at its own explicit `bounds.y`; there is no
//! flow/stack layout. This module supplies the general capability to STACK a
//! parent's children vertically: measure each child's wrapped rendered height and
//! position the next child directly below it plus a gap. It exists so a publisher
//! that cannot measure wrapped text — notably the projection layer, which would
//! emit one transcript node per conversational turn but has no font metrics — can
//! leave child vertical positions unresolved and have the *runtime* compute the
//! stack. Layout runs in the runtime/compositor, never in the model
//! (LLM-out-of-frame-loop), and the inter-child gap is supplied by the caller
//! from a design token (e.g. `PortalPartTokens::section_gap_px`) — this module
//! never invents spacing.
//!
//! It is split into a pure geometry core ([`stack_offsets`] / [`flow_total_height`],
//! no fonts, unit-testable in isolation) and a measurement bridge
//! ([`measure_child_height`] / [`resolve_vertical_flow`]) that reuses the
//! compositor's existing shaping so a measured child height agrees with what
//! the render path paints for the same content and width.
//!
//! [`FlowChild::markdown_tokens`] selects which render path a child is measured
//! against, and the "measured == painted" claim above is scoped accordingly
//! (hud-3xdlf):
//!
//! - `None` — plain-text content (e.g. the composer draft). Measured via
//!   [`composer_wrap_line_widths`], the SAME CPU wrapped-line shaper the
//!   composer's own measurement/render paths already share (`WRAPPED_TEXT_WRAP`,
//!   uniform `font_size_px` × [`LINE_HEIGHT_MULTIPLIER`]) — correct for plain
//!   text, but this path does NOT strip markdown syntax or apply per-span
//!   styling, so it must never be used to measure `TextMarkdownNode` content.
//! - `Some(tokens)` — markdown content (e.g. a transcript turn). Measured via
//!   [`crate::text::measure_markdown_content_height`], which reproduces
//!   [`crate::text::TextItem::from_text_markdown_cached`]'s shaping: `content`
//!   is parsed (markdown syntax stripped to `plain_text`, matching what
//!   actually paints), heading `size_scale` spans shape at their real
//!   (taller) per-span metrics, and the token-resolved
//!   `parsed.line_height_multiplier` is used instead of the plain-text
//!   constant. This is the path the intended per-turn transcript split
//!   (hud-uym23) will use once wired.
//!
//! This is the reusable resolution engine only. Wiring the resolved offsets into
//! the render sites and exposing a per-node layout mode on the scene/wire schema
//! are separate integration steps; the per-turn transcript split that would drive
//! this in production is gated on the Phase-1 Promotion Evidence Gate
//! (text-stream-portals §Phase-1 Promotion Evidence Gate).

use std::collections::HashMap;

use glyphon::FontSystem;
use tze_hud_scene::types::{FontFamily, Node, NodeData, NodeLayout, SceneId};

use crate::markdown::MarkdownTokens;
use crate::text::{
    LINE_HEIGHT_MULTIPLIER, composer_wrap_line_widths, measure_markdown_content_height,
    text_content_box_margins,
};

/// One child's inputs for vertical-flow height measurement.
#[derive(Clone, Copy, Debug)]
pub struct FlowChild<'a> {
    /// The child's text content (markdown source, as the render path receives it).
    pub content: &'a str,
    /// The width the child wraps to — its own content box width in px.
    pub wrap_width: f32,
    /// Font size in px used to shape and measure the child.
    pub font_size_px: f32,
    /// Font family used to shape and measure the child. Only consulted on the
    /// markdown path ([`Self::markdown_tokens`] is `Some`) — the plain-text
    /// path shapes at a fixed sans-serif family, matching
    /// [`composer_wrap_line_widths`]'s existing behavior.
    pub font_family: FontFamily,
    /// Total vertical padding (top + bottom) added around the text inside the
    /// child node, matching the render path's `text_margin * 2` so the measured
    /// row height equals the painted row height.
    pub vertical_padding: f32,
    /// `None` measures `content` as plain text (the composer/draft path).
    /// `Some(tokens)` measures `content` as markdown SOURCE, reproducing the
    /// render path's parse + per-span shaping (hud-3xdlf) — see the module
    /// docs for exactly what each path does and does not account for.
    pub markdown_tokens: Option<&'a MarkdownTokens>,
}

/// A resolved vertical-flow layout: the top y-offset of each child (in the same
/// coordinate space as the `start_y` passed to [`resolve_vertical_flow`]) plus the
/// total stacked height of the flow.
#[derive(Clone, Debug, PartialEq)]
pub struct VerticalFlowLayout {
    /// One entry per input child, in order: the child's resolved top y.
    pub offsets: Vec<f32>,
    /// Total height spanned by the stack (0.0 for an empty flow).
    pub total_height: f32,
}

/// Pure geometry: stack `heights` top-to-bottom from `start_y`, inserting `gap`
/// between adjacent items (never before the first or after the last). Returns the
/// resolved top y of each item, in order.
///
/// Defensive clamps keep a malformed input from making the stack run backwards: a
/// negative `gap` is treated as `0.0`, and a negative height contributes `0.0` to
/// the running cursor (its own offset is still emitted). The result length always
/// equals `heights.len()`.
pub fn stack_offsets(heights: &[f32], gap: f32, start_y: f32) -> Vec<f32> {
    let gap = gap.max(0.0);
    let mut offsets = Vec::with_capacity(heights.len());
    let mut cursor = start_y;
    for (index, height) in heights.iter().enumerate() {
        if index > 0 {
            cursor += gap;
        }
        offsets.push(cursor);
        cursor += height.max(0.0);
    }
    offsets
}

/// Pure geometry: the total height a stack of `heights` occupies with `gap`
/// between adjacent items. Returns `0.0` for an empty stack; a single item has no
/// gap. Negative heights and `gap` are clamped to `0.0`, matching
/// [`stack_offsets`].
pub fn flow_total_height(heights: &[f32], gap: f32) -> f32 {
    if heights.is_empty() {
        return 0.0;
    }
    let gap = gap.max(0.0);
    let sum: f32 = heights.iter().map(|h| h.max(0.0)).sum();
    sum + gap * (heights.len() as f32 - 1.0)
}

/// Measure the rendered height (px) of one flowed child's wrapped content at its
/// `wrap_width`, dispatching on [`FlowChild::markdown_tokens`] to the render
/// path's OWN shaping for that content kind — see the module docs for exactly
/// what "measured == painted" covers on each branch (hud-3xdlf).
///
/// - `markdown_tokens: None` — plain text, via [`composer_wrap_line_widths`]:
///   height = `wrapped_line_count * (font_size_px * LINE_HEIGHT_MULTIPLIER) +
///   vertical_padding`, with a floor of one line so an empty turn still
///   occupies a row rather than collapsing to zero.
/// - `markdown_tokens: Some(tokens)` — markdown source, via
///   [`crate::text::measure_markdown_content_height`]: `content` is parsed and
///   shaped exactly as [`crate::text::TextItem::from_text_markdown_cached`]
///   shapes it (stripped `plain_text`, per-span heading `size_scale`,
///   token-resolved line-height multiplier), and height is read back from
///   glyphon's actual per-line layout rather than a `line_count * constant`
///   product — the product formula assumes uniform line heights, which a
///   heading-containing turn violates.
///
/// `wrap_width` and `vertical_padding` are clamped to non-negative on both
/// branches.
pub fn measure_child_height(font_system: &mut FontSystem, child: &FlowChild<'_>) -> f32 {
    let wrap_width = child.wrap_width.max(1.0);
    let content_height = match child.markdown_tokens {
        Some(tokens) => measure_markdown_content_height(
            font_system,
            child.content,
            wrap_width,
            child.font_size_px,
            child.font_family,
            tokens,
        ),
        None => {
            let line_count = composer_wrap_line_widths(
                font_system,
                child.content,
                wrap_width,
                child.font_size_px,
                LINE_HEIGHT_MULTIPLIER,
            )
            .len()
            .max(1);
            let line_height = child.font_size_px * LINE_HEIGHT_MULTIPLIER;
            line_count as f32 * line_height
        }
    };
    content_height + child.vertical_padding.max(0.0)
}

/// Resolve a full vertical-flow layout for `children`: measure each child's height
/// (via [`measure_child_height`]) and stack them from `start_y` with `gap` between
/// rows (via [`stack_offsets`]).
///
/// `gap` MUST be supplied by the caller from a design token (never hardcoded here)
/// so a profile/token change reskins the flow spacing without touching this code.
pub fn resolve_vertical_flow(
    font_system: &mut FontSystem,
    children: &[FlowChild<'_>],
    gap: f32,
    start_y: f32,
) -> VerticalFlowLayout {
    let heights: Vec<f32> = children
        .iter()
        .map(|child| measure_child_height(font_system, child))
        .collect();
    let offsets = stack_offsets(&heights, gap, start_y);
    let total_height = flow_total_height(&heights, gap);
    VerticalFlowLayout {
        offsets,
        total_height,
    }
}

/// The height a single flowed child occupies in a vertical stack: measured
/// wrapped text height for `TextMarkdown` children, or the child's explicit
/// `bounds.height` for non-text children (solid color, image, hit region), which
/// carry their own fixed geometry rather than flowing text.
///
/// Text children are measured through the SAME content-box translation the render
/// path applies ([`text_content_box_margins`]): the wrap width is the content box
/// `bounds.width - 2*margin_x` (NOT the raw bounds width) and the vertical padding
/// is `2*margin_y`, so the measured row height equals the painted row height
/// (hud-yfj8u fidelity — the render path and this measurement share one margin
/// formula and cannot drift).
fn flow_child_height(font_system: &mut FontSystem, node: &Node) -> f32 {
    match &node.data {
        NodeData::TextMarkdown(tm) => {
            let line_height = tm.font_size_px * LINE_HEIGHT_MULTIPLIER;
            let (margin_x, margin_y) =
                text_content_box_margins(tm.bounds.width, tm.bounds.height, line_height);
            measure_child_height(
                font_system,
                &FlowChild {
                    content: &tm.content,
                    wrap_width: (tm.bounds.width - margin_x * 2.0).max(1.0),
                    font_size_px: tm.font_size_px,
                    font_family: tm.font_family,
                    vertical_padding: margin_y * 2.0,
                    // Plain-text measurement for now (hud-yfj8u): the pre-pass has
                    // no production caller and threads no `MarkdownTokens`.
                    // Transcript turns are markdown, so the render-wiring follow-up
                    // (hud-pd9bp) SHOULD thread the tile's `MarkdownTokens` here and
                    // pass `Some(..)` so stacked turn heights match the render
                    // path's parse+shape (hud-3xdlf); until then this matches the
                    // plain-text heights the resolver was unit-tested against.
                    markdown_tokens: None,
                },
            )
        }
        NodeData::SolidColor(n) => n.bounds.height.max(0.0),
        NodeData::StaticImage(n) => n.bounds.height.max(0.0),
        NodeData::HitRegion(n) => n.bounds.height.max(0.0),
    }
}

/// Resolve tile-local vertical-flow y-offsets for the direct children of every
/// [`NodeLayout::VerticalFlow`] node in `nodes`, keyed by child [`SceneId`].
///
/// For each flow parent, its children (in `children` order) are measured
/// ([`flow_child_height`]) and stacked from the PARENT's own `bounds.y` with
/// `gap` between rows ([`stack_offsets`]); each child's resolved top y is written
/// to the returned map. A child id absent from `nodes` (a dangling ref) is
/// skipped rather than aborting the walk.
///
/// **Behavior-preserving.** An `Absolute` parent contributes nothing, so a scene
/// with no `VerticalFlow` node yields an EMPTY map — the render path then falls
/// back to each child's own `bounds.y` and rendering is byte-identical to before
/// this capability existed. `gap` (design token) is supplied by the caller; each
/// text child's wrap width and vertical padding are derived from its own bounds
/// via the render path's shared content-box margin formula, so nothing is
/// invented here.
///
/// This is the compositor-side pre-pass. Substituting the resolved y at the
/// render geometry sites (and GPU pixel verification of the stacked result) is
/// the remaining integration step, deferred to a live-hardware evidence bead per
/// the hud-yfj8u live-verify scope.
pub fn resolve_tile_flow_offsets(
    font_system: &mut FontSystem,
    nodes: &HashMap<SceneId, Node>,
    gap: f32,
) -> HashMap<SceneId, f32> {
    let mut offsets = HashMap::new();
    for parent in nodes.values() {
        if parent.layout != NodeLayout::VerticalFlow {
            continue;
        }
        let children: Vec<&Node> = parent
            .children
            .iter()
            .filter_map(|id| nodes.get(id))
            .collect();
        let heights: Vec<f32> = children
            .iter()
            .map(|child| flow_child_height(font_system, child))
            .collect();
        let start_y = parent.data.bounds().y;
        for (child, y) in children.iter().zip(stack_offsets(&heights, gap, start_y)) {
            offsets.insert(child.id, y);
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::types::FontFamily;
    use tze_hud_scene::types::{Rect, Rgba, TextAlign, TextMarkdownNode, TextOverflow};

    // ── Pure geometry core (no fonts) ────────────────────────────────────────

    #[test]
    fn stack_offsets_empty_is_empty() {
        assert!(stack_offsets(&[], 8.0, 0.0).is_empty());
    }

    #[test]
    fn stack_offsets_single_has_no_gap() {
        assert_eq!(stack_offsets(&[20.0], 8.0, 5.0), vec![5.0]);
    }

    #[test]
    fn stack_offsets_stacks_with_gap() {
        // start 0, heights [10, 20, 5], gap 4:
        //  child0 @ 0
        //  child1 @ 0 + 10 + 4 = 14
        //  child2 @ 14 + 20 + 4 = 38
        assert_eq!(
            stack_offsets(&[10.0, 20.0, 5.0], 4.0, 0.0),
            vec![0.0, 14.0, 38.0]
        );
    }

    #[test]
    fn stack_offsets_honors_start_y() {
        assert_eq!(stack_offsets(&[10.0, 10.0], 0.0, 100.0), vec![100.0, 110.0]);
    }

    #[test]
    fn stack_offsets_clamps_negative_gap_and_height() {
        // Negative gap → 0; negative height → 0 contribution, offset still emitted.
        assert_eq!(
            stack_offsets(&[10.0, -5.0, 7.0], -3.0, 0.0),
            vec![0.0, 10.0, 10.0]
        );
    }

    #[test]
    fn flow_total_height_empty_single_and_multi() {
        assert_eq!(flow_total_height(&[], 4.0), 0.0);
        assert_eq!(flow_total_height(&[12.0], 4.0), 12.0);
        // 10 + 20 + 5 + 4*2 = 43
        assert_eq!(flow_total_height(&[10.0, 20.0, 5.0], 4.0), 43.0);
    }

    // ── Measurement bridge (real CPU font shaping, no GPU) ────────────────────

    #[test]
    fn measure_child_height_is_at_least_one_line_even_when_empty() {
        let mut fs = FontSystem::new();
        let child = FlowChild {
            content: "",
            wrap_width: 200.0,
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            vertical_padding: 6.0,
            markdown_tokens: None,
        };
        let h = measure_child_height(&mut fs, &child);
        // floor of one line: 16 * 1.4 + 6 = 28.4
        assert!(
            (h - (16.0 * LINE_HEIGHT_MULTIPLIER + 6.0)).abs() < 1e-3,
            "got {h}"
        );
    }

    #[test]
    fn measure_child_height_grows_when_content_wraps_to_more_lines() {
        let mut fs = FontSystem::new();
        let narrow = FlowChild {
            content: "the quick brown fox jumps over the lazy dog again and again",
            wrap_width: 60.0,
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            vertical_padding: 0.0,
            markdown_tokens: None,
        };
        let wide = FlowChild {
            wrap_width: 6000.0,
            ..narrow
        };
        let tall = measure_child_height(&mut fs, &narrow);
        let short = measure_child_height(&mut fs, &wide);
        assert!(
            tall > short,
            "narrow wrap must produce more lines → greater height: narrow={tall} wide={short}"
        );
        // Both are a whole number of lines tall (line_count * line_height, no padding).
        let line_height = 16.0 * LINE_HEIGHT_MULTIPLIER;
        assert!(
            (short / line_height).fract().abs() < 1e-3,
            "wide height not line-aligned: {short}"
        );
    }

    // ── Markdown measurement bridge (hud-3xdlf) ───────────────────────────────
    //
    // These cover the three divergences the bead closed: stripped-vs-raw source
    // (different wrap/line count), heading `size_scale` (taller lines, not a
    // uniform `line_count * constant` product), and token-resolved
    // `line_height_multiplier` (not the plain-text constant). Each test either
    // (a) independently reconstructs the expected height from the same
    // low-level shared primitives `measure_markdown_content_height` itself
    // calls (`parse_markdown_subset` + `markdown_spans_to_styled_runs` +
    // `styled_run_spans`), assembled inline here rather than by calling
    // `measure_child_height` a second time — proving the implementation's
    // internal Buffer-building agrees with an independent assembly, not just
    // with itself — or (b) asserts a differential/structural property a bug in
    // any one of the three divergences would violate.

    use crate::markdown::parse_markdown_subset;
    use crate::text::{WRAPPED_TEXT_WRAP, markdown_spans_to_styled_runs, styled_run_spans};
    use glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};

    /// Independently shape `content` exactly the way
    /// [`measure_markdown_content_height`] does, using the same shared
    /// primitives assembled inline (not by calling that function), and return
    /// the resulting height. This is the "expected" side of the
    /// measured-vs-painted comparisons below.
    fn independently_shaped_markdown_height(
        fs: &mut FontSystem,
        content: &str,
        wrap_width: f32,
        font_size_px: f32,
        tokens: &MarkdownTokens,
    ) -> f32 {
        let parsed = parse_markdown_subset(content, tokens);
        let styled_runs = markdown_spans_to_styled_runs(&parsed.plain_text, &parsed.spans);
        let base_line_height = font_size_px * parsed.line_height_multiplier;
        let mut buf = Buffer::new(fs, Metrics::new(font_size_px, base_line_height));
        buf.set_size(fs, Some(wrap_width.max(1.0)), None);
        buf.set_wrap(fs, WRAPPED_TEXT_WRAP);
        let base_attrs = Attrs::new().family(Family::SansSerif).weight(Weight(400));
        let spans = styled_run_spans(
            &parsed.plain_text,
            &styled_runs,
            base_attrs,
            FontFamily::SystemSansSerif,
            font_size_px,
            parsed.line_height_multiplier,
        );
        buf.set_rich_text(fs, spans, base_attrs, Shaping::Advanced);
        buf.shape_until_scroll(fs, false);
        buf.layout_runs()
            .last()
            .map(|run| run.line_top + run.line_height)
            .unwrap_or(base_line_height)
    }

    #[test]
    fn measure_child_height_markdown_matches_independently_shaped_height_for_heading() {
        let mut fs = FontSystem::new();
        let tokens = MarkdownTokens::default();
        // H1 heading (1.75x scale by default) followed by two body lines.
        let content = "# Big Heading\nbody line one\nbody line two";
        let child = FlowChild {
            content,
            wrap_width: 400.0,
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            vertical_padding: 4.0,
            markdown_tokens: Some(&tokens),
        };
        let measured = measure_child_height(&mut fs, &child);
        let expected =
            independently_shaped_markdown_height(&mut fs, content, 400.0, 16.0, &tokens) + 4.0;
        assert!(
            (measured - expected).abs() < 1e-2,
            "measured={measured} expected={expected}"
        );
    }

    #[test]
    fn measure_child_height_markdown_heading_taller_than_uniform_line_estimate() {
        // The bug this bead closes: `line_count * font_size_px * LINE_HEIGHT_MULTIPLIER`
        // assumes every line is the same height, which a heading line violates —
        // it shapes taller than a body line via its `size_scale`d per-span metrics.
        let mut fs = FontSystem::new();
        let tokens = MarkdownTokens::default();
        let content = "# Big Heading\nbody line one";
        let child = FlowChild {
            content,
            wrap_width: 400.0,
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            vertical_padding: 0.0,
            markdown_tokens: Some(&tokens),
        };
        let measured = measure_child_height(&mut fs, &child);
        // Two raw lines' worth of UNIFORM (non-heading) line height — what the
        // pre-fix formula would have produced.
        let uniform_two_line_estimate = 2.0 * 16.0 * LINE_HEIGHT_MULTIPLIER;
        assert!(
            measured > uniform_two_line_estimate + 1.0,
            "a heading line must add real height beyond a uniform 2-line estimate: \
             measured={measured} uniform_estimate={uniform_two_line_estimate}"
        );
    }

    #[test]
    fn measure_child_height_markdown_strips_syntax_before_wrapping() {
        // Raw markdown source is LONGER than its stripped plain_text (the `**`
        // markers and `` ` ``/link brackets cost bytes that never paint). At a
        // wrap width chosen so the extra syntax bytes are the difference between
        // wrapping to 1 line and 2, the markdown path (which wraps the STRIPPED
        // text) must measure fewer lines than the plain-text path (which would
        // wrap the RAW source, exactly the pre-fix bug).
        let mut fs = FontSystem::new();
        let tokens = MarkdownTokens::default();
        let content = "**bold** `code` [a link](https://example.com/x) plain";
        let narrow_wrap = 260.0;

        let markdown_child = FlowChild {
            content,
            wrap_width: narrow_wrap,
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            vertical_padding: 0.0,
            markdown_tokens: Some(&tokens),
        };
        let plain_child = FlowChild {
            markdown_tokens: None,
            ..markdown_child
        };

        let markdown_height = measure_child_height(&mut fs, &markdown_child);
        let plain_height = measure_child_height(&mut fs, &plain_child);
        assert!(
            markdown_height < plain_height,
            "markdown path must measure the STRIPPED (shorter) text, wrapping to \
             fewer/no-more lines than the RAW-source plain-text path at the same \
             width: markdown={markdown_height} plain={plain_height}"
        );

        // Cross-check against the independently-parsed plain_text directly: the
        // parsed text really is shorter than the raw source (sanity that the
        // fixture actually exercises stripping, not a coincidence of wrapping).
        let parsed = parse_markdown_subset(content, &tokens);
        assert!(
            parsed.plain_text.len() < content.len(),
            "fixture must strip real syntax bytes: stripped={} raw={}",
            parsed.plain_text.len(),
            content.len()
        );
    }

    #[test]
    fn measure_child_height_markdown_uses_token_line_height_not_plain_text_constant() {
        // A MarkdownTokens with a line_height_multiplier that diverges from the
        // plain-text LINE_HEIGHT_MULTIPLIER constant. If measure_child_height's
        // markdown branch ever fell back to the constant (the pre-fix bug), this
        // height would not match the independently-shaped expectation below.
        let mut fs = FontSystem::new();
        let tokens = MarkdownTokens {
            line_height_multiplier: 2.2,
            ..MarkdownTokens::default()
        };
        assert!(
            (tokens.line_height_multiplier - LINE_HEIGHT_MULTIPLIER).abs() > 0.5,
            "fixture must diverge meaningfully from the plain-text constant"
        );
        let content = "one line only, no wrap";
        let child = FlowChild {
            content,
            wrap_width: 400.0,
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            vertical_padding: 0.0,
            markdown_tokens: Some(&tokens),
        };
        let measured = measure_child_height(&mut fs, &child);
        let expected = independently_shaped_markdown_height(&mut fs, content, 400.0, 16.0, &tokens);
        assert!(
            (measured - expected).abs() < 1e-2,
            "measured={measured} expected={expected}"
        );
        // And it must NOT equal what the plain-text constant would have given
        // for a single unwrapped line.
        let constant_based = 16.0 * LINE_HEIGHT_MULTIPLIER;
        assert!(
            (measured - constant_based).abs() > 1.0,
            "must diverge from the plain-text-constant estimate: measured={measured} \
             constant_based={constant_based}"
        );
    }

    #[test]
    fn measure_child_height_markdown_multi_paragraph_grows_with_content() {
        let mut fs = FontSystem::new();
        let tokens = MarkdownTokens::default();
        let one_paragraph = "First paragraph of turn content.";
        let three_paragraphs = "First paragraph of turn content.\n\nSecond paragraph, more content here.\n\nThird paragraph closes the turn.";
        fn make_child<'a>(content: &'a str, tokens: &'a MarkdownTokens) -> FlowChild<'a> {
            FlowChild {
                content,
                wrap_width: 300.0,
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                vertical_padding: 0.0,
                markdown_tokens: Some(tokens),
            }
        }
        let short = measure_child_height(&mut fs, &make_child(one_paragraph, &tokens));
        let tall = measure_child_height(&mut fs, &make_child(three_paragraphs, &tokens));
        assert!(
            tall > short,
            "three paragraphs must measure taller than one: short={short} tall={tall}"
        );
    }

    #[test]
    fn measure_child_height_markdown_inline_styles_do_not_panic_and_stay_finite() {
        // Bold / inline-code / link runs exercise styled_run_spans' weight,
        // monospace, and (implicitly, via markdown_spans_to_styled_runs)
        // color-run construction. Height itself is only loosely asserted
        // (non-degenerate); the primary contract here is "does not panic and
        // stays finite/positive" across every styled-run kind at once.
        let mut fs = FontSystem::new();
        let tokens = MarkdownTokens::default();
        let content = "**bold** and *italic* and `inline code` and [a link](https://x.example) and normal text too";
        let child = FlowChild {
            content,
            wrap_width: 220.0,
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            vertical_padding: 2.0,
            markdown_tokens: Some(&tokens),
        };
        let h = measure_child_height(&mut fs, &child);
        assert!(h.is_finite() && h > 0.0, "got {h}");
        // Non-trivial: must wrap to at least a couple of lines at this width.
        let one_line = 16.0 * LINE_HEIGHT_MULTIPLIER;
        assert!(h > one_line * 1.5, "expected multi-line wrap, got {h}");
    }

    #[test]
    fn measure_child_height_markdown_degenerate_inputs_stay_finite_and_nan_safe() {
        let mut fs = FontSystem::new();
        let tokens = MarkdownTokens::default();
        let cases: [(&str, f32, f32, f32); 5] = [
            // (content, wrap_width, font_size_px, vertical_padding)
            ("", 200.0, 16.0, 0.0),               // empty content
            ("# only a heading", 0.0, 16.0, 0.0), // zero wrap width
            ("body", 200.0, 0.0, 0.0),            // zero font size
            ("body", 200.0, 16.0, -10.0),         // negative padding
            ("body", -50.0, 16.0, -5.0),          // negative wrap width AND padding
        ];
        for (content, wrap_width, font_size_px, vertical_padding) in cases {
            let child = FlowChild {
                content,
                wrap_width,
                font_size_px,
                font_family: FontFamily::SystemSansSerif,
                vertical_padding,
                markdown_tokens: Some(&tokens),
            };
            let h = measure_child_height(&mut fs, &child);
            assert!(
                h.is_finite() && h >= 0.0,
                "degenerate input {child:?} produced non-finite/negative height: {h}"
            );
        }
    }

    // ── Full resolution (the demonstration) ───────────────────────────────────

    #[test]
    fn resolve_vertical_flow_stacks_children_without_overlap() {
        let mut fs = FontSystem::new();
        let children = [
            FlowChild {
                content: "assistant turn one",
                wrap_width: 300.0,
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                vertical_padding: 4.0,
                markdown_tokens: None,
            },
            FlowChild {
                content: "tool: ran a command\nand printed two lines",
                wrap_width: 300.0,
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                vertical_padding: 4.0,
                markdown_tokens: None,
            },
            FlowChild {
                content: "assistant turn three",
                wrap_width: 300.0,
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                vertical_padding: 4.0,
                markdown_tokens: None,
            },
        ];
        let gap = 8.0;
        let layout = resolve_vertical_flow(&mut fs, &children, gap, 12.0);

        assert_eq!(layout.offsets.len(), 3);
        // First child sits at the flow origin.
        assert!((layout.offsets[0] - 12.0).abs() < 1e-3);
        // Each subsequent child begins at least its predecessor's offset + a
        // positive height + the gap — i.e. strictly below, no overlap.
        for i in 1..layout.offsets.len() {
            let prev_height = measure_child_height(&mut fs, &children[i - 1]);
            assert!(
                layout.offsets[i] >= layout.offsets[i - 1] + prev_height + gap - 1e-3,
                "child {i} must not overlap child {}: offsets={:?}",
                i - 1,
                layout.offsets
            );
        }
        // Total height spans from the first offset to the bottom of the last child.
        let last_height = measure_child_height(&mut fs, &children[2]);
        let spanned = layout.offsets[2] + last_height - layout.offsets[0];
        assert!(
            (spanned - layout.total_height).abs() < 1e-3,
            "spanned={spanned} total={}",
            layout.total_height
        );
    }

    #[test]
    fn resolve_vertical_flow_empty_is_empty() {
        let mut fs = FontSystem::new();
        let layout = resolve_vertical_flow(&mut fs, &[], 8.0, 0.0);
        assert!(layout.offsets.is_empty());
        assert_eq!(layout.total_height, 0.0);
    }

    // ── Tile-level pre-pass resolver ──────────────────────────────────────────

    fn text_node(content: &str, y: f32, width: f32) -> Node {
        Node {
            id: SceneId::new(),
            children: vec![],
            layout: NodeLayout::Absolute,
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: content.to_string(),
                bounds: Rect::new(0.0, y, width, 20.0),
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::new(1.0, 1.0, 1.0, 1.0),
                background: None,
                alignment: TextAlign::Start,
                overflow: TextOverflow::Ellipsis,
                color_runs: Box::default(),
            }),
        }
    }

    fn scene_map(nodes: Vec<Node>) -> HashMap<SceneId, Node> {
        nodes.into_iter().map(|n| (n.id, n)).collect()
    }

    #[test]
    fn resolve_tile_flow_offsets_empty_for_all_absolute() {
        // Behavior-preserving: no VerticalFlow node → empty map → render path
        // falls back to each child's own bounds.y (byte-identical).
        let mut fs = FontSystem::new();
        let a = text_node("one", 0.0, 300.0);
        let b = text_node("two", 40.0, 300.0);
        let map = scene_map(vec![a, b]);
        let offsets = resolve_tile_flow_offsets(&mut fs, &map, 8.0);
        assert!(
            offsets.is_empty(),
            "absolute-only scene must resolve no flow offsets"
        );
    }

    #[test]
    fn resolve_tile_flow_offsets_stacks_flow_children_from_parent_top() {
        let mut fs = FontSystem::new();
        let c0 = text_node("assistant turn", 0.0, 300.0);
        let c1 = text_node("tool: output line one\nand a second line", 0.0, 300.0);
        let c2 = text_node("assistant again", 0.0, 300.0);
        let (id0, id1, id2) = (c0.id, c1.id, c2.id);

        // A VerticalFlow parent anchored at y=12 owning the three children.
        let mut parent = text_node("", 12.0, 300.0);
        parent.layout = NodeLayout::VerticalFlow;
        parent.children = vec![id0, id1, id2];

        let map = scene_map(vec![parent, c0, c1, c2]);
        let gap = 8.0;
        let offsets = resolve_tile_flow_offsets(&mut fs, &map, gap);

        assert_eq!(offsets.len(), 3, "every flow child gets a resolved y");
        // First child sits at the parent's top.
        assert!(
            (offsets[&id0] - 12.0).abs() < 1e-3,
            "first child at parent top"
        );
        // Children are strictly ordered top-to-bottom and do not overlap.
        assert!(offsets[&id0] < offsets[&id1] && offsets[&id1] < offsets[&id2]);
        let h0 = flow_child_height(&mut fs, &map[&id0]);
        assert!(
            offsets[&id1] >= offsets[&id0] + h0 + gap - 1e-3,
            "second child must clear the first plus the gap: {offsets:?}"
        );
    }
}
