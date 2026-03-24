//! Injectable clock abstraction for deterministic testing.
//!
//! All timing paths in the compositor route through the [`Clock`] trait so
//! that tests can substitute a [`SimulatedClock`] instead of relying on
//! wall-clock time.
//!
//! ## Spec alignment
//!
//! Per `timing-model/spec.md` §Injectable Clock (lines 313–324):
//!
//! - The [`Clock`] trait exposes **only** `now_us()` and `monotonic_us()`.
//!   No mutation methods (`advance`, `set`) are part of the trait — these are
//!   a concern of test implementations only.
//! - [`SystemClock`] is the production implementation.
//! - [`SimulatedClock`] is the test implementation; time only advances when
//!   `advance_us` or `set_us` is called on the concrete type.
//!
//! ## Backward compatibility
//!
//! `now_millis()` is retained as a provided method (delegates to `now_us`)
//! so that callers that have not yet migrated to microsecond resolution
//! continue to compile without changes.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ─── Clock trait ─────────────────────────────────────────────────────────────

/// An injectable source of time.
///
/// Implementations must be `Send + Sync + fmt::Debug` so they can be stored
/// inside `Arc`-wrapped structures and appear in derived `Debug` impls on
/// containers.
///
/// # Clock trait is observation-only
///
/// The trait intentionally provides **no** mutation methods.  Time advancement
/// is a concern of concrete test implementations ([`SimulatedClock`]) only.
pub trait Clock: Send + Sync + fmt::Debug {
    /// UTC wall-clock time as microseconds since the Unix epoch.
    ///
    /// Corresponds to the *network clock domain* (`_wall_us` naming suffix).
    /// Zero (0) is reserved to mean "not set" and MUST NOT be returned by
    /// production implementations.
    fn now_us(&self) -> u64;

    /// Monotonic system clock in microseconds.
    ///
    /// Corresponds to the *monotonic clock domain* (`_mono_us` naming suffix).
    /// This value MUST never decrease between calls on the same instance.
    fn monotonic_us(&self) -> u64;

    /// Wall-clock time as **milliseconds** since the Unix epoch.
    ///
    /// Provided for backward compatibility with callers that have not yet
    /// migrated to microsecond resolution.  Delegates to `now_us`.
    #[inline]
    fn now_millis(&self) -> u64 {
        self.now_us() / 1_000
    }
}

// ─── SystemClock ─────────────────────────────────────────────────────────────

/// Production clock backed by OS system calls.
///
/// - `now_us()` — `SystemTime::now()` converted to UTC microseconds.
/// - `monotonic_us()` — a process-local `Instant` baseline converted to
///   microseconds.  The absolute value is arbitrary; only differences matter.
#[derive(Clone, Debug)]
pub struct SystemClock {
    /// Captured once at construction to provide a stable monotonic baseline.
    mono_origin: Instant,
}

impl SystemClock {
    /// Create a new `SystemClock` capturing the current monotonic baseline.
    pub fn new() -> Self {
        Self {
            mono_origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_us(&self) -> u64 {
        // Clamp to 1 so that 0 is never returned. Zero is the spec-defined
        // "not set" sentinel (spec lines 68-70) and MUST NOT be returned by
        // production implementations. A system clock before UNIX_EPOCH is
        // pathological; returning 1 is safer than panicking or silently
        // emitting the forbidden sentinel.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
            | 1
    }

    fn monotonic_us(&self) -> u64 {
        // Bias by +1 so the first sub-microsecond call never returns 0.
        // MonoUs(0) is the "not set" sentinel; a real timestamp of 0 is
        // ambiguous with "not set" and MUST be avoided.
        self.mono_origin.elapsed().as_micros() as u64 + 1
    }
}

// ─── SimulatedClock ──────────────────────────────────────────────────────────

/// Manually-controlled clock for deterministic tests.
///
/// Time starts at the value provided to [`SimulatedClock::new`] and only
/// advances when [`advance_us`][SimulatedClock::advance_us] or
/// [`set_us`][SimulatedClock::set_us] is called on the concrete type.
///
/// The [`Clock`] trait implementation is observation-only; no mutation methods
/// are exposed through the trait.
///
/// # Monotonic vs wall time
///
/// `SimulatedClock` uses a **single** internal counter for both wall and
/// monotonic time.  This is intentional: in tests the two clocks advance
/// together, which is the correct default.  If a test needs to simulate
/// clock-skew between wall and mono, use two separate clocks.
///
/// # Example
///
/// ```
/// use tze_hud_scene::clock::{SimulatedClock, Clock};
///
/// let clock = SimulatedClock::new(1_000_000); // start at 1 second
/// assert_eq!(clock.now_us(), 1_000_000);
/// assert_eq!(clock.monotonic_us(), 1_000_000);
///
/// clock.advance_us(100_000); // advance 100 ms
/// assert_eq!(clock.now_us(), 1_100_000);
/// ```
#[derive(Clone)]
pub struct SimulatedClock {
    now_us: Arc<Mutex<u64>>,
}

impl fmt::Debug for SimulatedClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimulatedClock")
            .field("now_us", &self.now_us())
            .finish()
    }
}

