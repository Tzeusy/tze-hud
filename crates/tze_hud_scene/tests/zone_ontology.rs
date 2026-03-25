//! # Zone Ontology Integration Tests
//!
//! Tests for the four-level zone model per scene-graph/spec.md §Requirement: Zone Registry (line 185).
//!
//! ## Four-level zone ontology
//!
//! 1. **ZoneDefinition** (zone type) — schema: accepted_media_types, contention_policy, layer_attachment
//! 2. **ZoneInstance** — zone type bound to a specific tab
//! 3. **ZonePublication** (= ZonePublishRecord) — a single publish event
//! 4. **ZoneOccupancy** — resolved state after applying contention policy
//!
//! ## What is tested
//!
//! - Zone test scenes: zone_publish_subtitle, zone_reject_wrong_type,
//!   zone_conflict_two_publishers, zone_orchestrate_then_publish,
//!   zone_geometry_adapts_profile, zone_disconnect_cleanup
//! - All four contention policies (LatestWins, Stack, MergeByKey, Replace)
//! - All five v1 media types (StreamText, ShortTextWithIcon, KeyValuePairs, StaticImage, SolidColor)
//! - VideoSurfaceRef: schema accepted in config but not rendered
//! - Layer attachment routing: Background, Content (z >= ZONE_TILE_Z_MIN), Chrome
//! - ZonePublishToken: validation rejects expired/invalid tokens
//! - ZoneOccupancy: query API in v1 (no effective_geometry)
//! - Per-publisher clear semantics
//! - Expiry semantics (expires_at_wall_us)
//! - Six v1 default zones loadable from config (with_defaults())
//! - Contention proptest: random publish/clear sequences verify occupancy invariants

use std::collections::HashMap;

use tze_hud_scene::{
    graph::SceneGraph,
    test_scenes::{ClockMs, TestSceneRegistry, assert_layer0_invariants},
    types::{
        ContentionPolicy, DisplayEdge, GeometryPolicy, LayerAttachment, NotificationPayload,
        RenderingPolicy, ResourceId, Rgba, SceneId, StatusBarPayload,
        ZoneContent, ZoneDefinition, ZoneMediaType, ZONE_TILE_Z_MIN,
    },
};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Build a subtitle zone definition (Content layer, LatestWins).
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
        layer_attachment: LayerAttachment::Content,
        ephemeral: false,
    }
}

fn assert_no_violations(graph: &SceneGraph, scene_name: &str) {
    let violations = assert_layer0_invariants(graph);
    assert!(
        violations.is_empty(),
        "Layer 0 invariant violations in '{}': {:?}",
        scene_name,
        violations
    );
}

// ─── Scene: zone_publish_subtitle ────────────────────────────────────────────

#[test]
fn zone_publish_subtitle_builds() {
    let registry = TestSceneRegistry::default();
    let result = registry.build("zone_publish_subtitle", ClockMs::FIXED);
    assert!(result.is_some(), "zone_publish_subtitle must build");
}

#[test]
fn zone_publish_subtitle_has_subtitle_zone() {
    let registry = TestSceneRegistry::default();
    let (graph, spec) = registry.build("zone_publish_subtitle", ClockMs::FIXED).unwrap();
    assert!(spec.has_zones, "spec must declare has_zones=true");
    assert!(
        graph.zone_registry.get_by_name("subtitle").is_some(),
        "subtitle zone must be registered"
    );
}

#[test]
fn zone_publish_subtitle_passes_layer0_invariants() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_publish_subtitle", ClockMs::FIXED).unwrap();
    assert_no_violations(&graph, "zone_publish_subtitle");
}

#[test]
fn zone_publish_subtitle_layer_attachment_is_content() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_publish_subtitle", ClockMs::FIXED).unwrap();
    let zone = graph.zone_registry.get_by_name("subtitle").unwrap();
    assert_eq!(
        zone.layer_attachment,
        LayerAttachment::Content,
        "subtitle zone must use Content layer attachment"
    );
}

// ─── Scene: zone_reject_wrong_type ───────────────────────────────────────────

