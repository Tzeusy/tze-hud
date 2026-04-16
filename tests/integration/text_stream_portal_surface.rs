//! Resident raw-tile text stream portal pilot surface tests (hud-t98e.2).
//!
//! Covers the phase-0 surface requirements:
//! - collapsed + expanded portal surfaces built only from v1 node types
//! - bounded transcript materialization for expanded viewport
//! - content-layer governance compatibility (privacy redaction + lease/orphan path)

use image::{ImageBuffer, Rgb};
use tze_hud_resource::{
    AgentBudget, CAPABILITY_UPLOAD_RESOURCE, ResourceStore, ResourceStoreConfig, ResourceType,
    UploadId, UploadStartRequest,
};
use tze_hud_runtime::{
    ContentClassification, RedactionStyle, TileRedactionState, ViewerClass, build_redaction_cmds,
    hit_regions_enabled, is_tile_redacted,
};
use tze_hud_scene::{
    Capability, MAX_MARKDOWN_BYTES, ZONE_TILE_Z_MIN,
    graph::{MAX_NODES_PER_TILE, SceneGraph},
    lease::LeaseState,
    mutation::{MutationBatch, SceneMutation},
    types::{
        FontFamily, HitRegionNode, ImageFitMode, InputMode, Node, NodeData, Rect, SceneId,
        SolidColorNode, TextAlign, TextMarkdownNode, TextOverflow, TileScrollConfig,
    },
};

const DISPLAY_W: f32 = 1920.0;
const DISPLAY_H: f32 = 1080.0;

const COLLAPSED_W: f32 = 420.0;
const COLLAPSED_H: f32 = 96.0;
const EXPANDED_W: f32 = 720.0;
const EXPANDED_H: f32 = 360.0;
const PORTAL_Z_ORDER: u32 = 160;

const ICON_W: u32 = 24;
const ICON_H: u32 = 24;

const INTERACTION_EXPAND: &str = "portal.expand";
const INTERACTION_COLLAPSE: &str = "portal.collapse";
const INTERACTION_REPLY: &str = "portal.reply";

#[derive(Clone, Debug)]
struct PortalSurfaceState {
    portal_id: String,
    session_title: String,
    history: Vec<String>,
    unread_count: usize,
    expanded: bool,
    viewport_start_line: usize,
    viewport_max_lines: usize,
}

impl PortalSurfaceState {
    fn activity_text(&self) -> String {
        if self.unread_count == 0 {
            "idle".to_string()
        } else {
            format!("{} unread", self.unread_count)
        }
    }

    fn bounded_transcript_markdown(&self) -> String {
        let end = (self.viewport_start_line + self.viewport_max_lines).min(self.history.len());
        let mut start = self.viewport_start_line.min(end);
        loop {
            let joined = self.history[start..end].join("\n");
            if joined.len() <= MAX_MARKDOWN_BYTES || start + 1 >= end {
                return joined;
            }
            start += 1;
        }
    }
}

fn make_batch(namespace: &str, lease_id: SceneId, mutations: Vec<SceneMutation>) -> MutationBatch {
    MutationBatch {
        batch_id: SceneId::new(),
        agent_namespace: namespace.to_string(),
        mutations,
        timing_hints: None,
        lease_id: Some(lease_id),
    }
}

