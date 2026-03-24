//! Validation errors for scene graph operations.
//!
//! # Structured batch errors (RFC 0001 §3.4)
//!
//! [`BatchRejected`] is the structured rejection response for a [`crate::mutation::MutationBatch`].
//! It carries a [`BatchValidationError`] for each failing mutation, containing:
//! - `mutation_index` (0-based)
//! - `mutation_type` (human-readable name)
//! - [`ValidationErrorCode`] (stable across minor versions)
//! - `message` (human-readable)
//! - `context` (JSON — machine-readable field/value/constraint)
//! - `correction_hint` (optional JSON — machine-readable suggested fix)

use crate::types::{ResourceId, SceneId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── Low-level ValidationError (internal per-operation) ──────────────────────

#[derive(Clone, Debug, Error, PartialEq)]
pub enum ValidationError {
    #[error("tab not found: {id}")]
    TabNotFound { id: SceneId },

    #[error("tile not found: {id}")]
    TileNotFound { id: SceneId },

    #[error("node not found: {id}")]
    NodeNotFound { id: SceneId },

    #[error("lease not found: {id}")]
    LeaseNotFound { id: SceneId },

    #[error("lease expired: {id}")]
    LeaseExpired { id: SceneId },

    #[error("duplicate display order: {order}")]
    DuplicateDisplayOrder { order: u32 },

    /// An ID that must be unique appears more than once in the scene graph.
    /// RFC 0001 §2.1: no TabId, TileId, or NodeId may appear more than once.
    #[error("duplicate id: {id}")]
    DuplicateId { id: SceneId },

    #[error("invalid field '{field}': {reason}")]
    InvalidField { field: String, reason: String },

    /// Tile bounds failed a spatial constraint (non-positive dimensions or out-of-display-area).
    /// RFC 0001 §2.3.
    #[error("bounds out of range: {reason}")]
    BoundsOutOfRange { reason: String },

    #[error("budget exceeded: {resource}")]
    BudgetExceeded { resource: String },

    /// A tile already has the maximum number of nodes (64 per RFC 0001 §2.1).
    #[error("node count exceeded for tile {tile_id}: has {current}, limit {limit}")]
    NodeCountExceeded { tile_id: SceneId, current: usize, limit: usize },

    #[error("capability missing: {capability}")]
    CapabilityMissing { capability: String },

    /// A mutation referenced a ResourceId that is not registered with the runtime.
    /// RFC 0001 §2.4: StaticImageNode referencing unknown ResourceId.
    #[error("resource not found: {id}")]
    ResourceNotFound { id: ResourceId },

    #[error("zone not found: {name}")]
    ZoneNotFound { name: String },

    #[error("zone publish token invalid for zone '{zone}'")]
    ZonePublishTokenInvalid { zone: String },

    #[error("zone media type mismatch for zone '{zone}'")]
    ZoneMediaTypeMismatch { zone: String },

    #[error("zone '{zone}' has reached max publishers ({max})")]
    ZoneMaxPublishersReached { zone: String, max: u32 },

    #[error("zone '{zone}' has reached max keys ({max})")]
    ZoneMaxKeysReached { zone: String, max: u32 },

    #[error("sync group not found: {id}")]
    SyncGroupNotFound { id: crate::types::SceneId },

    #[error("sync group limit exceeded: {limit} sync groups per namespace")]
    SyncGroupLimitExceeded { limit: usize },

    #[error("sync group member limit exceeded: {limit} tiles per sync group")]
    SyncGroupMemberLimitExceeded { limit: usize },

    /// A mutation was rejected because the requesting agent does not own the target tile.
    /// RFC 0001 §1.2: namespace isolation.
    #[error("namespace mismatch: tile {tile_id} belongs to namespace '{tile_namespace}', not '{agent_namespace}'")]
    NamespaceMismatch {
        tile_id: SceneId,
        tile_namespace: String,
        agent_namespace: String,
    },

    /// A cycle was introduced in the node tree by a batch mutation.
    #[error("cycle detected in node tree: node {node_id} creates a cycle")]
    CycleDetected { node_id: SceneId },

    /// Two non-passthrough tiles on the same tab share a z_order with overlapping bounds.
    #[error("z-order conflict: tiles {tile_a} and {tile_b} share z_order {z_order} with overlapping bounds")]
    ZOrderConflict { tile_a: SceneId, tile_b: SceneId, z_order: u32 },

    /// Batch size exceeded the 1000-mutation hard limit.
    #[error("batch size exceeded: max {max}, got {got}")]
    BatchSizeExceeded { max: usize, got: usize },
}

// ─── Stable error codes (RFC 0001 §3.4) ─────────────────────────────────────

/// Stable validation error codes. These are stable across minor versions and
/// are the machine-readable discriminants in [`BatchValidationError`].
///
/// Codes map 1-1 to the five validation stages:
/// - Stage 1 (Lease): `LeaseNotFound`, `LeaseExpired`, `LeaseInvalidState`
/// - Stage 2 (Budget): `BudgetExceeded`, `BatchSizeExceeded`
/// - Stage 3 (Bounds): `BoundsOutOfRange`, `BoundsInvalid`
/// - Stage 4 (Type): `TypeMismatch`, `CapabilityMissing`
/// - Stage 5 (Invariant): `CycleDetected`, `ZOrderConflict`, `DuplicateId`,
///   `ReferenceInvalid`, `TabNotFound`, `TileNotFound`, `NodeNotFound`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationErrorCode {
    // Stage 1 – Lease
    LeaseNotFound,
    LeaseExpired,
    LeaseInvalidState,

    // Stage 2 – Budget
    BudgetExceeded,
    BatchSizeExceeded,

    // Stage 3 – Bounds
    BoundsOutOfRange,
    BoundsInvalid,

    // Stage 4 – Type / capability
    TypeMismatch,
    CapabilityMissing,

    // Stage 5 – Invariants
    CycleDetected,
    ZOrderConflict,
    DuplicateId,
    ReferenceInvalid,
    TabNotFound,
    TileNotFound,
    NodeNotFound,

    // Zone / zone-related
    ZoneNotFound,
    ZonePublishTokenInvalid,
    ZoneMediaTypeMismatch,
    ZoneMaxPublishersReached,
    ZoneMaxKeysReached,

    // Sync group
    SyncGroupNotFound,
    SyncGroupLimitExceeded,
    SyncGroupMemberLimitExceeded,

    // Misc
    InvalidField,
    DuplicateDisplayOrder,
    NodeCountExceeded,
    ResourceNotFound,
    NamespaceMismatch,

    // Unknown / future-proof catch-all
    Unknown,
}

