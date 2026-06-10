//! Portal part inventory and token mapping for the text-stream portal pilot.
//!
//! Implements §6.2 of `text-stream-portal-phase1/tasks.md`:
//! - Portal part inventory (frame, header, composer, transcript body, divider,
//!   collapsed card)
//! - Token mapping each part consumes
//! - `PortalPartTokens`: resolved visual values extracted from a `DesignTokenMap`
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
//! | `portal.divider.color` | divider | separator line color (RGBA hex) |
//! | `portal.collapsed_card.background` | collapsed card | compact view backdrop (RGBA hex) |
//! | `portal.collapsed_card.text_color` | collapsed card | compact text color (RGBA hex) |
//! | `portal.collapsed_card.font_size` | collapsed card | compact text font size in px |
//! | `portal.transition.in_ms` | transitions | collapsed→expanded duration (ms) |
//! | `portal.transition.out_ms` | transitions | expanded→collapsed duration (ms) |

use crate::tokens::{DesignTokenMap, Rgba, parse_color_hex, parse_numeric};

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

pub const PORTAL_TOKEN_TRANSCRIPT_BACKGROUND: &str = "portal.transcript.background";
pub const PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR: &str = "portal.transcript.text_color";
pub const PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE: &str = "portal.transcript.font_size";

pub const PORTAL_TOKEN_DIVIDER_COLOR: &str = "portal.divider.color";

pub const PORTAL_TOKEN_COLLAPSED_BACKGROUND: &str = "portal.collapsed_card.background";
pub const PORTAL_TOKEN_COLLAPSED_TEXT_COLOR: &str = "portal.collapsed_card.text_color";
pub const PORTAL_TOKEN_COLLAPSED_FONT_SIZE: &str = "portal.collapsed_card.font_size";

pub const PORTAL_TOKEN_TRANSITION_IN_MS: &str = "portal.transition.in_ms";
pub const PORTAL_TOKEN_TRANSITION_OUT_MS: &str = "portal.transition.out_ms";

// ── Portal token fallback defaults ───────────────────────────────────────────

/// Default values for portal tokens (used when token is absent from resolved map).
///
/// These defaults are deliberately distinct from the 30 canonical tokens so the
/// profile-swap test can distinguish between the canonical and portal layers.
/// Colors use the same palette as the existing exemplar adapter literals,
/// expressed as resolved token defaults rather than inline constants.
///
/// NOTE: The numeric defaults here (as strings) must match the float/integer
/// defaults in `tze_hud_projection::resident_grpc::PortalVisualTokens::default`.
/// There is no compile-time link (the crates are independent), so update both
/// sides if you change any default value.
mod defaults {
    pub const FRAME_BACKGROUND: &str = "#111720";
    pub const FRAME_OPACITY: &str = "0.90";
    pub const FRAME_BORDER_COLOR: &str = "#2A3344";

    pub const HEADER_TEXT_COLOR: &str = "#F5F8FF";
    pub const HEADER_FONT_SIZE: &str = "14";

    pub const COMPOSER_BACKGROUND: &str = "#0F1418";
    pub const COMPOSER_TEXT_COLOR: &str = "#E0E8F4";
    pub const COMPOSER_FONT_SIZE: &str = "13";

    pub const TRANSCRIPT_BACKGROUND: &str = "#0A0D11";
    pub const TRANSCRIPT_TEXT_COLOR: &str = "#E6EFFA";
    pub const TRANSCRIPT_FONT_SIZE: &str = "13";

    pub const DIVIDER_COLOR: &str = "#2A3344";

    pub const COLLAPSED_BACKGROUND: &str = "#1A1F28";
    pub const COLLAPSED_TEXT_COLOR: &str = "#C8D6E8";
    pub const COLLAPSED_FONT_SIZE: &str = "12";

    pub const TRANSITION_IN_MS: &str = "120";
    pub const TRANSITION_OUT_MS: &str = "80";
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

    // Transcript body
    pub transcript_background: Rgba,
    pub transcript_text_color: Rgba,
    pub transcript_font_size_px: f32,

    // Divider
    pub divider_color: Rgba,

    // Collapsed card
    pub collapsed_background: Rgba,
    pub collapsed_text_color: Rgba,
    pub collapsed_font_size_px: f32,

    // Transitions (zone-transition duration)
    pub transition_in_ms: u32,
    pub transition_out_ms: u32,
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

            transcript_background: parse_color_hex(defaults::TRANSCRIPT_BACKGROUND)
                .expect("transcript background default is valid hex"),
            transcript_text_color: parse_color_hex(defaults::TRANSCRIPT_TEXT_COLOR)
                .expect("transcript text default is valid hex"),
            transcript_font_size_px: parse_numeric(defaults::TRANSCRIPT_FONT_SIZE)
                .expect("transcript font size default is valid numeric"),

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
            token_map
                .get($key)
                .and_then(|v| parse_color_hex(v))
                .unwrap_or($fallback)
        };
    }

    macro_rules! resolve_f32 {
        ($key:expr, $fallback:expr) => {
            token_map
                .get($key)
                .and_then(|v| parse_numeric(v))
                .unwrap_or($fallback)
        };
    }

    macro_rules! resolve_u32 {
        ($key:expr, $fallback:expr) => {
            token_map
                .get($key)
                .and_then(|v| {
                    // Require a positive integer string: no negatives, no
                    // decimals, no very-large floats that would overflow u32.
                    // parse_numeric accepts any finite f32 — we add strictness.
                    let n = parse_numeric(v)?;
                    if n < 1.0 || n > u32::MAX as f32 || n.fract() != 0.0 {
                        return None;
                    }
                    Some(n as u32)
                })
                .unwrap_or($fallback)
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
}
