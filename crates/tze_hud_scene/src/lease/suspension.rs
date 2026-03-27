//! Safe-mode suspend/resume and max-suspension-timeout management.
//!
//! Implements:
//! - spec §Requirement: Safe Mode Suspends Leases (lease-governance/spec.md lines 92-99)
//! - spec §Requirement: Safe Mode Resume (spec lines 105-108)
//! - spec §Requirement: Max Suspension Time (spec lines 114-122)
//! - spec §Requirement: Suspension Preserves State (spec lines 123-131)
//!
//! ## Responsibility
//!
//! [`SuspensionManager`] is a multi-lease coordinator: it tracks which leases are
//! currently suspended and enforces the max-suspension-timeout by identifying
//! which leases have been suspended beyond `max_suspension_time_ms`.
//!
//! State preservation (tiles, node trees, zone publications, resources) is
//! guaranteed by the scene graph itself: the suspension operations only change
//! the lease `state` field; no tile or node data is freed.  This module is
//! responsible for asserting that guarantee at the manager level.
//!
//! ## Clock convention
//!
//! All timestamps use milliseconds from [`Clock::now_millis()`].
//! Spec §Max Suspension Time default: 300,000 ms (5 minutes).

use crate::clock::Clock;
use crate::types::{LeaseState, SceneId};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Default max suspension time before a lease is force-revoked (ms).
///
/// Spec §Requirement: Max Suspension Time: "default: 300,000ms / 5 minutes".
pub const DEFAULT_MAX_SUSPENSION_MS: u64 = 300_000;

/// Safe-mode suspension must complete within 1 frame (16.6 ms).
///
/// Used in latency assertions in tests; not enforced at runtime (the compositor
/// schedules the suspension flush on the next frame boundary).
pub const SAFE_MODE_SUSPEND_DEADLINE_MS: u64 = 17; // 1 frame ≈ 16.6 ms, rounded up

/// Safe-mode resume must complete within 2 frames (33.2 ms).
pub const SAFE_MODE_RESUME_DEADLINE_MS: u64 = 34; // 2 frames ≈ 33.2 ms, rounded up

// ─── SuspendedEntry ──────────────────────────────────────────────────────────

/// Per-lease record kept by [`SuspensionManager`] while the lease is suspended.
#[derive(Clone, Debug, PartialEq)]
pub struct SuspendedEntry {
    /// The suspended lease ID.
    pub lease_id: SceneId,
    /// Wall-clock time when suspension started (ms).
    pub suspended_at_ms: u64,
    /// TTL remaining at suspension entry (ms).  Preserved for resume.
    pub ttl_remaining_at_suspend_ms: Option<u64>,
}

// ─── SafeModeResult ──────────────────────────────────────────────────────────

/// Result of a safe-mode entry operation.
#[derive(Clone, Debug, PartialEq)]
pub struct SafeModeResult {
    /// Lease IDs that were transitioned from ACTIVE → SUSPENDED.
    pub suspended_lease_ids: Vec<SceneId>,
}

/// Result of a safe-mode exit operation.
#[derive(Clone, Debug, PartialEq)]
pub struct SafeModeResumeResult {
    /// Per-lease resume data.
    pub resumed: Vec<LeaseResumeData>,
}

/// Data vended to the session layer per resumed lease.
#[derive(Clone, Debug, PartialEq)]
pub struct LeaseResumeData {
    /// The resumed lease ID.
    pub lease_id: SceneId,
    /// Wall-clock timestamp (ms) of resume.
    pub resumed_at_ms: u64,
    /// Duration spent in suspension (ms).  Used to populate `suspension_duration_us`
    /// in the wire `LeaseResume` message (multiply by 1000).
    pub suspension_duration_ms: u64,
    /// Adjusted expiry wall-clock timestamp (ms).
    ///
    /// Spec formula: `granted_at_wall_ms + ttl_ms + suspension_duration_ms`.
    /// `None` if TTL is indefinite.
    pub adjusted_expires_at_ms: Option<u64>,
}

// ─── SuspensionTimeoutEntry ───────────────────────────────────────────────────

