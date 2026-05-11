//! Agent session management — authentication, capabilities, session state.
//!
//! NOTE: This module uses `proto::SceneEvent` (from `events_legacy.proto`) in
//! channel types for backwards-compatibility. New event dispatch code should
//! migrate to `InputEnvelope` / `EventBatch` (RFC 0004). The `SceneEvent`
//! usage here is retained for compatibility until that migration completes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tonic::Status;
use tze_hud_resource::{ResourceStore, RuntimeWidgetStore};
use tze_hud_scene::SceneId;
use tze_hud_scene::element_store::ElementStore;
use tze_hud_scene::graph::SceneGraph;

use crate::proto::SceneEvent;
use crate::proto::session::ServerMessage;
use crate::token::TokenStore;

/// Runtime input-capture command sent from the gRPC session plane to the local
/// input processor owned by the compositor/window thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputCaptureCommand {
    Request {
        tile_id: SceneId,
        node_id: SceneId,
        device_id: u32,
        release_on_up: bool,
    },
    Release {
        device_id: u32,
    },
}

/// Current degradation level of the runtime (RFC 0005 §3.4).
///
/// Mirrors `DegradationLevel` from `session.proto` as a plain Rust enum so that
/// the compositor and other non-proto code can track the level without depending
/// on generated proto types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDegradationLevel {
    Normal = 1,
    CoalescingMore = 2,
    MediaQualityReduced = 3,
    StreamsReduced = 4,
    RenderingSimplified = 5,
    SheddingTiles = 6,
    AudioOnlyFallback = 7,
}

impl RuntimeDegradationLevel {
    /// Convert to the proto enum integer value.
    pub fn to_proto_i32(self) -> i32 {
        self as i32
    }
}

/// Stored runtime widget SVG asset metadata/content keyed by strong hash.
#[derive(Debug, Clone)]
pub struct WidgetAssetRecord {
    pub asset_handle: String,
    pub widget_type_id: String,
    pub svg_filename: String,
    pub owner_namespace: String,
    pub bytes: Vec<u8>,
}

/// In-memory runtime widget asset register/upload store.
#[derive(Debug, Clone)]
pub struct WidgetAssetStore {
    /// Content-addressed entries keyed by BLAKE3 hash bytes.
    pub by_hash: HashMap<[u8; 32], WidgetAssetRecord>,
    /// Aggregate stored bytes across all widget assets.
    pub total_bytes: u64,
    /// Aggregate stored bytes per publishing namespace.
    pub per_namespace_bytes: HashMap<String, u64>,
    /// Global store budget cap.
    pub max_total_bytes: u64,
    /// Per-namespace store budget cap.
    pub max_namespace_bytes: u64,
}

impl WidgetAssetStore {
    pub fn new_with_limits(max_total_bytes: u64, max_namespace_bytes: u64) -> Self {
        Self {
            by_hash: HashMap::new(),
            total_bytes: 0,
            per_namespace_bytes: HashMap::new(),
            max_total_bytes,
            max_namespace_bytes,
        }
    }
}

impl Default for WidgetAssetStore {
    fn default() -> Self {
        // Conservative in-memory limits for the v1 protocol layer.
        Self::new_with_limits(64 * 1024 * 1024, 16 * 1024 * 1024)
    }
}

