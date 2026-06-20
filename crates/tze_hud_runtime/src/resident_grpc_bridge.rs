//! Resident gRPC portal bridge (hud-d7frs).
//!
//! This module connects the **resident gRPC text-stream portal adapter**
//! ([`tze_hud_projection::resident_grpc::ResidentGrpcPortalAdapter`]) to a live
//! `HudSession` gRPC server as an *authenticated, capability-scoped* client. It
//! is the production counterpart of the stdio `projection_authority` dev harness
//! (`crates/tze_hud_projection/src/bin/projection_authority.rs`), which only
//! emits drain records to stdout for "a caller to forward" — i.e. the gRPC-
//! bridged resident path was *built yet unconnected* until this module.
//!
//! ## Two adapter families, one authority
//!
//! The in-process MCP cooperative path
//! ([`crate::portal_projection_driver::InProcessPortalDriver`]) hosts the single
//! [`ProjectionAuthority`] and materialises portal state by applying scene
//! mutations directly on the winit thread. This bridge is the **second adapter
//! family** required by the RFC 0013 §7.2 promotion gate
//! (`openspec/specs/text-stream-portals/spec.md` — *External Adapter Isolation*
//! and *Cooperative LLM Projection Adapter*): it takes the same authority's
//! [`ProjectedPortalState`] and materialises it over a real, authenticated gRPC
//! `HudSession` stream rather than via direct scene access.
//!
//! ## External Adapter Isolation (auth posture)
//!
//! Per the *External Adapter Isolation* requirement, an adapter that emits portal
//! output MUST authenticate and operate under explicit capability grants rather
//! than implicit local trust. This bridge therefore:
//!
//! - **fails closed** on an empty PSK ([`ResidentGrpcBridgeError::MissingPsk`]),
//!   mirroring the PSK-gated resident posture landed in #944 (hud-nu65o);
//! - presents the configured PSK in the `SessionInit` handshake;
//! - requests a capability-scoped session/lease
//!   ([`PORTAL_CAPABILITIES`] = `create_tiles` + `modify_own_tiles`) and verifies
//!   the runtime actually granted them before publishing;
//! - treats runtime denial (handshake, lease, or mutation) as authoritative.
//!
//! It never gains authority over an external process or transport lifecycle: it
//! is a cooperative gRPC client of the runtime's own session server.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Streaming;

use tze_hud_projection::ProjectedPortalState;
use tze_hud_projection::resident_grpc::{
    PortalVisualTokens, ResidentGrpcPortalAdapter, ResidentGrpcPortalConfig,
};
use tze_hud_protocol::proto::session::{
    ClientMessage, LeaseRequest, LeaseResponse, MutationResult, ServerMessage, SessionInit,
    client_message::Payload as ClientPayload, hud_session_client::HudSessionClient,
    server_message::Payload as ServerPayload,
};

/// Canonical v1 capability scope required for the resident portal adapter to
/// create and update its own raw tiles. Kept minimal (no input/topology/zone
/// scopes) so the resident session is least-privilege.
pub const PORTAL_CAPABILITIES: [&str; 2] = ["create_tiles", "modify_own_tiles"];

/// Default lease TTL requested for a resident portal lease.
const DEFAULT_LEASE_TTL_MS: u64 = 60_000;

/// Default lease priority (2 = agent-owned default per RFC 0008).
const DEFAULT_LEASE_PRIORITY: u32 = 2;

/// Bound on the outbound `ClientMessage` channel feeding the gRPC stream.
const OUTBOUND_CHANNEL_CAPACITY: usize = 64;

/// Bound on the inbound `ProjectedPortalState` channel feeding the bridge task.
///
/// State updates are latest-relevant; if the bridge falls behind, the runtime
/// drops the oldest queued snapshot (see [`spawn_resident_grpc_bridge`]).
const STATE_CHANNEL_CAPACITY: usize = 64;

