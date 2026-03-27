//! Timestamp validation and frame-quantization logic.
//!
//! # Spec alignment
//!
//! - `§Requirement: Timestamp Validation` (lines 107-122)
//! - `§Requirement: Frame Quantization` (lines 94-105)
//! - `§Requirement: Arrival Time Is Not Presentation Time` (lines 50-61)
//! - `§Requirement: Message Class Typed Enum` (lines 369-380)
//!
//! ## Frame quantization rule
//!
//! A mutation with `present_at_wall_us = T` is "in scope" for frame F iff
//! `T <= frame_F_vsync_wall_us`. Strict no-earlier-than guarantee.
//!
//! ## Session baseline
//!
//! `session_open_wall_us` is recorded when the agent's gRPC session is
//! established. It is used as the reference for the TIMESTAMP_TOO_OLD check:
//! mutations with `present_at_wall_us` more than 60s before
//! `session_open_wall_us` are rejected.

use crate::timing::{
    WallUs,
    errors::{TimingError, TimingWarning},
    hints::{DeliveryPolicy, MessageClass, TimingHints},
};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Mutations with `present_at_wall_us` more than this many microseconds in the
/// past (relative to `session_open_wall_us`) are rejected with
/// `TIMESTAMP_TOO_OLD`.
///
/// Spec: 60 seconds = 60_000_000 µs.
pub const TIMESTAMP_TOO_OLD_THRESHOLD_US: u64 = 60_000_000;

/// Default upper bound for future scheduling (5 minutes in µs).
///
/// This is the default `max_future_schedule_us` from `TimingConfig`.
pub const DEFAULT_MAX_FUTURE_SCHEDULE_US: u64 = 300_000_000;

/// Clock drift threshold above which `CLOCK_SKEW_EXCESSIVE` is returned.
///
/// Spec: 1 second = 1_000_000 µs.
pub const CLOCK_SKEW_EXCESSIVE_THRESHOLD_US: u64 = 1_000_000;

/// Clock drift threshold above which `CLOCK_SKEW_HIGH` warning is emitted.
///
/// Spec: 100 ms = 100_000 µs.
pub const CLOCK_SKEW_HIGH_THRESHOLD_US: u64 = 100_000;

// ─── TimestampValidationInput ────────────────────────────────────────────────

/// Context required to validate a single set of timing hints.
#[derive(Clone, Debug)]
pub struct TimestampValidationInput {
    /// UTC wall-clock time at session establishment (spec lines 46-48).
    pub session_open_wall_us: WallUs,
    /// Current compositor wall-clock time.
    pub now_wall_us: WallUs,
    /// Maximum future scheduling horizon (from `TimingConfig`).
    pub max_future_schedule_us: u64,
    /// Estimated agent-clock skew in microseconds (positive = agent ahead).
    /// Used for `CLOCK_SKEW_EXCESSIVE` / `CLOCK_SKEW_HIGH` checks.
    pub estimated_skew_us: i64,
}

// ─── validate_timing_hints ────────────────────────────────────────────────────

