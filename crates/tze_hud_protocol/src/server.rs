//! gRPC server implementation for the SceneService.

use crate::convert;
use crate::proto::scene_service_server::SceneService;
use crate::proto::*;
use crate::session::{SessionRegistry, SESSION_EVENT_CHANNEL_CAPACITY};
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_scene::mutation::{MutationBatch as SceneMutationBatch, SceneMutation};

/// Shared state between the gRPC server and the compositor.
pub struct SharedState {
    pub scene: SceneGraph,
    pub sessions: SessionRegistry,
}

/// The gRPC service implementation.
pub struct SceneServiceImpl {
    pub state: Arc<Mutex<SharedState>>,
}

impl SceneServiceImpl {
    pub fn new(scene: SceneGraph, psk: &str) -> Self {
        Self {
            state: Arc::new(Mutex::new(SharedState {
                scene,
                sessions: SessionRegistry::new(psk),
            })),
        }
    }
}

// ─── Helper ────────────────────────────────────────────────────────────────

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ─── Service impl ──────────────────────────────────────────────────────────

#[tonic::async_trait]
impl SceneService for SceneServiceImpl {
    async fn authenticate(
        &self,
        request: Request<ConnectRequest>,
    ) -> Result<Response<ConnectResponse>, Status> {
        let req = request.into_inner();
        let mut state = self.state.lock().await;

        match state.sessions.authenticate(
            &req.agent_name,
            &req.pre_shared_key,
            &req.requested_capabilities,
        ) {
            Ok(session) => Ok(Response::new(ConnectResponse {
                session_id: session.session_id,
                namespace: session.namespace,
                granted_capabilities: session.capabilities,
                error: String::new(),
            })),
            Err(e) => Ok(Response::new(ConnectResponse {
                session_id: String::new(),
                namespace: String::new(),
                granted_capabilities: vec![],
                error: e,
            })),
        }
    }

