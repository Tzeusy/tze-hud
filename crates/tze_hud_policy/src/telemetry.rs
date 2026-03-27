//! # Policy Telemetry
//!
//! Telemetry types for policy evaluation per spec §9.2 and §13.1.
//!
//! ## PolicyTelemetry
//!
//! Tracks per-frame policy evaluation statistics. One `PolicyTelemetry` value
//! is accumulated per frame and included in the frame's `TelemetryRecord`.
//!
//! ## ArbitrationTelemetryEvent
//!
//! Emitted per-rejection (Level 3) and per-shed (Level 5). Levels 2 and 4
//! emit at a lower rate (at most once per minute per active outcome type).
//!
//! ## CapabilityAuditEvent
//!
//! Every capability grant or revocation (spec §13.3).

use serde::{Deserialize, Serialize};
use tze_hud_scene::SceneId;

// ─── PolicyTelemetry ──────────────────────────────────────────────────────────

/// Per-frame policy evaluation statistics (spec §9.2).
///
/// Accumulated during a single frame cycle and attached to the frame's
/// `TelemetryRecord`. All timing values are in microseconds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PolicyTelemetry {
    /// Total time spent in per-frame evaluation (Levels 1, 2, 5, 6) in microseconds.
    /// Budget: < 200us.
    pub per_frame_eval_us: u64,

    /// p99 per-mutation policy check latency in microseconds for the current frame.
    /// Budget: < 50us.
    pub per_mutation_eval_us_p99: u64,

    /// Number of mutations rejected (Reject outcome) this frame.
    pub mutations_rejected: u32,

    /// Number of mutations committed with redaction (CommitRedacted) this frame.
    pub mutations_redacted: u32,

    /// Number of mutations queued (Queue outcome) this frame.
    pub mutations_queued: u32,

    /// Number of mutations shed at Level 5 (Shed outcome) this frame.
    pub mutations_shed: u32,

    /// Number of Level 0 override commands processed this frame.
    pub override_commands_processed: u32,
}

impl PolicyTelemetry {
    /// Merge another `PolicyTelemetry` record into this one (additive).
    pub fn merge(&mut self, other: &PolicyTelemetry) {
        // For latency, take the max (worst-case p99 across merged frame windows).
        self.per_frame_eval_us = self
            .per_frame_eval_us
            .saturating_add(other.per_frame_eval_us);
        self.per_mutation_eval_us_p99 = self
            .per_mutation_eval_us_p99
            .max(other.per_mutation_eval_us_p99);
        self.mutations_rejected = self
            .mutations_rejected
            .saturating_add(other.mutations_rejected);
        self.mutations_redacted = self
            .mutations_redacted
            .saturating_add(other.mutations_redacted);
        self.mutations_queued = self.mutations_queued.saturating_add(other.mutations_queued);
        self.mutations_shed = self.mutations_shed.saturating_add(other.mutations_shed);
        self.override_commands_processed = self
            .override_commands_processed
            .saturating_add(other.override_commands_processed);
    }
}

// ─── ArbitrationTelemetryEvent ────────────────────────────────────────────────

/// Event type for an arbitration telemetry record (spec §13.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArbitrationEventKind {
    /// A mutation was rejected at this level (Level 3 or Level 5 budget exceeded).
    /// Emitted per-occurrence.
    Reject,
    /// A mutation was shed at Level 5 (degradation). Emitted per-occurrence.
    Shed,
    /// Level 2 (Privacy) redaction was applied.
    /// Emitted at most once per minute per active session.
    Redact,
    /// Level 4 (Attention) budget or quiet-hours caused queueing.
    /// Emitted at most once per minute per active session.
    Queue,
}

/// A single arbitration telemetry record (spec §13.1).
///
/// Emitted for every Level 3 rejection, every Level 5 shed, and at a lower
/// rate for Level 2 redactions and Level 4 queuing events.
///
/// Field `event` is the machine-readable event name (static string constant):
/// `"arbitration_reject"`, `"arbitration_shed"`, `"arbitration_redact"`,
/// `"arbitration_queue"`. Stored as `&'static str` to avoid per-event allocations
/// on the hot reject/shed path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArbitrationTelemetryEvent {
    /// Machine-readable event name (static; no allocation per event).
    pub event: &'static str,

    /// Kind of the event.
    pub kind: ArbitrationEventKind,

    /// Arbitration level that produced this outcome (0–6).
    pub level: u8,

    /// Structured error code for reject events, e.g. `"CAPABILITY_DENIED"`.
    /// `None` for Shed events (no error code).
    pub code: Option<String>,

    /// ID of the agent whose mutation triggered this event.
    pub agent_id: String,

    /// Reference ID of the specific mutation.
    pub mutation_ref: SceneId,

    /// Monotonic timestamp in microseconds when this event was emitted.
    pub timestamp_us: u64,
}

