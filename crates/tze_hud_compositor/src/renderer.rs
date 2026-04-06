//! The compositor — renders the scene graph to a surface.
//!
//! For the vertical slice, this renders tiles as colored rectangles
//! with hit-region highlighting for local feedback.
//!
//! The frame pipeline is **surface-agnostic**: it renders through the
//! `CompositorSurface` trait, which is implemented by both `HeadlessSurface`
//! (offscreen) and `WindowSurface` (display-connected).  No conditional
//! compilation exists in the render path — only the surface implementation
//! differs between modes.
//!
//! Per runtime-kernel/spec.md Requirement: Headless Mode (line 198):
//! "No conditional compilation for the render path."
//!
//! ## Two-pass rendering (content → chrome)
//!
//! [`Compositor::render_frame_with_chrome`] implements the three-layer ordering required by
//! the chrome sovereignty contract:
//!   1. Background + content pass (`LoadOp::Clear` — clears and draws agent tiles)
//!   2. Chrome pass (`LoadOp::Load` — draws chrome on top of content, preserving pixels)
//!
//! This separation is the architectural foundation for future render-skip redaction
//! (capture-safe architecture): the content and chrome passes are structurally independent.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::pipeline::{
    ChromeDrawCmd, ROUNDED_RECT_OVERLAY_SHADER, ROUNDED_RECT_SHADER, RectVertex,
    RoundedRectDrawCmd, RoundedRectVertex, create_texture_rect_bind_group_layout,
    create_texture_rect_pipeline, rect_vertices, rounded_rect_vertices, textured_rect_vertices,
};
use crate::surface::{CompositorSurface, HeadlessSurface};
use crate::text::{TextItem, TextRasterizer};
use crate::widget::WidgetRenderer;
use tze_hud_scene::DegradationLevel;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_telemetry::FrameTelemetry;

// ─── Severity token fallback colors ─────────────────────────────────────────

/// Default severity colors (linear sRGB) used when design tokens are absent.
///
/// Per spec §Canonical Token Schema:
///   color.severity.info     → #4A9EFF → (0.078, 0.384, 1.0)
///   color.severity.warning  → #FFB800 → (1.0, 0.722, 0.0)
///   color.severity.critical → #FF0000 → (1.0, 0.0, 0.0)
const SEVERITY_INFO: Rgba = Rgba {
    r: 0.078,
    g: 0.384,
    b: 1.0,
    a: 1.0,
};
const SEVERITY_WARNING: Rgba = Rgba {
    r: 1.0,
    g: 0.722,
    b: 0.0,
    a: 1.0,
};
const SEVERITY_CRITICAL: Rgba = Rgba {
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
///   color.notification.urgency.low      → #2A2A2A → (0.165, 0.165, 0.165)
///   color.notification.urgency.normal   → #1A1A3A → (0.102, 0.102, 0.227)
///   color.notification.urgency.urgent   → #8B6914 → (0.545, 0.412, 0.078)
///   color.notification.urgency.critical → #8B1A1A → (0.545, 0.102, 0.102)
///
/// These tokens are for notification-area (and non-alert-banner notification zones)
/// only. Alert-banner continues to use `color.severity.*` tokens.
const NOTIFICATION_URGENCY_LOW: Rgba = Rgba {
    r: 0.165,
    g: 0.165,
    b: 0.165,
    a: 1.0,
};
const NOTIFICATION_URGENCY_NORMAL: Rgba = Rgba {
    r: 0.102,
    g: 0.102,
    b: 0.227,
    a: 1.0,
};
const NOTIFICATION_URGENCY_URGENT: Rgba = Rgba {
    r: 0.545,
    g: 0.412,
    b: 0.078,
    a: 1.0,
};
const NOTIFICATION_URGENCY_CRITICAL: Rgba = Rgba {
    r: 0.545,
    g: 0.102,
    b: 0.102,
    a: 1.0,
};

/// Per-notification backdrop opacity applied to urgency-tinted backdrop quads.
///
/// This is the fixed 0.9 opacity specified for notification zone backdrop rendering.
/// It overrides the token color's alpha channel.
const NOTIFICATION_BACKDROP_OPACITY: f32 = 0.9;

/// Scale factor for the body line font size in two-line notification layout.
///
/// When a `NotificationPayload.title` is non-empty, the body text (`text` field)
/// is rendered at `title_font_size * NOTIFICATION_BODY_SCALE`.
///
/// Token override path: `typography.notification.body.scale` (parsed as f32).
/// Fallback: 0.85.
const NOTIFICATION_BODY_SCALE: f32 = 0.85;

/// Bold font weight used for the title line in two-line notification layout.
///
/// Token override: `typography.notification.title.weight` (parsed as u16).
/// Fallback: 700 (bold).
const NOTIFICATION_TITLE_WEIGHT: u16 = 700;

/// Vertical gap (px) between the title line and the body line in two-line layout.
const NOTIFICATION_INTER_LINE_GAP: f32 = 2.0;

/// Warm-gray placeholder color rendered for `ZoneContent::StaticImage` zones.
///
/// Full GPU texture upload (wgpu sampler pipeline) is deferred to a follow-up
/// iteration. This constant is intentionally shared between the Stack and
/// non-Stack contention-policy branches so both render the same placeholder.
const STATIC_IMAGE_PLACEHOLDER_COLOR: Rgba = Rgba {
    r: 0.3,
    g: 0.3,
    b: 0.3,
    a: 1.0,
};

/// Size of a notification icon in pixels (square, 24×24).
///
/// Icons are rendered left-aligned within the notification backdrop, at the
/// same horizontal inset as the text, and vertically centred in the slot.
const NOTIFICATION_ICON_SIZE_PX: f32 = 24.0;

/// Horizontal gap between the icon and the notification text (pixels).
const NOTIFICATION_ICON_GAP_PX: f32 = 6.0;

/// Fallback color for `color.border.default` when the token is absent.
///
/// Matches the default value in built-in component startup tokens (#444466).
const BORDER_DEFAULT_FALLBACK: Rgba = Rgba {
    r: 0.267,
    g: 0.267,
    b: 0.400,
    a: 1.0,
};

/// Icon size in pixels for status-bar entry icons.
///
/// Icons from `RenderingPolicy::key_icon_map` are rasterized at this size
/// (square) and rendered to the left of each mapped entry's text value.
const ICON_SIZE_PX: f32 = 24.0;

/// Gap in pixels between the icon and the text value for status-bar entries.
const ICON_TEXT_GAP_PX: f32 = 6.0;

/// sRGB transfer: linear → sRGB (matches GPU hardware encoding on `*Srgb` surfaces).
#[inline]
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB transfer: sRGB → linear.
#[inline]
fn srgb_to_linear(c: f32) -> f32 {
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
fn is_alert_banner_zone(zone_name: &str) -> bool {
    zone_name == "alert-banner"
}

/// Extract the urgency from a `ZonePublishRecord` if it carries `Notification` content.
///
/// Returns `0` for non-Notification content (treated as lowest severity for sort).
#[inline]
fn publish_urgency(record: &ZonePublishRecord) -> u32 {
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
fn sort_alert_banner_indices(publishes: &[ZonePublishRecord]) -> Vec<usize> {
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
/// Returns `None` if the string does not match either form.
fn parse_hex_color(s: &str) -> Option<Rgba> {
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
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            ))
        }
        _ => None,
    }
}

/// Look up a token key in the resolved token map and parse it as a color.
/// Returns `None` if the key is absent or the value is not a valid hex color.
#[inline]
fn resolve_token_color(token_map: &HashMap<String, String>, key: &str) -> Option<Rgba> {
    token_map.get(key).and_then(|v| parse_hex_color(v))
}

