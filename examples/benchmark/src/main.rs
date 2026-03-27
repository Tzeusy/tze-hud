//! # tze_hud Layer-3 Performance Validation Benchmark
//!
//! Runs the hardware-normalized calibration harness and performance scenarios,
//! then validates timing against normalized budgets.
//!
//! ## Usage
//!
//! ```sh
//! cargo run --bin benchmark --features headless -- --emit telemetry.json
//! cargo run --bin benchmark --features headless -- --emit telemetry.json --frames 300
//! cargo run --bin benchmark --features headless -- --emit telemetry.json --cpu-only
//! ```
//!
//! ## Calibration workloads
//!
//! 1. **CPU** — scene-graph mutations via `tze_hud_scene::calibration::calibrate()`.
//! 2. **GPU** — fill/composition: render a fixed multi-tile scene at target
//!    resolution, measure frames per second over a warmup + timed window.
//! 3. **Upload** — texture upload: create and update texture-backed tiles
//!    in rapid succession, measure throughput.
//!
//! ## Output (--emit path)
//!
//! Emits a JSON object to the specified path containing:
//! - `calibration`: hardware factors for all three dimensions
//! - `sessions`: array of per-scenario session summaries
//! - `validation`: `ValidationReport` with per-metric assertion outcomes
//!
//! ## Without --features headless
//!
//! Without the `headless` feature this binary prints a usage message and exits.
//! Only unit tests (serialization round-trips) are compiled unconditionally.

// Shared types and serialization structures — always compiled.
use serde::{Deserialize, Serialize};

use tze_hud_scene::calibration::CalibrationResult;
use tze_hud_telemetry::{HardwareFactors, SessionSummary, ValidationReport};

// ─── Shared output types (always compiled — needed for tests) ─────────────────

/// GPU fill/composition calibration result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GpuCalibrationResult {
    /// GPU frames per second achieved during calibration.
    pub fps: f64,
    /// Hardware factor: reference_fps / observed_fps (>1 = slower than reference).
    pub gpu_factor: f64,
    /// Duration of the calibration run in microseconds.
    pub calibration_duration_us: u64,
}

/// Texture-upload calibration result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadCalibrationResult {
    /// Tile operations per second achieved during upload calibration.
    pub tile_ops_per_sec: f64,
    /// Hardware factor: reference_ops / observed_ops.
    pub upload_factor: f64,
    /// Duration of the calibration run in microseconds.
    pub calibration_duration_us: u64,
}

/// Result of one benchmark scenario.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub name: String,
    pub summary: SessionSummary,
}

/// Serializable calibration summary.
#[derive(Debug, Serialize, Deserialize)]
pub struct CalibrationOutput {
    /// CPU scene-graph calibration result.
    pub cpu: CalibrationResult,
    /// GPU fill/composition calibration result.  None if --cpu-only.
    pub gpu: Option<GpuCalibrationResult>,
    /// Upload calibration result.  None if --cpu-only.
    pub upload: Option<UploadCalibrationResult>,
    /// Normalized hardware factors used for budget assertions.
    pub factors: HardwareFactors,
}

/// Full benchmark output emitted to `--emit path`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkOutput {
    /// Hardware calibration results.
    pub calibration: CalibrationOutput,
    /// Per-scenario session summaries.
    pub sessions: Vec<ScenarioResult>,
    /// Layer-3 validation report (runs against the steady-state session).
    pub validation: ValidationReport,
}

