//! Severity / urgency / tile-background color constants and free color helpers.
//!
//! Moved from `renderer.rs` banners 1–3 by Step R-1 of the renderer module split
//! (hud-fgryk).  No logic was changed; only visibility modifiers were added where
//! Rust's module-privacy rules require them.

use std::collections::HashMap;

use tze_hud_scene::types::*;

// ─── Severity token fallback colors ─────────────────────────────────────────

/// Default severity colors (linear sRGB) used when design tokens are absent.
///
/// Per spec §Canonical Token Schema:
///   color.severity.info     → #4A9EFF → (0.078, 0.384, 1.0)
///   color.severity.warning  → #FFB800 → (1.0, 0.722, 0.0)
///   color.severity.critical → #FF0000 → (1.0, 0.0, 0.0)
pub(super) const SEVERITY_INFO: Rgba = Rgba {
    r: 0.078,
    g: 0.384,
    b: 1.0,
    a: 1.0,
};
pub(super) const SEVERITY_WARNING: Rgba = Rgba {
    r: 1.0,
    g: 0.722,
    b: 0.0,
    a: 1.0,
};
pub(super) const SEVERITY_CRITICAL: Rgba = Rgba {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

// ─── Notification urgency token fallback colors ───────────────────────────────

/// Default notification urgency backdrop colors (linear sRGB) used when
/// `color.notification.urgency.*` design tokens are absent.
///
/// Per spec §Notification Urgency Backdrop Token Schema:
///   color.notification.urgency.low      → #000000 → (0.0, 0.0, 0.0)
///   color.notification.urgency.normal   → #0C1426 → (0.0037, 0.007, 0.0194)
///   color.notification.urgency.urgent   → #2A1E08 → (0.0232, 0.013, 0.0024)
///   color.notification.urgency.critical → #450612 → (0.0595, 0.0018, 0.006)
///
/// These tokens are for notification-area (and non-alert-banner notification zones)
/// only. Alert-banner continues to use `color.severity.*` tokens.
pub(super) const NOTIFICATION_URGENCY_LOW: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
pub(super) const NOTIFICATION_URGENCY_NORMAL: Rgba = Rgba {
    r: 0.0037,
    g: 0.007,
    b: 0.0194,
    a: 1.0,
};
pub(super) const NOTIFICATION_URGENCY_URGENT: Rgba = Rgba {
    r: 0.0232,
    g: 0.013,
    b: 0.0024,
    a: 1.0,
};
pub(super) const NOTIFICATION_URGENCY_CRITICAL: Rgba = Rgba {
    r: 0.0595,
    g: 0.0018,
    b: 0.006,
    a: 1.0,
};

/// Per-notification backdrop opacity applied to urgency-tinted backdrop quads.
///
/// This is the fixed 0.8 opacity specified for notification zone backdrop rendering.
/// It overrides the token color's alpha channel.
pub(super) const NOTIFICATION_BACKDROP_OPACITY: f32 = 0.8;

/// Scale factor for the body line font size in two-line notification layout.
///
/// When a `NotificationPayload.title` is non-empty, the body text (`text` field)
/// is rendered at `title_font_size * NOTIFICATION_BODY_SCALE`.
///
/// Token override path: `typography.notification.body.scale` (parsed as f32).
/// Fallback: 0.85.
pub(super) const NOTIFICATION_BODY_SCALE: f32 = 0.85;

/// Bold font weight used for the title line in two-line notification layout.
///
/// Token override: `typography.notification.title.weight` (parsed as u16).
/// Fallback: 700 (bold).
pub(super) const NOTIFICATION_TITLE_WEIGHT: u16 = 700;

/// Adaptive mid-drag re-truncation cadence thresholds (hud-3to8i).
///
/// # Cadence contract (hud-ghhxa — spec §6b.3)
///
/// When a portal tile's bounds change rapidly (resize hotkey repeat, or
/// pointer-drag resize gesture), the scene version increments on every change
/// and `prime_truncation_cache` would otherwise re-prime every frame —
/// O(n) per prime in text content length, which can exceed the Stage 5 / Stage 6
/// frame budget on large content.
///
/// The cadence gate in `Compositor::prime_truncation_cache` ensures at most one
/// re-prime per the adaptive interval during a continuous geometry change, while
/// guaranteeing that every distinct intermediate geometry *is* eventually
/// reflected in the truncation output — not only at drag-end.
///
/// The interval is derived from the total byte count of Ellipsis text content
/// visible in the scene at the time of the last successful prime:
///
/// - **Short content** (< 1 KiB): 16 ms ≈ 60 Hz — cheap re-prime keeps
///   truncation visually responsive during resize.
/// - **Medium content** (1 KiB – 16 KiB): 50 ms ≈ 20 Hz — the former fixed
///   default; good balance for typical transcript panes.
/// - **Long content** (≥ 16 KiB): 100 ms ≈ 10 Hz — throttled to protect the
///   frame budget when O(n) shaping cost is highest.
///
/// The adaptive decision is O(1) (a single `usize` comparison against constants
/// using the last-prime byte count) and must never allocate or block on the hot
/// path.
///
/// The gate is bypassed when the sentinel is `u64::MAX` (a forced re-prime
/// requested by `set_token_map` or initialisation) so that token/font-metric
/// changes are always reflected immediately regardless of resize cadence.
pub(super) const RESIZE_REPRIME_SHORT_THRESHOLD_BYTES: usize = 1_024; // < 1 KiB → fast cadence
pub(super) const RESIZE_REPRIME_LONG_THRESHOLD_BYTES: usize = 16_384; // ≥ 16 KiB → slow cadence

/// Re-prime interval for short content (< 1 KiB): ≈60 Hz.
pub(super) const RESIZE_REPRIME_INTERVAL_SHORT_MS: u64 = 16;
/// Re-prime interval for medium content (1 KiB – 16 KiB): ≈20 Hz.
pub(super) const RESIZE_REPRIME_INTERVAL_MEDIUM_MS: u64 = 50;
/// Re-prime interval for long content (≥ 16 KiB): ≈10 Hz.
pub(super) const RESIZE_REPRIME_INTERVAL_LONG_MS: u64 = 100;

/// Compute the adaptive re-prime interval in milliseconds from the total byte
/// count of Ellipsis text content in the scene.
///
/// This is O(1) — a single `usize` comparison — and must never allocate.
/// See the [`RESIZE_REPRIME_SHORT_THRESHOLD_BYTES`] /
/// [`RESIZE_REPRIME_LONG_THRESHOLD_BYTES`] constants for the threshold values.
pub(crate) fn adaptive_reprime_interval_ms(total_content_bytes: usize) -> u64 {
    if total_content_bytes < RESIZE_REPRIME_SHORT_THRESHOLD_BYTES {
        RESIZE_REPRIME_INTERVAL_SHORT_MS
    } else if total_content_bytes < RESIZE_REPRIME_LONG_THRESHOLD_BYTES {
        RESIZE_REPRIME_INTERVAL_MEDIUM_MS
    } else {
        RESIZE_REPRIME_INTERVAL_LONG_MS
    }
}

/// Vertical gap (px) between the title line and the body line in two-line layout.
pub(super) const NOTIFICATION_INTER_LINE_GAP: f32 = 2.0;

/// Warm-gray placeholder color rendered for `ZoneContent::StaticImage` zones.
///
/// Full GPU texture upload (wgpu sampler pipeline) is deferred to a follow-up
/// iteration. This constant is intentionally shared between the Stack and
/// non-Stack contention-policy branches so both render the same placeholder.
pub(super) const STATIC_IMAGE_PLACEHOLDER_COLOR: Rgba = Rgba {
    r: 0.3,
    g: 0.3,
    b: 0.3,
    a: 1.0,
};

/// Dark placeholder color rendered for `ZoneContent::VideoSurfaceRef` zones.
///
/// Rendered when the video surface is in `Admitted`, `Placeholder`, or
/// `Closed`/`Revoked` state (no frame available).  Distinct from
/// `STATIC_IMAGE_PLACEHOLDER_COLOR` so video zones are visually identifiable
/// (dark/off vs warm-gray/loading).
///
/// In `Streaming` state the compositor will eventually draw the decoded
/// frame texture; until real GStreamer → GPU upload lands (a follow-up task),
/// this placeholder is always shown.  When in `LastFrameWithBadge` state
/// (B11 media drop), the same placeholder is shown with a disconnection-badge
/// overlay emitted by the chrome layer.
///
/// Per engineering-bar.md §1: placeholder behavior is tested in
/// `video_surface.rs` unit tests; frame-timing budget (Stage 6 < 4 ms) is
/// not materially affected by the color quad.
pub(super) const VIDEO_SURFACE_PLACEHOLDER_COLOR: Rgba = Rgba {
    r: 0.05,
    g: 0.05,
    b: 0.05,
    a: 1.0,
};

/// Size of a notification icon in pixels (square, 24×24).
///
/// Icons are rendered left-aligned within the notification backdrop, at the
/// same horizontal inset as the text, and vertically centred in the slot.
pub(super) const NOTIFICATION_ICON_SIZE_PX: f32 = 24.0;

/// Horizontal gap between the icon and the notification text (pixels).
pub(super) const NOTIFICATION_ICON_GAP_PX: f32 = 6.0;

/// Side length of the notification dismiss affordance (square, top-right).
pub(super) const NOTIFICATION_DISMISS_BUTTON_SIZE_PX: f32 = 20.0;

/// Horizontal breathing room between the dismiss affordance and notification text.
pub(super) const NOTIFICATION_DISMISS_GAP_PX: f32 = 8.0;

/// Font size in pixels for the dismiss ("X") button label.
///
/// Token override: `typography.notification.dismiss.font_size_px` (parsed as f32,
/// strips a trailing `px` suffix if present).
/// Fallback: 12.0 px.
pub(super) const NOTIFICATION_DISMISS_FONT_SIZE_PX: f32 = 12.0;

/// Font weight for the dismiss ("X") button label.
///
/// Token override: `typography.notification.dismiss.font_weight` (parsed as u16).
/// Fallback: 700 (bold).
pub(super) const NOTIFICATION_DISMISS_FONT_WEIGHT: u16 = 700;

/// Fallback color for `color.border.default` when the token is absent.
///
/// Matches the default value in built-in component startup tokens (#444466).
pub(super) const BORDER_DEFAULT_FALLBACK: Rgba = Rgba {
    r: 0.267,
    g: 0.267,
    b: 0.400,
    a: 1.0,
};

// ─── Tile background token fallback colors ────────────────────────────────────

/// Default tile background color for `TextMarkdown` tiles (linear sRGB) used
/// when the `color.tile.background.text_markdown` design token is absent.
///
/// Per spec §Canonical Token Schema:
///   color.tile.background.text_markdown → #6C6C88 → (0.15, 0.15, 0.25) linear
///
/// Note: the sRGB hex for linear (0.15, 0.15, 0.25) is #6C6C88, not #636380.
/// (#636380 linearises to ≈(0.125, 0.125, 0.216); #262640 to ≈(0.019, 0.019, 0.051).)
pub(super) const TILE_BG_TEXT_MARKDOWN: Rgba = Rgba {
    r: 0.15,
    g: 0.15,
    b: 0.25,
    a: 1.0,
};

/// Default tile background color for `StaticImage` tiles (linear sRGB) used
/// when the `color.tile.background.static_image` design token is absent.
///
/// Per spec §Canonical Token Schema:
///   color.tile.background.static_image → #373737 → (0.05, 0.05, 0.05)
pub(super) const TILE_BG_STATIC_IMAGE: Rgba = Rgba {
    r: 0.05,
    g: 0.05,
    b: 0.05,
    a: 1.0,
};

/// Default tile background color for tiles with unknown/default content type
/// (linear sRGB) used when the `color.tile.background.default` design token
/// is absent.
///
/// Per spec §Canonical Token Schema:
///   color.tile.background.default → #505073 → (0.1, 0.1, 0.2)
pub(super) const TILE_BG_DEFAULT: Rgba = Rgba {
    r: 0.1,
    g: 0.1,
    b: 0.2,
    a: 1.0,
};

/// Icon size in pixels for status-bar entry icons.
///
/// Icons from `RenderingPolicy::key_icon_map` are rasterized at this size
/// (square) and rendered to the left of each mapped entry's text value.
pub(crate) const ICON_SIZE_PX: f32 = 24.0;

/// Gap in pixels between the icon and the text value for status-bar entries.
pub(super) const ICON_TEXT_GAP_PX: f32 = 6.0;

/// sRGB transfer: linear → sRGB (matches GPU hardware encoding on `*Srgb` surfaces).
#[inline]
pub(super) fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB transfer: sRGB → linear.
#[inline]
pub(super) fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Returns `true` if the zone name is an alert-banner zone.
///
/// Per spec §V1 Component Type Definitions: the alert-banner zone name is
/// `"alert-banner"`.  notification-area uses urgency-tinted notification
/// backdrop tokens, not severity tokens.
#[inline]
pub(super) fn is_alert_banner_zone(zone_name: &str) -> bool {
    zone_name == "alert-banner"
}

/// Extract the urgency from a `ZonePublishRecord` if it carries `Notification` content.
///
/// Returns `0` for non-Notification content (treated as lowest severity for sort).
#[inline]
pub(super) fn publish_urgency(record: &ZonePublishRecord) -> u32 {
    match &record.content {
        ZoneContent::Notification(n) => n.urgency,
        _ => 0,
    }
}

/// Sort alert-banner publications into display order: severity-descending (critical first),
/// then recency-descending (newer first) within the same severity level.
///
/// Returns indices into `publishes` in the order they should occupy slots 0, 1, 2, …
/// (slot 0 = topmost = highest severity / newest).
///
/// This is a pure helper so both `collect_text_items` and `render_zone_content` use
/// the same ordering without duplicating logic.
pub(super) fn sort_alert_banner_indices(publishes: &[ZonePublishRecord]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..publishes.len()).collect();
    // Primary key: urgency descending (3=critical at top).
    // Secondary key: published_at_wall_us descending (newer above older).
    // Tertiary key: original index descending (newer inserts above older on exact timestamp ties).
    indices.sort_by(|&a, &b| {
        let ua = publish_urgency(&publishes[a]);
        let ub = publish_urgency(&publishes[b]);
        ub.cmp(&ua)
            .then_with(|| {
                publishes[b]
                    .published_at_wall_us
                    .cmp(&publishes[a].published_at_wall_us)
            })
            .then_with(|| b.cmp(&a))
    });
    indices
}

