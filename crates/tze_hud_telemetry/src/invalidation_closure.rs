//! Typed, fail-closed accounting for change-proportional render work.
//!
//! The normal per-frame telemetry stream is intentionally not used as the
//! source of truth here: an efficiency gate must distinguish the work eligible
//! for one committed change from every operation actually performed on its
//! behalf.  This module provides the versioned artifact and pure validator used
//! by Layer 3 evidence producers. Until a retained/delta renderer emits a real
//! scoped capture, this contract deliberately cannot certify a proportional
//! render path from a hand-built fixture.

use std::collections::BTreeSet;
use std::fmt::Debug;

use serde::{Deserialize, Serialize};

use crate::idle_efficiency::{
    ConstrainedProfileIdentity, EfficiencyPacingIdentity, EfficiencyRendererIdentity,
    EfficiencyRuntimeIdentity, EfficiencyScenarioIdentity, EfficiencyViewport,
};

/// Version of the change-proportional efficiency artifact schema.
pub const CHANGE_EFFICIENCY_SCHEMA_VERSION: u32 = 2;
/// Stable name of the canonical one-node, fifty-tile scenario.
pub const ONE_NODE_FIFTY_TILE_SCENARIO_NAME: &str = "one_node_change_50_tiles";
/// Version of the canonical one-node, fifty-tile scenario.
pub const ONE_NODE_FIFTY_TILE_SCENARIO_VERSION: u32 = 1;
/// Stable name of the retained transparent-overlap, fifty-tile scenario.
pub const TRANSPARENT_OVERLAP_FIFTY_TILE_SCENARIO_NAME: &str =
    "transparent_overlap_change_50_tiles";
/// Version of the retained transparent-overlap, fifty-tile scenario.
pub const TRANSPARENT_OVERLAP_FIFTY_TILE_SCENARIO_VERSION: u32 = 1;

/// Completion state declared by a change-efficiency measurement producer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeMeasurementStatus {
    /// The producer completed every required observation for the change.
    Complete,
    /// The producer could not complete a valid observation.
    Invalid,
}

/// Origin of the operation evidence carried by a change-efficiency artifact.
///
/// A schema fixture may exercise the validator, but it must never certify an
/// incremental renderer.  Certification is reserved for a compositor capture
/// taken after a retained render plan has actually encoded and submitted its
/// scoped GPU work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeMeasurementProvenance {
    /// Hand-built or test-fixture evidence; useful for contract coverage only.
    Fixture,
    /// Work captured from the retained headless compositor path.
    ObservedRetainedRuntime,
    /// A real full-frame runtime fallback captured as a non-passing diagnostic.
    ObservedFullFrameRuntime,
}

/// Why a closure member legitimately expands beyond the directly changed item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidationDependencyReason {
    /// The member is directly changed by the committed presentation change.
    DirectChange,
    /// A parent or sibling layout dependency changes its output.
    ParentLayout,
    /// Transparent or otherwise visually overlapping output changes.
    VisualOverlap,
    /// Runtime-owned chrome changes its output in response to the change.
    RuntimeChrome,
    /// A texture or other resource must be refreshed for the changed output.
    ResourceDependency,
    /// The runtime-owned surface state itself changed.
    RuntimeSurface,
}

/// Integer pixel rectangle used for closure and actual damage regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PixelRect {
    /// Left coordinate in the presentation viewport.
    pub x: u32,
    /// Top coordinate in the presentation viewport.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl PixelRect {
    fn right(self) -> Option<u32> {
        self.x.checked_add(self.width)
    }

    fn bottom(self) -> Option<u32> {
        self.y.checked_add(self.height)
    }

    fn is_non_empty(self) -> bool {
        self.width > 0 && self.height > 0 && self.right().is_some() && self.bottom().is_some()
    }

    fn contains(self, other: Self) -> bool {
        match (self.right(), self.bottom(), other.right(), other.bottom()) {
            (Some(right), Some(bottom), Some(other_right), Some(other_bottom)) => {
                self.x <= other.x
                    && self.y <= other.y
                    && right >= other_right
                    && bottom >= other_bottom
            }
            _ => false,
        }
    }
}

/// Typed identity for node-scoped layout and raster work.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeWorkItemId {
    /// Tile that owns the node.
    pub tile_id: String,
    /// Stable node identity within the scene.
    pub node_id: String,
}

/// Typed identity for a texture-upload work item.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TextureUploadWorkItemId {
    /// Tile whose output references the resource.
    pub tile_id: String,
    /// Stable resource identity.
    pub resource_id: String,
}

/// Typed identity for a render-plan item encoded into a frame.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RenderPlanWorkItemId {
    /// Tile whose plan contains this item.
    pub tile_id: String,
    /// Stable plan-item identity.
    pub plan_id: String,
}

/// Typed identity for a compositing-damage region.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DamageWorkItemId {
    /// Tile or runtime surface that owns this region.
    pub tile_id: String,
    /// Stable region identity within that owner.
    pub region_id: String,
    /// Affected output bounds.
    pub bounds: PixelRect,
}

