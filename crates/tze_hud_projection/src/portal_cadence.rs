//! Portal cadence coalescing with cross-portal fairness (hud-5jbra.5, hud-zmt1a).
//!
//! ## Overview
//!
//! Implements work-conserving coalescing with a bounded cross-portal fairness
//! guarantee for concurrent text-stream portals (spec §5.1, tasks.md §5.1–5.4).
//!
//! This module lives in `tze_hud_projection` so that `ProjectionAuthority` can
//! hold a `PortalCadenceCoalescer` field without pulling in the full runtime
//! crate. `tze_hud_runtime` re-exports everything from here.
//!
//! Two properties are enforced:
//!
//! 1. **Work-conserving**: when render capacity exists and committed units are
//!    pending for any portal, the coalescer returns a portal key on every
//!    `next_ready_portal` call until no portal has pending work.
//!
//! 2. **Cross-portal fairness**: under equal sustained input rates across N
//!    portals, no portal's presentation lag diverges from any other portal's by
//!    more than one service round. The bound is structural — round-robin prevents
//!    unbounded starvation — not a hard real-time guarantee.
//!
//! ## Message class
//!
//! Transcript appends are **state-stream** traffic: coalescing may skip
//! intermediate snapshots but must always expose the latest coherent window.
//! The coalescer never drops the most-recent pending snapshot for any portal.
//!
//! ## Spec references
//!
//! - tasks.md §5.1: work-conserving coalescing with cross-portal fairness
//! - tasks.md §5.4: dual-portal fairness test under equal sustained rates
//! - design.md §5: coalescing policy and fairness liveness property
//! - engineering-bar.md §2: frame / input / stage budget constraints
//!
//! ## Usage pattern
//!
//! ```rust,ignore
//! let mut coalescer = PortalCadenceCoalescer::new();
//!
//! // On each transcript append (from the adapter):
//! // record_append(portal_key, payload_bytes, sequence, submitted_at_us)
//! coalescer.record_append("portal://a", snapshot_bytes, seq, now_us);
//!
//! // On each frame (Stage 3 Mutation Intake):
//! while let Some(key) = coalescer.next_ready_portal() {
//!     let (payload, _seq) = coalescer.take_snapshot(&key).unwrap();
//!     // … apply payload to scene graph …
//! }
//! ```

use std::collections::{HashMap, VecDeque};

// ─── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of bytes held for a single portal snapshot.
///
/// This is an upper safety cap on the snapshot buffer, not a normative
/// workload budget. The normative workload (tasks.md §5.2) specifies ≥ 4 KiB
/// burst payloads and ≥ 200 scalars/s sustained; a single portal snapshot
/// will be much smaller in practice. 65,535 bytes is chosen as a generous
/// hard ceiling that prevents unbounded memory growth from a misbehaving
/// producer without imposing an artificial constraint on legitimate large
/// transcript windows (e.g. long tool-output flushes).
/// We keep only the *latest* snapshot per portal, so this is the maximum
/// retained size per portal key.
pub const MAX_PORTAL_SNAPSHOT_BYTES: usize = 65_535;

/// Cadence harness: minimum sustained scalar rate (scalars/second).
pub const CADENCE_MIN_SCALARS_PER_SEC: u64 = 200;

/// Cadence harness: minimum increment rate (appends/second).
pub const CADENCE_MIN_INCREMENTS_PER_SEC: u64 = 10;

/// Cadence harness: minimum sustained duration for the soak criterion (seconds).
pub const CADENCE_SUSTAINED_SECS: u64 = 60;

/// Cadence harness: burst payload bytes (tasks.md §5.2 — "≥ 4096 bytes in 250 ms").
pub const CADENCE_BURST_BYTES: usize = 4_096;

/// Cadence harness: burst window in milliseconds.
pub const CADENCE_BURST_WINDOW_MS: u64 = 250;

// ─── PendingPortalSnapshot ─────────────────────────────────────────────────────

