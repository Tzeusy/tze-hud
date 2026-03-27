//! JSON-RPC 2.0 wire types.

use crate::error::JsonRpcError;
use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 request object.
#[derive(Clone, Debug, Deserialize)]
pub struct McpRequest {
    /// Must be "2.0".
    pub jsonrpc: String,
    /// Method (tool) name.
    pub method: String,
    /// Tool parameters as a JSON object.
    #[serde(default)]
    pub params: serde_json::Value,
    /// Request ID (string, number, or null).
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response object.
#[derive(Clone, Debug, Serialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<serde_json::Value>,
}

/// Type alias for tool handler results.
pub type McpResult<T> = Result<T, crate::McpError>;

impl McpResponse {
    /// Successful response.
    pub fn ok(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Error response.
    pub fn err(id: Option<serde_json::Value>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(error),
            id,
        }
    }
}
