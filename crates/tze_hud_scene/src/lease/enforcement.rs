//! Three-tier budget enforcement ladder.
//!
//! Implements the temporal escalation state machine from
//! `lease-governance/spec.md §Requirement: Three-Tier Budget Enforcement Ladder`
//! and the post-revocation resource cleanup requirements (spec lines 253-260).
//!
//! ## Tier ladder
//!
//! ```text
//! Normal ─(≥80%)──► Warning ─(5s unresolved)──► Throttle ─(30s sustained)──► Revocation
//!    ▲                  │                              │
//!    └──(drops <80%)────┘              ◄───────────────┘ (drops <80%)
//! ```
//!
//! Additionally, **critical bypass** triggers skip the ladder and go directly
//! to Revocation:
//! - `CriticalTextureOomAttempt`
//! - `RepeatedInvariantViolations` (> 10 in session lifetime)
//! - Protocol violations indicating malicious intent
//!
//! ## Clock injection
//!
//! All time comparisons are routed through a `C: Clock` instance so that
//! tests can use `TestClock` to deterministically advance time.
//!
//! ## Effective rate-Hz calculation
//!
//! When in `Throttle` tier, the effective `update_rate_hz` for the lease is
//! 50% of its nominal budget value (spec §Throttle after 5 seconds).

use crate::clock::Clock;
use super::BudgetTier;

// ─── Enforcement time constants ───────────────────────────────────────────────

/// Warning state duration before escalating to Throttle (5 seconds in ms).
const WARNING_GRACE_MS: u64 = 5_000;

/// Throttle state duration before escalating to Revocation (30 seconds in ms).
const THROTTLE_GRACE_MS: u64 = 30_000;

/// Maximum invariant violations in a session lifetime before critical revocation.
const MAX_INVARIANT_VIOLATIONS: u32 = 10;

// ─── Critical bypass triggers ─────────────────────────────────────────────────

/// Events that bypass the three-tier ladder and trigger immediate revocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CriticalBypassTrigger {
    /// An agent attempted to allocate texture memory past the absolute ceiling.
    CriticalTextureOomAttempt,
    /// The session has accumulated > 10 invariant violations.
    RepeatedInvariantViolations { count: u32 },
    /// Protocol violation indicating malicious or buggy agent behaviour.
    ProtocolViolation { detail: String },
}

// ─── Enforcement action ───────────────────────────────────────────────────────

/// The outcome of a ladder `tick()` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnforcementAction {
    /// No state change — continue normal operation.
    None,
    /// Budget usage entered or remains in Warning; send `BudgetWarning` event.
    Warn,
    /// Budget usage escalated to Throttle; effective `update_rate_hz` is halved.
    Throttle,
    /// Budget usage escalated to (or stayed in) Revocation.
    Revoke,
}

// ─── EnforcementLadder ───────────────────────────────────────────────────────

/// Per-lease three-tier budget enforcement ladder.
///
/// The caller must:
/// 1. Create one `EnforcementLadder` per lease when the lease is granted.
/// 2. Call `tick(usage_fraction)` on every mutation intake (or polling
///    cycle) to advance the ladder based on the current budget fraction;
///    elapsed time is derived from the injected `Clock`.
/// 3. Act on the returned `EnforcementAction`.
/// 4. Call `report_critical_bypass()` when a critical violation is detected.
///
/// All timing is expressed in **milliseconds** using `C::now_millis()`.
pub struct EnforcementLadder<C: Clock> {
    clock: C,
    tier: BudgetTier,

    /// Timestamp when budget first entered Warning tier (ms).
    warning_started_ms: Option<u64>,
    /// Timestamp when budget entered Throttle tier (ms).
    throttle_started_ms: Option<u64>,

    /// Cumulative invariant violation counter (session lifetime).
    invariant_violation_count: u32,
}

impl<C: Clock> EnforcementLadder<C> {
    /// Create a new ladder in `Normal` state, backed by the given clock.
    pub fn new(clock: C) -> Self {
        Self {
            clock,
            tier: BudgetTier::Normal,
            warning_started_ms: None,
            throttle_started_ms: None,
            invariant_violation_count: 0,
        }
    }

    /// Current enforcement tier.
    pub fn tier(&self) -> BudgetTier {
        self.tier
    }

