//! # Level 4 Attention Evaluation
//!
//! Implements the pure attention-gate function for Level 4 of the arbitration
//! stack. This module encapsulates all quiet-hours and attention-budget logic
//! in a single, testable, side-effect-free function.
//!
//! ## Spec Reference
//!
//! - policy-arbitration/spec.md — Requirement: Level 4 Attention Management (lines 143-166)
//! - RFC 0010 §3.1, §7
//!
//! ## Purity Contract
//!
//! `evaluate_attention` is a pure function over `AttentionContext`. It produces
//! an `AttentionDecision` — a structured verdict — without touching counters or
//! any external state. The caller (frame pipeline) is responsible for:
//!
//! 1. Building an `AttentionContext` snapshot from live `AttentionBudget` counters.
//! 2. Calling `evaluate_attention`.
//! 3. Acting on the decision (Queue, Discard, Pass, etc.).
//! 4. If the decision is `Pass`, incrementing the live counters.
//!
//! ## Attention Decision Variants
//!
//! | Variant          | Description                                       |
//! |------------------|---------------------------------------------------|
//! | `Pass`           | Mutation is allowed through                       |
//! | `QueueQuietHours`| Deferred until quiet hours end (FIFO)             |
//! | `Discard`        | Dropped silently (LOW during quiet hours)         |
//! | `Coalesce`       | Budget exhausted — latest-wins within agent+zone  |
//!
//! ## Quiet Hours Pass-Through
//!
//! The `pass_through_class` threshold in `AttentionContext` controls which
//! classes pass through quiet hours. The comparison is on `InterruptionClass`
//! discriminant value:
//!
//! - If `interruption_class <= pass_through_class` → passes (same or more urgent).
//! - If `interruption_class > pass_through_class` → queued.
//!
//! Default `pass_through_class` is `High`, so `Critical` and `High` pass by
//! default; `Normal`, `Low`, and `Silent` are filtered (though `Critical` and
//! `Silent` are handled before this comparison).

use crate::types::{AttentionContext, InterruptionClass, QueueReason};

// ─── Decision type ────────────────────────────────────────────────────────────

/// The verdict produced by the attention gate.
///
/// The caller converts this into the appropriate `ArbitrationOutcome`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttentionDecision {
    /// Mutation passes the attention gate. The caller MUST increment budget counters.
    Pass,

    /// Mutation is queued until quiet hours end. Delivered FIFO on quiet hours exit.
    ///
    /// The `queue_reason` is `QuietHours { window_end_us }`.
    QueueQuietHours {
        /// When the quiet hours window ends, in monotonic microseconds. `None` if unknown.
        window_end_us: Option<u64>,
    },

    /// Mutation is discarded silently. No error to agent.
    ///
    /// Only applies to `LOW` interruptions during quiet hours (too stale to be useful).
    Discard,

    /// Budget exhausted. Mutation is coalesced (latest-wins within agent+zone key).
    ///
    /// The `queue_reason` is `AttentionBudgetExhausted { per_agent, per_zone }`.
    Coalesce {
        /// Per-agent budget was exhausted.
        per_agent: bool,
        /// Per-zone budget was exhausted.
        per_zone: bool,
        /// When the budget will next refill, in monotonic microseconds.
        budget_refill_us: Option<u64>,
    },
}

impl AttentionDecision {
    /// Returns `true` if this decision allows the mutation through.
    pub fn is_pass(&self) -> bool {
        matches!(self, AttentionDecision::Pass)
    }

    /// Returns the `QueueReason` for use in `ArbitrationOutcome::Queue`, if applicable.
    pub fn into_queue_reason(self) -> Option<QueueReason> {
        match self {
            AttentionDecision::QueueQuietHours { window_end_us } => {
                Some(QueueReason::QuietHours { window_end_us })
            }
            AttentionDecision::Coalesce {
                per_agent,
                per_zone,
                ..
            } => Some(QueueReason::AttentionBudgetExhausted {
                per_agent,
                per_zone,
            }),
            _ => None,
        }
    }
}

// ─── Pure evaluator ──────────────────────────────────────────────────────────

