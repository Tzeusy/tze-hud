//! Clock drift detection, estimation, and enforcement.
//!
//! # Spec alignment
//!
//! - `§Requirement: Clock Drift Detection and Correction` (lines 210-221)
//! - `§Requirement: Clock Drift Enforcement` (lines 223-234)
//! - `§Requirement: Session Clock Sync Point` (lines 41-48)
//! - `§Requirement: ClockSync RPC` (lines 382-389)
//! - `§Requirement: Safe Mode Timing Behavior` — estimation window frozen
//!   during safe mode (lines 343-358)
//!
//! ## Clock drift estimation
//!
//! The compositor maintains a **sliding window of the last 32 agent
//! timestamps** (`agent_ts - compositor_ts`). The **median** of this window
//! is used as the signed skew estimate.
//!
//! When the absolute difference between consecutive samples exceeds
//! `clock_jump_detection_ms` (default 50ms), the window is **reset** to the
//! current single sample.
//!
//! ## Enforcement tiers
//!
//! | Drift | Action |
//! |---|---|
//! | <= 100ms | Apply correction transparently; no warning |
//! | 100ms – 1s | Apply correction; emit `CLOCK_SKEW_HIGH` warning |
//! | > 1s | Reject mutation with `CLOCK_SKEW_EXCESSIVE` |
//! | 3 consecutive ClockSync failures | Terminate session |

use crate::timing::domains::{MonoUs, WallUs};
use crate::timing::errors::{TimingError, TimingWarning};
use crate::timing::scheduling::{
    CLOCK_SKEW_EXCESSIVE_THRESHOLD_US, CLOCK_SKEW_HIGH_THRESHOLD_US,
};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Sliding window size for clock-skew estimation.
pub const CLOCK_DRIFT_WINDOW_SIZE: usize = 32;

/// Default clock-jump detection threshold in microseconds (50ms).
pub const DEFAULT_CLOCK_JUMP_DETECTION_US: u64 = 50_000;

// ─── SessionClockSync ────────────────────────────────────────────────────────

/// Timestamps recorded at session open (spec lines 41-48).
#[derive(Clone, Debug)]
pub struct SessionClockSync {
    /// Compositor monotonic clock at session establishment.
    pub session_open_mono_us: MonoUs,
    /// Compositor wall-clock at session establishment.
    pub session_open_wall_us: WallUs,
    /// Initial clock-skew estimate (wall - mono at session open).
    pub initial_skew_us: i64,
}

impl SessionClockSync {
    /// Record the session open timestamp pair and compute initial skew.
    ///
    /// From spec §Requirement: Session Clock Sync Point (lines 46-48):
    /// `initial_skew = session_open_wall_us - session_open_mono_us`.
    pub fn new(session_open_mono_us: MonoUs, session_open_wall_us: WallUs) -> Self {
        let initial_skew_us =
            (session_open_wall_us.as_u64() as i128 - session_open_mono_us.as_u64() as i128)
                .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        Self {
            session_open_mono_us,
            session_open_wall_us,
            initial_skew_us,
        }
    }
}

// ─── ClockDriftEstimator ─────────────────────────────────────────────────────

/// Sliding-window median clock-skew estimator.
///
/// Maintains the last `CLOCK_DRIFT_WINDOW_SIZE` (32) skew samples.
/// The current estimate is the **median** of the window.
///
/// # Jump detection
///
/// If the new sample differs from the previous sample by more than
/// `clock_jump_detection_us`, the window is reset to contain only the new
/// sample.
///
/// # Safe mode
///
/// Call [`ClockDriftEstimator::freeze_estimation`] during safe mode and
/// [`ClockDriftEstimator::reset_window`] on exit.
#[derive(Clone, Debug)]
pub struct ClockDriftEstimator {
    /// Ring-buffer of the last N skew samples (microseconds).
    samples: [i64; CLOCK_DRIFT_WINDOW_SIZE],
    /// Number of valid samples in `samples` (0–32).
    count: usize,
    /// Write position (next slot to overwrite).
    head: usize,
    /// Last sample added (for jump detection).
    last_sample: Option<i64>,
    /// Clock-jump detection threshold in microseconds.
    clock_jump_detection_us: u64,
    /// When `true`, new samples are ignored (safe mode).
    frozen: bool,
}

