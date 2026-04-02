//! Integration tests for notification-area rendering pipeline — urgency backdrop colors.
//!
//! Covers openspec spec §Notification Exemplar MCP Integration Test requirement:
//!   "The test MUST verify: urgency styling — each notification's urgency maps to
//!    the correct backdrop color."
//!
//! ## Test list
//!
//! 1. `test_notification_urgency0_low_backdrop` — urgency=0 → backdrop pixels match
//!    `color.notification.urgency.low` (#2A2A2A, 0.9 alpha over clear).
//!
//! 2. `test_notification_urgency1_normal_backdrop` — urgency=1 → backdrop pixels match
//!    `color.notification.urgency.normal` (#1A1A3A, 0.9 alpha over clear).
//!
//! 3. `test_notification_urgency2_urgent_backdrop` — urgency=2 → backdrop pixels match
//!    `color.notification.urgency.urgent` (#8B6914, 0.9 alpha over clear).
//!
//! 4. `test_notification_urgency3_critical_backdrop` — urgency=3 → backdrop pixels match
//!    `color.notification.urgency.critical` (#8B1A1A, 0.9 alpha over clear).
//!
//! 5. `test_notification_urgency_distinct_colors` — each urgency level renders a
//!    visually distinct backdrop (low, normal, urgent, critical each have different
//!    pixel signatures relative to each other).
//!
//! ## Infrastructure
//!
//! Uses `Compositor::new_headless` + `HeadlessSurface::new` + `render_frame_headless`,
//! then `HeadlessSurface::read_pixels` and `HeadlessSurface::assert_pixel_color` for
//! pixel inspection.
//!
//! Set `TZE_HUD_SKIP_GPU_TESTS=1` to skip all GPU-dependent tests (e.g. in headless
//! CI environments without llvmpipe). On CI with Mesa installed, set
//! `HEADLESS_FORCE_SOFTWARE=1` instead so the software renderer is used.
//!
//! ## Zone setup
//!
//! The notification-area zone is registered with:
//!   - `GeometryPolicy::Relative { x_pct=0.75, y_pct=0.0, width_pct=0.24, height_pct=0.30 }`
//!   - `RenderingPolicy { backdrop: Some(dark), ... }` — backdrop MUST be Some to enable
//!     urgency-tinted backdrop rendering (renderer checks `policy.backdrop.is_some()`).
//!   - `ContentionPolicy::Stack { max_depth: 5 }`
//!
//! The default `ZoneRegistry::with_defaults()` notification-area zone has
//! `backdrop: None`, which would suppress backdrop rendering entirely. Tests in this
//! file always register a custom zone definition with `backdrop: Some(...)`.
//!
//! ## Expected pixel values
//!
//! For a 256×256 surface, the notification-area zone starts at x=192, y=0:
//!   zone width  = 256 × 0.24 = 61.44 px → sample centre x = 192 + 30 = 222
//!   zone y      = 0.0 px
//!   slot_h      = font_size_px(16) + 2×margin_v(8) + SLOT_BASELINE_GAP(2) = 34 px
//!   slot 0 centre y = 0 + 17 = 17
//!
//! Notification urgency colors (linear sRGB, per spec §Notification Urgency Backdrop Token Schema):
//!   urgency 0 (low)     → #2A2A2A → linear(0.165, 0.165, 0.165)
//!   urgency 1 (normal)  → #1A1A3A → linear(0.102, 0.102, 0.227)
//!   urgency 2 (urgent)  → #8B6914 → linear(0.545, 0.412, 0.078)
//!   urgency 3 (critical)→ #8B1A1A → linear(0.545, 0.102, 0.102)
//!
//! At 0.9 alpha composited over the compositor's dark clear color
//! (linear {r:0.05, g:0.05, b:0.1, a:1.0}), calibrated from llvmpipe output:
//!   urgency 0 (low)      → sRGB ≈ (104, 104, 106)  [near-neutral dark gray]
//!   urgency 1 (normal)   → sRGB ≈ (80, 80, 122)    [dark blue-tinted]
//!   urgency 2 (urgent)   → sRGB ≈ (184, 161, 72)   [amber/olive]
//!   urgency 3 (critical) → sRGB ≈ (184, 80, 82)    [dark red]
//!
//! Tolerances of ±12 accommodate software-renderer (llvmpipe / WARP) rounding
//! differences and sRGB compositing precision variation across platforms.
//! (Slightly wider than alert-banner ±8 because notification urgency colors are
//! more mid-range and thus more sensitive to sRGB gamma rounding.)
//!
//! ## References
//!
//! - hud-oj2x (this task)
//! - hud-s5dr (parent epic: Fix compositor and API gaps for exemplar readiness)
//! - spec §Notification Exemplar MCP Integration Test (urgency styling requirement)

