//! MCP JSON-RPC server.
//!
//! The server wraps the shared [`SceneGraph`] and dispatches incoming
//! JSON-RPC 2.0 requests to the appropriate tool handler.
//!
//! ## Authentication
//!
//! Every call must carry a pre-shared key (PSK).  The PSK may be supplied as:
//!
//! 1. A JSON-RPC `params` top-level field `"_auth"` (string): preferred for
//!    stdio/NDJSON transports where custom headers are unavailable.
//! 2. An HTTP `Authorization: Bearer <key>` header value passed by the caller
//!    via [`CallerContext::with_bearer`].
//!
//! Each call is authenticated independently — there is no persistent session
//! state.  The PSK is never echoed in responses.
//!
//! ## Guest vs Resident tools
//!
//! **Guest tools** (unconditionally accessible to any authenticated caller):
//! - `publish_to_zone`
//! - `list_zones`
//! - `list_scene`
//!
//! **Resident tools** (require the `resident_mcp` capability):
//! - `create_tab`
//! - `create_tile`
//! - `set_content`
//! - `dismiss`
//!
//! Calling a resident tool without the capability returns a structured
//! JSON-RPC 2.0 error: code -32603, `data.error_code="CAPABILITY_REQUIRED"`.
//!
//! ## Transport
//!
//! The server is transport-agnostic at this layer: [`McpServer::dispatch`]
//! accepts a raw `&str` (the JSON-RPC request body) plus a [`CallerContext`]
//! and returns a `String` (the JSON-RPC response body). Callers can wire this
//! to HTTP, stdio, or any other transport.

use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use tze_hud_scene::graph::SceneGraph;
use tracing::{debug, error, warn};

use crate::{
    error::{JsonRpcError, McpError},
    tools,
    types::{McpRequest, McpResponse},
};

// ─── Caller context ──────────────────────────────────────────────────────────

/// Authentication and capability context for a single MCP call.
///
/// This is constructed by the transport layer and passed to
/// [`McpServer::dispatch`].  It carries:
/// - The pre-shared key (if available from the transport).
/// - The set of capabilities granted to this caller (e.g. `resident_mcp`).
///
/// Per spec §8.4: each call is authenticated independently — no persistent
/// session state is maintained at this layer.
#[derive(Clone, Debug, Default)]
pub struct CallerContext {
    /// Pre-shared key extracted from the transport (e.g. Bearer token from an
    /// HTTP header).  If `None`, only in-params auth is attempted.
    pub bearer_token: Option<String>,

    /// Capabilities granted to this caller.  The only capability that affects
    /// tool routing in v1 is `resident_mcp`.
    pub capabilities: Vec<String>,
}

impl CallerContext {
    /// Create an unauthenticated context with no capabilities.
    pub fn guest() -> Self {
        Self::default()
    }

    /// Create a context with a bearer token (from HTTP `Authorization` header).
    pub fn with_bearer(token: impl Into<String>) -> Self {
        Self {
            bearer_token: Some(token.into()),
            capabilities: vec![],
        }
    }

    /// Grant the `resident_mcp` capability, allowing resident tool access.
    pub fn with_resident_mcp(mut self) -> Self {
        self.capabilities.push("resident_mcp".to_string());
        self
    }

    /// Returns `true` if this context has the `resident_mcp` capability.
    pub fn has_resident_mcp(&self) -> bool {
        self.capabilities.iter().any(|c| c == "resident_mcp")
    }
}

// ─── Server configuration ────────────────────────────────────────────────────

/// Server-level configuration.
///
/// If `pre_shared_key` is `None`, authentication is skipped and all calls
/// are treated as authenticated (useful for tests and local-only deployments).
#[derive(Clone, Debug, Default)]
pub struct McpConfig {
    /// Optional pre-shared key for MCP authentication.  When set, every call
    /// must supply a matching key via bearer token or `_auth` param.
    pub pre_shared_key: Option<String>,
}

impl McpConfig {
    /// Require authentication with the given PSK.
    pub fn with_psk(key: impl Into<String>) -> Self {
        Self {
            pre_shared_key: Some(key.into()),
        }
    }
}

// ─── Guest / Resident tool classification ────────────────────────────────────

