//! MCP error types.
//!
//! JSON-RPC 2.0 error codes follow the spec:
//! - Parse error:      -32700
//! - Invalid request:  -32600
//! - Method not found: -32601
//! - Invalid params:   -32602
//! - Internal error:   -32603
//!
//! Application-level errors use -32000 and below.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tze_hud_projection::ProjectionErrorCode;

/// Standard JSON-RPC 2.0 error codes.
pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal error — also used for capability-required denials per spec §8.3.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// Scene validation error (e.g. tab not found, lease expired).
    pub const SCENE_ERROR: i64 = -32000;
    /// The requested zone does not exist.
    pub const ZONE_NOT_FOUND: i64 = -32001;
    /// No active tab in the scene when one is required.
    pub const NO_ACTIVE_TAB: i64 = -32002;
    /// Invalid ID format (e.g. malformed UUID).
    pub const INVALID_ID: i64 = -32003;
    /// Authentication failed (bad or missing pre-shared key).
    pub const UNAUTHENTICATED: i64 = -32004;
}

/// A serializable JSON-RPC 2.0 error object.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn parse_error() -> Self {
        Self::new(codes::PARSE_ERROR, "Parse error")
    }

    pub fn invalid_request() -> Self {
        Self::new(codes::INVALID_REQUEST, "Invalid Request")
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            codes::METHOD_NOT_FOUND,
            format!("Method not found: {method}"),
        )
    }

    pub fn invalid_params(reason: impl Into<String>) -> Self {
        Self::new(codes::INVALID_PARAMS, reason.into())
    }

    pub fn internal(reason: impl Into<String>) -> Self {
        Self::new(codes::INTERNAL_ERROR, reason.into())
    }

    pub fn scene_error(reason: impl Into<String>) -> Self {
        Self::new(codes::SCENE_ERROR, reason.into())
    }

    pub fn zone_not_found(name: &str) -> Self {
        Self::new(codes::ZONE_NOT_FOUND, format!("Zone not found: {name}"))
    }

    pub fn no_active_tab() -> Self {
        Self::new(codes::NO_ACTIVE_TAB, "No active tab in scene")
    }

    pub fn invalid_id(reason: impl Into<String>) -> Self {
        Self::new(codes::INVALID_ID, reason.into())
    }

    pub fn unauthenticated() -> Self {
        Self::new(codes::UNAUTHENTICATED, "Authentication required")
    }

    /// Projection authority rejection carrying a stable `PROJECTION_*` code.
    ///
    /// Mirrors [`Self::capability_required`]: the JSON-RPC `code` stays
    /// `-32603` (internal error, the resident-MCP convention for denials), but
    /// the stable code is carried in `data.error_code` so the LLM can branch on
    /// it (`PROJECTION_TOKEN_EXPIRED` = hard stop, `PROJECTION_RATE_LIMITED` =
    /// defer) instead of seeing an opaque flattened message (hud-s8a62).
    ///
    /// Returns a JSON-RPC 2.0 error with:
    /// - code: -32603 (Internal error)
    /// - message: the human-readable rejection detail
    /// - data.error_code: the stable `PROJECTION_*` string
    /// - data.message: the same human-readable detail
    pub fn projection_rejected(
        error_code: ProjectionErrorCode,
        message: impl Into<String>,
    ) -> Self {
        let message = message.into();
        let data = serde_json::json!({
            "error_code": error_code.as_str(),
            "message": message.clone(),
        });
        Self::new(codes::INTERNAL_ERROR, message).with_data(data)
    }

    /// Capability-required error per spec §8.3.
    ///
    /// Returns a JSON-RPC 2.0 error with:
    /// - code: -32603 (Internal error, as mandated by spec §8.3)
    /// - data.error_code: "CAPABILITY_REQUIRED"
    /// - data.context: "tool=<tool_name>"
    /// - data.hint: {"required_capability": "resident_mcp", "resolution": "..."}
    pub fn capability_required(tool_name: &str) -> Self {
        let data = serde_json::json!({
            "error_code": "CAPABILITY_REQUIRED",
            "message": "Capability required",
            "context": format!("tool={tool_name}"),
            "hint": {
                "required_capability": "resident_mcp",
                "resolution": "obtain resident_mcp capability via session handshake"
            }
        });
        Self::new(codes::INTERNAL_ERROR, "Capability required").with_data(data)
    }
}

