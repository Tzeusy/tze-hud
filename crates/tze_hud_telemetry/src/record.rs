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

    // ── Split input latency measurements ────────────────────────────────────
    //
    // These three fields carry the split latency measurements required by
    // validation-framework/spec.md §"Split Latency Budgets". Each records
    // the elapsed time from the triggering input event to a specific pipeline
    // boundary for the *current frame*. A value of 0 means no input event
    // occurred this frame for that measurement point.

    /// input_to_local_ack — time from input event arrival to Stage 2 completion
    /// (local visual feedback rendered). p99 budget: 4ms (4_000 µs).
    /// Populated by the input processor; 0 when no input event occurred this frame.
    pub input_to_local_ack_us: u64,

    /// input_to_scene_commit — time from input event arrival to Stage 4
    /// completion (agent mutation reflected in scene graph). p99 budget: 50ms.
    /// Populated when an agent commits a mutation in response to this frame's
    /// input; 0 when no agent response was committed this frame.
    pub input_to_scene_commit_us: u64,

    /// input_to_next_present — time from input event arrival to Stage 7
    /// completion (GPU present of the frame containing the agent response).
    /// p99 budget: 33ms (two frames at 60Hz). Populated when Stage 7 completes
    /// on a frame that carries a scene commit triggered by input; 0 otherwise.
    pub input_to_next_present_us: u64,

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
            // Split input latency measurements
            input_to_local_ack_us: 0,
            input_to_scene_commit_us: 0,
            input_to_next_present_us: 0,
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

/// Telemetry event emitted when the degradation level changes.
///
/// Emitted on every level transition (both advance and recovery).
/// Consumers use this to track degradation history and tune thresholds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DegradationEvent {
    /// Frame number when the transition occurred.
    pub frame_number: u64,
    /// Previous degradation level (0 = Normal, 5 = Emergency).
    pub previous_level: u8,
    /// New degradation level after transition.
    pub new_level: u8,
    /// The rolling-window p95 frame time (µs) that triggered this transition.
    pub frame_time_p95_us: u64,
    /// Direction of the transition.
    pub direction: DegradationDirection,
}

/// Direction of a degradation level transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DegradationDirection {
    /// Level worsened (frame_time_p95 > 14ms trigger threshold).
    Advance,
    /// Level improved (frame_time_p95 < 12ms sustained over 30 frames).
    Recover,
}

/// Per-session aggregated telemetry summary.
///
/// Covers all Layer-3 performance requirements:
/// - Per-session totals: total_frames, fps, elapsed_us
/// - Frame time percentiles (p50/p95/p99) via `frame_time`
/// - Full latency breakdown: input_to_local_ack, input_to_scene_commit, input_to_next_present
/// - Peak tracking: peak_frame_time_us, peak_tile_count
/// - Violation counters: lease_violations, budget_overruns, sync_drift_violations
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SessionSummary {
    /// Total frames rendered in this session.
    pub total_frames: u64,
    /// Total session duration in microseconds (set externally when session ends).
    #[serde(default)]
    pub elapsed_us: u64,
    /// Average FPS over the session (computed from total_frames / elapsed_us).
    /// Zero if elapsed_us == 0.
    #[serde(default)]
    pub fps: f64,
    /// Per-frame total time (Stage 1 start → Stage 7 end), microseconds.
    pub frame_time: LatencyBucket,
    /// input_to_local_ack — time from input event to Stage 2 completion.
    /// Spec: p99 < 4ms (4_000 µs). Purely local, no network.
    #[serde(default)]
    pub input_to_local_ack: LatencyBucket,
    /// input_to_scene_commit — time from input event to Stage 4 completion.
    /// Spec: p99 < 50ms (50_000 µs). Covers agent response round-trip.
    #[serde(default)]
    pub input_to_scene_commit: LatencyBucket,
    /// input_to_next_present — time from input event to Stage 7 completion
    /// (GPU present of frame containing agent response).
    /// Spec: p99 < 33ms (33_000 µs) at 60Hz (two frames).
    #[serde(default)]
    pub input_to_next_present: LatencyBucket,
    /// Hit-test latency.
    pub hit_test_latency: LatencyBucket,
    /// Mutation batch validation latency.
    pub validation_latency: LatencyBucket,
    /// Scene diff computation latency.
    pub diff_latency: LatencyBucket,
    /// Lease acquire latency.
    pub lease_acquire_latency: LatencyBucket,
    /// Agent connect latency.
    pub agent_connect_latency: LatencyBucket,
    /// Peak single-frame time observed (microseconds).
    #[serde(default)]
    pub peak_frame_time_us: u64,
    /// Peak tile count seen in any single frame.
    #[serde(default)]
    pub peak_tile_count: u32,
    /// Number of lease violations observed (zero is the pass threshold).
    #[serde(default)]
    pub lease_violations: u64,
    /// Number of budget overruns observed (zero is the pass threshold).
    #[serde(default)]
    pub budget_overruns: u64,
    /// Number of sync drift violations (drift > 500µs).
    #[serde(default)]
    pub sync_drift_violations: u64,
}

