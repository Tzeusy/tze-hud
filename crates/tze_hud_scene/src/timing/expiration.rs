//! Min-heap expiry tracking for tiles.
//!
//! # Spec alignment
//!
//! Implements `timing-model/spec.md §Requirement: Expiration Policy`
//! (lines 253-268).
//!
//! ## Contract
//!
//! - `O(expired_items)` per frame evaluation (not O(total tiles)).
//! - Non-negotiable under load: runs even at degradation Level 4/5.
//! - Expiring tile MUST be removed from its sync group before deletion.
//! - An expired tile produces a `TileExpired` event.
//!
//! ## Freeze semantics
//!
//! During freeze (spec §Requirement: Freeze Override Timing Behavior):
//! - Callers MUST NOT call [`ExpirationHeap::drain_expired`].
//! - After unfreeze, calling it will expire all past-due tiles in one pass.
//!
//! ## Safe mode semantics
//!
//! During safe mode, expiry runs normally; do NOT skip `drain_expired`.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use serde::{Deserialize, Serialize};

use crate::timing::WallUs;
use crate::types::SceneId;

// ─── ExpirationEntry ─────────────────────────────────────────────────────────

/// An entry in the expiration heap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpirationEntry {
    /// The wall-clock time at which the tile expires.
    pub expires_at_wall_us: WallUs,
    /// The tile to expire.
    pub tile_id: SceneId,
}

impl Ord for ExpirationEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Wrapped in Reverse so the heap is a min-heap.
        Reverse(self.expires_at_wall_us).cmp(&Reverse(other.expires_at_wall_us))
    }
}

impl PartialOrd for ExpirationEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ─── ExpirationHeap ──────────────────────────────────────────────────────────

/// Min-heap for efficient tile expiry evaluation.
///
/// At each frame's Stage 4 the compositor calls
/// [`ExpirationHeap::drain_expired`] with the current `vsync_wall_us`.  Only
/// the entries at the top of the heap (those with the smallest
/// `expires_at_wall_us`) are inspected — touching O(expired_items) elements,
/// not O(total tiles).
///
/// # Usage
///
/// ```
/// use tze_hud_scene::timing::expiration::ExpirationHeap;
/// use tze_hud_scene::timing::WallUs;
/// use tze_hud_scene::types::SceneId;
///
/// let mut heap = ExpirationHeap::new();
/// let tile_id = SceneId::new();
/// heap.register(tile_id, WallUs(1_000_500));
///
/// // Frame at vsync = 1_000_000 → nothing expired yet
/// let expired = heap.drain_expired(WallUs(1_000_000));
/// assert!(expired.is_empty());
///
/// // Frame at vsync = 1_000_500 → tile expired
/// let expired = heap.drain_expired(WallUs(1_000_500));
/// assert_eq!(expired, vec![tile_id]);
/// ```
#[derive(Clone, Debug, Default)]
pub struct ExpirationHeap {
    heap: BinaryHeap<ExpirationEntry>,
}

impl ExpirationHeap {
    /// Create an empty expiration heap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tile with its expiry time.
    ///
    /// Does nothing if `expires_at_wall_us` is zero (not set).
    pub fn register(&mut self, tile_id: SceneId, expires_at_wall_us: WallUs) {
        if expires_at_wall_us.is_set() {
            self.heap.push(ExpirationEntry {
                expires_at_wall_us,
                tile_id,
            });
        }
    }

    /// Deregister a tile from the heap.
    ///
    /// Required when a tile is deleted before its expiry time, or when a tile
    /// leaves a sync group before expiry (spec: expiring tile MUST be removed
    /// from sync group before deletion).
    ///
    /// Uses a linear scan and rebuild — acceptable since this is infrequent
    /// compared to `drain_expired`.
    pub fn remove(&mut self, tile_id: SceneId) {
        let entries: Vec<_> = self.heap.drain().filter(|e| e.tile_id != tile_id).collect();
        self.heap = entries.into();
    }

    /// Drain all tiles whose `expires_at_wall_us <= vsync_wall_us`.
    ///
    /// Returns the `tile_id`s of all expired tiles in expiry-time order
    /// (earliest first).
    ///
    /// This is `O(k log n)` where `k` is the number of expired tiles and
    /// `n` is the total registration count — satisfying the spec's
    /// "touch only `k` entries" requirement.
    ///
    /// Spec: lines 258-260, 266-268.
    ///
    /// ## Freeze semantics
    ///
    /// Do NOT call this method while the scene is frozen.  Call it on the
    /// first post-unfreeze frame to expire all past-due tiles immediately.
    pub fn drain_expired(&mut self, vsync_wall_us: WallUs) -> Vec<SceneId> {
        let mut expired = Vec::new();
        loop {
            match self.heap.peek() {
                Some(entry) if entry.expires_at_wall_us.as_u64() <= vsync_wall_us.as_u64() => {
                    expired.push(self.heap.pop().expect("peek succeeded").tile_id);
                }
                _ => break,
            }
        }
        expired
    }

