//! # Level 5 Resource — Per-Agent Budget Enforcement and Degradation Shedding
//!
//! Implements Level 5 resource enforcement per spec §Requirement: Level 5 Resource Enforcement
//! and §Requirement: Degradation Does Not Bypass Arbitration.
//!
//! ## Key Behaviors
//!
//! - Over-budget batches are **rejected atomically** (agent receives structured error).
//! - Degradation shedding does **NOT** produce an error to the agent.
//! - Zone state MUST be updated even for shed mutations (scene state correct, render omitted).
//! - Transactional mutations (`CreateTile`, `DeleteTile`, `LeaseRequest`, `LeaseRelease`)
//!   MUST never be shed.
//! - Resource budgets are paused during freeze (Level 0).
//!
//! ## Degradation Does Not Bypass Arbitration (spec §12.2)
//!
//! A mutation shed at Level 5 MUST have already passed all higher levels (3, 2, 4).
//! The per-mutation pipeline enforces this by evaluating Level 5 after higher levels.

use crate::types::{ArbitrationError, ArbitrationErrorCode, ArbitrationLevel, ArbitrationOutcome, MutationKind, ResourceContext};
use tze_hud_scene::SceneId;

// ─── Resource decision ────────────────────────────────────────────────────────

/// Outcome of Level 5 resource evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceDecision {
    /// Mutation passes resource checks — proceed to Level 6.
    Pass,
    /// Mutation must be shed (degradation shedding — no error to agent).
    /// Zone state is updated; render output is omitted.
    Shed { degradation_level: u32 },
    /// Per-agent tile budget exceeded — reject the batch atomically.
    /// Agent IS informed via structured error (Reject, not Shed).
    BudgetExceeded,
    /// Budgets are paused (during freeze). Mutation passes resource gate.
    BudgetsPaused,
}

/// Evaluate Level 5 resource enforcement.
///
/// # Arguments
///
/// - `ctx` — resource context snapshot
/// - `mutation_ref` — mutation ID for error construction
/// - `kind` — mutation kind; transactional mutations are NEVER shed
///
/// # Returns
///
/// A `ResourceDecision` describing the resource outcome.
pub fn evaluate_resource(
    ctx: &ResourceContext,
    _mutation_ref: SceneId,
    kind: MutationKind,
) -> ResourceDecision {
    // During freeze, resource budgets are paused (spec §6.2).
    if ctx.budgets_paused {
        return ResourceDecision::BudgetsPaused;
    }

    // Per-agent budget exceeded → reject atomically (spec §7.2 line 169).
    if ctx.budget_exceeded {
        return ResourceDecision::BudgetExceeded;
    }

    // Degradation shedding — transactional mutations are never shed (spec §11.6).
    if ctx.should_shed && kind != MutationKind::Transactional {
        return ResourceDecision::Shed { degradation_level: ctx.degradation_level };
    }

    ResourceDecision::Pass
}

/// Convert a `ResourceDecision` to an `ArbitrationOutcome`.
///
/// This is a convenience function for the per-mutation pipeline layer.
pub fn resource_decision_to_outcome(
    decision: ResourceDecision,
    agent_id: impl Into<String>,
    mutation_ref: SceneId,
) -> Option<ArbitrationOutcome> {
    match decision {
        ResourceDecision::Pass | ResourceDecision::BudgetsPaused => None,
        ResourceDecision::Shed { degradation_level } => {
            Some(ArbitrationOutcome::Shed { degradation_level })
        }
        ResourceDecision::BudgetExceeded => Some(ArbitrationOutcome::Reject(ArbitrationError {
            code: ArbitrationErrorCode::TileBudgetExceeded,
            agent_id: agent_id.into(),
            mutation_ref,
            message: "Per-agent tile budget exceeded; batch rejected atomically".to_string(),
            hint: Some("Reduce tile count or wait for budget refill".to_string()),
            level: ArbitrationLevel::Resource.index(),
        })),
    }
}

/// Returns `true` if the given mutation kind is transactional (must never be shed).
///
/// Transactional mutations: `CreateTile`, `DeleteTile`, `LeaseRequest`, `LeaseRelease`.
/// These are represented in the policy crate as `MutationKind::Transactional`.
#[inline]
pub fn is_transactional(kind: MutationKind) -> bool {
    kind == MutationKind::Transactional
}

#[cfg(test)]
mod resource_tests {
    use super::*;
    use crate::types::ResourceContext;

    fn default_ctx() -> ResourceContext {
        ResourceContext {
            degradation_level: 0,
            tiles_used: 0,
            tiles_limit: 100,
            should_shed: false,
            is_transactional: false,
            budget_exceeded: false,
            budgets_paused: false,
        }
    }

    // ─── Transactional mutations never shed (spec lines 177-179) ─────────────

    /// WHEN degradation at Level 5 and CreateTile mutation arrives
    /// THEN mutation NOT shed, passes through (spec lines 177-179)
    #[test]
    fn test_transactional_mutation_never_shed() {
        let ctx = ResourceContext { should_shed: true, degradation_level: 5, ..default_ctx() };
        let decision = evaluate_resource(&ctx, SceneId::new(), MutationKind::Transactional);
        assert_eq!(decision, ResourceDecision::Pass);
    }

