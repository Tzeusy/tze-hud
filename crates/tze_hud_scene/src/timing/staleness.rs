//! Staleness indicators for tiles.
//!
//! # Spec alignment
//!
//! Implements `timing-model/spec.md §Requirement: Staleness Indicators`
//! (lines 300-311).
//!
//! ## Two staleness conditions
//!
//! 1. **Content staleness**: no mutation within `tile_stale_threshold_ms`
//!    (default 5000ms) for `STATE_STREAM` / `TRANSACTIONAL` tiles with a
//!    registered agent session.
//! 2. **Sync group staleness**: arrival spread exceeds `sync_drift_budget_us`.
//!    (Tracked as a bool flag set by the sync-group layer; this module does not
//!    compute the spread itself.)
//!
//! ## Cleared on
//!
//! - A new valid mutation arriving.
//! - The agent disconnecting.
//!
//! ## Freeze semantics (spec §Requirement: Freeze Override Timing Behavior)
//!
//! The staleness timer is **suspended** during freeze.  Call
//! [`TileStaleness::suspend`] on freeze and [`TileStaleness::resume`] on
//! unfreeze.  Time elapsed during freeze does not count.
//!
//! ## Safe mode semantics (spec §Requirement: Safe Mode Timing Behavior)
//!
//! Staleness is **suppressed** for suspended sessions during safe mode.
//! The timer keeps running but [`TileStaleness::is_stale`] returns `false`
//! when `session_suspended` is set.  On safe mode exit call
//! [`TileStaleness::reset`] to restart the timer from zero.

use serde::{Deserialize, Serialize};

// ─── TileStaleness ───────────────────────────────────────────────────────────

/// Staleness state for a single tile.
///
/// The compositor advances time by calling
/// [`TileStaleness::on_frame`] each frame. When a mutation arrives,
/// call [`TileStaleness::on_mutation`] to reset the timer.
///
/// Internally tracks elapsed microseconds so it works correctly with any
/// injectable clock.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TileStaleness {
    /// Cumulative idle time in microseconds since last mutation.
    elapsed_idle_us: u64,
    /// Staleness threshold in microseconds.
    threshold_us: u64,
    /// Freeze flag: when `true`, `on_frame` does not advance the timer.
    suspended: bool,
    /// When `true`, `is_stale()` returns `false` even if the timer has
    /// expired.  Used during safe mode for suspended sessions.
    session_suspended: bool,
    /// Whether sync-group drift staleness is active.
    sync_drift_stale: bool,
}

impl TileStaleness {
    /// Create a new staleness tracker.
    ///
    /// `threshold_us` should be `tile_stale_threshold_ms * 1000` (default
    /// 5_000_000 µs = 5 seconds).
    pub fn new(threshold_us: u64) -> Self {
        Self {
            elapsed_idle_us: 0,
            threshold_us,
            suspended: false,
            session_suspended: false,
            sync_drift_stale: false,
        }
    }

    /// Advance the idle timer by `delta_us` microseconds.
    ///
    /// Call once per frame with the elapsed frame duration.
    /// Does nothing while [`suspended`][Self::suspended].
    pub fn on_frame(&mut self, delta_us: u64) {
        if !self.suspended {
            self.elapsed_idle_us = self.elapsed_idle_us.saturating_add(delta_us);
        }
    }

    /// Reset the idle timer.
    ///
    /// Call when a new valid mutation arrives for this tile.
    ///
    /// Spec: lines 309-311 — staleness indicator MUST be cleared immediately
    /// when a new valid mutation arrives.
    pub fn on_mutation(&mut self) {
        self.elapsed_idle_us = 0;
        self.sync_drift_stale = false;
    }

    /// Returns `true` if the tile is considered stale.
    ///
    /// Stale iff:
    /// - `elapsed_idle_us >= threshold_us` (content staleness), OR
    /// - `sync_drift_stale` is set (sync-group drift staleness),
    ///
    /// AND `session_suspended == false`.
    pub fn is_stale(&self) -> bool {
        if self.session_suspended {
            return false;
        }
        self.sync_drift_stale || self.elapsed_idle_us >= self.threshold_us
    }

    /// Suspend the timer (freeze).
    ///
    /// From spec §Requirement: Freeze Override Timing Behavior:
    /// `tile_stale_threshold_ms` timer MUST be suspended during freeze.
    pub fn suspend(&mut self) {
        self.suspended = true;
    }

    /// Resume the timer after freeze.
    ///
    /// The idle timer continues from where it was before freeze.
    pub fn resume(&mut self) {
        self.suspended = false;
    }

