//! Clock-domain newtype wrappers.
//!
//! tze_hud recognises four clock domains (spec `timing-model/spec.md` §Clock
//! Domain Separation, lines 10–21).  This module provides compile-time
//! type-safe wrappers for the two that appear in v1 Rust/proto fields:
//!
//! | Wrapper       | Domain         | Field suffix | Unit                         |
//! |---------------|----------------|--------------|------------------------------|
//! | [`WallUs`]    | Network (UTC)  | `_wall_us`   | UTC microseconds since epoch |
//! | [`MonoUs`]    | Monotonic OS   | `_mono_us`   | Monotonic microseconds       |
//! | [`DurationUs`]| —              | *(delta)*    | Microsecond delta            |
//!
//! `WallUs` and `MonoUs` are **not interchangeable**: passing one where the
//! other is expected is a compile-time error.  Converting between them
//! requires an explicit [`ClockOffset`] calibration value.
//!
//! ## Zero-value semantics
//!
//! Per spec lines 68–70: a timestamp of `0` means "not set".
//! [`WallUs::is_set`] and [`MonoUs::is_set`] encode this convention.
//!
//! ## Field naming convention
//!
//! All timestamp fields in proto and Rust structs MUST encode their domain in
//! the suffix:
//! - `_wall_us` — use [`WallUs`]
//! - `_mono_us` — use [`MonoUs`]
//! - no domain suffix — use [`DurationUs`] (delta / frame-relative, not a
//!   timestamp)
//!
//! A plain `_us` suffix without domain indicator MUST NOT be used for
//! absolute timestamps.

use serde::{Deserialize, Serialize};

// ─── WallUs ──────────────────────────────────────────────────────────────────

/// UTC wall-clock timestamp in microseconds since the Unix epoch.
///
/// Corresponds to the *network clock domain* and MUST be used for fields
/// with the `_wall_us` suffix (e.g. `present_at_wall_us`, `created_at_wall_us`,
/// `session_open_wall_us`).
///
/// # Zero semantics
///
/// `WallUs(0)` means "not set" (spec lines 68–70).  Production clocks MUST NOT
/// return 0.  Use [`WallUs::NOT_SET`] as the canonical sentinel.
///
/// # Cross-domain assignment is a compile error
///
/// ```compile_fail
/// use tze_hud_scene::timing::{WallUs, MonoUs};
///
/// let wall: WallUs = WallUs(1_000_000);
/// let mono: MonoUs = wall; // ERROR: mismatched types
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
         Serialize, Deserialize)]
#[repr(transparent)]
pub struct WallUs(pub u64);

impl WallUs {
    /// Sentinel value meaning "not set".  Always `WallUs(0)`.
    pub const NOT_SET: Self = Self(0);

    /// `true` if this timestamp carries a real value (i.e. is non-zero).
    #[inline]
    pub fn is_set(self) -> bool {
        self.0 != 0
    }

    /// Raw microsecond value.
    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Convert to [`MonoUs`] using a calibration offset.
    ///
    /// `offset = wall_us - mono_us` at the calibration point.
    #[inline]
    pub fn to_mono(self, offset: ClockOffset) -> MonoUs {
        // Use saturating_neg() instead of the unary `-` operator to avoid
        // overflow when offset.0 == i64::MIN (which would panic in debug
        // builds and wrap in release).
        MonoUs(self.0.saturating_add_signed(offset.0.saturating_neg()))
    }
}

impl From<u64> for WallUs {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<WallUs> for u64 {
    fn from(v: WallUs) -> Self {
        v.0
    }
}

impl std::fmt::Display for WallUs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}µs(wall)", self.0)
    }
}

// ─── MonoUs ──────────────────────────────────────────────────────────────────

/// Monotonic system-clock timestamp in microseconds.
///
/// Corresponds to the *monotonic clock domain* and MUST be used for fields
/// with the `_mono_us` suffix (e.g. `vsync_mono_us`, `session_open_mono_us`,
/// `timestamp_mono_us`).
///
/// Monotonic values MUST NOT be compared directly with wall-clock values.
/// Use [`MonoUs::to_wall`] with a [`ClockOffset`] for inter-domain arithmetic.
///
/// # Zero semantics
///
/// `MonoUs(0)` means "not set".  Use [`MonoUs::NOT_SET`] as the sentinel.
///
/// # Cross-domain assignment is a compile error
///
/// ```compile_fail
/// use tze_hud_scene::timing::{MonoUs, WallUs};
///
/// let mono: MonoUs = MonoUs(5_000_000);
/// let wall: WallUs = mono; // ERROR: mismatched types
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
         Serialize, Deserialize)]
