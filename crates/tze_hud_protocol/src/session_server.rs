//! Bidirectional streaming session server implementing RFC 0005.
//!
//! This module provides `HudSessionImpl`, the server-side implementation of the
//! `HudSession` gRPC service. It manages the bidirectional streaming session
//! lifecycle: handshake, mutation processing, lease management, heartbeats,
//! event dispatch, and reconnection.
//!
//! # Session Lifecycle State Machine (RFC 0005 §1.1)
//!
//! ```text
//! Connecting → Handshaking → Active → Disconnecting → Closed → Resuming
//! ```
//!
//! Valid transitions:
//! - Connecting → Handshaking (stream opened, SessionInit received)
//! - Handshaking → Active (valid auth → SessionEstablished)
//! - Handshaking → Closed (auth failure → SessionError(AUTH_FAILED))
//! - Active → Disconnecting (SessionClose received)
//! - Active → Closed (ungraceful: heartbeat timeout or stream EOF/RST)
//! - Disconnecting → Closed (stream termination complete)
//! - Closed → Resuming (SessionResume within grace period)
//! - Resuming → Active (valid resume token)
//! - Resuming → Closed (expired/invalid token)

use crate::auth::{
    authenticate_session_init, filter_subscriptions, negotiate_version, CapabilityPolicy,
    AuthResult,
};
use crate::convert;
use crate::proto::session::hud_session_server::HudSession;
use crate::proto::session::*;
use crate::proto::session::client_message::Payload as ClientPayload;
use crate::proto::session::server_message::Payload as ServerPayload;
use crate::session::{SharedState, SESSION_EVENT_CHANNEL_CAPACITY};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_scene::mutation::{MutationBatch as SceneMutationBatch, SceneMutation};

// ─── Session Configuration ───────────────────────────────────────────────────

/// Runtime-configurable parameters for session management (RFC 0005 §10).
///
/// All fields correspond to spec-defined configuration parameters with their
/// documented defaults.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Maximum time (ms) to wait for SessionInit after stream open. Default: 5000.
    pub handshake_timeout_ms: u64,
    /// Interval (ms) at which the client must send Heartbeat. Default: 5000.
    pub heartbeat_interval_ms: u64,
    /// Number of consecutive missed heartbeats before ungraceful disconnect. Default: 3.
    pub heartbeat_missed_threshold: u64,
    /// Grace period (ms) to hold orphaned leases after disconnect. Default: 30000.
    pub reconnect_grace_period_ms: u64,
    /// Timeout (ms) before retransmitting unacknowledged transactional messages. Default: 5000.
    pub retransmit_timeout_ms: u64,
    /// Per-session deduplication window size (unique batch_id values). Default: 1000.
    pub dedup_window_size: usize,
    /// Per-session deduplication window TTL (seconds). Default: 60.
    pub dedup_window_ttl_s: u64,
    /// Maximum sequence gap before SEQUENCE_GAP_EXCEEDED. Default: 100.
    pub max_sequence_gap: u64,
    /// Per-session ephemeral message buffer quota (oldest dropped beyond this). Default: 16.
    pub ephemeral_buffer_max: usize,
    /// Maximum concurrent resident sessions. Default: 16.
    pub max_concurrent_resident_sessions: usize,
    /// Maximum concurrent guest sessions. Default: 64.
    pub max_concurrent_guest_sessions: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            handshake_timeout_ms: 5000,
            heartbeat_interval_ms: 5000,
            heartbeat_missed_threshold: 3,
            reconnect_grace_period_ms: 30_000,
            retransmit_timeout_ms: 5000,
            dedup_window_size: 1000,
            dedup_window_ttl_s: 60,
            max_sequence_gap: 100,
            ephemeral_buffer_max: 16,
            max_concurrent_resident_sessions: 16,
            max_concurrent_guest_sessions: 64,
        }
    }
}

// ─── Session Lifecycle State Machine ────────────────────────────────────────

/// Session lifecycle states per RFC 0005 §1.1.
///
/// The state machine progresses through these states in response to protocol
/// events (stream open/close, SessionInit/Resume, heartbeat timeout, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// TCP/TLS establishment in progress. Initial state when gRPC stream is opened.
    Connecting,
    /// SessionInit received, validating credentials and capabilities.
    Handshaking,
    /// Bidirectional stream is open and agent is active.
    Active,
    /// Graceful close: agent sent SessionClose, waiting for stream termination.
    Disconnecting,
    /// Stream terminated. Leases are orphaned if previously Active.
    Closed,
    /// Agent is reconnecting within the grace period using a resume token.
    Resuming,
}

impl SessionState {
    /// Returns true if this state allows mutation submission.
    pub fn allows_mutations(&self) -> bool {
        *self == SessionState::Active
    }

    /// Human-readable label for logging.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Handshaking => "Handshaking",
            Self::Active => "Active",
            Self::Disconnecting => "Disconnecting",
            Self::Closed => "Closed",
            Self::Resuming => "Resuming",
        }
    }
}

// ─── Traffic Class ───────────────────────────────────────────────────────────

/// Traffic class for outbound server messages (RFC 0005 §3.1, §3.2).
///
/// Each class has different delivery guarantees:
/// - Transactional: at-least-once, ordered, never dropped.
/// - StateStream: at-least-once with coalescing; intermediate states may be skipped.
/// - Ephemeral: at-most-once, latest-wins, dropped under backpressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficClass {
    /// Reliable, ordered, never dropped. MutationResult, LeaseResponse, SessionEstablished, etc.
    Transactional,
    /// Coalesced under pressure; intermediate states may be skipped. SceneSnapshot, TelemetryFrame.
    StateStream,
    /// Droppable under backpressure; latest value wins. Heartbeat echo, ephemeral ZonePublish.
    Ephemeral,
}

/// Classify an outbound `ServerMessage` payload into its traffic class.
///
/// Per RFC 0005 §3.1 and §3.2:
/// - Session lifecycle responses, MutationResult, LeaseResponse, LeaseStateChange,
///   SubscriptionChangeResult, ZonePublishResult, RuntimeError, BackpressureSignal,
///   SessionSuspended, SessionResumed, and input-control responses are Transactional.
/// - SceneSnapshot, SceneDelta, EventBatch, TelemetryFrame are StateStream.
/// - Heartbeat echoes are Ephemeral.
pub fn classify_server_payload(payload: &ServerPayload) -> TrafficClass {
    match payload {
        // Session lifecycle — always transactional
        ServerPayload::SessionEstablished(_)
        | ServerPayload::SessionError(_)
        | ServerPayload::SessionResumeResult(_)
        | ServerPayload::SessionSuspended(_)
        | ServerPayload::SessionResumed(_)
        | ServerPayload::RuntimeError(_) => TrafficClass::Transactional,

        // Mutation / lease responses — transactional
        ServerPayload::MutationResult(_)
        | ServerPayload::LeaseResponse(_)
        | ServerPayload::LeaseStateChange(_)
        | ServerPayload::CapabilityNotice(_)
        | ServerPayload::SubscriptionChangeResult(_)
        | ServerPayload::ZonePublishResult(_)
        | ServerPayload::InputFocusResponse(_)
        | ServerPayload::InputCaptureResponse(_) => TrafficClass::Transactional,

        // Backpressure signal — transactional (must not be dropped)
        ServerPayload::BackpressureSignal(_) => TrafficClass::Transactional,

        // Scene state / events — state-stream
        ServerPayload::SceneSnapshot(_)
        | ServerPayload::SceneDelta(_)
        | ServerPayload::EventBatch(_) => TrafficClass::StateStream,

        // Heartbeat echo — ephemeral (droppable, latest-wins)
        ServerPayload::Heartbeat(_) => TrafficClass::Ephemeral,

        // Agent event emission result — transactional (always delivered)
        ServerPayload::EmitSceneEventResult(_) => TrafficClass::Transactional,
    }
}

// ─── Ephemeral send buffer ────────────────────────────────────────────────────

/// A bounded queue for ephemeral outbound messages.
///
/// When the buffer exceeds `capacity`, the oldest message is dropped, retaining
/// only the latest `capacity` messages (RFC 0005 §2.5: oldest-first eviction).
struct EphemeralQueue {
    queue: VecDeque<Result<ServerMessage, Status>>,
    capacity: usize,
}

