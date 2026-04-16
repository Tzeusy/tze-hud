//! Transport-agnostic portal bridge + tmux pilot adapter tests (hud-t98e.3).
//!
//! This validates the first adapter path without teaching runtime code about
//! tmux/PTY/process lifecycle semantics:
//! - bridge contract is generic output chunks + bounded input submission + status
//! - tmux-backed adapter maps into that contract externally
//! - non-tmux adapter fixtures can use the same unchanged bridge contract
//! - pilot updates are sent over the existing primary resident session stream

use std::collections::VecDeque;

use tokio_stream::StreamExt;
use tze_hud_protocol::auth::{RUNTIME_MAX_VERSION, RUNTIME_MIN_VERSION};
use tze_hud_protocol::proto;
use tze_hud_protocol::proto::session as session_proto;
#[allow(deprecated)]
use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;
use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;
use tze_hud_scene::types::{NodeData, ZoneRegistry};

const DISPLAY_W: f32 = 1920.0;
const DISPLAY_H: f32 = 1080.0;
const TEST_PSK: &str = "text-portal-adapter-test-key";
const MAX_VIEWER_INPUT_BYTES: usize = 512;

fn now_wall_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn now_mono_us() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_micros() as u64
}

#[derive(Clone, Debug)]
struct AdapterClock;

impl AdapterClock {
    fn now_wall_us(&self) -> u64 {
        now_wall_us()
    }

