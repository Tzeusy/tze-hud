//! # ArbitrationStack — Seven-Level Policy Evaluation
//!
//! Implements the fixed 7-level arbitration stack per policy-arbitration/spec.md.
//!
//! ## Key Properties
//!
//! - **Pure function**: `evaluate` is side-effect-free. All state is in `PolicyContext`.
//! - **Short-circuit**: evaluation stops at the first decisive level.
//! - **Immutable ordering**: levels 0-6 are doctrine; the array is `const`.
//! - **Override composition**: handled explicitly (Block+Transform → queued-with-redaction).
//!
//! ## Level Summary (spec §1.1)
//!
//! | Level | Name           | Override Types       |
//! |-------|----------------|----------------------|
//! | 0     | Human Override | Suppress/Redirect/Block |
//! | 1     | Safety         | Suppress/Redirect    |
//! | 2     | Privacy        | Transform            |
//! | 3     | Security       | Suppress             |
//! | 4     | Attention      | Block                |
//! | 5     | Resource       | Suppress/Transform   |
//! | 6     | Content        | Suppress             |

use crate::types::{
    ArbitrationError, ArbitrationErrorCode, ArbitrationLevel, ArbitrationOutcome,
    AttentionContext, BlockReason, ContentContext, InterruptionClass, MutationKind, OverrideState,
    PolicyContext, PrivacyContext, QueueReason, RedactionReason, ResourceContext,
    SecurityContext, VisibilityClassification,
};
use tze_hud_scene::{SceneId, types::ContentionPolicy};

/// The arbitration stack: a stateless evaluator over `PolicyContext`.
///
/// Instantiate once; call `evaluate` for every mutation.
pub struct ArbitrationStack;

impl ArbitrationStack {
    /// Create a new arbitration stack.
    pub fn new() -> Self {
        ArbitrationStack
    }

    /// Evaluate a mutation against the full arbitration stack.
    ///
    /// # Arguments
    /// - `ctx` — read-only policy snapshot (safety, privacy, security, attention, resource, content)
    /// - `mutation_ref` — a unique ID identifying this mutation (for error reporting)
    /// - `content_classification` — the content's visibility classification (used by Level 2)
    /// - `required_capabilities` — capabilities the agent must hold (checked at Level 3)
    /// - `target_namespace` — the namespace this mutation writes to (namespace isolation check)
    /// - `kind` — the mutation kind, which selects the evaluation path
    ///
    /// # Returns
    /// An `ArbitrationOutcome` — one of: Commit, CommitRedacted, Queue, Reject, Shed, Blocked.
    ///
    /// # Pure function contract
    /// This function has no side effects. It reads `ctx` and returns a decision.
    /// The caller is responsible for executing the outcome.
    pub fn evaluate(
        &self,
        ctx: &PolicyContext,
        mutation_ref: SceneId,
        content_classification: VisibilityClassification,
        required_capabilities: &[&str],
        target_namespace: &str,
        kind: MutationKind,
    ) -> ArbitrationOutcome {
        match kind {
            MutationKind::ZonePublication => self.evaluate_zone_publication(
                ctx,
                mutation_ref,
                content_classification,
                required_capabilities,
                target_namespace,
            ),
            MutationKind::TileMutation | MutationKind::Transactional => self
                .evaluate_tile_mutation(
                    ctx,
                    mutation_ref,
                    required_capabilities,
                    target_namespace,
                    kind == MutationKind::Transactional,
                ),
        }
    }

    // ─── Zone publication path: 0 → 3 → 2 → 4 → 5 → 6 ──────────────────────

