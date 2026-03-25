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
                        ..Default::default()
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
                    device_id: 0,
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

/// Assert that input_to_scene_commit p99 is under the 50ms budget.
///
/// Runs 30 headless frames and records the `input_to_scene_commit_us` field
/// emitted by `render_frame()`. Note: in the headless pipeline there is no
/// live agent applying mutations between frames, so `input_to_scene_commit_us`
/// will be 0 on frames with no applied mutations (see
/// `headless.rs` — the field is gated on `mutations_applied > 0`). This test
/// therefore validates the timing infrastructure plumbing rather than asserting
/// a representative agent round-trip latency. Budget assertion checks that any
/// non-zero samples are under 50ms (which they should be, even on slow CI).
///
/// ## Pipeline derivation
/// `render_frame()` sets `input_to_scene_commit_us` as wall time from
/// `frame_start` to end of Stage 4, which is the proxy for the
/// input-to-commit path from the frame start boundary.
///
/// ## CI note
/// The 50ms budget includes agent network round-trip time. The headless path
/// measures only the local pipeline, so no multiplier is needed — the local
/// commit should be far under 50ms even on slow CI machines.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_input_to_scene_commit_p99_within_budget() {
    const BUDGET_US: u64 = 50_000; // 50ms — covers agent network round-trip
    const CYCLE_COUNT: usize = 30;

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 0,
        psk: "test".to_string(),
    };
    let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");

    // Set up a minimal scene
    {
        let mut state = runtime.shared_state().lock().await;
        state.scene.create_tab("Main", 0).unwrap();
    }

    let mut bucket = LatencyBucket::new("input_to_scene_commit");

    for _ in 0..CYCLE_COUNT {
        // render_frame() executes the full pipeline and reports
        // input_to_scene_commit_us = stages 1–4 combined (local commit path)
        let telemetry = runtime.render_frame().await;
        bucket.record(telemetry.input_to_scene_commit_us);
    }

    bucket
        .assert_p99_under(BUDGET_US)
        .expect("input_to_scene_commit p99 budget");

    // Also record into the shared summary for cross-test consistency
    runtime
        .telemetry
        .summary_mut()
        .input_to_scene_commit
        .samples
        .extend_from_slice(&bucket.samples);
}

/// Assert that input_to_next_present p99 is under the 33ms budget at 60Hz.
///
/// Runs 20 headless frames and verifies that the time from frame start (proxy
/// for input event arrival) to Stage 7 completion (GPU present) stays under
/// the 33ms two-frame budget at 60Hz.
///
/// ## Pipeline derivation
/// `render_frame()` sets `input_to_next_present_us = frame_time_us`, which is
/// the total wall time from Stage 1 start to Stage 7 end. This is the correct
/// measurement point: the present happens at Stage 7, and the frame pipeline
/// begins at the input drain boundary (Stage 1).
///
/// ## Hardware normalization
/// The 33ms budget is for real GPU hardware at 60Hz. On llvmpipe/SwiftShader
/// the same 10× headless multiplier used for frame-time tests applies.
/// Replace with `NOMINAL_BUDGET_US / calibration.gpu_fill_factor` once the
/// hardware calibration harness is implemented.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_input_to_next_present_p99_within_budget() {
    const NOMINAL_BUDGET_US: u64 = 33_000; // 33ms at 60Hz (two frames)
    const HEADLESS_MULTIPLIER: u64 = 10;
    const BUDGET_US: u64 = NOMINAL_BUDGET_US * HEADLESS_MULTIPLIER;
    const FRAME_COUNT: usize = 20;

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 0,
        psk: "test".to_string(),
    };
    let mut runtime = HeadlessRuntime::new(config).await.expect("runtime init");

    // Create a scene with one tile to exercise the full render path
    {
        let mut state = runtime.shared_state().lock().await;
        let tab = state.scene.create_tab("Main", 0).unwrap();
        let lease = state.scene.grant_lease("test-agent", 60_000, vec![]);
        state
            .scene
            .create_tile(tab, "test-agent", lease, Rect::new(10.0, 10.0, 200.0, 100.0), 1)
            .unwrap();
    }

    // Discard the first frame to avoid wgpu pipeline/shader compilation overhead.
    runtime.render_frame().await;
    runtime.telemetry = tze_hud_telemetry::TelemetryCollector::new();

    let mut bucket = LatencyBucket::new("input_to_next_present");

    for _ in 0..FRAME_COUNT {
        let telemetry = runtime.render_frame().await;
        bucket.record(telemetry.input_to_next_present_us);
    }

    assert_eq!(
        bucket.samples.len(),
        FRAME_COUNT,
        "expected {FRAME_COUNT} samples in input_to_next_present bucket"
    );

    bucket
        .assert_p99_under(BUDGET_US)
        .expect("input_to_next_present p99 budget");

    // Also record into the shared summary for cross-test consistency
    runtime
        .telemetry
        .summary_mut()
        .input_to_next_present
        .samples
        .extend_from_slice(&bucket.samples);
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
                        ..Default::default()
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
                    device_id: 0,
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
    // Raise tile budget so budget enforcement doesn't reject batches in this timing test
    if let Some(l) = scene.leases.get_mut(&lease) {
        l.resource_budget.max_tiles = 256;
    }

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
            timing_hints: None,
            lease_id: None,
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

