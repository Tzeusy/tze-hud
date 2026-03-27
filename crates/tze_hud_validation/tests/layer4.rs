//! Layer 4 developer visibility artifact integration tests.
//!
//! These tests cover the acceptance criteria from validation-framework/spec.md:
//!
//!   Requirement: Layer 4 - Developer Visibility Artifacts (lines 118-134)
//!
//!   Scenario: PR CI artifact generation (line 123-125)
//!     WHEN a CI run executes for a pull request
//!     THEN Layer 4 MUST generate the full artifact set including index.html,
//!          manifest.json, per-scene directories with rendered/golden/diff/telemetry,
//!          and benchmarks
//!
//!   Scenario: Machine-readable manifest (line 127-129)
//!     WHEN Layer 4 artifacts are generated
//!     THEN manifest.json MUST contain an entry for every test scene with status
//!          (pass/fail), metrics, and paths to all artifact files
//!
//!   Scenario: Per-scene explanation (line 131-134)
//!     WHEN a test scene's artifacts are generated
//!     THEN explanation.md MUST be auto-generated from scene registry metadata
//!
//!   Requirement: LLM Development Loop (lines 253-264)
//!     WHEN a test fails with structured output
//!     THEN the output MUST be sufficient for an LLM to diagnose the root cause
//!
//!   Requirement: V1 Success Criterion - Autonomous LLM Development Workflow (lines 324-331)
//!     An LLM MUST be able to open a PR with developer visibility artifacts.

use std::fs;
use std::path::PathBuf;

use tze_hud_scene::test_scenes::{ClockMs, TestSceneRegistry};
use tze_hud_validation::{
    ArtifactBuilder, ArtifactOptions, BenchmarkArtifactInput, SceneArtifactInput, SceneDescription,
    SceneMetrics, SceneStatus, generate_explanation_md, llm_summary_json,
};

// ─── Test helpers ─────────────────────────────────────────────────────────────

fn temp_dir(suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "tze_hud_layer4_integ_{}_{}",
        suffix,
        std::process::id()
    ))
}

fn minimal_rgba8(w: u32, h: u32) -> Vec<u8> {
    vec![128u8, 64, 32, 255].repeat((w * h) as usize)
}

fn make_scene_desc(name: &str) -> SceneDescription {
    let registry = TestSceneRegistry::new();
    if let Some((_graph, spec)) = registry.build(name, ClockMs::FIXED) {
        SceneDescription {
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            expected_tab_count: spec.expected_tab_count,
            expected_tile_count: spec.expected_tile_count,
            has_hit_regions: spec.has_hit_regions,
            has_zones: spec.has_zones,
        }
    } else {
        SceneDescription {
            name: name.to_string(),
            description: "Fallback description.".to_string(),
            expected_tab_count: 1,
            expected_tile_count: 1,
            has_hit_regions: false,
            has_zones: false,
        }
    }
}

fn make_builder(tmp: &PathBuf, branch: &str) -> ArtifactBuilder {
    let opts = ArtifactOptions {
        output_root: tmp.clone(),
        branch: branch.to_string(),
        spec_ids: vec![
            "layer-4-pr-ci".to_string(),
            "layer-4-manifest".to_string(),
            "layer-4-per-scene-explanation".to_string(),
        ],
    };
    ArtifactBuilder::new(tmp, branch, opts).expect("builder creation failed")
}

// ─── Spec: PR CI artifact generation (lines 123-125) ─────────────────────────

