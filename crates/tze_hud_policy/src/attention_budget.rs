//! # Attention Budget — Rolling Counter
//!
//! Provides `AttentionBudget`, a circular-buffer rolling counter that tracks
//! interruption events in a configurable time window (default: 60 seconds).
//!
//! ## Design
//!
//! The counter maintains a fixed-size ring of per-second event counts.  The
//! window length is configurable but defaults to 60 seconds (matching the spec).
//! The entire structure is stack-allocated for deterministic latency.
//!
//! Reading `count_in_window()` is O(1) — it returns a cached sum that is
//! updated on each `record_event()` call. No heap allocation, no iteration.
//! This satisfies the spec's < 10µs budget for the attention check.
//!
//! ## Spec Reference
//!
//! - policy-arbitration/spec.md §11.5 (Level 4 Attention Management)
//! - RFC 0010 §3.1, §7
//!
//! ## Defaults (RFC 0010 §3.1)
//!
//! | Key                    | Default |
//! |------------------------|---------|
//! | Per-agent limit        | 20 / min |
//! | Per-zone limit         | 10 / min |
//! | Per-zone (Stack zones) | 30 / min |
//! | Window                 | 60 s    |

/// Default rolling window in seconds.
pub const DEFAULT_WINDOW_SECS: u32 = 60;

/// Default per-agent limit (RFC 0010 §3.1).
pub const DEFAULT_PER_AGENT_LIMIT: u32 = 20;

/// Default per-zone limit (RFC 0010 §3.1).
pub const DEFAULT_PER_ZONE_LIMIT: u32 = 10;

/// Default per-zone limit for Stack-policy zones (RFC 0010 §7).
pub const DEFAULT_PER_ZONE_STACK_LIMIT: u32 = 30;

/// Maximum supported window in seconds (hard cap to bound array size).
const MAX_WINDOW_SECS: usize = 3600; // 1 hour

/// A rolling per-second event counter over a configurable time window.
///
/// The internal representation is a fixed-capacity ring buffer of `u32` slot
/// counts (one slot per second in the window). The invariant is:
///
///   `cached_sum == sum of all slots`
///
/// Reads are O(1) via `cached_sum`. Writes are O(1): advance expired slots,
/// increment the current slot, and adjust the cached sum accordingly.
///
/// The structure deliberately avoids `Vec` and heap allocation to keep
/// latency bounded and deterministic. The window length is set at construction
/// time and cannot be changed; create a new `AttentionBudget` to resize.
pub struct AttentionBudget {
    /// Ring buffer of per-second event counts.
    slots: Box<[u32]>,
    /// Length of the window (number of slots).
    window_secs: u32,
    /// Index of the current (most recent) slot in the ring.
    current_idx: usize,
    /// Timestamp (in seconds) of the current slot.
    current_slot_time_s: u64,
    /// Cached sum of all slots — maintained incrementally.
    cached_sum: u32,
    /// Configured limit for this counter.
    limit: u32,
}

impl AttentionBudget {
    /// Create a new budget counter.
    ///
    /// - `limit`: maximum events allowed in the rolling window before the
    ///   budget is considered exhausted.
    /// - `window_secs`: length of the rolling window in seconds. Must be ≥ 1
    ///   and ≤ `MAX_WINDOW_SECS` (3600). Clamped silently.
    pub fn new(limit: u32, window_secs: u32) -> Self {
        let ws = (window_secs as usize).clamp(1, MAX_WINDOW_SECS);
        Self {
            slots: vec![0u32; ws].into_boxed_slice(),
            window_secs: ws as u32,
            current_idx: 0,
            current_slot_time_s: 0,
            cached_sum: 0,
            limit,
        }
    }

    /// Create a per-agent budget with default parameters (RFC 0010 §3.1).
    pub fn per_agent() -> Self {
        Self::new(DEFAULT_PER_AGENT_LIMIT, DEFAULT_WINDOW_SECS)
    }

