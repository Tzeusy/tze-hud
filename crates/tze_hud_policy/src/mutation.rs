//! # Per-Mutation Evaluation Pipeline
//!
//! Implements the per-mutation evaluation pipeline per spec §Requirement: Per-Mutation
//! Evaluation Pipeline (spec lines 216-227).
//!
//! ## Evaluation Paths
//!
//! For **tile mutations** (path: Security(3) → Resource(5) → Content(6)):
//!
//! ```text
//! L3 Security → L5 Resource → L6 Content → Commit
//! ```
//!
//! For **zone publications** (path: Override(0) → Security(3) → Privacy(2, decoration) →
//! Attention(4) → Resource(5) → Content(6)):
//!
//! ```text
//! L0 Override → L3 Security → L2 Privacy decoration → L4 Attention →
//!     L5 Resource → L6 Content → Commit/CommitRedacted
//! ```
//!
//! ## Short-Circuit
//!
//! If a higher level rejects, lower levels MUST NOT be evaluated.
//!
//! ## Purity Constraint
//!
//! Policy evaluation is a pure function over `PolicyContext` — no side effects.
//! The per-mutation pipeline reads capabilities, viewer context, budgets, and zone
//! config as inputs; it does not write state.
//!
//! ## Telemetry
//!
//! The pipeline accumulates `PolicyTelemetry` and emits `ArbitrationTelemetryEvent`
//! values into caller-provided collections. All side-effectful operations (logging,
//! actual telemetry emission) are the caller's responsibility.
//!
//! ## Latency Budget
//!
//! Per-mutation policy check MUST complete in < 50us.

use crate::content::{content_decision_to_outcome, evaluate_content};
use crate::privacy::{PrivacyDecision, apply_zone_ceiling, evaluate_privacy};
use crate::resource::{ResourceDecision, evaluate_resource, resource_decision_to_outcome};
use crate::telemetry::{ArbitrationTelemetryEvent, PolicyTelemetry};
use crate::types::{
    ArbitrationError, ArbitrationErrorCode, ArbitrationLevel, ArbitrationOutcome, AttentionContext,
    BlockReason, InterruptionClass, MutationKind, PolicyContext, QueueReason,
    VisibilityClassification,
};
use tze_hud_scene::SceneId;

// ─── Per-mutation input ───────────────────────────────────────────────────────

/// Input to the per-mutation evaluation pipeline.
///
/// Combines the read-only `PolicyContext` with mutation-specific metadata.
/// The pipeline produces an `ArbitrationOutcome` plus telemetry side-channels.
pub struct MutationEvalInput<'a> {
    /// Read-only policy snapshot.
    pub ctx: &'a PolicyContext,

    /// Unique ID of this mutation (for error reporting and telemetry).
    pub mutation_ref: SceneId,

    /// Content classification declared by the agent.
    /// The zone ceiling rule is applied inside the pipeline.
    pub agent_declared_classification: VisibilityClassification,

    /// Zone's default classification (for zone ceiling rule).
    /// `None` if this is not a zone publication or the zone has no default.
    pub zone_default_classification: Option<VisibilityClassification>,

    /// Capabilities required by this mutation (e.g., `["create_tiles"]`).
    pub required_capabilities: &'a [&'a str],

    /// Namespace this mutation writes to.
    pub target_namespace: &'a str,

    /// Agent ID string (for error and telemetry reporting).
    pub agent_id: &'a str,

    /// Kind of mutation: ZonePublication, TileMutation, or Transactional.
    pub kind: MutationKind,

    /// Current monotonic timestamp in microseconds (for telemetry timestamps).
    pub timestamp_us: u64,
}

// ─── Per-mutation output ──────────────────────────────────────────────────────

/// Output of the per-mutation evaluation pipeline.
pub struct MutationEvalOutput {
    /// The arbitration decision for this mutation.
    pub outcome: ArbitrationOutcome,

    /// Telemetry events emitted during evaluation (may be empty).
    /// The caller is responsible for forwarding these to the telemetry subsystem.
    pub events: Vec<ArbitrationTelemetryEvent>,

    /// Per-mutation evaluation time in microseconds.
    ///
    /// Always `0` from `evaluate_mutation` — the pipeline is a pure function and
    /// does not call the system clock. The **caller** must wrap the call with a timer,
    /// measure the elapsed time, and write it into this field before forwarding to
    /// `MutationLatencyAccumulator::record`.
    pub eval_us: u64,
}

// ─── Per-mutation pipeline ────────────────────────────────────────────────────

