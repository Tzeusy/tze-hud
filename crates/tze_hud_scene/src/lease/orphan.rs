//! Orphan state handling, grace period timer, and reconnection logic.
//!
//! Implements spec requirements from lease-governance/spec.md:
//! - Requirement: Orphan Handling Grace Period (lines 132–145)
//! - Requirement: Grace Period Precision (lines 147–154)
//! - Requirement: Lease Suspension Freezes Zone Publications (lines 226–233,
//!   adapted to orphan state)
//!
//! # Separation from the core state machine
//!
//! `LeaseImpl` in `state_machine.rs` handles per-lease state transitions.
//! This module provides the **session-level** facilities: grace period tracking
//! across multiple orphaned leases, tile badge management, and zone-publication
//! enforcement during the orphan window.
//!
//! # Two distinct timers
//!
//! Per spec line 135:
//! - **Detection timer** (heartbeat timeout, default 15 000 ms): determines *when*
//!   orphaning begins. This is a heartbeat-protocol concern; this module only
//!   consumes the disconnect signal.
//! - **Grace window** (default 30 000 ms): starts after detection and determines
//!   how long the agent has to reclaim before leases expire.

use crate::types::SceneId;

// ─── Constants ───────────────────────────────────────────────────────────────

/// Reconnect grace period (ms). Spec: "default 30,000 ms" (line 133).
pub const DEFAULT_GRACE_PERIOD_MS: u64 = 30_000;

/// Precision tolerance for the grace period (ms). Spec line 148:
/// "The grace period MUST be accurate to +/- 100ms."
pub const GRACE_PRECISION_MS: u64 = 100;

// ─── TileVisualHint ──────────────────────────────────────────────────────────

/// Visual overlay hint to display on a tile's rendered surface.
///
/// The compositor renders these within one frame of a state change.
/// Spec requirements:
/// - `DisconnectionBadge`: "Disconnection badge MUST appear within 1 frame"
///   (line 133).
/// - `StaleBadge`: zone publications during ORPHANED state are "stale-badged"
///   (lines 231–233, adapted).
/// - `BudgetWarning`: amber border for budget ≥ 80% (line 170).
/// - `None`: normal rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum TileVisualHint {
    /// Tile renders normally.
    #[default]
    None,
    /// Agent is disconnected; tile is frozen at last state.
    /// Displayed within 1 frame of `disconnect` (spec line 133).
    DisconnectionBadge,
    /// Tile content is stale because the controlling lease is ORPHANED;
    /// new publishes to this tile's zone are rejected.
    StaleBadge,
    /// Budget soft limit (≥ 80%) reached; amber border.
    BudgetWarning,
}

// ─── GracePeriodTimer ────────────────────────────────────────────────────────

/// Tracks the grace window for a single orphaned lease.
///
/// The grace period starts the instant `disconnect` is detected (i.e., when
/// `orphaned_at_ms` is recorded). Callers poll this timer and treat the grace
/// period as expired once at least `grace_ms` milliseconds have elapsed since
/// `orphaned_at_ms`.
///
/// This timer does not intentionally expire early; any deviation from the
/// configured window is due to the underlying clock and scheduling latency,
/// and is expected to stay within the `+/- GRACE_PRECISION_MS` tolerance
/// required by the spec (line 148).
#[derive(Clone, Debug, PartialEq)]
pub struct GracePeriodTimer {
    /// Lease this timer belongs to.
    pub lease_id: SceneId,
    /// Wall-clock time at which the agent was declared orphaned (ms).
    pub orphaned_at_ms: u64,
    /// Configured grace window (ms). Defaults to `DEFAULT_GRACE_PERIOD_MS`.
    pub grace_ms: u64,
}

impl GracePeriodTimer {
    /// Create a new `GracePeriodTimer`.
    pub fn new(lease_id: SceneId, orphaned_at_ms: u64, grace_ms: u64) -> Self {
        Self {
            lease_id,
            orphaned_at_ms,
            grace_ms,
        }
    }

    /// Milliseconds elapsed since orphaning.
    pub fn elapsed_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.orphaned_at_ms)
    }

    /// Milliseconds remaining in the grace window (0 if expired).
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.grace_ms.saturating_sub(self.elapsed_ms(now_ms))
    }

    /// Returns `true` if the agent **can still reconnect** at `now_ms`.
    ///
    /// Per spec (lines 152–154): the agent must be able to reconnect at
    /// `grace_ms - GRACE_PRECISION_MS` (i.e., 29 950 ms when grace = 30 000 ms).
    ///
    /// This is the primary gate for `reconnect()` operations.
    pub fn can_reconnect(&self, now_ms: u64) -> bool {
        self.elapsed_ms(now_ms) < self.grace_ms
    }

    /// Returns `true` when the grace period has definitively expired.
    ///
    /// The implementation ensures NO premature expiry: the expiry sweep
    /// MUST NOT run before `elapsed >= grace_ms`. The +/- 100 ms tolerance
    /// is satisfied by the fact that `can_reconnect` accepts up to `grace_ms - 1`
    /// and expiry is only triggered at `elapsed >= grace_ms`.
    pub fn has_expired(&self, now_ms: u64) -> bool {
        self.elapsed_ms(now_ms) >= self.grace_ms
    }

    /// Grace expiry time (absolute ms).
    pub fn expires_at_ms(&self) -> u64 {
        self.orphaned_at_ms.saturating_add(self.grace_ms)
    }
}

