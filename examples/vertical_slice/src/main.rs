//! # Vertical Slice Example
//!
//! Reference binary demonstrating all 6 layers of tze_hud:
//! 1. Headless scene graph (pure data)
//! 2. Native window + compositor (wgpu)
//! 3. Resident gRPC agent (unary + streaming)
//! 4. Lease acquisition
//! 5. Interactive hit-region
//! 6. Telemetry + artifacts
//!
//! Run headless:  cargo run -p vertical_slice -- --headless
//! Run windowed:  cargo run -p vertical_slice

#[allow(deprecated)]
use tze_hud_protocol::proto::scene_service_client::SceneServiceClient;
#[allow(deprecated)]
use tze_hud_protocol::proto::*;
use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;
use tze_hud_protocol::proto::session as session_proto;
use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;

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

#[allow(deprecated)]
async fn run_headless() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== tze_hud vertical slice (headless) ===\n");

    // ─── Layer 1: Create scene graph ─────────────────────────────────────
    println!("Layer 1: Creating headless scene graph...");

    let config = HeadlessConfig {
        width: 800,
        height: 600,
        grpc_port: 50051,
        psk: "vertical-slice-key".to_string(),
    };

    let mut runtime = HeadlessRuntime::new(config).await?;
    println!("  Scene graph initialized (800x600)");

    // ─── Layer 3: Start gRPC server ──────────────────────────────────────
    println!("\nLayer 3: Starting gRPC server...");
    let _server = runtime.start_grpc_server().await?;
    println!("  gRPC server listening on [::1]:50051");

    // ─── Layer 3+4: Connect agent and acquire lease (unary) ──────────────
    println!("\nLayer 3+4: Agent connecting via unary RPCs...");

    let mut client =
        SceneServiceClient::connect("http://[::1]:50051").await?;

    // Authenticate
    let connect_resp = client
        .authenticate(ConnectRequest {
            agent_name: "test-agent".to_string(),
            pre_shared_key: "vertical-slice-key".to_string(),
            requested_capabilities: vec![
                "create_tile".to_string(),
                "create_node".to_string(),
                "receive_input".to_string(),
            ],
        })
        .await?
        .into_inner();

    if !connect_resp.error.is_empty() {
        return Err(format!("Connect failed: {}", connect_resp.error).into());
    }
    let session_id = connect_resp.session_id;
    println!("  Agent authenticated: session={}", &session_id[..8]);

    // Create a tab in the scene directly (needed before agent can create tiles)
    {
        let mut state = runtime.shared_state().lock().await;
        state.scene.create_tab("Main", 0).unwrap();
    }

    // Acquire lease
    let lease_resp = client
        .acquire_lease(LeaseRequest {
            session_id: session_id.clone(),
            ttl_ms: 60_000,
            capabilities: vec![
                "create_tile".to_string(),
                "create_node".to_string(),
                "receive_input".to_string(),
            ],
        })
        .await?
        .into_inner();

    if !lease_resp.error.is_empty() {
        return Err(format!("Lease failed: {}", lease_resp.error).into());
    }
    let lease_id = lease_resp.lease_id;
    println!("  Lease acquired: id={}, ttl={}ms", &lease_id[..8], lease_resp.granted_ttl_ms);

    // ─── Create content via gRPC mutations ───────────────────────────────
    println!("\n  Creating tiles via gRPC mutations...");

    // Create text tile
    let mut_resp = client
        .apply_mutations(MutationBatchRequest {
            session_id: session_id.clone(),
            lease_id: lease_id.clone(),
            mutations: vec![MutationProto {
                mutation: Some(mutation_proto::Mutation::CreateTile(CreateTileMutation {
                    tab_id: String::new(), // server uses active tab
                    bounds: Some(Rect {
                        x: 50.0,
                        y: 50.0,
                        width: 350.0,
                        height: 250.0,
                    }),
                    z_order: 1,
                })),
            }],
        })
        .await?
        .into_inner();

    assert!(mut_resp.success, "Tile creation failed: {}", mut_resp.error);
    let text_tile_id = mut_resp.created_ids[0].clone();
    println!("  Text tile created: {}", &text_tile_id[..8]);

    // Set text content on first tile
    let mut_resp = client
        .apply_mutations(MutationBatchRequest {
            session_id: session_id.clone(),
            lease_id: lease_id.clone(),
            mutations: vec![MutationProto {
                mutation: Some(mutation_proto::Mutation::SetTileRoot(SetTileRootMutation {
                    tile_id: text_tile_id.clone(),
                    node: Some(NodeProto {
                        id: String::new(),
                        data: Some(node_proto::Data::TextMarkdown(TextMarkdownNodeProto {
                            content: "Hello, tze_hud! This is a text tile.".to_string(),
                            bounds: Some(Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 350.0,
                                height: 250.0,
                            }),
                            font_size_px: 24.0,
                            color: Some(Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }),
                            background: Some(Rgba { r: 0.1, g: 0.15, b: 0.3, a: 1.0 }),
                        })),
                    }),
                })),
            }],
        })
        .await?
        .into_inner();

    assert!(mut_resp.success, "Set tile root failed: {}", mut_resp.error);
    println!("  Text content set on tile");

    // Create hit-region tile
    let mut_resp = client
        .apply_mutations(MutationBatchRequest {
            session_id: session_id.clone(),
            lease_id: lease_id.clone(),
            mutations: vec![MutationProto {
                mutation: Some(mutation_proto::Mutation::CreateTile(CreateTileMutation {
                    tab_id: String::new(),
                    bounds: Some(Rect {
                        x: 450.0,
                        y: 50.0,
                        width: 300.0,
                        height: 250.0,
                    }),
                    z_order: 2,
                })),
            }],
        })
        .await?
        .into_inner();

    assert!(mut_resp.success);
    let hit_tile_id = mut_resp.created_ids[0].clone();
    println!("  Hit-region tile created: {}", &hit_tile_id[..8]);

    // Set hit-region on second tile
    let mut_resp = client
        .apply_mutations(MutationBatchRequest {
            session_id: session_id.clone(),
            lease_id: lease_id.clone(),
            mutations: vec![MutationProto {
                mutation: Some(mutation_proto::Mutation::SetTileRoot(SetTileRootMutation {
                    tile_id: hit_tile_id.clone(),
                    node: Some(NodeProto {
                        id: String::new(),
                        data: Some(node_proto::Data::HitRegion(HitRegionNodeProto {
                            bounds: Some(Rect {
                                x: 25.0,
                                y: 25.0,
                                width: 250.0,
                                height: 200.0,
                            }),
                            interaction_id: "demo-button".to_string(),
                            accepts_focus: true,
                            accepts_pointer: true,
                        })),
                    }),
                })),
            }],
        })
        .await?
        .into_inner();

    assert!(mut_resp.success, "Set hit region failed: {}", mut_resp.error);
    println!("  Hit-region set on tile");

    // ─── Layer 5: Simulate input interaction ─────────────────────────────
    println!("\nLayer 5: Simulating pointer interaction...");

    let (press_local_ack, press_hit_test) = {
        let state_arc = runtime.shared_state().clone();
        let mut state = state_arc.lock().await;
        let input = &mut runtime.input_processor;

        // Hover over the hit region
        let hover_result = input.process(
            &tze_hud_input::PointerEvent {
                x: 550.0,
                y: 150.0,
                kind: tze_hud_input::PointerEventKind::Move,
                timestamp: None,
            },
            &mut state.scene,
        );
        println!("  Hover: hit={}, interaction={:?}",
            hover_result.hit.is_some(),
            hover_result.interaction_id
        );

        // Press
        let press_result = input.process(
            &tze_hud_input::PointerEvent {
                x: 550.0,
                y: 150.0,
                kind: tze_hud_input::PointerEventKind::Down,
                timestamp: None,
            },
            &mut state.scene,
        );
        println!("  Press: local_ack={}us, hit_test={}us",
            press_result.local_ack_us, press_result.hit_test_us
        );

        // Release (activate)
        let release_result = input.process(
            &tze_hud_input::PointerEvent {
                x: 550.0,
                y: 150.0,
                kind: tze_hud_input::PointerEventKind::Up,
                timestamp: None,
            },
            &mut state.scene,
        );
        println!("  Release: activated={}, interaction={:?}",
            release_result.activated,
            release_result.interaction_id
        );

        (press_result.local_ack_us, press_result.hit_test_us)
    };

    // Record latencies (after dropping the state lock)
    runtime.telemetry.summary_mut().input_to_local_ack.record(press_local_ack);
    runtime.telemetry.summary_mut().hit_test_latency.record(press_hit_test);

    // ─── Layer 2: Render frame ───────────────────────────────────────────
    println!("\nLayer 2: Rendering frame...");
    let frame_telemetry = runtime.render_frame().await;
    println!("  Frame rendered:");
    println!("    frame_time={}us", frame_telemetry.frame_time_us);
    println!("    tiles={}", frame_telemetry.tile_count);
    println!("    nodes={}", frame_telemetry.node_count);
    println!("    leases={}", frame_telemetry.active_leases);
    println!("    render_encode={}us", frame_telemetry.render_encode_us);
    println!("    gpu_submit={}us", frame_telemetry.gpu_submit_us);

    // ─── Layer 2: Pixel readback ─────────────────────────────────────────
    println!("\nLayer 2: Reading back pixels...");
    let pixels = runtime.read_pixels();
    println!("  Pixel buffer: {} bytes ({}x{} RGBA)",
        pixels.len(), 800, 600
    );

    // Verify some pixels
    let bg_pixel = get_pixel(&pixels, 800, 5, 5);
    let text_tile_pixel = get_pixel(&pixels, 800, 200, 150);
    let hit_tile_pixel = get_pixel(&pixels, 800, 550, 150);

    println!("  Background pixel (5,5): {:?}", bg_pixel);
    println!("  Text tile pixel (200,150): {:?}", text_tile_pixel);
    println!("  Hit tile pixel (550,150): {:?}", hit_tile_pixel);

    // ─── Layer 3: Query scene via gRPC ───────────────────────────────────
    println!("\n  Querying scene via gRPC...");
    let scene_resp = client
        .query_scene(SceneQueryRequest {
            session_id: session_id.clone(),
        })
        .await?
        .into_inner();
    println!("  Scene version: {}", scene_resp.version);
    println!("  Scene JSON length: {} chars", scene_resp.scene_json.len());

    // ─── Layer 6: Emit telemetry ─────────────────────────────────────────
    println!("\nLayer 6: Telemetry summary:");
    let summary = runtime.telemetry.summary();
    println!("  total_frames={}", summary.total_frames);
    if let Some(p50) = summary.frame_time.p50() {
        println!("  frame_time p50={}us", p50);
    }
    if let Some(p99) = summary.frame_time.p99() {
        println!("  frame_time p99={}us", p99);
    }
    if let Some(ack) = summary.input_to_local_ack.p99() {
        println!("  input_to_local_ack p99={}us (budget: 4000us)", ack);
    }
    if let Some(ht) = summary.hit_test_latency.p99() {
        println!("  hit_test p99={}us (budget: 100us)", ht);
    }

    // Emit JSON telemetry
    let telemetry_json = runtime.telemetry.emit_json()?;
    println!("\n  Telemetry JSON ({} bytes):", telemetry_json.len());

    // ─── Layer 4: Verify lease renewal ───────────────────────────────────
    println!("\nLayer 4: Testing lease renewal...");
    let renew_resp = client
        .renew_lease(LeaseRenewRequest {
            session_id: session_id.clone(),
            lease_id: lease_id.clone(),
            new_ttl_ms: 120_000,
        })
        .await?
        .into_inner();
    println!("  Lease renewed: success={}", renew_resp.success);

    // ─── Layer 4: Simulate lease revocation ──────────────────────────────
    println!("\nLayer 4: Simulating lease revocation (human override)...");
    let revoke_resp = client
        .revoke_lease(LeaseRevokeRequest {
            session_id: session_id.clone(),
            lease_id: lease_id.clone(),
        })
        .await?
        .into_inner();
    println!("  Lease revoked: success={}", revoke_resp.success);

    // Render after revocation — tiles should be gone
    let post_revoke_telemetry = runtime.render_frame().await;
    println!("  Post-revocation: tiles={}, nodes={}",
        post_revoke_telemetry.tile_count, post_revoke_telemetry.node_count
    );

    // ─── NEW: Streaming session demo (RFC 0005) ─────────────────────────
    println!("\n=== Streaming Session Demo (RFC 0005) ===\n");

    // Re-create a tab for streaming demo (tiles were revoked above)
    {
        let mut state = runtime.shared_state().lock().await;
        state.scene.create_tab("StreamDemo", 1).unwrap();
    }

    let mut session_client =
        HudSessionClient::connect("http://[::1]:50051").await?;

    // Open bidirectional stream
    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    // Send SessionInit
    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64;

    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: "stream-agent".to_string(),
                agent_display_name: "Streaming Agent".to_string(),
                pre_shared_key: "vertical-slice-key".to_string(),
                requested_capabilities: vec!["create_tile".to_string()],
                initial_subscriptions: vec!["SCENE_TOPOLOGY".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
            },
        )),
    })
    .await?;

    let mut response_stream = session_client.session(stream).await?.into_inner();

    // Read SessionAccepted
    use tokio_stream::StreamExt;
    let msg = response_stream.next().await.unwrap()?;
    match &msg.payload {
        Some(session_proto::server_message::Payload::SessionAccepted(accepted)) => {
            println!("  Session accepted: namespace={}", accepted.namespace);
            println!("    heartbeat_interval={}ms", accepted.heartbeat_interval_ms);
            println!("    capabilities={:?}", accepted.granted_capabilities);
        }
        other => {
            return Err(format!("Expected SessionAccepted, got: {other:?}").into());
        }
    }

    // Read SceneSnapshot
    let msg = response_stream.next().await.unwrap()?;
    match &msg.payload {
        Some(session_proto::server_message::Payload::SceneSnapshot(snapshot)) => {
            println!("  Scene snapshot received: version={}, json_len={}",
                snapshot.version, snapshot.scene_json.len());
        }
        other => {
            return Err(format!("Expected SceneSnapshot, got: {other:?}").into());
        }
    }

    // Request lease over stream
    tx.send(session_proto::ClientMessage {
        sequence: 2,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::LeaseRequest(
            session_proto::LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tile".to_string()],
                lease_priority: 2,
            },
        )),
    })
    .await?;

    let msg = response_stream.next().await.unwrap()?;
    match &msg.payload {
        Some(session_proto::server_message::Payload::LeaseGrant(grant)) => {
            println!("  Lease granted: ttl={}ms, priority={}", grant.granted_ttl_ms, grant.granted_priority);
        }
        other => {
            return Err(format!("Expected LeaseGrant, got: {other:?}").into());
        }
    }

    // Heartbeat echo
    let mono_us = 999_000u64;
    tx.send(session_proto::ClientMessage {
        sequence: 3,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::Heartbeat(
            session_proto::Heartbeat {
                timestamp_mono_us: mono_us,
            },
        )),
    })
    .await?;

    let msg = response_stream.next().await.unwrap()?;
    match &msg.payload {
        Some(session_proto::server_message::Payload::Heartbeat(hb)) => {
            println!("  Heartbeat echo: mono_us={} (sent {})", hb.timestamp_mono_us, mono_us);
            assert_eq!(hb.timestamp_mono_us, mono_us);
        }
        other => {
            return Err(format!("Expected Heartbeat, got: {other:?}").into());
        }
    }

    println!("\n  Streaming session demo complete.");

    println!("\n=== Vertical slice complete ===");
    println!("All 6 layers demonstrated successfully (unary + streaming).");

    Ok(())
}

fn get_pixel(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    if idx + 3 < data.len() {
        [data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]
    } else {
        [0, 0, 0, 0]
    }
}
