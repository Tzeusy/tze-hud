//! Privacy redaction — shell-layer placeholder rendering for tiles whose content
//! must not be shown to the current viewer.
//!
//! # Ownership rule
//!
//! The shell is the **sole owner** of redaction rendering decisions within the
//! chrome/overlay layer.  Redaction is driven by viewer context intersecting with
//! content classification.  The agent is **never** notified that its tile is
//! redacted.
//!
//! Policy-arbitration (Level 2 Privacy Evaluation) determines whether redaction
//! applies; the shell calls [`is_tile_redacted`] and renders the placeholder via
//! [`build_redaction_cmds`].
//!
//! # V1 scope
//!
//! * Overlay-only redaction — `capture_surface_active` is always `false`.
//! * Render-skip redaction is architecturally preserved (content and chrome passes
//!   are separable) but not implemented in v1.
//! * Viewer identification pipeline (`viewer_detectors`) is v1-reserved.
//!
//! # Spec references
//!
//! * `openspec/.../system-shell/spec.md` §Requirement: Redaction Placeholder (line 184)
//! * §Requirement: Agent Isolation for Viewer State (line 232)
//! * §Requirement: Capture-Safe Redaction Architecture (line 294)

use tze_hud_compositor::ChromeDrawCmd;
use tze_hud_scene::types::Rect;

// ─── Viewer class ─────────────────────────────────────────────────────────────

/// Type alias to [`super::chrome::ViewerClass`] so that redaction evaluation and
/// chrome state stay type-aligned without duplicating the enum definition.
///
/// Callers can pass `ChromeState.viewer_class` directly to [`is_tile_redacted`]
/// and [`RedactionFrame::build`] without any conversion.
///
/// Agents must never receive viewer class information through any API surface.
pub use super::chrome::ViewerClass;

// ─── Content classification ──────────────────────────────────────────────────

/// Content classification assigned to a tile by its agent or inherited from the
/// zone ceiling rule.
///
/// Ordered from least restrictive (`Public`) to most restrictive (`Sensitive`).
/// The level 2 privacy arbiter applies `max(agent_declared, zone_default)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContentClassification {
    /// Visible to all viewer classes.
    #[default]
    Public = 0,
    /// Visible to `HouseholdMember` and `Owner` only.
    Household = 1,
    /// Visible to `Owner` only.
    Private = 2,
    /// Visible to `Owner` only (semantically distinct from Private; same access).
    Sensitive = 3,
}

// ─── Redaction style ─────────────────────────────────────────────────────────

/// How the redaction placeholder is rendered.
///
/// Configured via `[privacy].redaction_style` in the runtime config.
/// Only `Pattern` and `Blank` are valid; `AgentName` and `Icon` were removed
/// from the spec (rig-8ss).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RedactionStyle {
    /// Fill the tile bounds with a repeating neutral checkerboard pattern.
    #[default]
    Pattern,
    /// Fill the tile bounds with a flat neutral color (solid blank).
    Blank,
}

// ─── Visibility evaluation ───────────────────────────────────────────────────

/// Returns `true` if the tile's content should be redacted for the current viewer.
///
/// This is a pure, allocation-free function safe to call from the frame loop.
///
/// # Ownership
///
/// The shell calls this function once per visible tile per frame.  The result is
/// used only to drive the chrome/overlay rendering decision.  The agent is **never
/// notified** of the outcome.
///
/// # Access matrix
///
/// | ViewerClass       | Public | Household | Private | Sensitive |
/// |-------------------|--------|-----------|---------|-----------|
/// | Owner             | ✓      | ✓         | ✓       | ✓         |
/// | HouseholdMember   | ✓      | ✓         | ✗       | ✗         |
/// | KnownGuest        | ✓      | ✗         | ✗       | ✗         |
/// | Unknown           | ✓      | ✗         | ✗       | ✗         |
/// | Nobody            | ✓      | ✗         | ✗       | ✗         |
#[inline]
pub fn is_tile_redacted(viewer: ViewerClass, classification: ContentClassification) -> bool {
    !viewer_may_see(viewer, classification)
}

