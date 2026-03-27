//! Degradation ladder integration: threshold tracking, tile shedding and recovery.
//!
//! Implements:
//! - Requirement: Degradation Does Not Change Lease State (lines 262-269)
//! - Requirement: Tile Shedding Order (lines 271-278)
//! - Requirement: Degradation Trigger Threshold (lines 280-288)
//!
//! ## Degradation Levels
//!
//! | Level | Name            | Action                                                   |
//! |-------|-----------------|----------------------------------------------------------|
//! | 0     | Nominal         | All tiles rendered normally                              |
//! | 1     | Minor           | Minor visual degradation (compositor responsibility)     |
//! | 2     | Moderate        | Moderate degradation                                     |
//! | 3     | Significant     | Significant degradation                                  |
//! | 4     | Shed Tiles      | ~25% of tiles removed from render pass (not scene graph) |
//! | 5     | Emergency       | Only highest-priority tile + chrome rendered             |
//!
//! **Key invariant**: Leases remain ACTIVE at all degradation levels.
//! Tile shedding removes tiles from the *render pass* only — they remain in the
//! scene graph and their leases are not revoked.
//!
//! ## Thresholds
//!
//! - **Entry**: `frame_time_p95 > 14ms` over a 10-frame window → advance one level.
//! - **Recovery**: `frame_time_p95 < 12ms` over a 30-frame window → recover one level.

use crate::lease::priority::{TileSheddingEntry, shed_count_for_level4, shedding_order};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Entry threshold (ms): p95 frame time above this triggers degradation.
pub const ENTRY_THRESHOLD_MS: f64 = 14.0;
/// Recovery threshold (ms): p95 frame time must stay below this to recover.
pub const RECOVERY_THRESHOLD_MS: f64 = 12.0;
/// Number of frames in the entry window.
pub const ENTRY_WINDOW_FRAMES: usize = 10;
/// Number of frames in the recovery window.
pub const RECOVERY_WINDOW_FRAMES: usize = 30;

// ─── Degradation Level ───────────────────────────────────────────────────────

/// Degradation ladder levels (0 = nominal, 5 = emergency).
///
/// Per spec §Requirement: Degradation Does Not Change Lease State (lines 262-269):
/// leases remain ACTIVE at all levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DegradationLevel {
    /// Normal operation — all tiles rendered.
    Nominal = 0,
    /// Minor degradation.
    Minor = 1,
    /// Moderate degradation.
    Moderate = 2,
    /// Significant degradation.
    Significant = 3,
    /// Tile shedding — ~25% of active tiles removed from render pass.
    ShedTiles = 4,
    /// Emergency — only highest-priority tile + chrome rendered.
    Emergency = 5,
}

impl DegradationLevel {
    /// Returns the next-higher degradation level, or `None` if already at Emergency.
    pub fn advance(self) -> Option<DegradationLevel> {
        match self {
            DegradationLevel::Nominal => Some(DegradationLevel::Minor),
            DegradationLevel::Minor => Some(DegradationLevel::Moderate),
            DegradationLevel::Moderate => Some(DegradationLevel::Significant),
            DegradationLevel::Significant => Some(DegradationLevel::ShedTiles),
            DegradationLevel::ShedTiles => Some(DegradationLevel::Emergency),
            DegradationLevel::Emergency => None,
        }
    }

    /// Returns the next-lower degradation level (recovery), or `None` if already Nominal.
    pub fn recover(self) -> Option<DegradationLevel> {
        match self {
            DegradationLevel::Nominal => None,
            DegradationLevel::Minor => Some(DegradationLevel::Nominal),
            DegradationLevel::Moderate => Some(DegradationLevel::Minor),
            DegradationLevel::Significant => Some(DegradationLevel::Moderate),
            DegradationLevel::ShedTiles => Some(DegradationLevel::Significant),
            DegradationLevel::Emergency => Some(DegradationLevel::ShedTiles),
        }
    }
}

// ─── Frame-time window ───────────────────────────────────────────────────────

/// Rolling window of frame times used to compute p95 for degradation decisions.
///
/// The window is bounded: old samples are evicted as new ones arrive.
#[derive(Clone, Debug)]
pub struct FrameTimeWindow {
    samples: std::collections::VecDeque<f64>,
    capacity: usize,
}