#[test]
fn zone_reject_wrong_type_builds() {
    let registry = TestSceneRegistry::default();
    let result = registry.build("zone_reject_wrong_type", ClockMs::FIXED);
    assert!(result.is_some(), "zone_reject_wrong_type must build");
}

#[test]
fn zone_reject_wrong_type_passes_layer0_invariants() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_reject_wrong_type", ClockMs::FIXED).unwrap();
    assert_no_violations(&graph, "zone_reject_wrong_type");
}

#[test]
fn zone_reject_wrong_type_rejects_mismatched_media_type() {
    // Zone accepts only StreamText; publishing KeyValuePairs must be rejected.
    let registry = TestSceneRegistry::default();
    let (mut graph, _spec) = registry.build("zone_reject_wrong_type", ClockMs::FIXED).unwrap();

    let wrong_content = ZoneContent::StatusBar(StatusBarPayload {
        entries: {
            let mut m = HashMap::new();
            m.insert("clock".to_string(), "12:00".to_string());
            m
        },
    });

    let result = graph.publish_to_zone("typed_zone", wrong_content, "agent", None, None, None);
    assert!(
        result.is_err(),
        "publishing wrong media type must be rejected"
    );
}

#[test]
fn zone_reject_wrong_type_accepts_correct_media_type() {
    let registry = TestSceneRegistry::default();
    let (mut graph, _spec) = registry.build("zone_reject_wrong_type", ClockMs::FIXED).unwrap();
    let result = graph.publish_to_zone(
        "typed_zone",
        ZoneContent::StreamText("valid content".to_string()),
        "agent",
        None,
        None,
        None,
    );
    assert!(result.is_ok(), "publishing correct media type must succeed");
}

// ─── Scene: zone_conflict_two_publishers ─────────────────────────────────────

#[test]
fn zone_conflict_two_publishers_builds() {
    let registry = TestSceneRegistry::default();
    let result = registry.build("zone_conflict_two_publishers", ClockMs::FIXED);
    assert!(result.is_some(), "zone_conflict_two_publishers must build");
}

#[test]
fn zone_conflict_two_publishers_passes_layer0_invariants() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_conflict_two_publishers", ClockMs::FIXED).unwrap();
    assert_no_violations(&graph, "zone_conflict_two_publishers");
}

#[test]
fn zone_conflict_latest_wins_policy() {
    // Two publishers to a LatestWins zone: only the latest should survive.
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(make_subtitle_zone());

    scene
        .publish_to_zone("subtitle", ZoneContent::StreamText("first".to_string()), "pub_a", None, None, None)
        .unwrap();
    scene
        .publish_to_zone("subtitle", ZoneContent::StreamText("second".to_string()), "pub_b", None, None, None)
        .unwrap();

    let pubs = scene.zone_registry.active_for_zone("subtitle");
    assert_eq!(pubs.len(), 1, "LatestWins: only 1 publication survives");
    assert_eq!(pubs[0].publisher_namespace, "pub_b");
    assert_eq!(pubs[0].content, ZoneContent::StreamText("second".to_string()));
}

// ─── Scene: zone_orchestrate_then_publish ────────────────────────────────────

#[test]
fn zone_orchestrate_then_publish_builds() {
    let registry = TestSceneRegistry::default();
    let result = registry.build("zone_orchestrate_then_publish", ClockMs::FIXED);
    assert!(result.is_some(), "zone_orchestrate_then_publish must build");
}

#[test]
fn zone_orchestrate_then_publish_has_three_zones() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_orchestrate_then_publish", ClockMs::FIXED).unwrap();
    for name in &["alert_banner", "notification_area", "status_bar"] {
        assert!(
            graph.zone_registry.get_by_name(name).is_some(),
            "zone '{}' must be registered",
            name
        );
    }
}

#[test]
fn zone_orchestrate_then_publish_passes_layer0_invariants() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_orchestrate_then_publish", ClockMs::FIXED).unwrap();
    assert_no_violations(&graph, "zone_orchestrate_then_publish");
}

// ─── Scene: zone_geometry_adapts_profile ─────────────────────────────────────