impl ValidationErrorCode {
    /// Derive the stable code from a [`ValidationError`].
    pub fn from_error(e: &ValidationError) -> Self {
        match e {
            ValidationError::LeaseNotFound { .. } => Self::LeaseNotFound,
            ValidationError::LeaseExpired { .. } => Self::LeaseExpired,
            ValidationError::TabNotFound { .. } => Self::TabNotFound,
            ValidationError::TileNotFound { .. } => Self::TileNotFound,
            ValidationError::NodeNotFound { .. } => Self::NodeNotFound,
            ValidationError::DuplicateDisplayOrder { .. } => Self::DuplicateDisplayOrder,
            ValidationError::DuplicateId { .. } => Self::DuplicateId,
            // BoundsOutOfRange covers two cases:
            // - invalid dimensions (width/height <= 0.0) → BoundsInvalid
            // - outside display area → BoundsOutOfRange
            ValidationError::BoundsOutOfRange { reason } if reason.contains("must be > 0.0") => {
                Self::BoundsInvalid
            }
            ValidationError::BoundsOutOfRange { .. } => Self::BoundsOutOfRange,
            ValidationError::NodeCountExceeded { .. } => Self::NodeCountExceeded,
            ValidationError::ResourceNotFound { .. } => Self::ResourceNotFound,
            ValidationError::NamespaceMismatch { .. } => Self::NamespaceMismatch,
            ValidationError::InvalidField { field, .. }
                if field == "bounds" || field == "width" || field == "height" =>
            {
                Self::BoundsInvalid
            }
            ValidationError::InvalidField { field, .. } if field == "lease_state" => {
                Self::LeaseInvalidState
            }
            ValidationError::InvalidField { .. } => Self::InvalidField,
            ValidationError::BudgetExceeded { .. } => Self::BudgetExceeded,
            ValidationError::CapabilityMissing { .. } => Self::CapabilityMissing,
            ValidationError::ZoneNotFound { .. } => Self::ZoneNotFound,
            ValidationError::ZonePublishTokenInvalid { .. } => Self::ZonePublishTokenInvalid,
            ValidationError::ZoneMediaTypeMismatch { .. } => Self::ZoneMediaTypeMismatch,
            ValidationError::ZoneMaxPublishersReached { .. } => Self::ZoneMaxPublishersReached,
            ValidationError::ZoneMaxKeysReached { .. } => Self::ZoneMaxKeysReached,
            ValidationError::SyncGroupNotFound { .. } => Self::SyncGroupNotFound,
            ValidationError::SyncGroupLimitExceeded { .. } => Self::SyncGroupLimitExceeded,
            ValidationError::SyncGroupMemberLimitExceeded { .. } => {
                Self::SyncGroupMemberLimitExceeded
            }
            ValidationError::CycleDetected { .. } => Self::CycleDetected,
            ValidationError::ZOrderConflict { .. } => Self::ZOrderConflict,
            ValidationError::BatchSizeExceeded { .. } => Self::BatchSizeExceeded,
        }
    }
}