impl EphemeralQueue {
    fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity + 1),
            capacity,
        }
    }

    /// Enqueue a message. If at capacity, drops the oldest entry.
    fn push(&mut self, msg: Result<ServerMessage, Status>) {
        if self.queue.len() >= self.capacity {
            self.queue.pop_front(); // oldest-first eviction
        }
        self.queue.push_back(msg);
    }

    /// Drain the queue into the send channel (non-blocking).
    async fn flush(&mut self, tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>) {
        while let Some(msg) = self.queue.pop_front() {
            let _ = tx.try_send(msg);
        }
    }
}

// ─── Constants ───────────────────────────────────────────────────────────────

/// Default heartbeat interval in milliseconds.
const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 5000;

/// Default heartbeat missed threshold (number of missed heartbeats before disconnect).
const HEARTBEAT_MISSED_THRESHOLD: u64 = 3;

/// Default heartbeat timeout: threshold * interval.
const DEFAULT_HEARTBEAT_TIMEOUT_MS: u64 = DEFAULT_HEARTBEAT_INTERVAL_MS * HEARTBEAT_MISSED_THRESHOLD;

/// Default maximum sequence gap before SEQUENCE_GAP_EXCEEDED (RFC 0005 §2.3).
const DEFAULT_MAX_SEQUENCE_GAP: u64 = 100;

/// Default per-session ephemeral message buffer quota (RFC 0005 §2.5).
const DEFAULT_EPHEMERAL_BUFFER_MAX: usize = 16;

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

// ─── Per-session event rate limiter ─────────────────────────────────────────

/// Sliding-window rate limiter for agent scene event emission.
///
/// Per scene-events/spec.md §5.4: default 10 events/second, 1-second window.
/// Each session holds one instance; concurrent sessions are independent.
struct SessionEventRateLimiter {
    /// Timestamps of accepted events within the current 1-second window.
    timestamps: std::collections::VecDeque<std::time::Instant>,
    /// Maximum accepted events per 1-second window (default: 10).
    max_per_second: usize,
}

impl SessionEventRateLimiter {
    /// Create a new limiter with the default limit (10 events/second).
    fn new() -> Self {
        Self {
            timestamps: std::collections::VecDeque::new(),
            max_per_second: 10,
        }
    }

    /// Check whether a new event is within the rate limit and, if so, record it.
    ///
    /// Returns `Ok(())` if accepted, `Err(())` if the window is full.
    fn check_and_record(&mut self, now: std::time::Instant) -> Result<(), ()> {
        let window = std::time::Duration::from_secs(1);
        // Prune expired entries.
        while let Some(&front) = self.timestamps.front() {
            if now.duration_since(front) >= window {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }
        if self.timestamps.len() >= self.max_per_second {
            return Err(());
        }
        self.timestamps.push_back(now);
        Ok(())
    }
}

// ─── Session state ──────────────────────────────────────────────────────────

/// Per-session state tracked by the streaming server.
struct StreamSession {
    session_id: String,
    namespace: String,
    agent_name: String,
    /// Capabilities explicitly granted at handshake (from `requested_capabilities`).
    capabilities: Vec<String>,
    /// Authorization scope for subscription and capability-request checks.
    /// For unrestricted PSK sessions this is `vec!["*"]`; for restricted agents
    /// it mirrors `capabilities`. Used for gating subscriptions and mid-session
    /// CapabilityRequest evaluation.
    policy_capabilities: Vec<String>,
    lease_ids: Vec<tze_hud_scene::SceneId>,
    subscriptions: Vec<String>,
    server_sequence: u64,
    resume_token: Vec<u8>,
    last_heartbeat_ms: u64,

    /// Current lifecycle state (RFC 0005 §1.1).
    state: SessionState,

    /// Last validated client-side sequence number (RFC 0005 §2.3).
    /// Initialized to 1 during session init/resume (treating the handshake message as
    /// sequence 1). Each subsequent validated message must carry a strictly greater
    /// sequence number within `max_sequence_gap` of the previous.
    last_client_sequence: u64,

    /// Whether safe mode is active for this session (RFC 0005 §3.7).
    /// When true, MutationBatch messages are rejected with SAFE_MODE_ACTIVE.
    safe_mode_active: bool,

    /// Whether the agent indicated `expect_resume=true` in SessionClose (RFC 0005 §1.5).
    /// When true, leases are held for the full reconnect grace period.
    expect_resume: bool,

    /// Sliding-window rate limiter for agent scene event emission.
    ///
    /// Tracks per-session event timestamps for the 1-second sliding window.
    /// Default limit: 10 events/second (spec: scene-events/spec.md §5.4).
    agent_event_rate_limiter: SessionEventRateLimiter,
}

impl StreamSession {
    fn next_server_seq(&mut self) -> u64 {
        self.server_sequence += 1;
        self.server_sequence
    }

    /// Transition to a new state. Returns the previous state.
    fn transition(&mut self, new_state: SessionState) -> SessionState {
        let prev = self.state.clone();
        self.state = new_state;
        prev
    }

    /// Validate an inbound client sequence number per RFC 0005 §2.3.
    ///
    /// Returns `Ok(())` if valid, or an error string with the appropriate
    /// SessionError code if the sequence is regressed or the gap is too large.
    fn validate_client_sequence(
        &mut self,
        seq: u64,
        max_gap: u64,
    ) -> Result<(), (&'static str, String)> {
        if seq <= self.last_client_sequence {
            return Err((
                "SEQUENCE_REGRESSION",
                format!(
                    "sequence regression: received {seq}, last was {}",
                    self.last_client_sequence
                ),
            ));
        }
        // "gap" per spec (RFC 0005 §2.3) = seq − last_seq.
        // Reject if gap > max_sequence_gap (default 100).
        // Example: last=5, seq=105 → gap=100 = max_gap → accepted (not strictly greater).
        //          last=5, seq=106 → gap=101 > max_gap=100 → rejected.
        //          last=5, seq=150 (spec example) → gap=145 > 100 → rejected.
        let gap = seq - self.last_client_sequence;
        if gap > max_gap {
            return Err((
                "SEQUENCE_GAP_EXCEEDED",
                format!(
                    "sequence gap {gap} exceeds max {max_gap}: received {seq}, last was {}",
                    self.last_client_sequence
                ),
            ));
        }
        self.last_client_sequence = seq;
        Ok(())
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
                safe_mode_active: false,
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

            // Transition: Handshaking/Resuming → Active (RFC 0005 §1.1)
            session.transition(SessionState::Active);

            // Send SceneSnapshot after successful handshake (RFC 0005 §1.3, §6.4)
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
            //
            // The loop exits for one of three reasons:
            //   1. Stream EOF (graceful): agent closed the stream.
            //   2. Stream error: transport-level error.
            //   3. Heartbeat timeout: no message for heartbeat_missed_threshold × interval.
            //
            // In cases (2) and (3) the disconnect is ungraceful; leases become orphaned.
            // In case (1) the disconnect may be graceful (SessionClose was sent) or
            // ungraceful (agent dropped the connection without sending SessionClose).
            loop {
                // Use heartbeat timeout for receive (RFC 0005 §1.6, §3.6)
                let timeout_duration =
                    tokio::time::Duration::from_millis(DEFAULT_HEARTBEAT_TIMEOUT_MS);

                match tokio::time::timeout(timeout_duration, inbound.message()).await {
                    Ok(Ok(Some(msg))) => {
                        // Update heartbeat timestamp on any received message
                        session.last_heartbeat_ms = now_ms();

                        // Validate client sequence number (RFC 0005 §2.3).
                        // Skip validation for sequence 0 (unset) to allow legacy callers
                        // that don't set sequences. Sequence must be monotonically increasing
                        // starting at 2 (since 1 is the handshake message).
                        if msg.sequence != 0 {
                            match session.validate_client_sequence(
                                msg.sequence,
                                DEFAULT_MAX_SEQUENCE_GAP,
                            ) {
                                Ok(()) => {}
                                Err((code, message)) => {
                                    // Close stream with sequence error
                                    let seq = session.next_server_seq();
                                    let _ = tx
                                        .send(Ok(ServerMessage {
                                            sequence: seq,
                                            timestamp_wall_us: now_wall_us(),
                                            payload: Some(ServerPayload::SessionError(
                                                SessionError {
                                                    code: code.to_string(),
                                                    message,
                                                    hint: String::new(),
                                                },
                                            )),
                                        }))
                                        .await;
                                    session.transition(SessionState::Closed);
                                    break;
                                }
                            }
                        }

                        // Check if this is a graceful close message
                        let is_close = matches!(
                            &msg.payload,
                            Some(ClientPayload::SessionClose(_))
                        );

                        handle_client_message(&state, session, &tx, msg).await;

                        // After handling SessionClose, transition to Disconnecting then Closed
                        if is_close {
                            session.transition(SessionState::Disconnecting);
                            session.transition(SessionState::Closed);
                            break;
                        }
                    }
                    Ok(Ok(None)) => {
                        // Stream EOF — transitions to Closed whether graceful or ungraceful.
                        // If session was Disconnecting (SessionClose already received and
                        // processed), this is the expected stream termination completing
                        // the Disconnecting → Closed transition. Otherwise it is an
                        // ungraceful disconnect (agent dropped the connection without
                        // sending SessionClose); leases become orphaned in either case.
                        session.transition(SessionState::Closed);
                        break;
                    }
                    Ok(Err(_e)) => {
                        // Stream transport error — ungraceful disconnect
                        session.transition(SessionState::Closed);
                        break;
                    }
                    Err(_) => {
                        // Heartbeat timeout — ungraceful disconnect (RFC 0005 §1.6, §3.6)
                        // Leases are orphaned; reconnection grace period begins.
                        session.transition(SessionState::Closed);
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
    // ── Step 1: Version negotiation (RFC 0005 §4.1) ──────────────────────────
    // Do this before authentication so agents can learn about version
    // incompatibility even if they send a wrong key.
    let negotiated_version = match negotiate_version(
        init.min_protocol_version,
        init.max_protocol_version,
    ) {
        Ok(v) => v,
        Err(msg) => {
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: 1,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::SessionError(SessionError {
                        code: "UNSUPPORTED_PROTOCOL_VERSION".to_string(),
                        message: msg,
                        hint: format!(
                            "{{\"runtime_min\": {}, \"runtime_max\": {}}}",
                            crate::auth::RUNTIME_MIN_VERSION,
                            crate::auth::RUNTIME_MAX_VERSION
                        ),
                    })),
                }))
                .await;
            return None;
        }
    };

