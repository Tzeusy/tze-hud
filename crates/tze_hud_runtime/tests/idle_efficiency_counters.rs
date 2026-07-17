use tze_hud_runtime::{IdleEfficiencyCounters, RuntimeWakeupSource};

#[test]
fn measurement_delta_excludes_settling_activity_and_preserves_source_attribution() {
    let counters = IdleEfficiencyCounters::default();
    counters.record_compositor_wakeup(RuntimeWakeupSource::SceneChange);
    counters.record_gpu_submission();
    let baseline = counters.snapshot();

    counters.record_main_wakeup(RuntimeWakeupSource::FrameReady);
    counters.record_compositor_wakeup(RuntimeWakeupSource::AnimationDeadline);
    counters.record_surface_acquisition();
    counters.record_present();

    let measured = counters.snapshot().delta_since(&baseline).unwrap();
    assert_eq!(measured.main_loop_wakeups, 1);
    assert_eq!(measured.compositor_loop_wakeups, 1);
    assert_eq!(measured.gpu_queue_submissions, 0);
    assert_eq!(measured.surface_acquisitions, 1);
    assert_eq!(measured.presents, 1);
    assert_eq!(measured.sources["main.frame_ready"], 1);
    assert_eq!(measured.sources["compositor.animation_deadline"], 1);
    assert!(!measured.sources.contains_key("compositor.scene_change"));
}

#[test]
fn external_and_sampler_activity_are_separate_from_runtime_wakeups() {
    let counters = IdleEfficiencyCounters::default();
    counters.record_excluded_sampler_wakeup();
    counters.record_excluded_operating_system_wakeup();

    let snapshot = counters.snapshot();
    assert_eq!(snapshot.main_loop_wakeups, 0);
    assert_eq!(snapshot.compositor_loop_wakeups, 0);
    assert_eq!(snapshot.excluded_sampler_wakeups, 1);
    assert_eq!(snapshot.excluded_operating_system_wakeups, 1);
}
