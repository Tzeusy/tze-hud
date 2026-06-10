//! Phase-1 Markdown subset parser for tze_hud portal text.
//!
//! # Design contract (hud-5jbra.2)
//!
//! Parsing happens **at content-commit time**, never on the per-frame render
//! path.  The compositor calls [`MarkdownCache::prime`] once after each
//! scene commit, which parses any new or changed [`TextMarkdownNode`] content
//! and stores the result in a BLAKE3-keyed cache.  The frame pipeline
//! consumes only cached [`ParsedMarkdown`] values — a node whose content has
//! not changed incurs **zero** parse cost per frame.
//!
//! # Subset
//!
//! Supported constructs (rendered with token-driven styling):
//! - ATX headings (H1–H6)
//! - Bold (`**text**` / `__text__`)
//! - Italic (`*text*` / `_text_`)
//! - Bold-italic (`***text***`)
//! - Inline code (`` `code` ``)
//! - Fenced code blocks (`` ``` `` and `~~~`)
//! - Indented code blocks (4-space / tab indent)
//! - Ordered lists (` 1. item`)
//! - Unordered lists (`- item`, `* item`, `+ item`)
//! - Links (rendered as styled non-navigable text; destination ignored)
//!
//! Excluded constructs (rendered as literal source text, never silently
//! dropped):
//! - Tables
//! - Images (`![alt](url)`)
//! - Raw HTML
//! - Blockquotes (`> text`)
//! - Footnotes
//! - Strikethrough (`~~text~~`)
//! - Task lists (`- [ ] item`)
//! - Autolinks (`<https://...>`)
//!
//! # Styling
//!
//! All visual values come from [`MarkdownTokens`], which is resolved from the
//! runtime's design-token map at startup (and on any token-map update).  No
//! values are hardcoded in this module; callers that have not yet loaded a
//! token map receive sensible defaults.
//!
//! [`TextMarkdownNode`]: tze_hud_scene::types::TextMarkdownNode
//! [`MarkdownCache::prime`]: MarkdownCache::prime

use std::collections::HashMap;

use tze_hud_scene::types::Rgba;

// ─── MarkdownTokens ──────────────────────────────────────────────────────────

/// Token-resolved styling values for the Phase-1 markdown subset.
///
/// All fields carry defaults so rendering works before a token map is loaded.
/// Populated by [`MarkdownTokens::from_token_map`].
///
/// # Token keys
///
/// | Key | Effect |
/// |-----|--------|
/// | `typography.code.family` | `"monospace"` enables monospace for code spans |
/// | `color.code.background` | Background tint behind inline code (currently propagated as color modifier) |
/// | `color.link.text` | Link text color |
/// | `typography.heading.N.weight` | CSS weight for heading level N (1–6) |
/// | `typography.heading.N.scale` | Font-size multiplier for heading level N |
/// | `typography.emphasis.italic` | Whether emphasis uses italic (always true) |
#[derive(Clone, Debug)]
pub struct MarkdownTokens {
    /// Font size multiplier per heading level (index 0 = H1, 5 = H6).
    pub heading_scale: [f32; 6],
    /// CSS font weight per heading level (index 0 = H1, 5 = H6).
    pub heading_weight: [u16; 6],
    /// Color override for link text.  `None` = no override (falls back to node color).
    pub link_color: Option<Rgba>,
    /// Whether inline code and code blocks use the monospace family.  Defaults to `true`.
    pub code_monospace: bool,
    /// Optional foreground color for code spans and blocks.  `None` = no override.
    pub code_color: Option<Rgba>,
}

impl Default for MarkdownTokens {
    fn default() -> Self {
        // Sensible defaults: heading weight decreases with level, scale reflects
        // a modest typographic ramp.  These match the canonical token schema
        // described in the spec (no token key = fall back to these values).
        Self {
            heading_scale: [1.75, 1.50, 1.25, 1.10, 1.00, 0.90],
            heading_weight: [700, 700, 700, 700, 600, 600],
            link_color: None,
            code_monospace: true,
            code_color: None,
        }
    }
}

impl MarkdownTokens {
    /// Resolve styling from a design-token map.
    ///
    /// Unrecognised or unparseable token values fall back to [`Default`]
    /// values, so callers receive sensible rendering even with a partial or
    /// empty token map.
    pub fn from_token_map(map: &HashMap<String, String>) -> Self {
        let mut tokens = Self::default();

        // Heading weights: typography.heading.{1..6}.weight
        for (i, level) in (1u8..=6).enumerate() {
            let key = format!("typography.heading.{level}.weight");
            if let Some(w) = map.get(&key).and_then(|v| v.parse::<u16>().ok()) {
                if (100..=900).contains(&w) {
                    tokens.heading_weight[i] = w;
                }
            }
        }

        // Heading scales: typography.heading.{1..6}.scale
        for (i, level) in (1u8..=6).enumerate() {
            let key = format!("typography.heading.{level}.scale");
            if let Some(s) = map.get(&key).and_then(|v| v.parse::<f32>().ok()) {
                if s > 0.0 {
                    tokens.heading_scale[i] = s;
                }
            }
        }

        // Link color: color.link.text (hex #RRGGBB or #RRGGBBAA)
        if let Some(c) = map.get("color.link.text").and_then(|v| parse_hex_color(v)) {
            tokens.link_color = Some(c);
        }

        // Code family: typography.code.family = "monospace" | "sans-serif" | ...
        if let Some(fam) = map.get("typography.code.family") {
            tokens.code_monospace = fam.to_lowercase().contains("mono");
        }

        // Code foreground: color.code.text
        if let Some(c) = map.get("color.code.text").and_then(|v| parse_hex_color(v)) {
            tokens.code_color = Some(c);
        }

        tokens
    }
}

// ─── StyleAttr ───────────────────────────────────────────────────────────────

/// Style attributes that can be applied to a byte range in the plain-text output.
///
/// These are compositor-native: they are used to build glyphon `Attrs` spans
/// in `text.rs` without any per-frame string manipulation.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleAttr {
    /// Overrides the base font weight if `Some`.  E.g. `Some(700)` for bold.
    pub weight: Option<u16>,
    /// When `true`, the italic style variant is requested.
    pub italic: bool,
    /// When `true`, monospace family is used instead of the node default.
    pub monospace: bool,
    /// Overrides the text color if `Some`.
    pub color: Option<Rgba>,
}

impl StyleAttr {
    /// The "no styling" identity — used for spans with no markdown decoration.
    pub fn plain() -> Self {
        Self {
            weight: None,
            italic: false,
            monospace: false,
            color: None,
        }
    }

    /// Returns `true` when no attribute override is active.
    pub fn is_plain(&self) -> bool {
        self.weight.is_none() && !self.italic && !self.monospace && self.color.is_none()
    }
}

// ─── StyledSpan ──────────────────────────────────────────────────────────────

/// A styled span in the [`ParsedMarkdown::plain_text`] output.
///
/// Byte offsets are valid UTF-8 boundaries into [`ParsedMarkdown::plain_text`].
/// The span is exclusive-end: `[start_byte, end_byte)`.
#[derive(Clone, Debug, PartialEq)]
pub struct StyledSpan {
    /// Inclusive byte offset into `ParsedMarkdown::plain_text`.
    pub start_byte: usize,
    /// Exclusive byte offset into `ParsedMarkdown::plain_text`.
    pub end_byte: usize,
    /// Style attributes for this span.
    pub attr: StyleAttr,
}

// ─── ParsedMarkdown ──────────────────────────────────────────────────────────

/// The cached result of parsing a single `TextMarkdownNode` content string.
///
/// Produced by [`parse_markdown_subset`] and stored by [`MarkdownCache`].
///
/// - `plain_text` is the rendered text (markup stripped; excluded constructs
///   preserve literal source text).
/// - `spans` covers only runs with non-plain styling; unstyled gaps between
///   spans inherit the node's base style.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedMarkdown {
    /// Plain text output, suitable for glyph layout.
    pub plain_text: String,
    /// Styled spans, sorted by `start_byte`, non-overlapping.
    pub spans: Vec<StyledSpan>,
}

// ─── MarkdownCache ────────────────────────────────────────────────────────────

/// Content-addressed cache of [`ParsedMarkdown`] values.
///
/// The key is a BLAKE3 hash of the raw content bytes.  This gives O(1)
/// lookup: the compositor builds the key from the node's content once
/// per commit, then consults the cache on every frame with zero parse work.
///
/// The cache is unbounded in Phase-1 (content strings are capped at 65535
/// bytes each; total cache growth is bounded by the total number of distinct
/// `TextMarkdownNode` content values alive in the scene).  A proper LRU
/// eviction policy is deferred to a post-promotion follow-up.
#[derive(Default)]
pub struct MarkdownCache {
    /// Keyed by BLAKE3 hash of the raw content string.
    entries: HashMap<[u8; 32], ParsedMarkdown>,
}