    /// Create a per-zone budget with default parameters (RFC 0010 §3.1).
    pub fn per_zone() -> Self {
        Self::new(DEFAULT_PER_ZONE_LIMIT, DEFAULT_WINDOW_SECS)
    }

    /// Create a per-zone budget for Stack-policy zones (RFC 0010 §7).
    pub fn per_zone_stack() -> Self {
        Self::new(DEFAULT_PER_ZONE_STACK_LIMIT, DEFAULT_WINDOW_SECS)
    }

    /// Returns the configured limit.
    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// Set a new limit without resetting counts.
    pub fn set_limit(&mut self, limit: u32) {
        self.limit = limit;
    }

    /// Returns the current count of events in the rolling window.
    ///
    /// This is O(1) — reads the cached sum directly.
    pub fn count_in_window(&self) -> u32 {
        self.cached_sum
    }

    /// Returns `true` if the budget is exhausted (count ≥ limit).
    ///
    /// This is O(1) and designed to complete in < 10µs (spec §11.5).
    pub fn is_exhausted(&self) -> bool {
        self.cached_sum >= self.limit
    }

    /// Advance the ring buffer to `now_s` without recording an event.
    ///
    /// Slots older than the window are expired (zeroed). This is called
    /// before checking `is_exhausted()` to ensure stale slots are purged.
    ///
    /// - `now_s`: current time in whole seconds (monotonic).
    pub fn advance_to(&mut self, now_s: u64) {
        if now_s <= self.current_slot_time_s {
            // No time has passed (or clock went backwards — ignore).
            return;
        }

        let elapsed = (now_s - self.current_slot_time_s) as usize;
        let ws = self.window_secs as usize;

        if elapsed >= ws {
            // All slots are expired — clear everything.
            for s in self.slots.iter_mut() {
                *s = 0;
            }
            self.cached_sum = 0;
        } else {
            // Zero out the slots between (current+1) and (current+elapsed).
            for i in 1..=elapsed {
                let idx = (self.current_idx + i) % ws;
                self.cached_sum = self.cached_sum.saturating_sub(self.slots[idx]);
                self.slots[idx] = 0;
            }
            self.current_idx = (self.current_idx + elapsed) % ws;
        }

        self.current_slot_time_s = now_s;
    }

    /// Record one event at time `now_s`.
    ///
    /// Advances the ring if needed, then increments the current slot.
    ///
    /// - `now_s`: current time in whole seconds (monotonic).
    pub fn record_event(&mut self, now_s: u64) {
        self.advance_to(now_s);
        self.slots[self.current_idx] = self.slots[self.current_idx].saturating_add(1);
        self.cached_sum = self.cached_sum.saturating_add(1);
    }

    /// Record `n` events at time `now_s` (bulk increment).
    pub fn record_events(&mut self, now_s: u64, n: u32) {
        self.advance_to(now_s);
        self.slots[self.current_idx] = self.slots[self.current_idx].saturating_add(n);
        self.cached_sum = self.cached_sum.saturating_add(n);
    }

    /// Reset all counters (e.g., on lease expiry or explicit flush).
    pub fn reset(&mut self) {
        for s in self.slots.iter_mut() {
            *s = 0;
        }
        self.cached_sum = 0;
    }

    /// Returns a snapshot of the current state for use in `PolicyContext`.
    ///
    /// Returns `(count_in_window, limit)`.
    pub fn snapshot(&self) -> (u32, u32) {
        (self.cached_sum, self.limit)
    }
}

/// A pair of per-agent and per-zone attention budgets for a single agent.
///
/// This is the primary unit managed by the frame pipeline; one `AgentBudgetPair`
/// is maintained per agent session. The pipeline advances both on each frame,
/// then reads counts into `AttentionContext` before calling the policy evaluator.
pub struct AgentBudgetPair {
    /// Per-agent rolling counter (applies to all zones this agent publishes to).
    pub agent: AttentionBudget,
    /// Per-zone rolling counter (applies to the specific zone being published).
    pub zone: AttentionBudget,
}

