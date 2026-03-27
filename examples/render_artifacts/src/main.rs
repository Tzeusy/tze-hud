//! # render-artifacts — Layer 4 Developer Visibility Artifact Generator
//!
//! Generates the full Layer 4 artifact set for one validation run.
//!
//! ## Usage
//!
//! ```sh
//! # Basic run — uses scene registry goldens from tests/golden/
//! cargo run --bin render-artifacts -- --branch main
//!
//! # Specify output directory and branch
//! cargo run --bin render-artifacts -- --output test_results --branch feature/foo
//!
//! # Load telemetry from a previous benchmark run
//! cargo run --bin render-artifacts -- --telemetry-dir /path/to/telemetry
//!
//! # Print the LLM-readable summary to stdout
//! cargo run --bin render-artifacts -- --branch main --print-summary
//! ```
//!
//! ## Output
//!
//! Creates `{output}/{YYYYMMDD-HHmmss}-{branch}/` containing:
//! - `index.html`      — self-contained gallery
//! - `manifest.json`   — machine-readable index
//! - `scenes/{name}/`  — per-scene artifacts
//! - `benchmarks/{name}/` — per-benchmark artifacts
//!
//! ## Spec references
//!
//! - validation-framework/spec.md §Layer 4 - Developer Visibility Artifacts (lines 118-134)
//! - validation-framework/spec.md §LLM Development Loop (lines 253-264):
//!   step 4 "render-artifacts binary for Layer 4 (include summary.md in PR)"

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tze_hud_scene::test_scenes::{ClockMs, TestSceneRegistry};
use tze_hud_validation::golden::{GoldenStore, find_golden_dir};
use tze_hud_validation::layer4::{
    ArtifactBuilder, ArtifactOptions, BenchmarkArtifactInput, SceneArtifactInput, SceneDescription,
    SceneMetrics, SceneStatus, llm_summary_json,
};

// ─── CLI args ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Args {
    /// Output root directory (default: `test_results`).
    output: PathBuf,
    /// Git branch name (default: detected from environment, else "unknown").
    branch: String,
    /// Optional path to a directory containing per-scene telemetry JSON files.
    /// File naming convention: `{scene_name}_telemetry.json`.
    telemetry_dir: Option<PathBuf>,
    /// Optional path to a benchmark output JSON (from `benchmark --emit`).
    benchmark_json: Option<PathBuf>,
    /// Print the LLM-readable summary JSON to stdout after generation.
    print_summary: bool,
}