    fn evaluate_zone_publication(
        &self,
        ctx: &PolicyContext,
        mutation_ref: SceneId,
        content_classification: VisibilityClassification,
        required_capabilities: &[&str],
        target_namespace: &str,
    ) -> ArbitrationOutcome {
        // Level 0: Human Override — freeze check
        if let Some(outcome) = self.evaluate_level0_freeze(&ctx.override_state, mutation_ref) {
            return outcome;
        }

        // Level 3: Security gate (before Privacy — Security rejects, no redaction needed)
        if let Some(outcome) = self.evaluate_level3_security(
            &ctx.security_context,
            required_capabilities,
            target_namespace,
            mutation_ref,
        ) {
            return outcome;
        }

        // Level 2: Privacy decoration (Transform — never suppresses zone publications)
        let redacted = self.evaluate_level2_privacy_redaction(
            &ctx.privacy_context,
            content_classification,
        );

        // Level 4: Attention gate
        let attention_outcome =
            self.evaluate_level4_attention(&ctx.attention_context, mutation_ref);

        // Compose Level 2 (Transform) with Level 4 (Block).
        // If both would apply: queued-with-redaction (spec §7.3).
        match attention_outcome {
            Some(ArbitrationOutcome::Queue { queue_reason, earliest_present_us, .. }) => {
                // Level 4 blocks; if Level 2 would also redact, compose them.
                return ArbitrationOutcome::Queue {
                    queue_reason,
                    earliest_present_us,
                    redacted,
                };
            }
            Some(other) => return other,
            None => {}
        }

        // Level 5: Resource gate
        if let Some(outcome) =
            self.evaluate_level5_resource(&ctx.resource_context, mutation_ref, false)
        {
            return outcome;
        }

        // Level 6: Content (zone contention)
        if let Some(outcome) = self.evaluate_level6_content(&ctx.content_context, mutation_ref) {
            return outcome;
        }

        // All levels passed.
        if redacted {
            let redaction_reason = self.compute_redaction_reason(&ctx.privacy_context, content_classification);
            ArbitrationOutcome::CommitRedacted { redaction_reason }
        } else {
            ArbitrationOutcome::Commit
        }
    }

    // ─── Tile mutation path: 3 → 5 → 6 ─────────────────────────────────────

    fn evaluate_tile_mutation(
        &self,
        ctx: &PolicyContext,
        mutation_ref: SceneId,
        required_capabilities: &[&str],
        target_namespace: &str,
        is_transactional: bool,
    ) -> ArbitrationOutcome {
        // Level 3: Security
        if let Some(outcome) = self.evaluate_level3_security(
            &ctx.security_context,
            required_capabilities,
            target_namespace,
            mutation_ref,
        ) {
            return outcome;
        }

        // Level 5: Resource
        if let Some(outcome) =
            self.evaluate_level5_resource(&ctx.resource_context, mutation_ref, is_transactional)
        {
            return outcome;
        }

        // Level 6: Content
        if let Some(outcome) = self.evaluate_level6_content(&ctx.content_context, mutation_ref) {
            return outcome;
        }

        ArbitrationOutcome::Commit
    }

    // ─── Per-level evaluators ─────────────────────────────────────────────────

    /// Level 0: Human Override — freeze check.
    ///
    /// Returns `Some(Blocked)` if freeze is active, `None` otherwise.
    /// Note: auto-unfreeze timeout is checked by the shell, not here.
    fn evaluate_level0_freeze(
        &self,
        state: &OverrideState,
        _mutation_ref: SceneId,
    ) -> Option<ArbitrationOutcome> {
        if state.freeze_active {
            Some(ArbitrationOutcome::Blocked { block_reason: BlockReason::Freeze })
        } else {
            None
        }
    }

    /// Level 2: Privacy redaction decoration.
    ///
    /// Returns `true` if the content should be redacted (CommitRedacted or queued-with-redaction).
    /// Privacy uses Transform — the mutation is COMMITTED, not rejected.
    fn evaluate_level2_privacy_redaction(
        &self,
        ctx: &PrivacyContext,
        classification: VisibilityClassification,
    ) -> bool {
        !ctx.effective_viewer_class.may_see(classification)
    }

    /// Compute the redaction reason given the privacy context and classification.
    fn compute_redaction_reason(
        &self,
        ctx: &PrivacyContext,
        classification: VisibilityClassification,
    ) -> RedactionReason {
        if ctx.viewer_classes.len() > 1 {
            // Multiple viewers — most restrictive rule applied.
            RedactionReason::MultiViewerRestriction
        } else {
            RedactionReason::ViewerClassInsufficient {
                required: classification,
                actual: ctx.effective_viewer_class,
            }
        }
    }

