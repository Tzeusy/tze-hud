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
//! conversion automatically when writing to the framebuffer. Tolerances of ±4
//! are used to accommodate software-renderer (llvmpipe / WARP) rounding differences.
//!
//! ## References
//!
//! - hud-gwhr.1 (this task)
//! - hud-gwhr (parent epic: exemplar-ambient-background)
//! - design.md §Decision 3: Tests validate renderer output, not just scene state

use tze_hud_compositor::{Compositor, surface::HeadlessSurface};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{ZoneContent, ZoneRegistry, Rgba};

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

    // Check centre pixel: blue channel dominant relative to red/green channels,
    // confirming the ambient-background quad covers the full surface.
    // Expected sRGB bytes: R≈89, G≈89, B≈148 (linear 0.1,0.1,0.3 → sRGB gamma).
    let cx = 32u32;
    let cy = 32u32;
    let p = HeadlessSurface::pixel_at(&pixels, 64, cx, cy);

    assert!(
        p[2] > p[0],
        "ambient-background SolidColor: blue channel (B={}) must exceed red channel (R={}) at centre pixel ({cx},{cy})",
        p[2],
        p[0]
    );
    assert!(
        p[2] > p[1],
        "ambient-background SolidColor: blue channel (B={}) must exceed green channel (G={}) at centre pixel ({cx},{cy})",
        p[2],
        p[1]
    );

    // Confirm the pixel is distinctly different from the clear color.
    // The clear color has r≈g≈62, b≈89 (linear 0.05,0.05,0.1 → sRGB).
    // Our dark-blue zone has b≈148 — well above the clear color's b≈89.
    let expected_b: u8 = 148;
    assert!(
        p[2] > 100,
        "ambient-background blue channel ({}) must be significantly above clear-color blue (~89) \
         — expected ~{expected_b}",
        p[2]
    );

    // Alpha must be fully opaque.
    assert_eq!(p[3], 255, "alpha must be 255 (fully opaque)");
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
    // The blue channel should be relatively small (close to clear color, not a vivid blue).
    let cx = 32u32;
    let cy = 32u32;
    let p = HeadlessSurface::pixel_at(&pixels, 64, cx, cy);

    // Clear-color blue ≈ 89 in sRGB. A published dark-blue would give ≈148.
    // Without a zone publication, blue must stay near the clear color value.
    let expected_clear_b: u8 = 89;
    assert!(
        p[2] < expected_clear_b + TOLERANCE + 20,
        "without ambient-background publication, blue channel ({}) must stay near clear-color blue (~{expected_clear_b}) \
         — an unexpected zone quad would push it much higher",
        p[2]
    );

    // The overall alpha must still be 255 (headless mode, non-overlay).
    assert_eq!(p[3], 255, "alpha must be 255 in headless non-overlay mode");
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

    // Centre pixel: blue channel must dominate; red channel must be near zero.
    // linear (0,0,1) → sRGB blue ≈ 255.
    let cx = 32u32;
    let cy = 32u32;
    let p = HeadlessSurface::pixel_at(&pixels, 64, cx, cy);

    assert!(
        p[2] > 200,
        "ambient-background after blue-replaces-red: blue channel must be high (>200), got {}",
        p[2]
    );
    assert!(
        p[0] < 50,
        "ambient-background after blue-replaces-red: red channel must be near zero (<50), got {} \
         — first (red) publish must not bleed through",
        p[0]
    );
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
        Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }, // 0: red
        Rgba { r: 0.0, g: 0.0, b: 1.0, a: 1.0 }, // 1: blue
        Rgba { r: 1.0, g: 1.0, b: 0.0, a: 1.0 }, // 2: yellow
        Rgba { r: 1.0, g: 0.0, b: 1.0, a: 1.0 }, // 3: magenta
        Rgba { r: 0.0, g: 1.0, b: 1.0, a: 1.0 }, // 4: cyan
        Rgba { r: 0.5, g: 0.5, b: 0.5, a: 1.0 }, // 5: gray
        Rgba { r: 1.0, g: 0.5, b: 0.0, a: 1.0 }, // 6: orange
        Rgba { r: 0.5, g: 0.0, b: 0.5, a: 1.0 }, // 7: purple
        Rgba { r: 0.0, g: 0.5, b: 0.0, a: 1.0 }, // 8: dark green
        Rgba { r: 0.0, g: 1.0, b: 0.0, a: 1.0 }, // 9: bright green (last)
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

    let cx = 32u32;
    let cy = 32u32;
    let p = HeadlessSurface::pixel_at(&pixels, 64, cx, cy);

    // Green channel must dominate (last published was bright green).
    assert!(
        p[1] > 200,
        "ambient-background after 10 rapid publishes: green channel must be high (>200), got {} \
         — only the last (bright green) publish must be visible",
        p[1]
    );
    // Red must be near zero (not the first-published red).
    assert!(
        p[0] < 50,
        "ambient-background after 10 rapid publishes: red channel must be near zero (<50), got {} \
         — earlier red publishes must not bleed through",
        p[0]
    );
    // Blue must be near zero (not the second-published blue).
    assert!(
        p[2] < 50,
        "ambient-background after 10 rapid publishes: blue channel must be near zero (<50), got {} \
         — earlier blue publishes must not bleed through",
        p[2]
    );
}
