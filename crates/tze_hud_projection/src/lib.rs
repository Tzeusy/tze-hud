//! Provider-neutral cooperative HUD projection operation contract.
//!
//! This crate owns the low-token operation schema for the external projection
//! authority described by `openspec/changes/cooperative-hud-projection/`.
//! It deliberately models projection-daemon operations, not runtime v1 MCP
//! tools. If the contract is exposed through MCP, that MCP server belongs to
//! the projection daemon and talks outward to the HUD over the resident control
//! plane.

#[cfg(feature = "resident-grpc")]
pub mod resident_grpc;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use subtle::ConstantTimeEq;
use thiserror::Error;

/// Default maximum bytes accepted by one `publish_output` request.
pub const DEFAULT_MAX_OUTPUT_BYTES_PER_CALL: usize = 16_384;
/// Default maximum bytes accepted by `publish_status.status_text`.
pub const DEFAULT_MAX_STATUS_TEXT_BYTES: usize = 512;
/// Default retained transcript byte budget for a projection.
pub const DEFAULT_MAX_RETAINED_TRANSCRIPT_BYTES: usize = 262_144;
/// Default visible transcript byte budget for portal materialization.
pub const DEFAULT_MAX_VISIBLE_TRANSCRIPT_BYTES: usize = 16_384;
/// Default maximum number of pending HUD input items.
pub const DEFAULT_MAX_PENDING_INPUT_ITEMS: usize = 32;
/// Default maximum bytes in one HUD input item.
pub const DEFAULT_MAX_PENDING_INPUT_BYTES_PER_ITEM: usize = 4_096;
/// Default maximum aggregate pending HUD input bytes.
pub const DEFAULT_MAX_PENDING_INPUT_TOTAL_BYTES: usize = 32_768;
/// Default maximum pending items returned by one poll.
pub const DEFAULT_MAX_POLL_ITEMS: usize = 8;
/// Default maximum bytes returned by one pending-input poll.
pub const DEFAULT_MAX_POLL_RESPONSE_BYTES: usize = 16_384;
/// Default maximum HUD portal updates per second.
pub const DEFAULT_MAX_PORTAL_UPDATES_PER_SECOND: u32 = 10;
/// Default maximum retained publish-output logical-unit IDs per projection.
pub const DEFAULT_MAX_SEEN_LOGICAL_UNITS: usize = 4_096;
/// Default maximum retained audit records for the in-memory authority.
pub const DEFAULT_MAX_AUDIT_RECORDS: usize = 4_096;
/// Owner tokens are 256-bit random values encoded as lowercase hex.
pub const OWNER_TOKEN_ENTROPY_BITS: usize = 256;
/// Default owner-token lifetime in wall-clock microseconds.
pub const DEFAULT_OWNER_TOKEN_TTL_WALL_US: u64 = 24 * 60 * 60 * 1_000_000;
/// One wall-clock second in microseconds, used for portal update-rate windows.
pub const PORTAL_UPDATE_RATE_WINDOW_WALL_US: u64 = 1_000_000;

const MAX_PROJECTION_ID_BYTES: usize = 128;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_CALLER_IDENTITY_BYTES: usize = 256;
const MAX_DISPLAY_NAME_BYTES: usize = 128;
const MAX_HINT_BYTES: usize = 256;
const MAX_STATUS_SUMMARY_BYTES: usize = 512;
const MAX_REASON_BYTES: usize = 512;
const MAX_ACK_MESSAGE_BYTES: usize = 512;
const MAX_PORTAL_ID_BYTES: usize = 192;
const DEFAULT_PORTAL_INPUT_TTL_WALL_US: u64 = 10 * 60 * 1_000_000;

/// Stable append-only projection error codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectionErrorCode {
    ProjectionNotFound,
    ProjectionAlreadyAttached,
    ProjectionUnauthorized,
    ProjectionTokenExpired,
    ProjectionInvalidArgument,
    ProjectionOutputTooLarge,
    ProjectionInputTooLarge,
    ProjectionInputQueueFull,
    ProjectionRateLimited,
    ProjectionStateConflict,
    ProjectionHudUnavailable,
    ProjectionInternalError,
}

impl ProjectionErrorCode {
    /// Stable wire string for this error code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectionNotFound => "PROJECTION_NOT_FOUND",
            Self::ProjectionAlreadyAttached => "PROJECTION_ALREADY_ATTACHED",
            Self::ProjectionUnauthorized => "PROJECTION_UNAUTHORIZED",
            Self::ProjectionTokenExpired => "PROJECTION_TOKEN_EXPIRED",
            Self::ProjectionInvalidArgument => "PROJECTION_INVALID_ARGUMENT",
            Self::ProjectionOutputTooLarge => "PROJECTION_OUTPUT_TOO_LARGE",
            Self::ProjectionInputTooLarge => "PROJECTION_INPUT_TOO_LARGE",
            Self::ProjectionInputQueueFull => "PROJECTION_INPUT_QUEUE_FULL",
            Self::ProjectionRateLimited => "PROJECTION_RATE_LIMITED",
            Self::ProjectionStateConflict => "PROJECTION_STATE_CONFLICT",
            Self::ProjectionHudUnavailable => "PROJECTION_HUD_UNAVAILABLE",
            Self::ProjectionInternalError => "PROJECTION_INTERNAL_ERROR",
        }
    }
}

impl fmt::Display for ProjectionErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Initial stable error-code set required by the projection contract.
pub const INITIAL_ERROR_CODES: [ProjectionErrorCode; 12] = [
    ProjectionErrorCode::ProjectionNotFound,
    ProjectionErrorCode::ProjectionAlreadyAttached,
    ProjectionErrorCode::ProjectionUnauthorized,
    ProjectionErrorCode::ProjectionTokenExpired,
    ProjectionErrorCode::ProjectionInvalidArgument,
    ProjectionErrorCode::ProjectionOutputTooLarge,
    ProjectionErrorCode::ProjectionInputTooLarge,
    ProjectionErrorCode::ProjectionInputQueueFull,
    ProjectionErrorCode::ProjectionRateLimited,
    ProjectionErrorCode::ProjectionStateConflict,
    ProjectionErrorCode::ProjectionHudUnavailable,
    ProjectionErrorCode::ProjectionInternalError,
];

/// Provider-neutral projection operation names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOperation {
    Attach,
    PublishOutput,
    PublishStatus,
    GetPendingInput,
    AcknowledgeInput,
    Detach,
    Cleanup,
}

/// LLM provider kind. Provider-specific behavior must stay outside the core
/// projection operation semantics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Codex,
    Claude,
    Opencode,
    Other,
}

/// Projection lifecycle visible through bounded status summaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionLifecycleState {
    Attached,
    Active,
    Degraded,
    HudUnavailable,
    Detached,
    CleanupPending,
    Expired,
}

/// Viewer-facing classification. Missing classification defaults to private.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentClassification {
    Public,
    Household,
    #[default]
    Private,
    Sensitive,
}

/// Output kind for published transcript units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    #[default]
    Assistant,
    Tool,
    Status,
    Error,
    Other,
}

/// Acknowledgement state accepted from the owning LLM session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputAckState {
    Handled,
    Deferred,
    Rejected,
}

/// Daemon-owned input delivery state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputDeliveryState {
    Pending,
    Delivered,
    Deferred,
    Handled,
    Rejected,
    Expired,
}

impl InputDeliveryState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Handled | Self::Rejected | Self::Expired)
    }
}

/// Authority path used for cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupAuthority {
    Owner,
    Operator,
}

/// Cooperative projected-session adapter family. The v1 projection path is a
/// text-stream portal adapter, not a PTY or terminal-capture adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedPortalAdapterFamily {
    CooperativeProjection,
}

/// Runtime authority used to publish projected portal mutations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedPortalRuntimeAuthority {
    ResidentSessionLease,
}

/// Projected portals render as content-layer territory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedPortalLayer {
    Content,
}

/// Expanded or collapsed projected-portal presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedPortalPresentation {
    Expanded,
    Collapsed,
}

/// Ambient attention state exposed by projected portals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedPortalAttention {
    Ambient,
}

/// Local-first HUD composer feedback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalInputFeedbackState {
    Accepted,
    Rejected,
}

/// Audit category. Owner cleanup and operator cleanup are intentionally
/// separate categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionAuditCategory {
    Attach,
    OwnerPublish,
    OwnerStatus,
    OwnerInputRead,
    OwnerInputAck,
    OwnerDetach,
    OwnerCleanup,
    OperatorCleanup,
    AuthDenied,
    BoundsDenied,
    ConflictDenied,
}

/// Configurable contract bounds. Defaults match the v1 OpenSpec values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionBounds {
    pub max_output_bytes_per_call: usize,
    pub max_status_text_bytes: usize,
    pub max_retained_transcript_bytes: usize,
    pub max_visible_transcript_bytes: usize,
    pub max_pending_input_items: usize,
    pub max_pending_input_bytes_per_item: usize,
    pub max_pending_input_total_bytes: usize,
    pub max_poll_items: usize,
    pub max_poll_response_bytes: usize,
    pub max_portal_updates_per_second: u32,
    pub max_seen_logical_units: usize,
    pub max_audit_records: usize,
    pub owner_token_ttl_wall_us: u64,
}

impl Default for ProjectionBounds {
    fn default() -> Self {
        Self {
            max_output_bytes_per_call: DEFAULT_MAX_OUTPUT_BYTES_PER_CALL,
            max_status_text_bytes: DEFAULT_MAX_STATUS_TEXT_BYTES,
            max_retained_transcript_bytes: DEFAULT_MAX_RETAINED_TRANSCRIPT_BYTES,
            max_visible_transcript_bytes: DEFAULT_MAX_VISIBLE_TRANSCRIPT_BYTES,
            max_pending_input_items: DEFAULT_MAX_PENDING_INPUT_ITEMS,
            max_pending_input_bytes_per_item: DEFAULT_MAX_PENDING_INPUT_BYTES_PER_ITEM,
            max_pending_input_total_bytes: DEFAULT_MAX_PENDING_INPUT_TOTAL_BYTES,
            max_poll_items: DEFAULT_MAX_POLL_ITEMS,
            max_poll_response_bytes: DEFAULT_MAX_POLL_RESPONSE_BYTES,
            max_portal_updates_per_second: DEFAULT_MAX_PORTAL_UPDATES_PER_SECOND,
            max_seen_logical_units: DEFAULT_MAX_SEEN_LOGICAL_UNITS,
            max_audit_records: DEFAULT_MAX_AUDIT_RECORDS,
            owner_token_ttl_wall_us: DEFAULT_OWNER_TOKEN_TTL_WALL_US,
        }
    }
}

impl ProjectionBounds {
    pub fn validate(&self) -> Result<(), ProjectionContractError> {
        if self.max_output_bytes_per_call == 0
            || self.max_status_text_bytes == 0
            || self.max_retained_transcript_bytes == 0
            || self.max_visible_transcript_bytes == 0
            || self.max_pending_input_items == 0
            || self.max_pending_input_bytes_per_item == 0
            || self.max_pending_input_total_bytes == 0
            || self.max_poll_items == 0
            || self.max_poll_response_bytes == 0
            || self.max_portal_updates_per_second == 0
            || self.max_seen_logical_units == 0
            || self.max_audit_records == 0
            || self.owner_token_ttl_wall_us == 0
        {
            return Err(ProjectionContractError::InvalidArgument(
                "projection bounds must be non-zero".to_string(),
            ));
        }
        if self.max_visible_transcript_bytes > self.max_retained_transcript_bytes {
            return Err(ProjectionContractError::InvalidArgument(
                "visible transcript bound cannot exceed retained transcript bound".to_string(),
            ));
        }
        Ok(())
    }
}

/// Shared request envelope fields present on every operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationEnvelope {
    pub operation: ProjectionOperation,
    pub projection_id: String,
    pub request_id: String,
    pub client_timestamp_wall_us: u64,
}

impl OperationEnvelope {
    fn validate(&self, expected: ProjectionOperation) -> Result<(), ProjectionContractError> {
        if self.operation != expected {
            return Err(ProjectionContractError::InvalidArgument(format!(
                "operation must be {expected:?}"
            )));
        }
        validate_non_empty_bounded(
            "projection_id",
            &self.projection_id,
            MAX_PROJECTION_ID_BYTES,
        )?;
        validate_non_empty_bounded("request_id", &self.request_id, MAX_REQUEST_ID_BYTES)?;
        if self.client_timestamp_wall_us == 0 {
            return Err(ProjectionContractError::InvalidArgument(
                "client_timestamp_wall_us must be non-zero".to_string(),
            ));
        }
        Ok(())
    }
}

/// `attach` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachRequest {
    #[serde(flatten)]
    pub envelope: OperationEnvelope,
    pub provider_kind: ProviderKind,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_profile_hint: Option<String>,
    #[serde(default)]
    pub content_classification: ContentClassification,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hud_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl AttachRequest {
    pub fn validate(&self) -> Result<(), ProjectionContractError> {
        self.envelope.validate(ProjectionOperation::Attach)?;
        validate_non_empty_bounded("display_name", &self.display_name, MAX_DISPLAY_NAME_BYTES)?;
        validate_optional_bounded("workspace_hint", &self.workspace_hint, MAX_HINT_BYTES)?;
        validate_optional_bounded("repository_hint", &self.repository_hint, MAX_HINT_BYTES)?;
        validate_optional_bounded("icon_profile_hint", &self.icon_profile_hint, MAX_HINT_BYTES)?;
        validate_optional_bounded("hud_target", &self.hud_target, MAX_HINT_BYTES)?;
        validate_optional_bounded(
            "idempotency_key",
            &self.idempotency_key,
            MAX_REQUEST_ID_BYTES,
        )?;
        Ok(())
    }
}

/// `publish_output` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishOutputRequest {
    #[serde(flatten)]
    pub envelope: OperationEnvelope,
    pub owner_token: String,
    pub output_text: String,
    #[serde(default)]
    pub output_kind: OutputKind,
    #[serde(default)]
    pub content_classification: ContentClassification,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_unit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coalesce_key: Option<String>,
}

impl PublishOutputRequest {
    pub fn validate(&self, bounds: &ProjectionBounds) -> Result<(), ProjectionContractError> {
        self.envelope.validate(ProjectionOperation::PublishOutput)?;
        validate_owner_token(&self.owner_token)?;
        if self.output_text.len() > bounds.max_output_bytes_per_call {
            return Err(ProjectionContractError::StableCode(
                ProjectionErrorCode::ProjectionOutputTooLarge,
                "output_text exceeds max_output_bytes_per_call".to_string(),
            ));
        }
        validate_optional_bounded(
            "logical_unit_id",
            &self.logical_unit_id,
            MAX_REQUEST_ID_BYTES,
        )?;
        validate_optional_bounded("coalesce_key", &self.coalesce_key, MAX_REQUEST_ID_BYTES)?;
        Ok(())
    }
}

/// `publish_status` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishStatusRequest {
    #[serde(flatten)]
    pub envelope: OperationEnvelope,
    pub owner_token: String,
    pub lifecycle_state: ProjectionLifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
}

impl PublishStatusRequest {
    pub fn validate(&self, bounds: &ProjectionBounds) -> Result<(), ProjectionContractError> {
        self.envelope.validate(ProjectionOperation::PublishStatus)?;
        validate_owner_token(&self.owner_token)?;
        if self
            .status_text
            .as_ref()
            .is_some_and(|text| text.len() > bounds.max_status_text_bytes)
        {
            return Err(ProjectionContractError::InvalidArgument(
                "status_text exceeds max_status_text_bytes".to_string(),
            ));
        }
        Ok(())
    }
}

/// `get_pending_input` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetPendingInputRequest {
    #[serde(flatten)]
    pub envelope: OperationEnvelope,
    pub owner_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
}

impl GetPendingInputRequest {
    pub fn validate(&self) -> Result<(), ProjectionContractError> {
        self.envelope
            .validate(ProjectionOperation::GetPendingInput)?;
        validate_owner_token(&self.owner_token)
    }
}

/// `acknowledge_input` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcknowledgeInputRequest {
    #[serde(flatten)]
    pub envelope: OperationEnvelope,
    pub owner_token: String,
    pub input_id: String,
    pub ack_state: InputAckState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before_wall_us: Option<u64>,
}

impl AcknowledgeInputRequest {
    pub fn validate(&self) -> Result<(), ProjectionContractError> {
        self.envelope
            .validate(ProjectionOperation::AcknowledgeInput)?;
        validate_owner_token(&self.owner_token)?;
        validate_non_empty_bounded("input_id", &self.input_id, MAX_REQUEST_ID_BYTES)?;
        validate_optional_bounded("ack_message", &self.ack_message, MAX_ACK_MESSAGE_BYTES)?;
        if self.ack_state != InputAckState::Deferred && self.not_before_wall_us.is_some() {
            return Err(ProjectionContractError::InvalidArgument(
                "not_before_wall_us is valid only when ack_state is deferred".to_string(),
            ));
        }
        if self.ack_state == InputAckState::Deferred && self.not_before_wall_us == Some(0) {
            return Err(ProjectionContractError::InvalidArgument(
                "not_before_wall_us must be non-zero when present".to_string(),
            ));
        }
        Ok(())
    }
}

/// `detach` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachRequest {
    #[serde(flatten)]
    pub envelope: OperationEnvelope,
    pub owner_token: String,
    pub reason: String,
}

impl DetachRequest {
    pub fn validate(&self) -> Result<(), ProjectionContractError> {
        self.envelope.validate(ProjectionOperation::Detach)?;
        validate_owner_token(&self.owner_token)?;
        validate_non_empty_bounded("reason", &self.reason, MAX_REASON_BYTES)
    }
}

/// `cleanup` request. Owner cleanup requires `owner_token`; operator cleanup
/// requires a separate operator authority credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupRequest {
    #[serde(flatten)]
    pub envelope: OperationEnvelope,
    pub cleanup_authority: CleanupAuthority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_authority: Option<String>,
    pub reason: String,
}