/// The latest pending snapshot for a single portal key.
#[derive(Debug)]
struct PendingPortalSnapshot {
    /// Latest coalesced payload (full visible-window snapshot).
    pub payload: Vec<u8>,
    /// Monotonic sequence counter from the source (used to enforce latest-wins).
    pub sequence: u64,
    /// Wall-clock time of the most recent append (µs, for fairness bookkeeping).
    pub submitted_at_us: u64,
}

// ─── PortalCadenceCoalescer ────────────────────────────────────────────────────

/// Work-conserving multi-portal coalescer with round-robin cross-portal fairness.
///
/// Each `portal_key` is served in round-robin insertion order. Every call to
/// [`next_ready_portal`] advances the internal pointer exactly once, returning
/// the next portal key that has a pending snapshot. The pointer wraps around
/// after all keys have been visited, guaranteeing that, under equal sustained
/// rates, no portal's accumulated lag diverges from any other's by more than
/// one complete round.
///
/// # Snapshot semantics
///
/// Only the **latest** snapshot per portal key is retained. If `record_append`
/// is called N times for a portal before a [`take_snapshot`], the coalescer
/// holds only the N-th snapshot — intermediate states are intentionally
/// discarded. This matches the state-stream latest-wins rule.
///
/// # Stale-sequence guard
///
/// After a snapshot is drained via [`take_snapshot`], the portal key remains in
/// `service_order` (for round-robin continuity) but is absent from `pending`.
/// Without additional tracking, a subsequent [`record_append`] with a sequence
/// number ≤ the last drained sequence would fall into the `None` branch (key
/// absent from `pending`) and be incorrectly accepted as a fresh snapshot.
///
/// To prevent this, `last_drained_sequence` tracks the highest sequence number
/// ever drained for each portal key. A new append is rejected if its sequence
/// is ≤ the last drained sequence, even when no pending snapshot is present.
#[derive(Debug, Default)]
pub struct PortalCadenceCoalescer {
    /// Map from portal key to its latest pending snapshot.
    pending: HashMap<String, PendingPortalSnapshot>,
    /// Ordered list of portal keys, maintained as the round-robin service queue.
    /// Keys appear in insertion order; the service pointer wraps.
    service_order: VecDeque<String>,
    /// Highest sequence number drained via `take_snapshot` per portal key.
    /// Used to reject stale appends after a drain (post-drain stale-sequence guard).
    last_drained_sequence: HashMap<String, u64>,
    /// Number of snapshots taken (for diagnostics).
    total_taken: u64,
    /// Number of appends that were coalesced (superseded a previous pending snapshot).
    total_coalesced: u64,
}