/// Validate `hints` against the spec's timestamp validation rules.
///
/// Returns `Ok(warnings)` on success (zero or more non-fatal warnings) or
/// `Err(TimingError)` on a fatal validation failure.
///
/// Checks performed (in order):
/// 1. `INVALID_DELIVERY_POLICY` — `DROP_IF_LATE` with non-`EphemeralRealtime`
/// 2. `CLOCK_SKEW_EXCESSIVE` — absolute skew > 1 s
/// 3. `TIMESTAMP_TOO_OLD` — `present_at_wall_us` more than 60 s in the past
/// 4. `TIMESTAMP_TOO_FUTURE` — `present_at_wall_us` beyond `max_future_schedule_us`
/// 5. `TIMESTAMP_EXPIRY_BEFORE_PRESENT` — `expires_at_wall_us <= present_at_wall_us`
///
/// The `CLOCK_SKEW_HIGH` warning is appended to the returned vec when
/// `|estimated_skew_us|` > 100 ms (but <= 1 s).
///
/// # Note on `RELATIVE_SCHEDULE_CONFLICT`
///
/// The `Schedule` field is a Rust enum — only one variant can be set at a
/// time, so mutual exclusion is enforced by the type system in this crate.
/// For the proto wire-decode path, where a non-compliant client may set
/// multiple `oneof` fields simultaneously, callers MUST check for conflict
/// during deserialization and return [`TimingError::RelativeScheduleConflict`]
/// before calling this function.
pub fn validate_timing_hints(
    hints: &TimingHints,
    ctx: &TimestampValidationInput,
) -> Result<Vec<TimingWarning>, TimingError> {
    let mut warnings = Vec::new();

    // ── 1. Delivery policy mutual exclusion ──────────────────────────────────
    if hints.delivery_policy == DeliveryPolicy::DropIfLate
        && hints.message_class != MessageClass::EphemeralRealtime
    {
        return Err(TimingError::InvalidDeliveryPolicy);
    }

    // ── 2. Clock skew checks ─────────────────────────────────────────────────
    let abs_skew = ctx.estimated_skew_us.unsigned_abs();
    if abs_skew > CLOCK_SKEW_EXCESSIVE_THRESHOLD_US {
        return Err(TimingError::ClockSkewExcessive);
    }
    if abs_skew > CLOCK_SKEW_HIGH_THRESHOLD_US {
        warnings.push(TimingWarning::ClockSkewHigh {
            estimated_skew_us: ctx.estimated_skew_us,
        });
    }

    // ── 3. present_at_wall_us validation ─────────────────────────────────────
    let present_at = hints.present_at_wall_us();
    if present_at.is_set() {
        let t = present_at.as_u64();
        let now = ctx.now_wall_us.as_u64();

        // TIMESTAMP_TOO_OLD: present_at more than 60 s before session open
        // (spec lines 112-114).  The spec says "60 seconds before
        // session_open_wall_us", which we conservatively treat as: if
        // present_at < session_open_wall_us - 60s.
        let session_open = ctx.session_open_wall_us.as_u64();
        let stale_threshold = session_open.saturating_sub(TIMESTAMP_TOO_OLD_THRESHOLD_US);
        if t < stale_threshold {
            return Err(TimingError::TimestampTooOld);
        }

        // TIMESTAMP_TOO_FUTURE: present_at more than max_future_schedule_us
        // ahead of now (spec lines 116-118).
        let far_future = now.saturating_add(ctx.max_future_schedule_us);
        if t > far_future {
            return Err(TimingError::TimestampTooFuture);
        }
    }

    // ── 4. expires_at validation ─────────────────────────────────────────────
    if hints.expires_at_wall_us.is_set() && present_at.is_set() {
        if hints.expires_at_wall_us.as_u64() <= present_at.as_u64() {
            return Err(TimingError::TimestampExpiryBeforePresent);
        }
    }
    // Edge case: expires_at set, present_at = 0 ("immediate").  The
    // compositor will apply immediately, so expires_at must be in the future.
    if hints.expires_at_wall_us.is_set() && !present_at.is_set() {
        let now = ctx.now_wall_us.as_u64();
        if hints.expires_at_wall_us.as_u64() <= now {
            return Err(TimingError::TimestampExpiryBeforePresent);
        }
    }

    Ok(warnings)
}

// ─── Frame Quantization ───────────────────────────────────────────────────────

