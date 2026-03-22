//! Injectable clock abstraction for deterministic testing.
//!
//! The scene graph calls `now_millis()` in several places (lease grant, tab
//! creation, expiry). By routing through a `Clock` trait the caller can
//! substitute a `TestClock` in tests instead of sleeping or accepting
//! non-deterministic system time.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of wall-clock time expressed as milliseconds since the Unix epoch.
///
/// Implementations must be `Send + Sync + fmt::Debug` so they can be stored
/// inside `Arc`-wrapped structures and derived `Debug` impls on containers.
pub trait Clock: Send + Sync + fmt::Debug {
    /// Return the current time as milliseconds since the Unix epoch.
    fn now_millis(&self) -> u64;
}

// ─── SystemClock ─────────────────────────────────────────────────────────────

/// Production clock backed by `SystemTime`.
#[derive(Clone, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

// ─── TestClock ───────────────────────────────────────────────────────────────

/// Manually-controlled clock for deterministic tests.
///
/// Time starts at 0 and only advances when `advance` or `set` is called.
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

    /// Set the clock to an absolute value.
    pub fn set(&self, ms: u64) {
        let mut guard = self.now_ms.lock().expect("TestClock poisoned");
        *guard = ms;
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> u64 {
        *self.now_ms.lock().expect("TestClock poisoned")
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let clock = TestClock::new(u64::MAX);
        clock.advance(1); // must not panic
        assert_eq!(clock.now_millis(), u64::MAX);
    }

    #[test]
    fn test_system_clock_returns_nonzero() {
        let clock = SystemClock;
        assert!(clock.now_millis() > 0);
    }
}