impl PortalCadenceCoalescer {
    /// Create a new empty coalescer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new snapshot for `portal_key`.
    ///
    /// If a snapshot is already pending for this key, the newer one replaces it
    /// (latest-wins coalescing). The byte length of `payload` is clamped to
    /// [`MAX_PORTAL_SNAPSHOT_BYTES`] before storage.
    ///
    /// ## Byte-cap semantics (follow-tail alignment)
    ///
    /// When `payload.len() > MAX_PORTAL_SNAPSHOT_BYTES`, the **oldest (head)**
    /// content is dropped to preserve the newest (tail) content. For streaming
    /// transcripts this is correct: the consumer follows the tail, so the newest
    /// content must always be kept. The drop point is walked forward to a valid
    /// UTF-8 character boundary to avoid producing invalid UTF-8.
    ///
    /// ## Stale-sequence guard
    ///
    /// `sequence` must be strictly greater than both the currently-pending
    /// snapshot sequence AND the last-drained sequence for this portal key.
    /// An incoming snapshot with `sequence ≤ max(pending.sequence,
    /// last_drained_sequence)` is silently dropped (stale update).
    ///
    /// This prevents stale appends from being accepted after a drain: without
    /// this guard, a sequence that was fresh relative to the `None`-pending
    /// branch (key absent from `pending` after drain) could slip through.
    ///
    /// `submitted_at_us` is the wall-clock submission timestamp in microseconds.
    ///
    /// Returns `true` if the snapshot was recorded, `false` if it was dropped
    /// as stale.
    pub fn record_append(
        &mut self,
        portal_key: &str,
        payload: Vec<u8>,
        sequence: u64,
        submitted_at_us: u64,
    ) -> bool {
        // Byte-cap: drop oldest (head) content to keep newest (tail).
        // This aligns with follow-tail / tail-anchored presentation direction.
        let payload = if payload.len() > MAX_PORTAL_SNAPSHOT_BYTES {
            // Skip the oldest bytes, finding a valid UTF-8 start boundary.
            let drop_bytes = payload.len() - MAX_PORTAL_SNAPSHOT_BYTES;
            let mut start = drop_bytes;
            if let Ok(s) = std::str::from_utf8(&payload) {
                // Walk forward from `start` until a char boundary is found.
                while start < s.len() && !s.is_char_boundary(start) {
                    start += 1;
                }
            }
            payload[start..].to_vec()
        } else {
            payload
        };

        match self.pending.get_mut(portal_key) {
            Some(existing) => {
                if sequence <= existing.sequence {
                    // Stale relative to pending snapshot — drop.
                    return false;
                }
                // Replace in-place (latest-wins).
                existing.payload = payload;
                existing.sequence = sequence;
                existing.submitted_at_us = submitted_at_us;
                self.total_coalesced += 1;
            }
            None => {
                // No pending snapshot. Guard against stale appends post-drain:
                // reject if sequence ≤ last drained sequence for this key.
                if let Some(&last_drained) = self.last_drained_sequence.get(portal_key) {
                    if sequence <= last_drained {
                        // Stale relative to last drain — drop.
                        return false;
                    }
                }
                // New snapshot for this key. Add to service order only if the
                // key is not already present (it may have been drained via
                // take_snapshot but remains in service_order for round-robin).
                if !self.service_order.iter().any(|k| k == portal_key) {
                    self.service_order.push_back(portal_key.to_string());
                }
                self.pending.insert(
                    portal_key.to_string(),
                    PendingPortalSnapshot {
                        payload,
                        sequence,
                        submitted_at_us,
                    },
                );
            }
        }
        true
    }

    /// Return the next portal key that has a pending snapshot, advancing the
    /// round-robin pointer.
    ///
    /// Returns `None` when no portal has a pending snapshot (coalescer is idle).
    ///
    /// The service order is maintained as a `VecDeque`. The front key is
    /// inspected first; if it has no pending snapshot (already drained), it is
    /// moved to the back and the next key is tried. At most `N` keys (where N
    /// is the total number of registered portals) are inspected per call.
    /// This is O(N) in the worst case, but N is bounded by the number of
    /// concurrent portals (typically ≤ 8 in practice).
    pub fn next_ready_portal(&mut self) -> Option<String> {
        let n = self.service_order.len();
        for _ in 0..n {
            let key = self.service_order.pop_front()?;
            if self.pending.contains_key(&key) {
                // Found a ready portal. Rotate the key to the back so the next
                // call services a different portal (round-robin fairness).
                self.service_order.push_back(key.clone());
                return Some(key);
            } else {
                // Portal has no pending snapshot; move to back and continue.
                self.service_order.push_back(key);
            }
        }
        None
    }

    /// Take (consume) the pending snapshot for `portal_key`, if present.
    ///
    /// Returns the payload bytes and sequence, or `None` if no snapshot is
    /// pending. After this call, `portal_key` has no pending snapshot until
    /// the next `record_append`.
    ///
    /// The drained sequence is recorded in `last_drained_sequence` so that
    /// subsequent [`record_append`] calls with stale sequences are rejected
    /// even when the `pending` map has no entry for this key.
    pub fn take_snapshot(&mut self, portal_key: &str) -> Option<(Vec<u8>, u64)> {
        if let Some(snap) = self.pending.remove(portal_key) {
            // Record the drained sequence for the post-drain stale guard.
            // Use get_mut to avoid allocating a String for the key on the hot
            // path when an entry already exists.
            if let Some(v) = self.last_drained_sequence.get_mut(portal_key) {
                if snap.sequence > *v {
                    *v = snap.sequence;
                }
            } else {
                self.last_drained_sequence
                    .insert(portal_key.to_string(), snap.sequence);
            }
            self.total_taken += 1;
            Some((snap.payload, snap.sequence))
        } else {
            None
        }
    }

