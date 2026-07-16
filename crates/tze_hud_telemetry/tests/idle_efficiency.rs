use std::collections::BTreeMap;

use tze_hud_telemetry::{
    ConstrainedProfileIdentity, EfficiencyGpuCounters, EfficiencyPacingIdentity,
    EfficiencyPacingMode, EfficiencyRendererIdentity, EfficiencyRuntimeIdentity,
    EfficiencyScenarioIdentity, EfficiencyViewport, EfficiencyWakeupCounters, EfficiencyWindowMode,
    QuiescentEfficiencyArtifact, QuiescentMeasurementStatus,
};

fn valid_artifact() -> QuiescentEfficiencyArtifact {
    QuiescentEfficiencyArtifact {
        schema_version: 1,
        scenario: EfficiencyScenarioIdentity {
            name: "quiescent_static_scene".into(),
            version: 1,
        },
        runtime: EfficiencyRuntimeIdentity {
            build: "test-build".into(),
            window_mode: EfficiencyWindowMode::Headless,
        },
        pacing: EfficiencyPacingIdentity {
            mode: EfficiencyPacingMode::EventDriven,
            requested_cadence_hz: None,
        },
        renderer: EfficiencyRendererIdentity {
            backend: "vulkan".into(),
            adapter: "llvmpipe".into(),
            software: true,
        },
        viewport: EfficiencyViewport {
            width: 640,
            height: 360,
        },
        constrained_profile: Some(ConstrainedProfileIdentity {
            operating_system: "linux".into(),
            cpu_model: "test-cpu".into(),
            logical_cpu_limit: 2,
            cpu_limit_enforcement: "taskset:0,1".into(),
            memory_limit_bytes: None,
        }),
        settling_duration_ms: 5_000,
        interval_duration_ms: 60_000,
        status: QuiescentMeasurementStatus::Complete,
        wakeups: EfficiencyWakeupCounters {
            combined_runtime_driven: 2,
            main_loop: 1,
            compositor_loop: 1,
            sources: BTreeMap::from([
                ("main.operator_capture".into(), 1),
                ("compositor.shutdown".into(), 1),
            ]),
            excluded_sampler: 60,
            excluded_operating_system: 0,
        },
        gpu: EfficiencyGpuCounters {
            queue_submissions: 0,
            surface_acquisitions: 0,
            presents: 0,
        },
    }
}

#[test]
fn complete_constrained_quiescent_artifact_passes_exact_budgets() {
    let report = valid_artifact().validate(true);
    assert!(report.passed, "{report:#?}");
    assert_eq!(report.combined_runtime_wakeups.actual, 2);
    assert_eq!(report.combined_runtime_wakeups.ceiling, 120);
    assert_eq!(report.gpu_queue_submissions.actual, 0);
    assert_eq!(report.gpu_queue_submissions.ceiling, 0);
}

#[test]
fn fixed_cadence_cannot_masquerade_as_quiescent_evidence() {
    let mut artifact = valid_artifact();
    artifact.pacing.mode = EfficiencyPacingMode::FixedCadence;
    artifact.pacing.requested_cadence_hz = Some(60);

    let report = artifact.validate(true);
    assert!(!report.passed);
    assert!(
        report
            .violations
            .iter()
            .any(|message| message.contains("event-driven")),
        "{report:#?}"
    );
}

#[test]
fn missing_required_counter_fails_deserialization_instead_of_defaulting_to_zero() {
    let mut value = serde_json::to_value(valid_artifact()).unwrap();
    value["gpu"].as_object_mut().unwrap().remove("presents");

    let error = serde_json::from_value::<QuiescentEfficiencyArtifact>(value).unwrap_err();
    assert!(error.to_string().contains("presents"), "{error}");
}

#[test]
fn missing_constrained_identity_fails_closed_when_lane_requires_it() {
    let mut artifact = valid_artifact();
    artifact.constrained_profile = None;

    let report = artifact.validate(true);
    assert!(!report.passed);
    assert!(
        report
            .violations
            .iter()
            .any(|message| message.contains("constrained_profile")),
        "{report:#?}"
    );
}

#[test]
fn any_idle_gpu_work_fails_the_zero_budget() {
    let mut artifact = valid_artifact();
    artifact.gpu.queue_submissions = 1;

    let report = artifact.validate(true);
    assert!(!report.passed);
    assert_eq!(report.gpu_queue_submissions.actual, 1);
    assert!(
        report
            .violations
            .iter()
            .any(|message| message.contains("queue submissions")),
        "{report:#?}"
    );
}
