//! Text-collection methods for the compositor.
//!
//! Moved from `renderer/mod.rs` text-collection cluster (formerly ~L3546–4287
//! pre-split), plus `collect_ellipsis_text_items_from_node` from the
//! module-level helpers section, by Step R-4 of the renderer module split
//! (hud-fgryk).  No logic was changed; only visibility modifiers were added
//! where Rust's module-privacy rules require them (listed in the PR body).

use std::collections::HashMap;
use std::sync::Arc;

use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;

use crate::text::TextItem;

use super::icon::parse_notification_icon;
use super::token_colors::{
    ICON_SIZE_PX, ICON_TEXT_GAP_PX, NOTIFICATION_BODY_SCALE, NOTIFICATION_DISMISS_BUTTON_SIZE_PX,
    NOTIFICATION_DISMISS_FONT_SIZE_PX, NOTIFICATION_DISMISS_FONT_WEIGHT,
    NOTIFICATION_DISMISS_GAP_PX, NOTIFICATION_ICON_GAP_PX, NOTIFICATION_ICON_SIZE_PX,
    NOTIFICATION_INTER_LINE_GAP, NOTIFICATION_TITLE_WEIGHT, is_alert_banner_zone,
    notification_dismiss_bounds, resolve_composer_overlay_tokens,
};

/// Default line-height multiplier (`font_size_px × 1.4 = line_height_px`).
///
/// Mirrors [`crate::markdown::MarkdownTokens::default().line_height_multiplier`]
/// (1.4) and is used by all notification/icon-inset `TextItem` builders below so
/// we avoid constructing a default `MarkdownTokens` struct six times per frame.
const DEFAULT_LINE_HEIGHT_MULTIPLIER: f32 = 1.4;

// ─── Text-collection impl block ───────────────────────────────────────────────

impl super::Compositor {
    /// Build per-entry `TextItem`s for a StatusBar zone with icon mappings.
    ///
    /// Called when `RenderingPolicy::key_icon_map` is non-empty.  Each entry
    /// occupies one slot (height = `stack_slot_height(policy)`).  Entries whose
    /// key is mapped to an SVG icon have their text x-origin shifted right by
    /// `ICON_SIZE_PX + ICON_TEXT_GAP_PX` so the icon quad can be rendered to
    /// the left without overlap.  Entries without an icon mapping are rendered
    /// at the unshifted position.
    ///
    /// Returns a unicode emoji prefix for well-known status-bar keys.
    /// Used as a v1 fallback when no SVG icon mapping exists.
    ///
    /// # Parameters
    /// - `key` — the status-bar entry key.
    ///
    /// # Returns
    /// A unicode emoji string, or empty string if key is not recognized.
    fn status_bar_icon_prefix(key: &str) -> &str {
        match key {
            "battery" => "🔋",
            "wifi" | "network" => "📶",
            "cpu" => "🖥",
            "memory" | "mem" => "💾",
            "time" | "clock" => "🕐",
            "weather" => "☀",
            "temperature" | "temp" => "🌡",
            _ => "",
        }
    }

    /// Each entry is a single `TextItem` rendered by delegating to
    /// `TextItem::from_zone_policy` with adjusted `(x, y, w, h)` that place
    /// the text in its row and account for icon inset.
    ///
    /// # Parameters
    /// - `sorted` — entries sorted by key (deterministic row order).
    /// - `key_icon_map` — key → SVG path mapping from `RenderingPolicy`.
    /// - `zx`, `zy`, `zw` — zone pixel bounds (x, y, width).
    /// - `policy` — the zone's `RenderingPolicy`.
    /// - `opacity` — current animation opacity.
    fn status_bar_icon_text_items(
        sorted: &[(&String, &String)],
        key_icon_map: &HashMap<String, String>,
        zx: f32,
        zy: f32,
        zw: f32,
        policy: &RenderingPolicy,
        opacity: f32,
    ) -> Vec<TextItem> {
        const STATUS_BAR_MAX_ENTRIES: usize = 8;
        let slot_h = Self::stack_slot_height(policy);

        sorted
            .iter()
            .take(STATUS_BAR_MAX_ENTRIES)
            .enumerate()
            .map(|(i, (k, v))| {
                let entry_y = zy + i as f32 * slot_h;
                let has_icon = key_icon_map.contains_key(k.as_str());
                let icon_inset = if has_icon {
                    ICON_SIZE_PX + ICON_TEXT_GAP_PX
                } else {
                    0.0
                };
                // Pass adjusted bounds to from_zone_policy so it applies margins
                // on top of the per-entry position and icon inset.
                // Effective pixel_x = (zx + icon_inset) + margin_h
                // Effective bounds_width = (zw - icon_inset) - 2*margin_h
                let text = if has_icon {
                    format!("{k}: {v}")
                } else {
                    let prefix = Self::status_bar_icon_prefix(k.as_str());
                    if prefix.is_empty() {
                        format!("{k}: {v}")
                    } else {
                        format!("{prefix} {k}: {v}")
                    }
                };
                TextItem::from_zone_policy(
                    &text,
                    zx + icon_inset,
                    entry_y,
                    zw - icon_inset,
                    slot_h,
                    policy,
                    opacity,
                )
            })
            .collect()
    }

    /// Build a zone-derived [`TextItem`] with the shared boilerplate filled in.
    ///
    /// All notification and icon-inset `TextItem` literals in
    /// [`Self::collect_text_items`] share four constant tail fields
    /// (`color_runs`, `styled_runs`, `line_height_multiplier`,
    /// `viewport`) and always set `clip_*` to mirror `pixel_*` / `bounds_*`.
    /// This builder centralises those seven fields so each call site only
    /// supplies the twelve fields that actually vary.
    ///
    /// Call sites (all in `collect_text_items`):
    /// - Stack / single-line notification (text = body, `alignment = Start`)
    /// - Stack / two-line title line (`font_weight = notif_title_weight`)
    /// - Stack / two-line body line (`font_size_px = body_font_size`)
    /// - MergeByKey fallback icon-inset notification
    /// - LatestWins/Replace icon-inset notification (identical to MergeByKey)
    // Lint suppressed deliberately: this IS the consolidation the lint would
    // suggest — it already absorbs seven shared fields, leaving only the
    // twelve genuinely-varying `TextItem` fields, each a distinct primitive.
    // Bundling them into a struct would just rename the same flat argument list.
    #[allow(clippy::too_many_arguments)]
    fn make_zone_text_item(
        text: Arc<str>,
        pixel_x: f32,
        pixel_y: f32,
        bounds_width: f32,
        bounds_height: f32,
        font_size_px: f32,
        font_family: FontFamily,
        font_weight: u16,
        color: [u8; 4],
        alignment: TextAlign,
        overflow: TextOverflow,
        outline_color: Option<[u8; 4]>,
        outline_width: Option<f32>,
        opacity: f32,
    ) -> TextItem {
        TextItem {
            text,
            pixel_x,
            pixel_y,
            bounds_width,
            bounds_height,
            clip_pixel_x: pixel_x,
            clip_pixel_y: pixel_y,
            clip_bounds_width: bounds_width,
            clip_bounds_height: bounds_height,
            font_size_px,
            font_family,
            font_weight,
            color,
            alignment,
            overflow,
            outline_color,
            outline_width,
            opacity,
            color_runs: Box::default(),
            styled_runs: Box::default(),
            line_height_multiplier: DEFAULT_LINE_HEIGHT_MULTIPLIER,
            viewport: crate::overflow::TruncationViewport::HeadAnchored,
        }
    }