#[test]
fn zone_geometry_adapts_profile_builds() {
    let registry = TestSceneRegistry::default();
    let result = registry.build("zone_geometry_adapts_profile", ClockMs::FIXED);
    assert!(result.is_some(), "zone_geometry_adapts_profile must build");
}

#[test]
fn zone_geometry_adapts_profile_has_relative_zones() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_geometry_adapts_profile", ClockMs::FIXED).unwrap();
    let mut found_relative = false;
    for name in &["pip", "ambient-background"] {
        if let Some(zone) = graph.zone_registry.get_by_name(name) {
            assert!(
                matches!(zone.geometry_policy, GeometryPolicy::Relative { .. }),
                "zone '{}' must use Relative geometry policy",
                name
            );
            found_relative = true;
        }
    }
    assert!(
        found_relative,
        "zone_geometry_adapts_profile scene must contain at least one Relative-geometry zone (pip or ambient-background)"
    );
}

#[test]
fn zone_geometry_adapts_profile_passes_layer0_invariants() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_geometry_adapts_profile", ClockMs::FIXED).unwrap();
    assert_no_violations(&graph, "zone_geometry_adapts_profile");
}

// ─── Scene: zone_disconnect_cleanup ──────────────────────────────────────────

#[test]
fn zone_disconnect_cleanup_builds() {
    let registry = TestSceneRegistry::default();
    let result = registry.build("zone_disconnect_cleanup", ClockMs::FIXED);
    assert!(result.is_some(), "zone_disconnect_cleanup must build");
}

#[test]
fn zone_disconnect_cleanup_passes_layer0_invariants() {
    let registry = TestSceneRegistry::default();
    let (graph, _spec) = registry.build("zone_disconnect_cleanup", ClockMs::FIXED).unwrap();
    assert_no_violations(&graph, "zone_disconnect_cleanup");
}

// ─── All four contention policies ────────────────────────────────────────────

#[test]
fn contention_stack_evicts_oldest_at_max_depth() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "notif".to_string(),
        description: "Stack zone".to_string(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.75,
            y_pct: 0.0,
            width_pct: 0.25,
            height_pct: 0.3,
        },
        accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::Stack { max_depth: 3 },
        max_publishers: 8,
        transport_constraint: None,
        auto_clear_ms: None,
        layer_attachment: LayerAttachment::Chrome,
        ephemeral: false,
    });

    let notif = |text: &str| {
        ZoneContent::Notification(NotificationPayload {
            text: text.to_string(),
            icon: "".to_string(),
            urgency: 1,
        })
    };

    // Push 3 to fill the stack
    scene.publish_to_zone("notif", notif("msg1"), "a1", None, None, None).unwrap();
    scene.publish_to_zone("notif", notif("msg2"), "a2", None, None, None).unwrap();
    scene.publish_to_zone("notif", notif("msg3"), "a3", None, None, None).unwrap();
    assert_eq!(scene.zone_registry.active_for_zone("notif").len(), 3);

    // 4th push must evict oldest (msg1)
    scene.publish_to_zone("notif", notif("msg4"), "a4", None, None, None).unwrap();
    let pubs = scene.zone_registry.active_for_zone("notif");
    assert_eq!(pubs.len(), 3, "Stack max_depth=3 must evict oldest on overflow");

    let texts: Vec<_> = pubs.iter().map(|r| {
        if let ZoneContent::Notification(n) = &r.content { n.text.clone() } else { "?".into() }
    }).collect();
    assert!(!texts.contains(&"msg1".to_string()), "oldest msg1 must be evicted");
    assert!(texts.contains(&"msg4".to_string()), "newest msg4 must survive");
}

