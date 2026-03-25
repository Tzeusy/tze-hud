//! Diff heatmap generation for Layer 2 visual regression failure output.
//!
//! When SSIM falls below the threshold, the structured failure output includes:
//! 1. Per-region SSIM scores (from `ssim::SsimResult`)
//! 2. A diff heatmap image (RGBA8 bytes) — red channel encodes per-pixel difference
//! 3. A JSON failure record
//!
//! # Diff heatmap encoding
//!
//! For each pixel at position (x, y):
//!   delta = |luma(src) - luma(ref)|  (0–255)
//!   heatmap pixel = (delta, 0, 255 - delta, 255)
//!
//! Blue → no difference, red → maximum difference.
//! This is easy to parse and visually obvious.

use crate::ssim::SsimResult;
use serde::{Deserialize, Serialize};

/// Generate a diff heatmap between two RGBA8 images.
///
/// Output is an RGBA8 buffer of the same dimensions where:
/// - Blue = no difference
/// - Red = maximum difference
///
/// Both inputs must be `width * height * 4` bytes.
pub fn generate_heatmap(src: &[u8], ref_img: &[u8], width: u32, height: u32) -> Vec<u8> {
    let n = (width * height) as usize;
    assert_eq!(src.len(), n * 4);
    assert_eq!(ref_img.len(), n * 4);

    let mut out = vec![0u8; n * 4];
    for i in 0..n {
        let sr = src[i * 4] as f64;
        let sg = src[i * 4 + 1] as f64;
        let sb = src[i * 4 + 2] as f64;
        let rr = ref_img[i * 4] as f64;
        let rg = ref_img[i * 4 + 1] as f64;
        let rb = ref_img[i * 4 + 2] as f64;

        let luma_s = 0.2126 * sr + 0.7152 * sg + 0.0722 * sb;
        let luma_r = 0.2126 * rr + 0.7152 * rg + 0.0722 * rb;

        let delta = (luma_s - luma_r).abs().round() as u8;
        // Red channel = difference, blue channel = inverse difference.
        out[i * 4] = delta;                 // R: difference magnitude
        out[i * 4 + 1] = 0;                 // G: unused
        out[i * 4 + 2] = 255u8.saturating_sub(delta); // B: inverse
        out[i * 4 + 3] = 255;               // A: opaque
    }
    out
}

/// Encode a diff heatmap as a PNG byte vector.
pub fn encode_heatmap_png(
    heatmap: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    use image::{ImageBuffer, Rgba};
    let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(width, height, heatmap.to_vec())
        .ok_or_else(|| "heatmap buffer size mismatch".to_string())?;
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}

// ─── Structured failure output ────────────────────────────────────────────────

/// Per-region SSIM score for JSON output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionFailureDetail {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub ssim_score: f64,
    pub passed: bool,
}

/// Full structured failure record emitted when SSIM is below threshold.
///
/// Format: JSON. Consumed by CI, LLM agents, and Layer 4 artifact generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsimFailureRecord {
    /// Scene name from the test scene registry.
    pub scene_name: String,
    /// Backend identifier (e.g., "software", "hardware").
    pub backend: String,
    /// Test type: "layout" or "media_composition".
    pub test_type: String,
    /// The SSIM threshold that was required.
    pub threshold: f64,
    /// The actual global SSIM score.
    pub actual_ssim: f64,
    /// Difference from threshold (negative = failed by this much).
    pub delta: f64,
    /// Whether this test passed.
    pub passed: bool,
    /// Per-region SSIM breakdown.
    pub regions: Vec<RegionFailureDetail>,
    /// Path to the diff heatmap PNG (relative to test output directory).
    /// Set to None when no heatmap was written.
    pub heatmap_path: Option<String>,
    /// Number of windows evaluated in the SSIM computation.
    pub window_count: usize,
    /// Minimum per-window SSIM across all windows.
    pub min_window_ssim: f64,
    /// Maximum per-window SSIM across all windows.
    pub max_window_ssim: f64,
}

impl SsimFailureRecord {
    /// Build a failure record from a completed SSIM result.
    pub fn from_ssim_result(
        scene_name: &str,
        backend: &str,
        test_type: &str,
        threshold: f64,
        result: &SsimResult,
    ) -> Self {
        let passed = result.passes(threshold);
        let delta = result.mean - threshold;

        let (min_w, max_w) = if result.windows.is_empty() {
            (1.0, 1.0)
        } else {
            let min = result.windows.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = result.windows.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            (min, max)
        };

        let regions = result.regions.iter().map(|r| RegionFailureDetail {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
            ssim_score: r.score,
            passed: r.score >= threshold,
        }).collect();

        Self {
            scene_name: scene_name.to_string(),
            backend: backend.to_string(),
            test_type: test_type.to_string(),
            threshold,
            actual_ssim: result.mean,
            delta,
            passed,
            regions,
            heatmap_path: None,
            window_count: result.windows.len(),
            min_window_ssim: min_w,
            max_window_ssim: max_w,
        }
    }

