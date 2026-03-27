//! # Per-Frame Evaluation Pipeline
//!
//! Implements the per-frame arbitration evaluation specified in
//! policy-arbitration/spec.md (lines 194-205).
//!
//! ## Evaluation Order
//!
//! At the start of each frame cycle, **before mutation intake**:
//!
//! ```text
//! Level 1 (Safety) → Level 2 (Privacy) → Level 5 (Resource) → Level 6 (Content)
//! ```
//!
//! If Level 1 triggers safe mode or catastrophic exit, Levels 2/5/6 MUST NOT be
//! evaluated for that frame (short-circuit on safety signal).
//!
//! ## Latency Budget
//!
//! Total per-frame evaluation MUST complete in < 200us (spec line 195, §9.1).
//!
//! ## Purity Constraint
//!
//! `evaluate_frame` is a pure function over typed inputs. It produces a
//! `FrameEvaluation` describing signals for the frame. The system shell reads
//! `FrameEvaluation` and executes state transitions (enter safe mode, suspend
//! leases, etc.).

use crate::safety::{GpuFailureContext, SafetySignal, evaluate_safety};
use crate::types::{
    ArbitrationLevel, PolicyContext, RedactionReason, ViewerClass, VisibilityClassification,
};

// ─── Frame evaluation output ──────────────────────────────────────────────────

/// Result of evaluating the per-frame pipeline.
///
/// This is a **pure output** — no side effects. The compositor and system shell
/// read this struct and act accordingly.
#[derive(Clone, Debug)]
pub struct FrameEvaluation {
    /// Safety signal from Level 1. If `should_short_circuit()`, pipeline stopped here.
    pub safety_signal: SafetySignal,

    /// Privacy redaction flag from Level 2.
    /// `None` if Level 1 short-circuited and Level 2 was not evaluated.
    pub privacy_redacted: Option<bool>,

    /// Redaction reason, if `privacy_redacted == Some(true)`.
    pub privacy_redaction_reason: Option<RedactionReason>,

    /// Resource signal from Level 5.
    /// `None` if Level 1 short-circuited.
    pub resource_signal: Option<ResourceFrameSignal>,

    /// Content signal from Level 6.
    /// `None` if Level 1 short-circuited.
    pub content_signal: Option<ContentFrameSignal>,

    /// Which levels were evaluated this frame.
    pub levels_evaluated: Vec<ArbitrationLevel>,
}

/// Resource-level signal produced by per-frame Level 5 evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceFrameSignal {
    /// Resource budgets are nominal.
    Nominal,
    /// Per-frame resource budgets are paused (scene is frozen).
    Paused,
    /// Degradation is currently active at the given level; the degradation ladder applies.
    ///
    /// This signal is emitted every frame that `degradation_level > 0`, not only when
    /// the level transitions. The compositor should apply degradation policy at `level`
    /// for this frame.
    DegradationActive { level: u32 },
}

/// Content-level signal produced by per-frame Level 6 evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentFrameSignal {
    /// Content contention state is nominal.
    Nominal,
}

// ─── Per-frame context ────────────────────────────────────────────────────────

/// Per-frame input context for Level 2 (Privacy) evaluation.
///
/// This carries the content classification for the frame being evaluated.
/// In practice, the policy layer evaluates the *most restrictive* classification
/// present in the scene for per-frame checks (zone default ceiling).
#[derive(Clone, Debug)]
pub struct FramePrivacyContext {
    /// Effective content classification for this frame (zone default ceiling applied).
    pub effective_classification: VisibilityClassification,
}

impl Default for FramePrivacyContext {
    fn default() -> Self {
        Self {
            effective_classification: VisibilityClassification::Public,
        }
    }
}

// ─── Per-frame evaluation (pure) ─────────────────────────────────────────────

