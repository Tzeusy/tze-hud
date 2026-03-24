//! Scene-graph-level reference counting and GC candidacy tracking.
//!
//! This module wraps `DedupIndex` with:
//!
//! - **Refcount operations**: `inc_ref` / `dec_ref` operate on the `AtomicU32`
//!   inside `ResourceRecord`, completing in < 1 µs (spec line 319).
//! - **GC candidacy registry**: a parallel table that records the timestamp at
//!   which each resource's refcount first reached zero.  The GC module polls
//!   this table to determine which resources have outlived their grace period.
//!
//! ## Design
//!
//! The `DedupIndex` itself is the append-only authoritative map of live
//! resources.  This module does NOT modify the dedup index — it only reads
//! records from it and maintains the separate `GcCandidateTable`.
//!
//! Separation of concerns:
//!
//! | Component | Responsibility |
//! |---|---|
//! | `DedupIndex` / `ResourceRecord.refcount` | Atomic refcount per resource |
//! | `GcCandidateTable` | Which resources are at refcount 0, and since when |
//! | `GcRunner` (gc.rs) | Driving eviction decisions on the candidate table |
//! | `RefcountLayer` | Coordinates inc/dec with candidacy table updates |
//!
//! Source: RFC 0011 §4.1–§4.5; spec lines 128–131, 192–194.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::dedup::DedupIndex;
use crate::types::ResourceId;

// ─── GC candidate table ───────────────────────────────────────────────────────

/// Entry in the GC candidate table.
#[derive(Debug, Clone)]
pub struct GcCandidate {
    /// Wall-clock milliseconds when this resource's refcount first reached zero.
    pub zero_since_ms: u64,
}

/// Thread-safe table of resources whose refcount is currently zero.
///
/// Resources enter the table when `dec_ref` brings refcount to 0.
/// Resources leave the table when:
/// - `inc_ref` resurrects them (refcount > 0 again), or
/// - The GC evicts them (grace period elapsed).
///
/// This is a separate, lightweight structure to enable O(n_candidates) GC scans
/// rather than O(n_total_resources) full-store scans.
#[derive(Debug, Clone, Default)]
pub struct GcCandidateTable {
    inner: Arc<Mutex<HashMap<ResourceId, GcCandidate>>>,
}

impl GcCandidateTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `id` as a GC candidate starting at `now_ms`.
    pub fn enter(&self, id: ResourceId, now_ms: u64) {
        let mut guard = self.inner.lock().expect("GcCandidateTable lock");
        guard.entry(id).or_insert(GcCandidate { zero_since_ms: now_ms });
    }

    /// Remove `id` from the candidate table (resurrection or eviction).
    pub fn remove(&self, id: &ResourceId) {
        let mut guard = self.inner.lock().expect("GcCandidateTable lock");
        guard.remove(id);
    }

    /// Snapshot all current candidates.
    ///
    /// Returns a `Vec` so the caller does not hold the lock during the GC
    /// work loop.
    pub fn snapshot(&self) -> Vec<(ResourceId, GcCandidate)> {
        let guard = self.inner.lock().expect("GcCandidateTable lock");
        guard
            .iter()
            .map(|(id, c)| (*id, c.clone()))
            .collect()
    }

    /// Number of resources currently in candidacy.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("GcCandidateTable lock").len()
    }

    /// `true` if no resources are in candidacy.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ─── RefcountLayer ────────────────────────────────────────────────────────────

/// Refcount errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefcountError {
    /// The resource is not in the store.
    NotFound,
    /// Decrementing would take the refcount below zero (underflow bug).
    Underflow,
}

impl std::fmt::Display for RefcountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefcountError::NotFound => write!(f, "resource not found in store"),
            RefcountError::Underflow => write!(f, "refcount underflow — this is a bug"),
        }
    }
}

impl std::error::Error for RefcountError {}

/// Coordinates refcount operations with the GC candidate table.
///
/// All public methods are designed to be called from the compositor thread
/// during mutation commit.  Operations complete in < 1 µs (AtomicU32 CAS plus
/// a `Mutex`-protected HashMap insert/remove).
///
/// The `DedupIndex` owns `ResourceRecord` (and its `AtomicU32 refcount`).
/// This layer adds the candidacy lifecycle on top.
#[derive(Clone, Debug)]
pub struct RefcountLayer {
    dedup: DedupIndex,
    candidates: GcCandidateTable,
}