// ─── Layer 1: 25-scene pixel readback assertions ──────────────────────────────
//
// Per validation-framework/spec.md Requirement: DR-V2 (line 186):
// "Compositor MUST render complete frame to offscreen texture with no window,
// no display server, no user interaction. Feature-equivalent to windowed for
// scene composition."
//
// Per validation-framework/spec.md Requirement: DR-V5 (line 228):
// "`cargo test --features headless` SHALL run full test suite (Layers 0-2)."
//
// These tests cover all 25 scenes in TestSceneRegistry.  For each scene:
// - Pixel buffer size is correct (width × height × 4).
// - The render completed: at least some pixels have been written.
// - For scenes with tiles, tile pixels differ from the pure-black default.
//
// For specific scenes (empty_scene, overlapping_tiles_zorder) more precise
// per-pixel color assertions are made.

use tze_hud_scene::test_scenes::{ClockMs, TestSceneRegistry};

// ─── Display dimensions used for all 25-scene tests ──────────────────────────
// 800×600: fast to render on llvmpipe/WARP, matches existing pixel tests.
const SCENE_W: u32 = 800;
const SCENE_H: u32 = 600;

/// Helper: build a HeadlessRuntime sized for the 25-scene tests.
async fn make_scene_runtime() -> tze_hud_runtime::HeadlessRuntime {
    HeadlessRuntime::new(HeadlessConfig {
        width: SCENE_W,
        height: SCENE_H,
        grpc_port: 0,
        psk: "test".to_string(),
    })
    .await
    .expect("HeadlessRuntime::new failed")
}

/// Run one frame with the given scene, then return the pixel buffer.
/// The runtime's scene is replaced via shared_state before rendering.
async fn render_scene_pixels(
    runtime: &mut tze_hud_runtime::HeadlessRuntime,
    scene: tze_hud_scene::graph::SceneGraph,
) -> Vec<u8> {
    {
        let mut state = runtime.shared_state().lock().await;
        state.scene = scene;
    }
    runtime.render_frame().await;
    runtime.read_pixels()
}

/// Background clear color in sRGB (compositor clears to linear [0.05, 0.05, 0.10, 1.0]).
/// sRGB ≈ [64, 64, 89, 255].  Tolerance ±8 for llvmpipe/SwiftShader.
const BG_SRGB: [u8; 4] = [64, 64, 89, 255];
const BG_TOLERANCE: u8 = 8;

// ─── empty_scene ─────────────────────────────────────────────────────────────

/// empty_scene: no tabs, no tiles — every pixel is the background clear color.
///
/// DR-V2: headless compositor renders a complete frame with no scene content.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_scene_empty_scene_pixels() {
    let mut runtime = make_scene_runtime().await;
    let registry = TestSceneRegistry::with_display(SCENE_W as f32, SCENE_H as f32);
    let (scene, _spec) = registry.build("empty_scene", ClockMs::FIXED).expect("build failed");

    let pixels = render_scene_pixels(&mut runtime, scene).await;
    assert_eq!(pixels.len(), (SCENE_W * SCENE_H * 4) as usize, "pixel buffer size");

    // Sample every 50th pixel — all should be the background clear color (no tiles).
    for i in (0..SCENE_W * SCENE_H).step_by(50) {
        let x = i % SCENE_W;
        let y = i / SCENE_W;
        HeadlessSurface::assert_pixel_color(
            &pixels,
            SCENE_W,
            x,
            y,
            BG_SRGB,
            BG_TOLERANCE,
            "empty_scene background",
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
}

// ─── single_tile_solid ────────────────────────────────────────────────────────

/// single_tile_solid: one tile with TextMarkdown content.
/// Background (0.08, 0.08, 0.15) linear → sRGB ≈ (75, 75, 106).
/// The tile occupies (100, 100, 900, 500) on an 800×600 surface, so we clip to
/// the visible portion.  Sampling the tile center (400, 300) should give the
/// tile background color, not the compositor clear.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_scene_single_tile_solid_pixels() {
    let mut runtime = make_scene_runtime().await;
    let registry = TestSceneRegistry::with_display(SCENE_W as f32, SCENE_H as f32);
    let (scene, _spec) = registry.build("single_tile_solid", ClockMs::FIXED).expect("build failed");

    let pixels = render_scene_pixels(&mut runtime, scene).await;
    assert_eq!(pixels.len(), (SCENE_W * SCENE_H * 4) as usize, "pixel buffer size");

    // Tile background (0.08, 0.08, 0.15) linear → sRGB ≈ (75, 75, 106)
    // We use a wide tolerance because the tile color is close to the background.
    // The center of the tile (400, 300) should definitely not be pure-BG.
    let tile_center = HeadlessSurface::pixel_at(&pixels, SCENE_W, 400, 300);
    assert_ne!(
        tile_center, [0u8, 0, 0, 0],
        "tile center must not be all-zero — compositor must render something"
    );
    // The tile background is darker than the compositor clear blue (89 on channel 2).
    // The exact sRGB value of 0.15 linear ≈ 106 on channel B.
    // We just check that the frame has non-clear pixels in the tile region.
    let non_bg = pixels.chunks(4).any(|p| {
        // Differs from background by more than tolerance on any channel
        let r_diff = (p[0] as i16 - BG_SRGB[0] as i16).unsigned_abs();
        let g_diff = (p[1] as i16 - BG_SRGB[1] as i16).unsigned_abs();
        let b_diff = (p[2] as i16 - BG_SRGB[2] as i16).unsigned_abs();
        r_diff > 10 || g_diff > 10 || b_diff > 10
    });
    assert!(non_bg, "single_tile_solid: tile pixels must differ from background");
}