impl FrameTimeWindow {
    /// Create a new window with the given capacity (number of frames).
    ///
    /// # Panics
    /// Panics if `capacity == 0` (a zero-capacity window cannot hold any samples
    /// and would grow unboundedly on `push`).
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "FrameTimeWindow capacity must be > 0");
        FrameTimeWindow {
            samples: std::collections::VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Record a new frame time (ms) into the window.
    ///
    /// When the window is full, the oldest sample is evicted.
    pub fn push(&mut self, frame_ms: f64) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(frame_ms);
    }

    /// Returns `true` if the window has exactly `capacity` samples.
    pub fn is_full(&self) -> bool {
        self.samples.len() == self.capacity
    }

    /// Compute the p95 frame time (ms) from the current window.
    ///
    /// Returns `None` if the window is empty.  Uses nearest-rank method.
    pub fn p95(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = self.samples.iter().cloned().collect();
        sorted.sort_unstable_by(f64::total_cmp);
        // Nearest-rank: index = ceil(0.95 * n) - 1
        let n = sorted.len();
        let idx = ((0.95 * n as f64).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1);
        Some(sorted[idx])
    }

    /// Number of samples currently in the window.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Returns `true` if no samples have been recorded.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

// ─── DegradationTracker ──────────────────────────────────────────────────────

/// Tracks degradation state and computes level transitions based on frame times.
///
/// # Invariants
/// - Leases are NEVER touched by this tracker — only the render-pass exclusion
///   set is managed.  Per spec §Requirement: Degradation Does Not Change Lease
///   State (lines 262-269).
/// - Shed tiles are tracked in `shed_tiles` (indices into the tile set).  On
///   recovery from Level 4, the set is cleared and all tiles resume rendering.
#[derive(Clone, Debug)]
pub struct DegradationTracker {
    /// Current degradation level.
    level: DegradationLevel,
    /// Sliding window for entry threshold (10 frames, entry if p95 > 14ms).
    entry_window: FrameTimeWindow,
    /// Sliding window for recovery threshold (30 frames, recover if p95 < 12ms).
    recovery_window: FrameTimeWindow,
    /// Indices (into the tile array provided by the caller) of tiles currently shed.
    shed_tiles: Vec<usize>,
}

impl DegradationTracker {
    /// Create a new tracker at `DegradationLevel::Nominal`.
    pub fn new() -> Self {
        DegradationTracker {
            level: DegradationLevel::Nominal,
            entry_window: FrameTimeWindow::new(ENTRY_WINDOW_FRAMES),
            recovery_window: FrameTimeWindow::new(RECOVERY_WINDOW_FRAMES),
            shed_tiles: Vec::new(),
        }
    }

    /// Current degradation level.
    pub fn level(&self) -> DegradationLevel {
        self.level
    }

    /// The tile indices currently excluded from the render pass.
    ///
    /// This is empty unless the level is `ShedTiles` or `Emergency`.
    /// On recovery to `Nominal` the set is cleared.
    pub fn shed_tiles(&self) -> &[usize] {
        &self.shed_tiles
    }

    /// Record a new frame time sample (ms).
    ///
    /// After recording, this method evaluates the entry and recovery thresholds
    /// and may advance or recover the degradation level by one step.
    ///
    /// `tiles` is the current set of active tiles (used to compute the shed set
    /// when advancing to `ShedTiles`).  It is only read when the level is about
    /// to advance to or recover from `ShedTiles`.
    ///
    /// Returns `true` if the level changed (advance or recovery).
    pub fn record_frame(&mut self, frame_ms: f64, tiles: &[TileSheddingEntry]) -> bool {
        self.entry_window.push(frame_ms);
        self.recovery_window.push(frame_ms);

        // Check for advance (only when entry window is full).
        if self.entry_window.is_full() {
            if let Some(p95) = self.entry_window.p95() {
                if p95 > ENTRY_THRESHOLD_MS {
                    return self.advance(tiles);
                }
            }
        }

        // Check for recovery (only when recovery window is full).
        if self.recovery_window.is_full() {
            if let Some(p95) = self.recovery_window.p95() {
                if p95 < RECOVERY_THRESHOLD_MS {
                    return self.recover_one_level();
                }
            }
        }

        false
    }