trait WorkItemIdentity {
    fn validate_components(
        &self,
        category_name: &str,
        evidence_kind: &str,
        violations: &mut Vec<String>,
    );
}

fn validate_required_component(
    value: &str,
    component_name: &str,
    category_name: &str,
    evidence_kind: &str,
    violations: &mut Vec<String>,
) {
    if value.trim().is_empty() {
        violations.push(format!(
            "{category_name} {evidence_kind} identity {component_name} must be non-empty"
        ));
    }
}

impl WorkItemIdentity for NodeWorkItemId {
    fn validate_components(
        &self,
        category_name: &str,
        evidence_kind: &str,
        violations: &mut Vec<String>,
    ) {
        validate_required_component(
            &self.tile_id,
            "tile_id",
            category_name,
            evidence_kind,
            violations,
        );
        validate_required_component(
            &self.node_id,
            "node_id",
            category_name,
            evidence_kind,
            violations,
        );
    }
}

impl WorkItemIdentity for TextureUploadWorkItemId {
    fn validate_components(
        &self,
        category_name: &str,
        evidence_kind: &str,
        violations: &mut Vec<String>,
    ) {
        validate_required_component(
            &self.tile_id,
            "tile_id",
            category_name,
            evidence_kind,
            violations,
        );
        validate_required_component(
            &self.resource_id,
            "resource_id",
            category_name,
            evidence_kind,
            violations,
        );
    }
}

impl WorkItemIdentity for RenderPlanWorkItemId {
    fn validate_components(
        &self,
        category_name: &str,
        evidence_kind: &str,
        violations: &mut Vec<String>,
    ) {
        validate_required_component(
            &self.tile_id,
            "tile_id",
            category_name,
            evidence_kind,
            violations,
        );
        validate_required_component(
            &self.plan_id,
            "plan_id",
            category_name,
            evidence_kind,
            violations,
        );
    }
}

impl WorkItemIdentity for DamageWorkItemId {
    fn validate_components(
        &self,
        category_name: &str,
        evidence_kind: &str,
        violations: &mut Vec<String>,
    ) {
        validate_required_component(
            &self.tile_id,
            "tile_id",
            category_name,
            evidence_kind,
            violations,
        );
        validate_required_component(
            &self.region_id,
            "region_id",
            category_name,
            evidence_kind,
            violations,
        );
    }
}

/// One eligible member of an invalidation closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosureWorkItem<T> {
    /// Typed work-item identity.
    pub identity: T,
    /// Explicit justification for including this member in the closure.
    pub dependency_reason: InvalidationDependencyReason,
}

/// Actual operations performed for one typed work item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActualWorkItem<T> {
    /// Typed work-item identity.
    pub identity: T,
    /// Number of operations, including repeated processing of the same item.
    pub operations: u64,
}

/// Closure membership and actual operation evidence for one work category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationCategory<T> {
    /// Unique eligible work items in the closure.
    pub closure_items: Vec<ClosureWorkItem<T>>,
    /// Every actual operation performed for the category.
    pub actual_work: Vec<ActualWorkItem<T>>,
}

/// Texture-upload actual work, including the uploaded byte count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureUploadActualWork {
    /// Typed texture-upload identity.
    pub identity: TextureUploadWorkItemId,
    /// Number of upload operations for this resource.
    pub operations: u64,
    /// Total bytes uploaded by those operations.
    pub uploaded_bytes: u64,
}

/// Texture-upload closure evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureUploadCategory {
    /// Unique eligible texture-upload work items.
    pub closure_items: Vec<ClosureWorkItem<TextureUploadWorkItemId>>,
    /// Actual texture-upload operations and bytes.
    pub actual_work: Vec<TextureUploadActualWork>,
}

/// Alias for the typed composition-damage category.
pub type DamageCategory = InvalidationCategory<DamageWorkItemId>;

/// All typed work categories for one presentation-relevant change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationClosure {
    /// Layout-resolution work keyed by node identity.
    pub layout: InvalidationCategory<NodeWorkItemId>,
    /// Text/image raster work keyed by node identity.
    pub raster: InvalidationCategory<NodeWorkItemId>,
    /// Texture uploads keyed by tile and resource identity.
    pub texture_upload: TextureUploadCategory,
    /// Render-encoding work keyed by tile and plan-item identity.
    pub render_encoding: InvalidationCategory<RenderPlanWorkItemId>,
    /// Composition-damage work keyed by output region identity.
    pub composition_damage: DamageCategory,
}

/// Encoder-side observations that distinguish a retained partial update from a
/// full-frame fallback.
///
/// These counters are deliberately separate from the typed closure.  The
/// closure answers *which* work was eligible; this observation records whether
/// the renderer nonetheless cleared or encoded the whole surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRenderWorkObservation {
    /// Number of full-surface clear operations encoded for this change.
    pub full_surface_clear_operations: u64,
    /// Number of full-frame encodes performed for this change.
    pub full_frame_encode_operations: u64,
    /// Number of closure-scoped render encodes performed for this change.
    pub scoped_render_encode_operations: u64,
}

