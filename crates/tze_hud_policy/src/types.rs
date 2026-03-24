//! Core policy types: VisibilityClassification, ViewerClass, InterruptionClass,
//! override types, arbitration outcomes, and policy context.
//!
//! These are pure data types — no side effects, no async.

use serde::{Deserialize, Serialize};
use tze_hud_scene::{SceneId, types::ContentionPolicy};

// ─── Privacy types ───────────────────────────────────────────────────────────

/// Content classification for privacy access control.
///
/// Ordered from least restrictive (Public) to most restrictive (Sensitive).
/// `max(agent_declared, zone_default)` is always applied (zone ceiling rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisibilityClassification {
    /// Visible to all viewer classes.
    Public = 0,
    /// Visible to HouseholdMember and Owner.
    Household = 1,
    /// Visible to Owner only.
    Private = 2,
    /// Visible to Owner only (most restricted; same access as Private but semantically distinct).
    Sensitive = 3,
}

/// The class of the current viewer(s).
///
/// Ordered from most permissive (Owner) to least permissive (Nobody).
/// When multiple viewers are present, the most restrictive class applies
/// (spec §2.2: "Nobody > Unknown > KnownGuest > HouseholdMember > Owner").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewerClass {
    /// Display owner — sees all content including Sensitive.
    Owner,
    /// A trusted household member — sees Public and Household.
    HouseholdMember,
    /// A recognized but non-household visitor — sees only Public.
    KnownGuest,
    /// An unrecognized person — sees only Public.
    Unknown,
    /// No one present — sees only Public.
    Nobody,
}

impl ViewerClass {
    /// Numeric restriction level: higher = more restrictive.
    ///
    /// Used for within-level tie-breaking at Level 2.
    pub fn restriction_level(self) -> u8 {
        match self {
            ViewerClass::Owner => 0,
            ViewerClass::HouseholdMember => 1,
            ViewerClass::KnownGuest => 2,
            ViewerClass::Unknown => 3,
            ViewerClass::Nobody => 4,
        }
    }

    /// Returns true if this viewer class may see content at `classification`.
    pub fn may_see(self, classification: VisibilityClassification) -> bool {
        match self {
            ViewerClass::Owner => true,
            ViewerClass::HouseholdMember => {
                classification == VisibilityClassification::Public
                    || classification == VisibilityClassification::Household
            }
            ViewerClass::KnownGuest | ViewerClass::Unknown | ViewerClass::Nobody => {
                classification == VisibilityClassification::Public
            }
        }
    }

    /// Most restrictive of two viewer classes (spec §2.2).
    pub fn most_restrictive(a: ViewerClass, b: ViewerClass) -> ViewerClass {
        if a.restriction_level() >= b.restriction_level() { a } else { b }
    }
}

// ─── Attention / Interruption types ─────────────────────────────────────────

/// Interruption classification for attention management.
///
/// Lower numeric value = higher urgency (spec RFC 0010 §3.1).
/// Only the runtime may emit CRITICAL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum InterruptionClass {
    /// Always passes, bypasses quiet hours and budget. Runtime-only.
    Critical = 0,
    /// Passes quiet hours (by default). Subject to budget.
    High = 1,
    /// Standard. Filtered by quiet hours and attention budget.
    #[default]
    Normal = 2,
    /// Discarded during quiet hours (too stale to be useful later).
    Low = 3,
    /// Never interrupts. Always passes. Zero cost against budget.
    Silent = 4,
}

// ─── Override types (spec §7.1) ───────────────────────────────────────────────

/// The four override types used by arbitration levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverrideType {
    /// Action prevented. Agent is informed via structured error.
    Suppress,
    /// Action rerouted to a different target (e.g., input to chrome).
    Redirect,
    /// Action modified before commit (e.g., redaction placeholder).
    Transform,
    /// Action queued for later delivery when condition clears.
    Block,
}

// ─── Arbitration levels ───────────────────────────────────────────────────────

