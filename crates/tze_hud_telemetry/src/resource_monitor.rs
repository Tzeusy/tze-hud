//! Resource growth monitor for soak and leak test validation.
//!
//! Satisfies DR-V8: Soak and Leak Tests (validation-framework/spec.md lines 298-310).
//!
//! ## Usage
//!
//! ```no_run
//! use tze_hud_telemetry::resource_monitor::{ResourceSnapshot, ResourceMonitor};
//!
//! let mut monitor = ResourceMonitor::new();
//!
//! // Capture baseline at hour 1
//! let snap = ResourceSnapshot::new(10, 5, 2, 3);
//! monitor.record(snap);
//!
//! // ... run for N more hours ...
//!
//! // Assert no monotonic growth vs baseline
//! monitor.assert_no_monotonic_growth(0.05).unwrap();
//! ```
//!
//! ## Design
//!
//! `ResourceMonitor` records periodic `ResourceSnapshot`s and provides:
//!
//! - **`assert_no_monotonic_growth`** — asserts that no metric grew by more than
//!   the spec-required 5% relative to the baseline (hour-1 watermark). Any
//!   metric that exceeds the threshold is a test failure per:
//!   > "resource utilization at hour N SHALL be within 5% of resource utilization
//!   > at hour 1 for the same steady-state workload."
//!   (validation-framework/spec.md line 299)
//!
//! - **`assert_post_disconnect_zero`** — asserts that a named agent's resource
//!   footprint has reached zero. Satisfies:
//!   > "After an agent disconnects and leases expire, its resource footprint
//!   > MUST be zero."
//!   (validation-framework/spec.md line 299)
//!
//! - **`growth_trend`** — returns the maximum percentage growth across all
//!   tracked metrics relative to the first snapshot, useful for logging.

use serde::{Deserialize, Serialize};

/// Maximum permitted growth fraction before the spec's "monotonic growth is a bug"
/// threshold is exceeded (validation-framework/spec.md line 299).
///
/// Value: 5% (0.05).
pub const SPEC_GROWTH_TOLERANCE: f64 = 0.05;

/// A point-in-time snapshot of runtime resource utilisation.
///
/// All counters represent absolute values at the moment of capture. They are
/// obtained from the `SceneGraph` and `SessionRegistry` at the time of the
/// snapshot.
///
/// ## What is tracked
///
/// | Field | Source |
/// |---|---|
/// | `tile_count` | `SceneGraph::tile_count()` |
/// | `node_count` | `SceneGraph::node_count()` |
/// | `lease_count` | `SceneGraph::leases.len()` |
/// | `session_count` | `SessionRegistry::session_count()` |
/// | `zone_entry_count` | `ZoneRegistry` active entry count |
/// | `texture_bytes` | Sum of `ResourceUsage::texture_bytes` over all leases |
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    /// Wall-clock timestamp in seconds (seconds since process start, or
    /// test-clock epoch). Used for trend display only — not for assertions.
    pub elapsed_secs: f64,
    /// Total tiles in the scene graph.
    pub tile_count: usize,
    /// Total nodes in the scene graph.
    pub node_count: usize,
    /// Total active leases.
    pub lease_count: usize,
    /// Total connected sessions.
    pub session_count: usize,
    /// Total active zone publication entries.
    pub zone_entry_count: usize,
    /// Estimated texture memory in bytes across all lease-owned tiles.
    pub texture_bytes: u64,
}

impl ResourceSnapshot {
    /// Create a snapshot with explicit field values (typical test path).
    pub fn new(
        tile_count: usize,
        node_count: usize,
        lease_count: usize,
        session_count: usize,
    ) -> Self {
        Self {
            elapsed_secs: 0.0,
            tile_count,
            node_count,
            lease_count,
            session_count,
            zone_entry_count: 0,
            texture_bytes: 0,
        }
    }

    /// Create a full snapshot with all fields.
    pub fn full(
        elapsed_secs: f64,
        tile_count: usize,
        node_count: usize,
        lease_count: usize,
        session_count: usize,
        zone_entry_count: usize,
        texture_bytes: u64,
    ) -> Self {
        Self {
            elapsed_secs,
            tile_count,
            node_count,
            lease_count,
            session_count,
            zone_entry_count,
            texture_bytes,
        }
    }

