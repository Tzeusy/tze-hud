//! # Dashboard Tile Agent — Exemplar gRPC Agent Reference
//!
//! Proves the raw tile API composes correctly by creating a polished,
//! interactive dashboard tile via the gRPC session stream.
//!
//! This exemplar uses the **raw tile API exclusively** — no zones, no widgets,
//! no design tokens.  The agent directly manages tiles, nodes, leases, and
//! input events over a single bidirectional gRPC stream.
//!
//! ## Phases
//!
//! **Phase 1 — Session Establishment (tasks.md §1.1–1.2)** ← this binary
//!   - gRPC client setup connecting to HudSession bidirectional stream
//!   - `SessionInit` → `SessionEstablished`: verify session_id, namespace
//!
//! Future phases (separate bead tasks):
//!   - Phase 2: Lease acquisition (tasks.md §2)
//!   - Phase 3: Resource upload (tasks.md §3)
//!   - Phase 4: Atomic tile creation batch (tasks.md §4)
//!   - Phase 5: Periodic content update (tasks.md §6)
//!   - Phase 6: Input callbacks — Refresh + Dismiss (tasks.md §8)
//!
//! ## Running
//!
//! Headless (production config — enforces capability governance):
//!   cargo run -p dashboard_tile_agent -- --headless
//!
//! Headless (dev mode — unrestricted caps, TEST/DEV ONLY):
//!   cargo run -p dashboard_tile_agent --features dev-mode -- --headless --dev
//!
//! ## Spec references
//!
//! - `session-protocol/spec.md` — SessionInit/SessionEstablished handshake
//! - `configuration/spec.md` §Capability Vocabulary — canonical capability names
//! - `openspec/changes/exemplar-dashboard-tile/tasks.md` §1 (1.1–1.2)
//! - `openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md`

#[allow(deprecated)]
use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;
use tze_hud_protocol::proto::session as session_proto;
use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;

/// Embedded production config — capability governance always active by default.
/// File lives at `config/production.toml` relative to this source file.
const PRODUCTION_CONFIG: &str = include_str!("../config/production.toml");

/// Agent identifier registered in `config/production.toml`.
const AGENT_ID: &str = "dashboard-tile-agent";

/// Human-readable label shown in runtime admin panels.
const AGENT_DISPLAY_NAME: &str = "Dashboard Tile Agent";

/// Pre-shared key — must match the runtime's configured PSK.
const AGENT_PSK: &str = "dashboard-tile-key";

/// gRPC port the exemplar runtime listens on.
const GRPC_PORT: u16 = 50052;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");
    let dev_mode = args.iter().any(|a| a == "--dev");

    if headless {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        rt.block_on(run_headless(dev_mode))
    } else {
        println!("Dashboard tile agent: pass --headless to run in headless mode.");
        println!("  cargo run -p dashboard_tile_agent -- --headless");
        Ok(())
    }
}

fn now_wall_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

/// Run the headless runtime and execute the dashboard tile exemplar phases.
///
/// # Production path (default, `dev_mode = false`)
///
/// Loads `config/production.toml` (embedded at compile time).  Only
/// `dashboard-tile-agent` may connect, with the declared capabilities.
/// No Cargo feature flags required at build time for this path.
///
/// # Dev mode (opt-in, TEST/DEV ONLY)
///
/// Bypasses config governance: all capabilities granted to any agent.
/// Requires `--features dev-mode` at build time and `--dev` at runtime.
/// **NEVER use dev mode in production deployments.**
async fn run_headless(dev_mode: bool) -> Result<(), Box<dyn std::error::Error>> {
    if dev_mode {
        println!(
            "=== Dashboard Tile Agent (DEV MODE — unrestricted caps, TEST/DEV ONLY) ===\n"
        );
    } else {
        println!("=== Dashboard Tile Agent (production config) ===\n");
    }

    // ─── Initialize runtime ────────────────────────────────────────────────
    let config_toml = if dev_mode {
        None // dev-mode: requires --features dev-mode
    } else {
        Some(PRODUCTION_CONFIG.to_string())
    };

    let config = HeadlessConfig {
        width: 1920,
        height: 1080,
        grpc_port: GRPC_PORT,
        psk: AGENT_PSK.to_string(),
        config_toml,
    };

    let runtime = HeadlessRuntime::new(config).await?;
    let _server = runtime.start_grpc_server().await?;
    println!(
        "Runtime initialized: 1920x1080, gRPC on [::1]:{GRPC_PORT}\n"
    );

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 1: Session Establishment (tasks.md §1.1–1.2)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Connects a gRPC client to the HudSession bidirectional stream.
    // Sends SessionInit with agent identity, capability request, and subscription
    // declarations.  Reads SessionEstablished and verifies the session_id and
    // namespace assignment are non-empty (spec §SessionEstablished).
    //
    // Spec references:
    //   session-protocol/spec.md §Requirement: Session Establishment
    //   configuration/spec.md §Capability Vocabulary (canonical cap names)
    // ─────────────────────────────────────────────────────────────────────────
    println!("=== Phase 1: Session Establishment ===\n");

    let session_state = establish_session().await?;

    println!("  Phase 1 PASSED: session established.");
    println!("    namespace  = {}", session_state.namespace);
    println!("    session_id = {} bytes (non-empty)", session_state.session_id.len());
    println!("    protocol   = v{}.{}",
        session_state.negotiated_protocol_version / 1000,
        session_state.negotiated_protocol_version % 1000,
    );

    println!("\n=== Exemplar Phase 1 complete ===");
    println!(
        "Next: implement Phase 2 (lease acquisition) in tasks.md §2 [hud-rqea]."
    );

    Ok(())
}

