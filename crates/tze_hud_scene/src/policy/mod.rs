//! Policy evaluator trait for v1 arbitration.
//!
//! Encodes the seven-level arbitration stack from
//! `policy-arbitration/spec.md §Requirement: Seven-Level Arbitration Stack`
//! and related requirements.  This module defines **only** the trait contract
//! and supporting types — no implementation is provided here.

use crate::clock::Clock;

// Re-export the canonical InterruptionClass from events to avoid duplicate
// wire-level type definitions and potential mismatches across subsystems.
pub use crate::events::InterruptionClass;

// ─── Policy Levels ───────────────────────────────────────────────────────────

/// The seven arbitration levels ordered by precedence (0 = highest).
///
/// From spec §Requirement: Seven-Level Arbitration Stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PolicyLevel {
    /// Level 0 — Human Override (dismiss, safe mode, freeze, mute).
    HumanOverride = 0,
    /// Level 1 — Safety (GPU health, scene integrity).
    Safety = 1,
    /// Level 2 — Privacy (visibility classification vs viewer class).
    Privacy = 2,
    /// Level 3 — Security (capability scopes, lease validity, namespace isolation).
    Security = 3,
    /// Level 4 — Attention (interruption budget, quiet hours).
    Attention = 4,
    /// Level 5 — Resource (per-agent budgets, degradation ladder).
    Resource = 5,
    /// Level 6 — Content (zone contention resolution).
    Content = 6,
}

// ─── Visibility & Viewer ─────────────────────────────────────────────────────

/// Tile visibility classification.
///
/// From spec §Requirement: Level 2 Privacy Evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VisibilityClassification {
    Public = 0,
    Household = 1,
    Private = 2,
    Sensitive = 3,
}

/// Who is currently viewing the screen.
///
/// Access matrix: Owner sees all; HouseholdMember sees Public+Household;
/// KnownGuest/Unknown/Nobody see only Public.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerClass {
    Owner,
    HouseholdMember,
    KnownGuest,
    Unknown,
    Nobody,
}

impl ViewerClass {
    /// Returns the maximum `VisibilityClassification` this viewer may see without redaction.
    pub fn visibility_ceiling(self) -> VisibilityClassification {
        match self {
            ViewerClass::Owner => VisibilityClassification::Sensitive,
            ViewerClass::HouseholdMember => VisibilityClassification::Household,
            ViewerClass::KnownGuest | ViewerClass::Unknown | ViewerClass::Nobody => {
                VisibilityClassification::Public
            }
        }
    }
}

// ─── Policy Context ──────────────────────────────────────────────────────────

/// Input to the policy evaluator for a single mutation.
///
/// A correct implementation must evaluate levels in fixed order (0→6),
/// short-circuiting on the first decisive rejection.
#[derive(Clone, Debug)]
pub struct PolicyContext {
    /// Is the GPU device currently healthy?
    pub gpu_healthy: bool,
    /// Is the scene graph currently in safe mode?
    pub safe_mode_active: bool,
    /// Is the scene currently frozen (Level 0)?
    pub frozen: bool,
    /// Visibility classification of the tile being mutated.
    pub tile_classification: VisibilityClassification,
    /// Most restrictive viewer currently present.
    pub viewer_class: ViewerClass,
    /// Capabilities held by the requesting agent.
    pub agent_capabilities: Vec<String>,
    /// Namespace of the tile being mutated.
    pub tile_namespace: String,
    /// Namespace of the requesting agent.
    pub agent_namespace: String,
    /// Interruption class of the mutation.
    pub interruption_class: InterruptionClass,
    /// Fraction of the per-agent attention budget already consumed [0.0, 1.0].
    pub attention_usage_fraction: f64,
    /// Fraction of the resource budget already consumed [0.0, 1.0].
    pub resource_usage_fraction: f64,
    /// Whether quiet hours are currently active.
    pub quiet_hours_active: bool,
    /// Required capabilities for this mutation.
    pub required_capabilities: Vec<String>,
    /// Zone contention policy name (e.g., "LatestWins", "Stack").
    pub zone_contention_policy: String,
    /// True if this is a zone publish mutation (full stack evaluation required).
    pub is_zone_publish: bool,
}

// ─── Policy Decision ─────────────────────────────────────────────────────────

