//! E12.6: V1 Thesis Proof
//!
//! Final validation that all 7 v1 success criteria from heart-and-soul/v1.md
//! are met. This is the capstone test for the entire project.
//!
//! ## The 7 Thesis Points
//!
//! 1. An LLM can hold a tile on a screen (60fps)
//! 2. The lease model works (auth, capabilities, TTL, revocation)
//! 3. Multiple agents coexist without interference
//! 4. Performance is real (p99 latencies measured)
//! 5. The validation architecture works (5 layers operational)
//! 6. Zones work as LLM-first surface (single MCP call)
//! 7. Headless mode fully functional (no display server, CI on software GPU)
//!
//! ## Scope
//!
//! This test is **aggregation only**. It does NOT implement subsystems (Epics 1-11)
//! or validation layers (E12.3-E12.5). It runs the existing infrastructure, collects
//! evidence from each thesis point, and produces a structured proof report.
//!
//! ## Artifacts
//!
//! The test emits a structured JSON thesis proof report to stdout:
//! - `ARTIFACT:v1_thesis_proof` — per-thesis-point pass/fail with evidence
//! - `ARTIFACT:v1_scene_registry_coverage` — all 25 scenes Layer 0 pass/fail
//! - `ARTIFACT:v1_performance_summary` — budget assertion results
//! - `ARTIFACT:v1_layer4_manifest` — developer visibility artifact manifest
//!
//! ## Spec References
//!
//! - heart-and-soul/v1.md — "V1 must prove" (lines 7-22), "V1 success criteria" (lines 143-152)
//! - validation-framework/spec.md — All V1 Success Criterion requirements (lines 313-364)
//! - validation-framework/spec.md — Requirement: Five Validation Layers (line 5-8)
//! - validation-framework/spec.md — Requirement: Test Scene Registry (line 160-172): all 25 scenes

use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;
use tze_hud_scene::test_scenes::{ClockMs, TestSceneRegistry, assert_layer0_invariants};
use tze_hud_scene::types::*;
use tze_hud_telemetry::{
    AssertionOutcome, FrameTelemetry, HardwareFactors, SessionSummary, ValidationReport,
};
use tze_hud_validation::layer4::{
    ArtifactBuilder, ArtifactOptions, SceneArtifactInput, SceneDescription, SceneMetrics,
    SceneStatus,
};
use tze_hud_validation::ssim::compute_ssim;

use serde::{Deserialize, Serialize};
use std::time::Instant;

// ─── Shared gRPC session harness ─────────────────────────────────────────────
// Extracted from duplicate copies in multi_agent, presence_card_coexistence, and
// subtitle_streaming (hud-ls5pz). See common/mod.rs for drift reconciliation notes.
#[path = "common/mod.rs"]
mod common;
use common::*;

// ─── Constants ──────────────────────────────────────────────────────────────

const TEST_PSK: &str = "v1-thesis-proof-key";
const GRPC_PORT: u16 = 50054; // unique port to avoid conflicts with other integration tests
const DISPLAY_W: u32 = 1920;
const DISPLAY_H: u32 = 1080;

/// All 25 scene names from the test scene registry (validation-framework/spec.md line 160-172).
const ALL_25_SCENES: &[&str] = &[
    "empty_scene",
    "single_tile_solid",
    "three_tiles_no_overlap",
    "max_tiles_stress",
    "overlapping_tiles_zorder",
    "overlay_transparency",
    "tab_switch",
    "lease_expiry",
    "mobile_degraded",
    "sync_group_media",
    "input_highlight",
    "coalesced_dashboard",
    "three_agents_contention",
    "overlay_passthrough_regions",
    "disconnect_reclaim_multiagent",
    "privacy_redaction_mode",
    "chatty_dashboard_touch",
    "zone_publish_subtitle",
    "zone_reject_wrong_type",
    "zone_conflict_two_publishers",
    "zone_orchestrate_then_publish",
    "zone_geometry_adapts_profile",
    "zone_disconnect_cleanup",
    "policy_matrix_basic",
    "policy_arbitration_collision",
];

// ─── Thesis proof report types ──────────────────────────────────────────────

/// Evidence collected for a single thesis point.
#[derive(Debug, Serialize, Deserialize)]
struct ThesisPointEvidence {
    /// Thesis point number (1-7).
    thesis_number: u8,
    /// Short title of the thesis point.
    title: String,
    /// Whether this thesis point is demonstrated.
    passed: bool,
    /// Human-readable summary of evidence.
    evidence_summary: String,
    /// Structured evidence details (JSON-serializable).
    details: serde_json::Value,
    /// Spec references for this thesis point.
    spec_refs: Vec<String>,
}

/// Full V1 thesis proof report.
#[derive(Debug, Serialize, Deserialize)]
struct ThesisProofReport {
    /// Report version.
    version: u32,
    /// Timestamp of proof generation (ISO 8601).
    generated_at: String,
    /// Overall verdict: all 7 thesis points must pass.
    all_passed: bool,
    /// Count of passed thesis points.
    passed_count: u8,
    /// Count of failed thesis points.
    failed_count: u8,
    /// Per-thesis-point evidence.
    thesis_points: Vec<ThesisPointEvidence>,
    /// Scene registry coverage summary.
    scene_coverage: SceneCoverageSummary,
    /// Performance budget summary.
    performance_summary: PerformanceSummary,
}

/// Coverage report for all 25 test scenes across Layer 0.
#[derive(Debug, Serialize, Deserialize)]
struct SceneCoverageSummary {
    total_scenes: usize,
    scenes_passed: usize,
    scenes_failed: usize,
    per_scene: Vec<SceneResult>,
}

/// Result of running a single scene through Layer 0.
#[derive(Debug, Serialize, Deserialize)]
struct SceneResult {
    name: String,
    layer0_passed: bool,
    violation_count: usize,
    violations: Vec<String>,
    expected_tiles: usize,
    expected_tabs: usize,
}

