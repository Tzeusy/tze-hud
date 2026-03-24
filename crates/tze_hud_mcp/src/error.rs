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
        Self::new(codes::METHOD_NOT_FOUND, format!("Method not found: {method}"))
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
        }
    }
}

impl From<tze_hud_scene::ValidationError> for McpError {
    fn from(e: tze_hud_scene::ValidationError) -> Self {
        McpError::SceneError(e.to_string())
    }
}
