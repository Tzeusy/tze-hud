//! # Per-Event Evaluation Pipeline
//!
//! Implements the per-event arbitration evaluation specified in
//! policy-arbitration/spec.md (lines 207-214).
//!
//! ## Evaluation Order
//!
//! During input drain (Stage 1) and local feedback (Stage 2):
//!
//! ```text
//! Level 0 (Human Override) → Level 4 (Attention) → Level 3 (Security)
//! ```
//!
//! If Level 0 triggers safe mode, Levels 4 and 3 MUST stop (short-circuit).
//!
//! ## Override Takes Absolute Priority
//!
//! Override commands (dismiss, safe mode, freeze, mute) are **local, instant, and
//! cannot be intercepted** (spec §1.1, §11.1). They are processed before any
//! `MutationBatch` intake. The per-event pipeline enforces this ordering.
//!
//! ## Purity Constraint
//!
//! `evaluate_event` is a pure function over typed inputs. It produces an
//! `EventEvaluation` describing the evaluation result. The compositor reads
//! `EventEvaluation` and acts accordingly.

use crate::override_queue::OverrideCommand;
use crate::types::{
    ArbitrationError, ArbitrationErrorCode, ArbitrationLevel, ArbitrationOutcome, AttentionContext,
    BlockReason, InterruptionClass, QueueReason, SecurityContext,
};
use tze_hud_scene::SceneId;

// ─── Event evaluation output ──────────────────────────────────────────────────

/// Result of evaluating the per-event pipeline for a single input event.
///
/// This is a **pure output** — no side effects.
#[derive(Clone, Debug)]
pub struct EventEvaluation {
    /// The arbitration outcome for this event.
    pub outcome: EventOutcome,

    /// Which levels were evaluated.
    pub levels_evaluated: Vec<ArbitrationLevel>,
}

/// Outcome of per-event evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventOutcome {
    /// Event accepted and delivered to the local feedback loop.
    Accept,

    /// Event discarded because a higher-priority override (e.g., safe mode) preempted it.
    ///
    /// Triggered when Level 0 enters safe mode during the same frame. The event
    /// is discarded, not queued (spec line 213-214: "override is processed first and
    /// tab-switch event is discarded").
    Discarded { reason: DiscardReason },

    /// Event queued by Level 4 (Attention).
    Queued {
        queue_reason: QueueReason,
        earliest_present_us: Option<u64>,
    },

    /// Event rejected by Level 3 (Security).
    Rejected(ArbitrationError),

    /// Event blocked by Level 0 freeze.
    Blocked { block_reason: BlockReason },
}

/// Why an event was discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardReason {
    /// Safe mode was activated by Level 0; the event was preempted.
    SafeModePreempted,
    /// LOW-priority event was shed during quiet hours (spec RFC 0010 §3.1).
    ///
    /// LOW content (background sync, telemetry, status refreshes) is discarded
    /// during quiet hours because it is too stale by quiet hours exit to be useful.
    QuietHoursShed,
}

// ─── Per-event input context ──────────────────────────────────────────────────

/// Context for evaluating a single input event through the per-event pipeline.
#[derive(Clone, Debug)]
pub struct EventContext {
    /// The override command (if any) that arrived in the same frame as this event.
    ///
    /// If `Some(OverrideCommand::SafeMode)`, this event is discarded after Level 0.
    pub override_command: Option<OverrideCommand>,

    /// Whether the scene is currently frozen (Level 0 freeze check).
    pub freeze_active: bool,

    /// Whether safe mode is currently active (used to discard incoming events).
    pub safe_mode_active: bool,

    /// Attention context for Level 4 evaluation.
    pub attention_context: AttentionContext,

    /// Security context for Level 3 evaluation.
    pub security_context: SecurityContext,

    /// Required capabilities for this event (checked at Level 3).
    pub required_capabilities: Vec<String>,

    /// Target namespace for this event (checked at Level 3 namespace isolation).
    pub target_namespace: String,

    /// A unique ID for this event (used in structured error reporting).
    pub event_ref: SceneId,
}

// ─── Per-event evaluation (pure) ─────────────────────────────────────────────