/// Evaluate the Level 4 Attention gate for a single mutation.
///
/// This is a **pure function** — it takes a read-only `AttentionContext`
/// snapshot and returns an `AttentionDecision`. No counters are modified.
///
/// ## Evaluation Order (spec §3.3, §3.4)
///
/// 1. `CRITICAL` → always `Pass` (bypasses quiet hours and budget).
/// 2. `SILENT` → always `Pass` (zero budget cost; never filtered).
/// 3. Quiet hours active?
///    - `LOW` → `Discard` (too stale by quiet hours exit).
///    - `NORMAL` → `QueueQuietHours` (FIFO delivery when quiet hours end).
///    - `HIGH` → check `pass_through_class`:
///      - passes if `interruption_class <= pass_through_class` (same or more urgent).
///      - queued otherwise.
/// 4. Attention budget check:
///    - If either per-agent or per-zone budget is exhausted → `Coalesce`.
/// 5. Otherwise → `Pass`.
///
/// ## Latency Contract
///
/// This function performs only integer comparisons and reads on a small
/// inline struct. It must complete in < 10µs under nominal load (spec §11.5).
/// There is no heap access, no locking, and no iteration.
///
/// # Arguments
///
/// - `ctx`: read-only attention context snapshot. All counters must be
///   pre-populated by the caller (frame pipeline) before this call.
///
/// # Returns
///
/// An `AttentionDecision` describing what the pipeline should do.
pub fn evaluate_attention(ctx: &AttentionContext) -> AttentionDecision {
    // ── Step 1: CRITICAL — bypasses everything ────────────────────────────
    if ctx.interruption_class == InterruptionClass::Critical {
        return AttentionDecision::Pass;
    }

    // ── Step 2: SILENT — always passes, zero budget cost ──────────────────
    if ctx.interruption_class == InterruptionClass::Silent {
        return AttentionDecision::Pass;
    }

    // ── Step 3: Quiet hours filter ────────────────────────────────────────
    if ctx.quiet_hours_active {
        match ctx.interruption_class {
            InterruptionClass::Critical | InterruptionClass::Silent => {
                // Already handled above — unreachable.
                unreachable!("Critical and Silent are handled before quiet hours check")
            }
            InterruptionClass::Low => {
                // LOW is discarded during quiet hours — too stale by the time
                // quiet hours end to be useful (spec lines 152-154).
                return AttentionDecision::Discard;
            }
            InterruptionClass::Normal => {
                // NORMAL is queued until quiet hours end, delivered FIFO (spec lines 148-150).
                return AttentionDecision::QueueQuietHours {
                    window_end_us: ctx.quiet_hours_end_us,
                };
            }
            InterruptionClass::High => {
                // HIGH passes quiet hours if it meets the pass_through_class threshold.
                // The threshold comparison: if interruption_class > pass_through_class,
                // the mutation is less urgent than the threshold → queue it.
                // Ordering: Critical(0) < High(1) < Normal(2) < Low(3) < Silent(4).
                if ctx.interruption_class > ctx.pass_through_class {
                    // HIGH is less urgent than the configured threshold → queue.
                    return AttentionDecision::QueueQuietHours {
                        window_end_us: ctx.quiet_hours_end_us,
                    };
                }
                // HIGH meets threshold → falls through to budget check.
            }
        }
    }

    // ── Step 4: Attention budget check ────────────────────────────────────
    // CRITICAL and SILENT are already returned above; only HIGH/NORMAL/LOW
    // reach here (and LOW during quiet hours is discarded above).
    let agent_exhausted = ctx.agent_budget_exhausted();
    let zone_exhausted = ctx.zone_budget_exhausted();

    if agent_exhausted || zone_exhausted {
        // Budget exhausted — mutations must be coalesced (latest-wins within
        // agent+zone key) until the budget refills (spec lines 164-166).
        return AttentionDecision::Coalesce {
            per_agent: agent_exhausted,
            per_zone: zone_exhausted,
            budget_refill_us: ctx.budget_refill_us,
        };
    }

    // ── Step 5: Pass ──────────────────────────────────────────────────────
    AttentionDecision::Pass
}

#[cfg(test)]
mod attention_eval_tests {
    use super::*;
    use crate::types::AttentionContext;

    fn base_ctx() -> AttentionContext {
        AttentionContext {
            quiet_hours_active: false,
            quiet_hours_end_us: None,
            per_agent_interruptions_last_60s: 0,
            per_agent_limit: 20,
            per_zone_interruptions_last_60s: 0,
            per_zone_limit: 10,
            pass_through_class: InterruptionClass::High,
            interruption_class: InterruptionClass::Normal,
            budget_refill_us: None,
        }
    }

    // ─── CRITICAL bypasses everything ────────────────────────────────────────

    /// WHEN a CRITICAL interruption arrives during quiet hours with an exhausted
    /// attention budget THEN the mutation passes through both (spec lines 160-162).
    #[test]
    fn test_critical_bypasses_quiet_hours_and_budget() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = true;
        ctx.quiet_hours_end_us = Some(7_200_000_000);
        ctx.interruption_class = InterruptionClass::Critical;
        ctx.per_agent_interruptions_last_60s = 100; // exhausted
        ctx.per_zone_interruptions_last_60s = 100; // exhausted