async fn upload_png_icon(
    store: &ResourceStore,
    agent_namespace: &str,
) -> tze_hud_scene::ResourceId {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_fn(ICON_W, ICON_H, |_, _| Rgb([32, 178, 170]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("portal icon fixture must encode as PNG");

    let hash = *blake3::hash(&png).as_bytes();
    let upload_id = UploadId::from_bytes(uuid::Uuid::now_v7().into_bytes());
    let stored = store
        .handle_upload_start(UploadStartRequest {
            agent_namespace: agent_namespace.to_string(),
            agent_capabilities: vec![CAPABILITY_UPLOAD_RESOURCE.to_string()],
            agent_budget: AgentBudget {
                texture_bytes_total_limit: 0,
                texture_bytes_total_used: 0,
            },
            upload_id,
            resource_type: ResourceType::ImagePng,
            expected_hash: hash,
            total_size: png.len(),
            inline_data: png,
            width: ICON_W,
            height: ICON_H,
        })
        .await
        .expect("portal icon upload must succeed")
        .expect("inline portal icon upload must complete immediately");
    tze_hud_scene::ResourceId::from_bytes(*stored.resource_id.as_bytes())
}

fn portal_bounds(expanded: bool) -> Rect {
    if expanded {
        Rect::new(48.0, 160.0, EXPANDED_W, EXPANDED_H)
    } else {
        Rect::new(48.0, 160.0, COLLAPSED_W, COLLAPSED_H)
    }
}

fn build_collapsed_nodes(
    state: &PortalSurfaceState,
    icon_id: tze_hud_scene::ResourceId,
) -> Vec<Node> {
    let root = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::SolidColor(SolidColorNode {
            color: tze_hud_scene::Rgba::new(0.10, 0.12, 0.16, 0.88),
            bounds: Rect::new(0.0, 0.0, COLLAPSED_W, COLLAPSED_H),
        }),
    };
    let title = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: format!("**{}**", state.session_title),
            bounds: Rect::new(44.0, 10.0, COLLAPSED_W - 120.0, 24.0),
            font_size_px: 15.0,
            font_family: FontFamily::SystemSansSerif,
            color: tze_hud_scene::Rgba::new(0.96, 0.98, 1.0, 1.0),
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Ellipsis,
        }),
    };
    let preview = state
        .history
        .last()
        .cloned()
        .unwrap_or_else(|| "<empty stream>".to_string());
    let preview = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: preview,
            bounds: Rect::new(44.0, 40.0, COLLAPSED_W - 120.0, 18.0),
            font_size_px: 12.0,
            font_family: FontFamily::SystemMonospace,
            color: tze_hud_scene::Rgba::new(0.82, 0.88, 0.94, 0.96),
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Ellipsis,
        }),
    };
    let activity = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: state.activity_text(),
            bounds: Rect::new(COLLAPSED_W - 130.0, 64.0, 80.0, 18.0),
            font_size_px: 11.0,
            font_family: FontFamily::SystemSansSerif,
            color: tze_hud_scene::Rgba::new(0.48, 0.95, 0.68, 0.96),
            background: None,
            alignment: TextAlign::End,
            overflow: TextOverflow::Clip,
        }),
    };
    let icon = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::StaticImage(tze_hud_scene::StaticImageNode {
            resource_id: icon_id,
            width: ICON_W,
            height: ICON_H,
            decoded_bytes: (ICON_W as u64) * (ICON_H as u64) * 4,
            fit_mode: ImageFitMode::Contain,
            bounds: Rect::new(12.0, 10.0, 24.0, 24.0),
        }),
    };
    let expand_hit = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::HitRegion(HitRegionNode {
            bounds: Rect::new(COLLAPSED_W - 46.0, 10.0, 34.0, 24.0),
            interaction_id: INTERACTION_EXPAND.to_string(),
            accepts_focus: true,
            accepts_pointer: true,
            ..Default::default()
        }),
    };

    vec![root, icon, title, preview, activity, expand_hit]
}