    fn now_mono_us(&self) -> u64 {
        now_mono_us()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PortalSessionStatus {
    Connecting,
    Live,
}

#[derive(Clone, Debug)]
struct PortalSessionIdentity {
    portal_id: String,
    display_name: String,
}

#[derive(Clone, Debug)]
struct PortalOutputChunk {
    ordinal: u64,
    text: String,
    emitted_wall_us: u64,
    observed_mono_us: u64,
}

#[derive(Clone, Debug)]
struct ViewerInputSubmission {
    text: String,
    submitted_wall_us: u64,
    submitted_mono_us: u64,
}

trait PortalAdapter {
    fn identity(&self) -> &PortalSessionIdentity;
    fn status(&self) -> PortalSessionStatus;
    fn drain_output(&mut self) -> Vec<PortalOutputChunk>;
    fn submit_input(&mut self, submission: ViewerInputSubmission) -> Result<(), String>;
}

struct PortalBridge<A: PortalAdapter> {
    adapter: A,
    transcript: VecDeque<String>,
    max_retained_lines: usize,
}

impl<A: PortalAdapter> PortalBridge<A> {
    fn new(adapter: A, max_retained_lines: usize) -> Self {
        Self {
            adapter,
            transcript: VecDeque::new(),
            max_retained_lines,
        }
    }

    fn identity(&self) -> &PortalSessionIdentity {
        self.adapter.identity()
    }

    fn status(&self) -> PortalSessionStatus {
        self.adapter.status()
    }

    fn ingest_adapter_output(&mut self) -> usize {
        let mut chunks = self.adapter.drain_output();
        chunks.sort_by_key(|c| c.ordinal);
        let mut appended = 0usize;

        for chunk in chunks {
            // Timing fields are transport metadata only; they never override
            // runtime presentation order, which stays ordinal-based.
            let _ = (chunk.emitted_wall_us, chunk.observed_mono_us);
            self.transcript.push_back(chunk.text);
            appended += 1;
        }

        while self.transcript.len() > self.max_retained_lines {
            self.transcript.pop_front();
        }

        appended
    }

    fn visible_markdown(&self, max_lines: usize) -> String {
        let keep = max_lines.min(self.transcript.len());
        let start = self.transcript.len().saturating_sub(keep);
        self.transcript
            .iter()
            .skip(start)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn submit_bounded_input(&mut self, text: &str) -> Result<(), String> {
        if text.len() > MAX_VIEWER_INPUT_BYTES {
            return Err(format!(
                "viewer input exceeds {} bytes (got {})",
                MAX_VIEWER_INPUT_BYTES,
                text.len()
            ));
        }

        self.adapter.submit_input(ViewerInputSubmission {
            text: text.to_string(),
            submitted_wall_us: now_wall_us(),
            submitted_mono_us: now_mono_us(),
        })
    }
}

#[derive(Debug)]
struct TmuxPilotAdapter {
    // Internal tmux selector stays private and never crosses the bridge trait.
    pane_selector: String,
    identity: PortalSessionIdentity,
    status: PortalSessionStatus,
    next_ordinal: u64,
    pending_lines: VecDeque<String>,
    submitted_inputs: Vec<String>,
}

impl TmuxPilotAdapter {
    fn new(portal_id: &str, display_name: &str, pane_selector: &str) -> Self {
        Self {
            pane_selector: pane_selector.to_string(),
            identity: PortalSessionIdentity {
                portal_id: portal_id.to_string(),
                display_name: display_name.to_string(),
            },
            status: PortalSessionStatus::Connecting,
            next_ordinal: 1,
            pending_lines: VecDeque::new(),
            submitted_inputs: Vec::new(),
        }
    }

    fn mark_live(&mut self) {
        self.status = PortalSessionStatus::Live;
    }

    fn queue_tmux_line(&mut self, line: &str) {
        let _ = &self.pane_selector;
        self.pending_lines.push_back(line.to_string());
    }

    fn submitted_inputs(&self) -> &[String] {
        &self.submitted_inputs
    }
}

impl PortalAdapter for TmuxPilotAdapter {
    fn identity(&self) -> &PortalSessionIdentity {
        &self.identity
    }

    fn status(&self) -> PortalSessionStatus {
        self.status.clone()
    }

    fn drain_output(&mut self) -> Vec<PortalOutputChunk> {
        let mut out = Vec::new();
        while let Some(line) = self.pending_lines.pop_front() {
            out.push(PortalOutputChunk {
                ordinal: self.next_ordinal,
                text: line,
                emitted_wall_us: now_wall_us(),
                observed_mono_us: now_mono_us(),
            });
            self.next_ordinal += 1;
        }
        out
    }

    fn submit_input(&mut self, submission: ViewerInputSubmission) -> Result<(), String> {
        let _ = (submission.submitted_wall_us, submission.submitted_mono_us);
        self.submitted_inputs.push(submission.text);
        Ok(())
    }
}

#[derive(Debug)]
struct RelayChatAdapter {
    // Internal relay selector stays private and never crosses the bridge trait.
    channel_ref: String,
    clock: AdapterClock,
    identity: PortalSessionIdentity,
    status: PortalSessionStatus,
    next_ordinal: u64,
    pending_messages: VecDeque<String>,
    submitted_inputs: Vec<String>,
}

impl RelayChatAdapter {
    fn new(portal_id: &str, display_name: &str, channel_ref: &str) -> Self {
        Self {
            channel_ref: channel_ref.to_string(),
            clock: AdapterClock,
            identity: PortalSessionIdentity {
                portal_id: portal_id.to_string(),
                display_name: display_name.to_string(),
            },
            status: PortalSessionStatus::Connecting,
            next_ordinal: 1,
            pending_messages: VecDeque::new(),
            submitted_inputs: Vec::new(),
        }
    }

    fn mark_live(&mut self) {
        self.status = PortalSessionStatus::Live;
    }

    fn queue_relay_message(&mut self, message: &str) {
        let _ = &self.channel_ref;
        self.pending_messages.push_back(message.to_string());
    }

    fn submitted_inputs(&self) -> &[String] {
        &self.submitted_inputs
    }
}

impl PortalAdapter for RelayChatAdapter {
    fn identity(&self) -> &PortalSessionIdentity {
        &self.identity
    }

    fn status(&self) -> PortalSessionStatus {
        self.status.clone()
    }

    fn drain_output(&mut self) -> Vec<PortalOutputChunk> {
        let mut out = Vec::new();
        let wall = self.clock.now_wall_us();
        let mono = self.clock.now_mono_us();
        while let Some(message) = self.pending_messages.pop_front() {
            out.push(PortalOutputChunk {
                ordinal: self.next_ordinal,
                text: message,
                emitted_wall_us: wall,
                observed_mono_us: mono,
            });
            self.next_ordinal += 1;
        }
        out
    }

    fn submit_input(&mut self, submission: ViewerInputSubmission) -> Result<(), String> {
        let _ = (submission.submitted_wall_us, submission.submitted_mono_us);
        self.submitted_inputs.push(submission.text);
        Ok(())
    }
}

struct AgentSession {
    namespace: String,
    lease_id: Vec<u8>,
    tx: tokio::sync::mpsc::Sender<session_proto::ClientMessage>,
    rx: tonic::codec::Streaming<session_proto::ServerMessage>,
    sequence: u64,
}

impl AgentSession {
    fn next_seq(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    async fn next_non_state_change(
        &mut self,
    ) -> Result<session_proto::ServerMessage, Box<dyn std::error::Error>> {
        loop {
            let msg = self
                .rx
                .next()
                .await
                .ok_or("server stream ended unexpectedly")??;
            if let Some(session_proto::server_message::Payload::LeaseStateChange(_)) = msg.payload {
                continue;
            }
            return Ok(msg);
        }
    }
}

struct SessionSceneVerification {
    session_tile_count: usize,
    contains_incremental: bool,
    contains_identity: bool,
    session_count: usize,
}

async fn verify_scene_for_namespace(
    runtime: &HeadlessRuntime,
    namespace: &str,
    incremental_needle: &str,
    identity_needle: &str,
) -> SessionSceneVerification {
    let state = runtime.shared_state().lock().await;
    let scene = state.scene.lock().await;
    let mut has_incremental = false;
    let mut has_identity = false;
    let mut count = 0;
    for tile in scene.tiles.values() {
        if tile.namespace != namespace {
            continue;
        }
        count += 1;
        if let Some(root_id) = tile.root_node {
            if let Some(node) = scene.nodes.get(&root_id) {
                if let NodeData::TextMarkdown(text) = &node.data {
                    has_incremental |= text.content.contains(incremental_needle);
                    has_identity |= text.content.contains(identity_needle);
                }
            }
        }
    }
    SessionSceneVerification {
        session_tile_count: count,
        contains_incremental: has_incremental,
        contains_identity: has_identity,
        session_count: state.sessions.session_count(),
    }
}

async fn connect_agent(
    agent_id: &str,
    grpc_port: u16,
) -> Result<AgentSession, Box<dyn std::error::Error>> {
    let mut client = HudSessionClient::connect(format!("http://[::1]:{grpc_port}")).await?;

    let (tx, rx_chan) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx_chan);

    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: format!("{agent_id} (portal adapter test)"),
                pre_shared_key: TEST_PSK.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                initial_subscriptions: vec!["SCENE_TOPOLOGY".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_wall_us(),
                min_protocol_version: RUNTIME_MIN_VERSION,
                max_protocol_version: RUNTIME_MAX_VERSION,
                auth_credential: None,
            },
        )),
    })
    .await?;

    let mut rx = client.session(stream).await?.into_inner();

    let established = rx.next().await.ok_or("missing SessionEstablished")??;
    let namespace = match established.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(est)) => est.namespace,
        other => {
            return Err(format!("expected SessionEstablished, got: {other:?}").into());
        }
    };

    let _snapshot = rx.next().await.ok_or("missing SceneSnapshot")??;

    tx.send(session_proto::ClientMessage {
        sequence: 2,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::LeaseRequest(
            session_proto::LeaseRequest {
                ttl_ms: 120_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            },
        )),
    })
    .await?;

    let mut session = AgentSession {
        namespace,
        lease_id: Vec::new(),
        tx,
        rx,
        sequence: 2,
    };

    let msg = session.next_non_state_change().await?;
    match msg.payload {
        Some(session_proto::server_message::Payload::LeaseResponse(resp)) if resp.granted => {
            session.lease_id = resp.lease_id;
            Ok(session)
        }
        other => Err(format!("expected granted LeaseResponse, got: {other:?}").into()),
    }
}

