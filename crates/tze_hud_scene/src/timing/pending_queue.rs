//! Per-agent pending mutation queue.
//!
//! # Spec alignment
//!
//! Implements `timing-model/spec.md §Requirement: Presentation Deadline`
//! (lines 236-251) and `§Requirement: Session Close Pending Queue Flush`
//! (lines 291-298).
//!
//! ## Contract
//!
//! - Sorted ascending by `present_at_wall_us`.
//! - Maximum depth: `TimingConfig::pending_queue_depth_per_agent` (default 256).
//! - Insertions beyond max depth → `PENDING_QUEUE_FULL`.
//! - Drain: call [`PendingQueue::drain_ready`] with the current frame's
//!   `vsync_wall_us` to extract all entries whose `present_at_wall_us <=
//!   vsync_wall_us`.
//! - Session close: call [`PendingQueue::flush`] to discard all entries.
//!
//! ## Freeze semantics
//!
//! When frozen (spec §Requirement: Freeze Override Timing Behavior), the
//! compositor must **not** call `drain_ready`. Entries remain queued. After
//! unfreeze, call `drain_ready` normally; all past-due entries will be
//! extracted in the first post-unfreeze frame.

use std::collections::BinaryHeap;

use serde::{Deserialize, Serialize};

use crate::timing::{errors::TimingError, WallUs};

// ─── PendingEntry ─────────────────────────────────────────────────────────────

/// A single queued mutation entry.
///
/// `T` is the caller-supplied payload type (e.g., a serialized mutation).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingEntry<T> {
    /// Absolute presentation timestamp (spec field `present_at_wall_us`).
    /// Zero means "apply at earliest available frame."
    pub present_at_wall_us: WallUs,
    /// The mutation payload.
    pub payload: T,
}

// Implement `Ord` / `PartialOrd` so `BinaryHeap` becomes a **min**-heap.
// Rust's `BinaryHeap` is a max-heap, so we reverse the comparison.
impl<T: Eq> Eq for PendingEntry<T> {}
impl<T: PartialEq> PartialEq for PendingEntry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.present_at_wall_us == other.present_at_wall_us
    }
}
impl<T: Eq> Ord for PendingEntry<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reversed so smallest `present_at_wall_us` is at the top.
        other.present_at_wall_us.cmp(&self.present_at_wall_us)
    }
}
impl<T: Eq + PartialEq> PartialOrd for PendingEntry<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ─── PendingQueue ────────────────────────────────────────────────────────────

/// Per-agent sorted pending mutation queue.
///
/// Internally backed by a binary min-heap (`BinaryHeap` with reversed ordering)
/// so that `drain_ready` runs in O(k log n) where k is the number of due
/// entries and n is the queue depth — not O(n).
#[derive(Clone, Debug)]
pub struct PendingQueue<T: Eq> {
    heap: BinaryHeap<PendingEntry<T>>,
    max_depth: usize,
}

impl<T: Eq + Clone> PendingQueue<T> {
    /// Create a new queue with the given maximum depth.
    ///
    /// Use `TimingConfig::pending_queue_depth_per_agent` (default 256) as the
    /// argument.
    pub fn new(max_depth: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(max_depth.min(4096)),
            max_depth,
        }
    }

    /// Current number of entries in the queue.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Returns `true` if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Enqueue a mutation.
    ///
    /// Returns `Err(TimingError::PendingQueueFull)` if the queue is at max
    /// depth.
    ///
    /// Spec: lines 245-247.
    pub fn push(&mut self, entry: PendingEntry<T>) -> Result<(), TimingError> {
        if self.heap.len() >= self.max_depth {
            return Err(TimingError::PendingQueueFull);
        }
        self.heap.push(entry);
        Ok(())
    }

    /// Drain all entries whose `present_at_wall_us <= vsync_wall_us`.
    ///
    /// Returns the drained entries in ascending `present_at_wall_us` order
    /// (earliest first).
    ///
    /// Zero-valued `present_at_wall_us` (= immediate) is always drained.
    ///
    /// This implements the spec drain condition:
    /// > When frame's vsync_wall_us >= mutation's present_at_wall_us,
    /// > extract from pending queue to Stage 4.
    ///
    /// Spec: lines 241-243.
    ///
    /// ## Freeze semantics
    ///
    /// Do **not** call this method while the scene is frozen. After unfreeze,
    /// calling it will extract all past-due entries in a single pass.
    pub fn drain_ready(&mut self, vsync_wall_us: WallUs) -> Vec<PendingEntry<T>> {
        let mut ready = Vec::new();
        loop {
            match self.heap.peek() {
                Some(entry) if is_entry_ready(entry, vsync_wall_us) => {
                    ready.push(self.heap.pop().expect("peek succeeded"));
                }
                _ => break,
            }
        }
        // The heap gives us entries in descending order (max-heap reversed to
        // min), so we get the smallest timestamps first already.
        ready
    }

    /// Discard all entries (session close).
    ///
    /// From spec §Requirement: Session Close Pending Queue Flush (lines 291-298):
    /// all entries MUST be discarded; a reconnecting agent starts with an empty
    /// queue.
    pub fn flush(&mut self) {
        self.heap.clear();
    }
}

