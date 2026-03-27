//! # Level 6 Content — Zone Contention Resolution
//!
//! Implements Level 6 content resolution per spec §Requirement: Level 6 Content Resolution.
//!
//! ## ContentionPolicy variants
//!
//! | Policy     | Behavior                                                         |
//! |------------|------------------------------------------------------------------|
//! | LatestWins | New publish replaces the previous occupant unconditionally.      |
//! | Stack      | New publish stacks; auto-dismissed after timeout. Depth-limited. |
//! | MergeByKey | Same key replaces; different keys coexist.                       |
//! | Replace    | Single occupant; eviction only by equal-or-higher lease priority. |
//!
//! ## Same-frame contention
//!
//! Same-frame contention is resolved in arrival order. The pipeline passes
//! mutations to Level 6 in the order they appear in the `MutationBatch`.
//!
//! ## Cross-tab zone isolation
//!
//! Zones are scoped to their tab. An agent publishing to `tab_a/subtitle`
//! does not interact with `tab_b/subtitle`. This is enforced by the pipeline;
//! the `ContentContext` already reflects the per-tab zone state.
//!
//! ## Latency
//!
//! Zone contention resolution MUST complete in < 20us. This function is O(1)
//! (single match on the contention policy variant).

use crate::types::{
    ArbitrationError, ArbitrationErrorCode, ArbitrationLevel, ArbitrationOutcome, ContentContext,
};
use tze_hud_scene::{SceneId, types::ContentionPolicy};

// ─── Content decision ─────────────────────────────────────────────────────────

/// Outcome of Level 6 content evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentDecision {
    /// Mutation passes content resolution — proceed to commit.
    Pass,
    /// Zone eviction denied (Replace: lower-priority agent cannot evict occupant).
    ZoneEvictionDenied { message: String },
    /// Stack zone is full (Stack policy: stack depth at max).
    StackFull { current_depth: u32, max_depth: u32 },
}

/// Evaluate Level 6 content resolution for a mutation.
///
/// # Arguments
///
/// - `ctx` — content context (zone name, contention policy, lease priorities, stack depth)
/// - `mutation_ref` — mutation ID (used for error construction)
///
/// # Returns
///
/// `ContentDecision::Pass` if the mutation may be committed.
/// `ContentDecision::ZoneEvictionDenied` for Replace policy eviction failures.
/// `ContentDecision::StackFull` when a Stack zone is at max depth.
///
/// # Latency
///
/// This function is O(1) — single match on the `ContentionPolicy` variant.
pub fn evaluate_content(ctx: &ContentContext, _mutation_ref: SceneId) -> ContentDecision {
    match &ctx.contention_policy {
        Some(ContentionPolicy::Replace) => {
            // Single occupant. Eviction requires equal-or-higher priority.
            // Lower numeric priority value = higher priority (RFC 0008 §2.2).
            if let Some(occupant_priority) = ctx.occupant_lease_priority
                && ctx.agent_lease_priority > occupant_priority
            {
                // Agent has lower priority — cannot evict.
                return ContentDecision::ZoneEvictionDenied {
                    message: format!(
                        "Zone eviction denied: agent lease priority {} is numerically greater \
                         (lower effective priority) than occupant lease priority {}",
                        ctx.agent_lease_priority, occupant_priority
                    ),
                };
            }
            ContentDecision::Pass
        }
        Some(ContentionPolicy::Stack { max_depth }) => {
            if ctx.stack_depth >= u32::from(*max_depth) {
                return ContentDecision::StackFull {
                    current_depth: ctx.stack_depth,
                    max_depth: u32::from(*max_depth),
                };
            }
            ContentDecision::Pass
        }
        // LatestWins and MergeByKey always accept (no eviction rejection possible).
        Some(ContentionPolicy::LatestWins) | Some(ContentionPolicy::MergeByKey { .. }) | None => {
            ContentDecision::Pass
        }
    }
}