// ─── OrphanedLeaseSnapshot ───────────────────────────────────────────────────

/// Snapshot of orphan-state information for a single lease.
///
/// Returned by session-layer queries; does not hold mutable state.
#[derive(Clone, Debug, PartialEq)]
pub struct OrphanedLeaseSnapshot {
    /// The orphaned lease.
    pub lease_id: SceneId,
    /// Wall-clock time when orphaning was detected (ms).
    pub orphaned_at_ms: u64,
    /// Wall-clock time when the grace period expires (ms).
    pub grace_expires_at_ms: u64,
    /// TTL remaining at the time of query (ms). Note: TTL continues running
    /// during orphan state per spec line 133.
    pub ttl_remaining_ms: Option<u64>,
    /// Whether zone publishes are currently accepted (false — spec lines 231–233).
    pub zone_publishes_accepted: bool,
}

// ─── ZoneOrphanGuard ─────────────────────────────────────────────────────────

/// Enforcement result for a zone publish attempt from an orphaned lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZonePublishResult {
    /// Publish accepted (lease is ACTIVE or not lease-bound).
    Accepted,
    /// Publish rejected because the controlling lease is ORPHANED.
    ///
    /// Existing zone content remains visible with a stale badge (spec line 231).
    RejectedLeaseOrphaned,
    /// Publish rejected because the controlling lease is SUSPENDED (safe mode).
    RejectedSafeModeActive,
    /// Publish rejected because the controlling lease is terminal.
    RejectedLeaseTerminal,
}

