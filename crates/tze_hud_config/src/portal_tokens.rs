//! Portal part inventory and token mapping for the text-stream portal pilot.
//!
//! Implements §6.2 of `text-stream-portal-phase1/tasks.md`:
//! - Portal part inventory (frame, header, composer, transcript body, divider,
//!   collapsed card)
//! - Token mapping each part consumes
//! - `PortalPartTokens`: resolved visual values extracted from a `DesignTokenMap`
//!
//! Also implements §6b window management tokens (amendment 2026-06-10):
//! - Window geometry bounds (min size, resize step, affordance size)
//! - Scroll-position indicator styling
//!
//! **Pre-promotion rule:** the exemplar adapter MUST source every published visual
//! value from the runtime's resolved token set (via `PortalPartTokens`) rather than
//! literal values. A profile/token change MUST reskin the portal end-to-end without
//! touching adapter logic. See `about/heart-and-soul/v1.md` and CLAUDE.md
//! "visual identity is modular".
//!
//! ## Canonical portal token keys (profile-scoped, pre-promotion)
//!
//! These keys are **portal-scoped**: they are prefixed with `portal.` to avoid
//! colliding with canonical component-shape-language keys. They are resolvable
//! via profile-scoped overrides and fall back to the canonical token defaults.
//! At promotion time they will be canonicalized in the `text-portal` component
//! type contract via a separate component-shape-language delta.
//!
//! | Key | Part | Property |
//! |-----|------|----------|
//! | `portal.frame.background` | frame | backdrop fill (RGBA hex) |
//! | `portal.frame.opacity` | frame | backdrop opacity (0.0–1.0) |
//! | `portal.frame.border_color` | frame | border stroke color (RGBA hex) |
//! | `portal.header.text_color` | header | title text color (RGBA hex) |
//! | `portal.header.font_size` | header | title font size in px |
//! | `portal.composer.background` | composer | input area backdrop color (RGBA hex) |
//! | `portal.composer.text_color` | composer | draft text color (RGBA hex) |
//! | `portal.composer.font_size` | composer | draft font size in px |
//! | `portal.transcript.background` | transcript body | content backdrop color (RGBA hex) |
//! | `portal.transcript.text_color` | transcript body | content text color (RGBA hex) |
//! | `portal.transcript.font_size` | transcript body | content font size in px |
//! | `portal.transcript.dim_text_color` | transcript body | dimmed text shown while disconnected/stale (RGBA hex) |
//! | `portal.transcript.dim_background` | transcript body | dimmed backdrop shown while disconnected/stale (RGBA hex) |
//! | `portal.stale_marker.color` | stale marker | content-free disconnect marker color (RGBA hex) |
//! | `portal.lifecycle.active_color` | lifecycle affordance | accent for `Active` (RGBA hex) |
//! | `portal.lifecycle.attached_color` | lifecycle affordance | accent for `Attached`/ready (RGBA hex) |
//! | `portal.lifecycle.attention_color` | lifecycle affordance | accent for `Degraded`/`HudUnavailable` (RGBA hex) |
//! | `portal.lifecycle.inactive_color` | lifecycle affordance | accent for `Detached`/`CleanupPending`/`Expired` (RGBA hex) |
//! | `portal.divider.color` | divider | separator line color (RGBA hex) |
//! | `portal.collapsed_card.background` | collapsed card | compact view backdrop (RGBA hex) |
//! | `portal.collapsed_card.text_color` | collapsed card | compact text color (RGBA hex) |
//! | `portal.collapsed_card.font_size` | collapsed card | compact text font size in px |
//! | `portal.transition.in_ms` | transitions | collapsed→expanded duration (ms) |
//! | `portal.transition.out_ms` | transitions | expanded→collapsed duration (ms) |
//!
//! ### §6b Window management tokens (amendment 2026-06-10)
//!
//! | Key | Part | Property |
//! |-----|------|----------|
//! | `portal.window.min_width_px` | window bounds | legible minimum width in px |
//! | `portal.window.min_height_px` | window bounds | legible minimum height in px |
//! | `portal.window.resize_step_px` | hotkey resize | pixels per Ctrl+`+`/`-` step |
//! | `portal.window.resize_affordance_px` | resize affordance | capture region size on frame edges |
//! | `portal.scroll_indicator.color` | scroll indicator | thumb color (RGBA hex) |
//! | `portal.scroll_indicator.width_px` | scroll indicator | track width in px |
//! | `portal.scroll_indicator.min_height_px` | scroll indicator | minimum thumb height in px |

use crate::tokens::{DesignTokenMap, Rgba, parse_color_hex, parse_numeric};
use tracing::warn;

// ── Canonical portal token keys ───────────────────────────────────────────────

/// Canonical portal token keys — pre-promotion profile-scoped defaults.
///
/// These are the authoritative key names for the portal part inventory.
/// At promotion time, a `text-portal` component type contract will canonicalize
/// them through the component-shape-language delta.
pub const PORTAL_TOKEN_FRAME_BACKGROUND: &str = "portal.frame.background";
pub const PORTAL_TOKEN_FRAME_OPACITY: &str = "portal.frame.opacity";
pub const PORTAL_TOKEN_FRAME_BORDER_COLOR: &str = "portal.frame.border_color";

pub const PORTAL_TOKEN_HEADER_TEXT_COLOR: &str = "portal.header.text_color";
pub const PORTAL_TOKEN_HEADER_FONT_SIZE: &str = "portal.header.font_size";

pub const PORTAL_TOKEN_COMPOSER_BACKGROUND: &str = "portal.composer.background";
pub const PORTAL_TOKEN_COMPOSER_TEXT_COLOR: &str = "portal.composer.text_color";
pub const PORTAL_TOKEN_COMPOSER_FONT_SIZE: &str = "portal.composer.font_size";
/// Border / indicator color shown when the composer draft reaches its byte cap.
/// Rendered as a visual signal that no further input will be accepted until
/// the user deletes content. Defaults to a muted amber to convey "limit reached"
/// without alarming the user.
pub const PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR: &str = "portal.composer.at_capacity_color";

pub const PORTAL_TOKEN_TRANSCRIPT_BACKGROUND: &str = "portal.transcript.background";
pub const PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR: &str = "portal.transcript.text_color";
pub const PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE: &str = "portal.transcript.font_size";

// ── Degraded / disconnect tokens (portal-disconnect-resume-ux §2/§3) ─────────
//
// Token-resolved degraded treatment for a portal whose driving stream/session
// has dropped (lifecycle `Degraded`/`HudUnavailable`). The retained transcript
// window is dimmed and a content-free stale marker is shown rather than blanking
// or faking liveness. Every value here is token-driven — the adapter never
// hardcodes a degraded color (CLAUDE.md "visual identity is modular").

/// Dimmed transcript text color shown while the portal is disconnected/stale.
/// Distinctly muted relative to the live `transcript.text_color` so a viewer
/// reads the retained window as inactive without it vanishing.
pub const PORTAL_TOKEN_TRANSCRIPT_DIM_TEXT_COLOR: &str = "portal.transcript.dim_text_color";
/// Dimmed transcript background shown while the portal is disconnected/stale.
pub const PORTAL_TOKEN_TRANSCRIPT_DIM_BACKGROUND: &str = "portal.transcript.dim_background";
/// Color of the content-free stale/disconnect marker. Muted amber to convey
/// "ambient stale state" without escalating attention (spec: going stale does
/// not self-escalate attention).
pub const PORTAL_TOKEN_STALE_MARKER_COLOR: &str = "portal.stale_marker.color";

