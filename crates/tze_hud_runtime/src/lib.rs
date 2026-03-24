//! # tze_hud_runtime
//!
//! Runtime kernel for tze_hud. Orchestrates the 8-stage frame pipeline:
//!
//! | Stage | Name               | Thread     | Budget (p99) |
//! |-------|--------------------|------------|-------------|
//! | 1     | Input Drain        | Main       | < 500µs     |
//! | 2     | Local Feedback     | Main       | < 500µs     |
//! | 3     | Mutation Intake    | Compositor | < 1ms       |
//! | 4     | Scene Commit       | Compositor | < 1ms       |
//! | 5     | Layout Resolve     | Compositor | < 1ms       |
//! | 6     | Render Encode      | Compositor | < 4ms       |
//! | 7     | GPU Submit+Present | Comp+Main  | < 8ms       |
//! | 8     | Telemetry Emit     | Telemetry  | < 200µs     |
//!
//! See `pipeline.rs` for the `FramePipeline` orchestrator and `HitTestSnapshot`
//! (ArcSwap-backed lock-free tile bounds for Stage 2).
//!
//! ## Bead 3: Interruption classification and quiet hours
//!
//! - [`attention_budget`] — per-agent and per-zone rolling interruption budgets,
//!   80% warning, exhaustion coalescing, earned-urgency tracker.
//! - [`quiet_hours`] — quiet-hours gate (deliver / queue / discard) and
//!   per-zone FIFO queues with LatestWins coalescing.

pub mod agent_events;
pub mod budget;
pub mod degradation;
pub mod headless;
pub mod subscriptions;
pub mod event_bus;
pub mod quiet_hours;
pub mod attention_budget;
pub mod shell;
pub mod tab_switch_trigger;
pub mod pipeline;

pub use agent_events::{
    AgentEventHandler, EmissionError, EmissionOutcome, EmissionResult,
    MAX_PAYLOAD_BYTES, DEFAULT_MAX_EVENTS_PER_SECOND,
};
pub use agent_events::rate_limiter::AgentEventRateLimiter;
pub use budget::{
    AgentResourceState, BudgetCheckOutcome, BudgetEnforcer, BudgetState,
    BudgetTelemetrySink, CollectingTelemetrySink, NoopTelemetrySink,
};
pub use degradation::{
    DegradationConfig, DegradationController, DegradationLevel, TileDescriptor,
};
pub use headless::HeadlessRuntime;
pub use subscriptions::{
    AgentSubscriptions, Subscription, SubscriptionChangeOutcome, SubscriptionRegistry,
    CATEGORY_AGENT_EVENTS, CATEGORY_ATTENTION_EVENTS, CATEGORY_DEGRADATION_NOTICES,
    CATEGORY_FOCUS_EVENTS, CATEGORY_INPUT_EVENTS, CATEGORY_LEASE_CHANGES,
    CATEGORY_SCENE_TOPOLOGY, CATEGORY_TELEMETRY_FRAMES, CATEGORY_ZONE_EVENTS,
    MAX_SUBSCRIPTIONS_PER_AGENT, MANDATORY_CATEGORIES,
    category_prefix, required_capability,
};
pub use event_bus::{
    AggregateRateLimiter, ClassifiedEvent, EventBus, InterruptionClass,
    SubscriberQueue, AGGREGATE_RATE_CAP,
};
pub use quiet_hours::{GateDecision, QuietHoursConfig, QuietHoursGate, ZoneContentionPolicy, ZoneQueue};
pub use attention_budget::{
    AttentionBudgetOutcome, AttentionBudgetTracker, EarnedUrgencyConfig, EarnedUrgencyTracker,
    UrgencyRecord, DEFAULT_AGENT_BUDGET, DEFAULT_ZONE_BUDGET, DEFAULT_STACK_ZONE_BUDGET,
    WARNING_FRACTION, ROLLING_WINDOW_US,
};
pub use shell::chrome::{
    AgentVisibleTopology, AuditPayload, AuditTrigger, ChromeLayout,
    ChromeRenderer, ChromeShortcut, ChromeState, ChromeTab, CollectingAuditSink,
    DiagnosticSnapshot, DismissTileResult, NoopAuditSink, RevokeReason, SafeModeEntryReason,
    ShellAuditEvent, ShellAuditSink, ShortcutResult, SystemHealth, TabBarPosition,
    ViewerClass, ViewerClassTransition, collect_diagnostic, handle_shortcut,
    strip_chrome_from_topology,
};
pub use shell::safe_mode::{
    classify_safe_mode_input, LeaseResumeInfo, SafeModeController, SafeModeEntryResult,
    SafeModeExitResult, SafeModeInput, SafeModeInputResult, ShellOverrideState,
};
// ChromeDrawCmd is defined in tze_hud_compositor to avoid circular deps.
pub use tze_hud_compositor::ChromeDrawCmd;
pub use tab_switch_trigger::{
    ACTIVE_TAB_CHANGED_EVENT_TYPE, AttentionGate, BlockingGate,
    PermissiveGate, TabSwitchOutcome, TabSwitchTrigger,
};
pub use shell::{
    classify_mutation_batch, EnqueueResult, FreezeManager, FreezeQueue, FreezeState,
    MutationTrafficClass, QueuedMutation, DEFAULT_AUTO_UNFREEZE_MS,
    DEFAULT_FREEZE_QUEUE_CAPACITY, QUEUE_PRESSURE_FRACTION,
};

pub use shell::redaction::{
    ContentClassification,
    RedactionStyle,
    RedactionFrame,
    TileRedactionState,
    is_tile_redacted,
    hit_regions_enabled,
    build_redaction_cmds,
    PATTERN_CELL_PX,
    MAX_PATTERN_ACCENT_RECTS,
    REDACTION_BLANK_COLOR,
    REDACTION_PATTERN_BASE,
    REDACTION_PATTERN_ACCENT,
};

pub use pipeline::{
    FramePipeline, HitTestSnapshot, TileBoundsEntry,
    STAGE1_BUDGET_US, STAGE2_BUDGET_US, STAGE12_COMBINED_BUDGET_US,
    STAGE3_BUDGET_US, STAGE4_BUDGET_US, STAGE5_BUDGET_US,
    STAGE6_BUDGET_US, STAGE7_BUDGET_US, STAGE8_BUDGET_US,
    TOTAL_PIPELINE_BUDGET_US, INPUT_TO_LOCAL_ACK_BUDGET_US,
    INPUT_TO_SCENE_COMMIT_BUDGET_US, INPUT_TO_NEXT_PRESENT_BUDGET_US,
};