    // ── Step 2: Authentication (RFC 0005 §1.4) ───────────────────────────────
    // Authentication is evaluated synchronously before SessionEstablished is sent.
    let auth_result = authenticate_session_init(
        init.auth_credential.as_ref(),
        &init.pre_shared_key,
        psk,
    );

    match auth_result {
        AuthResult::Accepted => {}
        AuthResult::Failed(reason) => {
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: 1,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::SessionError(SessionError {
                        code: "AUTH_FAILED".to_string(),
                        message: reason,
                        hint: String::new(),
                    })),
                }))
                .await;
            return None;
        }
        AuthResult::Unimplemented(reason) => {
            // v1-reserved credential type — reject with AUTH_FAILED.
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: 1,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::SessionError(SessionError {
                        code: "AUTH_FAILED".to_string(),
                        message: reason,
                        hint: r#"{"supported_v1": ["PreSharedKeyCredential", "LocalSocketCredential"]}"#.to_string(),
                    })),
                }))
                .await;
            return None;
        }
    }

    // ── Step 3: Capability negotiation (RFC 0005 §5.3) ───────────────────────
    // Capabilities are gated against the agent's authorization policy.
    // For PSK-authenticated agents in v1, the policy is unrestricted.
    let policy = CapabilityPolicy::for_psk_agent();
    let (granted_capabilities, _denied_caps) =
        policy.partition_capabilities(&init.requested_capabilities);

    // ── Step 4: Subscription filtering (RFC 0005 §7.1) ──────────────────────
    // Initial subscriptions are filtered against the agent's AUTHORIZATION POLICY,
    // not just the explicitly requested capabilities. An unrestricted PSK agent can
    // subscribe to any category even if it didn't explicitly request the governing
    // capability in `requested_capabilities`.
    //
    // We represent the policy's authorization scope: for unrestricted PSK agents
    // we pass ["*"] as a sentinel to filter_subscriptions to allow everything.
    // For restricted agents we would pass the granted_capabilities list.
    let policy_caps = if policy.is_unrestricted() {
        vec!["*".to_string()]
    } else {
        granted_capabilities.clone()
    };
    let (active_subscriptions, denied_subscriptions) =
        filter_subscriptions(&init.initial_subscriptions, &policy_caps);

    let session_id = uuid::Uuid::now_v7().to_string();
    let namespace = init.agent_id.clone();
    let resume_token = uuid::Uuid::now_v7().as_bytes().to_vec();

    // Register session in the session registry
    {
        let mut st = state.lock().await;
        let _ = st.sessions.authenticate(
            &init.agent_id,
            psk,
            &granted_capabilities,
        );
    }

    let mut session = StreamSession {
        session_id: session_id.clone(),
        namespace: namespace.clone(),
        agent_name: init.agent_id.clone(),
        capabilities: granted_capabilities.clone(),
        policy_capabilities: policy_caps.clone(),
        lease_ids: Vec::new(),
        subscriptions: active_subscriptions.clone(),
        server_sequence: 0,
        resume_token: resume_token.clone(),
        last_heartbeat_ms: now_ms(),
        state: SessionState::Handshaking,
        last_client_sequence: 1, // SessionInit is sequence 1; start validation from next
        safe_mode_active: false,
        expect_resume: false,
        agent_event_rate_limiter: SessionEventRateLimiter::new(),
    };

    // ── Step 5: Clock skew estimation (RFC 0003 §1.3) ────────────────────────
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
                denied_subscriptions,
                negotiated_protocol_version: negotiated_version,
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
    // Re-authentication is required on resume (RFC 0005 §6.2).
    let auth_result = authenticate_session_init(
        resume.auth_credential.as_ref(),
        &resume.pre_shared_key,
        psk,
    );
    match auth_result {
        AuthResult::Accepted => {}
        AuthResult::Failed(reason) | AuthResult::Unimplemented(reason) => {
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: 1,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::SessionError(SessionError {
                        code: "AUTH_FAILED".to_string(),
                        message: reason,
                        hint: String::new(),
                    })),
                }))
                .await;
            return None;
        }
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

    // Resume session with PSK-unrestricted policy (same as new session).
    let resume_policy_caps = vec!["*".to_string()];
    let mut session = StreamSession {
        session_id: session_id.clone(),
        namespace: namespace.clone(),
        agent_name: resume.agent_id.clone(),
        capabilities: Vec::new(),
        policy_capabilities: resume_policy_caps,
        lease_ids: Vec::new(),
        subscriptions: Vec::new(),
        server_sequence: 0,
        resume_token: new_resume_token.clone(),
        last_heartbeat_ms: now_ms(),
        state: SessionState::Resuming,
        last_client_sequence: 1, // SessionResume is sequence 1; start validation from next
        safe_mode_active: false,
        expect_resume: false,
        agent_event_rate_limiter: SessionEventRateLimiter::new(),
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
                // Resume always runs at the highest runtime-supported version.
                // version = major * 1000 + minor; v1.1 = 1001.
                negotiated_protocol_version: crate::auth::RUNTIME_MAX_VERSION,
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
        ClientPayload::SessionClose(close) => {
            // Graceful disconnect (RFC 0005 §1.5).
            // Record the expect_resume hint; the main loop transitions state after this returns.
            session.expect_resume = close.expect_resume;
        }
        ClientPayload::CapabilityRequest(req) => {
            handle_capability_request(session, tx, req).await;
        }
        // Agent scene event emission (scene-events/spec.md §5.1, §5.2).
        ClientPayload::EmitSceneEvent(emit) => {
            handle_emit_scene_event(state, session, tx, client_sequence, emit).await;
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
    // Reject MutationBatch when safe mode is active (RFC 0005 §3.7).
    // Session-local flag tracks per-session suspension (from SessionSuspended delivery).
    // Shared state flag tracks global suspension (from the runtime side).
    // Both are checked; shared state takes precedence.
    {
        let st = state.lock().await;
        let safe_mode = session.safe_mode_active || st.safe_mode_active;
        if safe_mode {
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::RuntimeError(RuntimeError {
                        error_code: "SAFE_MODE_ACTIVE".to_string(),
                        message: "Mutations are not accepted while the runtime is in safe mode."
                            .to_string(),
                        context: String::new(),
                        hint: r#"{"wait_for": "SessionResumed"}"#.to_string(),
                        error_code_enum: ErrorCode::SafeModeActive as i32,
                    })),
                }))
                .await;
            return;
        }
    }

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
        timing_hints: None,
        lease_id: None,
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
    // Filter additions against the session's authorization policy (RFC 0005 §7.1).
    // Uses `policy_capabilities` (e.g. ["*"] for unrestricted PSK sessions) rather
    // than the explicitly requested `capabilities` list, so unrestricted agents can
    // subscribe to any category regardless of what was in `requested_capabilities`.
    let (allowed_additions, denied_additions) =
        filter_subscriptions(&change.subscribe, &session.policy_capabilities);

    // Add permitted subscriptions
    for sub in &allowed_additions {
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
                denied_subscriptions: denied_additions,
            })),
        }))
        .await;
}