// ── Lifecycle affordance tokens (cooperative-hud-projection §lifecycle) ───────
//
// Token-resolved accent colors driving the viewer-facing affordance for a
// projection's published `lifecycle_state` (active / attached-ready /
// attention / inactive). Each is ambient by design — the cooperative-projection
// and text-stream-portal specs require portal activity to stay ambient/gentle and
// MUST NOT self-escalate interruption class. The adapter maps each
// `ProjectionLifecycleState` variant onto one of these four semantic accents; no
// literal color ever appears in the render path (CLAUDE.md "visual identity is
// modular").

/// Accent shown while the projected session is actively working (`Active`).
/// Calm teal-green: "live" without demanding attention.
pub const PORTAL_TOKEN_LIFECYCLE_ACTIVE_COLOR: &str = "portal.lifecycle.active_color";
/// Accent shown while the session is attached/ready but not actively producing
/// output (`Attached`). Soft blue: present and reachable.
pub const PORTAL_TOKEN_LIFECYCLE_ATTACHED_COLOR: &str = "portal.lifecycle.attached_color";
/// Accent shown while the session needs attention — degraded or HUD-unavailable
/// (`Degraded` / `HudUnavailable`). Ambient amber: "attention earned" without
/// alarming, consistent with the stale-marker convention.
pub const PORTAL_TOKEN_LIFECYCLE_ATTENTION_COLOR: &str = "portal.lifecycle.attention_color";
/// Accent shown while the session is winding down or gone — detached, cleanup
/// pending, or expired (`Detached` / `CleanupPending` / `Expired`). Muted slate:
/// reads as "inactive" without vanishing.
pub const PORTAL_TOKEN_LIFECYCLE_INACTIVE_COLOR: &str = "portal.lifecycle.inactive_color";
/// Width (px) of the lifecycle affordance accent bar painted along the portal
/// tile's left edge. Geometry only — the accent *color* is token-resolved via the
/// four `portal.lifecycle.*_color` keys above; this token sizes the bar so the
/// adapter/compositor never hardcodes a literal width either (hud-m48i0).
pub const PORTAL_TOKEN_LIFECYCLE_ACCENT_WIDTH_PX: &str = "portal.lifecycle.accent_width_px";

pub const PORTAL_TOKEN_DIVIDER_COLOR: &str = "portal.divider.color";

pub const PORTAL_TOKEN_COLLAPSED_BACKGROUND: &str = "portal.collapsed_card.background";
pub const PORTAL_TOKEN_COLLAPSED_TEXT_COLOR: &str = "portal.collapsed_card.text_color";
pub const PORTAL_TOKEN_COLLAPSED_FONT_SIZE: &str = "portal.collapsed_card.font_size";

pub const PORTAL_TOKEN_TRANSITION_IN_MS: &str = "portal.transition.in_ms";
pub const PORTAL_TOKEN_TRANSITION_OUT_MS: &str = "portal.transition.out_ms";

// ── §6b Window management tokens (amendment 2026-06-10) ──────────────────────

/// Minimum portal width in pixels (legibility bound per §6b.3).
pub const PORTAL_TOKEN_WINDOW_MIN_WIDTH_PX: &str = "portal.window.min_width_px";
/// Minimum portal height in pixels (legibility bound per §6b.3).
pub const PORTAL_TOKEN_WINDOW_MIN_HEIGHT_PX: &str = "portal.window.min_height_px";
/// Pixels per focus-scoped Ctrl+`+`/`=`/`-` resize step (§6b.2).
pub const PORTAL_TOKEN_WINDOW_RESIZE_STEP_PX: &str = "portal.window.resize_step_px";
/// Width/height of the pointer capture region on frame edges/corners (§6b.1).
pub const PORTAL_TOKEN_WINDOW_RESIZE_AFFORDANCE_PX: &str = "portal.window.resize_affordance_px";

// ── §6b Scroll-position indicator tokens ─────────────────────────────────────

/// Thumb color for scroll-position indicator (RGBA hex); redaction-safe (§6b.5).
pub const PORTAL_TOKEN_SCROLL_INDICATOR_COLOR: &str = "portal.scroll_indicator.color";
/// Track width of the scroll-position indicator in px (§6b.5).
pub const PORTAL_TOKEN_SCROLL_INDICATOR_WIDTH_PX: &str = "portal.scroll_indicator.width_px";
/// Minimum thumb height in px — prevents thumb from vanishing on deep content (§6b.5).
pub const PORTAL_TOKEN_SCROLL_INDICATOR_MIN_HEIGHT_PX: &str =
    "portal.scroll_indicator.min_height_px";

// ── Portal token fallback defaults ───────────────────────────────────────────

/// Default values for portal tokens (used when token is absent from resolved map).
///
/// These defaults are deliberately distinct from the 30 canonical tokens so the
/// profile-swap test can distinguish between the canonical and portal layers.
/// Colors use the same palette as the existing exemplar adapter literals,
/// expressed as resolved token defaults rather than inline constants.
///
/// These string constants are the **single source of truth** for the default portal
/// palette. `tze_hud_projection::resident_grpc::PortalVisualTokens::default()` derives
/// from `PortalPartTokens::default()` (which parses these constants) via
/// `portal_visual_tokens_from_part_tokens`, so changing a value here propagates to
/// both sides automatically (hud-dcynv consolidation).
mod defaults {
    pub const FRAME_BACKGROUND: &str = "#111720";
    pub const FRAME_OPACITY: &str = "0.90";
    pub const FRAME_BORDER_COLOR: &str = "#2A3344";

    pub const HEADER_TEXT_COLOR: &str = "#F5F8FF";
    pub const HEADER_FONT_SIZE: &str = "16";

    pub const COMPOSER_BACKGROUND: &str = "#0F1418";
    pub const COMPOSER_TEXT_COLOR: &str = "#E0E8F4";
    pub const COMPOSER_FONT_SIZE: &str = "16";
    /// Muted amber — conveys "limit reached" without alarming the user.
    pub const COMPOSER_AT_CAPACITY_COLOR: &str = "#B87333";

    pub const TRANSCRIPT_BACKGROUND: &str = "#0A0D11";
    pub const TRANSCRIPT_TEXT_COLOR: &str = "#E6EFFA";
    pub const TRANSCRIPT_FONT_SIZE: &str = "16";

    // Degraded / disconnect treatment (§2/§3). Dim text/background read as
    // "inactive" relative to the live transcript palette above; the stale
    // marker uses a muted amber (matching the at-capacity convention) so it is
    // ambient, not alarming.
    pub const TRANSCRIPT_DIM_TEXT_COLOR: &str = "#6B7689";
    pub const TRANSCRIPT_DIM_BACKGROUND: &str = "#07090C";
    pub const STALE_MARKER_COLOR: &str = "#B87333";

    // Lifecycle affordance accents — ambient, mutually distinct (see token-key
    // docs above). Active: calm teal-green; attached/ready: soft blue;
    // attention: amber (distinct from the stale marker); inactive: muted slate.
    pub const LIFECYCLE_ACTIVE_COLOR: &str = "#4FA88A";
    pub const LIFECYCLE_ATTACHED_COLOR: &str = "#5A8FC0";
    pub const LIFECYCLE_ATTENTION_COLOR: &str = "#C28A3D";
    pub const LIFECYCLE_INACTIVE_COLOR: &str = "#5A6373";
    /// Left-edge accent bar width (px). Slim enough to read as a margin marker
    /// (like an editor's gutter indicator) without crowding the transcript text.
    pub const LIFECYCLE_ACCENT_WIDTH_PX: &str = "4";