/// Result of a successful session establishment handshake.
///
/// Returned by [`establish_session`] after `SessionEstablished` is received and
/// validated.  Carries the state needed by subsequent phases (lease, mutations).
pub struct SessionState {
    /// Opaque session identifier assigned by the runtime (UUIDv7, 16 bytes).
    /// Non-empty per spec §SessionEstablished field 1.
    pub session_id: Vec<u8>,

    /// Agent's namespace in the scene graph (RFC 0001 §1.2).
    /// Scopes all scene objects the agent creates.  Non-empty per spec.
    pub namespace: String,

    /// Capabilities actually granted after intersecting the requested set
    /// with the agent's authorization policy.
    pub granted_capabilities: Vec<String>,

    /// Resume token for reconnecting within the grace period.
    pub resume_token: Vec<u8>,

    /// Heartbeat interval the runtime expects from this client (ms).
    pub heartbeat_interval_ms: u64,

    /// Negotiated protocol version: `major * 1000 + minor`.
    pub negotiated_protocol_version: u32,

    /// Active subscription categories confirmed by the runtime.
    pub active_subscriptions: Vec<String>,
}

/// Connect to the HudSession gRPC stream and perform the session handshake.
///
/// # Session handshake (tasks.md §1.1–1.2)
///
/// 1. Opens a bidirectional stream to `HudSession.Session`.
/// 2. Sends `SessionInit` with:
///    - `agent_id` = "dashboard-tile-agent"
///    - `requested_capabilities` = [create_tiles, modify_own_tiles,
///      access_input_events]
///    - `initial_subscriptions` = [LEASE_CHANGES]
///    - `min_protocol_version` / `max_protocol_version` = 1000–1001
/// 3. Reads `SessionEstablished` from the server.
/// 4. Verifies `session_id` is non-empty (spec §SessionEstablished field 1).
/// 5. Verifies `namespace` is non-empty (spec §SessionEstablished field 2).
/// 6. Skips the `SceneSnapshot` that immediately follows per spec.
/// 7. Returns [`SessionState`] with all negotiated parameters.
///
/// # Errors
///
/// Returns an error if the server sends `SessionError`, the stream closes
/// unexpectedly, or a required verification assertion fails.
///
/// # Spec references
///
/// - session-protocol/spec.md §Requirement: Session Establishment
/// - configuration/spec.md §Capability Vocabulary (lines 149-164)
/// - openspec/changes/exemplar-dashboard-tile/tasks.md §1.1, §1.2
pub async fn establish_session() -> Result<SessionState, Box<dyn std::error::Error>> {
    use tokio_stream::StreamExt as _;

    // ── 1. Connect gRPC client to HudSession ──────────────────────────────
    //
    // HudSessionClient wraps a single bidirectional `Session` RPC.
    // All session traffic — handshake, mutations, events, heartbeats,
    // lease management — flows over this one stream per agent.
    #[allow(deprecated)]
    let mut session_client =
        HudSessionClient::connect(format!("http://[::1]:{GRPC_PORT}")).await?;

    // Channel for client → server messages.  Buffer = 64 gives the agent
    // headroom during bursts (e.g., mutation batches) without unbounded growth.
    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    // Open the bidirectional stream.  `session()` returns a Response wrapping
    // the server message stream; `.into_inner()` unwraps to the raw stream.
    let mut response_stream = session_client.session(stream).await?.into_inner();

    // ── 2. Send SessionInit ────────────────────────────────────────────────
    //
    // SessionInit is the first message the client MUST send on a new connection.
    // The runtime closes the stream if it does not arrive within the handshake
    // timeout (default 5000 ms).
    //
    // Capability vocabulary (configuration/spec.md §Capability Vocabulary):
    //   - "create_tiles"         — create tiles in a leased area
    //   - "modify_own_tiles"     — mutate tiles owned by this agent
    //   - "access_input_events"  — receive pointer / keyboard events
    //
    // LEASE_CHANGES is a mandatory subscription category (always active).
    // Listing it in `initial_subscriptions` is spec-compliant and explicit
    // about the agent's intent (session-protocol/spec.md §Subscriptions).
    let now_us = now_wall_us();
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: AGENT_ID.to_string(),
                agent_display_name: AGENT_DISPLAY_NAME.to_string(),
                pre_shared_key: AGENT_PSK.to_string(),
                // Canonical v1 capability names — non-canonical names are
                // rejected with CONFIG_UNKNOWN_CAPABILITY.
                requested_capabilities: vec![
                    "create_tiles".to_string(),        // create tiles in leased area
                    "modify_own_tiles".to_string(),    // mutate tiles owned by this agent
                    "access_input_events".to_string(), // receive pointer/keyboard events
                ],
                // LEASE_CHANGES is mandatory; listing it explicitly is idiomatic.
                initial_subscriptions: vec![
                    "LEASE_CHANGES".to_string(),
                ],
                resume_token: Vec::new(),   // new session, no prior resume token
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000, // v1.0
                max_protocol_version: 1001, // v1.1
                auth_credential: None,
            },
        )),
    })
    .await?;

    // ── 3. Receive SessionEstablished ──────────────────────────────────────
    //
    // The runtime processes SessionInit and responds with either
    // SessionEstablished (success) or SessionError (auth failure, version
    // mismatch, duplicate agent_id, etc.).
    let msg = response_stream.next().await
        .ok_or("stream closed before SessionEstablished")??;

    let established = match msg.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(ref e)) => {
            // ── 4 & 5. Verify session_id and namespace (tasks.md §1.2) ────
            //
            // spec §SessionEstablished:
            //   field 1 (session_id): opaque UUIDv7, 16 bytes — MUST be non-empty
            //   field 2 (namespace):  agent's scene namespace   — MUST be non-empty
            assert!(
                !e.session_id.is_empty(),
                "session_id MUST be non-empty (spec §SessionEstablished field 1)"
            );
            assert!(
                !e.namespace.is_empty(),
                "namespace MUST be non-empty (spec §SessionEstablished field 2)"
            );

            println!("  session_id           = {} bytes (UUIDv7)", e.session_id.len());
            println!("  namespace            = {}", e.namespace);
            println!("  heartbeat_ms         = {}", e.heartbeat_interval_ms);
            println!("  granted_capabilities = {:?}", e.granted_capabilities);
            println!("  active_subscriptions = {:?}", e.active_subscriptions);
            println!("  clock_skew           = {}us", e.estimated_skew_us);
            println!(
                "  protocol_version     = v{}.{}",
                e.negotiated_protocol_version / 1000,
                e.negotiated_protocol_version % 1000,
            );

            e.clone()
        }
        Some(session_proto::server_message::Payload::SessionError(ref err)) => {
            return Err(format!(
                "Session rejected by runtime: code={}, message={}, hint={}",
                err.code, err.message, err.hint
            )
            .into());
        }
        other => {
            return Err(format!(
                "Expected SessionEstablished as first server message, got: {other:?}"
            )
            .into());
        }
    };

    // ── 6. Skip SceneSnapshot ──────────────────────────────────────────────
    //
    // Per spec §Session Establishment: "Followed immediately by a SceneSnapshot."
    // The exemplar does not consume the snapshot at this phase; drain it so the
    // stream is ready for subsequent phase messages.
    let snapshot_msg = response_stream.next().await
        .ok_or("stream closed before SceneSnapshot")??;
    match snapshot_msg.payload {
        Some(session_proto::server_message::Payload::SceneSnapshot(ref snap)) => {
            println!(
                "  SceneSnapshot received: seq={}, json_len={}",
                snap.sequence,
                snap.snapshot_json.len()
            );
        }
        other => {
            return Err(format!(
                "Expected SceneSnapshot after SessionEstablished, got: {other:?}"
            )
            .into());
        }
    }

    // ── 7. Return SessionState ─────────────────────────────────────────────
    Ok(SessionState {
        session_id: established.session_id.clone(),
        namespace: established.namespace.clone(),
        granted_capabilities: established.granted_capabilities.clone(),
        resume_token: established.resume_token.clone(),
        heartbeat_interval_ms: established.heartbeat_interval_ms,
        negotiated_protocol_version: established.negotiated_protocol_version,
        active_subscriptions: established.active_subscriptions.clone(),
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Integration tests for Phase 1 (tasks.md §1.1–1.2).
    //!
    //! - [`test_session_establishment_returns_nonempty_session_id`]
    //!   Verifies that `establish_session` produces a non-empty session_id.
    //!
    //! - [`test_session_establishment_returns_nonempty_namespace`]
    //!   Verifies that `establish_session` produces a non-empty namespace.
    //!
    //! Both tests spin up a `HeadlessRuntime` with `dev-mode` (unrestricted
    //! capabilities; no registered-agent config required) and connect a real
    //! gRPC client, exercising the full handshake path in headless CI.

    use tze_hud_runtime::HeadlessRuntime;
    use tze_hud_runtime::headless::HeadlessConfig;

    const TEST_PSK: &str = "dashboard-tile-test-key";

    async fn start_test_runtime(port: u16) -> Result<
        tokio::task::JoinHandle<()>,
        Box<dyn std::error::Error>,
    > {
        let config = HeadlessConfig {
            width: 800,
            height: 600,
            grpc_port: port,
            psk: TEST_PSK.to_string(),
            config_toml: None, // dev-mode: unrestricted capabilities
        };
        let runtime = HeadlessRuntime::new(config).await?;
        let server = runtime.start_grpc_server().await?;
        Ok(server)
    }

    /// Task 1.2 — verify session_id is non-empty after successful handshake.
    ///
    /// Spec §SessionEstablished field 1: "Opaque session identifier (UUIDv7),
    /// MUST be non-empty."
    #[tokio::test]
    async fn test_session_establishment_returns_nonempty_session_id() {
        use tokio_stream::StreamExt as _;
        use crate::session_proto;
        #[allow(deprecated)]
        use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;

        let port = 50053u16;
        let _server = start_test_runtime(port).await.expect("runtime start");

        // Allow the server a moment to bind before the client connects.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        #[allow(deprecated)]
        let mut client = HudSessionClient::connect(format!("http://[::1]:{port}"))
            .await
            .expect("gRPC connect");

        let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(16);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut resp = client.session(stream).await.expect("session RPC").into_inner();

        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        tx.send(session_proto::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_us,
            payload: Some(session_proto::client_message::Payload::SessionInit(
                session_proto::SessionInit {
                    agent_id: "test-dashboard-agent".to_string(),
                    agent_display_name: "Test Dashboard Agent".to_string(),
                    pre_shared_key: TEST_PSK.to_string(),
                    requested_capabilities: vec![
                        "create_tiles".to_string(),
                        "modify_own_tiles".to_string(),
                        "access_input_events".to_string(),
                    ],
                    initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                    resume_token: Vec::new(),
                    agent_timestamp_wall_us: now_us,
                    min_protocol_version: 1000,
                    max_protocol_version: 1001,
                    auth_credential: None,
                },
            )),
        })
        .await
        .expect("send SessionInit");

        // First message must be SessionEstablished.
        let msg = resp.next().await.expect("server message").expect("no error");
        match msg.payload {
            Some(session_proto::server_message::Payload::SessionEstablished(e)) => {
                assert!(
                    !e.session_id.is_empty(),
                    "session_id must be non-empty (tasks.md §1.2)"
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Task 1.2 — verify namespace is non-empty after successful handshake.
    ///
    /// Spec §SessionEstablished field 2: "Agent's namespace in the scene
    /// (RFC 0001 §1.2). MUST be non-empty."
    #[tokio::test]
    async fn test_session_establishment_returns_nonempty_namespace() {
        use tokio_stream::StreamExt as _;
        use crate::session_proto;
        #[allow(deprecated)]
        use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;

        let port = 50054u16;
        let _server = start_test_runtime(port).await.expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        #[allow(deprecated)]
        let mut client = HudSessionClient::connect(format!("http://[::1]:{port}"))
            .await
            .expect("gRPC connect");

        let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(16);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut resp = client.session(stream).await.expect("session RPC").into_inner();

        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        tx.send(session_proto::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_us,
            payload: Some(session_proto::client_message::Payload::SessionInit(
                session_proto::SessionInit {
                    agent_id: "test-namespace-agent".to_string(),
                    agent_display_name: "Test Namespace Agent".to_string(),
                    pre_shared_key: TEST_PSK.to_string(),
                    requested_capabilities: vec![
                        "create_tiles".to_string(),
                        "modify_own_tiles".to_string(),
                        "access_input_events".to_string(),
                    ],
                    initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                    resume_token: Vec::new(),
                    agent_timestamp_wall_us: now_us,
                    min_protocol_version: 1000,
                    max_protocol_version: 1001,
                    auth_credential: None,
                },
            )),
        })
        .await
        .expect("send SessionInit");

        let msg = resp.next().await.expect("server message").expect("no error");
        match msg.payload {
            Some(session_proto::server_message::Payload::SessionEstablished(e)) => {
                assert!(
                    !e.namespace.is_empty(),
                    "namespace must be non-empty (tasks.md §1.2)"
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }
}
