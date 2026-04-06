//! Headless visual regression tests for the SDF rounded-rectangle pipeline.
//!
//! Covers issue hud-ltgk.1 — Add SDF rounded rectangle pipeline to compositor.
//!
//! ## Test list
//!
//! 1. `test_rounded_rect_backdrop_interior_pixel` — Zone with `backdrop_radius > 0`
//!    renders a backdrop; sampling the interior of the zone shows the backdrop color.
//!
//! 2. `test_rounded_rect_corners_are_transparent` — The four corners of the zone's
//!    bounding box are transparent (show clear color) when `backdrop_radius` is
//!    large enough to round them off.
//!
//! 3. `test_flat_rect_unaffected_by_rounded_rect_pipeline` — A second zone with
//!    `backdrop_radius = None` still renders a sharp flat rectangle (no regression
//!    on the existing pipeline).
//!
//! 4. `test_no_backdrop_no_rounded_rect` — A zone with `backdrop = None` and
//!    `backdrop_radius = Some(8.0)` produces no visible backdrop (consistent with
//!    the existing flat-rect contract: backdrop_radius only activates when a
//!    backdrop color is set).
//!
//! ## Infrastructure
//!
//! Uses `Compositor::new_headless` + `HeadlessSurface::new` + `render_frame_headless`,
//! then `HeadlessSurface::read_pixels` and `HeadlessSurface::assert_pixel_color`.
//!
//! Set `TZE_HUD_SKIP_GPU_TESTS=1` to skip all GPU-dependent tests.
//! Set `HEADLESS_FORCE_SOFTWARE=1` to use llvmpipe on CI.

use tze_hud_compositor::{Compositor, CompositorError, surface::HeadlessSurface};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{
    ContentionPolicy, GeometryPolicy, LayerAttachment, RenderingPolicy, Rgba, SceneId,
    ZoneContent, ZoneDefinition, ZoneMediaType, ZoneRegistry,
};

// ─── Constants ────────────────────────────────────────────────────────────────

const SURFACE_W: u32 = 256;
const SURFACE_H: u32 = 256;

/// Pixel tolerance for GPU comparisons (accounts for software-renderer rounding).
const TOLERANCE: u8 = 12;

/// The compositor's clear color (linear {r:0.05, g:0.05, b:0.1, a:1.0})
/// maps to sRGB ≈ [63, 63, 89].
const CLEAR_COLOR_EXPECTED: [u8; 4] = [63, 63, 89, 255];

// ─── Helpers ──────────────────────────────────────────────────────────────────

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

macro_rules! gpu_or_skip {
    ($expr:expr) => {
        match $expr {
            Some(v) => v,
            None => return,
        }
    };
}

/// Register a test zone that occupies the centre of the display (50% wide, 20% tall).
///
/// The zone geometry for a 256×256 surface:
///   x = 256 × 0.25 = 64.0
///   y = 256 × 0.40 = 102.4  → ≈ 102
///   w = 256 × 0.50 = 128.0
///   h = 256 × 0.20 = 51.2   → ≈ 51
///   centre: (128, 128)
///
/// With `backdrop_radius = 16.0`, the 16px corners are rounded off.  The
/// centre pixel (128, 128) is well inside the shape regardless of the radius.
fn register_rounded_zone(scene: &mut SceneGraph, radius: Option<f32>) -> &'static str {
    let name = "rounded-test";
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: name.to_owned(),
        description: "Test zone for rounded-rect SDF pipeline".to_owned(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.25,
            y_pct: 0.40,
            width_pct: 0.50,
            height_pct: 0.20,
        },
        accepted_media_types: vec![ZoneMediaType::StreamText],
        rendering_policy: RenderingPolicy {
            // Use a bright green backdrop so it's distinct from the clear color.
            backdrop: Some(Rgba {
                r: 0.0,
                g: 0.5,
                b: 0.0,
                a: 1.0,
            }),
            backdrop_radius: radius,
            ..Default::default()
        },
        contention_policy: ContentionPolicy::LatestWins,
        max_publishers: 1,
        transport_constraint: None,
        auto_clear_ms: None,
        ephemeral: false,
        layer_attachment: LayerAttachment::Content,
    });
    name
}

