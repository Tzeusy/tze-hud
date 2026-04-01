//! Compositor integration tests for ambient-background SolidColor rendering.
//!
//! Covers openspec/changes/exemplar-ambient-background/specs/exemplar-ambient-background/spec.md
//! — Requirements:
//!   - "Ambient Background Zone Visual Contract"
//!     Scenarios: "Solid color background fills the display",
//!                "No publication yields transparent/clear background"
//!   - "Ambient Background Latest-Wins Contention"
//!     Scenarios: "New solid color replaces previous solid color",
//!                "Rapid replacement under contention"
//!
//! ## Test list
//!
//! 1. `test_ambient_background_solid_color_renders` — Publish dark-blue SolidColor,
//!    render frame, assert pixels are blue-dominant (not the clear color).
//!
//! 2. `test_ambient_background_no_publication_empty` — No content published → zone's
//!    active_publishes is empty and background renders as the runtime clear color.
//!
//! 3. `test_ambient_background_replacement_contention` — Publish red then blue, render
//!    one frame, assert blue pixels + publication count is 1.
//!
//! 4. `test_ambient_background_rapid_replacement` — Publish 10 colors in sequence
//!    within a single frame interval, assert only last color visible + count is 1.
//!
//! ## Infrastructure
//!
//! Uses `Compositor::new_headless` + `HeadlessSurface::new` (matching the pattern
//! of the inline test helper `make_compositor_and_surface` in renderer.rs), plus
//! `HeadlessSurface::assert_pixel_color` and `HeadlessSurface::pixel_at` for pixel
//! inspection.
//!
//! ## sRGB note
//!
//! The headless surface uses `Rgba8UnormSrgb`. wgpu applies linear→sRGB gamma
//! conversion automatically when writing to the framebuffer. Tolerances of ±8
//! are used to accommodate software-renderer (llvmpipe / WARP) rounding differences.
//!
//! ## References
//!
//! - hud-gwhr.1 (this task)
//! - hud-gwhr (parent epic: exemplar-ambient-background)
//! - design.md §Decision 3: Tests validate renderer output, not just scene state

use tze_hud_compositor::{Compositor, surface::HeadlessSurface};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{Rgba, ZoneContent, ZoneRegistry};

// ─── Helper ───────────────────────────────────────────────────────────────────

/// Create a headless compositor and matching surface pair.
///
/// Mirrors the private `make_compositor_and_surface` helper in renderer.rs.
async fn make_compositor_and_surface(w: u32, h: u32) -> (Compositor, HeadlessSurface) {
    let compositor = Compositor::new_headless(w, h)
        .await
        .expect("headless compositor must be creatable in test environment");
    let surface = HeadlessSurface::new(&compositor.device, w, h);
    (compositor, surface)
}

/// Create a fresh `SceneGraph` with the full default zone registry
/// (subtitle, notification-area, status-bar, pip, ambient-background, alert-banner).
fn scene_with_defaults(w: f32, h: f32) -> SceneGraph {
    let mut scene = SceneGraph::new(w, h);
    scene.zone_registry = ZoneRegistry::with_defaults();
    scene
}

// ─── sRGB expected values ─────────────────────────────────────────────────────
//
// The compositor renders to Rgba8UnormSrgb; wgpu converts linear→sRGB on write.
// Approximate expected byte values for the test colors (linear → sRGB → u8):
//
//   Rgba { r:0.1, g:0.1, b:0.3, a:1.0 } (dark blue)
//     r: 1.055*0.1^(1/2.4)-0.055 ≈ 0.349 → ~89
//     g: same ≈ 89
//     b: 1.055*0.3^(1/2.4)-0.055 ≈ 0.581 → ~148
//
//   Rgba { r:1.0, g:0.0, b:0.0, a:1.0 } (red) → sRGB [255, 0, 0]
//   Rgba { r:0.0, g:0.0, b:1.0, a:1.0 } (blue) → sRGB [0, 0, 255]
//
//   Clear color (non-overlay): linear { r:0.05, g:0.05, b:0.1, a:1.0 }
//     r: 1.055*0.05^(1/2.4)-0.055 ≈ 0.242 → ~62 (approximate, closer to 56-65)
//     b: ≈ 0.349 → ~89
//
// Tolerances of ±8 accommodate software-renderer (llvmpipe / WARP) differences
// and minor platform-specific precision variation.

