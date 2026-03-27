//! # Quiet Hours Gate
//!
//! Evaluates whether a scene event should be delivered immediately, queued, or
//! discarded based on the current quiet-hours state and the event's
//! `InterruptionClass`.
//!
//! Spec: scene-events/spec.md §Requirement: Quiet Hours Enforcement, lines 69-89.
//! Spec: scene-events/spec.md §Requirement: Quiet Hours Queue Semantics, lines 92-104.
//!
//! ## Pipeline position
//!
//! The gate sits at Stage 3 (Policy Filtering) of the four-stage event
//! pipeline:
//!
//! ```text
//! Emission → Classification → [Quiet Hours Gate] → Delivery fan-out
//! ```
//!
//! ## Quiet-hours delivery rules
//!
//! | Class    | During quiet hours                                    |
//! |----------|------------------------------------------------------|
//! | CRITICAL | Always delivered immediately; never queued.          |
//! | HIGH     | Delivered when `pass_through_class <= HIGH` (default)|
//! | NORMAL   | Queued in per-zone FIFO; delivered on exit.          |
//! | LOW      | Discarded — too stale by quiet hours exit.           |
//! | SILENT   | Always delivered (never interrupts).                 |
//!
//! Quiet hours affect **delivery**, not generation.  Events are still created,
//! logged, and counted for telemetry.
//!
//! ## `pass_through_class`
//!
//! The runtime configuration exposes `pass_through_class` (default `HIGH`).
//! When `pass_through_class <= HIGH` (i.e. configured to HIGH or CRITICAL),
//! HIGH events are delivered immediately.  When set to NORMAL or lower,
//! HIGH events are deferred to the queue.

pub mod queue;

use std::collections::HashMap;
use uuid::Uuid;

use tze_hud_scene::events::{InterruptionClass, SceneEvent};

pub use queue::{ZoneContentionPolicy, ZoneQueue};

// ─── Gate decision ────────────────────────────────────────────────────────────

/// The quiet-hours gate's decision for a single event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision {
    /// Deliver immediately (CRITICAL, HIGH when pass-through, SILENT).
    Deliver,
    /// Queue for delivery when quiet hours end (NORMAL).
    Queue,
    /// Discard — event is too stale to be useful on quiet hours exit (LOW).
    Discard,
}

// ─── Gate configuration ───────────────────────────────────────────────────────

/// Configuration for the quiet hours gate.
#[derive(Clone, Debug)]
pub struct QuietHoursConfig {
    /// The maximum interruption class that passes through quiet hours
    /// immediately (default: `HIGH`).
    ///
    /// - If `pass_through_class <= HIGH` (i.e. HIGH or CRITICAL):
    ///   HIGH events are delivered immediately.
    /// - If `pass_through_class == NORMAL`:
    ///   HIGH events are deferred to the queue.
    ///
    /// CRITICAL always passes through regardless of this setting.
    pub pass_through_class: InterruptionClass,
}

impl Default for QuietHoursConfig {
    fn default() -> Self {
        Self {
            // Default: HIGH events pass through quiet hours.
            // Spec: scene-events/spec.md line 80.
            pass_through_class: InterruptionClass::High,
        }
    }
}

// ─── Gate ─────────────────────────────────────────────────────────────────────

/// Quiet-hours gate and per-zone queue manager.
///
/// Maintains quiet-hours state and per-zone queues for deferred events.
///
/// # Thread safety
///
/// This struct is not `Send + Sync` by itself.  The caller is responsible for
/// synchronising access (typically via a Mutex or channel on the event bus).
#[derive(Debug)]
pub struct QuietHoursGate {
    config: QuietHoursConfig,
    active: bool,
    /// Per-zone queues, keyed by zone UUID.
    zone_queues: HashMap<Uuid, ZoneQueue>,
}

impl QuietHoursGate {
    /// Create a new gate with default configuration.
    pub fn new() -> Self {
        Self {
            config: QuietHoursConfig::default(),
            active: false,
            zone_queues: HashMap::new(),
        }
    }

    /// Create a new gate with the given configuration.
    pub fn with_config(config: QuietHoursConfig) -> Self {
        Self {
            config,
            active: false,
            zone_queues: HashMap::new(),
        }
    }

    /// Whether quiet hours are currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Activate or deactivate quiet hours.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Register a zone with its contention policy and optional max depth.
    ///
    /// Must be called before any events for that zone are processed.
    /// If the zone is already registered, this is a no-op.
    pub fn register_zone(
        &mut self,
        zone_id: Uuid,
        policy: ZoneContentionPolicy,
        max_depth: Option<usize>,
    ) {
        self.zone_queues.entry(zone_id).or_insert_with(|| {
            let depth = max_depth.unwrap_or(100);
            ZoneQueue::new(policy, depth)
        });
    }