    /// Peek at the submitted_at timestamp of the current pending snapshot for
    /// `portal_key` without consuming it. Returns `None` if no snapshot is
    /// pending.
    pub fn peek_submitted_at(&self, portal_key: &str) -> Option<u64> {
        self.pending.get(portal_key).map(|s| s.submitted_at_us)
    }

    /// Returns `true` if the coalescer has any pending work.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Returns the number of portal keys currently registered (with or without
    /// pending snapshots).
    pub fn portal_count(&self) -> usize {
        self.service_order.len()
    }

    /// Returns the number of portal keys that currently have a pending snapshot.
    pub fn pending_portal_count(&self) -> usize {
        self.pending.len()
    }

    /// Diagnostic: number of snapshots taken since creation.
    pub fn total_taken(&self) -> u64 {
        self.total_taken
    }

    /// Diagnostic: number of snapshots coalesced (superseded before being taken).
    pub fn total_coalesced(&self) -> u64 {
        self.total_coalesced
    }

    /// Remove a portal key from the service order, discarding any pending
    /// snapshot and clearing the drained-sequence tracking entry.
    /// Called when a portal session ends.
    pub fn remove_portal(&mut self, portal_key: &str) {
        self.pending.remove(portal_key);
        self.service_order.retain(|k| k != portal_key);
        self.last_drained_sequence.remove(portal_key);
    }

    /// Drain all pending snapshots in service order (round-robin), returning
    /// them as a vec of `(portal_key, payload, sequence)` tuples.
    ///
    /// Used at frame boundary when all pending mutations should be consumed.
    /// After this call, the coalescer is idle.
    pub fn drain_all(&mut self) -> Vec<(String, Vec<u8>, u64)> {
        let mut out = Vec::with_capacity(self.pending.len());
        while let Some(key) = self.next_ready_portal() {
            if let Some((payload, seq)) = self.take_snapshot(&key) {
                out.push((key, payload, seq));
            }
        }
        out
    }
}

// ─── CadenceWorkload ──────────────────────────────────────────────────────────

/// Normative cadence workload generator for harness tests (tasks.md §5.2).
///
/// Generates the two normative workloads:
///
/// 1. **Sustained**: appends totaling ≥ `CADENCE_MIN_SCALARS_PER_SEC` Unicode
///    scalars per second, delivered in ≥ `CADENCE_MIN_INCREMENTS_PER_SEC`
///    increments per second.
///
/// 2. **Burst**: ≥ `CADENCE_BURST_BYTES` bytes arriving within
///    `CADENCE_BURST_WINDOW_MS` milliseconds — representative of a
///    tool-output flush.
///
/// All timestamps are in microseconds.
#[derive(Debug)]
pub struct CadenceWorkload {
    /// Simulated wall-clock time (µs), advanced by `tick`.
    pub now_us: u64,
    /// Accumulated scalar count in the current measurement window.
    pub scalars_this_window: u64,
    /// Accumulated increment count in the current measurement window.
    pub increments_this_window: u64,
    /// Window start (µs).
    pub window_start_us: u64,
}

impl CadenceWorkload {
    /// Create a new workload generator starting at time 0.
    pub fn new() -> Self {
        Self {
            now_us: 0,
            scalars_this_window: 0,
            increments_this_window: 0,
            window_start_us: 0,
        }
    }

    /// Advance the simulated clock by `delta_us` microseconds.
    pub fn tick(&mut self, delta_us: u64) {
        self.now_us += delta_us;
    }

    /// Record a transcript append of `scalar_count` Unicode scalars.
    pub fn record_append(&mut self, scalar_count: u64) {
        self.scalars_this_window += scalar_count;
        self.increments_this_window += 1;
    }