    /// Level 3: Security gate.
    ///
    /// Returns `Some(Reject(...))` if any security check fails, `None` if all pass.
    /// Security is conjunctive: all required capabilities must pass.
    fn evaluate_level3_security(
        &self,
        ctx: &SecurityContext,
        required_capabilities: &[&str],
        target_namespace: &str,
        mutation_ref: SceneId,
    ) -> Option<ArbitrationOutcome> {
        // Lease validity check
        if !ctx.lease_valid {
            return Some(ArbitrationOutcome::Reject(ArbitrationError {
                code: ArbitrationErrorCode::LeaseInvalid,
                agent_id: ctx.agent_namespace.clone(),
                mutation_ref,
                message: "Lease is not in Active state".to_string(),
                hint: None,
                level: ArbitrationLevel::Security.index(),
            }));
        }

        // Namespace isolation check
        if !target_namespace.is_empty()
            && !ctx.agent_namespace.is_empty()
            && target_namespace != ctx.agent_namespace
        {
            return Some(ArbitrationOutcome::Reject(ArbitrationError {
                code: ArbitrationErrorCode::NamespaceViolation,
                agent_id: ctx.agent_namespace.clone(),
                mutation_ref,
                message: format!(
                    "Namespace violation: agent '{}' may not write to '{}'",
                    ctx.agent_namespace, target_namespace
                ),
                hint: Some(format!("Agent namespace: '{}'", ctx.agent_namespace)),
                level: ArbitrationLevel::Security.index(),
            }));
        }

        // Capability checks (conjunctive — all must pass)
        for &cap in required_capabilities {
            if !ctx.has_capability(cap) {
                return Some(ArbitrationOutcome::Reject(ArbitrationError {
                    code: ArbitrationErrorCode::CapabilityDenied,
                    agent_id: ctx.agent_namespace.clone(),
                    mutation_ref,
                    message: format!("Missing required capability: '{}'", cap),
                    hint: Some(format!("Required: '{}'", cap)),
                    level: ArbitrationLevel::Security.index(),
                }));
            }
        }

        None
    }

    /// Level 4: Attention gate.
    ///
    /// Returns `Some(Queue(...))` if the mutation should be deferred.
    /// CRITICAL interruptions bypass both quiet hours and budget.
    fn evaluate_level4_attention(
        &self,
        ctx: &AttentionContext,
        _mutation_ref: SceneId,
    ) -> Option<ArbitrationOutcome> {
        // CRITICAL bypasses everything.
        if ctx.interruption_class == InterruptionClass::Critical {
            return None;
        }

        // SILENT has zero budget cost and always passes.
        if ctx.interruption_class == InterruptionClass::Silent {
            return None;
        }

        // LOW is discarded during quiet hours (not queued; too stale to be useful).
        // Handled by the caller as a Reject? Per spec §11.5, LOW is "discarded during quiet hours".
        // The spec says Queue for NORMAL, Discard for LOW. LOW discard = Suppress outcome.
        // However, spec §3.4 says only "Queue(QuietHours)" as outcomes, not Reject.
        // Reading spec scene-events/spec.md line 70: "LOW SHALL be discarded".
        // We use Reject here conceptually — but spec says "not queued". We don't have a Discard
        // outcome variant; treat it as Shed (no error) since it's not a security/policy violation.
        // Actually, for policy-arbitration purposes, LOW during quiet hours is a Queue-like behavior
        // that the spec says to discard. We return Queue to indicate deferral, but the layer above
        // this (the evaluation pipeline, bead #2/#3) will handle the discard semantics.
        // For this stack, we Queue it; the caller distinguishes LOW+quiet_hours as Discard.
        if ctx.quiet_hours_active {
            match ctx.interruption_class {
                InterruptionClass::Critical => unreachable!("handled above"),
                InterruptionClass::Silent => unreachable!("handled above"),
                InterruptionClass::Low => {
                    // LOW is discarded during quiet hours. Signal as Shed (no error, zone-state applies).
                    return Some(ArbitrationOutcome::Shed { degradation_level: 0 });
                }
                InterruptionClass::Normal => {
                    // NORMAL is queued until quiet hours end.
                    return Some(ArbitrationOutcome::Queue {
                        queue_reason: QueueReason::QuietHours {
                            window_end_us: ctx.quiet_hours_end_us,
                        },
                        earliest_present_us: ctx.quiet_hours_end_us,
                        redacted: false, // will be overwritten by compose logic above
                    });
                }
                InterruptionClass::High => {
                    // Queue HIGH if the mutation's interruption class is less urgent than the
                    // zone's pass-through threshold (spec §4.2).
                    // InterruptionClass ordering: Critical(0) < High(1) < Normal(2) < Low(3).
                    // "Less urgent" means numerically greater. So if interruption_class > pass_through_class,
                    // the mutation does not meet the threshold and must be queued.
                    if ctx.interruption_class > ctx.pass_through_class {
                        return Some(ArbitrationOutcome::Queue {
                            queue_reason: QueueReason::QuietHours {
                                window_end_us: ctx.quiet_hours_end_us,
                            },
                            earliest_present_us: ctx.quiet_hours_end_us,
                            redacted: false,
                        });
                    }
                    // Otherwise HIGH meets the threshold and passes quiet hours.
                }
            }
        }

        // Attention budget check (CRITICAL is already exempt above).
        let agent_exhausted = ctx.agent_budget_exhausted();
        let zone_exhausted = ctx.zone_budget_exhausted();
        if agent_exhausted || zone_exhausted {
            // When budget is exhausted, mutations are coalesced (latest-wins).
            // At the stack level, this is represented as Queue(AttentionBudgetExhausted).
            return Some(ArbitrationOutcome::Queue {
                queue_reason: QueueReason::AttentionBudgetExhausted {
                    per_agent: agent_exhausted,
                    per_zone: zone_exhausted,
                },
                earliest_present_us: ctx.budget_refill_us,
                redacted: false,
            });
        }

        None
    }

