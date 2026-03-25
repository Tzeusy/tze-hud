//! Hardware-normalized calibration for performance budgets.
//!
//! Instead of raw multipliers (5x, 10x) for CI/software-rendering environments,
//! the calibration harness runs a fixed reference workload and measures actual
//! throughput. This produces a `speed_factor` that scales performance budgets
//! to the current hardware.
//!
//! # Doctrine alignment
//!
//! From `validation.md`: "Raw timing numbers are meaningless across machines."
//! This module implements the scene-graph CPU calibration dimension described
//! there: rapid scene mutation (create, delete, resize, reparent tiles) with
//! no rendering, measuring pure CPU scene-graph throughput.
//!
//! # Usage
//!
//! ```rust
//! use tze_hud_scene::calibration::{test_budget, budgets};
//!
//! // In a test assertion:
//! // assert!(elapsed_us < test_budget(budgets::INPUT_ACK_BUDGET_US));
//! ```

use crate::graph::SceneGraph;
use crate::mutation::{MutationBatch, SceneMutation};
use crate::types::{
    Capability, FontFamily, Node, NodeData, Rect, Rgba, SceneId, SolidColorNode, TextAlign,
    TextMarkdownNode, TextOverflow,
};

use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};
use std::time::Instant;

// ─── Budget constants ────────────────────────────────────────────────────────

/// Reference hardware performance budgets (in microseconds unless noted).
///
/// These are the target budgets on reference hardware (speed_factor = 1.0).
/// In tests, use [`test_budget`] to scale them for the current machine.
pub mod budgets {
    /// 60 Hz frame budget: 16.6ms.
    pub const FRAME_BUDGET_US: u64 = 16_600;
    /// Local input acknowledgement: < 4ms (no agent roundtrip).
    pub const INPUT_ACK_BUDGET_US: u64 = 4_000;
    /// Hit-test against the scene graph.
    pub const HIT_TEST_BUDGET_US: u64 = 100;
    /// Transaction validation per mutation batch.
    pub const TRANSACTION_VALIDATION_BUDGET_US: u64 = 200;
    /// Scene diff computation.
    pub const SCENE_DIFF_BUDGET_US: u64 = 500;
    /// Lease grant (including bookkeeping).
    pub const LEASE_GRANT_BUDGET_US: u64 = 1_000;
    /// Per-mutation budget enforcement check.
    pub const BUDGET_ENFORCEMENT_US: u64 = 50;
    /// Policy evaluation per request.
    pub const POLICY_EVALUATION_US: u64 = 200;
    /// Event classification.
    pub const EVENT_CLASSIFICATION_US: u64 = 5;
    /// Event delivery to agent.
    pub const EVENT_DELIVERY_US: u64 = 100;
    /// Event dispatch to agent (hit-test + session lookup + serialization + enqueue):
    /// < 2ms from Stage 2 completion (spec.md line 356-358 / RFC 0004 §8.2).
    pub const EVENT_DISPATCH_BUDGET_US: u64 = 2_000;
}

// ─── Calibration result ─────────────────────────────────────────────────────

/// Result of running the reference calibration workload.
///
/// All three calibration dimensions are tracked per the validation-framework spec
/// (lines 137-157): CPU scene-graph, GPU fill/composition, and texture upload.
///
/// GPU-derived fields (`gpu_fill_factor`, `texture_upload_factor`) are `Option<f64>`
/// because the scene crate has no GPU dependency.  Callers with access to a GPU
/// context (e.g., headless runtime tests) should populate these fields after running
/// the GPU calibration workloads and then store the result in the `CALIBRATION`
/// static via [`set_gpu_factors`].
///
/// Per spec line 156: when `gpu_fill_factor` or `texture_upload_factor` is `None`,
/// performance tests that depend on those dimensions MUST treat their result as
/// "uncalibrated" and emit a warning rather than a hard pass/fail.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalibrationResult {
    /// How many times slower this machine is than the reference for CPU scene-graph work.
    /// 1.0 = reference hardware, 2.0 = half speed, 0.5 = double speed.
    pub speed_factor: f64,
    /// Scene-graph operations per second achieved during calibration.
    pub scene_ops_per_sec: f64,
    /// SHA-256 throughput in MB/s (synthetic CPU load dimension).
    pub hash_throughput_mbps: f64,
    /// GPU fill/composition factor: ratio of reference GPU throughput to actual.
    ///
    /// 1.0 = reference GPU, >1.0 = slower (e.g., llvmpipe in CI ≈ 8–12×).
    /// `None` means GPU calibration has not been run.
    /// Populate via [`set_gpu_factors`] after running GPU calibration workloads.
    #[serde(default)]
    pub gpu_fill_factor: Option<f64>,
    /// Texture upload factor: ratio of reference upload throughput to actual.
    ///
    /// 1.0 = reference hardware, >1.0 = slower.
    /// `None` means texture upload calibration has not been run.
    /// Populate via [`set_gpu_factors`] after running GPU calibration workloads.
    #[serde(default)]
    pub texture_upload_factor: Option<f64>,
    /// Unix timestamp (seconds) when calibration was performed.
    pub timestamp: u64,
    /// Duration of the calibration run in microseconds.
    pub calibration_duration_us: u64,
}