/// Evaluate the per-event arbitration pipeline for a single input event.
///
/// ## Evaluation order
///
/// `Level 0 (Human Override) → Level 4 (Attention) → Level 3 (Security)`
///
/// Short-circuit: if Level 0 triggers safe mode (via `OverrideCommand::SafeMode`),
/// Levels 4 and 3 are NOT evaluated and the event is discarded.
///
/// ## Arguments
///
/// - `ectx` — per-event context (override state, attention, security).
///
/// ## Returns
///
/// An `EventEvaluation` describing the outcome.
///
/// # Pure function contract
///
/// No side effects. The caller executes state transitions based on the returned
/// `EventEvaluation`.
pub fn evaluate_event(ectx: &EventContext) -> EventEvaluation {
    let mut levels_evaluated = Vec::with_capacity(3);

    // ─── Level 0: Human Override ─────────────────────────────────────────────
    levels_evaluated.push(ArbitrationLevel::HumanOverride);

    // If safe mode was activated this frame (by an override command), discard
    // all non-override events. This models the spec scenario at lines 212-214:
    // "Ctrl+Shift+Escape and tab-switch arrive in same frame → override processed
    //  first, tab-switch discarded."
    if let Some(OverrideCommand::SafeMode) = &ectx.override_command {
        return EventEvaluation {
            outcome: EventOutcome::Discarded {
                reason: DiscardReason::SafeModePreempted,
            },
            levels_evaluated,
        };
    }

    // If safe mode is already active, discard incoming agent events.
    if ectx.safe_mode_active {
        return EventEvaluation {
            outcome: EventOutcome::Discarded {
                reason: DiscardReason::SafeModePreempted,
            },
            levels_evaluated,
        };
    }

    // Freeze check: agent mutations are blocked (not rejected) during freeze.
    if ectx.freeze_active {
        return EventEvaluation {
            outcome: EventOutcome::Blocked {
                block_reason: BlockReason::Freeze,
            },
            levels_evaluated,
        };
    }

    // ─── Level 4: Attention ──────────────────────────────────────────────────
    levels_evaluated.push(ArbitrationLevel::Attention);

    if let Some(attention_outcome) =
        evaluate_event_level4_attention(&ectx.attention_context, ectx.event_ref)
    {
        return EventEvaluation {
            outcome: attention_outcome,
            levels_evaluated,
        };
    }

    // ─── Level 3: Security ───────────────────────────────────────────────────
    levels_evaluated.push(ArbitrationLevel::Security);

    if let Some(reject_outcome) = evaluate_event_level3_security(
        &ectx.security_context,
        &ectx.required_capabilities,
        &ectx.target_namespace,
        ectx.event_ref,
    ) {
        return EventEvaluation {
            outcome: reject_outcome,
            levels_evaluated,
        };
    }

    EventEvaluation {
        outcome: EventOutcome::Accept,
        levels_evaluated,
    }
}

// ─── Per-level event evaluators ───────────────────────────────────────────────