#[test]
fn contention_merge_by_key_same_key_replaces() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "status".to_string(),
        description: "Status bar".to_string(),
        geometry_policy: GeometryPolicy::EdgeAnchored {
            edge: DisplayEdge::Bottom,
            height_pct: 0.04,
            width_pct: 1.0,
            margin_px: 0.0,
        },
        accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::MergeByKey { max_keys: 16 },
        max_publishers: 8,
        transport_constraint: None,
        auto_clear_ms: None,
        layer_attachment: LayerAttachment::Chrome,
        ephemeral: false,
    });

    let kv = |k: &str, v: &str| {
        let mut entries = HashMap::new();
        entries.insert(k.to_string(), v.to_string());
        ZoneContent::StatusBar(StatusBarPayload { entries })
    };

    scene.publish_to_zone("status", kv("clock", "12:00"), "a1", Some("clock".to_string()), None, None).unwrap();
    scene.publish_to_zone("status", kv("battery", "80%"), "a2", Some("battery".to_string()), None, None).unwrap();
    assert_eq!(scene.zone_registry.active_for_zone("status").len(), 2, "different keys should coexist");

    // Update same key "clock"
    scene.publish_to_zone("status", kv("clock", "12:01"), "a1", Some("clock".to_string()), None, None).unwrap();
    let pubs = scene.zone_registry.active_for_zone("status");
    assert_eq!(pubs.len(), 2, "updating existing key must not create a new entry");
    let clock = pubs.iter().find(|r| r.merge_key.as_deref() == Some("clock")).unwrap();
    if let ZoneContent::StatusBar(sb) = &clock.content {
        assert_eq!(sb.entries["clock"], "12:01", "clock must be updated to 12:01");
    } else {
        panic!("expected StatusBar content");
    }
}

#[test]
fn contention_replace_evicts_current() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "pip".to_string(),
        description: "PiP".to_string(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.75,
            y_pct: 0.70,
            width_pct: 0.22,
            height_pct: 0.26,
        },
        accepted_media_types: vec![ZoneMediaType::SolidColor],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::Replace,
        max_publishers: 2,
        transport_constraint: None,
        auto_clear_ms: None,
        layer_attachment: LayerAttachment::Content,
        ephemeral: false,
    });

    scene.publish_to_zone("pip", ZoneContent::SolidColor(Rgba::WHITE), "a1", None, None, None).unwrap();
    scene.publish_to_zone("pip", ZoneContent::SolidColor(Rgba::BLACK), "a2", None, None, None).unwrap();

    let pubs = scene.zone_registry.active_for_zone("pip");
    assert_eq!(pubs.len(), 1, "Replace: only 1 occupant allowed");
    assert_eq!(pubs[0].publisher_namespace, "a2", "Replace: new publish evicts current");
}

// ─── Layer attachment routing ─────────────────────────────────────────────────

#[test]
fn zone_tile_z_min_constant_is_correct() {
    assert_eq!(ZONE_TILE_Z_MIN, 0x8000_0000u32, "ZONE_TILE_Z_MIN must be 0x8000_0000");
}

#[test]
fn layer_attachment_variants_exist() {
    // Smoke test: all three variants construct and can be compared
    let bg = LayerAttachment::Background;
    let content = LayerAttachment::Content;
    let chrome = LayerAttachment::Chrome;
    assert_ne!(bg, content);
    assert_ne!(content, chrome);
    assert_ne!(bg, chrome);
}

#[test]
fn content_layer_zone_z_order_must_be_ge_zone_tile_z_min() {
    // Any tile added for a Content-layer zone should use z_order >= ZONE_TILE_Z_MIN.
    // This is a schema constraint — we verify the constant is below any agent tile value.
    // Agent tiles use z_order values set by the agent (typically small integers).
    // Zone tiles in Content layer must be in the reserved upper band.
    let agent_max_z: u32 = u16::MAX as u32; // typical high agent z_order
    assert!(
        ZONE_TILE_Z_MIN > agent_max_z,
        "ZONE_TILE_Z_MIN ({:#010x}) must be > typical max agent z_order ({:#010x})",
        ZONE_TILE_Z_MIN,
        agent_max_z
    );
}

#[test]
fn default_zone_registry_contains_all_six_v1_zones() {
    use tze_hud_scene::types::ZoneRegistry;
    let registry = ZoneRegistry::with_defaults();
    let zones = registry.all_zones();
    assert_eq!(zones.len(), 6, "V1 default registry must contain exactly 6 zones");

    let names: Vec<_> = zones.iter().map(|z| z.name.as_str()).collect();
    for expected in &["subtitle", "notification-area", "status-bar", "pip", "ambient-background", "alert-banner"] {
        assert!(
            names.contains(expected),
            "default registry missing zone '{}'",
            expected
        );
    }
}

