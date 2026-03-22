//! Agent session management — authentication, capabilities, session state.

use std::collections::HashMap;
use tze_hud_scene::SceneId;

/// A connected agent session.
#[derive(Clone, Debug)]
pub struct AgentSession {
    pub session_id: String,
    pub namespace: String,
    pub agent_name: String,
    pub capabilities: Vec<String>,
    pub lease_ids: Vec<SceneId>,
    pub event_subscribed: bool,
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
}