    /// WHEN degradation at Level 5 and non-transactional mutation arrives
    /// THEN mutation IS shed (no error)
    #[test]
    fn test_non_transactional_mutation_shed() {
        let ctx =
            ResourceContext { should_shed: true, degradation_level: 3, ..default_ctx() };
        let decision = evaluate_resource(&ctx, SceneId::new(), MutationKind::TileMutation);
        assert_eq!(decision, ResourceDecision::Shed { degradation_level: 3 });
    }

    #[test]
    fn test_zone_publication_shed_when_degrading() {
        let ctx = ResourceContext { should_shed: true, degradation_level: 2, ..default_ctx() };
        let decision = evaluate_resource(&ctx, SceneId::new(), MutationKind::ZonePublication);
        assert_eq!(decision, ResourceDecision::Shed { degradation_level: 2 });
    }

    // ─── Budget exceeded → Reject (spec §7.2 line 169) ───────────────────────

    #[test]
    fn test_budget_exceeded_is_rejected_not_shed() {
        let ctx = ResourceContext { budget_exceeded: true, ..default_ctx() };
        let decision = evaluate_resource(&ctx, SceneId::new(), MutationKind::TileMutation);
        assert_eq!(decision, ResourceDecision::BudgetExceeded);
    }

    /// Budget exceeded takes priority over shedding
    #[test]
    fn test_budget_exceeded_takes_priority_over_shedding() {
        let ctx = ResourceContext {
            budget_exceeded: true,
            should_shed: true,
            degradation_level: 5,
            ..default_ctx()
        };
        let decision = evaluate_resource(&ctx, SceneId::new(), MutationKind::TileMutation);
        // BudgetExceeded must win (agent is informed via Reject)
        assert_eq!(decision, ResourceDecision::BudgetExceeded);
    }

    // ─── Budgets paused during freeze ─────────────────────────────────────────

    /// WHEN resource budgets are paused (freeze active) THEN mutation passes resource gate
    #[test]
    fn test_budgets_paused_during_freeze() {
        let ctx = ResourceContext {
            budgets_paused: true,
            should_shed: true,
            budget_exceeded: true,
            ..default_ctx()
        };
        // Even if shed and budget_exceeded would normally fire, paused budgets take precedence
        let decision = evaluate_resource(&ctx, SceneId::new(), MutationKind::ZonePublication);
        assert_eq!(decision, ResourceDecision::BudgetsPaused);
    }

    // ─── Normal pass ──────────────────────────────────────────────────────────

    #[test]
    fn test_nominal_pass() {
        let ctx = default_ctx();
        let decision = evaluate_resource(&ctx, SceneId::new(), MutationKind::ZonePublication);
        assert_eq!(decision, ResourceDecision::Pass);
    }

    // ─── resource_decision_to_outcome conversion ──────────────────────────────

    #[test]
    fn test_resource_pass_produces_no_outcome() {
        let outcome = resource_decision_to_outcome(ResourceDecision::Pass, "agent_a", SceneId::new());
        assert!(outcome.is_none());
    }

    #[test]
    fn test_resource_budgets_paused_produces_no_outcome() {
        let outcome =
            resource_decision_to_outcome(ResourceDecision::BudgetsPaused, "agent_a", SceneId::new());
        assert!(outcome.is_none());
    }

    #[test]
    fn test_resource_shed_produces_shed_outcome() {
        let outcome = resource_decision_to_outcome(
            ResourceDecision::Shed { degradation_level: 4 },
            "agent_a",
            SceneId::new(),
        );
        assert!(matches!(outcome, Some(ArbitrationOutcome::Shed { degradation_level: 4 })));
    }

    #[test]
    fn test_resource_budget_exceeded_produces_reject_outcome() {
        let mutation_ref = SceneId::new();
        let outcome = resource_decision_to_outcome(
            ResourceDecision::BudgetExceeded,
            "agent_a",
            mutation_ref,
        );
        assert!(
            matches!(
                &outcome,
                Some(ArbitrationOutcome::Reject(err))
                    if err.code == ArbitrationErrorCode::TileBudgetExceeded
                    && err.level == ArbitrationLevel::Resource.index()
            ),
            "BudgetExceeded must produce Reject(TileBudgetExceeded) at Level 5"
        );
    }

    // ─── is_transactional helper ──────────────────────────────────────────────

    #[test]
    fn test_is_transactional_true_for_transactional() {
        assert!(is_transactional(MutationKind::Transactional));
    }

    #[test]
    fn test_is_transactional_false_for_non_transactional() {
        assert!(!is_transactional(MutationKind::TileMutation));
        assert!(!is_transactional(MutationKind::ZonePublication));
    }

    /// WHEN mutation shed at Level 5 THEN it has already passed Levels 3, 2, 4 (spec lines 360-362)
    /// This is enforced by the pipeline in mutation.rs; here we verify the resource module
    /// itself only yields Shed for non-transactional mutations under degradation.
    #[test]
    fn test_shed_mutation_has_passed_capability_check() {
        // The resource module is responsible only for Level 5 evaluation.
        // The per-mutation pipeline (mutation.rs) enforces ordering:
        // a Shed result here means all higher levels already returned None.
        let ctx = ResourceContext { should_shed: true, degradation_level: 2, ..default_ctx() };
        let decision = evaluate_resource(&ctx, SceneId::new(), MutationKind::TileMutation);
        // If we get Shed, it means Level 3 (security) and earlier checks passed.
        assert_eq!(decision, ResourceDecision::Shed { degradation_level: 2 });
    }
}