// ─── Reference baseline ─────────────────────────────────────────────────────

/// Reference baseline: scene-graph ops/sec on target hardware.
///
/// This was measured on a reference machine (modern x86-64, ~4 GHz, no
/// software rendering overhead). The calibration workload produces this
/// many operations per second on that machine.
///
/// The workload: create a 50-tile scene, then apply 100 mutation batches
/// (each with 5 mutations: 2 UpdateTileBounds + 2 SetTileRoot + 1 DeleteTile/CreateTile).
/// Total: ~550 scene-graph operations.
const REFERENCE_SCENE_OPS_PER_SEC: f64 = 550_000.0;

/// Reference baseline: synthetic hash throughput in MB/s.
const REFERENCE_HASH_THROUGHPUT_MBPS: f64 = 800.0;

/// Number of tiles to create in the reference scene.
const CALIBRATION_TILES: usize = 50;

/// Number of mutation batches to apply.
const CALIBRATION_BATCHES: usize = 100;

/// Mutations per batch in the calibration workload.
const MUTATIONS_PER_BATCH: usize = 5;

/// Minimum speed factor (prevents unreasonably tight budgets).
const MIN_SPEED_FACTOR: f64 = 0.5;

/// Maximum speed factor (prevents unreasonably loose budgets on very slow machines).
const MAX_SPEED_FACTOR: f64 = 50.0;

/// Cache validity period in seconds (24 hours).
const CACHE_VALIDITY_SECS: u64 = 86_400;

// ─── Calibration ────────────────────────────────────────────────────────────

/// Run the reference calibration workload and compute hardware speed factor.
///
/// The workload exercises the same code paths that performance budgets protect:
/// tile creation, mutation batches, node tree replacement, bounds updates, and
/// tile deletion. It runs in < 500ms on most hardware.
pub fn calibrate() -> CalibrationResult {
    let overall_start = Instant::now();

    // ── Phase 1: Scene-graph CPU calibration ────────────────────────────
    let scene_ops = run_scene_workload();

    // ── Phase 2: Synthetic CPU load (hash throughput) ───────────────────
    let hash_mbps = run_hash_workload();

    // ── Compute speed factor ────────────────────────────────────────────
    // Weighted average: scene ops dominate (80%) since that's what the
    // budgets actually protect; hash throughput (20%) catches general CPU
    // speed differences that affect non-scene-graph paths.
    let scene_factor = REFERENCE_SCENE_OPS_PER_SEC / scene_ops.max(1.0);
    let hash_factor = REFERENCE_HASH_THROUGHPUT_MBPS / hash_mbps.max(0.01);
    let raw_factor = scene_factor * 0.8 + hash_factor * 0.2;
    let speed_factor = raw_factor.clamp(MIN_SPEED_FACTOR, MAX_SPEED_FACTOR);

    let calibration_duration_us = overall_start.elapsed().as_micros() as u64;

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    CalibrationResult {
        speed_factor,
        scene_ops_per_sec: scene_ops,
        hash_throughput_mbps: hash_mbps,
        // GPU fields are None until populated by a GPU-context caller via set_gpu_factors().
        gpu_fill_factor: None,
        texture_upload_factor: None,
        timestamp: now_secs,
        calibration_duration_us,
    }
}

