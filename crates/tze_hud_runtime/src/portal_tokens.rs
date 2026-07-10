//! Production bridge: `PortalPartTokens` → `PortalVisualTokens`.
//!
//! The canonical conversion function lives in `tze_hud_projection::resident_grpc`
//! so that it can be used directly from the projection authority binary without
//! pulling in the full runtime crate (which would create a circular dependency:
//! `tze_hud_runtime` already depends on `tze_hud_projection`).
//!
//! This module re-exports `portal_visual_tokens_from_part_tokens` for consumers
//! that import from `tze_hud_runtime`.
//!
//! ## Production wiring contract
//!
//! Any code that constructs a `ResidentGrpcPortalAdapter` MUST call
//! `portal_visual_tokens_from_part_tokens` instead of hand-constructing
//! `PortalVisualTokens`. When a token-map swap occurs (e.g. profile hot-reload
//! via `compositor.set_token_map`), call `resolve_portal_tokens` on the new
//! `DesignTokenMap` to get `PortalPartTokens`, then pass the result to this
//! function, and forward the resulting `PortalVisualTokens` to
//! `adapter.set_visual_tokens(...)`.
//!
//! ```rust,ignore
//! use tze_hud_config::{resolve_portal_tokens, tokens::DesignTokenMap};
//! use tze_hud_projection::resident_grpc::{ResidentGrpcPortalAdapter, ResidentGrpcPortalConfig};
//! use tze_hud_runtime::portal_tokens::portal_visual_tokens_from_part_tokens;
//!
//! // At adapter construction:
//! let part_tokens = resolve_portal_tokens(&resolved_token_map);
//! let visual_tokens = portal_visual_tokens_from_part_tokens(&part_tokens);
//! let adapter = ResidentGrpcPortalAdapter::with_tokens(config, visual_tokens);
//!
//! // On profile hot-reload:
//! let new_part_tokens = resolve_portal_tokens(&new_token_map);
//! adapter.set_visual_tokens(portal_visual_tokens_from_part_tokens(&new_part_tokens));
//! ```

pub use tze_hud_projection::resident_grpc::portal_visual_tokens_from_part_tokens;