/// Parse a `#RRGGBB` or `#RRGGBBAA` hex string into `Rgba`.
///
/// This is a minimal, allocation-free parser used to resolve token color
/// values at render time without depending on `tze_hud_config`.
/// Hex channels are interpreted as sRGB design-token values and converted to
/// linear RGB for the compositor pipeline.
/// Returns `None` if the string does not match either form.
pub(super) fn parse_hex_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if !s.starts_with('#') || !s.is_ascii() {
        return None;
    }
    let hex = &s[1..];
    match hex.len() {
        6 | 8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = if hex.len() == 8 {
                u8::from_str_radix(&hex[6..8], 16).ok()?
            } else {
                255
            };
            Some(Rgba::new(
                srgb_to_linear(r as f32 / 255.0),
                srgb_to_linear(g as f32 / 255.0),
                srgb_to_linear(b as f32 / 255.0),
                a as f32 / 255.0,
            ))
        }
        _ => None,
    }
}

/// Look up a token key in the resolved token map and parse it as a color.
/// Returns `None` if the key is absent or the value is not a valid hex color.
#[inline]
pub(super) fn resolve_token_color(token_map: &HashMap<String, String>, key: &str) -> Option<Rgba> {
    token_map.get(key).and_then(|v| parse_hex_color(v))
}

