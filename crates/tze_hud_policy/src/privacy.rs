//! # Level 2 Privacy — Evaluation, Redaction, and Zone Ceiling Rule
//!
//! Implements Level 2 privacy enforcement per spec §Requirement: Level 2 Privacy Evaluation,
//! §Requirement: Privacy Zone Ceiling Rule, and §Requirement: Redaction Ownership.
//!
//! ## Access Matrix (spec §2.2)
//!
//! | Viewer Class       | Sees Public | Sees Household | Sees Private | Sees Sensitive |
//! |--------------------|-------------|----------------|--------------|----------------|
//! | Owner              | ✓           | ✓              | ✓            | ✓              |
//! | HouseholdMember    | ✓           | ✓              | ✗            | ✗              |
//! | KnownGuest         | ✓           | ✗              | ✗            | ✗              |
//! | Unknown            | ✓           | ✗              | ✗            | ✗              |
//! | Nobody             | ✓           | ✗              | ✗            | ✗              |
//!
//! ## Zone Ceiling Rule (spec §Requirement: Privacy Zone Ceiling Rule)
//!
//! `effective_classification = max(agent_declared, zone_default)`
//!
//! An agent cannot reduce the visibility restriction of its content below the zone's default.
//!
//! ## Redaction Ownership
//!
//! Level 2 owns ALL redaction decisions. The `[privacy]` config section is the
//! single source of truth for `redaction_style`. `ChromeConfig` MUST NOT contain
//! `redaction_style`. The chrome layer renders the visual but does not decide what to redact.

use crate::types::{PrivacyContext, RedactionReason, ViewerClass, VisibilityClassification};

// ─── Zone ceiling rule ────────────────────────────────────────────────────────

/// Apply the zone ceiling rule: effective classification is `max(agent_declared, zone_default)`.
///
/// An agent declares a classification for its content, but the zone imposes a minimum
/// floor (default classification). The agent cannot escalate visibility beyond the zone's
/// ceiling (i.e., cannot make content more visible than the zone allows).
///
/// # Example
///
/// WHEN agent declares `public` classification in a zone with `household` default
/// THEN effective classification is `household` (the higher restriction).
///
/// The `VisibilityClassification` enum is ordered from least restrictive (Public=0)
/// to most restrictive (Sensitive=3), so `max` picks the more restrictive.
#[inline]
pub fn apply_zone_ceiling(
    agent_declared: VisibilityClassification,
    zone_default: VisibilityClassification,
) -> VisibilityClassification {
    // Higher enum value = more restrictive. The zone ceiling is the floor:
    // effective = max(agent_declared, zone_default) in restriction order.
    if zone_default > agent_declared {
        zone_default
    } else {
        agent_declared
    }
}

// ─── Privacy evaluation ───────────────────────────────────────────────────────

/// Result of Level 2 privacy evaluation for a single mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrivacyDecision {
    /// Content is visible to the effective viewer — commit without redaction.
    Visible,
    /// Content must be redacted. The mutation is COMMITTED but rendered with a
    /// redaction placeholder. (Transform override type — never Suppress.)
    Redact(RedactionReason),
}

/// Evaluate Level 2 privacy for a mutation.
///
/// Takes the effective classification after zone ceiling has been applied and
/// produces a `PrivacyDecision`.
///
/// # Arguments
///
/// - `ctx` — the privacy context (effective viewer class, viewer list, redaction style)
/// - `effective_classification` — `max(agent_declared, zone_default)` already applied
///
/// # Returns
///
/// `PrivacyDecision::Visible` if the effective viewer may see the content;
/// `PrivacyDecision::Redact(reason)` otherwise.
///
/// # Privacy transitions
///
/// Transitions MUST complete in < 2 frames (33.2ms). This function is pure and
/// O(1); timing is a runtime scheduling concern.
pub fn evaluate_privacy(
    ctx: &PrivacyContext,
    effective_classification: VisibilityClassification,
) -> PrivacyDecision {
    if ctx.effective_viewer_class.may_see(effective_classification) {
        PrivacyDecision::Visible
    } else {
        let reason = compute_redaction_reason(ctx, effective_classification);
        PrivacyDecision::Redact(reason)
    }
}

/// Compute the `RedactionReason` for a content/viewer mismatch.
fn compute_redaction_reason(
    ctx: &PrivacyContext,
    classification: VisibilityClassification,
) -> RedactionReason {
    if ctx.viewer_classes.len() > 1 {
        RedactionReason::MultiViewerRestriction
    } else {
        RedactionReason::ViewerClassInsufficient {
            required: classification,
            actual: ctx.effective_viewer_class,
        }
    }
}

// ─── Multi-viewer helper ──────────────────────────────────────────────────────

/// Given a slice of active viewer classes, return the most restrictive one.
///
/// When multiple viewers are present, the most restrictive viewer class MUST apply
/// (spec §2.2: "Nobody > Unknown > KnownGuest > HouseholdMember > Owner").
///
/// Returns `ViewerClass::Nobody` if the slice is empty (no viewer present).
pub fn most_restrictive_viewer(viewers: &[ViewerClass]) -> ViewerClass {
    viewers
        .iter()
        .copied()
        .reduce(ViewerClass::most_restrictive)
        .unwrap_or(ViewerClass::Nobody)
}

#[cfg(test)]
mod privacy_tests {
    use super::*;
    use crate::types::{PrivacyContext, RedactionStyle};

    fn make_ctx(viewer: ViewerClass) -> PrivacyContext {
        PrivacyContext {
            effective_viewer_class: viewer,
            viewer_classes: vec![viewer],
            redaction_style: RedactionStyle::Pattern,
        }
    }

    // ─── Zone ceiling rule ────────────────────────────────────────────────────

