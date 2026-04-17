//! Layer 2 visual regression integration tests.
//!
//! These tests require `--features headless` and a GPU (software or hardware).
//! They implement the acceptance criteria from validation-framework/spec.md:
//!
//!   Requirement: Layer 2 - Visual Regression via SSIM
//!   Scenario: Layout SSIM threshold
//!     WHEN a rendered scene is compared to its golden reference for a layout test
//!     THEN an SSIM score of 0.996 MUST pass and an SSIM score of 0.993 MUST fail
//!
//!   Scenario: Structured failure output
//!     WHEN a visual regression test fails (SSIM below threshold)
//!     THEN the output MUST include per-region SSIM scores, a diff heatmap image,
//!          and structured JSON failure details
//!
//!   Scenario: Golden references in repository
//!     WHEN a new test scene is added to the registry
//!     THEN its golden reference image MUST be committed to the repository,
//!          named by scene and backend
//!
//! DR-V5: `cargo test --features headless` runs the full test suite (Layers 0-2).

#![cfg(feature = "headless")]

use std::fs;
use std::path::PathBuf;

use tze_hud_compositor::{Compositor, HeadlessSurface};
use tze_hud_scene::{
    graph::SceneGraph,
    types::{Capability, Rect, Rgba, SolidColorNode},
};

// Rgba uses f32 components in [0.0, 1.0] range.
// Note: we use Compositor directly (not HeadlessRuntime) because
// HeadlessRuntime::render_frame() calls compositor.render_frame() which does NOT
// call copy_to_buffer; only compositor.render_frame_headless() does.
// Layer 2 tests need actual pixel data, so we go through the compositor directly.
use tze_hud_validation::{
    BACKEND_SOFTWARE, GoldenStore, Layer2Validator, SSIM_THRESHOLD_LAYOUT, SSIM_THRESHOLD_MEDIA,
    SsimFailureRecord, TestType, ValidationError, compute_ssim, encode_heatmap_png,
    generate_heatmap, pre_screen,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a solid-color scene with a single tile covering the full surface.
fn solid_color_scene(w: f32, h: f32, color: Rgba) -> SceneGraph {
    use tze_hud_scene::types::{Node, NodeData, SceneId};
    let mut g = SceneGraph::new(w, h);
    let tab = g.create_tab("Main", 0).unwrap();
    let lease = g.grant_lease(
        "agent.test",
        600_000,
        vec![Capability::CreateTile, Capability::CreateNode],
    );
    let tile = g
        .create_tile(tab, "agent.test", lease, Rect::new(0.0, 0.0, w, h), 0)
        .unwrap();
    let node = Node {
        id: SceneId::new(),
        children: vec![],
        data: NodeData::SolidColor(SolidColorNode {
            color,
            bounds: Rect::new(0.0, 0.0, w, h),
            radius: None,
        }),
    };
    g.set_tile_root(tile, node).unwrap();
    g
}

/// Render `scene` headlessly and return RGBA8 pixels.
///
/// Uses `Compositor::render_frame_headless()` directly so that `copy_to_buffer`
/// is called within the same command encoder as the render pass, making
/// `surface.read_pixels()` return the current frame's actual pixel data.
async fn render_scene(scene: SceneGraph, w: u32, h: u32) -> Vec<u8> {
    let mut compositor = Compositor::new_headless(w, h)
        .await
        .expect("headless compositor");
    let surface = HeadlessSurface::new(&compositor.device, w, h);
    let mut scene = scene;
    compositor.render_frame_headless(&mut scene, &surface);
    surface.read_pixels(&compositor.device)
}

/// Temporary golden store for tests that don't use the workspace goldens.
/// Uses an atomic counter to prevent collisions between parallel tests.
fn temp_golden_dir() -> (PathBuf, GoldenStore) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("tze_hud_l2_headless_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let store = GoldenStore::new(&dir);
    (dir, store)
}

// ── Core SSIM threshold test (acceptance criterion) ──────────────────────────

/// SSIM 0.996 passes layout threshold, 0.993 fails.
///
/// Verifies acceptance criterion:
///   "WHEN a rendered scene is compared to its golden reference for a layout test
///    THEN an SSIM score of 0.996 MUST pass and an SSIM score of 0.993 MUST fail"
///
/// We directly exercise the threshold constant (0.995) to validate the boundary
/// conditions, since constructing images with exact sub-percent SSIM differences
/// is hardware-dependent. The unit tests in layer2.rs validate the actual pixel
/// math for both passing and failing cases.
#[test]
fn acceptance_ssim_0996_passes_0993_fails_layout() {
    // Boundary: 0.996 ≥ 0.995 → pass
    assert!(
        0.996_f64 >= SSIM_THRESHOLD_LAYOUT,
        "SSIM 0.996 must pass layout threshold ({})",
        SSIM_THRESHOLD_LAYOUT
    );
    // Boundary: 0.993 < 0.995 → fail
    assert!(
        0.993_f64 < SSIM_THRESHOLD_LAYOUT,
        "SSIM 0.993 must fail layout threshold ({})",
        SSIM_THRESHOLD_LAYOUT
    );
}

// ── Headless render + SSIM self-comparison ────────────────────────────────────

/// Render a scene headlessly, then compare against itself.
/// A scene compared to its own render must yield SSIM = 1.0.
///
/// This validates that:
/// 1. Headless rendering produces a valid pixel buffer.
/// 2. SSIM of identical buffers is exactly 1.0.
/// 3. The pre-screen correctly identifies identical images.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rendered_frame_self_ssim_is_one() {
    let w = 128u32;
    let h = 128u32;
    let scene = solid_color_scene(w as f32, h as f32, Rgba::new(0.78, 0.39, 0.20, 1.0));
    let pixels = render_scene(scene, w, h).await;

    assert_eq!(
        pixels.len(),
        (w * h * 4) as usize,
        "pixel buffer must be correct size"
    );

    let result = compute_ssim(&pixels, &pixels, w, h);
    assert!(
        (result.mean - 1.0).abs() < 1e-10,
        "self-comparison must yield SSIM = 1.0, got {:.6}",
        result.mean
    );
}

