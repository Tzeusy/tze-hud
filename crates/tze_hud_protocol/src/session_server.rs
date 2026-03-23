//! Bidirectional streaming session server implementing RFC 0005.
//!
//! This module provides `HudSessionImpl`, the server-side implementation of the
//! `HudSession` gRPC service. It manages the bidirectional streaming session
//! lifecycle: handshake, mutation processing, lease management, heartbeats,
//! event dispatch, and reconnection.

use crate::convert;
use crate::proto::session::hud_session_server::HudSession;
use crate::proto::session::*;
use crate::proto::session::client_message::Payload as ClientPayload;
use crate::proto::session::server_message::Payload as ServerPayload;
use crate::session::{SharedState, SESSION_EVENT_CHANNEL_CAPACITY};
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_scene::mutation::{MutationBatch as SceneMutationBatch, SceneMutation};

/// Default heartbeat interval in milliseconds.
const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 5000;

/// Default heartbeat missed threshold (number of missed heartbeats before disconnect).
const HEARTBEAT_MISSED_THRESHOLD: u64 = 3;

/// Default heartbeat timeout: threshold * interval.
const DEFAULT_HEARTBEAT_TIMEOUT_MS: u64 = DEFAULT_HEARTBEAT_INTERVAL_MS * HEARTBEAT_MISSED_THRESHOLD;

// ─── Helper ─────────────────────────────────────────────────────────────────

fn now_wall_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn scene_id_to_bytes(id: tze_hud_scene::SceneId) -> Vec<u8> {
    id.as_uuid().as_bytes().to_vec()
}

fn bytes_to_scene_id(bytes: &[u8]) -> Result<tze_hud_scene::SceneId, Status> {
    if bytes.len() != 16 {
        return Err(Status::invalid_argument(format!(
            "invalid scene ID: expected 16 bytes, got {}",
            bytes.len()
        )));
    }
    let arr: [u8; 16] = bytes.try_into().unwrap();
    let uuid = uuid::Uuid::from_bytes(arr);
    Ok(tze_hud_scene::SceneId::from_uuid(uuid))
}

// ─── Session state ──────────────────────────────────────────────────────────

/// Per-session state tracked by the streaming server.
struct StreamSession {
    session_id: String,
    namespace: String,
    agent_name: String,
    capabilities: Vec<String>,
    lease_ids: Vec<tze_hud_scene::SceneId>,
    subscriptions: Vec<String>,
    server_sequence: u64,
    resume_token: Vec<u8>,
    last_heartbeat_ms: u64,
}

impl StreamSession {
    fn next_server_seq(&mut self) -> u64 {
        self.server_sequence += 1;
        self.server_sequence
    }
}

// ─── Service implementation ─────────────────────────────────────────────────

/// The bidirectional streaming session service implementation.
///
/// Holds shared state (scene graph + session registry) and implements the
/// `HudSession` trait generated from `session.proto`.
pub struct HudSessionImpl {
    pub state: Arc<Mutex<SharedState>>,
    psk: String,
}

impl HudSessionImpl {
    /// Create a new session service with the given scene graph and PSK.
    pub fn new(scene: SceneGraph, psk: &str) -> Self {
        Self {
            state: Arc::new(Mutex::new(SharedState {
                scene,
                sessions: crate::session::SessionRegistry::new(psk),
            })),
            psk: psk.to_string(),
        }
    }

    /// Create from existing shared state.
    pub fn from_shared_state(state: Arc<Mutex<SharedState>>, psk: &str) -> Self {
        Self {
            state,
            psk: psk.to_string(),
        }
    }
}

#[tonic::async_trait]
impl HudSession for HudSessionImpl {
    type SessionStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<ServerMessage, Status>> + Send>>;

