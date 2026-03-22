//! # Budget and pixel assertions for the vertical slice
//!
//! Validates the latency budgets defined in `heart-and-soul/validation.md`
//! (Layer 3) and pixel readback correctness (Layer 1).
//!
//! ## Budgets (from validation.md Layer 3)
//! - Frame time p99 < 16.6ms  (16_600 µs)
//! - input_to_local_ack p99 < 4ms  (4_000 µs)
//! - Hit-test p99 < 100µs
//! - Transaction validation p99 < 200µs
//! - Scene diff p99 < 500µs
//!
//! ## Pixel assertions (Layer 1)
//! Render a known scene and verify background, tile, and z-order pixels are
//! within ±tolerance per channel (±2 is the spec; wider for llvmpipe CI).

use tze_hud_compositor::HeadlessSurface;
use tze_hud_input::{PointerEvent, PointerEventKind};
use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;
use tze_hud_scene::diff::SceneDiff;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::mutation::{MutationBatch, SceneMutation};
use tze_hud_scene::types::{
    Capability, FontFamily, HitRegionNode, Node, NodeData, Rect, Rgba, SceneId, SolidColorNode,
    TextAlign, TextMarkdownNode, TextOverflow,
};
use tze_hud_telemetry::LatencyBucket;

// ─── Layer 3: p99 budget assertions ──────────────────────────────────────────

/// Assert that frame time p99 is under the 16.6ms budget (normalized).
///
/// Runs 20 frames headlessly and verifies the p99 telemetry bucket stays
/// within the budget.
///
/// ## Hardware normalization
/// `validation.md` requires budgets to be tested against hardware-normalized
/// values.  The calibration infrastructure (Section "Hardware-normalized
/// performance") is not yet built, so this test applies a conservative
/// multiplier for headless / software-GPU environments (llvmpipe, SwiftShader).
///
/// The raw 16.6ms budget applies to real GPU hardware.  On a software-rasterised
/// CI runner the effective ceiling is `16.6ms × 10 = 166ms`.  Once the
/// calibration vector is implemented (GPU fill factor measured at startup), this
/// constant must be replaced by `BUDGET_US / gpu_fill_factor`.
///
/// See: heart-and-soul/validation.md §"Hardware-normalized performance"
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_frame_time_p99_within_budget() {
    // Nominal budget: 16.6ms.  10× headless multiplier for llvmpipe/SwiftShader.
    // Replace with `NOMINAL_BUDGET_US / calibration.gpu_fill_factor` once
    // hardware calibration is implemented.
    const NOMINAL_BUDGET_US: u64 = 16_600;
    const HEADLESS_MULTIPLIER: u64 = 10;
    const BUDGET_US: u64 = NOMINAL_BUDGET_US * HEADLESS_MULTIPLIER;
    const FRAME_COUNT: usize = 20;

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 0, // no gRPC needed for this test
        psk: "test".to_string(),
    };
    let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");

    // Create a simple scene with one tile
    {
        let mut state = runtime.shared_state().lock().await;
        let tab = state.scene.create_tab("Main", 0).unwrap();
        let lease = state.scene.grant_lease("test-agent", 60_000, vec![]);
        state
            .scene
            .create_tile(tab, "test-agent", lease, Rect::new(10.0, 10.0, 200.0, 100.0), 1)
            .unwrap();
    }

    // Discard the first frame: wgpu incurs pipeline/shader compilation overhead
    // on the first render that would otherwise inflate the p99 unrealistically.
    runtime.render_frame().await;
    runtime.telemetry = tze_hud_telemetry::TelemetryCollector::new();

    for _ in 0..FRAME_COUNT {
        runtime.render_frame().await;
    }

    let summary = runtime.telemetry.summary();
    assert_eq!(
        summary.total_frames, FRAME_COUNT as u64,
        "expected {FRAME_COUNT} frames recorded"
    );

    summary
        .frame_time
        .assert_p99_under(BUDGET_US)
        .expect("frame_time p99 budget");
}