    /// Check whether the current 1-second window satisfies the sustained
    /// cadence requirement. Resets the window counters on a new second.
    ///
    /// Returns `true` if ≥ `CADENCE_MIN_SCALARS_PER_SEC` scalars AND
    /// ≥ `CADENCE_MIN_INCREMENTS_PER_SEC` increments were delivered in the
    /// most-recent 1-second window.
    pub fn window_passes_sustained(&mut self) -> bool {
        let window_us = 1_000_000u64;
        if self.now_us >= self.window_start_us + window_us {
            let passes = self.scalars_this_window >= CADENCE_MIN_SCALARS_PER_SEC
                && self.increments_this_window >= CADENCE_MIN_INCREMENTS_PER_SEC;
            self.window_start_us = self.now_us;
            self.scalars_this_window = 0;
            self.increments_this_window = 0;
            passes
        } else {
            false
        }
    }

    /// Build a sustained-stream payload for `increment_count` increments
    /// distributed across `total_duration_us` simulated microseconds.
    ///
    /// Returns a vec of `(timestamp_us, payload_bytes, scalar_count)` tuples
    /// that, when submitted in order, satisfy the sustained cadence requirement
    /// for `duration_secs` seconds.
    ///
    /// Parameters:
    /// - `scalars_per_sec`: scalar rate (≥ `CADENCE_MIN_SCALARS_PER_SEC`)
    /// - `increments_per_sec`: increment rate (≥ `CADENCE_MIN_INCREMENTS_PER_SEC`)
    /// - `duration_secs`: how many simulated seconds of work to generate
    pub fn build_sustained_stream(
        scalars_per_sec: u64,
        increments_per_sec: u64,
        duration_secs: u64,
    ) -> Vec<(u64, Vec<u8>, u64)> {
        assert!(scalars_per_sec >= CADENCE_MIN_SCALARS_PER_SEC);
        assert!(increments_per_sec >= CADENCE_MIN_INCREMENTS_PER_SEC);
        assert!(duration_secs >= 1);

        let total_increments = increments_per_sec * duration_secs;
        let scalars_per_increment = scalars_per_sec.div_ceil(increments_per_sec);
        let interval_us = 1_000_000u64 / increments_per_sec;

        let mut out = Vec::with_capacity(total_increments as usize);
        for i in 0..total_increments {
            let ts = i * interval_us;
            // Use ASCII 'a' (1 byte / 1 scalar) for simplicity.
            let payload = vec![b'a'; scalars_per_increment as usize];
            out.push((ts, payload, scalars_per_increment));
        }
        out
    }

    /// Build a burst payload: a single `CADENCE_BURST_BYTES`-sized chunk
    /// submitted at `start_us`, representing a tool-output flush.
    pub fn build_burst(start_us: u64) -> (u64, Vec<u8>, u64) {
        let payload = vec![b'x'; CADENCE_BURST_BYTES];
        let scalar_count = CADENCE_BURST_BYTES as u64; // ASCII = 1 byte/scalar
        (start_us, payload, scalar_count)
    }
}

impl Default for CadenceWorkload {
    fn default() -> Self {
        Self::new()
    }
}

// ─── FairnessProbe ────────────────────────────────────────────────────────────

/// Measures cross-portal fairness for verification (tasks.md §5.4).
///
/// Records the number of snapshots served per portal key and verifies that,
/// under equal sustained input rates, service counts do not diverge by more
/// than one complete round (the maximum structural guarantee of round-robin).
///
/// "Unbounded divergence" is defined as any portal receiving more than
/// `total_service_rounds * (1 + 1/N)` services while another receives fewer
/// than `total_service_rounds * (1 - 1/N)`, where N is the portal count.
/// For practical purposes the test asserts `max_count - min_count ≤ portal_count`.
#[derive(Debug, Default)]
pub struct FairnessProbe {
    service_counts: HashMap<String, u64>,
}

