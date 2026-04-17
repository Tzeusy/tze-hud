//! Text stream portal governance and shell-isolation validation (hud-t98e.4).
//!
//! Focus: lease lifecycle, privacy redaction, safe-mode/freeze behavior,
//! ambient attention defaults, and shell isolation constraints for the
//! phase-0 raw-tile portal pilot.

use std::sync::Arc;

use tze_hud_runtime::{
    AttentionBudgetOutcome, AttentionBudgetTracker, ChromeState, ContentClassification,
    EnqueueResult, FreezeQueue, MutationTrafficClass, QueuedMutation, RedactionFrame,
    RedactionStyle, TileRedactionState, ViewerClass, build_redaction_cmds, collect_diagnostic,
    hit_regions_enabled, is_tile_redacted,
};
use tze_hud_scene::{
    Capability, Clock, SceneGraph, SceneId, TestClock,
    events::InterruptionClass,
    lease::{LeaseState, ORPHAN_GRACE_PERIOD_MS},
    mutation::{MutationBatch, SceneMutation},
    types::{
        FontFamily, Node, NodeData, Rect, Rgba, SolidColorNode, TextAlign, TextMarkdownNode,
        TextOverflow,
    },
};

const DISPLAY_W: f32 = 1920.0;
const DISPLAY_H: f32 = 1080.0;
const PORTAL_BOUNDS: Rect = Rect {
    x: 64.0,
    y: 132.0,
    width: 720.0,
    height: 320.0,
};

fn make_batch(namespace: &str, lease_id: SceneId, mutations: Vec<SceneMutation>) -> MutationBatch {
    MutationBatch {
        batch_id: SceneId::new(),
        agent_namespace: namespace.to_string(),
        mutations,
        timing_hints: None,
        lease_id: Some(lease_id),
    }
}

fn portal_root(transcript: &str) -> (Node, Vec<Node>) {
    let root = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::SolidColor(SolidColorNode {
            color: Rgba::new(0.09, 0.11, 0.14, 0.9),
            bounds: Rect::new(0.0, 0.0, PORTAL_BOUNDS.width, PORTAL_BOUNDS.height),
        }),
    };
    let transcript_node = Node {
        id: SceneId::new(),
        children: Vec::new(),
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: transcript.to_string(),
            bounds: Rect::new(
                16.0,
                44.0,
                PORTAL_BOUNDS.width - 32.0,
                PORTAL_BOUNDS.height - 84.0,
            ),
            font_size_px: 13.0,
            font_family: FontFamily::SystemMonospace,
            color: Rgba::new(0.92, 0.95, 1.0, 0.98),
            background: None,
            alignment: TextAlign::Start,
            overflow: TextOverflow::Clip,
        }),
    };
    (root, vec![transcript_node])
}

fn root_batch_for_tile(tile_id: SceneId, root: Node, children: Vec<Node>) -> Vec<SceneMutation> {
    let root_id = root.id;
    let mut mutations = vec![SceneMutation::SetTileRoot {
        tile_id,
        node: root,
    }];
    mutations.extend(children.into_iter().map(|node| SceneMutation::AddNode {
        tile_id,
        parent_id: Some(root_id),
        node,
    }));
    mutations
}

fn create_portal_scene(ttl_ms: u64) -> (SceneGraph, TestClock, SceneId, SceneId, SceneId) {
    let clock = TestClock::new(1_000);
    let mut scene = SceneGraph::new_with_clock(DISPLAY_W, DISPLAY_H, Arc::new(clock.clone()));
    let namespace = "portal-gov";
    let tab_id = scene.create_tab("Main", 0).expect("tab");
    scene.active_tab = Some(tab_id);
    let lease_id = scene.grant_lease(
        namespace,
        ttl_ms,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );

    let create = scene.apply_batch(&make_batch(
        namespace,
        lease_id,
        vec![SceneMutation::CreateTile {
            tab_id,
            namespace: namespace.to_string(),
            lease_id,
            bounds: PORTAL_BOUNDS,
            z_order: 180,
        }],
    ));
    assert!(create.applied, "portal tile create must apply");
    let tile_id = create.created_ids[0];

    let (root, children) = portal_root("line 0");
    let set_root = scene.apply_batch(&make_batch(
        namespace,
        lease_id,
        root_batch_for_tile(tile_id, root, children),
    ));
    assert!(set_root.applied, "initial portal transcript must apply");

    (scene, clock, tab_id, lease_id, tile_id)
}

fn portal_text(scene: &SceneGraph, tile_id: SceneId) -> String {
    let tile = scene.tiles.get(&tile_id).expect("tile exists");
    let root_id = tile.root_node.expect("root exists");
    let root = scene.nodes.get(&root_id).expect("root node exists");
    for child in &root.children {
        let node = scene.nodes.get(child).expect("child exists");
        if let NodeData::TextMarkdown(t) = &node.data {
            return t.content.clone();
        }
    }
    panic!("portal root must contain transcript TextMarkdown node");
}

