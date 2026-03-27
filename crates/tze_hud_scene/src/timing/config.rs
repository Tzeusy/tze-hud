//! Timing configuration with defaults and validation ranges.
//!
//! # Spec alignment
//!
//! Implements `timing-model/spec.md §Requirement: Timing Configuration`
//! (lines 391-402).
//!
//! All timing parameters are configurable with documented defaults and
//! validation ranges.  Out-of-range values MUST be rejected.

use serde::{Deserialize, Serialize};

// ─── TimingConfig ─────────────────────────────────────────────────────────────

/// All timing parameters configurable at runtime startup.
///
/// Use [`TimingConfig::default`] to get all spec defaults.
/// Call [`TimingConfig::validate`] to check ranges before applying.
///
/// From spec §Requirement: Timing Configuration (lines 391-402).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingConfig {
    /// Frame rate target.
    ///
    /// Default: 60. Range: 1–240.
    pub target_fps: u32,

    /// Maximum agent clock drift in milliseconds before CLOCK_SKEW_HIGH is
    /// emitted.
    ///
    /// Default: 100. Range: 1–10000.
    pub max_agent_clock_drift_ms: u32,

    /// Maximum vsync jitter tolerance in milliseconds.
    ///
    /// Default: 2. Range: 0–100.
    pub max_vsync_jitter_ms: u32,

    /// Maximum `present_at_wall_us` in the future (microseconds).
    ///
    /// Default: 300_000_000 (5 minutes). Range: 1_000_000–3_600_000_000.
    pub max_future_schedule_us: u64,

    /// Maximum deferral frames for an `AllOrDefer` sync group before
    /// force-commit.
    ///
    /// Default: 3. Range: 1–60.
    pub sync_group_max_defer_frames: u32,

    /// Maximum depth of the per-agent pending queue.
    ///
    /// Default: 256. Range: 16–4096.
    pub pending_queue_depth_per_agent: u32,

    /// Budget for sync-group arrival spread (microseconds).
    ///
    /// Default: 500. Range: 1–100_000.
    pub sync_drift_budget_us: u64,

    /// Tile staleness threshold in milliseconds.
    ///
    /// Default: 5000. Range: 500–300_000.
    pub tile_stale_threshold_ms: u64,

    /// Clock-jump detection threshold in milliseconds.  If consecutive skew
    /// samples differ by more than this, the estimation window is reset.
    ///
    /// Default: 50. Range: 10–10_000.
    pub clock_jump_detection_ms: u64,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            target_fps: 60,
            max_agent_clock_drift_ms: 100,
            max_vsync_jitter_ms: 2,
            max_future_schedule_us: 300_000_000,
            sync_group_max_defer_frames: 3,
            pending_queue_depth_per_agent: 256,
            sync_drift_budget_us: 500,
            tile_stale_threshold_ms: 5_000,
            clock_jump_detection_ms: 50,
        }
    }
}

// ─── Validation ───────────────────────────────────────────────────────────────

/// A single out-of-range validation error for `TimingConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingConfigError {
    /// Name of the offending field.
    pub field: &'static str,
    /// The supplied value as a string for error reporting.
    pub got: String,
    /// Human-readable description of the valid range.
    pub expected_range: &'static str,
}

impl std::fmt::Display for TimingConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TimingConfig field '{}' = {} is out of range ({})",
            self.field, self.got, self.expected_range
        )
    }
}