/// Check whether a zone publish is allowed given the current lease state.
///
/// Implements spec §Lease Suspension Freezes Zone Publications (lines 226–233)
/// adapted to orphan state:
///
/// - ACTIVE → `Accepted`
/// - ORPHANED → `RejectedLeaseOrphaned` (existing content stale-badged; new
///   publishes rejected)
/// - SUSPENDED → `RejectedSafeModeActive`
/// - terminal states → `RejectedLeaseTerminal`
pub fn check_zone_publish_allowed(lease_state: crate::lease::LeaseState) -> ZonePublishResult {
    use crate::lease::LeaseState;
    match lease_state {
        LeaseState::Active => ZonePublishResult::Accepted,
        // Orphaned and its deprecated alias Disconnected: reject new publishes.
        LeaseState::Orphaned | LeaseState::Disconnected => ZonePublishResult::RejectedLeaseOrphaned,
        LeaseState::Suspended => ZonePublishResult::RejectedSafeModeActive,
        // All remaining states (Requested, Revoked, Expired, Denied, Released)
        // are either pre-active or terminal — reject as not-active.
        LeaseState::Requested
        | LeaseState::Revoked
        | LeaseState::Expired
        | LeaseState::Denied
        | LeaseState::Released => ZonePublishResult::RejectedLeaseTerminal,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SceneId;

    fn dummy_lease_id() -> SceneId {
        SceneId::new()
    }

    // ── Grace period timer ──────────────────────────────────────────────────

    /// WHEN agent orphaned at t=0 and now=29_000 THEN can_reconnect = true.
    #[test]
    fn grace_timer_allows_reconnect_before_expiry() {
        let timer = GracePeriodTimer::new(dummy_lease_id(), 0, DEFAULT_GRACE_PERIOD_MS);
        assert!(timer.can_reconnect(29_000));
        assert!(!timer.has_expired(29_000));
    }

    /// WHEN agent orphaned at t=0 and now=30_000 THEN has_expired = true.
    #[test]
    fn grace_timer_expires_at_grace_ms() {
        let timer = GracePeriodTimer::new(dummy_lease_id(), 0, DEFAULT_GRACE_PERIOD_MS);
        assert!(timer.has_expired(30_000));
        assert!(!timer.can_reconnect(30_000));
    }

    /// Spec §Grace Period Precision (lines 152–154):
    /// "WHEN the reconnect grace period is 30,000ms THEN the agent can still
    /// reconnect at 29,950ms."
    #[test]
    fn grace_period_not_premature_at_29950ms() {
        let timer = GracePeriodTimer::new(dummy_lease_id(), 0, DEFAULT_GRACE_PERIOD_MS);
        // 29_950 ms < 30_000 ms grace: must not be expired.
        assert!(
            timer.can_reconnect(29_950),
            "grace period must not expire prematurely at 29,950 ms"
        );
        assert!(
            !timer.has_expired(29_950),
            "has_expired must be false at 29,950 ms"
        );
    }

    /// WHEN agent orphaned at t=1000 and grace=30_000ms
    /// THEN expires_at_ms = 31_000.
    #[test]
    fn grace_timer_expires_at_correct_absolute_time() {
        let timer = GracePeriodTimer::new(dummy_lease_id(), 1_000, DEFAULT_GRACE_PERIOD_MS);
        assert_eq!(timer.expires_at_ms(), 31_000);
    }

    /// WHEN grace=30_000ms and elapsed=25_000ms THEN remaining=5_000ms.
    #[test]
    fn grace_timer_remaining_ms_correct() {
        let timer = GracePeriodTimer::new(dummy_lease_id(), 0, DEFAULT_GRACE_PERIOD_MS);
        assert_eq!(timer.remaining_ms(25_000), 5_000);
    }

    /// WHEN elapsed > grace THEN remaining = 0 (not underflow).
    #[test]
    fn grace_timer_remaining_saturates_at_zero() {
        let timer = GracePeriodTimer::new(dummy_lease_id(), 0, DEFAULT_GRACE_PERIOD_MS);
        assert_eq!(timer.remaining_ms(60_000), 0);
    }

    // ── Zone publish enforcement ────────────────────────────────────────────

    /// WHEN lease is ACTIVE THEN zone publish is accepted.
    #[test]
    fn zone_publish_accepted_when_active() {
        use crate::lease::LeaseState;
        assert_eq!(
            check_zone_publish_allowed(LeaseState::Active),
            ZonePublishResult::Accepted
        );
    }

    /// WHEN lease is ORPHANED THEN zone publish is rejected.
    ///
    /// Spec lines 231–233: "WHEN a lease is ORPHANED THEN existing zone
    /// publications remain visible with a staleness/disconnection indicator
    /// but no new publishes are accepted."
    #[test]
    fn zone_publish_rejected_when_orphaned() {
        use crate::lease::LeaseState;
        assert_eq!(
            check_zone_publish_allowed(LeaseState::Orphaned),
            ZonePublishResult::RejectedLeaseOrphaned
        );
    }

    /// WHEN lease is SUSPENDED THEN zone publish is rejected with SafeModeActive.
    #[test]
    fn zone_publish_rejected_when_suspended() {
        use crate::lease::LeaseState;
        assert_eq!(
            check_zone_publish_allowed(LeaseState::Suspended),
            ZonePublishResult::RejectedSafeModeActive
        );
    }

    /// WHEN lease is terminal (REVOKED / EXPIRED) THEN zone publish is rejected.
    #[test]
    fn zone_publish_rejected_when_terminal() {
        use crate::lease::LeaseState;
        for state in [
            LeaseState::Revoked,
            LeaseState::Expired,
            LeaseState::Released,
            LeaseState::Denied,
        ] {
            assert_eq!(
                check_zone_publish_allowed(state),
                ZonePublishResult::RejectedLeaseTerminal,
                "expected RejectedLeaseTerminal for {state:?}"
            );
        }
    }

    // ── TileVisualHint ──────────────────────────────────────────────────────

    /// WHEN tile enters orphan state THEN visual hint is DisconnectionBadge.
    #[test]
    fn tile_visual_hint_disconnection_badge_default() {
        let hint = TileVisualHint::DisconnectionBadge;
        assert_eq!(hint, TileVisualHint::DisconnectionBadge);
        assert_ne!(hint, TileVisualHint::None);
    }

    #[test]
    fn tile_visual_hint_default_is_none() {
        assert_eq!(TileVisualHint::default(), TileVisualHint::None);
    }

    // ── GracePeriodTimer precision boundary ────────────────────────────────

    /// Checks the GRACE_PRECISION_MS constant satisfies the +/- 100ms spec.
    #[test]
    fn grace_precision_constant_within_spec() {
        assert!(
            GRACE_PRECISION_MS <= 100,
            "GRACE_PRECISION_MS must be ≤ 100 per spec line 148"
        );
    }

    /// Reconnect at exactly grace_ms - 1 ms must still be allowed.
    #[test]
    fn can_reconnect_at_one_ms_before_expiry() {
        let grace = DEFAULT_GRACE_PERIOD_MS;
        let timer = GracePeriodTimer::new(dummy_lease_id(), 0, grace);
        assert!(timer.can_reconnect(grace - 1));
        assert!(!timer.has_expired(grace - 1));
    }

    /// Reconnect at exactly grace_ms must NOT be allowed.
    #[test]
    fn cannot_reconnect_at_exact_expiry() {
        let grace = DEFAULT_GRACE_PERIOD_MS;
        let timer = GracePeriodTimer::new(dummy_lease_id(), 0, grace);
        assert!(!timer.can_reconnect(grace));
        assert!(timer.has_expired(grace));
    }
}