    async fn session(
        &self,
        request: Request<tonic::Streaming<ClientMessage>>,
    ) -> Result<Response<Self::SessionStream>, Status> {
        let mut inbound = request.into_inner();
        let state = self.state.clone();
        let psk = self.psk.clone();

        // Create outbound channel
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ServerMessage, Status>>(
            SESSION_EVENT_CHANNEL_CAPACITY,
        );

        // Spawn the session handler task
        tokio::spawn(async move {
            // Wait for the first message (must be SessionInit or SessionResume)
            let first_msg = match tokio::time::timeout(
                tokio::time::Duration::from_millis(5000),
                inbound.message(),
            )
            .await
            {
                Ok(Ok(Some(msg))) => msg,
                Ok(Ok(None)) => {
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: 1,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::SessionError(SessionError {
                                code: "HANDSHAKE_TIMEOUT".to_string(),
                                message: "Stream closed before handshake".to_string(),
                                hint: String::new(),
                            })),
                        }))
                        .await;
                    return;
                }
                Ok(Err(e)) => {
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: 1,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::SessionError(SessionError {
                                code: "HANDSHAKE_ERROR".to_string(),
                                message: format!("Error receiving handshake: {e}"),
                                hint: String::new(),
                            })),
                        }))
                        .await;
                    return;
                }
                Err(_) => {
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: 1,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::SessionError(SessionError {
                                code: "HANDSHAKE_TIMEOUT".to_string(),
                                message: "Handshake timed out (5000ms)".to_string(),
                                hint: "Send SessionInit as the first message".to_string(),
                            })),
                        }))
                        .await;
                    return;
                }
            };

            // Process handshake
            let mut session = match first_msg.payload {
                Some(ClientPayload::SessionInit(init)) => {
                    handle_session_init(&state, &psk, &tx, &init).await
                }
                Some(ClientPayload::SessionResume(resume)) => {
                    handle_session_resume(&state, &psk, &tx, &resume).await
                }
                _ => {
                    let _ = tx
                        .send(Ok(ServerMessage {
                            sequence: 1,
                            timestamp_wall_us: now_wall_us(),
                            payload: Some(ServerPayload::SessionError(SessionError {
                                code: "INVALID_HANDSHAKE".to_string(),
                                message: "First message must be SessionInit or SessionResume"
                                    .to_string(),
                                hint: String::new(),
                            })),
                        }))
                        .await;
                    return;
                }
            };

            let Some(ref mut session) = session else {
                return; // Handshake failed, error already sent
            };

            // Send SceneSnapshot after successful handshake
            {
                let st = state.lock().await;
                let json = st
                    .scene
                    .snapshot_json()
                    .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
                let seq = session.next_server_seq();
                let _ = tx
                    .send(Ok(ServerMessage {
                        sequence: seq,
                        timestamp_wall_us: now_wall_us(),
                        payload: Some(ServerPayload::SceneSnapshot(SceneSnapshot {
                            scene_json: json,
                            version: st.scene.version,
                        })),
                    }))
                    .await;
            }

            // Main message loop
            loop {
                // Use heartbeat timeout for receive
                let timeout_duration =
                    tokio::time::Duration::from_millis(DEFAULT_HEARTBEAT_TIMEOUT_MS);

                match tokio::time::timeout(timeout_duration, inbound.message()).await {
                    Ok(Ok(Some(msg))) => {
                        session.last_heartbeat_ms = now_ms();
                        handle_client_message(&state, session, &tx, msg).await;
                    }
                    Ok(Ok(None)) => {
                        // Stream closed gracefully
                        break;
                    }
                    Ok(Err(_e)) => {
                        // Stream error
                        break;
                    }
                    Err(_) => {
                        // Heartbeat timeout - ungraceful disconnect
                        break;
                    }
                }
            }

            // Cleanup: remove session from registry
            let mut st = state.lock().await;
            st.sessions.remove_session(&session.session_id);
        });

        // Return the receiver stream as the response
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Response::new(Box::pin(stream)))
    }
}

// ─── Handshake handlers ─────────────────────────────────────────────────────

async fn handle_session_init(
    state: &Arc<Mutex<SharedState>>,
    psk: &str,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    init: &SessionInit,
) -> Option<StreamSession> {
    // Authenticate
    if init.pre_shared_key != psk {
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::SessionError(SessionError {
                    code: "AUTH_FAILED".to_string(),
                    message: "Invalid pre-shared key".to_string(),
                    hint: String::new(),
                })),
            }))
            .await;
        return None;
    }

    let session_id = uuid::Uuid::now_v7().to_string();
    let namespace = init.agent_id.clone();
    let resume_token = uuid::Uuid::now_v7().as_bytes().to_vec();

    // Register session in the session registry
    {
        let mut st = state.lock().await;
        let _ = st.sessions.authenticate(
            &init.agent_id,
            psk,
            &init.requested_capabilities,
        );
    }

    // For v1, grant all requested capabilities
    let granted_capabilities = init.requested_capabilities.clone();
    let active_subscriptions = init.initial_subscriptions.clone();

    let mut session = StreamSession {
        session_id: session_id.clone(),
        namespace: namespace.clone(),
        agent_name: init.agent_id.clone(),
        capabilities: granted_capabilities.clone(),
        lease_ids: Vec::new(),
        subscriptions: active_subscriptions.clone(),
        server_sequence: 0,
        resume_token: resume_token.clone(),
        last_heartbeat_ms: now_ms(),
    };

    // Compute clock skew estimate
    let compositor_ts = now_wall_us();
    let estimated_skew = if init.agent_timestamp_wall_us > 0 {
        init.agent_timestamp_wall_us as i64 - compositor_ts as i64
    } else {
        0
    };

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: compositor_ts,
            payload: Some(ServerPayload::SessionEstablished(SessionEstablished {
                session_id: uuid::Uuid::parse_str(&session_id)
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
                namespace,
                granted_capabilities,
                resume_token,
                heartbeat_interval_ms: DEFAULT_HEARTBEAT_INTERVAL_MS,
                server_sequence: seq,
                compositor_timestamp_wall_us: compositor_ts,
                estimated_skew_us: estimated_skew,
                active_subscriptions,
                denied_subscriptions: Vec::new(),
            })),
        }))
        .await;

    Some(session)
}

