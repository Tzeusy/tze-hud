//! # Dashboard Tile Agent — Exemplar gRPC Agent Reference
//!
//! Proves the raw tile API composes correctly by creating a polished,
//! interactive dashboard tile via the gRPC session stream.
//!
//! This exemplar uses the **raw tile API exclusively** — no zones, no widgets,
//! no design tokens.  The agent directly manages tiles, nodes, leases, and
//! input events over a single bidirectional gRPC stream.
//!
//! ## Phases
//!
//! **Phase 1 — Session Establishment (tasks.md §1.1–1.2)** ← this binary
//!   - gRPC client setup connecting to HudSession bidirectional stream
//!   - `SessionInit` → `SessionEstablished`: verify session_id, namespace
//!
//! Future phases (separate bead tasks):
//!   - Phase 2: Lease acquisition (tasks.md §2)
//!   - Phase 3: Resource upload (tasks.md §3)
//!   - Phase 4: Atomic tile creation batch (tasks.md §4)
//!   - Phase 5: Periodic content update (tasks.md §6)
//!   - Phase 6: Input callbacks — Refresh + Dismiss (tasks.md §8)
//!
//! ## Running
//!
//! Headless (production config — enforces capability governance):
//!   cargo run -p dashboard_tile_agent -- --headless
//!
//! Headless (dev mode — unrestricted caps, TEST/DEV ONLY):
//!   cargo run -p dashboard_tile_agent --features dev-mode -- --headless --dev
//!
//! ## Spec references
//!
//! - `session-protocol/spec.md` — SessionInit/SessionEstablished handshake
//! - `configuration/spec.md` §Capability Vocabulary — canonical capability names
//! - `openspec/changes/exemplar-dashboard-tile/tasks.md` §1 (1.1–1.2)
//! - `openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md`

use tze_hud_protocol::proto::session as session_proto;
#[allow(deprecated)]
use tze_hud_protocol::proto::session::hud_session_client::HudSessionClient;
use tze_hud_resource::{
    AgentBudget, CAPABILITY_UPLOAD_RESOURCE, ResourceStore, ResourceStoreConfig, ResourceType,
    UploadId, UploadStartRequest,
};
use tze_hud_runtime::HeadlessRuntime;
use tze_hud_runtime::headless::HeadlessConfig;
/// Embedded production config — capability governance always active by default.
/// File lives at `config/production.toml` relative to this source file.
const PRODUCTION_CONFIG: &str = include_str!("../config/production.toml");

/// Agent identifier registered in `config/production.toml`.
const AGENT_ID: &str = "dashboard-tile-agent";

/// Human-readable label shown in runtime admin panels.
const AGENT_DISPLAY_NAME: &str = "Dashboard Tile Agent";

/// Pre-shared key — must match the runtime's configured PSK.
const AGENT_PSK: &str = "dashboard-tile-key";

/// gRPC port the exemplar runtime listens on.
const GRPC_PORT: u16 = 50052;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");
    let dev_mode = args.iter().any(|a| a == "--dev");

    if headless {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        rt.block_on(run_headless(dev_mode))
    } else {
        println!("Dashboard tile agent: pass --headless to run in headless mode.");
        println!("  cargo run -p dashboard_tile_agent -- --headless");
        Ok(())
    }
}

fn now_wall_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(1) // clock before UNIX epoch: return 1 (non-zero per timing-model spec)
}

/// Run the headless runtime and execute the dashboard tile exemplar phases.
///
/// # Production path (default, `dev_mode = false`)
///
/// Loads `config/production.toml` (embedded at compile time).  Only
/// `dashboard-tile-agent` may connect, with the declared capabilities.
/// No Cargo feature flags required at build time for this path.
///
/// # Dev mode (opt-in, TEST/DEV ONLY)
///
/// Bypasses config governance: all capabilities granted to any agent.
/// Requires `--features dev-mode` at build time and `--dev` at runtime.
/// **NEVER use dev mode in production deployments.**
async fn run_headless(dev_mode: bool) -> Result<(), Box<dyn std::error::Error>> {
    if dev_mode {
        println!("=== Dashboard Tile Agent (DEV MODE — unrestricted caps, TEST/DEV ONLY) ===\n");
    } else {
        println!("=== Dashboard Tile Agent (production config) ===\n");
    }

    // ─── Initialize runtime ────────────────────────────────────────────────
    let config_toml = if dev_mode {
        None // dev-mode: requires --features dev-mode
    } else {
        Some(PRODUCTION_CONFIG.to_string())
    };

    let config = HeadlessConfig {
        width: 1920,
        height: 1080,
        grpc_port: GRPC_PORT,
        psk: AGENT_PSK.to_string(),
        config_toml,
    };

    let runtime = HeadlessRuntime::new(config).await?;
    let _server = runtime.start_grpc_server().await?;
    println!("Runtime initialized: 1920x1080, gRPC on [::1]:{GRPC_PORT}\n");

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 1: Session Establishment (tasks.md §1.1–1.2)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Connects a gRPC client to the HudSession bidirectional stream.
    // Sends SessionInit with agent identity, capability request, and subscription
    // declarations.  Reads SessionEstablished and verifies the session_id and
    // namespace assignment are non-empty (spec §SessionEstablished).
    //
    // Spec references:
    //   session-protocol/spec.md §Requirement: Session Establishment
    //   configuration/spec.md §Capability Vocabulary (canonical cap names)
    // ─────────────────────────────────────────────────────────────────────────
    println!("=== Phase 1: Session Establishment ===\n");

    let session_state = establish_session().await?;

    println!("  Phase 1 PASSED: session established.");
    println!("    namespace  = {}", session_state.namespace);
    println!(
        "    session_id = {} bytes (non-empty)",
        session_state.session_id.len()
    );
    println!(
        "    protocol   = v{}.{}",
        session_state.negotiated_protocol_version / 1000,
        session_state.negotiated_protocol_version % 1000,
    );

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 2: Lease Acquisition (tasks.md §2.1–2.2)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Sends LeaseRequest with:
    //   - ttl_ms = 60000 (60-second TTL per spec §Lease Request With AutoRenew)
    //   - capabilities = [create_tiles, modify_own_tiles] (spec-mandated scope)
    //   - lease_priority = 2 (default agent-owned band)
    //
    // Reads LeaseResponse and verifies granted = true and a 16-byte UUIDv7 lease_id.
    // Stores the lease_id for use in tile creation batches (Phase 4+).
    //
    // Spec references:
    //   lease-governance/spec.md §Requirement: Lease Request With AutoRenew
    //   openspec/changes/exemplar-dashboard-tile/tasks.md §2.1–2.2
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n=== Phase 2: Lease Acquisition ===\n");

    let lease_id = request_lease(GRPC_PORT, AGENT_PSK, AGENT_ID, AGENT_DISPLAY_NAME).await?;

    println!("  Phase 2 PASSED: lease granted.");
    println!("    lease_id   = {} bytes (UUIDv7)", lease_id.len());

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 3: Resource Upload (tasks.md §3.1)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Uploads a 48×48 PNG icon via the ResourceStore inline fast path.
    // Captures the returned BLAKE3 ResourceId (32 bytes) for use in StaticImageNode.
    //
    // The ResourceId is content-addressed: BLAKE3(raw_bytes) → unique 32-byte digest.
    // Any two uploads of identical bytes return the same ResourceId (deduplication).
    //
    // Spec references:
    //   resource-store/spec.md §Requirement: Resource Upload Before Tile Creation
    //   openspec/changes/exemplar-dashboard-tile/tasks.md §3.1
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n=== Phase 3: Resource Upload ===\n");

    let resource_id = upload_icon(AGENT_ID).await?;

    println!("  Phase 3 PASSED: icon uploaded.");
    println!("    resource_id = {} bytes (BLAKE3)", resource_id.len());

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 4: Atomic Tile Creation Batch (tasks.md §4.1–4.4)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Registers the uploaded resource with the runtime's scene graph, then
    // submits the full creation batch over gRPC:
    //
    //   Batch A (transactional): CreateTile (400×300 at (50,50), z_order=100)
    //   Batch B (atomic node batch): 1× AddNode(root=SolidColorNode bg)
    //                                5× AddNode(StaticImageNode, 2× TextMarkdown,
    //                                           2× HitRegionNode) as children of bg
    //                                + UpdateTileOpacity(1.0)
    //                                + UpdateTileInputMode(Passthrough)
    //
    // Atomicity contract: Batch B is all-or-nothing — if any node is invalid,
    // no nodes are committed (spec §Decision 2).
    //
    // The two-batch approach (rather than one batch with CreateTile + SetTileRoot)
    // is required because the tile_id is not known until Batch A's created_ids
    // are returned.  The atomicity property holds for Batch B itself.
    //
    // Spec references:
    //   openspec/changes/exemplar-dashboard-tile/tasks.md §4.1–4.4
    //   openspec/changes/exemplar-dashboard-tile/design.md §Decision 2
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n=== Phase 4: Atomic Tile Creation Batch ===\n");

    // Set up the runtime scene for tile creation:
    //   1. Create a default tab and make it active (CreateTile requires an active tab).
    //   2. Register the uploaded resource so StaticImageNode references are accepted
    //      (resource-store/spec.md §Resource Upload Before Tile Creation).
    {
        let resource_id_bytes: [u8; 32] = resource_id
            .as_slice()
            .try_into()
            .map_err(|_| "resource_id must be exactly 32 bytes")?;
        let scene_resource_id = tze_hud_scene::types::ResourceId::from_bytes(resource_id_bytes);
        let st = runtime.state.lock().await;
        let mut scene = st.scene.lock().await;
        let tab_id = scene.create_tab("Main", 0)?;
        scene.active_tab = Some(tab_id);
        scene.register_resource(scene_resource_id);
    }

    let tile_state = create_tile_batch(
        GRPC_PORT,
        AGENT_PSK,
        AGENT_ID,
        AGENT_DISPLAY_NAME,
        resource_id.clone(),
    )
    .await?;

    println!("  Phase 4 PASSED: tile created with all 6 nodes.");
    println!(
        "    tile_id   = {} bytes (UUIDv7)",
        tile_state.tile_id.len()
    );
    println!(
        "    node_ids  = {} nodes created",
        tile_state.node_ids.len()
    );

    // ─────────────────────────────────────────────────────────────────────────
    // PHASE 5: Periodic Content Update (tasks.md §6.1)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Runs a 5-second periodic SetTileRoot update that rebuilds the full 6-node
    // tree with an updated body TextMarkdownNode content.  Demonstrates the live
    // content refresh cycle described in tasks.md §6.
    //
    // Spec references:
    //   openspec/changes/exemplar-dashboard-tile/tasks.md §6.1–6.3
    // ─────────────────────────────────────────────────────────────────────────
    println!("\n=== Phase 5: Periodic Content Update (5s) ===\n");

    // Run one cycle of periodic content update immediately to demonstrate the
    // mechanism (rather than waiting 5 real seconds in the headless exemplar).
    let update_result = do_content_update(
        GRPC_PORT,
        AGENT_PSK,
        AGENT_ID,
        AGENT_DISPLAY_NAME,
        tile_state.tile_id.clone(),
        resource_id.clone(),
        1, // cycle #1
    )
    .await;

    match update_result {
        Ok(()) => println!("  Phase 5 PASSED: content update cycle 1 accepted."),
        Err(e) => println!("  Phase 5 FAILED: {e}"),
    }

    println!("\n=== Exemplar Phases 1–5 complete ===");
    println!(
        "  lease_id    = {} bytes (Phase 4)",
        tile_state.lease_id.len()
    );
    println!("  resource_id = {} bytes", resource_id.len());
    println!("  tile_id     = {} bytes", tile_state.tile_id.len());
    println!("Periodic update loop would run every 5s in production (see tasks.md §6.1).");
    println!("Input handling (§7) and agent callbacks (§8) verified by integration tests.");

    Ok(())
}

// ─── Phase 5: Periodic Content Update ────────────────────────────────────────

/// Build the updated body TextMarkdownNode content for content cycle `n`.
fn content_update_body(cycle: u32) -> String {
    format!("**Status**: operational\nUpdate cycle: {cycle}")
}

/// Submit a SetTileRoot batch over gRPC to rebuild the full 6-node tree with
/// updated body content.
///
/// # Tasks.md §6.1 — periodic SetTileRoot rebuild
///
/// The batch atomically replaces the tile root with:
///   1. SolidColorNode bg (new root via SetTileRoot)
///   2. StaticImageNode icon (AddNode, child of bg)
///   3. TextMarkdownNode header (AddNode, child of bg — unchanged)
///   4. TextMarkdownNode body (AddNode, child of bg — content changes)
///   5. HitRegionNode refresh (AddNode, child of bg)
///   6. HitRegionNode dismiss (AddNode, child of bg)
///
/// Spec references:
///   openspec/changes/exemplar-dashboard-tile/tasks.md §6.1–6.3
pub async fn do_content_update(
    port: u16,
    psk: &str,
    agent_id: &str,
    agent_display_name: &str,
    tile_id_bytes: Vec<u8>,
    resource_id_bytes: Vec<u8>,
    cycle: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    use tokio_stream::StreamExt as _;

    #[allow(deprecated)]
    let mut session_client = session_proto::hud_session_client::HudSessionClient::connect(format!(
        "http://[::1]:{port}"
    ))
    .await?;

    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let mut response_stream = session_client.session(stream).await?.into_inner();

    // Session handshake.
    let now_us = now_wall_us();
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_display_name.to_string(),
                pre_shared_key: psk.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            },
        )),
    })
    .await?;

    // Drain SessionEstablished + SceneSnapshot.
    for _ in 0..2 {
        response_stream
            .next()
            .await
            .ok_or("stream closed during handshake")??;
    }

    // Acquire a lease.
    tx.send(session_proto::ClientMessage {
        sequence: 2,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::LeaseRequest(
            session_proto::LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            },
        )),
    })
    .await?;

    let lease_id_bytes: Vec<u8> = loop {
        let msg = response_stream
            .next()
            .await
            .ok_or("stream closed before LeaseResponse")??;
        match msg.payload {
            Some(session_proto::server_message::Payload::LeaseStateChange(_)) => continue,
            Some(session_proto::server_message::Payload::LeaseResponse(resp)) => {
                if !resp.granted {
                    return Err(format!(
                        "LeaseResponse denied for content update: code={}, reason={}",
                        resp.deny_code, resp.deny_reason
                    )
                    .into());
                }
                break resp.lease_id;
            }
            other => {
                return Err(format!("Expected LeaseResponse, got: {other:?}").into());
            }
        }
    };

    // Build new root bg node.
    let bg_uuid = uuid::Uuid::now_v7();
    let bg_node_id_le = bg_uuid.to_bytes_le().to_vec();
    let bg_parent_id_be = bg_uuid.as_bytes().to_vec();

    let bg_node = tze_hud_protocol::proto::NodeProto {
        id: bg_node_id_le,
        data: Some(tze_hud_protocol::proto::node_proto::Data::SolidColor(
            tze_hud_protocol::proto::SolidColorNodeProto {
                color: Some(tze_hud_protocol::proto::Rgba {
                    r: 0.07,
                    g: 0.07,
                    b: 0.07,
                    a: 0.90,
                }),
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: TILE_W,
                    height: TILE_H,
                }),
            },
        )),
    };

    let icon_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::StaticImage(
            tze_hud_protocol::proto::StaticImageNodeProto {
                resource_id: resource_id_bytes,
                width: ICON_W,
                height: ICON_H,
                decoded_bytes: (ICON_W * ICON_H * 4) as u64,
                fit_mode: tze_hud_protocol::proto::ImageFitModeProto::ImageFitModeContain as i32,
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 16.0,
                    y: 16.0,
                    width: ICON_W as f32,
                    height: ICON_H as f32,
                }),
            },
        )),
    };

    let header_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::TextMarkdown(
            tze_hud_protocol::proto::TextMarkdownNodeProto {
                content: "**Dashboard Agent**".to_string(),
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 76.0,
                    y: 20.0,
                    width: 308.0,
                    height: 32.0,
                }),
                font_size_px: 18.0,
                color: Some(tze_hud_protocol::proto::Rgba {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                }),
                background: None,
            },
        )),
    };

    // Updated body node — content changes per cycle (tasks.md §6.1).
    let body_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::TextMarkdown(
            tze_hud_protocol::proto::TextMarkdownNodeProto {
                content: content_update_body(cycle),
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 16.0,
                    y: 72.0,
                    width: 368.0,
                    height: 180.0,
                }),
                font_size_px: 14.0,
                color: Some(tze_hud_protocol::proto::Rgba {
                    r: 0.78,
                    g: 0.78,
                    b: 0.78,
                    a: 1.0,
                }),
                background: None,
            },
        )),
    };

    let refresh_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::HitRegion(
            tze_hud_protocol::proto::HitRegionNodeProto {
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 16.0,
                    y: 256.0,
                    width: 176.0,
                    height: 36.0,
                }),
                interaction_id: "refresh-button".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
            },
        )),
    };

    let dismiss_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::HitRegion(
            tze_hud_protocol::proto::HitRegionNodeProto {
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 208.0,
                    y: 256.0,
                    width: 176.0,
                    height: 36.0,
                }),
                interaction_id: "dismiss-button".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
            },
        )),
    };

    // Content update batch: SetTileRoot (new bg) + 5× AddNode (children).
    // tasks.md §6.1: "full 6-node tree is rebuilt" — SetTileRoot replaces old
    // root and all its children, then AddNode rebuilds the subtree.
    let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();
    tx.send(session_proto::ClientMessage {
        sequence: 3,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::MutationBatch(
            session_proto::MutationBatch {
                batch_id: batch_id.clone(),
                lease_id: lease_id_bytes,
                mutations: vec![
                    // 1. SetTileRoot: new bg node replaces old tree atomically.
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::SetTileRoot(
                                tze_hud_protocol::proto::SetTileRootMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    node: Some(bg_node),
                                },
                            ),
                        ),
                    },
                    // 2. icon → child of new bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                            tze_hud_protocol::proto::AddNodeMutation {
                                tile_id: tile_id_bytes.clone(),
                                parent_id: bg_parent_id_be.clone(),
                                node: Some(icon_node),
                            },
                        )),
                    },
                    // 3. header → child of new bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                            tze_hud_protocol::proto::AddNodeMutation {
                                tile_id: tile_id_bytes.clone(),
                                parent_id: bg_parent_id_be.clone(),
                                node: Some(header_node),
                            },
                        )),
                    },
                    // 4. body (updated) → child of new bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                            tze_hud_protocol::proto::AddNodeMutation {
                                tile_id: tile_id_bytes.clone(),
                                parent_id: bg_parent_id_be.clone(),
                                node: Some(body_node),
                            },
                        )),
                    },
                    // 5. refresh button → child of new bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                            tze_hud_protocol::proto::AddNodeMutation {
                                tile_id: tile_id_bytes.clone(),
                                parent_id: bg_parent_id_be.clone(),
                                node: Some(refresh_node),
                            },
                        )),
                    },
                    // 6. dismiss button → child of new bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                            tze_hud_protocol::proto::AddNodeMutation {
                                tile_id: tile_id_bytes.clone(),
                                parent_id: bg_parent_id_be.clone(),
                                node: Some(dismiss_node),
                            },
                        )),
                    },
                ],
                timing: None,
            },
        )),
    })
    .await?;

    // Wait for MutationResult confirming the update.
    loop {
        let msg = response_stream
            .next()
            .await
            .ok_or("stream closed before MutationResult (content update)")??;
        match msg.payload {
            Some(session_proto::server_message::Payload::LeaseStateChange(_)) => continue,
            Some(session_proto::server_message::Payload::MutationResult(result)) => {
                if result.batch_id != batch_id {
                    return Err(format!(
                        "MutationResult batch_id mismatch for content update: \
                         expected {:?}, got {:?}",
                        batch_id, result.batch_id
                    )
                    .into());
                }
                if !result.accepted {
                    return Err(format!(
                        "Content update batch rejected: code={}, msg={}",
                        result.error_code, result.error_message
                    )
                    .into());
                }
                println!(
                    "  Content update cycle {cycle}: accepted=true, {} new nodes",
                    result.created_ids.len()
                );
                return Ok(());
            }
            other => {
                return Err(
                    format!("Expected MutationResult (content update), got: {other:?}").into(),
                );
            }
        }
    }
}