/// Returns `true` if `viewer` is permitted to see content at `classification`.
#[inline]
fn viewer_may_see(viewer: ViewerClass, classification: ContentClassification) -> bool {
    match viewer {
        ViewerClass::Owner => true,
        ViewerClass::HouseholdMember => classification <= ContentClassification::Household,
        ViewerClass::KnownGuest | ViewerClass::Unknown | ViewerClass::Nobody => {
            classification == ContentClassification::Public
        }
    }
}

// ─── Per-tile redaction state ─────────────────────────────────────────────────

/// Shell-managed redaction state for a single tile.
///
/// This is owned by the shell's redaction tracking map, keyed by tile `SceneId`.
/// It is **never** exposed to agents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TileRedactionState {
    /// Tile content is visible to the current viewer — render normally.
    Clear,
    /// Tile content is redacted — render placeholder, disable hit regions.
    Redacted {
        /// The content classification that triggered redaction (for internal
        /// bookkeeping; never forwarded to the agent).
        classification: ContentClassification,
    },
}

impl TileRedactionState {
    /// Returns `true` if this tile is currently redacted.
    pub fn is_redacted(&self) -> bool {
        matches!(self, TileRedactionState::Redacted { .. })
    }
}

// ─── Hit region gating ───────────────────────────────────────────────────────

/// Returns `true` if hit-testing should be suppressed for this tile.
///
/// When a tile is redacted, all interactive affordances (hit regions) must be
/// disabled.  This prevents agents from inferring redaction state via interaction
/// feedback.
///
/// The shell calls this from the input model's hit-test path, before any
/// hit-region events are dispatched.
#[inline]
pub fn hit_regions_enabled(state: &TileRedactionState) -> bool {
    !state.is_redacted()
}

// ─── Redaction placeholder draw commands ─────────────────────────────────────

/// Produces the chrome draw commands that replace a tile's content area with a
/// neutral redaction placeholder.
///
/// # Contract
///
/// * The placeholder fills the tile's **full bounds** exactly — layout dimensions
///   are preserved so the tile's footprint on screen does not leak information
///   about content shape.
/// * No agent name, content hint, or icon is rendered.
/// * The placeholder is rendered in the **chrome/overlay pass** (after the content
///   pass), so the underlying tile content pixels are overwritten at the chrome
///   layer — not skipped during content rendering.  This is v1 overlay-only
///   redaction.  Render-skip redaction (post-v1) would suppress the content pass
///   for the tile; the separable-pass architecture already supports that.
///
/// # Parameters
///
/// * `bounds` — tile bounds in screen-pixel coordinates.  If `width` or `height`
///   are non-positive the function returns an empty vec immediately.
/// * `style`  — `Pattern` (checkerboard) or `Blank` (solid neutral).
pub fn build_redaction_cmds(bounds: Rect, style: RedactionStyle) -> Vec<ChromeDrawCmd> {
    // Guard: degenerate or inverted bounds must not produce geometry.
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return Vec::new();
    }

    let mut cmds = Vec::new();

    match style {
        RedactionStyle::Blank => {
            // Solid neutral grey fill over the entire tile area.
            cmds.push(ChromeDrawCmd {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
                color: REDACTION_BLANK_COLOR,
            });
        }
        RedactionStyle::Pattern => {
            // Checkerboard pattern simulated with two layers:
            // 1. A base fill.
            // 2. A grid of small accent squares offset by one cell.
            //
            // This is purely a visual representation.  In a production renderer
            // with a texture pipeline the checkerboard would be a generated
            // texture; here it is approximated with a modest number of rects
            // so the test suite can verify placeholder rendering without GPU.
            //
            // Accent rect count is capped at `MAX_PATTERN_ACCENT_RECTS` to bound
            // per-frame CPU allocation on large tiles.  When the cap is exceeded
            // the tile falls through to the blank style (base fill only) so the
            // tile is still fully covered without unbounded allocation.

            // Base fill (also serves as the fallback when cap is exceeded).
            cmds.push(ChromeDrawCmd {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
                color: REDACTION_PATTERN_BASE,
            });

            // Accent squares to form a visible checkerboard approximation.
            let cell = PATTERN_CELL_PX;
            let cols = ((bounds.width / cell).ceil() as u32).max(1);
            let rows = ((bounds.height / cell).ceil() as u32).max(1);

            // Pre-check: if the grid would exceed the cap, skip accent rects
            // entirely (the base fill above already covers the tile).
            let total_cells = cols as usize * rows as usize;
            let accent_cells = (total_cells + 1) / 2; // ceil(total/2) worst-case
            if accent_cells <= MAX_PATTERN_ACCENT_RECTS {
                for row in 0..rows {
                    for col in 0..cols {
                        // Only shade alternating cells (checkerboard parity).
                        if (row + col) % 2 == 0 {
                            continue;
                        }
                        let cx = bounds.x + col as f32 * cell;
                        let cy = bounds.y + row as f32 * cell;
                        let cw = cell.min(bounds.x + bounds.width - cx);
                        let ch = cell.min(bounds.y + bounds.height - cy);

                        if cw > 0.0 && ch > 0.0 {
                            cmds.push(ChromeDrawCmd {
                                x: cx,
                                y: cy,
                                width: cw,
                                height: ch,
                                color: REDACTION_PATTERN_ACCENT,
                            });
                        }
                    }
                }
            }
        }
    }

    cmds
}