    /// Effective `update_rate_hz` multiplier.
    ///
    /// Returns `1.0` in Normal/Warning/Revocation tiers (Revocation stops all
    /// mutations), and `0.5` in Throttle (50% reduction per spec).
    pub fn effective_rate_multiplier(&self) -> f32 {
        match self.tier {
            BudgetTier::Throttle => 0.5,
            _ => 1.0,
        }
    }

    /// Advance the ladder based on `usage_fraction` (0.0–1.0) and current time.
    ///
    /// * `usage_fraction = max(dim_pct for all dimensions)` — the *highest*
    ///   fraction across all budget dimensions drives the tier.
    ///
    /// Returns the `EnforcementAction` the caller should take.
    ///
    /// # Hard limit and relationship to the lease state machine
    ///
    /// This ladder models only the *soft*, time-based escalation (Normal →
    /// Warning → Throttle → Revocation).  It deliberately does **not** duplicate
    /// the *hard* budget cutoff implemented by the lease state machine
    /// (`lease/state_machine.rs`), which revokes immediately and returns
    /// `BudgetHardLimitExceeded` when `usage_fraction >= 1.0`.
    ///
    /// Callers must invoke [`check_budget_hard`][super::budget::check_budget_hard]
    /// **before** calling `tick`; `tick` only manages the time-based tier ladder.
    /// Consequently, passing `usage_fraction >= 1.0` to `tick` does *not* by
    /// itself set the tier to `Revocation` — the ladder only reaches `Revocation`
    /// via an explicit critical bypass (`report_critical_bypass()`) or via
    /// sustained `Throttle` for the configured grace period.  This difference
    /// from the `state_machine.rs` behavior is intentional to avoid double
    /// enforcement of the hard limit.
    pub fn tick(&mut self, usage_fraction: f64) -> EnforcementAction {
        let now_ms = self.clock.now_millis();

        match self.tier {
            BudgetTier::Revocation => {
                // Terminal tier — always report Revoke until the lease is torn down.
                return EnforcementAction::Revoke;
            }
            BudgetTier::Normal => {
                if usage_fraction >= 0.80 {
                    self.tier = BudgetTier::Warning;
                    self.warning_started_ms = Some(now_ms);
                    return EnforcementAction::Warn;
                }
                // Below 80% — stay Normal.
            }
            BudgetTier::Warning => {
                if usage_fraction < 0.80 {
                    // Usage dropped below threshold — back to Normal.
                    self.tier = BudgetTier::Normal;
                    self.warning_started_ms = None;
                    return EnforcementAction::None;
                }
                // Still at or above 80% — check grace period.
                if let Some(warn_start) = self.warning_started_ms {
                    if now_ms.saturating_sub(warn_start) >= WARNING_GRACE_MS {
                        // Grace period elapsed → Throttle.
                        self.tier = BudgetTier::Throttle;
                        self.warning_started_ms = None; // clear Warning state on exit
                        self.throttle_started_ms = Some(now_ms);
                        return EnforcementAction::Throttle;
                    }
                }
                return EnforcementAction::Warn;
            }
            BudgetTier::Throttle => {
                if usage_fraction < 0.80 {
                    // Throttle is sticky — once the agent has been throttled, the runtime
                    // must explicitly reset the ladder (e.g. via a new lease grant).
                    // This matches LeaseImpl::update_budget_usage in state_machine.rs:
                    // "Throttle and Revocation are sticky (require explicit resolution)."
                    return EnforcementAction::Throttle;
                }
                // Throttle sustained — check if we have reached 30s.
                if let Some(throttle_start) = self.throttle_started_ms {
                    if now_ms.saturating_sub(throttle_start) >= THROTTLE_GRACE_MS {
                        self.tier = BudgetTier::Revocation;
                        self.warning_started_ms = None;
                        self.throttle_started_ms = None;
                        return EnforcementAction::Revoke;
                    }
                }
                return EnforcementAction::Throttle;
            }
        }

        EnforcementAction::None
    }

    /// Trigger immediate revocation, bypassing the three-tier ladder.
    ///
    /// Must be called when:
    /// - `CriticalTextureOomAttempt` is detected.
    /// - `RepeatedInvariantViolations` threshold is crossed (> 10 in lifetime).
    /// - A malicious/protocol-violating behaviour is detected.
    ///
    /// Returns `EnforcementAction::Revoke` always.
    pub fn report_critical_bypass(&mut self, _trigger: CriticalBypassTrigger) -> EnforcementAction {
        self.tier = BudgetTier::Revocation;
        self.warning_started_ms = None;
        self.throttle_started_ms = None;
        EnforcementAction::Revoke
    }