// ─── Structured per-mutation validation error (RFC 0001 §3.4) ───────────────

/// Structured per-mutation validation error included in a [`BatchRejected`] response.
///
/// All fields are stable across minor versions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchValidationError {
    /// 0-based index of the failing mutation within the batch.
    pub mutation_index: usize,

    /// Human-readable mutation type name (e.g. `"CreateTile"`).
    pub mutation_type: String,

    /// Stable error code.
    pub code: ValidationErrorCode,

    /// Human-readable diagnostic message.
    pub message: String,

    /// Machine-readable structured context (JSON object).
    ///
    /// Always a JSON object. Keys: `field`, `value`, `constraint` (where applicable),
    /// plus error-specific fields.
    pub context: serde_json::Value,

    /// Optional machine-readable correction hint (JSON object).
    ///
    /// Present when the runtime can suggest a concrete fix. Keys vary by error code.
    pub correction_hint: Option<serde_json::Value>,
}

impl BatchValidationError {
    /// Build a [`BatchValidationError`] from a raw [`ValidationError`].
    pub fn from_validation_error(
        mutation_index: usize,
        mutation_type: impl Into<String>,
        error: &ValidationError,
    ) -> Self {
        let code = ValidationErrorCode::from_error(error);
        let message = error.to_string();
        let (context, correction_hint) = build_context_and_hint(error);

        Self {
            mutation_index,
            mutation_type: mutation_type.into(),
            code,
            message,
            context,
            correction_hint,
        }
    }
}