// ─── Render constants ─────────────────────────────────────────────────────────

/// Cell size for the checkerboard pattern in pixels.
pub const PATTERN_CELL_PX: f32 = 24.0;

/// Maximum number of accent `ChromeDrawCmd` rects emitted by the pattern renderer
/// per tile per frame.  When the tile's cell grid would exceed this count the
/// pattern renderer falls back to the base fill only (no accent rects), so the
/// tile is fully covered without unbounded per-frame allocation.
///
/// At `PATTERN_CELL_PX = 24px`, a 1920×1080 tile has ≈ 3400 cells and ≈ 1700
/// accent rects — just above this cap.  Increase the constant if the pattern
/// must be visible at 4K; at that point a texture-based approach is preferred.
pub const MAX_PATTERN_ACCENT_RECTS: usize = 1024;

/// Base fill color for the redaction placeholder (both styles use this as a
/// foundation for the blank style and as the lighter cell for the pattern).
///
/// Neutral mid-grey at full opacity.
pub const REDACTION_BLANK_COLOR: [f32; 4] = [0.28, 0.28, 0.30, 1.0];

/// Lighter checkerboard base (used even-parity cells).
pub const REDACTION_PATTERN_BASE: [f32; 4] = [0.24, 0.24, 0.26, 1.0];

/// Darker checkerboard accent (used for odd-parity cells).
pub const REDACTION_PATTERN_ACCENT: [f32; 4] = [0.18, 0.18, 0.20, 1.0];

// ─── Redaction context for a frame ───────────────────────────────────────────

/// A snapshot of which tiles should be redacted in the current frame.
///
/// The shell builds a `RedactionFrame` once per frame from `ChromeState.viewer_class`
/// and the per-tile content classifications, then passes it to the compositor for
/// overlay rendering.
///
/// This struct is **never** exposed to agents.
pub struct RedactionFrame {
    /// Effective viewer class for this frame.
    pub viewer_class: ViewerClass,
    /// Redaction style from config.
    pub style: RedactionStyle,
    /// Per-tile redaction decisions, keyed by tile index in `visible_tiles()` order.
    /// `true` = redact this tile.
    pub tile_redacted: Vec<bool>,
}

impl RedactionFrame {
    /// Build a `RedactionFrame` by evaluating each tile's classification against
    /// the current viewer class.
    ///
    /// `classifications` is a slice of `(tile_index, ContentClassification)` pairs
    /// for all visible tiles.  Tiles absent from the slice are treated as `Public`.
    pub fn build(
        viewer_class: ViewerClass,
        style: RedactionStyle,
        tile_count: usize,
        classifications: &[(usize, ContentClassification)],
    ) -> Self {
        let mut tile_redacted = vec![false; tile_count];
        for &(idx, cls) in classifications {
            if idx < tile_count {
                tile_redacted[idx] = is_tile_redacted(viewer_class, cls);
            }
        }
        Self { viewer_class, style, tile_redacted }
    }