    /// Record an invariant violation.  Returns `true` if the critical threshold
    /// (> 10) has been reached and immediate revocation is required.
    pub fn record_invariant_violation(&mut self) -> bool {
        self.invariant_violation_count += 1;
        if self.invariant_violation_count > MAX_INVARIANT_VIOLATIONS {
            self.report_critical_bypass(CriticalBypassTrigger::RepeatedInvariantViolations {
                count: self.invariant_violation_count,
            });
            true
        } else {
            false
        }
    }

    /// Number of invariant violations accumulated in this session.
    pub fn invariant_violation_count(&self) -> u32 {
        self.invariant_violation_count
    }

    /// Milliseconds elapsed in Warning tier (0 if not currently in Warning).
    pub fn warning_duration_ms(&self) -> u64 {
        if self.tier != BudgetTier::Warning {
            return 0;
        }
        self.warning_started_ms
            .map(|s| self.clock.now_millis().saturating_sub(s))
            .unwrap_or(0)
    }

    /// Milliseconds elapsed in Throttle tier (0 if not currently throttled).
    pub fn throttle_duration_ms(&self) -> u64 {
        if self.tier != BudgetTier::Throttle {
            return 0;
        }
        self.throttle_started_ms
            .map(|s| self.clock.now_millis().saturating_sub(s))
            .unwrap_or(0)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    fn make_ladder(start_ms: u64) -> (EnforcementLadder<TestClock>, TestClock) {
        let clock = TestClock::new(start_ms);
        let ladder = EnforcementLadder::new(clock.clone());
        (ladder, clock)
    }

    // ── 1. Normal tier ────────────────────────────────────────────────────

    /// WHEN usage is below 80% THEN tier stays Normal.
    #[test]
    fn test_normal_tier_below_80pct() {
        let (mut ladder, _) = make_ladder(0);
        let action = ladder.tick(0.79);
        assert_eq!(ladder.tier(), BudgetTier::Normal);
        assert_eq!(action, EnforcementAction::None);
    }

    /// WHEN usage at 0% THEN tier is Normal.
    #[test]
    fn test_normal_tier_at_zero() {
        let (mut ladder, _) = make_ladder(0);
        let action = ladder.tick(0.0);
        assert_eq!(ladder.tier(), BudgetTier::Normal);
        assert_eq!(action, EnforcementAction::None);
    }

    // ── 2. Warning tier ───────────────────────────────────────────────────

    /// WHEN usage reaches 80% THEN tier transitions to Warning and action is Warn.
    #[test]
    fn test_warning_triggered_at_80pct() {
        let (mut ladder, _) = make_ladder(0);
        let action = ladder.tick(0.80);
        assert_eq!(ladder.tier(), BudgetTier::Warning, "should enter Warning at 80%");
        assert_eq!(action, EnforcementAction::Warn);
    }

    /// WHEN warning resolves (usage drops below 80%) THEN tier returns to Normal.
    #[test]
    fn test_warning_resolved_returns_to_normal() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85); // enter Warning
        clock.advance(1_000); // 1s into warning
        let action = ladder.tick(0.70); // resolved
        assert_eq!(ladder.tier(), BudgetTier::Normal, "should return to Normal when resolved");
        assert_eq!(action, EnforcementAction::None);
    }

    /// WHEN in Warning and usage remains above 80% for < 5s THEN stays in Warning.
    #[test]
    fn test_warning_sustained_under_5s_stays_warning() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85); // enter Warning
        clock.advance(4_999); // 4.999s — still within grace
        let action = ladder.tick(0.85);
        assert_eq!(ladder.tier(), BudgetTier::Warning);
        assert_eq!(action, EnforcementAction::Warn);
    }

    // ── 3. Throttle tier ──────────────────────────────────────────────────

    /// WHEN warning unresolved for 5s THEN tier transitions to Throttle.
    /// Spec scenario: "Throttle after 5 seconds" (spec lines 192-194).
    #[test]
    fn test_throttle_after_5s_warning() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85); // enter Warning
        assert_eq!(ladder.tier(), BudgetTier::Warning);
        clock.advance(5_001); // > 5s
        let action = ladder.tick(0.85);
        assert_eq!(ladder.tier(), BudgetTier::Throttle, "should throttle after 5s");
        assert_eq!(action, EnforcementAction::Throttle);
    }

    /// WHEN in Throttle THEN effective_rate_multiplier is 0.5.
    #[test]
    fn test_throttle_halves_effective_rate() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85);
        clock.advance(5_001);
        ladder.tick(0.85); // Throttle now
        assert_eq!(ladder.tier(), BudgetTier::Throttle);
        assert!((ladder.effective_rate_multiplier() - 0.5).abs() < f32::EPSILON,
            "throttle should halve effective rate");
    }

    /// WHEN throttle is active and usage drops THEN Throttle remains sticky (not auto-reset).
    /// Throttle requires explicit runtime intervention to clear, matching LeaseImpl semantics.
    #[test]
    fn test_throttle_sticky_when_usage_drops() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85);
        clock.advance(5_001);
        ladder.tick(0.85); // Throttle
        let action = ladder.tick(0.50); // usage drops but Throttle stays
        assert_eq!(ladder.tier(), BudgetTier::Throttle,
            "Throttle is sticky — does not auto-reset when usage drops");
        assert_eq!(action, EnforcementAction::Throttle);
    }

    // ── 4. Revocation tier ────────────────────────────────────────────────

    /// WHEN throttle sustained for 30s THEN tier becomes Revocation.
    /// Spec scenario: "Revocation after 30 seconds throttle" (spec lines 196-198).
    #[test]
    fn test_revocation_after_30s_throttle() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85);           // Warning
        clock.advance(5_001);
        ladder.tick(0.85);           // Throttle
        clock.advance(30_001);       // 30s+ in Throttle
        let action = ladder.tick(0.85);
        assert_eq!(ladder.tier(), BudgetTier::Revocation, "should revoke after 30s throttle");
        assert_eq!(action, EnforcementAction::Revoke);
    }

    /// WHEN in Revocation THEN all ticks return Revoke (terminal).
    #[test]
    fn test_revocation_is_terminal() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85);
        clock.advance(5_001);
        ladder.tick(0.85); // Throttle
        clock.advance(30_001);
        ladder.tick(0.85); // Revocation
        // Further ticks must still return Revoke.
        let action = ladder.tick(0.0); // even if usage drops
        assert_eq!(ladder.tier(), BudgetTier::Revocation);
        assert_eq!(action, EnforcementAction::Revoke);
    }

    // ── 5. Critical bypass ────────────────────────────────────────────────

    /// WHEN critical bypass is reported THEN tier immediately becomes Revocation.
    /// Spec scenario: "Critical bypass" (spec lines 200-202).
    #[test]
    fn test_critical_bypass_immediate_revocation() {
        let (mut ladder, _) = make_ladder(0);
        // Still at Normal
        assert_eq!(ladder.tier(), BudgetTier::Normal);
        let action = ladder.report_critical_bypass(CriticalBypassTrigger::CriticalTextureOomAttempt);
        assert_eq!(ladder.tier(), BudgetTier::Revocation, "critical bypass must immediately revoke");
        assert_eq!(action, EnforcementAction::Revoke);
    }

    /// WHEN in Warning and critical bypass is reported THEN immediately Revocation (skips Throttle).
    #[test]
    fn test_critical_bypass_from_warning_skips_throttle() {
        let (mut ladder, _) = make_ladder(0);
        ladder.tick(0.85); // Warning
        let action = ladder.report_critical_bypass(CriticalBypassTrigger::CriticalTextureOomAttempt);
        assert_eq!(ladder.tier(), BudgetTier::Revocation);
        assert_eq!(action, EnforcementAction::Revoke);
    }

    /// WHEN invariant violation count exceeds 10 THEN critical bypass triggers.
    #[test]
    fn test_invariant_violations_above_10_triggers_revocation() {
        let (mut ladder, _) = make_ladder(0);
        for i in 0..=10u32 {
            let is_critical = ladder.record_invariant_violation();
            if i < 10 {
                assert!(!is_critical, "violation {} should not trigger (need >10)", i + 1);
            } else {
                assert!(is_critical, "11th violation should trigger critical bypass");
            }
        }
        assert_eq!(ladder.tier(), BudgetTier::Revocation);
    }

    /// WHEN 10 violations accumulated THEN not yet critical (threshold is > 10).
    #[test]
    fn test_invariant_violations_exactly_10_not_critical() {
        let (mut ladder, _) = make_ladder(0);
        for _ in 0..10 {
            let is_critical = ladder.record_invariant_violation();
            assert!(!is_critical);
        }
        assert_ne!(ladder.tier(), BudgetTier::Revocation,
            "exactly 10 violations must NOT trigger revocation (threshold is >10)");
    }

    // ── 6. Effective rate multiplier ──────────────────────────────────────

    /// WHEN in Normal tier THEN effective rate multiplier is 1.0.
    #[test]
    fn test_effective_rate_multiplier_normal() {
        let (ladder, _) = make_ladder(0);
        assert_eq!(ladder.effective_rate_multiplier(), 1.0);
    }

    /// WHEN in Warning tier THEN effective rate multiplier is 1.0 (mutations accepted).
    #[test]
    fn test_effective_rate_multiplier_warning() {
        let (mut ladder, _) = make_ladder(0);
        ladder.tick(0.85);
        assert_eq!(ladder.tier(), BudgetTier::Warning);
        assert_eq!(ladder.effective_rate_multiplier(), 1.0,
            "warning tier must not reduce rate");
    }

    // ── 7. Duration helpers ───────────────────────────────────────────────

    /// WHEN in Warning tier THEN warning_duration_ms returns elapsed time.
    #[test]
    fn test_warning_duration_increases_while_in_warning() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85);
        clock.advance(2_500);
        let dur = ladder.warning_duration_ms();
        assert!(dur >= 2_400 && dur <= 2_600,
            "expected ≈2500ms warning duration, got {dur}");
    }

    /// WHEN not in Warning tier THEN warning_duration_ms is 0.
    #[test]
    fn test_warning_duration_zero_when_not_warning() {
        let (ladder, _) = make_ladder(0);
        assert_eq!(ladder.warning_duration_ms(), 0);
    }

    /// WHEN in Throttle tier THEN throttle_duration_ms returns elapsed time.
    #[test]
    fn test_throttle_duration_increases_while_throttled() {
        let (mut ladder, clock) = make_ladder(0);
        ladder.tick(0.85);
        clock.advance(5_001);
        ladder.tick(0.85); // enter Throttle
        clock.advance(10_000);
        let dur = ladder.throttle_duration_ms();
        assert!(dur >= 9_900 && dur <= 10_100,
            "expected ≈10000ms throttle duration, got {dur}");
    }

    // ── 8. Full ladder walkthrough ────────────────────────────────────────

    /// Walk through Normal → Warning → Throttle → Revocation using TestClock.
    #[test]
    fn test_full_ladder_progression() {
        let (mut ladder, clock) = make_ladder(0);

        // Phase 1: Normal
        assert_eq!(ladder.tick(0.50), EnforcementAction::None);
        assert_eq!(ladder.tier(), BudgetTier::Normal);

        // Phase 2: Enter Warning at 80%
        assert_eq!(ladder.tick(0.80), EnforcementAction::Warn);
        assert_eq!(ladder.tier(), BudgetTier::Warning);

        // Phase 3: Warning sustained for exactly 5 seconds — still Warning
        clock.advance(4_999);
        assert_eq!(ladder.tick(0.85), EnforcementAction::Warn);
        assert_eq!(ladder.tier(), BudgetTier::Warning);

        // Phase 4: Past 5s — Throttle
        clock.advance(2); // total 5_001ms
        assert_eq!(ladder.tick(0.85), EnforcementAction::Throttle);
        assert_eq!(ladder.tier(), BudgetTier::Throttle);

        // Phase 5: Throttle sustained for exactly 30 seconds — still Throttle
        clock.advance(29_999);
        assert_eq!(ladder.tick(0.85), EnforcementAction::Throttle);
        assert_eq!(ladder.tier(), BudgetTier::Throttle);

        // Phase 6: Past 30s — Revocation
        clock.advance(2); // total 30_001ms in Throttle
        assert_eq!(ladder.tick(0.85), EnforcementAction::Revoke);
        assert_eq!(ladder.tier(), BudgetTier::Revocation);

        // Phase 7: Revocation is terminal
        clock.advance(60_000);
        assert_eq!(ladder.tick(0.0), EnforcementAction::Revoke); // even at 0% usage
    }
}