fn build_expanded_nodes(
    state: &PortalSurfaceState,
    icon_id: tze_hud_scene::ResourceId,
) -> Vec<Node> {
    let root = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::SolidColor(SolidColorNode {
            color: tze_hud_scene::Rgba::new(0.08, 0.10, 0.13, 0.92),
            bounds: Rect::new(0.0, 0.0, EXPANDED_W, EXPANDED_H),
        }),
    };
    let transcript_text = state.bounded_transcript_markdown();
    let transcript = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: transcript_text,
            bounds: Rect::new(12.0, 44.0, EXPANDED_W - 24.0, EXPANDED_H - 108.0),
            font_size_px: 13.0,
            font_family: FontFamily::SystemMonospace,
            color: tze_hud_scene::Rgba::new(0.90, 0.94, 1.0, 0.98),
            background: Some(tze_hud_scene::Rgba::new(0.03, 0.04, 0.06, 0.78)),
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
        }),
    };
    let title = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: format!("{} · {}", state.portal_id, state.activity_text()),
            bounds: Rect::new(44.0, 10.0, EXPANDED_W - 180.0, 24.0),
            font_size_px: 14.0,
            font_family: FontFamily::SystemSansSerif,
            color: tze_hud_scene::Rgba::new(0.96, 0.98, 1.0, 1.0),
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Ellipsis,
        }),
    };
    let icon = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::StaticImage(tze_hud_scene::StaticImageNode {
            resource_id: icon_id,
            width: ICON_W,
            height: ICON_H,
            decoded_bytes: (ICON_W as u64) * (ICON_H as u64) * 4,
            fit_mode: ImageFitMode::Contain,
            bounds: Rect::new(12.0, 10.0, 24.0, 24.0),
        }),
    };
    let reply_label = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: "Reply".to_string(),
            bounds: Rect::new(EXPANDED_W - 140.0, EXPANDED_H - 38.0, 60.0, 20.0),
            font_size_px: 12.0,
            font_family: FontFamily::SystemSansSerif,
            color: tze_hud_scene::Rgba::new(0.72, 0.86, 1.0, 1.0),
            background: None,
            alignment: TextAlign::Center,
            overflow: TextOverflow::Clip,
        }),
    };
    let collapse_hit = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::HitRegion(HitRegionNode {
            bounds: Rect::new(EXPANDED_W - 46.0, 10.0, 34.0, 24.0),
            interaction_id: INTERACTION_COLLAPSE.to_string(),
            accepts_focus: true,
            accepts_pointer: true,
            ..Default::default()
        }),
    };
    let reply_hit = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::HitRegion(HitRegionNode {
            bounds: Rect::new(EXPANDED_W - 150.0, EXPANDED_H - 42.0, 74.0, 28.0),
            interaction_id: INTERACTION_REPLY.to_string(),
            accepts_focus: true,
            accepts_pointer: true,
            ..Default::default()
        }),
    };

    vec![
        root,
        icon,
        title,
        transcript,
        reply_label,
        collapse_hit,
        reply_hit,
    ]
}

fn root_batch_for_tile(tile_id: SceneId, root: Node, children: Vec<Node>) -> Vec<SceneMutation> {
    let root_id = root.id;
    let mut mutations = vec![SceneMutation::SetTileRoot {
        tile_id,
        node: root.clone(),
    }];
    mutations.extend(children.into_iter().map(|node| SceneMutation::AddNode {
        tile_id,
        parent_id: Some(root_id),
        node,
    }));
    mutations
}

fn materialized_text_nodes(scene: &SceneGraph, tile_id: SceneId) -> Vec<String> {
    let tile = scene.tiles.get(&tile_id).expect("tile must exist");
    let root = tile.root_node.expect("tile must have root node");
    let root_node = scene.nodes.get(&root).expect("root node must exist");
    let mut texts = Vec::new();
    for child in &root_node.children {
        let node = scene.nodes.get(child).expect("child node must exist");
        if let NodeData::TextMarkdown(t) = &node.data {
            texts.push(t.content.clone());
        }
    }
    texts
}