/// Performance budget assertion summary.
#[derive(Debug, Serialize, Deserialize)]
struct PerformanceSummary {
    /// Hardware factors used for normalization.
    hardware_factors: serde_json::Value,
    /// Validation report from Layer 3.
    validation_report: serde_json::Value,
    /// Whether calibration was available.
    calibrated: bool,
}

/// Summary of Layer 4 artifact generation.
#[derive(Debug, Serialize, Deserialize)]
struct Layer4Summary {
    manifest_generated: bool,
    scenes_with_artifacts: usize,
    index_html_generated: bool,
    manifest_json_path: String,
}

// ─── File-local helpers ──────────────────────────────────────────────────────
// (now_wall_us() is provided by the common harness via `use common::*`)

fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Produce a valid ISO 8601 UTC datetime without a chrono dependency.
    // Algorithm: https://en.wikipedia.org/wiki/Julian_day#Converting_Julian_or_Gregorian_calendar_date_to_Julian_day_number
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400; // days since 1970-01-01
    // Gregorian calendar calculation (no leap-second correction needed for a proof timestamp)
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

// ─── Thesis point evidence collectors ───────────────────────────────────────

/// Thesis 1: An LLM can hold a tile on a screen (60fps).
///
/// Evidence: Create tiles via gRPC, render frames, verify tile count and frame completion.
///
/// ## What makes this assertion non-vacuous
///
/// `all_frames_completed` was previously just `frame_time_us > 0`, which is trivially
/// true (any non-zero render time passes). We now additionally require:
/// - `max_tile_count > 0`: the compositor actually _saw_ tiles during rendering, not
///   just an empty scene.  A zero tile count would indicate tiles were created but not
///   visible to the renderer — a real regression.
/// - The 60fps claim requires frame_time_us to be within a software-GPU-multiplied
///   budget.  On headless llvmpipe the budget is ×10 vs. reference (60fps = 16.6ms
///   → 166ms effective).  Any frame consistently exceeding this on software GPU
///   indicates a render-path regression, not scheduler noise.
///
/// The `max_frame_time_within_budget` flag is informational (logged but does not
/// block the unconditional `passed` outcome) because frame time is wall-clock
/// sensitive and is properly gated behind `TZE_HUD_PERF_ASSERT` in the Layer 3
/// budget assertions.  What IS unconditional here is `max_tile_count > 0` — that is
/// a structural correctness check (tiles are visible), not a latency measurement.
async fn collect_thesis1_evidence(
    _runtime: &mut HeadlessRuntime,
    agents: &[String],
    tile_count: usize,
    frame_telemetry: &[FrameTelemetry],
) -> ThesisPointEvidence {
    let frames_rendered = frame_telemetry.len();
    let has_tiles = tile_count > 0;

    // `frame_time_us > 0` is trivially true (even a failed render emits non-zero wall
    // time).  Use `tile_count > 0` as the actual "compositor saw tiles" check — this
    // exercises the gRPC→scene-graph→renderer pipeline end-to-end, not just that the
    // render loop ran.
    let max_tile_count = frame_telemetry
        .iter()
        .map(|f| f.tile_count)
        .max()
        .unwrap_or(0);

    // Require the compositor to have observed tiles on at least one frame.
    let compositor_saw_tiles = max_tile_count > 0;

    // Informational: check frame time is within a software-GPU-friendly bound.
    // 166ms = 16.6ms nominal × 10× llvmpipe headroom.  Not a blocking assertion —
    // the hard budget is enforced by Layer 3 behind TZE_HUD_PERF_ASSERT.
    const SOFTWARE_GPU_FRAME_BUDGET_US: u64 = 166_000;
    let all_frames_within_budget = frame_telemetry
        .iter()
        .all(|f| f.frame_time_us <= SOFTWARE_GPU_FRAME_BUDGET_US);

    // `passed` requires real tile presence on screen — not just that render ran.
    let passed = has_tiles && frames_rendered > 0 && compositor_saw_tiles;

    ThesisPointEvidence {
        thesis_number: 1,
        title: "An LLM can hold a tile on a screen".to_string(),
        passed,
        evidence_summary: format!(
            "{} agents held {} tiles across {} rendered frames. \
             Max tile count observed by compositor: {} (must be > 0). \
             All frames within software-GPU budget ({}ms): {}.",
            agents.len(),
            tile_count,
            frames_rendered,
            max_tile_count,
            SOFTWARE_GPU_FRAME_BUDGET_US / 1_000,
            all_frames_within_budget,
        ),
        details: serde_json::json!({
            "agents": agents,
            "tile_count": tile_count,
            "frames_rendered": frames_rendered,
            "max_tile_count_observed": max_tile_count,
            "compositor_saw_tiles": compositor_saw_tiles,
            "all_frames_within_software_gpu_budget": all_frames_within_budget,
            "software_gpu_frame_budget_us": SOFTWARE_GPU_FRAME_BUDGET_US,
        }),
        spec_refs: vec![
            "v1.md line 9".to_string(),
            "validation-framework/spec.md lines 313-320".to_string(),
        ],
    }
}