/// Errors raised while connecting or publishing through the resident gRPC bridge.
#[derive(Debug, thiserror::Error)]
pub enum ResidentGrpcBridgeError {
    /// The configured PSK was empty — refuse to connect (fail-closed). The
    /// resident transport must authenticate; an empty secret never grants.
    #[error("resident gRPC portal bridge requires a non-empty PSK (fail-closed)")]
    MissingPsk,
    /// gRPC channel/transport-level failure (connect or stream open).
    #[error("resident gRPC transport error: {0}")]
    Transport(String),
    /// The session stream ended before the expected message arrived.
    #[error("resident gRPC session stream closed before {0}")]
    StreamClosed(&'static str),
    /// The server rejected the `SessionInit` handshake.
    #[error("resident gRPC handshake rejected: {0}")]
    Handshake(String),
    /// The runtime did not grant a capability the bridge requires.
    #[error("resident gRPC session not granted required capability {0:?}")]
    CapabilityNotGranted(&'static str),
    /// The runtime denied the lease request.
    #[error("resident gRPC lease denied: {code} {reason}")]
    LeaseDenied { code: String, reason: String },
    /// The runtime rejected a mutation batch.
    #[error("resident gRPC mutation rejected: {code} {message}")]
    MutationRejected { code: String, message: String },
    /// The outbound stream is closed (server hung up).
    #[error("resident gRPC outbound stream closed")]
    OutboundClosed,
    /// A `CreateTile` mutation was accepted but returned no tile id.
    #[error("resident gRPC CreateTile returned no created tile id")]
    MissingCreatedTile,
    /// The adapter failed to build an outbound message.
    #[error("resident gRPC adapter error: {0}")]
    Adapter(String),
}

/// Connection + identity configuration for the resident gRPC bridge.
#[derive(Clone, Debug)]
pub struct ResidentGrpcBridgeConfig {
    /// gRPC endpoint of the `HudSession` server, e.g. `http://127.0.0.1:50051`.
    pub endpoint: String,
    /// Pre-shared key presented in the handshake. MUST be non-empty.
    pub psk: String,
    /// Provider-neutral agent identity for the resident session.
    pub agent_id: String,
    /// Requested lease TTL in milliseconds.
    pub lease_ttl_ms: u64,
}

impl ResidentGrpcBridgeConfig {
    /// Build a config with the default lease TTL.
    pub fn new(
        endpoint: impl Into<String>,
        psk: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            psk: psk.into(),
            agent_id: agent_id.into(),
            lease_ttl_ms: DEFAULT_LEASE_TTL_MS,
        }
    }
}

/// An authenticated, capability-scoped resident gRPC portal client.
///
/// Holds one bidirectional `HudSession` stream and one
/// [`ResidentGrpcPortalAdapter`] per projection. Drive it by calling
/// [`ResidentGrpcPortalBridge::publish_state`] with authority-derived
/// [`ProjectedPortalState`]; the bridge renders the state into `HudSession`
/// mutations and ships them over the authenticated stream.
pub struct ResidentGrpcPortalBridge {
    /// Outbound sender feeding the gRPC client stream.
    tx: mpsc::Sender<ClientMessage>,
    /// Inbound server message stream.
    stream: Streaming<ServerMessage>,
    /// Per-projection adapters (own tile-id state + lease identity).
    adapters: HashMap<String, ResidentGrpcPortalAdapter>,
    /// Resolved visual tokens applied to every adapter.
    visual_tokens: PortalVisualTokens,
    /// Requested lease TTL.
    lease_ttl_ms: u64,
    /// Monotonic client message sequence.
    sequence: u64,
    /// Namespace assigned by the server at handshake.
    namespace: String,
    /// Capabilities the server granted at handshake.
    granted_capabilities: Vec<String>,
}

impl ResidentGrpcPortalBridge {
    /// Connect to the `HudSession` server, perform the authenticated handshake,
    /// and verify the required capability scope was granted.
    ///
    /// Fails closed on an empty PSK before opening any socket.
    pub async fn connect(
        config: &ResidentGrpcBridgeConfig,
        visual_tokens: PortalVisualTokens,
    ) -> Result<Self, ResidentGrpcBridgeError> {
        if config.psk.trim().is_empty() {
            return Err(ResidentGrpcBridgeError::MissingPsk);
        }

        let mut client = HudSessionClient::connect(config.endpoint.clone())
            .await
            .map_err(|e| ResidentGrpcBridgeError::Transport(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<ClientMessage>(OUTBOUND_CHANNEL_CAPACITY);
        let inbound = ReceiverStream::new(rx);

        // SessionInit MUST be the first message on the stream (RFC 0005 §4.1).
        let init = ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: config.agent_id.clone(),
                agent_display_name: format!("{} (resident gRPC portal)", config.agent_id),
                pre_shared_key: config.psk.clone(),
                requested_capabilities: PORTAL_CAPABILITIES.iter().map(|s| s.to_string()).collect(),
                initial_subscriptions: vec![],
                resume_token: vec![],
                agent_timestamp_wall_us: now_wall_us(),
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        };
        tx.send(init)
            .await
            .map_err(|_| ResidentGrpcBridgeError::OutboundClosed)?;

        let mut stream = client
            .session(inbound)
            .await
            .map_err(|e| ResidentGrpcBridgeError::Transport(e.to_string()))?
            .into_inner();

        // First server message must be SessionEstablished (or a SessionError).
        let established = loop {
            let msg = stream
                .next()
                .await
                .ok_or(ResidentGrpcBridgeError::StreamClosed("SessionEstablished"))?
                .map_err(|e| ResidentGrpcBridgeError::Transport(e.to_string()))?;
            match msg.payload {
                Some(ServerPayload::SessionEstablished(e)) => break e,
                Some(ServerPayload::SessionError(err)) => {
                    return Err(ResidentGrpcBridgeError::Handshake(format!(
                        "{}: {}",
                        err.code, err.message
                    )));
                }
                // Tolerate leading scene snapshots / lease state noise.
                _ => continue,
            }
        };

        // Capability verification: the runtime is the final authorizer; refuse to
        // proceed unless it granted the scope we need.
        for required in PORTAL_CAPABILITIES {
            if !established
                .granted_capabilities
                .iter()
                .any(|c| c == required)
            {
                return Err(ResidentGrpcBridgeError::CapabilityNotGranted(required));
            }
        }

        Ok(Self {
            tx,
            stream,
            adapters: HashMap::new(),
            visual_tokens,
            lease_ttl_ms: config.lease_ttl_ms,
            sequence: 1,
            namespace: established.namespace,
            granted_capabilities: established.granted_capabilities,
        })
    }

    /// Namespace assigned to this resident session by the runtime.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Capabilities the runtime granted at handshake.
    pub fn granted_capabilities(&self) -> &[String] {
        &self.granted_capabilities
    }

    /// Render `state` for `projection_id` and ship it over the authenticated
    /// gRPC stream, creating the portal tile on first publish.
    pub async fn publish_state(
        &mut self,
        projection_id: &str,
        state: &ProjectedPortalState,
    ) -> Result<(), ResidentGrpcBridgeError> {
        self.ensure_projection(projection_id).await?;

        let needs_create = self
            .adapters
            .get(projection_id)
            .map(|a| a.tile_id().is_none())
            .unwrap_or(false);

        if needs_create {
            let seq = self.next_seq();
            let ts = now_wall_us();
            let (message, batch_id) = {
                let adapter = self
                    .adapters
                    .get(projection_id)
                    .ok_or(ResidentGrpcBridgeError::MissingCreatedTile)?;
                let cmd = adapter
                    .ensure_portal_tile_message(state, seq, ts)
                    .map_err(|e| ResidentGrpcBridgeError::Adapter(e.to_string()))?;
                let batch_id = batch_id_of(&cmd.message);
                (cmd.message, batch_id)
            };
            Self::send(&self.tx, message).await?;
            let result = self.read_mutation_result(&batch_id).await?;
            if !result.accepted {
                return Err(ResidentGrpcBridgeError::MutationRejected {
                    code: result.error_code,
                    message: result.error_message,
                });
            }
            let tile_id = result
                .created_ids
                .into_iter()
                .next()
                .ok_or(ResidentGrpcBridgeError::MissingCreatedTile)?;
            if let Some(adapter) = self.adapters.get_mut(projection_id) {
                adapter.record_created_tile(tile_id);
            }
        }

        // Publish the portal content into the (now existing) tile.
        let seq = self.next_seq();
        let ts = now_wall_us();
        let (message, batch_id) = {
            let adapter = self
                .adapters
                .get(projection_id)
                .ok_or(ResidentGrpcBridgeError::MissingCreatedTile)?;
            let cmd = adapter
                .render_portal_message(state, seq, ts)
                .map_err(|e| ResidentGrpcBridgeError::Adapter(e.to_string()))?;
            let batch_id = batch_id_of(&cmd.message);
            (cmd.message, batch_id)
        };
        Self::send(&self.tx, message).await?;
        let result = self.read_mutation_result(&batch_id).await?;
        if !result.accepted {
            return Err(ResidentGrpcBridgeError::MutationRejected {
                code: result.error_code,
                message: result.error_message,
            });
        }
        Ok(())
    }

    /// Cleanly close the session: dropping `self` drops the outbound sender,
    /// which closes the client→server stream so the runtime tears down the
    /// session and releases the lease through its normal cleanup path.
    pub async fn shutdown(self) {
        // Explicit for readability; `Drop` would do the same.
        drop(self.tx);
        drop(self.stream);
    }

    /// Acquire a capability-scoped lease for `projection_id` and construct its
    /// adapter, if not already present.
    async fn ensure_projection(
        &mut self,
        projection_id: &str,
    ) -> Result<(), ResidentGrpcBridgeError> {
        if self.adapters.contains_key(projection_id) {
            return Ok(());
        }

        let seq = self.next_seq();
        let lease_req = ClientMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: self.lease_ttl_ms,
                capabilities: PORTAL_CAPABILITIES.iter().map(|s| s.to_string()).collect(),
                lease_priority: DEFAULT_LEASE_PRIORITY,
            })),
        };
        Self::send(&self.tx, lease_req).await?;
        let resp = self.read_lease_response().await?;
        if !resp.granted {
            return Err(ResidentGrpcBridgeError::LeaseDenied {
                code: resp.deny_code,
                reason: resp.deny_reason,
            });
        }

        let config = ResidentGrpcPortalConfig::new(resp.lease_id);
        let adapter = ResidentGrpcPortalAdapter::with_tokens(config, self.visual_tokens.clone());
        self.adapters.insert(projection_id.to_string(), adapter);
        Ok(())
    }

    fn next_seq(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    /// Send one outbound message. Takes `&mpsc::Sender` (Send + Sync) rather than
    /// `&self` so the resulting future stays `Send` — the bridge holds a
    /// `tonic::Streaming` which is `Send` but not `Sync`, so a `&self` borrow
    /// across an `.await` would make the spawned task non-`Send`.
    async fn send(
        tx: &mpsc::Sender<ClientMessage>,
        message: ClientMessage,
    ) -> Result<(), ResidentGrpcBridgeError> {
        tx.send(message)
            .await
            .map_err(|_| ResidentGrpcBridgeError::OutboundClosed)
    }

    async fn read_lease_response(&mut self) -> Result<LeaseResponse, ResidentGrpcBridgeError> {
        loop {
            let msg = self
                .stream
                .next()
                .await
                .ok_or(ResidentGrpcBridgeError::StreamClosed("LeaseResponse"))?
                .map_err(|e| ResidentGrpcBridgeError::Transport(e.to_string()))?;
            match msg.payload {
                Some(ServerPayload::LeaseResponse(resp)) => return Ok(resp),
                // A terminal session error must fail fast rather than blocking the
                // read loop forever waiting for a LeaseResponse that will never come.
                Some(ServerPayload::SessionError(err)) => {
                    return Err(ResidentGrpcBridgeError::Handshake(format!(
                        "session error while awaiting LeaseResponse: {}: {}",
                        err.code, err.message
                    )));
                }
                // LeaseStateChange / SceneSnapshot may interleave; keep reading.
                _ => continue,
            }
        }
    }

    async fn read_mutation_result(
        &mut self,
        batch_id: &[u8],
    ) -> Result<MutationResult, ResidentGrpcBridgeError> {
        loop {
            let msg = self
                .stream
                .next()
                .await
                .ok_or(ResidentGrpcBridgeError::StreamClosed("MutationResult"))?
                .map_err(|e| ResidentGrpcBridgeError::Transport(e.to_string()))?;
            match msg.payload {
                Some(ServerPayload::MutationResult(result)) if result.batch_id == batch_id => {
                    return Ok(result);
                }
                // A terminal session error must fail fast rather than blocking the
                // read loop forever waiting for a MutationResult that will never come.
                Some(ServerPayload::SessionError(err)) => {
                    return Err(ResidentGrpcBridgeError::Handshake(format!(
                        "session error while awaiting MutationResult: {}: {}",
                        err.code, err.message
                    )));
                }
                _ => continue,
            }
        }
    }
}

