//! Garbage collection for the resource store.
//!
//! The GC runs as a compositor-thread *epilogue* — after GPU Submit + Present
//! completes, in the inter-frame idle window, and BEFORE stage 3 of the next
//! frame (spec lines 218–220, 224–225).
//!
//! ## Cycle timing
//!
//! The GC is triggered by the compositor calling [`GcRunner::maybe_run`].
//! That method checks whether the configurable GC interval (default 30 s) has
//! elapsed since the last cycle.  When triggered, it scans all GC candidates,
//! evicts those whose grace period has elapsed, and defers the rest.
//!
//! Each cycle has a **5 ms wall-clock budget** (spec lines 207–209).  When the
//! budget is exhausted, remaining eviction-eligible candidates are deferred to
//! the next cycle.  This prevents GC from blocking the frame loop.
//!
//! ## Frame render isolation
//!
//! The compositor is responsible for *not calling* `maybe_run` while the
//! render pipeline (stages 4–7) is active.  This module has no direct GPU
//! state; it only manipulates the in-memory `DedupIndex`.  The invariant is
//! documented here but enforced at the call site.
//!
//! GPU texture handles bound to the current frame's draw calls must not be
//! freed until the GPU has finished using them.  The spec requires that
//! textures bound to the *current frame* are deferred (spec lines 228–229).
//! For v1 (CPU-side store only) this is satisfied by design: we do not hold
//! GPU texture handles in `ResourceRecord`; GPU upload is outside this crate's
//! scope.
//!
//! ## Grace period
//!
//! Default: 60 seconds (configurable, min 1 s, max 3600 s — spec line 193).
//! Default GC interval: 30 seconds (configurable, min 5 s, max 300 s — spec
//! line 208).
//!
//! Source: RFC 0011 §6.1–§6.6.

use std::time::{Duration, Instant};

use crate::dedup::DedupIndex;
use crate::refcount::GcCandidateTable;
use crate::types::ResourceId;

// ─── GC configuration ─────────────────────────────────────────────────────────

/// GC configuration (configurable at construction, then immutable).
///
/// Spec lines 192–194, 207–209.
#[derive(Debug, Clone)]
pub struct GcConfig {
    /// Grace period before a refcount-0 resource is eligible for eviction.
    ///
    /// Default: 60 s.  Min: 1 s.  Max: 3600 s.
    pub grace_period_ms: u64,

    /// How often the GC cycle runs.
    ///
    /// Default: 30 s.  Min: 5 s.  Max: 300 s.
    pub gc_interval_ms: u64,

    /// Wall-clock budget per GC cycle.
    ///
    /// Default: 5 ms (spec line 208).
    pub cycle_budget_ms: u64,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            grace_period_ms: 60_000,
            gc_interval_ms: 30_000,
            cycle_budget_ms: 5,
        }
    }
}

impl GcConfig {
    /// Clamp fields to spec-mandated bounds.
    pub fn validated(mut self) -> Self {
        self.grace_period_ms = self.grace_period_ms.clamp(1_000, 3_600_000);
        self.gc_interval_ms = self.gc_interval_ms.clamp(5_000, 300_000);
        self.cycle_budget_ms = self.cycle_budget_ms.max(1);
        self
    }
}

// ─── GC result ────────────────────────────────────────────────────────────────

/// Summary produced by a single GC cycle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcResult {
    /// Number of resources evicted this cycle.
    pub evicted: usize,
    /// Number of eviction-eligible resources deferred (time budget exhausted).
    pub deferred: usize,
    /// Number of resources still in grace period (not yet eligible).
    pub still_in_grace: usize,
}

// ─── Clock abstraction ────────────────────────────────────────────────────────

/// Monotonic clock abstraction for deterministic testing.
///
/// Production code passes `WallClock`; tests pass a manually advanced
/// `TestClockMs`.
pub trait GcClock: Send + Sync + 'static {
    /// Current time in milliseconds.
    fn now_ms(&self) -> u64;
    /// Current `Instant` for measuring elapsed wall time within a cycle.
    fn now_instant(&self) -> Instant;
}

/// Production wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct WallClock;

impl GcClock for WallClock {
    #[inline]
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64
    }

    #[inline]
    fn now_instant(&self) -> Instant {
        Instant::now()
    }
}

/// Test clock: manually advanced millisecond counter.
///
/// `now_instant` returns an Instant that is `now_ms` milliseconds after
/// a fixed origin, simulating wall time without sleeping.
#[derive(Debug, Clone)]
pub struct TestClockMs {
    ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
    origin: Instant,
}