    /// Level 5: Resource gate.
    ///
    /// Returns `Some(Reject(TileBudgetExceeded))` if the per-agent tile budget is exceeded
    /// (spec §7.2 line 169: "Over-budget batches MUST be rejected atomically" — agent informed).
    /// Returns `Some(Shed)` if degradation shedding applies.
    /// Transactional mutations are NEVER shed (spec §11.6).
    /// Returns `None` if budgets are paused (during freeze) or all checks pass.
    fn evaluate_level5_resource(
        &self,
        ctx: &ResourceContext,
        mutation_ref: SceneId,
        is_transactional: bool,
    ) -> Option<ArbitrationOutcome> {
        // During freeze, resource budgets are paused (spec §6.2).
        if ctx.budgets_paused {
            return None;
        }

        // Per-agent budget exceeded → reject the batch atomically (spec §7.2 line 169).
        // "Over-budget batches MUST be rejected atomically." The agent IS informed via structured
        // error (Reject, not Shed). Shed means no error to agent; Reject means agent is informed.
        if ctx.budget_exceeded {
            return Some(ArbitrationOutcome::Reject(ArbitrationError {
                code: ArbitrationErrorCode::TileBudgetExceeded,
                agent_id: String::new(), // filled by pipeline layer
                mutation_ref,
                message: "Per-agent tile budget exceeded; batch rejected atomically".to_string(),
                hint: Some("Reduce tile count or wait for budget refill".to_string()),
                level: ArbitrationLevel::Resource.index(),
            }));
        }

        // Degradation shedding — transactional mutations are never shed.
        if ctx.should_shed && !is_transactional {
            return Some(ArbitrationOutcome::Shed {
                degradation_level: ctx.degradation_level,
            });
        }

        None
    }