impl MarkdownCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached parsed result for `content`, or `None` if it has
    /// not been primed yet.
    ///
    /// This hashes `content` on every call (O(content_bytes)).  Prefer
    /// [`MarkdownCache::get_by_key`] when a precomputed key is available
    /// (e.g. stored on the scene node at commit time) to keep lookups O(1).
    pub fn get(&self, content: &str) -> Option<&ParsedMarkdown> {
        let key = blake3::hash(content.as_bytes());
        self.entries.get(key.as_bytes())
    }

    /// Return the cached parsed result for a precomputed BLAKE3 key, or
    /// `None` if it has not been primed yet.
    ///
    /// This is a true O(32-byte) lookup with zero hashing cost.  Use this
    /// on the frame path: compute the key once at content-commit time, store
    /// it on the scene node, and pass it here every frame.
    #[inline]
    pub fn get_by_key(&self, key: &[u8; 32]) -> Option<&ParsedMarkdown> {
        self.entries.get(key)
    }

    /// Compute the BLAKE3 content key for `content`.
    ///
    /// Call this once at content-commit time and store the key alongside the
    /// node so frame-time lookups can use [`MarkdownCache::get_by_key`].
    #[inline]
    pub fn compute_key(content: &str) -> [u8; 32] {
        *blake3::hash(content.as_bytes()).as_bytes()
    }

    /// Parse and cache the content if it is not already present.
    ///
    /// Returns a reference to the cached result.  Calling this for the same
    /// content string twice is a no-op after the first call.
    ///
    /// This method is called at content-commit time, **not** on the frame path.
    pub fn prime<'a>(&'a mut self, content: &str, tokens: &MarkdownTokens) -> &'a ParsedMarkdown {
        let key = *blake3::hash(content.as_bytes()).as_bytes();
        // Use entry API to avoid double-hashing.
        self.entries
            .entry(key)
            .or_insert_with(|| parse_markdown_subset(content, tokens))
    }

    /// Number of distinct content hashes currently cached.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the cache is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Evict all entries whose raw content is no longer in `live_keys`.
    ///
    /// Each element of `live_keys` should be the BLAKE3 hash of a content
    /// string still referenced by the scene.  This keeps the cache bounded
    /// when scene nodes are removed.
    pub fn evict_except(&mut self, live_keys: &[[u8; 32]]) {
        let keep: std::collections::HashSet<[u8; 32]> = live_keys.iter().copied().collect();
        self.entries.retain(|k, _| keep.contains(k));
    }
}

// ─── parse_markdown_subset ───────────────────────────────────────────────────

/// Parse the Phase-1 CommonMark subset from `content` into a [`ParsedMarkdown`].
///
/// The parser is intentionally simple and allocation-efficient: it walks the
/// input line by line, then processes inline markup within each line.  It does
/// not implement a full CommonMark spec-compliant parser.
///
/// # Excluded constructs
///
/// Tables, images, raw HTML, blockquotes, footnotes, strikethrough, task lists,
/// and autolinks are **not** parsed.  Their literal source text is included in
/// the output verbatim so transcript content is never silently dropped.
pub fn parse_markdown_subset(content: &str, tokens: &MarkdownTokens) -> ParsedMarkdown {
    let mut plain = String::with_capacity(content.len());
    let mut spans: Vec<StyledSpan> = Vec::new();

    let mut in_fenced_block = false;
    let mut fence_char: Option<char> = None;
    let mut fence_marker_len: usize = 0;
    // Track whether the previous output line was non-empty for blank-line
    // separation between blocks.
    let mut prev_was_empty = true;

    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();
    let mut i = 0;

    while i < n {
        let raw = lines[i];

        // ── Fenced code block (``` or ~~~) ───────────────────────────────
        if let Some((ch, len)) = detect_fence_open(raw) {
            if !in_fenced_block {
                in_fenced_block = true;
                fence_char = Some(ch);
                fence_marker_len = len;
                // Emit blank line separator if needed.
                if !prev_was_empty && !plain.is_empty() {
                    plain.push('\n');
                }
                i += 1;
                // Collect fence body until closing fence or EOF.
                while i < n {
                    let body_line = lines[i];
                    if is_fence_close(body_line, ch, len) {
                        in_fenced_block = false;
                        fence_char = None;
                        fence_marker_len = 0;
                        i += 1;
                        break;
                    }
                    // Emit body as monospace styled span.
                    let start = plain.len();
                    plain.push_str(body_line);
                    plain.push('\n');
                    let end = plain.len() - 1; // exclude trailing newline from span
                    if start < end {
                        spans.push(StyledSpan {
                            start_byte: start,
                            end_byte: end,
                            attr: StyleAttr {
                                weight: None,
                                italic: false,
                                monospace: tokens.code_monospace,
                                color: tokens.code_color,
                            },
                        });
                    }
                    prev_was_empty = body_line.is_empty();
                    i += 1;
                }
                continue;
            }
        }

        // ── Indented code block (4-space or tab) ─────────────────────────
        if !in_fenced_block && (raw.starts_with("    ") || raw.starts_with('\t')) {
            let body = if let Some(stripped) = raw.strip_prefix("    ") {
                stripped
            } else {
                &raw[1..]
            };
            if !prev_was_empty && !plain.is_empty() {
                plain.push('\n');
            }
            let start = plain.len();
            plain.push_str(body);
            let end = plain.len();
            if start < end {
                spans.push(StyledSpan {
                    start_byte: start,
                    end_byte: end,
                    attr: StyleAttr {
                        weight: None,
                        italic: false,
                        monospace: tokens.code_monospace,
                        color: tokens.code_color,
                    },
                });
            }
            prev_was_empty = body.is_empty();
            i += 1;
            continue;
        }

        // ── ATX heading (# … ######) ─────────────────────────────────────
        if !in_fenced_block {
            if let Some((level, heading_text)) = parse_atx_heading(raw) {
                if !prev_was_empty && !plain.is_empty() {
                    plain.push('\n');
                }
                let level_idx = (level as usize).saturating_sub(1).min(5);
                let attr = StyleAttr {
                    weight: Some(tokens.heading_weight[level_idx]),
                    italic: false,
                    monospace: false,
                    color: None,
                };
                let start = plain.len();
                let prev_len = spans.len();
                // Process inline markup inside the heading text.
                // base_override propagates heading weight into nested spans.
                let heading_chars: Vec<char> = heading_text.chars().collect();
                process_inline(&heading_chars, &mut plain, &mut spans, tokens, Some(&attr));
                let end = plain.len();
                // Fill only the unstyled *gaps* within [start, end) with the
                // heading base style.  This avoids inserting a wide overlapping
                // span across ranges that already have inner-markup spans.
                if start < end {
                    fill_gaps_with_base(&attr, start, end, prev_len, &mut spans);
                }
                prev_was_empty = false;
                i += 1;
                continue;
            }
        }

        // ── List item (unordered or ordered) ────────────────────────────
        if !in_fenced_block {
            if let Some((indent_spaces, item_text)) = parse_list_item(raw) {
                if !prev_was_empty && !plain.is_empty() {
                    plain.push('\n');
                }
                // Emit indent (2 spaces per level, minimum 0).
                let indent = "  ".repeat((indent_spaces / 2).min(4));
                plain.push_str(&indent);
                plain.push_str("• ");
                let item_chars: Vec<char> = item_text.chars().collect();
                process_inline(&item_chars, &mut plain, &mut spans, tokens, None);
                prev_was_empty = false;
                i += 1;
                continue;
            }
        }

        // ── Blank line ───────────────────────────────────────────────────
        if raw.trim().is_empty() {
            if !prev_was_empty && !plain.is_empty() {
                plain.push('\n');
            }
            prev_was_empty = true;
            i += 1;
            continue;
        }

        // ── Normal paragraph line ────────────────────────────────────────
        if !prev_was_empty && !plain.is_empty() {
            plain.push('\n');
        }
        let line_chars: Vec<char> = raw.chars().collect();
        process_inline(&line_chars, &mut plain, &mut spans, tokens, None);
        prev_was_empty = false;
        i += 1;

        // suppress unused variable warnings for fence variables in non-fenced path
        let _ = fence_char;
        let _ = fence_marker_len;
    }

    // Normalise: sort spans and remove zero-length ones.
    spans.sort_by_key(|s| (s.start_byte, s.end_byte));
    spans.retain(|s| s.start_byte < s.end_byte && s.end_byte <= plain.len());

    ParsedMarkdown {
        plain_text: plain,
        spans,
    }
}

// ─── Inline processing ───────────────────────────────────────────────────────

/// Maximum nesting depth for inline markup recursion.
///
/// Beyond this depth `process_inline_inner` emits remaining characters as
/// literal text rather than recursing, bounding stack consumption to a safe
/// constant regardless of how deeply adversarial input nests brackets or
/// emphasis markers.  A 64 KiB payload with only `[` characters fits ~32k
/// nesting levels; this cap stops recursion well before the ~1 MiB stack
/// limit on the Windows main thread.
const MAX_INLINE_DEPTH: usize = 100;

/// Process inline markdown constructs within a single paragraph/heading/list
/// item `chars` and append the result to `out`, emitting styled spans.
///
/// `base_override` is a style that has already been applied to the containing
/// block (e.g. heading weight); inline markup *adds* to it.
///
/// Callers that start from a `&str` should convert once with
/// `.chars().collect::<Vec<char>>()` and pass the resulting slice.
/// Recursive calls pass sub-slices of the already-collected `Vec<char>`,
/// eliminating the per-call allocation that the original `text: &str`
/// approach incurred.
///
/// This is the public entry point; it delegates to [`process_inline_inner`]
/// with `depth = 0`.
fn process_inline(
    chars: &[char],
    out: &mut String,
    spans: &mut Vec<StyledSpan>,
    tokens: &MarkdownTokens,
    base_override: Option<&StyleAttr>,
) {
    // Build the bracket-match table once for this slice so that the link
    // branch can resolve `[…]` in O(1) instead of scanning to end-of-input
    // per opening bracket.  Total cost: O(chars.len()).  Skip the allocation
    // entirely when there are no brackets on the line.
    let bracket_matches = if chars.contains(&'[') {
        build_bracket_matches(chars)
    } else {
        Vec::new()
    };
    // Build the paren-match table once for this slice so that the link-dest
    // branch can resolve `(…)` in O(1) instead of scanning to end-of-input
    // per opening `(`.  Total cost: O(chars.len()).  Skip the allocation
    // entirely when there are no parens on the line.
    let paren_matches = if chars.contains(&'(') {
        build_paren_matches(chars)
    } else {
        Vec::new()
    };
    process_inline_inner(
        chars,
        out,
        spans,
        tokens,
        base_override,
        0,
        &bracket_matches,
        &paren_matches,
    );
}