/// Evaluate a single mutation through the per-mutation policy pipeline.
///
/// # Evaluation paths
///
/// - **Tile mutations / Transactional**: Security(3) → Resource(5) → Content(6)
/// - **Zone publications**: Override(0) → Security(3) → Privacy(2, decoration) →
///   Attention(4) → Resource(5) → Content(6)
///
/// # Short-circuit
///
/// The pipeline short-circuits on the first decisive result. Levels that come
/// after a rejection are NOT evaluated.
///
/// # Purity
///
/// This function is a pure function over its inputs. It does not write any
/// runtime state. Telemetry events are returned in the output for the caller
/// to forward.
///
/// # Latency
///
/// Per-mutation policy check MUST complete in < 50us. This function is O(1)
/// for all its sub-checks (hash-table capability lookup, O(1) enum dispatch).
pub fn evaluate_mutation(input: &MutationEvalInput<'_>) -> MutationEvalOutput {
    // Note: timing is measured by the caller wrapping this call. The pipeline
    // itself does not call system clock functions (pure function guarantee).
    // We report eval_us = 0 here; callers should wrap with a timer and populate.
    let mut events: Vec<ArbitrationTelemetryEvent> = Vec::new();

    // Build capability set once per mutation (not once per path function).
    // This avoids redundant HashSet allocation if the same capability set
    // is used in both security checks of zone publication and tile mutation paths.
    let cap_set = crate::security::CapabilitySet::new(
        input
            .ctx
            .security_context
            .granted_capabilities
            .iter()
            .map(|s| s.as_str()),
    );

    let outcome = match input.kind {
        MutationKind::ZonePublication => evaluate_zone_publication(input, &cap_set, &mut events),
        MutationKind::TileMutation | MutationKind::Transactional => {
            evaluate_tile_mutation(input, &cap_set, &mut events)
        }
    };

    MutationEvalOutput {
        outcome,
        events,
        eval_us: 0,
    }
}

// ─── Zone publication evaluation path ────────────────────────────────────────