/// The seven arbitration levels, numbered 0 (highest) to 6 (lowest).
///
/// This ordering is doctrine and MUST NOT be modified (spec §1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum ArbitrationLevel {
    /// Level 0: Human Override — dismiss, safe mode, freeze, mute.
    HumanOverride = 0,
    /// Level 1: Safety — GPU health, scene integrity, degradation emergency.
    Safety = 1,
    /// Level 2: Privacy — viewer context, content classification, redaction.
    Privacy = 2,
    /// Level 3: Security — capability enforcement, lease validity, namespace isolation.
    Security = 3,
    /// Level 4: Attention — interruption class, quiet hours, attention budget.
    Attention = 4,
    /// Level 5: Resource — per-agent budget, degradation ladder, tile shedding.
    Resource = 5,
    /// Level 6: Content — zone contention, agent priority, z-order.
    Content = 6,
}

impl ArbitrationLevel {
    /// All seven levels in precedence order (highest first).
    pub const ALL: [ArbitrationLevel; 7] = [
        ArbitrationLevel::HumanOverride,
        ArbitrationLevel::Safety,
        ArbitrationLevel::Privacy,
        ArbitrationLevel::Security,
        ArbitrationLevel::Attention,
        ArbitrationLevel::Resource,
        ArbitrationLevel::Content,
    ];

    /// Numeric index (0-6).
    pub fn index(self) -> u8 {
        self as u8
    }

    /// Override types permitted at this level (spec §7.2).
    pub fn permitted_override_types(self) -> &'static [OverrideType] {
        match self {
            ArbitrationLevel::HumanOverride => &[
                OverrideType::Suppress,
                OverrideType::Redirect,
                OverrideType::Block,
            ],
            ArbitrationLevel::Safety => &[OverrideType::Suppress, OverrideType::Redirect],
            ArbitrationLevel::Privacy => &[OverrideType::Transform],
            ArbitrationLevel::Security => &[OverrideType::Suppress],
            ArbitrationLevel::Attention => &[OverrideType::Block],
            ArbitrationLevel::Resource => &[OverrideType::Suppress, OverrideType::Transform],
            ArbitrationLevel::Content => &[OverrideType::Suppress],
        }
    }
}

// ─── ArbitrationOutcome (spec §1.3) ──────────────────────────────────────────

/// The six possible outcomes of passing a mutation through the arbitration stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArbitrationOutcome {
    /// Mutation accepted, commit to scene.
    Commit,

    /// Mutation accepted but rendered with redaction placeholder.
    /// The mutation IS committed; rendering is filtered.
    CommitRedacted { redaction_reason: RedactionReason },

    /// Mutation deferred until queue condition clears.
    Queue {
        queue_reason: QueueReason,
        /// Monotonic microseconds of earliest possible delivery, if known.
        earliest_present_us: Option<u64>,
        /// Redaction flag: when delivered, render with placeholder.
        redacted: bool,
    },

    /// Mutation rejected. Agent receives structured error.
    Reject(ArbitrationError),

    /// Mutation shed by resource/degradation policy. No error to agent.
    /// Zone-state effects are applied but render output is omitted.
    Shed { degradation_level: u32 },

    /// Mutation blocked by human override (freeze). Queued for later.
    Blocked { block_reason: BlockReason },
}

/// Why content was redacted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RedactionReason {
    /// Content classification exceeds current viewer's access.
    ViewerClassInsufficient {
        required: VisibilityClassification,
        actual: ViewerClass,
    },
    /// Multi-viewer: most-restrictive rule applied.
    MultiViewerRestriction,
}

/// Why a mutation was queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueReason {
    QuietHours { window_end_us: Option<u64> },
    AttentionBudgetExhausted { per_agent: bool, per_zone: bool },
}

/// Why a mutation was blocked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// Scene is frozen by human override. Queued until unfreeze.
    Freeze,
}

/// Structured arbitration error (emitted on Reject).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbitrationError {
    pub code: ArbitrationErrorCode,
    pub agent_id: String,
    pub mutation_ref: SceneId,
    pub message: String,
    pub hint: Option<String>,
    /// Which arbitration level rejected (0-6).
    pub level: u8,
}