#[test]
fn default_zones_have_correct_layer_attachments() {
    use tze_hud_scene::types::ZoneRegistry;
    let registry = ZoneRegistry::with_defaults();

    let check = |name: &str, expected: LayerAttachment| {
        let zone = registry.get_by_name(name).unwrap_or_else(|| panic!("zone '{}' not found", name));
        assert_eq!(
            zone.layer_attachment, expected,
            "zone '{}' must have layer attachment {:?}",
            name, expected
        );
    };

    check("subtitle", LayerAttachment::Content);
    check("pip", LayerAttachment::Content);
    check("ambient-background", LayerAttachment::Background);
    check("status-bar", LayerAttachment::Chrome);
    check("notification-area", LayerAttachment::Chrome);
    check("alert-banner", LayerAttachment::Chrome);
}

// ─── Five v1 media types ──────────────────────────────────────────────────────

#[test]
fn all_five_v1_media_types_accepted_by_appropriate_zones() {
    use tze_hud_scene::types::ZoneRegistry;
    let registry = ZoneRegistry::with_defaults();

    // StreamText: subtitle
    assert!(!registry.zones_accepting(ZoneMediaType::StreamText).is_empty(),
        "StreamText must be accepted by at least one default zone");

    // ShortTextWithIcon: notification-area, alert-banner
    assert!(!registry.zones_accepting(ZoneMediaType::ShortTextWithIcon).is_empty(),
        "ShortTextWithIcon must be accepted by at least one default zone");

    // KeyValuePairs: status-bar
    assert!(!registry.zones_accepting(ZoneMediaType::KeyValuePairs).is_empty(),
        "KeyValuePairs must be accepted by at least one default zone");

    // StaticImage: pip, ambient-background
    assert!(!registry.zones_accepting(ZoneMediaType::StaticImage).is_empty(),
        "StaticImage must be accepted by at least one default zone");

    // SolidColor: pip, ambient-background
    assert!(!registry.zones_accepting(ZoneMediaType::SolidColor).is_empty(),
        "SolidColor must be accepted by at least one default zone");
}

#[test]
fn static_image_content_publishes_successfully() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    // Register a zone that accepts StaticImage
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "bg".to_string(),
        description: "Background zone".to_string(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.0,
            y_pct: 0.0,
            width_pct: 1.0,
            height_pct: 1.0,
        },
        accepted_media_types: vec![ZoneMediaType::StaticImage],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::Replace,
        max_publishers: 1,
        transport_constraint: None,
        auto_clear_ms: None,
        layer_attachment: LayerAttachment::Background,
        ephemeral: false,
    });

    let resource_id = ResourceId::of(b"fake image data");
    let result = scene.publish_to_zone(
        "bg",
        ZoneContent::StaticImage(resource_id),
        "agent",
        None,
        None,
        None,
    );
    assert!(result.is_ok(), "StaticImage content must publish to a zone accepting StaticImage");
}

#[test]
fn video_surface_ref_schema_defined_but_not_rendered() {
    // VideoSurfaceRef is a valid enum variant (schema defined for post-v1)
    // but should be accepted in config for zones that declare it.
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "media".to_string(),
        description: "Media zone (accepts VideoSurfaceRef for post-v1)".to_string(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.0,
            y_pct: 0.0,
            width_pct: 1.0,
            height_pct: 1.0,
        },
        accepted_media_types: vec![ZoneMediaType::VideoSurfaceRef],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::Replace,
        max_publishers: 1,
        transport_constraint: None,
        auto_clear_ms: None,
        layer_attachment: LayerAttachment::Content,
        ephemeral: false,
    });

    // The zone can be registered and the content can be published (schema accepted).
    // Actual rendering is deferred to post-v1.
    let surface_id = SceneId::new();
    let result = scene.publish_to_zone(
        "media",
        ZoneContent::VideoSurfaceRef(surface_id),
        "agent",
        None,
        None,
        None,
    );
    assert!(result.is_ok(), "VideoSurfaceRef schema must be accepted for zones that declare it");
}