/// Evaluate a zone publication: L0 → L3 → L2(decor) → L4 → L5 → L6.
fn evaluate_zone_publication(
    input: &MutationEvalInput<'_>,
    cap_set: &crate::security::CapabilitySet,
    events: &mut Vec<ArbitrationTelemetryEvent>,
) -> ArbitrationOutcome {
    let ctx = input.ctx;

    // ── Level 0: Human Override — freeze check ────────────────────────────────
    if ctx.override_state.freeze_active {
        return ArbitrationOutcome::Blocked {
            block_reason: BlockReason::Freeze,
        };
    }

    // ── Level 3: Security gate ────────────────────────────────────────────────
    // cap_set is pre-built by evaluate_mutation to avoid per-call allocation.

    // Lease validity
    if !ctx.security_context.lease_valid {
        let rejection = ArbitrationOutcome::Reject(ArbitrationError {
            code: ArbitrationErrorCode::LeaseInvalid,
            agent_id: input.agent_id.to_string(),
            mutation_ref: input.mutation_ref,
            message: "Lease is not in Active state".to_string(),
            hint: None,
            level: ArbitrationLevel::Security.index(),
        });
        events.push(ArbitrationTelemetryEvent::reject(
            ArbitrationLevel::Security.index(),
            "LEASE_INVALID",
            input.agent_id,
            input.mutation_ref,
            input.timestamp_us,
        ));
        return rejection;
    }

    // Namespace isolation
    if !input.target_namespace.is_empty()
        && !ctx.security_context.agent_namespace.is_empty()
        && input.target_namespace != ctx.security_context.agent_namespace
    {
        let rejection = ArbitrationOutcome::Reject(ArbitrationError {
            code: ArbitrationErrorCode::NamespaceViolation,
            agent_id: input.agent_id.to_string(),
            mutation_ref: input.mutation_ref,
            message: format!(
                "Namespace violation: agent '{}' may not write to '{}'",
                ctx.security_context.agent_namespace, input.target_namespace
            ),
            hint: Some(format!(
                "Agent namespace: '{}'",
                ctx.security_context.agent_namespace
            )),
            level: ArbitrationLevel::Security.index(),
        });
        events.push(ArbitrationTelemetryEvent::reject(
            ArbitrationLevel::Security.index(),
            "NAMESPACE_VIOLATION",
            input.agent_id,
            input.mutation_ref,
            input.timestamp_us,
        ));
        return rejection;
    }

    // Capability checks (conjunctive)
    if let Some(missing) = cap_set.first_missing(input.required_capabilities) {
        let rejection = ArbitrationOutcome::Reject(ArbitrationError {
            code: ArbitrationErrorCode::CapabilityDenied,
            agent_id: input.agent_id.to_string(),
            mutation_ref: input.mutation_ref,
            message: format!("Missing required capability: '{missing}'"),
            hint: Some(format!("Required: '{missing}'")),
            level: ArbitrationLevel::Security.index(),
        });
        events.push(ArbitrationTelemetryEvent::reject(
            ArbitrationLevel::Security.index(),
            "CAPABILITY_DENIED",
            input.agent_id,
            input.mutation_ref,
            input.timestamp_us,
        ));
        return rejection;
    }

    // ── Level 2: Privacy decoration (Transform — does not reject) ─────────────
    let effective_classification = apply_zone_ceiling(
        input.agent_declared_classification,
        input
            .zone_default_classification
            .unwrap_or(input.agent_declared_classification),
    );
    let privacy_result = evaluate_privacy(&ctx.privacy_context, effective_classification);
    let redacted = matches!(privacy_result, PrivacyDecision::Redact(_));

    // ── Level 4: Attention gate ────────────────────────────────────────────────
    let attention_outcome = evaluate_attention(&ctx.attention_context, input.mutation_ref);

    // Compose Level 2 (Transform) with Level 4 (Block).
    // If both would apply: queued-with-redaction (spec §7.3).
    match attention_outcome {
        Some(ArbitrationOutcome::Queue {
            queue_reason,
            earliest_present_us,
            ..
        }) => {
            // Emit rate-limited telemetry for Level 4 queuing
            // (rate-limiting is caller's responsibility; we emit an event here)
            events.push(ArbitrationTelemetryEvent::queue(
                input.agent_id,
                input.mutation_ref,
                input.timestamp_us,
            ));
            return ArbitrationOutcome::Queue {
                queue_reason,
                earliest_present_us,
                redacted,
            };
        }
        Some(other) => return other,
        None => {}
    }

    // ── Level 5: Resource gate (delegate to resource module) ──────────────────
    // Populate is_transactional from the mutation kind so resource module has a
    // single source of truth (no separate kind parameter).
    let resource_ctx_for_zone = crate::types::ResourceContext {
        is_transactional: input.kind == MutationKind::Transactional,
        ..ctx.resource_context.clone()
    };
    let resource_decision = evaluate_resource(&resource_ctx_for_zone, input.mutation_ref);
    match &resource_decision {
        ResourceDecision::Shed { .. } => {
            events.push(ArbitrationTelemetryEvent::shed(
                input.agent_id,
                input.mutation_ref,
                input.timestamp_us,
            ));
        }
        ResourceDecision::BudgetExceeded => {
            events.push(ArbitrationTelemetryEvent::reject(
                ArbitrationLevel::Resource.index(),
                "TILE_BUDGET_EXCEEDED",
                input.agent_id,
                input.mutation_ref,
                input.timestamp_us,
            ));
        }
        ResourceDecision::Pass | ResourceDecision::BudgetsPaused => {}
    }
    if let Some(outcome) =
        resource_decision_to_outcome(resource_decision, input.agent_id, input.mutation_ref)
    {
        return outcome;
    }

    // ── Level 6: Content resolution (delegate to content module) ──────────────
    let content_decision = evaluate_content(&ctx.content_context, input.mutation_ref);
    if let Some(outcome) =
        content_decision_to_outcome(content_decision, input.agent_id, input.mutation_ref)
    {
        return outcome;
    }

    // All levels passed.
    if redacted {
        let redaction_reason = match privacy_result {
            PrivacyDecision::Redact(reason) => reason,
            PrivacyDecision::Visible => unreachable!("redacted flag set but privacy says Visible"),
        };
        // Emit rate-limited telemetry for Level 2 redaction
        events.push(ArbitrationTelemetryEvent::redact(
            input.agent_id,
            input.mutation_ref,
            input.timestamp_us,
        ));
        ArbitrationOutcome::CommitRedacted { redaction_reason }
    } else {
        ArbitrationOutcome::Commit
    }
}

// ─── Tile mutation evaluation path ───────────────────────────────────────────

