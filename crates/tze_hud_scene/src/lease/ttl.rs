//! TTL timer and auto-renewal logic for lease lifecycle management.
//!
//! Implements spec §Requirement: Auto-Renewal Policy (lease-governance/spec.md lines 71-91)
//! and §Requirement: TTL Accounting Precision (spec lines 289-296).
//!
//! ## Responsibility
//!
//! This module manages the **timing layer** above the core state machine:
//!
//! - [`TtlState`] tracks how much TTL has been consumed, pausing during suspension.
//! - [`AutoRenewalArm`] encodes whether the 75%-elapsed renewal timer is armed.
//! - [`TtlCheck`] is the result type returned when the session layer polls a lease.
//!
//! ## Clock convention
//!
//! All timestamps use milliseconds from the injected [`Clock::now_millis()`].
//! Precision: ±100 ms per spec §TTL Accounting Precision.

use super::RenewalPolicy;
use crate::clock::Clock;

// ─── Constants ───────────────────────────────────────────────────────────────

/// Auto-renewal fires when this fraction of TTL has elapsed.
///
/// Spec §Requirement: Auto-Renewal Policy: "runtime auto-renews at 75% TTL elapsed".
pub const AUTO_RENEW_THRESHOLD: f64 = 0.75;

// ─── AutoRenewalArm ──────────────────────────────────────────────────────────

/// Whether the auto-renewal timer is currently armed for a lease.
///
/// The timer is disarmed when:
/// 1. The agent enters budget-warning state.
/// 2. The session enters `Disconnecting` state.
/// 3. Safe mode is entered (TTL clock is paused; timer resumes on safe-mode exit).
///
/// For `Manual` and `OneShot` policies the arm state is always `NotApplicable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoRenewalArm {
    /// Policy is MANUAL or ONE_SHOT — timer is never armed.
    NotApplicable,
    /// Policy is AUTO_RENEW and the timer is currently armed.
    Armed,
    /// Policy is AUTO_RENEW but the timer was explicitly disarmed (budget warning,
    /// disconnecting session, or safe mode).
    Disarmed,
}

// ─── DisarmReason ────────────────────────────────────────────────────────────

/// Why the auto-renewal timer was disarmed.
///
/// The reason is stored in `TtlState` so that `on_resume` can correctly
/// distinguish a safe-mode disarm (which MUST re-arm on safe-mode exit) from
/// a budget-warning or session-disconnecting disarm (which MUST NOT be
/// re-armed automatically — the session layer must call `rearm_renewal`
/// explicitly when the condition clears).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisarmReason {
    BudgetWarning,
    SessionDisconnecting,
    SafeMode,
    /// Timer re-armed after a transient disarm (e.g. budget warning cleared).
    Rearm,
}

// ─── TtlCheck ────────────────────────────────────────────────────────────────

/// Result of a TTL poll — what the session layer should do for this lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TtlCheck {
    /// Lease is healthy; nothing to do.
    Ok,
    /// AUTO_RENEW policy: 75% threshold reached — runtime should renew now.
    AutoRenewDue,
    /// TTL has elapsed — lease should be expired.
    Expired,
    /// Lease has no TTL (indefinite).
    Indefinite,
}

// ─── TtlState ────────────────────────────────────────────────────────────────