/// A lease that has exceeded the max suspension time and should be revoked.
#[derive(Clone, Debug, PartialEq)]
pub struct SuspensionTimeoutEntry {
    /// The lease ID that timed out.
    pub lease_id: SceneId,
    /// How long it was suspended (ms).
    pub suspension_duration_ms: u64,
}

// ─── SuspensionManager ───────────────────────────────────────────────────────

/// Multi-lease suspension coordinator for a single runtime (or session).
///
/// Tracks which leases are currently suspended and when they entered
/// suspension.  On each tick, callers should call [`check_timeouts`] to find
/// leases that have exceeded `max_suspension_time_ms` and must be revoked.
///
/// State preservation invariant: `SuspensionManager` never frees tile or node
/// data.  The scene graph owns scene state; this coordinator only manages
/// suspension bookkeeping.
#[derive(Clone, Debug)]
pub struct SuspensionManager<C: Clock> {
    clock: C,
    /// Active suspension entries, ordered by `suspended_at_ms` ascending
    /// (oldest first) to support progressive revocation.
    suspended: Vec<SuspendedEntry>,
    /// Maximum time a lease may remain suspended before being revoked (ms).
    max_suspension_ms: u64,
}

impl<C: Clock> SuspensionManager<C> {
    /// Create a new `SuspensionManager` with the given max suspension time.
    ///
    /// Use [`DEFAULT_MAX_SUSPENSION_MS`] for the spec-default 5 minutes.
    pub fn new(max_suspension_ms: u64, clock: C) -> Self {
        SuspensionManager {
            clock,
            suspended: Vec::new(),
            max_suspension_ms,
        }
    }

    /// Create a `SuspensionManager` with the spec-default max suspension time.
    pub fn new_default(clock: C) -> Self {
        Self::new(DEFAULT_MAX_SUSPENSION_MS, clock)
    }

    // ── Safe-mode entry ───────────────────────────────────────────────────────

    /// Record a batch of ACTIVE lease IDs as suspended (safe-mode entry).
    ///
    /// Callers are responsible for:
    /// 1. Filtering to only ACTIVE leases before calling this.
    /// 2. Transitioning each lease's state to `LeaseState::Suspended` in the
    ///    scene graph.
    ///
    /// Returns `SafeModeResult` with the IDs that were registered.
    ///
    /// Spec: "This suspension MUST complete within 1 frame (16.6ms)." — enforced
    /// by the compositor frame loop; this function is synchronous and O(n) in the
    /// number of active leases.
    pub fn on_safe_mode_enter(
        &mut self,
        active_lease_ids: &[(SceneId, Option<u64>)],
    ) -> SafeModeResult {
        let now_ms = self.clock.now_millis();
        let mut suspended_lease_ids = Vec::with_capacity(active_lease_ids.len());
        for &(lease_id, ttl_remaining) in active_lease_ids {
            // Skip if already tracked (idempotent).
            if self.suspended.iter().any(|e| e.lease_id == lease_id) {
                continue;
            }
            self.suspended.push(SuspendedEntry {
                lease_id,
                suspended_at_ms: now_ms,
                ttl_remaining_at_suspend_ms: ttl_remaining,
            });
            suspended_lease_ids.push(lease_id);
        }
        // Keep sorted oldest-first for progressive revocation.
        self.suspended.sort_by_key(|e| e.suspended_at_ms);
        SafeModeResult {
            suspended_lease_ids,
        }
    }

    // ── Safe-mode exit ────────────────────────────────────────────────────────