/// Structured reason for a full-surface invalidation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullSurfaceInvalidationReason {
    /// The surface was created and has no prior contents to preserve.
    SurfaceCreation,
    /// The presentation surface changed dimensions.
    Resize,
    /// GPU device or surface recovery invalidated retained contents.
    DeviceRecovery,
    /// The active backend cannot make the required partial-present guarantee.
    UnsupportedPartialPresentBackend,
    /// The retained planner cannot establish the bounded closure required for
    /// this changed scene, so it deliberately falls back to a full frame.
    UnsupportedRetainedSceneChange,
}

/// Whether the renderer/backend reports partial-present support for this event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialPresentCapability {
    /// The backend can preserve unaffected output regions.
    Supported,
    /// The backend cannot preserve unaffected output regions for this event.
    Unsupported,
}

/// Required metadata attached to every full-surface diagnostic event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullSurfaceInvalidation {
    /// Structured cause of the full-surface invalidation.
    pub reason: FullSurfaceInvalidationReason,
    /// Capability reported by the backend for this event.
    pub partial_present_capability: PartialPresentCapability,
}

/// Versioned evidence for one committed presentation-relevant change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEfficiencyArtifact {
    /// Schema version for fail-closed compatibility checks.
    pub schema_version: u32,
    /// Stable scenario identity and version.
    pub scenario: EfficiencyScenarioIdentity,
    /// Runtime build and window-mode identity.
    pub runtime: EfficiencyRuntimeIdentity,
    /// Pacing identity for the observed frame.
    pub pacing: EfficiencyPacingIdentity,
    /// Renderer/backend identity.
    pub renderer: EfficiencyRendererIdentity,
    /// Presentation viewport.
    pub viewport: EfficiencyViewport,
    /// Constrained-runner identity when the artifact comes from that lane.
    pub constrained_profile: Option<ConstrainedProfileIdentity>,
    /// Time allowed for prior frame activity to settle before this change was measured.
    pub settling_duration_ms: u64,
    /// Measurement interval covering the committed change and its resulting work.
    pub interval_duration_ms: u64,
    /// Whether the measurement completed all required observations.
    pub status: ChangeMeasurementStatus,
    /// Whether this is fixture data or a completed retained-runtime capture.
    pub measurement_provenance: ChangeMeasurementProvenance,
    /// Number of visible scene tiles before the changed work was rendered.
    pub scene_tile_count: u32,
    /// Typed closure membership and actual-operation evidence.
    pub closure: InvalidationClosure,
    /// Observed encoder work, kept distinct from closure eligibility.
    pub render_observation: ChangeRenderWorkObservation,
    /// Draw calls encoded for this change; not a substitute for encode work.
    pub encoded_draw_calls: u64,
    /// Required only when the damaged region covers the whole viewport.
    pub full_surface_invalidation: Option<FullSurfaceInvalidation>,
}

/// Exact rational representation of work amplification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmplificationRatio {
    /// Actual operations in the category.
    pub numerator: u64,
    /// Unique eligible closure work items in the category.
    pub denominator: u64,
}

/// Derived counts and amplification for one typed work category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEfficiencyCategoryReport {
    /// Number of unique eligible closure work-item identities.
    pub closure_cardinality: u64,
    /// Every operation actually performed, including repeats.
    pub actual_operation_count: u64,
    /// Exact structured ratio of actual operations to closure cardinality.
    pub amplification: AmplificationRatio,
}

/// Derived texture-upload counts and uploaded-byte total for one change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEfficiencyTextureUploadReport {
    /// Closure cardinality, actual operations, and operation amplification.
    pub category: ChangeEfficiencyCategoryReport,
    /// Total bytes uploaded by the actual texture-upload operations.
    pub uploaded_byte_count: u64,
}

/// Derived composition-damage evidence for the artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEfficiencyDamageReport {
    /// Closure and actual-operation counts for damage regions.
    pub category: ChangeEfficiencyCategoryReport,
    /// Union area of eligible affected output bounds.
    pub closure_affected_pixel_area: u64,
    /// Union area actually damaged by the change.
    pub damaged_pixel_area: u64,
    /// Total viewport area, retained so area amplification is auditable.
    pub viewport_pixel_area: u64,
    /// Exact structured ratio of damaged to eligible pixel area.
    pub amplification: AmplificationRatio,
}

/// Final gate classification for one change-efficiency artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeEfficiencyValidationStatus {
    /// A required field or invariant is invalid, missing, or inconsistent.
    InvalidArtifact,
    /// Full-surface fallback was structured but cannot satisfy the ordinary gate.
    DiagnosticFullSurface,
    /// The artifact satisfies the schema contract but lacks a real scoped runtime
    /// producer, so it cannot certify change-proportional rendering yet.
    PendingRuntimeInstrumentation,
    /// An opaque retained compositor capture proved scoped work and all closure
    /// invariants held. Raw artifact JSON never reaches this status by itself.
    CertifiedRetainedRuntime,
}