fn publish_text(scene: &mut SceneGraph, zone_name: &str) {
    scene
        .publish_to_zone(
            zone_name,
            ZoneContent::StreamText("test".to_string()),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish_to_zone must succeed");
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Requirement: SDF rounded-rect pipeline — interior pixel shows backdrop color.
///
/// A zone with `backdrop_radius = Some(16.0)` and a green backdrop must render
/// the backdrop at the zone's interior centre pixel.
///
/// The centre pixel (128, 128) is far from the rounded corners, so the SDF
/// value is strongly negative and the alpha should be 1.0.
#[tokio::test]
async fn test_rounded_rect_backdrop_interior_pixel() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);

    let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
    scene.zone_registry = ZoneRegistry::with_defaults();

    let name = register_rounded_zone(&mut scene, Some(16.0));
    publish_text(&mut scene, name);

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    // Linear green (0.0, 0.5, 0.0) → sRGB ≈ (0, 188, 0).
    // Allow generous tolerance for software-renderer variation.
    let expected_green: [u8; 4] = [0, 188, 0, 255];

    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        128, // centre x
        128, // centre y — well inside the zone
        expected_green,
        TOLERANCE,
        "interior of rounded-rect zone must show green backdrop",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// Requirement: SDF rounded-rect pipeline — corners are transparent.
///
/// With `backdrop_radius = 40.0` on a zone that is ~128px wide and ~51px tall,
/// the corner radius is clamped to ~25px (half the shorter side).  The exact
/// corner pixels (top-left of the bounding box) must show the clear color
/// rather than the backdrop color, because the SDF computes positive distance
/// at the corners (outside the shape).
///
/// Zone geometry (256×256):
///   x=64, y=102, w=128, h=51
///   Top-left corner of the bounding box: (64, 102)
///   That pixel is at the extreme corner — outside the rounded shape.
#[tokio::test]
async fn test_rounded_rect_corners_are_transparent() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);

    let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
    scene.zone_registry = ZoneRegistry::with_defaults();

    // Use a large radius so the corners are visibly rounded off.
    // radius=40 will be clamped to min(128/2, 51/2)=25.
    let name = register_rounded_zone(&mut scene, Some(40.0));
    publish_text(&mut scene, name);

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    // Sample the very top-left corner of the bounding box (pixel 64, 102).
    // This pixel is outside the rounded shape and must be the clear color.
    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        64,  // zone left edge
        102, // zone top edge
        CLEAR_COLOR_EXPECTED,
        TOLERANCE,
        "corner pixel of rounded-rect zone must be transparent (show clear color)",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// Requirement: Existing flat-rect rendering unaffected.
///
/// A zone with `backdrop_radius = None` continues to use the flat-rect pipeline.
/// The interior pixel of such a zone must show the backdrop color (not clear color).
#[tokio::test]
async fn test_flat_rect_unaffected_by_rounded_rect_pipeline() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);

    let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
    scene.zone_registry = ZoneRegistry::with_defaults();

    // backdrop_radius = None → flat rect path.
    let name = register_rounded_zone(&mut scene, None);
    publish_text(&mut scene, name);

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    let expected_green: [u8; 4] = [0, 188, 0, 255];

    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        128,
        128,
        expected_green,
        TOLERANCE,
        "flat-rect zone interior must still show green backdrop (no regression)",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// Requirement: backdrop_radius without backdrop color → no visible backdrop.
///
/// When `backdrop = None` and `backdrop_radius = Some(8.0)`, no backdrop quad
/// is emitted (neither flat-rect nor rounded-rect).  The sampled pixel must
/// remain the clear color.
#[tokio::test]
async fn test_no_backdrop_no_rounded_rect() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);

    let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
    scene.zone_registry = ZoneRegistry::with_defaults();

    // Register with backdrop = None (overrides the helper's green).
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "no-backdrop-zone".to_owned(),
        description: "Zone with radius but no backdrop color".to_owned(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.25,
            y_pct: 0.40,
            width_pct: 0.50,
            height_pct: 0.20,
        },
        accepted_media_types: vec![ZoneMediaType::StreamText],
        rendering_policy: RenderingPolicy {
            backdrop: None,
            backdrop_radius: Some(8.0),
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
            "no-backdrop-zone",
            ZoneContent::StreamText("hello".to_string()),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish_to_zone must succeed");

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    // No backdrop was registered, so the pixel must show the clear color.
    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        128,
        128,
        CLEAR_COLOR_EXPECTED,
        TOLERANCE,
        "zone with backdrop=None must not render any backdrop even with backdrop_radius set",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}