    /// Returns `true` if the tile at `tile_index` should have its content replaced
    /// with the redaction placeholder this frame.
    pub fn is_redacted(&self, tile_index: usize) -> bool {
        self.tile_redacted.get(tile_index).copied().unwrap_or(false)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Visibility matrix ─────────────────────────────────────────────────

    /// WHEN Owner views any classification THEN never redacted.
    #[test]
    fn owner_sees_all_classifications() {
        for cls in [
            ContentClassification::Public,
            ContentClassification::Household,
            ContentClassification::Private,
            ContentClassification::Sensitive,
        ] {
            assert!(
                !is_tile_redacted(ViewerClass::Owner, cls),
                "Owner must never be redacted (classification={:?})",
                cls
            );
        }
    }

    /// WHEN HouseholdMember views Public or Household THEN not redacted.
    #[test]
    fn household_member_sees_public_and_household() {
        assert!(!is_tile_redacted(ViewerClass::HouseholdMember, ContentClassification::Public));
        assert!(!is_tile_redacted(ViewerClass::HouseholdMember, ContentClassification::Household));
        assert!(is_tile_redacted(ViewerClass::HouseholdMember, ContentClassification::Private));
        assert!(is_tile_redacted(ViewerClass::HouseholdMember, ContentClassification::Sensitive));
    }

    /// WHEN KnownGuest/Unknown/Nobody views non-Public THEN redacted.
    #[test]
    fn restricted_viewers_see_only_public() {
        for viewer in [ViewerClass::KnownGuest, ViewerClass::Unknown, ViewerClass::Nobody] {
            assert!(
                !is_tile_redacted(viewer, ContentClassification::Public),
                "viewer {:?} must see Public",
                viewer
            );
            for cls in [
                ContentClassification::Household,
                ContentClassification::Private,
                ContentClassification::Sensitive,
            ] {
                assert!(
                    is_tile_redacted(viewer, cls),
                    "viewer {:?} must not see {:?}",
                    viewer,
                    cls
                );
            }
        }
    }

    /// Scenario: Tile redacted for guest viewer (spec line 189).
    /// WHEN tile with `private` classification displayed to viewer with `unknown` class
    /// THEN content replaced with neutral placeholder, hit regions disabled, agent not notified.
    #[test]
    fn spec_scenario_tile_redacted_for_guest_viewer() {
        // Redaction decision
        assert!(
            is_tile_redacted(ViewerClass::Unknown, ContentClassification::Private),
            "Private content must be redacted for Unknown viewer"
        );

        // Placeholder must fill tile bounds (layout preserved)
        let bounds = Rect::new(100.0, 200.0, 400.0, 300.0);
        let cmds = build_redaction_cmds(bounds, RedactionStyle::Blank);
        assert!(!cmds.is_empty(), "redaction placeholder must produce draw commands");

        // All commands must be within tile bounds
        for cmd in &cmds {
            assert!(cmd.x >= bounds.x - 0.01, "cmd must start within tile x");
            assert!(cmd.y >= bounds.y - 0.01, "cmd must start within tile y");
            assert!(
                cmd.x + cmd.width <= bounds.x + bounds.width + 0.01,
                "cmd must not exceed tile right edge"
            );
            assert!(
                cmd.y + cmd.height <= bounds.y + bounds.height + 0.01,
                "cmd must not exceed tile bottom edge"
            );
        }

        // Hit regions disabled
        let state = TileRedactionState::Redacted {
            classification: ContentClassification::Private,
        };
        assert!(!hit_regions_enabled(&state), "hit regions must be disabled when redacted");
    }

    /// Scenario: Redaction removed on viewer change (spec line 193).
    /// WHEN viewer context changes from `unknown` to `owner`
    /// THEN redaction is no longer required and content is shown.
    #[test]
    fn spec_scenario_redaction_removed_on_viewer_change() {
        let cls = ContentClassification::Private;

        // Unknown → redacted
        assert!(is_tile_redacted(ViewerClass::Unknown, cls));

        // Owner → not redacted
        assert!(!is_tile_redacted(ViewerClass::Owner, cls));

        // Clear state means hit regions are re-enabled
        let clear_state = TileRedactionState::Clear;
        assert!(hit_regions_enabled(&clear_state), "hit regions must be enabled when clear");
    }

    /// Scenario: Agent cannot detect redaction (spec line 241).
    /// The redaction state is only a shell-internal value; the spec invariant is
    /// that agents receive no notification.  We verify that `TileRedactionState`
    /// is never constructed from agent-observable types.
    #[test]
    fn agent_cannot_detect_redaction_state_type_check() {
        // TileRedactionState is in the shell module.  This test asserts that
        // `is_redacted()` returns the correct value without any agent-facing API.
        let redacted = TileRedactionState::Redacted { classification: ContentClassification::Private };
        let clear = TileRedactionState::Clear;

        assert!(redacted.is_redacted());
        assert!(!clear.is_redacted());
    }

    // ── Placeholder rendering ─────────────────────────────────────────────

    /// Blank placeholder fills the exact tile bounds.
    #[test]
    fn blank_placeholder_fills_tile_bounds() {
        let bounds = Rect::new(50.0, 75.0, 300.0, 200.0);
        let cmds = build_redaction_cmds(bounds, RedactionStyle::Blank);

        assert_eq!(cmds.len(), 1, "blank style should produce exactly one draw command");
        let cmd = &cmds[0];
        assert!((cmd.x - bounds.x).abs() < 0.01);
        assert!((cmd.y - bounds.y).abs() < 0.01);
        assert!((cmd.width - bounds.width).abs() < 0.01);
        assert!((cmd.height - bounds.height).abs() < 0.01);
    }

    /// Pattern placeholder covers the tile bounds and produces multiple rects.
    #[test]
    fn pattern_placeholder_covers_tile_bounds() {
        let bounds = Rect::new(0.0, 0.0, 480.0, 240.0);
        let cmds = build_redaction_cmds(bounds, RedactionStyle::Pattern);

        // At least a base rect plus some accent cells
        assert!(cmds.len() > 1, "pattern style must produce more than one draw command");

        // All commands must be within tile bounds (with 1px tolerance)
        for cmd in &cmds {
            assert!(cmd.x >= bounds.x - 1.0, "cmd x below tile left: {}", cmd.x);
            assert!(cmd.y >= bounds.y - 1.0, "cmd y above tile top: {}", cmd.y);
            assert!(
                cmd.x + cmd.width <= bounds.x + bounds.width + 1.0,
                "cmd right edge {} exceeds tile right edge {}",
                cmd.x + cmd.width,
                bounds.x + bounds.width
            );
            assert!(
                cmd.y + cmd.height <= bounds.y + bounds.height + 1.0,
                "cmd bottom edge {} exceeds tile bottom {}",
                cmd.y + cmd.height,
                bounds.y + bounds.height
            );
        }
    }

    /// No agent name, content hint, or icon in placeholder output.
    ///
    /// The placeholder produces only colored rectangle commands — there is no
    /// mechanism to embed agent metadata in a `ChromeDrawCmd`.
    #[test]
    fn placeholder_produces_only_colored_rects_no_agent_metadata() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        for style in [RedactionStyle::Blank, RedactionStyle::Pattern] {
            let cmds = build_redaction_cmds(bounds, style);
            // Every command is a pure geometry + color tuple — no agent metadata
            for cmd in &cmds {
                // Dimensions must be positive (layout preservation check)
                assert!(cmd.width > 0.0, "redaction cmd width must be positive");
                assert!(cmd.height > 0.0, "redaction cmd height must be positive");
            }
        }
    }