async fn create_tile(
    session: &mut AgentSession,
    bounds: proto::Rect,
    z_order: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let seq = session.next_seq();
    session
        .tx
        .send(session_proto::ClientMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(session_proto::client_message::Payload::MutationBatch(
                session_proto::MutationBatch {
                    batch_id: uuid::Uuid::now_v7().as_bytes().to_vec(),
                    lease_id: session.lease_id.clone(),
                    mutations: vec![proto::MutationProto {
                        mutation: Some(proto::mutation_proto::Mutation::CreateTile(
                            proto::CreateTileMutation {
                                tab_id: vec![],
                                bounds: Some(bounds),
                                z_order,
                            },
                        )),
                    }],
                    timing: None,
                },
            )),
        })
        .await?;

    let msg = session.next_non_state_change().await?;
    match msg.payload {
        Some(session_proto::server_message::Payload::MutationResult(result)) if result.accepted => {
            Ok(result
                .created_ids
                .first()
                .cloned()
                .expect("MutationResult accepted but no IDs created"))
        }
        Some(session_proto::server_message::Payload::MutationResult(result)) => Err(format!(
            "CreateTile rejected: {} {}",
            result.error_code, result.error_message
        )
        .into()),
        other => Err(format!("expected MutationResult, got: {other:?}").into()),
    }
}