// ─── Phase 6: Agent Event Handler ─────────────────────────────────────────────

/// Classify an event batch received from the gRPC stream and determine what
/// action the agent should take.
///
/// # Tasks.md §8.1 — agent-side event handler
///
/// Receives an `EventBatch` and extracts `ClickEvent` or
/// `CommandInputEvent(ACTIVATE)` events, matching on `interaction_id` to
/// determine whether to refresh content or dismiss the tile.
///
/// Returns the list of `AgentAction` values the agent should perform.
/// The caller is responsible for executing each action (submitting mutations,
/// sending `LeaseRelease`, etc.).
///
/// Spec references:
///   openspec/changes/exemplar-dashboard-tile/tasks.md §8.1–8.3
pub fn handle_event_batch(batch: &tze_hud_protocol::proto::EventBatch) -> Vec<AgentAction> {
    use tze_hud_protocol::proto::input_envelope::Event;

    let mut actions = Vec::new();

    for envelope in &batch.events {
        match &envelope.event {
            // ── ClickEvent (pointer-driven activation) ──────────────────────
            //
            // tasks.md §8.1: extract ClickEvent, match on interaction_id.
            Some(Event::Click(click)) => {
                if click.interaction_id == "refresh-button" {
                    // tasks.md §8.2: Refresh triggers content update.
                    actions.push(AgentAction::RefreshContent);
                } else if click.interaction_id == "dismiss-button" {
                    // tasks.md §8.3: Dismiss triggers LeaseRelease.
                    actions.push(AgentAction::Dismiss);
                }
            }
            // ── CommandInputEvent(ACTIVATE) — keyboard/gamepad activation ──
            //
            // tasks.md §8.1: extract CommandInputEvent, check action == ACTIVATE.
            // tasks.md §8.5: ACTIVATE on focused Dismiss → AgentAction::Dismiss.
            Some(Event::CommandInput(cmd)) => {
                let is_activate =
                    cmd.action == tze_hud_protocol::proto::CommandAction::Activate as i32;
                if is_activate {
                    if cmd.interaction_id == "refresh-button" {
                        actions.push(AgentAction::RefreshContent);
                    } else if cmd.interaction_id == "dismiss-button" {
                        actions.push(AgentAction::Dismiss);
                    }
                }
            }
            _ => {} // Other events (pointer move, focus, etc.) — not handled here.
        }
    }

    actions
}

/// Action the agent should take in response to an input event.
///
/// Returned by [`handle_event_batch`].
///
/// Spec references:
///   openspec/changes/exemplar-dashboard-tile/tasks.md §8.2–8.3
#[derive(Debug, PartialEq, Eq)]
pub enum AgentAction {
    /// Trigger an immediate content update (SetTileRoot batch).
    /// tasks.md §8.2: "Refresh triggers content update."
    RefreshContent,
    /// Send LeaseRelease and expect tile removal from scene.
    /// tasks.md §8.3: "Dismiss triggers LeaseRelease."
    Dismiss,
}

/// Result of a successful session establishment handshake.
///
/// Returned by [`establish_session`] after `SessionEstablished` is received and
/// validated.  Carries the state needed by subsequent phases (lease, mutations).
pub struct SessionState {
    /// Opaque session identifier assigned by the runtime (UUIDv7, 16 bytes).
    /// Non-empty per spec §SessionEstablished field 1.
    pub session_id: Vec<u8>,

    /// Agent's namespace in the scene graph (RFC 0001 §1.2).
    /// Scopes all scene objects the agent creates.  Non-empty per spec.
    pub namespace: String,

    /// Capabilities actually granted after intersecting the requested set
    /// with the agent's authorization policy.
    pub granted_capabilities: Vec<String>,

    /// Resume token for reconnecting within the grace period.
    pub resume_token: Vec<u8>,

    /// Heartbeat interval the runtime expects from this client (ms).
    pub heartbeat_interval_ms: u64,

    /// Negotiated protocol version: `major * 1000 + minor`.
    pub negotiated_protocol_version: u32,

    /// Active subscription categories confirmed by the runtime.
    pub active_subscriptions: Vec<String>,
}

/// Connect to the HudSession gRPC stream and perform the session handshake.
///
/// # Session handshake (tasks.md §1.1–1.2)
///
/// 1. Opens a bidirectional stream to `HudSession.Session`.
/// 2. Sends `SessionInit` with:
///    - `agent_id` = "dashboard-tile-agent"
///    - `requested_capabilities` = [create_tiles, modify_own_tiles,
///      access_input_events]
///    - `initial_subscriptions` = [LEASE_CHANGES]
///    - `min_protocol_version` / `max_protocol_version` = 1000–1001
/// 3. Reads `SessionEstablished` from the server.
/// 4. Verifies `session_id` is non-empty (spec §SessionEstablished field 1).
/// 5. Verifies `namespace` is non-empty (spec §SessionEstablished field 2).
/// 6. Skips the `SceneSnapshot` that immediately follows per spec.
/// 7. Returns [`SessionState`] with all negotiated parameters.
///
/// # Errors
///
/// Returns an error if the server sends `SessionError`, the stream closes
/// unexpectedly, or a required verification assertion fails.
///
/// # Spec references
///
/// - session-protocol/spec.md §Requirement: Session Establishment
/// - configuration/spec.md §Capability Vocabulary (lines 149-164)
/// - openspec/changes/exemplar-dashboard-tile/tasks.md §1.1, §1.2
pub async fn establish_session() -> Result<SessionState, Box<dyn std::error::Error>> {
    establish_session_with(GRPC_PORT, AGENT_PSK, AGENT_ID, AGENT_DISPLAY_NAME).await
}

/// Parameterized session-establishment helper used by tests and the public API.
///
/// Accepts connection parameters so tests can spin up isolated runtimes on
/// ephemeral ports without conflicting with production constants.
async fn establish_session_with(
    port: u16,
    psk: &str,
    agent_id: &str,
    agent_display_name: &str,
) -> Result<SessionState, Box<dyn std::error::Error>> {
    use tokio_stream::StreamExt as _;

    // ── 1. Connect gRPC client to HudSession ──────────────────────────────
    //
    // HudSessionClient wraps a single bidirectional `Session` RPC.
    // All session traffic — handshake, mutations, events, heartbeats,
    // lease management — flows over this one stream per agent.
    #[allow(deprecated)]
    let mut session_client = HudSessionClient::connect(format!("http://[::1]:{port}")).await?;

    // Channel for client → server messages.  Buffer = 64 gives the agent
    // headroom during bursts (e.g., mutation batches) without unbounded growth.
    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    // Open the bidirectional stream.  `session()` returns a Response wrapping
    // the server message stream; `.into_inner()` unwraps to the raw stream.
    let mut response_stream = session_client.session(stream).await?.into_inner();

    // ── 2. Send SessionInit ────────────────────────────────────────────────
    //
    // SessionInit is the first message the client MUST send on a new connection.
    // The runtime closes the stream if it does not arrive within the handshake
    // timeout (default 5000 ms).
    //
    // Capability vocabulary (configuration/spec.md §Capability Vocabulary):
    //   - "create_tiles"         — create tiles in a leased area
    //   - "modify_own_tiles"     — mutate tiles owned by this agent
    //   - "access_input_events"  — receive pointer / keyboard events
    //
    // LEASE_CHANGES is a mandatory subscription category (always active).
    // Listing it in `initial_subscriptions` is spec-compliant and explicit
    // about the agent's intent (session-protocol/spec.md §Subscriptions).
    let now_us = now_wall_us();
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_display_name.to_string(),
                pre_shared_key: psk.to_string(),
                // Canonical v1 capability names — non-canonical names are
                // rejected with CONFIG_UNKNOWN_CAPABILITY.
                requested_capabilities: vec![
                    "create_tiles".to_string(),        // create tiles in leased area
                    "modify_own_tiles".to_string(),    // mutate tiles owned by this agent
                    "access_input_events".to_string(), // receive pointer/keyboard events
                ],
                // LEASE_CHANGES is mandatory; listing it explicitly is idiomatic.
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(), // new session, no prior resume token
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000, // v1.0
                max_protocol_version: 1001, // v1.1
                auth_credential: None,
            },
        )),
    })
    .await?;

    // ── 3. Receive SessionEstablished ──────────────────────────────────────
    //
    // The runtime processes SessionInit and responds with either
    // SessionEstablished (success) or SessionError (auth failure, version
    // mismatch, duplicate agent_id, etc.).
    let msg = response_stream
        .next()
        .await
        .ok_or("stream closed before SessionEstablished")??;

    let established = match msg.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(ref e)) => {
            // ── 4 & 5. Verify session_id and namespace (tasks.md §1.2) ────
            //
            // spec §SessionEstablished:
            //   field 1 (session_id): opaque UUIDv7, 16 bytes — MUST be non-empty
            //   field 2 (namespace):  agent's scene namespace   — MUST be non-empty
            if e.session_id.is_empty() {
                return Err(
                    "session_id MUST be non-empty (spec §SessionEstablished field 1)".into(),
                );
            }
            if e.namespace.is_empty() {
                return Err(
                    "namespace MUST be non-empty (spec §SessionEstablished field 2)".into(),
                );
            }

            println!(
                "  session_id           = {} bytes (UUIDv7)",
                e.session_id.len()
            );
            println!("  namespace            = {}", e.namespace);
            println!("  heartbeat_ms         = {}", e.heartbeat_interval_ms);
            println!("  granted_capabilities = {:?}", e.granted_capabilities);
            println!("  active_subscriptions = {:?}", e.active_subscriptions);
            println!("  clock_skew           = {}us", e.estimated_skew_us);
            println!(
                "  protocol_version     = v{}.{}",
                e.negotiated_protocol_version / 1000,
                e.negotiated_protocol_version % 1000,
            );

            e.clone()
        }
        Some(session_proto::server_message::Payload::SessionError(ref err)) => {
            return Err(format!(
                "Session rejected by runtime: code={}, message={}, hint={}",
                err.code, err.message, err.hint
            )
            .into());
        }
        other => {
            return Err(format!(
                "Expected SessionEstablished as first server message, got: {other:?}"
            )
            .into());
        }
    };

    // ── 6. Skip SceneSnapshot ──────────────────────────────────────────────
    //
    // Per spec §Session Establishment: "Followed immediately by a SceneSnapshot."
    // The exemplar does not consume the snapshot at this phase; drain it so the
    // stream is ready for subsequent phase messages.
    let snapshot_msg = response_stream
        .next()
        .await
        .ok_or("stream closed before SceneSnapshot")??;
    match snapshot_msg.payload {
        Some(session_proto::server_message::Payload::SceneSnapshot(ref snap)) => {
            println!(
                "  SceneSnapshot received: seq={}, json_len={}",
                snap.sequence,
                snap.snapshot_json.len()
            );
        }
        other => {
            return Err(
                format!("Expected SceneSnapshot after SessionEstablished, got: {other:?}").into(),
            );
        }
    }

    // ── 7. Return SessionState ─────────────────────────────────────────────
    Ok(SessionState {
        session_id: established.session_id.clone(),
        namespace: established.namespace.clone(),
        granted_capabilities: established.granted_capabilities.clone(),
        resume_token: established.resume_token.clone(),
        heartbeat_interval_ms: established.heartbeat_interval_ms,
        negotiated_protocol_version: established.negotiated_protocol_version,
        active_subscriptions: established.active_subscriptions.clone(),
    })
}

// ─── Phase 2: Lease Acquisition ──────────────────────────────────────────────

/// Request a lease on a new gRPC session and return the granted `lease_id` bytes.
///
/// # Lease request (tasks.md §2.1–2.2)
///
/// 1. Opens a new gRPC session by duplicating the SessionInit handshake inline
///    (not via `establish_session_with`) so that each phase remains independently
///    testable without coupling to the Phase 1 session state.
/// 2. Sends `LeaseRequest` with:
///    - `ttl_ms = 60000` (spec §Lease Request With AutoRenew: 60-second TTL)
///    - `capabilities = ["create_tiles", "modify_own_tiles"]`
///    - `lease_priority = 2` (default agent-owned band)
/// 3. Reads the next non-state-change message and expects `LeaseResponse`.
/// 4. Verifies `granted = true` (spec §Scenario: Lease granted with requested parameters).
/// 5. Returns the 16-byte UUIDv7 `lease_id` for use in subsequent MutationBatch calls.
///
/// # Phase 4 integration note
///
/// This function opens a short-lived session and drops it after the lease is granted.
/// In the current standalone Phase 2 test this is sufficient to verify the lease protocol.
/// Phase 4 (tile creation batch, tasks.md §4) MUST reuse the established session stream
/// from Phase 1 rather than calling this function, so that the lease remains attached to
/// the live session used by MutationBatch calls and is not orphaned on session disconnect.
///
/// # Spec references
///
/// - lease-governance/spec.md §Requirement: Lease Request With AutoRenew
/// - openspec/changes/exemplar-dashboard-tile/tasks.md §2.1–2.2
pub async fn request_lease(
    port: u16,
    psk: &str,
    agent_id: &str,
    agent_display_name: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use tokio_stream::StreamExt as _;

    // ── 1. Establish a fresh gRPC session ──────────────────────────────────
    //
    // We open a new session rather than sharing one from Phase 1 so that
    // each phase is independently testable in unit tests without coupling.
    #[allow(deprecated)]
    let mut session_client = session_proto::hud_session_client::HudSessionClient::connect(format!(
        "http://[::1]:{port}"
    ))
    .await?;

    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let mut response_stream = session_client.session(stream).await?.into_inner();

    // Send SessionInit.
    let now_us = now_wall_us();
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_display_name.to_string(),
                pre_shared_key: psk.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                    "access_input_events".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            },
        )),
    })
    .await?;

    // Drain SessionEstablished.
    let first = response_stream
        .next()
        .await
        .ok_or("stream closed before SessionEstablished")??;
    match first.payload {
        Some(session_proto::server_message::Payload::SessionEstablished(_)) => {}
        other => {
            return Err(format!(
                "Expected SessionEstablished as first server message, got: {other:?}"
            )
            .into());
        }
    }

    // Drain SceneSnapshot.
    let second = response_stream
        .next()
        .await
        .ok_or("stream closed before SceneSnapshot")??;
    match second.payload {
        Some(session_proto::server_message::Payload::SceneSnapshot(_)) => {}
        other => {
            return Err(
                format!("Expected SceneSnapshot after SessionEstablished, got: {other:?}").into(),
            );
        }
    }

    // ── 2. Send LeaseRequest (tasks.md §2.1) ──────────────────────────────
    //
    // Spec §Requirement: Lease Request With AutoRenew:
    //   ttl_ms = 60000, capabilities = [create_tiles, modify_own_tiles], lease_priority = 2.
    //
    // Note: renewal policy (AutoRenew) and resource budgets are server-side concerns;
    // they are not fields on the LeaseRequest proto.
    tx.send(session_proto::ClientMessage {
        sequence: 2,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::LeaseRequest(
            session_proto::LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            },
        )),
    })
    .await?;

    // ── 3–4. Receive LeaseResponse and verify granted = true (tasks.md §2.2) ──
    //
    // Drain any interleaved LeaseStateChange messages (REQUESTED→ACTIVE)
    // before asserting the LeaseResponse, consistent with the test-suite pattern.
    loop {
        let msg = response_stream
            .next()
            .await
            .ok_or("stream closed before LeaseResponse")??;
        match msg.payload {
            Some(session_proto::server_message::Payload::LeaseStateChange(_)) => {
                // Drain — not the response we're waiting for.
                continue;
            }
            Some(session_proto::server_message::Payload::LeaseResponse(resp)) => {
                if !resp.granted {
                    return Err(format!(
                        "LeaseResponse denied: code={}, reason={}",
                        resp.deny_code, resp.deny_reason
                    )
                    .into());
                }
                if resp.lease_id.len() != 16 {
                    return Err(format!(
                        "LeaseResponse granted but lease_id is {} bytes (must be 16-byte UUIDv7)",
                        resp.lease_id.len()
                    )
                    .into());
                }
                println!(
                    "  LeaseResponse: granted=true, ttl={}ms",
                    resp.granted_ttl_ms
                );
                println!("    granted_capabilities = {:?}", resp.granted_capabilities);
                println!("    granted_priority     = {}", resp.granted_priority);
                // ── 5. Return lease_id ────────────────────────────────────
                return Ok(resp.lease_id);
            }
            other => {
                return Err(format!("Expected LeaseResponse, got: {other:?}").into());
            }
        }
    }
}