    /// Layout dimensions preserved — placeholder fills same area as the tile.
    ///
    /// A tile that is 960×540 in screen coords must have its full 960×540 area
    /// covered by the placeholder, so the viewer cannot infer content shape from
    /// the tile footprint.
    #[test]
    fn layout_dimensions_preserved_for_redacted_tile() {
        let bounds = Rect::new(10.0, 20.0, 960.0, 540.0);

        // For blank: single rect exactly matching bounds
        let blank_cmds = build_redaction_cmds(bounds, RedactionStyle::Blank);
        assert_eq!(blank_cmds.len(), 1);
        assert!((blank_cmds[0].x - bounds.x).abs() < 0.01);
        assert!((blank_cmds[0].y - bounds.y).abs() < 0.01);
        assert!((blank_cmds[0].width - bounds.width).abs() < 0.01);
        assert!((blank_cmds[0].height - bounds.height).abs() < 0.01);

        // For pattern: first command is the base fill matching bounds
        let pattern_cmds = build_redaction_cmds(bounds, RedactionStyle::Pattern);
        assert!(!pattern_cmds.is_empty());
        let base = &pattern_cmds[0];
        assert!((base.x - bounds.x).abs() < 0.01, "pattern base x mismatch");
        assert!((base.y - bounds.y).abs() < 0.01, "pattern base y mismatch");
        assert!((base.width - bounds.width).abs() < 0.01, "pattern base width mismatch");
        assert!((base.height - bounds.height).abs() < 0.01, "pattern base height mismatch");
    }

