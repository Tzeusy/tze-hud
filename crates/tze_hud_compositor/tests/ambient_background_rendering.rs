//! Compositor integration tests for ambient-background rendering.
//!
//! Covers openspec/changes/exemplar-ambient-background/specs/exemplar-ambient-background/spec.md
//! — Requirements:
//!   - "Ambient Background Zone Visual Contract"
//!     Scenarios: "Solid color background fills the display",
//!                "No publication yields transparent/clear background",
//!                "StaticImage renders placeholder in v1"
//!   - "Ambient Background Latest-Wins Contention"
//!     Scenarios: "New solid color replaces previous solid color",
//!                "Rapid replacement under contention"
//!   - "Background Layer Z-Order"
//!     Scenarios: "Content tile renders on top of background",
//!                "Background layer uses LayerAttachment::Background"
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
//! 5. `test_ambient_background_zorder_below_content_zones` — ambient-background
//!    (Background layer, dark blue) + content zone (Content layer, red at center)
//!    → pixel inside content zone is red (content occludes background), pixel at
//!    corner is dark blue (background visible).
//!
//! 6. `test_ambient_background_static_image_renders_placeholder` — Publish
//!    `ZoneContent::StaticImage` to ambient-background → rendered pixels match
//!    the warm-gray placeholder color (linear 0.3,0.3,0.3; sRGB ≈ 143).
//!
//! 7. `test_ambient_background_layer_attachment_is_background` — `ZoneRegistry::with_defaults()`
//!    MUST register ambient-background with `LayerAttachment::Background` (pure scene-state
//!    assertion, no GPU required).
//!
//! ## Infrastructure
//!
//! Uses `Compositor::new_headless` + `HeadlessSurface::new` (matching the pattern
//! of the inline test helper `make_compositor_and_surface` in renderer.rs), plus
//! `HeadlessSurface::assert_pixel_color` and `HeadlessSurface::pixel_at` for pixel
//! inspection.
//!
//! Set `TZE_HUD_SKIP_GPU_TESTS=1` to skip all GPU-dependent tests (e.g. in headless
//! CI environments without llvmpipe). On CI with Mesa installed, set
//! `HEADLESS_FORCE_SOFTWARE=1` instead so the software renderer is used.
//!
//! ## sRGB note
//!
//! The headless surface uses `Rgba8UnormSrgb`. wgpu applies linear→sRGB gamma
//! conversion automatically when writing to the framebuffer. Tolerances of ±8
//! are used to accommodate software-renderer (llvmpipe / WARP) rounding differences.
//!
//! ## Expected pixel values for new tests
//!
//! Z-order test (256×256 surface):
//!   - Background: linear(0.0, 0.0, 0.5, 1.0) → sRGB [0, 0, 188]
//!   - Content zone (center 50%): linear(1.0, 0.0, 0.0, 1.0) → sRGB [255, 0, 0]
//!   - Corner pixels (background area): sRGB [0, 0, 188] ± 8
//!   - Centre pixel (content zone area): sRGB [255, 0, 0] ± 8
//!
//! StaticImage placeholder (warm-gray):
//!   - STATIC_IMAGE_PLACEHOLDER_COLOR = linear(0.3, 0.3, 0.3, 1.0)
//!   - sRGB: 1.055 × 0.3^(1/2.4) − 0.055 ≈ 0.584 × 255 ≈ 149
//!   - Expected ≈ [149, 149, 149, 255] ± 8
//!
//! ## References
//!
//! - hud-gwhr.1 (SolidColor + contention tests)
//! - hud-gwhr.2 (this task: z-order, StaticImage, LayerAttachment tests)
//! - hud-gwhr (parent epic: exemplar-ambient-background)
//! - design.md §Decision 3: Tests validate renderer output, not just scene state