/// Returns `true` if a mutation with `present_at_wall_us = present_at` is
/// "in scope" for frame F whose vsync wall-clock time is `vsync_wall_us`.
///
/// Strict no-earlier-than rule: `present_at <= vsync_wall_us`.
///
/// From spec §Requirement: Frame Quantization (lines 94-105):
/// > A present_at_wall_us timestamp T is "in scope" for frame F iff
/// > T <= frame_F_vsync_wall_us.
///
/// When `present_at` is `WallUs::NOT_SET` (zero), the mutation is immediate
/// and is always in scope.
pub fn is_in_scope_for_frame(present_at: WallUs, vsync_wall_us: WallUs) -> bool {
    if !present_at.is_set() {
        // Zero = "immediate" — apply at earliest available frame.
        return true;
    }
    present_at.as_u64() <= vsync_wall_us.as_u64()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::DurationUs;
    use crate::timing::hints::{DeliveryPolicy, MessageClass, Schedule, TimingHints};

    fn ctx_at(now_us: u64) -> TimestampValidationInput {
        TimestampValidationInput {
            session_open_wall_us: WallUs(now_us),
            now_wall_us: WallUs(now_us),
            max_future_schedule_us: DEFAULT_MAX_FUTURE_SCHEDULE_US,
            estimated_skew_us: 0,
        }
    }

    // ── Delivery policy ──

    /// WHEN DROP_IF_LATE + TRANSACTIONAL THEN reject with INVALID_DELIVERY_POLICY.
    #[test]
    fn drop_if_late_with_transactional_rejected() {
        let mut hints = TimingHints::new();
        hints.message_class = MessageClass::Transactional;
        hints.delivery_policy = DeliveryPolicy::DropIfLate;
        let ctx = ctx_at(1_000_000_000);
        let err = validate_timing_hints(&hints, &ctx).unwrap_err();
        assert_eq!(err, TimingError::InvalidDeliveryPolicy);
    }

    /// WHEN DROP_IF_LATE + EPHEMERAL_REALTIME THEN accept.
    #[test]
    fn drop_if_late_with_ephemeral_accepted() {
        let mut hints = TimingHints::new();
        hints.message_class = MessageClass::EphemeralRealtime;
        hints.delivery_policy = DeliveryPolicy::DropIfLate;
        let ctx = ctx_at(1_000_000_000);
        assert!(validate_timing_hints(&hints, &ctx).is_ok());
    }

    // ── Clock skew enforcement ──

    /// WHEN skew > 1s THEN reject with CLOCK_SKEW_EXCESSIVE.
    #[test]
    fn clock_skew_excessive_rejected() {
        let hints = TimingHints::new();
        let mut ctx = ctx_at(1_000_000_000);
        ctx.estimated_skew_us = 1_500_000; // 1.5 s
        let err = validate_timing_hints(&hints, &ctx).unwrap_err();
        assert_eq!(err, TimingError::ClockSkewExcessive);
    }

    /// WHEN skew is negative but > 1s THEN reject.
    #[test]
    fn clock_skew_excessive_negative_rejected() {
        let hints = TimingHints::new();
        let mut ctx = ctx_at(1_000_000_000);
        ctx.estimated_skew_us = -1_100_000;
        let err = validate_timing_hints(&hints, &ctx).unwrap_err();
        assert_eq!(err, TimingError::ClockSkewExcessive);
    }

    /// WHEN skew = 200ms THEN warn with CLOCK_SKEW_HIGH, continue.
    #[test]
    fn clock_skew_high_emits_warning() {
        let hints = TimingHints::new();
        let mut ctx = ctx_at(1_000_000_000);
        ctx.estimated_skew_us = 200_000; // 200 ms
        let warnings = validate_timing_hints(&hints, &ctx).unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| matches!(w, TimingWarning::ClockSkewHigh { .. })),
            "expected CLOCK_SKEW_HIGH warning"
        );
    }

    /// WHEN skew = 50ms THEN no warning.
    #[test]
    fn clock_skew_50ms_no_warning() {
        let hints = TimingHints::new();
        let mut ctx = ctx_at(1_000_000_000);
        ctx.estimated_skew_us = 50_000;
        let warnings = validate_timing_hints(&hints, &ctx).unwrap();
        assert!(warnings.is_empty());
    }

    // ── Timestamp too old ──

    /// WHEN present_at > 60s before session_open THEN reject with TIMESTAMP_TOO_OLD.
    #[test]
    fn timestamp_too_old_rejected() {
        let mut hints = TimingHints::new();
        let session_open = 1_000_000_000_u64; // some epoch value
        // present_at is 61 seconds before session_open
        let present_at = session_open - 61_000_000;
        hints.schedule = Some(Schedule::PresentAt(WallUs(present_at)));
        let mut ctx = ctx_at(session_open);
        ctx.session_open_wall_us = WallUs(session_open);
        let err = validate_timing_hints(&hints, &ctx).unwrap_err();
        assert_eq!(err, TimingError::TimestampTooOld);
    }

    /// WHEN present_at = 59s before session_open THEN accepted.
    #[test]
    fn timestamp_59s_old_accepted() {
        let mut hints = TimingHints::new();
        let session_open = 1_000_000_000_u64;
        let present_at = session_open - 59_000_000;
        hints.schedule = Some(Schedule::PresentAt(WallUs(present_at)));
        let ctx = ctx_at(session_open);
        assert!(validate_timing_hints(&hints, &ctx).is_ok());
    }

    // ── Timestamp too future ──

    /// WHEN present_at > 5min in future THEN reject with TIMESTAMP_TOO_FUTURE.
    #[test]
    fn timestamp_too_future_rejected() {
        let mut hints = TimingHints::new();
        let now = 1_000_000_000_u64;
        hints.schedule = Some(Schedule::PresentAt(WallUs(
            now + DEFAULT_MAX_FUTURE_SCHEDULE_US + 1,
        )));
        let ctx = ctx_at(now);
        let err = validate_timing_hints(&hints, &ctx).unwrap_err();
        assert_eq!(err, TimingError::TimestampTooFuture);
    }

    /// WHEN present_at = exactly max_future_schedule_us ahead THEN accepted.
    #[test]
    fn timestamp_at_max_future_accepted() {
        let mut hints = TimingHints::new();
        let now = 1_000_000_000_u64;
        hints.schedule = Some(Schedule::PresentAt(WallUs(
            now + DEFAULT_MAX_FUTURE_SCHEDULE_US,
        )));
        let ctx = ctx_at(now);
        assert!(validate_timing_hints(&hints, &ctx).is_ok());
    }

    // ── Expiry before present ──

    /// WHEN expires_at <= present_at THEN reject with TIMESTAMP_EXPIRY_BEFORE_PRESENT.
    #[test]
    fn expiry_before_present_rejected() {
        let mut hints = TimingHints::new();
        let t = 1_000_000_000_u64;
        hints.schedule = Some(Schedule::PresentAt(WallUs(t)));
        hints.expires_at_wall_us = WallUs(t); // equal — invalid
        let ctx = ctx_at(t);
        let err = validate_timing_hints(&hints, &ctx).unwrap_err();
        assert_eq!(err, TimingError::TimestampExpiryBeforePresent);
    }

    #[test]
    fn expiry_strictly_after_present_accepted() {
        let mut hints = TimingHints::new();
        let t = 1_000_000_000_u64;
        hints.schedule = Some(Schedule::PresentAt(WallUs(t)));
        hints.expires_at_wall_us = WallUs(t + 1_000_000); // +1s
        let ctx = ctx_at(t);
        assert!(validate_timing_hints(&hints, &ctx).is_ok());
    }

    // ── Frame quantization ──

    /// WHEN present_at = V+1ms, frame vsync = V THEN NOT in scope.
    #[test]
    fn present_at_v_plus_1ms_not_in_scope_for_frame_v() {
        let vsync = WallUs(1_000_000_000);
        let present_at = WallUs(1_000_001_000); // vsync + 1ms
        assert!(!is_in_scope_for_frame(present_at, vsync));
    }

    /// WHEN present_at = V THEN in scope for frame at V.
    #[test]
    fn present_at_exactly_vsync_in_scope() {
        let vsync = WallUs(1_000_000_000);
        let present_at = WallUs(1_000_000_000);
        assert!(is_in_scope_for_frame(present_at, vsync));
    }

    /// WHEN present_at = 0 (immediate) THEN always in scope.
    #[test]
    fn present_at_zero_always_in_scope() {
        let vsync = WallUs(1_000_000_000);
        assert!(is_in_scope_for_frame(WallUs::NOT_SET, vsync));
    }

    /// WHEN present_at < vsync THEN in scope (mutation is due).
    #[test]
    fn present_at_before_vsync_in_scope() {
        let vsync = WallUs(1_000_000_000);
        let present_at = WallUs(999_000_000);
        assert!(is_in_scope_for_frame(present_at, vsync));
    }

    // ── Relative scheduling: hints without a present_at are valid ──

    #[test]
    fn after_us_hint_passes_validation() {
        let mut hints = TimingHints::new();
        hints.schedule = Some(Schedule::AfterUs(DurationUs(500_000)));
        let ctx = ctx_at(1_000_000_000);
        assert!(validate_timing_hints(&hints, &ctx).is_ok());
    }
}