// ─── Phase 3: Resource Upload ─────────────────────────────────────────────────

/// Icon dimensions per spec §Dashboard Tile Composition node 2.
const ICON_W: u32 = 48;
const ICON_H: u32 = 48;

/// Upload the dashboard tile's 48×48 PNG icon and return the BLAKE3 `ResourceId` bytes.
///
/// # Resource upload (tasks.md §3.1)
///
/// Uses the `ResourceStore` inline fast path (≤ 64 KiB per RFC 0011 §3):
/// 1. Generates a 48×48 solid-color PNG in memory (no filesystem I/O).
/// 2. Computes the BLAKE3 content hash as the expected `ResourceId`.
/// 3. Calls `ResourceStore::handle_upload_start` with `inline_data`.
/// 4. Returns the 32-byte `ResourceId` bytes (the BLAKE3 digest of the raw PNG).
///
/// The `ResourceId` is content-addressed: identical bytes always yield the same id
/// (deduplication contract, RFC 0011 §4).  The agent MUST pass this id in the
/// `StaticImageNode.resource_id` field of the tile creation batch (Phase 4).
///
/// # Phase 4 integration note
///
/// This function uploads into a standalone in-memory `ResourceStore` to prove the
/// resource upload protocol and content-addressed identity (tasks.md §3.1).
/// Phase 4 (tile creation batch, tasks.md §4) MUST upload through the runtime-owned
/// path (e.g. the session `upload_resource` RPC) so that the resource becomes
/// registered in the runtime's `SceneGraph::registered_resources` set.  Without that
/// registration, `add_node_to_tile_checked` / `set_tile_root_checked` will reject the
/// `StaticImageNode` reference with `ResourceNotFound`.
///
/// # Spec references
///
/// - resource-store/spec.md §Requirement: Resource Upload Before Tile Creation
/// - resource-store/spec.md §Requirement: Content-Addressed Resource Identity
/// - openspec/changes/exemplar-dashboard-tile/tasks.md §3.1
pub async fn upload_icon(agent_namespace: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // ── 1. Generate a 48×48 solid-color PNG ───────────────────────────────
    //
    // Representative placeholder icon.  In production an agent would supply
    // its own brand asset here.  The test fixture uses a solid steel-blue fill.
    let png_bytes: Vec<u8> = {
        use image::{ImageBuffer, Rgb};
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(ICON_W, ICON_H, |_, _| Rgb([70u8, 130, 180]));
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| format!("PNG encoding failed: {e}"))?;
        buf
    };

    // ── 2. Compute expected BLAKE3 ResourceId ─────────────────────────────
    let resource_id = tze_hud_resource::types::ResourceId::from_content(&png_bytes);

    // ── 3. Upload via ResourceStore inline fast path ──────────────────────
    let store = ResourceStore::new(ResourceStoreConfig::default());
    let upload_id = UploadId::from_bytes(uuid::Uuid::now_v7().into_bytes());

    let result = store
        .handle_upload_start(UploadStartRequest {
            agent_namespace: agent_namespace.to_string(),
            // The `upload_resource` capability is required by the store.
            agent_capabilities: vec![CAPABILITY_UPLOAD_RESOURCE.to_string()],
            agent_budget: AgentBudget {
                texture_bytes_total_limit: 0, // 0 = unlimited
                texture_bytes_total_used: 0,
            },
            upload_id,
            resource_type: ResourceType::ImagePng,
            expected_hash: *resource_id.as_bytes(),
            total_size: png_bytes.len(),
            inline_data: png_bytes,
            width: ICON_W,
            height: ICON_H,
        })
        .await
        .map_err(|e| format!("ResourceStore upload_start failed: {e:?}"))?
        .ok_or("inline upload must return ResourceStored immediately")?;

    // ── 4. Return 32-byte BLAKE3 ResourceId ──────────────────────────────
    Ok(result.resource_id.as_bytes().to_vec())
}

// ─── Phase 4: Atomic Tile Creation Batch ─────────────────────────────────────

/// Result of a successful tile creation batch (Phase 4).
///
/// Returned by [`create_tile_batch`] after both the CreateTile batch and the
/// 6-node batch have been accepted and applied.
pub struct TileCreationState {
    /// The tile's UUIDv7 SceneId bytes (16 bytes) from `MutationResult.created_ids[0]`.
    pub tile_id: Vec<u8>,
    /// The lease's UUIDv7 bytes (16 bytes) used for the tile creation.
    pub lease_id: Vec<u8>,
    /// The 6 node SceneId bytes from the second batch's `created_ids`.
    pub node_ids: Vec<Vec<u8>>,
}

/// Dashboard tile geometry per spec §Decision 6 / §Dashboard Tile Composition.
const TILE_X: f32 = 50.0;
const TILE_Y: f32 = 50.0;
const TILE_W: f32 = 400.0;
const TILE_H: f32 = 300.0;
/// Agent-owned band z_order (< ZONE_TILE_Z_MIN = 0x8000_0000).
const TILE_Z_ORDER: u32 = 100;