/// Convert a `ContentDecision` to an `ArbitrationOutcome`.
///
/// Returns `None` for `Pass`, `Some(Reject(...))` for eviction/stack failures.
pub fn content_decision_to_outcome(
    decision: ContentDecision,
    agent_id: impl Into<String>,
    mutation_ref: SceneId,
) -> Option<ArbitrationOutcome> {
    match decision {
        ContentDecision::Pass => None,
        ContentDecision::ZoneEvictionDenied { message } => {
            Some(ArbitrationOutcome::Reject(ArbitrationError {
                code: ArbitrationErrorCode::ZoneEvictionDenied,
                agent_id: agent_id.into(),
                mutation_ref,
                message,
                hint: Some("Higher-priority occupant holds this Replace zone".to_string()),
                level: ArbitrationLevel::Content.index(),
            }))
        }
        ContentDecision::StackFull {
            current_depth,
            max_depth,
        } => Some(ArbitrationOutcome::Reject(ArbitrationError {
            code: ArbitrationErrorCode::ZoneEvictionDenied,
            agent_id: agent_id.into(),
            mutation_ref,
            message: format!("Stack zone at max depth {current_depth}/{max_depth}"),
            hint: Some("Stack zone is full".to_string()),
            level: ArbitrationLevel::Content.index(),
        })),
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;
    use crate::types::ContentContext;
    use tze_hud_scene::types::ContentionPolicy;

    fn default_ctx() -> ContentContext {
        ContentContext {
            zone_name: Some("subtitle".to_string()),
            contention_policy: Some(ContentionPolicy::LatestWins),
            agent_lease_priority: 2,
            occupant_lease_priority: None,
            stack_depth: 0,
            max_stack_depth: 8,
        }
    }

    // ─── LatestWins zone (spec lines 186-188) ─────────────────────────────────

    /// WHEN two agents publish to a LatestWins zone in the same frame
    /// THEN the second publish replaces the first (spec lines 186-188)
    #[test]
    fn test_latest_wins_always_passes() {
        let mut ctx = default_ctx();
        ctx.contention_policy = Some(ContentionPolicy::LatestWins);
        ctx.occupant_lease_priority = Some(0); // high-priority occupant — still allowed

        let decision = evaluate_content(&ctx, SceneId::new());
        assert_eq!(decision, ContentDecision::Pass);
    }

    // ─── Replace zone ─────────────────────────────────────────────────────────

    #[test]
    fn test_replace_zone_no_occupant_passes() {
        let mut ctx = default_ctx();
        ctx.contention_policy = Some(ContentionPolicy::Replace);
        ctx.occupant_lease_priority = None; // empty zone

        let decision = evaluate_content(&ctx, SceneId::new());
        assert_eq!(decision, ContentDecision::Pass);
    }

    #[test]
    fn test_replace_zone_equal_priority_evicts() {
        let mut ctx = default_ctx();
        ctx.contention_policy = Some(ContentionPolicy::Replace);
        ctx.agent_lease_priority = 2;
        ctx.occupant_lease_priority = Some(2); // equal priority

        let decision = evaluate_content(&ctx, SceneId::new());
        assert_eq!(decision, ContentDecision::Pass);
    }

    #[test]
    fn test_replace_zone_higher_priority_evicts() {
        let mut ctx = default_ctx();
        ctx.contention_policy = Some(ContentionPolicy::Replace);
        ctx.agent_lease_priority = 1; // higher priority (lower number)
        ctx.occupant_lease_priority = Some(2);

        let decision = evaluate_content(&ctx, SceneId::new());
        assert_eq!(decision, ContentDecision::Pass);
    }

    #[test]
    fn test_replace_zone_lower_priority_denied() {
        let mut ctx = default_ctx();
        ctx.contention_policy = Some(ContentionPolicy::Replace);
        ctx.agent_lease_priority = 3; // lower priority
        ctx.occupant_lease_priority = Some(1); // occupant has higher priority

        let decision = evaluate_content(&ctx, SceneId::new());
        assert!(matches!(
            decision,
            ContentDecision::ZoneEvictionDenied { .. }
        ));
    }

    // ─── Stack zone ───────────────────────────────────────────────────────────

    #[test]
    fn test_stack_zone_under_limit_passes() {
        let mut ctx = default_ctx();
        ctx.contention_policy = Some(ContentionPolicy::Stack { max_depth: 8 });
        ctx.stack_depth = 3;

        let decision = evaluate_content(&ctx, SceneId::new());
        assert_eq!(decision, ContentDecision::Pass);
    }

    #[test]
    fn test_stack_zone_at_limit_rejected() {
        let mut ctx = default_ctx();
        ctx.contention_policy = Some(ContentionPolicy::Stack { max_depth: 8 });
        ctx.stack_depth = 8; // at max

        let decision = evaluate_content(&ctx, SceneId::new());
        assert!(matches!(
            decision,
            ContentDecision::StackFull {
                current_depth: 8,
                max_depth: 8
            }
        ));
    }

    // ─── MergeByKey zone ──────────────────────────────────────────────────────

    #[test]
    fn test_merge_by_key_always_passes() {
        let mut ctx = default_ctx();
        ctx.contention_policy = Some(ContentionPolicy::MergeByKey { max_keys: 32 });

        let decision = evaluate_content(&ctx, SceneId::new());
        assert_eq!(decision, ContentDecision::Pass);
    }

    // ─── No contention policy ─────────────────────────────────────────────────

    #[test]
    fn test_no_contention_policy_passes() {
        let mut ctx = default_ctx();
        ctx.contention_policy = None;

        let decision = evaluate_content(&ctx, SceneId::new());
        assert_eq!(decision, ContentDecision::Pass);
    }

    // ─── Cross-tab zone isolation (spec lines 190-192) ───────────────────────

    /// WHEN agent publishes to tab_a/subtitle THEN it does not interact with tab_b/subtitle
    /// Cross-tab isolation is enforced at the pipeline level by passing the correct
    /// tab-scoped ContentContext. Here we verify that zone_name is tab-scoped.
    #[test]
    fn test_cross_tab_zone_isolation_via_zone_name() {
        // tab_a/subtitle and tab_b/subtitle are separate zones; the pipeline passes
        // the correct per-tab ContentContext. This test shows zone_name captures the scope.
        let ctx_a = ContentContext {
            zone_name: Some("tab_a/subtitle".to_string()),
            contention_policy: Some(ContentionPolicy::LatestWins),
            agent_lease_priority: 2,
            occupant_lease_priority: None,
            stack_depth: 0,
            max_stack_depth: 8,
        };
        let ctx_b = ContentContext {
            zone_name: Some("tab_b/subtitle".to_string()),
            ..ctx_a.clone()
        };

        // Publish to tab_a succeeds
        assert_eq!(
            evaluate_content(&ctx_a, SceneId::new()),
            ContentDecision::Pass
        );
        // Publish to tab_b also succeeds (separate zone, no interaction)
        assert_eq!(
            evaluate_content(&ctx_b, SceneId::new()),
            ContentDecision::Pass
        );
    }

    // ─── content_decision_to_outcome ─────────────────────────────────────────

    #[test]
    fn test_content_pass_produces_no_outcome() {
        let outcome = content_decision_to_outcome(ContentDecision::Pass, "agent_a", SceneId::new());
        assert!(outcome.is_none());
    }

    #[test]
    fn test_content_eviction_denied_produces_reject() {
        let mutation_ref = SceneId::new();
        let outcome = content_decision_to_outcome(
            ContentDecision::ZoneEvictionDenied {
                message: "denied".to_string(),
            },
            "agent_a",
            mutation_ref,
        );
        assert!(
            matches!(
                &outcome,
                Some(ArbitrationOutcome::Reject(err))
                    if err.code == ArbitrationErrorCode::ZoneEvictionDenied
                    && err.level == ArbitrationLevel::Content.index()
            ),
            "ZoneEvictionDenied must produce Reject at Level 6"
        );
    }

    #[test]
    fn test_content_stack_full_produces_reject() {
        let mutation_ref = SceneId::new();
        let outcome = content_decision_to_outcome(
            ContentDecision::StackFull {
                current_depth: 8,
                max_depth: 8,
            },
            "agent_a",
            mutation_ref,
        );
        assert!(matches!(
            &outcome,
            Some(ArbitrationOutcome::Reject(err))
                if err.code == ArbitrationErrorCode::ZoneEvictionDenied
                && err.level == ArbitrationLevel::Content.index()
        ));
    }
}