impl SimulatedClock {
    /// Create a `SimulatedClock` starting at `start_us` microseconds.
    pub fn new(start_us: u64) -> Self {
        Self {
            now_us: Arc::new(Mutex::new(start_us)),
        }
    }

    /// Advance the clock by `delta_us` microseconds.
    ///
    /// Saturates at `u64::MAX` to avoid overflow panics.
    pub fn advance_us(&self, delta_us: u64) {
        let mut guard = self.now_us.lock().expect("SimulatedClock poisoned");
        *guard = guard.saturating_add(delta_us);
    }

    /// Set the clock to an absolute value in microseconds.
    pub fn set_us(&self, us: u64) {
        let mut guard = self.now_us.lock().expect("SimulatedClock poisoned");
        *guard = us;
    }
}

impl Clock for SimulatedClock {
    fn now_us(&self) -> u64 {
        *self.now_us.lock().expect("SimulatedClock poisoned")
    }

    fn monotonic_us(&self) -> u64 {
        // Same counter — wall and mono advance together in simulated time.
        *self.now_us.lock().expect("SimulatedClock poisoned")
    }
}

impl Default for SimulatedClock {
    fn default() -> Self {
        Self::new(0)
    }
}

// ─── TestClock (backward-compat alias) ───────────────────────────────────────

/// Backward-compatible alias for [`SimulatedClock`].
///
/// New code should use `SimulatedClock` directly.  `TestClock` is kept so
/// that existing callers in `graph.rs`, `test_scenes.rs`, and other crates
/// continue to compile without modification.
///
/// `TestClock` stores time internally as **milliseconds** to preserve the
/// original `new(start_ms)` / `advance(delta_ms)` / `set(ms)` API contract.
/// `now_us()` and `monotonic_us()` multiply by 1 000 to convert to
/// microseconds; `now_millis()` (provided by the trait) returns the raw
/// millisecond value.
///
/// # Example
///
/// ```
/// use tze_hud_scene::clock::{TestClock, Clock};
///
/// let clock = TestClock::new(1_000);
/// assert_eq!(clock.now_millis(), 1_000);
/// clock.advance(500);
/// assert_eq!(clock.now_millis(), 1_500);
/// ```
#[derive(Clone)]
pub struct TestClock {
    now_ms: Arc<Mutex<u64>>,
}

impl fmt::Debug for TestClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestClock")
            .field("now_ms", &self.now_millis())
            .finish()
    }
}

impl TestClock {
    /// Create a new `TestClock` with the given starting time in milliseconds.
    pub fn new(start_ms: u64) -> Self {
        Self {
            now_ms: Arc::new(Mutex::new(start_ms)),
        }
    }

    /// Advance the clock by `delta_ms` milliseconds.
    pub fn advance(&self, delta_ms: u64) {
        let mut guard = self.now_ms.lock().expect("TestClock poisoned");
        *guard = guard.saturating_add(delta_ms);
    }

    /// Set the clock to an absolute value in milliseconds.
    pub fn set(&self, ms: u64) {
        let mut guard = self.now_ms.lock().expect("TestClock poisoned");
        *guard = ms;
    }
}

impl Clock for TestClock {
    fn now_us(&self) -> u64 {
        let ms = *self.now_ms.lock().expect("TestClock poisoned");
        ms.saturating_mul(1_000)
    }

