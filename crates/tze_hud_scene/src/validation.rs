//! Validation errors for scene graph operations.

use crate::types::SceneId;
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

    #[error("invalid field '{field}': {reason}")]
    InvalidField { field: String, reason: String },

    #[error("budget exceeded: {resource}")]
    BudgetExceeded { resource: String },

    #[error("capability missing: {capability}")]
    CapabilityMissing { capability: String },

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

    #[error("tile already in a sync group")]
    TileAlreadyInSyncGroup { tile_id: crate::types::SceneId },

    #[error("sync group limit exceeded: {limit} sync groups per namespace")]
    SyncGroupLimitExceeded { limit: usize },

    #[error("sync group member limit exceeded: {limit} tiles per sync group")]
    SyncGroupMemberLimitExceeded { limit: usize },
}
