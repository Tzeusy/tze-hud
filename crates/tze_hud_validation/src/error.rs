//! Error types for the tze_hud_validation crate.

use std::path::PathBuf;
use thiserror::Error;

/// Errors produced during Layer 2 visual regression validation.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// The golden reference image does not exist for the given scene and backend.
    ///
    /// Fix: generate the golden by running `cargo run --bin update-goldens --features headless`.
    #[error(
        "golden reference not found for scene '{scene}' backend '{backend}': {path}",
        path = path.display()
    )]
    GoldenNotFound {
        scene: String,
        backend: String,
        path: PathBuf,
    },

    /// I/O or image codec error reading or writing a golden image.
    #[error("golden I/O error at {}: {cause}", path.display())]
    GoldenIo { path: PathBuf, cause: String },

    /// The rendered image dimensions do not match the golden reference.
    #[error("dimension mismatch: rendered {rendered_w}×{rendered_h}, golden {golden_w}×{golden_h}")]
    DimensionMismatch {
        rendered_w: u32,
        rendered_h: u32,
        golden_w: u32,
        golden_h: u32,
    },

    /// The rendered buffer length does not match `width * height * 4`.
    ///
    /// Returned by `Layer2Validator::compare` when the caller passes a buffer
    /// of the wrong size, preventing a downstream panic in `compute_ssim`.
    #[error(
        "rendered buffer size mismatch: got {actual_len} bytes, \
         expected {width}×{height}×4 = {expected_len} bytes"
    )]
    BufferSizeMismatch {
        actual_len: usize,
        expected_len: usize,
        width: u32,
        height: u32,
    },

    /// SSIM fell below the required threshold.
    ///
    /// This is the primary validation failure mode.
    #[error(
        "SSIM regression in scene '{scene}': \
         actual={actual:.6} threshold={threshold:.6} delta={delta:+.6}"
    )]
    SsimRegression {
        scene: String,
        actual: f64,
        threshold: f64,
        delta: f64,
    },
}
