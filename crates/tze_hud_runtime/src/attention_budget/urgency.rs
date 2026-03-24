//! # Earned Urgency Tracker
//!
//! Tracks per-agent HIGH-class event rates and logs a warning when an agent
//! escalates too frequently, implementing the "earned urgency" principle.
//!
//! Spec: scene-events/spec.md §Requirement: Earned Urgency Tracking, lines 317-324.
//!
//! ## Principle
//!
//! > "An agent that escalates everything is an agent that means nothing."
//!
//! Per-HIGH-class budget enforcement (hard limiting) is deferred to post-v1.
//! This module only **tracks and warns** — it does not reject or coalesce events.
//!
//! ## Configuration
//!
//! | Parameter              | Default |
//! |------------------------|---------|
//! | `high_rate_threshold`  | 4 HIGH events per agent per minute |
//! | Window                 | 60 seconds (rolling) |
//!
//! ## Spec scenario (line 324)
//!
//! > WHEN an agent emits 5 HIGH-class events within one minute (exceeding the
//! > default threshold of 4) THEN the runtime MUST log a warning about the
//! > agent's disproportionate HIGH-class usage.

use std::collections::{HashMap, VecDeque};

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for the earned-urgency tracker.
#[derive(Clone, Debug)]
pub struct EarnedUrgencyConfig {
    /// Number of HIGH-class events per agent per minute that triggers a warning.
    /// Default: 4.
    pub high_rate_threshold: u32,
    /// Rolling window size in microseconds (default: 60 seconds = 60_000_000 µs).
    pub window_us: u64,
}

impl Default for EarnedUrgencyConfig {
    fn default() -> Self {
        Self {
            high_rate_threshold: 4,
            window_us: 60_000_000, // 60 seconds
        }
    }
}

// ─── Tracker ──────────────────────────────────────────────────────────────────

/// Tracks per-agent HIGH-class event rates.
///
/// Warnings are surfaced via the `last_warning` field and via `log::warn!`.
/// Callers should check [`EarnedUrgencyTracker::record_high_event`] return value
/// to know when a warning was triggered.
#[derive(Debug, Default)]
pub struct EarnedUrgencyTracker {
    config: EarnedUrgencyConfig,
    /// Per-agent sliding window of HIGH-event timestamps (microseconds).
    high_event_times: HashMap<String, VecDeque<u64>>,
}

/// Outcome of recording a HIGH event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrgencyRecord {
    /// Under threshold — no warning.
    Ok,
    /// Threshold exceeded — a warning was logged.
    ///
    /// Contains the current HIGH event count within the window.
    ThresholdExceeded { count: u32 },
}

impl EarnedUrgencyTracker {
    /// Create a tracker with default configuration.
    pub fn new() -> Self {
        Self {
            config: EarnedUrgencyConfig::default(),
            high_event_times: HashMap::new(),
        }
    }

    /// Create a tracker with the given configuration.
    pub fn with_config(config: EarnedUrgencyConfig) -> Self {
        Self {
            config,
            high_event_times: HashMap::new(),
        }
    }

    /// Record a HIGH-class event for `agent_namespace` at `now_us`.
    ///
    /// Returns `UrgencyRecord::ThresholdExceeded` (with current count) if the
    /// agent's HIGH-event rate exceeds the configured threshold, and logs a
    /// `warn!` message.
    ///
    /// Spec: scene-events/spec.md lines 317-324.
    pub fn record_high_event(&mut self, agent_namespace: &str, now_us: u64) -> UrgencyRecord {
        let window = self
            .high_event_times
            .entry(agent_namespace.to_string())
            .or_default();

        // Expire events outside the rolling window.
        let cutoff = now_us.saturating_sub(self.config.window_us);
        while window.front().is_some_and(|&t| t < cutoff) {
            window.pop_front();
        }

        window.push_back(now_us);
        let count = window.len() as u32;

        if count > self.config.high_rate_threshold {
            log::warn!(
                "earned_urgency: agent '{}' has emitted {} HIGH-class events in the last {}s \
                 (threshold: {}). An agent that escalates everything is an agent that means \
                 nothing.",
                agent_namespace,
                count,
                self.config.window_us / 1_000_000,
                self.config.high_rate_threshold,
            );
            UrgencyRecord::ThresholdExceeded { count }
        } else {
            UrgencyRecord::Ok
        }
    }