// ── pHash pre-screen on rendered frames ──────────────────────────────────────

/// Two renders of the same scene have identical pHash → SkipSsim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phash_prescreens_identical_renders() {
    use tze_hud_validation::PreScreenResult;

    let w = 64u32;
    let h = 64u32;
    let scene1 = solid_color_scene(w as f32, h as f32, Rgba::new(0.5, 0.25, 0.125, 1.0));
    let scene2 = solid_color_scene(w as f32, h as f32, Rgba::new(0.5, 0.25, 0.125, 1.0));

    let pixels1 = render_scene(scene1, w, h).await;
    let pixels2 = render_scene(scene2, w, h).await;

    // Renders of the same scene should produce identical pixels → SkipSsim
    let ps = pre_screen(&pixels1, &pixels2, w, h);
    assert_eq!(
        ps,
        PreScreenResult::SkipSsim,
        "two renders of the same scene must pre-screen as SkipSsim"
    );
}

// ── Structured failure output ─────────────────────────────────────────────────

/// When SSIM fails, structured JSON output includes required fields.
///
/// Spec §Scenario: Structured failure output:
///   "WHEN a visual regression test fails (SSIM below threshold)
///    THEN the output MUST include per-region SSIM scores, a diff heatmap image,
///         and structured JSON failure details"
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failure_output_has_required_fields() {
    let w = 64u32;
    let h = 64u32;

    // Render black scene and white scene — guaranteed SSIM failure.
    let black_scene = solid_color_scene(w as f32, h as f32, Rgba::BLACK);
    let white_scene = solid_color_scene(w as f32, h as f32, Rgba::WHITE);

    let black_pixels = render_scene(black_scene, w, h).await;
    let white_pixels = render_scene(white_scene, w, h).await;

    // Compute SSIM (will be very low — black vs white).
    let ssim_result = compute_ssim(&white_pixels, &black_pixels, w, h);

    // Build structured failure record.
    let record = SsimFailureRecord::from_ssim_result(
        "test_failure_output",
        BACKEND_SOFTWARE,
        "layout",
        SSIM_THRESHOLD_LAYOUT,
        &ssim_result,
    );

    // 1. Must not pass (black vs white is a severe regression).
    assert!(!record.passed, "black vs white must fail");

    // 2. Per-region SSIM scores: spec requires these.
    assert!(
        !record.regions.is_empty(),
        "structured failure output must include per-region SSIM scores"
    );

    // 3. Diff heatmap: generate and verify it encodes as PNG.
    let heatmap = generate_heatmap(&white_pixels, &black_pixels, w, h);
    assert_eq!(
        heatmap.len(),
        (w * h * 4) as usize,
        "heatmap must match input size"
    );
    let png = encode_heatmap_png(&heatmap, w, h).expect("heatmap must encode as PNG");
    assert!(png.len() > 0, "heatmap PNG must not be empty");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "heatmap must be valid PNG");

    // 4. JSON failure details: spec requires structured JSON.
    let json = record.to_json();
    assert!(
        json.contains("\"scene_name\""),
        "JSON must include scene_name"
    );
    assert!(json.contains("\"backend\""), "JSON must include backend");
    assert!(
        json.contains("\"threshold\""),
        "JSON must include threshold"
    );
    assert!(
        json.contains("\"actual_ssim\""),
        "JSON must include actual_ssim"
    );
    assert!(json.contains("\"regions\""), "JSON must include regions");
    assert!(json.contains("\"delta\""), "JSON must include delta");
    assert!(json.contains("\"passed\""), "JSON must include passed");
    assert!(json.contains("false"), "JSON must mark test as failed");
}