/// Shared state between the gRPC server and the compositor.
///
/// # Scene coherence
///
/// `scene` is an `Arc<Mutex<SceneGraph>>` shared across both the gRPC session
/// server and the MCP server.  Callers that already hold the `SharedState`
/// mutex should acquire the inner scene lock by calling
/// `st.scene.lock().await`.  The compositor thread pre-clones the `Arc` at
/// startup and locks it independently (never holding the outer `SharedState`
/// lock while waiting for the inner lock) to avoid nested-lock priority
/// inversion.
pub struct SharedState {
    pub scene: Arc<Mutex<SceneGraph>>,
    pub sessions: SessionRegistry,
    /// Resident scene-resource upload store (RFC 0011 on HudSession stream).
    pub resource_store: ResourceStore,
    pub widget_asset_store: WidgetAssetStore,
    /// Durable runtime widget asset store (v1 scoped durability exception).
    /// When `None`, widget asset registration uses in-memory fallback semantics.
    pub runtime_widget_store: Option<RuntimeWidgetStore>,
    /// Persistent element identity store (zone/widget/tile Scene IDs).
    pub element_store: ElementStore,
    /// On-disk path for `element_store.toml`. When `None`, persistence is disabled.
    pub element_store_path: Option<PathBuf>,
    /// Whether the runtime is currently in safe mode (RFC 0005 §3.7).
    /// When true, all active sessions reject MutationBatch with SAFE_MODE_ACTIVE.
    pub safe_mode_active: bool,
    /// In-memory resume token store (RFC 0005 §6.1).
    /// Cleared on process restart; never persisted.
    pub token_store: TokenStore,
    /// Runtime-wide freeze state (system-shell/spec.md §Freeze Scene).
    ///
    /// The shell is the sole writer of this field. When `freeze_active` is
    /// `true`, mutation batches are queued (not rejected) until unfreeze.
    ///
    /// Per the invariant: `safe_mode_active = true` implies
    /// `freeze_active = false`. Safe mode entry cancels freeze
    /// and discards all per-session freeze queues.
    pub freeze_active: bool,
    /// Current degradation level (RFC 0005 §3.4).
    /// Default: Normal (no degradation).
    pub degradation_level: RuntimeDegradationLevel,
    /// Runtime-global Windows media ingress admission slot.
    ///
    /// The active exemplar slice admits at most one inbound media stream for
    /// the approved surface, regardless of how many sessions are connected.
    pub media_ingress_active: Option<MediaIngressSharedState>,
    /// Optional bridge for session-plane pointer capture requests. Windowed
    /// runtime installs this so gRPC InputCaptureRequest/InputCaptureRelease
    /// mutates the same InputProcessor used for OS pointer routing.
    pub input_capture_tx: Option<mpsc::UnboundedSender<InputCaptureCommand>>,
}

/// Runtime-global owner for the currently admitted Windows media ingress stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaIngressSharedState {
    pub publisher_namespace: String,
    pub stream_epoch: u64,
    pub zone_name: String,
    pub surface_id: SceneId,
}

/// Bounded per-session event channel capacity (events).
pub const SESSION_EVENT_CHANNEL_CAPACITY: usize = 256;

/// A connected agent session.
#[derive(Debug)]
pub struct AgentSession {
    pub session_id: String,
    pub namespace: String,
    pub agent_name: String,
    pub capabilities: Vec<String>,
    pub lease_ids: Vec<SceneId>,
    pub event_subscribed: bool,
    /// Sender half of the per-session event channel.
    /// Present once the agent calls SubscribeEvents; None before that.
    pub event_tx: Option<mpsc::Sender<SceneEvent>>,
    /// Sender half of the per-session ServerMessage channel.
    ///
    /// Used by the safe mode controller to deliver `SessionSuspended` and
    /// `SessionResumed` messages outside the normal event subscription path.
    /// Registered by the session handler when the stream is established.
    /// These messages are transactional (never dropped) — per RFC 0005 §3.1.
    pub server_message_tx: Option<mpsc::Sender<Result<ServerMessage, Status>>>,
}

impl Clone for AgentSession {
    fn clone(&self) -> Self {
        // event_tx and server_message_tx are not cloned — channels are owned by the session record.
        Self {
            session_id: self.session_id.clone(),
            namespace: self.namespace.clone(),
            agent_name: self.agent_name.clone(),
            capabilities: self.capabilities.clone(),
            lease_ids: self.lease_ids.clone(),
            event_subscribed: self.event_subscribed,
            event_tx: None,
            server_message_tx: None,
        }
    }
}

/// Session registry for connected agents.
pub struct SessionRegistry {
    sessions: HashMap<String, AgentSession>,
    /// Pre-shared key for authentication (hardcoded for vertical slice).
    psk: String,
}

