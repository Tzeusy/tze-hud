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

use tze_hud_protocol::proto::session as session_proto;
#[allow(deprecated)]
use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;
use tze_hud_resource::{
    AgentBudget, CAPABILITY_UPLOAD_RESOURCE, ResourceStore, ResourceStoreConfig, ResourceType,
    UploadId, UploadStartRequest,
};
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
        .map(|d| d.as_micros() as u64)
        .unwrap_or(1) // clock before UNIX epoch: return 1 (non-zero per timing-model spec)
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
        println!("=== Dashboard Tile Agent (DEV MODE — unrestricted caps, TEST/DEV ONLY) ===\n");
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
    println!("Runtime initialized: 1920x1080, gRPC on [::1]:{GRPC_PORT}\n");

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
    println!(
        "    session_id = {} bytes (non-empty)",
        session_state.session_id.len()
    );
    println!(
        "    protocol   = v{}.{}",
        session_state.negotiated_protocol_version / 1000,
        session_state.negotiated_protocol_version % 1000,
    );

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 2: Lease Acquisition (tasks.md §2.1–2.2)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Sends LeaseRequest with:
    //   - ttl_ms = 60000 (60-second TTL per spec §Lease Request With AutoRenew)
    //   - capabilities = [create_tiles, modify_own_tiles] (spec-mandated scope)
    //   - lease_priority = 2 (default agent-owned band)
    //
    // Reads LeaseResponse and verifies granted = true and a 16-byte UUIDv7 lease_id.
    // Stores the lease_id for use in tile creation batches (Phase 4+).
    //
    // Spec references:
    //   lease-governance/spec.md §Requirement: Lease Request With AutoRenew
    //   openspec/changes/exemplar-dashboard-tile/tasks.md §2.1–2.2
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n=== Phase 2: Lease Acquisition ===\n");

    let lease_id = request_lease(GRPC_PORT, AGENT_PSK, AGENT_ID, AGENT_DISPLAY_NAME).await?;

    println!("  Phase 2 PASSED: lease granted.");
    println!("    lease_id   = {} bytes (UUIDv7)", lease_id.len());

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 3: Resource Upload (tasks.md §3.1)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Uploads a 48×48 PNG icon via the ResourceStore inline fast path.
    // Captures the returned BLAKE3 ResourceId (32 bytes) for use in StaticImageNode.
    //
    // The ResourceId is content-addressed: BLAKE3(raw_bytes) → unique 32-byte digest.
    // Any two uploads of identical bytes return the same ResourceId (deduplication).
    //
    // Spec references:
    //   resource-store/spec.md §Requirement: Resource Upload Before Tile Creation
    //   openspec/changes/exemplar-dashboard-tile/tasks.md §3.1
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n=== Phase 3: Resource Upload ===\n");

    let resource_id = upload_icon(AGENT_ID).await?;

    println!("  Phase 3 PASSED: icon uploaded.");
    println!("    resource_id = {} bytes (BLAKE3)", resource_id.len());

    println!("\n=== Exemplar Phases 1–3 complete ===");
    println!("  lease_id    = {} bytes", lease_id.len());
    println!("  resource_id = {} bytes", resource_id.len());
    println!("Next: implement Phase 4 (atomic tile creation batch) in tasks.md §4 [hud-xerv].");

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
    establish_session_with(GRPC_PORT, AGENT_PSK, AGENT_ID, AGENT_DISPLAY_NAME).await
}

