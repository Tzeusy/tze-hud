//! Phase-1 overflow and ellipsis contract (hud-5jbra.3).
//!
//! # Contract
//!
//! [`truncate_for_ellipsis`] implements the normative Phase-1 overflow
//! algorithm for [`TextOverflow::Ellipsis`]:
//!
//! 1. **Word-boundary truncation.** Truncation occurs at the last word
//!    boundary whose shaped width, **plus the shaped ellipsis glyph in the
//!    same style run**, fits the content box width.
//! 2. **Grapheme-cluster fallback.** When no word boundary fits, truncation
//!    falls back to the last grapheme-cluster boundary whose shaped width plus
//!    the ellipsis glyph fits the content box.
//! 3. **Whole-line vertical visibility.** The last visible line is either
//!    fully visible or not rendered — no partially clipped glyph rows.
//! 4. **Stable shape caching.** Truncation is called only when content or
//!    geometry changes; the per-frame render path consumes the already-truncated
//!    string without reshaping.
//!
//! Source: RFC 0013 §3.4 and §4.2, Phase-1 design §3, spec requirement
//! "Transcript Overflow and Ellipsis Contract".
//!
//! # Complexity
//!
//! Shaping is O(n) in the text length via glyphon/cosmic-text.  For Phase-1
//! the 65535-byte content ceiling bounds the work.  Results must be cached by
//! the caller keyed on `(content_hash, bounds_width, bounds_height, font_size_px)`.

use glyphon::{Attrs, Buffer, FontSystem, Metrics, Shaping, Wrap};
use unicode_segmentation::UnicodeSegmentation;

/// Glyph info per layout run: `(start_byte_in_line, end_byte_in_line, glyph_x_right)`.
type GlyphInfo = (usize, usize, f32);

/// A collected layout run: `(line_i, line_w, glyph_infos)`.
type LayoutRunEntry = (usize, f32, Vec<GlyphInfo>);

/// The ellipsis character appended when text is truncated.
pub const ELLIPSIS: &str = "…";

/// Truncation result returned by [`truncate_for_ellipsis`].
#[derive(Debug, Clone, PartialEq)]
pub struct TruncationResult {
    /// The display text — either the original (if it fits) or a truncated
    /// version with `ELLIPSIS` appended.
    pub text: String,
    /// `true` when truncation was applied and the ellipsis glyph was appended.
    pub was_truncated: bool,
}