#[test]
fn lease_expiry_removes_portal_tile() {
    let (mut scene, clock, _tab_id, lease_id, tile_id) = create_portal_scene(20);
    clock.advance(25);

    let expiries = scene.expire_leases();
    assert!(
        expiries.iter().any(|e| e.lease_id == lease_id),
        "lease expiry list must include portal lease"
    );
    assert_eq!(
        scene.leases[&lease_id].state,
        LeaseState::Expired,
        "lease must be terminal after ttl expiry"
    );
    assert!(
        !scene.tiles.contains_key(&tile_id),
        "portal tile must be removed on lease expiry"
    );
}

#[test]
fn lease_revocation_removes_portal_tile() {
    let (mut scene, _clock, _tab_id, lease_id, tile_id) = create_portal_scene(120_000);
    scene.revoke_lease(lease_id).expect("revoke lease");

    assert_eq!(
        scene.leases[&lease_id].state,
        LeaseState::Revoked,
        "lease must transition to revoked"
    );
    assert!(
        !scene.tiles.contains_key(&tile_id),
        "revoked portal lease must remove tile"
    );
}

#[test]
fn orphaned_portal_freezes_and_grace_expiry_removes_tile() {
    let (mut scene, clock, _tab_id, lease_id, tile_id) = create_portal_scene(120_000);
    let before = portal_text(&scene, tile_id);

    scene
        .disconnect_lease(&lease_id, clock.now_millis())
        .expect("disconnect lease");
    assert_eq!(
        scene.leases[&lease_id].state,
        LeaseState::Orphaned,
        "disconnect must transition lease to orphaned"
    );
    assert!(
        scene.tiles.contains_key(&tile_id),
        "orphaned portal stays visible through grace period"
    );

    let (root, children) = portal_root("line after disconnect");
    let rejected = scene.apply_batch(&make_batch(
        "portal-gov",
        lease_id,
        root_batch_for_tile(tile_id, root, children),
    ));
    assert!(
        !rejected.applied,
        "orphaned lease must reject further portal updates"
    );
    assert_eq!(
        portal_text(&scene, tile_id),
        before,
        "portal surface must freeze at last coherent transcript state"
    );

    clock.advance(ORPHAN_GRACE_PERIOD_MS + 1);
    let expiries = scene.expire_leases();
    assert!(
        expiries.iter().any(|e| e.lease_id == lease_id),
        "orphaned lease must expire after grace timeout"
    );
    assert_eq!(scene.leases[&lease_id].state, LeaseState::Expired);
    assert!(
        !scene.tiles.contains_key(&tile_id),
        "portal tile must be removed when orphan grace expires"
    );
}

#[test]
fn redaction_preserves_geometry_and_hides_portal_content() {
    let (scene, _clock, _tab_id, _lease_id, tile_id) = create_portal_scene(120_000);
    let tile_bounds = scene.tiles.get(&tile_id).expect("tile exists").bounds;
    let raw_text = portal_text(&scene, tile_id);

    assert!(
        is_tile_redacted(ViewerClass::KnownGuest, ContentClassification::Private),
        "known guest must not see private portal transcript"
    );
    let redaction_state = TileRedactionState::Redacted {
        classification: ContentClassification::Private,
    };
    assert!(
        !hit_regions_enabled(&redaction_state),
        "redacted portal must disable interactive affordances"
    );

    let cmds = build_redaction_cmds(tile_bounds, RedactionStyle::Pattern);
    assert!(
        cmds.iter().any(|cmd| {
            cmd.x == tile_bounds.x
                && cmd.y == tile_bounds.y
                && cmd.width == tile_bounds.width
                && cmd.height == tile_bounds.height
        }),
        "redaction overlay must preserve tile geometry"
    );

    let frame = RedactionFrame::build(
        ViewerClass::KnownGuest,
        RedactionStyle::Pattern,
        1,
        &[(0, ContentClassification::Private)],
    );
    assert!(
        frame.is_redacted(0),
        "redaction frame must mark private tile as redacted for known guest viewer"
    );
    assert_eq!(
        portal_text(&scene, tile_id),
        raw_text,
        "redaction must not mutate the underlying scene transcript data"
    );
    let overlay_debug = format!("{cmds:?}");
    assert!(
        !overlay_debug.contains(&raw_text),
        "redaction overlay commands must not carry transcript content"
    );
}

