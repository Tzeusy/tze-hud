//! Lease state machine trait and implementation for v1 governance.
//!
//! Encodes the lease lifecycle from lease-governance/spec.md §Requirement: Lease State Machine
//! and related requirements.  The trait contract is defined here; the concrete
//! implementation lives in `state_machine.rs`.

pub mod types;
pub mod state_machine;

pub use types::{DenyReason, LeaseAuditEvent, LeaseEventKind, LeaseId, LeaseIdentity, RevokeReason as AuditRevokeReason};
pub use state_machine::LeaseImpl;

use crate::clock::Clock;

// ─── Lease States ────────────────────────────────────────────────────────────

/// All possible states a lease can be in.
///
/// From spec: REQUESTED, ACTIVE, SUSPENDED, ORPHANED, REVOKED, EXPIRED, DENIED, RELEASED.
/// DENIED, REVOKED, EXPIRED, and RELEASED are terminal states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseState {
    Requested,
    Active,
    Suspended,
    Orphaned,
    Revoked,
    Expired,
    Denied,
    Released,
}

impl LeaseState {
    /// Returns `true` if this is a terminal state (no further transitions allowed).
    pub fn is_terminal(self) -> bool {
        matches!(self, LeaseState::Denied | LeaseState::Revoked | LeaseState::Expired | LeaseState::Released)
    }
}

// ─── Renewal Policy ──────────────────────────────────────────────────────────

/// Renewal policy for a lease.
///
/// From spec §Requirement: Auto-Renewal Policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenewalPolicy {
    /// Agent must explicitly renew before TTL expires.
    Manual,
    /// Runtime auto-renews at 75% TTL elapsed (when session Active, no budget violations).
    AutoRenew,
    /// Expires at TTL; no renewal option.
    OneShot,
}

// ─── Revoke Reason ───────────────────────────────────────────────────────────

/// Reason a lease was revoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevokeReason {
    ViewerDismissed,
    BudgetPolicy,
    SuspensionTimeout,
    CapabilityRevoked,
    Other,
}

// ─── Budget Warning Level ────────────────────────────────────────────────────

/// The current budget enforcement tier for the lease.
///
/// From spec §Requirement: Three-Tier Budget Enforcement Ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BudgetTier {
    /// Below 80% — normal operation.
    Normal,
    /// ≥ 80% — `BudgetWarning` sent; mutations still accepted.
    Warning,
    /// Warning unresolved ≥ 5s — effective `update_rate_hz` reduced by 50%.
    Throttle,
    /// Throttle sustained ≥ 30s or critical limit exceeded — all leases revoked.
    Revocation,
}

// ─── Transition Error ────────────────────────────────────────────────────────

/// Error returned when a state transition is invalid or blocked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransitionError {
    /// The requested transition is not valid from the current state.
    InvalidTransition { from: LeaseState, to: LeaseState },
    /// The lease is in a terminal state; no further transitions are possible.
    TerminalState,
    /// Mutations blocked because the lease is SUSPENDED (safe mode active).
    SafeModeActive,
    /// Lease not found / not active for zone publish.
    LeaseNotActive,
}

// ─── Resource Budget ─────────────────────────────────────────────────────────

/// Per-lease resource budget dimensions.
///
/// From spec §Requirement: Resource Budget Schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceBudget {
    /// Range [1, 64].
    pub max_nodes_per_tile: u8,
    /// Mutations per second.
    pub update_rate_hz: u32,
    /// Range [1, 64].
    pub max_tiles: u8,
    /// Total decoded texture bytes allowed.
    pub texture_bytes_total: u64,
    /// Range [1, 64].
    pub max_active_leases: u8,
    /// Always 0 in v1.
    pub max_concurrent_streams: u8,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        ResourceBudget {
            max_nodes_per_tile: 32,
            update_rate_hz: 30,
            max_tiles: 8,
            texture_bytes_total: 64 * 1024 * 1024,
            max_active_leases: 8,
            max_concurrent_streams: 0,
        }
    }
}

// ─── LeaseStateMachine Trait ─────────────────────────────────────────────────