async fn set_tile_root_text(
    session: &mut AgentSession,
    tile_id: Vec<u8>,
    text: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let seq = session.next_seq();
    session
        .tx
        .send(session_proto::ClientMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(session_proto::client_message::Payload::MutationBatch(
                session_proto::MutationBatch {
                    batch_id: uuid::Uuid::now_v7().as_bytes().to_vec(),
                    lease_id: session.lease_id.clone(),
                    mutations: vec![proto::MutationProto {
                        mutation: Some(proto::mutation_proto::Mutation::SetTileRoot(
                            proto::SetTileRootMutation {
                                tile_id,
                                node: Some(proto::NodeProto {
                                    id: vec![],
                                    data: Some(proto::node_proto::Data::TextMarkdown(
                                        proto::TextMarkdownNodeProto {
                                            content: text,
                                            bounds: Some(proto::Rect {
                                                x: 16.0,
                                                y: 16.0,
                                                width: 680.0,
                                                height: 300.0,
                                            }),
                                            font_size_px: 14.0,
                                            color: Some(proto::Rgba {
                                                r: 0.95,
                                                g: 0.97,
                                                b: 1.0,
                                                a: 1.0,
                                            }),
                                            background: Some(proto::Rgba {
                                                r: 0.05,
                                                g: 0.06,
                                                b: 0.08,
                                                a: 0.8,
                                            }),
                                        },
                                    )),
                                }),
                            },
                        )),
                    }],
                    timing: None,
                },
            )),
        })
        .await?;

    let msg = session.next_non_state_change().await?;
    match msg.payload {
        Some(session_proto::server_message::Payload::MutationResult(result)) if result.accepted => {
            Ok(())
        }
        Some(session_proto::server_message::Payload::MutationResult(result)) => Err(format!(
            "SetTileRoot rejected: {} {}",
            result.error_code, result.error_message
        )
        .into()),
        other => Err(format!("expected MutationResult, got: {other:?}").into()),
    }
}