/// Look up a severity token key in the resolved token map and parse it as a
/// color.  Returns `None` if the key is absent or the value is not a valid
/// hex color.
#[inline]
pub(super) fn resolve_severity_token(
    token_map: &HashMap<String, String>,
    key: &str,
) -> Option<Rgba> {
    resolve_token_color(token_map, key)
}

/// Map a `NotificationPayload.urgency` level to a severity backdrop color.
///
/// Looks up `color.severity.{info,warning,critical}` in `token_map` first;
/// falls back to hardcoded SEVERITY_* constants when the key is absent or
/// cannot be parsed as a hex color.
///
/// Per spec §Notification Urgency-to-Severity Token Mapping:
///   urgency 0, 1 → color.severity.info   (fallback: #4A9EFF)
///   urgency 2    → color.severity.warning (fallback: #FFB800)
///   urgency 3    → color.severity.critical (fallback: #FF0000)
///
/// The returned `Rgba` alpha is 1.0 (unless the token itself carries an alpha
/// via `#RRGGBBAA`); `backdrop_opacity` from the policy is applied by the
/// caller after this lookup.
///
/// MUST NOT be used for notification-area zones. Only for alert-banner zones.
pub(super) fn urgency_to_severity_color(urgency: u32, token_map: &HashMap<String, String>) -> Rgba {
    match urgency {
        3 => resolve_severity_token(token_map, "color.severity.critical")
            .unwrap_or(SEVERITY_CRITICAL),
        2 => {
            resolve_severity_token(token_map, "color.severity.warning").unwrap_or(SEVERITY_WARNING)
        }
        _ => resolve_severity_token(token_map, "color.severity.info").unwrap_or(SEVERITY_INFO),
    }
}