impl CleanupRequest {
    pub fn validate(&self) -> Result<(), ProjectionContractError> {
        self.envelope.validate(ProjectionOperation::Cleanup)?;
        validate_non_empty_bounded("reason", &self.reason, MAX_REASON_BYTES)?;
        match self.cleanup_authority {
            CleanupAuthority::Owner => {
                validate_owner_token(self.owner_token.as_deref().ok_or_else(|| {
                    ProjectionContractError::InvalidArgument(
                        "owner cleanup requires owner_token".to_string(),
                    )
                })?)
            }
            CleanupAuthority::Operator => validate_non_empty_bounded(
                "operator_authority",
                self.operator_authority.as_deref().ok_or_else(|| {
                    ProjectionContractError::InvalidArgument(
                        "operator cleanup requires operator_authority".to_string(),
                    )
                })?,
                MAX_HINT_BYTES,
            ),
        }
    }
}

/// Bounded pending input item returned by `get_pending_input`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingInputItem {
    pub input_id: String,
    pub projection_id: String,
    pub submission_text: String,
    pub submitted_at_wall_us: u64,
    pub expires_at_wall_us: u64,
    pub delivery_state: InputDeliveryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at_wall_us: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before_wall_us: Option<u64>,
    #[serde(default)]
    pub content_classification: ContentClassification,
}

/// HUD-originated text submitted from an expanded projected portal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalInputSubmission {
    pub input_id: String,
    pub submission_text: String,
    pub submitted_at_wall_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_wall_us: Option<u64>,
    #[serde(default)]
    pub content_classification: ContentClassification,
}

impl PortalInputSubmission {
    fn effective_expires_at_wall_us(&self) -> Result<u64, ProjectionErrorCode> {
        if self.submitted_at_wall_us == 0 {
            return Err(ProjectionErrorCode::ProjectionInvalidArgument);
        }
        let expires_at_wall_us = self.expires_at_wall_us.map(Ok).unwrap_or_else(|| {
            self.submitted_at_wall_us
                .checked_add(DEFAULT_PORTAL_INPUT_TTL_WALL_US)
                .ok_or(ProjectionErrorCode::ProjectionInvalidArgument)
        })?;
        if expires_at_wall_us <= self.submitted_at_wall_us {
            return Err(ProjectionErrorCode::ProjectionInvalidArgument);
        }
        Ok(expires_at_wall_us)
    }
}

/// Bounded local feedback returned after a HUD composer submission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalInputFeedback {
    pub projection_id: String,
    pub input_id: String,
    pub feedback_state: PortalInputFeedbackState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ProjectionErrorCode>,
    pub pending_input_count: usize,
    pub pending_input_bytes: usize,
    pub status_summary: String,
}

/// Runtime session metadata retained by the projection authority while a HUD
/// connection is live. Lease use is authorized against these grants, not
/// against cached lease identity alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudConnectionMetadata {
    pub connection_id: String,
    pub authenticated_session_id: String,
    #[serde(default)]
    pub granted_capabilities: Vec<String>,
    pub connected_at_wall_us: u64,
    pub last_reconnect_wall_us: u64,
}

impl HudConnectionMetadata {
    fn validate(&self) -> Result<(), ProjectionContractError> {
        validate_non_empty_bounded("connection_id", &self.connection_id, MAX_HINT_BYTES)?;
        validate_non_empty_bounded(
            "authenticated_session_id",
            &self.authenticated_session_id,
            MAX_HINT_BYTES,
        )?;
        for capability in &self.granted_capabilities {
            validate_non_empty_bounded("granted_capability", capability, MAX_HINT_BYTES)?;
        }
        if self.connected_at_wall_us == 0 || self.last_reconnect_wall_us == 0 {
            return Err(ProjectionContractError::InvalidArgument(
                "HUD connection timestamps must be non-zero".to_string(),
            ));
        }
        Ok(())
    }
}

/// Advisory runtime lease identity cached by the projection authority. This
/// value never authorizes republish by itself; callers must also have a fresh
/// authenticated HUD connection with grants covering requested capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisoryLeaseIdentity {
    pub lease_id: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub acquired_at_wall_us: u64,
    pub expires_at_wall_us: u64,
}

impl AdvisoryLeaseIdentity {
    fn validate(&self) -> Result<(), ProjectionContractError> {
        validate_non_empty_bounded("lease_id", &self.lease_id, MAX_HINT_BYTES)?;
        for capability in &self.capabilities {
            validate_non_empty_bounded("lease_capability", capability, MAX_HINT_BYTES)?;
        }
        if self.acquired_at_wall_us == 0 || self.expires_at_wall_us == 0 {
            return Err(ProjectionContractError::InvalidArgument(
                "lease timestamps must be non-zero".to_string(),
            ));
        }
        if self.acquired_at_wall_us >= self.expires_at_wall_us {
            return Err(ProjectionContractError::InvalidArgument(
                "lease expiry must be after acquisition".to_string(),
            ));
        }
        Ok(())
    }
}

/// Reconnect counters retained while the projection authority process is alive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconnectBookkeeping {
    pub reconnect_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_disconnect_wall_us: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reconnect_wall_us: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_wall_us: Option<u64>,
}

/// One retained transcript logical unit. Text is memory-only v1 private state
/// and is purged when the owning projection is removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptUnit {
    pub sequence: u64,
    pub output_text: String,
    pub output_kind: OutputKind,
    pub content_classification: ContentClassification,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_unit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coalesce_key: Option<String>,
    pub appended_at_wall_us: u64,
}

impl TranscriptUnit {
    fn byte_len(&self) -> usize {
        self.output_text.len()
    }
}

/// Bounded portal update materialization returned to a daemon adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalTranscriptUpdate {
    pub projection_id: String,
    pub visible_transcript: Vec<TranscriptUnit>,
    pub visible_transcript_bytes: usize,
    pub coalesced_output_count: usize,
    pub unread_output_count: usize,
}

/// Compact memory-only state summary that excludes transcript text, pending
/// input text, owner tokens, and lease credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionStateSummary {
    pub projection_id: String,
    pub lifecycle_state: ProjectionLifecycleState,
    pub content_classification: ContentClassification,
    pub has_hud_connection: bool,
    pub has_advisory_lease: bool,
    pub retained_transcript_bytes: usize,
    pub visible_transcript_bytes: usize,
    pub retained_transcript_units: usize,
    pub pending_input_count: usize,
    pub pending_input_bytes: usize,
    pub unread_output_count: usize,
    pub reconnect: ReconnectBookkeeping,
}

/// Common operation response envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionResponse {
    pub request_id: String,
    pub projection_id: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ProjectionErrorCode>,
    pub server_timestamp_wall_us: u64,
    pub status_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<ProjectionLifecycleState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_input: Vec<PendingInputItem>,
    #[serde(default)]
    pub pending_remaining_count: usize,
    #[serde(default)]
    pub pending_remaining_bytes: usize,
    #[serde(default)]
    pub portal_update_ready: bool,
    #[serde(default)]
    pub coalesced_output_count: usize,
}

impl ProjectionResponse {
    fn accepted(
        request_id: impl Into<String>,
        projection_id: impl Into<String>,
        server_timestamp_wall_us: u64,
        status_summary: impl Into<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            projection_id: projection_id.into(),
            accepted: true,
            error_code: None,
            server_timestamp_wall_us,
            status_summary: bounded_copy(status_summary.into(), MAX_STATUS_SUMMARY_BYTES),
            owner_token: None,
            lifecycle_state: None,
            pending_input: Vec::new(),
            pending_remaining_count: 0,
            pending_remaining_bytes: 0,
            portal_update_ready: false,
            coalesced_output_count: 0,
        }
    }

    fn denied(
        request_id: impl Into<String>,
        projection_id: impl Into<String>,
        server_timestamp_wall_us: u64,
        error_code: ProjectionErrorCode,
        status_summary: impl Into<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            projection_id: projection_id.into(),
            accepted: false,
            error_code: Some(error_code),
            server_timestamp_wall_us,
            status_summary: bounded_copy(status_summary.into(), MAX_STATUS_SUMMARY_BYTES),
            owner_token: None,
            lifecycle_state: None,
            pending_input: Vec::new(),
            pending_remaining_count: 0,
            pending_remaining_bytes: 0,
            portal_update_ready: false,
            coalesced_output_count: 0,
        }
    }

    fn with_portal_update_state(mut self, session: &ProjectionSession) -> Self {
        self.portal_update_ready = session.last_publish_portal_update_ready;
        self.coalesced_output_count = session.coalesced_portal_update_count;
        self
    }
}

/// Structured audit record. It intentionally excludes transcript text, HUD input
/// text, and owner/operator credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionAuditRecord {
    pub timestamp_wall_us: u64,
    pub operation: ProjectionOperation,
    pub projection_id: String,
    pub caller_identity: String,
    pub request_id: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ProjectionErrorCode>,
    pub reason: String,
    pub category: ProjectionAuditCategory,
}

/// Bounded identity metadata for a live projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionIdentitySummary {
    pub provider_kind: ProviderKind,
    pub display_name: String,
    pub content_classification: ContentClassification,
    pub lifecycle_state: ProjectionLifecycleState,
}

/// Viewer and runtime policy applied while materializing projected portal
/// state. Defaults fail closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedPortalPolicy {
    pub viewer_clearance: ContentClassification,
    pub reveal_identity: bool,
    pub reveal_lifecycle: bool,
    pub reveal_transcript: bool,
    pub reveal_unread: bool,
    pub reveal_pending_input: bool,
    pub allow_input: bool,
    pub safe_mode_active: bool,
    pub frozen: bool,
    pub dismissed: bool,
}

impl ProjectedPortalPolicy {
    /// Policy fixture that permits all private projected-session fields.
    pub fn permit_all() -> Self {
        Self {
            viewer_clearance: ContentClassification::Sensitive,
            reveal_identity: true,
            reveal_lifecycle: true,
            reveal_transcript: true,
            reveal_unread: true,
            reveal_pending_input: true,
            allow_input: true,
            safe_mode_active: false,
            frozen: false,
            dismissed: false,
        }
    }

    fn permits(&self, classification: ContentClassification) -> bool {
        classification <= self.viewer_clearance
    }
}

impl Default for ProjectedPortalPolicy {
    fn default() -> Self {
        Self {
            viewer_clearance: ContentClassification::Public,
            reveal_identity: false,
            reveal_lifecycle: false,
            reveal_transcript: false,
            reveal_unread: false,
            reveal_pending_input: false,
            allow_input: false,
            safe_mode_active: false,
            frozen: false,
            dismissed: false,
        }
    }
}

/// Bounded state for resident-session text-stream portal materialization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedPortalState {
    pub projection_id: String,
    pub portal_id: String,
    pub adapter_family: ProjectedPortalAdapterFamily,
    pub runtime_authority: ProjectedPortalRuntimeAuthority,
    pub layer: ProjectedPortalLayer,
    pub presentation: ProjectedPortalPresentation,
    pub preserve_geometry: bool,
    pub redacted: bool,
    pub interaction_enabled: bool,
    pub attention: ProjectedPortalAttention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_kind: Option<ProviderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_profile_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<ProjectionLifecycleState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(default)]
    pub visible_transcript: Vec<TranscriptUnit>,
    pub visible_transcript_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread_output_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_input_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_input_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_input_feedback: Option<PortalInputFeedback>,
}

/// How a provider-neutral LLM session entered projection authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionOrigin {
    /// Already-running session that opted in through the cooperative contract.
    Attached,
    /// Authority-supervised launch. This records intent and metadata; it is not
    /// terminal capture or PTY ownership.
    Launched(LaunchSessionSpec),
}

/// Redacted, provider-neutral launch metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSessionSpec {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment_keys: Vec<String>,
}

impl LaunchSessionSpec {
    fn validate(&self) -> Result<(), ProjectionContractError> {
        validate_non_empty_bounded("launch_command", &self.command, MAX_HINT_BYTES)?;
        for arg in &self.args {
            validate_non_empty_bounded("launch_arg", arg, MAX_HINT_BYTES)?;
        }
        validate_optional_bounded(
            "launch_working_directory",
            &self.working_directory,
            MAX_HINT_BYTES,
        )?;
        for key in &self.environment_keys {
            validate_non_empty_bounded("launch_environment_key", key, MAX_HINT_BYTES)?;
        }
        Ok(())
    }
}

/// Runtime credential source. Values are never stored here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum HudCredentialSource {
    EnvVar(String),
    ProtectedConfigKey(String),
}

impl HudCredentialSource {
    fn validate(&self) -> Result<(), ProjectionContractError> {
        match self {
            Self::EnvVar(name) => {
                validate_non_empty_bounded("credential_env_var", name, MAX_HINT_BYTES)
            }
            Self::ProtectedConfigKey(name) => {
                validate_non_empty_bounded("credential_config_key", name, MAX_HINT_BYTES)
            }
        }
    }

    fn redacted_marker(&self) -> String {
        match self {
            Self::EnvVar(name) => format!("env:{name}:redacted"),
            Self::ProtectedConfigKey(name) => format!("protected-config:{name}:redacted"),
        }
    }
}

/// Local Windows HUD runtime target metadata retained by the external authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsHudTarget {
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grpc_endpoint: Option<String>,
    pub credential_source: HudCredentialSource,
    pub runtime_audience: String,
}

impl WindowsHudTarget {
    fn validate(&self) -> Result<(), ProjectionContractError> {
        validate_non_empty_bounded("hud_target_id", &self.target_id, MAX_HINT_BYTES)?;
        validate_optional_bounded("mcp_url", &self.mcp_url, MAX_HINT_BYTES)?;
        validate_optional_bounded("grpc_endpoint", &self.grpc_endpoint, MAX_HINT_BYTES)?;
        if self.mcp_url.is_none() && self.grpc_endpoint.is_none() {
            return Err(ProjectionContractError::InvalidArgument(
                "Windows HUD target requires mcp_url or grpc_endpoint".to_string(),
            ));
        }
        self.credential_source.validate()?;
        validate_non_empty_bounded("runtime_audience", &self.runtime_audience, MAX_HINT_BYTES)
    }
}

/// Projection attention intent. V1 defaults to ambient presence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionAttentionIntent {
    #[default]
    Ambient,
    Gentle,
    Interruptive,
}

/// Surface class requested by an external managed session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "surface")]
pub enum PresenceSurfaceRoute {
    Zone {
        zone_name: String,
        content_kind: String,
        ttl_ms: u64,
    },
    Widget {
        widget_name: String,
        #[serde(default)]
        parameters: HashMap<String, WidgetParameterValue>,
        ttl_ms: u64,
    },
    Portal {
        #[serde(default)]
        requested_capabilities: Vec<String>,
        lease_ttl_ms: u64,
    },
}

impl PresenceSurfaceRoute {
    fn validate(&self) -> Result<(), ProjectionContractError> {
        match self {
            Self::Zone {
                zone_name,
                content_kind,
                ttl_ms,
            } => {
                validate_non_empty_bounded("zone_name", zone_name, MAX_HINT_BYTES)?;
                validate_non_empty_bounded("zone_content_kind", content_kind, MAX_HINT_BYTES)?;
                validate_non_zero("zone_ttl_ms", *ttl_ms)
            }
            Self::Widget {
                widget_name,
                parameters,
                ttl_ms,
            } => {
                validate_non_empty_bounded("widget_name", widget_name, MAX_HINT_BYTES)?;
                if parameters.is_empty() {
                    return Err(ProjectionContractError::InvalidArgument(
                        "widget route requires at least one parameter".to_string(),
                    ));
                }
                for key in parameters.keys() {
                    validate_non_empty_bounded("widget_parameter_name", key, MAX_HINT_BYTES)?;
                }
                validate_non_zero("widget_ttl_ms", *ttl_ms)
            }
            Self::Portal {
                requested_capabilities,
                lease_ttl_ms,
            } => {
                for capability in requested_capabilities {
                    validate_non_empty_bounded("portal_capability", capability, MAX_HINT_BYTES)?;
                }
                validate_non_zero("portal_lease_ttl_ms", *lease_ttl_ms)
            }
        }
    }
}

/// Bounded widget parameter value used by route plans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum WidgetParameterValue {
    F32Milli(i64),
    Text(String),
    ColorRgba([u8; 4]),
    Enum(String),
}

/// Request to register or update one managed external session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedSessionRequest {
    pub projection_id: String,
    pub provider_kind: ProviderKind,
    pub display_name: String,
    pub origin: ManagedSessionOrigin,
    pub hud_target_id: String,
    pub surface_route: PresenceSurfaceRoute,
    #[serde(default)]
    pub content_classification: ContentClassification,
    #[serde(default)]
    pub attention_intent: ProjectionAttentionIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_profile_hint: Option<String>,
}

impl ManagedSessionRequest {
    fn validate(&self) -> Result<(), ProjectionContractError> {
        validate_non_empty_bounded(
            "projection_id",
            &self.projection_id,
            MAX_PROJECTION_ID_BYTES,
        )?;
        validate_non_empty_bounded("display_name", &self.display_name, MAX_DISPLAY_NAME_BYTES)?;
        validate_non_empty_bounded("hud_target_id", &self.hud_target_id, MAX_HINT_BYTES)?;
        validate_optional_bounded("workspace_hint", &self.workspace_hint, MAX_HINT_BYTES)?;
        validate_optional_bounded("repository_hint", &self.repository_hint, MAX_HINT_BYTES)?;
        validate_optional_bounded("icon_profile_hint", &self.icon_profile_hint, MAX_HINT_BYTES)?;
        if let ManagedSessionOrigin::Launched(spec) = &self.origin {
            spec.validate()?;
        }
        self.surface_route.validate()
    }
}

/// Runtime-facing command plan for a managed session. This is advisory: the
/// runtime remains the final policy and capability authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum HudSurfaceCommandPlan {
    ZonePublish {
        zone_name: String,
        content_kind: String,
        ttl_ms: u64,
        agent_id: String,
    },
    WidgetPublish {
        widget_name: String,
        parameters: HashMap<String, WidgetParameterValue>,
        ttl_ms: u64,
        agent_id: String,
    },
    PortalLease {
        portal_id: String,
        requested_capabilities: Vec<String>,
        lease_ttl_ms: u64,
        agent_id: String,
    },
}

