//! # tze_hud_policy
//!
//! Seven-level policy arbitration stack for tze_hud.
//!
//! ## Overview
//!
//! This crate implements the fixed 7-level arbitration stack specified in
//! `policy-arbitration/spec.md`. The stack evaluates every mutation against
//! all applicable policy levels in strict precedence order:
//!
//! | Level | Name           | Override Types        |
//! |-------|----------------|-----------------------|
//! | 0     | Human Override | Suppress/Redirect/Block |
//! | 1     | Safety         | Suppress/Redirect     |
//! | 2     | Privacy        | Transform             |
//! | 3     | Security       | Suppress              |
//! | 4     | Attention      | Block                 |
//! | 5     | Resource       | Suppress/Transform    |
//! | 6     | Content        | Suppress              |
//!
//! ## Design Principles
//!
//! - **Pure function**: `ArbitrationStack::evaluate` has no side effects.
//! - **Short-circuit**: higher levels are evaluated first; lower levels skip on decisive result.
//! - **Override composition**: see spec §7.3 (Transform+Block → queued-with-redaction).
//! - **Purity constraint**: freeze/safe-mode state transitions are owned by the system shell.
//!   The policy crate only reads `OverrideState`; it never writes it.
//!
//! ## Authority Boundary
//!
//! This crate is the **read-only policy arbitration authority**.
//!
//! - It evaluates mutations against a snapshot of current state (`PolicyContext`) and
//!   returns a decision (`ArbitrationOutcome`).
//! - It has **no side effects**: it never mutates session state, resource counters,
//!   override flags, or attention budgets.
//! - All state transitions (budget enforcement ladder, override flag writes, resource
//!   accounting) are the exclusive responsibility of their respective authorities:
//!   - **Override state / freeze / safe-mode** → `tze_hud_runtime::shell::SafeModeController`
//!   - **Resource budget enforcement ladder** → `tze_hud_runtime::budget::BudgetEnforcer`
//!   - **Resource accounting (decoded bytes)** → `tze_hud_resource::budget::BudgetRegistry`
//!   - **Scene orchestration** → `tze_hud_runtime` (orchestrates; does not evaluate policy)
//!
//! The runtime builds a `PolicyContext` snapshot, calls this crate to evaluate it, and then
//! executes the resulting outcome. Policy evaluation and outcome execution are always separated.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tze_hud_policy::{ArbitrationStack, PolicyContext, ArbitrationOutcome, MutationKind};
//! use tze_hud_policy::types::VisibilityClassification;
//! use tze_hud_scene::SceneId;
//!
//! let stack = ArbitrationStack::new();
//! let outcome = stack.evaluate(
//!     &ctx,
//!     SceneId::new(),
//!     VisibilityClassification::Public,
//!     &["create_tiles"],
//!     "agent_a",
//!     MutationKind::Transactional,
//! );
//! ```

pub mod types;
pub mod stack;
pub mod mutation;
pub mod security;
pub mod privacy;
pub mod resource;
pub mod content;
pub mod telemetry;
pub mod override_queue;
pub mod safety;
pub mod frame;
pub mod event;
pub mod interruption;
pub mod attention_budget;
pub mod attention;
mod tests;

// ─── Public API re-exports ────────────────────────────────────────────────────

pub use types::{
    // Arbitration levels and outcomes
    ArbitrationLevel,
    ArbitrationOutcome,
    ArbitrationError,
    ArbitrationErrorCode,
    BlockReason,
    QueueReason,
    RedactionReason,
    // Override types
    OverrideType,
    // Privacy
    VisibilityClassification,
    ViewerClass,
    RedactionStyle,
    // Attention
    InterruptionClass,
    // Mutation classification
    MutationKind,
    // Policy context and sub-contexts
    PolicyContext,
    OverrideState,
    SafetyState,
    PrivacyContext,
    SecurityContext,
    AttentionContext,
    ResourceContext,
    ContentContext,
};

pub use stack::{ArbitrationStack, PolicyEvaluator};

// ─── Per-mutation evaluation pipeline re-exports ──────────────────────────────

pub use mutation::{
    BatchEvalResult,
    MutationEvalInput,
    MutationEvalOutput,
    evaluate_mutation,
    evaluate_batch,
};

pub use security::{
    CapabilityNameCheck,
    CapabilitySet,
    ConfigUnknownCapability,
    check_canonical_capability_name,
    superseded_canonical,
    validate_capability_names,
};

pub use privacy::{
    PrivacyDecision,
    apply_zone_ceiling,
    evaluate_privacy,
    most_restrictive_viewer,
};

pub use resource::{
    ResourceDecision,
    evaluate_resource,
    resource_decision_to_outcome,
};

pub use content::{
    ContentDecision,
    evaluate_content,
    content_decision_to_outcome,
};

pub use telemetry::{
    ArbitrationEventKind,
    ArbitrationTelemetryEvent,
    CapabilityAuditEvent,
    CapabilityAuditKind,
    MutationLatencyAccumulator,
    PolicyTelemetry,
};

// ─── Per-frame / per-event pipeline re-exports ────────────────────────────────

pub use override_queue::{OverrideCommand, OverrideCommandQueue, OVERRIDE_QUEUE_CAPACITY};

pub use safety::{
    GpuFailureContext, SafetySignal, SafeModeEntryReason, evaluate_safety,
};

pub use frame::{
    ContentFrameSignal, FrameEvaluation, FramePrivacyContext, ResourceFrameSignal, evaluate_frame,
};

pub use event::{
    DiscardReason, EventContext, EventEvaluation, EventOutcome, evaluate_event,
    safe_mode_activated_in_batch,
};

// ─── Dedicated attention module re-exports ────────────────────────────────────

pub use attention::{AttentionDecision, evaluate_attention};
pub use attention_budget::{
    AttentionBudget,
    AgentBudgetPair,
    DEFAULT_PER_AGENT_LIMIT,
    DEFAULT_PER_ZONE_LIMIT,
    DEFAULT_PER_ZONE_STACK_LIMIT,
    DEFAULT_WINDOW_SECS,
};
