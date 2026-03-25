//! # tze_hud_validation
//!
//! Layer 2 visual regression validation for tze_hud via SSIM
//! (Structural Similarity Index Measure).
//!
//! ## Architecture
//!
//! This crate implements the validation-framework/spec.md
//! Requirement: Layer 2 - Visual Regression via SSIM (lines 54-70).
//!
//! ### Modules
//!
//! - [`ssim`]   — SSIM computation (mean + per-region breakdown)
//! - [`phash`]  — Perceptual hash pre-screening (fast path before SSIM)
//! - [`golden`] — Golden reference management (naming, load, update)
//! - [`diff`]   — Diff heatmap generation + structured JSON failure output
//! - [`layer2`] — Entry point tying everything together
//! - [`error`]  — Error types
//!
//! ## Usage
//!
//! ```rust,no_run
//! use tze_hud_validation::layer2::{Layer2Validator, TestType};
//!
//! let pixels: Vec<u8> = vec![0u8; 1920 * 1080 * 4];
//! let validator = Layer2Validator::new_from_workspace();
//! // Compare rendered RGBA8 pixels against the committed golden reference.
//! let _ = validator.compare(
//!     "single_tile_solid", "software", TestType::Layout, &pixels, 1920, 1080
//! );
//! ```
//!
//! ## Feature flags
//!
//! - `headless` — Enables integration tests that require a headless GPU compositor.
//!   Tests in the `tests/` directory are gated on this feature.
//!
//! ## SSIM thresholds (spec §Layer 2)
//!
//! | Test type         | Threshold |
//! |-------------------|-----------|
//! | Layout            | 0.995     |
//! | Media composition | 0.990     |
//!
//! An SSIM score of 0.996 passes layout; 0.993 fails.

pub mod error;
pub mod ssim;
pub mod phash;
pub mod golden;
pub mod diff;
pub mod layer2;

pub use error::ValidationError;
pub use layer2::{
    Layer2Validator, TestType, ComparisonOutcome,
    SSIM_THRESHOLD_LAYOUT, SSIM_THRESHOLD_MEDIA,
};
pub use ssim::{compute_ssim, SsimResult, RegionSsim};
pub use phash::{compute_phash, pre_screen, PHash, PreScreenResult};
pub use golden::{GoldenStore, GoldenImage, find_golden_dir, BACKEND_SOFTWARE, BACKEND_HARDWARE};
pub use diff::{generate_heatmap, encode_heatmap_png, SsimFailureRecord, RegionFailureDetail};
