//! Event coalescing under backpressure (RFC 0010 §8.5, §8.6).
//!
//! Under backpressure the runtime coalesces events that share a coalesce key:
//! only the **latest** event for each key is retained. Lease and degradation
//! events are **never** dropped, regardless of backpressure.
//!
//! The coalesce buffer per subscriber is bounded to 64 entries (spec line 221).

use std::collections::VecDeque;

/// Maximum number of entries in the coalesce buffer per subscriber.
pub const COALESCE_BUFFER_CAPACITY: usize = 64;

/// Coalesce key for an event.
///
/// Events with the same key will be coalesced (latest-wins) under backpressure.
/// Events with `Never` are never coalesced and never dropped.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CoalesceKey {
    /// A tile-specific key (e.g., for TileUpdated: keyed by tile_id).
    TileId(String),
    /// A zone-specific key (e.g., for ZoneOccupancyChanged: keyed by zone name).
    ZoneName(String),
    /// Singleton — only one event of this type is ever queued (latest-wins).
    /// Used for events like ActiveTabChanged where only the final state matters.
    Singleton(String),
    /// Never coalesced and never dropped (lease events, degradation events).
    Never,
}

/// Derive the coalesce key for an event type + optional entity id.
///
/// Callers supply:
/// - `event_type`: the dotted event type string (e.g., "scene.tile.updated")
/// - `entity_id`: the primary subject ID if relevant (tile_id, zone_name, etc.)
///
/// Returns the coalesce key to use.
pub fn coalesce_key_for(event_type: &str, entity_id: Option<&str>) -> CoalesceKey {
    // Lease and degradation events: never drop (spec line 229-231)
    if event_type.starts_with("system.lease_") || event_type.starts_with("system.degradation_") {
        return CoalesceKey::Never;
    }

    match event_type {
        // TileUpdated: coalesce per tile_id
        "scene.tile.updated" => {
            if let Some(id) = entity_id {
                CoalesceKey::TileId(id.to_string())
            } else {
                CoalesceKey::Singleton("scene.tile.updated".to_string())
            }
        }
        // ActiveTabChanged: singleton (only last state matters)
        "scene.tab.active_changed" => {
            CoalesceKey::Singleton("scene.tab.active_changed".to_string())
        }
        // ZoneOccupancyChanged: coalesce per zone name
        "scene.zone.occupancy_changed" => {
            if let Some(name) = entity_id {
                CoalesceKey::ZoneName(name.to_string())
            } else {
                CoalesceKey::Singleton("scene.zone.occupancy_changed".to_string())
            }
        }
        // For all other coalescing cases not handled above, use a per-type singleton
        _ => CoalesceKey::Singleton(event_type.to_string()),
    }
}

/// A single entry in the coalesce buffer.
#[derive(Clone, Debug)]
pub struct CoalesceEntry<E: Clone> {
    pub key: CoalesceKey,
    pub event: E,
}

/// Per-subscriber coalesce buffer with bounded capacity.
///
/// Under backpressure:
/// - Events with the same coalesce key replace earlier events with that key
///   (latest-wins semantics).
/// - Events with `CoalesceKey::Never` are always appended (never replaced).
/// - When the buffer is full and no existing key matches, the oldest coalesable
///   event (i.e., not `Never`) is evicted to make room. If no coalesable event
///   exists and the buffer is full, the new event is dropped (except for `Never`
///   events, which are always kept).
#[derive(Debug)]
pub struct CoalesceBuffer<E: Clone> {
    entries: VecDeque<CoalesceEntry<E>>,
    capacity: usize,
}