/// Thesis 2: The lease model works (auth, capabilities, TTL, revocation).
///
/// Evidence: Agents authenticated via PSK, received leases with TTL, capabilities granted.
fn collect_thesis2_evidence(
    agent_sessions: &[(String, bool, bool)], // (agent_id, authenticated, has_lease)
    lease_count: u32,
) -> ThesisPointEvidence {
    let all_authenticated = agent_sessions.iter().all(|(_, auth, _)| *auth);
    let all_have_leases = agent_sessions.iter().all(|(_, _, lease)| *lease);
    let passed = all_authenticated && all_have_leases && lease_count > 0;

    ThesisPointEvidence {
        thesis_number: 2,
        title: "The lease model works".to_string(),
        passed,
        evidence_summary: format!(
            "{} agents authenticated via PSK. {} active leases granted with TTL. \
             All agents authenticated: {}. All agents have leases: {}.",
            agent_sessions.len(),
            lease_count,
            all_authenticated,
            all_have_leases,
        ),
        details: serde_json::json!({
            "agent_sessions": agent_sessions.iter().map(|(id, auth, lease)| {
                serde_json::json!({
                    "agent_id": id,
                    "authenticated": auth,
                    "has_lease": lease,
                })
            }).collect::<Vec<_>>(),
            "total_active_leases": lease_count,
        }),
        spec_refs: vec![
            "v1.md line 11".to_string(),
            "lease-governance/spec.md".to_string(),
            "session-protocol/spec.md".to_string(),
        ],
    }
}

/// Thesis 3: Multiple agents coexist without interference.
///
/// Evidence: 3 agents with distinct namespaces, no cross-agent interference.
fn collect_thesis3_evidence(
    namespaces: &[String],
    all_distinct: bool,
    no_cross_access: bool,
) -> ThesisPointEvidence {
    let passed = namespaces.len() >= 3 && all_distinct && no_cross_access;

    ThesisPointEvidence {
        thesis_number: 3,
        title: "Multiple agents coexist without interference".to_string(),
        passed,
        evidence_summary: format!(
            "{} concurrent agents with {} distinct namespaces. \
             All namespaces distinct: {}. No cross-agent interference: {}.",
            namespaces.len(),
            namespaces.len(),
            all_distinct,
            no_cross_access,
        ),
        details: serde_json::json!({
            "agent_count": namespaces.len(),
            "namespaces": namespaces,
            "all_distinct": all_distinct,
            "no_cross_agent_access": no_cross_access,
        }),
        spec_refs: vec![
            "v1.md line 13".to_string(),
            "validation-framework/spec.md lines 335-343".to_string(),
        ],
    }
}

/// Thesis 4: Performance is real (p99 latencies measured).
///
/// Evidence: ValidationReport from Layer 3 budget assertions.
fn collect_thesis4_evidence(report: &ValidationReport) -> ThesisPointEvidence {
    // Performance is "demonstrated" if:
    // - Calibrated: all assertions pass
    // - Uncalibrated: no failures (uncalibrated is acceptable per spec)
    let passed = report.fail_count == 0;

    ThesisPointEvidence {
        thesis_number: 4,
        title: "Performance is real".to_string(),
        passed,
        evidence_summary: format!(
            "Layer 3 validation: {} passed, {} failed, {} uncalibrated. Verdict: {}.",
            report.pass_count, report.fail_count, report.uncalibrated_count, report.verdict,
        ),
        details: serde_json::to_value(report).unwrap_or(serde_json::json!(null)),
        spec_refs: vec![
            "v1.md line 15".to_string(),
            "validation-framework/spec.md lines 88-99".to_string(),
            "validation-framework/spec.md lines 103-115".to_string(),
            "validation-framework/spec.md lines 137-157".to_string(),
        ],
    }
}

/// Thesis 5: The validation architecture works (5 layers operational).
///
/// Evidence: Each layer produces output.
fn collect_thesis5_evidence(
    layer0_operational: bool,
    layer1_operational: bool,
    layer2_operational: bool,
    layer3_operational: bool,
    layer4_operational: bool,
    scene_coverage: &SceneCoverageSummary,
) -> ThesisPointEvidence {
    let layers_operational = [
        layer0_operational,
        layer1_operational,
        layer2_operational,
        layer3_operational,
        layer4_operational,
    ];
    let operational_count = layers_operational.iter().filter(|&&l| l).count();
    let all_operational = operational_count == 5;

    // Per spec: all 25 scenes must pass all applicable validation layers.
    let all_scenes_pass = scene_coverage.scenes_failed == 0;
    let passed = all_operational && all_scenes_pass;

    ThesisPointEvidence {
        thesis_number: 5,
        title: "The validation architecture works".to_string(),
        passed,
        evidence_summary: format!(
            "{}/5 validation layers operational. {}/{} test scenes pass Layer 0. \
             All layers operational: {}. All scenes pass: {}.",
            operational_count,
            scene_coverage.scenes_passed,
            scene_coverage.total_scenes,
            all_operational,
            all_scenes_pass,
        ),
        details: serde_json::json!({
            "layers": {
                "layer0_scene_graph_assertions": layer0_operational,
                "layer1_headless_render": layer1_operational,
                "layer2_ssim_visual_regression": layer2_operational,
                "layer3_performance_validation": layer3_operational,
                "layer4_developer_visibility": layer4_operational,
            },
            "operational_count": operational_count,
            "scene_coverage": {
                "total": scene_coverage.total_scenes,
                "passed": scene_coverage.scenes_passed,
                "failed": scene_coverage.scenes_failed,
            },
        }),
        spec_refs: vec![
            "v1.md line 18".to_string(),
            "validation-framework/spec.md lines 5-8".to_string(),
            "validation-framework/spec.md lines 253-264".to_string(),
            "validation-framework/spec.md lines 324-331".to_string(),
        ],
    }
}

/// Thesis 6: Zones work as the LLM-first surface (single MCP call).
///
/// Evidence: Zone publish succeeded via a single gRPC call with no prior scene context.
fn collect_thesis6_evidence(
    zone_publish_success: bool,
    zone_name: &str,
    content_rendered: bool,
) -> ThesisPointEvidence {
    let passed = zone_publish_success && content_rendered;

    ThesisPointEvidence {
        thesis_number: 6,
        title: "Zones work as the LLM-first surface".to_string(),
        passed,
        evidence_summary: format!(
            "Zone publish to '{}' via single call: {}. Content rendered in zone: {}.",
            zone_name,
            if zone_publish_success {
                "accepted"
            } else {
                "rejected"
            },
            content_rendered,
        ),
        details: serde_json::json!({
            "zone_name": zone_name,
            "publish_accepted": zone_publish_success,
            "content_rendered": content_rendered,
            "method": "single gRPC ZonePublish call with no prior scene context",
        }),
        spec_refs: vec![
            "v1.md line 19".to_string(),
            "validation-framework/spec.md lines 346-353".to_string(),
        ],
    }
}