/// Handle to a spawned resident gRPC bridge task.
///
/// Feed authority-derived [`ProjectedPortalState`] snapshots through
/// [`ResidentGrpcBridgeHandle::state_sender`]; call
/// [`ResidentGrpcBridgeHandle::shutdown`] (async) or
/// [`ResidentGrpcBridgeHandle::abort`] (sync, for teardown) to stop the task
/// without leaking it.
pub struct ResidentGrpcBridgeHandle {
    state_tx: mpsc::Sender<(String, ProjectedPortalState)>,
    join: tokio::task::JoinHandle<()>,
}

impl ResidentGrpcBridgeHandle {
    /// A cloneable sender for feeding `(projection_id, state)` to the bridge.
    pub fn state_sender(&self) -> mpsc::Sender<(String, ProjectedPortalState)> {
        self.state_tx.clone()
    }

    /// Stop the task cooperatively (closes the feed channel, awaits exit).
    pub async fn shutdown(self) {
        drop(self.state_tx);
        let _ = self.join.await;
    }

    /// Abort the task synchronously (for sync teardown paths). Guarantees the
    /// task is cancelled so no listener/stream is leaked.
    pub fn abort(&self) {
        self.join.abort();
    }
}

/// Spawn a resident gRPC bridge task on the given runtime handle.
///
/// The task connects (authenticating with the configured PSK and verifying the
/// capability grant), then consumes `(projection_id, state)` updates and
/// publishes each over the authenticated stream. Connection failures are logged
/// and end the task (the in-process path is unaffected).
pub fn spawn_resident_grpc_bridge(
    runtime: &tokio::runtime::Handle,
    config: ResidentGrpcBridgeConfig,
    visual_tokens: PortalVisualTokens,
) -> ResidentGrpcBridgeHandle {
    let (state_tx, mut state_rx) =
        mpsc::channel::<(String, ProjectedPortalState)>(STATE_CHANNEL_CAPACITY);

    let join = runtime.spawn(async move {
        let mut bridge = match ResidentGrpcPortalBridge::connect(&config, visual_tokens).await {
            Ok(bridge) => bridge,
            Err(e) => {
                tracing::error!(
                    endpoint = %config.endpoint,
                    error = %e,
                    "resident gRPC portal bridge failed to connect; bridge disabled"
                );
                return;
            }
        };
        tracing::info!(
            endpoint = %config.endpoint,
            namespace = %bridge.namespace(),
            "resident gRPC portal bridge connected (two-adapter-families gate)"
        );

        while let Some((projection_id, state)) = state_rx.recv().await {
            if let Err(e) = bridge.publish_state(&projection_id, &state).await {
                tracing::warn!(
                    projection_id = %projection_id,
                    error = %e,
                    "resident gRPC portal bridge publish failed"
                );
                // A transport/stream failure is unrecoverable for this session;
                // stop so we do not spin on a dead stream.
                if matches!(
                    e,
                    ResidentGrpcBridgeError::OutboundClosed
                        | ResidentGrpcBridgeError::StreamClosed(_)
                        | ResidentGrpcBridgeError::Transport(_)
                ) {
                    break;
                }
            }
        }

        bridge.shutdown().await;
        tracing::info!("resident gRPC portal bridge task exited");
    });

    ResidentGrpcBridgeHandle { state_tx, join }
}

