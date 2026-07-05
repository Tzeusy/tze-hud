//! Token-styled "jump to latest" affordance for scrolled-back portal tiles.
//!
//! Auto follow-tail already exists (`FollowTailAnchor` / `ScrollState`) and
//! `ScrollState::reset_to_tail` can snap a tile's viewport back to the tail,
//! but there was no user-facing control to trigger it — a viewer scrolled up
//! into transcript history had no way back to the live edge short of manually
//! scrolling all the way down (hud-9ci61).
//!
//! This module computes the geometry of a small clickable pill that:
//! - the compositor renders ONLY when a tile is scrolled away from the tail
//!   (`renderer/frame.rs`, alongside the existing scroll-position indicator),
//! - the compositor also hit-tests each frame
//!   (`renderer/hit_regions.rs::populate_zone_hit_regions`), and
//! - the runtime wires to `InputProcessor::reset_tile_scroll_to_tail` on
//!   click (`windowed/lifecycle.rs`, `headless.rs`).
//!
//! ## Design
//!
//! Token-styled: every visual property comes from [`JumpToLatestTokens`],
//! resolved by the compositor from the portal token map. No literal visual
//! values are permitted in the geometry computation itself.
//!
//! Ambient/subtle: this is a presence engine, not a chat app — the pill is a
//! small fixed-size affordance anchored to the tile, not a loud banner.
//! Placement (bottom-center) and appearance (a plain filled rect, no icon)
//! are intentionally minimal for this first pass; the owner may refine
//! sizing, iconography, or position in a follow-up.
//!
//! ## Local feedback first
//!
//! The click handler calls `ScrollState::reset_to_tail` (via
//! `InputProcessor::reset_tile_scroll_to_tail`) synchronously in the same
//! pointer-up dispatch that produced the hit — no roundtrip to an adapter.

use serde::{Deserialize, Serialize};

/// Token-resolved visual properties for the "jump to latest" pill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JumpToLatestTokens {
    /// Pill fill color (RGBA components, each in [0.0, 1.0]).
    pub color_r: f32,
    pub color_g: f32,
    pub color_b: f32,
    pub color_a: f32,
    /// Pill width in pixels (clamped to the tile width at render time).
    pub width_px: f32,
    /// Pill height in pixels (clamped to the tile height at render time).
    pub height_px: f32,
    /// Margin (px) between the pill's bottom edge and the tile's bottom edge.
    pub margin_px: f32,
}

impl Default for JumpToLatestTokens {
    fn default() -> Self {
        // Must match `token_colors::resolve_jump_to_latest_tokens` fallback
        // defaults in tze_hud_compositor. No compile-time link; update both
        // sides when changing defaults.
        Self {
            // #4A5568 — the same neutral chrome tone as the scroll indicator
            // thumb, ambient rather than alarming.
            color_r: 0x4A as f32 / 255.0,
            color_g: 0x55 as f32 / 255.0,
            color_b: 0x68 as f32 / 255.0,
            color_a: 0.9,
            width_px: 96.0,
            height_px: 24.0,
            margin_px: 8.0,
        }
    }
}

/// Computed geometry for the "jump to latest" pill, in tile-local pixels
/// (origin = tile top-left, matching [`crate::ScrollIndicatorGeometry`]'s
/// convention).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JumpToLatestGeometry {
    /// X offset of the pill's left edge within the tile.
    pub x_px: f32,
    /// Y offset of the pill's top edge within the tile.
    pub y_px: f32,
    /// Pill width.
    pub width_px: f32,
    /// Pill height.
    pub height_px: f32,
}