    pub const DIVIDER_COLOR: &str = "#2A3344";

    pub const COLLAPSED_BACKGROUND: &str = "#1A1F28";
    pub const COLLAPSED_TEXT_COLOR: &str = "#C8D6E8";
    pub const COLLAPSED_FONT_SIZE: &str = "14";

    pub const TRANSITION_IN_MS: &str = "120";
    pub const TRANSITION_OUT_MS: &str = "80";

    // §6b window management defaults
    /// Legible minimum portal width (px). Chosen to fit at least two columns of
    /// readable text at default font size.
    pub const WINDOW_MIN_WIDTH_PX: &str = "240";
    /// Legible minimum portal height (px). Chosen to fit at least three lines of
    /// text plus header and composer.
    pub const WINDOW_MIN_HEIGHT_PX: &str = "160";
    /// Pixels per Ctrl+`+`/`-` resize step. Large enough to feel meaningful,
    /// small enough for fine control.
    pub const WINDOW_RESIZE_STEP_PX: &str = "32";
    /// Width of the pointer capture region on frame edges/corners (px).
    /// 8px balances discoverability with accidental activation avoidance.
    pub const WINDOW_RESIZE_AFFORDANCE_PX: &str = "8";

    // §6b scroll-indicator defaults
    pub const SCROLL_INDICATOR_COLOR: &str = "#4A5568";
    pub const SCROLL_INDICATOR_WIDTH_PX: &str = "4";
    pub const SCROLL_INDICATOR_MIN_HEIGHT_PX: &str = "24";
}

// ── PortalPartTokens ──────────────────────────────────────────────────────────

/// Resolved visual properties for each portal surface part.
///
/// Constructed from a `DesignTokenMap` via [`resolve_portal_tokens`]. Every
/// field is already parsed from its token string representation — the adapter
/// uses these values directly when building scene mutations.
///
/// **No literal colors/sizes are permitted in the adapter publish path.** All
/// visual properties MUST flow through this struct. This is the pre-promotion
/// enforcement of "visual identity is modular" (CLAUDE.md core rule).
#[derive(Clone, Debug, PartialEq)]
pub struct PortalPartTokens {
    // Frame (outer backdrop + border)
    pub frame_background: Rgba,
    pub frame_opacity: f32,
    pub frame_border_color: Rgba,

    // Header strip
    pub header_text_color: Rgba,
    pub header_font_size_px: f32,

    // Composer (input area)
    pub composer_background: Rgba,
    pub composer_text_color: Rgba,
    pub composer_font_size_px: f32,
    /// Indicator color rendered when the draft is at its byte cap (§4.1 / §4.8).
    /// Applied as a distinct visual signal within the composer region; never
    /// hardcoded in the compositor — always token-driven per CLAUDE.md doctrine.
    pub composer_at_capacity_color: Rgba,

    // Transcript body
    pub transcript_background: Rgba,
    pub transcript_text_color: Rgba,
    pub transcript_font_size_px: f32,

    // Degraded / disconnect treatment (portal-disconnect-resume-ux §2/§3).
    /// Dimmed transcript text shown while the portal is disconnected/stale.
    pub transcript_dim_text_color: Rgba,
    /// Dimmed transcript background shown while the portal is disconnected/stale.
    pub transcript_dim_background: Rgba,
    /// Color of the content-free stale/disconnect marker (ambient, not alarming).
    pub stale_marker_color: Rgba,

    // Lifecycle affordance accents (cooperative-hud-projection §lifecycle).
    // Each maps a `ProjectionLifecycleState` group onto an ambient accent; the
    // adapter never hardcodes a lifecycle color.
    /// Accent for the actively-working state (`Active`).
    pub lifecycle_active_color: Rgba,
    /// Accent for the attached/ready state (`Attached`).
    pub lifecycle_attached_color: Rgba,
    /// Accent for attention states (`Degraded` / `HudUnavailable`).
    pub lifecycle_attention_color: Rgba,
    /// Accent for winding-down states (`Detached` / `CleanupPending` / `Expired`).
    pub lifecycle_inactive_color: Rgba,
    /// Width (px) of the left-edge lifecycle accent bar. Geometry only — the bar
    /// *color* comes from the four accents above; this keeps the adapter and
    /// compositor free of any literal accent dimension (hud-m48i0).
    pub lifecycle_accent_width_px: f32,

    // Divider
    pub divider_color: Rgba,

    // Collapsed card
    pub collapsed_background: Rgba,
    pub collapsed_text_color: Rgba,
    pub collapsed_font_size_px: f32,

    // Transitions (zone-transition duration)
    pub transition_in_ms: u32,
    pub transition_out_ms: u32,

    // §6b Window management (amendment 2026-06-10)
    /// Legible minimum portal width in pixels (§6b.3 legibility bound).
    pub window_min_width_px: f32,
    /// Legible minimum portal height in pixels (§6b.3 legibility bound).
    pub window_min_height_px: f32,
    /// Pixels per Ctrl+`+`/`=`/`-` resize step (§6b.2 token-defined step).
    pub window_resize_step_px: f32,
    /// Width/height of pointer capture region on frame edges/corners (§6b.1).
    pub window_resize_affordance_px: f32,

    // §6b Scroll-position indicators (amendment 2026-06-10)
    /// Scroll indicator thumb color — geometry-only, carries no content (§6b.5).
    pub scroll_indicator_color: Rgba,
    /// Scroll indicator track width in px (§6b.5).
    pub scroll_indicator_width_px: f32,
    /// Minimum scroll indicator thumb height in px (§6b.5).
    pub scroll_indicator_min_height_px: f32,
}