/// Inner recursive worker for inline processing.
///
/// `depth` is the current nesting level.  When it reaches [`MAX_INLINE_DEPTH`]
/// all remaining characters are emitted as literals — the parser degrades
/// gracefully instead of overflowing the stack.
///
/// `bracket_matches[i]` is `Some(j)` when `chars[i] == '['` and `j` is the
/// index of the matching `']'` (computed once by the top-level caller).  The
/// slice is valid for the *full* original `chars` only.  Recursive calls
/// operate on sub-slices and pass an empty slice for `bracket_matches`; the
/// callee rebuilds the table on-demand for its sub-slice when `[` characters
/// are present, so nested links are still parsed correctly.
///
/// `paren_matches[i]` is `Some(j)` when `chars[i] == '('` and `j` is the
/// index of the matching `')'` (computed once by the top-level caller).  Same
/// scope rules as `bracket_matches`: recursive sub-slice calls pass `&[]` and
/// the fallback O(n) depth scan is used.
///
/// # Argument count
///
/// The eight parameters are all contextual state that must be threaded through
/// the recursion: chars, output, spans, tokens, base_override, depth, and two
/// precomputed match tables (bracket and paren).  A context struct would add
/// indirection with no readability benefit at this call frequency; the lint is
/// suppressed instead.
#[allow(clippy::too_many_arguments)]
fn process_inline_inner(
    chars: &[char],
    out: &mut String,
    spans: &mut Vec<StyledSpan>,
    tokens: &MarkdownTokens,
    base_override: Option<&StyleAttr>,
    depth: usize,
    bracket_matches: &[Option<usize>],
    paren_matches: &[Option<usize>],
) {
    // Depth cap: emit all remaining characters as literals so adversarial
    // deeply-nested input cannot overflow the stack.
    if depth >= MAX_INLINE_DEPTH {
        for &c in chars {
            out.push(c);
        }
        return;
    }

    let n = chars.len();
    let mut i = 0;

    // Memo of emphasis-close failures, keyed by (marker, count).  A scan for a
    // closing run starting at `from` searches a *suffix*; if it fails, every
    // later start (a shorter suffix) for the same (marker, count) also fails.
    // We record the lowest `from` known to have no close and short-circuit
    // future scans, turning the otherwise-O(n²) marker flood (`*`×65535) into
    // amortized O(n).  This is a pure short-circuit — it never changes which
    // emphasis spans match.
    let mut emphasis_close_fail = EmphasisCloseMemo::new();
    // Memo of backtick-close failures, keyed by tick_count.  Same principle as
    // EmphasisCloseMemo: if no closing run of length `tick_count` exists from
    // position `from`, no later position can succeed either.  Converts the
    // `` ` ``×65534 flood (O(n²)) into amortized O(n).
    let mut backtick_close_fail = BacktickCloseMemo::new();

    while i < n {
        let ch = chars[i];

        // ── Images: `![alt](url)` → excluded, render full literal source ──
        // Per contract: excluded constructs are never silently dropped or
        // transformed — emit the verbatim source substring so agents can
        // see exactly what was in the input.
        if ch == '!' && i + 1 < n && chars[i + 1] == '[' {
            if let Some(end) =
                find_link_end_with_table(chars, i + 1, bracket_matches, paren_matches)
            {
                // Emit the full `![alt](url)` construct verbatim.
                for &c in &chars[i..=end] {
                    out.push(c);
                }
                i = end + 1;
                continue;
            }
            // No matching `](...)` — emit the `!` literally and let the `[`
            // branch handle the rest on the next iteration.
            out.push(ch);
            i += 1;
            continue;
        }

        // ── Links: `[text](url)` → styled text, no navigation ────────────
        if ch == '[' {
            // Use the precomputed bracket-match table for O(1) lookup.
            let bracket_close = if i < bracket_matches.len() {
                bracket_matches[i]
            } else {
                None
            };
            if let Some(bracket_close) = bracket_close {
                if bracket_close + 1 < n && chars[bracket_close + 1] == '(' {
                    if let Some(paren_close) =
                        find_paren_close(chars, bracket_close + 1, paren_matches)
                    {
                        let start = out.len();
                        let prev_len = spans.len();
                        // Build a bracket-match table for the sub-slice so nested
                        // links inside link text (e.g. `**[x](u)**`) are parsed
                        // correctly by the recursive call.
                        let inner_slice = &chars[i + 1..bracket_close];
                        let inner_bm = if inner_slice.contains(&'[') {
                            build_bracket_matches(inner_slice)
                        } else {
                            Vec::new()
                        };
                        let mut link_attr = base_override.cloned().unwrap_or_else(StyleAttr::plain);
                        if tokens.link_color.is_some() {
                            link_attr.color = tokens.link_color;
                        }
                        process_inline_inner(
                            inner_slice,
                            out,
                            spans,
                            tokens,
                            Some(&link_attr),
                            depth + 1,
                            &inner_bm,
                            &[], // paren_matches is for the full line; sub-slice uses fallback
                        );
                        let end = out.len();
                        // Use fill_gaps_with_base to cover unstyled gaps with the
                        // link style, keeping spans non-overlapping.
                        if start < end {
                            fill_gaps_with_base(&link_attr, start, end, prev_len, spans);
                        }
                        i = paren_close + 1;
                        continue;
                    }
                }
            }
            // No match → emit literally.
            out.push(ch);
            i += 1;
            continue;
        }

        // ── Inline code: `code` ──────────────────────────────────────────
        if ch == '`' {
            // Count opening backtick run.
            let tick_start = i;
            let mut tick_count = 1;
            while i + tick_count < n && chars[i + tick_count] == '`' {
                tick_count += 1;
            }
            // Find matching closing run via memo to avoid O(n²) on backtick floods.
            if let Some(close_start) =
                backtick_close_fail.find(chars, tick_start + tick_count, tick_count)
            {
                let start = out.len();
                for &c in &chars[tick_start + tick_count..close_start] {
                    out.push(c);
                }
                let end = out.len();
                if start < end {
                    spans.push(StyledSpan {
                        start_byte: start,
                        end_byte: end,
                        attr: StyleAttr {
                            weight: base_override.and_then(|b| b.weight),
                            italic: false,
                            monospace: tokens.code_monospace,
                            color: tokens
                                .code_color
                                .or_else(|| base_override.and_then(|b| b.color)),
                        },
                    });
                }
                i = close_start + tick_count;
                continue;
            }
            // No closing backtick — emit the leading backtick literally.
            out.push(ch);
            i += 1;
            continue;
        }

        // ── Bold+italic: ***text*** or ___text___ ────────────────────────
        if (ch == '*' || ch == '_') && i + 2 < n && chars[i + 1] == ch && chars[i + 2] == ch {
            let marker = ch;
            let open_end = i + 3;
            if let Some(close_start) = emphasis_close_fail.find(chars, open_end, marker, 3) {
                let start = out.len();
                let prev_len = spans.len();
                let inner_attr = StyleAttr {
                    weight: Some(700),
                    italic: true,
                    monospace: base_override.map(|b| b.monospace).unwrap_or(false),
                    color: base_override.and_then(|b| b.color),
                };
                process_inline_inner(
                    &chars[open_end..close_start],
                    out,
                    spans,
                    tokens,
                    Some(&inner_attr),
                    depth + 1,
                    &[],
                    &[],
                );
                let end = out.len();
                // Fill only unstyled gaps to avoid overlapping spans.
                if start < end {
                    fill_gaps_with_base(&inner_attr, start, end, prev_len, spans);
                }
                i = close_start + 3;
                continue;
            }
        }

        // ── Bold: **text** or __text__ ───────────────────────────────────
        if (ch == '*' || ch == '_') && i + 1 < n && chars[i + 1] == ch {
            let marker = ch;
            let open_end = i + 2;
            if let Some(close_start) = emphasis_close_fail.find(chars, open_end, marker, 2) {
                let start = out.len();
                let prev_len = spans.len();
                let base_weight = base_override.and_then(|b| b.weight).unwrap_or(400);
                let bold_attr = StyleAttr {
                    weight: Some(base_weight.max(700)),
                    italic: base_override.map(|b| b.italic).unwrap_or(false),
                    monospace: base_override.map(|b| b.monospace).unwrap_or(false),
                    color: base_override.and_then(|b| b.color),
                };
                process_inline_inner(
                    &chars[open_end..close_start],
                    out,
                    spans,
                    tokens,
                    Some(&bold_attr),
                    depth + 1,
                    &[],
                    &[],
                );
                let end = out.len();
                // Fill only unstyled gaps to avoid overlapping spans.
                if start < end {
                    fill_gaps_with_base(&bold_attr, start, end, prev_len, spans);
                }
                i = close_start + 2;
                continue;
            }
        }

        // ── Italic: *text* or _text_ ─────────────────────────────────────
        if ch == '*' || ch == '_' {
            let marker = ch;
            let open_end = i + 1;
            // Avoid matching lone _ in the middle of a word for `_`.
            let is_word_boundary = if marker == '_' {
                // Simple heuristic: only open if preceded by space/SOL or followed by non-space.
                i == 0 || chars[i - 1].is_whitespace() || chars[i - 1].is_ascii_punctuation()
            } else {
                true
            };
            if is_word_boundary {
                if let Some(close_start) = emphasis_close_fail.find(chars, open_end, marker, 1) {
                    let start = out.len();
                    let prev_len = spans.len();
                    let italic_attr = StyleAttr {
                        weight: base_override.and_then(|b| b.weight),
                        italic: true,
                        monospace: base_override.map(|b| b.monospace).unwrap_or(false),
                        color: base_override.and_then(|b| b.color),
                    };
                    process_inline_inner(
                        &chars[open_end..close_start],
                        out,
                        spans,
                        tokens,
                        Some(&italic_attr),
                        depth + 1,
                        &[],
                        &[],
                    );
                    let end = out.len();
                    // Fill only unstyled gaps to avoid overlapping spans.
                    if start < end {
                        fill_gaps_with_base(&italic_attr, start, end, prev_len, spans);
                    }
                    i = close_start + 1;
                    continue;
                }
            }
        }

        // ── Everything else — emit literally ─────────────────────────────
        out.push(ch);
        i += 1;
    }
}