/// Evaluate the per-frame arbitration pipeline.
///
/// ## Arguments
///
/// - `ctx` — read-only policy context snapshot (all runtime state).
/// - `gpu` — GPU failure context for this frame.
/// - `frame_privacy` — per-frame privacy context (effective classification for the frame).
///
/// ## Returns
///
/// A `FrameEvaluation` describing the per-frame policy outcomes.
/// The caller (compositor / system shell) is responsible for acting on the signals.
///
/// ## Evaluation order
///
/// `Level 1 (Safety) → Level 2 (Privacy) → Level 5 (Resource) → Level 6 (Content)`
///
/// Short-circuit: if Level 1 signals safe mode or catastrophic exit, Levels 2/5/6
/// are NOT evaluated.
///
/// # Pure function contract
///
/// No side effects. All state transitions are signaled via the returned
/// `FrameEvaluation`, not executed here.
pub fn evaluate_frame(
    ctx: &PolicyContext,
    gpu: &GpuFailureContext,
    frame_privacy: &FramePrivacyContext,
) -> FrameEvaluation {
    let mut levels_evaluated = Vec::with_capacity(4);

    // ─── Level 1: Safety ─────────────────────────────────────────────────────
    levels_evaluated.push(ArbitrationLevel::Safety);
    let safety_signal = evaluate_safety(&ctx.safety_state, gpu);

    // Short-circuit: if Level 1 triggers safe mode or catastrophic exit, stop here.
    if safety_signal.should_short_circuit() {
        return FrameEvaluation {
            safety_signal,
            privacy_redacted: None,
            privacy_redaction_reason: None,
            resource_signal: None,
            content_signal: None,
            levels_evaluated,
        };
    }

    // ─── Level 2: Privacy ────────────────────────────────────────────────────
    levels_evaluated.push(ArbitrationLevel::Privacy);
    let viewer = ctx.privacy_context.effective_viewer_class;
    let privacy_redacted = !viewer.may_see(frame_privacy.effective_classification);
    let privacy_redaction_reason = if privacy_redacted {
        Some(compute_frame_redaction_reason(
            &ctx.privacy_context.viewer_classes,
            viewer,
            frame_privacy.effective_classification,
        ))
    } else {
        None
    };

    // ─── Level 5: Resource ───────────────────────────────────────────────────
    levels_evaluated.push(ArbitrationLevel::Resource);
    let resource_signal = evaluate_frame_level5_resource(&ctx.resource_context);

    // ─── Level 6: Content ────────────────────────────────────────────────────
    levels_evaluated.push(ArbitrationLevel::Content);
    let content_signal = ContentFrameSignal::Nominal;

    FrameEvaluation {
        safety_signal,
        privacy_redacted: Some(privacy_redacted),
        privacy_redaction_reason,
        resource_signal: Some(resource_signal),
        content_signal: Some(content_signal),
        levels_evaluated,
    }
}

/// Compute per-frame redaction reason.
fn compute_frame_redaction_reason(
    viewer_classes: &[ViewerClass],
    effective_viewer: ViewerClass,
    classification: VisibilityClassification,
) -> RedactionReason {
    if viewer_classes.len() > 1 {
        RedactionReason::MultiViewerRestriction
    } else {
        RedactionReason::ViewerClassInsufficient {
            required: classification,
            actual: effective_viewer,
        }
    }
}

