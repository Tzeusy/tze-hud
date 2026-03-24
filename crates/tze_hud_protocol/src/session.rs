//! Agent session management — authentication, capabilities, session state.

use std::collections::HashMap;
use tokio::sync::mpsc;
use tze_hud_scene::SceneId;
use tze_hud_scene::graph::SceneGraph;

use crate::proto::SceneEvent;
use crate::token::TokenStore;

/// Shared state between the gRPC server and the compositor.
pub struct SharedState {
    pub scene: SceneGraph,
    pub sessions: SessionRegistry,
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
}

impl Clone for AgentSession {
    fn clone(&self) -> Self {
        // event_tx is not cloned — the channel is owned by the session record.
        Self {
            session_id: self.session_id.clone(),
            namespace: self.namespace.clone(),
            agent_name: self.agent_name.clone(),
            capabilities: self.capabilities.clone(),
            lease_ids: self.lease_ids.clone(),
            event_subscribed: self.event_subscribed,
            event_tx: None,
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
}