impl TimingConfig {
    /// Validate all fields against their documented ranges.
    ///
    /// Returns a `Vec` of all violations found (never stops at the first).
    /// An empty vec means the config is valid.
    ///
    /// From spec §Requirement: Timing Configuration (lines 391-402):
    /// - `target_fps` range: 1–240
    /// - `max_agent_clock_drift_ms` range: 1–10000
    /// - `max_vsync_jitter_ms` range: 0–100
    /// - `max_future_schedule_us` range: 1_000_000–3_600_000_000
    /// - `sync_group_max_defer_frames` range: 1–60
    /// - `pending_queue_depth_per_agent` range: 16–4096
    /// - `sync_drift_budget_us` range: 1–100_000
    /// - `tile_stale_threshold_ms` range: 500–300_000
    /// - `clock_jump_detection_ms` range: 10–10_000
    pub fn validate(&self) -> Vec<TimingConfigError> {
        let mut errors = Vec::new();

        if self.target_fps < 1 || self.target_fps > 240 {
            errors.push(TimingConfigError {
                field: "target_fps",
                got: self.target_fps.to_string(),
                expected_range: "1-240",
            });
        }

        if self.max_agent_clock_drift_ms < 1 || self.max_agent_clock_drift_ms > 10_000 {
            errors.push(TimingConfigError {
                field: "max_agent_clock_drift_ms",
                got: self.max_agent_clock_drift_ms.to_string(),
                expected_range: "1-10000",
            });
        }

        // max_vsync_jitter_ms: 0-100 (0 is valid)
        if self.max_vsync_jitter_ms > 100 {
            errors.push(TimingConfigError {
                field: "max_vsync_jitter_ms",
                got: self.max_vsync_jitter_ms.to_string(),
                expected_range: "0-100",
            });
        }

        if self.max_future_schedule_us < 1_000_000 || self.max_future_schedule_us > 3_600_000_000 {
            errors.push(TimingConfigError {
                field: "max_future_schedule_us",
                got: self.max_future_schedule_us.to_string(),
                expected_range: "1000000-3600000000",
            });
        }

        if self.sync_group_max_defer_frames < 1 || self.sync_group_max_defer_frames > 60 {
            errors.push(TimingConfigError {
                field: "sync_group_max_defer_frames",
                got: self.sync_group_max_defer_frames.to_string(),
                expected_range: "1-60",
            });
        }

        if self.pending_queue_depth_per_agent < 16 || self.pending_queue_depth_per_agent > 4096 {
            errors.push(TimingConfigError {
                field: "pending_queue_depth_per_agent",
                got: self.pending_queue_depth_per_agent.to_string(),
                expected_range: "16-4096",
            });
        }

        if self.sync_drift_budget_us < 1 || self.sync_drift_budget_us > 100_000 {
            errors.push(TimingConfigError {
                field: "sync_drift_budget_us",
                got: self.sync_drift_budget_us.to_string(),
                expected_range: "1-100000",
            });
        }

        if self.tile_stale_threshold_ms < 500 || self.tile_stale_threshold_ms > 300_000 {
            errors.push(TimingConfigError {
                field: "tile_stale_threshold_ms",
                got: self.tile_stale_threshold_ms.to_string(),
                expected_range: "500-300000",
            });
        }

        if self.clock_jump_detection_ms < 10 || self.clock_jump_detection_ms > 10_000 {
            errors.push(TimingConfigError {
                field: "clock_jump_detection_ms",
                got: self.clock_jump_detection_ms.to_string(),
                expected_range: "10-10000",
            });
        }

        errors
    }