impl Default for PortalPartTokens {
    fn default() -> Self {
        Self {
            frame_background: parse_color_hex(defaults::FRAME_BACKGROUND)
                .expect("frame background default is valid hex"),
            frame_opacity: parse_numeric(defaults::FRAME_OPACITY)
                .expect("frame opacity default is valid numeric"),
            frame_border_color: parse_color_hex(defaults::FRAME_BORDER_COLOR)
                .expect("frame border default is valid hex"),

            header_text_color: parse_color_hex(defaults::HEADER_TEXT_COLOR)
                .expect("header text default is valid hex"),
            header_font_size_px: parse_numeric(defaults::HEADER_FONT_SIZE)
                .expect("header font size default is valid numeric"),

            composer_background: parse_color_hex(defaults::COMPOSER_BACKGROUND)
                .expect("composer background default is valid hex"),
            composer_text_color: parse_color_hex(defaults::COMPOSER_TEXT_COLOR)
                .expect("composer text default is valid hex"),
            composer_font_size_px: parse_numeric(defaults::COMPOSER_FONT_SIZE)
                .expect("composer font size default is valid numeric"),
            composer_at_capacity_color: parse_color_hex(defaults::COMPOSER_AT_CAPACITY_COLOR)
                .expect("composer at-capacity color default is valid hex"),

            transcript_background: parse_color_hex(defaults::TRANSCRIPT_BACKGROUND)
                .expect("transcript background default is valid hex"),
            transcript_text_color: parse_color_hex(defaults::TRANSCRIPT_TEXT_COLOR)
                .expect("transcript text default is valid hex"),
            transcript_font_size_px: parse_numeric(defaults::TRANSCRIPT_FONT_SIZE)
                .expect("transcript font size default is valid numeric"),

            transcript_dim_text_color: parse_color_hex(defaults::TRANSCRIPT_DIM_TEXT_COLOR)
                .expect("transcript dim text default is valid hex"),
            transcript_dim_background: parse_color_hex(defaults::TRANSCRIPT_DIM_BACKGROUND)
                .expect("transcript dim background default is valid hex"),
            stale_marker_color: parse_color_hex(defaults::STALE_MARKER_COLOR)
                .expect("stale marker color default is valid hex"),

            lifecycle_active_color: parse_color_hex(defaults::LIFECYCLE_ACTIVE_COLOR)
                .expect("lifecycle active color default is valid hex"),
            lifecycle_attached_color: parse_color_hex(defaults::LIFECYCLE_ATTACHED_COLOR)
                .expect("lifecycle attached color default is valid hex"),
            lifecycle_attention_color: parse_color_hex(defaults::LIFECYCLE_ATTENTION_COLOR)
                .expect("lifecycle attention color default is valid hex"),
            lifecycle_inactive_color: parse_color_hex(defaults::LIFECYCLE_INACTIVE_COLOR)
                .expect("lifecycle inactive color default is valid hex"),
            lifecycle_accent_width_px: parse_numeric(defaults::LIFECYCLE_ACCENT_WIDTH_PX)
                .expect("lifecycle accent width default is valid numeric"),

            divider_color: parse_color_hex(defaults::DIVIDER_COLOR)
                .expect("divider color default is valid hex"),

            collapsed_background: parse_color_hex(defaults::COLLAPSED_BACKGROUND)
                .expect("collapsed background default is valid hex"),
            collapsed_text_color: parse_color_hex(defaults::COLLAPSED_TEXT_COLOR)
                .expect("collapsed text default is valid hex"),
            collapsed_font_size_px: parse_numeric(defaults::COLLAPSED_FONT_SIZE)
                .expect("collapsed font size default is valid numeric"),

            transition_in_ms: parse_numeric(defaults::TRANSITION_IN_MS)
                .expect("transition in default is valid numeric")
                as u32,
            transition_out_ms: parse_numeric(defaults::TRANSITION_OUT_MS)
                .expect("transition out default is valid numeric")
                as u32,

            // §6b window management defaults
            window_min_width_px: parse_numeric(defaults::WINDOW_MIN_WIDTH_PX)
                .expect("window min width default is valid numeric"),
            window_min_height_px: parse_numeric(defaults::WINDOW_MIN_HEIGHT_PX)
                .expect("window min height default is valid numeric"),
            window_resize_step_px: parse_numeric(defaults::WINDOW_RESIZE_STEP_PX)
                .expect("window resize step default is valid numeric"),
            window_resize_affordance_px: parse_numeric(defaults::WINDOW_RESIZE_AFFORDANCE_PX)
                .expect("window resize affordance default is valid numeric"),

            // §6b scroll indicator defaults
            scroll_indicator_color: parse_color_hex(defaults::SCROLL_INDICATOR_COLOR)
                .expect("scroll indicator color default is valid hex"),
            scroll_indicator_width_px: parse_numeric(defaults::SCROLL_INDICATOR_WIDTH_PX)
                .expect("scroll indicator width default is valid numeric"),
            scroll_indicator_min_height_px: parse_numeric(defaults::SCROLL_INDICATOR_MIN_HEIGHT_PX)
                .expect("scroll indicator min height default is valid numeric"),
        }
    }
}

// ── Resolution ────────────────────────────────────────────────────────────────

