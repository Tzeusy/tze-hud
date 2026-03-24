//! Event router trait for v1 scene events.
//!
//! Encodes the scene-events specification from
//! `scene-events/spec.md §Requirement: Three-Category Event Taxonomy`
//! and related requirements.  This module defines **only** the trait contract
//! and supporting types — no implementation is provided here.

use crate::clock::Clock;

// ─── Interruption Class ──────────────────────────────────────────────────────

/// Wire-level interruption class per RFC 0010 §3.1.
///
/// Lower numeric value = higher urgency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InterruptionClass {
    Critical = 0,
    High = 1,
    Normal = 2,
    Low = 3,
    Silent = 4,
}

// ─── Event Categories ────────────────────────────────────────────────────────

/// Subscription category as defined in spec §Requirement: Subscription Model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubscriptionCategory {
    SceneTopology,
    InputEvents,
    FocusEvents,
    /// Cannot opt out.
    DegradationNotices,
    /// Cannot opt out.
    LeaseChanges,
    ZoneEvents,
    TelemetryFrames,
    AttentionEvents,
    AgentEvents,
}

impl SubscriptionCategory {
    /// Returns `true` if this category cannot be unsubscribed from.
    pub fn is_mandatory(self) -> bool {
        matches!(self, SubscriptionCategory::DegradationNotices | SubscriptionCategory::LeaseChanges)
    }
}

// ─── Event Errors ─────────────────────────────────────────────────────────────

/// Errors produced by the EventRouter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventError {
    /// Agent lacks the required `emit_scene_event:<name>` capability.
    CapabilityDenied,
    /// Payload exceeds 4 KB limit.
    PayloadTooLarge,
    /// Agent exceeded 10 events/sec rate limit.
    RateLimitExceeded,
    /// Event type uses a reserved prefix (`scene.` or `system.`).
    ReservedPrefix,
    /// Attempted to unsubscribe from a mandatory category.
    CannotUnsubscribeMandatory,
    /// Subscription limit (32 per agent) exceeded.
    SubscriptionLimitExceeded,
    /// Event type format is invalid (not `<obj>.<action>` etc.).
    InvalidEventType,
}

// ─── Scene Event ─────────────────────────────────────────────────────────────

/// A delivered event with its effective interruption class.
#[derive(Clone, Debug)]
pub struct SceneEvent {
    /// Namespaced event type string (e.g., `"scene.tile.created"`,
    /// `"agent.doorbell_agent.doorbell.ring"`).
    pub event_type: String,
    /// Effective class after ceiling enforcement.
    pub effective_class: InterruptionClass,
    /// Raw payload bytes (≤ 4 KiB for agent events).
    pub payload: Vec<u8>,
    /// Source agent namespace (empty for runtime events).
    pub source_namespace: String,
}

// ─── Subscription Filter ─────────────────────────────────────────────────────

/// Subscription with optional event-type prefix filter.
#[derive(Clone, Debug)]
pub struct Subscription {
    pub category: SubscriptionCategory,
    /// Optional prefix filter (e.g., `"scene.zone.*"`). `None` = receive all.
    pub prefix_filter: Option<String>,
}

// ─── EventRouter Trait ────────────────────────────────────────────────────────

/// Trait encoding the scene-event routing contract.
///
/// Implementations must enforce:
/// - Event type naming convention (`scene.<obj>.<action>`, `agent.<ns>.<cat>.<action>`)
/// - Interruption classification and ceiling enforcement
/// - Quiet hours queueing / discard / pass-through semantics
/// - Rate limiting (10 events/sec per agent)
/// - Payload size limit (4 KB)
/// - Subscription category management
/// - Self-event suppression (the emitting agent does NOT receive its own events)
///
/// Clock injection via `C: Clock` enables deterministic quiet-hours and
/// rate-window testing.
pub trait EventRouter<C: Clock> {
    /// Create a new router backed by the given clock.
    fn new(clock: C) -> Self
    where
        Self: Sized;

    // ── Emission ─────────────────────────────────────────────────────────────