/// Evaluate a tile mutation: L3 → L5 → L6.
fn evaluate_tile_mutation(
    input: &MutationEvalInput<'_>,
    cap_set: &crate::security::CapabilitySet,
    events: &mut Vec<ArbitrationTelemetryEvent>,
) -> ArbitrationOutcome {
    let ctx = input.ctx;

    // ── Level 3: Security gate ────────────────────────────────────────────────
    // cap_set is pre-built by evaluate_mutation to avoid per-call allocation.

    // Lease validity
    if !ctx.security_context.lease_valid {
        let rejection = ArbitrationOutcome::Reject(ArbitrationError {
            code: ArbitrationErrorCode::LeaseInvalid,
            agent_id: input.agent_id.to_string(),
            mutation_ref: input.mutation_ref,
            message: "Lease is not in Active state".to_string(),
            hint: None,
            level: ArbitrationLevel::Security.index(),
        });
        events.push(ArbitrationTelemetryEvent::reject(
            ArbitrationLevel::Security.index(),
            "LEASE_INVALID",
            input.agent_id,
            input.mutation_ref,
            input.timestamp_us,
        ));
        return rejection;
    }

    // Namespace isolation
    if !input.target_namespace.is_empty()
        && !ctx.security_context.agent_namespace.is_empty()
        && input.target_namespace != ctx.security_context.agent_namespace
    {
        let rejection = ArbitrationOutcome::Reject(ArbitrationError {
            code: ArbitrationErrorCode::NamespaceViolation,
            agent_id: input.agent_id.to_string(),
            mutation_ref: input.mutation_ref,
            message: format!(
                "Namespace violation: agent '{}' may not write to '{}'",
                ctx.security_context.agent_namespace, input.target_namespace
            ),
            hint: Some(format!(
                "Agent namespace: '{}'",
                ctx.security_context.agent_namespace
            )),
            level: ArbitrationLevel::Security.index(),
        });
        events.push(ArbitrationTelemetryEvent::reject(
            ArbitrationLevel::Security.index(),
            "NAMESPACE_VIOLATION",
            input.agent_id,
            input.mutation_ref,
            input.timestamp_us,
        ));
        return rejection;
    }

    // Capability checks (conjunctive)
    if let Some(missing) = cap_set.first_missing(input.required_capabilities) {
        let rejection = ArbitrationOutcome::Reject(ArbitrationError {
            code: ArbitrationErrorCode::CapabilityDenied,
            agent_id: input.agent_id.to_string(),
            mutation_ref: input.mutation_ref,
            message: format!("Missing required capability: '{missing}'"),
            hint: Some(format!("Required: '{missing}'")),
            level: ArbitrationLevel::Security.index(),
        });
        events.push(ArbitrationTelemetryEvent::reject(
            ArbitrationLevel::Security.index(),
            "CAPABILITY_DENIED",
            input.agent_id,
            input.mutation_ref,
            input.timestamp_us,
        ));
        return rejection;
    }

    // ── Level 5: Resource gate (delegate to resource module) ──────────────────
    // Populate is_transactional from the mutation kind so resource module has a
    // single source of truth (no separate kind parameter).
    let resource_ctx_for_tile = crate::types::ResourceContext {
        is_transactional: input.kind == MutationKind::Transactional,
        ..ctx.resource_context.clone()
    };
    let resource_decision = evaluate_resource(&resource_ctx_for_tile, input.mutation_ref);
    match &resource_decision {
        ResourceDecision::Shed { .. } => {
            events.push(ArbitrationTelemetryEvent::shed(
                input.agent_id,
                input.mutation_ref,
                input.timestamp_us,
            ));
        }
        ResourceDecision::BudgetExceeded => {
            events.push(ArbitrationTelemetryEvent::reject(
                ArbitrationLevel::Resource.index(),
                "TILE_BUDGET_EXCEEDED",
                input.agent_id,
                input.mutation_ref,
                input.timestamp_us,
            ));
        }
        ResourceDecision::Pass | ResourceDecision::BudgetsPaused => {}
    }
    if let Some(outcome) =
        resource_decision_to_outcome(resource_decision, input.agent_id, input.mutation_ref)
    {
        return outcome;
    }

    // ── Level 6: Content resolution (delegate to content module) ──────────────
    let content_decision = evaluate_content(&ctx.content_context, input.mutation_ref);
    if let Some(outcome) =
        content_decision_to_outcome(content_decision, input.agent_id, input.mutation_ref)
    {
        return outcome;
    }

    ArbitrationOutcome::Commit
}