    /// WHEN agent declares public classification in zone with household default
    /// THEN effective classification is household (spec lines 113-115)
    #[test]
    fn test_zone_ceiling_enforced_public_vs_household() {
        let effective = apply_zone_ceiling(
            VisibilityClassification::Public,
            VisibilityClassification::Household,
        );
        assert_eq!(effective, VisibilityClassification::Household);
    }

    #[test]
    fn test_zone_ceiling_agent_higher_than_zone_default() {
        // Agent declares private in a public zone → effective is private (agent's is more restrictive)
        let effective = apply_zone_ceiling(
            VisibilityClassification::Private,
            VisibilityClassification::Public,
        );
        assert_eq!(effective, VisibilityClassification::Private);
    }

    #[test]
    fn test_zone_ceiling_equal_classification() {
        let effective = apply_zone_ceiling(
            VisibilityClassification::Household,
            VisibilityClassification::Household,
        );
        assert_eq!(effective, VisibilityClassification::Household);
    }

    #[test]
    fn test_zone_ceiling_zone_sensitive_enforces_ceiling() {
        // Zone default is sensitive → regardless of agent's public declaration, effective is sensitive
        let effective = apply_zone_ceiling(
            VisibilityClassification::Public,
            VisibilityClassification::Sensitive,
        );
        assert_eq!(effective, VisibilityClassification::Sensitive);
    }

    // ─── Access matrix ────────────────────────────────────────────────────────

    /// WHEN tile has private classification and viewer is known_guest
    /// THEN tile committed with redaction (spec lines 97-98)
    #[test]
    fn test_private_content_redacted_for_known_guest() {
        let ctx = make_ctx(ViewerClass::KnownGuest);
        let decision = evaluate_privacy(&ctx, VisibilityClassification::Private);
        assert!(
            matches!(decision, PrivacyDecision::Redact(_)),
            "Private content must be redacted for KnownGuest"
        );
    }

    /// WHEN sole viewer is owner THEN all content shown without redaction (spec lines 104-106)
    #[test]
    fn test_owner_sees_sensitive_without_redaction() {
        let ctx = make_ctx(ViewerClass::Owner);
        let decision = evaluate_privacy(&ctx, VisibilityClassification::Sensitive);
        assert_eq!(decision, PrivacyDecision::Visible);
    }

    #[test]
    fn test_household_member_sees_household_without_redaction() {
        let ctx = make_ctx(ViewerClass::HouseholdMember);
        let decision = evaluate_privacy(&ctx, VisibilityClassification::Household);
        assert_eq!(decision, PrivacyDecision::Visible);
    }

    #[test]
    fn test_household_member_does_not_see_private() {
        let ctx = make_ctx(ViewerClass::HouseholdMember);
        let decision = evaluate_privacy(&ctx, VisibilityClassification::Private);
        assert!(matches!(decision, PrivacyDecision::Redact(_)));
    }

    #[test]
    fn test_public_content_visible_to_all() {
        for viewer in [
            ViewerClass::Owner,
            ViewerClass::HouseholdMember,
            ViewerClass::KnownGuest,
            ViewerClass::Unknown,
            ViewerClass::Nobody,
        ] {
            let ctx = make_ctx(viewer);
            let decision = evaluate_privacy(&ctx, VisibilityClassification::Public);
            assert_eq!(
                decision,
                PrivacyDecision::Visible,
                "Public content must be visible to {viewer:?}"
            );
        }
    }

    // ─── Multi-viewer restriction ─────────────────────────────────────────────

    /// WHEN owner and guest both present THEN most restrictive viewer class (guest) applies
    /// (spec lines 100-102)
    #[test]
    fn test_multi_viewer_most_restrictive_applies() {
        // Most restrictive pre-computed and placed in effective_viewer_class
        let ctx = PrivacyContext {
            effective_viewer_class: ViewerClass::KnownGuest, // most restrictive
            viewer_classes: vec![ViewerClass::Owner, ViewerClass::KnownGuest],
            redaction_style: RedactionStyle::Pattern,
        };
        let decision = evaluate_privacy(&ctx, VisibilityClassification::Household);
        // KnownGuest cannot see Household content
        assert!(matches!(
            decision,
            PrivacyDecision::Redact(RedactionReason::MultiViewerRestriction)
        ));
    }

    #[test]
    fn test_single_viewer_reason_is_viewer_class_insufficient() {
        let ctx = make_ctx(ViewerClass::Unknown);
        let decision = evaluate_privacy(&ctx, VisibilityClassification::Household);
        assert!(matches!(
            decision,
            PrivacyDecision::Redact(RedactionReason::ViewerClassInsufficient {
                required: VisibilityClassification::Household,
                actual: ViewerClass::Unknown,
            })
        ));
    }

    // ─── most_restrictive_viewer helper ──────────────────────────────────────

    #[test]
    fn test_most_restrictive_viewer_empty_returns_nobody() {
        assert_eq!(most_restrictive_viewer(&[]), ViewerClass::Nobody);
    }

    #[test]
    fn test_most_restrictive_viewer_single() {
        assert_eq!(
            most_restrictive_viewer(&[ViewerClass::HouseholdMember]),
            ViewerClass::HouseholdMember
        );
    }

    #[test]
    fn test_most_restrictive_viewer_owner_plus_guest() {
        let result = most_restrictive_viewer(&[ViewerClass::Owner, ViewerClass::KnownGuest]);
        assert_eq!(result, ViewerClass::KnownGuest);
    }

    #[test]
    fn test_most_restrictive_viewer_nobody_dominates() {
        let result = most_restrictive_viewer(&[
            ViewerClass::Owner,
            ViewerClass::HouseholdMember,
            ViewerClass::Nobody,
        ]);
        assert_eq!(result, ViewerClass::Nobody);
    }
}