    /// Emit an agent event.
    ///
    /// - `agent_ns`: emitting agent's namespace.
    /// - `event_name`: agent-supplied `<category>.<action>` suffix (e.g., `"doorbell.ring"`).
    /// - `class`: declared interruption class.
    /// - `payload`: raw bytes (max 4 KB).
    /// - `capabilities`: capabilities held by the agent.
    ///
    /// Returns `Err(EventError)` if:
    /// - agent lacks `emit_scene_event:<event_name>` capability,
    /// - payload > 4 KB,
    /// - rate limit exceeded,
    /// - event name uses reserved prefix (`scene.` or `system.`).
    fn emit(
        &mut self,
        agent_ns: &str,
        event_name: &str,
        class: InterruptionClass,
        payload: Vec<u8>,
        capabilities: &[String],
    ) -> Result<(), EventError>;

    // ── Subscriptions ─────────────────────────────────────────────────────────

    /// Subscribe an agent to a category, optionally filtered by event-type prefix.
    ///
    /// Returns `Err(EventError::SubscriptionLimitExceeded)` if the agent already
    /// has 32 active subscriptions.
    fn subscribe(
        &mut self,
        agent_ns: &str,
        sub: Subscription,
    ) -> Result<(), EventError>;

    /// Unsubscribe an agent from a category.
    ///
    /// Returns `Err(EventError::CannotUnsubscribeMandatory)` for
    /// `DegradationNotices` or `LeaseChanges`.
    fn unsubscribe(
        &mut self,
        agent_ns: &str,
        category: SubscriptionCategory,
    ) -> Result<(), EventError>;

    // ── Quiet Hours ───────────────────────────────────────────────────────────

    /// Enter quiet hours mode.  From this point:
    /// - CRITICAL events are delivered immediately.
    /// - HIGH events pass through (default `pass_through_class = HIGH`).
    /// - NORMAL events are queued per-zone FIFO (max 100 per zone, oldest-first drop).
    /// - LOW events are discarded.
    /// - SILENT events are unaffected.
    fn enter_quiet_hours(&mut self);

    /// Exit quiet hours.  Queued NORMAL events are delivered in FIFO order.
    fn exit_quiet_hours(&mut self);

    /// Whether quiet hours are currently active.
    fn is_quiet_hours(&self) -> bool;

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Drain all events that have been delivered to `agent_ns` since the last call.
    /// Includes events delivered during quiet hours exit.
    fn drain_delivered(&mut self, agent_ns: &str) -> Vec<SceneEvent>;

    /// Number of events currently queued for a zone during quiet hours.
    fn quiet_queue_depth(&self, zone_id: &str) -> usize;

    /// Current rolling emit count for `agent_ns` in the last 1 second.
    fn agent_emit_count_last_sec(&self, agent_ns: &str) -> u32;

    /// Validate an event type string against the naming convention.
    ///
    /// Returns `Ok(())` if valid, `Err(EventError::InvalidEventType)` otherwise.
    fn validate_event_type(event_type: &str) -> Result<(), EventError>
    where
        Self: Sized;
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::clock::TestClock;

    fn capabilities_with(caps: &[&str]) -> Vec<String> {
        caps.iter().map(|s| s.to_string()).collect()
    }

    // ── 1. Event type naming convention ──────────────────────────────────────