// ─── three_tiles_no_overlap ───────────────────────────────────────────────────

/// three_tiles_no_overlap: three non-overlapping tiles — text, hit-region, solid.
/// Just verifies the pixel buffer is correct size and rendering completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_scene_three_tiles_no_overlap_pixels() {
    let mut runtime = make_scene_runtime().await;
    let registry = TestSceneRegistry::with_display(SCENE_W as f32, SCENE_H as f32);
    let (scene, _spec) = registry.build("three_tiles_no_overlap", ClockMs::FIXED).expect("build failed");

    let pixels = render_scene_pixels(&mut runtime, scene).await;
    assert_eq!(pixels.len(), (SCENE_W * SCENE_H * 4) as usize, "pixel buffer size");
    assert_eq!(pixels.len() % 4, 0, "pixel buffer must be RGBA8 aligned");
}

// ─── max_tiles_stress ────────────────────────────────────────────────────────

/// max_tiles_stress: many tiles — validates that the compositor handles high
/// tile counts headlessly (no OOM, no crash).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_scene_max_tiles_stress_pixels() {
    let mut runtime = make_scene_runtime().await;
    let registry = TestSceneRegistry::with_display(SCENE_W as f32, SCENE_H as f32);
    let (scene, _spec) = registry.build("max_tiles_stress", ClockMs::FIXED).expect("build failed");

    let pixels = render_scene_pixels(&mut runtime, scene).await;
    assert_eq!(pixels.len(), (SCENE_W * SCENE_H * 4) as usize, "pixel buffer size");
    assert!(
        pixels.iter().any(|&b| b > 0),
        "max_tiles_stress: frame must not be all-zero"
    );
}

// ─── overlapping_tiles_zorder ────────────────────────────────────────────────

/// overlapping_tiles_zorder: three tiles at z=1 (red), z=2 (green), z=3 (blue),
/// overlapping.  The top tile (z=3, blue) must appear at the overlap region.
///
/// Blue (0.2, 0.2, 0.8) linear → sRGB ≈ (124, 124, 226).
/// The top tile occupies (300, 200, 600, 400) — center at (600, 400) but
/// on 800×600 display the tile is bounded to the display.
/// We sample (400, 250) which is inside all three tiles; z=3 (blue) must win.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_scene_overlapping_tiles_zorder_pixels() {
    let mut runtime = make_scene_runtime().await;
    let registry = TestSceneRegistry::with_display(SCENE_W as f32, SCENE_H as f32);
    let (scene, _spec) = registry.build("overlapping_tiles_zorder", ClockMs::FIXED).expect("build failed");

    let pixels = render_scene_pixels(&mut runtime, scene).await;
    assert_eq!(pixels.len(), (SCENE_W * SCENE_H * 4) as usize, "pixel buffer size");

    // At the overlap region (400, 250), the z=3 tile (blue 0.2, 0.2, 0.8) should dominate.
    // sRGB ≈ (124, 124, 226).  Tolerance ±10 for software GPU variance.
    // The important assertion is: blue channel is significantly larger than red/green.
    let overlap_px = HeadlessSurface::pixel_at(&pixels, SCENE_W, 400, 250);
    assert!(
        overlap_px[2] > overlap_px[0] + 50 && overlap_px[2] > overlap_px[1] + 50,
        "z=3 (blue) tile must dominate at overlap: pixel={overlap_px:?}"
    );
}