impl<E: Clone> CoalesceBuffer<E> {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(COALESCE_BUFFER_CAPACITY),
            capacity: COALESCE_BUFFER_CAPACITY,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push an event into the buffer.
    ///
    /// `coalesce` — when `true` (backpressure active), events with the same
    /// coalescing key replace earlier events (latest-wins). When `false` (no
    /// backpressure), events are appended in FIFO order without coalescing.
    ///
    /// Regardless of `coalesce`:
    /// - `CoalesceKey::Never` entries are always appended and never dropped.
    ///   If the buffer is full, the oldest coalesable entry is evicted to make
    ///   room. If no coalesable entry exists the buffer grows past capacity
    ///   (the only correct behaviour: "never drop" is stronger than the size
    ///   bound, and this edge case is rare — it requires a slow subscriber
    ///   receiving a flood of lease/degradation events).
    /// - When `coalesce` is `false` and the buffer is full, the oldest
    ///   coalesable entry is evicted (graceful degradation under unexpected
    ///   overload without losing Never events).
    pub fn push(&mut self, key: CoalesceKey, event: E, coalesce: bool) {
        if key == CoalesceKey::Never {
            // Transactional — never drop. If full, evict oldest coalesable entry.
            if self.entries.len() >= self.capacity {
                self.evict_oldest_coalesable();
            }
            self.entries.push_back(CoalesceEntry { key, event });
            return;
        }

        // Under backpressure: check if an existing entry with the same key exists;
        // if so, replace it (latest-wins coalescing).
        if coalesce {
            for entry in self.entries.iter_mut() {
                if entry.key == key {
                    entry.event = event;
                    return;
                }
            }
        }

        // No existing entry with this key (or not coalescing). If the buffer is
        // full, evict the oldest coalesable entry to make room.
        if self.entries.len() >= self.capacity {
            if self.evict_oldest_coalesable() {
                // Eviction succeeded; there is room now.
            } else {
                // All entries are Never; drop this event (backpressure shedding).
                return;
            }
        }

        self.entries.push_back(CoalesceEntry { key, event });
    }

    /// Drain all events from the buffer, returning them in order.
    pub fn drain(&mut self) -> Vec<E> {
        self.entries.drain(..).map(|e| e.event).collect()
    }

    /// Returns the number of entries in the buffer.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Evict the oldest entry that is not `CoalesceKey::Never`.
    ///
    /// Returns `true` if an entry was evicted, `false` if no coalesable entry
    /// was found (all entries are `Never`).
    fn evict_oldest_coalesable(&mut self) -> bool {
        let pos = self
            .entries
            .iter()
            .position(|e| e.key != CoalesceKey::Never);
        if let Some(idx) = pos {
            self.entries.remove(idx);
            true
        } else {
            false
        }
    }
}

impl<E: Clone> Default for CoalesceBuffer<E> {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic coalescing (coalesce=true, i.e. under backpressure) ──────────────

    #[test]
    fn test_same_tile_updates_coalesced() {
        let mut buf: CoalesceBuffer<&str> = CoalesceBuffer::new();
        let key = CoalesceKey::TileId("tile-1".to_string());

        buf.push(key.clone(), "update-1", true);
        buf.push(key.clone(), "update-2", true);
        buf.push(key.clone(), "update-3", true);

        let drained = buf.drain();
        // Only the latest should remain
        assert_eq!(drained, vec!["update-3"]);
    }

    #[test]
    fn test_different_tiles_not_coalesced() {
        let mut buf: CoalesceBuffer<&str> = CoalesceBuffer::new();

        buf.push(CoalesceKey::TileId("tile-1".to_string()), "t1", true);
        buf.push(CoalesceKey::TileId("tile-2".to_string()), "t2", true);
        buf.push(CoalesceKey::TileId("tile-1".to_string()), "t1-v2", true);

        let drained = buf.drain();
        // tile-2 untouched, tile-1 coalesced to latest
        assert_eq!(drained.len(), 2);
        assert!(drained.contains(&"t2"));
        assert!(drained.contains(&"t1-v2"));
        assert!(!drained.contains(&"t1"));
    }

    #[test]
    fn test_singleton_coalesced() {
        let mut buf: CoalesceBuffer<u32> = CoalesceBuffer::new();
        let key = CoalesceKey::Singleton("scene.tab.active_changed".to_string());

        buf.push(key.clone(), 1, true);
        buf.push(key.clone(), 2, true);
        buf.push(key.clone(), 3, true);

        let drained = buf.drain();
        assert_eq!(drained, vec![3]);
    }

    // ── No coalescing when not under backpressure ──────────────────────────────

    #[test]
    fn test_no_coalescing_without_backpressure() {
        let mut buf: CoalesceBuffer<u32> = CoalesceBuffer::new();
        let key = CoalesceKey::TileId("tile-1".to_string());

        buf.push(key.clone(), 1, false);
        buf.push(key.clone(), 2, false);
        buf.push(key.clone(), 3, false);

        let drained = buf.drain();
        // All three retained when not under backpressure
        assert_eq!(drained, vec![1, 2, 3]);
    }