impl FairnessProbe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-register a portal key so that portals with zero services are
    /// included in the fairness check.
    ///
    /// Without pre-registration, a portal that is completely starved (receives
    /// 0 services) will be absent from `service_counts`, causing `assert_fair`
    /// to see fewer portals than expected and potentially return `Ok(())`
    /// incorrectly. Call this for every portal that participates in the
    /// workload before beginning the fairness measurement.
    pub fn register_portal(&mut self, portal_key: &str) {
        self.service_counts
            .entry(portal_key.to_string())
            .or_insert(0);
    }

    /// Record one service event for `portal_key`.
    pub fn record_service(&mut self, portal_key: &str) {
        *self
            .service_counts
            .entry(portal_key.to_string())
            .or_insert(0) += 1;
    }

    /// Number of distinct portals seen.
    pub fn portal_count(&self) -> usize {
        self.service_counts.len()
    }

    /// Returns `(min_services, max_services)` across all registered portals.
    pub fn service_range(&self) -> (u64, u64) {
        let min = self.service_counts.values().copied().min().unwrap_or(0);
        let max = self.service_counts.values().copied().max().unwrap_or(0);
        (min, max)
    }

    /// Returns the service count for `portal_key`.
    pub fn count_for(&self, portal_key: &str) -> u64 {
        self.service_counts.get(portal_key).copied().unwrap_or(0)
    }

    /// Verify the round-robin fairness bound:
    ///
    /// `max_services - min_services ≤ portal_count`
    ///
    /// Returns `Ok(())` on pass, `Err(message)` with diagnostic detail on fail.
    pub fn assert_fair(&self) -> Result<(), String> {
        if self.service_counts.len() < 2 {
            return Ok(()); // Single portal is trivially fair.
        }
        let (min, max) = self.service_range();
        let n = self.service_counts.len() as u64;
        if max.saturating_sub(min) <= n {
            Ok(())
        } else {
            Err(format!(
                "fairness violated: max={max} min={min} spread={} > portal_count={n}; \
                 counts={:?}",
                max.saturating_sub(min),
                self.service_counts
            ))
        }
    }
}

