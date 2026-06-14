//! Scene graph: the core data structure holding all tabs, tiles, nodes, leases.
//! Pure data — no GPU, no async, no I/O.

use crate::clock::{Clock, SystemClock};
use crate::types::*;
use crate::validation::ValidationError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Returns a `SystemClock` wrapped in `Arc<dyn Clock>`.
/// Used as the serde default for the `clock` field so that deserialized
/// graphs behave like freshly constructed ones.
fn default_clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock::new())
}

/// The root scene graph.
///
/// Time-dependent operations (lease grant, tab creation timestamps, expiry
/// checks) are routed through the injected [`Clock`].  Use
/// [`SceneGraph::new`] for production code — it installs a [`SystemClock`].
/// Use [`SceneGraph::new_with_clock`] in tests to inject a [`TestClock`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneGraph {
    /// Clock used for all `now_millis()` calls inside the graph.
    /// Skipped during serialization; restored to `SystemClock` on
    /// deserialization.
    #[serde(skip, default = "default_clock")]
    clock: Arc<dyn Clock>,
    /// All tabs, keyed by ID.
    pub tabs: HashMap<SceneId, Tab>,
    /// The currently active tab.
    pub active_tab: Option<SceneId>,
    /// All tiles, keyed by ID.
    pub tiles: HashMap<SceneId, Tile>,
    /// All nodes, keyed by ID.
    pub nodes: HashMap<SceneId, Node>,
    /// Active leases, keyed by ID.
    pub leases: HashMap<SceneId, Lease>,
    /// Hit region local state, keyed by node ID.
    pub hit_region_states: HashMap<SceneId, HitRegionLocalState>,
    /// Zone registry.
    pub zone_registry: ZoneRegistry,
    /// Widget registry.
    pub widget_registry: WidgetRegistry,
    /// Sync groups, keyed by ID.
    pub sync_groups: HashMap<SceneId, SyncGroup>,
    /// Display area (the viewport dimensions).
    pub display_area: Rect,
    /// Monotonic version counter, incremented on every mutation.
    pub version: u64,
    /// Monotonically increasing sequence number assigned to each committed batch.
    ///
    /// Incremented by [`SceneGraph::next_sequence_number`] on every successful
    /// [`crate::mutation::MutationBatch`] commit. Per RFC 0001 §3.5.
    pub sequence_number: u64,
    /// Map of ResourceIds to their scene-node reference counts.
    ///
    /// A resource is available for use in [`NodeData::StaticImage`] nodes when it
    /// has an entry in this map (regardless of the count value).  The count tracks
    /// how many live scene nodes currently reference the resource:
    ///
    /// - `register_resource` inserts the entry (count = 0) if not already present.
    /// - Inserting a `StaticImageNode` into the scene increments the count.
    /// - Removing a `StaticImageNode` from the scene decrements the count; when the
    ///   count reaches zero the entry is removed, freeing the resource from the
    ///   registry.
    ///
    /// An agent-submitted AddNode or SetTileRoot with a StaticImageNode is rejected
    /// if the ResourceId is not present in this map.
    ///
    /// Ephemeral: skipped during serialization (resources are in-memory only,
    /// per RFC 0001 §2.4 and resource-store/spec.md §Requirement: V1 ephemerality).
    #[serde(skip, default)]
    pub registered_resources: HashMap<ResourceId, u32>,

    /// Transient per-frame state owned by the runtime and compositor layers.
    ///
    /// Groups all render-scratch and input-feedback fields that must not pollute
    /// the pure scene-model public surface.  See [`RuntimeOverlayState`] for the
    /// full field inventory and lifetime semantics.
    #[serde(skip, default)]
    pub overlay: RuntimeOverlayState,
}

/// Maximum number of tabs in a scene. RFC 0001 §2.1.
pub const MAX_TABS: usize = 256;

/// Maximum number of tiles per tab. RFC 0001 §2.1.
pub const MAX_TILES_PER_TAB: usize = 1024;

/// Maximum number of nodes per tile. RFC 0001 §2.1.
pub const MAX_NODES_PER_TILE: usize = 64;

/// Maximum name length for tabs, in UTF-8 bytes. RFC 0001 §2.2.
pub const MAX_TAB_NAME_BYTES: usize = 128;

/// Maximum content size for TextMarkdownNode, in UTF-8 bytes. RFC 0001 §2.4.
pub const MAX_MARKDOWN_BYTES: usize = 65_535;

/// The z-order threshold below which agent-owned tiles must fall.
/// Tiles with z_order >= ZONE_TILE_Z_MIN are reserved for runtime-managed
/// zone tiles. RFC 0001 §2.3.
pub const ZONE_TILE_Z_MIN: u32 = 0x8000_0000;

// ─── Contention policy helper ────────────────────────────────────────────────

pub mod contention;
pub(crate) use contention::apply_contention;

pub mod overlay;
pub use overlay::RuntimeOverlayState;

pub mod budget;
pub mod leases;
pub mod node_tree;
pub mod queries;
pub mod resources;
pub mod snapshot;
pub mod sync_groups;
pub mod tabs;
pub mod tiles;
pub use tiles::validate_text_markdown_node_data;
pub mod zone_ops;

impl SceneGraph {
    // ─── Notification auto-dismiss TTL constants ─────────────────────────
    /// Default auto-dismiss TTL (µs) for low/normal notifications (urgency 0, 1).
    pub const NOTIFICATION_TTL_INFO_US: u64 = 8_000_000; // 8 seconds
    /// Default auto-dismiss TTL (µs) for urgent notifications (urgency 2).
    pub const NOTIFICATION_TTL_WARNING_US: u64 = 15_000_000; // 15 seconds
    /// Default auto-dismiss TTL (µs) for critical notifications (urgency 3+).
    pub const NOTIFICATION_TTL_CRITICAL_US: u64 = 30_000_000; // 30 seconds

    /// Create a new empty scene graph using the real system clock.
    pub fn new(width: f32, height: f32) -> Self {
        Self::new_with_clock(width, height, Arc::new(SystemClock::new()))
    }

    /// Create a new empty scene graph with an injected clock.
    ///
    /// Prefer this constructor in tests so that time-dependent behaviour
    /// (lease expiry, timestamps) is fully deterministic.
    pub fn new_with_clock(width: f32, height: f32, clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            tabs: HashMap::new(),
            active_tab: None,
            tiles: HashMap::new(),
            nodes: HashMap::new(),
            leases: HashMap::new(),
            hit_region_states: HashMap::new(),
            zone_registry: ZoneRegistry::new(),
            widget_registry: WidgetRegistry::new(),
            sync_groups: HashMap::new(),
            display_area: Rect::new(0.0, 0.0, width, height),
            version: 0,
            sequence_number: 0,
            registered_resources: HashMap::new(),
            overlay: RuntimeOverlayState::default(),
        }
    }

    // ─── Sequence number (RFC 0001 §3.5) ────────────────────────────────

    /// Advance the sequence counter and return the new value.
    ///
    /// Called by the mutation pipeline on every successful batch commit.
    /// Sequence numbers are strictly monotonically increasing u64 values.
    pub(crate) fn next_sequence_number(&mut self) -> u64 {
        self.sequence_number += 1;
        self.sequence_number
    }

    // ─── Clock accessor ──────────────────────────────────────────────────

    /// Return the current time in milliseconds from the injected clock.
    pub(crate) fn now_millis(&self) -> u64 {
        self.clock.now_millis()
    }
}

fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Decision returned by `SceneGraph::evaluate_sync_group_commit`.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncGroupCommitDecision {
    /// Commit the listed tiles' pending mutations this frame.
    Commit { tiles: Vec<SceneId> },
    /// Defer the entire group to the next frame (AllOrDefer policy).
    Defer,
    /// Force-commit with the listed tiles after exhausting max_deferrals.
    /// The compositor should emit a `sync_group_force_commit` telemetry event.
    ForceCommit { tiles: Vec<SceneId> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    /// Convenience: build a SceneGraph backed by a TestClock starting at t=1000ms.
    fn scene_with_test_clock() -> (SceneGraph, TestClock) {
        let clock = TestClock::new(1_000);
        let scene = SceneGraph::new_with_clock(1920.0, 1080.0, Arc::new(clock.clone()));
        (scene, clock)
    }

    #[test]
    fn test_create_scene_with_tab_and_tiles() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        // Create a tab
        let tab_id = scene.create_tab("Main", 0).unwrap();
        assert_eq!(scene.active_tab, Some(tab_id));

        // Grant a lease
        let lease_id = scene.grant_lease(
            "test-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        // Create two tiles
        let tile1_id = scene
            .create_tile(
                tab_id,
                "test-agent",
                lease_id,
                Rect::new(10.0, 10.0, 400.0, 300.0),
                1,
            )
            .unwrap();

        let tile2_id = scene
            .create_tile(
                tab_id,
                "test-agent",
                lease_id,
                Rect::new(420.0, 10.0, 400.0, 300.0),
                2,
            )
            .unwrap();

        assert_eq!(scene.tile_count(), 2);

        // Add nodes
        let text_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "Hello, tze_hud!".to_string(),
                bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
                font_size_px: 24.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: Some(Rgba::new(0.1, 0.1, 0.2, 1.0)),
                alignment: TextAlign::Center,
                overflow: TextOverflow::Clip,
                color_runs: Box::default(),
            }),
        };
        scene.set_tile_root(tile1_id, text_node).unwrap();

        let hit_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(50.0, 50.0, 200.0, 100.0),
                interaction_id: "btn-click".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };
        scene.set_tile_root(tile2_id, hit_node.clone()).unwrap();

        assert_eq!(scene.node_count(), 2);
        assert!(scene.hit_region_states.contains_key(&hit_node.id));
    }

    #[test]
    fn test_hit_test() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        let tile_id = scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();

        let hr_node_id = SceneId::new();
        let hit_node = Node {
            id: hr_node_id,
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(50.0, 50.0, 200.0, 100.0),
                interaction_id: "btn".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };
        scene.set_tile_root(tile_id, hit_node).unwrap();

        // Hit the hit region (tile at 100,100; region at 50,50 within tile = 150,150 global)
        let result = scene.hit_test(200.0, 180.0);
        assert_eq!(
            result,
            HitResult::NodeHit {
                tile_id,
                node_id: hr_node_id,
                interaction_id: "btn".to_string(),
            }
        );

        // Miss the hit region but hit the tile
        let result = scene.hit_test(110.0, 110.0);
        assert_eq!(result, HitResult::TileHit { tile_id });

        // Miss everything
        let result = scene.hit_test(10.0, 10.0);
        assert_eq!(result, HitResult::Passthrough);
    }

    #[test]
    fn test_hit_test_applies_tile_scroll_offset() {
        let mut scene = SceneGraph::new(800.0, 600.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "scroll-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "scroll-agent",
                lease_id,
                Rect::new(100.0, 100.0, 300.0, 200.0),
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
                        bounds: Rect::new(10.0, 60.0, 120.0, 40.0),
                        interaction_id: "scroll-hit".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        scene
            .set_tile_scroll_offset_local(tile_id, 0.0, 50.0)
            .unwrap();

        assert_eq!(
            scene.hit_test(120.0, 115.0),
            HitResult::NodeHit {
                tile_id,
                node_id,
                interaction_id: "scroll-hit".to_string(),
            }
        );
    }

    #[test]
    fn test_hit_test_zone_regions_without_active_tab() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        // Intentionally do not create/activate a tab

        // Add a zone hit region (as the compositor would do each frame)
        scene.overlay.zone_hit_regions.push(ZoneHitRegion {
            zone_name: "notifications".to_string(),
            published_at_wall_us: 123456,
            publisher_namespace: "test".to_string(),
            bounds: Rect::new(100.0, 100.0, 200.0, 150.0),
            kind: ZoneInteractionKind::Dismiss,
            interaction_id: "zone:notifications:dismiss:123456:test".to_string(),
            tab_order: 0,
        });

        // Hit the zone region even though active_tab is None
        let result = scene.hit_test(150.0, 125.0);
        match result {
            HitResult::ZoneInteraction {
                zone_name,
                published_at_wall_us,
                publisher_namespace,
                interaction_id,
                kind: ZoneInteractionKind::Dismiss,
            } => {
                assert_eq!(zone_name, "notifications");
                assert_eq!(published_at_wall_us, 123456);
                assert_eq!(publisher_namespace, "test");
                assert_eq!(interaction_id, "zone:notifications:dismiss:123456:test");
            }
            _ => panic!("Expected ZoneInteraction, got {result:?}"),
        }

        // Miss the zone region
        let result = scene.hit_test(50.0, 50.0);
        assert_eq!(result, HitResult::Passthrough);
    }

    #[test]
    fn test_snapshot_roundtrip() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();

        let json = scene.snapshot_json().unwrap();
        let restored = SceneGraph::from_json(&json).unwrap();

        assert_eq!(scene.tile_count(), restored.tile_count());
        assert_eq!(scene.active_tab, restored.active_tab);
        assert_eq!(scene.version, restored.version);
    }

    #[test]
    fn take_snapshot_includes_display_area() {
        let scene = SceneGraph::new(2560.0, 1440.0);

        let snapshot = scene.take_snapshot(1_000, 2_000);

        assert_eq!(snapshot.display_area, Rect::new(0.0, 0.0, 2560.0, 1440.0));
        assert!(snapshot.verify_checksum());
    }

    #[test]
    fn test_lease_expiry() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // Grant a lease with a 500 ms TTL.
        // Clock is at t=1000; lease expires at t=1500.
        let lease_id = scene.grant_lease(
            "test",
            500,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();

        assert_eq!(scene.tile_count(), 1);

        // Before expiry: clock still at t=1000, lease lives.
        let expired = scene.expire_leases();
        assert_eq!(expired.len(), 0);
        assert_eq!(scene.tile_count(), 1);

        // Advance past the TTL.
        clock.advance(501);
        let expired = scene.expire_leases();
        assert_eq!(expired.len(), 1);
        assert_eq!(scene.tile_count(), 0);
    }

    #[test]
    fn test_tab_created_at_uses_clock() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        assert_eq!(scene.tabs[&tab_id].created_at_ms, 1_000);

        // Advancing the clock does NOT retroactively change existing timestamps.
        clock.advance(100);
        assert_eq!(scene.tabs[&tab_id].created_at_ms, 1_000);
    }

    #[test]
    fn test_renew_lease_uses_clock() {
        let (mut scene, clock) = scene_with_test_clock();
        // Clock at t=1000.
        let lease_id = scene.grant_lease("test", 5_000, vec![]);
        assert_eq!(scene.leases[&lease_id].granted_at_ms, 1_000);

        // Advance clock then renew.
        clock.advance(2_000);
        scene.renew_lease(lease_id, 10_000).unwrap();
        assert_eq!(scene.leases[&lease_id].granted_at_ms, 3_000);
        assert_eq!(scene.leases[&lease_id].ttl_ms, 10_000);
    }

    #[test]
    fn test_lease_revocation_cleans_tiles() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(200.0, 0.0, 100.0, 100.0),
                2,
            )
            .unwrap();

        assert_eq!(scene.tile_count(), 2);
        scene.revoke_lease(lease_id).unwrap();
        assert_eq!(scene.tile_count(), 0);
        // Revoked leases remain in the map with terminal state
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Revoked);
    }

    #[test]
    fn test_visible_tiles_sorted_by_z_order() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                5,
            )
            .unwrap();
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                3,
            )
            .unwrap();

        let visible = scene.visible_tiles();
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].z_order, 1);
        assert_eq!(visible[1].z_order, 3);
        assert_eq!(visible[2].z_order, 5);
    }

    // ─── Zone tests ───────────────────────────────────────────────────────

    fn make_subtitle_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_string(),
            description: "Subtitle overlay".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 48.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 2,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        }
    }

    fn make_notification_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "notifications".to_string(),
            description: "Notification stack".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 3 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: Some(5_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        }
    }

    fn make_status_bar_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_string(),
            description: "Status bar".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 8 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        }
    }

    fn dummy_token() -> ZonePublishToken {
        ZonePublishToken {
            token: vec![0xDE, 0xAD, 0xBE, 0xEF],
        }
    }

    #[test]
    fn test_zone_register_unregister() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone = make_subtitle_zone();

        scene.register_zone(zone.clone());
        assert!(scene.zone_registry.get_by_name("subtitle").is_some());

        let removed = scene.unregister_zone("subtitle");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "subtitle");
        assert!(scene.zone_registry.get_by_name("subtitle").is_none());
    }

    #[test]
    fn test_zone_query_by_name() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.register_zone(make_notification_zone());

        let zone = scene.zone_registry.get_by_name("subtitle").unwrap();
        assert_eq!(zone.name, "subtitle");
        assert!(scene.zone_registry.get_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_zone_query_by_media_type() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.register_zone(make_notification_zone());

        let stream_text_zones = scene
            .zone_registry
            .zones_accepting(ZoneMediaType::StreamText);
        assert_eq!(stream_text_zones.len(), 1);
        assert_eq!(stream_text_zones[0].name, "subtitle");

        let notif_zones = scene
            .zone_registry
            .zones_accepting(ZoneMediaType::ShortTextWithIcon);
        assert_eq!(notif_zones.len(), 1);
        assert_eq!(notif_zones[0].name, "notifications");
    }

    #[test]
    fn test_default_zones_populated() {
        let registry = ZoneRegistry::with_defaults();
        assert!(registry.get_by_name("status-bar").is_some());
        assert!(registry.get_by_name("notification-area").is_some());
        assert!(registry.get_by_name("subtitle").is_some());
    }

    #[test]
    fn test_zone_publish_not_found() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result = scene.publish_to_zone(
            "nonexistent",
            ZoneContent::StreamText("hello".to_string()),
            "agent",
            None,
            None,
            None,
        );
        assert!(matches!(result, Err(ValidationError::ZoneNotFound { .. })));
    }

    #[test]
    fn test_zone_publish_media_type_mismatch() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone()); // accepts StreamText only

        let result = scene.publish_to_zone(
            "subtitle",
            ZoneContent::Notification(NotificationPayload {
                text: "Hello".to_string(),
                icon: "".to_string(),
                urgency: 1,
                ttl_ms: None,
                title: String::new(),
                actions: Vec::new(),
            }),
            "agent",
            None,
            None,
            None,
        );
        assert!(matches!(
            result,
            Err(ValidationError::ZoneMediaTypeMismatch { .. })
        ));
    }

    #[test]
    fn test_contention_latest_wins() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("first".to_string()),
                "a1",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("second".to_string()),
                "a2",
                None,
                None,
                None,
            )
            .unwrap();

        let publishes = scene.zone_registry.active_for_zone("subtitle");
        assert_eq!(publishes.len(), 1);
        assert_eq!(
            publishes[0].content,
            ZoneContent::StreamText("second".to_string())
        );
        assert_eq!(publishes[0].publisher_namespace, "a2");
    }

    #[test]
    fn test_contention_stack() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_notification_zone()); // Stack { max_depth: 3 }

        let notification = |text: &str| {
            ZoneContent::Notification(NotificationPayload {
                text: text.to_string(),
                icon: "".to_string(),
                urgency: 1,
                ttl_ms: None,
                title: String::new(),
                actions: Vec::new(),
            })
        };

        scene
            .publish_to_zone(
                "notifications",
                notification("msg1"),
                "a1",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notifications",
                notification("msg2"),
                "a2",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notifications",
                notification("msg3"),
                "a3",
                None,
                None,
                None,
            )
            .unwrap();

        let publishes = scene.zone_registry.active_for_zone("notifications");
        assert_eq!(publishes.len(), 3);

        // 4th publish should trim the oldest
        scene
            .publish_to_zone(
                "notifications",
                notification("msg4"),
                "a4",
                None,
                None,
                None,
            )
            .unwrap();
        let publishes = scene.zone_registry.active_for_zone("notifications");
        assert_eq!(publishes.len(), 3);
        // Oldest (msg1) should be gone, newest (msg4) at end
        if let ZoneContent::Notification(n) = &publishes[0].content {
            assert_eq!(n.text, "msg2");
        } else {
            panic!("expected Notification");
        }
        if let ZoneContent::Notification(n) = &publishes[2].content {
            assert_eq!(n.text, "msg4");
        } else {
            panic!("expected Notification");
        }
    }

    // ─── Alert-Banner Auto-Dismiss Tests ────────────────────────────────

    /// Helper: build a zone definition that accepts ShortTextWithIcon
    /// (Notification content) with Stack contention policy.
    fn make_alert_banner_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_string(),
            description: "Alert banner zone".to_string(),
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

    fn publish_notification(scene: &mut SceneGraph, urgency: u32, expires_at: Option<u64>) {
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: format!("urgency-{urgency}"),
                    icon: "".to_string(),
                    urgency,
                    ttl_ms: None,
                    title: String::new(),
                    actions: Vec::new(),
                }),
                "test-agent",
                None,
                expires_at,
                None,
            )
            .unwrap();
    }

    /// urgency 0 (low) → expires_at = now + 8 s
    #[test]
    fn test_notification_auto_dismiss_urgency_info_low() {
        let (mut scene, clock) = scene_with_test_clock();
        scene.register_zone(make_alert_banner_zone());

        publish_notification(&mut scene, 0, None);

        let record = &scene.zone_registry.active_for_zone("alert-banner")[0];
        let expected = clock.now_us() + SceneGraph::NOTIFICATION_TTL_INFO_US;
        assert_eq!(
            record.expires_at_wall_us,
            Some(expected),
            "urgency 0 (low) should auto-dismiss after 8 s"
        );
    }

    /// urgency 1 (normal) → expires_at = now + 8 s
    #[test]
    fn test_notification_auto_dismiss_urgency_info_normal() {
        let (mut scene, clock) = scene_with_test_clock();
        scene.register_zone(make_alert_banner_zone());

        publish_notification(&mut scene, 1, None);

        let record = &scene.zone_registry.active_for_zone("alert-banner")[0];
        let expected = clock.now_us() + SceneGraph::NOTIFICATION_TTL_INFO_US;
        assert_eq!(
            record.expires_at_wall_us,
            Some(expected),
            "urgency 1 (normal) should auto-dismiss after 8 s"
        );
    }

    /// urgency 2 (urgent) → expires_at = now + 15 s
    #[test]
    fn test_notification_auto_dismiss_urgency_warning() {
        let (mut scene, clock) = scene_with_test_clock();
        scene.register_zone(make_alert_banner_zone());

        publish_notification(&mut scene, 2, None);

        let record = &scene.zone_registry.active_for_zone("alert-banner")[0];
        let expected = clock.now_us() + SceneGraph::NOTIFICATION_TTL_WARNING_US;
        assert_eq!(
            record.expires_at_wall_us,
            Some(expected),
            "urgency 2 (urgent) should auto-dismiss after 15 s"
        );
    }

    /// urgency 3 (critical) → expires_at = now + 30 s
    #[test]
    fn test_notification_auto_dismiss_urgency_critical() {
        let (mut scene, clock) = scene_with_test_clock();
        scene.register_zone(make_alert_banner_zone());

        publish_notification(&mut scene, 3, None);

        let record = &scene.zone_registry.active_for_zone("alert-banner")[0];
        let expected = clock.now_us() + SceneGraph::NOTIFICATION_TTL_CRITICAL_US;
        assert_eq!(
            record.expires_at_wall_us,
            Some(expected),
            "urgency 3 (critical) should auto-dismiss after 30 s"
        );
    }

    /// Publisher-supplied expires_at takes precedence over the urgency default.
    #[test]
    fn test_notification_auto_dismiss_publisher_override() {
        let (mut scene, clock) = scene_with_test_clock();
        scene.register_zone(make_alert_banner_zone());

        // Use a custom expiry that differs from both the default and the clock.
        let publisher_expires_at = clock.now_us() + 60_000_000u64; // 60 s
        publish_notification(&mut scene, 1, Some(publisher_expires_at));

        let record = &scene.zone_registry.active_for_zone("alert-banner")[0];
        assert_eq!(
            record.expires_at_wall_us,
            Some(publisher_expires_at),
            "publisher-supplied expires_at must take precedence over urgency default"
        );
    }

    /// Non-Notification content (StreamText) must NOT have expires_at auto-set.
    #[test]
    fn test_non_notification_content_no_auto_dismiss() {
        let (mut scene, _clock) = scene_with_test_clock();
        scene.register_zone(make_subtitle_zone()); // subtitle zone accepts StreamText

        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("hello".to_string()),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        let record = &scene.zone_registry.active_for_zone("subtitle")[0];
        assert_eq!(
            record.expires_at_wall_us, None,
            "non-Notification content must not have auto-dismiss expires_at"
        );
    }

    /// End-to-end: advance clock past expiry and verify drain removes the publication.
    #[test]
    fn test_notification_auto_dismiss_drain_removes_after_expiry() {
        let (mut scene, clock) = scene_with_test_clock();
        scene.register_zone(make_alert_banner_zone());

        // Publish a low-urgency notification (auto-dismiss after 8 s).
        publish_notification(&mut scene, 0, None);
        assert_eq!(
            scene.zone_registry.active_for_zone("alert-banner").len(),
            1,
            "notification must be present before expiry"
        );

        // Advance clock to just before the TTL boundary — must still be visible.
        clock.advance(SceneGraph::NOTIFICATION_TTL_INFO_US / 1_000 - 1); // advance in ms
        let drained = scene.drain_expired_zone_publications();
        assert_eq!(drained, 0, "must not expire before TTL elapses");
        assert_eq!(scene.zone_registry.active_for_zone("alert-banner").len(), 1,);

        // Advance past the TTL boundary — must be removed.
        clock.advance(2); // total elapsed > 8 s
        let drained = scene.drain_expired_zone_publications();
        assert_eq!(drained, 1, "expired notification must be drained");
        assert_eq!(
            scene.zone_registry.active_for_zone("alert-banner").len(),
            0,
            "zone must be empty after auto-dismiss drain"
        );
    }

    // ─── Sync Group Tests ────────────────────────────────────────────────

    fn make_scene_with_tiles(count: usize) -> (SceneGraph, SceneId, Vec<SceneId>) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let mut tile_ids = Vec::new();
        for i in 0..count {
            let tile_id = scene
                .create_tile(
                    tab_id,
                    "agent",
                    lease_id,
                    Rect::new(i as f32 * 110.0, 0.0, 100.0, 100.0),
                    i as u32,
                )
                .unwrap();
            tile_ids.push(tile_id);
        }
        (scene, tab_id, tile_ids)
    }

    #[test]
    fn test_create_sync_group() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);

        let group_id = scene
            .create_sync_group(
                Some("test-group".to_string()),
                "agent",
                SyncCommitPolicy::AllOrDefer,
                3,
            )
            .unwrap();

        assert_eq!(scene.sync_group_count(), 1);
        let group = scene.sync_groups.get(&group_id).unwrap();
        assert_eq!(group.name, Some("test-group".to_string()));
        assert_eq!(group.owner_namespace, "agent");
        assert_eq!(group.commit_policy, SyncCommitPolicy::AllOrDefer);
        assert_eq!(group.max_deferrals, 3);
        assert!(group.members.is_empty());
    }

    #[test]
    fn test_delete_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);

        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();

        // Join both tiles
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Deleting the group should release tiles
        scene.delete_sync_group(group_id).unwrap();
        assert_eq!(scene.sync_group_count(), 0);

        // Tiles should have no sync_group reference
        assert_eq!(scene.tiles[&tiles[0]].sync_group, None);
        assert_eq!(scene.tiles[&tiles[1]].sync_group, None);
    }

    #[test]
    fn test_delete_nonexistent_sync_group_errors() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);
        let fake_id = SceneId::new();
        let result = scene.delete_sync_group(fake_id);
        assert!(matches!(
            result,
            Err(ValidationError::SyncGroupNotFound { .. })
        ));
    }

    #[test]
    fn test_join_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();

        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        assert_eq!(scene.sync_groups[&group_id].members.len(), 2);
        assert_eq!(scene.tiles[&tiles[0]].sync_group, Some(group_id));
        assert_eq!(scene.tiles[&tiles[1]].sync_group, Some(group_id));
    }

    #[test]
    fn test_join_replaces_old_group_membership() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        let group_a = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();
        let group_b = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();

        scene.join_sync_group(tiles[0], group_a).unwrap();
        // Now join a different group — should leave group_a automatically
        scene.join_sync_group(tiles[0], group_b).unwrap();

        assert!(!scene.sync_groups[&group_a].members.contains(&tiles[0]));
        assert!(scene.sync_groups[&group_b].members.contains(&tiles[0]));
        assert_eq!(scene.tiles[&tiles[0]].sync_group, Some(group_b));
    }

    #[test]
    fn test_leave_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();

        scene.join_sync_group(tiles[0], group_id).unwrap();
        assert!(scene.sync_groups[&group_id].members.contains(&tiles[0]));

        scene.leave_sync_group(tiles[0]).unwrap();
        assert!(!scene.sync_groups[&group_id].members.contains(&tiles[0]));
        assert_eq!(scene.tiles[&tiles[0]].sync_group, None);
        // Group still exists after tile leaves
        assert_eq!(scene.sync_group_count(), 1);
    }

    #[test]
    fn test_leave_when_not_in_group_is_noop() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        // No group created — tile has no sync_group; leave should succeed silently
        let result = scene.leave_sync_group(tiles[0]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_available_members_commit_policy() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has a pending mutation
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();

        // AvailableMembers: commit whatever is ready, no deferral
        match decision {
            SyncGroupCommitDecision::Commit { tiles: committed } => {
                assert_eq!(committed, vec![tiles[0]]);
            }
            other => panic!("Expected Commit, got {other:?}"),
        }
    }

    #[test]
    fn test_contention_merge_by_key() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_status_bar_zone()); // MergeByKey { max_keys: 8 }

        let kv = |k: &str, v: &str| {
            let mut entries = std::collections::HashMap::new();
            entries.insert(k.to_string(), v.to_string());
            ZoneContent::StatusBar(StatusBarPayload { entries })
        };

        // Publish with different keys
        scene
            .publish_to_zone(
                "status-bar",
                kv("clock", "12:00"),
                "a1",
                Some("clock".to_string()),
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "status-bar",
                kv("battery", "80%"),
                "a2",
                Some("battery".to_string()),
                None,
                None,
            )
            .unwrap();

        let publishes = scene.zone_registry.active_for_zone("status-bar");
        assert_eq!(publishes.len(), 2);

        // Update existing key "clock"
        scene
            .publish_to_zone(
                "status-bar",
                kv("clock", "12:01"),
                "a1",
                Some("clock".to_string()),
                None,
                None,
            )
            .unwrap();
        let publishes = scene.zone_registry.active_for_zone("status-bar");
        assert_eq!(publishes.len(), 2); // Still 2 (clock replaced, battery retained)
        let clock = publishes
            .iter()
            .find(|r| r.merge_key.as_deref() == Some("clock"))
            .unwrap();
        if let ZoneContent::StatusBar(sb) = &clock.content {
            assert_eq!(sb.entries["clock"], "12:01");
        } else {
            panic!("expected StatusBar");
        }
    }

    #[test]
    fn test_contention_replace() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone = ZoneDefinition {
            id: SceneId::new(),
            name: "pip".to_string(),
            description: "Picture in picture".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.80,
                y_pct: 0.80,
                width_pct: 0.18,
                height_pct: 0.18,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        };
        scene.register_zone(zone);

        scene
            .publish_to_zone(
                "pip",
                ZoneContent::SolidColor(Rgba::WHITE),
                "a1",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "pip",
                ZoneContent::SolidColor(Rgba::BLACK),
                "a2",
                None,
                None,
                None,
            )
            .unwrap();

        let publishes = scene.zone_registry.active_for_zone("pip");
        assert_eq!(publishes.len(), 1);
        assert_eq!(publishes[0].publisher_namespace, "a2");
    }

    // ─── Contention policy: apply_contention extraction tests ────────────────
    // These tests were added alongside the extraction of apply_contention (the
    // shared helper used by all three zone/widget publish entry points).  They
    // specifically cover the behaviors that were either untested or diverged in
    // the pre-extraction widget copy.
    //
    // Issue: hud-r5q6p
    //   - max_publishers rejection was untested on zones, absent on widgets.
    //   - max_depth == 0 was treated as "unbounded" on widgets but "reject all"
    //     (trim-to-zero) on zones — the zone behavior is canonical.
    //   - All three entry points (publish_to_zone, publish_to_zone_with_breakpoints,
    //     publish_to_widget) now share the single apply_contention function.

    /// Zone Stack: WHEN a publisher exceeds max_publishers THEN ZoneMaxPublishersReached.
    ///
    /// max_publishers is per-namespace: each agent gets its own per-namespace
    /// slot count.  This test uses a zone with max_publishers=1 and two publishes
    /// from the same namespace to trigger the limit.
    #[test]
    fn test_contention_zone_max_publishers_rejected() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "single-pub".to_string(),
            description: "Stack zone with max_publishers=1".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.5,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 10 },
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        let notification = |text: &str| {
            ZoneContent::Notification(NotificationPayload {
                text: text.to_string(),
                icon: String::new(),
                urgency: 1,
                ttl_ms: None,
                title: String::new(),
                actions: Vec::new(),
            })
        };

        // First publish from "agent.a" succeeds.
        scene
            .publish_to_zone(
                "single-pub",
                notification("first"),
                "agent.a",
                None,
                None,
                None,
            )
            .expect("first publish from agent.a should succeed");

        // Second publish from the same namespace must be rejected.
        let err = scene
            .publish_to_zone(
                "single-pub",
                notification("second"),
                "agent.a",
                None,
                None,
                None,
            )
            .expect_err("second publish from same namespace must be rejected");

        assert!(
            matches!(
                err,
                ValidationError::ZoneMaxPublishersReached { max: 1, .. }
            ),
            "expected ZoneMaxPublishersReached(max=1), got: {err:?}"
        );

        // A different namespace is unaffected — it has its own slot count.
        scene
            .publish_to_zone(
                "single-pub",
                notification("from-b"),
                "agent.b",
                None,
                None,
                None,
            )
            .expect("publish from a different namespace must succeed");
    }

    /// Zone Stack: WHEN max_depth == 0 THEN every publish is trimmed to zero
    /// (canonical behavior — mirrors widget path after apply_contention fix).
    #[test]
    fn test_contention_zone_stack_max_depth_zero_discards_all() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "depth-zero".to_string(),
            description: "Stack zone with max_depth=0".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.5,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 0 },
            max_publishers: 100,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        let notification = |text: &str| {
            ZoneContent::Notification(NotificationPayload {
                text: text.to_string(),
                icon: String::new(),
                urgency: 1,
                ttl_ms: None,
                title: String::new(),
                actions: Vec::new(),
            })
        };

        for i in 0..3 {
            scene
                .publish_to_zone(
                    "depth-zero",
                    notification(&format!("msg{i}")),
                    &format!("agent.{i}"),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }

        let active = scene.zone_registry.active_for_zone("depth-zero");
        assert_eq!(
            active.len(),
            0,
            "Stack(max_depth=0) must trim to 0 — all publishes discarded"
        );
    }

    /// Widget Stack: WHEN a publisher exceeds max_publishers THEN WidgetMaxPublishersReached.
    #[test]
    fn test_contention_widget_max_publishers_rejected() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();

        scene.widget_registry.register_definition(WidgetDefinition {
            id: "counter".to_string(),
            name: "counter".to_string(),
            description: "test counter widget".to_string(),
            parameter_schema: vec![WidgetParameterDeclaration {
                name: "value".to_string(),
                param_type: WidgetParamType::F32,
                default_value: WidgetParameterValue::F32(0.0),
                constraints: None,
            }],
            layers: vec![],
            default_geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.1,
                height_pct: 0.1,
            },
            default_rendering_policy: RenderingPolicy::default(),
            default_contention_policy: ContentionPolicy::Stack { max_depth: 10 },
            max_publishers: 1,
            ephemeral: false,
            hover_behavior: None,
        });
        scene.widget_registry.register_instance(WidgetInstance {
            id: SceneId::new(),
            widget_type_name: "counter".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "counter".to_string(),
            current_params: std::collections::HashMap::from([(
                "value".to_string(),
                WidgetParameterValue::F32(0.0),
            )]),
        });

        let params = || {
            std::collections::HashMap::from([("value".to_string(), WidgetParameterValue::F32(0.5))])
        };

        // First publish from "agent.a" succeeds.
        scene
            .publish_to_widget("counter", params(), "agent.a", None, 0, None)
            .expect("first publish from agent.a should succeed");

        // Second publish from the same namespace must be rejected.
        let err = scene
            .publish_to_widget("counter", params(), "agent.a", None, 0, None)
            .expect_err("second publish from same namespace must be rejected");

        assert!(
            matches!(
                err,
                ValidationError::WidgetMaxPublishersReached { max: 1, .. }
            ),
            "expected WidgetMaxPublishersReached(max=1), got: {err:?}"
        );

        // A different namespace is unaffected.
        scene
            .publish_to_widget("counter", params(), "agent.b", None, 0, None)
            .expect("publish from a different namespace must succeed");
    }

    /// Cross-entry-point: publish_to_zone and publish_to_zone_with_breakpoints
    /// must produce identical record counts and apply identical contention logic.
    ///
    /// This test verifies that both paths share the same apply_contention function.
    #[test]
    fn test_contention_zone_vs_breakpoints_entry_point_consistency() {
        // Zone via publish_to_zone.
        let mut scene_a = SceneGraph::new(1920.0, 1080.0);
        scene_a.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "sub".to_string(),
            description: String::new(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 0.1,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 2 },
            max_publishers: 2,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Zone via publish_to_zone_with_breakpoints.
        let mut scene_b = SceneGraph::new(1920.0, 1080.0);
        scene_b.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "sub".to_string(),
            description: String::new(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 0.1,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 2 },
            max_publishers: 2,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        for (ns, text) in [
            ("agent.a", "hello"),
            ("agent.b", "world"),
            ("agent.a", "overflow"),
        ] {
            let _ = scene_a.publish_to_zone(
                "sub",
                ZoneContent::StreamText(text.to_string()),
                ns,
                None,
                None,
                None,
            );
            let _ = scene_b.publish_to_zone_with_breakpoints(
                "sub",
                ZoneContent::StreamText(text.to_string()),
                ns,
                None,
                None,
                None,
                Vec::new(),
            );
        }

        let count_a = scene_a.zone_registry.active_for_zone("sub").len();
        let count_b = scene_b.zone_registry.active_for_zone("sub").len();
        assert_eq!(
            count_a, count_b,
            "publish_to_zone and publish_to_zone_with_breakpoints must produce identical record counts; got {count_a} vs {count_b}"
        );

        // Both should be 2: agent.a's first publish is at the limit for that namespace
        // (max_publishers=2 across all namespaces but only 1 per ns is counted before
        // the limit kicks in at max_publishers-per-namespace=2).
        // Actually max_publishers is per-namespace: agent.a published "hello" and
        // tried "overflow" as 2nd — 2nd is allowed since max_publishers=2.
        // agent.b published "world" as 1st = allowed.
        // Total stack is trimmed to max_depth=2 from back.
        assert_eq!(
            count_a, 2,
            "Stack(max_depth=2, max_publishers=2) should hold exactly 2 records after 3 publishes"
        );
    }

    #[test]
    fn test_clear_zone() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("hello".to_string()),
                "a1",
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 1);

        scene.clear_zone("subtitle").unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 0);
    }

    #[test]
    fn test_clear_zone_not_found() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result = scene.clear_zone("nonexistent");
        assert!(matches!(result, Err(ValidationError::ZoneNotFound { .. })));
    }

    #[test]
    fn test_zone_registry_snapshot() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("hi".to_string()),
                "a1",
                None,
                None,
                None,
            )
            .unwrap();

        let snap = scene.zone_registry.snapshot();
        assert_eq!(snap.zones.len(), 1);
        assert_eq!(snap.active_publishes.len(), 1);
        assert_eq!(snap.active_publishes[0].zone_name, "subtitle");
    }

    #[test]
    fn test_zone_publish_via_mutation_batch() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        use crate::mutation::{MutationBatch, SceneMutation};

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::PublishToZone {
                zone_name: "subtitle".to_string(),
                content: ZoneContent::StreamText("batch publish".to_string()),
                publish_token: dummy_token(),
                merge_key: None,
                expires_at_wall_us: None,
                content_classification: None,
                breakpoints: Vec::new(),
            }],
            timing_hints: None,
            lease_id: None,
        };

        let result = scene.apply_batch(&batch);
        assert!(result.applied, "batch should be applied");
        let publishes = scene.zone_registry.active_for_zone("subtitle");
        assert_eq!(publishes.len(), 1);
        assert_eq!(
            publishes[0].content,
            ZoneContent::StreamText("batch publish".to_string())
        );
    }

    #[test]
    fn test_clear_zone_via_mutation_batch() {
        // Per spec: ClearZone clears publications by THIS agent (batch.agent_namespace).
        // Publish as "agent", then clear as "agent" — should clear.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("hello".to_string()),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 1);

        use crate::mutation::{MutationBatch, SceneMutation};

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![SceneMutation::ClearZone {
                zone_name: "subtitle".to_string(),
                publish_token: dummy_token(),
            }],
            timing_hints: None,
            lease_id: None,
        };

        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        // "agent" published, "agent" cleared — should be 0
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 0);
    }

    #[test]
    fn test_clear_zone_per_publisher_only_affects_own_publishes() {
        // Publish as two agents; ClearZone from agent "a1" should only remove "a1"'s publish.
        // subtitle zone has max_publishers=2 for this test; use a zone that supports 2 publishers.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        // Use a Stack zone so both publishes can coexist
        let stack_zone = ZoneDefinition {
            id: SceneId::new(),
            name: "shared".to_string(),
            description: "Stack zone for publisher isolation test".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 0.1,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 4 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        };
        scene.register_zone(stack_zone);

        scene
            .publish_to_zone(
                "shared",
                ZoneContent::StreamText("from a1".to_string()),
                "a1",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "shared",
                ZoneContent::StreamText("from a2".to_string()),
                "a2",
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("shared").len(), 2);

        // Clear only "a1"'s publication
        scene.clear_zone_for_publisher("shared", "a1").unwrap();
        let pubs = scene.zone_registry.active_for_zone("shared");
        assert_eq!(pubs.len(), 1);
        assert_eq!(pubs[0].publisher_namespace, "a2");
    }

    #[test]
    fn test_all_or_defer_commits_when_all_ready() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);
        pending.insert(tiles[1]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();

        // All members ready → Commit
        match decision {
            SyncGroupCommitDecision::Commit { tiles: committed } => {
                assert_eq!(committed.len(), 2);
            }
            other => panic!("Expected Commit, got {other:?}"),
        }
        // Deferral counter should be reset to 0
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 0);
    }

    #[test]
    fn test_all_or_defer_defers_when_incomplete() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has a pending mutation
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 1);

        // Second deferral
        let decision2 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision2, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 2);

        // Third deferral
        let decision3 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision3, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 3);
    }

    #[test]
    fn test_all_or_defer_force_commits_after_max_deferrals() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        // max_deferrals = 2
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 2)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has pending mutations — tile[1] is always missing
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        // Frame 1: deferral_count goes 0 → 1
        let d1 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(d1, SyncGroupCommitDecision::Defer);

        // Frame 2: deferral_count goes 1 → 2
        let d2 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(d2, SyncGroupCommitDecision::Defer);

        // Frame 3: deferral_count == max_deferrals (2) → force commit
        let d3 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        match d3 {
            SyncGroupCommitDecision::ForceCommit { tiles: committed } => {
                // Only tile[0] should be committed (tile[1] has no pending)
                assert_eq!(committed, vec![tiles[0]]);
            }
            other => panic!("Expected ForceCommit, got {other:?}"),
        }
        // Deferral counter reset after force-commit
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 0);
    }

    #[test]
    fn test_sync_group_namespace_limit() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);

        // Create 16 sync groups (the namespace limit)
        for i in 0..SceneGraph::MAX_SYNC_GROUPS_PER_NAMESPACE {
            scene
                .create_sync_group(
                    Some(format!("group-{i}")),
                    "agent",
                    SyncCommitPolicy::AllOrDefer,
                    3,
                )
                .unwrap();
        }
        assert_eq!(
            scene.sync_group_count(),
            SceneGraph::MAX_SYNC_GROUPS_PER_NAMESPACE
        );

        // 17th should fail
        let result = scene.create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3);
        assert!(matches!(
            result,
            Err(ValidationError::SyncGroupLimitExceeded { .. })
        ));

        // A different namespace can still create groups
        let other_group =
            scene.create_sync_group(None, "other-agent", SyncCommitPolicy::AllOrDefer, 3);
        assert!(other_group.is_ok());
    }

    // ─── StaticImageNode tests ────────────────────────────────────────────

    /// Build a test `ResourceId` and decoded size for a w×h RGBA8 image.
    ///
    /// Per RS-4 ephemerality contract, `StaticImageNode` carries only the
    /// content-addressed `ResourceId` and the decoded byte count for budget
    /// accounting — no raw pixel data is embedded in the scene graph.
    fn make_test_image_resource(w: u32, h: u32) -> (ResourceId, u64) {
        // Compute a deterministic ResourceId from the dimensions (as a stand-in
        // for "the BLAKE3 hash of the actual pixel bytes").  In production this
        // would be the ResourceId returned by the resource store after upload.
        let fake_bytes: Vec<u8> = (0..w * h).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let resource_id = ResourceId::of(&fake_bytes);
        let decoded_bytes = u64::from(w * h * 4);
        (resource_id, decoded_bytes)
    }

    #[test]
    fn test_static_image_node_creation() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 400.0, 300.0),
                1,
            )
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(64, 48);
        scene.register_resource(resource_id);
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 64,
                height: 48,
                decoded_bytes,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
            }),
        };

        scene.set_tile_root(tile_id, node.clone()).unwrap();
        assert_eq!(scene.node_count(), 1);

        let stored = scene.nodes.get(&node.id).unwrap();
        if let NodeData::StaticImage(si) = &stored.data {
            assert_eq!(si.resource_id, resource_id);
            assert_eq!(si.width, 64);
            assert_eq!(si.height, 48);
            assert_eq!(si.decoded_bytes, 64u64 * 48 * 4);
            assert_eq!(si.fit_mode, ImageFitMode::Contain);
        } else {
            panic!("expected StaticImage node data");
        }
    }

    #[test]
    fn test_static_image_node_all_fit_modes() {
        // Verify all ImageFitMode variants are constructable and round-trip through JSON.
        let (resource_id, decoded_bytes) = make_test_image_resource(4, 4);
        for fit_mode in [
            ImageFitMode::Contain,
            ImageFitMode::Cover,
            ImageFitMode::Fill,
            ImageFitMode::ScaleDown,
        ] {
            let node_data = NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 4,
                height: 4,
                decoded_bytes,
                fit_mode,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            });
            let json = serde_json::to_string(&node_data).unwrap();
            // Acceptance (RS-4): snapshot must NOT contain raw blob data.
            assert!(
                !json.contains("image_data"),
                "snapshot JSON must not contain image_data blob"
            );
            let restored: NodeData = serde_json::from_str(&json).unwrap();
            if let NodeData::StaticImage(si) = restored {
                assert_eq!(si.fit_mode, fit_mode);
                assert_eq!(si.resource_id, resource_id);
            } else {
                panic!("wrong variant after JSON roundtrip");
            }
        }
    }

    #[test]
    fn test_static_image_node_snapshot_roundtrip() {
        let mut scene = SceneGraph::new(1280.0, 720.0);
        let tab_id = scene.create_tab("Tab", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(10.0, 10.0, 200.0, 150.0),
                1,
            )
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(16, 16);
        scene.register_resource(resource_id);
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 16,
                height: 16,
                decoded_bytes,
                fit_mode: ImageFitMode::Cover,
                bounds: Rect::new(0.0, 0.0, 200.0, 150.0),
            }),
        };
        scene.set_tile_root(tile_id, node).unwrap();

        let json = scene.snapshot_json().unwrap();

        // Acceptance (RS-4): scene snapshot includes ResourceId references but NOT blob data.
        // The JSON must not contain raw pixel data.
        assert!(
            !json.contains("image_data"),
            "snapshot JSON must not embed raw image blob data (RS-4 ephemerality contract)"
        );

        let restored = SceneGraph::from_json(&json).unwrap();

        assert_eq!(scene.node_count(), restored.node_count());
        // Verify the node data survived the roundtrip.
        for n in restored.nodes.values() {
            if let NodeData::StaticImage(si) = &n.data {
                assert_eq!(
                    si.resource_id, resource_id,
                    "resource_id must survive snapshot roundtrip"
                );
                assert_eq!(si.fit_mode, ImageFitMode::Cover);
                assert_eq!(si.width, 16);
                assert_eq!(si.height, 16);
                assert_eq!(si.decoded_bytes, decoded_bytes);
            }
        }
    }

    #[test]
    fn test_static_image_node_replace_with_set_tile_root() {
        // Verify that replacing a StaticImageNode via set_tile_root removes the old node.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(8, 8);
        scene.register_resource(resource_id);
        let node1 = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 8,
                height: 8,
                decoded_bytes,
                fit_mode: ImageFitMode::Fill,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            }),
        };
        let node1_id = node1.id;
        scene.set_tile_root(tile_id, node1).unwrap();
        assert_eq!(scene.node_count(), 1);
        assert!(scene.nodes.contains_key(&node1_id));

        // Replace with a SolidColor node.
        let node2 = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                radius: None,
            }),
        };
        scene.set_tile_root(tile_id, node2).unwrap();
        // Old image node should be gone.
        assert!(!scene.nodes.contains_key(&node1_id));
        assert_eq!(scene.node_count(), 1);
    }

    // ─── UpdateNodeContent + StaticImage decoded_bytes tests ────────────

    /// Helper: build a scene with a lease, a tile, and a StaticImage root node.
    /// Returns (scene, lease_id, tile_id, node_id, original_decoded_bytes).
    fn scene_with_static_image_node(
        w: u32,
        h: u32,
    ) -> (SceneGraph, SceneId, SceneId, SceneId, u64) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        let (resource_id, decoded_bytes) = make_test_image_resource(w, h);
        // Register the resource so that subsequent checked mutations (which
        // enforce resource-upload-before-use) can reference it.
        scene.register_resource(resource_id);
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: w,
                height: h,
                decoded_bytes,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
            }),
        };
        let node_id = node.id;
        scene.set_tile_root(tile_id, node).unwrap();
        (scene, lease_id, tile_id, node_id, decoded_bytes)
    }

    #[test]
    fn test_update_static_image_same_resource_preserves_decoded_bytes() {
        // WHEN UpdateNodeContent is applied with the same resource_id and decoded_bytes=0
        // (as proto ingest always produces), the stored decoded_bytes must be preserved.
        let (mut scene, lease_id, tile_id, node_id, original_decoded_bytes) =
            scene_with_static_image_node(64, 48);
        assert_eq!(original_decoded_bytes, 64 * 48 * 4);

        let (resource_id, _) = make_test_image_resource(64, 48);

        // Simulate proto-ingest: decoded_bytes is zeroed out.
        let result = scene.update_node_content_checked(
            tile_id,
            node_id,
            NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 64,
                height: 48,
                decoded_bytes: 0,              // proto ingest always zeros this
                fit_mode: ImageFitMode::Cover, // changed fit mode
                bounds: Rect::new(10.0, 10.0, 380.0, 280.0),
            }),
            "agent",
        );
        assert!(result.is_ok(), "update should succeed: {result:?}");

        // decoded_bytes must be restored from the stored node — not zero.
        let stored = &scene.nodes[&node_id];
        match &stored.data {
            NodeData::StaticImage(si) => {
                assert_eq!(
                    si.decoded_bytes, original_decoded_bytes,
                    "decoded_bytes must be preserved when resource_id is unchanged"
                );
                // Other fields must reflect the update.
                assert_eq!(si.fit_mode, ImageFitMode::Cover);
            }
            _ => panic!("expected StaticImage node"),
        }

        // Texture budget accounting must also reflect the correct bytes.
        let usage = scene.lease_resource_usage(&lease_id);
        assert_eq!(
            usage.texture_bytes, original_decoded_bytes,
            "lease texture_bytes must still account for the full image size"
        );
    }

    #[test]
    fn test_update_static_image_new_resource_uses_caller_decoded_bytes() {
        // WHEN UpdateNodeContent replaces a StaticImage with a different resource_id
        // AND the caller supplies non-zero decoded_bytes (as the session server should),
        // the new decoded_bytes must be used — not the old value.
        let (mut scene, lease_id, tile_id, node_id, original_decoded_bytes) =
            scene_with_static_image_node(64, 48);

        let (new_resource_id, new_decoded_bytes) = make_test_image_resource(128, 96);
        assert_ne!(
            new_resource_id,
            make_test_image_resource(64, 48).0,
            "resources must differ for this test to be meaningful"
        );
        // Register the new resource before the checked update (mirrors real-world
        // flow where the session server uploads the resource before referencing it).
        scene.register_resource(new_resource_id);

        let result = scene.update_node_content_checked(
            tile_id,
            node_id,
            NodeData::StaticImage(StaticImageNode {
                resource_id: new_resource_id,
                width: 128,
                height: 96,
                decoded_bytes: new_decoded_bytes, // caller explicitly provides the new size
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
            }),
            "agent",
        );
        assert!(result.is_ok(), "update should succeed: {result:?}");

        let stored = &scene.nodes[&node_id];
        match &stored.data {
            NodeData::StaticImage(si) => {
                assert_eq!(si.resource_id, new_resource_id);
                assert_eq!(
                    si.decoded_bytes, new_decoded_bytes,
                    "decoded_bytes must reflect the new resource size"
                );
                assert_ne!(
                    si.decoded_bytes, original_decoded_bytes,
                    "old decoded_bytes must not be carried forward to a new resource"
                );
            }
            _ => panic!("expected StaticImage node"),
        }

        let usage = scene.lease_resource_usage(&lease_id);
        assert_eq!(
            usage.texture_bytes, new_decoded_bytes,
            "lease texture_bytes must account for the new image size"
        );
    }

    #[test]
    fn test_update_static_image_decoded_bytes_zero_after_resource_change_is_zero() {
        // WHEN UpdateNodeContent replaces a StaticImage with a different resource_id
        // AND decoded_bytes is 0 (caller bug / missing resource-store lookup),
        // the graph stores 0 (does NOT inherit the old resource's bytes).
        // This is the correct conservative behaviour: it's better to under-report
        // (visible as a budget accounting gap) than to silently charge the wrong amount.
        let (mut scene, _lease_id, tile_id, node_id, _) = scene_with_static_image_node(64, 48);

        let (new_resource_id, _) = make_test_image_resource(128, 96);
        // Register the new resource — even though decoded_bytes is 0 (simulating a
        // caller bug), the resource itself must be registered for the checked path to
        // accept the update.
        scene.register_resource(new_resource_id);

        let result = scene.update_node_content_checked(
            tile_id,
            node_id,
            NodeData::StaticImage(StaticImageNode {
                resource_id: new_resource_id,
                width: 128,
                height: 96,
                decoded_bytes: 0, // caller failed to populate
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
            }),
            "agent",
        );
        assert!(result.is_ok(), "update should succeed");

        let stored = &scene.nodes[&node_id];
        match &stored.data {
            NodeData::StaticImage(si) => {
                assert_eq!(
                    si.decoded_bytes, 0,
                    "with a changed resource_id and decoded_bytes=0, graph must store 0"
                );
            }
            _ => panic!("expected StaticImage node"),
        }
    }

    // ─── Resource ref-count tracking tests (hud-uar4) ────────────────────
    //
    // Spec: resource-store/spec.md §Requirement: Resource Freed On Last Tile Removal
    // When the last tile referencing a resource is removed (via lease expiry,
    // explicit DeleteTile, or SetTileRoot replacement), the resource MUST be freed
    // from the registry.  If another tile still references the same resource the
    // registry entry MUST be preserved.

    /// Single tile with a StaticImage resource: removing the tile frees the resource.
    #[test]
    fn resource_freed_when_only_referencing_tile_is_removed() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                1,
            )
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(32, 32);
        scene.register_resource(resource_id);
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::StaticImage(StaticImageNode {
                        resource_id,
                        width: 32,
                        height: 32,
                        decoded_bytes,
                        fit_mode: ImageFitMode::Contain,
                        bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
                    }),
                },
            )
            .unwrap();

        // Resource must be registered and ref count = 1.
        assert!(
            scene.is_resource_registered(&resource_id),
            "resource must be registered after tile is set"
        );
        assert_eq!(
            scene.resource_ref_count(&resource_id),
            Some(1),
            "ref count must be 1 while one tile references it"
        );

        // Remove the tile (explicit delete).
        scene.delete_tile(tile_id, "agent").unwrap();

        // Resource must be freed.
        assert!(
            !scene.is_resource_registered(&resource_id),
            "resource must be freed when the last referencing tile is removed"
        );
        assert_eq!(
            scene.resource_ref_count(&resource_id),
            None,
            "resource_ref_count must return None after resource is freed"
        );
    }

    /// Two tiles share the same resource: removing one preserves it; removing both frees it.
    #[test]
    fn resource_kept_alive_while_second_tile_references_it_then_freed() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        let tile_a = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                1,
            )
            .unwrap();
        let tile_b = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(200.0, 0.0, 200.0, 200.0),
                2,
            )
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(16, 16);
        scene.register_resource(resource_id);

        let make_image_node = || Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 16,
                height: 16,
                decoded_bytes,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
            }),
        };

        scene.set_tile_root(tile_a, make_image_node()).unwrap();
        scene.set_tile_root(tile_b, make_image_node()).unwrap();

        assert_eq!(
            scene.resource_ref_count(&resource_id),
            Some(2),
            "ref count must be 2 when two tiles reference the same resource"
        );

        // Remove first tile — resource must still be alive.
        scene.delete_tile(tile_a, "agent").unwrap();
        assert!(
            scene.is_resource_registered(&resource_id),
            "resource must still be registered while tile_b references it"
        );
        assert_eq!(
            scene.resource_ref_count(&resource_id),
            Some(1),
            "ref count must drop to 1 after first tile is removed"
        );

        // Remove second tile — resource must be freed.
        scene.delete_tile(tile_b, "agent").unwrap();
        assert!(
            !scene.is_resource_registered(&resource_id),
            "resource must be freed after both tiles are removed"
        );
        assert_eq!(
            scene.resource_ref_count(&resource_id),
            None,
            "resource_ref_count must return None after last tile removed"
        );
    }

    /// Lease expiry path: tiles removed by `expire_leases` also decrement resource refs.
    #[test]
    fn resource_freed_on_lease_expiry() {
        use crate::clock::TestClock;
        let clock = Arc::new(TestClock::new(1_000));
        let mut scene = SceneGraph::new_with_clock(1920.0, 1080.0, clock.clone());

        let tab_id = scene.create_tab("Main", 0).unwrap();
        // Grant a short lease (100 ms TTL).
        let lease_id = scene.grant_lease("agent", 100, vec![Capability::CreateTiles]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                1,
            )
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(8, 8);
        scene.register_resource(resource_id);
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::StaticImage(StaticImageNode {
                        resource_id,
                        width: 8,
                        height: 8,
                        decoded_bytes,
                        fit_mode: ImageFitMode::Contain,
                        bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
                    }),
                },
            )
            .unwrap();

        assert_eq!(scene.resource_ref_count(&resource_id), Some(1));

        // Advance past TTL and trigger lease expiry sweep.
        clock.advance(200);
        let expiries = scene.expire_leases();
        assert_eq!(expiries.len(), 1, "one lease should have expired");
        assert_eq!(expiries[0].removed_tiles.len(), 1, "one tile removed");

        assert!(
            !scene.is_resource_registered(&resource_id),
            "resource must be freed when the lease expires and removes its tile"
        );
    }

    /// SetTileRoot replacement: old resource loses a ref, new resource gains one.
    #[test]
    fn resource_refs_updated_on_set_tile_root_replacement() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::ModifyOwnTiles]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                1,
            )
            .unwrap();

        let (res_a, bytes_a) = make_test_image_resource(4, 4);
        let (res_b, bytes_b) = make_test_image_resource(8, 8);
        scene.register_resource(res_a);
        scene.register_resource(res_b);

        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::StaticImage(StaticImageNode {
                        resource_id: res_a,
                        width: 4,
                        height: 4,
                        decoded_bytes: bytes_a,
                        fit_mode: ImageFitMode::Contain,
                        bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
                    }),
                },
            )
            .unwrap();
        assert_eq!(scene.resource_ref_count(&res_a), Some(1));
        assert_eq!(
            scene.resource_ref_count(&res_b),
            Some(0),
            "res_b registered but not yet referenced by any node"
        );

        // Replace tile root with a node referencing res_b.
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::StaticImage(StaticImageNode {
                        resource_id: res_b,
                        width: 8,
                        height: 8,
                        decoded_bytes: bytes_b,
                        fit_mode: ImageFitMode::Contain,
                        bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
                    }),
                },
            )
            .unwrap();

        // res_a must have been freed (ref count 0 → removed).
        assert!(
            !scene.is_resource_registered(&res_a),
            "res_a must be freed after its node is replaced"
        );
        // res_b must now have ref count 1.
        assert_eq!(
            scene.resource_ref_count(&res_b),
            Some(1),
            "res_b must have ref count 1 after becoming the tile root"
        );
    }

    /// UpdateNodeContent with a different resource_id: ref counts are updated correctly.
    #[test]
    fn resource_refs_updated_on_update_node_content_resource_swap() {
        let (mut scene, _lease_id, tile_id, node_id, _) = scene_with_static_image_node(32, 32);
        let (old_resource_id, _) = make_test_image_resource(32, 32);

        // old resource should have ref count 1 from the initial set_tile_root.
        assert_eq!(scene.resource_ref_count(&old_resource_id), Some(1));

        let (new_resource_id, new_decoded_bytes) = make_test_image_resource(64, 64);
        scene.register_resource(new_resource_id);

        scene
            .update_node_content_checked(
                tile_id,
                node_id,
                NodeData::StaticImage(StaticImageNode {
                    resource_id: new_resource_id,
                    width: 64,
                    height: 64,
                    decoded_bytes: new_decoded_bytes,
                    fit_mode: ImageFitMode::Contain,
                    bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
                }),
                "agent",
            )
            .unwrap();

        // Old resource must be freed.
        assert!(
            !scene.is_resource_registered(&old_resource_id),
            "old resource must be freed after UpdateNodeContent swaps it out"
        );
        // New resource must have ref count 1.
        assert_eq!(
            scene.resource_ref_count(&new_resource_id),
            Some(1),
            "new resource must have ref count 1 after node is updated"
        );
    }

    /// UpdateNodeContent with the SAME resource_id must not change the ref count.
    #[test]
    fn resource_refs_unchanged_on_update_node_content_same_resource() {
        let (mut scene, _lease_id, tile_id, node_id, decoded_bytes) =
            scene_with_static_image_node(32, 32);
        let (resource_id, _) = make_test_image_resource(32, 32);

        assert_eq!(scene.resource_ref_count(&resource_id), Some(1));

        // Update node content with the same resource_id (only fit_mode changes).
        scene
            .update_node_content_checked(
                tile_id,
                node_id,
                NodeData::StaticImage(StaticImageNode {
                    resource_id, // same
                    width: 32,
                    height: 32,
                    decoded_bytes,                 // same
                    fit_mode: ImageFitMode::Cover, // changed
                    bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
                }),
                "agent",
            )
            .unwrap();

        // Ref count must be unchanged.
        assert_eq!(
            scene.resource_ref_count(&resource_id),
            Some(1),
            "ref count must remain 1 when UpdateNodeContent uses the same resource_id"
        );
    }

    // ─── Lease State Machine Tests (RFC 0008) ───────────────────────────

    #[test]
    fn test_lease_state_defaults_to_active() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Active);
        assert!(scene.leases[&lease_id].is_active());
        assert!(scene.leases[&lease_id].is_mutations_allowed());
    }

    #[test]
    fn test_lease_suspend_from_active() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        clock.advance(10_000); // 10s elapsed
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        let lease = &scene.leases[&lease_id];
        assert_eq!(lease.state, LeaseState::Suspended);
        assert!(!lease.is_mutations_allowed());
        assert!(lease.suspended_at_ms.is_some());
        assert!(lease.ttl_remaining_at_suspend_ms.is_some());
        // 60_000 - 10_000 = 50_000 remaining at suspend
        assert_eq!(lease.ttl_remaining_at_suspend_ms, Some(50_000));
    }

    #[test]
    fn test_lease_suspend_invalid_from_non_active() {
        let (mut scene, _clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        // Suspend once (valid)
        scene.suspend_lease(&lease_id, 1000).unwrap();

        // Suspend again from Suspended state (invalid)
        let err = scene.suspend_lease(&lease_id, 2000).unwrap_err();
        assert!(matches!(
            err,
            LeaseError::InvalidTransition {
                from: LeaseState::Suspended,
                to: LeaseState::Suspended,
            }
        ));
    }

    #[test]
    fn test_lease_resume_from_suspended() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        clock.advance(10_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        clock.advance(5_000); // 5s in suspended state
        scene.resume_lease(&lease_id, clock.now_millis()).unwrap();

        let lease = &scene.leases[&lease_id];
        assert_eq!(lease.state, LeaseState::Active);
        assert!(lease.is_mutations_allowed());
        assert!(lease.suspended_at_ms.is_none());
        assert!(lease.ttl_remaining_at_suspend_ms.is_none());
        // After resume: TTL should reflect the remaining time from suspension
        // remaining was 50_000 at suspend; now granted_at_ms is set to resume time
        // so remaining_ms(now) should be ~50_000
        assert_eq!(lease.remaining_ms(clock.now_millis()), 50_000);
    }

    #[test]
    fn test_lease_resume_invalid_from_active() {
        let (mut scene, _clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        let err = scene.resume_lease(&lease_id, 1000).unwrap_err();
        assert!(matches!(
            err,
            LeaseError::InvalidTransition {
                from: LeaseState::Active,
                to: LeaseState::Active,
            }
        ));
    }

    #[test]
    fn test_lease_disconnect_from_active() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        clock.advance(5_000);
        scene
            .disconnect_lease(&lease_id, clock.now_millis())
            .unwrap();

        let lease = &scene.leases[&lease_id];
        assert_eq!(lease.state, LeaseState::Orphaned);
        assert!(!lease.is_mutations_allowed());
        assert_eq!(lease.disconnected_at_ms, Some(6_000)); // 1000 start + 5000
    }

    #[test]
    fn test_lease_disconnect_invalid_from_suspended() {
        let (mut scene, _clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        scene.suspend_lease(&lease_id, 1000).unwrap();

        let err = scene.disconnect_lease(&lease_id, 2000).unwrap_err();
        assert!(matches!(
            err,
            LeaseError::InvalidTransition {
                from: LeaseState::Suspended,
                to: LeaseState::Orphaned,
            }
        ));
    }

    #[test]
    fn test_lease_reconnect_within_grace() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        clock.advance(5_000);
        scene
            .disconnect_lease(&lease_id, clock.now_millis())
            .unwrap();

        // Reconnect within the 30s grace period
        clock.advance(10_000);
        scene
            .reconnect_lease(&lease_id, clock.now_millis())
            .unwrap();

        let lease = &scene.leases[&lease_id];
        assert_eq!(lease.state, LeaseState::Active);
        assert!(lease.is_mutations_allowed());
        assert!(lease.disconnected_at_ms.is_none());
    }

    #[test]
    fn test_lease_reconnect_after_grace_fails() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 120_000, vec![]);

        clock.advance(5_000);
        scene
            .disconnect_lease(&lease_id, clock.now_millis())
            .unwrap();

        // Advance past the 30s grace period
        clock.advance(31_000);
        let err = scene
            .reconnect_lease(&lease_id, clock.now_millis())
            .unwrap_err();
        assert!(matches!(err, LeaseError::InvalidTransition { .. }));
    }

    #[test]
    fn test_lease_revoke_from_any_non_terminal() {
        let (mut scene, _clock) = scene_with_test_clock();

        // Revoke from Active
        let l1 = scene.grant_lease("t1", 60_000, vec![]);
        scene.leases.get_mut(&l1).unwrap().revoke().unwrap();
        assert_eq!(scene.leases[&l1].state, LeaseState::Revoked);

        // Revoke from Suspended
        let l2 = scene.grant_lease("t2", 60_000, vec![]);
        scene.leases.get_mut(&l2).unwrap().suspend(1000).unwrap();
        scene.leases.get_mut(&l2).unwrap().revoke().unwrap();
        assert_eq!(scene.leases[&l2].state, LeaseState::Revoked);

        // Revoke from Orphaned
        let l3 = scene.grant_lease("t3", 60_000, vec![]);
        scene.leases.get_mut(&l3).unwrap().disconnect(1000).unwrap();
        scene.leases.get_mut(&l3).unwrap().revoke().unwrap();
        assert_eq!(scene.leases[&l3].state, LeaseState::Revoked);
    }

    #[test]
    fn test_lease_revoke_from_terminal_fails() {
        let (mut scene, _clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        scene.leases.get_mut(&lease_id).unwrap().revoke().unwrap();

        // Already revoked — should fail
        let err = scene
            .leases
            .get_mut(&lease_id)
            .unwrap()
            .revoke()
            .unwrap_err();
        assert!(matches!(
            err,
            LeaseError::InvalidTransition {
                from: LeaseState::Revoked,
                to: LeaseState::Revoked,
            }
        ));
    }

    #[test]
    fn test_lease_is_expired_not_when_suspended() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 1_000, vec![]);

        // Suspend at t=500ms (halfway)
        clock.advance(500);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Advance well past TTL
        clock.advance(10_000);
        assert!(!scene.leases[&lease_id].is_expired(clock.now_millis()));
    }

    // ─── Budget Enforcement Tests ───────────────────────────────────────

    #[test]
    fn test_budget_tile_count_within_limit() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        // Default budget: max_tiles = 8. Create 1 tile — should be fine.
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert!(!result.budget_warning);
    }

    #[test]
    fn test_budget_tile_count_exceeds_limit() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        // Set budget to max 2 tiles
        scene
            .leases
            .get_mut(&lease_id)
            .unwrap()
            .resource_budget
            .max_tiles = 2;

        // Create 2 tiles (OK)
        for i in 0..2 {
            let batch = crate::mutation::MutationBatch {
                batch_id: SceneId::new(),
                agent_namespace: "test".to_string(),
                mutations: vec![crate::mutation::SceneMutation::CreateTile {
                    tab_id,
                    namespace: "test".to_string(),
                    lease_id,
                    bounds: Rect::new(i as f32 * 120.0, 0.0, 100.0, 100.0),
                    z_order: i + 1,
                }],
                timing_hints: None,
                lease_id: None,
            };
            let result = scene.apply_batch(&batch);
            assert!(result.applied);
        }

        // Create a 3rd tile — should be rejected
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(240.0, 0.0, 100.0, 100.0),
                z_order: 3,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        assert!(result.error.is_some());
        assert_eq!(scene.tile_count(), 2);
    }

    #[test]
    fn test_budget_soft_limit_warning() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        // Set budget to max 5 tiles; soft limit at 80% = 4 tiles
        scene
            .leases
            .get_mut(&lease_id)
            .unwrap()
            .resource_budget
            .max_tiles = 5;

        // Create 4 tiles (should trigger soft limit warning on the 4th)
        for i in 0..4 {
            let batch = crate::mutation::MutationBatch {
                batch_id: SceneId::new(),
                agent_namespace: "test".to_string(),
                mutations: vec![crate::mutation::SceneMutation::CreateTile {
                    tab_id,
                    namespace: "test".to_string(),
                    lease_id,
                    bounds: Rect::new(i as f32 * 120.0, 0.0, 100.0, 100.0),
                    z_order: i + 1,
                }],
                timing_hints: None,
                lease_id: None,
            };
            scene.apply_batch(&batch);
        }

        assert!(scene.is_lease_budget_warning(&lease_id));

        // 5th tile should succeed (within hard limit) but with budget_warning
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(480.0, 0.0, 100.0, 100.0),
                z_order: 5,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert!(result.budget_warning);
    }

    // ─── Suspension Tests ───────────────────────────────────────────────

    #[test]
    fn test_suspend_blocks_mutations() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        // Suspend the lease
        clock.advance(1_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Try to create a tile — should fail
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        assert_eq!(scene.tile_count(), 0);
    }

    #[test]
    fn test_resume_allows_mutations_again() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        // Suspend then resume
        clock.advance(1_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();
        clock.advance(2_000);
        scene.resume_lease(&lease_id, clock.now_millis()).unwrap();

        // Create a tile — should succeed
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert_eq!(scene.tile_count(), 1);
    }

    #[test]
    fn test_ttl_paused_during_suspension() {
        let (mut scene, clock) = scene_with_test_clock();
        // Grant a 10-second lease
        let lease_id = scene.grant_lease("test", 10_000, vec![]);

        // At t=5s, suspend
        clock.advance(5_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();
        let remaining_at_suspend = scene.leases[&lease_id].ttl_remaining_at_suspend_ms;
        assert_eq!(remaining_at_suspend, Some(5_000));

        // Advance 20 seconds while suspended
        clock.advance(20_000);
        // Should NOT be expired (TTL paused)
        assert!(!scene.leases[&lease_id].is_expired(clock.now_millis()));
        // Remaining should still be 5_000
        assert_eq!(
            scene.leases[&lease_id].remaining_ms(clock.now_millis()),
            5_000
        );

        // Resume
        scene.resume_lease(&lease_id, clock.now_millis()).unwrap();
        // Now remaining should be 5_000 from the resume point
        assert_eq!(
            scene.leases[&lease_id].remaining_ms(clock.now_millis()),
            5_000
        );

        // Advance 4 seconds — not yet expired
        clock.advance(4_000);
        assert!(!scene.leases[&lease_id].is_expired(clock.now_millis()));
        assert_eq!(
            scene.leases[&lease_id].remaining_ms(clock.now_millis()),
            1_000
        );

        // Advance 2 more seconds — now expired
        clock.advance(2_000);
        assert!(scene.leases[&lease_id].is_expired(clock.now_millis()));
    }

    // ─── Grace Period Tests ─────────────────────────────────────────────

    #[test]
    fn test_grace_period_disconnect_and_reconnect() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            120_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();

        // Disconnect
        clock.advance(5_000);
        scene
            .disconnect_lease(&lease_id, clock.now_millis())
            .unwrap();
        assert_eq!(scene.tile_count(), 1); // Tiles preserved

        // Reconnect within grace (30s)
        clock.advance(15_000);
        scene
            .reconnect_lease(&lease_id, clock.now_millis())
            .unwrap();
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Active);
        assert_eq!(scene.tile_count(), 1); // Tiles still there
    }

    #[test]
    fn test_grace_period_expiry_cleans_up() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            120_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();

        // Disconnect
        clock.advance(5_000);
        scene
            .disconnect_lease(&lease_id, clock.now_millis())
            .unwrap();

        // Grace period expires (30s)
        clock.advance(31_000);
        let expiries = scene.expire_leases();
        assert_eq!(expiries.len(), 1);
        assert_eq!(expiries[0].terminal_state, LeaseState::Expired);
        assert_eq!(scene.tile_count(), 0); // Tiles cleaned up
    }

    #[test]
    fn test_grace_period_check() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 120_000, vec![]);

        clock.advance(5_000);
        scene
            .disconnect_lease(&lease_id, clock.now_millis())
            .unwrap();

        // Not expired yet
        clock.advance(29_000);
        assert!(!scene.leases[&lease_id].check_grace_expired(clock.now_millis()));

        // Expired
        clock.advance(2_000);
        assert!(scene.leases[&lease_id].check_grace_expired(clock.now_millis()));
    }

    // ─── Safe Mode Tests ────────────────────────────────────────────────

    #[test]
    fn test_suspend_all_leases() {
        let (mut scene, clock) = scene_with_test_clock();
        let l1 = scene.grant_lease("agent1", 60_000, vec![]);
        let l2 = scene.grant_lease("agent2", 60_000, vec![]);
        let l3 = scene.grant_lease("agent3", 60_000, vec![]);

        clock.advance(5_000);
        scene.suspend_all_leases(clock.now_millis());

        assert_eq!(scene.leases[&l1].state, LeaseState::Suspended);
        assert_eq!(scene.leases[&l2].state, LeaseState::Suspended);
        assert_eq!(scene.leases[&l3].state, LeaseState::Suspended);
    }

    #[test]
    fn test_resume_all_leases() {
        let (mut scene, clock) = scene_with_test_clock();
        let l1 = scene.grant_lease("agent1", 60_000, vec![]);
        let l2 = scene.grant_lease("agent2", 60_000, vec![]);

        clock.advance(5_000);
        scene.suspend_all_leases(clock.now_millis());

        clock.advance(2_000);
        scene.resume_all_leases(clock.now_millis());

        assert_eq!(scene.leases[&l1].state, LeaseState::Active);
        assert_eq!(scene.leases[&l2].state, LeaseState::Active);
    }

    #[test]
    fn test_suspend_all_skips_non_active() {
        let (mut scene, clock) = scene_with_test_clock();
        let l1 = scene.grant_lease("agent1", 60_000, vec![]);
        let l2 = scene.grant_lease("agent2", 60_000, vec![]);

        // Disconnect l2 first
        clock.advance(1_000);
        scene.disconnect_lease(&l2, clock.now_millis()).unwrap();

        // Suspend all — only l1 should be suspended
        clock.advance(1_000);
        scene.suspend_all_leases(clock.now_millis());

        assert_eq!(scene.leases[&l1].state, LeaseState::Suspended);
        assert_eq!(scene.leases[&l2].state, LeaseState::Orphaned); // Unchanged (not suspended)
    }

    #[test]
    fn test_resume_all_only_resumes_suspended() {
        let (mut scene, clock) = scene_with_test_clock();
        let l1 = scene.grant_lease("agent1", 60_000, vec![]);
        let l2 = scene.grant_lease("agent2", 60_000, vec![]);

        // Disconnect l2
        clock.advance(1_000);
        scene.disconnect_lease(&l2, clock.now_millis()).unwrap();

        // Suspend only l1
        clock.advance(1_000);
        scene.suspend_lease(&l1, clock.now_millis()).unwrap();

        // Resume all — only l1 should be resumed
        clock.advance(1_000);
        scene.resume_all_leases(clock.now_millis());

        assert_eq!(scene.leases[&l1].state, LeaseState::Active);
        assert_eq!(scene.leases[&l2].state, LeaseState::Orphaned); // Unchanged (not suspended)
    }

    #[test]
    fn test_suspension_timeout_revokes() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            600_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();

        clock.advance(1_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Use a short max_suspend for testing
        let max_suspend = 5_000;
        clock.advance(6_000);

        let expiries = scene.expire_leases_with_max_suspend(max_suspend);
        assert_eq!(expiries.len(), 1);
        assert_eq!(expiries[0].terminal_state, LeaseState::Revoked);
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Revoked);
        assert_eq!(scene.tile_count(), 0);
    }

    // ─── Renewal Policy Tests ───────────────────────────────────────────

    #[test]
    fn test_renewal_policy_defaults_to_manual() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        assert_eq!(
            scene.leases[&lease_id].renewal_policy,
            RenewalPolicy::Manual
        );
    }

    #[test]
    fn test_lease_priority_defaults_to_normal() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        assert_eq!(scene.leases[&lease_id].priority, 2);
    }

    // ─── Priority Persistence Tests ─────────────────────────────────────
    // Spec §Requirement: Priority Assignment (lease-governance/spec.md lines 49-60)
    // Spec §Requirement: Priority Sort Semantics (lease-governance/spec.md lines 62-69)

    /// WHEN grant_lease_with_priority is called with priority 1
    /// THEN the persisted lease priority is 1.
    ///
    /// Validates that the scene graph stores the effective priority verbatim so the
    /// degradation ladder can sort tiles by (lease_priority ASC, z_order DESC) without
    /// consulting the session layer.
    #[test]
    fn test_grant_lease_with_priority_persists_value() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_high = scene.grant_lease_with_priority("agent-high", 60_000, 1, vec![]);
        let lease_normal = scene.grant_lease_with_priority("agent-normal", 60_000, 2, vec![]);
        let lease_low = scene.grant_lease_with_priority("agent-low", 60_000, 3, vec![]);

        assert_eq!(
            scene.leases[&lease_high].priority, 1,
            "high priority must be stored as 1"
        );
        assert_eq!(
            scene.leases[&lease_normal].priority, 2,
            "normal priority must be stored as 2"
        );
        assert_eq!(
            scene.leases[&lease_low].priority, 3,
            "low priority must be stored as 3"
        );
    }

    /// WHEN a lease is renewed THEN the stored priority is preserved unchanged.
    ///
    /// Spec: renewal updates the TTL clock but must not change the effective priority.
    #[test]
    fn test_renew_lease_preserves_priority() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease_with_priority("agent", 60_000, 1, vec![]);

        // Verify priority before renewal.
        assert_eq!(scene.leases[&lease_id].priority, 1);

        // Renew the lease with a new TTL.
        scene
            .renew_lease(lease_id, 120_000)
            .expect("renewal must succeed");

        // Priority must remain unchanged after renewal.
        assert_eq!(
            scene.leases[&lease_id].priority, 1,
            "priority must be preserved across renewal"
        );
        // TTL must be updated.
        assert_eq!(scene.leases[&lease_id].ttl_ms, 120_000);
    }

    /// WHEN multiple leases are granted with distinct priorities
    /// THEN the degradation ladder shedding order is (priority DESC numerically, z_order ASC).
    ///
    /// Spec §Requirement: Tile Shedding Order (runtime-kernel/spec.md lines 263-270):
    /// tiles with the highest lease_priority values (least important) shed first.
    #[test]
    fn test_grant_lease_with_priority_shedding_order() {
        use crate::lease::priority::{TileSheddingEntry, shed_count_for_level4, shedding_order};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let _l_high = scene.grant_lease_with_priority("chrome", 60_000, 0, vec![]);
        let _l_normal = scene.grant_lease_with_priority("agent-normal", 60_000, 2, vec![]);
        let _l_low = scene.grant_lease_with_priority("agent-low", 60_000, 3, vec![]);

        // Build TileSheddingEntry list using the stored priorities.
        // (In production the runtime reads l.priority directly from the lease record.)
        let entries: Vec<TileSheddingEntry> = scene
            .leases
            .values()
            .enumerate()
            .map(|(i, l)| TileSheddingEntry::new(i, l.priority, 5))
            .collect();

        let count = shed_count_for_level4(entries.len());
        let shed = shedding_order(&entries, count);

        // The shed entry must be the lease with the highest priority value (priority=3).
        let shed_priorities: Vec<u8> = shed
            .iter()
            .map(|&i| entries[i].key.lease_priority)
            .collect();
        assert!(
            shed_priorities.iter().all(|&p| p == 3),
            "only the lowest-priority (highest value) lease should shed first; got {shed_priorities:?}"
        );
    }

    // ─── Resource Usage Tests ───────────────────────────────────────────

    #[test]
    fn test_lease_resource_usage() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();
        scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(200.0, 0.0, 100.0, 100.0),
                2,
            )
            .unwrap();

        let usage = scene.lease_resource_usage(&lease_id);
        assert_eq!(usage.tiles, 2);
    }

    #[test]
    fn test_renew_lease_fails_when_not_active() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        // Suspend lease
        clock.advance(1_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Renew should fail (lease not active)
        let err = scene.renew_lease(lease_id, 120_000);
        assert!(err.is_err());
    }

    // ─── Live capability revocation tests (RFC 0001 §3.3) ───────────────────

    /// WHEN a capability is revoked from an active lease
    /// THEN the capability is removed from the scope and the lease stays Active.
    #[test]
    fn revoke_capability_removes_cap_from_active_lease() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        scene
            .revoke_capability(lease_id, &Capability::CreateTiles)
            .expect("revoke_capability must succeed");

        let caps = scene
            .lease_capabilities(&lease_id)
            .expect("lease must exist");
        assert!(
            !caps.contains(&Capability::CreateTiles),
            "CreateTiles must be removed"
        );
        assert!(
            caps.contains(&Capability::ModifyOwnTiles),
            "ModifyOwnTiles must remain"
        );
        // Lease must still be Active.
        assert_eq!(
            scene.leases[&lease_id].state,
            LeaseState::Active,
            "lease must remain Active after capability revocation"
        );
    }

    /// WHEN a capability is revoked
    /// THEN subsequent mutations requiring that capability are rejected with CapabilityMissing.
    ///
    /// This is the core RFC 0001 §3.3 requirement: enforcement is at mutation time
    /// against the live scope, not just at grant time.
    #[test]
    fn revoke_capability_blocks_subsequent_mutations() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![
                Capability::CreateTiles,
                Capability::ModifyOwnTiles,
                Capability::ManageTabs,
            ],
        );

        // CreateTile (no capability check path) succeeds.
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .expect("create_tile must succeed before revocation");

        // Revoke ManageTabs.
        scene
            .revoke_capability(lease_id, &Capability::ManageTabs)
            .expect("revoke must succeed");

        // Tab management is now blocked because ManageTabs was revoked.
        let err = scene
            .create_tab_with_lease("New Tab", 1, lease_id)
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::CapabilityMissing { .. }),
            "expected CapabilityMissing after ManageTabs revocation, got {err:?}"
        );

        // ModifyOwnTiles (not revoked) still works for tile mutations.
        scene
            .update_tile_bounds(tile_id, Rect::new(10.0, 10.0, 50.0, 50.0), "agent")
            .expect("modify_own_tiles must still work");
    }

    /// WHEN revoke_capability is called on a non-existent lease
    /// THEN LeaseNotFound is returned.
    #[test]
    fn revoke_capability_unknown_lease_returns_not_found() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let unknown_id = SceneId::new();
        let err = scene
            .revoke_capability(unknown_id, &Capability::CreateTiles)
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::LeaseNotFound { .. }),
            "expected LeaseNotFound, got {err:?}"
        );
    }

    /// WHEN revoke_capability is called on a terminal (revoked) lease
    /// THEN an InvalidField error is returned.
    #[test]
    fn revoke_capability_on_terminal_lease_returns_invalid_field() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTiles]);
        scene
            .revoke_lease(lease_id)
            .expect("full revoke must succeed");

        let err = scene
            .revoke_capability(lease_id, &Capability::CreateTiles)
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidField { ref field, .. } if field == "lease_terminal"),
            "expected InvalidField(lease_terminal), got {err:?}"
        );
    }

    /// WHEN revoke_capability is called for a cap not in the lease scope
    /// THEN an InvalidField error (capability_not_present) is returned.
    #[test]
    fn revoke_capability_not_in_scope_returns_invalid_field() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTiles]);
        let err = scene
            .revoke_capability(lease_id, &Capability::ManageTabs)
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidField { ref field, .. } if field == "capability_not_present"),
            "expected InvalidField(capability_not_present), got {err:?}"
        );
    }

    /// WHEN all capabilities are revoked one by one
    /// THEN the lease scope is empty and the lease remains Active.
    #[test]
    fn revoke_all_capabilities_leaves_empty_scope_and_active_lease() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        for cap in &[Capability::CreateTiles, Capability::ModifyOwnTiles] {
            scene
                .revoke_capability(lease_id, cap)
                .expect("revoke must succeed");
        }
        let caps = scene
            .lease_capabilities(&lease_id)
            .expect("lease must exist");
        assert!(caps.is_empty(), "capability scope must be empty");
        assert_eq!(
            scene.leases[&lease_id].state,
            LeaseState::Active,
            "lease must remain Active"
        );
    }

    /// WHEN a capability is revoked from a suspended (non-terminal) lease
    /// THEN the capability is removed even in SUSPENDED state.
    #[test]
    fn revoke_capability_on_suspended_lease_succeeds() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        // Suspend the lease (safe mode).
        clock.advance(100);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Capability revocation must succeed on a suspended lease.
        scene
            .revoke_capability(lease_id, &Capability::CreateTiles)
            .expect("revoke must work on suspended lease");

        let caps = scene
            .lease_capabilities(&lease_id)
            .expect("lease must exist");
        assert!(
            !caps.contains(&Capability::CreateTiles),
            "CreateTiles must be removed from suspended lease"
        );
    }

    /// lease_capabilities returns None for unknown lease IDs.
    #[test]
    fn lease_capabilities_returns_none_for_unknown_id() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        assert!(scene.lease_capabilities(&SceneId::new()).is_none());
    }

    /// WHEN revoke_capability succeeds
    /// THEN it returns Ok((cap_name_string, revoked_at_wall_us)) so callers can populate
    /// the LeaseEventKind::CapabilityRevoked audit event fields.
    #[test]
    fn revoke_capability_returns_cap_name_and_timestamp() {
        let (mut scene, clock) = scene_with_test_clock();
        clock.advance(1_000_000); // 1 second in μs
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTiles]);
        let (cap_name, revoked_at_us) = scene
            .revoke_capability(lease_id, &Capability::CreateTiles)
            .expect("revoke_capability must succeed");
        // The name must identify the capability that was removed.
        assert!(
            cap_name.contains("CreateTile"),
            "cap_name must identify CreateTiles, got: {cap_name:?}"
        );
        // The timestamp must be non-zero (clock was advanced before the call).
        assert!(
            revoked_at_us > 0,
            "revoked_at_wall_us must be non-zero, got: {revoked_at_us}"
        );
    }

    #[test]
    fn test_lease_expiry_returns_lease_expiry_struct() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "test",
            500,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();

        clock.advance(501);
        let expiries = scene.expire_leases();
        assert_eq!(expiries.len(), 1);
        assert_eq!(expiries[0].lease_id, lease_id);
        assert_eq!(expiries[0].terminal_state, LeaseState::Expired);
        assert!(expiries[0].removed_tiles.contains(&tile_id));
    }
}

