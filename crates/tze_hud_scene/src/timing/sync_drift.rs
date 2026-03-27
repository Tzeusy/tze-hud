//! Sync group drift budget tracking and telemetry.
//!
//! Tracks the worst mutation-arrival spread within committed sync groups for
//! a frame. When the spread exceeds `sync_drift_budget_us`, a telemetry alert
//! is emitted and the slow member's tiles should have their staleness
//! indicators activated.
//!
//! # Spec alignment
//!
//! - Sync Drift Budget (timing-model/spec.md lines 197–208)
//!
//! ## Key definitions
//!
//! - **Arrival spread**: for a committed sync group in one frame, the
//!   difference between the latest and earliest `arrival_wall_us` values
//!   among all committed member tiles.
//! - **`sync_group_max_drift_us`**: the worst spread observed across *all*
//!   committed sync groups in a frame.
//! - **Budget**: default 500 µs (`sync_drift_budget_us`). Configurable via
//!   `TimingConfig`.

use serde::{Deserialize, Serialize};

use crate::timing::domains::{DurationUs, WallUs};
use crate::types::SceneId;

/// Default drift budget (500 µs).
///
/// Spec: timing-model/spec.md line 198.
pub const DEFAULT_SYNC_DRIFT_BUDGET_US: DurationUs = DurationUs(500);

/// Per-frame drift telemetry record for sync groups.
///
/// The compositor fills this in at Stage 4 (Scene Commit) for each frame that
/// has at least one committed sync group.
///
/// Spec: timing-model/spec.md lines 197–208.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameSyncDriftRecord {
    /// Worst mutation-arrival spread across all committed sync groups
    /// in this frame.
    ///
    /// `DurationUs(0)` means no sync groups were committed this frame.
    pub sync_group_max_drift_us: DurationUs,

    /// `true` if `sync_group_max_drift_us > sync_drift_budget_us`.
    ///
    /// Spec: timing-model/spec.md lines 202–204.
    pub sync_drift_budget_exceeded: bool,

    /// Tiles whose staleness indicator must be activated because their arrival
    /// contributed to a budget exceedance.
    ///
    /// These are the "slow" tiles — their `arrival_wall_us` was the latest
    /// within their sync group AND the group's spread exceeded the budget.
    pub stale_tiles: Vec<SceneId>,
}

/// Telemetry alert emitted when the drift budget is exceeded.
///
/// Spec: timing-model/spec.md line 198 — "emit sync_drift_high telemetry alert".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncDriftHighAlert {
    /// The group whose commit triggered the exceedance.
    pub group_id: SceneId,
    /// Observed spread.
    pub observed_drift_us: DurationUs,
    /// Configured budget.
    pub budget_us: DurationUs,
    /// Tiles that arrived late and should have their staleness indicator set.
    pub slow_tiles: Vec<SceneId>,
}

/// Per-tile arrival record used as input to drift computation.
#[derive(Clone, Debug, PartialEq)]
pub struct TileArrival {
    /// The tile.
    pub tile_id: SceneId,
    /// Wall-clock time when the mutation arrived at the compositor.
    pub arrival_wall_us: WallUs,
}

/// Compute the drift spread for a set of committed tile arrivals.
///
/// Returns `(spread, slow_tiles)` where `spread` is the difference
/// between the latest and earliest arrival times, and `slow_tiles` is the
/// set of tiles whose arrival time equals the latest value (i.e. they were
/// the "slow" contributors).
///
/// Returns `(DurationUs::ZERO, vec![])` if `arrivals` is empty or has only one entry.
pub fn compute_spread(arrivals: &[TileArrival]) -> (DurationUs, Vec<SceneId>) {
    if arrivals.len() <= 1 {
        return (DurationUs::ZERO, vec![]);
    }

    let min_arrival = arrivals
        .iter()
        .map(|a| a.arrival_wall_us)
        .min()
        .unwrap_or(WallUs::NOT_SET);
    let max_arrival = arrivals
        .iter()
        .map(|a| a.arrival_wall_us)
        .max()
        .unwrap_or(WallUs::NOT_SET);
    let spread = DurationUs(max_arrival.0.saturating_sub(min_arrival.0));

    let slow_tiles: Vec<SceneId> = arrivals
        .iter()
        .filter(|a| a.arrival_wall_us == max_arrival)
        .map(|a| a.tile_id)
        .collect();

    (spread, slow_tiles)
}