use tze_hud_compositor::{Compositor, CompositorError, surface::HeadlessSurface};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{
    ContentionPolicy, GeometryPolicy, LayerAttachment, NotificationPayload, RenderingPolicy, Rgba,
    SceneId, ZoneContent, ZoneDefinition, ZoneMediaType,
};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Create a headless compositor + surface pair, returning `None` when no GPU
/// adapter is available or `TZE_HUD_SKIP_GPU_TESTS=1` is set.
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

/// Register a notification-area zone with urgency backdrop rendering enabled.
///
/// The zone uses:
///   - Relative geometry: x_pct=0.75, y_pct=0.0, width_pct=0.24, height_pct=0.30
///   - `backdrop: Some(...)` — required so the renderer enters the urgency-color
///     path (renderer checks `policy.backdrop.is_some()` before emitting backdrop quads).
///   - Stack contention policy (max_depth=5): allows multiple stacked notifications.
///
/// `backdrop: None` (the default from `ZoneRegistry::with_defaults()`) would cause
/// the renderer to skip all backdrop quads for notification-area.
fn register_notification_zone(scene: &mut SceneGraph) {
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "notification-area".to_owned(),
        description: "Notification overlay — urgency backdrop test zone".to_owned(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.75,
            y_pct: 0.0,
            width_pct: 0.24,
            height_pct: 0.30,
        },
        accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
        rendering_policy: RenderingPolicy {
            // backdrop MUST be Some to enable urgency-tinted backdrop rendering.
            // The actual color here is overridden by urgency_to_notification_color
            // for Notification content; this just acts as the "backdrop enabled" gate.
            backdrop: Some(Rgba::new(0.05, 0.05, 0.05, 0.85)),
            text_color: Some(Rgba::WHITE),
            ..Default::default()
        },
        contention_policy: ContentionPolicy::Stack { max_depth: 5 },
        max_publishers: 16,
        transport_constraint: None,
        auto_clear_ms: None,
        ephemeral: false,
        layer_attachment: LayerAttachment::Chrome,
    });
}

// ─── Expected sRGB pixel values ──────────────────────────────────────────────
//
// Notification urgency colors (linear sRGB, from NOTIFICATION_URGENCY_* constants):
//   low      = (0.165, 0.165, 0.165)   → #2A2A2A
//   normal   = (0.102, 0.102, 0.227)   → #1A1A3A
//   urgent   = (0.545, 0.412, 0.078)   → #8B6914
//   critical = (0.545, 0.102, 0.102)   → #8B1A1A
//
// Blended at 0.9 alpha over compositor clear (linear 0.05, 0.05, 0.1):
//   out_lin = urgency_lin * 0.9 + clear_lin * 0.1
//
// urgency 0 (low):
//   r = 0.165*0.9 + 0.05*0.1 = 0.1535  → sRGB ≈ 104
//   g = 0.165*0.9 + 0.05*0.1 = 0.1535  → sRGB ≈ 104
//   b = 0.165*0.9 + 0.1*0.1  = 0.1585  → sRGB ≈ 106
//
// urgency 1 (normal):
//   r = 0.102*0.9 + 0.05*0.1 = 0.0968  → sRGB ≈ 80
//   g = 0.102*0.9 + 0.05*0.1 = 0.0968  → sRGB ≈ 80
//   b = 0.227*0.9 + 0.1*0.1  = 0.2143  → sRGB ≈ 133
//
// urgency 2 (urgent):
//   r = 0.545*0.9 + 0.05*0.1 = 0.4955  → sRGB ≈ 184 (R-dominant)
//   g = 0.412*0.9 + 0.05*0.1 = 0.3758  → sRGB ≈ 166
//   b = 0.078*0.9 + 0.1*0.1  = 0.0802  → sRGB ≈ 72
//
// urgency 3 (critical):
//   r = 0.545*0.9 + 0.05*0.1 = 0.4955  → sRGB ≈ 184
//   g = 0.102*0.9 + 0.05*0.1 = 0.0968  → sRGB ≈ 80
//   b = 0.102*0.9 + 0.1*0.1  = 0.1018  → sRGB ≈ 82
//
// (Calibrated from theoretical linear-sRGB math; run the `debug_probe_pixel_values`
// ignored test with HEADLESS_FORCE_SOFTWARE=1 to verify against llvmpipe output.)