/// Bounded, redacted route plan suitable for audit and demo evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedSessionRoutePlan {
    pub projection_id: String,
    pub provider_kind: ProviderKind,
    pub display_name: String,
    pub origin: ManagedSessionOrigin,
    pub hud_target_id: String,
    pub runtime_audience: String,
    pub credential_redacted: String,
    pub lifecycle_state: ProjectionLifecycleState,
    pub content_classification: ContentClassification,
    pub attention_intent: ProjectionAttentionIntent,
    pub surface_command: HudSurfaceCommandPlan,
    pub cleanup_on_detach: bool,
}

/// Handle returned after registering a managed session. The owner token is
/// returned only to the caller and is intentionally absent from route plans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedSessionHandle {
    pub route_plan: ManagedSessionRoutePlan,
    pub owner_token: String,
}

#[derive(Clone, Debug)]
struct ManagedSessionRecord {
    route_plan: ManagedSessionRoutePlan,
}

/// External authority layer for launched/attached provider-neutral sessions.
#[derive(Debug)]
pub struct ExternalAgentProjectionAuthority {
    projection_authority: ProjectionAuthority,
    targets: HashMap<String, WindowsHudTarget>,
    managed_sessions: HashMap<String, ManagedSessionRecord>,
}

impl ExternalAgentProjectionAuthority {
    pub fn new(bounds: ProjectionBounds) -> Result<Self, ProjectionContractError> {
        Ok(Self {
            projection_authority: ProjectionAuthority::new(bounds)?,
            targets: HashMap::new(),
            managed_sessions: HashMap::new(),
        })
    }

    pub fn projection_authority(&self) -> &ProjectionAuthority {
        &self.projection_authority
    }

    pub fn projection_authority_mut(&mut self) -> &mut ProjectionAuthority {
        &mut self.projection_authority
    }

    pub fn register_windows_target(
        &mut self,
        target: WindowsHudTarget,
    ) -> Result<(), ProjectionContractError> {
        target.validate()?;
        self.targets.insert(target.target_id.clone(), target);
        Ok(())
    }

    pub fn manage_session(
        &mut self,
        request: ManagedSessionRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> Result<ManagedSessionHandle, ProjectionErrorCode> {
        request.validate().map_err(|error| error.code())?;
        let target = self
            .targets
            .get(&request.hud_target_id)
            .ok_or(ProjectionErrorCode::ProjectionInvalidArgument)?
            .clone();

        let attach = AttachRequest {
            envelope: OperationEnvelope {
                operation: ProjectionOperation::Attach,
                projection_id: request.projection_id.clone(),
                request_id: format!("manage-{}", request.projection_id),
                client_timestamp_wall_us: server_timestamp_wall_us,
            },
            provider_kind: request.provider_kind.clone(),
            display_name: request.display_name.clone(),
            workspace_hint: request.workspace_hint.clone(),
            repository_hint: request.repository_hint.clone(),
            icon_profile_hint: request.icon_profile_hint.clone(),
            content_classification: request.content_classification,
            hud_target: Some(target.target_id.clone()),
            idempotency_key: Some(format!("managed-{}", request.projection_id)),
        };
        let response = self.projection_authority.handle_attach(
            attach,
            caller_identity,
            server_timestamp_wall_us,
        );
        if !response.accepted {
            return Err(response
                .error_code
                .unwrap_or(ProjectionErrorCode::ProjectionInternalError));
        }
        let owner_token = response
            .owner_token
            .ok_or(ProjectionErrorCode::ProjectionAlreadyAttached)?;
        let route_plan = route_plan_for_request(&request, &target);
        self.managed_sessions.insert(
            request.projection_id.clone(),
            ManagedSessionRecord {
                route_plan: route_plan.clone(),
            },
        );
        Ok(ManagedSessionHandle {
            route_plan,
            owner_token,
        })
    }

    pub fn route_plan(&self, projection_id: &str) -> Option<&ManagedSessionRoutePlan> {
        self.managed_sessions
            .get(projection_id)
            .map(|record| &record.route_plan)
    }

    pub fn managed_session_count(&self) -> usize {
        self.managed_sessions.len()
    }

    pub fn revoke_session(&mut self, projection_id: &str) -> Result<(), ProjectionErrorCode> {
        self.managed_sessions
            .remove(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        self.projection_authority.expire_projection(projection_id);
        Ok(())
    }

    pub fn expire_token_expired_sessions(&mut self, server_timestamp_wall_us: u64) -> usize {
        let expired_count = self
            .projection_authority
            .expire_token_expired_projections(server_timestamp_wall_us);
        self.managed_sessions
            .retain(|projection_id, _| self.projection_authority.has_projection(projection_id));
        expired_count
    }

    pub fn mark_hud_disconnected(
        &mut self,
        projection_id: &str,
        disconnected_at_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        self.projection_authority
            .mark_hud_disconnected(projection_id, disconnected_at_wall_us)
    }

    pub fn record_hud_connection(
        &mut self,
        projection_id: &str,
        metadata: HudConnectionMetadata,
    ) -> Result<(), ProjectionErrorCode> {
        self.projection_authority
            .record_hud_connection(projection_id, metadata)
    }

    pub fn three_session_demo_plan(&self) -> Vec<ManagedSessionRoutePlan> {
        let mut plans: Vec<_> = self
            .managed_sessions
            .values()
            .map(|record| record.route_plan.clone())
            .collect();
        plans.sort_by(|left, right| left.projection_id.cmp(&right.projection_id));
        plans
    }
}

impl Default for ExternalAgentProjectionAuthority {
    fn default() -> Self {
        Self::new(ProjectionBounds::default()).expect("default bounds are valid")
    }
}

/// Errors raised by schema validation or token generation.
#[derive(Debug, Error)]
pub enum ProjectionContractError {
    #[error("invalid projection argument: {0}")]
    InvalidArgument(String),
    #[error("{0}: {1}")]
    StableCode(ProjectionErrorCode, String),
    #[error("unable to generate owner token")]
    TokenGeneration,
}

impl ProjectionContractError {
    fn code(&self) -> ProjectionErrorCode {
        match self {
            Self::InvalidArgument(_) => ProjectionErrorCode::ProjectionInvalidArgument,
            Self::StableCode(code, _) => *code,
            Self::TokenGeneration => ProjectionErrorCode::ProjectionInternalError,
        }
    }
}

#[derive(Clone, Debug)]
struct ProjectionSession {
    projection_id: String,
    provider_kind: ProviderKind,
    display_name: String,
    workspace_hint: Option<String>,
    repository_hint: Option<String>,
    icon_profile_hint: Option<String>,
    portal_id: String,
    portal_presentation: ProjectedPortalPresentation,
    owner_token_verifier: String,
    owner_token_expires_at_wall_us: u64,
    lifecycle_state: ProjectionLifecycleState,
    latest_status_text: Option<String>,
    content_classification: ContentClassification,
    attach_idempotency_key: Option<String>,
    hud_connection: Option<HudConnectionMetadata>,
    advisory_lease: Option<AdvisoryLeaseIdentity>,
    reconnect: ReconnectBookkeeping,
    retained_transcript: VecDeque<TranscriptUnit>,
    retained_transcript_bytes: usize,
    next_transcript_sequence: u64,
    unread_output_count: usize,
    portal_rate_window_started_at_wall_us: u64,
    portal_updates_in_window: u32,
    coalesced_portal_update_count: usize,
    last_publish_portal_update_ready: bool,
    seen_logical_units: HashSet<String>,
    seen_logical_unit_order: VecDeque<String>,
    completed_input_ack_states: HashMap<String, InputDeliveryState>,
    completed_input_ack_order: VecDeque<String>,
    pending_input: VecDeque<PendingInputItem>,
    pending_input_bytes: usize,
    last_input_feedback: Option<PortalInputFeedback>,
    portal_update_pending: bool,
}

struct ProjectionAuditEvent<'a> {
    envelope: &'a OperationEnvelope,
    caller_identity: &'a str,
    server_timestamp_wall_us: u64,
    accepted: bool,
    error_code: Option<ProjectionErrorCode>,
    reason: &'a str,
    category: ProjectionAuditCategory,
}

/// Minimal in-memory authority that enforces the operation contract. Production
/// daemon storage can wrap or replace this, but must preserve these semantics.
#[derive(Debug)]
pub struct ProjectionAuthority {
    bounds: ProjectionBounds,
    sessions: HashMap<String, ProjectionSession>,
    operator_authority_verifier: Option<String>,
    audit_log: Vec<ProjectionAuditRecord>,
}

impl ProjectionAuthority {
    pub fn new(bounds: ProjectionBounds) -> Result<Self, ProjectionContractError> {
        bounds.validate()?;
        Ok(Self {
            bounds,
            sessions: HashMap::new(),
            operator_authority_verifier: None,
            audit_log: Vec::new(),
        })
    }

    /// Configure a separate operator authority credential for operator cleanup.
    pub fn set_operator_authority(
        &mut self,
        credential: &str,
    ) -> Result<(), ProjectionContractError> {
        validate_non_empty_bounded("operator_authority", credential, MAX_HINT_BYTES)?;
        self.operator_authority_verifier = Some(verifier_for_secret(credential));
        Ok(())
    }

    pub fn bounds(&self) -> &ProjectionBounds {
        &self.bounds
    }

    pub fn audit_log(&self) -> &[ProjectionAuditRecord] {
        &self.audit_log
    }

    pub fn has_projection(&self, projection_id: &str) -> bool {
        self.sessions.contains_key(projection_id)
    }

    pub fn projection_identity(&self, projection_id: &str) -> Option<ProjectionIdentitySummary> {
        self.sessions
            .get(projection_id)
            .map(|session| ProjectionIdentitySummary {
                provider_kind: session.provider_kind.clone(),
                display_name: session.display_name.clone(),
                content_classification: session.content_classification,
                lifecycle_state: session.lifecycle_state,
            })
    }

    pub fn state_summary(&self, projection_id: &str) -> Option<ProjectionStateSummary> {
        self.sessions.get(projection_id).map(|session| {
            let visible_transcript_bytes =
                visible_transcript_window(session, self.bounds.max_visible_transcript_bytes)
                    .iter()
                    .map(TranscriptUnit::byte_len)
                    .sum();
            ProjectionStateSummary {
                projection_id: projection_id.to_string(),
                lifecycle_state: session.lifecycle_state,
                content_classification: session.content_classification,
                has_hud_connection: session.hud_connection.is_some(),
                has_advisory_lease: session.advisory_lease.is_some(),
                retained_transcript_bytes: session.retained_transcript_bytes,
                visible_transcript_bytes,
                retained_transcript_units: session.retained_transcript.len(),
                pending_input_count: session
                    .pending_input
                    .iter()
                    .filter(|item| !item.delivery_state.is_terminal())
                    .count(),
                pending_input_bytes: session.pending_input_bytes,
                unread_output_count: session.unread_output_count,
                reconnect: session.reconnect,
            }
        })
    }

    pub fn visible_transcript_window(&self, projection_id: &str) -> Option<Vec<TranscriptUnit>> {
        self.sessions.get(projection_id).map(|session| {
            visible_transcript_window(session, self.bounds.max_visible_transcript_bytes)
        })
    }

    /// Materialize the bounded text-stream portal state for a projected
    /// session. This returns data for an external daemon/resident-session
    /// adapter; it does not expose runtime scene state or process authority.
    pub fn projected_portal_state(
        &self,
        projection_id: &str,
        policy: &ProjectedPortalPolicy,
    ) -> Option<ProjectedPortalState> {
        self.sessions.get(projection_id).map(|session| {
            projected_portal_state(session, policy, self.bounds.max_visible_transcript_bytes)
        })
    }

    /// Collapse a projected portal into its compact content-layer surface.
    pub fn collapse_projected_portal(
        &mut self,
        projection_id: &str,
    ) -> Result<(), ProjectionErrorCode> {
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        session.portal_presentation = ProjectedPortalPresentation::Collapsed;
        Ok(())
    }

    /// Expand a projected portal back to its transcript/composer surface.
    pub fn expand_projected_portal(
        &mut self,
        projection_id: &str,
    ) -> Result<(), ProjectionErrorCode> {
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        session.portal_presentation = ProjectedPortalPresentation::Expanded;
        Ok(())
    }

    pub fn record_hud_connection(
        &mut self,
        projection_id: &str,
        metadata: HudConnectionMetadata,
    ) -> Result<(), ProjectionErrorCode> {
        metadata.validate().map_err(|error| error.code())?;
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let is_reconnect = session
            .reconnect
            .last_reconnect_wall_us
            .is_some_and(|last| last < metadata.last_reconnect_wall_us);
        let connection_changed = session.hud_connection.as_ref().is_some_and(|connection| {
            connection.connection_id != metadata.connection_id
                || connection.authenticated_session_id != metadata.authenticated_session_id
        });
        if is_reconnect {
            session.reconnect.reconnect_count += 1;
        }
        session.reconnect.last_reconnect_wall_us = Some(metadata.last_reconnect_wall_us);
        if is_reconnect || connection_changed {
            session.advisory_lease = None;
        }
        session.hud_connection = Some(metadata);
        promote_to_active_if_recovering(session);
        Ok(())
    }

    pub fn mark_hud_disconnected(
        &mut self,
        projection_id: &str,
        disconnected_at_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        if disconnected_at_wall_us == 0 {
            return Err(ProjectionErrorCode::ProjectionInvalidArgument);
        }
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        session.hud_connection = None;
        session.advisory_lease = None;
        session.reconnect.last_disconnect_wall_us = Some(disconnected_at_wall_us);
        session.lifecycle_state = ProjectionLifecycleState::HudUnavailable;
        Ok(())
    }

    pub fn record_heartbeat(
        &mut self,
        projection_id: &str,
        heartbeat_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        if heartbeat_wall_us == 0 {
            return Err(ProjectionErrorCode::ProjectionInvalidArgument);
        }
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        if session.hud_connection.is_none() {
            return Err(ProjectionErrorCode::ProjectionHudUnavailable);
        }
        if session
            .reconnect
            .last_heartbeat_wall_us
            .is_some_and(|last| heartbeat_wall_us < last)
        {
            return Err(ProjectionErrorCode::ProjectionStateConflict);
        }
        session.reconnect.last_heartbeat_wall_us = Some(heartbeat_wall_us);
        promote_to_active_if_recovering(session);
        Ok(())
    }