    /// Returns `true` if all fields are within their valid ranges.
    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }

    /// Frame period in microseconds derived from `target_fps`.
    ///
    /// Panics if `target_fps == 0`; use after successful [`validate`].
    pub fn frame_period_us(&self) -> u64 {
        1_000_000 / self.target_fps as u64
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Default values ──

    #[test]
    fn default_config_is_valid() {
        let cfg = TimingConfig::default();
        assert!(
            cfg.is_valid(),
            "default config must be valid: {:?}",
            cfg.validate()
        );
    }

    #[test]
    fn default_target_fps() {
        assert_eq!(TimingConfig::default().target_fps, 60);
    }

    #[test]
    fn default_max_agent_clock_drift_ms() {
        assert_eq!(TimingConfig::default().max_agent_clock_drift_ms, 100);
    }

    #[test]
    fn default_max_vsync_jitter_ms() {
        assert_eq!(TimingConfig::default().max_vsync_jitter_ms, 2);
    }

    #[test]
    fn default_max_future_schedule_us() {
        assert_eq!(TimingConfig::default().max_future_schedule_us, 300_000_000);
    }

    #[test]
    fn default_sync_group_max_defer_frames() {
        assert_eq!(TimingConfig::default().sync_group_max_defer_frames, 3);
    }

    #[test]
    fn default_pending_queue_depth_per_agent() {
        assert_eq!(TimingConfig::default().pending_queue_depth_per_agent, 256);
    }

    #[test]
    fn default_sync_drift_budget_us() {
        assert_eq!(TimingConfig::default().sync_drift_budget_us, 500);
    }

    #[test]
    fn default_tile_stale_threshold_ms() {
        assert_eq!(TimingConfig::default().tile_stale_threshold_ms, 5_000);
    }

    #[test]
    fn default_clock_jump_detection_ms() {
        assert_eq!(TimingConfig::default().clock_jump_detection_ms, 50);
    }

    // ── Range validation ──

    /// WHEN target_fps = 0, THEN reject (spec lines 396-398).
    #[test]
    fn target_fps_zero_rejected() {
        let mut cfg = TimingConfig::default();
        cfg.target_fps = 0;
        let errors = cfg.validate();
        assert!(
            errors.iter().any(|e| e.field == "target_fps"),
            "target_fps=0 should be rejected"
        );
    }

    #[test]
    fn target_fps_241_rejected() {
        let mut cfg = TimingConfig::default();
        cfg.target_fps = 241;
        assert!(cfg.validate().iter().any(|e| e.field == "target_fps"));
    }

    #[test]
    fn target_fps_boundary_values_accepted() {
        let mut cfg = TimingConfig::default();
        cfg.target_fps = 1;
        assert!(cfg.is_valid());
        cfg.target_fps = 240;
        assert!(cfg.is_valid());
    }

    #[test]
    fn max_vsync_jitter_zero_is_valid() {
        let mut cfg = TimingConfig::default();
        cfg.max_vsync_jitter_ms = 0;
        assert!(cfg.is_valid(), "0 is valid for max_vsync_jitter_ms");
    }

    #[test]
    fn max_vsync_jitter_101_rejected() {
        let mut cfg = TimingConfig::default();
        cfg.max_vsync_jitter_ms = 101;
        assert!(
            cfg.validate()
                .iter()
                .any(|e| e.field == "max_vsync_jitter_ms")
        );
    }

    #[test]
    fn max_future_schedule_too_small_rejected() {
        let mut cfg = TimingConfig::default();
        cfg.max_future_schedule_us = 999_999;
        assert!(
            cfg.validate()
                .iter()
                .any(|e| e.field == "max_future_schedule_us")
        );
    }

    #[test]
    fn max_future_schedule_too_large_rejected() {
        let mut cfg = TimingConfig::default();
        cfg.max_future_schedule_us = 3_600_000_001;
        assert!(
            cfg.validate()
                .iter()
                .any(|e| e.field == "max_future_schedule_us")
        );
    }

    #[test]
    fn pending_queue_depth_too_small_rejected() {
        let mut cfg = TimingConfig::default();
        cfg.pending_queue_depth_per_agent = 15;
        assert!(
            cfg.validate()
                .iter()
                .any(|e| e.field == "pending_queue_depth_per_agent")
        );
    }

    #[test]
    fn pending_queue_depth_too_large_rejected() {
        let mut cfg = TimingConfig::default();
        cfg.pending_queue_depth_per_agent = 4097;
        assert!(
            cfg.validate()
                .iter()
                .any(|e| e.field == "pending_queue_depth_per_agent")
        );
    }

    #[test]
    fn multiple_errors_collected() {
        // target_fps AND max_vsync_jitter both invalid
        let cfg = TimingConfig {
            target_fps: 0,
            max_vsync_jitter_ms: 200,
            ..TimingConfig::default()
        };
        let errors = cfg.validate();
        assert!(
            errors.len() >= 2,
            "expected at least 2 errors, got: {errors:?}"
        );
    }

    // ── Frame period ──

    #[test]
    fn frame_period_us_at_60fps() {
        let cfg = TimingConfig::default();
        assert_eq!(cfg.frame_period_us(), 16_666);
    }

    #[test]
    fn frame_period_us_at_120fps() {
        let mut cfg = TimingConfig::default();
        cfg.target_fps = 120;
        assert_eq!(cfg.frame_period_us(), 8_333);
    }
}