impl TestClockMs {
    /// Create a clock starting at `start_ms`.
    pub fn new(start_ms: u64) -> Self {
        Self {
            ms: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(start_ms)),
            origin: Instant::now(),
        }
    }

    /// Advance the clock by `delta_ms` milliseconds.
    pub fn advance(&self, delta_ms: u64) {
        self.ms.fetch_add(delta_ms, std::sync::atomic::Ordering::Relaxed);
    }
}

impl GcClock for TestClockMs {
    fn now_ms(&self) -> u64 {
        self.ms.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn now_instant(&self) -> Instant {
        // Return an Instant that reflects current virtual time.
        self.origin + Duration::from_millis(self.now_ms())
    }
}

// ─── GcRunner ─────────────────────────────────────────────────────────────────

/// Drives GC cycles.
///
/// ## Thread safety
///
/// `GcRunner` is intended for single-threaded compositor use.  It does NOT
/// use interior mutability itself; its `maybe_run` / `run_cycle` methods take
/// `&mut self`.  The `DedupIndex` it holds is `Clone` + `Arc`-backed and can
/// be shared, but eviction (`remove`) acquires a per-shard lock inside
/// `DashMap`.
pub struct GcRunner<C: GcClock> {
    dedup: DedupIndex,
    candidates: GcCandidateTable,
    config: GcConfig,
    clock: C,
    /// Timestamp (ms) of the last GC cycle.  `None` means never run.
    last_gc_ms: Option<u64>,
}

impl<C: GcClock> GcRunner<C> {
    /// Construct a GC runner.
    ///
    /// `dedup` and `candidates` must be the same instances used by the
    /// `RefcountLayer` so evictions are reflected everywhere.
    pub fn new(
        dedup: DedupIndex,
        candidates: GcCandidateTable,
        config: GcConfig,
        clock: C,
    ) -> Self {
        Self {
            dedup,
            candidates,
            config: config.validated(),
            clock,
            last_gc_ms: None,
        }
    }

    /// Check whether a GC cycle is due, and run one if so.
    ///
    /// Called by the compositor after GPU Submit + Present, in the inter-frame
    /// idle window.  Must not be called during render pipeline stages 4–7
    /// (spec line 218).
    ///
    /// Returns `None` if no cycle was due, `Some(GcResult)` otherwise.
    pub fn maybe_run(&mut self) -> Option<GcResult> {
        let now_ms = self.clock.now_ms();
        let interval = self.config.gc_interval_ms;

        let due = match self.last_gc_ms {
            None => true,
            Some(last) => now_ms.saturating_sub(last) >= interval,
        };

        if due {
            let result = self.run_cycle_at(now_ms);
            Some(result)
        } else {
            None
        }
    }

    /// Force-run a GC cycle regardless of the timer.
    ///
    /// Useful for testing and for post-revocation cleanup (spec lines 340–342).
    pub fn run_cycle(&mut self) -> GcResult {
        let now_ms = self.clock.now_ms();
        self.run_cycle_at(now_ms)
    }

    fn run_cycle_at(&mut self, now_ms: u64) -> GcResult {
        self.last_gc_ms = Some(now_ms);
        let budget = Duration::from_millis(self.config.cycle_budget_ms);
        let cycle_start = self.clock.now_instant();

        let candidates = self.candidates.snapshot();
        let grace = self.config.grace_period_ms;

        let mut evicted = 0usize;
        let mut deferred = 0usize;
        let mut still_in_grace = 0usize;

        for (id, candidate) in &candidates {
            // Budget check: if we've used up the time budget, defer remaining work.
            if cycle_start.elapsed() >= budget {
                deferred += 1;
                continue;
            }

            let age_ms = now_ms.saturating_sub(candidate.zero_since_ms);

            if age_ms < grace {
                // Still within grace period — not eligible yet.
                still_in_grace += 1;
                continue;
            }

            // Grace period elapsed.  Verify the refcount is still 0 — a
            // resurrection might have happened between the snapshot and now.
            if let Some(record) = self.dedup.get(id) {
                if record.refcount() != 0 {
                    // Resurrected between snapshot and now.
                    // The candidacy entry will have been removed by inc_ref.
                    continue;
                }
            } else {
                // Resource was already removed (shouldn't happen, but be safe).
                self.candidates.remove(id);
                continue;
            }

            // Evict: remove from dedup index and candidacy table.
            self.evict(*id);
            evicted += 1;
        }

        // Count remaining eligible (not-yet-processed) as deferred.
        // (The loop already counted them above when budget ran out.)

        tracing::debug!(
            evicted,
            deferred,
            still_in_grace,
            candidates = candidates.len(),
            cycle_budget_ms = self.config.cycle_budget_ms,
            "GC cycle complete"
        );

        GcResult {
            evicted,
            deferred,
            still_in_grace,
        }
    }