/// Outcome of policy evaluation for a single mutation.
///
/// From spec §Requirement: ArbitrationOutcome Types.
///
/// Note on composed outcomes: `QueueRedacted` handles the cross-level scenario
/// where privacy (Level 2) requires redaction AND quiet hours (Level 4) requires
/// queueing.  Per spec §Cross-Level Conflict Resolution: "Privacy redaction plus
/// quiet hours — mutation is committed with redaction AND quiet hours evaluation
/// still runs".  `QueueRedacted` encodes this combined semantics so implementations
/// can satisfy the contract unambiguously without losing either dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArbitrationAction {
    /// Accepted and committed to scene.
    Commit,
    /// Committed but rendered with redaction placeholder.
    CommitRedacted,
    /// Deferred until condition clears.
    Queue,
    /// Queued AND will be committed with redaction when delivered (privacy + quiet hours).
    QueueRedacted,
    /// Rejected with structured error.
    Reject,
    /// Shed by resource/degradation policy — no error.
    Shed,
    /// Queued by human override freeze.
    Blocked,
}

/// Structured error code for rejections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyRejectCode {
    CapabilityRequired { missing: String },
    NamespaceViolation,
    SafeModeActive,
    BudgetExceeded,
    LeaseNotActive,
    Other(String),
}

/// Full policy decision returned by `PolicyEvaluator::evaluate`.
#[derive(Clone, Debug)]
pub struct PolicyDecision {
    /// Which action to take for this mutation.
    pub action: ArbitrationAction,
    /// The highest (lowest numeric) level that was decisive.
    pub applied_level: PolicyLevel,
    /// Human-readable explanation (and machine-readable code for rejections).
    pub reason: String,
    /// If action == Reject, the structured error code.
    pub reject_code: Option<PolicyRejectCode>,
}

// ─── PolicyEvaluator Trait ───────────────────────────────────────────────────

/// Trait encoding the seven-level policy arbitration stack.
///
/// Implementations must evaluate levels in strict order (0→6) and
/// short-circuit on the first decisive result.  The total evaluation time
/// for a single mutation MUST be < 200µs (< 50µs for per-mutation checks).
///
/// Clock injection via `C: Clock` enables deterministic time-based assertions
/// (e.g., attention budget rolling windows).
pub trait PolicyEvaluator<C: Clock> {
    /// Create a new evaluator backed by the given clock.
    fn new(clock: C) -> Self
    where
        Self: Sized;

    /// Evaluate the arbitration stack for one mutation.  Must return in < 200µs.
    fn evaluate(&mut self, ctx: &PolicyContext) -> PolicyDecision;

    /// Record an attention event for the given agent+zone pair.
    /// Increments the rolling counter (60-second window).
    fn record_attention_event(&mut self, agent_ns: &str, zone_id: &str, class: InterruptionClass);

    /// Returns `true` if the agent's attention budget is currently exhausted.
    fn is_attention_budget_exhausted(&self, agent_ns: &str) -> bool;

    /// Current rolling interruption count for the given agent (last 60 seconds).
    fn agent_interruption_count(&self, agent_ns: &str) -> u32;

    /// Returns `true` if quiet hours are currently active.
    fn quiet_hours_active(&self) -> bool;

    /// Enter/exit quiet hours mode (used in tests via TestClock).
    fn set_quiet_hours(&mut self, active: bool);
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::clock::TestClock;

    fn allow_ctx() -> PolicyContext {
        PolicyContext {
            gpu_healthy: true,
            safe_mode_active: false,
            frozen: false,
            tile_classification: VisibilityClassification::Public,
            viewer_class: ViewerClass::Owner,
            agent_capabilities: vec!["create_tiles".into()],
            tile_namespace: "agent_a".into(),
            agent_namespace: "agent_a".into(),
            interruption_class: InterruptionClass::Normal,
            attention_usage_fraction: 0.0,
            resource_usage_fraction: 0.0,
            quiet_hours_active: false,
            required_capabilities: vec!["create_tiles".into()],
            zone_contention_policy: "LatestWins".into(),
            is_zone_publish: false,
        }
    }

    // ── 1. Stack ordering ─────────────────────────────────────────────────────

    /// WHEN arbitration stack is initialized THEN it contains exactly 7 levels 0-6.
    #[test]
    fn test_policy_levels_ordered_correctly() {
        // Enum ordering check — this is pure type-level, no impl needed.
        assert!(PolicyLevel::HumanOverride < PolicyLevel::Safety);
        assert!(PolicyLevel::Safety < PolicyLevel::Privacy);
        assert!(PolicyLevel::Privacy < PolicyLevel::Security);
        assert!(PolicyLevel::Security < PolicyLevel::Attention);
        assert!(PolicyLevel::Attention < PolicyLevel::Resource);
        assert!(PolicyLevel::Resource < PolicyLevel::Content);
        assert_eq!(PolicyLevel::HumanOverride as u8, 0);
        assert_eq!(PolicyLevel::Content as u8, 6);
    }