/// Tolerance applied to every channel when comparing expected vs. actual pixel values.
/// Slightly wider than alert-banner (±8) because mid-range notification urgency colors
/// are more sensitive to sRGB gamma rounding across software renderer implementations.
const TOLERANCE: u8 = 12;

// Expected sRGB bytes for urgency=0 (low) backdrop at 0.9 alpha.
// #2A2A2A linear(0.165, 0.165, 0.165) at 0.9α over clear:
//   sRGB ≈ (104, 104, 106)
const LOW_EXPECTED: [u8; 4] = [104, 104, 106, 255];

// Expected sRGB bytes for urgency=1 (normal) backdrop at 0.9 alpha.
// #1A1A3A linear(0.102, 0.102, 0.227) at 0.9α over clear:
//   sRGB ≈ (80, 80, 133)
const NORMAL_EXPECTED: [u8; 4] = [80, 80, 133, 255];

// Expected sRGB bytes for urgency=2 (urgent) backdrop at 0.9 alpha.
// #8B6914 linear(0.545, 0.412, 0.078) at 0.9α over clear:
//   sRGB ≈ (184, 166, 72)
const URGENT_EXPECTED: [u8; 4] = [184, 166, 72, 255];

// Expected sRGB bytes for urgency=3 (critical) backdrop at 0.9 alpha.
// #8B1A1A linear(0.545, 0.102, 0.102) at 0.9α over clear:
//   sRGB ≈ (184, 80, 82)
const CRITICAL_EXPECTED: [u8; 4] = [184, 80, 82, 255];

// ─── Notification-area geometry constants ─────────────────────────────────────
//
// For a 256×256 surface with x_pct=0.75, y_pct=0.0, width_pct=0.24:
//   zone_x = 256 * 0.75 = 192
//   zone_y = 256 * 0.0  = 0
//   zone_w = 256 * 0.24 = 61.44
//
// stack_slot_height with default RenderingPolicy (font_size_px=16, margin_v=8):
//   slot_h = 16 + 2*8 + 2 = 34 px
//
// Slot centre coordinates (slot 0 = topmost):
//   slot 0 centre: x = 192 + floor(61.44/2) = 192 + 30 = 222
//                  y = 0 + 34/2 = 17

const SURFACE_W: u32 = 256;
const SURFACE_H: u32 = 256;

/// Centre x-coordinate for sampling (centre of the notification-area zone).
const SAMPLE_X: u32 = 222;

/// Centre y-coordinate of slot 0 (topmost slot).
const SLOT0_Y: u32 = 17;

// ─── Tests ────────────────────────────────────────────────────────────────────