    /// Force-advance the degradation level by one step.
    ///
    /// If advancing to `ShedTiles`, computes which tiles to shed from `tiles`.
    /// If already at `Emergency`, does nothing and returns `false`.
    pub fn advance(&mut self, tiles: &[TileSheddingEntry]) -> bool {
        if let Some(next) = self.level.advance() {
            self.level = next;
            if next == DegradationLevel::ShedTiles {
                self.compute_shed_set(tiles);
            }
            // Reset entry window after advancing to prevent immediate re-trigger.
            self.entry_window = FrameTimeWindow::new(ENTRY_WINDOW_FRAMES);
            true
        } else {
            false
        }
    }

    /// Force-recover the degradation level by one step.
    ///
    /// On recovery from `ShedTiles` to `Significant`, the shed set is cleared:
    /// all previously shed tiles resume rendering.
    /// On recovery from `Nominal`, does nothing and returns `false`.
    pub fn recover_one_level(&mut self) -> bool {
        if let Some(prev) = self.level.recover() {
            // Clear shed set when recovering away from ShedTiles level.
            if self.level == DegradationLevel::ShedTiles {
                self.shed_tiles.clear();
            }
            self.level = prev;
            // Reset recovery window after recovering.
            self.recovery_window = FrameTimeWindow::new(RECOVERY_WINDOW_FRAMES);
            true
        } else {
            false
        }
    }

    /// Compute and store the shed set for Level 4 (`ShedTiles`).
    ///
    /// Sheds approximately 25% of the given tiles using the spec sort key
    /// `(lease_priority ASC, z_order DESC)` — least important first.
    fn compute_shed_set(&mut self, tiles: &[TileSheddingEntry]) {
        let count = shed_count_for_level4(tiles.len());
        self.shed_tiles = shedding_order(tiles, count);
    }

    /// Returns `true` if `tile_index` is currently in the shed set (excluded
    /// from the render pass).
    ///
    /// Callers use this to skip rendering shed tiles without changing their
    /// lease state.
    pub fn is_shed(&self, tile_index: usize) -> bool {
        self.shed_tiles.contains(&tile_index)
    }
}

