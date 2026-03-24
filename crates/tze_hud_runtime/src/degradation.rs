//! 6-level degradation state machine for the tze_hud runtime.
//!
//! Implements the degradation ladder from RFC 0002 §6 as specified in
//! runtime-kernel/spec.md:
//!
//! - Requirement: Degradation Ladder (line 220) — 6 levels from Normal to Emergency
//! - Requirement: Degradation Trigger (line 237) — p95 > 14ms over 10-frame window
//! - Requirement: Degradation Hysteresis (line 250) — p95 < 12ms over 30-frame window
//! - Requirement: Tile Shedding Order (line 263) — (lease_priority ASC, z_order DESC)
//!
//! ## Performance Characteristics
//!
//! - Frame-time sample recording: O(1) amortized (ring buffer push/pop)
//! - p95 evaluation: O(N) where N ≤ 30 (window size) — well within < 100µs budget
//! - Tile shedding sort: O(n log n) where n = tile count (< 1ms p99 for v1 limits)
//!
//! ## Threading Model
//!
//! `DegradationController` is single-threaded. Only the compositor thread calls
//! it, always from within the frame loop after the frame completes.

use std::collections::VecDeque;

use tze_hud_scene::types::SceneId;
use tze_hud_telemetry::{DegradationDirection, DegradationEvent};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Number of frames in the trigger rolling window (≈166ms at 60fps).
const TRIGGER_WINDOW: usize = 10;

/// Number of frames in the recovery rolling window (≈500ms at 60fps).
const RECOVERY_WINDOW: usize = 30;

/// Trigger threshold: frame_time_p95 must exceed this to advance a level (µs).
const TRIGGER_THRESHOLD_US: u64 = 14_000; // 14ms

/// Recovery threshold: frame_time_p95 must be below this to recover a level (µs).
const RECOVERY_THRESHOLD_US: u64 = 12_000; // 12ms

// ─── Degradation Level ────────────────────────────────────────────────────────

/// The 6 degradation levels from RFC 0002 §6.2.
///
/// Levels are ordered: Normal (0) is best, Emergency (5) is worst.
/// The runtime advances one level at a time (trigger) and recovers one level
/// at a time (hysteresis). To go from Level 5 to Normal requires 5 successive
/// 30-frame clean windows (~2.5 seconds at 60fps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DegradationLevel {
    /// Level 0 — Full quality rendering. No restrictions.
    Normal = 0,
    /// Level 1 — Coalesce: reduce outbound SceneEvent frequency for
    /// state-stream tiles by the configured coalesce ratio (default 2×).
    Coalesce = 1,
    /// Level 2 — ReduceTextureQuality: scale down textures whose linear
    /// dimensions exceed 512px by the configured scale factor (default 50%).
    ReduceTextureQuality = 2,
    /// Level 3 — DisableTransparency: force semi-transparent tiles to opaque;
    /// skip alpha-blend render passes entirely.
    DisableTransparency = 3,
    /// Level 4 — ShedTiles: remove lowest-priority tiles from the render pass
    /// (sorted by lease_priority ASC, z_order DESC). Tiles remain in the scene
    /// graph but are not rendered.
    ShedTiles = 4,
    /// Level 5 — Emergency: render only the chrome layer plus the single
    /// highest-priority tile. All other tiles are visually suppressed.
    Emergency = 5,
}

impl DegradationLevel {
    /// Return the level one step worse, clamped at Emergency.
    fn advance(self) -> Self {
        match self {
            Self::Normal => Self::Coalesce,
            Self::Coalesce => Self::ReduceTextureQuality,
            Self::ReduceTextureQuality => Self::DisableTransparency,
            Self::DisableTransparency => Self::ShedTiles,
            Self::ShedTiles => Self::Emergency,
            Self::Emergency => Self::Emergency,
        }
    }

    /// Return the level one step better, clamped at Normal.
    fn recover(self) -> Self {
        match self {
            Self::Normal => Self::Normal,
            Self::Coalesce => Self::Normal,
            Self::ReduceTextureQuality => Self::Coalesce,
            Self::DisableTransparency => Self::ReduceTextureQuality,
            Self::ShedTiles => Self::DisableTransparency,
            Self::Emergency => Self::ShedTiles,
        }
    }

    /// Whether this level is Normal.
    pub fn is_normal(self) -> bool {
        self == Self::Normal
    }

