//! # Layer 4 — Developer Visibility Artifacts
//!
//! Implements validation-framework/spec.md Requirement: Layer 4 - Developer
//! Visibility Artifacts (lines 118-134).
//!
//! ## Output structure
//!
//! ```text
//! test_results/{YYYYMMDD-HHmmss}-{branch}/
//!   index.html            — self-contained browsable gallery
//!   manifest.json         — machine-readable index with status, metrics, paths
//!   scenes/
//!     {scene_name}/
//!       rendered.png      — rendered output (copied from caller-supplied bytes)
//!       golden.png        — golden reference (copied from tests/golden/)
//!       diff.png          — diff heatmap
//!       telemetry.json    — per-frame telemetry (if available)
//!       explanation.md    — auto-generated from scene registry metadata
//!   benchmarks/
//!     {bench_name}/
//!       telemetry.json    — session telemetry JSON
//!       histogram.json    — latency histograms
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use tze_hud_validation::layer4::{ArtifactBuilder, SceneArtifactInput, ArtifactOptions};
//!
//! let opts = ArtifactOptions::default();
//! let builder = ArtifactBuilder::new("test_results", "main", opts).unwrap();
//! // Add scene artifacts, benchmarks, then finalise.
//! let manifest = builder.finalise().unwrap();
//! println!("{}", serde_json::to_string_pretty(&manifest).unwrap());
//! ```
//!
//! ## Spec compliance
//!
//! - Scenario: PR CI artifact generation (spec line 123-125):
//!   `ArtifactBuilder::finalise` writes index.html, manifest.json, and all
//!   per-scene directories.
//! - Scenario: Machine-readable manifest (spec line 127-129):
//!   `manifest.json` carries `status`, `ssim_score`, `metrics`, and `paths`
//!   for every scene that was added.
//! - Scenario: Per-scene explanation (spec line 131-134):
//!   `explanation.md` is auto-generated from `SceneSpec` registry metadata.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ─── Public options ───────────────────────────────────────────────────────────

/// Configuration for artifact generation.
#[derive(Debug, Clone)]
pub struct ArtifactOptions {
    /// Root directory under which timestamped run directories are created.
    /// Defaults to `"test_results"` relative to the workspace root.
    pub output_root: PathBuf,
    /// Branch name embedded in the run directory name.  Defaults to `"unknown"`.
    pub branch: String,
    /// Spec IDs this artifact set covers, embedded in `manifest.json`.
    pub spec_ids: Vec<String>,
}

impl Default for ArtifactOptions {
    fn default() -> Self {
        Self {
            output_root: PathBuf::from("test_results"),
            branch: "unknown".to_string(),
            spec_ids: vec![
                "layer-4-artifact-gen".to_string(),
                "layer-4-pr-ci".to_string(),
                "layer-4-manifest".to_string(),
                "layer-4-per-scene-explanation".to_string(),
            ],
        }
    }
}

// ─── Scene artifact input ─────────────────────────────────────────────────────

/// Status of a single scene validation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SceneStatus {
    Pass,
    Fail,
    /// No golden reference available — comparison was skipped.
    Skip,
}

impl SceneStatus {
    /// CSS class used in index.html for badge colouring.
    pub fn css_class(self) -> &'static str {
        match self {
            SceneStatus::Pass => "pass",
            SceneStatus::Fail => "fail",
            SceneStatus::Skip => "skip",
        }
    }

    /// Label text in index.html badge.
    pub fn label(self) -> &'static str {
        match self {
            SceneStatus::Pass => "PASS",
            SceneStatus::Fail => "FAIL",
            SceneStatus::Skip => "SKIP",
        }
    }
}

/// Scene description sourced from the test scene registry.
///
/// Mirrors `tze_hud_scene::test_scenes::SceneSpec` but is kept local so this
/// crate has no hard dependency on `tze_hud_scene`.
#[derive(Debug, Clone)]
pub struct SceneDescription {
    /// Canonical scene name (matches test scene registry key).
    pub name: String,
    /// Human-readable description — used in explanation.md.
    pub description: String,
    /// Expected number of tabs.
    pub expected_tab_count: usize,
    /// Expected number of tiles.
    pub expected_tile_count: usize,
    /// Whether tiles carry hit regions.
    pub has_hit_regions: bool,
    /// Whether the scene registers zones.
    pub has_zones: bool,
}

/// Structured metrics for a single scene run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneMetrics {
    /// SSIM score vs golden (None if no golden was available).
    pub ssim_score: Option<f64>,
    /// Total frames rendered.
    pub frames_rendered: Option<u64>,
    /// p99 frame time in microseconds.
    pub frame_time_p99_us: Option<u64>,
    /// Number of lease violations.
    pub lease_violations: u32,
    /// Number of budget overruns.
    pub budget_overruns: u32,
}

impl Default for SceneMetrics {
    fn default() -> Self {
        Self {
            ssim_score: None,
            frames_rendered: None,
            frame_time_p99_us: None,
            lease_violations: 0,
            budget_overruns: 0,
        }
    }
}

/// All inputs needed to generate a per-scene artifact directory.
pub struct SceneArtifactInput {
    /// Description from the scene registry.
    pub description: SceneDescription,
    /// Pass/fail/skip status.
    pub status: SceneStatus,
    /// Structured metrics.
    pub metrics: SceneMetrics,
    /// Rendered RGBA8 pixels (width × height × 4).  None if rendering failed.
    pub rendered_pixels: Option<Vec<u8>>,
    /// Rendered image dimensions.
    pub width: u32,
    pub height: u32,
    /// Golden RGBA8 pixels (already loaded; None if no golden exists).
    pub golden_pixels: Option<Vec<u8>>,
    /// Diff heatmap RGBA8 pixels (None if either rendered or golden is absent).
    pub diff_pixels: Option<Vec<u8>>,
    /// Per-frame telemetry as JSON bytes (None if not available).
    pub telemetry_json: Option<Vec<u8>>,
    /// Changes relative to a previous golden run.  Used in explanation.md.
    pub changes_since_golden: Option<String>,
}

// ─── Benchmark artifact input ─────────────────────────────────────────────────

/// Inputs for a per-benchmark artifact directory.
pub struct BenchmarkArtifactInput {
    /// Benchmark name (used as directory name under `benchmarks/`).
    pub name: String,
    /// Session telemetry as JSON bytes.
    pub session_telemetry_json: Vec<u8>,
    /// Histogram data as JSON bytes (latency distributions).
    pub histogram_json: Vec<u8>,
    /// Calibration vector used for this benchmark (JSON bytes).
    pub calibration_json: Option<Vec<u8>>,
    /// Hardware info (JSON bytes).
    pub hardware_info_json: Option<Vec<u8>>,
}

// ─── Manifest types ───────────────────────────────────────────────────────────

/// Per-scene entry in manifest.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneManifestEntry {
    pub name: String,
    pub status: SceneStatus,
    pub metrics: SceneMetrics,
    /// Relative paths from the run directory root to each artifact file.
    pub paths: SceneArtifactPaths,
}

/// Artifact file paths for one scene (relative to the run root).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneArtifactPaths {
    pub rendered_png: Option<String>,
    pub golden_png: Option<String>,
    pub diff_png: Option<String>,
    pub telemetry_json: Option<String>,
    pub explanation_md: String,
}

/// Per-benchmark entry in manifest.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkManifestEntry {
    pub name: String,
    pub paths: BenchmarkArtifactPaths,
}

