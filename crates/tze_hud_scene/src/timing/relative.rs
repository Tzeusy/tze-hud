//! Relative scheduling primitive conversion.
//!
//! # Spec alignment
//!
//! Implements `timing-model/spec.md §Requirement: Relative Scheduling Primitives`
//! (lines 270-289).
//!
//! Three `Schedule` variants are wire-level convenience sugar over absolute
//! `present_at_wall_us`. They MUST be converted to absolute wall-clock time at
//! Stage 3 intake and MUST NOT be stored in the scene graph, telemetry, or
//! internal state.
//!
//! ## Variants and conversion rules
//!
//! | Variant | Rule |
//! |---|---|
//! | `AfterUs(d)` | `target_mono = intake_mono + d`; then `wall = target_mono + clock_offset` |
//! | `FramesFromNow(n)` | `wall = vsync_wall + n * frame_period_us` |
//! | `NextFrame` | Identical to `FramesFromNow(1)` |
//!
//! ## After conversion
//!
//! The returned `WallUs` is inserted into the pending queue as an absolute
//! `present_at_wall_us`. The original relative field MUST NOT be retained.
//!
//! ## Mutual exclusivity
//!
//! The [`Schedule`] enum enforces mutual exclusivity at the type level — only
//! one variant can be set. For the proto wire-decode path (where a non-
//! compliant client might set multiple `oneof` fields), callers must check for
//! conflict during deserialization and return
//! [`TimingError::RelativeScheduleConflict`] before calling this module.

use crate::timing::{
    hints::Schedule,
    domains::ClockOffset,
    DurationUs, MonoUs, WallUs,
};

// ─── IntakeContext ────────────────────────────────────────────────────────────

/// Runtime context available at Stage 3 intake.
///
/// All values are sampled once when a [`MutationBatch`] enters Stage 3 and
/// MUST NOT change during the conversion.
///
/// [`MutationBatch`]: crate::mutation::MutationBatch
#[derive(Clone, Copy, Debug)]
pub struct IntakeContext {
    /// Monotonic clock value at the moment the batch was received (Stage 3).
    ///
    /// Used by `AfterUs` conversion: `target_mono = intake_mono_us + after_us`.
    pub intake_mono_us: MonoUs,

    /// Compositor clock offset: `wall_us - mono_us` at the most recent
    /// calibration point.
    ///
    /// This is the compositor's own wall-vs-monotonic calibration offset, NOT
    /// an agent-clock-skew measurement. A positive value means the compositor's
    /// wall clock is ahead of its monotonic clock.
    ///
    /// Used to convert a monotonic target time to an absolute wall timestamp:
    /// `present_at_wall_us = target_mono_us + clock_offset.0`.
    ///
    /// Use the [`ClockOffset`] newtype to avoid accidentally passing an
    /// agent-side skew value (a different concept) here.
    pub clock_offset: ClockOffset,

    /// Wall-clock time of the current frame's vsync.
    ///
    /// Used as the base for `FramesFromNow` and `NextFrame` conversions.
    pub vsync_wall_us: WallUs,

    /// Frame period derived from `TimingConfig::target_fps`.
    ///
    /// For 60 fps: `DurationUs(16_666)` µs.
    pub frame_period_us: DurationUs,
}

// ─── Conversion functions ─────────────────────────────────────────────────────

/// Convert `after_us` to an absolute `WallUs` presentation timestamp.
///
/// Spec lines 275-277:
/// > The compositor MUST compute `target_mono_us = monotonic_us_at_intake +
/// > after_us`, convert to wall-clock via the session clock-skew estimate
/// > (`present_at_wall_us = target_mono_us + skew_us`), and enter the
/// > mutation into the pending queue at Stage 3 intake.
///
/// Arithmetic saturates on overflow to preserve liveness (a clamped value is
/// always preferable to a panic or wrap-around).
pub fn resolve_after_us(after_us: DurationUs, ctx: &IntakeContext) -> WallUs {
    // target_mono = intake_mono + after_us (saturating)
    let target_mono = MonoUs(ctx.intake_mono_us.0.saturating_add(after_us.as_u64()));

    // present_at_wall = target_mono + clock_offset (signed, saturating via MonoUs::to_wall)
    // MonoUs::to_wall saturates and returns WallUs; we additionally clamp to
    // [1, u64::MAX] — zero is the "not set" sentinel.
    let wall = target_mono.to_wall(ctx.clock_offset);

    // Guard: if the offset is extremely negative and pushes us to zero (NOT_SET
    // sentinel), clamp to 1 so callers always receive a real timestamp.
    if wall == WallUs::NOT_SET {
        WallUs(1)
    } else {
        wall
    }
}