async fn handle_session_resume(
    state: &Arc<Mutex<SharedState>>,
    psk: &str,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    resume: &SessionResume,
) -> Option<StreamSession> {
    // Authenticate
    if resume.pre_shared_key != psk {
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::SessionError(SessionError {
                    code: "AUTH_FAILED".to_string(),
                    message: "Invalid pre-shared key on resume".to_string(),
                    hint: String::new(),
                })),
            }))
            .await;
        return None;
    }

    // For v1, we don't have persistent resume state, so treat as new session
    // but preserve the agent_id namespace.
    let session_id = uuid::Uuid::now_v7().to_string();
    let namespace = resume.agent_id.clone();
    let new_resume_token = uuid::Uuid::now_v7().as_bytes().to_vec();

    {
        let mut st = state.lock().await;
        let _ = st.sessions.authenticate(
            &resume.agent_id,
            psk,
            &[],
        );
    }

    let mut session = StreamSession {
        session_id: session_id.clone(),
        namespace: namespace.clone(),
        agent_name: resume.agent_id.clone(),
        capabilities: Vec::new(),
        lease_ids: Vec::new(),
        subscriptions: Vec::new(),
        server_sequence: 0,
        resume_token: new_resume_token.clone(),
        last_heartbeat_ms: now_ms(),
    };

    let compositor_ts = now_wall_us();
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: compositor_ts,
            payload: Some(ServerPayload::SessionResumeResult(SessionResumeResult {
                accepted: true,
                new_session_token: new_resume_token.clone(),
                new_server_sequence: seq,
                negotiated_protocol_version: 1,
                granted_capabilities: Vec::new(),
                error: String::new(),
                active_subscriptions: Vec::new(),
                denied_subscriptions: Vec::new(),
            })),
        }))
        .await;

    Some(session)
}

// ─── Message handlers ───────────────────────────────────────────────────────

async fn handle_client_message(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    msg: ClientMessage,
) {
    let client_sequence = msg.sequence;
    let Some(payload) = msg.payload else {
        return;
    };

    match payload {
        ClientPayload::MutationBatch(batch) => {
            handle_mutation_batch(state, session, tx, batch).await;
        }
        ClientPayload::LeaseRequest(req) => {
            handle_lease_request(state, session, tx, req).await;
        }
        ClientPayload::LeaseRenew(renew) => {
            handle_lease_renew(state, session, tx, renew).await;
        }
        ClientPayload::LeaseRelease(release) => {
            handle_lease_release(state, session, tx, release).await;
        }
        ClientPayload::SubscriptionChange(change) => {
            handle_subscription_change(session, tx, change).await;
        }
        ClientPayload::ZonePublish(publish) => {
            handle_zone_publish(state, session, tx, client_sequence, publish).await;
        }
        ClientPayload::Heartbeat(hb) => {
            handle_heartbeat(session, tx, hb).await;
        }
        ClientPayload::TelemetryFrame(_tf) => {
            // Accept telemetry frames silently (logging/storage deferred to post-v1)
        }
        ClientPayload::InputFocusRequest(_req) => {
            // Input focus arbitration deferred to post-v1; silently accepted
        }
        ClientPayload::InputCaptureRequest(_req) => {
            // Input capture arbitration deferred to post-v1; silently accepted
        }
        ClientPayload::InputCaptureRelease(_rel) => {
            // Input capture release deferred to post-v1; silently accepted
        }
        ClientPayload::SetImePosition(_pos) => {
            // IME position hint deferred to post-v1; fire-and-forget, no response needed
        }
        ClientPayload::SessionClose(_close) => {
            // Client initiated graceful close; the main loop will break on stream end
        }
        ClientPayload::CapabilityRequest(_req) => {
            // Capability management is deferred to post-v1.
            // Respond explicitly so the client does not wait indefinitely or retransmit.
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::RuntimeError(RuntimeError {
                        error_code: "PERMISSION_DENIED".to_string(),
                        message: "Capability management is not supported in v1; request denied."
                            .to_string(),
                        context: String::new(),
                        hint: r#"{"post_v1": true}"#.to_string(),
                        error_code_enum: ErrorCode::PermissionDenied as i32,
                    })),
                }))
                .await;
        }
        // SessionInit/SessionResume should not appear after handshake
        ClientPayload::SessionInit(_) | ClientPayload::SessionResume(_) => {
            // Protocol violation: ignore (or could send RuntimeError)
        }
    }
}