    /// Number of registered (not yet expired) entries.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Returns `true` if no tiles are tracked.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic register / drain ──

    #[test]
    fn register_and_drain_single_tile() {
        let mut heap = ExpirationHeap::new();
        let tile_id = SceneId::new();
        heap.register(tile_id, WallUs(1_000));

        let expired = heap.drain_expired(WallUs(1_000));
        assert_eq!(expired, vec![tile_id]);
        assert!(heap.is_empty());
    }

    /// WHEN vsync < expires_at THEN tile NOT expired.
    #[test]
    fn tile_not_expired_before_vsync() {
        let mut heap = ExpirationHeap::new();
        let tile_id = SceneId::new();
        heap.register(tile_id, WallUs(5_000));

        let expired = heap.drain_expired(WallUs(4_999));
        assert!(expired.is_empty());
        assert_eq!(heap.len(), 1);
    }

    /// WHEN 1000 tiles exist but only 2 have expired THEN only 2 touched.
    #[test]
    fn only_expired_tiles_touched() {
        let mut heap = ExpirationHeap::new();

        // 998 tiles with far-future expiry
        let far_tile_ids: Vec<_> = (0..998).map(|_| SceneId::new()).collect();
        for tid in &far_tile_ids {
            heap.register(*tid, WallUs(999_999_999));
        }

        // 2 tiles that expire at vsync=1000
        let exp1 = SceneId::new();
        let exp2 = SceneId::new();
        heap.register(exp1, WallUs(1_000));
        heap.register(exp2, WallUs(1_000));

        assert_eq!(heap.len(), 1000);

        let expired = heap.drain_expired(WallUs(1_000));
        assert_eq!(expired.len(), 2, "exactly 2 tiles should expire");
        assert_eq!(heap.len(), 998, "998 tiles remain");
    }

    /// Expiry is non-negotiable: runs regardless of degradation level.
    /// (The test just verifies the API doesn't gate on any flag.)
    #[test]
    fn expiry_runs_unconditionally() {
        let mut heap = ExpirationHeap::new();
        let tile_id = SceneId::new();
        heap.register(tile_id, WallUs(100));
        // Calling drain_expired from "degraded" context still works
        let expired = heap.drain_expired(WallUs(100));
        assert_eq!(expired, vec![tile_id]);
    }

    // ── Remove (deregister) ──

    #[test]
    fn remove_tile_before_expiry() {
        let mut heap = ExpirationHeap::new();
        let tile_id = SceneId::new();
        heap.register(tile_id, WallUs(5_000));
        assert_eq!(heap.len(), 1);

        heap.remove(tile_id);
        assert!(heap.is_empty());

        // Drain should return nothing
        let expired = heap.drain_expired(WallUs(5_000));
        assert!(expired.is_empty());
    }

    // ── Zero-value ignored ──

    #[test]
    fn register_zero_expires_at_ignored() {
        let mut heap = ExpirationHeap::new();
        let tile_id = SceneId::new();
        heap.register(tile_id, WallUs::NOT_SET); // 0 = no expiry
        assert!(heap.is_empty());
    }

    // ── Multiple expiry times ──

    #[test]
    fn drain_multiple_expired_returns_ascending_order() {
        let mut heap = ExpirationHeap::new();
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let t3 = SceneId::new();

        // Insert in reverse order to verify heap sorts them
        heap.register(t3, WallUs(3_000));
        heap.register(t1, WallUs(1_000));
        heap.register(t2, WallUs(2_000));

        let expired = heap.drain_expired(WallUs(3_000));
        // All three tiles must be present in ascending expiry-time order.
        assert_eq!(expired.len(), 3, "all three tiles must expire");
        assert_eq!(expired[0], t1, "t1 (expires 1000) must be first (earliest)");
        assert_eq!(expired[1], t2, "t2 (expires 2000) must be second");
        assert_eq!(expired[2], t3, "t3 (expires 3000) must be last");
    }

    // ── Freeze-then-unfreeze semantics ──

    /// WHEN frozen (drain not called) and expiry passes, THEN expired on first
    /// post-unfreeze call to drain_expired.
    #[test]
    fn freeze_then_unfreeze_expires_past_due() {
        let mut heap = ExpirationHeap::new();
        let tile_id = SceneId::new();
        heap.register(tile_id, WallUs(1_000));

        // "Frozen" — do not call drain_expired at vsync=1000
        // "Unfreezes" — call drain at vsync=2000
        let expired = heap.drain_expired(WallUs(2_000));
        assert_eq!(
            expired,
            vec![tile_id],
            "past-due tile must expire on unfreeze"
        );
    }
}