/// Per-lease TTL accounting state.
///
/// Tracks elapsed time while correctly excluding suspension periods.
///
/// ## Precision guarantee
///
/// The effective expiry formula (per spec §TTL Accounting Precision) is:
///
/// ```text
/// effective_expiry = granted_at_wall_us + (ttl_ms * 1000) + suspension_duration_us
/// ```
///
/// This implementation stores `granted_at_ms` and `total_suspension_ms` and
/// derives remaining TTL as:
///
/// ```text
/// remaining = ttl_ms - ((now_ms - granted_at_ms) - total_suspension_ms)
/// ```
///
/// Accuracy is bounded by clock resolution (±1 ms in tests with `TestClock`;
/// ±a few ms with `SystemClock`) — well within the ±100 ms spec tolerance.
#[derive(Clone, Debug)]
pub struct TtlState<C: Clock> {
    clock: C,
    /// Original TTL in milliseconds.  0 = indefinite.
    ttl_ms: u64,
    /// Wall-clock time when the lease was activated (ms).
    granted_at_ms: u64,
    /// Total accumulated suspension time (ms) — excluded from TTL consumption.
    total_suspension_ms: u64,
    /// Timestamp when the current suspension started, if currently suspended.
    suspended_at_ms: Option<u64>,
    /// TTL remaining at the moment suspension started (ms).
    ttl_remaining_at_suspend_ms: Option<u64>,
    /// Renewal policy.
    renewal_policy: RenewalPolicy,
    /// Whether the auto-renewal timer is armed.
    auto_renewal_arm: AutoRenewalArm,
    /// Why the auto-renewal timer was disarmed.
    ///
    /// `None` when the timer is Armed or NotApplicable.  Stored so that
    /// `on_resume` only re-arms timers that were disarmed *because of safe mode*;
    /// budget-warning and session-disconnecting disarms are NOT auto-reversed.
    disarm_reason: Option<DisarmReason>,
    /// Whether a renewal has already been fired for the current TTL window
    /// (prevents duplicate AUTO_RENEW events before the session layer resets).
    renewal_fired: bool,
}