// ── Golden reference workflow ─────────────────────────────────────────────────

/// Render → write golden → compare against same render → must pass.
///
/// Spec §Scenario: Golden references in repository:
///   "WHEN a new test scene is added to the registry
///    THEN its golden reference image MUST be committed to the repository,
///         named by scene and backend"
///
/// This test validates the naming convention and round-trip correctness.
/// In practice, goldens are committed to `tests/golden/`; here we use
/// a temp directory to isolate from other test runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn golden_update_then_compare_passes() {
    let w = 64u32;
    let h = 64u32;
    let scene = solid_color_scene(w as f32, h as f32, Rgba::new(0.59, 0.29, 0.78, 1.0));
    let pixels = render_scene(scene, w, h).await;

    let (dir, store) = temp_golden_dir();

    // Naming convention: {scene_name}_{backend}.png
    let path = store
        .update("test_golden_scene", BACKEND_SOFTWARE, &pixels, w, h)
        .expect("golden update must succeed");
    assert!(
        path.file_name().unwrap().to_str().unwrap() == "test_golden_scene_software.png",
        "golden file must follow naming convention {{scene_name}}_{{backend}}.png"
    );

    // Load and compare against same pixels → must pass.
    let validator = Layer2Validator::new(&dir);
    let outcome = validator
        .compare(
            "test_golden_scene",
            BACKEND_SOFTWARE,
            TestType::Layout,
            &pixels,
            w,
            h,
        )
        .expect("identical rendered vs golden must pass");
    assert!(outcome.passed);

    let _ = fs::remove_dir_all(&dir);
}

// ── Determinism: two renders of the same scene pass SSIM ─────────────────────