    /// WHEN safety event (GPU failure) fires THEN Level 1 short-circuits.
    pub fn test_safety_short_circuits_lower_levels<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        let mut ctx = allow_ctx();
        ctx.gpu_healthy = false;
        ctx.safe_mode_active = true;
        let decision = evaluator.evaluate(&ctx);
        // Level 1 Safety must produce Reject or Blocked; lower levels must be skipped.
        assert!(
            matches!(
                decision.action,
                ArbitrationAction::Reject | ArbitrationAction::Blocked
            ),
            "GPU failure should cause reject/blocked at Level 1, got {:?}",
            decision.action
        );
        assert_eq!(decision.applied_level, PolicyLevel::Safety);
    }

    /// WHEN privacy redaction required THEN Level 2 applied, action = CommitRedacted.
    pub fn test_privacy_redaction_applied_before_lower_levels<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        let mut ctx = allow_ctx();
        ctx.tile_classification = VisibilityClassification::Private;
        ctx.viewer_class = ViewerClass::KnownGuest; // can only see Public
        let decision = evaluator.evaluate(&ctx);
        assert_eq!(decision.action, ArbitrationAction::CommitRedacted);
        assert_eq!(decision.applied_level, PolicyLevel::Privacy);
    }

    /// WHEN owner views private tile THEN no redaction applied.
    pub fn test_owner_sees_private_tile<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        let mut ctx = allow_ctx();
        ctx.tile_classification = VisibilityClassification::Private;
        ctx.viewer_class = ViewerClass::Owner;
        let decision = evaluator.evaluate(&ctx);
        // Owner can see private content — no redaction.
        assert_ne!(decision.action, ArbitrationAction::CommitRedacted);
    }

    /// WHEN capability check fails THEN Level 3 returns CAPABILITY_REQUIRED.
    pub fn test_capability_required_at_level_3<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        let mut ctx = allow_ctx();
        ctx.agent_capabilities = vec![]; // no capabilities
        ctx.required_capabilities = vec!["create_tiles".into()];
        let decision = evaluator.evaluate(&ctx);
        assert_eq!(decision.action, ArbitrationAction::Reject);
        assert_eq!(decision.applied_level, PolicyLevel::Security);
        assert!(matches!(
            decision.reject_code,
            Some(PolicyRejectCode::CapabilityRequired { .. })
        ));
    }

    /// WHEN agent attempts mutation in another agent's namespace THEN NamespaceViolation.
    pub fn test_namespace_violation_at_level_3<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        let mut ctx = allow_ctx();
        ctx.tile_namespace = "agent_b".into();
        ctx.agent_namespace = "agent_a".into(); // different namespace
        let decision = evaluator.evaluate(&ctx);
        assert_eq!(decision.action, ArbitrationAction::Reject);
        assert_eq!(decision.applied_level, PolicyLevel::Security);
        assert!(matches!(
            decision.reject_code,
            Some(PolicyRejectCode::NamespaceViolation)
        ));
    }

    /// WHEN scene frozen (Level 0) THEN mutation is Blocked regardless of lower levels.
    pub fn test_frozen_scene_blocks_mutation<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        let mut ctx = allow_ctx();
        ctx.frozen = true;
        let decision = evaluator.evaluate(&ctx);
        assert_eq!(decision.action, ArbitrationAction::Blocked);
        assert_eq!(decision.applied_level, PolicyLevel::HumanOverride);
    }

    /// WHEN attention budget at 80% THEN AttentionBudgetWarning condition applies.
    pub fn test_attention_budget_warning_at_80_percent<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        // Simulate 16 attention events (80% of default 20/min)
        for _ in 0..16 {
            evaluator.record_attention_event("agent_a", "zone_1", InterruptionClass::Normal);
        }
        // At 80%, warning should be detectable.
        let count = evaluator.agent_interruption_count("agent_a");
        assert_eq!(count, 16, "should have 16 interruptions recorded");
    }

    /// WHEN attention budget at 100% (>20/min) THEN mutations coalesced (latest-wins).
    pub fn test_attention_budget_exhausted_coalesces<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        for _ in 0..21 {
            evaluator.record_attention_event("agent_a", "zone_1", InterruptionClass::Normal);
        }
        assert!(
            evaluator.is_attention_budget_exhausted("agent_a"),
            "21 events should exhaust 20/min budget"
        );
    }

    /// WHEN quiet hours active and NORMAL mutation arrives THEN Queue action.
    pub fn test_quiet_hours_queues_normal_mutations<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        evaluator.set_quiet_hours(true);
        let mut ctx = allow_ctx();
        ctx.quiet_hours_active = true;
        ctx.interruption_class = InterruptionClass::Normal;
        let decision = evaluator.evaluate(&ctx);
        assert_eq!(decision.action, ArbitrationAction::Queue);
    }

    /// WHEN quiet hours active and CRITICAL mutation arrives THEN immediate delivery.
    pub fn test_quiet_hours_passes_critical<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        evaluator.set_quiet_hours(true);
        let mut ctx = allow_ctx();
        ctx.quiet_hours_active = true;
        ctx.interruption_class = InterruptionClass::Critical;
        let decision = evaluator.evaluate(&ctx);
        // CRITICAL bypasses quiet hours.
        assert_ne!(
            decision.action,
            ArbitrationAction::Queue,
            "CRITICAL should not be queued"
        );
    }

    /// WHEN security short-circuits THEN lower levels (Resource, Content) not evaluated.
    pub fn test_security_short_circuits_resource_and_content<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        let mut ctx = allow_ctx();
        ctx.agent_capabilities = vec![];
        ctx.required_capabilities = vec!["create_tiles".into()];
        // Resource and Content would both pass, but Security should short-circuit.
        let decision = evaluator.evaluate(&ctx);
        assert_eq!(decision.applied_level, PolicyLevel::Security);
        // Level 5 (Resource) and Level 6 (Content) must NOT have been reached.
        assert!(decision.applied_level < PolicyLevel::Resource);
    }

    /// WHEN privacy redacts AND quiet hours are active THEN QueueRedacted (composed action).
    /// Per spec: mutation committed with redaction AND quiet hours evaluation still runs.
    /// `QueueRedacted` encodes this combined semantics: the mutation will be committed
    /// with redaction when delivered, but is queued until quiet hours exit.
    pub fn test_privacy_redaction_composed_with_quiet_hours<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        evaluator.set_quiet_hours(true);
        let mut ctx = allow_ctx();
        ctx.tile_classification = VisibilityClassification::Private;
        ctx.viewer_class = ViewerClass::KnownGuest;
        ctx.quiet_hours_active = true;
        ctx.interruption_class = InterruptionClass::Normal;
        ctx.is_zone_publish = true; // zone publish gets full stack
        let decision = evaluator.evaluate(&ctx);
        // Per spec §Cross-Level Conflict Resolution: "Scenario: Privacy redaction plus quiet hours"
        // mutation IS committed with redaction AND quiet hours evaluation runs →
        // action should be QueueRedacted (both privacy and attention levels decisive).
        assert_eq!(decision.action, ArbitrationAction::QueueRedacted);
    }

    /// WHEN multi-viewer scenario (Owner + Guest) THEN most restrictive viewer class applies.
    pub fn test_multi_viewer_most_restrictive<E: PolicyEvaluator<TestClock>>() {
        let clock = TestClock::new(0);
        let mut evaluator = E::new(clock);
        // Most restrictive viewer already injected into ctx as viewer_class.
        let mut ctx = allow_ctx();
        ctx.tile_classification = VisibilityClassification::Private;
        ctx.viewer_class = ViewerClass::KnownGuest; // most restrictive wins
        let decision = evaluator.evaluate(&ctx);
        assert_eq!(decision.action, ArbitrationAction::CommitRedacted);
    }

    // ── Trait-level compile check (no-op; ensures generic bounds hold) ────────

    #[test]
    #[ignore = "no implementation yet"]
    fn test_policy_evaluator_generic_compile_check() {
        // This test's sole purpose is to confirm the trait can be instantiated
        // generically in a test context.  Replace with a real impl to pass.
        fn use_evaluator<E: PolicyEvaluator<TestClock>>() {
            let clock = TestClock::new(0);
            let mut ev = E::new(clock);
            let ctx = allow_ctx();
            let _ = ev.evaluate(&ctx);
        }
        // Calling use_evaluator::<ConcreteImpl>() will pass once an impl exists.
    }
}