#[repr(transparent)]
pub struct MonoUs(pub u64);

impl MonoUs {
    /// Sentinel value meaning "not set".  Always `MonoUs(0)`.
    pub const NOT_SET: Self = Self(0);

    /// `true` if this timestamp carries a real value (i.e. is non-zero).
    #[inline]
    pub fn is_set(self) -> bool {
        self.0 != 0
    }

    /// Raw microsecond value.
    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Convert to [`WallUs`] using a calibration offset.
    ///
    /// `offset = wall_us - mono_us` at the calibration point.
    #[inline]
    pub fn to_wall(self, offset: ClockOffset) -> WallUs {
        WallUs(self.0.saturating_add_signed(offset.0))
    }
}

impl From<u64> for MonoUs {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<MonoUs> for u64 {
    fn from(v: MonoUs) -> Self {
        v.0
    }
}

impl std::fmt::Display for MonoUs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}µs(mono)", self.0)
    }
}

// ─── DurationUs ──────────────────────────────────────────────────────────────

/// A duration (delta) in microseconds — NOT a timestamp.
///
/// Use this type for fields that express an interval or offset rather than an
/// absolute point in time.  Such fields MUST NOT carry a `_wall_us` or
/// `_mono_us` suffix; they use a plain unit description (e.g. `after_us`,
/// `duration_us`, `ttl_us`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
         Serialize, Deserialize)]
#[repr(transparent)]
pub struct DurationUs(pub u64);

impl DurationUs {
    /// Zero duration.
    pub const ZERO: Self = Self(0);

    /// Raw microsecond value.
    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Add this duration to a [`WallUs`] timestamp.
    #[inline]
    pub fn after_wall(self, base: WallUs) -> WallUs {
        WallUs(base.0.saturating_add(self.0))
    }

    /// Add this duration to a [`MonoUs`] timestamp.
    #[inline]
    pub fn after_mono(self, base: MonoUs) -> MonoUs {
        MonoUs(base.0.saturating_add(self.0))
    }
}

impl From<u64> for DurationUs {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<DurationUs> for u64 {
    fn from(v: DurationUs) -> Self {
        v.0
    }
}

impl std::fmt::Display for DurationUs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}µs", self.0)
    }
}

// ─── ClockOffset ─────────────────────────────────────────────────────────────

/// Calibration offset used to convert between [`WallUs`] and [`MonoUs`].
///
/// Computed at session open as:
/// ```text
/// offset = session_open_wall_us - session_open_mono_us
/// ```
///
/// A positive offset means the wall clock is ahead of the monotonic clock.
/// The value is signed to handle the case where monotonic is ahead of
/// an absolute wall timestamp (e.g. when wall time is set in the past).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockOffset(pub i64);

