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

pub mod attention;
pub mod attention_budget;
pub mod content;
pub mod event;
pub mod frame;
pub mod interruption;
pub mod mutation;
pub mod override_queue;
pub mod privacy;
pub mod resource;
pub mod safety;
pub mod security;
pub mod stack;
pub mod telemetry;
mod tests;
pub mod types;

// ─── Public API re-exports ────────────────────────────────────────────────────

pub use types::{
    ArbitrationError,
    ArbitrationErrorCode,
    // Arbitration levels and outcomes
    ArbitrationLevel,
    ArbitrationOutcome,
    AttentionContext,
    BlockReason,
    ContentContext,
    // Attention
    InterruptionClass,
    // Mutation classification
    MutationKind,
    OverrideState,
    // Override types
    OverrideType,
    // Policy context and sub-contexts
    PolicyContext,
    PrivacyContext,
    QueueReason,
    RedactionReason,
    RedactionStyle,
    ResourceContext,
    SafetyState,
    SecurityContext,
    ViewerClass,
    // Privacy
    VisibilityClassification,
};

pub use stack::{ArbitrationStack, PolicyEvaluator};

// ─── Per-mutation evaluation pipeline re-exports ──────────────────────────────

pub use mutation::{
    BatchEvalResult, MutationEvalInput, MutationEvalOutput, evaluate_batch, evaluate_mutation,
};

pub use security::{
    CapabilityNameCheck, CapabilitySet, ConfigUnknownCapability, check_canonical_capability_name,
    superseded_canonical, validate_capability_names,
};

pub use privacy::{PrivacyDecision, apply_zone_ceiling, evaluate_privacy, most_restrictive_viewer};

pub use resource::{ResourceDecision, evaluate_resource, resource_decision_to_outcome};

pub use content::{ContentDecision, content_decision_to_outcome, evaluate_content};

pub use telemetry::{
    ArbitrationEventKind, ArbitrationTelemetryEvent, CapabilityAuditEvent, CapabilityAuditKind,
    MutationLatencyAccumulator, PolicyTelemetry,
};

// ─── Per-frame / per-event pipeline re-exports ────────────────────────────────

pub use override_queue::{OVERRIDE_QUEUE_CAPACITY, OverrideCommand, OverrideCommandQueue};

pub use safety::{GpuFailureContext, SafeModeEntryReason, SafetySignal, evaluate_safety};

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
    AgentBudgetPair, AttentionBudget, DEFAULT_PER_AGENT_LIMIT, DEFAULT_PER_ZONE_LIMIT,
    DEFAULT_PER_ZONE_STACK_LIMIT, DEFAULT_WINDOW_SECS,
};