// ─── Spec scenario tests (RFC 0001 §2.1–§2.4) ────────────────────────────────
//
// Each test corresponds to a WHEN/THEN scenario from the issue spec.

#[cfg(test)]
mod spec_scenarios {
    use super::*;
    use crate::clock::TestClock;
    use crate::types::{
        Capability, FontFamily, HitRegionNode, Node, NodeData, Rect, Rgba, SceneId, SolidColorNode,
        TextAlign, TextMarkdownNode, TextOverflow,
    };
    use std::sync::Arc;

    fn make_scene() -> SceneGraph {
        SceneGraph::new(1920.0, 1080.0)
    }

    fn make_scene_with_clock() -> (SceneGraph, Arc<TestClock>) {
        let clock = Arc::new(TestClock::new(1_000_000));
        let scene = SceneGraph::new_with_clock(1920.0, 1080.0, clock.clone());
        (scene, clock)
    }

    // ─ Tab limit enforcement (spec line 50) ──────────────────────────────────
    // WHEN an agent attempts CreateTab and 256 tabs already exist
    // THEN the runtime MUST reject with BudgetExceeded

    #[test]
    fn tab_limit_256_enforced() {
        let mut scene = make_scene();
        for i in 0..MAX_TABS {
            scene
                .create_tab(&format!("Tab {i}"), i as u32)
                .expect("should create tab");
        }
        assert_eq!(scene.tabs.len(), MAX_TABS);
        let err = scene.create_tab("Overflow", MAX_TABS as u32).unwrap_err();
        assert!(
            matches!(err, ValidationError::BudgetExceeded { .. }),
            "expected BudgetExceeded, got {err:?}"
        );
    }

