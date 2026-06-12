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
//! - `Ellipsis` — measured word-boundary truncation with the ellipsis glyph
//!   included in the shaped-width budget, implemented in [`crate::overflow`]
//!   (hud-5jbra.3).  Grapheme-cluster fallback applies when no word boundary
//!   fits.  Whole-line vertical visibility is enforced: no partially clipped
//!   glyph rows.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, Style,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use tze_hud_scene::types::{
    FontFamily, RenderingPolicy, Rgba, TextAlign, TextMarkdownNode, TextOverflow,
};
use wgpu::{Device, MultisampleState, Queue};

use crate::overflow::{self, TruncationResult, TruncationViewport};

// ─── TruncationCache ─────────────────────────────────────────────────────────

/// Cache key for one truncation computation.
///
/// Keyed on `(content_hash, bounds_width, bounds_height, font_size_px,
/// font_family, font_weight, viewport_mode)`.  Float fields are stored as
/// IEEE 754 bit patterns (endian-stable because the process never crosses a
/// machine boundary) so they are `Eq + Hash` without lossy rounding.
///
/// `viewport_mode` distinguishes head-anchored from tail-anchored truncation:
/// the same content at the same geometry produces different display text
/// depending on which anchor mode is active (spec tasks 3.2 and 3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TruncationKey {
    /// BLAKE3 hash of the content string (32 bytes).
    content_hash: [u8; 32],
    /// `bounds_width` as IEEE 754 bits.
    bounds_width_bits: u32,
    /// `bounds_height` as IEEE 754 bits.
    bounds_height_bits: u32,
    /// `font_size_px` as IEEE 754 bits.
    font_size_px_bits: u32,
    /// Font family discriminant (SansSerif=0, Monospace=1, Serif=2).
    font_family: u8,
    /// CSS font weight (100–900).
    font_weight: u16,
    /// Viewport anchor mode: 0 = `HeadAnchored`, 1 = `TailAnchored`.
    ///
    /// Same content at the same geometry produces different truncated text
    /// depending on anchor mode, so it must be part of the cache key.
    viewport_mode: u8,
}

impl TruncationKey {
    fn new(
        content: &str,
        bounds_width: f32,
        bounds_height: f32,
        font_size_px: f32,
        font_family: FontFamily,
        font_weight: u16,
        viewport: TruncationViewport,
    ) -> Self {
        Self {
            content_hash: *blake3::hash(content.as_bytes()).as_bytes(),
            bounds_width_bits: bounds_width.to_bits(),
            bounds_height_bits: bounds_height.to_bits(),
            font_size_px_bits: font_size_px.to_bits(),
            font_family: match font_family {
                FontFamily::SystemSansSerif => 0,
                FontFamily::SystemMonospace => 1,
                FontFamily::SystemSerif => 2,
            },
            font_weight,
            viewport_mode: match viewport {
                TruncationViewport::HeadAnchored => 0,
                TruncationViewport::TailAnchored => 1,
            },
        }
    }
}

/// Content-addressed cache of [`TruncationResult`] values.
///
/// Keyed on `(content_hash, bounds_width, bounds_height, font_size_px,
/// font_family, font_weight)`.  The cache is populated outside the per-frame
/// pipeline by [`TruncationCache::prime`] and looked up in O(1) on the frame
/// path via [`TruncationCache::get_by_key`].
///
/// This enforces the "stable-shape-caching" contract from RFC 0013 §3.4 and
/// §4.2, Phase-1 design §3: truncation is called only when content or geometry
/// changes — never on every frame.
///
/// Mirrors [`crate::markdown::MarkdownCache`] in structure and ownership model.
#[derive(Default)]
pub(crate) struct TruncationCache {
    entries: HashMap<TruncationKey, TruncationResult>,
}