    /// Evaluate the gate decision for `event`.
    ///
    /// - If quiet hours are **not** active: always returns `Deliver`.
    /// - If quiet hours are **active**: applies the delivery rules per spec
    ///   lines 69-89.
    ///
    /// This method does NOT enqueue the event.  If the decision is `Queue`,
    /// the caller should call [`Self::enqueue`] with the same event.
    ///
    /// # Spec
    ///
    /// scene-events/spec.md lines 69-89.
    pub fn evaluate(&self, event: &SceneEvent) -> GateDecision {
        if !self.active {
            return GateDecision::Deliver;
        }

        match event.interruption_class {
            // CRITICAL: always delivered immediately (spec line 75-76).
            InterruptionClass::Critical => GateDecision::Deliver,

            // SILENT: always passes, never interrupts (spec line 64-65).
            InterruptionClass::Silent => GateDecision::Deliver,

            // HIGH: passes when pass_through_class <= HIGH (spec line 79-80).
            // "pass_through_class <= HIGH" means pass_through is HIGH or CRITICAL
            // (lower numeric value = higher urgency).
            InterruptionClass::High => {
                if self.config.pass_through_class <= InterruptionClass::High {
                    GateDecision::Deliver
                } else {
                    GateDecision::Queue
                }
            }

            // NORMAL: queued (spec line 83-84).
            InterruptionClass::Normal => GateDecision::Queue,

            // LOW: discarded (spec line 87-88).
            InterruptionClass::Low => GateDecision::Discard,
        }
    }

    /// Enqueue a `Queue`-decided event into the zone's per-zone queue.
    ///
    /// If the zone is not yet registered, a default FIFO queue with depth 100
    /// is created automatically.
    ///
    /// The event is always enqueued. If the zone's queue was at capacity, the
    /// oldest event is silently dropped to make room (overflow drops oldest-first).
    pub fn enqueue(&mut self, zone_id: Uuid, event: SceneEvent) {
        let queue = self
            .zone_queues
            .entry(zone_id)
            .or_insert_with(|| ZoneQueue::new_with_default_depth(ZoneContentionPolicy::Fifo));
        queue.push(event);
    }

    /// Drain all per-zone queues and return the events to deliver.
    ///
    /// Called when quiet hours end.  Each zone's queue is drained according
    /// to its contention policy:
    /// - `Fifo` → all events in enqueue order.
    /// - `LatestWins` → only the last event.
    ///
    /// After draining, all queues are empty.
    ///
    /// The returned events preserve zone-local FIFO ordering within each zone,
    /// but ordering across zones is determined by the order zones appear in the
    /// internal map.  Callers that need strict global ordering should sort by
    /// `timestamp_mono_us`.
    ///
    /// Spec: scene-events/spec.md lines 92-104.
    pub fn drain_queues(&mut self) -> Vec<SceneEvent> {
        let mut events: Vec<SceneEvent> = Vec::new();
        for queue in self.zone_queues.values_mut() {
            events.extend(queue.drain());
        }
        events
    }

    /// Number of queued events for a specific zone.
    pub fn zone_queue_len(&self, zone_id: Uuid) -> usize {
        self.zone_queues
            .get(&zone_id)
            .map(|q: &ZoneQueue| q.len())
            .unwrap_or(0)
    }

    /// Total number of queued events across all zones.
    pub fn total_queued(&self) -> usize {
        self.zone_queues.values().map(|q: &ZoneQueue| q.len()).sum()
    }
}

impl Default for QuietHoursGate {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::events::{EventPayload, EventSource, SceneEventBuilder};

    fn make_event_with_class(class: InterruptionClass, seq: u64) -> SceneEvent {
        SceneEventBuilder::new(
            "scene.zone.occupancy_changed",
            class,
            EventPayload::ZoneOccupancyChanged {
                zone_id: Uuid::nil(),
                occupant_count: 0,
            },
        )
        .source(EventSource::system())
        .sequence(seq)
        .build()
    }

    // ── No quiet hours ────────────────────────────────────────────────────────

    /// When quiet hours are NOT active all classes are delivered immediately.
    #[test]
    fn all_classes_deliver_when_quiet_hours_inactive() {
        let gate = QuietHoursGate::new();
        for cls in [
            InterruptionClass::Critical,
            InterruptionClass::High,
            InterruptionClass::Normal,
            InterruptionClass::Low,
            InterruptionClass::Silent,
        ] {
            let evt = make_event_with_class(cls, 1);
            assert_eq!(
                gate.evaluate(&evt),
                GateDecision::Deliver,
                "class {cls:?} should deliver when quiet hours are inactive",
            );
        }
    }

    // ── CRITICAL during quiet hours ───────────────────────────────────────────

    /// WHEN quiet hours are active and a CRITICAL event is generated THEN it
    /// MUST be delivered immediately (spec line 75-76).
    #[test]
    fn critical_bypasses_quiet_hours() {
        let mut gate = QuietHoursGate::new();
        gate.set_active(true);

        let evt = make_event_with_class(InterruptionClass::Critical, 1);
        assert_eq!(gate.evaluate(&evt), GateDecision::Deliver);
    }

    // ── SILENT during quiet hours ─────────────────────────────────────────────

    /// WHEN quiet hours are active and an event has class SILENT THEN it MUST
    /// be delivered immediately (spec lines 64-65).
    #[test]
    fn silent_bypasses_quiet_hours() {
        let mut gate = QuietHoursGate::new();
        gate.set_active(true);

        let evt = make_event_with_class(InterruptionClass::Silent, 1);
        assert_eq!(gate.evaluate(&evt), GateDecision::Deliver);
    }