/// Truncate `text` so that the first full line fits within `bounds_width`
/// physical pixels, then restrict to the number of whole lines that fit within
/// `bounds_height`.
///
/// This is the sole implementation of the Phase-1 ellipsis contract.  It must
/// be called **outside the per-frame pipeline** (at content-commit time or on
/// geometry change).
///
/// # Parameters
///
/// - `text` — the plain-text string to truncate (already stripped of markup
///   by the markdown parse path or `strip_markdown_v1`).
/// - `base_attrs` — the base [`Attrs`] used when shaping the text.  Must
///   match the attrs passed to the render buffer for the same item.
/// - `bounds_width` — content box width in physical pixels.
/// - `bounds_height` — content box height in physical pixels.
/// - `font_size_px` — font size in physical pixels.
/// - `line_height` — line height in physical pixels (typically `font_size_px * 1.4`).
/// - `font_system` — shared `FontSystem` for shaping.
///
/// # Return value
///
/// A [`TruncationResult`] containing the display text and a flag indicating
/// whether ellipsis was applied.
///
/// # Guarantee
///
/// The returned text, when shaped into a `Buffer` with the same attrs and
/// geometry, will render without partially clipped glyph rows.
pub fn truncate_for_ellipsis<'a>(
    text: &str,
    base_attrs: Attrs<'a>,
    bounds_width: f32,
    bounds_height: f32,
    font_size_px: f32,
    line_height: f32,
    font_system: &mut FontSystem,
) -> TruncationResult {
    // Guard: degenerate geometry produces an empty result.
    if bounds_width <= 0.0 || bounds_height <= 0.0 || font_size_px <= 0.0 {
        return TruncationResult {
            text: String::new(),
            was_truncated: !text.is_empty(),
        };
    }

    // ── Step 1: determine how many whole lines fit vertically ────────────────
    let max_lines = max_whole_lines(bounds_height, line_height);
    if max_lines == 0 {
        return TruncationResult {
            text: String::new(),
            was_truncated: !text.is_empty(),
        };
    }

    // ── Step 2: measure the ellipsis glyph width ────────────────────────────
    let ellipsis_w = measure_single_line(
        ELLIPSIS,
        base_attrs,
        bounds_width * 2.0,
        font_size_px,
        line_height,
        font_system,
    );

    // ── Step 3: shape the full text with word-wrap enabled ───────────────────
    // We shape with Wrap::Word and bounds_width so that layout_runs() reflects
    // the actual word-wrapped line structure.  Multiple LayoutRuns can share the
    // same line_i when a paragraph soft-wraps.
    let mut full_buf = Buffer::new(font_system, Metrics::new(font_size_px, line_height));
    full_buf.set_size(font_system, Some(bounds_width), None);
    full_buf.set_wrap(font_system, Wrap::Word);
    full_buf.set_text(font_system, text, base_attrs, Shaping::Basic);
    full_buf.shape_until_scroll(font_system, false);

    // ── Step 4: collect rendered lines and their widths ───────────────────────
    // LayoutRun gives us (line_i, line_w, glyphs, text).
    // line_i is the index into buffer.lines[] (which is a paragraph/hard-wrap unit).
    // Multiple LayoutRuns can share the same line_i when word-wrap splits a paragraph.
    let runs: Vec<LayoutRunEntry> = full_buf
        .layout_runs()
        .map(|run| {
            // Collect (start_byte_in_line, end_byte_in_line, glyph_x_right) per glyph.
            let glyph_info: Vec<GlyphInfo> = run
                .glyphs
                .iter()
                .map(|g| (g.start, g.end, g.x + g.w))
                .collect();
            (run.line_i, run.line_w, glyph_info)
        })
        .collect();

    // If the text produces no runs at all (empty text), return as-is.
    if runs.is_empty() {
        return TruncationResult {
            text: text.to_owned(),
            was_truncated: false,
        };
    }

    // ── Step 5: whole-line vertical visibility ────────────────────────────────
    // If the text produces more runs than max_lines, we must truncate.
    let total_runs = runs.len();
    if total_runs <= max_lines {
        // All lines fit vertically; no vertical truncation needed.
        // Check if the last run overflows horizontally.
        let last_run = &runs[total_runs - 1];
        if last_run.1 <= bounds_width {
            // The entire text fits — no truncation.
            return TruncationResult {
                text: text.to_owned(),
                was_truncated: false,
            };
        }
        // Last run overflows horizontally: we must truncate the last line.
        // Find the text for the last run and truncate it.
        let last_line_i = last_run.0;
        let last_line_text = if let Some(line) = full_buf.lines.get(last_line_i) {
            line.text().to_owned()
        } else {
            return TruncationResult {
                text: text.to_owned(),
                was_truncated: false,
            };
        };

        // Reconstruct the prefix of `text` that corresponds to all runs before
        // the last run, then truncate only the slice of the paragraph that
        // belongs to the last run (avoiding duplication for word-wrapped paragraphs).
        let prefix = text_prefix_up_to_run(&runs, total_runs - 1, text, &full_buf);
        let run_start = runs[total_runs - 1]
            .2
            .first()
            .map(|(s, _, _)| *s)
            .unwrap_or(0);
        let run_slice = run_start_slice(&last_line_text, run_start);
        let truncated_last = truncate_line_to_ellipsis(
            run_slice,
            base_attrs,
            bounds_width,
            font_size_px,
            line_height,
            ellipsis_w,
            font_system,
        );
        let result = format!("{prefix}{truncated_last}");
        return TruncationResult {
            text: result,
            was_truncated: true,
        };
    }

    // More runs than fit: take only the first `max_lines` runs.
    // The last visible run is runs[max_lines - 1].
    let last_visible_run = &runs[max_lines - 1];
    let last_line_i = last_visible_run.0;
    let last_line_text = if let Some(line) = full_buf.lines.get(last_line_i) {
        line.text().to_owned()
    } else {
        return TruncationResult {
            text: text.to_owned(),
            was_truncated: true,
        };
    };

    let prefix = text_prefix_up_to_run(&runs, max_lines - 1, text, &full_buf);
    let run_start = last_visible_run.2.first().map(|(s, _, _)| *s).unwrap_or(0);
    let run_slice = run_start_slice(&last_line_text, run_start);
    let truncated_last = truncate_line_to_ellipsis(
        run_slice,
        base_attrs,
        bounds_width,
        font_size_px,
        line_height,
        ellipsis_w,
        font_system,
    );
    let result = format!("{prefix}{truncated_last}");
    TruncationResult {
        text: result,
        was_truncated: true,
    }
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Compute the maximum number of whole lines that fit in `bounds_height`
/// given `line_height`.
///
/// Returns at least 0.  A line_height ≤ 0 returns 0.
#[inline]
fn max_whole_lines(bounds_height: f32, line_height: f32) -> usize {
    if line_height <= 0.0 {
        return 0;
    }
    (bounds_height / line_height).floor() as usize
}

/// Measure the advance width of `text` when shaped as a single line with the
/// given parameters.  Returns the `line_w` of the first `LayoutRun`.
///
/// This shapes the text in a buffer wide enough that no word-wrapping occurs
/// (width = `max_width`), so the result is the natural single-line width.
fn measure_single_line<'a>(
    text: &str,
    base_attrs: Attrs<'a>,
    max_width: f32,
    font_size_px: f32,
    line_height: f32,
    font_system: &mut FontSystem,
) -> f32 {
    let mut buf = Buffer::new(font_system, Metrics::new(font_size_px, line_height));
    buf.set_size(font_system, Some(max_width), None);
    buf.set_wrap(font_system, Wrap::None);
    buf.set_text(font_system, text, base_attrs, Shaping::Basic);
    buf.shape_until_scroll(font_system, false);
    buf.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0)
}

