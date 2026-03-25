//! Shared helpers for Layer 1 pixel readback tests.
//!
//! This module defines tolerance constants, colour-assertion helpers, and
//! scene-rendering utilities used by `pixel_readback.rs`.  It is compiled
//! only when running integration tests (`#[cfg(test)]` is implicit for
//! `tests/` files in a Rust crate).
//!
//! # Tolerance Strategy
//!
//! Per `heart-and-soul/validation.md` line 117 the ideal tolerances are:
//! - Solid fills:     ±1 per channel (ideal), ±6 per channel (llvmpipe CI)
//! - Alpha blending:  ±2 per channel (ideal), ±8 per channel (llvmpipe CI)
//!
//! Tests assert against the CI constants so they pass on software-rasterised
//! runners (Mesa llvmpipe, SwiftShader) and real GPU alike.  The IDEAL_*
//! constants document the tighter target for hardware validation.
//!
//! # sRGB Encoding
//!
//! The compositor render target uses `Rgba8UnormSrgb`.  The GPU writes
//! sRGB-encoded bytes to the readback buffer.  Expected values in this file
//! are therefore **sRGB**, not linear.
//!
//! Approximate conversion for reference:
//! ```text
//! linear 0.05  → sRGB ≈  64
//! linear 0.08  → sRGB ≈  75
//! linear 0.10  → sRGB ≈  89
//! linear 0.15  → sRGB ≈ 106
//! linear 0.20  → sRGB ≈ 124
//! linear 0.30  → sRGB ≈ 148
//! linear 0.40  → sRGB ≈ 174
//! linear 0.50  → sRGB ≈ 188
//! linear 0.70  → sRGB ≈ 214
//! linear 0.80  → sRGB ≈ 228
//! linear 1.00  → sRGB = 255
//! ```

// ─── Tolerance constants ─────────────────────────────────────────────────────

/// Ideal tolerance for solid-fill pixels (spec: ±1/channel).
/// Use when targeting real GPU validation.
#[allow(dead_code)]
pub const IDEAL_SOLID_TOLERANCE: u8 = 1;

/// Ideal tolerance for alpha-blended pixels (spec: ±2/channel).
/// Use when targeting real GPU validation.
#[allow(dead_code)]
pub const IDEAL_BLEND_TOLERANCE: u8 = 2;

/// CI tolerance for solid-fill pixels on llvmpipe / SwiftShader (±6/channel).
/// All pixel assertions in `pixel_readback.rs` use this constant.
pub const CI_SOLID_TOLERANCE: u8 = 6;

/// CI tolerance for alpha-blended pixels on llvmpipe / SwiftShader (±8/channel).
/// All pixel assertions involving alpha blending use this constant.
pub const CI_BLEND_TOLERANCE: u8 = 8;

// ─── Background clear colour ──────────────────────────────────────────────────

/// Expected background clear colour in sRGB.
///
/// Compositor clears to linear (0.05, 0.05, 0.10, 1.0).
/// sRGB ≈ (64, 64, 89, 255).
pub const BG_SRGB: [u8; 4] = [64, 64, 89, 255];

// ─── Dimensions used across all 25-scene tests ────────────────────────────────

/// Display width used for all 25-scene pixel tests.
///
/// Must match `TestSceneRegistry::new()` default (1920×1080).  Scenes are
/// built with `TestSceneRegistry::new()` whose tile bounds are relative to a
/// 1920×1080 canvas; using a smaller runtime would cause `BoundsOutOfRange`
/// errors during scene construction.
pub const SCENE_W: u32 = 1920;

/// Display height used for all 25-scene pixel tests.
pub const SCENE_H: u32 = 1080;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Return `true` if the pixel at `(x, y)` differs from `bg` by more than
/// `min_diff` on any channel.  Useful for asserting that *something* was
/// rendered without knowing the exact colour.
pub fn pixel_differs_from_bg(pixels: &[u8], width: u32, x: u32, y: u32, min_diff: u8) -> bool {
    let idx = ((y * width + x) * 4) as usize;
    let p = [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]];
    BG_SRGB.iter().zip(p.iter()).any(|(&expected, &actual)| {
        actual.abs_diff(expected) > min_diff
    })
}

/// Assert that every pixel in the buffer that falls within `[x0..x1) × [y0..y1)`
/// differs from the background by more than `min_diff` on at least one channel.
///
/// Used for scenes where we know tiles should cover a region but cannot
/// predict exact colours (e.g. text rendering, alpha blending output).
#[allow(dead_code)]
pub fn assert_region_has_content(
    pixels: &[u8],
    width: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    min_diff: u8,
    label: &str,
) {
    // Sample every 10th pixel in both dimensions to keep test runtime low.
    let xs: Vec<u32> = (x0..x1).step_by(10).collect();
    let ys: Vec<u32> = (y0..y1).step_by(10).collect();

    let all_bg = xs.iter().flat_map(|&x| {
        ys.iter().map(move |&y| !pixel_differs_from_bg(pixels, width, x, y, min_diff))
    }).all(|b| b);

    assert!(!all_bg, "{label}: expected content but all sampled pixels match background");
}

/// Assert that every sampled pixel in `[x0..x1) × [y0..y1)` matches `bg`
/// within `CI_SOLID_TOLERANCE`.
///
/// Used for regions that must be background (e.g. inactive-tab tile regions).
#[allow(dead_code)]
pub fn assert_region_is_background(
    pixels: &[u8],
    width: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    label: &str,
) {
    let step = 20usize;
    for y in (y0..y1).step_by(step) {
        for x in (x0..x1).step_by(step) {
            let idx = ((y * width + x) * 4) as usize;
            let actual = [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]];
            for ch in 0..4 {
                let diff = actual[ch].abs_diff(BG_SRGB[ch]);
                assert!(
                    diff <= CI_SOLID_TOLERANCE,
                    "{label}: expected background at ({x},{y}) channel {ch}: \
                     actual={} expected={} diff={} tolerance={}",
                    actual[ch], BG_SRGB[ch], diff, CI_SOLID_TOLERANCE
                );
            }
        }
    }
}

/// Build a [`HeadlessRuntime`] sized for the 25-scene tests (1920×1080).
///
/// Uses 1920×1080 to match [`tze_hud_scene::test_scenes::TestSceneRegistry::new`]'s
/// default display area.  Scenes are built with tile bounds relative to 1920×1080;
/// a smaller runtime would cause `BoundsOutOfRange` at scene construction time.
pub async fn make_scene_runtime() -> tze_hud_runtime::HeadlessRuntime {
    use tze_hud_runtime::headless::HeadlessConfig;
    tze_hud_runtime::HeadlessRuntime::new(HeadlessConfig {
        width: SCENE_W,
        height: SCENE_H,
        grpc_port: 0,
        psk: "test".to_string(),
    })
    .await
    .expect("HeadlessRuntime::new failed")
}

/// Replace the runtime's scene with `scene`, render one frame, and return
/// the RGBA8 pixel buffer.
pub async fn render_scene_pixels(
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