    /// Serialize to a pretty-printed JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("SsimFailureRecord must be serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssim::compute_ssim;

    fn solid_rgba(w: u32, h: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
        let n = (w * h * 4) as usize;
        let mut buf = vec![0u8; n];
        for i in 0..(w * h) as usize {
            buf[i * 4] = r;
            buf[i * 4 + 1] = g;
            buf[i * 4 + 2] = b;
            buf[i * 4 + 3] = 255;
        }
        buf
    }

    /// Heatmap of identical images is all-blue (zero delta).
    #[test]
    fn heatmap_identical_is_blue() {
        let img = solid_rgba(16, 16, 200, 100, 50);
        let heatmap = generate_heatmap(&img, &img, 16, 16);
        for i in 0..(16 * 16) as usize {
            assert_eq!(heatmap[i * 4], 0, "R channel must be 0 for identical pixels");
            assert_eq!(heatmap[i * 4 + 2], 255, "B channel must be 255 for identical pixels");
        }
    }

    /// Heatmap of black vs white has max red, zero blue.
    #[test]
    fn heatmap_black_vs_white_is_red() {
        let black = solid_rgba(8, 8, 0, 0, 0);
        let white = solid_rgba(8, 8, 255, 255, 255);
        let heatmap = generate_heatmap(&black, &white, 8, 8);
        for i in 0..(8 * 8) as usize {
            assert_eq!(heatmap[i * 4], 255, "R channel must be 255 for max difference");
            assert_eq!(heatmap[i * 4 + 2], 0, "B channel must be 0 for max difference");
        }
    }

    /// Heatmap size matches input dimensions.
    #[test]
    fn heatmap_correct_size() {
        let a = solid_rgba(32, 16, 100, 100, 100);
        let b = solid_rgba(32, 16, 200, 200, 200);
        let heatmap = generate_heatmap(&a, &b, 32, 16);
        assert_eq!(heatmap.len(), 32 * 16 * 4, "heatmap must match input dimensions");
    }

    /// encode_heatmap_png produces valid PNG bytes.
    #[test]
    fn encode_heatmap_png_roundtrip() {
        let black = solid_rgba(16, 16, 0, 0, 0);
        let white = solid_rgba(16, 16, 255, 255, 255);
        let heatmap = generate_heatmap(&black, &white, 16, 16);
        let png = encode_heatmap_png(&heatmap, 16, 16).expect("PNG encoding must succeed");
        assert!(png.len() > 0, "PNG must not be empty");
        // Verify it starts with PNG magic bytes.
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "must be a valid PNG");
    }

    /// SsimFailureRecord JSON must contain required fields.
    #[test]
    fn failure_record_json_structure() {
        let black = solid_rgba(64, 64, 0, 0, 0);
        let white = solid_rgba(64, 64, 255, 255, 255);
        let ssim_result = compute_ssim(&black, &white, 64, 64);
        let record = SsimFailureRecord::from_ssim_result(
            "single_tile_solid",
            "software",
            "layout",
            0.995,
            &ssim_result,
        );

        let json = record.to_json();
        assert!(json.contains("\"scene_name\""), "JSON must include scene_name");
        assert!(json.contains("\"backend\""), "JSON must include backend");
        assert!(json.contains("\"threshold\""), "JSON must include threshold");
        assert!(json.contains("\"actual_ssim\""), "JSON must include actual_ssim");
        assert!(json.contains("\"regions\""), "JSON must include regions");
        assert!(json.contains("\"delta\""), "JSON must include delta");
        assert!(json.contains("\"passed\""), "JSON must include passed");
    }

    /// Failed record correctly marks passed = false.
    #[test]
    fn failure_record_failed_test_marked() {
        let black = solid_rgba(64, 64, 0, 0, 0);
        let white = solid_rgba(64, 64, 255, 255, 255);
        let ssim_result = compute_ssim(&black, &white, 64, 64);
        let record = SsimFailureRecord::from_ssim_result(
            "scene",
            "software",
            "layout",
            0.995,
            &ssim_result,
        );
        assert!(!record.passed, "black vs white must produce a failed record");
        assert!(record.delta < 0.0, "delta must be negative when failing");
    }

    /// Passed record correctly marks passed = true for identical images.
    #[test]
    fn failure_record_passed_test_marked() {
        let img = solid_rgba(64, 64, 128, 64, 200);
        let ssim_result = compute_ssim(&img, &img, 64, 64);
        let record = SsimFailureRecord::from_ssim_result(
            "scene",
            "software",
            "layout",
            0.995,
            &ssim_result,
        );
        assert!(record.passed, "identical images must produce a passed record");
    }
}