/// Parameterized session-establishment helper used by tests and the public API.
///
/// Accepts connection parameters so tests can spin up isolated runtimes on
/// ephemeral ports without conflicting with production constants.
async fn establish_session_with(
    port: u16,
    psk: &str,
    agent_id: &str,
    agent_display_name: &str,
) -> Result<SessionState, Box<dyn std::error::Error>> {
    use tokio_stream::StreamExt as _;

    // ── 1. Connect gRPC client to HudSession ──────────────────────────────
    //
    // HudSessionClient wraps a single bidirectional `Session` RPC.
    // All session traffic — handshake, mutations, events, heartbeats,
    // lease management — flows over this one stream per agent.
    #[allow(deprecated)]
    let mut session_client = HudSessionClient::connect(format!("http://[::1]:{port}")).await?;

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
                agent_id: agent_id.to_string(),
                agent_display_name: agent_display_name.to_string(),
                pre_shared_key: psk.to_string(),
                // Canonical v1 capability names — non-canonical names are
                // rejected with CONFIG_UNKNOWN_CAPABILITY.
                requested_capabilities: vec![
                    "create_tiles".to_string(),        // create tiles in leased area
                    "modify_own_tiles".to_string(),    // mutate tiles owned by this agent
                    "access_input_events".to_string(), // receive pointer/keyboard events
                ],
                // LEASE_CHANGES is mandatory; listing it explicitly is idiomatic.
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(), // new session, no prior resume token
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
    let msg = response_stream
        .next()
        .await
        .ok_or("stream closed before SessionEstablished")??;

    let established = match msg.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(ref e)) => {
            // ── 4 & 5. Verify session_id and namespace (tasks.md §1.2) ────
            //
            // spec §SessionEstablished:
            //   field 1 (session_id): opaque UUIDv7, 16 bytes — MUST be non-empty
            //   field 2 (namespace):  agent's scene namespace   — MUST be non-empty
            if e.session_id.is_empty() {
                return Err(
                    "session_id MUST be non-empty (spec §SessionEstablished field 1)".into(),
                );
            }
            if e.namespace.is_empty() {
                return Err(
                    "namespace MUST be non-empty (spec §SessionEstablished field 2)".into(),
                );
            }

            println!(
                "  session_id           = {} bytes (UUIDv7)",
                e.session_id.len()
            );
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
    let snapshot_msg = response_stream
        .next()
        .await
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
            return Err(
                format!("Expected SceneSnapshot after SessionEstablished, got: {other:?}").into(),
            );
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

// ─── Phase 2: Lease Acquisition ──────────────────────────────────────────────

/// Request a lease on a new gRPC session and return the granted `lease_id` bytes.
///
/// # Lease request (tasks.md §2.1–2.2)
///
/// 1. Opens a new gRPC session by duplicating the SessionInit handshake inline
///    (not via `establish_session_with`) so that each phase remains independently
///    testable without coupling to the Phase 1 session state.
/// 2. Sends `LeaseRequest` with:
///    - `ttl_ms = 60000` (spec §Lease Request With AutoRenew: 60-second TTL)
///    - `capabilities = ["create_tiles", "modify_own_tiles"]`
///    - `lease_priority = 2` (default agent-owned band)
/// 3. Reads the next non-state-change message and expects `LeaseResponse`.
/// 4. Verifies `granted = true` (spec §Scenario: Lease granted with requested parameters).
/// 5. Returns the 16-byte UUIDv7 `lease_id` for use in subsequent MutationBatch calls.
///
/// # Phase 4 integration note
///
/// This function opens a short-lived session and drops it after the lease is granted.
/// In the current standalone Phase 2 test this is sufficient to verify the lease protocol.
/// Phase 4 (tile creation batch, tasks.md §4) MUST reuse the established session stream
/// from Phase 1 rather than calling this function, so that the lease remains attached to
/// the live session used by MutationBatch calls and is not orphaned on session disconnect.
///
/// # Spec references
///
/// - lease-governance/spec.md §Requirement: Lease Request With AutoRenew
/// - openspec/changes/exemplar-dashboard-tile/tasks.md §2.1–2.2
pub async fn request_lease(
    port: u16,
    psk: &str,
    agent_id: &str,
    agent_display_name: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use tokio_stream::StreamExt as _;

    // ── 1. Establish a fresh gRPC session ──────────────────────────────────
    //
    // We open a new session rather than sharing one from Phase 1 so that
    // each phase is independently testable in unit tests without coupling.
    #[allow(deprecated)]
    let mut session_client = session_proto::hud_session_client::HudSessionClient::connect(format!(
        "http://[::1]:{port}"
    ))
    .await?;

    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let mut response_stream = session_client.session(stream).await?.into_inner();

    // Send SessionInit.
    let now_us = now_wall_us();
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_display_name.to_string(),
                pre_shared_key: psk.to_string(),
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
    .await?;

    // Drain SessionEstablished.
    let first = response_stream
        .next()
        .await
        .ok_or("stream closed before SessionEstablished")??;
    match first.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(_)) => {}
        other => {
            return Err(format!(
                "Expected SessionEstablished as first server message, got: {other:?}"
            )
            .into());
        }
    }

    // Drain SceneSnapshot.
    let second = response_stream
        .next()
        .await
        .ok_or("stream closed before SceneSnapshot")??;
    match second.payload {
        Some(session_proto::server_message::Payload::SceneSnapshot(_)) => {}
        other => {
            return Err(
                format!("Expected SceneSnapshot after SessionEstablished, got: {other:?}").into(),
            );
        }
    }

    // ── 2. Send LeaseRequest (tasks.md §2.1) ──────────────────────────────
    //
    // Spec §Requirement: Lease Request With AutoRenew:
    //   ttl_ms = 60000, capabilities = [create_tiles, modify_own_tiles], lease_priority = 2.
    //
    // Note: renewal policy (AutoRenew) and resource budgets are server-side concerns;
    // they are not fields on the LeaseRequest proto.
    tx.send(session_proto::ClientMessage {
        sequence: 2,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::LeaseRequest(
            session_proto::LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            },
        )),
    })
    .await?;

    // ── 3–4. Receive LeaseResponse and verify granted = true (tasks.md §2.2) ──
    //
    // Drain any interleaved LeaseStateChange messages (REQUESTED→ACTIVE)
    // before asserting the LeaseResponse, consistent with the test-suite pattern.
    loop {
        let msg = response_stream
            .next()
            .await
            .ok_or("stream closed before LeaseResponse")??;
        match msg.payload {
            Some(session_proto::server_message::Payload::LeaseStateChange(_)) => {
                // Drain — not the response we're waiting for.
                continue;
            }
            Some(session_proto::server_message::Payload::LeaseResponse(resp)) => {
                if !resp.granted {
                    return Err(format!(
                        "LeaseResponse denied: code={}, reason={}",
                        resp.deny_code, resp.deny_reason
                    )
                    .into());
                }
                if resp.lease_id.len() != 16 {
                    return Err(format!(
                        "LeaseResponse granted but lease_id is {} bytes (must be 16-byte UUIDv7)",
                        resp.lease_id.len()
                    )
                    .into());
                }
                println!(
                    "  LeaseResponse: granted=true, ttl={}ms",
                    resp.granted_ttl_ms
                );
                println!("    granted_capabilities = {:?}", resp.granted_capabilities);
                println!("    granted_priority     = {}", resp.granted_priority);
                // ── 5. Return lease_id ────────────────────────────────────
                return Ok(resp.lease_id);
            }
            other => {
                return Err(format!("Expected LeaseResponse, got: {other:?}").into());
            }
        }
    }
}

