//! MCP JSON-RPC server.
//!
//! The server wraps the shared [`SceneGraph`] and dispatches incoming
//! JSON-RPC 2.0 requests to the appropriate tool handler.
//!
//! ## Transport
//!
//! The server is transport-agnostic at this layer: [`McpServer::dispatch`]
//! accepts a raw `&str` (the JSON-RPC request body) and returns a `String`
//! (the JSON-RPC response body). Callers can wire this to HTTP, stdio, or any
//! other transport.
//!
//! A reference HTTP server using Tokio is provided via [`McpServer::run_http`]
//! for integration testing and development use.

use std::sync::Arc;
use tokio::sync::Mutex;
use tze_hud_scene::graph::SceneGraph;
use tracing::{debug, error, warn};

use crate::{
    error::JsonRpcError,
    tools,
    types::{McpRequest, McpResponse},
};

/// Shared server state — wraps the scene graph behind an async mutex so that
/// concurrent requests serialize scene mutations safely.
pub struct McpServer {
    scene: Arc<Mutex<SceneGraph>>,
}

impl McpServer {
    /// Create a new server backed by the given scene graph.
    pub fn new(scene: SceneGraph) -> Self {
        Self {
            scene: Arc::new(Mutex::new(scene)),
        }
    }

    /// Create a new server sharing an existing arc-wrapped scene graph.
    ///
    /// Use this when the scene is also shared with the gRPC control plane.
    pub fn with_shared_scene(scene: Arc<Mutex<SceneGraph>>) -> Self {
        Self { scene }
    }

    /// Borrow the underlying shared scene handle.
    pub fn scene_handle(&self) -> Arc<Mutex<SceneGraph>> {
        Arc::clone(&self.scene)
    }

    /// Dispatch a raw JSON-RPC 2.0 request body and return the response body.
    ///
    /// This is the single entry point for all transports.
    pub async fn dispatch(&self, body: &str) -> String {
        // Parse the request
        let request: McpRequest = match serde_json::from_str(body) {
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

        debug!(method = %request.method, "MCP: dispatching tool call");

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
    /// on a single TCP stream, no TLS, no auth, no keep-alive.
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

                // Extract the body (everything after the double CRLF in HTTP)
                let raw = &buf[..n];
                let body = if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    std::str::from_utf8(&raw[pos + 4..]).unwrap_or("")
                } else {
                    std::str::from_utf8(raw).unwrap_or("")
                };

                let response_body = server.dispatch(body).await;

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
    use tze_hud_scene::{graph::SceneGraph, types::{ZoneDefinition, GeometryPolicy, ZoneMediaType, RenderingPolicy, ContentionPolicy}, SceneId};

    async fn server_with_tab() -> (McpServer, SceneId) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).expect("create tab");
        let server = McpServer::new(scene);
        (server, tab_id)
    }

    fn parse_response(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).expect("valid JSON response")
    }

    // ── JSON-RPC protocol compliance ─────────────────────────────────────────

    #[tokio::test]
    async fn test_malformed_json_returns_parse_error() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server.dispatch("{not valid json").await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn test_wrong_jsonrpc_version_returns_invalid_request() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(r#"{"jsonrpc":"1.0","method":"list_zones","id":1}"#)
            .await;
        let resp = parse_response(&raw);
        assert_eq!(resp["error"]["code"], -32600);
    }

    #[tokio::test]
    async fn test_unknown_method_returns_method_not_found() {
        let (server, _) = server_with_tab().await;
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"does_not_exist","id":1}"#)
            .await;
        let resp = parse_response(&raw);
        // JSON-RPC 2.0: unknown method must return -32601 Method not found
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn test_request_id_echoed_in_response() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":42}"#)
            .await;
        let resp = parse_response(&raw);
        assert_eq!(resp["id"], 42);
    }

    // ── create_tab ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_create_tab() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tab","params":{"name":"Alerts"},"id":1}"#,
            )
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null());
        assert!(!resp["result"]["tab_id"].as_str().unwrap().is_empty());
        assert_eq!(resp["result"]["name"], "Alerts");
    }

    // ── create_tile ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_create_tile() {
        let (server, _) = server_with_tab().await;
        let raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tile","params":{"namespace":"a","bounds":{"x":0,"y":0,"width":200,"height":200}},"id":2}"#,
            )
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
        assert!(!resp["result"]["tile_id"].as_str().unwrap().is_empty());
    }

    // ── set_content ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_set_content() {
        let (server, _) = server_with_tab().await;

        // First create a tile
        let tile_raw = server
            .dispatch(
                r#"{"jsonrpc":"2.0","method":"create_tile","params":{"namespace":"a","bounds":{"x":0,"y":0,"width":200,"height":200}},"id":1}"#,
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
        let raw = server.dispatch(&content_req.to_string()).await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
        assert_eq!(resp["result"]["content_len"], 11);
    }

    // ── publish_to_zone ─────────────────────────────────────────────────────

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
                },
            );
        }

        let req = json!({
            "jsonrpc": "2.0",
            "method": "publish_to_zone",
            "params": {"zone": "status", "content": "All systems go"},
            "id": 3
        });
        let raw = server.dispatch(&req.to_string()).await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
        assert_eq!(resp["result"]["zone"], "status");
    }

    // ── list_zones ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dispatch_list_zones_empty() {
        let server = McpServer::new(SceneGraph::new(1920.0, 1080.0));
        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":null,"id":4}"#)
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
                },
            );
        }

        let raw = server
            .dispatch(r#"{"jsonrpc":"2.0","method":"list_zones","params":{},"id":5}"#)
            .await;
        let resp = parse_response(&raw);
        assert!(resp["error"].is_null());
        assert_eq!(resp["result"]["count"], 1);
        assert_eq!(resp["result"]["zones"][0]["name"], "hud");
    }
}