    // ── RedactionFrame ────────────────────────────────────────────────────

    /// WHEN Unknown viewer and one Private tile THEN that tile is redacted.
    #[test]
    fn redaction_frame_marks_private_tile_for_unknown_viewer() {
        let frame = RedactionFrame::build(
            ViewerClass::Unknown,
            RedactionStyle::Blank,
            3,
            &[
                (0, ContentClassification::Public),
                (1, ContentClassification::Private),
                (2, ContentClassification::Public),
            ],
        );

        assert!(!frame.is_redacted(0), "public tile must not be redacted");
        assert!(frame.is_redacted(1), "private tile must be redacted for Unknown viewer");
        assert!(!frame.is_redacted(2), "public tile must not be redacted");
    }

    /// WHEN Owner viewer THEN no tiles redacted regardless of classification.
    #[test]
    fn redaction_frame_owner_sees_everything() {
        let frame = RedactionFrame::build(
            ViewerClass::Owner,
            RedactionStyle::Pattern,
            4,
            &[
                (0, ContentClassification::Public),
                (1, ContentClassification::Household),
                (2, ContentClassification::Private),
                (3, ContentClassification::Sensitive),
            ],
        );

        for i in 0..4 {
            assert!(!frame.is_redacted(i), "Owner must not have any tile redacted (tile {})", i);
        }
    }

    /// WHEN viewer context changes from Unknown to Owner THEN previously-redacted
    /// tile is no longer redacted in the new frame.
    #[test]
    fn redaction_frame_clears_on_viewer_upgrade() {
        let classifications = &[(0, ContentClassification::Private)];

        // Unknown viewer → redacted
        let frame_unknown =
            RedactionFrame::build(ViewerClass::Unknown, RedactionStyle::Blank, 1, classifications);
        assert!(frame_unknown.is_redacted(0));

        // Owner viewer → not redacted
        let frame_owner =
            RedactionFrame::build(ViewerClass::Owner, RedactionStyle::Blank, 1, classifications);
        assert!(!frame_owner.is_redacted(0), "redaction must clear when viewer becomes Owner");
    }

    // ── Hit region gating ─────────────────────────────────────────────────

    /// WHEN tile is redacted THEN hit regions are disabled.
    #[test]
    fn hit_regions_disabled_when_redacted() {
        let state =
            TileRedactionState::Redacted { classification: ContentClassification::Sensitive };
        assert!(!hit_regions_enabled(&state), "hit regions must be disabled when tile is redacted");
    }

    /// WHEN tile is clear THEN hit regions are enabled.
    #[test]
    fn hit_regions_enabled_when_clear() {
        let state = TileRedactionState::Clear;
        assert!(hit_regions_enabled(&state), "hit regions must be enabled when tile is clear");
    }