/// Level 4 (Attention) per-event evaluation.
///
/// Returns `Some(EventOutcome::Queued)` if the event should be deferred.
/// Returns `None` if the event passes.
fn evaluate_event_level4_attention(
    ctx: &AttentionContext,
    _event_ref: SceneId,
) -> Option<EventOutcome> {
    // CRITICAL bypasses everything.
    if ctx.interruption_class == InterruptionClass::Critical {
        return None;
    }

    // SILENT always passes with zero budget cost.
    if ctx.interruption_class == InterruptionClass::Silent {
        return None;
    }

    // Quiet hours gate.
    if ctx.quiet_hours_active {
        match ctx.interruption_class {
            InterruptionClass::Critical | InterruptionClass::Silent => {
                unreachable!("handled above")
            }
            InterruptionClass::Low => {
                // LOW is discarded during quiet hours, not queued.
                // Spec RFC 0010 §3.1: LOW content (background sync, telemetry, status refreshes)
                // is too stale by quiet hours exit to be useful.
                return Some(EventOutcome::Discarded {
                    reason: DiscardReason::QuietHoursShed,
                });
            }
            InterruptionClass::Normal => {
                return Some(EventOutcome::Queued {
                    queue_reason: QueueReason::QuietHours {
                        window_end_us: ctx.quiet_hours_end_us,
                    },
                    earliest_present_us: ctx.quiet_hours_end_us,
                });
            }
            InterruptionClass::High => {
                // HIGH passes unless pass_through_class is set higher.
                if ctx.interruption_class > ctx.pass_through_class {
                    return Some(EventOutcome::Queued {
                        queue_reason: QueueReason::QuietHours {
                            window_end_us: ctx.quiet_hours_end_us,
                        },
                        earliest_present_us: ctx.quiet_hours_end_us,
                    });
                }
            }
        }
    }

    // Attention budget check.
    let agent_exhausted = ctx.agent_budget_exhausted();
    let zone_exhausted = ctx.zone_budget_exhausted();
    if agent_exhausted || zone_exhausted {
        return Some(EventOutcome::Queued {
            queue_reason: QueueReason::AttentionBudgetExhausted {
                per_agent: agent_exhausted,
                per_zone: zone_exhausted,
            },
            earliest_present_us: ctx.budget_refill_us,
        });
    }

    None
}

/// Level 3 (Security) per-event evaluation.
///
/// Returns `Some(EventOutcome::Rejected(...))` if any security check fails.
/// Returns `None` if all checks pass.
fn evaluate_event_level3_security(
    ctx: &SecurityContext,
    required_capabilities: &[String],
    target_namespace: &str,
    event_ref: SceneId,
) -> Option<EventOutcome> {
    // Lease validity check.
    if !ctx.lease_valid {
        return Some(EventOutcome::Rejected(ArbitrationError {
            code: ArbitrationErrorCode::LeaseInvalid,
            agent_id: ctx.agent_namespace.clone(),
            mutation_ref: event_ref,
            message: "Lease is not in Active state".to_string(),
            hint: None,
            level: ArbitrationLevel::Security.index(),
        }));
    }

    // Namespace isolation check.
    if !target_namespace.is_empty()
        && !ctx.agent_namespace.is_empty()
        && target_namespace != ctx.agent_namespace
    {
        return Some(EventOutcome::Rejected(ArbitrationError {
            code: ArbitrationErrorCode::NamespaceViolation,
            agent_id: ctx.agent_namespace.clone(),
            mutation_ref: event_ref,
            message: format!(
                "Namespace violation: agent '{}' may not write to '{}'",
                ctx.agent_namespace, target_namespace
            ),
            hint: Some(format!("Agent namespace: '{}'", ctx.agent_namespace)),
            level: ArbitrationLevel::Security.index(),
        }));
    }

    // Capability checks (conjunctive — all must pass).
    for cap in required_capabilities {
        if !ctx.has_capability(cap) {
            return Some(EventOutcome::Rejected(ArbitrationError {
                code: ArbitrationErrorCode::CapabilityDenied,
                agent_id: ctx.agent_namespace.clone(),
                mutation_ref: event_ref,
                message: format!("Missing required capability: '{cap}'"),
                hint: Some(format!("Required: '{cap}'")),
                level: ArbitrationLevel::Security.index(),
            }));
        }
    }

    None
}

// ─── Override-preemption helper ───────────────────────────────────────────────

/// Process a batch of override commands from the `OverrideCommandQueue` and determine
/// whether safe mode was activated.
///
/// Returns `true` if any of the commands is `OverrideCommand::SafeMode`.
///
/// The compositor calls this before constructing `EventContext` for each event in the
/// current frame, ensuring override commands are processed first (spec §11.1).
pub fn safe_mode_activated_in_batch(commands: &[OverrideCommand]) -> bool {
    commands.contains(&OverrideCommand::SafeMode)
}

// ─── Conversion helpers ───────────────────────────────────────────────────────

