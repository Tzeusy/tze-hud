//! # Quiet Hours Queue
//!
//! Per-zone FIFO queue that holds events deferred during quiet hours.
//!
//! Spec: scene-events/spec.md §Requirement: Quiet Hours Queue Semantics, lines 92-104.
//!
//! ## Semantics
//!
//! - Events accumulate in a per-zone FIFO queue.
//! - On quiet hours exit, events are dequeued and delivered in FIFO order.
//! - Zones with `LatestWins` contention policy coalesce their queued publishes
//!   so that only the last entry survives to delivery.
//! - Maximum depth per zone is configurable (default 100).  On overflow,
//!   the **oldest** entry is dropped to make room.
//!
//! ## What does NOT live here
//!
//! The actual delivery loop (draining the queue when quiet hours end) is
//! performed by `quiet_hours::Gate::drain_queue`, not by this module.

use std::collections::VecDeque;

use tze_hud_scene::events::SceneEvent;

// ─── Contention policy ────────────────────────────────────────────────────────

/// Zone contention policy — controls how queued zone publishes are delivered.
///
/// Spec: scene-events/spec.md line 99 (LatestWins scenario).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneContentionPolicy {
    /// Deliver all queued events in FIFO order.
    Fifo,
    /// Deliver only the last queued event; earlier ones are discarded.
    ///
    /// Spec: WHEN a LatestWins zone receives N queued publishes
    /// THEN on quiet hours exit only the last publish MUST be delivered.
    LatestWins,
}

// ─── Per-zone queue ───────────────────────────────────────────────────────────

/// A bounded FIFO queue for one zone's deferred events.
///
/// Spec: scene-events/spec.md §Requirement: Quiet Hours Queue Semantics,
/// lines 92-104.
#[derive(Debug)]
pub struct ZoneQueue {
    /// The zone's contention policy governs drain behaviour.
    pub policy: ZoneContentionPolicy,
    /// Configurable maximum depth. Default: 100.
    pub max_depth: usize,
    /// Pending events, oldest first.
    events: VecDeque<SceneEvent>,
}

impl ZoneQueue {
    /// Create a new queue with the given policy and depth limit.
    pub fn new(policy: ZoneContentionPolicy, max_depth: usize) -> Self {
        Self {
            policy,
            max_depth,
            events: VecDeque::new(),
        }
    }

    /// Create a queue with default depth (100).
    pub fn new_with_default_depth(policy: ZoneContentionPolicy) -> Self {
        Self::new(policy, 100)
    }

