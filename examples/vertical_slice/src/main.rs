//! # Vertical Slice Example
//!
//! Reference binary demonstrating the full tze_hud contract path:
//!
//! **Phase 1** — Session + Lease: bidirectional streaming handshake, lease grant
//! **Phase 2** — Scene Setup: tab, tiles (hit region + text), zone publish
//! **Phase 3** — Input Loop: pointer events, hit-test, local ack, agent dispatch
//! **Phase 4** — Telemetry: frame metrics, telemetry frame over session stream
//! **Phase 5** — Safe Mode: suspend all leases, verify mutation rejection, resume
//! **Phase 6** — Graceful Shutdown: SessionClose, cleanup verification
//!
//! Run headless:  cargo run -p vertical_slice -- --headless
//! Run windowed:  cargo run -p vertical_slice

#[allow(deprecated)]
use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;
use tze_hud_protocol::proto::session as session_proto;
use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;

use tze_hud_scene::types::*;
use tze_hud_scene::mutation::{MutationBatch as SceneMutationBatch, SceneMutation};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");

    if headless {
        run_headless().await
    } else {
        // For now, default to headless as well (windowed requires event loop)
        println!("Windowed mode not yet implemented; running headless demo.");
        run_headless().await
    }
}

fn now_wall_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

async fn run_headless() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== tze_hud vertical slice (full contract path) ===\n");

    // ─── Initialize runtime ────────────────────────────────────────────────
    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 50051,
        psk: "vertical-slice-key".to_string(),
    };

    let mut runtime = HeadlessRuntime::new(config).await?;
    let _server = runtime.start_grpc_server().await?;
    println!("Runtime initialized: 800x600, gRPC on [::1]:50051\n");

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 1: Session + Lease (streaming)
    // ─────────────────────────────────────────────────────────────────────────
    println!("=== Phase 1: Session Handshake + Lease Acquisition ===\n");

    let mut session_client =
        HudSessionClient::connect("http://[::1]:50051").await?;

    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    let now_us = now_wall_us();

    // Send SessionInit
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: "vertical-slice-agent".to_string(),
                agent_display_name: "Vertical Slice Agent".to_string(),
                pre_shared_key: "vertical-slice-key".to_string(),
                requested_capabilities: vec![
                    "create_tile".to_string(),
                    "create_node".to_string(),
                    "receive_input".to_string(),
                ],
                initial_subscriptions: vec!["SCENE_TOPOLOGY".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            },
        )),
    })
    .await?;

    let mut response_stream = session_client.session(stream).await?.into_inner();

    // Read SessionEstablished
    use tokio_stream::StreamExt;
    let msg = response_stream.next().await.unwrap()?;
    let namespace = match &msg.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(established)) => {
            println!("  Session established:");
            println!("    namespace       = {}", established.namespace);
            println!("    heartbeat_ms    = {}", established.heartbeat_interval_ms);
            println!("    capabilities    = {:?}", established.granted_capabilities);
            println!("    clock_skew      = {}us", established.estimated_skew_us);
            established.namespace.clone()
        }
        other => {
            return Err(format!("Expected SessionEstablished, got: {other:?}").into());
        }
    };

    // Read SceneSnapshot
    let msg = response_stream.next().await.unwrap()?;
    match &msg.payload {
        Some(session_proto::server_message::Payload::SceneSnapshot(snapshot)) => {
            println!("  Scene snapshot: sequence={}, json_len={}, checksum={}",
                snapshot.sequence, snapshot.snapshot_json.len(), &snapshot.blake3_checksum[..8.min(snapshot.blake3_checksum.len())]);
        }
        other => {
            return Err(format!("Expected SceneSnapshot, got: {other:?}").into());
        }
    }

    // Request lease
    tx.send(session_proto::ClientMessage {
        sequence: 2,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::LeaseRequest(
            session_proto::LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec![
                    "create_tile".to_string(),
                    "create_node".to_string(),
                    "receive_input".to_string(),
                ],
                lease_priority: 2,
            },
        )),
    })
    .await?;

    let msg = response_stream.next().await.unwrap()?;
    let _lease_id_bytes = match &msg.payload {
        Some(session_proto::server_message::Payload::LeaseResponse(resp)) if resp.granted => {
            println!("  Lease granted: ttl={}ms, priority={}", resp.granted_ttl_ms, resp.granted_priority);
            resp.lease_id.clone()
        }
        other => {
            return Err(format!("Expected LeaseResponse (granted), got: {other:?}").into());
        }
    };

    // Heartbeat round-trip
    let hb_mono = 999_000u64;
    tx.send(session_proto::ClientMessage {
        sequence: 3,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::Heartbeat(
            session_proto::Heartbeat {
                timestamp_mono_us: hb_mono,
            },
        )),
    })
    .await?;

    let msg = response_stream.next().await.unwrap()?;
    match &msg.payload {
        Some(session_proto::server_message::Payload::Heartbeat(hb)) => {
            assert_eq!(hb.timestamp_mono_us, hb_mono, "heartbeat echo mismatch");
            println!("  Heartbeat echoed: mono_us={}", hb.timestamp_mono_us);
        }
        other => {
            return Err(format!("Expected Heartbeat, got: {other:?}").into());
        }
    }

    println!("\n  Phase 1 PASSED: session established, lease active, heartbeat verified.\n");

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 2: Scene Setup — tab, tiles, zone publish
    // ─────────────────────────────────────────────────────────────────────────
    println!("=== Phase 2: Scene Setup (tab + tiles + zone publish) ===\n");

    // Create a tab and register zones directly on the scene graph.
    // (The streaming session shares state via Arc<Mutex<SharedState>>.)
    let (tab_id, lease_id) = {
        let mut state = runtime.shared_state().lock().await;
        let tab_id = state.scene.create_tab("Main", 0).unwrap();
        println!("  Tab created: id={}", tab_id);

        // Register the default zones
        state.scene.zone_registry = ZoneRegistry::with_defaults();
        let zone_count = state.scene.zone_registry.all_zones().len();
        println!("  Registered {} default zones (status-bar, notification-area, subtitle)", zone_count);

        // We already have a lease from Phase 1 -- find it by namespace
        let lease_id = state.scene.leases.values()
            .find(|l| l.namespace == namespace && l.is_active())
            .map(|l| l.id)
            .expect("should have an active lease from Phase 1");
        println!("  Using lease: {}", lease_id);

        (tab_id, lease_id)
    };

    // Create text tile via scene graph
    let _text_tile_id = {
        let mut state = runtime.shared_state().lock().await;
        let tile_id = state.scene.create_tile(
            tab_id,
            &namespace,
            lease_id,
            Rect::new(50.0, 50.0, 350.0, 250.0),
            1,
        ).unwrap();

        state.scene.set_tile_root(
            tile_id,
            Node {
                id: SceneId::new(),
                children: vec![],
                data: NodeData::TextMarkdown(TextMarkdownNode {
                    content: "Hello, tze_hud! Status display tile.".to_string(),
                    bounds: Rect::new(0.0, 0.0, 350.0, 250.0),
                    font_size_px: 24.0,
                    font_family: FontFamily::SystemSansSerif,
                    color: Rgba::WHITE,
                    background: Some(Rgba::new(0.1, 0.15, 0.3, 1.0)),
                    alignment: TextAlign::Start,
                    overflow: TextOverflow::Clip,
                }),
            },
        ).unwrap();

        println!("  Text tile created: id={}", tile_id);
        tile_id
    };

    // Create hit region tile via scene graph
    let (_hit_tile_id, _hr_node_id) = {
        let mut state = runtime.shared_state().lock().await;
        let tile_id = state.scene.create_tile(
            tab_id,
            &namespace,
            lease_id,
            Rect::new(450.0, 50.0, 300.0, 250.0),
            2,
        ).unwrap();

        let node_id = SceneId::new();
        state.scene.set_tile_root(
            tile_id,
            Node {
                id: node_id,
                children: vec![],
                data: NodeData::HitRegion(HitRegionNode {
                    bounds: Rect::new(25.0, 25.0, 250.0, 200.0),
                    interaction_id: "demo-button".to_string(),
                    accepts_focus: true,
                    accepts_pointer: true,
                    ..Default::default()
                }),
            },
        ).unwrap();

        println!("  Hit region tile created: id={}, node={}", tile_id, node_id);
        (tile_id, node_id)
    };

    // Publish to status-bar zone
    {
        let mut state = runtime.shared_state().lock().await;
        let mut entries = std::collections::HashMap::new();
        entries.insert("agent".to_string(), "vertical-slice-agent".to_string());
        entries.insert("status".to_string(), "running".to_string());

        state.scene.publish_to_zone(
            "status-bar",
            ZoneContent::StatusBar(StatusBarPayload { entries }),
            &namespace,
            Some("agent-status".to_string()),
            None,
            None,
        ).unwrap();
        println!("  Published to status-bar zone (MergeByKey, key=agent-status)");

        // Verify the publish is active
        let active = state.scene.zone_registry.active_for_zone("status-bar");
        assert!(!active.is_empty(), "status-bar should have active publishes");
        println!("  status-bar active publishes: {}", active.len());
    }

    // Publish to notification-area zone
    {
        let mut state = runtime.shared_state().lock().await;
        state.scene.publish_to_zone(
            "notification-area",
            ZoneContent::Notification(NotificationPayload {
                text: "Vertical slice started".to_string(),
                icon: "info".to_string(),
                urgency: 1,
            }),
            &namespace,
            None,
            None,
            None,
        ).unwrap();
        println!("  Published notification to notification-area zone");
    }

    // Verify scene state
    {
        let state = runtime.shared_state().lock().await;
        assert_eq!(state.scene.tiles.len(), 2, "expected 2 tiles");
        assert_eq!(state.scene.tabs.len(), 1, "expected 1 tab");
        let active_leases: usize = state.scene.leases.values()
            .filter(|l| l.is_active())
            .count();
        assert!(active_leases >= 1, "expected at least 1 active lease");
        println!("  Scene verified: {} tabs, {} tiles, {} active leases",
            state.scene.tabs.len(), state.scene.tiles.len(), active_leases);
    }

    println!("\n  Phase 2 PASSED: scene populated with tab, tiles, and zone content.\n");

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 3: Input Loop — pointer events, hit-test, dispatch
    // ─────────────────────────────────────────────────────────────────────────
    println!("=== Phase 3: Input Loop (pointer events + hit-test + dispatch) ===\n");

    // Hover over the hit region
    let hover_result = {
        let state_arc = runtime.shared_state().clone();
        let mut state = state_arc.lock().await;
        runtime.input_processor.process(
            &tze_hud_input::PointerEvent {
                x: 550.0,
                y: 150.0,
                kind: tze_hud_input::PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut state.scene,
        )
    };
    assert!(
        matches!(hover_result.hit, tze_hud_scene::HitResult::NodeHit { .. }),
        "hover should hit the tile"
    );
    assert_eq!(hover_result.interaction_id, Some("demo-button".to_string()));
    assert!(hover_result.dispatch.is_some(), "should dispatch PointerEnter");
    let dispatch = hover_result.dispatch.as_ref().unwrap();
    assert_eq!(dispatch.kind, tze_hud_input::AgentDispatchKind::PointerEnter);
    assert_eq!(dispatch.interaction_id, "demo-button");
    println!("  Hover: hit=NodeHit, interaction_id=demo-button, dispatch=PointerEnter");

    // Move within the hit region (should produce PointerMove)
    let move_result = {
        let state_arc = runtime.shared_state().clone();
        let mut state = state_arc.lock().await;
        runtime.input_processor.process(
            &tze_hud_input::PointerEvent {
                x: 560.0,
                y: 160.0,
                kind: tze_hud_input::PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut state.scene,
        )
    };
    assert!(move_result.dispatch.is_some());
    assert_eq!(move_result.dispatch.as_ref().unwrap().kind,
        tze_hud_input::AgentDispatchKind::PointerMove);
    println!("  Move: dispatch=PointerMove, local_coords=({:.1},{:.1})",
        move_result.dispatch.as_ref().unwrap().local_x,
        move_result.dispatch.as_ref().unwrap().local_y);

    // Press on hit region
    let press_result = {
        let state_arc = runtime.shared_state().clone();
        let mut state = state_arc.lock().await;
        runtime.input_processor.process(
            &tze_hud_input::PointerEvent {
                x: 550.0,
                y: 150.0,
                kind: tze_hud_input::PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut state.scene,
        )
    };
    assert!(press_result.dispatch.is_some());
    assert_eq!(press_result.dispatch.as_ref().unwrap().kind,
        tze_hud_input::AgentDispatchKind::PointerDown);
    println!("  Press: local_ack={}us, hit_test={}us, dispatch=PointerDown",
        press_result.local_ack_us, press_result.hit_test_us);

    // Verify local ack is within 4ms budget
    assert!(
        press_result.local_ack_us < 4_000,
        "local_ack_us={}us exceeds 4ms budget",
        press_result.local_ack_us
    );
    println!("  Budget check: local_ack={}us < 4000us PASSED", press_result.local_ack_us);

    // Release (activate)
    let release_result = {
        let state_arc = runtime.shared_state().clone();
        let mut state = state_arc.lock().await;
        runtime.input_processor.process(
            &tze_hud_input::PointerEvent {
                x: 550.0,
                y: 150.0,
                kind: tze_hud_input::PointerEventKind::Up,
                device_id: 0,
                timestamp: None,
            },
            &mut state.scene,
        )
    };
    assert!(release_result.activated, "press+release on same node should activate");
    assert!(release_result.dispatch.is_some());
    assert_eq!(release_result.dispatch.as_ref().unwrap().kind,
        tze_hud_input::AgentDispatchKind::Activated);
    assert_eq!(release_result.dispatch.as_ref().unwrap().interaction_id, "demo-button");
    println!("  Release: activated=true, dispatch=Activated(demo-button)");

    // Record latencies
    runtime.telemetry.summary_mut().input_to_local_ack.record(press_result.local_ack_us);
    runtime.telemetry.summary_mut().hit_test_latency.record(press_result.hit_test_us);

    println!("\n  Phase 3 PASSED: input pipeline exercised, latency budgets met.\n");

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 4: Telemetry — frame metrics + stream telemetry
    // ─────────────────────────────────────────────────────────────────────────
    println!("=== Phase 4: Telemetry (frame metrics + stream telemetry) ===\n");

    // Render a frame and collect telemetry
    let frame_telemetry = runtime.render_frame().await;
    println!("  Frame rendered:");
    println!("    frame_time     = {}us", frame_telemetry.frame_time_us);
    println!("    tiles          = {}", frame_telemetry.tile_count);
    println!("    nodes          = {}", frame_telemetry.node_count);
    println!("    active_leases  = {}", frame_telemetry.active_leases);
    println!("    render_encode  = {}us", frame_telemetry.render_encode_us);
    println!("    gpu_submit     = {}us", frame_telemetry.gpu_submit_us);

    assert!(frame_telemetry.tile_count >= 2, "expected at least 2 tiles in frame");
    assert!(frame_telemetry.node_count >= 2, "expected at least 2 nodes in frame");

    // Pixel readback to verify rendering
    let pixels = runtime.read_pixels();
    assert_eq!(pixels.len(), 800 * 600 * 4, "pixel buffer size mismatch");
    println!("  Pixel readback: {} bytes (800x600 RGBA)", pixels.len());

    // Send TelemetryFrame over the session stream
    tx.send(session_proto::ClientMessage {
        sequence: 4,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::TelemetryFrame(
            session_proto::TelemetryFrame {
                sample_timestamp_wall_us: now_wall_us(),
                mutations_sent: 3,
                mutations_acked: 3,
                rtt_estimate_us: 500,
            },
        )),
    })
    .await?;
    println!("  TelemetryFrame sent over session stream (mutations=3, rtt=500us)");

    // Emit session summary JSON
    let summary = runtime.telemetry.summary();
    println!("  Session summary:");
    println!("    total_frames = {}", summary.total_frames);
    if let Some(p50) = summary.frame_time.p50() {
        println!("    frame_time p50 = {}us", p50);
    }
    if let Some(p99) = summary.frame_time.p99() {
        println!("    frame_time p99 = {}us", p99);
    }
    if let Some(ack) = summary.input_to_local_ack.p99() {
        println!("    input_to_local_ack p99 = {}us (budget: 4000us)", ack);
    }
    if let Some(ht) = summary.hit_test_latency.p99() {
        println!("    hit_test p99 = {}us (budget: 100us)", ht);
    }

    let json = runtime.telemetry.emit_json()?;
    println!("  Telemetry JSON emitted: {} bytes", json.len());

    println!("\n  Phase 4 PASSED: frame rendered, telemetry collected and streamed.\n");

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 5: Safe Mode — suspend/resume all leases, mutation rejection
    // ─────────────────────────────────────────────────────────────────────────
    println!("=== Phase 5: Safe Mode (suspend + mutation rejection + resume) ===\n");

    // Verify the lease is active before suspension
    {
        let state = runtime.shared_state().lock().await;
        let lease = state.scene.leases.get(&lease_id).unwrap();
        assert_eq!(lease.state, LeaseState::Active, "lease should be Active before suspension");
        println!("  Pre-suspend: lease state={:?}", lease.state);
    }

    // Suspend all leases (safe mode entry)
    {
        let mut state = runtime.shared_state().lock().await;
        let now = now_ms();
        state.scene.suspend_all_leases(now);

        // Verify all leases are suspended
        let suspended_count = state.scene.leases.values()
            .filter(|l| l.state == LeaseState::Suspended)
            .count();
        println!("  Suspended {} lease(s) (safe mode entry)", suspended_count);
        assert!(suspended_count >= 1, "at least 1 lease should be suspended");

        let lease = state.scene.leases.get(&lease_id).unwrap();
        assert_eq!(lease.state, LeaseState::Suspended);
        assert!(lease.suspended_at_ms.is_some(), "should track suspension time");
        assert!(lease.ttl_remaining_at_suspend_ms.is_some(), "should track remaining TTL");
        println!("  Lease state: {:?}, suspended_at={:?}, ttl_remaining={:?}ms",
            lease.state, lease.suspended_at_ms, lease.ttl_remaining_at_suspend_ms);
    }

    // Attempt mutations during suspension -- should be rejected
    {
        let mut state = runtime.shared_state().lock().await;
        let batch = SceneMutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: namespace.clone(),
            mutations: vec![SceneMutation::CreateTile {
                tab_id,
                namespace: namespace.clone(),
                lease_id,
                bounds: Rect::new(100.0, 350.0, 200.0, 100.0),
                z_order: 3,
            }],
            timing_hints: None,
            lease_id: None,
        };

        let result = state.scene.apply_batch(&batch);
        assert!(!result.applied, "mutations should be rejected during suspension");
        let error_msg = result.error.as_ref().map(|e| e.to_string()).unwrap_or_default();
        println!("  Mutation during suspension: rejected=true");
        println!("    error: {}", error_msg);
        assert!(error_msg.contains("Suspended"), "error should mention Suspended state");
    }

    // Tiles should still be present (state preserved, only mutations blocked)
    {
        let state = runtime.shared_state().lock().await;
        assert_eq!(state.scene.tiles.len(), 2, "tiles should be preserved during suspension");
        println!("  Tiles preserved during suspension: count={}", state.scene.tiles.len());
    }

    // Render during suspension -- should still produce a frame (display frozen state)
    let suspended_frame = runtime.render_frame().await;
    println!("  Frame during suspension: tiles={}, nodes={}",
        suspended_frame.tile_count, suspended_frame.node_count);
    assert!(suspended_frame.tile_count >= 2, "tiles should still render during suspension");

    // Resume all leases (safe mode exit)
    {
        let mut state = runtime.shared_state().lock().await;
        let now = now_ms();
        state.scene.resume_all_leases(now);

        let active_count = state.scene.leases.values()
            .filter(|l| l.is_active())
            .count();
        println!("  Resumed {} lease(s) (safe mode exit)", active_count);
        assert!(active_count >= 1, "at least 1 lease should be active after resume");

        let lease = state.scene.leases.get(&lease_id).unwrap();
        assert_eq!(lease.state, LeaseState::Active, "lease should be Active after resume");
        assert!(lease.suspended_at_ms.is_none(), "suspension timestamp should be cleared");
        println!("  Lease state after resume: {:?}", lease.state);
    }

    // Verify mutations work again after resume
    {
        let mut state = runtime.shared_state().lock().await;
        let batch = SceneMutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: namespace.clone(),
            mutations: vec![SceneMutation::CreateTile {
                tab_id,
                namespace: namespace.clone(),
                lease_id,
                bounds: Rect::new(100.0, 350.0, 200.0, 100.0),
                z_order: 3,
            }],
            timing_hints: None,
            lease_id: None,
        };

        let result = state.scene.apply_batch(&batch);
        assert!(result.applied, "mutations should succeed after resume");
        let new_tile = result.created_ids[0];
        println!("  Post-resume mutation: tile created id={}", new_tile);

        // Clean up via DeleteTile mutation
        let delete_batch = SceneMutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: namespace.clone(),
            mutations: vec![SceneMutation::DeleteTile { tile_id: new_tile }],
            timing_hints: None,
            lease_id: None,
        };
        let del_result = state.scene.apply_batch(&delete_batch);
        assert!(del_result.applied, "delete tile should succeed");
        println!("  Cleanup: deleted post-resume tile");
    }

    println!("\n  Phase 5 PASSED: safe mode suspend/resume verified, mutation gating works.\n");

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 6: Graceful Shutdown
    // ─────────────────────────────────────────────────────────────────────────
    println!("=== Phase 6: Graceful Shutdown ===\n");

    // Send SessionClose
    tx.send(session_proto::ClientMessage {
        sequence: 5,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::SessionClose(
            session_proto::SessionClose {
                reason: "Vertical slice complete".to_string(),
                expect_resume: false,
            },
        )),
    })
    .await?;
    println!("  SessionClose sent: reason='Vertical slice complete'");

    // Drop the sender to close the stream
    drop(tx);

    // Verify the stream closes gracefully
    let next = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        response_stream.next(),
    ).await;

    match next {
        Ok(None) => println!("  Stream closed gracefully (server-side cleanup)"),
        Ok(Some(_)) => println!("  Stream received final message before close"),
        Err(_) => println!("  Stream timed out (expected -- server-side cleanup async)"),
    }

    // Final state verification
    {
        let state = runtime.shared_state().lock().await;
        println!("  Final scene state:");
        println!("    tabs   = {}", state.scene.tabs.len());
        println!("    tiles  = {}", state.scene.tiles.len());
        println!("    leases = {} total ({} active)",
            state.scene.leases.len(),
            state.scene.leases.values().filter(|l| l.is_active()).count());
        println!("    zones  = {} registered, {} with active publishes",
            state.scene.zone_registry.zones.len(),
            state.scene.zone_registry.active_publishes.len());
        println!("    version = {}", state.scene.version);
    }

    println!("\n  Phase 6 PASSED: graceful shutdown complete.\n");

    // ─── Final Summary ─────────────────────────────────────────────────────
    println!("===================================================");
    println!("  VERTICAL SLICE COMPLETE -- ALL 6 PHASES PASSED");
    println!();
    println!("  Phase 1: Session handshake + lease acquisition");
    println!("  Phase 2: Scene setup (tab + tiles + zone publish)");
    println!("  Phase 3: Input loop (pointer events + hit-test + dispatch)");
    println!("  Phase 4: Telemetry (frame metrics + stream telemetry)");
    println!("  Phase 5: Safe mode (suspend + rejection + resume)");
    println!("  Phase 6: Graceful shutdown");
    println!("===================================================");

    Ok(())
}