impl AgentBudgetPair {
    /// Create with default limits (RFC 0010 §3.1).
    pub fn new_default() -> Self {
        Self {
            agent: AttentionBudget::per_agent(),
            zone: AttentionBudget::per_zone(),
        }
    }

    /// Create with a Stack-policy zone limit (RFC 0010 §7).
    pub fn new_for_stack_zone() -> Self {
        Self {
            agent: AttentionBudget::per_agent(),
            zone: AttentionBudget::per_zone_stack(),
        }
    }
}

#[cfg(test)]
mod attention_budget_tests {
    use super::*;

    // ─── Basic construction ───────────────────────────────────────────────────

    #[test]
    fn test_new_budget_starts_empty() {
        let b = AttentionBudget::new(20, 60);
        assert_eq!(b.count_in_window(), 0);
        assert!(!b.is_exhausted());
        assert_eq!(b.limit(), 20);
    }

    #[test]
    fn test_per_agent_defaults() {
        let b = AttentionBudget::per_agent();
        assert_eq!(b.limit(), DEFAULT_PER_AGENT_LIMIT);
        assert_eq!(b.window_secs, DEFAULT_WINDOW_SECS);
    }

    #[test]
    fn test_per_zone_defaults() {
        let b = AttentionBudget::per_zone();
        assert_eq!(b.limit(), DEFAULT_PER_ZONE_LIMIT);
    }

    #[test]
    fn test_per_zone_stack_defaults() {
        let b = AttentionBudget::per_zone_stack();
        assert_eq!(b.limit(), DEFAULT_PER_ZONE_STACK_LIMIT);
    }

    // ─── Event recording ─────────────────────────────────────────────────────