impl<C: Clock> TtlState<C> {
    /// Create a new `TtlState` for a lease that has just been activated.
    ///
    /// `clock.now_millis()` is captured as `granted_at_ms`.
    pub fn new_activated(ttl_ms: u64, renewal_policy: RenewalPolicy, clock: C) -> Self {
        let granted_at_ms = clock.now_millis();
        let auto_renewal_arm = match renewal_policy {
            RenewalPolicy::AutoRenew => AutoRenewalArm::Armed,
            _ => AutoRenewalArm::NotApplicable,
        };
        TtlState {
            clock,
            ttl_ms,
            granted_at_ms,
            total_suspension_ms: 0,
            suspended_at_ms: None,
            ttl_remaining_at_suspend_ms: None,
            renewal_policy,
            auto_renewal_arm,
            disarm_reason: None,
            renewal_fired: false,
        }
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Milliseconds of TTL remaining.  Returns `None` for indefinite leases.
    ///
    /// Accounts for suspension: time spent SUSPENDED does not count.
    pub fn remaining_ms(&self) -> Option<u64> {
        if self.ttl_ms == 0 {
            return None;
        }
        Some(self.remaining_ms_at(self.clock.now_millis()))
    }

    /// TTL remaining at a specific `now_ms` (for deterministic testing).
    pub fn remaining_ms_at(&self, now_ms: u64) -> u64 {
        if self.ttl_ms == 0 {
            return u64::MAX;
        }
        // If suspended, TTL is frozen at the saved value.
        if let Some(frozen) = self.ttl_remaining_at_suspend_ms {
            return frozen;
        }
        let elapsed = now_ms.saturating_sub(self.granted_at_ms);
        let effective_elapsed = elapsed.saturating_sub(self.total_suspension_ms);
        self.ttl_ms.saturating_sub(effective_elapsed)
    }

    /// Total suspension duration accumulated so far (ms).
    ///
    /// Includes any ongoing suspension up to `now`.
    pub fn total_suspension_ms(&self) -> u64 {
        let ongoing = match self.suspended_at_ms {
            Some(at) => self.clock.now_millis().saturating_sub(at),
            None => 0,
        };
        self.total_suspension_ms + ongoing
    }

    /// Expiry wall-clock timestamp (ms).  `None` for indefinite leases.
    ///
    /// Per spec: `effective_expiry_ms = granted_at_ms + ttl_ms + total_suspension_ms`.
    /// Call after resuming from suspension to get the accurate adjusted value.
    ///
    /// Uses `saturating_add` to avoid overflow on pathological inputs.
    pub fn adjusted_expires_at_ms(&self) -> Option<u64> {
        if self.ttl_ms == 0 {
            return None;
        }
        Some(
            self.granted_at_ms
                .saturating_add(self.ttl_ms)
                .saturating_add(self.total_suspension_ms()),
        )
    }

    /// Poll the lease for timer events the session layer needs to act on.
    ///
    /// Returns [`TtlCheck::AutoRenewDue`] once when the 75% threshold is first
    /// crossed while the renewal timer is armed.  After the session layer renews
    /// the lease, it must call [`reset_renewal_window`] to re-arm.
    pub fn poll(&mut self) -> TtlCheck {
        if self.ttl_ms == 0 {
            return TtlCheck::Indefinite;
        }
        // TTL is paused while suspended.
        if self.suspended_at_ms.is_some() {
            return TtlCheck::Ok;
        }
        let now_ms = self.clock.now_millis();
        let remaining = self.remaining_ms_at(now_ms);
        if remaining == 0 {
            return TtlCheck::Expired;
        }
        // Auto-renewal: fire when 75% of TTL has elapsed, once per window.
        // Use integer math (ttl_ms * 3 / 4) to avoid floating-point rounding
        // and edge cases with very small TTLs.  The constant AUTO_RENEW_THRESHOLD
        // documents the spec value (0.75) but the enforcement uses integer
        // arithmetic for determinism.
        if self.auto_renewal_arm == AutoRenewalArm::Armed && !self.renewal_fired {
            let elapsed = self.ttl_ms.saturating_sub(remaining);
            let threshold = self.ttl_ms.saturating_mul(3) / 4;
            if elapsed >= threshold {
                self.renewal_fired = true;
                return TtlCheck::AutoRenewDue;
            }
        }
        TtlCheck::Ok
    }

    // ── Transitions ───────────────────────────────────────────────────────────

    /// Called when the lease enters SUSPENDED state (safe mode entry).
    ///
    /// Freezes the TTL clock and disarms the auto-renewal timer (if it was
    /// Armed).  The disarm reason is recorded as `SafeMode` so that `on_resume`
    /// can correctly re-arm only safe-mode-caused disarms.
    ///
    /// This method is idempotent: if the lease is already suspended, it
    /// returns immediately without overwriting the original suspension timestamp
    /// or TTL snapshot.
    ///
    /// Spec §Auto-Renewal Policy: "timer is also paused and resumes with
    /// the TTL clock on safe mode exit".
    pub fn on_suspend(&mut self) {
        // Idempotent: do not overwrite the original suspension timestamp.
        if self.suspended_at_ms.is_some() {
            return;
        }
        let now_ms = self.clock.now_millis();
        let remaining = self.remaining_ms_at(now_ms);
        self.suspended_at_ms = Some(now_ms);
        self.ttl_remaining_at_suspend_ms = Some(remaining);
        // Disarm auto-renewal timer and record the reason as SafeMode, so
        // that on_resume() knows it can safely re-arm when safe mode exits.
        if self.auto_renewal_arm == AutoRenewalArm::Armed {
            self.auto_renewal_arm = AutoRenewalArm::Disarmed;
            self.disarm_reason = Some(DisarmReason::SafeMode);
        }
    }

    /// Called when the lease exits SUSPENDED state (safe mode exit).
    ///
    /// Resumes the TTL clock.  Re-arms the auto-renewal timer **only** if the
    /// timer was disarmed specifically because of safe mode entry (`DisarmReason::SafeMode`).
    ///
    /// A timer disarmed for `BudgetWarning` or `SessionDisconnecting` is NOT
    /// automatically re-armed here — the session layer must call `rearm_renewal`
    /// when the condition clears.
    pub fn on_resume(&mut self) {
        let now_ms = self.clock.now_millis();
        if let Some(susp_at) = self.suspended_at_ms {
            self.total_suspension_ms += now_ms.saturating_sub(susp_at);
        }
        self.suspended_at_ms = None;
        self.ttl_remaining_at_suspend_ms = None;
        // Re-arm ONLY if the disarm was caused by safe-mode entry.
        // Budget-warning and session-disconnecting disarms are NOT reversed here.
        if self.renewal_policy == RenewalPolicy::AutoRenew
            && self.auto_renewal_arm == AutoRenewalArm::Disarmed
            && self.disarm_reason == Some(DisarmReason::SafeMode)
        {
            self.auto_renewal_arm = AutoRenewalArm::Armed;
            self.disarm_reason = None;
        }
    }

    /// Disarm the auto-renewal timer (budget warning, session disconnecting, etc.).
    ///
    /// The `reason` is stored in state and used by `on_resume` to determine
    /// whether the timer should be re-armed automatically on safe-mode exit.
    ///
    /// Has no effect for `Manual` or `OneShot` policies.
    pub fn disarm_renewal(&mut self, reason: DisarmReason) {
        if self.auto_renewal_arm == AutoRenewalArm::Armed {
            self.auto_renewal_arm = AutoRenewalArm::Disarmed;
            self.disarm_reason = Some(reason);
        }
    }

    /// Re-arm the auto-renewal timer.
    ///
    /// Called when a budget warning is cleared before TTL expires.
    /// Has no effect for `Manual` or `OneShot` policies.
    pub fn rearm_renewal(&mut self) {
        if self.renewal_policy == RenewalPolicy::AutoRenew
            && self.auto_renewal_arm == AutoRenewalArm::Disarmed
        {
            self.auto_renewal_arm = AutoRenewalArm::Armed;
            self.disarm_reason = None;
        }
    }

    /// Reset the renewal window after a successful renewal.
    ///
    /// Called by the session layer after it has renewed the lease.  Adjusts
    /// `granted_at_ms` to the current time so the next 75% threshold is
    /// computed against the fresh TTL window.
    pub fn reset_renewal_window(&mut self, new_ttl_ms: u64) {
        let now_ms = self.clock.now_millis();
        self.granted_at_ms = now_ms;
        self.ttl_ms = new_ttl_ms;
        self.total_suspension_ms = 0;
        self.suspended_at_ms = None;
        self.ttl_remaining_at_suspend_ms = None;
        self.renewal_fired = false;
        // Keep auto_renewal_arm as-is (may have been disarmed for budget).
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// The renewal policy for this lease.
    pub fn renewal_policy(&self) -> RenewalPolicy {
        self.renewal_policy
    }

    /// Whether the auto-renewal timer is currently armed.
    pub fn auto_renewal_arm(&self) -> AutoRenewalArm {
        self.auto_renewal_arm
    }

    /// Whether the TTL clock is currently paused (lease is suspended).
    pub fn is_suspended(&self) -> bool {
        self.suspended_at_ms.is_some()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    fn make_ttl(ttl_ms: u64, policy: RenewalPolicy, start_ms: u64) -> TtlState<TestClock> {
        let clock = TestClock::new(start_ms);
        TtlState::new_activated(ttl_ms, policy, clock)
    }

    // ── TTL remaining ─────────────────────────────────────────────────────────

    #[test]
    fn ttl_remaining_full_at_activation() {
        let clock = TestClock::new(0);
        let ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock);
        assert_eq!(ttl.remaining_ms(), Some(60_000));
    }

    #[test]
    fn ttl_remaining_decreases_with_time() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock.clone());
        clock.advance(10_000);
        // poll to observe any events
        let _ = ttl.poll();
        assert_eq!(ttl.remaining_ms(), Some(50_000));
    }