        assert_eq!(evaluate_attention(&ctx), AttentionDecision::Pass);
    }

    #[test]
    fn test_critical_passes_without_quiet_hours() {
        let mut ctx = base_ctx();
        ctx.interruption_class = InterruptionClass::Critical;
        assert_eq!(evaluate_attention(&ctx), AttentionDecision::Pass);
    }

    // ─── SILENT always passes ─────────────────────────────────────────────────

    #[test]
    fn test_silent_passes_during_quiet_hours() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = true;
        ctx.interruption_class = InterruptionClass::Silent;
        assert_eq!(evaluate_attention(&ctx), AttentionDecision::Pass);
    }

    #[test]
    fn test_silent_passes_with_exhausted_budget() {
        let mut ctx = base_ctx();
        ctx.interruption_class = InterruptionClass::Silent;
        ctx.per_agent_interruptions_last_60s = 100;
        ctx.per_zone_interruptions_last_60s = 100;
        assert_eq!(evaluate_attention(&ctx), AttentionDecision::Pass);
    }

    // ─── Quiet hours: NORMAL queued (FIFO) ───────────────────────────────────

    /// WHEN quiet hours are active and a mutation with NORMAL interruption class
    /// arrives THEN the mutation is queued until quiet hours end (spec lines 148-150).
    #[test]
    fn test_quiet_hours_queue_normal() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = true;
        ctx.quiet_hours_end_us = Some(7_200_000_000);
        ctx.interruption_class = InterruptionClass::Normal;

        let decision = evaluate_attention(&ctx);
        assert_eq!(
            decision,
            AttentionDecision::QueueQuietHours {
                window_end_us: Some(7_200_000_000)
            }
        );
    }

    #[test]
    fn test_quiet_hours_queue_normal_unknown_end() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = true;
        ctx.quiet_hours_end_us = None;
        ctx.interruption_class = InterruptionClass::Normal;

        let decision = evaluate_attention(&ctx);
        assert_eq!(
            decision,
            AttentionDecision::QueueQuietHours {
                window_end_us: None
            }
        );
    }

    // ─── Quiet hours: LOW discarded ───────────────────────────────────────────

    /// WHEN quiet hours are active and a mutation with LOW interruption class
    /// arrives THEN the mutation is discarded (spec lines 152-154).
    #[test]
    fn test_quiet_hours_discard_low() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = true;
        ctx.interruption_class = InterruptionClass::Low;

        assert_eq!(evaluate_attention(&ctx), AttentionDecision::Discard);
    }

    // ─── Quiet hours: HIGH with pass_through_class ───────────────────────────

    /// WHEN quiet hours are active and HIGH arrives with default pass_through_class=High
    /// THEN HIGH passes through (spec lines 156-158).
    #[test]
    fn test_quiet_hours_high_passes_with_default_threshold() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = true;
        ctx.interruption_class = InterruptionClass::High;
        ctx.pass_through_class = InterruptionClass::High; // default

        assert_eq!(evaluate_attention(&ctx), AttentionDecision::Pass);
    }

    /// WHEN pass_through_class is raised to CRITICAL and HIGH arrives
    /// THEN HIGH is queued (it does not meet the stricter Critical-only threshold).
    #[test]
    fn test_quiet_hours_high_queued_when_threshold_is_critical() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = true;
        ctx.quiet_hours_end_us = Some(7_200_000_000);
        ctx.interruption_class = InterruptionClass::High;
        ctx.pass_through_class = InterruptionClass::Critical; // only CRITICAL passes

        let decision = evaluate_attention(&ctx);
        assert_eq!(
            decision,
            AttentionDecision::QueueQuietHours {
                window_end_us: Some(7_200_000_000)
            },
            "HIGH must be queued when pass_through_class=Critical"
        );
    }

    // ─── Attention budget exhausted → coalesce ───────────────────────────────

    /// WHEN an agent exceeds 20 interruptions per minute THEN subsequent mutations
    /// are coalesced (latest-wins) until the budget refills (spec lines 164-166).
    #[test]
    fn test_agent_budget_exhausted_coalesces() {
        let mut ctx = base_ctx();
        ctx.per_agent_interruptions_last_60s = 20; // at limit
        ctx.per_agent_limit = 20;
        ctx.interruption_class = InterruptionClass::Normal;

        let decision = evaluate_attention(&ctx);
        assert!(
            matches!(
                &decision,
                AttentionDecision::Coalesce {
                    per_agent: true,
                    ..
                }
            ),
            "Agent budget exhausted must coalesce; got {:?}",
            decision
        );
    }

    #[test]
    fn test_zone_budget_exhausted_coalesces() {
        let mut ctx = base_ctx();
        ctx.per_zone_interruptions_last_60s = 10; // at limit
        ctx.per_zone_limit = 10;
        ctx.interruption_class = InterruptionClass::Normal;

        let decision = evaluate_attention(&ctx);
        assert!(
            matches!(
                &decision,
                AttentionDecision::Coalesce { per_zone: true, .. }
            ),
            "Zone budget exhausted must coalesce; got {:?}",
            decision
        );
    }

    #[test]
    fn test_both_budgets_exhausted_coalesces() {
        let mut ctx = base_ctx();
        ctx.per_agent_interruptions_last_60s = 20;
        ctx.per_agent_limit = 20;
        ctx.per_zone_interruptions_last_60s = 10;
        ctx.per_zone_limit = 10;
        ctx.interruption_class = InterruptionClass::Normal;

        let decision = evaluate_attention(&ctx);
        assert!(
            matches!(
                &decision,
                AttentionDecision::Coalesce {
                    per_agent: true,
                    per_zone: true,
                    ..
                }
            ),
            "Both budgets exhausted: per_agent and per_zone must both be true"
        );
    }

    #[test]
    fn test_coalesce_carries_refill_timestamp() {
        let mut ctx = base_ctx();
        ctx.per_agent_interruptions_last_60s = 20;
        ctx.per_agent_limit = 20;
        ctx.budget_refill_us = Some(9_000_000);
        ctx.interruption_class = InterruptionClass::Normal;

        if let AttentionDecision::Coalesce {
            budget_refill_us, ..
        } = evaluate_attention(&ctx)
        {
            assert_eq!(budget_refill_us, Some(9_000_000));
        } else {
            panic!("Expected Coalesce");
        }
    }

    // ─── Normal pass ─────────────────────────────────────────────────────────

    #[test]
    fn test_normal_passes_within_budget_no_quiet_hours() {
        let ctx = base_ctx();
        assert_eq!(evaluate_attention(&ctx), AttentionDecision::Pass);
    }

    // ─── Quiet hours inactive: LOW is not discarded ───────────────────────────

    #[test]
    fn test_low_not_discarded_outside_quiet_hours() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = false;
        ctx.interruption_class = InterruptionClass::Low;

        // LOW outside quiet hours: subject to budget check
        // Budget not exhausted → Pass
        assert_eq!(evaluate_attention(&ctx), AttentionDecision::Pass);
    }

    // ─── into_queue_reason helpers ───────────────────────────────────────────

    #[test]
    fn test_into_queue_reason_quiet_hours() {
        let d = AttentionDecision::QueueQuietHours {
            window_end_us: Some(42),
        };
        let reason = d.into_queue_reason();
        assert!(matches!(
            reason,
            Some(QueueReason::QuietHours {
                window_end_us: Some(42)
            })
        ));
    }

    #[test]
    fn test_into_queue_reason_coalesce() {
        let d = AttentionDecision::Coalesce {
            per_agent: true,
            per_zone: false,
            budget_refill_us: None,
        };
        let reason = d.into_queue_reason();
        assert!(matches!(
            reason,
            Some(QueueReason::AttentionBudgetExhausted {
                per_agent: true,
                per_zone: false
            })
        ));
    }

    #[test]
    fn test_into_queue_reason_pass_is_none() {
        let d = AttentionDecision::Pass;
        assert_eq!(d.into_queue_reason(), None);
    }

    #[test]
    fn test_into_queue_reason_discard_is_none() {
        let d = AttentionDecision::Discard;
        assert_eq!(d.into_queue_reason(), None);
    }

    // ─── Safe-mode cross-level scenario ──────────────────────────────────────

    /// WHEN Level 1 (Safety) enters safe mode while Level 4 (Attention) has queued
    /// notifications THEN the queued notifications MUST be discarded (spec lines 36-38).
    ///
    /// This is enforced at the pipeline layer (not in this function). The attention
    /// evaluator's purity means it cannot clear a queue — that's the caller's job.
    /// We verify here that the evaluator correctly signals Queue for NORMAL during
    /// quiet hours, so the pipeline knows what to discard on safe mode transition.
    #[test]
    fn test_safe_mode_precondition_normal_queued_during_quiet_hours() {
        let mut ctx = base_ctx();
        ctx.quiet_hours_active = true;
        ctx.quiet_hours_end_us = Some(1_000_000);
        ctx.interruption_class = InterruptionClass::Normal;

        // Evaluator says: Queue
        let decision = evaluate_attention(&ctx);
        assert!(
            matches!(&decision, AttentionDecision::QueueQuietHours { .. }),
            "NORMAL during quiet hours: evaluator must say QueueQuietHours"
        );
        // The pipeline is responsible for discarding this queue if safe mode fires.
    }
}