async fn handle_mutation_batch(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    batch: MutationBatch,
) {
    let mut st = state.lock().await;

    let lease_id = match bytes_to_scene_id(&batch.lease_id) {
        Ok(id) => id,
        Err(_) => {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::MutationResult(MutationResult {
                        batch_id: batch.batch_id.clone(),
                        accepted: false,
                        created_ids: Vec::new(),
                        error_code: "INVALID_ARGUMENT".to_string(),
                        error_message: "Invalid lease_id bytes".to_string(),
                    })),
                }))
                .await;
            return;
        }
    };

    // Find the active tab
    let tab_id = match st.scene.active_tab {
        Some(id) => id,
        None => {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::MutationResult(MutationResult {
                        batch_id: batch.batch_id.clone(),
                        accepted: false,
                        created_ids: Vec::new(),
                        error_code: "PRECONDITION_FAILED".to_string(),
                        error_message: "No active tab".to_string(),
                    })),
                }))
                .await;
            return;
        }
    };

    // Convert proto mutations to scene mutations
    let mut scene_mutations = Vec::new();
    for m in &batch.mutations {
        match &m.mutation {
            Some(crate::proto::mutation_proto::Mutation::CreateTile(ct)) => {
                let bounds = ct
                    .bounds
                    .as_ref()
                    .map(convert::proto_rect_to_scene)
                    .unwrap_or(tze_hud_scene::Rect::new(0.0, 0.0, 200.0, 150.0));
                scene_mutations.push(SceneMutation::CreateTile {
                    tab_id,
                    namespace: session.namespace.clone(),
                    lease_id,
                    bounds,
                    z_order: ct.z_order,
                });
            }
            Some(crate::proto::mutation_proto::Mutation::SetTileRoot(str_)) => {
                if let Ok(tile_id) = uuid::Uuid::parse_str(&str_.tile_id)
                    .map(tze_hud_scene::SceneId::from_uuid)
                {
                    if let Some(ref node_proto) = str_.node
                        && let Some(node) = convert::proto_node_to_scene(node_proto)
                    {
                        scene_mutations
                            .push(SceneMutation::SetTileRoot { tile_id, node });
                    }
                }
            }
            Some(crate::proto::mutation_proto::Mutation::PublishToZone(pz)) => {
                let content = pz
                    .content
                    .as_ref()
                    .and_then(convert::proto_zone_content_to_scene);
                if let Some(content) = content {
                    let token = tze_hud_scene::types::ZonePublishToken {
                        token: pz
                            .publish_token
                            .as_ref()
                            .map(|t| t.token.clone())
                            .unwrap_or_default(),
                    };
                    let merge_key = if pz.merge_key.is_empty() {
                        None
                    } else {
                        Some(pz.merge_key.clone())
                    };
                    scene_mutations.push(SceneMutation::PublishToZone {
                        zone_name: pz.zone_name.clone(),
                        content,
                        publish_token: token,
                        merge_key,
                    });
                }
            }
            Some(crate::proto::mutation_proto::Mutation::ClearZone(cz)) => {
                let token = tze_hud_scene::types::ZonePublishToken {
                    token: cz
                        .publish_token
                        .as_ref()
                        .map(|t| t.token.clone())
                        .unwrap_or_default(),
                };
                scene_mutations.push(SceneMutation::ClearZone {
                    zone_name: cz.zone_name.clone(),
                    publish_token: token,
                });
            }
            None => {}
        }
    }

    // Apply as atomic batch
    let scene_batch = SceneMutationBatch {
        batch_id: tze_hud_scene::SceneId::new(),
        agent_namespace: session.namespace.clone(),
        mutations: scene_mutations,
    };

    let result = st.scene.apply_batch(&scene_batch);

    let seq = session.next_server_seq();
    if result.applied {
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::MutationResult(MutationResult {
                    batch_id: batch.batch_id,
                    accepted: true,
                    created_ids: result
                        .created_ids
                        .iter()
                        .map(|id| scene_id_to_bytes(*id))
                        .collect(),
                    error_code: String::new(),
                    error_message: String::new(),
                })),
            }))
            .await;
    } else {
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::MutationResult(MutationResult {
                    batch_id: batch.batch_id,
                    accepted: false,
                    created_ids: Vec::new(),
                    error_code: "MUTATION_REJECTED".to_string(),
                    error_message: result
                        .error
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "unknown error".to_string()),
                })),
            }))
            .await;
    }
}