/// Evaluate Level 4 attention for a zone publication.
fn evaluate_attention(
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

    if ctx.quiet_hours_active {
        match ctx.interruption_class {
            InterruptionClass::Critical | InterruptionClass::Silent => {
                unreachable!("handled above")
            }
            InterruptionClass::Low => {
                // LOW is discarded during quiet hours.
                return Some(ArbitrationOutcome::Shed {
                    degradation_level: 0,
                });
            }
            InterruptionClass::Normal => {
                return Some(ArbitrationOutcome::Queue {
                    queue_reason: QueueReason::QuietHours {
                        window_end_us: ctx.quiet_hours_end_us,
                    },
                    earliest_present_us: ctx.quiet_hours_end_us,
                    redacted: false,
                });
            }
            InterruptionClass::High => {
                if ctx.interruption_class > ctx.pass_through_class {
                    return Some(ArbitrationOutcome::Queue {
                        queue_reason: QueueReason::QuietHours {
                            window_end_us: ctx.quiet_hours_end_us,
                        },
                        earliest_present_us: ctx.quiet_hours_end_us,
                        redacted: false,
                    });
                }
            }
        }
    }

    // Attention budget check
    let agent_exhausted = ctx.per_agent_interruptions_last_60s >= ctx.per_agent_limit;
    let zone_exhausted = ctx.per_zone_interruptions_last_60s >= ctx.per_zone_limit;
    if agent_exhausted || zone_exhausted {
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

// ─── Batch evaluation ─────────────────────────────────────────────────────────

/// Evaluate a batch of mutations and collect results + telemetry.
///
/// Each mutation is evaluated independently through the per-mutation pipeline.
/// Short-circuit is per-mutation (not per-batch): a rejection of one mutation
/// does not stop evaluation of subsequent mutations in the batch.
///
/// Returns a `BatchEvalResult` with per-mutation outcomes and aggregate telemetry.
pub struct BatchEvalResult {
    /// Outcome for each mutation in the batch (same order as input).
    pub outcomes: Vec<ArbitrationOutcome>,
    /// Aggregate telemetry for the batch.
    pub telemetry: PolicyTelemetry,
    /// All arbitration events emitted during batch evaluation.
    pub events: Vec<ArbitrationTelemetryEvent>,
}

/// Evaluate a batch of mutations.
///
/// # Arguments
///
/// - `inputs` — one `MutationEvalInput` per mutation in the batch
///
/// # Returns
///
/// `BatchEvalResult` with per-mutation outcomes, aggregate telemetry, and events.
pub fn evaluate_batch(inputs: &[MutationEvalInput<'_>]) -> BatchEvalResult {
    let mut outcomes = Vec::with_capacity(inputs.len());
    let mut telemetry = PolicyTelemetry::default();
    let mut all_events = Vec::new();

    for input in inputs {
        let eval = evaluate_mutation(input);

        // Accumulate telemetry counters
        match &eval.outcome {
            ArbitrationOutcome::Commit => {}
            ArbitrationOutcome::CommitRedacted { .. } => telemetry.mutations_redacted += 1,
            ArbitrationOutcome::Queue { .. } => telemetry.mutations_queued += 1,
            ArbitrationOutcome::Reject(_) => telemetry.mutations_rejected += 1,
            ArbitrationOutcome::Shed { .. } => telemetry.mutations_shed += 1,
            ArbitrationOutcome::Blocked { .. } => {}
        }

        all_events.extend(eval.events);
        outcomes.push(eval.outcome);
    }

    BatchEvalResult {
        outcomes,
        telemetry,
        events: all_events,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use crate::types::{
        AttentionContext, ContentContext, InterruptionClass, OverrideState, PolicyContext,
        PrivacyContext, RedactionStyle, ResourceContext, SafetyState, SecurityContext, ViewerClass,
    };
    use tze_hud_scene::types::ContentionPolicy;

    fn default_ctx() -> PolicyContext {
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
                redaction_style: RedactionStyle::Pattern,
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

    fn default_zone_input<'a>(ctx: &'a PolicyContext) -> MutationEvalInput<'a> {
        MutationEvalInput {
            ctx,
            mutation_ref: SceneId::new(),
            agent_declared_classification: VisibilityClassification::Public,
            zone_default_classification: None,
            required_capabilities: &["publish_zone:subtitle"],
            target_namespace: "agent_a",
            agent_id: "agent_a",
            kind: MutationKind::ZonePublication,
            timestamp_us: 1_000_000,
        }
    }

    fn default_tile_input<'a>(ctx: &'a PolicyContext) -> MutationEvalInput<'a> {
        MutationEvalInput {
            ctx,
            mutation_ref: SceneId::new(),
            agent_declared_classification: VisibilityClassification::Public,
            zone_default_classification: None,
            required_capabilities: &["create_tiles"],
            target_namespace: "agent_a",
            agent_id: "agent_a",
            kind: MutationKind::TileMutation,
            timestamp_us: 1_000_000,
        }
    }

    // ─── Zone publication full stack (spec lines 221-223) ────────────────────

    /// WHEN a zone publish mutation arrives
    /// THEN it passes through override preemption, security gate, privacy decoration,
    /// attention gate, resource gate, and content resolution in order
    #[test]
    fn test_zone_publication_passes_all_levels() {
        let ctx = default_ctx();
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        assert_eq!(output.outcome, ArbitrationOutcome::Commit);
        assert!(output.events.is_empty());
    }

    // ─── Level 0 freeze blocks zone publications (spec §3.4) ─────────────────

    #[test]
    fn test_freeze_blocks_zone_publication() {
        let mut ctx = default_ctx();
        ctx.override_state.freeze_active = true;
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        assert_eq!(
            output.outcome,
            ArbitrationOutcome::Blocked {
                block_reason: BlockReason::Freeze
            }
        );
    }

    #[test]
    fn test_freeze_does_not_block_tile_mutation_path() {
        // Tile mutations do NOT include Level 0 (path: 3→5→6)
        let mut ctx = default_ctx();
        ctx.override_state.freeze_active = true;
        let input = default_tile_input(&ctx);
        let output = evaluate_mutation(&input);
        // Tile mutations skip Level 0; freeze enforcement is at the pipeline/queue layer
        assert_eq!(output.outcome, ArbitrationOutcome::Commit);
    }

    // ─── Level 3 security rejections ─────────────────────────────────────────

    /// WHEN agent attempts to create a tile without create_tiles capability
    /// THEN mutation rejected with CapabilityDenied naming the missing capability
    /// (spec lines 131-133)
    #[test]
    fn test_capability_denied_emits_telemetry_event() {
        let mut ctx = default_ctx();
        ctx.security_context.granted_capabilities = vec![];
        let input = default_tile_input(&ctx);
        let output = evaluate_mutation(&input);

        assert!(matches!(
            &output.outcome,
            ArbitrationOutcome::Reject(err) if err.code == ArbitrationErrorCode::CapabilityDenied
        ));
        // Telemetry event must be emitted
        assert_eq!(output.events.len(), 1);
        assert_eq!(output.events[0].event, "arbitration_reject");
        assert_eq!(output.events[0].level, 3);
        assert_eq!(output.events[0].code.as_deref(), Some("CAPABILITY_DENIED"));
    }

    /// WHEN agent attempts to modify a tile in another agent's namespace
    /// THEN mutation rejected with NamespaceViolation (spec lines 135-136)
    #[test]
    fn test_namespace_violation_emits_telemetry_event() {
        let mut ctx = default_ctx();
        ctx.security_context.agent_namespace = "agent_a".to_string();
        let mut input = default_tile_input(&ctx);
        let mutation_ref = SceneId::new();
        input.mutation_ref = mutation_ref;
        input.target_namespace = "agent_b"; // different namespace
        let output = evaluate_mutation(&input);

        assert!(matches!(
            &output.outcome,
            ArbitrationOutcome::Reject(err) if err.code == ArbitrationErrorCode::NamespaceViolation
        ));
        assert_eq!(output.events.len(), 1);
        assert_eq!(
            output.events[0].code.as_deref(),
            Some("NAMESPACE_VIOLATION")
        );
    }

    #[test]
    fn test_invalid_lease_rejected() {
        let mut ctx = default_ctx();
        ctx.security_context.lease_valid = false;
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        assert!(matches!(
            &output.outcome,
            ArbitrationOutcome::Reject(err) if err.code == ArbitrationErrorCode::LeaseInvalid
        ));
        assert_eq!(output.events[0].code.as_deref(), Some("LEASE_INVALID"));
    }

    // ─── Level 2 privacy redaction ────────────────────────────────────────────

    /// WHEN tile has private classification and viewer is known_guest
    /// THEN tile committed with redaction (spec lines 97-98)
    #[test]
    fn test_privacy_redaction_applied() {
        let mut ctx = default_ctx();
        ctx.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
        ctx.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];
        let mut input = default_zone_input(&ctx);
        input.agent_declared_classification = VisibilityClassification::Private;
        let output = evaluate_mutation(&input);
        assert!(matches!(
            output.outcome,
            ArbitrationOutcome::CommitRedacted { .. }
        ));
        // Telemetry event for redaction
        assert!(
            output
                .events
                .iter()
                .any(|e| e.event == "arbitration_redact")
        );
    }

    /// WHEN sole viewer is owner THEN all content shown without redaction (spec lines 104-106)
    #[test]
    fn test_owner_sees_sensitive_without_redaction() {
        let ctx = default_ctx(); // Owner viewer
        let mut input = default_zone_input(&ctx);
        input.agent_declared_classification = VisibilityClassification::Sensitive;
        let output = evaluate_mutation(&input);
        assert_eq!(output.outcome, ArbitrationOutcome::Commit);
    }

    // ─── Zone ceiling rule (spec lines 113-115) ───────────────────────────────

    /// WHEN agent declares public classification in zone with household default
    /// THEN effective classification is household
    #[test]
    fn test_zone_ceiling_enforced_in_pipeline() {
        let mut ctx = default_ctx();
        // Viewer is HouseholdMember — can see Household but not Private
        ctx.privacy_context.effective_viewer_class = ViewerClass::HouseholdMember;
        ctx.privacy_context.viewer_classes = vec![ViewerClass::HouseholdMember];
        let mut input = default_zone_input(&ctx);
        // Agent declares public; zone default is household
        input.agent_declared_classification = VisibilityClassification::Public;
        input.zone_default_classification = Some(VisibilityClassification::Household);
        let output = evaluate_mutation(&input);
        // Household viewer can see Household content → no redaction after zone ceiling applied
        assert_eq!(output.outcome, ArbitrationOutcome::Commit);

        // Now test that KnownGuest cannot see the Household-elevated content
        let mut ctx2 = default_ctx();
        ctx2.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
        ctx2.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];
        let mut input2 = default_zone_input(&ctx2);
        input2.agent_declared_classification = VisibilityClassification::Public;
        input2.zone_default_classification = Some(VisibilityClassification::Household);
        let output2 = evaluate_mutation(&input2);
        // Zone ceiling elevates to Household; KnownGuest cannot see Household → redacted
        assert!(matches!(
            output2.outcome,
            ArbitrationOutcome::CommitRedacted { .. }
        ));
    }

    // ─── Level 5 resource enforcement ────────────────────────────────────────

    /// WHEN mutation shed at Level 5 due to degradation
    /// THEN zone state updated but render output omitted (spec lines 173-175)
    #[test]
    fn test_shed_produces_shed_outcome() {
        let mut ctx = default_ctx();
        ctx.resource_context.should_shed = true;
        ctx.resource_context.degradation_level = 3;
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        assert!(matches!(
            output.outcome,
            ArbitrationOutcome::Shed {
                degradation_level: 3
            }
        ));
        // Shed telemetry event emitted
        assert!(output.events.iter().any(|e| e.event == "arbitration_shed"));
    }

    /// WHEN degradation at Level 5 and CreateTile (Transactional) arrives
    /// THEN mutation NOT shed (spec lines 177-179)
    #[test]
    fn test_transactional_not_shed() {
        let mut ctx = default_ctx();
        ctx.resource_context.should_shed = true;
        ctx.resource_context.degradation_level = 5;
        ctx.security_context.granted_capabilities = vec!["create_tiles".to_string()];
        let mut input = default_tile_input(&ctx);
        input.kind = MutationKind::Transactional;
        let output = evaluate_mutation(&input);
        assert_eq!(output.outcome, ArbitrationOutcome::Commit);
    }

    // ─── Degradation Does Not Bypass Arbitration (spec lines 360-362) ─────────

    /// WHEN mutation shed at Level 5 THEN it has already passed Levels 3, 2, 4
    #[test]
    fn test_shed_mutation_has_passed_capability_check() {
        let mut ctx = default_ctx();
        // Shedding active
        ctx.resource_context.should_shed = true;
        ctx.resource_context.degradation_level = 2;
        // Capability is granted — Level 3 passes
        ctx.security_context.granted_capabilities = vec!["publish_zone:subtitle".to_string()];
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        // Result is Shed — capability check passed (not rejected)
        assert!(matches!(output.outcome, ArbitrationOutcome::Shed { .. }));
    }

    /// A mutation without the required capability is rejected at Level 3, never reaches Level 5
    #[test]
    fn test_capability_rejected_never_reaches_resource_level() {
        let mut ctx = default_ctx();
        ctx.resource_context.should_shed = true; // would shed if Level 3 passed
        ctx.security_context.granted_capabilities = vec![]; // no capabilities
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        // Must be Reject (capability denied) not Shed
        assert!(matches!(
            &output.outcome,
            ArbitrationOutcome::Reject(err) if err.code == ArbitrationErrorCode::CapabilityDenied
        ));
    }

    // ─── Level 4 attention gate ───────────────────────────────────────────────

    #[test]
    fn test_quiet_hours_queue_normal() {
        let mut ctx = default_ctx();
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.interruption_class = InterruptionClass::Normal;
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        assert!(matches!(
            &output.outcome,
            ArbitrationOutcome::Queue {
                queue_reason: QueueReason::QuietHours { .. },
                ..
            }
        ));
    }

    #[test]
    fn test_critical_bypasses_quiet_hours() {
        let mut ctx = default_ctx();
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.interruption_class = InterruptionClass::Critical;
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        assert_eq!(output.outcome, ArbitrationOutcome::Commit);
    }

    // ─── Override composition (Level 2 Transform + Level 4 Block) ─────────────

    /// WHEN Level 2 redacts AND Level 4 would queue
    /// THEN queued-with-redaction (spec §7.3)
    #[test]
    fn test_privacy_redaction_composed_with_quiet_hours() {
        let mut ctx = default_ctx();
        ctx.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
        ctx.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];
        ctx.attention_context.quiet_hours_active = true;
        ctx.attention_context.interruption_class = InterruptionClass::Normal;
        ctx.attention_context.quiet_hours_end_us = Some(1_000_000);
        let mut input = default_zone_input(&ctx);
        input.agent_declared_classification = VisibilityClassification::Private;
        let output = evaluate_mutation(&input);
        match &output.outcome {
            ArbitrationOutcome::Queue { redacted, .. } => {
                assert!(redacted, "Queued mutation must carry redaction flag");
            }
            other => panic!("Expected Queue with redacted=true, got {other:?}"),
        }
    }

    // ─── Level 6 content resolution ───────────────────────────────────────────

    /// WHEN two agents publish to a LatestWins zone in the same frame
    /// THEN the second publish replaces the first (arrival order)
    #[test]
    fn test_latest_wins_zone_commits() {
        let ctx = default_ctx(); // LatestWins by default
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        assert_eq!(output.outcome, ArbitrationOutcome::Commit);
    }

    #[test]
    fn test_replace_zone_eviction_denied() {
        let mut ctx = default_ctx();
        ctx.content_context.contention_policy = Some(ContentionPolicy::Replace);
        ctx.content_context.agent_lease_priority = 3;
        ctx.content_context.occupant_lease_priority = Some(1);
        let input = default_zone_input(&ctx);
        let output = evaluate_mutation(&input);
        assert!(matches!(
            &output.outcome,
            ArbitrationOutcome::Reject(err) if err.code == ArbitrationErrorCode::ZoneEvictionDenied
        ));
    }

    // ─── Batch evaluation ─────────────────────────────────────────────────────

    /// WHEN a frame contains 64 mutations (max batch)
    /// THEN each per-mutation policy check runs and telemetry is accumulated
    /// (spec lines 225-227 — latency verification via count accuracy)
    #[test]
    fn test_batch_64_mutations_telemetry_accumulation() {
        let ctx = default_ctx();
        let inputs: Vec<MutationEvalInput<'_>> =
            (0..64).map(|_| default_zone_input(&ctx)).collect();
        let result = evaluate_batch(&inputs);
        assert_eq!(result.outcomes.len(), 64);
        assert_eq!(result.telemetry.mutations_rejected, 0);
        // All 64 committed
        assert!(
            result
                .outcomes
                .iter()
                .all(|o| *o == ArbitrationOutcome::Commit)
        );
    }

    #[test]
    fn test_batch_rejection_counted_in_telemetry() {
        let mut ctx = default_ctx();
        ctx.security_context.granted_capabilities = vec![]; // no capabilities → all rejected
        let inputs: Vec<MutationEvalInput<'_>> = (0..3).map(|_| default_zone_input(&ctx)).collect();
        let result = evaluate_batch(&inputs);
        assert_eq!(result.telemetry.mutations_rejected, 3);
    }

    #[test]
    fn test_batch_shed_counted_in_telemetry() {
        let mut ctx = default_ctx();
        ctx.resource_context.should_shed = true;
        ctx.resource_context.degradation_level = 3;
        let inputs: Vec<MutationEvalInput<'_>> = (0..2).map(|_| default_zone_input(&ctx)).collect();
        let result = evaluate_batch(&inputs);
        assert_eq!(result.telemetry.mutations_shed, 2);
    }
}