    /// Current HIGH event count for `agent_namespace` within the rolling window.
    ///
    /// Does NOT advance or expire entries; call `record_high_event` to update.
    pub fn high_count(&self, agent_namespace: &str) -> u32 {
        self.high_event_times
            .get(agent_namespace)
            .map(|w| w.len() as u32)
            .unwrap_or(0)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_MINUTE_US: u64 = 60_000_000;

    // ── Basic tracking ────────────────────────────────────────────────────────

    /// Under threshold — no warning.
    #[test]
    fn under_threshold_no_warning() {
        let mut tracker = EarnedUrgencyTracker::new(); // threshold=4
        for i in 0..4 {
            let result = tracker.record_high_event("agent_a", i * 1_000_000);
            assert_eq!(result, UrgencyRecord::Ok);
        }
    }

    /// Exactly at threshold — no warning (threshold is a strict >, not >=).
    #[test]
    fn at_threshold_no_warning() {
        let mut tracker = EarnedUrgencyTracker::new(); // threshold=4
        for i in 0..4 {
            let result = tracker.record_high_event("agent_a", i * 1_000_000);
            assert_eq!(result, UrgencyRecord::Ok, "event {} should be under threshold", i);
        }
        // 4 events, threshold is 4 — still Ok (needs > 4 to warn).
        assert_eq!(tracker.high_count("agent_a"), 4);
    }

    /// WHEN an agent emits 5 HIGH-class events within one minute (exceeding the
    /// default threshold of 4) THEN the runtime MUST log a warning (spec line 324).
    #[test]
    fn fifth_event_exceeds_threshold() {
        let mut tracker = EarnedUrgencyTracker::new(); // threshold=4
        for i in 0..4 {
            tracker.record_high_event("agent_a", i * 1_000_000);
        }
        // 5th event crosses threshold.
        let result = tracker.record_high_event("agent_a", 4_000_000);
        assert_eq!(result, UrgencyRecord::ThresholdExceeded { count: 5 });
    }

    // ── Rolling window expiry ─────────────────────────────────────────────────

    /// Events outside the rolling window are expired before counting.
    ///
    /// All events are clustered at t=0 so they all expire when we jump to t=61s.
    /// The 60s window cutoff at t=61s is 1_000_000µs; events at t=0 < cutoff.
    #[test]
    fn old_events_expire_from_window() {
        let mut tracker = EarnedUrgencyTracker::new(); // window=60s, threshold=4

        // Emit 4 events at t=0 (same timestamp).
        // At t=61s their cutoff will be 61_000_000 - 60_000_000 = 1_000_000µs,
        // and 0 < 1_000_000 so all four will be expired.
        for _ in 0..4 {
            tracker.record_high_event("agent_a", 0);
        }
        assert_eq!(tracker.high_count("agent_a"), 4);

        // Jump to t=61s — all events at t=0 have expired.
        let now = 61_000_000;
        let result = tracker.record_high_event("agent_a", now);
        // Only 1 event in window now — under threshold.
        assert_eq!(result, UrgencyRecord::Ok, "after expiry, single new event should be Ok");
        // After the fresh record, the window contains only the one fresh event.
        assert_eq!(tracker.high_count("agent_a"), 1, "only the fresh event should remain");
    }

    /// Events exactly at window boundary are included.
    #[test]
    fn event_at_window_boundary_included() {
        let mut tracker = EarnedUrgencyTracker::new();
        // Emit 4 events at t=0..3.
        for i in 0..4 {
            tracker.record_high_event("agent_a", i * 1_000_000);
        }
        // Advance by exactly 60s — window is [0, 60s). Event at t=0 is just at
        // the cutoff (cutoff = now - 60s = 60s - 60s = 0). Since cutoff is `< cutoff`
        // (strict), the event at t=0 is still included.
        let now = ONE_MINUTE_US; // 60s
        let result = tracker.record_high_event("agent_a", now);
        // 5 events (0,1,2,3,60) in window — threshold exceeded.
        assert_eq!(result, UrgencyRecord::ThresholdExceeded { count: 5 });
    }

    // ── Per-agent isolation ───────────────────────────────────────────────────

    /// Different agents are tracked independently.
    #[test]
    fn separate_agents_tracked_independently() {
        let mut tracker = EarnedUrgencyTracker::new();

        // agent_a hits threshold.
        for i in 0..5 {
            tracker.record_high_event("agent_a", i * 1_000_000);
        }

        // agent_b is clean.
        assert_eq!(tracker.high_count("agent_b"), 0);

        let result = tracker.record_high_event("agent_b", 0);
        assert_eq!(result, UrgencyRecord::Ok);
    }

    // ── Custom configuration ──────────────────────────────────────────────────

    /// Custom threshold of 2: warning fires on 3rd event.
    #[test]
    fn custom_threshold_fires_correctly() {
        let config = EarnedUrgencyConfig {
            high_rate_threshold: 2,
            window_us: ONE_MINUTE_US,
        };
        let mut tracker = EarnedUrgencyTracker::with_config(config);

        tracker.record_high_event("agent_x", 0);
        tracker.record_high_event("agent_x", 1_000);
        let result = tracker.record_high_event("agent_x", 2_000);
        assert_eq!(result, UrgencyRecord::ThresholdExceeded { count: 3 });
    }
}