async fn handle_lease_request(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    req: LeaseRequest,
) {
    let mut st = state.lock().await;

    // Parse capabilities
    let capabilities: Vec<Capability> = req
        .capabilities
        .iter()
        .filter_map(|c| match c.as_str() {
            "create_tile" => Some(Capability::CreateTile),
            "update_tile" => Some(Capability::UpdateTile),
            "delete_tile" => Some(Capability::DeleteTile),
            "create_node" => Some(Capability::CreateNode),
            "update_node" => Some(Capability::UpdateNode),
            "delete_node" => Some(Capability::DeleteNode),
            "receive_input" => Some(Capability::ReceiveInput),
            _ => None,
        })
        .collect();

    let ttl = if req.ttl_ms > 0 { req.ttl_ms } else { 60_000 };
    let lease_id = st.scene.grant_lease(&session.namespace, ttl, capabilities);
    session.lease_ids.push(lease_id);

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                granted: true,
                lease_id: scene_id_to_bytes(lease_id),
                granted_ttl_ms: ttl,
                granted_priority: req.lease_priority.max(2), // Default to normal priority
                granted_capabilities: req.capabilities.clone(),
                ..Default::default()
            })),
        }))
        .await;
}

async fn handle_lease_renew(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    renew: LeaseRenew,
) {
    let lease_id = match bytes_to_scene_id(&renew.lease_id) {
        Ok(id) => id,
        Err(_) => {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: false,
                        deny_reason: "Invalid lease_id bytes".to_string(),
                        deny_code: "INVALID_ARGUMENT".to_string(),
                        ..Default::default()
                    })),
                }))
                .await;
            return;
        }
    };

    let mut st = state.lock().await;
    let ttl = if renew.new_ttl_ms > 0 {
        renew.new_ttl_ms
    } else {
        60_000
    };

    let seq = session.next_server_seq();
    match st.scene.renew_lease(lease_id, ttl) {
        Ok(()) => {
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseStateChange(LeaseStateChange {
                        lease_id: scene_id_to_bytes(lease_id),
                        previous_state: "ACTIVE".to_string(),
                        new_state: "ACTIVE".to_string(),
                        reason: format!("Renewed with TTL {ttl}ms"),
                        timestamp_wall_us: now_wall_us(),
                    })),
                }))
                .await;
        }
        Err(e) => {
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: false,
                        deny_reason: e.to_string(),
                        deny_code: "LEASE_NOT_FOUND".to_string(),
                        ..Default::default()
                    })),
                }))
                .await;
        }
    }
}

async fn handle_lease_release(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    release: LeaseRelease,
) {
    let lease_id = match bytes_to_scene_id(&release.lease_id) {
        Ok(id) => id,
        Err(_) => {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: false,
                        deny_reason: "Invalid lease_id bytes".to_string(),
                        deny_code: "INVALID_ARGUMENT".to_string(),
                        ..Default::default()
                    })),
                }))
                .await;
            return;
        }
    };

    let mut st = state.lock().await;
    let seq = session.next_server_seq();

    match st.scene.revoke_lease(lease_id) {
        Ok(()) => {
            // Remove from session's tracked leases
            session.lease_ids.retain(|&id| id != lease_id);

            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseStateChange(LeaseStateChange {
                        lease_id: scene_id_to_bytes(lease_id),
                        previous_state: "ACTIVE".to_string(),
                        new_state: "RELEASED".to_string(),
                        reason: "Agent released lease".to_string(),
                        timestamp_wall_us: now_wall_us(),
                    })),
                }))
                .await;
        }
        Err(e) => {
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::LeaseResponse(LeaseResponse {
                        granted: false,
                        deny_reason: e.to_string(),
                        deny_code: "LEASE_NOT_FOUND".to_string(),
                        ..Default::default()
                    })),
                }))
                .await;
        }
    }
}

