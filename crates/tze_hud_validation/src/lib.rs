//! # tze_hud_validation
//!
//! Visual regression and developer visibility artifact validation for tze_hud.
//!
//! ## Architecture
//!
//! This crate implements two validation layers:
//!
//! - **Layer 2** (SSIM visual regression) — validation-framework/spec.md lines 54-70.
//! - **Layer 4** (developer visibility artifacts) — validation-framework/spec.md lines 118-134.
//!
//! ### Modules
//!
//! - [`ssim`]   — SSIM computation (mean + per-region breakdown)
//! - [`phash`]  — Perceptual hash pre-screening (fast path before SSIM)
//! - [`golden`] — Golden reference management (naming, load, update)
//! - [`diff`]   — Diff heatmap generation + structured JSON failure output
//! - [`layer2`] — Layer 2 entry point tying SSIM/golden/diff together
//! - [`layer4`] — Layer 4 artifact builder (index.html, manifest.json, per-scene dirs)
//! - [`error`]  — Error types
//!
//! ## Usage
//!
//! ### Layer 2 (SSIM comparison)
//!
//! ```rust,no_run
//! use tze_hud_validation::layer2::{Layer2Validator, TestType};
//!
//! let pixels: Vec<u8> = vec![0u8; 1920 * 1080 * 4];
//! let validator = Layer2Validator::new_from_workspace();
//! let _ = validator.compare(
//!     "single_tile_solid", "software", TestType::Layout, &pixels, 1920, 1080
//! );
//! ```
//!
//! ### Layer 4 (artifact generation)
//!
//! ```rust,no_run
//! use tze_hud_validation::layer4::{ArtifactBuilder, ArtifactOptions};
//!
//! let opts = ArtifactOptions::default();
//! let builder = ArtifactBuilder::new("test_results", "main", opts).unwrap();
//! let manifest = builder.finalise().unwrap();
//! println!("{}", serde_json::to_string_pretty(&manifest).unwrap());
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
pub mod layer4;

pub use error::ValidationError;
pub use layer2::{
    Layer2Validator, TestType, ComparisonOutcome,
    SSIM_THRESHOLD_LAYOUT, SSIM_THRESHOLD_MEDIA,
};
pub use ssim::{compute_ssim, SsimResult, RegionSsim};
pub use phash::{compute_phash, pre_screen, PHash, PreScreenResult};
pub use golden::{GoldenStore, GoldenImage, find_golden_dir, BACKEND_SOFTWARE, BACKEND_HARDWARE};
pub use diff::{generate_heatmap, encode_heatmap_png, SsimFailureRecord, RegionFailureDetail};
pub use layer4::{
    ArtifactBuilder, ArtifactManifest, ArtifactOptions, SceneArtifactInput, SceneDescription,
    SceneMetrics, SceneStatus, BenchmarkArtifactInput, generate_explanation_md, llm_summary_json,
};