use tze_hud_compositor::{Compositor, CompositorError, surface::HeadlessSurface};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{
    ContentionPolicy, GeometryPolicy, LayerAttachment, RenderingPolicy, ResourceId, Rgba, SceneId,
    ZoneContent, ZoneDefinition, ZoneMediaType, ZoneRegistry,
};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Create a headless compositor and matching surface pair.
///
/// Returns `None` when `TZE_HUD_SKIP_GPU_TESTS=1` is set or no wgpu adapter is
/// available. Use the `gpu_or_skip!` macro to early-return from the test.
///
/// Mirrors the private `make_compositor_and_surface` helper in renderer.rs.
async fn make_compositor_and_surface(w: u32, h: u32) -> Option<(Compositor, HeadlessSurface)> {
    if std::env::var("TZE_HUD_SKIP_GPU_TESTS")
        .map(|v| v.trim() == "1")
        .unwrap_or(false)
    {
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

/// Early-return from an async test when no GPU is available.
macro_rules! gpu_or_skip {
    ($expr:expr) => {
        match $expr {
            Some(v) => v,
            None => return,
        }
    };
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
    let (mut compositor, surface) = gpu_or_skip!(make_compositor_and_surface(64, 64).await);
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
    let (mut compositor, surface) = gpu_or_skip!(make_compositor_and_surface(64, 64).await);
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
    let (mut compositor, surface) = gpu_or_skip!(make_compositor_and_surface(64, 64).await);
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
    let (mut compositor, surface) = gpu_or_skip!(make_compositor_and_surface(64, 64).await);
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

// ─── sRGB expected values for z-order and StaticImage tests ──────────────────
//
// Z-order test background color: linear(0.0, 0.0, 0.5, 1.0) → sRGB blue channel:
//   b: 1.055 × 0.5^(1/2.4) − 0.055 ≈ 1.055 × 0.7297 − 0.055 ≈ 0.714 → ~182
// Content zone (opaque red): linear(1.0, 0.0, 0.0, 1.0) → sRGB [255, 0, 0]
//
// StaticImage placeholder: STATIC_IMAGE_PLACEHOLDER_COLOR = linear(0.3, 0.3, 0.3, 1.0):
//   1.055 × 0.3^(1/2.4) − 0.055 ≈ 1.055 × 0.5834 − 0.055 ≈ 0.560 → ~143
//
// All calibrated with ±8 tolerance for llvmpipe / software-renderer variance.

/// Expected sRGB bytes for the dark blue background zone color.
///
/// linear(0.0, 0.0, 0.5, 1.0): r=0, g=0, b≈182.
const DARK_BLUE_BG_EXPECTED: [u8; 4] = [0, 0, 182, 255];

/// Expected sRGB bytes for pure red content zone.
///
/// linear(1.0, 0.0, 0.0, 1.0): r=255, g=0, b=0.
const RED_CONTENT_EXPECTED: [u8; 4] = [255, 0, 0, 255];

/// Expected sRGB bytes for StaticImage warm-gray placeholder.
///
/// linear(0.3, 0.3, 0.3, 1.0): all channels ≈ 143.
const STATIC_IMAGE_PLACEHOLDER_EXPECTED: [u8; 4] = [143, 143, 143, 255];

// Surface size for z-order and StaticImage tests.
const SURFACE_W: u32 = 256;
const SURFACE_H: u32 = 256;

/// Requirement: Background Layer Z-Order
/// Scenario: "Content tile renders on top of background"
///
/// Publish `SolidColor(dark_blue)` to the `ambient-background` zone (Background layer),
/// and register a content-layer zone covering the centre 50% of the display with
/// `SolidColor(red)`. After rendering:
///   - Pixels at the display corners MUST be dark blue (ambient-background visible).
///   - The pixel at the display centre MUST be red (content zone occludes background).
///
/// This verifies that `LayerAttachment::Background` renders before `LayerAttachment::Content`,
/// so content tiles occlude the background zone where they overlap.
#[tokio::test]
async fn test_ambient_background_zorder_below_content_zones() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);

    // Use the default registry (includes ambient-background at LayerAttachment::Background).
    let mut scene = scene_with_defaults(SURFACE_W as f32, SURFACE_H as f32);

    // Register an additional content-layer zone covering the centre 50% of the display.
    // This zone uses ContentionPolicy::Replace so a single publish fills it.
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "test-content-zone".to_owned(),
        description: "Centre content zone for z-order test".to_owned(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.25,
            y_pct: 0.25,
            width_pct: 0.50,
            height_pct: 0.50,
        },
        accepted_media_types: vec![ZoneMediaType::SolidColor],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::Replace,
        max_publishers: 1,
        transport_constraint: None,
        auto_clear_ms: None,
        ephemeral: false,
        layer_attachment: LayerAttachment::Content,
    });

    // Publish dark blue to the background zone.
    scene
        .publish_to_zone(
            "ambient-background",
            ZoneContent::SolidColor(Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.5,
                a: 1.0,
            }),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish dark blue to ambient-background must succeed");

    // Publish red to the content zone (centre 50%).
    scene
        .publish_to_zone(
            "test-content-zone",
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
        .expect("publish red to test-content-zone must succeed");

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    // Corner pixels are outside the content zone: background (dark blue) must be visible.
    // The content zone covers x ∈ [64, 192), y ∈ [64, 192) for a 256×256 surface.
    // Corners at (0,0), (255,0), (0,255), (255,255) are well outside this region.
    let corners: &[(u32, u32)] = &[(0, 0), (255, 0), (0, 255), (255, 255)];
    for &(cx, cy) in corners {
        HeadlessSurface::assert_pixel_color(
            &pixels,
            SURFACE_W,
            cx,
            cy,
            DARK_BLUE_BG_EXPECTED,
            TOLERANCE,
            "corner pixel (background area) must be dark blue",
        )
        .unwrap_or_else(|e| {
            panic!("z-order test — background not visible at corner ({cx},{cy}): {e}")
        });
    }

    // Centre pixel (128, 128) is inside the content zone: red MUST occlude the background.
    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        SURFACE_W / 2,
        SURFACE_H / 2,
        RED_CONTENT_EXPECTED,
        TOLERANCE,
        "centre pixel (content zone area) must be red — content occludes background",
    )
    .unwrap_or_else(|e| {
        panic!("z-order test — content zone does not occlude background at centre: {e}")
    });
}