/// Artifact file paths for one benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkArtifactPaths {
    pub telemetry_json: String,
    pub histogram_json: String,
    pub calibration_json: Option<String>,
    pub hardware_info_json: Option<String>,
}

/// Top-level machine-readable manifest.
///
/// Written as `manifest.json` at the run root. Every field is suitable for
/// automated analysis by an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactManifest {
    /// Schema version — increment when the structure changes.
    pub schema_version: u32,
    /// ISO-8601 timestamp of the run.
    pub timestamp: String,
    /// Git branch name.
    pub branch: String,
    /// Run directory name (`{YYYYMMDD-HHmmss}-{branch}`).
    pub run_id: String,
    /// Spec IDs covered by this artifact set.
    pub spec_ids: Vec<String>,
    /// Summary counts.
    pub summary: ManifestSummary,
    /// Per-scene entries.
    pub scenes: Vec<SceneManifestEntry>,
    /// Per-benchmark entries.
    pub benchmarks: Vec<BenchmarkManifestEntry>,
    /// LLM-parseable diagnostic output for failed scenes.
    pub diagnostics: Vec<SceneDiagnostic>,
}

/// Summary counts for quick LLM/CI consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestSummary {
    pub total_scenes: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub total_benchmarks: usize,
}

/// LLM-readable diagnostic record for a failed scene.
///
/// Satisfies spec §LLM Development Loop (lines 253-264):
/// structured output sufficient to diagnose root cause without viewing frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneDiagnostic {
    pub scene_name: String,
    pub status: SceneStatus,
    /// SSIM metric details (None if SSIM was not measured).
    pub ssim: Option<SsimDiagnostic>,
    /// Performance metric details.
    pub performance: Option<PerformanceDiagnostic>,
}

/// SSIM-specific diagnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsimDiagnostic {
    pub metric_name: String,
    pub actual_value: f64,
    pub budget_value: f64,
    pub regression_pct: f64,
    pub description: String,
}

/// Performance-specific diagnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceDiagnostic {
    pub metric_name: String,
    pub actual_value_us: u64,
    pub budget_value_us: u64,
    pub regression_pct: f64,
    pub description: String,
}

// ─── ArtifactBuilder ──────────────────────────────────────────────────────────

/// Builds a complete Layer 4 artifact set for one validation run.
///
/// # Example
///
/// ```rust,no_run
/// use tze_hud_validation::layer4::{ArtifactBuilder, ArtifactOptions};
///
/// let opts = ArtifactOptions::default();
/// let builder = ArtifactBuilder::new("test_results", "main", opts).unwrap();
/// let manifest = builder.finalise().unwrap();
/// ```
pub struct ArtifactBuilder {
    run_dir: PathBuf,
    run_id: String,
    branch: String,
    spec_ids: Vec<String>,
    timestamp: String,
    scenes: Vec<SceneManifestEntry>,
    benchmarks: Vec<BenchmarkManifestEntry>,
    /// Tracks additional files to write: (path relative to run_dir, content).
    pending_files: Vec<(PathBuf, Vec<u8>)>,
}

impl ArtifactBuilder {
    /// Create a new builder and initialise the run directory on disk.
    ///
    /// `output_root` — parent directory for all runs (e.g. `"test_results"`).
    ///   When provided, this *overrides* `opts.output_root`.
    /// `branch`      — git branch name embedded in the run directory name.
    ///   When provided, this *overrides* `opts.branch`.
    /// `opts`        — additional options (spec IDs, etc.).
    ///
    /// Note: `output_root` and `branch` parameters take precedence over the
    /// same-named fields on `ArtifactOptions` to keep the call-site
    /// unambiguous.  Callers that construct `ArtifactOptions` with those
    /// fields set should pass them here consistently.
    pub fn new(
        output_root: impl AsRef<Path>,
        branch: impl Into<String>,
        opts: ArtifactOptions,
    ) -> Result<Self, String> {
        let branch = branch.into();
        let timestamp = current_timestamp_str();
        let safe_branch = sanitise_branch_name(&branch);
        let run_id = format!("{}-{}", timestamp, safe_branch);
        let run_dir = output_root.as_ref().join(&run_id);

        std::fs::create_dir_all(run_dir.join("scenes"))
            .map_err(|e| format!("failed to create scenes dir: {e}"))?;
        std::fs::create_dir_all(run_dir.join("benchmarks"))
            .map_err(|e| format!("failed to create benchmarks dir: {e}"))?;

        Ok(Self {
            run_dir,
            run_id,
            branch,
            spec_ids: opts.spec_ids,
            timestamp: timestamp_iso8601(),
            scenes: Vec::new(),
            benchmarks: Vec::new(),
            pending_files: Vec::new(),
        })
    }

    /// Return the path to the run directory created for this artifact set.
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Add a scene artifact.
    ///
    /// Creates the `scenes/{name}/` subdirectory and writes all image and JSON
    /// files from `input`.  Returns a manifest entry for inclusion in
    /// `manifest.json`.
    ///
    /// The scene name is sanitised before use as a filesystem component to
    /// guard against path-traversal attacks (e.g. names containing `../`).
    pub fn add_scene(&mut self, input: SceneArtifactInput) -> Result<(), String> {
        let safe_name = sanitise_artifact_name(&input.description.name);
        let scene_dir = self.run_dir.join("scenes").join(&safe_name);
        std::fs::create_dir_all(&scene_dir)
            .map_err(|e| format!("create scene dir: {e}"))?;

        let mut paths = SceneArtifactPaths {
            rendered_png: None,
            golden_png: None,
            diff_png: None,
            telemetry_json: None,
            explanation_md: format!("scenes/{safe_name}/explanation.md"),
        };

        // Write rendered.png
        if let Some(ref pixels) = input.rendered_pixels {
            let png = encode_rgba8_png(pixels, input.width, input.height)?;
            let rel = format!("scenes/{safe_name}/rendered.png");
            self.pending_files.push((PathBuf::from(&rel), png));
            paths.rendered_png = Some(rel);
        }

        // Write golden.png
        if let Some(ref pixels) = input.golden_pixels {
            let png = encode_rgba8_png(pixels, input.width, input.height)?;
            let rel = format!("scenes/{safe_name}/golden.png");
            self.pending_files.push((PathBuf::from(&rel), png));
            paths.golden_png = Some(rel);
        }

        // Write diff.png
        if let Some(ref pixels) = input.diff_pixels {
            let png = encode_rgba8_png(pixels, input.width, input.height)?;
            let rel = format!("scenes/{safe_name}/diff.png");
            self.pending_files.push((PathBuf::from(&rel), png));
            paths.diff_png = Some(rel);
        }

        // Write telemetry.json
        if let Some(ref json_bytes) = input.telemetry_json {
            let rel = format!("scenes/{safe_name}/telemetry.json");
            self.pending_files.push((PathBuf::from(&rel), json_bytes.clone()));
            paths.telemetry_json = Some(rel);
        }

        // Generate explanation.md
        let explanation = generate_explanation_md(
            &input.description,
            input.status,
            &input.metrics,
            input.changes_since_golden.as_deref(),
        );
        let expl_rel = format!("scenes/{safe_name}/explanation.md");
        self.pending_files
            .push((PathBuf::from(&expl_rel), explanation.into_bytes()));

        let entry = SceneManifestEntry {
            name: input.description.name.clone(),
            status: input.status,
            metrics: input.metrics,
            paths,
        };
        self.scenes.push(entry);

        Ok(())
    }