/// Assert that input_to_local_ack p99 is under the 4ms budget.
///
/// Simulates 30 pointer-press events and verifies each local-ack latency
/// (entirely local, no network roundtrip) satisfies the p99 budget.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_input_to_local_ack_p99_within_budget() {
    const BUDGET_US: u64 = 4_000; // 4 ms
    const EVENT_COUNT: usize = 30;

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 0,
        psk: "test".to_string(),
    };
    let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");

    // Set up a scene with a hit region
    {
        let mut state = runtime.shared_state().lock().await;
        let tab = state.scene.create_tab("Main", 0).unwrap();
        let lease = state.scene.grant_lease("test-agent", 60_000, vec![]);
        let tile = state
            .scene
            .create_tile(tab, "test-agent", lease, Rect::new(100.0, 100.0, 200.0, 200.0), 1)
            .unwrap();
        state
            .scene
            .set_tile_root(
                tile,
                Node {
                    id: SceneId::new(),
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
                        interaction_id: "test-button".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                    }),
                    children: vec![],
                },
            )
            .unwrap();
    }

    for _ in 0..EVENT_COUNT {
        let (local_ack_us, hit_test_us) = {
            let state_arc = runtime.shared_state().clone();
            let mut state = state_arc.lock().await;
            let result = runtime.input_processor.process(
                &PointerEvent {
                    x: 150.0,
                    y: 150.0,
                    kind: PointerEventKind::Down,
                    timestamp: None,
                },
                &mut state.scene,
            );
            (result.local_ack_us, result.hit_test_us)
        };
        runtime.telemetry.summary_mut().input_to_local_ack.record(local_ack_us);
        runtime.telemetry.summary_mut().hit_test_latency.record(hit_test_us);
    }

    let summary = runtime.telemetry.summary();

    summary
        .input_to_local_ack
        .assert_p99_under(BUDGET_US)
        .expect("input_to_local_ack p99 budget");
}

/// Assert that hit-test p99 is under the 100µs budget.
///
/// Exercises the hit-test path in isolation via repeated pointer-move events
/// over a large hit region.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hit_test_p99_within_budget() {
    const BUDGET_US: u64 = 100; // 100 µs
    const EVENT_COUNT: usize = 50;

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 0,
        psk: "test".to_string(),
    };
    let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");

    {
        let mut state = runtime.shared_state().lock().await;
        let tab = state.scene.create_tab("Main", 0).unwrap();
        let lease = state.scene.grant_lease("test-agent", 60_000, vec![]);
        let tile = state
            .scene
            .create_tile(tab, "test-agent", lease, Rect::new(50.0, 50.0, 400.0, 400.0), 1)
            .unwrap();
        state
            .scene
            .set_tile_root(
                tile,
                Node {
                    id: SceneId::new(),
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 400.0, 400.0),
                        interaction_id: "large-button".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                    }),
                    children: vec![],
                },
            )
            .unwrap();
    }

    for _ in 0..EVENT_COUNT {
        let hit_test_us = {
            let state_arc = runtime.shared_state().clone();
            let mut state = state_arc.lock().await;
            let result = runtime.input_processor.process(
                &PointerEvent {
                    x: 200.0,
                    y: 200.0,
                    kind: PointerEventKind::Move,
                    timestamp: None,
                },
                &mut state.scene,
            );
            result.hit_test_us
        };
        runtime.telemetry.summary_mut().hit_test_latency.record(hit_test_us);
    }

    let summary = runtime.telemetry.summary();
    summary
        .hit_test_latency
        .assert_p99_under(BUDGET_US)
        .expect("hit_test p99 budget");
}

/// Assert that transaction validation p99 is under the 200µs budget.
///
/// Applies 50 single-mutation batches and records the round-trip latency of
/// each `apply_batch` call including validation and scene mutation.
///
/// ## CI note
/// When GPU tests run in parallel (wgpu initialization is CPU-intensive),
/// the scheduler may spike latency on this CPU-only path.  The budget is
/// therefore set to `200µs × 5 = 1000µs` for CI headroom.  Replace with the
/// nominal `NOMINAL_BUDGET_US` once dedicated benchmark infrastructure is in
/// place (see `heart-and-soul/validation.md` §"Hardware-normalized performance").
#[test]
fn test_transaction_validation_p99_within_budget() {
    const NOMINAL_BUDGET_US: u64 = 200; // target on real hardware
    const CI_MULTIPLIER: u64 = 5;
    const BUDGET_US: u64 = NOMINAL_BUDGET_US * CI_MULTIPLIER; // 1ms for CI
    const BATCH_COUNT: usize = 50;

    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab = scene.create_tab("Main", 0).unwrap();
    let lease = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);

    let mut validation_bucket = LatencyBucket::new("validation");

    for i in 0..BATCH_COUNT {
        let start = std::time::Instant::now();
        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::CreateTile {
                tab_id: tab,
                namespace: "agent".to_string(),
                lease_id: lease,
                bounds: Rect::new(
                    (i as f32 * 5.0) % 1500.0,
                    (i as f32 * 3.0) % 900.0,
                    80.0,
                    60.0,
                ),
                z_order: (i as u32 % 10) + 1,
            }],
        };
        let result = scene.apply_batch(&batch);
        let elapsed_us = start.elapsed().as_micros() as u64;
        validation_bucket.record(elapsed_us);
        assert!(result.applied, "batch {i} should have applied");
    }

    validation_bucket
        .assert_p99_under(BUDGET_US)
        .expect("transaction validation p99 budget");
}