/// Map a `NotificationPayload.urgency` level to a notification urgency backdrop color.
///
/// Looks up `color.notification.urgency.{low,normal,urgent,critical}` in `token_map`
/// first; falls back to hardcoded NOTIFICATION_URGENCY_* constants when the key is
/// absent or cannot be parsed as a hex color.
///
/// Per spec §Notification Urgency Backdrop Token Schema:
///   urgency 0     → color.notification.urgency.low      (fallback: #000000)
///   urgency 1     → color.notification.urgency.normal   (fallback: #0C1426)
///   urgency 2     → color.notification.urgency.urgent   (fallback: #2A1E08)
///   urgency 3+    → color.notification.urgency.critical (fallback: #450612)
///
/// Urgency values greater than 3 are clamped to 3 (critical).
///
/// MUST NOT use `color.severity.*` tokens — those are for alert-banner only.
pub(super) fn urgency_to_notification_color(
    urgency: u32,
    token_map: &HashMap<String, String>,
) -> Rgba {
    // Clamp urgency >3 to critical (3).
    let level = urgency.min(3);
    match level {
        0 => resolve_token_color(token_map, "color.notification.urgency.low")
            .unwrap_or(NOTIFICATION_URGENCY_LOW),
        1 => resolve_token_color(token_map, "color.notification.urgency.normal")
            .unwrap_or(NOTIFICATION_URGENCY_NORMAL),
        2 => resolve_token_color(token_map, "color.notification.urgency.urgent")
            .unwrap_or(NOTIFICATION_URGENCY_URGENT),
        _ => resolve_token_color(token_map, "color.notification.urgency.critical")
            .unwrap_or(NOTIFICATION_URGENCY_CRITICAL),
    }
}

/// Resolve the `color.border.default` token from the map.
///
/// Falls back to `BORDER_DEFAULT_FALLBACK` (#444466) when the token is absent
/// or cannot be parsed as a valid hex color.
#[inline]
pub(super) fn resolve_border_default_color(token_map: &HashMap<String, String>) -> Rgba {
    resolve_token_color(token_map, "color.border.default").unwrap_or(BORDER_DEFAULT_FALLBACK)
}

/// Resolve the tile background color for a given content-type key.
///
/// Looks up `color.tile.background.{key}` in `token_map` first; falls back to
/// the provided `fallback` constant when the key is absent or cannot be parsed
/// as a valid hex color.
///
/// Accepted keys (per §Canonical Token Schema):
///   - `"text_markdown"` → fallback `TILE_BG_TEXT_MARKDOWN` (#636380)
///   - `"static_image"`  → fallback `TILE_BG_STATIC_IMAGE`  (#373737)
///   - `"default"`       → fallback `TILE_BG_DEFAULT`        (#505073)
///
/// The caller supplies the `fallback` value so this function does not need
/// to know the full token namespace; callers should prefer the typed wrapper
/// `resolve_tile_bg_*` helpers below.
#[inline]
pub(super) fn resolve_tile_bg_token(
    token_map: &HashMap<String, String>,
    key: &str,
    fallback: Rgba,
) -> Rgba {
    resolve_token_color(token_map, key).unwrap_or(fallback)
}

#[inline]
pub(super) fn resolve_notification_control_color(
    policy: &RenderingPolicy,
    token_map: &HashMap<String, String>,
) -> Rgba {
    policy.text_color.unwrap_or_else(|| {
        resolve_token_color(token_map, "color.text.primary")
            .unwrap_or(Rgba::new(1.0, 1.0, 1.0, 0.875))
    })
}

/// Resolve `ScrollIndicatorTokens` from the compositor token map.
///
/// Keys follow the portal token namespace (`portal.scroll_indicator.*`).
/// Falls back to `ScrollIndicatorTokens::default()` for any missing or
/// unparsable token — token defaults in both crates must stay in sync per
/// the module-level contract in `tze_hud_input::scroll_indicator`.
#[inline]
pub(super) fn resolve_scroll_indicator_tokens(
    token_map: &HashMap<String, String>,
) -> tze_hud_input::ScrollIndicatorTokens {
    let defaults = tze_hud_input::ScrollIndicatorTokens::default();

    // Color: "portal.scroll_indicator.color" as #RRGGBB[AA].
    let (color_r, color_g, color_b, color_a) =
        if let Some(c) = resolve_token_color(token_map, "portal.scroll_indicator.color") {
            (c.r, c.g, c.b, c.a)
        } else {
            (
                defaults.color_r,
                defaults.color_g,
                defaults.color_b,
                defaults.color_a,
            )
        };

    let width_px = token_map
        .get("portal.scroll_indicator.width_px")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(defaults.width_px);

    let min_thumb_height_px = token_map
        .get("portal.scroll_indicator.min_height_px")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(defaults.min_thumb_height_px);

    tze_hud_input::ScrollIndicatorTokens {
        color_r,
        color_g,
        color_b,
        color_a,
        width_px,
        min_thumb_height_px,
    }
}