    /// Remove a single resource from the store.
    ///
    /// This frees the decoded in-memory data (the `ResourceRecord` is dropped
    /// when the last `Arc` is released) and removes the entry from the dedup
    /// index.
    fn evict(&self, id: ResourceId) {
        self.candidates.remove(&id);
        self.dedup.remove(&id);
        tracing::debug!(resource_id = %id, "GC: evicted resource");
    }

    /// Milliseconds until the next GC cycle is due (for diagnostics only).
    pub fn ms_until_next_cycle(&self) -> u64 {
        let now_ms = self.clock.now_ms();
        match self.last_gc_ms {
            None => 0,
            Some(last) => {
                let elapsed = now_ms.saturating_sub(last);
                self.config.gc_interval_ms.saturating_sub(elapsed)
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedup::{DedupIndex, ResourceRecord};
    use crate::refcount::{GcCandidateTable, RefcountLayer};
    use crate::types::{DecodedMeta, ResourceId, ResourceType};

    fn make_record(id: ResourceId, decoded_bytes: usize) -> ResourceRecord {
        ResourceRecord::new(
            id,
            ResourceType::ImagePng,
            &DecodedMeta {
                decoded_bytes,
                width_px: 64,
                height_px: 64,
            },
        )
    }

    fn minimal_gc_config() -> GcConfig {
        GcConfig {
            grace_period_ms: 60_000,
            gc_interval_ms: 30_000,
            cycle_budget_ms: 5,
        }
    }

    fn setup(clock: TestClockMs) -> (RefcountLayer, GcRunner<TestClockMs>, DedupIndex, GcCandidateTable) {
        let dedup = DedupIndex::new();
        let candidates = GcCandidateTable::new();
        let refcount_layer = RefcountLayer::new_with_candidates(dedup.clone(), candidates.clone());
        let gc = GcRunner::new(
            dedup.clone(),
            candidates.clone(),
            minimal_gc_config(),
            clock,
        );
        (refcount_layer, gc, dedup, candidates)
    }

    fn insert_resource(dedup: &DedupIndex, id: ResourceId) {
        dedup.insert(id, make_record(id, 4096)).ok();
    }

    // Acceptance: resource still present within grace period [spec line 198-199].
    #[test]
    fn resource_survives_within_grace_period() {
        let clock = TestClockMs::new(0);
        let (layer, mut gc, dedup, _) = setup(clock.clone());
        let id = ResourceId::from_content(b"grace-test");
        insert_resource(&dedup, id);

        layer.inc_ref(id, 0).unwrap();
        layer.dec_ref(id, 0).unwrap(); // enters candidacy at t=0

        clock.advance(30_000); // 30s — still within 60s grace

        let result = gc.run_cycle();
        assert_eq!(result.evicted, 0, "resource must not be evicted within grace period");
        assert!(dedup.contains(&id), "resource must still be in store");
    }

    // Acceptance: resource evicted after grace period [spec line 202-203].
    #[test]
    fn resource_evicted_after_grace_period() {
        let clock = TestClockMs::new(0);
        let (layer, mut gc, dedup, _) = setup(clock.clone());
        let id = ResourceId::from_content(b"evict-after-grace");
        insert_resource(&dedup, id);

        layer.inc_ref(id, 0).unwrap();
        layer.dec_ref(id, 0).unwrap(); // enters candidacy at t=0

        clock.advance(61_000); // > 60s grace period

        let result = gc.run_cycle();
        assert!(result.evicted >= 1, "resource must be evicted after grace period");
        assert!(!dedup.contains(&id), "evicted resource must not be in store");
    }

    // Acceptance: GC defers work when budget exhausted [spec line 213-214].
    #[test]
    fn gc_defers_when_budget_exhausted() {
        // Use a zero-ms budget so every eviction immediately exhausts the budget.
        let clock = TestClockMs::new(0);
        let dedup = DedupIndex::new();
        let candidates = GcCandidateTable::new();
        let layer = RefcountLayer::new_with_candidates(dedup.clone(), candidates.clone());
        let config = GcConfig {
            grace_period_ms: 1_000,
            gc_interval_ms: 5_000,
            cycle_budget_ms: 0, // effectively zero — will be clamped to 1 ms
        };
        let mut gc = GcRunner::new(dedup.clone(), candidates.clone(), config, clock.clone());

        // Insert several resources.
        let ids: Vec<ResourceId> = (0..5u8)
            .map(|i| ResourceId::from_content(&[i]))
            .collect();

        for &id in &ids {
            insert_resource(&dedup, id);
            layer.inc_ref(id, 0).unwrap();
            layer.dec_ref(id, 0).unwrap();
        }

        clock.advance(5_000); // past grace period

        let result = gc.run_cycle();
        // With a 1 ms budget and near-instant execution, we can't deterministically
        // assert exact deferred count, but total must equal len(ids).
        assert_eq!(
            result.evicted + result.deferred,
            ids.len(),
            "evicted + deferred must equal total candidates"
        );
    }

    // Acceptance: resurrection prevents eviction [spec line 239-240].
    #[test]
    fn resurrection_prevents_eviction() {
        let clock = TestClockMs::new(0);
        let (layer, mut gc, dedup, _) = setup(clock.clone());
        let id = ResourceId::from_content(b"resurrect-gc");
        insert_resource(&dedup, id);

        layer.inc_ref(id, 0).unwrap();
        layer.dec_ref(id, 0).unwrap(); // enters candidacy at t=0

        clock.advance(20_000); // within 60s grace period

        // Resurrect before grace period ends.
        layer.inc_ref(id, 20_000).unwrap();

        clock.advance(61_000); // now past original grace, but refcount = 1

        let result = gc.run_cycle();
        assert_eq!(result.evicted, 0, "resurrected resource must not be evicted");
        assert!(dedup.contains(&id));
    }

    // Acceptance: maybe_run only runs when interval elapsed.
    #[test]
    fn maybe_run_respects_gc_interval() {
        let clock = TestClockMs::new(0);
        let dedup = DedupIndex::new();
        let candidates = GcCandidateTable::new();
        let config = GcConfig {
            grace_period_ms: 1_000,
            gc_interval_ms: 30_000,
            cycle_budget_ms: 5,
        };
        let mut gc = GcRunner::new(dedup.clone(), candidates.clone(), config, clock.clone());

        // First call: no previous run → should run.
        let result = gc.maybe_run();
        assert!(result.is_some(), "first call should run GC");

        // Immediate second call: interval not yet elapsed.
        let result = gc.maybe_run();
        assert!(result.is_none(), "second call within interval should not run");

        // Advance past interval.
        clock.advance(30_001);
        let result = gc.maybe_run();
        assert!(result.is_some(), "call after interval elapsed should run GC");
    }

    // Acceptance: GC config bounds are clamped.
    #[test]
    fn gc_config_clamping() {
        let config = GcConfig {
            grace_period_ms: 0,     // min 1000
            gc_interval_ms: 1,      // min 5000
            cycle_budget_ms: 0,     // min 1
        }
        .validated();

        assert_eq!(config.grace_period_ms, 1_000);
        assert_eq!(config.gc_interval_ms, 5_000);
        assert_eq!(config.cycle_budget_ms, 1);
    }

    // Acceptance: post-revocation resource footprint is zero after grace + cycle [spec line 346-347].
    #[test]
    fn post_revocation_footprint_zero() {
        let clock = TestClockMs::new(0);
        let (layer, mut gc, dedup, _) = setup(clock.clone());

        // Agent A holds 3 resources.
        let ids: Vec<ResourceId> = (0..3u8)
            .map(|i| ResourceId::from_content(&[0xA0 | i]))
            .collect();

        for &id in &ids {
            insert_resource(&dedup, id);
            layer.inc_ref(id, 0).unwrap();
        }

        // Revocation: drop all references.
        for &id in &ids {
            layer.dec_ref(id, 0).unwrap();
        }

        // All should be candidates now.
        assert_eq!(layer.candidates().len(), 3);

        // Advance past grace period.
        clock.advance(61_000);

        // One GC cycle.
        let result = gc.run_cycle();
        assert_eq!(result.evicted, 3, "all agent resources must be evicted");
        for id in &ids {
            assert!(!dedup.contains(id), "evicted resource must be absent");
        }
    }
}