#[test]
fn tmux_adapter_satisfies_transport_agnostic_bridge_contract() {
    let mut adapter =
        TmuxPilotAdapter::new("portal://pilot/tmux-1", "TMUX Portal", "session:build.1");
    assert_eq!(adapter.status(), PortalSessionStatus::Connecting);

    adapter.mark_live();
    adapter.queue_tmux_line("build: compiling crate A");
    adapter.queue_tmux_line("build: tests passed");

    let mut bridge = PortalBridge::new(adapter, 64);
    assert_eq!(bridge.status(), PortalSessionStatus::Live);

    let appended = bridge.ingest_adapter_output();
    assert_eq!(appended, 2, "must ingest incremental output chunks");

    let markdown = bridge.visible_markdown(16);
    assert!(markdown.contains("compiling crate A"));
    assert!(markdown.contains("tests passed"));

    bridge
        .submit_bounded_input("interrupt current step")
        .expect("bounded viewer input must pass through bridge");

    assert_eq!(
        bridge.adapter.submitted_inputs(),
        &["interrupt current step".to_string()],
        "bridge must forward viewer input transactionally to adapter"
    );

    assert_eq!(
        bridge.identity().portal_id,
        "portal://pilot/tmux-1",
        "bridge identity must stay transport-agnostic"
    );
}

#[test]
fn non_tmux_adapter_satisfies_transport_agnostic_bridge_contract() {
    let mut adapter = RelayChatAdapter::new(
        "portal://pilot/non-tmux-1",
        "Relay Chat Portal",
        "relay://ops-alerts",
    );
    assert_eq!(adapter.status(), PortalSessionStatus::Connecting);

    adapter.mark_live();
    adapter.queue_relay_message("incident-bot: build queue recovered");
    adapter.queue_relay_message("operator: monitoring next deploy wave");

    let mut bridge = PortalBridge::new(adapter, 64);
    assert_eq!(bridge.status(), PortalSessionStatus::Live);

    let appended = bridge.ingest_adapter_output();
    assert_eq!(appended, 2, "must ingest incremental output chunks");

    let markdown = bridge.visible_markdown(16);
    assert!(markdown.contains("build queue recovered"));
    assert!(markdown.contains("monitoring next deploy wave"));

    bridge
        .submit_bounded_input("acknowledge and continue")
        .expect("bounded viewer input must pass through bridge");

    assert_eq!(
        bridge.adapter.submitted_inputs(),
        &["acknowledge and continue".to_string()],
        "bridge must forward viewer input transactionally to adapter"
    );

    assert_eq!(
        bridge.identity().portal_id,
        "portal://pilot/non-tmux-1",
        "bridge identity must stay transport-agnostic"
    );
}

#[tokio::test]
async fn tmux_pilot_drives_portal_over_existing_primary_session_stream()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = std::net::TcpListener::bind("[::1]:0")?;
    let grpc_port = listener.local_addr()?.port();
    drop(listener);

    let config = HeadlessConfig {
        width: DISPLAY_W as u32,
        height: DISPLAY_H as u32,
        grpc_port,
        psk: TEST_PSK.to_string(),
        config_toml: None,
    };
    let runtime = HeadlessRuntime::new(config).await?;

    {
        let state = runtime.shared_state().lock().await;
        let mut scene = state.scene.lock().await;
        let tab_id = scene.create_tab("Portal-Adapter-Test", 0)?;
        scene.active_tab = Some(tab_id);
        scene.zone_registry = ZoneRegistry::with_defaults();
    }

    let _server = runtime.start_grpc_server().await?;

    let mut session = connect_agent("portal-adapter-agent", grpc_port).await?;

    let mut adapter =
        TmuxPilotAdapter::new("portal://pilot/tmux-live", "TMUX Pilot", "session:pilot.0");
    adapter.mark_live();
    adapter.queue_tmux_line("agent> starting pilot stream");

    let mut bridge = PortalBridge::new(adapter, 128);

    let tile_id = create_tile(
        &mut session,
        proto::Rect {
            x: 64.0,
            y: 180.0,
            width: 720.0,
            height: 360.0,
        },
        150,
    )
    .await?;

    bridge.ingest_adapter_output();
    let first = format!(
        "**{}** ({:?})\n{}",
        bridge.identity().display_name,
        bridge.status(),
        bridge.visible_markdown(32)
    );
    set_tile_root_text(&mut session, tile_id.clone(), first).await?;

    bridge
        .adapter
        .queue_tmux_line("agent> rendered incremental update");
    bridge.ingest_adapter_output();
    bridge
        .submit_bounded_input("interrupt")
        .expect("viewer input should remain bounded and flow through the adapter contract");

    let second = format!(
        "**{}** ({:?})\n{}",
        bridge.identity().display_name,
        bridge.status(),
        bridge.visible_markdown(32)
    );
    set_tile_root_text(&mut session, tile_id, second).await?;

    let scene_verification = verify_scene_for_namespace(
        &runtime,
        &session.namespace,
        "incremental update",
        "TMUX Pilot",
    )
    .await;

    assert_eq!(
        scene_verification.session_tile_count, 1,
        "pilot creates one governed content-layer tile"
    );
    assert!(
        scene_verification.contains_incremental,
        "stream increments must land in portal content"
    );
    assert!(
        scene_verification.contains_identity,
        "identity text must be rendered from bridge contract"
    );
    assert_eq!(
        scene_verification.session_count, 1,
        "tmux pilot path must not open an additional HudSession stream"
    );
    assert_eq!(
        session.sequence, 5,
        "all portal activity must run through the single primary resident stream"
    );

    Ok(())
}