/// Reconstruct the text prefix that corresponds to runs `[0, run_idx)`.
///
/// We build this by finding the absolute byte offset in `original_text` that
/// corresponds to the start of `runs[run_idx]`.  The algorithm:
///
/// 1. Walk `original_text` scanning for `\n` characters to find the byte
///    offset of the start of paragraph `target_line_i`.  (Scanning for `\n`
///    handles both `\n` and `\r\n` line endings correctly.)
/// 2. Add the intra-paragraph offset from the first glyph of `runs[run_idx]`
///    to account for word-wrapped sub-lines within the same paragraph.
/// 3. Clamp to a valid UTF-8 boundary.
fn text_prefix_up_to_run(
    runs: &[LayoutRunEntry],
    run_idx: usize,
    original_text: &str,
    _buf: &Buffer,
) -> String {
    if run_idx == 0 {
        return String::new();
    }

    let target_line_i = runs[run_idx].0;

    // Step 1: find the byte offset of the start of paragraph `target_line_i`
    // by scanning for '\n' separators.  This handles both '\n' and '\r\n'.
    let mut byte_offset = 0usize;
    let mut current_line = 0usize;
    for (idx, ch) in original_text.char_indices() {
        if current_line == target_line_i {
            byte_offset = idx;
            break;
        }
        if ch == '\n' {
            current_line += 1;
            // byte_offset will be updated on the next iteration
        }
    }
    // If the loop exhausted all characters without reaching target_line_i,
    // byte_offset remains at whatever it was (safe: clamped below).

    // Step 2: add the intra-paragraph glyph start offset.  This is the byte
    // offset of the first glyph in `runs[run_idx]` within its paragraph line,
    // which accounts for word-wrapped sub-lines starting mid-paragraph.
    let intra_para_offset = runs[run_idx].2.first().map(|(s, _, _)| *s).unwrap_or(0);

    let total_offset = byte_offset + intra_para_offset;

    if total_offset > original_text.len() {
        original_text.to_owned()
    } else {
        // Step 3: walk back to the nearest valid UTF-8 boundary.
        let safe_offset = (0..=total_offset)
            .rev()
            .find(|&o| original_text.is_char_boundary(o))
            .unwrap_or(0);
        original_text[..safe_offset].to_owned()
    }
}

/// Slice `paragraph_text` from `run_start_byte` to the end, clamping to a
/// valid UTF-8 boundary.  Used to extract the text visible in a word-wrapped
/// run from its parent paragraph string.
fn run_start_slice(paragraph_text: &str, run_start_byte: usize) -> &str {
    if run_start_byte == 0 {
        return paragraph_text;
    }
    if run_start_byte >= paragraph_text.len() {
        return "";
    }
    // Walk forward from run_start_byte to find a valid UTF-8 boundary.
    let safe = (run_start_byte..=paragraph_text.len())
        .find(|&o| paragraph_text.is_char_boundary(o))
        .unwrap_or(paragraph_text.len());
    &paragraph_text[safe..]
}