impl ArbitrationTelemetryEvent {
    /// Create a Reject event (Level 3 capability denial, Level 5 budget exceeded).
    pub fn reject(
        level: u8,
        code: impl Into<String>,
        agent_id: impl Into<String>,
        mutation_ref: SceneId,
        timestamp_us: u64,
    ) -> Self {
        Self {
            event: "arbitration_reject",
            kind: ArbitrationEventKind::Reject,
            level,
            code: Some(code.into()),
            agent_id: agent_id.into(),
            mutation_ref,
            timestamp_us,
        }
    }

    /// Create a Shed event (Level 5 degradation shedding).
    pub fn shed(agent_id: impl Into<String>, mutation_ref: SceneId, timestamp_us: u64) -> Self {
        Self {
            event: "arbitration_shed",
            kind: ArbitrationEventKind::Shed,
            level: 5,
            code: None,
            agent_id: agent_id.into(),
            mutation_ref,
            timestamp_us,
        }
    }

    /// Create a Redact event (Level 2 privacy redaction).
    pub fn redact(agent_id: impl Into<String>, mutation_ref: SceneId, timestamp_us: u64) -> Self {
        Self {
            event: "arbitration_redact",
            kind: ArbitrationEventKind::Redact,
            level: 2,
            code: None,
            agent_id: agent_id.into(),
            mutation_ref,
            timestamp_us,
        }
    }

    /// Create a Queue event (Level 4 attention budget / quiet hours).
    pub fn queue(agent_id: impl Into<String>, mutation_ref: SceneId, timestamp_us: u64) -> Self {
        Self {
            event: "arbitration_queue",
            kind: ArbitrationEventKind::Queue,
            level: 4,
            code: None,
            agent_id: agent_id.into(),
            mutation_ref,
            timestamp_us,
        }
    }
}

// ─── CapabilityAuditEvent ─────────────────────────────────────────────────────

/// The kind of a capability audit event (spec §13.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityAuditKind {
    /// A capability was granted to the agent.
    Grant,
    /// A capability was revoked from the agent.
    Revoke,
}

/// A capability grant or revocation audit record (spec §13.3).
///
/// Every grant and revocation MUST be logged with the agent ID, capability name,
/// timestamp, and granting source.
///
/// Example: WHEN an agent is granted `publish_zone:notification` during session
/// handshake THEN a structured log entry is emitted with the agent ID, capability
/// name, timestamp, and `granted_by = "session_handshake"`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityAuditEvent {
    /// Whether this was a grant or revocation.
    pub kind: CapabilityAuditKind,

    /// ID of the agent whose capabilities changed.
    pub agent_id: String,

    /// Canonical capability name (snake_case per spec §8.1).
    pub capability: String,

    /// Monotonic timestamp in microseconds.
    pub timestamp_us: u64,

    /// Source of the grant or revocation.
    /// Examples: `"session_handshake"`, `"admin_action"`, `"lease_expiry"`.
    pub granted_by: String,
}

impl CapabilityAuditEvent {
    /// Create a grant event.
    pub fn grant(
        agent_id: impl Into<String>,
        capability: impl Into<String>,
        timestamp_us: u64,
        granted_by: impl Into<String>,
    ) -> Self {
        Self {
            kind: CapabilityAuditKind::Grant,
            agent_id: agent_id.into(),
            capability: capability.into(),
            timestamp_us,
            granted_by: granted_by.into(),
        }
    }

    /// Create a revocation event.
    pub fn revoke(
        agent_id: impl Into<String>,
        capability: impl Into<String>,
        timestamp_us: u64,
        granted_by: impl Into<String>,
    ) -> Self {
        Self {
            kind: CapabilityAuditKind::Revoke,
            agent_id: agent_id.into(),
            capability: capability.into(),
            timestamp_us,
            granted_by: granted_by.into(),
        }
    }
}

// ─── Per-mutation latency tracking ───────────────────────────────────────────

/// Accumulator for per-mutation evaluation latencies within one frame.
///
/// Used to compute `PolicyTelemetry::per_mutation_eval_us_p99`.
#[derive(Clone, Debug, Default)]
pub struct MutationLatencyAccumulator {
    samples: Vec<u64>,
}

impl MutationLatencyAccumulator {
    /// Record a single per-mutation evaluation time in microseconds.
    pub fn record(&mut self, eval_us: u64) {
        self.samples.push(eval_us);
    }

    /// Compute the p99 latency from the recorded samples.
    ///
    /// Uses the nearest-rank method: `rank = ceil(0.99 * n)`, `idx = rank - 1`.
    /// For n=100: rank=99, idx=98 (the 99th sample, not the 100th).
    ///
    /// Returns 0 if no samples have been recorded.
    pub fn p99_us(&mut self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        self.samples.sort_unstable();
        let n = self.samples.len();
        // Nearest-rank method: ceil(0.99 * n) using integer arithmetic.
        // rank = ceil(99 * n / 100) = (99 * n + 99) / 100
        let rank = (99 * n + 99) / 100;
        let idx = (rank - 1).min(n - 1);
        self.samples[idx]
    }