/// Deterministic, machine-readable result of closure validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEfficiencyValidation {
    /// Whether this artifact certifies the ordinary closure-scoped gate.
    ///
    /// This remains false until retained rendering supplies a real scoped
    /// producer. A schema fixture must never become a substitute for that proof.
    pub passed: bool,
    /// Whether all schema-level closure and damage invariants were satisfied.
    ///
    /// This is intentionally separate from [`Self::passed`], so a valid fixture
    /// remains visible without being mistaken for production evidence.
    pub contract_satisfied: bool,
    /// Status that distinguishes invalid evidence from an explicit diagnostic fallback.
    pub status: ChangeEfficiencyValidationStatus,
    /// Layout work evidence.
    pub layout: ChangeEfficiencyCategoryReport,
    /// Raster work evidence.
    pub raster: ChangeEfficiencyCategoryReport,
    /// Texture-upload work evidence.
    pub texture_upload: ChangeEfficiencyTextureUploadReport,
    /// Render-encoding work evidence.
    pub render_encoding: ChangeEfficiencyCategoryReport,
    /// Composition-damage evidence.
    pub composition_damage: ChangeEfficiencyDamageReport,
    /// Draw calls reported independently from render-encoding operations.
    pub encoded_draw_calls: u64,
    /// Actionable invariant violations.
    pub violations: Vec<String>,
}

impl ChangeEfficiencyArtifact {
    /// Validate the fail-closed invalidation-closure and damage contracts.
    ///
    /// Artifact data validates the contract but remains non-certifying, even
    /// when deserialized data claims retained-runtime provenance. The compositor
    /// owns the opaque capture that can upgrade a valid observed artifact to a
    /// certification after the actual retained render returns.
    pub fn validate(&self) -> ChangeEfficiencyValidation {
        let mut violations = Vec::new();

        self.validate_identity(&mut violations);

        let layout = validate_category("layout", &self.closure.layout, false, &mut violations);
        let raster = validate_category("raster", &self.closure.raster, false, &mut violations);
        let texture_upload = validate_texture_upload(&self.closure.texture_upload, &mut violations);
        let render_encoding = validate_category(
            "render_encoding",
            &self.closure.render_encoding,
            false,
            &mut violations,
        );
        let composition_damage = validate_damage(
            &self.closure.composition_damage,
            self.viewport.clone(),
            self.full_surface_invalidation.is_some(),
            &mut violations,
        );

        let full_surface = composition_damage.damaged_pixel_area
            == composition_damage.viewport_pixel_area
            && composition_damage.viewport_pixel_area > 0;
        self.validate_full_surface(full_surface, &mut violations);
        self.validate_render_observation(&render_encoding, full_surface, &mut violations);

        if !full_surface {
            self.validate_canonical_one_node_scenario(
                &layout,
                &raster,
                &texture_upload,
                &render_encoding,
                &composition_damage,
                &mut violations,
            );
            self.validate_transparent_overlap_scenario(
                &layout,
                &raster,
                &texture_upload,
                &render_encoding,
                &composition_damage,
                &mut violations,
            );
        }

        let contract_satisfied = violations.is_empty() && !full_surface;
        let status = if !violations.is_empty() {
            ChangeEfficiencyValidationStatus::InvalidArtifact
        } else if full_surface {
            ChangeEfficiencyValidationStatus::DiagnosticFullSurface
        } else {
            ChangeEfficiencyValidationStatus::PendingRuntimeInstrumentation
        };
        let passed = false;

        ChangeEfficiencyValidation {
            passed,
            contract_satisfied,
            status,
            layout,
            raster,
            texture_upload,
            render_encoding,
            composition_damage,
            encoded_draw_calls: self.encoded_draw_calls,
            violations,
        }
    }

    fn validate_identity(&self, violations: &mut Vec<String>) {
        if self.schema_version != CHANGE_EFFICIENCY_SCHEMA_VERSION {
            violations.push(format!(
                "schema_version must be {CHANGE_EFFICIENCY_SCHEMA_VERSION}, got {}",
                self.schema_version
            ));
        }
        if self.scenario.name.trim().is_empty() || self.scenario.version == 0 {
            violations.push("scenario name must be non-empty and version must be non-zero".into());
        }
        if self.runtime.build.trim().is_empty() {
            violations.push("runtime build identity must be non-empty".into());
        }
        if self.renderer.backend.trim().is_empty() || self.renderer.adapter.trim().is_empty() {
            violations.push("renderer backend and adapter identities must be non-empty".into());
        }
        if self.viewport.width == 0 || self.viewport.height == 0 {
            violations.push("viewport dimensions must be non-zero".into());
        }
        if self.interval_duration_ms == 0 {
            violations.push("change measurement interval must be non-zero".into());
        }
        if self.status != ChangeMeasurementStatus::Complete {
            violations.push("change measurement status must be complete".into());
        }
        if self.scene_tile_count == 0 {
            violations.push("scene_tile_count must be non-zero".into());
        }
    }