/// Convert `frames_from_now = N` to an absolute `WallUs` presentation timestamp.
///
/// Spec lines 279-281:
/// > The compositor MUST convert it to `present_at_wall_us` corresponding to
/// > frame N+current vsync and apply at that frame.
///
/// `vsync_wall_us + N * frame_period_us`, saturating on overflow.
/// `N == 0` maps to the current vsync (same as an immediate absolute timestamp
/// at the current vsync time).
pub fn resolve_frames_from_now(n_frames: u32, ctx: &IntakeContext) -> WallUs {
    let offset = (n_frames as u64).saturating_mul(ctx.frame_period_us.as_u64());
    WallUs(ctx.vsync_wall_us.0.saturating_add(offset))
}

/// Convert `next_frame = true` to an absolute `WallUs` presentation timestamp.
///
/// Spec lines 283-285:
/// > It MUST be treated identically to `frames_from_now = 1`; the mutation
/// > MUST NOT be applied in the current frame.
///
/// Equivalent to `resolve_frames_from_now(1, ctx)`.
pub fn resolve_next_frame(ctx: &IntakeContext) -> WallUs {
    resolve_frames_from_now(1, ctx)
}

// ─── Primary entry point ──────────────────────────────────────────────────────

/// Resolve any [`Schedule`] variant to an absolute `present_at_wall_us`.
///
/// - `PresentAt(t)` — returned unchanged.
/// - `AfterUs(d)` — converted via [`resolve_after_us`].
/// - `FramesFromNow(n)` — converted via [`resolve_frames_from_now`].
/// - `NextFrame` — converted via [`resolve_next_frame`].
/// - `None` or `PresentAt(0)` — returns [`WallUs::NOT_SET`] (immediate).
///
/// This function is **infallible**: the [`Schedule`] enum enforces mutual
/// exclusivity at the type level, so no conflict can occur here. Wire-level
/// conflict detection (where a non-compliant proto client may set multiple
/// `oneof` fields) MUST be performed by the deserializer before constructing
/// a `Schedule` value — returning [`crate::timing::TimingError::RelativeScheduleConflict`]
/// to the caller before this function is ever reached.
///
/// # Post-conversion invariant
///
/// The raw relative value MUST NOT appear in any scene graph state, telemetry
/// record, or stored mutation after this call (spec lines 287-289).  Callers
/// MUST discard the original [`Schedule`] and use only the returned `WallUs`.
pub fn resolve_schedule(schedule: Option<Schedule>, ctx: &IntakeContext) -> WallUs {
    match schedule {
        None | Some(Schedule::PresentAt(WallUs(0))) => {
            // `None` or zero PresentAt → immediate (WallUs::NOT_SET).
            WallUs::NOT_SET
        }
        Some(Schedule::PresentAt(t)) => t,
        Some(Schedule::AfterUs(d)) => resolve_after_us(d, ctx),
        Some(Schedule::FramesFromNow(n)) => resolve_frames_from_now(n, ctx),
        Some(Schedule::NextFrame) => resolve_next_frame(ctx),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a canonical [`IntakeContext`] for tests.
    ///
    /// Defaults:
    /// - `intake_mono_us = 10_000_000` (10 s)
    /// - `clock_offset = ClockOffset(0)` (wall == mono)
    /// - `vsync_wall_us = 10_000_000` (10 s)
    /// - `frame_period_us = DurationUs(16_666)` (≈60 fps)
    fn ctx() -> IntakeContext {
        IntakeContext {
            intake_mono_us: MonoUs(10_000_000),
            clock_offset: ClockOffset(0),
            vsync_wall_us: WallUs(10_000_000),
            frame_period_us: DurationUs(16_666),
        }
    }

    // ── after_us conversion (spec lines 275-277) ──────────────────────────────

    /// WHEN after_us = 500,000 (500ms) THEN present_at = intake_mono + 500ms
    /// (with zero skew → wall = mono).
    #[test]
    fn after_us_500ms_zero_skew() {
        let ctx = ctx();
        let wall = resolve_after_us(DurationUs(500_000), &ctx);
        // target_mono = 10_000_000 + 500_000 = 10_500_000
        // skew = 0 → wall = 10_500_000
        assert_eq!(wall, WallUs(10_500_000));
    }

    /// WHEN clock_offset is positive (wall ahead of mono by 100ms) THEN wall
    /// time is target_mono + offset.
    #[test]
    fn after_us_positive_skew() {
        let mut ctx = ctx();
        ctx.clock_offset = ClockOffset(100_000); // wall is 100ms ahead of mono
        let wall = resolve_after_us(DurationUs(0), &ctx);
        // target_mono = 10_000_000; wall = 10_000_000 + 100_000 = 10_100_000
        assert_eq!(wall, WallUs(10_100_000));
    }

    /// WHEN clock_offset is negative (mono ahead of wall by 200ms) THEN wall
    /// = mono - 200ms.
    #[test]
    fn after_us_negative_skew() {
        let mut ctx = ctx();
        ctx.clock_offset = ClockOffset(-200_000); // mono is 200ms ahead of wall
        let wall = resolve_after_us(DurationUs(0), &ctx);
        // wall = 10_000_000 - 200_000 = 9_800_000
        assert_eq!(wall, WallUs(9_800_000));
    }

    /// WHEN after_us is very large THEN arithmetic saturates rather than
    /// overflowing.
    #[test]
    fn after_us_saturation() {
        let ctx = ctx();
        let wall = resolve_after_us(DurationUs(u64::MAX), &ctx);
        assert_eq!(wall, WallUs(u64::MAX));
    }

    /// WHEN clock_offset would push result to zero (NOT_SET sentinel) THEN
    /// result is clamped to 1.
    #[test]
    fn after_us_negative_skew_clamps_to_one() {
        let mut ctx = ctx();
        ctx.intake_mono_us = MonoUs(1); // near zero
        ctx.clock_offset = ClockOffset(i64::MIN); // maximally negative offset
        let wall = resolve_after_us(DurationUs(0), &ctx);
        assert_eq!(wall.0, 1, "must not return the NOT_SET sentinel (0)");
    }

    // ── frames_from_now conversion (spec lines 279-281) ───────────────────────

    /// WHEN frames_from_now = 3 THEN present_at = vsync + 3 * frame_period.
    #[test]
    fn frames_from_now_3() {
        let ctx = ctx();
        let wall = resolve_frames_from_now(3, &ctx);
        // vsync = 10_000_000; 3 * 16_666 = 49_998
        assert_eq!(wall, WallUs(10_049_998));
    }

    /// WHEN frames_from_now = 0 THEN present_at = vsync (current frame).
    #[test]
    fn frames_from_now_0_is_current_vsync() {
        let ctx = ctx();
        let wall = resolve_frames_from_now(0, &ctx);
        assert_eq!(wall, WallUs(10_000_000));
    }

    /// WHEN the computed offset would overflow u64 THEN arithmetic saturates.
    ///
    /// Use a frame_period_us large enough to guarantee overflow with u32::MAX
    /// frames. For this test we set frame_period_us to a value where
    /// `u32::MAX * frame_period_us > u64::MAX`:
    /// - `u64::MAX / u32::MAX ≈ 4_294_967_297` so any frame_period_us > that
    ///   guarantees saturating_mul overflows.
    #[test]
    fn frames_from_now_saturation() {
        let mut ctx = ctx();
        // Make frame_period_us large enough that u32::MAX * frame_period_us
        // overflows u64.
        ctx.frame_period_us = DurationUs(5_000_000_000); // 5000 s per frame — pathological
        let wall = resolve_frames_from_now(u32::MAX, &ctx);
        // (u32::MAX as u64).saturating_mul(5_000_000_000) overflows → u64::MAX
        // then vsync + u64::MAX saturates again → u64::MAX
        assert_eq!(wall, WallUs(u64::MAX));
    }

    // ── next_frame equivalence (spec lines 283-285) ───────────────────────────

    /// WHEN next_frame = true THEN result is identical to frames_from_now = 1.
    #[test]
    fn next_frame_equals_frames_from_now_1() {
        let ctx = ctx();
        assert_eq!(resolve_next_frame(&ctx), resolve_frames_from_now(1, &ctx));
    }

    /// WHEN next_frame is used THEN result is strictly after the current vsync.
    #[test]
    fn next_frame_is_after_current_vsync() {
        let ctx = ctx();
        let wall = resolve_next_frame(&ctx);
        assert!(
            wall.as_u64() > ctx.vsync_wall_us.as_u64(),
            "next_frame MUST NOT be applied in the current frame"
        );
    }

    // ── resolve_schedule (primary entry point) ────────────────────────────────

    /// WHEN schedule = None THEN present_at = WallUs::NOT_SET (immediate).
    #[test]
    fn resolve_none_is_immediate() {
        let ctx = ctx();
        let wall = resolve_schedule(None, &ctx);
        assert_eq!(wall, WallUs::NOT_SET);
    }

    /// WHEN schedule = PresentAt(0) THEN present_at = WallUs::NOT_SET.
    #[test]
    fn resolve_present_at_zero_is_immediate() {
        let ctx = ctx();
        let wall = resolve_schedule(Some(Schedule::PresentAt(WallUs(0))), &ctx);
        assert_eq!(wall, WallUs::NOT_SET);
    }

    /// WHEN schedule = PresentAt(T) with T != 0 THEN present_at = T unchanged.
    #[test]
    fn resolve_present_at_passthrough() {
        let ctx = ctx();
        let t = WallUs(99_999_999);
        let wall = resolve_schedule(Some(Schedule::PresentAt(t)), &ctx);
        assert_eq!(wall, t);
    }

    /// WHEN schedule = AfterUs(d) THEN result = resolve_after_us(d, ctx).
    #[test]
    fn resolve_after_us_delegates() {
        let ctx = ctx();
        let d = DurationUs(1_000_000);
        let expected = resolve_after_us(d, &ctx);
        let got = resolve_schedule(Some(Schedule::AfterUs(d)), &ctx);
        assert_eq!(got, expected);
    }

    /// WHEN schedule = FramesFromNow(n) THEN result = resolve_frames_from_now(n, ctx).
    #[test]
    fn resolve_frames_from_now_delegates() {
        let ctx = ctx();
        let n = 5u32;
        let expected = resolve_frames_from_now(n, &ctx);
        let got = resolve_schedule(Some(Schedule::FramesFromNow(n)), &ctx);
        assert_eq!(got, expected);
    }

    /// WHEN schedule = NextFrame THEN result = resolve_next_frame(ctx).
    #[test]
    fn resolve_next_frame_delegates() {
        let ctx = ctx();
        let expected = resolve_next_frame(&ctx);
        let got = resolve_schedule(Some(Schedule::NextFrame), &ctx);
        assert_eq!(got, expected);
    }

    // ── Post-conversion invariant: relative fields not stored ─────────────────
    // Spec lines 287-289: after conversion, the raw relative value MUST NOT
    // appear in any scene graph state, telemetry record, or stored mutation.
    //
    // This is a contract enforced by the calling code (Stage 3 intake), not by
    // this module. The tests below verify that `resolve_schedule` always returns
    // an absolute WallUs — callers can then discard the original Schedule.

    /// AfterUs resolves to an absolute WallUs that carries no relative info.
    #[test]
    fn after_us_yields_absolute_wall_time() {
        let ctx = ctx();
        let wall = resolve_schedule(Some(Schedule::AfterUs(DurationUs(500_000))), &ctx);
        // Result is a WallUs — no relative info survives.
        assert!(wall.is_set(), "resolved value must be an absolute timestamp");
    }

    /// FramesFromNow resolves to an absolute WallUs.
    #[test]
    fn frames_from_now_yields_absolute_wall_time() {
        let ctx = ctx();
        let wall = resolve_schedule(Some(Schedule::FramesFromNow(2)), &ctx);
        assert!(wall.is_set());
    }

    /// NextFrame resolves to an absolute WallUs.
    #[test]
    fn next_frame_yields_absolute_wall_time() {
        let ctx = ctx();
        let wall = resolve_schedule(Some(Schedule::NextFrame), &ctx);
        assert!(wall.is_set());
    }

    // ── Composition with pending queue ────────────────────────────────────────
    // Verify that resolved timestamps can be inserted into a PendingQueue
    // and are subject to the same drain semantics as absolute present_at values.

    #[test]
    fn resolved_after_us_enters_pending_queue_correctly() {
        use crate::timing::pending_queue::{PendingEntry, PendingQueue};

        let ctx = ctx();
        // after_us = 500ms → present_at = intake_mono + 500ms = 10_500_000
        let present_at = resolve_after_us(DurationUs(500_000), &ctx);

        let mut queue: PendingQueue<u32> = PendingQueue::new(256);
        queue
            .push(PendingEntry {
                present_at_wall_us: present_at,
                payload: 42,
            })
            .unwrap();

        // Not drained at vsync = 10_000_000 (before present_at)
        let ready = queue.drain_ready(WallUs(10_000_000));
        assert!(ready.is_empty(), "must wait for present_at");

        // Drained at vsync = 10_500_000 (== present_at)
        let ready = queue.drain_ready(WallUs(10_500_000));
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].payload, 42);
    }

    #[test]
    fn resolved_frames_from_now_enters_pending_queue_correctly() {
        use crate::timing::pending_queue::{PendingEntry, PendingQueue};

        let ctx = ctx();
        // frames_from_now = 2 → vsync + 2 * 16_666 = 10_033_332
        let present_at = resolve_frames_from_now(2, &ctx);

        let mut queue: PendingQueue<u32> = PendingQueue::new(256);
        queue
            .push(PendingEntry {
                present_at_wall_us: present_at,
                payload: 99,
            })
            .unwrap();

        // Not drained before present_at
        let ready = queue.drain_ready(WallUs(10_000_000));
        assert!(ready.is_empty());

        // Drained at or after present_at
        let ready = queue.drain_ready(present_at);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].payload, 99);
    }

    #[test]
    fn resolved_next_frame_enters_pending_queue_correctly() {
        use crate::timing::pending_queue::{PendingEntry, PendingQueue};

        let ctx = ctx();
        let present_at = resolve_next_frame(&ctx);

        let mut queue: PendingQueue<u32> = PendingQueue::new(256);
        queue
            .push(PendingEntry {
                present_at_wall_us: present_at,
                payload: 7,
            })
            .unwrap();

        // Not drained at current vsync
        let ready = queue.drain_ready(ctx.vsync_wall_us);
        assert!(ready.is_empty(), "next_frame must NOT apply in current frame");

        // Drained at next vsync
        let ready = queue.drain_ready(present_at);
        assert_eq!(ready.len(), 1);
    }

    // ── Multiple entries: relative alongside absolute ─────────────────────────
    // Verify relative scheduling is sugar into the same absolute pipeline.

    #[test]
    fn relative_and_absolute_share_same_queue() {
        use crate::timing::pending_queue::{PendingEntry, PendingQueue};

        let ctx = ctx();
        let absolute_present = WallUs(10_500_000);
        let relative_present = resolve_after_us(DurationUs(500_000), &ctx); // → 10_500_000

        // Both should resolve to the same wall time when skew = 0 and
        // intake_mono == vsync_wall.
        assert_eq!(relative_present, absolute_present);

        let mut queue: PendingQueue<u32> = PendingQueue::new(256);
        queue
            .push(PendingEntry {
                present_at_wall_us: absolute_present,
                payload: 1,
            })
            .unwrap();
        queue
            .push(PendingEntry {
                present_at_wall_us: relative_present,
                payload: 2,
            })
            .unwrap();

        // Both drain at the same vsync
        let ready = queue.drain_ready(WallUs(10_500_000));
        assert_eq!(ready.len(), 2);
    }
}