    /// Build a `TextItem` for `ZoneContent::StreamText`, honouring the zone's
    /// tail-anchor opt-in.
    ///
    /// Delegates geometry, color, and typography to
    /// [`TextItem::from_zone_policy`] (so all visual properties still come from
    /// `RenderingPolicy` — no hardcoded styling), then selects the truncation
    /// viewport:
    ///
    /// - `policy.stream_tail_anchored == Some(true)` →
    ///   [`TruncationViewport::TailAnchored`]: a streaming surface shows the
    ///   **newest** content (the tail), mirroring the transcript portal's
    ///   follow-tail behaviour (spec §3.2).
    /// - otherwise → [`TruncationViewport::HeadAnchored`] (the default;
    ///   preserves the pre-existing behaviour for all zones that have not opted
    ///   in).
    ///
    /// The viewport only changes which side is truncated when overflow resolves
    /// to [`TextOverflow::Ellipsis`]; for `Clip` it is inert.  This is the single
    /// place the zone-vs-portal tail-anchor decision is made for StreamText, so
    /// the per-frame item and any primed cache entry agree on `viewport` (the
    /// truncation key includes it).
    ///
    /// [`TruncationViewport`]: crate::overflow::TruncationViewport
    /// [`TextOverflow`]: tze_hud_scene::types::TextOverflow
    #[allow(clippy::too_many_arguments)]
    pub(super) fn zone_stream_text_item(
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        policy: &RenderingPolicy,
        opacity: f32,
    ) -> TextItem {
        let mut item = TextItem::from_zone_policy(text, x, y, w, h, policy, opacity);
        if policy.stream_tail_anchored == Some(true) {
            item.viewport = crate::overflow::TruncationViewport::TailAnchored;
        }
        item
    }

    /// Collect the `Ellipsis`-overflow `ZoneContent::StreamText` `TextItem`s in
    /// the scene, for off-frame truncation-cache priming.
    ///
    /// Zone StreamText items are not produced by the tile-node walk in
    /// [`Compositor::prime_truncation_cache`], so without this pass an
    /// overflowing streaming zone would hit the render path cold and fall back to
    /// raw clipping (never tail- or head-anchored truncation).  This method
    /// rebuilds the same StreamText items `collect_text_items` emits — identical
    /// geometry, policy, and `viewport` (via [`Self::zone_stream_text_item`]) —
    /// so the per-frame key matches the primed cache entry.
    ///
    /// Only `Ellipsis` items are emitted: `effective_truncation_key` returns
    /// `None` for any other overflow, so priming them would be wasted work.
    /// Opacity is irrelevant to the truncation key (which keys on text, geometry,
    /// font, and viewport only), so this pass uses `1.0`.
    ///
    /// # Word-by-word reveal interaction
    ///
    /// This pass primes the **full** StreamText content.  While a
    /// `LatestWins`/`Replace` zone is mid word-by-word reveal, the render path
    /// keys on the partially-revealed *prefix* (`visible_text`), so the primed
    /// full-text entry will not match until the reveal completes — those frames
    /// fall back to raw clipping (the doctrinal arrival≠presentation graceful
    /// path).  Once the reveal finishes (`visible_text == text`) the keys agree
    /// and the steady-state streaming surface renders correctly anchored.  Reveal
    /// prefixes are short and typically fit without overflow, so this is rarely
    /// observable in practice.
    pub(super) fn collect_zone_stream_text_ellipsis_items(
        &self,
        scene: &SceneGraph,
        sw: f32,
        sh: f32,
    ) -> Vec<TextItem> {
        let mut items: Vec<TextItem> = Vec::new();

        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            let policy = &zone_def.rendering_policy;
            // Truncation (and thus anchoring) only applies to Ellipsis overflow.
            if policy.overflow != Some(TextOverflow::Ellipsis) {
                continue;
            }

            let (zx, zy, zw, zh) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);