impl TruncationCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Number of cached entries.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no entries are cached.
    #[inline]
    #[allow(dead_code)] // used in tests; companion to len()
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up a cached truncation result.  Returns `None` on a cache miss.
    ///
    /// O(1) hash-map lookup; no shaping, no hashing of the content string.
    /// Callers on the frame path must pre-compute the key via [`TruncationKey::new`]
    /// and call [`TruncationCache::get_by_key`].
    #[inline]
    pub(crate) fn get_by_key(&self, key: &TruncationKey) -> Option<&TruncationResult> {
        self.entries.get(key)
    }

    /// Compute and cache the truncation result if not already present.
    ///
    /// Returns a reference to the cached result.  Calling this twice for the
    /// same key is a no-op after the first call (same content + same geometry +
    /// same viewport mode → same result).
    ///
    /// # Viewport dispatch
    ///
    /// The `viewport` parameter selects the truncation algorithm:
    ///
    /// - [`TruncationViewport::HeadAnchored`] (default) — calls
    ///   [`overflow::truncate_for_ellipsis`].  Used for static tiles and
    ///   scrolled-back viewports (spec task 3.3 append-stability guarantee).
    ///
    /// - [`TruncationViewport::TailAnchored`] — calls
    ///   [`overflow::truncate_tail_anchored`].  Used for follow-tail streaming
    ///   transcripts whose viewport is at the tail (spec task 3.2 whole-line
    ///   advancement guarantee).
    ///
    /// Call this at content-commit time, **not** on the frame path.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prime<'a>(
        &'a mut self,
        key: TruncationKey,
        content: &str,
        bounds_width: f32,
        bounds_height: f32,
        font_size_px: f32,
        font_family: FontFamily,
        font_weight: u16,
        viewport: TruncationViewport,
        font_system: &mut FontSystem,
    ) -> &'a TruncationResult {
        self.entries.entry(key).or_insert_with(|| {
            let family = match font_family {
                FontFamily::SystemSansSerif => Family::SansSerif,
                FontFamily::SystemMonospace => Family::Monospace,
                FontFamily::SystemSerif => Family::Serif,
            };
            let weight = Weight(font_weight.clamp(100, 900));
            let base_attrs = Attrs::new().family(family).weight(weight);
            // Cross-ref: PORTAL_LINE_HEIGHT_MULTIPLIER in
            // crates/tze_hud_projection/src/bin/projection_authority.rs mirrors this
            // value. If you change 1.4 here, update that constant in the same PR.
            let line_height = font_size_px * 1.4;
            // Dispatch to the correct truncation algorithm via TruncationViewport.
            // TailAnchored shows the LAST max_lines runs (newest content visible).
            // HeadAnchored shows the FIRST max_lines runs (default / scrolled-back).
            match viewport {
                TruncationViewport::TailAnchored => overflow::truncate_tail_anchored(
                    content,
                    base_attrs,
                    bounds_width,
                    bounds_height,
                    font_size_px,
                    line_height,
                    font_system,
                ),
                TruncationViewport::HeadAnchored => overflow::truncate_for_ellipsis(
                    content,
                    base_attrs,
                    bounds_width,
                    bounds_height,
                    font_size_px,
                    line_height,
                    font_system,
                ),
            }
        })
    }

    /// Evict all entries whose key is not in `live_keys`.
    ///
    /// Call this after priming to keep the cache bounded to the live node set.
    pub(crate) fn evict_except(&mut self, live_keys: &[TruncationKey]) {
        let keep: HashSet<TruncationKey> = live_keys.iter().copied().collect();
        self.entries.retain(|k, _| keep.contains(k));
    }

    /// Drop all cached entries.  Used when token map changes and font metrics
    /// may shift (font fallback / substitution may change after `set_token_map`).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

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
    /// Truncation result cache for `TextOverflow::Ellipsis` items.
    ///
    /// Keyed on `(content_hash, bounds_width, bounds_height, font_size_px,
    /// font_family, font_weight)`.  Populated outside the frame loop by
    /// [`TextRasterizer::prime_truncation_cache`]; the frame path looks up
    /// pre-computed results in O(1) via [`TruncationCache::get_by_key`].
    ///
    /// This satisfies the Phase-1 stable-shape-caching contract (RFC 0013
    /// §3.4, §4.2): `truncate_for_ellipsis` is called exactly once per
    /// content/geometry change, never on every frame.
    pub(crate) truncation_cache: TruncationCache,
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
            truncation_cache: TruncationCache::new(),
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

    /// Prime the truncation cache for all `TextOverflow::Ellipsis` items in `items`.
    ///
    /// Must be called **outside the per-frame pipeline** (at content-commit time
    /// or on geometry change), never inside `prepare_text_items`.  After this
    /// call, the frame path resolves each Ellipsis item in O(1) via
    /// `TruncationCache::get_by_key`.
    ///
    /// The `viewport` field on each `TextItem` drives truncation-mode selection:
    /// - [`TruncationViewport::TailAnchored`] items call
    ///   [`overflow::truncate_tail_anchored`] (spec task 3.2 — newest content
    ///   always visible for follow-tail streaming transcripts).
    /// - [`TruncationViewport::HeadAnchored`] items call
    ///   [`overflow::truncate_for_ellipsis`] (default / scrolled-back viewports,
    ///   spec task 3.3 append-stability guarantee).
    ///
    /// Returns the set of live [`TruncationKey`]s so the caller can evict stale
    /// entries with [`TruncationCache::evict_except`].
    pub(crate) fn prime_truncation_cache(&mut self, items: &[TextItem]) -> Vec<TruncationKey> {
        let mut live_keys: Vec<TruncationKey> = Vec::new();
        for item in items {
            if item.overflow != TextOverflow::Ellipsis {
                continue;
            }
            // (hud-so7zu) Use the effective measurement weight/family from
            // styled runs so that bold/monospace spans are measured accurately.
            // The truncation fit decision uses these attrs; if only base_weight
            // is used, wider bold/monospace text can wrap to more lines than
            // measured and get hard-clipped by TextBounds.
            let (meas_weight, meas_mono) =
                styled_runs_effective_measurement(&item.styled_runs, item.font_weight);
            let meas_family = if meas_mono {
                FontFamily::SystemMonospace
            } else {
                item.font_family
            };
            let key = TruncationKey::new(
                &item.text,
                item.bounds_width,
                item.bounds_height,
                item.font_size_px,
                meas_family,
                meas_weight,
                item.viewport,
            );
            live_keys.push(key);
            self.truncation_cache.prime(
                key,
                &item.text,
                item.bounds_width,
                item.bounds_height,
                item.font_size_px,
                meas_family,
                meas_weight,
                item.viewport,
                &mut self.font_system,
            );
        }
        live_keys
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

        // Phase 0 (hud-5jbra.3 / hud-wgq7j): resolve ellipsis-truncated text
        // strings from the truncation cache.
        //
        // The cache is primed outside the frame loop by `prime_truncation_cache`
        // (called from `Compositor::prime_truncation_cache` gated on scene.version).
        // On a warm path (after at least one commit), a cache hit is one BLAKE3
        // hash to reconstruct the key plus one HashMap lookup — zero shaping work.
        //
        // On a cache miss (e.g. very first frame before any commit, or a newly
        // added item whose geometry was not yet in the cache), we fall back to
        // calling `truncate_for_ellipsis` inline to avoid dropped glyphs.  This
        // fallback also populates the cache so the next frame is free.
        //
        // For Clip items the entry is `None` (use `item.text` directly).
        // For Ellipsis items the entry is `Some(truncated_text_string)`.
        //
        // Two-pass structure: we first build all keys and prime any misses
        // (requiring &mut self.truncation_cache + &mut self.font_system), then
        // do the final HashMap lookup pass.  This avoids interleaving mutable
        // and immutable borrows of the same struct fields inside a single closure.
        let keys: Vec<Option<TruncationKey>> = items
            .iter()
            .map(|item| {
                if item.overflow != TextOverflow::Ellipsis {
                    return None;
                }
                // (hud-so7zu) Use effective measurement attrs from styled runs.
                let (meas_weight, meas_mono) =
                    styled_runs_effective_measurement(&item.styled_runs, item.font_weight);
                let meas_family = if meas_mono {
                    FontFamily::SystemMonospace
                } else {
                    item.font_family
                };
                Some(TruncationKey::new(
                    &item.text,
                    item.bounds_width,
                    item.bounds_height,
                    item.font_size_px,
                    meas_family,
                    meas_weight,
                    item.viewport,
                ))
            })
            .collect();

        // Pass 1: prime any cache misses.  Misses are uncommon (first frame or
        // newly-added items); hits are no-ops via `entry().or_insert_with`.
        for (item, key_opt) in items.iter().zip(keys.iter()) {
            if let Some(key) = key_opt {
                // (hud-so7zu) Re-derive effective measurement attrs for the prime call.
                let (meas_weight, meas_mono) =
                    styled_runs_effective_measurement(&item.styled_runs, item.font_weight);
                let meas_family = if meas_mono {
                    FontFamily::SystemMonospace
                } else {
                    item.font_family
                };
                self.truncation_cache.prime(
                    *key,
                    &item.text,
                    item.bounds_width,
                    item.bounds_height,
                    item.font_size_px,
                    meas_family,
                    meas_weight,
                    item.viewport,
                    &mut self.font_system,
                );
            }
        }

        // Pass 2: collect final truncated strings.  All Ellipsis entries are now
        // in the cache; `get_by_key` will not return None for any Ellipsis item.
        let truncated_texts: Vec<Option<String>> = keys
            .iter()
            .map(|key_opt| {
                key_opt
                    .as_ref()
                    .and_then(|k| self.truncation_cache.get_by_key(k))
                    .map(|r| r.text.clone())
            })
            .collect();

        // Phase 1: build one Buffer per item (shared by all outline + fill passes).
        let buffers: Vec<Buffer> = items
            .iter()
            .zip(truncated_texts.iter())
            .map(|(item, truncated)| {
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

                // Use the pre-truncated text when overflow == Ellipsis; otherwise use item.text.
                let effective_text: &str = truncated.as_deref().unwrap_or(&item.text);

                if !item.styled_runs.is_empty() {
                    // Phase-1 markdown styled-run path (hud-5jbra.2).
                    //
                    // `styled_runs` carry per-span weight, italic, monospace, and
                    // optional color from the parse cache.  We build `(text_slice,
                    // Attrs)` pairs and call `set_rich_text` once — zero re-parse cost.
                    //
                    // When ellipsis truncation was applied, byte offsets in
                    // `styled_runs` must be remapped to the truncated string's
                    // coordinate system (hud-so7zu).
                    //
                    // HeadAnchored: truncated text is "{prefix}…" (trailing
                    // ellipsis).  Offsets within the prefix are valid as-is;
                    // we clamp at prefix_len = len - ELLIPSIS.len().
                    //
                    // TailAnchored: truncated text is "…\n{tail}" (leading
                    // ellipsis, hud-fe4ik).  Original offsets address the tail
                    // portion of the original text; they must be shifted so
                    // they address the correct bytes in "…\n{tail}".  See
                    // `reslice_styled_runs_tail_anchored` for the shift logic.
                    if let Some(truncated_str) = truncated {
                        let resliced = if item.viewport == TruncationViewport::TailAnchored
                            && truncated_str.starts_with(crate::overflow::ELLIPSIS)
                        {
                            // Leading-ellipsis output: shift offsets from
                            // original-text space into "…\n{tail}" space.
                            reslice_styled_runs_tail_anchored(
                                &item.text,
                                truncated_str,
                                &item.styled_runs,
                            )
                        } else {
                            // Trailing-ellipsis output (HeadAnchored): compute
                            // the byte length of the plain-text prefix
                            // (excluding "…").
                            let content_end = truncated_str
                                .strip_suffix(crate::overflow::ELLIPSIS)
                                .map(|s| s.len())
                                .unwrap_or(truncated_str.len());
                            reslice_styled_runs(&item.styled_runs, content_end)
                        };
                        if resliced.is_empty() {
                            // No runs survive truncation — fall back to plain text.
                            buf.set_text(
                                &mut self.font_system,
                                effective_text,
                                base_attrs,
                                Shaping::Basic,
                            );
                        } else {
                            let spans = styled_run_spans(
                                effective_text,
                                &resliced,
                                base_attrs,
                                item.font_family,
                            );
                            buf.set_rich_text(
                                &mut self.font_system,
                                spans,
                                base_attrs,
                                Shaping::Basic,
                            );
                        }
                    } else {
                        let spans = styled_run_spans(
                            &item.text,
                            &item.styled_runs,
                            base_attrs,
                            item.font_family,
                        );
                        buf.set_rich_text(&mut self.font_system, spans, base_attrs, Shaping::Basic);
                    }
                } else if item.color_runs.is_empty() {
                    // Fast path: no inline runs — use uniform base color.
                    buf.set_text(
                        &mut self.font_system,
                        effective_text,
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
                    //
                    // When ellipsis truncation was applied, remap color_run
                    // byte offsets to the truncated string's coordinate system
                    // (hud-so7zu / hud-fe4ik).  TailAnchored uses a
                    // leading-ellipsis shift; HeadAnchored uses a trailing-
                    // ellipsis clamp.  See reslice_color_runs_tail_anchored.
                    if let Some(truncated_str) = truncated {
                        let resliced = if item.viewport == TruncationViewport::TailAnchored
                            && truncated_str.starts_with(crate::overflow::ELLIPSIS)
                        {
                            reslice_color_runs_tail_anchored(
                                &item.text,
                                truncated_str,
                                &item.color_runs,
                            )
                        } else {
                            let content_end = truncated_str
                                .strip_suffix(crate::overflow::ELLIPSIS)
                                .map(|s| s.len())
                                .unwrap_or(truncated_str.len());
                            reslice_color_runs(&item.color_runs, content_end)
                        };
                        if resliced.is_empty() {
                            buf.set_text(
                                &mut self.font_system,
                                effective_text,
                                base_attrs,
                                Shaping::Basic,
                            );
                        } else {
                            let spans = color_run_spans(effective_text, &resliced, base_attrs);
                            buf.set_rich_text(
                                &mut self.font_system,
                                spans,
                                base_attrs,
                                Shaping::Basic,
                            );
                        }
                    } else {
                        let spans = color_run_spans(&item.text, &item.color_runs, base_attrs);
                        buf.set_rich_text(&mut self.font_system, spans, base_attrs, Shaping::Basic);
                    }
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
                left: item.clip_pixel_x as i32,
                top: item.clip_pixel_y as i32,
                right: (item.clip_pixel_x + item.clip_bounds_width) as i32,
                bottom: (item.clip_pixel_y + item.clip_bounds_height) as i32,
            };

            // Outline passes (only when outline is active).
            if let (Some(oc), Some(ow)) = (item.outline_color, item.outline_width) {
                if ow > 0.0 {
                    for (dx, dy) in &OUTLINE_DIRS {
                        let offset = ow;
                        text_areas.push(TextArea {
                            buffer: buf,
                            left: item.pixel_x + dx * offset,
                            top: item.pixel_y + dy * offset,
                            scale: 1.0,
                            bounds,
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
    ///
    /// Stored as `Arc<str>` so the per-frame path in
    /// [`TextItem::from_text_markdown_cached`] can clone it with a cheap
    /// reference-count bump rather than a deep string copy.  Call sites that
    /// need a `&str` can deref directly; those that need an owned `String` can
    /// call `.to_string()`.
    pub text: Arc<str>,
    /// Left edge in physical pixels (absolute, not tile-relative).
    pub pixel_x: f32,
    /// Top edge in physical pixels (absolute, not tile-relative).
    pub pixel_y: f32,
    /// Available width for word-wrap and clip.
    pub bounds_width: f32,
    /// Available height for clip.
    pub bounds_height: f32,
    /// Left edge of the glyph clip rectangle in physical pixels.
    pub clip_pixel_x: f32,
    /// Top edge of the glyph clip rectangle in physical pixels.
    pub clip_pixel_y: f32,
    /// Width of the glyph clip rectangle in physical pixels.
    pub clip_bounds_width: f32,
    /// Height of the glyph clip rectangle in physical pixels.
    pub clip_bounds_height: f32,
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
    /// Markdown-derived styled runs from the Phase-1 parse cache
    /// (hud-5jbra.2).
    ///
    /// When non-empty these runs take precedence over `color_runs` for
    /// determining per-span font weight, italic style, and font family.
    /// Color overrides in `styled_runs` also override `color_runs` for the
    /// same byte range.
    ///
    /// Populated by [`TextItem::from_text_markdown_cached`]; empty for all
    /// other constructors.
    pub styled_runs: Box<[StyledRunItem]>,
    /// Truncation viewport anchor mode.
    ///
    /// Controls which lines are shown when the text overflows the bounds:
    /// - [`TruncationViewport::HeadAnchored`] (default) — show the FIRST
    ///   `max_lines` (static tiles, scrolled-back viewports, spec task 3.3).
    /// - [`TruncationViewport::TailAnchored`] — show the LAST `max_lines`
    ///   (follow-tail streaming tiles at the tail, spec task 3.2).
    ///
    /// Set by `collect_text_items_from_node` / `collect_ellipsis_text_items_from_node`
    /// based on the tile's current `follow_tail_at_tail` state in the scene.
    pub viewport: TruncationViewport,
}

/// A single styled run for Phase-1 markdown rendering, with byte offsets into
/// the **plain-text** `TextItem::text`.
///
/// Carries full per-span style attributes — weight, italic, monospace, and
/// optional color override — resolved from design tokens at parse-commit time.
/// The frame pipeline consumes these without re-parsing.
#[derive(Debug, Clone)]
pub struct StyledRunItem {
    /// Inclusive byte offset into `TextItem::text`.
    pub start_byte: usize,
    /// Exclusive byte offset into `TextItem::text`.
    pub end_byte: usize,
    /// CSS font weight override (100–900); `None` = use `TextItem::font_weight`.
    pub weight: Option<u16>,
    /// When `true`, request the italic style variant.
    pub italic: bool,
    /// When `true`, use the monospace font family instead of the item default.
    pub monospace: bool,
    /// Optional color override (linear sRGB); `None` = use `TextItem::color`.
    pub color: Option<[u8; 4]>,
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
    /// When `node.color_runs` contains at least one non-empty range
    /// (`start_byte < end_byte`), Markdown stripping is skipped and the raw
    /// content is used as-is so byte offsets in the runs remain valid against
    /// the text.  Callers that supply such runs must pre-strip Markdown before
    /// populating `content`, or accept that markup characters appear in the
    /// output.
    ///
    /// Zero-length runs (`start_byte == end_byte`) are treated as pure
    /// sentinels (no pixel coverage); their presence does **not** suppress
    /// Markdown stripping.
    ///
    /// # Deprecation notice — empty `color_runs` path
    ///
    /// When `color_runs` is entirely empty (no pixel runs), this constructor
    /// calls `strip_markdown_v1` which is **lossy** — it strips markdown syntax
    /// characters but does not produce [`StyledRunItem`]s, so all markdown
    /// structure and styling is discarded.
    ///
    /// In the Phase-1 render path the compositor now always goes through
    /// [`TextItem::from_text_markdown_cached`] (primed at commit-time by
    /// `Compositor::prime_markdown_cache`).  The renderer's cache-miss
    /// fallback (hud-xcp9b) parses inline via `parse_markdown_subset` rather
    /// than falling through to this constructor, so the lossy strip path is no
    /// longer reachable from the render pipeline.
    ///
    /// **Do not call this constructor with an empty `color_runs` slice from any
    /// new production code.**  Use `from_text_markdown_cached` +
    /// `MarkdownCache::prime` instead for markdown-fidelity rendering.
    pub fn from_text_markdown_node(node: &TextMarkdownNode, tile_x: f32, tile_y: f32) -> Self {
        // Skip Markdown stripping only when at least one run has a non-empty
        // byte range — those runs reference offsets into the raw (un-stripped)
        // content and stripping would invalidate them.
        // Zero-length sentinel runs (start_byte == end_byte) carry metadata
        // only; they do not reference content offsets, so we can still strip.
        let has_pixel_runs = node.color_runs.iter().any(|r| r.start_byte < r.end_byte);
        let text: Arc<str> = if has_pixel_runs {
            Arc::from(node.content.as_str())
        } else {
            Arc::from(strip_markdown_v1(&node.content).as_str())
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
            clip_pixel_x: x,
            clip_pixel_y: y,
            clip_bounds_width: w,
            clip_bounds_height: h,
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
            styled_runs: Box::default(),
            // Viewport defaults to HeadAnchored; callers that know the tile is
            // at-tail (follow-tail streaming tiles) override this after construction.
            viewport: TruncationViewport::HeadAnchored,
        }
    }

    /// Build a `TextItem` from a `TextMarkdownNode` using a pre-parsed
    /// [`ParsedMarkdown`] from the [`MarkdownCache`] (Phase-1 hud-5jbra.2).
    ///
    /// This is the replacement for `from_text_markdown_node` in the Phase-1
    /// markdown path.  The `plain_text` and `styled_runs` from the cache are
    /// used directly — **no markdown parsing happens here**, satisfying the
    /// zero-per-frame-parse-cost requirement.
    ///
    /// # color_runs incompatibility
    ///
    /// This constructor uses `parsed.plain_text` as the text base (the
    /// Markdown-stripped version), which makes `node.color_runs` byte offsets
    /// invalid.  Consequently, `color_runs` is **not** preserved: the returned
    /// `TextItem` always has `color_runs: Box::default()`.
    ///
    /// Callers **must not** invoke this constructor when `node.color_runs` is
    /// non-empty.  Use [`TextItem::from_text_markdown_node`] instead; it
    /// preserves the raw content and maps `color_runs` correctly.
    ///
    /// `tile_x` / `tile_y` are the pixel-space position of the tile origin.
    ///
    /// [`ParsedMarkdown`]: crate::markdown::ParsedMarkdown
    /// [`MarkdownCache`]: crate::markdown::MarkdownCache
    pub fn from_text_markdown_cached(
        node: &TextMarkdownNode,
        tile_x: f32,
        tile_y: f32,
        parsed: &crate::markdown::ParsedMarkdown,
    ) -> Self {
        // Callers must not invoke this constructor when color_runs is non-empty
        // (see the # color_runs incompatibility doc comment above).  Assert in
        // debug builds so misuse is caught early without any release overhead.
        debug_assert!(
            node.color_runs.is_empty(),
            "from_text_markdown_cached called with non-empty color_runs; \
             use from_text_markdown_node instead"
        );
        use crate::markdown::StyledSpan;

        // PERF: Arc::clone is a refcount bump — no heap allocation or string copy.
        // This is the hot per-frame path: `parsed` lives in the MarkdownCache and
        // its `plain_text: Arc<str>` is shared across all frames for the same content.
        let text: Arc<str> = Arc::clone(&parsed.plain_text);

        // Convert linear f32 color [0..1] to sRGB u8 [0..255].
        let r = linear_to_srgb_u8(node.color.r);
        let g = linear_to_srgb_u8(node.color.g);
        let b = linear_to_srgb_u8(node.color.b);
        let a = (node.color.a * 255.0).clamp(0.0, 255.0) as u8;

        // Geometry: same inset margin logic as `from_text_markdown_node`.
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

        // Convert ParsedMarkdown spans to StyledRunItems.
        let styled_runs: Box<[StyledRunItem]> = parsed
            .spans
            .iter()
            .filter_map(|span: &StyledSpan| {
                let start = span.start_byte.min(text.len());
                let end = span.end_byte.min(text.len());
                if start >= end {
                    return None;
                }
                if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                    return None;
                }
                let color = span.attr.color.map(rgba_to_srgb_u8);
                Some(StyledRunItem {
                    start_byte: start,
                    end_byte: end,
                    weight: span.attr.weight,
                    italic: span.attr.italic,
                    monospace: span.attr.monospace,
                    color,
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
            clip_pixel_x: x,
            clip_pixel_y: y,
            clip_bounds_width: w,
            clip_bounds_height: h,
            font_size_px,
            font_family: node.font_family,
            font_weight: 400,
            color: [r, g, b, a],
            alignment: node.alignment,
            overflow: node.overflow,
            outline_color: None,
            outline_width: None,
            opacity: 1.0,
            color_runs: Box::default(),
            styled_runs,
            // Viewport defaults to HeadAnchored; callers that know the tile is
            // at-tail (follow-tail streaming tiles) override this after construction.
            viewport: TruncationViewport::HeadAnchored,
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
            text: Arc::from(text),
            pixel_x: x + margin_h,
            pixel_y: y + margin_v,
            bounds_width: (w - margin_h * 2.0).max(1.0),
            bounds_height: (h - margin_v * 2.0).max(1.0),
            clip_pixel_x: x + margin_h,
            clip_pixel_y: y + margin_v,
            clip_bounds_width: (w - margin_h * 2.0).max(1.0),
            clip_bounds_height: (h - margin_v * 2.0).max(1.0),
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
            styled_runs: Box::default(),
            viewport: TruncationViewport::HeadAnchored,
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
            text: Arc::from(text),
            pixel_x: x + margin,
            pixel_y: y + margin,
            bounds_width: (w - margin * 2.0).max(1.0),
            bounds_height: (h - margin * 2.0).max(1.0),
            clip_pixel_x: x + margin,
            clip_pixel_y: y + margin,
            clip_bounds_width: (w - margin * 2.0).max(1.0),
            clip_bounds_height: (h - margin * 2.0).max(1.0),
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
            styled_runs: Box::default(),
            viewport: TruncationViewport::HeadAnchored,
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
            text: Arc::from(text),
            pixel_x: x + margin,
            pixel_y: y + margin,
            bounds_width: (w - margin * 2.0).max(1.0),
            bounds_height: (h - margin * 2.0).max(1.0),
            clip_pixel_x: x + margin,
            clip_pixel_y: y + margin,
            clip_bounds_width: (w - margin * 2.0).max(1.0),
            clip_bounds_height: (h - margin * 2.0).max(1.0),
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
            styled_runs: Box::default(),
            viewport: TruncationViewport::HeadAnchored,
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
        out.push(ColorRunItem {
            start_byte: s,
            end_byte: e,
            color: c,
        });
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

/// Build `(text_slice, Attrs)` pairs for [`Buffer::set_rich_text`] from a set of
/// [`StyledRunItem`]s produced by the Phase-1 markdown parse cache.
///
/// Each run carries weight, italic, monospace, and optional color.  Gaps
/// between runs receive `base_attrs` with no overrides.
///
/// `base_family` is the node-level font family (e.g. `SansSerif`) used for
/// non-monospace runs.  Monospace runs use `Family::Monospace` regardless.
pub(crate) fn styled_run_spans<'t, 'a>(
    text: &'t str,
    runs: &[StyledRunItem],
    base_attrs: Attrs<'a>,
    base_family: FontFamily,
) -> Vec<(&'t str, Attrs<'a>)> {
    if runs.is_empty() {
        return vec![(text, base_attrs)];
    }

    let mono_family = Family::Monospace;
    let sans_family = match base_family {
        FontFamily::SystemSansSerif => Family::SansSerif,
        FontFamily::SystemMonospace => Family::Monospace,
        FontFamily::SystemSerif => Family::Serif,
    };

    let mut spans: Vec<(&'t str, Attrs<'a>)> = Vec::with_capacity(runs.len() * 2 + 1);
    let mut cursor = 0usize;

    for run in runs {
        let start = run.start_byte.min(text.len());
        let end = run.end_byte.min(text.len());
        if start >= end {
            continue;
        }
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        // Clamp start to cursor so overlapping runs (which should not occur
        // after the fill_gaps_with_base fix, but may in edge cases) never
        // cause duplicate text slices or backward indexing.
        let start = start.max(cursor);
        if start >= end {
            cursor = cursor.max(end);
            continue;
        }

        // Emit unstyled gap before this run.
        if cursor < start && text.is_char_boundary(cursor) {
            spans.push((&text[cursor..start], base_attrs));
        }

        // Build attrs for this run.
        let run_weight = run
            .weight
            .map(|w| Weight(w.clamp(100, 900)))
            .unwrap_or_else(|| {
                // Inherit base weight (already applied to base_attrs).
                base_attrs.weight
            });
        let run_style = if run.italic {
            Style::Italic
        } else {
            Style::Normal
        };
        let run_family = if run.monospace {
            mono_family
        } else {
            sans_family
        };

        let mut run_attrs = base_attrs
            .weight(run_weight)
            .style(run_style)
            .family(run_family);

        if let Some(c) = run.color {
            run_attrs = run_attrs.color(Color::rgba(c[0], c[1], c[2], c[3]));
        }

        spans.push((&text[start..end], run_attrs));
        cursor = end;
    }

    // Trailing unstyled text.
    if cursor < text.len() && text.is_char_boundary(cursor) {
        spans.push((&text[cursor..], base_attrs));
    }

    if spans.is_empty() {
        spans.push((text, base_attrs));
    }

    spans
}

/// Re-slice [`StyledRunItem`]s to a truncated byte range.
///
/// When ellipsis truncation is applied, the truncated text is a plain-text
/// prefix with "…" appended.  The original `styled_runs` byte offsets remain
/// valid for the prefix portion of the truncated text, but must be clamped to
/// `content_end_byte` (the byte length of the prefix, excluding "…").
///
/// Runs entirely beyond `content_end_byte` are dropped.
/// Runs that partially overlap `content_end_byte` are clamped at that boundary,
/// provided both the run start and the clamped end are valid UTF-8 boundaries in
/// `truncated_text`.  Invalid boundary runs are dropped rather than panicking.
///
/// Returns an owned slice ready to pass to [`styled_run_spans`].
pub(crate) fn reslice_styled_runs(
    runs: &[StyledRunItem],
    content_end_byte: usize,
) -> Vec<StyledRunItem> {
    runs.iter()
        .filter_map(|run| {
            let start = run.start_byte;
            let end = run.end_byte.min(content_end_byte);
            if start >= end || start >= content_end_byte {
                return None;
            }
            Some(StyledRunItem {
                start_byte: start,
                end_byte: end,
                weight: run.weight,
                italic: run.italic,
                monospace: run.monospace,
                color: run.color,
            })
        })
        .collect()
}

/// Re-slice [`ColorRunItem`]s to a truncated byte range.
///
/// Analogous to [`reslice_styled_runs`]: clamps runs to `content_end_byte`
/// (the byte length of the plain-text prefix, excluding the trailing "…").
/// Runs entirely beyond `content_end_byte` are dropped; partial runs are clamped.
pub(crate) fn reslice_color_runs(
    runs: &[ColorRunItem],
    content_end_byte: usize,
) -> Vec<ColorRunItem> {
    runs.iter()
        .filter_map(|run| {
            let start = run.start_byte;
            let end = run.end_byte.min(content_end_byte);
            if start >= end || start >= content_end_byte {
                return None;
            }
            Some(ColorRunItem {
                start_byte: start,
                end_byte: end,
                color: run.color,
            })
        })
        .collect()
}

/// Re-slice [`StyledRunItem`]s for a **tail-anchored** truncated string.
///
/// `truncate_tail_anchored` produces a leading-ellipsis output:
/// `"…\n{visible_tail}"`.  The original `styled_runs` byte offsets are
/// relative to the **original** text; they must be shifted to become relative
/// to `effective_text` (= the leading-ellipsis truncated string).
///
/// The shift is computed by finding where `visible_tail` begins in
/// `original_text` (`tail_origin`) and applying:
///
/// ```text
/// new_offset = old_offset - tail_origin + leading_bytes
/// ```
///
/// where `leading_bytes = ELLIPSIS.len() + 1` (the `"…\n"` prefix = 4 bytes).
///
/// If the first visible line was also horizontally truncated (its text no
/// longer matches the suffix of `original_text`), the shift cannot be computed
/// exactly.  In that case an empty `Vec` is returned and the caller must fall
/// back to `buf.set_text` (unstyled) to avoid mis-styled glyphs.
///
/// Runs entirely outside the visible tail range are dropped; runs partially
/// overlapping `tail_origin` are clamped to the tail start.
///
/// Returns a `Vec<StyledRunItem>` with offsets valid against `truncated_str`.
pub(crate) fn reslice_styled_runs_tail_anchored(
    original_text: &str,
    truncated_str: &str,
    runs: &[StyledRunItem],
) -> Vec<StyledRunItem> {
    // "…\n" prefix: ELLIPSIS (3 UTF-8 bytes) + newline (1 byte).
    let leading_bytes = crate::overflow::ELLIPSIS.len() + 1;

    // Extract the visible tail content (after "…\n").
    let tail_content = match truncated_str.get(leading_bytes..) {
        Some(s) => s,
        None => return Vec::new(), // truncated_str is too short — malformed
    };

    // Determine where the tail content begins in the original text.
    // If original_text ends with tail_content, the shift is unambiguous.
    // Otherwise the first visible line was further truncated; we cannot safely
    // remap offsets, so return empty to trigger the unstyled fallback.
    if !original_text.ends_with(tail_content) {
        return Vec::new();
    }
    let tail_origin = original_text.len() - tail_content.len();

    // Shift: subtract tail_origin (to make offsets relative to tail_content),
    // then add leading_bytes (to account for the "…\n" prefix in truncated_str).
    // Saturate at 0 for runs that start before tail_origin (clamped to tail).
    runs.iter()
        .filter_map(|run| {
            // Drop runs entirely before the visible tail.
            if run.end_byte <= tail_origin {
                return None;
            }
            // Clamp start to tail_origin (run starts before visible range).
            let orig_start = run.start_byte.max(tail_origin);
            let orig_end = run.end_byte;

            // Shift to effective_text coordinates.
            let new_start = orig_start - tail_origin + leading_bytes;
            let new_end = orig_end - tail_origin + leading_bytes;

            // Clamp new_end to truncated_str length (defensive).
            let new_end = new_end.min(truncated_str.len());

            if new_start >= new_end {
                return None;
            }

            Some(StyledRunItem {
                start_byte: new_start,
                end_byte: new_end,
                weight: run.weight,
                italic: run.italic,
                monospace: run.monospace,
                color: run.color,
            })
        })
        .collect()
}

/// Re-slice [`ColorRunItem`]s for a **tail-anchored** truncated string.
///
/// Analogous to [`reslice_styled_runs_tail_anchored`]: shifts run offsets from
/// the original-text coordinate system to the leading-ellipsis `"…\n{tail}"`
/// coordinate system produced by [`overflow::truncate_tail_anchored`].
///
/// Returns an empty `Vec` when the first visible line was horizontally
/// truncated (offsets cannot be safely remapped); the caller must fall back to
/// `buf.set_text`.
pub(crate) fn reslice_color_runs_tail_anchored(
    original_text: &str,
    truncated_str: &str,
    runs: &[ColorRunItem],
) -> Vec<ColorRunItem> {
    let leading_bytes = crate::overflow::ELLIPSIS.len() + 1;

    let tail_content = match truncated_str.get(leading_bytes..) {
        Some(s) => s,
        None => return Vec::new(),
    };

    if !original_text.ends_with(tail_content) {
        return Vec::new();
    }
    let tail_origin = original_text.len() - tail_content.len();

    runs.iter()
        .filter_map(|run| {
            if run.end_byte <= tail_origin {
                return None;
            }
            let orig_start = run.start_byte.max(tail_origin);
            let orig_end = run.end_byte;
            let new_start = orig_start - tail_origin + leading_bytes;
            let new_end = (orig_end - tail_origin + leading_bytes).min(truncated_str.len());
            if new_start >= new_end {
                return None;
            }
            Some(ColorRunItem {
                start_byte: new_start,
                end_byte: new_end,
                color: run.color,
            })
        })
        .collect()
}

/// Compute the "widest" effective measurement weight and font family for a
/// `TextItem` that has [`StyledRunItem`]s.
///
/// Used when priming the truncation cache for items with styled runs (hud-so7zu).
/// Styled runs may include bold (weight=700) or monospace spans that are wider
/// than the base font.  If the truncation measurement uses only the base attrs,
/// bold/monospace text wraps to more lines than measured, causing the fit
/// decision to declare text fits when it actually overflows.
///
/// Returns `(effective_weight, use_monospace)` where:
/// - `effective_weight` is the maximum weight across all runs that explicitly
///   set a weight (fallback: `base_weight`).
/// - `use_monospace` is `true` if any run requests the monospace family.
///
/// Callers should use these values to build the `base_attrs` passed to
/// [`overflow::truncate_for_ellipsis`] / [`overflow::truncate_tail_anchored`]
/// so the measurement is pessimistic (wider) and the fit decision is accurate.
pub(crate) fn styled_runs_effective_measurement(
    runs: &[StyledRunItem],
    base_weight: u16,
) -> (u16, bool) {
    let mut max_weight = base_weight;
    let mut any_monospace = false;
    for run in runs {
        if let Some(w) = run.weight {
            max_weight = max_weight.max(w);
        }
        if run.monospace {
            any_monospace = true;
        }
    }
    (max_weight, any_monospace)
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
        assert_eq!(&*item.text, "Alert: ready");
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
        assert_eq!(&*item.text, "Hello\nworld");
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
        let policy = RenderingPolicy {
            overflow: Some(TextOverflow::Ellipsis),
            ..RenderingPolicy::default()
        };
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
        let policy = RenderingPolicy {
            overflow: Some(TextOverflow::Clip),
            ..RenderingPolicy::default()
        };
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
        assert!(
            spans[1].1.color_opt.is_none(),
            "gap span must use base_attrs (no color_opt)"
        );
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
        assert!(
            spans[1].1.color_opt.is_none(),
            "trailing span must use base_attrs"
        );
    }

    /// Empty runs slice falls back to a single full-text base-attrs span.
    #[test]
    fn color_run_spans_empty_runs_fallback() {
        let text = "no color";
        let spans = color_run_spans(text, &[], Attrs::new());
        assert_eq!(spans.len(), 1, "no runs → single fallback span");
        assert_eq!(spans[0].0, "no color");
        assert!(
            spans[0].1.color_opt.is_none(),
            "fallback span must use base_attrs"
        );
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
        assert!(
            !spans.is_empty(),
            "degenerate run must not produce empty spans"
        );
        let combined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            combined, "text",
            "all text must be covered even with degenerate run"
        );
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
            ColorRunItem {
                start_byte: 0,
                end_byte: 10,
                color: red,
            },
            ColorRunItem {
                start_byte: 5,
                end_byte: 15,
                color: blue,
            },
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
        assert_eq!(
            (red_color.r(), red_color.g(), red_color.b()),
            (255, 0, 0),
            "first span should be red"
        );

        // Second span: "56789abcde" (bytes 5..15) — blue (later run wins intersection).
        assert_eq!(spans[1].0, &text[5..15], "blue span should cover 5..15");
        assert!(
            spans[1].1.color_opt.is_some(),
            "second span must be colored"
        );
        let blue_color = spans[1].1.color_opt.unwrap();
        assert_eq!(
            (blue_color.r(), blue_color.g(), blue_color.b()),
            (0, 0, 255),
            "second span should be blue (later-writer-wins)"
        );

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
            ColorRunItem {
                start_byte: 0,
                end_byte: 8,
                color: red,
            },
            ColorRunItem {
                start_byte: 4,
                end_byte: 12,
                color: green,
            },
            ColorRunItem {
                start_byte: 10,
                end_byte: 15,
                color: blue,
            },
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
            ColorRunItem {
                start_byte: 0,
                end_byte: 5,
                color: red,
            },
            ColorRunItem {
                start_byte: 6,
                end_byte: 11,
                color: blue,
            },
        ];
        // Reversed order: same runs but blue listed first.
        let unsorted_runs = [
            ColorRunItem {
                start_byte: 6,
                end_byte: 11,
                color: blue,
            },
            ColorRunItem {
                start_byte: 0,
                end_byte: 5,
                color: red,
            },
        ];

        let base = Attrs::new();
        let sorted_spans = color_run_spans(text, &sorted_runs, base);
        let unsorted_spans = color_run_spans(text, &unsorted_runs, base);

        // Text coverage must be identical.
        let sorted_text: String = sorted_spans.iter().map(|(s, _)| *s).collect();
        let unsorted_text: String = unsorted_spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(sorted_text, unsorted_text, "text coverage must match");

        // Span count must match.
        assert_eq!(
            sorted_spans.len(),
            unsorted_spans.len(),
            "span count must match"
        );

        // Each span's slice and color must match.
        for i in 0..sorted_spans.len() {
            assert_eq!(
                sorted_spans[i].0, unsorted_spans[i].0,
                "span[{i}] text slice must match"
            );
            assert_eq!(
                sorted_spans[i].1.color_opt, unsorted_spans[i].1.color_opt,
                "span[{i}] color must match"
            );
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
            ColorRunItem {
                start_byte: 0,
                end_byte: 3,
                color: red,
            },
            ColorRunItem {
                start_byte: 3,
                end_byte: 6,
                color: blue,
            },
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
            ColorRunItem {
                start_byte: 0,
                end_byte: 12,
                color: red,
            },
            ColorRunItem {
                start_byte: 4,
                end_byte: 8,
                color: blue,
            }, // nested inside red
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
        assert_eq!(
            (c1.r(), c1.g(), c1.b()),
            (0, 0, 255),
            "inner = blue (later wins)"
        );
        let c2 = spans[2].1.color_opt.unwrap();
        assert_eq!(
            (c2.r(), c2.g(), c2.b()),
            (255, 0, 0),
            "suffix = red (resumed)"
        );
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
            ColorRunItem {
                start_byte: 5,
                end_byte: 15,
                color: blue,
            },
            ColorRunItem {
                start_byte: 0,
                end_byte: 10,
                color: red,
            },
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
        assert_eq!(
            (c0.r(), c0.g(), c0.b()),
            (255, 0, 0),
            "red (higher index) wins 0..10"
        );
        let c1 = spans[1].1.color_opt.unwrap();
        assert_eq!(
            (c1.r(), c1.g(), c1.b()),
            (0, 0, 255),
            "blue covers unclaimed 10..15"
        );
    }

    /// Zero-length and out-of-bounds runs are silently dropped.
    #[test]
    fn color_run_spans_degenerate_runs_dropped() {
        let text = "hello"; // 5 bytes
        let red = [255u8, 0, 0, 255];
        let runs = [
            // zero-length: start == end
            ColorRunItem {
                start_byte: 2,
                end_byte: 2,
                color: red,
            },
            // out-of-bounds: extends past text.len()
            ColorRunItem {
                start_byte: 3,
                end_byte: 100,
                color: red,
            },
            // valid run
            ColorRunItem {
                start_byte: 0,
                end_byte: 3,
                color: red,
            },
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

    // ── color_run_spans multiline tests [hud-vwggh] ───────────────────────────

    /// A single color run spanning bytes that cross two `\n` boundaries is
    /// produced as one contiguous styled span.
    ///
    /// text  = "line 1\nline 2\nline 3"  (20 bytes)
    ///          0123456 789012 3456789
    ///                ^           ^
    ///         run:  3..16  →  "e 1\nline 2\nli"
    ///
    /// Expected spans:
    ///   [0] "lin"          (bytes 0..3)   — unstyled prefix on line 1
    ///   [1] "e 1\nline 2\nli" (bytes 3..16) — single colored span crossing both \n
    ///   [2] "ne 3"         (bytes 16..20) — unstyled suffix on line 3
    ///
    /// This verifies that `color_run_spans` does not split on `\n` itself and
    /// that the resulting string slices include the newline characters verbatim.
    /// `set_rich_text` (glyphon) handles the actual line splitting during shaping.
    #[test]
    fn color_run_spans_cross_newline_single_run() {
        let text = "line 1\nline 2\nline 3";
        // byte layout: "line 1"=0..6, '\n'=6, "line 2"=7..13, '\n'=13, "line 3"=14..20
        let runs = [ColorRunItem {
            start_byte: 3,
            end_byte: 16, // spans "e 1\nline 2\nli"
            color: [255, 128, 0, 255],
        }];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);

        assert_eq!(
            spans.len(),
            3,
            "cross-newline run must produce: prefix + colored + suffix"
        );

        // Unstyled prefix (part of line 1 before the run).
        assert_eq!(spans[0].0, "lin", "prefix span must be 'lin'");
        assert!(
            spans[0].1.color_opt.is_none(),
            "prefix span must use base_attrs (no color)"
        );

        // Colored span crossing both newlines — the slice includes the '\n' chars.
        assert_eq!(
            spans[1].0, "e 1\nline 2\nli",
            "colored span must include both newline characters verbatim"
        );
        assert!(
            spans[1].1.color_opt.is_some(),
            "colored span must carry color_opt"
        );

        // Unstyled suffix (part of line 3 after the run end).
        assert_eq!(spans[2].0, "ne 3", "suffix span must be 'ne 3'");
        assert!(
            spans[2].1.color_opt.is_none(),
            "suffix span must use base_attrs (no color)"
        );

        // Sanity: concatenating all spans reconstructs the original text.
        let reconstructed: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            reconstructed, text,
            "spans must reconstruct the original text exactly"
        );
    }

    /// Three color runs each constrained to one line produce per-line color spans
    /// with the `\n` separator characters falling into unstyled gap spans between
    /// them.  No color bleeds across a newline boundary.
    ///
    /// text  = "red line\nblue line\ngreen line"  (29 bytes)
    ///          01234567  8 9012345678  9012345678
    ///
    /// Runs:  [0..8] red, [9..18] blue, [19..29] green
    ///
    /// Expected spans (5 total):
    ///   [0] "red line"  (0..8)   — red
    ///   [1] "\n"        (8..9)   — unstyled gap
    ///   [2] "blue line" (9..18)  — blue
    ///   [3] "\n"        (18..19) — unstyled gap
    ///   [4] "green line"(19..29) — green
    #[test]
    fn color_run_spans_per_line_runs_no_bleed() {
        let text = "red line\nblue line\ngreen line";
        // byte layout: "red line"=0..8, '\n'=8, "blue line"=9..18, '\n'=18, "green line"=19..29
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let green = [0, 200, 0, 255];
        let runs = [
            ColorRunItem {
                start_byte: 0,
                end_byte: 8,
                color: red,
            },
            ColorRunItem {
                start_byte: 9,
                end_byte: 18,
                color: blue,
            },
            ColorRunItem {
                start_byte: 19,
                end_byte: 29,
                color: green,
            },
        ];
        let base = Attrs::new();
        let spans = color_run_spans(text, &runs, base);

        assert_eq!(
            spans.len(),
            5,
            "3 per-line runs with \\n gaps must produce 5 spans (run, \\n, run, \\n, run)"
        );

        // Line 1: red
        assert_eq!(spans[0].0, "red line", "first span must be 'red line'");
        assert!(
            spans[0].1.color_opt.is_some(),
            "line-1 span must carry color_opt"
        );

        // Gap: the '\n' between line 1 and line 2 is unstyled.
        assert_eq!(
            spans[1].0, "\n",
            "gap between line 1 and line 2 must be '\\n'"
        );
        assert!(
            spans[1].1.color_opt.is_none(),
            "newline gap must use base_attrs — no color bleed from line-1 run"
        );

        // Line 2: blue
        assert_eq!(spans[2].0, "blue line", "second span must be 'blue line'");
        assert!(
            spans[2].1.color_opt.is_some(),
            "line-2 span must carry color_opt"
        );

        // Gap: the '\n' between line 2 and line 3 is unstyled.
        assert_eq!(
            spans[3].0, "\n",
            "gap between line 2 and line 3 must be '\\n'"
        );
        assert!(
            spans[3].1.color_opt.is_none(),
            "newline gap must use base_attrs — no color bleed from line-2 run"
        );

        // Line 3: green
        assert_eq!(spans[4].0, "green line", "third span must be 'green line'");
        assert!(
            spans[4].1.color_opt.is_some(),
            "line-3 span must carry color_opt"
        );

        // Sanity: concatenating all spans reconstructs the original text.
        let reconstructed: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            reconstructed, text,
            "spans must reconstruct the original text exactly"
        );
    }

    // ── TruncationCache tests (hud-wgq7j) ────────────────────────────────────
    //
    // These tests verify the stable-shape-caching contract: `truncate_for_ellipsis`
    // is called exactly once per content/geometry change and zero times on
    // subsequent frames with the same scene.

    /// Cache hit for the same (content, geometry) pair is guaranteed after prime().
    ///
    /// This is the key invariant: zero truncation work per frame on unchanged
    /// content — mirroring `cache_hit_after_prime` in `markdown.rs`.
    #[test]
    fn truncation_cache_hit_after_prime() {
        let mut fs = FontSystem::new();
        let mut cache = TruncationCache::new();

        let content =
            "This is a long sentence that will definitely need truncation at narrow widths.";
        let bounds_w = 80.0_f32;
        let bounds_h = 50.0_f32;
        let font_size = 16.0_f32;
        let family = FontFamily::SystemSansSerif;
        let weight = 400u16;

        let key = TruncationKey::new(
            content,
            bounds_w,
            bounds_h,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
        );

        // Prime: first call computes truncation.
        let first = cache
            .prime(
                key,
                content,
                bounds_w,
                bounds_h,
                font_size,
                family,
                weight,
                TruncationViewport::HeadAnchored,
                &mut fs,
            )
            .clone();

        // Second call with identical key: must hit the cache (no re-shaping).
        let second = cache
            .prime(
                key,
                content,
                bounds_w,
                bounds_h,
                font_size,
                family,
                weight,
                TruncationViewport::HeadAnchored,
                &mut fs,
            )
            .clone();

        assert_eq!(
            first, second,
            "cached truncation result must be identical to the computed result"
        );
        assert_eq!(
            cache.len(),
            1,
            "only one cache entry for the same (content, geometry) pair"
        );
    }

    /// Different geometry produces a separate cache entry.
    #[test]
    fn truncation_cache_different_geometry_different_entry() {
        let mut fs = FontSystem::new();
        let mut cache = TruncationCache::new();

        let content = "Hello world this is a somewhat long string";
        let family = FontFamily::SystemSansSerif;
        let weight = 400u16;
        let font_size = 16.0_f32;

        let key_narrow = TruncationKey::new(
            content,
            60.0,
            50.0,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
        );
        let key_wide = TruncationKey::new(
            content,
            300.0,
            50.0,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
        );

        cache.prime(
            key_narrow,
            content,
            60.0,
            50.0,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
            &mut fs,
        );
        cache.prime(
            key_wide,
            content,
            300.0,
            50.0,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
            &mut fs,
        );

        assert_eq!(
            cache.len(),
            2,
            "two distinct geometries → two cache entries"
        );

        // The wide result should not be truncated; the narrow one should be.
        let narrow_result = cache
            .get_by_key(&key_narrow)
            .expect("narrow must be cached");
        let wide_result = cache.get_by_key(&key_wide).expect("wide must be cached");
        assert!(
            wide_result.text.len() >= narrow_result.text.len(),
            "wider bounds must produce an equal-or-longer result"
        );
    }

    /// get_by_key returns None for a key that has not been primed.
    #[test]
    fn truncation_cache_miss_before_prime() {
        let cache = TruncationCache::new();
        let key = TruncationKey::new(
            "text",
            100.0,
            50.0,
            16.0,
            FontFamily::SystemSansSerif,
            400,
            TruncationViewport::HeadAnchored,
        );
        assert!(
            cache.get_by_key(&key).is_none(),
            "cache must be empty before prime"
        );
    }

    /// Zero-truncation-work invariant: after priming, a second `prime` call
    /// must not call `truncate_for_ellipsis` again (proven via cache.len()
    /// staying at 1, not 2, after the second call with the same key).
    ///
    /// This is the per-frame zero-cost assertion: once primed, the frame path
    /// pays only O(1) for a lookup, not O(n) for shaping.
    #[test]
    fn truncation_cache_zero_work_on_unchanged_scene() {
        let mut fs = FontSystem::new();
        let mut cache = TruncationCache::new();

        let content = "Transcript line that is long enough to require ellipsis truncation here";
        let bounds_w = 120.0_f32;
        let bounds_h = 30.0_f32;
        let font_size = 14.0_f32;
        let family = FontFamily::SystemSansSerif;
        let weight = 400u16;

        let key = TruncationKey::new(
            content,
            bounds_w,
            bounds_h,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
        );

        // Frame 1: cold start — cache miss, computes and stores result.
        let frame1_result = cache
            .prime(
                key,
                content,
                bounds_w,
                bounds_h,
                font_size,
                family,
                weight,
                TruncationViewport::HeadAnchored,
                &mut fs,
            )
            .clone();

        // Frame 2+: unchanged scene — cache hit, zero shaping work.
        // We call prime() 9 more times to simulate 9 subsequent frames.
        for _ in 0..9 {
            let frame_n_result = cache
                .prime(
                    key,
                    content,
                    bounds_w,
                    bounds_h,
                    font_size,
                    family,
                    weight,
                    TruncationViewport::HeadAnchored,
                    &mut fs,
                )
                .clone();
            assert_eq!(
                frame1_result, frame_n_result,
                "repeated prime() with unchanged scene must return the cached result"
            );
        }

        // The cache still holds exactly one entry — no spurious re-insertions.
        assert_eq!(
            cache.len(),
            1,
            "cache must hold exactly one entry for one (content, geometry) pair"
        );
    }

    /// evict_except removes entries not in the live set.
    #[test]
    fn truncation_cache_evict_removes_stale_entries() {
        let mut fs = FontSystem::new();
        let mut cache = TruncationCache::new();

        let family = FontFamily::SystemSansSerif;
        let weight = 400u16;
        let font_size = 14.0_f32;

        let content_a = "Keep this line";
        let content_b = "Evict this line";

        let key_a = TruncationKey::new(
            content_a,
            120.0,
            30.0,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
        );
        let key_b = TruncationKey::new(
            content_b,
            120.0,
            30.0,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
        );

        cache.prime(
            key_a,
            content_a,
            120.0,
            30.0,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
            &mut fs,
        );
        cache.prime(
            key_b,
            content_b,
            120.0,
            30.0,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
            &mut fs,
        );
        assert_eq!(cache.len(), 2);

        cache.evict_except(&[key_a]);

        assert_eq!(cache.len(), 1, "evict_except must remove the stale entry");
        assert!(
            cache.get_by_key(&key_a).is_some(),
            "kept entry must remain after eviction"
        );
        assert!(
            cache.get_by_key(&key_b).is_none(),
            "evicted entry must be gone"
        );
    }

    /// clear() empties the cache entirely.
    #[test]
    fn truncation_cache_clear_empties_all_entries() {
        let mut fs = FontSystem::new();
        let mut cache = TruncationCache::new();

        let family = FontFamily::SystemSansSerif;
        let weight = 400u16;
        let font_size = 14.0_f32;

        for i in 0..3 {
            let content = format!("Content line {i}");
            let key = TruncationKey::new(
                &content,
                120.0,
                30.0,
                font_size,
                family,
                weight,
                TruncationViewport::HeadAnchored,
            );
            cache.prime(
                key,
                &content,
                120.0,
                30.0,
                font_size,
                family,
                weight,
                TruncationViewport::HeadAnchored,
                &mut fs,
            );
        }
        assert_eq!(cache.len(), 3);

        cache.clear();
        assert_eq!(cache.len(), 0, "clear() must empty the cache");
        assert!(
            cache.is_empty(),
            "is_empty() must return true after clear()"
        );
    }

    /// Frame-path lookup is O(1): get_by_key with a pre-computed key returns
    /// the result without touching font_system.
    #[test]
    fn truncation_cache_frame_path_lookup_is_o1() {
        let mut fs = FontSystem::new();
        let mut cache = TruncationCache::new();

        let content = "Short text for O(1) lookup test";
        let key = TruncationKey::new(
            content,
            200.0,
            50.0,
            16.0,
            FontFamily::SystemSansSerif,
            400,
            TruncationViewport::HeadAnchored,
        );

        // Prime once (commit-time cost).
        cache.prime(
            key,
            content,
            200.0,
            50.0,
            16.0,
            FontFamily::SystemSansSerif,
            400,
            TruncationViewport::HeadAnchored,
            &mut fs,
        );

        // Frame-path: get_by_key must return Some without touching font_system.
        let result = cache.get_by_key(&key);
        assert!(result.is_some(), "frame-path lookup must hit after prime");

        // Result must not be truncated for a 200px-wide box and short text.
        let result = result.unwrap();
        assert!(
            result.text.contains("Short text"),
            "short text must pass through unchanged"
        );
    }

    // ── Mid-drag re-truncation tests (hud-ghhxa — spec §6b.3) ─────────────────
    //
    // These tests verify that when a portal tile's bounds change during a resize
    // drag, the TruncationCache correctly re-resolves at each new intermediate
    // geometry.  The key invariant from spec §6b.3: "pane re-layout under the
    // overflow contract at every intermediate geometry."
    //
    // Because TruncationCache is keyed on bounds (bounds_width_bits /
    // bounds_height_bits are part of TruncationKey), a bounds change produces a
    // cache miss.  These tests confirm that:
    //   a) priming at a new (narrower) geometry produces truncation at that
    //      geometry, not at the old geometry.
    //   b) an intermediate geometry is independently primed and its result
    //      differs from the initial geometry.
    //   c) the cache holds separate entries for each distinct geometry.

    /// Priming at a narrower intermediate geometry produces a separate cache
    /// entry with truncation that fits within the new (narrower) bounds.
    ///
    /// This is the core §6b.3 invariant: at each intermediate geometry during
    /// a resize, the overflow contract is re-applied to the new bounds.
    #[test]
    fn intermediate_resize_geometry_re_resolves_truncation() {
        let mut fs = FontSystem::new();
        let mut cache = TruncationCache::new();

        // Long content that will overflow a narrow box but fits a wide one.
        let content = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"; // 39 'A's
        let family = FontFamily::SystemSansSerif;
        let weight = 400u16;
        let font_size = 16.0_f32;
        let height = 100.0_f32;

        // ── Step 1: prime at initial (wide) geometry ──────────────────────────
        let wide_w = 600.0_f32;
        let key_wide = TruncationKey::new(
            content,
            wide_w,
            height,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
        );
        let result_wide = cache
            .prime(
                key_wide,
                content,
                wide_w,
                height,
                font_size,
                family,
                weight,
                TruncationViewport::HeadAnchored,
                &mut fs,
            )
            .clone();

        // Wide box: long content likely fits without truncation.
        // (We don't assert on was_truncated for the wide case as it depends on
        // font metrics — just record the result for comparison.)

        // ── Step 2: prime at an intermediate (narrow) geometry ───────────────
        // This simulates a resize drag that has moved the pane to a narrower width.
        let narrow_w = 60.0_f32; // narrow enough to force truncation
        let key_narrow = TruncationKey::new(
            content,
            narrow_w,
            height,
            font_size,
            family,
            weight,
            TruncationViewport::HeadAnchored,
        );
        let result_narrow = cache
            .prime(
                key_narrow,
                content,
                narrow_w,
                height,
                font_size,
                family,
                weight,
                TruncationViewport::HeadAnchored,
                &mut fs,
            )
            .clone();

        // The narrow-geometry result must be independently cached (separate key).
        assert_eq!(
            cache.len(),
            2,
            "wide and narrow geometries must produce separate cache entries \
             (one for each distinct bounds_width); got {} entries",
            cache.len(),
        );

        // The narrow result must be truncated (39 'A's will not fit in 60px).
        assert!(
            result_narrow.was_truncated,
            "content must be truncated at the intermediate narrow geometry ({narrow_w}px); \
             got: {:?}",
            result_narrow.text,
        );
        assert!(
            result_narrow.text.ends_with(crate::overflow::ELLIPSIS),
            "truncated result at intermediate geometry must end with '…'; got: {:?}",
            result_narrow.text,
        );

        // The narrow result must be strictly shorter than the wide result
        // (or at least differ — the wide result may or may not truncate
        // depending on font metrics, but the narrow one always will).
        if !result_wide.was_truncated {
            assert_ne!(
                result_wide.text, result_narrow.text,
                "wide (untruncated) and narrow (truncated) results must differ \
                 — the narrow intermediate geometry must trigger independent re-resolution"
            );
        }
    }

    /// A sequence of three intermediate geometries each produces an independently
    /// cached result.  The cache holds one entry per distinct bounds, and each
    /// entry reflects the correct truncation for its bounds.
    ///
    /// This validates that the re-truncation is not just at drag-end (the widest
    /// or narrowest bound) but at *every* distinct intermediate geometry the
    /// cache is primed for.
    #[test]
    fn sequence_of_intermediate_geometries_all_independently_cached() {
        let mut fs = FontSystem::new();
        let mut cache = TruncationCache::new();

        // Content long enough to overflow at mid and narrow widths.
        let content = "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW"; // 43 'W's
        let family = FontFamily::SystemSansSerif;
        let weight = 400u16;
        let font_size = 16.0_f32;
        let height = 100.0_f32;

        let widths: [f32; 3] = [500.0, 200.0, 60.0];

        // Prime at each intermediate geometry in order (simulating a resize drag
        // from wide to mid to narrow).
        let mut results = Vec::new();
        for &w in &widths {
            let key = TruncationKey::new(
                content,
                w,
                height,
                font_size,
                family,
                weight,
                TruncationViewport::HeadAnchored,
            );
            let result = cache
                .prime(
                    key,
                    content,
                    w,
                    height,
                    font_size,
                    family,
                    weight,
                    TruncationViewport::HeadAnchored,
                    &mut fs,
                )
                .clone();
            results.push((w, result));
        }

        // Each geometry must have produced a separate cache entry.
        assert_eq!(
            cache.len(),
            3,
            "three distinct intermediate geometries must produce three cache entries; \
             got {} entries",
            cache.len(),
        );

        // The narrowest geometry must be truncated.
        let (narrow_w, ref narrow_result) = results[2];
        assert!(
            narrow_result.was_truncated,
            "content must be truncated at narrowest intermediate geometry ({narrow_w}px)"
        );
        assert!(
            narrow_result.text.ends_with(crate::overflow::ELLIPSIS),
            "narrowest intermediate result must end with '…'; got: {:?}",
            narrow_result.text,
        );

        // The narrow result must be shorter than the widest result (which may
        // not be truncated).  At minimum the texts must differ.
        let (wide_w, ref wide_result) = results[0];
        if !wide_result.was_truncated {
            assert_ne!(
                wide_result.text, narrow_result.text,
                "intermediate geometry at {wide_w}px and {narrow_w}px must produce \
                 different truncation results (widths differ significantly)"
            );
        }
    }

    /// Verify that TruncationKey encodes bounds in the key: the same content
    /// at two different widths produces two different keys (so the cache miss
    /// mechanism that drives intermediate-geometry re-resolution is sound).
    ///
    /// This is the structural guarantee underlying the entire §6b.3 re-truncation
    /// design: the TruncationCache already invalidates naturally on bounds change
    /// because bounds are part of the key.
    #[test]
    fn truncation_key_encodes_bounds_width_distinctly() {
        let content = "any content string";
        let key_a = TruncationKey::new(
            content,
            100.0,
            50.0,
            16.0,
            FontFamily::SystemSansSerif,
            400,
            TruncationViewport::HeadAnchored,
        );
        let key_b = TruncationKey::new(
            content,
            200.0,
            50.0,
            16.0,
            FontFamily::SystemSansSerif,
            400,
            TruncationViewport::HeadAnchored,
        );
        assert_ne!(
            key_a, key_b,
            "TruncationKey must differ when bounds_width differs \
             (required for mid-drag cache-invalidation correctness)"
        );
    }

    /// Verify that TruncationKey encodes bounds height in the key.
    #[test]
    fn truncation_key_encodes_bounds_height_distinctly() {
        let content = "any content string";
        let key_a = TruncationKey::new(
            content,
            200.0,
            50.0,
            16.0,
            FontFamily::SystemSansSerif,
            400,
            TruncationViewport::HeadAnchored,
        );
        let key_b = TruncationKey::new(
            content,
            200.0,
            80.0,
            16.0,
            FontFamily::SystemSansSerif,
            400,
            TruncationViewport::HeadAnchored,
        );
        assert_ne!(
            key_a, key_b,
            "TruncationKey must differ when bounds_height differs \
             (required for mid-drag cache-invalidation correctness)"
        );
    }

    // ── hud-so7zu: styled/color run reslice tests ─────────────────────────────

    /// reslice_styled_runs: runs entirely before content_end_byte are preserved.
    #[test]
    fn reslice_styled_runs_runs_within_range_preserved() {
        let runs = vec![
            StyledRunItem {
                start_byte: 0,
                end_byte: 4,
                weight: Some(700),
                italic: false,
                monospace: false,
                color: None,
            },
            StyledRunItem {
                start_byte: 5,
                end_byte: 10,
                weight: None,
                italic: true,
                monospace: false,
                color: None,
            },
        ];
        // content_end = 10: both runs fit entirely
        let result = reslice_styled_runs(&runs, 10);
        assert_eq!(result.len(), 2, "both runs within range must be preserved");
        assert_eq!(result[0].start_byte, 0);
        assert_eq!(result[0].end_byte, 4);
        assert_eq!(result[1].start_byte, 5);
        assert_eq!(result[1].end_byte, 10);
    }

    /// reslice_styled_runs: runs starting at or beyond content_end_byte are dropped.
    #[test]
    fn reslice_styled_runs_beyond_truncation_dropped() {
        let runs = vec![
            StyledRunItem {
                start_byte: 0,
                end_byte: 3,
                weight: Some(700),
                italic: false,
                monospace: false,
                color: None,
            },
            StyledRunItem {
                start_byte: 5,
                end_byte: 10,
                weight: None,
                italic: false,
                monospace: false,
                color: None,
            },
        ];
        // content_end = 4: second run (5..10) is entirely beyond truncation point
        let result = reslice_styled_runs(&runs, 4);
        assert_eq!(result.len(), 1, "run beyond content_end must be dropped");
        assert_eq!(result[0].start_byte, 0);
        assert_eq!(result[0].end_byte, 3);
    }

    /// reslice_styled_runs: runs partially overlapping content_end_byte are clamped.
    #[test]
    fn reslice_styled_runs_partial_overlap_clamped() {
        let runs = vec![StyledRunItem {
            start_byte: 3,
            end_byte: 12,
            weight: Some(700),
            italic: false,
            monospace: false,
            color: None,
        }];
        // content_end = 8: run 3..12 overlaps; should become 3..8
        let result = reslice_styled_runs(&runs, 8);
        assert_eq!(
            result.len(),
            1,
            "partially overlapping run should be clamped"
        );
        assert_eq!(result[0].start_byte, 3);
        assert_eq!(
            result[0].end_byte, 8,
            "end_byte must be clamped to content_end"
        );
        assert_eq!(result[0].weight, Some(700), "weight must be preserved");
    }

    /// reslice_color_runs: runs entirely before content_end_byte are preserved.
    #[test]
    fn reslice_color_runs_within_range_preserved() {
        let runs = vec![
            ColorRunItem {
                start_byte: 0,
                end_byte: 4,
                color: [255, 0, 0, 255],
            },
            ColorRunItem {
                start_byte: 5,
                end_byte: 9,
                color: [0, 0, 255, 255],
            },
        ];
        let result = reslice_color_runs(&runs, 10);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].end_byte, 4);
        assert_eq!(result[1].end_byte, 9);
    }

    /// reslice_color_runs: runs beyond content_end_byte are dropped.
    #[test]
    fn reslice_color_runs_beyond_truncation_dropped() {
        let runs = vec![
            ColorRunItem {
                start_byte: 0,
                end_byte: 3,
                color: [255, 0, 0, 255],
            },
            ColorRunItem {
                start_byte: 6,
                end_byte: 12,
                color: [0, 255, 0, 255],
            },
        ];
        let result = reslice_color_runs(&runs, 5);
        assert_eq!(
            result.len(),
            1,
            "second run entirely beyond content_end must be dropped"
        );
        assert_eq!(result[0].color, [255, 0, 0, 255]);
    }

    /// reslice_color_runs: partial overlap is clamped to content_end.
    #[test]
    fn reslice_color_runs_partial_overlap_clamped() {
        let runs = vec![ColorRunItem {
            start_byte: 2,
            end_byte: 10,
            color: [0, 128, 255, 255],
        }];
        let result = reslice_color_runs(&runs, 6);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_byte, 2);
        assert_eq!(result[0].end_byte, 6, "end clamped to content_end");
        assert_eq!(result[0].color, [0, 128, 255, 255], "color preserved");
    }

    // ── hud-so7zu: styled_runs_effective_measurement tests ───────────────────

    /// Base case: no styled runs → returns base_weight and no monospace.
    #[test]
    fn styled_runs_effective_measurement_no_runs() {
        let (weight, mono) = styled_runs_effective_measurement(&[], 400);
        assert_eq!(weight, 400, "no runs: effective weight = base_weight");
        assert!(!mono, "no runs: no monospace");
    }

    /// Bold run raises effective weight.
    #[test]
    fn styled_runs_effective_measurement_bold_run_raises_weight() {
        let runs = vec![StyledRunItem {
            start_byte: 0,
            end_byte: 5,
            weight: Some(700),
            italic: false,
            monospace: false,
            color: None,
        }];
        let (weight, mono) = styled_runs_effective_measurement(&runs, 400);
        assert_eq!(weight, 700, "bold run must raise effective weight to 700");
        assert!(!mono);
    }

    /// Monospace run sets use_monospace flag.
    #[test]
    fn styled_runs_effective_measurement_monospace_run_sets_flag() {
        let runs = vec![StyledRunItem {
            start_byte: 0,
            end_byte: 5,
            weight: None,
            italic: false,
            monospace: true,
            color: None,
        }];
        let (weight, mono) = styled_runs_effective_measurement(&runs, 400);
        assert_eq!(
            weight, 400,
            "monospace run without weight: base_weight unchanged"
        );
        assert!(mono, "monospace run must set use_monospace");
    }

    /// Multiple runs: maximum weight is returned.
    #[test]
    fn styled_runs_effective_measurement_max_weight_returned() {
        let runs = vec![
            StyledRunItem {
                start_byte: 0,
                end_byte: 3,
                weight: Some(600),
                italic: false,
                monospace: false,
                color: None,
            },
            StyledRunItem {
                start_byte: 4,
                end_byte: 8,
                weight: Some(800),
                italic: false,
                monospace: false,
                color: None,
            },
            StyledRunItem {
                start_byte: 9,
                end_byte: 12,
                weight: Some(500),
                italic: false,
                monospace: false,
                color: None,
            },
        ];
        let (weight, mono) = styled_runs_effective_measurement(&runs, 400);
        assert_eq!(weight, 800, "maximum weight across runs must be returned");
        assert!(!mono);
    }

    // ── hud-fe4ik: tail-anchored styled-run reslice tests ────────────────────
    //
    // `truncate_tail_anchored` emits LEADING-ellipsis output ("…\n{tail}"),
    // so styled_run byte offsets (relative to original text) must be shifted
    // into the "…\n{tail}" coordinate space.  These tests verify the new
    // `reslice_styled_runs_tail_anchored` and `reslice_color_runs_tail_anchored`
    // helpers, and also confirm that the spans they produce slice the correct
    // substrings from the leading-ellipsis effective text.

    /// Basic shift: a bold run on the visible tail is remapped to effective_text coords.
    ///
    /// Original text:  "line1\nline2\nline3"  (bytes 0..17)
    /// Truncated (tail-anchored): "…\nline2\nline3"
    ///   - "…" = 3 bytes, "\n" = 1 byte → leading_bytes = 4
    ///   - tail_origin = 17 - 12 = 5  ("line1\n" is the dropped prefix, 6 bytes)
    ///
    /// Wait — let's compute carefully:
    ///   "line1\nline2\nline3"
    ///    0123456789012345678
    ///    "line2\nline3" starts at byte 6 (len 12)
    ///    tail_origin = 18 - 12 = 6
    ///
    /// A bold run at [6..10] ("line") in original text should shift to
    ///   new_start = 6 - 6 + 4 = 4
    ///   new_end   = 10 - 6 + 4 = 8
    /// In effective_text "…\nline2\nline3":
    ///   bytes 0..3 = "…", byte 3 = "\n", bytes 4..10 = "line2\n", bytes 10..15 = "line3"
    ///   → [4..8] = "line" ✓ (correct glyphs styled)
    #[test]
    fn reslice_styled_runs_tail_anchored_basic_shift() {
        let original = "line1\nline2\nline3";
        // "line2\nline3" = 12 bytes; starts at byte 6 in original
        let truncated = "…\nline2\nline3";
        // ELLIPSIS = "…" = 3 bytes, + "\n" = 4 bytes leading_bytes
        // tail_origin = 17 - 12 = 5? No: original.len() = 17 actually.
        // "line1\nline2\nline3": l-i-n-e-1 = 5, \n = 1, l-i-n-e-2 = 5, \n = 1, l-i-n-e-3 = 5 → 17 bytes
        // "line2\nline3": l-i-n-e-2 = 5, \n = 1, l-i-n-e-3 = 5 → 11 bytes
        // tail_origin = 17 - 11 = 6 ✓ ("line1\n" = 6 bytes)
        // leading_bytes = 3 + 1 = 4
        // bold run [6..11] ("line2") in original → [6-6+4 .. 11-6+4] = [4..9]
        // In "…\nline2\nline3": bytes [4..9] = "line2" ✓
        let runs = vec![StyledRunItem {
            start_byte: 6,
            end_byte: 11,
            weight: Some(700),
            italic: false,
            monospace: false,
            color: None,
        }];
        let result = reslice_styled_runs_tail_anchored(original, truncated, &runs);
        assert_eq!(
            result.len(),
            1,
            "bold run on visible tail must survive reslice"
        );
        assert_eq!(
            result[0].start_byte, 4,
            "start must be shifted to leading-ellipsis coords"
        );
        assert_eq!(
            result[0].end_byte, 9,
            "end must be shifted to leading-ellipsis coords"
        );
        assert_eq!(result[0].weight, Some(700), "weight preserved");
        // Verify the span slices the correct substring.
        assert_eq!(
            &truncated[result[0].start_byte..result[0].end_byte],
            "line2",
            "resliced run must address 'line2' in the effective text"
        );
    }

    /// Run entirely before the visible tail is dropped.
    #[test]
    fn reslice_styled_runs_tail_anchored_run_before_tail_dropped() {
        let original = "line1\nline2\nline3";
        let truncated = "…\nline2\nline3";
        // Run [0..5] = "line1" is in the dropped prefix → should be dropped.
        let runs = vec![StyledRunItem {
            start_byte: 0,
            end_byte: 5,
            weight: Some(700),
            italic: false,
            monospace: false,
            color: None,
        }];
        let result = reslice_styled_runs_tail_anchored(original, truncated, &runs);
        assert!(
            result.is_empty(),
            "run entirely before tail must be dropped; got: {result:?}"
        );
    }

    /// Run partially overlapping the tail start is clamped to the tail start.
    #[test]
    fn reslice_styled_runs_tail_anchored_partial_overlap_clamped() {
        let original = "line1\nline2\nline3";
        let truncated = "…\nline2\nline3";
        // tail_origin = 6; run [3..9] straddles the tail boundary.
        // Clamped start = 6; end = 9.
        // new_start = 6 - 6 + 4 = 4; new_end = 9 - 6 + 4 = 7.
        // truncated[4..7] = "lin" (start of "line2")
        let runs = vec![StyledRunItem {
            start_byte: 3,
            end_byte: 9,
            weight: Some(700),
            italic: false,
            monospace: false,
            color: None,
        }];
        let result = reslice_styled_runs_tail_anchored(original, truncated, &runs);
        assert_eq!(
            result.len(),
            1,
            "partially overlapping run must survive as clamped"
        );
        assert_eq!(
            result[0].start_byte, 4,
            "clamped start must be at tail start in effective coords"
        );
        assert_eq!(result[0].end_byte, 7);
        assert_eq!(
            &truncated[result[0].start_byte..result[0].end_byte],
            "lin",
            "sliced text must be the overlap region in effective text"
        );
    }

    /// Multiple runs: only runs on the visible tail survive; their offsets are
    /// correct addresses into the leading-ellipsis effective text.
    ///
    /// This is the primary regression test for hud-fe4ik: before the fix,
    /// `content_end` would equal `truncated.len()` (strip_suffix failing on
    /// leading-ellipsis output), no clipping would occur, and the original
    /// offsets (0..5 and 6..11) would be applied verbatim to "…\nline2\nline3".
    /// Byte 0..5 of that string is "…\nli" (not "line1") — wrong glyphs styled.
    #[test]
    fn reslice_styled_runs_tail_anchored_multiple_runs_regression() {
        let original = "line1\nline2\nline3";
        // tail_origin = 6; leading_bytes = 4
        let truncated = "…\nline2\nline3";

        let runs = vec![
            // Run on dropped prefix ("line1") → should be dropped
            StyledRunItem {
                start_byte: 0,
                end_byte: 5,
                weight: Some(700),
                italic: false,
                monospace: false,
                color: None,
            },
            // Run on visible tail ("line2") → should map to [4..9] in effective_text
            StyledRunItem {
                start_byte: 6,
                end_byte: 11,
                weight: Some(400),
                italic: true,
                monospace: false,
                color: Some([255, 0, 0, 255]),
            },
        ];

        let result = reslice_styled_runs_tail_anchored(original, truncated, &runs);
        assert_eq!(
            result.len(),
            1,
            "only the run on the visible tail survives; got {result:?}"
        );
        assert_eq!(
            result[0].start_byte, 4,
            "run on 'line2' must start at byte 4 (after '…\\n') in effective text"
        );
        assert_eq!(
            result[0].end_byte, 9,
            "run on 'line2' must end at byte 9 in effective text"
        );
        assert_eq!(
            &truncated[result[0].start_byte..result[0].end_byte],
            "line2",
            "shifted run must address 'line2' in '…\\nline2\\nline3'"
        );

        // Demonstrate the pre-fix bug: if offsets were NOT shifted, byte
        // range [6..11] in truncated "…\nline2\nline3" addresses "\nline" — wrong.
        // After the fix the range [4..9] correctly addresses "line2".
        assert_ne!(
            &truncated[6..11],
            "line2",
            "unshifted offset 6..11 must NOT address 'line2' in effective text \
             (confirms the pre-fix mis-styling)"
        );
    }

    /// `reslice_color_runs_tail_anchored`: basic color run on visible tail.
    #[test]
    fn reslice_color_runs_tail_anchored_basic_shift() {
        let original = "line1\nline2\nline3";
        let truncated = "…\nline2\nline3";
        // Run [6..11] = "line2" in original → [4..9] in effective_text.
        let runs = vec![ColorRunItem {
            start_byte: 6,
            end_byte: 11,
            color: [0, 128, 255, 255],
        }];
        let result = reslice_color_runs_tail_anchored(original, truncated, &runs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_byte, 4);
        assert_eq!(result[0].end_byte, 9);
        assert_eq!(result[0].color, [0, 128, 255, 255], "color preserved");
        assert_eq!(
            &truncated[result[0].start_byte..result[0].end_byte],
            "line2",
            "color run must address 'line2' in effective text"
        );
    }

    /// `reslice_color_runs_tail_anchored`: run before tail is dropped.
    #[test]
    fn reslice_color_runs_tail_anchored_run_before_tail_dropped() {
        let original = "line1\nline2\nline3";
        let truncated = "…\nline2\nline3";
        let runs = vec![ColorRunItem {
            start_byte: 0,
            end_byte: 5,
            color: [255, 0, 0, 255],
        }];
        let result = reslice_color_runs_tail_anchored(original, truncated, &runs);
        assert!(
            result.is_empty(),
            "color run before tail must be dropped; got {result:?}"
        );
    }

    /// `reslice_styled_runs_tail_anchored` falls back (empty) when first
    /// visible line was further horizontally truncated (no suffix match).
    #[test]
    fn reslice_styled_runs_tail_anchored_first_line_truncated_fallback() {
        let original = "line1\nthis_is_a_very_long_line_2\nline3";
        // First visible line was horizontally truncated: "this_is_a_very…"
        // So truncated_str does NOT end with original text's suffix.
        let truncated = "…\nthis_is_a_very…\nline3";
        let runs = vec![StyledRunItem {
            start_byte: 6,
            end_byte: 15,
            weight: Some(700),
            italic: false,
            monospace: false,
            color: None,
        }];
        // original.ends_with("this_is_a_very…\nline3") is false, so empty is returned.
        let result = reslice_styled_runs_tail_anchored(original, truncated, &runs);
        assert!(
            result.is_empty(),
            "when first line is horizontally truncated, must return empty (unstyled fallback); \
             got {result:?}"
        );
    }
}