fn now_wall_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn batch_id_of(message: &ClientMessage) -> Vec<u8> {
    match &message.payload {
        Some(ClientPayload::MutationBatch(batch)) => batch.batch_id.clone(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tze_hud_projection::{
        AttachRequest, ContentClassification, OperationEnvelope, OutputKind, ProjectedPortalPolicy,
        ProjectionAuthority, ProjectionBounds, ProjectionOperation, ProviderKind,
        PublishOutputRequest,
    };
    use tze_hud_protocol::proto::session::hud_session_server::HudSessionServer;
    use tze_hud_protocol::session_server::HudSessionImpl;
    use tze_hud_scene::graph::SceneGraph;

    const TEST_PSK: &str = "resident-test-psk";

    /// Start an in-process `HudSession` gRPC server (production service impl) on
    /// an ephemeral loopback port and return its `http://` endpoint.
    async fn start_server() -> (String, tokio::task::JoinHandle<()>) {
        let mut scene = SceneGraph::new(1280.0, 720.0);
        // CreateTile with an empty tab_id targets the active tab; a fresh scene
        // has none, so seed one (auto-activated as the first tab).
        scene
            .create_tab("main", 0)
            .expect("create active tab for test scene");
        let service = HudSessionImpl::new(scene, TEST_PSK);

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        // Brief settle so the server task is listening before connect.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        (format!("http://[::1]:{}", addr.port()), handle)
    }

    /// Drive a real `ProjectionAuthority`: attach a projection and publish output
    /// so `projected_portal_state` returns content.
    fn authority_with_published_state(projection_id: &str) -> ProjectionAuthority {
        let mut authority = ProjectionAuthority::new(ProjectionBounds::default())
            .expect("authority init must succeed");
        let now_us = 1_000;

        let attach = authority.handle_attach(
            AttachRequest {
                envelope: OperationEnvelope {
                    operation: ProjectionOperation::Attach,
                    projection_id: projection_id.to_string(),
                    request_id: "req-attach".to_string(),
                    client_timestamp_wall_us: now_us,
                },
                provider_kind: ProviderKind::Other,
                display_name: "Resident Bridge Test".to_string(),
                workspace_hint: None,
                repository_hint: None,
                icon_profile_hint: None,
                content_classification: ContentClassification::Public,
                hud_target: None,
                idempotency_key: None,
            },
            "test-actor",
            now_us,
        );
        assert!(attach.accepted, "attach must be accepted");
        let owner_token = attach.owner_token.unwrap_or_default();

        let publish = authority.handle_publish_output(
            PublishOutputRequest {
                envelope: OperationEnvelope {
                    operation: ProjectionOperation::PublishOutput,
                    projection_id: projection_id.to_string(),
                    request_id: "req-publish".to_string(),
                    client_timestamp_wall_us: now_us + 1,
                },
                owner_token,
                output_text: "hello from the resident gRPC bridge".to_string(),
                output_kind: OutputKind::Assistant,
                content_classification: ContentClassification::Public,
                logical_unit_id: None,
                coalesce_key: None,
            },
            "test-actor",
            now_us + 1,
        );
        assert!(publish.accepted, "publish must be accepted");
        authority
    }

    #[tokio::test]
    async fn empty_psk_fails_closed_before_connect() {
        let config = ResidentGrpcBridgeConfig::new("http://[::1]:1", "   ", "resident-portal");
        let err =
            match ResidentGrpcPortalBridge::connect(&config, PortalVisualTokens::default()).await {
                Ok(_) => panic!("empty PSK must fail closed"),
                Err(e) => e,
            };
        assert!(matches!(err, ResidentGrpcBridgeError::MissingPsk));
    }

    #[tokio::test]
    async fn wrong_psk_is_rejected_at_handshake() {
        let (endpoint, _server) = start_server().await;
        let config = ResidentGrpcBridgeConfig::new(endpoint, "not-the-psk", "resident-portal");
        let err =
            match ResidentGrpcPortalBridge::connect(&config, PortalVisualTokens::default()).await {
                Ok(_) => panic!("wrong PSK must be rejected"),
                Err(e) => e,
            };
        assert!(
            matches!(err, ResidentGrpcBridgeError::Handshake(_)),
            "expected handshake rejection, got {err:?}"
        );
    }

    /// End-to-end: a real `ProjectionAuthority` produces state; the resident gRPC
    /// adapter renders it; the authenticated bridge ships it over a real gRPC
    /// `HudSession` stream; the production server accepts the create + publish.
    #[tokio::test]
    async fn resident_grpc_adapter_path_reaches_authority_end_to_end() {
        let projection_id = "proj-e2e";
        let authority = authority_with_published_state(projection_id);
        let state = authority
            .projected_portal_state(projection_id, &ProjectedPortalPolicy::permit_all())
            .expect("authority must yield projected portal state");

        let (endpoint, _server) = start_server().await;
        let config = ResidentGrpcBridgeConfig::new(endpoint, TEST_PSK, "resident-portal");

        let mut bridge = ResidentGrpcPortalBridge::connect(&config, PortalVisualTokens::default())
            .await
            .expect("authenticated connect must succeed");

        // Capability scope was actually granted by the runtime.
        for cap in PORTAL_CAPABILITIES {
            assert!(
                bridge.granted_capabilities().iter().any(|c| c == cap),
                "runtime must grant {cap}"
            );
        }

        // First publish creates the tile and publishes content over gRPC.
        bridge
            .publish_state(projection_id, &state)
            .await
            .expect("first publish (create + render) must be accepted");

        // Second publish reuses the existing tile.
        bridge
            .publish_state(projection_id, &state)
            .await
            .expect("second publish (reuse tile) must be accepted");

        bridge.shutdown().await;
    }
}