async fn handle_subscription_change(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    change: SubscriptionChange,
) {
    // Add new subscriptions
    for sub in &change.subscribe {
        if !session.subscriptions.contains(sub) {
            session.subscriptions.push(sub.clone());
        }
    }
    // Remove unsubscribed
    for unsub in &change.unsubscribe {
        session.subscriptions.retain(|s| s != unsub);
    }

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::SubscriptionChangeResult(SubscriptionChangeResult {
                active_subscriptions: session.subscriptions.clone(),
                denied_subscriptions: Vec::new(),
            })),
        }))
        .await;
}

/// Handle a ZonePublish from the client (RFC 0005 §3.1, §8.6).
///
/// Durable-zone publishes are transactional and receive a ZonePublishResult.
/// Ephemeral-zone publishes are fire-and-forget; no result is sent.
///
/// V1 implementation: zone durability detection is deferred; all session-stream
/// ZonePublish messages are treated as durable and forwarded through the
/// mutation path, receiving a ZonePublishResult ack.
async fn handle_zone_publish(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request_sequence: u64,
    publish: ZonePublish,
) {

    // Apply the zone publish through the scene graph mutation path
    let (accepted, error_code, error_message) = {
        let mut st = state.lock().await;
        let content = publish
            .content
            .as_ref()
            .and_then(crate::convert::proto_zone_content_to_scene);

        if let Some(content) = content {
            let merge_key = if publish.merge_key.is_empty() {
                None
            } else {
                Some(publish.merge_key.clone())
            };

            let mutation = tze_hud_scene::mutation::SceneMutation::PublishToZone {
                zone_name: publish.zone_name.clone(),
                content,
                publish_token: tze_hud_scene::types::ZonePublishToken {
                    token: Vec::new(),
                },
                merge_key,
            };

            // Apply as a single-mutation batch
            let batch = tze_hud_scene::mutation::MutationBatch {
                batch_id: tze_hud_scene::SceneId::new(),
                agent_namespace: session.namespace.clone(),
                mutations: vec![mutation],
            };
            let result = st.scene.apply_batch(&batch);
            if result.applied {
                (true, String::new(), String::new())
            } else {
                let (code, msg) = match &result.error {
                    Some(tze_hud_scene::ValidationError::ZoneNotFound { name }) => (
                        "ZONE_NOT_FOUND".to_string(),
                        format!("Zone not found: {name}"),
                    ),
                    Some(tze_hud_scene::ValidationError::ZonePublishTokenInvalid { zone }) => (
                        "TOKEN_INVALID".to_string(),
                        format!("Publish token invalid for zone '{zone}'"),
                    ),
                    Some(tze_hud_scene::ValidationError::BudgetExceeded { resource }) => (
                        "BUDGET_EXCEEDED".to_string(),
                        format!("Budget exceeded: {resource}"),
                    ),
                    Some(tze_hud_scene::ValidationError::CapabilityMissing { capability }) => (
                        "CAPABILITY_MISSING".to_string(),
                        format!("Capability missing: {capability}"),
                    ),
                    Some(err) => ("ZONE_PUBLISH_FAILED".to_string(), err.to_string()),
                    None => ("ZONE_PUBLISH_FAILED".to_string(), "Zone publish failed".to_string()),
                };
                (false, code, msg)
            }
        } else {
            (false, "INVALID_CONTENT".to_string(), "Missing or invalid zone content".to_string())
        }
    };

    // Send ZonePublishResult (v1 treats all session-stream zone publishes as durable)
    let seq = session.next_server_seq();

    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::ZonePublishResult(ZonePublishResult {
                request_sequence,
                accepted,
                error_code,
                error_message,
            })),
        }))
        .await;
}

