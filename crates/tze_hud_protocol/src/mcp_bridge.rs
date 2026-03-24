//! MCP bridge error codes and shared constants.
//!
//! Per spec §8.5: MCP errors SHALL use JSON-RPC 2.0 error objects with a
//! structured `data` field matching the `RuntimeError` proto (error_code,
//! message, context, hint). Error codes SHALL be the same stable codes used
//! in gRPC [`RuntimeError`][crate::proto::session::RuntimeError] responses.
//!
//! This module provides the canonical error code strings so that both the
//! MCP bridge (`tze_hud_mcp`) and the gRPC session layer share the same
//! stable identifiers.

/// Stable error codes shared across the MCP JSON-RPC bridge and the gRPC
/// `RuntimeError` message.
///
/// These are the `error_code` field values in both the JSON-RPC `data` object
/// (MCP bridge) and the protobuf `RuntimeError.error_code` field (gRPC session
/// layer).  Keeping them in one place prevents drift.
pub mod error_codes {
    /// A required capability was not granted to the calling agent.
    ///
    /// MCP context: returned when a guest agent calls a resident tool without
    /// the `resident_mcp` capability.  The `hint` field carries
    /// `{"required_capability": "resident_mcp", "resolution": "..."}`.
    pub const CAPABILITY_REQUIRED: &str = "CAPABILITY_REQUIRED";

    /// The agent's lease has expired.
    ///
    /// Returned by both MCP tool calls and gRPC mutation processing when the
    /// submitting agent's lease is no longer valid.
    pub const LEASE_EXPIRED: &str = "LEASE_EXPIRED";

    /// The scene mutation was rejected because safe mode is active.
    ///
    /// See `SessionSuspended` / `SessionResumed` in the session protocol.
    pub const SAFE_MODE_ACTIVE: &str = "SAFE_MODE_ACTIVE";

    /// The supplied timestamp is too old relative to the session open time.
    pub const TIMESTAMP_TOO_OLD: &str = "TIMESTAMP_TOO_OLD";

    /// The supplied timestamp is too far in the future.
    pub const TIMESTAMP_TOO_FUTURE: &str = "TIMESTAMP_TOO_FUTURE";

    /// `expires_at` is before or equal to `present_at` on a mutation.
    pub const TIMESTAMP_EXPIRY_BEFORE_PRESENT: &str = "TIMESTAMP_EXPIRY_BEFORE_PRESENT";

    /// A requested resource (tile, tab, zone) was not found.
    pub const NOT_FOUND: &str = "NOT_FOUND";

    /// The request was denied due to insufficient permissions.
    pub const PERMISSION_DENIED: &str = "PERMISSION_DENIED";
}

/// Build a JSON-RPC 2.0 `error.data` object that matches the `RuntimeError`
/// proto structure.
///
/// This is a convenience helper for constructing the structured error payload
/// required by spec §8.5.
///
/// # Example
///
/// ```rust
/// use tze_hud_protocol::mcp_bridge::{error_codes, build_runtime_error_data};
/// let data = build_runtime_error_data(
///     error_codes::LEASE_EXPIRED,
///     "Lease has expired",
///     Some("lease_id=abc123"),
///     None,
/// );
/// assert_eq!(data["error_code"], "LEASE_EXPIRED");
/// ```
pub fn build_runtime_error_data(
    error_code: &str,
    message: &str,
    context: Option<&str>,
    hint: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "error_code": error_code,
        "message": message,
    });

    if let Some(ctx) = context {
        obj["context"] = serde_json::Value::String(ctx.to_owned());
    }

    if let Some(h) = hint {
        obj["hint"] = h;
    }

    obj
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_runtime_error_data_minimal() {
        let data = build_runtime_error_data(error_codes::LEASE_EXPIRED, "Lease expired", None, None);
        assert_eq!(data["error_code"], "LEASE_EXPIRED");
        assert_eq!(data["message"], "Lease expired");
        assert!(data.get("context").is_none() || data["context"].is_null());
    }

    #[test]
    fn test_build_runtime_error_data_with_context_and_hint() {
        let hint = serde_json::json!({"required_capability": "resident_mcp"});
        let data = build_runtime_error_data(
            error_codes::CAPABILITY_REQUIRED,
            "Capability required",
            Some("tool=create_tile"),
            Some(hint),
        );
        assert_eq!(data["error_code"], "CAPABILITY_REQUIRED");
        assert_eq!(data["context"], "tool=create_tile");
        assert_eq!(data["hint"]["required_capability"], "resident_mcp");
    }

    #[test]
    fn test_error_codes_are_stable() {
        // Verify the string values are exactly as spec defines them.
        assert_eq!(error_codes::CAPABILITY_REQUIRED, "CAPABILITY_REQUIRED");
        assert_eq!(error_codes::LEASE_EXPIRED, "LEASE_EXPIRED");
        assert_eq!(error_codes::SAFE_MODE_ACTIVE, "SAFE_MODE_ACTIVE");
    }
}
