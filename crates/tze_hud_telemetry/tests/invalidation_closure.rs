use tze_hud_telemetry::{
    ActualWorkItem, ChangeEfficiencyArtifact, ChangeEfficiencyValidationStatus,
    ChangeMeasurementProvenance, ChangeMeasurementStatus, ChangeRenderWorkObservation,
    ClosureWorkItem, ConstrainedProfileIdentity, DamageCategory, DamageWorkItemId,
    EfficiencyPacingIdentity, EfficiencyPacingMode, EfficiencyRendererIdentity,
    EfficiencyRuntimeIdentity, EfficiencyScenarioIdentity, EfficiencyViewport,
    FullSurfaceInvalidation, FullSurfaceInvalidationReason, InvalidationCategory,
    InvalidationClosure, InvalidationDependencyReason, NodeWorkItemId, PartialPresentCapability,
    PixelRect, RenderPlanWorkItemId, TextureUploadActualWork, TextureUploadCategory,
    TextureUploadWorkItemId,
};

fn node(tile: &str, node: &str) -> NodeWorkItemId {
    NodeWorkItemId {
        tile_id: tile.into(),
        node_id: node.into(),
    }
}

fn damage(tile: &str, region: &str, bounds: PixelRect) -> DamageWorkItemId {
    DamageWorkItemId {
        tile_id: tile.into(),
        region_id: region.into(),
        bounds,
    }
}

fn valid_one_node_artifact() -> ChangeEfficiencyArtifact {
    let changed_node = node("tile-0", "node-0");
    let changed_region = damage(
        "tile-0",
        "tile-0-bounds",
        PixelRect {
            x: 0,
            y: 0,
            width: 100,
            height: 80,
        },
    );

    ChangeEfficiencyArtifact {
        schema_version: 2,
        scenario: EfficiencyScenarioIdentity {
            name: "one_node_change_50_tiles".into(),
            version: 1,
        },
        runtime: EfficiencyRuntimeIdentity {
            build: "test-build".into(),
            window_mode: tze_hud_telemetry::EfficiencyWindowMode::Headless,
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
            width: 1_000,
            height: 500,
        },
        constrained_profile: Some(ConstrainedProfileIdentity {
            operating_system: "linux".into(),
            cpu_model: "test-cpu".into(),
            logical_cpu_limit: 2,
            cpu_limit_enforcement: "taskset:0,1".into(),
            memory_limit_bytes: None,
        }),
        settling_duration_ms: 0,
        interval_duration_ms: 1,
        status: ChangeMeasurementStatus::Complete,
        measurement_provenance: ChangeMeasurementProvenance::Fixture,
        scene_tile_count: 50,
        closure: InvalidationClosure {
            layout: InvalidationCategory {
                closure_items: vec![ClosureWorkItem {
                    identity: changed_node.clone(),
                    dependency_reason: InvalidationDependencyReason::DirectChange,
                }],
                actual_work: vec![ActualWorkItem {
                    identity: changed_node.clone(),
                    operations: 1,
                }],
            },
            raster: InvalidationCategory {
                closure_items: vec![ClosureWorkItem {
                    identity: changed_node.clone(),
                    dependency_reason: InvalidationDependencyReason::DirectChange,
                }],
                actual_work: vec![ActualWorkItem {
                    identity: changed_node,
                    operations: 1,
                }],
            },
            texture_upload: TextureUploadCategory {
                closure_items: vec![],
                actual_work: vec![],
            },
            render_encoding: InvalidationCategory {
                closure_items: vec![ClosureWorkItem {
                    identity: RenderPlanWorkItemId {
                        tile_id: "tile-0".into(),
                        plan_id: "tile-0-main".into(),
                    },
                    dependency_reason: InvalidationDependencyReason::DirectChange,
                }],
                actual_work: vec![ActualWorkItem {
                    identity: RenderPlanWorkItemId {
                        tile_id: "tile-0".into(),
                        plan_id: "tile-0-main".into(),
                    },
                    operations: 1,
                }],
            },
            composition_damage: DamageCategory {
                closure_items: vec![ClosureWorkItem {
                    identity: changed_region.clone(),
                    dependency_reason: InvalidationDependencyReason::DirectChange,
                }],
                actual_work: vec![ActualWorkItem {
                    identity: changed_region,
                    operations: 1,
                }],
            },
        },
        render_observation: ChangeRenderWorkObservation {
            full_surface_clear_operations: 0,
            full_frame_encode_operations: 0,
            scoped_render_encode_operations: 1,
        },
        encoded_draw_calls: 1,
        full_surface_invalidation: None,
    }
}