/// Submit the atomic tile creation batch over gRPC and return the created state.
///
/// # Two-batch approach (tasks.md §4.1)
///
/// Because the tile_id is not known until Batch A is committed, the creation is
/// split into two atomic batches:
///
/// **Batch A — CreateTile:**
/// - `CreateTile { bounds: (50,50, 400×300), z_order: 100 }`
/// - Returns `tile_id` from `MutationResult.created_ids[0]`.
///
/// **Batch B — 6-node tree + opacity + input_mode (atomic):**
/// - `AddNode(parent=None, SolidColorNode)` → bg becomes tile root.
/// - `AddNode(parent=bg, StaticImageNode)` → icon (48×48, resource_id).
/// - `AddNode(parent=bg, TextMarkdownNode)` → header ("**Dashboard Agent**").
/// - `AddNode(parent=bg, TextMarkdownNode)` → body (live stats placeholder).
/// - `AddNode(parent=bg, HitRegionNode)` → Refresh button.
/// - `AddNode(parent=bg, HitRegionNode)` → Dismiss button.
/// - `UpdateTileOpacity { opacity: 1.0 }`.
/// - `UpdateTileInputMode { input_mode: Passthrough }`.
///
/// Batch B is all-or-nothing: if any mutation fails (e.g., ResourceNotFound),
/// no nodes are committed and the tile remains root-less (tasks.md §4.4).
///
/// # Spec references
///
/// - openspec/changes/exemplar-dashboard-tile/tasks.md §4.1–4.2
/// - openspec/changes/exemplar-dashboard-tile/design.md §Decision 2 (atomicity)
/// - openspec/changes/exemplar-dashboard-tile/design.md §Decision 6 (geometry)
pub async fn create_tile_batch(
    port: u16,
    psk: &str,
    agent_id: &str,
    agent_display_name: &str,
    resource_id_bytes: Vec<u8>,
) -> Result<TileCreationState, Box<dyn std::error::Error>> {
    use tokio_stream::StreamExt as _;

    // ── 1. Open a new gRPC session ─────────────────────────────────────────
    #[allow(deprecated)]
    let mut session_client = session_proto::hud_session_client::HudSessionClient::connect(format!(
        "http://[::1]:{port}"
    ))
    .await?;

    let (tx, rx) = tokio::sync::mpsc::channel::<session_proto::ClientMessage>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let mut response_stream = session_client.session(stream).await?.into_inner();

    // ── 2. Session handshake ───────────────────────────────────────────────
    let now_us = now_wall_us();
    tx.send(session_proto::ClientMessage {
        sequence: 1,
        timestamp_wall_us: now_us,
        payload: Some(session_proto::client_message::Payload::SessionInit(
            session_proto::SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_display_name.to_string(),
                pre_shared_key: psk.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            },
        )),
    })
    .await?;

    // Drain SessionEstablished + SceneSnapshot.
    for _ in 0..2 {
        response_stream
            .next()
            .await
            .ok_or("stream closed during handshake")??;
    }

    // ── 3. Request lease ───────────────────────────────────────────────────
    tx.send(session_proto::ClientMessage {
        sequence: 2,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::LeaseRequest(
            session_proto::LeaseRequest {
                ttl_ms: 60_000,
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                lease_priority: 2,
            },
        )),
    })
    .await?;

    // Drain lease state changes; expect LeaseResponse.
    let lease_id_bytes: Vec<u8> = loop {
        let msg = response_stream
            .next()
            .await
            .ok_or("stream closed before LeaseResponse")??;
        match msg.payload {
            Some(session_proto::server_message::Payload::LeaseStateChange(_)) => continue,
            Some(session_proto::server_message::Payload::LeaseResponse(resp)) => {
                if !resp.granted {
                    return Err(format!(
                        "LeaseResponse denied: code={}, reason={}",
                        resp.deny_code, resp.deny_reason
                    )
                    .into());
                }
                if resp.lease_id.len() != 16 {
                    return Err(format!(
                        "lease_id must be 16 bytes (UUIDv7), got {} bytes",
                        resp.lease_id.len()
                    )
                    .into());
                }
                println!("  Lease granted: {} bytes", resp.lease_id.len());
                break resp.lease_id;
            }
            other => {
                return Err(format!("Expected LeaseResponse, got: {other:?}").into());
            }
        }
    };

    // ── 4. Batch A: CreateTile (400×300 at (50,50), z_order=100) ──────────
    //
    // tasks.md §4.1: CreateTile with spec-mandated geometry.
    // Note: opacity and input_mode cannot be set here; they require separate
    // mutations (UpdateTileOpacity, UpdateTileInputMode) per proto design.
    let batch_a_id = uuid::Uuid::now_v7().as_bytes().to_vec();
    tx.send(session_proto::ClientMessage {
        sequence: 3,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::MutationBatch(
            session_proto::MutationBatch {
                batch_id: batch_a_id.clone(),
                lease_id: lease_id_bytes.clone(),
                mutations: vec![tze_hud_protocol::proto::MutationProto {
                    mutation: Some(
                        tze_hud_protocol::proto::mutation_proto::Mutation::CreateTile(
                            tze_hud_protocol::proto::CreateTileMutation {
                                tab_id: vec![], // empty = server infers active tab
                                bounds: Some(tze_hud_protocol::proto::Rect {
                                    x: TILE_X,
                                    y: TILE_Y,
                                    width: TILE_W,
                                    height: TILE_H,
                                }),
                                z_order: TILE_Z_ORDER,
                            },
                        ),
                    ),
                }],
                timing: None,
            },
        )),
    })
    .await?;

    // Drain any LeaseStateChange; expect MutationResult for Batch A.
    // tasks.md §4.2: verify echoed batch_id and 16-byte tile_id before proceeding.
    let tile_id_bytes: Vec<u8> = loop {
        let msg = response_stream
            .next()
            .await
            .ok_or("stream closed before MutationResult (CreateTile)")??;
        match msg.payload {
            Some(session_proto::server_message::Payload::LeaseStateChange(_)) => continue,
            Some(session_proto::server_message::Payload::MutationResult(result)) => {
                if result.batch_id != batch_a_id {
                    return Err(format!(
                        "MutationResult batch_id mismatch for Batch A: expected {:?}, got {:?}",
                        batch_a_id, result.batch_id
                    )
                    .into());
                }
                if !result.accepted {
                    return Err(format!(
                        "CreateTile batch rejected: code={}, msg={}",
                        result.error_code, result.error_message
                    )
                    .into());
                }
                if result.created_ids.is_empty() {
                    return Err("MutationResult for CreateTile must include created_ids".into());
                }
                let id = result.created_ids[0].clone();
                if id.len() != 16 {
                    return Err(format!(
                        "tile_id must be 16 bytes (UUIDv7 SceneId), got {} bytes — tasks.md §4.2",
                        id.len()
                    )
                    .into());
                }
                println!(
                    "  Batch A (CreateTile): accepted=true, tile_id={} bytes",
                    id.len()
                );
                break id;
            }
            other => {
                return Err(format!("Expected MutationResult (CreateTile), got: {other:?}").into());
            }
        }
    };

    // ── 5. Batch B: 6-node tree + UpdateTileOpacity + UpdateTileInputMode ──
    //
    // tasks.md §4.1: atomic batch with bg root + 5 children + opacity + input_mode.
    //
    // Node layout per spec §Dashboard Tile Composition:
    //   1. SolidColorNode bg  — root (parent=None → becomes tile root)
    //   2. StaticImageNode     — icon 48×48 at (16,16)
    //   3. TextMarkdownNode    — header at (76,20)
    //   4. TextMarkdownNode    — body at (16,72)
    //   5. HitRegionNode       — Refresh at (16,256)
    //   6. HitRegionNode       — Dismiss at (208,256)
    //
    // Painter's model: bg renders first (background), then children in tree
    // order (icon behind header, header behind body, body behind buttons).

    // Build node proto messages.
    //
    // Node IDs are encoded differently depending on the field:
    //   NodeProto.id          → 16 bytes, little-endian (uuid::Uuid::to_bytes_le)
    //   AddNodeMutation.parent_id → 16 bytes, big-endian RFC 4122 (uuid::Uuid::as_bytes)
    //
    // We keep both encodings of bg_id to avoid mixing them up.
    let bg_uuid = uuid::Uuid::now_v7();
    let bg_node_id_le = bg_uuid.to_bytes_le().to_vec(); // for NodeProto.id
    let bg_parent_id_be = bg_uuid.as_bytes().to_vec(); // for AddNodeMutation.parent_id

    let bg_node = tze_hud_protocol::proto::NodeProto {
        id: bg_node_id_le,
        data: Some(tze_hud_protocol::proto::node_proto::Data::SolidColor(
            tze_hud_protocol::proto::SolidColorNodeProto {
                color: Some(tze_hud_protocol::proto::Rgba {
                    r: 0.07,
                    g: 0.07,
                    b: 0.07,
                    a: 0.90,
                }),
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: TILE_W,
                    height: TILE_H,
                }),
            },
        )),
    };

    let icon_node = tze_hud_protocol::proto::NodeProto {
        id: vec![], // empty = server assigns a fresh UUIDv7
        data: Some(tze_hud_protocol::proto::node_proto::Data::StaticImage(
            tze_hud_protocol::proto::StaticImageNodeProto {
                resource_id: resource_id_bytes,
                width: ICON_W,
                height: ICON_H,
                decoded_bytes: (ICON_W * ICON_H * 4) as u64,
                fit_mode: tze_hud_protocol::proto::ImageFitModeProto::ImageFitModeContain as i32,
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 16.0,
                    y: 16.0,
                    width: ICON_W as f32,
                    height: ICON_H as f32,
                }),
            },
        )),
    };

    let header_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::TextMarkdown(
            tze_hud_protocol::proto::TextMarkdownNodeProto {
                content: "**Dashboard Agent**".to_string(),
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 76.0,
                    y: 20.0,
                    width: 308.0,
                    height: 32.0,
                }),
                font_size_px: 18.0,
                color: Some(tze_hud_protocol::proto::Rgba {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                }),
                background: None,
            },
        )),
    };

    let body_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::TextMarkdown(
            tze_hud_protocol::proto::TextMarkdownNodeProto {
                content: "**Status**: operational\nUptime: 0s".to_string(),
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 16.0,
                    y: 72.0,
                    width: 368.0,
                    height: 180.0,
                }),
                font_size_px: 14.0,
                color: Some(tze_hud_protocol::proto::Rgba {
                    r: 0.78,
                    g: 0.78,
                    b: 0.78,
                    a: 1.0,
                }),
                background: None,
            },
        )),
    };

    let refresh_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::HitRegion(
            tze_hud_protocol::proto::HitRegionNodeProto {
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 16.0,
                    y: 256.0,
                    width: 176.0,
                    height: 36.0,
                }),
                interaction_id: "refresh-button".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
            },
        )),
    };

    let dismiss_node = tze_hud_protocol::proto::NodeProto {
        id: vec![],
        data: Some(tze_hud_protocol::proto::node_proto::Data::HitRegion(
            tze_hud_protocol::proto::HitRegionNodeProto {
                bounds: Some(tze_hud_protocol::proto::Rect {
                    x: 208.0,
                    y: 256.0,
                    width: 176.0,
                    height: 36.0,
                }),
                interaction_id: "dismiss-button".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
            },
        )),
    };

    // Use the big-endian UUID bytes for parent_id in AddNodeMutation
    // (wire contract: big-endian RFC 4122, matching bytes_to_scene_id in session_server).
    let batch_b_id = uuid::Uuid::now_v7().as_bytes().to_vec();
    tx.send(session_proto::ClientMessage {
        sequence: 4,
        timestamp_wall_us: now_wall_us(),
        payload: Some(session_proto::client_message::Payload::MutationBatch(
            session_proto::MutationBatch {
                batch_id: batch_b_id.clone(),
                lease_id: lease_id_bytes.clone(),
                mutations: vec![
                    // 1. bg node → becomes tile root (parent=None, tile has no root)
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                                tze_hud_protocol::proto::AddNodeMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    parent_id: vec![],    // empty = root
                                    node: Some(bg_node),
                                },
                            ),
                        ),
                    },
                    // 2. icon → child of bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                                tze_hud_protocol::proto::AddNodeMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    parent_id: bg_parent_id_be.clone(),
                                    node: Some(icon_node),
                                },
                            ),
                        ),
                    },
                    // 3. header → child of bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                                tze_hud_protocol::proto::AddNodeMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    parent_id: bg_parent_id_be.clone(),
                                    node: Some(header_node),
                                },
                            ),
                        ),
                    },
                    // 4. body → child of bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                                tze_hud_protocol::proto::AddNodeMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    parent_id: bg_parent_id_be.clone(),
                                    node: Some(body_node),
                                },
                            ),
                        ),
                    },
                    // 5. refresh button → child of bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                                tze_hud_protocol::proto::AddNodeMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    parent_id: bg_parent_id_be.clone(),
                                    node: Some(refresh_node),
                                },
                            ),
                        ),
                    },
                    // 6. dismiss button → child of bg
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                                tze_hud_protocol::proto::AddNodeMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    parent_id: bg_parent_id_be.clone(),
                                    node: Some(dismiss_node),
                                },
                            ),
                        ),
                    },
                    // UpdateTileOpacity — separate from CreateTile per spec
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::UpdateTileOpacity(
                                tze_hud_protocol::proto::UpdateTileOpacityMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    opacity: 1.0,
                                },
                            ),
                        ),
                    },
                    // UpdateTileInputMode — separate from CreateTile per spec
                    tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::UpdateTileInputMode(
                                tze_hud_protocol::proto::UpdateTileInputModeMutation {
                                    tile_id: tile_id_bytes.clone(),
                                    input_mode:
                                        tze_hud_protocol::proto::TileInputModeProto::TileInputModePassthrough
                                            as i32,
                                },
                            ),
                        ),
                    },
                ],
                timing: None,
            },
        )),
    })
    .await?;

    // Drain any interleaved state messages; expect MutationResult for Batch B.
    // tasks.md §4.2: verify echoed batch_id and exactly 6 created_ids (1 bg + 5 children).
    let node_ids: Vec<Vec<u8>> = loop {
        let msg = response_stream
            .next()
            .await
            .ok_or("stream closed before MutationResult (node batch)")??;
        match msg.payload {
            Some(session_proto::server_message::Payload::LeaseStateChange(_)) => continue,
            Some(session_proto::server_message::Payload::MutationResult(result)) => {
                if result.batch_id != batch_b_id {
                    return Err(format!(
                        "MutationResult batch_id mismatch for Batch B: expected {:?}, got {:?}",
                        batch_b_id, result.batch_id
                    )
                    .into());
                }
                if !result.accepted {
                    return Err(format!(
                        "Node batch rejected: code={}, msg={}",
                        result.error_code, result.error_message
                    )
                    .into());
                }
                if result.created_ids.len() != 6 {
                    return Err(format!(
                        "Batch B must create exactly 6 nodes (bg + 5 children), \
                         got {} created_ids — tasks.md §4.2",
                        result.created_ids.len()
                    )
                    .into());
                }
                println!(
                    "  Batch B (6-node tree + opacity + input_mode): accepted=true, \
                     {} node_ids",
                    result.created_ids.len()
                );
                break result.created_ids;
            }
            other => {
                return Err(format!("Expected MutationResult (node batch), got: {other:?}").into());
            }
        }
    };

    Ok(TileCreationState {
        tile_id: tile_id_bytes,
        lease_id: lease_id_bytes,
        node_ids,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Integration tests for Phases 1–3 (tasks.md §1.1–1.2, §2.1–2.3, §3.1).
    //!
    //! Phase 1 (§1.1–1.2):
    //! - [`test_session_establishment_returns_nonempty_session_id`]
    //! - [`test_session_establishment_returns_nonempty_namespace`]
    //!
    //! Phase 2 (§2.1–2.3):
    //! - [`test_lease_grant_returns_granted_true_and_16_byte_lease_id`]
    //!   Verifies `request_lease` returns a 16-byte UUIDv7 lease_id with granted=true.
    //! - [`test_lease_request_with_invalid_capability_is_denied`]
    //!   Verifies that requesting a non-canonical capability denies the lease.
    //!
    //! Phase 3 (§3.1):
    //! - [`test_upload_icon_returns_32_byte_blake3_resource_id`]
    //!   Verifies `upload_icon` returns a 32-byte BLAKE3 ResourceId.
    //!
    //! All tests spin up a `HeadlessRuntime` with `dev-mode` (unrestricted
    //! capabilities; no registered-agent config required) on an ephemeral port.
    //!
    //! Ephemeral ports prevent port-conflict flakiness in parallel CI.
    //! Each `server` JoinHandle is aborted after assertions complete.

    use tze_hud_runtime::HeadlessRuntime;
    use tze_hud_runtime::headless::HeadlessConfig;

    const TEST_PSK: &str = "dashboard-tile-test-key";
    const TEST_AGENT_ID: &str = "test-dashboard-agent";
    const TEST_AGENT_DISPLAY_NAME: &str = "Test Dashboard Agent";

    /// Bind an ephemeral port and return it.  The listener is dropped before
    /// the gRPC server starts; there is a brief TOCTOU window, but this is the
    /// same pattern used across the integration test suite.
    fn ephemeral_port() -> u16 {
        let listener = std::net::TcpListener::bind("[::1]:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("get local addr").port();
        drop(listener);
        port
    }

    async fn start_test_runtime(
        port: u16,
    ) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
        let config = HeadlessConfig {
            width: 800,
            height: 600,
            grpc_port: port,
            psk: TEST_PSK.to_string(),
            config_toml: None, // dev-mode: unrestricted capabilities
        };
        let runtime = HeadlessRuntime::new(config).await?;
        let server = runtime.start_grpc_server().await?;
        Ok(server)
    }

    // ── Phase 2 helpers ───────────────────────────────────────────────────────

    /// Drain messages from `stream` until the first non-`LeaseStateChange` message.
    async fn next_non_state_change(
        stream: &mut tonic::Streaming<tze_hud_protocol::proto::session::ServerMessage>,
    ) -> tze_hud_protocol::proto::session::ServerMessage {
        use tokio_stream::StreamExt as _;
        loop {
            let msg = stream
                .next()
                .await
                .expect("stream closed before LeaseResponse")
                .expect("stream error");
            match &msg.payload {
                Some(
                    tze_hud_protocol::proto::session::server_message::Payload::LeaseStateChange(_),
                ) => continue,
                _ => return msg,
            }
        }
    }

    /// Task 1.2 — verify session_id is non-empty after successful handshake.
    ///
    /// Spec §SessionEstablished field 1: "Opaque session identifier (UUIDv7),
    /// MUST be non-empty."
    #[tokio::test]
    async fn test_session_establishment_returns_nonempty_session_id() {
        let port = ephemeral_port();
        let server = start_test_runtime(port).await.expect("runtime start");

        // Allow the server a moment to bind before the client connects.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let state =
            crate::establish_session_with(port, TEST_PSK, TEST_AGENT_ID, TEST_AGENT_DISPLAY_NAME)
                .await
                .expect("establish_session_with");

        assert!(
            !state.session_id.is_empty(),
            "session_id must be non-empty (tasks.md §1.2)"
        );

        server.abort();
    }

    /// Task 1.2 — verify namespace is non-empty after successful handshake.
    ///
    /// Spec §SessionEstablished field 2: "Agent's namespace in the scene
    /// (RFC 0001 §1.2). MUST be non-empty."
    #[tokio::test]
    async fn test_session_establishment_returns_nonempty_namespace() {
        let port = ephemeral_port();
        let server = start_test_runtime(port).await.expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let state =
            crate::establish_session_with(port, TEST_PSK, TEST_AGENT_ID, TEST_AGENT_DISPLAY_NAME)
                .await
                .expect("establish_session_with");

        assert!(
            !state.namespace.is_empty(),
            "namespace must be non-empty (tasks.md §1.2)"
        );

        server.abort();
    }

    // ── Phase 2: Lease Acquisition tests ─────────────────────────────────────

    /// Task 2.1–2.2 — `request_lease` returns granted=true and a 16-byte UUIDv7 lease_id.
    ///
    /// Spec §Requirement: Lease Request With AutoRenew — Scenario: Lease granted
    /// with requested parameters.
    /// tasks.md §2.1: send LeaseRequest { ttl_ms=60000, capabilities=[create_tiles,
    ///   modify_own_tiles], lease_priority=2 }.
    /// tasks.md §2.2: verify LeaseResponse.granted=true and store the 16-byte lease_id.
    #[tokio::test]
    async fn test_lease_grant_returns_granted_true_and_16_byte_lease_id() {
        let port = ephemeral_port();
        let server = start_test_runtime(port).await.expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let lease_id_bytes =
            crate::request_lease(port, TEST_PSK, TEST_AGENT_ID, TEST_AGENT_DISPLAY_NAME)
                .await
                .expect("request_lease");

        // tasks.md §2.2: lease_id MUST be exactly 16 bytes (UUIDv7 SceneId).
        assert_eq!(
            lease_id_bytes.len(),
            16,
            "lease_id must be 16 bytes (UUIDv7) — tasks.md §2.2"
        );

        server.abort();
    }

    /// Task 2.3 — LeaseRequest with a non-canonical capability is denied.
    ///
    /// Spec §Requirement: Lease Request With AutoRenew — Scenario: Tile creation
    /// requires active lease (only valid capabilities may be requested).
    /// tasks.md §2.3: add test — lease request without required capabilities is denied.
    ///
    /// "create_tile" (singular) is a legacy non-canonical name rejected since
    /// RFC 0005 Round 14. The server MUST respond with:
    ///   LeaseResponse { granted: false, deny_code: "CONFIG_UNKNOWN_CAPABILITY" }.
    #[tokio::test]
    async fn test_lease_request_with_invalid_capability_is_denied() {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;

        let port = ephemeral_port();
        let server = start_test_runtime(port).await.expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // ── 1. Open a session ─────────────────────────────────────────────────
        #[allow(deprecated)]
        let mut session_client =
            sp::hud_session_client::HudSessionClient::connect(format!("http://[::1]:{port}"))
                .await
                .expect("connect");

        let (tx, rx) = tokio::sync::mpsc::channel::<sp::ClientMessage>(64);
        let stream_req = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut resp_stream = session_client
            .session(stream_req)
            .await
            .expect("session rpc")
            .into_inner();

        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(1);

        // SessionInit with valid capabilities for the session.
        tx.send(sp::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_us,
            payload: Some(sp::client_message::Payload::SessionInit(sp::SessionInit {
                agent_id: "bad-cap-test-agent".to_string(),
                agent_display_name: "Bad Cap Test".to_string(),
                pre_shared_key: TEST_PSK.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: Vec::new(),
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        // Drain SessionEstablished + SceneSnapshot.
        for _ in 0..2 {
            resp_stream
                .next()
                .await
                .expect("stream not closed")
                .expect("no stream error");
        }

        // ── 2. Send a LeaseRequest with a non-canonical (legacy singular) capability ──
        //
        // "create_tile" (singular) was superseded by "create_tiles" (plural) in
        // RFC 0005 Round 14.  The server must reject this with CONFIG_UNKNOWN_CAPABILITY.
        tx.send(sp::ClientMessage {
            sequence: 2,
            timestamp_wall_us: now_us,
            payload: Some(sp::client_message::Payload::LeaseRequest(
                sp::LeaseRequest {
                    ttl_ms: 60_000,
                    capabilities: vec!["create_tile".to_string()], // non-canonical singular form
                    lease_priority: 2,
                },
            )),
        })
        .await
        .unwrap();

        // ── 3. Assert denial ──────────────────────────────────────────────────
        let resp_msg = next_non_state_change(&mut resp_stream).await;
        match resp_msg.payload {
            Some(sp::server_message::Payload::LeaseResponse(resp)) => {
                assert!(
                    !resp.granted,
                    "LeaseResponse must NOT be granted for non-canonical capability — tasks.md §2.3"
                );
                assert_eq!(
                    resp.deny_code, "CONFIG_UNKNOWN_CAPABILITY",
                    "deny_code must be CONFIG_UNKNOWN_CAPABILITY for unknown capability, \
                     got: {:?}",
                    resp.deny_code
                );
                assert!(
                    !resp.deny_reason.is_empty(),
                    "deny_reason must be non-empty — tasks.md §2.3"
                );
            }
            other => panic!(
                "Expected LeaseResponse(denied) for non-canonical capability, got: {other:?}"
            ),
        }

        server.abort();
    }

    // ── Phase 3: Resource Upload tests ───────────────────────────────────────

    /// Task 3.1 — `upload_icon` returns a 32-byte BLAKE3 ResourceId.
    ///
    /// Spec §Requirement: Resource Upload Before Tile Creation — Content-Addressed
    /// Resource Identity: ResourceId = BLAKE3(raw_bytes), 32 bytes.
    /// tasks.md §3.1: upload 48×48 PNG icon, capture ResourceId (BLAKE3 hash).
    #[tokio::test]
    async fn test_upload_icon_returns_32_byte_blake3_resource_id() {
        let resource_id_bytes = crate::upload_icon(TEST_AGENT_ID)
            .await
            .expect("upload_icon");

        // tasks.md §3.1: ResourceId is a 32-byte BLAKE3 digest.
        assert_eq!(
            resource_id_bytes.len(),
            blake3::OUT_LEN, // 32 bytes
            "ResourceId must be 32 bytes (BLAKE3 digest) — tasks.md §3.1, got {} bytes",
            resource_id_bytes.len()
        );
    }

    // ── Phase 4: Atomic Tile Creation Batch tests ─────────────────────────────
    //
    // Tests verify the gRPC wire protocol for tile creation:
    //   §4.1–4.2: create_tile_batch returns accepted=true and a 16-byte tile_id
    //   §4.3, §5.1: scene graph has 6 nodes in painter's model order after commit
    //   §4.4: partial batch failure (invalid bounds) rejects entire batch atomically
    //   §5.2: z_order=100 is below ZONE_TILE_Z_MIN (agent-owned band)
    //   §5.3: chrome z_order (>= ZONE_TILE_Z_MIN) renders above dashboard tile
    //
    // All tests use an isolated HeadlessRuntime on an ephemeral port.

    /// Start a HeadlessRuntime and return both the JoinHandle and the shared state Arc.
    ///
    /// The shared state is needed by Phase 4 tests to create a tab and register
    /// the icon resource before submitting mutation batches over the wire.
    async fn start_test_runtime_with_state(
        port: u16,
    ) -> Result<
        (
            tokio::task::JoinHandle<()>,
            std::sync::Arc<tokio::sync::Mutex<tze_hud_protocol::session::SharedState>>,
        ),
        Box<dyn std::error::Error>,
    > {
        let config = HeadlessConfig {
            width: 1920,
            height: 1080,
            grpc_port: port,
            psk: TEST_PSK.to_string(),
            config_toml: None, // dev-mode: unrestricted capabilities
        };
        let runtime = HeadlessRuntime::new(config).await?;
        let state = runtime.state.clone();
        let server = runtime.start_grpc_server().await?;
        Ok((server, state))
    }

    /// Prepare the scene for tile creation:
    ///   - Create a tab and set it as active (CreateTile requires an active tab).
    ///   - Register the resource_id in the scene (StaticImageNode validation).
    async fn setup_scene_with_resource(
        state: &std::sync::Arc<tokio::sync::Mutex<tze_hud_protocol::session::SharedState>>,
        resource_id_bytes: &[u8],
    ) {
        let resource_id_arr: [u8; 32] = resource_id_bytes
            .try_into()
            .expect("resource_id must be 32 bytes");
        let scene_resource_id = tze_hud_scene::types::ResourceId::from_bytes(resource_id_arr);
        let st = state.lock().await;
        let mut scene = st.scene.lock().await;
        let tab_id = scene.create_tab("Test Main", 0).expect("create_tab");
        scene.active_tab = Some(tab_id);
        scene.register_resource(scene_resource_id);
    }

    /// Task 4.1–4.2 — `create_tile_batch` returns accepted=true and a 16-byte tile_id.
    ///
    /// Spec §Requirement: Atomic Tile Creation Batch
    /// Scenario: Successful atomic tile creation
    /// tasks.md §4.1: MutationBatch with CreateTile (400×300 at (50,50), z_order=100),
    ///   SetTileRoot (6-node tree via AddNode), UpdateTileOpacity, UpdateTileInputMode.
    /// tasks.md §4.2: MutationResult accepted=true, non-empty batch_id, created_ids.
    #[tokio::test]
    async fn test_create_tile_batch_accepted_returns_tile_id() {
        let port = ephemeral_port();
        let (server, state) = start_test_runtime_with_state(port)
            .await
            .expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Upload icon and register resource
        let resource_id_bytes = crate::upload_icon(TEST_AGENT_ID)
            .await
            .expect("upload_icon");
        setup_scene_with_resource(&state, &resource_id_bytes).await;

        let tile_state = crate::create_tile_batch(
            port,
            TEST_PSK,
            TEST_AGENT_ID,
            TEST_AGENT_DISPLAY_NAME,
            resource_id_bytes,
        )
        .await
        .expect("create_tile_batch");

        // tasks.md §4.2: tile_id must be a 16-byte UUIDv7 SceneId.
        assert_eq!(
            tile_state.tile_id.len(),
            16,
            "tile_id must be 16 bytes (UUIDv7) — tasks.md §4.2, got {} bytes",
            tile_state.tile_id.len()
        );

        // tasks.md §4.2: created_ids must contain exactly 6 nodes (bg + 5 children).
        assert_eq!(
            tile_state.node_ids.len(),
            6,
            "node_ids must contain exactly 6 created IDs (bg + 5 children) — tasks.md §4.2, \
             got {}",
            tile_state.node_ids.len()
        );

        server.abort();
    }

    /// Task 4.3, 5.1 — scene has 6 nodes in painter's model order after batch.
    ///
    /// Spec §Requirement: Dashboard Tile Composition
    /// Scenario: All four node types / Painter's model compositing order
    /// tasks.md §4.3: scene graph contains tile with all 6 nodes in correct tree order.
    /// tasks.md §5.1: painter's model ordering —
    ///   SolidColorNode (bg root), then StaticImageNode (icon), then 2× TextMarkdownNode
    ///   (header, body), then 2× HitRegionNode (refresh, dismiss) as children in tree order.
    #[tokio::test]
    async fn test_scene_has_6_nodes_in_painters_model_order() {
        use tze_hud_scene::types::NodeData;

        let port = ephemeral_port();
        let (server, state) = start_test_runtime_with_state(port)
            .await
            .expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let resource_id_bytes = crate::upload_icon(TEST_AGENT_ID)
            .await
            .expect("upload_icon");
        setup_scene_with_resource(&state, &resource_id_bytes).await;

        let tile_state = crate::create_tile_batch(
            port,
            TEST_PSK,
            TEST_AGENT_ID,
            TEST_AGENT_DISPLAY_NAME,
            resource_id_bytes,
        )
        .await
        .expect("create_tile_batch");

        // Inspect the scene graph.
        let st = state.lock().await;
        let scene = st.scene.lock().await;

        // tasks.md §4.3: exactly 6 nodes in the scene.
        assert_eq!(
            scene.node_count(),
            6,
            "scene must contain exactly 6 nodes — tasks.md §4.3, got {}",
            scene.node_count()
        );

        // Decode tile_id from wire bytes (big-endian RFC 4122).
        let tile_id_arr: [u8; 16] = tile_state
            .tile_id
            .as_slice()
            .try_into()
            .expect("tile_id must be 16 bytes");
        let tile_uuid = uuid::Uuid::from_bytes(tile_id_arr);
        let tile_scene_id = tze_hud_scene::SceneId::from_uuid(tile_uuid);

        let tile = scene
            .tiles
            .get(&tile_scene_id)
            .expect("tile must exist in scene");

        // tasks.md §5.1: root must be SolidColorNode (background).
        let root_id = tile.root_node.expect("tile must have a root node");
        let root = scene.nodes.get(&root_id).expect("root node must exist");
        assert!(
            matches!(root.data, NodeData::SolidColor(_)),
            "root node must be SolidColorNode (background) — tasks.md §5.1"
        );

        // tasks.md §4.3: root must have exactly 5 children.
        assert_eq!(
            root.children.len(),
            5,
            "root must have exactly 5 children (icon, header, body, refresh, dismiss) \
             — tasks.md §4.3"
        );

        // tasks.md §5.1: children in painter's model order.
        let child_types: Vec<&str> = root
            .children
            .iter()
            .map(|&cid| {
                let child = scene.nodes.get(&cid).expect("child node must exist");
                match &child.data {
                    NodeData::SolidColor(_) => "SolidColor",
                    NodeData::StaticImage(_) => "StaticImage",
                    NodeData::TextMarkdown(_) => "TextMarkdown",
                    NodeData::HitRegion(_) => "HitRegion",
                }
            })
            .collect();

        assert_eq!(
            child_types,
            [
                "StaticImage",
                "TextMarkdown",
                "TextMarkdown",
                "HitRegion",
                "HitRegion"
            ],
            "children must be in painter's model order: StaticImage, TextMarkdown (header), \
             TextMarkdown (body), HitRegion (refresh), HitRegion (dismiss) — tasks.md §5.1"
        );

        // Verify HitRegion interaction_ids (§4.3 — correct tree order / content).
        let refresh_node = scene
            .nodes
            .get(&root.children[3])
            .expect("refresh node must exist");
        if let NodeData::HitRegion(hr) = &refresh_node.data {
            assert_eq!(
                hr.interaction_id, "refresh-button",
                "children[3] interaction_id must be 'refresh-button' — tasks.md §4.3"
            );
        } else {
            panic!("children[3] must be HitRegionNode (refresh) — tasks.md §4.3");
        }

        let dismiss_node = scene
            .nodes
            .get(&root.children[4])
            .expect("dismiss node must exist");
        if let NodeData::HitRegion(hr) = &dismiss_node.data {
            assert_eq!(
                hr.interaction_id, "dismiss-button",
                "children[4] interaction_id must be 'dismiss-button' — tasks.md §4.3"
            );
        } else {
            panic!("children[4] must be HitRegionNode (dismiss) — tasks.md §4.3");
        }

        server.abort();
    }

    /// Task 4.4 — partial batch failure rejects entire batch atomically.
    ///
    /// Spec §Requirement: Atomic Tile Creation Batch
    /// Scenario: Partial failure rejects entire batch
    /// tasks.md §4.4: a batch containing one valid CreateTile and one CreateTile with
    ///   width=0 (invalid bounds per RFC 0001 §2.3) is rejected atomically —
    ///   no tiles from the failed batch appear in the scene.
    #[tokio::test]
    async fn test_partial_batch_failure_rejects_atomically() {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;

        let port = ephemeral_port();
        let (server, state) = start_test_runtime_with_state(port)
            .await
            .expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Create a tab (CreateTile requires an active tab); skip resource upload
        // since the batch will be rejected before any StaticImageNode is processed.
        {
            let st = state.lock().await;
            let mut scene = st.scene.lock().await;
            let tab_id = scene.create_tab("Test Tab", 0).expect("create_tab");
            scene.active_tab = Some(tab_id);
        }

        // Open a session and acquire a lease.
        #[allow(deprecated)]
        let mut session_client =
            sp::hud_session_client::HudSessionClient::connect(format!("http://[::1]:{port}"))
                .await
                .expect("connect");

        let (tx, rx) = tokio::sync::mpsc::channel::<sp::ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut response_stream = session_client
            .session(stream)
            .await
            .expect("session rpc")
            .into_inner();

        let now_us = crate::now_wall_us();
        tx.send(sp::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_us,
            payload: Some(sp::client_message::Payload::SessionInit(sp::SessionInit {
                agent_id: "partial-fail-test-agent".to_string(),
                agent_display_name: "Partial Fail Test".to_string(),
                pre_shared_key: TEST_PSK.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: vec![],
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        // Drain SessionEstablished + SceneSnapshot.
        for _ in 0..2 {
            response_stream
                .next()
                .await
                .expect("stream open")
                .expect("no error");
        }

        // Acquire lease.
        tx.send(sp::ClientMessage {
            sequence: 2,
            timestamp_wall_us: crate::now_wall_us(),
            payload: Some(sp::client_message::Payload::LeaseRequest(
                sp::LeaseRequest {
                    ttl_ms: 60_000,
                    capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                    lease_priority: 2,
                },
            )),
        })
        .await
        .unwrap();

        let lease_id_bytes: Vec<u8> = loop {
            let msg = next_non_state_change(&mut response_stream).await;
            if let Some(sp::server_message::Payload::LeaseResponse(resp)) = msg.payload {
                assert!(resp.granted, "lease must be granted for partial-fail test");
                break resp.lease_id;
            }
        };

        // Batch A: CreateTile (valid)
        let batch_a_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(sp::ClientMessage {
            sequence: 3,
            timestamp_wall_us: crate::now_wall_us(),
            payload: Some(sp::client_message::Payload::MutationBatch(
                sp::MutationBatch {
                    batch_id: batch_a_id,
                    lease_id: lease_id_bytes.clone(),
                    mutations: vec![tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::CreateTile(
                                tze_hud_protocol::proto::CreateTileMutation {
                                    tab_id: vec![],
                                    bounds: Some(tze_hud_protocol::proto::Rect {
                                        x: 50.0,
                                        y: 50.0,
                                        width: 400.0,
                                        height: 300.0,
                                        ..Default::default()
                                    }),
                                    z_order: 100,
                                },
                            ),
                        ),
                    }],
                    timing: None,
                },
            )),
        })
        .await
        .unwrap();

        let tile_id_bytes: Vec<u8> = loop {
            let msg = next_non_state_change(&mut response_stream).await;
            if let Some(sp::server_message::Payload::MutationResult(result)) = msg.payload {
                assert!(result.accepted, "CreateTile must succeed; got: {result:?}");
                break result.created_ids[0].clone();
            }
        };

        // Record tile count before the failing batch.
        let tile_count_before = {
            let st = state.lock().await;
            st.scene.lock().await.tile_count()
        };

        // Batch B: two CreateTile mutations — the second has width=0 (invalid tile bounds).
        // RFC 0001 §2.3: tile width and height must be > 0. The entire batch must be
        // rejected atomically (tasks.md §4.4: all-or-nothing).
        let batch_b_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(sp::ClientMessage {
            sequence: 4,
            timestamp_wall_us: crate::now_wall_us(),
            payload: Some(sp::client_message::Payload::MutationBatch(
                sp::MutationBatch {
                    batch_id: batch_b_id.clone(),
                    lease_id: lease_id_bytes.clone(),
                    mutations: vec![
                        // Valid CreateTile
                        tze_hud_protocol::proto::MutationProto {
                            mutation: Some(
                                tze_hud_protocol::proto::mutation_proto::Mutation::CreateTile(
                                    tze_hud_protocol::proto::CreateTileMutation {
                                        tab_id: vec![],
                                        bounds: Some(tze_hud_protocol::proto::Rect {
                                            x: 50.0,
                                            y: 200.0,
                                            width: 200.0,
                                            height: 100.0,
                                            ..Default::default()
                                        }),
                                        z_order: 101,
                                    },
                                ),
                            ),
                        },
                        // Invalid CreateTile: width=0 → bounds validation fails (RFC 0001 §2.3)
                        tze_hud_protocol::proto::MutationProto {
                            mutation: Some(
                                tze_hud_protocol::proto::mutation_proto::Mutation::CreateTile(
                                    tze_hud_protocol::proto::CreateTileMutation {
                                        tab_id: vec![],
                                        bounds: Some(tze_hud_protocol::proto::Rect {
                                            x: 0.0,
                                            y: 0.0,
                                            width: 0.0, // INVALID: width must be > 0
                                            height: 50.0,
                                            ..Default::default()
                                        }),
                                        z_order: 102,
                                    },
                                ),
                            ),
                        },
                    ],
                    timing: None,
                },
            )),
        })
        .await
        .unwrap();

        // Expect MutationResult with accepted=false (entire batch rejected).
        let result_msg = next_non_state_change(&mut response_stream).await;
        match result_msg.payload {
            Some(sp::server_message::Payload::MutationResult(result)) => {
                assert_eq!(
                    result.batch_id, batch_b_id,
                    "batch_id must be echoed back — tasks.md §4.2"
                );
                assert!(
                    !result.accepted,
                    "batch with width=0 CreateTile must be rejected atomically — tasks.md §4.4; \
                     got: accepted={}, error_code={}, msg={}",
                    result.accepted, result.error_code, result.error_message
                );
            }
            other => panic!(
                "Expected MutationResult (rejected) for partial batch failure, got: {other:?}"
            ),
        }

        // tasks.md §4.4: tile count must not change — no tiles from Batch B were committed.
        // (The tile itself was created in Batch A and is still present.)
        let tile_count_after = {
            let st = state.lock().await;
            st.scene.lock().await.tile_count()
        };
        assert_eq!(
            tile_count_after, tile_count_before,
            "tile count must not change after rejected CreateTile batch — tasks.md §4.4"
        );

        // The tile from Batch A must have no root node (Batch B was fully rolled back).
        {
            let tile_id_arr: [u8; 16] = tile_id_bytes
                .as_slice()
                .try_into()
                .expect("tile_id must be 16 bytes");
            let tile_uuid = uuid::Uuid::from_bytes(tile_id_arr);
            let tile_scene_id = tze_hud_scene::SceneId::from_uuid(tile_uuid);
            let st = state.lock().await;
            let scene = st.scene.lock().await;
            let tile = scene
                .tiles
                .get(&tile_scene_id)
                .expect("tile must still exist (created in Batch A)");
            assert!(
                tile.root_node.is_none(),
                "tile root must be None after rejected node batch (atomicity) — tasks.md §4.4"
            );
        }

        server.abort();
    }

    /// Task 4.4 (node batch atomicity) — 6-node batch rejected when resource is unregistered.
    ///
    /// Spec §Requirement: Atomic Tile Creation Batch
    /// Scenario: ResourceNotFound causes entire 6-node batch to be rejected
    /// tasks.md §4.4: if any mutation in Batch B fails (e.g., StaticImageNode references an
    ///   unregistered resource_id), the entire batch is rejected atomically — no nodes from
    ///   that batch are committed.  The tile from Batch A persists but has no root node.
    #[tokio::test]
    async fn test_node_batch_rejected_atomically_on_unregistered_resource() {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;

        let port = ephemeral_port();
        let (server, state) = start_test_runtime_with_state(port)
            .await
            .expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Create a tab but do NOT register any resource — StaticImageNode will fail.
        {
            let st = state.lock().await;
            let mut scene = st.scene.lock().await;
            let tab_id = scene.create_tab("Test Tab", 0).expect("create_tab");
            scene.active_tab = Some(tab_id);
        }

        // Use a random (unregistered) 32-byte resource_id.
        let unregistered_resource_id = vec![0xABu8; 32];

        // Open session and acquire lease.
        #[allow(deprecated)]
        let mut session_client =
            sp::hud_session_client::HudSessionClient::connect(format!("http://[::1]:{port}"))
                .await
                .expect("connect");

        let (tx, rx) = tokio::sync::mpsc::channel::<sp::ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut response_stream = session_client
            .session(stream)
            .await
            .expect("session rpc")
            .into_inner();

        let now_us = crate::now_wall_us();
        tx.send(sp::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_us,
            payload: Some(sp::client_message::Payload::SessionInit(sp::SessionInit {
                agent_id: "node-atomicity-test-agent".to_string(),
                agent_display_name: "Node Atomicity Test".to_string(),
                pre_shared_key: TEST_PSK.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: vec![],
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        for _ in 0..2 {
            response_stream
                .next()
                .await
                .expect("stream open")
                .expect("no error");
        }

        tx.send(sp::ClientMessage {
            sequence: 2,
            timestamp_wall_us: crate::now_wall_us(),
            payload: Some(sp::client_message::Payload::LeaseRequest(
                sp::LeaseRequest {
                    ttl_ms: 60_000,
                    capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                    lease_priority: 2,
                },
            )),
        })
        .await
        .unwrap();

        let lease_id_bytes: Vec<u8> = loop {
            let msg = next_non_state_change(&mut response_stream).await;
            if let Some(sp::server_message::Payload::LeaseResponse(resp)) = msg.payload {
                assert!(
                    resp.granted,
                    "lease must be granted for node-atomicity test"
                );
                break resp.lease_id;
            }
        };

        // Batch A: CreateTile (valid)
        let batch_a_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(sp::ClientMessage {
            sequence: 3,
            timestamp_wall_us: crate::now_wall_us(),
            payload: Some(sp::client_message::Payload::MutationBatch(
                sp::MutationBatch {
                    batch_id: batch_a_id,
                    lease_id: lease_id_bytes.clone(),
                    mutations: vec![tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::CreateTile(
                                tze_hud_protocol::proto::CreateTileMutation {
                                    tab_id: vec![],
                                    bounds: Some(tze_hud_protocol::proto::Rect {
                                        x: 50.0,
                                        y: 50.0,
                                        width: 400.0,
                                        height: 300.0,
                                        ..Default::default()
                                    }),
                                    z_order: 100,
                                },
                            ),
                        ),
                    }],
                    timing: None,
                },
            )),
        })
        .await
        .unwrap();

        let tile_id_bytes: Vec<u8> = loop {
            let msg = next_non_state_change(&mut response_stream).await;
            if let Some(sp::server_message::Payload::MutationResult(result)) = msg.payload {
                assert!(result.accepted, "CreateTile must succeed");
                break result.created_ids[0].clone();
            }
        };

        // Record node count before the failing batch.
        let node_count_before = {
            let st = state.lock().await;
            st.scene.lock().await.node_count()
        };

        // Batch B: SolidColorNode (valid) + StaticImageNode with unregistered resource_id.
        // The StaticImageNode will fail ResourceNotFound; entire batch must be rejected.
        let bg_uuid = uuid::Uuid::now_v7();
        let bg_node_id_le = bg_uuid.to_bytes_le().to_vec();
        let bg_parent_id_be = bg_uuid.as_bytes().to_vec();

        let bg_node = tze_hud_protocol::proto::NodeProto {
            id: bg_node_id_le,
            data: Some(tze_hud_protocol::proto::node_proto::Data::SolidColor(
                tze_hud_protocol::proto::SolidColorNodeProto {
                    color: Some(tze_hud_protocol::proto::Rgba {
                        r: 0.07,
                        g: 0.07,
                        b: 0.07,
                        a: 0.90,
                    }),
                    bounds: Some(tze_hud_protocol::proto::Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 400.0,
                        height: 300.0,
                        ..Default::default()
                    }),
                },
            )),
        };
        let icon_node = tze_hud_protocol::proto::NodeProto {
            id: vec![],
            data: Some(tze_hud_protocol::proto::node_proto::Data::StaticImage(
                tze_hud_protocol::proto::StaticImageNodeProto {
                    resource_id: unregistered_resource_id, // triggers ResourceNotFound
                    width: 48,
                    height: 48,
                    decoded_bytes: (48u64 * 48 * 4),
                    fit_mode: tze_hud_protocol::proto::ImageFitModeProto::ImageFitModeContain
                        as i32,
                    bounds: Some(tze_hud_protocol::proto::Rect {
                        x: 16.0,
                        y: 16.0,
                        width: 48.0,
                        height: 48.0,
                        ..Default::default()
                    }),
                },
            )),
        };

        let batch_b_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        tx.send(sp::ClientMessage {
            sequence: 4,
            timestamp_wall_us: crate::now_wall_us(),
            payload: Some(sp::client_message::Payload::MutationBatch(
                sp::MutationBatch {
                    batch_id: batch_b_id.clone(),
                    lease_id: lease_id_bytes.clone(),
                    mutations: vec![
                        tze_hud_protocol::proto::MutationProto {
                            mutation: Some(
                                tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                                    tze_hud_protocol::proto::AddNodeMutation {
                                        tile_id: tile_id_bytes.clone(),
                                        parent_id: vec![],
                                        node: Some(bg_node),
                                    },
                                ),
                            ),
                        },
                        tze_hud_protocol::proto::MutationProto {
                            mutation: Some(
                                tze_hud_protocol::proto::mutation_proto::Mutation::AddNode(
                                    tze_hud_protocol::proto::AddNodeMutation {
                                        tile_id: tile_id_bytes.clone(),
                                        parent_id: bg_parent_id_be,
                                        node: Some(icon_node),
                                    },
                                ),
                            ),
                        },
                    ],
                    timing: None,
                },
            )),
        })
        .await
        .unwrap();

        // Expect rejected batch (entire batch must be refused atomically).
        let result_msg = next_non_state_change(&mut response_stream).await;
        match result_msg.payload {
            Some(sp::server_message::Payload::MutationResult(result)) => {
                assert_eq!(
                    result.batch_id, batch_b_id,
                    "batch_id must be echoed back — tasks.md §4.2"
                );
                assert!(
                    !result.accepted,
                    "batch with unregistered StaticImageNode resource_id must be rejected \
                     atomically — tasks.md §4.4; got: accepted={}, error_code={}, msg={}",
                    result.accepted, result.error_code, result.error_message
                );
            }
            other => panic!(
                "Expected rejected MutationResult for unregistered-resource batch, got: {other:?}"
            ),
        }

        // tasks.md §4.4: node count must not change — no nodes from Batch B were committed.
        let node_count_after = {
            let st = state.lock().await;
            st.scene.lock().await.node_count()
        };
        assert_eq!(
            node_count_after, node_count_before,
            "node count must not change after rejected node batch — tasks.md §4.4"
        );

        // The tile from Batch A must still exist but have no root node.
        {
            let tile_id_arr: [u8; 16] = tile_id_bytes
                .as_slice()
                .try_into()
                .expect("tile_id must be 16 bytes");
            let tile_uuid = uuid::Uuid::from_bytes(tile_id_arr);
            let tile_scene_id = tze_hud_scene::SceneId::from_uuid(tile_uuid);
            let st = state.lock().await;
            let scene = st.scene.lock().await;
            let tile = scene
                .tiles
                .get(&tile_scene_id)
                .expect("tile must still exist (created in Batch A)");
            assert!(
                tile.root_node.is_none(),
                "tile root must be None after rejected node batch (atomicity) — tasks.md §4.4"
            );
        }

        server.abort();
    }

    // ── Phase 5: Intra-Tile Compositing Verification (pure logic tests) ───────

    /// Task 5.2 — z_order=100 is below ZONE_TILE_Z_MIN, confirming agent-owned band.
    ///
    /// Spec §Requirement: Z-Order Compositing at Content Layer
    /// Scenario: Agent tile in content band
    /// tasks.md §5.2: verify z_order=100 places the tile below ZONE_TILE_Z_MIN (0x8000_0000).
    #[test]
    fn test_z_order_100_is_in_agent_owned_band_below_zone_tile_z_min() {
        assert!(
            crate::TILE_Z_ORDER < tze_hud_scene::types::ZONE_TILE_Z_MIN,
            "TILE_Z_ORDER={} must be below ZONE_TILE_Z_MIN=0x{:08x} — tasks.md §5.2",
            crate::TILE_Z_ORDER,
            tze_hud_scene::types::ZONE_TILE_Z_MIN,
        );
    }

    /// Task 5.3 — chrome layer z_order renders above the dashboard tile.
    ///
    /// Spec §Requirement: Z-Order Compositing at Content Layer
    /// Scenario: Chrome layer renders above dashboard tile
    /// tasks.md §5.3: verify chrome layer elements (tab bar, disconnection badges)
    ///   render above the dashboard tile.
    ///
    /// Chrome tiles have lease priority 0 and MUST use z_order >= ZONE_TILE_Z_MIN.
    /// The hit-test contract checks chrome tiles before content tiles regardless of z_order
    /// (per scene-graph/spec.md §Requirement: Hit-Testing Contract, RFC 0001 §5.1-5.2).
    #[test]
    fn test_chrome_z_order_renders_above_dashboard_tile() {
        // Chrome elements use z_orders >= ZONE_TILE_Z_MIN.
        let chrome_z = tze_hud_scene::types::ZONE_TILE_Z_MIN + 1;
        let dashboard_z = crate::TILE_Z_ORDER;
        assert!(
            chrome_z > dashboard_z,
            "chrome z (0x{:08x}) must exceed dashboard z ({}) — tasks.md §5.3",
            chrome_z,
            dashboard_z
        );
    }

    // ── Phase 6: Periodic Content Update tests ────────────────────────────────

    /// Task 6.2 — content update succeeds when lease is ACTIVE.
    ///
    /// Spec §Requirement: Periodic Content Update
    /// Scenario: Content update succeeds with active lease
    /// tasks.md §6.2: content update (SetTileRoot + 5× AddNode batch) is accepted
    ///   when the agent holds an ACTIVE lease, and the body TextMarkdownNode
    ///   reflects the new content.
    #[tokio::test]
    async fn test_content_update_succeeds_with_active_lease() {
        use tze_hud_scene::types::NodeData;

        let port = ephemeral_port();
        let (server, state) = start_test_runtime_with_state(port)
            .await
            .expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Upload icon and set up scene.
        let resource_id_bytes = crate::upload_icon(TEST_AGENT_ID)
            .await
            .expect("upload_icon");
        setup_scene_with_resource(&state, &resource_id_bytes).await;

        // Phase 4: create the tile.
        let tile_state = crate::create_tile_batch(
            port,
            TEST_PSK,
            TEST_AGENT_ID,
            TEST_AGENT_DISPLAY_NAME,
            resource_id_bytes.clone(),
        )
        .await
        .expect("create_tile_batch");

        // Phase 6.2: submit a content update (cycle 42).
        let update_result = crate::do_content_update(
            port,
            TEST_PSK,
            TEST_AGENT_ID,
            TEST_AGENT_DISPLAY_NAME,
            tile_state.tile_id.clone(),
            resource_id_bytes.clone(),
            42,
        )
        .await;
        assert!(
            update_result.is_ok(),
            "content update must succeed with active lease — tasks.md §6.2; \
             got: {:?}",
            update_result
        );

        // tasks.md §6.2: verify body TextMarkdownNode reflects new content.
        let st = state.lock().await;
        let scene = st.scene.lock().await;

        let tile_id_arr: [u8; 16] = tile_state
            .tile_id
            .as_slice()
            .try_into()
            .expect("tile_id 16 bytes");
        let tile_uuid = uuid::Uuid::from_bytes(tile_id_arr);
        let tile_scene_id = tze_hud_scene::SceneId::from_uuid(tile_uuid);

        let tile = scene
            .tiles
            .get(&tile_scene_id)
            .expect("tile must exist after content update");
        let root_id = tile
            .root_node
            .expect("tile must have a root after content update");
        let root = scene.nodes.get(&root_id).expect("root node must exist");

        // Find the TextMarkdown body (child index 2 = header, 3 = body).
        // After content update the tree is freshly rebuilt with the same layout.
        let body_node = root
            .children
            .iter()
            .filter_map(|cid| scene.nodes.get(cid))
            .find(|n| {
                if let NodeData::TextMarkdown(tm) = &n.data {
                    tm.content.contains("Update cycle: 42")
                } else {
                    false
                }
            })
            .expect("body TextMarkdownNode must contain 'Update cycle: 42' — tasks.md §6.2");

        assert!(
            matches!(body_node.data, NodeData::TextMarkdown(_)),
            "updated node must be TextMarkdownNode — tasks.md §6.2"
        );

        server.abort();
    }

    /// Task 6.3 — content update is rejected when lease is unknown/expired.
    ///
    /// Spec §Requirement: Periodic Content Update
    /// Scenario: Content update fails with expired lease
    /// tasks.md §6.3: SetTileRoot is rejected when submitted under a lease_id
    ///   that is unknown to the runtime (simulates expired/released lease).
    ///
    /// We open a second session B, which holds no active lease, and submit a
    /// MutationBatch using an all-zero (fabricated/unknown) lease_id.  The
    /// runtime must reject it with a non-empty error_code.  This exercises the
    /// "unknown/expired lease" path without requiring the full TTL to elapse.
    ///
    /// Note: the doc comment previously mentioned "LeaseRelease" but the test
    /// uses a fabricated all-zero lease_id, which is the correct approach for
    /// testing the expired/unknown lease path without coupling to TTL timing.
    #[tokio::test]
    async fn test_content_update_rejected_when_lease_inactive() {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;

        let port = ephemeral_port();
        let (server, state) = start_test_runtime_with_state(port)
            .await
            .expect("runtime start");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let resource_id_bytes = crate::upload_icon(TEST_AGENT_ID)
            .await
            .expect("upload_icon");
        setup_scene_with_resource(&state, &resource_id_bytes).await;

        // ── 1. Create tile on session A (gets lease). ─────────────────────
        let tile_state = crate::create_tile_batch(
            port,
            TEST_PSK,
            TEST_AGENT_ID,
            TEST_AGENT_DISPLAY_NAME,
            resource_id_bytes.clone(),
        )
        .await
        .expect("create_tile_batch");

        // ── 2. Open session B and try to update with a fabricated lease id.
        //       No valid lease exists on session B, so the update must be
        //       rejected. This simulates the "expired / unknown lease" path.
        #[allow(deprecated)]
        let mut session_client =
            sp::hud_session_client::HudSessionClient::connect(format!("http://[::1]:{port}"))
                .await
                .expect("connect");

        let (tx, rx) = tokio::sync::mpsc::channel::<sp::ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut response_stream = session_client
            .session(stream)
            .await
            .expect("session rpc")
            .into_inner();

        let now_us = crate::now_wall_us();
        tx.send(sp::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_us,
            payload: Some(sp::client_message::Payload::SessionInit(sp::SessionInit {
                agent_id: "expired-lease-update-agent".to_string(),
                agent_display_name: "Expired Lease Test".to_string(),
                pre_shared_key: TEST_PSK.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                initial_subscriptions: vec!["LEASE_CHANGES".to_string()],
                resume_token: vec![],
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        for _ in 0..2 {
            response_stream
                .next()
                .await
                .expect("stream open")
                .expect("no error");
        }

        // Use a zeroed (non-existent) lease_id — simulates expired/unknown lease.
        let dead_lease_id = vec![0u8; 16];
        let batch_id = uuid::Uuid::now_v7().as_bytes().to_vec();

        // Build a minimal SetTileRoot batch.
        let bg_uuid = uuid::Uuid::now_v7();
        let bg_node = tze_hud_protocol::proto::NodeProto {
            id: bg_uuid.to_bytes_le().to_vec(),
            data: Some(tze_hud_protocol::proto::node_proto::Data::SolidColor(
                tze_hud_protocol::proto::SolidColorNodeProto {
                    color: Some(tze_hud_protocol::proto::Rgba {
                        r: 0.1,
                        g: 0.1,
                        b: 0.1,
                        a: 1.0,
                    }),
                    bounds: Some(tze_hud_protocol::proto::Rect {
                        x: 0.0,
                        y: 0.0,
                        width: crate::TILE_W,
                        height: crate::TILE_H,
                    }),
                },
            )),
        };

        tx.send(sp::ClientMessage {
            sequence: 2,
            timestamp_wall_us: crate::now_wall_us(),
            payload: Some(sp::client_message::Payload::MutationBatch(
                sp::MutationBatch {
                    batch_id: batch_id.clone(),
                    lease_id: dead_lease_id,
                    mutations: vec![tze_hud_protocol::proto::MutationProto {
                        mutation: Some(
                            tze_hud_protocol::proto::mutation_proto::Mutation::SetTileRoot(
                                tze_hud_protocol::proto::SetTileRootMutation {
                                    tile_id: tile_state.tile_id.clone(),
                                    node: Some(bg_node),
                                },
                            ),
                        ),
                    }],
                    timing: None,
                },
            )),
        })
        .await
        .unwrap();

        // Expect MutationResult rejected — expired/unknown lease.
        let result_msg = next_non_state_change(&mut response_stream).await;
        match result_msg.payload {
            Some(sp::server_message::Payload::MutationResult(result)) => {
                assert!(
                    !result.accepted,
                    "SetTileRoot with expired/unknown lease must be rejected — tasks.md §6.3; \
                     got accepted=true, code={}, msg={}",
                    result.error_code, result.error_message
                );
                assert!(
                    !result.error_code.is_empty(),
                    "error_code must be non-empty for rejected content update — tasks.md §6.3"
                );
            }
            other => panic!(
                "Expected MutationResult (rejected) for expired-lease update, got: {other:?}"
            ),
        }

        server.abort();
    }

    // ── Phase 7: HitRegionNode Input Capture tests ─────────────────────────────
    //
    // Tests in this section exercise the InputProcessor layer directly against a
    // constructed scene graph.  No gRPC session or runtime is needed — all inputs
    // are injected synthetically.
    //
    // The scene layout mirrors the dashboard tile (tasks.md §4.1):
    //   - Tile at (50,50), 400×300.
    //   - HitRegionNode "refresh-button" at local (16,256), 176×36.
    //     → display-space centre: (50+16+88, 50+256+18) = (154, 324).
    //   - HitRegionNode "dismiss-button" at local (208,256), 176×36.
    //     → display-space centre: (50+208+88, 50+256+18) = (346, 324).

    /// Build a minimal scene with the dashboard tile and return the scene + tile_id.
    ///
    /// Creates the same 6-node tree as Phase 4 using scene-layer APIs directly
    /// (no gRPC), so hit-test and InputProcessor tests can operate without a
    /// running server.
    fn build_dashboard_scene() -> (tze_hud_scene::graph::SceneGraph, tze_hud_scene::SceneId) {
        use tze_hud_scene::graph::SceneGraph;
        use tze_hud_scene::types::{
            HitRegionNode, Node, NodeData, Rect, SolidColorNode, TextMarkdownNode,
        };
        use tze_hud_scene::{Capability, Rgba};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Test", 0).expect("create_tab");
        scene.active_tab = Some(tab_id);

        // Lease + tile (no real resource needed for input tests).
        let lease_id = scene.grant_lease(
            "test-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        let tile_id = scene
            .create_tile(
                tab_id,
                "test-agent",
                lease_id,
                Rect::new(crate::TILE_X, crate::TILE_Y, crate::TILE_W, crate::TILE_H),
                crate::TILE_Z_ORDER,
            )
            .expect("create_tile");

        // Build the 6-node tree using direct struct construction.
        let bg_id = tze_hud_scene::SceneId::new();
        let bg = Node {
            id: bg_id,
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(0.07, 0.07, 0.07, 0.90),
                bounds: Rect::new(0.0, 0.0, crate::TILE_W, crate::TILE_H),
            }),
        };
        scene
            .add_node_to_tile_checked(tile_id, None, bg, "test-agent")
            .expect("add bg");

        let header = Node {
            id: tze_hud_scene::SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "**Dashboard Agent**".to_string(),
                bounds: Rect::new(76.0, 20.0, 308.0, 32.0),
                font_size_px: 18.0,
                font_family: tze_hud_scene::types::FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: None,
                alignment: tze_hud_scene::types::TextAlign::Start,
                overflow: tze_hud_scene::types::TextOverflow::Clip,
            }),
        };
        scene
            .add_node_to_tile_checked(tile_id, Some(bg_id), header, "test-agent")
            .expect("add header");

        let body = Node {
            id: tze_hud_scene::SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "**Status**: operational".to_string(),
                bounds: Rect::new(16.0, 72.0, 368.0, 180.0),
                font_size_px: 14.0,
                font_family: tze_hud_scene::types::FontFamily::SystemSansSerif,
                color: Rgba::new(0.78, 0.78, 0.78, 1.0),
                background: None,
                alignment: tze_hud_scene::types::TextAlign::Start,
                overflow: tze_hud_scene::types::TextOverflow::Clip,
            }),
        };
        scene
            .add_node_to_tile_checked(tile_id, Some(bg_id), body, "test-agent")
            .expect("add body");

        let refresh = Node {
            id: tze_hud_scene::SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(16.0, 256.0, 176.0, 36.0),
                interaction_id: "refresh-button".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };
        scene
            .add_node_to_tile_checked(tile_id, Some(bg_id), refresh, "test-agent")
            .expect("add refresh");

        let dismiss = Node {
            id: tze_hud_scene::SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(208.0, 256.0, 176.0, 36.0),
                interaction_id: "dismiss-button".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };
        scene
            .add_node_to_tile_checked(tile_id, Some(bg_id), dismiss, "test-agent")
            .expect("add dismiss");

        (scene, tile_id)
    }

    /// Task 7.1 — PointerDownEvent at Refresh coordinates → NodeHit "refresh-button".
    ///
    /// Spec §Requirement: HitRegionNode input capture
    /// Scenario: Pointer down hits Refresh button
    /// tasks.md §7.1: injected PointerDownEvent at coordinates within "Refresh"
    ///   HitRegionNode bounds produces a NodeHit with interaction_id = "refresh-button".
    ///
    /// Refresh button local bounds: x=16, y=256, w=176, h=36.
    /// Tile origin: (50, 50).
    /// Display-space hit point: (50+16+88, 50+256+18) = (154, 324) — centre of Refresh.
    #[test]
    fn test_pointer_down_at_refresh_coordinates_hits_refresh_button() {
        use tze_hud_scene::HitResult;

        let (scene, _tile_id) = build_dashboard_scene();

        // Centre of Refresh button in display space.
        let hit_x = crate::TILE_X + 16.0 + 88.0; // 154.0
        let hit_y = crate::TILE_Y + 256.0 + 18.0; // 324.0

        let result = scene.hit_test(hit_x, hit_y);

        assert!(
            matches!(result, HitResult::NodeHit { .. }),
            "pointer at ({hit_x},{hit_y}) must hit a HitRegionNode — tasks.md §7.1; \
             got: {result:?}"
        );

        if let HitResult::NodeHit { interaction_id, .. } = result {
            assert_eq!(
                interaction_id, "refresh-button",
                "interaction_id must be 'refresh-button' — tasks.md §7.1"
            );
        }
    }

    /// Task 7.2 — HitRegionLocalState.pressed = true within p99 < 4ms.
    ///
    /// Spec §Requirement: Local feedback latency (p99 < 4ms)
    /// Scenario: PointerDown sets pressed state immediately
    /// tasks.md §7.2: HitRegionLocalState.pressed is set to true within p99 < 4ms
    ///   of PointerDownEvent arrival (headless, synthetic injection).
    ///
    /// The 4ms budget covers the full local-feedback path:
    ///   hit-test + state mutation + SceneLocalPatch emission.
    /// Under headless synthetic injection this is typically < 100µs.
    #[test]
    fn test_pointer_down_sets_pressed_within_latency_budget() {
        use std::time::Instant;
        use tze_hud_input::{InputProcessor, PointerEvent, PointerEventKind};

        let (mut scene, _tile_id) = build_dashboard_scene();
        let mut processor = InputProcessor::new();

        // Centre of Refresh button in display space.
        let hit_x = crate::TILE_X + 16.0 + 88.0;
        let hit_y = crate::TILE_Y + 256.0 + 18.0;

        let event = PointerEvent {
            x: hit_x,
            y: hit_y,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: Some(Instant::now()),
        };

        let t0 = Instant::now();
        let result = processor.process(&event, &mut scene);
        let elapsed_us = t0.elapsed().as_micros() as u64;

        // tasks.md §7.2: local_ack_us must be within the calibrated 4ms budget.
        // Use tze_hud_scene::calibration::test_budget() to scale the raw budget
        // by the measured hardware speed factor, preventing flakiness on slower CI
        // machines while still enforcing the intended latency contract.
        use tze_hud_scene::calibration::{budgets, test_budget};
        let ack_budget = test_budget(budgets::INPUT_ACK_BUDGET_US);
        assert!(
            result.local_ack_us < ack_budget,
            "local_ack_us must be < {}µs (calibrated 4ms p99 budget) — tasks.md §7.2; \
             got {}µs",
            ack_budget,
            result.local_ack_us
        );

        // Also assert wall-clock elapsed time is within calibrated budget.
        assert!(
            elapsed_us < ack_budget,
            "wall-clock elapsed must be < {}µs (calibrated) — tasks.md §7.2; got {}µs",
            ack_budget,
            elapsed_us
        );

        // Verify pressed state is set in the scene graph.
        let refresh_node_id = result
            .hit
            .node_hit_ids()
            .map(|(_, nid)| nid)
            .expect("PointerDown at Refresh must produce NodeHit — tasks.md §7.2");

        let state = scene
            .hit_region_states
            .get(&refresh_node_id)
            .expect("HitRegionLocalState must exist for refresh node — tasks.md §7.2");

        assert!(
            state.pressed,
            "HitRegionLocalState.pressed must be true after PointerDown — tasks.md §7.2"
        );
    }

    /// Task 7.3 — hovered set on PointerEnter, cleared on PointerLeave.
    ///
    /// Spec §Requirement: HitRegionNode local state transitions
    /// Scenario: Pointer enter/leave updates hovered state
    /// tasks.md §7.3: HitRegionLocalState.hovered is set on PointerEnterEvent
    ///   and cleared on PointerLeaveEvent for both buttons.
    ///
    /// We drive the InputProcessor with Move events (which trigger enter/leave
    /// transitions) — Move over Refresh, then Move off to an empty area.
    #[test]
    fn test_hovered_state_set_on_pointer_enter_cleared_on_pointer_leave() {
        use tze_hud_input::{InputProcessor, PointerEvent, PointerEventKind};

        let (mut scene, _tile_id) = build_dashboard_scene();
        let mut processor = InputProcessor::new();

        let refresh_x = crate::TILE_X + 16.0 + 88.0; // centre of Refresh
        let refresh_y = crate::TILE_Y + 256.0 + 18.0;

        // ── Step 1: Move pointer over Refresh → hovered = true ─────────────
        let enter_event = PointerEvent {
            x: refresh_x,
            y: refresh_y,
            kind: PointerEventKind::Move,
            device_id: 0,
            timestamp: None,
        };
        let enter_result = processor.process(&enter_event, &mut scene);

        let refresh_node_id = enter_result
            .hit
            .node_hit_ids()
            .map(|(_, nid)| nid)
            .expect("Move over Refresh must hit HitRegionNode — tasks.md §7.3");

        let state_after_enter = scene
            .hit_region_states
            .get(&refresh_node_id)
            .expect("HitRegionLocalState must exist — tasks.md §7.3");
        assert!(
            state_after_enter.hovered,
            "hovered must be true after pointer enters Refresh — tasks.md §7.3"
        );

        // ── Step 2: Move pointer off tile → hovered = false ─────────────────
        let leave_event = PointerEvent {
            x: 0.0, // off all tiles
            y: 0.0,
            kind: PointerEventKind::Move,
            device_id: 0,
            timestamp: None,
        };
        processor.process(&leave_event, &mut scene);

        let state_after_leave = scene
            .hit_region_states
            .get(&refresh_node_id)
            .expect("HitRegionLocalState must still exist — tasks.md §7.3");
        assert!(
            !state_after_leave.hovered,
            "hovered must be false after pointer leaves Refresh — tasks.md §7.3"
        );
    }

    /// Task 7.4 — PointerUpEvent with release_on_up = true clears pressed + releases capture.
    ///
    /// Spec §Requirement: Pointer capture release on up
    /// Scenario: PointerUp with release_on_up clears pressed state
    /// tasks.md §7.4: PointerUpEvent with release_on_up = true clears pressed
    ///   state and releases pointer capture.
    ///
    /// We set `auto_capture = true` and `release_on_up = true` on the Refresh
    /// HitRegionNode so that PointerDown automatically acquires capture and
    /// PointerUp automatically releases it.
    #[test]
    fn test_pointer_up_with_release_on_up_clears_pressed_and_releases_capture() {
        use tze_hud_input::{InputProcessor, PointerEvent, PointerEventKind};
        use tze_hud_scene::types::NodeData;

        let (mut scene, tile_id) = build_dashboard_scene();
        let mut processor = InputProcessor::new();

        // Find the refresh button node id and configure release_on_up + auto_capture.
        // We do this by finding it in the scene and rebuilding with the right flags.
        let root_id = scene
            .tiles
            .get(&tile_id)
            .and_then(|t| t.root_node)
            .expect("tile must have root");
        let root = scene.nodes.get(&root_id).expect("root exists");
        let refresh_node_id = root
            .children
            .iter()
            .find(|cid| {
                scene
                    .nodes
                    .get(*cid)
                    .and_then(|n| {
                        if let NodeData::HitRegion(hr) = &n.data {
                            if hr.interaction_id == "refresh-button" {
                                Some(())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .is_some()
            })
            .copied()
            .expect("refresh-button must exist");

        // Mutate the HitRegionNode to set release_on_up=true, auto_capture=true.
        if let Some(n) = scene.nodes.get_mut(&refresh_node_id) {
            if let NodeData::HitRegion(ref mut hr) = n.data {
                hr.release_on_up = true;
                hr.auto_capture = true;
            }
        }

        let hit_x = crate::TILE_X + 16.0 + 88.0;
        let hit_y = crate::TILE_Y + 256.0 + 18.0;

        // ── Step 1: PointerDown → pressed = true, capture acquired ──────────
        let down = PointerEvent {
            x: hit_x,
            y: hit_y,
            kind: PointerEventKind::Down,
            device_id: 1,
            timestamp: None,
        };
        processor.process(&down, &mut scene);

        let state_after_down = scene
            .hit_region_states
            .get(&refresh_node_id)
            .expect("HitRegionLocalState exists after down");
        assert!(
            state_after_down.pressed,
            "pressed must be true after PointerDown — tasks.md §7.4"
        );
        assert!(
            processor.capture.is_captured(1),
            "capture must be acquired on PointerDown with auto_capture — tasks.md §7.4"
        );

        // ── Step 2: PointerUp → pressed = false, capture released ───────────
        let up = PointerEvent {
            x: hit_x,
            y: hit_y,
            kind: PointerEventKind::Up,
            device_id: 1,
            timestamp: None,
        };
        processor.process(&up, &mut scene);

        let state_after_up = scene
            .hit_region_states
            .get(&refresh_node_id)
            .expect("HitRegionLocalState exists after up");
        assert!(
            !state_after_up.pressed,
            "pressed must be false after PointerUp — tasks.md §7.4"
        );
        assert!(
            !processor.capture.is_captured(1),
            "capture must be released after PointerUp with release_on_up — tasks.md §7.4"
        );
    }

    /// Task 7.5 — focus ring is rendered when focus transfers to a HitRegionNode.
    ///
    /// Spec §Requirement: Focus ring rendering
    /// Scenario: Click-to-focus sets focused state on HitRegionNode
    /// tasks.md §7.5: focus ring is rendered when focus transfers to a
    ///   HitRegionNode via Tab key or click.
    ///
    /// We verify that:
    ///   1. `process_with_focus` on PointerDown sets `focused = true` in
    ///      `HitRegionLocalState` for the hit node.
    ///   2. The FocusTransition returned by process_with_focus carries the
    ///      correct FocusGainedEvent.
    #[test]
    fn test_focus_ring_rendered_on_click_to_focus() {
        use tze_hud_input::{FocusManager, InputProcessor, PointerEvent, PointerEventKind};

        let (mut scene, tile_id) = build_dashboard_scene();
        let tab_id = scene.active_tab.expect("active_tab must be set");
        let mut processor = InputProcessor::new();
        let mut focus_manager = FocusManager::new();
        focus_manager.add_tab(tab_id);

        let hit_x = crate::TILE_X + 16.0 + 88.0; // Refresh centre
        let hit_y = crate::TILE_Y + 256.0 + 18.0;

        let down = PointerEvent {
            x: hit_x,
            y: hit_y,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };

        let (_result, focus_transition) =
            processor.process_with_focus(&down, &mut scene, &mut focus_manager, tab_id);

        // tasks.md §7.5: process_with_focus must return a FocusTransition with a
        // FocusGainedEvent for the clicked node.
        let transition = focus_transition
            .expect("click on HitRegionNode must produce FocusTransition — tasks.md §7.5");

        let (gained_ev, _) = transition
            .gained
            .as_ref()
            .expect("FocusTransition must have 'gained' — tasks.md §7.5");

        let gained_node_id = gained_ev
            .node_id
            .expect("FocusGainedEvent must carry node_id — tasks.md §7.5");

        // Verify focused state is reflected in HitRegionLocalState.
        let state = scene
            .hit_region_states
            .get(&gained_node_id)
            .expect("HitRegionLocalState must exist for focused node — tasks.md §7.5");

        assert!(
            state.focused,
            "HitRegionLocalState.focused must be true after click-to-focus — tasks.md §7.5"
        );
    }

    // ── Phase 8: Agent Event Handling tests ───────────────────────────────────

    /// Helper: build a `HudSessionImpl` + TCP server + client, returning an
    /// event injector closure and shared state so callers can inject input events.
    ///
    /// Returns:
    ///   - gRPC client for connecting agent sessions
    ///   - server JoinHandle (abort to shut down)
    ///   - `inject_fn`: closure that calls `inject_input_event(namespace, batch)`
    ///   - `state`: shared state Arc for scene inspection
    async fn setup_test_with_inject() -> (
        tze_hud_protocol::proto::session::hud_session_client::HudSessionClient<
            tonic::transport::Channel,
        >,
        tokio::task::JoinHandle<()>,
        // Inject function: (namespace, EventBatch) → sent count
        Box<dyn Fn(String, tze_hud_protocol::proto::EventBatch) -> usize + Send + Sync>,
        std::sync::Arc<tokio::sync::Mutex<tze_hud_protocol::session::SharedState>>,
    ) {
        use tze_hud_protocol::proto::session::hud_session_server::HudSessionServer;
        use tze_hud_protocol::session_server::HudSessionImpl;
        use tze_hud_scene::graph::SceneGraph;

        let scene = SceneGraph::new(1920.0, 1080.0);
        let service = HudSessionImpl::new(scene, TEST_PSK);

        // Clone the broadcast sender BEFORE moving service into the server.
        // `broadcast::Sender` is Clone — cloning gives another handle to the
        // same channel, so `inject_input_event` still reaches all subscribers.
        let input_event_tx = service.input_event_tx.clone();
        let state = service.state.clone();

        let listener = tokio::net::TcpListener::bind("[::1]:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");

        let handle = tokio::spawn(async move {
            let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
            tonic::transport::Server::builder()
                .add_service(HudSessionServer::new(service))
                .serve_with_incoming(incoming)
                .await
                .expect("server");
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        #[allow(deprecated)]
        let client =
            tze_hud_protocol::proto::session::hud_session_client::HudSessionClient::connect(
                format!("http://[::1]:{}", addr.port()),
            )
            .await
            .expect("connect");

        // Wrap the sender in a closure that mimics `inject_input_event`.
        let inject_fn = Box::new(
            move |namespace: String, batch: tze_hud_protocol::proto::EventBatch| -> usize {
                input_event_tx.send((namespace, batch)).unwrap_or_default()
            },
        );

        (client, handle, inject_fn, state)
    }

    /// Helper: perform handshake and subscribe to INPUT_EVENTS.
    async fn handshake_with_input_events(
        client: &mut tze_hud_protocol::proto::session::hud_session_client::HudSessionClient<
            tonic::transport::Channel,
        >,
        agent_id: &str,
    ) -> (
        tokio::sync::mpsc::Sender<tze_hud_protocol::proto::session::ClientMessage>,
        tonic::Streaming<tze_hud_protocol::proto::session::ServerMessage>,
    ) {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;

        let (tx, rx) = tokio::sync::mpsc::channel::<sp::ClientMessage>(64);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        let now_us = crate::now_wall_us();
        tx.send(sp::ClientMessage {
            sequence: 1,
            timestamp_wall_us: now_us,
            payload: Some(sp::client_message::Payload::SessionInit(sp::SessionInit {
                agent_id: agent_id.to_string(),
                agent_display_name: agent_id.to_string(),
                pre_shared_key: TEST_PSK.to_string(),
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                    "access_input_events".to_string(),
                ],
                // Subscribe to INPUT_EVENTS to receive ClickEvent / CommandInputEvent.
                initial_subscriptions: vec![
                    "LEASE_CHANGES".to_string(),
                    "INPUT_EVENTS".to_string(),
                ],
                resume_token: vec![],
                agent_timestamp_wall_us: now_us,
                min_protocol_version: 1000,
                max_protocol_version: 1001,
                auth_credential: None,
            })),
        })
        .await
        .unwrap();

        let mut stream = client.session(stream).await.unwrap().into_inner();

        // Drain SessionEstablished + SceneSnapshot.
        for _ in 0..2 {
            stream.next().await.expect("stream open").expect("no error");
        }

        (tx, stream)
    }

    /// Task 8.4 — click on Refresh dispatches ClickEvent with correct fields.
    ///
    /// Spec §Requirement: Agent Callbacks on Button Activation
    /// Scenario: Click on Refresh dispatches ClickEvent to agent
    /// tasks.md §8.4: click on Refresh dispatches ClickEvent with correct
    ///   interaction_id = "refresh-button" to agent.
    ///
    /// We inject a ClickEvent into the session's input channel using
    /// `inject_input_event` and verify the agent receives it with the correct
    /// interaction_id.
    #[tokio::test]
    async fn test_click_on_refresh_dispatches_click_event_with_correct_interaction_id() {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;
        use tze_hud_protocol::proto::{
            ClickEvent, EventBatch, InputEnvelope, input_envelope::Event,
        };

        let (mut client, server, inject_fn, state) = setup_test_with_inject().await;

        // Register a tab and namespace for the session.
        {
            let st = state.lock().await;
            let mut scene = st.scene.lock().await;
            let tab_id = scene.create_tab("Main", 0).expect("create_tab");
            scene.active_tab = Some(tab_id);
        }

        let (tx, mut stream) =
            handshake_with_input_events(&mut client, "refresh-click-test-agent").await;

        // Determine the agent's namespace from SessionEstablished.
        // The namespace is the agent_id by convention in dev-mode.
        let namespace = "refresh-click-test-agent".to_string();

        // Build a synthetic tile_id and node_id.
        let tile_id_bytes = uuid::Uuid::now_v7().as_bytes().to_vec();
        let node_id_bytes = uuid::Uuid::now_v7().as_bytes().to_vec();

        // Inject a ClickEvent with interaction_id = "refresh-button".
        let batch = EventBatch {
            frame_number: 1,
            batch_ts_us: crate::now_wall_us(),
            events: vec![InputEnvelope {
                event: Some(Event::Click(ClickEvent {
                    tile_id: tile_id_bytes.clone(),
                    node_id: node_id_bytes.clone(),
                    interaction_id: "refresh-button".to_string(),
                    timestamp_mono_us: 0,
                    device_id: "mouse-0".to_string(),
                    local_x: 104.0,
                    local_y: 274.0,
                    button: 0,
                })),
            }],
        };

        inject_fn(namespace.clone(), batch);

        // Agent must receive EventBatch with the ClickEvent.
        // Drain messages; skip non-EventBatch messages (e.g. LeaseStateChange).
        let received_batch = loop {
            let msg = tokio::time::timeout(tokio::time::Duration::from_millis(500), stream.next())
                .await
                .expect("timed out waiting for EventBatch — tasks.md §8.4")
                .expect("stream ended")
                .expect("stream error");

            if let Some(sp::server_message::Payload::EventBatch(batch)) = msg.payload {
                break batch;
            }
        };

        assert_eq!(
            received_batch.events.len(),
            1,
            "EventBatch must contain exactly 1 event — tasks.md §8.4"
        );

        match &received_batch.events[0].event {
            Some(Event::Click(click)) => {
                assert_eq!(
                    click.interaction_id, "refresh-button",
                    "ClickEvent.interaction_id must be 'refresh-button' — tasks.md §8.4"
                );
                assert_eq!(
                    click.tile_id, tile_id_bytes,
                    "ClickEvent.tile_id must match injected value — tasks.md §8.4"
                );
                assert_eq!(
                    click.node_id, node_id_bytes,
                    "ClickEvent.node_id must match injected value — tasks.md §8.4"
                );
            }
            other => panic!("Expected ClickEvent in EventBatch, got: {other:?} — tasks.md §8.4"),
        }

        drop(tx);
        server.abort();
    }

    /// Task 8.5 — ACTIVATE command on focused Dismiss dispatches CommandInputEvent.
    ///
    /// Spec §Requirement: Agent Callbacks on Button Activation
    /// Scenario: ACTIVATE on focused Dismiss dispatches CommandInputEvent
    /// tasks.md §8.5: ACTIVATE command on focused Dismiss button dispatches
    ///   CommandInputEvent with action = ACTIVATE and interaction_id = "dismiss-button".
    #[tokio::test]
    async fn test_activate_command_on_dismiss_dispatches_command_input_event() {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;
        use tze_hud_protocol::proto::{
            CommandAction, CommandInputEvent, EventBatch, InputEnvelope, input_envelope::Event,
        };

        let (mut client, server, inject_fn, state) = setup_test_with_inject().await;

        {
            let st = state.lock().await;
            let mut scene = st.scene.lock().await;
            let tab_id = scene.create_tab("Main", 0).expect("create_tab");
            scene.active_tab = Some(tab_id);
        }

        let (tx, mut stream) =
            handshake_with_input_events(&mut client, "dismiss-activate-test-agent").await;

        let namespace = "dismiss-activate-test-agent".to_string();
        let tile_id_bytes = uuid::Uuid::now_v7().as_bytes().to_vec();
        let node_id_bytes = uuid::Uuid::now_v7().as_bytes().to_vec();

        // Inject a CommandInputEvent(ACTIVATE) with interaction_id = "dismiss-button".
        let batch = EventBatch {
            frame_number: 2,
            batch_ts_us: crate::now_wall_us(),
            events: vec![InputEnvelope {
                event: Some(Event::CommandInput(CommandInputEvent {
                    tile_id: tile_id_bytes.clone(),
                    node_id: node_id_bytes.clone(),
                    interaction_id: "dismiss-button".to_string(),
                    timestamp_mono_us: 0,
                    device_id: "keyboard-0".to_string(),
                    action: CommandAction::Activate as i32,
                    source: 0,
                })),
            }],
        };

        inject_fn(namespace.clone(), batch);

        // Drain until EventBatch arrives.
        let received_batch = loop {
            let msg = tokio::time::timeout(tokio::time::Duration::from_millis(500), stream.next())
                .await
                .expect("timed out waiting for EventBatch — tasks.md §8.5")
                .expect("stream ended")
                .expect("stream error");

            if let Some(sp::server_message::Payload::EventBatch(b)) = msg.payload {
                break b;
            }
        };

        assert_eq!(
            received_batch.events.len(),
            1,
            "EventBatch must have 1 event — tasks.md §8.5"
        );

        match &received_batch.events[0].event {
            Some(Event::CommandInput(cmd)) => {
                assert_eq!(
                    cmd.interaction_id, "dismiss-button",
                    "CommandInputEvent.interaction_id must be 'dismiss-button' — tasks.md §8.5"
                );
                assert_eq!(
                    cmd.action,
                    CommandAction::Activate as i32,
                    "CommandInputEvent.action must be ACTIVATE — tasks.md §8.5"
                );
            }
            other => panic!(
                "Expected CommandInputEvent(ACTIVATE) in EventBatch, got: {other:?} — tasks.md §8.5"
            ),
        }

        drop(tx);
        server.abort();
    }

    /// Task 8.6 — keyboard-only path: NAVIGATE_NEXT + ACTIVATE reaches all buttons.
    ///
    /// Spec §Requirement: Agent Callbacks on Button Activation
    /// Scenario: All buttons reachable via keyboard navigation
    /// tasks.md §8.6: all buttons are reachable and activatable without a pointer
    ///   (NAVIGATE_NEXT + ACTIVATE).
    ///
    /// We inject a NAVIGATE_NEXT command (Tab key) followed by ACTIVATE on both
    /// buttons and verify the agent receives CommandInputEvents for each.
    #[tokio::test]
    async fn test_navigate_next_plus_activate_reaches_both_buttons() {
        use tokio_stream::StreamExt as _;
        use tze_hud_protocol::proto::session as sp;
        use tze_hud_protocol::proto::{
            CommandAction, CommandInputEvent, EventBatch, InputEnvelope, input_envelope::Event,
        };

        let (mut client, server, inject_fn, state) = setup_test_with_inject().await;

        {
            let st = state.lock().await;
            let mut scene = st.scene.lock().await;
            let tab_id = scene.create_tab("Main", 0).expect("create_tab");
            scene.active_tab = Some(tab_id);
        }

        let (tx, mut stream) =
            handshake_with_input_events(&mut client, "keyboard-nav-test-agent").await;

        let namespace = "keyboard-nav-test-agent".to_string();
        let tile_id_bytes = uuid::Uuid::now_v7().as_bytes().to_vec();
        let refresh_node_id = uuid::Uuid::now_v7().as_bytes().to_vec();
        let dismiss_node_id = uuid::Uuid::now_v7().as_bytes().to_vec();

        // Inject: NAVIGATE_NEXT to Refresh, then ACTIVATE on Refresh.
        let nav_refresh_batch = EventBatch {
            frame_number: 3,
            batch_ts_us: crate::now_wall_us(),
            events: vec![
                InputEnvelope {
                    event: Some(Event::CommandInput(CommandInputEvent {
                        tile_id: tile_id_bytes.clone(),
                        node_id: refresh_node_id.clone(),
                        interaction_id: "refresh-button".to_string(),
                        timestamp_mono_us: 1,
                        device_id: "keyboard-0".to_string(),
                        action: CommandAction::NavigateNext as i32,
                        source: 0,
                    })),
                },
                InputEnvelope {
                    event: Some(Event::CommandInput(CommandInputEvent {
                        tile_id: tile_id_bytes.clone(),
                        node_id: refresh_node_id.clone(),
                        interaction_id: "refresh-button".to_string(),
                        timestamp_mono_us: 2,
                        device_id: "keyboard-0".to_string(),
                        action: CommandAction::Activate as i32,
                        source: 0,
                    })),
                },
            ],
        };

        inject_fn(namespace.clone(), nav_refresh_batch);

        // Wait for EventBatch with NAVIGATE_NEXT + ACTIVATE for Refresh.
        let batch1 = loop {
            let msg = tokio::time::timeout(tokio::time::Duration::from_millis(500), stream.next())
                .await
                .expect("timed out waiting for nav-refresh batch — tasks.md §8.6")
                .expect("stream ended")
                .expect("stream error");

            if let Some(sp::server_message::Payload::EventBatch(b)) = msg.payload {
                break b;
            }
        };

        assert_eq!(
            batch1.events.len(),
            2,
            "nav-refresh batch must have 2 events (NAVIGATE_NEXT + ACTIVATE) — tasks.md §8.6"
        );

        // Verify NAVIGATE_NEXT event.
        match &batch1.events[0].event {
            Some(Event::CommandInput(cmd)) => {
                assert_eq!(
                    cmd.action,
                    CommandAction::NavigateNext as i32,
                    "first event must be NAVIGATE_NEXT — tasks.md §8.6"
                );
                assert_eq!(cmd.interaction_id, "refresh-button");
            }
            other => panic!("Expected CommandInput(NAVIGATE_NEXT), got: {other:?}"),
        }

        // Verify ACTIVATE event on Refresh.
        match &batch1.events[1].event {
            Some(Event::CommandInput(cmd)) => {
                assert_eq!(
                    cmd.action,
                    CommandAction::Activate as i32,
                    "second event must be ACTIVATE — tasks.md §8.6"
                );
                assert_eq!(cmd.interaction_id, "refresh-button");
            }
            other => panic!("Expected CommandInput(ACTIVATE) on refresh, got: {other:?}"),
        }

        // Inject: NAVIGATE_NEXT to Dismiss, then ACTIVATE on Dismiss.
        let nav_dismiss_batch = EventBatch {
            frame_number: 4,
            batch_ts_us: crate::now_wall_us(),
            events: vec![
                InputEnvelope {
                    event: Some(Event::CommandInput(CommandInputEvent {
                        tile_id: tile_id_bytes.clone(),
                        node_id: dismiss_node_id.clone(),
                        interaction_id: "dismiss-button".to_string(),
                        timestamp_mono_us: 3,
                        device_id: "keyboard-0".to_string(),
                        action: CommandAction::NavigateNext as i32,
                        source: 0,
                    })),
                },
                InputEnvelope {
                    event: Some(Event::CommandInput(CommandInputEvent {
                        tile_id: tile_id_bytes.clone(),
                        node_id: dismiss_node_id.clone(),
                        interaction_id: "dismiss-button".to_string(),
                        timestamp_mono_us: 4,
                        device_id: "keyboard-0".to_string(),
                        action: CommandAction::Activate as i32,
                        source: 0,
                    })),
                },
            ],
        };

        inject_fn(namespace.clone(), nav_dismiss_batch);

        let batch2 = loop {
            let msg = tokio::time::timeout(tokio::time::Duration::from_millis(500), stream.next())
                .await
                .expect("timed out waiting for nav-dismiss batch — tasks.md §8.6")
                .expect("stream ended")
                .expect("stream error");

            if let Some(sp::server_message::Payload::EventBatch(b)) = msg.payload {
                break b;
            }
        };

        assert_eq!(
            batch2.events.len(),
            2,
            "nav-dismiss batch must have 2 events — tasks.md §8.6"
        );

        // Verify ACTIVATE on Dismiss button.
        match &batch2.events[1].event {
            Some(Event::CommandInput(cmd)) => {
                assert_eq!(
                    cmd.action,
                    CommandAction::Activate as i32,
                    "second event must be ACTIVATE on dismiss — tasks.md §8.6"
                );
                assert_eq!(cmd.interaction_id, "dismiss-button");
            }
            other => panic!("Expected CommandInput(ACTIVATE) on dismiss, got: {other:?}"),
        }

        drop(tx);
        server.abort();
    }

    // ── Phase 8: handle_event_batch unit tests ────────────────────────────────

    /// Task 8.1 — handle_event_batch extracts ClickEvent and returns RefreshContent.
    ///
    /// tasks.md §8.1: receive EventBatch, extract ClickEvent.
    /// tasks.md §8.2: ClickEvent on "refresh-button" → RefreshContent action.
    #[test]
    fn test_handle_event_batch_click_refresh_returns_refresh_content() {
        use tze_hud_protocol::proto::{
            ClickEvent, EventBatch, InputEnvelope, input_envelope::Event,
        };

        let batch = EventBatch {
            frame_number: 1,
            batch_ts_us: 0,
            events: vec![InputEnvelope {
                event: Some(Event::Click(ClickEvent {
                    tile_id: vec![],
                    node_id: vec![],
                    interaction_id: "refresh-button".to_string(),
                    timestamp_mono_us: 0,
                    device_id: String::new(),
                    local_x: 0.0,
                    local_y: 0.0,
                    button: 0,
                })),
            }],
        };

        let actions = crate::handle_event_batch(&batch);
        assert_eq!(
            actions,
            vec![crate::AgentAction::RefreshContent],
            "ClickEvent on 'refresh-button' must return RefreshContent — tasks.md §8.1, §8.2"
        );
    }

    /// Task 8.1, 8.3 — handle_event_batch extracts ClickEvent and returns Dismiss.
    ///
    /// tasks.md §8.1: receive EventBatch, extract ClickEvent.
    /// tasks.md §8.3: ClickEvent on "dismiss-button" → Dismiss action.
    #[test]
    fn test_handle_event_batch_click_dismiss_returns_dismiss() {
        use tze_hud_protocol::proto::{
            ClickEvent, EventBatch, InputEnvelope, input_envelope::Event,
        };

        let batch = EventBatch {
            frame_number: 1,
            batch_ts_us: 0,
            events: vec![InputEnvelope {
                event: Some(Event::Click(ClickEvent {
                    tile_id: vec![],
                    node_id: vec![],
                    interaction_id: "dismiss-button".to_string(),
                    timestamp_mono_us: 0,
                    device_id: String::new(),
                    local_x: 0.0,
                    local_y: 0.0,
                    button: 0,
                })),
            }],
        };

        let actions = crate::handle_event_batch(&batch);
        assert_eq!(
            actions,
            vec![crate::AgentAction::Dismiss],
            "ClickEvent on 'dismiss-button' must return Dismiss — tasks.md §8.1, §8.3"
        );
    }

    /// Task 8.1, 8.5 — handle_event_batch extracts CommandInputEvent(ACTIVATE) → Dismiss.
    ///
    /// tasks.md §8.1: extract CommandInputEvent(ACTIVATE).
    /// tasks.md §8.5: ACTIVATE on "dismiss-button" → Dismiss action.
    #[test]
    fn test_handle_event_batch_command_activate_dismiss_returns_dismiss() {
        use tze_hud_protocol::proto::{
            CommandAction, CommandInputEvent, EventBatch, InputEnvelope, input_envelope::Event,
        };

        let batch = EventBatch {
            frame_number: 1,
            batch_ts_us: 0,
            events: vec![InputEnvelope {
                event: Some(Event::CommandInput(CommandInputEvent {
                    tile_id: vec![],
                    node_id: vec![],
                    interaction_id: "dismiss-button".to_string(),
                    timestamp_mono_us: 0,
                    device_id: String::new(),
                    action: CommandAction::Activate as i32,
                    source: 0,
                })),
            }],
        };

        let actions = crate::handle_event_batch(&batch);
        assert_eq!(
            actions,
            vec![crate::AgentAction::Dismiss],
            "CommandInput(ACTIVATE) on 'dismiss-button' must return Dismiss — tasks.md §8.1, §8.5"
        );
    }

    /// Task 8.6 event routing — NAVIGATE_NEXT is not an activation.
    ///
    /// tasks.md §8.6: NAVIGATE_NEXT is navigation, not activation — handle_event_batch
    ///   must NOT return RefreshContent or Dismiss for NAVIGATE_NEXT.
    #[test]
    fn test_handle_event_batch_navigate_next_is_not_activation() {
        use tze_hud_protocol::proto::{
            CommandAction, CommandInputEvent, EventBatch, InputEnvelope, input_envelope::Event,
        };

        let batch = EventBatch {
            frame_number: 1,
            batch_ts_us: 0,
            events: vec![InputEnvelope {
                event: Some(Event::CommandInput(CommandInputEvent {
                    tile_id: vec![],
                    node_id: vec![],
                    interaction_id: "refresh-button".to_string(),
                    timestamp_mono_us: 0,
                    device_id: String::new(),
                    action: CommandAction::NavigateNext as i32,
                    source: 0,
                })),
            }],
        };

        let actions = crate::handle_event_batch(&batch);
        assert!(
            actions.is_empty(),
            "NAVIGATE_NEXT must NOT produce an AgentAction — tasks.md §8.6; got: {actions:?}"
        );
    }
}
