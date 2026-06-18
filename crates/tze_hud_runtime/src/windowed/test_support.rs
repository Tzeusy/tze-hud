use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;
use tze_hud_input::{FocusManager, FocusRequest};
use tze_hud_protocol::session::{RuntimeDegradationLevel, SessionRegistry, SharedState};
use tze_hud_protocol::token::TokenStore;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{
    ContentionPolicy, DragHandleElementKind, GeometryPolicy, LayerAttachment, RenderingPolicy,
    TileScrollConfig, ZoneDefinition, ZoneMediaType,
};
use tze_hud_scene::{
    Capability, DragHandleHitRegion, HitRegionNode, Node, NodeData, Rect, SceneId,
};

pub(super) fn scene_with_capture_tile() -> (SceneGraph, SceneId) {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Main", 0).unwrap();
    let lease_id = scene.grant_lease(
        "portal-agent",
        60_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );
    let tile_id = scene
        .create_tile(
            tab_id,
            "portal-agent",
            lease_id,
            Rect::new(300.0, 400.0, 600.0, 200.0),
            1,
        )
        .unwrap();
    let node_id = SceneId::new();
    scene
        .set_tile_root(
            tile_id,
            Node {
                id: node_id,
                children: vec![],
                data: NodeData::HitRegion(HitRegionNode {
                    bounds: Rect::new(0.0, 0.0, 200.0, 40.0),
                    interaction_id: "portal-submit".to_string(),
                    accepts_pointer: true,
                    accepts_focus: true,
                    ..Default::default()
                }),
            },
        )
        .unwrap();
    (scene, tile_id)
}

pub(super) fn make_shared_state() -> Arc<TokioMutex<SharedState>> {
    let scene = Arc::new(TokioMutex::new(SceneGraph::new(1920.0, 1080.0)));
    let sessions = SessionRegistry::new("test-psk");
    Arc::new(TokioMutex::new(SharedState {
        scene,
        sessions,
        resource_store: tze_hud_resource::ResourceStore::new(
            tze_hud_resource::ResourceStoreConfig::default(),
        ),
        widget_asset_store: tze_hud_protocol::session::WidgetAssetStore::default(),
        runtime_widget_store: None,
        element_store: tze_hud_scene::element_store::ElementStore::default(),
        element_store_path: None,
        safe_mode_atomic: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        active_tab_mirror: Arc::new(std::sync::Mutex::new(None)),
        token_store: TokenStore::new(),
        freeze_active: false,
        degradation_level: RuntimeDegradationLevel::Normal,
        media_ingress_active: None,
        input_capture_tx: None,
    }))
}

pub(super) fn make_test_zone(name: &str) -> ZoneDefinition {
    ZoneDefinition {
        id: SceneId::new(),
        name: name.to_string(),
        description: format!("test zone: {name}"),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.0,
            y_pct: 0.0,
            width_pct: 1.0,
            height_pct: 0.1,
        },
        accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::Stack { max_depth: 8 },
        max_publishers: 8,
        transport_constraint: None,
        auto_clear_ms: None,
        ephemeral: false,
        layer_attachment: LayerAttachment::Chrome,
    }
}

pub(super) fn scene_with_drag_handle_tile(
    initial_x: f32,
    initial_y: f32,
    tile_w: f32,
    tile_h: f32,
) -> (SceneGraph, SceneId, SceneId, String) {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Main", 0).expect("tab must be created");
    let lease_id = scene.grant_lease(
        "portal-agent",
        60_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );
    let tile_id = scene
        .create_tile(
            tab_id,
            "portal-agent",
            lease_id,
            Rect::new(initial_x, initial_y, tile_w, tile_h),
            1,
        )
        .expect("tile must be created");

    let element_id = tile_id;
    let interaction_id = format!(
        "drag-handle:{:032x}",
        element_id
            .to_bytes_le()
            .iter()
            .fold(0u128, |acc, &b| (acc << 8) | b as u128)
    );
    let handle_bounds = Rect::new(
        initial_x + tile_w / 2.0 - 20.0,
        initial_y - 10.0,
        40.0,
        20.0,
    );
    scene
        .overlay
        .drag_handle_hit_regions
        .push(DragHandleHitRegion {
            element_id,
            element_kind: DragHandleElementKind::Tile,
            bounds: handle_bounds,
            interaction_id: interaction_id.clone(),
            hit_region: HitRegionNode {
                bounds: handle_bounds,
                interaction_id: interaction_id.clone(),
                accepts_pointer: true,
                ..Default::default()
            },
            tab_order: 0,
        });

    (scene, tile_id, element_id, interaction_id)
}

pub(super) fn scene_with_composer_in_nonactive_tab()
-> (SceneGraph, SceneId, SceneId, SceneId, f32, f32) {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let other_tab = scene.create_tab("Other", 0).unwrap();
    let portal_tab = scene.create_tab("Portal", 1).unwrap();
    assert_eq!(scene.active_tab, Some(other_tab));

    let lease_id = scene.grant_lease(
        "portal-agent",
        60_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );
    let tile_id = scene
        .create_tile(
            portal_tab,
            "portal-agent",
            lease_id,
            Rect::new(100.0, 100.0, 400.0, 300.0),
            1,
        )
        .unwrap();

    let node_id = SceneId::new();
    scene
        .set_tile_root(
            tile_id,
            Node {
                id: node_id,
                children: vec![],
                data: NodeData::HitRegion(HitRegionNode {
                    bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
                    interaction_id: "portal-composer-focus".to_string(),
                    accepts_focus: true,
                    accepts_pointer: true,
                    accepts_composer_input: true,
                    ..Default::default()
                }),
            },
        )
        .unwrap();

    (scene, portal_tab, tile_id, node_id, 150.0, 150.0)
}

pub(super) fn portal_scene_with_focus() -> (SceneGraph, SceneId, SceneId, FocusManager) {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Main", 0).unwrap();
    let lease_id = scene.grant_lease(
        "portal-agent",
        60_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );
    let tile_id = scene
        .create_tile(
            tab_id,
            "portal-agent",
            lease_id,
            Rect::new(100.0, 100.0, 400.0, 300.0),
            1,
        )
        .unwrap();

    let _ = scene.register_tile_scroll_config(tile_id, TileScrollConfig::vertical());

    let mut fm = FocusManager::new();
    fm.request_focus(
        FocusRequest {
            tile_id,
            node_id: None,
            steal: true,
            requesting_namespace: "portal-agent".to_string(),
        },
        tab_id,
        &scene,
    );

    (scene, tab_id, tile_id, fm)
}