/// Error codes for arbitration rejections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArbitrationErrorCode {
    // Level 3: Security
    CapabilityDenied,
    CapabilityScopeInsufficient,
    NamespaceViolation,
    LeaseInvalid,
    // Level 6: Content
    ZoneEvictionDenied,
}

// ─── PolicyContext ─────────────────────────────────────────────────────────────

/// Read-only snapshot of all policy-relevant runtime state.
///
/// Policy evaluation is a pure function over PolicyContext — no side effects.
/// The system shell owns the state machine; the policy evaluator only reads.
#[derive(Clone, Debug)]
pub struct PolicyContext {
    /// Level 0: Human override state.
    pub override_state: OverrideState,
    /// Level 1: Safety state.
    pub safety_state: SafetyState,
    /// Level 2: Privacy context.
    pub privacy_context: PrivacyContext,
    /// Level 3: Security context.
    pub security_context: SecurityContext,
    /// Level 4: Attention context.
    pub attention_context: AttentionContext,
    /// Level 5: Resource context.
    pub resource_context: ResourceContext,
    /// Level 6: Content context.
    pub content_context: ContentContext,
}

/// Read-only human override state (Level 0).
#[derive(Clone, Debug, Default)]
pub struct OverrideState {
    /// Scene is currently frozen by human override.
    pub freeze_active: bool,
    /// Safe mode is currently active.
    pub safe_mode_active: bool,
    /// Current freeze duration in milliseconds (0 if not frozen).
    pub freeze_duration_ms: u64,
    /// Maximum allowed freeze duration in milliseconds.
    /// Default: 300_000 (5 minutes).
    pub max_freeze_duration_ms: u64,
}

/// Read-only safety state (Level 1).
#[derive(Clone, Debug, Default)]
pub struct SafetyState {
    /// GPU device is healthy (no DeviceError::Lost).
    pub gpu_healthy: bool,
    /// Scene graph integrity check passed.
    pub scene_graph_intact: bool,
    /// Frame-time p95 in microseconds over the rolling 10-frame window.
    pub frame_time_p95_us: u64,
    /// Emergency frame-time threshold in microseconds (default: 14_000 = 14ms).
    pub emergency_threshold_us: u64,
}

impl SafetyState {
    /// Returns true if the safety level would trigger an emergency response.
    pub fn is_emergency(&self) -> bool {
        !self.gpu_healthy
            || !self.scene_graph_intact
            || self.frame_time_p95_us > self.emergency_threshold_us
    }
}

/// Read-only privacy context (Level 2).
#[derive(Clone, Debug)]
pub struct PrivacyContext {
    /// Effective viewer class (most restrictive if multiple viewers).
    pub effective_viewer_class: ViewerClass,
    /// All active viewer classes (may be empty if no viewer present).
    pub viewer_classes: Vec<ViewerClass>,
    /// Redaction style configured in `[privacy]` config section.
    pub redaction_style: RedactionStyle,
}

impl Default for PrivacyContext {
    fn default() -> Self {
        Self {
            effective_viewer_class: ViewerClass::Nobody,
            viewer_classes: vec![],
            redaction_style: RedactionStyle::Pattern,
        }
    }
}

impl PrivacyContext {
    /// Compute effective viewer class from list (most restrictive).
    pub fn compute_effective(viewers: &[ViewerClass]) -> ViewerClass {
        viewers
            .iter()
            .copied()
            .reduce(ViewerClass::most_restrictive)
            .unwrap_or(ViewerClass::Nobody)
    }
}

/// Redaction placeholder style (spec §5.2, owned by `[privacy]` config).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedactionStyle {
    Pattern,
    AgentName,
    Icon,
    Blank,
}

/// Read-only security context (Level 3).
#[derive(Clone, Debug, Default)]
pub struct SecurityContext {
    /// Capabilities granted to the agent making this request.
    pub granted_capabilities: Vec<String>,
    /// Namespace of the agent making this request.
    pub agent_namespace: String,
    /// Lease is currently valid and active.
    pub lease_valid: bool,
    /// Lease ID for this request.
    pub lease_id: Option<SceneId>,
}