#[test]
fn canonical_one_node_change_in_fifty_tiles_is_non_certifying_without_runtime_accounting() {
    let report = valid_one_node_artifact().validate();

    assert!(!report.passed, "{report:#?}");
    assert!(report.contract_satisfied, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::PendingRuntimeInstrumentation,
        "{report:#?}"
    );
    assert_eq!(report.layout.closure_cardinality, 1);
    assert_eq!(report.raster.actual_operation_count, 1);
    assert_eq!(report.texture_upload.category.actual_operation_count, 0);
    assert_eq!(report.texture_upload.uploaded_byte_count, 0);
    assert_eq!(report.render_encoding.actual_operation_count, 1);
    assert_eq!(report.composition_damage.damaged_pixel_area, 8_000);
}

#[test]
fn full_frame_encoder_work_cannot_hide_inside_a_scoped_capture() {
    let mut artifact = valid_one_node_artifact();
    artifact.measurement_provenance = ChangeMeasurementProvenance::ObservedRetainedRuntime;
    artifact.render_observation.full_surface_clear_operations = 1;
    artifact.render_observation.full_frame_encode_operations = 1;

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::InvalidArtifact,
        "{report:#?}"
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("full-surface clear")),
        "{report:#?}"
    );
}

#[test]
fn observed_retained_runtime_provenance_round_trips_as_required_schema_metadata() {
    let mut value = serde_json::to_value(valid_one_node_artifact()).unwrap();
    value["measurement_provenance"] = serde_json::json!("observed_retained_runtime");
    let artifact: ChangeEfficiencyArtifact = serde_json::from_value(value).unwrap();

    assert_eq!(
        artifact.measurement_provenance,
        ChangeMeasurementProvenance::ObservedRetainedRuntime
    );
    let report = artifact.validate();
    assert!(!report.passed, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::PendingRuntimeInstrumentation,
        "raw artifact metadata must not impersonate an opaque compositor capture: {report:#?}"
    );
}

#[test]
fn unrelated_actual_work_fails_the_canonical_fifty_tile_gate() {
    let mut artifact = valid_one_node_artifact();
    artifact.closure.raster.actual_work.push(ActualWorkItem {
        identity: node("tile-49", "node-49"),
        operations: 1,
    });

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("raster actual work item")),
        "{report:#?}"
    );
}

#[test]
fn repeated_actual_processing_exceeding_the_closure_fails() {
    let mut artifact = valid_one_node_artifact();
    artifact.closure.render_encoding.actual_work[0].operations = 2;

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("render_encoding actual operations")),
        "{report:#?}"
    );
}

#[test]
fn dependency_expansion_requires_a_typed_reason_and_remains_bounded() {
    let mut artifact = valid_one_node_artifact();
    artifact.scenario.name = "dependency_expansion".into();
    artifact.scene_tile_count = 2;
    let parent = node("tile-0", "parent-layout");
    artifact.closure.layout.closure_items.push(ClosureWorkItem {
        identity: parent.clone(),
        dependency_reason: InvalidationDependencyReason::ParentLayout,
    });
    artifact.closure.layout.actual_work.push(ActualWorkItem {
        identity: parent,
        operations: 1,
    });

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert!(report.contract_satisfied, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::PendingRuntimeInstrumentation,
        "{report:#?}"
    );
    assert_eq!(report.layout.closure_cardinality, 2);
}

#[test]
fn texture_upload_report_includes_uploaded_bytes() {
    let mut artifact = valid_one_node_artifact();
    artifact.scenario.name = "texture_upload_bytes".into();
    artifact.scene_tile_count = 1;
    let texture = TextureUploadWorkItemId {
        tile_id: "tile-0".into(),
        resource_id: "glyph-atlas-0".into(),
    };
    artifact.closure.texture_upload.closure_items = vec![ClosureWorkItem {
        identity: texture.clone(),
        dependency_reason: InvalidationDependencyReason::ResourceDependency,
    }];
    artifact.closure.texture_upload.actual_work = vec![TextureUploadActualWork {
        identity: texture,
        operations: 1,
        uploaded_bytes: 512,
    }];

    let report = artifact.validate();

    assert!(report.contract_satisfied, "{report:#?}");
    assert_eq!(report.texture_upload.category.actual_operation_count, 1);
    assert_eq!(report.texture_upload.uploaded_byte_count, 512);
}