impl SessionRegistry {
    pub fn new(psk: &str) -> Self {
        Self {
            sessions: HashMap::new(),
            psk: psk.to_string(),
        }
    }

    /// Authenticate an agent and create a session.
    pub fn authenticate(
        &mut self,
        agent_name: &str,
        key: &str,
        requested_caps: &[String],
    ) -> Result<AgentSession, String> {
        if key != self.psk {
            return Err("authentication failed: invalid pre-shared key".to_string());
        }

        let session_id = uuid::Uuid::now_v7().to_string();
        let namespace = agent_name.to_string();

        // For vertical slice, grant all requested capabilities
        let session = AgentSession {
            session_id: session_id.clone(),
            namespace: namespace.clone(),
            agent_name: agent_name.to_string(),
            capabilities: requested_caps.to_vec(),
            lease_ids: Vec::new(),
            event_subscribed: false,
            event_tx: None,
            server_message_tx: None,
        };

        self.sessions.insert(session_id, session.clone());
        Ok(session)
    }

    pub fn get_session(&self, session_id: &str) -> Option<&AgentSession> {
        self.sessions.get(session_id)
    }

    pub fn get_session_mut(&mut self, session_id: &str) -> Option<&mut AgentSession> {
        self.sessions.get_mut(session_id)
    }

    pub fn remove_session(&mut self, session_id: &str) -> Option<AgentSession> {
        self.sessions.remove(session_id)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Find the session that owns the given namespace (agent name).
    pub fn session_for_namespace(&self, namespace: &str) -> Option<&AgentSession> {
        self.sessions.values().find(|s| s.namespace == namespace)
    }

    /// Send a SceneEvent to the agent owning `namespace`.
    /// Returns `true` if the event was enqueued, `false` if the agent has no
    /// active subscription or the channel is full.
    pub fn dispatch_to_namespace(&self, namespace: &str, event: SceneEvent) -> bool {
        if let Some(session) = self.session_for_namespace(namespace) {
            if let Some(tx) = &session.event_tx {
                return tx.try_send(event).is_ok();
            }
        }
        false
    }

    /// Broadcast a SceneEvent to ALL subscribed sessions.
    /// Used for scene-wide events (e.g., tile lifecycle if needed by multiple agents).
    pub fn broadcast(&self, event: SceneEvent) {
        for session in self.sessions.values() {
            if let Some(tx) = &session.event_tx {
                let _ = tx.try_send(event.clone());
            }
        }
    }

    /// Broadcast a `ServerMessage` to all connected sessions via their direct server channels.
    ///
    /// Used by the safe mode controller to deliver `SessionSuspended` and `SessionResumed`
    /// to all active session streams.  These messages are transactional (RFC 0005 §3.1) and
    /// must not be dropped; if the channel is full the send will fail and the drop is logged
    /// as a warning (the session's backpressure signal path handles overflow recovery).
    ///
    /// Returns the count of sessions that received the message.
    pub fn broadcast_server_message(&self, msg: ServerMessage) -> usize {
        let mut sent = 0;
        for session in self.sessions.values() {
            if let Some(tx) = &session.server_message_tx {
                if let Err(e) = tx.try_send(Ok(msg.clone())) {
                    tracing::warn!(
                        session_id = %session.session_id,
                        "Failed to deliver transactional ServerMessage (channel full or closed); message dropped: {}",
                        e
                    );
                } else {
                    sent += 1;
                }
            }
        }
        sent
    }

    /// Register the `ServerMessage` sender for an existing session.
    ///
    /// Called by the session handler when a new session stream is established.
    /// Allows the safe mode controller to deliver out-of-band control messages
    /// (`SessionSuspended`, `SessionResumed`) to all active sessions.
    ///
    /// Returns `true` if the sender was registered successfully, `false` if the
    /// `session_id` is not found in the registry.
    pub fn register_server_message_tx(
        &mut self,
        session_id: &str,
        tx: mpsc::Sender<Result<ServerMessage, Status>>,
    ) -> bool {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.server_message_tx = Some(tx);
            true
        } else {
            false
        }
    }
}