    fn validate_full_surface(&self, full_surface: bool, violations: &mut Vec<String>) {
        match (full_surface, &self.full_surface_invalidation) {
            (true, None) => violations.push(
                "full-surface damage requires a structured full_surface_invalidation reason".into(),
            ),
            (false, Some(_)) => violations.push(
                "full_surface_invalidation metadata is invalid without full-surface damage".into(),
            ),
            (true, Some(metadata)) => {
                if metadata.reason
                    == FullSurfaceInvalidationReason::UnsupportedPartialPresentBackend
                    && metadata.partial_present_capability == PartialPresentCapability::Supported
                {
                    violations.push(
                        "unsupported_partial_present_backend requires unsupported capability"
                            .into(),
                    );
                }
            }
            (false, None) => {}
        }
    }

    fn validate_render_observation(
        &self,
        render_encoding: &ChangeEfficiencyCategoryReport,
        full_surface: bool,
        violations: &mut Vec<String>,
    ) {
        let observation = &self.render_observation;
        if !full_surface
            && (observation.full_surface_clear_operations > 0
                || observation.full_frame_encode_operations > 0)
        {
            violations.push(
                "closure-scoped damage cannot include full-surface clear or full-frame encode work"
                    .into(),
            );
        }
        if self.measurement_provenance == ChangeMeasurementProvenance::ObservedRetainedRuntime
            && !full_surface
            && observation.scoped_render_encode_operations != render_encoding.actual_operation_count
        {
            violations.push(format!(
                "observed retained render encodes {} do not equal render-encoding operations {}",
                observation.scoped_render_encode_operations, render_encoding.actual_operation_count
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_canonical_one_node_scenario(
        &self,
        layout: &ChangeEfficiencyCategoryReport,
        raster: &ChangeEfficiencyCategoryReport,
        texture_upload: &ChangeEfficiencyTextureUploadReport,
        render_encoding: &ChangeEfficiencyCategoryReport,
        composition_damage: &ChangeEfficiencyDamageReport,
        violations: &mut Vec<String>,
    ) {
        if self.scenario.name != ONE_NODE_FIFTY_TILE_SCENARIO_NAME
            || self.scenario.version != ONE_NODE_FIFTY_TILE_SCENARIO_VERSION
        {
            return;
        }

        if self.scene_tile_count != 50 {
            violations.push(format!(
                "canonical one-node scenario must contain 50 tiles, got {}",
                self.scene_tile_count
            ));
        }

        let Some(changed_node) = self.closure.layout.closure_items.first() else {
            violations.push("canonical one-node scenario requires a layout closure member".into());
            return;
        };
        if self.closure.layout.closure_items.len() != 1
            || changed_node.dependency_reason != InvalidationDependencyReason::DirectChange
            || layout.actual_operation_count != 1
        {
            violations.push(
                "canonical one-node scenario requires exactly one directly changed layout item"
                    .into(),
            );
        }

        let changed_identity = &changed_node.identity;
        validate_canonical_node_category(
            "raster",
            &self.closure.raster,
            changed_identity,
            raster,
            violations,
        );
        validate_canonical_render_category(
            &self.closure.render_encoding,
            &changed_identity.tile_id,
            render_encoding,
            violations,
        );
        validate_canonical_damage_category(
            &self.closure.composition_damage,
            &changed_identity.tile_id,
            composition_damage,
            violations,
        );

        if self.render_observation.full_surface_clear_operations != 0
            || self.render_observation.full_frame_encode_operations != 0
            || self.render_observation.scoped_render_encode_operations != 1
        {
            violations.push(
                "canonical one-node scenario requires one scoped encode and no full-surface clear or full-frame encode"
                    .into(),
            );
        }

        if texture_upload.category.actual_operation_count > 0
            || texture_upload.category.closure_cardinality > 0
        {
            for item in &self.closure.texture_upload.closure_items {
                if item.identity.tile_id != changed_identity.tile_id
                    || !matches!(
                        item.dependency_reason,
                        InvalidationDependencyReason::DirectChange
                            | InvalidationDependencyReason::ResourceDependency
                    )
                {
                    violations.push(
                        "canonical one-node texture upload closure includes an unrelated item"
                            .into(),
                    );
                }
            }
            if texture_upload.category.actual_operation_count
                > texture_upload.category.closure_cardinality
            {
                violations
                    .push("canonical one-node texture upload operations exceed its closure".into());
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_transparent_overlap_scenario(
        &self,
        layout: &ChangeEfficiencyCategoryReport,
        raster: &ChangeEfficiencyCategoryReport,
        texture_upload: &ChangeEfficiencyTextureUploadReport,
        render_encoding: &ChangeEfficiencyCategoryReport,
        composition_damage: &ChangeEfficiencyDamageReport,
        violations: &mut Vec<String>,
    ) {
        if self.scenario.name != TRANSPARENT_OVERLAP_FIFTY_TILE_SCENARIO_NAME {
            return;
        }
        if self.scenario.version != TRANSPARENT_OVERLAP_FIFTY_TILE_SCENARIO_VERSION {
            violations.push(format!(
                "transparent-overlap scenario version must be {TRANSPARENT_OVERLAP_FIFTY_TILE_SCENARIO_VERSION}, got {}",
                self.scenario.version
            ));
            return;
        }

        if self.scene_tile_count != 50 {
            violations.push(format!(
                "transparent-overlap scenario must contain 50 tiles, got {}",
                self.scene_tile_count
            ));
        }

        let Some(lower) = self.closure.layout.closure_items.first() else {
            violations.push(
                "transparent-overlap scenario requires a directly changed lower layout item".into(),
            );
            return;
        };
        let Some(upper) = self.closure.layout.closure_items.get(1) else {
            violations.push(
                "transparent-overlap scenario requires a visual-overlap upper layout item".into(),
            );
            return;
        };

        if lower.dependency_reason != InvalidationDependencyReason::DirectChange
            || upper.dependency_reason != InvalidationDependencyReason::VisualOverlap
            || lower.identity.tile_id == upper.identity.tile_id
            || self.closure.layout.closure_items.len() != 2
            || !actual_node_work_is_z_ordered(
                &self.closure.layout,
                &lower.identity,
                &upper.identity,
            )
            || layout.actual_operation_count != 2
        {
            violations.push(
                "transparent-overlap scenario requires exactly one DirectChange lower layout item followed by one VisualOverlap upper item"
                    .into(),
            );
        }

        validate_transparent_overlap_node_category(
            "raster",
            &self.closure.raster,
            &lower.identity,
            &upper.identity,
            raster,
            violations,
        );
        validate_transparent_overlap_render_category(
            &self.closure.render_encoding,
            &lower.identity.tile_id,
            &upper.identity.tile_id,
            render_encoding,
            violations,
        );
        validate_transparent_overlap_damage_category(
            &self.closure.composition_damage,
            &lower.identity.tile_id,
            &upper.identity.tile_id,
            composition_damage,
            violations,
        );

        if texture_upload.category.closure_cardinality != 0
            || texture_upload.category.actual_operation_count != 0
            || texture_upload.uploaded_byte_count != 0
        {
            violations.push(
                "transparent-overlap scenario requires zero undeclared texture or glyph upload work"
                    .into(),
            );
        }

        if self.render_observation.full_surface_clear_operations != 0
            || self.render_observation.full_frame_encode_operations != 0
            || self.render_observation.scoped_render_encode_operations != 2
        {
            violations.push(
                "transparent-overlap scenario requires two scoped encodes and no full-surface clear or full-frame encode"
                    .into(),
            );
        }
    }
}

fn actual_node_work_is_z_ordered(
    category: &InvalidationCategory<NodeWorkItemId>,
    lower: &NodeWorkItemId,
    upper: &NodeWorkItemId,
) -> bool {
    category.actual_work.len() == 2
        && category.actual_work[0].identity == *lower
        && category.actual_work[0].operations == 1
        && category.actual_work[1].identity == *upper
        && category.actual_work[1].operations == 1
}

fn validate_transparent_overlap_node_category(
    category_name: &str,
    category: &InvalidationCategory<NodeWorkItemId>,
    lower: &NodeWorkItemId,
    upper: &NodeWorkItemId,
    report: &ChangeEfficiencyCategoryReport,
    violations: &mut Vec<String>,
) {
    if category.closure_items.len() != 2
        || category.closure_items[0].identity != *lower
        || category.closure_items[0].dependency_reason != InvalidationDependencyReason::DirectChange
        || category.closure_items[1].identity != *upper
        || category.closure_items[1].dependency_reason
            != InvalidationDependencyReason::VisualOverlap
        || !actual_node_work_is_z_ordered(category, lower, upper)
        || report.actual_operation_count != 2
    {
        violations.push(format!(
            "transparent-overlap scenario requires z-ordered DirectChange and VisualOverlap {category_name} items"
        ));
    }
}

fn validate_transparent_overlap_render_category(
    category: &InvalidationCategory<RenderPlanWorkItemId>,
    lower_tile_id: &str,
    upper_tile_id: &str,
    report: &ChangeEfficiencyCategoryReport,
    violations: &mut Vec<String>,
) {
    let expected = [
        (lower_tile_id, InvalidationDependencyReason::DirectChange),
        (upper_tile_id, InvalidationDependencyReason::VisualOverlap),
    ];
    let expected_len = expected.len();
    let closure_is_expected = category.closure_items.len() == expected_len
        && category
            .closure_items
            .iter()
            .zip(expected)
            .all(|(item, (tile_id, reason))| {
                item.identity.tile_id == tile_id && item.dependency_reason == reason
            });
    let actual_is_expected = category.actual_work.len() == expected_len
        && category
            .actual_work
            .iter()
            .zip([lower_tile_id, upper_tile_id])
            .all(|(item, tile_id)| item.identity.tile_id == tile_id && item.operations == 1);
    if !closure_is_expected || !actual_is_expected || report.actual_operation_count != 2 {
        violations.push(
            "transparent-overlap scenario requires z-ordered DirectChange and VisualOverlap render-encoding items"
                .into(),
        );
    }
}

fn validate_transparent_overlap_damage_category(
    category: &DamageCategory,
    lower_tile_id: &str,
    upper_tile_id: &str,
    report: &ChangeEfficiencyDamageReport,
    violations: &mut Vec<String>,
) {
    let closure_is_expected = category.closure_items.len() == 2
        && category.closure_items[0].identity.tile_id == lower_tile_id
        && category.closure_items[0].dependency_reason
            == InvalidationDependencyReason::DirectChange
        && category.closure_items[1].identity.tile_id == upper_tile_id
        && category.closure_items[1].dependency_reason
            == InvalidationDependencyReason::VisualOverlap;
    let actual_is_expected = category.actual_work.len() == 2
        && category.actual_work[0].identity == category.closure_items[0].identity
        && category.actual_work[0].operations == 1
        && category.actual_work[1].identity == category.closure_items[1].identity
        && category.actual_work[1].operations == 1;
    if !closure_is_expected || !actual_is_expected || report.category.actual_operation_count != 2 {
        violations.push(
            "transparent-overlap scenario requires direct lower and visual-overlap upper damage members"
                .into(),
        );
    }
}

fn validate_canonical_node_category(
    category_name: &str,
    category: &InvalidationCategory<NodeWorkItemId>,
    changed: &NodeWorkItemId,
    report: &ChangeEfficiencyCategoryReport,
    violations: &mut Vec<String>,
) {
    if category.closure_items.len() != 1
        || category.closure_items[0].identity != *changed
        || category.closure_items[0].dependency_reason != InvalidationDependencyReason::DirectChange
        || report.actual_operation_count != 1
    {
        violations.push(format!(
            "canonical one-node scenario requires exactly one directly changed {category_name} item"
        ));
    }
}

fn validate_canonical_render_category(
    category: &InvalidationCategory<RenderPlanWorkItemId>,
    changed_tile_id: &str,
    report: &ChangeEfficiencyCategoryReport,
    violations: &mut Vec<String>,
) {
    if category.closure_items.len() != 1
        || category.closure_items[0].identity.tile_id != changed_tile_id
        || category.closure_items[0].dependency_reason != InvalidationDependencyReason::DirectChange
        || report.actual_operation_count != 1
    {
        violations.push(
            "canonical one-node scenario requires exactly one directly changed render-encoding item"
                .into(),
        );
    }
}

fn validate_canonical_damage_category(
    category: &DamageCategory,
    changed_tile_id: &str,
    report: &ChangeEfficiencyDamageReport,
    violations: &mut Vec<String>,
) {
    if category.closure_items.len() != 1
        || category.closure_items[0].identity.tile_id != changed_tile_id
        || category.closure_items[0].dependency_reason != InvalidationDependencyReason::DirectChange
        || report.category.actual_operation_count != 1
    {
        violations.push(
            "canonical one-node scenario requires exactly one directly changed damage region"
                .into(),
        );
    }
}

fn validate_category<T>(
    category_name: &str,
    category: &InvalidationCategory<T>,
    allow_outside_closure: bool,
    violations: &mut Vec<String>,
) -> ChangeEfficiencyCategoryReport
where
    T: Clone + Debug + Ord + WorkItemIdentity,
{
    let mut eligible = BTreeSet::new();
    for work in &category.closure_items {
        work.identity
            .validate_components(category_name, "closure", violations);
        if !eligible.insert(work.identity.clone()) {
            violations.push(format!(
                "{category_name} closure contains duplicate identity {:?}",
                work.identity
            ));
        }
    }

    let mut actual_operation_count = 0_u64;
    for work in &category.actual_work {
        work.identity
            .validate_components(category_name, "actual_work", violations);
        if work.operations == 0 {
            violations.push(format!(
                "{category_name} actual work item {:?} has zero operations",
                work.identity
            ));
        }
        if !allow_outside_closure && !eligible.contains(&work.identity) {
            violations.push(format!(
                "{category_name} actual work item {:?} is outside the invalidation closure",
                work.identity
            ));
        }
        match actual_operation_count.checked_add(work.operations) {
            Some(total) => actual_operation_count = total,
            None => violations.push(format!("{category_name} actual operation count overflow")),
        }
    }

    let closure_cardinality = eligible.len() as u64;
    if !allow_outside_closure && actual_operation_count > closure_cardinality {
        violations.push(format!(
            "{category_name} actual operations {actual_operation_count} exceed closure cardinality {closure_cardinality}"
        ));
    }

    ChangeEfficiencyCategoryReport {
        closure_cardinality,
        actual_operation_count,
        amplification: AmplificationRatio {
            numerator: actual_operation_count,
            denominator: closure_cardinality,
        },
    }
}

fn validate_texture_upload(
    category: &TextureUploadCategory,
    violations: &mut Vec<String>,
) -> ChangeEfficiencyTextureUploadReport {
    let generic = InvalidationCategory {
        closure_items: category.closure_items.clone(),
        actual_work: category
            .actual_work
            .iter()
            .map(|work| ActualWorkItem {
                identity: work.identity.clone(),
                operations: work.operations,
            })
            .collect(),
    };
    let category_report = validate_category("texture_upload", &generic, false, violations);
    let mut uploaded_byte_count = 0_u64;
    for work in &category.actual_work {
        if work.operations > 0 && work.uploaded_bytes == 0 {
            violations.push(format!(
                "texture_upload actual work item {:?} has no uploaded bytes",
                work.identity
            ));
        }
        match uploaded_byte_count.checked_add(work.uploaded_bytes) {
            Some(total) => uploaded_byte_count = total,
            None => violations.push("texture_upload uploaded byte count overflow".into()),
        }
    }
    ChangeEfficiencyTextureUploadReport {
        category: category_report,
        uploaded_byte_count,
    }
}

fn validate_damage(
    category: &DamageCategory,
    viewport: EfficiencyViewport,
    has_full_surface_metadata: bool,
    violations: &mut Vec<String>,
) -> ChangeEfficiencyDamageReport {
    let viewport_rect = PixelRect {
        x: 0,
        y: 0,
        width: viewport.width,
        height: viewport.height,
    };

    let closure_rects: Vec<_> = category
        .closure_items
        .iter()
        .map(|work| work.identity.bounds)
        .collect();
    let actual_rects: Vec<_> = category
        .actual_work
        .iter()
        .map(|work| work.identity.bounds)
        .collect();
    let viewport_pixel_area = u64::from(viewport.width) * u64::from(viewport.height);
    let damaged_pixel_area = union_area(&actual_rects);
    let is_structured_full_surface = has_full_surface_metadata
        && viewport_pixel_area > 0
        && damaged_pixel_area == viewport_pixel_area;
    let category_report = validate_category(
        "composition_damage",
        category,
        is_structured_full_surface,
        violations,
    );

    for rect in &closure_rects {
        if !rect.is_non_empty() {
            violations
                .push("composition_damage closure contains an empty or overflowing rect".into());
        }
        if !viewport_rect.contains(*rect) {
            violations.push("composition_damage closure rect lies outside the viewport".into());
        }
    }
    for rect in &actual_rects {
        if !rect.is_non_empty() {
            violations.push(
                "composition_damage actual work contains an empty or overflowing rect".into(),
            );
        }
        if !viewport_rect.contains(*rect) {
            violations.push("composition_damage actual rect lies outside the viewport".into());
        }
        if !is_structured_full_surface && !rect_is_covered_by_union(*rect, &closure_rects) {
            violations.push(
                "composition_damage actual rect is outside the closure affected-output union"
                    .into(),
            );
        }
    }

    let closure_affected_pixel_area = union_area(&closure_rects);
    if !is_structured_full_surface && damaged_pixel_area > closure_affected_pixel_area {
        violations.push(format!(
            "damaged pixel area {damaged_pixel_area} exceeds closure affected area {closure_affected_pixel_area}"
        ));
    }

    ChangeEfficiencyDamageReport {
        category: category_report,
        closure_affected_pixel_area,
        damaged_pixel_area,
        viewport_pixel_area,
        amplification: AmplificationRatio {
            numerator: damaged_pixel_area,
            denominator: closure_affected_pixel_area,
        },
    }
}

fn rect_is_covered_by_union(target: PixelRect, covers: &[PixelRect]) -> bool {
    if !target.is_non_empty() {
        return false;
    }
    let (Some(target_right), Some(target_bottom)) = (target.right(), target.bottom()) else {
        return false;
    };

    let mut xs = vec![target.x, target_right];
    let mut ys = vec![target.y, target_bottom];
    for rect in covers {
        let (Some(right), Some(bottom)) = (rect.right(), rect.bottom()) else {
            continue;
        };
        let left = rect.x.max(target.x);
        let top = rect.y.max(target.y);
        let clipped_right = right.min(target_right);
        let clipped_bottom = bottom.min(target_bottom);
        if left < clipped_right && top < clipped_bottom {
            xs.extend([left, clipped_right]);
            ys.extend([top, clipped_bottom]);
        }
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();

    xs.windows(2).all(|x| {
        ys.windows(2).all(|y| {
            let cell = PixelRect {
                x: x[0],
                y: y[0],
                width: x[1] - x[0],
                height: y[1] - y[0],
            };
            covers.iter().copied().any(|cover| cover.contains(cell))
        })
    })
}

fn union_area(rects: &[PixelRect]) -> u64 {
    let valid_rects: Vec<_> = rects
        .iter()
        .copied()
        .filter(|rect| rect.is_non_empty())
        .collect();
    if valid_rects.is_empty() {
        return 0;
    }

    let mut xs = Vec::with_capacity(valid_rects.len() * 2);
    let mut ys = Vec::with_capacity(valid_rects.len() * 2);
    for rect in &valid_rects {
        let Some(right) = rect.right() else {
            continue;
        };
        let Some(bottom) = rect.bottom() else {
            continue;
        };
        xs.extend([rect.x, right]);
        ys.extend([rect.y, bottom]);
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();

    let mut area = 0_u64;
    for x in xs.windows(2) {
        for y in ys.windows(2) {
            let cell = PixelRect {
                x: x[0],
                y: y[0],
                width: x[1] - x[0],
                height: y[1] - y[0],
            };
            if valid_rects.iter().copied().any(|rect| rect.contains(cell)) {
                area += u64::from(cell.width) * u64::from(cell.height);
            }
        }
    }
    area
}