    #[test]
    fn ttl_indefinite_returns_none() {
        let ttl = make_ttl(0, RenewalPolicy::Manual, 0);
        assert_eq!(ttl.remaining_ms(), None);
    }

    // ── Suspension pauses TTL ─────────────────────────────────────────────────

    /// Spec §TTL Accounting Precision: lease with ttl_ms=60_000 suspended for
    /// 10_000ms → effective expiry extended by 10_000ms.
    #[test]
    fn ttl_paused_during_suspension_within_100ms_tolerance() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock.clone());

        // Activate for 10s before suspending
        clock.advance(10_000);
        ttl.on_suspend();

        // Suspended for 10s — TTL clock frozen
        clock.advance(10_000);
        let frozen = ttl.remaining_ms();
        // Should be ~50_000ms (60_000 - 10_000 elapsed), regardless of 10s in suspension
        assert_eq!(frozen, Some(50_000));

        // Resume
        ttl.on_resume();
        let remaining = ttl.remaining_ms().unwrap();
        // After resume, still ~50_000ms (suspension not counted)
        assert!(
            remaining >= 49_900 && remaining <= 50_100,
            "expected ≈50_000ms after resume, got {remaining}"
        );
    }

    /// ONE_SHOT lease: TTL clock paused during suspension, full remaining TTL available on resume.
    #[test]
    fn one_shot_ttl_paused_during_suspension() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(30_000, RenewalPolicy::OneShot, clock.clone());

        clock.advance(5_000);
        ttl.on_suspend();
        clock.advance(10_000); // should not count
        ttl.on_resume();

        let remaining = ttl.remaining_ms().unwrap();
        // Should be ≈25_000ms (30_000 - 5_000 before suspension)
        assert!(
            remaining >= 24_900 && remaining <= 25_100,
            "ONE_SHOT: expected ≈25_000ms, got {remaining}"
        );
    }

    /// Multiple suspension cycles: total suspension accumulates correctly.
    #[test]
    fn multiple_suspension_cycles_accumulate() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock.clone());

        // First suspend/resume cycle: 5s active, 5s suspended
        clock.advance(5_000);
        ttl.on_suspend();
        clock.advance(5_000);
        ttl.on_resume();

        // Second suspend/resume cycle: 5s active, 3s suspended
        clock.advance(5_000);
        ttl.on_suspend();
        clock.advance(3_000);
        ttl.on_resume();

        // Effective elapsed = 5s + 5s = 10s (suspensions not counted)
        let remaining = ttl.remaining_ms().unwrap();
        assert!(
            remaining >= 49_900 && remaining <= 50_100,
            "expected ≈50_000ms, got {remaining}"
        );
    }

    // ── Adjusted expiry ───────────────────────────────────────────────────────

    /// Spec formula: effective_expiry = granted_at + ttl + total_suspension.
    #[test]
    fn adjusted_expires_at_ms_accounts_for_suspension() {
        let clock = TestClock::new(1_000); // granted_at = 1_000ms
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock.clone());

        ttl.on_suspend();
        clock.advance(10_000);
        ttl.on_resume();

        let expires_at = ttl.adjusted_expires_at_ms().unwrap();
        // = granted_at(1_000) + ttl(60_000) + suspension(10_000) = 71_000
        assert_eq!(expires_at, 71_000);
    }

    // ── Auto-renewal ──────────────────────────────────────────────────────────

    /// AUTO_RENEW policy: timer armed at activation.
    #[test]
    fn auto_renew_arm_state_at_activation() {
        let ttl = make_ttl(60_000, RenewalPolicy::AutoRenew, 0);
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::Armed);
    }

    /// MANUAL policy: timer not applicable.
    #[test]
    fn manual_renewal_arm_not_applicable() {
        let ttl = make_ttl(60_000, RenewalPolicy::Manual, 0);
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::NotApplicable);
    }

    /// ONE_SHOT policy: timer not applicable.
    #[test]
    fn one_shot_renewal_arm_not_applicable() {
        let ttl = make_ttl(30_000, RenewalPolicy::OneShot, 0);
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::NotApplicable);
    }

    /// AUTO_RENEW: poll returns AutoRenewDue at 75% TTL elapsed.
    #[test]
    fn auto_renew_fires_at_75_percent_ttl() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::AutoRenew, clock.clone());

        // Just before 75% threshold: 74% = 44_400ms
        clock.advance(44_399);
        assert_eq!(ttl.poll(), TtlCheck::Ok);

        // At 75% threshold: 45_000ms
        clock.advance(601); // total = 45_000ms
        assert_eq!(ttl.poll(), TtlCheck::AutoRenewDue);
    }

    /// AUTO_RENEW: AutoRenewDue fires only once per TTL window.
    #[test]
    fn auto_renew_fires_only_once_per_window() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::AutoRenew, clock.clone());

        clock.advance(46_000); // past 75%
        assert_eq!(ttl.poll(), TtlCheck::AutoRenewDue);
        // Second poll: should not fire again
        assert_eq!(ttl.poll(), TtlCheck::Ok);
        clock.advance(5_000);
        assert_eq!(ttl.poll(), TtlCheck::Ok);
    }

    /// AUTO_RENEW: after reset_renewal_window, fires again at next 75% threshold.
    #[test]
    fn auto_renew_fires_again_after_reset() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::AutoRenew, clock.clone());

        clock.advance(46_000); // 75%+ elapsed
        assert_eq!(ttl.poll(), TtlCheck::AutoRenewDue);

        // Session layer renews: reset window with fresh 60_000ms TTL
        ttl.reset_renewal_window(60_000);
        // Should not fire immediately (0ms elapsed after reset)
        assert_eq!(ttl.poll(), TtlCheck::Ok);

        // Advance to 75% of new window
        clock.advance(46_000);
        assert_eq!(ttl.poll(), TtlCheck::AutoRenewDue);
    }

    /// AUTO_RENEW disabled during budget warning: disarm prevents renewal.
    #[test]
    fn auto_renew_disabled_during_budget_warning() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::AutoRenew, clock.clone());

        // Disarm due to budget warning
        ttl.disarm_renewal(DisarmReason::BudgetWarning);
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::Disarmed);

        // Advance past 75% threshold — no renewal should fire
        clock.advance(46_000);
        assert_eq!(ttl.poll(), TtlCheck::Ok);
    }

    /// AUTO_RENEW: timer re-arms when budget warning clears.
    #[test]
    fn auto_renew_rearms_when_budget_warning_cleared() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::AutoRenew, clock.clone());

        ttl.disarm_renewal(DisarmReason::BudgetWarning);
        ttl.rearm_renewal();
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::Armed);

        // Should fire at 75% now
        clock.advance(46_000);
        assert_eq!(ttl.poll(), TtlCheck::AutoRenewDue);
    }

    /// AUTO_RENEW: timer paused on safe-mode entry, resumes on exit.
    #[test]
    fn auto_renew_paused_during_safe_mode_and_resumes() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::AutoRenew, clock.clone());

        // Enter safe mode before threshold
        clock.advance(40_000);
        ttl.on_suspend();
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::Disarmed);

        // Time passes in suspension — should not fire
        clock.advance(20_000);
        assert_eq!(ttl.poll(), TtlCheck::Ok);

        // Exit safe mode — timer re-arms
        ttl.on_resume();
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::Armed);

        // Now at 40s effective elapsed; need 5s more to hit 75% = 45s
        clock.advance(5_001);
        assert_eq!(ttl.poll(), TtlCheck::AutoRenewDue);
    }

    // ── Expiry ────────────────────────────────────────────────────────────────

    /// Poll returns Expired when TTL is fully consumed.
    #[test]
    fn poll_returns_expired_when_ttl_elapsed() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(10_000, RenewalPolicy::Manual, clock.clone());
        clock.advance(10_001);
        assert_eq!(ttl.poll(), TtlCheck::Expired);
    }

    /// ONE_SHOT: poll returns Expired at TTL, no auto-renewal.
    #[test]
    fn one_shot_expires_at_ttl_no_renewal() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(1_000, RenewalPolicy::OneShot, clock.clone());
        clock.advance(1_001);
        assert_eq!(ttl.poll(), TtlCheck::Expired);
    }

    /// Indefinite lease: poll always returns Indefinite.
    #[test]
    fn indefinite_lease_poll_returns_indefinite() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(0, RenewalPolicy::Manual, clock.clone());
        clock.advance(999_999);
        assert_eq!(ttl.poll(), TtlCheck::Indefinite);
    }

    // ── Suspension tracking ───────────────────────────────────────────────────

    #[test]
    fn is_suspended_false_when_active() {
        let ttl = make_ttl(60_000, RenewalPolicy::Manual, 0);
        assert!(!ttl.is_suspended());
    }

    #[test]
    fn is_suspended_true_after_on_suspend() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock.clone());
        ttl.on_suspend();
        assert!(ttl.is_suspended());
    }

    #[test]
    fn is_suspended_false_after_on_resume() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock.clone());
        ttl.on_suspend();
        ttl.on_resume();
        assert!(!ttl.is_suspended());
    }

    /// total_suspension_ms reflects ongoing suspension.
    #[test]
    fn total_suspension_ms_includes_ongoing() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock.clone());
        clock.advance(5_000);
        ttl.on_suspend();
        clock.advance(3_000);
        // total_suspension_ms should be ≈3_000 (ongoing)
        let total = ttl.total_suspension_ms();
        assert!(
            total >= 2_900 && total <= 3_100,
            "expected ≈3_000ms, got {total}"
        );
    }

    // ── Idempotent on_suspend ─────────────────────────────────────────────────

    /// on_suspend is idempotent: calling it twice preserves the original timestamp.
    #[test]
    fn on_suspend_idempotent_preserves_original_timestamp() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::Manual, clock.clone());

        clock.advance(5_000);
        ttl.on_suspend();
        // Capture state after first suspend
        let remaining_after_first = ttl.remaining_ms().unwrap();

        // Advance time while "suspended" and call on_suspend again
        clock.advance(3_000);
        ttl.on_suspend(); // should be a no-op

        // Remaining TTL should still be the value from the first suspend
        let remaining_after_second = ttl.remaining_ms().unwrap();
        assert_eq!(
            remaining_after_first, remaining_after_second,
            "on_suspend must be idempotent: second call should not change frozen TTL"
        );

        // After resume, TTL accounts for total suspension (8s), not just 3s
        ttl.on_resume();
        let remaining = ttl.remaining_ms().unwrap();
        // effective elapsed = 5s (before first suspend); suspension = 8s counted
        // remaining = 60_000 - 5_000 = 55_000ms
        assert!(
            remaining >= 54_900 && remaining <= 55_100,
            "expected ≈55_000ms after resume, got {remaining}"
        );
    }

    // ── Budget-warning disarm not re-armed on safe-mode exit ──────────────────

    /// Spec §Auto-Renewal Policy: a timer disarmed for budget warning MUST NOT
    /// be re-armed on safe-mode exit.  Only SafeMode-caused disarms resume automatically.
    #[test]
    fn budget_warning_disarm_not_rearmed_on_safe_mode_exit() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::AutoRenew, clock.clone());

        // Disarm for budget warning BEFORE entering safe mode
        ttl.disarm_renewal(DisarmReason::BudgetWarning);
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::Disarmed);

        // Enter safe mode (timer already disarmed — on_suspend should not overwrite reason)
        clock.advance(10_000);
        ttl.on_suspend();
        clock.advance(5_000);

        // Exit safe mode — timer must NOT be re-armed (disarm was for budget, not safe mode)
        ttl.on_resume();
        assert_eq!(
            ttl.auto_renewal_arm(),
            AutoRenewalArm::Disarmed,
            "budget-warning disarm must survive safe-mode exit"
        );

        // No auto-renewal should fire past 75%
        clock.advance(40_000); // 50s elapsed effective
        assert_eq!(
            ttl.poll(),
            TtlCheck::Ok,
            "budget-warning disarm should still prevent auto-renewal after safe-mode exit"
        );
    }

    /// Spec §Auto-Renewal Policy: a timer disarmed for safe-mode MUST be re-armed
    /// on safe-mode exit (normal happy-path case).
    #[test]
    fn safe_mode_disarm_is_rearmed_on_safe_mode_exit() {
        let clock = TestClock::new(0);
        let mut ttl = TtlState::new_activated(60_000, RenewalPolicy::AutoRenew, clock.clone());

        // Timer is armed; enter safe mode (which should disarm it with SafeMode reason)
        clock.advance(30_000);
        ttl.on_suspend();
        assert_eq!(ttl.auto_renewal_arm(), AutoRenewalArm::Disarmed);

        clock.advance(5_000);
        // Exit safe mode — timer MUST be re-armed (disarm was SafeMode)
        ttl.on_resume();
        assert_eq!(
            ttl.auto_renewal_arm(),
            AutoRenewalArm::Armed,
            "safe-mode disarm must be re-armed on safe-mode exit"
        );

        // Auto-renewal fires at 75% of effective elapsed (30s already past)
        clock.advance(15_001); // total effective = 45_001ms ≥ 75%
        assert_eq!(ttl.poll(), TtlCheck::AutoRenewDue);
    }
}