/// Tool categories per spec §8.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolClass {
    /// Accessible to any authenticated caller, no capability required.
    Guest,
    /// Requires the `resident_mcp` capability.
    Resident,
    /// Unrecognised method.
    Unknown,
}

fn classify_tool(method: &str) -> ToolClass {
    match method {
        // Guest tools — unconditionally accessible
        "publish_to_zone" | "list_zones" | "list_scene" => ToolClass::Guest,
        // Resident tools — require resident_mcp capability
        "create_tab" | "create_tile" | "set_content" | "dismiss" => ToolClass::Resident,
        _ => ToolClass::Unknown,
    }
}

// ─── Server ──────────────────────────────────────────────────────────────────

/// Shared server state — wraps the scene graph behind an async mutex so that
/// concurrent requests serialize scene mutations safely.
pub struct McpServer {
    scene: Arc<Mutex<SceneGraph>>,
    config: McpConfig,
}

impl McpServer {
    /// Create a new server backed by the given scene graph with no auth.
    pub fn new(scene: SceneGraph) -> Self {
        Self {
            scene: Arc::new(Mutex::new(scene)),
            config: McpConfig::default(),
        }
    }

    /// Create a new server sharing an existing arc-wrapped scene graph.
    ///
    /// Use this when the scene is also shared with the gRPC control plane.
    pub fn with_shared_scene(scene: Arc<Mutex<SceneGraph>>) -> Self {
        Self {
            scene,
            config: McpConfig::default(),
        }
    }

    /// Attach server configuration (auth settings, etc.).
    pub fn with_config(mut self, config: McpConfig) -> Self {
        self.config = config;
        self
    }

    /// Borrow the underlying shared scene handle.
    pub fn scene_handle(&self) -> Arc<Mutex<SceneGraph>> {
        Arc::clone(&self.scene)
    }