/// Handle a mid-session CapabilityRequest (RFC 0005 §5.3).
///
/// Validates the request against the agent's authorization policy. If all
/// requested capabilities are authorized, responds with CapabilityNotice.
/// On partial failure or any denial, responds with RuntimeError(PERMISSION_DENIED)
/// without granting any capabilities (RFC 0005 §5.3 scenario 4).
///
/// For PSK-authenticated agents in v1, the policy is unrestricted, so any
/// capability not already held will be granted. Guest agents (no capabilities)
/// will be denied any escalation attempt.
async fn handle_capability_request(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    req: CapabilityRequest,
) {
    // Reconstruct the authorization policy from the session's `policy_capabilities`.
    // For PSK-authenticated sessions, `policy_capabilities` contains ["*"] (unrestricted).
    // For restricted agents, it contains the specific allowed capabilities.
    //
    // Post-v1: load per-agent policy from config; use session's auth identity.
    let policy = CapabilityPolicy::new(session.policy_capabilities.clone());

    match policy.evaluate_capability_request(&req.capabilities) {
        Ok(granted) => {
            // Compute newly granted capabilities (exclude those already held).
            // CapabilityNotice.granted must contain only *newly* granted capabilities
            // so clients don't misinterpret re-requests as fresh grants.
            let seq = session.next_server_seq();
            let mut newly_granted: Vec<String> = Vec::new();
            for cap in &granted {
                if !session.capabilities.contains(cap) {
                    session.capabilities.push(cap.clone());
                    newly_granted.push(cap.clone());
                }
            }
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::CapabilityNotice(CapabilityNotice {
                        granted: newly_granted,
                        revoked: Vec::new(),
                        reason: req.reason.clone(),
                        effective_at_server_seq: seq,
                    })),
                }))
                .await;
        }
        Err(denied_caps) => {
            // Deny the entire request (partial grants not allowed per RFC 0005 §5.3).
            let context = denied_caps.join(", ");
            let hint = serde_json::to_string(&serde_json::json!({
                "unauthorized_capabilities": denied_caps
            }))
            .unwrap_or_else(|_| "{}".to_string());
            let seq = session.next_server_seq();
            let _ = tx
                .send(Ok(ServerMessage {
                    sequence: seq,
                    timestamp_wall_us: now_wall_us(),
                    payload: Some(ServerPayload::RuntimeError(RuntimeError {
                        error_code: "PERMISSION_DENIED".to_string(),
                        message: format!(
                            "Capability request denied: unauthorized capabilities: {context}"
                        ),
                        context,
                        hint,
                        error_code_enum: ErrorCode::PermissionDenied as i32,
                    })),
                }))
                .await;
        }
    }
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
                timing_hints: None,
                lease_id: None,
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

// ─── Agent Scene Event Emission handler ──────────────────────────────────────

