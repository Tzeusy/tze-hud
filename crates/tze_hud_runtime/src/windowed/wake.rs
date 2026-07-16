//! Event-driven wake scheduling for the production windowed runtime.
//!
//! The generation counter is the load-bearing missed-wakeup guard: callers
//! checkpoint before inspecting render work, then wait against that checkpoint.
//! A notification that races with inspection advances the generation, so the
//! waiter observes it without parking.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use winit::event_loop::{ControlFlow, EventLoopProxy};

use crate::idle_efficiency::{IdleEfficiencyCounters, RuntimeWakeupSource};

/// One explicit instant at which runtime-owned work becomes due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Deadline {
    pub(super) at: Instant,
    pub(super) source: RuntimeWakeupSource,
}

impl Deadline {
    pub(super) const fn new(at: Instant, source: RuntimeWakeupSource) -> Self {
        Self { at, source }
    }
}

/// Select the winit sleep policy from explicit deadlines.
///
/// Static idle has no deadline and therefore uses `Wait`. Due work is expressed
/// as `WaitUntil(now)` rather than `Poll`, keeping the production loop free of a
/// correctness dependency on bounded spinning.
pub(super) fn control_flow_for_deadlines(
    _now: Instant,
    deadlines: impl IntoIterator<Item = Deadline>,
) -> ControlFlow {
    deadlines
        .into_iter()
        .min_by_key(|deadline| deadline.at)
        .map_or(ControlFlow::Wait, |deadline| {
            ControlFlow::WaitUntil(deadline.at)
        })
}

pub(super) fn deadline_from_wall_us(
    deadline_wall_us: u64,
    source: RuntimeWakeupSource,
) -> Deadline {
    let now_wall_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;
    Deadline::new(
        Instant::now() + Duration::from_micros(deadline_wall_us.saturating_sub(now_wall_us)),
        source,
    )
}

#[derive(Debug)]
struct WakeState {
    generation: u64,
    source: RuntimeWakeupSource,
}

impl Default for WakeState {
    fn default() -> Self {
        Self {
            generation: 0,
            source: RuntimeWakeupSource::SceneChange,
        }
    }
}

/// Race-free compositor-thread wake primitive.
#[derive(Clone, Debug, Default)]
pub(super) struct CompositorWake {
    inner: Arc<(Mutex<WakeState>, Condvar)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WakeObservation {
    pub(super) generation: u64,
    pub(super) source: RuntimeWakeupSource,
    pub(super) timed_out: bool,
}

impl CompositorWake {
    /// Snapshot before inspecting work. Pass this value to `wait` after the
    /// inspection so a notification during that interval cannot be missed.
    pub(super) fn checkpoint(&self) -> u64 {
        self.inner
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .generation
    }

    pub(super) fn notify(&self, source: RuntimeWakeupSource) {
        let (state_lock, ready) = &*self.inner;
        let mut state = state_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.source = source;
        drop(state);
        ready.notify_one();
    }