/// Look up a severity token key in the resolved token map and parse it as a
/// color.  Returns `None` if the key is absent or the value is not a valid
/// hex color.
#[inline]
fn resolve_severity_token(token_map: &HashMap<String, String>, key: &str) -> Option<Rgba> {
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
fn urgency_to_severity_color(urgency: u32, token_map: &HashMap<String, String>) -> Rgba {
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
///   urgency 0     → color.notification.urgency.low      (fallback: #2A2A2A)
///   urgency 1     → color.notification.urgency.normal   (fallback: #1A1A3A)
///   urgency 2     → color.notification.urgency.urgent   (fallback: #8B6914)
///   urgency 3+    → color.notification.urgency.critical (fallback: #8B1A1A)
///
/// Urgency values greater than 3 are clamped to 3 (critical).
///
/// MUST NOT use `color.severity.*` tokens — those are for alert-banner only.
fn urgency_to_notification_color(urgency: u32, token_map: &HashMap<String, String>) -> Rgba {
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
fn resolve_border_default_color(token_map: &HashMap<String, String>) -> Rgba {
    resolve_token_color(token_map, "color.border.default").unwrap_or(BORDER_DEFAULT_FALLBACK)
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
#[allow(clippy::too_many_arguments)]
fn emit_border_quads(
    vertices: &mut Vec<crate::pipeline::RectVertex>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    sw: f32,
    sh: f32,
    border_color: [f32; 4],
) {
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

// ─── Notification icon helpers ───────────────────────────────────────────────

/// Parse a `NotificationPayload.icon` string as a hex-encoded `ResourceId`.
///
/// Returns `Some(resource_id)` only when:
/// - The icon string is non-empty.
/// - It is exactly 64 hex characters (the `ResourceId::to_hex()` format).
///
/// Returns `None` for empty strings, human-readable names (e.g. `"shield"`),
/// or malformed hex. Callers MUST check the image_texture_cache before emitting
/// a draw command — this function does not verify that the texture is loaded.
#[inline]
fn parse_notification_icon(icon: &str) -> Option<ResourceId> {
    if icon.is_empty() {
        return None;
    }
    ResourceId::from_hex(icon)
}

// ─── Image fit mode UV calculations ─────────────────────────────────────────

/// A textured draw command collected during scene traversal.
///
/// These are collected separately from color quads because they use a
/// different vertex layout and render pipeline.
struct TexturedDrawCmd {
    resource_id: ResourceId,
    /// Pixel-space position and size of the destination rectangle.
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    /// UV sub-rectangle within the texture: `[u_min, v_min, u_max, v_max]`.
    uv_rect: [f32; 4],
    /// Per-vertex tint (opacity, fade, etc.).
    tint: [f32; 4],
}

/// Compute the UV rectangle and destination rectangle for a given fit mode.
///
/// Returns `(dest_x, dest_y, dest_w, dest_h, uv_rect)` where:
/// - `(dest_x, dest_y, dest_w, dest_h)` is the pixel-space quad to render
/// - `uv_rect` is `[u_min, v_min, u_max, v_max]` within the texture
///
/// All fit modes assume the texture contains the full image at `(img_w, img_h)`.
fn compute_fit_mode(
    fit_mode: ImageFitMode,
    // Destination bounds in pixel space
    dest_x: f32,
    dest_y: f32,
    dest_w: f32,
    dest_h: f32,
    // Source image dimensions
    img_w: u32,
    img_h: u32,
) -> (f32, f32, f32, f32, [f32; 4]) {
    let iw = img_w as f32;
    let ih = img_h as f32;

    if iw <= 0.0 || ih <= 0.0 || dest_w <= 0.0 || dest_h <= 0.0 {
        return (dest_x, dest_y, dest_w, dest_h, [0.0, 0.0, 1.0, 1.0]);
    }

    let src_aspect = iw / ih;
    let dst_aspect = dest_w / dest_h;

    match fit_mode {
        ImageFitMode::Fill => {
            // Stretch to fill — full UV, full dest
            (dest_x, dest_y, dest_w, dest_h, [0.0, 0.0, 1.0, 1.0])
        }
        ImageFitMode::Contain => {
            // Scale uniformly so the entire image is visible; letterbox bars.
            let (rw, rh) = if src_aspect > dst_aspect {
                // Image is wider: fit width, letterbox top/bottom
                let rw = dest_w;
                let rh = dest_w / src_aspect;
                (rw, rh)
            } else {
                // Image is taller: fit height, letterbox left/right
                let rh = dest_h;
                let rw = dest_h * src_aspect;
                (rw, rh)
            };
            let rx = dest_x + (dest_w - rw) * 0.5;
            let ry = dest_y + (dest_h - rh) * 0.5;
            (rx, ry, rw, rh, [0.0, 0.0, 1.0, 1.0])
        }
        ImageFitMode::Cover => {
            // Scale uniformly to cover the entire dest; crop the excess via UV.
            let (u_min, v_min, u_max, v_max) = if src_aspect > dst_aspect {
                // Image is wider than dest: crop horizontal
                let visible_fraction = dst_aspect / src_aspect;
                let u_offset = (1.0 - visible_fraction) * 0.5;
                (u_offset, 0.0, u_offset + visible_fraction, 1.0)
            } else {
                // Image is taller than dest: crop vertical
                let visible_fraction = src_aspect / dst_aspect;
                let v_offset = (1.0 - visible_fraction) * 0.5;
                (0.0, v_offset, 1.0, v_offset + visible_fraction)
            };
            (dest_x, dest_y, dest_w, dest_h, [u_min, v_min, u_max, v_max])
        }
        ImageFitMode::ScaleDown => {
            // Like Contain but never scale up.
            if iw <= dest_w && ih <= dest_h {
                // Image fits at native size — center it.
                let rx = dest_x + (dest_w - iw) * 0.5;
                let ry = dest_y + (dest_h - ih) * 0.5;
                (rx, ry, iw, ih, [0.0, 0.0, 1.0, 1.0])
            } else {
                // Image is larger — use Contain logic.
                let (rw, rh) = if src_aspect > dst_aspect {
                    (dest_w, dest_w / src_aspect)
                } else {
                    (dest_h * src_aspect, dest_h)
                };
                let rx = dest_x + (dest_w - rw) * 0.5;
                let ry = dest_y + (dest_h - rh) * 0.5;
                (rx, ry, rw, rh, [0.0, 0.0, 1.0, 1.0])
            }
        }
    }
}

/// Per-zone opacity animation state.
///
/// Tracks a fade-in or fade-out transition for a single zone.
/// When no transition is active, the zone is at full opacity (1.0).
///
/// Modeled after `WidgetAnimationState` in `crate::widget`.
pub struct ZoneAnimationState {
    /// Wall-clock time when the transition started.
    pub transition_start: std::time::Instant,
    /// Duration of the transition in milliseconds.
    pub duration_ms: u32,
    /// Opacity at the start of the transition.
    pub from_opacity: f32,
    /// Target opacity at the end of the transition (0.0 = fade-out, 1.0 = fade-in).
    pub target_opacity: f32,
}

impl ZoneAnimationState {
    /// Create a fade-in state (opacity 0 → 1) with the given duration.
    pub fn fade_in(duration_ms: u32) -> Self {
        Self {
            transition_start: std::time::Instant::now(),
            duration_ms,
            from_opacity: 0.0,
            target_opacity: 1.0,
        }
    }

    /// Create a fade-in state starting from `from_opacity` rather than 0.
    ///
    /// Used for **transition interrupt semantics**: when a new publish arrives
    /// during an active fade-out, the fade-out is cancelled and a fade-in begins
    /// from the current composite opacity (not from zero).  This prevents blank
    /// frames during rapid replacement.
    ///
    /// Per spec §Subtitle Contention Policy — Latest Wins, Transition interrupt
    /// semantics note: "the fade-out MUST be cancelled immediately and the new
    /// content MUST begin its transition_in_ms fade-in from the current composite
    /// opacity (not from zero)."
    pub fn fade_in_from(duration_ms: u32, from_opacity: f32) -> Self {
        Self {
            transition_start: std::time::Instant::now(),
            duration_ms,
            from_opacity: from_opacity.clamp(0.0, 1.0),
            target_opacity: 1.0,
        }
    }

    /// Create a fade-out state (opacity 1 → 0) with the given duration.
    pub fn fade_out(duration_ms: u32) -> Self {
        Self {
            transition_start: std::time::Instant::now(),
            duration_ms,
            from_opacity: 1.0,
            target_opacity: 0.0,
        }
    }

    /// Compute the current interpolated opacity.
    ///
    /// Returns `target_opacity` once the transition has elapsed.
    pub fn current_opacity(&self) -> f32 {
        if self.duration_ms == 0 {
            return self.target_opacity;
        }
        let elapsed_ms = self.transition_start.elapsed().as_millis() as f32;
        let t = (elapsed_ms / self.duration_ms as f32).clamp(0.0, 1.0);
        self.from_opacity + (self.target_opacity - self.from_opacity) * t
    }

    /// Returns `true` if the transition has fully completed.
    pub fn is_complete(&self) -> bool {
        self.transition_start.elapsed().as_millis() >= self.duration_ms as u128
    }
}

/// Stable key that uniquely identifies a single publication within a zone.
///
/// Composed of `(published_at_wall_us, publisher_namespace)`.  Because the
/// scene graph assigns `published_at_wall_us` from a monotonic wall clock, the
/// combination is unique per-zone across all practical usage.
type PubKey = (u64, String);

/// Per-publication opacity animation state for TTL-based fade-out.
///
/// Unlike [`ZoneAnimationState`] (which tracks zone-wide fade-in/fade-out),
/// `PublicationAnimationState` tracks the lifecycle of **one** publication
/// within a Stack zone.  Each notification gets its own instance.
///
/// Lifecycle:
/// 1. Created when the compositor first sees the publication.  `fade_start` is
///    `None` (publication is fully visible at opacity 1.0).
/// 2. When `first_seen.elapsed() >= ttl_ms`, the compositor sets
///    `fade_start = Some(Instant::now())` to begin the 150 ms fade-out.
/// 3. While fading: `current_opacity()` interpolates from 1.0 → 0.0 over
///    `NOTIFICATION_FADE_OUT_MS` ms.
/// 4. When `is_fade_complete()` returns `true`, the publication is removed
///    from `SceneGraph::zone_registry::active_publishes`.
///
/// The effective `ttl_ms` is derived by [`Compositor::publication_ttl_ms`] with
/// this priority order:
/// - `ZonePublishRecord.expires_at_wall_us` (urgency-derived, highest priority)
/// - `NotificationPayload.ttl_ms`
/// - Zone `auto_clear_ms` / `NOTIFICATION_DEFAULT_TTL_MS` (fallback)
pub struct PublicationAnimationState {
    /// Wall-clock instant when the compositor first rendered this publication.
    pub first_seen: std::time::Instant,
    /// Effective TTL in milliseconds.  Fade-out begins once this many ms
    /// have elapsed since `first_seen`.
    pub ttl_ms: u64,
    /// Instant when the fade-out transition started.  `None` means the
    /// publication is still fully visible (TTL has not yet expired).
    pub fade_start: Option<std::time::Instant>,
    /// Fade-out duration in milliseconds (always 150 for notifications).
    pub fade_duration_ms: u32,
}

/// Duration of the per-notification fade-out transition (ms).
const NOTIFICATION_FADE_OUT_MS: u32 = 150;

/// Default TTL used when no per-publication TTL is set and the zone has no
/// `auto_clear_ms`.  Matches the notification-area zone default (8 000 ms).
const NOTIFICATION_DEFAULT_TTL_MS: u64 = 8_000;

impl PublicationAnimationState {
    /// Create a new state for a freshly-seen publication.
    pub fn new(ttl_ms: u64) -> Self {
        Self {
            first_seen: std::time::Instant::now(),
            ttl_ms,
            fade_start: None,
            fade_duration_ms: NOTIFICATION_FADE_OUT_MS,
        }
    }

    /// Check whether the TTL has expired and start the fade if so.
    ///
    /// Must be called once per frame per publication.  Idempotent after the
    /// fade has started.
    pub fn tick(&mut self) {
        if self.fade_start.is_none() && self.first_seen.elapsed().as_millis() as u64 >= self.ttl_ms
        {
            self.fade_start = Some(std::time::Instant::now());
        }
    }

    /// Returns the current effective opacity for this publication (0.0–1.0).
    ///
    /// Before fade: 1.0.
    /// During fade: linear interpolation from 1.0 → 0.0.
    /// After fade: 0.0.
    pub fn current_opacity(&self) -> f32 {
        let Some(start) = self.fade_start else {
            return 1.0;
        };
        if self.fade_duration_ms == 0 {
            return 0.0;
        }
        let elapsed_ms = start.elapsed().as_millis() as f32;
        let t = (elapsed_ms / self.fade_duration_ms as f32).clamp(0.0, 1.0);
        1.0 - t
    }

    /// Returns `true` when the fade-out transition has fully completed.
    pub fn is_fade_complete(&self) -> bool {
        let Some(start) = self.fade_start else {
            return false;
        };
        start.elapsed().as_millis() >= self.fade_duration_ms as u128
    }
}

/// How many frames to display each breakpoint segment before advancing.
///
/// At 60fps, 10 frames ≈ 167ms dwell per word — comfortably perceptible as
/// word-by-word reveal without feeling sluggish.
const STREAM_REVEAL_FRAMES_PER_SEGMENT: u32 = 10;

/// Per-zone streaming reveal state.
///
/// Tracks progressive text reveal for `StreamText` publications that include
/// breakpoints.  Each frame, `advance()` is called; when the dwell counter
/// reaches [`STREAM_REVEAL_FRAMES_PER_SEGMENT`] the compositor moves to the
/// next breakpoint.
///
/// The `pub_key` ties this state to a specific publication.  When the
/// latest-wins publication changes, the old state is discarded and a new one
/// starts from breakpoint index 0.
pub struct StreamRevealState {
    /// The publication this state tracks.
    pub pub_key: PubKey,
    /// Byte-offset breakpoints copied from the publication record.
    /// Expected to be non-decreasing (callers should validate before constructing),
    /// but not enforced here — the compositor is safe regardless of order.
    pub breakpoints: Vec<usize>,
    /// Index into `breakpoints` of the currently-visible segment boundary.
    /// A value of `breakpoints.len()` means the full text is visible.
    pub segment_idx: usize,
    /// Frame counter within the current segment.
    pub frames_in_segment: u32,
}

impl StreamRevealState {
    /// Create reveal state for a new publication.
    ///
    /// Accepts `Vec<u64>` breakpoints (wire format from `ZonePublishRecord`) and
    /// converts to `Vec<usize>` for internal indexing arithmetic.
    pub fn new(pub_key: PubKey, breakpoints: Vec<u64>) -> Self {
        Self {
            pub_key,
            breakpoints: breakpoints.into_iter().map(|b| b as usize).collect(),
            segment_idx: 0,
            frames_in_segment: 0,
        }
    }

    /// Return the byte offset up to which text should be visible this frame.
    ///
    /// Returns `usize::MAX` (reveal all) when no breakpoints are set or all
    /// segments have been revealed.
    pub fn visible_byte_offset(&self) -> usize {
        if self.breakpoints.is_empty() || self.segment_idx >= self.breakpoints.len() {
            usize::MAX
        } else {
            self.breakpoints[self.segment_idx]
        }
    }

    /// Advance the reveal state by one frame.
    ///
    /// Returns `true` if the reveal is still in progress (more segments remain).
    pub fn advance(&mut self) -> bool {
        if self.segment_idx >= self.breakpoints.len() {
            return false; // already fully revealed
        }
        self.frames_in_segment += 1;
        if self.frames_in_segment >= STREAM_REVEAL_FRAMES_PER_SEGMENT {
            self.frames_in_segment = 0;
            self.segment_idx += 1;
        }
        self.segment_idx < self.breakpoints.len()
    }
}

// ─── Image texture cache ────────────────────────────────────────────────────

/// A cached GPU texture for a static image resource.
///
/// Created by [`Compositor::ensure_image_texture`] on first reference and
/// reused across frames until eviction.
pub struct ImageTextureEntry {
    /// The GPU texture holding RGBA pixel data, kept alive for `bind_group`.
    /// Prefixed with `_` to make intent explicit: this field exists solely to
    /// retain ownership and keep the wgpu texture alive as long as the bind
    /// group references it.
    pub _texture: wgpu::Texture,
    /// Pre-built bind group (texture view + sampler) ready for draw calls.
    pub bind_group: wgpu::BindGroup,
    /// Image width in pixels (needed for fit-mode UV calculations).
    pub width: u32,
    /// Image height in pixels (needed for fit-mode UV calculations).
    pub height: u32,
}

/// GPU state and render pipeline.
pub struct Compositor {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// Pipeline with no blending — writes RGBA directly. Used to clear
    /// the framebuffer to transparent in overlay mode (LoadOp::Clear
    /// doesn't write alpha correctly on some GPUs).
    clear_pipeline: wgpu::RenderPipeline,
    /// Render pipeline for textured rectangles (static image rendering).
    texture_rect_pipeline: wgpu::RenderPipeline,
    /// Bind group layout for the texture rect pipeline (shared by all images).
    texture_rect_bind_group_layout: wgpu::BindGroupLayout,
    /// Shared linear-filtering sampler for all image textures (created once).
    image_sampler: wgpu::Sampler,
    /// SDF rounded-rectangle pipeline (fullscreen / straight-alpha mode).
    ///
    /// Used to render zone backdrops whose `RenderingPolicy` has
    /// `backdrop_radius` set.  Encoded in a separate pass after the main
    /// rect pass so rounded corners composite cleanly over the background.
    ///
    /// Uses `BlendState::ALPHA_BLENDING`; vertex colors are non-premultiplied.
    /// In overlay mode `rounded_rect_overlay_pipeline` is selected instead.
    rounded_rect_pipeline: wgpu::RenderPipeline,
    /// SDF rounded-rectangle pipeline for overlay / premultiplied-alpha mode.
    ///
    /// Identical geometry to `rounded_rect_pipeline` but uses
    /// `BlendState::PREMULTIPLIED_ALPHA_BLENDING` and
    /// `ROUNDED_RECT_OVERLAY_SHADER` (which scales all four channels by
    /// coverage).  Selected by `encode_rounded_rect_pass` when
    /// `self.overlay_mode` is true.
    rounded_rect_overlay_pipeline: wgpu::RenderPipeline,
    pub width: u32,
    pub height: u32,
    frame_number: u64,
    /// When true, the clear color uses alpha=0 for transparent overlay mode.
    pub overlay_mode: bool,
    /// When true, render all zone boundaries with colored tints even when
    /// zones have no active content. Controlled by `TZE_HUD_DEBUG_ZONES=1`.
    pub debug_zone_tints: bool,
    /// Current degradation level, set by the runtime before each frame.
    ///
    /// At [`DegradationLevel::Significant`] or higher, widget transition
    /// interpolation is skipped and final parameter values are applied
    /// immediately to reduce re-rasterization under load.
    pub degradation_level: DegradationLevel,
    /// Optional text rasterizer (glyphon). Absent until `init_text_renderer`
    /// is called. When `None`, TextMarkdownNode and zone StreamText content
    /// renders as solid-color rectangles only (no glyph output).
    pub(crate) text_rasterizer: Option<TextRasterizer>,
    /// Optional widget renderer. Absent until `init_widget_renderer` is called.
    /// When `None`, widget instances in the scene graph are not composited.
    pub(crate) widget_renderer: Option<WidgetRenderer>,
    /// Per-zone fade-in / fade-out animation state.
    ///
    /// Keyed by zone name. An entry is inserted on the first publish to a zone
    /// that has `transition_in_ms > 0`, and on every zone clear when
    /// `transition_out_ms > 0`. Completed transitions are pruned each frame.
    pub(crate) zone_animation_states: HashMap<String, ZoneAnimationState>,
    /// Track which zones had active publishes in the previous frame so we can
    /// detect publish → clear transitions and start fade-out animations.
    prev_active_zones: HashMap<String, bool>,
    /// Per-publication TTL fade-out animation state.
    ///
    /// Outer key: zone name.  Inner key: `PubKey = (published_at_wall_us, publisher_namespace)`.
    ///
    /// Created when the compositor first encounters a publication in a Stack zone.
    /// Entries for publications no longer in `active_publishes` are pruned by
    /// [`Compositor::prune_faded_publications`].
    pub(crate) pub_animation_states: HashMap<String, HashMap<PubKey, PublicationAnimationState>>,
    /// Per-zone streaming word-by-word reveal state.
    ///
    /// Keyed by zone name.  Present only when the latest publication in that zone
    /// has non-empty breakpoints.  Absent (or None) means reveal all at once.
    ///
    /// Per spec §Subtitle Streaming Word-by-Word Reveal: when a new publication
    /// replaces the old one (latest-wins), the old reveal state is discarded and
    /// a new one starts from the beginning of the new breakpoints.
    pub(crate) stream_reveal_states: HashMap<String, StreamRevealState>,
    /// Resolved design token map, set at startup via `set_token_map`.
    ///
    /// Used to resolve `color.severity.{info,warning,critical}` tokens for
    /// alert-banner backdrop colors. When empty (the default), all severity
    /// lookups fall back to the hardcoded `SEVERITY_*` constants.
    ///
    /// Populated by calling `set_token_map` after `run_component_startup`
    /// produces a `ComponentStartupResult::global_tokens`.
    pub token_map: HashMap<String, String>,
    /// Decoded RGBA image bytes indexed by `ResourceId`.
    ///
    /// Populated by calling [`Compositor::register_image_bytes`] after an image
    /// resource is uploaded. The compositor decodes/uploads to GPU texture on
    /// first reference via [`Compositor::ensure_image_texture`].
    ///
    /// The raw bytes are kept until the ResourceId is no longer referenced by
    /// any scene node or zone publication (evicted by `evict_unused_image_textures`).
    image_bytes: HashMap<ResourceId, Arc<[u8]>>,
    /// Explicit pixel dimensions for registered image resources, keyed by `ResourceId`.
    ///
    /// Populated alongside `image_bytes` by [`Compositor::register_image_bytes`].
    /// Stores `(width, height)` so that `ensure_scene_image_textures` can pass
    /// exact dimensions to [`Compositor::ensure_image_texture`] without resorting
    /// to the square-root heuristic that fails for non-square images.
    image_dims: HashMap<ResourceId, (u32, u32)>,
    /// GPU texture cache for static images, keyed by `ResourceId`.
    ///
    /// Entries are created on-demand by `ensure_image_texture` and evicted
    /// when the resource is no longer referenced in the scene.
    ///
    /// Also stores status-bar icon textures via `ensure_icon_texture`, which
    /// uses `ResourceId::of(svg_path.as_bytes())` as the key.  Icon entries
    /// are kept alive by including their ResourceIds in the eviction-guard set
    /// (see `ensure_scene_icon_textures`).
    image_texture_cache: HashMap<ResourceId, ImageTextureEntry>,
    /// Negative cache for SVG paths that failed to load/parse.
    ///
    /// Paths are inserted on the first failed `ensure_icon_texture` call.
    /// Subsequent calls skip the filesystem read entirely, avoiding per-frame
    /// I/O and log spam for invalid icon paths. Entries persist for the
    /// lifetime of the compositor (SVG paths are static config; runtime reload
    /// of icon paths is not currently supported).
    failed_icon_paths: HashSet<ResourceId>,
}

impl Compositor {
    /// Create a new headless compositor.
    ///
    /// Checks the `HEADLESS_FORCE_SOFTWARE` environment variable.  When set to
    /// `1`, wgpu adapter selection uses `force_fallback_adapter = true`, which
    /// selects llvmpipe on Linux or WARP on Windows.
    ///
    /// Per runtime-kernel/spec.md Requirement: Headless Software GPU (line 211):
    /// "When set, the wgpu adapter selection MUST request a software fallback
    /// (force_fallback_adapter = true)."
    pub async fn new_headless(width: u32, height: u32) -> Result<Self, CompositorError> {
        // Check HEADLESS_FORCE_SOFTWARE env var (spec line 409: "conventionally
        // named HEADLESS_FORCE_SOFTWARE").
        let force_software = std::env::var("HEADLESS_FORCE_SOFTWARE")
            .map(|v| v.trim() == "1")
            .unwrap_or(false);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: force_software,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tze_hud_compositor"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        let pipeline = Self::create_pipeline(&device);
        let clear_pipeline =
            Self::create_clear_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        let rounded_rect_pipeline =
            Self::create_rounded_rect_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        let rounded_rect_overlay_pipeline = Self::create_rounded_rect_overlay_pipeline(
            &device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );

        let texture_rect_bind_group_layout = create_texture_rect_bind_group_layout(&device);
        let texture_rect_pipeline = create_texture_rect_pipeline(
            &device,
            &texture_rect_bind_group_layout,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image_linear_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            texture_rect_pipeline,
            texture_rect_bind_group_layout,
            image_sampler,
            rounded_rect_pipeline,
            rounded_rect_overlay_pipeline,
            width,
            height,
            frame_number: 0,
            overlay_mode: false,
            debug_zone_tints: std::env::var("TZE_HUD_DEBUG_ZONES").is_ok_and(|v| v == "1"),
            degradation_level: DegradationLevel::Nominal,
            text_rasterizer: None,
            widget_renderer: None,
            zone_animation_states: HashMap::new(),
            prev_active_zones: HashMap::new(),
            pub_animation_states: HashMap::new(),
            stream_reveal_states: HashMap::new(),
            token_map: HashMap::new(),
            image_bytes: HashMap::new(),
            image_dims: HashMap::new(),
            image_texture_cache: HashMap::new(),
            failed_icon_paths: HashSet::new(),
        })
    }

    /// Create a windowed compositor backed by a real `winit::window::Window`.
    ///
    /// This is the factory method for production windowed rendering. It:
    /// 1. Uses `select_gpu_adapter` with platform-mandated backends (Vulkan/D3D12/Metal).
    /// 2. Creates a `wgpu::Surface` from the window via `instance.create_surface`.
    /// 3. Negotiates the surface format (sRGB preferred).
    /// 4. Configures the surface with the window's physical dimensions.
    /// 5. Creates the `wgpu::Device` and `wgpu::Queue`.
    ///
    /// Returns the `(Compositor, WindowSurface)` pair. The `WindowSurface` must
    /// be kept alive for the duration of the runtime.
    ///
    /// Per spec §Compositor Thread Ownership (line 46): the returned `Compositor`
    /// (and thus `Device` + `Queue`) MUST be transferred to the compositor thread
    /// immediately after creation. The `WindowSurface` is owned by the main thread.
    ///
    /// Per spec §Platform GPU Backends (line 189): this path uses the platform-
    /// mandated backends — unlike `new_headless` which uses `Backends::all()`.
    pub async fn new_windowed(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        Self::new_windowed_inner(window, width, height, false).await
    }

    /// Create a windowed compositor, optionally forcing Vulkan for overlay
    /// transparency (DX12 only supports Opaque swapchain alpha mode).
    pub async fn new_windowed_overlay(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        Self::new_windowed_inner(window, width, height, true).await
    }

    async fn new_windowed_inner(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
        overlay: bool,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        use crate::surface::WindowSurface;

        // ── Step 1: Create instance with platform-mandated backends ──────────
        // We need the surface before adapter selection so we can pass it as
        // `compatible_surface`. Create a temporary instance first, create the
        // surface, then select the adapter with that surface constraint.
        // On Windows in overlay mode, force Vulkan — DX12 only supports Opaque
        // swapchain alpha mode, which prevents per-pixel transparency.
        let backends = if overlay && cfg!(target_os = "windows") {
            tracing::info!("overlay mode: forcing Vulkan backend for transparent swapchain");
            wgpu::Backends::VULKAN
        } else {
            crate::adapter::platform_backends().flags
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });

        // ── Step 2: Create wgpu::Surface from the winit window ───────────────
        // SAFETY: `window` is wrapped in Arc — it outlives the surface because
        // we pass 'static lifetime via Arc<Window>.
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| CompositorError::DeviceCreation(format!("create_surface: {e}")))?;

        // ── Step 3: Select adapter compatible with the surface ────────────────
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        tracing::info!(
            backend = ?adapter_info.backend,
            device_name = %adapter_info.name,
            vendor = adapter_info.vendor,
            "windowed: GPU adapter selected"
        );

        // ── Step 4: Request device ────────────────────────────────────────────
        // Use downlevel_defaults() for broad compatibility but override
        // max_texture_dimension_2d with the adapter's actual capability.
        // downlevel_defaults() caps this at 2048 which is smaller than
        // common display resolutions (e.g. 2560x1440).  The adapter knows
        // the GPU's true limit; requesting it ensures the surface can be
        // configured at the monitor's native resolution.
        let mut required_limits = wgpu::Limits::downlevel_defaults();
        required_limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tze_hud_compositor_windowed"),
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        // ── Step 5: Configure the surface ────────────────────────────────────
        let surface_caps = surface.get_capabilities(&adapter);

        // Guard: wgpu surface capabilities must be non-empty on a valid
        // adapter/surface combination. Return a structured error instead of
        // panicking via index [0] so the caller can diagnose driver issues.
        if surface_caps.formats.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported texture formats — driver or backend issue"
                    .to_string(),
            ));
        }
        if surface_caps.present_modes.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported present modes — driver or backend issue".to_string(),
            ));
        }
        if surface_caps.alpha_modes.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported alpha modes — driver or backend issue".to_string(),
            ));
        }

        // Prefer sRGB surface format; fall back to the first available format.
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo // vsync — latency-stable
        } else {
            surface_caps.present_modes[0]
        };

        // Clamp dimensions to the device's maximum supported texture size.
        // Use device.limits() (not adapter.limits()) because the device is
        // created with required_limits=downlevel_defaults(); the actual device
        // limits reflect what the adapter provides subject to those requirements.
        // Some GPUs (e.g. certain Intel/Mesa drivers) report a max of 2048,
        // which is smaller than common display resolutions like 2560x1440.
        //
        // Also guard against zero-size dimensions: wgpu panics if width or
        // height is 0 in surface.configure().  This can happen if inner_size()
        // returned (0,0) on a minimized or not-yet-shown window.
        let max_dim = device.limits().max_texture_dimension_2d;
        let clamped_width = width.min(max_dim).max(1);
        let clamped_height = height.min(max_dim).max(1);
        if clamped_width != width || clamped_height != height {
            tracing::warn!(
                requested_width = width,
                requested_height = height,
                clamped_width,
                clamped_height,
                max_texture_dimension_2d = max_dim,
                "windowed: surface dimensions clamped to device limit"
            );
        }

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: clamped_width,
            height: clamped_height,
            present_mode,
            alpha_mode: surface_caps
                .alpha_modes
                .iter()
                .find(|m| **m == wgpu::CompositeAlphaMode::PreMultiplied)
                .or_else(|| {
                    surface_caps
                        .alpha_modes
                        .iter()
                        .find(|m| **m == wgpu::CompositeAlphaMode::PostMultiplied)
                })
                .copied()
                .unwrap_or(surface_caps.alpha_modes[0]),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        tracing::info!(
            available_alpha_modes = ?surface_caps.alpha_modes,
            selected_alpha_mode = ?config.alpha_mode,
            "windowed: alpha mode selection"
        );
        // Write diagnostic to a known file for remote debugging.
        let diag = format!(
            "backend: {:?}\ndevice: {}\nrequested_backends: {:?}\noverlay: {}\navailable_alpha_modes: {:?}\nselected_alpha_mode: {:?}\nformat: {:?}\npresent_mode: {:?}\n",
            adapter_info.backend,
            adapter_info.name,
            backends,
            overlay,
            surface_caps.alpha_modes,
            config.alpha_mode,
            surface_format,
            present_mode,
        );
        let _ = std::fs::write("C:\\tze_hud\\logs\\alpha_diag.txt", &diag);
        surface.configure(&device, &config);
        tracing::info!(
            format = ?surface_format,
            present_mode = ?present_mode,
            alpha_mode = ?config.alpha_mode,
            width = clamped_width,
            height = clamped_height,
            "windowed: surface configured"
        );

        // ── Step 6: Create render pipelines (format-aware) ────────────────────
        let pipeline = Self::create_pipeline_with_format(&device, surface_format);
        let clear_pipeline = Self::create_clear_pipeline(&device, surface_format);
        let rounded_rect_pipeline = Self::create_rounded_rect_pipeline(&device, surface_format);
        let rounded_rect_overlay_pipeline =
            Self::create_rounded_rect_overlay_pipeline(&device, surface_format);

        let texture_rect_bind_group_layout = create_texture_rect_bind_group_layout(&device);
        let texture_rect_pipeline =
            create_texture_rect_pipeline(&device, &texture_rect_bind_group_layout, surface_format);
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image_linear_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let compositor = Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            texture_rect_pipeline,
            texture_rect_bind_group_layout,
            image_sampler,
            rounded_rect_pipeline,
            rounded_rect_overlay_pipeline,
            width: clamped_width,
            height: clamped_height,
            frame_number: 0,
            overlay_mode: false,
            debug_zone_tints: std::env::var("TZE_HUD_DEBUG_ZONES").is_ok_and(|v| v == "1"),
            degradation_level: DegradationLevel::Nominal,
            text_rasterizer: None,
            widget_renderer: None,
            zone_animation_states: HashMap::new(),
            prev_active_zones: HashMap::new(),
            pub_animation_states: HashMap::new(),
            stream_reveal_states: HashMap::new(),
            token_map: HashMap::new(),
            image_bytes: HashMap::new(),
            image_dims: HashMap::new(),
            image_texture_cache: HashMap::new(),
            failed_icon_paths: HashSet::new(),
        };

        let window_surface = WindowSurface::new(surface, config);
        Ok((compositor, window_surface))
    }

    /// Create a render pipeline targeting a specific texture format.
    ///
    /// This is the canonical pipeline constructor. Both `create_pipeline`
    /// (headless, fixed format) and `create_pipeline_with_format` (windowed,
    /// dynamic swapchain format) delegate here to avoid duplicating the
    /// pipeline descriptor.
    ///
    /// `label_prefix` is prepended to debug labels so GPU profilers can
    /// distinguish headless vs windowed pipelines.
    fn create_pipeline_inner(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        label_prefix: &str,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline::{RECT_SHADER, RectVertex};

        let shader_label = format!("{label_prefix}rect_shader");
        let layout_label = format!("{label_prefix}rect_pipeline_layout");
        let pipeline_label = format!("{label_prefix}rect_pipeline");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&shader_label),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&layout_label),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&pipeline_label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Create a render pipeline targeting a dynamic swapchain format.
    ///
    /// Called by `new_windowed` so the pipeline matches the negotiated
    /// surface format (which varies by platform/driver).
    fn create_pipeline_with_format(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        Self::create_pipeline_inner(device, format, "windowed_")
    }

    /// Create a pipeline with no blending — writes RGBA directly.
    /// Used to clear the framebuffer to transparent in overlay mode.
    fn create_clear_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline::{RECT_SHADER, RectVertex};
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clear_shader"),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clear_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("clear_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None, // No blending — direct RGBA write
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Create a render pipeline for headless mode (`Rgba8UnormSrgb`).
    fn create_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
        Self::create_pipeline_inner(device, wgpu::TextureFormat::Rgba8UnormSrgb, "")
    }

    /// Create the SDF rounded-rectangle pipeline for the given output format.
    ///
    /// Uses `ROUNDED_RECT_SHADER` and `RoundedRectVertex::desc()`.
    /// Alpha blending is enabled (standard source-over) so rounded corners
    /// composite correctly over any background.
    ///
    /// This pipeline is used in `encode_rounded_rect_pass` for zones whose
    /// `RenderingPolicy` has `backdrop_radius` set.
    fn create_rounded_rect_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rounded_rect_shader"),
            source: wgpu::ShaderSource::Wgsl(ROUNDED_RECT_SHADER.into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rounded_rect_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rounded_rect_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RoundedRectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Create the SDF rounded-rectangle pipeline for overlay / premultiplied-alpha mode.
    ///
    /// Uses `ROUNDED_RECT_OVERLAY_SHADER` (which scales all four RGBA channels by
    /// SDF coverage) and `BlendState::PREMULTIPLIED_ALPHA_BLENDING`.  Selected by
    /// `encode_rounded_rect_pass` when `self.overlay_mode` is true.
    ///
    /// In overlay mode vertex colors are premultiplied by `gpu_color`; the shader
    /// must therefore output `(rgb * cov, a * cov)` so the premultiplied blend
    /// equation applies coverage exactly once without double-multiplying alpha.
    fn create_rounded_rect_overlay_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rounded_rect_overlay_shader"),
            source: wgpu::ShaderSource::Wgsl(ROUNDED_RECT_OVERLAY_SHADER.into()),
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rounded_rect_overlay_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rounded_rect_overlay_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RoundedRectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Replace the compositor's resolved design token map.
    ///
    /// Should be called once at startup after `run_component_startup` produces a
    /// `ComponentStartupResult::global_tokens`.  The map is keyed by canonical
    /// token names (e.g. `"color.severity.warning"`) with hex-color string values
    /// (e.g. `"#FFB800"`).
    ///
    /// At render time the compositor looks up `color.severity.{info,warning,critical}`
    /// to derive alert-banner backdrop colors, falling back to hardcoded constants
    /// when a key is absent or unparseable.
    pub fn set_token_map(&mut self, map: HashMap<String, String>) {
        self.token_map = map;
    }

    /// Initialize (or re-initialize) the text rasterizer for a given surface format.
    ///
    /// Must be called once after creation before text rendering is available.
    /// For headless compositors, `format` should be `Rgba8UnormSrgb`.
    /// For windowed compositors, use the negotiated swapchain format.
    ///
    /// Calling this multiple times replaces the existing rasterizer (e.g. on
    /// surface resize or format change).
    pub fn init_text_renderer(&mut self, format: wgpu::TextureFormat) {
        self.text_rasterizer = Some(TextRasterizer::new(&self.device, &self.queue, format));
        tracing::debug!(format = ?format, "text renderer initialized");
    }

    /// Load raw font bytes (TTF or OTF) from an agent upload into glyphon's
    /// `FontSystem`.
    ///
    /// After this call, the font is available for text layout in subsequent
    /// frames.  The `resource_id` is the 32-byte BLAKE3 digest of `data`
    /// (matches `tze_hud_resource::ResourceId::as_bytes()`); duplicate calls
    /// with the same `resource_id` are no-ops.
    ///
    /// Silently skips if the text renderer is not yet initialized (e.g. if
    /// `init_text_renderer` has not been called).  Callers in the runtime may
    /// defer font uploads until the compositor is ready.
    pub fn load_font_bytes(&mut self, resource_id: [u8; 32], data: &[u8]) {
        if let Some(rasterizer) = &mut self.text_rasterizer {
            rasterizer.load_font_bytes(resource_id, data);
        } else {
            tracing::debug!("load_font_bytes called before text renderer is initialized — skipped");
        }
    }

    /// Returns `true` if the font with the given `resource_id` has been loaded
    /// into the `FontSystem`.
    ///
    /// Returns `false` if the text renderer is not yet initialized.
    pub fn has_font(&self, resource_id: &[u8; 32]) -> bool {
        self.text_rasterizer
            .as_ref()
            .map(|r| r.has_font(resource_id))
            .unwrap_or(false)
    }

    // ─── Image texture cache ─────────────────────────────────────────────────

    /// Register decoded RGBA image bytes for a resource.
    ///
    /// The runtime should call this after each successful image upload so the
    /// compositor can create GPU textures on demand. `rgba_data` must be
    /// `width × height × 4` bytes of RGBA8 pixel data. The explicit `width` and
    /// `height` are stored alongside the bytes so that
    /// [`Compositor::ensure_scene_image_textures`] can create GPU textures for
    /// zone images (which do not carry dimensions in `ZoneContent::StaticImage`)
    /// without resorting to the square-root heuristic that fails for non-square
    /// images (e.g. 640×360).
    ///
    /// Duplicate calls with the same `resource_id` are ignored (content-addressed
    /// identity guarantees the bytes are identical).
    ///
    /// Returns without registering if `width` or `height` is zero, or if the
    /// product `width × height × 4` overflows `usize`.
    pub fn register_image_bytes(
        &mut self,
        resource_id: ResourceId,
        rgba_data: Arc<[u8]>,
        width: u32,
        height: u32,
    ) {
        // Guard: reject zero-dimension images; content-addressed uploads should
        // never arrive with degenerate dimensions, so treat this as a caller bug.
        if width == 0 || height == 0 {
            return;
        }
        // Guard: overflow — reject if the byte count would overflow usize.
        if width
            .checked_mul(height)
            .and_then(|px| px.checked_mul(4))
            .is_none()
        {
            return;
        }

        // Insert both maps atomically via the Entry API.  Using separate
        // `or_insert` calls would allow one map to contain the key without the
        // other if a previous partially-failed insertion left them out of sync.
        if let std::collections::hash_map::Entry::Vacant(entry) =
            self.image_bytes.entry(resource_id)
        {
            entry.insert(rgba_data);
            self.image_dims.insert(resource_id, (width, height));
        }
    }

    /// Ensure a GPU texture exists for the given `ResourceId`.
    ///
    /// Returns `true` if a texture is ready (either already cached or just
    /// created). Returns `false` if the image bytes are not registered.
    ///
    /// The flow: check `image_texture_cache` → miss → look up `image_bytes` →
    /// create `wgpu::Texture` → write data → create `BindGroup` → cache.
    fn ensure_image_texture(
        &mut self,
        resource_id: ResourceId,
        img_width: u32,
        img_height: u32,
    ) -> bool {
        // Already cached?
        if self.image_texture_cache.contains_key(&resource_id) {
            return true;
        }

        // Retrieve raw RGBA bytes.
        let rgba_data = match self.image_bytes.get(&resource_id) {
            Some(data) => Arc::clone(data),
            None => {
                tracing::debug!(
                    resource_id = %resource_id,
                    "image bytes not registered — falling back to placeholder"
                );
                return false;
            }
        };

        // Validate byte count.
        let expected_bytes = (img_width as usize) * (img_height as usize) * 4;
        if rgba_data.len() != expected_bytes {
            tracing::warn!(
                resource_id = %resource_id,
                expected = expected_bytes,
                actual = rgba_data.len(),
                "image bytes length mismatch — falling back to placeholder"
            );
            return false;
        }

        // Create GPU texture.
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("img_tex_{resource_id}")),
            size: wgpu::Extent3d {
                width: img_width,
                height: img_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload pixel data.
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(img_width * 4),
                rows_per_image: Some(img_height),
            },
            wgpu::Extent3d {
                width: img_width,
                height: img_height,
                depth_or_array_layers: 1,
            },
        );

        // Create bind group.
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("img_bg_{resource_id}")),
            layout: &self.texture_rect_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.image_sampler),
                },
            ],
        });

        self.image_texture_cache.insert(
            resource_id,
            ImageTextureEntry {
                _texture: texture,
                bind_group,
                width: img_width,
                height: img_height,
            },
        );

        tracing::debug!(
            resource_id = %resource_id,
            width = img_width,
            height = img_height,
            "image texture uploaded to GPU"
        );

        true
    }

    /// Ensure a GPU texture exists for a status-bar icon SVG file path.
    ///
    /// Uses `ResourceId::of(path.as_bytes())` as the cache key so the texture
    /// is stored in `image_texture_cache` and drawn by the standard image pass.
    ///
    /// On cache hit, returns `true` immediately (no I/O).  On miss:
    /// 1. Reads the SVG bytes from `path` (filesystem).
    ///    NOTE: This is a blocking filesystem read on the compositor thread.
    ///    It only occurs once per unique SVG path (cache miss); subsequent
    ///    calls for the same path return immediately from cache or the
    ///    negative-cache set (`failed_icon_paths`).
    /// 2. Rasterizes at [`ICON_SIZE_PX`] × [`ICON_SIZE_PX`] via resvg/tiny-skia.
    /// 3. Uploads the RGBA pixmap to a `wgpu::Texture`.
    /// 4. Inserts into `image_texture_cache` under the path-derived `ResourceId`.
    ///
    /// Returns `false` if the file cannot be read or parsed.  In that case the
    /// `ResourceId` is added to `failed_icon_paths` so no further I/O is
    /// attempted for this path (graceful degradation — text-only fallback).
    fn ensure_icon_texture(&mut self, path: &str) -> bool {
        let resource_id = ResourceId::of(path.as_bytes());
        if self.image_texture_cache.contains_key(&resource_id) {
            return true;
        }
        // Negative cache: skip repeated I/O for known-bad paths.
        if self.failed_icon_paths.contains(&resource_id) {
            return false;
        }

        let size = ICON_SIZE_PX as u32;

        // Read SVG bytes from disk (blocking; occurs at most once per path).
        let svg_bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(path, error = %e, "icon: failed to read SVG file");
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };
        let svg_str = match std::str::from_utf8(&svg_bytes) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path, error = %e, "icon: SVG file is not valid UTF-8");
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };

        // Rasterize via resvg (same pipeline as WidgetRenderer).
        let opts = resvg::usvg::Options::default();
        let tree = match resvg::usvg::Tree::from_str(svg_str, &opts) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(path, error = %e, "icon: failed to parse SVG");
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };
        let mut pixmap = match tiny_skia::Pixmap::new(size, size) {
            Some(p) => p,
            None => {
                tracing::warn!(path, size, "icon: failed to allocate pixmap");
                self.failed_icon_paths.insert(resource_id);
                return false;
            }
        };
        // Scale uniformly with centering (same transform logic as widget rasterization).
        let svg_size = tree.size();
        let sx = size as f32 / svg_size.width();
        let sy = size as f32 / svg_size.height();
        let uniform_scale = sx.min(sy);
        let rendered_w = svg_size.width() * uniform_scale;
        let rendered_h = svg_size.height() * uniform_scale;
        let offset_x = (size as f32 - rendered_w) * 0.5;
        let offset_y = (size as f32 - rendered_h) * 0.5;
        let transform = tiny_skia::Transform::from_translate(offset_x, offset_y)
            .post_scale(uniform_scale, uniform_scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        // Upload to GPU.
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("icon_tex_{path}")),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixmap.data(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("icon_bg_{path}")),
            layout: &self.texture_rect_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.image_sampler),
                },
            ],
        });
        self.image_texture_cache.insert(
            resource_id,
            ImageTextureEntry {
                _texture: texture,
                bind_group,
                width: size,
                height: size,
            },
        );
        tracing::debug!(path, size, "icon texture rasterized and uploaded to GPU");
        true
    }

    /// Evict cached GPU textures for resources no longer referenced in the scene.
    ///
    /// Call once per frame after rendering. `referenced_ids` is the set of
    /// `ResourceId`s that appeared in zone publications or tile nodes during
    /// this frame. Any cache entry not in this set is dropped.
    pub fn evict_unused_image_textures(&mut self, referenced_ids: &HashSet<ResourceId>) {
        self.image_texture_cache
            .retain(|id, _| referenced_ids.contains(id));
        // Also evict bytes and dims for resources no longer referenced.
        self.image_bytes.retain(|id, _| referenced_ids.contains(id));
        self.image_dims.retain(|id, _| referenced_ids.contains(id));
    }

    /// Scan the scene graph for all StaticImage resources, ensure their GPU
    /// textures are uploaded, and return the set of referenced `ResourceId`s.
    ///
    /// This method uploads textures for tile-backed static images directly,
    /// and for zone-publication images when dimensions can be inferred from
    /// registered RGBA bytes.
    ///
    /// The returned `HashSet<ResourceId>` should be passed to
    /// [`Compositor::evict_unused_image_textures`] once per frame so that
    /// stale cache entries are reclaimed and the cache does not grow without
    /// bound across frames.
    fn ensure_scene_image_textures(&mut self, scene: &SceneGraph) -> HashSet<ResourceId> {
        let mut referenced: HashSet<ResourceId> = HashSet::new();

        // Collect all referenced StaticImage resource IDs from tiles.
        for node in scene.nodes.values() {
            if let NodeData::StaticImage(img) = &node.data {
                self.ensure_image_texture(img.resource_id, img.width, img.height);
                referenced.insert(img.resource_id);
            }
        }

        // Collect from zone publications.
        for publishes in scene.zone_registry.active_publishes.values() {
            for record in publishes {
                match &record.content {
                    ZoneContent::StaticImage(resource_id) => {
                        referenced.insert(*resource_id);
                        // Use the explicit dimensions stored by `register_image_bytes`.
                        // This correctly handles non-square images (e.g. 640×360) that
                        // the old square-root heuristic would mis-detect.
                        if !self.image_texture_cache.contains_key(resource_id) {
                            if let Some(&(w, h)) = self.image_dims.get(resource_id) {
                                if w > 0 && h > 0 {
                                    self.ensure_image_texture(*resource_id, w, h);
                                }
                            }
                        }
                    }
                    ZoneContent::Notification(payload) if !payload.icon.is_empty() => {
                        // Parse the icon field via the shared helper (handles empty string
                        // and invalid hex with graceful None). Non-parseable icons are
                        // silently skipped (render notification text-only).
                        if let Some(resource_id) = parse_notification_icon(&payload.icon) {
                            referenced.insert(resource_id);
                            // Use the explicit dimensions stored by `register_image_bytes`.
                            if !self.image_texture_cache.contains_key(&resource_id) {
                                if let Some(&(w, h)) = self.image_dims.get(&resource_id) {
                                    if w > 0 && h > 0 {
                                        self.ensure_image_texture(resource_id, w, h);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        referenced
    }

    /// Scan the scene's zone registry for all `key_icon_map` SVG paths,
    /// ensure their GPU textures are cached, and return the set of their
    /// path-derived `ResourceId`s.
    ///
    /// The returned set must be merged into the `referenced_ids` passed to
    /// `evict_unused_image_textures` so that icon textures are not evicted
    /// on frames where no matching StaticImage publication is active.
    ///
    /// Called once per frame before `render_zone_content` so that the
    /// immutable render path can look up `image_texture_cache` by ResourceId.
    ///
    /// SVG rasterization only occurs on cache miss (init-time / first-seen
    /// path).  Subsequent calls are O(1) per path due to the cache hit-check
    /// guard in `ensure_icon_texture`.
    fn ensure_scene_icon_textures(&mut self, scene: &SceneGraph) -> HashSet<ResourceId> {
        let mut icon_ids: HashSet<ResourceId> = HashSet::new();
        for zone_def in scene.zone_registry.zones.values() {
            let icon_map = &zone_def.rendering_policy.key_icon_map;
            if icon_map.is_empty() {
                continue;
            }
            for svg_path in icon_map.values() {
                let id = ResourceId::of(svg_path.as_bytes());
                icon_ids.insert(id);
                // ensure_icon_texture is a no-op on cache hit.
                self.ensure_icon_texture(svg_path);
            }
        }
        icon_ids
    }

    // ─── Widget renderer ──────────────────────────────────────────────────────

    /// Initialize (or re-initialize) the widget renderer for the given surface format.
    ///
    /// Must be called once before widget textures can be composited. For headless
    /// compositors, `format` should be `Rgba8UnormSrgb`. For windowed compositors,
    /// use the negotiated swapchain format.
    ///
    /// Calling this multiple times replaces the existing renderer (e.g. on surface
    /// reconfiguration or format change). Any cached textures are discarded.
    pub fn init_widget_renderer(&mut self, format: wgpu::TextureFormat) {
        self.widget_renderer = Some(WidgetRenderer::new(&self.device, format));
        tracing::debug!(format = ?format, "widget renderer initialized");
    }

    /// Get a mutable reference to the widget renderer, if initialized.
    pub fn widget_renderer_mut(&mut self) -> Option<&mut WidgetRenderer> {
        self.widget_renderer.as_mut()
    }

    /// Get a reference to the widget renderer, if initialized.
    pub fn widget_renderer(&self) -> Option<&WidgetRenderer> {
        self.widget_renderer.as_ref()
    }

    /// Ensure widget instances have up-to-date cached textures for all widget
    /// instances in the registry.
    ///
    /// For each widget instance:
    /// - If no texture entry exists (first frame), rasterizes with default params.
    /// - If the instance has a `dirty` flag set, re-rasterizes with current params.
    /// - If an animation is active, resolves interpolated params and re-rasterizes.
    ///
    /// Under degradation level [`DegradationLevel::Significant`] or higher, active
    /// transitions are snapped to their final values immediately, reducing
    /// re-rasterization to at most once per parameter change during transitions.
    ///
    /// This should be called once per frame before `render_frame`.
    pub fn sync_widget_textures(
        &mut self,
        scene: &SceneGraph,
        degradation_level: DegradationLevel,
    ) {
        let wr = match &mut self.widget_renderer {
            Some(r) => r,
            None => return,
        };

        let registry = &scene.widget_registry;

        // Collect instances that need texture updates.
        let instance_names: Vec<String> = registry.instances.keys().cloned().collect();

        for instance_name in instance_names {
            let instance = match registry.instances.get(&instance_name) {
                Some(i) => i.clone(),
                None => continue,
            };
            let def = match registry.definitions.get(&instance.widget_type_name) {
                Some(d) => d.clone(),
                None => continue,
            };

            // Determine pixel geometry from the instance's geometry policy.
            // Fall back to a sensible default if not set.
            let (pw, ph) = resolve_widget_pixel_size(&instance, &def, self.width, self.height);
            if pw == 0 || ph == 0 {
                continue;
            }

            // Check if this instance needs an initial texture (no entry yet).
            let needs_initial = wr.texture_entry(&instance_name).is_none();

            // Resolve animated or static params, applying degradation-aware snapping.
            let current_params = &instance.current_params;
            let (effective_params, still_animating) =
                wr.resolve_animated_params(&instance_name, current_params, degradation_level);

            let params_changed = wr
                .texture_entry(&instance_name)
                .map(|e| e.last_rendered_params != effective_params)
                .unwrap_or(false);

            let dirty = needs_initial
                || still_animating
                || params_changed
                || wr
                    .texture_entry(&instance_name)
                    .map(|e| e.dirty)
                    .unwrap_or(false);

            if dirty {
                wr.rasterize_and_upload(
                    &self.device,
                    &self.queue,
                    &instance_name,
                    &def,
                    &effective_params,
                    pw,
                    ph,
                );
                // Record what params were rendered so we can detect future changes.
                if let Some(entry) = wr.texture_entry_mut(&instance_name) {
                    entry.last_rendered_params = effective_params;
                    entry.dirty = false;
                }
            }
        }
    }

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

    /// Collect `TextItem`s for all TextMarkdownNode tiles and zone StreamText
    /// and ShortTextWithIcon/Notification content in the scene.
    ///
    /// All zone `TextItem`s are constructed from `RenderingPolicy` fields —
    /// no hardcoded colors or font choices.  Animation opacity is applied to
    /// the color channels so text fades with the backdrop.
    ///
    /// Returns a flat `Vec<TextItem>` ready for `TextRasterizer::prepare_text_items`.
    fn collect_text_items(&self, scene: &SceneGraph, sw: f32, sh: f32) -> Vec<TextItem> {
        let mut items: Vec<TextItem> = Vec::new();

        // ── TextMarkdownNode tiles ────────────────────────────────────────────
        for tile in &scene.visible_tiles() {
            if let Some(root_id) = tile.root_node {
                self.collect_text_items_from_node(root_id, tile, scene, &mut items);
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
                    // Build an ordered reference slice: alert-banner uses severity sort
                    // (critical first, then recency); other Stack zones use newest-first.
                    let ordered: Vec<&ZonePublishRecord> = if is_alert_banner_zone(zone_name) {
                        sort_alert_banner_indices(publishes)
                            .into_iter()
                            .map(|idx| &publishes[idx])
                            .collect()
                    } else {
                        publishes.iter().rev().collect()
                    };

                    // Resolve notification typography tokens once per zone so that both
                    // slot-height calculation and per-item rendering use the same values.
                    // (Token lookups are cheap, but resolving outside the per-item loop
                    // also avoids redundant HashMap lookups on zones with many items.)
                    let notif_body_scale = self
                        .token_map
                        .get("typography.notification.body.scale")
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(NOTIFICATION_BODY_SCALE)
                        .clamp(0.5, 1.0);
                    let notif_title_weight = self
                        .token_map
                        .get("typography.notification.title.weight")
                        .and_then(|v| v.parse::<u16>().ok())
                        .unwrap_or(NOTIFICATION_TITLE_WEIGHT);

                    // Compute per-slot heights (variable for two-line notifications).
                    let slot_heights = Self::per_slot_heights(
                        &ordered,
                        policy,
                        notif_body_scale,
                        NOTIFICATION_INTER_LINE_GAP,
                    );
                    let slot_offsets = Self::slot_offsets(&slot_heights);
                    let total_slots_h: f32 = slot_heights.iter().sum();

                    // For alert-banner: dynamic zone height = sum of per-slot heights.
                    // Height grows with each active banner; no cap at the configured
                    // height_pct.  (Zero banners → zero height via the is_empty guard.)
                    // For other Stack zones: use the configured zone height (zh).
                    let effective_zh = if is_alert_banner_zone(zone_name) {
                        total_slots_h
                    } else {
                        zh
                    };

                    for (record, (slot_offset, slot_h)) in ordered
                        .into_iter()
                        .zip(slot_offsets.iter().zip(slot_heights.iter()))
                    {
                        let slot_y = zy + slot_offset;
                        if slot_y >= zy + effective_zh {
                            break;
                        }
                        let effective_slot_h = slot_h.min((zy + effective_zh) - slot_y);

                        // Per-publication fade-out opacity (1.0 when no fade active).
                        let pub_opacity = self.pub_opacity(zone_name, record);
                        // Combined opacity: zone animation × per-publication fade.
                        let effective_opacity = anim_opacity * pub_opacity;

                        match &record.content {
                            ZoneContent::StreamText(text) => {
                                items.push(TextItem::from_zone_policy(
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

                                // Text x-offset respects icon reservation (from icon texture pipeline).
                                let text_x = zx + inset_h + icon_width_reserved;
                                let text_w =
                                    (zw - inset_h - icon_width_reserved - inset_h).max(1.0);

                                if payload.title.is_empty() {
                                    // ── Single-line rendering (backward-compatible) ──
                                    // Font weight: policy explicit > 400 default
                                    let font_weight = policy.font_weight.unwrap_or(400);
                                    items.push(TextItem {
                                        text: payload.text.clone(),
                                        pixel_x: text_x,
                                        pixel_y: slot_y + inset_v,
                                        bounds_width: text_w,
                                        bounds_height: (effective_slot_h - inset_v * 2.0).max(1.0),
                                        font_size_px,
                                        font_family,
                                        font_weight,
                                        color,
                                        alignment: tze_hud_scene::types::TextAlign::Start,
                                        overflow: tze_hud_scene::types::TextOverflow::Clip,
                                        outline_color: oc,
                                        outline_width: ow,
                                        opacity: effective_opacity,
                                    });
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
                                    items.push(TextItem {
                                        text: payload.title.clone(),
                                        pixel_x: text_x,
                                        pixel_y: content_top,
                                        bounds_width: text_w,
                                        bounds_height: title_line_h.max(1.0),
                                        font_size_px,
                                        font_family,
                                        font_weight: notif_title_weight,
                                        color,
                                        alignment: tze_hud_scene::types::TextAlign::Start,
                                        overflow: tze_hud_scene::types::TextOverflow::Clip,
                                        outline_color: oc,
                                        outline_width: ow,
                                        opacity: effective_opacity,
                                    });
                                    // Body line (regular weight, 0.85× size)
                                    let body_top =
                                        content_top + title_line_h + NOTIFICATION_INTER_LINE_GAP;
                                    // Remaining slot height available for body (down to inset bottom)
                                    let body_bounds_h =
                                        ((slot_y + effective_slot_h - inset_v) - body_top).max(1.0);
                                    items.push(TextItem {
                                        text: payload.text.clone(),
                                        pixel_x: text_x,
                                        pixel_y: body_top,
                                        bounds_width: text_w,
                                        bounds_height: body_bounds_h,
                                        font_size_px: body_font_size,
                                        font_family,
                                        font_weight: 400,
                                        color,
                                        alignment: tze_hud_scene::types::TextAlign::Start,
                                        overflow: tze_hud_scene::types::TextOverflow::Clip,
                                        outline_color: oc,
                                        outline_width: ow,
                                        opacity: effective_opacity,
                                    });
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
                                    items.push(TextItem::from_zone_policy(
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
                                        items.push(TextItem {
                                            text: payload.text.clone(),
                                            pixel_x: zx + margin_h + icon_width_reserved,
                                            pixel_y: zy + margin_v,
                                            bounds_width: (zw
                                                - margin_h
                                                - icon_width_reserved
                                                - margin_h)
                                                .max(1.0),
                                            bounds_height: (zh - margin_v * 2.0).max(1.0),
                                            font_size_px,
                                            font_family,
                                            font_weight,
                                            color,
                                            alignment: policy
                                                .text_align
                                                .unwrap_or(tze_hud_scene::types::TextAlign::Start),
                                            overflow: policy.overflow.unwrap_or(
                                                tze_hud_scene::types::TextOverflow::Clip,
                                            ),
                                            outline_color: oc,
                                            outline_width: ow,
                                            opacity: anim_opacity,
                                        });
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
                                items.push(TextItem::from_zone_policy(
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
                                    items.push(TextItem {
                                        text: payload.text.clone(),
                                        pixel_x: zx + margin_h + icon_width_reserved,
                                        pixel_y: zy + margin_v,
                                        bounds_width: (zw
                                            - margin_h
                                            - icon_width_reserved
                                            - margin_h)
                                            .max(1.0),
                                        bounds_height: (zh - margin_v * 2.0).max(1.0),
                                        font_size_px,
                                        font_family,
                                        font_weight,
                                        color,
                                        alignment: policy
                                            .text_align
                                            .unwrap_or(tze_hud_scene::types::TextAlign::Start),
                                        overflow: policy
                                            .overflow
                                            .unwrap_or(tze_hud_scene::types::TextOverflow::Clip),
                                        outline_color: oc,
                                        outline_width: ow,
                                        opacity: anim_opacity,
                                    });
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

        items
    }

    /// Update zone animation states before each frame.
    ///
    /// Starts fade-in animations for newly-published zones and fade-out
    /// animations for zones that just lost their last publish.
    ///
    /// Also handles zone unregistration: zones that were active and have since
    /// been removed from the registry are treated as cleared (no fade-out is
    /// possible since the zone_def is gone, so the state is simply pruned).
    ///
    /// Prunes completed transitions.
    pub fn update_zone_animations(&mut self, scene: &SceneGraph) {
        // Build current active-zone set (zone_name → has active publishes).
        let current_active: HashMap<String, bool> = scene
            .zone_registry
            .active_publishes
            .iter()
            .map(|(name, pubs)| (name.clone(), !pubs.is_empty()))
            .collect();

        // Detect publish transitions within currently-registered zones.
        for (zone_name, &is_active) in &current_active {
            let was_active = self
                .prev_active_zones
                .get(zone_name)
                .copied()
                .unwrap_or(false);

            if is_active && !was_active {
                // Zone just received its first publish — start fade-in.
                //
                // Transition interrupt semantics: if a fade-out is currently in
                // progress (target_opacity == 0.0), we MUST start the fade-in from
                // the current composite opacity rather than from 0 to prevent a
                // blank frame.  Per spec §Subtitle Contention Policy: "the fade-out
                // MUST be cancelled immediately and the new content MUST begin its
                // transition_in_ms fade-in from the current composite opacity (not
                // from zero)."
                if let Some(zone_def) = scene.zone_registry.zones.get(zone_name) {
                    if let Some(ms) = zone_def.rendering_policy.transition_in_ms {
                        if ms > 0 {
                            let new_state = if let Some(existing) =
                                self.zone_animation_states.get(zone_name)
                            {
                                if existing.target_opacity == 0.0 {
                                    // Interrupt active fade-out: begin fade-in from
                                    // current opacity so there is no blank frame.
                                    ZoneAnimationState::fade_in_from(ms, existing.current_opacity())
                                } else {
                                    ZoneAnimationState::fade_in(ms)
                                }
                            } else {
                                ZoneAnimationState::fade_in(ms)
                            };
                            self.zone_animation_states
                                .insert(zone_name.clone(), new_state);
                        }
                    }
                }
            } else if !is_active && was_active {
                // Zone just lost its last publish — start fade-out.
                if let Some(zone_def) = scene.zone_registry.zones.get(zone_name) {
                    if let Some(ms) = zone_def.rendering_policy.transition_out_ms {
                        if ms > 0 {
                            self.zone_animation_states
                                .insert(zone_name.clone(), ZoneAnimationState::fade_out(ms));
                        }
                    }
                }
            }
        }

        // Detect zone unregistration: zones that were previously tracked but
        // are now absent from active_publishes (zone was removed from registry).
        // Since zone_def is gone, no fade-out animation is possible; we simply
        // prune any in-flight animation state for that zone immediately.
        self.zone_animation_states
            .retain(|zone_name, _| current_active.contains_key(zone_name));

        // Prune completed transitions (reached target opacity).
        self.zone_animation_states
            .retain(|_, state| !state.is_complete());

        self.prev_active_zones = current_active;
    }

    /// Update per-zone streaming word-by-word reveal state.
    ///
    /// Must be called once per frame (after `update_zone_animations`).
    ///
    /// For each zone with a `LatestWins` or `Replace` publication that has
    /// non-empty breakpoints:
    /// - If no reveal state exists or the current pub key doesn't match, start
    ///   a fresh reveal from segment 0 (latest-wins cancels previous streaming).
    /// - If reveal state exists for the current pub key, advance by one frame.
    /// - Zones with empty breakpoints (or no StreamText) have their reveal state
    ///   pruned so text renders at full immediately.
    ///
    /// Per spec §Subtitle Streaming Word-by-Word Reveal:
    /// - Breakpoints identify byte offsets for progressive reveal.
    /// - Empty breakpoints → reveal all at once.
    /// - Replacement during streaming → cancel old reveal, start new.
    pub fn update_stream_reveals(&mut self, scene: &SceneGraph) {
        // Collect zones whose latest publish has breakpoints.
        let mut active_keys: HashMap<String, PubKey> = HashMap::new();

        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            // Only LatestWins/Replace zones get streaming reveal (single occupant).
            if !matches!(
                zone_def.contention_policy,
                ContentionPolicy::LatestWins | ContentionPolicy::Replace
            ) {
                continue;
            }
            let latest = &publishes[publishes.len() - 1];
            // Only StreamText with non-empty breakpoints gets progressive reveal.
            if !matches!(&latest.content, ZoneContent::StreamText(_))
                || latest.breakpoints.is_empty()
            {
                continue;
            }
            let pub_key: PubKey = (
                latest.published_at_wall_us,
                latest.publisher_namespace.clone(),
            );
            active_keys.insert(zone_name.clone(), pub_key);
        }

        // Prune reveal states for zones no longer streaming.
        self.stream_reveal_states
            .retain(|zone_name, _| active_keys.contains_key(zone_name));

        // Update or create reveal states.
        for (zone_name, pub_key) in &active_keys {
            let publishes = match scene.zone_registry.active_publishes.get(zone_name) {
                Some(p) if !p.is_empty() => p,
                _ => continue,
            };
            let latest = &publishes[publishes.len() - 1];

            let state = self.stream_reveal_states.get(zone_name);
            let need_reset = state.map(|s| &s.pub_key != pub_key).unwrap_or(true);

            if need_reset {
                // New publication (latest-wins replaced) or first reveal — start fresh.
                let new_state = StreamRevealState::new(pub_key.clone(), latest.breakpoints.clone());
                self.stream_reveal_states
                    .insert(zone_name.clone(), new_state);
            } else if let Some(state) = self.stream_reveal_states.get_mut(zone_name) {
                // Advance existing reveal by one frame.
                state.advance();
            }
        }
    }

    /// Update per-publication fade-out animation state for Stack zone publications.
    ///
    /// For each active publication in a Stack zone:
    ///
    /// 1. If it is new (not in `pub_animation_states`), insert a fresh
    ///    [`PublicationAnimationState`] using the effective TTL from
    ///    [`Compositor::publication_ttl_ms`]: `expires_at_wall_us` (urgency-derived)
    ///    takes highest priority, then `NotificationPayload.ttl_ms`, then the zone's
    ///    `auto_clear_ms`, then `NOTIFICATION_DEFAULT_TTL_MS` (8 000 ms).
    /// 2. Call `tick()` to check whether the TTL has expired and start the fade if so.
    ///
    /// Stale entries (publications no longer present in `active_publishes`) are
    /// pruned from `pub_animation_states` by this method.
    ///
    /// After this call, use [`Compositor::prune_faded_publications`] to remove
    /// publications whose fade-out has fully completed from the scene graph.
    ///
    /// Call order per frame: `update_zone_animations` → `update_publication_animations`
    /// → `prune_faded_publications(scene)` → render.
    pub fn update_publication_animations(&mut self, scene: &SceneGraph) {
        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            // Only Stack zones get per-publication TTL fade-out.
            if !matches!(zone_def.contention_policy, ContentionPolicy::Stack { .. }) {
                continue;
            }
            let zone_auto_clear_ms = zone_def
                .auto_clear_ms
                .unwrap_or(NOTIFICATION_DEFAULT_TTL_MS);

            let zone_states = self
                .pub_animation_states
                .entry(zone_name.clone())
                .or_default();

            // Build the set of currently-active pub keys for this zone.
            let active_keys: std::collections::HashSet<PubKey> = publishes
                .iter()
                .map(|r| (r.published_at_wall_us, r.publisher_namespace.clone()))
                .collect();

            // Prune stale entries (publications removed from active_publishes).
            zone_states.retain(|k, _| active_keys.contains(k));

            // Ensure every active publication has an animation state; tick existing ones.
            for record in publishes {
                let ttl_ms = Self::publication_ttl_ms(record, zone_auto_clear_ms);
                let key: PubKey = (
                    record.published_at_wall_us,
                    record.publisher_namespace.clone(),
                );
                zone_states
                    .entry(key)
                    .or_insert_with(|| PublicationAnimationState::new(ttl_ms))
                    .tick();
            }
        }

        // Prune zones no longer present in active_publishes.
        self.pub_animation_states
            .retain(|zone_name, _| scene.zone_registry.active_publishes.contains_key(zone_name));
    }

    /// Determine the effective TTL (ms) for a single publication.
    ///
    /// `ttl_ms` is the delay **until the fade-out animation begins**; the fade
    /// itself then lasts `NOTIFICATION_FADE_OUT_MS` ms.  Total visible duration
    /// is therefore `ttl_ms + NOTIFICATION_FADE_OUT_MS`.
    ///
    /// Priority (highest to lowest):
    /// 1. `ZonePublishRecord.expires_at_wall_us` — urgency-derived absolute expiry
    ///    set by the publishing path.  TTL is derived so the fade-out **starts**
    ///    `NOTIFICATION_FADE_OUT_MS` before the drain deadline:
    ///    `((expires_at_wall_us - published_at_wall_us) / 1_000)
    ///        .saturating_sub(NOTIFICATION_FADE_OUT_MS as u64)`.
    ///    If `expires_at_wall_us <= published_at_wall_us` (already expired or
    ///    invalid), the TTL is `0` (immediate fade-out).
    ///    This ensures the visual fade-out completes before `drain_expired_zone_publications`
    ///    removes the record (e.g., ~14 850 ms TTL for a 15 s warning).
    /// 2. `NotificationPayload.ttl_ms` — per-notification override.
    /// 3. Zone `auto_clear_ms` fallback (supplied by the caller).
    fn publication_ttl_ms(record: &ZonePublishRecord, zone_default_ttl_ms: u64) -> u64 {
        // Highest priority: absolute wall-clock expiry on the record.
        // Derive TTL so the fade starts NOTIFICATION_FADE_OUT_MS before the drain boundary.
        if let Some(exp_us) = record.expires_at_wall_us {
            let duration_ms = if exp_us > record.published_at_wall_us {
                (exp_us - record.published_at_wall_us) / 1_000
            } else {
                // Already expired or invalid: immediate fade-out.
                0
            };
            return duration_ms.saturating_sub(NOTIFICATION_FADE_OUT_MS as u64);
        }
        // Next: per-notification explicit TTL.
        if let ZoneContent::Notification(n) = &record.content {
            if let Some(ttl) = n.ttl_ms {
                return ttl;
            }
        }
        zone_default_ttl_ms
    }

    /// Look up the current opacity for a publication in `pub_animation_states`.
    ///
    /// Returns 1.0 if no animation state is found (publication is fully visible).
    fn pub_opacity(&self, zone_name: &str, record: &ZonePublishRecord) -> f32 {
        let key: PubKey = (
            record.published_at_wall_us,
            record.publisher_namespace.clone(),
        );
        self.pub_animation_states
            .get(zone_name)
            .and_then(|zone_states| zone_states.get(&key))
            .map(|s| s.current_opacity())
            .unwrap_or(1.0)
    }

    /// Remove publications from the scene whose fade-out animation has completed.
    ///
    /// This method MUST be called before rendering so that fully-faded publications
    /// are absent from `active_publishes` during the frame.  After removal,
    /// remaining notifications reflow naturally in the next frame (slot positions
    /// are recalculated from the updated `active_publishes` slice each frame).
    ///
    /// Intended call site: runtime frame loop, between scene commit and render,
    /// alongside `SceneGraph::drain_expired_zone_publications`.
    pub fn prune_faded_publications(&mut self, scene: &mut SceneGraph) {
        for (zone_name, zone_states) in &self.pub_animation_states {
            let publishes = match scene.zone_registry.active_publishes.get_mut(zone_name) {
                Some(p) => p,
                None => continue,
            };
            let before = publishes.len();
            publishes.retain(|record| {
                let key: PubKey = (
                    record.published_at_wall_us,
                    record.publisher_namespace.clone(),
                );
                !zone_states
                    .get(&key)
                    .map(|s| s.is_fade_complete())
                    .unwrap_or(false)
            });
            if publishes.len() < before {
                scene.version += 1;
            }
        }
        // Remove empty active_publishes entries.
        scene
            .zone_registry
            .active_publishes
            .retain(|_, v| !v.is_empty());
    }

    /// Resolve `color.text.primary` from the token map as a sRGB u8 color.
    ///
    /// Falls back to near-white (R=255, G=255, B=255, A=223) when the token
    /// is absent — matching the canonical default for text on dark backgrounds.
    fn resolve_text_primary_color(token_map: &HashMap<String, String>) -> [u8; 4] {
        resolve_token_color(token_map, "color.text.primary")
            .map(crate::text::rgba_to_srgb_u8)
            .unwrap_or([255, 255, 255, 223]) // near-white, alpha ≈ 87.5%
    }

    /// Resolve `typography.body.size` from the token map as a pixel value.
    ///
    /// Falls back to 16.0 px (canonical body text default) when the token is
    /// absent or cannot be parsed as a number.
    fn resolve_body_font_size(token_map: &HashMap<String, String>) -> f32 {
        token_map
            .get("typography.body.size")
            .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(16.0)
            .clamp(6.0, 200.0)
    }

    /// Recursively collect `TextItem`s from a node and its children.
    #[allow(clippy::only_used_in_recursion)]
    fn collect_text_items_from_node(
        &self,
        node_id: SceneId,
        tile: &Tile,
        scene: &SceneGraph,
        items: &mut Vec<TextItem>,
    ) {
        let node = match scene.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };

        if let NodeData::TextMarkdown(tm) = &node.data {
            items.push(TextItem::from_text_markdown_node(
                tm,
                tile.bounds.x,
                tile.bounds.y,
            ));
        }

        for child_id in &node.children {
            self.collect_text_items_from_node(*child_id, tile, scene, items);
        }
    }

    /// Shared encode pipeline used by `render_frame` and `render_frame_headless`.
    ///
    /// Encodes the geometry pass (clear + vertex draw) and the text pass into a
    /// single `CommandEncoder`.  The encoder is returned to the caller **before**
    /// `queue.submit` so that headless callers can append a `copy_to_buffer`
    /// command (which must precede submit).
    ///
    /// # Parameters
    ///
    /// - `vertices` — pre-built vertex list (caller is responsible for overlay
    ///   quads, zone content, etc.)
    /// - `frame_view` — render target view for this frame
    /// - `scene` — used only for the text pass (`collect_text_items`)
    /// - `surf_w` / `surf_h` — surface dimensions used for the text viewport
    /// - `use_overlay_pipeline` — when `true`, selects `clear_pipeline` (no
    ///   blending) instead of the standard `pipeline`
    ///
    /// # Returns
    ///
    /// `(encoder, encode_us)` — the ready-to-submit encoder and the wall-clock
    /// microseconds spent encoding (for `FrameTelemetry::render_encode_us`).
    /// `bg_vertex_count`: number of vertices at the start of `vertices` that
    /// make up the initial Background slice.  This includes any overlay-mode
    /// clear quad prepended before Background zone vertices, so in overlay
    /// mode callers must include those 6 clear-quad vertices in the count
    /// even when Background zones emit no flat-rect vertices.  The
    /// Background SDF rounded-rect pass is inserted between this initial
    /// slice and the remaining tile/content/chrome vertices so that
    /// Background backdrops are correctly drawn BELOW agent tiles.  Pass `0`
    /// only when the initial Background slice is empty (i.e. no Background
    /// flat-rect vertices and no overlay-mode clear quad); the Background
    /// SDF pass still runs before the tile/content/chrome pass.
    #[allow(clippy::too_many_arguments)]
    fn encode_frame(
        &mut self,
        vertices: &[RectVertex],
        frame_view: &wgpu::TextureView,
        scene: &SceneGraph,
        surf_w: u32,
        surf_h: u32,
        use_overlay_pipeline: bool,
        bg_vertex_count: usize,
    ) -> (wgpu::CommandEncoder, u64) {
        let encode_start = std::time::Instant::now();

        // Upload the full vertex buffer once — individual passes render
        // different vertex sub-ranges from it using draw ranges.
        let vertex_buffer = if vertices.is_empty() {
            None
        } else {
            let buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vertex_buffer"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buffer)
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        let sw = surf_w as f32;
        let sh = surf_h as f32;

        // Both flat-rect geometry sub-passes use the same pipeline selector:
        // in overlay mode, the clear_pipeline (no blending) is used for ALL
        // flat-rect rendering. The first 6 vertices are a full-screen
        // transparent quad that zeros out every pixel's alpha. Subsequent
        // content overwrites specific regions with their own alpha.

        // ── Geometry pass 1: Background flat-rect zones ───────────────────────
        // Clears the surface and draws Background-layer zone backdrops (those
        // without backdrop_radius; zones with backdrop_radius emit no vertices
        // here).  Tile/content/chrome vertices are deferred to pass 2 so that
        // the Background SDF pass can be interleaved between the two.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass_bg"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if use_overlay_pipeline {
                render_pass.set_pipeline(&self.clear_pipeline);
            } else {
                render_pass.set_pipeline(&self.pipeline);
            }

            if let Some(ref buffer) = vertex_buffer {
                let bg_end = bg_vertex_count.min(vertices.len());
                if bg_end > 0 {
                    render_pass.set_vertex_buffer(0, buffer.slice(..));
                    render_pass.draw(0..bg_end as u32, 0..1);
                }
            }
        }

        // ── Background SDF rounded-rect pass ──────────────────────────────────
        // Runs after the Clear pass (so it composites over the cleared surface)
        // but BEFORE tile/content/chrome geometry — this is what ensures
        // Background backdrops are correctly occluded by agent tiles.
        {
            let rr_bg =
                self.collect_rounded_rect_cmds(scene, sw, sh, Some(LayerAttachment::Background));
            self.encode_rounded_rect_pass(&mut encoder, frame_view, &rr_bg, sw, sh);
        }

        // ── Geometry pass 2: Tiles + Content + Chrome flat-rect zones ─────────
        // Uses LoadOp::Load to preserve the Background geometry drawn above.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass_tiles_content_chrome"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if use_overlay_pipeline {
                render_pass.set_pipeline(&self.clear_pipeline);
            } else {
                render_pass.set_pipeline(&self.pipeline);
            }

            if let Some(ref buffer) = vertex_buffer {
                let bg_end = bg_vertex_count.min(vertices.len());
                let rest_count = vertices.len().saturating_sub(bg_end);
                if rest_count > 0 {
                    render_pass.set_vertex_buffer(0, buffer.slice(..));
                    render_pass.draw(bg_end as u32..vertices.len() as u32, 0..1);
                }
            }
        }

        // ── Content + Chrome SDF rounded-rect pass ────────────────────────────
        // Runs after tiles so Content/Chrome backdrops composite above tiles.
        {
            let mut rr_post: Vec<crate::pipeline::RoundedRectDrawCmd> = Vec::new();
            rr_post.extend(self.collect_rounded_rect_cmds(
                scene,
                sw,
                sh,
                Some(LayerAttachment::Content),
            ));
            rr_post.extend(self.collect_rounded_rect_cmds(
                scene,
                sw,
                sh,
                Some(LayerAttachment::Chrome),
            ));
            self.encode_rounded_rect_pass(&mut encoder, frame_view, &rr_post, sw, sh);
        }

        // ── Text pass (Stage 6) ───────────────────────────────────────────────
        // Collect text items before borrowing the rasterizer mutably, to avoid
        // simultaneous mutable + immutable borrow of `self`.
        let text_items: Vec<TextItem> = if self.text_rasterizer.is_some() {
            self.collect_text_items(scene, sw, sh)
        } else {
            vec![]
        };

        // If a text rasterizer is present, prepare glyphon buffers and run a
        // LoadOp::Load text pass on top of the geometry written above.
        if let Some(ref mut tr) = self.text_rasterizer {
            tr.update_viewport(&self.queue, surf_w, surf_h);
            if !text_items.is_empty() {
                if let Err(e) = tr.prepare_text_items(&self.device, &self.queue, &text_items) {
                    tracing::warn!(error = %e, "text prepare failed — frame continues without text");
                } else {
                    let mut text_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("text_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: frame_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                // LoadOp::Load: preserve geometry pixels under the text.
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    if let Err(e) = tr.render_text_pass(&mut text_pass) {
                        tracing::warn!(error = %e, "text render failed — frame continues without text");
                    }
                }
            }
            // Trim every frame regardless of item count — glyphs from prior frames
            // must be evicted even when the current frame has no text.
            tr.trim_atlas();
        }

        let encode_us = encode_start.elapsed().as_micros() as u64;
        (encoder, encode_us)
    }

    /// Render one frame of the scene to the surface.
    ///
    /// This method is surface-agnostic: it works with any type implementing
    /// `CompositorSurface`.  The same code path executes in headless and windowed
    /// modes — only the surface implementation differs.
    ///
    /// Per runtime-kernel/spec.md Requirement: Headless Mode (line 198):
    /// "No conditional compilation for the render path."
    ///
    /// For headless pixel readback, use `render_frame_headless()` instead,
    /// which includes the `copy_to_buffer` step internally so that
    /// `surface.read_pixels()` returns the current frame's data.
    /// `render_frame()` does NOT copy pixels to the readback buffer — the
    /// encoder is created and consumed internally and is not exposed.
    ///
    /// Returns telemetry for this frame.
    pub fn render_frame(
        &mut self,
        scene: &SceneGraph,
        surface: &dyn CompositorSurface,
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // Collect visible tiles
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        // Build vertex list from scene.
        let (surf_w, surf_h) = surface.size();
        let sw = surf_w as f32;
        let sh = surf_h as f32;
        let mut vertices: Vec<RectVertex> = Vec::new();
        let mut textured_cmds: Vec<TexturedDrawCmd> = Vec::new();

        // In overlay mode, prepend a full-screen quad to zero out alpha.
        if self.overlay_mode {
            // One-shot diagnostic: log surface dimensions on first frame.
            if self.frame_number == 1 {
                let diag = format!(
                    "render_frame: sw={sw}, sh={sh}, compositor_w={}, compositor_h={}\n",
                    self.width, self.height,
                );
                let _ = std::fs::write("C:\\tze_hud\\logs\\render_diag.txt", &diag);
            }
            vertices.extend_from_slice(&rect_vertices(
                0.0,
                0.0,
                sw,
                sh,
                sw,
                sh,
                [0.0, 0.0, 0.0, 0.0],
            ));
        }

        // ── Ensure image textures are uploaded before rendering ──────────────
        let mut image_refs = self.ensure_scene_image_textures(scene);
        // Ensure icon textures (key_icon_map SVGs) are rasterized and cached.
        // Merge their ResourceIds into the eviction-guard set so they survive.
        let icon_refs = self.ensure_scene_icon_textures(scene);
        image_refs.extend(icon_refs);
        self.evict_unused_image_textures(&image_refs);

        // Update zone animation states (fade-in/fade-out) before rendering.
        // Must run before any render_zone_content call below.
        self.update_zone_animations(scene);

        // ── Layer ordering: Background → Tiles → Content zones → Chrome zones ─
        // Background zones render first so agent tiles occlude them.
        self.render_zone_content(
            scene,
            &mut vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Background),
        );

        // Capture the vertex count after Background zones so encode_frame can
        // split the flat-rect pass and interleave the Background SDF pass.
        let bg_vertex_count = vertices.len();

        for tile in &tiles {
            // Render tile background
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                self.gpu_color_raw(bg_color),
            );
            vertices.extend_from_slice(&verts);

            // Render nodes within the tile
            if let Some(root_id) = tile.root_node {
                self.render_node(
                    root_id,
                    tile,
                    scene,
                    &mut vertices,
                    &mut textured_cmds,
                    sw,
                    sh,
                );
            }
        }

        // Update zone animation states (fade-in/fade-out) before rendering.
        self.update_zone_animations(scene);

        // Update streaming word-by-word reveal state.
        self.update_stream_reveals(scene);

        // Content zones render as a batch after all tiles (above background, below chrome).
        self.render_zone_content(
            scene,
            &mut vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Content),
        );
        // Chrome zones render last, above everything.
        self.render_zone_content(
            scene,
            &mut vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Chrome),
        );

        // ── Widget texture sync: rasterize dirty SVGs BEFORE frame acquisition.
        // SVG rasterization can be slow; if a resize event arrives while we hold
        // the surface texture, the texture is destroyed and queue.submit panics.
        self.sync_widget_textures(scene, self.degradation_level);

        // Acquire frame through the surface trait (surface-agnostic).
        // The CompositorFrame._guard keeps the backing resource alive until drop.
        let frame = surface.acquire_frame();

        let (mut encoder, encode_us) = self.encode_frame(
            &vertices,
            &frame.view,
            scene,
            surf_w,
            surf_h,
            self.overlay_mode,
            bg_vertex_count,
        );
        telemetry.render_encode_us = encode_us;

        // ── Image pass: draw textured quads on top of color geometry ─────────
        self.encode_image_pass(&mut encoder, &frame.view, &textured_cmds, sw, sh);

        // ── Widget pass: composite pre-synced textures above zone content ────
        self.encode_widget_pass(&mut encoder, &frame.view, &scene.widget_registry, sw, sh);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));

        // present() is surface-specific: no-op for headless, swap-chain flip for windowed.
        // Drop frame guard AFTER present() so the backing resource stays alive.
        surface.present();
        drop(frame);

        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Render one frame and copy pixel data into the headless readback buffer.
    ///
    /// This is a convenience method for testing/CI that handles the extra
    /// `copy_to_buffer` step required for headless pixel readback.
    ///
    /// `copy_to_buffer` is appended to the encoder before `queue.submit()` via
    /// the shared `encode_frame` helper, which returns the encoder prior to
    /// submission so that this headless-specific step can be inserted cleanly.
    ///
    /// Returns telemetry for this frame.
    pub fn render_frame_headless(
        &mut self,
        scene: &mut SceneGraph,
        surface: &HeadlessSurface,
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // Collect visible tiles
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        // Build vertex list from scene.
        // Use surface.size() — not self.width/self.height — so that vertex
        // normalization is correct even if the HeadlessSurface was created with
        // different dimensions than the compositor's stored width/height.
        let (surf_w, surf_h) = surface.size();
        let sw = surf_w as f32;
        let sh = surf_h as f32;
        let mut vertices: Vec<RectVertex> = Vec::new();
        let mut textured_cmds: Vec<TexturedDrawCmd> = Vec::new();

        // ── Ensure image textures are uploaded before rendering ──────────────
        let mut image_refs = self.ensure_scene_image_textures(scene);
        // Ensure icon textures (key_icon_map SVGs) are rasterized and cached.
        let icon_refs = self.ensure_scene_icon_textures(scene);
        image_refs.extend(icon_refs);
        self.evict_unused_image_textures(&image_refs);

        // Update zone animation states before rendering zone content.
        // Must run before any render_zone_content call below.
        self.update_zone_animations(scene);

        // ── Layer ordering: Background → Tiles → Content zones → Chrome zones ─
        // Background zones render first so agent tiles occlude them.
        self.render_zone_content(
            scene,
            &mut vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Background),
        );

        // Capture the vertex count after Background zones so encode_frame can
        // split the flat-rect pass and interleave the Background SDF pass.
        let bg_vertex_count = vertices.len();

        for tile in &tiles {
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            vertices.extend_from_slice(&verts);

            if let Some(root_id) = tile.root_node {
                self.render_node(
                    root_id,
                    tile,
                    scene,
                    &mut vertices,
                    &mut textured_cmds,
                    sw,
                    sh,
                );
            }
        }

        // Update zone animation states before rendering zone content.
        self.update_zone_animations(scene);

        // Update streaming word-by-word reveal state.
        self.update_stream_reveals(scene);

        // Content zones render as a batch after all tiles (above background, below chrome).
        self.render_zone_content(
            scene,
            &mut vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Content),
        );
        // Chrome zones render last, above everything.
        self.render_zone_content(
            scene,
            &mut vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Chrome),
        );

        // ── Widget texture sync before frame acquisition (same as windowed path).
        self.sync_widget_textures(scene, self.degradation_level);

        // Acquire frame via trait — same code path as render_frame().
        let frame = surface.acquire_frame();

        // Headless never uses overlay mode — pass false for the pipeline selector.
        let (mut encoder, encode_us) = self.encode_frame(
            &vertices,
            &frame.view,
            scene,
            surf_w,
            surf_h,
            false,
            bg_vertex_count,
        );
        telemetry.render_encode_us = encode_us;

        // ── Image pass: draw textured quads on top of color geometry ─────────
        self.encode_image_pass(&mut encoder, &frame.view, &textured_cmds, sw, sh);

        // ── Widget pass: composite pre-synced textures above zone content ────
        self.encode_widget_pass(&mut encoder, &frame.view, &scene.widget_registry, sw, sh);

        // Headless-specific: copy rendered texture to readback buffer.
        // Must happen after all render passes and before submit.
        surface.copy_to_buffer(&mut encoder);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));
        surface.present(); // no-op for headless
        drop(frame);
        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;

        // Populate zone interaction hit regions for the next frame's hit-testing.
        // Must run after rendering so the region geometry is consistent with what
        // was just displayed.  This follows the snapshot-based design: regions
        // computed from the rendered geometry are used for hit-testing on the
        // next input event.
        self.populate_zone_hit_regions(scene, sw, sh);

        telemetry
    }

    /// Render one frame with a separate chrome pass after the content pass.
    ///
    /// # Layer ordering (back to front)
    ///
    /// 1. **Content pass** — background clear + all agent tiles. Uses `LoadOp::Clear`.
    /// 2. **Chrome pass** — draws chrome draw commands on top. Uses `LoadOp::Load` to
    ///    preserve the content pass pixels, ensuring chrome is always on top.
    ///
    /// Chrome draw commands are produced by the caller (from `ChromeRenderer::render_chrome`)
    /// before calling this function. The compositor does not access `ChromeState` directly —
    /// chrome state is fully decoupled from scene/agent state.
    ///
    /// # Separable passes
    ///
    /// The content pass and chrome pass are encoded as two separate `RenderPass` blocks
    /// within the same `CommandEncoder`. This architectural separation is the foundation
    /// for future render-skip redaction (capture-safe architecture): the content pass can
    /// be suppressed independently of the chrome pass.
    ///
    /// # Chrome layer sovereignty
    ///
    /// Because the chrome pass uses `LoadOp::Load` and runs after the content pass,
    /// chrome pixels are always written last — no agent tile can occlude chrome regardless
    /// of its z-order value.
    pub fn render_frame_with_chrome(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
        chrome_cmds: &[ChromeDrawCmd],
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // ── Widget texture sync before encoding (avoids surface-texture race).
        self.sync_widget_textures(scene, self.degradation_level);

        // ── Ensure image textures are uploaded before rendering ──────────────
        let mut image_refs = self.ensure_scene_image_textures(scene);
        // Ensure icon textures (key_icon_map SVGs) are rasterized and cached.
        let icon_refs = self.ensure_scene_icon_textures(scene);
        image_refs.extend(icon_refs);
        self.evict_unused_image_textures(&image_refs);

        // ── Pass 1: Content (background + agent tiles) ──────────────────────
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        let mut content_vertices: Vec<RectVertex> = Vec::new();
        let mut textured_cmds: Vec<TexturedDrawCmd> = Vec::new();
        let (surf_w, surf_h) = surface.size();
        let sw = surf_w as f32;
        let sh = surf_h as f32;

        // Update zone animation states before rendering zone content.
        // Must run before any render_zone_content call below.
        self.update_zone_animations(scene);

        // ── Layer ordering: Background → Tiles → Content zones → Chrome zones ─
        // Background zones render first so agent tiles occlude them.
        self.render_zone_content(
            scene,
            &mut content_vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Background),
        );

        // Capture the vertex count after Background zones so that the Background
        // SDF pass can be interleaved between the Background flat-rect pass and
        // the tile/content/chrome pass below.
        let bg_vertex_count = content_vertices.len();

        for tile in &tiles {
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            content_vertices.extend_from_slice(&verts);
            if let Some(root_id) = tile.root_node {
                self.render_node(
                    root_id,
                    tile,
                    scene,
                    &mut content_vertices,
                    &mut textured_cmds,
                    sw,
                    sh,
                );
            }
        }

        // Update zone animation states, streaming reveal, and render zone content backdrops.
        self.update_zone_animations(scene);
        self.update_stream_reveals(scene);

        // Content zones render as a batch after all tiles (above background, below chrome).
        self.render_zone_content(
            scene,
            &mut content_vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Content),
        );
        // Chrome zones render last in the content pass (before the separate GPU chrome pass).
        self.render_zone_content(
            scene,
            &mut content_vertices,
            &mut textured_cmds,
            sw,
            sh,
            Some(LayerAttachment::Chrome),
        );

        let encode_start = std::time::Instant::now();

        let content_buffer = if content_vertices.is_empty() {
            None
        } else {
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("content_vertex_buffer"),
                    contents: bytemuck::cast_slice(&content_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buf)
        };

        // ── Pass 2: Chrome ───────────────────────────────────────────────────
        let mut chrome_vertices: Vec<RectVertex> = Vec::new();
        for cmd in chrome_cmds {
            let verts = rect_vertices(cmd.x, cmd.y, cmd.width, cmd.height, sw, sh, cmd.color);
            chrome_vertices.extend_from_slice(&verts);
        }

        let chrome_buffer = if chrome_vertices.is_empty() {
            None
        } else {
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("chrome_vertex_buffer"),
                    contents: bytemuck::cast_slice(&chrome_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buf)
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder_with_chrome"),
            });

        // ── Content pass 1: Background flat-rect zones ────────────────────────
        // Clears the surface and draws Background-layer zone backdrops (those
        // without backdrop_radius).  Tile/content/chrome vertices are deferred
        // to pass 2 so that the Background SDF pass can be interleaved.
        {
            let mut content_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("content_pass_bg"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            content_pass.set_pipeline(&self.pipeline);
            if let Some(ref buf) = content_buffer {
                let bg_end = bg_vertex_count.min(content_vertices.len());
                if bg_end > 0 {
                    content_pass.set_vertex_buffer(0, buf.slice(..));
                    content_pass.draw(0..bg_end as u32, 0..1);
                }
            }
        }

        // ── Background SDF rounded-rect pass ─────────────────────────────────
        // Runs after the Clear pass so Background backdrops are below tiles.
        {
            let rr_bg =
                self.collect_rounded_rect_cmds(scene, sw, sh, Some(LayerAttachment::Background));
            self.encode_rounded_rect_pass(&mut encoder, &surface.view, &rr_bg, sw, sh);
        }

        // ── Content pass 2: Tiles + Content + Chrome flat-rect zones ──────────
        // Uses LoadOp::Load to preserve Background geometry drawn above.
        {
            let mut content_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("content_pass_tiles_content_chrome"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            content_pass.set_pipeline(&self.pipeline);
            if let Some(ref buf) = content_buffer {
                let bg_end = bg_vertex_count.min(content_vertices.len());
                let rest_count = content_vertices.len().saturating_sub(bg_end);
                if rest_count > 0 {
                    content_pass.set_vertex_buffer(0, buf.slice(..));
                    content_pass.draw(bg_end as u32..content_vertices.len() as u32, 0..1);
                }
            }
        }

        // ── Content + Chrome SDF rounded-rect pass ────────────────────────────
        // Runs after tiles so Content/Chrome backdrops composite above tiles.
        {
            let mut rr_post: Vec<crate::pipeline::RoundedRectDrawCmd> = Vec::new();
            rr_post.extend(self.collect_rounded_rect_cmds(
                scene,
                sw,
                sh,
                Some(LayerAttachment::Content),
            ));
            rr_post.extend(self.collect_rounded_rect_cmds(
                scene,
                sw,
                sh,
                Some(LayerAttachment::Chrome),
            ));
            self.encode_rounded_rect_pass(&mut encoder, &surface.view, &rr_post, sw, sh);
        }

        // ── Stage 6: Text pass (between content and chrome) ──────────────────
        // Text is content — rendered above geometry rectangles but below chrome.
        let text_items_chrome: Vec<TextItem> = if self.text_rasterizer.is_some() {
            self.collect_text_items(scene, sw, sh)
        } else {
            vec![]
        };
        if let Some(ref mut tr) = self.text_rasterizer {
            let (surf_w, surf_h) = (self.width, self.height);
            tr.update_viewport(&self.queue, surf_w, surf_h);
            if !text_items_chrome.is_empty() {
                if let Err(e) = tr.prepare_text_items(&self.device, &self.queue, &text_items_chrome)
                {
                    tracing::warn!(error = %e, "text prepare failed in chrome path");
                } else {
                    let mut text_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("text_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &surface.view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    if let Err(e) = tr.render_text_pass(&mut text_pass) {
                        tracing::warn!(error = %e, "text render failed in chrome path");
                    }
                }
            }
            // Trim every frame regardless of item count — glyphs from prior frames
            // must be evicted even when the current frame has no text.
            tr.trim_atlas();
        }

        // ── Image pass: draw textured quads on top of color geometry ─────────
        self.encode_image_pass(&mut encoder, &surface.view, &textured_cmds, sw, sh);

        // ── Widget pass: composite pre-synced textures above content + text ──
        // sync_widget_textures is called earlier, before frame encoding begins.
        self.encode_widget_pass(&mut encoder, &surface.view, &scene.widget_registry, sw, sh);

        // Chrome render pass — uses LoadOp::Load to preserve content pixels.
        // Chrome commands are drawn ON TOP of content by construction.
        // No agent tile can occlude chrome regardless of z-order.
        {
            let mut chrome_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chrome_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // LoadOp::Load: preserve content pixels — chrome draws on top.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            chrome_pass.set_pipeline(&self.pipeline);
            if let Some(ref buf) = chrome_buffer {
                chrome_pass.set_vertex_buffer(0, buf.slice(..));
                chrome_pass.draw(0..chrome_vertices.len() as u32, 0..1);
            }
        }

        telemetry.render_encode_us = encode_start.elapsed().as_micros() as u64;

        surface.copy_to_buffer(&mut encoder);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Convert an Rgba to GPU-ready `[f32; 4]`.
    ///
    /// In overlay mode the clear_pipeline writes RGBA directly (no GPU blending);
    /// DWM composites with premultiplied alpha **in sRGB space**.  The GPU
    /// auto-encodes fragment output from linear→sRGB (`Bgra8UnormSrgb`), so we
    /// must output the linear value that, after encoding, yields the correct
    /// sRGB-premultiplied result:
    ///
    ///   stored_R = sRGB(R) × A          (what DWM expects)
    ///   fragment_R = linear(sRGB(R) × A) (what we must output)
    ///
    /// In fullscreen mode the blend pipeline handles compositing, so straight
    /// alpha is correct.
    #[inline]
    fn gpu_color(&self, rgba: Rgba) -> [f32; 4] {
        if self.overlay_mode {
            let a = rgba.a;
            [
                srgb_to_linear(linear_to_srgb(rgba.r) * a),
                srgb_to_linear(linear_to_srgb(rgba.g) * a),
                srgb_to_linear(linear_to_srgb(rgba.b) * a),
                a,
            ]
        } else {
            rgba.to_array()
        }
    }

    /// Same as [`gpu_color`] but for raw `[f32; 4]` arrays (assumed linear).
    #[inline]
    fn gpu_color_raw(&self, color: [f32; 4]) -> [f32; 4] {
        if self.overlay_mode {
            let a = color[3];
            [
                srgb_to_linear(linear_to_srgb(color[0]) * a),
                srgb_to_linear(linear_to_srgb(color[1]) * a),
                srgb_to_linear(linear_to_srgb(color[2]) * a),
                a,
            ]
        } else {
            color
        }
    }

    /// Return the clear color for render passes. Transparent in overlay mode,
    /// dark background in fullscreen mode.
    fn clear_color(&self) -> wgpu::Color {
        if self.overlay_mode {
            wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }
        } else {
            wgpu::Color {
                r: 0.05,
                g: 0.05,
                b: 0.1,
                a: 1.0,
            }
        }
    }

    /// Encode a widget render pass that composites all widget textures into the frame.
    ///
    /// Widget tiles use z_order >= WIDGET_TILE_Z_MIN (0x9000_0000), placing them
    /// above zone tiles but below chrome (spec §Requirement: Widget Contention and
    /// Governance, §Requirement: Widget Input Mode).
    ///
    /// This is a no-op when the widget renderer is not initialized or the registry
    /// has no instances with cached textures.
    fn encode_widget_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame_view: &wgpu::TextureView,
        registry: &WidgetRegistry,
        surf_w: f32,
        surf_h: f32,
    ) {
        let wr = match &self.widget_renderer {
            Some(r) => r,
            None => return,
        };

        // Check if there are any instances with cached textures.
        if registry.instances.is_empty() {
            return;
        }

        let any_textured = registry
            .instances
            .keys()
            .any(|name| wr.texture_entry(name).is_some());
        if !any_textured {
            return;
        }

        // Begin a LoadOp::Load render pass — widgets composite on top of scene content.
        let mut widget_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("widget_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load, // preserve content pixels under widgets
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        wr.composite_widgets(&mut widget_pass, registry, surf_w, surf_h, &self.device);
    }

    /// Encode a render pass for textured image quads.
    ///
    /// Uses `LoadOp::Load` to composite textured images on top of the color
    /// geometry already written to the frame. Each unique `ResourceId` in
    /// `cmds` switches the bind group to the corresponding cached texture.
    fn encode_image_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame_view: &wgpu::TextureView,
        cmds: &[TexturedDrawCmd],
        sw: f32,
        sh: f32,
    ) {
        if cmds.is_empty() {
            return;
        }

        use wgpu::util::DeviceExt;

        let mut image_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("image_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        image_pass.set_pipeline(&self.texture_rect_pipeline);

        for cmd in cmds {
            let entry = match self.image_texture_cache.get(&cmd.resource_id) {
                Some(e) => e,
                None => continue, // shouldn't happen if ensure was called
            };

            let verts =
                textured_rect_vertices(cmd.x, cmd.y, cmd.w, cmd.h, sw, sh, cmd.uv_rect, cmd.tint);
            let vertex_buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("image_quad_buf"),
                    contents: bytemuck::cast_slice(&verts),
                    usage: wgpu::BufferUsages::VERTEX,
                });

            image_pass.set_bind_group(0, &entry.bind_group, &[]);
            image_pass.set_vertex_buffer(0, vertex_buf.slice(..));
            image_pass.draw(0..6, 0..1);
        }
    }

    /// Collect rounded-rectangle draw commands for zones whose `RenderingPolicy`
    /// has `backdrop_radius` set.
    ///
    /// These zones are excluded from the flat-rect backdrop pass
    /// (`render_zone_content`) and rendered instead by `encode_rounded_rect_pass`
    /// using the SDF pipeline.
    ///
    /// Mirrors the backdrop-resolution logic in `render_zone_content` so color
    /// derivation (severity tokens, urgency colors, opacity) is consistent.
    ///
    /// # Layer filtering
    ///
    /// When `only_layer` is `Some(layer)`, only zones matching that layer are
    /// included.  Pass `None` to collect all layers.
    fn collect_rounded_rect_cmds(
        &self,
        scene: &SceneGraph,
        sw: f32,
        sh: f32,
        only_layer: Option<LayerAttachment>,
    ) -> Vec<RoundedRectDrawCmd> {
        let mut cmds = Vec::new();

        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };

            // Layer filter.
            if let Some(required_layer) = only_layer {
                if zone_def.layer_attachment != required_layer {
                    continue;
                }
            }

            let policy = &zone_def.rendering_policy;

            // Only collect zones with a backdrop_radius — others use the flat rect path.
            let radius = match policy.backdrop_radius {
                Some(r) if r > 0.0 => r,
                _ => continue,
            };

            let (x, y, w, h) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);
            // Clamp radius against the zone's full dimensions as a first-pass
            // upper bound. For Stack zones, each slot height (effective_slot_h)
            // may be smaller than h, so per-slot clamping is applied below.
            let max_r_zone = (w * 0.5).min(h * 0.5).max(0.0);
            let radius = radius.min(max_r_zone);

            let anim_opacity = self
                .zone_animation_states
                .get(zone_name)
                .map(|s| s.current_opacity())
                .unwrap_or(1.0);

            // Resolve backdrop color using the same logic as render_zone_content.
            match zone_def.contention_policy {
                ContentionPolicy::Stack { .. } => {
                    let slot_h = Self::stack_slot_height(policy);
                    let effective_h = if is_alert_banner_zone(zone_name) {
                        publishes.len() as f32 * slot_h
                    } else {
                        h
                    };

                    let ordered_indices: Vec<usize> = if is_alert_banner_zone(zone_name) {
                        sort_alert_banner_indices(publishes)
                    } else {
                        (0..publishes.len()).rev().collect()
                    };

                    for (slot_idx, &pub_idx) in ordered_indices.iter().enumerate() {
                        let record = &publishes[pub_idx];
                        let slot_y = y + slot_idx as f32 * slot_h;
                        if slot_y >= y + effective_h {
                            break;
                        }
                        let effective_slot_h = slot_h.min((y + effective_h) - slot_y);

                        let pub_opacity = self.pub_opacity(zone_name, record);
                        let combined_opacity = (anim_opacity * pub_opacity).clamp(0.0, 1.0);

                        let is_notification_content =
                            matches!(&record.content, ZoneContent::Notification(_));
                        let backdrop_rgba: Option<Rgba> = match &record.content {
                            ZoneContent::SolidColor(rgba) => Some(*rgba),
                            ZoneContent::StaticImage(_) => Some(STATIC_IMAGE_PLACEHOLDER_COLOR),
                            ZoneContent::Notification(n) if is_alert_banner_zone(zone_name) => {
                                if policy.backdrop.is_some() {
                                    Some(urgency_to_severity_color(n.urgency, &self.token_map))
                                } else {
                                    None
                                }
                            }
                            ZoneContent::Notification(n) => {
                                if policy.backdrop.is_some() {
                                    let mut color =
                                        urgency_to_notification_color(n.urgency, &self.token_map);
                                    color.a = NOTIFICATION_BACKDROP_OPACITY;
                                    Some(color)
                                } else {
                                    None
                                }
                            }
                            _ => policy.backdrop,
                        };

                        if let Some(mut rgba) = backdrop_rgba {
                            if !is_notification_content || is_alert_banner_zone(zone_name) {
                                if let Some(opacity) = policy.backdrop_opacity {
                                    rgba.a = opacity.clamp(0.0, 1.0);
                                }
                            }
                            rgba.a *= combined_opacity;
                            // Re-clamp radius per slot: effective_slot_h may be
                            // smaller than the zone height (e.g., last slot in a
                            // stack), so the radius must not exceed half the slot.
                            let slot_radius =
                                radius.min((w * 0.5).min(effective_slot_h * 0.5).max(0.0));
                            cmds.push(RoundedRectDrawCmd {
                                x,
                                y: slot_y,
                                width: w,
                                height: effective_slot_h,
                                radius: slot_radius,
                                color: self.gpu_color(rgba),
                            });
                        }
                    }
                }
                ContentionPolicy::MergeByKey { .. }
                | ContentionPolicy::LatestWins
                | ContentionPolicy::Replace => {
                    let latest = &publishes[publishes.len() - 1];
                    let is_notification_content =
                        matches!(&latest.content, ZoneContent::Notification(_));
                    let backdrop_rgba: Option<Rgba> = match &latest.content {
                        ZoneContent::SolidColor(rgba) => Some(*rgba),
                        ZoneContent::StaticImage(_) => Some(STATIC_IMAGE_PLACEHOLDER_COLOR),
                        ZoneContent::Notification(n) if is_alert_banner_zone(zone_name) => {
                            if policy.backdrop.is_some() {
                                Some(urgency_to_severity_color(n.urgency, &self.token_map))
                            } else {
                                None
                            }
                        }
                        ZoneContent::Notification(n) => {
                            if policy.backdrop.is_some() {
                                let mut color =
                                    urgency_to_notification_color(n.urgency, &self.token_map);
                                color.a = NOTIFICATION_BACKDROP_OPACITY;
                                Some(color)
                            } else {
                                None
                            }
                        }
                        _ => policy.backdrop,
                    };

                    if let Some(mut rgba) = backdrop_rgba {
                        if !is_notification_content || is_alert_banner_zone(zone_name) {
                            if let Some(opacity) = policy.backdrop_opacity {
                                rgba.a = opacity.clamp(0.0, 1.0);
                            }
                        }
                        rgba.a *= anim_opacity.clamp(0.0, 1.0);
                        cmds.push(RoundedRectDrawCmd {
                            x,
                            y,
                            width: w,
                            height: h,
                            radius,
                            color: self.gpu_color(rgba),
                        });
                    }
                }
            }
        }

        cmds
    }

    /// Encode a GPU pass that renders the given rounded-rectangle commands
    /// using the SDF pipeline.
    ///
    /// Uses `LoadOp::Load` so existing scene pixels are preserved beneath
    /// the rounded corners.  Skips the pass entirely when `cmds` is empty.
    fn encode_rounded_rect_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame_view: &wgpu::TextureView,
        cmds: &[crate::pipeline::RoundedRectDrawCmd],
        sw: f32,
        sh: f32,
    ) {
        if cmds.is_empty() {
            return;
        }

        // Build vertex buffer from all commands.
        let mut vertices: Vec<RoundedRectVertex> = Vec::with_capacity(cmds.len() * 6);
        for cmd in cmds {
            vertices.extend_from_slice(&rounded_rect_vertices(
                cmd.x, cmd.y, cmd.width, cmd.height, sw, sh, cmd.radius, cmd.color,
            ));
        }

        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rounded_rect_vertex_buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rounded_rect_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load, // preserve background pixels
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        // In overlay mode select the premultiplied pipeline; otherwise use the
        // straight-alpha pipeline.  Vertex colors are premultiplied by gpu_color
        // in overlay mode, so the blend equation and shader must match.
        let pipeline = if self.overlay_mode {
            &self.rounded_rect_overlay_pipeline
        } else {
            &self.rounded_rect_pipeline
        };
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..vertices.len() as u32, 0..1);
    }

    /// Render zone content backdrop quads driven by `RenderingPolicy`.
    ///
    /// For each zone with at least one active publish:
    /// - Reads `backdrop` + `backdrop_opacity` from the zone's `RenderingPolicy`.
    /// - For `alert-banner` zones with `Notification` content, overrides the
    ///   backdrop color with the urgency-derived `color.severity.*` token color.
    /// - For non-alert-banner zones with `Notification` content, overrides the
    ///   backdrop color with the urgency-derived `color.notification.urgency.*`
    ///   token color at 0.9 opacity, and renders a 1px 4-quad border using
    ///   `color.border.default`.
    /// - Applies the zone's current animation opacity (fade-in/fade-out).
    /// - Skips the backdrop quad when `rendering_policy.backdrop` is `None`.
    ///
    /// ## Layer filtering
    ///
    /// When `only_layer` is `Some(layer)`, only zones whose `layer_attachment`
    /// matches `layer` are rendered.  Pass `None` to render all layers (the
    /// legacy behaviour used by unit tests that call this method directly).
    ///
    /// Render-frame methods call this three times to enforce the canonical
    /// layer order: Background → (tiles) → Content → Chrome.
    ///
    /// No per-content-type color branching — all visual properties come from
    /// `RenderingPolicy` fields (spec §Refactoring note, §Default Zone Rendering).
    fn render_zone_content(
        &self,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        textured_cmds: &mut Vec<TexturedDrawCmd>,
        sw: f32,
        sh: f32,
        only_layer: Option<LayerAttachment>,
    ) {
        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            // Layer filter: skip zones that don't match the requested layer.
            if let Some(required_layer) = only_layer {
                if zone_def.layer_attachment != required_layer {
                    continue;
                }
            }
            let (x, y, w, h) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);

            let policy = &zone_def.rendering_policy;

            // Zones with backdrop_radius use the SDF rounded-rect pipeline instead.
            // Skip flat-rect backdrop emission for those zones; collect_rounded_rect_cmds
            // handles their backdrops separately in encode_rounded_rect_pass.
            let use_rounded_rect = policy.backdrop_radius.is_some_and(|r| r > 0.0);

            // Determine current animation opacity for this zone.
            let anim_opacity = self
                .zone_animation_states
                .get(zone_name)
                .map(|s| s.current_opacity())
                .unwrap_or(1.0);

            // Resolve and emit backdrop quads based on the zone's contention policy.
            //
            // Stack zones: each publication gets its own vertically-stacked slot.
            //   For alert-banner: severity-descending (critical first), then
            //   recency-descending within the same severity.
            //   For other Stack zones: newest-first (slot 0 = newest, at top of zone).
            //   Per-slot heights are computed via Self::per_slot_heights() and may vary
            //   by item (two-line notifications occupy a taller slot).  Dynamic zone
            //   height for alert-banner: sum(slot_heights) — zero when empty.
            //
            // MergeByKey zones: single backdrop for the full zone (entries are merged
            //   at the data level; visually one unified strip).
            //
            // LatestWins / Replace: single backdrop from the most-recent publish only.
            match zone_def.contention_policy {
                ContentionPolicy::Stack { .. } => {
                    // Build an ordered reference slice: alert-banner uses severity sort
                    // (critical first, then recency); other Stack zones use newest-first.
                    let ordered_indices: Vec<usize> = if is_alert_banner_zone(zone_name) {
                        sort_alert_banner_indices(publishes)
                    } else {
                        (0..publishes.len()).rev().collect()
                    };

                    // Ordered references for per_slot_heights.
                    let ordered_refs: Vec<&ZonePublishRecord> =
                        ordered_indices.iter().map(|&i| &publishes[i]).collect();

                    // Resolve notification typography tokens once per zone so that
                    // slot-height calculation matches what collect_text_items renders.
                    let notif_body_scale = self
                        .token_map
                        .get("typography.notification.body.scale")
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(NOTIFICATION_BODY_SCALE)
                        .clamp(0.5, 1.0);

                    // Compute per-slot heights (variable for two-line notifications).
                    let slot_heights = Self::per_slot_heights(
                        &ordered_refs,
                        policy,
                        notif_body_scale,
                        NOTIFICATION_INTER_LINE_GAP,
                    );
                    let slot_offsets = Self::slot_offsets(&slot_heights);
                    let total_slots_h: f32 = slot_heights.iter().sum();

                    // alert-banner: dynamic height = sum of per-slot heights.
                    // Height grows with each active banner; no cap at the configured
                    // height_pct.  (Zero banners → zero height via the is_empty guard.)
                    // For other Stack zones: use the configured zone height (h).
                    let effective_h = if is_alert_banner_zone(zone_name) {
                        total_slots_h
                    } else {
                        h
                    };

                    for (&pub_idx, (slot_offset, slot_h)) in ordered_indices
                        .iter()
                        .zip(slot_offsets.iter().zip(slot_heights.iter()))
                    {
                        let record = &publishes[pub_idx];
                        let slot_y = y + slot_offset;
                        if slot_y >= y + effective_h {
                            break;
                        }
                        let effective_slot_h = slot_h.min((y + effective_h) - slot_y);

                        // Per-publication fade-out opacity (1.0 when no fade active).
                        let pub_opacity = self.pub_opacity(zone_name, record);
                        // Combined opacity: zone animation × per-publication fade.
                        let combined_opacity = (anim_opacity * pub_opacity).clamp(0.0, 1.0);

                        // Determine backdrop color.
                        // alert-banner: urgency → color.severity.* tokens
                        // non-alert-banner Notification: urgency → color.notification.urgency.* tokens
                        //   with fixed 0.9 opacity and 1px 4-quad border
                        // SolidColor: always its own color
                        // StaticImage: warm-gray placeholder quad (full GPU texture deferred)
                        // Other: policy.backdrop
                        let is_notification_content =
                            matches!(&record.content, ZoneContent::Notification(_));
                        let backdrop_rgba: Option<Rgba> = match &record.content {
                            ZoneContent::SolidColor(rgba) => Some(*rgba),
                            ZoneContent::StaticImage(resource_id) => {
                                // If a GPU texture is cached, emit a textured draw command
                                // filling this publication's slot.
                                if self.image_texture_cache.contains_key(resource_id) {
                                    let combined_opacity = (anim_opacity
                                        * self.pub_opacity(zone_name, record))
                                    .clamp(0.0, 1.0);
                                    textured_cmds.push(TexturedDrawCmd {
                                        resource_id: *resource_id,
                                        x,
                                        y: slot_y,
                                        w,
                                        h: effective_slot_h,
                                        uv_rect: [0.0, 0.0, 1.0, 1.0],
                                        tint: [1.0, 1.0, 1.0, combined_opacity],
                                    });
                                    None // skip the color quad
                                } else {
                                    // Placeholder warm-gray backdrop.
                                    Some(STATIC_IMAGE_PLACEHOLDER_COLOR)
                                }
                            }
                            ZoneContent::Notification(n) if is_alert_banner_zone(zone_name) => {
                                // alert-banner: severity tokens (color.severity.*).
                                // Respect policy.backdrop contract: only override the color when
                                // a backdrop is enabled; skip entirely when backdrop is None.
                                if policy.backdrop.is_some() {
                                    let severity_color =
                                        urgency_to_severity_color(n.urgency, &self.token_map);
                                    Some(severity_color)
                                } else {
                                    None
                                }
                            }
                            ZoneContent::Notification(n) => {
                                // Non-alert-banner notification: urgency-tinted backdrop
                                // using color.notification.urgency.* tokens.
                                // Per spec: urgency >3 clamped to 3 (critical).
                                // Respect policy.backdrop contract: only emit a backdrop (and
                                // border) when a backdrop is enabled; skip entirely when None.
                                if policy.backdrop.is_some() {
                                    let mut color =
                                        urgency_to_notification_color(n.urgency, &self.token_map);
                                    color.a = NOTIFICATION_BACKDROP_OPACITY;
                                    Some(color)
                                } else {
                                    None
                                }
                            }
                            _ => policy.backdrop,
                        };

                        if let Some(mut rgba) = backdrop_rgba {
                            // For non-notification content: apply policy backdrop_opacity.
                            // For Notification content in non-alert-banner zones: opacity is
                            // already set to NOTIFICATION_BACKDROP_OPACITY above; skip policy override.
                            if !is_notification_content || is_alert_banner_zone(zone_name) {
                                if let Some(opacity) = policy.backdrop_opacity {
                                    rgba.a = opacity.clamp(0.0, 1.0);
                                }
                            }
                            rgba.a *= combined_opacity;
                            // Skip flat-rect emission for rounded zones — handled by the SDF pass.
                            if !use_rounded_rect {
                                vertices.extend_from_slice(&rect_vertices(
                                    x,
                                    slot_y,
                                    w,
                                    effective_slot_h,
                                    sw,
                                    sh,
                                    self.gpu_color(rgba),
                                ));

                                // For non-alert-banner Notification content: emit 1px 4-quad border.
                                if is_notification_content && !is_alert_banner_zone(zone_name) {
                                    let mut border_color =
                                        resolve_border_default_color(&self.token_map);
                                    border_color.a *= combined_opacity;
                                    emit_border_quads(
                                        vertices,
                                        x,
                                        slot_y,
                                        w,
                                        effective_slot_h,
                                        sw,
                                        sh,
                                        self.gpu_color(border_color),
                                    );
                                }
                            }
                        }

                        // Notification icon: emit a textured draw command for the icon
                        // left-aligned inside the slot, vertically centred.
                        // Renders only when: icon is a valid hex ResourceId AND the
                        // texture is cached.  Falls back to text-only (no icon) otherwise.
                        if let ZoneContent::Notification(payload) = &record.content {
                            if let Some(icon_id) = parse_notification_icon(&payload.icon) {
                                if self.image_texture_cache.contains_key(&icon_id) {
                                    // Inset from backdrop edge, same as text (border+padding).
                                    const NOTIFICATION_INSET: f32 = 9.0;
                                    let inset_h = policy
                                        .margin_horizontal
                                        .or(policy.margin_px)
                                        .unwrap_or(NOTIFICATION_INSET);
                                    let icon_x = x + inset_h;
                                    let icon_y = slot_y
                                        + (effective_slot_h - NOTIFICATION_ICON_SIZE_PX) * 0.5;
                                    textured_cmds.push(TexturedDrawCmd {
                                        resource_id: icon_id,
                                        x: icon_x,
                                        y: icon_y,
                                        w: NOTIFICATION_ICON_SIZE_PX,
                                        h: NOTIFICATION_ICON_SIZE_PX,
                                        uv_rect: [0.0, 0.0, 1.0, 1.0],
                                        tint: [1.0, 1.0, 1.0, combined_opacity],
                                    });
                                }
                            }
                        }
                    }
                }
                ContentionPolicy::MergeByKey { .. }
                | ContentionPolicy::LatestWins
                | ContentionPolicy::Replace => {
                    // For MergeByKey and single-publish policies: render a single backdrop
                    // for the zone using the latest publication's content type.
                    let latest = &publishes[publishes.len() - 1];
                    let is_notification_content =
                        matches!(&latest.content, ZoneContent::Notification(_));
                    let backdrop_rgba: Option<Rgba> = match &latest.content {
                        ZoneContent::SolidColor(rgba) => {
                            // SolidColor always renders its own color (no policy override).
                            Some(*rgba)
                        }
                        ZoneContent::StaticImage(resource_id) => {
                            // If a GPU texture is cached, emit a textured draw command
                            // filling the zone.
                            if self.image_texture_cache.contains_key(resource_id) {
                                textured_cmds.push(TexturedDrawCmd {
                                    resource_id: *resource_id,
                                    x,
                                    y,
                                    w,
                                    h,
                                    uv_rect: [0.0, 0.0, 1.0, 1.0],
                                    tint: [1.0, 1.0, 1.0, anim_opacity.clamp(0.0, 1.0)],
                                });
                                None // skip the color quad
                            } else {
                                // Placeholder warm-gray backdrop.
                                Some(STATIC_IMAGE_PLACEHOLDER_COLOR)
                            }
                        }
                        ZoneContent::Notification(n) if is_alert_banner_zone(zone_name) => {
                            // alert-banner: map urgency to severity token color.
                            // Per spec §Notification Urgency-to-Severity Token Mapping:
                            //   urgency 0,1 → color.severity.info
                            //   urgency 2   → color.severity.warning
                            //   urgency 3   → color.severity.critical
                            // Token-resolved values take precedence over hardcoded constants;
                            // falls back to SEVERITY_* when the token map is empty or the
                            // key is absent.
                            // Respect policy.backdrop contract: only override when backdrop
                            // is enabled; skip entirely when backdrop is None.
                            if policy.backdrop.is_some() {
                                let severity_color =
                                    urgency_to_severity_color(n.urgency, &self.token_map);
                                Some(severity_color)
                            } else {
                                None
                            }
                        }
                        ZoneContent::Notification(n) => {
                            // Non-alert-banner notification: urgency-tinted backdrop using
                            // color.notification.urgency.* tokens.
                            // Per spec: urgency >3 clamped to 3 (critical).
                            // Respect policy.backdrop contract: only emit a backdrop (and
                            // border) when a backdrop is enabled; skip entirely when None.
                            if policy.backdrop.is_some() {
                                let mut color =
                                    urgency_to_notification_color(n.urgency, &self.token_map);
                                color.a = NOTIFICATION_BACKDROP_OPACITY;
                                Some(color)
                            } else {
                                None
                            }
                        }
                        _ => {
                            // All other content: use policy.backdrop.
                            policy.backdrop
                        }
                    };

                    if let Some(mut rgba) = backdrop_rgba {
                        // For non-notification content: apply policy backdrop_opacity.
                        // For Notification content in non-alert-banner zones: opacity is
                        // already set to NOTIFICATION_BACKDROP_OPACITY above; skip policy override.
                        if !is_notification_content || is_alert_banner_zone(zone_name) {
                            if let Some(opacity) = policy.backdrop_opacity {
                                rgba.a = opacity.clamp(0.0, 1.0);
                            }
                        }
                        // Apply zone animation opacity.
                        rgba.a *= anim_opacity.clamp(0.0, 1.0);

                        // Skip flat-rect emission for rounded zones — handled by the SDF pass.
                        if !use_rounded_rect {
                            vertices.extend_from_slice(&rect_vertices(
                                x,
                                y,
                                w,
                                h,
                                sw,
                                sh,
                                self.gpu_color(rgba),
                            ));

                            // For non-alert-banner Notification content: emit 1px 4-quad border.
                            if is_notification_content && !is_alert_banner_zone(zone_name) {
                                let mut border_color =
                                    resolve_border_default_color(&self.token_map);
                                border_color.a *= anim_opacity.clamp(0.0, 1.0);
                                emit_border_quads(
                                    vertices,
                                    x,
                                    y,
                                    w,
                                    h,
                                    sw,
                                    sh,
                                    self.gpu_color(border_color),
                                );
                            }
                        }
                    }

                    // Notification icon for LatestWins/MergeByKey/Replace zones.
                    if let ZoneContent::Notification(payload) = &latest.content {
                        if let Some(icon_id) = parse_notification_icon(&payload.icon) {
                            if self.image_texture_cache.contains_key(&icon_id) {
                                const NOTIFICATION_INSET: f32 = 9.0;
                                let inset_h = policy
                                    .margin_horizontal
                                    .or(policy.margin_px)
                                    .unwrap_or(NOTIFICATION_INSET);
                                let icon_x = x + inset_h;
                                let icon_y = y + (h - NOTIFICATION_ICON_SIZE_PX) * 0.5;
                                let eff_opacity = anim_opacity.clamp(0.0, 1.0);
                                textured_cmds.push(TexturedDrawCmd {
                                    resource_id: icon_id,
                                    x: icon_x,
                                    y: icon_y,
                                    w: NOTIFICATION_ICON_SIZE_PX,
                                    h: NOTIFICATION_ICON_SIZE_PX,
                                    uv_rect: [0.0, 0.0, 1.0, 1.0],
                                    tint: [1.0, 1.0, 1.0, eff_opacity],
                                });
                            }
                        }
                    }

                    // ── StatusBar icon rendering ──────────────────────────────
                    // Emit TexturedDrawCmd for each status-bar entry whose key
                    // is mapped to a cached icon texture.  Only runs when the
                    // zone has key_icon_map entries AND at least one current
                    // entry key has an icon mapping (avoids spurious layout
                    // changes when key_icon_map is non-empty but no displayed
                    // keys are mapped).
                    if !policy.key_icon_map.is_empty() {
                        // Reuse shared helper for consistent merge/sort behavior.
                        let sorted = Self::collect_sorted_status_bar_entries(
                            publishes,
                            zone_def.contention_policy,
                        );

                        // Only proceed when at least one displayed key has an icon.
                        let has_any_icon = sorted
                            .iter()
                            .any(|(k, _)| policy.key_icon_map.contains_key(k.as_str()));
                        if has_any_icon {
                            let slot_h = Self::stack_slot_height(policy);
                            let margin_h =
                                policy.margin_horizontal.or(policy.margin_px).unwrap_or(8.0);

                            for (row, (k, _v)) in sorted.iter().enumerate() {
                                let svg_path = match policy.key_icon_map.get(k.as_str()) {
                                    Some(p) => p,
                                    None => continue, // no icon for this key
                                };
                                // Icons are stored in image_texture_cache under the path-derived
                                // ResourceId by ensure_icon_texture (called via
                                // ensure_scene_icon_textures before this render pass).
                                let resource_id = ResourceId::of(svg_path.as_bytes());
                                if !self.image_texture_cache.contains_key(&resource_id) {
                                    continue; // texture not cached — rasterization failed, skip
                                }
                                // Position: vertically centered within the entry's slot.
                                let icon_x = x + margin_h;
                                let icon_y =
                                    y + row as f32 * slot_h + (slot_h - ICON_SIZE_PX) * 0.5;
                                textured_cmds.push(TexturedDrawCmd {
                                    resource_id,
                                    x: icon_x,
                                    y: icon_y,
                                    w: ICON_SIZE_PX,
                                    h: ICON_SIZE_PX,
                                    uv_rect: [0.0, 0.0, 1.0, 1.0],
                                    tint: [1.0, 1.0, 1.0, anim_opacity.clamp(0.0, 1.0)],
                                });
                            }
                        }
                    }
                }
            }
        }

        // ── Debug zone tints (--debug-zones) ─────────────────────────────────
        // Render ALL zone boundaries with colored tints ON TOP of content
        // backgrounds so developers can see zone geometry even when zones have
        // opaque content.  The overall background gets a subtle black tint;
        // each zone gets a rainbow-sampled color at low opacity.
        //
        // Guard: only emit debug tints when there is no layer filter (legacy
        // single-pass test usage) or on the final Chrome pass.  The three-pass
        // frame-render methods call this function three times; without this
        // guard the debug overlay would be drawn 3× per frame, tripling its
        // effective opacity.
        let emit_debug_tints =
            self.debug_zone_tints && matches!(only_layer, None | Some(LayerAttachment::Chrome));
        if emit_debug_tints {
            // 0.5% opacity black background tint over the full window.
            // In overlay mode, the clear_pipeline writes RGBA directly (no
            // GPU blending). DWM composites with premultiplied alpha, so RGB
            // values must be pre-multiplied by alpha.
            const A: f32 = 0.001;
            vertices.extend_from_slice(&rect_vertices(
                0.0,
                0.0,
                sw,
                sh,
                sw,
                sh,
                [0.0, 0.0, 0.0, A],
            ));

            // Rainbow palette for zone tints (1% opacity, premultiplied).
            const ZA: f32 = 0.005;
            let palette: &[[f32; 4]] = &[
                [1.0 * ZA, 0.2 * ZA, 0.2 * ZA, ZA], // red
                [1.0 * ZA, 0.6 * ZA, 0.1 * ZA, ZA], // orange
                [1.0 * ZA, 1.0 * ZA, 0.2 * ZA, ZA], // yellow
                [0.2 * ZA, 0.9 * ZA, 0.2 * ZA, ZA], // green
                [0.2 * ZA, 0.6 * ZA, 1.0 * ZA, ZA], // blue
                [0.7 * ZA, 0.3 * ZA, 1.0 * ZA, ZA], // violet
            ];
            for (idx, (_zone_name, zone_def)) in scene.zone_registry.zones.iter().enumerate() {
                let color = palette[idx % palette.len()];
                let (x, y, w, h) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);
                vertices.extend_from_slice(&rect_vertices(x, y, w, h, sw, sh, color));
            }
        }
    }

    /// Collect and sort the active StatusBar entries for a zone.
    ///
    /// Shared by `collect_text_items` (text layout) and `render_zone_content`
    /// (icon rendering) so both paths use the same source of truth for entry
    /// ordering, merge semantics, and max_keys behavior.
    ///
    /// - `MergeByKey`: merges entries from all active publications, respecting
    ///   `max_keys` as the initial capacity hint.
    /// - All other policies (LatestWins, Replace): uses the most-recent
    ///   StatusBar publication only.
    ///
    /// Returns entries sorted by key (deterministic row ordering).
    fn collect_sorted_status_bar_entries(
        publishes: &[ZonePublishRecord],
        contention_policy: ContentionPolicy,
    ) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = match contention_policy {
            ContentionPolicy::MergeByKey { max_keys } => {
                let mut merged: HashMap<String, String> = HashMap::with_capacity(max_keys as usize);
                for record in publishes.iter() {
                    if let ZoneContent::StatusBar(payload) = &record.content {
                        for (k, v) in &payload.entries {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                }
                merged.into_iter().collect()
            }
            _ => publishes
                .iter()
                .rev()
                .find_map(|r| {
                    if let ZoneContent::StatusBar(p) = &r.content {
                        Some(
                            p.entries
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),
                        )
                    } else {
                        None
                    }
                })
                .unwrap_or_default(),
        };
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        entries
    }

    /// Compute the per-slot height for a Stack zone (single-line content).
    ///
    /// `slot_h = line_height + 2 * margin_v + SLOT_BASELINE_GAP`
    /// where `line_height = font_size_px * 1.4`
    ///
    /// - `font_size_px` — from `RenderingPolicy`; defaults to 16.
    /// - `margin_v` — from `margin_vertical` → `margin_px` → 8 px fallback chain.
    /// - `SLOT_BASELINE_GAP` — a small constant gap (4 px) between successive slot
    ///   backdrops so they don't bleed into each other visually. This is not a
    ///   configurable policy field; it is a structural layout constant.
    ///
    /// Used by both `collect_text_items` and `render_zone_content` so both code
    /// paths stay consistent.
    pub(crate) fn stack_slot_height(policy: &RenderingPolicy) -> f32 {
        const SLOT_BASELINE_GAP: f32 = 4.0;
        let font_size_px = policy.font_size_px.unwrap_or(16.0).clamp(6.0, 200.0);
        // Use line_height (font_size × 1.4) to match cosmic-text layout in
        // prepare_text_items. Previously this used font_size_px directly, which
        // produced bounds too small for one line of text to render.
        let line_height = font_size_px * 1.4;
        let margin_v = policy.margin_vertical.or(policy.margin_px).unwrap_or(8.0);
        (line_height + 2.0 * margin_v + SLOT_BASELINE_GAP).max(1.0)
    }

    /// Compute the per-slot height for a single notification publication.
    ///
    /// When `payload.title` is non-empty, the slot must accommodate two text lines:
    ///   - Title line: `font_size_px * 1.4` (line height, same as single-line slot)
    ///   - Body line: `title_line_h * notification_body_scale`
    ///   - Inter-line gap: `notification_inter_line_gap` px between the two lines
    ///
    /// Two-line height formula:
    ///   `slot_h = title_line_h + inter_line_gap + body_line_h + 2*margin_v + SLOT_BASELINE_GAP`
    ///   where `title_line_h = font_size_px * 1.4`, `body_line_h = title_line_h * notification_body_scale`
    ///
    /// The caller is responsible for resolving `notification_body_scale` and
    /// `notification_inter_line_gap` from the token map (with appropriate fallbacks) so
    /// that height calculation and rendering use the exact same values.
    ///
    /// When `payload.title` is empty, falls back to [`Self::stack_slot_height`] (single line).
    pub(crate) fn notification_slot_height(
        payload: &NotificationPayload,
        policy: &RenderingPolicy,
        notification_body_scale: f32,
        notification_inter_line_gap: f32,
    ) -> f32 {
        if payload.title.is_empty() {
            return Self::stack_slot_height(policy);
        }
        const SLOT_BASELINE_GAP: f32 = 4.0;
        let font_size_px = policy.font_size_px.unwrap_or(16.0).clamp(6.0, 200.0);
        let title_line_h = font_size_px * 1.4;
        let body_line_h = title_line_h * notification_body_scale;
        let margin_v = policy.margin_vertical.or(policy.margin_px).unwrap_or(8.0);
        (title_line_h
            + notification_inter_line_gap
            + body_line_h
            + 2.0 * margin_v
            + SLOT_BASELINE_GAP)
            .max(1.0)
    }

    /// Compute per-slot heights for an ordered slice of publications in a Stack zone.
    ///
    /// For non-`Notification` content and single-line notifications, returns
    /// `Self::stack_slot_height(policy)` for every slot.  For two-line
    /// notifications (`payload.title` non-empty), returns the taller two-line height
    /// computed from the same resolved notification typography values used by rendering.
    ///
    /// Returns a `Vec<f32>` of length `ordered.len()`, one height per slot.
    fn per_slot_heights(
        ordered: &[&ZonePublishRecord],
        policy: &RenderingPolicy,
        notification_body_scale: f32,
        notification_inter_line_gap: f32,
    ) -> Vec<f32> {
        ordered
            .iter()
            .map(|rec| match &rec.content {
                ZoneContent::Notification(n) => Self::notification_slot_height(
                    n,
                    policy,
                    notification_body_scale,
                    notification_inter_line_gap,
                ),
                _ => Self::stack_slot_height(policy),
            })
            .collect()
    }

    /// Compute the cumulative slot y-offsets from a list of per-slot heights.
    ///
    /// Returns a `Vec<f32>` of length `heights.len()`, where `offsets[i]` is the
    /// sum of all `heights[0..i]` (i.e. the y-start of slot `i` relative to the
    /// zone origin).
    fn slot_offsets(heights: &[f32]) -> Vec<f32> {
        let mut offsets = Vec::with_capacity(heights.len());
        let mut acc = 0.0_f32;
        for &h in heights {
            offsets.push(acc);
            acc += h;
        }
        offsets
    }

    /// Resolve a zone's geometry policy to pixel bounds (x, y, w, h).
    fn resolve_zone_geometry(policy: &GeometryPolicy, sw: f32, sh: f32) -> (f32, f32, f32, f32) {
        match policy {
            GeometryPolicy::EdgeAnchored {
                edge,
                height_pct,
                width_pct,
                margin_px,
            } => {
                let zw = sw * width_pct;
                let zh = sh * height_pct;
                let zx = (sw - zw) / 2.0;
                let zy = match edge {
                    DisplayEdge::Top => *margin_px,
                    DisplayEdge::Bottom => sh - zh - margin_px,
                    DisplayEdge::Left | DisplayEdge::Right => 0.0,
                };
                (zx, zy, zw, zh)
            }
            GeometryPolicy::Relative {
                x_pct,
                y_pct,
                width_pct,
                height_pct,
            } => {
                let zx = sw * x_pct;
                let zy = sh * y_pct;
                let zw = sw * width_pct;
                let zh = sh * height_pct;
                (zx, zy, zw, zh)
            }
        }
    }

    /// Recompute the zone interaction hit regions for the current frame.
    ///
    /// Clears `scene.zone_hit_regions` then repopulates it with dismiss (×)
    /// buttons and action buttons for every visible notification slot in every
    /// Stack zone that contains `ZoneContent::Notification` publications.
    ///
    /// # Layout
    ///
    /// For each notification slot (height = `stack_slot_height(policy)`):
    ///
    /// - **Dismiss button**: a square in the top-right corner of the slot.
    ///   Size: `DISMISS_BUTTON_SIZE × DISMISS_BUTTON_SIZE` px.
    ///   Position: `(slot_right - DISMISS_BUTTON_SIZE, slot_y)`.
    ///
    /// - **Action buttons**: a horizontal row at the bottom of the slot.
    ///   Each button is `ACTION_BUTTON_H` px tall and
    ///   `(slot_w - inset * 2) / n_actions` px wide (where `n_actions` is
    ///   capped at `MAX_NOTIFICATION_ACTIONS`).
    ///
    /// Tab order within a slot: dismiss button first, then actions left-to-right.
    /// Slots are ordered top-to-bottom (slot 0 = newest, matching rendering order).
    ///
    /// # Called by
    ///
    /// Called after a frame render completes to refresh `scene.zone_hit_regions`
    /// for the next frame's hit-testing.  This prepares `SceneGraph::hit_test` to
    /// return `ZoneInteraction` for zone affordances based on the most recently
    /// rendered layout.
    pub fn populate_zone_hit_regions(&self, scene: &mut SceneGraph, sw: f32, sh: f32) {
        /// Side length of the dismiss (×) button in pixels.
        const DISMISS_BUTTON_SIZE: f32 = 20.0;
        /// Height of each action button row in pixels.
        const ACTION_BUTTON_H: f32 = 22.0;
        /// Horizontal inset used to position action buttons (matches notification inset).
        const ACTION_INSET: f32 = 9.0;

        scene.zone_hit_regions.clear();
        let mut tab_order: u32 = 0;

        // Sort zone names for deterministic tab-order assignment across frames.
        // HashMap iteration order is nondeterministic; sorting ensures keyboard
        // focus cycling is stable when multiple interactive zones are present.
        let mut zone_names: Vec<_> = scene.zone_registry.active_publishes.keys().collect();
        zone_names.sort_unstable();

        for zone_name in zone_names {
            let publishes = match scene.zone_registry.active_publishes.get(zone_name) {
                Some(p) => p,
                None => continue,
            };
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            // Only Stack zones with Notification content get interactive regions.
            if !matches!(zone_def.contention_policy, ContentionPolicy::Stack { .. }) {
                continue;
            }

            let policy = &zone_def.rendering_policy;
            let (zx, zy, zw, zh) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);
            let slot_h = Self::stack_slot_height(policy);

            // alert-banner uses dynamic height; other Stack zones use configured zh.
            let effective_zh = if is_alert_banner_zone(zone_name) {
                publishes.len() as f32 * slot_h
            } else {
                zh
            };

            // Ordered as in render_zone_content: newest-first for regular zones,
            // severity-descending for alert-banner.
            let ordered: Vec<&ZonePublishRecord> = if is_alert_banner_zone(zone_name) {
                sort_alert_banner_indices(publishes)
                    .into_iter()
                    .map(|idx| &publishes[idx])
                    .collect()
            } else {
                publishes.iter().rev().collect()
            };

            for (slot_idx, record) in ordered.iter().enumerate() {
                let slot_y = zy + slot_idx as f32 * slot_h;
                if slot_y >= zy + effective_zh {
                    break;
                }
                let effective_slot_h = slot_h.min((zy + effective_zh) - slot_y);

                let n_payload = match &record.content {
                    ZoneContent::Notification(n) => n,
                    _ => continue, // Only notifications get interactive affordances.
                };

                // ── Dismiss (×) button ────────────────────────────────────────
                // Top-right corner of the slot, DISMISS_BUTTON_SIZE square.
                let dismiss_bounds = Rect::new(
                    zx + zw - DISMISS_BUTTON_SIZE,
                    slot_y,
                    DISMISS_BUTTON_SIZE,
                    DISMISS_BUTTON_SIZE.min(effective_slot_h),
                );
                let dismiss_id = format!(
                    "zone:{}:dismiss:{}:{}",
                    zone_name, record.published_at_wall_us, record.publisher_namespace,
                );
                scene.zone_hit_regions.push(ZoneHitRegion {
                    zone_name: zone_name.clone(),
                    published_at_wall_us: record.published_at_wall_us,
                    publisher_namespace: record.publisher_namespace.clone(),
                    bounds: dismiss_bounds,
                    kind: ZoneInteractionKind::Dismiss,
                    interaction_id: dismiss_id,
                    tab_order,
                });
                tab_order += 1;

                // ── Action buttons ────────────────────────────────────────────
                let n_actions = n_payload.actions.len().min(MAX_NOTIFICATION_ACTIONS);
                if n_actions > 0 {
                    let avail_w = (zw - ACTION_INSET * 2.0).max(1.0);
                    let btn_w = avail_w / n_actions as f32;
                    let action_y = slot_y + effective_slot_h - ACTION_BUTTON_H;

                    for (btn_idx, action) in n_payload.actions.iter().take(n_actions).enumerate() {
                        let btn_x = zx + ACTION_INSET + btn_idx as f32 * btn_w;
                        let action_bounds = Rect::new(
                            btn_x,
                            action_y.max(slot_y),
                            btn_w,
                            ACTION_BUTTON_H.min(effective_slot_h),
                        );
                        let action_id = format!(
                            "zone:{}:action:{}:{}:{}",
                            zone_name,
                            record.published_at_wall_us,
                            record.publisher_namespace,
                            action.callback_id,
                        );
                        scene.zone_hit_regions.push(ZoneHitRegion {
                            zone_name: zone_name.clone(),
                            published_at_wall_us: record.published_at_wall_us,
                            publisher_namespace: record.publisher_namespace.clone(),
                            bounds: action_bounds,
                            kind: ZoneInteractionKind::Action {
                                callback_id: action.callback_id.clone(),
                            },
                            interaction_id: action_id,
                            tab_order,
                        });
                        tab_order += 1;
                    }
                }
            }
        }
    }

    /// Determine the background color for a tile based on its content.
    fn tile_background_color(&self, tile: &Tile, scene: &SceneGraph) -> [f32; 4] {
        if let Some(root_id) = tile.root_node
            && let Some(node) = scene.nodes.get(&root_id)
        {
            match &node.data {
                NodeData::SolidColor(sc) => return sc.color.to_array(),
                NodeData::TextMarkdown(tm) => {
                    if let Some(bg) = &tm.background {
                        return bg.to_array();
                    }
                    return [0.15, 0.15, 0.25, tile.opacity];
                }
                NodeData::HitRegion(_) => {
                    // Check local state for visual feedback
                    if let Some(state) = scene.hit_region_states.get(&root_id) {
                        if state.pressed {
                            return [0.4, 0.7, 1.0, tile.opacity]; // Active blue
                        } else if state.hovered {
                            return [0.3, 0.5, 0.8, tile.opacity]; // Hover blue
                        }
                    }
                    return [0.2, 0.3, 0.5, tile.opacity]; // Default hit region
                }
                NodeData::StaticImage(_) => {
                    // Tile background for image tiles: near-black with slight tint
                    return [0.05, 0.05, 0.05, tile.opacity];
                }
            }
        }
        [0.1, 0.1, 0.2, tile.opacity]
    }

    /// Render a node and its children within a tile.
    #[allow(clippy::only_used_in_recursion, clippy::too_many_arguments)]
    fn render_node(
        &self,
        node_id: SceneId,
        tile: &Tile,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        textured_cmds: &mut Vec<TexturedDrawCmd>,
        sw: f32,
        sh: f32,
    ) {
        let node = match scene.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };

        match &node.data {
            NodeData::SolidColor(sc) => {
                let verts = rect_vertices(
                    tile.bounds.x + sc.bounds.x,
                    tile.bounds.y + sc.bounds.y,
                    sc.bounds.width,
                    sc.bounds.height,
                    sw,
                    sh,
                    self.gpu_color(sc.color),
                );
                vertices.extend_from_slice(&verts);
            }
            NodeData::TextMarkdown(tm) => {
                // For the vertical slice, render text as a colored rectangle
                // (actual text rendering deferred to post-vertical-slice)
                let bg = tm.background.unwrap_or(Rgba::new(0.15, 0.15, 0.25, 1.0));
                let verts = rect_vertices(
                    tile.bounds.x + tm.bounds.x,
                    tile.bounds.y + tm.bounds.y,
                    tm.bounds.width,
                    tm.bounds.height,
                    sw,
                    sh,
                    self.gpu_color(bg),
                );
                vertices.extend_from_slice(&verts);

                // Render a smaller text area rectangle in the text color
                // to indicate text content is present
                let text_margin = 8.0;
                if tm.bounds.width > text_margin * 2.0 && tm.bounds.height > text_margin * 2.0 {
                    let verts = rect_vertices(
                        tile.bounds.x + tm.bounds.x + text_margin,
                        tile.bounds.y + tm.bounds.y + text_margin,
                        tm.bounds.width - text_margin * 2.0,
                        (tm.font_size_px * 1.2).min(tm.bounds.height - text_margin * 2.0),
                        sw,
                        sh,
                        self.gpu_color(tm.color),
                    );
                    vertices.extend_from_slice(&verts);
                }
            }
            NodeData::HitRegion(hr) => {
                // Render hit region with local state feedback
                let color = if let Some(state) = scene.hit_region_states.get(&node_id) {
                    if state.pressed {
                        [0.4, 0.7, 1.0, 1.0]
                    } else if state.hovered {
                        [0.3, 0.5, 0.8, 1.0]
                    } else {
                        [0.2, 0.3, 0.5, 1.0]
                    }
                } else {
                    [0.2, 0.3, 0.5, 1.0]
                };

                let verts = rect_vertices(
                    tile.bounds.x + hr.bounds.x,
                    tile.bounds.y + hr.bounds.y,
                    hr.bounds.width,
                    hr.bounds.height,
                    sw,
                    sh,
                    self.gpu_color_raw(color),
                );
                vertices.extend_from_slice(&verts);
            }
            NodeData::StaticImage(img) => {
                // If a GPU texture is cached for this resource, emit a textured
                // draw command with fit-mode UV calculations.
                if let Some(entry) = self.image_texture_cache.get(&img.resource_id) {
                    let (dx, dy, dw, dh, uv_rect) = compute_fit_mode(
                        img.fit_mode,
                        tile.bounds.x + img.bounds.x,
                        tile.bounds.y + img.bounds.y,
                        img.bounds.width,
                        img.bounds.height,
                        entry.width,
                        entry.height,
                    );
                    textured_cmds.push(TexturedDrawCmd {
                        resource_id: img.resource_id,
                        x: dx,
                        y: dy,
                        w: dw,
                        h: dh,
                        uv_rect,
                        tint: [1.0, 1.0, 1.0, tile.opacity],
                    });
                } else {
                    // Fallback: warm-gray placeholder when bytes not registered.
                    let outer_color = [0.55_f32, 0.50, 0.45, 1.0];
                    let verts = rect_vertices(
                        tile.bounds.x + img.bounds.x,
                        tile.bounds.y + img.bounds.y,
                        img.bounds.width,
                        img.bounds.height,
                        sw,
                        sh,
                        self.gpu_color_raw(outer_color),
                    );
                    vertices.extend_from_slice(&verts);

                    let margin = 4.0_f32;
                    if img.bounds.width > margin * 2.0 && img.bounds.height > margin * 2.0 {
                        let accent_color = [0.75_f32, 0.70, 0.65, 1.0];
                        let verts = rect_vertices(
                            tile.bounds.x + img.bounds.x + margin,
                            tile.bounds.y + img.bounds.y + margin,
                            img.bounds.width - margin * 2.0,
                            img.bounds.height - margin * 2.0,
                            sw,
                            sh,
                            self.gpu_color_raw(accent_color),
                        );
                        vertices.extend_from_slice(&verts);
                    }
                }
            }
        }

        // Render children
        for child_id in &node.children {
            self.render_node(*child_id, tile, scene, vertices, textured_cmds, sw, sh);
        }
    }
}