/// A sync group's committed arrivals in one frame.
#[derive(Clone, Debug)]
pub struct SyncGroupArrival {
    /// Group ID.
    pub group_id: SceneId,
    /// Arrival records for each committed member tile.
    pub tile_arrivals: Vec<TileArrival>,
}

/// Evaluate drift across all committed sync groups in a frame.
///
/// # Arguments
///
/// * `groups` — All sync groups that were committed this frame, with per-tile
///   arrival records.
/// * `budget_us` — Configured drift budget. Use
///   [`DEFAULT_SYNC_DRIFT_BUDGET_US`] for the spec default.
///
/// # Returns
///
/// A [`FrameSyncDriftRecord`] and a vec of [`SyncDriftHighAlert`]s (one per
/// group that exceeded the budget).
pub fn evaluate_frame_drift(
    groups: &[SyncGroupArrival],
    budget_us: DurationUs,
) -> (FrameSyncDriftRecord, Vec<SyncDriftHighAlert>) {
    let mut max_drift_us = DurationUs::ZERO;
    let mut all_stale_tiles: Vec<SceneId> = Vec::new();
    let mut alerts: Vec<SyncDriftHighAlert> = Vec::new();

    for group in groups {
        let (spread, slow_tiles) = compute_spread(&group.tile_arrivals);
        if spread > max_drift_us {
            max_drift_us = spread;
        }
        if spread > budget_us {
            all_stale_tiles.extend_from_slice(&slow_tiles);
            alerts.push(SyncDriftHighAlert {
                group_id: group.group_id,
                observed_drift_us: spread,
                budget_us,
                slow_tiles,
            });
        }
    }

    let record = FrameSyncDriftRecord {
        sync_group_max_drift_us: max_drift_us,
        sync_drift_budget_exceeded: max_drift_us > budget_us,
        stale_tiles: all_stale_tiles,
    };

    (record, alerts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrival(tile_id: SceneId, arrival_wall_us: u64) -> TileArrival {
        TileArrival {
            tile_id,
            arrival_wall_us: WallUs(arrival_wall_us),
        }
    }

    // ── compute_spread ────────────────────────────────────────────────────────

    #[test]
    fn spread_zero_for_empty_arrivals() {
        let (spread, slow) = compute_spread(&[]);
        assert_eq!(spread, DurationUs::ZERO);
        assert!(slow.is_empty());
    }

    #[test]
    fn spread_zero_for_single_arrival() {
        let t1 = SceneId::new();
        let (spread, slow) = compute_spread(&[arrival(t1, 1_000_000)]);
        assert_eq!(spread, DurationUs::ZERO);
        assert!(slow.is_empty());
    }

    #[test]
    fn spread_computed_for_two_arrivals() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let arrivals = vec![arrival(t1, 1_000_000), arrival(t2, 1_000_300)];
        let (spread, slow) = compute_spread(&arrivals);
        assert_eq!(spread, DurationUs(300));
        assert_eq!(slow, vec![t2]); // t2 arrived latest
    }

    #[test]
    fn spread_slow_tile_is_latest_arrival() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let t3 = SceneId::new();
        let arrivals = vec![
            arrival(t1, 1_000_000),
            arrival(t2, 1_000_800), // latest — slow
            arrival(t3, 1_000_200),
        ];
        let (spread, slow) = compute_spread(&arrivals);
        assert_eq!(spread, DurationUs(800));
        assert_eq!(slow, vec![t2]);
    }

    // ── evaluate_frame_drift: within budget ────────────────────────────────────

    #[test]
    fn drift_within_budget_returns_no_alerts_and_false_flag() {
        let gid = SceneId::new();
        let t1 = SceneId::new();
        let t2 = SceneId::new();

        // 300 µs spread — below 500 µs default budget
        let group = SyncGroupArrival {
            group_id: gid,
            tile_arrivals: vec![arrival(t1, 1_000_000), arrival(t2, 1_000_300)],
        };

        let (record, alerts) = evaluate_frame_drift(&[group], DEFAULT_SYNC_DRIFT_BUDGET_US);

        assert_eq!(record.sync_group_max_drift_us, DurationUs(300));
        assert!(
            !record.sync_drift_budget_exceeded,
            "300µs < 500µs budget should not exceed"
        );
        assert!(record.stale_tiles.is_empty());
        assert!(alerts.is_empty());
    }

    // ── evaluate_frame_drift: exceeds budget ─────────────────────────────────────

    #[test]
    fn drift_exceeded_800us_sets_flag_and_activates_staleness() {
        let gid = SceneId::new();
        let t1 = SceneId::new();
        let t2 = SceneId::new();

        // 800 µs spread — exceeds 500 µs default budget
        let group = SyncGroupArrival {
            group_id: gid,
            tile_arrivals: vec![arrival(t1, 1_000_000), arrival(t2, 1_000_800)],
        };

        let (record, alerts) = evaluate_frame_drift(&[group], DEFAULT_SYNC_DRIFT_BUDGET_US);

        assert_eq!(record.sync_group_max_drift_us, DurationUs(800));
        assert!(
            record.sync_drift_budget_exceeded,
            "800µs > 500µs budget must set exceeded flag"
        );
        assert!(
            record.stale_tiles.contains(&t2),
            "t2 (slow tile) must be in stale_tiles"
        );
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].group_id, gid);
        assert_eq!(alerts[0].observed_drift_us, DurationUs(800));
        assert!(alerts[0].slow_tiles.contains(&t2));
    }

    // ── evaluate_frame_drift: no groups committed ─────────────────────────────

    #[test]
    fn no_groups_committed_yields_zero_drift_and_no_exceeded() {
        let (record, alerts) = evaluate_frame_drift(&[], DEFAULT_SYNC_DRIFT_BUDGET_US);
        assert_eq!(record.sync_group_max_drift_us, DurationUs::ZERO);
        assert!(!record.sync_drift_budget_exceeded);
        assert!(record.stale_tiles.is_empty());
        assert!(alerts.is_empty());
    }

    // ── evaluate_frame_drift: multiple groups, one exceeds ────────────────────

    #[test]
    fn multiple_groups_max_drift_is_worst_across_all() {
        let gid1 = SceneId::new();
        let gid2 = SceneId::new();
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let t3 = SceneId::new();
        let t4 = SceneId::new();

        // Group 1: 200 µs spread (within budget)
        let g1 = SyncGroupArrival {
            group_id: gid1,
            tile_arrivals: vec![arrival(t1, 2_000_000), arrival(t2, 2_000_200)],
        };
        // Group 2: 700 µs spread (exceeds budget)
        let g2 = SyncGroupArrival {
            group_id: gid2,
            tile_arrivals: vec![arrival(t3, 3_000_000), arrival(t4, 3_000_700)],
        };

        let (record, alerts) = evaluate_frame_drift(&[g1, g2], DEFAULT_SYNC_DRIFT_BUDGET_US);

        assert_eq!(
            record.sync_group_max_drift_us,
            DurationUs(700),
            "max drift should be worst across both groups"
        );
        assert!(record.sync_drift_budget_exceeded);
        // Only t4 from group 2 is stale
        assert!(!record.stale_tiles.contains(&t2));
        assert!(record.stale_tiles.contains(&t4));
        // Only one alert (for group 2)
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].group_id, gid2);
    }

    // ── Spec constant ─────────────────────────────────────────────────────────

    #[test]
    fn default_budget_is_500_us() {
        assert_eq!(DEFAULT_SYNC_DRIFT_BUDGET_US.0, 500);
    }
}