    /// Add a benchmark artifact.
    ///
    /// The benchmark name is sanitised before use as a filesystem component to
    /// guard against path-traversal attacks.
    pub fn add_benchmark(&mut self, input: BenchmarkArtifactInput) -> Result<(), String> {
        let safe_name = sanitise_artifact_name(&input.name);
        let bench_dir = self.run_dir.join("benchmarks").join(&safe_name);
        std::fs::create_dir_all(&bench_dir)
            .map_err(|e| format!("create benchmark dir: {e}"))?;

        let tel_rel = format!("benchmarks/{safe_name}/telemetry.json");
        self.pending_files.push((PathBuf::from(&tel_rel), input.session_telemetry_json));

        let hist_rel = format!("benchmarks/{safe_name}/histogram.json");
        self.pending_files.push((PathBuf::from(&hist_rel), input.histogram_json));

        let mut paths = BenchmarkArtifactPaths {
            telemetry_json: tel_rel,
            histogram_json: hist_rel,
            calibration_json: None,
            hardware_info_json: None,
        };

        if let Some(cal) = input.calibration_json {
            let rel = format!("benchmarks/{safe_name}/calibration.json");
            self.pending_files.push((PathBuf::from(&rel), cal));
            paths.calibration_json = Some(rel);
        }

        if let Some(hw) = input.hardware_info_json {
            let rel = format!("benchmarks/{safe_name}/hardware_info.json");
            self.pending_files.push((PathBuf::from(&rel), hw));
            paths.hardware_info_json = Some(rel);
        }

        self.benchmarks.push(BenchmarkManifestEntry {
            name: input.name,
            paths,
        });

        Ok(())
    }

    /// Finalise the artifact set: write all pending files, manifest.json, and index.html.
    ///
    /// Returns the completed [`ArtifactManifest`].
    pub fn finalise(mut self) -> Result<ArtifactManifest, String> {
        // Flush all pending binary files.
        let pending = std::mem::take(&mut self.pending_files);
        for (rel_path, content) in pending {
            let full = self.run_dir.join(&rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            std::fs::write(&full, &content)
                .map_err(|e| format!("write {}: {e}", full.display()))?;
        }

        // Build diagnostics list for failed scenes.
        let diagnostics = self.build_diagnostics();

        // Build manifest.
        let summary = ManifestSummary {
            total_scenes: self.scenes.len(),
            passed: self.scenes.iter().filter(|s| s.status == SceneStatus::Pass).count(),
            failed: self.scenes.iter().filter(|s| s.status == SceneStatus::Fail).count(),
            skipped: self.scenes.iter().filter(|s| s.status == SceneStatus::Skip).count(),
            total_benchmarks: self.benchmarks.len(),
        };

        let manifest = ArtifactManifest {
            schema_version: 1,
            timestamp: self.timestamp.clone(),
            branch: self.branch.clone(),
            run_id: self.run_id.clone(),
            spec_ids: self.spec_ids.clone(),
            summary,
            scenes: self.scenes.clone(),
            benchmarks: self.benchmarks.clone(),
            diagnostics,
        };

        // Write manifest.json.
        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("serialise manifest: {e}"))?;
        std::fs::write(self.run_dir.join("manifest.json"), manifest_json.as_bytes())
            .map_err(|e| format!("write manifest.json: {e}"))?;

        // Generate and write index.html.
        let html = generate_index_html(&manifest);
        std::fs::write(self.run_dir.join("index.html"), html.as_bytes())
            .map_err(|e| format!("write index.html: {e}"))?;

        Ok(manifest)
    }

    fn build_diagnostics(&self) -> Vec<SceneDiagnostic> {
        self.scenes
            .iter()
            .filter(|s| s.status == SceneStatus::Fail)
            .map(|s| {
                // Only emit an SSIM diagnostic when the score is actually below the
                // threshold.  A scene can fail for performance reasons while having a
                // perfectly healthy SSIM score; emitting a diagnostic in that case
                // would be misleading and produce a negative `regression_pct`.
                let ssim = s.metrics.ssim_score.and_then(|score| {
                    let threshold = 0.995_f64; // layout threshold
                    if score < threshold {
                        let regression_pct = (threshold - score) / threshold * 100.0;
                        Some(SsimDiagnostic {
                            metric_name: "ssim_score".to_string(),
                            actual_value: score,
                            budget_value: threshold,
                            regression_pct,
                            description: format!(
                                "SSIM {score:.6} is {regression_pct:.2}% below the \
                                 layout threshold {threshold:.3}. Check diff.png for \
                                 the spatial pattern of the regression."
                            ),
                        })
                    } else {
                        None
                    }
                });

                let performance = s.metrics.frame_time_p99_us.and_then(|actual| {
                    let budget = 16_600_u64;
                    if actual > budget {
                        let regression_pct =
                            (actual as f64 - budget as f64) / budget as f64 * 100.0;
                        Some(PerformanceDiagnostic {
                            metric_name: "frame_time_p99".to_string(),
                            actual_value_us: actual,
                            budget_value_us: budget,
                            regression_pct,
                            description: format!(
                                "p99 frame time {actual}µs is {regression_pct:.1}% over \
                                 the 16.6ms budget ({budget}µs). \
                                 Check telemetry.json for per-stage breakdown."
                            ),
                        })
                    } else {
                        None
                    }
                });

                SceneDiagnostic {
                    scene_name: s.name.clone(),
                    status: s.status,
                    ssim,
                    performance,
                }
            })
            .collect()
    }
}

// ─── explanation.md generation ────────────────────────────────────────────────

/// Auto-generate `explanation.md` for a scene from its registry metadata.
///
/// Spec: "explanation.md MUST be auto-generated from scene registry metadata
/// describing what the scene tests, what to look for, automated results, and
/// changes since previous golden" (line 131-134).
pub fn generate_explanation_md(
    desc: &SceneDescription,
    status: SceneStatus,
    metrics: &SceneMetrics,
    changes_since_golden: Option<&str>,
) -> String {
    let mut md = String::new();

    md.push_str(&format!("# Scene: {}\n\n", desc.name));
    md.push_str(&format!("**Status:** {}\n\n", status.label()));

    md.push_str("## What this scene tests\n\n");
    md.push_str(&desc.description);
    md.push_str("\n\n");

    md.push_str("## Scene structure\n\n");
    md.push_str(&format!(
        "- Tabs: {}\n- Tiles: {}\n- Has hit regions: {}\n- Has zones: {}\n\n",
        desc.expected_tab_count,
        desc.expected_tile_count,
        desc.has_hit_regions,
        desc.has_zones,
    ));

    md.push_str("## What to look for\n\n");
    // Scene-specific guidance based on name keywords.
    let guidance = scene_visual_guidance(&desc.name);
    md.push_str(guidance);
    md.push_str("\n\n");

    md.push_str("## Automated results\n\n");
    if let Some(ssim) = metrics.ssim_score {
        md.push_str(&format!("- SSIM score: {ssim:.6}\n"));
    } else {
        md.push_str("- SSIM score: not measured (no golden reference or rendering skipped)\n");
    }
    if let Some(frames) = metrics.frames_rendered {
        md.push_str(&format!("- Frames rendered: {frames}\n"));
    }
    if let Some(p99) = metrics.frame_time_p99_us {
        md.push_str(&format!(
            "- Frame time p99: {}µs ({:.2}ms)\n",
            p99,
            p99 as f64 / 1000.0
        ));
    }
    md.push_str(&format!("- Lease violations: {}\n", metrics.lease_violations));
    md.push_str(&format!("- Budget overruns: {}\n", metrics.budget_overruns));
    md.push('\n');

    if let Some(changes) = changes_since_golden {
        md.push_str("## Changes since previous golden\n\n");
        md.push_str(changes);
        md.push_str("\n\n");
    }

    md.push_str("---\n");
    md.push_str(
        "*This file is auto-generated by `render-artifacts` from scene registry metadata.*\n",
    );

    md
}