    // ─ Tile limit enforcement (spec line 54) ─────────────────────────────────
    // WHEN an agent attempts CreateTile on a tab that already has 1024 tiles
    // THEN the runtime MUST reject with BudgetExceeded

    #[test]
    fn tile_limit_1024_per_tab_enforced() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        // The test scene is 1920×1080; tiles are 1px×1px at unique positions.
        // Use a grid: 32 cols × 32 rows = 1024. We'll use tiny tiles in bounds.
        // Actually: MAX_TILES_PER_TAB = 1024.
        for i in 0..(MAX_TILES_PER_TAB) {
            let x = (i % 40) as f32 * 48.0;
            let y = (i / 40) as f32 * 42.0;
            if x + 40.0 <= 1920.0 && y + 40.0 <= 1080.0 {
                scene
                    .create_tile(
                        tab_id,
                        "agent",
                        lease_id,
                        Rect::new(x, y, 40.0, 40.0),
                        i as u32,
                    )
                    .expect("should create tile within limit");
            } else {
                // Re-use same position for tiles that would go out of bounds (unchecked path ignores bounds)
                scene
                    .create_tile(
                        tab_id,
                        "agent",
                        lease_id,
                        Rect::new(0.0, 0.0, 1.0, 1.0),
                        i as u32,
                    )
                    .expect("should create tile within limit");
            }
        }
        assert_eq!(
            scene.tiles.values().filter(|t| t.tab_id == tab_id).count(),
            MAX_TILES_PER_TAB
        );