impl From<EventOutcome> for ArbitrationOutcome {
    fn from(o: EventOutcome) -> ArbitrationOutcome {
        match o {
            EventOutcome::Accept => ArbitrationOutcome::Commit,
            EventOutcome::Discarded { .. } => ArbitrationOutcome::Shed {
                degradation_level: 0,
            },
            EventOutcome::Queued {
                queue_reason,
                earliest_present_us,
            } => ArbitrationOutcome::Queue {
                queue_reason,
                earliest_present_us,
                redacted: false,
            },
            EventOutcome::Rejected(err) => ArbitrationOutcome::Reject(err),
            EventOutcome::Blocked { block_reason } => ArbitrationOutcome::Blocked { block_reason },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::override_queue::OverrideCommand;
    use crate::types::{AttentionContext, InterruptionClass, SecurityContext};
    use tze_hud_scene::SceneId;

    fn default_attention() -> AttentionContext {
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

    fn default_security() -> SecurityContext {
        SecurityContext {
            granted_capabilities: vec!["access_input_events".to_string()],
            agent_namespace: "agent_a".to_string(),
            lease_valid: true,
            lease_id: Some(SceneId::new()),
        }
    }

    fn default_event_ctx() -> EventContext {
        EventContext {
            override_command: None,
            freeze_active: false,
            safe_mode_active: false,
            attention_context: default_attention(),
            security_context: default_security(),
            required_capabilities: vec!["access_input_events".to_string()],
            target_namespace: "agent_a".to_string(),
            event_ref: SceneId::new(),
        }
    }

    // ─── Nominal path ─────────────────────────────────────────────────────────

    #[test]
    fn test_nominal_event_accepted_evaluates_l0_l4_l3() {
        let ectx = default_event_ctx();
        let result = evaluate_event(&ectx);

        assert_eq!(result.outcome, EventOutcome::Accept);

        // All three levels evaluated
        assert_eq!(result.levels_evaluated.len(), 3);
        assert_eq!(result.levels_evaluated[0], ArbitrationLevel::HumanOverride);
        assert_eq!(result.levels_evaluated[1], ArbitrationLevel::Attention);
        assert_eq!(result.levels_evaluated[2], ArbitrationLevel::Security);
    }

    // ─── Per-event evaluation order: L0 → L4 → L3 ───────────────────────────

    #[test]
    fn test_per_event_evaluation_order_is_0_4_3() {
        let ectx = default_event_ctx();
        let result = evaluate_event(&ectx);

        let order: Vec<u8> = result.levels_evaluated.iter().map(|l| l.index()).collect();
        assert_eq!(order, vec![0, 4, 3]);
    }

    // ─── Level 0 short-circuit: SafeMode ─────────────────────────────────────

    /// WHEN a Ctrl+Shift+Escape input and a tab-switch event arrive in the same frame
    /// THEN the override (safe mode) is processed first and the tab-switch event is discarded
    /// (spec lines 212-214)
    #[test]
    fn test_safe_mode_override_discards_event() {
        let mut ectx = default_event_ctx();
        ectx.override_command = Some(OverrideCommand::SafeMode);

        let result = evaluate_event(&ectx);

        assert_eq!(
            result.outcome,
            EventOutcome::Discarded {
                reason: DiscardReason::SafeModePreempted
            }
        );
        // Only Level 0 evaluated
        assert_eq!(result.levels_evaluated.len(), 1);
        assert_eq!(result.levels_evaluated[0], ArbitrationLevel::HumanOverride);
    }

    #[test]
    fn test_safe_mode_active_discards_new_events() {
        let mut ectx = default_event_ctx();
        ectx.safe_mode_active = true;

        let result = evaluate_event(&ectx);

        assert_eq!(
            result.outcome,
            EventOutcome::Discarded {
                reason: DiscardReason::SafeModePreempted
            }
        );
        assert_eq!(result.levels_evaluated.len(), 1);
    }

    /// WHEN the viewer presses Ctrl+Shift+Escape while mutation intake is in progress
    /// THEN override command is processed before any pending mutations in the current batch
    /// (spec lines 53-55)
    #[test]
    fn test_safe_mode_preempts_mutation_intake() {
        // Simulate: override arrived; subsequent event in same frame is discarded
        let mut ectx = default_event_ctx();
        ectx.override_command = Some(OverrideCommand::SafeMode);

        let result = evaluate_event(&ectx);
        assert!(
            matches!(result.outcome, EventOutcome::Discarded { .. }),
            "Event should be discarded when SafeMode override is present"
        );
    }

    // ─── Level 0: Dismiss override (cannot be vetoed) ────────────────────────

    /// WHEN an agent has a valid lease with high priority and the viewer dismisses its tile
    /// THEN the tile is dismissed regardless of priority or capability (spec lines 57-59)
    ///
    /// The Dismiss command itself is accepted at Level 0 (it is an override command,
    /// not a regular event). This test verifies that a Dismiss command does NOT trigger
    /// safe-mode short-circuiting but also that the event is NOT discarded (dismiss is
    /// not safe mode; it is a targeted action).
    #[test]
    fn test_dismiss_override_does_not_trigger_safe_mode_discard() {
        let mut ectx = default_event_ctx();
        ectx.override_command = Some(OverrideCommand::Dismiss {
            tile_id: SceneId::new(),
        });

        let result = evaluate_event(&ectx);
        // Dismiss is a valid override command but does not trigger safe-mode short-circuit.
        // The event itself proceeds through attention and security.
        assert_eq!(result.outcome, EventOutcome::Accept);
        assert_eq!(result.levels_evaluated.len(), 3);
    }

    // ─── Level 0: Freeze ─────────────────────────────────────────────────────

    #[test]
    fn test_freeze_blocks_event() {
        let mut ectx = default_event_ctx();
        ectx.freeze_active = true;

        let result = evaluate_event(&ectx);
        assert_eq!(
            result.outcome,
            EventOutcome::Blocked {
                block_reason: BlockReason::Freeze
            }
        );
        // Only Level 0 evaluated (freeze short-circuits)
        assert_eq!(result.levels_evaluated.len(), 1);
    }

    // ─── Level 4: Attention ──────────────────────────────────────────────────

    #[test]
    fn test_quiet_hours_queues_normal_interruption() {
        let mut ectx = default_event_ctx();
        ectx.attention_context.quiet_hours_active = true;
        ectx.attention_context.quiet_hours_end_us = Some(100_000);
        ectx.attention_context.interruption_class = InterruptionClass::Normal;

        let result = evaluate_event(&ectx);
        assert!(
            matches!(result.outcome, EventOutcome::Queued { .. }),
            "Expected Queued, got {:?}",
            result.outcome
        );
        assert_eq!(result.levels_evaluated.len(), 2); // L0 + L4
    }

    /// WHEN quiet hours are active and a mutation with LOW interruption class arrives
    /// THEN the mutation is discarded (not queued) because LOW content is too stale
    /// by quiet hours exit to be useful (spec RFC 0010 §3.1)
    #[test]
    fn test_quiet_hours_discards_low_interruption() {
        let mut ectx = default_event_ctx();
        ectx.attention_context.quiet_hours_active = true;
        ectx.attention_context.quiet_hours_end_us = Some(100_000);
        ectx.attention_context.interruption_class = InterruptionClass::Low;

        let result = evaluate_event(&ectx);
        assert_eq!(
            result.outcome,
            EventOutcome::Discarded {
                reason: DiscardReason::QuietHoursShed
            },
            "Expected Discarded(QuietHoursShed) for LOW during quiet hours, got {:?}",
            result.outcome
        );
        assert_eq!(result.levels_evaluated.len(), 2); // L0 + L4
    }

    #[test]
    fn test_critical_bypasses_quiet_hours_and_budget() {
        let mut ectx = default_event_ctx();
        ectx.attention_context.quiet_hours_active = true;
        ectx.attention_context.per_agent_interruptions_last_60s = 100; // budget exhausted
        ectx.attention_context.interruption_class = InterruptionClass::Critical;

        let result = evaluate_event(&ectx);
        assert_eq!(result.outcome, EventOutcome::Accept);
        assert_eq!(result.levels_evaluated.len(), 3); // all three levels
    }

    #[test]
    fn test_budget_exhausted_queues_event() {
        let mut ectx = default_event_ctx();
        ectx.attention_context.per_agent_interruptions_last_60s = 20; // at limit
        ectx.attention_context.per_agent_limit = 20;

        let result = evaluate_event(&ectx);
        assert!(matches!(
            result.outcome,
            EventOutcome::Queued {
                queue_reason: QueueReason::AttentionBudgetExhausted { .. },
                ..
            }
        ));
    }

    // ─── Level 3: Security ───────────────────────────────────────────────────

    #[test]
    fn test_security_rejects_missing_capability() {
        let mut ectx = default_event_ctx();
        ectx.required_capabilities = vec!["create_tiles".to_string()];
        // Security context does NOT have create_tiles
        ectx.security_context.granted_capabilities = vec!["access_input_events".to_string()];

        let result = evaluate_event(&ectx);
        assert!(
            matches!(
                &result.outcome,
                EventOutcome::Rejected(err) if err.code == ArbitrationErrorCode::CapabilityDenied
            ),
            "Expected CapabilityDenied, got {:?}",
            result.outcome
        );
        assert_eq!(result.levels_evaluated.len(), 3);
    }

    #[test]
    fn test_security_rejects_invalid_lease() {
        let mut ectx = default_event_ctx();
        ectx.security_context.lease_valid = false;

        let result = evaluate_event(&ectx);
        assert!(matches!(
            &result.outcome,
            EventOutcome::Rejected(err) if err.code == ArbitrationErrorCode::LeaseInvalid
        ));
    }

    #[test]
    fn test_security_rejects_namespace_violation() {
        let mut ectx = default_event_ctx();
        ectx.target_namespace = "agent_b".to_string(); // different namespace

        let result = evaluate_event(&ectx);
        assert!(matches!(
            &result.outcome,
            EventOutcome::Rejected(err) if err.code == ArbitrationErrorCode::NamespaceViolation
        ));
    }

    // ─── safe_mode_activated_in_batch helper ─────────────────────────────────

    #[test]
    fn test_safe_mode_activated_in_batch_detects_safe_mode() {
        let cmds = vec![
            OverrideCommand::Freeze,
            OverrideCommand::SafeMode,
            OverrideCommand::Mute,
        ];
        assert!(safe_mode_activated_in_batch(&cmds));
    }

    #[test]
    fn test_safe_mode_activated_in_batch_negative() {
        let cmds = vec![OverrideCommand::Freeze, OverrideCommand::Mute];
        assert!(!safe_mode_activated_in_batch(&cmds));
    }

    #[test]
    fn test_safe_mode_activated_in_empty_batch_is_false() {
        assert!(!safe_mode_activated_in_batch(&[]));
    }

    // ─── Human override response latency (spec lines 307-309) ────────────────

    /// WHEN a human override command is issued
    /// THEN override response completes within 1 frame (16.6ms)
    #[test]
    fn test_override_response_completes_within_one_frame() {
        let mut ectx = default_event_ctx();
        ectx.override_command = Some(OverrideCommand::SafeMode);

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = evaluate_event(&ectx);
        }
        let elapsed_ms = start.elapsed().as_millis();
        let per_call_ms = elapsed_ms as f64 / 1000.0;

        assert!(
            per_call_ms < 16.6,
            "override response took {per_call_ms:.3}ms, expected < 16.6ms"
        );
    }

    // ─── ArbitrationOutcome conversion ───────────────────────────────────────

    #[test]
    fn test_accept_converts_to_commit() {
        let outcome: ArbitrationOutcome = EventOutcome::Accept.into();
        assert_eq!(outcome, ArbitrationOutcome::Commit);
    }

    #[test]
    fn test_discarded_converts_to_shed() {
        let outcome: ArbitrationOutcome = EventOutcome::Discarded {
            reason: DiscardReason::SafeModePreempted,
        }
        .into();
        assert!(matches!(outcome, ArbitrationOutcome::Shed { .. }));
    }

    #[test]
    fn test_blocked_converts_to_blocked() {
        let outcome: ArbitrationOutcome = EventOutcome::Blocked {
            block_reason: BlockReason::Freeze,
        }
        .into();
        assert!(matches!(outcome, ArbitrationOutcome::Blocked { .. }));
    }
}