impl ClockOffset {
    /// Compute the offset from a (wall, mono) calibration pair.
    ///
    /// Returns `None` if the subtraction would overflow.
    pub fn from_pair(wall: WallUs, mono: MonoUs) -> Option<Self> {
        (wall.0 as i128)
            .checked_sub(mono.0 as i128)
            .and_then(|v| i64::try_from(v).ok())
            .map(Self)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── WallUs ──

    #[test]
    fn wall_us_not_set_is_zero() {
        assert_eq!(WallUs::NOT_SET.0, 0);
        assert!(!WallUs::NOT_SET.is_set());
    }

    #[test]
    fn wall_us_nonzero_is_set() {
        assert!(WallUs(1).is_set());
        assert!(WallUs(u64::MAX).is_set());
    }

    #[test]
    fn wall_us_from_u64() {
        let v: WallUs = WallUs::from(42_000_000);
        assert_eq!(v.as_u64(), 42_000_000);
    }

    #[test]
    fn wall_us_roundtrip_u64() {
        let v = WallUs(999);
        assert_eq!(u64::from(v), 999);
    }

    #[test]
    fn wall_us_display() {
        assert_eq!(format!("{}", WallUs(1_500_000)), "1500000µs(wall)");
    }

    // ── MonoUs ──

    #[test]
    fn mono_us_not_set_is_zero() {
        assert_eq!(MonoUs::NOT_SET.0, 0);
        assert!(!MonoUs::NOT_SET.is_set());
    }

    #[test]
    fn mono_us_nonzero_is_set() {
        assert!(MonoUs(1).is_set());
    }

    #[test]
    fn mono_us_display() {
        assert_eq!(format!("{}", MonoUs(2_000_000)), "2000000µs(mono)");
    }

    // ── DurationUs ──

    #[test]
    fn duration_us_zero() {
        assert_eq!(DurationUs::ZERO.as_u64(), 0);
    }

    #[test]
    fn duration_after_wall() {
        let base = WallUs(1_000_000);
        let delta = DurationUs(500_000);
        assert_eq!(delta.after_wall(base), WallUs(1_500_000));
    }

    #[test]
    fn duration_after_mono() {
        let base = MonoUs(2_000_000);
        let delta = DurationUs(100_000);
        assert_eq!(delta.after_mono(base), MonoUs(2_100_000));
    }

    #[test]
    fn duration_after_wall_saturates_on_overflow() {
        let base = WallUs(u64::MAX);
        let delta = DurationUs(1);
        assert_eq!(delta.after_wall(base), WallUs(u64::MAX));
    }

    // ── ClockOffset ──

    #[test]
    fn clock_offset_from_pair_positive() {
        // wall > mono: offset is positive
        let offset = ClockOffset::from_pair(WallUs(2_000_000), MonoUs(1_000_000)).unwrap();
        assert_eq!(offset.0, 1_000_000);
    }

    #[test]
    fn clock_offset_from_pair_negative() {
        // mono > wall: offset is negative
        let offset = ClockOffset::from_pair(WallUs(1_000_000), MonoUs(2_000_000)).unwrap();
        assert_eq!(offset.0, -1_000_000);
    }

    #[test]
    fn clock_offset_from_pair_overflow_returns_none() {
        // u64::MAX - 0 overflows i64
        let result = ClockOffset::from_pair(WallUs(u64::MAX), MonoUs(0));
        assert!(result.is_none());
    }

    // ── Cross-domain conversion ──

    #[test]
    fn wall_to_mono_roundtrip() {
        let offset = ClockOffset(1_000_000); // wall is 1s ahead
        let wall = WallUs(5_000_000);
        let mono = wall.to_mono(offset);
        assert_eq!(mono, MonoUs(4_000_000));
        // Round-trip back
        assert_eq!(mono.to_wall(offset), wall);
    }

    #[test]
    fn mono_to_wall_roundtrip() {
        let offset = ClockOffset(-500_000); // mono is 0.5s ahead
        let mono = MonoUs(3_000_000);
        let wall = mono.to_wall(offset);
        assert_eq!(wall, WallUs(2_500_000));
        assert_eq!(wall.to_mono(offset), mono);
    }

    // ── Spec: cross-domain assignment is a compile error ──
    // The tests below are compile_fail doc-tests in the struct documentation.
    // They are not repeated here because they cannot be written as #[test].

    // ── Spec: zero-value semantics (spec lines 68-70) ──

    #[test]
    fn zero_means_not_set_wall() {
        // present_at_wall_us = 0 → "not set" / immediate
        let ts = WallUs(0);
        assert!(!ts.is_set(), "0 must mean 'not set'");
    }

    #[test]
    fn zero_means_not_set_mono() {
        let ts = MonoUs(0);
        assert!(!ts.is_set(), "0 must mean 'not set'");
    }

    // ── Ordering ──

    #[test]
    fn wall_us_ordering() {
        assert!(WallUs(100) < WallUs(200));
        assert!(WallUs(200) > WallUs(100));
        assert!(WallUs(100) == WallUs(100));
    }

    #[test]
    fn mono_us_ordering() {
        assert!(MonoUs(50) < MonoUs(51));
    }
}
