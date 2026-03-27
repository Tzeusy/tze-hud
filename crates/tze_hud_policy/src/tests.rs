//! Tests for the seven-level arbitration stack.
//!
//! Each test maps to a WHEN/THEN scenario from policy-arbitration/spec.md.
//! Tests are organized by spec requirement.

#[cfg(test)]
mod arbitration_stack_tests {
    use crate::{
        ArbitrationErrorCode, ArbitrationLevel, ArbitrationOutcome, ArbitrationStack,
        AttentionContext, BlockReason, ContentContext, InterruptionClass, MutationKind,
        OverrideState, PolicyContext, PrivacyContext, ResourceContext, SafetyState,
        SecurityContext, ViewerClass, VisibilityClassification,
    };
    use tze_hud_scene::{SceneId, types::ContentionPolicy};

    // ─── Helpers ──────────────────────────────────────────────────────────────

    fn default_policy_context() -> PolicyContext {
        PolicyContext {
            override_state: OverrideState {
                freeze_active: false,
                safe_mode_active: false,
                freeze_duration_ms: 0,
                max_freeze_duration_ms: 300_000,
            },
            safety_state: SafetyState {
                gpu_healthy: true,
                scene_graph_intact: true,
                frame_time_p95_us: 5_000,
                emergency_threshold_us: 14_000,
            },
            privacy_context: PrivacyContext {
                effective_viewer_class: ViewerClass::Owner,
                viewer_classes: vec![ViewerClass::Owner],
                redaction_style: crate::RedactionStyle::Pattern,
            },
            security_context: SecurityContext {
                granted_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                    "publish_zone:subtitle".to_string(),
                ],
                agent_namespace: "agent_a".to_string(),
                lease_valid: true,
                lease_id: Some(SceneId::new()),
            },
            attention_context: AttentionContext {
                quiet_hours_active: false,
                quiet_hours_end_us: None,
                per_agent_interruptions_last_60s: 0,
                per_agent_limit: 20,
                per_zone_interruptions_last_60s: 0,
                per_zone_limit: 10,
                pass_through_class: InterruptionClass::High,
                interruption_class: InterruptionClass::Normal,
                budget_refill_us: None,
            },
            resource_context: ResourceContext {
                degradation_level: 0,
                tiles_used: 0,
                tiles_limit: 100,
                should_shed: false,
                is_transactional: false,
                budget_exceeded: false,
                budgets_paused: false,
            },
            content_context: ContentContext {
                zone_name: Some("subtitle".to_string()),
                contention_policy: Some(ContentionPolicy::LatestWins),
                agent_lease_priority: 2,
                occupant_lease_priority: None,
                stack_depth: 0,
                max_stack_depth: 8,
            },
        }
    }

    fn make_stack() -> ArbitrationStack {
        ArbitrationStack::new()
    }

    // ─── Requirement: Stack levels are fixed (spec lines 15-17) ──────────────

    /// WHEN the arbitration stack is initialized
    /// THEN it contains exactly 7 levels numbered 0-6 with Human Override at 0 and Content at 6
    #[test]
    fn test_stack_has_exactly_7_levels() {
        let stack = make_stack();
        stack.assert_stack_invariants();

        let levels = ArbitrationLevel::ALL;
        assert_eq!(levels.len(), 7);
        assert_eq!(levels[0], ArbitrationLevel::HumanOverride);
        assert_eq!(levels[6], ArbitrationLevel::Content);
    }

    #[test]
    fn test_stack_level_indices_are_0_through_6() {
        for (i, level) in ArbitrationLevel::ALL.iter().enumerate() {
            assert_eq!(level.index(), i as u8);
        }
    }

    #[test]
    fn test_stack_ordering_is_immutable() {
        // Verify that higher levels have lower numeric index
        assert!(ArbitrationLevel::HumanOverride < ArbitrationLevel::Safety);
        assert!(ArbitrationLevel::Safety < ArbitrationLevel::Privacy);
        assert!(ArbitrationLevel::Privacy < ArbitrationLevel::Security);
        assert!(ArbitrationLevel::Security < ArbitrationLevel::Attention);
        assert!(ArbitrationLevel::Attention < ArbitrationLevel::Resource);
        assert!(ArbitrationLevel::Resource < ArbitrationLevel::Content);
    }

    // ─── Requirement: Override Type Taxonomy (spec lines 242-253) ────────────

    #[test]
    fn test_level0_override_types() {
        let types = ArbitrationLevel::HumanOverride.permitted_override_types();
        use crate::OverrideType;
        assert!(types.contains(&crate::OverrideType::Suppress));
        assert!(types.contains(&OverrideType::Redirect));
        assert!(types.contains(&OverrideType::Block));
    }

    #[test]
    fn test_level1_override_types() {
        let types = ArbitrationLevel::Safety.permitted_override_types();
        use crate::OverrideType;
        assert!(types.contains(&OverrideType::Suppress));
        assert!(types.contains(&OverrideType::Redirect));
        assert!(!types.contains(&OverrideType::Block));
        assert!(!types.contains(&OverrideType::Transform));
    }

    #[test]
    fn test_level2_override_types_transform_only() {
        let types = ArbitrationLevel::Privacy.permitted_override_types();
        use crate::OverrideType;
        assert_eq!(types.len(), 1);
        assert_eq!(types[0], OverrideType::Transform);
    }

    #[test]
    fn test_level3_override_types_suppress_only() {
        let types = ArbitrationLevel::Security.permitted_override_types();
        use crate::OverrideType;
        assert_eq!(types.len(), 1);
        assert_eq!(types[0], OverrideType::Suppress);
    }

    #[test]
    fn test_level4_override_types_block_only() {
        let types = ArbitrationLevel::Attention.permitted_override_types();
        use crate::OverrideType;
        assert_eq!(types.len(), 1);
        assert_eq!(types[0], OverrideType::Block);
    }

    #[test]
    fn test_level5_override_types() {
        let types = ArbitrationLevel::Resource.permitted_override_types();
        use crate::OverrideType;
        assert!(types.contains(&OverrideType::Suppress));
        assert!(types.contains(&OverrideType::Transform));
    }

    #[test]
    fn test_level6_override_types_suppress_only() {
        let types = ArbitrationLevel::Content.permitted_override_types();
        use crate::OverrideType;
        assert_eq!(types.len(), 1);
        assert_eq!(types[0], OverrideType::Suppress);
    }

    // ─── Requirement: ArbitrationOutcome Types (spec lines 229-241) ──────────

    /// WHEN CommitRedacted outcome
    /// THEN mutation committed but rendering uses redaction placeholder
    #[test]
    fn test_commit_redacted_outcome_for_private_content_guest_viewer() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        // Private content, guest viewer
        ctx.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
        ctx.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Private,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert!(
            matches!(&outcome, ArbitrationOutcome::CommitRedacted { .. }),
            "Expected CommitRedacted, got {outcome:?}"
        );
    }

    /// WHEN scene frozen and agent submits mutation
    /// THEN outcome is Blocked with block_reason=Freeze, mutation queued
    #[test]
    fn test_blocked_by_freeze() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.override_state.freeze_active = true;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert_eq!(
            outcome,
            ArbitrationOutcome::Blocked {
                block_reason: BlockReason::Freeze
            }
        );
    }

    // ─── Requirement: Cross-Level Conflict Resolution (spec lines 19-46) ─────

    /// WHEN Level 2 (Privacy) says "redact tile" but Level 6 (Content) says "show tile"
    /// THEN the tile is redacted and the content contention result is irrelevant
    #[test]
    fn test_privacy_overrides_content() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
        ctx.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];
        // Content says "latest wins" — would normally allow
        ctx.content_context.contention_policy = Some(ContentionPolicy::LatestWins);

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Private,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        // Privacy must win: CommitRedacted (not Commit)
        assert!(
            matches!(&outcome, ArbitrationOutcome::CommitRedacted { .. }),
            "Privacy must override Content: expected CommitRedacted, got {outcome:?}"
        );
    }

    /// WHEN Level 0 (Human) freezes the scene but Level 5 (Resource) would shed a tile
    /// THEN the tile stays frozen and is not shed
    #[test]
    fn test_human_override_overrides_resource_shed() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.override_state.freeze_active = true;
        ctx.resource_context.should_shed = true;
        ctx.resource_context.degradation_level = 3;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        // Level 0 must win: Blocked (not Shed)
        assert_eq!(
            outcome,
            ArbitrationOutcome::Blocked {
                block_reason: BlockReason::Freeze
            },
            "Human Override must win over Resource shed"
        );
    }

    /// WHEN Level 3 (Security) denies a capability
    /// THEN Level 5 (Resource) and Level 6 (Content) are never evaluated for that mutation
    #[test]
    fn test_security_short_circuits_lower_levels() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        // Remove the required capability
        ctx.security_context.granted_capabilities = vec![];
        // Even if resource would shed and content would reject, security wins
        ctx.resource_context.should_shed = true;
        ctx.content_context.contention_policy = Some(ContentionPolicy::Replace);
        ctx.content_context.occupant_lease_priority = Some(0); // high priority occupant

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["create_tiles"],
            "agent_a",
            MutationKind::TileMutation,
        );

        // Must be Reject (Security), not Shed or ZoneEvictionDenied
        assert!(
            matches!(&outcome, ArbitrationOutcome::Reject(err) if
                err.code == crate::ArbitrationErrorCode::CapabilityDenied &&
                err.level == ArbitrationLevel::Security.index()
            ),
            "Security must short-circuit: expected Reject(CapabilityDenied), got {outcome:?}"
        );
    }

    /// WHEN Level 1 (Safety) enters safe mode while Level 4 (Attention) has queued notifications
    /// THEN the queued notifications MUST be discarded; safe mode overrides
    ///
    /// This is a design-level invariant: the stack short-circuits at Level 0 freeze.
    /// Safe mode is a Level 1 concern managed by the shell; we verify that when freeze
    /// is active, the stack returns Blocked before reaching Attention.
    #[test]
    fn test_freeze_short_circuits_attention_queue() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.override_state.freeze_active = true;
        // Attention would queue (quiet hours active)
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.interruption_class = InterruptionClass::Normal;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        // Freeze must win — Blocked, not Queue
        assert_eq!(
            outcome,
            ArbitrationOutcome::Blocked {
                block_reason: BlockReason::Freeze
            }
        );
    }

    // ─── Requirement: Override Composition (spec lines 255-267) ─────────────

    /// WHEN Level 0 blocks (freeze) and Level 5 would suppress (budget)
    /// THEN the mutation is blocked (freeze wins)
    #[test]
    fn test_block_wins_over_suppress_freeze_vs_budget() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.override_state.freeze_active = true;
        ctx.resource_context.budget_exceeded = true;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert_eq!(
            outcome,
            ArbitrationOutcome::Blocked {
                block_reason: BlockReason::Freeze
            },
            "Freeze (Block) must win over budget exceeded (Suppress)"
        );
    }

    /// WHEN Level 2 redacts and Level 4 queues for quiet hours
    /// THEN the mutation is queued with a redaction flag and rendered with placeholder when delivered
    #[test]
    fn test_transform_and_block_composed_redacted_queued() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        // Privacy would redact
        ctx.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
        ctx.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];
        // Attention would queue (quiet hours)
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.quiet_hours_end_us = Some(1_000_000);
        ctx.attention_context.interruption_class = InterruptionClass::Normal;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Private,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        // Must be queued-with-redaction
        match &outcome {
            ArbitrationOutcome::Queue {
                redacted,
                queue_reason,
                ..
            } => {
                assert!(redacted, "Queued mutation must carry redaction flag");
                assert!(
                    matches!(queue_reason, crate::QueueReason::QuietHours { .. }),
                    "Queue reason must be QuietHours"
                );
            }
            other => panic!("Expected Queue with redacted=true, got {other:?}"),
        }
    }

    // ─── Requirement: Within-Level Conflict Resolution (spec lines 342-354) ──

    /// WHEN two viewers with different access levels present
    /// THEN most restrictive applies
    #[test]
    fn test_level2_most_restrictive_viewer_wins() {
        // Owner + Guest → Guest (most restrictive)
        let combined = ViewerClass::most_restrictive(ViewerClass::Owner, ViewerClass::KnownGuest);
        assert_eq!(combined, ViewerClass::KnownGuest);

        // HouseholdMember + Unknown → Unknown
        let combined2 =
            ViewerClass::most_restrictive(ViewerClass::HouseholdMember, ViewerClass::Unknown);
        assert_eq!(combined2, ViewerClass::Unknown);

        // Nobody is the most restrictive of all
        let combined3 = ViewerClass::most_restrictive(ViewerClass::Owner, ViewerClass::Nobody);
        assert_eq!(combined3, ViewerClass::Nobody);
    }

    #[test]
    fn test_privacy_context_compute_effective() {
        let viewers = vec![ViewerClass::Owner, ViewerClass::KnownGuest];
        let effective = PrivacyContext::compute_effective(&viewers);
        assert_eq!(effective, ViewerClass::KnownGuest);
    }

    /// WHEN mutation requires both create_tiles and publish_zone:subtitle capabilities
    /// THEN both must pass; failure of either rejects
    #[test]
    fn test_level3_conjunctive_both_capabilities_required() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        // Agent only has create_tiles but NOT publish_zone:subtitle
        ctx.security_context.granted_capabilities = vec!["create_tiles".to_string()];

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["create_tiles", "publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert!(
            matches!(&outcome, ArbitrationOutcome::Reject(err) if
                err.code == crate::ArbitrationErrorCode::CapabilityDenied
            ),
            "Conjunctive check: missing second capability must reject"
        );
    }

    #[test]
    fn test_level3_conjunctive_first_capability_missing() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        // Agent has second but not first
        ctx.security_context.granted_capabilities = vec!["publish_zone:subtitle".to_string()];

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["create_tiles", "publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert!(
            matches!(&outcome, ArbitrationOutcome::Reject(err) if
                err.code == crate::ArbitrationErrorCode::CapabilityDenied
            ),
            "First missing capability must reject"
        );
    }

    // ─── Requirement: Freeze Semantics at Level 0 (spec lines 268-279) ───────

    /// WHEN scene frozen and frame-time would trigger degradation
    /// THEN the degradation ladder does not advance
    /// (In the stack, this is represented by budgets_paused=true during freeze)
    #[test]
    fn test_resource_budgets_paused_during_freeze() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.override_state.freeze_active = true;
        ctx.resource_context.budgets_paused = true;
        ctx.resource_context.should_shed = true; // would shed if budgets were active

        // Freeze wins first (Level 0)
        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );
        assert_eq!(
            outcome,
            ArbitrationOutcome::Blocked {
                block_reason: BlockReason::Freeze
            }
        );
    }

    // ─── Privacy access matrix (spec lines 91-104) ───────────────────────────

    #[test]
    fn test_owner_sees_all_content() {
        assert!(ViewerClass::Owner.may_see(VisibilityClassification::Public));
        assert!(ViewerClass::Owner.may_see(VisibilityClassification::Household));
        assert!(ViewerClass::Owner.may_see(VisibilityClassification::Private));
        assert!(ViewerClass::Owner.may_see(VisibilityClassification::Sensitive));
    }

    #[test]
    fn test_household_member_sees_public_and_household() {
        assert!(ViewerClass::HouseholdMember.may_see(VisibilityClassification::Public));
        assert!(ViewerClass::HouseholdMember.may_see(VisibilityClassification::Household));
        assert!(!ViewerClass::HouseholdMember.may_see(VisibilityClassification::Private));
        assert!(!ViewerClass::HouseholdMember.may_see(VisibilityClassification::Sensitive));
    }

    #[test]
    fn test_known_guest_sees_only_public() {
        assert!(ViewerClass::KnownGuest.may_see(VisibilityClassification::Public));
        assert!(!ViewerClass::KnownGuest.may_see(VisibilityClassification::Household));
        assert!(!ViewerClass::KnownGuest.may_see(VisibilityClassification::Private));
        assert!(!ViewerClass::KnownGuest.may_see(VisibilityClassification::Sensitive));
    }

    #[test]
    fn test_unknown_viewer_sees_only_public() {
        assert!(ViewerClass::Unknown.may_see(VisibilityClassification::Public));
        assert!(!ViewerClass::Unknown.may_see(VisibilityClassification::Household));
    }

    #[test]
    fn test_nobody_sees_only_public() {
        assert!(ViewerClass::Nobody.may_see(VisibilityClassification::Public));
        assert!(!ViewerClass::Nobody.may_see(VisibilityClassification::Household));
    }

    #[test]
    fn test_owner_with_public_content_commits_directly() {
        let stack = make_stack();
        let ctx = default_policy_context(); // Owner viewer, public content

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert_eq!(outcome, ArbitrationOutcome::Commit);
    }

    // ─── Requirement: Level 3 Security specific cases ────────────────────────

    #[test]
    fn test_namespace_violation_rejected() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        // Agent namespace is "agent_a" but target is "agent_b"
        ctx.security_context.agent_namespace = "agent_a".to_string();

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["modify_own_tiles"],
            "agent_b", // different namespace
            MutationKind::TileMutation,
        );

        assert!(
            matches!(&outcome, ArbitrationOutcome::Reject(err) if
                err.code == crate::ArbitrationErrorCode::NamespaceViolation
            ),
            "Namespace violation must be rejected"
        );
    }

    #[test]
    fn test_invalid_lease_rejected() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.security_context.lease_valid = false;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["create_tiles"],
            "agent_a",
            MutationKind::Transactional,
        );

        assert!(matches!(&outcome, ArbitrationOutcome::Reject(err) if
            err.code == crate::ArbitrationErrorCode::LeaseInvalid
        ));
    }

    #[test]
    fn test_publish_zone_wildcard_capability() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.security_context.granted_capabilities = vec!["publish_zone:*".to_string()];

        // Should match any publish_zone:<name>
        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:notification"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert_eq!(outcome, ArbitrationOutcome::Commit);
    }

    // ─── Requirement: Attention Management (spec lines 143-166) ─────────────

    #[test]
    fn test_quiet_hours_queue_normal_interruption() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.quiet_hours_end_us = Some(7_200_000_000); // 2 hours in future
        ctx.attention_context.interruption_class = InterruptionClass::Normal;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert!(
            matches!(
                &outcome,
                ArbitrationOutcome::Queue {
                    queue_reason: crate::QueueReason::QuietHours { .. },
                    ..
                }
            ),
            "NORMAL during quiet hours must be queued"
        );
    }

    #[test]
    fn test_quiet_hours_discard_low_interruption() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.interruption_class = InterruptionClass::Low;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        // LOW during quiet hours → Shed (discarded, no error)
        assert!(
            matches!(&outcome, ArbitrationOutcome::Shed { .. }),
            "LOW during quiet hours must be discarded (Shed)"
        );
    }

    #[test]
    fn test_quiet_hours_high_passes_with_default_pass_through() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.interruption_class = InterruptionClass::High;
        ctx.attention_context.pass_through_class = InterruptionClass::High; // default

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        // HIGH passes when pass_through_class is High
        assert_eq!(outcome, ArbitrationOutcome::Commit);
    }

    #[test]
    fn test_critical_bypasses_quiet_hours_and_budget() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.interruption_class = InterruptionClass::Critical;
        ctx.attention_context.per_agent_interruptions_last_60s = 100; // exhausted
        ctx.attention_context.per_zone_interruptions_last_60s = 100; // exhausted

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        // CRITICAL bypasses everything
        assert_eq!(outcome, ArbitrationOutcome::Commit);
    }

    #[test]
    fn test_attention_budget_exhausted_queues() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.attention_context.per_agent_interruptions_last_60s = 20; // at limit
        ctx.attention_context.per_agent_limit = 20;
        ctx.attention_context.interruption_class = InterruptionClass::Normal;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert!(
            matches!(
                &outcome,
                ArbitrationOutcome::Queue {
                    queue_reason: crate::QueueReason::AttentionBudgetExhausted {
                        per_agent: true,
                        ..
                    },
                    ..
                }
            ),
            "Budget exhausted must queue"
        );
    }

    // ─── Requirement: Resource Enforcement (spec lines 168-179) ─────────────

    #[test]
    fn test_transactional_mutation_never_shed() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.resource_context.should_shed = true;
        ctx.resource_context.degradation_level = 5;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["create_tiles"],
            "agent_a",
            MutationKind::Transactional,
        );

        // Transactional mutations are NEVER shed
        assert_ne!(
            outcome,
            ArbitrationOutcome::Shed {
                degradation_level: 5
            },
            "Transactional mutation must not be shed"
        );
        assert_eq!(outcome, ArbitrationOutcome::Commit);
    }

    #[test]
    fn test_shed_non_transactional_mutation() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.resource_context.should_shed = true;
        ctx.resource_context.degradation_level = 3;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["create_tiles"],
            "agent_a",
            MutationKind::TileMutation,
        );

        assert_eq!(
            outcome,
            ArbitrationOutcome::Shed {
                degradation_level: 3
            }
        );
    }

    // ─── Requirement: Content Level (spec lines 181-193) ─────────────────────

    #[test]
    fn test_replace_zone_eviction_denied_lower_priority() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.content_context.contention_policy = Some(ContentionPolicy::Replace);
        ctx.content_context.agent_lease_priority = 3; // lower priority
        ctx.content_context.occupant_lease_priority = Some(1); // higher priority occupant

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert!(
            matches!(&outcome, ArbitrationOutcome::Reject(err) if
                err.code == crate::ArbitrationErrorCode::ZoneEvictionDenied
            ),
            "Lower-priority agent must not evict higher-priority occupant"
        );
    }

    #[test]
    fn test_replace_zone_eviction_allowed_equal_priority() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.content_context.contention_policy = Some(ContentionPolicy::Replace);
        ctx.content_context.agent_lease_priority = 2;
        ctx.content_context.occupant_lease_priority = Some(2); // equal priority

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert_eq!(outcome, ArbitrationOutcome::Commit);
    }

    #[test]
    fn test_latest_wins_zone_always_commits() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.content_context.contention_policy = Some(ContentionPolicy::LatestWins);
        ctx.content_context.occupant_lease_priority = Some(0); // even highest priority occupant

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert_eq!(outcome, ArbitrationOutcome::Commit);
    }

    #[test]
    fn test_stack_zone_full_rejected() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.content_context.contention_policy = Some(ContentionPolicy::Stack { max_depth: 8 });
        ctx.content_context.stack_depth = 8; // at max
        ctx.content_context.max_stack_depth = 8;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert!(
            matches!(&outcome, ArbitrationOutcome::Reject(err) if
                err.code == crate::ArbitrationErrorCode::ZoneEvictionDenied &&
                err.level == ArbitrationLevel::Content.index()
            ),
            "Full stack zone must reject"
        );
    }

    // ─── Viewer dismisses tile with valid lease (spec lines 44-46) ───────────

    /// WHEN the viewer dismisses a tile whose agent holds a valid ACTIVE lease
    /// THEN the lease is revoked immediately (Level 0 wins over Level 3 Security)
    ///
    /// Note: The dismiss command is a Level 0 Override Command processed by the
    /// `OverrideCommandQueue` before any `MutationBatch` intake (spec §3.3, §3.4).
    /// The arbitration stack enforces this for zone publications (which check Level 0).
    /// For tile mutations, the dismiss manifests as a Blocked outcome when freeze is active
    /// (freeze is the Level 0 mechanism that blocks all agent input during override).
    /// This test verifies Level 0 freeze blocks zone publications — the per-mutation path
    /// that does include override preemption.
    #[test]
    fn test_viewer_dismisses_tile_level0_wins_over_security() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        // Simulate Level 0 active (freeze/dismiss) — shell-owned state transition
        ctx.override_state.freeze_active = true;
        // Agent has valid, high-priority lease with all capabilities
        ctx.security_context.lease_valid = true;
        ctx.security_context.granted_capabilities = vec![
            "create_tiles".to_string(),
            "modify_own_tiles".to_string(),
            "overlay_privileges".to_string(),
            "publish_zone:subtitle".to_string(),
        ];

        // Zone publications check Level 0 first (spec §3.4 full path)
        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        // Level 0 wins even over valid lease (Level 3)
        assert_eq!(
            outcome,
            ArbitrationOutcome::Blocked {
                block_reason: BlockReason::Freeze
            },
            "Level 0 freeze must block zone publications before Security check"
        );
    }

    /// Tile mutations do not include Level 0 in their evaluation path (spec §3.4).
    /// Dismiss/freeze at Level 0 is handled by the override command queue before
    /// mutation intake, so tile mutations from a frozen scene don't reach the stack.
    #[test]
    fn test_tile_mutation_path_does_not_include_level0() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        // Freeze active — but tile mutation path is 3→5→6, not 0→3→2→4→5→6
        ctx.override_state.freeze_active = true;
        ctx.security_context.lease_valid = true;
        ctx.security_context.granted_capabilities = vec!["modify_own_tiles".to_string()];

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["modify_own_tiles"],
            "agent_a",
            MutationKind::TileMutation,
        );

        // Tile mutation path: 3→5→6. Freeze is not checked on this path.
        // The freeze enforcement happens in the pipeline layer (bead #2/#3) BEFORE
        // the mutation reaches the stack, or via the OverrideCommandQueue.
        // Here, the stack evaluates only what it's responsible for.
        assert_eq!(
            outcome,
            ArbitrationOutcome::Commit,
            "Tile mutation path skips Level 0 — freeze enforced at pipeline layer"
        );
    }

    // ─── Purity constraint verification ──────────────────────────────────────

    /// Verify the stack is stateless — same context always produces same result.
    #[test]
    fn test_evaluate_is_deterministic() {
        let stack = make_stack();
        let ctx = default_policy_context();
        let mutation_ref = SceneId::new();

        let result1 = stack.evaluate(
            &ctx,
            mutation_ref,
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );
        let result2 = stack.evaluate(
            &ctx,
            mutation_ref,
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert_eq!(result1, result2, "Stack must be deterministic");
    }

    // ─── Error level attribution ──────────────────────────────────────────────

    #[test]
    fn test_security_rejection_has_correct_level() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.security_context.granted_capabilities = vec![];

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["create_tiles"],
            "agent_a",
            MutationKind::Transactional,
        );

        if let ArbitrationOutcome::Reject(err) = outcome {
            assert_eq!(err.level, 3, "Security rejection must name Level 3");
        } else {
            panic!("Expected Reject");
        }
    }

    #[test]
    fn test_zone_eviction_rejection_has_correct_level() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.content_context.contention_policy = Some(ContentionPolicy::Replace);
        ctx.content_context.agent_lease_priority = 3;
        ctx.content_context.occupant_lease_priority = Some(1);

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        if let ArbitrationOutcome::Reject(err) = outcome {
            assert_eq!(err.level, 6, "Zone eviction rejection must name Level 6");
        } else {
            panic!("Expected Reject");
        }
    }

    // ─── Bug fixes: spec compliance ───────────────────────────────────────────

    /// WHEN the per-agent tile budget is exceeded
    /// THEN the batch is rejected atomically with TileBudgetExceeded (spec §7.2 line 169).
    ///
    /// "Over-budget batches MUST be rejected atomically." The agent IS informed via a structured
    /// Reject error. Shed would NOT inform the agent — that is spec non-compliant.
    #[test]
    fn test_budget_exceeded_returns_reject_not_shed() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.resource_context.budget_exceeded = true;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["create_tiles"],
            "agent_a",
            MutationKind::TileMutation,
        );

        assert!(
            matches!(
                &outcome,
                ArbitrationOutcome::Reject(err)
                    if err.code == ArbitrationErrorCode::TileBudgetExceeded
                    && err.level == ArbitrationLevel::Resource.index()
            ),
            "Over-budget batch must be Reject(TileBudgetExceeded) at Level 5, got {outcome:?}"
        );
        // Explicitly confirm it is NOT Shed — agent must be informed.
        assert!(
            !matches!(&outcome, ArbitrationOutcome::Shed { .. }),
            "Budget exceeded must NOT return Shed (agent must be informed)"
        );
    }

    /// WHEN quiet hours are active and interruption_class is HIGH but pass_through_class is CRITICAL
    /// THEN the HIGH mutation is queued (it does not meet the stricter Critical-only threshold).
    ///
    /// Spec §4.2: mutations LESS urgent than the zone's pass-through threshold are queued.
    /// InterruptionClass ordering: Critical(0) < High(1) — so High is less urgent than Critical.
    /// If pass_through_class=Critical(0), then High(1) > Critical(0) → queued.
    #[test]
    fn test_pass_through_class_high_queued_when_threshold_is_critical() {
        let stack = make_stack();
        let mut ctx = default_policy_context();
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.quiet_hours_end_us = Some(7_200_000_000);
        ctx.attention_context.interruption_class = InterruptionClass::High;
        // Only Critical passes through — stricter than the default High threshold.
        ctx.attention_context.pass_through_class = InterruptionClass::Critical;

        let outcome = stack.evaluate(
            &ctx,
            SceneId::new(),
            VisibilityClassification::Public,
            &["publish_zone:subtitle"],
            "agent_a",
            MutationKind::ZonePublication,
        );

        assert!(
            matches!(
                &outcome,
                ArbitrationOutcome::Queue {
                    queue_reason: crate::QueueReason::QuietHours { .. },
                    ..
                }
            ),
            "HIGH must be queued when pass_through_class=Critical (threshold not met); got {outcome:?}"
        );
    }
}