impl SessionSummary {
    pub fn new() -> Self {
        Self {
            total_frames: 0,
            elapsed_us: 0,
            fps: 0.0,
            frame_time: LatencyBucket::new("frame_time"),
            input_to_local_ack: LatencyBucket::new("input_to_local_ack"),
            input_to_scene_commit: LatencyBucket::new("input_to_scene_commit"),
            input_to_next_present: LatencyBucket::new("input_to_next_present"),
            hit_test_latency: LatencyBucket::new("hit_test"),
            validation_latency: LatencyBucket::new("validation"),
            diff_latency: LatencyBucket::new("diff"),
            lease_acquire_latency: LatencyBucket::new("lease_acquire"),
            agent_connect_latency: LatencyBucket::new("agent_connect"),
            peak_frame_time_us: 0,
            peak_tile_count: 0,
            lease_violations: 0,
            budget_overruns: 0,
            sync_drift_violations: 0,
        }
    }

    /// Record a frame's telemetry into this summary.
    ///
    /// Updates total_frames, frame_time bucket, and peak_frame_time_us.
    pub fn record_frame(&mut self, frame_time_us: u64, tile_count: u32) {
        self.total_frames += 1;
        self.frame_time.record(frame_time_us);
        if frame_time_us > self.peak_frame_time_us {
            self.peak_frame_time_us = frame_time_us;
        }
        if tile_count > self.peak_tile_count {
            self.peak_tile_count = tile_count;
        }
    }

    /// Finalize: compute FPS from total_frames and elapsed_us.
    ///
    /// Call this once the session ends and `elapsed_us` has been set.
    pub fn finalize(&mut self) {
        if self.elapsed_us > 0 {
            self.fps = self.total_frames as f64 / (self.elapsed_us as f64 / 1_000_000.0);
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

    /// Verify that all three split latency buckets exist in SessionSummary and
    /// serialize to their canonical names.
    #[test]
    fn test_session_summary_has_three_split_latency_buckets() {
        let mut summary = SessionSummary::new();

        // Populate each bucket independently
        summary.input_to_local_ack.record(1_000);    // 1ms
        summary.input_to_scene_commit.record(10_000); // 10ms
        summary.input_to_next_present.record(20_000); // 20ms

        // Budget assertions must pass for all three
        assert!(
            summary.input_to_local_ack.assert_p99_under(4_000).is_ok(),
            "input_to_local_ack p99 must be under 4ms budget"
        );
        assert!(
            summary.input_to_scene_commit.assert_p99_under(50_000).is_ok(),
            "input_to_scene_commit p99 must be under 50ms budget"
        );
        assert!(
            summary.input_to_next_present.assert_p99_under(33_000).is_ok(),
            "input_to_next_present p99 must be under 33ms budget"
        );

        // Serialized JSON must contain all three bucket names
        let json = summary.to_json().unwrap();
        assert!(json.contains("input_to_local_ack"), "JSON must contain input_to_local_ack");
        assert!(json.contains("input_to_scene_commit"), "JSON must contain input_to_scene_commit");
        assert!(json.contains("input_to_next_present"), "JSON must contain input_to_next_present");
    }

    /// Verify that FrameTelemetry carries all three split latency fields.
    #[test]
    fn test_frame_telemetry_has_split_latency_fields() {
        let mut frame = FrameTelemetry::new(1);
        frame.input_to_local_ack_us = 500;     // 0.5ms
        frame.input_to_scene_commit_us = 5_000; // 5ms
        frame.input_to_next_present_us = 15_000; // 15ms

        // Fields round-trip through the struct
        assert_eq!(frame.input_to_local_ack_us, 500);
        assert_eq!(frame.input_to_scene_commit_us, 5_000);
        assert_eq!(frame.input_to_next_present_us, 15_000);

        // Serialized JSON must contain all three field names
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("input_to_local_ack_us"), "JSON must contain input_to_local_ack_us");
        assert!(json.contains("input_to_scene_commit_us"), "JSON must contain input_to_scene_commit_us");
        assert!(json.contains("input_to_next_present_us"), "JSON must contain input_to_next_present_us");
    }

    #[test]
    fn test_session_summary_record_frame_updates_peaks() {
        let mut summary = SessionSummary::new();
        summary.record_frame(10_000, 5);
        summary.record_frame(20_000, 3);
        summary.record_frame(15_000, 8);

        assert_eq!(summary.total_frames, 3);
        assert_eq!(summary.peak_frame_time_us, 20_000);
        assert_eq!(summary.peak_tile_count, 8);
    }

    #[test]
    fn test_session_summary_finalize_computes_fps() {
        let mut summary = SessionSummary::new();
        summary.total_frames = 60;
        summary.elapsed_us = 1_000_000; // 1 second
        summary.finalize();
        assert!((summary.fps - 60.0).abs() < 0.001);
    }

    #[test]
    fn test_session_summary_finalize_zero_elapsed() {
        let mut summary = SessionSummary::new();
        summary.total_frames = 10;
        summary.elapsed_us = 0;
        summary.finalize();
        assert_eq!(summary.fps, 0.0);
    }

    #[test]
    fn test_session_summary_has_input_to_next_present() {
        let mut summary = SessionSummary::new();
        summary.input_to_next_present.record(25_000);
        assert_eq!(summary.input_to_next_present.p99(), Some(25_000));
        let json = summary.to_json().unwrap();
        assert!(json.contains("input_to_next_present"));
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