/// Resolve the resident gRPC bridge's `PortalVisualTokens` from the runtime's
/// LOADED startup design tokens.
///
/// `startup_tokens` is the runtime's `startup_compositor_tokens`: the canonical
/// defaults already pre-merged with the active profile's token overrides (the
/// same map applied to the in-process driver via `apply_token_map`). This is the
/// bridge-spawn counterpart to the in-process driver's `resolve_visual_tokens`
/// (`portal_projection_driver.rs`): both resolve against the same startup token
/// map so a bridged portal renders identically to an in-process one instead of
/// falling back to the unstyled canonical-default palette (hud-ygtiy).
///
/// Passing an empty map yields the canonical-default palette (the behaviour
/// before hud-ygtiy — active-profile styling was ignored). Doctrine: never
/// hardcode colours/fonts — `RenderingPolicy` is populated from these resolved
/// design tokens.
pub fn resolve_bridge_visual_tokens(
    startup_tokens: &tze_hud_config::tokens::DesignTokenMap,
) -> tze_hud_projection::resident_grpc::PortalVisualTokens {
    let resolved = tze_hud_config::tokens::resolve_tokens(
        &tze_hud_config::tokens::DesignTokenMap::new(),
        startup_tokens,
    );
    portal_visual_tokens_from_part_tokens(&tze_hud_config::resolve_portal_tokens(&resolved))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_config::resolve_portal_tokens;
    use tze_hud_projection::resident_grpc::PortalVisualTokens;

    /// Single-source-of-truth assertion (hud-dcynv): `PortalVisualTokens::default()` now
    /// derives from `tze_hud_config::PortalPartTokens::default()` via
    /// `portal_visual_tokens_from_part_tokens`, so both sides MUST agree exactly.
    ///
    /// Previously `PortalVisualTokens::default()` contained hand-coded floats that
    /// diverged from the config defaults by up to 3 ULPs of rounding. This test
    /// enforces the post-consolidation invariant: there is one canonical palette,
    /// and any future change to `tze_hud_config::portal_tokens::defaults` propagates
    /// here automatically — a change to one side that diverges from the other will
    /// fail this test.
    #[test]
    fn portal_visual_defaults_are_single_source_of_truth() {
        let empty: tze_hud_config::tokens::DesignTokenMap = std::collections::HashMap::new();
        let part_tokens = resolve_portal_tokens(&empty);
        // This is the "config path": config defaults → conversion function
        let from_config = portal_visual_tokens_from_part_tokens(&part_tokens);
        // This is the "projection default path": PortalVisualTokens::default()
        // which now delegates to portal_visual_tokens_from_part_tokens(&PortalPartTokens::default())
        let default_direct = PortalVisualTokens::default();

        // They must be exactly equal — same code path, same inputs.
        assert_eq!(
            from_config, default_direct,
            "PortalVisualTokens::default() must be exactly equal to \
             portal_visual_tokens_from_part_tokens(PortalPartTokens::default()); \
             divergence means the single-source-of-truth invariant was broken"
        );
    }

    /// Round-trip: `resolve_portal_tokens` → `portal_visual_tokens_from_part_tokens`
    /// must yield the same transcript/collapsed values as `PortalVisualTokens::default`.
    ///
    /// This verifies that the default-palette constants in both crates match the
    /// resolved defaults coming through `tze_hud_config::resolve_portal_tokens`.
    #[test]
    fn default_part_tokens_round_trip_matches_visual_defaults() {
        let empty: tze_hud_config::tokens::DesignTokenMap = std::collections::HashMap::new();
        let part_tokens = resolve_portal_tokens(&empty);
        let visual = portal_visual_tokens_from_part_tokens(&part_tokens);
        let default = PortalVisualTokens::default();

        // Transcript background (all four channels, including alpha)
        let eps = 1e-2_f32;
        assert!(
            (visual.transcript_background.r - default.transcript_background.r).abs() < eps,
            "transcript_background.r mismatch: got {}, expected {}",
            visual.transcript_background.r,
            default.transcript_background.r
        );
        assert!(
            (visual.transcript_background.g - default.transcript_background.g).abs() < eps,
            "transcript_background.g mismatch"
        );
        assert!(
            (visual.transcript_background.b - default.transcript_background.b).abs() < eps,
            "transcript_background.b mismatch"
        );
        assert!(
            (visual.transcript_background.a - default.transcript_background.a).abs() < eps,
            "transcript_background.a mismatch: got {}, expected {}",
            visual.transcript_background.a,
            default.transcript_background.a
        );

        // Transcript text color
        assert!(
            (visual.transcript_text_color.r - default.transcript_text_color.r).abs() < eps,
            "transcript_text_color.r mismatch: got {}, expected {}",
            visual.transcript_text_color.r,
            default.transcript_text_color.r
        );

        // Collapsed background (all four channels, including alpha)
        assert!(
            (visual.collapsed_background.r - default.collapsed_background.r).abs() < eps,
            "collapsed_background.r mismatch: got {}, expected {}",
            visual.collapsed_background.r,
            default.collapsed_background.r
        );
        assert!(
            (visual.collapsed_background.a - default.collapsed_background.a).abs() < eps,
            "collapsed_background.a mismatch: got {}, expected {}",
            visual.collapsed_background.a,
            default.collapsed_background.a
        );

        // Font sizes
        assert!(
            (visual.transcript_font_size_px - default.transcript_font_size_px).abs() < eps,
            "transcript_font_size_px mismatch"
        );
        assert!(
            (visual.collapsed_font_size_px - default.collapsed_font_size_px).abs() < eps,
            "collapsed_font_size_px mismatch"
        );

        // Composer fields
        assert!(
            (visual.composer_font_size_px - default.composer_font_size_px).abs() < eps,
            "composer_font_size_px mismatch"
        );
        // NOTE: the composer at-capacity color is no longer mirrored on
        // PortalVisualTokens -- it is resolved + applied compositor-side
        // (`portal.composer.at_capacity_color` -> `composer_draft_base_color`),
        // and its non-zero-alpha default is asserted in tze_hud_config's
        // portal_tokens tests (hud-9gyao).
    }

    /// Profile override propagates end-to-end through the canonical conversion chain.
    #[test]
    fn profile_override_propagates_through_conversion() {
        use tze_hud_config::tokens::resolve_tokens;
        use tze_hud_config::{
            PORTAL_TOKEN_COLLAPSED_FONT_SIZE, PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR,
        };

        let empty: tze_hud_config::tokens::DesignTokenMap = std::collections::HashMap::new();

        // Baseline
        let baseline_part = resolve_portal_tokens(&empty);
        let baseline = portal_visual_tokens_from_part_tokens(&baseline_part);

        // Override transcript text color and collapsed font size
        let mut overrides = tze_hud_config::tokens::DesignTokenMap::new();
        overrides.insert(
            PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR.to_string(),
            "#FF0000".to_string(), // red sentinel
        );
        overrides.insert(
            PORTAL_TOKEN_COLLAPSED_FONT_SIZE.to_string(),
            "20".to_string(),
        );
        let resolved = resolve_tokens(&empty, &overrides);
        let override_part = resolve_portal_tokens(&resolved);
        let overridden = portal_visual_tokens_from_part_tokens(&override_part);

        // Transcript text color must change to red
        assert!(
            (overridden.transcript_text_color.r - 1.0).abs() < 1e-3,
            "overridden transcript text color must have r=1.0 (red)"
        );
        assert!(
            overridden.transcript_text_color.g.abs() < 1e-3,
            "overridden transcript text color must have g=0.0"
        );
        assert!(
            overridden.transcript_text_color.b.abs() < 1e-3,
            "overridden transcript text color must have b=0.0"
        );

        // Collapsed font size must change
        assert!(
            (overridden.collapsed_font_size_px - 20.0).abs() < 1e-4,
            "overridden collapsed font size must be 20px"
        );

        // Baseline transcript text color must differ
        assert_ne!(
            baseline.transcript_text_color.r, overridden.transcript_text_color.r,
            "baseline and overridden transcript text colors must differ"
        );
    }

    /// hud-ygtiy: the resident gRPC bridge must resolve its visual tokens from the
    /// runtime's LOADED startup design tokens (active profile), NOT empty maps.
    ///
    /// This is the exact production path: `windowed::run` builds the bridge's
    /// `PortalVisualTokens` via `resolve_bridge_visual_tokens(&startup_compositor_tokens)`.
    /// The test proves that an active-profile token override (a sentinel that
    /// differs from the canonical default) appears in the bridge's resolved
    /// output — i.e. a bridged portal is styled by the active profile, not the
    /// unstyled canonical-default palette it used before hud-ygtiy.
    #[test]
    fn bridge_resolves_active_profile_tokens_not_empty_defaults() {
        use tze_hud_config::tokens::DesignTokenMap;
        use tze_hud_config::{
            PORTAL_TOKEN_COLLAPSED_FONT_SIZE, PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR,
        };

        // Empty startup tokens → canonical-default palette (pre-hud-ygtiy behaviour).
        let empty_defaults = resolve_bridge_visual_tokens(&DesignTokenMap::new());

        // Simulate the runtime's `startup_compositor_tokens`: canonical defaults
        // pre-merged with an active profile's overrides (sentinels that differ
        // from the canonical defaults).
        let mut startup_tokens = DesignTokenMap::new();
        startup_tokens.insert(
            PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR.to_string(),
            "#FF0000".to_string(), // red sentinel — distinct from the default text color
        );
        startup_tokens.insert(
            PORTAL_TOKEN_COLLAPSED_FONT_SIZE.to_string(),
            "20".to_string(),
        );

        let from_profile = resolve_bridge_visual_tokens(&startup_tokens);

        // The active-profile override MUST appear in the bridge's resolved output.
        assert!(
            (from_profile.transcript_text_color.r - 1.0).abs() < 1e-3
                && from_profile.transcript_text_color.g.abs() < 1e-3
                && from_profile.transcript_text_color.b.abs() < 1e-3,
            "bridge must resolve the active-profile transcript text color (red), got {:?}",
            from_profile.transcript_text_color
        );
        assert!(
            (from_profile.collapsed_font_size_px - 20.0).abs() < 1e-4,
            "bridge must resolve the active-profile collapsed font size (20px), got {}",
            from_profile.collapsed_font_size_px
        );

        // And it must NOT equal the empty-default palette — proving the bridge no
        // longer ignores the active profile (the hud-ygtiy regression).
        assert_ne!(
            from_profile.transcript_text_color.r, empty_defaults.transcript_text_color.r,
            "bridge tokens must differ from the empty-default palette (active profile ignored)"
        );
        assert_ne!(
            from_profile.collapsed_font_size_px, empty_defaults.collapsed_font_size_px,
            "bridge collapsed font size must differ from the empty default"
        );
    }
}