    /// Resume all currently suspended leases (safe-mode exit).
    ///
    /// Callers are responsible for transitioning each lease state back to
    /// `LeaseState::Active` in the scene graph using the data returned.
    ///
    /// Spec: "Resume MUST complete within 2 frames (33.2ms)."
    ///
    /// Returns `SafeModeResumeResult` with per-lease resume data including
    /// `adjusted_expires_at_ms` and `suspension_duration_ms` for wire messages.
    pub fn on_safe_mode_exit(&mut self) -> SafeModeResumeResult {
        let now_ms = self.clock.now_millis();
        let mut resumed = Vec::with_capacity(self.suspended.len());

        for entry in self.suspended.drain(..) {
            let suspension_duration_ms = now_ms.saturating_sub(entry.suspended_at_ms);
            // Spec: adjusted_expires_at = granted_at + ttl + suspension_duration.
            // Here we only have ttl_remaining; if the caller also has granted_at and ttl_ms
            // they should compute it themselves.  We provide a convenience best-effort value:
            // adjusted = now + ttl_remaining (same net result for remaining TTL).
            let adjusted_expires_at_ms = entry
                .ttl_remaining_at_suspend_ms
                .map(|remaining| now_ms + remaining);
            resumed.push(LeaseResumeData {
                lease_id: entry.lease_id,
                resumed_at_ms: now_ms,
                suspension_duration_ms,
                adjusted_expires_at_ms,
            });
        }
        SafeModeResumeResult { resumed }
    }

    // ── Max suspension timeout ────────────────────────────────────────────────

    /// Identify leases that have exceeded the max suspension time.
    ///
    /// Spec §Max Suspension Time: "suspended leases MUST be progressively
    /// revoked (oldest first)".  Returns entries sorted oldest-suspended-first.
    ///
    /// Callers should:
    /// 1. Call `revoke_lease(entry.lease_id)` on each returned entry.
    /// 2. Call `remove_timeout_lease(entry.lease_id)` to clean up the manager.
    pub fn check_timeouts(&self) -> Vec<SuspensionTimeoutEntry> {
        let now_ms = self.clock.now_millis();
        self.suspended
            .iter()
            .filter(|e| now_ms.saturating_sub(e.suspended_at_ms) >= self.max_suspension_ms)
            .map(|e| SuspensionTimeoutEntry {
                lease_id: e.lease_id,
                suspension_duration_ms: now_ms.saturating_sub(e.suspended_at_ms),
            })
            .collect()
    }