/// Resolve the transcript optimal-measure cap (`portal.transcript.max_measure_px`).
///
/// A portal transcript otherwise wraps to the full node width, which on a wide
/// tile produces overlong, hard-to-read lines. This token caps the effective
/// wrapping measure so body text holds a comfortable line length regardless of
/// how wide the surface is dragged.
///
/// `0` (the default) means **unbounded** — the transcript keeps wrapping to the
/// full node width, so the default profile is unchanged. Any positive value is
/// the maximum wrap width in physical pixels; the render/prime paths clamp the
/// effective measure to `min(node_width, max_measure_px)` via
/// [`super::text::clamp_transcript_measure`]. Missing, unparsable,
/// non-finite, or negative values fall back to `0.0` (unbounded).
#[inline]
pub(super) fn resolve_transcript_max_measure_px(token_map: &HashMap<String, String>) -> f32 {
    token_map
        .get("portal.transcript.max_measure_px")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0)
}

/// Resolved focus-ring visual tokens (linear sRGB color + width in px).
///
/// The keyboard focus ring is drawn by [`Compositor::render_node`] around the
/// focused `HitRegionNode` so a keyboard-only viewer can always see where focus
/// landed (RFC 0004 §5.6). Color and width are token-driven — never hardcoded in
/// the compositor — with defaults mirrored from `tze_hud_input`'s
/// [`DEFAULT_FOCUS_RING_COLOR`](tze_hud_input::DEFAULT_FOCUS_RING_COLOR) /
/// [`DEFAULT_FOCUS_RING_WIDTH_PX`](tze_hud_input::DEFAULT_FOCUS_RING_WIDTH_PX)
/// so both crates stay in sync.
#[derive(Clone, Copy, Debug)]
pub(super) struct FocusRingTokens {
    /// Ring color as a linear-sRGB `[r, g, b, a]` array (ready for `gpu_color_raw`).
    pub(super) color: [f32; 4],
    /// Ring stroke width in physical pixels.
    pub(super) width_px: f32,
}

/// Resolve [`FocusRingTokens`] from the compositor token map.
///
/// Keys follow the portal token namespace (`portal.focus_ring.*`). Falls back to
/// the `tze_hud_input` focus-ring defaults for any missing/unparsable token.
#[inline]
pub(super) fn resolve_focus_ring_tokens(token_map: &HashMap<String, String>) -> FocusRingTokens {
    let default_color = tze_hud_input::DEFAULT_FOCUS_RING_COLOR;

    let color = resolve_token_color(token_map, "portal.focus_ring.color")
        .unwrap_or(default_color)
        .to_array();

    let width_px = token_map
        .get("portal.focus_ring.width_px")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(tze_hud_input::DEFAULT_FOCUS_RING_WIDTH_PX);

    FocusRingTokens { color, width_px }
}

/// Resolved visual tokens for the portal resize-grip affordance
/// (vd-crude-resize-handle-grip). The grip is a token-colored dot-grid mark at
/// the portal's bottom-right resize corner; `hover_color` tints it when the
/// pointer is over the resize band.
///
/// Colors are linear-sRGB `[r, g, b, a]` (converted from sRGB hex by
/// `parse_hex_color`), ready for `gpu_color_raw`.
#[derive(Clone, Copy, Debug)]
pub(super) struct ResizeGripTokens {
    /// Resting grip mark color.
    pub(super) color: [f32; 4],
    /// Grip mark tint while the pointer is over the resize band.
    pub(super) hover_color: [f32; 4],
    /// Grip square extent in physical pixels (the corner mark's width/height).
    pub(super) size_px: f32,
}

impl ResizeGripTokens {
    /// The grip mark color for the current pointer state: `hover_color` when the
    /// pointer is over the portal's resize band, otherwise the resting `color`.
    #[inline]
    pub(super) fn mark_color(&self, hovered: bool) -> [f32; 4] {
        if hovered {
            self.hover_color
        } else {
            self.color
        }
    }
}

// Fallback resize-grip values — MUST stay in sync with `tze_hud_config`'s
// `portal_tokens::defaults::WINDOW_RESIZE_GRIP_*`. The two crates are
// intentionally independent (no compile-time link), so update both when a
// default changes.
const RESIZE_GRIP_DEFAULT_COLOR_HEX: &str = "#5A6373";
const RESIZE_GRIP_DEFAULT_HOVER_HEX: &str = "#8A93A6";
const RESIZE_GRIP_DEFAULT_SIZE_PX: f32 = 14.0;

/// Resolve [`ResizeGripTokens`] from the compositor token map.
///
/// Keys follow the portal token namespace (`portal.window.resize_grip.*`).
/// Falls back to the `tze_hud_config` portal-token defaults for any missing or
/// unparsable token. The size falls back to a positive default and is never
/// allowed to be non-finite or negative.
#[inline]
pub(super) fn resolve_resize_grip_tokens(token_map: &HashMap<String, String>) -> ResizeGripTokens {
    let color = resolve_token_color(token_map, "portal.window.resize_grip.color")
        .or_else(|| parse_hex_color(RESIZE_GRIP_DEFAULT_COLOR_HEX))
        .unwrap_or(Rgba::WHITE)
        .to_array();

    let hover_color = resolve_token_color(token_map, "portal.window.resize_grip.hover_color")
        .or_else(|| parse_hex_color(RESIZE_GRIP_DEFAULT_HOVER_HEX))
        .unwrap_or(Rgba::WHITE)
        .to_array();

    let size_px = token_map
        .get("portal.window.resize_grip.size_px")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(RESIZE_GRIP_DEFAULT_SIZE_PX);

    ResizeGripTokens {
        color,
        hover_color,
        size_px,
    }
}