    /// Return a zero snapshot (all fields zero, elapsed=0). Useful as a sentinel.
    pub fn zero(elapsed_secs: f64) -> Self {
        Self {
            elapsed_secs,
            tile_count: 0,
            node_count: 0,
            lease_count: 0,
            session_count: 0,
            zone_entry_count: 0,
            texture_bytes: 0,
        }
    }

    /// Check whether all resource counters are exactly zero.
    ///
    /// Satisfies validation-framework/spec.md line 307:
    /// > "WHEN an agent disconnects and its leases expire … resource footprint
    /// > … MUST reach exactly zero."
    pub fn is_zero_footprint(&self) -> bool {
        self.tile_count == 0
            && self.node_count == 0
            && self.lease_count == 0
            && self.zone_entry_count == 0
            && self.texture_bytes == 0
    }

    /// Compute growth ratio relative to a baseline snapshot.
    ///
    /// For each metric, returns `(current - baseline) / baseline` if baseline > 0,
    /// or 0.0 if the baseline is zero (no growth possible from nothing).
    ///
    /// Returns a `GrowthRatios` struct with one entry per tracked metric.
    pub fn growth_ratios_vs(&self, baseline: &ResourceSnapshot) -> GrowthRatios {
        fn ratio(current: u64, base: u64) -> f64 {
            if base == 0 {
                0.0
            } else {
                (current as f64 - base as f64) / base as f64
            }
        }

        GrowthRatios {
            tile_count: ratio(self.tile_count as u64, baseline.tile_count as u64),
            node_count: ratio(self.node_count as u64, baseline.node_count as u64),
            lease_count: ratio(self.lease_count as u64, baseline.lease_count as u64),
            session_count: ratio(self.session_count as u64, baseline.session_count as u64),
            zone_entry_count: ratio(
                self.zone_entry_count as u64,
                baseline.zone_entry_count as u64,
            ),
            texture_bytes: ratio(self.texture_bytes, baseline.texture_bytes),
        }
    }
}

/// Growth ratios for each tracked resource metric.
///
/// Each value is `(current - baseline) / baseline`, clamped to 0.0 when baseline = 0.
/// Positive values indicate growth; negative values indicate shrinkage.
///
/// The spec's pass criterion is: all values ≤ [`SPEC_GROWTH_TOLERANCE`] (5%).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GrowthRatios {
    pub tile_count: f64,
    pub node_count: f64,
    pub lease_count: f64,
    pub session_count: f64,
    pub zone_entry_count: f64,
    pub texture_bytes: f64,
}

impl GrowthRatios {
    /// Return the maximum growth ratio across all metrics.
    ///
    /// A positive result indicates the worst-case growth; negative means all
    /// metrics shrank relative to baseline.
    pub fn max_growth(&self) -> f64 {
        let values = [
            self.tile_count,
            self.node_count,
            self.lease_count,
            self.session_count,
            self.zone_entry_count,
            self.texture_bytes,
        ];
        values.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    }

    /// Return the name of the metric with the highest growth ratio.
    ///
    /// Useful for reporting which metric is the culprit when an assertion fails.
    pub fn worst_metric(&self) -> &'static str {
        let named: &[(&'static str, f64)] = &[
            ("tile_count", self.tile_count),
            ("node_count", self.node_count),
            ("lease_count", self.lease_count),
            ("session_count", self.session_count),
            ("zone_entry_count", self.zone_entry_count),
            ("texture_bytes", self.texture_bytes),
        ];
        named
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(name, _)| *name)
            .unwrap_or("(none)")
    }
}

/// Resource growth monitor — records periodic snapshots and asserts spec compliance.
///
/// ## Lifecycle
///
/// 1. Call [`ResourceMonitor::record`] periodically during the soak test.
/// 2. After the test, call [`ResourceMonitor::assert_no_monotonic_growth`] with
///    the tolerance fraction (typically [`SPEC_GROWTH_TOLERANCE`]).
///
/// ## Baseline
///
/// The baseline is the **first recorded snapshot** (the "hour 1" measurement).
/// All subsequent snapshots are compared against this baseline. The spec states:
/// > "resource utilization at hour N SHALL be within 5% of resource utilization
/// > at hour 1 for the same steady-state workload."
pub struct ResourceMonitor {
    snapshots: Vec<ResourceSnapshot>,
}

impl ResourceMonitor {
    /// Create a new, empty monitor.
    pub fn new() -> Self {
        Self {
            snapshots: Vec::new(),
        }
    }