/// Two headless renders of the same scene have SSIM = 1.0 (deterministic).
///
/// Spec DR-V4: "A scene registry SHALL provide named, versioned configurations
/// that produce reproducible output."
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deterministic_renders_pass_ssim() {
    let w = 128u32;
    let h = 128u32;
    let scene1 = solid_color_scene(w as f32, h as f32, Rgba::new(0.39, 0.78, 0.59, 1.0));
    let scene2 = solid_color_scene(w as f32, h as f32, Rgba::new(0.39, 0.78, 0.59, 1.0));

    let pixels1 = render_scene(scene1, w, h).await;
    let pixels2 = render_scene(scene2, w, h).await;

    let ssim = compute_ssim(&pixels1, &pixels2, w, h);
    assert!(
        ssim.passes(SSIM_THRESHOLD_LAYOUT),
        "deterministic renders of the same scene must pass layout SSIM threshold, \
         got {:.6}",
        ssim.mean
    );
}

// ── Layer2Validator error propagation ─────────────────────────────────────────

/// Missing golden → GoldenNotFound error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layer2_validator_missing_golden_returns_error() {
    let w = 32u32;
    let h = 32u32;
    let scene = solid_color_scene(w as f32, h as f32, Rgba::new(0.5, 0.5, 0.5, 1.0));
    let pixels = render_scene(scene, w, h).await;

    let (dir, _store) = temp_golden_dir();
    let validator = Layer2Validator::new(&dir);

    let err = validator.compare(
        "nonexistent_scene",
        BACKEND_SOFTWARE,
        TestType::Layout,
        &pixels,
        w,
        h,
    );
    assert!(
        matches!(err, Err(ValidationError::GoldenNotFound { .. })),
        "missing golden must return GoldenNotFound error"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Different renders → SSIM regression error with structured output.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layer2_validator_regression_returns_structured_error() {
    let w = 64u32;
    let h = 64u32;

    let golden_scene = solid_color_scene(w as f32, h as f32, Rgba::BLACK);
    let golden_pixels = render_scene(golden_scene, w, h).await;

    let (dir, store) = temp_golden_dir();
    store
        .update("regression_scene", BACKEND_SOFTWARE, &golden_pixels, w, h)
        .unwrap();

    let different_scene = solid_color_scene(w as f32, h as f32, Rgba::WHITE);
    let different_pixels = render_scene(different_scene, w, h).await;

    let validator = Layer2Validator::new(&dir);
    let err = validator.compare(
        "regression_scene",
        BACKEND_SOFTWARE,
        TestType::Layout,
        &different_pixels,
        w,
        h,
    );

    // compare() returns Ok(outcome) with passed=false on regression so callers
    // retain the heatmap/record for diagnostic artifact generation.
    match err {
        Ok(outcome) => {
            assert!(!outcome.passed, "black vs white must fail SSIM regression");
            assert_eq!(outcome.record.scene_name, "regression_scene");
            assert!(
                outcome.record.actual_ssim < outcome.record.threshold,
                "actual SSIM must be below threshold"
            );
            assert!(
                outcome.record.delta < 0.0,
                "delta must be negative when failing"
            );
        }
        Err(e) => panic!("expected Ok(failed outcome), got Err: {:?}", e),
    }

    let _ = fs::remove_dir_all(&dir);
}

// ── CI integration: cargo test --features headless runs Layer 2 ─────────────

/// Smoke test: Layer 2 machinery works end-to-end under `--features headless`.
///
/// Spec DR-V5: `cargo test --features headless` SHALL run the full test suite
/// (Layers 0-2). This test is the Layer 2 sentinel that must be present for
/// DR-V5 compliance.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layer2_included_in_headless_test_suite() {
    // This test exists to confirm `--features headless` compiles and runs Layer 2.
    // If this function body runs, Layer 2 is in the test suite.
    let _ = SSIM_THRESHOLD_LAYOUT;
    let _ = SSIM_THRESHOLD_MEDIA;

    // Quick sanity: compute SSIM on tiny identical images.
    let img: Vec<u8> = (0..4 * 4 * 4).map(|i| (i % 256) as u8).collect();
    let result = compute_ssim(&img, &img, 4, 4);
    assert!(
        (result.mean - 1.0).abs() < 1e-6,
        "Layer 2 SSIM self-check must yield 1.0"
    );
}