    // ── Never-drop events ─────────────────────────────────────────────────────

    #[test]
    fn test_never_events_not_coalesced() {
        let mut buf: CoalesceBuffer<&str> = CoalesceBuffer::new();

        buf.push(CoalesceKey::Never, "lease-revoked-1", false);
        buf.push(CoalesceKey::Never, "lease-revoked-2", false);
        buf.push(CoalesceKey::Never, "degradation-changed", false);

        let drained = buf.drain();
        // All three retained
        assert_eq!(drained.len(), 3);
        assert!(drained.contains(&"lease-revoked-1"));
        assert!(drained.contains(&"lease-revoked-2"));
        assert!(drained.contains(&"degradation-changed"));
    }

    #[test]
    fn test_never_event_preserved_under_backpressure() {
        let mut buf: CoalesceBuffer<u32> = CoalesceBuffer::with_capacity(4);

        // Fill with coalesable events
        for i in 0..4u32 {
            buf.push(CoalesceKey::TileId(format!("tile-{i}")), i, true);
        }
        assert_eq!(buf.len(), 4);

        // Pushing a Never event should evict the oldest coalesable entry
        buf.push(CoalesceKey::Never, 99, false);
        assert_eq!(buf.len(), 4); // still 4 — one was evicted

        let drained = buf.drain();
        assert!(drained.contains(&99)); // never event retained
        assert!(!drained.contains(&0)); // oldest evicted
    }

    #[test]
    fn test_coalesable_event_dropped_when_buffer_full_of_never() {
        let mut buf: CoalesceBuffer<u32> = CoalesceBuffer::with_capacity(2);

        buf.push(CoalesceKey::Never, 1, false);
        buf.push(CoalesceKey::Never, 2, false);
        assert_eq!(buf.len(), 2);

        // Coalesable event: no room; should be dropped
        buf.push(CoalesceKey::TileId("tile-1".to_string()), 3, true);
        let drained = buf.drain();
        assert_eq!(drained, vec![1, 2]); // 3 dropped
    }

    // ── Capacity bound ────────────────────────────────────────────────────────

    #[test]
    fn test_buffer_capacity_bounded_at_64() {
        assert_eq!(COALESCE_BUFFER_CAPACITY, 64);
    }

    #[test]
    fn test_buffer_bounded_coalesable_eviction() {
        let mut buf: CoalesceBuffer<u32> = CoalesceBuffer::with_capacity(4);

        // Push 4 unique coalesable events
        for i in 0..4u32 {
            buf.push(CoalesceKey::TileId(format!("tile-{i}")), i, true);
        }
        // Push a 5th — should evict the oldest (tile-0 → value 0)
        buf.push(CoalesceKey::TileId("tile-4".to_string()), 4, true);
        let drained = buf.drain();
        assert_eq!(drained.len(), 4);
        assert!(!drained.contains(&0)); // oldest evicted
        assert!(drained.contains(&4)); // new one retained
    }

    // ── coalesce_key_for helper ───────────────────────────────────────────────

    #[test]
    fn test_lease_event_never_key() {
        assert_eq!(
            coalesce_key_for("system.lease_revoked", None),
            CoalesceKey::Never
        );
        assert_eq!(
            coalesce_key_for("system.lease_granted", None),
            CoalesceKey::Never
        );
    }

    #[test]
    fn test_degradation_event_never_key() {
        assert_eq!(
            coalesce_key_for("system.degradation_changed", None),
            CoalesceKey::Never
        );
    }

    #[test]
    fn test_tile_updated_keyed_by_tile_id() {
        let key = coalesce_key_for("scene.tile.updated", Some("tile-abc"));
        assert_eq!(key, CoalesceKey::TileId("tile-abc".to_string()));
    }

    #[test]
    fn test_active_tab_changed_singleton() {
        let key = coalesce_key_for("scene.tab.active_changed", None);
        assert_eq!(
            key,
            CoalesceKey::Singleton("scene.tab.active_changed".to_string())
        );
    }

    #[test]
    fn test_zone_occupancy_keyed_by_zone_name() {
        let key = coalesce_key_for("scene.zone.occupancy_changed", Some("main_zone"));
        assert_eq!(key, CoalesceKey::ZoneName("main_zone".to_string()));
    }
}
