//! gRPC server implementation for the SceneService.

use crate::convert;
use crate::proto::scene_service_server::SceneService;
use crate::proto::*;
use crate::session::SessionRegistry;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tonic::{Request, Response, Status};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_scene::mutation::{MutationBatch as SceneMutationBatch, SceneMutation};

/// Shared state between the gRPC server and the compositor.
pub struct SharedState {
    pub scene: SceneGraph,
    pub sessions: SessionRegistry,
    pub event_tx: broadcast::Sender<SceneEvent>,
}

/// The gRPC service implementation.
pub struct SceneServiceImpl {
    pub state: Arc<Mutex<SharedState>>,
}

impl SceneServiceImpl {
    pub fn new(scene: SceneGraph, psk: &str) -> Self {
        let (event_tx, _) = broadcast::channel(1024);
        Self {
            state: Arc::new(Mutex::new(SharedState {
                scene,
                sessions: SessionRegistry::new(psk),
                event_tx,
            })),
        }
    }
}

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
        let _session = state
            .sessions
            .get_session(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;

        let lease_id = parse_scene_id(&req.lease_id)?;
        let ttl = if req.new_ttl_ms > 0 { req.new_ttl_ms } else { 60_000 };

        match state.scene.renew_lease(lease_id, ttl) {
            Ok(()) => Ok(Response::new(LeaseRenewResponse {
                success: true,
                error: String::new(),
            })),
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

        let _session = state
            .sessions
            .get_session(&req.session_id)
            .ok_or_else(|| Status::unauthenticated("invalid session"))?;

        let lease_id = parse_scene_id(&req.lease_id)?;

        match state.scene.revoke_lease(lease_id) {
            Ok(()) => Ok(Response::new(LeaseRevokeResponse {
                success: true,
                error: String::new(),
            })),
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
            // Broadcast events for created IDs
            for id in &result.created_ids {
                let _ = state.event_tx.send(SceneEvent {
                    event_type: "mutation_applied".to_string(),
                    tile_id: id.to_string(),
                    node_id: String::new(),
                    interaction_id: String::new(),
                    details: format!("batch applied by {namespace}"),
                    timestamp_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                });
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

    type SubscribeEventsStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<SceneEvent, Status>> + Send>>;

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
        session.event_subscribed = true;

        let rx = state.event_tx.subscribe();
        let stream = tokio_stream::wrappers::BroadcastStream::new(rx);
        let mapped = tokio_stream::StreamExt::map(stream, |item| {
            item.map_err(|e| Status::internal(e.to_string()))
        });

        Ok(Response::new(Box::pin(mapped)))
    }
}

fn parse_scene_id(s: &str) -> Result<tze_hud_scene::SceneId, Status> {
    uuid::Uuid::parse_str(s)
        .map(tze_hud_scene::SceneId::from_uuid)
        .map_err(|e| Status::invalid_argument(format!("invalid scene ID '{s}': {e}")))
}