/// Resolve `PortalPartTokens` from a three-layer resolved design token map.
///
/// Missing or unparseable portal tokens fall back to the hardcoded defaults
/// rather than failing. This matches the portal-scoped override semantics:
/// profile overrides can change any token; absent tokens get defaults.
///
/// # Arguments
///
/// * `token_map` — the fully resolved token map (from `resolve_tokens`);
///   portal-scoped overrides are already merged in at the highest priority.
pub fn resolve_portal_tokens(token_map: &DesignTokenMap) -> PortalPartTokens {
    let defaults = PortalPartTokens::default();

    macro_rules! resolve_color {
        ($key:expr, $fallback:expr) => {
            match token_map.get($key) {
                None => $fallback,
                Some(v) => match parse_color_hex(v) {
                    Some(c) => c,
                    None => {
                        warn!(
                            token_key = $key,
                            bad_value = %v,
                            "portal token color is unparseable; using default fallback",
                        );
                        $fallback
                    }
                },
            }
        };
    }

    macro_rules! resolve_f32 {
        ($key:expr, $fallback:expr) => {
            match token_map.get($key) {
                None => $fallback,
                Some(v) => match parse_numeric(v) {
                    Some(n) => n,
                    None => {
                        warn!(
                            token_key = $key,
                            bad_value = %v,
                            "portal token numeric is unparseable; using default fallback",
                        );
                        $fallback
                    }
                },
            }
        };
    }

    macro_rules! resolve_u32 {
        ($key:expr, $fallback:expr) => {
            match token_map.get($key) {
                None => $fallback,
                Some(v) => {
                    // Require a positive integer string: no negatives, no
                    // decimals, no very-large floats that would overflow u32.
                    // parse_numeric accepts any finite f32 — we add strictness.
                    let parsed = parse_numeric(v).and_then(|n| {
                        if n < 1.0 || n > u32::MAX as f32 || n.fract() != 0.0 {
                            None
                        } else {
                            Some(n as u32)
                        }
                    });
                    match parsed {
                        Some(n) => n,
                        None => {
                            warn!(
                                token_key = $key,
                                bad_value = %v,
                                "portal token u32 is unparseable or out-of-range; \
                                 using default fallback",
                            );
                            $fallback
                        }
                    }
                }
            }
        };
    }

    PortalPartTokens {
        frame_background: resolve_color!(PORTAL_TOKEN_FRAME_BACKGROUND, defaults.frame_background),
        frame_opacity: resolve_f32!(PORTAL_TOKEN_FRAME_OPACITY, defaults.frame_opacity),
        frame_border_color: resolve_color!(
            PORTAL_TOKEN_FRAME_BORDER_COLOR,
            defaults.frame_border_color
        ),

        header_text_color: resolve_color!(
            PORTAL_TOKEN_HEADER_TEXT_COLOR,
            defaults.header_text_color
        ),
        header_font_size_px: resolve_f32!(
            PORTAL_TOKEN_HEADER_FONT_SIZE,
            defaults.header_font_size_px
        ),

        composer_background: resolve_color!(
            PORTAL_TOKEN_COMPOSER_BACKGROUND,
            defaults.composer_background
        ),
        composer_text_color: resolve_color!(
            PORTAL_TOKEN_COMPOSER_TEXT_COLOR,
            defaults.composer_text_color
        ),
        composer_font_size_px: resolve_f32!(
            PORTAL_TOKEN_COMPOSER_FONT_SIZE,
            defaults.composer_font_size_px
        ),
        composer_at_capacity_color: resolve_color!(
            PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR,
            defaults.composer_at_capacity_color
        ),

        transcript_background: resolve_color!(
            PORTAL_TOKEN_TRANSCRIPT_BACKGROUND,
            defaults.transcript_background
        ),
        transcript_text_color: resolve_color!(
            PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR,
            defaults.transcript_text_color
        ),
        transcript_font_size_px: resolve_f32!(
            PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE,
            defaults.transcript_font_size_px
        ),

        transcript_dim_text_color: resolve_color!(
            PORTAL_TOKEN_TRANSCRIPT_DIM_TEXT_COLOR,
            defaults.transcript_dim_text_color
        ),
        transcript_dim_background: resolve_color!(
            PORTAL_TOKEN_TRANSCRIPT_DIM_BACKGROUND,
            defaults.transcript_dim_background
        ),
        stale_marker_color: resolve_color!(
            PORTAL_TOKEN_STALE_MARKER_COLOR,
            defaults.stale_marker_color
        ),

        lifecycle_active_color: resolve_color!(
            PORTAL_TOKEN_LIFECYCLE_ACTIVE_COLOR,
            defaults.lifecycle_active_color
        ),
        lifecycle_attached_color: resolve_color!(
            PORTAL_TOKEN_LIFECYCLE_ATTACHED_COLOR,
            defaults.lifecycle_attached_color
        ),
        lifecycle_attention_color: resolve_color!(
            PORTAL_TOKEN_LIFECYCLE_ATTENTION_COLOR,
            defaults.lifecycle_attention_color
        ),
        lifecycle_inactive_color: resolve_color!(
            PORTAL_TOKEN_LIFECYCLE_INACTIVE_COLOR,
            defaults.lifecycle_inactive_color
        ),
        lifecycle_accent_width_px: resolve_f32!(
            PORTAL_TOKEN_LIFECYCLE_ACCENT_WIDTH_PX,
            defaults.lifecycle_accent_width_px
        ),

        divider_color: resolve_color!(PORTAL_TOKEN_DIVIDER_COLOR, defaults.divider_color),

        collapsed_background: resolve_color!(
            PORTAL_TOKEN_COLLAPSED_BACKGROUND,
            defaults.collapsed_background
        ),
        collapsed_text_color: resolve_color!(
            PORTAL_TOKEN_COLLAPSED_TEXT_COLOR,
            defaults.collapsed_text_color
        ),
        collapsed_font_size_px: resolve_f32!(
            PORTAL_TOKEN_COLLAPSED_FONT_SIZE,
            defaults.collapsed_font_size_px
        ),

        transition_in_ms: resolve_u32!(PORTAL_TOKEN_TRANSITION_IN_MS, defaults.transition_in_ms),
        transition_out_ms: resolve_u32!(PORTAL_TOKEN_TRANSITION_OUT_MS, defaults.transition_out_ms),

        // §6b window management
        window_min_width_px: resolve_f32!(
            PORTAL_TOKEN_WINDOW_MIN_WIDTH_PX,
            defaults.window_min_width_px
        ),
        window_min_height_px: resolve_f32!(
            PORTAL_TOKEN_WINDOW_MIN_HEIGHT_PX,
            defaults.window_min_height_px
        ),
        window_resize_step_px: resolve_f32!(
            PORTAL_TOKEN_WINDOW_RESIZE_STEP_PX,
            defaults.window_resize_step_px
        ),
        window_resize_affordance_px: resolve_f32!(
            PORTAL_TOKEN_WINDOW_RESIZE_AFFORDANCE_PX,
            defaults.window_resize_affordance_px
        ),

        // §6b scroll indicators
        scroll_indicator_color: resolve_color!(
            PORTAL_TOKEN_SCROLL_INDICATOR_COLOR,
            defaults.scroll_indicator_color
        ),
        scroll_indicator_width_px: resolve_f32!(
            PORTAL_TOKEN_SCROLL_INDICATOR_WIDTH_PX,
            defaults.scroll_indicator_width_px
        ),
        scroll_indicator_min_height_px: resolve_f32!(
            PORTAL_TOKEN_SCROLL_INDICATOR_MIN_HEIGHT_PX,
            defaults.scroll_indicator_min_height_px
        ),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{DesignTokenMap, resolve_tokens};

    fn empty_map() -> DesignTokenMap {
        DesignTokenMap::new()
    }

    // ── Default fallback resolution ───────────────────────────────────────

    #[test]
    fn resolve_portal_tokens_defaults_on_empty_map() {
        let tokens = resolve_portal_tokens(&empty_map());
        let defaults = PortalPartTokens::default();
        // Spot-check a selection of fields
        assert_eq!(tokens.frame_opacity, defaults.frame_opacity);
        assert_eq!(tokens.header_font_size_px, defaults.header_font_size_px);
        assert_eq!(tokens.transition_in_ms, defaults.transition_in_ms);
        assert_eq!(tokens.transition_out_ms, defaults.transition_out_ms);
    }

    #[test]
    fn resolve_portal_tokens_all_fields_populated() {
        let tokens = resolve_portal_tokens(&empty_map());
        // Every f32 field must be finite and positive
        assert!(tokens.frame_opacity > 0.0 && tokens.frame_opacity <= 1.0);
        assert!(tokens.header_font_size_px > 0.0);
        assert!(tokens.composer_font_size_px > 0.0);
        assert!(tokens.transcript_font_size_px > 0.0);
        assert!(tokens.collapsed_font_size_px > 0.0);
        assert!(tokens.transition_in_ms > 0);
        assert!(tokens.transition_out_ms > 0);
        // Composer at-capacity color must have a non-zero alpha (visible indicator)
        assert!(
            tokens.composer_at_capacity_color.a > 0.0,
            "at-capacity color must have non-zero alpha so it is visible"
        );
        // §6b window management fields
        assert!(tokens.window_min_width_px > 0.0);
        assert!(tokens.window_min_height_px > 0.0);
        assert!(tokens.window_resize_step_px > 0.0);
        assert!(tokens.window_resize_affordance_px > 0.0);
        // §6b scroll indicator fields
        assert!(tokens.scroll_indicator_width_px > 0.0);
        assert!(tokens.scroll_indicator_min_height_px > 0.0);
    }

    #[test]
    fn default_portal_text_sizes_are_readable_without_focus_resize() {
        let tokens = resolve_portal_tokens(&empty_map());

        assert!(
            tokens.header_font_size_px >= 16.0,
            "header default font must be comfortably readable without hotkey resize; got {}px",
            tokens.header_font_size_px
        );
        assert!(
            tokens.transcript_font_size_px >= 16.0,
            "transcript default font must be comfortably readable without hotkey resize; got {}px",
            tokens.transcript_font_size_px
        );
        assert!(
            tokens.composer_font_size_px >= 16.0,
            "composer default font must be comfortably readable without hotkey resize; got {}px",
            tokens.composer_font_size_px
        );
        assert!(
            tokens.collapsed_font_size_px >= 14.0,
            "collapsed-card default font must remain readable in compact mode; got {}px",
            tokens.collapsed_font_size_px
        );
    }

    // ── Profile-scoped override propagation ──────────────────────────────

    #[test]
    fn profile_override_propagates_to_portal_tokens() {
        // Verify that a profile-scoped override for portal.transcript.text_color
        // propagates through resolve_portal_tokens — this is the pre-promotion
        // §6.1 contract: token change → portal reskin, no adapter logic change.
        let mut overrides = DesignTokenMap::new();
        overrides.insert(
            PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR.to_string(),
            "#FF00FF".to_string(), // magenta sentinel
        );
        let resolved = resolve_tokens(&empty_map(), &overrides);
        let tokens = resolve_portal_tokens(&resolved);

        assert!(
            (tokens.transcript_text_color.r - 1.0).abs() < 1e-3,
            "overridden r must be 1.0 (FF)"
        );
        assert!(
            tokens.transcript_text_color.g.abs() < 1e-3,
            "overridden g must be 0.0 (00)"
        );
        assert!(
            (tokens.transcript_text_color.b - 1.0).abs() < 1e-3,
            "overridden b must be 1.0 (FF)"
        );
    }

    #[test]
    fn profile_override_changes_frame_opacity() {
        let mut overrides = DesignTokenMap::new();
        overrides.insert(PORTAL_TOKEN_FRAME_OPACITY.to_string(), "0.5".to_string());
        let resolved = resolve_tokens(&empty_map(), &overrides);
        let tokens = resolve_portal_tokens(&resolved);
        assert!((tokens.frame_opacity - 0.5).abs() < 1e-4);
    }

    #[test]
    fn profile_override_changes_transition_ms() {
        let mut overrides = DesignTokenMap::new();
        overrides.insert(PORTAL_TOKEN_TRANSITION_IN_MS.to_string(), "250".to_string());
        overrides.insert(
            PORTAL_TOKEN_TRANSITION_OUT_MS.to_string(),
            "150".to_string(),
        );
        let resolved = resolve_tokens(&empty_map(), &overrides);
        let tokens = resolve_portal_tokens(&resolved);
        assert_eq!(tokens.transition_in_ms, 250);
        assert_eq!(tokens.transition_out_ms, 150);
    }

    // ── Profile-swap reskin (§6.4 core scenario) ─────────────────────────

    /// Profile swap reskins portal without adapter logic change.
    ///
    /// Demonstrates §6.1: a profile change propagates to all portal parts
    /// through `resolve_portal_tokens`, with zero adapter code changes.
    /// The "adapter logic change" is defined as changing the code path that
    /// calls `resolve_portal_tokens` — here we prove that only token values
    /// change across profiles, never the calling code.
    #[test]
    fn profile_swap_reskins_all_portal_parts() {
        // Profile A: dark theme (defaults)
        let profile_a_tokens = resolve_portal_tokens(&empty_map());

        // Profile B: custom theme (all portal parts overridden)
        let mut profile_b_overrides = DesignTokenMap::new();
        profile_b_overrides.insert(
            PORTAL_TOKEN_FRAME_BACKGROUND.to_string(),
            "#FFFFFF".to_string(), // white
        );
        profile_b_overrides.insert(PORTAL_TOKEN_FRAME_OPACITY.to_string(), "1.0".to_string());
        profile_b_overrides.insert(
            PORTAL_TOKEN_HEADER_TEXT_COLOR.to_string(),
            "#000000".to_string(), // black
        );
        profile_b_overrides.insert(PORTAL_TOKEN_HEADER_FONT_SIZE.to_string(), "18".to_string());
        profile_b_overrides.insert(
            PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR.to_string(),
            "#333333".to_string(),
        );
        profile_b_overrides.insert(
            PORTAL_TOKEN_TRANSCRIPT_BACKGROUND.to_string(),
            "#F5F5F5".to_string(),
        );
        profile_b_overrides.insert(
            PORTAL_TOKEN_COLLAPSED_BACKGROUND.to_string(),
            "#EEEEEE".to_string(),
        );
        profile_b_overrides.insert(
            PORTAL_TOKEN_DIVIDER_COLOR.to_string(),
            "#CCCCCC".to_string(),
        );

        let resolved_b = resolve_tokens(&empty_map(), &profile_b_overrides);
        let profile_b_tokens = resolve_portal_tokens(&resolved_b);

        // Frame background must differ (white vs dark)
        assert_ne!(
            profile_a_tokens.frame_background, profile_b_tokens.frame_background,
            "profile swap must change frame background"
        );

        // Header text color must differ (black vs near-white)
        assert_ne!(
            profile_a_tokens.header_text_color, profile_b_tokens.header_text_color,
            "profile swap must change header text color"
        );

        // Header font size must differ
        assert!(
            (profile_b_tokens.header_font_size_px - 18.0).abs() < 1e-4,
            "profile B header font size must be 18px"
        );
        assert!(
            (profile_a_tokens.header_font_size_px - 18.0).abs() > 1e-1,
            "profile A header font size must differ from 18px"
        );

        // Transcript background must differ
        assert_ne!(
            profile_a_tokens.transcript_background, profile_b_tokens.transcript_background,
            "profile swap must change transcript background"
        );

        // Collapsed background must differ
        assert_ne!(
            profile_a_tokens.collapsed_background, profile_b_tokens.collapsed_background,
            "profile swap must change collapsed background"
        );

        // Divider color must differ
        assert_ne!(
            profile_a_tokens.divider_color, profile_b_tokens.divider_color,
            "profile swap must change divider color"
        );
    }

    // ── Token propagation on republish (§6.4) ────────────────────────────

    /// Verifies that a token value change propagates through the portal token
    /// map on every republish without requiring any adapter code change.
    /// "Republish" here is represented by resolving the token map a second time.
    #[test]
    fn token_change_propagates_on_republish() {
        // First publish cycle: default tokens
        let first = resolve_portal_tokens(&empty_map());

        // Token change (simulate profile hot-reload changing transcript background)
        let mut new_overrides = DesignTokenMap::new();
        new_overrides.insert(
            PORTAL_TOKEN_TRANSCRIPT_BACKGROUND.to_string(),
            "#2A4080".to_string(), // navy blue
        );
        let new_map = resolve_tokens(&empty_map(), &new_overrides);

        // Second publish cycle: updated tokens
        let second = resolve_portal_tokens(&new_map);

        // The token change must propagate
        assert_ne!(
            first.transcript_background, second.transcript_background,
            "token change must propagate to republish"
        );

        // All other fields must be unchanged (only transcript background changed)
        assert_eq!(
            first.frame_background, second.frame_background,
            "unmodified tokens must stay the same after partial update"
        );
        assert_eq!(
            first.header_text_color, second.header_text_color,
            "unmodified tokens must stay the same after partial update"
        );
    }

    // ── Unparseable token fallback ────────────────────────────────────────

    #[test]
    fn unparseable_token_falls_back_to_default() {
        let mut bad_overrides = DesignTokenMap::new();
        // Inject an invalid color for a portal token key
        bad_overrides.insert(
            PORTAL_TOKEN_FRAME_BACKGROUND.to_string(),
            "not-a-hex-color".to_string(),
        );
        bad_overrides.insert(
            PORTAL_TOKEN_FRAME_OPACITY.to_string(),
            "not-a-number".to_string(),
        );
        let resolved = resolve_tokens(&empty_map(), &bad_overrides);
        let tokens = resolve_portal_tokens(&resolved);
        let defaults = PortalPartTokens::default();

        // Must fall back to defaults, not panic
        assert_eq!(
            tokens.frame_background, defaults.frame_background,
            "unparseable color must fall back to default"
        );
        assert_eq!(
            tokens.frame_opacity, defaults.frame_opacity,
            "unparseable numeric must fall back to default"
        );
    }

    // ── resolve_u32 validation ────────────────────────────────────────────

    /// Verifies that resolve_u32 rejects invalid transition duration values and
    /// falls back to defaults. Invalid values include negatives (which would cast
    /// to 0 via `as u32`), decimals, and excessively large floats.
    #[test]
    fn invalid_transition_ms_falls_back_to_default() {
        let defaults = PortalPartTokens::default();

        // Negative value → fallback (0 would violate the > 0 invariant)
        let mut bad = DesignTokenMap::new();
        bad.insert(PORTAL_TOKEN_TRANSITION_IN_MS.to_string(), "-1".to_string());
        let resolved = resolve_tokens(&empty_map(), &bad);
        let tokens = resolve_portal_tokens(&resolved);
        assert_eq!(
            tokens.transition_in_ms, defaults.transition_in_ms,
            "negative transition_in_ms must fall back to default"
        );

        // Decimal value → fallback
        let mut bad2 = DesignTokenMap::new();
        bad2.insert(
            PORTAL_TOKEN_TRANSITION_OUT_MS.to_string(),
            "0.5".to_string(),
        );
        let resolved2 = resolve_tokens(&empty_map(), &bad2);
        let tokens2 = resolve_portal_tokens(&resolved2);
        assert_eq!(
            tokens2.transition_out_ms, defaults.transition_out_ms,
            "decimal transition_out_ms must fall back to default"
        );

        // Zero value → fallback (> 0 invariant)
        let mut bad3 = DesignTokenMap::new();
        bad3.insert(PORTAL_TOKEN_TRANSITION_IN_MS.to_string(), "0".to_string());
        let resolved3 = resolve_tokens(&empty_map(), &bad3);
        let tokens3 = resolve_portal_tokens(&resolved3);
        assert_eq!(
            tokens3.transition_in_ms, defaults.transition_in_ms,
            "zero transition_in_ms must fall back to default"
        );
    }

    // ── §6b window management token tests ────────────────────────────────

    #[test]
    fn window_management_tokens_default_values_are_valid() {
        let tokens = resolve_portal_tokens(&empty_map());
        // Defaults must be positive
        assert!(
            tokens.window_min_width_px > 0.0,
            "window_min_width_px must be positive"
        );
        assert!(
            tokens.window_min_height_px > 0.0,
            "window_min_height_px must be positive"
        );
        assert!(
            tokens.window_resize_step_px > 0.0,
            "window_resize_step_px must be positive"
        );
        assert!(
            tokens.window_resize_affordance_px > 0.0,
            "window_resize_affordance_px must be positive"
        );
        // Defaults must satisfy basic legibility invariant: min size > affordance
        assert!(
            tokens.window_min_width_px > tokens.window_resize_affordance_px,
            "min width must be larger than the affordance region"
        );
        assert!(
            tokens.window_min_height_px > tokens.window_resize_affordance_px,
            "min height must be larger than the affordance region"
        );
    }

    #[test]
    fn window_management_tokens_override_propagates() {
        let mut overrides = DesignTokenMap::new();
        overrides.insert(
            PORTAL_TOKEN_WINDOW_MIN_WIDTH_PX.to_string(),
            "320".to_string(),
        );
        overrides.insert(
            PORTAL_TOKEN_WINDOW_RESIZE_STEP_PX.to_string(),
            "16".to_string(),
        );
        overrides.insert(
            PORTAL_TOKEN_WINDOW_RESIZE_AFFORDANCE_PX.to_string(),
            "12".to_string(),
        );
        let resolved = resolve_tokens(&empty_map(), &overrides);
        let tokens = resolve_portal_tokens(&resolved);

        assert!(
            (tokens.window_min_width_px - 320.0).abs() < 1e-4,
            "window_min_width_px override must propagate"
        );
        assert!(
            (tokens.window_resize_step_px - 16.0).abs() < 1e-4,
            "window_resize_step_px override must propagate"
        );
        assert!(
            (tokens.window_resize_affordance_px - 12.0).abs() < 1e-4,
            "window_resize_affordance_px override must propagate"
        );
    }

    // ── §6b scroll indicator token tests ─────────────────────────────────

    #[test]
    fn scroll_indicator_tokens_default_values_are_valid() {
        let tokens = resolve_portal_tokens(&empty_map());
        assert!(
            tokens.scroll_indicator_width_px > 0.0,
            "scroll_indicator_width_px must be positive"
        );
        assert!(
            tokens.scroll_indicator_min_height_px > 0.0,
            "scroll_indicator_min_height_px must be positive"
        );
    }

    #[test]
    fn scroll_indicator_color_override_propagates() {
        let mut overrides = DesignTokenMap::new();
        overrides.insert(
            PORTAL_TOKEN_SCROLL_INDICATOR_COLOR.to_string(),
            "#FF8800".to_string(), // orange sentinel
        );
        let resolved = resolve_tokens(&empty_map(), &overrides);
        let tokens = resolve_portal_tokens(&resolved);

        // r=1.0, g=0x88/0xFF≈0.533, b=0.0
        assert!(
            (tokens.scroll_indicator_color.r - 1.0).abs() < 1e-2,
            "scroll indicator color red channel must match override"
        );
        assert!(
            tokens.scroll_indicator_color.b.abs() < 1e-2,
            "scroll indicator color blue channel must be 0"
        );
    }

    /// The lifecycle accent-bar width resolves to a positive default and a
    /// profile-scoped override propagates — the bar geometry is token-driven so
    /// neither the adapter nor the compositor hardcodes the accent dimension
    /// (hud-m48i0).
    #[test]
    fn lifecycle_accent_width_default_and_override() {
        let defaults = resolve_portal_tokens(&empty_map());
        assert!(
            defaults.lifecycle_accent_width_px > 0.0,
            "lifecycle accent bar must have a positive default width so it is visible"
        );

        let mut overrides = DesignTokenMap::new();
        overrides.insert(
            PORTAL_TOKEN_LIFECYCLE_ACCENT_WIDTH_PX.to_string(),
            "9".to_string(),
        );
        let resolved = resolve_tokens(&empty_map(), &overrides);
        let tokens = resolve_portal_tokens(&resolved);
        assert!(
            (tokens.lifecycle_accent_width_px - 9.0).abs() < 1e-4,
            "lifecycle accent width override must propagate"
        );
    }

    // ── §2/§3 degraded / disconnect token tests ─────────────────────────

    /// Degraded-treatment tokens resolve to valid, visible colors and read as
    /// distinctly dimmer than the live transcript palette (spec §2: the retained
    /// window is dimmed rather than blanked).
    #[test]
    fn degraded_tokens_default_values_are_valid() {
        let tokens = resolve_portal_tokens(&empty_map());
        // Stale marker must be visible (non-zero alpha) so the disconnect
        // affordance is actually shown.
        assert!(
            tokens.stale_marker_color.a > 0.0,
            "stale marker color must have non-zero alpha so it is visible"
        );
        // Dim treatment must differ from the live transcript palette — otherwise
        // a disconnected portal would be indistinguishable from a live one.
        assert_ne!(
            tokens.transcript_dim_text_color, tokens.transcript_text_color,
            "dim text color must differ from live transcript text color"
        );
        assert_ne!(
            tokens.transcript_dim_background, tokens.transcript_background,
            "dim background must differ from live transcript background"
        );
    }

    /// Profile-scoped overrides for the degraded tokens propagate through
    /// `resolve_portal_tokens` (spec §2: degraded treatment is token-resolved,
    /// not hardcoded — a profile change reskins it with no adapter change).
    #[test]
    fn degraded_token_overrides_propagate() {
        let mut overrides = DesignTokenMap::new();
        overrides.insert(
            PORTAL_TOKEN_TRANSCRIPT_DIM_TEXT_COLOR.to_string(),
            "#FF00FF".to_string(), // magenta sentinel
        );
        overrides.insert(
            PORTAL_TOKEN_STALE_MARKER_COLOR.to_string(),
            "#00FF00".to_string(), // green sentinel
        );
        let resolved = resolve_tokens(&empty_map(), &overrides);
        let tokens = resolve_portal_tokens(&resolved);

        assert!(
            (tokens.transcript_dim_text_color.r - 1.0).abs() < 1e-3
                && tokens.transcript_dim_text_color.g.abs() < 1e-3
                && (tokens.transcript_dim_text_color.b - 1.0).abs() < 1e-3,
            "dim text color override must propagate (magenta)"
        );
        assert!(
            tokens.stale_marker_color.r.abs() < 1e-3
                && (tokens.stale_marker_color.g - 1.0).abs() < 1e-3
                && tokens.stale_marker_color.b.abs() < 1e-3,
            "stale marker color override must propagate (green)"
        );
    }

    // ── Diagnostic warn path (hud-dcynv) ─────────────────────────────────

    /// Verifies that a present-but-unparseable token (color) falls back to the
    /// default value and does NOT panic. The `tracing::warn!` is emitted on the
    /// same code path, but subscriber capture requires the `tracing_test` crate
    /// which is not in this workspace. The behavioral invariant (fallback used)
    /// is sufficient to assert the warn code path was reached.
    #[test]
    fn unparseable_color_token_triggers_fallback_and_warn_path() {
        // ALL color-bearing token keys injected with bad values.
        let mut bad = DesignTokenMap::new();
        for key in [
            PORTAL_TOKEN_FRAME_BACKGROUND,
            PORTAL_TOKEN_FRAME_BORDER_COLOR,
            PORTAL_TOKEN_HEADER_TEXT_COLOR,
            PORTAL_TOKEN_COMPOSER_BACKGROUND,
            PORTAL_TOKEN_COMPOSER_TEXT_COLOR,
            PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR,
            PORTAL_TOKEN_TRANSCRIPT_BACKGROUND,
            PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR,
            PORTAL_TOKEN_TRANSCRIPT_DIM_TEXT_COLOR,
            PORTAL_TOKEN_TRANSCRIPT_DIM_BACKGROUND,
            PORTAL_TOKEN_STALE_MARKER_COLOR,
            PORTAL_TOKEN_DIVIDER_COLOR,
            PORTAL_TOKEN_COLLAPSED_BACKGROUND,
            PORTAL_TOKEN_COLLAPSED_TEXT_COLOR,
            PORTAL_TOKEN_SCROLL_INDICATOR_COLOR,
        ] {
            bad.insert(key.to_string(), "!!not-hex!!".to_string());
        }
        let resolved = resolve_tokens(&empty_map(), &bad);
        let tokens = resolve_portal_tokens(&resolved);
        let defaults = PortalPartTokens::default();

        // Every color field must fall back to the canonical default.
        assert_eq!(
            tokens.frame_background, defaults.frame_background,
            "bad frame_background must fall back to default"
        );
        assert_eq!(
            tokens.frame_border_color, defaults.frame_border_color,
            "bad frame_border_color must fall back to default"
        );
        assert_eq!(
            tokens.header_text_color, defaults.header_text_color,
            "bad header_text_color must fall back to default"
        );
        assert_eq!(
            tokens.composer_background, defaults.composer_background,
            "bad composer_background must fall back to default"
        );
        assert_eq!(
            tokens.transcript_background, defaults.transcript_background,
            "bad transcript_background must fall back to default"
        );
        assert_eq!(
            tokens.collapsed_background, defaults.collapsed_background,
            "bad collapsed_background must fall back to default"
        );
        assert_eq!(
            tokens.scroll_indicator_color, defaults.scroll_indicator_color,
            "bad scroll_indicator_color must fall back to default"
        );
    }

    /// Verifies that a present-but-unparseable numeric token falls back to the
    /// default value. The warn is emitted on the same code path.
    #[test]
    fn unparseable_numeric_token_triggers_fallback_and_warn_path() {
        let mut bad = DesignTokenMap::new();
        for key in [
            PORTAL_TOKEN_FRAME_OPACITY,
            PORTAL_TOKEN_HEADER_FONT_SIZE,
            PORTAL_TOKEN_COMPOSER_FONT_SIZE,
            PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE,
            PORTAL_TOKEN_COLLAPSED_FONT_SIZE,
            PORTAL_TOKEN_WINDOW_MIN_WIDTH_PX,
            PORTAL_TOKEN_WINDOW_MIN_HEIGHT_PX,
            PORTAL_TOKEN_WINDOW_RESIZE_STEP_PX,
            PORTAL_TOKEN_WINDOW_RESIZE_AFFORDANCE_PX,
            PORTAL_TOKEN_SCROLL_INDICATOR_WIDTH_PX,
            PORTAL_TOKEN_SCROLL_INDICATOR_MIN_HEIGHT_PX,
        ] {
            bad.insert(key.to_string(), "definitely-not-a-number".to_string());
        }
        let resolved = resolve_tokens(&empty_map(), &bad);
        let tokens = resolve_portal_tokens(&resolved);
        let defaults = PortalPartTokens::default();

        assert!(
            (tokens.frame_opacity - defaults.frame_opacity).abs() < 1e-6,
            "bad frame_opacity must fall back to default"
        );
        assert!(
            (tokens.header_font_size_px - defaults.header_font_size_px).abs() < 1e-6,
            "bad header_font_size_px must fall back to default"
        );
        assert!(
            (tokens.window_min_width_px - defaults.window_min_width_px).abs() < 1e-6,
            "bad window_min_width_px must fall back to default"
        );
        assert!(
            (tokens.scroll_indicator_width_px - defaults.scroll_indicator_width_px).abs() < 1e-6,
            "bad scroll_indicator_width_px must fall back to default"
        );
    }

    /// Verifies that a present-but-invalid u32 token (negative / decimal / zero)
    /// falls back to the default value. The warn is emitted on the same code path.
    #[test]
    fn invalid_u32_token_triggers_fallback_and_warn_path() {
        let defaults = PortalPartTokens::default();

        for bad_value in ["-5", "0", "0.5", "1.9", "not-a-number"] {
            let mut bad = DesignTokenMap::new();
            bad.insert(
                PORTAL_TOKEN_TRANSITION_IN_MS.to_string(),
                bad_value.to_string(),
            );
            bad.insert(
                PORTAL_TOKEN_TRANSITION_OUT_MS.to_string(),
                bad_value.to_string(),
            );
            let resolved = resolve_tokens(&empty_map(), &bad);
            let tokens = resolve_portal_tokens(&resolved);

            assert_eq!(
                tokens.transition_in_ms, defaults.transition_in_ms,
                "bad transition_in_ms ({bad_value:?}) must fall back to default"
            );
            assert_eq!(
                tokens.transition_out_ms, defaults.transition_out_ms,
                "bad transition_out_ms ({bad_value:?}) must fall back to default"
            );
        }
    }
}