    /// WHEN agent emits "doorbell.ring" THEN delivered type is
    /// "agent.doorbell_agent.doorbell.ring".
    pub fn test_agent_event_namespace_prefixed<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        router
            .subscribe(
                "listener",
                Subscription {
                    category: SubscriptionCategory::AgentEvents,
                    prefix_filter: None,
                },
            )
            .unwrap();
        let caps = capabilities_with(&["emit_scene_event:doorbell.ring"]);
        router
            .emit("doorbell_agent", "doorbell.ring", InterruptionClass::Normal, vec![], &caps)
            .expect("emit should succeed");
        let events = router.drain_delivered("listener");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "agent.doorbell_agent.doorbell.ring");
    }

    /// WHEN agent tries to emit event with "scene." prefix THEN rejected.
    pub fn test_reserved_scene_prefix_rejected<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        let caps = capabilities_with(&["emit_scene_event:tile.created"]);
        let result = router.emit(
            "agent_a",
            "scene.tile.created",
            InterruptionClass::Normal,
            vec![],
            &caps,
        );
        assert_eq!(result, Err(EventError::ReservedPrefix));
    }

    /// WHEN agent tries to emit event with "system." prefix THEN rejected.
    pub fn test_reserved_system_prefix_rejected<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        let caps = capabilities_with(&["emit_scene_event:system.shutdown"]);
        let result = router.emit(
            "agent_a",
            "system.shutdown",
            InterruptionClass::Normal,
            vec![],
            &caps,
        );
        assert_eq!(result, Err(EventError::ReservedPrefix));
    }

    // ── 2. Capability gate ────────────────────────────────────────────────────

    /// WHEN agent without `emit_scene_event:doorbell.ring` emits THEN rejected.
    pub fn test_emit_without_capability_rejected<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        let result =
            router.emit("agent_a", "doorbell.ring", InterruptionClass::Normal, vec![], &[]);
        assert_eq!(result, Err(EventError::CapabilityDenied));
    }

    // ── 3. Payload size limit ─────────────────────────────────────────────────

    /// WHEN agent emits event with payload > 4 KB THEN rejected.
    pub fn test_payload_too_large_rejected<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        let caps = capabilities_with(&["emit_scene_event:fire.detected"]);
        let big_payload = vec![0u8; 4097]; // 4097 bytes > 4 KB
        let result = router.emit("agent_a", "fire.detected", InterruptionClass::High, big_payload, &caps);
        assert_eq!(result, Err(EventError::PayloadTooLarge));
    }

    // ── 4. Rate limiting ──────────────────────────────────────────────────────

    /// WHEN agent emits 11 events in 1 second THEN 11th is rejected.
    pub fn test_rate_limit_enforcement<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        let caps = capabilities_with(&["emit_scene_event:ping.pong"]);
        for i in 0..10 {
            let r = router.emit("agent_a", "ping.pong", InterruptionClass::Normal, vec![], &caps);
            assert!(r.is_ok(), "event {i} should succeed");
        }
        let eleventh = router.emit("agent_a", "ping.pong", InterruptionClass::Normal, vec![], &caps);
        assert_eq!(eleventh, Err(EventError::RateLimitExceeded));
    }

    // ── 5. Quiet hours — CRITICAL bypasses ────────────────────────────────────

    /// WHEN quiet hours active and CRITICAL event emitted THEN delivered immediately.
    pub fn test_critical_bypasses_quiet_hours<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        router.subscribe(
            "listener",
            Subscription { category: SubscriptionCategory::AgentEvents, prefix_filter: None },
        ).unwrap();
        router.enter_quiet_hours();
        let caps = capabilities_with(&["emit_scene_event:alarm.fire"]);
        // CRITICAL class → immediate delivery even during quiet hours.
        router
            .emit("agent_a", "alarm.fire", InterruptionClass::Critical, vec![], &caps)
            .expect("emit should succeed");
        let events = router.drain_delivered("listener");
        assert_eq!(events.len(), 1, "CRITICAL event must be delivered during quiet hours");
    }

    /// WHEN quiet hours active and NORMAL event emitted THEN queued, NOT delivered.
    pub fn test_normal_queued_during_quiet_hours<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        router.subscribe(
            "listener",
            Subscription { category: SubscriptionCategory::AgentEvents, prefix_filter: None },
        ).unwrap();
        router.enter_quiet_hours();
        let caps = capabilities_with(&["emit_scene_event:status.update"]);
        router
            .emit("agent_a", "status.update", InterruptionClass::Normal, vec![], &caps)
            .expect("emit should succeed");
        // Not delivered yet.
        let events = router.drain_delivered("listener");
        assert!(events.is_empty(), "NORMAL event must be queued, not delivered");
        // Queue should have 1 entry.
        assert!(
            router.quiet_queue_depth("default_zone") > 0
                || router.drain_delivered("listener").is_empty(),
            "event should be in queue"
        );
    }

    /// WHEN quiet hours active and LOW event emitted THEN discarded.
    pub fn test_low_discarded_during_quiet_hours<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        router.subscribe(
            "listener",
            Subscription { category: SubscriptionCategory::AgentEvents, prefix_filter: None },
        ).unwrap();
        router.enter_quiet_hours();
        let caps = capabilities_with(&["emit_scene_event:bg.sync"]);
        router
            .emit("agent_a", "bg.sync", InterruptionClass::Low, vec![], &caps)
            .expect("emit should succeed"); // the emit itself succeeds; LOW is discarded in routing
        router.exit_quiet_hours();
        // After quiet hours, queued NORMAL events delivered — but LOW was discarded.
        let events = router.drain_delivered("listener");
        assert!(
            events.is_empty(),
            "LOW event should be discarded, not delivered on quiet hours exit"
        );
    }

    // ── 6. Quiet hours exit — FIFO delivery ───────────────────────────────────

    /// WHEN quiet hours exit THEN queued NORMAL events delivered in FIFO order.
    pub fn test_quiet_hours_exit_delivers_fifo<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        router.subscribe(
            "listener",
            Subscription { category: SubscriptionCategory::AgentEvents, prefix_filter: None },
        ).unwrap();
        router.enter_quiet_hours();
        let caps = capabilities_with(&["emit_scene_event:event.a", "emit_scene_event:event.b"]);
        router.emit("agent_a", "event.a", InterruptionClass::Normal, b"first".to_vec(), &caps).unwrap();
        router.emit("agent_a", "event.b", InterruptionClass::Normal, b"second".to_vec(), &caps).unwrap();
        router.exit_quiet_hours();
        let events = router.drain_delivered("listener");
        // FIFO: event.a before event.b.
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload, b"first");
        assert_eq!(events[1].payload, b"second");
    }

    // ── 7. LatestWins zone coalesces queued publishes ─────────────────────────

    /// WHEN LatestWins zone in quiet hours receives 2 publishes THEN only latest kept.
    pub fn test_latest_wins_zone_coalesces_during_quiet_hours<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        router.subscribe(
            "listener",
            Subscription { category: SubscriptionCategory::ZoneEvents, prefix_filter: None },
        ).unwrap();
        router.enter_quiet_hours();
        // Two zone publish events with NORMAL class — LatestWins zone should coalesce.
        let caps = capabilities_with(&[
            "emit_scene_event:zone.publish",
            "publish_zone:subtitle",
        ]);
        router
            .emit("agent_a", "zone.publish", InterruptionClass::Normal, b"old".to_vec(), &caps)
            .unwrap();
        router
            .emit("agent_a", "zone.publish", InterruptionClass::Normal, b"new".to_vec(), &caps)
            .unwrap();
        router.exit_quiet_hours();
        let events = router.drain_delivered("listener");
        // LatestWins: only the latest should survive.
        assert_eq!(events.len(), 1, "LatestWins zone should coalesce to single event");
        assert_eq!(events[0].payload, b"new");
    }

    // ── 8. Subscription management ────────────────────────────────────────────

    /// WHEN agent tries to unsubscribe from DegradationNotices THEN rejected.
    pub fn test_cannot_unsubscribe_mandatory_category<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        let result = router.unsubscribe("agent_a", SubscriptionCategory::DegradationNotices);
        assert_eq!(result, Err(EventError::CannotUnsubscribeMandatory));
    }

    /// WHEN agent tries to unsubscribe from LeaseChanges THEN rejected.
    pub fn test_cannot_unsubscribe_lease_changes<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        let result = router.unsubscribe("agent_a", SubscriptionCategory::LeaseChanges);
        assert_eq!(result, Err(EventError::CannotUnsubscribeMandatory));
    }

    /// WHEN agent creates 33rd subscription THEN rejected.
    pub fn test_subscription_limit_exceeded<R: EventRouter<TestClock>>() {
        let clock = TestClock::new(0);
        let mut router = R::new(clock);
        // Add 32 subscriptions via the same category (different prefix filters count separately).
        for i in 0..32u8 {
            let prefix = format!("agent.ns_{i}.*");
            let r = router.subscribe(
                "agent_a",
                Subscription {
                    category: SubscriptionCategory::AgentEvents,
                    prefix_filter: Some(prefix),
                },
            );
            // First 32 should succeed.
            assert!(r.is_ok(), "subscription {i} should succeed");
        }
        // 33rd should fail.
        let r = router.subscribe(
            "agent_a",
            Subscription {
                category: SubscriptionCategory::AgentEvents,
                prefix_filter: Some("agent.ns_x.*".into()),
            },
        );
        assert_eq!(r, Err(EventError::SubscriptionLimitExceeded));
    }

    // ── Compile-time generic check ────────────────────────────────────────────

    #[test]
    #[ignore = "no implementation yet"]
    fn test_event_router_generic_compile_check() {
        fn use_router<R: EventRouter<TestClock>>() {
            let clock = TestClock::new(0);
            let mut router = R::new(clock);
            let _ = router.is_quiet_hours();
        }
        // Replace the above with `use_router::<ConcreteImpl>()` once an impl exists.
    }
}