/// WHEN a CI run executes for a pull request
/// THEN Layer 4 MUST generate the full artifact set
#[test]
fn test_pr_ci_artifact_generation_full_set() {
    let tmp = temp_dir("full_set");
    let _ = fs::remove_dir_all(&tmp);

    let mut builder = make_builder(&tmp, "ci-branch");
    let run_dir = builder.run_dir().to_path_buf();

    // Add a passing scene with all artifacts.
    builder
        .add_scene(SceneArtifactInput {
            description: make_scene_desc("single_tile_solid"),
            status: SceneStatus::Pass,
            metrics: SceneMetrics {
                ssim_score: Some(0.998),
                frames_rendered: Some(60),
                frame_time_p99_us: Some(12_000),
                lease_violations: 0,
                budget_overruns: 0,
            },
            rendered_pixels: Some(minimal_rgba8(16, 9)),
            width: 16,
            height: 9,
            golden_pixels: Some(minimal_rgba8(16, 9)),
            diff_pixels: Some(minimal_rgba8(16, 9)),
            telemetry_json: Some(br#"{"total_frames": 60}"#.to_vec()),
            changes_since_golden: None,
        })
        .unwrap();

    // Add a benchmark.
    builder
        .add_benchmark(BenchmarkArtifactInput {
            name: "max_tiles_stress".to_string(),
            session_telemetry_json: br#"{"fps": 62.4}"#.to_vec(),
            histogram_json: br#"{"p99_us": 14200}"#.to_vec(),
            calibration_json: Some(br#"{"cpu": 1.0, "gpu": 1.0}"#.to_vec()),
            hardware_info_json: Some(br#"{"vendor": "llvmpipe"}"#.to_vec()),
        })
        .unwrap();

    let manifest = builder.finalise().unwrap();

    // index.html MUST exist.
    assert!(
        run_dir.join("index.html").exists(),
        "index.html must be generated"
    );

    // manifest.json MUST exist.
    assert!(
        run_dir.join("manifest.json").exists(),
        "manifest.json must be generated"
    );

    // Per-scene directory MUST contain all four artifact files.
    let scene_dir = run_dir.join("scenes").join("single_tile_solid");
    assert!(
        scene_dir.join("rendered.png").exists(),
        "rendered.png must exist"
    );
    assert!(
        scene_dir.join("golden.png").exists(),
        "golden.png must exist"
    );
    assert!(scene_dir.join("diff.png").exists(), "diff.png must exist");
    assert!(
        scene_dir.join("telemetry.json").exists(),
        "telemetry.json must exist"
    );
    assert!(
        scene_dir.join("explanation.md").exists(),
        "explanation.md must exist"
    );

    // Per-benchmark directory MUST exist.
    let bench_dir = run_dir.join("benchmarks").join("max_tiles_stress");
    assert!(
        bench_dir.join("telemetry.json").exists(),
        "benchmark telemetry.json must exist"
    );
    assert!(
        bench_dir.join("histogram.json").exists(),
        "benchmark histogram.json must exist"
    );
    assert!(
        bench_dir.join("calibration.json").exists(),
        "benchmark calibration.json must exist"
    );

    // Manifest counts must be accurate.
    assert_eq!(manifest.summary.total_scenes, 1);
    assert_eq!(manifest.summary.passed, 1);
    assert_eq!(manifest.summary.total_benchmarks, 1);

    let _ = fs::remove_dir_all(&tmp);
}

// ─── Spec: Machine-readable manifest (lines 127-129) ─────────────────────────

/// WHEN Layer 4 artifacts are generated
/// THEN manifest.json MUST contain an entry for every test scene with
///      status, metrics, and paths to all artifact files
#[test]
fn test_manifest_json_contains_entry_per_scene_with_status_metrics_paths() {
    let tmp = temp_dir("manifest");
    let _ = fs::remove_dir_all(&tmp);

    // Add all 25 registered scenes.
    let mut builder = make_builder(&tmp, "main");
    let scene_names = TestSceneRegistry::scene_names();
    for &name in scene_names {
        builder
            .add_scene(SceneArtifactInput {
                description: make_scene_desc(name),
                status: SceneStatus::Skip,
                metrics: SceneMetrics::default(),
                rendered_pixels: None,
                width: 1920,
                height: 1080,
                golden_pixels: None,
                diff_pixels: None,
                telemetry_json: None,
                changes_since_golden: None,
            })
            .unwrap();
    }

    let run_dir = builder.run_dir().to_path_buf();
    let manifest = builder.finalise().unwrap();

    // Every scene must appear in the manifest.
    assert_eq!(
        manifest.scenes.len(),
        scene_names.len(),
        "manifest must have one entry per scene"
    );

    // Parse the written manifest.json and verify structure.
    let json_bytes = fs::read(run_dir.join("manifest.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();

    let scenes = parsed["scenes"].as_array().expect("scenes must be array");
    assert_eq!(
        scenes.len(),
        scene_names.len(),
        "JSON scenes array count mismatch"
    );

    for scene_entry in scenes {
        assert!(
            scene_entry["name"].is_string(),
            "each scene must have a name"
        );
        assert!(
            scene_entry["status"].is_string(),
            "each scene must have a status"
        );
        assert!(
            scene_entry["metrics"].is_object(),
            "each scene must have metrics"
        );
        assert!(
            scene_entry["paths"].is_object(),
            "each scene must have paths"
        );
        assert!(
            scene_entry["paths"]["explanation_md"].is_string(),
            "explanation_md path required"
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

/// Spec: manifest.json schema version and spec_ids are present.
#[test]
fn test_manifest_schema_version_and_spec_ids() {
    let tmp = temp_dir("schema");
    let _ = fs::remove_dir_all(&tmp);

    let opts = ArtifactOptions {
        output_root: tmp.clone(),
        branch: "main".to_string(),
        spec_ids: vec![
            "layer-4-pr-ci".to_string(),
            "layer-4-manifest".to_string(),
            "layer-4-per-scene-explanation".to_string(),
        ],
    };
    let builder = ArtifactBuilder::new(&tmp, "main", opts).unwrap();
    let run_dir = builder.run_dir().to_path_buf();
    let _ = builder.finalise().unwrap();

    let json_bytes = fs::read(run_dir.join("manifest.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();

    assert_eq!(parsed["schema_version"], 1);
    let spec_ids = parsed["spec_ids"]
        .as_array()
        .expect("spec_ids must be array");
    let ids: Vec<&str> = spec_ids.iter().filter_map(|v| v.as_str()).collect();
    assert!(ids.contains(&"layer-4-pr-ci"), "must contain pr-ci spec id");
    assert!(
        ids.contains(&"layer-4-manifest"),
        "must contain manifest spec id"
    );

    let _ = fs::remove_dir_all(&tmp);
}

// ─── Spec: Per-scene explanation (lines 131-134) ──────────────────────────────

/// WHEN a test scene's artifacts are generated
/// THEN explanation.md MUST be auto-generated from scene registry metadata
#[test]
fn test_explanation_md_generated_from_registry_metadata() {
    let registry = TestSceneRegistry::new();
    let test_scenes = [
        "single_tile_solid",
        "overlapping_tiles_zorder",
        "zone_publish_subtitle",
        "lease_expiry",
    ];

    for &name in &test_scenes {
        let (_graph, spec) = registry.build(name, ClockMs::FIXED).unwrap();
        let desc = SceneDescription {
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            expected_tab_count: spec.expected_tab_count,
            expected_tile_count: spec.expected_tile_count,
            has_hit_regions: spec.has_hit_regions,
            has_zones: spec.has_zones,
        };
        let metrics = SceneMetrics {
            ssim_score: Some(0.997),
            ..Default::default()
        };

        let md = generate_explanation_md(&desc, SceneStatus::Pass, &metrics, None);

        assert!(
            md.contains(&format!("# Scene: {name}")),
            "explanation.md must contain scene name in title for {name}"
        );
        assert!(
            md.contains(spec.description),
            "explanation.md must contain the registry description for {name}"
        );
        assert!(
            md.contains("## What this scene tests"),
            "explanation.md must have 'What this scene tests' section for {name}"
        );
        assert!(
            md.contains("## What to look for"),
            "explanation.md must have 'What to look for' section for {name}"
        );
        assert!(
            md.contains("## Automated results"),
            "explanation.md must have 'Automated results' section for {name}"
        );
        assert!(
            md.contains("auto-generated"),
            "explanation.md must note auto-generation for {name}"
        );
    }
}

/// explanation.md for `changes_since_golden` correctly includes the delta text.
#[test]
fn test_explanation_md_includes_changes_since_golden() {
    let desc = SceneDescription {
        name: "tab_switch".to_string(),
        description: "Tab switch animation test.".to_string(),
        expected_tab_count: 2,
        expected_tile_count: 2,
        has_hit_regions: false,
        has_zones: false,
    };
    let changes = "z-order of the content tile changed from 2 to 3 due to spec update";
    let md = generate_explanation_md(
        &desc,
        SceneStatus::Fail,
        &SceneMetrics::default(),
        Some(changes),
    );

    assert!(
        md.contains("Changes since previous golden"),
        "must have changes section"
    );
    assert!(
        md.contains(changes),
        "must include the actual change description"
    );
}

// ─── Spec: All 25 scenes produce artifacts ────────────────────────────────────

/// All 25 test scene names from the registry must be processable.
#[test]
fn test_all_25_scenes_produce_artifact_directories() {
    let tmp = temp_dir("all25");
    let _ = fs::remove_dir_all(&tmp);

    let scene_names = TestSceneRegistry::scene_names();
    assert_eq!(
        scene_names.len(),
        25,
        "registry must contain exactly 25 scenes"
    );

    let mut builder = make_builder(&tmp, "main");
    let run_dir = builder.run_dir().to_path_buf();

    for &name in scene_names {
        builder
            .add_scene(SceneArtifactInput {
                description: make_scene_desc(name),
                status: SceneStatus::Skip,
                metrics: SceneMetrics::default(),
                rendered_pixels: None,
                width: 1,
                height: 1,
                golden_pixels: None,
                diff_pixels: None,
                telemetry_json: None,
                changes_since_golden: None,
            })
            .unwrap();
    }

    let manifest = builder.finalise().unwrap();

    assert_eq!(
        manifest.summary.total_scenes, 25,
        "all 25 scenes in manifest"
    );

    // Each scene must have an explanation.md.
    for &name in scene_names {
        let expl = run_dir.join("scenes").join(name).join("explanation.md");
        assert!(expl.exists(), "explanation.md missing for scene {name}");
        let content = fs::read_to_string(&expl).unwrap();
        assert!(
            content.contains(&format!("# Scene: {name}")),
            "explanation.md for {name} must have correct title"
        );
    }

    let _ = fs::remove_dir_all(&tmp);
}

// ─── Spec: index.html self-contained gallery ──────────────────────────────────

/// index.html must be self-contained with no external dependencies.
#[test]
fn test_index_html_is_self_contained_no_external_deps() {
    let tmp = temp_dir("html_self");
    let _ = fs::remove_dir_all(&tmp);

    let mut builder = make_builder(&tmp, "main");
    let run_dir = builder.run_dir().to_path_buf();

    builder
        .add_scene(SceneArtifactInput {
            description: make_scene_desc("empty_scene"),
            status: SceneStatus::Pass,
            metrics: SceneMetrics::default(),
            rendered_pixels: None,
            width: 1,
            height: 1,
            golden_pixels: None,
            diff_pixels: None,
            telemetry_json: None,
            changes_since_golden: None,
        })
        .unwrap();

    let _ = builder.finalise().unwrap();
    let html = fs::read_to_string(run_dir.join("index.html")).unwrap();

    // Self-contained: no links to external HTTP resources.
    assert!(
        !html.contains("href=\"http"),
        "no external href links allowed"
    );
    assert!(
        !html.contains("src=\"http"),
        "no external script/img src allowed"
    );
    assert!(!html.contains("//cdn"), "no CDN references allowed");
    assert!(
        !html.contains("//fonts."),
        "no external font references allowed"
    );

    // Must contain inline CSS and JS.
    assert!(html.contains("<style>"), "must have inline CSS");
    assert!(html.contains("<script>"), "must have inline JS");

    // Must embed the LLM-readable manifest JSON.
    assert!(
        html.contains("application/json"),
        "must embed LLM-readable JSON"
    );
    assert!(
        html.contains("artifact-manifest"),
        "must have manifest script element"
    );
}

/// index.html must include pass/fail badges.
#[test]
fn test_index_html_contains_status_badges() {
    let tmp = temp_dir("html_badges");
    let _ = fs::remove_dir_all(&tmp);

    let mut builder = make_builder(&tmp, "main");
    let run_dir = builder.run_dir().to_path_buf();

    builder
        .add_scene(SceneArtifactInput {
            description: make_scene_desc("single_tile_solid"),
            status: SceneStatus::Pass,
            metrics: SceneMetrics {
                ssim_score: Some(0.999),
                ..Default::default()
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

    builder
        .add_scene(SceneArtifactInput {
            description: make_scene_desc("overlapping_tiles_zorder"),
            status: SceneStatus::Fail,
            metrics: SceneMetrics {
                ssim_score: Some(0.991),
                ..Default::default()
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

    let _ = builder.finalise().unwrap();
    let html = fs::read_to_string(run_dir.join("index.html")).unwrap();

    // Summary bar must show counts.
    assert!(html.contains("1 PASS"), "must show pass count");
    assert!(html.contains("1 FAIL"), "must show fail count");

    // Filter buttons must exist.
    assert!(
        html.contains("filterScenes"),
        "must have filter JS function"
    );

    let _ = fs::remove_dir_all(&tmp);
}

// ─── Spec: LLM-readable diagnostic output (lines 258-264) ────────────────────

/// WHEN a test fails with structured output
/// THEN the output MUST be sufficient for an LLM to diagnose the root cause.
#[test]
fn test_llm_structured_failure_output_for_ssim_regression() {
    let tmp = temp_dir("llm_diag");
    let _ = fs::remove_dir_all(&tmp);

    let mut builder = make_builder(&tmp, "feature");

    // Add a failing scene with SSIM below threshold.
    builder
        .add_scene(SceneArtifactInput {
            description: make_scene_desc("overlapping_tiles_zorder"),
            status: SceneStatus::Fail,
            metrics: SceneMetrics {
                ssim_score: Some(0.988), // below 0.995 layout threshold
                frames_rendered: Some(120),
                frame_time_p99_us: Some(14_000),
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

    // manifest.diagnostics must have an entry for the failed scene.
    assert_eq!(manifest.diagnostics.len(), 1);
    let diag = &manifest.diagnostics[0];
    assert_eq!(diag.scene_name, "overlapping_tiles_zorder");

    let ssim_diag = diag.ssim.as_ref().expect("SSIM diagnostic must be present");
    assert_eq!(ssim_diag.metric_name, "ssim_score");
    assert!((ssim_diag.actual_value - 0.988).abs() < 1e-6);
    assert!((ssim_diag.budget_value - 0.995).abs() < 1e-6);
    assert!(
        ssim_diag.regression_pct > 0.0,
        "regression_pct must be positive"
    );

    // Description must contain actionable text.
    assert!(ssim_diag.description.contains("SSIM"), "must mention SSIM");
    assert!(
        ssim_diag.description.contains("diff.png"),
        "must reference diff.png for diagnosis"
    );

    let _ = fs::remove_dir_all(&tmp);
}

/// Frame time performance regression diagnostic.
#[test]
fn test_llm_structured_failure_output_for_frame_time_regression() {
    let tmp = temp_dir("llm_perf");
    let _ = fs::remove_dir_all(&tmp);

    let mut builder = make_builder(&tmp, "perf-fix");

    builder
        .add_scene(SceneArtifactInput {
            description: make_scene_desc("max_tiles_stress"),
            status: SceneStatus::Fail,
            metrics: SceneMetrics {
                ssim_score: None,
                frames_rendered: Some(300),
                frame_time_p99_us: Some(25_000), // 25ms — over 16.6ms budget
                lease_violations: 0,
                budget_overruns: 1,
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
    assert_eq!(diag.scene_name, "max_tiles_stress");

    let perf = diag
        .performance
        .as_ref()
        .expect("performance diagnostic must be present");
    assert_eq!(perf.metric_name, "frame_time_p99");
    assert_eq!(perf.actual_value_us, 25_000);
    assert_eq!(perf.budget_value_us, 16_600);
    assert!(perf.regression_pct > 0.0);
    assert!(
        perf.description.contains("p99 frame time"),
        "must describe the metric"
    );
    assert!(
        perf.description.contains("telemetry.json"),
        "must reference telemetry.json"
    );

    let _ = fs::remove_dir_all(&tmp);
}

/// LLM summary JSON is parseable and contains all required fields.
#[test]
fn test_llm_summary_json_parseable_all_fields() {
    let tmp = temp_dir("llm_json");
    let _ = fs::remove_dir_all(&tmp);

    let mut builder = make_builder(&tmp, "main");

    builder
        .add_scene(SceneArtifactInput {
            description: make_scene_desc("empty_scene"),
            status: SceneStatus::Pass,
            metrics: SceneMetrics::default(),
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
    let json_str = llm_summary_json(&manifest).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Required top-level fields for LLM consumption.
    assert!(parsed["run_id"].is_string(), "must have run_id");
    assert!(parsed["branch"].is_string(), "must have branch");
    assert!(parsed["timestamp"].is_string(), "must have timestamp");
    assert!(parsed["summary"].is_object(), "must have summary");
    assert!(
        parsed["all_scenes_status"].is_object(),
        "must have all_scenes_status"
    );

    // Each scene status must be present.
    assert_eq!(parsed["all_scenes_status"]["empty_scene"], "PASS");

    let _ = fs::remove_dir_all(&tmp);
}

// ─── Spec: Benchmark context and hardware info ────────────────────────────────

/// Benchmark directory includes calibration vector and hardware info.
#[test]
fn test_benchmark_artifacts_include_calibration_and_hardware() {
    let tmp = temp_dir("bench_context");
    let _ = fs::remove_dir_all(&tmp);

    let mut builder = make_builder(&tmp, "main");
    let run_dir = builder.run_dir().to_path_buf();

    let calibration_json = serde_json::json!({
        "cpu": 1.0,
        "gpu": 0.12,
        "upload": 0.15,
        "note": "CI runner with llvmpipe"
    });
    let hardware_json = serde_json::json!({
        "vendor": "Mesa",
        "renderer": "llvmpipe",
        "cpu_cores": 4
    });

    builder
        .add_benchmark(BenchmarkArtifactInput {
            name: "coalesced_dashboard".to_string(),
            session_telemetry_json: br#"{"total_frames": 300, "fps": 60.1}"#.to_vec(),
            histogram_json: br#"{"frame_time": {"p50": 12000, "p95": 14500, "p99": 16000}}"#
                .to_vec(),
            calibration_json: Some(serde_json::to_vec_pretty(&calibration_json).unwrap()),
            hardware_info_json: Some(serde_json::to_vec_pretty(&hardware_json).unwrap()),
        })
        .unwrap();

    let manifest = builder.finalise().unwrap();

    let bench_dir = run_dir.join("benchmarks").join("coalesced_dashboard");
    assert!(
        bench_dir.join("calibration.json").exists(),
        "calibration.json must exist"
    );
    assert!(
        bench_dir.join("hardware_info.json").exists(),
        "hardware_info.json must exist"
    );

    // Verify calibration contents are valid JSON.
    let cal_bytes = fs::read(bench_dir.join("calibration.json")).unwrap();
    let cal: serde_json::Value = serde_json::from_slice(&cal_bytes).unwrap();
    assert_eq!(cal["cpu"], 1.0_f64);
    assert!((cal["gpu"].as_f64().unwrap() - 0.12).abs() < 1e-6);

    assert_eq!(manifest.benchmarks[0].name, "coalesced_dashboard");
    assert!(
        manifest.benchmarks[0].paths.calibration_json.is_some(),
        "manifest must include calibration path"
    );
    assert!(
        manifest.benchmarks[0].paths.hardware_info_json.is_some(),
        "manifest must include hardware_info path"
    );

    let _ = fs::remove_dir_all(&tmp);
}

// ─── Spec: Run directory naming ───────────────────────────────────────────────

/// Run directory name must follow `{YYYYMMDD-HHmmss}-{branch}` format.
#[test]
fn test_run_directory_timestamp_branch_naming() {
    let tmp = temp_dir("naming");
    let _ = fs::remove_dir_all(&tmp);

    let builder = make_builder(&tmp, "feature-foo");
    let run_dir = builder.run_dir().to_path_buf();
    let _ = builder.finalise().unwrap();

    // The run dir parent must be the output root.
    assert_eq!(run_dir.parent().unwrap(), tmp.as_path());

    let name = run_dir.file_name().unwrap().to_str().unwrap();
    // Format: YYYYMMDDHHmmss-branch-name
    assert!(
        name.len() > 15,
        "run directory name must be longer than timestamp: got {name}"
    );
    assert!(
        name.contains("feature-foo"),
        "run directory must contain branch name: got {name}"
    );
    // Timestamp part: first 15 chars must be digits + dash.
    let timestamp_part = &name[..15];
    assert!(
        timestamp_part
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-'),
        "timestamp prefix {timestamp_part:?} must be digits and dashes"
    );

    let _ = fs::remove_dir_all(&tmp);
}

/// Branch names with slashes are sanitised for filesystem safety.
#[test]
fn test_branch_name_with_slash_sanitised() {
    let tmp = temp_dir("slash_branch");
    let _ = fs::remove_dir_all(&tmp);

    let builder = make_builder(&tmp, "feature/foo-bar");
    let run_dir = builder.run_dir().to_path_buf();
    let _ = builder.finalise().unwrap();

    let name = run_dir.file_name().unwrap().to_str().unwrap();
    assert!(
        !name.contains('/'),
        "run dir must not contain slashes: got {name}"
    );
    assert!(
        name.contains("feature-foo-bar"),
        "slash replaced with dash in run dir name: got {name}"
    );

    let _ = fs::remove_dir_all(&tmp);
}
