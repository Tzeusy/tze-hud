//! Telemetry data types.

use serde::{Deserialize, Serialize};

/// Per-frame telemetry record.
///
/// All stage timings are in microseconds (us). Stage names map to the
/// 8-stage frame pipeline defined in RFC 0002 §3.2:
///
/// | Stage | Thread     | Budget (p99) |
/// |-------|-----------|-------------|
/// | 1     | Main       | < 500us      |
/// | 2     | Main       | < 500us      |
/// | 3     | Compositor | < 1ms        |
/// | 4     | Compositor | < 1ms        |
/// | 5     | Compositor | < 1ms        |
/// | 6     | Compositor | < 4ms        |
/// | 7     | Compositor+Main | < 8ms   |
/// | 8     | Telemetry  | < 200us      |
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameTelemetry {
    /// Frame number (monotonically increasing).
    pub frame_number: u64,
    /// Timestamp of frame start (microseconds since the Unix epoch).
    ///
    /// Populated by the `FrameRecorder` using wall-clock time (`Clock::now_us()`).
    /// Not to be confused with a process-local monotonic offset.
    pub timestamp_us: u64,
    /// Total frame time in microseconds (Stage 1 start → Stage 7 end).
    pub frame_time_us: u64,

    // ── Per-stage timings ────────────────────────────────────────────────────

    /// Stage 1 — Input Drain (main thread). p99 budget: 500us.
    /// Drain OS input events, attach hardware timestamps, enqueue InputEvent records.
    pub stage1_input_drain_us: u64,

    /// Stage 2 — Local Feedback (main thread). p99 budget: 500us.
    /// Hit-test against tile bounds snapshot (ArcSwap), update pressed/hovered flags.
    pub stage2_local_feedback_us: u64,

    /// Stage 3 — Mutation Intake (compositor thread). p99 budget: 1ms.
    /// Drain MutationBatch channel, apply agent envelope limits. Each batch is atomic.
    pub stage3_mutation_intake_us: u64,

    /// Stage 4 — Scene Commit (compositor thread). p99 budget: 1ms.
    /// Apply validated batches with all-or-nothing semantics; publish hit-test snapshot.
    pub stage4_scene_commit_us: u64,

    /// Stage 5 — Layout Resolve (compositor thread). p99 budget: 1ms.
    /// Incremental layout: recompute only changed tiles, z-order, compositing regions.
    pub stage5_layout_resolve_us: u64,

    /// Stage 6 — Render Encode (compositor thread). p99 budget: 4ms.
    /// Build wgpu CommandEncoder; issue draw calls. MUST NOT submit to GPU queue.
    pub stage6_render_encode_us: u64,

    /// Stage 7 — GPU Submit + Present (compositor+main thread). p99 budget: 8ms.
    /// Submit CommandBuffer; signal main thread; main thread calls surface.present().
    pub stage7_gpu_submit_us: u64,

    /// Stage 8 — Telemetry Emit (telemetry thread). p99 budget: 200us.
    /// Non-blocking channel send of TelemetryRecord to telemetry thread.
    pub stage8_telemetry_emit_us: u64,

    // ── Legacy field aliases (in-process API compatibility only) ────────────
    //
    // These fields are excluded from serialization (`#[serde(skip)]`) so they
    // do NOT appear in JSON telemetry output. They exist solely for in-process
    // Rust callers that were written against the pre-stage-naming API.
    // If downstream consumers read the serialized JSON, migrate to the canonical
    // `stageN_*_us` field names; these aliases will not be present in the output.

    /// Alias for stage1_input_drain_us (in-process only; not serialized).
    #[serde(skip)]
    pub input_drain_us: u64,
    /// Alias for stage4_scene_commit_us (in-process only; not serialized).
    #[serde(skip)]
    pub scene_commit_us: u64,
    /// Alias for stage6_render_encode_us (in-process only; not serialized).
    #[serde(skip)]
    pub render_encode_us: u64,
    /// Alias for stage7_gpu_submit_us (in-process only; not serialized).
    #[serde(skip)]
    pub gpu_submit_us: u64,

    // ── Scene counters ───────────────────────────────────────────────────────

    /// Number of visible tiles this frame.
    pub tile_count: u32,
    /// Number of nodes rendered this frame.
    pub node_count: u32,
    /// Number of active leases.
    pub active_leases: u32,
    /// Number of mutations applied this frame.
    pub mutations_applied: u32,
    /// Number of hit-region states updated this frame.
    pub hit_region_updates: u32,
    /// Number of tiles that had layout recomputed (incremental layout).
    pub tiles_layout_recomputed: u32,
    /// Number of telemetry overflow drops since process start (non-blocking telemetry channel).
    pub telemetry_overflow_count: u64,
}