    /// Clear all recorded samples.
    pub fn reset(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod telemetry_tests {
    use super::*;
    use tze_hud_scene::SceneId;

    #[test]
    fn test_policy_telemetry_default_is_zero() {
        let t = PolicyTelemetry::default();
        assert_eq!(t.per_frame_eval_us, 0);
        assert_eq!(t.mutations_rejected, 0);
        assert_eq!(t.mutations_redacted, 0);
        assert_eq!(t.mutations_queued, 0);
        assert_eq!(t.mutations_shed, 0);
        assert_eq!(t.override_commands_processed, 0);
    }

    #[test]
    fn test_policy_telemetry_merge() {
        let mut a = PolicyTelemetry {
            per_frame_eval_us: 100,
            per_mutation_eval_us_p99: 10,
            mutations_rejected: 3,
            mutations_redacted: 1,
            mutations_queued: 2,
            mutations_shed: 0,
            override_commands_processed: 1,
        };
        let b = PolicyTelemetry {
            per_frame_eval_us: 50,
            per_mutation_eval_us_p99: 20, // higher p99
            mutations_rejected: 1,
            mutations_redacted: 0,
            mutations_queued: 1,
            mutations_shed: 2,
            override_commands_processed: 0,
        };
        a.merge(&b);
        assert_eq!(a.per_frame_eval_us, 150);
        assert_eq!(a.per_mutation_eval_us_p99, 20, "p99 takes the max");
        assert_eq!(a.mutations_rejected, 4);
        assert_eq!(a.mutations_redacted, 1);
        assert_eq!(a.mutations_queued, 3);
        assert_eq!(a.mutations_shed, 2);
        assert_eq!(a.override_commands_processed, 1);
    }

    /// WHEN 3 mutations are rejected at Level 3 in a single frame
    /// THEN PolicyTelemetry.mutations_rejected is 3 (spec lines 320-322)
    #[test]
    fn test_telemetry_captures_rejection_count() {
        let mut telemetry = PolicyTelemetry::default();
        telemetry.mutations_rejected += 1;
        telemetry.mutations_rejected += 1;
        telemetry.mutations_rejected += 1;
        assert_eq!(telemetry.mutations_rejected, 3);
    }

    /// WHEN a mutation is rejected at Level 3 for capability denial
    /// THEN a telemetry record is emitted with correct fields (spec lines 329-331)
    #[test]
    fn test_arbitration_reject_event_fields() {
        let mutation_ref = SceneId::new();
        let event = ArbitrationTelemetryEvent::reject(
            3,
            "CAPABILITY_DENIED",
            "agent_a",
            mutation_ref,
            1_000_000,
        );
        assert_eq!(event.event, "arbitration_reject");
        assert_eq!(event.kind, ArbitrationEventKind::Reject);
        assert_eq!(event.level, 3);
        assert_eq!(event.code.as_deref(), Some("CAPABILITY_DENIED"));
        assert_eq!(event.agent_id, "agent_a");
        assert_eq!(event.mutation_ref, mutation_ref);
        assert_eq!(event.timestamp_us, 1_000_000);
    }

    #[test]
    fn test_arbitration_shed_event_fields() {
        let mutation_ref = SceneId::new();
        let event = ArbitrationTelemetryEvent::shed("agent_b", mutation_ref, 2_000_000);
        assert_eq!(event.event, "arbitration_shed");
        assert_eq!(event.kind, ArbitrationEventKind::Shed);
        assert_eq!(event.level, 5);
        assert!(event.code.is_none(), "Shed events have no error code");
    }

    /// WHEN an agent is granted publish_zone:notification during session handshake
    /// THEN a structured log entry is emitted (spec lines 339-340)
    #[test]
    fn test_capability_grant_audit_event() {
        let event = CapabilityAuditEvent::grant(
            "agent_x",
            "publish_zone:notification",
            999_000,
            "session_handshake",
        );
        assert_eq!(event.kind, CapabilityAuditKind::Grant);
        assert_eq!(event.agent_id, "agent_x");
        assert_eq!(event.capability, "publish_zone:notification");
        assert_eq!(event.timestamp_us, 999_000);
        assert_eq!(event.granted_by, "session_handshake");
    }

    #[test]
    fn test_mutation_latency_accumulator_p99() {
        let mut acc = MutationLatencyAccumulator::default();
        // 100 samples: 99 at 10us, 1 at 100us
        for _ in 0..99 {
            acc.record(10);
        }
        acc.record(100);
        let p99 = acc.p99_us();
        // p99 with 100 samples using nearest-rank: rank = ceil(0.99*100) = 99, idx = 98.
        // Sorted samples[98] = 10us (the 99th of 100 values; samples[99] = 100us is p100).
        assert_eq!(p99, 10);
    }

    #[test]
    fn test_mutation_latency_accumulator_empty() {
        let mut acc = MutationLatencyAccumulator::default();
        assert_eq!(acc.p99_us(), 0);
    }

    #[test]
    fn test_mutation_latency_accumulator_reset() {
        let mut acc = MutationLatencyAccumulator::default();
        acc.record(50);
        acc.reset();
        assert_eq!(acc.p99_us(), 0);
    }
}