#[test]
fn canonical_changed_tile_resource_upload_is_contract_valid() {
    let mut artifact = valid_one_node_artifact();
    let texture = TextureUploadWorkItemId {
        tile_id: "tile-0".into(),
        resource_id: "glyph-atlas-0".into(),
    };
    artifact.closure.texture_upload.closure_items = vec![ClosureWorkItem {
        identity: texture.clone(),
        dependency_reason: InvalidationDependencyReason::ResourceDependency,
    }];
    artifact.closure.texture_upload.actual_work = vec![TextureUploadActualWork {
        identity: texture,
        operations: 1,
        uploaded_bytes: 512,
    }];

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert!(report.contract_satisfied, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::PendingRuntimeInstrumentation,
        "{report:#?}"
    );
}

#[test]
fn blank_typed_identity_components_fail_closed() {
    let mut artifact = valid_one_node_artifact();
    artifact.scenario.name = "identity_validation".into();
    artifact.scene_tile_count = 1;

    artifact.closure.layout.closure_items[0]
        .identity
        .tile_id
        .clear();
    artifact.closure.layout.closure_items[0]
        .identity
        .node_id
        .clear();
    artifact.closure.layout.actual_work[0]
        .identity
        .tile_id
        .clear();
    artifact.closure.layout.actual_work[0]
        .identity
        .node_id
        .clear();
    artifact.closure.raster.closure_items[0]
        .identity
        .tile_id
        .clear();
    artifact.closure.raster.closure_items[0]
        .identity
        .node_id
        .clear();
    artifact.closure.raster.actual_work[0]
        .identity
        .tile_id
        .clear();
    artifact.closure.raster.actual_work[0]
        .identity
        .node_id
        .clear();
    artifact.closure.render_encoding.closure_items[0]
        .identity
        .tile_id
        .clear();
    artifact.closure.render_encoding.closure_items[0]
        .identity
        .plan_id
        .clear();
    artifact.closure.render_encoding.actual_work[0]
        .identity
        .tile_id
        .clear();
    artifact.closure.render_encoding.actual_work[0]
        .identity
        .plan_id
        .clear();
    artifact.closure.composition_damage.closure_items[0]
        .identity
        .tile_id
        .clear();
    artifact.closure.composition_damage.closure_items[0]
        .identity
        .region_id
        .clear();
    artifact.closure.composition_damage.actual_work[0]
        .identity
        .tile_id
        .clear();
    artifact.closure.composition_damage.actual_work[0]
        .identity
        .region_id
        .clear();

    let texture = TextureUploadWorkItemId {
        tile_id: "".into(),
        resource_id: "".into(),
    };
    artifact.closure.texture_upload.closure_items = vec![ClosureWorkItem {
        identity: texture.clone(),
        dependency_reason: InvalidationDependencyReason::ResourceDependency,
    }];
    artifact.closure.texture_upload.actual_work = vec![TextureUploadActualWork {
        identity: texture,
        operations: 1,
        uploaded_bytes: 1,
    }];

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert!(!report.contract_satisfied, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::InvalidArtifact,
        "{report:#?}"
    );
    for component in ["tile_id", "node_id", "resource_id", "plan_id", "region_id"] {
        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation.contains(component)),
            "missing {component} violation: {report:#?}"
        );
    }
}

#[test]
fn full_surface_fallback_is_diagnostic_not_a_proportional_pass() {
    let mut artifact = valid_one_node_artifact();
    let full_surface = damage(
        "runtime",
        "surface",
        PixelRect {
            x: 0,
            y: 0,
            width: 1_000,
            height: 500,
        },
    );
    artifact.closure.composition_damage.closure_items = vec![ClosureWorkItem {
        identity: full_surface.clone(),
        dependency_reason: InvalidationDependencyReason::RuntimeSurface,
    }];
    artifact.closure.composition_damage.actual_work = vec![ActualWorkItem {
        identity: full_surface,
        operations: 1,
    }];
    artifact.full_surface_invalidation = Some(FullSurfaceInvalidation {
        reason: FullSurfaceInvalidationReason::Resize,
        partial_present_capability: PartialPresentCapability::Supported,
    });

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::DiagnosticFullSurface,
        "{report:#?}"
    );
}