    /// Record a new snapshot.
    ///
    /// The first snapshot becomes the baseline for all future assertions.
    pub fn record(&mut self, snap: ResourceSnapshot) {
        self.snapshots.push(snap);
    }

    /// Return a reference to all recorded snapshots.
    pub fn snapshots(&self) -> &[ResourceSnapshot] {
        &self.snapshots
    }

    /// Return the baseline snapshot (first recorded), if any.
    pub fn baseline(&self) -> Option<&ResourceSnapshot> {
        self.snapshots.first()
    }

    /// Return the most recently recorded snapshot, if any.
    pub fn latest(&self) -> Option<&ResourceSnapshot> {
        self.snapshots.last()
    }

    /// Return the count of recorded snapshots.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Return true if no snapshots have been recorded.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Compute the growth ratio of the latest snapshot vs the baseline.
    ///
    /// Returns `None` if fewer than two snapshots have been recorded.
    pub fn growth_trend(&self) -> Option<GrowthRatios> {
        let baseline = self.baseline()?;
        let latest = self.latest()?;
        // Need at least 2 snapshots to compute a trend
        if self.snapshots.len() < 2 {
            return None;
        }
        Some(latest.growth_ratios_vs(baseline))
    }

    /// Assert that no tracked metric grew by more than `tolerance` relative to
    /// the baseline snapshot.
    ///
    /// Per validation-framework/spec.md line 299:
    /// > "resource utilization at hour N SHALL be within 5% of resource
    /// > utilization at hour 1 for the same steady-state workload. Any
    /// > monotonic growth SHALL be a bug."
    ///
    /// `tolerance` is a fraction, e.g. `0.05` for 5%.
    ///
    /// Returns `Ok(GrowthRatios)` if all metrics are within tolerance.
    /// Returns `Err(message)` with the offending metric if any metric exceeds
    /// the tolerance.
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Fewer than 2 snapshots have been recorded (nothing to compare).
    /// - Any metric exceeds the tolerance relative to baseline.
    pub fn assert_no_monotonic_growth(
        &self,
        tolerance: f64,
    ) -> Result<GrowthRatios, String> {
        if self.snapshots.len() < 2 {
            return Err(format!(
                "not enough snapshots to assess growth: need ≥ 2, got {}",
                self.snapshots.len()
            ));
        }
        let baseline = self.baseline().unwrap();
        let latest = self.latest().unwrap();
        let ratios = latest.growth_ratios_vs(baseline);
        let max = ratios.max_growth();
        if max > tolerance {
            return Err(format!(
                "resource growth exceeded {:.1}% tolerance: {} grew by {:.1}% \
                 (baseline@{:.0}s → latest@{:.0}s)",
                tolerance * 100.0,
                ratios.worst_metric(),
                max * 100.0,
                baseline.elapsed_secs,
                latest.elapsed_secs,
            ));
        }
        Ok(ratios)
    }

    /// Assert that the agent's resource footprint is exactly zero.
    ///
    /// Takes a snapshot captured after the agent disconnected and its leases
    /// expired, and verifies that no residue remains.
    ///
    /// Per validation-framework/spec.md line 308:
    /// > "WHEN an agent disconnects and its leases expire during a soak test
    /// > THEN the agent's resource footprint (memory, textures, scene graph
    /// > nodes) MUST reach exactly zero"
    ///
    /// `agent_label` is used only for the error message.
    pub fn assert_post_disconnect_zero(
        &self,
        agent_label: &str,
        post_disconnect_snap: &ResourceSnapshot,
    ) -> Result<(), String> {
        if !post_disconnect_snap.is_zero_footprint() {
            return Err(format!(
                "post-disconnect resource footprint not zero for '{}': \
                 tiles={}, nodes={}, leases={}, zone_entries={}, texture_bytes={}",
                agent_label,
                post_disconnect_snap.tile_count,
                post_disconnect_snap.node_count,
                post_disconnect_snap.lease_count,
                post_disconnect_snap.zone_entry_count,
                post_disconnect_snap.texture_bytes,
            ));
        }
        Ok(())
    }

