//! Structured timing errors returned by timestamp validation and scheduling.
//!
//! # Spec alignment
//!
//! Error codes from `timing-model/spec.md §Requirement: Timestamp Validation`
//! (lines 107-122), `§Requirement: Presentation Deadline` (lines 236-251),
//! `§Requirement: Clock Drift Enforcement` (lines 223-234), and
//! `§Requirement: Message Class Typed Enum` (lines 369-380).

use thiserror::Error;

/// Structured timing error codes.
///
/// All variants map to the spec-defined rejection codes.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TimingError {
    /// `present_at_wall_us` is more than 60 seconds in the past.
    ///
    /// Spec: lines 112-114.
    #[error("TIMESTAMP_TOO_OLD: present_at_wall_us is more than 60s in the past")]
    TimestampTooOld,

    /// `present_at_wall_us` is more than `max_future_schedule_us` in the
    /// future.
    ///
    /// Spec: lines 116-118.
    #[error("TIMESTAMP_TOO_FUTURE: present_at_wall_us exceeds max_future_schedule_us")]
    TimestampTooFuture,

    /// `expires_at_wall_us <= present_at_wall_us` when both are non-zero.
    ///
    /// Spec: lines 120-122.
    #[error("TIMESTAMP_EXPIRY_BEFORE_PRESENT: expires_at_wall_us must be after present_at_wall_us")]
    TimestampExpiryBeforePresent,

    /// `delivery_policy = DROP_IF_LATE` used with a non-`EphemeralRealtime`
    /// message class.
    ///
    /// Spec: lines 378-380.
    #[error("INVALID_DELIVERY_POLICY: DROP_IF_LATE requires MessageClass::EphemeralRealtime")]
    InvalidDeliveryPolicy,

    /// Clock skew exceeds 1 second — mutation rejected.
    ///
    /// Spec: lines 232-234.
    #[error("CLOCK_SKEW_EXCESSIVE: agent clock drift exceeds 1 second")]
    ClockSkewExcessive,

    /// The per-agent pending queue is at its maximum depth (256 by default).
    ///
    /// Spec: lines 245-247.
    #[error("PENDING_QUEUE_FULL: per-agent pending queue is at maximum depth")]
    PendingQueueFull,

    /// Two or more mutually exclusive schedule fields were set simultaneously.
    ///
    /// Spec: lines 81-83.
    #[error("RELATIVE_SCHEDULE_CONFLICT: only one schedule variant may be set")]
    RelativeScheduleConflict,
}

/// Non-fatal timing warning emitted into the session event stream / telemetry.
///
/// Unlike [`TimingError`], warnings do not reject the mutation — processing
/// continues with correction applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimingWarning {
    /// Agent clock drift is > `max_agent_clock_drift_ms` (default 100ms) but
    /// <= 1 second.
    ///
    /// Spec: lines 228-230.
    ClockSkewHigh {
        /// Estimated signed skew in microseconds (positive = agent is ahead).
        estimated_skew_us: i64,
    },
}

impl std::fmt::Display for TimingWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimingWarning::ClockSkewHigh { estimated_skew_us } => write!(
                f,
                "CLOCK_SKEW_HIGH: estimated skew {}µs",
                estimated_skew_us
            ),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_error_display_too_old() {
        let e = TimingError::TimestampTooOld;
        assert!(e.to_string().contains("TIMESTAMP_TOO_OLD"));
    }

    #[test]
    fn timing_error_display_too_future() {
        let e = TimingError::TimestampTooFuture;
        assert!(e.to_string().contains("TIMESTAMP_TOO_FUTURE"));
    }

    #[test]
    fn timing_error_display_expiry_before_present() {
        let e = TimingError::TimestampExpiryBeforePresent;
        assert!(e.to_string().contains("TIMESTAMP_EXPIRY_BEFORE_PRESENT"));
    }

    #[test]
    fn timing_error_display_invalid_delivery() {
        let e = TimingError::InvalidDeliveryPolicy;
        assert!(e.to_string().contains("INVALID_DELIVERY_POLICY"));
    }

    #[test]
    fn timing_error_display_clock_skew_excessive() {
        let e = TimingError::ClockSkewExcessive;
        assert!(e.to_string().contains("CLOCK_SKEW_EXCESSIVE"));
    }

    #[test]
    fn timing_error_display_pending_queue_full() {
        let e = TimingError::PendingQueueFull;
        assert!(e.to_string().contains("PENDING_QUEUE_FULL"));
    }

    #[test]
    fn timing_error_display_relative_schedule_conflict() {
        let e = TimingError::RelativeScheduleConflict;
        assert!(e.to_string().contains("RELATIVE_SCHEDULE_CONFLICT"));
    }

    #[test]
    fn timing_warning_display_clock_skew_high() {
        let w = TimingWarning::ClockSkewHigh {
            estimated_skew_us: 150_000,
        };
        assert!(w.to_string().contains("CLOCK_SKEW_HIGH"));
        assert!(w.to_string().contains("150000"));
    }
}