// ─── Span helpers ─────────────────────────────────────────────────────────────

/// Fill the unstyled byte *gaps* within `[block_start, block_end)` with
/// `base_attr`, appending the new spans to `spans`.
///
/// This is the non-overlapping alternative to inserting a single wide "base"
/// span that would overlap all inner spans.  The caller must have already
/// emitted all inner spans for the block via [`process_inline_inner`]; those
/// spans begin at index `prev_len` in `spans`.
///
/// `prev_len` is the length of `spans` **before** the inner call; this
/// confines the scan to `spans[prev_len..]`, making the function O(new_spans)
/// rather than O(all_spans), which is critical for span-dense adversarial
/// content.
fn fill_gaps_with_base(
    base_attr: &StyleAttr,
    block_start: usize,
    block_end: usize,
    prev_len: usize,
    spans: &mut Vec<StyledSpan>,
) {
    // Collect byte ranges from only the spans added during the inner call.
    // This is O(new_spans) regardless of total spans in the vec.
    // Nested constructs (e.g. links containing emphasis) can push spans in
    // non-start-order; sort so the gap-fill loop is correct.
    let mut inner_ranges: Vec<(usize, usize)> = spans[prev_len..]
        .iter()
        .filter(|s| s.start_byte >= block_start && s.end_byte <= block_end)
        .map(|s| (s.start_byte, s.end_byte))
        .collect();
    inner_ranges.sort_unstable_by_key(|&(s, _)| s);

    // Walk the block range and emit a base-style span for each gap not covered
    // by an inner span.
    let mut cursor = block_start;
    for (s, e) in &inner_ranges {
        if cursor < *s {
            spans.push(StyledSpan {
                start_byte: cursor,
                end_byte: *s,
                attr: base_attr.clone(),
            });
        }
        cursor = cursor.max(*e);
    }
    // Trailing gap.
    if cursor < block_end {
        spans.push(StyledSpan {
            start_byte: cursor,
            end_byte: block_end,
            attr: base_attr.clone(),
        });
    }
}

// ─── Parser helpers ──────────────────────────────────────────────────────────

/// Detect an ATX heading opening (`# ` through `###### `).
/// Returns `(level, heading_text)` or `None`.
fn parse_atx_heading(line: &str) -> Option<(u8, &str)> {
    if !line.starts_with('#') {
        return None;
    }
    let mut level = 0u8;
    let mut rest = line;
    while rest.starts_with('#') && level < 6 {
        level += 1;
        rest = &rest[1..];
    }
    if level == 0 {
        return None;
    }
    // Heading must be followed by a space or be empty.
    if rest.is_empty() {
        return Some((level, ""));
    }
    if let Some(stripped) = rest.strip_prefix(' ') {
        let text = stripped.trim_end_matches('#').trim_end();
        return Some((level, text));
    }
    None
}

/// Detect a list item (ordered or unordered, with leading indent).
/// Returns `(indent_spaces, item_text)` or `None`.
fn parse_list_item(line: &str) -> Option<(usize, &str)> {
    // Count leading spaces.
    let trimmed_start = line.len() - line.trim_start().len();
    let rest = line.trim_start();

    // Unordered: starts with `- `, `* `, or `+ `.
    // `&rest[2..]` is safe even when `rest.len() == 2` (returns "").
    if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        return Some((trimmed_start, &rest[2..]));
    }

    // Ordered: starts with digits followed by `.` or `)` and a space.
    let mut digit_count = 0;
    for ch in rest.chars() {
        if ch.is_ascii_digit() {
            digit_count += 1;
        } else {
            break;
        }
    }
    if digit_count > 0 && digit_count <= 9 {
        let after_digits = &rest[digit_count..];
        if after_digits.starts_with(". ") || after_digits.starts_with(") ") {
            return Some((trimmed_start, &after_digits[2..]));
        }
    }

    None
}

/// Detect a fenced code block opening line (```` ``` ```` or `~~~`).
/// Returns `(fence_char, fence_len)` or `None`.
fn detect_fence_open(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        let count = trimmed.chars().take_while(|&c| c == '`').count();
        if count >= 3 {
            return Some(('`', count));
        }
    }
    if trimmed.starts_with("~~~") {
        let count = trimmed.chars().take_while(|&c| c == '~').count();
        if count >= 3 {
            return Some(('~', count));
        }
    }
    None
}

/// Return `true` if `line` is a closing fence for a block opened with `(fence_char, fence_len)`.
fn is_fence_close(line: &str, fence_char: char, fence_len: usize) -> bool {
    let trimmed = line.trim();
    let count = trimmed.chars().take_while(|&c| c == fence_char).count();
    count >= fence_len && trimmed.chars().skip(count).all(|c| c.is_whitespace())
}

/// Build a bracket-match table for `chars` in a single O(n) pass.
///
/// Returns a `Vec<Option<usize>>` of length `chars.len()`.  Entry `i` is
/// `Some(j)` when `chars[i] == '['` and `j` is the index of its matching
/// `']'`; all other entries are `None`.  Unmatched `[` (no closing `]`) map
/// to `None`.
///
/// Building this table once per line lets the link branch look up bracket
/// matches in O(1) instead of scanning to end-of-input per opening `[`.
/// Without it, `[`×65535 triggers ~2 × 10⁹ character comparisons; with it,
/// the whole line costs one O(n) pass.
fn build_bracket_matches(chars: &[char]) -> Vec<Option<usize>> {
    // Fast path: no `[` means no bracket pairs — skip the allocation entirely.
    if !chars.contains(&'[') {
        return Vec::new();
    }
    let n = chars.len();
    let mut table = vec![None; n];
    // Stack of open-bracket positions waiting for their closing `]`.
    let mut stack: Vec<usize> = Vec::new();
    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '[' => stack.push(i),
            ']' => {
                if let Some(open) = stack.pop() {
                    table[open] = Some(i);
                }
            }
            _ => {}
        }
    }
    table
}

/// Find the index of the closing `]` for a `[` at `open_pos` in `chars`.
///
/// This is the fallback O(n) scan used when no precomputed table is available
/// (e.g. for image constructs whose sub-slice does not carry a table).
fn find_bracket_close(chars: &[char], open_pos: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &ch) in chars.iter().enumerate().skip(open_pos) {
        if ch == '[' {
            depth += 1;
        } else if ch == ']' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Build a paren-match table for `chars` in a single O(n) pass.
///
/// Returns a `Vec<Option<usize>>` of length `chars.len()`.  Entry `i` is
/// `Some(j)` when `chars[i] == '('` and `j` is the index of its matching
/// `')'`; all other entries are `None`.  Unmatched `(` (no closing `)`) map
/// to `None`.
///
/// Building this table once per line lets the link branch look up paren
/// matches in O(1) instead of scanning to end-of-input per opening `(`.
/// Without it, `[](`×21845 triggers ~10⁹ character comparisons; with it,
/// the whole line costs one O(n) pass.
fn build_paren_matches(chars: &[char]) -> Vec<Option<usize>> {
    // Fast path: no `(` means no paren pairs — skip the allocation entirely.
    if !chars.contains(&'(') {
        return Vec::new();
    }
    let n = chars.len();
    let mut table = vec![None; n];
    // Stack of open-paren positions waiting for their closing `)`.
    let mut stack: Vec<usize> = Vec::new();
    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '(' => stack.push(i),
            ')' => {
                if let Some(open) = stack.pop() {
                    table[open] = Some(i);
                }
            }
            _ => {}
        }
    }
    table
}