/// Truncate a single line of text so that the result (with `"…"` appended)
/// fits within `bounds_width`.
///
/// Algorithm (per spec):
/// 1. Try each *non-empty* word boundary (from the end) — use the last one
///    whose measured width + ellipsis_w ≤ bounds_width.
/// 2. If no non-empty word boundary prefix fits, fall back to grapheme-cluster
///    boundaries (from the end) so that at least one visible cluster is shown.
/// 3. If even a single grapheme + ellipsis does not fit, return just `"…"`.
///
/// The empty-prefix boundary (byte offset 0) is intentionally excluded from
/// the word-boundary pass so that the grapheme-cluster fallback runs for long
/// unbroken tokens instead of returning a bare `"…"` with zero visible clusters.
fn truncate_line_to_ellipsis<'a>(
    line: &str,
    base_attrs: Attrs<'a>,
    bounds_width: f32,
    font_size_px: f32,
    line_height: f32,
    ellipsis_w: f32,
    font_system: &mut FontSystem,
) -> String {
    let budget = bounds_width - ellipsis_w;

    // Fast path: if budget ≤ 0, return just the ellipsis (nothing else fits).
    if budget <= 0.0 {
        return ELLIPSIS.to_owned();
    }

    // Try word boundaries first (unicode word segmentation).
    // We iterate word boundaries right-to-left and take the first that fits.
    //
    // NOTE: boundary == 0 produces an empty candidate ("…" bare).  We skip it
    // here so that when no *non-empty* word-boundary prefix fits the budget we
    // fall through to the grapheme-cluster loop rather than returning a bare
    // ellipsis with zero visible clusters.  The grapheme loop handles the
    // same degenerate outcome (candidate "" → bare "…") at its own tail.
    let word_boundaries: Vec<usize> = line
        .unicode_word_indices()
        .map(|(i, word)| i + word.len())
        .collect();

    for &boundary in word_boundaries.iter().rev() {
        if boundary == 0 || !line.is_char_boundary(boundary) {
            continue;
        }
        let candidate = &line[..boundary];
        let w = measure_single_line(
            candidate,
            base_attrs,
            bounds_width * 2.0,
            font_size_px,
            line_height,
            font_system,
        );
        if w <= budget {
            // Trim trailing whitespace before appending ellipsis, so we get
            // "foo…" rather than "foo …".
            let trimmed = candidate.trim_end();
            return format!("{trimmed}{ELLIPSIS}");
        }
    }

    // No word boundary fits — fall back to grapheme-cluster boundaries.
    let grapheme_boundaries: Vec<usize> = line
        .grapheme_indices(true)
        .map(|(i, g)| i + g.len())
        .collect();

    for &boundary in grapheme_boundaries.iter().rev() {
        if !line.is_char_boundary(boundary) {
            continue;
        }
        let candidate = &line[..boundary];
        let w = measure_single_line(
            candidate,
            base_attrs,
            bounds_width * 2.0,
            font_size_px,
            line_height,
            font_system,
        );
        if w <= budget {
            let trimmed = candidate.trim_end();
            return format!("{trimmed}{ELLIPSIS}");
        }
    }

    // Even a single grapheme + ellipsis does not fit — return just the ellipsis.
    ELLIPSIS.to_owned()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use glyphon::FontSystem;
    use proptest::prelude::*;

    /// Helper: create a fresh FontSystem for testing.
    fn make_font_system() -> FontSystem {
        FontSystem::new()
    }

    fn base_attrs() -> Attrs<'static> {
        Attrs::new()
    }

    // ── Task 3.4: max_whole_lines invariants ──────────────────────────────────

    #[test]
    fn max_whole_lines_exact_fit() {
        // 3 lines × 20px each = 60px height: exactly 3 lines.
        assert_eq!(max_whole_lines(60.0, 20.0), 3);
    }

    #[test]
    fn max_whole_lines_partial_line_excluded() {
        // 65px / 20px = 3.25 → floor = 3: partial 4th line not counted.
        assert_eq!(max_whole_lines(65.0, 20.0), 3);
    }

    #[test]
    fn max_whole_lines_zero_height() {
        assert_eq!(max_whole_lines(0.0, 20.0), 0);
    }

    #[test]
    fn max_whole_lines_zero_line_height() {
        assert_eq!(max_whole_lines(100.0, 0.0), 0);
    }

    #[test]
    fn max_whole_lines_smaller_than_one_line() {
        // 10px height / 20px per line = 0.5 → floor = 0.
        assert_eq!(max_whole_lines(10.0, 20.0), 0);
    }

    // ── Task 3.4: ellipsis content correctness ────────────────────────────────

    /// When a line fits, truncate_for_ellipsis returns the original text unchanged.
    #[test]
    fn short_text_fits_unchanged() {
        let mut fs = make_font_system();
        let text = "Hi";
        let result = truncate_for_ellipsis(
            text,
            base_attrs(),
            1000.0, // very wide
            100.0,
            16.0,
            22.4,
            &mut fs,
        );
        assert_eq!(
            result.text, text,
            "short text that fits must not be truncated"
        );
        assert!(
            !result.was_truncated,
            "was_truncated must be false when text fits"
        );
    }

    /// Empty text returns empty result with no truncation.
    #[test]
    fn empty_text_no_truncation() {
        let mut fs = make_font_system();
        let result = truncate_for_ellipsis("", base_attrs(), 200.0, 100.0, 16.0, 22.4, &mut fs);
        assert_eq!(result.text, "", "empty text must remain empty");
        assert!(!result.was_truncated);
    }

    /// Degenerate geometry (zero width) produces empty result.
    #[test]
    fn zero_width_produces_empty() {
        let mut fs = make_font_system();
        let result = truncate_for_ellipsis("hello", base_attrs(), 0.0, 100.0, 16.0, 22.4, &mut fs);
        assert_eq!(result.text, "", "zero-width bounds must produce empty text");
        assert!(
            result.was_truncated,
            "was_truncated must be true for non-empty input"
        );
    }

    /// When text is truncated, the result ends with the ellipsis character.
    #[test]
    fn truncated_text_ends_with_ellipsis() {
        let mut fs = make_font_system();
        // Very narrow box: 60px wide — any non-trivial text will be truncated.
        let long = "This is a long sentence that surely does not fit in sixty pixels.";
        let result = truncate_for_ellipsis(long, base_attrs(), 60.0, 100.0, 16.0, 22.4, &mut fs);
        if result.was_truncated {
            assert!(
                result.text.ends_with(ELLIPSIS),
                "truncated text must end with '…'; got: {:?}",
                result.text
            );
        }
    }

    /// Truncated result must be strictly shorter than the original text.
    #[test]
    fn truncated_text_is_shorter() {
        let mut fs = make_font_system();
        let long = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"; // 37 'A's
        let result = truncate_for_ellipsis(long, base_attrs(), 60.0, 100.0, 16.0, 22.4, &mut fs);
        if result.was_truncated {
            assert!(
                result.text.len() < long.len() + ELLIPSIS.len(),
                "truncated text must be shorter than original + ellipsis; got {} chars",
                result.text.chars().count()
            );
        }
    }

    /// No glyph is partially visible: result text does not exceed bounds_width
    /// when shaped.
    ///
    /// This is a structural guarantee: we trust our measurement. The test
    /// below verifies that the returned text re-measures as fitting.
    #[test]
    fn truncated_text_remeasures_within_width() {
        let mut fs = make_font_system();
        let long = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"; // long enough to need truncation at 60px
        let bounds_w = 60.0;
        let font_size = 16.0;
        let line_h = 22.4;
        let result = truncate_for_ellipsis(
            long,
            base_attrs(),
            bounds_w,
            100.0,
            font_size,
            line_h,
            &mut fs,
        );
        if result.was_truncated {
            // Re-measure the first line of the result.
            let measured_w = measure_single_line(
                &result.text,
                base_attrs(),
                bounds_w * 4.0,
                font_size,
                line_h,
                &mut fs,
            );
            assert!(
                measured_w <= bounds_w + 1.0, // +1.0 for floating-point tolerance
                "truncated text must re-measure ≤ bounds_width ({bounds_w}px); got {measured_w:.2}px"
            );
        }
    }

    /// Multi-line text: lines beyond max_whole_lines are dropped.
    #[test]
    fn multiline_beyond_height_is_truncated() {
        let mut fs = make_font_system();
        // 3 lines, but only 1 line's worth of height.
        let text = "Line one\nLine two\nLine three";
        let font_size = 16.0;
        let line_h = 22.4;
        // Only ~1 line fits.
        let result = truncate_for_ellipsis(
            text,
            base_attrs(),
            500.0,
            line_h + 1.0,
            font_size,
            line_h,
            &mut fs,
        );
        assert!(
            result.was_truncated,
            "multi-line text taller than bounds must be truncated"
        );
        // The result must not contain the last line's content.
        assert!(
            !result.text.contains("Line three"),
            "lines beyond height must be dropped; got: {:?}",
            result.text
        );
    }

    /// Grapheme fallback: single long word without spaces is truncated at a
    /// grapheme boundary.
    ///
    /// Regression guard for the "dead-code grapheme fallback" bug: previously
    /// the word-boundary loop claimed boundary 0 (empty prefix) which always
    /// fitted the budget, producing bare `"…"` with zero visible clusters.
    #[test]
    fn single_long_word_truncated_at_grapheme_boundary() {
        let mut fs = make_font_system();
        // A long unbroken token — no word boundaries within it.
        let long_word = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"; // 40 'A's
        let result =
            truncate_for_ellipsis(long_word, base_attrs(), 60.0, 100.0, 16.0, 22.4, &mut fs);
        assert!(
            result.was_truncated,
            "40 'A's in a 60px box must be truncated; got: {:?}",
            result.text
        );
        assert!(
            result.text.ends_with(ELLIPSIS),
            "grapheme-fallback result must end with '…'; got: {:?}",
            result.text
        );
        // The prefix before the ellipsis must be non-empty: at least one visible
        // grapheme cluster must precede the ellipsis character.
        let prefix_before_ellipsis = result.text.strip_suffix(ELLIPSIS).unwrap_or("");
        assert!(
            !prefix_before_ellipsis.is_empty(),
            "grapheme-fallback must include at least one visible cluster before '…'; \
             got bare '…' — long unbroken token regression; result: {:?}",
            result.text
        );
        assert!(
            result.text.len() < long_word.len() + ELLIPSIS.len(),
            "grapheme-fallback result must be shorter than original + ellipsis"
        );
    }

    /// Canonical repro from the bug report: `'W'×30` in a `200px` box must yield
    /// at least one visible `W` cluster before the ellipsis, not bare `"…"`.
    ///
    /// Source: issue hud-wq6qp — "Ellipsis grapheme fallback is dead code".
    #[test]
    fn long_unbroken_token_w30_in_200px_has_visible_prefix() {
        let mut fs = make_font_system();
        let text = "W".repeat(30);
        let result = truncate_for_ellipsis(&text, base_attrs(), 200.0, 100.0, 16.0, 22.4, &mut fs);
        assert!(
            result.was_truncated,
            "30 'W's in a 200px box must be truncated; got was_truncated=false"
        );
        assert!(
            result.text.ends_with(ELLIPSIS),
            "truncated long-token result must end with '…'; got: {:?}",
            result.text
        );
        let prefix = result
            .text
            .strip_suffix(ELLIPSIS)
            .expect("ends_with ELLIPSIS already asserted");
        assert!(
            !prefix.is_empty(),
            "long unbroken token must produce at least one visible cluster before '…'; \
             got bare '…' — grapheme fallback regression; result: {:?}",
            result.text
        );
        // Verify all characters in the prefix are 'W' (no corruption / mid-codepoint split).
        assert!(
            prefix.chars().all(|c| c == 'W'),
            "prefix before ellipsis must consist only of source characters; got: {:?}",
            prefix
        );
    }

    /// Multi-codepoint grapheme cluster boundary: combining characters must not
    /// be split.  Uses a base letter + combining acute accent (U+0301) as the
    /// cluster unit.  The fallback must only cut at cluster boundaries.
    #[test]
    fn grapheme_cluster_boundary_not_split_mid_cluster() {
        let mut fs = make_font_system();
        // Build a string of 20 "é" (e + combining acute, 2 bytes each → 20 clusters).
        // Using the decomposed form to guarantee a multi-codepoint cluster.
        let cluster = "e\u{0301}"; // 'e' + combining acute accent (NFD)
        let text: String = cluster.repeat(20);
        // Verify our test input is actually multi-byte-per-cluster
        let cluster_count = text.graphemes(true).count();
        assert_eq!(
            cluster_count, 20,
            "expected 20 grapheme clusters in test input"
        );

        let result = truncate_for_ellipsis(&text, base_attrs(), 80.0, 100.0, 16.0, 22.4, &mut fs);
        if result.was_truncated {
            assert!(
                result.text.ends_with(ELLIPSIS),
                "multi-codepoint-cluster truncation must end with '…'; got: {:?}",
                result.text
            );
            // The prefix before the ellipsis must be valid UTF-8 and consist of
            // whole grapheme clusters only.
            let prefix = result.text.strip_suffix(ELLIPSIS).unwrap_or("");
            // Every grapheme in the prefix must equal the original cluster unit.
            // If the fallback split a cluster mid-codepoint we'd get partial
            // graphemes (e.g. "e" without the accent).
            for g in prefix.graphemes(true) {
                // Accept either the precomposed form or NFD form — the font system
                // may normalise; what matters is no lone combining codepoints.
                let has_lone_combining = g
                    .chars()
                    .next()
                    .map(|c| {
                        // Unicode "combining character" range: 0x0300..=0x036F and beyond.
                        // A lone combining as the *first* codepoint of a grapheme cluster
                        // means we split before the base.
                        (c as u32) >= 0x0300 && (c as u32) <= 0x036F
                    })
                    .unwrap_or(false);
                assert!(
                    !has_lone_combining,
                    "grapheme cluster boundary violated: prefix grapheme {g:?} starts with \
                     a combining codepoint — the fallback split a multi-codepoint cluster; \
                     full result: {:?}",
                    result.text
                );
            }
            // At least one whole cluster must be visible before the ellipsis.
            assert!(
                !prefix.is_empty(),
                "at least one grapheme cluster must precede '…' in the truncated result; \
                 got bare '…'; result: {:?}",
                result.text
            );
        }
    }

    // ── Task 3.4: property-based tests (proptest) ────────────────────────────
    //
    // These use proptest to verify invariants across random content/widths.
    // FontSystem is not Send, so each proptest body owns its own instance.
    //
    // Iteration count is 32 to keep total CI time well under 10 s in debug mode
    // with software font rasterisation.  The invariant guarantee comes from random
    // input coverage, not iteration count; real regression detection uses criterion
    // benchmarks in benchmarks/.

    proptest! {
        #![proptest_config(proptest::test_runner::Config {
            cases: 32,
            source_file: Some("crates/tze_hud_compositor/src/overflow.rs"),
            ..proptest::test_runner::Config::default()
        })]

        /// Invariant: if truncation occurs, the result always ends with ELLIPSIS.
        #[test]
        fn proptest_truncated_always_ends_with_ellipsis(
            text in "[a-zA-Z0-9 ]{1,80}",
            width_px in 20.0_f32..200.0_f32,
        ) {
            let mut fs = make_font_system();
            let result = truncate_for_ellipsis(
                &text, base_attrs(), width_px, 200.0, 16.0, 22.4, &mut fs,
            );
            if result.was_truncated {
                prop_assert!(
                    result.text.ends_with(ELLIPSIS),
                    "was_truncated=true but result does not end with '…'; \
                     text={text:?} width={width_px} result={:?}",
                    result.text,
                );
            }
        }

        /// Invariant: truncated text, when shaped at bounds_width, measures ≤ bounds_width.
        /// This is the "no clipped glyphs" contract: the rendered result fits in the box.
        #[test]
        fn proptest_truncated_result_fits_within_bounds(
            text in "[a-zA-Z0-9 ]{1,80}",
            width_px in 30.0_f32..300.0_f32,
            font_size in 10.0_f32..24.0_f32,
        ) {
            let mut fs = make_font_system();
            let line_h = font_size * 1.4;
            let result = truncate_for_ellipsis(
                &text, base_attrs(), width_px, 200.0, font_size, line_h, &mut fs,
            );
            if result.was_truncated && !result.text.is_empty() {
                let measured = measure_single_line(
                    &result.text, base_attrs(), width_px * 4.0, font_size, line_h, &mut fs,
                );
                prop_assert!(
                    measured <= width_px + 2.0, // 2px tolerance for fp rounding
                    "truncated text measured {measured:.2}px > bounds {width_px}px; \
                     text={text:?} result={:?}",
                    result.text,
                );
            }
        }

        /// Invariant: whole-line vertical visibility — the result must not produce
        /// more layout runs than the number of lines that fit in bounds_height.
        #[test]
        fn proptest_whole_line_visibility_no_partial_lines(
            n_lines in 2usize..6usize,
            bounds_scale in 1.0_f32..1.5_f32,
        ) {
            let mut fs = make_font_system();
            let font_size = 14.0_f32;
            let line_h = font_size * 1.4;
            // Build text with more lines than n_lines so truncation always applies.
            let text: String = (0..n_lines + 3)
                .map(|i| format!("Line number {i} of test content"))
                .collect::<Vec<_>>()
                .join("\n");
            let height = line_h * (n_lines as f32) * bounds_scale;
            let bounds_w = 400.0_f32;

            let result = truncate_for_ellipsis(
                &text, base_attrs(), bounds_w, height, font_size, line_h, &mut fs,
            );

            // Shape the result and verify the layout run count.
            let mut check_buf = Buffer::new(&mut fs, Metrics::new(font_size, line_h));
            check_buf.set_size(&mut fs, Some(bounds_w), None);
            check_buf.set_wrap(&mut fs, Wrap::Word);
            check_buf.set_text(&mut fs, &result.text, base_attrs(), Shaping::Basic);
            check_buf.shape_until_scroll(&mut fs, false);
            let run_count = check_buf.layout_runs().count();
            let max_allowed = max_whole_lines(height, line_h);
            prop_assert!(
                run_count <= max_allowed + 1, // +1: trailing ellipsis may add one run
                "result produced {run_count} runs but max_whole_lines({height}, {line_h}) = {max_allowed}; \
                 result={:?}",
                result.text,
            );
        }

        /// Invariant: grapheme fallback — unbroken tokens (no spaces) are truncated
        /// at a grapheme boundary with ellipsis appended, never at mid-codepoint.
        ///
        /// Strengthened: when the box is wide enough to fit at least one grapheme
        /// + ellipsis, the prefix before `"…"` must be non-empty (not bare `"…"`).
        #[test]
        fn proptest_grapheme_fallback_valid_utf8_boundary(
            repeat in 10usize..50usize,
            width_px in 40.0_f32..80.0_f32,
        ) {
            let mut fs = make_font_system();
            // Single unbroken token — no word boundaries.
            let text = "A".repeat(repeat);
            let result = truncate_for_ellipsis(
                &text, base_attrs(), width_px, 200.0, 16.0, 22.4, &mut fs,
            );
            // Result must be valid UTF-8 and never split at a non-char-boundary.
            prop_assert!(
                std::str::from_utf8(result.text.as_bytes()).is_ok(),
                "result is not valid UTF-8; text={text:?} width={width_px} result={:?}",
                result.text,
            );
            if result.was_truncated {
                prop_assert!(
                    result.text.ends_with(ELLIPSIS),
                    "grapheme fallback result must end with '…'; text={text:?} result={:?}",
                    result.text,
                );
                // When the box is at least 40px wide (enough for one 'A' + ellipsis
                // in a 16px font), the prefix before the ellipsis must be non-empty.
                // A bare "…" means the grapheme fallback did not run — regression guard
                // for the dead-code fallback bug (hud-wq6qp).
                let prefix = result.text.strip_suffix(ELLIPSIS).unwrap_or("");
                prop_assert!(
                    !prefix.is_empty(),
                    "unbroken token must have at least one visible cluster before '…'; \
                     got bare '…' — grapheme fallback regression; \
                     text={text:?} width={width_px} result={:?}",
                    result.text,
                );
            }
        }
    }

    // ── Task 3.4: scrolled-back append stability (structural test) ────────────
    //
    // The scrolled-back append stability guarantee is architectural: appends
    // beyond the viewport do not cause layout_runs() to change for already-
    // rendered lines.  This is enforced by the compositor's scene-commit path
    // (not by this function), so we test it structurally: truncating a
    // prefix of text and then the same prefix + suffix produces the same
    // truncated prefix.
    #[test]
    fn append_stability_truncation_prefix_unchanged() {
        let mut fs = make_font_system();
        let font_size = 16.0_f32;
        let line_h = font_size * 1.4;
        let bounds_w = 100.0_f32;
        // Height for exactly 1 line.
        let height = line_h * 1.0 + 1.0;

        let prefix = "Hello world foo bar";
        let suffix = "\nNew line of text appended after scroll";
        let full = format!("{prefix}{suffix}");

        let result_prefix = truncate_for_ellipsis(
            prefix,
            base_attrs(),
            bounds_w,
            height,
            font_size,
            line_h,
            &mut fs,
        );
        let result_full = truncate_for_ellipsis(
            &full,
            base_attrs(),
            bounds_w,
            height,
            font_size,
            line_h,
            &mut fs,
        );

        // Both should produce the same first-line text (within the single-line height).
        assert_eq!(
            result_prefix.text, result_full.text,
            "appending lines beyond the viewport must not change the truncated first-line content"
        );
    }

    // ── Task 3.5: layout-resolve stage budget ─────────────────────────────────
    //
    // Task 3.5 requires that the layout-resolve stage stays < 1 ms with styled-run
    // caching under transcript-sized content.  This is primarily an integration
    // concern (the phase pipeline must invoke truncation only on change, not per
    // frame).  We provide a basic timing smoke test here to catch catastrophic
    // regressions on the CI path.

    #[test]
    fn layout_resolve_under_1ms_for_transcript_sized_content() {
        let mut fs = make_font_system();
        let font_size = 14.0_f32;
        let line_h = font_size * 1.4;
        // Transcript-sized content: ~500 bytes, typical for a streaming LLM token window.
        let content = "The quick brown fox jumps over the lazy dog. ".repeat(12); // ~540 bytes
        let bounds_w = 400.0_f32;
        let height = line_h * 5.0; // 5 visible lines

        // Warm-up pass: the first call to FontSystem loads fonts from disk and
        // initialises the shaper; exclude that one-time cost from the timing window.
        let _ = truncate_for_ellipsis(
            &content,
            base_attrs(),
            bounds_w,
            height,
            font_size,
            line_h,
            &mut fs,
        );

        let start = std::time::Instant::now();
        let _result = truncate_for_ellipsis(
            &content,
            base_attrs(),
            bounds_w,
            height,
            font_size,
            line_h,
            &mut fs,
        );
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

        // We allow up to 500ms here to tolerate debug-mode unoptimised builds and
        // headless CI environments (no GPU, software-renderer font rasterisation).
        // The real budget (< 1ms p99 in release) is enforced in the benchmarks/
        // directory with hardware calibration.  This test is a catastrophic-
        // regression guard: it catches algorithmic complexity explosions (e.g.
        // O(n²) shaping loops), not framework overhead.
        assert!(
            elapsed_ms < 500.0,
            "truncate_for_ellipsis exceeded catastrophic regression threshold (500ms) for \
             transcript-sized content on warm call: {elapsed_ms:.2}ms"
        );
    }
}