    // ── HIGH during quiet hours (default config) ──────────────────────────────

    /// WHEN quiet hours are active with default pass_through_class=HIGH, and a
    /// HIGH event is generated THEN it MUST be delivered immediately (spec line 80).
    #[test]
    fn high_passes_quiet_hours_with_default_config() {
        let mut gate = QuietHoursGate::new(); // default: pass_through = HIGH
        gate.set_active(true);

        let evt = make_event_with_class(InterruptionClass::High, 1);
        assert_eq!(gate.evaluate(&evt), GateDecision::Deliver);
    }

    /// WHEN pass_through_class is set to NORMAL (more restrictive), HIGH events
    /// are deferred to the queue.
    #[test]
    fn high_deferred_when_pass_through_class_is_normal() {
        let config = QuietHoursConfig {
            pass_through_class: InterruptionClass::Normal,
        };
        let mut gate = QuietHoursGate::with_config(config);
        gate.set_active(true);

        let evt = make_event_with_class(InterruptionClass::High, 1);
        assert_eq!(gate.evaluate(&evt), GateDecision::Queue);
    }

    // ── NORMAL during quiet hours ─────────────────────────────────────────────

    /// WHEN quiet hours are active and a NORMAL zone update event occurs THEN
    /// it MUST be queued (spec lines 83-84).
    #[test]
    fn normal_queued_during_quiet_hours() {
        let mut gate = QuietHoursGate::new();
        gate.set_active(true);

        let evt = make_event_with_class(InterruptionClass::Normal, 1);
        assert_eq!(gate.evaluate(&evt), GateDecision::Queue);
    }

    // ── LOW during quiet hours ────────────────────────────────────────────────

    /// WHEN quiet hours are active and a LOW-class event is generated THEN it
    /// MUST be discarded (spec lines 87-88).
    #[test]
    fn low_discarded_during_quiet_hours() {
        let mut gate = QuietHoursGate::new();
        gate.set_active(true);

        let evt = make_event_with_class(InterruptionClass::Low, 1);
        assert_eq!(gate.evaluate(&evt), GateDecision::Discard);
    }

    // ── Enqueue and drain ─────────────────────────────────────────────────────

    /// Events can be enqueued and then drained on quiet hours exit.
    #[test]
    fn enqueue_and_drain_fifo_zone() {
        let mut gate = QuietHoursGate::new();
        let zone_id = Uuid::now_v7();
        gate.register_zone(zone_id, ZoneContentionPolicy::Fifo, None);
        gate.set_active(true);

        for seq in 1..=3 {
            let evt = make_event_with_class(InterruptionClass::Normal, seq);
            gate.enqueue(zone_id, evt);
        }
        assert_eq!(gate.zone_queue_len(zone_id), 3);

        let drained = gate.drain_queues();
        assert_eq!(drained.len(), 3);
        assert_eq!(gate.total_queued(), 0);
    }

    /// WHEN a LatestWins zone receives N queued publishes THEN on drain, only
    /// the last is returned (spec line 99).
    #[test]
    fn drain_latest_wins_zone_returns_only_last() {
        let mut gate = QuietHoursGate::new();
        let zone_id = Uuid::now_v7();
        gate.register_zone(zone_id, ZoneContentionPolicy::LatestWins, None);
        gate.set_active(true);

        for seq in 1..=10 {
            let evt = make_event_with_class(InterruptionClass::Normal, seq);
            gate.enqueue(zone_id, evt);
        }

        let drained = gate.drain_queues();
        assert_eq!(
            drained.len(),
            1,
            "LatestWins must deliver only the last event"
        );
        assert_eq!(drained[0].sequence, 10);
    }

    /// Queue depth limit: overflow drops oldest (spec line 103).
    #[test]
    fn zone_queue_overflow_drops_oldest() {
        let mut gate = QuietHoursGate::new();
        let zone_id = Uuid::now_v7();
        gate.register_zone(zone_id, ZoneContentionPolicy::Fifo, Some(3));
        gate.set_active(true);

        for seq in 1..=4 {
            let evt = make_event_with_class(InterruptionClass::Normal, seq);
            gate.enqueue(zone_id, evt);
        }
        // max_depth is 3; seq=1 must have been dropped.
        assert_eq!(gate.zone_queue_len(zone_id), 3);

        let drained = gate.drain_queues();
        let seqs: Vec<u64> = drained.iter().map(|e| e.sequence).collect();
        assert_eq!(seqs, vec![2, 3, 4]);
    }

    /// `evaluate` returns Deliver for all classes when quiet hours deactivated.
    #[test]
    fn deactivating_quiet_hours_resumes_delivery() {
        let mut gate = QuietHoursGate::new();
        gate.set_active(true);
        gate.set_active(false); // deactivate

        let evt = make_event_with_class(InterruptionClass::Normal, 1);
        assert_eq!(gate.evaluate(&evt), GateDecision::Deliver);
    }
}