/// Assert that scene diff p99 is under the 500µs budget.
///
/// Computes diffs between before/after snapshots of a scene with 10 tiles and
/// verifies the p99 latency across 50 iterations.
#[test]
fn test_scene_diff_p99_within_budget() {
    const BUDGET_US: u64 = 500; // 500 µs
    const DIFF_COUNT: usize = 50;

    let mut diff_bucket = LatencyBucket::new("diff");

    for i in 0..DIFF_COUNT {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab = scene.create_tab("Main", 0).unwrap();
        let lease = scene.grant_lease("agent", 60_000, vec![]);

        // Build a scene with a handful of tiles
        for j in 0..10 {
            scene
                .create_tile(
                    tab,
                    "agent",
                    lease,
                    Rect::new(
                        (j as f32 * 100.0) % 1600.0,
                        (i as f32 * 20.0) % 900.0,
                        90.0,
                        70.0,
                    ),
                    j as u32 + 1,
                )
                .unwrap();
        }

        let snapshot = scene.clone();

        // Add one more tile
        scene
            .create_tile(tab, "agent", lease, Rect::new(5.0, 5.0, 50.0, 50.0), 99)
            .unwrap();

        let start = std::time::Instant::now();
        let diff = SceneDiff::compute(&snapshot, &scene);
        let elapsed_us = start.elapsed().as_micros() as u64;
        diff_bucket.record(elapsed_us);

        assert!(!diff.is_empty(), "diff should detect the new tile");
    }

    diff_bucket
        .assert_p99_under(BUDGET_US)
        .expect("scene diff p99 budget");
}

// ─── Layer 1: Pixel readback assertions ──────────────────────────────────────

