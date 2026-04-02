//! # MCP Integration Tests: publish_to_zone — Status-Bar Zone
//!
//! Exercises `publish_to_zone` for the `status-bar` zone type via the MCP
//! JSON-RPC tool handler.  Each test exercises one of the four scenarios from
//! openspec/changes/exemplar-status-bar/specs/exemplar-status-bar/spec.md
//! §Requirement: MCP Integration Test Scenarios.
//!
//! ## Zone configuration
//! The status-bar zone uses:
//! - `accepted_media_types: [KeyValuePairs]` (maps to `ZoneContent::StatusBar`)
//! - `contention_policy: MergeByKey { max_keys: 32 }`
//! - `layer_attachment: Chrome` (always-on-top)
//!
//! ## Four scenarios
//! 1. Single key publish — success + exactly one active publication
//! 2. Multi-agent publish — 3 namespaces, different merge_keys, all coexist
//! 3. Key update — re-publish same merge_key, value updated, count stable
//! 4. Key removal via empty value — publication stored, empty value preserved

use serde_json::json;
use tze_hud_mcp::tools::handle_publish_to_zone;
use tze_hud_scene::{
    graph::SceneGraph,
    types::{
        ContentionPolicy, GeometryPolicy, LayerAttachment, RenderingPolicy, SceneId, ZoneContent,
        ZoneDefinition, ZoneMediaType,
    },
};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Build a scene with the canonical status-bar zone.
///
/// Geometry: full-width 4%-height strip at the bottom (EdgeAnchored).
/// Contention: MergeByKey with max_keys=32 (spec §Merge-by-Key Contention).
/// Layer: Chrome — always rendered above content tiles.
fn scene_with_status_bar() -> (SceneGraph, String) {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let zone_name = "status-bar".to_string();
    scene.zone_registry.zones.insert(
        zone_name.clone(),
        ZoneDefinition {
            id: SceneId::new(),
            name: zone_name.clone(),
            description: "Polished status bar exemplar (merge-by-key, chrome layer)".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.96,
                width_pct: 1.0,
                height_pct: 0.04,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 32 },
            max_publishers: 64,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        },
    );
    (scene, zone_name)
}

// ─── Test 1: Single key publish ──────────────────────────────────────────────

/// **Scenario: MCP publish single status key**
///
/// WHEN an MCP client calls `publish_to_zone` with a `status_bar` payload
/// carrying `merge_key: "weather"` and `entries: {"weather": "72F"}`,
/// THEN the call SHALL succeed AND the status-bar zone SHALL have exactly
/// one active publication with merge_key `"weather"`.
#[test]
fn test_status_bar_single_key_publish() {
    let (mut scene, zone) = scene_with_status_bar();

    let result = handle_publish_to_zone(
        json!({
            "zone_name": zone,
            "namespace": "agent-weather",
            "content": {
                "type": "status_bar",
                "entries": {"weather": "72F"}
            },
            "merge_key": "weather"
        }),
        &mut scene,
    )
    .expect("single key publish must succeed");

    // Response must echo the zone name and merge_key.
    assert_eq!(result.zone_name, zone);
    assert_eq!(
        result.merge_key.as_deref(),
        Some("weather"),
        "response must echo the merge_key"
    );

    // Zone must have exactly one active publication.
    let publishes = scene
        .zone_registry
        .active_publishes
        .get(&zone)
        .expect("zone must have active publications after publish");
    assert_eq!(
        publishes.len(),
        1,
        "exactly one active publication must exist after single key publish"
    );

    // The publication must carry the weather entry.
    let record = &publishes[0];
    assert_eq!(
        record.merge_key.as_deref(),
        Some("weather"),
        "publication must carry merge_key 'weather'"
    );
    assert!(
        matches!(
            &record.content,
            ZoneContent::StatusBar(p) if p.entries.get("weather").map(String::as_str) == Some("72F")
        ),
        "publication must carry the correct 'weather' entry value"
    );
}

// ─── Test 2: Multi-agent publish ─────────────────────────────────────────────

/// **Scenario: MCP publish from multiple agents with different keys**
///
/// WHEN three MCP clients (different namespaces) each publish a distinct
/// merge_key to the status-bar zone,
/// THEN the zone SHALL have exactly 3 active publications AND each
/// publication's merge_key SHALL correspond to its entry key.
#[test]
fn test_status_bar_multi_agent_publish() {
    let (mut scene, zone) = scene_with_status_bar();

    // Agent A: weather
    handle_publish_to_zone(
        json!({
            "zone_name": zone,
            "namespace": "agent-weather",
            "content": {"type": "status_bar", "entries": {"weather": "72F"}},
            "merge_key": "weather"
        }),
        &mut scene,
    )
    .expect("agent-weather publish must succeed");

    // Agent B: battery
    handle_publish_to_zone(
        json!({
            "zone_name": zone,
            "namespace": "agent-power",
            "content": {"type": "status_bar", "entries": {"battery": "85%"}},
            "merge_key": "battery"
        }),
        &mut scene,
    )
    .expect("agent-power publish must succeed");

    // Agent C: time
    handle_publish_to_zone(
        json!({
            "zone_name": zone,
            "namespace": "agent-clock",
            "content": {"type": "status_bar", "entries": {"time": "3:42 PM"}},
            "merge_key": "time"
        }),
        &mut scene,
    )
    .expect("agent-clock publish must succeed");

    // All three publications must coexist.
    let publishes = scene
        .zone_registry
        .active_publishes
        .get(&zone)
        .expect("zone must have publications");
    assert_eq!(
        publishes.len(),
        3,
        "three agents with different merge_keys must coexist (got {})",
        publishes.len()
    );

    // Each merge_key must be present exactly once.
    for key in ["weather", "battery", "time"] {
        assert!(
            publishes
                .iter()
                .any(|r| r.merge_key.as_deref() == Some(key)),
            "merge_key '{key}' must be present in active publications"
        );
    }
}