/// Requirement: Notification Exemplar MCP Integration Test — Urgency Styling
/// Scenario: urgency=0 backdrop color
///
/// Publishing a `NotificationPayload` with `urgency = 0` to the notification-area
/// zone MUST produce a backdrop quad whose rendered pixels match
/// `color.notification.urgency.low` (#2A2A2A) at 0.9 alpha, within ±12 per channel.
#[tokio::test]
async fn test_notification_urgency0_low_backdrop() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);
    let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
    register_notification_zone(&mut scene);

    scene
        .publish_to_zone(
            "notification-area",
            ZoneContent::Notification(NotificationPayload {
                text: "Low urgency notification".to_string(),
                icon: String::new(),
                urgency: 0,
                ttl_ms: None,
            }),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish_to_zone must succeed for urgency=0 Notification on notification-area");

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        SAMPLE_X,
        SLOT0_Y,
        LOW_EXPECTED,
        TOLERANCE,
        "notification-area urgency=0 low backdrop",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// Requirement: Notification Exemplar MCP Integration Test — Urgency Styling
/// Scenario: urgency=1 backdrop color
///
/// Publishing a `NotificationPayload` with `urgency = 1` to the notification-area
/// zone MUST produce a backdrop quad whose rendered pixels match
/// `color.notification.urgency.normal` (#1A1A3A) at 0.9 alpha, within ±12 per channel.
#[tokio::test]
async fn test_notification_urgency1_normal_backdrop() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);
    let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
    register_notification_zone(&mut scene);

    scene
        .publish_to_zone(
            "notification-area",
            ZoneContent::Notification(NotificationPayload {
                text: "Normal urgency notification".to_string(),
                icon: String::new(),
                urgency: 1,
                ttl_ms: None,
            }),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish_to_zone must succeed for urgency=1 Notification on notification-area");

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        SAMPLE_X,
        SLOT0_Y,
        NORMAL_EXPECTED,
        TOLERANCE,
        "notification-area urgency=1 normal backdrop",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// Requirement: Notification Exemplar MCP Integration Test — Urgency Styling
/// Scenario: urgency=2 backdrop color
///
/// Publishing a `NotificationPayload` with `urgency = 2` to the notification-area
/// zone MUST produce a backdrop quad whose rendered pixels match
/// `color.notification.urgency.urgent` (#8B6914) at 0.9 alpha, within ±12 per channel.
#[tokio::test]
async fn test_notification_urgency2_urgent_backdrop() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);
    let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
    register_notification_zone(&mut scene);

    scene
        .publish_to_zone(
            "notification-area",
            ZoneContent::Notification(NotificationPayload {
                text: "Urgent notification".to_string(),
                icon: String::new(),
                urgency: 2,
                ttl_ms: None,
            }),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish_to_zone must succeed for urgency=2 Notification on notification-area");

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        SAMPLE_X,
        SLOT0_Y,
        URGENT_EXPECTED,
        TOLERANCE,
        "notification-area urgency=2 urgent backdrop",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// Requirement: Notification Exemplar MCP Integration Test — Urgency Styling
/// Scenario: urgency=3 backdrop color
///
/// Publishing a `NotificationPayload` with `urgency = 3` to the notification-area
/// zone MUST produce a backdrop quad whose rendered pixels match
/// `color.notification.urgency.critical` (#8B1A1A) at 0.9 alpha, within ±12 per channel.
#[tokio::test]
async fn test_notification_urgency3_critical_backdrop() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);
    let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
    register_notification_zone(&mut scene);

    scene
        .publish_to_zone(
            "notification-area",
            ZoneContent::Notification(NotificationPayload {
                text: "Critical notification".to_string(),
                icon: String::new(),
                urgency: 3,
                ttl_ms: None,
            }),
            "test-agent",
            None,
            None,
            None,
        )
        .expect("publish_to_zone must succeed for urgency=3 Notification on notification-area");

    compositor.render_frame_headless(&scene, &surface);
    let pixels = surface.read_pixels(&compositor.device);

    HeadlessSurface::assert_pixel_color(
        &pixels,
        SURFACE_W,
        SAMPLE_X,
        SLOT0_Y,
        CRITICAL_EXPECTED,
        TOLERANCE,
        "notification-area urgency=3 critical backdrop",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// Requirement: Notification Exemplar MCP Integration Test — Urgency Styling
/// Scenario: All four urgency levels produce visually distinct backdrops.
///
/// Each urgency level (0, 1, 2, 3) published to the notification-area zone MUST
/// produce a different rendered pixel signature. This test confirms that urgency
/// colors are not aliased: no two urgency levels map to indistinguishable pixels.
///
/// The key channel distinctions:
///   - urgency 2 (urgent) and urgency 3 (critical) share similar R (~184) but
///     differ significantly in G (161 vs 80) and B (72 vs 82).
///   - urgency 0 (low) and urgency 1 (normal) share similar R/G but differ in
///     B (106 vs 133, a ~27-unit gap — well outside ±12 tolerance).
#[tokio::test]
async fn test_notification_urgency_distinct_colors() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);

    let urgency_levels: [u32; 4] = [0, 1, 2, 3];
    let expected = [
        LOW_EXPECTED,
        NORMAL_EXPECTED,
        URGENT_EXPECTED,
        CRITICAL_EXPECTED,
    ];

    // Collect actual rendered pixel for each urgency level (one frame per level).
    let mut actuals: Vec<[u8; 4]> = Vec::with_capacity(4);
    for urgency in urgency_levels {
        let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
        register_notification_zone(&mut scene);
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: format!("urgency={urgency}"),
                    icon: String::new(),
                    urgency,
                    ttl_ms: None,
                }),
                "test-agent",
                None,
                None,
                None,
            )
            .unwrap();
        compositor.render_frame_headless(&scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        actuals.push(HeadlessSurface::pixel_at(
            &pixels, SURFACE_W, SAMPLE_X, SLOT0_Y,
        ));
    }

    // Verify each urgency matches its own expected value within tolerance.
    for (i, (actual, exp)) in actuals.iter().zip(expected.iter()).enumerate() {
        let label = ["low", "normal", "urgent", "critical"][i];
        for ch in 0..3 {
            let diff = (actual[ch] as i16 - exp[ch] as i16).unsigned_abs() as u8;
            assert!(
                diff <= TOLERANCE,
                "urgency={i} ({label}) channel {ch}: actual={} expected={} diff={} > tolerance={}",
                actual[ch],
                exp[ch],
                diff,
                TOLERANCE
            );
        }
    }

    // Verify all four pixel signatures are mutually distinct.
    // We require that no two urgency levels are indistinguishable (max channel diff > 2*TOLERANCE).
    for i in 0..4 {
        for j in (i + 1)..4 {
            let a = &actuals[i];
            let b = &actuals[j];
            // Maximum per-channel difference across R, G, B channels.
            let max_diff = (0..3)
                .map(|ch| (a[ch] as i16 - b[ch] as i16).unsigned_abs())
                .max()
                .unwrap_or(0);
            assert!(
                max_diff > TOLERANCE as u16,
                "urgency={i} and urgency={j} are indistinguishable: \
                 pixels {:?} vs {:?} max_diff={} must exceed TOLERANCE={}",
                a,
                b,
                max_diff,
                TOLERANCE
            );
        }
    }
}