impl RefcountLayer {
    /// Wrap an existing `DedupIndex` with a fresh candidacy table.
    pub fn new(dedup: DedupIndex) -> Self {
        Self {
            dedup,
            candidates: GcCandidateTable::new(),
        }
    }

    /// Wrap an existing `DedupIndex` and share the given `GcCandidateTable`.
    ///
    /// Use this when the `GcRunner` needs to share the same candidate table
    /// instance.
    pub fn new_with_candidates(dedup: DedupIndex, candidates: GcCandidateTable) -> Self {
        Self { dedup, candidates }
    }

    /// Increment the refcount for `id`.
    ///
    /// If the resource was a GC candidate (refcount 0), removes it from the
    /// candidate table, resurrecting the resource.
    ///
    /// Returns the new refcount.
    ///
    /// # Errors
    ///
    /// - `RefcountError::NotFound` — `id` is not in the store.
    ///
    /// # Timing
    ///
    /// < 1 µs (spec line 319): one `AtomicU32::fetch_add` + conditional
    /// Mutex-guarded `HashMap::remove` (only on resurrection).
    pub fn inc_ref(&self, id: ResourceId, _now_ms: u64) -> Result<u32, RefcountError> {
        let record = self.dedup.get(&id).ok_or(RefcountError::NotFound)?;
        let new_count = record.inc_refcount();
        // If this was a resurrection (previous refcount was 0), remove from candidates.
        // `inc_refcount` returns the *new* value; old value was new_count - 1.
        if new_count == 1 {
            self.candidates.remove(&id);
        }
        Ok(new_count)
    }

    /// Decrement the refcount for `id`.
    ///
    /// If the refcount reaches 0, enters the resource into GC candidacy.
    ///
    /// Returns the new refcount.
    ///
    /// # Errors
    ///
    /// - `RefcountError::NotFound` — `id` is not in the store.
    /// - `RefcountError::Underflow` — refcount was already 0 (bug; panics in
    ///   debug, returns error in release per spec lines 145–148).
    ///
    /// # Timing
    ///
    /// < 1 µs (spec line 319): one `AtomicU32` CAS + conditional Mutex-guarded
    /// `HashMap::insert` (only when reaching 0).
    pub fn dec_ref(&self, id: ResourceId, now_ms: u64) -> Result<u32, RefcountError> {
        let record = self.dedup.get(&id).ok_or(RefcountError::NotFound)?;

        // dec_refcount handles the underflow: panics in debug, clamps and logs
        // in release (spec lines 145–148).  We need to detect underflow to
        // return RefcountError::Underflow to the caller in release builds.
        let current = record.refcount();
        if current == 0 {
            // Already at zero — dec_refcount would underflow.
            debug_assert!(
                false,
                "dec_ref called on resource {} already at refcount 0",
                id
            );
            tracing::error!(
                resource_id = %id,
                "dec_ref underflow — refcount already at 0; this is a bug"
            );
            return Err(RefcountError::Underflow);
        }

        let new_count = record.dec_refcount();

        if new_count == 0 {
            // Resource just entered GC candidacy.
            self.candidates.enter(id, now_ms);
        }

        Ok(new_count)
    }

    /// Current refcount.  `None` if the resource is not in the store.
    #[inline]
    pub fn refcount(&self, id: ResourceId) -> Option<u32> {
        self.dedup.get(&id).map(|r| r.refcount())
    }

    /// `true` if the resource is in the store (whether live or a GC candidate).
    #[inline]
    pub fn contains(&self, id: ResourceId) -> bool {
        self.dedup.contains(&id)
    }

    /// Access the underlying dedup index (for upload operations).
    pub fn dedup_index(&self) -> &DedupIndex {
        &self.dedup
    }