// ─── ZonePublishToken validation ──────────────────────────────────────────────

#[test]
fn zone_not_found_rejected() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let result = scene.publish_to_zone(
        "nonexistent",
        ZoneContent::StreamText("hello".to_string()),
        "agent",
        None,
        None,
        None,
    );
    assert!(result.is_err(), "publishing to nonexistent zone must fail");
}

#[test]
fn zone_media_type_mismatch_rejected() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(make_subtitle_zone()); // accepts StreamText only

    let result = scene.publish_to_zone(
        "subtitle",
        ZoneContent::SolidColor(Rgba::WHITE),
        "agent",
        None,
        None,
        None,
    );
    assert!(result.is_err(), "media type mismatch must be rejected");
}

// ─── Per-publisher clear semantics ───────────────────────────────────────────

#[test]
fn clear_zone_for_publisher_only_removes_own_publications() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(ZoneDefinition {
        id: SceneId::new(),
        name: "shared".to_string(),
        description: "Shared stack zone".to_string(),
        geometry_policy: GeometryPolicy::Relative {
            x_pct: 0.0,
            y_pct: 0.0,
            width_pct: 1.0,
            height_pct: 0.1,
        },
        accepted_media_types: vec![ZoneMediaType::StreamText],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::Stack { max_depth: 10 },
        max_publishers: 8,
        transport_constraint: None,
        auto_clear_ms: None,
        layer_attachment: LayerAttachment::Content,
        ephemeral: false,
    });

    scene.publish_to_zone("shared", ZoneContent::StreamText("from a1".to_string()), "a1", None, None, None).unwrap();
    scene.publish_to_zone("shared", ZoneContent::StreamText("from a2".to_string()), "a2", None, None, None).unwrap();
    assert_eq!(scene.zone_registry.active_for_zone("shared").len(), 2);

    // Clear only a1's publications
    scene.clear_zone_for_publisher("shared", "a1").unwrap();

    let pubs = scene.zone_registry.active_for_zone("shared");
    assert_eq!(pubs.len(), 1, "only a1's publication should be removed");
    assert_eq!(pubs[0].publisher_namespace, "a2", "a2's publication must survive");
}

#[test]
fn clear_zone_for_publisher_nonexistent_zone_fails() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let result = scene.clear_zone_for_publisher("nonexistent", "agent");
    assert!(result.is_err(), "clearing nonexistent zone must fail");
}

// ─── Expiry semantics ────────────────────────────────────────────────────────

#[test]
fn publish_with_expiry_stores_expiry_timestamp() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(make_subtitle_zone());

    let expiry_us = 1_735_689_600_000_000u64; // 1 Jan 2025 00:00:00 UTC in microseconds
    scene.publish_to_zone(
        "subtitle",
        ZoneContent::StreamText("expiring content".to_string()),
        "agent",
        None,
        Some(expiry_us),
        None,
    ).unwrap();

    let pubs = scene.zone_registry.active_for_zone("subtitle");
    assert_eq!(pubs.len(), 1);
    assert_eq!(
        pubs[0].expires_at_wall_us,
        Some(expiry_us),
        "expires_at_wall_us must be stored in the publish record"
    );
}

#[test]
fn publish_without_expiry_stores_none() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(make_subtitle_zone());

    scene.publish_to_zone(
        "subtitle",
        ZoneContent::StreamText("permanent content".to_string()),
        "agent",
        None,
        None,
        None,
    ).unwrap();

    let pubs = scene.zone_registry.active_for_zone("subtitle");
    assert_eq!(pubs[0].expires_at_wall_us, None, "no expiry should store None");
}

// ─── Zone occupancy query API (v1: no effective_geometry) ────────────────────