#[test]
fn full_surface_fallback_can_exceed_the_normal_damage_closure() {
    let mut artifact = valid_one_node_artifact();
    artifact.closure.composition_damage.actual_work = vec![ActualWorkItem {
        identity: damage(
            "runtime",
            "surface",
            PixelRect {
                x: 0,
                y: 0,
                width: 1_000,
                height: 500,
            },
        ),
        operations: 1,
    }];
    artifact.full_surface_invalidation = Some(FullSurfaceInvalidation {
        reason: FullSurfaceInvalidationReason::DeviceRecovery,
        partial_present_capability: PartialPresentCapability::Supported,
    });

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::DiagnosticFullSurface,
        "{report:#?}"
    );
    assert!(report.violations.is_empty(), "{report:#?}");
}

#[test]
fn full_surface_damage_without_metadata_fails_closed() {
    let mut artifact = valid_one_node_artifact();
    artifact.closure.composition_damage.actual_work = vec![ActualWorkItem {
        identity: damage(
            "runtime",
            "surface",
            PixelRect {
                x: 0,
                y: 0,
                width: 1_000,
                height: 500,
            },
        ),
        operations: 1,
    }];

    let report = artifact.validate();

    assert!(!report.passed, "{report:#?}");
    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::InvalidArtifact,
        "{report:#?}"
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("full_surface_invalidation reason")),
        "{report:#?}"
    );
}

#[test]
fn missing_closure_field_fails_deserialization_instead_of_defaulting_to_zero() {
    let mut value = serde_json::to_value(valid_one_node_artifact()).unwrap();
    value["closure"]
        .as_object_mut()
        .unwrap()
        .remove("render_encoding");

    let error = serde_json::from_value::<ChangeEfficiencyArtifact>(value).unwrap_err();

    assert!(error.to_string().contains("render_encoding"), "{error}");
}

#[test]
fn missing_actual_operation_counter_fails_deserialization_instead_of_defaulting_to_zero() {
    let mut value = serde_json::to_value(valid_one_node_artifact()).unwrap();
    value["closure"]["layout"]["actual_work"][0]
        .as_object_mut()
        .unwrap()
        .remove("operations");

    let error = serde_json::from_value::<ChangeEfficiencyArtifact>(value).unwrap_err();

    assert!(error.to_string().contains("operations"), "{error}");
}

#[test]
fn missing_measurement_interval_fails_deserialization_instead_of_defaulting_to_zero() {
    let mut value = serde_json::to_value(valid_one_node_artifact()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("interval_duration_ms");

    let error = serde_json::from_value::<ChangeEfficiencyArtifact>(value).unwrap_err();

    assert!(
        error.to_string().contains("interval_duration_ms"),
        "{error}"
    );
}

#[test]
fn observed_provenance_and_encoder_observation_are_required_schema_fields() {
    for field in ["measurement_provenance", "render_observation"] {
        let mut value = serde_json::to_value(valid_one_node_artifact()).unwrap();
        value.as_object_mut().unwrap().remove(field);

        let error = serde_json::from_value::<ChangeEfficiencyArtifact>(value).unwrap_err();

        assert!(error.to_string().contains(field), "{field}: {error}");
    }
}

#[test]
fn zero_measurement_interval_fails_the_contract() {
    let mut artifact = valid_one_node_artifact();
    artifact.interval_duration_ms = 0;

    let report = artifact.validate();

    assert_eq!(
        report.status,
        ChangeEfficiencyValidationStatus::InvalidArtifact,
        "{report:#?}"
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("measurement interval")),
        "{report:#?}"
    );
}

#[test]
fn missing_full_surface_reason_fails_deserialization_instead_of_defaulting_to_pass() {
    let mut artifact = valid_one_node_artifact();
    artifact.closure.composition_damage.actual_work = vec![ActualWorkItem {
        identity: damage(
            "runtime",
            "surface",
            PixelRect {
                x: 0,
                y: 0,
                width: 1_000,
                height: 500,
            },
        ),
        operations: 1,
    }];
    artifact.full_surface_invalidation = Some(FullSurfaceInvalidation {
        reason: FullSurfaceInvalidationReason::SurfaceCreation,
        partial_present_capability: PartialPresentCapability::Supported,
    });
    let mut value = serde_json::to_value(artifact).unwrap();
    value["full_surface_invalidation"]
        .as_object_mut()
        .unwrap()
        .remove("reason");

    let error = serde_json::from_value::<ChangeEfficiencyArtifact>(value).unwrap_err();

    assert!(error.to_string().contains("reason"), "{error}");
}