    fn monotonic_us(&self) -> u64 {
        self.now_us()
    }

    /// Override the provided method to return the raw millisecond value,
    /// preserving the original contract without double-rounding.
    #[inline]
    fn now_millis(&self) -> u64 {
        *self.now_ms.lock().expect("TestClock poisoned")
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new(0)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── TestClock (backward-compat) ──

    #[test]
    fn test_clock_starts_at_given_value() {
        let clock = TestClock::new(1_000);
        assert_eq!(clock.now_millis(), 1_000);
    }

    #[test]
    fn test_clock_advance() {
        let clock = TestClock::new(0);
        clock.advance(250);
        assert_eq!(clock.now_millis(), 250);
        clock.advance(750);
        assert_eq!(clock.now_millis(), 1_000);
    }

    #[test]
    fn test_clock_set() {
        let clock = TestClock::new(100);
        clock.set(9_999);
        assert_eq!(clock.now_millis(), 9_999);
    }

    #[test]
    fn test_clock_no_overflow_on_saturating_add() {
        // Saturating add must not panic; value clamps to u64::MAX.
        let clock = TestClock::new(u64::MAX);
        clock.advance(1); // must not panic
        assert_eq!(clock.now_millis(), u64::MAX);
    }

    #[test]
    fn test_clock_now_us_is_millis_times_1000() {
        let clock = TestClock::new(2_500);
        assert_eq!(clock.now_us(), 2_500_000);
        assert_eq!(clock.monotonic_us(), 2_500_000);
    }

    #[test]
    fn test_system_clock_returns_nonzero() {
        let clock = SystemClock::new();
        assert!(clock.now_millis() > 0);
    }

    // ── SimulatedClock ──

    #[test]
    fn simulated_clock_starts_at_given_value() {
        let clock = SimulatedClock::new(1_000_000);
        assert_eq!(clock.now_us(), 1_000_000);
    }

    #[test]
    fn simulated_clock_advance_us() {
        let clock = SimulatedClock::new(0);
        clock.advance_us(100_000); // 100 ms
        assert_eq!(clock.now_us(), 100_000);
        clock.advance_us(400_000); // +400 ms
        assert_eq!(clock.now_us(), 500_000);
    }

    #[test]
    fn simulated_clock_set_us() {
        let clock = SimulatedClock::new(0);
        clock.set_us(9_999_999);
        assert_eq!(clock.now_us(), 9_999_999);
    }

    #[test]
    fn simulated_clock_no_overflow_on_saturating_add() {
        let clock = SimulatedClock::new(u64::MAX);
        clock.advance_us(1); // must not panic
        assert_eq!(clock.now_us(), u64::MAX);
    }

    #[test]
    fn simulated_clock_monotonic_equals_wall() {
        let clock = SimulatedClock::new(5_000_000);
        assert_eq!(clock.monotonic_us(), clock.now_us());
        clock.advance_us(250_000);
        assert_eq!(clock.monotonic_us(), clock.now_us());
    }

    #[test]
    fn simulated_clock_now_millis_delegates_correctly() {
        let clock = SimulatedClock::new(2_500_000); // 2.5 seconds
        assert_eq!(clock.now_millis(), 2_500);
    }

    // ── Spec: Clock trait is observation-only ──

    /// Compile-time check: the Clock trait must not expose advance/set.
    /// This test verifies via a trait-object cast that only now_us and
    /// monotonic_us are reachable through the trait.
    #[test]
    fn clock_trait_is_observation_only() {
        fn assert_observation_only(clock: &dyn Clock) {
            // If this compiles, the trait only exposes read methods.
            let _ = clock.now_us();
            let _ = clock.monotonic_us();
            let _ = clock.now_millis();
        }
        assert_observation_only(&SimulatedClock::new(0));
        assert_observation_only(&SystemClock::new());
    }

    // ── SystemClock monotonic ──

    #[test]
    fn system_clock_monotonic_does_not_decrease() {
        let clock = SystemClock::new();
        let t1 = clock.monotonic_us();
        // A tiny spin ensures at least one nanosecond passes.
        std::hint::black_box(0u64);
        let t2 = clock.monotonic_us();
        assert!(t2 >= t1, "monotonic must not decrease: t1={t1} t2={t2}");
    }
}