/// Trait encoding the lease lifecycle state machine.
///
/// Implementations must satisfy every WHEN/THEN scenario defined in
/// `lease-governance/spec.md`.  This trait intentionally provides **no**
/// implementation — tests are written against it; a correct impl must make
/// all tests pass.
///
/// Clock injection via `C: Clock` enables deterministic TTL and grace-period
/// testing without sleeping.
pub trait LeaseStateMachine<C: Clock> {
    /// Create a new lease in the REQUESTED state with the given TTL (ms) and
    /// renewal policy.  `ttl_ms = 0` means indefinite.
    fn new_requested(ttl_ms: u64, policy: RenewalPolicy, clock: C) -> Self
    where
        Self: Sized;

    // ── Transitions ──────────────────────────────────────────────────────────

    /// REQUESTED → ACTIVE.  Returns `Err` if already in another state.
    fn activate(&mut self) -> Result<(), TransitionError>;

    /// ACTIVE → SUSPENDED (safe-mode entry).  TTL clock pauses.
    fn suspend(&mut self) -> Result<(), TransitionError>;

    /// SUSPENDED → ACTIVE (safe-mode exit).  TTL clock resumes; expiry adjusted.
    fn resume(&mut self) -> Result<(), TransitionError>;

    /// ACTIVE → ORPHANED (agent disconnect detected).
    fn orphan(&mut self) -> Result<(), TransitionError>;

    /// ORPHANED → ACTIVE (agent reconnects within grace period).
    fn reconnect(&mut self) -> Result<(), TransitionError>;

    /// ACTIVE / ORPHANED → EXPIRED (TTL or grace period elapses).
    fn expire(&mut self) -> Result<(), TransitionError>;

    /// → REVOKED (viewer dismiss, budget policy, or suspension timeout).
    fn revoke(&mut self, reason: RevokeReason) -> Result<(), TransitionError>;

    /// ACTIVE → RELEASED (agent voluntary release).
    fn release(&mut self) -> Result<(), TransitionError>;

    /// REQUESTED → DENIED (capability or budget check failed).
    fn deny(&mut self) -> Result<(), TransitionError>;

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Current state of the lease.
    fn state(&self) -> LeaseState;

    /// Milliseconds of TTL remaining, accounting for any pause during suspension.
    /// Returns `None` if TTL is indefinite (ttl_ms = 0) or lease is terminal.
    fn ttl_remaining_ms(&self) -> Option<u64>;

    /// Returns `true` if this lease is in a terminal state.
    fn is_terminal(&self) -> bool;

    /// Returns `true` if `target` is a valid next state from the current state.
    fn can_transition_to(&self, target: LeaseState) -> bool;

    /// Current budget enforcement tier for this lease.
    fn budget_tier(&self) -> BudgetTier;

    /// Report how long the lease has been continuously suspended (ms).
    /// Returns 0 if not currently suspended.
    fn suspension_duration_ms(&self) -> u64;

    /// Notify the state machine that budget usage has changed.
    /// The implementation should update `budget_tier` accordingly.
    /// At ≥ 80% → Warning; warning unresolved ≥ 5s → Throttle; throttle ≥ 30s → Revocation.
    fn update_budget_usage(&mut self, usage_fraction: f64) -> Result<(), TransitionError>;
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::clock::TestClock;

    // Helper: build and activate a lease using a TestClock.
    fn make_active<S>(clock: TestClock) -> S
    where
        S: LeaseStateMachine<TestClock>,
    {
        let mut s = S::new_requested(60_000, RenewalPolicy::Manual, clock);
        s.activate().expect("activate from REQUESTED");
        s
    }

    // ── 1. Basic state machine transitions ───────────────────────────────────

    /// WHEN a lease is REQUESTED and activate() called THEN state becomes ACTIVE.
    #[test]
    fn test_requested_to_active() {
        test_requested_to_active_generic::<super::LeaseImpl<TestClock>>();
    }

