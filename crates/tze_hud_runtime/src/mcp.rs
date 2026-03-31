//! # mcp
//!
//! MCP HTTP server lifecycle integration for the windowed runtime.
//!
//! This module wires [`tze_hud_mcp::McpServer`] into the runtime's
//! [`NetworkRuntime`] (Tokio multi-thread pool) and connects it to the shared
//! scene state produced at startup.
//!
//! ## Architecture note — scene coherence
//!
//! The MCP server and gRPC session server share a single canonical
//! [`Arc<Mutex<SceneGraph>>`].  Mutations applied over gRPC are immediately
//! visible to MCP queries and vice versa.  The `Arc` originates in
//! `windowed.rs` where `shared_scene` is created, stored in
//! [`SharedState::scene`], and also passed directly to
//! [`start_mcp_http_server`].
//!
//! ## Shutdown
//!
//! The HTTP task listens on a [`ShutdownToken`] receiver.  When the token is
//! triggered (e.g. by `WindowEvent::CloseRequested`), the task exits its
//! accept loop and returns, allowing the [`NetworkRuntime`]'s Tokio runtime
//! to drain and drop cleanly.  Dropping the [`tokio::runtime::Runtime`] waits
//! for all spawned tasks to complete before returning.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tze_hud_mcp::{CallerContext, McpConfig, McpServer};
use tze_hud_scene::graph::SceneGraph;

use crate::threads::ShutdownToken;

// ─── MCP lifecycle ────────────────────────────────────────────────────────────

/// Configuration for the runtime MCP HTTP server.
///
/// Analogous to [`crate::windowed::WindowedConfig`] but scoped to MCP.
/// Created internally by the windowed runtime from `WindowedConfig` fields.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Address and port to bind the MCP HTTP listener.
    ///
    /// Conventionally `0.0.0.0:<port>` or `127.0.0.1:<port>`.
    pub bind_addr: SocketAddr,

    /// Pre-shared key for MCP authentication.
    ///
    /// Every MCP request must supply a matching key via HTTP `Authorization:
    /// Bearer <key>` or the JSON-RPC `_auth` param.  When empty, a client
    /// sending an empty bearer token will authenticate — use a non-empty key
    /// in production.  To reject all calls unconditionally, do not start the
    /// MCP server (set `mcp_port = 0` in `WindowedConfig`).
    pub psk: String,
}

/// Start the MCP HTTP server task on the calling Tokio runtime.
///
/// Binds the listener, logs the effective address, and spawns the serve loop.
/// Returns the join handle so the caller can await it during shutdown.
///
/// # Parameters
///
/// * `scene`    — shared scene graph for MCP tool dispatch.
/// * `config`   — MCP server configuration (bind address, PSK).
/// * `shutdown` — token that stops the accept loop when triggered.
///
/// # Errors
///
/// Returns an error if `TcpListener::bind` fails (e.g., address in use).
pub async fn start_mcp_http_server(
    scene: Arc<Mutex<SceneGraph>>,
    config: McpServerConfig,
    shutdown: ShutdownToken,
) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let listener = TcpListener::bind(config.bind_addr).await?;
    let local_addr = listener.local_addr()?;

    tracing::info!(
        addr = %local_addr,
        "MCP HTTP listener bound"
    );

    let server =
        Arc::new(McpServer::with_shared_scene(scene).with_config(McpConfig::with_psk(&config.psk)));

    let handle = tokio::spawn(async move {
        run_accept_loop(listener, server, shutdown, local_addr).await;
    });

    Ok(handle)
}