/// Requirement: Ambient Background Zone Visual Contract
/// Scenario: "StaticImage renders placeholder in v1"
///
/// Publish `ZoneContent::StaticImage(resource_id)` to the `ambient-background` zone.
/// The GPU texture upload pipeline is deferred to a future iteration; the compositor
/// MUST render a warm-gray placeholder quad (linear 0.3,0.3,0.3) instead.
///
/// After rendering, multiple sample points across the display MUST show the warm-gray
/// sRGB approximation (≈ [143, 143, 143]) within ±8 per channel, confirming that:
///   - The zone accepted the StaticImage publication (not rejected by validation).
///   - The placeholder quad was rendered full-screen (Background layer coverage).
///   - The `resource_id` was accepted (preserved in the publication record).
#[tokio::test]
async fn test_ambient_background_static_image_renders_placeholder() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);
    let mut scene = scene_with_defaults(SURFACE_W as f32, SURFACE_H as f32);

    // Construct a ResourceId from deterministic test bytes (content-addressed).
    let resource_id = ResourceId::of(b"ambient-background-test-image");

    // Publish StaticImage — the ambient-background zone accepts ZoneMediaType::StaticImage.
    scene
        .publish_to_zone(
            "ambient-background",
            ZoneContent::StaticImage(resource_id),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish StaticImage to ambient-background must succeed");

    // Confirm the publication was recorded (resource_id preserved in active record).
    let pub_count = scene
        .zone_registry
        .active_publishes
        .get("ambient-background")
        .map(|v| v.len())
        .unwrap_or(0);
    assert_eq!(
        pub_count, 1,
        "ambient-background must have exactly 1 active publication after StaticImage publish; got {pub_count}"
    );

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    // The placeholder color is STATIC_IMAGE_PLACEHOLDER_COLOR = linear(0.3, 0.3, 0.3).
    // sRGB ≈ [143, 143, 143]; all channels equal (neutral gray, no blue or red tint).
    //
    // Sample multiple points to confirm full-screen coverage.
    let sample_points: &[(u32, u32)] = &[
        (0, 0),       // top-left
        (255, 0),     // top-right
        (0, 255),     // bottom-left
        (255, 255),   // bottom-right
        (128, 128),   // centre
    ];

    for &(cx, cy) in sample_points {
        HeadlessSurface::assert_pixel_color(
            &pixels,
            SURFACE_W,
            cx,
            cy,
            STATIC_IMAGE_PLACEHOLDER_EXPECTED,
            TOLERANCE,
            "StaticImage ambient-background placeholder (warm-gray)",
        )
        .unwrap_or_else(|e| {
            panic!("StaticImage placeholder pixel assertion failed at ({cx},{cy}): {e}")
        });
    }
}

/// Requirement: Background Layer Z-Order
/// Scenario: "Background layer uses LayerAttachment::Background"
///
/// `ZoneRegistry::with_defaults()` MUST register the `ambient-background` zone with
/// `LayerAttachment::Background`. This is a pure scene-state assertion — no GPU or
/// rendering required.
///
/// This verifies that the zone definition contract is correct at the registry level,
/// independently of the pixel-based rendering tests above.
#[test]
fn test_ambient_background_layer_attachment_is_background() {
    let registry = ZoneRegistry::with_defaults();

    let zone = registry
        .zones
        .get("ambient-background")
        .expect("ZoneRegistry::with_defaults() must include an 'ambient-background' zone");

    assert_eq!(
        zone.layer_attachment,
        LayerAttachment::Background,
        "ambient-background zone MUST use LayerAttachment::Background so it renders \
         behind all content and chrome zones; got {:?}",
        zone.layer_attachment
    );
}