/// Verify background pixels match the compositor clear color after rendering
/// an empty scene.
///
/// Color math
/// ──────────
/// The compositor uses `Rgba8UnormSrgb` as the render target.  The GPU
/// converts linear vertex colors to sRGB on write, so pixels in the readback
/// buffer are sRGB-encoded.
///
/// Background clear: linear (0.05, 0.05, 0.10, 1.0)
///   → sRGB ≈ (64, 64, 89, 255)
///
/// Tolerance ±6 per channel to accommodate llvmpipe/SwiftShader variance.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_layer1_pixel_readback_background() {
    // Background clear (0.05, 0.05, 0.10, 1.0) linear → sRGB ≈ (64, 64, 89, 255)
    const EXPECTED_BG: [u8; 4] = [64, 64, 89, 255];
    const TOLERANCE: u8 = 6;

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 0,
        psk: "test".to_string(),
    };
    let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");

    // Empty scene — every pixel should be the background clear color
    runtime.render_frame().await;
    let pixels = runtime.read_pixels();
    assert_eq!(pixels.len(), 800 * 600 * 4, "pixel buffer size mismatch");

    // Sample corners and center
    for (x, y) in [(5u32, 5u32), (795, 5), (5, 595), (795, 595), (400, 300)] {
        HeadlessSurface::assert_pixel_color(
            &pixels,
            800,
            x,
            y,
            EXPECTED_BG,
            TOLERANCE,
            &format!("background ({x},{y})"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
}

/// Verify that a tile region renders with its expected background color.
///
/// A TextMarkdown tile is created with `background = (0.10, 0.15, 0.30)`
/// linear.  After rendering, the pixel at the tile center should approximate
/// the sRGB encoding of that color: ≈ (89, 105, 148).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_layer1_pixel_readback_tile_color() {
    // Text tile background (0.10, 0.15, 0.30) linear → sRGB ≈ (89, 105, 148)
    const EXPECTED_TILE: [u8; 4] = [89, 105, 148, 255];
    const TOLERANCE: u8 = 8;

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 0,
        psk: "test".to_string(),
    };
    let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");

    let (tile_x, tile_y, tile_w, tile_h) = (50u32, 50u32, 350u32, 250u32);
    {
        let mut state = runtime.shared_state().lock().await;
        state.scene.create_tab("Main", 0).unwrap();
        let tab = state.scene.active_tab.unwrap();
        let lease = state.scene.grant_lease("agent", 60_000, vec![]);
        let tile = state
            .scene
            .create_tile(
                tab,
                "agent",
                lease,
                Rect::new(tile_x as f32, tile_y as f32, tile_w as f32, tile_h as f32),
                1,
            )
            .unwrap();
        state
            .scene
            .set_tile_root(
                tile,
                Node {
                    id: SceneId::new(),
                    data: NodeData::TextMarkdown(TextMarkdownNode {
                        content: "Layer 1 test".to_string(),
                        bounds: Rect::new(0.0, 0.0, tile_w as f32, tile_h as f32),
                        font_size_px: 16.0,
                        font_family: FontFamily::SystemSansSerif,
                        color: Rgba::WHITE,
                        background: Some(Rgba::new(0.10, 0.15, 0.30, 1.0)),
                        alignment: TextAlign::Start,
                        overflow: TextOverflow::Clip,
                    }),
                    children: vec![],
                },
            )
            .unwrap();
    }

    runtime.render_frame().await;
    let pixels = runtime.read_pixels();

    // Sample the tile interior, well away from edges
    let sample_x = tile_x + tile_w / 2;
    let sample_y = tile_y + tile_h / 2;

    HeadlessSurface::assert_pixel_color(
        &pixels,
        800,
        sample_x,
        sample_y,
        EXPECTED_TILE,
        TOLERANCE,
        "text tile center",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// Verify that a higher-z tile paints over a lower-z tile at the overlap.
///
/// Tile A (z=1) is a blue solid rect.  Tile B (z=2, overlapping) is red.
/// The pixel at the overlap must be red (sRGB ≈ (255, 0, 0)).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_layer1_pixel_readback_z_order() {
    // Red solid (1.0, 0.0, 0.0) linear → sRGB = (255, 0, 0)
    const EXPECTED_HIGH_Z: [u8; 4] = [255, 0, 0, 255];
    const TOLERANCE: u8 = 4;

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 0,
        psk: "test".to_string(),
    };
    let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");

    {
        let mut state = runtime.shared_state().lock().await;
        state.scene.create_tab("Main", 0).unwrap();
        let tab = state.scene.active_tab.unwrap();
        let lease = state.scene.grant_lease("agent", 60_000, vec![]);

        // Tile A at z=1 (blue)
        let tile_a = state
            .scene
            .create_tile(tab, "agent", lease, Rect::new(100.0, 100.0, 300.0, 200.0), 1)
            .unwrap();
        state
            .scene
            .set_tile_root(
                tile_a,
                Node {
                    id: SceneId::new(),
                    data: NodeData::SolidColor(SolidColorNode {
                        bounds: Rect::new(0.0, 0.0, 300.0, 200.0),
                        color: Rgba::new(0.20, 0.30, 0.50, 1.0),
                    }),
                    children: vec![],
                },
            )
            .unwrap();

        // Tile B at z=2 (red) — overlaps the center of Tile A
        let tile_b = state
            .scene
            .create_tile(tab, "agent", lease, Rect::new(150.0, 150.0, 100.0, 100.0), 2)
            .unwrap();
        state
            .scene
            .set_tile_root(
                tile_b,
                Node {
                    id: SceneId::new(),
                    data: NodeData::SolidColor(SolidColorNode {
                        bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                        color: Rgba::new(1.0, 0.0, 0.0, 1.0), // red
                    }),
                    children: vec![],
                },
            )
            .unwrap();
    }

    runtime.render_frame().await;
    let pixels = runtime.read_pixels();

    // Point inside Tile B (z=2) — must be red
    HeadlessSurface::assert_pixel_color(
        &pixels,
        800,
        200,
        200,
        EXPECTED_HIGH_Z,
        TOLERANCE,
        "high-z tile (red) at overlap",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

// Unit tests for LatencyBucket::assert_p99_under live in
// crates/tze_hud_telemetry/src/record.rs.
//
// Unit tests for HeadlessSurface::assert_pixel_color live in
// crates/tze_hud_compositor/src/surface.rs.