impl SecurityContext {
    /// Returns true if the agent holds the named capability.
    ///
    /// Supports `publish_zone:*` wildcard for zone publishing.
    pub fn has_capability(&self, required: &str) -> bool {
        for cap in &self.granted_capabilities {
            if cap == required {
                return true;
            }
            // Wildcard: publish_zone:* covers any publish_zone:<name>
            if cap == "publish_zone:*" && required.starts_with("publish_zone:") {
                return true;
            }
        }
        false
    }
}

/// Read-only attention context (Level 4).
#[derive(Clone, Debug, Default)]
pub struct AttentionContext {
    /// Is quiet hours mode currently active?
    pub quiet_hours_active: bool,
    /// Monotonic microseconds when the current quiet hours window ends.
    /// `None` if not in quiet hours, or if the end is not known.
    pub quiet_hours_end_us: Option<u64>,
    /// Per-agent interruption budget: interruptions in the rolling 60s window.
    pub per_agent_interruptions_last_60s: u32,
    /// Per-agent interruption limit (default: 20).
    pub per_agent_limit: u32,
    /// Per-zone interruption budget: interruptions in the rolling 60s window.
    pub per_zone_interruptions_last_60s: u32,
    /// Per-zone interruption limit (default: 10, 30 for Stack-policy zones).
    pub per_zone_limit: u32,
    /// The pass-through class threshold for quiet hours.
    /// Events at or below this urgency (numerically lower or equal) pass through quiet hours.
    /// Default: High (1).
    pub pass_through_class: InterruptionClass,
    /// The interruption class of the mutation being evaluated.
    pub interruption_class: InterruptionClass,
    /// Monotonic microseconds: when would the attention budget refill?
    pub budget_refill_us: Option<u64>,
}

impl AttentionContext {
    /// Returns true if the per-agent budget is exhausted (at or over limit).
    pub fn agent_budget_exhausted(&self) -> bool {
        self.per_agent_interruptions_last_60s >= self.per_agent_limit
    }

    /// Returns true if the per-zone budget is exhausted.
    pub fn zone_budget_exhausted(&self) -> bool {
        self.per_zone_interruptions_last_60s >= self.per_zone_limit
    }
}

/// Read-only resource context (Level 5).
#[derive(Clone, Debug, Default)]
pub struct ResourceContext {
    /// Current degradation level (0 = nominal, higher = more degraded).
    pub degradation_level: u32,
    /// Per-agent tile budget: current usage.
    pub tiles_used: u32,
    /// Per-agent tile budget: hard limit.
    pub tiles_limit: u32,
    /// Is this mutation's priority class subject to shedding at the current degradation level?
    pub should_shed: bool,
    /// Is this a transactional mutation (CreateTile, DeleteTile, LeaseRequest, LeaseRelease)?
    /// Transactional mutations are NEVER shed (spec §11.6).
    pub is_transactional: bool,
    /// Is the per-agent tile budget exceeded?
    pub budget_exceeded: bool,
    /// Resource budgets are paused (during freeze).
    pub budgets_paused: bool,
}

/// Read-only content context (Level 6).
#[derive(Clone, Debug, Default)]
pub struct ContentContext {
    /// Zone name being published to (if this is a zone publication).
    pub zone_name: Option<String>,
    /// Contention policy for the target zone.
    pub contention_policy: Option<ContentionPolicy>,
    /// Lease priority of the current agent (lower = higher priority; 0 = Critical).
    pub agent_lease_priority: u32,
    /// Lease priority of the current zone occupant, if any.
    pub occupant_lease_priority: Option<u32>,
    /// Number of current stack occupants (for Stack policy depth check).
    pub stack_depth: u32,
    /// Maximum stack depth for Stack policy zones.
    pub max_stack_depth: u32,
}

// ─── MutationKind ─────────────────────────────────────────────────────────────

/// The kind of mutation being evaluated, used to select the evaluation path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationKind {
    /// A zone publication (full evaluation path: 0→3→2→4→5→6).
    ZonePublication,
    /// A tile mutation (path: 3→5→6).
    TileMutation,
    /// A transactional mutation (CreateTile, DeleteTile, LeaseRequest, LeaseRelease).
    /// Path: 3→5→6 but NEVER shed at Level 5.
    Transactional,
}