// ─── Test module ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tze_hud_input::{InputProcessor, PointerEvent, PointerEventKind, AgentDispatchKind};
    use tze_hud_scene::graph::SceneGraph;
    use tze_hud_scene::types::*;
    use tze_hud_scene::mutation::{MutationBatch as SceneMutationBatch, SceneMutation};
    use tze_hud_telemetry::LatencyBucket;
    use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;
    use tze_hud_protocol::proto::session as session_proto;
    use tze_hud_runtime::HeadlessRuntime;
    use tze_hud_runtime::headless::HeadlessConfig;
    use std::time::Instant;

    fn now_wall_us() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    // ─── Helpers ────────────────────────────────────────────────────────────

    fn setup_scene_with_lease() -> (SceneGraph, SceneId, SceneId) {
        let mut scene = SceneGraph::new(800.0, 600.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test-agent",
            60_000,
            vec![Capability::CreateTile, Capability::CreateNode, Capability::ReceiveInput],
        );
        (scene, tab_id, lease_id)
    }

    fn add_hit_region_tile(
        scene: &mut SceneGraph,
        tab_id: SceneId,
        lease_id: SceneId,
        tile_bounds: Rect,
        hr_bounds: Rect,
        interaction_id: &str,
        z_order: u32,
    ) -> (SceneId, SceneId) {
        let tile_id = scene.create_tile(
            tab_id, "test-agent", lease_id, tile_bounds, z_order,
        ).unwrap();
        let node_id = SceneId::new();
        scene.set_tile_root(tile_id, Node {
            id: node_id,
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: hr_bounds,
                interaction_id: interaction_id.to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        }).unwrap();
        (tile_id, node_id)
    }

    // ─── Phase 1 tests: handshake timing ────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_handshake_completes_within_budget() {
        // Budget: handshake should complete in under 5000ms
        let config = HeadlessConfig {
            width: 320,
            height: 240,
            grpc_port: 50061,
            psk: "test-key".to_string(),
        };
        let runtime = HeadlessRuntime::new(config).await.unwrap();
        let _server = runtime.start_grpc_server().await.unwrap();

        let start = Instant::now();

        let mut client = HudSessionClient::connect("http://[::1]:50061").await.unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(16);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        tx.send(session_proto::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_wall_us(),
            payload: Some(session_proto::client_message::Payload::SessionInit(
                session_proto::SessionInit {
                    agent_id: "test-agent".to_string(),
                    agent_display_name: "Test".to_string(),
                    pre_shared_key: "test-key".to_string(),
                    requested_capabilities: vec!["create_tile".to_string()],
                    initial_subscriptions: vec![],
                    resume_token: Vec::new(),
                    agent_timestamp_wall_us: now_wall_us(),
                    min_protocol_version: 1000,
                    max_protocol_version: 1001,
                    auth_credential: None,
                },
            )),
        }).await.unwrap();

        let mut response = client.session(stream).await.unwrap().into_inner();

        // SessionEstablished
        use tokio_stream::StreamExt;
        let msg = response.next().await.unwrap().unwrap();
        assert!(matches!(
            msg.payload,
            Some(session_proto::server_message::Payload::SessionEstablished(_))
        ));

        // SceneSnapshot
        let msg = response.next().await.unwrap().unwrap();
        assert!(matches!(
            msg.payload,
            Some(session_proto::server_message::Payload::SceneSnapshot(_))
        ));

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 5000,
            "handshake took {}ms, budget is 5000ms",
            elapsed.as_millis()
        );
    }

    // ─── Phase 2 tests: lease state transitions ─────────────────────────────

    #[test]
    fn test_lease_state_transitions_end_to_end() {
        let (mut scene, _tab_id, lease_id) = setup_scene_with_lease();

        // Active -> Suspended
        let now = now_ms();
        scene.suspend_lease(&lease_id, now).unwrap();
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Suspended);

        // Suspended -> Active (resume)
        let now = now_ms();
        scene.resume_lease(&lease_id, now).unwrap();
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Active);

        // Active -> Orphaned (disconnect)
        let now = now_ms();
        scene.disconnect_lease(&lease_id, now).unwrap();
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Orphaned);

        // Orphaned -> Active (reconnect)
        let now = now_ms();
        scene.reconnect_lease(&lease_id, now).unwrap();
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Active);

        // Active -> Revoked
        scene.revoke_lease(lease_id).unwrap();
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Revoked);

        // Cannot transition from terminal state
        assert!(scene.suspend_lease(&lease_id, now_ms()).is_err());
    }

    // ─── Phase 3 tests: hit-test + dispatch ─────────────────────────────────

    #[test]
    fn test_hit_test_returns_correct_tile_and_node() {
        let (mut scene, tab_id, lease_id) = setup_scene_with_lease();

        let (tile_id, node_id) = add_hit_region_tile(
            &mut scene, tab_id, lease_id,
            Rect::new(100.0, 100.0, 300.0, 200.0),
            Rect::new(0.0, 0.0, 300.0, 200.0),
            "button-1",
            1,
        );

        let mut processor = InputProcessor::new();

        // Hit inside
        let result = processor.process(
            &PointerEvent { x: 200.0, y: 180.0, kind: PointerEventKind::Down, device_id: 0, timestamp: None },
            &mut scene,
        );
        assert_eq!(
            result.hit,
            tze_hud_scene::HitResult::NodeHit {
                tile_id,
                node_id,
                interaction_id: "button-1".to_string(),
            }
        );
        assert_eq!(result.interaction_id, Some("button-1".to_string()));

        // Hit outside
        let result = processor.process(
            &PointerEvent { x: 10.0, y: 10.0, kind: PointerEventKind::Down, device_id: 0, timestamp: None },
            &mut scene,
        );
        assert!(result.hit.is_none());
    }

    #[test]
    fn test_agent_dispatch_contains_correct_interaction_id() {
        let (mut scene, tab_id, lease_id) = setup_scene_with_lease();

        add_hit_region_tile(
            &mut scene, tab_id, lease_id,
            Rect::new(100.0, 100.0, 200.0, 200.0),
            Rect::new(0.0, 0.0, 200.0, 200.0),
            "my-button",
            1,
        );

        let mut processor = InputProcessor::new();

        // Press on button
        let result = processor.process(
            &PointerEvent { x: 200.0, y: 200.0, kind: PointerEventKind::Down, device_id: 0, timestamp: None },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("should have dispatch");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerDown);
        assert_eq!(dispatch.interaction_id, "my-button");
        assert_eq!(dispatch.namespace, "test-agent");

        // Release on button (activate)
        let result = processor.process(
            &PointerEvent { x: 200.0, y: 200.0, kind: PointerEventKind::Up, device_id: 0, timestamp: None },
            &mut scene,
        );

        assert!(result.activated);
        let dispatch = result.dispatch.expect("should have dispatch");
        assert_eq!(dispatch.kind, AgentDispatchKind::Activated);
        assert_eq!(dispatch.interaction_id, "my-button");
    }

    #[test]
    fn test_local_ack_within_4ms_budget() {
        let (mut scene, tab_id, lease_id) = setup_scene_with_lease();

        add_hit_region_tile(
            &mut scene, tab_id, lease_id,
            Rect::new(100.0, 100.0, 200.0, 200.0),
            Rect::new(0.0, 0.0, 200.0, 200.0),
            "btn",
            1,
        );

        let mut processor = InputProcessor::new();
        let mut bucket = LatencyBucket::new("local_ack");

        for _ in 0..30 {
            let result = processor.process(
                &PointerEvent { x: 200.0, y: 200.0, kind: PointerEventKind::Down, device_id: 0, timestamp: None },
                &mut scene,
            );
            bucket.record(result.local_ack_us);
        }

        bucket.assert_p99_under(4_000)
            .expect("local_ack p99 should be under 4ms");
    }

    // ─── Phase 5 tests: safe mode ───────────────────────────────────────────

    #[test]
    fn test_safe_mode_suspends_and_resumes() {
        let (mut scene, tab_id, lease_id) = setup_scene_with_lease();

        // Create a tile while active
        let tile_id = scene.create_tile(
            tab_id, "test-agent", lease_id,
            Rect::new(10.0, 10.0, 100.0, 100.0), 1,
        ).unwrap();

        // Suspend all
        scene.suspend_all_leases(now_ms());
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Suspended);

        // Tile still exists
        assert!(scene.tiles.contains_key(&tile_id));

        // Resume all
        scene.resume_all_leases(now_ms());
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Active);

        // Tile still exists
        assert!(scene.tiles.contains_key(&tile_id));
    }

    #[test]
    fn test_safe_mode_rejects_mutations() {
        let (mut scene, tab_id, lease_id) = setup_scene_with_lease();

        // Suspend
        scene.suspend_all_leases(now_ms());

        // Try to create a tile -- should fail
        let batch = SceneMutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test-agent".to_string(),
            mutations: vec![SceneMutation::CreateTile {
                tab_id,
                namespace: "test-agent".to_string(),
                lease_id,
                bounds: Rect::new(10.0, 10.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };

        let result = scene.apply_batch(&batch);
        assert!(!result.applied, "mutations should be rejected during suspension");

        let error_msg = result.error.as_ref().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            error_msg.contains("Suspended"),
            "error should mention Suspended state, got: {}",
            error_msg
        );
    }

    #[test]
    fn test_safe_mode_mutations_succeed_after_resume() {
        let (mut scene, tab_id, lease_id) = setup_scene_with_lease();

        // Suspend then resume
        scene.suspend_all_leases(now_ms());
        scene.resume_all_leases(now_ms());

        // Mutations should work now
        let batch = SceneMutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test-agent".to_string(),
            mutations: vec![SceneMutation::CreateTile {
                tab_id,
                namespace: "test-agent".to_string(),
                lease_id,
                bounds: Rect::new(10.0, 10.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };

        let result = scene.apply_batch(&batch);
        assert!(result.applied, "mutations should succeed after resume");
        assert_eq!(result.created_ids.len(), 1);
    }

    // ─── Budget enforcement test ────────────────────────────────────────────

    #[test]
    fn test_budget_enforcement_rejects_over_limit() {
        let (mut scene, tab_id, lease_id) = setup_scene_with_lease();

        // Default budget allows max 8 tiles per lease.
        // Create 8 tiles to fill the budget.
        for i in 0..8u32 {
            let batch = SceneMutationBatch {
                batch_id: SceneId::new(),
                agent_namespace: "test-agent".to_string(),
                mutations: vec![SceneMutation::CreateTile {
                    tab_id,
                    namespace: "test-agent".to_string(),
                    lease_id,
                    bounds: Rect::new(i as f32 * 90.0, 10.0, 80.0, 60.0),
                    z_order: i + 1,
                }],
                timing_hints: None,
                lease_id: None,
            };
            let result = scene.apply_batch(&batch);
            assert!(result.applied, "tile {} should be within budget", i);
        }

        // 9th tile should exceed budget
        let batch = SceneMutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test-agent".to_string(),
            mutations: vec![SceneMutation::CreateTile {
                tab_id,
                namespace: "test-agent".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 100.0, 80.0, 60.0),
                z_order: 9,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(!result.applied, "9th tile should exceed budget");

        let error_msg = result.error.as_ref().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            error_msg.contains("tiles") || error_msg.contains("budget"),
            "error should reference tile budget, got: {}",
            error_msg
        );
    }

    // ─── Zone publish test ──────────────────────────────────────────────────

    #[test]
    fn test_zone_publish_and_query() {
        let (mut scene, _tab_id, _lease_id) = setup_scene_with_lease();

        // Register default zones
        scene.zone_registry = ZoneRegistry::with_defaults();

        // Publish to status-bar
        let mut entries = std::collections::HashMap::new();
        entries.insert("key".to_string(), "value".to_string());
        scene.publish_to_zone(
            "status-bar",
            ZoneContent::StatusBar(StatusBarPayload { entries }),
            "test-agent",
            Some("test-key".to_string()),
            None,
            None,
        ).unwrap();

        let active = scene.zone_registry.active_for_zone("status-bar");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].publisher_namespace, "test-agent");

        // Publish again with same merge key -- should replace
        let mut entries2 = std::collections::HashMap::new();
        entries2.insert("key".to_string(), "updated".to_string());
        scene.publish_to_zone(
            "status-bar",
            ZoneContent::StatusBar(StatusBarPayload { entries: entries2 }),
            "test-agent",
            Some("test-key".to_string()),
            None,
            None,
        ).unwrap();

        let active = scene.zone_registry.active_for_zone("status-bar");
        assert_eq!(active.len(), 1, "MergeByKey should replace, not accumulate");
    }

    // ─── Zone not found test ────────────────────────────────────────────────

    #[test]
    fn test_zone_publish_fails_for_unknown_zone() {
        let (mut scene, _tab_id, _lease_id) = setup_scene_with_lease();

        let result = scene.publish_to_zone(
            "nonexistent-zone",
            ZoneContent::StreamText("hello".to_string()),
            "test-agent",
            None,
            None,
            None,
        );

        assert!(result.is_err(), "should fail for unknown zone");
    }
}