// ─── Unit tests (always compiled) ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_output_serializes_round_trip() {
        let cpu = CalibrationResult {
            speed_factor: 1.0,
            scene_ops_per_sec: 550_000.0,
            hash_throughput_mbps: 800.0,
            gpu_fill_factor: None,
            texture_upload_factor: None,
            timestamp: 1_700_000_000,
            calibration_duration_us: 100_000,
        };

        let factors = HardwareFactors::new(1.0, 1.0, 1.0);
        let summary = SessionSummary::new();
        let validation = ValidationReport::run(&summary, &factors);

        let output = BenchmarkOutput {
            calibration: CalibrationOutput {
                cpu,
                gpu: None,
                upload: None,
                factors: factors.clone(),
            },
            sessions: vec![ScenarioResult {
                name: "test".to_string(),
                summary,
            }],
            validation,
        };

        let json = serde_json::to_string_pretty(&output).unwrap();
        assert!(json.contains("calibration"));
        assert!(json.contains("sessions"));
        assert!(json.contains("validation"));

        // Round-trip
        let deserialized: BenchmarkOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sessions.len(), 1);
        assert_eq!(deserialized.sessions[0].name, "test");
    }

    #[test]
    fn test_gpu_calibration_result_serializes() {
        let r = GpuCalibrationResult {
            fps: 350.0,
            gpu_factor: 1.43,
            calibration_duration_us: 500_000,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("fps"));
        assert!(json.contains("gpu_factor"));
    }

    #[test]
    fn test_upload_calibration_result_serializes() {
        let r = UploadCalibrationResult {
            tile_ops_per_sec: 3_500.0,
            upload_factor: 1.43,
            calibration_duration_us: 200_000,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("tile_ops_per_sec"));
        assert!(json.contains("upload_factor"));
    }

    #[test]
    fn test_scenario_result_contains_session_summary() {
        let mut summary = SessionSummary::new();
        // record_frame increments total_frames internally
        for _ in 0..120 {
            summary.record_frame(12_000, 10);
        }
        summary.elapsed_us = 2_000_000;
        summary.finalize();

        let result = ScenarioResult {
            name: "steady_state_render".to_string(),
            summary,
        };
        assert_eq!(result.name, "steady_state_render");
        assert_eq!(result.summary.total_frames, 120);
        assert!((result.summary.fps - 60.0).abs() < 0.001);
    }
}

// ─── Headless implementation ──────────────────────────────────────────────────
// Everything below requires the `headless` feature.

#[cfg(feature = "headless")]
mod headless_impl {
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;
    use tracing::{info, warn};

    use tze_hud_runtime::headless::{HeadlessConfig, HeadlessRuntime};
    use tze_hud_scene::calibration::calibrate as calibrate_cpu;
    use tze_hud_scene::mutation::{MutationBatch, SceneMutation};
    use tze_hud_scene::types::{Capability, Node, NodeData, Rect, Rgba, SceneId, SolidColorNode};

    // ── Budget constants ─────────────────────────────────────────────────────

    /// Reference GPU throughput: frames per second on reference hardware
    /// (modern dGPU, simple multi-tile scene, 1920x1080).
    const REFERENCE_GPU_FPS: f64 = 500.0;

    /// Number of calibration frames to render for GPU calibration.
    const GPU_CALIBRATION_FRAMES: u64 = 50;

    /// Tile count for GPU calibration scene (multi-tile with alpha blending).
    const GPU_CALIBRATION_TILES: usize = 20;

    /// Reference upload throughput: tile create+update+delete ops/sec on reference hardware.
    const REFERENCE_UPLOAD_OPS_PER_SEC: f64 = 5_000.0;

    /// Number of upload calibration cycles.
    const UPLOAD_CALIBRATION_CYCLES: usize = 100;

    /// Tiles per upload cycle.
    const UPLOAD_TILES_PER_CYCLE: usize = 10;

    // ── CLI args ─────────────────────────────────────────────────────────────

    pub struct Args {
        /// Path to emit the telemetry JSON to.
        pub emit: Option<PathBuf>,
        /// Number of benchmark frames to render per scenario.
        pub frames: u64,
        /// Skip GPU and upload calibration (CPU-only mode for faster iteration).
        pub cpu_only: bool,
    }