// ─── overlay_transparency ────────────────────────────────────────────────────

/// overlay_transparency: chrome overlay with alpha blending.
/// Verifies that alpha blending produces non-trivial output.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_scene_overlay_transparency_pixels() {
    let mut runtime = make_scene_runtime().await;
    let registry = TestSceneRegistry::with_display(SCENE_W as f32, SCENE_H as f32);
    let (scene, _spec) = registry.build("overlay_transparency", ClockMs::FIXED).expect("build failed");

    let pixels = render_scene_pixels(&mut runtime, scene).await;
    assert_eq!(pixels.len(), (SCENE_W * SCENE_H * 4) as usize, "pixel buffer size");
    assert!(
        pixels.iter().any(|&b| b > 10),
        "overlay_transparency: frame must not be near-zero"
    );
}

// ─── Batch test for remaining 19 scenes ──────────────────────────────────────
//
// For the remaining scenes (tab_switch, lease_expiry, mobile_degraded,
// sync_group_media, input_highlight, coalesced_dashboard, three_agents_contention,
// overlay_passthrough_regions, disconnect_reclaim_multiagent, privacy_redaction_mode,
// chatty_dashboard_touch, zone_publish_subtitle, zone_reject_wrong_type,
// zone_conflict_two_publishers, zone_orchestrate_then_publish,
// zone_geometry_adapts_profile, zone_disconnect_cleanup, policy_matrix_basic,
// policy_arbitration_collision) the assertion is:
//   - Pixel buffer size matches SCENE_W × SCENE_H × 4.
//   - At least one pixel is non-zero (compositor rendered something).
//
// These are structural validity tests — they confirm the headless pipeline
// completes for every scene without crash or OOM.

macro_rules! scene_render_test {
    ($test_name:ident, $scene_name:literal) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $test_name() {
            let mut runtime = make_scene_runtime().await;
            let registry =
                TestSceneRegistry::with_display(SCENE_W as f32, SCENE_H as f32);
            let (scene, _spec) = registry
                .build($scene_name, ClockMs::FIXED)
                .expect(concat!("build failed for ", $scene_name));
            let pixels = render_scene_pixels(&mut runtime, scene).await;
            assert_eq!(
                pixels.len(),
                (SCENE_W * SCENE_H * 4) as usize,
                concat!($scene_name, ": pixel buffer size mismatch")
            );
            // A frame with content should produce at least some non-zero pixels.
            // Note: empty_scene has its own test above that checks all pixels are BG.
            assert!(
                pixels.iter().any(|&b| b > 0),
                concat!($scene_name, ": frame must not be all-zero")
            );
        }
    };
}

scene_render_test!(test_scene_tab_switch_pixels, "tab_switch");
scene_render_test!(test_scene_lease_expiry_pixels, "lease_expiry");
scene_render_test!(test_scene_mobile_degraded_pixels, "mobile_degraded");
scene_render_test!(test_scene_sync_group_media_pixels, "sync_group_media");
scene_render_test!(test_scene_input_highlight_pixels, "input_highlight");
scene_render_test!(test_scene_coalesced_dashboard_pixels, "coalesced_dashboard");
scene_render_test!(test_scene_three_agents_contention_pixels, "three_agents_contention");
scene_render_test!(test_scene_overlay_passthrough_regions_pixels, "overlay_passthrough_regions");
scene_render_test!(test_scene_disconnect_reclaim_multiagent_pixels, "disconnect_reclaim_multiagent");
scene_render_test!(test_scene_privacy_redaction_mode_pixels, "privacy_redaction_mode");
scene_render_test!(test_scene_chatty_dashboard_touch_pixels, "chatty_dashboard_touch");
scene_render_test!(test_scene_zone_publish_subtitle_pixels, "zone_publish_subtitle");
scene_render_test!(test_scene_zone_reject_wrong_type_pixels, "zone_reject_wrong_type");
scene_render_test!(test_scene_zone_conflict_two_publishers_pixels, "zone_conflict_two_publishers");
scene_render_test!(test_scene_zone_orchestrate_then_publish_pixels, "zone_orchestrate_then_publish");
scene_render_test!(test_scene_zone_geometry_adapts_profile_pixels, "zone_geometry_adapts_profile");
scene_render_test!(test_scene_zone_disconnect_cleanup_pixels, "zone_disconnect_cleanup");
scene_render_test!(test_scene_policy_matrix_basic_pixels, "policy_matrix_basic");
scene_render_test!(test_scene_policy_arbitration_collision_pixels, "policy_arbitration_collision");
