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
    is_transactional,
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