    /// Access the GC candidate table (for the GC runner).
    pub fn candidates(&self) -> &GcCandidateTable {
        &self.candidates
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedup::ResourceRecord;
    use crate::types::{DecodedMeta, ResourceType};

    fn make_refcount_layer() -> RefcountLayer {
        RefcountLayer::new(DedupIndex::new())
    }

    fn insert_resource(layer: &RefcountLayer, id: ResourceId) {
        let record = ResourceRecord::new(
            id,
            ResourceType::ImagePng,
            &DecodedMeta {
                decoded_bytes: 1024,
                width_px: 64,
                height_px: 64,
            },
        );
        layer.dedup_index().insert(id, record).ok();
    }

    // Acceptance: WHEN node created referencing X THEN refcount incremented [spec line 134-135].
    #[test]
    fn inc_ref_increments_refcount() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"inc-test");
        insert_resource(&layer, id);

        assert_eq!(layer.refcount(id), Some(0));
        let new = layer.inc_ref(id, 0).unwrap();
        assert_eq!(new, 1);
        assert_eq!(layer.refcount(id), Some(1));
    }

    // Acceptance: WHEN tile deleted THEN refcount decremented [spec line 138-139].
    #[test]
    fn dec_ref_decrements_refcount() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"dec-test");
        insert_resource(&layer, id);

        layer.inc_ref(id, 0).unwrap();
        layer.inc_ref(id, 0).unwrap(); // refcount = 2
        let new = layer.dec_ref(id, 0).unwrap();
        assert_eq!(new, 1);
    }

    // Acceptance: cross-agent sharing — A and B ref X, A deletes → refcount = 1 [spec line 142-143].
    #[test]
    fn cross_agent_sharing_refcount() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"shared");
        insert_resource(&layer, id);

        layer.inc_ref(id, 0).unwrap(); // agent A
        layer.inc_ref(id, 0).unwrap(); // agent B
        let new = layer.dec_ref(id, 0).unwrap(); // agent A deletes
        assert_eq!(new, 1, "agent B still holds a reference");
        assert!(!layer.candidates().snapshot().iter().any(|(rid, _)| *rid == id),
            "resource must not be GC candidate while refcount > 0");
    }

    // Acceptance: refcount 0 → enters GC candidacy [spec line 192-194].
    #[test]
    fn refcount_zero_enters_gc_candidacy() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"gc-candidate");
        insert_resource(&layer, id);

        layer.inc_ref(id, 0).unwrap();
        layer.dec_ref(id, 1000).unwrap(); // now at 0

        let candidates = layer.candidates().snapshot();
        assert!(
            candidates.iter().any(|(rid, _)| *rid == id),
            "resource should be in GC candidate table"
        );
    }

    // Acceptance: resource resurrected within grace period [spec line 239-240].
    #[test]
    fn resurrection_removes_from_gc_candidacy() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"resurrect");
        insert_resource(&layer, id);

        layer.inc_ref(id, 0).unwrap();
        layer.dec_ref(id, 1000).unwrap(); // enter candidacy

        // Still in store.
        assert!(layer.contains(id));

        // Resurrect.
        let new = layer.inc_ref(id, 20_000).unwrap();
        assert_eq!(new, 1);

        // Must be removed from candidates.
        let candidates = layer.candidates().snapshot();
        assert!(
            !candidates.iter().any(|(rid, _)| *rid == id),
            "resurrected resource must not be in GC candidate table"
        );
    }

    // Acceptance: refcount underflow → error in release, panic in debug [spec line 145-148].
    #[test]
    #[cfg(not(debug_assertions))]
    fn dec_ref_at_zero_returns_underflow_in_release() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"underflow-release");
        insert_resource(&layer, id);

        // refcount is 0 after upload, dec_ref should return Underflow.
        let result = layer.dec_ref(id, 0);
        assert_eq!(result, Err(RefcountError::Underflow));
    }

    #[test]
    #[should_panic]
    #[cfg(debug_assertions)]
    fn dec_ref_at_zero_panics_in_debug() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"underflow-debug");
        insert_resource(&layer, id);

        // Should panic in debug builds (spec line 145-148).
        let _ = layer.dec_ref(id, 0);
    }

    // Acceptance: not-found resource returns error.
    #[test]
    fn inc_ref_not_found() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"not-found");
        assert_eq!(layer.inc_ref(id, 0), Err(RefcountError::NotFound));
    }

    #[test]
    fn dec_ref_not_found() {
        let layer = make_refcount_layer();
        let id = ResourceId::from_content(b"not-found-dec");
        assert_eq!(layer.dec_ref(id, 0), Err(RefcountError::NotFound));
    }
}