    pub(super) fn wait(&self, checkpoint: u64, deadline: Option<Deadline>) -> WakeObservation {
        let (state_lock, ready) = &*self.inner;
        let mut state = state_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        while state.generation == checkpoint {
            let Some(deadline) = deadline else {
                state = ready
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                continue;
            };

            let now = Instant::now();
            if now >= deadline.at {
                return WakeObservation {
                    generation: state.generation,
                    source: deadline.source,
                    timed_out: true,
                };
            }
            let timeout = deadline.at.saturating_duration_since(now);
            let (next, result) = ready
                .wait_timeout(state, timeout)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            if result.timed_out() && state.generation == checkpoint {
                return WakeObservation {
                    generation: state.generation,
                    source: deadline.source,
                    timed_out: true,
                };
            }
        }

        WakeObservation {
            generation: state.generation,
            source: state.source,
            timed_out: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RuntimeWakeEvent;

/// Single production owner of cross-thread wake delivery and wake counters.
#[derive(Clone)]
pub(super) struct WindowedWake {
    proxy: Option<EventLoopProxy<RuntimeWakeEvent>>,
    proxy_event_pending: Arc<AtomicBool>,
    main_source: Arc<Mutex<RuntimeWakeupSource>>,
    compositor: CompositorWake,
    counters: Arc<IdleEfficiencyCounters>,
}

impl WindowedWake {
    pub(super) fn new(proxy: EventLoopProxy<RuntimeWakeEvent>) -> Self {
        Self {
            proxy: Some(proxy),
            proxy_event_pending: Arc::new(AtomicBool::new(false)),
            main_source: Arc::new(Mutex::new(RuntimeWakeupSource::SceneChange)),
            compositor: CompositorWake::default(),
            counters: Arc::new(IdleEfficiencyCounters::default()),
        }
    }

    #[cfg(test)]
    pub(super) fn disconnected() -> Self {
        Self {
            proxy: None,
            proxy_event_pending: Arc::new(AtomicBool::new(false)),
            main_source: Arc::new(Mutex::new(RuntimeWakeupSource::SceneChange)),
            compositor: CompositorWake::default(),
            counters: Arc::new(IdleEfficiencyCounters::default()),
        }
    }

    pub(super) fn render_notifier(&self) -> tze_hud_scene::render_wake::RenderWakeNotifier {
        let wake = self.clone();
        tze_hud_scene::render_wake::RenderWakeNotifier::new(move || {
            wake.notify(RuntimeWakeupSource::SceneChange);
        })
    }

    pub(super) fn notify(&self, source: RuntimeWakeupSource) {
        self.compositor.notify(source);
        self.notify_main(source);
    }

    pub(super) fn notify_main(&self, source: RuntimeWakeupSource) {
        *self
            .main_source
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = source;
        if !self.proxy_event_pending.swap(true, Ordering::AcqRel) {
            let sent = self
                .proxy
                .as_ref()
                .is_some_and(|proxy| proxy.send_event(RuntimeWakeEvent).is_ok());
            if !sent {
                self.proxy_event_pending.store(false, Ordering::Release);
            }
        }
    }

    pub(super) fn notify_compositor(&self, source: RuntimeWakeupSource) {
        self.compositor.notify(source);
    }

    pub(super) fn take_main_source(&self) -> RuntimeWakeupSource {
        self.proxy_event_pending.store(false, Ordering::Release);
        *self
            .main_source
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn compositor(&self) -> &CompositorWake {
        &self.compositor
    }

    pub(super) fn counters(&self) -> &Arc<IdleEfficiencyCounters> {
        &self.counters
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::idle_efficiency::{IdleEfficiencyCounters, RuntimeWakeupSource};

    use super::{CompositorWake, Deadline, control_flow_for_deadlines};

    #[test]
    fn static_idle_selects_wait_without_a_bounded_poll() {
        let now = Instant::now();
        assert_eq!(
            control_flow_for_deadlines(now, std::iter::empty()),
            winit::event_loop::ControlFlow::Wait
        );
    }

    #[test]
    fn earliest_future_deadline_selects_wait_until() {
        let now = Instant::now();
        let later = Deadline::new(
            now + Duration::from_millis(40),
            RuntimeWakeupSource::TtlDeadline,
        );
        let earlier = Deadline::new(
            now + Duration::from_millis(10),
            RuntimeWakeupSource::AnimationDeadline,
        );
        assert_eq!(
            control_flow_for_deadlines(now, [later, earlier]),
            winit::event_loop::ControlFlow::WaitUntil(earlier.at)
        );
    }

    #[test]
    fn already_due_work_uses_wait_until_not_poll() {
        let now = Instant::now();
        let due = Deadline::new(now, RuntimeWakeupSource::TtlDeadline);
        assert_eq!(
            control_flow_for_deadlines(now, [due]),
            winit::event_loop::ControlFlow::WaitUntil(now)
        );
    }

    #[test]
    fn benchmark_cadence_rearms_with_successive_wait_until_deadlines() {
        let start = Instant::now();
        let interval = Duration::from_millis(16);
        for frame in 1..=4 {
            let at = start + interval * frame;
            let deadline = Deadline::new(at, RuntimeWakeupSource::AnimationDeadline);
            assert_eq!(
                control_flow_for_deadlines(start, [deadline]),
                winit::event_loop::ControlFlow::WaitUntil(at)
            );
        }
    }

    #[test]
    fn sixty_seconds_of_static_idle_has_zero_runtime_wakeups() {
        let counters = IdleEfficiencyCounters::default();
        let baseline = counters.snapshot();
        let now = Instant::now();
        for second in 0..60 {
            let simulated_now = now + Duration::from_secs(second);
            assert_eq!(
                control_flow_for_deadlines(simulated_now, std::iter::empty()),
                winit::event_loop::ControlFlow::Wait
            );
        }
        let delta = counters
            .snapshot()
            .delta_since(&baseline)
            .expect("monotonic counters");
        assert_eq!(delta.combined_runtime_wakeups(), 0);
        assert!(delta.combined_runtime_wakeups() <= 120);
    }

    #[test]
    fn notification_before_park_is_observed_from_the_pre_work_checkpoint() {
        let wake = CompositorWake::default();
        let checkpoint = wake.checkpoint();
        wake.notify(RuntimeWakeupSource::SceneChange);

        let observed = wake.wait(checkpoint, None);
        assert_eq!(observed.source, RuntimeWakeupSource::SceneChange);
        assert!(!observed.timed_out);
        assert!(observed.generation > checkpoint);
    }

    #[test]
    fn notification_during_park_wakes_the_waiter() {
        let wake = CompositorWake::default();
        let checkpoint = wake.checkpoint();
        let notifier = wake.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            notifier.notify(RuntimeWakeupSource::SceneChange);
        });

        let observed = wake.wait(
            checkpoint,
            Some(Deadline::new(
                Instant::now() + Duration::from_secs(1),
                RuntimeWakeupSource::TtlDeadline,
            )),
        );
        thread.join().expect("notifier thread");
        assert_eq!(observed.source, RuntimeWakeupSource::SceneChange);
        assert!(!observed.timed_out);
    }

    #[test]
    fn timeout_is_attributed_to_its_deadline_source() {
        let wake = CompositorWake::default();
        let checkpoint = wake.checkpoint();
        let observed = wake.wait(
            checkpoint,
            Some(Deadline::new(
                Instant::now() + Duration::from_millis(1),
                RuntimeWakeupSource::TtlDeadline,
            )),
        );
        assert_eq!(observed.source, RuntimeWakeupSource::TtlDeadline);
        assert!(observed.timed_out);
    }
}