    /// Enqueue an event.
    ///
    /// If the queue is at `max_depth`, the oldest event is dropped first
    /// (overflow drops oldest-first).
    ///
    /// Spec: scene-events/spec.md line 103:
    /// > WHEN the quiet hours queue for a zone reaches 100 entries and a new
    /// > event arrives THEN the oldest entry MUST be dropped.
    pub fn push(&mut self, event: SceneEvent) {
        if self.events.len() >= self.max_depth {
            // Drop oldest to make room.
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// Number of events currently in the queue.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Drain the queue according to the zone's contention policy.
    ///
    /// Returns the events that should be delivered on quiet hours exit.
    ///
    /// - `Fifo` → returns all events in enqueue order.
    /// - `LatestWins` → returns only the last enqueued event (if any).
    ///
    /// The internal queue is cleared after draining.
    ///
    /// Spec: scene-events/spec.md lines 92-104.
    pub fn drain(&mut self) -> Vec<SceneEvent> {
        let events: VecDeque<SceneEvent> = std::mem::take(&mut self.events);
        match self.policy {
            ZoneContentionPolicy::Fifo => events.into_iter().collect(),
            ZoneContentionPolicy::LatestWins => {
                // Spec line 99: only the last publish is delivered.
                match events.into_iter().last() {
                    Some(e) => vec![e],
                    None => vec![],
                }
            }
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::events::{EventPayload, EventSource, InterruptionClass, SceneEventBuilder};
    use uuid::Uuid;

    fn make_event(seq: u64) -> SceneEvent {
        SceneEventBuilder::new(
            "scene.zone.occupancy_changed",
            InterruptionClass::Normal,
            EventPayload::ZoneOccupancyChanged {
                zone_id: Uuid::nil(),
                occupant_count: seq as u32,
            },
        )
        .source(EventSource::system())
        .sequence(seq)
        .build()
    }

    // ── Fifo policy ───────────────────────────────────────────────────────────

    /// FIFO queue delivers all events in enqueue order.
    #[test]
    fn fifo_queue_delivers_in_order() {
        let mut q = ZoneQueue::new(ZoneContentionPolicy::Fifo, 100);
        for seq in 1..=5 {
            q.push(make_event(seq));
        }
        let drained = q.drain();
        assert_eq!(drained.len(), 5);
        for (i, evt) in drained.iter().enumerate() {
            assert_eq!(evt.sequence, (i + 1) as u64);
        }
    }

    /// Queue is empty after draining.
    #[test]
    fn queue_is_empty_after_drain() {
        let mut q = ZoneQueue::new(ZoneContentionPolicy::Fifo, 100);
        q.push(make_event(1));
        q.drain();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    // ── LatestWins policy ─────────────────────────────────────────────────────

    /// WHEN a LatestWins zone receives 10 queued publishes THEN on quiet hours
    /// exit, only the last publish MUST be delivered (spec line 99).
    #[test]
    fn latest_wins_delivers_only_last_event() {
        let mut q = ZoneQueue::new(ZoneContentionPolicy::LatestWins, 100);
        for seq in 1..=10 {
            q.push(make_event(seq));
        }
        let drained = q.drain();
        assert_eq!(drained.len(), 1, "LatestWins must deliver exactly one event");
        assert_eq!(drained[0].sequence, 10, "LatestWins must deliver the last event");
    }

    /// LatestWins with no events returns empty vec.
    #[test]
    fn latest_wins_empty_queue_returns_empty() {
        let mut q = ZoneQueue::new(ZoneContentionPolicy::LatestWins, 100);
        let drained = q.drain();
        assert!(drained.is_empty());
    }

    // ── Queue overflow ────────────────────────────────────────────────────────

    /// WHEN the queue reaches max depth and a new event arrives THEN the oldest
    /// entry MUST be dropped (spec line 103).
    #[test]
    fn overflow_drops_oldest_event() {
        let mut q = ZoneQueue::new(ZoneContentionPolicy::Fifo, 3);
        q.push(make_event(1)); // oldest
        q.push(make_event(2));
        q.push(make_event(3)); // queue full
        q.push(make_event(4)); // triggers overflow → seq=1 dropped

        assert_eq!(q.len(), 3);
        let drained = q.drain();
        let seqs: Vec<u64> = drained.iter().map(|e| e.sequence).collect();
        assert_eq!(seqs, vec![2, 3, 4], "oldest (seq=1) must have been dropped");
    }

    /// Queue depth default is 100.
    #[test]
    fn default_depth_is_100() {
        let q = ZoneQueue::new_with_default_depth(ZoneContentionPolicy::Fifo);
        assert_eq!(q.max_depth, 100);
    }

    /// Overflow at exactly max_depth (boundary check).
    #[test]
    fn overflow_at_exact_max_depth() {
        const DEPTH: usize = 5;
        let mut q = ZoneQueue::new(ZoneContentionPolicy::Fifo, DEPTH);
        // Fill to max_depth.
        for seq in 1..=(DEPTH as u64) {
            q.push(make_event(seq));
        }
        assert_eq!(q.len(), DEPTH);

        // One more triggers drop.
        q.push(make_event(DEPTH as u64 + 1));
        assert_eq!(q.len(), DEPTH, "length must remain at max_depth after overflow");

        // First event (seq=1) must be gone.
        let drained = q.drain();
        assert_eq!(drained[0].sequence, 2, "event seq=1 must have been dropped");
    }
}