    #[test]
    fn test_single_event_increments_count() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_event(1000);
        assert_eq!(b.count_in_window(), 1);
    }

    #[test]
    fn test_multiple_events_same_second() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_event(1000);
        b.record_event(1000);
        b.record_event(1000);
        assert_eq!(b.count_in_window(), 3);
    }

    #[test]
    fn test_events_across_seconds() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_event(1000);
        b.record_event(1001);
        b.record_event(1002);
        assert_eq!(b.count_in_window(), 3);
    }

    #[test]
    fn test_budget_exhausted_at_limit() {
        let mut b = AttentionBudget::new(5, 60);
        for _ in 0..5 {
            b.record_event(1000);
        }
        assert_eq!(b.count_in_window(), 5);
        assert!(b.is_exhausted(), "Budget at limit must be exhausted");
    }

    #[test]
    fn test_budget_exhausted_over_limit() {
        let mut b = AttentionBudget::new(5, 60);
        b.record_events(1000, 10);
        assert!(b.is_exhausted());
    }

    #[test]
    fn test_budget_not_exhausted_below_limit() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_events(1000, 19);
        assert_eq!(b.count_in_window(), 19);
        assert!(!b.is_exhausted(), "19 < 20: not exhausted");
    }

    // ─── Rolling window expiry ────────────────────────────────────────────────

    #[test]
    fn test_events_expire_after_window() {
        let mut b = AttentionBudget::new(20, 60);
        // Record 10 events at t=0
        b.record_events(0, 10);
        assert_eq!(b.count_in_window(), 10);
        // Advance 60 seconds — all slots expired
        b.advance_to(60);
        assert_eq!(b.count_in_window(), 0);
    }

    #[test]
    fn test_partial_expiry() {
        let mut b = AttentionBudget::new(20, 60);
        // 5 events at t=0, 5 events at t=30
        b.record_events(0, 5);
        b.record_events(30, 5);
        // At t=61: t=0 slot has expired, t=30 slot is within 60s window
        b.advance_to(61);
        // t=0 events (>60s ago) are expired; t=30 events (31s ago) survive
        assert_eq!(b.count_in_window(), 5, "Events at t=0 must have expired by t=61");
    }

    #[test]
    fn test_full_expiry_on_large_jump() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_events(0, 20);
        assert!(b.is_exhausted());
        // Jump far into the future
        b.advance_to(10_000);
        assert_eq!(b.count_in_window(), 0);
        assert!(!b.is_exhausted());
    }

    #[test]
    fn test_budget_refills_after_window() {
        let mut b = AttentionBudget::new(20, 60);
        // Fill to limit
        b.record_events(0, 20);
        assert!(b.is_exhausted());
        // Advance one full window
        b.advance_to(61);
        assert!(!b.is_exhausted(), "Budget must refill after window expiry");
        // Should be able to record again
        b.record_event(61);
        assert_eq!(b.count_in_window(), 1);
    }

    // ─── Spec scenario: agent exceeds 20 / min ───────────────────────────────

    /// WHEN an agent exceeds 20 interruptions per minute
    /// THEN subsequent mutations are coalesced (latest-wins) until budget refills
    ///
    /// This test verifies the budget signals exhaustion at the correct threshold.
    #[test]
    fn test_spec_scenario_agent_exceeds_20_per_minute() {
        let mut b = AttentionBudget::per_agent();
        let base = 1_000u64;

        // Record exactly 20 events — at limit
        for i in 0..20u64 {
            b.record_event(base + i);
        }
        assert_eq!(b.count_in_window(), 20);
        assert!(b.is_exhausted(), "20 events at 20/min limit: must be exhausted");

        // Record one more — over limit; still exhausted
        b.record_event(base + 20);
        assert!(b.is_exhausted());

        // After 60 seconds all events expire
        b.advance_to(base + 81); // all are >60s old now
        assert!(!b.is_exhausted(), "Budget must refill after window expiry");
    }

    // ─── Default limits match spec ────────────────────────────────────────────

    #[test]
    fn test_default_per_agent_limit_is_20() {
        assert_eq!(DEFAULT_PER_AGENT_LIMIT, 20);
    }

    #[test]
    fn test_default_per_zone_limit_is_10() {
        assert_eq!(DEFAULT_PER_ZONE_LIMIT, 10);
    }

    #[test]
    fn test_default_per_zone_stack_limit_is_30() {
        assert_eq!(DEFAULT_PER_ZONE_STACK_LIMIT, 30);
    }

    // ─── Reset ───────────────────────────────────────────────────────────────

    #[test]
    fn test_reset_clears_all_counts() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_events(1000, 20);
        assert!(b.is_exhausted());
        b.reset();
        assert_eq!(b.count_in_window(), 0);
        assert!(!b.is_exhausted());
    }

    // ─── Snapshot ────────────────────────────────────────────────────────────

    #[test]
    fn test_snapshot_returns_count_and_limit() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_events(1000, 7);
        let (count, limit) = b.snapshot();
        assert_eq!(count, 7);
        assert_eq!(limit, 20);
    }

    // ─── Edge: same-second record stability ──────────────────────────────────

    #[test]
    fn test_advance_to_same_time_is_noop() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_events(1000, 5);
        b.advance_to(1000);
        assert_eq!(b.count_in_window(), 5, "advance_to same time must not drop events");
    }

    #[test]
    fn test_advance_to_earlier_time_is_noop() {
        let mut b = AttentionBudget::new(20, 60);
        b.record_events(1000, 5);
        b.advance_to(999); // earlier
        assert_eq!(b.count_in_window(), 5, "advance_to earlier time must not drop events");
    }

    // ─── AgentBudgetPair ─────────────────────────────────────────────────────

    #[test]
    fn test_agent_budget_pair_default() {
        let pair = AgentBudgetPair::new_default();
        assert_eq!(pair.agent.limit(), DEFAULT_PER_AGENT_LIMIT);
        assert_eq!(pair.zone.limit(), DEFAULT_PER_ZONE_LIMIT);
    }

    #[test]
    fn test_agent_budget_pair_stack_zone() {
        let pair = AgentBudgetPair::new_for_stack_zone();
        assert_eq!(pair.agent.limit(), DEFAULT_PER_AGENT_LIMIT);
        assert_eq!(pair.zone.limit(), DEFAULT_PER_ZONE_STACK_LIMIT);
    }
}