impl FrameTelemetry {
    pub fn new(frame_number: u64) -> Self {
        Self {
            frame_number,
            timestamp_us: 0,
            frame_time_us: 0,
            stage1_input_drain_us: 0,
            stage2_local_feedback_us: 0,
            stage3_mutation_intake_us: 0,
            stage4_scene_commit_us: 0,
            stage5_layout_resolve_us: 0,
            stage6_render_encode_us: 0,
            stage7_gpu_submit_us: 0,
            stage8_telemetry_emit_us: 0,
            // Legacy aliases
            input_drain_us: 0,
            scene_commit_us: 0,
            render_encode_us: 0,
            gpu_submit_us: 0,
            tile_count: 0,
            node_count: 0,
            active_leases: 0,
            mutations_applied: 0,
            hit_region_updates: 0,
            tiles_layout_recomputed: 0,
            telemetry_overflow_count: 0,
        }
    }

    /// Synchronize legacy alias fields from the per-stage fields.
    ///
    /// Call this after setting all stage fields to keep the deprecated aliases
    /// consistent with the canonical per-stage values.
    pub fn sync_legacy_aliases(&mut self) {
        self.input_drain_us = self.stage1_input_drain_us;
        self.scene_commit_us = self.stage4_scene_commit_us;
        self.render_encode_us = self.stage6_render_encode_us;
        self.gpu_submit_us = self.stage7_gpu_submit_us;
    }
}

/// Latency measurement bucket.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LatencyBucket {
    pub name: String,
    pub samples: Vec<u64>, // microseconds
}

impl LatencyBucket {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            samples: Vec::new(),
        }
    }

    pub fn record(&mut self, us: u64) {
        self.samples.push(us);
    }

    pub fn percentile(&self, pct: f64) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        // Nearest-rank method: ceil(pct/100 * N) - 1, clamped to valid range
        let rank = ((pct / 100.0) * sorted.len() as f64).ceil() as usize;
        let idx = rank.saturating_sub(1).min(sorted.len() - 1);
        Some(sorted[idx])
    }

    pub fn p50(&self) -> Option<u64> {
        self.percentile(50.0)
    }

    pub fn p95(&self) -> Option<u64> {
        self.percentile(95.0)
    }

    pub fn p99(&self) -> Option<u64> {
        self.percentile(99.0)
    }

    /// Assert that the p99 value is under the given budget (in microseconds).
    ///
    /// Returns `Ok(p99_value)` on pass, `Err(message)` on failure or if there
    /// are no samples.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tze_hud_telemetry::LatencyBucket;
    /// let mut bucket = LatencyBucket::new("frame_time");
    /// for _ in 0..100 { bucket.record(5_000); }
    /// assert!(bucket.assert_p99_under(16_600).is_ok());
    /// ```
    pub fn assert_p99_under(&self, budget_us: u64) -> Result<u64, String> {
        match self.p99() {
            None => Err(format!(
                "budget assertion failed for '{}': no samples recorded",
                self.name
            )),
            Some(p99) if p99 > budget_us => Err(format!(
                "budget assertion failed for '{}': p99={p99}us exceeds budget={budget_us}us \
                 (over by {}us, {:.1}%)",
                self.name,
                p99 - budget_us,
                (p99 as f64 / budget_us as f64 - 1.0) * 100.0,
            )),
            Some(p99) => Ok(p99),
        }
    }
}