/// Build the `context` and `correction_hint` JSON for a [`ValidationError`].
fn build_context_and_hint(
    error: &ValidationError,
) -> (serde_json::Value, Option<serde_json::Value>) {
    use serde_json::json;

    match error {
        ValidationError::LeaseExpired { id } => (
            json!({ "field": "lease_id", "value": id.to_string(), "constraint": "lease must be in Active state" }),
            Some(json!({ "action": "renew_lease", "lease_id": id.to_string() })),
        ),
        ValidationError::LeaseNotFound { id } => (
            json!({ "field": "lease_id", "value": id.to_string(), "constraint": "lease must exist" }),
            None,
        ),
        ValidationError::TileNotFound { id } => (
            json!({ "field": "tile_id", "value": id.to_string(), "constraint": "tile must exist" }),
            None,
        ),
        ValidationError::TabNotFound { id } => (
            json!({ "field": "tab_id", "value": id.to_string(), "constraint": "tab must exist" }),
            None,
        ),
        ValidationError::NodeNotFound { id } => (
            json!({ "field": "node_id", "value": id.to_string(), "constraint": "node must exist" }),
            None,
        ),
        ValidationError::DuplicateId { id } => (
            json!({ "field": "id", "value": id.to_string(), "constraint": "id must be unique in the scene graph" }),
            None,
        ),
        ValidationError::BoundsOutOfRange { reason } => (
            json!({ "field": "bounds", "constraint": reason }),
            None,
        ),
        ValidationError::NodeCountExceeded { tile_id, current, limit } => (
            json!({ "field": "node_count", "value": current, "constraint": format!("max {} nodes per tile", limit), "tile_id": tile_id.to_string() }),
            None,
        ),
        ValidationError::ResourceNotFound { id } => (
            json!({ "field": "resource_id", "value": id.to_string(), "constraint": "resource must be registered" }),
            None,
        ),
        ValidationError::NamespaceMismatch { tile_id, tile_namespace, agent_namespace } => (
            json!({ "field": "namespace", "value": agent_namespace, "constraint": format!("tile {} belongs to namespace '{}'", tile_id, tile_namespace) }),
            None,
        ),
        ValidationError::BudgetExceeded { resource } => (
            json!({ "field": "resource_budget", "value": resource, "constraint": "must not exceed lease budget" }),
            Some(json!({ "action": "reduce_resource_usage", "resource": resource })),
        ),
        ValidationError::InvalidField { field, reason } => (
            json!({ "field": field, "constraint": reason }),
            None,
        ),
        ValidationError::BatchSizeExceeded { max, got } => (
            json!({ "field": "mutations", "value": got, "constraint": format!("max {} mutations per batch", max) }),
            Some(json!({ "action": "split_batch", "max_batch_size": max })),
        ),
        ValidationError::CycleDetected { node_id } => (
            json!({ "field": "node_id", "value": node_id.to_string(), "constraint": "node tree must be acyclic" }),
            None,
        ),
        ValidationError::ZOrderConflict { tile_a, tile_b, z_order } => (
            json!({
                "field": "z_order",
                "value": z_order,
                "constraint": "non-passthrough tiles on the same tab with overlapping bounds must have distinct z_order",
                "conflicting_tiles": [tile_a.to_string(), tile_b.to_string()]
            }),
            Some(json!({ "action": "adjust_z_order", "suggested_z_order": z_order + 1 })),
        ),
        ValidationError::CapabilityMissing { capability } => (
            json!({ "field": "capability", "value": capability, "constraint": "lease must have this capability" }),
            None,
        ),
        ValidationError::DuplicateDisplayOrder { order } => (
            json!({ "field": "display_order", "value": order, "constraint": "display_order must be unique per scene" }),
            None,
        ),
        ValidationError::ZoneNotFound { name } => (
            json!({ "field": "zone_name", "value": name, "constraint": "zone must be registered" }),
            None,
        ),
        ValidationError::ZonePublishTokenInvalid { zone } => (
            json!({ "field": "publish_token", "value": zone, "constraint": "token must be valid for zone" }),
            None,
        ),
        ValidationError::ZoneMediaTypeMismatch { zone } => (
            json!({ "field": "content", "value": zone, "constraint": "content media type must match zone accepted types" }),
            None,
        ),
        ValidationError::ZoneMaxPublishersReached { zone, max } => (
            json!({ "field": "zone_name", "value": zone, "constraint": format!("max {} publishers", max) }),
            None,
        ),
        ValidationError::ZoneMaxKeysReached { zone, max } => (
            json!({ "field": "zone_name", "value": zone, "constraint": format!("max {} keys", max) }),
            None,
        ),
        ValidationError::SyncGroupNotFound { id } => (
            json!({ "field": "group_id", "value": id.to_string(), "constraint": "sync group must exist" }),
            None,
        ),
        ValidationError::SyncGroupLimitExceeded { limit } => (
            json!({ "field": "sync_group_count", "constraint": format!("max {} sync groups per namespace", limit) }),
            None,
        ),
        ValidationError::SyncGroupMemberLimitExceeded { limit } => (
            json!({ "field": "member_count", "constraint": format!("max {} tiles per sync group", limit) }),
            None,
        ),
    }
}

// ─── BatchRejected (RFC 0001 §3.4) ───────────────────────────────────────────

/// The structured rejection response for an atomic [`crate::mutation::MutationBatch`].
///
/// The batch was rejected entirely (all-or-nothing). At least one
/// [`BatchValidationError`] is always present. Additional entries may be present
/// if multiple mutations failed pre-flight validation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchRejected {
    /// ID of the batch that was rejected.
    pub batch_id: SceneId,

    /// Structured validation errors. At least one is always present.
    ///
    /// Errors are ordered by `mutation_index`. For batch-level rejections
    /// (e.g. `BatchSizeExceeded`), `mutation_index` is `usize::MAX`.
    pub errors: Vec<BatchValidationError>,
}

impl BatchRejected {
    /// Create a single-error batch rejection.
    pub fn single(
        batch_id: SceneId,
        mutation_index: usize,
        mutation_type: impl Into<String>,
        error: &ValidationError,
    ) -> Self {
        Self {
            batch_id,
            errors: vec![BatchValidationError::from_validation_error(
                mutation_index,
                mutation_type,
                error,
            )],
        }
    }

    /// Create a batch-level rejection (e.g. `BatchSizeExceeded`).
    ///
    /// Uses `mutation_index = usize::MAX` to signal a batch-level error.
    pub fn batch_level(batch_id: SceneId, mutation_type: impl Into<String>, error: &ValidationError) -> Self {
        Self::single(batch_id, usize::MAX, mutation_type, error)
    }

    /// Return the primary (first) error code.
    pub fn primary_code(&self) -> Option<ValidationErrorCode> {
        self.errors.first().map(|e| e.code)
    }
}