impl Default for DegradationTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tile_entries(priorities_and_z: &[(u8, u32)]) -> Vec<TileSheddingEntry> {
        priorities_and_z
            .iter()
            .enumerate()
            .map(|(i, &(p, z))| TileSheddingEntry::new(i, p, z))
            .collect()
    }

    // ── FrameTimeWindow ───────────────────────────────────────────────────────

    #[test]
    fn frame_window_p95_single_sample() {
        let mut w = FrameTimeWindow::new(10);
        w.push(15.0);
        assert_eq!(w.p95(), Some(15.0));
    }

    #[test]
    fn frame_window_p95_ten_equal_samples() {
        let mut w = FrameTimeWindow::new(10);
        for _ in 0..10 {
            w.push(10.0);
        }
        assert_eq!(w.p95(), Some(10.0));
    }

    #[test]
    fn frame_window_evicts_oldest() {
        let mut w = FrameTimeWindow::new(3);
        w.push(5.0);
        w.push(5.0);
        w.push(5.0);
        w.push(20.0); // evicts first 5.0
        assert_eq!(w.len(), 3);
        // p95 of [5, 5, 20] = 20
        assert_eq!(w.p95(), Some(20.0));
    }

    #[test]
    fn frame_window_p95_empty_returns_none() {
        let w = FrameTimeWindow::new(10);
        assert_eq!(w.p95(), None);
    }

    // ── DegradationLevel transitions ──────────────────────────────────────────

    #[test]
    fn level_advance_from_nominal() {
        assert_eq!(
            DegradationLevel::Nominal.advance(),
            Some(DegradationLevel::Minor)
        );
    }

    #[test]
    fn level_advance_from_emergency_returns_none() {
        assert_eq!(DegradationLevel::Emergency.advance(), None);
    }

    #[test]
    fn level_recover_from_minor() {
        assert_eq!(
            DegradationLevel::Minor.recover(),
            Some(DegradationLevel::Nominal)
        );
    }

    #[test]
    fn level_recover_from_nominal_returns_none() {
        assert_eq!(DegradationLevel::Nominal.recover(), None);
    }

    // ── DegradationTracker ────────────────────────────────────────────────────

    /// Degradation entry threshold: p95 > 14ms over 10-frame window triggers advance.
    ///
    /// Spec scenario (lines 285-287):
    /// "WHEN frame_time_p95 > 14ms is sustained over a 10-frame window
    ///  THEN the degradation ladder advances and tile shedding begins"
    #[test]
    fn tracker_advances_when_p95_exceeds_entry_threshold() {
        let mut tracker = DegradationTracker::new();
        let tiles = tile_entries(&[(2, 5), (2, 3), (3, 1), (3, 0)]);

        // Push 9 frames at 15ms (above 14ms threshold) — window not full yet.
        for _ in 0..9 {
            let changed = tracker.record_frame(15.0, &tiles);
            assert!(!changed, "should not advance before window is full");
        }
        assert_eq!(tracker.level(), DegradationLevel::Nominal);

        // 10th frame fills the window and triggers advance.
        let changed = tracker.record_frame(15.0, &tiles);
        assert!(changed, "should advance after 10 frames above threshold");
        assert_eq!(tracker.level(), DegradationLevel::Minor);
    }

    /// Recovery threshold: p95 < 12ms over 30-frame window triggers recovery.
    #[test]
    fn tracker_recovers_when_p95_below_recovery_threshold() {
        let mut tracker = DegradationTracker::new();
        let tiles = tile_entries(&[(2, 5)]);

        // Advance to Minor by filling entry window with 15ms frames.
        for _ in 0..10 {
            tracker.record_frame(15.0, &tiles);
        }
        assert_eq!(tracker.level(), DegradationLevel::Minor);

        // Push 30 frames of 10ms.  The recovery window (capacity 30) starts with
        // the 10 × 15ms frames from the advance phase; as we push 10ms frames the
        // 15ms frames are evicted.
        //
        // Recovery fires as soon as the 30-frame window's p95 drops below 12ms.
        // With the nearest-rank method (index = ceil(0.95×n) − 1 = 28 of 30),
        // recovery fires once the 29th-lowest sample (index 28) is ≤ 10ms —
        // i.e. when at most 1 of the 30 window entries is still at 15ms.
        // That happens after 29 pushes of 10ms (evicting 29 of the original 10 × 15ms
        // entries from the mixed window — once the window fills at push 20, each
        // subsequent push evicts one old 15ms frame; by push 29 only 1 of the 10
        // original 15ms frames remains in the window, making p95 = 10ms).
        //
        // The recovery window becomes full after 20 pushes (10 initial + 20 new = 30).
        // From push 21 onwards the 15ms frames are evicted one-by-one.
        //
        // After 28 × 10ms pushes (i = 0..27): still at least 2 × 15ms frames in
        // the window → p95 = 15ms → no recovery.
        // After 29 × 10ms pushes (i = 28): only 1 × 15ms remains → p95 = 10ms
        // → recovery fires.
        let mut recovered = false;
        for i in 0..30 {
            let changed = tracker.record_frame(10.0, &tiles);
            if changed {
                // Recovery must happen somewhere in the range [28, 29] — once the
                // p95 of the mixed window drops below 12ms.
                assert!(
                    i >= 28,
                    "recovery should not happen before the window has mostly 10ms frames (i={})",
                    i
                );
                recovered = true;
                break;
            }
        }
        assert!(recovered, "should have recovered within 30 frames");
        assert_eq!(tracker.level(), DegradationLevel::Nominal);
    }

    /// Spec scenario (lines 267-269):
    /// "WHEN the degradation ladder reaches Level 4 and a tile is shed from the render pass
    ///  THEN the tile's lease remains ACTIVE, the agent can still submit mutations, and the
    ///  tile remains in the scene graph"
    ///
    /// This test validates that `DegradationTracker` marks tiles as shed WITHOUT changing
    /// lease state (lease state management is the SceneGraph's responsibility; this
    /// test confirms the tracker API does not touch leases).
    #[test]
    fn tracker_at_shed_tiles_marks_tiles_shed_but_does_not_revoke_leases() {
        let mut tracker = DegradationTracker::new();
        let tiles = tile_entries(&[
            (1, 10), // high-priority, stays
            (2, 5),  // normal
            (3, 1),  // low-priority, shed first
            (3, 0),  // low-priority lowest z, shed next
        ]);

        // Advance directly to ShedTiles (Level 4).
        for _ in 0..4 {
            tracker.advance(&tiles);
        }
        assert_eq!(tracker.level(), DegradationLevel::ShedTiles);

        // ~25% of 4 tiles = 1 tile shed.
        let shed = tracker.shed_tiles();
        assert_eq!(shed.len(), 1, "should shed ~25% = 1 tile");
        assert!(tracker.is_shed(shed[0]), "shed tile should be flagged");

        // The shed tile must be the least important (priority=3, z=0 = index 3).
        assert_eq!(
            shed[0], 3,
            "least-important tile (priority=3, z=0) should shed first"
        );

        // Other tiles not shed.
        assert!(!tracker.is_shed(0), "high-priority tile must not be shed");
    }

    /// Spec scenario (lines 276-278):
    /// "WHEN degradation recovers from Level 4 to Level 0
    ///  THEN previously shed tiles resume rendering with their last committed content
    ///  without any agent action"
    #[test]
    fn tracker_shed_tiles_cleared_on_recovery_from_shed_tiles() {
        let mut tracker = DegradationTracker::new();
        let tiles = tile_entries(&[(2, 5), (3, 1)]);

        // Advance to ShedTiles.
        for _ in 0..4 {
            tracker.advance(&tiles);
        }
        assert_eq!(tracker.level(), DegradationLevel::ShedTiles);
        assert!(!tracker.shed_tiles().is_empty());

        // Recover one level.
        let changed = tracker.recover_one_level();
        assert!(changed);
        assert_eq!(tracker.level(), DegradationLevel::Significant);
        // Shed set must be cleared on recovery from ShedTiles.
        assert!(
            tracker.shed_tiles().is_empty(),
            "shed set must be cleared when recovering from ShedTiles"
        );
    }

    /// Degradation does NOT change lease state — this is an API contract test.
    /// The tracker has no knowledge of leases; it only tracks render-pass exclusion.
    #[test]
    fn tracker_has_no_lease_mutation_api() {
        // This is a compile-time / API design assertion: DegradationTracker exposes
        // no method that takes a &mut Lease or mutates lease state.
        // The absence of such a method enforces the invariant from spec lines 262-269.
        let tracker = DegradationTracker::new();
        // Only render-pass information is exposed.
        let _ = tracker.level();
        let _ = tracker.shed_tiles();
        let _ = tracker.is_shed(0);
    }

    /// Shed tiles are re-computed when advancing to ShedTiles for the second time
    /// (after recovering from ShedTiles and then degrading again).
    #[test]
    fn tracker_recomputes_shed_set_on_re_entry_to_shed_tiles() {
        let mut tracker = DegradationTracker::new();
        let tiles_initial = tile_entries(&[(2, 5), (3, 1)]);

        // First entry to ShedTiles.
        for _ in 0..4 {
            tracker.advance(&tiles_initial);
        }
        let first_shed = tracker.shed_tiles().to_vec();
        assert!(!first_shed.is_empty());

        // Recover to Significant.
        tracker.recover_one_level();
        assert!(tracker.shed_tiles().is_empty());

        // Advance again to ShedTiles with a different tile set.
        let tiles_new = tile_entries(&[(1, 10), (3, 2), (2, 5)]);
        tracker.advance(&tiles_new);
        // Shed set should be recomputed from the new tile set.
        let second_shed = tracker.shed_tiles().to_vec();
        assert!(!second_shed.is_empty());
    }

    /// Three-agent contention: priority 1/2/3, z 10/5/1.
    /// At Level 4 (1 shed from 3), priority=3 z=1 tile sheds first.
    #[test]
    fn tracker_three_agents_contention_shedding_order() {
        let mut tracker = DegradationTracker::new();
        let tiles = tile_entries(&[
            (1, 10), // agent.high_prio
            (2, 5),  // agent.normal_prio
            (3, 1),  // agent.low_prio
        ]);

        for _ in 0..4 {
            tracker.advance(&tiles);
        }
        assert_eq!(tracker.level(), DegradationLevel::ShedTiles);
        // ceil(3/4) = 1 shed.
        assert_eq!(tracker.shed_tiles().len(), 1);
        assert_eq!(
            tracker.shed_tiles()[0],
            2,
            "low_prio tile (index 2) sheds first"
        );
        assert!(!tracker.is_shed(0), "high_prio tile must not be shed");
    }
}