impl ClockDriftEstimator {
    /// Create a new estimator.
    ///
    /// `clock_jump_detection_us` should be
    /// `TimingConfig::clock_jump_detection_ms * 1000`.
    pub fn new(clock_jump_detection_us: u64) -> Self {
        Self {
            samples: [0i64; CLOCK_DRIFT_WINDOW_SIZE],
            count: 0,
            head: 0,
            last_sample: None,
            clock_jump_detection_us,
            frozen: false,
        }
    }

    /// Add a new skew sample.
    ///
    /// `agent_wall_us` is the agent-supplied timestamp;
    /// `compositor_wall_us` is the compositor's current wall clock.
    ///
    /// Returns the **updated estimate** (median of window after insertion).
    ///
    /// If the estimator is frozen (safe mode), the sample is ignored and the
    /// current estimate is returned unchanged.
    pub fn push_sample(&mut self, agent_wall_us: u64, compositor_wall_us: u64) -> i64 {
        if self.frozen {
            return self.estimate();
        }

        let sample = agent_wall_us as i128 - compositor_wall_us as i128;
        let sample = sample.clamp(i64::MIN as i128, i64::MAX as i128) as i64;

        // Jump detection: if previous sample exists and difference > threshold → reset.
        if let Some(prev) = self.last_sample {
            let diff = (sample - prev).unsigned_abs();
            if diff > self.clock_jump_detection_us {
                self.count = 0;
                self.head = 0;
                // Spec: reset window to current single sample.
            }
        }

        self.samples[self.head] = sample;
        self.head = (self.head + 1) % CLOCK_DRIFT_WINDOW_SIZE;
        self.count = (self.count + 1).min(CLOCK_DRIFT_WINDOW_SIZE);
        self.last_sample = Some(sample);

        self.estimate()
    }

    /// Current skew estimate (median of window).
    ///
    /// Returns `0` if the window is empty.
    pub fn estimate(&self) -> i64 {
        if self.count == 0 {
            return 0;
        }
        let mut sorted: Vec<i64> = self.samples[..self.count].to_vec();
        sorted.sort_unstable();
        let mid = self.count / 2;
        if self.count % 2 == 0 {
            // Average of two middle values computed in i128 to avoid precision
            // loss and potential overflow with large skew magnitudes.
            let a = sorted[mid - 1] as i128;
            let b = sorted[mid] as i128;
            let avg = (a + b) / 2;
            avg.clamp(i64::MIN as i128, i64::MAX as i128) as i64
        } else {
            sorted[mid]
        }
    }

    /// Reset the estimation window (called on safe mode exit).
    ///
    /// From spec §Requirement: Safe Mode Timing Behavior (lines 357-358):
    /// each session's clock-skew estimation window MUST be reset to empty.
    pub fn reset_window(&mut self) {
        self.count = 0;
        self.head = 0;
        self.last_sample = None;
        self.frozen = false;
    }

    /// Freeze new sample ingestion (called during safe mode).
    ///
    /// The current estimate remains but no new samples are accepted.
    pub fn freeze_estimation(&mut self) {
        self.frozen = true;
    }

    /// Unfreeze (alias for `reset_window` — safe mode exit resets the window).
    pub fn unfreeze_and_reset(&mut self) {
        self.reset_window();
    }

    /// Returns `true` if the estimator is frozen.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Number of samples in the current window.
    pub fn sample_count(&self) -> usize {
        self.count
    }
}

// ─── ClockSyncRequest / Response ─────────────────────────────────────────────

/// Request payload for the `ClockSync` unary RPC.
///
/// From spec §Requirement: ClockSync RPC (lines 382-389).
#[derive(Clone, Debug)]
pub struct ClockSyncRequest {
    /// Agent-supplied current wall-clock time (UTC µs since epoch).
    pub agent_timestamp_wall_us: WallUs,
}