    /// Dispatch a raw JSON-RPC 2.0 request body and return the response body.
    ///
    /// `ctx` carries authentication material and capabilities for this call.
    /// Per spec §8.4 each call is authenticated independently.
    ///
    /// This is the single entry point for all transports.
    pub async fn dispatch(&self, body: &str, ctx: &CallerContext) -> String {
        // Parse the request
        let mut request: McpRequest = match serde_json::from_str(body) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "MCP: failed to parse JSON-RPC request");
                let resp = McpResponse::err(None, JsonRpcError::parse_error());
                return serde_json::to_string(&resp).unwrap_or_default();
            }
        };

        // Validate JSON-RPC version
        if request.jsonrpc != "2.0" {
            let resp = McpResponse::err(
                request.id.clone(),
                JsonRpcError::invalid_request(),
            );
            return serde_json::to_string(&resp).unwrap_or_default();
        }

        // ── Per-call authentication (spec §8.4) ──────────────────────────────
        if let Some(ref expected) = self.config.pre_shared_key {
            // Attempt auth from two independent sources (spec §8.4: either is valid):
            // 1. CallerContext bearer token (from HTTP Authorization header).
            // 2. `_auth` param field in the JSON-RPC params object.
            //
            // Both are checked independently — if a bearer token is present but
            // wrong, the `_auth` param can still authenticate the call.  This
            // prevents a rogue/stale transport header from blocking valid in-params auth.
            //
            // Constant-time comparison (via `subtle`) prevents timing side-channels.
            let bearer_key = ctx.bearer_token.as_deref();
            let param_key = request
                .params
                .as_object()
                .and_then(|o| o.get("_auth"))
                .and_then(|v| v.as_str());

            let expected_bytes = expected.as_bytes();
            let authenticated = bearer_key
                .map(|k| k.as_bytes().ct_eq(expected_bytes).into())
                .unwrap_or(false)
                || param_key
                    .map(|k| k.as_bytes().ct_eq(expected_bytes).into())
                    .unwrap_or(false);

            if authenticated {
                // Authenticated. Strip _auth from params so handlers never
                // see it (avoids unknown-field errors in typed params structs).
                if let Some(obj) = request.params.as_object_mut() {
                    obj.remove("_auth");
                }
            } else {
                warn!(method = %request.method, "MCP: authentication failed");
                let resp = McpResponse::err(
                    request.id.clone(),
                    JsonRpcError::from(McpError::Unauthenticated),
                );
                return serde_json::to_string(&resp).unwrap_or_default();
            }
        }

        debug!(method = %request.method, "MCP: dispatching tool call");

        // ── Guest / Resident capability gate (spec §8.1, §8.3) ──────────────
        let tool_class = classify_tool(&request.method);

        if tool_class == ToolClass::Unknown {
            let resp = McpResponse::err(
                request.id.clone(),
                JsonRpcError::method_not_found(&request.method),
            );
            return serde_json::to_string(&resp).unwrap_or_default();
        }

        if tool_class == ToolClass::Resident && !ctx.has_resident_mcp() {
            // Return structured CAPABILITY_REQUIRED error per spec §8.3.
            warn!(
                method = %request.method,
                "MCP: resident tool called without resident_mcp capability"
            );
            let resp = McpResponse::err(
                request.id.clone(),
                JsonRpcError::capability_required(&request.method),
            );
            return serde_json::to_string(&resp).unwrap_or_default();
        }

        let id = request.id.clone();
        let result = self.invoke_tool(&request.method, request.params).await;

        let response = match result {
            Ok(value) => McpResponse::ok(id, value),
            Err(e) => {
                error!(error = %e, method = %request.method, "MCP: tool error");
                McpResponse::err(id, JsonRpcError::from(e))
            }
        };

        serde_json::to_string(&response).unwrap_or_else(|e| {
            error!(error = %e, "MCP: failed to serialize response");
            r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal serialization error"},"id":null}"#.to_string()
        })
    }

    /// Invoke the named tool with the given parameters.
    async fn invoke_tool(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, crate::McpError> {
        let mut scene = self.scene.lock().await;

        match method {
            "create_tab" => {
                let r = tools::handle_create_tab(params, &mut scene)?;
                Ok(serde_json::to_value(r)
                    .map_err(|e| crate::McpError::Internal(e.to_string()))?)
            }
            "create_tile" => {
                let r = tools::handle_create_tile(params, &mut scene)?;
                Ok(serde_json::to_value(r)
                    .map_err(|e| crate::McpError::Internal(e.to_string()))?)
            }
            "set_content" => {
                let r = tools::handle_set_content(params, &mut scene)?;
                Ok(serde_json::to_value(r)
                    .map_err(|e| crate::McpError::Internal(e.to_string()))?)
            }
            "dismiss" => {
                let r = tools::handle_dismiss(params, &mut scene)?;
                Ok(serde_json::to_value(r)
                    .map_err(|e| crate::McpError::Internal(e.to_string()))?)
            }
            "publish_to_zone" => {
                let r = tools::handle_publish_to_zone(params, &mut scene)?;
                Ok(serde_json::to_value(r)
                    .map_err(|e| crate::McpError::Internal(e.to_string()))?)
            }
            "list_zones" => {
                let r = tools::handle_list_zones(params, &scene)?;
                Ok(serde_json::to_value(r)
                    .map_err(|e| crate::McpError::Internal(e.to_string()))?)
            }
            "list_scene" => {
                let r = tools::handle_list_scene(params, &scene)?;
                Ok(serde_json::to_value(r)
                    .map_err(|e| crate::McpError::Internal(e.to_string()))?)
            }
            unknown => Err(crate::McpError::MethodNotFound(unknown.to_string())),
        }
    }

    /// Run a minimal HTTP server on the given address, dispatching all POST `/`
    /// requests to [`Self::dispatch`].
    ///
    /// This is a lightweight reference implementation for integration tests and
    /// development. It is intentionally simple: one-request-at-a-time parsing
    /// on a single TCP stream, no TLS, no keep-alive.
    ///
    /// For production use, wire [`Self::dispatch`] into your HTTP framework of
    /// choice (axum, actix-web, hyper, etc.).
    #[cfg(feature = "http")]
    pub async fn run_http(self: Arc<Self>, addr: &str) -> std::io::Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind(addr).await?;
        tracing::info!(addr = %addr, "MCP HTTP server listening");

        loop {
            let (mut stream, peer) = listener.accept().await?;
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                let n = match stream.read(&mut buf).await {
                    Ok(n) if n == 0 => return,
                    Ok(n) => n,
                    Err(e) => {
                        error!(peer = %peer, error = %e, "MCP: read error");
                        return;
                    }
                };

                // Extract headers and body from the raw HTTP request
                let raw = &buf[..n];
                let (header_section, body) =
                    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&raw[..pos]).unwrap_or("");
                        let body = std::str::from_utf8(&raw[pos + 4..]).unwrap_or("");
                        (headers, body)
                    } else {
                        ("", std::str::from_utf8(raw).unwrap_or(""))
                    };

                // Extract Bearer token from Authorization header.
                // Header name matching is case-insensitive (HTTP/1.1 §3.2).
                // Scheme matching is also case-insensitive per RFC 7235 §2.1
                // ("Bearer" is the registered scheme but clients may lowercase it).
                let bearer_token = header_section
                    .lines()
                    .find(|l| l.to_lowercase().starts_with("authorization:"))
                    .and_then(|l| l.splitn(2, ':').nth(1))
                    .map(|v| v.trim())
                    .and_then(|v| {
                        // Split into scheme + credentials; accept any case of "Bearer".
                        let mut parts = v.splitn(2, ' ');
                        match (parts.next(), parts.next()) {
                            (Some(scheme), Some(credentials))
                                if scheme.eq_ignore_ascii_case("bearer") =>
                            {
                                Some(credentials.trim().to_owned())
                            }
                            _ => None,
                        }
                    });

                let ctx = if let Some(token) = bearer_token {
                    CallerContext::with_bearer(token)
                } else {
                    CallerContext::guest()
                };

                let response_body = server.dispatch(body, &ctx).await;

                let http_response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );

                if let Err(e) = stream.write_all(http_response.as_bytes()).await {
                    error!(peer = %peer, error = %e, "MCP: write error");
                }
            });
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tze_hud_scene::{graph::SceneGraph, types::{ZoneDefinition, GeometryPolicy, ZoneMediaType, RenderingPolicy, ContentionPolicy, LayerAttachment}, SceneId};

    async fn server_with_tab() -> (McpServer, SceneId) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).expect("create tab");
        let server = McpServer::new(scene);
        (server, tab_id)
    }

    fn parse_response(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).expect("valid JSON response")
    }

    fn guest() -> CallerContext {
        CallerContext::guest()
    }

    fn resident() -> CallerContext {
        CallerContext::guest().with_resident_mcp()
    }

    // ── JSON-RPC protocol compliance ─────────────────────────────────────────

    #[tokio::test]
    async fn test_malformed_json_returns_parse_error() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server.dispatch("{not valid json", &guest()).await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn test_wrong_jsonrpc_version_returns_invalid_request() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(r#"{"jsonrpc":"1.0","method":"list_zones","id":1}"#, &guest())
            .await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32600);
    }

    #[tokio::test]
    async fn test_unknown_method_returns_method_not_found() {
        let (server, _) = server_with_tab().await;
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"does_not_exist","id":1}"#, &guest())
            .await;
        let resp = parse_response(&raw);
        // JSON-RPC 2.0: unknown method must return -32601 Method not found
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn test_request_id_echoed_in_response() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":42}"#, &guest())
            .await;
        let resp = parse_response(&raw);
        assert_eq!(resp["id"], 42);
    }

    // ── create_tab (resident) ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_create_tab() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tab","params":{"name":"Alerts"},"id":1}"#,
                &resident(),
            )
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null());
        assert!(!resp["result"]["tab_id"].as_str().unwrap().is_empty());
        assert_eq!(resp["result"]["name"], "Alerts");
    }

    // ── create_tile (resident) ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_create_tile() {
        let (server, _) = server_with_tab().await;
        let raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tile","params":{"namespace":"a","bounds":{"x":0,"y":0,"width":200,"height":200}},"id":2}"#,
                &resident(),
            )
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
        assert!(!resp["result"]["tile_id"].as_str().unwrap().is_empty());
    }

    // ── set_content (resident) ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_set_content() {
        let (server, _) = server_with_tab().await;

        // First create a tile (resident)
        let tile_raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tile","params":{"namespace":"a","bounds":{"x":0,"y":0,"width":200,"height":200}},"id":1}"#,
                &resident(),
            )
            .await;
        let tile_resp = parse_response(&tile_raw);
        let tile_id = tile_resp["result"]["tile_id"].as_str().unwrap();

        let content_req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "set_content",
            "params": {"tile_id": tile_id, "content": "# Hello MCP"},
            "id": 2
        });
        let raw = server.dispatch(&content_req.to_string(), &resident()).await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
        assert_eq!(resp["result"]["content_len"], 11);
    }

    // ── publish_to_zone (guest) ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_publish_to_zone() {
        let (server, _) = server_with_tab().await;

        // Register a zone directly on the shared scene
        {
            let mut scene = server.scene.lock().await;
            scene.zone_registry.zones.insert(
                "status".to_string(),
                ZoneDefinition {
                    id: SceneId::new(),
                    name: "status".to_string(),
                    description: "Status zone".to_string(),
                    geometry_policy: GeometryPolicy::Relative { x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 0.05 },
                    accepted_media_types: vec![ZoneMediaType::StreamText],
                    rendering_policy: RenderingPolicy::default(),
                    contention_policy: ContentionPolicy::LatestWins,
                    max_publishers: 4,
                    transport_constraint: None,
                    auto_clear_ms: None,
                    ephemeral: false,
                    layer_attachment: LayerAttachment::Content,
                },
            );
        }

        // Guest caller can publish to zone without any capabilities
        let req = json!({
            "jsonrpc": "2.0",
            "method": "publish_to_zone",
            "params": {"zone_name": "status", "content": "All systems go"},
            "id": 3
        });
        let raw = server.dispatch(&req.to_string(), &guest()).await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
        assert_eq!(resp["result"]["zone_name"], "status");
    }

    // ── list_zones (guest) ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_list_zones_empty() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":4}"#, &guest())
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null());
        assert_eq!(resp["result"]["count"], 0);
    }

    #[tokio::test]
    async fn test_dispatch_list_zones_populated() {
        let (server, _) = server_with_tab().await;

        {
            let mut scene = server.scene.lock().await;
            scene.zone_registry.zones.insert(
                "hud".to_string(),
                ZoneDefinition {
                    id: SceneId::new(),
                    name: "hud".to_string(),
                    description: "HUD zone".to_string(),
                    geometry_policy: GeometryPolicy::Relative { x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 0.05 },
                    accepted_media_types: vec![ZoneMediaType::StreamText],
                    rendering_policy: RenderingPolicy::default(),
                    contention_policy: ContentionPolicy::LatestWins,
                    max_publishers: 4,
                    transport_constraint: None,
                    auto_clear_ms: None,
                    ephemeral: false,
                    layer_attachment: LayerAttachment::Content,
                },
            );
        }

        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":{},"id":5}"#, &guest())
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null());
        assert_eq!(resp["result"]["count"], 1);
        assert_eq!(resp["result"]["zones"][0]["name"], "hud");
    }

    // ── Guest / Resident access control (spec §8.1, §8.3) ───────────────────

    #[tokio::test]
    async fn test_guest_cannot_call_resident_tool_create_tile() {
        let (server, _) = server_with_tab().await;
        let raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tile","params":{"namespace":"a","bounds":{"x":0,"y":0,"width":200,"height":200}},"id":10}"#,
                &guest(),
            )
            .await;
        let resp = parse_response(&raw);
        // code must be -32603 per spec §8.3
        assert_eq!(resp["error"]["code"], -32603);
        assert_eq!(resp["error"]["data"]["error_code"], "CAPABILITY_REQUIRED");
        assert_eq!(resp["error"]["data"]["context"], "tool=create_tile");
        assert_eq!(resp["error"]["data"]["hint"]["required_capability"], "resident_mcp");
    }

    #[tokio::test]
    async fn test_guest_cannot_call_create_tab() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tab","params":{"name":"X"},"id":11}"#,
                &guest(),
            )
            .await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32603);
        assert_eq!(resp["error"]["data"]["error_code"], "CAPABILITY_REQUIRED");
        assert_eq!(resp["error"]["data"]["context"], "tool=create_tab");
    }

    #[tokio::test]
    async fn test_guest_cannot_call_set_content() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let fake_id = SceneId::new().to_string();
        let req = json!({
            "jsonrpc": "2.0",
            "method": "set_content",
            "params": {"tile_id": fake_id, "content": "hi"},
            "id": 12
        });
        let raw = server.dispatch(&req.to_string(), &guest()).await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32603);
        assert_eq!(resp["error"]["data"]["error_code"], "CAPABILITY_REQUIRED");
    }

    #[tokio::test]
    async fn test_guest_cannot_call_dismiss() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let fake_id = SceneId::new().to_string();
        let req = json!({
            "jsonrpc": "2.0",
            "method": "dismiss",
            "params": {"tile_id": fake_id},
            "id": 13
        });
        let raw = server.dispatch(&req.to_string(), &guest()).await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32603);
        assert_eq!(resp["error"]["data"]["error_code"], "CAPABILITY_REQUIRED");
    }

    #[tokio::test]
    async fn test_structured_error_has_hint_field() {
        let (server, _) = server_with_tab().await;
        let raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tile","params":{"namespace":"a","bounds":{"x":0,"y":0,"width":200,"height":200}},"id":14}"#,
                &guest(),
            )
            .await;
        let resp = parse_response(&raw);
        let hint = &resp["error"]["data"]["hint"];
        assert_eq!(hint["required_capability"], "resident_mcp");
        assert_eq!(hint["resolution"], "obtain resident_mcp capability via session handshake");
    }

    #[tokio::test]
    async fn test_guest_can_call_list_zones() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":20}"#, &guest())
            .await;
        let resp = parse_response(&raw);
        // Should succeed with no error
        assert!(resp["error"].is_null(), "guest should be able to call list_zones");
    }

    #[tokio::test]
    async fn test_guest_can_call_list_scene() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_scene","params":null,"id":21}"#, &guest())
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "guest should be able to call list_scene");
    }

    #[tokio::test]
    async fn test_resident_can_call_resident_tools() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tab","params":{"name":"T"},"id":22}"#,
                &resident(),
            )
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "resident should be able to call create_tab");
    }

    // ── Per-call authentication (spec §8.4) ──────────────────────────────────

    #[tokio::test]
    async fn test_auth_accepted_via_bearer_token() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0))
            .with_config(McpConfig::with_psk("secret-key"));
        let ctx = CallerContext::with_bearer("secret-key");
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":30}"#, &ctx)
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "valid bearer token should authenticate");
    }

    #[tokio::test]
    async fn test_auth_accepted_via_params_field() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0))
            .with_config(McpConfig::with_psk("secret-key"));
        let req = json!({
            "jsonrpc": "2.0",
            "method": "list_zones",
            "params": {"_auth": "secret-key"},
            "id": 31
        });
        let raw = server.dispatch(&req.to_string(), &CallerContext::guest()).await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "valid _auth param should authenticate");
    }

    #[tokio::test]
    async fn test_auth_rejected_with_wrong_key() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0))
            .with_config(McpConfig::with_psk("secret-key"));
        let ctx = CallerContext::with_bearer("wrong-key");
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":32}"#, &ctx)
            .await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32004, "wrong key should be rejected");
    }

    #[tokio::test]
    async fn test_auth_rejected_with_no_key() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0))
            .with_config(McpConfig::with_psk("secret-key"));
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":33}"#, &CallerContext::guest())
            .await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32004, "missing key should be rejected");
    }

    #[tokio::test]
    async fn test_each_call_authenticated_independently() {
        // Spec §8.4: two consecutive calls, each must carry auth independently.
        let server = Arc::new(
            McpServer::new(SceneGraph::new(1920.0, 1080.0))
                .with_config(McpConfig::with_psk("k")),
        );
        let good = CallerContext::with_bearer("k");
        let bad = CallerContext::guest();

        let r1 = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":40}"#, &good)
            .await;
        let r2 = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":41}"#, &bad)
            .await;

        let resp1 = parse_response(&r1);
        let resp2 = parse_response(&r2);

        // First call: success (authenticated)
        assert!(resp1["error"].is_null(), "first call with valid key should succeed");
        // Second call: rejected (no key — per-call auth, no persistent session)
        assert_eq!(resp2["error"]["code"], -32004, "second call without key should fail");
    }

    #[tokio::test]
    async fn test_no_psk_config_skips_auth() {
        // When no PSK is configured, all calls are accepted.
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        // No auth in context
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":50}"#, &CallerContext::guest())
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "unauthenticated call should succeed when no PSK configured");
    }
}