// ─── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PortalCadenceCoalescer ───────────────────────────────────────────────

    #[test]
    fn single_portal_snapshot_roundtrip() {
        let mut c = PortalCadenceCoalescer::new();
        c.record_append("portal://a", b"hello world".to_vec(), 1, 1000);
        let key = c.next_ready_portal().expect("portal should be ready");
        assert_eq!(key, "portal://a");
        let (payload, seq) = c.take_snapshot(&key).expect("snapshot should be present");
        assert_eq!(payload, b"hello world");
        assert_eq!(seq, 1);
        assert!(!c.has_pending());
    }

    #[test]
    fn latest_wins_coalescing() {
        let mut c = PortalCadenceCoalescer::new();
        c.record_append("portal://a", b"v1".to_vec(), 1, 100);
        c.record_append("portal://a", b"v2".to_vec(), 2, 200);
        c.record_append("portal://a", b"v3".to_vec(), 3, 300);
        let key = c.next_ready_portal().unwrap();
        let (payload, seq) = c.take_snapshot(&key).unwrap();
        assert_eq!(payload, b"v3", "latest snapshot must win");
        assert_eq!(seq, 3);
        assert_eq!(
            c.total_coalesced(),
            2,
            "two intermediate snapshots coalesced"
        );
    }

    #[test]
    fn stale_sequence_dropped() {
        let mut c = PortalCadenceCoalescer::new();
        c.record_append("portal://a", b"newer".to_vec(), 5, 500);
        let accepted = c.record_append("portal://a", b"older".to_vec(), 3, 300);
        assert!(!accepted, "stale sequence must be dropped");
        let key = c.next_ready_portal().unwrap();
        let (payload, _) = c.take_snapshot(&key).unwrap();
        assert_eq!(payload, b"newer");
    }

    #[test]
    fn round_robin_two_portals() {
        let mut c = PortalCadenceCoalescer::new();
        // Alternate appends: A, B, A, B
        c.record_append("portal://a", b"a1".to_vec(), 1, 100);
        c.record_append("portal://b", b"b1".to_vec(), 1, 100);

        let first = c.next_ready_portal().unwrap();
        let second = c.next_ready_portal().unwrap();
        assert_ne!(first, second, "round-robin must alternate portals");
    }

    #[test]
    fn round_robin_fairness_under_equal_rates() {
        let mut c = PortalCadenceCoalescer::new();
        let mut probe = FairnessProbe::new();

        // 4 portals, 100 rounds each
        let keys = ["portal://a", "portal://b", "portal://c", "portal://d"];
        for round in 0u64..100 {
            // Each portal gets one append per round (equal rates).
            for key in keys {
                c.record_append(
                    key,
                    format!("snap-{round}").into_bytes(),
                    round + 1,
                    round * 1000,
                );
            }
            // Drain all pending snapshots for this round (simulates one frame).
            let served = c.drain_all();
            for (key, _, _) in served {
                probe.record_service(&key);
            }
        }

        probe
            .assert_fair()
            .expect("round-robin must be fair under equal rates");
    }

    #[test]
    fn work_conserving_no_idle_under_load() {
        let mut c = PortalCadenceCoalescer::new();
        // Submit 10 rounds of appends for 2 portals.
        for round in 0u64..10 {
            c.record_append("portal://x", vec![0u8; 16], round, round * 100);
            c.record_append("portal://y", vec![1u8; 16], round, round * 100);
        }
        // All 10 rounds coalesced to 1 each; should drain 2 total.
        let drained = c.drain_all();
        assert_eq!(
            drained.len(),
            2,
            "work-conserving: must drain one snapshot per portal (latest)"
        );
    }

    #[test]
    fn portal_removal_clears_pending() {
        let mut c = PortalCadenceCoalescer::new();
        c.record_append("portal://a", b"payload".to_vec(), 1, 0);
        c.remove_portal("portal://a");
        assert!(!c.has_pending());
        assert_eq!(c.portal_count(), 0);
    }

    #[test]
    fn drain_all_returns_round_robin_order() {
        let mut c = PortalCadenceCoalescer::new();
        // Insert in order: a, b, c.
        c.record_append("portal://a", b"a".to_vec(), 1, 0);
        c.record_append("portal://b", b"b".to_vec(), 1, 0);
        c.record_append("portal://c", b"c".to_vec(), 1, 0);
        let drained = c.drain_all();
        let keys: Vec<&str> = drained.iter().map(|(k, _, _)| k.as_str()).collect();
        // Round-robin order must match insertion order.
        assert_eq!(keys, vec!["portal://a", "portal://b", "portal://c"]);
    }

    #[test]
    fn snapshot_bytes_clamped_to_max() {
        let mut c = PortalCadenceCoalescer::new();
        let large = vec![b'x'; MAX_PORTAL_SNAPSHOT_BYTES + 100];
        c.record_append("portal://a", large, 1, 0);
        let key = c.next_ready_portal().unwrap();
        let (payload, _) = c.take_snapshot(&key).unwrap();
        assert_eq!(payload.len(), MAX_PORTAL_SNAPSHOT_BYTES);
    }

    /// Byte-cap must drop the oldest (head) bytes, not the newest (tail).
    ///
    /// For streaming transcripts the consumer follows the tail. Dropping the
    /// head (oldest content) preserves the newest content that the viewer cares
    /// about. This is the inverse of the prior behaviour (which dropped the tail).
    #[test]
    fn byte_cap_drops_oldest_head_content_not_tail() {
        let mut c = PortalCadenceCoalescer::new();
        // Build a payload just over the limit:
        //   [ old content | sentinel tail ]
        // Old content = MAX_PORTAL_SNAPSHOT_BYTES bytes of 'o'
        // Sentinel tail = 8 bytes of 'N' (NEW content) that must survive the cap.
        const SENTINEL: &[u8] = b"NEWEST!!";
        let mut payload = vec![b'o'; MAX_PORTAL_SNAPSHOT_BYTES];
        payload.extend_from_slice(SENTINEL);
        // payload.len() == MAX_PORTAL_SNAPSHOT_BYTES + 8

        c.record_append("portal://a", payload, 1, 0);
        let key = c.next_ready_portal().unwrap();
        let (stored, _) = c.take_snapshot(&key).unwrap();

        // Stored payload must be exactly MAX_PORTAL_SNAPSHOT_BYTES bytes.
        assert_eq!(stored.len(), MAX_PORTAL_SNAPSHOT_BYTES);
        // The tail (SENTINEL) must be present — it's the newest content.
        assert!(
            stored.ends_with(SENTINEL),
            "byte-cap must keep newest tail content; last 8 bytes: {:?}",
            &stored[stored.len() - 8..]
        );
        // The very first byte must be 'o' (oldest surviving content after cap),
        // not 'N' (sentinel). Confirms oldest content was dropped, not newest.
        assert_eq!(
            stored[0], b'o',
            "first byte after cap must be old content, not new"
        );
    }

    /// After draining a snapshot, a stale sequence (≤ last drained) must be
    /// rejected even when there is no pending snapshot for the portal key.
    ///
    /// Previously, the `None` branch in `record_append` would accept any sequence
    /// when the key was absent from `pending` (which happens post-drain). This
    /// guarded that path.
    #[test]
    fn stale_sequence_rejected_after_drain() {
        let mut c = PortalCadenceCoalescer::new();

        // Record and drain snapshot at sequence 10.
        c.record_append("portal://a", b"snap-10".to_vec(), 10, 1_000);
        let key = c.next_ready_portal().unwrap();
        let (_, drained_seq) = c.take_snapshot(&key).unwrap();
        assert_eq!(drained_seq, 10);

        // The portal key is no longer in `pending` (drained).
        assert!(!c.has_pending());

        // Now try to record a stale sequence (≤ last drained = 10).
        let accepted_equal = c.record_append("portal://a", b"stale-10".to_vec(), 10, 2_000);
        assert!(
            !accepted_equal,
            "sequence == last_drained must be rejected post-drain"
        );

        let accepted_lower = c.record_append("portal://a", b"stale-5".to_vec(), 5, 2_000);
        assert!(
            !accepted_lower,
            "sequence < last_drained must be rejected post-drain"
        );

        // A fresh sequence (> last drained) must be accepted.
        let accepted_fresh = c.record_append("portal://a", b"fresh-11".to_vec(), 11, 3_000);
        assert!(accepted_fresh, "sequence > last_drained must be accepted");
        assert!(c.has_pending(), "fresh append must create a pending entry");
    }

    // ── CadenceWorkload ──────────────────────────────────────────────────────

    #[test]
    fn build_sustained_stream_meets_rate() {
        let stream = CadenceWorkload::build_sustained_stream(200, 10, 1);
        // 10 increments, each with ceil(200/10) = 20 scalars.
        assert_eq!(stream.len(), 10);
        let total_scalars: u64 = stream.iter().map(|(_, _, n)| n).sum();
        assert!(
            total_scalars >= CADENCE_MIN_SCALARS_PER_SEC,
            "sustained stream must carry ≥{CADENCE_MIN_SCALARS_PER_SEC} scalars"
        );
    }

    #[test]
    fn build_burst_meets_size() {
        let (_, payload, _scalar_count) = CadenceWorkload::build_burst(0);
        assert_eq!(
            payload.len(),
            CADENCE_BURST_BYTES,
            "burst payload must be exactly {CADENCE_BURST_BYTES} bytes"
        );
    }

    #[test]
    fn workload_window_tracking() {
        let mut wl = CadenceWorkload::new();
        // Simulate 10 appends of 20 scalars each in 1 second.
        let interval = 100_000u64; // 100ms
        for _ in 0..10 {
            wl.record_append(20);
            wl.tick(interval);
        }
        // At 1s mark window resets; check window passes.
        assert!(
            wl.window_passes_sustained(),
            "10 increments * 20 scalars = 200 scalars/s must pass the sustained check"
        );
    }

    // ── FairnessProbe ────────────────────────────────────────────────────────

    #[test]
    fn fairness_probe_passes_balanced_counts() {
        let mut probe = FairnessProbe::new();
        for _ in 0..100 {
            probe.record_service("portal://a");
            probe.record_service("portal://b");
        }
        probe.assert_fair().unwrap();
    }

    #[test]
    fn fairness_probe_fails_on_starvation() {
        let mut probe = FairnessProbe::new();
        for _ in 0..100 {
            probe.record_service("portal://a");
        }
        probe.record_service("portal://b"); // heavily unbalanced
        let result = probe.assert_fair();
        assert!(
            result.is_err(),
            "starvation of 99 services must fail the fairness check"
        );
    }
}