/// Internal accept loop — runs until the shutdown token is triggered or the
/// listener returns an unrecoverable error.
async fn run_accept_loop(
    listener: TcpListener,
    server: Arc<McpServer>,
    shutdown: ShutdownToken,
    local_addr: SocketAddr,
) {
    let mut shutdown_rx = shutdown.subscribe();

    tracing::info!(addr = %local_addr, "MCP HTTP accept loop started");

    loop {
        tokio::select! {
            // Shutdown signal received — stop accepting new connections.
            _ = shutdown_rx.recv() => {
                tracing::info!(addr = %local_addr, "MCP HTTP server: shutdown signal received, stopping accept loop");
                break;
            }

            // Also check the atomic flag for the case where the broadcast was
            // sent before we subscribed (racing startup).
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)),
                if shutdown.is_triggered() => {
                tracing::info!(addr = %local_addr, "MCP HTTP server: shutdown flag detected, stopping accept loop");
                break;
            }

            // Accept a new connection.
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, peer)) => {
                        let srv = Arc::clone(&server);
                        tokio::spawn(handle_connection(stream, peer, srv));
                    }
                    Err(e) => {
                        tracing::error!(
                            addr = %local_addr,
                            error = %e,
                            "MCP HTTP: accept error — stopping server"
                        );
                        break;
                    }
                }
            }
        }
    }

    tracing::info!(addr = %local_addr, "MCP HTTP accept loop exited");
}