fn parse_args() -> Args {
    let mut output = PathBuf::from("test_results");
    let mut branch = detect_branch();
    let mut telemetry_dir: Option<PathBuf> = None;
    let mut benchmark_json: Option<PathBuf> = None;
    let mut print_summary = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                if i < args.len() {
                    output = PathBuf::from(&args[i]);
                } else {
                    eprintln!("error: --output requires a value");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--branch" | "-b" => {
                i += 1;
                if i < args.len() {
                    branch = args[i].clone();
                } else {
                    eprintln!("error: --branch requires a value");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--telemetry-dir" => {
                i += 1;
                if i < args.len() {
                    telemetry_dir = Some(PathBuf::from(&args[i]));
                } else {
                    eprintln!("error: --telemetry-dir requires a value");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--benchmark-json" => {
                i += 1;
                if i < args.len() {
                    benchmark_json = Some(PathBuf::from(&args[i]));
                } else {
                    eprintln!("error: --benchmark-json requires a value");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "--print-summary" => {
                print_summary = true;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    Args {
        output,
        branch,
        telemetry_dir,
        benchmark_json,
        print_summary,
    }
}

fn print_usage() {
    eprintln!(
        "Usage: render-artifacts [OPTIONS]\n\
         \n\
         Options:\n\
         --output, -o <dir>         Output root directory [default: test_results]\n\
         --branch, -b <name>        Git branch name [default: auto-detect]\n\
         --telemetry-dir <dir>      Directory with per-scene telemetry JSON files\n\
         --benchmark-json <path>    Path to benchmark output JSON\n\
         --print-summary            Print LLM-readable summary JSON to stdout\n\
         --help, -h                 Show this help\n"
    );
}

/// Attempt to detect the current git branch from the environment or git.
fn detect_branch() -> String {
    // CI environments often set these.
    for var in &["GITHUB_HEAD_REF", "GITHUB_REF_NAME", "CI_COMMIT_BRANCH"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    // Fall back to reading .git/HEAD.
    if let Ok(head) = std::fs::read_to_string(".git/HEAD") {
        let head = head.trim();
        if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
            return branch.to_string();
        }
    }
    "unknown".to_string()
}

// ─── Benchmark output types (mirrors examples/benchmark/src/main.rs) ─────────

/// Minimal subset of BenchmarkOutput we need to parse.
#[derive(Debug, Deserialize, Serialize)]
struct BenchmarkOutput {
    pub sessions: Vec<SessionResult>,
    pub validation: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
struct SessionResult {
    pub name: String,
    pub summary: serde_json::Value,
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args = parse_args();

    eprintln!("render-artifacts: generating Layer 4 artifact set");
    eprintln!("  output root : {}", args.output.display());
    eprintln!("  branch      : {}", args.branch);

    let opts = ArtifactOptions {
        output_root: args.output.clone(),
        branch: args.branch.clone(),
        spec_ids: vec![
            "layer-4-artifact-gen".to_string(),
            "layer-4-pr-ci".to_string(),
            "layer-4-manifest".to_string(),
            "layer-4-per-scene-explanation".to_string(),
            "llm-development-loop".to_string(),
        ],
    };

    let mut builder = match ArtifactBuilder::new(&args.output, &args.branch, opts) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to create artifact builder: {e}");
            std::process::exit(1);
        }
    };

    eprintln!("  run dir     : {}", builder.run_dir().display());

    // Discover the golden store (best-effort; missing goldens → SKIP status).
    let golden_store = find_golden_dir().map(|dir| GoldenStore::new(dir));

    // Walk every registered test scene.
    let registry = TestSceneRegistry::new();
    let scene_names = TestSceneRegistry::scene_names();
    let mut scenes_processed = 0usize;

    for &name in scene_names {
        let (_graph, spec) = match registry.build(name, ClockMs::FIXED) {
            Some(pair) => pair,
            None => {
                eprintln!("  warning: scene '{name}' not found in registry, skipping");
                continue;
            }
        };

        let desc = SceneDescription {
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            expected_tab_count: spec.expected_tab_count,
            expected_tile_count: spec.expected_tile_count,
            has_hit_regions: spec.has_hit_regions,
            has_zones: spec.has_zones,
        };

        // Try to load golden reference (software backend).
        let (golden_pixels, golden_w, golden_h) = match &golden_store {
            Some(store) => match store.load(name, "software") {
                Ok(img) => (Some(img.pixels.clone()), img.width, img.height),
                Err(_) => (None, 1920, 1080),
            },
            None => (None, 1920, 1080),
        };

        // Load per-scene telemetry if available.
        let telemetry_json = args.telemetry_dir.as_ref().and_then(|dir| {
            let path = dir.join(format!("{name}_telemetry.json"));
            std::fs::read(&path).ok()
        });

        // Determine status.
        // Without a rendered image (which requires a live render pass), we
        // mark as SKIP.  This binary can be extended to drive the headless
        // compositor when the `headless` feature is enabled.
        let status = if golden_pixels.is_some() {
            // Golden exists — would normally compare a live render; mark SKIP
            // because we have no rendered pixels in this code path.
            SceneStatus::Skip
        } else {
            SceneStatus::Skip
        };

        let metrics = telemetry_json
            .as_ref()
            .map_or_else(SceneMetrics::default, |j| {
                // Best-effort parse frame count from telemetry.
                serde_json::from_slice::<serde_json::Value>(j)
                    .ok()
                    .and_then(|v| {
                        let frames = v["total_frames"].as_u64();
                        let p99 = v["frame_time"]["p99"].as_u64();
                        Some(SceneMetrics {
                            ssim_score: None,
                            frames_rendered: frames,
                            frame_time_p99_us: p99,
                            lease_violations: 0,
                            budget_overruns: 0,
                        })
                    })
                    .unwrap_or_default()
            });

        let input = SceneArtifactInput {
            description: desc,
            status,
            metrics,
            rendered_pixels: None,
            width: golden_w,
            height: golden_h,
            golden_pixels,
            diff_pixels: None,
            telemetry_json,
            changes_since_golden: None,
        };

        if let Err(e) = builder.add_scene(input) {
            eprintln!("  warning: add_scene '{name}': {e}");
        }
        scenes_processed += 1;
    }

    eprintln!("  scenes processed: {scenes_processed}");

    // Add benchmark artifacts if a benchmark JSON was provided.
    if let Some(ref bench_path) = args.benchmark_json {
        match load_benchmark_artifacts(&mut builder, bench_path) {
            Ok(n) => eprintln!("  benchmarks added: {n}"),
            Err(e) => eprintln!("  warning: benchmark artifacts: {e}"),
        }
    }

    let manifest = match builder.finalise() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: finalise failed: {e}");
            std::process::exit(1);
        }
    };

    eprintln!(
        "render-artifacts: done — {} scenes ({} pass, {} fail, {} skip), {} benchmarks",
        manifest.summary.total_scenes,
        manifest.summary.passed,
        manifest.summary.failed,
        manifest.summary.skipped,
        manifest.summary.total_benchmarks,
    );

    if args.print_summary {
        match llm_summary_json(&manifest) {
            Ok(json) => println!("{json}"),
            Err(e) => eprintln!("warning: llm summary serialisation failed: {e}"),
        }
    }
}

/// Load benchmark sessions from a `BenchmarkOutput` JSON file and add them
/// as benchmark artifacts.
///
/// Returns the number of benchmark artifacts added.
fn load_benchmark_artifacts(
    builder: &mut ArtifactBuilder,
    path: &PathBuf,
) -> Result<usize, String> {
    let json_bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;

    let bench: BenchmarkOutput =
        serde_json::from_slice(&json_bytes).map_err(|e| format!("parse benchmark JSON: {e}"))?;

    let mut count = 0;
    for session in &bench.sessions {
        let telemetry_json = serde_json::to_vec_pretty(&session.summary)
            .map_err(|e| format!("re-serialise session: {e}"))?;
        let histogram_json = serde_json::to_vec_pretty(&session.summary)
            .map_err(|e| format!("histogram placeholder: {e}"))?;

        let input = BenchmarkArtifactInput {
            name: session.name.clone(),
            session_telemetry_json: telemetry_json,
            histogram_json,
            calibration_json: None,
            hardware_info_json: None,
        };
        builder.add_benchmark(input)?;
        count += 1;
    }

    Ok(count)
}

// ─── Unit tests (always compiled) ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_branch_returns_string() {
        // Just verify it returns without panicking.
        let branch = detect_branch();
        assert!(!branch.is_empty());
    }

    #[test]
    fn test_main_invocable_with_minimal_args() {
        // Smoke test: create a builder and finalise without scenes.
        let tmp = std::env::temp_dir().join(format!(
            "tze_hud_render_artifacts_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);

        let opts = tze_hud_validation::layer4::ArtifactOptions {
            output_root: tmp.clone(),
            branch: "test".to_string(),
            spec_ids: vec![],
        };
        let builder = ArtifactBuilder::new(&tmp, "test", opts).unwrap();
        let manifest = builder.finalise().unwrap();

        // Verify the exact expected output files are present in the run directory.
        let run_dir = tmp.join(&manifest.run_id);
        assert!(
            run_dir.join("manifest.json").exists(),
            "manifest.json must exist in run dir"
        );
        assert!(
            run_dir.join("index.html").exists(),
            "index.html must exist in run dir"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