/// Level 5 per-frame resource evaluation.
///
/// Returns the per-frame resource signal (Nominal / Paused / DegradationActive).
fn evaluate_frame_level5_resource(ctx: &crate::types::ResourceContext) -> ResourceFrameSignal {
    if ctx.budgets_paused {
        return ResourceFrameSignal::Paused;
    }
    if ctx.degradation_level > 0 {
        return ResourceFrameSignal::DegradationActive {
            level: ctx.degradation_level,
        };
    }
    ResourceFrameSignal::Nominal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::GpuFailureContext;
    use crate::types::{
        AttentionContext, ContentContext, InterruptionClass, OverrideState, PolicyContext,
        PrivacyContext, RedactionStyle, ResourceContext, SafetyState, SecurityContext, ViewerClass,
        VisibilityClassification,
    };
    use tze_hud_scene::types::ContentionPolicy;

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
                redaction_style: RedactionStyle::Pattern,
            },
            security_context: SecurityContext::default(),
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

    fn no_gpu_failure() -> GpuFailureContext {
        GpuFailureContext::default()
    }

    fn public_frame_privacy() -> FramePrivacyContext {
        FramePrivacyContext {
            effective_classification: VisibilityClassification::Public,
        }
    }

    // ─── Nominal path ─────────────────────────────────────────────────────────

    /// WHEN a frame cycle begins under normal load
    /// THEN the full per-frame evaluation (Levels 1, 2, 5, 6) completes and all signals are nominal
    #[test]
    fn test_nominal_frame_evaluates_all_four_levels() {
        let ctx = default_policy_context();
        let result = evaluate_frame(&ctx, &no_gpu_failure(), &public_frame_privacy());

        assert_eq!(result.safety_signal, SafetySignal::Nominal);
        assert_eq!(result.privacy_redacted, Some(false));
        assert_eq!(result.resource_signal, Some(ResourceFrameSignal::Nominal));
        assert_eq!(result.content_signal, Some(ContentFrameSignal::Nominal));

        // All four levels evaluated
        assert_eq!(result.levels_evaluated.len(), 4);
        assert_eq!(result.levels_evaluated[0], ArbitrationLevel::Safety);
        assert_eq!(result.levels_evaluated[1], ArbitrationLevel::Privacy);
        assert_eq!(result.levels_evaluated[2], ArbitrationLevel::Resource);
        assert_eq!(result.levels_evaluated[3], ArbitrationLevel::Content);
    }

    // ─── Per-frame evaluation ordering: L1 → L2 → L5 → L6 ───────────────────

    #[test]
    fn test_per_frame_evaluation_order_is_1_2_5_6() {
        let ctx = default_policy_context();
        let result = evaluate_frame(&ctx, &no_gpu_failure(), &public_frame_privacy());

        let order: Vec<u8> = result.levels_evaluated.iter().map(|l| l.index()).collect();
        assert_eq!(order, vec![1, 2, 5, 6]);
    }

    // ─── Level 1 short-circuit ────────────────────────────────────────────────

    /// WHEN a frame cycle begins and GPU health check triggers safe mode
    /// THEN if safe mode triggers, no further per-frame evaluation occurs (spec line 199-201)
    #[test]
    fn test_safe_mode_short_circuits_levels_2_5_6() {
        let mut ctx = default_policy_context();
        ctx.safety_state.gpu_healthy = false;

        let result = evaluate_frame(&ctx, &no_gpu_failure(), &public_frame_privacy());

        assert!(result.safety_signal.should_short_circuit());
        assert_eq!(result.privacy_redacted, None);
        assert_eq!(result.resource_signal, None);
        assert_eq!(result.content_signal, None);

        // Only Level 1 evaluated
        assert_eq!(result.levels_evaluated.len(), 1);
        assert_eq!(result.levels_evaluated[0], ArbitrationLevel::Safety);
    }

    #[test]
    fn test_scene_corruption_short_circuits() {
        let mut ctx = default_policy_context();
        ctx.safety_state.scene_graph_intact = false;

        let result = evaluate_frame(&ctx, &no_gpu_failure(), &public_frame_privacy());

        assert!(result.safety_signal.should_short_circuit());
        assert_eq!(result.privacy_redacted, None);
        assert!(result.levels_evaluated.len() == 1);
    }

    #[test]
    fn test_catastrophic_exit_short_circuits() {
        let gpu = GpuFailureContext {
            overlay_cannot_render: true,
            ..Default::default()
        };
        let ctx = default_policy_context();
        let result = evaluate_frame(&ctx, &gpu, &public_frame_privacy());

        assert_eq!(result.safety_signal, SafetySignal::CatastrophicExit);
        assert_eq!(result.privacy_redacted, None);
        assert_eq!(result.levels_evaluated.len(), 1);
    }

    #[test]
    fn test_gpu_reconfiguration_does_not_short_circuit() {
        let gpu = GpuFailureContext {
            surface_lost: true,
            surface_reconfigure_succeeded: true,
            ..Default::default()
        };
        let ctx = default_policy_context();
        let result = evaluate_frame(&ctx, &gpu, &public_frame_privacy());

        assert_eq!(result.safety_signal, SafetySignal::GpuReconfiguration);
        // Tier 1 does NOT short-circuit; all four levels are evaluated
        assert_eq!(result.levels_evaluated.len(), 4);
    }

    // ─── Level 2: Privacy ────────────────────────────────────────────────────

    #[test]
    fn test_private_content_redacted_for_guest_in_frame() {
        let mut ctx = default_policy_context();
        ctx.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
        ctx.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];

        let frame_privacy = FramePrivacyContext {
            effective_classification: VisibilityClassification::Private,
        };
        let result = evaluate_frame(&ctx, &no_gpu_failure(), &frame_privacy);

        assert_eq!(result.privacy_redacted, Some(true));
        assert!(result.privacy_redaction_reason.is_some());
    }

    #[test]
    fn test_public_content_not_redacted_for_guest_in_frame() {
        let mut ctx = default_policy_context();
        ctx.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
        ctx.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];

        let result = evaluate_frame(&ctx, &no_gpu_failure(), &public_frame_privacy());
        assert_eq!(result.privacy_redacted, Some(false));
        assert!(result.privacy_redaction_reason.is_none());
    }

    #[test]
    fn test_owner_sees_sensitive_content() {
        let ctx = default_policy_context();
        let frame_privacy = FramePrivacyContext {
            effective_classification: VisibilityClassification::Sensitive,
        };
        let result = evaluate_frame(&ctx, &no_gpu_failure(), &frame_privacy);
        assert_eq!(result.privacy_redacted, Some(false));
    }

    // ─── Level 5: Resource ───────────────────────────────────────────────────

    #[test]
    fn test_resource_signal_paused_during_freeze() {
        let mut ctx = default_policy_context();
        ctx.resource_context.budgets_paused = true;

        let result = evaluate_frame(&ctx, &no_gpu_failure(), &public_frame_privacy());
        assert_eq!(result.resource_signal, Some(ResourceFrameSignal::Paused));
    }

    #[test]
    fn test_resource_signal_degradation_changed() {
        let mut ctx = default_policy_context();
        ctx.resource_context.degradation_level = 3;

        let result = evaluate_frame(&ctx, &no_gpu_failure(), &public_frame_privacy());
        assert_eq!(
            result.resource_signal,
            Some(ResourceFrameSignal::DegradationActive { level: 3 })
        );
    }

    // ─── Policy evaluation latency budget (spec lines 299-301) ───────────────

    /// WHEN the runtime operates under normal load
    /// THEN the full per-frame evaluation completes in < 200us
    ///
    /// This is a timing smoke test. It does not fail on latency alone (that would be
    /// flaky on CI), but it ensures the function completes quickly and logs the duration.
    #[test]
    fn test_per_frame_evaluation_completes_quickly() {
        let ctx = default_policy_context();
        let gpu = no_gpu_failure();
        let frame_privacy = public_frame_privacy();

        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = evaluate_frame(&ctx, &gpu, &frame_privacy);
        }
        let elapsed_us = start.elapsed().as_micros();
        let per_call_us = elapsed_us / 100;

        // Smoke test: log observed latency for manual inspection.
        // We do not assert on microsecond-scale latency here — CI machines vary too
        // widely for a 200us hard gate to be reliable. Real latency budgets are
        // enforced by integration benchmarks (criterion). The production requirement
        // is < 200us (spec §9.1).
        eprintln!(
            "per-frame evaluation average latency: {per_call_us}us over 100 iterations (spec budget: 200us)"
        );
    }

    // ─── Privacy transition latency (spec lines 311-312) ─────────────────────

    /// WHEN viewer class changes require privacy re-evaluation
    /// THEN all privacy transitions complete within 2 frames (33.2ms)
    ///
    /// We test this by running 100 evaluations with a viewer class change and
    /// verifying total duration is well within 33.2ms.
    #[test]
    fn test_privacy_transition_latency() {
        let mut ctx = default_policy_context();
        let gpu = no_gpu_failure();
        let frame_privacy = FramePrivacyContext {
            effective_classification: VisibilityClassification::Private,
        };

        let start = std::time::Instant::now();
        // Simulate viewer transition: owner → guest → owner alternating
        for i in 0..100 {
            if i % 2 == 0 {
                ctx.privacy_context.effective_viewer_class = ViewerClass::KnownGuest;
                ctx.privacy_context.viewer_classes = vec![ViewerClass::KnownGuest];
            } else {
                ctx.privacy_context.effective_viewer_class = ViewerClass::Owner;
                ctx.privacy_context.viewer_classes = vec![ViewerClass::Owner];
            }
            let _ = evaluate_frame(&ctx, &gpu, &frame_privacy);
        }
        let elapsed_ms = start.elapsed().as_millis();
        let per_call_ms = elapsed_ms as f64 / 100.0;

        // Each transition must complete well within 33.2ms (2 frames)
        assert!(
            per_call_ms < 33.2,
            "privacy transition took {per_call_ms:.2}ms, expected < 33.2ms"
        );
    }
}