/// Resolved visual tokens for the runtime-authored viewer reply echo
/// (hud-nx7yq.3) — a kind-distinct color plus font size for viewer history lines
/// rendered above the composer strip on raw-tile portals.
#[derive(Clone, Copy, Debug)]
pub(super) struct ViewerEchoTokens {
    /// Viewer-line text color as sRGB `[r, g, b, a]` u8 (ready for `TextItem`).
    pub(super) color: [u8; 4],
    /// Viewer-line font size in physical pixels.
    pub(super) font_size_px: f32,
}

/// Default viewer-echo text color: a calm blue (#8AB4F8) distinct from the
/// near-white transcript body text, so viewer replies read as their own kind.
const VIEWER_ECHO_DEFAULT_COLOR: [u8; 4] = [0x8A, 0xB4, 0xF8, 0xFF];
const VIEWER_ECHO_DEFAULT_FONT_SIZE_PX: f32 = 15.0;

/// Resolve [`ViewerEchoTokens`] from the compositor token map.
///
/// Keys follow the portal token namespace (`portal.viewer_echo.*`). The color
/// falls back to a distinct viewer accent and the font size to a chat-history
/// default; never hardcoded at the call site.
#[inline]
pub(super) fn resolve_viewer_echo_tokens(token_map: &HashMap<String, String>) -> ViewerEchoTokens {
    let color = resolve_token_color(token_map, "portal.viewer_echo.text_color")
        .map(crate::text::rgba_to_srgb_u8)
        .unwrap_or(VIEWER_ECHO_DEFAULT_COLOR);

    let font_size_px = token_map
        .get("portal.viewer_echo.font_size_px")
        .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(VIEWER_ECHO_DEFAULT_FONT_SIZE_PX)
        .clamp(6.0, 200.0);

    ViewerEchoTokens {
        color,
        font_size_px,
    }
}

#[inline]
pub(super) fn notification_dismiss_bounds(
    x: f32,
    slot_y: f32,
    w: f32,
    effective_slot_h: f32,
) -> Rect {
    Rect::new(
        x + w - NOTIFICATION_DISMISS_BUTTON_SIZE_PX,
        slot_y,
        NOTIFICATION_DISMISS_BUTTON_SIZE_PX,
        NOTIFICATION_DISMISS_BUTTON_SIZE_PX.min(effective_slot_h),
    )
}

/// Resolved visual tokens for the local composer echo overlay.
///
/// Populated from the compositor token map in
/// [`resolve_composer_overlay_tokens`].  All colors are **linear sRGB**
/// (converted from sRGB hex by `parse_hex_color`).
pub(super) struct ComposerOverlayTokens {
    /// Background fill for the composer strip (linear sRGB).
    pub(super) bg_r: f32,
    pub(super) bg_g: f32,
    pub(super) bg_b: f32,
    pub(super) bg_a: f32,
    /// Text / caret color (linear sRGB).
    pub(super) text_r: f32,
    pub(super) text_g: f32,
    pub(super) text_b: f32,
    pub(super) text_a: f32,
    /// At-capacity indicator color (linear sRGB).
    pub(super) at_capacity_r: f32,
    pub(super) at_capacity_g: f32,
    pub(super) at_capacity_b: f32,
    pub(super) at_capacity_a: f32,
    /// Selection highlight background color (sRGB u8).
    ///
    /// Stored in sRGB u8 (not linear) because it is handed to
    /// [`StyledRunItem::background_color`] which expects the same encoding as
    /// the rest of the text pipeline's backdrop quads.
    pub(super) selection_bg: [u8; 4],
    /// Caret glyph foreground color (sRGB u8).
    ///
    /// Sourced from `portal.composer.caret_color` so a profile can accent the
    /// caret independently of the composer text color (hud-khfgx,
    /// vd-caret-selection-placeholder-not-tokenized). Defaults to the composer
    /// text color, so the default profile's caret is visually unchanged. Stored in
    /// sRGB u8 to match [`StyledRunItem::color`], the same encoding as the text
    /// pipeline's other foreground runs.
    pub(super) caret_color: [u8; 4],
    /// Font size in pixels.
    pub(super) font_size_px: f32,
    /// Maximum number of text lines the composer box grows to before it scrolls
    /// internally (hud-nx7yq.1).
    ///
    /// `1` selects the single-line horizontal caret-follow profile (hud-zlfi4);
    /// `> 1` selects the multi-line wrap-and-grow profile. Sourced from the
    /// `portal.composer.max_lines` token; defaults to
    /// [`COMPOSER_OVERLAY_DEFAULT_MAX_LINES`].
    pub(super) max_lines: u32,
    /// Vertical anchoring of the composer input box within its region (hud-nottc).
    ///
    /// - [`ComposerVerticalAnchor::Bottom`] (default) — the input box pins to the
    ///   BOTTOM edge of the region and grows UPWARD (the bottom-chat composer strip,
    ///   `portal-bottom-chat-composer`). The projection-portal path uses this.
    /// - [`ComposerVerticalAnchor::Top`] — the input box pins to the TOP edge of
    ///   the region; the draft caret rests at the pane's top-left content origin
    ///   when empty and the text flows DOWNWARD (document-style) as it grows, with
    ///   no teleport between empty and non-empty states. The exemplar two-pane
    ///   input pane uses this.
    ///
    /// Sourced from the `portal.composer.anchor` token (`"top"` / `"bottom"`);
    /// defaults to `Bottom` so every existing bottom-strip profile is unchanged.
    pub(super) anchor: ComposerVerticalAnchor,
    /// Horizontal + vertical content inset (physical px) between the composer
    /// region edge and the draft text, applied uniformly on every side.
    ///
    /// This is the composer's slice of the shared `portal.spacing.content_inset_px`
    /// token (hud-ar10c): the compositor's composer geometry
    /// (`composer_input_box` box padding, the caret-follow window width, the draft
    /// `pixel_x`/clip, and the viewer-echo zone width) all resolve their inset from
    /// here instead of a hardcoded literal. Defaults to
    /// [`COMPOSER_OVERLAY_DEFAULT_CONTENT_INSET_PX`] (6.0), which reproduces the
    /// previous `COMPOSER_TEXT_MARGIN` literal so the default profile is unchanged;
    /// caret-follow geometry keys off this value, so a profile that widens it shifts
    /// the caret origin in lockstep.
    pub(super) content_inset_px: f32,
}