    /// Numeric representation matching the spec (0–5).
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl std::fmt::Display for DegradationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::Coalesce => write!(f, "Coalesce"),
            Self::ReduceTextureQuality => write!(f, "ReduceTextureQuality"),
            Self::DisableTransparency => write!(f, "DisableTransparency"),
            Self::ShedTiles => write!(f, "ShedTiles"),
            Self::Emergency => write!(f, "Emergency"),
        }
    }
}

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configurable parameters for the degradation controller.
///
/// All ratios/factors are validated defaults from spec Implementation Notes
/// (line 415–417). Operators may override via runtime configuration.
#[derive(Clone, Debug)]
pub struct DegradationConfig {
    /// Level 1: reduce outbound SceneEvent frequency by this divisor.
    /// Default: 2 (one notification per two frames).
    pub coalesce_ratio: u32,

    /// Level 2: textures with linear dimensions exceeding this value (pixels)
    /// are scaled down by `texture_scale_factor`.
    /// Default: 512.
    pub texture_quality_threshold_px: u32,

    /// Level 2: scale factor applied to large textures (as a fraction of 1.0).
    /// Default: 0.5 (50% reduction).
    pub texture_scale_factor: f32,
}

impl Default for DegradationConfig {
    fn default() -> Self {
        Self {
            coalesce_ratio: 2,
            texture_quality_threshold_px: 512,
            texture_scale_factor: 0.5,
        }
    }
}

// ─── Tile descriptor ──────────────────────────────────────────────────────────

/// A tile descriptor used for shedding decisions.
///
/// `lease_priority`: 0 = highest priority (never shed first).
/// `z_order`: higher value = painted on top = more important.
///
/// Shedding order: (lease_priority DESC numerically, z_order ASC) — i.e. we
/// shed the tile with the *largest* lease_priority number first, and within
/// the same priority class we shed the one with the *smallest* z_order first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileDescriptor {
    pub tile_id: SceneId,
    pub lease_priority: u32,
    pub z_order: u32,
}

// ─── Degradation Controller ───────────────────────────────────────────────────

/// Rolling-window degradation state machine.
///
/// Call [`DegradationController::record_frame`] once per frame after the frame
/// completes. The controller evaluates trigger and recovery conditions and
/// advances or recovers the degradation level as needed.
///
/// Query [`DegradationController::level`] to determine what restrictions should
/// be applied to the current frame.
pub struct DegradationController {
    /// Current degradation level.
    level: DegradationLevel,

    /// Ring buffer of recent frame times (µs), capacity = RECOVERY_WINDOW.
    ///
    /// We keep the longer window (30 frames) because it subsumes the shorter
    /// (10 frames). The p95 over the last N entries gives the rolling window.
    frame_times: VecDeque<u64>,

    /// Number of consecutive 30-frame windows where p95 < 12ms.
    /// Used for the full-recovery path from Level 5.
    clean_recovery_windows: u32,

    /// Running frame count within the current recovery window.
    frames_in_current_window: u32,

    /// Configuration.
    config: DegradationConfig,

    /// Monotonically increasing frame counter for telemetry.
    frame_number: u64,
}

impl DegradationController {
    /// Create a new controller starting at Normal.
    pub fn new(config: DegradationConfig) -> Self {
        Self {
            level: DegradationLevel::Normal,
            frame_times: VecDeque::with_capacity(RECOVERY_WINDOW + 1),
            clean_recovery_windows: 0,
            frames_in_current_window: 0,
            config,
            frame_number: 0,
        }
    }