/// Handle a single HTTP connection: read request, dispatch, write response.
///
/// This is a minimal HTTP/1.0 handler — one request per connection, no
/// keep-alive, no TLS.  For production-grade serving, replace with an axum
/// or hyper-based integration.
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    peer: SocketAddr,
    server: Arc<McpServer>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Read the full HTTP request.  A single read() may return a partial TCP
    // segment (headers without body, or truncated body), so we loop until we
    // find the header/body boundary and have received Content-Length bytes of
    // body.  Cap total read at 64 KiB to bound memory.
    const MAX_REQUEST: usize = 65536;
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];

    // Phase 1: read until we have the full headers (terminated by \r\n\r\n).
    let header_end;
    loop {
        let n = match stream.read(&mut tmp).await {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) => {
                tracing::debug!(peer = %peer, error = %e, "MCP: read error");
                return;
            }
        };
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_REQUEST {
            tracing::debug!(peer = %peer, "MCP: request too large, dropping");
            return;
        }
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = pos;
            break;
        }
    }

    let body_start = header_end + 4;

    // Parse Content-Length and extract bearer token from headers before we
    // mutate `buf` further (satisfies the borrow checker).
    let (content_length, bearer_token) = {
        let header_section = std::str::from_utf8(&buf[..header_end]).unwrap_or("");

        let cl: usize = header_section
            .lines()
            .find(|l| l.to_lowercase().starts_with("content-length:"))
            .and_then(|l| l.split_once(':').map(|x| x.1))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);

        let bt = header_section
            .lines()
            .find(|l| l.to_lowercase().starts_with("authorization:"))
            .and_then(|l| l.split_once(':').map(|x| x.1))
            .map(|v| v.trim().to_owned())
            .and_then(|v| {
                let mut parts = v.splitn(2, ' ');
                match (parts.next(), parts.next()) {
                    (Some(scheme), Some(credentials)) if scheme.eq_ignore_ascii_case("bearer") => {
                        Some(credentials.trim().to_owned())
                    }
                    _ => None,
                }
            });

        (cl, bt)
    };

    // Phase 2: read remaining body bytes if we don't have them yet.
    let body_end = body_start + content_length;
    while buf.len() < body_end {
        if buf.len() > MAX_REQUEST {
            tracing::debug!(peer = %peer, "MCP: request too large, dropping");
            return;
        }
        let n = match stream.read(&mut tmp).await {
            Ok(0) => break, // EOF — use what we have
            Ok(n) => n,
            Err(e) => {
                tracing::debug!(peer = %peer, error = %e, "MCP: read error (body)");
                return;
            }
        };
        buf.extend_from_slice(&tmp[..n]);
    }

    let body = std::str::from_utf8(&buf[body_start..buf.len().min(body_end)]).unwrap_or("");

    let ctx = if let Some(token) = bearer_token {
        CallerContext::with_bearer(token)
    } else {
        CallerContext::guest()
    };

    let response_body = server.dispatch(body, &ctx).await;

    let http_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body,
    );

    if let Err(e) = stream.write_all(http_response.as_bytes()).await {
        tracing::debug!(peer = %peer, error = %e, "MCP: write error");
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tze_hud_scene::graph::SceneGraph;

    fn make_scene() -> Arc<Mutex<SceneGraph>> {
        Arc::new(Mutex::new(SceneGraph::new(1920.0, 1080.0)))
    }

    fn make_config(port: u16, psk: &str) -> McpServerConfig {
        McpServerConfig {
            bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
            psk: psk.to_string(),
        }
    }

    /// Helper: send an HTTP POST to the given addr with a JSON body.
    async fn http_post(addr: SocketAddr, body: &str, bearer: Option<&str>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let mut conn = TcpStream::connect(addr).await.expect("connect");

        let auth_header = bearer
            .map(|t| format!("Authorization: Bearer {t}\r\n"))
            .unwrap_or_default();

        let request = format!(
            "POST / HTTP/1.0\r\nContent-Type: application/json\r\n{auth_header}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );

        conn.write_all(request.as_bytes()).await.expect("write");

        let mut resp = Vec::new();
        conn.read_to_end(&mut resp).await.expect("read");
        String::from_utf8_lossy(&resp).into_owned()
    }

    #[tokio::test]
    async fn mcp_server_binds_and_responds() {
        let scene = make_scene();
        let config = make_config(0, "test-psk");
        let shutdown = ShutdownToken::new();

        let handle = start_mcp_http_server(scene, config, shutdown.clone())
            .await
            .expect("bind should succeed");

        // Give the task a moment to start its accept loop.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Signal shutdown.
        shutdown.trigger(crate::threads::ShutdownReason::Clean);
        handle.await.expect("task panicked");
    }

    #[tokio::test]
    async fn mcp_http_list_zones_authenticated() {
        use std::net::TcpListener as StdListener;

        // Bind to find a free port, then drop to release it for the server.
        let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr: SocketAddr = std_listener.local_addr().unwrap();
        drop(std_listener);

        let scene = make_scene();
        let config = McpServerConfig {
            bind_addr: addr,
            psk: "test-key".to_string(),
        };
        let shutdown = ShutdownToken::new();

        let handle = start_mcp_http_server(scene, config, shutdown.clone())
            .await
            .expect("bind");

        // Give the task time to enter accept loop.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let body = r#"{"jsonrpc":"2.0","method":"list_zones","params":{},"id":1}"#;
        let resp = http_post(addr, body, Some("test-key")).await;

        // Response should be HTTP 200 with a JSON-RPC result.
        assert!(
            resp.contains("HTTP/1.1 200"),
            "expected HTTP 200, got: {resp}"
        );
        assert!(
            resp.contains("\"result\""),
            "expected result field, got: {resp}"
        );

        shutdown.trigger(crate::threads::ShutdownReason::Clean);
        handle.await.expect("task");
    }

    #[tokio::test]
    async fn mcp_http_unauthenticated_returns_error() {
        use std::net::TcpListener as StdListener;

        let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr: SocketAddr = std_listener.local_addr().unwrap();
        drop(std_listener);

        let scene = make_scene();
        let config = McpServerConfig {
            bind_addr: addr,
            psk: "real-key".to_string(),
        };
        let shutdown = ShutdownToken::new();

        let handle = start_mcp_http_server(scene, config, shutdown.clone())
            .await
            .expect("bind");

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Send request with NO bearer token — should get an error response.
        let body = r#"{"jsonrpc":"2.0","method":"list_zones","params":{},"id":2}"#;
        let resp = http_post(addr, body, None).await;

        // HTTP status is always 200 (JSON-RPC over HTTP carries errors in body).
        assert!(resp.contains("HTTP/1.1 200"), "expected HTTP 200");
        // JSON-RPC body must contain "error".
        assert!(resp.contains("\"error\""), "expected error, got: {resp}");

        shutdown.trigger(crate::threads::ShutdownReason::Clean);
        handle.await.expect("task");
    }

    #[tokio::test]
    async fn mcp_http_wrong_psk_returns_error() {
        use std::net::TcpListener as StdListener;

        let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr: SocketAddr = std_listener.local_addr().unwrap();
        drop(std_listener);

        let scene = make_scene();
        let config = McpServerConfig {
            bind_addr: addr,
            psk: "correct-key".to_string(),
        };
        let shutdown = ShutdownToken::new();

        let handle = start_mcp_http_server(scene, config, shutdown.clone())
            .await
            .expect("bind");

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Send with wrong bearer token.
        let body = r#"{"jsonrpc":"2.0","method":"list_zones","params":{},"id":3}"#;
        let resp = http_post(addr, body, Some("wrong-key")).await;

        assert!(resp.contains("HTTP/1.1 200"));
        assert!(
            resp.contains("\"error\""),
            "expected auth error, got: {resp}"
        );

        shutdown.trigger(crate::threads::ShutdownReason::Clean);
        handle.await.expect("task");
    }

    #[tokio::test]
    async fn mcp_http_publish_to_zone_authenticated() {
        use std::net::TcpListener as StdListener;
        use tze_hud_scene::SceneId;
        use tze_hud_scene::types::{
            ContentionPolicy, GeometryPolicy, LayerAttachment, RenderingPolicy, ZoneDefinition,
            ZoneMediaType,
        };

        let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr: SocketAddr = std_listener.local_addr().unwrap();
        drop(std_listener);

        // Seed the scene with a zone so publish_to_zone has somewhere to write.
        let scene = make_scene();
        {
            let mut s = scene.lock().await;
            s.zone_registry.zones.insert(
                "test-zone".to_string(),
                ZoneDefinition {
                    id: SceneId::new(),
                    name: "test-zone".to_string(),
                    description: "Test Zone".to_string(),
                    geometry_policy: GeometryPolicy::Relative {
                        x_pct: 0.0,
                        y_pct: 0.0,
                        width_pct: 1.0,
                        height_pct: 0.05,
                    },
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

        let config = McpServerConfig {
            bind_addr: addr,
            psk: "test-key".to_string(),
        };
        let shutdown = ShutdownToken::new();

        let handle = start_mcp_http_server(scene, config, shutdown.clone())
            .await
            .expect("bind");

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let body = r#"{"jsonrpc":"2.0","method":"publish_to_zone","params":{"zone_name":"test-zone","content":"hello from MCP"},"id":4}"#;
        let resp = http_post(addr, body, Some("test-key")).await;

        assert!(resp.contains("HTTP/1.1 200"));
        // Either success or a known publish error — not an auth error.
        assert!(
            !resp.contains("Unauthenticated"),
            "should not get auth error, got: {resp}"
        );

        shutdown.trigger(crate::threads::ShutdownReason::Clean);
        handle.await.expect("task");
    }

    #[tokio::test]
    async fn mcp_server_shuts_down_cleanly() {
        use std::net::TcpListener as StdListener;

        let std_listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr: SocketAddr = std_listener.local_addr().unwrap();
        drop(std_listener);

        let scene = make_scene();
        let config = McpServerConfig {
            bind_addr: addr,
            psk: "key".to_string(),
        };
        let shutdown = ShutdownToken::new();

        let handle = start_mcp_http_server(scene, config, shutdown.clone())
            .await
            .expect("bind");

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Trigger shutdown and verify the task exits within a reasonable time.
        shutdown.trigger(crate::threads::ShutdownReason::Clean);
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(
            result.is_ok(),
            "MCP server task did not exit within 2s after shutdown"
        );
    }
}