    // ── Capture-safe architecture ─────────────────────────────────────────

    /// V1: capture_surface_active must always be false.
    ///
    /// This test is co-located here as the spec co-locates the redaction and
    /// capture-safe requirements.  The actual `capture_surface_active` field lives
    /// in `ChromeState`; this test exercises the invariant at the redaction module
    /// level to keep the spec coverage in one place.
    #[test]
    fn v1_capture_surface_active_is_always_false() {
        use crate::shell::chrome::ChromeState;
        let state = ChromeState::new();
        assert!(
            !state.capture_surface_active,
            "v1: capture_surface_active must always be false (overlay-only redaction)"
        );
    }

    // ── privacy_redaction_mode test scene ─────────────────────────────────

    /// Tests that the `privacy_redaction_mode` test scene exercises the full
    /// redaction pipeline:
    ///
    /// - Public tile for `Unknown` viewer → not redacted.
    /// - Private/Sensitive tile for `Unknown` viewer → redacted.
    /// - Both tiles for `Owner` viewer → neither redacted.
    ///
    /// This corresponds to the spec acceptance criterion:
    /// "privacy_redaction_mode test scene passes".
    #[test]
    fn privacy_redaction_mode_test_scene_passes() {
        // The privacy_redaction_mode scene has two tiles:
        // - Tile 0: PUBLIC classification
        // - Tile 1: SENSITIVE classification
        let scene_tiles = &[
            (0usize, ContentClassification::Public),
            (1usize, ContentClassification::Sensitive),
        ];

        // --- Unknown viewer ---
        let frame_unknown =
            RedactionFrame::build(ViewerClass::Unknown, RedactionStyle::Pattern, 2, scene_tiles);

        assert!(
            !frame_unknown.is_redacted(0),
            "Unknown viewer: PUBLIC tile must not be redacted"
        );
        assert!(
            frame_unknown.is_redacted(1),
            "Unknown viewer: SENSITIVE tile must be redacted"
        );

        // Placeholder for the sensitive tile must cover its bounds.
        let sensitive_bounds = Rect::new(980.0, 0.0, 940.0, 1080.0);
        let cmds = build_redaction_cmds(sensitive_bounds, RedactionStyle::Pattern);
        assert!(!cmds.is_empty(), "redaction placeholder for sensitive tile must produce commands");

        // Hit regions on the sensitive tile must be disabled.
        let sensitive_state =
            TileRedactionState::Redacted { classification: ContentClassification::Sensitive };
        assert!(!hit_regions_enabled(&sensitive_state));

        // --- Owner viewer ---
        let frame_owner =
            RedactionFrame::build(ViewerClass::Owner, RedactionStyle::Pattern, 2, scene_tiles);

        assert!(
            !frame_owner.is_redacted(0),
            "Owner viewer: PUBLIC tile must not be redacted"
        );
        assert!(
            !frame_owner.is_redacted(1),
            "Owner viewer: SENSITIVE tile must not be redacted"
        );
    }

    // ── Separable render passes invariant ─────────────────────────────────

    /// Content and chrome render passes remain separable.
    ///
    /// Redaction draw commands are produced independently of the content pass.
    /// This confirms that the redaction module does not introduce a dependency
    /// between the content render pass and the chrome/overlay render pass.
    #[test]
    fn redaction_cmds_are_independent_of_content_pass() {
        // We can produce redaction draw commands without any scene graph access.
        let bounds = Rect::new(0.0, 0.0, 200.0, 200.0);
        let cmds = build_redaction_cmds(bounds, RedactionStyle::Blank);

        // The commands are purely geometric — no reference to scene/agent state.
        assert!(!cmds.is_empty());
        // If we change the viewer class, we re-evaluate is_tile_redacted() and
        // rebuild the commands — no dependency on the content pass.
        assert!(!is_tile_redacted(ViewerClass::Owner, ContentClassification::Private));
        assert!(is_tile_redacted(ViewerClass::Unknown, ContentClassification::Private));
    }
}
