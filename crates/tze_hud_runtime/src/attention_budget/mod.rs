//! # Attention Budget Enforcement
//!
//! Maintains per-agent and per-zone rolling counts of non-silent interruptions
//! and enforces the attention budget rules from the spec.
//!
//! Spec: scene-events/spec.md §Requirement: Attention Budget Enforcement,
//! lines 137-153.
//!
//! ## Defaults
//!
//! | Budget                        | Default  |
//! |-------------------------------|----------|
//! | Per-agent (rolling 1 minute)  | 20 /min  |
//! | Per-zone (rolling 1 minute)   | 10 /min  |
//! | Per-zone for Stack policy     | 30 /min  |
//! | Warning threshold             | 80%      |
//!
//! ## Rules
//!
//! - **CRITICAL** is exempt from all budget counters.
//! - **SILENT** carries zero interruption cost.
//! - At 80% of the limit: emit `AttentionBudgetWarningEvent` to agents
//!   subscribed to `ATTENTION_EVENTS`.
//! - When budget exhausted: mutations proceed, but visual presentation is
//!   coalesced (latest-wins within the coalesce buffer).
//!
//! ## Relationship to the event pipeline
//!
//! The attention budget sits at Stage 3 (Policy Filtering). It is consulted
//! **after** the quiet-hours gate.  HIGH events may pass through quiet hours
//! but are still subject to budget enforcement.

pub mod urgency;

use std::collections::{HashMap, VecDeque};

use tze_hud_scene::events::{EventPayload, EventSource, InterruptionClass, SceneEvent,
                             SceneEventBuilder};

pub use urgency::{EarnedUrgencyConfig, EarnedUrgencyTracker, UrgencyRecord};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Default per-agent interruption budget (interruptions per minute).
pub const DEFAULT_AGENT_BUDGET: u32 = 20;
/// Default per-zone interruption budget (interruptions per minute).
pub const DEFAULT_ZONE_BUDGET: u32 = 10;
/// Default per-zone budget for Stack-policy zones.
pub const DEFAULT_STACK_ZONE_BUDGET: u32 = 30;
/// Budget fraction at which the warning is emitted.
pub const WARNING_FRACTION: f64 = 0.80;
/// Rolling window size in microseconds (1 minute).
pub const ROLLING_WINDOW_US: u64 = 60_000_000;

// ─── Budget check result ──────────────────────────────────────────────────────

/// Result of an attention-budget check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttentionBudgetOutcome {
    /// Under budget — deliver normally.
    Ok,
    /// At or above 80% of budget — warning emitted; still deliver normally.
    Warning,
    /// Budget exhausted — mutations proceed but visuals coalesced.
    Coalesce,
    /// Event is CRITICAL — exempt from budget, always deliver immediately.
    CriticalExempt,
    /// Event is SILENT — zero cost, always deliver.
    SilentPassthrough,
}

// ─── Per-entity budget state ──────────────────────────────────────────────────

#[derive(Debug)]
struct InterruptionWindow {
    /// Rolling timestamps of non-silent, non-critical interruptions in µs.
    timestamps: VecDeque<u64>,
    /// Budget limit (interruptions per window).
    limit: u32,
}

impl InterruptionWindow {
    fn new(limit: u32) -> Self {
        Self {
            timestamps: VecDeque::new(),
            limit,
        }
    }

    /// Expire entries older than the rolling window.
    fn expire(&mut self, now_us: u64) {
        let cutoff = now_us.saturating_sub(ROLLING_WINDOW_US);
        while self.timestamps.front().is_some_and(|&t| t < cutoff) {
            self.timestamps.pop_front();
        }
    }

    /// Current count within window.
    fn count(&self) -> u32 {
        self.timestamps.len() as u32
    }

    /// Record an interruption at `now_us`. Returns current count after recording.
    fn record(&mut self, now_us: u64) -> u32 {
        self.timestamps.push_back(now_us);
        self.count()
    }

    /// Whether the budget is exhausted (count >= limit).
    fn is_exhausted(&self) -> bool {
        self.count() >= self.limit
    }

    /// Whether the budget is at or above the warning threshold (count >= 80% * limit).
    fn is_at_warning(&self) -> bool {
        let warn_threshold = (self.limit as f64 * WARNING_FRACTION).floor() as u32;
        self.count() >= warn_threshold
    }
}

// ─── Attention budget tracker ─────────────────────────────────────────────────