/// Response payload for the `ClockSync` unary RPC.
///
/// From spec §Requirement: ClockSync RPC (lines 382-389).
#[derive(Clone, Debug)]
pub struct ClockSyncResponse {
    /// Compositor monotonic clock at RPC handling time.
    pub compositor_mono_us: MonoUs,
    /// Compositor wall-clock at RPC handling time.
    pub compositor_wall_us: WallUs,
    /// Estimated skew: `agent_timestamp_wall_us - compositor_wall_us` (signed).
    /// Positive = agent is ahead.
    pub estimated_skew_us: i64,
    /// `true` if `|estimated_skew_us| <= max_agent_clock_drift_us` (100ms by
    /// default).
    pub skew_within_tolerance: bool,
    /// Optional non-fatal warning (`CLOCK_SKEW_HIGH`).
    pub warning: Option<TimingWarning>,
}

/// Handle a `ClockSync` RPC given the compositor's current clocks and the
/// existing drift estimate.
///
/// Updates the estimator and returns a `ClockSyncResponse`.
///
/// From spec (lines 387-389): the compositor MUST return its current monotonic
/// and wall clock values and the estimated skew.
pub fn handle_clock_sync(
    req: &ClockSyncRequest,
    compositor_mono_us: MonoUs,
    compositor_wall_us: WallUs,
    estimator: &mut ClockDriftEstimator,
) -> Result<ClockSyncResponse, TimingError> {
    let estimated_skew_us =
        estimator.push_sample(req.agent_timestamp_wall_us.as_u64(), compositor_wall_us.as_u64());
    let abs_skew = estimated_skew_us.unsigned_abs();

    // CLOCK_SKEW_EXCESSIVE — reject
    if abs_skew > CLOCK_SKEW_EXCESSIVE_THRESHOLD_US {
        return Err(TimingError::ClockSkewExcessive);
    }

    let warning = if abs_skew > CLOCK_SKEW_HIGH_THRESHOLD_US {
        Some(TimingWarning::ClockSkewHigh { estimated_skew_us })
    } else {
        None
    };

    Ok(ClockSyncResponse {
        compositor_mono_us,
        compositor_wall_us,
        estimated_skew_us,
        skew_within_tolerance: abs_skew <= CLOCK_SKEW_HIGH_THRESHOLD_US,
        warning,
    })
}

// ─── VsyncSyncPoint ───────────────────────────────────────────────────────────

/// Per-frame vsync sync point triple.
///
/// From spec §Requirement: Vsync Sync Point (lines 32-39).
///
/// At the start of each frame the compositor records this triple and includes
/// it in the `FrameTimingRecord`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VsyncSyncPoint {
    /// Frame number (monotonically increasing from session start).
    pub frame_number: u64,
    /// Monotonic clock at vsync.
    pub vsync_mono_us: MonoUs,
    /// Wall-clock at vsync (sampled once and cached per frame).
    pub vsync_wall_us: WallUs,
}

impl VsyncSyncPoint {
    /// Create a new vsync sync point.
    pub fn new(frame_number: u64, vsync_mono_us: MonoUs, vsync_wall_us: WallUs) -> Self {
        Self {
            frame_number,
            vsync_mono_us,
            vsync_wall_us,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SessionClockSync ──

    #[test]
    fn session_clock_sync_initial_skew() {
        let sync = SessionClockSync::new(MonoUs(1_000_000), WallUs(1_050_000)); // wall is 50ms ahead
        assert_eq!(sync.initial_skew_us, 50_000);
    }

    #[test]
    fn session_clock_sync_negative_skew() {
        let sync = SessionClockSync::new(MonoUs(1_100_000), WallUs(1_000_000)); // mono is 100ms ahead
        assert_eq!(sync.initial_skew_us, -100_000);
    }

    // ── ClockDriftEstimator ──

    #[test]
    fn estimator_single_sample() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let skew = est.push_sample(1_050_000, 1_000_000); // +50ms
        assert_eq!(skew, 50_000);
    }

    #[test]
    fn estimator_multiple_samples_median() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let base = 1_000_000_000_u64;
        // Skews: 10ms, 20ms, 30ms → median = 20ms
        est.push_sample(base + 10_000, base);
        est.push_sample(base + 20_000, base);
        est.push_sample(base + 30_000, base);
        let estimate = est.estimate();
        assert_eq!(estimate, 20_000, "median of [10k, 20k, 30k] should be 20k");
    }