    /// Remove a lease from the suspension tracker (after it has been revoked or resumed).
    pub fn remove_lease(&mut self, lease_id: SceneId) {
        self.suspended.retain(|e| e.lease_id != lease_id);
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Number of currently suspended leases tracked by this manager.
    pub fn suspended_count(&self) -> usize {
        self.suspended.len()
    }

    /// Whether any leases are currently suspended.
    pub fn has_suspended_leases(&self) -> bool {
        !self.suspended.is_empty()
    }

    /// How long a specific lease has been suspended (ms).  Returns 0 if not tracked.
    pub fn suspension_duration_ms(&self, lease_id: SceneId) -> u64 {
        let now_ms = self.clock.now_millis();
        self.suspended
            .iter()
            .find(|e| e.lease_id == lease_id)
            .map(|e| now_ms.saturating_sub(e.suspended_at_ms))
            .unwrap_or(0)
    }

    /// The max suspension time configured for this manager (ms).
    pub fn max_suspension_ms(&self) -> u64 {
        self.max_suspension_ms
    }
}

// ─── Suspension state preservation assertion ─────────────────────────────────

/// Assert the suspension state-preservation invariant:
///
/// When a lease transitions from ACTIVE to SUSPENDED and back to ACTIVE,
/// all tile IDs, node IDs, and zone publication state must remain unchanged.
///
/// This is a test helper, not a runtime guard.  Production code preserves
/// state by design (the scene graph never frees data on suspension).
///
/// # Panics
///
/// Panics if `tile_count_before != tile_count_after` or
/// `node_count_before != node_count_after`, providing a failure message.
pub fn assert_state_preserved(
    tile_count_before: usize,
    node_count_before: usize,
    tile_count_after: usize,
    node_count_after: usize,
) {
    assert_eq!(
        tile_count_before, tile_count_after,
        "Suspension violated state-preservation invariant: tile count changed \
         ({tile_count_before} → {tile_count_after})"
    );
    assert_eq!(
        node_count_before, node_count_after,
        "Suspension violated state-preservation invariant: node count changed \
         ({node_count_before} → {node_count_after})"
    );
}

// ─── LeaseState helpers ───────────────────────────────────────────────────────

/// Returns `true` if the lease state permits safe-mode suspension.
///
/// Only `Active` leases are suspended on safe-mode entry (spec: "all ACTIVE
/// leases MUST transition to SUSPENDED").  Already-suspended, terminal, or
/// orphaned leases are not touched.
pub fn is_suspendable(state: LeaseState) -> bool {
    state == LeaseState::Active
}

/// Returns `true` if the lease state can be resumed on safe-mode exit.
pub fn is_resumable(state: LeaseState) -> bool {
    state == LeaseState::Suspended
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;
    use crate::types::SceneId;

    fn make_manager(start_ms: u64) -> SuspensionManager<TestClock> {
        let clock = TestClock::new(start_ms);
        SuspensionManager::new_default(clock)
    }

    // ── Safe-mode entry ───────────────────────────────────────────────────────

    #[test]
    fn safe_mode_enter_registers_active_leases() {
        let mut mgr = make_manager(0);
        let id1 = SceneId::new();
        let id2 = SceneId::new();
        let result = mgr.on_safe_mode_enter(&[(id1, Some(50_000)), (id2, Some(30_000))]);
        assert_eq!(result.suspended_lease_ids.len(), 2);
        assert!(result.suspended_lease_ids.contains(&id1));
        assert!(result.suspended_lease_ids.contains(&id2));
        assert_eq!(mgr.suspended_count(), 2);
    }

    #[test]
    fn safe_mode_enter_idempotent_for_already_suspended() {
        let mut mgr = make_manager(0);
        let id = SceneId::new();
        mgr.on_safe_mode_enter(&[(id, Some(60_000))]);
        // Call again with the same ID — should not double-register.
        let result = mgr.on_safe_mode_enter(&[(id, Some(60_000))]);
        assert!(result.suspended_lease_ids.is_empty());
        assert_eq!(mgr.suspended_count(), 1);
    }

    #[test]
    fn safe_mode_enter_with_zero_leases() {
        let mut mgr = make_manager(0);
        let result = mgr.on_safe_mode_enter(&[]);
        assert!(result.suspended_lease_ids.is_empty());
        assert_eq!(mgr.suspended_count(), 0);
    }

    // ── Safe-mode exit ────────────────────────────────────────────────────────

    /// Spec §Safe Mode Resume: all SUSPENDED leases return to ACTIVE with same
    /// LeaseId.
    #[test]
    fn safe_mode_exit_resumes_all_suspended_leases() {
        let clock = TestClock::new(0);
        let mut mgr = SuspensionManager::new_default(clock.clone());
        let id1 = SceneId::new();
        let id2 = SceneId::new();
        mgr.on_safe_mode_enter(&[(id1, Some(50_000)), (id2, Some(30_000))]);

        clock.advance(2_000); // 2s in suspension
        let result = mgr.on_safe_mode_exit();

        assert_eq!(result.resumed.len(), 2);
        assert_eq!(mgr.suspended_count(), 0);
        for r in &result.resumed {
            assert!(
                r.suspension_duration_ms >= 1_900 && r.suspension_duration_ms <= 2_100,
                "expected ≈2_000ms, got {}",
                r.suspension_duration_ms
            );
        }
    }

    /// Spec §Resume: adjusted_expires_at_ms = now + ttl_remaining.
    #[test]
    fn safe_mode_exit_provides_adjusted_expiry() {
        let clock = TestClock::new(1_000);
        let mut mgr = SuspensionManager::new_default(clock.clone());
        let id = SceneId::new();
        mgr.on_safe_mode_enter(&[(id, Some(55_000))]);

        clock.advance(5_000); // suspended for 5s (resume at t=6_000)
        let result = mgr.on_safe_mode_exit();

        let resume_data = result.resumed.iter().find(|r| r.lease_id == id).unwrap();
        // adjusted_expires_at_ms = resume_at(6_000) + ttl_remaining(55_000) = 61_000
        assert_eq!(resume_data.adjusted_expires_at_ms, Some(61_000));
    }

    /// Spec §Resume: TTL clock correctly adjusted after suspension.
    ///
    /// Scenario: lease with ttl_ms=60_000 suspended for 10_000ms, then resumed.
    /// Effective expiry is extended by 10_000ms.
    #[test]
    fn resume_ttl_adjusted_within_100ms_tolerance() {
        let clock = TestClock::new(0);
        let mut mgr = SuspensionManager::new_default(clock.clone());
        let id = SceneId::new();
        // 60s lease, 0ms elapsed before suspension → 60_000ms remaining.
        mgr.on_safe_mode_enter(&[(id, Some(60_000))]);

        clock.advance(10_000); // 10s in suspension
        let result = mgr.on_safe_mode_exit();

        let r = result.resumed.iter().find(|r| r.lease_id == id).unwrap();
        // adjusted_expires_at_ms = 10_000 + 60_000 = 70_000
        // (i.e. the expiry is 10s further out than original 60s from t=0)
        let expected_expires = 70_000u64;
        let actual = r.adjusted_expires_at_ms.unwrap();
        assert!(
            actual >= expected_expires.saturating_sub(100) && actual <= expected_expires + 100,
            "expected adjusted expiry ≈{expected_expires}ms, got {actual}ms"
        );
    }

    // ── Max suspension timeout ────────────────────────────────────────────────

    /// Spec §Max Suspension Time: lease revoked after max_suspension_time_ms.
    #[test]
    fn check_timeouts_returns_expired_leases_after_max_time() {
        let clock = TestClock::new(0);
        let mut mgr = SuspensionManager::new(10_000, clock.clone()); // 10s for test speed
        let id = SceneId::new();
        mgr.on_safe_mode_enter(&[(id, Some(60_000))]);

        // Before timeout
        clock.advance(9_999);
        assert!(mgr.check_timeouts().is_empty());

        // At/after timeout
        clock.advance(2); // total = 10_001ms
        let timeouts = mgr.check_timeouts();
        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].lease_id, id);
        assert!(timeouts[0].suspension_duration_ms >= 10_001);
    }

    /// Spec §Max Suspension Time: progressive revocation oldest-first.
    #[test]
    fn check_timeouts_ordered_oldest_first() {
        let clock = TestClock::new(0);
        let mut mgr = SuspensionManager::new(5_000, clock.clone());

        let id_old = SceneId::new();
        let id_new = SceneId::new();

        // Suspend id_old first
        mgr.on_safe_mode_enter(&[(id_old, Some(60_000))]);
        clock.advance(2_000);
        // Suspend id_new 2s later
        mgr.on_safe_mode_enter(&[(id_new, Some(60_000))]);
        clock.advance(4_000); // total: id_old at 6_000ms, id_new at 4_000ms

        let timeouts = mgr.check_timeouts();
        // Only id_old should be timed out (6_000 ≥ 5_000; id_new at 4_000 < 5_000)
        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].lease_id, id_old);
    }

    /// Default max suspension time is 300,000ms (5 minutes).
    #[test]
    fn default_max_suspension_ms_is_5_minutes() {
        let mgr = make_manager(0);
        assert_eq!(mgr.max_suspension_ms(), DEFAULT_MAX_SUSPENSION_MS);
        assert_eq!(mgr.max_suspension_ms(), 300_000);
    }

    /// remove_lease() cleans up the entry so it doesn't appear in future timeouts.
    #[test]
    fn remove_lease_clears_timeout_tracking() {
        let clock = TestClock::new(0);
        let mut mgr = SuspensionManager::new(1_000, clock.clone());
        let id = SceneId::new();
        mgr.on_safe_mode_enter(&[(id, Some(60_000))]);

        clock.advance(2_000);
        let timeouts = mgr.check_timeouts();
        assert_eq!(timeouts.len(), 1);

        mgr.remove_lease(id);
        assert!(mgr.check_timeouts().is_empty());
        assert_eq!(mgr.suspended_count(), 0);
    }

    // ── suspension_duration_ms ─────────────────────────────────────────────────

    #[test]
    fn suspension_duration_ms_tracks_elapsed_time() {
        let clock = TestClock::new(0);
        let mut mgr = SuspensionManager::new_default(clock.clone());
        let id = SceneId::new();
        mgr.on_safe_mode_enter(&[(id, Some(60_000))]);

        clock.advance(7_000);
        let dur = mgr.suspension_duration_ms(id);
        assert!(
            (6_900..=7_100).contains(&dur),
            "expected ≈7_000ms, got {dur}"
        );
    }

    #[test]
    fn suspension_duration_ms_returns_zero_for_unknown_lease() {
        let mgr = make_manager(0);
        let unknown = SceneId::new();
        assert_eq!(mgr.suspension_duration_ms(unknown), 0);
    }

    // ── State preservation ────────────────────────────────────────────────────

    #[test]
    fn state_preserved_assertion_passes_when_equal() {
        // Should not panic
        assert_state_preserved(5, 10, 5, 10);
    }

    #[test]
    #[should_panic(expected = "tile count changed")]
    fn state_preserved_assertion_panics_on_tile_change() {
        assert_state_preserved(5, 10, 4, 10);
    }

    #[test]
    #[should_panic(expected = "node count changed")]
    fn state_preserved_assertion_panics_on_node_change() {
        assert_state_preserved(5, 10, 5, 9);
    }

    // ── Helper functions ──────────────────────────────────────────────────────

    #[test]
    fn is_suspendable_only_for_active() {
        assert!(is_suspendable(LeaseState::Active));
        assert!(!is_suspendable(LeaseState::Suspended));
        assert!(!is_suspendable(LeaseState::Orphaned));
        assert!(!is_suspendable(LeaseState::Revoked));
        assert!(!is_suspendable(LeaseState::Expired));
    }

    #[test]
    fn is_resumable_only_for_suspended() {
        assert!(is_resumable(LeaseState::Suspended));
        assert!(!is_resumable(LeaseState::Active));
        assert!(!is_resumable(LeaseState::Orphaned));
        assert!(!is_resumable(LeaseState::Revoked));
    }

    // ── End-to-end: full safe-mode cycle ─────────────────────────────────────

    /// Full safe-mode cycle: enter → wait → exit; verify no state was lost.
    ///
    /// This test simulates the sequence at the SuspensionManager level.
    /// The actual scene-graph state preservation is tested in session_lifecycle.rs.
    #[test]
    fn full_safe_mode_cycle_enter_and_exit() {
        let clock = TestClock::new(1_000);
        let mut mgr = SuspensionManager::new_default(clock.clone());

        let lease_a = SceneId::new();
        let lease_b = SceneId::new();

        // Enter safe mode with 2 active leases
        let enter_result =
            mgr.on_safe_mode_enter(&[(lease_a, Some(60_000)), (lease_b, Some(30_000))]);
        assert_eq!(enter_result.suspended_lease_ids.len(), 2);
        assert!(mgr.has_suspended_leases());

        // 2 minutes pass — still within 5-minute max
        clock.advance(120_000);
        assert!(mgr.check_timeouts().is_empty());

        // Exit safe mode
        let exit_result = mgr.on_safe_mode_exit();
        assert_eq!(exit_result.resumed.len(), 2);
        assert!(!mgr.has_suspended_leases());

        // Verify per-lease resume data
        for r in &exit_result.resumed {
            // suspension_duration_ms ≈ 120_000
            assert!(
                r.suspension_duration_ms >= 119_900 && r.suspension_duration_ms <= 120_100,
                "expected ≈120_000ms, got {}",
                r.suspension_duration_ms
            );
        }
    }

    /// Suspension timeout scenario: lease suspended for > 5 minutes → must be revoked.
    #[test]
    fn suspension_timeout_scenario_5_minutes() {
        let clock = TestClock::new(0);
        let mut mgr = SuspensionManager::new_default(clock.clone());
        let id = SceneId::new();

        mgr.on_safe_mode_enter(&[(id, Some(60_000))]);

        // Advance past 5-minute max
        clock.advance(300_001);
        let timeouts = mgr.check_timeouts();
        assert_eq!(timeouts.len(), 1);
        assert_eq!(timeouts[0].lease_id, id);

        // Simulate caller revoking the lease
        mgr.remove_lease(id);
        assert_eq!(mgr.suspended_count(), 0);
    }
}