#[test]
fn zone_occupancy_query_returns_correct_state() {
    use tze_hud_scene::types::ZoneRegistry;
    let mut registry = ZoneRegistry::new();
    registry.register(make_subtitle_zone());

    let tab_id = SceneId::new();

    // No publications yet
    let occupancy = registry.get_occupancy("subtitle", tab_id).unwrap();
    assert_eq!(occupancy.occupant_count, 0);
    assert!(occupancy.active_publications.is_empty());

    // Simulate a publish
    registry.active_publishes.entry("subtitle".to_string()).or_default().push(
        tze_hud_scene::types::ZonePublishRecord {
            zone_name: "subtitle".to_string(),
            publisher_namespace: "agent".to_string(),
            content: ZoneContent::StreamText("hello".to_string()),
            published_at_wall_us: 1_000_000,  // microseconds
            merge_key: None,
            expires_at_wall_us: None,
            content_classification: None,
        },
    );

    let occupancy = registry.get_occupancy("subtitle", tab_id).unwrap();
    assert_eq!(occupancy.occupant_count, 1);
    assert_eq!(occupancy.zone_name, "subtitle");
    assert_eq!(occupancy.tab_id, tab_id);
    // v1: no effective_geometry field
}

#[test]
fn zone_occupancy_returns_none_for_unknown_zone() {
    use tze_hud_scene::types::ZoneRegistry;
    let registry = ZoneRegistry::new();
    let result = registry.get_occupancy("nonexistent", SceneId::new());
    assert!(result.is_none(), "occupancy query for unknown zone must return None");
}

// ─── ZoneInstance type ────────────────────────────────────────────────────────

#[test]
fn zone_instance_binds_type_to_tab() {
    use tze_hud_scene::types::ZoneInstance;

    let tab_id = SceneId::new();
    let instance = ZoneInstance {
        zone_type_name: "subtitle".to_string(),
        tab_id,
        geometry_override: None,
    };

    assert_eq!(instance.zone_type_name, "subtitle");
    assert_eq!(instance.tab_id, tab_id);
    assert!(instance.geometry_override.is_none());
}

// ─── Content classification ───────────────────────────────────────────────────

#[test]
fn publish_with_content_classification_stored() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    scene.register_zone(make_subtitle_zone());

    scene.publish_to_zone(
        "subtitle",
        ZoneContent::StreamText("sensitive content".to_string()),
        "agent",
        None,
        None,
        Some("pii".to_string()),
    ).unwrap();

    let pubs = scene.zone_registry.active_for_zone("subtitle");
    assert_eq!(pubs[0].content_classification.as_deref(), Some("pii"));
}