    /// Emit the full snapshot history as a JSON string.
    ///
    /// Used by CI to produce a structured artifact for trend analysis.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        #[derive(Serialize)]
        struct Report<'a> {
            snapshot_count: usize,
            baseline: Option<&'a ResourceSnapshot>,
            latest: Option<&'a ResourceSnapshot>,
            growth_trend: Option<GrowthRatios>,
            snapshots: &'a [ResourceSnapshot],
        }
        let report = Report {
            snapshot_count: self.snapshots.len(),
            baseline: self.baseline(),
            latest: self.latest(),
            growth_trend: self.growth_trend(),
            snapshots: &self.snapshots,
        };
        serde_json::to_string_pretty(&report)
    }
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Per-agent footprint ──────────────────────────────────────────────────────

/// Resource footprint for a single named agent, captured at a moment in time.
///
/// Used for post-disconnect cleanup validation:
/// after an agent's leases expire, its `AgentFootprint` MUST be all-zero.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentFootprint {
    /// Agent namespace / identifier.
    pub namespace: String,
    /// Wall clock offset when snapshot was taken (seconds since test start).
    pub elapsed_secs: f64,
    /// Number of tiles owned by this agent.
    pub tiles: usize,
    /// Number of scene-graph nodes owned by this agent.
    pub nodes: usize,
    /// Number of active leases held by this agent.
    pub leases: usize,
    /// Number of zone publication entries from this agent.
    pub zone_entries: usize,
    /// Estimated texture bytes owned by this agent.
    pub texture_bytes: u64,
}

impl AgentFootprint {
    /// Create a footprint with all counts at zero.
    pub fn zero(namespace: &str, elapsed_secs: f64) -> Self {
        Self {
            namespace: namespace.to_string(),
            elapsed_secs,
            tiles: 0,
            nodes: 0,
            leases: 0,
            zone_entries: 0,
            texture_bytes: 0,
        }
    }

    /// Return true if all resource counts are exactly zero.
    pub fn is_zero(&self) -> bool {
        self.tiles == 0
            && self.nodes == 0
            && self.leases == 0
            && self.zone_entries == 0
            && self.texture_bytes == 0
    }