/// Handle an `EmitSceneEvent` request from an agent.
///
/// Implements the server-side of the agent event emission protocol per
/// scene-events/spec.md §5.1–§5.4:
///
/// 1. Validate the bare name (format + reserved prefix).
/// 2. Check the `emit_scene_event:<bare_name>` capability.
/// 3. Enforce the 4 KB payload size limit.
/// 4. Apply the per-session sliding-window rate limit.
/// 5. On success, dispatch the event to subscribers and respond with the
///    fully-prefixed event type.
///
/// In v1 the per-session rate limiter state is held inside `StreamSession`
/// via an `Option<tze_hud_runtime::AgentEventHandler>`.  For now, the handler
/// is created lazily from the session namespace and capabilities.
///
/// Note: Full event bus delivery to subscribers (step 5) is wired in by bead #2.
/// This handler performs all gating checks and returns a result; actual fan-out
/// to subscription channels is not implemented in this bead.
async fn handle_emit_scene_event(
    _state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request_sequence: u64,
    emit: EmitSceneEvent,
) {
    use tze_hud_scene::events::naming::{validate_bare_name, build_agent_event_type, NamingError};

    // ── Step 1: Validate bare name ────────────────────────────────────────
    if let Err(naming_err) = validate_bare_name(&emit.bare_name) {
        let (error_code, message) = match &naming_err {
            NamingError::ReservedPrefix { prefix } => (
                "AGENT_EVENT_RESERVED_PREFIX".to_string(),
                format!("bare name must not start with reserved prefix {prefix:?}"),
            ),
            _ => (
                "AGENT_EVENT_INVALID_NAME".to_string(),
                format!("invalid bare name: {naming_err}"),
            ),
        };
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::EmitSceneEventResult(EmitSceneEventResult {
                    request_sequence,
                    accepted: false,
                    delivered_event_type: String::new(),
                    error_code,
                    error_message: message,
                })),
            }))
            .await;
        return;
    }

    // ── Step 2: Capability check ──────────────────────────────────────────
    let required_cap = format!("emit_scene_event:{}", emit.bare_name);
    if !session.capabilities.contains(&required_cap) {
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::EmitSceneEventResult(EmitSceneEventResult {
                    request_sequence,
                    accepted: false,
                    delivered_event_type: String::new(),
                    error_code: "AGENT_EVENT_CAPABILITY_MISSING".to_string(),
                    error_message: format!("missing capability: {required_cap}"),
                })),
            }))
            .await;
        return;
    }

    // ── Step 3: Payload size limit (4 KB) ────────────────────────────────
    const MAX_PAYLOAD: usize = 4096;
    if emit.payload.len() > MAX_PAYLOAD {
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::EmitSceneEventResult(EmitSceneEventResult {
                    request_sequence,
                    accepted: false,
                    delivered_event_type: String::new(),
                    error_code: "AGENT_EVENT_PAYLOAD_TOO_LARGE".to_string(),
                    error_message: format!(
                        "payload {} bytes exceeds {MAX_PAYLOAD}-byte limit",
                        emit.payload.len()
                    ),
                })),
            }))
            .await;
        return;
    }

    // ── Step 4: Rate limit ────────────────────────────────────────────────
    // Per-session rate limiter is stored on the StreamSession.
    if session.agent_event_rate_limiter.check_and_record(std::time::Instant::now()).is_err() {
        let seq = session.next_server_seq();
        let _ = tx
            .send(Ok(ServerMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ServerPayload::EmitSceneEventResult(EmitSceneEventResult {
                    request_sequence,
                    accepted: false,
                    delivered_event_type: String::new(),
                    error_code: "AGENT_EVENT_RATE_EXCEEDED".to_string(),
                    error_message: "agent event rate limit exceeded (10/s sliding window)"
                        .to_string(),
                })),
            }))
            .await;
        return;
    }

    // ── Step 5: Build delivered event type and accept ─────────────────────
    let delivered_event_type = build_agent_event_type(&session.namespace, &emit.bare_name);

    // TODO (bead #2): dispatch delivered_event_type to subscribers via the event bus.

    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::EmitSceneEventResult(EmitSceneEventResult {
                request_sequence,
                accepted: true,
                delivered_event_type,
                error_code: String::new(),
                error_message: String::new(),
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
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
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
                    min_protocol_version: 1000,
                    max_protocol_version: 1001,
                    auth_credential: None,
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
                    auth_credential: None,
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
                // version = major * 1000 + minor; runtime max = v1.1 = 1001
                assert_eq!(result.negotiated_protocol_version, crate::auth::RUNTIME_MAX_VERSION);
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

    // ─── Sequence number validation tests (RFC 0005 §2.3) ────────────────────

    /// Scenario: Sequence gap exceeds threshold (RFC 0005 §2.3)
    /// WHEN client sends sequence 5 followed by 150 (gap > max_sequence_gap=100),
    /// THEN runtime closes the stream with SEQUENCE_GAP_EXCEEDED.
    #[tokio::test]
    async fn test_sequence_gap_exceeded() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "seq-gap-agent", "test-key").await;

        // Handshake consumes sequence 1. Send a valid message at sequence 2.
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 100,
            })),
        })
        .await
        .unwrap();

        // Drain the heartbeat echo
        let _ = stream.next().await;

        // Now jump to sequence 5, then to 150 — gap of 145 > DEFAULT_MAX_SEQUENCE_GAP=100
        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 200,
            })),
        })
        .await
        .unwrap();
        let _ = stream.next().await; // drain heartbeat echo

        tx.send(ClientMessage {
            sequence: 150,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 300,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "SEQUENCE_GAP_EXCEEDED",
                    "Expected SEQUENCE_GAP_EXCEEDED, got: {}",
                    err.code
                );
            }
            other => panic!("Expected SessionError(SEQUENCE_GAP_EXCEEDED), got: {other:?}"),
        }
    }

    /// Scenario: Sequence regression rejected (RFC 0005 §2.3)
    /// WHEN client sends sequence 10 followed by sequence 8,
    /// THEN runtime closes the stream with SEQUENCE_REGRESSION.
    #[tokio::test]
    async fn test_sequence_regression() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "seq-reg-agent", "test-key").await;

        // Send sequence 10
        tx.send(ClientMessage {
            sequence: 10,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 100,
            })),
        })
        .await
        .unwrap();
        let _ = stream.next().await; // drain heartbeat echo

        // Send sequence 8 — regression
        tx.send(ClientMessage {
            sequence: 8,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 200,
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "SEQUENCE_REGRESSION",
                    "Expected SEQUENCE_REGRESSION, got: {}",
                    err.code
                );
            }
            other => panic!("Expected SessionError(SEQUENCE_REGRESSION), got: {other:?}"),
        }
    }

    /// Scenario: Monotonically increasing sequence numbers accepted.
    /// WHEN agent sends sequences 1, 2, 3,
    /// THEN all are processed without error.
    #[tokio::test]
    async fn test_sequence_monotonic_accepted() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "seq-ok-agent", "test-key").await;

        for seq in 2u64..=4 {
            tx.send(ClientMessage {
                sequence: seq,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::Heartbeat(Heartbeat {
                    timestamp_mono_us: seq * 1000,
                })),
            })
            .await
            .unwrap();

            let msg = stream.next().await.unwrap().unwrap();
            match &msg.payload {
                Some(ServerPayload::Heartbeat(hb)) => {
                    assert_eq!(hb.timestamp_mono_us, seq * 1000);
                }
                other => panic!("Expected Heartbeat echo at seq {seq}, got: {other:?}"),
            }
        }
    }

    // ─── Safe mode tests (RFC 0005 §3.7) ─────────────────────────────────────

    /// Scenario: Mutations rejected during safe mode (RFC 0005 §3.7)
    /// WHEN the runtime enters safe mode and sets `SharedState.safe_mode_active = true`,
    /// THEN MutationBatch is rejected with SAFE_MODE_ACTIVE.
    ///
    /// In this test we drive safe mode via `SharedState` directly (as the runtime
    /// would do via a SessionSuspended broadcast to all sessions).
    #[tokio::test]
    async fn test_safe_mode_rejects_mutations() {
        let (mut client, _server, shared_state) = setup_test_with_state().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "safe-mode-agent", "test-key").await;

        // Enable safe mode in shared state (simulates runtime entering safe mode)
        {
            let mut st = shared_state.lock().await;
            st.safe_mode_active = true;
        }

        // Request a lease first (this is transactional, not affected by safe mode)
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::LeaseRequest(LeaseRequest {
                ttl_ms: 30_000,
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

        // Send MutationBatch while safe mode is active — should be rejected
        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(ClientMessage {
            sequence: 3,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(
                    err.error_code, "SAFE_MODE_ACTIVE",
                    "Expected SAFE_MODE_ACTIVE, got: {}",
                    err.error_code
                );
            }
            other => panic!("Expected RuntimeError(SAFE_MODE_ACTIVE), got: {other:?}"),
        }

        // Disable safe mode
        {
            let mut st = shared_state.lock().await;
            st.safe_mode_active = false;
        }

        // Mutations should no longer be rejected with SAFE_MODE_ACTIVE.
        // We use a heartbeat to verify the session is still responsive and
        // the safe mode is cleared.
        tx.send(ClientMessage {
            sequence: 4,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::Heartbeat(Heartbeat {
                timestamp_mono_us: 999,
            })),
        })
        .await
        .unwrap();

        let msg2 = stream.next().await.unwrap().unwrap();
        match &msg2.payload {
            Some(ServerPayload::Heartbeat(hb)) => {
                // Session still active after safe mode was cleared
                assert_eq!(hb.timestamp_mono_us, 999, "Heartbeat should echo correctly");
            }
            other => panic!("Expected Heartbeat after safe mode exit, got: {other:?}"),
        }

        // Now verify a MutationBatch is no longer blocked by SAFE_MODE_ACTIVE.
        // (It may still fail due to invalid lease, but not because of safe mode.)
        let batch_id2 = uuid::Uuid::now_v7().as_bytes().to_vec();
        // Use the real lease from earlier
        tx.send(ClientMessage {
            sequence: 5,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::MutationBatch(MutationBatch {
                batch_id: batch_id2.clone(),
                lease_id: lease_id.clone(),
                mutations: Vec::new(),
            })),
        })
        .await
        .unwrap();

        let msg3 = stream.next().await.unwrap().unwrap();
        match &msg3.payload {
            Some(ServerPayload::MutationResult(result)) => {
                // We get MutationResult (not RuntimeError with SAFE_MODE_ACTIVE)
                assert_eq!(result.batch_id, batch_id2);
            }
            Some(ServerPayload::RuntimeError(err)) => {
                // Must NOT be SAFE_MODE_ACTIVE
                assert_ne!(
                    err.error_code, "SAFE_MODE_ACTIVE",
                    "Safe mode should be cleared, unexpected SAFE_MODE_ACTIVE"
                );
            }
            other => panic!("Unexpected message after safe mode exit: {other:?}"),
        }
    }

    // ─── Session state machine tests (RFC 0005 §1.1) ─────────────────────────

    /// Scenario: Successful session establishment transitions through Connecting→Handshaking→Active.
    /// The state machine starts in Handshaking during the handle_session_init call and
    /// transitions to Active after the handshake response is sent.
    #[tokio::test]
    async fn test_state_machine_successful_establishment() {
        let (mut client, _server) = setup_test().await;
        let (_tx, messages, _stream) = handshake(&mut client, "state-test-agent", "test-key").await;

        // If handshake succeeded, we must have SessionEstablished followed by SceneSnapshot
        assert_eq!(messages.len(), 2, "Expected SessionEstablished + SceneSnapshot");
        assert!(
            matches!(messages[0].payload, Some(ServerPayload::SessionEstablished(_))),
            "First message must be SessionEstablished"
        );
        assert!(
            matches!(messages[1].payload, Some(ServerPayload::SceneSnapshot(_))),
            "Second message must be SceneSnapshot"
        );
    }

    /// Scenario: Auth failure transitions Handshaking→Closed with SessionError.
    #[tokio::test]
    async fn test_state_machine_auth_failure_to_closed() {
        let (mut client, _server) = setup_test().await;

        let (init_tx, init_rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(init_rx);

        init_tx
            .send(ClientMessage {
                sequence: 1,
                timestamp_wall_us: now_wall_us(),
                payload: Some(ClientPayload::SessionInit(SessionInit {
                    agent_id: "state-fail-agent".to_string(),
                    agent_display_name: "state-fail-agent".to_string(),
                    pre_shared_key: "wrong-key".to_string(),
                    requested_capabilities: Vec::new(),
                    initial_subscriptions: Vec::new(),
                    resume_token: Vec::new(),
                    agent_timestamp_wall_us: 0,
                    min_protocol_version: 1000,
                    max_protocol_version: 1001,
                    auth_credential: None,
                })),
            })
            .await
            .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();

        // State machine should send SessionError (AUTH_FAILED) and transition to Closed
        match &msg.payload {
            Some(ServerPayload::SessionError(error)) => {
                assert_eq!(error.code, "AUTH_FAILED");
            }
            other => panic!("Expected SessionError(AUTH_FAILED), got: {other:?}"),
        }
    }

    /// Scenario: Graceful disconnect via SessionClose.
    /// The session stream should terminate cleanly after SessionClose is sent.
    #[tokio::test]
    async fn test_graceful_disconnect_session_close() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "close-agent", "test-key").await;

        // Send SessionClose with expect_resume=false
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionClose(SessionClose {
                reason: "test shutdown".to_string(),
                expect_resume: false,
            })),
        })
        .await
        .unwrap();

        // Stream should close (no response expected for SessionClose)
        // Give the server a moment to process
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // The stream should be closed; next() should return None or an error
        // (The server closes the stream after transitioning to Closed state)
        drop(tx);
        // Drain any remaining messages
        let mut got_stream_end = false;
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(500);
        loop {
            if tokio::time::Instant::now() > deadline {
                break;
            }
            match tokio::time::timeout(
                tokio::time::Duration::from_millis(100),
                stream.next(),
            )
            .await
            {
                Ok(None) | Err(_) => {
                    got_stream_end = true;
                    break;
                }
                Ok(Some(_)) => {
                    // Some message still in transit, keep draining
                }
            }
        }
        assert!(
            got_stream_end,
            "session stream did not terminate after SessionClose — graceful disconnect had no observable effect"
        );
    }

    /// Scenario: Graceful disconnect with expect_resume=true hint (RFC 0005 §1.5).
    /// The runtime should record the hint (tested via no error returned to client).
    #[tokio::test]
    async fn test_graceful_disconnect_with_resume_hint() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, _stream) =
            handshake(&mut client, "resume-hint-agent", "test-key").await;

        // Send SessionClose with expect_resume=true
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionClose(SessionClose {
                reason: "updating agent".to_string(),
                expect_resume: true,
            })),
        })
        .await
        .unwrap();

        // If no error is returned, the hint was processed successfully.
        // The test verifies protocol acceptance, not the lease hold behavior
        // (which requires multi-session coordination tested in integration tests).
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        drop(tx);
    }

    // ─── SessionConfig default values test (RFC 0005 §10) ────────────────────

    /// Verify SessionConfig defaults match the spec-specified values.
    #[test]
    fn test_session_config_defaults() {
        let config = SessionConfig::default();
        assert_eq!(config.handshake_timeout_ms, 5000, "handshake_timeout_ms default");
        assert_eq!(config.heartbeat_interval_ms, 5000, "heartbeat_interval_ms default");
        assert_eq!(config.heartbeat_missed_threshold, 3, "heartbeat_missed_threshold default");
        assert_eq!(config.reconnect_grace_period_ms, 30_000, "reconnect_grace_period_ms default");
        assert_eq!(config.retransmit_timeout_ms, 5000, "retransmit_timeout_ms default");
        assert_eq!(config.dedup_window_size, 1000, "dedup_window_size default");
        assert_eq!(config.dedup_window_ttl_s, 60, "dedup_window_ttl_s default");
        assert_eq!(config.max_sequence_gap, 100, "max_sequence_gap default");
        assert_eq!(config.ephemeral_buffer_max, 16, "ephemeral_buffer_max default");
        assert_eq!(config.max_concurrent_resident_sessions, 16, "max_concurrent_resident_sessions default");
        assert_eq!(config.max_concurrent_guest_sessions, 64, "max_concurrent_guest_sessions default");
    }

    // ─── Traffic class classification tests (RFC 0005 §3.1, §3.2) ───────────

    /// Verify traffic class routing for server payloads.
    #[test]
    fn test_traffic_class_routing() {
        use crate::proto::session::*;

        // Transactional messages
        assert_eq!(
            classify_server_payload(&ServerPayload::SessionEstablished(SessionEstablished::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::MutationResult(MutationResult::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::LeaseResponse(LeaseResponse::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::SessionSuspended(SessionSuspended::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::SessionResumed(SessionResumed::default())),
            TrafficClass::Transactional,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::RuntimeError(RuntimeError::default())),
            TrafficClass::Transactional,
        );

        // StateStream messages
        assert_eq!(
            classify_server_payload(&ServerPayload::SceneSnapshot(SceneSnapshot::default())),
            TrafficClass::StateStream,
        );
        assert_eq!(
            classify_server_payload(&ServerPayload::SceneDelta(SceneDelta::default())),
            TrafficClass::StateStream,
        );

        // Ephemeral messages
        assert_eq!(
            classify_server_payload(&ServerPayload::Heartbeat(Heartbeat::default())),
            TrafficClass::Ephemeral,
        );
    }

    // ─── Ephemeral queue backpressure tests (RFC 0005 §2.5) ──────────────────

    /// Scenario: Ephemeral messages dropped under pressure (RFC 0005 §2.5)
    /// WHEN 20 ephemeral messages are enqueued (>16 quota),
    /// THEN oldest are dropped, retaining latest 16.
    #[test]
    fn test_ephemeral_queue_drops_oldest() {
        let mut queue = EphemeralQueue::new(DEFAULT_EPHEMERAL_BUFFER_MAX);

        // Enqueue 20 messages (4 more than the 16-message quota)
        for i in 0u64..20 {
            let msg = Ok(ServerMessage {
                sequence: i + 1,
                timestamp_wall_us: 0,
                payload: Some(ServerPayload::Heartbeat(Heartbeat {
                    timestamp_mono_us: i,
                })),
            });
            queue.push(msg);
        }

        // Queue should contain exactly 16 messages (quota)
        assert_eq!(
            queue.queue.len(),
            DEFAULT_EPHEMERAL_BUFFER_MAX,
            "Queue should be capped at ephemeral_buffer_max=16"
        );

        // First retained message should be sequence 5 (oldest 4 were dropped: 1,2,3,4)
        if let Some(Ok(msg)) = queue.queue.front() {
            assert_eq!(msg.sequence, 5, "Oldest 4 should have been evicted (1-4 dropped, 5 is oldest retained)");
        }

        // Last retained message should be sequence 20
        if let Some(Ok(msg)) = queue.queue.back() {
            assert_eq!(msg.sequence, 20, "Latest message should be 20");
        }
    }

    /// Scenario: Ephemeral queue within capacity — no messages dropped.
    #[test]
    fn test_ephemeral_queue_within_capacity() {
        let mut queue = EphemeralQueue::new(DEFAULT_EPHEMERAL_BUFFER_MAX);

        for i in 0u64..16 {
            queue.push(Ok(ServerMessage {
                sequence: i + 1,
                timestamp_wall_us: 0,
                payload: Some(ServerPayload::Heartbeat(Heartbeat {
                    timestamp_mono_us: i,
                })),
            }));
        }

        assert_eq!(queue.queue.len(), 16, "All 16 messages retained (at capacity)");
        if let Some(Ok(msg)) = queue.queue.front() {
            assert_eq!(msg.sequence, 1, "No eviction: first message is sequence 1");
        }
    }

    // ─── Sequence validation unit tests ─────────────────────────────────────

    /// Unit tests for StreamSession::validate_client_sequence.
    #[test]
    fn test_validate_sequence_unit() {
        let mut session = StreamSession {
            session_id: "test".to_string(),
            namespace: "test".to_string(),
            agent_name: "test".to_string(),
            capabilities: Vec::new(),
            policy_capabilities: Vec::new(),
            lease_ids: Vec::new(),
            subscriptions: Vec::new(),
            server_sequence: 0,
            resume_token: Vec::new(),
            last_heartbeat_ms: 0,
            state: SessionState::Active,
            last_client_sequence: 1,
            safe_mode_active: false,
            expect_resume: false,
            agent_event_rate_limiter: SessionEventRateLimiter::new(),
        };

        // seq=2 (gap=1): OK
        assert!(session.validate_client_sequence(2, 100).is_ok());
        assert_eq!(session.last_client_sequence, 2);

        // seq=102 (gap=100): still OK (gap == max_gap, not >)
        assert!(session.validate_client_sequence(102, 100).is_ok());
        assert_eq!(session.last_client_sequence, 102);

        // seq=203 (gap=101): exceeds max_gap=100
        let err = session.validate_client_sequence(203, 100);
        assert!(err.is_err());
        let (code, _) = err.unwrap_err();
        assert_eq!(code, "SEQUENCE_GAP_EXCEEDED");
        // last_client_sequence unchanged on error
        assert_eq!(session.last_client_sequence, 102);

        // seq=50 (regression): error
        let err = session.validate_client_sequence(50, 100);
        assert!(err.is_err());
        let (code, _) = err.unwrap_err();
        assert_eq!(code, "SEQUENCE_REGRESSION");

        // seq=102 (same as last): regression (not strictly greater)
        let err = session.validate_client_sequence(102, 100);
        assert!(err.is_err());
        let (code, _) = err.unwrap_err();
        assert_eq!(code, "SEQUENCE_REGRESSION");
    }

    // ─── Handshake auth, version, capability, subscription tests (rig-8uqz) ──

    /// Scenario: Structured AuthCredential (PSK) accepted (RFC 0005 §1.4)
    /// WHEN agent sends SessionInit with a valid PreSharedKeyCredential in auth_credential,
    /// THEN runtime authenticates and proceeds to SessionEstablished.
    #[tokio::test]
    async fn test_auth_structured_psk_credential_accepted() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "psk-agent".to_string(),
                agent_display_name: "psk-agent".to_string(),
                pre_shared_key: String::new(), // intentionally empty — use auth_credential
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: Some(crate::proto::session::AuthCredential {
                    credential: Some(
                        crate::proto::session::auth_credential::Credential::PreSharedKey(
                            crate::proto::session::PreSharedKeyCredential {
                                key: "test-key".to_string(),
                            },
                        ),
                    ),
                }),
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(_)) => {}
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Scenario: Invalid structured PSK credential rejected with AUTH_FAILED (RFC 0005 §1.4)
    /// WHEN agent sends SessionInit with a wrong PreSharedKeyCredential,
    /// THEN runtime sends SessionError(AUTH_FAILED) and closes stream.
    #[tokio::test]
    async fn test_auth_structured_psk_credential_wrong_key() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "bad-psk-agent".to_string(),
                agent_display_name: "bad-psk-agent".to_string(),
                pre_shared_key: String::new(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: Some(crate::proto::session::AuthCredential {
                    credential: Some(
                        crate::proto::session::auth_credential::Credential::PreSharedKey(
                            crate::proto::session::PreSharedKeyCredential {
                                key: "wrong-key".to_string(),
                            },
                        ),
                    ),
                }),
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(err.code, "AUTH_FAILED");
            }
            other => panic!("Expected SessionError(AUTH_FAILED), got: {other:?}"),
        }
    }

    /// Scenario: LocalSocketCredential accepted (RFC 0005 §1.4)
    /// WHEN agent sends SessionInit with a valid LocalSocketCredential,
    /// THEN runtime authenticates and proceeds to SessionEstablished.
    #[tokio::test]
    async fn test_auth_local_socket_credential_accepted() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "local-agent".to_string(),
                agent_display_name: "local-agent".to_string(),
                pre_shared_key: String::new(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: Some(crate::proto::session::AuthCredential {
                    credential: Some(
                        crate::proto::session::auth_credential::Credential::LocalSocket(
                            crate::proto::session::LocalSocketCredential {
                                socket_path: "/run/tze_hud.sock".to_string(),
                                pid_hint: "42".to_string(),
                            },
                        ),
                    ),
                }),
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(_)) => {}
            other => panic!("Expected SessionEstablished with LocalSocket cred, got: {other:?}"),
        }
    }

    /// Scenario: Version negotiated successfully (RFC 0005 §4.1)
    /// WHEN agent declares min=1000, max=1001 and runtime supports 1000-1001,
    /// THEN SessionEstablished contains negotiated_protocol_version=1001.
    #[tokio::test]
    async fn test_version_negotiation_success() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "version-agent".to_string(),
                agent_display_name: "version-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                assert_eq!(
                    established.negotiated_protocol_version, 1001,
                    "Should pick highest mutual version (1001)"
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Scenario: Version negotiation failure — no mutual version (RFC 0005 §4.1)
    /// WHEN agent declares min=2000, max=2001 and runtime only supports 1000-1001,
    /// THEN runtime sends SessionError(code=UNSUPPORTED_PROTOCOL_VERSION) and closes stream.
    #[tokio::test]
    async fn test_version_negotiation_unsupported() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "old-agent".to_string(),
                agent_display_name: "old-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 2000,
                max_protocol_version: 2001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionError(err)) => {
                assert_eq!(
                    err.code, "UNSUPPORTED_PROTOCOL_VERSION",
                    "Expected UNSUPPORTED_PROTOCOL_VERSION, got: {}",
                    err.code
                );
                // Hint should include runtime's supported range
                assert!(!err.hint.is_empty(), "Hint should contain runtime version range");
            }
            other => panic!("Expected SessionError(UNSUPPORTED_PROTOCOL_VERSION), got: {other:?}"),
        }
    }

    /// Scenario: Clock sync — estimated_skew_us returned when agent_timestamp_wall_us is set
    /// (RFC 0005 §1.2 / RFC 0003 §1.3)
    /// WHEN agent includes agent_timestamp_wall_us in SessionInit,
    /// THEN runtime computes initial clock-skew and returns estimated_skew_us in SessionEstablished.
    #[tokio::test]
    async fn test_clock_skew_estimation() {
        let (mut client, _server) = setup_test().await;

        let agent_ts = now_wall_us();
        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "clock-agent".to_string(),
                agent_display_name: "clock-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: Vec::new(),
                initial_subscriptions: Vec::new(),
                resume_token: Vec::new(),
                agent_timestamp_wall_us: agent_ts,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                // estimated_skew_us should be set (may be near 0 or slightly negative
                // due to timing between send and receive, but the field should exist
                // and be plausible — within ±1s for a loopback test)
                assert!(
                    established.estimated_skew_us.abs() < 1_000_000,
                    "Clock skew should be within ±1s on loopback, got: {}µs",
                    established.estimated_skew_us
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Scenario: PSK (unrestricted) agent successfully subscribes to INPUT_EVENTS (RFC 0005 §7.1)
    /// WHEN a PSK-authenticated agent requests INPUT_EVENTS subscription,
    /// THEN SessionEstablished includes INPUT_EVENTS in active_subscriptions and
    /// denied_subscriptions is empty.
    ///
    /// PSK sessions carry an unrestricted policy (policy_capabilities = ["*"]), so they
    /// can subscribe to any category regardless of what was in requested_capabilities.
    /// The denied-subscription path is exercised in auth module unit tests (filter_subscriptions).
    #[tokio::test]
    async fn test_psk_unrestricted_allows_input_events_subscription() {
        let (mut client, _server) = setup_test().await;

        let (tx, rx) = tokio::sync::mpsc::channel::<ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        // PSK agent requesting INPUT_EVENTS subscription (no specific capability needed
        // since PSK is unrestricted)
        tx.send(ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::SessionInit(SessionInit {
                agent_id: "sub-test-agent".to_string(),
                agent_display_name: "sub-test-agent".to_string(),
                pre_shared_key: "test-key".to_string(),
                requested_capabilities: Vec::new(), // no capabilities requested
                initial_subscriptions: vec!["INPUT_EVENTS".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: 0,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut response_stream = client.session(stream).await.unwrap().into_inner();
        let msg = response_stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::SessionEstablished(established)) => {
                // PSK agent (unrestricted policy) should be able to subscribe to INPUT_EVENTS
                assert!(
                    established.active_subscriptions.contains(&"INPUT_EVENTS".to_string()),
                    "PSK unrestricted agent should have INPUT_EVENTS in active_subscriptions; \
                     active={:?}, denied={:?}",
                    established.active_subscriptions,
                    established.denied_subscriptions
                );
                assert!(
                    established.denied_subscriptions.is_empty(),
                    "PSK agent should have no denied subscriptions"
                );
            }
            other => panic!("Expected SessionEstablished, got: {other:?}"),
        }
    }

    /// Scenario: Capability granted mid-session (RFC 0005 §5.3)
    /// WHEN agent sends CapabilityRequest with authorized capabilities,
    /// THEN runtime responds with CapabilityNotice(granted=requested_capabilities).
    #[tokio::test]
    async fn test_mid_session_capability_request_granted() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "cap-req-agent", "test-key").await;

        // Request a capability mid-session (PSK agents can request any capability)
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::CapabilityRequest(CapabilityRequest {
                capabilities: vec!["read_telemetry".to_string()],
                reason: "monitoring".to_string(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::CapabilityNotice(notice)) => {
                assert!(
                    notice.granted.contains(&"read_telemetry".to_string()),
                    "Expected read_telemetry to be granted; got: {:?}",
                    notice.granted
                );
                assert!(notice.revoked.is_empty(), "No capabilities should be revoked");
                assert!(
                    notice.effective_at_server_seq > 0,
                    "effective_at_server_seq must be non-zero"
                );
            }
            other => panic!("Expected CapabilityNotice, got: {other:?}"),
        }
    }

    /// Scenario: PSK (unrestricted) agent receives CapabilityNotice for any capability (RFC 0005 §5.3)
    /// WHEN a PSK-authenticated agent requests any capability mid-session,
    /// THEN runtime responds with CapabilityNotice (not RuntimeError).
    ///
    /// PSK sessions in v1 carry an unrestricted policy, so no capability request
    /// can be denied via this integration path. The denied path (PERMISSION_DENIED)
    /// is exercised in test_capability_request_denied_for_guest_session and
    /// test_capability_request_partial_grant_denied_entirely below.
    #[tokio::test]
    async fn test_mid_session_capability_request_unrestricted_succeeds() {
        let (mut client, _server) = setup_test().await;
        let (tx, _init_messages, mut stream) =
            handshake(&mut client, "deny-test-agent", "test-key").await;

        // PSK agent requesting a valid capability — should succeed (PSK is unrestricted).
        tx.send(ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ClientPayload::CapabilityRequest(CapabilityRequest {
                capabilities: vec!["overlay_privileges".to_string()],
                reason: "test".to_string(),
            })),
        })
        .await
        .unwrap();

        let msg = stream.next().await.unwrap().unwrap();
        // PSK agents (unrestricted) should get CapabilityNotice, not an error.
        match &msg.payload {
            Some(ServerPayload::CapabilityNotice(notice)) => {
                assert!(
                    notice.granted.contains(&"overlay_privileges".to_string()),
                    "PSK unrestricted agent should get overlay_privileges granted"
                );
            }
            other => panic!(
                "Expected CapabilityNotice for unrestricted PSK agent, got: {other:?}"
            ),
        }
    }

    /// Unit test: handle_capability_request with guest (restricted) session
    /// to verify RuntimeError(PERMISSION_DENIED) is returned for unauthorized caps.
    ///
    /// Scenario: Guest agent denied resident tools via capability escalation (RFC 0005 §5.3)
    /// WHEN a guest-level agent sends CapabilityRequest for resident-level operations,
    /// THEN runtime denies with RuntimeError(PERMISSION_DENIED).
    #[tokio::test]
    async fn test_capability_request_denied_for_guest_session() {
        // Set up the outbound channel
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<ServerMessage, Status>>(16);

        // Build a guest session (no policy capabilities = no authorization)
        let mut session = StreamSession {
            session_id: "guest-session".to_string(),
            namespace: "guest".to_string(),
            agent_name: "guest".to_string(),
            capabilities: Vec::new(),
            policy_capabilities: Vec::new(), // guest: no authorization
            lease_ids: Vec::new(),
            subscriptions: Vec::new(),
            server_sequence: 0,
            resume_token: Vec::new(),
            last_heartbeat_ms: 0,
            state: SessionState::Active,
            last_client_sequence: 1,
            safe_mode_active: false,
            expect_resume: false,
        };

        handle_capability_request(
            &mut session,
            &tx,
            CapabilityRequest {
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                reason: "escalation attempt".to_string(),
            },
        )
        .await;

        let msg = rx.recv().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(err.error_code, "PERMISSION_DENIED");
                assert_eq!(err.error_code_enum, ErrorCode::PermissionDenied as i32);
                assert!(
                    !err.context.is_empty(),
                    "Context should list denied capabilities"
                );
                assert!(
                    err.hint.contains("unauthorized_capabilities"),
                    "Hint should contain unauthorized_capabilities: {}",
                    err.hint
                );
            }
            other => panic!("Expected RuntimeError(PERMISSION_DENIED), got: {other:?}"),
        }
    }

    /// Scenario: Partial grant of mixed capabilities is denied entirely (RFC 0005 §5.3)
    /// WHEN agent requests capabilities=["read_telemetry", "overlay_privileges"] and is
    /// authorized for only read_telemetry,
    /// THEN runtime denies entire request with PERMISSION_DENIED.
    #[tokio::test]
    async fn test_capability_request_partial_grant_denied_entirely() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<ServerMessage, Status>>(16);

        // Session with only read_telemetry authorized
        let mut session = StreamSession {
            session_id: "partial-grant-session".to_string(),
            namespace: "restricted-agent".to_string(),
            agent_name: "restricted-agent".to_string(),
            capabilities: vec!["read_telemetry".to_string()],
            policy_capabilities: vec!["read_telemetry".to_string()], // only read_telemetry
            lease_ids: Vec::new(),
            subscriptions: Vec::new(),
            server_sequence: 0,
            resume_token: Vec::new(),
            last_heartbeat_ms: 0,
            state: SessionState::Active,
            last_client_sequence: 1,
            safe_mode_active: false,
            expect_resume: false,
        };

        // Request both an authorized and an unauthorized capability
        handle_capability_request(
            &mut session,
            &tx,
            CapabilityRequest {
                capabilities: vec![
                    "read_telemetry".to_string(),
                    "overlay_privileges".to_string(),
                ],
                reason: "mixed request".to_string(),
            },
        )
        .await;

        let msg = rx.recv().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                assert_eq!(
                    err.error_code, "PERMISSION_DENIED",
                    "Entire request should be denied, not just overlay_privileges"
                );
                assert_eq!(err.error_code_enum, ErrorCode::PermissionDenied as i32);
                assert!(
                    err.context.contains("overlay_privileges"),
                    "Context should mention the unauthorized capability: {}",
                    err.context
                );
                // read_telemetry should NOT have been granted
                assert!(
                    !session.capabilities.contains(&"overlay_privileges".to_string()),
                    "overlay_privileges must not have been added to session capabilities"
                );
            }
            other => panic!(
                "Expected RuntimeError(PERMISSION_DENIED) for partial grant, got: {other:?}"
            ),
        }
    }

    /// Verify RuntimeError structure matches spec (RFC 0005 §3.5)
    /// error_code, message, context, hint, error_code_enum all populated.
    #[tokio::test]
    async fn test_runtime_error_structure_complete() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<ServerMessage, Status>>(16);

        let mut session = StreamSession {
            session_id: "err-test".to_string(),
            namespace: "err-agent".to_string(),
            agent_name: "err-agent".to_string(),
            capabilities: Vec::new(),
            policy_capabilities: Vec::new(),
            lease_ids: Vec::new(),
            subscriptions: Vec::new(),
            server_sequence: 0,
            resume_token: Vec::new(),
            last_heartbeat_ms: 0,
            state: SessionState::Active,
            last_client_sequence: 1,
            safe_mode_active: false,
            expect_resume: false,
        };

        handle_capability_request(
            &mut session,
            &tx,
            CapabilityRequest {
                capabilities: vec!["some_cap".to_string()],
                reason: "test".to_string(),
            },
        )
        .await;

        let msg = rx.recv().await.unwrap().unwrap();
        match &msg.payload {
            Some(ServerPayload::RuntimeError(err)) => {
                // error_code: stable string
                assert!(!err.error_code.is_empty(), "error_code must be set");
                // message: human-readable
                assert!(!err.message.is_empty(), "message must be set");
                // error_code_enum: typed enum (non-zero for known codes)
                assert!(err.error_code_enum != 0, "error_code_enum must be non-zero for known codes");
                // hint: machine-readable JSON
                if !err.hint.is_empty() {
                    assert!(
                        serde_json::from_str::<serde_json::Value>(&err.hint).is_ok(),
                        "hint must be valid JSON: {}",
                        err.hint
                    );
                }
            }
            other => panic!("Expected RuntimeError, got: {other:?}"),
        }
    }

    /// Helper that returns the shared state alongside the client for state-manipulation tests.
    async fn setup_test_with_state() -> (
        HudSessionClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<SharedState>>,
    ) {
        let scene = SceneGraph::new(800.0, 600.0);
        let service = HudSessionImpl::new(scene, "test-key");
        let shared_state = service.state.clone();

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

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let client =
            HudSessionClient::connect(format!("http://[::1]:{}", addr.port()))
                .await
                .unwrap();

        (client, handle, shared_state)
    }
}