/// Compute "jump to latest" pill geometry for a single tile.
///
/// Returns `None` whenever the pill must not be shown:
/// - `scrolled_back == false` (the tile is already at the tail), or
/// - the viewport dimensions are non-positive or non-finite.
///
/// Callers MUST additionally gate on content overflow — e.g. only call this
/// when [`crate::compute_scroll_indicator`] also returns `Some` for the same
/// tile — there is no point offering to "jump to latest" content that
/// already fits entirely in the viewport (scroll state can never report
/// `ScrolledBack` for such a tile in practice, but callers should not rely on
/// that as the sole guard).
///
/// # Arguments
///
/// * `viewport_w_px` / `viewport_h_px` — visible size of the tile.
/// * `scrolled_back` — `true` when the tile's `FollowTailAnchor` is
///   `ScrolledBack` (the viewer has scrolled away from the tail).
/// * `tokens` — resolved pill styling from the portal token set.
pub fn compute_jump_to_latest_pill(
    viewport_w_px: f32,
    viewport_h_px: f32,
    scrolled_back: bool,
    tokens: &JumpToLatestTokens,
) -> Option<JumpToLatestGeometry> {
    if !scrolled_back {
        return None;
    }
    // Reject non-finite inputs before any comparison (NaN comparisons are
    // always false, so a `<= 0.0` check alone would silently pass for NaN).
    if !viewport_w_px.is_finite() || !viewport_h_px.is_finite() {
        return None;
    }
    if viewport_w_px <= 0.0 || viewport_h_px <= 0.0 {
        return None;
    }

    let width = tokens.width_px.min(viewport_w_px).max(0.0);
    let height = tokens.height_px.min(viewport_h_px).max(0.0);
    // Bottom-center: ambient, and out of the way of the scroll indicator
    // thumb (which tracks the tile's right edge).
    let x = ((viewport_w_px - width) / 2.0).max(0.0);
    let y = (viewport_h_px - height - tokens.margin_px).max(0.0);

    Some(JumpToLatestGeometry {
        x_px: x,
        y_px: y,
        width_px: width,
        height_px: height,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_tokens() -> JumpToLatestTokens {
        JumpToLatestTokens::default()
    }

    // ─── Visibility gates on scrolled-back state ──────────────────────────

    #[test]
    fn hidden_when_at_tail() {
        let tokens = default_tokens();
        let result = compute_jump_to_latest_pill(400.0, 300.0, false, &tokens);
        assert!(
            result.is_none(),
            "pill must be hidden when the tile is at the tail (not scrolled back)"
        );
    }

    #[test]
    fn shown_when_scrolled_back() {
        let tokens = default_tokens();
        let result = compute_jump_to_latest_pill(400.0, 300.0, true, &tokens);
        assert!(
            result.is_some(),
            "pill must appear when the tile is scrolled back"
        );
    }

    // ─── Geometry ──────────────────────────────────────────────────────────

    #[test]
    fn pill_anchored_bottom_center() {
        let tokens = default_tokens();
        let geom = compute_jump_to_latest_pill(400.0, 300.0, true, &tokens).unwrap();
        let expected_x = (400.0 - geom.width_px) / 2.0;
        let expected_y = 300.0 - geom.height_px - tokens.margin_px;
        assert!(
            (geom.x_px - expected_x).abs() < f32::EPSILON,
            "pill must be horizontally centered"
        );
        assert!(
            (geom.y_px - expected_y).abs() < f32::EPSILON,
            "pill must sit `margin_px` above the tile's bottom edge"
        );
    }

    #[test]
    fn pill_size_matches_tokens() {
        let mut tokens = default_tokens();
        tokens.width_px = 50.0;
        tokens.height_px = 20.0;
        let geom = compute_jump_to_latest_pill(400.0, 300.0, true, &tokens).unwrap();
        assert!((geom.width_px - 50.0).abs() < f32::EPSILON);
        assert!((geom.height_px - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn pill_clamped_to_tiny_viewport() {
        let tokens = default_tokens(); // width=96, height=24
        let geom = compute_jump_to_latest_pill(40.0, 10.0, true, &tokens).unwrap();
        assert!(geom.width_px <= 40.0, "pill width must not exceed viewport");
        assert!(
            geom.height_px <= 10.0,
            "pill height must not exceed viewport"
        );
        assert!(geom.x_px >= 0.0);
        assert!(geom.y_px >= 0.0);
    }

    #[test]
    fn non_finite_viewport_rejected() {
        let tokens = default_tokens();
        assert!(compute_jump_to_latest_pill(f32::NAN, 300.0, true, &tokens).is_none());
        assert!(compute_jump_to_latest_pill(400.0, f32::INFINITY, true, &tokens).is_none());
    }

    #[test]
    fn non_positive_viewport_rejected() {
        let tokens = default_tokens();
        assert!(compute_jump_to_latest_pill(0.0, 300.0, true, &tokens).is_none());
        assert!(compute_jump_to_latest_pill(400.0, -1.0, true, &tokens).is_none());
    }
}