/// Run the scene-graph mutation workload and return ops/sec.
fn run_scene_workload() -> f64 {
    let start = Instant::now();
    let mut total_ops: u64 = 0;

    // Create a scene with CALIBRATION_TILES tiles
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("calibration", 0).expect("create_tab");
    total_ops += 1;

    // Grant a lease with a high budget for the calibration workload
    let lease_id = scene.grant_lease(
        "calibration",
        300_000,
        vec![Capability::CreateTile, Capability::CreateNode, Capability::UpdateTile],
    );
    // Override the budget to allow many tiles
    if let Some(lease) = scene.leases.get_mut(&lease_id) {
        lease.resource_budget.max_tiles = (CALIBRATION_TILES + 10) as u32;
    }
    total_ops += 1;

    // Create tiles
    let mut tile_ids = Vec::with_capacity(CALIBRATION_TILES);
    let cols = 10u32;
    for i in 0..CALIBRATION_TILES {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let bounds = Rect::new(col as f32 * 190.0, row as f32 * 180.0, 180.0, 170.0);
        let tile_id = scene
            .create_tile(tab_id, "calibration", lease_id, bounds, (i + 1) as u32)
            .expect("create_tile during calibration");

        // Set a root node
        let node = if i % 2 == 0 {
            Node {
                id: SceneId::new(),
                children: vec![],
                data: NodeData::SolidColor(SolidColorNode {
                    color: Rgba::new(0.5, 0.5, 0.5, 1.0),
                    bounds: Rect::new(0.0, 0.0, 180.0, 170.0),
                }),
            }
        } else {
            Node {
                id: SceneId::new(),
                children: vec![],
                data: NodeData::TextMarkdown(TextMarkdownNode {
                    content: format!("calibration tile {i}"),
                    bounds: Rect::new(0.0, 0.0, 180.0, 170.0),
                    font_size_px: 14.0,
                    font_family: FontFamily::SystemMonospace,
                    color: Rgba::WHITE,
                    background: None,
                    alignment: TextAlign::Start,
                    overflow: TextOverflow::Clip,
                }),
            }
        };
        scene.set_tile_root(tile_id, node).expect("set_tile_root during calibration");
        tile_ids.push(tile_id);
        total_ops += 2; // create_tile + set_tile_root
    }

    // Apply CALIBRATION_BATCHES mutation batches
    for batch_idx in 0..CALIBRATION_BATCHES {
        let mut mutations = Vec::with_capacity(MUTATIONS_PER_BATCH);

        // Pick tiles to mutate (cycling through available tiles)
        let base = batch_idx % tile_ids.len();

        // Mutation 1-2: UpdateTileBounds on two tiles
        for offset in 0..2 {
            let idx = (base + offset) % tile_ids.len();
            let jitter = (batch_idx as f32) * 0.1;
            mutations.push(SceneMutation::UpdateTileBounds {
                tile_id: tile_ids[idx],
                bounds: Rect::new(
                    (idx as u32 % cols) as f32 * 190.0 + jitter,
                    (idx as u32 / cols) as f32 * 180.0,
                    180.0,
                    170.0,
                ),
            });
        }

        // Mutation 3-4: SetTileRoot on two tiles (replace node tree)
        for offset in 2..4 {
            let idx = (base + offset) % tile_ids.len();
            let node = Node {
                id: SceneId::new(),
                children: vec![],
                data: NodeData::SolidColor(SolidColorNode {
                    color: Rgba::new(
                        batch_idx as f32 / CALIBRATION_BATCHES as f32,
                        0.5,
                        0.5,
                        1.0,
                    ),
                    bounds: Rect::new(0.0, 0.0, 180.0, 170.0),
                }),
            };
            mutations.push(SceneMutation::SetTileRoot {
                tile_id: tile_ids[idx],
                node,
            });
        }

        // Mutation 5: Delete a tile and recreate it (exercises full lifecycle)
        let recycle_idx = (base + 4) % tile_ids.len();
        mutations.push(SceneMutation::DeleteTile {
            tile_id: tile_ids[recycle_idx],
        });

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "calibration".to_string(),
            mutations,
            timing_hints: None,
            lease_id: None,
        };

        let result = scene.apply_batch(&batch);
        total_ops += MUTATIONS_PER_BATCH as u64;

        // Recreate the deleted tile
        if result.applied {
            let col = (recycle_idx as u32) % cols;
            let row = (recycle_idx as u32) / cols;
            let bounds = Rect::new(col as f32 * 190.0, row as f32 * 180.0, 180.0, 170.0);
            if let Ok(new_tile_id) = scene.create_tile(
                tab_id,
                "calibration",
                lease_id,
                bounds,
                (recycle_idx + 1) as u32,
            ) {
                let node = Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        color: Rgba::new(0.3, 0.3, 0.3, 1.0),
                        bounds: Rect::new(0.0, 0.0, 180.0, 170.0),
                    }),
                };
                let _ = scene.set_tile_root(new_tile_id, node);
                tile_ids[recycle_idx] = new_tile_id;
                total_ops += 2;
            }
        }
    }

    // Also exercise hit_test — it's on the hot path
    for i in 0..100 {
        let x = (i as f32 * 19.2) % 1920.0;
        let y = (i as f32 * 10.8) % 1080.0;
        let _ = scene.hit_test(x, y);
        total_ops += 1;
    }

    let elapsed_secs = start.elapsed().as_secs_f64();
    total_ops as f64 / elapsed_secs.max(1e-9)
}

