//! Validation errors for scene graph operations.

use crate::types::{ResourceId, SceneId};
use thiserror::Error;

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
}