// ─── Phase 3: Resource Upload ─────────────────────────────────────────────────

/// Icon dimensions per spec §Dashboard Tile Composition node 2.
const ICON_W: u32 = 48;
const ICON_H: u32 = 48;

/// Upload the dashboard tile's 48×48 PNG icon and return the BLAKE3 `ResourceId` bytes.
///
/// # Resource upload (tasks.md §3.1)
///
/// Uses the `ResourceStore` inline fast path (≤ 64 KiB per RFC 0011 §3):
/// 1. Generates a 48×48 solid-color PNG in memory (no filesystem I/O).
/// 2. Computes the BLAKE3 content hash as the expected `ResourceId`.
/// 3. Calls `ResourceStore::handle_upload_start` with `inline_data`.
/// 4. Returns the 32-byte `ResourceId` bytes (the BLAKE3 digest of the raw PNG).
///
/// The `ResourceId` is content-addressed: identical bytes always yield the same id
/// (deduplication contract, RFC 0011 §4).  The agent MUST pass this id in the
/// `StaticImageNode.resource_id` field of the tile creation batch (Phase 4).
///
/// # Phase 4 integration note
///
/// This function uploads into a standalone in-memory `ResourceStore` to prove the
/// resource upload protocol and content-addressed identity (tasks.md §3.1).
/// Phase 4 (tile creation batch, tasks.md §4) MUST upload through the runtime-owned
/// path (e.g. the session `upload_resource` RPC) so that the resource becomes
/// registered in the runtime's `SceneGraph::registered_resources` set.  Without that
/// registration, `add_node_to_tile_checked` / `set_tile_root_checked` will reject the
/// `StaticImageNode` reference with `ResourceNotFound`.
///
/// # Spec references
///
/// - resource-store/spec.md §Requirement: Resource Upload Before Tile Creation
/// - resource-store/spec.md §Requirement: Content-Addressed Resource Identity
/// - openspec/changes/exemplar-dashboard-tile/tasks.md §3.1
pub async fn upload_icon(agent_namespace: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // ── 1. Generate a 48×48 solid-color PNG ───────────────────────────────
    //
    // Representative placeholder icon.  In production an agent would supply
    // its own brand asset here.  The test fixture uses a solid steel-blue fill.
    let png_bytes: Vec<u8> = {
        use image::{ImageBuffer, Rgb};
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(ICON_W, ICON_H, |_, _| Rgb([70u8, 130, 180]));
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| format!("PNG encoding failed: {e}"))?;
        buf
    };

    // ── 2. Compute expected BLAKE3 ResourceId ─────────────────────────────
    let resource_id = tze_hud_resource::types::ResourceId::from_content(&png_bytes);

    // ── 3. Upload via ResourceStore inline fast path ──────────────────────
    let store = ResourceStore::new(ResourceStoreConfig::default());
    let upload_id = UploadId::from_bytes(uuid::Uuid::now_v7().into_bytes());

    let result = store
        .handle_upload_start(UploadStartRequest {
            agent_namespace: agent_namespace.to_string(),
            // The `upload_resource` capability is required by the store.
            agent_capabilities: vec![CAPABILITY_UPLOAD_RESOURCE.to_string()],
            agent_budget: AgentBudget {
                texture_bytes_total_limit: 0, // 0 = unlimited
                texture_bytes_total_used: 0,
            },
            upload_id,
            resource_type: ResourceType::ImagePng,
            expected_hash: *resource_id.as_bytes(),
            total_size: png_bytes.len(),
            inline_data: png_bytes,
            width: ICON_W,
            height: ICON_H,
        })
        .await
        .map_err(|e| format!("ResourceStore upload_start failed: {e:?}"))?
        .ok_or("inline upload must return ResourceStored immediately")?;

    // ── 4. Return 32-byte BLAKE3 ResourceId ──────────────────────────────
    Ok(result.resource_id.as_bytes().to_vec())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Integration tests for Phases 1–3 (tasks.md §1.1–1.2, §2.1–2.3, §3.1).
    //!
    //! Phase 1 (§1.1–1.2):
    //! - [`test_session_establishment_returns_nonempty_session_id`]
    //! - [`test_session_establishment_returns_nonempty_namespace`]
    //!
    //! Phase 2 (§2.1–2.3):
    //! - [`test_lease_grant_returns_granted_true_and_16_byte_lease_id`]
    //!   Verifies `request_lease` returns a 16-byte UUIDv7 lease_id with granted=true.
    //! - [`test_lease_request_with_invalid_capability_is_denied`]
    //!   Verifies that requesting a non-canonical capability denies the lease.
    //!
    //! Phase 3 (§3.1):
    //! - [`test_upload_icon_returns_32_byte_blake3_resource_id`]
    //!   Verifies `upload_icon` returns a 32-byte BLAKE3 ResourceId.
    //!
    //! All tests spin up a `HeadlessRuntime` with `dev-mode` (unrestricted
    //! capabilities; no registered-agent config required) on an ephemeral port.
    //!
    //! Ephemeral ports prevent port-conflict flakiness in parallel CI.
    //! Each `server` JoinHandle is aborted after assertions complete.

    use tze_hud_runtime::HeadlessRuntime;
    use tze_hud_runtime::headless::HeadlessConfig;

    const TEST_PSK: &str = "dashboard-tile-test-key";
    const TEST_AGENT_ID: &str = "test-dashboard-agent";
    const TEST_AGENT_DISPLAY_NAME: &str = "Test Dashboard Agent";

    /// Bind an ephemeral port and return it.  The listener is dropped before
    /// the gRPC server starts; there is a brief TOCTOU window, but this is the
    /// same pattern used across the integration test suite.
    fn ephemeral_port() -> u16 {
        let listener = std::net::TcpListener::bind("[::1]:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("get local addr").port();
        drop(listener);
        port
    }

    async fn start_test_runtime(
        port: u16,
    ) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
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

    // ── Phase 2 helpers ───────────────────────────────────────────────────────

    /// Drain messages from `stream` until the first non-`LeaseStateChange` message.
    async fn next_non_state_change(
        stream: &mut tonic::Streaming<tze_hud_protocol::proto::session::ServerMessage>,
    ) -> tze_hud_protocol::proto::session::ServerMessage {
        use tokio_stream::StreamExt as _;
        loop {
            let msg = stream
                .next()
                .await
                .expect("stream closed before LeaseResponse")
                .expect("stream error");
            match &msg.payload {
                Some(
                    tze_hud_protocol::proto::session::server_message::Payload::LeaseStateChange(_),
                ) => continue,
                _ => return msg,
            }
        }
    }

    /// Task 1.2 — verify session_id is non-empty after successful handshake.
    ///
    /// Spec §SessionEstablished field 1: "Opaque session identifier (UUIDv7),
    /// MUST be non-empty."
    #[tokio::test]
    async fn test_session_establishment_returns_nonempty_session_id() {
        let port = ephemeral_port();
        let server = start_test_runtime(port).await.expect("runtime start");

        // Allow the server a moment to bind before the client connects.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let state =
            crate::establish_session_with(port, TEST_PSK, TEST_AGENT_ID, TEST_AGENT_DISPLAY_NAME)
                .await
                .expect("establish_session_with");

        assert!(
            !state.session_id.is_empty(),
            "session_id must be non-empty (tasks.md §1.2)"
        );

        server.abort();
    }

    /// Task 1.2 — verify namespace is non-empty after successful handshake.
    ///
    /// Spec §SessionEstablished field 2: "Agent's namespace in the scene
    /// (RFC 0001 §1.2). MUST be non-empty."
    #[tokio::test]
    async fn test_session_establishment_returns_nonempty_namespace() {
        let port = ephemeral_port();
        let server = start_test_runtime(port).await.expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let state =
            crate::establish_session_with(port, TEST_PSK, TEST_AGENT_ID, TEST_AGENT_DISPLAY_NAME)
                .await
                .expect("establish_session_with");

        assert!(
            !state.namespace.is_empty(),
            "namespace must be non-empty (tasks.md §1.2)"
        );

        server.abort();
    }

    // ── Phase 2: Lease Acquisition tests ─────────────────────────────────────

    /// Task 2.1–2.2 — `request_lease` returns granted=true and a 16-byte UUIDv7 lease_id.
    ///
    /// Spec §Requirement: Lease Request With AutoRenew — Scenario: Lease granted
    /// with requested parameters.
    /// tasks.md §2.1: send LeaseRequest { ttl_ms=60000, capabilities=[create_tiles,
    ///   modify_own_tiles], lease_priority=2 }.
    /// tasks.md §2.2: verify LeaseResponse.granted=true and store the 16-byte lease_id.
    #[tokio::test]
    async fn test_lease_grant_returns_granted_true_and_16_byte_lease_id() {
        let port = ephemeral_port();
        let server = start_test_runtime(port).await.expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let lease_id_bytes =
            crate::request_lease(port, TEST_PSK, TEST_AGENT_ID, TEST_AGENT_DISPLAY_NAME)
                .await
                .expect("request_lease");

        // tasks.md §2.2: lease_id MUST be exactly 16 bytes (UUIDv7 SceneId).
        assert_eq!(
            lease_id_bytes.len(),
            16,
            "lease_id must be 16 bytes (UUIDv7) — tasks.md §2.2"
        );

        server.abort();
    }

    /// Task 2.3 — LeaseRequest with a non-canonical capability is denied.
    ///
    /// Spec §Requirement: Lease Request With AutoRenew — Scenario: Tile creation
    /// requires active lease (only valid capabilities may be requested).
    /// tasks.md §2.3: add test — lease request without required capabilities is denied.
    ///
    /// "create_tile" (singular) is a legacy non-canonical name rejected since
    /// RFC 0005 Round 14. The server MUST respond with:
    ///   LeaseResponse { granted: false, deny_code: "CONFIG_UNKNOWN_CAPABILITY" }.
    #[tokio::test]
    async fn test_lease_request_with_invalid_capability_is_denied() {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;

        let port = ephemeral_port();
        let server = start_test_runtime(port).await.expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // ── 1. Open a session ─────────────────────────────────────────────────
        #[allow(deprecated)]
        let mut session_client =
            sp::hud_session_client::HudSessionClient::connect(format!("http://[::1]:{port}"))
                .await
                .expect("connect");

        let (tx, rx) = tokio::sync::mpsc::channel::<sp::ClientMessage>(64);
        let stream_req = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut resp_stream = session_client
            .session(stream_req)
            .await
            .expect("session rpc")
            .into_inner();

        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(1);

        // SessionInit with valid capabilities for the session.
        tx.send(sp::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_us,
            payload: Some(sp::client_message::Payload::SessionInit(sp::SessionInit {
                agent_id: "bad-cap-test-agent".to_string(),
                agent_display_name: "Bad Cap Test".to_string(),
                pre_shared_key: TEST_PSK.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        // Drain SessionEstablished + SceneSnapshot.
        for _ in 0..2 {
            resp_stream
                .next()
                .await
                .expect("stream not closed")
                .expect("no stream error");
        }

        // ── 2. Send a LeaseRequest with a non-canonical (legacy singular) capability ──
        //
        // "create_tile" (singular) was superseded by "create_tiles" (plural) in
        // RFC 0005 Round 14.  The server must reject this with CONFIG_UNKNOWN_CAPABILITY.
        tx.send(sp::ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_us,
            payload: Some(sp::client_message::Payload::LeaseRequest(
                sp::LeaseRequest {
                    ttl_ms: 60_000,
                    capabilities: vec!["create_tile".to_string()], // non-canonical singular form
                    lease_priority: 2,
                },
            )),
        })
        .await
        .unwrap();

        // ── 3. Assert denial ──────────────────────────────────────────────────
        let resp_msg = next_non_state_change(&mut resp_stream).await;
        match resp_msg.payload {
            Some(sp::server_message::Payload::LeaseResponse(resp)) => {
                assert!(
                    !resp.granted,
                    "LeaseResponse must NOT be granted for non-canonical capability — tasks.md §2.3"
                );
                assert_eq!(
                    resp.deny_code, "CONFIG_UNKNOWN_CAPABILITY",
                    "deny_code must be CONFIG_UNKNOWN_CAPABILITY for unknown capability, \
                     got: {:?}",
                    resp.deny_code
                );
                assert!(
                    !resp.deny_reason.is_empty(),
                    "deny_reason must be non-empty — tasks.md §2.3"
                );
            }
            other => panic!(
                "Expected LeaseResponse(denied) for non-canonical capability, got: {other:?}"
            ),
        }

        server.abort();
    }

    // ── Phase 3: Resource Upload tests ───────────────────────────────────────

    /// Task 3.1 — `upload_icon` returns a 32-byte BLAKE3 ResourceId.
    ///
    /// Spec §Requirement: Resource Upload Before Tile Creation — Content-Addressed
    /// Resource Identity: ResourceId = BLAKE3(raw_bytes), 32 bytes.
    /// tasks.md §3.1: upload 48×48 PNG icon, capture ResourceId (BLAKE3 hash).
    #[tokio::test]
    async fn test_upload_icon_returns_32_byte_blake3_resource_id() {
        let resource_id_bytes = crate::upload_icon(TEST_AGENT_ID)
            .await
            .expect("upload_icon");

        // tasks.md §3.1: ResourceId is a 32-byte BLAKE3 digest.
        assert_eq!(
            resource_id_bytes.len(),
            blake3::OUT_LEN, // 32 bytes
            "ResourceId must be 32 bytes (BLAKE3 digest) — tasks.md §3.1, got {} bytes",
            resource_id_bytes.len()
        );
    }
}
