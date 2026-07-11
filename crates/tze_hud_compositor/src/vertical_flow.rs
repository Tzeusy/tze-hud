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
//! compositor's existing CPU wrapped-line shaper so a measured child height agrees
//! with what the render path paints for the same content and width.
//!
//! This is the reusable resolution engine only. Wiring the resolved offsets into
//! the render sites and exposing a per-node layout mode on the scene/wire schema
//! are separate integration steps; the per-turn transcript split that would drive
//! this in production is gated on the Phase-1 Promotion Evidence Gate
//! (text-stream-portals §Phase-1 Promotion Evidence Gate).

use glyphon::FontSystem;

use crate::text::{composer_wrap_line_widths, LINE_HEIGHT_MULTIPLIER};

/// One child's inputs for vertical-flow height measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowChild<'a> {
    /// The child's text content (markdown source, as the render path receives it).
    pub content: &'a str,
    /// The width the child wraps to — its own content box width in px.
    pub wrap_width: f32,
    /// Font size in px used to shape and measure the child.
    pub font_size_px: f32,
    /// Total vertical padding (top + bottom) added around the text inside the
    /// child node, matching the render path's `text_margin * 2` so the measured
    /// row height equals the painted row height.
    pub vertical_padding: f32,
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
/// `wrap_width`, reusing the compositor's CPU wrapped-line shaper
/// ([`composer_wrap_line_widths`]) so the measured height matches what the render
/// path paints for the same content, width, and font size.
///
/// Height = `wrapped_line_count * (font_size_px * LINE_HEIGHT_MULTIPLIER) +
/// vertical_padding`, with a floor of one line so an empty turn still occupies a
/// row rather than collapsing to zero. `wrap_width` and `vertical_padding` are
/// clamped to non-negative.
pub fn measure_child_height(font_system: &mut FontSystem, child: &FlowChild<'_>) -> f32 {
    let line_count = composer_wrap_line_widths(
        font_system,
        child.content,
        child.wrap_width.max(1.0),
        child.font_size_px,
        LINE_HEIGHT_MULTIPLIER,
    )
    .len()
    .max(1);
    let line_height = child.font_size_px * LINE_HEIGHT_MULTIPLIER;
    line_count as f32 * line_height + child.vertical_padding.max(0.0)
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(stack_offsets(&[10.0, 20.0, 5.0], 4.0, 0.0), vec![0.0, 14.0, 38.0]);
    }

    #[test]
    fn stack_offsets_honors_start_y() {
        assert_eq!(stack_offsets(&[10.0, 10.0], 0.0, 100.0), vec![100.0, 110.0]);
    }

    #[test]
    fn stack_offsets_clamps_negative_gap_and_height() {
        // Negative gap → 0; negative height → 0 contribution, offset still emitted.
        assert_eq!(stack_offsets(&[10.0, -5.0, 7.0], -3.0, 0.0), vec![0.0, 10.0, 10.0]);
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
            vertical_padding: 6.0,
        };
        let h = measure_child_height(&mut fs, &child);
        // floor of one line: 16 * 1.4 + 6 = 28.4
        assert!((h - (16.0 * LINE_HEIGHT_MULTIPLIER + 6.0)).abs() < 1e-3, "got {h}");
    }

    #[test]
    fn measure_child_height_grows_when_content_wraps_to_more_lines() {
        let mut fs = FontSystem::new();
        let narrow = FlowChild {
            content: "the quick brown fox jumps over the lazy dog again and again",
            wrap_width: 60.0,
            font_size_px: 16.0,
            vertical_padding: 0.0,
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
        assert!((short / line_height).fract().abs() < 1e-3, "wide height not line-aligned: {short}");
    }

    // ── Full resolution (the demonstration) ───────────────────────────────────

    #[test]
    fn resolve_vertical_flow_stacks_children_without_overlap() {
        let mut fs = FontSystem::new();
        let children = [
            FlowChild { content: "assistant turn one", wrap_width: 300.0, font_size_px: 16.0, vertical_padding: 4.0 },
            FlowChild { content: "tool: ran a command\nand printed two lines", wrap_width: 300.0, font_size_px: 16.0, vertical_padding: 4.0 },
            FlowChild { content: "assistant turn three", wrap_width: 300.0, font_size_px: 16.0, vertical_padding: 4.0 },
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
        assert!((spanned - layout.total_height).abs() < 1e-3, "spanned={spanned} total={}", layout.total_height);
    }

    #[test]
    fn resolve_vertical_flow_empty_is_empty() {
        let mut fs = FontSystem::new();
        let layout = resolve_vertical_flow(&mut fs, &[], 8.0, 0.0);
        assert!(layout.offsets.is_empty());
        assert_eq!(layout.total_height, 0.0);
    }
}