// ─── Test 3: Key update ──────────────────────────────────────────────────────

/// **Scenario: MCP key update preserves other keys**
///
/// WHEN three agents have active publications AND agent A re-publishes the
/// same merge_key with a new value,
/// THEN the zone SHALL still have exactly 3 active publications AND the
/// updated key SHALL show the new value while the others are unchanged.
#[test]
fn test_status_bar_key_update() {
    let (mut scene, zone) = scene_with_status_bar();

    // Publish three keys from three agents.
    for (ns, key, val) in [
        ("agent-weather", "weather", "72F"),
        ("agent-power", "battery", "85%"),
        ("agent-clock", "time", "3:42 PM"),
    ] {
        handle_publish_to_zone(
            json!({
                "zone_name": zone,
                "namespace": ns,
                "content": {"type": "status_bar", "entries": {key: val}},
                "merge_key": key
            }),
            &mut scene,
        )
        .unwrap_or_else(|e| panic!("initial publish for key '{key}' must succeed: {e:?}"));
    }

    // Verify initial state: 3 publications, weather = "72F".
    {
        let pubs = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(pubs.len(), 3, "must have 3 publications before update");
        let weather = pubs
            .iter()
            .find(|r| r.merge_key.as_deref() == Some("weather"))
            .expect("weather key must be present before update");
        assert!(
            matches!(&weather.content, ZoneContent::StatusBar(p) if p.entries.get("weather").map(String::as_str) == Some("72F")),
            "weather must show '72F' before update"
        );
    }

    // Agent A updates weather to "75F".
    handle_publish_to_zone(
        json!({
            "zone_name": zone,
            "namespace": "agent-weather",
            "content": {"type": "status_bar", "entries": {"weather": "75F"}},
            "merge_key": "weather"
        }),
        &mut scene,
    )
    .expect("weather key update must succeed");

    // Verify post-update state.
    let pubs = scene
        .zone_registry
        .active_publishes
        .get(&zone)
        .expect("zone must still have publications after update");

    // Count must stay at 3 (key replaced in-place, not appended).
    assert_eq!(
        pubs.len(),
        3,
        "publication count must remain 3 after key update (got {})",
        pubs.len()
    );

    // Weather must now show "75F".
    let weather = pubs
        .iter()
        .find(|r| r.merge_key.as_deref() == Some("weather"))
        .expect("weather key must still be present after update");
    assert!(
        matches!(&weather.content, ZoneContent::StatusBar(p) if p.entries.get("weather").map(String::as_str) == Some("75F")),
        "weather must show '75F' after update"
    );

    // Battery and time must be unchanged.
    let battery = pubs
        .iter()
        .find(|r| r.merge_key.as_deref() == Some("battery"))
        .expect("battery key must still be present");
    assert!(
        matches!(&battery.content, ZoneContent::StatusBar(p) if p.entries.get("battery").map(String::as_str) == Some("85%")),
        "battery must be unchanged after weather update"
    );

    let time_pub = pubs
        .iter()
        .find(|r| r.merge_key.as_deref() == Some("time"))
        .expect("time key must still be present");
    assert!(
        matches!(&time_pub.content, ZoneContent::StatusBar(p) if p.entries.get("time").map(String::as_str) == Some("3:42 PM")),
        "time must be unchanged after weather update"
    );
}

// ─── Test 4: Key removal via empty value ─────────────────────────────────────

/// **Scenario: MCP key removal via empty value**
///
/// WHEN agent A publishes a `status_bar` payload with the same merge_key but
/// an empty string value (`"weather": ""`),
/// THEN the publication SHALL be stored (merge-by-key replacement) AND the
/// entry's value SHALL be the empty string (compositor convention: skip
/// rendering entries with empty values).
#[test]
fn test_status_bar_key_removal_via_empty_value() {
    let (mut scene, zone) = scene_with_status_bar();

    // Publish an initial weather entry.
    handle_publish_to_zone(
        json!({
            "zone_name": zone,
            "namespace": "agent-weather",
            "content": {"type": "status_bar", "entries": {"weather": "72F"}},
            "merge_key": "weather"
        }),
        &mut scene,
    )
    .expect("initial weather publish must succeed");

    // Verify initial state.
    {
        let pubs = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(pubs.len(), 1, "one publication before removal");
    }

    // Re-publish with empty value — the spec says the compositor will skip
    // rendering entries with empty values; the scene-graph stores the record.
    handle_publish_to_zone(
        json!({
            "zone_name": zone,
            "namespace": "agent-weather",
            "content": {"type": "status_bar", "entries": {"weather": ""}},
            "merge_key": "weather"
        }),
        &mut scene,
    )
    .expect("empty-value publish must succeed (merge-by-key replacement)");

    // Publication must still exist (MergeByKey replacement, not deletion).
    let pubs = scene
        .zone_registry
        .active_publishes
        .get(&zone)
        .expect("zone must have the replaced publication");
    assert_eq!(
        pubs.len(),
        1,
        "must still have exactly one publication after empty-value replace (got {})",
        pubs.len()
    );

    // The stored value must be the empty string.
    let record = &pubs[0];
    assert_eq!(
        record.merge_key.as_deref(),
        Some("weather"),
        "merge_key must still be 'weather'"
    );
    assert!(
        matches!(
            &record.content,
            ZoneContent::StatusBar(p) if p.entries.get("weather").map(String::as_str) == Some("")
        ),
        "stored entry value must be the empty string (compositor skips rendering)"
    );
}