/// High-level MCP error for internal use. Converts to [`JsonRpcError`] for the wire.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("parse error: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("invalid params: {0}")]
    InvalidParams(String),

    #[error("scene error: {0}")]
    SceneError(String),

    #[error("zone not found: {0}")]
    ZoneNotFound(String),

    #[error("no active tab")]
    NoActiveTab,

    #[error("invalid id: {0}")]
    InvalidId(String),

    #[error("method not found: {0}")]
    MethodNotFound(String),

    #[error("internal error: {0}")]
    Internal(String),

    /// Caller tried to invoke a resident tool without the `resident_mcp` capability.
    /// Carries the tool name for the structured error response (spec §8.3).
    #[error("capability required to call tool: {0}")]
    CapabilityRequired(String),

    /// Authentication failed: bad or missing pre-shared key (spec §8.4).
    #[error("authentication required")]
    Unauthenticated,

    /// Projection authority rejected a `portal_projection_*` operation.
    /// Carries the stable [`ProjectionErrorCode`] so it reaches the wire as
    /// `data.error_code` instead of flattening to an opaque `-32603` message.
    #[error("projection rejected ({error_code}): {message}")]
    ProjectionRejected {
        /// Stable `PROJECTION_*` code surfaced to the LLM.
        error_code: ProjectionErrorCode,
        /// Human-readable rejection detail.
        message: String,
    },
}

impl From<McpError> for JsonRpcError {
    fn from(e: McpError) -> Self {
        match e {
            McpError::ParseError(inner) => {
                JsonRpcError::parse_error().with_data(serde_json::json!(inner.to_string()))
            }
            McpError::InvalidParams(msg) => JsonRpcError::invalid_params(msg),
            McpError::SceneError(msg) => JsonRpcError::scene_error(msg),
            McpError::ZoneNotFound(name) => JsonRpcError::zone_not_found(&name),
            McpError::NoActiveTab => JsonRpcError::no_active_tab(),
            McpError::InvalidId(msg) => JsonRpcError::invalid_id(msg),
            McpError::MethodNotFound(method) => JsonRpcError::method_not_found(&method),
            McpError::Internal(msg) => JsonRpcError::internal(msg),
            McpError::CapabilityRequired(tool) => JsonRpcError::capability_required(&tool),
            McpError::Unauthenticated => JsonRpcError::unauthenticated(),
            McpError::ProjectionRejected {
                error_code,
                message,
            } => JsonRpcError::projection_rejected(error_code, message),
        }
    }
}

impl From<tze_hud_scene::ValidationError> for McpError {
    fn from(e: tze_hud_scene::ValidationError) -> Self {
        McpError::SceneError(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tze_hud_projection::INITIAL_ERROR_CODES;

    #[test]
    fn projection_error_mapping_is_total_distinct_and_deterministic() {
        let expected_json_rpc_codes = [
            -32100, -32101, -32102, -32103, -32104, -32105, -32106, -32107, -32108,
            -32109, -32110, -32111,
        ];
        let mut observed_codes = HashSet::new();

        for (error_code, expected_json_rpc_code) in INITIAL_ERROR_CODES
            .into_iter()
            .zip(expected_json_rpc_codes)
        {
            let first = JsonRpcError::projection_rejected(
                error_code,
                "portal_projection_publish",
            );
            let second = JsonRpcError::projection_rejected(
                error_code,
                "portal_projection_publish",
            );

            assert_eq!(first.code, expected_json_rpc_code, "{error_code}");
            assert!(
                observed_codes.insert(first.code),
                "{error_code} reused JSON-RPC code {}",
                first.code
            );
            assert_eq!(first.code, second.code, "{error_code}");
            assert_eq!(first.message, second.message, "{error_code}");
            assert!(
                !first.message.is_empty() && first.message.len() <= 160,
                "{error_code} message must be bounded and self-describing: {:?}",
                first.message
            );

            let data = first.data.expect("projection errors carry structured data");
            assert_eq!(data["error_code"], error_code.as_str(), "{error_code}");
            assert_eq!(
                data["context"]["operation"],
                "portal_projection_publish",
                "{error_code} must name the failed operation"
            );
            assert_eq!(data["context"]["subsystem"], "portal_projection");
            assert!(
                data["hint"]["recovery_operation"].is_string(),
                "{error_code} must name a recovery operation"
            );
            assert!(
                data["hint"]["resolution"].is_string(),
                "{error_code} must prescribe a resolution"
            );
        }

        assert_eq!(observed_codes.len(), INITIAL_ERROR_CODES.len());
    }

    #[test]
    fn owner_token_rejections_prescribe_authenticated_idempotent_reattach() {
        for error_code in [
            ProjectionErrorCode::ProjectionUnauthorized,
            ProjectionErrorCode::ProjectionTokenExpired,
        ] {
            let wire = JsonRpcError::projection_rejected(
                error_code,
                "portal_projection_get_pending_input",
            );
            assert_ne!(wire.code, codes::INTERNAL_ERROR, "{error_code}");
            let data = wire.data.expect("owner-token rejection has recovery data");
            assert_eq!(
                data["hint"]["recovery_operation"],
                "portal_projection_attach"
            );
            let resolution = data["hint"]["resolution"]
                .as_str()
                .expect("resolution is text");
            assert!(resolution.contains("authenticated"), "{resolution}");
            assert!(resolution.contains("original idempotency_key"), "{resolution}");
            assert!(resolution.contains("rotate the owner token"), "{resolution}");
        }
    }
}