#[tokio::test]
async fn collapsed_and_expanded_portal_surface_use_only_v1_node_types() {
    let namespace = "portal-agent";
    let store = ResourceStore::new(ResourceStoreConfig::default());
    let icon_id = upload_png_icon(&store, namespace).await;

    let mut scene = SceneGraph::new(DISPLAY_W, DISPLAY_H);
    let tab_id = scene.create_tab("Main", 0).expect("must create tab");
    scene.active_tab = Some(tab_id);
    scene.register_resource(icon_id);
    let lease_id = scene.grant_lease(
        namespace,
        120_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );

    let create = scene.apply_batch(&make_batch(
        namespace,
        lease_id,
        vec![SceneMutation::CreateTile {
            tab_id,
            namespace: namespace.to_string(),
            lease_id,
            bounds: portal_bounds(false),
            z_order: PORTAL_Z_ORDER,
        }],
    ));
    assert!(create.applied, "collapsed portal tile must be creatable");
    let tile_id = create.created_ids[0];
    assert!(
        PORTAL_Z_ORDER < ZONE_TILE_Z_MIN,
        "portal pilot tile must stay below runtime-managed zone band"
    );
    assert_ne!(
        scene.leases[&lease_id].priority, 0,
        "portal pilot must remain content-layer, not chrome lease-priority 0"
    );

    let collapsed = PortalSurfaceState {
        portal_id: "portal://pilot/1".to_string(),
        session_title: "Resident Text Stream".to_string(),
        history: vec!["warmup output".to_string(), "portal ready".to_string()],
        unread_count: 1,
        expanded: false,
        viewport_start_line: 0,
        viewport_max_lines: 12,
    };
    assert!(!collapsed.expanded, "collapsed state must be false");
    let mut collapsed_nodes = build_collapsed_nodes(&collapsed, icon_id);
    let collapsed_root = collapsed_nodes.remove(0);
    let collapsed_batch = scene.apply_batch(&make_batch(
        namespace,
        lease_id,
        root_batch_for_tile(tile_id, collapsed_root, collapsed_nodes),
    ));
    assert!(collapsed_batch.applied, "collapsed root batch must apply");

    let collapsed_kinds = materialized_text_nodes(&scene, tile_id);
    assert!(
        collapsed_kinds
            .iter()
            .any(|c| c.contains("Resident Text Stream")),
        "collapsed state must include portal identity text in content layer"
    );
    assert!(
        collapsed_kinds.iter().any(|c| c.contains("unread")),
        "collapsed state must include portal activity text in content layer"
    );

    let expanded = PortalSurfaceState {
        expanded: true,
        viewport_start_line: 0,
        viewport_max_lines: 20,
        ..collapsed
    };
    assert!(expanded.expanded, "expanded state must be true");
    let mut expanded_nodes = build_expanded_nodes(&expanded, icon_id);
    let expanded_root = expanded_nodes.remove(0);
    let expanded_batch = scene.apply_batch(&make_batch(
        namespace,
        lease_id,
        vec![
            SceneMutation::UpdateTileBounds {
                tile_id,
                bounds: portal_bounds(true),
            },
            SceneMutation::UpdateTileInputMode {
                tile_id,
                input_mode: InputMode::Capture,
            },
        ]
        .into_iter()
        .chain(root_batch_for_tile(tile_id, expanded_root, expanded_nodes))
        .collect(),
    ));
    assert!(expanded_batch.applied, "expanded root batch must apply");

    let tile = scene.tiles.get(&tile_id).expect("tile must still exist");
    assert_eq!(tile.bounds, portal_bounds(true));
    assert_eq!(tile.input_mode, InputMode::Capture);

    let root = tile.root_node.expect("expanded tile must have root");
    let root_node = scene.nodes.get(&root).expect("expanded root must exist");
    for child in &root_node.children {
        let node = scene.nodes.get(child).expect("child must exist");
        match node.data {
            NodeData::SolidColor(_)
            | NodeData::TextMarkdown(_)
            | NodeData::StaticImage(_)
            | NodeData::HitRegion(_) => {}
        }
    }
}

#[test]
fn expanded_transcript_materialization_is_bounded_to_viewport_and_budget() {
    let history: Vec<String> = (0..240)
        .map(|i| format!("[{i:03}] {}", "x".repeat(420)))
        .collect();
    let state = PortalSurfaceState {
        portal_id: "portal://pilot/2".to_string(),
        session_title: "Budget Window".to_string(),
        history,
        unread_count: 0,
        expanded: true,
        viewport_start_line: 120,
        viewport_max_lines: 80,
    };
    let markdown = state.bounded_transcript_markdown();
    assert!(
        markdown.len() <= MAX_MARKDOWN_BYTES,
        "materialized transcript must stay within TextMarkdown node byte budget"
    );
    let line_count = markdown.lines().count();
    assert!(
        line_count <= state.viewport_max_lines,
        "materialized transcript must not exceed viewport line window"
    );

    let mut scene = SceneGraph::new(DISPLAY_W, DISPLAY_H);
    let tab_id = scene.create_tab("Main", 0).expect("must create tab");
    scene.active_tab = Some(tab_id);
    let lease_id = scene.grant_lease(
        "portal-agent",
        120_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );
    let create = scene.apply_batch(&make_batch(
        "portal-agent",
        lease_id,
        vec![SceneMutation::CreateTile {
            tab_id,
            namespace: "portal-agent".to_string(),
            lease_id,
            bounds: portal_bounds(true),
            z_order: PORTAL_Z_ORDER,
        }],
    ));
    assert!(create.applied);
    let tile_id = create.created_ids[0];
    scene
        .register_tile_scroll_config(
            tile_id,
            TileScrollConfig {
                scrollable_x: false,
                scrollable_y: true,
                content_width: Some(EXPANDED_W),
                content_height: Some(EXPANDED_H * 2.5),
            },
        )
        .expect("expanded portal tile must allow local-first scroll config");
    let (sx, sy) = scene.tile_scroll_offset_local(tile_id);
    assert_eq!((sx, sy), (0.0, 0.0));
    scene
        .set_tile_scroll_offset_local(tile_id, 0.0, 48.0)
        .expect("local-first scroll offset must be writable");

    let root = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::SolidColor(SolidColorNode {
            color: tze_hud_scene::Rgba::new(0.08, 0.10, 0.13, 0.92),
            bounds: Rect::new(0.0, 0.0, EXPANDED_W, EXPANDED_H),
        }),
    };
    let transcript = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: markdown,
            bounds: Rect::new(12.0, 44.0, EXPANDED_W - 24.0, EXPANDED_H - 108.0),
            font_size_px: 13.0,
            font_family: FontFamily::SystemMonospace,
            color: tze_hud_scene::Rgba::new(0.90, 0.94, 1.0, 0.98),
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
        }),
    };
    let apply = scene.apply_batch(&make_batch(
        "portal-agent",
        lease_id,
        root_batch_for_tile(tile_id, root, vec![transcript]),
    ));
    assert!(
        apply.applied,
        "bounded expanded transcript batch must apply"
    );
    assert!(
        scene.node_count() <= MAX_NODES_PER_TILE,
        "expanded pilot node count must stay under per-tile node budget"
    );
}

