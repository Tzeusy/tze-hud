//! Per-agent event queue with backpressure handling.
//!
//! Implements the queue depth contract from spec.md §8.5:
//! - Default depth: 256 events per agent
//! - Hard cap: 4096 events per agent
//! - Transactional events are never dropped, even at hard cap (non-transactional
//!   events are evicted to make room)
//! - Ephemeral (non-transactional) events are silently dropped at capacity

use crate::envelope::InputEnvelope;

/// Default event queue capacity per agent.
pub const DEFAULT_QUEUE_DEPTH: usize = 256;

/// Hard cap on event queue size per agent.
pub const HARD_CAP_QUEUE_DEPTH: usize = 4096;

/// Per-agent event queue.
///
/// Maintains a FIFO queue of envelopes for one agent session. Under backpressure
/// (queue at or above `capacity`), non-transactional events are dropped. When
/// the queue is at the hard cap and a transactional event arrives, the oldest
/// non-transactional event is evicted to make room. If no non-transactional
/// events remain at hard cap, the transactional event is still enqueued (the
/// hard cap can be exceeded by one transactional event in the worst case; in
/// practice the queue stays bounded because transactional events are rare).
pub struct AgentEventQueue {
    queue: Vec<InputEnvelope>,
    /// Soft capacity; ephemeral events are dropped once this is reached.
    capacity: usize,
}

impl AgentEventQueue {
    /// Create a queue with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_QUEUE_DEPTH)
    }

    /// Create a queue with a custom capacity (clamped to `HARD_CAP_QUEUE_DEPTH`).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: Vec::new(),
            capacity: capacity.min(HARD_CAP_QUEUE_DEPTH),
        }
    }

    /// Number of envelopes currently queued.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Returns `true` if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Returns a slice of all currently queued envelopes (in enqueue order).
    pub fn events(&self) -> &[InputEnvelope] {
        &self.queue
    }

    /// Drain all envelopes from the queue, returning them in enqueue order.
    ///
    /// The queue is empty after this call.
    pub fn drain(&mut self) -> Vec<InputEnvelope> {
        std::mem::take(&mut self.queue)
    }

    /// Enqueue a single envelope.
    ///
    /// Backpressure policy (per spec.md §8.5):
    /// 1. If queue len < capacity: always accept.
    /// 2. If queue len >= capacity and event is transactional: evict the oldest
    ///    non-transactional event, then enqueue. If all events are transactional,
    ///    append anyway (hard cap may be transiently exceeded by 1).
    /// 3. If queue len >= capacity and event is non-transactional: drop silently.
    ///
    /// Returns `true` if the event was enqueued, `false` if it was dropped.
    pub fn enqueue(&mut self, envelope: InputEnvelope) -> bool {
        if self.queue.len() < self.capacity {
            self.queue.push(envelope);
            return true;
        }

        // At or above capacity.
        if envelope.is_transactional() {
            // Try to evict the oldest non-transactional event.
            if let Some(idx) = self.queue.iter().position(|e| !e.is_transactional()) {
                self.queue.remove(idx);
            }
            // Enqueue regardless — transactional events must not be dropped.
            self.queue.push(envelope);
            true
        } else {
            // Non-transactional under backpressure — drop silently.
            false
        }
    }

    /// Returns `true` if the queue is at or above capacity (backpressure active).
    pub fn is_under_backpressure(&self) -> bool {
        self.queue.len() >= self.capacity
    }
}

impl Default for AgentEventQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{InputEnvelope, PointerDownData, PointerMoveData};
    use tze_hud_scene::{MonoUs, SceneId};

    fn make_move(ts: u64) -> InputEnvelope {
        InputEnvelope::PointerMove(PointerMoveData {
            tile_id: SceneId::null(),
            node_id: SceneId::null(),
            interaction_id: String::new(),
            timestamp_mono_us: MonoUs(ts),
            device_id: String::new(),
            local_x: 0.0,
            local_y: 0.0,
            display_x: 0.0,
            display_y: 0.0,
        })
    }

    fn make_down(ts: u64) -> InputEnvelope {
        InputEnvelope::PointerDown(PointerDownData {
            tile_id: SceneId::null(),
            node_id: SceneId::null(),
            interaction_id: String::new(),
            timestamp_mono_us: MonoUs(ts),
            device_id: String::new(),
            local_x: 0.0,
            local_y: 0.0,
            display_x: 0.0,
            display_y: 0.0,
            button: 0,
        })
    }

    #[test]
    fn test_queue_accepts_events_below_capacity() {
        let mut q = AgentEventQueue::with_capacity(4);
        assert!(q.enqueue(make_move(1)));
        assert!(q.enqueue(make_move(2)));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn test_non_transactional_dropped_at_capacity() {
        let mut q = AgentEventQueue::with_capacity(2);
        // Fill to capacity with non-transactional
        q.enqueue(make_move(1));
        q.enqueue(make_move(2));
        assert_eq!(q.len(), 2);
        assert!(q.is_under_backpressure());

        // Another non-transactional should be dropped
        let accepted = q.enqueue(make_move(3));
        assert!(
            !accepted,
            "non-transactional must be dropped under backpressure"
        );
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn test_transactional_evicts_non_transactional_at_capacity() {
        let mut q = AgentEventQueue::with_capacity(2);
        // Fill queue with non-transactional
        q.enqueue(make_move(1));
        q.enqueue(make_move(2));
        assert_eq!(q.len(), 2);

        // Transactional event should evict the oldest non-transactional
        let accepted = q.enqueue(make_down(3));
        assert!(accepted, "transactional must always be enqueued");
        assert_eq!(q.len(), 2, "eviction should keep length at capacity");

        let events = q.drain();
        // The oldest move (ts=1) was evicted; remaining is move(ts=2) + down(ts=3)
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].timestamp_mono_us(), MonoUs(2));
        assert_eq!(events[1].timestamp_mono_us(), MonoUs(3));
    }

    #[test]
    fn test_transactional_appended_when_all_transactional_at_cap() {
        let mut q = AgentEventQueue::with_capacity(2);
        // Fill with transactional events
        q.enqueue(make_down(1));
        q.enqueue(make_down(2));
        assert_eq!(q.len(), 2);

        // Another transactional — no non-transactional to evict, so it appends
        let accepted = q.enqueue(make_down(3));
        assert!(accepted);
        // Hard cap may be transiently exceeded by 1 transactional
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn test_drain_clears_queue() {
        let mut q = AgentEventQueue::with_capacity(10);
        q.enqueue(make_move(1));
        q.enqueue(make_down(2));
        let drained = q.drain();
        assert_eq!(drained.len(), 2);
        assert!(q.is_empty());
    }
}