        let err = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 1.0, 1.0),
                MAX_TILES_PER_TAB as u32,
            )
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::BudgetExceeded { .. }),
            "expected BudgetExceeded, got {err:?}"
        );
    }

    // ─ Node limit enforcement (spec line 58) ─────────────────────────────────
    // WHEN an agent attempts InsertNode on a tile with 64 nodes
    // THEN the runtime MUST reject with NodeCountExceeded

    #[test]
    fn node_limit_64_per_tile_enforced() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 400.0, 400.0),
                1,
            )
            .unwrap();

        // Add root node first, then chain children off the root.
        let root_id = SceneId::new();
        let root_node = Node {
            id: root_id,
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 400.0, 400.0),
                radius: None,
            }),
        };
        scene
            .add_node_to_tile(tile_id, None, root_node)
            .expect("root should be added");

        // Add MAX_NODES_PER_TILE - 1 children off the root (total will be MAX_NODES_PER_TILE)
        for i in 1..MAX_NODES_PER_TILE {
            let child = Node {
                id: SceneId::new(),
                children: vec![],
                data: NodeData::SolidColor(SolidColorNode {
                    color: Rgba::new(0.1 * (i % 10) as f32, 0.0, 0.0, 1.0),
                    bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
                    radius: None,
                }),
            };
            scene
                .add_node_to_tile(tile_id, Some(root_id), child)
                .unwrap_or_else(|e| panic!("should add child {i} ok: {e:?}"));
        }

        // Verify we have exactly MAX_NODES_PER_TILE nodes in the tile
        let count = scene.count_node_subtree(root_id);
        assert_eq!(
            count as usize, MAX_NODES_PER_TILE,
            "should have exactly {MAX_NODES_PER_TILE} nodes"
        );

        // One more should be rejected
        let overflow_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::BLACK,
                bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: None,
            }),
        };
        let err = scene
            .add_node_to_tile(tile_id, Some(root_id), overflow_node)
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::NodeCountExceeded { .. }),
            "expected NodeCountExceeded, got {err:?}"
        );
    }

    // ─ Duplicate NodeId rejection (spec line 62) ─────────────────────────────
    // WHEN an agent attempts to add a node with a NodeId that already exists in the scene
    // THEN the runtime MUST reject with DuplicateId

    #[test]
    fn duplicate_node_id_rejected() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                1,
            )
            .unwrap();

        let node_id = SceneId::new();
        let node = Node {
            id: node_id,
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                radius: None,
            }),
        };
        // First insertion succeeds
        scene
            .add_node_to_tile(tile_id, None, node.clone())
            .expect("first insert should succeed");

        // Second insertion with the same node ID should fail
        let tile_id2 = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(200.0, 0.0, 200.0, 200.0),
                2,
            )
            .unwrap();
        let err = scene.add_node_to_tile(tile_id2, None, node).unwrap_err();
        assert!(
            matches!(err, ValidationError::DuplicateId { id } if id == node_id),
            "expected DuplicateId, got {err:?}"
        );
    }

    // ─ Tab name too long (spec line 79) ──────────────────────────────────────
    // WHEN an agent submits CreateTab with a name exceeding 128 UTF-8 bytes
    // THEN the runtime MUST reject with InvalidFieldValue

    #[test]
    fn tab_name_too_long_rejected() {
        let mut scene = make_scene();
        let long_name = "a".repeat(MAX_TAB_NAME_BYTES + 1);
        let err = scene.create_tab(&long_name, 0).unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidField { ref field, .. } if field == "name"),
            "expected InvalidField for name, got {err:?}"
        );
    }

    // ─ Tab mutation without capability (spec line 83) ─────────────────────────
    // WHEN an agent without manage_tabs capability submits CreateTab
    // THEN the runtime MUST reject with CapabilityMissing

    #[test]
    fn tab_create_without_manage_tabs_rejected() {
        let mut scene = make_scene();
        // Lease with no capabilities
        let lease_id = scene.grant_lease("agent", 300_000, vec![]);
        let err = scene
            .create_tab_with_lease("My Tab", 0, lease_id)
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::CapabilityMissing { ref capability } if capability.contains("ManageTabs")),
            "expected CapabilityMissing(ManageTabs), got {err:?}"
        );
    }

    // ─ Create and switch tab (spec line 71) ──────────────────────────────────
    // WHEN an agent with manage_tabs submits CreateTab + SwitchActiveTab
    // THEN the new tab MUST be created and become active

    #[test]
    fn create_and_switch_tab_with_capability() {
        let mut scene = make_scene();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::ManageTabs]);
        let tab_id = scene.create_tab_with_lease("New Tab", 0, lease_id).unwrap();
        scene
            .switch_active_tab_with_lease(tab_id, lease_id)
            .unwrap();
        assert_eq!(scene.active_tab, Some(tab_id));
    }

    // ─ Tab rename (spec line 75) ─────────────────────────────────────────────
    // WHEN an agent submits RenameTab with a new name of 100 UTF-8 bytes
    // THEN the tab name MUST be updated

    #[test]
    fn rename_tab_with_100_byte_name() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Original", 0).unwrap();
        let new_name = "a".repeat(100);
        scene.rename_tab(tab_id, &new_name).unwrap();
        assert_eq!(scene.tabs[&tab_id].name, new_name);
    }

    // ─ Create tile with valid lease (spec line 92) ────────────────────────────
    // WHEN an agent with create_tiles + modify_own_tiles and valid lease submits CreateTile
    // THEN the tile MUST be created with specified bounds, z_order, and opacity

    #[test]
    fn create_tile_checked_requires_capabilities() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // No capabilities — should fail
        let lease_no_caps = scene.grant_lease("agent", 300_000, vec![]);
        let err = scene
            .create_tile_checked(
                tab_id,
                "agent",
                lease_no_caps,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::CapabilityMissing { .. }),
            "got {err:?}"
        );

        // Only create_tiles (not modify_own_tiles) — should still fail
        let lease_create_only = scene.grant_lease("agent", 300_000, vec![Capability::CreateTiles]);
        let err = scene
            .create_tile_checked(
                tab_id,
                "agent",
                lease_create_only,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::CapabilityMissing { .. }),
            "got {err:?}"
        );

        // Full capabilities — should succeed
        let lease_full = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile_checked(
                tab_id,
                "agent",
                lease_full,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                5,
            )
            .unwrap();
        assert_eq!(scene.tiles[&tile_id].z_order, 5);
        assert!((scene.tiles[&tile_id].opacity - 1.0).abs() < f32::EPSILON);
    }

    // ─ Tile mutation with expired lease (spec line 96) ───────────────────────
    // WHEN an agent submits UpdateTileBounds but the tile's lease has expired
    // THEN the runtime MUST reject with LeaseExpired

    #[test]
    fn tile_mutation_with_expired_lease_rejected() {
        let (mut scene, clock) = make_scene_with_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            100,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                1,
            )
            .unwrap();

        // Advance clock past TTL
        clock.advance(200);

        let err = scene
            .update_tile_bounds(tile_id, Rect::new(10.0, 10.0, 100.0, 100.0), "agent")
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::LeaseExpired { .. }),
            "expected LeaseExpired, got {err:?}"
        );
    }

    // ─ Delete tile (spec line 100) ─────────────────────────────────────────────
    // WHEN an agent submits DeleteTile for a tile it owns with a valid lease
    // THEN the tile and all its nodes MUST be removed

    #[test]
    fn delete_tile_removes_tile_and_nodes() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 200.0),
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
                    data: NodeData::SolidColor(SolidColorNode {
                        color: Rgba::WHITE,
                        bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
                        radius: None,
                    }),
                },
            )
            .unwrap();
        assert!(scene.nodes.contains_key(&node_id));

        scene.delete_tile(tile_id, "agent").unwrap();
        assert!(
            !scene.tiles.contains_key(&tile_id),
            "tile should be removed"
        );
        assert!(
            !scene.nodes.contains_key(&node_id),
            "nodes should be removed with tile"
        );
    }

    // ─ Opacity out of range (spec line 109) ──────────────────────────────────
    // WHEN an agent submits UpdateTileOpacity with opacity = 1.5
    // THEN the runtime MUST reject with InvalidFieldValue

    #[test]
    fn opacity_out_of_range_rejected() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                1,
            )
            .unwrap();

        let err = scene
            .update_tile_opacity(tile_id, 1.5, "agent")
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidField { ref field, .. } if field == "opacity"),
            "expected InvalidField(opacity), got {err:?}"
        );

        let err2 = scene
            .update_tile_opacity(tile_id, -0.1, "agent")
            .unwrap_err();
        assert!(
            matches!(err2, ValidationError::InvalidField { .. }),
            "got {err2:?}"
        );
    }

    // ─ Zero-size bounds (spec line 113) ──────────────────────────────────────
    // WHEN an agent submits CreateTile with width = 0.0
    // THEN the runtime MUST reject with BoundsOutOfRange

    #[test]
    fn zero_size_bounds_rejected() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // create_tile_checked requires CreateTiles + ModifyOwnTiles; use correct capabilities
        // so the bounds check is reached (not capability check).
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        let err = scene
            .create_tile_checked(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 0.0, 100.0), // width = 0.0
                1,
            )
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::BoundsOutOfRange { .. }),
            "expected BoundsOutOfRange, got {err:?}"
        );

        // Use the basic create_tile (no capability check) to also confirm bounds are rejected
        let lease_unchecked = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let err2 = scene
            .create_tile(
                tab_id,
                "agent",
                lease_unchecked,
                Rect::new(0.0, 0.0, 0.0, 100.0),
                1,
            )
            .unwrap_err();
        assert!(
            matches!(err2, ValidationError::BoundsOutOfRange { .. }),
            "expected BoundsOutOfRange, got {err2:?}"
        );
    }

    // ─ Bounds outside tab area (spec line 117) ───────────────────────────────
    // WHEN UpdateTileBounds with x + width exceeding tab display width
    // THEN reject with BoundsOutOfRange

    #[test]
    fn bounds_outside_display_rejected() {
        let mut scene = make_scene(); // 1920×1080
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        let err = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(1800.0, 0.0, 200.0, 100.0),
                1,
            ) // x + w = 2000 > 1920
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::BoundsOutOfRange { .. }),
            "expected BoundsOutOfRange, got {err:?}"
        );
    }

    // ─ Z-order in reserved zone band (spec line 121) ─────────────────────────
    // WHEN CreateTile with z_order = ZONE_TILE_Z_MIN
    // THEN reject with InvalidFieldValue

    #[test]
    fn z_order_reserved_zone_band_rejected() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );

        let err = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                ZONE_TILE_Z_MIN,
            )
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidField { ref field, .. } if field == "z_order"),
            "expected InvalidField(z_order), got {err:?}"
        );

        // Also reject z_order above the threshold
        let err2 = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                ZONE_TILE_Z_MIN + 1,
            )
            .unwrap_err();
        assert!(
            matches!(err2, ValidationError::InvalidField { .. }),
            "got {err2:?}"
        );

        // z_order just below threshold is fine
        scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                ZONE_TILE_Z_MIN - 1,
            )
            .expect("z_order just below ZONE_TILE_Z_MIN must succeed");
    }

    // ─ TextMarkdownNode content limit (spec line 130) ─────────────────────────
    // WHEN TextMarkdownNode with content exceeding 65535 UTF-8 bytes
    // THEN reject with InvalidFieldValue

    #[test]
    fn text_markdown_content_limit_enforced() {
        let oversized = "x".repeat(MAX_MARKDOWN_BYTES + 1);
        // Validate that the node construction itself is possible but the validation
        // catches it. We check via validate_node_data if it exists, or directly.
        // For now, test that creating such content is flagged at the graph level.
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: oversized.clone(),
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: None,
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
                color_runs: Box::default(),
            }),
        };
        // The validation function
        let err = validate_text_markdown_node_data(&node.data);
        assert!(err.is_some(), "oversized content should be flagged");
    }

    // ─ Cross-namespace tile access denied (spec line 37) ─────────────────────
    // WHEN agent "weather-agent" attempts to mutate a tile owned by namespace "cal"
    // THEN reject with CapabilityMissing or LeaseNotFound

    #[test]
    fn cross_namespace_tile_access_denied() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let cal_lease = scene.grant_lease(
            "cal",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "cal",
                cal_lease,
                Rect::new(0.0, 0.0, 200.0, 200.0),
                1,
            )
            .unwrap();

        // weather-agent tries to update bounds of cal's tile
        let err = scene
            .update_tile_bounds(tile_id, Rect::new(10.0, 10.0, 100.0, 100.0), "wtr")
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::NamespaceMismatch { .. }),
            "expected NamespaceMismatch, got {err:?}"
        );
    }

    // ─ Struct size budgets (spec line 307, 311) ───────────────────────────────
    // Tile < 200 bytes, Node < 150 bytes

    #[test]
    fn tile_struct_size_under_200_bytes() {
        use std::mem::size_of;
        let tile_size = size_of::<Tile>();
        assert!(
            tile_size < 200,
            "Tile struct is {tile_size} bytes, must be < 200 bytes per RFC 0001 §8"
        );
    }

    #[test]
    fn node_struct_size_under_150_bytes() {
        use std::mem::size_of;
        let node_size = size_of::<Node>();
        assert!(
            node_size < 150,
            "Node struct is {node_size} bytes, must be < 150 bytes per RFC 0001 §8"
        );
    }

    // ─ Tab CRUD full cycle ────────────────────────────────────────────────────

    #[test]
    fn tab_delete_removes_tiles_too() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();
        assert_eq!(scene.tile_count(), 1);

        scene.delete_tab(tab_id).unwrap();
        assert_eq!(scene.tabs.len(), 0, "tab should be removed");
        assert_eq!(scene.tile_count(), 0, "tiles should be removed with tab");
        assert_eq!(
            scene.active_tab, None,
            "active_tab should be None after deleting last tab"
        );
    }

    #[test]
    fn tab_reorder_updates_display_order() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        scene.reorder_tab(tab_id, 5).unwrap();
        assert_eq!(scene.tabs[&tab_id].display_order, 5);
    }

    #[test]
    fn tab_reorder_conflict_rejected() {
        let mut scene = make_scene();
        let tab_a = scene.create_tab("A", 0).unwrap();
        let _tab_b = scene.create_tab("B", 1).unwrap();
        // Try to give tab_a the same order as tab_b
        let err = scene.reorder_tab(tab_a, 1).unwrap_err();
        assert!(
            matches!(err, ValidationError::DuplicateDisplayOrder { .. }),
            "got {err:?}"
        );
    }

    // ─ Opacity valid range ────────────────────────────────────────────────────

    #[test]
    fn tile_opacity_accepts_boundary_values() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                1,
            )
            .unwrap();

        scene.update_tile_opacity(tile_id, 0.0, "agent").unwrap();
        assert!((scene.tiles[&tile_id].opacity - 0.0).abs() < f32::EPSILON);

        scene.update_tile_opacity(tile_id, 1.0, "agent").unwrap();
        assert!((scene.tiles[&tile_id].opacity - 1.0).abs() < f32::EPSILON);

        scene.update_tile_opacity(tile_id, 0.5, "agent").unwrap();
        assert!((scene.tiles[&tile_id].opacity - 0.5).abs() < f32::EPSILON);
    }

    // ─ All 25 test scenes pass Layer 0 invariants ────────────────────────────

    #[test]
    fn all_25_test_scenes_pass_layer0_invariants() {
        use crate::test_scenes::{ClockMs, TestSceneRegistry, assert_layer0_invariants};

        let registry = TestSceneRegistry::new();
        let names = TestSceneRegistry::scene_names();
        assert_eq!(
            names.len(),
            25,
            "must have exactly 25 registered scenes, got {}",
            names.len()
        );

        for name in names {
            let (graph, _spec) = registry
                .build(name, ClockMs::FIXED)
                .unwrap_or_else(|| panic!("scene '{name}' failed to build"));
            let violations = assert_layer0_invariants(&graph);
            assert!(
                violations.is_empty(),
                "scene '{name}' has Layer 0 violations: {violations:?}"
            );
        }
    }

    // ─ V1 node types constructable without GPU ───────────────────────────────

    #[test]
    fn all_v1_node_types_constructable() {
        // SolidColorNode
        let _ = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(0.5, 0.5, 0.5, 1.0),
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                radius: None,
            }),
        };

        // TextMarkdownNode
        let _ = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "# Hello".to_string(),
                bounds: Rect::new(0.0, 0.0, 400.0, 200.0),
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: None,
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
                color_runs: Box::default(),
            }),
        };

        // HitRegionNode
        let _ = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(10.0, 10.0, 100.0, 50.0),
                interaction_id: "btn-ok".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };

        // StaticImageNode — constructable without GPU context
        // RS-4: uses resource_id + decoded_bytes, no raw blob data embedded.
        use crate::types::ImageFitMode;
        use crate::types::StaticImageNode;
        let _ = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id: ResourceId::of(b"4x4 test image"),
                width: 4,
                height: 4,
                decoded_bytes: 4u64 * 4 * 4, // 4×4 RGBA8
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            }),
        };
    }

    // ─── Widget system unit tests ─────────────────────────────────────────────
    //
    // Acceptance criteria from hud-mim2.7:
    // 1. WidgetParameterValue validation (f32 NaN/Inf rejection, type mismatch, enum constraint)
    // 2. Widget registry (definition registration, instance creation, publish, occupancy)
    // 3. Widget contention policies (LatestWins, Stack, MergeByKey, Replace)
    //
    // Source: widget-system/spec.md §Requirement: Widget Parameter Validation,
    //         §Requirement: Widget Registry, §Requirement: Widget Contention.

    // ── Helpers ───────────────────────────────────────────────────────────────

    use crate::types::{
        ContentionPolicy, GeometryPolicy, RenderingPolicy, WidgetDefinition, WidgetInstance,
        WidgetParamConstraints, WidgetParamType, WidgetParameterDeclaration, WidgetParameterValue,
        WidgetSvgLayer,
    };

    /// Build a minimal gauge WidgetDefinition for testing.
    ///
    /// Parameters: level (f32, 0–1), label (string), severity (enum info/warning/error).
    fn make_gauge_definition() -> WidgetDefinition {
        WidgetDefinition {
            id: "gauge".to_string(),
            name: "gauge".to_string(),
            description: "test gauge".to_string(),
            parameter_schema: vec![
                WidgetParameterDeclaration {
                    name: "level".to_string(),
                    param_type: WidgetParamType::F32,
                    default_value: WidgetParameterValue::F32(0.0),
                    constraints: Some(WidgetParamConstraints {
                        f32_min: Some(0.0),
                        f32_max: Some(1.0),
                        ..Default::default()
                    }),
                },
                WidgetParameterDeclaration {
                    name: "label".to_string(),
                    param_type: WidgetParamType::String,
                    default_value: WidgetParameterValue::String(String::new()),
                    constraints: None,
                },
                WidgetParameterDeclaration {
                    name: "severity".to_string(),
                    param_type: WidgetParamType::Enum,
                    default_value: WidgetParameterValue::Enum("info".to_string()),
                    constraints: Some(WidgetParamConstraints {
                        enum_allowed_values: vec![
                            "info".to_string(),
                            "warning".to_string(),
                            "error".to_string(),
                        ],
                        ..Default::default()
                    }),
                },
            ],
            layers: vec![WidgetSvgLayer {
                svg_file: "fill.svg".to_string(),
                bindings: vec![],
            }],
            default_geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.25,
            },
            default_rendering_policy: RenderingPolicy::default(),
            default_contention_policy: ContentionPolicy::LatestWins,
            max_publishers: u32::MAX,
            ephemeral: false,
            hover_behavior: None,
        }
    }

    /// Register gauge definition + instance in a scene with one tab.
    fn scene_with_gauge(contention: ContentionPolicy) -> (SceneGraph, SceneId /* tab_id */) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();

        let mut def = make_gauge_definition();
        def.default_contention_policy = contention;

        scene.widget_registry.register_definition(def);
        scene.widget_registry.register_instance(WidgetInstance {
            id: SceneId::new(),
            widget_type_name: "gauge".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "gauge".to_string(),
            current_params: std::collections::HashMap::from([
                ("level".to_string(), WidgetParameterValue::F32(0.0)),
                (
                    "label".to_string(),
                    WidgetParameterValue::String(String::new()),
                ),
                (
                    "severity".to_string(),
                    WidgetParameterValue::Enum("info".to_string()),
                ),
            ]),
        });

        (scene, tab_id)
    }

    // ── WidgetParameterValue validation ───────────────────────────────────────

    /// WHEN an f32 NaN value is submitted THEN publish_to_widget returns
    /// WidgetParameterInvalidValue.
    /// Source: widget-system/spec.md §Requirement: Widget Parameter Validation (F32 invariant).
    #[test]
    fn widget_publish_f32_nan_rejected() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "level".to_string(),
            WidgetParameterValue::F32(f32::NAN),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(
            matches!(
                result,
                Err(ValidationError::WidgetParameterInvalidValue { .. })
            ),
            "NaN f32 should produce WidgetParameterInvalidValue, got: {result:?}"
        );
    }

    /// WHEN an f32 +Inf value is submitted THEN publish_to_widget returns
    /// WidgetParameterInvalidValue.
    #[test]
    fn widget_publish_f32_pos_inf_rejected() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "level".to_string(),
            WidgetParameterValue::F32(f32::INFINITY),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(
            matches!(
                result,
                Err(ValidationError::WidgetParameterInvalidValue { .. })
            ),
            "positive infinity f32 should produce WidgetParameterInvalidValue, got: {result:?}"
        );
    }

    /// WHEN an f32 -Inf value is submitted THEN publish_to_widget returns
    /// WidgetParameterInvalidValue.
    #[test]
    fn widget_publish_f32_neg_inf_rejected() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "level".to_string(),
            WidgetParameterValue::F32(f32::NEG_INFINITY),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(
            matches!(
                result,
                Err(ValidationError::WidgetParameterInvalidValue { .. })
            ),
            "negative infinity f32 should produce WidgetParameterInvalidValue, got: {result:?}"
        );
    }

    /// WHEN a string value is submitted for an f32 parameter THEN type mismatch error.
    /// Source: widget-system/spec.md §Requirement: Widget Parameter Validation (type safety).
    #[test]
    fn widget_publish_f32_type_mismatch_rejected() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "level".to_string(),
            WidgetParameterValue::String("not a float".to_string()),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(
            matches!(
                result,
                Err(ValidationError::WidgetParameterTypeMismatch { .. })
            ),
            "string for f32 param should produce WidgetParameterTypeMismatch, got: {result:?}"
        );
    }

    /// WHEN an enum value outside allowed_values is submitted THEN invalid value error.
    /// Source: widget-system/spec.md §Requirement: Widget Parameter Validation (enum constraint).
    #[test]
    fn widget_publish_enum_out_of_allowed_values_rejected() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "severity".to_string(),
            WidgetParameterValue::Enum("critical".to_string()),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(
            matches!(
                result,
                Err(ValidationError::WidgetParameterInvalidValue { .. })
            ),
            "enum value outside allowed_values should produce WidgetParameterInvalidValue, got: {result:?}"
        );
    }

    /// WHEN an enum value within allowed_values is submitted THEN publish succeeds.
    #[test]
    fn widget_publish_enum_in_allowed_values_accepted() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "severity".to_string(),
            WidgetParameterValue::Enum("warning".to_string()),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(
            result.is_ok(),
            "valid enum value should be accepted, got: {result:?}"
        );
    }

    /// WHEN an f32 value is within [min, max] THEN it is accepted unchanged.
    #[test]
    fn widget_publish_f32_in_range_accepted_unchanged() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "level".to_string(),
            WidgetParameterValue::F32(0.75),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(result.is_ok(), "in-range f32 should be accepted");
    }

    /// WHEN an f32 value exceeds max THEN it is clamped, not rejected.
    /// Source: widget-system/spec.md — f32 out of range is clamped.
    #[test]
    fn widget_publish_f32_above_max_clamped() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        // level has max=1.0; submit 2.5 — should clamp to 1.0 without error
        let params = std::collections::HashMap::from([(
            "level".to_string(),
            WidgetParameterValue::F32(2.5),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(result.is_ok(), "out-of-range f32 should clamp, not reject");

        // The recorded publish should contain the clamped value.
        let pubs = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(pubs.len(), 1);
        let recorded_level = pubs[0].params.get("level");
        assert!(
            matches!(recorded_level, Some(WidgetParameterValue::F32(v)) if (*v - 1.0).abs() < 1e-6),
            "clamped value should be 1.0, got: {recorded_level:?}"
        );
    }

    /// WHEN a parameter name is not in the widget schema THEN unknown-parameter error.
    #[test]
    fn widget_publish_unknown_parameter_rejected() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "bogus_param".to_string(),
            WidgetParameterValue::F32(0.5),
        )]);
        let result = scene.publish_to_widget("gauge", params, "agent.test", None, 0, None);
        assert!(
            matches!(result, Err(ValidationError::WidgetUnknownParameter { .. })),
            "unknown param name should produce WidgetUnknownParameter, got: {result:?}"
        );
    }

    /// WHEN a widget instance is not found THEN WidgetNotFound error.
    #[test]
    fn widget_publish_nonexistent_widget_rejected() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let params = std::collections::HashMap::from([(
            "level".to_string(),
            WidgetParameterValue::F32(0.5),
        )]);
        let result = scene.publish_to_widget("no-such-widget", params, "agent", None, 0, None);
        assert!(
            matches!(result, Err(ValidationError::WidgetNotFound { .. })),
            "nonexistent widget should produce WidgetNotFound, got: {result:?}"
        );
    }

    // ── Widget registry unit tests ─────────────────────────────────────────────

    /// WHEN a widget definition is registered THEN it can be retrieved by id.
    /// Source: widget-system/spec.md §Requirement: Widget Registry.
    #[test]
    fn widget_registry_register_and_retrieve_definition() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let def = make_gauge_definition();
        scene.widget_registry.register_definition(def.clone());

        let retrieved = scene.widget_registry.get_definition("gauge");
        assert!(
            retrieved.is_some(),
            "registered definition should be retrievable"
        );
        assert_eq!(retrieved.unwrap().id, "gauge");
        assert_eq!(retrieved.unwrap().parameter_schema.len(), 3);
    }

    /// WHEN a widget instance is registered THEN it can be retrieved by instance_name.
    #[test]
    fn widget_registry_register_and_retrieve_instance() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();

        scene
            .widget_registry
            .register_definition(make_gauge_definition());
        let instance = WidgetInstance {
            id: SceneId::new(),
            widget_type_name: "gauge".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "cpu-gauge".to_string(),
            current_params: Default::default(),
        };
        scene.widget_registry.register_instance(instance);

        let retrieved = scene.widget_registry.get_instance("cpu-gauge");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().instance_name, "cpu-gauge");
        assert_eq!(retrieved.unwrap().widget_type_name, "gauge");
    }

    /// WHEN a definition is registered with the same id THEN it overwrites the old one.
    #[test]
    fn widget_registry_definition_overwrites_on_duplicate_id() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut def1 = make_gauge_definition();
        def1.description = "first".to_string();
        let mut def2 = make_gauge_definition();
        def2.description = "second".to_string();

        scene.widget_registry.register_definition(def1);
        scene.widget_registry.register_definition(def2);

        let retrieved = scene.widget_registry.get_definition("gauge").unwrap();
        assert_eq!(
            retrieved.description, "second",
            "second registration should win"
        );
    }

    #[test]
    fn widget_registry_runtime_svg_handle_round_trip() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.widget_registry.register_runtime_svg_handle(
            "gauge",
            "fill.svg",
            "asset:runtime-handle",
        );
        assert_eq!(
            scene
                .widget_registry
                .runtime_svg_handle("gauge", "fill.svg"),
            Some("asset:runtime-handle")
        );
    }

    /// `remove_tile_and_nodes` populates `recently_removed_tile_ids`; draining
    /// that queue via `drain_removed_tile_ids` yields the removed tile ID.
    ///
    /// This is the scene-layer half of the hud-4tuw5 contract.  The windowed
    /// runtime drains this queue in `prune_portal_resize_states` to eagerly
    /// remove the tile's entry from `portal_resize_states`.
    #[test]
    fn portal_resize_drain_queue_populated_by_remove_tile() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![
                crate::Capability::CreateTiles,
                crate::Capability::ModifyOwnTiles,
            ],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                crate::Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();

        // Drain queue must be empty before any removal.
        assert!(
            scene.drain_removed_tile_ids().is_empty(),
            "drain queue must be empty before any tile removal"
        );

        // Remove the tile via the canonical path.
        scene.remove_tile_and_nodes(tile_id);

        // The tile must no longer be in the tiles map.
        assert!(
            !scene.tiles.contains_key(&tile_id),
            "tile must be absent from scene after remove_tile_and_nodes"
        );

        // Drain the queue — must yield exactly the removed tile ID.
        let removed_ids = scene.drain_removed_tile_ids();
        assert_eq!(
            removed_ids,
            vec![tile_id],
            "drain queue must contain exactly the removed tile ID (hud-4tuw5)"
        );

        // Queue must be empty after drain (idempotent).
        assert!(
            scene.drain_removed_tile_ids().is_empty(),
            "drain queue must be empty after drain"
        );
    }

    /// Multiple successive tile removals each append to the drain queue;
    /// a single `drain_removed_tile_ids` call returns all of them.
    #[test]
    fn portal_resize_drain_queue_accumulates_multiple_removals() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "portal-agent",
            60_000,
            vec![
                crate::Capability::CreateTiles,
                crate::Capability::ModifyOwnTiles,
            ],
        );
        let tile_a = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                crate::Rect::new(0.0, 0.0, 300.0, 200.0),
                1,
            )
            .unwrap();
        let tile_b = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                crate::Rect::new(400.0, 0.0, 300.0, 200.0),
                2,
            )
            .unwrap();

        scene.remove_tile_and_nodes(tile_a);
        scene.remove_tile_and_nodes(tile_b);

        let removed_ids = scene.drain_removed_tile_ids();
        assert_eq!(
            removed_ids.len(),
            2,
            "both removed tile IDs must be in queue"
        );
        assert!(
            removed_ids.contains(&tile_a),
            "tile_a must be in the drain queue"
        );
        assert!(
            removed_ids.contains(&tile_b),
            "tile_b must be in the drain queue"
        );

        assert!(
            scene.drain_removed_tile_ids().is_empty(),
            "drain queue must be empty after drain"
        );
    }

    #[test]
    fn pending_widget_svg_queue_drains_in_fifo_order() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.enqueue_widget_svg_asset("gauge", "a.svg", vec![1, 2, 3]);
        scene.enqueue_widget_svg_asset("gauge", "b.svg", vec![4, 5]);

        let drained = scene.drain_pending_widget_svg_assets();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, "gauge");
        assert_eq!(drained[0].1, "a.svg");
        assert_eq!(drained[0].2, vec![1, 2, 3]);
        assert_eq!(drained[1].1, "b.svg");
        assert!(scene.drain_pending_widget_svg_assets().is_empty());
    }

    /// WHEN querying occupancy with no active publications THEN effective_params
    /// falls back to the definition's parameter defaults.
    #[test]
    fn widget_registry_occupancy_defaults_when_no_publications() {
        let (scene, tab_id) = scene_with_gauge(ContentionPolicy::LatestWins);

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(occ.occupant_count, 0);
        assert_eq!(occ.active_publications.len(), 0);

        // Should fall back to definition defaults for all three declared parameters.
        let level = occ.effective_params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.0).abs() < 1e-6),
            "default level should be 0.0, got: {level:?}"
        );
        let label = occ.effective_params.get("label");
        assert!(
            matches!(label, Some(WidgetParameterValue::String(s)) if s.is_empty()),
            "default label should be empty string, got: {label:?}"
        );
        let severity = occ.effective_params.get("severity");
        assert!(
            matches!(severity, Some(WidgetParameterValue::Enum(s)) if s == "info"),
            "default severity should be 'info', got: {severity:?}"
        );
    }

    /// WHEN querying occupancy for an unknown instance THEN None is returned.
    #[test]
    fn widget_registry_occupancy_unknown_instance_returns_none() {
        let (scene, tab_id) = scene_with_gauge(ContentionPolicy::LatestWins);
        let occ = scene.widget_registry.get_occupancy("no-such-gauge", tab_id);
        assert!(occ.is_none(), "unknown instance should return None");
    }

    // ── get_occupancy per-policy effective_params tests ───────────────────────

    /// LatestWins: WHEN one publication is active THEN effective_params = that
    /// publication's params merged over schema defaults.
    ///
    /// Source: widget-system/spec.md §Requirement: Widget Contention.
    #[test]
    fn widget_occupancy_latest_wins_merges_over_defaults() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::LatestWins);

        // Publish only "level"; "label" and "severity" should fall back to defaults.
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.75),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(occ.occupant_count, 1);

        // Published param should reflect the publication value.
        let level = occ.effective_params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.75).abs() < 1e-6),
            "LatestWins level should be 0.75, got: {level:?}"
        );

        // Unpublished params should retain schema defaults.
        let label = occ.effective_params.get("label");
        assert!(
            matches!(label, Some(WidgetParameterValue::String(s)) if s.is_empty()),
            "LatestWins: missing label should fall back to default empty string, got: {label:?}"
        );
        let severity = occ.effective_params.get("severity");
        assert!(
            matches!(severity, Some(WidgetParameterValue::Enum(s)) if s == "info"),
            "LatestWins: missing severity should fall back to default 'info', got: {severity:?}"
        );
    }

    /// LatestWins: WHEN two sequential publishes arrive THEN effective_params
    /// reflects only the most recent one (merged over defaults).
    #[test]
    fn widget_occupancy_latest_wins_uses_most_recent() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::LatestWins);

        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.2),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.9),
                )]),
                "agent.b",
                None,
                0,
                None,
            )
            .unwrap();

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(
            occ.occupant_count, 1,
            "LatestWins retains only 1 publication"
        );
        let level = occ.effective_params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.9).abs() < 1e-6),
            "LatestWins: most recent level (0.9) should win, got: {level:?}"
        );
    }

    /// Stack: WHEN three publishes arrive THEN effective_params reflects the
    /// top-of-stack (most recent) publication merged over defaults.
    ///
    /// Source: widget-system/spec.md §Requirement: Widget Contention (Stack).
    #[test]
    fn widget_occupancy_stack_uses_top_of_stack() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::Stack { max_depth: 5 });

        for (i, level) in [0.1f32, 0.5f32, 0.8f32].iter().enumerate() {
            scene
                .publish_to_widget(
                    "gauge",
                    std::collections::HashMap::from([(
                        "level".to_string(),
                        WidgetParameterValue::F32(*level),
                    )]),
                    &format!("agent.{i}"),
                    None,
                    0,
                    None,
                )
                .unwrap();
        }

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(
            occ.occupant_count, 3,
            "Stack should have 3 active publications"
        );

        // Top-of-stack = most recent = last pushed = 0.8.
        let level = occ.effective_params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.8).abs() < 1e-6),
            "Stack: top-of-stack level should be 0.8, got: {level:?}"
        );

        // Unpublished params should fall back to schema defaults.
        let label = occ.effective_params.get("label");
        assert!(
            matches!(label, Some(WidgetParameterValue::String(s)) if s.is_empty()),
            "Stack: missing label should fall back to default empty string, got: {label:?}"
        );
    }

    /// Stack: WHEN stack exceeds max_depth THEN effective_params still reflects
    /// the most recent (top-of-stack) publication.
    #[test]
    fn widget_occupancy_stack_top_after_depth_cap() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::Stack { max_depth: 3 });

        // Push 5 publications; oldest 2 will be evicted, leaving levels [0.2, 0.3, 0.4].
        for (i, level) in [0.0f32, 0.1f32, 0.2f32, 0.3f32, 0.4f32].iter().enumerate() {
            scene
                .publish_to_widget(
                    "gauge",
                    std::collections::HashMap::from([(
                        "level".to_string(),
                        WidgetParameterValue::F32(*level),
                    )]),
                    &format!("agent.{i}"),
                    None,
                    0,
                    None,
                )
                .unwrap();
        }

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(
            occ.occupant_count, 3,
            "Stack(3) should cap at 3 publications"
        );

        // Top-of-stack is the most recent surviving publication (0.4).
        let level = occ.effective_params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.4).abs() < 1e-6),
            "Stack: top-of-stack after depth cap should be 0.4, got: {level:?}"
        );
    }

    /// MergeByKey: WHEN two different-keyed publications are active THEN
    /// effective_params merges both over defaults.
    ///
    /// Source: widget-system/spec.md §Requirement: Widget Contention (MergeByKey).
    #[test]
    fn widget_occupancy_merge_by_key_merges_all_keys_over_defaults() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::MergeByKey { max_keys: 8 });

        // "cpu" key sets level=0.4; "mem" key sets level=0.6.
        // Since both touch the same param ("level"), the last-inserted key wins.
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.4),
                )]),
                "agent.a",
                Some("cpu".to_string()),
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([
                    ("level".to_string(), WidgetParameterValue::F32(0.6)),
                    (
                        "label".to_string(),
                        WidgetParameterValue::String("mem".to_string()),
                    ),
                ]),
                "agent.b",
                Some("mem".to_string()),
                0,
                None,
            )
            .unwrap();

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(
            occ.occupant_count, 2,
            "MergeByKey should have 2 active publications"
        );

        // "mem" was pushed after "cpu", so its level (0.6) wins for "level".
        let level = occ.effective_params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.6).abs() < 1e-6),
            "MergeByKey: last-inserted key's level (0.6) should win, got: {level:?}"
        );

        // "label" was only set by "mem" — should appear in effective_params.
        let label = occ.effective_params.get("label");
        assert!(
            matches!(label, Some(WidgetParameterValue::String(s)) if s == "mem"),
            "MergeByKey: label from 'mem' key should be 'mem', got: {label:?}"
        );

        // "severity" was not set by either key — should fall back to schema default.
        let severity = occ.effective_params.get("severity");
        assert!(
            matches!(severity, Some(WidgetParameterValue::Enum(s)) if s == "info"),
            "MergeByKey: missing severity should fall back to default 'info', got: {severity:?}"
        );
    }

    /// MergeByKey: WHEN the same key is updated THEN effective_params reflects
    /// the updated value.
    #[test]
    fn widget_occupancy_merge_by_key_updated_key_reflects_latest_value() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::MergeByKey { max_keys: 8 });

        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.3),
                )]),
                "agent.a",
                Some("cpu".to_string()),
                0,
                None,
            )
            .unwrap();
        // Same key — should replace the previous value in-place.
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.7),
                )]),
                "agent.a",
                Some("cpu".to_string()),
                0,
                None,
            )
            .unwrap();

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(
            occ.occupant_count, 1,
            "Same-key update should not add a second record"
        );

        let level = occ.effective_params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.7).abs() < 1e-6),
            "MergeByKey: updated key level should be 0.7, got: {level:?}"
        );
    }

    /// Replace: WHEN a publication is active THEN effective_params = that
    /// publication's params only (no defaults for missing keys).
    ///
    /// Source: widget-system/spec.md §Requirement: Widget Contention (Replace).
    #[test]
    fn widget_occupancy_replace_no_default_fallback_for_missing_keys() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::Replace);

        // Publish only "level" — "label" and "severity" are omitted intentionally.
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.5),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(occ.occupant_count, 1);

        let level = occ.effective_params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.5).abs() < 1e-6),
            "Replace level should be 0.5, got: {level:?}"
        );

        // Replace must NOT include defaults for missing keys.
        assert!(
            !occ.effective_params.contains_key("label"),
            "Replace: absent keys must NOT be filled from defaults (label), got: {:?}",
            occ.effective_params.get("label")
        );
        assert!(
            !occ.effective_params.contains_key("severity"),
            "Replace: absent keys must NOT be filled from defaults (severity), got: {:?}",
            occ.effective_params.get("severity")
        );
    }

    /// Replace: WHEN two sequential publishes arrive THEN effective_params
    /// reflects only the most recent one (no merge, no defaults).
    #[test]
    fn widget_occupancy_replace_uses_most_recent_params_only() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::Replace);

        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.1),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "label".to_string(),
                    WidgetParameterValue::String("replaced".to_string()),
                )]),
                "agent.b",
                None,
                0,
                None,
            )
            .unwrap();

        let occ = scene
            .widget_registry
            .get_occupancy("gauge", tab_id)
            .unwrap();
        assert_eq!(occ.occupant_count, 1, "Replace retains only 1 publication");

        // Second publish only set "label"; "level" must NOT appear (not in params,
        // and Replace does not fall back to defaults).
        assert!(
            !occ.effective_params.contains_key("level"),
            "Replace: prior 'level' must be gone after Replace by second publish, got: {:?}",
            occ.effective_params.get("level")
        );
        let label = occ.effective_params.get("label");
        assert!(
            matches!(label, Some(WidgetParameterValue::String(s)) if s == "replaced"),
            "Replace: label from second publish should be 'replaced', got: {label:?}"
        );
    }

    /// WHEN a publish is recorded THEN active_for_widget returns it.
    #[test]
    fn widget_registry_publish_recorded_in_active_for_widget() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);
        let params = std::collections::HashMap::from([(
            "level".to_string(),
            WidgetParameterValue::F32(0.8),
        )]);
        scene
            .publish_to_widget("gauge", params, "agent.a", None, 0, None)
            .unwrap();

        let active = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(active.len(), 1);
        let level = active[0].params.get("level");
        assert!(
            matches!(level, Some(WidgetParameterValue::F32(v)) if (*v - 0.8).abs() < 1e-6),
            "recorded level should be 0.8, got: {level:?}"
        );
    }

    /// WHEN snapshot() is called THEN it includes all registered types and instances.
    #[test]
    fn widget_registry_snapshot_includes_all_types_and_instances() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::LatestWins);

        // Add a second instance
        scene.widget_registry.register_instance(WidgetInstance {
            id: SceneId::new(),
            widget_type_name: "gauge".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "mem-gauge".to_string(),
            current_params: Default::default(),
        });

        let snapshot = scene.widget_registry.snapshot();
        assert_eq!(snapshot.widget_types.len(), 1, "one type registered");
        assert_eq!(snapshot.widget_instances.len(), 2, "two instances");
    }

    // ── Widget contention policy tests ─────────────────────────────────────────

    /// LatestWins: WHEN two publishes arrive THEN only the latest is retained.
    /// Source: widget-system/spec.md §Requirement: Widget Contention.
    #[test]
    fn widget_contention_latest_wins_replaces_previous() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);

        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.3),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.7),
                )]),
                "agent.b",
                None,
                0,
                None,
            )
            .unwrap();

        let active = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(active.len(), 1, "LatestWins keeps only one publication");
        assert!(
            matches!(active[0].params.get("level"), Some(WidgetParameterValue::F32(v)) if (*v - 0.7).abs() < 1e-6),
            "latest publish (0.7) should win"
        );
    }

    /// Replace: identical to LatestWins in effect — only one record retained.
    #[test]
    fn widget_contention_replace_retains_only_latest() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::Replace);

        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.1),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.9),
                )]),
                "agent.b",
                None,
                0,
                None,
            )
            .unwrap();

        let active = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(active.len(), 1, "Replace keeps only one publication");
        assert!(
            matches!(active[0].params.get("level"), Some(WidgetParameterValue::F32(v)) if (*v - 0.9).abs() < 1e-6),
        );
    }

    /// Stack: WHEN max_depth=3 and 4 publishes arrive THEN oldest is evicted.
    /// Source: widget-system/spec.md §Requirement: Widget Contention (Stack depth cap).
    #[test]
    fn widget_contention_stack_evicts_oldest_at_max_depth() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::Stack { max_depth: 3 });

        for i in 0u32..4 {
            scene
                .publish_to_widget(
                    "gauge",
                    std::collections::HashMap::from([(
                        "level".to_string(),
                        WidgetParameterValue::F32(i as f32 * 0.25),
                    )]),
                    &format!("agent.{i}"),
                    None,
                    0,
                    None,
                )
                .unwrap();
        }

        let active = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(active.len(), 3, "Stack(3) should keep at most 3 records");

        // The oldest (i=0, level=0.0) should have been evicted.
        let has_zero = active.iter().any(|r| {
            matches!(r.params.get("level"), Some(WidgetParameterValue::F32(v)) if (*v).abs() < 1e-6)
        });
        assert!(!has_zero, "oldest publish (level=0.0) should be evicted");

        // The correct items (i=1,2,3) should all be present.
        let levels: std::collections::BTreeSet<u32> = active
            .iter()
            .filter_map(|r| {
                if let Some(WidgetParameterValue::F32(v)) = r.params.get("level") {
                    Some((v * 4.0).round() as u32)
                } else {
                    None
                }
            })
            .collect();
        let expected_levels: std::collections::BTreeSet<u32> = [1, 2, 3].into();
        assert_eq!(
            levels, expected_levels,
            "Stack(3) should contain levels for i=1, 2, 3"
        );
    }

    /// Stack: WHEN max_depth=0 THEN every publish is immediately trimmed out,
    /// leaving the stack empty.
    ///
    /// Canonical semantics (matches zone publish_to_zone behavior): the push is
    /// followed by a trim that drains all entries when max_depth == 0, so the
    /// record is silently discarded.  The old widget implementation had a
    /// diverged `if max > 0 &&` guard that made max_depth=0 unbounded instead —
    /// that was a bug corrected by extracting apply_contention.
    #[test]
    fn widget_contention_stack_max_depth_zero_discards_all() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::Stack { max_depth: 0 });

        for i in 0u32..3 {
            scene
                .publish_to_widget(
                    "gauge",
                    std::collections::HashMap::from([(
                        "level".to_string(),
                        WidgetParameterValue::F32(i as f32 * 0.1),
                    )]),
                    &format!("agent.{i}"),
                    None,
                    0,
                    None,
                )
                .unwrap();
        }

        let active = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(
            active.len(),
            0,
            "Stack(0) trims to 0: all publishes must be discarded (canonical semantics)"
        );
    }

    /// MergeByKey: WHEN same key is published twice THEN the record is replaced.
    /// WHEN a different key is published THEN both records coexist.
    /// Source: widget-system/spec.md §Requirement: Widget Contention (MergeByKey).
    #[test]
    fn widget_contention_merge_by_key_replaces_same_key() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::MergeByKey { max_keys: 8 });

        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.4),
                )]),
                "agent.a",
                Some("cpu".to_string()),
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.6),
                )]),
                "agent.b",
                Some("mem".to_string()),
                0,
                None,
            )
            .unwrap();
        // Overwrite "cpu" key
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.2),
                )]),
                "agent.a",
                Some("cpu".to_string()),
                0,
                None,
            )
            .unwrap();

        let active = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(active.len(), 2, "MergeByKey should keep one record per key");

        let cpu_pub = active
            .iter()
            .find(|r| r.merge_key.as_deref() == Some("cpu"))
            .unwrap();
        assert!(
            matches!(cpu_pub.params.get("level"), Some(WidgetParameterValue::F32(v)) if (*v - 0.2).abs() < 1e-6),
            "cpu key should have updated to 0.2"
        );

        // The mem key must remain unaffected at its original value (0.6).
        let mem_pub = active
            .iter()
            .find(|r| r.merge_key.as_deref() == Some("mem"))
            .unwrap();
        assert!(
            matches!(mem_pub.params.get("level"), Some(WidgetParameterValue::F32(v)) if (*v - 0.6).abs() < 1e-6),
            "mem key should be unaffected and still be 0.6"
        );
    }

    // ── Widget publication TTL / expiry tests ─────────────────────────────────

    /// Helper: scene with a gauge backed by a controllable TestClock.
    fn scene_with_gauge_and_clock(
        contention: ContentionPolicy,
    ) -> (SceneGraph, SceneId, TestClock) {
        let clock = TestClock::new(1_000); // t=1 000 ms = 1 000 000 µs
        let mut scene = SceneGraph::new_with_clock(1920.0, 1080.0, Arc::new(clock.clone()));
        let tab_id = scene.create_tab("Main", 0).unwrap();

        let mut def = make_gauge_definition();
        def.default_contention_policy = contention;
        scene.widget_registry.register_definition(def);
        scene.widget_registry.register_instance(WidgetInstance {
            id: SceneId::new(),
            widget_type_name: "gauge".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "gauge".to_string(),
            current_params: std::collections::HashMap::from([
                ("level".to_string(), WidgetParameterValue::F32(0.0)),
                (
                    "label".to_string(),
                    WidgetParameterValue::String(String::new()),
                ),
                (
                    "severity".to_string(),
                    WidgetParameterValue::Enum("info".to_string()),
                ),
            ]),
        });

        (scene, tab_id, clock)
    }

    /// WHEN drain_expired_widget_publications is called before any expiry time
    /// has elapsed THEN no publications are removed.
    ///
    /// Source: widget-system/spec.md §Requirement: Expiration Policy.
    #[test]
    fn widget_ttl_publication_not_expired_before_deadline() {
        let (mut scene, _tab, _clock) = scene_with_gauge_and_clock(ContentionPolicy::LatestWins);

        // Publish with an expiry 10 s in the future (clock is at 1 000 ms = 1 000 000 µs).
        let expires_at = 1_000_000u64 + 10_000_000u64; // +10 s
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.5),
                )]),
                "agent.test",
                None,
                0,
                Some(expires_at),
            )
            .unwrap();

        // Drain without advancing the clock — publication must survive.
        let removed = scene.drain_expired_widget_publications();
        assert_eq!(removed, 0, "no publications should expire before deadline");
        assert_eq!(
            scene.widget_registry.active_for_widget("gauge").len(),
            1,
            "publication must still be present"
        );
    }

    /// WHEN drain_expired_widget_publications is called after the expiry time
    /// has elapsed THEN the publication is removed.
    ///
    /// Source: widget-system/spec.md §Requirement: Expiration Policy.
    #[test]
    fn widget_ttl_publication_expires_after_deadline() {
        let (mut scene, _tab, clock) = scene_with_gauge_and_clock(ContentionPolicy::LatestWins);

        // Publish with a 1 s TTL (expires 1 s after t=1 000 ms).
        let expires_at = 1_000_000u64 + 1_000_000u64; // expires at t=2 000 ms
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.5),
                )]),
                "agent.test",
                None,
                0,
                Some(expires_at),
            )
            .unwrap();

        // Advance clock past the expiry point.
        clock.advance(1_001); // now at t=2 001 ms = 2 001 000 µs

        let removed = scene.drain_expired_widget_publications();
        assert_eq!(removed, 1, "one publication should have expired");
        assert_eq!(
            scene.widget_registry.active_for_widget("gauge").len(),
            0,
            "expired publication must be removed"
        );
    }

    /// WHEN drain_expired_widget_publications removes all publications from a
    /// widget THEN the active_publishes entry is cleaned up (no empty Vec left).
    ///
    /// Source: widget-system/spec.md §Requirement: Expiration Policy.
    #[test]
    fn widget_ttl_empty_entry_cleaned_up_after_expiry() {
        let (mut scene, _tab, clock) = scene_with_gauge_and_clock(ContentionPolicy::LatestWins);

        let expires_at = 1_000_000u64 + 500_000u64; // +500 ms
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.75),
                )]),
                "agent.test",
                None,
                0,
                Some(expires_at),
            )
            .unwrap();

        clock.advance(600); // advance 600 ms past expiry
        scene.drain_expired_widget_publications();

        // The HashMap entry itself must be gone (no empty Vec).
        assert!(
            !scene.widget_registry.active_publishes.contains_key("gauge"),
            "empty widget publication entry must be removed after expiry"
        );
    }

    /// WHEN a publication with no expiry and one with an expiry coexist (Stack
    /// policy) THEN only the expired publication is removed.
    ///
    /// Source: widget-system/spec.md §Requirement: Expiration Policy.
    #[test]
    fn widget_ttl_only_expired_publication_removed_when_mixed() {
        let (mut scene, _tab, clock) =
            scene_with_gauge_and_clock(ContentionPolicy::Stack { max_depth: 10 });

        let now_us = 1_000_000u64; // clock starts at t=1 000 ms
        let expires_soon = now_us + 500_000u64; // expires in 500 ms

        // Publish the soon-to-expire record first.
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.1),
                )]),
                "agent.short",
                None,
                0,
                Some(expires_soon),
            )
            .unwrap();

        // Publish a permanent record (no expiry).
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.9),
                )]),
                "agent.permanent",
                None,
                0,
                None,
            )
            .unwrap();

        assert_eq!(
            scene.widget_registry.active_for_widget("gauge").len(),
            2,
            "both publications should be present before expiry"
        );

        // Advance clock past the short expiry.
        clock.advance(600);

        let removed = scene.drain_expired_widget_publications();
        assert_eq!(removed, 1, "only the TTL publication should expire");

        let remaining = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(remaining.len(), 1, "one publication should remain");
        assert_eq!(
            remaining[0].publisher_namespace, "agent.permanent",
            "the permanent publication should survive"
        );
    }

    /// WHEN drain_expired_widget_publications removes a publication THEN the
    /// scene version is incremented.
    ///
    /// Source: widget-system/spec.md §Requirement: Expiration Policy.
    #[test]
    fn widget_ttl_expiry_bumps_scene_version() {
        let (mut scene, _tab, clock) = scene_with_gauge_and_clock(ContentionPolicy::LatestWins);

        let expires_at = 1_000_000u64 + 200_000u64;
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.3),
                )]),
                "agent.test",
                None,
                0,
                Some(expires_at),
            )
            .unwrap();

        let version_before = scene.version;
        clock.advance(300);
        scene.drain_expired_widget_publications();

        assert!(
            scene.version > version_before,
            "scene version must be incremented when a widget publication expires"
        );
    }

    /// WHEN drain_expired_widget_publications is called with no publications
    /// THEN it returns 0 and does not panic.
    ///
    /// Source: widget-system/spec.md §Requirement: Expiration Policy.
    #[test]
    fn widget_ttl_drain_with_no_publications_is_noop() {
        let (mut scene, _tab, _clock) = scene_with_gauge_and_clock(ContentionPolicy::LatestWins);

        let removed = scene.drain_expired_widget_publications();
        assert_eq!(removed, 0, "draining an empty registry must return 0");
    }

    // ── clear_widget_for_publisher tests ──────────────────────────────────────

    /// WHEN clear_widget_for_publisher is called with the publishing namespace
    /// THEN that agent's publications are removed and the widget reverts to defaults.
    #[test]
    fn clear_widget_for_publisher_removes_own_publications() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);

        // Publish as "agent.a"
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.9),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        assert_eq!(scene.widget_registry.active_for_widget("gauge").len(), 1);

        // Clear as "agent.a" — should remove the publication
        scene
            .clear_widget_for_publisher("gauge", "agent.a")
            .unwrap();
        assert_eq!(
            scene.widget_registry.active_for_widget("gauge").len(),
            0,
            "agent.a's publication should be cleared"
        );
        match scene.widget_registry.instances["gauge"]
            .current_params
            .get("level")
        {
            Some(WidgetParameterValue::F32(v)) => {
                assert!(
                    (*v - 0.0).abs() < f32::EPSILON,
                    "level should reset to default after clear, got {v}"
                )
            }
            other => panic!("expected default F32 level after clear, got {other:?}"),
        }
    }

    /// WHEN the top stacked widget publication is cleared THEN current_params
    /// refresh to the remaining publication instead of retaining stale pixels.
    #[test]
    fn clear_widget_for_publisher_refreshes_current_params_from_remaining_publish() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::Stack { max_depth: 4 });

        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.3),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.7),
                )]),
                "agent.b",
                None,
                0,
                None,
            )
            .unwrap();

        scene
            .clear_widget_for_publisher("gauge", "agent.b")
            .unwrap();

        match scene.widget_registry.instances["gauge"]
            .current_params
            .get("level")
        {
            Some(WidgetParameterValue::F32(v)) => {
                assert!(
                    (*v - 0.3).abs() < f32::EPSILON,
                    "level should refresh to remaining publication, got {v}"
                )
            }
            other => panic!("expected remaining F32 level after clear, got {other:?}"),
        }
    }

    /// WHEN clear_widget_for_publisher is called with a different namespace
    /// THEN only the matching publisher's records are removed.
    #[test]
    fn clear_widget_for_publisher_only_affects_own_publications() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::Stack { max_depth: 4 });

        // Publish as "agent.a" and "agent.b"
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.3),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.7),
                )]),
                "agent.b",
                None,
                0,
                None,
            )
            .unwrap();
        assert_eq!(scene.widget_registry.active_for_widget("gauge").len(), 2);

        // Clear as "agent.a" — only "agent.a"'s publication should be removed
        scene
            .clear_widget_for_publisher("gauge", "agent.a")
            .unwrap();
        let remaining = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(
            remaining.len(),
            1,
            "only agent.a's publication should be cleared"
        );
        assert_eq!(
            remaining[0].publisher_namespace, "agent.b",
            "agent.b's publication should remain"
        );
    }

    /// WHEN clear_widget_for_publisher is called for a namespace with no publications
    /// THEN it succeeds as a no-op.
    #[test]
    fn clear_widget_for_publisher_noop_when_no_publications() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);

        // No publications yet — clear should succeed silently
        let result = scene.clear_widget_for_publisher("gauge", "agent.nobody");
        assert!(
            result.is_ok(),
            "should succeed even when no publications exist"
        );
        assert_eq!(scene.widget_registry.active_for_widget("gauge").len(), 0);
    }

    /// WHEN clear_widget_for_publisher is called with an unknown widget name
    /// THEN it returns WidgetNotFound.
    #[test]
    fn clear_widget_for_publisher_widget_not_found() {
        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::LatestWins);

        let result = scene.clear_widget_for_publisher("nonexistent", "agent.a");
        assert!(
            matches!(result, Err(ValidationError::WidgetNotFound { .. })),
            "unknown widget should produce WidgetNotFound, got: {result:?}"
        );
    }

    /// WHEN clear_widget_publications_for_namespace is called
    /// THEN ALL widget publications for that namespace are removed across all widgets.
    #[test]
    fn clear_widget_publications_for_namespace_removes_all_for_namespace() {
        let (mut scene, tab_id) = scene_with_gauge(ContentionPolicy::LatestWins);

        // Register a second widget instance using the same definition
        scene.widget_registry.register_instance(WidgetInstance {
            id: SceneId::new(),
            widget_type_name: "gauge".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "mem-gauge".to_string(),
            current_params: Default::default(),
        });

        // Publish as "agent.a" to both widgets
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.5),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "mem-gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.8),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();

        // Publish as "agent.b" to "gauge" only
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.9),
                )]),
                "agent.b",
                None,
                0,
                None,
            )
            .unwrap();

        // Clear ALL of "agent.a" publications
        scene.clear_widget_publications_for_namespace("agent.a");

        // "agent.a"'s publication on "gauge" is gone; "agent.b"'s remains
        let gauge_pubs = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(
            gauge_pubs.len(),
            1,
            "only agent.b's gauge pub should remain"
        );
        assert_eq!(gauge_pubs[0].publisher_namespace, "agent.b");

        // "agent.a"'s publication on "mem-gauge" is gone
        let mem_pubs = scene.widget_registry.active_for_widget("mem-gauge");
        assert_eq!(
            mem_pubs.len(),
            0,
            "agent.a's mem-gauge pub should be cleared"
        );
        match scene.widget_registry.instances["mem-gauge"]
            .current_params
            .get("level")
        {
            Some(WidgetParameterValue::F32(v)) => {
                assert!(
                    (*v - 0.0).abs() < f32::EPSILON,
                    "mem-gauge should reset to default, got {v}"
                )
            }
            other => {
                panic!("expected default level for mem-gauge after namespace clear, got {other:?}")
            }
        }
    }

    /// WHEN ClearWidget is sent as a scene mutation batch
    /// THEN it removes the agent's publications via the standard pipeline.
    #[test]
    fn clear_widget_via_mutation_batch() {
        use crate::mutation::{MutationBatch, SceneMutation};

        let (mut scene, _tab) = scene_with_gauge(ContentionPolicy::Stack { max_depth: 4 });

        // Publish as two agents
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.5),
                )]),
                "agent.a",
                None,
                0,
                None,
            )
            .unwrap();
        scene
            .publish_to_widget(
                "gauge",
                std::collections::HashMap::from([(
                    "level".to_string(),
                    WidgetParameterValue::F32(0.3),
                )]),
                "agent.b",
                None,
                0,
                None,
            )
            .unwrap();
        assert_eq!(scene.widget_registry.active_for_widget("gauge").len(), 2);

        // Send ClearWidget from "agent.a"
        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent.a".to_string(),
            mutations: vec![SceneMutation::ClearWidget {
                widget_name: "gauge".to_string(),
                instance_id: None,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(result.applied, "ClearWidget batch should be accepted");

        // Only "agent.b"'s publication should remain
        let remaining = scene.widget_registry.active_for_widget("gauge");
        assert_eq!(
            remaining.len(),
            1,
            "agent.a's publication should be cleared"
        );
        assert_eq!(remaining[0].publisher_namespace, "agent.b");
    }

    // ─── Cycle-guard tests ───────────────────────────────────────────────────
    //
    // These tests inject synthetic cycles directly into `scene.nodes` (bypassing
    // the public API which would normally prevent cycles) to verify that each DFS
    // traversal function terminates instead of recursing indefinitely.

    /// Helper: build a SolidColor node with explicit id and children list.
    fn solid_node(id: SceneId, children: Vec<SceneId>) -> Node {
        Node {
            id,
            children,
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                radius: None,
            }),
        }
    }

    /// Helper: build a HitRegion node with explicit id and children list.
    fn hit_node(id: SceneId, children: Vec<SceneId>) -> Node {
        Node {
            id,
            children,
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                interaction_id: "cycle-test".to_string(),
                accepts_pointer: true,
                accepts_focus: false,
                ..Default::default()
            }),
        }
    }

    /// count_node_subtree: cycle A→B→A terminates and returns a finite count.
    #[test]
    fn count_node_subtree_cycle_terminates() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        let id_b = SceneId::new();
        // A points to B, B points back to A — a direct 2-node cycle.
        scene.nodes.insert(id_a, solid_node(id_a, vec![id_b]));
        scene.nodes.insert(id_b, solid_node(id_b, vec![id_a]));

        // Must not hang; result should be finite (2: A + B, cycle back to A is skipped).
        let count = scene.count_node_subtree(id_a);
        assert_eq!(count, 2, "cycle should be detected; each node counted once");
    }

    /// count_node_subtree: self-referencing node (A→A) terminates.
    #[test]
    fn count_node_subtree_self_loop_terminates() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        scene.nodes.insert(id_a, solid_node(id_a, vec![id_a]));

        let count = scene.count_node_subtree(id_a);
        assert_eq!(count, 1, "self-loop: node counted once, cycle skipped");
    }

    /// sum_texture_bytes: cycle terminates and returns zero (no StaticImage nodes).
    #[test]
    fn sum_texture_bytes_cycle_terminates() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        let id_b = SceneId::new();
        scene.nodes.insert(id_a, solid_node(id_a, vec![id_b]));
        scene.nodes.insert(id_b, solid_node(id_b, vec![id_a]));

        // Must not hang; no StaticImage nodes so result is 0.
        let bytes = scene.sum_texture_bytes(id_a);
        assert_eq!(
            bytes, 0,
            "cycle should terminate; no texture bytes in solid-color nodes"
        );
    }

    /// hit_test_node: cycle terminates; HitRegion nodes in a cycle are still tested.
    #[test]
    fn hit_test_node_cycle_terminates() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        let id_b = SceneId::new();
        // Both nodes are HitRegion with accepts_pointer=true; A→B→A forms a cycle.
        scene.nodes.insert(id_a, hit_node(id_a, vec![id_b]));
        scene.nodes.insert(id_b, hit_node(id_b, vec![id_a]));

        // Point (50,50) is inside both nodes' bounds (0,0,100,100). Must not hang.
        let hit = scene.hit_test_node(id_a, 50.0, 50.0);
        assert!(
            hit.is_some(),
            "a HitRegion node should be found before cycle is detected"
        );
    }

    /// hit_test_node: no hit when point is outside all node bounds.
    #[test]
    fn hit_test_node_cycle_no_hit_outside_bounds() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        let id_b = SceneId::new();
        scene.nodes.insert(id_a, hit_node(id_a, vec![id_b]));
        scene.nodes.insert(id_b, hit_node(id_b, vec![id_a]));

        // Point (200, 200) is outside bounds (0,0,100,100). Must not hang.
        let hit = scene.hit_test_node(id_a, 200.0, 200.0);
        assert!(
            hit.is_none(),
            "point outside all bounds should yield no hit"
        );
    }

    /// is_node_in_subtree: returns true for a direct child.
    #[test]
    fn is_node_in_subtree_direct_child() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        let id_b = SceneId::new();
        scene.nodes.insert(id_a, solid_node(id_a, vec![id_b]));
        scene.nodes.insert(id_b, solid_node(id_b, vec![]));

        assert!(scene.is_node_in_subtree(id_a, id_b));
        assert!(!scene.is_node_in_subtree(id_b, id_a));
    }

    /// is_node_in_subtree: returns true when target equals root.
    #[test]
    fn is_node_in_subtree_root_equals_target() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        scene.nodes.insert(id_a, solid_node(id_a, vec![]));

        assert!(scene.is_node_in_subtree(id_a, id_a));
    }

    /// is_node_in_subtree: cycle A→B→A terminates; B is reachable from A.
    #[test]
    fn is_node_in_subtree_cycle_terminates() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        let id_b = SceneId::new();
        scene.nodes.insert(id_a, solid_node(id_a, vec![id_b]));
        scene.nodes.insert(id_b, solid_node(id_b, vec![id_a]));

        // Must not hang; B is reachable from A.
        assert!(scene.is_node_in_subtree(id_a, id_b));
    }

    /// is_node_in_subtree: cycle terminates when target is not in the subgraph.
    #[test]
    fn is_node_in_subtree_cycle_unreachable_node() {
        let mut scene = make_scene();
        let id_a = SceneId::new();
        let id_b = SceneId::new();
        let id_c = SceneId::new(); // not inserted — unreachable
        scene.nodes.insert(id_a, solid_node(id_a, vec![id_b]));
        scene.nodes.insert(id_b, solid_node(id_b, vec![id_a]));

        // Must not hang; C is not reachable from A.
        assert!(!scene.is_node_in_subtree(id_a, id_c));
    }
}