/// True iff the entry should be drained for a frame with the given vsync time.
#[inline]
fn is_entry_ready<T>(entry: &PendingEntry<T>, vsync_wall_us: WallUs) -> bool {
    // Zero = immediate — always due.
    if !entry.present_at_wall_us.is_set() {
        return true;
    }
    entry.present_at_wall_us.as_u64() <= vsync_wall_us.as_u64()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(present_at: u64, payload: u32) -> PendingEntry<u32> {
        PendingEntry {
            present_at_wall_us: WallUs(present_at),
            payload,
        }
    }

    // ── Push ──

    #[test]
    fn push_single_entry_succeeds() {
        let mut q: PendingQueue<u32> = PendingQueue::new(4);
        q.push(entry(1_000, 1)).unwrap();
        assert_eq!(q.len(), 1);
    }

    /// WHEN queue has max entries and a new one is pushed THEN PENDING_QUEUE_FULL.
    #[test]
    fn push_beyond_max_depth_returns_full() {
        let mut q: PendingQueue<u32> = PendingQueue::new(2);
        q.push(entry(1_000, 1)).unwrap();
        q.push(entry(2_000, 2)).unwrap();
        let err = q.push(entry(3_000, 3)).unwrap_err();
        assert_eq!(err, TimingError::PendingQueueFull);
    }

    // ── Drain ──

    /// WHEN vsync >= present_at THEN entry is drained.
    #[test]
    fn drain_ready_extracts_due_entries() {
        let mut q: PendingQueue<u32> = PendingQueue::new(10);
        q.push(entry(1_000, 1)).unwrap();
        q.push(entry(2_000, 2)).unwrap();
        q.push(entry(3_000, 3)).unwrap();

        // Drain up to vsync = 2000
        let ready = q.drain_ready(WallUs(2_000));
        assert_eq!(ready.len(), 2, "entries at 1000 and 2000 should drain");
        // Remaining: entry at 3000
        assert_eq!(q.len(), 1);
    }

    /// WHEN vsync < all present_at THEN nothing is drained.
    #[test]
    fn drain_ready_no_entries_due() {
        let mut q: PendingQueue<u32> = PendingQueue::new(4);
        q.push(entry(5_000, 1)).unwrap();
        q.push(entry(6_000, 2)).unwrap();

        let ready = q.drain_ready(WallUs(4_000));
        assert!(ready.is_empty());
        assert_eq!(q.len(), 2);
    }

    /// Entries with present_at = 0 (immediate) are always drained.
    #[test]
    fn immediate_entries_always_drained() {
        let mut q: PendingQueue<u32> = PendingQueue::new(4);
        q.push(entry(0, 99)).unwrap(); // immediate
        q.push(entry(5_000, 1)).unwrap(); // future

        let ready = q.drain_ready(WallUs(1_000));
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].payload, 99);
    }

    /// Drained entries are in ascending present_at order (earliest first).
    #[test]
    fn drain_returns_ascending_order() {
        let mut q: PendingQueue<u32> = PendingQueue::new(10);
        // Insert in reverse order
        q.push(entry(3_000, 3)).unwrap();
        q.push(entry(1_000, 1)).unwrap();
        q.push(entry(2_000, 2)).unwrap();

        let ready = q.drain_ready(WallUs(3_000));
        assert_eq!(ready.len(), 3);
        // Should come out in order 1000, 2000, 3000
        assert_eq!(ready[0].present_at_wall_us, WallUs(1_000));
        assert_eq!(ready[1].present_at_wall_us, WallUs(2_000));
        assert_eq!(ready[2].present_at_wall_us, WallUs(3_000));
    }

    // ── Session close flush ──

    /// WHEN session closes THEN all entries are discarded.
    #[test]
    fn flush_discards_all_entries() {
        let mut q: PendingQueue<u32> = PendingQueue::new(256);
        for i in 0..10 {
            q.push(entry(i as u64 * 1_000, i)).unwrap();
        }
        assert_eq!(q.len(), 10);
        q.flush();
        assert!(q.is_empty());
        // Reconnecting agent gets empty queue; nothing drains
        let ready = q.drain_ready(WallUs(u64::MAX));
        assert!(ready.is_empty());
    }

    // ── No-earlier-than guarantee (spec lines 99-101) ──

    /// WHEN present_at = V + 1ms, vsync = V THEN NOT drained.
    #[test]
    fn no_earlier_than_guarantee() {
        let vsync = WallUs(1_000_000_000); // V
        let present_at = WallUs(1_000_001_000); // V + 1ms
        let mut q: PendingQueue<u32> = PendingQueue::new(4);
        q.push(PendingEntry {
            present_at_wall_us: present_at,
            payload: 42,
        })
        .unwrap();
        // Frame at vsync = V → NOT drained
        let ready = q.drain_ready(vsync);
        assert!(ready.is_empty(), "mutation MUST NOT apply before present_at");
        // Frame at vsync = V + 1ms → now drained
        let ready = q.drain_ready(present_at);
        assert_eq!(ready.len(), 1);
    }
}