    pub fn record_advisory_lease(
        &mut self,
        projection_id: &str,
        lease: AdvisoryLeaseIdentity,
        server_timestamp_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        lease.validate().map_err(|error| error.code())?;
        if server_timestamp_wall_us >= lease.expires_at_wall_us {
            return Err(ProjectionErrorCode::ProjectionTokenExpired);
        }
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let Some(connection) = session.hud_connection.as_ref() else {
            return Err(ProjectionErrorCode::ProjectionHudUnavailable);
        };
        if !capabilities_are_subset(&lease.capabilities, &connection.granted_capabilities) {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        session.advisory_lease = Some(lease);
        Ok(())
    }

    pub fn authorize_portal_republish(
        &mut self,
        projection_id: &str,
        lease_id: &str,
        requested_capabilities: &[String],
        server_timestamp_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        validate_non_empty_bounded("lease_id", lease_id, MAX_HINT_BYTES)
            .map_err(|error| error.code())?;
        for capability in requested_capabilities {
            validate_non_empty_bounded("requested_capability", capability, MAX_HINT_BYTES)
                .map_err(|error| error.code())?;
        }

        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let Some(connection) = session.hud_connection.as_ref() else {
            session.advisory_lease = None;
            return Err(ProjectionErrorCode::ProjectionHudUnavailable);
        };
        if !capabilities_are_subset(requested_capabilities, &connection.granted_capabilities) {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        let Some(lease) = session.advisory_lease.as_ref() else {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        };
        if server_timestamp_wall_us >= lease.expires_at_wall_us {
            session.advisory_lease = None;
            return Err(ProjectionErrorCode::ProjectionTokenExpired);
        }
        if lease.lease_id != lease_id {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        if !capabilities_are_subset(requested_capabilities, &lease.capabilities) {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        Ok(())
    }

    pub fn take_due_portal_update(
        &mut self,
        projection_id: &str,
        server_timestamp_wall_us: u64,
    ) -> Result<Option<PortalTranscriptUpdate>, ProjectionErrorCode> {
        let max_updates = self.bounds.max_portal_updates_per_second;
        let max_visible = self.bounds.max_visible_transcript_bytes;
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        if session.unread_output_count == 0 {
            return Ok(None);
        }
        if !session.portal_update_pending {
            if !portal_update_allowed(session, server_timestamp_wall_us, max_updates) {
                return Ok(None);
            }
            session.portal_update_pending = true;
        }
        let visible_transcript = visible_transcript_window(session, max_visible);
        let visible_transcript_bytes = visible_transcript
            .iter()
            .map(TranscriptUnit::byte_len)
            .sum();
        let coalesced_output_count = session.coalesced_portal_update_count;
        let unread_output_count = session.unread_output_count;
        session.coalesced_portal_update_count = 0;
        session.unread_output_count = 0;
        session.portal_update_pending = false;
        session.last_publish_portal_update_ready = false;
        Ok(Some(PortalTranscriptUpdate {
            projection_id: projection_id.to_string(),
            visible_transcript,
            visible_transcript_bytes,
            coalesced_output_count,
            unread_output_count,
        }))
    }

    pub fn expire_projection(&mut self, projection_id: &str) -> bool {
        self.sessions.remove(projection_id).is_some()
    }

    pub fn expire_token_expired_projections(&mut self, server_timestamp_wall_us: u64) -> usize {
        let before = self.sessions.len();
        self.sessions
            .retain(|_, session| server_timestamp_wall_us < session.owner_token_expires_at_wall_us);
        before - self.sessions.len()
    }

    pub fn owner_token_verifier_for_test(&self, projection_id: &str) -> Option<&str> {
        self.sessions
            .get(projection_id)
            .map(|session| session.owner_token_verifier.as_str())
    }

    pub fn handle_attach(
        &mut self,
        request: AttachRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        if let Err(error) = validate_non_empty_bounded(
            "caller_identity",
            caller_identity,
            MAX_CALLER_IDENTITY_BYTES,
        ) {
            return self.validation_denial(
                &request.envelope,
                "invalid-caller",
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::AuthDenied,
            );
        }

        if let Some(existing) = self.sessions.get(&request.envelope.projection_id) {
            if request.idempotency_key.is_some()
                && request.idempotency_key == existing.attach_idempotency_key
            {
                let mut response = ProjectionResponse::accepted(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    "projection already attached for matching idempotency key",
                );
                response.lifecycle_state = Some(existing.lifecycle_state);
                self.audit(ProjectionAuditEvent {
                    envelope: &request.envelope,
                    caller_identity,
                    server_timestamp_wall_us,
                    accepted: true,
                    error_code: None,
                    reason: "idempotent attach replay",
                    category: ProjectionAuditCategory::Attach,
                });
                return response;
            }
            let response = ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                ProjectionErrorCode::ProjectionAlreadyAttached,
                "projection_id is already attached",
            );
            self.audit(ProjectionAuditEvent {
                envelope: &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                accepted: false,
                error_code: Some(ProjectionErrorCode::ProjectionAlreadyAttached),
                reason: "attach conflict",
                category: ProjectionAuditCategory::ConflictDenied,
            });
            return response;
        }

        let owner_token = match generate_owner_token() {
            Ok(token) => token,
            Err(error) => {
                return self.validation_denial(
                    &request.envelope,
                    caller_identity,
                    server_timestamp_wall_us,
                    error,
                    ProjectionAuditCategory::AuthDenied,
                );
            }
        };
        let owner_token_verifier = verifier_for_secret(&owner_token);
        self.sessions.insert(
            request.envelope.projection_id.clone(),
            ProjectionSession {
                projection_id: request.envelope.projection_id.clone(),
                provider_kind: request.provider_kind,
                display_name: request.display_name,
                workspace_hint: request.workspace_hint,
                repository_hint: request.repository_hint,
                icon_profile_hint: request.icon_profile_hint,
                portal_id: portal_id_for_projection(&request.envelope.projection_id),
                portal_presentation: ProjectedPortalPresentation::Expanded,
                owner_token_verifier,
                owner_token_expires_at_wall_us: server_timestamp_wall_us
                    + self.bounds.owner_token_ttl_wall_us,
                lifecycle_state: ProjectionLifecycleState::Attached,
                latest_status_text: None,
                content_classification: request.content_classification,
                attach_idempotency_key: request.idempotency_key,
                hud_connection: None,
                advisory_lease: None,
                reconnect: ReconnectBookkeeping::default(),
                retained_transcript: VecDeque::new(),
                retained_transcript_bytes: 0,
                next_transcript_sequence: 0,
                unread_output_count: 0,
                portal_rate_window_started_at_wall_us: 0,
                portal_updates_in_window: 0,
                coalesced_portal_update_count: 0,
                last_publish_portal_update_ready: false,
                seen_logical_units: HashSet::new(),
                seen_logical_unit_order: VecDeque::new(),
                completed_input_ack_states: HashMap::new(),
                completed_input_ack_order: VecDeque::new(),
                pending_input: VecDeque::new(),
                pending_input_bytes: 0,
                last_input_feedback: None,
                portal_update_pending: false,
            },
        );

        let mut response = ProjectionResponse::accepted(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            "projection attached",
        );
        response.owner_token = Some(owner_token);
        response.lifecycle_state = Some(ProjectionLifecycleState::Attached);
        self.audit(ProjectionAuditEvent {
            envelope: &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            accepted: true,
            error_code: None,
            reason: "attach accepted",
            category: ProjectionAuditCategory::Attach,
        });
        response
    }

    pub fn handle_publish_output(
        &mut self,
        request: PublishOutputRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate(&self.bounds) {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let max_retained_transcript_bytes = self.bounds.max_retained_transcript_bytes;
        let max_seen_logical_units = self.bounds.max_seen_logical_units;
        let max_visible_transcript_bytes = self.bounds.max_visible_transcript_bytes;
        let max_portal_updates_per_second = self.bounds.max_portal_updates_per_second;
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerPublish,
        ) {
            Ok(session) => {
                if let Some(logical_unit_id) = &request.logical_unit_id {
                    if remember_logical_unit(session, logical_unit_id, max_seen_logical_units) {
                        ProjectionResponse::accepted(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            "duplicate logical_unit_id accepted idempotently",
                        )
                    } else {
                        append_transcript_unit(
                            session,
                            &request,
                            server_timestamp_wall_us,
                            max_retained_transcript_bytes,
                            max_visible_transcript_bytes,
                            max_portal_updates_per_second,
                        );
                        let mut response = ProjectionResponse::accepted(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            "output accepted",
                        )
                        .with_portal_update_state(session);
                        response.status_summary = if response.portal_update_ready {
                            "output accepted".to_string()
                        } else {
                            "output accepted and coalesced for next portal update".to_string()
                        };
                        response
                    }
                } else {
                    append_transcript_unit(
                        session,
                        &request,
                        server_timestamp_wall_us,
                        max_retained_transcript_bytes,
                        max_visible_transcript_bytes,
                        max_portal_updates_per_second,
                    );
                    let mut response = ProjectionResponse::accepted(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        "output accepted",
                    )
                    .with_portal_update_state(session);
                    response.status_summary = if response.portal_update_ready {
                        "output accepted".to_string()
                    } else {
                        "output accepted and coalesced for next portal update".to_string()
                    };
                    response
                }
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerPublish
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn handle_publish_status(
        &mut self,
        request: PublishStatusRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate(&self.bounds) {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerStatus,
        ) {
            Ok(session) => {
                session.lifecycle_state = request.lifecycle_state;
                session.latest_status_text = request.status_text;
                let mut response = ProjectionResponse::accepted(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    "status accepted",
                );
                response.lifecycle_state = Some(session.lifecycle_state);
                response
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerStatus
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn enqueue_input(
        &mut self,
        projection_id: &str,
        input_id: &str,
        submission_text: String,
        submitted_at_wall_us: u64,
        expires_at_wall_us: u64,
        content_classification: Option<ContentClassification>,
    ) -> Result<(), ProjectionErrorCode> {
        let item = PendingInputItem {
            input_id: input_id.to_string(),
            projection_id: projection_id.to_string(),
            submission_text,
            submitted_at_wall_us,
            expires_at_wall_us,
            delivery_state: InputDeliveryState::Pending,
            delivered_at_wall_us: None,
            not_before_wall_us: None,
            content_classification: content_classification.unwrap_or_default(),
        };
        self.enqueue_input_item(projection_id, item)
    }

    /// Submit HUD composer text into the cooperative pending-input inbox and
    /// return bounded local-first feedback for the portal surface.
    pub fn submit_portal_input(
        &mut self,
        projection_id: &str,
        submission: PortalInputSubmission,
    ) -> PortalInputFeedback {
        let input_id = submission.input_id.clone();
        let result = match submission.effective_expires_at_wall_us() {
            Ok(expires_at_wall_us) => self.enqueue_input_item(
                projection_id,
                PendingInputItem {
                    input_id: submission.input_id,
                    projection_id: projection_id.to_string(),
                    submission_text: submission.submission_text,
                    submitted_at_wall_us: submission.submitted_at_wall_us,
                    expires_at_wall_us,
                    delivery_state: InputDeliveryState::Pending,
                    delivered_at_wall_us: None,
                    not_before_wall_us: None,
                    content_classification: submission.content_classification,
                },
            ),
            Err(code) => Err(code),
        };

        let (pending_input_count, pending_input_bytes) = self
            .state_summary(projection_id)
            .map(|summary| (summary.pending_input_count, summary.pending_input_bytes))
            .unwrap_or_default();
        let feedback = match result {
            Ok(()) => PortalInputFeedback {
                projection_id: projection_id.to_string(),
                input_id,
                feedback_state: PortalInputFeedbackState::Accepted,
                error_code: None,
                pending_input_count,
                pending_input_bytes,
                status_summary: "portal input accepted".to_string(),
            },
            Err(code) => PortalInputFeedback {
                projection_id: projection_id.to_string(),
                input_id,
                feedback_state: PortalInputFeedbackState::Rejected,
                error_code: Some(code),
                pending_input_count,
                pending_input_bytes,
                status_summary: format!("{code}: portal input rejected"),
            },
        };
        if let Some(session) = self.sessions.get_mut(projection_id) {
            session.last_input_feedback = Some(feedback.clone());
        }
        feedback
    }

    fn enqueue_input_item(
        &mut self,
        projection_id: &str,
        item: PendingInputItem,
    ) -> Result<(), ProjectionErrorCode> {
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        validate_pending_input_item(&item, &self.bounds)?;
        prune_terminal_pending_input(session, self.bounds.max_pending_input_items);
        if session
            .pending_input
            .iter()
            .any(|pending| pending.input_id == item.input_id)
            || session
                .completed_input_ack_states
                .contains_key(&item.input_id)
        {
            return Err(ProjectionErrorCode::ProjectionStateConflict);
        }
        if item.submission_text.len() > self.bounds.max_pending_input_bytes_per_item {
            return Err(ProjectionErrorCode::ProjectionInputTooLarge);
        }
        if session.pending_input.len() >= self.bounds.max_pending_input_items {
            return Err(ProjectionErrorCode::ProjectionInputQueueFull);
        }
        if session.pending_input_bytes + item.submission_text.len()
            > self.bounds.max_pending_input_total_bytes
        {
            return Err(ProjectionErrorCode::ProjectionInputQueueFull);
        }
        session.pending_input_bytes += item.submission_text.len();
        session.pending_input.push_back(item);
        Ok(())
    }

    pub fn handle_get_pending_input(
        &mut self,
        request: GetPendingInputRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let max_items = request
            .max_items
            .unwrap_or(self.bounds.max_poll_items)
            .min(self.bounds.max_poll_items);
        let max_bytes = request
            .max_bytes
            .unwrap_or(self.bounds.max_poll_response_bytes)
            .min(self.bounds.max_poll_response_bytes);
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerInputRead,
        ) {
            Ok(session) => {
                expire_pending(session, server_timestamp_wall_us);
                let mut used_bytes = 0usize;
                let mut returned = Vec::new();
                let mut remaining_count = 0usize;
                let mut remaining_bytes = 0usize;
                for item in session.pending_input.iter_mut() {
                    if !matches!(
                        item.delivery_state,
                        InputDeliveryState::Pending | InputDeliveryState::Deferred
                    ) {
                        continue;
                    }
                    if item.delivery_state == InputDeliveryState::Deferred
                        && item
                            .not_before_wall_us
                            .is_some_and(|not_before| server_timestamp_wall_us < not_before)
                    {
                        continue;
                    }
                    let item_bytes = item.submission_text.len();
                    if returned.len() < max_items && used_bytes + item_bytes <= max_bytes {
                        item.delivery_state = InputDeliveryState::Delivered;
                        item.delivered_at_wall_us = Some(server_timestamp_wall_us);
                        used_bytes += item_bytes;
                        returned.push(item.clone());
                    } else {
                        remaining_count += 1;
                        remaining_bytes += item_bytes;
                    }
                }
                let mut response = ProjectionResponse::accepted(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    "pending input returned",
                );
                response.pending_input = returned;
                response.pending_remaining_count = remaining_count;
                response.pending_remaining_bytes = remaining_bytes;
                response
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerInputRead
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn handle_acknowledge_input(
        &mut self,
        request: AcknowledgeInputRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerInputAck,
        ) {
            Ok(session) => acknowledge_input(session, &request, server_timestamp_wall_us),
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerInputAck
            } else if response.error_code == Some(ProjectionErrorCode::ProjectionStateConflict) {
                ProjectionAuditCategory::ConflictDenied
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn handle_detach(
        &mut self,
        request: DetachRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerDetach,
        ) {
            Ok(_) => {
                self.sessions.remove(&request.envelope.projection_id);
                ProjectionResponse::accepted(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    "projection detached and private state purged",
                )
            }
            Err(code) => ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                code,
                "owner authorization failed",
            ),
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            if response.accepted {
                ProjectionAuditCategory::OwnerDetach
            } else {
                ProjectionAuditCategory::AuthDenied
            },
        );
        response
    }

    pub fn handle_cleanup(
        &mut self,
        request: CleanupRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> ProjectionResponse {
        if let Err(error) = request.validate() {
            return self.validation_denial(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                error,
                ProjectionAuditCategory::BoundsDenied,
            );
        }

        let response = match request.cleanup_authority {
            CleanupAuthority::Owner => {
                let owner_token = request.owner_token.as_deref().unwrap_or_default();
                match self.authorize_owner(
                    &request.envelope,
                    owner_token,
                    server_timestamp_wall_us,
                    ProjectionAuditCategory::OwnerCleanup,
                ) {
                    Ok(_) => {
                        self.sessions.remove(&request.envelope.projection_id);
                        ProjectionResponse::accepted(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            "owner cleanup purged projection state",
                        )
                    }
                    Err(code) => ProjectionResponse::denied(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        code,
                        "owner authorization failed",
                    ),
                }
            }
            CleanupAuthority::Operator => {
                let credential = request.operator_authority.as_deref().unwrap_or_default();
                if self
                    .operator_authority_verifier
                    .as_deref()
                    .is_some_and(|verifier| {
                        constant_time_eq(verifier, &verifier_for_secret(credential))
                    })
                {
                    if self
                        .sessions
                        .remove(&request.envelope.projection_id)
                        .is_some()
                    {
                        ProjectionResponse::accepted(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            "operator cleanup purged projection state",
                        )
                    } else {
                        ProjectionResponse::denied(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            ProjectionErrorCode::ProjectionNotFound,
                            "projection not found",
                        )
                    }
                } else {
                    ProjectionResponse::denied(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        ProjectionErrorCode::ProjectionUnauthorized,
                        "operator authority failed",
                    )
                }
            }
        };
        self.audit_from_response(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            match (response.accepted, request.cleanup_authority) {
                (true, CleanupAuthority::Owner) => ProjectionAuditCategory::OwnerCleanup,
                (true, CleanupAuthority::Operator) => ProjectionAuditCategory::OperatorCleanup,
                (false, _) => ProjectionAuditCategory::AuthDenied,
            },
        );
        response
    }

    fn authorize_owner(
        &mut self,
        envelope: &OperationEnvelope,
        owner_token: &str,
        server_timestamp_wall_us: u64,
        _category: ProjectionAuditCategory,
    ) -> Result<&mut ProjectionSession, ProjectionErrorCode> {
        if self
            .sessions
            .get(&envelope.projection_id)
            .is_some_and(|session| {
                server_timestamp_wall_us >= session.owner_token_expires_at_wall_us
            })
        {
            self.sessions.remove(&envelope.projection_id);
            return Err(ProjectionErrorCode::ProjectionTokenExpired);
        }
        let session = self
            .sessions
            .get_mut(&envelope.projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let presented = verifier_for_secret(owner_token);
        if !constant_time_eq(&session.owner_token_verifier, &presented) {
            return Err(ProjectionErrorCode::ProjectionUnauthorized);
        }
        Ok(session)
    }

    fn validation_denial(
        &mut self,
        envelope: &OperationEnvelope,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
        error: ProjectionContractError,
        category: ProjectionAuditCategory,
    ) -> ProjectionResponse {
        let code = error.code();
        let response = ProjectionResponse::denied(
            &envelope.request_id,
            &envelope.projection_id,
            server_timestamp_wall_us,
            code,
            error.to_string(),
        );
        self.audit_from_response(
            envelope,
            caller_identity,
            server_timestamp_wall_us,
            &response,
            category,
        );
        response
    }

    fn audit_from_response(
        &mut self,
        envelope: &OperationEnvelope,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
        response: &ProjectionResponse,
        category: ProjectionAuditCategory,
    ) {
        self.audit(ProjectionAuditEvent {
            envelope,
            caller_identity,
            server_timestamp_wall_us,
            accepted: response.accepted,
            error_code: response.error_code,
            reason: &response.status_summary,
            category,
        });
    }

    fn audit(&mut self, event: ProjectionAuditEvent<'_>) {
        self.audit_log.push(ProjectionAuditRecord {
            timestamp_wall_us: event.server_timestamp_wall_us,
            operation: event.envelope.operation,
            projection_id: event.envelope.projection_id.clone(),
            caller_identity: bounded_copy(
                event.caller_identity.to_string(),
                MAX_CALLER_IDENTITY_BYTES,
            ),
            request_id: event.envelope.request_id.clone(),
            accepted: event.accepted,
            error_code: event.error_code,
            reason: bounded_copy(event.reason.to_string(), MAX_REASON_BYTES),
            category: event.category,
        });
        if self.audit_log.len() > self.bounds.max_audit_records {
            let overflow = self.audit_log.len() - self.bounds.max_audit_records;
            self.audit_log.drain(0..overflow);
        }
    }
}

impl Default for ProjectionAuthority {
    fn default() -> Self {
        Self::new(ProjectionBounds::default()).expect("default projection bounds are valid")
    }
}

fn route_plan_for_request(
    request: &ManagedSessionRequest,
    target: &WindowsHudTarget,
) -> ManagedSessionRoutePlan {
    let agent_id = format!("projection:{}", request.projection_id);
    let surface_command = match &request.surface_route {
        PresenceSurfaceRoute::Zone {
            zone_name,
            content_kind,
            ttl_ms,
        } => HudSurfaceCommandPlan::ZonePublish {
            zone_name: zone_name.clone(),
            content_kind: content_kind.clone(),
            ttl_ms: *ttl_ms,
            agent_id,
        },
        PresenceSurfaceRoute::Widget {
            widget_name,
            parameters,
            ttl_ms,
        } => HudSurfaceCommandPlan::WidgetPublish {
            widget_name: widget_name.clone(),
            parameters: parameters.clone(),
            ttl_ms: *ttl_ms,
            agent_id,
        },
        PresenceSurfaceRoute::Portal {
            requested_capabilities,
            lease_ttl_ms,
        } => HudSurfaceCommandPlan::PortalLease {
            portal_id: portal_id_for_projection(&request.projection_id),
            requested_capabilities: requested_capabilities.clone(),
            lease_ttl_ms: *lease_ttl_ms,
            agent_id,
        },
    };

    ManagedSessionRoutePlan {
        projection_id: request.projection_id.clone(),
        provider_kind: request.provider_kind.clone(),
        display_name: request.display_name.clone(),
        origin: request.origin.clone(),
        hud_target_id: target.target_id.clone(),
        runtime_audience: target.runtime_audience.clone(),
        credential_redacted: target.credential_source.redacted_marker(),
        lifecycle_state: ProjectionLifecycleState::Attached,
        content_classification: request.content_classification,
        attention_intent: request.attention_intent,
        surface_command,
        cleanup_on_detach: true,
    }
}

fn projected_portal_state(
    session: &ProjectionSession,
    policy: &ProjectedPortalPolicy,
    max_visible_transcript_bytes: usize,
) -> ProjectedPortalState {
    let projection_visible = policy.permits(session.content_classification);
    let expanded = session.portal_presentation == ProjectedPortalPresentation::Expanded;
    let identity_visible = projection_visible && policy.reveal_identity;
    let lifecycle_visible = projection_visible && policy.reveal_lifecycle;
    let transcript_visible = expanded && projection_visible && policy.reveal_transcript;
    let unread_visible = projection_visible && policy.reveal_unread;
    let pending_visible = projection_visible && policy.reveal_pending_input;
    let redacted = !identity_visible || !lifecycle_visible || (expanded && !transcript_visible);
    let interaction_enabled = session.portal_presentation == ProjectedPortalPresentation::Expanded
        && projection_visible
        && policy.allow_input
        && !redacted
        && !policy.safe_mode_active
        && !policy.frozen
        && !policy.dismissed;
    let visible_transcript: Vec<TranscriptUnit> = if transcript_visible {
        visible_transcript_window(session, max_visible_transcript_bytes)
            .into_iter()
            .filter(|unit| policy.permits(unit.content_classification))
            .collect()
    } else {
        Vec::new()
    };
    let visible_transcript_bytes = visible_transcript
        .iter()
        .map(TranscriptUnit::byte_len)
        .sum();
    let pending_input_count = session
        .pending_input
        .iter()
        .filter(|item| !item.delivery_state.is_terminal())
        .count();

    ProjectedPortalState {
        projection_id: session.projection_id.clone(),
        portal_id: session.portal_id.clone(),
        adapter_family: ProjectedPortalAdapterFamily::CooperativeProjection,
        runtime_authority: ProjectedPortalRuntimeAuthority::ResidentSessionLease,
        layer: ProjectedPortalLayer::Content,
        presentation: session.portal_presentation,
        preserve_geometry: true,
        redacted,
        interaction_enabled,
        attention: ProjectedPortalAttention::Ambient,
        provider_kind: identity_visible.then(|| session.provider_kind.clone()),
        display_name: identity_visible.then(|| session.display_name.clone()),
        workspace_hint: identity_visible
            .then(|| session.workspace_hint.clone())
            .flatten(),
        repository_hint: identity_visible
            .then(|| session.repository_hint.clone())
            .flatten(),
        icon_profile_hint: identity_visible
            .then(|| session.icon_profile_hint.clone())
            .flatten(),
        lifecycle_state: lifecycle_visible.then_some(session.lifecycle_state),
        status_text: lifecycle_visible
            .then(|| session.latest_status_text.clone())
            .flatten(),
        visible_transcript,
        visible_transcript_bytes,
        unread_output_count: unread_visible.then_some(session.unread_output_count),
        pending_input_count: pending_visible.then_some(pending_input_count),
        pending_input_bytes: pending_visible.then_some(session.pending_input_bytes),
        last_input_feedback: session.last_input_feedback.as_ref().and_then(|feedback| {
            pending_visible.then(|| {
                if redacted {
                    redacted_feedback(feedback)
                } else {
                    feedback.clone()
                }
            })
        }),
    }
}

fn redacted_feedback(feedback: &PortalInputFeedback) -> PortalInputFeedback {
    PortalInputFeedback {
        projection_id: feedback.projection_id.clone(),
        input_id: String::new(),
        feedback_state: feedback.feedback_state,
        error_code: feedback.error_code,
        pending_input_count: feedback.pending_input_count,
        pending_input_bytes: feedback.pending_input_bytes,
        status_summary: feedback.status_summary.clone(),
    }
}

fn portal_id_for_projection(projection_id: &str) -> String {
    let prefix = "text-stream://projection/";
    let mut portal_id = String::with_capacity(prefix.len() + projection_id.len());
    portal_id.push_str(prefix);
    portal_id.push_str(projection_id);
    bounded_copy(portal_id, MAX_PORTAL_ID_BYTES)
}

fn validate_pending_input_item(
    item: &PendingInputItem,
    bounds: &ProjectionBounds,
) -> Result<(), ProjectionErrorCode> {
    validate_non_empty_bounded("input_id", &item.input_id, MAX_REQUEST_ID_BYTES)
        .map_err(|error| error.code())?;
    validate_non_empty_bounded(
        "projection_id",
        &item.projection_id,
        MAX_PROJECTION_ID_BYTES,
    )
    .map_err(|error| error.code())?;
    if item.submitted_at_wall_us == 0
        || item.expires_at_wall_us == 0
        || item.submitted_at_wall_us >= item.expires_at_wall_us
    {
        return Err(ProjectionErrorCode::ProjectionInvalidArgument);
    }
    if item.submission_text.len() > bounds.max_pending_input_bytes_per_item {
        return Err(ProjectionErrorCode::ProjectionInputTooLarge);
    }
    Ok(())
}

fn append_transcript_unit(
    session: &mut ProjectionSession,
    request: &PublishOutputRequest,
    server_timestamp_wall_us: u64,
    max_retained_transcript_bytes: usize,
    max_visible_transcript_bytes: usize,
    max_portal_updates_per_second: u32,
) {
    let portal_update_ready = portal_update_allowed(
        session,
        server_timestamp_wall_us,
        max_portal_updates_per_second,
    );
    session.last_publish_portal_update_ready = portal_update_ready;
    if portal_update_ready {
        session.portal_update_pending = true;
    }

    if !portal_update_ready {
        session.coalesced_portal_update_count += 1;
        if let Some(coalesce_key) = &request.coalesce_key {
            if let Some(existing) = session
                .retained_transcript
                .iter_mut()
                .rev()
                .find(|unit| unit.coalesce_key.as_ref() == Some(coalesce_key))
            {
                session.retained_transcript_bytes = session
                    .retained_transcript_bytes
                    .saturating_sub(existing.byte_len());
                existing.output_text = request.output_text.clone();
                existing.output_kind = request.output_kind;
                existing.content_classification = request.content_classification;
                existing.logical_unit_id = request.logical_unit_id.clone();
                existing.appended_at_wall_us = server_timestamp_wall_us;
                session.retained_transcript_bytes += existing.byte_len();
                prune_retained_transcript(
                    session,
                    max_retained_transcript_bytes,
                    max_visible_transcript_bytes,
                );
                promote_to_active_if_recovering(session);
                session.unread_output_count += 1;
                return;
            }
        }
    }

    let unit = TranscriptUnit {
        sequence: session.next_transcript_sequence,
        output_text: request.output_text.clone(),
        output_kind: request.output_kind,
        content_classification: request.content_classification,
        logical_unit_id: request.logical_unit_id.clone(),
        coalesce_key: request.coalesce_key.clone(),
        appended_at_wall_us: server_timestamp_wall_us,
    };
    session.next_transcript_sequence += 1;
    session.retained_transcript_bytes += unit.byte_len();
    session.retained_transcript.push_back(unit);
    prune_retained_transcript(
        session,
        max_retained_transcript_bytes,
        max_visible_transcript_bytes,
    );
    promote_to_active_if_recovering(session);
    session.unread_output_count += 1;
}

fn promote_to_active_if_recovering(session: &mut ProjectionSession) {
    if matches!(
        session.lifecycle_state,
        ProjectionLifecycleState::Attached | ProjectionLifecycleState::HudUnavailable
    ) {
        session.lifecycle_state = ProjectionLifecycleState::Active;
    }
}

fn portal_update_allowed(
    session: &mut ProjectionSession,
    server_timestamp_wall_us: u64,
    max_portal_updates_per_second: u32,
) -> bool {
    if session.portal_rate_window_started_at_wall_us == 0
        || server_timestamp_wall_us
            >= session.portal_rate_window_started_at_wall_us + PORTAL_UPDATE_RATE_WINDOW_WALL_US
    {
        session.portal_rate_window_started_at_wall_us = server_timestamp_wall_us;
        session.portal_updates_in_window = 0;
    }
    if session.portal_updates_in_window < max_portal_updates_per_second {
        session.portal_updates_in_window += 1;
        true
    } else {
        false
    }
}

fn prune_retained_transcript(
    session: &mut ProjectionSession,
    max_retained_transcript_bytes: usize,
    max_visible_transcript_bytes: usize,
) {
    let mut visible_bytes = 0usize;
    let mut oldest_visible_sequence = None;
    for unit in session.retained_transcript.iter().rev() {
        let next_visible_bytes = visible_bytes.saturating_add(unit.byte_len());
        if next_visible_bytes > max_visible_transcript_bytes {
            break;
        }
        visible_bytes = next_visible_bytes;
        oldest_visible_sequence = Some(unit.sequence);
    }
    while session.retained_transcript_bytes > max_retained_transcript_bytes {
        let Some(front) = session.retained_transcript.front() else {
            session.retained_transcript_bytes = 0;
            break;
        };
        if oldest_visible_sequence.is_some_and(|sequence| front.sequence >= sequence)
            && session.retained_transcript.len() == 1
        {
            break;
        }
        let Some(pruned) = session.retained_transcript.pop_front() else {
            break;
        };
        session.retained_transcript_bytes = session
            .retained_transcript_bytes
            .saturating_sub(pruned.byte_len());
    }
}

fn visible_transcript_window(
    session: &ProjectionSession,
    max_visible_transcript_bytes: usize,
) -> Vec<TranscriptUnit> {
    let mut visible = Vec::new();
    let mut visible_bytes = 0usize;
    for unit in session.retained_transcript.iter().rev() {
        let unit_bytes = unit.byte_len();
        if visible_bytes + unit_bytes > max_visible_transcript_bytes {
            break;
        }
        visible_bytes += unit_bytes;
        visible.push(unit.clone());
    }
    visible.reverse();
    visible
}

fn capabilities_are_subset(requested: &[String], granted: &[String]) -> bool {
    requested
        .iter()
        .all(|capability| granted.iter().any(|granted| granted == capability))
}

fn remember_logical_unit(
    session: &mut ProjectionSession,
    logical_unit_id: &str,
    max_seen_logical_units: usize,
) -> bool {
    if session.seen_logical_units.contains(logical_unit_id) {
        return true;
    }
    session
        .seen_logical_units
        .insert(logical_unit_id.to_string());
    session
        .seen_logical_unit_order
        .push_back(logical_unit_id.to_string());
    while session.seen_logical_unit_order.len() > max_seen_logical_units {
        if let Some(evicted) = session.seen_logical_unit_order.pop_front() {
            session.seen_logical_units.remove(&evicted);
        }
    }
    false
}

fn requested_delivery_state(ack_state: InputAckState) -> InputDeliveryState {
    match ack_state {
        InputAckState::Handled => InputDeliveryState::Handled,
        InputAckState::Rejected => InputDeliveryState::Rejected,
        InputAckState::Deferred => InputDeliveryState::Deferred,
    }
}

fn terminal_ack_replay_response(
    terminal_state: InputDeliveryState,
    request: &AcknowledgeInputRequest,
    server_timestamp_wall_us: u64,
) -> ProjectionResponse {
    if terminal_state == requested_delivery_state(request.ack_state) {
        return ProjectionResponse::accepted(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            "terminal acknowledgement replay accepted idempotently",
        );
    }
    ProjectionResponse::denied(
        &request.envelope.request_id,
        &request.envelope.projection_id,
        server_timestamp_wall_us,
        ProjectionErrorCode::ProjectionStateConflict,
        "conflicting acknowledgement for terminal input",
    )
}

fn remember_terminal_input(
    session: &mut ProjectionSession,
    input_id: &str,
    delivery_state: InputDeliveryState,
    max_completed_input_tombstones: usize,
) {
    if !delivery_state.is_terminal() {
        return;
    }
    if session
        .completed_input_ack_states
        .insert(input_id.to_string(), delivery_state)
        .is_none()
    {
        session
            .completed_input_ack_order
            .push_back(input_id.to_string());
    }
    while session.completed_input_ack_order.len() > max_completed_input_tombstones {
        if let Some(evicted) = session.completed_input_ack_order.pop_front() {
            session.completed_input_ack_states.remove(&evicted);
        }
    }
}

fn prune_terminal_pending_input(
    session: &mut ProjectionSession,
    max_completed_input_tombstones: usize,
) {
    let mut retained = VecDeque::with_capacity(session.pending_input.len());
    while let Some(item) = session.pending_input.pop_front() {
        if item.delivery_state.is_terminal() {
            remember_terminal_input(
                session,
                &item.input_id,
                item.delivery_state,
                max_completed_input_tombstones,
            );
        } else {
            retained.push_back(item);
        }
    }
    session.pending_input = retained;
}

fn acknowledge_input(
    session: &mut ProjectionSession,
    request: &AcknowledgeInputRequest,
    server_timestamp_wall_us: u64,
) -> ProjectionResponse {
    expire_pending(session, server_timestamp_wall_us);
    let Some(item) = session
        .pending_input
        .iter_mut()
        .find(|item| item.input_id == request.input_id)
    else {
        if let Some(terminal_state) = session.completed_input_ack_states.get(&request.input_id) {
            return terminal_ack_replay_response(
                *terminal_state,
                request,
                server_timestamp_wall_us,
            );
        }
        return ProjectionResponse::denied(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            ProjectionErrorCode::ProjectionNotFound,
            "input_id not found",
        );
    };

    if request.ack_state != InputAckState::Deferred && request.not_before_wall_us.is_some() {
        return ProjectionResponse::denied(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            ProjectionErrorCode::ProjectionInvalidArgument,
            "not_before_wall_us is only valid for deferred acknowledgements",
        );
    }

    if let Some(not_before_wall_us) = request.not_before_wall_us {
        if not_before_wall_us >= item.expires_at_wall_us {
            return ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                ProjectionErrorCode::ProjectionInvalidArgument,
                "not_before_wall_us must be before expires_at_wall_us",
            );
        }
    }

    if item.delivery_state.is_terminal() {
        return terminal_ack_replay_response(
            item.delivery_state,
            request,
            server_timestamp_wall_us,
        );
    }

    match request.ack_state {
        InputAckState::Handled => {
            session.pending_input_bytes = session
                .pending_input_bytes
                .saturating_sub(item.submission_text.len());
            item.delivery_state = InputDeliveryState::Handled;
        }
        InputAckState::Rejected => {
            session.pending_input_bytes = session
                .pending_input_bytes
                .saturating_sub(item.submission_text.len());
            item.delivery_state = InputDeliveryState::Rejected;
        }
        InputAckState::Deferred => {
            if item.delivery_state != InputDeliveryState::Delivered {
                return ProjectionResponse::denied(
                    &request.envelope.request_id,
                    &request.envelope.projection_id,
                    server_timestamp_wall_us,
                    ProjectionErrorCode::ProjectionStateConflict,
                    "only delivered input can be deferred",
                );
            }
            item.delivery_state = InputDeliveryState::Deferred;
            item.not_before_wall_us = request.not_before_wall_us;
        }
    }

    ProjectionResponse::accepted(
        &request.envelope.request_id,
        &request.envelope.projection_id,
        server_timestamp_wall_us,
        "acknowledgement accepted",
    )
}

fn expire_pending(session: &mut ProjectionSession, server_timestamp_wall_us: u64) {
    for item in &mut session.pending_input {
        if !item.delivery_state.is_terminal() && server_timestamp_wall_us >= item.expires_at_wall_us
        {
            session.pending_input_bytes = session
                .pending_input_bytes
                .saturating_sub(item.submission_text.len());
            item.delivery_state = InputDeliveryState::Expired;
        }
    }
}

fn validate_owner_token(owner_token: &str) -> Result<(), ProjectionContractError> {
    validate_non_empty_bounded("owner_token", owner_token, OWNER_TOKEN_ENTROPY_BITS / 4)
}

fn validate_non_empty_bounded(
    field: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ProjectionContractError> {
    if value.is_empty() {
        return Err(ProjectionContractError::InvalidArgument(format!(
            "{field} is required"
        )));
    }
    if value.len() > max_bytes {
        return Err(ProjectionContractError::InvalidArgument(format!(
            "{field} exceeds {max_bytes} bytes"
        )));
    }
    Ok(())
}

fn validate_optional_bounded(
    field: &str,
    value: &Option<String>,
    max_bytes: usize,
) -> Result<(), ProjectionContractError> {
    if let Some(value) = value {
        validate_non_empty_bounded(field, value, max_bytes)?;
    }
    Ok(())
}

fn validate_non_zero(field: &str, value: u64) -> Result<(), ProjectionContractError> {
    if value == 0 {
        return Err(ProjectionContractError::InvalidArgument(format!(
            "{field} must be non-zero"
        )));
    }
    Ok(())
}

fn generate_owner_token() -> Result<String, ProjectionContractError> {
    let mut token_bytes = [0u8; OWNER_TOKEN_ENTROPY_BITS / 8];
    getrandom::fill(&mut token_bytes).map_err(|_| ProjectionContractError::TokenGeneration)?;
    Ok(hex_encode(&token_bytes))
}

fn verifier_for_secret(secret: &str) -> String {
    blake3::hash(secret.as_bytes()).to_hex().to_string()
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    left.as_bytes().ct_eq(right.as_bytes()).into()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn bounded_copy(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn envelope(
        operation: ProjectionOperation,
        projection_id: &str,
        request_id: &str,
    ) -> OperationEnvelope {
        OperationEnvelope {
            operation,
            projection_id: projection_id.to_string(),
            request_id: request_id.to_string(),
            client_timestamp_wall_us: 1,
        }
    }

    fn attach_request(projection_id: &str, request_id: &str) -> AttachRequest {
        AttachRequest {
            envelope: envelope(ProjectionOperation::Attach, projection_id, request_id),
            provider_kind: ProviderKind::Codex,
            display_name: "Codex Session".to_string(),
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: None,
            content_classification: ContentClassification::Private,
            hud_target: None,
            idempotency_key: Some("attach-once".to_string()),
        }
    }

    fn attach(authority: &mut ProjectionAuthority, projection_id: &str) -> String {
        authority
            .handle_attach(attach_request(projection_id, "attach-1"), "caller-a", 10)
            .owner_token
            .expect("attach must issue owner token")
    }

    fn output_request(
        projection_id: &str,
        owner_token: &str,
        request_id: &str,
    ) -> PublishOutputRequest {
        PublishOutputRequest {
            envelope: envelope(
                ProjectionOperation::PublishOutput,
                projection_id,
                request_id,
            ),
            owner_token: owner_token.to_string(),
            output_text: "hello projection".to_string(),
            output_kind: OutputKind::Assistant,
            content_classification: ContentClassification::Private,
            logical_unit_id: Some("unit-1".to_string()),
            coalesce_key: None,
        }
    }

    fn connection_metadata(grants: &[&str]) -> HudConnectionMetadata {
        HudConnectionMetadata {
            connection_id: "connection-1".to_string(),
            authenticated_session_id: "runtime-session-1".to_string(),
            granted_capabilities: grants.iter().map(|grant| (*grant).to_string()).collect(),
            connected_at_wall_us: 20,
            last_reconnect_wall_us: 20,
        }
    }

    fn advisory_lease(capabilities: &[&str], expires_at_wall_us: u64) -> AdvisoryLeaseIdentity {
        AdvisoryLeaseIdentity {
            lease_id: "lease-1".to_string(),
            capabilities: capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect(),
            acquired_at_wall_us: 21,
            expires_at_wall_us,
        }
    }

    fn portal_submission(input_id: &str, text: &str) -> PortalInputSubmission {
        PortalInputSubmission {
            input_id: input_id.to_string(),
            submission_text: text.to_string(),
            submitted_at_wall_us: 30,
            expires_at_wall_us: Some(1_000),
            content_classification: ContentClassification::Private,
        }
    }

    fn windows_target() -> WindowsHudTarget {
        WindowsHudTarget {
            target_id: "windows-local".to_string(),
            mcp_url: Some("http://tzehouse-windows.parrot-hen.ts.net:9090/mcp".to_string()),
            grpc_endpoint: Some("tzehouse-windows.parrot-hen.ts.net:50051".to_string()),
            credential_source: HudCredentialSource::EnvVar("TZE_HUD_PSK".to_string()),
            runtime_audience: "local-windows-hud".to_string(),
        }
    }

    fn managed_zone_session(projection_id: &str) -> ManagedSessionRequest {
        ManagedSessionRequest {
            projection_id: projection_id.to_string(),
            provider_kind: ProviderKind::Codex,
            display_name: "Codex Status".to_string(),
            origin: ManagedSessionOrigin::Attached,
            hud_target_id: "windows-local".to_string(),
            surface_route: PresenceSurfaceRoute::Zone {
                zone_name: "status-bar".to_string(),
                content_kind: "status".to_string(),
                ttl_ms: 10_000,
            },
            content_classification: ContentClassification::Household,
            attention_intent: ProjectionAttentionIntent::Ambient,
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: None,
        }
    }

    fn managed_widget_session(projection_id: &str) -> ManagedSessionRequest {
        let mut parameters = HashMap::new();
        parameters.insert("progress".to_string(), WidgetParameterValue::F32Milli(420));
        ManagedSessionRequest {
            projection_id: projection_id.to_string(),
            provider_kind: ProviderKind::Claude,
            display_name: "Claude Progress".to_string(),
            origin: ManagedSessionOrigin::Launched(LaunchSessionSpec {
                command: "claude".to_string(),
                args: vec!["--continue".to_string()],
                working_directory: Some("/home/tze/gt/tze_hud/mayor/rig".to_string()),
                environment_keys: vec!["ANTHROPIC_API_KEY".to_string()],
            }),
            hud_target_id: "windows-local".to_string(),
            surface_route: PresenceSurfaceRoute::Widget {
                widget_name: "main-progress".to_string(),
                parameters,
                ttl_ms: 10_000,
            },
            content_classification: ContentClassification::Private,
            attention_intent: ProjectionAttentionIntent::Ambient,
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: Some("claude".to_string()),
        }
    }

    fn managed_portal_session(projection_id: &str) -> ManagedSessionRequest {
        ManagedSessionRequest {
            projection_id: projection_id.to_string(),
            provider_kind: ProviderKind::Opencode,
            display_name: "Opencode Questions".to_string(),
            origin: ManagedSessionOrigin::Attached,
            hud_target_id: "windows-local".to_string(),
            surface_route: PresenceSurfaceRoute::Portal {
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                lease_ttl_ms: 30_000,
            },
            content_classification: ContentClassification::Private,
            attention_intent: ProjectionAttentionIntent::Gentle,
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: Some("opencode".to_string()),
        }
    }

    #[test]
    fn external_authority_plans_three_provider_neutral_sessions_across_existing_surfaces() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");

        let zone = authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .expect("zone session is managed");
        let widget = authority
            .manage_session(managed_widget_session("agent-progress"), "manager", 11)
            .expect("widget session is managed");
        let portal = authority
            .manage_session(managed_portal_session("agent-question"), "manager", 12)
            .expect("portal session is managed");

        assert_eq!(authority.managed_session_count(), 3);
        assert!(
            authority
                .projection_authority()
                .has_projection("agent-status")
        );
        assert!(
            authority
                .projection_authority()
                .has_projection("agent-progress")
        );
        assert!(
            authority
                .projection_authority()
                .has_projection("agent-question")
        );

        assert!(matches!(
            zone.route_plan.surface_command,
            HudSurfaceCommandPlan::ZonePublish { .. }
        ));
        assert!(matches!(
            widget.route_plan.surface_command,
            HudSurfaceCommandPlan::WidgetPublish { .. }
        ));
        assert!(matches!(
            portal.route_plan.surface_command,
            HudSurfaceCommandPlan::PortalLease { .. }
        ));
        assert_eq!(
            zone.route_plan.attention_intent,
            ProjectionAttentionIntent::Ambient
        );
        assert_eq!(
            widget.route_plan.attention_intent,
            ProjectionAttentionIntent::Ambient
        );
        assert_eq!(
            portal.route_plan.attention_intent,
            ProjectionAttentionIntent::Gentle
        );

        let demo = authority.three_session_demo_plan();
        assert_eq!(demo.len(), 3);
        assert_eq!(demo[0].projection_id, "agent-progress");
        assert_eq!(demo[1].projection_id, "agent-question");
        assert_eq!(demo[2].projection_id, "agent-status");
    }

    #[test]
    fn external_authority_route_plans_redact_credentials_and_expose_no_capture_authority() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");

        let handle = authority
            .manage_session(managed_widget_session("agent-progress"), "manager", 10)
            .expect("widget session is managed");
        let serialized = serde_json::to_string(&handle.route_plan).unwrap();

        assert!(serialized.contains("env:TZE_HUD_PSK:redacted"));
        assert!(!serialized.contains(&handle.owner_token));
        assert!(!serialized.contains("operator-secret"));
        for forbidden in [
            "pty",
            "terminal_capture",
            "stdin",
            "stdout",
            "raw_keystroke",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "route plan must not expose {forbidden} authority"
            );
        }
    }

    #[test]
    fn external_authority_revokes_one_session_without_mutating_others() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .unwrap();
        authority
            .manage_session(managed_widget_session("agent-progress"), "manager", 11)
            .unwrap();
        authority
            .manage_session(managed_portal_session("agent-question"), "manager", 12)
            .unwrap();

        authority.revoke_session("agent-progress").unwrap();

        assert_eq!(authority.managed_session_count(), 2);
        assert!(authority.route_plan("agent-progress").is_none());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-progress")
                .is_none()
        );
        assert!(authority.route_plan("agent-status").is_some());
        assert!(authority.route_plan("agent-question").is_some());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-status")
                .is_some()
        );
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-question")
                .is_some()
        );
    }

    #[test]
    fn external_authority_expiry_purges_managed_session_and_preserves_unexpired_sessions() {
        let mut authority = ExternalAgentProjectionAuthority::new(ProjectionBounds {
            owner_token_ttl_wall_us: 20,
            ..ProjectionBounds::default()
        })
        .unwrap();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .unwrap();
        authority
            .manage_session(managed_portal_session("agent-question"), "manager", 11)
            .unwrap();

        assert_eq!(authority.expire_token_expired_sessions(29), 0);
        assert_eq!(authority.managed_session_count(), 2);

        assert_eq!(authority.expire_token_expired_sessions(30), 1);
        assert_eq!(authority.managed_session_count(), 1);
        assert!(authority.route_plan("agent-status").is_none());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-status")
                .is_none()
        );
        assert!(authority.route_plan("agent-question").is_some());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-question")
                .is_some()
        );

        assert_eq!(authority.expire_token_expired_sessions(31), 1);
        assert_eq!(authority.managed_session_count(), 0);
        assert!(authority.route_plan("agent-question").is_none());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-question")
                .is_none()
        );
    }

    #[test]
    fn external_authority_reconnect_requires_fresh_runtime_lease_authority() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_portal_session("agent-question"), "manager", 10)
            .unwrap();
        authority
            .record_hud_connection(
                "agent-question",
                HudConnectionMetadata {
                    connection_id: "connection-1".to_string(),
                    authenticated_session_id: "runtime-session-1".to_string(),
                    granted_capabilities: vec![
                        "create_tiles".to_string(),
                        "modify_own_tiles".to_string(),
                    ],
                    connected_at_wall_us: 20,
                    last_reconnect_wall_us: 20,
                },
            )
            .unwrap();
        authority
            .projection_authority_mut()
            .record_advisory_lease(
                "agent-question",
                AdvisoryLeaseIdentity {
                    lease_id: "lease-1".to_string(),
                    capabilities: vec!["create_tiles".to_string()],
                    acquired_at_wall_us: 21,
                    expires_at_wall_us: 100,
                },
                22,
            )
            .unwrap();

        authority
            .mark_hud_disconnected("agent-question", 30)
            .unwrap();
        let disconnected = authority
            .projection_authority()
            .state_summary("agent-question")
            .unwrap();
        assert_eq!(
            disconnected.lifecycle_state,
            ProjectionLifecycleState::HudUnavailable
        );
        assert!(!disconnected.has_advisory_lease);

        authority
            .record_hud_connection(
                "agent-question",
                HudConnectionMetadata {
                    connection_id: "connection-2".to_string(),
                    authenticated_session_id: "runtime-session-2".to_string(),
                    granted_capabilities: vec![
                        "create_tiles".to_string(),
                        "modify_own_tiles".to_string(),
                    ],
                    connected_at_wall_us: 40,
                    last_reconnect_wall_us: 40,
                },
            )
            .unwrap();
        let stale = authority
            .projection_authority_mut()
            .authorize_portal_republish(
                "agent-question",
                "lease-1",
                &["create_tiles".to_string()],
                41,
            );
        assert_eq!(stale, Err(ProjectionErrorCode::ProjectionUnauthorized));

        authority
            .projection_authority_mut()
            .record_advisory_lease(
                "agent-question",
                AdvisoryLeaseIdentity {
                    lease_id: "lease-2".to_string(),
                    capabilities: vec!["create_tiles".to_string()],
                    acquired_at_wall_us: 42,
                    expires_at_wall_us: 100,
                },
                43,
            )
            .unwrap();
        assert_eq!(
            authority
                .projection_authority_mut()
                .authorize_portal_republish(
                    "agent-question",
                    "lease-2",
                    &["create_tiles".to_string()],
                    44,
                ),
            Ok(())
        );
    }

    #[test]
    fn schema_uses_required_wall_clock_and_owner_token_fields() {
        let attach_json = serde_json::to_value(attach_request("projection-a", "req-a")).unwrap();
        assert_eq!(attach_json["operation"], "attach");
        assert_eq!(attach_json["client_timestamp_wall_us"], 1);
        assert!(attach_json.get("owner_token").is_none());

        let publish_json =
            serde_json::to_value(output_request("projection-a", "owner-token", "req-b")).unwrap();
        assert_eq!(publish_json["operation"], "publish_output");
        assert_eq!(publish_json["owner_token"], "owner-token");
        assert_eq!(publish_json["content_classification"], "private");
    }

    #[test]
    fn stable_error_code_set_is_append_only_wire_shape() {
        let codes: Vec<&str> = INITIAL_ERROR_CODES
            .iter()
            .map(|code| code.as_str())
            .collect();
        assert_eq!(
            codes,
            vec![
                "PROJECTION_NOT_FOUND",
                "PROJECTION_ALREADY_ATTACHED",
                "PROJECTION_UNAUTHORIZED",
                "PROJECTION_TOKEN_EXPIRED",
                "PROJECTION_INVALID_ARGUMENT",
                "PROJECTION_OUTPUT_TOO_LARGE",
                "PROJECTION_INPUT_TOO_LARGE",
                "PROJECTION_INPUT_QUEUE_FULL",
                "PROJECTION_RATE_LIMITED",
                "PROJECTION_STATE_CONFLICT",
                "PROJECTION_HUD_UNAVAILABLE",
                "PROJECTION_INTERNAL_ERROR",
            ]
        );
        assert_eq!(
            serde_json::to_string(&ProjectionErrorCode::ProjectionUnauthorized).unwrap(),
            "\"PROJECTION_UNAUTHORIZED\""
        );
    }

    #[test]
    fn attach_materializes_content_layer_projected_portal_and_reuses_idempotently() {
        let mut authority = ProjectionAuthority::default();
        let first =
            authority.handle_attach(attach_request("projection-a", "req-a"), "caller-a", 10);
        assert!(first.accepted);

        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .expect("attach creates portal state");
        assert_eq!(
            state.adapter_family,
            ProjectedPortalAdapterFamily::CooperativeProjection
        );
        assert_eq!(
            state.runtime_authority,
            ProjectedPortalRuntimeAuthority::ResidentSessionLease
        );
        assert_eq!(state.layer, ProjectedPortalLayer::Content);
        assert_eq!(state.presentation, ProjectedPortalPresentation::Expanded);
        assert_eq!(state.display_name.as_deref(), Some("Codex Session"));
        assert_eq!(state.workspace_hint.as_deref(), Some("mayor/rig"));
        assert!(state.interaction_enabled);

        authority.collapse_projected_portal("projection-a").unwrap();
        let collapsed = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .expect("collapsed portal state remains materializable");
        assert_eq!(collapsed.portal_id, state.portal_id);
        assert_eq!(
            collapsed.presentation,
            ProjectedPortalPresentation::Collapsed
        );
        assert!(collapsed.visible_transcript.is_empty());
        assert!(!collapsed.interaction_enabled);

        let replay =
            authority.handle_attach(attach_request("projection-a", "req-b"), "caller-a", 11);
        assert!(replay.accepted);
        assert!(replay.owner_token.is_none());
        assert_eq!(
            authority
                .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
                .unwrap()
                .portal_id,
            state.portal_id
        );
    }

    #[test]
    fn successful_attach_issues_high_entropy_token_and_stores_only_verifier() {
        let mut authority = ProjectionAuthority::default();
        let response =
            authority.handle_attach(attach_request("projection-a", "req-a"), "caller-a", 10);
        assert!(response.accepted);
        let owner_token = response.owner_token.expect("attach must return token once");
        assert_eq!(owner_token.len(), OWNER_TOKEN_ENTROPY_BITS / 4);
        assert!(owner_token.chars().all(|ch| ch.is_ascii_hexdigit()));

        let verifier = authority
            .owner_token_verifier_for_test("projection-a")
            .expect("session stores verifier");
        assert_ne!(verifier, owner_token);
        assert_eq!(verifier.len(), 64);
    }

    #[test]
    fn attach_conflict_is_deterministic_and_idempotent_replay_does_not_expose_token() {
        let mut authority = ProjectionAuthority::default();
        let first =
            authority.handle_attach(attach_request("projection-a", "req-a"), "caller-a", 10);
        assert!(first.accepted);
        assert!(first.owner_token.is_some());

        let replay =
            authority.handle_attach(attach_request("projection-a", "req-b"), "caller-a", 11);
        assert!(replay.accepted);
        assert!(replay.owner_token.is_none());

        let mut conflicting = attach_request("projection-a", "req-c");
        conflicting.idempotency_key = Some("different-key".to_string());
        let conflict = authority.handle_attach(conflicting, "caller-b", 12);
        assert!(!conflict.accepted);
        assert_eq!(
            conflict.error_code,
            Some(ProjectionErrorCode::ProjectionAlreadyAttached)
        );
    }

    #[test]
    fn cross_projection_read_fails_closed_and_audits_without_payload_text() {
        let mut authority = ProjectionAuthority::default();
        let _token_a = attach(&mut authority, "projection-a");
        let token_b = attach(&mut authority, "projection-b");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "private operator text".to_string(),
                20,
                1_000,
                None,
            )
            .unwrap();

        let denied = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-read",
                ),
                owner_token: token_b,
                max_items: None,
                max_bytes: None,
            },
            "caller-b",
            30,
        );
        assert!(!denied.accepted);
        assert_eq!(
            denied.error_code,
            Some(ProjectionErrorCode::ProjectionUnauthorized)
        );
        assert!(denied.pending_input.is_empty());

        let audit = authority.audit_log().last().expect("denial audit exists");
        assert_eq!(audit.category, ProjectionAuditCategory::AuthDenied);
        assert_eq!(
            audit.error_code,
            Some(ProjectionErrorCode::ProjectionUnauthorized)
        );
        assert!(!audit.reason.contains("private operator text"));
    }

    #[test]
    fn oversized_output_is_rejected_with_stable_code() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_output_bytes_per_call: 4,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        let mut request = output_request("projection-a", &owner_token, "req-output");
        request.output_text = "too large".to_string();

        let response = authority.handle_publish_output(request, "caller-a", 20);
        assert!(!response.accepted);
        assert_eq!(
            response.error_code,
            Some(ProjectionErrorCode::ProjectionOutputTooLarge)
        );
    }

    #[test]
    fn logical_unit_id_replay_is_idempotent() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let first = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output-1"),
            "caller-a",
            20,
        );
        let replay = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output-2"),
            "caller-a",
            21,
        );
        assert!(first.accepted);
        assert!(replay.accepted);
        assert!(replay.status_summary.contains("idempotently"));
    }

    #[test]
    fn logical_unit_id_cache_is_bounded() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_seen_logical_units: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        let first = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output-1"),
            "caller-a",
            20,
        );
        let mut second_request = output_request("projection-a", &owner_token, "req-output-2");
        second_request.logical_unit_id = Some("unit-2".to_string());
        let second = authority.handle_publish_output(second_request, "caller-a", 21);
        let first_again = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output-3"),
            "caller-a",
            22,
        );

        assert!(first.accepted);
        assert!(second.accepted);
        assert!(first_again.accepted);
        assert!(!first_again.status_summary.contains("idempotently"));
    }

    #[test]
    fn audit_log_is_bounded_without_payload_text() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_audit_records: 2,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        for index in 0..3 {
            let mut request =
                output_request("projection-a", &owner_token, &format!("req-output-{index}"));
            request.output_text = format!("private transcript {index}");
            let response = authority.handle_publish_output(request, "caller-a", 20 + index);
            assert!(response.accepted);
        }

        assert_eq!(authority.audit_log().len(), 2);
        assert!(
            authority
                .audit_log()
                .iter()
                .all(|audit| !audit.reason.contains("private transcript"))
        );
    }

    #[test]
    fn portal_composer_submission_is_transactional_bounded_inbox_feedback() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_pending_input_items: 1,
            max_pending_input_bytes_per_item: 4,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        let oversized = authority.submit_portal_input(
            "projection-a",
            portal_submission("input-too-large", "12345"),
        );
        assert_eq!(oversized.feedback_state, PortalInputFeedbackState::Rejected);
        assert_eq!(
            oversized.error_code,
            Some(ProjectionErrorCode::ProjectionInputTooLarge)
        );
        assert_eq!(oversized.pending_input_count, 0);

        let accepted =
            authority.submit_portal_input("projection-a", portal_submission("input-1", "ok"));
        assert_eq!(accepted.feedback_state, PortalInputFeedbackState::Accepted);
        assert_eq!(accepted.pending_input_count, 1);
        assert_eq!(accepted.pending_input_bytes, 2);

        let full =
            authority.submit_portal_input("projection-a", portal_submission("input-2", "yo"));
        assert_eq!(full.feedback_state, PortalInputFeedbackState::Rejected);
        assert_eq!(
            full.error_code,
            Some(ProjectionErrorCode::ProjectionInputQueueFull)
        );
        assert_eq!(full.pending_input_count, 1);

        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .expect("portal state includes pending feedback");
        assert_eq!(state.pending_input_count, Some(1));
        assert_eq!(state.pending_input_bytes, Some(2));
        assert_eq!(
            state.last_input_feedback.as_ref().map(|f| f.feedback_state),
            Some(PortalInputFeedbackState::Rejected)
        );
        assert_eq!(
            state
                .last_input_feedback
                .as_ref()
                .map(|f| f.input_id.as_str()),
            Some("input-2")
        );

        let poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token,
                max_items: None,
                max_bytes: None,
            },
            "caller-a",
            40,
        );
        assert!(poll.accepted);
        assert_eq!(poll.pending_input.len(), 1);
        assert_eq!(poll.pending_input[0].input_id, "input-1");
        assert_eq!(poll.pending_input[0].submission_text, "ok");
    }

    #[test]
    fn default_bounds_match_projection_spec_values() {
        let bounds = ProjectionBounds::default();
        assert_eq!(
            bounds.max_output_bytes_per_call,
            DEFAULT_MAX_OUTPUT_BYTES_PER_CALL
        );
        assert_eq!(bounds.max_status_text_bytes, DEFAULT_MAX_STATUS_TEXT_BYTES);
        assert_eq!(
            bounds.max_retained_transcript_bytes,
            DEFAULT_MAX_RETAINED_TRANSCRIPT_BYTES
        );
        assert_eq!(
            bounds.max_visible_transcript_bytes,
            DEFAULT_MAX_VISIBLE_TRANSCRIPT_BYTES
        );
        assert_eq!(
            bounds.max_pending_input_items,
            DEFAULT_MAX_PENDING_INPUT_ITEMS
        );
        assert_eq!(
            bounds.max_pending_input_bytes_per_item,
            DEFAULT_MAX_PENDING_INPUT_BYTES_PER_ITEM
        );
        assert_eq!(
            bounds.max_pending_input_total_bytes,
            DEFAULT_MAX_PENDING_INPUT_TOTAL_BYTES
        );
        assert_eq!(bounds.max_poll_items, DEFAULT_MAX_POLL_ITEMS);
        assert_eq!(
            bounds.max_poll_response_bytes,
            DEFAULT_MAX_POLL_RESPONSE_BYTES
        );
        assert_eq!(
            bounds.max_portal_updates_per_second,
            DEFAULT_MAX_PORTAL_UPDATES_PER_SECOND
        );
    }

    #[test]
    fn collapsed_redacted_projection_preserves_geometry_and_suppresses_private_affordances() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let mut output = output_request("projection-a", &owner_token, "req-output");
        output.output_text = "private projected transcript".to_string();
        assert!(
            authority
                .handle_publish_output(output, "caller-a", 20)
                .accepted
        );
        let feedback =
            authority.submit_portal_input("projection-a", portal_submission("input-1", "help"));
        assert_eq!(feedback.feedback_state, PortalInputFeedbackState::Accepted);
        authority.collapse_projected_portal("projection-a").unwrap();

        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::default())
            .expect("redacted portal still materializes");
        assert_eq!(state.presentation, ProjectedPortalPresentation::Collapsed);
        assert!(state.preserve_geometry);
        assert!(state.redacted);
        assert!(!state.interaction_enabled);
        assert_eq!(state.layer, ProjectedPortalLayer::Content);
        assert!(state.provider_kind.is_none());
        assert!(state.display_name.is_none());
        assert!(state.workspace_hint.is_none());
        assert!(state.lifecycle_state.is_none());
        assert!(state.visible_transcript.is_empty());
        assert_eq!(state.unread_output_count, None);
        assert_eq!(state.pending_input_count, None);
        assert!(
            !serde_json::to_string(&state)
                .unwrap()
                .contains("private projected transcript")
        );
    }

    #[test]
    fn portal_submission_default_ttl_overflow_is_rejected_not_panicked() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");

        let feedback = authority.submit_portal_input(
            "projection-a",
            PortalInputSubmission {
                input_id: "input-overflow".to_string(),
                submission_text: "help".to_string(),
                submitted_at_wall_us: u64::MAX,
                expires_at_wall_us: None,
                content_classification: ContentClassification::Private,
            },
        );

        assert_eq!(feedback.feedback_state, PortalInputFeedbackState::Rejected);
        assert_eq!(
            feedback.error_code,
            Some(ProjectionErrorCode::ProjectionInvalidArgument)
        );
        assert_eq!(feedback.pending_input_count, 0);
    }

    #[test]
    fn projection_private_state_is_memory_only_and_purged_on_detach_cleanup_and_expiry() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            owner_token_ttl_wall_us: 30,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .record_hud_connection(
                "projection-a",
                connection_metadata(&["create_tiles", "modify_own_tiles"]),
            )
            .unwrap();
        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 22)
            .unwrap();
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "operator private text".to_string(),
                23,
                100,
                None,
            )
            .unwrap();
        let mut output = output_request("projection-a", &owner_token, "req-output");
        output.output_text = "private transcript text".to_string();
        assert!(
            authority
                .handle_publish_output(output, "caller-a", 24)
                .accepted
        );

        let summary = authority.state_summary("projection-a").unwrap();
        assert!(summary.has_hud_connection);
        assert!(summary.has_advisory_lease);
        assert_eq!(summary.pending_input_count, 1);
        assert!(summary.retained_transcript_bytes > 0);
        assert_eq!(
            authority.visible_transcript_window("projection-a").unwrap()[0].output_text,
            "private transcript text"
        );

        let detached = authority.handle_detach(
            DetachRequest {
                envelope: envelope(ProjectionOperation::Detach, "projection-a", "req-detach"),
                owner_token: owner_token.clone(),
                reason: "done".to_string(),
            },
            "caller-a",
            25,
        );
        assert!(detached.accepted);
        assert!(!authority.has_projection("projection-a"));
        assert!(
            authority
                .audit_log()
                .iter()
                .all(|audit| !audit.reason.contains("private transcript")
                    && !audit.reason.contains("operator private text"))
        );

        let mut restarted = ProjectionAuthority::default();
        assert!(!restarted.has_projection("projection-a"));
        let fresh = attach(&mut restarted, "projection-a");
        assert_ne!(fresh, owner_token);

        let expired_token = attach(&mut restarted, "projection-expiring");
        assert!(restarted.has_projection("projection-expiring"));
        let expired = restarted.handle_publish_status(
            PublishStatusRequest {
                envelope: envelope(
                    ProjectionOperation::PublishStatus,
                    "projection-expiring",
                    "req-expired",
                ),
                owner_token: expired_token,
                lifecycle_state: ProjectionLifecycleState::Active,
                status_text: None,
            },
            "caller-a",
            DEFAULT_OWNER_TOKEN_TTL_WALL_US + 20,
        );
        assert!(!expired.accepted);
        assert_eq!(
            expired.error_code,
            Some(ProjectionErrorCode::ProjectionTokenExpired)
        );
        assert!(!restarted.has_projection("projection-expiring"));
    }

    #[test]
    fn stale_or_overbroad_lease_identity_cannot_authorize_republish() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");
        authority
            .record_hud_connection(
                "projection-a",
                connection_metadata(&["create_tiles", "modify_own_tiles"]),
            )
            .unwrap();
        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 22)
            .unwrap();

        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                30
            ),
            Ok(())
        );
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("upload_resource")],
                31
            ),
            Err(ProjectionErrorCode::ProjectionUnauthorized)
        );
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                101
            ),
            Err(ProjectionErrorCode::ProjectionTokenExpired)
        );
        assert!(
            !authority
                .state_summary("projection-a")
                .unwrap()
                .has_advisory_lease
        );

        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 200), 120)
            .unwrap();
        authority
            .mark_hud_disconnected("projection-a", 130)
            .unwrap();
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                131
            ),
            Err(ProjectionErrorCode::ProjectionHudUnavailable)
        );
    }

    #[test]
    fn reconnect_updates_bookkeeping_and_requires_fresh_lease() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");
        authority
            .record_hud_connection(
                "projection-a",
                connection_metadata(&["create_tiles", "modify_own_tiles"]),
            )
            .unwrap();
        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 22)
            .unwrap();

        let mut reconnected = connection_metadata(&["create_tiles", "modify_own_tiles"]);
        reconnected.connection_id = "connection-2".to_string();
        reconnected.authenticated_session_id = "runtime-session-2".to_string();
        reconnected.connected_at_wall_us = 40;
        reconnected.last_reconnect_wall_us = 40;
        authority
            .record_hud_connection("projection-a", reconnected)
            .unwrap();

        let summary = authority.state_summary("projection-a").unwrap();
        assert_eq!(summary.reconnect.reconnect_count, 1);
        assert_eq!(summary.reconnect.last_reconnect_wall_us, Some(40));
        assert!(!summary.has_advisory_lease);
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                41
            ),
            Err(ProjectionErrorCode::ProjectionUnauthorized)
        );

        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 42)
            .unwrap();
        authority.mark_hud_disconnected("projection-a", 50).unwrap();
        let mut after_disconnect = connection_metadata(&["create_tiles"]);
        after_disconnect.connection_id = "connection-3".to_string();
        after_disconnect.authenticated_session_id = "runtime-session-3".to_string();
        after_disconnect.connected_at_wall_us = 60;
        after_disconnect.last_reconnect_wall_us = 60;
        authority
            .record_hud_connection("projection-a", after_disconnect)
            .unwrap();

        let summary = authority.state_summary("projection-a").unwrap();
        assert_eq!(summary.reconnect.reconnect_count, 2);
        assert_eq!(summary.reconnect.last_disconnect_wall_us, Some(50));
        assert!(!summary.has_advisory_lease);
    }

    #[test]
    fn heartbeat_requires_live_connection_and_is_monotonic() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");

        assert_eq!(
            authority.record_heartbeat("projection-a", 25),
            Err(ProjectionErrorCode::ProjectionHudUnavailable)
        );

        authority
            .record_hud_connection("projection-a", connection_metadata(&["create_tiles"]))
            .unwrap();
        authority.record_heartbeat("projection-a", 30).unwrap();
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .reconnect
                .last_heartbeat_wall_us,
            Some(30)
        );
        assert_eq!(
            authority.record_heartbeat("projection-a", 29),
            Err(ProjectionErrorCode::ProjectionStateConflict)
        );

        authority.mark_hud_disconnected("projection-a", 40).unwrap();
        assert_eq!(
            authority.record_heartbeat("projection-a", 41),
            Err(ProjectionErrorCode::ProjectionHudUnavailable)
        );
    }

    #[test]
    fn reconnect_preserves_transcript_inbox_ack_state_and_requires_new_lease() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let mut output = output_request("projection-a", &owner_token, "req-output");
        output.output_text = "retained across HUD reconnect".to_string();
        assert!(
            authority
                .handle_publish_output(output, "caller-a", 20)
                .accepted
        );
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "operator input survives reconnect".to_string(),
                21,
                1_000,
                None,
            )
            .unwrap();
        let delivered = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(1),
                max_bytes: None,
            },
            "caller-a",
            22,
        );
        assert!(delivered.accepted);
        authority
            .record_hud_connection("projection-a", connection_metadata(&["create_tiles"]))
            .unwrap();
        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 23)
            .unwrap();

        authority.mark_hud_disconnected("projection-a", 30).unwrap();
        let mut reconnected = connection_metadata(&["create_tiles"]);
        reconnected.connection_id = "connection-after-drop".to_string();
        reconnected.authenticated_session_id = "runtime-session-after-drop".to_string();
        reconnected.connected_at_wall_us = 40;
        reconnected.last_reconnect_wall_us = 40;
        authority
            .record_hud_connection("projection-a", reconnected)
            .unwrap();

        let summary = authority.state_summary("projection-a").unwrap();
        assert_eq!(summary.retained_transcript_units, 1);
        assert_eq!(summary.pending_input_count, 1);
        assert!(!summary.has_advisory_lease);
        assert_eq!(
            authority.visible_transcript_window("projection-a").unwrap()[0].output_text,
            "retained across HUD reconnect"
        );
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                41
            ),
            Err(ProjectionErrorCode::ProjectionUnauthorized)
        );

        let handled_after_reconnect = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-after-reconnect",
                ),
                owner_token,
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            42,
        );
        assert!(handled_after_reconnect.accepted);
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .pending_input_count,
            0
        );
    }

    #[test]
    fn owner_degraded_lifecycle_is_not_overwritten_by_connection_or_output() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let degraded = authority.handle_publish_status(
            PublishStatusRequest {
                envelope: envelope(
                    ProjectionOperation::PublishStatus,
                    "projection-a",
                    "req-status",
                ),
                owner_token: owner_token.clone(),
                lifecycle_state: ProjectionLifecycleState::Degraded,
                status_text: Some("HUD projection is degraded".to_string()),
            },
            "caller-a",
            20,
        );
        assert!(degraded.accepted);

        authority
            .record_hud_connection("projection-a", connection_metadata(&["create_tiles"]))
            .unwrap();
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .lifecycle_state,
            ProjectionLifecycleState::Degraded
        );

        let published = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output"),
            "caller-a",
            21,
        );
        assert!(published.accepted);
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .lifecycle_state,
            ProjectionLifecycleState::Degraded
        );
    }

    #[test]
    fn acknowledgement_and_detach_cleanup_update_projected_portal_state() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let accepted =
            authority.submit_portal_input("projection-a", portal_submission("input-1", "ok"));
        assert_eq!(accepted.feedback_state, PortalInputFeedbackState::Accepted);
        assert_eq!(
            authority
                .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
                .unwrap()
                .pending_input_count,
            Some(1)
        );

        let poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token: owner_token.clone(),
                max_items: None,
                max_bytes: None,
            },
            "caller-a",
            40,
        );
        assert!(poll.accepted);
        let handled = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            41,
        );
        assert!(handled.accepted);
        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .unwrap();
        assert_eq!(state.pending_input_count, Some(0));
        assert_eq!(state.pending_input_bytes, Some(0));

        let detached = authority.handle_detach(
            DetachRequest {
                envelope: envelope(ProjectionOperation::Detach, "projection-a", "req-detach"),
                owner_token,
                reason: "session complete".to_string(),
            },
            "caller-a",
            42,
        );
        assert!(detached.accepted);
        assert!(
            authority
                .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
                .is_none()
        );
    }

    #[test]
    fn projected_portal_contract_has_no_terminal_or_process_authority() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");
        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .expect("portal state exists");

        assert_eq!(
            state.adapter_family,
            ProjectedPortalAdapterFamily::CooperativeProjection
        );
        assert_eq!(
            state.runtime_authority,
            ProjectedPortalRuntimeAuthority::ResidentSessionLease
        );
        let wire = serde_json::to_string(&state).unwrap();
        for forbidden in ["pty", "tmux", "terminal", "stdin", "process"] {
            assert!(
                !wire.contains(forbidden),
                "projected portal state must not expose {forbidden} authority"
            );
        }
    }

    #[test]
    fn provider_kind_does_not_change_projection_semantics() {
        for (index, provider_kind) in [
            ProviderKind::Codex,
            ProviderKind::Claude,
            ProviderKind::Opencode,
            ProviderKind::Other,
        ]
        .into_iter()
        .enumerate()
        {
            let projection_id = format!("projection-provider-{index}");
            let mut authority = ProjectionAuthority::default();
            let attach = authority.handle_attach(
                AttachRequest {
                    provider_kind,
                    display_name: format!("Provider {index}"),
                    ..attach_request(&projection_id, "req-attach")
                },
                "caller-a",
                10,
            );
            assert!(attach.accepted);
            let owner_token = attach.owner_token.expect("attach returns owner token");

            assert!(
                authority
                    .handle_publish_output(
                        output_request(&projection_id, &owner_token, "req-output"),
                        "caller-a",
                        20,
                    )
                    .accepted
            );
            let feedback =
                authority.submit_portal_input(&projection_id, portal_submission("input-1", "ok"));
            assert_eq!(feedback.feedback_state, PortalInputFeedbackState::Accepted);
            let poll = authority.handle_get_pending_input(
                GetPendingInputRequest {
                    envelope: envelope(
                        ProjectionOperation::GetPendingInput,
                        &projection_id,
                        "req-poll",
                    ),
                    owner_token: owner_token.clone(),
                    max_items: None,
                    max_bytes: None,
                },
                "caller-a",
                30,
            );
            assert!(poll.accepted);
            assert_eq!(poll.pending_input.len(), 1);
            let detached = authority.handle_detach(
                DetachRequest {
                    envelope: envelope(ProjectionOperation::Detach, &projection_id, "req-detach"),
                    owner_token,
                    reason: "done".to_string(),
                },
                "caller-a",
                40,
            );
            assert!(detached.accepted);
            assert!(!authority.has_projection(&projection_id));
        }
    }

    #[test]
    fn ready_portal_update_can_be_taken_without_spending_another_rate_slot() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        let mut first = output_request("projection-a", &owner_token, "req-output-1");
        first.output_text = "first".to_string();
        first.logical_unit_id = Some("unit-first".to_string());
        let first_response = authority.handle_publish_output(first, "caller-a", 20);
        assert!(first_response.accepted);
        assert!(first_response.portal_update_ready);

        let immediate = authority
            .take_due_portal_update("projection-a", 20)
            .unwrap()
            .expect("ready publish should be immediately materializable");
        assert_eq!(immediate.unread_output_count, 1);
        assert_eq!(immediate.coalesced_output_count, 0);
        assert!(
            authority
                .take_due_portal_update("projection-a", 20)
                .unwrap()
                .is_none()
        );

        let mut second = output_request("projection-a", &owner_token, "req-output-2");
        second.output_text = "second".to_string();
        second.logical_unit_id = Some("unit-second".to_string());
        let second_response = authority.handle_publish_output(second, "caller-a", 20);
        assert!(second_response.accepted);
        assert!(!second_response.portal_update_ready);
        assert!(
            authority
                .take_due_portal_update("projection-a", 20)
                .unwrap()
                .is_none()
        );

        let coalesced = authority
            .take_due_portal_update("projection-a", PORTAL_UPDATE_RATE_WINDOW_WALL_US + 20)
            .unwrap()
            .expect("coalesced publish should become due in the next rate window");
        assert_eq!(coalesced.unread_output_count, 1);
        assert_eq!(coalesced.coalesced_output_count, 1);
        assert_eq!(
            coalesced
                .visible_transcript
                .last()
                .expect("visible transcript includes second publish")
                .output_text,
            "second"
        );
    }

    #[test]
    fn transcript_pruning_and_portal_update_rate_coalescing_are_bounded() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_retained_transcript_bytes: 12,
            max_visible_transcript_bytes: 8,
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        let mut first = output_request("projection-a", &owner_token, "req-output-1");
        first.output_text = "aaaa".to_string();
        first.logical_unit_id = Some("unit-a".to_string());
        let first_response = authority.handle_publish_output(first, "caller-a", 20);
        assert!(first_response.accepted);
        assert!(first_response.portal_update_ready);

        let mut second = output_request("projection-a", &owner_token, "req-output-2");
        second.output_text = "bbbb".to_string();
        second.logical_unit_id = Some("unit-b".to_string());
        second.coalesce_key = Some("status-line".to_string());
        let second_response = authority.handle_publish_output(second, "caller-a", 20);
        assert!(second_response.accepted);
        assert!(!second_response.portal_update_ready);
        assert_eq!(second_response.coalesced_output_count, 1);

        let mut third = output_request("projection-a", &owner_token, "req-output-3");
        third.output_text = "cccc".to_string();
        third.logical_unit_id = Some("unit-c".to_string());
        third.coalesce_key = Some("status-line".to_string());
        let third_response = authority.handle_publish_output(third, "caller-a", 20);
        assert!(third_response.accepted);
        assert!(!third_response.portal_update_ready);
        assert_eq!(third_response.coalesced_output_count, 2);

        let retained = authority.visible_transcript_window("projection-a").unwrap();
        assert_eq!(
            retained
                .iter()
                .filter(|unit| unit.coalesce_key.as_deref() == Some("status-line"))
                .count(),
            1
        );
        assert_eq!(
            retained
                .iter()
                .find(|unit| unit.coalesce_key.as_deref() == Some("status-line"))
                .unwrap()
                .output_text,
            "cccc"
        );

        let mut fourth = output_request("projection-a", &owner_token, "req-output-4");
        fourth.output_text = "dddd".to_string();
        fourth.logical_unit_id = Some("unit-d".to_string());
        assert!(
            authority
                .handle_publish_output(fourth, "caller-a", 20)
                .accepted
        );

        let mut fifth = output_request("projection-a", &owner_token, "req-output-5");
        fifth.output_text = "eeee".to_string();
        fifth.logical_unit_id = Some("unit-e".to_string());
        let fifth_response = authority.handle_publish_output(
            fifth,
            "caller-a",
            PORTAL_UPDATE_RATE_WINDOW_WALL_US + 21,
        );
        assert!(fifth_response.accepted);
        assert!(fifth_response.portal_update_ready);
        assert_eq!(fifth_response.coalesced_output_count, 3);

        let summary = authority.state_summary("projection-a").unwrap();
        assert!(summary.retained_transcript_bytes <= 12);
        assert!(summary.visible_transcript_bytes <= 8);
        assert_eq!(summary.retained_transcript_units, 3);

        let update = authority
            .take_due_portal_update("projection-a", (PORTAL_UPDATE_RATE_WINDOW_WALL_US * 2) + 22)
            .unwrap()
            .expect("update should be due in the next rate window");
        assert!(update.visible_transcript_bytes <= 8);
        assert_eq!(update.coalesced_output_count, 3);
        assert_eq!(update.unread_output_count, 5);
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .unread_output_count,
            0
        );
    }

    #[test]
    fn pending_input_bounds_and_acknowledgement_state_conflicts_are_enforced() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_pending_input_items: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "first".to_string(),
                20,
                1_000,
                None,
            )
            .unwrap();
        assert_eq!(
            authority.enqueue_input(
                "projection-a",
                "input-2",
                "second".to_string(),
                21,
                1_000,
                None
            ),
            Err(ProjectionErrorCode::ProjectionInputQueueFull)
        );

        let poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(8),
                max_bytes: None,
            },
            "caller-a",
            30,
        );
        assert!(poll.accepted);
        assert_eq!(
            poll.pending_input[0].delivery_state,
            InputDeliveryState::Delivered
        );

        let handled = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-1",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            31,
        );
        assert!(handled.accepted);

        let replay = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-2",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            32,
        );
        assert!(replay.accepted);

        let conflict = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-3",
                ),
                owner_token,
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Rejected,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            33,
        );
        assert!(!conflict.accepted);
        assert_eq!(
            conflict.error_code,
            Some(ProjectionErrorCode::ProjectionStateConflict)
        );
    }

    #[test]
    fn deferred_input_redelivers_after_not_before_and_expires_before_delivery() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "defer me".to_string(),
                20,
                100,
                None,
            )
            .unwrap();
        authority
            .enqueue_input(
                "projection-a",
                "input-2",
                "expire me".to_string(),
                21,
                45,
                None,
            )
            .unwrap();

        let first_poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll-1",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(1),
                max_bytes: None,
            },
            "caller-a",
            30,
        );
        assert_eq!(first_poll.pending_input[0].input_id, "input-1");

        let deferred = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-defer",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Deferred,
                ack_message: None,
                not_before_wall_us: Some(60),
            },
            "caller-a",
            31,
        );
        assert!(deferred.accepted);

        let hidden = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll-2",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(8),
                max_bytes: None,
            },
            "caller-a",
            50,
        );
        assert!(hidden.pending_input.is_empty());

        let redelivered = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll-3",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(8),
                max_bytes: None,
            },
            "caller-a",
            61,
        );
        assert_eq!(redelivered.pending_input.len(), 1);
        assert_eq!(redelivered.pending_input[0].input_id, "input-1");
        assert_eq!(
            redelivered.pending_input[0].delivery_state,
            InputDeliveryState::Delivered
        );

        let expired_ack = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-expired-ack",
                ),
                owner_token,
                input_id: "input-2".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            62,
        );
        assert!(!expired_ack.accepted);
        assert_eq!(
            expired_ack.error_code,
            Some(ProjectionErrorCode::ProjectionStateConflict)
        );
    }

    #[test]
    fn terminal_pending_input_is_pruned_without_losing_ack_replay() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_pending_input_items: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "first".to_string(),
                20,
                1_000,
                None,
            )
            .unwrap();

        let poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(8),
                max_bytes: None,
            },
            "caller-a",
            30,
        );
        assert!(poll.accepted);

        let handled = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-1",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            31,
        );
        assert!(handled.accepted);

        authority
            .enqueue_input(
                "projection-a",
                "input-2",
                "second".to_string(),
                32,
                1_000,
                None,
            )
            .unwrap();

        let replay = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-2",
                ),
                owner_token,
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            33,
        );
        assert!(replay.accepted);
        assert!(replay.status_summary.contains("idempotently"));
    }

    #[test]
    fn not_before_is_rejected_for_terminal_acknowledgements() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "first".to_string(),
                20,
                1_000,
                None,
            )
            .unwrap();

        let response = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack",
                ),
                owner_token,
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: Some(50),
            },
            "caller-a",
            30,
        );

        assert!(!response.accepted);
        assert_eq!(
            response.error_code,
            Some(ProjectionErrorCode::ProjectionInvalidArgument)
        );
    }

    #[test]
    fn bounded_copy_preserves_utf8_boundaries() {
        assert_eq!(bounded_copy("hello".to_string(), 10), "hello");
        assert_eq!(bounded_copy("éclair".to_string(), 1), "");
        assert_eq!(bounded_copy("aéclair".to_string(), 2), "a");
    }

    #[test]
    fn owner_cleanup_and_operator_cleanup_use_distinct_authority_paths() {
        let mut authority = ProjectionAuthority::default();
        authority.set_operator_authority("operator-secret").unwrap();
        let owner_token = attach(&mut authority, "projection-owner");
        attach(&mut authority, "projection-operator");

        let owner_cleanup = authority.handle_cleanup(
            CleanupRequest {
                envelope: envelope(
                    ProjectionOperation::Cleanup,
                    "projection-owner",
                    "req-owner-cleanup",
                ),
                cleanup_authority: CleanupAuthority::Owner,
                owner_token: Some(owner_token),
                operator_authority: None,
                reason: "owner requested detach".to_string(),
            },
            "caller-a",
            40,
        );
        assert!(owner_cleanup.accepted);

        let operator_cleanup = authority.handle_cleanup(
            CleanupRequest {
                envelope: envelope(
                    ProjectionOperation::Cleanup,
                    "projection-operator",
                    "req-operator-cleanup",
                ),
                cleanup_authority: CleanupAuthority::Operator,
                owner_token: None,
                operator_authority: Some("operator-secret".to_string()),
                reason: "operator override".to_string(),
            },
            "operator",
            41,
        );
        assert!(operator_cleanup.accepted);

        assert!(
            authority
                .audit_log()
                .iter()
                .any(|audit| audit.category == ProjectionAuditCategory::OwnerCleanup)
        );
        assert!(
            authority
                .audit_log()
                .iter()
                .any(|audit| audit.category == ProjectionAuditCategory::OperatorCleanup)
        );
    }

    #[test]
    fn token_expiry_fails_with_stable_code() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            owner_token_ttl_wall_us: 5,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        let response = authority.handle_publish_status(
            PublishStatusRequest {
                envelope: envelope(
                    ProjectionOperation::PublishStatus,
                    "projection-a",
                    "req-status",
                ),
                owner_token,
                lifecycle_state: ProjectionLifecycleState::Active,
                status_text: None,
            },
            "caller-a",
            20,
        );
        assert!(!response.accepted);
        assert_eq!(
            response.error_code,
            Some(ProjectionErrorCode::ProjectionTokenExpired)
        );
        assert!(!authority.has_projection("projection-a"));
    }

    proptest! {
        #[test]
        fn pending_input_polling_is_fifo_and_bounded(
            item_count in 1usize..24,
            requested_items in 1usize..12,
            requested_bytes in 1usize..96,
        ) {
            let mut authority = ProjectionAuthority::new(ProjectionBounds {
                max_pending_input_items: 32,
                max_pending_input_total_bytes: 4096,
                max_poll_items: 16,
                max_poll_response_bytes: 128,
                ..ProjectionBounds::default()
            })
            .unwrap();
            let owner_token = attach(&mut authority, "projection-a");
            for index in 0..item_count {
                authority
                    .enqueue_input(
                        "projection-a",
                        &format!("input-{index}"),
                        format!("msg-{index:02}"),
                        20 + index as u64,
                        10_000,
                        None,
                    )
                    .unwrap();
            }

            let poll = authority.handle_get_pending_input(
                GetPendingInputRequest {
                    envelope: envelope(
                        ProjectionOperation::GetPendingInput,
                        "projection-a",
                        "req-poll",
                    ),
                    owner_token,
                    max_items: Some(requested_items),
                    max_bytes: Some(requested_bytes),
                },
                "caller-a",
                100,
            );

            prop_assert!(poll.accepted);
            prop_assert!(poll.pending_input.len() <= requested_items.min(16));
            let returned_bytes: usize = poll
                .pending_input
                .iter()
                .map(|item| item.submission_text.len())
                .sum();
            prop_assert!(returned_bytes <= requested_bytes.min(128));
            for (index, item) in poll.pending_input.iter().enumerate() {
                prop_assert_eq!(&item.input_id, &format!("input-{index}"));
                prop_assert_eq!(item.delivery_state, InputDeliveryState::Delivered);
                prop_assert_eq!(item.delivered_at_wall_us, Some(100));
            }
            prop_assert_eq!(
                poll.pending_remaining_count + poll.pending_input.len(),
                item_count
            );
        }

        #[test]
        fn lifecycle_state_machine_never_reuses_stale_connection_or_lease(
            actions in prop::collection::vec(0u8..6, 1..32),
        ) {
            let mut authority = ProjectionAuthority::default();
            let owner_token = attach(&mut authority, "projection-a");
            let mut projection_exists = true;
            let mut has_connection = false;
            let mut lease_expires_at = None;
            let mut now = 20u64;

            for action in actions {
                now += 10;
                match action {
                    0 => {
                        let mut metadata = connection_metadata(&["create_tiles"]);
                        metadata.connection_id = format!("connection-{now}");
                        metadata.authenticated_session_id = format!("runtime-session-{now}");
                        metadata.connected_at_wall_us = now;
                        metadata.last_reconnect_wall_us = now;
                        prop_assert_eq!(
                            authority.record_hud_connection("projection-a", metadata),
                            Ok(())
                        );
                        has_connection = true;
                        lease_expires_at = None;
                    }
                    1 => {
                        let result = authority.record_heartbeat("projection-a", now);
                        if has_connection {
                            prop_assert_eq!(result, Ok(()));
                        } else {
                            prop_assert_eq!(result, Err(ProjectionErrorCode::ProjectionHudUnavailable));
                        }
                    }
                    2 => {
                        let result = authority.record_advisory_lease(
                            "projection-a",
                            advisory_lease(&["create_tiles"], now + 100),
                            now,
                        );
                        if has_connection {
                            prop_assert_eq!(result, Ok(()));
                            lease_expires_at = Some(now + 100);
                        } else {
                            prop_assert_eq!(result, Err(ProjectionErrorCode::ProjectionHudUnavailable));
                        }
                    }
                    3 => {
                        prop_assert_eq!(
                            authority.mark_hud_disconnected("projection-a", now),
                            Ok(())
                        );
                        has_connection = false;
                        lease_expires_at = None;
                    }
                    4 => {
                        let result = authority.authorize_portal_republish(
                            "projection-a",
                            "lease-1",
                            &[String::from("create_tiles")],
                            now,
                        );
                        if has_connection && lease_expires_at.is_some_and(|expires_at| now < expires_at) {
                            prop_assert_eq!(result, Ok(()));
                        } else {
                            prop_assert!(result.is_err());
                            if has_connection && lease_expires_at.is_some_and(|expires_at| now >= expires_at) {
                                prop_assert_eq!(result, Err(ProjectionErrorCode::ProjectionTokenExpired));
                                lease_expires_at = None;
                            }
                        }
                    }
                    _ => {
                        let response = authority.handle_detach(
                            DetachRequest {
                                envelope: envelope(
                                    ProjectionOperation::Detach,
                                    "projection-a",
                                    "req-detach",
                                ),
                                owner_token: owner_token.clone(),
                                reason: "property lifecycle detach".to_string(),
                            },
                            "caller-a",
                            now,
                        );
                        prop_assert!(response.accepted);
                        projection_exists = false;
                    }
                }

                if !projection_exists {
                    prop_assert!(authority.state_summary("projection-a").is_none());
                    break;
                }

                let summary = authority.state_summary("projection-a").unwrap();
                prop_assert_eq!(summary.has_hud_connection, has_connection);
                if !has_connection {
                    prop_assert!(!summary.has_advisory_lease);
                }
            }
        }
    }
}