    pub fn parse_args() -> Args {
        let args: Vec<String> = std::env::args().collect();
        let mut emit = None;
        let mut frames = 120u64;
        let mut cpu_only = false;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--emit" => {
                    i += 1;
                    if let Some(path_str) = args.get(i) {
                        emit = Some(PathBuf::from(path_str));
                    } else {
                        eprintln!("Error: --emit requires a path argument");
                        eprintln!("Usage: --emit <path>");
                        std::process::exit(1);
                    }
                }
                "--frames" => {
                    i += 1;
                    if let Some(n_str) = args.get(i) {
                        match n_str.parse::<u64>() {
                            Ok(n) => frames = n,
                            Err(_) => {
                                eprintln!("Error: --frames requires a positive integer, got '{}'", n_str);
                                eprintln!("Usage: --frames <number>");
                                std::process::exit(1);
                            }
                        }
                    } else {
                        eprintln!("Error: --frames requires a numeric argument");
                        eprintln!("Usage: --frames <number>");
                        std::process::exit(1);
                    }
                }
                "--cpu-only" => {
                    cpu_only = true;
                }
                _ => {}
            }
            i += 1;
        }

        Args { emit, frames, cpu_only }
    }

    // ── GPU calibration ───────────────────────────────────────────────────────

    /// Run the GPU fill/composition calibration workload.
    ///
    /// Renders a fixed multi-tile scene with overlapping alpha-blended tiles at
    /// target resolution (1920x1080). Returns FPS and the hardware factor.
    pub async fn calibrate_gpu() -> GpuCalibrationResult {
        let start = Instant::now();

        let config = HeadlessConfig {
            width: 1920,
            height: 1080,
            grpc_port: 0,
            psk: "calibration".to_string(),
        };

        let mut runtime = match HeadlessRuntime::new(config).await {
            Ok(r) => r,
            Err(e) => {
                warn!("GPU calibration failed to init runtime: {}", e);
                return GpuCalibrationResult {
                    fps: 0.0,
                    gpu_factor: f64::NAN,
                    calibration_duration_us: start.elapsed().as_micros() as u64,
                };
            }
        };

        // Set up a multi-tile scene with overlapping alpha-blended tiles
        {
            let mut state = runtime.state.lock().await;
            let scene = &mut state.scene;
            let tab_id = match scene.create_tab("gpu_calibration", 0) {
                Ok(id) => id,
                Err(e) => {
                    warn!("GPU calibration: create_tab failed: {:?}", e);
                    return GpuCalibrationResult {
                        fps: 0.0,
                        gpu_factor: f64::NAN,
                        calibration_duration_us: start.elapsed().as_micros() as u64,
                    };
                }
            };

            let lease_id = scene.grant_lease(
                "gpu_calibration",
                300_000,
                vec![Capability::CreateTile, Capability::CreateNode, Capability::UpdateTile],
            );
            if let Some(lease) = scene.leases.get_mut(&lease_id) {
                lease.resource_budget.max_tiles = (GPU_CALIBRATION_TILES + 5) as u32;
            }

            let cols = 5usize;
            for i in 0..GPU_CALIBRATION_TILES {
                let col = i % cols;
                let row = i / cols;
                // Tiles overlap slightly for alpha-blending composition load
                let bounds = Rect::new(
                    col as f32 * 350.0,
                    row as f32 * 250.0,
                    380.0,
                    270.0,
                );
                if let Ok(tile_id) = scene.create_tile(
                    tab_id,
                    "gpu_calibration",
                    lease_id,
                    bounds,
                    (i + 1) as u32,
                ) {
                    let alpha = if i % 2 == 0 { 1.0f32 } else { 0.7f32 };
                    let node = Node {
                        id: SceneId::new(),
                        children: vec![],
                        data: NodeData::SolidColor(SolidColorNode {
                            color: Rgba::new(
                                i as f32 / GPU_CALIBRATION_TILES as f32,
                                0.5,
                                0.8,
                                alpha,
                            ),
                            bounds: Rect::new(0.0, 0.0, 380.0, 270.0),
                        }),
                    };
                    let _ = scene.set_tile_root(tile_id, node);
                }
            }
        }

        // Warmup: 5 frames (not measured)
        for _ in 0..5 {
            runtime.render_frame().await;
        }

        // Timed window: GPU_CALIBRATION_FRAMES frames
        let timed_start = Instant::now();
        for _ in 0..GPU_CALIBRATION_FRAMES {
            runtime.render_frame().await;
        }
        let timed_elapsed = timed_start.elapsed();

        let elapsed_secs = timed_elapsed.as_secs_f64();
        let fps = GPU_CALIBRATION_FRAMES as f64 / elapsed_secs.max(1e-9);
        let gpu_factor = (REFERENCE_GPU_FPS / fps).clamp(0.01, 100.0);

        GpuCalibrationResult {
            fps,
            gpu_factor,
            calibration_duration_us: start.elapsed().as_micros() as u64,
        }
    }

    // ── Upload calibration ────────────────────────────────────────────────────

    /// Run the texture-upload calibration workload.
    ///
    /// Creates and destroys tiles in rapid succession to measure the create/update/delete
    /// throughput, which is the primary bottleneck for texture-upload-heavy workloads.
    pub async fn calibrate_upload() -> UploadCalibrationResult {
        let start = Instant::now();

        let config = HeadlessConfig {
            width: 1920,
            height: 1080,
            grpc_port: 0,
            psk: "calibration".to_string(),
        };

        let mut runtime = match HeadlessRuntime::new(config).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Upload calibration failed to init runtime: {}", e);
                return UploadCalibrationResult {
                    tile_ops_per_sec: 0.0,
                    upload_factor: f64::NAN,
                    calibration_duration_us: start.elapsed().as_micros() as u64,
                };
            }
        };

        let tab_id;
        let lease_id;
        {
            let mut state = runtime.state.lock().await;
            let scene = &mut state.scene;
            tab_id = match scene.create_tab("upload_calibration", 0) {
                Ok(id) => id,
                Err(e) => {
                    warn!("Upload calibration: create_tab failed: {:?}", e);
                    return UploadCalibrationResult {
                        tile_ops_per_sec: 0.0,
                        upload_factor: f64::NAN,
                        calibration_duration_us: start.elapsed().as_micros() as u64,
                    };
                }
            };
            lease_id = scene.grant_lease(
                "upload_calibration",
                300_000,
                vec![Capability::CreateTile, Capability::CreateNode, Capability::UpdateTile],
            );
            if let Some(lease) = scene.leases.get_mut(&lease_id) {
                lease.resource_budget.max_tiles = (UPLOAD_TILES_PER_CYCLE + 5) as u32;
            }
        }

        // Timed upload cycle: create N tiles, render, delete N tiles, repeat
        let timed_start = Instant::now();
        let mut total_ops: u64 = 0;

        for cycle in 0..UPLOAD_CALIBRATION_CYCLES {
            let mut tile_ids = Vec::with_capacity(UPLOAD_TILES_PER_CYCLE);

            // Create UPLOAD_TILES_PER_CYCLE tiles
            {
                let mut state = runtime.state.lock().await;
                let scene = &mut state.scene;
                for i in 0..UPLOAD_TILES_PER_CYCLE {
                    let bounds = Rect::new(
                        (i as f32 % 10.0) * 190.0,
                        (i as f32 / 10.0).floor() * 180.0,
                        180.0,
                        170.0,
                    );
                    if let Ok(tile_id) = scene.create_tile(
                        tab_id,
                        "upload_calibration",
                        lease_id,
                        bounds,
                        (i + 1) as u32,
                    ) {
                        let node = Node {
                            id: SceneId::new(),
                            children: vec![],
                            data: NodeData::SolidColor(SolidColorNode {
                                color: Rgba::new(
                                    cycle as f32 / UPLOAD_CALIBRATION_CYCLES as f32,
                                    0.5,
                                    0.5,
                                    1.0,
                                ),
                                bounds: Rect::new(0.0, 0.0, 180.0, 170.0),
                            }),
                        };
                        // Count create unconditionally (it succeeded); count
                        // set_root only if it also succeeds, to avoid inflating
                        // throughput when the scene mutation fails.
                        total_ops += 1; // create
                        if scene.set_tile_root(tile_id, node).is_ok() {
                            total_ops += 1; // set_root
                        }
                        tile_ids.push(tile_id);
                    }
                }
            }

            // Render one frame with the tiles visible
            runtime.render_frame().await;

            // Delete all tiles (models a full tile lifecycle per cycle)
            {
                let mut state = runtime.state.lock().await;
                let scene = &mut state.scene;
                for tile_id in &tile_ids {
                    let batch = MutationBatch {
                        batch_id: SceneId::new(),
                        agent_namespace: "upload_calibration".to_string(),
                        mutations: vec![SceneMutation::DeleteTile { tile_id: *tile_id }],
                        timing_hints: None,
                        lease_id: None,
                    };
                    let _ = scene.apply_batch(&batch);
                    total_ops += 1;
                }
            }
        }

        let timed_elapsed_secs = timed_start.elapsed().as_secs_f64();
        let tile_ops_per_sec = total_ops as f64 / timed_elapsed_secs.max(1e-9);
        let upload_factor =
            (REFERENCE_UPLOAD_OPS_PER_SEC / tile_ops_per_sec).clamp(0.01, 100.0);

        UploadCalibrationResult {
            tile_ops_per_sec,
            upload_factor,
            calibration_duration_us: start.elapsed().as_micros() as u64,
        }
    }

    // ── Benchmark scenarios ───────────────────────────────────────────────────

    /// Run the "steady-state render" scenario.
    ///
    /// Renders a multi-tile scene for `frame_count` frames and collects telemetry.
    pub async fn run_steady_state_render(frame_count: u64) -> ScenarioResult {
        info!("Running steady-state render scenario ({} frames)", frame_count);

        let config = HeadlessConfig {
            width: 1920,
            height: 1080,
            grpc_port: 0,
            psk: "benchmark".to_string(),
        };

        let mut runtime = HeadlessRuntime::new(config)
            .await
            .expect("HeadlessRuntime::new failed — ensure the headless feature is enabled");

        // Set up the benchmark scene: 10 tiles with solid colors
        {
            let mut state = runtime.state.lock().await;
            let scene = &mut state.scene;
            let tab_id = scene.create_tab("bench", 0).expect("create_tab");
            let lease_id = scene.grant_lease(
                "bench",
                300_000,
                vec![Capability::CreateTile, Capability::CreateNode, Capability::UpdateTile],
            );
            if let Some(lease) = scene.leases.get_mut(&lease_id) {
                lease.resource_budget.max_tiles = 15;
            }

            for i in 0..10usize {
                let col = i % 5;
                let row = i / 5;
                let bounds =
                    Rect::new(col as f32 * 384.0, row as f32 * 540.0, 380.0, 536.0);
                if let Ok(tile_id) =
                    scene.create_tile(tab_id, "bench", lease_id, bounds, (i + 1) as u32)
                {
                    let node = Node {
                        id: SceneId::new(),
                        children: vec![],
                        data: NodeData::SolidColor(SolidColorNode {
                            color: Rgba::new(
                                i as f32 / 10.0,
                                0.5,
                                1.0 - i as f32 / 10.0,
                                1.0,
                            ),
                            bounds: Rect::new(0.0, 0.0, 380.0, 536.0),
                        }),
                    };
                    let _ = scene.set_tile_root(tile_id, node);
                }
            }
        }

        // Warmup: 5 frames (not included in summary)
        for _ in 0..5 {
            runtime.render_frame().await;
        }

        // Timed window
        let session_start = Instant::now();
        let mut summary = SessionSummary::new();

        for _ in 0..frame_count {
            let telemetry = runtime.render_frame().await;
            summary.record_frame(telemetry.frame_time_us, telemetry.tile_count);

            // input_to_local_ack: Stage 1 + Stage 2 (input drain + local feedback)
            let local_ack =
                telemetry.stage1_input_drain_us + telemetry.stage2_local_feedback_us;
            if local_ack > 0 {
                summary.input_to_local_ack.record(local_ack);
            }
            // input_to_scene_commit: Stage 3 + Stage 4 (mutation intake + commit)
            let scene_commit =
                telemetry.stage3_mutation_intake_us + telemetry.stage4_scene_commit_us;
            if scene_commit > 0 {
                summary.input_to_scene_commit.record(scene_commit);
            }
            // input_to_next_present: full pipeline latency in headless mode
            summary.input_to_next_present.record(telemetry.frame_time_us);
        }

        summary.elapsed_us = session_start.elapsed().as_micros() as u64;
        summary.finalize();

        ScenarioResult {
            name: "steady_state_render".to_string(),
            summary,
        }
    }

    /// Run the "high-mutation" scenario: apply bounds mutations every frame.
    pub async fn run_high_mutation(frame_count: u64) -> ScenarioResult {
        info!("Running high-mutation scenario ({} frames)", frame_count);

        let config = HeadlessConfig {
            width: 1920,
            height: 1080,
            grpc_port: 0,
            psk: "benchmark".to_string(),
        };

        let mut runtime = HeadlessRuntime::new(config)
            .await
            .expect("HeadlessRuntime::new failed");

        let tab_id;
        let lease_id;
        let mut tile_ids = Vec::new();

        {
            let mut state = runtime.state.lock().await;
            let scene = &mut state.scene;
            tab_id = scene.create_tab("mutation_bench", 0).expect("create_tab");
            lease_id = scene.grant_lease(
                "mutation_bench",
                300_000,
                vec![Capability::CreateTile, Capability::CreateNode, Capability::UpdateTile],
            );
            if let Some(lease) = scene.leases.get_mut(&lease_id) {
                lease.resource_budget.max_tiles = 15;
            }

            for i in 0..10usize {
                let col = i % 5;
                let row = i / 5;
                let bounds =
                    Rect::new(col as f32 * 384.0, row as f32 * 540.0, 380.0, 536.0);
                if let Ok(tile_id) =
                    scene.create_tile(tab_id, "mutation_bench", lease_id, bounds, (i + 1) as u32)
                {
                    tile_ids.push(tile_id);
                }
            }
        }

        if tile_ids.is_empty() {
            warn!("run_high_mutation: no tiles were created; cannot run mutation scenario");
            return ScenarioResult {
                name: "high_mutation".to_string(),
                summary: SessionSummary::new(),
            };
        }

        // Warmup
        for _ in 0..5 {
            runtime.render_frame().await;
        }

        let session_start = Instant::now();
        let mut summary = SessionSummary::new();

        for frame_idx in 0..frame_count {
            // Apply bounds mutation to 3 tiles per frame
            {
                let mut state = runtime.state.lock().await;
                let scene = &mut state.scene;
                let mut mutations = Vec::new();
                for offset in 0..3usize {
                    let idx = ((frame_idx as usize) + offset) % tile_ids.len();
                    let jitter = (frame_idx as f32) * 0.1;
                    let col = idx % 5;
                    let row = idx / 5;
                    mutations.push(SceneMutation::UpdateTileBounds {
                        tile_id: tile_ids[idx],
                        bounds: Rect::new(
                            col as f32 * 384.0 + jitter.sin() * 5.0,
                            row as f32 * 540.0,
                            380.0,
                            536.0,
                        ),
                    });
                }
                let batch = MutationBatch {
                    batch_id: SceneId::new(),
                    agent_namespace: "mutation_bench".to_string(),
                    mutations,
                    timing_hints: None,
                    lease_id: None,
                };
                let _ = scene.apply_batch(&batch);
            }

            let telemetry = runtime.render_frame().await;
            summary.record_frame(telemetry.frame_time_us, telemetry.tile_count);

            let local_ack =
                telemetry.stage1_input_drain_us + telemetry.stage2_local_feedback_us;
            if local_ack > 0 {
                summary.input_to_local_ack.record(local_ack);
            }
            let scene_commit =
                telemetry.stage3_mutation_intake_us + telemetry.stage4_scene_commit_us;
            if scene_commit > 0 {
                summary.input_to_scene_commit.record(scene_commit);
            }
            summary.input_to_next_present.record(telemetry.frame_time_us);
        }

        summary.elapsed_us = session_start.elapsed().as_micros() as u64;
        summary.finalize();

        ScenarioResult {
            name: "high_mutation".to_string(),
            summary,
        }
    }

    // ── Entry point ───────────────────────────────────────────────────────────

    pub async fn run() {
        use tze_hud_telemetry::AssertionOutcome;

        tracing_subscriber::fmt()
            .with_env_filter(
                std::env::var("RUST_LOG")
                    .unwrap_or_else(|_| "benchmark=info,tze_hud_runtime=warn".to_string())
                    .as_str(),
            )
            .init();

        let args = parse_args();

        // ── Phase 1: Calibration ─────────────────────────────────────────────
        info!("=== tze_hud Layer-3 Benchmark ===");
        info!("Phase 1: Hardware calibration");

        info!("  Running CPU scene-graph calibration...");
        let cpu_result = calibrate_cpu();
        info!(
            "  CPU: speed_factor={:.2}, scene_ops/s={:.0}, hash_mbps={:.1}",
            cpu_result.speed_factor,
            cpu_result.scene_ops_per_sec,
            cpu_result.hash_throughput_mbps,
        );

        let (gpu_result, upload_result) = if args.cpu_only {
            warn!("  --cpu-only: skipping GPU and upload calibration");
            (None, None)
        } else {
            info!("  Running GPU fill/composition calibration...");
            let gpu = calibrate_gpu().await;
            if gpu.gpu_factor.is_nan() {
                warn!(
                    "  GPU calibration failed (no suitable adapter?); using uncalibrated mode"
                );
            } else {
                info!("  GPU: fps={:.1}, gpu_factor={:.2}", gpu.fps, gpu.gpu_factor);
            }

            info!("  Running texture-upload calibration...");
            let upload = calibrate_upload().await;
            if upload.upload_factor.is_nan() {
                warn!("  Upload calibration failed; using uncalibrated mode");
            } else {
                info!(
                    "  Upload: tile_ops/s={:.0}, upload_factor={:.2}",
                    upload.tile_ops_per_sec, upload.upload_factor,
                );
            }

            (Some(gpu), Some(upload))
        };

        // Build HardwareFactors (NaN → None for uncalibrated dimensions)
        let factors = HardwareFactors {
            cpu: Some(cpu_result.speed_factor),
            gpu: gpu_result.as_ref().and_then(|g| {
                if g.gpu_factor.is_nan() { None } else { Some(g.gpu_factor) }
            }),
            upload: upload_result.as_ref().and_then(|u| {
                if u.upload_factor.is_nan() { None } else { Some(u.upload_factor) }
            }),
        };

        info!(
            "  Hardware factors: cpu={:?}, gpu={:?}, upload={:?}",
            factors.cpu, factors.gpu, factors.upload,
        );

        // ── Phase 2: Benchmark scenarios ─────────────────────────────────────
        info!("Phase 2: Benchmark scenarios ({} frames each)", args.frames);

        let steady_state = run_steady_state_render(args.frames).await;
        info!(
            "  steady_state_render: total_frames={}, fps={:.1}, p99_frame_time={}µs, peak={}µs",
            steady_state.summary.total_frames,
            steady_state.summary.fps,
            steady_state.summary.frame_time.p99().unwrap_or(0),
            steady_state.summary.peak_frame_time_us,
        );

        let high_mutation = run_high_mutation(args.frames).await;
        info!(
            "  high_mutation: total_frames={}, fps={:.1}, p99_frame_time={}µs, peak={}µs",
            high_mutation.summary.total_frames,
            high_mutation.summary.fps,
            high_mutation.summary.frame_time.p99().unwrap_or(0),
            high_mutation.summary.peak_frame_time_us,
        );

        // ── Phase 3: Validation ───────────────────────────────────────────────
        info!("Phase 3: Layer-3 budget validation");

        let validation = ValidationReport::run(&steady_state.summary, &factors);
        info!("  Verdict: {}", validation.verdict);
        for assertion in &validation.assertions {
            match assertion {
                AssertionOutcome::Pass { metric, observed, budget, .. } => {
                    info!("  PASS  {}: {}µs ≤ {}µs", metric, observed, budget);
                }
                AssertionOutcome::Fail {
                    metric,
                    observed,
                    budget,
                    overage_pct,
                    ..
                } => {
                    warn!(
                        "  FAIL  {}: {}µs > {}µs ({:.1}% over budget)",
                        metric, observed, budget, overage_pct,
                    );
                }
                AssertionOutcome::Uncalibrated { metric, reason, raw_value } => {
                    warn!("  UNCAL {}: raw={}µs ({})", metric, raw_value, reason);
                }
                AssertionOutcome::NoSamples { metric } => {
                    warn!("  NOSAMPLES {}", metric);
                }
            }
        }

        // ── Emit output ───────────────────────────────────────────────────────
        let output = BenchmarkOutput {
            calibration: CalibrationOutput {
                cpu: cpu_result,
                gpu: gpu_result,
                upload: upload_result,
                factors,
            },
            sessions: vec![steady_state, high_mutation],
            validation,
        };

        let json = serde_json::to_string_pretty(&output)
            .expect("failed to serialize benchmark output");

        if let Some(path) = &args.emit {
            std::fs::write(path, &json).unwrap_or_else(|e| {
                eprintln!("Failed to write telemetry to {:?}: {}", path, e);
            });
            info!("Telemetry written to {:?}", path);
        } else {
            println!("{}", json);
        }

        // Exit with non-zero if any definitive failures
        if output.validation.fail_count > 0 {
            std::process::exit(1);
        }
    }
} // mod headless_impl

// ─── Entry points ─────────────────────────────────────────────────────────────

#[cfg(feature = "headless")]
#[tokio::main]
async fn main() {
    headless_impl::run().await;
}

#[cfg(not(feature = "headless"))]
fn main() {
    eprintln!(
        "The benchmark binary requires the `headless` feature.\n\
         Run with:\n\
         \n  cargo run --bin benchmark --features headless -- --emit telemetry.json\n"
    );
    std::process::exit(1);
}
