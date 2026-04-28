//! Provider-neutral cooperative HUD projection operation contract.
//!
//! This crate owns the low-token operation schema for the external projection
//! authority described by `openspec/changes/cooperative-hud-projection/`.
//! It deliberately models projection-daemon operations, not runtime v1 MCP
//! tools. If the contract is exposed through MCP, that MCP server belongs to
//! the projection daemon and talks outward to the HUD over the resident control
//! plane.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
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
/// Owner tokens are 256-bit random values encoded as lowercase hex.
pub const OWNER_TOKEN_ENTROPY_BITS: usize = 256;
/// Default owner-token lifetime in wall-clock microseconds.
pub const DEFAULT_OWNER_TOKEN_TTL_WALL_US: u64 = 24 * 60 * 60 * 1_000_000;

const MAX_PROJECTION_ID_BYTES: usize = 128;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_CALLER_IDENTITY_BYTES: usize = 256;
const MAX_DISPLAY_NAME_BYTES: usize = 128;
const MAX_HINT_BYTES: usize = 256;
const MAX_STATUS_SUMMARY_BYTES: usize = 512;
const MAX_REASON_BYTES: usize = 512;
const MAX_ACK_MESSAGE_BYTES: usize = 512;

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
                "operation must be {:?}",
                expected
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
        }
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
    provider_kind: ProviderKind,
    display_name: String,
    owner_token_verifier: String,
    owner_token_expires_at_wall_us: u64,
    lifecycle_state: ProjectionLifecycleState,
    content_classification: ContentClassification,
    attach_idempotency_key: Option<String>,
    retained_transcript_bytes: usize,
    seen_logical_units: HashSet<String>,
    pending_input: VecDeque<PendingInputItem>,
    pending_input_bytes: usize,
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
                self.audit(
                    &request.envelope,
                    caller_identity,
                    server_timestamp_wall_us,
                    true,
                    None,
                    "idempotent attach replay",
                    ProjectionAuditCategory::Attach,
                );
                return response;
            }
            let response = ProjectionResponse::denied(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                ProjectionErrorCode::ProjectionAlreadyAttached,
                "projection_id is already attached",
            );
            self.audit(
                &request.envelope,
                caller_identity,
                server_timestamp_wall_us,
                false,
                Some(ProjectionErrorCode::ProjectionAlreadyAttached),
                "attach conflict",
                ProjectionAuditCategory::ConflictDenied,
            );
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
                provider_kind: request.provider_kind,
                display_name: request.display_name,
                owner_token_verifier,
                owner_token_expires_at_wall_us: server_timestamp_wall_us
                    + self.bounds.owner_token_ttl_wall_us,
                lifecycle_state: ProjectionLifecycleState::Attached,
                content_classification: request.content_classification,
                attach_idempotency_key: request.idempotency_key,
                retained_transcript_bytes: 0,
                seen_logical_units: HashSet::new(),
                pending_input: VecDeque::new(),
                pending_input_bytes: 0,
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
        self.audit(
            &request.envelope,
            caller_identity,
            server_timestamp_wall_us,
            true,
            None,
            "attach accepted",
            ProjectionAuditCategory::Attach,
        );
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
        let response = match self.authorize_owner(
            &request.envelope,
            &request.owner_token,
            server_timestamp_wall_us,
            ProjectionAuditCategory::OwnerPublish,
        ) {
            Ok(session) => {
                if let Some(logical_unit_id) = &request.logical_unit_id {
                    if !session.seen_logical_units.insert(logical_unit_id.clone()) {
                        ProjectionResponse::accepted(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            "duplicate logical_unit_id accepted idempotently",
                        )
                    } else {
                        append_transcript_bytes(
                            session,
                            request.output_text.len(),
                            max_retained_transcript_bytes,
                        );
                        ProjectionResponse::accepted(
                            &request.envelope.request_id,
                            &request.envelope.projection_id,
                            server_timestamp_wall_us,
                            "output accepted",
                        )
                    }
                } else {
                    append_transcript_bytes(
                        session,
                        request.output_text.len(),
                        max_retained_transcript_bytes,
                    );
                    ProjectionResponse::accepted(
                        &request.envelope.request_id,
                        &request.envelope.projection_id,
                        server_timestamp_wall_us,
                        "output accepted",
                    )
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
        let session = self
            .sessions
            .get_mut(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        if submission_text.len() > self.bounds.max_pending_input_bytes_per_item {
            return Err(ProjectionErrorCode::ProjectionInputTooLarge);
        }
        if session.pending_input.len() >= self.bounds.max_pending_input_items {
            return Err(ProjectionErrorCode::ProjectionInputQueueFull);
        }
        if session.pending_input_bytes + submission_text.len()
            > self.bounds.max_pending_input_total_bytes
        {
            return Err(ProjectionErrorCode::ProjectionInputQueueFull);
        }
        session.pending_input_bytes += submission_text.len();
        session.pending_input.push_back(PendingInputItem {
            input_id: input_id.to_string(),
            projection_id: projection_id.to_string(),
            submission_text,
            submitted_at_wall_us,
            expires_at_wall_us,
            delivery_state: InputDeliveryState::Pending,
            delivered_at_wall_us: None,
            not_before_wall_us: None,
            content_classification: content_classification.unwrap_or_default(),
        });
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
        let session = self
            .sessions
            .get_mut(&envelope.projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        if server_timestamp_wall_us >= session.owner_token_expires_at_wall_us {
            return Err(ProjectionErrorCode::ProjectionTokenExpired);
        }
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
        self.audit(
            envelope,
            caller_identity,
            server_timestamp_wall_us,
            response.accepted,
            response.error_code,
            &response.status_summary,
            category,
        );
    }

    fn audit(
        &mut self,
        envelope: &OperationEnvelope,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
        accepted: bool,
        error_code: Option<ProjectionErrorCode>,
        reason: &str,
        category: ProjectionAuditCategory,
    ) {
        self.audit_log.push(ProjectionAuditRecord {
            timestamp_wall_us: server_timestamp_wall_us,
            operation: envelope.operation,
            projection_id: envelope.projection_id.clone(),
            caller_identity: bounded_copy(caller_identity.to_string(), MAX_CALLER_IDENTITY_BYTES),
            request_id: envelope.request_id.clone(),
            accepted,
            error_code,
            reason: bounded_copy(reason.to_string(), MAX_REASON_BYTES),
            category,
        });
    }
}

impl Default for ProjectionAuthority {
    fn default() -> Self {
        Self::new(ProjectionBounds::default()).expect("default projection bounds are valid")
    }
}

fn append_transcript_bytes(
    session: &mut ProjectionSession,
    output_bytes: usize,
    max_retained_transcript_bytes: usize,
) {
    session.retained_transcript_bytes =
        (session.retained_transcript_bytes + output_bytes).min(max_retained_transcript_bytes);
    session.lifecycle_state = ProjectionLifecycleState::Active;
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
        return ProjectionResponse::denied(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            ProjectionErrorCode::ProjectionNotFound,
            "input_id not found",
        );
    };

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
        let requested_terminal = match request.ack_state {
            InputAckState::Handled => InputDeliveryState::Handled,
            InputAckState::Rejected => InputDeliveryState::Rejected,
            InputAckState::Deferred => InputDeliveryState::Deferred,
        };
        if item.delivery_state == requested_terminal {
            return ProjectionResponse::accepted(
                &request.envelope.request_id,
                &request.envelope.projection_id,
                server_timestamp_wall_us,
                "terminal acknowledgement replay accepted idempotently",
            );
        }
        return ProjectionResponse::denied(
            &request.envelope.request_id,
            &request.envelope.projection_id,
            server_timestamp_wall_us,
            ProjectionErrorCode::ProjectionStateConflict,
            "conflicting acknowledgement for terminal input",
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

fn generate_owner_token() -> Result<String, ProjectionContractError> {
    let mut token_bytes = [0u8; OWNER_TOKEN_ENTROPY_BITS / 8];
    getrandom::fill(&mut token_bytes).map_err(|_| ProjectionContractError::TokenGeneration)?;
    Ok(hex_encode(&token_bytes))
}

fn verifier_for_secret(secret: &str) -> String {
    blake3::hash(secret.as_bytes()).to_hex().to_string()
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.as_bytes().iter().zip(right.as_bytes()) {
        diff |= a ^ b;
    }
    diff == 0
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
    value.truncate(max_bytes);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