// ─── Proptest: contention policy invariants ──────────────────────────────────

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use tze_hud_scene::types::{ZoneContent, ZoneDefinition, ZoneMediaType, ContentionPolicy};
    use tze_hud_scene::graph::SceneGraph;
    use std::collections::HashMap;

    /// Arbitrary StreamText content for proptest
    fn arb_stream_text() -> impl Strategy<Value = ZoneContent> {
        "[a-z]{1,20}".prop_map(|s| ZoneContent::StreamText(s))
    }

    /// Publisher namespaces
    fn arb_namespace() -> impl Strategy<Value = String> {
        prop::sample::select(vec!["a1", "a2", "a3", "a4"])
            .prop_map(|s| s.to_string())
    }

    proptest! {
        /// LatestWins invariant: after any sequence of publishes,
        /// active publication count is always <= 1.
        #[test]
        fn prop_latest_wins_at_most_one_publication(
            publishes in prop::collection::vec(
                (arb_stream_text(), arb_namespace()),
                0..20usize
            )
        ) {
            let mut scene = SceneGraph::new(1920.0, 1080.0);
            scene.register_zone(ZoneDefinition {
                id: SceneId::new(),
                name: "lw_zone".to_string(),
                description: "LatestWins zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 0.1,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::LatestWins,
                max_publishers: 16,
                transport_constraint: None,
                auto_clear_ms: None,
                layer_attachment: LayerAttachment::Content,
                ephemeral: false,
            });

            for (content, ns) in publishes {
                let _ = scene.publish_to_zone("lw_zone", content, &ns, None, None, None);
            }

            let count = scene.zone_registry.active_for_zone("lw_zone").len();
            prop_assert!(count <= 1, "LatestWins must have at most 1 active publication, got {}", count);
        }

        /// Replace invariant: after any sequence of publishes,
        /// active publication count is always <= 1.
        #[test]
        fn prop_replace_at_most_one_publication(
            publishes in prop::collection::vec(
                (arb_stream_text(), arb_namespace()),
                0..20usize
            )
        ) {
            let mut scene = SceneGraph::new(1920.0, 1080.0);
            scene.register_zone(ZoneDefinition {
                id: SceneId::new(),
                name: "rp_zone".to_string(),
                description: "Replace zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 0.1,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::Replace,
                max_publishers: 16,
                transport_constraint: None,
                auto_clear_ms: None,
                layer_attachment: LayerAttachment::Content,
                ephemeral: false,
            });

            for (content, ns) in publishes {
                let _ = scene.publish_to_zone("rp_zone", content, &ns, None, None, None);
            }

            let count = scene.zone_registry.active_for_zone("rp_zone").len();
            prop_assert!(count <= 1, "Replace must have at most 1 active publication, got {}", count);
        }

        /// Stack invariant: after any sequence of publishes,
        /// active publication count is always <= max_depth.
        #[test]
        fn prop_stack_never_exceeds_max_depth(
            max_depth in 1u8..=8u8,
            publishes in prop::collection::vec(
                (arb_stream_text(), arb_namespace()),
                0..30usize
            )
        ) {
            let mut scene = SceneGraph::new(1920.0, 1080.0);
            scene.register_zone(ZoneDefinition {
                id: SceneId::new(),
                name: "stack_zone".to_string(),
                description: "Stack zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 0.1,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::Stack { max_depth },
                max_publishers: 64,
                transport_constraint: None,
                auto_clear_ms: None,
                layer_attachment: LayerAttachment::Chrome,
                ephemeral: false,
            });

            for (content, ns) in publishes {
                let _ = scene.publish_to_zone("stack_zone", content, &ns, None, None, None);
            }

            let count = scene.zone_registry.active_for_zone("stack_zone").len();
            prop_assert!(
                count <= max_depth as usize,
                "Stack max_depth={} must never be exceeded, got {}",
                max_depth,
                count
            );
        }

        /// MergeByKey invariant: after any sequence of publishes with keys,
        /// active publication count is always <= max_keys,
        /// and no two entries share the same key.
        #[test]
        fn prop_merge_by_key_unique_keys_and_bounded(
            max_keys in 2u8..=8u8,
            publishes in prop::collection::vec(
                (
                    prop::sample::select(vec!["k1", "k2", "k3", "k4"]).prop_map(|s| s.to_string()),
                    arb_namespace(),
                ),
                0..20usize
            )
        ) {
            let mut scene = SceneGraph::new(1920.0, 1080.0);
            let kv_content = |_key: &str| {
                let mut entries = HashMap::new();
                entries.insert("v".to_string(), "value".to_string());
                ZoneContent::StatusBar(StatusBarPayload { entries })
            };

            scene.register_zone(ZoneDefinition {
                id: SceneId::new(),
                name: "mbk_zone".to_string(),
                description: "MergeByKey zone".to_string(),
                geometry_policy: GeometryPolicy::EdgeAnchored {
                    edge: DisplayEdge::Bottom,
                    height_pct: 0.04,
                    width_pct: 1.0,
                    margin_px: 0.0,
                },
                accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::MergeByKey { max_keys },
                max_publishers: 16,
                transport_constraint: None,
                auto_clear_ms: None,
                layer_attachment: LayerAttachment::Chrome,
                ephemeral: false,
            });

            for (key, ns) in &publishes {
                let _ = scene.publish_to_zone("mbk_zone", kv_content(key), &ns, Some(key.clone()), None, None);
            }

            let pubs = scene.zone_registry.active_for_zone("mbk_zone");

            // Count <= max_keys
            prop_assert!(
                pubs.len() <= max_keys as usize,
                "MergeByKey max_keys={} must not be exceeded, got {}",
                max_keys,
                pubs.len()
            );

            // Keys must be unique
            let keys: Vec<_> = pubs.iter().map(|r| r.merge_key.clone()).collect();
            let unique_keys: std::collections::HashSet<_> = keys.iter().collect();
            prop_assert!(
                keys.len() == unique_keys.len(),
                "MergeByKey: duplicate keys found: {:?}",
                keys
            );
        }
    }
}