    /// Create a controller with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(DegradationConfig::default())
    }

    /// The current degradation level.
    pub fn level(&self) -> DegradationLevel {
        self.level
    }

    /// The current configuration.
    pub fn config(&self) -> &DegradationConfig {
        &self.config
    }

    /// Record a completed frame's time (in microseconds) and evaluate
    /// trigger / recovery conditions.
    ///
    /// Returns `Some(DegradationEvent)` if the level changed this frame, or
    /// `None` if the level is unchanged.
    ///
    /// This method MUST be called exactly once after every frame completes,
    /// so that the rolling windows advance correctly.
    ///
    /// ## Window semantics
    ///
    /// The controller maintains a single ring buffer of recent frame times
    /// (capacity = RECOVERY_WINDOW = 30). After any level change (advance OR
    /// recover) the buffer is cleared so that the next evaluation window
    /// starts with fresh observations. This prevents a single burst of high-
    /// latency frames from causing cascading multi-level advances in a single
    /// cycle, and ensures that the recovery window only counts frames observed
    /// AFTER the most recent level change.
    pub fn record_frame(&mut self, frame_time_us: u64) -> Option<DegradationEvent> {
        self.frame_number += 1;

        // ── Maintain ring buffer ──────────────────────────────────────────────
        //
        // We store up to RECOVERY_WINDOW (30) samples. When the buffer is full,
        // the oldest sample is evicted (the VecDeque acts as a ring buffer).
        if self.frame_times.len() >= RECOVERY_WINDOW {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(frame_time_us);

        // Track frames within the current 30-frame recovery window.
        self.frames_in_current_window += 1;

        let old_level = self.level;

        // ── Trigger: advance one level ────────────────────────────────────────
        //
        // Spec: trigger fires when p95 > 14ms over the 10-frame window.
        // We only trigger if we have accumulated at least TRIGGER_WINDOW samples
        // since the last level change.
        if self.level < DegradationLevel::Emergency
            && self.frame_times.len() >= TRIGGER_WINDOW
        {
            let p95_trigger = p95_of_last_n(&self.frame_times, TRIGGER_WINDOW);
            if p95_trigger > TRIGGER_THRESHOLD_US {
                self.level = self.level.advance();
                // Clear the ring buffer and reset window tracking.
                // New observations start from scratch after a level change,
                // preventing the same high-latency frames from causing
                // cascading advances or polluting the recovery window.
                self.frame_times.clear();
                self.clean_recovery_windows = 0;
                self.frames_in_current_window = 0;
                return Some(DegradationEvent {
                    frame_number: self.frame_number,
                    previous_level: old_level.as_u8(),
                    new_level: self.level.as_u8(),
                    frame_time_p95_us: p95_trigger,
                    direction: DegradationDirection::Advance,
                });
            }
        }

        // ── Recovery: recover one level ───────────────────────────────────────
        //
        // Spec: recovery requires p95 < 12ms sustained over a 30-frame window.
        // For Level 5, full recovery to Normal requires 5 such successive windows
        // (~2.5 seconds total at 60fps).
        if self.level > DegradationLevel::Normal
            && self.frames_in_current_window >= RECOVERY_WINDOW as u32
        {
            // A full 30-frame window has elapsed — evaluate it.
            let p95_recovery = p95_of_last_n(&self.frame_times, RECOVERY_WINDOW);

            if p95_recovery < RECOVERY_THRESHOLD_US {
                self.clean_recovery_windows += 1;
                // Always recover one level per clean window.
                self.level = self.level.recover();
                // Clear buffer and reset window tracking after level change.
                self.frame_times.clear();
                self.frames_in_current_window = 0;
                return Some(DegradationEvent {
                    frame_number: self.frame_number,
                    previous_level: old_level.as_u8(),
                    new_level: self.level.as_u8(),
                    frame_time_p95_us: p95_recovery,
                    direction: DegradationDirection::Recover,
                });
            } else {
                // Dirty window — reset clean window counter and start fresh.
                self.clean_recovery_windows = 0;
                self.frames_in_current_window = 0;
            }
        }

        None
    }

    /// Determine which tiles to suppress in the render pass at Level 4+ shedding.
    ///
    /// Returns the set of tile IDs that should be excluded from the render pass.
    /// Tiles remain in the scene graph; they are simply not presented.
    ///
    /// At Level 4 (ShedTiles), returns lowest-priority tiles sorted by
    /// (lease_priority ASC, z_order DESC) — i.e., highest lease_priority value
    /// is shed first; within a priority class, lowest z_order is shed first.
    ///
    /// At Level 5 (Emergency), returns all tiles except the single tile with
    /// the lowest lease_priority value (and, as a tiebreaker, the highest
    /// z_order within that priority class).
    ///
    /// For all other levels, returns an empty vec.
    ///
    /// The chrome layer is always preserved (it is never in `tiles`).
    ///
    /// # Complexity
    ///
    /// O(n log n) where n = tile count. Satisfies the < 1ms p99 budget for
    /// v1 tile limits (max 64 tiles per agent × number of agents).
    pub fn shed_tiles<'a>(&self, tiles: &'a [TileDescriptor]) -> Vec<&'a TileDescriptor> {
        match self.level {
            DegradationLevel::Normal
            | DegradationLevel::Coalesce
            | DegradationLevel::ReduceTextureQuality
            | DegradationLevel::DisableTransparency => {
                // No shedding at levels 0–3.
                vec![]
            }
            DegradationLevel::ShedTiles => {
                // Sort tiles by shedding priority: shed highest-lease_priority first.
                // Within a priority class, shed lowest z_order first.
                //
                // Result ordering (front = shed first):
                //   sort key: (lease_priority DESC, z_order ASC)
                let mut indexed: Vec<(usize, &TileDescriptor)> =
                    tiles.iter().enumerate().collect();
                indexed.sort_by(|(_, a), (_, b)| {
                    // Higher lease_priority number = lower importance = shed first.
                    b.lease_priority
                        .cmp(&a.lease_priority)
                        .then(a.z_order.cmp(&b.z_order))
                });
                // At Level 4, shed the lowest-priority half (at least 1).
                // For v1, we shed 25% consistent with the frame-time guardian.
                let shed_count = ((tiles.len() as f32 * 0.25).ceil() as usize).max(1);
                indexed
                    .into_iter()
                    .take(shed_count)
                    .map(|(_, t)| t)
                    .collect()
            }
            DegradationLevel::Emergency => {
                // Keep only the single highest-priority tile (lease_priority=0
                // and highest z_order within that class). Suppress everything else.
                if tiles.is_empty() {
                    return vec![];
                }
                // Find the "best" tile: smallest lease_priority, then largest z_order.
                let keep = tiles
                    .iter()
                    .min_by(|a, b| {
                        a.lease_priority
                            .cmp(&b.lease_priority)
                            .then(b.z_order.cmp(&a.z_order)) // higher z_order wins
                    })
                    .expect("non-empty slice always has a min");

                // Everything except `keep` is shed.
                tiles
                    .iter()
                    .filter(|t| t.tile_id != keep.tile_id)
                    .collect()
            }
        }
    }

    /// Compute the Level 1 coalesce: whether a notification should be sent for
    /// the current frame.
    ///
    /// Returns `true` if a state-stream notification should be emitted this
    /// frame (i.e., this frame is "on beat"). Returns `false` if the notification
    /// should be suppressed.
    ///
    /// At Level 0, always returns `true`. At Level 1+, returns `true` every
    /// `coalesce_ratio` frames.
    pub fn should_emit_state_stream(&self) -> bool {
        if self.level < DegradationLevel::Coalesce {
            return true;
        }
        let ratio = self.config.coalesce_ratio.max(1) as u64;
        self.frame_number.is_multiple_of(ratio)
    }

    /// Whether textures exceeding the threshold should be downscaled.
    ///
    /// Returns `true` at Level 2+.
    pub fn should_reduce_texture_quality(&self) -> bool {
        self.level >= DegradationLevel::ReduceTextureQuality
    }

    /// Whether alpha-blending should be disabled (all semi-transparent tiles
    /// forced to opaque).
    ///
    /// Returns `true` at Level 3+.
    pub fn should_disable_transparency(&self) -> bool {
        self.level >= DegradationLevel::DisableTransparency
    }

    /// The texture scale factor to apply when [`should_reduce_texture_quality`]
    /// is true. Dimensions exceeding `texture_quality_threshold_px` are scaled
    /// by this factor.
    pub fn texture_scale_factor(&self) -> f32 {
        self.config.texture_scale_factor
    }

    /// The texture dimension threshold (pixels) above which scaling is applied.
    pub fn texture_quality_threshold_px(&self) -> u32 {
        self.config.texture_quality_threshold_px
    }

    /// Number of consecutive frames evaluated so far (for testing / telemetry).
    pub fn frame_number(&self) -> u64 {
        self.frame_number
    }

    /// Number of successive clean 30-frame recovery windows counted since the
    /// last advance. Useful for telemetry and testing.
    pub fn clean_recovery_windows(&self) -> u32 {
        self.clean_recovery_windows
    }
}