#[tokio::test]
async fn non_tmux_adapter_drives_portal_over_existing_primary_session_stream()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = std::net::TcpListener::bind("[::1]:0")?;
    let grpc_port = listener.local_addr()?.port();
    drop(listener);

    let config = HeadlessConfig {
        width: DISPLAY_W as u32,
        height: DISPLAY_H as u32,
        grpc_port,
        psk: TEST_PSK.to_string(),
        config_toml: None,
    };
    let runtime = HeadlessRuntime::new(config).await?;

    {
        let state = runtime.shared_state().lock().await;
        let mut scene = state.scene.lock().await;
        let tab_id = scene.create_tab("Portal-Adapter-Test-NonTmux", 0)?;
        scene.active_tab = Some(tab_id);
        scene.zone_registry = ZoneRegistry::with_defaults();
    }

    let _server = runtime.start_grpc_server().await?;

    let mut session = connect_agent("portal-adapter-non-tmux-agent", grpc_port).await?;

    let mut adapter = RelayChatAdapter::new(
        "portal://pilot/non-tmux-live",
        "Relay Chat Pilot",
        "relay://incident-room",
    );
    adapter.mark_live();
    adapter.queue_relay_message("relay> opening non-tmux stream");

    let mut bridge = PortalBridge::new(adapter, 128);

    let tile_id = create_tile(
        &mut session,
        proto::Rect {
            x: 80.0,
            y: 220.0,
            width: 720.0,
            height: 360.0,
        },
        150,
    )
    .await?;

    bridge.ingest_adapter_output();
    let first = format!(
        "**{}** ({:?})\n{}",
        bridge.identity().display_name,
        bridge.status(),
        bridge.visible_markdown(32)
    );
    set_tile_root_text(&mut session, tile_id.clone(), first).await?;

    bridge
        .adapter
        .queue_relay_message("relay> propagated follow-up update");
    bridge.ingest_adapter_output();
    bridge
        .submit_bounded_input("ack")
        .expect("viewer input should remain bounded and flow through the adapter contract");

    let second = format!(
        "**{}** ({:?})\n{}",
        bridge.identity().display_name,
        bridge.status(),
        bridge.visible_markdown(32)
    );
    set_tile_root_text(&mut session, tile_id, second).await?;

    let scene_verification = verify_scene_for_namespace(
        &runtime,
        &session.namespace,
        "follow-up update",
        "Relay Chat Pilot",
    )
    .await;

    assert_eq!(
        scene_verification.session_tile_count, 1,
        "non-tmux pilot creates one governed content-layer tile"
    );
    assert!(
        scene_verification.contains_incremental,
        "stream increments must land in portal content"
    );
    assert!(
        scene_verification.contains_identity,
        "identity text must be rendered from bridge contract"
    );
    assert_eq!(
        scene_verification.session_count, 1,
        "non-tmux pilot path must not open an additional HudSession stream"
    );
    assert_eq!(
        session.sequence, 5,
        "all portal activity must run through the single primary resident stream"
    );

    Ok(())
}