            match zone_def.contention_policy {
                ContentionPolicy::Stack { .. } => {
                    let layout = self.zone_slot_layout(zone_name, publishes, policy, zh);
                    for (pub_idx, slot_y, effective_slot_h) in layout.iter_visible(zy) {
                        if let ZoneContent::StreamText(text) = &publishes[pub_idx].content {
                            items.push(Self::zone_stream_text_item(
                                text,
                                zx,
                                slot_y,
                                zw,
                                effective_slot_h,
                                policy,
                                1.0,
                            ));
                        }
                    }
                }
                ContentionPolicy::MergeByKey { .. }
                | ContentionPolicy::LatestWins
                | ContentionPolicy::Replace => {
                    // Only the most-recent StreamText publish renders (and is primed).
                    for record in publishes.iter().rev() {
                        if let ZoneContent::StreamText(text) = &record.content {
                            items.push(Self::zone_stream_text_item(
                                text, zx, zy, zw, zh, policy, 1.0,
                            ));
                            break;
                        }
                    }
                }
            }
        }

        items
    }

    /// Collect `TextItem`s for all TextMarkdownNode tiles and zone StreamText
    /// and ShortTextWithIcon/Notification content in the scene.
    ///
    /// All zone `TextItem`s are constructed from `RenderingPolicy` fields —
    /// no hardcoded colors or font choices.  Animation opacity is applied to
    /// the color channels so text fades with the backdrop.
    ///
    /// Returns a flat `Vec<TextItem>` ready for `TextRasterizer::prepare_text_items`.
    pub(super) fn collect_text_items(&self, scene: &SceneGraph, sw: f32, sh: f32) -> Vec<TextItem> {
        let mut items: Vec<TextItem> = Vec::new();

        // ── TextMarkdownNode tiles ────────────────────────────────────────────
        for tile in &Self::sort_tiles_with_drag_boost(scene.visible_tiles(), scene) {
            if let Some(root_id) = tile.root_node {
                // Compute scroll offset once per tile and pass it down so text
                // glyph positions track the scrolled content (Bounded Transcript
                // Viewport requirement — hud-w5ih).
                let (scroll_x, scroll_y) = self.display_tile_scroll_offset(scene, tile.id);
                // Determine follow-tail state so Ellipsis TextItems receive the
                // correct TruncationViewport (TailAnchored vs HeadAnchored).
                // Uses the shared helper that prime_truncation_cache /
                // collect_ellipsis_text_items_from_node also calls, so the
                // per-frame key always matches the primed cache entry (hud-lu50e,
                // hud-plz8q).
                let at_tail = super::tile_at_tail_for_ellipsis(tile.id, scene);

                // §6.3 portal transition: track item count before to apply
                // portal animation opacity to newly added items.
                let items_before = items.len();
                self.collect_text_items_from_node(
                    root_id, tile, scene, scroll_x, scroll_y, at_tail, &mut items,
                );

                // Apply portal tile animation opacity (§6.3 transition tokens).
                // Only scrollable tiles (portal tiles) have animation state;
                // all others return 1.0 from portal_tile_anim_opacity.
                let portal_anim = self.portal_tile_anim_opacity(tile.id);
                if portal_anim < 1.0 {
                    for item in &mut items[items_before..] {
                        item.opacity *= portal_anim;
                    }
                }

                // Per-segment streaming-reveal fade (hud-bl7yi): while a portal
                // tile's newly-appended content is revealing, ramp the leading
                // segment's glyph alpha (StreamFadeRamp) instead of snapping it
                // in. Steady tiles (`is_revealing() == false`) are untouched, so
                // their draw output is byte-identical to the no-reveal path.
                if let Some(reveal) = self.portal_tile_reveal_states.get(&tile.id) {
                    if reveal.is_revealing() {
                        let ramp = super::easing::StreamFadeRamp::default();
                        for item in &mut items[items_before..] {
                            apply_portal_reveal_fade(item, reveal, ramp);
                        }
                    }
                }
            }
        }

        // ── Zone StreamText, Notification, and StatusBar content ─────────────
        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };

            // Resolve zone geometry to pixel bounds.
            let (zx, zy, zw, zh) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);

            let policy = &zone_def.rendering_policy;

            // Current animation opacity for this zone.
            let anim_opacity = self
                .zone_animation_states
                .get(zone_name)
                .map(|s| s.current_opacity())
                .unwrap_or(1.0);

            // Emit TextItems based on contention policy.
            //
            // Stack: each publication occupies a vertically-stacked slot.
            //   For generic Stack zones: newest at top (slot 0 = newest).
            //   For alert-banner zones: severity-descending (critical first),
            //   then recency-descending (newer first) within the same severity.
            //   Per-slot heights are computed via Self::per_slot_heights() and may
            //   vary by item (two-line notifications occupy a taller slot).
            //   Dynamic height: alert-banner zone height = sum(slot_heights).
            //
            // MergeByKey: collect ALL StatusBar publications, merge their entries
            //   (last write wins per key), render the merged set as one text item.
            //
            // LatestWins / Replace: render only the most-recent publication.
            match zone_def.contention_policy {
                ContentionPolicy::Stack { .. } => {
                    // Slot geometry is computed once by zone_slot_layout and shared
                    // with render_zone_content / collect_all_rounded_rect_cmds (hud-qlerb).
                    let layout = self.zone_slot_layout(zone_name, publishes, policy, zh);

                    // Resolve notification typography tokens needed for text rendering
                    // (not slot geometry — those live in zone_slot_layout).
                    let notif_body_scale = self
                        .token_map
                        .get("typography.notification.body.scale")
                        .and_then(|v| v.parse::<f32>().ok())
                        .filter(|v| v.is_finite())
                        .unwrap_or(NOTIFICATION_BODY_SCALE)
                        .clamp(0.5, 1.0);
                    let notif_title_weight = self
                        .token_map
                        .get("typography.notification.title.weight")
                        .and_then(|v| v.parse::<u16>().ok())
                        .unwrap_or(NOTIFICATION_TITLE_WEIGHT);
                    let notif_dismiss_font_size_px = self
                        .token_map
                        .get("typography.notification.dismiss.font_size_px")
                        .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
                        .unwrap_or(NOTIFICATION_DISMISS_FONT_SIZE_PX)
                        .clamp(6.0, 200.0);
                    let notif_dismiss_font_weight = self
                        .token_map
                        .get("typography.notification.dismiss.font_weight")
                        .and_then(|v| v.parse::<u16>().ok())
                        .unwrap_or(NOTIFICATION_DISMISS_FONT_WEIGHT);

                    for (pub_idx, slot_y, effective_slot_h) in layout.iter_visible(zy) {
                        let record = &publishes[pub_idx];

                        // Per-publication fade-out opacity (1.0 when no fade active).
                        let pub_opacity = self.pub_opacity(zone_name, record);
                        // Combined opacity: zone animation × per-publication fade.
                        let effective_opacity = anim_opacity * pub_opacity;

                        match &record.content {
                            ZoneContent::StreamText(text) => {
                                items.push(Self::zone_stream_text_item(
                                    text,
                                    zx,
                                    slot_y,
                                    zw,
                                    effective_slot_h,
                                    policy,
                                    effective_opacity,
                                ));
                            }
                            ZoneContent::Notification(payload) => {
                                // Notification text rendering priority:
                                // 1. RenderingPolicy explicit values (set by zone configuration)
                                // 2. Token-resolved values (typography.body.size, color.text.primary)
                                // 3. Hardcoded defaults (16px, near-white)
                                //
                                // This allows alert-banner and other configured zones to override
                                // the defaults via RenderingPolicy while notification-area falls
                                // through to the token/default path.
                                //
                                // Layout: left-aligned with 9px inset (8px padding + 1px border),
                                // clips at content area boundary (no wrapping in v1).
                                // When a valid icon texture is cached, the text is additionally
                                // inset to the right of the icon (icon_size + gap).
                                // Spec-defined inset for notification-area zones:
                                // 1px border + 8px padding = 9px from backdrop edges.
                                // When the zone's RenderingPolicy explicitly sets
                                // margin_horizontal/margin_vertical, those values take
                                // precedence (e.g. alert-banner uses margin_horizontal=8).
                                const NOTIFICATION_BORDER_PX: f32 = 1.0;
                                const NOTIFICATION_PADDING_PX: f32 = 8.0;
                                const NOTIFICATION_INSET: f32 =
                                    NOTIFICATION_BORDER_PX + NOTIFICATION_PADDING_PX;
                                // Horizontal inset: policy margin_horizontal > margin_px > 9px
                                let inset_h = policy
                                    .margin_horizontal
                                    .or(policy.margin_px)
                                    .unwrap_or(NOTIFICATION_INSET);
                                // Vertical inset: policy margin_vertical > margin_px > 9px
                                let inset_v = policy
                                    .margin_vertical
                                    .or(policy.margin_px)
                                    .unwrap_or(NOTIFICATION_INSET);
                                // If a cached icon texture exists, inset text to the right of it.
                                let icon_width_reserved = parse_notification_icon(&payload.icon)
                                    .filter(|id| self.image_texture_cache.contains_key(id))
                                    .map(|_| NOTIFICATION_ICON_SIZE_PX + NOTIFICATION_ICON_GAP_PX)
                                    .unwrap_or(0.0);
                                // Font size: policy explicit > typography.body.size token > 16px
                                let font_size_px = policy.font_size_px.unwrap_or_else(|| {
                                    Self::resolve_body_font_size(&self.token_map)
                                });
                                // Font family: policy explicit > SystemSansSerif default
                                let font_family = policy
                                    .font_family
                                    .unwrap_or(tze_hud_scene::types::FontFamily::SystemSansSerif);
                                // Text color: policy explicit > color.text.primary token > near-white
                                let base_color = policy
                                    .text_color
                                    .map(crate::text::rgba_to_srgb_u8)
                                    .unwrap_or_else(|| {
                                        Self::resolve_text_primary_color(&self.token_map)
                                    });
                                let color = crate::text::apply_opacity_to_color(
                                    base_color,
                                    effective_opacity,
                                );
                                // Outline: use policy outline if set, otherwise
                                // derive from policy (ensures legibility on
                                // light-colored backdrops like warning/amber).
                                let (oc, ow) = match (policy.outline_color, policy.outline_width) {
                                    (Some(oc), Some(ow)) if ow > 0.0 => {
                                        let oc_srgb = crate::text::apply_opacity_to_color(
                                            crate::text::rgba_to_srgb_u8(oc),
                                            effective_opacity,
                                        );
                                        (Some(oc_srgb), Some(ow))
                                    }
                                    _ => (None, None),
                                };

                                let dismiss_reserved_w = if is_alert_banner_zone(zone_name) {
                                    0.0
                                } else {
                                    NOTIFICATION_DISMISS_BUTTON_SIZE_PX
                                        + NOTIFICATION_DISMISS_GAP_PX
                                };
                                // Text x-offset respects icon reservation (from icon texture pipeline).
                                let text_x = zx + inset_h + icon_width_reserved;
                                let text_w = (zw
                                    - inset_h
                                    - icon_width_reserved
                                    - inset_h
                                    - dismiss_reserved_w)
                                    .max(1.0);

                                if payload.title.is_empty() {
                                    // ── Single-line rendering (backward-compatible) ──
                                    // Font weight: policy explicit > 400 default
                                    let font_weight = policy.font_weight.unwrap_or(400);
                                    items.push(Self::make_zone_text_item(
                                        Arc::from(payload.text.as_str()),
                                        text_x,
                                        slot_y + inset_v,
                                        text_w,
                                        (effective_slot_h - inset_v * 2.0).max(1.0),
                                        font_size_px,
                                        font_family,
                                        font_weight,
                                        color,
                                        tze_hud_scene::types::TextAlign::Start,
                                        tze_hud_scene::types::TextOverflow::Clip,
                                        oc,
                                        ow,
                                        effective_opacity,
                                    ));
                                } else {
                                    // ── Two-line rendering: bold title + regular body ──
                                    //
                                    // Use pre-resolved token values (notif_title_weight,
                                    // notif_body_scale) to keep rendering consistent with
                                    // the slot-height calculation above.
                                    let body_font_size = font_size_px * notif_body_scale;
                                    let title_line_h = font_size_px * 1.4;
                                    let content_top = slot_y + inset_v;

                                    // Title line (bold)
                                    items.push(Self::make_zone_text_item(
                                        Arc::from(payload.title.as_str()),
                                        text_x,
                                        content_top,
                                        text_w,
                                        title_line_h.max(1.0),
                                        font_size_px,
                                        font_family,
                                        notif_title_weight,
                                        color,
                                        tze_hud_scene::types::TextAlign::Start,
                                        tze_hud_scene::types::TextOverflow::Clip,
                                        oc,
                                        ow,
                                        effective_opacity,
                                    ));
                                    // Body line (regular weight, 0.85× size)
                                    let body_top =
                                        content_top + title_line_h + NOTIFICATION_INTER_LINE_GAP;
                                    // Remaining slot height available for body (down to inset bottom)
                                    let body_bounds_h =
                                        ((slot_y + effective_slot_h - inset_v) - body_top).max(1.0);
                                    items.push(Self::make_zone_text_item(
                                        Arc::from(payload.text.as_str()),
                                        text_x,
                                        body_top,
                                        text_w,
                                        body_bounds_h,
                                        body_font_size,
                                        font_family,
                                        400,
                                        color,
                                        tze_hud_scene::types::TextAlign::Start,
                                        tze_hud_scene::types::TextOverflow::Clip,
                                        oc,
                                        ow,
                                        effective_opacity,
                                    ));
                                }

                                if !is_alert_banner_zone(zone_name)
                                    && self.text_rasterizer.is_some()
                                {
                                    let dismiss_bounds = notification_dismiss_bounds(
                                        zx,
                                        slot_y,
                                        zw,
                                        effective_slot_h,
                                    );
                                    let dismiss_color = crate::text::apply_opacity_to_color(
                                        base_color,
                                        effective_opacity,
                                    );
                                    static DISMISS_LABEL: std::sync::OnceLock<Arc<str>> =
                                        std::sync::OnceLock::new();
                                    items.push(Self::make_zone_text_item(
                                        Arc::clone(DISMISS_LABEL.get_or_init(|| Arc::from("X"))),
                                        dismiss_bounds.x,
                                        dismiss_bounds.y + 1.0,
                                        dismiss_bounds.width.max(1.0),
                                        dismiss_bounds.height.max(1.0),
                                        notif_dismiss_font_size_px,
                                        font_family,
                                        notif_dismiss_font_weight,
                                        dismiss_color,
                                        tze_hud_scene::types::TextAlign::Center,
                                        tze_hud_scene::types::TextOverflow::Clip,
                                        None,
                                        None,
                                        effective_opacity,
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                ContentionPolicy::MergeByKey { max_keys } => {
                    // Collect all StatusBar publications and merge their entries.
                    // For each key, the last publish wins (latest value).
                    let mut merged: HashMap<String, String> =
                        HashMap::with_capacity(max_keys as usize);
                    for record in publishes.iter() {
                        if let ZoneContent::StatusBar(payload) = &record.content {
                            for (k, v) in &payload.entries {
                                merged.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    if !merged.is_empty() {
                        let mut sorted: Vec<(&String, &String)> = merged.iter().collect();
                        sorted.sort_by_key(|(k, _)| k.as_str());
                        // Use per-entry layout only when key_icon_map is non-empty AND
                        // at least one current entry key has an icon mapping.  This
                        // avoids switching layout when the map is configured but none
                        // of the currently displayed entries are mapped.
                        let use_icon_layout = !policy.key_icon_map.is_empty()
                            && sorted
                                .iter()
                                .any(|(k, _)| policy.key_icon_map.contains_key(k.as_str()));
                        if !use_icon_layout {
                            // No icons for current entries: render all as a single
                            // newline-joined TextItem (existing behavior).
                            let text = sorted
                                .iter()
                                .map(|(k, v)| format!("{k}: {v}"))
                                .collect::<Vec<_>>()
                                .join("\n");
                            items.push(TextItem::from_zone_policy(
                                &text,
                                zx,
                                zy,
                                zw,
                                zh,
                                policy,
                                anim_opacity,
                            ));
                        } else {
                            // Icons configured for current entries: render each as an
                            // individual TextItem, with text x-inset when an icon is mapped.
                            items.extend(Self::status_bar_icon_text_items(
                                &sorted,
                                &policy.key_icon_map,
                                zx,
                                zy,
                                zw,
                                policy,
                                anim_opacity,
                            ));
                        }
                    } else {
                        // Fallback: render whatever the latest publication contains.
                        for record in publishes.iter().rev() {
                            match &record.content {
                                ZoneContent::StreamText(text) => {
                                    items.push(Self::zone_stream_text_item(
                                        text,
                                        zx,
                                        zy,
                                        zw,
                                        zh,
                                        policy,
                                        anim_opacity,
                                    ));
                                    break;
                                }
                                ZoneContent::Notification(payload) => {
                                    // MergeByKey fallback: icon-aware inset, same logic
                                    // as the LatestWins/Replace path.
                                    let icon_width_reserved =
                                        parse_notification_icon(&payload.icon)
                                            .filter(|id| self.image_texture_cache.contains_key(id))
                                            .map(|_| {
                                                NOTIFICATION_ICON_SIZE_PX + NOTIFICATION_ICON_GAP_PX
                                            })
                                            .unwrap_or(0.0);
                                    if icon_width_reserved > 0.0 {
                                        // Use the same default inset as the icon draw path
                                        // (NOTIFICATION_INSET = 9px) so text and icon stay
                                        // aligned when policy margins are not explicitly set.
                                        const NOTIFICATION_INSET: f32 = 9.0;
                                        let margin_h = policy
                                            .margin_horizontal
                                            .or(policy.margin_px)
                                            .unwrap_or(NOTIFICATION_INSET);
                                        let margin_v = policy
                                            .margin_vertical
                                            .or(policy.margin_px)
                                            .unwrap_or(NOTIFICATION_INSET);
                                        let font_size_px =
                                            policy.font_size_px.unwrap_or(16.0).clamp(6.0, 200.0);
                                        let font_family = policy.font_family.unwrap_or(
                                            tze_hud_scene::types::FontFamily::SystemSansSerif,
                                        );
                                        let font_weight =
                                            policy.font_weight.unwrap_or(400).clamp(100, 900);
                                        let base_color = policy
                                            .text_color
                                            .map(crate::text::rgba_to_srgb_u8)
                                            .unwrap_or([255, 255, 255, 220]);
                                        let color = crate::text::apply_opacity_to_color(
                                            base_color,
                                            anim_opacity,
                                        );
                                        let (oc, ow) =
                                            match (policy.outline_color, policy.outline_width) {
                                                (Some(oc), Some(ow)) if ow > 0.0 => {
                                                    let oc_srgb =
                                                        crate::text::apply_opacity_to_color(
                                                            crate::text::rgba_to_srgb_u8(oc),
                                                            anim_opacity,
                                                        );
                                                    (Some(oc_srgb), Some(ow))
                                                }
                                                _ => (None, None),
                                            };
                                        items.push(Self::make_zone_text_item(
                                            Arc::from(payload.text.as_str()),
                                            zx + margin_h + icon_width_reserved,
                                            zy + margin_v,
                                            (zw - margin_h - icon_width_reserved - margin_h)
                                                .max(1.0),
                                            (zh - margin_v * 2.0).max(1.0),
                                            font_size_px,
                                            font_family,
                                            font_weight,
                                            color,
                                            policy
                                                .text_align
                                                .unwrap_or(tze_hud_scene::types::TextAlign::Start),
                                            policy.overflow.unwrap_or(
                                                tze_hud_scene::types::TextOverflow::Clip,
                                            ),
                                            oc,
                                            ow,
                                            anim_opacity,
                                        ));
                                    } else {
                                        items.push(TextItem::from_zone_policy(
                                            &payload.text,
                                            zx,
                                            zy,
                                            zw,
                                            zh,
                                            policy,
                                            anim_opacity,
                                        ));
                                    }
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                ContentionPolicy::LatestWins | ContentionPolicy::Replace => {
                    // Use the most-recent publish only.
                    for record in publishes.iter().rev() {
                        match &record.content {
                            ZoneContent::StreamText(text) => {
                                // Apply streaming word-by-word reveal if active.
                                // The reveal state truncates the visible text to a
                                // byte offset (breakpoint boundary).  When no reveal
                                // state exists, the full text is rendered immediately.
                                let visible_text: &str = if let Some(state) =
                                    self.stream_reveal_states.get(zone_name)
                                {
                                    let offset = state.visible_byte_offset();
                                    if offset == usize::MAX {
                                        text.as_str()
                                    } else {
                                        // Clamp to string length, then walk backward to
                                        // a valid UTF-8 character boundary.  Breakpoints
                                        // come from external input and may not be on a
                                        // char boundary; slicing at a non-boundary panics.
                                        let mut safe_offset = offset.min(text.len());
                                        while safe_offset > 0 && !text.is_char_boundary(safe_offset)
                                        {
                                            safe_offset -= 1;
                                        }
                                        &text[..safe_offset]
                                    }
                                } else {
                                    text.as_str()
                                };
                                items.push(Self::zone_stream_text_item(
                                    visible_text,
                                    zx,
                                    zy,
                                    zw,
                                    zh,
                                    policy,
                                    anim_opacity,
                                ));
                                // Only render the most-recent StreamText publish.
                                break;
                            }
                            ZoneContent::Notification(payload) => {
                                // Render the notification text.
                                // When a valid icon texture is cached, inset the text
                                // to the right of the icon (icon_size + gap px).
                                let icon_width_reserved = parse_notification_icon(&payload.icon)
                                    .filter(|id| self.image_texture_cache.contains_key(id))
                                    .map(|_| NOTIFICATION_ICON_SIZE_PX + NOTIFICATION_ICON_GAP_PX)
                                    .unwrap_or(0.0);
                                if icon_width_reserved > 0.0 {
                                    // Build TextItem manually to apply icon offset.
                                    // Use the same default inset as the icon draw path
                                    // (NOTIFICATION_INSET = 9px) so text and icon stay
                                    // aligned when policy margins are not explicitly set.
                                    const NOTIFICATION_INSET: f32 = 9.0;
                                    let margin_h = policy
                                        .margin_horizontal
                                        .or(policy.margin_px)
                                        .unwrap_or(NOTIFICATION_INSET);
                                    let margin_v = policy
                                        .margin_vertical
                                        .or(policy.margin_px)
                                        .unwrap_or(NOTIFICATION_INSET);
                                    let font_size_px =
                                        policy.font_size_px.unwrap_or(16.0).clamp(6.0, 200.0);
                                    let font_family = policy.font_family.unwrap_or(
                                        tze_hud_scene::types::FontFamily::SystemSansSerif,
                                    );
                                    let font_weight =
                                        policy.font_weight.unwrap_or(400).clamp(100, 900);
                                    let base_color = policy
                                        .text_color
                                        .map(crate::text::rgba_to_srgb_u8)
                                        .unwrap_or([255, 255, 255, 220]);
                                    let color = crate::text::apply_opacity_to_color(
                                        base_color,
                                        anim_opacity,
                                    );
                                    let (oc, ow) =
                                        match (policy.outline_color, policy.outline_width) {
                                            (Some(oc), Some(ow)) if ow > 0.0 => {
                                                let oc_srgb = crate::text::apply_opacity_to_color(
                                                    crate::text::rgba_to_srgb_u8(oc),
                                                    anim_opacity,
                                                );
                                                (Some(oc_srgb), Some(ow))
                                            }
                                            _ => (None, None),
                                        };
                                    items.push(Self::make_zone_text_item(
                                        Arc::from(payload.text.as_str()),
                                        zx + margin_h + icon_width_reserved,
                                        zy + margin_v,
                                        (zw - margin_h - icon_width_reserved - margin_h).max(1.0),
                                        (zh - margin_v * 2.0).max(1.0),
                                        font_size_px,
                                        font_family,
                                        font_weight,
                                        color,
                                        policy
                                            .text_align
                                            .unwrap_or(tze_hud_scene::types::TextAlign::Start),
                                        policy
                                            .overflow
                                            .unwrap_or(tze_hud_scene::types::TextOverflow::Clip),
                                        oc,
                                        ow,
                                        anim_opacity,
                                    ));
                                } else {
                                    items.push(TextItem::from_zone_policy(
                                        &payload.text,
                                        zx,
                                        zy,
                                        zw,
                                        zh,
                                        policy,
                                        anim_opacity,
                                    ));
                                }
                                // Only render the most-recent publish.
                                break;
                            }
                            ZoneContent::StatusBar(payload) => {
                                // Format key-value pairs as "key: value" lines, sorted by key
                                // for deterministic output.
                                let mut sorted: Vec<(&String, &String)> =
                                    payload.entries.iter().collect();
                                sorted.sort_by_key(|(k, _)| k.as_str());
                                // Use per-entry layout only when key_icon_map is non-empty AND
                                // at least one current entry key has an icon mapping.
                                let use_icon_layout = !policy.key_icon_map.is_empty()
                                    && sorted
                                        .iter()
                                        .any(|(k, _)| policy.key_icon_map.contains_key(k.as_str()));
                                if !use_icon_layout {
                                    // No icons for current entries: single newline-joined TextItem.
                                    let text = sorted
                                        .iter()
                                        .map(|(k, v)| format!("{k}: {v}"))
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    items.push(TextItem::from_zone_policy(
                                        &text,
                                        zx,
                                        zy,
                                        zw,
                                        zh,
                                        policy,
                                        anim_opacity,
                                    ));
                                } else {
                                    // Icons configured for current entries: per-entry TextItems.
                                    items.extend(Self::status_bar_icon_text_items(
                                        &sorted,
                                        &policy.key_icon_map,
                                        zx,
                                        zy,
                                        zw,
                                        policy,
                                        anim_opacity,
                                    ));
                                }
                                // Only render the most-recent StatusBar publish.
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // ── Composer echo text (hud-r3ax6) ───────────────────────────────────
        // Inject a TextItem for the local composer draft + caret glyph on top of
        // the composer-active tile.  The background geometry is handled in
        // render_composer_overlay / render_frame tile loop.
        if self.local_composer.is_some() {
            let composer_tokens = resolve_composer_overlay_tokens(&self.token_map);
            for tile in &Self::sort_tiles_with_drag_boost(scene.visible_tiles(), scene) {
                if let Some(text_item) =
                    self.collect_composer_text_item(tile, scene, sw, sh, &composer_tokens)
                {
                    items.push(text_item);
                    // Only the first matching tile renders the composer (focus is exclusive).
                    break;
                }
            }
        }

        items
    }

    /// Resolve `color.text.primary` from the token map as a sRGB u8 color.
    ///
    /// Falls back to near-white (R=255, G=255, B=255, A=223) when the token
    /// is absent — matching the canonical default for text on dark backgrounds.
    pub(super) fn resolve_text_primary_color(token_map: &HashMap<String, String>) -> [u8; 4] {
        super::token_colors::resolve_token_color(token_map, "color.text.primary")
            .map(crate::text::rgba_to_srgb_u8)
            .unwrap_or([255, 255, 255, 223]) // near-white, alpha ≈ 87.5%
    }

    /// Resolve `typography.body.size` from the token map as a pixel value.
    ///
    /// Falls back to 16.0 px (canonical body text default) when the token is
    /// absent or cannot be parsed as a number.
    pub(super) fn resolve_body_font_size(token_map: &HashMap<String, String>) -> f32 {
        token_map
            .get("typography.body.size")
            .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(16.0)
            .clamp(6.0, 200.0)
    }

    /// Recursively collect `TextItem`s from a node and its children.
    ///
    /// `at_tail`: whether the owning tile is currently in follow-tail/at-tail
    /// mode.  When `true` and the node uses `TextOverflow::Ellipsis`, the
    /// resulting `TextItem`'s viewport is set to `TailAnchored` so the
    /// per-frame truncation key matches the primed cache entry (hud-lu50e).
    #[allow(clippy::only_used_in_recursion)]
    // All arguments are required: node identity, tile geometry, scene reference,
    // scroll offsets, tail-follow state, and the output accumulator.
    // This mirrors the parameter shape of the free-function twin
    // `collect_ellipsis_text_items_from_node` which also carries the same lint.
    #[allow(clippy::too_many_arguments)]
    fn collect_text_items_from_node(
        &self,
        node_id: SceneId,
        tile: &Tile,
        scene: &SceneGraph,
        scroll_x: f32,
        scroll_y: f32,
        at_tail: bool,
        items: &mut Vec<TextItem>,
    ) {
        let node = match scene.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };

        if let NodeData::TextMarkdown(tm) = &node.data {
            // Subtract scroll offset so text glyphs move with the scrolled
            // content — matches the geometry pass in `render_node` which already
            // applies `tile.bounds.x - scroll_x` / `tile.bounds.y - scroll_y`.
            //
            // Phase-1 (hud-5jbra.2): try the markdown cache first.  Use
            // `get_by_key` with a precomputed key (O(1) — no re-hash on the
            // frame path).  The cache is primed by `prime_markdown_cache` on
            // every scene-version change (commit-time prime, hud-380dl).
            //
            // color_runs bypass: `from_text_markdown_cached` uses markdown
            // plain_text as the text base and discards `node.color_runs`.
            // `color_runs` byte offsets are against the *raw* content (not the
            // stripped plain_text), so the cache path is incompatible.  Skip
            // it when the node carries inline color_runs and fall through to
            // `from_text_markdown_node` which preserves them correctly.
            //
            // Key lookup: the per-node key cache (node_key_cache) is populated
            // by prime_markdown_cache at content-commit time, so the frame path
            // never calls MarkdownCache::compute_key (which hashes the full
            // content string).  The lookup is a 32-byte HashMap read — O(1).
            // If the key is absent (first frame before any prime, or a node
            // added mid-frame) the fallback re-computes the key once and
            // consults the markdown_cache directly.  (hud-gpqde)
            //
            // Cache-miss fallback (hud-xcp9b): if the markdown cache is cold
            // (first frame before any commit-time prime, or a node added
            // mid-frame after prime_markdown_cache ran), we parse the content
            // inline on the spot and use from_text_markdown_cached so markdown
            // structure and styling are preserved.  This honors the 'never
            // dropped' contract (spec task 2.2).  A tracing::warn! fires so the
            // miss is observable in production logs.  No debug_assert: the
            // first-frame miss is normal operation, not a bug. [hud-rbf91]
            // In steady-state (commit-time-primed) frames this branch is never taken.
            //
            // Gate on *pixel-bearing* runs, not `is_empty()`: zero-length
            // sentinel runs (lifecycle/stale/at-capacity markers) reference no
            // content bytes, so a node carrying only sentinels stays on the
            // cached/styled markdown path instead of dropping to the lossy
            // raw-content constructor.  Only genuine pixel runs (start < end)
            // force the legacy path. (hud-9v3t6)
            let mut item = if !crate::text::markdown_node_has_pixel_runs(tm) {
                let content_key = self
                    .node_key_cache
                    .get(&node_id)
                    .copied()
                    .unwrap_or_else(|| crate::markdown::MarkdownCache::compute_key(&tm.content));
                // Load the current snapshot lock-free (hud-33qo7).  Pinned by the
                // returned Arc for the duration of this lookup.
                let markdown_cache = self.markdown_cache();
                if let Some(parsed) = markdown_cache.get_by_key(&content_key) {
                    TextItem::from_text_markdown_cached(
                        tm,
                        tile.bounds.x - scroll_x,
                        tile.bounds.y - scroll_y,
                        parsed,
                    )
                } else {
                    // Cache miss: parse inline (non-lossy) so styling is
                    // preserved.  This is the expected path on the first frame
                    // before any commit-time prime and for nodes added mid-frame
                    // after prime_markdown_cache ran — both are normal operation.
                    // In steady-state (commit-time-primed) frames this branch is
                    // never taken.  The warn! makes the miss observable in
                    // production logs without panicking. [hud-rbf91]
                    tracing::warn!(
                        node_id = ?node_id,
                        content_len = tm.content.len(),
                        "markdown cache miss on render path — expected commit-time prime \
                         (hud-xcp9b); parsing inline to preserve styling [hud-380dl]"
                    );
                    let parsed =
                        crate::markdown::parse_markdown_subset(&tm.content, &self.markdown_tokens);
                    TextItem::from_text_markdown_cached(
                        tm,
                        tile.bounds.x - scroll_x,
                        tile.bounds.y - scroll_y,
                        &parsed,
                    )
                }
            } else {
                // Pixel-bearing color_runs present: use the legacy path that
                // preserves raw content byte offsets.  The markdown cache is
                // intentionally bypassed here.
                TextItem::from_text_markdown_node(
                    tm,
                    tile.bounds.x - scroll_x,
                    tile.bounds.y - scroll_y,
                )
            };
            // Override viewport for at-tail Ellipsis tiles (hud-lu50e).
            // The prime path (collect_ellipsis_text_items_from_node) already
            // primes TailAnchored keys for these tiles; the per-frame key must
            // match or `prepare_text_items` will miss the primed entry and fall
            // back to inline head-anchored truncation — always showing the head
            // of the content instead of the newest lines.
            if at_tail && tm.overflow == TextOverflow::Ellipsis {
                item.viewport = crate::overflow::TruncationViewport::TailAnchored;
            }
            let unscrolled_x = item.pixel_x + scroll_x;
            let unscrolled_y = item.pixel_y + scroll_y;
            let clip_left = unscrolled_x.max(tile.bounds.x);
            let clip_top = unscrolled_y.max(tile.bounds.y);
            let clip_right =
                (unscrolled_x + item.bounds_width).min(tile.bounds.x + tile.bounds.width);
            let clip_bottom =
                (unscrolled_y + item.bounds_height).min(tile.bounds.y + tile.bounds.height);
            item.clip_pixel_x = clip_left;
            item.clip_pixel_y = clip_top;
            item.clip_bounds_width = (clip_right - clip_left).max(1.0);
            item.clip_bounds_height = (clip_bottom - clip_top).max(1.0);
            items.push(item);
        }

        for child_id in &node.children {
            self.collect_text_items_from_node(
                *child_id, tile, scene, scroll_x, scroll_y, at_tail, items,
            );
        }
    }
}

// ─── Module-level text helpers ────────────────────────────────────────────────

/// Apply the per-segment streaming-reveal fade to a portal-tile markdown
/// [`TextItem`], in place (hud-bl7yi).
///
/// Rewrites `item.styled_runs` into a *full-coverage* run list so every laid-out
/// byte carries an explicit alpha, driven by `reveal.alpha_for_byte`:
/// - pre-existing / already-revealed bytes keep their original style at full
///   alpha,
/// - the leading (currently-fading) segment is dimmed by the [`StreamFadeRamp`],
/// - not-yet-revealed segments are driven to alpha `0` (laid out, invisible).
///
/// Full coverage matters because unstyled gaps would otherwise render at the
/// item's default color (full alpha) and refuse to fade. Style attributes
/// (weight/italic/monospace/color/size/background) are inherited from the
/// original run covering each slice (last-writer-wins, matching the renderer's
/// run precedence); gaps fall back to the item's base color.
///
/// No-op unless the item lays out exactly the snapshot the reveal was anchored
/// to (guards against truncation/mismatch) and is a cached/styled markdown item
/// (empty `color_runs`). Slices never straddle a breakpoint, so `alpha_for_byte`
/// at the slice start is the alpha for the whole slice.
///
/// [`StreamFadeRamp`]: super::easing::StreamFadeRamp
fn apply_portal_reveal_fade(
    item: &mut TextItem,
    reveal: &super::draw_cmds::PortalTileStreamReveal,
    ramp: super::easing::StreamFadeRamp,
) {
    use crate::text::{StyledRunItem, apply_opacity_to_color};

    if !reveal.is_revealing() {
        return;
    }
    // The reveal's offsets index the plain-text snapshot it was built from; only
    // apply when this item lays out that exact text.
    if item.text.as_ref() != reveal.plain_text.as_ref() {
        return;
    }
    // Pixel-bearing color-run items use raw-content offsets (never reached for
    // portal cached markdown, but stay safe).
    if !item.color_runs.is_empty() {
        return;
    }
    let n = item.text.len();
    if n == 0 {
        return;
    }

    // Slice boundaries: existing run edges ∪ breakpoints ∪ reveal_start ∪
    // endpoints. Every breakpoint is a boundary, so no slice crosses a segment
    // and the alpha is constant across each slice.
    let mut bounds: Vec<usize> =
        Vec::with_capacity(item.styled_runs.len() * 2 + reveal.breakpoints.len() + 3);
    bounds.push(0);
    bounds.push(n);
    let push_bound = |bounds: &mut Vec<usize>, off: usize| {
        let off = off.min(n);
        if item.text.is_char_boundary(off) {
            bounds.push(off);
        }
    };
    push_bound(&mut bounds, reveal.reveal_start);
    for &b in &reveal.breakpoints {
        push_bound(&mut bounds, b);
    }
    for run in item.styled_runs.iter() {
        push_bound(&mut bounds, run.start_byte);
        push_bound(&mut bounds, run.end_byte);
    }
    bounds.sort_unstable();
    bounds.dedup();

    let base = item.color;
    let mut faded: Vec<StyledRunItem> = Vec::with_capacity(bounds.len());
    for win in bounds.windows(2) {
        let (s, e) = (win[0], win[1]);
        if s >= e {
            continue;
        }
        let alpha = reveal.alpha_for_byte(s, ramp);
        // Last original run covering `s` supplies the style (last-writer-wins).
        let style = item
            .styled_runs
            .iter()
            .rev()
            .find(|r| r.start_byte <= s && s < r.end_byte);
        let src_color = style.and_then(|r| r.color).unwrap_or(base);
        faded.push(StyledRunItem {
            start_byte: s,
            end_byte: e,
            weight: style.and_then(|r| r.weight),
            italic: style.map(|r| r.italic).unwrap_or(false),
            monospace: style.map(|r| r.monospace).unwrap_or(false),
            color: Some(apply_opacity_to_color(src_color, alpha)),
            background_color: style
                .and_then(|r| r.background_color)
                .map(|c| apply_opacity_to_color(c, alpha)),
            size_scale: style.and_then(|r| r.size_scale),
        });
    }
    item.styled_runs = faded.into_boxed_slice();
}

/// Collect [`TextItem`]s for all `TextOverflow::Ellipsis` nodes reachable from
/// `node_id`, without scroll offset (prime-time geometry).
///
/// This is a free function (not a method) to avoid a split-borrow conflict in
/// [`super::Compositor::prime_truncation_cache`], where `self.text_rasterizer` is
/// borrowed mutably while the markdown snapshot (loaded from the primer) and
/// `self.node_key_cache` are read immutably.
///
/// The geometry produced here is identical to what `collect_text_items_from_node`
/// produces at scroll_x=0, scroll_y=0 (valid because truncation is geometry-
/// dependent only on `bounds_width` / `bounds_height`, which are scroll-invariant).
///
/// `at_tail`: whether the tile owning these nodes is currently in follow-tail/at-tail
/// mode.  Callers must obtain this value via [`super::tile_at_tail_for_ellipsis`] to
/// guarantee alignment with the per-frame key built by `collect_text_items_from_node`.
/// `true` → `TailAnchored` truncation (spec §3.2 — newest lines visible);
/// `false` → `HeadAnchored` (spec §3.3 — viewport stability after user scroll-back).
///
/// `markdown_tokens`: used for the cache-miss non-lossy inline parse fallback
/// (hud-xcp9b); in steady-state this argument is never consumed.
// All arguments are required: node identity, scene reference, tile position,
// tail-follow state, markdown cache, node key cache, token store, and the output
// accumulator.  The parameter set mirrors the sibling method
// `collect_text_items_from_node`; no subset can be omitted.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_ellipsis_text_items_from_node(
    node_id: SceneId,
    scene: &SceneGraph,
    tile_x: f32,
    tile_y: f32,
    at_tail: bool,
    markdown_cache: &crate::markdown::MarkdownCache,
    node_key_cache: &HashMap<SceneId, [u8; 32]>,
    markdown_tokens: &crate::markdown::MarkdownTokens,
    items: &mut Vec<TextItem>,
) {
    let node = match scene.nodes.get(&node_id) {
        Some(n) => n,
        None => return,
    };

    if let NodeData::TextMarkdown(tm) = &node.data {
        if tm.overflow == tze_hud_scene::types::TextOverflow::Ellipsis {
            // color_runs bypass: same as in collect_text_items_from_node —
            // the markdown cache path drops color_runs, so skip it when the
            // node carries inline color_runs.
            //
            // Key lookup: use the per-node key cache (populated at prime time)
            // to avoid re-hashing content on the frame path.  Falls back to
            // compute_key only if the entry is absent (pre-prime first frame).
            // (hud-gpqde)
            //
            // Cache-miss fallback (hud-xcp9b): same non-lossy inline-parse
            // strategy as collect_text_items_from_node.  See that site for the
            // full rationale.  In steady-state (commit-time-primed) frames this
            // branch is never taken.
            //
            // Gate on *pixel-bearing* runs, not `is_empty()`: zero-length
            // sentinel runs carry metadata only and must not force the lossy
            // raw-content path. (hud-9v3t6)
            let mut item = if !crate::text::markdown_node_has_pixel_runs(tm) {
                let content_key = node_key_cache
                    .get(&node_id)
                    .copied()
                    .unwrap_or_else(|| crate::markdown::MarkdownCache::compute_key(&tm.content));
                if let Some(parsed) = markdown_cache.get_by_key(&content_key) {
                    TextItem::from_text_markdown_cached(tm, tile_x, tile_y, parsed)
                } else {
                    // Cache miss: same non-lossy inline-parse strategy as
                    // collect_text_items_from_node — normal on first frame /
                    // mid-frame node add.  warn! provides observability. [hud-rbf91]
                    tracing::warn!(
                        node_id = ?node_id,
                        content_len = tm.content.len(),
                        "markdown cache miss on ellipsis render path — expected commit-time prime \
                         (hud-xcp9b); parsing inline to preserve styling [hud-380dl]"
                    );
                    let parsed =
                        crate::markdown::parse_markdown_subset(&tm.content, markdown_tokens);
                    TextItem::from_text_markdown_cached(tm, tile_x, tile_y, &parsed)
                }
            } else {
                TextItem::from_text_markdown_node(tm, tile_x, tile_y)
            };
            // Override viewport based on the tile's follow-tail state.
            if at_tail {
                item.viewport = crate::overflow::TruncationViewport::TailAnchored;
            }
            items.push(item);
        }
    }

    for child_id in &node.children {
        collect_ellipsis_text_items_from_node(
            *child_id,
            scene,
            tile_x,
            tile_y,
            at_tail,
            markdown_cache,
            node_key_cache,
            markdown_tokens,
            items,
        );
    }
}

#[cfg(test)]
mod portal_reveal_render_tests {
    use super::super::draw_cmds::{PortalTileStreamReveal, derive_word_breakpoints};
    use super::super::easing::{Easing, StreamFadeRamp};
    use super::apply_portal_reveal_fade;
    use crate::markdown::{ParsedMarkdown, StyleAttr, StyledSpan};
    use crate::text::TextItem;
    use std::sync::Arc;
    use tze_hud_scene::types::{FontFamily, Rect, Rgba, TextAlign, TextMarkdownNode, TextOverflow};

    fn plain_attr() -> StyleAttr {
        StyleAttr {
            weight: None,
            italic: false,
            monospace: false,
            color: None,
            background_color: None,
            size_scale: None,
        }
    }

    /// Build a cached markdown `TextItem` whose laid-out text == `plain`, with a
    /// single bold span over the leading `bold_prefix` bytes.
    fn markdown_item(plain: &str, bold_prefix: usize) -> TextItem {
        let parsed = ParsedMarkdown {
            plain_text: Arc::from(plain),
            spans: vec![StyledSpan {
                start_byte: 0,
                end_byte: bold_prefix,
                attr: StyleAttr {
                    weight: Some(700),
                    ..plain_attr()
                },
            }],
            code_panels: vec![],
            list_items: vec![],
            line_height_multiplier: 1.4,
        };
        let node = TextMarkdownNode {
            content: plain.to_owned(),
            bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
            font_size_px: 16.0,
            font_family: FontFamily::SystemSansSerif,
            color: Rgba::WHITE,
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
            color_runs: Box::default(),
        };
        TextItem::from_text_markdown_cached(&node, 0.0, 0.0, &parsed)
    }

    /// Alpha (0..=255) of the styled run covering `byte`, or `None` if uncovered.
    fn alpha_at(item: &TextItem, byte: usize) -> Option<u8> {
        item.styled_runs
            .iter()
            .rev()
            .find(|r| r.start_byte <= byte && byte < r.end_byte)
            .and_then(|r| r.color)
            .map(|c| c[3])
    }

    #[test]
    fn fade_dims_leading_segment_and_hides_unrevealed_tail() {
        let plain = "old new1 new2";
        let item = markdown_item(plain, 3); // "old" bold
        let start = super::super::draw_cmds::common_prefix_len("old ", plain);
        let bps = derive_word_breakpoints(plain, start);
        let reveal = PortalTileStreamReveal::new(plain.into(), start, bps);
        let ramp = StreamFadeRamp::new(Easing::Linear);

        let mut faded = item.clone();
        apply_portal_reveal_fade(&mut faded, &reveal, ramp);

        // Pre-existing prefix stays fully opaque AND keeps its bold weight.
        assert_eq!(alpha_at(&faded, 0), Some(255), "prefix must stay opaque");
        let prefix_run = faded
            .styled_runs
            .iter()
            .find(|r| r.start_byte == 0 && r.end_byte > 0)
            .unwrap();
        assert_eq!(prefix_run.weight, Some(700), "prefix style must survive");

        // Leading segment (first appended word) is dimmed at t=0 (alpha 0).
        assert_eq!(
            alpha_at(&faded, start),
            Some(0),
            "leading starts transparent"
        );
        // Not-yet-revealed tail word is fully hidden.
        assert_eq!(alpha_at(&faded, plain.len() - 1), Some(0), "tail hidden");
    }

    #[test]
    fn fade_alpha_increases_with_reveal_progress() {
        let plain = "x ab cd";
        let item = markdown_item(plain, 0);
        let bps = derive_word_breakpoints(plain, 0);
        let mut reveal = PortalTileStreamReveal::new(plain.into(), 0, bps);
        let ramp = StreamFadeRamp::new(Easing::Linear);

        let mut a = item.clone();
        apply_portal_reveal_fade(&mut a, &reveal, ramp);
        let alpha0 = alpha_at(&a, 0).unwrap();

        reveal.advance();
        reveal.advance();
        let mut b = item.clone();
        apply_portal_reveal_fade(&mut b, &reveal, ramp);
        let alpha1 = alpha_at(&b, 0).unwrap();

        assert!(
            alpha1 > alpha0,
            "leading-segment draw alpha must rise with reveal progress: {alpha1} > {alpha0}"
        );
    }

    #[test]
    fn settled_reveal_leaves_item_untouched() {
        let plain = "old new1 new2";
        let item = markdown_item(plain, 3);
        // A settled (fully-revealed) reveal must be a no-op: steady tiles render
        // identically to the no-reveal path (deliverable #3).
        let reveal = PortalTileStreamReveal::settled(plain.into());
        let ramp = StreamFadeRamp::default();

        let mut after = item.clone();
        apply_portal_reveal_fade(&mut after, &reveal, ramp);
        assert_eq!(
            format!("{:?}", item.styled_runs),
            format!("{:?}", after.styled_runs),
            "settled reveal must not alter styled runs"
        );
    }
}