// ─── p95 helper ───────────────────────────────────────────────────────────────

/// Compute the p95 of the last `n` values in a ring buffer (VecDeque).
///
/// Panics if `n > deque.len()` — callers must guard with `len() >= n`.
///
/// Uses the nearest-rank method (consistent with [`LatencyBucket::percentile`]).
fn p95_of_last_n(deque: &VecDeque<u64>, n: usize) -> u64 {
    debug_assert!(deque.len() >= n, "caller must ensure len() >= n");
    let mut samples: Vec<u64> = deque.iter().rev().take(n).copied().collect();
    samples.sort_unstable();
    let rank = ((95.0_f64 / 100.0) * samples.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(samples.len() - 1);
    samples[idx]
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::types::SceneId;

    fn controller() -> DegradationController {
        DegradationController::with_defaults()
    }

    /// Push `n` frames of `frame_time_us` through the controller without
    /// caring about transition events.
    fn push_frames(ctrl: &mut DegradationController, frame_time_us: u64, n: usize) {
        for _ in 0..n {
            ctrl.record_frame(frame_time_us);
        }
    }

    // ── Level invariants ──────────────────────────────────────────────────────

    #[test]
    fn test_starts_at_normal() {
        let ctrl = controller();
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    #[test]
    fn test_level_advance_chain() {
        assert_eq!(DegradationLevel::Normal.advance(), DegradationLevel::Coalesce);
        assert_eq!(DegradationLevel::Coalesce.advance(), DegradationLevel::ReduceTextureQuality);
        assert_eq!(DegradationLevel::ReduceTextureQuality.advance(), DegradationLevel::DisableTransparency);
        assert_eq!(DegradationLevel::DisableTransparency.advance(), DegradationLevel::ShedTiles);
        assert_eq!(DegradationLevel::ShedTiles.advance(), DegradationLevel::Emergency);
        assert_eq!(DegradationLevel::Emergency.advance(), DegradationLevel::Emergency);
    }

    #[test]
    fn test_level_recover_chain() {
        assert_eq!(DegradationLevel::Normal.recover(), DegradationLevel::Normal);
        assert_eq!(DegradationLevel::Coalesce.recover(), DegradationLevel::Normal);
        assert_eq!(DegradationLevel::ReduceTextureQuality.recover(), DegradationLevel::Coalesce);
        assert_eq!(DegradationLevel::DisableTransparency.recover(), DegradationLevel::ReduceTextureQuality);
        assert_eq!(DegradationLevel::ShedTiles.recover(), DegradationLevel::DisableTransparency);
        assert_eq!(DegradationLevel::Emergency.recover(), DegradationLevel::ShedTiles);
    }

    // ── Trigger: sustained overbudget → advance ───────────────────────────────

    #[test]
    fn test_trigger_advances_level_after_10_frames_over_14ms() {
        let mut ctrl = controller();
        // 10 frames all at 20ms — p95 = 20ms > 14ms → must advance.
        let event = push_and_get_last_event(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert!(event.is_some(), "Expected a degradation advance event");
        let ev = event.unwrap();
        assert_eq!(ev.previous_level, 0); // Normal
        assert_eq!(ev.new_level, 1);      // Coalesce
        assert_eq!(ev.direction, DegradationDirection::Advance);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
    }

    #[test]
    fn test_trigger_requires_full_10_frame_window() {
        let mut ctrl = controller();
        // Only 9 frames — must NOT trigger yet.
        for _ in 0..(TRIGGER_WINDOW - 1) {
            let ev = ctrl.record_frame(20_000);
            assert!(ev.is_none(), "Should not trigger before full 10-frame window");
        }
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
        // 10th frame — now should trigger.
        let ev = ctrl.record_frame(20_000);
        assert!(ev.is_some());
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
    }

    // ── Transient spike tolerance ─────────────────────────────────────────────

    #[test]
    fn test_partial_window_does_not_trigger_degradation() {
        // Spec: trigger requires p95 > 14ms over the 10-frame ROLLING WINDOW.
        // Before accumulating 10 frames, the system MUST NOT trigger,
        // regardless of how large individual frames are.
        let mut ctrl = controller();
        // Push 9 frames well above the threshold — but no trigger yet (window not full).
        for i in 0..9 {
            let ev = ctrl.record_frame(50_000);
            assert!(ev.is_none(), "Frame {}: must not trigger before 10-frame window is full", i);
        }
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    #[test]
    fn test_10th_frame_first_possible_trigger() {
        let mut ctrl = controller();
        push_frames(&mut ctrl, 50_000, 9);
        // The 10th frame is the FIRST point at which the trigger can fire.
        let ev = ctrl.record_frame(50_000);
        assert!(ev.is_some(), "10th frame above threshold should trigger degradation");
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
    }

    #[test]
    fn test_p95_boundary_does_not_trigger_at_exactly_14ms() {
        let mut ctrl = controller();
        // All 10 frames at exactly 14ms — p95 = 14ms, NOT > 14ms. No trigger.
        push_frames(&mut ctrl, TRIGGER_THRESHOLD_US, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    // ── Hysteresis / recovery ─────────────────────────────────────────────────

    #[test]
    fn test_recovery_requires_30_frames_under_12ms() {
        let mut ctrl = controller();
        // Force to Level 1 by triggering.
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);

        // 29 clean frames — must NOT recover yet.
        for i in 0..(RECOVERY_WINDOW - 1) {
            let ev = ctrl.record_frame(5_000);
            assert!(ev.is_none(), "Frame {i}: should not recover before 30 frames");
        }
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);

        // 30th clean frame — should trigger recovery.
        let ev = ctrl.record_frame(5_000);
        assert!(ev.is_some(), "Should recover after 30 clean frames");
        let ev = ev.unwrap();
        assert_eq!(ev.previous_level, 1); // Coalesce
        assert_eq!(ev.new_level, 0);      // Normal
        assert_eq!(ev.direction, DegradationDirection::Recover);
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    #[test]
    fn test_recovery_one_level_at_a_time() {
        let mut ctrl = controller();
        // Advance twice (to Level 2 = ReduceTextureQuality).
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::ReduceTextureQuality);

        // First clean window of 30 frames → recover to Level 1.
        push_frames(&mut ctrl, 5_000, RECOVERY_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);

        // Second clean window of 30 frames → recover to Level 0.
        push_frames(&mut ctrl, 5_000, RECOVERY_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
    }

    #[test]
    fn test_recovery_threshold_exactly_12ms_does_not_recover() {
        let mut ctrl = controller();
        // Force to Level 1.
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);

        // 30 frames at exactly 12ms — p95 = 12ms, NOT < 12ms. Must not recover.
        push_frames(&mut ctrl, RECOVERY_THRESHOLD_US, RECOVERY_WINDOW);
        assert_eq!(ctrl.level(), DegradationLevel::Coalesce);
    }

    #[test]
    fn test_level5_to_normal_requires_5_successive_clean_windows() {
        let mut ctrl = controller();
        // Force to Emergency (Level 5) by advancing 5 times.
        for _ in 0..5 {
            push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW);
        }
        assert_eq!(ctrl.level(), DegradationLevel::Emergency);

        // 5 clean windows (each 30 frames) recovers one level per window.
        let expected = [
            DegradationLevel::ShedTiles,
            DegradationLevel::DisableTransparency,
            DegradationLevel::ReduceTextureQuality,
            DegradationLevel::Coalesce,
            DegradationLevel::Normal,
        ];
        for (i, expected_level) in expected.iter().enumerate() {
            push_frames(&mut ctrl, 5_000, RECOVERY_WINDOW);
            assert_eq!(
                ctrl.level(),
                *expected_level,
                "After {} clean windows, expected {:?}",
                i + 1,
                expected_level
            );
        }
    }

    // ── Tile shedding order ───────────────────────────────────────────────────

    #[test]
    fn test_no_shedding_at_level0_through_level3() {
        for level in [
            DegradationLevel::Normal,
            DegradationLevel::Coalesce,
            DegradationLevel::ReduceTextureQuality,
            DegradationLevel::DisableTransparency,
        ] {
            let mut ctrl = controller();
            // Force to specific level by poking internal state for simplicity.
            ctrl.level = level;
            let tiles = vec![
                TileDescriptor { tile_id: SceneId::new(), lease_priority: 0, z_order: 1 },
                TileDescriptor { tile_id: SceneId::new(), lease_priority: 1, z_order: 0 },
            ];
            let shed = ctrl.shed_tiles(&tiles);
            assert!(shed.is_empty(), "Level {level} must not shed any tiles");
        }
    }

    #[test]
    fn test_level4_sheds_lowest_priority_tiles_first() {
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::ShedTiles;

        let id_p0 = SceneId::new();
        let id_p1 = SceneId::new();
        let id_p2a = SceneId::new();
        let id_p2b = SceneId::new();

        // 4 tiles: priorities 0, 1, 2, 2 (two at priority 2)
        let tiles = vec![
            TileDescriptor { tile_id: id_p0, lease_priority: 0, z_order: 10 },
            TileDescriptor { tile_id: id_p1, lease_priority: 1, z_order: 5 },
            TileDescriptor { tile_id: id_p2a, lease_priority: 2, z_order: 3 },
            TileDescriptor { tile_id: id_p2b, lease_priority: 2, z_order: 1 },
        ];

        let shed = ctrl.shed_tiles(&tiles);
        // 25% of 4 = 1 tile shed. Should be the priority-2/lowest-z_order tile.
        assert_eq!(shed.len(), 1);
        assert_eq!(shed[0].tile_id, id_p2b, "Should shed p2/z1 (lowest z_order in highest priority)");
    }

    #[test]
    fn test_level4_spec_scenario_priority_0_1_2_sheds_priority2_first() {
        // Spec scenario (line 269): when entering Level 4 with tiles at
        // priority 0, 1, and 2 → priority-2 tiles MUST be shed first.
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::ShedTiles;

        let id_p0 = SceneId::new();
        let id_p1 = SceneId::new();
        let id_p2 = SceneId::new();

        let tiles = vec![
            TileDescriptor { tile_id: id_p0, lease_priority: 0, z_order: 5 },
            TileDescriptor { tile_id: id_p1, lease_priority: 1, z_order: 3 },
            TileDescriptor { tile_id: id_p2, lease_priority: 2, z_order: 1 },
        ];

        let shed = ctrl.shed_tiles(&tiles);
        assert!(!shed.is_empty());
        // At least id_p2 must be in shed; id_p0 must NOT be.
        let shed_ids: Vec<SceneId> = shed.iter().map(|t| t.tile_id).collect();
        assert!(shed_ids.contains(&id_p2), "Priority-2 tile must be shed first");
        assert!(!shed_ids.contains(&id_p0), "Priority-0 tile must be preserved");
    }

    #[test]
    fn test_level5_keeps_only_highest_priority_tile() {
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::Emergency;

        let id_high = SceneId::new();
        let id_med = SceneId::new();
        let id_low = SceneId::new();

        let tiles = vec![
            TileDescriptor { tile_id: id_high, lease_priority: 0, z_order: 10 },
            TileDescriptor { tile_id: id_med,  lease_priority: 1, z_order: 5 },
            TileDescriptor { tile_id: id_low,  lease_priority: 2, z_order: 1 },
        ];

        let shed = ctrl.shed_tiles(&tiles);
        let shed_ids: Vec<SceneId> = shed.iter().map(|t| t.tile_id).collect();

        // id_high (priority=0, z_order=10) must be kept; everything else shed.
        assert!(!shed_ids.contains(&id_high), "Highest-priority tile must not be shed");
        assert!(shed_ids.contains(&id_med), "Medium-priority tile must be shed at Level 5");
        assert!(shed_ids.contains(&id_low), "Lowest-priority tile must be shed at Level 5");
        assert_eq!(shed.len(), 2);
    }

    #[test]
    fn test_level5_empty_tiles_returns_empty() {
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::Emergency;
        let shed = ctrl.shed_tiles(&[]);
        assert!(shed.is_empty());
    }

    #[test]
    fn test_shedding_order_same_priority_higher_z_order_wins() {
        // Spec: within same priority class, higher z_order wins (preserved),
        // lower z_order is shed first.
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::ShedTiles;

        let id_high_z = SceneId::new();
        let id_low_z = SceneId::new();

        let tiles = vec![
            TileDescriptor { tile_id: id_high_z, lease_priority: 1, z_order: 100 },
            TileDescriptor { tile_id: id_low_z,  lease_priority: 1, z_order: 1 },
        ];

        let shed = ctrl.shed_tiles(&tiles);
        assert_eq!(shed.len(), 1);
        assert_eq!(shed[0].tile_id, id_low_z, "Lower z_order must be shed first within same priority");
    }

    // ── Level 1 coalescing ────────────────────────────────────────────────────

    #[test]
    fn test_coalesce_level0_always_emits() {
        let ctrl = controller();
        assert_eq!(ctrl.level(), DegradationLevel::Normal);
        // At Level 0, should always emit state-stream.
        for _ in 0..20 {
            assert!(ctrl.should_emit_state_stream());
        }
    }

    #[test]
    fn test_coalesce_level1_emits_every_ratio_frames() {
        let mut ctrl = controller();
        ctrl.level = DegradationLevel::Coalesce;
        // Default ratio = 2: emit on even frame_number, suppress on odd.
        // frame_number starts at 0 and increments each record_frame call.
        // We test should_emit_state_stream() directly against the frame counter.

        // frame_number = 0: 0 % 2 == 0 → emit
        // frame_number = 1: 1 % 2 == 1 → suppress
        // etc.
        let mut emit_count = 0;
        let total = 20;
        for _ in 0..total {
            // Simulate frame advance (record_frame updates frame_number).
            ctrl.frame_number += 1;
            if ctrl.should_emit_state_stream() {
                emit_count += 1;
            }
        }
        // With ratio=2, expect exactly 10 emissions in 20 frames.
        assert_eq!(emit_count, 10, "Coalesce ratio=2 should emit every other frame");
    }

    // ── Level 2/3 flags ───────────────────────────────────────────────────────

    #[test]
    fn test_reduce_texture_quality_flag() {
        let mut ctrl = controller();
        assert!(!ctrl.should_reduce_texture_quality());
        ctrl.level = DegradationLevel::ReduceTextureQuality;
        assert!(ctrl.should_reduce_texture_quality());
        ctrl.level = DegradationLevel::Emergency;
        assert!(ctrl.should_reduce_texture_quality());
    }

    #[test]
    fn test_disable_transparency_flag() {
        let mut ctrl = controller();
        assert!(!ctrl.should_disable_transparency());
        ctrl.level = DegradationLevel::DisableTransparency;
        assert!(ctrl.should_disable_transparency());
        ctrl.level = DegradationLevel::Emergency;
        assert!(ctrl.should_disable_transparency());
    }

    // ── Telemetry events ──────────────────────────────────────────────────────

    #[test]
    fn test_advance_event_has_correct_fields() {
        let mut ctrl = controller();
        let ev = push_and_get_last_event(&mut ctrl, 20_000, TRIGGER_WINDOW).unwrap();
        assert_eq!(ev.previous_level, 0);
        assert_eq!(ev.new_level, 1);
        assert_eq!(ev.direction, DegradationDirection::Advance);
        assert!(ev.frame_time_p95_us > TRIGGER_THRESHOLD_US,
            "p95 should exceed trigger threshold");
    }

    #[test]
    fn test_recover_event_has_correct_fields() {
        let mut ctrl = controller();
        push_frames(&mut ctrl, 20_000, TRIGGER_WINDOW); // advance to Coalesce
        let ev = push_and_get_last_event(&mut ctrl, 5_000, RECOVERY_WINDOW).unwrap();
        assert_eq!(ev.previous_level, 1);
        assert_eq!(ev.new_level, 0);
        assert_eq!(ev.direction, DegradationDirection::Recover);
        assert!(ev.frame_time_p95_us < RECOVERY_THRESHOLD_US,
            "p95 should be below recovery threshold");
    }

    // ── p95 helper ────────────────────────────────────────────────────────────

    #[test]
    fn test_p95_helper_correctness() {
        let mut deque: VecDeque<u64> = VecDeque::new();
        for i in 1..=10u64 {
            deque.push_back(i * 1000);
        }
        // Values: [1000, 2000, ..., 10000]
        // p95 nearest-rank: ceil(0.95*10) = 10 → index 9 → 10000
        assert_eq!(p95_of_last_n(&deque, 10), 10_000);
        // p95 of last 5: [6000, 7000, 8000, 9000, 10000]
        // ceil(0.95*5) = 5 → index 4 → 10000
        assert_eq!(p95_of_last_n(&deque, 5), 10_000);
    }

    // ── Helper ────────────────────────────────────────────────────────────────

    /// Push n frames and return the last non-None event (or None if no events).
    fn push_and_get_last_event(
        ctrl: &mut DegradationController,
        frame_time_us: u64,
        n: usize,
    ) -> Option<DegradationEvent> {
        let mut last = None;
        for _ in 0..n {
            if let Some(ev) = ctrl.record_frame(frame_time_us) {
                last = Some(ev);
            }
        }
        last
    }
}