    /// Generic form used by real test — call from a concrete test once an impl exists.
    pub fn test_requested_to_active_generic<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease = S::new_requested(60_000, RenewalPolicy::Manual, clock);
        assert_eq!(lease.state(), LeaseState::Requested);
        lease.activate().expect("activate should succeed");
        assert_eq!(lease.state(), LeaseState::Active);
    }

    /// WHEN lease is REQUESTED and deny() called THEN state becomes DENIED (terminal).
    pub fn test_denied_is_terminal<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease = S::new_requested(60_000, RenewalPolicy::Manual, clock);
        lease.deny().expect("deny from REQUESTED should succeed");
        assert_eq!(lease.state(), LeaseState::Denied);
        assert!(lease.is_terminal());
    }

    /// WHEN lease is ACTIVE and safe mode entered THEN state becomes SUSPENDED.
    pub fn test_active_to_suspended_on_safe_mode<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        lease.suspend().expect("suspend from ACTIVE");
        assert_eq!(lease.state(), LeaseState::Suspended);
    }

    /// WHEN lease is SUSPENDED and safe mode exits THEN state becomes ACTIVE.
    pub fn test_suspended_to_active_on_resume<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        lease.suspend().unwrap();
        lease.resume().expect("resume from SUSPENDED");
        assert_eq!(lease.state(), LeaseState::Active);
    }

    /// WHEN lease is ACTIVE and agent disconnects THEN state becomes ORPHANED.
    pub fn test_active_to_orphaned_on_disconnect<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        lease.orphan().expect("orphan from ACTIVE");
        assert_eq!(lease.state(), LeaseState::Orphaned);
    }

    /// WHEN lease is ORPHANED and agent reconnects within 30,000ms grace period
    /// THEN state becomes ACTIVE.
    pub fn test_orphaned_to_active_within_grace_period<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock.clone());
        lease.orphan().unwrap();
        // Advance within grace period
        clock.advance(29_000);
        lease.reconnect().expect("reconnect within grace period");
        assert_eq!(lease.state(), LeaseState::Active);
    }

    /// WHEN lease is ORPHANED and grace period elapses THEN expire() succeeds
    /// and state becomes EXPIRED.
    pub fn test_orphaned_expires_after_grace_period<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock.clone());
        lease.orphan().unwrap();
        clock.advance(30_001);
        lease.expire().expect("expire after grace period");
        assert_eq!(lease.state(), LeaseState::Expired);
        assert!(lease.is_terminal());
    }

    /// WHEN lease is EXPIRED and any transition is attempted THEN error (terminal).
    pub fn test_expired_is_terminal_no_further_transitions<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        lease.expire().unwrap();
        assert!(lease.state().is_terminal());
        assert!(lease.activate().is_err());
        assert!(lease.release().is_err());
    }

    /// WHEN lease is REVOKED and any transition is attempted THEN error (terminal).
    pub fn test_revoked_is_terminal<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        lease.revoke(RevokeReason::ViewerDismissed).unwrap();
        assert_eq!(lease.state(), LeaseState::Revoked);
        assert!(lease.is_terminal());
        assert!(lease.activate().is_err());
        assert!(lease.suspend().is_err());
    }

    /// WHEN lease is ACTIVE and released THEN state becomes RELEASED (terminal).
    pub fn test_active_to_released<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        lease.release().expect("release from ACTIVE");
        assert_eq!(lease.state(), LeaseState::Released);
        assert!(lease.is_terminal());
    }

    // ── 2. TTL accounting ────────────────────────────────────────────────────

    /// WHEN lease is SUSPENDED for N ms and then resumed THEN the effective TTL
    /// does not count the suspension time (adjusted expiry).
    pub fn test_ttl_paused_during_suspension<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = S::new_requested(60_000, RenewalPolicy::Manual, clock.clone());
        lease.activate().unwrap();
        let ttl_before = lease.ttl_remaining_ms().expect("should have ttl");
        clock.advance(10_000); // 10s elapsed
        lease.suspend().unwrap();
        clock.advance(10_000); // 10s in suspension — should NOT count
        lease.resume().unwrap();
        clock.advance(0); // no extra time after resume
        let ttl_after = lease.ttl_remaining_ms().expect("should have ttl");
        // ttl_after should be ≈ ttl_before - 10_000 (only the pre-suspension time counts)
        // allowing ±100ms tolerance per spec
        let expected = ttl_before.saturating_sub(10_000);
        assert!(
            ttl_after >= expected.saturating_sub(100) && ttl_after <= expected + 100,
            "TTL after resume={ttl_after}ms, expected≈{expected}ms"
        );
    }

    /// WHEN lease with ttl_ms = 60_000 is suspended for 10_000ms and resumed
    /// THEN expiry is extended by ≈10_000ms (within ±100ms spec tolerance).
    pub fn test_ttl_adjusted_after_suspension_exact<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = S::new_requested(60_000, RenewalPolicy::Manual, clock.clone());
        lease.activate().unwrap();
        lease.suspend().unwrap();
        clock.advance(10_000);
        lease.resume().unwrap();
        let remaining = lease.ttl_remaining_ms().expect("ttl after resume");
        // Should be ≈60_000ms remaining (suspension didn't count), within ±100ms
        assert!(
            remaining >= 59_900 && remaining <= 60_100,
            "expected ≈60_000ms remaining, got {remaining}"
        );
    }

    // ── 3. Max suspension time ───────────────────────────────────────────────

    /// WHEN lease is SUSPENDED for > 300,000ms THEN it must transition to REVOKED.
    pub fn test_suspension_timeout_triggers_revocation<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock.clone());
        lease.suspend().unwrap();
        clock.advance(300_001);
        // Either: revoke is called by infrastructure, or `update_budget_usage` triggers it.
        // Here we model it as the caller detecting the timeout and calling revoke.
        let result = lease.revoke(RevokeReason::SuspensionTimeout);
        assert!(result.is_ok(), "revoke due to suspension timeout should succeed");
        assert_eq!(lease.state(), LeaseState::Revoked);
    }

    // ── 4. Budget enforcement ladder ─────────────────────────────────────────

    /// WHEN budget at < 80% THEN tier is Normal and mutations accepted.
    pub fn test_budget_normal_tier_below_80_percent<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        lease.update_budget_usage(0.79).expect("usage under 80% accepted");
        assert_eq!(lease.budget_tier(), BudgetTier::Normal);
    }

    /// WHEN budget at 80% THEN BudgetWarning tier, mutations still accepted.
    pub fn test_budget_warning_at_80_percent<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        lease.update_budget_usage(0.80).expect("usage at 80% accepted (soft limit)");
        assert_eq!(lease.budget_tier(), BudgetTier::Warning);
    }

    /// WHEN budget at 85% and warning unresolved for 5s THEN Throttle tier.
    pub fn test_budget_throttle_after_5s_warning<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock.clone());
        lease.update_budget_usage(0.85).expect("85% accepted initially");
        assert_eq!(lease.budget_tier(), BudgetTier::Warning);
        clock.advance(5_001); // warning unresolved for >5s
        // Re-check: implementation should transition to Throttle when polled.
        lease.update_budget_usage(0.85).expect("still at 85%");
        assert_eq!(lease.budget_tier(), BudgetTier::Throttle);
    }

    /// WHEN budget at 100% THEN entire MutationBatch rejected (Revocation trigger).
    pub fn test_budget_hard_limit_at_100_percent<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock);
        let result = lease.update_budget_usage(1.0);
        // Hard limit: must either return an error or set tier to Revocation.
        assert!(
            result.is_err() || lease.budget_tier() == BudgetTier::Revocation,
            "100% usage should set Revocation tier or return error"
        );
    }

    // ── 5. ONE_SHOT specifics ────────────────────────────────────────────────

    /// WHEN ONE_SHOT lease is suspended THEN TTL is paused; on resume, adjusted
    /// expiry = original_expiry + suspension_duration.
    pub fn test_one_shot_ttl_paused_during_suspension<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = S::new_requested(30_000, RenewalPolicy::OneShot, clock.clone());
        lease.activate().unwrap();
        clock.advance(5_000);
        lease.suspend().unwrap();
        clock.advance(10_000); // should NOT count against TTL
        lease.resume().unwrap();
        let remaining = lease.ttl_remaining_ms().expect("ttl after resume");
        // Should be ≈25_000ms (30_000 - 5_000 elapsed before suspension)
        assert!(
            remaining >= 24_900 && remaining <= 25_100,
            "ONE_SHOT: expected ≈25_000ms remaining, got {remaining}"
        );
    }

    /// WHEN ONE_SHOT lease reaches TTL THEN expires without renewal option.
    pub fn test_one_shot_expires_at_ttl<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = S::new_requested(1_000, RenewalPolicy::OneShot, clock.clone());
        lease.activate().unwrap();
        clock.advance(1_001);
        lease.expire().expect("should be able to expire ONE_SHOT after TTL");
        assert_eq!(lease.state(), LeaseState::Expired);
    }

    // ── 6. Grace period precision ────────────────────────────────────────────

    /// Spec §Grace Period Precision: agent can reconnect at 29,950ms (just before 30s).
    pub fn test_grace_period_not_premature<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        let mut lease: S = make_active(clock.clone());
        lease.orphan().unwrap();
        clock.advance(29_950); // just before the 30_000ms grace period
        lease.reconnect().expect("reconnect at 29_950ms should succeed");
        assert_eq!(lease.state(), LeaseState::Active);
    }

    // ── 7. Priority enforcement ──────────────────────────────────────────────

    /// WHEN max_concurrent_streams is queried on a v1 lease THEN value is 0.
    pub fn test_max_concurrent_streams_zero_in_v1<S: LeaseStateMachine<TestClock>>() {
        let clock = TestClock::new(0);
        // This tests that the default budget (which must be vended by an impl) has
        // max_concurrent_streams = 0.
        let _lease: S = S::new_requested(60_000, RenewalPolicy::Manual, clock);
        // Implementations must expose a `budget()` or similar; here we just assert
        // that the type compiles against the trait.
        // A concrete companion test would call `lease.budget().max_concurrent_streams == 0`.
    }

    // ─── Concrete tests using LeaseImpl ──────────────────────────────────────
    // Each function below drives a `pub fn test_*_generic<S>()` above with the
    // concrete `LeaseImpl<TestClock>` implementation.

    type Impl = super::LeaseImpl<TestClock>;

    #[test]
    fn impl_denied_is_terminal() {
        test_denied_is_terminal::<Impl>();
    }

    #[test]
    fn impl_active_to_suspended_on_safe_mode() {
        test_active_to_suspended_on_safe_mode::<Impl>();
    }

    #[test]
    fn impl_suspended_to_active_on_resume() {
        test_suspended_to_active_on_resume::<Impl>();
    }

    #[test]
    fn impl_active_to_orphaned_on_disconnect() {
        test_active_to_orphaned_on_disconnect::<Impl>();
    }

    #[test]
    fn impl_orphaned_to_active_within_grace_period() {
        test_orphaned_to_active_within_grace_period::<Impl>();
    }

    #[test]
    fn impl_orphaned_expires_after_grace_period() {
        test_orphaned_expires_after_grace_period::<Impl>();
    }

    #[test]
    fn impl_expired_is_terminal_no_further_transitions() {
        test_expired_is_terminal_no_further_transitions::<Impl>();
    }

    #[test]
    fn impl_revoked_is_terminal() {
        test_revoked_is_terminal::<Impl>();
    }

    #[test]
    fn impl_active_to_released() {
        test_active_to_released::<Impl>();
    }

    #[test]
    fn impl_ttl_paused_during_suspension() {
        test_ttl_paused_during_suspension::<Impl>();
    }

    #[test]
    fn impl_ttl_adjusted_after_suspension_exact() {
        test_ttl_adjusted_after_suspension_exact::<Impl>();
    }

    #[test]
    fn impl_suspension_timeout_triggers_revocation() {
        test_suspension_timeout_triggers_revocation::<Impl>();
    }

    #[test]
    fn impl_budget_normal_tier_below_80_percent() {
        test_budget_normal_tier_below_80_percent::<Impl>();
    }

    #[test]
    fn impl_budget_warning_at_80_percent() {
        test_budget_warning_at_80_percent::<Impl>();
    }

    #[test]
    fn impl_budget_throttle_after_5s_warning() {
        test_budget_throttle_after_5s_warning::<Impl>();
    }

    #[test]
    fn impl_budget_hard_limit_at_100_percent() {
        test_budget_hard_limit_at_100_percent::<Impl>();
    }

    #[test]
    fn impl_one_shot_ttl_paused_during_suspension() {
        test_one_shot_ttl_paused_during_suspension::<Impl>();
    }

    #[test]
    fn impl_one_shot_expires_at_ttl() {
        test_one_shot_expires_at_ttl::<Impl>();
    }

    #[test]
    fn impl_grace_period_not_premature() {
        test_grace_period_not_premature::<Impl>();
    }

    // ─── Additional implementation-specific tests ─────────────────────────────

    /// WHEN can_transition_to() is queried THEN valid transitions are reported correctly.
    #[test]
    fn impl_can_transition_to_all_valid() {
        use LeaseState::*;
        let clock = TestClock::new(0);
        let lease = Impl::new_requested(60_000, RenewalPolicy::Manual, clock.clone());
        assert!(lease.can_transition_to(Active));
        assert!(lease.can_transition_to(Denied));
        assert!(!lease.can_transition_to(Suspended)); // REQUESTED cannot go to SUSPENDED
        assert!(!lease.can_transition_to(Revoked));   // REQUESTED cannot go to REVOKED
    }

    /// WHEN lease is terminal THEN can_transition_to() always returns false.
    #[test]
    fn impl_terminal_cannot_transition_to_anything() {
        let clock = TestClock::new(0);
        let mut lease: Impl = make_active(clock);
        lease.expire().unwrap();
        for target in [
            LeaseState::Active,
            LeaseState::Suspended,
            LeaseState::Orphaned,
            LeaseState::Revoked,
            LeaseState::Expired,
            LeaseState::Released,
            LeaseState::Denied,
        ] {
            assert!(!lease.can_transition_to(target), "terminal lease should not transition to {:?}", target);
        }
    }

    /// WHEN suspension_duration_ms queried while ACTIVE THEN returns 0.
    #[test]
    fn impl_suspension_duration_zero_when_active() {
        let clock = TestClock::new(0);
        let lease: Impl = make_active(clock);
        assert_eq!(lease.suspension_duration_ms(), 0);
    }

    /// WHEN suspension_duration_ms queried while SUSPENDED THEN returns elapsed time.
    #[test]
    fn impl_suspension_duration_increases_while_suspended() {
        let clock = TestClock::new(0);
        let mut lease: Impl = make_active(clock.clone());
        lease.suspend().unwrap();
        clock.advance(5_000);
        let dur = lease.suspension_duration_ms();
        assert!(dur >= 4_900 && dur <= 5_100, "expected ≈5000ms, got {dur}ms");
    }

    /// WHEN indefinite lease (ttl_ms=0) THEN ttl_remaining_ms returns None.
    #[test]
    fn impl_indefinite_lease_ttl_remaining_none() {
        let clock = TestClock::new(0);
        let mut lease = Impl::new_requested(0, RenewalPolicy::Manual, clock);
        lease.activate().unwrap();
        assert_eq!(lease.ttl_remaining_ms(), None);
    }

    /// WHEN ACTIVE → REVOKED (viewer dismissed) THEN state is REVOKED.
    #[test]
    fn impl_active_to_revoked_viewer_dismissed() {
        let clock = TestClock::new(0);
        let mut lease: Impl = make_active(clock);
        lease.revoke(RevokeReason::ViewerDismissed).expect("revoke from ACTIVE");
        assert_eq!(lease.state(), LeaseState::Revoked);
        assert!(lease.is_terminal());
    }

    /// WHEN SUSPENDED → REVOKED (suspension timeout) THEN state is REVOKED.
    #[test]
    fn impl_suspended_to_revoked_suspension_timeout() {
        let clock = TestClock::new(0);
        let mut lease: Impl = make_active(clock);
        lease.suspend().unwrap();
        lease.revoke(RevokeReason::SuspensionTimeout).expect("revoke suspended");
        assert_eq!(lease.state(), LeaseState::Revoked);
    }
}