    /// WHEN consecutive samples differ by > 50ms THEN window resets.
    #[test]
    fn clock_jump_resets_window() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let base = 1_000_000_000_u64;

        // 3 stable samples ~+10ms
        for _ in 0..3 {
            est.push_sample(base + 10_000, base);
        }
        assert_eq!(est.sample_count(), 3);

        // Large jump: new sample at +200ms (200ms - 10ms = 190ms difference > 50ms)
        est.push_sample(base + 200_000, base);
        // Window reset → only 1 sample
        assert_eq!(
            est.sample_count(),
            1,
            "window should reset after jump > 50ms"
        );
        assert_eq!(est.estimate(), 200_000);
    }

    /// WHEN window has 32 samples, new sample overwrites oldest (ring buffer).
    #[test]
    fn estimator_wraps_at_32_samples() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let base = 1_000_000_000_u64;
        for i in 0..33u64 {
            est.push_sample(base + 5_000 + i, base);
        }
        assert_eq!(est.sample_count(), CLOCK_DRIFT_WINDOW_SIZE);
    }

    // ── Safe mode: estimation window frozen ──

    #[test]
    fn frozen_estimator_ignores_new_samples() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let base = 1_000_000_000_u64;
        est.push_sample(base + 10_000, base); // +10ms
        let before = est.estimate();
        est.freeze_estimation();
        est.push_sample(base + 500_000, base); // +500ms — ignored
        assert_eq!(est.estimate(), before, "frozen estimator must not update");
    }

    #[test]
    fn reset_window_clears_samples() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let base = 1_000_000_000_u64;
        est.push_sample(base + 10_000, base);
        assert_eq!(est.sample_count(), 1);
        est.reset_window();
        assert_eq!(est.sample_count(), 0);
        assert_eq!(est.estimate(), 0);
    }

    // ── ClockSync RPC ──

    #[test]
    fn clock_sync_returns_skew_within_tolerance() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let req = ClockSyncRequest {
            agent_timestamp_wall_us: WallUs(1_000_050_000), // +50ms
        };
        let resp = handle_clock_sync(
            &req,
            MonoUs(1_000_000_000),
            WallUs(1_000_000_000),
            &mut est,
        )
        .unwrap();
        assert!(resp.skew_within_tolerance);
        assert_eq!(resp.estimated_skew_us, 50_000);
        assert!(resp.warning.is_none());
    }

    #[test]
    fn clock_sync_warns_on_high_skew() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let req = ClockSyncRequest {
            agent_timestamp_wall_us: WallUs(1_000_200_000), // +200ms
        };
        let resp = handle_clock_sync(
            &req,
            MonoUs(1_000_000_000),
            WallUs(1_000_000_000),
            &mut est,
        )
        .unwrap();
        assert!(!resp.skew_within_tolerance);
        assert!(resp.warning.is_some());
    }

    #[test]
    fn clock_sync_rejects_excessive_skew() {
        let mut est = ClockDriftEstimator::new(DEFAULT_CLOCK_JUMP_DETECTION_US);
        let req = ClockSyncRequest {
            agent_timestamp_wall_us: WallUs(1_002_000_000), // +2s
        };
        let err = handle_clock_sync(
            &req,
            MonoUs(1_000_000_000),
            WallUs(1_000_000_000),
            &mut est,
        )
        .unwrap_err();
        assert_eq!(err, TimingError::ClockSkewExcessive);
    }

    // ── VsyncSyncPoint ──

    #[test]
    fn vsync_sync_point_stores_triple() {
        let sp = VsyncSyncPoint::new(42, MonoUs(1_000_000), WallUs(2_000_000));
        assert_eq!(sp.frame_number, 42);
        assert_eq!(sp.vsync_mono_us, MonoUs(1_000_000));
        assert_eq!(sp.vsync_wall_us, WallUs(2_000_000));
    }
}