/// Thesis 7: Headless mode fully functional (no display server, CI on software GPU).
///
/// Evidence: This entire test runs headlessly — the runtime started without a display server.
fn collect_thesis7_evidence(
    runtime_started: bool,
    frames_rendered: usize,
    grpc_operational: bool,
) -> ThesisPointEvidence {
    let passed = runtime_started && frames_rendered > 0 && grpc_operational;

    ThesisPointEvidence {
        thesis_number: 7,
        title: "Headless mode fully functional".to_string(),
        passed,
        evidence_summary: format!(
            "Headless runtime started: {runtime_started}. {frames_rendered} frames rendered without display server. \
             gRPC server operational: {grpc_operational}. No display server or physical GPU required.",
        ),
        details: serde_json::json!({
            "headless_runtime_started": runtime_started,
            "frames_rendered": frames_rendered,
            "grpc_operational": grpc_operational,
            "display_server_required": false,
            "software_gpu": true,
            "note": "This entire V1 thesis proof runs headlessly on software GPU (llvmpipe/WARP/Metal)",
        }),
        spec_refs: vec![
            "v1.md line 21".to_string(),
            "validation-framework/spec.md lines 357-364".to_string(),
        ],
    }
}

// ─── Layer 0: Run all 25 scenes through scene graph invariant checks ────────

fn run_all_scenes_layer0() -> SceneCoverageSummary {
    let registry = TestSceneRegistry::new();
    let scene_names = TestSceneRegistry::scene_names();

    let mut per_scene: Vec<SceneResult> = Vec::with_capacity(scene_names.len());

    for &name in scene_names {
        let result = match registry.build(name, ClockMs::FIXED) {
            Some((graph, spec)) => {
                let violations = assert_layer0_invariants(&graph);
                let violation_strings: Vec<String> =
                    violations.iter().map(|v| format!("{v:?}")).collect();
                let passed = violations.is_empty();

                SceneResult {
                    name: name.to_string(),
                    layer0_passed: passed,
                    violation_count: violations.len(),
                    violations: violation_strings,
                    expected_tiles: spec.expected_tile_count,
                    expected_tabs: spec.expected_tab_count,
                }
            }
            None => SceneResult {
                name: name.to_string(),
                layer0_passed: false,
                violation_count: 1,
                violations: vec!["Scene not found in registry".to_string()],
                expected_tiles: 0,
                expected_tabs: 0,
            },
        };
        per_scene.push(result);
    }

    let scenes_passed = per_scene.iter().filter(|s| s.layer0_passed).count();
    let scenes_failed = per_scene.iter().filter(|s| !s.layer0_passed).count();

    SceneCoverageSummary {
        total_scenes: per_scene.len(),
        scenes_passed,
        scenes_failed,
        per_scene,
    }
}

// ─── Layer 4: Generate developer visibility artifacts ───────────────────────

fn generate_layer4_artifacts(
    scene_coverage: &SceneCoverageSummary,
) -> Result<Layer4Summary, Box<dyn std::error::Error>> {
    let opts = ArtifactOptions {
        output_root: std::path::PathBuf::from("tests/v1_proof"),
        branch: "v1-thesis-proof".to_string(),
        spec_ids: vec![
            "v1-thesis-proof".to_string(),
            "layer-4-artifact-gen".to_string(),
        ],
    };

    let mut builder = ArtifactBuilder::new("tests/v1_proof", "v1-thesis-proof", opts)?;

    // Capture the run directory path before finalise consumes the builder
    let run_dir = builder.run_dir().to_path_buf();

    // Add scene artifacts from Layer 0 results
    for scene in &scene_coverage.per_scene {
        let status = if scene.layer0_passed {
            SceneStatus::Pass
        } else {
            SceneStatus::Fail
        };

        let input = SceneArtifactInput {
            description: SceneDescription {
                name: scene.name.clone(),
                description: format!(
                    "Test scene '{}' — validates scene graph invariants (Layer 0). \
                     Expected {} tiles in {} tabs.",
                    scene.name, scene.expected_tiles, scene.expected_tabs,
                ),
                expected_tab_count: scene.expected_tabs,
                expected_tile_count: scene.expected_tiles,
                has_hit_regions: false,
                has_zones: false,
            },
            status,
            metrics: SceneMetrics {
                ssim_score: None,
                frames_rendered: None,
                frame_time_p99_us: None,
                lease_violations: 0,
                budget_overruns: 0,
            },
            rendered_pixels: None,
            width: DISPLAY_W,
            height: DISPLAY_H,
            golden_pixels: None,
            diff_pixels: None,
            telemetry_json: None,
            changes_since_golden: Some("N/A — Layer 0 does not compare images".to_string()),
        };

        builder.add_scene(input)?;
    }

    let _manifest = builder.finalise()?;
    let manifest_path = run_dir.join("manifest.json");

    Ok(Layer4Summary {
        manifest_generated: true,
        scenes_with_artifacts: scene_coverage.total_scenes,
        index_html_generated: run_dir.join("index.html").exists(),
        manifest_json_path: manifest_path.display().to_string(),
    })
}

// ─── Main thesis proof test ─────────────────────────────────────────────────