/// Tier in the budget enforcement ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetTier {
    /// Agent is within all limits.
    Normal,
    /// Agent has exceeded a limit; grace period 5s before throttle.
    Warning,
    /// Agent is throttled: updates coalesced more aggressively, rate halved.
    Throttled,
    /// Agent session has been revoked and will be torn down.
    Revoked,
}

/// The kind of budget violation that was detected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetViolationKind {
    TileCountExceeded,
    TextureMemoryExceeded,
    UpdateRateExceeded,
    NodeCountPerTileExceeded,
    CriticalTextureOomAttempt,
    RepeatedInvariantViolations,
}

/// Telemetry event emitted when an agent's budget state changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetViolationEvent {
    /// Namespace of the offending agent session.
    pub namespace: String,
    /// New tier the agent has been moved to.
    pub new_tier: BudgetTier,
    /// The violation that triggered the transition.
    pub violation_kind: BudgetViolationKind,
    /// Timestamp (microseconds since process start) of the event.
    pub timestamp_us: u64,
    /// Human-readable detail.
    pub detail: String,
}

/// Frame-time guardian shed event — emitted when tiles are dropped to meet budget.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameTimeShedEvent {
    /// Frame number when shedding occurred.
    pub frame_number: u64,
    /// Number of tiles shed this frame.
    pub tiles_shed: u32,
    /// Cumulative elapsed time (µs) at Stage 5 that triggered the guardian.
    pub elapsed_us_at_stage5: u64,
    /// How many consecutive frames shedding has been active.
    pub consecutive_shed_frames: u32,
}

/// Per-session aggregated telemetry summary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub total_frames: u64,
    pub frame_time: LatencyBucket,
    pub input_to_local_ack: LatencyBucket,
    pub input_to_scene_commit: LatencyBucket,
    pub hit_test_latency: LatencyBucket,
    pub validation_latency: LatencyBucket,
    pub diff_latency: LatencyBucket,
    pub lease_acquire_latency: LatencyBucket,
    pub agent_connect_latency: LatencyBucket,
}

impl SessionSummary {
    pub fn new() -> Self {
        Self {
            total_frames: 0,
            frame_time: LatencyBucket::new("frame_time"),
            input_to_local_ack: LatencyBucket::new("input_to_local_ack"),
            input_to_scene_commit: LatencyBucket::new("input_to_scene_commit"),
            hit_test_latency: LatencyBucket::new("hit_test"),
            validation_latency: LatencyBucket::new("validation"),
            diff_latency: LatencyBucket::new("diff"),
            lease_acquire_latency: LatencyBucket::new("lease_acquire"),
            agent_connect_latency: LatencyBucket::new("agent_connect"),
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

impl Default for SessionSummary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_bucket_percentiles() {
        let mut bucket = LatencyBucket::new("test");
        for i in 1..=100 {
            bucket.record(i);
        }
        assert_eq!(bucket.p50(), Some(50));
        assert_eq!(bucket.p95(), Some(95));
        assert_eq!(bucket.p99(), Some(99));
    }

    #[test]
    fn test_session_summary_serialization() {
        let mut summary = SessionSummary::new();
        summary.total_frames = 100;
        summary.frame_time.record(12000);
        summary.frame_time.record(14000);

        let json = summary.to_json().unwrap();
        assert!(json.contains("frame_time"));
        assert!(json.contains("12000"));
    }

    #[test]
    fn test_assert_p99_under_passes_when_within_budget() {
        let mut bucket = LatencyBucket::new("test");
        for _ in 0..100 {
            bucket.record(5_000);
        }
        assert!(bucket.assert_p99_under(16_600).is_ok());
    }

    #[test]
    fn test_assert_p99_under_fails_when_exceeds_budget() {
        let mut bucket = LatencyBucket::new("test");
        for _ in 0..100 {
            bucket.record(20_000); // 20ms — over budget
        }
        let result = bucket.assert_p99_under(16_600);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("20000us"), "error should contain actual: {msg}");
        assert!(msg.contains("16600us"), "error should contain budget: {msg}");
    }

    #[test]
    fn test_assert_p99_under_fails_with_no_samples() {
        let bucket = LatencyBucket::new("empty");
        let result = bucket.assert_p99_under(16_600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no samples"));
    }
}