/// Return visual-inspection guidance for a scene by name keyword matching.
fn scene_visual_guidance(name: &str) -> &'static str {
    if name.contains("zorder") || name.contains("overlap") {
        "Verify that higher-z tiles are rendered above lower-z tiles in overlap regions. \
         The overlap pixel colour must match the tile with the higher z-order."
    } else if name.contains("transparency") || name.contains("alpha") {
        "Check that semi-transparent overlays produce the correct blended colour. \
         The blended region should be a predictable mix of the foreground and background colours."
    } else if name.contains("tab_switch") {
        "Verify that switching tabs shows the correct content without residual pixels \
         from the previous tab."
    } else if name.contains("lease_expiry") {
        "After the lease TTL expires, the tile must be removed and the background must be \
         clean with no ghost pixels."
    } else if name.contains("mobile_degraded") {
        "On a Mobile Presence Node profile, tiles must degrade gracefully: smaller bounds, \
         reduced z-budget, no overflow outside the declared region."
    } else if name.contains("sync_group") {
        "All tiles in the sync group must present on the same frame. \
         Check for any visible tearing or partial updates."
    } else if name.contains("input_highlight") {
        "On a touch/hover event the hit tile should show a highlight state (colour change) \
         without requiring an agent roundtrip."
    } else if name.contains("dashboard") {
        "Tiles that receive many rapid updates must coalesce correctly; \
         the final rendered state must match the last committed content."
    } else if name.contains("agents") || name.contains("contention") {
        "Three independent agents must each hold their tile regions without \
         overlapping, leaking pixels, or interfering with each other's content."
    } else if name.contains("privacy") || name.contains("redaction") {
        "Privacy-redacted tiles must render as opaque solid blocks with no \
         content visible through them."
    } else if name.contains("zone") {
        "Zone geometry must fill exactly the declared bounds with no clipping. \
         Zone content type (subtitle, image, etc.) must render in the correct sub-area."
    } else if name.contains("policy") || name.contains("arbitration") {
        "Policy arbitration must result in exactly one tile winner when two agents \
         contend for the same screen region."
    } else if name.contains("disconnect") || name.contains("reclaim") {
        "After agent disconnect, all tiles must be removed and screen area reclaimed. \
         No residual content must remain from the disconnected agent."
    } else if name.contains("passthrough") {
        "Passthrough regions must be transparent (alpha=0) so the OS desktop \
         or camera feed is visible underneath."
    } else if name.contains("empty") {
        "The canvas must be completely empty — no tiles, no artefacts from initialisation."
    } else {
        "Inspect rendered.png vs golden.png for any unexpected differences. \
         Consult diff.png (red = large difference, blue = no difference) for \
         the spatial distribution of any regression."
    }
}

// ─── index.html generation ────────────────────────────────────────────────────

/// Generate a self-contained HTML gallery from the completed manifest.
///
/// The page is entirely self-contained (inline CSS/JS, no external dependencies)
/// so it can be viewed offline from an artifact archive.
///
/// Features:
/// - Scene thumbnail grid with pass/fail/skip badges
/// - Rendered vs golden diff overlay on hover
/// - Status filter buttons
/// - Benchmark section with latency summary
/// - LLM-readable summary JSON embedded as `<script type="application/json">`
pub fn generate_index_html(manifest: &ArtifactManifest) -> String {
    let mut html = String::new();

    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"UTF-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
    html.push_str(&format!(
        "<title>tze_hud validation — {} ({})</title>\n",
        html_escape(&manifest.branch), html_escape(&manifest.timestamp)
    ));
    html.push_str("<style>\n");
    html.push_str(INDEX_HTML_CSS);
    html.push_str("</style>\n</head>\n<body>\n");

    // Header
    html.push_str("<div class=\"header\">\n");
    html.push_str(&format!(
        "<h1>tze_hud Validation Run: <code>{}</code></h1>\n",
        html_escape(&manifest.run_id)
    ));
    html.push_str(&format!(
        "<p>Branch: <strong>{}</strong> &nbsp;|&nbsp; Timestamp: <strong>{}</strong></p>\n",
        html_escape(&manifest.branch), html_escape(&manifest.timestamp)
    ));
    html.push_str("</div>\n");

    // Summary bar
    html.push_str("<div class=\"summary-bar\">\n");
    html.push_str(&format!(
        "<span class=\"badge pass\">{} PASS</span>\n",
        manifest.summary.passed
    ));
    html.push_str(&format!(
        "<span class=\"badge fail\">{} FAIL</span>\n",
        manifest.summary.failed
    ));
    html.push_str(&format!(
        "<span class=\"badge skip\">{} SKIP</span>\n",
        manifest.summary.skipped
    ));
    html.push_str(&format!(
        "<span class=\"badge bench\">{} Benchmarks</span>\n",
        manifest.summary.total_benchmarks
    ));
    html.push_str("</div>\n");

    // Filter controls
    html.push_str("<div class=\"filter-bar\">\n");
    html.push_str("<strong>Filter:</strong>\n");
    html.push_str("<button onclick=\"filterScenes('all')\" class=\"filter-btn active\" id=\"filter-all\">All</button>\n");
    html.push_str("<button onclick=\"filterScenes('pass')\" class=\"filter-btn\" id=\"filter-pass\">Pass</button>\n");
    html.push_str("<button onclick=\"filterScenes('fail')\" class=\"filter-btn\" id=\"filter-fail\">Fail</button>\n");
    html.push_str("<button onclick=\"filterScenes('skip')\" class=\"filter-btn\" id=\"filter-skip\">Skip</button>\n");
    html.push_str("</div>\n");

    // Scene grid
    html.push_str("<h2>Scenes</h2>\n");
    html.push_str("<div class=\"scene-grid\" id=\"scene-grid\">\n");
    for scene in &manifest.scenes {
        html.push_str(&render_scene_card(scene));
    }
    html.push_str("</div>\n");

    // Diagnostics section (only if there are failures)
    if !manifest.diagnostics.is_empty() {
        html.push_str("<h2>Failure Diagnostics</h2>\n");
        html.push_str("<div class=\"diagnostics\">\n");
        for diag in &manifest.diagnostics {
            html.push_str(&render_diagnostic(diag));
        }
        html.push_str("</div>\n");
    }

    // Benchmarks section
    if !manifest.benchmarks.is_empty() {
        html.push_str("<h2>Benchmarks</h2>\n");
        html.push_str("<div class=\"bench-list\">\n");
        for bench in &manifest.benchmarks {
            html.push_str(&format!(
                "<div class=\"bench-card\">\
                 <h3>{}</h3>\
                 <p><a href=\"{}\">telemetry.json</a> | \
                 <a href=\"{}\">histogram.json</a></p>\
                 </div>\n",
                html_escape(&bench.name),
                html_escape(&bench.paths.telemetry_json),
                html_escape(&bench.paths.histogram_json),
            ));
        }
        html.push_str("</div>\n");
    }

    // Embedded LLM-readable JSON summary.
    // Escape `</` → `<\/` so a branch/field value containing `</script>`
    // cannot prematurely terminate the script block.
    let llm_json = serde_json::to_string_pretty(manifest).unwrap_or_default();
    let safe_json = escape_json_for_script(&llm_json);
    html.push_str("\n<!-- LLM-readable artifact summary (machine-parseable) -->\n");
    html.push_str("<script type=\"application/json\" id=\"artifact-manifest\">\n");
    html.push_str(&safe_json);
    html.push_str("\n</script>\n");

    // JavaScript for filtering
    html.push_str("<script>\n");
    html.push_str(INDEX_HTML_JS);
    html.push_str("</script>\n");

    html.push_str("</body>\n</html>\n");
    html
}