#[test]
fn safe_mode_suspend_blocks_portal_updates_until_resume() {
    let (mut scene, clock, _tab_id, lease_id, tile_id) = create_portal_scene(120_000);
    let baseline = portal_text(&scene, tile_id);

    scene
        .suspend_lease(&lease_id, clock.now_millis())
        .expect("suspend lease");
    assert_eq!(scene.leases[&lease_id].state, LeaseState::Suspended);

    let (root, children) = portal_root("line while safe mode active");
    let rejected = scene.apply_batch(&make_batch(
        "portal-gov",
        lease_id,
        root_batch_for_tile(tile_id, root, children),
    ));
    assert!(
        !rejected.applied,
        "suspended lease must reject updates while safe mode is active"
    );
    assert_eq!(portal_text(&scene, tile_id), baseline);

    scene
        .resume_lease(&lease_id, clock.now_millis() + 1)
        .expect("resume lease");
    let (root, children) = portal_root("line after safe mode exit");
    let accepted = scene.apply_batch(&make_batch(
        "portal-gov",
        lease_id,
        root_batch_for_tile(tile_id, root, children),
    ));
    assert!(
        accepted.applied,
        "portal updates should resume after safe mode exit"
    );
    assert_eq!(portal_text(&scene, tile_id), "line after safe mode exit");
}

#[test]
fn freeze_path_uses_generic_backpressure_signal_not_portal_specific_signal() {
    let mut queue = FreezeQueue::new(1);
    let first = queue.enqueue(QueuedMutation {
        batch_id: b"tx-1".to_vec(),
        original_batch_id: b"tx-1".to_vec(),
        traffic_class: MutationTrafficClass::Transactional,
        coalesce_key: None,
        submitted_at_wall_us: 1,
        payload: b"tx-1".to_vec(),
    });
    assert!(matches!(
        first,
        EnqueueResult::Queued | EnqueueResult::QueuedWithPressure
    ));

    let second = queue.enqueue(QueuedMutation {
        batch_id: b"tx-2".to_vec(),
        original_batch_id: b"tx-2".to_vec(),
        traffic_class: MutationTrafficClass::Transactional,
        coalesce_key: None,
        submitted_at_wall_us: 2,
        payload: b"tx-2".to_vec(),
    });
    assert!(
        matches!(second, EnqueueResult::BackpressureRequired { .. }),
        "freeze overflow should produce generic backpressure for transactional batches"
    );

    let proto = include_str!("../../crates/tze_hud_protocol/proto/session.proto");
    assert!(
        proto.contains("message BackpressureSignal"),
        "protocol must expose generic queue pressure signal shape"
    );
    assert!(
        !proto.contains("PORTAL_FREEZE"),
        "protocol must not expose a portal-specific freeze signal"
    );
}

#[test]
fn unread_backlog_defaults_to_ambient_attention_class() {
    let mut tracker = AttentionBudgetTracker::new();
    let mut saw_warning = false;
    let mut saw_coalesce = false;

    for i in 0..50_u64 {
        let outcome = tracker.record(
            "portal-gov",
            "portal-zone",
            InterruptionClass::Low,
            i * 1_000_000,
        );
        match outcome {
            AttentionBudgetOutcome::Ok => {}
            AttentionBudgetOutcome::Warning => saw_warning = true,
            AttentionBudgetOutcome::Coalesce => saw_coalesce = true,
            AttentionBudgetOutcome::CriticalExempt | AttentionBudgetOutcome::SilentPassthrough => {
                panic!("ambient portal backlog must not auto-upgrade into stronger interruption");
            }
        }
    }

    assert!(
        saw_warning,
        "ambient traffic should still respect budget warning"
    );
    assert!(
        saw_coalesce,
        "heavy ambient backlog should coalesce, not escalate urgency"
    );
}

#[test]
fn shell_status_snapshot_exposes_no_portal_identity_or_transcript() {
    let (scene, _clock, _tab_id, _lease_id, tile_id) = create_portal_scene(120_000);
    let transcript = portal_text(&scene, tile_id);
    let mut chrome = ChromeState::new();
    chrome.connected_agent_count = 1;
    chrome.add_tab(1, "portal://agent/session".to_string());
    chrome.add_tab(2, format!("preview:{transcript}"));
    assert!(
        chrome.tabs.iter().any(|tab| tab.name.contains("portal://")),
        "test setup must include portal identity in chrome input state"
    );
    assert!(
        chrome.tabs.iter().any(|tab| tab.name.contains(&transcript)),
        "test setup must include transcript-like content in chrome input state"
    );

    let snapshot = collect_diagnostic(&chrome, 123_456, scene.leases.len());
    assert_eq!(
        snapshot.tab_count, 2,
        "snapshot should still report tab count"
    );
    let text = snapshot.to_string();

    assert!(
        !text.contains("portal://"),
        "shell diagnostics must not expose portal identity"
    );
    assert!(
        !text.contains(&transcript),
        "shell diagnostics must not expose transcript-derived content"
    );
}

#[test]
fn shell_dismiss_override_removes_portal_tile() {
    let (mut scene, _clock, _tab_id, lease_id, tile_id) = create_portal_scene(120_000);
    // Shell dismiss maps to lease revocation for content-layer tiles.
    scene.revoke_lease(lease_id).expect("shell dismiss revoke");
    assert!(
        !scene.tiles.contains_key(&tile_id),
        "shell override dismiss must remove portal tile"
    );
}