    /// Mark the session as suspended (safe mode).
    ///
    /// From spec §Requirement: Safe Mode Timing Behavior:
    /// staleness suppressed for suspended sessions.
    pub fn set_session_suspended(&mut self, suspended: bool) {
        self.session_suspended = suspended;
    }

    /// Reset the idle timer to zero (called on safe mode exit).
    ///
    /// From spec §Requirement: Safe Mode Timing Behavior:
    /// staleness timers MUST be reset to 0 on safe mode exit.
    pub fn reset(&mut self) {
        self.elapsed_idle_us = 0;
        self.sync_drift_stale = false;
    }

    /// Set or clear the sync-drift staleness flag.
    ///
    /// Set by the sync-group layer when arrival spread exceeds
    /// `sync_drift_budget_us`.
    pub fn set_sync_drift_stale(&mut self, stale: bool) {
        self.sync_drift_stale = stale;
    }

    /// Current idle elapsed time in microseconds.
    pub fn elapsed_idle_us(&self) -> u64 {
        self.elapsed_idle_us
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLD_US: u64 = 5_000_000; // 5 seconds

    fn tracker() -> TileStaleness {
        TileStaleness::new(THRESHOLD_US)
    }

    // ── Content staleness ──

    /// WHEN tile idle for less than threshold THEN not stale.
    #[test]
    fn not_stale_before_threshold() {
        let mut t = tracker();
        t.on_frame(4_999_999);
        assert!(!t.is_stale());
    }

    /// WHEN tile idle for exactly the threshold THEN stale.
    #[test]
    fn stale_at_threshold() {
        let mut t = tracker();
        t.on_frame(THRESHOLD_US);
        assert!(t.is_stale());
    }

    /// WHEN STATE_STREAM tile receives no mutation for 5s THEN stale.
    #[test]
    fn stale_after_5_seconds() {
        let mut t = tracker();
        // 50 frames × 100_000 µs = 5 seconds
        for _ in 0..50 {
            t.on_frame(100_000);
        }
        assert!(t.is_stale());
    }

    /// WHEN stale tile receives mutation THEN no longer stale.
    #[test]
    fn mutation_clears_staleness() {
        let mut t = tracker();
        t.on_frame(THRESHOLD_US);
        assert!(t.is_stale());
        t.on_mutation();
        assert!(!t.is_stale());
    }

    // ── Freeze / suspend ──

    /// WHEN tile idle 4800ms and 2s freeze occurs THEN not stale until 200ms
    /// after unfreeze (spec lines 335-337).
    #[test]
    fn freeze_suspends_staleness_timer() {
        let mut t = tracker();
        // 4800ms of idle pre-freeze
        t.on_frame(4_800_000);
        assert!(!t.is_stale());

        // Freeze
        t.suspend();
        // 2000ms "passes" during freeze (timer doesn't advance)
        t.on_frame(2_000_000);
        assert!(!t.is_stale()); // not stale during freeze

        // Unfreeze
        t.resume();
        // Only 200ms more needed after unfreeze
        t.on_frame(100_000); // 4900ms total, not yet
        assert!(!t.is_stale());
        t.on_frame(100_000); // 5000ms total, now stale
        assert!(t.is_stale());
    }

    // ── Safe mode: session suspended ──

    /// WHEN safe mode active and session suspended THEN staleness NOT shown.
    #[test]
    fn safe_mode_suppresses_staleness() {
        let mut t = tracker();
        t.on_frame(THRESHOLD_US + 1_000_000); // well past threshold
        assert!(t.is_stale());

        t.set_session_suspended(true);
        assert!(!t.is_stale(), "staleness suppressed for suspended session");

        t.set_session_suspended(false);
        assert!(t.is_stale(), "staleness returns when session resumes");
    }

    /// WHEN safe mode exits THEN staleness timer reset to 0.
    #[test]
    fn safe_mode_exit_resets_timer() {
        let mut t = tracker();
        t.on_frame(THRESHOLD_US); // stale
        assert!(t.is_stale());

        // Safe mode exit
        t.reset();
        assert!(!t.is_stale());
        assert_eq!(t.elapsed_idle_us(), 0);
    }

    // ── Sync drift staleness ──

    #[test]
    fn sync_drift_flag_causes_staleness() {
        let mut t = tracker();
        t.set_sync_drift_stale(true);
        assert!(t.is_stale());
    }

    #[test]
    fn mutation_clears_sync_drift_staleness() {
        let mut t = tracker();
        t.set_sync_drift_stale(true);
        t.on_mutation();
        assert!(!t.is_stale());
    }
}