/// Run a synthetic CPU load workload and return throughput in MB/s.
///
/// Uses a simple iterative hash (FNV-1a inspired) to measure raw CPU
/// throughput without depending on external crates.
fn run_hash_workload() -> f64 {
    let start = Instant::now();
    let iterations = 100_000;
    let block_size = 64; // bytes per iteration
    let total_bytes = iterations * block_size;

    let mut state: u64 = 0xcbf29ce484222325; // FNV offset basis
    for i in 0..iterations {
        // FNV-1a style mixing
        for byte in 0..block_size {
            state ^= ((i * block_size + byte) & 0xFF) as u64;
            state = state.wrapping_mul(0x100000001b3); // FNV prime
        }
    }

    // Prevent the compiler from optimizing away the computation
    std::hint::black_box(state);

    let elapsed_secs = start.elapsed().as_secs_f64();
    let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
    total_mb / elapsed_secs.max(1e-9)
}

// ─── Budget scaling ─────────────────────────────────────────────────────────

/// Scale a reference budget by the calibration speed factor (CPU dimension).
///
/// On a machine twice as slow as the reference (speed_factor = 2.0),
/// a 100μs budget becomes 200μs. On a machine twice as fast
/// (speed_factor = 0.5), it becomes 50μs.
pub fn scaled_budget(base_budget_us: u64, calibration: &CalibrationResult) -> u64 {
    let scaled = base_budget_us as f64 * calibration.speed_factor;
    // Ensure at least 1μs to avoid zero budgets
    (scaled as u64).max(1)
}

/// Scale a reference budget by the GPU fill factor.
///
/// Returns `None` if `gpu_fill_factor` is not populated (uncalibrated), which
/// callers must treat as "uncalibrated" per the validation-framework spec —
/// they should emit a warning rather than a hard pass/fail assertion.
///
/// Returns `Some(scaled_us)` when the GPU factor is known.
pub fn gpu_scaled_budget(base_budget_us: u64, calibration: &CalibrationResult) -> Option<u64> {
    let factor = calibration.gpu_fill_factor?;
    let scaled = base_budget_us as f64 * factor;
    Some((scaled as u64).max(1))
}

/// Scale a reference budget by the texture upload factor.
///
/// Returns `None` when `texture_upload_factor` is not populated (uncalibrated).
pub fn texture_upload_scaled_budget(
    base_budget_us: u64,
    calibration: &CalibrationResult,
) -> Option<u64> {
    let factor = calibration.texture_upload_factor?;
    let scaled = base_budget_us as f64 * factor;
    Some((scaled as u64).max(1))
}

// ─── Test helper ────────────────────────────────────────────────────────────

/// Global calibration result, lazily initialized on first use.
static CALIBRATION: OnceLock<CalibrationResult> = OnceLock::new();