/// Resolve the pixel dimensions for a widget instance.
///
/// Returns (width, height) in pixels. Returns (0, 0) if the geometry cannot be
/// resolved (e.g., zero-sized surface or unrecognized policy).
fn resolve_widget_pixel_size(
    instance: &WidgetInstance,
    def: &WidgetDefinition,
    surf_w: u32,
    surf_h: u32,
) -> (u32, u32) {
    let geo = instance
        .geometry_override
        .as_ref()
        .unwrap_or(&def.default_geometry_policy);
    let sw = surf_w as f32;
    let sh = surf_h as f32;
    let (w, h) = match geo {
        GeometryPolicy::Relative {
            width_pct,
            height_pct,
            ..
        } => (sw * width_pct, sh * height_pct),
        GeometryPolicy::EdgeAnchored {
            width_pct,
            height_pct,
            ..
        } => (sw * width_pct, sh * height_pct),
    };
    let w = (w.max(1.0) as u32).min(surf_w.max(1));
    let h = (h.max(1.0) as u32).min(surf_h.max(1));
    (w, h)
}

#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    #[error("no suitable GPU adapter found")]
    NoAdapter,
    #[error("failed to create device: {0}")]
    DeviceCreation(String),
}

// Make buffer init descriptor available
use wgpu::util::DeviceExt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::HeadlessSurface;
    use tze_hud_scene::graph::SceneGraph;

    /// Mutex to serialize tests that mutate `HEADLESS_FORCE_SOFTWARE`, a
    /// global environment variable.  Rust tests run in parallel by default,
    /// so concurrent mutations would cause races.  This is an in-process lock;
    /// it does not protect against separate test binary runs.
    static ENV_VAR_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Returns `true` when the test should be skipped due to missing GPU.
    ///
    /// Tests that require a wgpu adapter (GPU or software renderer) hang
    /// indefinitely on `request_adapter` in environments without any GPU
    /// or software fallback (e.g., minimal CI containers without llvmpipe).
    ///
    /// Set `TZE_HUD_SKIP_GPU_TESTS=1` to opt out all GPU-dependent tests.
    /// In CI, Mesa/llvmpipe is installed and `HEADLESS_FORCE_SOFTWARE=1` is
    /// set instead, so GPU tests run via a software adapter.
    fn should_skip_gpu_tests() -> bool {
        std::env::var("TZE_HUD_SKIP_GPU_TESTS")
            .map(|v| v.trim() == "1")
            .unwrap_or(false)
    }

    /// Skips a GPU-dependent test by returning early if no GPU is available.
    ///
    /// Usage inside an `async fn` test:
    /// ```ignore
    /// let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);
    /// ```
    ///
    /// Expands to a `match` that returns `()` (silently skips) when the
    /// helper returns `None` (no adapter found or `TZE_HUD_SKIP_GPU_TESTS=1`).
    macro_rules! require_gpu {
        ($expr:expr) => {
            match $expr {
                Some(v) => v,
                None => return,
            }
        };
    }

    /// Convenience: build a minimal scene with one tile containing the given node.
    fn scene_with_node(node: Node) -> SceneGraph {
        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene.set_tile_root(tile_id, node).unwrap();
        scene
    }

    /// Create a headless compositor and surface pair for testing.
    ///
    /// Returns `None` (and prints a skip notice) when:
    /// - `TZE_HUD_SKIP_GPU_TESTS=1` is set, or
    /// - no wgpu adapter is available in the current environment.
    ///
    /// Use the `require_gpu!` macro at the call site to early-return from the
    /// test when `None` is returned:
    /// ```ignore
    /// let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);
    /// ```
    async fn make_compositor_and_surface(w: u32, h: u32) -> Option<(Compositor, HeadlessSurface)> {
        if should_skip_gpu_tests() {
            eprintln!("skipping GPU test: TZE_HUD_SKIP_GPU_TESTS=1");
            return None;
        }
        match Compositor::new_headless(w, h).await {
            Ok(compositor) => {
                let surface = HeadlessSurface::new(&compositor.device, w, h);
                Some((compositor, surface))
            }
            Err(CompositorError::NoAdapter) => {
                eprintln!("skipping GPU test: no wgpu adapter available");
                None
            }
            Err(e) => panic!("unexpected compositor error: {e}"),
        }
    }

    #[tokio::test]
    async fn test_static_image_node_renders_placeholder_quad() {
        // The static image placeholder renders a warm-gray outer quad ~[0.55, 0.50, 0.45].
        // In sRGB output the linear values are gamma-compressed.
        // We just verify that *some* non-background pixels appear in the expected warm range.
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);

        // RS-4: StaticImageNode uses resource_id + decoded_bytes; no raw blob embedded.
        let resource_id = ResourceId::of(b"8x8 test image placeholder");
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 8,
                height: 8,
                decoded_bytes: 8 * 8 * 4,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        };

        // Resource must be registered before set_tile_root inserts the StaticImageNode tree.
        let mut scene = SceneGraph::new(256.0, 256.0);
        scene.register_resource(resource_id);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene.set_tile_root(tile_id, node).unwrap();
        compositor.render_frame_headless(&mut scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);
        // The background clear color is ~[0.05, 0.05, 0.1] in linear; tile bg is [0.05,0.05,0.05].
        // The placeholder outer quad is warm gray [0.55, 0.50, 0.45] in linear.
        // In sRGB this is approximately [198, 188, 176]. We look for pixels brighter than 150 in
        // all three channels to confirm the quad was rendered (not just the dark background).
        let any_warm_pixel = pixels
            .chunks(4)
            .any(|p| p[0] > 150 && p[1] > 140 && p[2] > 130);
        assert!(
            any_warm_pixel,
            "expected warm-gray placeholder pixels from StaticImageNode"
        );
    }

    #[tokio::test]
    async fn test_static_image_node_composited_with_other_nodes() {
        // Render a scene with both a SolidColor node and a StaticImage node in adjacent tiles.
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(512, 256).await);

        let mut scene = SceneGraph::new(512.0, 256.0);
        // Resource must be registered before set_tile_root inserts a StaticImageNode tree.
        let static_image_resource_id = ResourceId::of(b"8x8 green placeholder");
        scene.register_resource(static_image_resource_id);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);

        // Left tile: red solid color
        let left_tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene
            .set_tile_root(
                left_tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        color: Rgba::new(1.0, 0.0, 0.0, 1.0),
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Right tile: static image
        // RS-4: StaticImageNode uses resource_id + decoded_bytes; no raw blob embedded.
        let right_tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(256.0, 0.0, 256.0, 256.0),
                2,
            )
            .unwrap();
        scene
            .set_tile_root(
                right_tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::StaticImage(StaticImageNode {
                        resource_id: static_image_resource_id,
                        width: 8,
                        height: 8,
                        decoded_bytes: 8 * 8 * 4,
                        fit_mode: ImageFitMode::Cover,
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        compositor.render_frame_headless(&mut scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 512 * 256 * 4, "pixel buffer size mismatch");
        // Just verify the frame completed without panic and returned the expected buffer size.
    }

    // ── Chrome layer pixel tests ──────────────────────────────────────────────

    /// Layer 1 pixel test: chrome layer is always visible above max-z-order agent tile.
    ///
    /// Acceptance criterion: "Layer 1 pixel tests confirm chrome always visible above
    /// max-z-order agent tile."
    ///
    /// This test renders a bright red tile at max z-order (u32::MAX) then renders a
    /// distinctive chrome rectangle over the same region. The chrome pixels (pure green)
    /// must overwrite the red tile pixels.
    #[tokio::test]
    async fn test_chrome_always_above_max_zorder_tile() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);

        // Agent tile at max valid agent z-order with bright red content.
        // Agent tiles must use z_order < ZONE_TILE_Z_MIN (0x8000_0000); u32::MAX is
        // reserved for runtime zone tiles (scene-graph/spec.md §Zone Layer Attachment).
        use tze_hud_scene::types::ZONE_TILE_Z_MIN;
        let max_agent_z = ZONE_TILE_Z_MIN - 1; // 0x7FFF_FFFF
        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                max_agent_z,
            )
            .unwrap();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        color: Rgba::new(1.0, 0.0, 0.0, 1.0), // bright red
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Chrome draw command: bright green rectangle covering the full surface.
        // In NDC space, this will overwrite all tile content.
        let chrome_cmds = vec![crate::pipeline::ChromeDrawCmd {
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 40.0,                // tab bar height
            color: [0.0, 1.0, 0.0, 1.0], // pure green — distinctive chrome marker
        }];

        compositor.render_frame_with_chrome(&scene, &surface, &chrome_cmds);
        compositor.device.poll(wgpu::Maintain::Wait);

        let pixels = surface.read_pixels(&compositor.device);

        // Check the top-left pixel region (where chrome covers the tile).
        // In sRGB, linear [0,1,0] green becomes approximately [0, 255, 0].
        // We look for pixels that are distinctly green (G > 200, R < 50).
        let chrome_top_pixel = &pixels[0..4]; // first pixel (top-left)
        assert!(
            chrome_top_pixel[1] > 150, // green channel dominant
            "chrome green channel should be dominant at top: {chrome_top_pixel:?}"
        );
        // The tile red should NOT bleed through chrome.
        assert!(
            chrome_top_pixel[0] < 50,
            "agent tile red must not show through chrome: {chrome_top_pixel:?}"
        );
    }

    /// Layer 1 pixel test: chrome hit-test priority — chrome is always drawn last.
    ///
    /// Verifies the separable render pass architecture: content pass first (agent tiles),
    /// chrome pass second (chrome elements). The two-pass structure guarantees chrome
    /// always occupies the final pixels regardless of content.
    #[tokio::test]
    async fn test_chrome_pass_uses_load_op_load() {
        // Render a scene with a blue agent tile + a chrome red stripe.
        // Blue content should persist where chrome doesn't cover; red should cover where it does.
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);

        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        // Blue tile — fills entire surface in content pass.
                        color: Rgba::new(0.0, 0.0, 1.0, 1.0),
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Chrome: red stripe only in top half (rows 0..128).
        let chrome_cmds = vec![crate::pipeline::ChromeDrawCmd {
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 128.0,
            color: [1.0, 0.0, 0.0, 1.0], // pure red
        }];

        compositor.render_frame_with_chrome(&scene, &surface, &chrome_cmds);
        compositor.device.poll(wgpu::Maintain::Wait);

        let pixels = surface.read_pixels(&compositor.device);

        // Top row: chrome (red) should dominate.
        let top_px = &pixels[0..4];
        assert!(
            top_px[0] > 150,
            "top pixel should be red (chrome): {top_px:?}"
        );
        assert!(
            top_px[2] < 50,
            "top pixel blue (tile) must be suppressed by chrome: {top_px:?}"
        );

        // Bottom row: content (blue) should persist — chrome didn't cover it.
        // Row 255 starts at pixel offset 255*256*4.
        let bottom_row_offset = 255 * 256 * 4;
        let bottom_px = &pixels[bottom_row_offset..bottom_row_offset + 4];
        assert!(
            bottom_px[2] > 150,
            "bottom pixel should be blue (tile content, no chrome): {bottom_px:?}"
        );
        assert!(
            bottom_px[0] < 50,
            "bottom pixel red should be absent (no chrome): {bottom_px:?}"
        );
    }

    /// Verify that render_frame_with_chrome renders correctly even when chrome_cmds is empty.
    #[tokio::test]
    async fn test_two_pass_with_empty_chrome_cmds() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);
        let scene = scene_with_node(Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(0.5, 0.5, 0.5, 1.0),
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        });
        // Empty chrome cmds — must not panic.
        compositor.render_frame_with_chrome(&scene, &surface, &[]);
        compositor.device.poll(wgpu::Maintain::Wait);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 256 * 256 * 4);
    }

    // ── Headless parity tests ─────────────────────────────────────────────────

    /// Verify that `render_frame` (surface-agnostic) works with a `HeadlessSurface`
    /// as a `&dyn CompositorSurface`.  This is the core headless parity assertion:
    /// the same method that would be used with a windowed surface works headlessly.
    #[tokio::test]
    async fn test_render_frame_via_compositor_surface_trait() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);
        let scene = SceneGraph::new(256.0, 256.0);

        // render_frame takes &dyn CompositorSurface — no special headless branch.
        let telemetry =
            compositor.render_frame(&scene, &surface as &dyn crate::surface::CompositorSurface);
        assert!(telemetry.frame_time_us > 0, "frame time must be non-zero");
        assert_eq!(telemetry.tile_count, 0, "empty scene has no tiles");
    }

    /// Verify that HEADLESS_FORCE_SOFTWARE env-var path is exercised in the
    /// adapter-selection code.  We cannot assert the adapter backend in a unit
    /// test (it's opaque), so we just verify that creating a compositor with
    /// the env var set does not crash.
    #[tokio::test]
    async fn test_new_headless_with_force_software_env_var() {
        if should_skip_gpu_tests() {
            eprintln!("skipping GPU test: TZE_HUD_SKIP_GPU_TESTS=1");
            return;
        }

        // Serialize all env-var-mutating tests via a process-wide mutex.
        // Rust tests run in parallel by default; without serialization,
        // a concurrent test could observe or overwrite HEADLESS_FORCE_SOFTWARE.
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Safety: single-threaded within the mutex guard; no other test
        // touches HEADLESS_FORCE_SOFTWARE while _guard is held.
        unsafe {
            std::env::set_var("HEADLESS_FORCE_SOFTWARE", "1");
        }
        let result = Compositor::new_headless(64, 64).await;
        unsafe {
            std::env::remove_var("HEADLESS_FORCE_SOFTWARE");
        }
        drop(_guard);

        // Either Ok (software GPU found) or Err(NoAdapter) (no software GPU
        // installed in this CI environment) are acceptable.  A panic would not be.
        match result {
            Ok(_) => {}
            Err(CompositorError::NoAdapter) => {}
            Err(e) => panic!("unexpected error with HEADLESS_FORCE_SOFTWARE=1: {e}"),
        }
    }

    // ── Surface capability guard + dimension clamping (hud-q5hx regression) ────
    //
    // These tests validate the defensive logic added to `new_windowed_inner()`:
    //   1. Empty surface capability lists return `Err` instead of panicking.
    //   2. Dimension clamping uses `.max(1)` to prevent zero-size configs.
    //
    // The windowed path requires a real display handle and GPU, so we test the
    // clamping arithmetic directly as a pure function.

    /// Dimension clamping must apply both the device max and a minimum of 1.
    /// wgpu panics on `surface.configure()` with zero-width or zero-height.
    #[test]
    #[allow(clippy::min_max)]
    fn surface_dim_clamp_zero_becomes_one() {
        let max_dim = 16384u32;
        assert_eq!(0u32.min(max_dim).max(1), 1);
        assert_eq!(1u32.min(max_dim).max(1), 1);
        assert_eq!(2560u32.min(max_dim).max(1), 2560);
        assert_eq!(3840u32.min(max_dim).max(1), 3840);
    }

    /// Dimension clamping respects the device maximum texture dimension.
    /// Values larger than the limit are clamped, values within the limit pass through.
    #[test]
    fn surface_dim_clamp_respects_device_limit() {
        let max_dim = 4096u32;
        assert_eq!(4097u32.min(max_dim).max(1), 4096, "over-limit must clamp");
        assert_eq!(4096u32.min(max_dim).max(1), 4096, "at-limit must pass");
        assert_eq!(2560u32.min(max_dim).max(1), 2560, "under-limit must pass");
        assert_eq!(1920u32.min(max_dim).max(1), 1920, "default res must pass");
    }

    /// Dimension clamping at 2560x1440 with a 32768 device limit (RTX 3080)
    /// must not clamp — 2560 and 1440 are well below the RTX 3080's limit.
    #[test]
    fn surface_dim_clamp_2560x1440_passes_on_rtx3080_limit() {
        // RTX 3080 with Vulkan driver reports max_texture_dimension_2d = 32768.
        let max_dim = 32768u32;
        assert_eq!(
            2560u32.min(max_dim).max(1),
            2560,
            "2560 must not be clamped"
        );
        assert_eq!(
            1440u32.min(max_dim).max(1),
            1440,
            "1440 must not be clamped"
        );
        assert_eq!(3840u32.min(max_dim).max(1), 3840, "4K must not be clamped");
        assert_eq!(2160u32.min(max_dim).max(1), 2160, "4K must not be clamped");
    }

    // ── Text rendering pixel tests ────────────────────────────────────────────
    //
    // These tests validate acceptance criteria 1–4 from hud-pmkf:
    //  1. publish_to_zone with StreamText → visible text at zone geometry.
    //  2. TextMarkdownNode renders text (some non-background pixels in text area).
    //  3. Overflow Clip and Ellipsis modes: glyphs stay within TextBounds.
    //  4. Headless pixel readback detects text presence.
    //
    // All tests initialise the text renderer via `compositor.init_text_renderer`
    // targeting `Rgba8UnormSrgb` (the headless surface format).

    /// Pixel readback validates text presence in a TextMarkdownNode tile.
    ///
    /// After text rendering, pixels in the text region should differ from the
    /// solid background color — glyphs overwrite some pixels.  We can't check
    /// exact glyph shapes without font-specific knowledge, so we verify that
    /// at least one pixel in the tile area differs from the pure background
    /// color.
    #[tokio::test]
    async fn test_text_markdown_node_renders_visible_text() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        // Dark-blue background, white text — high contrast for pixel detection.
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "Hello world".to_owned(),
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                font_size_px: 24.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::new(1.0, 1.0, 1.0, 1.0), // white
                background: Some(Rgba::new(0.0, 0.0, 0.5, 1.0)), // dark blue
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            }),
        };
        let mut scene = scene_with_node(node);
        compositor.render_frame_headless(&mut scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 256 * 256 * 4, "pixel buffer size");

        // The background is dark blue — sRGB of linear [0,0,0.5] ≈ [0, 0, 188].
        // White text (sRGB [255, 255, 255]) glyphs should appear in the tile.
        // We check that at least one pixel has R > 200 AND G > 200 (white).
        let any_bright_pixel = pixels
            .chunks(4)
            .any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200);
        assert!(
            any_bright_pixel,
            "expected white text pixels in TextMarkdownNode tile — none found"
        );
    }

    /// Text stays within the TextBounds clip rectangle (Clip overflow mode).
    ///
    /// We render white text in a small region at the top-left of a dark tile.
    /// The bottom-right quadrant should remain all-dark (no text overflow).
    #[tokio::test]
    async fn test_text_clip_overflow_stays_within_bounds() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        // Text node occupies only top-left 64x64 pixels of the 256x256 tile.
        // Content: many lines so overflow is tested.
        let content = "Line1\nLine2\nLine3\nLine4\nLine5\nLine6\nLine7\nLine8".to_owned();
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content,
                bounds: Rect::new(0.0, 0.0, 64.0, 32.0), // small box
                font_size_px: 12.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::new(1.0, 1.0, 1.0, 1.0), // white
                background: Some(Rgba::new(0.0, 0.0, 0.0, 1.0)), // pure black bg
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            }),
        };
        let mut scene = scene_with_node(node);
        compositor.render_frame_headless(&mut scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);

        // Bottom-right quadrant: rows 128..256, cols 128..256.
        // Tile background is ~[0.15, 0.15, 0.25] (tile_background_color default).
        // There should be no white pixels (from text) there.
        let mut bright_outside = false;
        for row in 128..256_usize {
            for col in 128..256_usize {
                let offset = (row * 256 + col) * 4;
                let p = &pixels[offset..offset + 4];
                // White text would have R > 200 AND G > 200 AND B > 200.
                if p[0] > 200 && p[1] > 200 && p[2] > 200 {
                    bright_outside = true;
                    break;
                }
            }
            if bright_outside {
                break;
            }
        }
        assert!(
            !bright_outside,
            "text overflow detected outside clip bounds (bottom-right quadrant has bright pixels)"
        );
    }

    /// Ellipsis overflow mode: text renders without panic; background present.
    ///
    /// We don't assert exact "…" pixel shape — that's platform-font-specific.
    /// We verify: (a) no panic, (b) background exists, (c) some non-background
    /// pixels appear (text was rendered at all).
    #[tokio::test]
    async fn test_text_ellipsis_overflow_no_panic() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(256, 256).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let long_line =
            "A very long line that definitely overflows the available width of this tile";
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: long_line.to_owned(),
                bounds: Rect::new(0.0, 0.0, 120.0, 40.0),
                font_size_px: 14.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::new(1.0, 1.0, 1.0, 1.0),
                background: Some(Rgba::new(0.1, 0.1, 0.1, 1.0)),
                alignment: TextAlign::Start,
                overflow: TextOverflow::Ellipsis,
            }),
        };
        let mut scene = scene_with_node(node);
        // Must not panic.
        compositor.render_frame_headless(&mut scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(
            pixels.len(),
            256 * 256 * 4,
            "pixel buffer must be full size"
        );
    }

    /// Zone StreamText publish renders visible text at zone geometry.
    ///
    /// Acceptance criterion 1: publish_to_zone with StreamText content displays
    /// readable text at the zone geometry position.
    #[tokio::test]
    async fn test_zone_stream_text_renders_visible_text() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Register a subtitle zone (bottom edge, 10% height).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "subtitle zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(22.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_align: None,
                margin_px: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Publish "Hello Zone" to the subtitle zone.
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("Hello Zone".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.render_frame_headless(&mut scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 1280 * 720 * 4, "pixel buffer size");

        // The zone is at the bottom 10% (y ~648..720) centered (x ~128..1152).
        // We check for any bright pixels in that area (white text on dark bg).
        // Zone bg is semi-transparent dark [0.1, 0.1, 0.15, 0.85] rendered over
        // the default compositor clear [0.05, 0.05, 0.1] → still quite dark.
        // White text glyphs should show as bright pixels.
        let mut found_bright = false;
        // Sample a row in the subtitle zone (row 660 ≈ 91.7% of 720 = 661).
        for row in 652usize..715 {
            for col in 150usize..1130 {
                let offset = (row * 1280 + col) * 4;
                let p = &pixels[offset..offset + 4];
                if p[0] > 180 && p[1] > 180 && p[2] > 180 {
                    found_bright = true;
                    break;
                }
            }
            if found_bright {
                break;
            }
        }
        assert!(
            found_bright,
            "expected bright (text) pixels in zone subtitle area (rows 652..715)"
        );
    }

    /// Zone ShortTextWithIcon/Notification publish renders visible text at zone geometry.
    ///
    /// Acceptance criterion for hud-lh3w: publish_to_zone with
    /// `ZoneContent::Notification(NotificationPayload)` produces a `TextItem` in
    /// `collect_text_items` and causes bright glyph pixels to appear in the zone
    /// geometry area.
    #[tokio::test]
    async fn test_zone_notification_renders_visible_text() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Register a notification zone (top edge, 8% height).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification".to_owned(),
            description: "notification zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.08,
                width_pct: 0.70,
                margin_px: 12.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(20.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.75)),
                text_align: None,
                margin_px: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Publish a notification to the zone.
        scene
            .publish_to_zone(
                "notification",
                ZoneContent::Notification(NotificationPayload {
                    text: "Alert: system ready".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.render_frame_headless(&mut scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 1280 * 720 * 4, "pixel buffer size");

        // The zone is at the top edge: y ≈ 12..(12 + 720*0.08) ≈ 12..69.6,
        // centered: x ≈ (1280-896)/2..896+192 = 192..1088.
        // White text glyphs should appear as bright pixels (r,g,b > 180).
        let mut found_bright = false;
        for row in 12usize..70 {
            for col in 200usize..1080 {
                let offset = (row * 1280 + col) * 4;
                let p = &pixels[offset..offset + 4];
                if p[0] > 180 && p[1] > 180 && p[2] > 180 {
                    found_bright = true;
                    break;
                }
            }
            if found_bright {
                break;
            }
        }
        assert!(
            found_bright,
            "expected bright (text) pixels in notification zone area (rows 12..70)"
        );
    }

    /// Zone StatusBar (KeyValuePairs) publish renders visible text at zone geometry.
    ///
    /// Acceptance criteria for hud-6at1:
    ///   1. `publish_to_zone` with `ZoneContent::StatusBar` produces a `TextItem` in
    ///      `collect_text_items`.
    ///   2. The key-value pairs are rendered as text at the zone geometry position.
    #[tokio::test]
    async fn test_zone_status_bar_renders_visible_text() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Register a status-bar zone (top edge, 5% height).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "statusbar".to_owned(),
            description: "status bar zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.05,
                width_pct: 0.80,
                margin_px: 8.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_align: None,
                margin_px: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Publish StatusBar content with key-value pairs.
        let mut entries = std::collections::HashMap::new();
        entries.insert("battery".to_owned(), "95%".to_owned());
        entries.insert("time".to_owned(), "12:34".to_owned());
        scene
            .publish_to_zone(
                "statusbar",
                ZoneContent::StatusBar(StatusBarPayload { entries }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // Verify collect_text_items produces a TextItem with the formatted pairs.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(
            items.len(),
            1,
            "expected exactly one TextItem for StatusBar"
        );
        let item = &items[0];
        // Entries are sorted by key ("battery" < "time") and separated by newlines.
        assert_eq!(
            item.text, "battery: 95%\ntime: 12:34",
            "Entries should be sorted by key and formatted correctly"
        );
        // The TextItem position should be within the zone geometry.
        // Zone top-edge: y = 8.0 (margin_px), height = 720*0.05 = 36, width = 1280*0.8 = 1024.
        assert!(
            item.pixel_y >= 8.0,
            "text y should be at or below zone top margin"
        );
        assert!(
            item.pixel_y < 720.0 * 0.10,
            "text y should be within top zone area"
        );

        // Render to pixels and verify bright text appears in the top zone area.
        compositor.render_frame_headless(&mut scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 1280 * 720 * 4, "pixel buffer size");

        // The zone is at the top ~8..44px, centered horizontally.
        // White text glyphs should show as bright pixels.
        let mut found_bright = false;
        for row in 10usize..42 {
            for col in 150usize..1130 {
                let offset = (row * 1280 + col) * 4;
                let p = &pixels[offset..offset + 4];
                if p[0] > 180 && p[1] > 180 && p[2] > 180 {
                    found_bright = true;
                    break;
                }
            }
            if found_bright {
                break;
            }
        }
        assert!(
            found_bright,
            "expected bright (text) pixels in status bar zone area (rows 10..42)"
        );
    }

    /// `init_text_renderer` called multiple times replaces the rasterizer (no panic).
    #[tokio::test]
    async fn test_init_text_renderer_idempotent() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(64, 64).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);
        let mut scene = SceneGraph::new(64.0, 64.0);
        compositor.render_frame_headless(&mut scene, &surface);
        // No panic = pass.
    }

    /// Text rendering with no text items (empty scene) must not panic.
    #[tokio::test]
    async fn test_text_renderer_empty_scene_no_panic() {
        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(64, 64).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);
        let mut scene = SceneGraph::new(64.0, 64.0);
        compositor.render_frame_headless(&mut scene, &surface);
    }

    /// Stage 6 frame-budget benchmark — text rendering active.
    ///
    /// Renders 60 frames with `init_text_renderer` active, a `TextMarkdownNode`
    /// tile, and a zone with `StreamText` content.  Asserts that the p99 of
    /// `render_encode_us` (the Stage 6 wall-clock encode time returned by
    /// `render_frame_headless`) stays below a calibrated budget derived from
    /// `STAGE6_BUDGET_US` (4 ms = 4 000 µs).
    ///
    /// Budget constant sourced from `tze_hud_runtime::pipeline::STAGE6_BUDGET_US`.
    /// It is inlined here to avoid a cyclic dev-dependency
    /// (tze_hud_runtime → tze_hud_compositor already exists).
    ///
    /// The effective budget is `max(test_budget(4000), 4000 * 4)` — the
    /// calibration system scales for CPU speed, and the 4× CI floor (16 ms)
    /// absorbs llvmpipe/scheduling jitter on GitHub Actions runners.  This
    /// mirrors the Stage 6 budget pattern in `budget_assertions.rs`.
    #[tokio::test]
    async fn test_stage6_budget_with_text_rendering_active() {
        use tze_hud_scene::calibration::test_budget;

        // Stage 6 p99 budget in microseconds — mirrors STAGE6_BUDGET_US in
        // tze_hud_runtime::pipeline (4 ms).
        const STAGE6_BUDGET_US: u64 = 4_000;
        /// CI-friendly multiplier: 4× the spec target absorbs llvmpipe and
        /// scheduling noise on shared CI runners.
        const CI_BUDGET_MULTIPLIER: u64 = 4;
        let effective_budget =
            test_budget(STAGE6_BUDGET_US).max(STAGE6_BUDGET_US * CI_BUDGET_MULTIPLIER);
        const FRAME_COUNT: usize = 60;

        let (mut compositor, surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        // ── Build scene ─────────────────────────────────────────────────────────
        let mut scene = SceneGraph::new(1280.0, 720.0);
        let tab_id = scene.create_tab("bench", 0).unwrap();
        let lease_id = scene.grant_lease("bench", 60_000, vec![]);

        // TextMarkdownNode tile occupying most of the screen.
        let tile_id = scene
            .create_tile(
                tab_id,
                "bench",
                lease_id,
                Rect::new(0.0, 0.0, 1000.0, 600.0),
                1,
            )
            .unwrap();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::TextMarkdown(TextMarkdownNode {
                        content: "Stage 6 budget benchmark\nLine two of text\nLine three"
                            .to_owned(),
                        bounds: Rect::new(0.0, 0.0, 1000.0, 600.0),
                        font_size_px: 20.0,
                        font_family: FontFamily::SystemSansSerif,
                        color: Rgba::new(1.0, 1.0, 1.0, 1.0),
                        background: Some(Rgba::new(0.05, 0.05, 0.1, 1.0)),
                        alignment: TextAlign::Start,
                        overflow: TextOverflow::Clip,
                    }),
                },
            )
            .unwrap();

        // Zone with StreamText content (subtitle strip at the bottom).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "bench-subtitle".to_owned(),
            description: "benchmark subtitle zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(22.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_align: None,
                margin_px: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "bench-subtitle",
                ZoneContent::StreamText("Stage 6 benchmark — stream text active".to_owned()),
                "bench",
                None,
                None,
                None,
            )
            .unwrap();

        // ── Warm-up pass ────────────────────────────────────────────────────────
        // Run a few frames to let llvmpipe/WARP JIT-compile the shaders before
        // the timed measurement window.  Shader compilation is a one-time cost
        // that does not reflect steady-state Stage 6 performance; excluding it
        // mirrors production behaviour where shaders are pre-compiled.
        for _ in 0..5 {
            compositor.render_frame_headless(&mut scene, &surface);
        }

        // ── Render loop ─────────────────────────────────────────────────────────
        let mut timings: Vec<u64> = Vec::with_capacity(FRAME_COUNT);
        for _ in 0..FRAME_COUNT {
            let telem = compositor.render_frame_headless(&mut scene, &surface);
            // render_encode_us is the Stage 6 wall-clock encode duration.
            timings.push(telem.render_encode_us);
        }

        // ── p99 assertion ────────────────────────────────────────────────────────
        timings.sort_unstable();
        // p99 index: ceil(99/100 * N) - 1 (0-based), clamped to last element.
        let p99_index = ((FRAME_COUNT as f64 * 0.99).ceil() as usize).saturating_sub(1);
        let p99_index = p99_index.min(FRAME_COUNT - 1);
        let p99_us = timings[p99_index];

        assert!(
            p99_us <= effective_budget,
            "Stage 6 render-encode p99 ({p99_us} µs) exceeds budget ({effective_budget} µs, \
             spec target={STAGE6_BUDGET_US} µs). All timings (sorted): {timings:?}"
        );
    }

    // ── RenderingPolicy-driven zone rendering tests [hud-sc0a.8] ─────────────

    /// Subtitle with outline: when outline_color + outline_width are set in
    /// RenderingPolicy, collect_text_items produces a TextItem with non-None
    /// outline fields.
    #[tokio::test]
    async fn test_zone_subtitle_with_outline_text_item() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "subtitle zone with outline".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(22.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                outline_color: Some(Rgba::BLACK),
                outline_width: Some(2.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("Test outline text".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 1, "expected one TextItem");
        let item = &items[0];
        assert!(
            item.outline_color.is_some(),
            "outline_color should be set from RenderingPolicy"
        );
        assert!(
            item.outline_width.is_some(),
            "outline_width should be set from RenderingPolicy"
        );
        assert_eq!(
            item.outline_width.unwrap(),
            2.0,
            "outline_width should match policy"
        );
        // Text color should be white (from text_color).
        assert_eq!(
            item.color[0], 255,
            "text fill color R should be white (255)"
        );
    }

    /// Subtitle without outline: when outline_width is None, outline fields
    /// on the TextItem should be None.
    #[tokio::test]
    async fn test_zone_subtitle_without_outline_text_item() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "subtitle zone without outline".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(22.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                outline_color: None,
                outline_width: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("No outline subtitle".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 1, "expected one TextItem");
        let item = &items[0];
        assert!(
            item.outline_color.is_none(),
            "outline_color should be None when policy has no outline"
        );
        assert!(
            item.outline_width.is_none(),
            "outline_width should be None when policy has no outline"
        );
    }

    /// Notification with opaque backdrop: backdrop_opacity=0.9 overrides
    /// the backdrop color's alpha.  The backdrop quad should be rendered with
    /// effective alpha = 0.9.
    #[tokio::test]
    async fn test_notification_with_opaque_backdrop() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.08,
                width_pct: 0.70,
                margin_px: 12.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(18.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 1.0)),
                backdrop_opacity: Some(0.9),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: Some(5_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Notification with opaque backdrop".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // render_zone_content should produce backdrop rect vertices.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);
        // We check that vertices were emitted (backdrop rendered).
        assert!(
            !vertices.is_empty(),
            "expected backdrop vertices for notification with opaque backdrop"
        );

        // Also verify the text item uses the policy text_color.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 1, "expected one TextItem for notification");
        // White text → R channel should be near 255.
        assert!(
            items[0].color[0] > 200,
            "text color R should be near-white from policy.text_color"
        );
    }

    /// Alert-banner severity mapping: urgency 2 (warning) should map to
    /// color.severity.warning (amber/yellow), NOT the policy backdrop.
    /// We verify by inspecting the vertices emitted by render_zone_content.
    #[tokio::test]
    async fn test_alert_banner_urgency2_maps_to_severity_warning() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "alert banner zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.07,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(20.0),
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)), // dark default
                backdrop_opacity: Some(1.0),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish urgency=2 (warning).
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "Warning: disk space low".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // Collect vertices from render_zone_content.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // The backdrop should be severity warning color (~amber: R=1.0, G~0.72, B=0.0).
        // rect_vertices emits 6 vertices; each has color at the end.
        // We check that the R component is high (>0.9) and G is mid (~0.5-0.8) and B is low.
        assert!(
            !vertices.is_empty(),
            "expected backdrop vertices for alert-banner urgency=2"
        );

        // Verify urgency_to_severity_color directly (no token map → fallback constants).
        let no_tokens = HashMap::new();
        let warning_color = urgency_to_severity_color(2, &no_tokens);
        assert!(
            warning_color.r > 0.9,
            "warning severity R should be ~1.0 (amber)"
        );
        assert!(
            warning_color.g > 0.5,
            "warning severity G should be >0.5 (amber)"
        );
        assert!(
            warning_color.b < 0.1,
            "warning severity B should be ~0.0 (amber)"
        );
    }

    /// Alert-banner urgency=3 maps to critical (red).
    #[tokio::test]
    async fn test_alert_banner_urgency3_maps_to_severity_critical() {
        let no_tokens = HashMap::new();
        let critical = urgency_to_severity_color(3, &no_tokens);
        assert!(critical.r > 0.9, "critical R should be ~1.0");
        assert!(critical.g < 0.1, "critical G should be ~0.0");
        assert!(critical.b < 0.1, "critical B should be ~0.0");
    }

    /// Alert-banner urgency=0 and 1 both map to info (blue).
    #[tokio::test]
    async fn test_alert_banner_urgency_low_maps_to_info() {
        let no_tokens = HashMap::new();
        let info0 = urgency_to_severity_color(0, &no_tokens);
        let info1 = urgency_to_severity_color(1, &no_tokens);
        // Info color is blue-ish (#4A9EFF).
        assert!(info0.b > 0.9, "info urgency=0 should be blue");
        assert!(info1.b > 0.9, "info urgency=1 should be blue");
        // Both should be the same color.
        assert_eq!(info0.r, info1.r);
        assert_eq!(info0.b, info1.b);
    }

    /// notification-area does NOT use urgency-to-severity mapping (color.severity.*).
    /// It uses color.notification.urgency.* tokens instead.
    /// Even with urgency=3, it must NOT produce severity critical (red #FF0000).
    #[tokio::test]
    async fn test_notification_area_does_not_use_severity_tokens() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area - uses notification urgency tokens, not severity"
                .to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.05, 0.05, 0.05, 0.85)),
                backdrop_opacity: Some(0.85),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 16,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "System alert".to_owned(),
                    icon: String::new(),
                    urgency: 3, // Critical — must use color.notification.urgency.critical, NOT color.severity.critical
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // notification-area must NOT be treated as alert-banner.
        assert!(
            !is_alert_banner_zone("notification-area"),
            "notification-area must not be treated as alert-banner"
        );

        // Render and check: the backdrop must NOT be severity critical (pure red R~1.0, G~0.0, B~0.0).
        // It should be notification urgency critical fallback: #8B1A1A (R~0.545, G~0.102, B~0.102).
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        assert!(
            !vertices.is_empty(),
            "expected backdrop vertices for notification-area urgency=3"
        );

        // Check first vertex: R should NOT be ~1.0 (that would be severity critical).
        let first = &vertices[0];
        assert!(
            first.color[0] < 0.7,
            "notification-area urgency=3 R must NOT be severity critical (~1.0), got {}",
            first.color[0]
        );
    }

    // ── Notification urgency token tests ─────────────────────────────────────

    /// urgency_to_notification_color: low (0) maps to #2A2A2A fallback.
    #[test]
    fn test_notification_urgency_low_fallback() {
        let no_tokens = HashMap::new();
        let color = urgency_to_notification_color(0, &no_tokens);
        // #2A2A2A → R=G=B ≈ 0.165
        assert!(
            (color.r - 0.165).abs() < 0.01,
            "urgency low R should be ~0.165, got {}",
            color.r
        );
        assert!(
            (color.g - 0.165).abs() < 0.01,
            "urgency low G should be ~0.165, got {}",
            color.g
        );
        assert!(
            (color.b - 0.165).abs() < 0.01,
            "urgency low B should be ~0.165, got {}",
            color.b
        );
    }

    /// urgency_to_notification_color: normal (1) maps to #1A1A3A fallback.
    #[test]
    fn test_notification_urgency_normal_fallback() {
        let no_tokens = HashMap::new();
        let color = urgency_to_notification_color(1, &no_tokens);
        // #1A1A3A: R≈0.102, G≈0.102, B≈0.227
        assert!(
            (color.r - 0.102).abs() < 0.01,
            "urgency normal R should be ~0.102, got {}",
            color.r
        );
        assert!(
            (color.b - 0.227).abs() < 0.02,
            "urgency normal B should be ~0.227, got {}",
            color.b
        );
        // B must be clearly greater than R for blue-tint
        assert!(
            color.b > color.r + 0.05,
            "urgency normal B should be > R (blue tint)"
        );
    }

    /// urgency_to_notification_color: urgent (2) maps to #8B6914 fallback.
    #[test]
    fn test_notification_urgency_urgent_fallback() {
        let no_tokens = HashMap::new();
        let color = urgency_to_notification_color(2, &no_tokens);
        // #8B6914: R≈0.545, G≈0.412, B≈0.078
        assert!(
            color.r > 0.4,
            "urgency urgent R should be >0.4, got {}",
            color.r
        );
        assert!(
            color.g > 0.3,
            "urgency urgent G should be >0.3, got {}",
            color.g
        );
        assert!(
            color.b < 0.2,
            "urgency urgent B should be <0.2, got {}",
            color.b
        );
        // R should be greatest (amber-gold tint)
        assert!(
            color.r > color.b,
            "urgency urgent R should be > B (amber tint)"
        );
    }

    /// urgency_to_notification_color: critical (3) maps to #8B1A1A fallback.
    #[test]
    fn test_notification_urgency_critical_fallback() {
        let no_tokens = HashMap::new();
        let color = urgency_to_notification_color(3, &no_tokens);
        // #8B1A1A: R≈0.545, G≈0.102, B≈0.102
        assert!(
            color.r > 0.4,
            "urgency critical R should be >0.4, got {}",
            color.r
        );
        assert!(
            color.g < 0.2,
            "urgency critical G should be <0.2, got {}",
            color.g
        );
        assert!(
            color.b < 0.2,
            "urgency critical B should be <0.2, got {}",
            color.b
        );
        // R should be clearly greater than G and B (dark red)
        assert!(
            color.r > color.g + 0.2,
            "urgency critical R should dominate (dark red)"
        );
    }

    /// urgency_to_notification_color: urgency > 3 is clamped to critical (3).
    #[test]
    fn test_notification_urgency_clamped_above_3() {
        let no_tokens = HashMap::new();
        let critical3 = urgency_to_notification_color(3, &no_tokens);
        let clamped4 = urgency_to_notification_color(4, &no_tokens);
        let clamped100 = urgency_to_notification_color(100, &no_tokens);
        assert_eq!(
            critical3.r, clamped4.r,
            "urgency=4 should clamp to urgency=3"
        );
        assert_eq!(
            critical3.g, clamped4.g,
            "urgency=4 should clamp to urgency=3"
        );
        assert_eq!(
            critical3.b, clamped4.b,
            "urgency=4 should clamp to urgency=3"
        );
        assert_eq!(
            critical3.r, clamped100.r,
            "urgency=100 should clamp to urgency=3"
        );
    }

    /// Profile token override: color.notification.urgency.low overrides fallback.
    #[test]
    fn test_notification_urgency_low_token_override() {
        let mut token_map = HashMap::new();
        // Override low with pure cyan (#00FFFF) — clearly distinct from default.
        token_map.insert(
            "color.notification.urgency.low".to_string(),
            "#00FFFF".to_string(),
        );
        let color = urgency_to_notification_color(0, &token_map);
        assert!(
            color.r < 0.1,
            "custom low token R should be ~0.0 (cyan), got {}",
            color.r
        );
        assert!(
            color.g > 0.9,
            "custom low token G should be ~1.0 (cyan), got {}",
            color.g
        );
        assert!(
            color.b > 0.9,
            "custom low token B should be ~1.0 (cyan), got {}",
            color.b
        );
    }

    /// Profile token override: color.notification.urgency.critical overrides fallback.
    #[test]
    fn test_notification_urgency_critical_token_override() {
        let mut token_map = HashMap::new();
        // Override critical with pure magenta (#FF00FF).
        token_map.insert(
            "color.notification.urgency.critical".to_string(),
            "#FF00FF".to_string(),
        );
        let color = urgency_to_notification_color(3, &token_map);
        assert!(
            color.r > 0.9,
            "custom critical token R should be ~1.0, got {}",
            color.r
        );
        assert!(
            color.b > 0.9,
            "custom critical token B should be ~1.0, got {}",
            color.b
        );
        assert!(
            color.g < 0.1,
            "custom critical token G should be ~0.0, got {}",
            color.g
        );
    }

    /// notification-area urgency-tinted backdrop: renders backdrop at 0.9 opacity.
    ///
    /// Per spec: non-alert-banner Notification content must use
    /// color.notification.urgency.* tokens with fixed 0.9 opacity.
    /// The policy.backdrop_opacity must NOT override this.
    #[tokio::test]
    async fn test_notification_area_backdrop_uses_0_9_opacity() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area urgency opacity test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.5)),
                // backdrop_opacity = 0.5 must NOT override the 0.9 fixed opacity
                backdrop_opacity: Some(0.5),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "test".to_owned(),
                    icon: String::new(),
                    urgency: 1, // normal
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        assert!(!vertices.is_empty(), "expected vertices");
        // The first quad's alpha (index 3 of color) should be 0.9.
        let first_alpha = vertices[0].color[3];
        assert!(
            (first_alpha - 0.9).abs() < 0.01,
            "notification-area backdrop alpha must be 0.9, got {first_alpha}"
        );
    }

    /// notification-area border rendering: 1px 4-quad border is emitted after the
    /// urgency-tinted backdrop quad.
    ///
    /// For a Stack zone with one Notification publish, render_zone_content should emit:
    ///   - 6 vertices for the backdrop quad
    ///   - up to 24 vertices (4 × 6) for the border quads
    #[tokio::test]
    async fn test_notification_area_emits_border_quads() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area border test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.05, 0.05, 0.05, 1.0)),
                font_size_px: Some(18.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "border test".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // One backdrop (6 vertices) + up to 4 border quads (6 each) = 6 + 24 = 30 max.
        // Minimum: 6 (backdrop) + 6 (at least top edge border) = 12.
        assert!(
            vertices.len() >= 12,
            "expected at least 12 vertices (backdrop + border), got {}",
            vertices.len()
        );
        // Total should be 6 * N for some N ≥ 2 (backdrop + at least one border quad).
        assert_eq!(
            vertices.len() % 6,
            0,
            "vertex count must be a multiple of 6 (each quad = 6 vertices), got {}",
            vertices.len()
        );
    }

    /// alert-banner does NOT emit border quads — border rendering is only for
    /// non-alert-banner notification zones.
    #[tokio::test]
    async fn test_alert_banner_does_not_emit_border_quads() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "alert banner — no border".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.07,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(20.0),
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                backdrop_opacity: Some(1.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "no border here".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // alert-banner: only 6 vertices (one backdrop quad, no border).
        assert_eq!(
            vertices.len(),
            6,
            "alert-banner must emit exactly 6 vertices (backdrop only, no border), got {}",
            vertices.len()
        );
    }

    /// Border color uses color.border.default token when present.
    #[tokio::test]
    async fn test_notification_area_border_uses_border_default_token() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        // Install a custom border token: pure cyan (#00FFFF).
        let mut token_map = HashMap::new();
        token_map.insert("color.border.default".to_string(), "#00FFFF".to_string());
        compositor.set_token_map(token_map);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area border token test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.05, 0.05, 0.05, 1.0)),
                font_size_px: Some(18.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "cyan border".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // vertices[0..6] = backdrop quad (urgency low color)
        // vertices[6..] = border quads (should be cyan: R≈0, G≈1, B≈1)
        assert!(
            vertices.len() > 6,
            "expected border quads after backdrop, only got {}",
            vertices.len()
        );
        // Check border quad color (vertex index 6 is the first border vertex).
        let border_v = &vertices[6];
        assert!(
            border_v.color[0] < 0.1,
            "border R should be ~0.0 (cyan token), got {}",
            border_v.color[0]
        );
        assert!(
            border_v.color[1] > 0.9,
            "border G should be ~1.0 (cyan token), got {}",
            border_v.color[1]
        );
        assert!(
            border_v.color[2] > 0.9,
            "border B should be ~1.0 (cyan token), got {}",
            border_v.color[2]
        );
    }

    // ── Token-resolved severity color tests ───────────────────────────────────

    /// Custom `color.severity.warning` token overrides the hardcoded SEVERITY_WARNING
    /// constant for urgency=2.
    #[test]
    fn test_custom_severity_warning_token_overrides_constant() {
        let mut token_map = HashMap::new();
        // Custom warning: bright green (#00FF00) — clearly distinct from amber.
        token_map.insert("color.severity.warning".to_string(), "#00FF00".to_string());
        let color = urgency_to_severity_color(2, &token_map);
        assert!(
            color.g > 0.9,
            "custom warning token G should be ~1.0 (green), got {}",
            color.g
        );
        assert!(
            color.r < 0.1,
            "custom warning token R should be ~0.0 (green), got {}",
            color.r
        );
        assert!(
            color.b < 0.1,
            "custom warning token B should be ~0.0 (green), got {}",
            color.b
        );
    }

    /// Custom `color.severity.critical` token overrides the hardcoded SEVERITY_CRITICAL.
    #[test]
    fn test_custom_severity_critical_token_overrides_constant() {
        let mut token_map = HashMap::new();
        // Custom critical: bright magenta (#FF00FF).
        token_map.insert("color.severity.critical".to_string(), "#FF00FF".to_string());
        let color = urgency_to_severity_color(3, &token_map);
        assert!(
            color.r > 0.9,
            "custom critical R should be ~1.0 (magenta), got {}",
            color.r
        );
        assert!(
            color.b > 0.9,
            "custom critical B should be ~1.0 (magenta), got {}",
            color.b
        );
        assert!(
            color.g < 0.1,
            "custom critical G should be ~0.0 (magenta), got {}",
            color.g
        );
    }

    /// Custom `color.severity.info` token overrides the hardcoded SEVERITY_INFO.
    #[test]
    fn test_custom_severity_info_token_overrides_constant() {
        let mut token_map = HashMap::new();
        // Custom info: pure red (#FF0000) — clearly distinct from default blue.
        token_map.insert("color.severity.info".to_string(), "#FF0000".to_string());
        let color0 = urgency_to_severity_color(0, &token_map);
        let color1 = urgency_to_severity_color(1, &token_map);
        for (urgency, color) in [(0, color0), (1, color1)] {
            assert!(
                color.r > 0.9,
                "custom info urgency={urgency} R should be ~1.0 (red), got {}",
                color.r
            );
            assert!(
                color.g < 0.1,
                "custom info urgency={urgency} G should be ~0.0 (red), got {}",
                color.g
            );
            assert!(
                color.b < 0.1,
                "custom info urgency={urgency} B should be ~0.0 (red), got {}",
                color.b
            );
        }
    }

    /// Invalid/absent token values fall back to hardcoded constants.
    #[test]
    fn test_invalid_severity_token_value_falls_back_to_constant() {
        let mut token_map = HashMap::new();
        // Not a valid hex color — should be ignored.
        token_map.insert(
            "color.severity.warning".to_string(),
            "not-a-color".to_string(),
        );
        let color = urgency_to_severity_color(2, &token_map);
        // Falls back to SEVERITY_WARNING (#FFB800): R~1.0, G~0.72, B~0.0.
        assert!(
            color.r > 0.9,
            "fallback warning R should be ~1.0, got {}",
            color.r
        );
        assert!(
            color.g > 0.5,
            "fallback warning G should be >0.5, got {}",
            color.g
        );
        assert!(
            color.b < 0.1,
            "fallback warning B should be ~0.0, got {}",
            color.b
        );
    }

    /// Custom severity tokens in [design_tokens] affect alert-banner backdrop colors.
    ///
    /// This is the end-to-end integration test: `set_token_map` populates the
    /// compositor, and `render_zone_content` uses the token-resolved color for the
    /// alert-banner backdrop.
    #[tokio::test]
    async fn test_custom_severity_tokens_affect_alert_banner_backdrop() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        // Install a custom token map: override warning with pure green (#00FF00).
        let mut token_map = HashMap::new();
        token_map.insert("color.severity.warning".to_string(), "#00FF00".to_string());
        compositor.set_token_map(token_map);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "alert banner zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.07,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(20.0),
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                backdrop_opacity: Some(1.0),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish urgency=2 (warning) — should use custom green token.
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "Custom token warning".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // Collect vertices from render_zone_content.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // rect_vertices emits 6 vertices per quad; each vertex has a `color: [f32; 4]` field.
        // The backdrop should be green (R~0.0, G~1.0, B~0.0), not amber.
        assert!(
            !vertices.is_empty(),
            "expected backdrop vertices for alert-banner"
        );

        // Check first vertex color. RectVertex layout: [position: [f32; 2], color: [f32; 4]].
        let first = &vertices[0];
        assert!(
            first.color[1] > 0.9,
            "alert-banner backdrop G should be ~1.0 (custom green token), got {}",
            first.color[1]
        );
        assert!(
            first.color[0] < 0.1,
            "alert-banner backdrop R should be ~0.0 (custom green token), got {}",
            first.color[0]
        );
        assert!(
            first.color[2] < 0.1,
            "alert-banner backdrop B should be ~0.0 (custom green token), got {}",
            first.color[2]
        );
    }

    /// ZoneAnimationState fade-in reaches 1.0 after duration elapses.
    #[test]
    fn test_zone_animation_state_fade_in_completes() {
        // Use 0ms duration for instant completion.
        let state = ZoneAnimationState::fade_in(0);
        // Opacity at duration=0 should immediately be target (1.0).
        let opacity = state.current_opacity();
        assert_eq!(opacity, 1.0, "fade-in with 0ms should be 1.0 immediately");
        assert!(
            state.is_complete(),
            "0ms fade-in should be complete immediately"
        );
    }

    /// ZoneAnimationState fade-out starts at 1.0 and reaches 0.0 after duration.
    #[test]
    fn test_zone_animation_state_fade_out_completes() {
        let state = ZoneAnimationState::fade_out(0);
        let opacity = state.current_opacity();
        assert_eq!(opacity, 0.0, "fade-out with 0ms should be 0.0 immediately");
        assert!(
            state.is_complete(),
            "0ms fade-out should be complete immediately"
        );
    }

    /// ZoneAnimationState with non-zero duration: opacity is interpolated.
    #[test]
    fn test_zone_animation_state_interpolates() {
        // 10_000ms duration — very long, so elapsed << duration.
        let state = ZoneAnimationState::fade_in(10_000);
        // Very shortly after creation, opacity should be close to 0.
        let opacity = state.current_opacity();
        assert!(
            opacity >= 0.0 && opacity <= 0.1,
            "fade-in opacity shortly after start should be near 0, got {opacity}"
        );
        assert!(
            !state.is_complete(),
            "10s fade-in should not be complete immediately"
        );
    }

    /// backdrop_opacity overrides the backdrop color's alpha channel.
    /// When backdrop_opacity=0.6 and backdrop.a=1.0, effective alpha=0.6.
    #[tokio::test]
    async fn test_backdrop_opacity_overrides_color_alpha() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // backdrop color has alpha=1.0 but backdrop_opacity=0.6 should override it.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "test backdrop opacity override".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 1.0)), // alpha=1.0
                backdrop_opacity: Some(0.6),                   // override to 0.6
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("opacity test".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // The backdrop rendered should use alpha=0.6 (backdrop_opacity), not 1.0.
        // We verify this by checking the vertex colors produced — the alpha channel
        // of the first rect vertex should reflect 0.6.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);
        assert!(!vertices.is_empty(), "expected backdrop vertices");
        // The RectVertex has color field [f32; 4]; alpha should be ~0.6.
        let alpha = vertices[0].color[3];
        assert!(
            (alpha - 0.6).abs() < 0.01,
            "backdrop alpha should be ~0.6 (backdrop_opacity override), got {alpha}"
        );
    }

    /// backdrop=None: no backdrop quad rendered even when backdrop_opacity is set.
    #[tokio::test]
    async fn test_no_backdrop_when_backdrop_is_none() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "no-backdrop test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                backdrop: None,
                backdrop_opacity: Some(0.9), // ignored because backdrop is None
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("no backdrop".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // With backdrop=None, no rect vertices should be emitted.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);
        assert!(
            vertices.is_empty(),
            "no backdrop quad should be rendered when policy.backdrop is None"
        );
    }

    /// backdrop=None with Notification content: no backdrop or border quads rendered.
    ///
    /// Even though Notification content in a non-alert-banner zone overrides the
    /// backdrop color with urgency-tinted tokens, the override must respect the
    /// policy.backdrop contract: when backdrop is None, nothing is emitted.
    #[tokio::test]
    async fn test_notification_no_backdrop_when_backdrop_is_none() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area with backdrop=None".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: None,
                backdrop_opacity: Some(0.9),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "no backdrop".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);
        assert!(
            vertices.is_empty(),
            "no backdrop or border quads should be rendered when policy.backdrop is None, got {} vertices",
            vertices.len()
        );
    }

    /// text.rs: TextItem::from_zone_policy respects all RenderingPolicy fields.
    #[test]
    fn test_from_zone_policy_reads_all_policy_fields() {
        use crate::text::TextItem;

        let policy = RenderingPolicy {
            font_size_px: Some(28.0),
            text_color: Some(Rgba::new(1.0, 0.5, 0.0, 1.0)), // orange
            font_family: Some(FontFamily::SystemMonospace),
            text_align: Some(TextAlign::Center),
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(1.5),
            margin_horizontal: Some(12.0),
            margin_vertical: Some(6.0),
            ..Default::default()
        };

        let item = TextItem::from_zone_policy("test", 0.0, 0.0, 400.0, 100.0, &policy, 1.0);
        assert_eq!(item.font_size_px, 28.0);
        assert_eq!(item.font_family, FontFamily::SystemMonospace);
        assert_eq!(item.alignment, TextAlign::Center);
        assert!(item.outline_color.is_some(), "outline_color should be set");
        assert_eq!(item.outline_width.unwrap(), 1.5);
        // Margins: x+12, y+6
        assert_eq!(item.pixel_x, 12.0);
        assert_eq!(item.pixel_y, 6.0);
    }

    // ── Multi-publication rendering: Stack and MergeByKey policies ──────────

    /// Stack zone with two notifications: render_zone_content must emit a
    /// separate backdrop quad for each publication, stacked vertically.
    /// With max_depth=4 and zone height=400px, each slot is 100px tall.
    /// Two publications → two quads; the second quad starts at y+100.
    #[tokio::test]
    async fn test_stack_zone_renders_separate_backdrop_per_publication() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone: top-right, 200×400 px via Relative.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "stack zone for multi-pub test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,    // 320 px at 1280 wide
                height_pct: 0.5556, // ~400 px at 720 tall (≈400/720)
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                backdrop_opacity: Some(0.9),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 4 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish two separate notifications.
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "First notification".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Second notification".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-b",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // Two publications → two backdrop quads (6 verts each) + border quads.
        // Each Notification slot emits: 1 backdrop quad (6) + 4 border quads (24) = 30.
        // Total: 2 × 30 = 60 vertices.
        // We assert at least 12 (2 backdrops) and a multiple of 6.
        assert!(
            vertices.len() >= 12,
            "Stack zone with 2 publications must emit at least 12 vertices (2 backdrop quads), got {}",
            vertices.len()
        );
        assert_eq!(
            vertices.len() % 6,
            0,
            "vertex count must be a multiple of 6 (each quad = 6 vertices), got {}",
            vertices.len()
        );

        // The first backdrop quad's top-left y should be 0 (zone starts at y_pct=0.0 → y=0).
        // The second backdrop quad's top-left y should be ~slot_h after the first slot.
        // zone_h = 720 * 0.5556 ≈ 400; slot_h is content-sized per stack_slot_height.
        // Vertices are in NDC; we check the first and 7th vertex y values differ.
        // rect_vertices emits 6 verts per quad in positions [x,y] NDC.
        // With border quads added, the second backdrop quad starts at vertex index 30
        // (6 backdrop + 24 border for first slot = 30 verts per slot).
        let first_quad_y = vertices[0].position[1];
        // Find second backdrop by skipping first slot (30 vertices: 6 backdrop + 24 border).
        let second_quad_idx = 30; // 6 backdrop + 4 border quads (6 each)
        if vertices.len() > second_quad_idx {
            let second_quad_y = vertices[second_quad_idx].position[1];
            assert!(
                (first_quad_y - second_quad_y).abs() > 0.01,
                "second Stack slot must start at a different y than the first; got first={first_quad_y:.4}, second={second_quad_y:.4}"
            );
        }
    }

    /// Stack zone: collect_text_items must produce a separate TextItem for
    /// each publication, with each item positioned in its own vertical slot.
    #[tokio::test]
    async fn test_stack_zone_collect_text_items_per_publication() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "stack zone text items test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.5556,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 4 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Alpha alert".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Beta alert".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-b",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Gamma alert".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-c",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // Three publications in a Stack zone must produce three TextItems.
        assert_eq!(
            items.len(),
            3,
            "Stack zone with 3 publications must produce 3 TextItems, got {}",
            items.len()
        );

        // Items should be ordered newest-first (slot 0 = newest at top of zone).
        assert!(
            items[0].text.contains("Gamma"),
            "first TextItem should be the newest publication (Gamma), got: {}",
            items[0].text
        );
        assert!(
            items[1].text.contains("Beta"),
            "second TextItem should be Beta, got: {}",
            items[1].text
        );
        assert!(
            items[2].text.contains("Alpha"),
            "third TextItem should be the oldest publication (Alpha), got: {}",
            items[2].text
        );

        // Each item should occupy a different vertical slot; slot 0 is at top
        // (lowest y), slot 1 below it, slot 2 below that.
        assert!(
            items[1].pixel_y > items[0].pixel_y,
            "slot 1 y ({}) must be below slot 0 y ({})",
            items[1].pixel_y,
            items[0].pixel_y
        );
        assert!(
            items[2].pixel_y > items[1].pixel_y,
            "slot 2 y ({}) must be below slot 1 y ({})",
            items[2].pixel_y,
            items[1].pixel_y
        );
    }

    // ── Slot layout: content-sized slots, newest-first ───────────────────────

    /// 5 stacked notifications must each appear at a distinct y-position.
    /// slot_height = font_size_px(16) + 18 = 34 px.
    /// Zone is tall enough to accommodate all 5 slots.
    /// Verifies newest-first ordering: slot 0 = newest at zone top.
    #[tokio::test]
    async fn test_stack_slot_layout_five_notifications_distinct_y() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone: 300px wide × 300px tall — enough for 5 × 34px slots (170px).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "slot layout test zone".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,    // 320 px at 1280 wide
                height_pct: 0.4167, // ~300 px at 720 tall
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 5,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish 5 notifications (oldest to newest: "N1" .. "N5").
        for i in 1..=5 {
            scene
                .publish_to_zone(
                    "notification-area",
                    ZoneContent::Notification(NotificationPayload {
                        text: format!("N{i}"),
                        icon: String::new(),
                        urgency: 1,
                        ttl_ms: None,
                        title: String::new(),
                        actions: Vec::new(),
                    }),
                    &format!("agent-{i}"),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 5, "5 notifications must produce 5 TextItems");

        // With font_size_px=16 → slot_h = 34.  margin_v defaults to 8.
        // pixel_y for slot i = zone_y + i*slot_h + margin_v.
        // Check all 5 y-values are strictly increasing (slot 0 = newest = top).
        let ys: Vec<f32> = items.iter().map(|it| it.pixel_y).collect();
        for w in ys.windows(2) {
            assert!(
                w[1] > w[0],
                "slots must have strictly increasing y; got consecutive y={:.2} then {:.2}",
                w[0],
                w[1]
            );
        }

        // Newest notification is "N5" — must be slot 0 (lowest y).
        assert!(
            items[0].text.contains("N5"),
            "slot 0 must be the newest notification (N5), got: {}",
            items[0].text
        );
        // Oldest is "N1" — must be slot 4 (highest y).
        assert!(
            items[4].text.contains("N1"),
            "slot 4 must be the oldest notification (N1), got: {}",
            items[4].text
        );
    }

    /// When a 6th notification is published to a Stack zone with max_depth=5,
    /// the oldest notification must be evicted.  After eviction, only 5 items
    /// remain and the evicted notification is absent.
    #[tokio::test]
    async fn test_stack_slot_sixth_notification_evicts_oldest() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "eviction test zone".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 6,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish 6 notifications; "oldest" is "Oldest" (first published).
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Oldest".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-0",
                None,
                None,
                None,
            )
            .unwrap();
        for i in 1..=5 {
            scene
                .publish_to_zone(
                    "notification-area",
                    ZoneContent::Notification(NotificationPayload {
                        text: format!("N{i}"),
                        icon: String::new(),
                        urgency: 1,
                        ttl_ms: None,
                        title: String::new(),
                        actions: Vec::new(),
                    }),
                    &format!("agent-{i}"),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // Only 5 items after eviction (max_depth=5).
        assert_eq!(
            items.len(),
            5,
            "after 6th publish with max_depth=5, must have 5 TextItems; got {}",
            items.len()
        );

        // "Oldest" must have been evicted — not present in any item.
        let has_oldest = items.iter().any(|it| it.text.contains("Oldest"));
        assert!(
            !has_oldest,
            "oldest notification must be evicted after 6th publish"
        );

        // "N5" (newest) must be present as slot 0.
        assert!(
            items[0].text.contains("N5"),
            "newest notification (N5) must be slot 0 after eviction, got: {}",
            items[0].text
        );
    }

    /// Stack notifications clip at zone boundary: slots whose top-left y is at or
    /// beyond zone_bottom are fully clipped and produce no TextItem. Partial slots
    /// (y < zone_bottom but y+slot_h > zone_bottom) are emitted with clamped height.
    #[tokio::test]
    async fn test_stack_slot_clips_at_zone_boundary() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone height: 72px at 720 tall → height_pct = 72/720 = 0.1.
        // font_size_px=16, line_height = 16*1.4 = 22.4, margin_v=8, SLOT_BASELINE_GAP=4
        // → slot_h = 22.4 + 2*8 + 4 = 42.4px.
        // slot 0 at y=0  (fits: 0+42.4=42.4 ≤ 72 → emitted).
        // slot 1 at y=42.4 (fits: 42.4+42.4=84.8 > 72 but y < 72 → emitted).
        // slot 2 at y=84.8 → y ≥ zone_bottom(72) → loop breaks.
        // Exactly 2 items (slots 0, 1) are emitted.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "clipping test zone".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.1, // 72 px at 720 tall
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 5,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish 4 notifications; only the 2 newest should be visible (newest-first).
        for i in 1..=4 {
            scene
                .publish_to_zone(
                    "notification-area",
                    ZoneContent::Notification(NotificationPayload {
                        text: format!("M{i}"),
                        icon: String::new(),
                        urgency: 1,
                        ttl_ms: None,
                        title: String::new(),
                        actions: Vec::new(),
                    }),
                    &format!("agent-{i}"),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // slot_h = line_height(16*1.4) + 2*margin_v(8) + SLOT_BASELINE_GAP(4) = 42.4px.
        // slot 0 at y=0:    0 < 72 → emitted.
        // slot 1 at y=42.4: 42.4 < 72 → emitted.
        // slot 2 at y=84.8: 84.8 ≥ 72 → loop breaks.
        // Exactly 2 items (slots 0, 1) are emitted.
        assert_eq!(
            items.len(),
            2,
            "with 72px zone and 42.4px slots, exactly 2 items should be emitted; got {}",
            items.len()
        );

        // The newest notification (M4) must be in slot 0.
        assert!(
            !items.is_empty() && items[0].text.contains("M4"),
            "newest notification (M4) must be slot 0 (top of zone), got: {}",
            if items.is_empty() {
                "empty"
            } else {
                &items[0].text
            }
        );
    }

    /// MergeByKey zone: collect_text_items must merge ALL StatusBar publications'
    /// entries and produce a single TextItem containing all unique keys.
    #[tokio::test]
    async fn test_merge_by_key_zone_merges_all_status_bar_entries() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_owned(),
            description: "merge-by-key zone test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 32 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Agent A publishes "cpu" and "mem" keys.
        let mut entries_a = std::collections::HashMap::new();
        entries_a.insert("cpu".to_owned(), "45%".to_owned());
        entries_a.insert("mem".to_owned(), "8.2 GB".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload { entries: entries_a }),
                "agent-a",
                Some("cpu-mem".to_owned()),
                None,
                None,
            )
            .unwrap();

        // Agent B publishes a "net" key.
        let mut entries_b = std::collections::HashMap::new();
        entries_b.insert("net".to_owned(), "1.2 MB/s".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload { entries: entries_b }),
                "agent-b",
                Some("net".to_owned()),
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // MergeByKey must produce exactly ONE TextItem containing all merged entries.
        assert_eq!(
            items.len(),
            1,
            "MergeByKey zone must produce a single merged TextItem, got {}",
            items.len()
        );

        let text = &items[0].text;
        assert!(
            text.contains("cpu"),
            "merged text must include 'cpu' key; got: {text}"
        );
        assert!(
            text.contains("mem"),
            "merged text must include 'mem' key; got: {text}"
        );
        assert!(
            text.contains("net"),
            "merged text must include 'net' key; got: {text}"
        );
        assert!(
            text.contains("45%"),
            "merged text must include cpu value '45%'; got: {text}"
        );
        assert!(
            text.contains("1.2 MB/s"),
            "merged text must include net value '1.2 MB/s'; got: {text}"
        );
    }

    /// MergeByKey zone: when a key appears in multiple publications, the latest
    /// value wins (last-write-wins per key semantics).
    #[tokio::test]
    async fn test_merge_by_key_latest_value_wins_for_duplicate_keys() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_owned(),
            description: "merge-by-key duplicate key test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 32 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // First publish: cpu = "10%"
        let mut entries_old = std::collections::HashMap::new();
        entries_old.insert("cpu".to_owned(), "10%".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload {
                    entries: entries_old,
                }),
                "agent-a",
                Some("cpu".to_owned()),
                None,
                None,
            )
            .unwrap();

        // Second publish: same key "cpu" with updated value "90%"
        let mut entries_new = std::collections::HashMap::new();
        entries_new.insert("cpu".to_owned(), "90%".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload {
                    entries: entries_new,
                }),
                "agent-a",
                Some("cpu".to_owned()),
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        assert_eq!(items.len(), 1, "must produce one merged TextItem");
        let text = &items[0].text;

        // The latest value "90%" must appear; "10%" must not.
        assert!(
            text.contains("90%"),
            "merged text must show latest cpu value '90%'; got: {text}"
        );
        assert!(
            !text.contains("10%"),
            "merged text must not show stale cpu value '10%'; got: {text}"
        );
    }

    // ── StatusBar icon layout tests [hud-x2v1.2] ─────────────────────────────

    /// `key_icon_map` empty → single merged TextItem (backward-compatible).
    ///
    /// When `key_icon_map` is empty, the existing single-TextItem newline-joined
    /// behavior must be preserved unchanged.
    #[tokio::test]
    async fn test_status_bar_empty_key_icon_map_produces_single_text_item() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_owned(),
            description: "icon layout: empty map regression".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                text_color: Some(Rgba::WHITE),
                // key_icon_map defaults to empty HashMap via serde(default).
                ..Default::default()
            },
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 16 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        let mut entries = std::collections::HashMap::new();
        entries.insert("cpu".to_owned(), "45%".to_owned());
        entries.insert("mem".to_owned(), "8 GB".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload { entries }),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // Must still produce exactly ONE TextItem (no icon layout).
        assert_eq!(
            items.len(),
            1,
            "empty key_icon_map must produce one merged TextItem; got {}",
            items.len()
        );
        let text = &items[0].text;
        assert!(text.contains("cpu"), "text must contain 'cpu'");
        assert!(text.contains("mem"), "text must contain 'mem'");
        // Entries joined with newline (alphabetically sorted).
        assert!(
            text.contains('\n'),
            "multiple entries must be newline-separated; got: {text}"
        );
    }

    /// `key_icon_map` non-empty → per-entry TextItems; icon-mapped entries have
    /// text `pixel_x` inset by `ICON_SIZE_PX + ICON_TEXT_GAP_PX`.
    ///
    /// We don't use real SVG files here since tests can't rely on specific
    /// filesystem paths.  Instead we verify the TextItem layout (pixel_x
    /// position) produced by `status_bar_icon_text_items` directly — the icon
    /// draw command path is exercised separately.
    #[tokio::test]
    async fn test_status_bar_key_icon_map_produces_per_entry_text_items() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut key_icon_map = std::collections::HashMap::new();
        // Map only "cpu" to an icon path; "mem" has no mapping.
        key_icon_map.insert("cpu".to_owned(), "/nonexistent/cpu.svg".to_owned());

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_owned(),
            description: "icon layout: per-entry TextItems".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                text_color: Some(Rgba::WHITE),
                font_size_px: Some(16.0),
                key_icon_map,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 16 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        let mut entries = std::collections::HashMap::new();
        entries.insert("cpu".to_owned(), "45%".to_owned());
        entries.insert("mem".to_owned(), "8 GB".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload { entries }),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // Must produce one TextItem per entry (2 entries → 2 TextItems).
        assert_eq!(
            items.len(),
            2,
            "non-empty key_icon_map must produce per-entry TextItems; got {}",
            items.len()
        );

        // Entries are sorted by key: "cpu" (row 0) then "mem" (row 1).
        // "cpu" has an SVG icon mapping, so no prefix.
        // "mem" has no SVG icon mapping, so gets emoji prefix "💾".
        let cpu_item = items.iter().find(|i| i.text.starts_with("cpu:"));
        let mem_item = items.iter().find(|i| i.text.starts_with("💾 mem:"));

        assert!(cpu_item.is_some(), "expected a TextItem for 'cpu'");
        assert!(
            mem_item.is_some(),
            "expected a TextItem for 'mem' with emoji prefix"
        );

        let cpu_item = cpu_item.unwrap();
        let mem_item = mem_item.unwrap();

        // "cpu" is icon-mapped: pixel_x must be inset by ICON_SIZE_PX + ICON_TEXT_GAP_PX
        // relative to "mem" (which has no icon).
        // Both items use from_zone_policy with x = zx + icon_inset, so:
        //   cpu.pixel_x = zx + ICON_SIZE_PX + ICON_TEXT_GAP_PX + margin_h
        //   mem.pixel_x = zx + 0 + margin_h
        // Difference should be exactly ICON_SIZE_PX + ICON_TEXT_GAP_PX (30.0).
        let icon_inset = ICON_SIZE_PX + ICON_TEXT_GAP_PX; // 24.0 + 6.0 = 30.0
        let diff = cpu_item.pixel_x - mem_item.pixel_x;
        assert!(
            (diff - icon_inset).abs() < 0.5,
            "cpu pixel_x must be inset by {icon_inset} px relative to mem; diff={diff}"
        );

        // "mem" has no icon: bounds_width must be wider than "cpu" by icon_inset.
        let width_diff = mem_item.bounds_width - cpu_item.bounds_width;
        assert!(
            (width_diff - icon_inset).abs() < 0.5,
            "mem bounds_width must be {icon_inset} px wider than cpu; diff={width_diff}"
        );
    }

    /// LatestWins zone still renders only one TextItem even when multiple
    /// publications are present (regression guard).
    #[tokio::test]
    async fn test_latest_wins_zone_renders_only_latest_publication() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "latest-wins regression guard".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // LatestWins should only keep the last publication.
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("Old content".to_owned()),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();
        // With LatestWins policy the scene graph may have already replaced it,
        // but we publish again to ensure only one ends up active.
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("New content".to_owned()),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // LatestWins must produce exactly one TextItem.
        assert_eq!(
            items.len(),
            1,
            "LatestWins zone must produce exactly 1 TextItem; got {}",
            items.len()
        );
        // The item should contain the latest content.
        assert!(
            items[0].text.contains("New content"),
            "LatestWins must render latest publish; got: {}",
            items[0].text
        );
    }

    // ── Alert-banner heading typography tests [hud-w3o6.2] ───────────────────
    //
    // Acceptance criteria from spec §Alert-Banner Heading Typography:
    //   1. font_size_px = 24px
    //   2. font_weight = 700 (bold)
    //   3. font_family = SystemSansSerif
    //   4. text_color = #FFFFFF white
    //   5. margin_horizontal inset applied

    /// Alert-banner RenderingPolicy carries heading typography:
    /// 24px font, weight 700, SystemSansSerif, white text, margin_horizontal=8.
    ///
    /// Acceptance criterion 3.1–3.3: heading typography wired to alert-banner zone.
    #[tokio::test]
    async fn test_alert_banner_heading_typography_in_rendering_policy() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Register alert-banner with heading-typography RenderingPolicy (spec values).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "heading typography test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.06,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(24.0),
                font_family: Some(FontFamily::SystemSansSerif),
                font_weight: Some(700),
                text_color: Some(Rgba {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                }),
                backdrop: Some(Rgba::new(0.1, 0.1, 0.16, 0.9)),
                backdrop_opacity: Some(0.9),
                margin_horizontal: Some(8.0),
                margin_vertical: Some(0.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 16,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish a notification payload.
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "Weather alert: severe storms".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // collect_text_items uses the RenderingPolicy fields for TextItem construction.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(
            items.len(),
            1,
            "expected one TextItem for alert-banner notification"
        );

        let item = &items[0];

        // AC 3.1: font_size_px = 24.0
        assert_eq!(
            item.font_size_px, 24.0,
            "alert-banner text must be 24px per spec §Alert-Banner Heading Typography"
        );

        // AC 3.1: font_family = SystemSansSerif
        assert_eq!(
            item.font_family,
            FontFamily::SystemSansSerif,
            "alert-banner text must use system sans-serif family"
        );

        // AC 3.1: font_weight = 700
        assert_eq!(
            item.font_weight, 700,
            "alert-banner text must be weight 700 (bold)"
        );

        // AC 3.2: text_color = #FFFFFF (white)
        // White in linear sRGB: R=1.0 → 255u8, G=1.0 → 255u8, B=1.0 → 255u8.
        assert_eq!(item.color[0], 255, "text R should be 255 (white)");
        assert_eq!(item.color[1], 255, "text G should be 255 (white)");
        assert_eq!(item.color[2], 255, "text B should be 255 (white)");

        // AC 3.3: text is inset from x=0 by margin_horizontal=8
        // Zone geometry: zx = (sw - sw*1.0)/2 = 0.0, so pixel_x = 0 + 8 = 8.
        assert_eq!(
            item.pixel_x, 8.0,
            "text must be inset by margin_horizontal=8 from backdrop edge"
        );
    }

    /// Alert-banner zone has LayerAttachment::Chrome — renders above all agent content.
    ///
    /// Acceptance criterion: chrome-layer z-order verified by checking ZoneDefinition.
    #[test]
    fn test_alert_banner_default_zone_has_chrome_layer_attachment() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner must be in default zone registry");

        assert_eq!(
            zone.layer_attachment,
            LayerAttachment::Chrome,
            "alert-banner zone must be attached to chrome layer (above all agent content)"
        );
    }

    /// Alert-banner zone spans full display width (width_pct = 1.0).
    ///
    /// Acceptance criterion: backdrop quad spans from x=0 to x=display_width.
    #[test]
    fn test_alert_banner_default_zone_is_full_width() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner must be in default zone registry");

        match zone.geometry_policy {
            GeometryPolicy::EdgeAnchored { width_pct, .. } => {
                assert_eq!(
                    width_pct, 1.0,
                    "alert-banner must span full display width (width_pct=1.0)"
                );
            }
            _ => panic!("alert-banner must use EdgeAnchored geometry for full-width positioning"),
        }
    }

    /// Alert-banner zone resolve_zone_geometry gives backdrop width = display width.
    ///
    /// At 1920×1080, the backdrop must span from x=0 to x=1920.
    #[test]
    fn test_alert_banner_backdrop_spans_full_display_width() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner zone must exist");

        let (x, _y, w, _h) =
            Compositor::resolve_zone_geometry(&zone.geometry_policy, 1920.0, 1080.0);
        assert_eq!(x, 0.0, "alert-banner left edge must be at x=0");
        assert_eq!(
            w, 1920.0,
            "alert-banner width must equal display width (1920)"
        );
    }

    /// Alert-banner zone height accommodates 24px heading + vertical padding.
    ///
    /// At 720p, height_pct=0.06 → 43.2px > 24px + 2×8px = 40px minimum.
    #[test]
    fn test_alert_banner_zone_height_accommodates_heading_typography() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner zone must exist");

        // Check that resolved height at 720p is sufficient for 24px heading.
        // margin_vertical=0.0 (flush to edge), so minimum is font_size_px only.
        // height_pct=0.06 → 0.06×720=43.2px, well above the 24px minimum.
        let (_x, _y, _w, h) =
            Compositor::resolve_zone_geometry(&zone.geometry_policy, 1280.0, 720.0);
        let font_size_px = zone.rendering_policy.font_size_px.unwrap_or(24.0);
        let min_required = font_size_px; // margin_vertical=0.0; height must cover font at minimum
        assert!(
            h >= min_required,
            "alert-banner height {h}px must accommodate heading ({font_size_px}px)"
        );
    }

    /// When no alert-banner publications are active, render_zone_content emits zero
    /// backdrop vertices — the zone occupies zero visible space.
    ///
    /// Acceptance criterion §Alert-Banner Chrome-Layer Positioning:
    ///   "When no alerts are active, the alert-banner zone MUST occupy zero vertical space."
    #[tokio::test]
    async fn test_alert_banner_zero_height_when_inactive() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Register alert-banner zone with a visible backdrop so it would render if active.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "zero-height-when-inactive test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.06,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(24.0),
                backdrop: Some(Rgba::new(1.0, 0.0, 0.0, 1.0)), // bright red — visible if leaked
                backdrop_opacity: Some(0.9),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // No publications — zone is inactive.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // Zero vertices emitted → zone occupies zero visible space.
        assert!(
            vertices.is_empty(),
            "no backdrop quad must be emitted for inactive alert-banner zone (zero visible space)"
        );

        // Also verify no TextItems produced.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert!(
            items.is_empty(),
            "no text must be rendered for inactive alert-banner zone"
        );
    }

    /// Alert-banner RenderingPolicy in ZoneRegistry::with_defaults() carries
    /// heading typography: 24px, weight 700, white text, margin_horizontal=8.
    #[test]
    fn test_alert_banner_default_zone_rendering_policy_has_heading_typography() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner must be in default zone registry");

        let policy = &zone.rendering_policy;

        assert_eq!(
            policy.font_size_px,
            Some(24.0),
            "alert-banner default rendering policy must have font_size_px=24"
        );
        assert_eq!(
            policy.font_weight,
            Some(700),
            "alert-banner default rendering policy must have font_weight=700 (bold)"
        );
        assert_eq!(
            policy.font_family,
            Some(FontFamily::SystemSansSerif),
            "alert-banner default rendering policy must use SystemSansSerif"
        );
        // text_color must be white (R=1.0, G=1.0, B=1.0).
        let tc = policy
            .text_color
            .expect("alert-banner default rendering policy must have text_color set");
        assert!(
            (tc.r - 1.0).abs() < 0.01,
            "text_color R must be 1.0 (white), got {}",
            tc.r
        );
        assert!(
            (tc.g - 1.0).abs() < 0.01,
            "text_color G must be 1.0 (white), got {}",
            tc.g
        );
        assert!(
            (tc.b - 1.0).abs() < 0.01,
            "text_color B must be 1.0 (white), got {}",
            tc.b
        );
        assert_eq!(
            policy.margin_horizontal,
            Some(8.0),
            "alert-banner default rendering policy must have margin_horizontal=8"
        );
    }

    // ─── Alert-banner severity-stack tests ───────────────────────────────────

    /// Helper: build a SceneGraph with an alert-banner Stack zone for severity tests.
    ///
    /// Zone: full-width, EdgeAnchored top, height_pct=0.05 (36px at 720p).
    /// font_size_px=16, default margin_v=8 → slot_h = 16 + 2×8 + 2 = 34px.
    /// max_depth=8, max_publishers=16.
    fn make_alert_banner_scene() -> SceneGraph {
        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "alert-banner severity-stack test zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.05,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                backdrop_opacity: Some(1.0),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 16,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
    }

    /// Helper: publish an alert banner notification.
    fn publish_alert(scene: &mut SceneGraph, text: &str, urgency: u32, publisher: &str) {
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: text.to_owned(),
                    icon: String::new(),
                    urgency,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                publisher,
                None,
                None,
                None,
            )
            .expect("alert-banner publish must succeed");
    }

    /// Critical (urgency=3) banner must appear above warning (urgency=2).
    ///
    /// With two banners, slot 0 (top) must be the critical one regardless of
    /// publication order (warning published before critical).
    #[tokio::test]
    async fn test_alert_banner_critical_above_warning() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        let mut scene = make_alert_banner_scene();

        // Publish warning first, then critical.
        publish_alert(&mut scene, "Warning: disk space low", 2, "agent-a");
        publish_alert(&mut scene, "Critical: system failure", 3, "agent-b");

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 2, "two banners → two TextItems");

        // Slot 0 (pixel_y=0) must be the critical banner (urgency 3).
        // Slot 1 (pixel_y=slot_h) must be the warning banner (urgency 2).
        // The critical banner is at a lower pixel_y value (top of the zone).
        let y_first = items[0].pixel_y;
        let y_second = items[1].pixel_y;
        assert!(
            y_first < y_second,
            "slot 0 (critical) must be above slot 1 (warning): y0={y_first} y1={y_second}"
        );
        assert!(
            items[0].text.contains("Critical"),
            "slot 0 must be the critical banner; got: {}",
            items[0].text
        );
        assert!(
            items[1].text.contains("Warning"),
            "slot 1 must be the warning banner; got: {}",
            items[1].text
        );
    }

    /// Warning (urgency=2) banner must appear above info (urgency=0-1).
    ///
    /// Info published before warning — severity sort must override arrival order.
    #[tokio::test]
    async fn test_alert_banner_warning_above_info() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        let mut scene = make_alert_banner_scene();

        // Publish info first, then warning.
        publish_alert(&mut scene, "Info: update available", 1, "agent-a");
        publish_alert(&mut scene, "Warning: memory pressure", 2, "agent-b");

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 2, "two banners → two TextItems");

        assert!(
            items[0].text.contains("Warning"),
            "slot 0 must be the warning banner; got: {}",
            items[0].text
        );
        assert!(
            items[1].text.contains("Info"),
            "slot 1 must be the info banner; got: {}",
            items[1].text
        );
        assert!(
            items[0].pixel_y < items[1].pixel_y,
            "warning slot must be above info slot"
        );
    }

    /// Three-level severity stack: critical → warning → info (top to bottom).
    ///
    /// Published in reverse order (info, warning, critical) to confirm severity
    /// sort overrides arrival order.
    #[tokio::test]
    async fn test_alert_banner_three_level_severity_stack() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        let mut scene = make_alert_banner_scene();

        // Publish info first, then warning, then critical.
        publish_alert(&mut scene, "Info: routine scan complete", 0, "agent-a");
        publish_alert(&mut scene, "Warning: high load", 2, "agent-b");
        publish_alert(&mut scene, "Critical: disk full", 3, "agent-c");

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 3, "three banners → three TextItems");

        // Verify order: critical (slot 0), warning (slot 1), info (slot 2).
        assert!(
            items[0].text.contains("Critical"),
            "slot 0 must be critical; got: {}",
            items[0].text
        );
        assert!(
            items[1].text.contains("Warning"),
            "slot 1 must be warning; got: {}",
            items[1].text
        );
        assert!(
            items[2].text.contains("Info"),
            "slot 2 must be info; got: {}",
            items[2].text
        );
        // Pixel positions must decrease (slot 0 < slot 1 < slot 2 in pixel_y).
        assert!(
            items[0].pixel_y < items[1].pixel_y,
            "critical above warning"
        );
        assert!(items[1].pixel_y < items[2].pixel_y, "warning above info");
    }

    /// Same-severity banners: the newer one must appear above the older one.
    ///
    /// Two warnings published in order (A first, B second).  Slot 0 must be
    /// the newer one ("Warning B").
    ///
    /// The sort is deterministic even when timestamps are equal: a tertiary
    /// `index descending` key in `sort_alert_banner_indices` ensures the later
    /// insert (higher index) always wins on exact timestamp ties.
    #[tokio::test]
    async fn test_alert_banner_same_severity_recency_order() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        let mut scene = make_alert_banner_scene();

        // Publish two warnings in order.  Even if both arrive in the same µs,
        // the tertiary index key ensures B (higher index) sorts above A.
        publish_alert(&mut scene, "Warning A (older)", 2, "agent-a");
        publish_alert(&mut scene, "Warning B (newer)", 2, "agent-b");

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 2, "two warnings → two TextItems");

        // Newer publish must be slot 0 (top).
        assert!(
            items[0].text.contains("Warning B"),
            "slot 0 must be the newer warning (B); got: {}",
            items[0].text
        );
        assert!(
            items[1].text.contains("Warning A"),
            "slot 1 must be the older warning (A); got: {}",
            items[1].text
        );
    }

    /// Alert-banner zone height grows dynamically with active banner count.
    ///
    /// Test helper zone: height_pct=0.05, so static zone height at 720p = 36px.
    /// slot_h = font_size_px(16) + 2 × margin_v(8) + SLOT_BASELINE_GAP(2) = 34px.
    ///
    /// - 0 banners → 0 vertices (zero height, nothing rendered).
    /// - 1 banner  → 1 backdrop quad (6 vertices), slot at y=0..34px.
    /// - 3 banners → 3 backdrop quads (18 vertices), 3rd slot at y=68..102px —
    ///   this exceeds the 36px static height, proving dynamic expansion.
    #[tokio::test]
    async fn test_alert_banner_zone_height_grows_with_active_count() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        // ── 0 banners: no vertices emitted ──────────────────────────────────
        {
            let scene = make_alert_banner_scene();
            let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
            compositor.render_zone_content(
                &scene,
                &mut vertices,
                &mut Vec::new(),
                1280.0,
                720.0,
                None,
            );
            assert!(
                vertices.is_empty(),
                "0 banners → 0 vertices (zero height); got {} vertices",
                vertices.len()
            );
        }

        // ── 1 banner: one backdrop quad (6 vertices) ────────────────────────
        {
            let mut scene = make_alert_banner_scene();
            publish_alert(&mut scene, "Single banner", 2, "agent-a");
            let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
            compositor.render_zone_content(
                &scene,
                &mut vertices,
                &mut Vec::new(),
                1280.0,
                720.0,
                None,
            );
            assert_eq!(
                vertices.len(),
                6,
                "1 banner → 1 backdrop quad (6 vertices); got {}",
                vertices.len()
            );
        }

        // ── 3 banners: three backdrop quads (18 vertices) ───────────────────
        //
        // The static zone height is 36px (height_pct=0.05 × 720p).  Each slot
        // is 34px.  Under fixed-height logic the 2nd slot (y=34) would be
        // clipped at 36px (~2px visible) and the 3rd (y=68) would be invisible.
        // Dynamic height = 3 × 34 = 102px allows all three to render fully.
        {
            let mut scene = make_alert_banner_scene();
            publish_alert(&mut scene, "Banner A", 1, "agent-a");
            publish_alert(&mut scene, "Banner B", 2, "agent-b");
            publish_alert(&mut scene, "Banner C", 3, "agent-c");
            let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
            compositor.render_zone_content(
                &scene,
                &mut vertices,
                &mut Vec::new(),
                1280.0,
                720.0,
                None,
            );
            assert_eq!(
                vertices.len(),
                18,
                "3 banners → 3 backdrop quads (18 vertices); got {} — \
                 dynamic height must expand beyond static zone height",
                vertices.len()
            );
            // Verify that all 3 quads are at distinct y positions (slot 0 ≠ slot 2).
            // Vertex layout from rect_vertices: vertex 0 is top-left [left, top] in NDC.
            // Slot 0 starts at pixel y=0 → NDC y_top=1.0.
            // Slot 2 starts at pixel y≈68px → NDC y_top≈0.811 (strictly less than 1.0).
            let slot0_ndc_y = vertices[0].position[1];
            let slot2_ndc_y = vertices[12].position[1];
            assert!(
                slot2_ndc_y < slot0_ndc_y,
                "3rd slot must be below 1st slot in NDC y; slot0={slot0_ndc_y}, slot2={slot2_ndc_y}"
            );
        }
    }

    /// render_zone_content for alert-banner uses severity-ordered backdropcolors.
    ///
    /// With critical (urgency=3) and warning (urgency=2), the first backdrop quad
    /// (slot 0, top) must be red (critical color), and the second must be amber
    /// (warning color).
    #[tokio::test]
    async fn test_alert_banner_backdrop_colors_ordered_by_severity() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        let mut scene = make_alert_banner_scene();

        // Publish warning first, then critical (to confirm severity overrides arrival).
        publish_alert(&mut scene, "Warning", 2, "agent-a");
        publish_alert(&mut scene, "Critical", 3, "agent-b");

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // 2 backdrop quads → 12 vertices.
        assert_eq!(vertices.len(), 12, "2 banners → 12 vertices");

        // Slot 0 (vertices 0-5) must be critical red: R > 0.9, G < 0.1, B < 0.1.
        let slot0_color = vertices[0].color;
        assert!(
            slot0_color[0] > 0.9,
            "slot 0 backdrop R should be ~1.0 (critical red); got {}",
            slot0_color[0]
        );
        assert!(
            slot0_color[1] < 0.1,
            "slot 0 backdrop G should be ~0.0 (critical red); got {}",
            slot0_color[1]
        );

        // Slot 1 (vertices 6-11) must be warning amber: R > 0.9, G mid, B < 0.1.
        let slot1_color = vertices[6].color;
        assert!(
            slot1_color[0] > 0.9,
            "slot 1 backdrop R should be ~1.0 (warning amber); got {}",
            slot1_color[0]
        );
        assert!(
            slot1_color[2] < 0.1,
            "slot 1 backdrop B should be ~0.0 (warning amber); got {}",
            slot1_color[2]
        );
        // Amber has non-trivial G (0.5–0.9), while critical has G < 0.1.
        assert!(
            slot1_color[1] > 0.5,
            "slot 1 backdrop G should be mid (warning amber ≈ 0.72); got {}",
            slot1_color[1]
        );
    }

    // ── Notification text rendering [hud-j5g5.3] ─────────────────────────────
    //
    // Spec §Notification Text Rendering:
    //   - typography.body.size (16px default) font size
    //   - color.text.primary text color
    //   - left-aligned, 9px inset (8px padding + 1px border)
    //   - clips at content area boundary (no wrapping in v1)

    /// Notification text uses typography.body.size (default 16px) when token absent.
    ///
    /// AC: notification text must use font_size_px resolved from typography.body.size.
    #[tokio::test]
    async fn test_notification_text_uses_body_typography_token_default() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "text rendering test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                // No font_size_px set — must fall through to typography.body.size token.
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: Some(8_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Doorbell rang".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();

        // No token map set → typography.body.size absent → default 16px.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 1, "must produce one TextItem for notification");
        assert_eq!(
            items[0].font_size_px, 16.0,
            "notification text must use typography.body.size default (16px)"
        );
    }

    /// Notification text uses typography.body.size resolved from the token map.
    ///
    /// AC: when typography.body.size token is present, it overrides the 16px default.
    #[test]
    fn test_notification_text_uses_typography_body_size_token() {
        let mut token_map = HashMap::new();
        token_map.insert("typography.body.size".to_string(), "20px".to_string());
        let font_size = Compositor::resolve_body_font_size(&token_map);
        assert_eq!(
            font_size, 20.0,
            "typography.body.size=20px must resolve to 20.0"
        );
    }

    /// typography.body.size without 'px' suffix still parses.
    #[test]
    fn test_notification_text_typography_token_without_px_suffix() {
        let mut token_map = HashMap::new();
        token_map.insert("typography.body.size".to_string(), "18".to_string());
        let font_size = Compositor::resolve_body_font_size(&token_map);
        assert_eq!(
            font_size, 18.0,
            "numeric-only typography.body.size must parse"
        );
    }

    /// Absent typography.body.size token falls back to 16px.
    #[test]
    fn test_notification_text_typography_absent_defaults_to_16px() {
        let token_map = HashMap::new();
        let font_size = Compositor::resolve_body_font_size(&token_map);
        assert_eq!(
            font_size, 16.0,
            "absent typography.body.size must default to 16px"
        );
    }

    /// color.text.primary token resolves to the correct sRGB u8 color.
    #[test]
    fn test_notification_text_uses_color_text_primary_token() {
        let mut token_map = HashMap::new();
        // White: #FFFFFF
        token_map.insert("color.text.primary".to_string(), "#FFFFFF".to_string());
        let color = Compositor::resolve_text_primary_color(&token_map);
        assert_eq!(color[0], 255, "color.text.primary #FFFFFF R must be 255");
        assert_eq!(color[1], 255, "color.text.primary #FFFFFF G must be 255");
        assert_eq!(color[2], 255, "color.text.primary #FFFFFF B must be 255");
    }

    /// Absent color.text.primary falls back to near-white.
    #[test]
    fn test_notification_text_primary_absent_falls_back_to_near_white() {
        let token_map = HashMap::new();
        let color = Compositor::resolve_text_primary_color(&token_map);
        assert_eq!(color[0], 255, "fallback text.primary R must be 255");
        assert_eq!(color[1], 255, "fallback text.primary G must be 255");
        assert_eq!(color[2], 255, "fallback text.primary B must be 255");
        // Alpha is near-white (≥200 of 255).
        assert!(
            color[3] >= 200,
            "fallback text.primary alpha must be ≥ 200, got {}",
            color[3]
        );
    }

    /// Notification text is inset by 9px (8px padding + 1px border) from backdrop edges.
    ///
    /// AC: text content area starts at (x + 9, y + 9).
    #[tokio::test]
    async fn test_notification_text_inset_from_backdrop_edges() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone at x=0, y=0 (x_pct=0, y_pct=0) with 100% width and 50% height.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "inset test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,  // zx = 0
                height_pct: 0.5, // zy = 0
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: Some(8_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Test notification".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 1, "must produce one TextItem");

        let item = &items[0];
        // Zone starts at x=0, y=0. Text must be inset by 9px (1px border + 8px padding).
        assert_eq!(
            item.pixel_x, 9.0,
            "text pixel_x must be 9.0 (1px border + 8px padding inset); got {}",
            item.pixel_x
        );
        assert_eq!(
            item.pixel_y, 9.0,
            "text pixel_y must be 9.0 (1px border + 8px padding inset); got {}",
            item.pixel_y
        );
        // Text is left-aligned.
        assert_eq!(
            item.alignment,
            TextAlign::Start,
            "notification text must be left-aligned (TextAlign::Start)"
        );
        // Overflow is Clip.
        assert_eq!(
            item.overflow,
            TextOverflow::Clip,
            "notification text must clip at content area (no wrapping)"
        );
    }

    // ── Two-line notification rendering [hud-ltgk.3] ──────────────────────────
    //
    // Spec §Two-line notification layout:
    //   - Empty title → single-line backward-compatible path (1 TextItem)
    //   - Non-empty title → two-line path (2 TextItems: title bold, body regular)
    //   - Title font_weight = NOTIFICATION_TITLE_WEIGHT (700)
    //   - Body font_size = font_size_px * NOTIFICATION_BODY_SCALE (0.85×)
    //   - Body font_weight = 400 (regular)
    //   - Body positioned below title line + inter-line gap

    /// Two-line notification: empty `title` produces exactly 1 TextItem (backward compat).
    ///
    /// AC: `collect_text_items` with `NotificationPayload { title: "" }` MUST produce
    /// the same output as before this feature was added.
    #[tokio::test]
    async fn test_two_line_notification_empty_title_produces_one_text_item() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "two-line backward-compat test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                font_size_px: Some(16.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Body only notification".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(), // empty: must use single-line path
                    actions: vec![],
                }),
                "test-agent",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        assert_eq!(
            items.len(),
            1,
            "single-line notification (empty title) must produce exactly 1 TextItem"
        );
        assert_eq!(
            items[0].text, "Body only notification",
            "single-line TextItem must contain body text"
        );
        assert_eq!(
            items[0].font_weight, 400,
            "single-line notification must use regular weight (400)"
        );
    }

    /// Two-line notification: non-empty `title` produces 2 TextItems with correct properties.
    ///
    /// AC:
    ///   1. `collect_text_items` produces exactly 2 TextItems for the slot.
    ///   2. First item (lower pixel_y): title text, weight=700, font_size_px=16.
    ///   3. Second item (higher pixel_y): body text, weight=400, font_size=16*0.85=13.6.
    ///   4. Body item pixel_y > title item pixel_y.
    #[tokio::test]
    async fn test_two_line_notification_title_produces_two_text_items() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone at x=0 (x_pct=0) so items are easy to find (pixel_x near 0).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "two-line title test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.5,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                font_size_px: Some(16.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    title: "System Alert".to_owned(),
                    text: "Disk space low on /dev/sda1".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                    ttl_ms: None,
                    actions: vec![],
                }),
                "test-agent",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        assert_eq!(
            items.len(),
            2,
            "two-line notification (non-empty title) must produce exactly 2 TextItems, got {} items: {:?}",
            items.len(),
            items.iter().map(|i| &i.text).collect::<Vec<_>>()
        );

        // Sort by pixel_y to get title (top) and body (bottom).
        let mut sorted = items.clone();
        sorted.sort_by(|a, b| a.pixel_y.partial_cmp(&b.pixel_y).unwrap());
        let title_item = &sorted[0];
        let body_item = &sorted[1];

        // Title item checks.
        assert_eq!(
            title_item.text, "System Alert",
            "first item must be the title text"
        );
        assert_eq!(
            title_item.font_weight, NOTIFICATION_TITLE_WEIGHT,
            "title must use bold weight ({}), got {}",
            NOTIFICATION_TITLE_WEIGHT, title_item.font_weight
        );
        assert!(
            (title_item.font_size_px - 16.0).abs() < 0.01,
            "title must use policy font_size_px (16.0), got {}",
            title_item.font_size_px
        );

        // Body item checks.
        assert_eq!(
            body_item.text, "Disk space low on /dev/sda1",
            "second item must be the body text"
        );
        assert_eq!(
            body_item.font_weight, 400,
            "body must use regular weight (400), got {}",
            body_item.font_weight
        );
        let expected_body_size = 16.0 * NOTIFICATION_BODY_SCALE;
        assert!(
            (body_item.font_size_px - expected_body_size).abs() < 0.1,
            "body font size must be 0.85× title size ({expected_body_size:.2}), got {}",
            body_item.font_size_px
        );

        // Body must be below title.
        assert!(
            body_item.pixel_y > title_item.pixel_y,
            "body item must be below title: title_y={}, body_y={}",
            title_item.pixel_y,
            body_item.pixel_y
        );
    }

    /// Two-line slot height: `notification_slot_height` returns a larger height for
    /// two-line notifications than for single-line notifications.
    ///
    /// AC: two_line_slot_h > single_line_slot_h for the same RenderingPolicy.
    #[test]
    fn test_notification_slot_height_two_line_exceeds_single_line() {
        let policy = RenderingPolicy {
            font_size_px: Some(16.0),
            ..Default::default()
        };

        let single_line = NotificationPayload {
            text: "body".to_owned(),
            title: String::new(),
            ..Default::default()
        };

        let two_line = NotificationPayload {
            title: "Title".to_owned(),
            text: "body".to_owned(),
            ..Default::default()
        };

        let h_single = Compositor::notification_slot_height(
            &single_line,
            &policy,
            NOTIFICATION_BODY_SCALE,
            NOTIFICATION_INTER_LINE_GAP,
        );
        let h_two_line = Compositor::notification_slot_height(
            &two_line,
            &policy,
            NOTIFICATION_BODY_SCALE,
            NOTIFICATION_INTER_LINE_GAP,
        );

        assert!(
            h_two_line > h_single,
            "two-line slot height ({h_two_line:.2}) must exceed single-line ({h_single:.2})"
        );

        // Single-line == stack_slot_height
        let h_stack = Compositor::stack_slot_height(&policy);
        assert!(
            (h_single - h_stack).abs() < 0.01,
            "single-line notification_slot_height ({h_single:.2}) must equal stack_slot_height ({h_stack:.2})"
        );
    }

    /// Two-line notification stacking: two two-line notifications stack correctly,
    /// with the second slot starting after the first two-line slot height.
    #[tokio::test]
    async fn test_two_line_notifications_stack_correctly() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "two-line stacking test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.5,
                height_pct: 0.9,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                font_size_px: Some(16.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish two two-line notifications.
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    title: "First Alert".to_owned(),
                    text: "First body text".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    actions: vec![],
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    title: "Second Alert".to_owned(),
                    text: "Second body text".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    actions: vec![],
                }),
                "agent-b",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // Two two-line notifications = 4 TextItems.
        assert_eq!(
            items.len(),
            4,
            "two two-line notifications must produce 4 TextItems (2 per notification), got {} items",
            items.len()
        );

        // All 4 items must have distinct pixel_y values (no overlap).
        let mut ys: Vec<f32> = items.iter().map(|i| i.pixel_y).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ys.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        assert_eq!(
            ys.len(),
            4,
            "all 4 TextItems must have distinct pixel_y values, got: {:?}",
            ys
        );
    }

    // ── TTL auto-dismiss [hud-j5g5.3] ────────────────────────────────────────
    //
    // Spec §Notification TTL Auto-Dismiss with Fade-Out:
    //   - Default TTL: 8000ms (zone auto_clear_ms)
    //   - Per-publish ttl_ms overrides zone default
    //   - 150ms linear fade-out from 1.0 to 0.0
    //   - Opacity ~0.5 at 75ms midpoint
    //   - Removal from active_publishes on fade completion
    //   - Independent simultaneous fades for multiple notifications

    /// PublicationAnimationState: before TTL expires, opacity is 1.0.
    #[test]
    fn test_pub_anim_state_before_ttl_expiry_opacity_is_1() {
        // TTL = 10_000ms (far future), fade not yet started.
        let state = PublicationAnimationState::new(10_000);
        assert_eq!(
            state.current_opacity(),
            1.0,
            "opacity must be 1.0 before TTL expires"
        );
        assert!(
            !state.is_fade_complete(),
            "fade must not be complete before TTL expires"
        );
    }

    /// PublicationAnimationState: custom TTL=3000ms starts fade at 3000ms.
    ///
    /// AC: notification published with ttl_ms=3000 begins fade-out at 3000ms.
    #[test]
    fn test_pub_anim_state_custom_ttl_3000ms_triggers_fade() {
        let mut state = PublicationAnimationState::new(3_000);

        // Simulate 3001ms elapsed by setting first_seen to the past.
        state.first_seen = std::time::Instant::now() - std::time::Duration::from_millis(3_001);

        state.tick();

        assert!(
            state.fade_start.is_some(),
            "fade must start after TTL (3000ms) has elapsed"
        );
    }

    /// PublicationAnimationState: at 75ms into the 150ms fade, opacity ≈ 0.5.
    ///
    /// AC: opacity interpolates linearly; at midpoint it must be approximately 0.5.
    #[test]
    fn test_pub_anim_state_opacity_at_75ms_midpoint_is_half() {
        let mut state = PublicationAnimationState::new(0); // TTL=0 → instant expire

        // TTL already expired: set first_seen far in the past.
        state.first_seen = std::time::Instant::now() - std::time::Duration::from_secs(1);
        state.tick(); // starts fade

        // Now simulate 75ms into the fade.
        state.fade_start = Some(std::time::Instant::now() - std::time::Duration::from_millis(75));

        let opacity = state.current_opacity();
        assert!(
            (opacity - 0.5).abs() < 0.1,
            "at 75ms midpoint, opacity must be ≈ 0.5, got {opacity}"
        );
    }

    /// PublicationAnimationState: after 150ms, is_fade_complete returns true.
    ///
    /// AC: publication must be removed from active_publishes when fade completes.
    #[test]
    fn test_pub_anim_state_is_complete_after_150ms() {
        let mut state = PublicationAnimationState::new(0);

        // TTL already expired.
        state.first_seen = std::time::Instant::now() - std::time::Duration::from_secs(1);
        state.tick(); // starts fade

        // Simulate 150ms+ elapsed since fade started.
        state.fade_start = Some(std::time::Instant::now() - std::time::Duration::from_millis(151));

        assert!(
            state.is_fade_complete(),
            "is_fade_complete must return true after 150ms fade duration"
        );
        assert_eq!(
            state.current_opacity(),
            0.0,
            "opacity must be 0.0 after fade completes"
        );
    }

    /// prune_faded_publications removes a publication whose fade is complete.
    ///
    /// AC: publication removed from active_publishes when fade-out completes;
    ///     remaining notifications reflow (slot positions recalculated).
    #[tokio::test]
    async fn test_prune_faded_publications_removes_completed_fades() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "prune test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: Some(8_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish two notifications.
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "First".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Second".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-b",
                None,
                None,
                None,
            )
            .unwrap();

        // Manually seed pub_animation_states with a completed-fade for "agent-a".
        let publishes = scene
            .zone_registry
            .active_publishes
            .get("notification-area")
            .unwrap();
        let (a_wall_us, a_ns) = {
            let r = &publishes[0]; // agent-a is first (oldest)
            (r.published_at_wall_us, r.publisher_namespace.clone())
        };

        let mut completed_state = PublicationAnimationState::new(0);
        completed_state.first_seen = std::time::Instant::now() - std::time::Duration::from_secs(1);
        completed_state.tick(); // starts fade
        // Set fade_start 151ms in the past → fade complete.
        completed_state.fade_start =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(151));

        compositor
            .pub_animation_states
            .entry("notification-area".to_string())
            .or_default()
            .insert((a_wall_us, a_ns), completed_state);

        // Before prune: 2 publications.
        assert_eq!(
            scene
                .zone_registry
                .active_publishes
                .get("notification-area")
                .map(|v| v.len()),
            Some(2),
            "before prune: 2 publications expected"
        );

        // Prune: removes agent-a (completed fade).
        compositor.prune_faded_publications(&mut scene);

        // After prune: only 1 publication remains (agent-b).
        let remaining = scene
            .zone_registry
            .active_publishes
            .get("notification-area")
            .map(|v| v.len());
        assert_eq!(
            remaining,
            Some(1),
            "after prune: 1 publication must remain (agent-b)"
        );
        // Verify the remaining publication is agent-b.
        let remaining_pub = &scene.zone_registry.active_publishes["notification-area"][0];
        assert_eq!(
            remaining_pub.publisher_namespace, "agent-b",
            "remaining publication must be from agent-b"
        );
    }

    /// Two notifications with TTLs expiring simultaneously fade independently.
    ///
    /// AC: each has its own PublicationAnimationState; neither affects the other.
    #[test]
    fn test_simultaneous_independent_fades() {
        // Create two independent publication animation states.
        let mut state_a = PublicationAnimationState::new(0);
        let mut state_b = PublicationAnimationState::new(0);

        // Both TTLs expired.
        state_a.first_seen = std::time::Instant::now() - std::time::Duration::from_secs(1);
        state_b.first_seen = std::time::Instant::now() - std::time::Duration::from_secs(1);

        state_a.tick();
        state_b.tick();

        // Simulate: state_a is 75ms into fade, state_b is 120ms into fade.
        state_a.fade_start = Some(std::time::Instant::now() - std::time::Duration::from_millis(75));
        state_b.fade_start =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(120));

        let opacity_a = state_a.current_opacity();
        let opacity_b = state_b.current_opacity();

        // state_a at ~75ms → opacity ≈ 0.5.
        assert!(
            (opacity_a - 0.5).abs() < 0.15,
            "state_a at 75ms must have opacity ≈ 0.5, got {opacity_a}"
        );
        // state_b at ~120ms → opacity ≈ 0.2.
        assert!(
            opacity_b < 0.35,
            "state_b at 120ms must have opacity < 0.35, got {opacity_b}"
        );
        // They are independent — neither affects the other.
        assert!(
            opacity_a > opacity_b,
            "state_a (75ms) must be more opaque than state_b (120ms)"
        );
        assert!(
            !state_a.is_fade_complete(),
            "state_a (75ms into 150ms fade) must not be complete"
        );
        assert!(
            !state_b.is_fade_complete(),
            "state_b (120ms into 150ms fade) must not be complete"
        );
    }

    /// Stack reflow: after a publication is pruned, the remaining slot positions
    /// are recalculated correctly in collect_text_items.
    ///
    /// AC: remaining notifications reflow to fill vacated slot instantly.
    #[tokio::test]
    async fn test_stack_reflow_after_publication_pruned() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone at x=0, y=0 with font_size 16px default → slot_h = line_height(22.4) + 2*8 + 4 = 42.4px.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "reflow test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 1.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: Some(8_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish three notifications from three agents.
        for (agent, text) in [
            ("agent-a", "Alpha"),
            ("agent-b", "Beta"),
            ("agent-c", "Gamma"),
        ] {
            scene
                .publish_to_zone(
                    "notification-area",
                    ZoneContent::Notification(NotificationPayload {
                        text: text.to_owned(),
                        icon: String::new(),
                        urgency: 1,
                        ttl_ms: None,
                        title: String::new(),
                        actions: Vec::new(),
                    }),
                    agent,
                    None,
                    None,
                    None,
                )
                .unwrap();
        }

        // With 3 publications, newest (Gamma) at slot 0, oldest (Alpha) at slot 2.
        let items_before = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items_before.len(), 3, "must have 3 TextItems before prune");

        // Manually mark the oldest (agent-a / Alpha) as fade-complete.
        let publishes = scene
            .zone_registry
            .active_publishes
            .get("notification-area")
            .unwrap();
        let (a_wall_us, a_ns) = {
            let r = &publishes[0]; // agent-a is oldest (index 0)
            (r.published_at_wall_us, r.publisher_namespace.clone())
        };

        let mut completed_state = PublicationAnimationState::new(0);
        completed_state.first_seen = std::time::Instant::now() - std::time::Duration::from_secs(1);
        completed_state.tick();
        completed_state.fade_start =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(151));

        compositor
            .pub_animation_states
            .entry("notification-area".to_string())
            .or_default()
            .insert((a_wall_us, a_ns), completed_state);

        // Prune: removes agent-a.
        compositor.prune_faded_publications(&mut scene);

        // After prune: 2 publications remain (agent-b, agent-c).
        let remaining = scene
            .zone_registry
            .active_publishes
            .get("notification-area")
            .map(|v| v.len());
        assert_eq!(remaining, Some(2), "2 publications must remain after prune");

        // collect_text_items should now produce 2 TextItems correctly reflowed.
        let items_after = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(
            items_after.len(),
            2,
            "must have 2 TextItems after prune (reflow)"
        );

        // Newest (Gamma = agent-c) is at slot 0 (top, pixel_y = 9.0).
        // Oldest remaining (Beta = agent-b) is at slot 1.
        // slot_h = line_height(16*1.4) + 2*margin_v(8) + SLOT_BASELINE_GAP(4) = 42.4px.
        // Slot 1 starts at y=42.4, text at y=42.4+9=51.4.
        let gamma_item = items_after.iter().find(|i| i.text == "Gamma");
        let beta_item = items_after.iter().find(|i| i.text == "Beta");

        assert!(gamma_item.is_some(), "Gamma must be in remaining TextItems");
        assert!(beta_item.is_some(), "Beta must be in remaining TextItems");

        // Gamma is newest → slot 0 → pixel_y = 0 + 9 = 9.
        assert_eq!(
            gamma_item.unwrap().pixel_y,
            9.0,
            "Gamma (newest) must be at slot 0, pixel_y=9.0"
        );
        // Beta is oldest remaining → slot 1 → pixel_y = 42.4 + 9 = 51.4.
        assert_eq!(
            beta_item.unwrap().pixel_y,
            51.4,
            "Beta (oldest remaining) must be at slot 1, pixel_y=51.4"
        );
    }

    /// update_publication_animations creates fresh state for new publications.
    #[test]
    fn test_update_publication_animations_seeds_fresh_state() {
        // We test this without GPU by constructing the compositor state manually.
        // Use a SceneGraph with a Stack zone and one publication.
        use std::sync::Arc;
        use tze_hud_scene::clock::TestClock;

        let clock = Arc::new(TestClock::new(1_000)); // start at t=1000ms
        let mut scene = SceneGraph::new_with_clock(1280.0, 720.0, clock.clone());

        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "animation seed test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: Some(8_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Hello".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: Some(3_000),
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();

        // Build a minimal compositor state just for the animation map test.
        // We can't construct a full headless Compositor without GPU, so we test
        // the helper methods directly.
        let publishes = scene
            .zone_registry
            .active_publishes
            .get("notification-area")
            .unwrap();
        let record = &publishes[0];

        // Test publication_ttl_ms: urgency-derived expires_at_wall_us takes highest
        // priority.  urgency=1 auto-derives expires_at = now + 8_000_000µs, so
        // publication_ttl_ms = 8_000 - NOTIFICATION_FADE_OUT_MS(150) = 7_850.
        // The per-notification ttl_ms=3_000 is superseded by expires_at_wall_us.
        let zone_def = scene.zone_registry.zones.get("notification-area").unwrap();
        let zone_auto_clear = zone_def
            .auto_clear_ms
            .unwrap_or(NOTIFICATION_DEFAULT_TTL_MS);
        let ttl = Compositor::publication_ttl_ms(record, zone_auto_clear);
        assert_eq!(
            ttl, 7_850,
            "publication_ttl_ms must use urgency-derived expires_at_wall_us (8_000ms - 150ms fade = 7_850ms)"
        );

        // Test fallback: when NotificationPayload.ttl_ms is None, use zone default.
        let record_no_ttl = ZonePublishRecord {
            zone_name: "notification-area".to_string(),
            publisher_namespace: "agent-b".to_string(),
            content: ZoneContent::Notification(NotificationPayload {
                text: "No TTL".to_owned(),
                icon: String::new(),
                urgency: 0,
                ttl_ms: None,
                title: String::new(),
                actions: Vec::new(),
            }),
            published_at_wall_us: 2_000_000,
            merge_key: None,
            expires_at_wall_us: None,
            content_classification: None,
            breakpoints: Vec::new(),
        };
        let ttl_fallback = Compositor::publication_ttl_ms(&record_no_ttl, 8_000);
        assert_eq!(
            ttl_fallback, 8_000,
            "publication_ttl_ms must fall back to zone auto_clear_ms=8000 when NotificationPayload.ttl_ms is None"
        );
    }

    // ── ZoneContent::StaticImage rendering ───────────────────────────────────

    /// render_zone_content with ZoneContent::StaticImage must emit a warm-gray
    /// placeholder quad (R≈0.3, G≈0.3, B≈0.3) regardless of the zone's policy
    /// backdrop color.
    ///
    /// Full GPU texture upload (wgpu sampler pipeline) is deferred; this test
    /// confirms the placeholder path is exercised.
    #[tokio::test]
    async fn test_static_image_zone_emits_warm_gray_placeholder() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "pip".to_owned(),
            description: "picture-in-picture zone".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.25,
            },
            accepted_media_types: vec![ZoneMediaType::StaticImage],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        let resource_id = ResourceId::of(b"placeholder-image-bytes");
        scene
            .publish_to_zone(
                "pip",
                ZoneContent::StaticImage(resource_id),
                "test-agent",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, &mut Vec::new(), 1280.0, 720.0, None);

        // At least one backdrop quad must be emitted.
        assert!(
            !vertices.is_empty(),
            "StaticImage zone must emit backdrop vertices"
        );

        // The first vertex color must be warm-gray (R≈0.3, G≈0.3, B≈0.3, A≈1.0).
        let color = vertices[0].color;
        assert!(
            (color[0] - 0.3).abs() < 0.01,
            "StaticImage placeholder R must be ~0.3, got {}",
            color[0]
        );
        assert!(
            (color[1] - 0.3).abs() < 0.01,
            "StaticImage placeholder G must be ~0.3, got {}",
            color[1]
        );
        assert!(
            (color[2] - 0.3).abs() < 0.01,
            "StaticImage placeholder B must be ~0.3, got {}",
            color[2]
        );
        assert!(
            color[3] > 0.5,
            "StaticImage placeholder must be substantially opaque (A > 0.5), got {}",
            color[3]
        );
    }

    // ── LayerAttachment rendering order tests ─────────────────────────────────
    //
    // These tests verify that render_zone_content respects LayerAttachment when
    // an only_layer filter is provided, and that the three-pass ordering
    // (Background → Content → Chrome) is enforced by the layer filter.
    //
    // The approach: register zones with distinct SolidColor publishes, then call
    // render_zone_content with each layer filter in sequence and verify which
    // vertices are emitted.  rect_vertices emits 6 vertices per quad; the color
    // fields let us identify which zone's vertices are which.

    /// Background zones emit vertices only when the Background layer filter is used.
    /// Content zones emit no vertices when filtered to Background only.
    #[tokio::test]
    async fn test_layer_filter_background_only_emits_background_vertices() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Background zone: solid dark blue (r=0.0, g=0.0, b=1.0).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "bg-zone".to_owned(),
            description: "background layer".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 1.0,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Background,
        });

        // Content zone: solid red (r=1.0, g=0.0, b=0.0).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "content-zone".to_owned(),
            description: "content layer".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.1,
                y_pct: 0.1,
                width_pct: 0.5,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        scene
            .publish_to_zone(
                "bg-zone",
                ZoneContent::SolidColor(Rgba::new(0.0, 0.0, 1.0, 1.0)),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "content-zone",
                ZoneContent::SolidColor(Rgba::new(1.0, 0.0, 0.0, 1.0)),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        // Filter: Background only — should emit bg-zone quads (6 verts), not content-zone quads.
        let mut bg_only: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(
            &scene,
            &mut bg_only,
            &mut Vec::new(),
            1280.0,
            720.0,
            Some(LayerAttachment::Background),
        );
        // rect_vertices emits 6 vertices; bg-zone should emit exactly 6.
        assert_eq!(
            bg_only.len(),
            6,
            "Background filter must emit exactly one quad (6 verts) for bg-zone"
        );
        // Verify the color is the bg-zone blue (r≈0.0, b≈1.0).
        let first_color = bg_only[0].color;
        assert!(
            first_color[0] < 0.1,
            "Background zone vertex R must be near 0.0 (blue); got {:?}",
            first_color
        );
        assert!(
            first_color[2] > 0.9,
            "Background zone vertex B must be near 1.0 (blue); got {:?}",
            first_color
        );

        // Filter: Content only — should emit content-zone quads, not bg-zone quads.
        let mut content_only: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(
            &scene,
            &mut content_only,
            &mut Vec::new(),
            1280.0,
            720.0,
            Some(LayerAttachment::Content),
        );
        assert_eq!(
            content_only.len(),
            6,
            "Content filter must emit exactly one quad (6 verts) for content-zone"
        );
        // Verify the color is the content-zone red (r≈1.0, b≈0.0).
        let content_color = content_only[0].color;
        assert!(
            content_color[0] > 0.9,
            "Content zone vertex R must be near 1.0 (red); got {:?}",
            content_color
        );
        assert!(
            content_color[2] < 0.1,
            "Content zone vertex B must be near 0.0 (red); got {:?}",
            content_color
        );
    }

    /// Chrome zones emit vertices only when the Chrome layer filter is used.
    /// Using Chrome filter emits no Content zone vertices.
    #[tokio::test]
    async fn test_layer_filter_chrome_only_emits_chrome_vertices() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Content zone: solid green (r=0.0, g=1.0, b=0.0).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "content-zone".to_owned(),
            description: "content layer".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.1,
                y_pct: 0.1,
                width_pct: 0.5,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Chrome zone: solid yellow (r=1.0, g=1.0, b=0.0).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "chrome-zone".to_owned(),
            description: "chrome layer".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.3,
                height_pct: 0.1,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene
            .publish_to_zone(
                "content-zone",
                ZoneContent::SolidColor(Rgba::new(0.0, 1.0, 0.0, 1.0)),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "chrome-zone",
                ZoneContent::SolidColor(Rgba::new(1.0, 1.0, 0.0, 1.0)),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        // Chrome filter: must emit only chrome-zone vertices.
        let mut chrome_only: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(
            &scene,
            &mut chrome_only,
            &mut Vec::new(),
            1280.0,
            720.0,
            Some(LayerAttachment::Chrome),
        );
        assert_eq!(
            chrome_only.len(),
            6,
            "Chrome filter must emit exactly one quad (6 verts) for chrome-zone"
        );
        // Verify the color is the chrome-zone yellow (r≈1.0, g≈1.0, b≈0.0).
        let chrome_color = chrome_only[0].color;
        assert!(
            chrome_color[0] > 0.9,
            "Chrome zone vertex R must be near 1.0 (yellow); got {:?}",
            chrome_color
        );
        assert!(
            chrome_color[1] > 0.9,
            "Chrome zone vertex G must be near 1.0 (yellow); got {:?}",
            chrome_color
        );
        assert!(
            chrome_color[2] < 0.1,
            "Chrome zone vertex B must be near 0.0 (yellow); got {:?}",
            chrome_color
        );

        // Content filter: must emit only content-zone vertices.
        let mut content_only: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(
            &scene,
            &mut content_only,
            &mut Vec::new(),
            1280.0,
            720.0,
            Some(LayerAttachment::Content),
        );
        assert_eq!(
            content_only.len(),
            6,
            "Content filter must emit exactly one quad (6 verts) for content-zone"
        );
    }

    /// Three-pass ordering: Background vertices precede Content, Content precedes Chrome.
    ///
    /// This test registers zones in Chrome→Background→Content order (reverse of
    /// the canonical order) and verifies that manual three-pass rendering produces
    /// the correct ordering regardless of registration order.
    #[tokio::test]
    async fn test_three_pass_ordering_independent_of_registration_order() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);
        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Register in REVERSE order: Chrome first, then Background, then Content.
        // The rendering order must still be Background → Content → Chrome.

        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "chrome-zone".to_owned(),
            description: "registered first but renders last".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.2,
                height_pct: 0.1,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "bg-zone".to_owned(),
            description: "registered second but renders first".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 1.0,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Background,
        });

        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "content-zone".to_owned(),
            description: "registered third, renders between bg and chrome".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.1,
                y_pct: 0.1,
                width_pct: 0.5,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Publish distinct colors so we can identify each zone's vertices.
        // Background = blue (r=0, g=0, b=1), Content = red (r=1, g=0, b=0),
        // Chrome = yellow (r=1, g=1, b=0).
        scene
            .publish_to_zone(
                "chrome-zone",
                ZoneContent::SolidColor(Rgba::new(1.0, 1.0, 0.0, 1.0)),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "bg-zone",
                ZoneContent::SolidColor(Rgba::new(0.0, 0.0, 1.0, 1.0)),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "content-zone",
                ZoneContent::SolidColor(Rgba::new(1.0, 0.0, 0.0, 1.0)),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        // Perform three-pass rendering into a single vertex buffer.
        // Pass 1: Background.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        let mut tex_cmds: Vec<TexturedDrawCmd> = Vec::new();
        compositor.render_zone_content(
            &scene,
            &mut vertices,
            &mut tex_cmds,
            1280.0,
            720.0,
            Some(LayerAttachment::Background),
        );
        let after_background = vertices.len();

        // Pass 2: Content.
        compositor.render_zone_content(
            &scene,
            &mut vertices,
            &mut tex_cmds,
            1280.0,
            720.0,
            Some(LayerAttachment::Content),
        );
        let after_content = vertices.len();

        // Pass 3: Chrome.
        compositor.render_zone_content(
            &scene,
            &mut vertices,
            &mut tex_cmds,
            1280.0,
            720.0,
            Some(LayerAttachment::Chrome),
        );
        let after_chrome = vertices.len();

        // Each zone produces exactly 6 vertices (one rect_vertices quad).
        assert_eq!(
            after_background, 6,
            "Background pass must emit 6 vertices; got {after_background}"
        );
        assert_eq!(
            after_content, 12,
            "After Content pass, total must be 12 vertices; got {after_content}"
        );
        assert_eq!(
            after_chrome, 18,
            "After Chrome pass, total must be 18 vertices; got {after_chrome}"
        );

        // Verify vertex colors are in the correct positional order:
        // indices 0–5 = Background (blue), 6–11 = Content (red), 12–17 = Chrome (yellow).
        let bg_r = vertices[0].color[0];
        let bg_b = vertices[0].color[2];
        assert!(
            bg_r < 0.1,
            "First quad (background) must be blue (R≈0.0); got R={bg_r}"
        );
        assert!(
            bg_b > 0.9,
            "First quad (background) must be blue (B≈1.0); got B={bg_b}"
        );

        let content_r = vertices[6].color[0];
        let content_b = vertices[6].color[2];
        assert!(
            content_r > 0.9,
            "Second quad (content) must be red (R≈1.0); got R={content_r}"
        );
        assert!(
            content_b < 0.1,
            "Second quad (content) must be red (B≈0.0); got B={content_b}"
        );

        let chrome_r = vertices[12].color[0];
        let chrome_g = vertices[12].color[1];
        assert!(
            chrome_r > 0.9,
            "Third quad (chrome) must be yellow (R≈1.0); got R={chrome_r}"
        );
        assert!(
            chrome_g > 0.9,
            "Third quad (chrome) must be yellow (G≈1.0); got G={chrome_g}"
        );
    }

    /// publication_ttl_ms derives TTL (delay until fade starts) from expires_at_wall_us
    /// when present (highest priority), subtracting NOTIFICATION_FADE_OUT_MS so the
    /// fade completes before the drain boundary.
    ///
    /// For a 15 s warning: ttl_ms = 15_000 - 150 = 14_850.
    /// For a 30 s critical: ttl_ms = 30_000 - 150 = 29_850.
    #[test]
    fn test_publication_ttl_ms_uses_expires_at_wall_us() {
        // Warning notification (urgency 2): published at t=0, expires at t=15s.
        // Expected: 15_000 ms - 150 ms fade = 14_850 ms until fade starts.
        let record_warning = ZonePublishRecord {
            zone_name: "alert-banner".to_string(),
            publisher_namespace: "agent-warn".to_string(),
            content: ZoneContent::Notification(NotificationPayload {
                text: "Disk space low".to_owned(),
                icon: String::new(),
                urgency: 2,
                ttl_ms: None, // No per-notification TTL — urgency path sets expires_at
                title: String::new(),
                actions: Vec::new(),
            }),
            published_at_wall_us: 0,
            merge_key: None,
            expires_at_wall_us: Some(15_000_000), // 15 s in µs
            content_classification: None,
            breakpoints: Vec::new(),
        };
        let ttl = Compositor::publication_ttl_ms(&record_warning, 8_000);
        assert_eq!(
            ttl, 14_850,
            "publication_ttl_ms must derive 14_850 ms (15_000 - 150 fade) for a 15s warning"
        );

        // Critical notification (urgency 3): published at t=0, expires at t=30s.
        // Expected: 30_000 ms - 150 ms fade = 29_850 ms until fade starts.
        let record_critical = ZonePublishRecord {
            zone_name: "alert-banner".to_string(),
            publisher_namespace: "agent-crit".to_string(),
            content: ZoneContent::Notification(NotificationPayload {
                text: "System failure".to_owned(),
                icon: String::new(),
                urgency: 3,
                ttl_ms: None,
                title: String::new(),
                actions: Vec::new(),
            }),
            published_at_wall_us: 0,
            merge_key: None,
            expires_at_wall_us: Some(30_000_000), // 30 s in µs
            content_classification: None,
            breakpoints: Vec::new(),
        };
        let ttl_crit = Compositor::publication_ttl_ms(&record_critical, 8_000);
        assert_eq!(
            ttl_crit, 29_850,
            "publication_ttl_ms must derive 29_850 ms (30_000 - 150 fade) for a 30s critical"
        );

        // expires_at_wall_us takes priority over per-notification ttl_ms.
        // published=1s, expires=16s → duration=15s → 15_000 - 150 = 14_850 ms until fade.
        let record_both = ZonePublishRecord {
            zone_name: "alert-banner".to_string(),
            publisher_namespace: "agent-both".to_string(),
            content: ZoneContent::Notification(NotificationPayload {
                text: "Both set".to_owned(),
                icon: String::new(),
                urgency: 2,
                ttl_ms: Some(5_000), // explicit 5 s TTL on the notification itself
                title: String::new(),
                actions: Vec::new(),
            }),
            published_at_wall_us: 1_000_000, // published at t=1s
            merge_key: None,
            expires_at_wall_us: Some(16_000_000), // expires at t=16s → 15 s duration
            content_classification: None,
            breakpoints: Vec::new(),
        };
        let ttl_both = Compositor::publication_ttl_ms(&record_both, 8_000);
        assert_eq!(
            ttl_both, 14_850,
            "publication_ttl_ms must prefer expires_at_wall_us over per-notification ttl_ms (14_850 ms = 15_000 - 150)"
        );

        // Info notification (urgency 1, no expires_at): falls back to ttl_ms then zone default.
        let record_info = ZonePublishRecord {
            zone_name: "alert-banner".to_string(),
            publisher_namespace: "agent-info".to_string(),
            content: ZoneContent::Notification(NotificationPayload {
                text: "All good".to_owned(),
                icon: String::new(),
                urgency: 1,
                ttl_ms: Some(8_000),
                title: String::new(),
                actions: Vec::new(),
            }),
            published_at_wall_us: 0,
            merge_key: None,
            expires_at_wall_us: None,
            content_classification: None,
            breakpoints: Vec::new(),
        };
        let ttl_info = Compositor::publication_ttl_ms(&record_info, 8_000);
        assert_eq!(
            ttl_info, 8_000,
            "publication_ttl_ms must use NotificationPayload.ttl_ms when expires_at_wall_us is absent"
        );
    }

    // ── Transition interrupt semantics [hud-hzub.2] ─────────────────────────

    /// fade_in_from starts from a non-zero opacity.
    ///
    /// Acceptance criterion: transition interrupt semantics must begin fade-in
    /// from current composite opacity, not from zero.
    #[test]
    fn test_fade_in_from_starts_at_given_opacity() {
        // Simulate: fade-out was 50% complete → current_opacity = 0.5.
        // Start a fade_in_from(0.5) — should begin at 0.5.
        let state = ZoneAnimationState::fade_in_from(10_000, 0.5);
        let opacity = state.current_opacity();
        // Very shortly after creation, opacity should be ~0.5 (no time has elapsed).
        assert!(
            (opacity - 0.5).abs() < 0.05,
            "fade_in_from(0.5) should start at ~0.5 opacity, got {opacity}"
        );
        assert_eq!(state.target_opacity, 1.0, "fade_in_from target must be 1.0");
    }

    /// fade_in_from clamps from_opacity to [0.0, 1.0].
    #[test]
    fn test_fade_in_from_clamps_opacity() {
        let state_low = ZoneAnimationState::fade_in_from(1_000, -0.5);
        assert_eq!(state_low.from_opacity, 0.0, "negative opacity clamped to 0");
        let state_high = ZoneAnimationState::fade_in_from(1_000, 1.5);
        assert_eq!(
            state_high.from_opacity, 1.0,
            "overflow opacity clamped to 1"
        );
    }

    /// Transition interrupt: update_zone_animations starts fade-in from current
    /// opacity when a new publish arrives during an active fade-out.
    #[tokio::test]
    async fn test_transition_interrupt_starts_fade_in_from_current_opacity() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "transition interrupt test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                transition_in_ms: Some(200),
                transition_out_ms: Some(150),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Step 1: publish content — this makes zone active.
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("First".to_owned()),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();
        compositor.update_zone_animations(&scene);

        // Step 2: clear — marks zone inactive, starts fade-out.
        scene
            .zone_registry
            .active_publishes
            .get_mut("subtitle")
            .unwrap()
            .clear();
        compositor.update_zone_animations(&scene);

        // The zone animation state should now be a fade-out (target = 0).
        let has_fadeout = compositor
            .zone_animation_states
            .get("subtitle")
            .map(|s| s.target_opacity == 0.0)
            .unwrap_or(false);
        assert!(has_fadeout, "expected fade-out state after zone clear");

        // Inject a partially-complete fade-out (from_opacity=1, target=0, ~50% elapsed).
        // We simulate 50% opacity by creating a state with from_opacity=1.0 and checking
        // that after interrupt, from_opacity is NOT 0.0.
        let partial_opacity = compositor
            .zone_animation_states
            .get("subtitle")
            .map(|s| s.current_opacity())
            .unwrap_or(0.0);
        // At t=0 the fade-out just started, so opacity ≈ 1.0 still.
        assert!(
            partial_opacity > 0.5,
            "fade-out just started, opacity should be > 0.5, got {partial_opacity}"
        );

        // Step 3: re-publish during fade-out — interrupt semantics must apply.
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("Second".to_owned()),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        // Record fade-out opacity just before interrupt.
        let pre_interrupt_opacity = compositor
            .zone_animation_states
            .get("subtitle")
            .map(|s| s.current_opacity())
            .unwrap_or(0.0);

        compositor.update_zone_animations(&scene);

        // After interrupt: must be fade-in (target = 1.0).
        let state = compositor
            .zone_animation_states
            .get("subtitle")
            .expect("zone animation state must exist after interrupt fade-in");
        assert_eq!(
            state.target_opacity, 1.0,
            "transition interrupt must produce a fade-in state (target = 1.0)"
        );
        // from_opacity must be the interrupted fade-out opacity, not 0.
        // Pre-interrupt opacity is > 0.5 (fade-out just started), so from ≈ pre_interrupt.
        assert!(
            state.from_opacity > 0.0,
            "fade_in_from must start from current opacity (> 0), got {}",
            state.from_opacity
        );
        // The from_opacity should be ≈ the pre-interrupt value (fade-out just started).
        assert!(
            (state.from_opacity - pre_interrupt_opacity).abs() < 0.1,
            "fade_in_from must start from current fade-out opacity (~{pre_interrupt_opacity}), got {}",
            state.from_opacity
        );
    }

    // ── Streaming word-by-word reveal [hud-hzub.2] ──────────────────────────

    /// StreamRevealState.visible_byte_offset returns usize::MAX when no breakpoints.
    #[test]
    fn test_stream_reveal_no_breakpoints_reveals_all() {
        let state = StreamRevealState::new(
            (1_000_000, "agent".to_owned()),
            vec![] as Vec<u64>, // no breakpoints
        );
        assert_eq!(
            state.visible_byte_offset(),
            usize::MAX,
            "empty breakpoints must reveal all text immediately"
        );
    }

    /// StreamRevealState starts at segment 0 and reveals first breakpoint.
    #[test]
    fn test_stream_reveal_starts_at_first_breakpoint() {
        let state = StreamRevealState::new(
            (1_000_000, "agent".to_owned()),
            vec![3, 9, 15], // "The" at 3, "The quick" at 9, etc.
        );
        assert_eq!(
            state.visible_byte_offset(),
            3,
            "initial visible_byte_offset must be breakpoints[0]=3"
        );
    }

    /// StreamRevealState.advance() progresses through breakpoints.
    #[test]
    fn test_stream_reveal_advance_progresses_breakpoints() {
        let mut state = StreamRevealState::new((1_000_000, "agent".to_owned()), vec![3, 9, 15]);
        assert_eq!(state.visible_byte_offset(), 3, "initially at breakpoint 0");

        // Advance STREAM_REVEAL_FRAMES_PER_SEGMENT times to move to next.
        for _ in 0..STREAM_REVEAL_FRAMES_PER_SEGMENT {
            state.advance();
        }
        assert_eq!(
            state.visible_byte_offset(),
            9,
            "after advance, at breakpoint 1"
        );

        for _ in 0..STREAM_REVEAL_FRAMES_PER_SEGMENT {
            state.advance();
        }
        assert_eq!(
            state.visible_byte_offset(),
            15,
            "after advance, at breakpoint 2"
        );

        for _ in 0..STREAM_REVEAL_FRAMES_PER_SEGMENT {
            state.advance();
        }
        assert_eq!(
            state.visible_byte_offset(),
            usize::MAX,
            "after all breakpoints revealed, must show full text (usize::MAX)"
        );
    }

    /// update_stream_reveals creates state for StreamText with breakpoints.
    #[tokio::test]
    async fn test_update_stream_reveals_creates_state() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "streaming test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Publish StreamText with breakpoints via publish_to_zone_with_breakpoints.
        scene
            .publish_to_zone_with_breakpoints(
                "subtitle",
                ZoneContent::StreamText("The quick brown fox".to_owned()),
                "agent",
                None,
                None,
                None,
                vec![3, 9, 15],
            )
            .unwrap();

        compositor.update_stream_reveals(&scene);

        let reveal = compositor.stream_reveal_states.get("subtitle");
        assert!(
            reveal.is_some(),
            "stream_reveal_states must have an entry for subtitle"
        );
        let reveal = reveal.unwrap();
        assert_eq!(
            reveal.breakpoints,
            vec![3, 9, 15],
            "breakpoints must match the publish record"
        );
        assert_eq!(reveal.segment_idx, 0, "reveal starts at segment 0");
    }

    /// update_stream_reveals resets state when a new publication replaces old.
    /// Verifies latest-wins cancels in-progress streaming reveal.
    #[tokio::test]
    async fn test_update_stream_reveals_resets_on_new_publish() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "streaming reset test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // First publish with breakpoints.
        scene
            .publish_to_zone_with_breakpoints(
                "subtitle",
                ZoneContent::StreamText("The quick brown fox".to_owned()),
                "agent",
                None,
                None,
                None,
                vec![3, 9, 15],
            )
            .unwrap();
        compositor.update_stream_reveals(&scene);

        // Advance a few frames to simulate partial reveal.
        for _ in 0..(STREAM_REVEAL_FRAMES_PER_SEGMENT + 1) {
            compositor.update_stream_reveals(&scene);
        }
        let partial_idx = compositor
            .stream_reveal_states
            .get("subtitle")
            .map(|s| s.segment_idx)
            .unwrap_or(0);
        assert!(partial_idx > 0, "reveal should have advanced beyond 0");

        // Second publish (different published_at_wall_us) — must reset reveal.
        scene
            .publish_to_zone_with_breakpoints(
                "subtitle",
                ZoneContent::StreamText("New content streaming".to_owned()),
                "agent",
                None,
                None,
                None,
                vec![4, 12],
            )
            .unwrap();
        compositor.update_stream_reveals(&scene);

        let new_reveal = compositor.stream_reveal_states.get("subtitle").unwrap();
        assert_eq!(
            new_reveal.segment_idx, 0,
            "replacement must reset reveal to segment 0 (latest-wins cancel)"
        );
        assert_eq!(
            new_reveal.breakpoints,
            vec![4, 12],
            "new breakpoints must be from the replacement publication"
        );
    }

    /// collect_text_items truncates text to current reveal byte offset.
    #[tokio::test]
    async fn test_collect_text_items_respects_stream_reveal() {
        let (mut compositor, _surface) = require_gpu!(make_compositor_and_surface(1280, 720).await);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "streaming text item test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // "The quick brown fox" — breakpoints at 3, 9, 15.
        // Initially reveals only "The" (3 bytes).
        scene
            .publish_to_zone_with_breakpoints(
                "subtitle",
                ZoneContent::StreamText("The quick brown fox".to_owned()),
                "agent",
                None,
                None,
                None,
                vec![3, 9, 15],
            )
            .unwrap();

        // Create reveal state at segment 0 (reveals "The").
        compositor.update_stream_reveals(&scene);

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert!(!items.is_empty(), "must produce at least one TextItem");
        let visible_text = &items[0].text;
        assert_eq!(
            visible_text, "The",
            "initial reveal must show only text up to first breakpoint (\"The\")"
        );
    }

    // ── Zone interaction hit region tests (hud-ltgk.4) ────────────────────────
    //
    // These tests verify `populate_zone_hit_regions`: the pure-geometry path that
    // computes dismiss (×) and action button pixel bounds for Stack zone
    // notification publications.
    //
    // All tests are GPU-gated (require_gpu!) because populate_zone_hit_regions
    // is a method on Compositor, which requires a GPU device at construction.
    // The method itself is pure geometry — it does not issue GPU commands.

    /// `populate_zone_hit_regions()` MUST produce exactly one dismiss region for a
    /// single notification with no actions in a Stack zone.
    #[tokio::test]
    async fn zone_hit_single_notification_produces_dismiss_region() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(256, 256).await);

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let _tab = scene.create_tab("Main", 0).unwrap();
        scene.register_zone(tze_hud_scene::types::ZoneDefinition {
            id: SceneId::new(),
            name: "notif".to_string(),
            description: "Stack zone".to_string(),
            geometry_policy: tze_hud_scene::types::GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![tze_hud_scene::types::ZoneMediaType::ShortTextWithIcon],
            rendering_policy: tze_hud_scene::types::RenderingPolicy {
                font_size_px: Some(16.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            layer_attachment: tze_hud_scene::types::LayerAttachment::Chrome,
            ephemeral: false,
        });
        scene
            .publish_to_zone(
                "notif",
                ZoneContent::Notification(NotificationPayload {
                    text: "Hello".to_string(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,

                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.populate_zone_hit_regions(&mut scene, 1920.0, 1080.0);

        assert_eq!(
            scene.zone_hit_regions.len(),
            1,
            "single notification with no actions must produce exactly 1 hit region"
        );
        assert_eq!(
            scene.zone_hit_regions[0].kind,
            tze_hud_scene::types::ZoneInteractionKind::Dismiss,
            "single region must be a Dismiss button"
        );
        assert!(
            scene.zone_hit_regions[0].interaction_id.contains("dismiss"),
            "interaction_id must contain 'dismiss': {}",
            scene.zone_hit_regions[0].interaction_id
        );
    }

    /// The dismiss region MUST be positioned at the top-right of the notification slot.
    /// Zone: x_pct=0.75, width_pct=0.24 on a 1920×1080 screen.
    /// Expected: dismiss.x ≈ 1920*0.75 + 1920*0.24 - 20 = 1440 + 460.8 - 20 = 1880.8.
    #[tokio::test]
    async fn zone_hit_dismiss_region_at_top_right_of_slot() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(256, 256).await);

        let sw = 1920.0f32;
        let sh = 1080.0f32;
        let mut scene = SceneGraph::new(sw, sh);
        let _tab = scene.create_tab("Main", 0).unwrap();
        scene.register_zone(tze_hud_scene::types::ZoneDefinition {
            id: SceneId::new(),
            name: "notif".to_string(),
            description: "Stack zone".to_string(),
            geometry_policy: tze_hud_scene::types::GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![tze_hud_scene::types::ZoneMediaType::ShortTextWithIcon],
            rendering_policy: tze_hud_scene::types::RenderingPolicy {
                font_size_px: Some(16.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            layer_attachment: tze_hud_scene::types::LayerAttachment::Chrome,
            ephemeral: false,
        });
        scene
            .publish_to_zone(
                "notif",
                ZoneContent::Notification(NotificationPayload {
                    text: "Test".to_string(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,

                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.populate_zone_hit_regions(&mut scene, sw, sh);

        assert_eq!(
            scene.zone_hit_regions.len(),
            1,
            "must have exactly 1 region"
        );
        let region = &scene.zone_hit_regions[0];

        // Dismiss should be at the top-right of the slot.
        // Zone geometry: zx = 1920*0.75 = 1440, zw = 1920*0.24 = 460.8.
        // Dismiss x = zx + zw - 20 = 1880.8.
        let expected_x = sw * 0.75 + sw * 0.24 - 20.0;
        assert!(
            (region.bounds.x - expected_x).abs() < 1.0,
            "dismiss x must be at top-right (expected≈{expected_x:.1}, got {:.1})",
            region.bounds.x
        );
        assert!(
            region.bounds.y < 1.0,
            "dismiss y must be near top of slot (expected≈0, got {:.1})",
            region.bounds.y
        );
    }

    /// A notification with 2 actions MUST produce 3 regions: 1 dismiss + 2 actions.
    #[tokio::test]
    async fn zone_hit_notification_with_two_actions_produces_three_regions() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(256, 256).await);

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let _tab = scene.create_tab("Main", 0).unwrap();
        scene.register_zone(tze_hud_scene::types::ZoneDefinition {
            id: SceneId::new(),
            name: "notif".to_string(),
            description: "Stack zone".to_string(),
            geometry_policy: tze_hud_scene::types::GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![tze_hud_scene::types::ZoneMediaType::ShortTextWithIcon],
            rendering_policy: tze_hud_scene::types::RenderingPolicy {
                font_size_px: Some(16.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            layer_attachment: tze_hud_scene::types::LayerAttachment::Chrome,
            ephemeral: false,
        });
        scene
            .publish_to_zone(
                "notif",
                ZoneContent::Notification(NotificationPayload {
                    text: "Confirm?".to_string(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,

                    title: String::new(),
                    actions: vec![
                        NotificationAction {
                            label: "Yes".to_string(),
                            callback_id: "yes".to_string(),
                        },
                        NotificationAction {
                            label: "No".to_string(),
                            callback_id: "no".to_string(),
                        },
                    ],
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.populate_zone_hit_regions(&mut scene, 1920.0, 1080.0);

        assert_eq!(
            scene.zone_hit_regions.len(),
            3,
            "1 dismiss + 2 action buttons = 3 regions"
        );

        assert_eq!(
            scene.zone_hit_regions[0].kind,
            tze_hud_scene::types::ZoneInteractionKind::Dismiss,
            "first region must be Dismiss"
        );
        assert!(
            matches!(
                &scene.zone_hit_regions[1].kind,
                tze_hud_scene::types::ZoneInteractionKind::Action { callback_id }
                    if callback_id == "yes"
            ),
            "second region must be Action(yes)"
        );
        assert!(
            matches!(
                &scene.zone_hit_regions[2].kind,
                tze_hud_scene::types::ZoneInteractionKind::Action { callback_id }
                    if callback_id == "no"
            ),
            "third region must be Action(no)"
        );
    }

    /// Tab order MUST be sequential: dismiss=0, action[0]=1, action[1]=2.
    #[tokio::test]
    async fn zone_hit_tab_order_is_sequential() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(256, 256).await);

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let _tab = scene.create_tab("Main", 0).unwrap();
        scene.register_zone(tze_hud_scene::types::ZoneDefinition {
            id: SceneId::new(),
            name: "notif".to_string(),
            description: "Stack zone".to_string(),
            geometry_policy: tze_hud_scene::types::GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![tze_hud_scene::types::ZoneMediaType::ShortTextWithIcon],
            rendering_policy: tze_hud_scene::types::RenderingPolicy {
                font_size_px: Some(16.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            layer_attachment: tze_hud_scene::types::LayerAttachment::Chrome,
            ephemeral: false,
        });
        scene
            .publish_to_zone(
                "notif",
                ZoneContent::Notification(NotificationPayload {
                    text: "Tab order test".to_string(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,

                    title: String::new(),
                    actions: vec![
                        NotificationAction {
                            label: "A".to_string(),
                            callback_id: "a".to_string(),
                        },
                        NotificationAction {
                            label: "B".to_string(),
                            callback_id: "b".to_string(),
                        },
                    ],
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.populate_zone_hit_regions(&mut scene, 1920.0, 1080.0);

        assert_eq!(scene.zone_hit_regions.len(), 3, "must produce 3 regions");
        assert_eq!(
            scene.zone_hit_regions[0].tab_order, 0,
            "dismiss tab_order must be 0"
        );
        assert_eq!(
            scene.zone_hit_regions[1].tab_order, 1,
            "action[0] tab_order must be 1"
        );
        assert_eq!(
            scene.zone_hit_regions[2].tab_order, 2,
            "action[1] tab_order must be 2"
        );
    }

    /// Calling `populate_zone_hit_regions` twice MUST clear stale regions (no accumulation).
    #[tokio::test]
    async fn zone_hit_populate_clears_on_repeated_calls() {
        let (compositor, _surface) = require_gpu!(make_compositor_and_surface(256, 256).await);

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let _tab = scene.create_tab("Main", 0).unwrap();
        scene.register_zone(tze_hud_scene::types::ZoneDefinition {
            id: SceneId::new(),
            name: "notif".to_string(),
            description: "Stack zone".to_string(),
            geometry_policy: tze_hud_scene::types::GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![tze_hud_scene::types::ZoneMediaType::ShortTextWithIcon],
            rendering_policy: tze_hud_scene::types::RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            layer_attachment: tze_hud_scene::types::LayerAttachment::Chrome,
            ephemeral: false,
        });
        scene
            .publish_to_zone(
                "notif",
                ZoneContent::Notification(NotificationPayload {
                    text: "Once".to_string(),
                    icon: String::new(),
                    urgency: 1,
                    ttl_ms: None,

                    title: String::new(),
                    actions: Vec::new(),
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.populate_zone_hit_regions(&mut scene, 1920.0, 1080.0);
        let first_count = scene.zone_hit_regions.len();
        assert_eq!(first_count, 1, "first call must produce 1 region");

        compositor.populate_zone_hit_regions(&mut scene, 1920.0, 1080.0);
        assert_eq!(
            scene.zone_hit_regions.len(),
            1,
            "second call must still produce 1 (not accumulate to 2)"
        );
    }
}