#[tokio::test]
async fn portal_surface_state_remains_governed_by_existing_privacy_and_orphan_rules() {
    let namespace = "portal-agent-governed";
    let store = ResourceStore::new(ResourceStoreConfig::default());
    let icon_id = upload_png_icon(&store, namespace).await;

    let mut scene = SceneGraph::new(DISPLAY_W, DISPLAY_H);
    let tab_id = scene.create_tab("Main", 0).expect("must create tab");
    scene.active_tab = Some(tab_id);
    scene.register_resource(icon_id);
    let lease_id = scene.grant_lease(
        namespace,
        120_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );
    let create = scene.apply_batch(&make_batch(
        namespace,
        lease_id,
        vec![SceneMutation::CreateTile {
            tab_id,
            namespace: namespace.to_string(),
            lease_id,
            bounds: portal_bounds(false),
            z_order: PORTAL_Z_ORDER,
        }],
    ));
    assert!(create.applied);
    let tile_id = create.created_ids[0];
    let collapsed = PortalSurfaceState {
        portal_id: "portal://gov/1".to_string(),
        session_title: "Governed".to_string(),
        history: vec!["sensitive response".to_string()],
        unread_count: 2,
        expanded: false,
        viewport_start_line: 0,
        viewport_max_lines: 8,
    };
    let mut nodes = build_collapsed_nodes(&collapsed, icon_id);
    let root = nodes.remove(0);
    assert!(
        scene
            .apply_batch(&make_batch(
                namespace,
                lease_id,
                root_batch_for_tile(tile_id, root, nodes),
            ))
            .applied
    );

    let redacted = is_tile_redacted(ViewerClass::KnownGuest, ContentClassification::Private);
    assert!(
        redacted,
        "portal content must redact under existing privacy policy"
    );
    let redaction_state = TileRedactionState::Redacted {
        classification: ContentClassification::Private,
    };
    assert!(
        !hit_regions_enabled(&redaction_state),
        "redacted portal must not keep hit regions interactive"
    );

    let tile_bounds = scene.tiles.get(&tile_id).expect("tile must exist").bounds;
    let cmds = build_redaction_cmds(tile_bounds, RedactionStyle::Pattern);
    let cover = cmds.iter().any(|cmd| {
        cmd.x == tile_bounds.x
            && cmd.y == tile_bounds.y
            && cmd.width == tile_bounds.width
            && cmd.height == tile_bounds.height
    });
    assert!(
        cover,
        "redaction placeholder must preserve portal geometry while replacing visible content"
    );

    scene
        .disconnect_lease(&lease_id, 10_000)
        .expect("disconnect should transition lease into orphan path");
    assert_eq!(
        scene.leases[&lease_id].state,
        LeaseState::Orphaned,
        "portal lease must enter normal orphan path on disconnect"
    );

    let rejected = scene.apply_batch(&make_batch(
        namespace,
        lease_id,
        vec![SceneMutation::UpdateTileBounds {
            tile_id,
            bounds: Rect::new(48.0, 220.0, COLLAPSED_W, COLLAPSED_H),
        }],
    ));
    assert!(
        !rejected.applied,
        "portal tile must not bypass existing lease/orphan governance after disconnect"
    );
}