    /// Assert that this footprint is exactly zero.
    ///
    /// Returns `Ok(())` if all fields are zero, `Err(message)` otherwise.
    pub fn assert_zero(&self) -> Result<(), String> {
        if self.is_zero() {
            Ok(())
        } else {
            Err(format!(
                "agent '{}' has non-zero footprint at t={:.0}s: \
                 tiles={}, nodes={}, leases={}, zone_entries={}, texture_bytes={}",
                self.namespace,
                self.elapsed_secs,
                self.tiles,
                self.nodes,
                self.leases,
                self.zone_entries,
                self.texture_bytes,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_snapshot_is_zero_footprint() {
        let zero = ResourceSnapshot::zero(0.0);
        assert!(zero.is_zero_footprint());

        let nonzero = ResourceSnapshot::new(1, 0, 0, 0);
        assert!(!nonzero.is_zero_footprint());
    }

    #[test]
    fn test_growth_ratios_baseline_zero_means_no_growth() {
        let baseline = ResourceSnapshot::zero(0.0);
        let later = ResourceSnapshot::new(10, 5, 2, 3);
        let ratios = later.growth_ratios_vs(&baseline);
        // All baseline fields are 0 — ratio function returns 0.0 for all
        assert_eq!(ratios.tile_count, 0.0);
        assert_eq!(ratios.max_growth(), 0.0);
    }

    #[test]
    fn test_growth_ratios_within_tolerance() {
        let baseline = ResourceSnapshot::full(10.0, 100, 50, 3, 3, 5, 1_000_000);
        // 4% growth on tiles — within 5% tolerance
        let later = ResourceSnapshot::full(3610.0, 104, 50, 3, 3, 5, 1_000_000);
        let ratios = later.growth_ratios_vs(&baseline);
        assert!(
            ratios.tile_count < 0.05,
            "4% growth should be within tolerance"
        );
        assert!(ratios.max_growth() <= 0.05);
    }

    #[test]
    fn test_growth_ratios_exceeds_tolerance() {
        let baseline = ResourceSnapshot::full(10.0, 100, 50, 3, 3, 5, 1_000_000);
        // 20% growth on tiles — exceeds 5% tolerance
        let later = ResourceSnapshot::full(3610.0, 120, 50, 3, 3, 5, 1_000_000);
        let ratios = later.growth_ratios_vs(&baseline);
        assert!(ratios.tile_count > 0.05, "20% growth should exceed tolerance");
        assert!(ratios.max_growth() > 0.05);
        assert_eq!(ratios.worst_metric(), "tile_count");
    }

    #[test]
    fn test_monitor_assert_no_monotonic_growth_passes() {
        let mut monitor = ResourceMonitor::new();
        monitor.record(ResourceSnapshot::full(10.0, 100, 50, 3, 3, 5, 0));
        // 3% growth — within spec tolerance
        monitor.record(ResourceSnapshot::full(3610.0, 103, 50, 3, 3, 5, 0));
        let result = monitor.assert_no_monotonic_growth(SPEC_GROWTH_TOLERANCE);
        assert!(result.is_ok(), "3% growth should pass: {:?}", result.err());
    }

    #[test]
    fn test_monitor_assert_no_monotonic_growth_fails_on_leak() {
        let mut monitor = ResourceMonitor::new();
        monitor.record(ResourceSnapshot::full(10.0, 100, 50, 3, 3, 5, 0));
        // 50% growth on nodes — memory leak scenario
        monitor.record(ResourceSnapshot::full(3610.0, 100, 75, 3, 3, 5, 0));
        let result = monitor.assert_no_monotonic_growth(SPEC_GROWTH_TOLERANCE);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("node_count"), "error should name the culprit metric: {msg}");
        assert!(msg.contains("50.0%"), "error should include growth percentage: {msg}");
    }

    #[test]
    fn test_monitor_requires_two_snapshots() {
        let mut monitor = ResourceMonitor::new();
        monitor.record(ResourceSnapshot::new(10, 5, 2, 3));
        let result = monitor.assert_no_monotonic_growth(SPEC_GROWTH_TOLERANCE);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not enough snapshots"));
    }

    #[test]
    fn test_assert_post_disconnect_zero_passes_on_zero() {
        let monitor = ResourceMonitor::new();
        let zero = ResourceSnapshot::zero(3601.0);
        assert!(monitor
            .assert_post_disconnect_zero("agent-alpha", &zero)
            .is_ok());
    }

    #[test]
    fn test_assert_post_disconnect_zero_fails_with_residue() {
        let monitor = ResourceMonitor::new();
        let nonzero = ResourceSnapshot::full(3601.0, 2, 4, 1, 0, 0, 0);
        let result = monitor.assert_post_disconnect_zero("agent-alpha", &nonzero);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("agent-alpha"), "error should name the agent: {msg}");
        assert!(msg.contains("tiles=2"), "error should list tile count: {msg}");
    }

    #[test]
    fn test_monitor_growth_trend_none_if_single_snapshot() {
        let mut monitor = ResourceMonitor::new();
        monitor.record(ResourceSnapshot::new(10, 5, 2, 3));
        assert!(monitor.growth_trend().is_none());
    }

    #[test]
    fn test_monitor_growth_trend_some_if_two_snapshots() {
        let mut monitor = ResourceMonitor::new();
        monitor.record(ResourceSnapshot::full(10.0, 100, 50, 3, 3, 5, 0));
        monitor.record(ResourceSnapshot::full(3610.0, 101, 50, 3, 3, 5, 0));
        assert!(monitor.growth_trend().is_some());
    }

    #[test]
    fn test_agent_footprint_assert_zero() {
        let zero = AgentFootprint::zero("agent-x", 3600.0);
        assert!(zero.is_zero());
        assert!(zero.assert_zero().is_ok());

        let nonzero = AgentFootprint {
            namespace: "agent-x".to_string(),
            elapsed_secs: 3600.0,
            tiles: 1,
            nodes: 2,
            leases: 1,
            zone_entries: 0,
            texture_bytes: 0,
        };
        assert!(!nonzero.is_zero());
        let result = nonzero.assert_zero();
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("agent-x"), "error must name the agent: {msg}");
        assert!(msg.contains("tiles=1"), "error must list tiles: {msg}");
    }

    #[test]
    fn test_monitor_to_json() {
        let mut monitor = ResourceMonitor::new();
        monitor.record(ResourceSnapshot::full(10.0, 100, 50, 3, 3, 5, 1024));
        monitor.record(ResourceSnapshot::full(3610.0, 101, 50, 3, 3, 5, 1024));
        let json = monitor.to_json().unwrap();
        assert!(json.contains("snapshot_count"));
        assert!(json.contains("tile_count"));
        assert!(json.contains("1024"));
    }
}
