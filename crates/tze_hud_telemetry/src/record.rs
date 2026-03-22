//! Telemetry data types.

use serde::{Deserialize, Serialize};

/// Per-frame telemetry record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameTelemetry {
    /// Frame number (monotonically increasing).
    pub frame_number: u64,
    /// Timestamp of frame start (microseconds since process start).
    pub timestamp_us: u64,
    /// Total frame time in microseconds.
    pub frame_time_us: u64,
    /// Time spent in input drain (microseconds).
    pub input_drain_us: u64,
    /// Time spent in scene commit (microseconds).
    pub scene_commit_us: u64,
    /// Time spent in render encode (microseconds).
    pub render_encode_us: u64,
    /// Time spent in GPU submit + present (microseconds).
    pub gpu_submit_us: u64,
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
}

impl FrameTelemetry {
    pub fn new(frame_number: u64) -> Self {
        Self {
            frame_number,
            timestamp_us: 0,
            frame_time_us: 0,
            input_drain_us: 0,
            scene_commit_us: 0,
            render_encode_us: 0,
            gpu_submit_us: 0,
            tile_count: 0,
            node_count: 0,
            active_leases: 0,
            mutations_applied: 0,
            hit_region_updates: 0,
        }
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
}