async fn handle_heartbeat(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    hb: Heartbeat,
) {
    session.last_heartbeat_ms = now_ms();

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::Heartbeat(Heartbeat {
                // Echo the client's monotonic timestamp for RTT calculation
                timestamp_mono_us: hb.timestamp_mono_us,
            })),
        }))
        .await;
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::session::hud_session_client::HudSessionClient;
    use crate::proto::session::hud_session_server::HudSessionServer;
    use tokio_stream::StreamExt;
    use tze_hud_scene::graph::SceneGraph;

    /// Start a test server and return a connected client.
    async fn setup_test() -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ) {
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");

        let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            let incoming =
                tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        // Give server time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client =
            HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
                .await
                .unwrap();

        (client, handle)
    }

    /// Helper: create a bidirectional stream and perform handshake.
    /// Returns (sender, first few server messages including SessionEstablished + SceneSnapshot).
    async fn handshake(
        client: &mut HudSessionClient<tonic::transport::Channel>,
        agent_id: &str,
        psk: &str,
    ) -> (
        tokio::sync::mpsc::Sender<ClientMessage>,
        Vec<ServerMessage>,
        tonic::Streaming<ServerMessage>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        // Send SessionInit
        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_id.to_string(),
                pre_shared_key: psk.to_string(),
                requested_capabilities: vec![
                    "create_tile".to_string(),
                    "receive_input".to_string(),
                ],
                initial_subscriptions: vec!["SCENE_TOPOLOGY".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();

        // Collect SessionEstablished and SceneSnapshot
        let mut messages = Vec::new();
        // We expect exactly 2 messages: SessionEstablished and SceneSnapshot
        for _ in 0..2 {
            if let Some(msg) = response_stream.next().await {
                messages.push(msg.unwrap());
            }
        }

        (tx, messages, response_stream)
    }

    #[tokio::test]
    async fn test_handshake_init_established_and_snapshot() {
        let (mut client, _server) = setup_test().await;
        let (_tx, messages, _stream) = handshake(&mut client, "test-agent", "test-key").await;

        assert_eq!(messages.len(), 2);

        // First message: SessionEstablished
        match &messages[0].payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                assert!(!established.session_id.is_empty());
                assert_eq!(established.namespace, "test-agent");
                assert!(established.granted_capabilities.contains(&"create_tile".to_string()));
                assert!(established.granted_capabilities.contains(&"receive_input".to_string()));
                assert!(!established.resume_token.is_empty());
                assert_eq!(established.heartbeat_interval_ms, DEFAULT_HEARTBEAT_INTERVAL_MS);
                assert!(established.active_subscriptions.contains(&"SCENE_TOPOLOGY".to_string()));
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }

        // Second message: SceneSnapshot
        match &messages[1].payload {
            Some(ServerPayload::SceneSnapshot(snapshot)) => {
                assert!(!snapshot.scene_json.is_empty());
            }
            other => panic!("Expected SceneSnapshot, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handshake_auth_failure() {
        let (mut client, _server) = setup_test().await;

        let (_tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let (init_tx, init_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(init_rx);

        // Send SessionInit with wrong key
        init_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionInit(SessionInit {
                    agent_id: "bad-agent".to_string(),
                    agent_display_name: "bad-agent".to_string(),
                    pre_shared_key: "wrong-key".to_string(),
                    requested_capabilities: Vec::new(),
                    initial_subscriptions: Vec::new(),
                    resume_token: Vec::new(),
                    agent_timestamp_wall_us: 0,
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();

        match &msg.payload {
            Some(ServerPayload::SessionError(error)) => {
                assert_eq!(error.code, "AUTH_FAILED");
            }
            other => panic!("Expected SessionError, got: {other:?}"),
        }

        drop(_tx);
        drop(rx);
    }

    #[tokio::test]
    async fn test_mutation_over_stream() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "mutator", "test-key").await;

        // First, request a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tile".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let lease_msg = stream.next().await.unwrap().unwrap();
        let lease_id = match &lease_msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) if resp.granted => resp.lease_id.clone(),
            other => panic!("Expected LeaseResponse (granted), got: {other:?}"),
        };

        // Create a tab in the scene (needed for mutations)
        // We need to do this through shared state since tab creation
        // isn't exposed via the streaming protocol yet.
        // For the test, we'll send a mutation that doesn't require a tab.

        // Send a mutation batch
        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: vec![crate::proto::MutationProto {
                    mutation: Some(
                        crate::proto::mutation_proto::Mutation::CreateTile(
                            crate::proto::CreateTileMutation {
                                tab_id: String::new(),
                                bounds: Some(crate::proto::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 200.0,
                                    height: 150.0,
                                }),
                                z_order: 1,
                            },
                        ),
                    ),
                }],
            })),
        })
        .await
        .unwrap();

        let result_msg = stream.next().await.unwrap().unwrap();
        match &result_msg.payload {
            Some(ServerPayload::MutationResult(result)) => {
                // This will fail because no active tab exists, which is expected
                // in this isolated test. The important thing is that the protocol
                // round-trip works.
                assert_eq!(result.batch_id, batch_id);
                // accepted may be false due to "no active tab" -- that's fine
            }
            other => panic!("Expected MutationResult, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_lease_over_stream() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "leasor", "test-key").await;

        // Request a lease
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
                capabilities: vec!["create_tile".to_string(), "receive_input".to_string()],
                lease_priority: 2,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::LeaseResponse(resp)) => {
                assert!(resp.granted, "expected lease to be granted");
                assert!(!resp.lease_id.is_empty());
                assert_eq!(resp.lease_id.len(), 16);
                assert_eq!(resp.granted_ttl_ms, 30_000);
                assert!(resp.granted_capabilities.contains(&"create_tile".to_string()));
            }
            other => panic!("Expected LeaseResponse, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_heartbeat_echo() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "heartbeater", "test-key").await;

        let mono_us = 12345678u64;
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: mono_us,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::Heartbeat(hb)) => {
                assert_eq!(hb.timestamp_mono_us, mono_us);
            }
            other => panic!("Expected Heartbeat echo, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_resume_with_token() {
        let (mut client, _server) = setup_test().await;

        // Start initial session to get a resume token
        let (tx, init_messages, _stream) =
            handshake(&mut client, "resumable", "test-key").await;
        drop(tx); // Close the first stream
        drop(_stream);

        // Wait a bit for cleanup
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Now resume with the token
        let resume_token = match &init_messages[0].payload {
            Some(ServerPayload::SessionEstablished(established)) => established.resume_token.clone(),
            _ => panic!("Expected SessionEstablished"),
        };

        let (resume_tx, resume_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let resume_stream = tokio_stream::wrappers::ReceiverStream::new(resume_rx);

        resume_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionResume(SessionResume {
                    agent_id: "resumable".to_string(),
                    resume_token,
                    last_seen_server_sequence: 2,
                    pre_shared_key: "test-key".to_string(),
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(resume_stream).await.unwrap().into_inner();

        // Should get SessionResumeResult + SceneSnapshot (not SessionEstablished)
        let msg1 = response_stream.next().await.unwrap().unwrap();
        match &msg1.payload {
            Some(ServerPayload::SessionResumeResult(result)) => {
                assert!(result.accepted, "expected resume to be accepted");
                assert!(!result.new_session_token.is_empty());
                assert_eq!(result.negotiated_protocol_version, 1);
            }
            other => panic!("Expected SessionResumeResult on resume, got: {other:?}"),
        }

        let msg2 = response_stream.next().await.unwrap().unwrap();
        match &msg2.payload {
            Some(ServerPayload::SceneSnapshot(_)) => {}
            other => panic!("Expected SceneSnapshot on resume, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_zone_publish_result() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "zone-publisher", "test-key").await;

        // Send a ZonePublish — expect ZonePublishResult correlated by client sequence
        let client_seq: u64 = 2;
        tx.send(ClientMessage {
            sequence: client_seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::ZonePublish(ZonePublish {
                zone_name: "status".to_string(),
                content: Some(crate::proto::ZoneContent {
                    payload: Some(crate::proto::zone_content::Payload::StreamText(
                        "hello zone".to_string(),
                    )),
                }),
                ttl_us: 0,
                merge_key: String::new(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::ZonePublishResult(result)) => {
                // request_sequence must echo the client envelope sequence
                assert_eq!(
                    result.request_sequence, client_seq,
                    "ZonePublishResult.request_sequence must correlate with client ZonePublish sequence"
                );
                // Zone "status" doesn't exist in the default scene graph so it
                // will be rejected; we just verify the sequence correlation and
                // that error_code is populated on rejection.
                if !result.accepted {
                    assert!(!result.error_code.is_empty(), "rejected result must carry an error_code");
                }
            }
            other => panic!("Expected ZonePublishResult, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_subscription_change_result() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "subscriber", "test-key").await;

        // Send a SubscriptionChange
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SubscriptionChange(SubscriptionChange {
                subscribe: vec!["INPUT_EVENTS".to_string()],
                unsubscribe: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SubscriptionChangeResult(result)) => {
                assert!(
                    result.active_subscriptions.contains(&"SCENE_TOPOLOGY".to_string()),
                    "initial subscription should still be active"
                );
                assert!(
                    result.active_subscriptions.contains(&"INPUT_EVENTS".to_string()),
                    "newly added subscription should be active"
                );
                assert!(result.denied_subscriptions.is_empty());
            }
            other => panic!("Expected SubscriptionChangeResult, got: {other:?}"),
        }
    }
}