    /// Level 6: Content (zone contention resolution).
    ///
    /// Returns `Some(Reject(ZoneEvictionDenied))` if a Replace zone eviction fails
    /// (lower-priority agent cannot evict a higher-priority occupant).
    /// Returns `None` for all other contention policies (LatestWins, Stack, MergeByKey).
    fn evaluate_level6_content(
        &self,
        ctx: &ContentContext,
        mutation_ref: SceneId,
    ) -> Option<ArbitrationOutcome> {
        match &ctx.contention_policy {
            Some(ContentionPolicy::Replace) => {
                // Replace: single occupant. Eviction requires equal-or-higher lease priority.
                // Lower numeric priority value = higher priority (spec RFC 0008 §2.2).
                if let Some(occupant_priority) = ctx.occupant_lease_priority
                    && ctx.agent_lease_priority > occupant_priority
                {
                    // Agent has lower priority — cannot evict.
                    return Some(ArbitrationOutcome::Reject(ArbitrationError {
                        code: ArbitrationErrorCode::ZoneEvictionDenied,
                        agent_id: String::new(), // filled by pipeline layer
                        mutation_ref,
                        message: format!(
                            "Zone eviction denied: agent priority {} < occupant priority {}",
                            ctx.agent_lease_priority, occupant_priority
                        ),
                        hint: Some("Higher-priority occupant holds this Replace zone".to_string()),
                        level: ArbitrationLevel::Content.index(),
                    }));
                }
                None
            }
            Some(ContentionPolicy::Stack { max_depth }) => {
                // Stack: depth check
                if ctx.stack_depth >= u32::from(*max_depth) {
                    // Stack is full. This is a content rejection.
                    return Some(ArbitrationOutcome::Reject(ArbitrationError {
                        code: ArbitrationErrorCode::ZoneEvictionDenied,
                        agent_id: String::new(),
                        mutation_ref,
                        message: format!(
                            "Stack zone at max depth {}/{}",
                            ctx.stack_depth, max_depth
                        ),
                        hint: Some("Stack zone is full".to_string()),
                        level: ArbitrationLevel::Content.index(),
                    }));
                }
                None
            }
            // LatestWins and MergeByKey always accept (no eviction rejection possible).
            Some(ContentionPolicy::LatestWins) | Some(ContentionPolicy::MergeByKey { .. }) | None => None,
        }
    }

    // ─── Stack-level query helpers ────────────────────────────────────────────

    /// Verify the stack contains exactly 7 levels in the correct order.
    ///
    /// This is a compile-time invariant, but this method enables runtime assertion in tests.
    pub fn assert_stack_invariants(&self) {
        let levels = ArbitrationLevel::ALL;
        assert_eq!(levels.len(), 7, "Stack must contain exactly 7 levels");
        assert_eq!(levels[0], ArbitrationLevel::HumanOverride, "Level 0 must be HumanOverride");
        assert_eq!(levels[1], ArbitrationLevel::Safety, "Level 1 must be Safety");
        assert_eq!(levels[2], ArbitrationLevel::Privacy, "Level 2 must be Privacy");
        assert_eq!(levels[3], ArbitrationLevel::Security, "Level 3 must be Security");
        assert_eq!(levels[4], ArbitrationLevel::Attention, "Level 4 must be Attention");
        assert_eq!(levels[5], ArbitrationLevel::Resource, "Level 5 must be Resource");
        assert_eq!(levels[6], ArbitrationLevel::Content, "Level 6 must be Content");
        for (i, level) in levels.iter().enumerate() {
            assert_eq!(level.index(), i as u8, "Level index must match position");
        }
    }
}

impl Default for ArbitrationStack {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Policy evaluator trait ────────────────────────────────────────────────────

/// The PolicyEvaluator trait encoding policy-arbitration/spec.md contract.
///
/// Implementors provide a pure function over PolicyContext. Tests from Epic 0
/// (rig-ho4b) exercise this trait.
pub trait PolicyEvaluator {
    /// Evaluate a mutation against the policy stack.
    ///
    /// - Input: typed `PolicyContext` (all relevant runtime state as read-only snapshot)
    /// - Output: `ArbitrationOutcome` with the applied level, action, and reason
    /// - Must evaluate levels in order (0→6 for zone publications, 3→5→6 for tile mutations)
    /// - Must short-circuit on first decisive result
    /// - MUST be a pure function: no side effects, no writes to shared state
    fn evaluate_mutation(
        &self,
        ctx: &PolicyContext,
        mutation_ref: SceneId,
        content_classification: VisibilityClassification,
        required_capabilities: &[&str],
        target_namespace: &str,
        kind: MutationKind,
    ) -> ArbitrationOutcome;
}

impl PolicyEvaluator for ArbitrationStack {
    fn evaluate_mutation(
        &self,
        ctx: &PolicyContext,
        mutation_ref: SceneId,
        content_classification: VisibilityClassification,
        required_capabilities: &[&str],
        target_namespace: &str,
        kind: MutationKind,
    ) -> ArbitrationOutcome {
        self.evaluate(
            ctx,
            mutation_ref,
            content_classification,
            required_capabilities,
            target_namespace,
            kind,
        )
    }
}