const TOLERANCE: u8 = 8;

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Requirement: Ambient Background Zone Visual Contract
/// Scenario: "Solid color background fills the display"
///
/// Publish `SolidColor(Rgba { r:0.1, g:0.1, b:0.3, a:1.0 })` to the
/// `ambient-background` zone, render a frame, and assert that the rendered
/// pixels are distinctly dark blue (blue channel dominant, not the clear color).
///
/// The ambient-background zone is registered with `Relative { 0,0,1,1 }` geometry
/// so its SolidColor quad covers the full display surface.
#[tokio::test]
async fn test_ambient_background_solid_color_renders() {
    let (mut compositor, surface) = make_compositor_and_surface(64, 64).await;
    let mut scene = scene_with_defaults(64.0, 64.0);

    // Publish dark-blue solid color to ambient-background.
    scene
        .publish_to_zone(
            "ambient-background",
            ZoneContent::SolidColor(Rgba {
                r: 0.1,
                g: 0.1,
                b: 0.3,
                a: 1.0,
            }),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish_to_zone must succeed for SolidColor on ambient-background zone");

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    assert_eq!(
        pixels.len(),
        64 * 64 * 4,
        "pixel buffer must be 64×64×4 bytes"
    );

    // Sample corners and centre to confirm the ambient-background quad covers the full surface.
    // Expected sRGB bytes: R≈89, G≈89, B≈148 (linear 0.1,0.1,0.3 → sRGB gamma).
    //
    // Sampling only the centre would pass even if the zone quad were incorrectly
    // sized/positioned yet still covered the centre.  Corners + centre together
    // validate full-screen coverage.
    let sample_points: &[(u32, u32)] = &[
        (0, 0),   // top-left corner
        (63, 0),  // top-right corner
        (0, 63),  // bottom-left corner
        (63, 63), // bottom-right corner
        (32, 32), // centre
    ];

    for &(cx, cy) in sample_points {
        HeadlessSurface::assert_pixel_color(
            &pixels,
            64,
            cx,
            cy,
            [89, 89, 148, 255],
            TOLERANCE,
            "ambient-background SolidColor",
        )
        .unwrap_or_else(|e| panic!("pixel assertion failed at ({cx},{cy}): {e}"));
    }
}

/// Requirement: Ambient Background Zone Visual Contract
/// Scenario: "No publication yields transparent/clear background"
///
/// When no content has been published to `ambient-background`, the zone's
/// `active_publishes` list MUST be empty and the background layer MUST render
/// as the runtime clear color with no zone-specific visual output.
///
/// We verify both the scene state (empty active_publishes) and that the rendered
/// pixels match the clear color rather than any zone backdrop color.
#[tokio::test]
async fn test_ambient_background_no_publication_empty() {
    let (mut compositor, surface) = make_compositor_and_surface(64, 64).await;
    let scene = scene_with_defaults(64.0, 64.0);

    // Assert: no publications in zone before render.
    let ambient_publishes = scene
        .zone_registry
        .active_publishes
        .get("ambient-background");
    let is_empty = ambient_publishes.map(|v| v.is_empty()).unwrap_or(true);
    assert!(
        is_empty,
        "ambient-background active_publishes must be empty when no content has been published"
    );

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    assert_eq!(
        pixels.len(),
        64 * 64 * 4,
        "pixel buffer must be 64×64×4 bytes"
    );

    // With no zone publication, the centre pixel should match the runtime clear color.
    // Clear color: linear {r:0.05, g:0.05, b:0.1, a:1.0} → sRGB ≈ {R≈62, G≈62, B≈89}.
    // A published dark-blue zone would give ≈{R:89, G:89, B:148} — well above clear.
    let cx = 32u32;
    let cy = 32u32;
    HeadlessSurface::assert_pixel_color(
        &pixels,
        64,
        cx,
        cy,
        [62, 62, 89, 255],
        TOLERANCE,
        "clear color (no ambient-background publication)",
    )
    .expect("rendered pixel must match the runtime clear color when no ambient-background content is published");
}

/// Requirement: Ambient Background Latest-Wins Contention
/// Scenario: "New solid color replaces previous solid color"
///
/// Publish red, then blue, to `ambient-background`. After the second publish
/// the zone's `active_publishes` count MUST be 1 (Replace policy), and the
/// rendered background MUST show blue pixels (the second publication).
#[tokio::test]
async fn test_ambient_background_replacement_contention() {
    let (mut compositor, surface) = make_compositor_and_surface(64, 64).await;
    let mut scene = scene_with_defaults(64.0, 64.0);

    // First publish: red.
    scene
        .publish_to_zone(
            "ambient-background",
            ZoneContent::SolidColor(Rgba {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            }),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("first publish (red) must succeed");

    // Second publish: blue — must evict red under Replace policy.
    scene
        .publish_to_zone(
            "ambient-background",
            ZoneContent::SolidColor(Rgba {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            }),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("second publish (blue) must succeed");

    // Scene state: Replace policy allows only one active publication.
    let pub_count = scene
        .zone_registry
        .active_publishes
        .get("ambient-background")
        .map(|v| v.len())
        .unwrap_or(0);
    assert_eq!(
        pub_count, 1,
        "ambient-background active publication count MUST be 1 after Replace (latest-wins); got {pub_count}"
    );

    // Render and verify only blue appears (not red).
    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    // Centre pixel must show pure blue (second publication); red must not bleed through.
    // linear (0,0,1) → sRGB blue ≈ 255; linear (1,0,0) → sRGB red = 255.
    let cx = 32u32;
    let cy = 32u32;
    HeadlessSurface::assert_pixel_color(
        &pixels,
        64,
        cx,
        cy,
        [0, 0, 255, 255],
        TOLERANCE,
        "blue replaces red",
    )
    .expect("rendered pixel must be pure blue — first (red) publish must not bleed through");
}

/// Requirement: Ambient Background Latest-Wins Contention
/// Scenario: "Rapid replacement under contention"
///
/// Publish 10 different solid colors in sequence (within a single frame interval).
/// After all publishes, the zone's `active_publishes` count MUST be exactly 1
/// (Replace policy), and the rendered frame MUST show only the last published color.
///
/// The last color is a saturated green `Rgba { r:0.0, g:1.0, b:0.0, a:1.0 }`.
#[tokio::test]
async fn test_ambient_background_rapid_replacement() {
    let (mut compositor, surface) = make_compositor_and_surface(64, 64).await;
    let mut scene = scene_with_defaults(64.0, 64.0);

    // Publish 10 different solid colors in rapid succession (no frame render between them).
    // The last published color (index 9) is saturated green.
    let colors: Vec<Rgba> = vec![
        Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }, // 0: red
        Rgba {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        }, // 1: blue
        Rgba {
            r: 1.0,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        }, // 2: yellow
        Rgba {
            r: 1.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        }, // 3: magenta
        Rgba {
            r: 0.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }, // 4: cyan
        Rgba {
            r: 0.5,
            g: 0.5,
            b: 0.5,
            a: 1.0,
        }, // 5: gray
        Rgba {
            r: 1.0,
            g: 0.5,
            b: 0.0,
            a: 1.0,
        }, // 6: orange
        Rgba {
            r: 0.5,
            g: 0.0,
            b: 0.5,
            a: 1.0,
        }, // 7: purple
        Rgba {
            r: 0.0,
            g: 0.5,
            b: 0.0,
            a: 1.0,
        }, // 8: dark green
        Rgba {
            r: 0.0,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        }, // 9: bright green (last)
    ];

    for color in &colors {
        scene
            .publish_to_zone(
                "ambient-background",
                ZoneContent::SolidColor(*color),
                "test-agent",
                None,
                None,
                None,
            )
            .expect("rapid publish must succeed for ambient-background zone");
    }

    // Scene state: Replace policy evicts on every publish → exactly 1 active record.
    let pub_count = scene
        .zone_registry
        .active_publishes
        .get("ambient-background")
        .map(|v| v.len())
        .unwrap_or(0);
    assert_eq!(
        pub_count, 1,
        "ambient-background active publication count MUST be exactly 1 after 10 rapid Replace \
         publishes; got {pub_count}"
    );

    // Render a single frame and verify only the last color (bright green) is visible.
    // linear (0,1,0) → sRGB green ≈ 255.
    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    // The last published color (index 9) is bright green: linear (0,1,0) → sRGB ≈ [0, 255, 0].
    // All prior colors (red, blue, yellow, …) must not bleed through.
    let cx = 32u32;
    let cy = 32u32;
    HeadlessSurface::assert_pixel_color(
        &pixels,
        64,
        cx,
        cy,
        [0, 255, 0, 255],
        TOLERANCE,
        "rapid replacement green",
    )
    .expect("rendered pixel must match the last published color (bright green) — earlier publishes must not bleed through");
}