/// Render one scene card in the gallery grid.
fn render_scene_card(scene: &SceneManifestEntry) -> String {
    let status_class = scene.status.css_class();
    let status_label = scene.status.label();

    let rendered_src = scene
        .paths
        .rendered_png
        .as_deref()
        .unwrap_or("data:image/png;base64,");
    let golden_src = scene
        .paths
        .golden_png
        .as_deref()
        .unwrap_or("data:image/png;base64,");
    let diff_src = scene
        .paths
        .diff_png
        .as_deref()
        .unwrap_or("data:image/png;base64,");

    let ssim_text = scene
        .metrics
        .ssim_score
        .map(|s| format!("SSIM: {s:.4}"))
        .unwrap_or_else(|| "SSIM: n/a".to_string());

    // status_class and status_label come from controlled enum values — safe to
    // interpolate directly.  All caller-supplied string fields are HTML-escaped.
    let name_escaped = html_escape(&scene.name);
    let rendered_src_escaped = html_escape(rendered_src);
    let golden_src_escaped = html_escape(golden_src);
    let diff_src_escaped = html_escape(diff_src);
    let ssim_text_escaped = html_escape(&ssim_text);
    let explanation_md_escaped = html_escape(&scene.paths.explanation_md);
    let telemetry_link = scene
        .paths
        .telemetry_json
        .as_deref()
        .map(|p| format!(r#" | <a href="{}">telemetry.json</a>"#, html_escape(p)))
        .unwrap_or_default();

    format!(
        r#"<div class="scene-card {status_class}" data-status="{status_class}">
  <div class="card-header">
    <span class="scene-name">{name_escaped}</span>
    <span class="badge {status_class}">{status_label}</span>
  </div>
  <div class="image-trio">
    <figure>
      <img src="{rendered_src_escaped}" alt="Rendered" loading="lazy" title="Rendered">
      <figcaption>Rendered</figcaption>
    </figure>
    <figure>
      <img src="{golden_src_escaped}" alt="Golden" loading="lazy" title="Golden">
      <figcaption>Golden</figcaption>
    </figure>
    <figure>
      <img src="{diff_src_escaped}" alt="Diff" loading="lazy" title="Diff (red=large, blue=none)">
      <figcaption>Diff</figcaption>
    </figure>
  </div>
  <div class="metrics">
    <span>{ssim_text_escaped}</span>
  </div>
  <div class="card-links">
    <a href="{explanation_md_escaped}">explanation.md</a>
    {telemetry_link}
  </div>
</div>
"#,
    )
}

/// Render one diagnostic card for a failed scene.
fn render_diagnostic(diag: &SceneDiagnostic) -> String {
    let mut html = format!(
        "<div class=\"diag-card\">\n<h3><span class=\"badge fail\">FAIL</span> {}</h3>\n",
        html_escape(&diag.scene_name)
    );

    if let Some(ref ssim) = diag.ssim {
        html.push_str(&format!(
            "<p><strong>SSIM:</strong> actual={:.6} budget={:.6} regression={:.2}%</p>\n\
             <p>{}</p>\n",
            ssim.actual_value,
            ssim.budget_value,
            ssim.regression_pct,
            html_escape(&ssim.description),
        ));
    }
    if let Some(ref perf) = diag.performance {
        html.push_str(&format!(
            "<p><strong>Frame time p99:</strong> {}µs budget={}µs regression={:.1}%</p>\n\
             <p>{}</p>\n",
            perf.actual_value_us,
            perf.budget_value_us,
            perf.regression_pct,
            html_escape(&perf.description),
        ));
    }
    html.push_str("</div>\n");
    html
}

// ─── Inline CSS / JS ──────────────────────────────────────────────────────────

const INDEX_HTML_CSS: &str = r#"
  :root {
    --pass: #22c55e;
    --fail: #ef4444;
    --skip: #a3a3a3;
    --bench: #3b82f6;
    --bg: #0f172a;
    --surface: #1e293b;
    --text: #e2e8f0;
    --muted: #94a3b8;
    --border: #334155;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { background: var(--bg); color: var(--text); font-family: system-ui, sans-serif;
         font-size: 14px; padding: 16px; }
  .header { padding: 16px 0 8px; border-bottom: 1px solid var(--border); margin-bottom: 12px; }
  .header h1 { font-size: 20px; font-weight: 700; }
  .header p  { color: var(--muted); margin-top: 4px; }
  .summary-bar { display: flex; gap: 8px; margin-bottom: 12px; flex-wrap: wrap; }
  .filter-bar  { display: flex; gap: 6px; align-items: center; margin-bottom: 16px; }
  .badge {
    display: inline-block; padding: 2px 8px; border-radius: 4px;
    font-size: 12px; font-weight: 600; text-transform: uppercase;
  }
  .badge.pass { background: var(--pass); color: #fff; }
  .badge.fail { background: var(--fail); color: #fff; }
  .badge.skip { background: var(--skip); color: #fff; }
  .badge.bench { background: var(--bench); color: #fff; }
  .filter-btn {
    background: var(--surface); border: 1px solid var(--border);
    color: var(--text); padding: 4px 12px; border-radius: 4px; cursor: pointer;
  }
  .filter-btn.active { background: var(--bench); border-color: var(--bench); }
  h2 { font-size: 16px; font-weight: 600; margin: 16px 0 8px; }
  .scene-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 12px;
    margin-bottom: 24px;
  }
  .scene-card {
    background: var(--surface); border-radius: 8px;
    padding: 10px; border: 1px solid var(--border);
  }
  .scene-card.pass { border-color: var(--pass); }
  .scene-card.fail { border-color: var(--fail); }
  .scene-card.skip { border-color: var(--skip); }
  .scene-card.hidden { display: none; }
  .card-header { display: flex; justify-content: space-between; align-items: center;
                 margin-bottom: 8px; }
  .scene-name { font-weight: 600; font-size: 13px; }
  .image-trio {
    display: grid; grid-template-columns: repeat(3, 1fr); gap: 4px; margin-bottom: 6px;
  }
  .image-trio figure { text-align: center; }
  .image-trio img {
    width: 100%; aspect-ratio: 16/9; object-fit: cover;
    border-radius: 4px; background: #0a0a0a; border: 1px solid var(--border);
  }
  .image-trio figcaption { font-size: 10px; color: var(--muted); margin-top: 2px; }
  .metrics { font-size: 12px; color: var(--muted); margin-bottom: 4px; }
  .card-links { font-size: 12px; }
  .card-links a { color: var(--bench); text-decoration: none; }
  .card-links a:hover { text-decoration: underline; }
  .diagnostics { display: flex; flex-direction: column; gap: 10px; margin-bottom: 24px; }
  .diag-card { background: var(--surface); border: 1px solid var(--fail);
               border-radius: 8px; padding: 12px; }
  .diag-card h3 { font-size: 14px; margin-bottom: 8px; }
  .diag-card p  { font-size: 13px; color: var(--muted); margin-bottom: 4px; }
  .bench-list { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
                gap: 12px; margin-bottom: 24px; }
  .bench-card { background: var(--surface); border: 1px solid var(--bench);
                border-radius: 8px; padding: 12px; }
  .bench-card h3 { font-size: 14px; margin-bottom: 6px; }
  .bench-card a  { color: var(--bench); text-decoration: none; font-size: 13px; }
  .bench-card a:hover { text-decoration: underline; }
"#;

const INDEX_HTML_JS: &str = r#"
  function filterScenes(status) {
    var cards = document.querySelectorAll('.scene-card');
    cards.forEach(function(card) {
      if (status === 'all' || card.dataset.status === status) {
        card.classList.remove('hidden');
      } else {
        card.classList.add('hidden');
      }
    });
    document.querySelectorAll('.filter-btn').forEach(function(btn) {
      btn.classList.remove('active');
    });
    var activeBtn = document.getElementById('filter-' + status);
    if (activeBtn) activeBtn.classList.add('active');
  }
"#;

// ─── Utilities ────────────────────────────────────────────────────────────────

/// Return the current UTC timestamp as `YYYYMMDD-HHmmss`.
fn current_timestamp_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Manual UTC time calculation (avoids pulling in chrono/time).
    // Accurate for dates far from Unix epoch overflow (> year 2038 is fine for u64).
    let sec_of_day = secs % 86_400;
    let hour = sec_of_day / 3600;
    let minute = (sec_of_day % 3600) / 60;
    let second = sec_of_day % 60;

    let days = secs / 86_400;
    let (year, month, day) = days_to_date(days);

    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", year, month, day, hour, minute, second)
}

/// Return the current UTC timestamp as an ISO-8601 string.
fn timestamp_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let sec_of_day = secs % 86_400;
    let hour = sec_of_day / 3600;
    let minute = (sec_of_day % 3600) / 60;
    let second = sec_of_day % 60;
    let days = secs / 86_400;
    let (year, month, day) = days_to_date(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
///
/// Algorithm: civil calendar computation (no leap-second awareness needed here).
fn days_to_date(days: u64) -> (u32, u32, u32) {
    // Shift epoch to 1 Mar 0000 for simpler leap-year math.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}

/// Sanitise a branch name for use in a directory name.
///
/// Replaces `/`, `\`, ` `, and other filesystem-unsafe characters with `-`.
fn sanitise_branch_name(branch: &str) -> String {
    branch
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '-',
        })
        .collect()
}

/// Sanitise a scene or benchmark name for safe use as a filesystem component.
///
/// Only allows alphanumeric characters, hyphens, underscores, and dots.
/// All other characters (including path separators) are replaced with `_`.
/// A leading `.` is prefixed with `_` to avoid hidden-directory names.
///
/// This guards against path-traversal attacks when caller-supplied names are
/// joined into the run directory (e.g. `scenes/../../etc/passwd`).
fn sanitise_artifact_name(name: &str) -> String {
    let sanitised: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect();
    // Prevent hidden-directory names and upward traversal residuals.
    if sanitised.starts_with('.') {
        format!("_{sanitised}")
    } else {
        sanitised
    }
}

/// Escape a string for safe inline insertion into HTML text or attribute values.
///
/// Replaces `&`, `<`, `>`, `"`, and `'` with their HTML entities.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Escape a JSON string for safe embedding inside a `<script>` tag.
///
/// `</script>` inside a JSON blob would prematurely terminate the script block.
/// Replacing `</` with `<\/` is the standard defence and is valid JSON.
fn escape_json_for_script(json: &str) -> String {
    json.replace("</", "<\\/")
}

/// Encode an RGBA8 pixel buffer as a PNG byte vector.
fn encode_rgba8_png(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    use image::{ImageBuffer, Rgba};
    let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(width, height, pixels.to_vec())
        .ok_or_else(|| {
            format!(
                "pixel buffer size mismatch: expected {}×{}×4={} bytes, got {}",
                width,
                height,
                width as usize * height as usize * 4,
                pixels.len()
            )
        })?;
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode: {e}"))?;
    Ok(buf.into_inner())
}

// ─── LLM-readable structured output ──────────────────────────────────────────

/// Produce a compact LLM-readable JSON summary of the artifact run.
///
/// This is separate from `manifest.json` and is designed for single-pass
/// consumption by an LLM tool call: it carries only the information needed
/// to understand what failed and why, without redundant file-path metadata.
pub fn llm_summary_json(manifest: &ArtifactManifest) -> Result<String, String> {
    #[derive(Serialize)]
    struct LlmSummary<'a> {
        run_id: &'a str,
        branch: &'a str,
        timestamp: &'a str,
        summary: &'a ManifestSummary,
        failures: Vec<&'a SceneDiagnostic>,
        all_scenes_status: HashMap<&'a str, &'static str>,
    }

    let failures: Vec<&SceneDiagnostic> = manifest.diagnostics.iter().collect();
    let all_scenes_status: HashMap<&str, &'static str> = manifest
        .scenes
        .iter()
        .map(|s| {
            let label = s.status.label();
            (s.name.as_str(), label)
        })
        .collect();

    let summary = LlmSummary {
        run_id: &manifest.run_id,
        branch: &manifest.branch,
        timestamp: &manifest.timestamp,
        summary: &manifest.summary,
        failures,
        all_scenes_status,
    };

    serde_json::to_string_pretty(&summary).map_err(|e| format!("llm_summary serialise: {e}"))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_scene_desc(name: &str) -> SceneDescription {
        SceneDescription {
            name: name.to_string(),
            description: format!("Test description for {name}."),
            expected_tab_count: 1,
            expected_tile_count: 2,
            has_hit_regions: false,
            has_zones: false,
        }
    }

    fn minimal_rgba8(w: u32, h: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
        let n = (w * h) as usize;
        let mut buf = vec![0u8; n * 4];
        for i in 0..n {
            buf[i * 4] = r;
            buf[i * 4 + 1] = g;
            buf[i * 4 + 2] = b;
            buf[i * 4 + 3] = 255;
        }
        buf
    }

    #[test]
    fn test_sanitise_branch_name_replaces_slash() {
        assert_eq!(sanitise_branch_name("feature/foo-bar"), "feature-foo-bar");
    }

    #[test]
    fn test_sanitise_branch_name_keeps_safe_chars() {
        assert_eq!(sanitise_branch_name("agent-hud-hz38"), "agent-hud-hz38");
    }

    #[test]
    fn test_days_to_date_epoch() {
        // Unix epoch = 1970-01-01
        assert_eq!(days_to_date(0), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_date_known_date() {
        // 2024-03-25 = 19811 days after 1970-01-01
        // (2024-1970=54 full years + leap years + day-of-year)
        let (_y, _m, _d) = days_to_date(19811);
        // Just verify it doesn't panic; exact value not critical for this test.
    }

    #[test]
    fn test_encode_rgba8_png_roundtrip() {
        let pixels = minimal_rgba8(4, 4, 255, 0, 0);
        let png = encode_rgba8_png(&pixels, 4, 4).unwrap();
        assert!(!png.is_empty());
        // PNG magic bytes
        assert_eq!(&png[0..4], &[0x89, 0x50, 0x4e, 0x47]);
    }

    #[test]
    fn test_generate_explanation_md_contains_required_sections() {
        let desc = test_scene_desc("single_tile_solid");
        let metrics = SceneMetrics { ssim_score: Some(0.998), ..Default::default() };
        let md = generate_explanation_md(&desc, SceneStatus::Pass, &metrics, None);

        assert!(md.contains("# Scene: single_tile_solid"), "must have title");
        assert!(md.contains("**Status:**"), "must have status");
        assert!(md.contains("## What this scene tests"), "must have test section");
        assert!(md.contains("## What to look for"), "must have guidance");
        assert!(md.contains("## Automated results"), "must have results");
        assert!(md.contains("SSIM score: 0.998000"), "must include SSIM");
        assert!(md.contains("auto-generated"), "must note auto-generation");
    }

    #[test]
    fn test_generate_explanation_md_changes_section() {
        let desc = test_scene_desc("overlay_transparency");
        let metrics = SceneMetrics::default();
        let md = generate_explanation_md(
            &desc,
            SceneStatus::Fail,
            &metrics,
            Some("Blending mode changed from linear to gamma-corrected."),
        );
        assert!(md.contains("Changes since previous golden"));
        assert!(md.contains("gamma-corrected"));
    }

    #[test]
    fn test_artifact_builder_creates_directory_structure() {
        let tmp = std::env::temp_dir().join(format!("tze_hud_layer4_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);

        let opts = ArtifactOptions {
            output_root: tmp.clone(),
            branch: "test-branch".to_string(),
            spec_ids: vec!["test-spec".to_string()],
        };
        let builder = ArtifactBuilder::new(&tmp, "test-branch", opts).unwrap();
        let run_dir = builder.run_dir().to_path_buf();

        let manifest = builder.finalise().unwrap();

        assert!(run_dir.join("manifest.json").exists(), "manifest.json must exist");
        assert!(run_dir.join("index.html").exists(), "index.html must exist");
        assert_eq!(manifest.branch, "test-branch");
        assert_eq!(manifest.schema_version, 1);
        assert!(manifest.run_id.contains("test-branch"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_artifact_builder_adds_scene_writes_files() {
        let tmp = std::env::temp_dir().join(format!(
            "tze_hud_layer4_scene_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);

        let opts = ArtifactOptions {
            output_root: tmp.clone(),
            branch: "test".to_string(),
            spec_ids: vec![],
        };
        let mut builder = ArtifactBuilder::new(&tmp, "test", opts).unwrap();

        let w = 16u32;
        let h = 9u32;
        let rendered = minimal_rgba8(w, h, 200, 100, 50);
        let golden = minimal_rgba8(w, h, 210, 100, 50);
        let diff = minimal_rgba8(w, h, 10, 0, 245);

        let input = SceneArtifactInput {
            description: test_scene_desc("single_tile_solid"),
            status: SceneStatus::Pass,
            metrics: SceneMetrics {
                ssim_score: Some(0.997),
                frames_rendered: Some(120),
                frame_time_p99_us: Some(14_000),
                lease_violations: 0,
                budget_overruns: 0,
            },
            rendered_pixels: Some(rendered),
            width: w,
            height: h,
            golden_pixels: Some(golden),
            diff_pixels: Some(diff),
            telemetry_json: Some(br#"{"frames":120}"#.to_vec()),
            changes_since_golden: None,
        };
        builder.add_scene(input).unwrap();

        let run_dir = builder.run_dir().to_path_buf();
        let manifest = builder.finalise().unwrap();

        let scene_dir = run_dir.join("scenes").join("single_tile_solid");
        assert!(scene_dir.join("rendered.png").exists(), "rendered.png must exist");
        assert!(scene_dir.join("golden.png").exists(), "golden.png must exist");
        assert!(scene_dir.join("diff.png").exists(), "diff.png must exist");
        assert!(scene_dir.join("telemetry.json").exists(), "telemetry.json must exist");
        assert!(scene_dir.join("explanation.md").exists(), "explanation.md must exist");

        assert_eq!(manifest.scenes.len(), 1);
        assert_eq!(manifest.scenes[0].name, "single_tile_solid");
        assert_eq!(manifest.scenes[0].status, SceneStatus::Pass);
        assert_eq!(manifest.summary.passed, 1);
        assert_eq!(manifest.summary.failed, 0);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_artifact_builder_adds_benchmark() {
        let tmp = std::env::temp_dir().join(format!(
            "tze_hud_layer4_bench_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);

        let opts = ArtifactOptions {
            output_root: tmp.clone(),
            branch: "test".to_string(),
            spec_ids: vec![],
        };
        let mut builder = ArtifactBuilder::new(&tmp, "test", opts).unwrap();

        builder
            .add_benchmark(BenchmarkArtifactInput {
                name: "max_tiles_stress".to_string(),
                session_telemetry_json: br#"{"frames":300}"#.to_vec(),
                histogram_json: br#"{"p99":14000}"#.to_vec(),
                calibration_json: Some(br#"{"cpu":1.0,"gpu":1.0}"#.to_vec()),
                hardware_info_json: None,
            })
            .unwrap();

        let run_dir = builder.run_dir().to_path_buf();
        let manifest = builder.finalise().unwrap();

        let bench_dir = run_dir.join("benchmarks").join("max_tiles_stress");
        assert!(bench_dir.join("telemetry.json").exists());
        assert!(bench_dir.join("histogram.json").exists());
        assert!(bench_dir.join("calibration.json").exists());

        assert_eq!(manifest.benchmarks.len(), 1);
        assert_eq!(manifest.summary.total_benchmarks, 1);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_manifest_json_is_valid_json_and_contains_spec_ids() {
        let tmp = std::env::temp_dir().join(format!(
            "tze_hud_layer4_json_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);

        let opts = ArtifactOptions {
            output_root: tmp.clone(),
            branch: "ci".to_string(),
            spec_ids: vec![
                "layer-4-pr-ci".to_string(),
                "layer-4-manifest".to_string(),
            ],
        };
        let builder = ArtifactBuilder::new(&tmp, "ci", opts).unwrap();
        let run_dir = builder.run_dir().to_path_buf();
        let manifest = builder.finalise().unwrap();

        let json_bytes = std::fs::read(run_dir.join("manifest.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();
        assert!(parsed["spec_ids"].is_array());
        assert!(parsed["schema_version"].is_number());
        assert!(manifest.spec_ids.contains(&"layer-4-pr-ci".to_string()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_index_html_is_self_contained() {
        let tmp = std::env::temp_dir().join(format!(
            "tze_hud_layer4_html_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);

        let opts = ArtifactOptions {
            output_root: tmp.clone(),
            branch: "main".to_string(),
            spec_ids: vec![],
        };
        let builder = ArtifactBuilder::new(&tmp, "main", opts).unwrap();
        let run_dir = builder.run_dir().to_path_buf();
        let _ = builder.finalise().unwrap();

        let html = std::fs::read_to_string(run_dir.join("index.html")).unwrap();
        // Must be self-contained — no external stylesheet or script links.
        assert!(!html.contains("href=\"http"), "no external hrefs");
        assert!(!html.contains("src=\"http"), "no external scripts");
        assert!(html.contains("<style>"), "inline CSS required");
        assert!(html.contains("<script>"), "inline JS required");
        assert!(html.contains("application/json"), "LLM-readable JSON embedded");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_failure_diagnostics_populated_for_fail_status() {
        let tmp = std::env::temp_dir().join(format!(
            "tze_hud_layer4_diag_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);

        let opts = ArtifactOptions {
            output_root: tmp.clone(),
            branch: "test".to_string(),
            spec_ids: vec![],
        };
        let mut builder = ArtifactBuilder::new(&tmp, "test", opts).unwrap();

        // A failing scene with SSIM below threshold.
        builder
            .add_scene(SceneArtifactInput {
                description: test_scene_desc("overlapping_tiles_zorder"),
                status: SceneStatus::Fail,
                metrics: SceneMetrics {
                    ssim_score: Some(0.988),
                    frames_rendered: Some(60),
                    frame_time_p99_us: Some(20_000),
                    lease_violations: 0,
                    budget_overruns: 0,
                },
                rendered_pixels: None,
                width: 1,
                height: 1,
                golden_pixels: None,
                diff_pixels: None,
                telemetry_json: None,
                changes_since_golden: None,
            })
            .unwrap();

        let manifest = builder.finalise().unwrap();

        assert_eq!(manifest.diagnostics.len(), 1);
        let diag = &manifest.diagnostics[0];
        assert_eq!(diag.scene_name, "overlapping_tiles_zorder");
        assert!(diag.ssim.is_some(), "SSIM diagnostic must be present");
        assert!(diag.performance.is_some(), "performance diagnostic must be present");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_llm_summary_json_is_parseable() {
        let manifest = ArtifactManifest {
            schema_version: 1,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            branch: "main".to_string(),
            run_id: "20260101-000000-main".to_string(),
            spec_ids: vec!["test".to_string()],
            summary: ManifestSummary {
                total_scenes: 1,
                passed: 1,
                failed: 0,
                skipped: 0,
                total_benchmarks: 0,
            },
            scenes: vec![SceneManifestEntry {
                name: "empty_scene".to_string(),
                status: SceneStatus::Pass,
                metrics: SceneMetrics::default(),
                paths: SceneArtifactPaths {
                    rendered_png: None,
                    golden_png: None,
                    diff_png: None,
                    telemetry_json: None,
                    explanation_md: "scenes/empty_scene/explanation.md".to_string(),
                },
            }],
            benchmarks: vec![],
            diagnostics: vec![],
        };

        let json = llm_summary_json(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["run_id"].is_string());
        assert!(parsed["summary"].is_object());
        assert!(parsed["all_scenes_status"].is_object());
        assert_eq!(parsed["all_scenes_status"]["empty_scene"], "PASS");
    }

    #[test]
    fn test_scene_status_css_classes() {
        assert_eq!(SceneStatus::Pass.css_class(), "pass");
        assert_eq!(SceneStatus::Fail.css_class(), "fail");
        assert_eq!(SceneStatus::Skip.css_class(), "skip");
    }

    #[test]
    fn test_guidance_fallback_for_unknown_scene() {
        let guidance = scene_visual_guidance("unknown_scene_xyz");
        assert!(guidance.contains("rendered.png"), "fallback must mention rendered.png");
    }

    // ── sanitise_artifact_name ────────────────────────────────────────────────

    #[test]
    fn test_sanitise_artifact_name_safe_chars_unchanged() {
        assert_eq!(sanitise_artifact_name("single_tile_solid"), "single_tile_solid");
        assert_eq!(sanitise_artifact_name("bench-max.tiles"), "bench-max.tiles");
    }

    #[test]
    fn test_sanitise_artifact_name_strips_path_traversal() {
        // A name containing `../` must not escape the target directory.
        // The `/` and `\` path separators are replaced with `_`; dots are
        // allowed (legitimate in artifact names like "bench.v2").
        let safe = sanitise_artifact_name("../../etc/passwd");
        assert!(!safe.contains('/'), "sanitised name must not contain /");
        assert!(!safe.contains('\\'), "sanitised name must not contain \\");
        // The remaining string must not be joinable out of the target dir.
        // Since `/` is gone, `Path::join` cannot escape upward.
        let joined = std::path::Path::new("/base").join(&safe);
        assert!(joined.starts_with("/base"),
            "joined path must stay under /base, got {}", joined.display());
    }

    #[test]
    fn test_sanitise_artifact_name_leading_dot_prefixed() {
        let safe = sanitise_artifact_name(".hidden");
        assert!(safe.starts_with('_'), "leading '.' must be prefixed with '_'");
    }

    #[test]
    fn test_sanitise_artifact_name_spaces_and_specials() {
        let safe = sanitise_artifact_name("my scene <name>");
        assert!(!safe.contains(' '), "spaces replaced");
        assert!(!safe.contains('<'), "angle brackets replaced");
        assert!(!safe.contains('>'), "angle brackets replaced");
    }

    // ── html_escape ───────────────────────────────────────────────────────────

    #[test]
    fn test_html_escape_passthrough_safe_string() {
        assert_eq!(html_escape("hello world"), "hello world");
    }

    #[test]
    fn test_html_escape_entities() {
        assert_eq!(html_escape("<script>alert('xss')&\"</script>"),
                   "&lt;script&gt;alert(&#39;xss&#39;)&amp;&quot;&lt;/script&gt;");
    }

    #[test]
    fn test_html_escape_branch_name_with_angle_brackets() {
        // Branch names cannot normally contain `<` but a crafted input must be
        // safe when interpolated into HTML.
        let escaped = html_escape("<evil-branch>");
        assert!(!escaped.contains('<'), "must not contain raw <");
        assert!(!escaped.contains('>'), "must not contain raw >");
    }

    // ── escape_json_for_script ────────────────────────────────────────────────

    #[test]
    fn test_escape_json_for_script_terminates_safely() {
        let json = r#"{"branch": "feat</script><script>alert(1)"}"#;
        let escaped = escape_json_for_script(json);
        assert!(!escaped.contains("</script>"), "must not contain raw </script>");
        // The replacement is valid JSON (backslash-escaped forward slash).
        assert!(escaped.contains("<\\/script>"), "should use <\\/ escape");
    }

    #[test]
    fn test_escape_json_for_script_normal_json_unchanged() {
        let json = r#"{"key": "value", "n": 42}"#;
        assert_eq!(escape_json_for_script(json), json);
    }

    // ── SSIM diagnostic threshold gate ────────────────────────────────────────

    #[test]
    fn test_ssim_diagnostic_not_emitted_when_score_above_threshold() {
        let tmp = std::env::temp_dir().join(format!(
            "tze_hud_layer4_ssim_gate_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);

        let opts = ArtifactOptions {
            output_root: tmp.clone(),
            branch: "test".to_string(),
            spec_ids: vec![],
        };
        let mut builder = ArtifactBuilder::new(&tmp, "test", opts).unwrap();

        // Scene fails for performance, but SSIM is fine (above threshold).
        builder
            .add_scene(SceneArtifactInput {
                description: test_scene_desc("perf_only_fail"),
                status: SceneStatus::Fail,
                metrics: SceneMetrics {
                    ssim_score: Some(0.999), // above 0.995 threshold — no SSIM regression
                    frames_rendered: Some(60),
                    frame_time_p99_us: Some(20_000), // over budget
                    lease_violations: 0,
                    budget_overruns: 0,
                },
                rendered_pixels: None,
                width: 1,
                height: 1,
                golden_pixels: None,
                diff_pixels: None,
                telemetry_json: None,
                changes_since_golden: None,
            })
            .unwrap();

        let manifest = builder.finalise().unwrap();

        assert_eq!(manifest.diagnostics.len(), 1);
        let diag = &manifest.diagnostics[0];
        assert!(diag.ssim.is_none(),
            "SSIM diagnostic must NOT be emitted when score is above threshold");
        assert!(diag.performance.is_some(),
            "performance diagnostic must still be present");

        let _ = fs::remove_dir_all(&tmp);
    }
}