/// Attention budget tracker for the event policy gate.
///
/// Maintains per-agent and per-zone rolling interruption budgets.
///
/// Call [`AttentionBudgetTracker::record`] for every non-CRITICAL, non-SILENT
/// event passing through the pipeline.  Inspect the returned
/// [`AttentionBudgetOutcome`] to determine presentation behavior.
#[derive(Debug, Default)]
pub struct AttentionBudgetTracker {
    agent_budgets: HashMap<String, InterruptionWindow>,
    zone_budgets: HashMap<String, InterruptionWindow>,
    /// Sequence counter used when synthesising `AttentionBudgetWarning` events.
    warning_seq: u64,
}

impl AttentionBudgetTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a zone with a custom budget.
    ///
    /// If the zone already has a budget entry, this is a no-op.
    /// For Stack-policy zones use `DEFAULT_STACK_ZONE_BUDGET`.
    pub fn register_zone(&mut self, zone_id: &str, limit: u32) {
        self.zone_budgets
            .entry(zone_id.to_string())
            .or_insert_with(|| InterruptionWindow::new(limit));
    }

    /// Record an interruption event for `agent_namespace` / `zone_id` at `now_us`.
    ///
    /// Returns the outcome, which governs how the event is presented:
    ///
    /// - `CriticalExempt` → CRITICAL class: always deliver; budget not touched.
    /// - `SilentPassthrough` → SILENT class: always deliver; zero cost.
    /// - `Coalesce` → budget exhausted: commit mutation, coalesce visuals.
    /// - `Warning` → at 80% threshold: emit warning and deliver normally.
    /// - `Ok` → under budget; deliver normally.
    ///
    /// When the result is `Warning` or first `Coalesce`, the caller should
    /// synthesise and emit an `AttentionBudgetWarning` payload to agents
    /// subscribed to `ATTENTION_EVENTS`.
    ///
    /// Spec: scene-events/spec.md lines 137-153.
    pub fn record(
        &mut self,
        agent_namespace: &str,
        zone_id: &str,
        class: InterruptionClass,
        now_us: u64,
    ) -> AttentionBudgetOutcome {
        // CRITICAL: always exempt.
        if class == InterruptionClass::Critical {
            return AttentionBudgetOutcome::CriticalExempt;
        }

        // SILENT: zero cost.
        if class == InterruptionClass::Silent {
            return AttentionBudgetOutcome::SilentPassthrough;
        }

        // Record agent budget.
        let agent_window = self
            .agent_budgets
            .entry(agent_namespace.to_string())
            .or_insert_with(|| InterruptionWindow::new(DEFAULT_AGENT_BUDGET));
        agent_window.expire(now_us);
        let _agent_count = agent_window.record(now_us);

        // Record zone budget (auto-register with default limit if unknown).
        let zone_window = self
            .zone_budgets
            .entry(zone_id.to_string())
            .or_insert_with(|| InterruptionWindow::new(DEFAULT_ZONE_BUDGET));
        zone_window.expire(now_us);
        zone_window.record(now_us);

        // Determine outcome based on agent budget (agent budget takes precedence).
        // Zone budget is tracked but agent budget drives the coalescing decision here.
        if agent_window.is_exhausted() {
            AttentionBudgetOutcome::Coalesce
        } else if agent_window.is_at_warning() {
            AttentionBudgetOutcome::Warning
        } else {
            // Check zone budget exhaustion separately.
            if zone_window.is_exhausted() {
                AttentionBudgetOutcome::Coalesce
            } else if zone_window.is_at_warning() {
                AttentionBudgetOutcome::Warning
            } else {
                AttentionBudgetOutcome::Ok
            }
        }
    }

    /// Current rolling interruption count for `agent_namespace`.
    ///
    /// Does NOT expire old entries. For accurate counts, use `record` which
    /// expires first.
    pub fn agent_count(&self, agent_namespace: &str) -> u32 {
        self.agent_budgets
            .get(agent_namespace)
            .map(|w| w.count())
            .unwrap_or(0)
    }

    /// Current rolling interruption count for `zone_id`.
    pub fn zone_count(&self, zone_id: &str) -> u32 {
        self.zone_budgets
            .get(zone_id)
            .map(|w| w.count())
            .unwrap_or(0)
    }

    /// Whether the agent budget is exhausted.
    pub fn is_agent_exhausted(&self, agent_namespace: &str) -> bool {
        self.agent_budgets
            .get(agent_namespace)
            .map(|w| w.is_exhausted())
            .unwrap_or(false)
    }

    /// Whether the zone budget is exhausted.
    pub fn is_zone_exhausted(&self, zone_id: &str) -> bool {
        self.zone_budgets
            .get(zone_id)
            .map(|w| w.is_exhausted())
            .unwrap_or(false)
    }

    /// Synthesise an `AttentionBudgetWarning` scene event.
    ///
    /// Callers should emit this to agents subscribed to `ATTENTION_EVENTS`.
    ///
    /// Spec: scene-events/spec.md line 144:
    /// > WHEN an agent's rolling interruption count reaches 80% of limit
    /// > THEN the runtime MUST emit AttentionBudgetWarningEvent.
    pub fn make_warning_event(
        &mut self,
        agent_namespace: &str,
        now_us: u64,
    ) -> SceneEvent {
        let used = self.agent_count(agent_namespace);
        self.warning_seq += 1;
        SceneEventBuilder::new(
            "system.attention_budget_warning",
            InterruptionClass::Silent, // warning is informational, not interruptive
            EventPayload::AttentionBudgetWarning {
                agent_namespace: agent_namespace.to_string(),
                used,
                limit: DEFAULT_AGENT_BUDGET,
            },
        )
        .wall_us(now_us)
        .mono_us(now_us)
        .source(EventSource::system())
        .sequence(self.warning_seq)
        .build()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── CRITICAL / SILENT exemptions ──────────────────────────────────────────

    /// CRITICAL events are exempt from all budget counters.
    #[test]
    fn critical_exempt_from_budget() {
        let mut tracker = AttentionBudgetTracker::new();
        // Exhaust budget first.
        for i in 0..=DEFAULT_AGENT_BUDGET {
            tracker.record("agent_a", "zone_1", InterruptionClass::Normal, i as u64 * 1_000);
        }
        // CRITICAL still passes.
        let outcome =
            tracker.record("agent_a", "zone_1", InterruptionClass::Critical, 100_000);
        assert_eq!(outcome, AttentionBudgetOutcome::CriticalExempt);
    }

    /// SILENT events carry zero interruption cost.
    #[test]
    fn silent_has_zero_cost() {
        let mut tracker = AttentionBudgetTracker::new();
        // Emit many SILENT events.
        for i in 0..100 {
            let outcome =
                tracker.record("agent_a", "zone_1", InterruptionClass::Silent, i * 1_000);
            assert_eq!(outcome, AttentionBudgetOutcome::SilentPassthrough);
        }
        // Agent budget count must remain 0.
        assert_eq!(tracker.agent_count("agent_a"), 0);
    }

    // ── Warning at 80% ────────────────────────────────────────────────────────

    /// WHEN an agent's count reaches 80% (16 of 20) THEN outcome is Warning.
    ///
    /// Spec: scene-events/spec.md line 144.
    ///
    /// Uses a high-budget zone to isolate the agent-level warning check from
    /// per-zone budget interference.
    #[test]
    fn warning_at_80_percent() {
        let mut tracker = AttentionBudgetTracker::new();
        // 80% of 20 = 16
        let warn_at = (DEFAULT_AGENT_BUDGET as f64 * WARNING_FRACTION).floor() as u32;
        assert_eq!(warn_at, 16);

        // Register zone with the same limit as the agent budget so it doesn't
        // fire a warning before the agent threshold (default zone budget is 10).
        tracker.register_zone("zone_high", DEFAULT_AGENT_BUDGET);

        // Record warn_at-1 events — should all be Ok.
        for i in 0..(warn_at - 1) {
            let outcome =
                tracker.record("agent_a", "zone_high", InterruptionClass::Normal, i as u64 * 1_000);
            assert_eq!(outcome, AttentionBudgetOutcome::Ok, "event {} should be Ok", i + 1);
        }
        // 16th event triggers agent-level warning.
        let outcome = tracker.record(
            "agent_a",
            "zone_high",
            InterruptionClass::Normal,
            (warn_at - 1) as u64 * 1_000,
        );
        assert_eq!(outcome, AttentionBudgetOutcome::Warning);
        assert_eq!(tracker.agent_count("agent_a"), warn_at);
    }

    // ── Budget exhaustion coalescing ──────────────────────────────────────────

    /// WHEN an agent exceeds its budget THEN outcome is Coalesce (spec line 148).
    #[test]
    fn budget_exhaustion_returns_coalesce() {
        let mut tracker = AttentionBudgetTracker::new();
        // Fill to limit.
        for i in 0..DEFAULT_AGENT_BUDGET {
            tracker.record("agent_a", "zone_1", InterruptionClass::Normal, i as u64 * 1_000);
        }
        // Next event (21st) → exhausted.
        let outcome = tracker.record(
            "agent_a",
            "zone_1",
            InterruptionClass::Normal,
            DEFAULT_AGENT_BUDGET as u64 * 1_000,
        );
        assert_eq!(outcome, AttentionBudgetOutcome::Coalesce);
        assert!(tracker.is_agent_exhausted("agent_a"));
    }

    /// WHEN a CRITICAL event is generated while budget is exhausted THEN it is
    /// exempt (CriticalExempt, not Coalesce) (spec line 152).
    #[test]
    fn critical_exempt_when_budget_exhausted() {
        let mut tracker = AttentionBudgetTracker::new();
        for i in 0..=DEFAULT_AGENT_BUDGET {
            tracker.record("agent_a", "zone_1", InterruptionClass::Normal, i as u64 * 1_000);
        }
        assert!(tracker.is_agent_exhausted("agent_a"));

        let outcome =
            tracker.record("agent_a", "zone_1", InterruptionClass::Critical, 999_999);
        assert_eq!(outcome, AttentionBudgetOutcome::CriticalExempt);
    }

    // ── Rolling window expiry ─────────────────────────────────────────────────

    /// After the rolling window expires, count resets and budget is no longer exhausted.
    ///
    /// Uses a high-budget zone to isolate expiry semantics to the agent budget.
    /// All events are clustered at t=0, so they all expire when we jump to t=61s.
    #[test]
    fn exhausted_budget_recovers_after_window_expiry() {
        let mut tracker = AttentionBudgetTracker::new();
        // Register zone with agent-level budget so zone budget doesn't interfere.
        tracker.register_zone("zone_high", DEFAULT_AGENT_BUDGET);

        // Cluster all exhausting events at t=0 (same timestamp).
        // They will all expire when we jump to t=61s (cutoff = 1_000_000).
        for _ in 0..=DEFAULT_AGENT_BUDGET {
            tracker.record("agent_a", "zone_high", InterruptionClass::Normal, 0);
        }
        assert!(tracker.is_agent_exhausted("agent_a"));

        // At t=61s all events (at t=0) have expired (window = 60s, cutoff = 1_000_000).
        // Note: cutoff uses saturating_sub so cutoff = 61_000_000 - 60_000_000 = 1_000_000.
        // Events at t=0 < cutoff=1_000_000 → expired.
        let now_recovered = 61_000_000u64;
        let outcome =
            tracker.record("agent_a", "zone_high", InterruptionClass::Normal, now_recovered);
        // Only 1 event in window now — under threshold.
        assert_eq!(outcome, AttentionBudgetOutcome::Ok);
    }

    // ── Zone budget ───────────────────────────────────────────────────────────

    /// Per-zone default budget is 10/min.
    #[test]
    fn zone_budget_default_is_10() {
        let mut tracker = AttentionBudgetTracker::new();
        // Register with default (not Stack-policy).
        tracker.register_zone("zone_1", DEFAULT_ZONE_BUDGET);

        // Fill zone budget (10 events).
        for i in 0..DEFAULT_ZONE_BUDGET {
            tracker.record("agent_a", "zone_1", InterruptionClass::Normal, i as u64 * 1_000);
        }
        assert!(tracker.is_zone_exhausted("zone_1"));
    }

    /// Stack-policy zones have budget of 30/min.
    #[test]
    fn stack_zone_budget_is_30() {
        let mut tracker = AttentionBudgetTracker::new();
        tracker.register_zone("stack_zone", DEFAULT_STACK_ZONE_BUDGET);

        // 30 events — should exhaust exactly at limit.
        for i in 0..DEFAULT_STACK_ZONE_BUDGET {
            tracker.record("agent_a", "stack_zone", InterruptionClass::Normal, i as u64 * 1_000);
        }
        assert!(tracker.is_zone_exhausted("stack_zone"));
    }

    // ── make_warning_event ────────────────────────────────────────────────────

    /// Warning event has correct event_type prefix for ATTENTION_EVENTS routing.
    #[test]
    fn warning_event_routes_to_attention_events() {
        let mut tracker = AttentionBudgetTracker::new();
        // Record 16 interruptions to put agent at warning level.
        for i in 0..16 {
            tracker.record("agent_a", "zone_1", InterruptionClass::Normal, i as u64 * 1_000);
        }
        let evt = tracker.make_warning_event("agent_a", 16_000);
        assert!(
            evt.event_type.starts_with("system.attention_"),
            "warning event must route to ATTENTION_EVENTS: {}",
            evt.event_type
        );
        assert!(evt.source.is_system());
        // Check payload.
        match &evt.payload {
            tze_hud_scene::events::EventPayload::AttentionBudgetWarning {
                agent_namespace,
                used,
                limit,
            } => {
                assert_eq!(agent_namespace, "agent_a");
                assert_eq!(*used, 16);
                assert_eq!(*limit, DEFAULT_AGENT_BUDGET);
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    /// Warning event sequence numbers are monotonically increasing.
    #[test]
    fn warning_event_sequence_monotonically_increasing() {
        let mut tracker = AttentionBudgetTracker::new();
        let e1 = tracker.make_warning_event("a", 0);
        let e2 = tracker.make_warning_event("a", 1_000);
        assert!(e2.sequence > e1.sequence);
    }
}