/// GPU calibration factors, separately mutable because GPU context is not
/// available during the initial `calibrate()` call (scene crate has no GPU dep).
///
/// Callers with GPU access (e.g., headless runtime tests) call [`set_gpu_factors`]
/// after running GPU calibration workloads.  Until set, both factors are `None`.
static GPU_FACTORS: RwLock<(Option<f64>, Option<f64>)> = RwLock::new((None, None));

/// Get a calibrated budget for use in test assertions.
///
/// On first call, runs the calibration workload (< 500ms) and caches the
/// result in memory for the duration of the test process. Subsequent calls
/// within the same process return instantly.
///
/// Also attempts to read/write a filesystem cache at
/// `$XDG_CACHE_HOME/tze_hud/calibration.json` (or `~/.cache/tze_hud/calibration.json`)
/// to avoid re-calibrating across test invocations within 24 hours.
///
/// # Example
///
/// ```rust,no_run
/// use tze_hud_scene::calibration::{test_budget, budgets};
///
/// let elapsed_us: u64 = 42; // from a timing measurement
/// assert!(
///     elapsed_us < test_budget(budgets::INPUT_ACK_BUDGET_US),
///     "local ack {}us exceeded calibrated budget {}us",
///     elapsed_us,
///     test_budget(budgets::INPUT_ACK_BUDGET_US),
/// );
/// ```
pub fn test_budget(base_us: u64) -> u64 {
    let cal = CALIBRATION.get_or_init(|| load_or_calibrate());
    scaled_budget(base_us, cal)
}

/// Get the current calibration result (runs calibration if needed).
pub fn current_calibration() -> &'static CalibrationResult {
    CALIBRATION.get_or_init(|| load_or_calibrate())
}

/// Get the current calibration result with GPU factors merged in.
///
/// Returns a clone of the static calibration result with `gpu_fill_factor` and
/// `texture_upload_factor` populated from the separately-stored GPU factors
/// (set via [`set_gpu_factors`]).  If GPU factors have not been set, both
/// fields remain `None` in the returned struct, which downstream callers
/// should treat as "uncalibrated" per the validation-framework spec.
pub fn current_calibration_with_gpu() -> CalibrationResult {
    let base = current_calibration().clone();
    let (gpu_fill, tex_upload) = GPU_FACTORS.read().unwrap_or_else(|e| e.into_inner()).clone();
    CalibrationResult {
        gpu_fill_factor: gpu_fill,
        texture_upload_factor: tex_upload,
        ..base
    }
}

/// Register GPU calibration factors measured by a GPU-context caller.
///
/// Call this once at the start of GPU-accelerated test suites, before any
/// budget assertions that use `gpu_fill_factor` or `texture_upload_factor`.
/// Calling it multiple times overwrites the previous values.
///
/// # Arguments
///
/// * `gpu_fill` — ratio of reference GPU fill throughput to actual;
///   1.0 = reference hardware, >1.0 = slower.
/// * `texture_upload` — ratio of reference texture upload throughput to actual;
///   1.0 = reference hardware, >1.0 = slower.
///
/// Both values are clamped to `[0.1, 200.0]` to prevent degenerate budgets.
pub fn set_gpu_factors(gpu_fill: f64, texture_upload: f64) {
    let gpu_fill = gpu_fill.clamp(0.1, 200.0);
    let texture_upload = texture_upload.clamp(0.1, 200.0);
    if let Ok(mut guard) = GPU_FACTORS.write() {
        *guard = (Some(gpu_fill), Some(texture_upload));
    }
}

// ─── Cache management ───────────────────────────────────────────────────────

/// Attempt to load calibration from cache, falling back to running it fresh.
fn load_or_calibrate() -> CalibrationResult {
    // Try loading from cache
    if let Some(cached) = load_cached_calibration() {
        return cached;
    }

    // Run fresh calibration
    let result = calibrate();

    // Try to cache it (best-effort)
    let _ = save_calibration_cache(&result);

    result
}

/// Return the cache file path, if determinable.
fn cache_path() -> Option<std::path::PathBuf> {
    let cache_dir = std::env::var("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            std::path::PathBuf::from(home).join(".cache")
        });

    Some(cache_dir.join("tze_hud").join("calibration.json"))
}