/// Vertical anchoring of the composer input box within its region (hud-nottc).
///
/// The composer draft echo shares one layout core (`composer_input_box`) between
/// two profiles; this parameter selects which edge the box pins to. See
/// [`ComposerOverlayTokens::anchor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComposerVerticalAnchor {
    /// Pin the box to the region TOP; caret starts at the pane content origin and
    /// text flows downward (exemplar two-pane input pane).
    Top,
    /// Pin the box to the region BOTTOM; the box grows upward (bottom-chat strip).
    Bottom,
}

const COMPOSER_OVERLAY_DEFAULT_FONT_SIZE_PX: f32 = 16.0;

/// Default composer max-line bound: the box grows to at most this many wrapped
/// lines, then scrolls internally (design proposal default, hud-nx7yq.1). A small
/// bound so a tall draft cannot swallow the transcript pane.
const COMPOSER_OVERLAY_DEFAULT_MAX_LINES: u32 = 6;

/// Default composer content inset in physical px, applied on every side between
/// the composer region edge and the draft text. Sourced from
/// `portal.spacing.content_inset_px`; this default mirrors the config-crate token
/// default (`PortalPartTokens::content_inset_px` = 6) and the historical
/// `COMPOSER_TEXT_MARGIN` literal, so an absent token reproduces the prior spacing
/// exactly (no visual regression).
const COMPOSER_OVERLAY_DEFAULT_CONTENT_INSET_PX: f32 = 6.0;

pub(super) fn resolve_composer_overlay_tokens(
    token_map: &HashMap<String, String>,
) -> ComposerOverlayTokens {
    // Background (default: #0F1418 @ 1.0 alpha)
    let (bg_r, bg_g, bg_b, bg_a) = resolve_token_color(token_map, "portal.composer.background")
        .map(|c| (c.r, c.g, c.b, c.a))
        .unwrap_or((0.059, 0.078, 0.094, 1.0));

    // Text color (default: #E0E8F4)
    let (text_r, text_g, text_b, text_a) =
        resolve_token_color(token_map, "portal.composer.text_color")
            .map(|c| (c.r, c.g, c.b, c.a))
            .unwrap_or((0.878, 0.910, 0.957, 1.0));

    // At-capacity indicator color (default: #B87333, muted amber)
    let (at_capacity_r, at_capacity_g, at_capacity_b, at_capacity_a) =
        resolve_token_color(token_map, "portal.composer.at_capacity_color")
            .map(|c| (c.r, c.g, c.b, c.a))
            .unwrap_or((0.722, 0.451, 0.200, 1.0));

    // Selection highlight background (default: #3A7BD5 @ 0.45 alpha — blue tint)
    //
    // `portal.composer.selection_color` is expected in the same `#RRGGBB` or
    // `#RRGGBBAA` format as other composer tokens.  We re-encode to sRGB u8 here
    // because `StyledRunItem::background_color` uses that encoding.
    let selection_bg: [u8; 4] = resolve_token_color(token_map, "portal.composer.selection_color")
        .map(|c| {
            // `c` is linear sRGB; convert RGB channels back to sRGB u8.
            let to_srgb_u8 = |v: f32| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
            let alpha_u8 = (c.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            [to_srgb_u8(c.r), to_srgb_u8(c.g), to_srgb_u8(c.b), alpha_u8]
        })
        // Default: #3A7BD5 @ ~115/255 alpha (≈ 0.45) — a calm blue selection
        .unwrap_or([0x3A, 0x7B, 0xD5, 0x73]);

    // Caret glyph color (default: the composer text color, so the default profile
    // is visually unchanged; a profile may accent the caret independently).
    // hud-khfgx: `portal.composer.caret_color`, same `#RRGGBB[AA]` format.
    let to_srgb_u8 = |v: f32| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    let to_alpha_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let caret_color: [u8; 4] = resolve_token_color(token_map, "portal.composer.caret_color")
        .map(|c| {
            [
                to_srgb_u8(c.r),
                to_srgb_u8(c.g),
                to_srgb_u8(c.b),
                to_alpha_u8(c.a),
            ]
        })
        // Default: the resolved composer text color (linear → sRGB u8).
        .unwrap_or([
            to_srgb_u8(text_r),
            to_srgb_u8(text_g),
            to_srgb_u8(text_b),
            to_alpha_u8(text_a),
        ]);

    // Font size (default: portal composer readable fallback)
    let font_size_px = token_map
        .get("portal.composer.font_size")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|&v| v.is_finite() && v > 0.0)
        .unwrap_or(COMPOSER_OVERLAY_DEFAULT_FONT_SIZE_PX);

    // Max visible line count before internal scroll. Clamp to at least 1 so a
    // stray `0`/negative token cannot degenerate the box to zero height.
    let max_lines = token_map
        .get("portal.composer.max_lines")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&v| v >= 1)
        .unwrap_or(COMPOSER_OVERLAY_DEFAULT_MAX_LINES);

    // Content inset (default 6.0). Shared portal spacing token — the composer's
    // horizontal + vertical padding between the region edge and the draft text.
    // Reject non-finite / negative values so a malformed token cannot collapse or
    // invert the box geometry; `0.0` (flush) is permitted.
    let content_inset_px = token_map
        .get("portal.spacing.content_inset_px")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|&v| v.is_finite() && v >= 0.0)
        .unwrap_or(COMPOSER_OVERLAY_DEFAULT_CONTENT_INSET_PX);

    // Vertical anchor (default: Bottom — the bottom-chat strip). Only an explicit
    // `top` (case-insensitive) selects the top-anchored exemplar input pane; any
    // other / missing / malformed value falls back to Bottom so existing profiles
    // are unchanged.
    let anchor = match token_map
        .get("portal.composer.anchor")
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("top") => ComposerVerticalAnchor::Top,
        _ => ComposerVerticalAnchor::Bottom,
    };

    ComposerOverlayTokens {
        bg_r,
        bg_g,
        bg_b,
        bg_a,
        text_r,
        text_g,
        text_b,
        text_a,
        at_capacity_r,
        at_capacity_g,
        at_capacity_b,
        at_capacity_a,
        selection_bg,
        caret_color,
        font_size_px,
        max_lines,
        anchor,
        content_inset_px,
    }
}