    async fn acquire_lease(
        &self,
        request: Request<LeaseRequest>,
    ) -> Result<Response<LeaseResponse>, Status> {
        let req = request.into_inner();
        let mut state = self.state.lock().await;

        // Validate session
        let session = state
            .sessions
            .get_session(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;
        let namespace = session.namespace.clone();

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
        let lease_id = state.scene.grant_lease(&namespace, ttl, capabilities);

        // Track lease in session
        if let Some(session) = state.sessions.get_session_mut(&req.session_id) {
            session.lease_ids.push(lease_id);
        }

        // Notify agent of lease_granted
        let event = SceneEvent {
            timestamp_ms: now_ms(),
            event: Some(scene_event::Event::Lease(LeaseEvent {
                lease_id: lease_id.to_string(),
                namespace: namespace.clone(),
                kind: LeaseEventKind::LeaseGranted as i32,
                timestamp_ms: now_ms(),
            })),
        };
        state.sessions.dispatch_to_namespace(&namespace, event);

        Ok(Response::new(LeaseResponse {
            lease_id: lease_id.to_string(),
            granted_ttl_ms: ttl,
            error: String::new(),
        }))
    }

    async fn renew_lease(
        &self,
        request: Request<LeaseRenewRequest>,
    ) -> Result<Response<LeaseRenewResponse>, Status> {
        let req = request.into_inner();
        let mut state = self.state.lock().await;

        // Validate session
        let session = state
            .sessions
            .get_session(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;
        let namespace = session.namespace.clone();

        let lease_id = parse_scene_id(&req.lease_id)?;
        let ttl = if req.new_ttl_ms > 0 { req.new_ttl_ms } else { 60_000 };

        match state.scene.renew_lease(lease_id, ttl) {
            Ok(()) => {
                let event = SceneEvent {
                    timestamp_ms: now_ms(),
                    event: Some(scene_event::Event::Lease(LeaseEvent {
                        lease_id: lease_id.to_string(),
                        namespace: namespace.clone(),
                        kind: LeaseEventKind::LeaseRenewed as i32,
                        timestamp_ms: now_ms(),
                    })),
                };
                state.sessions.dispatch_to_namespace(&namespace, event);

                Ok(Response::new(LeaseRenewResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(LeaseRenewResponse {
                success: false,
                error: e.to_string(),
            })),
        }
    }

    async fn revoke_lease(
        &self,
        request: Request<LeaseRevokeRequest>,
    ) -> Result<Response<LeaseRevokeResponse>, Status> {
        let req = request.into_inner();
        let mut state = self.state.lock().await;

        let session = state
            .sessions
            .get_session(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;
        let namespace = session.namespace.clone();

        let lease_id = parse_scene_id(&req.lease_id)?;

        match state.scene.revoke_lease(lease_id) {
            Ok(()) => {
                let event = SceneEvent {
                    timestamp_ms: now_ms(),
                    event: Some(scene_event::Event::Lease(LeaseEvent {
                        lease_id: lease_id.to_string(),
                        namespace: namespace.clone(),
                        kind: LeaseEventKind::LeaseRevoked as i32,
                        timestamp_ms: now_ms(),
                    })),
                };
                state.sessions.dispatch_to_namespace(&namespace, event);

                Ok(Response::new(LeaseRevokeResponse {
                    success: true,
                    error: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(LeaseRevokeResponse {
                success: false,
                error: e.to_string(),
            })),
        }
    }

    async fn apply_mutations(
        &self,
        request: Request<MutationBatchRequest>,
    ) -> Result<Response<MutationBatchResponse>, Status> {
        let req = request.into_inner();
        let mut state = self.state.lock().await;

        // Validate session
        let session = state
            .sessions
            .get_session(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;
        let namespace = session.namespace.clone();

        let lease_id = parse_scene_id(&req.lease_id)?;

        // Find the tab (use active tab or first tab)
        let tab_id = state
            .scene
            .active_tab
            .ok_or_else(|| Status::failed_precondition("no active tab"))?;

        // Convert proto mutations to scene mutations
        let mut scene_mutations = Vec::new();
        for m in &req.mutations {
            match &m.mutation {
                Some(mutation_proto::Mutation::CreateTile(ct)) => {
                    let bounds = ct
                        .bounds
                        .as_ref()
                        .map(convert::proto_rect_to_scene)
                        .unwrap_or(tze_hud_scene::Rect::new(0.0, 0.0, 200.0, 150.0));
                    scene_mutations.push(SceneMutation::CreateTile {
                        tab_id,
                        namespace: namespace.clone(),
                        lease_id,
                        bounds,
                        z_order: ct.z_order,
                    });
                }
                Some(mutation_proto::Mutation::SetTileRoot(str_)) => {
                    let tile_id = parse_scene_id(&str_.tile_id)?;
                    if let Some(ref node_proto) = str_.node {
                        if let Some(node) = convert::proto_node_to_scene(node_proto) {
                            scene_mutations.push(SceneMutation::SetTileRoot { tile_id, node });
                        }
                    }
                }
                Some(mutation_proto::Mutation::PublishToZone(pz)) => {
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
                Some(mutation_proto::Mutation::ClearZone(cz)) => {
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
        let batch = SceneMutationBatch {
            batch_id: tze_hud_scene::SceneId::new(),
            agent_namespace: namespace.clone(),
            mutations: scene_mutations,
        };

        let result = state.scene.apply_batch(&batch);

        if result.applied {
            let ts = now_ms();
            // Dispatch tile_created events to the owning agent for each new tile.
            for id in &result.created_ids {
                let event = SceneEvent {
                    timestamp_ms: ts,
                    event: Some(scene_event::Event::TileCreated(TileCreatedEvent {
                        tile_id: id.to_string(),
                        namespace: namespace.clone(),
                        timestamp_ms: ts,
                    })),
                };
                state.sessions.dispatch_to_namespace(&namespace, event);
            }

            Ok(Response::new(MutationBatchResponse {
                success: true,
                created_ids: result.created_ids.iter().map(|id| id.to_string()).collect(),
                error: String::new(),
            }))
        } else {
            Ok(Response::new(MutationBatchResponse {
                success: false,
                created_ids: vec![],
                error: result
                    .error
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unknown error".to_string()),
            }))
        }
    }

    async fn query_scene(
        &self,
        request: Request<SceneQueryRequest>,
    ) -> Result<Response<SceneQueryResponse>, Status> {
        let req = request.into_inner();
        let state = self.state.lock().await;

        // Validate session
        let _session = state
            .sessions
            .get_session(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;

        let json = state
            .scene
            .snapshot_json()
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(SceneQueryResponse {
            scene_json: json,
            version: state.scene.version,
        }))
    }

    async fn query_zone_registry(
        &self,
        request: Request<ZoneRegistryRequest>,
    ) -> Result<Response<ZoneRegistryResponse>, Status> {
        let req = request.into_inner();
        let state = self.state.lock().await;

        let _session = state
            .sessions
            .get_session(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;

        let snapshot = state.scene.zone_registry.snapshot();
        let registry_proto = convert::zone_registry_snapshot_to_proto(&snapshot);

        Ok(Response::new(ZoneRegistryResponse {
            registry: Some(registry_proto),
            error: String::new(),
        }))
    }

    type SubscribeEventsStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<SceneEvent, Status>> + Send>>;

    /// Subscribe to per-session events.
    ///
    /// Creates a bounded mpsc channel and stores the sender in the session.
    /// All subsequent server-side dispatches (scene mutations, input events,
    /// lease lifecycle) use that channel to push events to this specific agent.
    async fn subscribe_events(
        &self,
        request: Request<EventSubscribeRequest>,
    ) -> Result<Response<Self::SubscribeEventsStream>, Status> {
        let req = request.into_inner();
        let mut state = self.state.lock().await;

        // Validate session
        let session = state
            .sessions
            .get_session_mut(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;

        let (tx, rx) = tokio::sync::mpsc::channel(SESSION_EVENT_CHANNEL_CAPACITY);
        session.event_tx = Some(tx);
        session.event_subscribed = true;

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mapped = tokio_stream::StreamExt::map(stream, Ok);

        Ok(Response::new(Box::pin(mapped)))
    }
}

// ─── Public dispatch helpers ───────────────────────────────────────────────

/// Dispatch an input event from the input pipeline to the agent that owns
/// the given namespace.  Called from tze_hud_input after hit-testing.
pub fn dispatch_input_event(
    state: &mut SharedState,
    namespace: &str,
    tile_id: tze_hud_scene::SceneId,
    node_id: tze_hud_scene::SceneId,
    interaction_id: &str,
    display_x: f32,
    display_y: f32,
    local_x: f32,
    local_y: f32,
    kind: InputEventKind,
) {
    let ts = now_ms();
    let event = SceneEvent {
        timestamp_ms: ts,
        event: Some(scene_event::Event::Input(InputEvent {
            tile_id: tile_id.to_string(),
            node_id: node_id.to_string(),
            interaction_id: interaction_id.to_string(),
            local_x,
            local_y,
            display_x,
            display_y,
            kind: kind as i32,
            timestamp_ms: ts,
        })),
    };
    state.sessions.dispatch_to_namespace(namespace, event);
}

fn parse_scene_id(s: &str) -> Result<tze_hud_scene::SceneId, Status> {
    uuid::Uuid::parse_str(s)
        .map(tze_hud_scene::SceneId::from_uuid)
        .map_err(|e| Status::invalid_argument(format!("invalid scene ID '{s}': {e}")))
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::*;
    use crate::session::SessionRegistry;
    use tze_hud_scene::graph::SceneGraph;

    fn make_state(psk: &str) -> SharedState {
        let scene = SceneGraph::new(1920.0, 1080.0);
        SharedState {
            scene,
            sessions: SessionRegistry::new(psk),
        }
    }

    /// Authenticate a test agent and return (session_id, namespace).
    fn authenticate(state: &mut SharedState, agent: &str, psk: &str) -> String {
        state
            .sessions
            .authenticate(agent, psk, &["receive_input".to_string()])
            .unwrap()
            .session_id
    }

    // ── Session event channel tests ────────────────────────────────────

    #[test]
    fn test_subscribe_creates_per_session_channel() {
        let mut state = make_state("key");
        let session_id = authenticate(&mut state, "agent-a", "key");

        // Simulate what subscribe_events does: install a channel in the session
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        state
            .sessions
            .get_session_mut(&session_id)
            .unwrap()
            .event_tx = Some(tx);

        // Dispatch an event via the session registry
        let dispatched = state.sessions.dispatch_to_namespace("agent-a", SceneEvent {
            timestamp_ms: 1,
            event: Some(scene_event::Event::TileCreated(TileCreatedEvent {
                tile_id: "tile-1".to_string(),
                namespace: "agent-a".to_string(),
                timestamp_ms: 1,
            })),
        });
        assert!(dispatched, "event should be enqueued");

        // Verify it's in the channel
        let received = rx.try_recv().expect("event should be in channel");
        assert_eq!(received.timestamp_ms, 1);
        match received.event {
            Some(scene_event::Event::TileCreated(e)) => assert_eq!(e.tile_id, "tile-1"),
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn test_dispatch_does_not_cross_session_boundary() {
        let mut state = make_state("key");
        authenticate(&mut state, "agent-a", "key");
        let session_b = authenticate(&mut state, "agent-b", "key");

        // Only subscribe agent-b
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel(256);
        state
            .sessions
            .get_session_mut(&session_b)
            .unwrap()
            .event_tx = Some(tx_b);

        // Dispatch to agent-a (not subscribed)
        let dispatched = state.sessions.dispatch_to_namespace("agent-a", SceneEvent {
            timestamp_ms: 42,
            event: Some(scene_event::Event::TileCreated(TileCreatedEvent {
                tile_id: "x".to_string(),
                namespace: "agent-a".to_string(),
                timestamp_ms: 42,
            })),
        });
        assert!(!dispatched, "agent-a has no channel, should return false");

        // agent-b's channel must be empty
        assert!(rx_b.try_recv().is_err(), "agent-b must not receive agent-a events");
    }

    #[test]
    fn test_dispatch_input_event_reaches_agent() {
        let mut state = make_state("key");
        let session_id = authenticate(&mut state, "my-agent", "key");

        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        state
            .sessions
            .get_session_mut(&session_id)
            .unwrap()
            .event_tx = Some(tx);

        let fake_tile = tze_hud_scene::SceneId::new();
        let fake_node = tze_hud_scene::SceneId::new();

        dispatch_input_event(
            &mut state,
            "my-agent",
            fake_tile,
            fake_node,
            "btn-1",
            500.0,
            300.0,
            100.0,
            80.0,
            InputEventKind::Activated,
        );

        let event = rx.try_recv().expect("input event should be in channel");
        match event.event {
            Some(scene_event::Event::Input(ie)) => {
                assert_eq!(ie.interaction_id, "btn-1");
                assert_eq!(ie.kind, InputEventKind::Activated as i32);
                assert!((ie.display_x - 500.0).abs() < 0.01);
                assert!((ie.local_x - 100.0).abs() < 0.01);
                assert!((ie.local_y - 80.0).abs() < 0.01);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn test_lease_event_dispatched_on_acquire() {
        let mut state = make_state("key");
        let session_id = authenticate(&mut state, "lessee", "key");
        state.scene.create_tab("Main", 0).unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        state
            .sessions
            .get_session_mut(&session_id)
            .unwrap()
            .event_tx = Some(tx);

        // Grant a lease directly — mirrors what acquire_lease does
        let lease_id = state.scene.grant_lease("lessee", 60_000, vec![]);
        state
            .sessions
            .get_session_mut(&session_id)
            .unwrap()
            .lease_ids
            .push(lease_id);

        let event = SceneEvent {
            timestamp_ms: now_ms(),
            event: Some(scene_event::Event::Lease(LeaseEvent {
                lease_id: lease_id.to_string(),
                namespace: "lessee".to_string(),
                kind: LeaseEventKind::LeaseGranted as i32,
                timestamp_ms: now_ms(),
            })),
        };
        let dispatched = state.sessions.dispatch_to_namespace("lessee", event);
        assert!(dispatched);

        let received = rx.try_recv().expect("lease event should arrive");
        match received.event {
            Some(scene_event::Event::Lease(le)) => {
                assert_eq!(le.kind, LeaseEventKind::LeaseGranted as i32);
                assert_eq!(le.namespace, "lessee");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }
}