/// Load a cached calibration result if it exists and is still valid.
fn load_cached_calibration() -> Option<CalibrationResult> {
    let path = cache_path()?;

    let contents = std::fs::read_to_string(&path).ok()?;
    let result: CalibrationResult = serde_json::from_str(&contents).ok()?;

    // Check validity: must be within CACHE_VALIDITY_SECS
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if now_secs.saturating_sub(result.timestamp) > CACHE_VALIDITY_SECS {
        return None; // Stale cache
    }

    Some(result)
}

/// Save calibration result to the cache file (best-effort).
fn save_calibration_cache(result: &CalibrationResult) -> Result<(), Box<dyn std::error::Error>> {
    let path = cache_path().ok_or("could not determine cache path")?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&path, json)?;

    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calibration_runs_and_produces_valid_result() {
        let result = calibrate();

        // Speed factor must be within clamped range
        assert!(
            result.speed_factor >= MIN_SPEED_FACTOR,
            "speed_factor {} below minimum {}",
            result.speed_factor,
            MIN_SPEED_FACTOR,
        );
        assert!(
            result.speed_factor <= MAX_SPEED_FACTOR,
            "speed_factor {} above maximum {}",
            result.speed_factor,
            MAX_SPEED_FACTOR,
        );

        // Scene ops should be positive
        assert!(
            result.scene_ops_per_sec > 0.0,
            "scene_ops_per_sec should be positive, got {}",
            result.scene_ops_per_sec,
        );

        // Hash throughput should be positive
        assert!(
            result.hash_throughput_mbps > 0.0,
            "hash_throughput_mbps should be positive, got {}",
            result.hash_throughput_mbps,
        );

        // Timestamp should be recent (within last minute)
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        assert!(
            now_secs - result.timestamp < 60,
            "timestamp should be recent",
        );
    }

    #[test]
    fn test_calibration_completes_under_500ms() {
        let start = Instant::now();
        let _result = calibrate();
        let elapsed_ms = start.elapsed().as_millis();

        assert!(
            elapsed_ms < 500,
            "calibration took {}ms, budget is 500ms",
            elapsed_ms,
        );
    }

    #[test]
    fn test_scaled_budget_scales_linearly() {
        let base = CalibrationResult {
            speed_factor: 2.0,
            scene_ops_per_sec: 100_000.0,
            hash_throughput_mbps: 400.0,
            gpu_fill_factor: None,
            texture_upload_factor: None,
            timestamp: 0,
            calibration_duration_us: 0,
        };

        assert_eq!(scaled_budget(100, &base), 200);
        assert_eq!(scaled_budget(1000, &base), 2000);
    }

    #[test]
    fn test_scaled_budget_floor_at_one() {
        let fast = CalibrationResult {
            speed_factor: 0.5,
            scene_ops_per_sec: 1_000_000.0,
            hash_throughput_mbps: 1600.0,
            gpu_fill_factor: None,
            texture_upload_factor: None,
            timestamp: 0,
            calibration_duration_us: 0,
        };

        // Even very small base budgets floor at 1μs
        assert!(scaled_budget(1, &fast) >= 1);
    }

    #[test]
    fn test_gpu_scaled_budget_returns_none_when_uncalibrated() {
        let uncalibrated = CalibrationResult {
            speed_factor: 1.0,
            scene_ops_per_sec: 550_000.0,
            hash_throughput_mbps: 800.0,
            gpu_fill_factor: None,
            texture_upload_factor: None,
            timestamp: 0,
            calibration_duration_us: 0,
        };
        assert!(gpu_scaled_budget(16_600, &uncalibrated).is_none());
        assert!(texture_upload_scaled_budget(1_000, &uncalibrated).is_none());
    }

    #[test]
    fn test_gpu_scaled_budget_scales_when_calibrated() {
        let calibrated = CalibrationResult {
            speed_factor: 1.0,
            scene_ops_per_sec: 550_000.0,
            hash_throughput_mbps: 800.0,
            gpu_fill_factor: Some(10.0),     // 10× slower GPU (like llvmpipe)
            texture_upload_factor: Some(5.0), // 5× slower upload
            timestamp: 0,
            calibration_duration_us: 0,
        };
        // Frame budget 16.6ms × 10 = 166ms
        assert_eq!(gpu_scaled_budget(16_600, &calibrated), Some(166_000));
        // Texture upload budget 1ms × 5 = 5ms
        assert_eq!(texture_upload_scaled_budget(1_000, &calibrated), Some(5_000));
    }

    #[test]
    fn test_set_gpu_factors_roundtrips_via_current_calibration_with_gpu() {
        // Reset to None by setting to known values
        set_gpu_factors(8.0, 4.0);
        let cal = current_calibration_with_gpu();
        assert!(cal.gpu_fill_factor.is_some());
        assert!(cal.texture_upload_factor.is_some());
        let fill = cal.gpu_fill_factor.unwrap();
        let upload = cal.texture_upload_factor.unwrap();
        assert!((fill - 8.0).abs() < f64::EPSILON, "gpu_fill_factor mismatch: {fill}");
        assert!((upload - 4.0).abs() < f64::EPSILON, "texture_upload_factor mismatch: {upload}");
    }

    #[test]
    fn test_test_budget_returns_consistent_results() {
        // test_budget() is backed by OnceLock, so repeated calls must match
        let budget1 = test_budget(budgets::INPUT_ACK_BUDGET_US);
        let budget2 = test_budget(budgets::INPUT_ACK_BUDGET_US);
        assert_eq!(budget1, budget2);
    }

    #[test]
    fn test_test_budget_scales_proportionally() {
        let small = test_budget(100);
        let large = test_budget(1000);

        // large should be ~10x small (within floating-point tolerance)
        let ratio = large as f64 / small as f64;
        assert!(
            (ratio - 10.0).abs() < 1.0,
            "expected ~10x ratio, got {ratio:.2} (small={small}, large={large})",
        );
    }

    #[test]
    fn test_all_budget_constants_are_positive() {
        assert!(budgets::FRAME_BUDGET_US > 0);
        assert!(budgets::INPUT_ACK_BUDGET_US > 0);
        assert!(budgets::HIT_TEST_BUDGET_US > 0);
        assert!(budgets::TRANSACTION_VALIDATION_BUDGET_US > 0);
        assert!(budgets::SCENE_DIFF_BUDGET_US > 0);
        assert!(budgets::LEASE_GRANT_BUDGET_US > 0);
        assert!(budgets::BUDGET_ENFORCEMENT_US > 0);
        assert!(budgets::POLICY_EVALUATION_US > 0);
        assert!(budgets::EVENT_CLASSIFICATION_US > 0);
        assert!(budgets::EVENT_DELIVERY_US > 0);
        assert!(budgets::EVENT_DISPATCH_BUDGET_US > 0);
    }

    #[test]
    fn test_cache_serialization_roundtrip() {
        let result = CalibrationResult {
            speed_factor: 1.5,
            scene_ops_per_sec: 300_000.0,
            hash_throughput_mbps: 500.0,
            gpu_fill_factor: Some(9.5),
            texture_upload_factor: Some(3.2),
            timestamp: 1_700_000_000,
            calibration_duration_us: 250_000,
        };

        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: CalibrationResult =
            serde_json::from_str(&json).expect("deserialize");

        assert!((deserialized.speed_factor - 1.5).abs() < f64::EPSILON);
        assert_eq!(deserialized.timestamp, 1_700_000_000);
        assert!(deserialized.gpu_fill_factor.is_some());
        assert!((deserialized.gpu_fill_factor.unwrap() - 9.5).abs() < f64::EPSILON);
        assert!(deserialized.texture_upload_factor.is_some());
        assert!((deserialized.texture_upload_factor.unwrap() - 3.2).abs() < 1e-10);
    }

    #[test]
    fn test_cache_serialization_backward_compat_no_gpu_fields() {
        // Old calibration JSON without gpu_fill_factor or texture_upload_factor
        // should still deserialize correctly (fields default to None).
        let old_json = r#"{
            "speed_factor": 2.0,
            "scene_ops_per_sec": 200000.0,
            "hash_throughput_mbps": 400.0,
            "timestamp": 1700000000,
            "calibration_duration_us": 300000
        }"#;
        let result: CalibrationResult =
            serde_json::from_str(old_json).expect("deserialize old JSON");
        assert_eq!(result.gpu_fill_factor, None);
        assert_eq!(result.texture_upload_factor, None);
        assert!((result.speed_factor - 2.0).abs() < f64::EPSILON);
    }
}
