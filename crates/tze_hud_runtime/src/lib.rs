//! # tze_hud_runtime
//!
//! Runtime kernel for tze_hud. Orchestrates the frame pipeline:
//! input drain → local feedback → mutation intake → scene commit →
//! render encode → GPU submit → telemetry emit.

pub mod budget;
pub mod headless;
pub mod subscriptions;
pub mod event_bus;

pub use budget::{
    AgentResourceState, BudgetCheckOutcome, BudgetEnforcer, BudgetState,
    BudgetTelemetrySink, CollectingTelemetrySink, NoopTelemetrySink,
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