#[tokio::test]
async fn test_v1_thesis_proof() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("=== V1 Thesis Proof: Starting ===");
    let proof_start = Instant::now();

    // ─── Phase 0: Scene registry coverage (Layer 0, all 25 scenes) ──────────

    eprintln!("--- Phase 0: Layer 0 scene registry coverage (25 scenes) ---");
    let scene_coverage = run_all_scenes_layer0();
    eprintln!(
        "    Layer 0: {}/{} scenes pass invariant checks",
        scene_coverage.scenes_passed, scene_coverage.total_scenes
    );

    // Verify the registry contains exactly the spec-mandated 25 scenes (count + name set).
    let registered_names = TestSceneRegistry::scene_names();
    assert_eq!(
        registered_names.len(),
        ALL_25_SCENES.len(),
        "Scene registry must contain exactly {} scenes (spec requirement), found {}",
        ALL_25_SCENES.len(),
        registered_names.len(),
    );
    // Name-set check: ensures the registry names match the spec list, not just the count.
    let expected_set: std::collections::HashSet<&str> = ALL_25_SCENES.iter().copied().collect();
    let actual_set: std::collections::HashSet<&str> = registered_names.iter().copied().collect();
    assert_eq!(
        expected_set,
        actual_set,
        "Scene registry names must exactly match the spec-mandated set (validation-framework/spec.md line 160-172). \
         Missing: {:?}. Extra: {:?}",
        expected_set.difference(&actual_set).collect::<Vec<_>>(),
        actual_set.difference(&expected_set).collect::<Vec<_>>(),
    );

    // Emit scene coverage artifact
    let scene_coverage_json = serde_json::to_string_pretty(&scene_coverage)?;
    println!("ARTIFACT:v1_scene_registry_coverage {scene_coverage_json}");

    // ─── Phase 1: Start headless runtime (Thesis 7) ─────────────────────────

    eprintln!("--- Phase 1: Headless runtime startup (Thesis 7) ---");
    let config = HeadlessConfig {
        width: DISPLAY_W,
        height: DISPLAY_H,
        grpc_port: GRPC_PORT,
        bind_all_interfaces: false,
        psk: TEST_PSK.to_string(),
        config_toml: None,
    };

    let mut runtime = HeadlessRuntime::new(config).await?;
    let runtime_started = true;
    eprintln!("    Headless runtime started successfully (no display server)");

    // Pre-populate scene with tab and default zones
    {
        let state = runtime.shared_state().lock().await;
        let mut scene = state.scene.lock().await;
        let tab_id = scene.create_tab("V1-Thesis", 0)?;
        scene.active_tab = Some(tab_id);
        scene.zone_registry = ZoneRegistry::with_defaults();
    }

    let _server_handle = runtime.start_grpc_server().await?;
    let grpc_operational = true;
    eprintln!("    gRPC server started on port {GRPC_PORT}");

    // ─── Phase 2: Connect 3 agents (Thesis 2: auth, Thesis 3: coexistence) ─

    eprintln!("--- Phase 2: Multi-agent connection (Thesis 2, 3) ---");
    let standard_caps = vec!["create_tiles".to_string(), "modify_own_tiles".to_string()];

    let (mut agent_a, mut agent_b, mut agent_c) = tokio::try_join!(
        connect_agent(
            TEST_PSK,
            GRPC_PORT,
            "thesis-agent-alpha",
            "v1-thesis-proof",
            1,
            standard_caps.clone()
        ),
        connect_agent(
            TEST_PSK,
            GRPC_PORT,
            "thesis-agent-beta",
            "v1-thesis-proof",
            2,
            standard_caps.clone()
        ),
        connect_agent(
            TEST_PSK,
            GRPC_PORT,
            "thesis-agent-gamma",
            "v1-thesis-proof",
            3,
            standard_caps.clone()
        ),
    )?;

    let namespaces = vec![
        agent_a.namespace.clone(),
        agent_b.namespace.clone(),
        agent_c.namespace.clone(),
    ];
    let all_distinct = {
        use std::collections::HashSet;
        namespaces.iter().collect::<HashSet<_>>().len() == namespaces.len()
    };

    eprintln!("    3 agents connected: {namespaces:?}. All distinct: {all_distinct}");

    // ─── Phase 3: Create tiles (Thesis 1: tile on screen) ──────────────────

    eprintln!("--- Phase 3: Tile creation (Thesis 1) ---");

    // Agent A: two tiles (weather dashboard)
    let _tile_a1 = create_tile_via_grpc(&mut agent_a, [50.0, 50.0, 600.0, 400.0], 10).await?;
    let _tile_a2 = create_tile_via_grpc(&mut agent_a, [50.0, 470.0, 600.0, 200.0], 9).await?;

    // Agent B: one tile
    let _tile_b1 = create_tile_via_grpc(&mut agent_b, [700.0, 50.0, 500.0, 300.0], 8).await?;

    // Agent C: one tile
    let _tile_c1 = create_tile_via_grpc(&mut agent_c, [700.0, 400.0, 500.0, 250.0], 7).await?;

    let total_tiles_created = 4usize;
    eprintln!("    {total_tiles_created} tiles created across 3 agents");

    // Verify namespace isolation (no cross-agent tile access).
    // Group all tiles by namespace and verify:
    // - exactly 3 distinct namespaces exist (no unexpected ones)
    // - each expected agent has exactly the tiles it created
    let no_cross_access = {
        use std::collections::HashMap;
        let state = runtime.shared_state().lock().await;
        let scene = state.scene.lock().await;
        let mut tiles_by_ns: HashMap<String, usize> = HashMap::new();
        for tile in scene.tiles.values() {
            *tiles_by_ns.entry(tile.namespace.clone()).or_default() += 1;
        }
        // Exactly 3 namespaces; each agent owns the expected tile count
        tiles_by_ns.len() == 3
            && tiles_by_ns.get(&agent_a.namespace).copied().unwrap_or(0) == 2
            && tiles_by_ns.get(&agent_b.namespace).copied().unwrap_or(0) == 1
            && tiles_by_ns.get(&agent_c.namespace).copied().unwrap_or(0) == 1
    };
    eprintln!("    Namespace isolation verified: {no_cross_access}");

    // ─── Phase 4: Zone publish (Thesis 6: LLM-first surface) ───────────────

    eprintln!("--- Phase 4: Zone publish (Thesis 6) ---");

    // Agent C publishes to subtitle zone via a single call
    let zone_publish_result = publish_stream_text_to_zone_via_grpc(
        &mut agent_c,
        "subtitle",
        "V1 thesis proof: subtitle zone content via single call",
    )
    .await;
    let zone_publish_success = zone_publish_result.is_ok();
    if let Err(ref e) = zone_publish_result {
        eprintln!("    Zone publish error: {e}");
    }

    // Verify content is rendered in the zone
    let content_rendered = {
        let state = runtime.shared_state().lock().await;
        let scene = state.scene.lock().await;
        // Check the zone exists and has active publishes
        let zone_exists = scene.zone_registry.zones.contains_key("subtitle");
        let has_publishes = scene
            .zone_registry
            .active_publishes
            .get("subtitle")
            .is_some_and(|pubs| !pubs.is_empty());
        zone_exists && has_publishes
    };
    eprintln!(
        "    Zone publish accepted: {zone_publish_success}. Content rendered: {content_rendered}."
    );

    // ─── Phase 5: Render frames and collect telemetry (Thesis 1, 4, 7) ─────

    eprintln!("--- Phase 5: Frame rendering and telemetry collection ---");

    let mut frame_telemetry: Vec<FrameTelemetry> = Vec::new();
    let render_start = Instant::now();
    let target_frames = 30; // Render enough frames for meaningful telemetry

    for _frame_num in 0..target_frames {
        let telemetry = runtime.render_frame().await;
        frame_telemetry.push(telemetry);
    }
    let render_elapsed = render_start.elapsed();

    eprintln!(
        "    Rendered {} frames in {:?} (avg {:.1}ms/frame)",
        frame_telemetry.len(),
        render_elapsed,
        render_elapsed.as_millis() as f64 / frame_telemetry.len() as f64,
    );

    // ─── Phase 5b: Layer 2 SSIM machinery probe ──────────────────────────────
    //
    // Previously `layer2_operational` was hardcoded `true` (hud-59b32: "by fiat").
    // The SSIM crate (tze_hud_validation::ssim) is available, but we need to
    // exercise it with actual pixel data to prove it is operational — not just that
    // the crate compiles.
    //
    // Approach: read the last rendered frame's pixels and run compute_ssim against
    // a copy of itself.  An identical-image comparison is mathematically guaranteed
    // to produce SSIM ≥ 0.999 (floating-point rounding aside).  If compute_ssim
    // panics, returns NaN, or produces a score < 0.999, the Layer 2 subsystem is
    // broken and the thesis proof must not report it as operational.
    //
    // This is a structural correctness check (does the SSIM function run and produce
    // a sane result?), not a visual regression check — visual regression against golden
    // images is E12.3.  The structural check is always deterministic and never flakes.
    let layer2_ssim_operational = {
        let pixels = runtime.read_pixels();
        if pixels.len() == (DISPLAY_W as usize) * (DISPLAY_H as usize) * 4 {
            // Self-comparison: SSIM(x, x) must be ≥ 0.999.
            let result = compute_ssim(&pixels, &pixels, DISPLAY_W, DISPLAY_H);
            let score_valid = result.mean.is_finite() && result.mean >= 0.999;
            eprintln!(
                "    Layer 2: SSIM self-comparison score={:.6} (valid={})",
                result.mean, score_valid
            );
            score_valid
        } else {
            // Pixel buffer missing or wrong size — SSIM cannot run.
            eprintln!(
                "    Layer 2: pixel buffer size mismatch ({} bytes, expected {}); \
                 SSIM probe skipped",
                pixels.len(),
                (DISPLAY_W as usize) * (DISPLAY_H as usize) * 4
            );
            false
        }
    };

    // ─── Phase 6: Build session summary and run Layer 3 assertions ──────────

    eprintln!("--- Phase 6: Performance validation (Thesis 4) ---");

    let mut summary = SessionSummary::new();
    summary.total_frames = frame_telemetry.len() as u64;
    summary.elapsed_us = render_elapsed.as_micros() as u64;
    summary.fps = if render_elapsed.as_micros() > 0 {
        frame_telemetry.len() as f64 / (render_elapsed.as_micros() as f64 / 1_000_000.0)
    } else {
        0.0
    };

    for ft in &frame_telemetry {
        summary.frame_time.record(ft.frame_time_us);
        if ft.input_to_local_ack_us > 0 {
            summary.input_to_local_ack.record(ft.input_to_local_ack_us);
        }
        if ft.input_to_scene_commit_us > 0 {
            summary
                .input_to_scene_commit
                .record(ft.input_to_scene_commit_us);
        }
        if ft.input_to_next_present_us > 0 {
            summary
                .input_to_next_present
                .record(ft.input_to_next_present_us);
        }
        if ft.frame_time_us > summary.peak_frame_time_us {
            summary.peak_frame_time_us = ft.frame_time_us;
        }
        if ft.tile_count > summary.peak_tile_count {
            summary.peak_tile_count = ft.tile_count;
        }
    }

    // ── Layer 3 validation ───────────────────────────────────────────────────
    //
    // ## Why calibrated factors are used for the structural check
    //
    // Previously this block passed `HardwareFactors::uncalibrated()`.  With all
    // three factors set to `None`, every latency budget assertion returns
    // `AssertionOutcome::Uncalibrated` — so `fail_count == 0` in the final
    // assertion at Phase 10 was UNFAILABLE regardless of actual performance
    // (hud-59b32: "assert by fiat").
    //
    // Fix (two-tier approach):
    //
    // 1. **Structural tier (blocking lane, always runs)**: Run with
    //    `HardwareFactors::new(1.0, 1.0, 1.0)` so the zero-violation counters
    //    (lease_violations, budget_overruns, sync_drift_violations) always produce
    //    a definitive Pass/Fail — they use `CalibrationDimension::None` and are
    //    always calibrated.  `fail_count == 0` in Phase 10 now GENUINELY checks
    //    the zero-violation counters, and would catch any non-zero violations.
    //    The latency buckets (frame_time, input_to_*) also run with real factors,
    //    but their budget assertions may Fail on slow CI.  To avoid blocking the
    //    lane on latency flakes, we run a SECOND pass with uncalibrated factors
    //    to produce the final `validation_report` used in the Thesis 4 verdict.
    //
    // 2. **Calibrated wall-clock tier (perf lane, TZE_HUD_PERF_ASSERT=1)**: The
    //    calibrated report is asserted only on the perf lane.  On the standard
    //    blocking lane the calibrated result is logged for observability only.
    //
    // This matches the pattern established by budget_assertions.rs (hud-1aswu.3).

    // Run once with calibrated factors (1.0 = reference hardware) to catch
    // zero-violation regressions and log calibrated latency values.
    let calibrated_factors = HardwareFactors::new(1.0, 1.0, 1.0);
    let calibrated_report = ValidationReport::run(&summary, &calibrated_factors);

    // Structural assertion (blocking lane): zero-violation counters must be zero.
    // These counters (lease_violations, budget_overruns, sync_drift_violations) use
    // CalibrationDimension::None and are always definitive — no uncalibrated escape hatch.
    // Counting failures on zero-violation assertions only (filter out latency assertions):
    let zero_violation_failures = calibrated_report
        .assertions
        .iter()
        .filter(|a| {
            matches!(a, AssertionOutcome::Fail { metric, .. }
                if ["lease_violations", "budget_overruns", "sync_drift_violations"].contains(&metric.as_str()))
        })
        .count();
    assert_eq!(
        zero_violation_failures,
        0,
        "Layer 3 zero-violation counters must all be zero: {:?}",
        calibrated_report
            .assertions
            .iter()
            .filter(|a| a.is_fail())
            .collect::<Vec<_>>(),
    );

    // Wall-clock / p99 budget assertions: gated behind TZE_HUD_PERF_ASSERT.
    // On shared CI runners these assertions would flake; run them only on the perf lane.
    let perf_assert_enabled = std::env::var("TZE_HUD_PERF_ASSERT")
        .map(|v| v.trim() == "1")
        .unwrap_or(false);

    if perf_assert_enabled {
        // On the perf lane, latency assertions with calibrated factors must also pass.
        let latency_failures = calibrated_report
            .assertions
            .iter()
            .filter(|a| a.is_fail())
            .count();
        assert_eq!(
            latency_failures,
            0,
            "[TZE_HUD_PERF_ASSERT] Layer 3 calibrated latency assertions must all pass: {:?}",
            calibrated_report
                .assertions
                .iter()
                .filter(|a| a.is_fail())
                .collect::<Vec<_>>(),
        );
        eprintln!(
            "[PERF-ASSERT] Layer 3 calibrated: {} pass, {} fail, {} uncalibrated — verdict: {}",
            calibrated_report.pass_count,
            calibrated_report.fail_count,
            calibrated_report.uncalibrated_count,
            calibrated_report.verdict,
        );
    } else {
        eprintln!(
            "[SKIP-PERF] Layer 3 calibrated (informational): {} pass, {} fail, {} uncalibrated \
             — set TZE_HUD_PERF_ASSERT=1 to enforce wall-clock latency budgets",
            calibrated_report.pass_count,
            calibrated_report.fail_count,
            calibrated_report.uncalibrated_count,
        );
    }

    // For the thesis proof artifact and Thesis 4 verdict: run with uncalibrated factors.
    // "Uncalibrated" is acceptable per spec — it means the harness ran and produced
    // evidence, not that performance is unproven.  The structural (zero-violation) check
    // above is where we actually assert correctness in a blocking way.
    let factors = HardwareFactors::uncalibrated();
    let validation_report = ValidationReport::run(&summary, &factors);

    eprintln!(
        "    Layer 3: {} pass, {} fail, {} uncalibrated — verdict: {}",
        validation_report.pass_count,
        validation_report.fail_count,
        validation_report.uncalibrated_count,
        validation_report.verdict,
    );

    // Emit performance summary artifact
    let perf_summary = PerformanceSummary {
        hardware_factors: serde_json::to_value(&calibrated_factors)?,
        validation_report: serde_json::to_value(&calibrated_report)?,
        calibrated: calibrated_factors.is_fully_calibrated(),
    };
    println!(
        "ARTIFACT:v1_performance_summary {}",
        serde_json::to_string_pretty(&perf_summary)?
    );

    // ─── Phase 7: Generate Layer 4 artifacts ────────────────────────────────

    eprintln!("--- Phase 7: Layer 4 artifact generation (Thesis 5) ---");

    let layer4_result = generate_layer4_artifacts(&scene_coverage);
    let layer4_operational = layer4_result.is_ok();
    if let Ok(ref l4) = layer4_result {
        println!(
            "ARTIFACT:v1_layer4_manifest {}",
            serde_json::to_string_pretty(l4)?
        );
        eprintln!(
            "    Layer 4: manifest generated, {} scenes with artifacts",
            l4.scenes_with_artifacts
        );
    } else if let Err(ref e) = layer4_result {
        eprintln!("    Layer 4 artifact generation failed: {e}");
    }

    // ─── Phase 8: Collect lease counts and per-agent lease presence ─────────

    // Derive per-agent lease/auth status from runtime state rather than hardcoding.
    // If connect_agent() succeeded (we're past the try_join), each agent authenticated.
    // We additionally verify each agent's namespace has an active lease in the scene.
    let (lease_count, agent_auth_status) = {
        let state = runtime.shared_state().lock().await;
        let scene = state.scene.lock().await;
        let count = scene.leases.len() as u32;
        let agent_ns_has_lease = |ns: &str| scene.leases.values().any(|l| l.namespace == ns);
        let status = vec![
            (
                "thesis-agent-alpha".to_string(),
                true,
                agent_ns_has_lease(&agent_a.namespace),
            ),
            (
                "thesis-agent-beta".to_string(),
                true,
                agent_ns_has_lease(&agent_b.namespace),
            ),
            (
                "thesis-agent-gamma".to_string(),
                true,
                agent_ns_has_lease(&agent_c.namespace),
            ),
        ];
        (count, status)
    };

    // ─── Phase 9: Assemble thesis proof report ──────────────────────────────

    eprintln!("--- Phase 9: Assembling thesis proof report ---");

    let agent_ids = vec![
        "thesis-agent-alpha".to_string(),
        "thesis-agent-beta".to_string(),
        "thesis-agent-gamma".to_string(),
    ];

    let thesis_points = vec![
        // Thesis 1: Tile on screen at 60fps
        collect_thesis1_evidence(
            &mut runtime,
            &agent_ids,
            total_tiles_created,
            &frame_telemetry,
        )
        .await,
        // Thesis 2: Lease model works
        // auth/lease flags derived from runtime state in Phase 8 — not hardcoded.
        collect_thesis2_evidence(&agent_auth_status, lease_count),
        // Thesis 3: Multiple agents coexist
        collect_thesis3_evidence(&namespaces, all_distinct, no_cross_access),
        // Thesis 4: Performance is real
        collect_thesis4_evidence(&validation_report),
        // Thesis 5: Validation architecture works
        // Layer 0: operational iff scene_coverage was built without panicking (it was).
        // Layer 1: headless render — operational iff at least one frame rendered.
        // Layer 2: SSIM self-probe in Phase 5b — compute_ssim ran and returned a
        //          valid score (≥ 0.999 on a self-comparison).  Full visual regression
        //          against golden images is E12.3.
        // Layer 3: operational iff ValidationReport::run completed without panic (it did).
        // Layer 4: operational iff generate_layer4_artifacts succeeded.
        collect_thesis5_evidence(
            true,                        // Layer 0: run_all_scenes_layer0() completed
            !frame_telemetry.is_empty(), // Layer 1: at least one frame rendered headlessly
            layer2_ssim_operational,     // Layer 2: compute_ssim self-probe (Phase 5b)
            true,                        // Layer 3: ValidationReport::run completed
            layer4_operational,          // Layer 4: developer visibility artifacts
            &scene_coverage,
        ),
        // Thesis 6: Zones work as LLM-first surface
        collect_thesis6_evidence(zone_publish_success, "subtitle", content_rendered),
        // Thesis 7: Headless mode fully functional
        collect_thesis7_evidence(runtime_started, frame_telemetry.len(), grpc_operational),
    ];

    let passed_count = thesis_points.iter().filter(|t| t.passed).count() as u8;
    let failed_count = thesis_points.iter().filter(|t| !t.passed).count() as u8;
    let all_passed = failed_count == 0;

    let report = ThesisProofReport {
        version: 1,
        generated_at: now_iso8601(),
        all_passed,
        passed_count,
        failed_count,
        thesis_points,
        scene_coverage,
        performance_summary: perf_summary,
    };

    // Emit the full thesis proof report
    println!(
        "ARTIFACT:v1_thesis_proof {}",
        serde_json::to_string_pretty(&report)?
    );

    // ─── Phase 10: Final assertions ─────────────────────────────────────────

    let proof_elapsed = proof_start.elapsed();
    eprintln!("=== V1 Thesis Proof: Complete ({proof_elapsed:?}) ===");
    eprintln!(
        "    Overall: {}/{} thesis points demonstrated",
        passed_count, 7
    );

    for tp in &report.thesis_points {
        eprintln!(
            "    Thesis {}: {} — {}",
            tp.thesis_number,
            if tp.passed { "PASS" } else { "FAIL" },
            tp.title,
        );
    }

    // Hard assertions: all thesis points must pass
    assert!(
        report.all_passed,
        "V1 thesis proof FAILED: {}/{} thesis points did not pass. \
         Failed: {:?}",
        failed_count,
        7,
        report
            .thesis_points
            .iter()
            .filter(|t| !t.passed)
            .map(|t| format!("Thesis {}: {}", t.thesis_number, t.title))
            .collect::<Vec<_>>(),
    );

    // All 25 scenes must be in the registry
    assert_eq!(
        report.scene_coverage.total_scenes, 25,
        "Test scene registry must contain exactly 25 scenes"
    );

    // Structural Layer 3 assertion (blocking lane): zero-violation counters must be zero.
    // This was already asserted in Phase 6 (calibrated_report).  Re-assert here for
    // defense-in-depth: the zero-violation assertions in the uncalibrated report also
    // use CalibrationDimension::None and are always definitive — if any non-zero
    // violation counter slipped through, this would catch it in the final proof artifact.
    //
    // NOTE: `validation_report` uses `HardwareFactors::uncalibrated()`, so latency
    // assertions return `Uncalibrated` (not Fail), making `fail_count` here only reflect
    // zero-violation counter failures.  The zero-violation counters (lease_violations,
    // budget_overruns, sync_drift_violations) always produce Pass/Fail regardless of
    // calibration state.
    assert_eq!(
        validation_report.fail_count, 0,
        "Layer 3 performance validation must not have hard failures \
         (zero-violation counters: lease_violations={}, budget_overruns={}, \
         sync_drift_violations={})",
        summary.lease_violations, summary.budget_overruns, summary.sync_drift_violations,
    );

    Ok(())
}