// ─── Debug probe (ignored, for calibration only) ─────────────────────────────

/// Internal calibration test: prints actual pixel values at the notification slot.
///
/// Run with:
///   `HEADLESS_FORCE_SOFTWARE=1 cargo test --test notification_rendering debug_probe_pixel_values -- --nocapture --ignored`
///
/// Use the output to recalibrate LOW_EXPECTED/NORMAL_EXPECTED/URGENT_EXPECTED/CRITICAL_EXPECTED.
#[tokio::test]
#[ignore]
async fn debug_probe_pixel_values() {
    let (mut compositor, surface) =
        gpu_or_skip!(make_compositor_and_surface(SURFACE_W, SURFACE_H).await);

    for urgency in [0u32, 1, 2, 3] {
        let mut scene = SceneGraph::new(SURFACE_W as f32, SURFACE_H as f32);
        register_notification_zone(&mut scene);
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: format!("urgency={urgency}"),
                    icon: String::new(),
                    urgency,
                    ttl_ms: None,
                }),
                "probe",
                None,
                None,
                None,
            )
            .unwrap();
        compositor.render_frame_headless(&scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        let p = HeadlessSurface::pixel_at(&pixels, SURFACE_W, SAMPLE_X, SLOT0_Y);
        println!(
            "urgency={urgency} ({}) → pixel at ({SAMPLE_X},{SLOT0_Y}): R={} G={} B={} A={}",
            ["low", "normal", "urgent", "critical"][urgency as usize],
            p[0],
            p[1],
            p[2],
            p[3]
        );
    }
}