/// Find the index of the closing `)` for a `(` at `open_pos` in `chars`,
/// using the precomputed paren-match table when available.
///
/// This is the O(1) fast path when `paren_matches` is non-empty.  Falls back
/// to the O(n) stack scan when the table is empty (e.g. sub-slice calls that
/// were not pre-allocated), preserving correctness in all cases.
fn find_paren_close(
    chars: &[char],
    open_pos: usize,
    paren_matches: &[Option<usize>],
) -> Option<usize> {
    if chars.get(open_pos) != Some(&'(') {
        return None;
    }
    // Fast path: table lookup when available.
    if open_pos < paren_matches.len() {
        return paren_matches[open_pos];
    }
    // Fallback: O(n) depth scan (used for sub-slices without a prebuilt table).
    let mut depth = 0usize;
    for (i, &ch) in chars.iter().enumerate().skip(open_pos) {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Find the end of a `![alt](url)` image construct using the precomputed
/// bracket-match and paren-match tables.  Returns the position of the final
/// `)`, or `None`.
///
/// Falls back to [`find_bracket_close`] / O(n) depth scan when the respective
/// table is empty (recursive sub-slice calls), preserving correctness.
fn find_link_end_with_table(
    chars: &[char],
    open_bracket: usize,
    bracket_matches: &[Option<usize>],
    paren_matches: &[Option<usize>],
) -> Option<usize> {
    let bracket_close = if open_bracket < bracket_matches.len() {
        bracket_matches[open_bracket]?
    } else {
        find_bracket_close(chars, open_bracket)?
    };
    if bracket_close + 1 < chars.len() && chars[bracket_close + 1] == '(' {
        find_paren_close(chars, bracket_close + 1, paren_matches)
    } else {
        None
    }
}

/// Find the closing backtick run of `tick_count` backticks starting search at `from`.
///
/// This is the O(n) base scanner.  Prefer calling it through
/// [`BacktickCloseMemo::find`] to get amortized-O(1) failure short-circuiting.
fn find_backtick_close(chars: &[char], from: usize, tick_count: usize) -> Option<usize> {
    let n = chars.len();
    let mut i = from;
    while i + tick_count <= n {
        if chars[i] == '`' {
            let mut run = 0;
            while i + run < n && chars[i + run] == '`' {
                run += 1;
            }
            if run == tick_count {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// Amortized-O(1) failure memo around [`find_backtick_close`].
///
/// A scan for a closing backtick run of length `tick_count` starting at `from`
/// examines the suffix `chars[from..]`.  If that scan fails, any later start
/// `from' >= from` (a shorter suffix) for the same `tick_count` also fails.
/// This memo records, per `tick_count`, the lowest `from` already proven to
/// have no close, and short-circuits future failing scans.
///
/// This converts the pathological `` ` ``×65534 flood — where the original code
/// re-scanned to end-of-input for every backtick position, costing O(n²) —
/// into amortized O(n) total.  It is a pure short-circuit: it never changes
/// which code spans match, only how fast failures are detected.
///
/// Backtick runs of length > `MAX_TICK` are handled by a direct scan (no memo
/// entry).  In practice, CommonMark uses at most a handful of backticks, so
/// this cap is never reached on valid input.
struct BacktickCloseMemo {
    /// `fail_from[tick_count - 1]` is `Some(f)` when no close run of that
    /// length exists for any start `>= f`.  Indexed by `tick_count - 1`.
    fail_from: [Option<usize>; Self::MAX_TICK],
}

impl BacktickCloseMemo {
    /// Maximum tick-count tracked by the memo.  Runs longer than this fall
    /// back to a direct scan.
    const MAX_TICK: usize = 8;

    fn new() -> Self {
        Self {
            fail_from: [None; Self::MAX_TICK],
        }
    }

    /// Find the closing backtick run, consulting and updating the failure memo.
    fn find(&mut self, chars: &[char], from: usize, tick_count: usize) -> Option<usize> {
        if tick_count == 0 || tick_count > Self::MAX_TICK {
            // Unsupported tick count — fall back to a direct scan.
            return find_backtick_close(chars, from, tick_count);
        }
        let col = tick_count - 1;
        // Short-circuit if a previous scan already proved no close from an
        // earlier (or equal) start position.
        if let Some(f) = self.fail_from[col] {
            if from >= f {
                return None;
            }
        }
        match find_backtick_close(chars, from, tick_count) {
            Some(close) => Some(close),
            None => {
                // Record the lowest failing start for this tick_count.
                // Since we short-circuit when from >= f (above), any existing
                // entry must have f > from, so `from` is always the new
                // minimum — a direct assign is correct and cheaper.
                self.fail_from[col] = Some(from);
                None
            }
        }
    }
}

/// Amortized-O(1) failure memo around [`find_emphasis_close`].
///
/// A scan for a closing emphasis run starting at `from` examines the *suffix*
/// `chars[from..]`.  If that scan finds no close, then any later start
/// `from' >= from` (a shorter suffix) with the same `(marker, count)` also has
/// no close.  This memo records, per `(marker, count)`, the lowest `from`
/// already proven to have no close, and short-circuits future failing scans.
///
/// This converts the pathological `*`×65535 / `_`×65535 marker flood — where
/// the original code re-scanned to end-of-input for every marker position,
/// costing O(n²) — into amortized O(n) total.  It is a pure short-circuit: it
/// never changes which emphasis spans match, only how fast failures are
/// detected.
///
/// Indexing: marker `*` → row 0, `_` → row 1; `count` 1/2/3 → column 0/1/2.
struct EmphasisCloseMemo {
    /// `fail_from[marker_idx][count_idx]` is `Some(f)` when no close exists for
    /// any start `>= f`.
    fail_from: [[Option<usize>; 3]; 2],
}

impl EmphasisCloseMemo {
    fn new() -> Self {
        Self {
            fail_from: [[None; 3]; 2],
        }
    }

    #[inline]
    fn index(marker: char, count: usize) -> Option<(usize, usize)> {
        let row = match marker {
            '*' => 0,
            '_' => 1,
            _ => return None,
        };
        if !(1..=3).contains(&count) {
            return None;
        }
        Some((row, count - 1))
    }

    /// Find the closing run, consulting and updating the failure memo.
    fn find(&mut self, chars: &[char], from: usize, marker: char, count: usize) -> Option<usize> {
        if let Some((row, col)) = Self::index(marker, count) {
            // Short-circuit if a previous scan already proved no close from an
            // earlier (or equal) start position.
            if let Some(f) = self.fail_from[row][col] {
                if from >= f {
                    return None;
                }
            }
            match find_emphasis_close(chars, from, marker, count) {
                Some(close) => Some(close),
                None => {
                    // Record the lowest failing start for this (marker, count).
                    // Since we already short-circuit when from >= f (above), any
                    // existing entry must have f > from, so `from` is always the
                    // new minimum — a direct assign is correct and cheaper.
                    self.fail_from[row][col] = Some(from);
                    None
                }
            }
        } else {
            // Unsupported marker/count — fall back to a direct scan.
            find_emphasis_close(chars, from, marker, count)
        }
    }
}

/// Find the closing emphasis marker (single char `marker` repeated `count` times)
/// starting search at `from`.  Returns the start of the closing marker.
fn find_emphasis_close(chars: &[char], from: usize, marker: char, count: usize) -> Option<usize> {
    let n = chars.len();
    let mut i = from;
    while i + count <= n {
        if chars[i] == marker {
            // Check if exactly `count` markers here (not more).
            let mut run = 0;
            while i + run < n && chars[i + run] == marker {
                run += 1;
            }
            if run == count {
                // For `_`, require word boundary on close side.
                let at_boundary = if marker == '_' {
                    i == 0
                        || chars[i - 1] != ' '
                        || (i + count < n && chars[i + count].is_whitespace())
                } else {
                    true
                };
                if at_boundary {
                    return Some(i);
                }
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

// ─── Color helper ─────────────────────────────────────────────────────────────

/// Convert an sRGB u8 channel value [0, 255] to linear f32 [0.0, 1.0].
///
/// Uses the IEC 61966-2-1 piecewise formula (same standard as wgpu's sRGB conversion).
fn srgb_u8_to_linear(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.040_45 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// Parse a hex color string (`#RGB`, `#RRGGBB`, `#RRGGBBAA`) into a linear-light [`Rgba`].
///
/// Hex values are interpreted as sRGB; the result is converted to linear for
/// use with glyphon and wgpu.  Returns `None` if the string does not match
/// any supported format.
fn parse_hex_color(s: &str) -> Option<Rgba> {
    let s = s.trim().strip_prefix('#')?;
    // Guard: non-ASCII bytes would make byte-index slicing below unsafe.
    if !s.is_ascii() {
        return None;
    }
    match s.len() {
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).ok()?;
            let g = u8::from_str_radix(&s[1..2], 16).ok()?;
            let b = u8::from_str_radix(&s[2..3], 16).ok()?;
            Some(Rgba::new(
                srgb_u8_to_linear(r * 17),
                srgb_u8_to_linear(g * 17),
                srgb_u8_to_linear(b * 17),
                1.0,
            ))
        }
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some(Rgba::new(
                srgb_u8_to_linear(r),
                srgb_u8_to_linear(g),
                srgb_u8_to_linear(b),
                1.0,
            ))
        }
        8 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            let a = u8::from_str_radix(&s[6..8], 16).ok()?;
            Some(Rgba::new(
                srgb_u8_to_linear(r),
                srgb_u8_to_linear(g),
                srgb_u8_to_linear(b),
                a as f32 / 255.0,
            ))
        }
        _ => None,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> MarkdownTokens {
        MarkdownTokens::default()
    }

    fn parse(s: &str) -> ParsedMarkdown {
        parse_markdown_subset(s, &tokens())
    }

    // ── Task 2.4 — Subset construct tests ─────────────────────────────────────

    /// H1 heading strips `#` marker and applies bold weight.
    #[test]
    fn heading_h1_stripped_and_styled() {
        let md = parse("# Hello World");
        assert_eq!(
            md.plain_text, "Hello World",
            "H1 heading must strip # prefix"
        );
        assert!(
            md.spans.iter().any(|s| s.attr.weight == Some(700)
                && md.plain_text[s.start_byte..s.end_byte].contains("Hello")),
            "H1 must have weight=700 span"
        );
    }

    /// H2–H6 headings each get the correct level-specific weight from tokens.
    #[test]
    fn headings_h1_through_h6_have_correct_weights() {
        for level in 1u8..=6 {
            let marker = "#".repeat(level as usize);
            let input = format!("{marker} Text");
            let md = parse(&input);
            assert_eq!(md.plain_text, "Text", "level {level}: expected 'Text'");
            let expected_weight = MarkdownTokens::default().heading_weight[(level - 1) as usize];
            assert!(
                md.spans
                    .iter()
                    .any(|s| s.attr.weight == Some(expected_weight)),
                "H{level} must carry weight={expected_weight}"
            );
        }
    }

    /// Strong (`**text**`) renders as bold.
    #[test]
    fn strong_renders_bold() {
        let md = parse("Hello **world**!");
        assert_eq!(md.plain_text, "Hello world!");
        let bold_span = md.spans.iter().find(|s| s.attr.weight == Some(700));
        assert!(bold_span.is_some(), "strong must produce weight=700 span");
        let span = bold_span.unwrap();
        assert_eq!(&md.plain_text[span.start_byte..span.end_byte], "world");
    }

    /// Emphasis (`*text*`) renders as italic.
    #[test]
    fn emphasis_renders_italic() {
        let md = parse("Hello *world*!");
        assert_eq!(md.plain_text, "Hello world!");
        let italic_span = md.spans.iter().find(|s| s.attr.italic);
        assert!(italic_span.is_some(), "emphasis must produce italic span");
        let span = italic_span.unwrap();
        assert_eq!(&md.plain_text[span.start_byte..span.end_byte], "world");
    }

    /// Bold+italic (`***text***`) renders as both bold and italic.
    #[test]
    fn bold_italic_renders_both() {
        let md = parse("***bold-italic***");
        assert_eq!(md.plain_text, "bold-italic");
        assert!(
            md.spans
                .iter()
                .any(|s| s.attr.weight == Some(700) && s.attr.italic),
            "bold-italic must produce weight=700 and italic=true"
        );
    }

    /// Inline code (`\`code\``) uses monospace family.
    #[test]
    fn inline_code_uses_monospace() {
        let md = parse("Use `fmt::Display` here.");
        assert_eq!(md.plain_text, "Use fmt::Display here.");
        let code_span = md.spans.iter().find(|s| s.attr.monospace);
        assert!(
            code_span.is_some(),
            "inline code must produce monospace span"
        );
        let span = code_span.unwrap();
        assert_eq!(
            &md.plain_text[span.start_byte..span.end_byte],
            "fmt::Display"
        );
    }

    /// Fenced code block (``` ``` ```) renders body as monospace.
    #[test]
    fn fenced_code_block_body_is_monospace() {
        let input = "```\nlet x = 1;\nprintln!(\"{x}\");\n```";
        let md = parse(input);
        assert!(
            md.plain_text.contains("let x = 1;"),
            "fenced block body must be in output"
        );
        assert!(
            md.plain_text.contains("println!"),
            "fenced block body second line must be in output"
        );
        assert!(
            md.spans.iter().any(|s| s.attr.monospace),
            "fenced block must produce monospace spans"
        );
    }

    /// Indented code block (4-space indent) renders as monospace.
    #[test]
    fn indented_code_block_is_monospace() {
        let input = "    fn hello() {}";
        let md = parse(input);
        assert_eq!(md.plain_text, "fn hello() {}");
        assert!(
            md.spans.iter().any(|s| s.attr.monospace),
            "indented block must produce monospace span"
        );
    }

    /// Unordered list items (`- item`) render with bullet prefix.
    #[test]
    fn unordered_list_items_have_bullet() {
        let input = "- alpha\n- beta\n- gamma";
        let md = parse(input);
        assert!(
            md.plain_text.contains("• alpha"),
            "list item must have bullet; got: {:?}",
            md.plain_text
        );
        assert!(md.plain_text.contains("• beta"));
        assert!(md.plain_text.contains("• gamma"));
    }

    /// Ordered list items (`1. item`) render with bullet prefix.
    #[test]
    fn ordered_list_items_have_bullet() {
        let input = "1. first\n2. second";
        let md = parse(input);
        assert!(
            md.plain_text.contains("• first"),
            "ordered list item must have bullet"
        );
        assert!(md.plain_text.contains("• second"));
    }

    /// Link `[text](url)` renders as styled text; destination is omitted.
    #[test]
    fn link_renders_text_not_url() {
        let md = parse("[release notes](https://example.com)");
        assert_eq!(
            md.plain_text, "release notes",
            "link must render only link text"
        );
        assert!(
            !md.plain_text.contains("example.com"),
            "link destination must not appear in output"
        );
    }

    /// Link text styled with token link color if present.
    #[test]
    fn link_styled_with_token_color() {
        let t = MarkdownTokens {
            link_color: Some(Rgba::new(0.0, 0.5, 1.0, 1.0)),
            ..MarkdownTokens::default()
        };
        let md = parse_markdown_subset("[click here](https://x.com)", &t);
        assert_eq!(md.plain_text, "click here");
        let link_span = md.spans.iter().find(|s| s.attr.color.is_some());
        assert!(
            link_span.is_some(),
            "link must carry color override when token set"
        );
    }

    // ── Task 2.4 — Excluded construct tests ───────────────────────────────────

    /// Markdown table renders as literal source text (not parsed).
    #[test]
    fn excluded_table_renders_literal() {
        let input = "| A | B |\n|---|---|\n| 1 | 2 |";
        let md = parse(input);
        // The table syntax should appear verbatim in the output.
        assert!(
            md.plain_text.contains("|"),
            "table pipes must appear literally"
        );
        assert!(
            md.plain_text.contains("A"),
            "table content must appear literally"
        );
    }

    /// Image `![alt](url)` renders as its full literal source — not dropped, not transformed.
    ///
    /// Per the excluded-construct contract: the verbatim source substring is
    /// emitted so no content is silently lost.
    #[test]
    fn excluded_image_renders_literal_source() {
        let md = parse("![diagram](img.png)");
        // The full source must appear verbatim — not silently dropped.
        assert!(
            !md.plain_text.is_empty(),
            "image construct must not be silently dropped"
        );
        assert_eq!(
            md.plain_text, "![diagram](img.png)",
            "image must render as verbatim source; got: {:?}",
            md.plain_text
        );
    }

    /// Raw HTML is rendered literally (not parsed or dropped).
    #[test]
    fn excluded_raw_html_renders_literally() {
        let input = "<strong>bold</strong>";
        let md = parse(input);
        // Raw HTML angle brackets should appear literally.
        assert!(
            md.plain_text.contains('<'),
            "raw HTML must not be dropped; got: {:?}",
            md.plain_text
        );
    }

    /// Blockquote (`> text`) renders as literal text (not styled as a blockquote).
    #[test]
    fn excluded_blockquote_renders_literally() {
        let input = "> This is a blockquote";
        let md = parse(input);
        assert!(
            md.plain_text.contains('>'),
            "blockquote marker must appear literally"
        );
        assert!(
            md.plain_text.contains("This is a blockquote"),
            "blockquote content must not be dropped"
        );
    }

    /// Strikethrough (`~~text~~`) renders as literal text.
    #[test]
    fn excluded_strikethrough_renders_literally() {
        let input = "~~crossed out~~";
        let md = parse(input);
        assert!(
            md.plain_text.contains("crossed out"),
            "strikethrough content must appear literally"
        );
        assert!(
            md.plain_text.contains("~~"),
            "strikethrough markers must appear literally (not parsed)"
        );
    }

    /// Task list (`- [ ] item`) renders as literal text (not a checkbox widget).
    #[test]
    fn excluded_task_list_renders_literally() {
        let input = "- [ ] todo item";
        let md = parse(input);
        // The checkbox syntax [ ] should remain visible in some form.
        // (May render as bullet item; the key property is content not dropped.)
        assert!(
            md.plain_text.contains("todo item"),
            "task list content must not be dropped"
        );
    }

    /// Link non-navigability: link text renders, no URL in output, and the
    /// span carries no href or navigation attribute.
    #[test]
    fn link_not_navigable_no_url_in_output() {
        let md = parse("[RFC 0001](https://internal.example/rfc/0001)");
        assert_eq!(md.plain_text, "RFC 0001");
        assert!(!md.plain_text.contains("http"), "URL must not appear");
        // No span should carry a URL (StyledSpan has no href field — this is structural).
    }

    // ── Task 2.5 — Zero per-frame parse cost ──────────────────────────────────

    /// Cache hit for the same content string is guaranteed after prime().
    #[test]
    fn cache_hit_after_prime() {
        let mut cache = MarkdownCache::new();
        let t = tokens();
        let content = "# Hello\n**world**";

        // First call primes the cache.
        let first = cache.prime(content, &t).clone();
        // Second call must hit the cache (no re-parse — same result).
        let second = cache.prime(content, &t).clone();

        assert_eq!(
            first, second,
            "cached result must be identical to parsed result"
        );
        assert_eq!(cache.len(), 1, "only one cache entry for the same content");
    }

    /// Two different content strings produce separate cache entries.
    #[test]
    fn different_content_different_cache_entries() {
        let mut cache = MarkdownCache::new();
        let t = tokens();

        cache.prime("# Hello", &t);
        cache.prime("# World", &t);

        assert_eq!(cache.len(), 2, "two distinct content strings → two entries");
    }

    /// `get` returns None for content not yet primed.
    #[test]
    fn cache_miss_before_prime() {
        let cache = MarkdownCache::new();
        assert!(
            cache.get("unparsed content").is_none(),
            "cache must be empty before prime"
        );
    }

    /// Large payload (65535 bytes) primes without panic.
    #[test]
    fn large_payload_65535_bytes_primes_without_panic() {
        let content = "a".repeat(65535);
        let mut cache = MarkdownCache::new();
        let _ = cache.prime(&content, &tokens());
        assert_eq!(cache.len(), 1);
    }

    /// `evict_except` removes stale entries.
    #[test]
    fn evict_removes_stale_entries() {
        let mut cache = MarkdownCache::new();
        let t = tokens();

        let content_a = "# Keep";
        let content_b = "# Evict";
        cache.prime(content_a, &t);
        cache.prime(content_b, &t);
        assert_eq!(cache.len(), 2);

        let keep_key = *blake3::hash(content_a.as_bytes()).as_bytes();
        cache.evict_except(&[keep_key]);

        assert_eq!(cache.len(), 1, "evict_except must remove stale entry");
        assert!(
            cache.get(content_a).is_some(),
            "kept entry must remain after eviction"
        );
        assert!(cache.get(content_b).is_none(), "evicted entry must be gone");
    }

    // ── MarkdownTokens tests ───────────────────────────────────────────────────

    /// Empty token map produces defaults.
    #[test]
    fn empty_token_map_gives_defaults() {
        let map = HashMap::new();
        let t = MarkdownTokens::from_token_map(&map);
        let d = MarkdownTokens::default();
        assert_eq!(t.heading_weight, d.heading_weight);
        assert_eq!(t.heading_scale, d.heading_scale);
        assert!(t.link_color.is_none());
        assert!(t.code_monospace); // defaults to true
    }

    /// Token map with heading weight overrides.
    #[test]
    fn token_map_heading_weight_overrides() {
        let mut map = HashMap::new();
        map.insert("typography.heading.1.weight".to_string(), "900".to_string());
        map.insert("typography.heading.3.weight".to_string(), "500".to_string());
        let t = MarkdownTokens::from_token_map(&map);
        assert_eq!(t.heading_weight[0], 900, "H1 weight should be overridden");
        assert_eq!(t.heading_weight[2], 500, "H3 weight should be overridden");
        // Unset levels use defaults.
        assert_eq!(
            t.heading_weight[1],
            MarkdownTokens::default().heading_weight[1]
        );
    }

    /// Token map with link color override.
    #[test]
    fn token_map_link_color_override() {
        let mut map = HashMap::new();
        map.insert("color.link.text".to_string(), "#0066FF".to_string());
        let t = MarkdownTokens::from_token_map(&map);
        assert!(t.link_color.is_some(), "link color must be set from token");
        let c = t.link_color.unwrap();
        // #0066FF sRGB → r≈0, g≈0.14, b≈1.0 after gamma conversion — just
        // check that blue dominates.
        assert!(c.b > c.r, "blue must dominate for #0066FF");
    }

    /// `parse_hex_color` handles #RGB, #RRGGBB, and #RRGGBBAA.
    #[test]
    fn parse_hex_color_formats() {
        let white6 = parse_hex_color("#FFFFFF").expect("#RRGGBB");
        assert_eq!(white6.a, 1.0);

        let black3 = parse_hex_color("#000").expect("#RGB");
        assert_eq!(black3.r, 0.0);

        let semi = parse_hex_color("#FFFFFF80").expect("#RRGGBBAA");
        assert!(semi.a > 0.4 && semi.a < 0.6, "alpha ~0.5 from 0x80");
    }

    // ── Plain text and mixed content ──────────────────────────────────────────

    /// Plain text (no markdown) passes through unchanged.
    #[test]
    fn plain_text_passes_through() {
        let md = parse("Hello, world!");
        assert_eq!(md.plain_text, "Hello, world!");
        assert!(
            md.spans.is_empty(),
            "plain text must produce no styled spans"
        );
    }

    /// Empty input produces empty output.
    #[test]
    fn empty_input_produces_empty_output() {
        let md = parse("");
        assert_eq!(md.plain_text, "");
        assert!(md.spans.is_empty());
    }

    /// Multi-line content with mixed constructs covers all spans.
    #[test]
    fn mixed_content_spans_cover_plain_text() {
        let input = "# Title\n\nHello **world** and *folks*.\n\n- item one\n- item two";
        let md = parse(input);
        // Every styled span must be a valid range in plain_text.
        for span in &md.spans {
            assert!(
                span.start_byte <= span.end_byte,
                "span start must be <= end"
            );
            assert!(
                span.end_byte <= md.plain_text.len(),
                "span end must be <= plain_text.len()"
            );
            assert!(
                md.plain_text.is_char_boundary(span.start_byte),
                "span start must be a char boundary"
            );
            assert!(
                md.plain_text.is_char_boundary(span.end_byte),
                "span end must be a char boundary"
            );
        }
    }

    /// Multi-byte UTF-8 content (emoji, CJK) does not panic and produces valid boundaries.
    #[test]
    fn multibyte_utf8_content_valid_boundaries() {
        let input = "# 日本語タイトル\n\n**太字** と *斜体* テキスト。\n\n`コード`";
        let md = parse(input);
        for span in &md.spans {
            assert!(
                md.plain_text.is_char_boundary(span.start_byte),
                "start_byte must be char boundary"
            );
            assert!(
                md.plain_text.is_char_boundary(span.end_byte),
                "end_byte must be char boundary"
            );
        }
    }

    // ── Adversarial / DoS-resistance tests ────────────────────────────────────

    /// 65535 unmatched `[` characters complete in bounded time without stack
    /// overflow and without stalling the compositor.
    ///
    /// This exercises fix (b): the precomputed bracket-match table (`O(n)` single
    /// pass) replaces the original per-`[` end-of-input scan that was `O(n²)`.
    /// With 65535 brackets the old code performed ~2×10⁹ comparisons; the new
    /// code costs one `O(n)` build pass and `O(1)` lookups thereafter.
    ///
    /// The time assertion is gated by `#[ignore]` because wall-clock thresholds
    /// are not deterministic across CI runners.  Run manually with
    /// `cargo test -- --ignored` to validate timing.  Structural correctness
    /// (no panic, no dropped content) is asserted unconditionally in the
    /// `adversarial_*_no_stack_overflow` / `adversarial_*_completes_fast` tests.
    #[test]
    #[ignore = "wall-clock assertion; run with --ignored to validate timing locally"]
    fn adversarial_flood_of_unmatched_open_brackets_completes_fast() {
        let input = "[".repeat(65535);
        let deadline = std::time::Instant::now();
        let md = parse(&input);
        let elapsed = deadline.elapsed();
        // All characters must appear in the output (no content silently dropped).
        assert_eq!(
            md.plain_text.len(),
            65535,
            "all '[' must appear in plain text output"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "65535 unmatched '[' must complete in <5s (debug build); took {elapsed:?}"
        );
    }

    /// Deeply-nested bold markers (`**` × 32768 pairs) complete quickly without
    /// stack overflow.
    ///
    /// This exercises fix (a): the recursion depth cap in `process_inline_inner`.
    /// Beyond `MAX_INLINE_DEPTH` (100) the parser emits remaining characters as
    /// literals; this bounds stack consumption to a safe constant regardless of
    /// nesting depth.
    #[test]
    #[ignore = "wall-clock assertion; run with --ignored to validate timing locally"]
    fn adversarial_deeply_nested_bold_no_stack_overflow() {
        // Build "**" × 16384 + "x" + "**" × 16384 — deeply nested bold.
        let mut input = String::with_capacity(65535);
        for _ in 0..16384 {
            input.push_str("**");
        }
        input.push('x');
        for _ in 0..16384 {
            input.push_str("**");
        }
        let deadline = std::time::Instant::now();
        let md = parse(&input);
        let elapsed = deadline.elapsed();
        // Must not panic/overflow and must complete quickly.
        assert!(
            !md.plain_text.is_empty(),
            "deeply nested bold must produce non-empty output"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "deeply nested bold must complete in <5s (debug build); took {elapsed:?}"
        );
    }

    /// Deeply-nested italic markers (`*` × 32767 pairs) complete quickly without
    /// stack overflow.
    #[test]
    #[ignore = "wall-clock assertion; run with --ignored to validate timing locally"]
    fn adversarial_deeply_nested_italic_no_stack_overflow() {
        // Build "*" × 32767 + "x" + "*" × 32767
        let mut input = String::with_capacity(65535);
        for _ in 0..32767 {
            input.push('*');
        }
        input.push('x');
        for _ in 0..32767 {
            input.push('*');
        }
        let deadline = std::time::Instant::now();
        let md = parse(&input);
        let elapsed = deadline.elapsed();
        assert!(
            !md.plain_text.is_empty(),
            "deeply nested italic must produce non-empty output"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "deeply nested italic must complete in <5s (debug build); took {elapsed:?}"
        );
    }

    /// Deeply-nested link brackets (`[` × 32768 pairs) complete quickly without
    /// stack overflow.
    ///
    /// This exercises both fix (a) (depth cap) and fix (b) (bracket-match table).
    #[test]
    #[ignore = "wall-clock assertion; run with --ignored to validate timing locally"]
    fn adversarial_deeply_nested_link_brackets_no_stack_overflow() {
        // Build "[" × 32768 + "text" + "]" × 32768 — deeply nested brackets.
        let n = 32768usize;
        let mut input = String::with_capacity(n * 2 + 4);
        for _ in 0..n {
            input.push('[');
        }
        input.push_str("text");
        for _ in 0..n {
            input.push(']');
        }
        let deadline = std::time::Instant::now();
        let md = parse(&input);
        let elapsed = deadline.elapsed();
        assert!(
            md.plain_text.contains("text"),
            "link bracket flood must not drop inner text"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "deeply nested link brackets must complete in <5s (debug build); took {elapsed:?}"
        );
    }

    /// Span-dense content (many short bold spans) completes in bounded time.
    ///
    /// This exercises fix (c): `fill_gaps_with_base` now scans only the spans
    /// added during the current `process_inline_inner` call (`O(new_spans)`)
    /// rather than the full vec (`O(all_spans)`), eliminating the `O(spans²)`
    /// blowup that would otherwise occur on span-dense content.
    #[test]
    #[ignore = "wall-clock assertion; run with --ignored to validate timing locally"]
    fn adversarial_span_dense_bold_content_completes_fast() {
        // Build a heading with ~1000 alternating bold/plain segments.
        // Each "**x** " adds one styled span; fill_gaps_with_base is called once
        // per block.  Without the O(n) fix this would be O(1000²) operations.
        let segment = "**a** b ";
        let repeat = 1000;
        let body: String = segment.repeat(repeat);
        let input = format!("# {body}");
        let deadline = std::time::Instant::now();
        let md = parse(&input);
        let elapsed = deadline.elapsed();
        assert!(
            !md.plain_text.is_empty(),
            "span-dense heading must produce non-empty output"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "span-dense content must complete in <5s (debug build); took {elapsed:?}"
        );
    }

    /// Full 64 KiB adversarial payload with all three pathological patterns
    /// combined: mixed bracket floods, emphasis nesting, and span density.
    #[test]
    #[ignore = "wall-clock assertion; run with --ignored to validate timing locally"]
    fn adversarial_combined_64kib_completes_fast() {
        // 21845 repetitions of "[**x**] " ≈ 8 bytes each ≈ ~175 KiB; cap at 65535
        let segment = "[**x**] ";
        let n = 65535 / segment.len();
        let input: String = segment.repeat(n);
        let input = &input[..input.len().min(65535)];
        let deadline = std::time::Instant::now();
        let md = parse(input);
        let elapsed = deadline.elapsed();
        assert!(
            !md.plain_text.is_empty(),
            "combined adversarial input must produce non-empty output"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "combined 64KiB adversarial input must complete in <5s (debug build); took {elapsed:?}"
        );
    }

    // ── Paren-close adversarial tests (hud-xq0uo) ────────────────────────────

    /// 21845 repetitions of `[](` complete in bounded time.
    ///
    /// Before hud-xq0uo, `find_paren_close` re-scanned the full suffix for
    /// every unmatched `(`, costing O(n²) — ~969 ms in release on real hardware.
    /// The precomputed `build_paren_matches` table reduces each lookup to O(1),
    /// making the whole line O(n).
    ///
    /// This test is NOT `#[ignore]`-gated: O(n²) would make it hang for
    /// tens of seconds in any build mode; O(n) finishes instantly.
    #[test]
    fn adversarial_paren_flood_link_dest_completes_fast() {
        // "[](": 21845 repetitions ≈ 65535 bytes, all parens unmatched.
        let input = "[](".repeat(21845);
        let md = parse(&input);
        // All content must appear in the output (no silent drops).
        assert!(
            !md.plain_text.is_empty(),
            "paren flood must produce non-empty output"
        );
    }

    /// 16383 repetitions of `[a](` complete in bounded time.
    ///
    /// Variant: link text present (`a`) — exercises the bracket-table lookup
    /// followed by the paren-table lookup.  Empirical: ~963 ms before fix.
    ///
    /// This test is NOT `#[ignore]`-gated: see `adversarial_paren_flood_link_dest_completes_fast`.
    #[test]
    fn adversarial_paren_flood_with_link_text_completes_fast() {
        // "[a](": 16383 repetitions ≈ 65532 bytes.
        let input = "[a](".repeat(16383);
        let md = parse(&input);
        assert!(
            !md.plain_text.is_empty(),
            "paren flood with link text must produce non-empty output"
        );
    }

    /// 13107 repetitions of `![a](` complete in bounded time.
    ///
    /// Variant: image `!` prefix — exercises `find_link_end_with_table` which
    /// also calls `find_paren_close`.  Empirical: ~642 ms before fix.
    ///
    /// This test is NOT `#[ignore]`-gated: see `adversarial_paren_flood_link_dest_completes_fast`.
    #[test]
    fn adversarial_paren_flood_image_construct_completes_fast() {
        // "![a](": 13107 repetitions ≈ 65535 bytes.
        let input = "![a](".repeat(13107);
        let md = parse(&input);
        assert!(
            !md.plain_text.is_empty(),
            "image paren flood must produce non-empty output"
        );
    }

    // ── Backtick-close adversarial tests (hud-xq0uo) ─────────────────────────

    /// 65534 backticks (unmatched runs) complete in bounded time.
    ///
    /// Before hud-xq0uo, `find_backtick_close` re-scanned the full suffix for
    /// every unmatched backtick, costing O(n²) — ~696 ms in release on real
    /// hardware.  The `BacktickCloseMemo` short-circuits repeated failing scans,
    /// making the whole line amortized O(n).
    ///
    /// This test is NOT `#[ignore]`-gated: O(n²) would make it hang for
    /// tens of seconds in any build mode; amortized O(n) finishes instantly.
    ///
    /// Pattern: `a` + `` ` ``×65534 — a single letter followed by a flood of
    /// unmatched backticks.  Each backtick run of length 1 scans the entire
    /// remaining input before failing; without the memo this is O(n²).
    #[test]
    fn adversarial_backtick_flood_completes_fast() {
        // "a" + "`" × 65534: a single non-backtick followed by 65534 bare backticks.
        // No two adjacent backtick runs form a matched pair (they are all
        // adjacent, so only runs of exactly 1 exist everywhere — and there is
        // no non-backtick content between them for a close scan to land on).
        // Actually simpler: a repeated sequence that has no balanced backtick
        // pairs: "a" followed by backticks that are all one big run (no match).
        let mut input = String::with_capacity(65535);
        input.push('a');
        for _ in 0..65534 {
            input.push('`');
        }
        let md = parse(&input);
        // The plain-text output must contain the leading 'a'.
        assert!(
            md.plain_text.contains('a'),
            "backtick flood must not drop literal 'a' character"
        );
        // The output must be non-empty and not panic.
        assert!(
            !md.plain_text.is_empty(),
            "backtick flood must produce non-empty output"
        );
    }

    // ── Paren/backtick semantic correctness tests (hud-xq0uo) ────────────────

    /// Normal link `[text](url)` is correctly parsed after paren table is built.
    ///
    /// Ensures the paren-table lookup does not regress link parsing semantics.
    #[test]
    fn paren_table_link_semantic_correctness() {
        let md = parse("[hello](https://example.com)");
        assert_eq!(
            md.plain_text, "hello",
            "link text must be extracted, URL dropped"
        );
    }

    /// Link with nested parens in URL `[text](url(1))` is handled correctly.
    ///
    /// The paren table uses a depth-matching stack, so nested parens in the
    /// link destination match the outermost `)`.
    #[test]
    fn paren_table_nested_parens_in_url() {
        let md = parse("[doc](fn(arg))");
        // The link text should be emitted; the URL (including inner parens) is dropped.
        assert_eq!(
            md.plain_text, "doc",
            "nested parens in URL must not break link parsing"
        );
    }

    /// Multiple links on one line are all parsed correctly.
    #[test]
    fn paren_table_multiple_links_on_one_line() {
        let md = parse("[a](u1) and [b](u2)");
        assert_eq!(
            md.plain_text, "a and b",
            "multiple links must all be parsed correctly"
        );
    }

    /// Inline code spans are correctly parsed after the backtick memo is built.
    ///
    /// Ensures `BacktickCloseMemo` does not regress code-span parsing semantics.
    #[test]
    fn backtick_memo_inline_code_semantic_correctness() {
        let md = parse("Use `fmt::Display` here.");
        assert_eq!(md.plain_text, "Use fmt::Display here.");
        assert!(
            md.spans.iter().any(|s| s.attr.monospace),
            "inline code must produce a monospace span"
        );
        let span = md.spans.iter().find(|s| s.attr.monospace).unwrap();
        assert_eq!(
            &md.plain_text[span.start_byte..span.end_byte],
            "fmt::Display"
        );
    }

    /// Double-backtick code spans (`` ``code`` ``) are parsed correctly.
    ///
    /// Tests that tick_count=2 memo entries do not interfere with tick_count=1.
    #[test]
    fn backtick_memo_double_tick_span_correctness() {
        let md = parse("Look at ``a`b`` here.");
        assert_eq!(md.plain_text, "Look at a`b here.");
        assert!(
            md.spans.iter().any(|s| s.attr.monospace),
            "double-tick code span must produce a monospace span"
        );
    }

    /// Multiple code spans on one line are all parsed correctly.
    #[test]
    fn backtick_memo_multiple_spans_on_one_line() {
        let md = parse("`a` and `b` and `c`");
        assert_eq!(md.plain_text, "a and b and c");
        assert_eq!(
            md.spans.iter().filter(|s| s.attr.monospace).count(),
            3,
            "three separate code spans must each produce a monospace span"
        );
    }
}