/// Emit 4 thin 1px border quads positioned inside the given backdrop rectangle.
///
/// Produces a 1px inset border using four axis-aligned rectangles:
///   - top:    (x, y, w, 1)
///   - bottom: (x, y+h-1, w, 1)
///   - left:   (x, y+1, 1, h-2)
///   - right:  (x+w-1, y+1, 1, h-2)
///
/// The border is drawn inside the backdrop bounds (does not extend outside).
/// When `h < 2` or `w < 1`, the degenerate dimension quads are skipped (size ≤ 0
/// after the inset). Top/bottom edges require `w >= 1` to avoid degenerate quads
/// with zero or negative width.
///
/// `sw`/`sh` are the screen dimensions passed through to `rect_vertices`.
// All arguments are required primitive geometry inputs (x, y, w, h, sw, sh) plus
// a color; grouping them into a struct would create an arbitrary named bundle
// with no semantic benefit over the flat list already documented above.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_border_quads(
    vertices: &mut Vec<crate::pipeline::RectVertex>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    sw: f32,
    sh: f32,
    border_color: [f32; 4],
) {
    use crate::pipeline::rect_vertices;
    const BORDER_PX: f32 = 1.0;
    // Top edge.
    if h >= BORDER_PX && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(x, y, w, BORDER_PX, sw, sh, border_color));
    }
    // Bottom edge.
    if h >= BORDER_PX * 2.0 && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(
            x,
            y + h - BORDER_PX,
            w,
            BORDER_PX,
            sw,
            sh,
            border_color,
        ));
    }
    // Left edge (inset 1px top and bottom to avoid corner overlap).
    if h > BORDER_PX * 2.0 && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(
            x,
            y + BORDER_PX,
            BORDER_PX,
            h - BORDER_PX * 2.0,
            sw,
            sh,
            border_color,
        ));
    }
    // Right edge (inset 1px top and bottom to avoid corner overlap).
    if h > BORDER_PX * 2.0 && w >= BORDER_PX * 2.0 {
        vertices.extend_from_slice(&rect_vertices(
            x + w - BORDER_PX,
            y + BORDER_PX,
            BORDER_PX,
            h - BORDER_PX * 2.0,
            sw,
            sh,
            border_color,
        ));
    }
}

/// Emit a 2px inset highlight border around the given rectangle.
///
/// Used for v1-compatible drag visual feedback: a 2px border on the element
/// being dragged. Two quads are emitted per edge (stacked 1px each) to achieve
/// the 2px width.
///
/// Per the drag-to-reposition spec: MUST NOT require drop shadows, scale
/// pulses, or animated transitions.
// All arguments are required primitive geometry inputs (x, y, w, h, sw, sh) plus
// a color; same rationale as emit_border_quads — a struct would be a name-only
// wrapper with no cohesion beyond this single call site.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_drag_highlight_border(
    vertices: &mut Vec<crate::pipeline::RectVertex>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    sw: f32,
    sh: f32,
    color: [f32; 4],
) {
    use crate::pipeline::rect_vertices;
    const BORDER_PX: f32 = 2.0;
    // Top edge.
    if h >= BORDER_PX && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(x, y, w, BORDER_PX, sw, sh, color));
    }
    // Bottom edge.
    if h >= BORDER_PX * 2.0 && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(
            x,
            y + h - BORDER_PX,
            w,
            BORDER_PX,
            sw,
            sh,
            color,
        ));
    }
    // Left edge (inset by BORDER_PX top and bottom to avoid corner overlap).
    if h > BORDER_PX * 2.0 && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(
            x,
            y + BORDER_PX,
            BORDER_PX,
            h - BORDER_PX * 2.0,
            sw,
            sh,
            color,
        ));
    }
    // Right edge (inset by BORDER_PX top and bottom to avoid corner overlap).
    if h > BORDER_PX * 2.0 && w >= BORDER_PX * 2.0 {
        vertices.extend_from_slice(&rect_vertices(
            x + w - BORDER_PX,
            y + BORDER_PX,
            BORDER_PX,
            h - BORDER_PX * 2.0,
            sw,
            sh,
            color,
        ));
    }
}
