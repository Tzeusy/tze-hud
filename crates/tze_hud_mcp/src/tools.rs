//! MCP tool implementations.
//!
//! Each function takes `params: serde_json::Value` and a mutable reference to
//! the shared scene state, and returns a [`McpResult`] with a serializable
//! response value.
//!
//! Tool naming follows the issue spec:
//! - `create_tab`      → `handle_create_tab`
//! - `create_tile`     → `handle_create_tile`
//! - `set_content`     → `handle_set_content`
//! - `dismiss`         → `handle_dismiss`
//! - `publish_to_zone` → `handle_publish_to_zone`
//! - `list_zones`      → `handle_list_zones`
//! - `list_scene`      → `handle_list_scene`

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tze_hud_scene::{
    graph::SceneGraph,
    types::{
        Capability, FontFamily, Node, NodeData, Rect, Rgba, SceneId, TextAlign,
        TextMarkdownNode, TextOverflow, ZoneContent,
    },
};
use crate::{error::McpError, types::McpResult};

// ─── create_tab ─────────────────────────────────────────────────────────────

/// Parameters for `create_tab`.
#[derive(Debug, Deserialize)]
pub struct CreateTabParams {
    /// Human-readable name for the tab.
    pub name: String,
    /// Display order (must be unique across tabs). Defaults to next available.
    #[serde(default)]
    pub display_order: Option<u32>,
}

/// Response from `create_tab`.
#[derive(Debug, Serialize)]
pub struct CreateTabResult {
    /// The UUID of the newly created tab.
    pub tab_id: String,
    /// The name given to the tab.
    pub name: String,
    /// The display order assigned to this tab.
    pub display_order: u32,
}

/// Create a new tab in the scene.
///
/// If `display_order` is omitted, the next available order (max + 1) is used.
///
/// # Errors
/// - `invalid_params` if `name` is empty.
/// - `scene_error` if `display_order` is already taken.
pub fn handle_create_tab(params: Value, scene: &mut SceneGraph) -> McpResult<CreateTabResult> {
    let p: CreateTabParams = parse_params(params)?;

    if p.name.trim().is_empty() {
        return Err(McpError::InvalidParams("name must be non-empty".to_string()));
    }

    let order = p.display_order.unwrap_or_else(|| {
        scene
            .tabs
            .values()
            .map(|t| t.display_order)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0)
    });

    let tab_id = scene.create_tab(&p.name, order)?;

    Ok(CreateTabResult {
        tab_id: tab_id.to_string(),
        name: p.name,
        display_order: order,
    })
}

// ─── create_tile ────────────────────────────────────────────────────────────

/// Parameters for `create_tile`.
#[derive(Debug, Deserialize)]
pub struct CreateTileParams {
    /// ID of the tab to place the tile in. If omitted, uses the active tab.
    pub tab_id: Option<String>,
    /// Namespace (agent identity) for the tile. Used as the lease namespace.
    pub namespace: String,
    /// Bounds: x, y, width, height in display pixels.
    pub bounds: BoundsParams,
    /// Z-order (front = higher). Defaults to 1.
    #[serde(default = "default_z_order")]
    pub z_order: u32,
    /// Lease TTL in milliseconds. Defaults to 60 000 (1 minute).
    #[serde(default = "default_ttl_ms")]
    pub ttl_ms: u64,
}

/// Bounds as a plain JSON sub-object.
#[derive(Debug, Deserialize)]
pub struct BoundsParams {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

fn default_z_order() -> u32 {
    1
}

fn default_ttl_ms() -> u64 {
    60_000
}

/// Response from `create_tile`.
#[derive(Debug, Serialize)]
pub struct CreateTileResult {
    /// UUID of the newly created tile.
    pub tile_id: String,
    /// UUID of the lease granted to this tile.
    pub lease_id: String,
    /// The tab this tile belongs to.
    pub tab_id: String,
    /// Namespace under which the lease was granted.
    pub namespace: String,
}

/// Create a tile within a tab.
///
/// Automatically grants a lease for the tile with `CreateTile`, `UpdateTile`,
/// `CreateNode`, and `UpdateNode` capabilities.
///
/// # Errors
/// - `invalid_params` if namespace is empty or bounds are invalid.
/// - `no_active_tab` if `tab_id` is omitted and no tab is active.
/// - `invalid_id` if `tab_id` is provided but not a valid UUID.
/// - `scene_error` if the tab does not exist.
pub fn handle_create_tile(params: Value, scene: &mut SceneGraph) -> McpResult<CreateTileResult> {
    let p: CreateTileParams = parse_params(params)?;

    if p.namespace.trim().is_empty() {
        return Err(McpError::InvalidParams("namespace must be non-empty".to_string()));
    }

    if p.bounds.width <= 0.0 || p.bounds.height <= 0.0 {
        return Err(McpError::InvalidParams(
            "bounds.width and bounds.height must be > 0".to_string(),
        ));
    }

    // Resolve tab ID
    let tab_id = match p.tab_id {
        Some(ref s) => parse_scene_id(s)?,
        None => scene.active_tab.ok_or(McpError::NoActiveTab)?,
    };

    // Grant a lease with sufficient capabilities for tile+content operations
    let lease_id = scene.grant_lease(
        &p.namespace,
        p.ttl_ms,
        vec![
            Capability::CreateTile,
            Capability::UpdateTile,
            Capability::CreateNode,
            Capability::UpdateNode,
        ],
    );

    let bounds = Rect::new(p.bounds.x, p.bounds.y, p.bounds.width, p.bounds.height);
    let tile_id = scene.create_tile(tab_id, &p.namespace, lease_id, bounds, p.z_order)?;

    Ok(CreateTileResult {
        tile_id: tile_id.to_string(),
        lease_id: lease_id.to_string(),
        tab_id: tab_id.to_string(),
        namespace: p.namespace,
    })
}

// ─── set_content ─────────────────────────────────────────────────────────────

/// Parameters for `set_content`.
#[derive(Debug, Deserialize)]
pub struct SetContentParams {
    /// ID of the tile to set content on.
    pub tile_id: String,
    /// Markdown text to display.
    pub content: String,
    /// Font size in pixels. Defaults to 16.
    #[serde(default = "default_font_size")]
    pub font_size_px: f32,
    /// Hex or well-known color string for text. Defaults to white (#ffffff).
    #[serde(default = "default_color")]
    pub color: ColorParams,
    /// Hex or well-known color string for background. Optional.
    pub background: Option<ColorParams>,
    /// Text alignment: "start", "center", or "end". Defaults to "start".
    #[serde(default = "default_alignment")]
    pub alignment: String,
}

/// RGBA color as individual channels in [0.0, 1.0].
#[derive(Debug, Deserialize)]
pub struct ColorParams {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    #[serde(default = "default_alpha")]
    pub a: f32,
}

fn default_font_size() -> f32 {
    16.0
}

fn default_color() -> ColorParams {
    ColorParams { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }
}

fn default_alpha() -> f32 {
    1.0
}

fn default_alignment() -> String {
    "start".to_string()
}

/// Response from `set_content`.
#[derive(Debug, Serialize)]
pub struct SetContentResult {
    /// UUID of the tile that was updated.
    pub tile_id: String,
    /// UUID of the new root node created to hold the content.
    pub node_id: String,
    /// Number of characters in the content.
    pub content_len: usize,
}

/// Set markdown text content on a tile.
///
/// Replaces the tile's root node with a [`TextMarkdownNode`] spanning the
/// full tile bounds. Any previous root node is discarded.
///
/// # Errors
/// - `invalid_params` if `tile_id` is not a valid UUID or content is empty.
/// - `invalid_id` if `tile_id` is malformed.
/// - `scene_error` if the tile does not exist.
pub fn handle_set_content(params: Value, scene: &mut SceneGraph) -> McpResult<SetContentResult> {
    let p: SetContentParams = parse_params(params)?;

    if p.content.is_empty() {
        return Err(McpError::InvalidParams("content must be non-empty".to_string()));
    }

    if p.font_size_px <= 0.0 {
        return Err(McpError::InvalidParams("font_size_px must be > 0".to_string()));
    }

    let tile_id = parse_scene_id(&p.tile_id)?;

    // Look up the tile to get its bounds for the text node
    let tile_bounds = scene
        .tiles
        .get(&tile_id)
        .ok_or_else(|| McpError::SceneError(format!("tile not found: {tile_id}")))?
        .bounds;

    let alignment = match p.alignment.as_str() {
        "center" => TextAlign::Center,
        "end" => TextAlign::End,
        _ => TextAlign::Start,
    };

    let color = Rgba::new(p.color.r, p.color.g, p.color.b, p.color.a);
    let background = p.background.map(|bg| Rgba::new(bg.r, bg.g, bg.b, bg.a));

    let node_id = SceneId::new();
    let node = Node {
        id: node_id,
        children: vec![],
        data: NodeData::TextMarkdown(TextMarkdownNode {
            content: p.content.clone(),
            // Fill the entire tile
            bounds: Rect::new(0.0, 0.0, tile_bounds.width, tile_bounds.height),
            font_size_px: p.font_size_px,
            font_family: FontFamily::SystemSansSerif,
            color,
            background,
            alignment,
            overflow: TextOverflow::Clip,
        }),
    };

    scene.set_tile_root(tile_id, node)?;

    Ok(SetContentResult {
        tile_id: tile_id.to_string(),
        node_id: node_id.to_string(),
        content_len: p.content.len(),
    })
}

// ─── dismiss ─────────────────────────────────────────────────────────────────

/// Parameters for `dismiss`.
#[derive(Debug, Deserialize)]
pub struct DismissParams {
    /// ID of the tile to delete. The tile's lease is revoked and the tile
    /// (plus all its nodes) is removed from the scene.
    pub tile_id: String,
}

/// Response from `dismiss`.
#[derive(Debug, Serialize)]
pub struct DismissResult {
    /// UUID of the tile that was dismissed.
    pub tile_id: String,
}

/// Delete a tile and release its lease.
///
/// Revokes the lease associated with the tile, which removes the tile and all
/// of its nodes from the scene. This is the inverse of `create_tile`.
///
/// # Errors
/// - `invalid_id` if `tile_id` is not a valid UUID.
/// - `scene_error` if the tile does not exist or its lease is not found.
pub fn handle_dismiss(params: Value, scene: &mut SceneGraph) -> McpResult<DismissResult> {
    let p: DismissParams = parse_params(params)?;
    let tile_id = parse_scene_id(&p.tile_id)?;

    let lease_id = scene
        .tiles
        .get(&tile_id)
        .ok_or_else(|| McpError::SceneError(format!("tile not found: {tile_id}")))?
        .lease_id;

    scene.revoke_lease(lease_id)?;

    Ok(DismissResult {
        tile_id: p.tile_id,
    })
}

// ─── publish_to_zone ─────────────────────────────────────────────────────────

/// Parameters for `publish_to_zone`.
#[derive(Debug, Deserialize)]
pub struct PublishToZoneParams {
    /// Name of the target zone (must exist in the zone registry).
    pub zone_name: String,
    /// Markdown content to publish.
    pub content: String,
    /// Optional namespace for the lease. Defaults to "mcp".
    #[serde(default = "default_mcp_namespace")]
    pub namespace: String,
    /// Font size in pixels. Defaults to 16.
    #[serde(default = "default_font_size")]
    pub font_size_px: f32,
    /// TTL in microseconds. A value of 0 selects the built-in default of
    /// 60_000 ms (60_000_000 µs). Defaults to 0.
    #[serde(default)]
    pub ttl_us: u64,
    /// Merge key for idempotent zone publishes (optional).
    #[serde(default)]
    pub merge_key: Option<String>,
}

fn default_mcp_namespace() -> String {
    "mcp".to_string()
}

/// Response from `publish_to_zone`.
#[derive(Debug, Serialize)]
pub struct PublishToZoneResult {
    /// The zone name content was published to.
    pub zone_name: String,
    /// Effective TTL in microseconds applied to the lease (never 0 in response).
    pub ttl_us: u64,
    /// Echo of the merge key, if provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
}

/// Publish markdown content to a named zone.
///
/// This is the primary LLM-first tool: a single call with zero scene context
/// required. It looks up the zone by name, grants a lease, and delegates to
/// the zone publishing engine (`SceneGraph::publish_to_zone_with_lease`), which
/// enforces contention policies, validates media types, respects
/// `geometry_policy`, and stores the publication in `zone_registry.active_publishes`.
/// Tile creation is deferred to the compositor, which resolves zone publishes
/// to tiles at render time.
///
/// Zone publishes are global (not tab-scoped in v1). No active tab is required.
///
/// # Errors
/// - `invalid_params` if `zone_name` or `content` is empty.
/// - `zone_not_found` if the zone name is not registered.
/// - `scene_error` for contention policy violations (max publishers, max keys)
///   or lease enforcement failures (no active lease, orphaned/suspended lease).
pub fn handle_publish_to_zone(
    params: Value,
    scene: &mut SceneGraph,
) -> McpResult<PublishToZoneResult> {
    let p: PublishToZoneParams = parse_params(params)?;

    if p.zone_name.trim().is_empty() {
        return Err(McpError::InvalidParams("zone_name must be non-empty".to_string()));
    }
    if p.content.is_empty() {
        return Err(McpError::InvalidParams("content must be non-empty".to_string()));
    }

    // Validate zone exists before granting a lease, to fail fast on bad zone names.
    if !scene.zone_registry.zones.contains_key(&p.zone_name) {
        return Err(McpError::ZoneNotFound(p.zone_name));
    }

    // Convert ttl_us to ttl_ms for lease grant; 0 means use a sensible default.
    // Use div_ceil to ensure any positive sub-millisecond TTL rounds up to at
    // least 1 ms, preventing an unintended indefinite lease (ttl_ms == 0).
    let ttl_ms = if p.ttl_us == 0 {
        60_000u64 // 1 minute default
    } else {
        p.ttl_us.div_ceil(1_000)
    };

    // Grant lease for MCP session tracking. Zone publishing requires an active
    // lease (spec §Zone Publish Requires Active Lease); we grant one here so
    // that publish_to_zone_with_lease can verify it.
    // We grant PublishZone(<zone_name>) — the zone-specific capability required
    // by the spec (§Capability Vocabulary: publish_zone:<zone_name>). The
    // tile/node capabilities previously granted here are vestigial; tile
    // creation is now deferred to the compositor and never done by this handler.
    let _lease_id = scene.grant_lease(
        &p.namespace,
        ttl_ms,
        vec![Capability::PublishZone(p.zone_name.clone())],
    );

    // Delegate to the real zone engine. This enforces contention policy
    // (LatestWins / Stack / MergeByKey), validates accepted_media_types,
    // and stores the record in zone_registry.active_publishes.
    let content = ZoneContent::StreamText(p.content);
    scene.publish_to_zone_with_lease(&p.zone_name, content, &p.namespace, p.merge_key.clone())?;

    Ok(PublishToZoneResult {
        zone_name: p.zone_name,
        // Return the effective TTL used for the lease (ttl_ms converted back to
        // microseconds), not the raw request value, so callers know what was applied.
        ttl_us: ttl_ms * 1_000,
        merge_key: p.merge_key,
    })
}

// ─── list_zones ──────────────────────────────────────────────────────────────

/// Parameters for `list_zones` — no required fields.
#[derive(Debug, Deserialize, Default)]
pub struct ListZonesParams {}

/// A single zone entry in the list response.
#[derive(Debug, Serialize)]
pub struct ZoneEntry {
    /// Unique name of the zone.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Stable UUID for the zone definition.
    pub id: String,
    /// Whether the zone currently has any tiles visible on the active tab.
    pub has_content: bool,
}

/// Response from `list_zones`.
#[derive(Debug, Serialize)]
pub struct ListZonesResult {
    /// All registered zones.
    pub zones: Vec<ZoneEntry>,
    /// Total number of zones.
    pub count: usize,
}

/// List all available zones and their current state.
///
/// `has_content` is true when `zone_registry.active_publishes` contains at
/// least one record for the zone — i.e., something has been published to the
/// zone and the record has not been evicted by contention policy or expiry.
/// This is the authoritative occupancy check, not a tile-namespace heuristic.
///
/// # Errors
/// - None (always succeeds; returns an empty list if no zones are registered).
pub fn handle_list_zones(params: Value, scene: &SceneGraph) -> McpResult<ListZonesResult> {
    // Use the same parse_params helper as other tool handlers; tolerates null → {}
    let _: ListZonesParams = parse_params(params)?;

    let mut zones: Vec<ZoneEntry> = scene
        .zone_registry
        .zones
        .values()
        .map(|z| ZoneEntry {
            name: z.name.clone(),
            description: z.description.clone(),
            id: z.id.to_string(),
            // A zone has content when zone_registry.active_publishes contains
            // at least one record for it. This is the authoritative source of
            // zone occupancy (not a tile-namespace heuristic).
            has_content: scene
                .zone_registry
                .active_publishes
                .get(&z.name)
                .is_some_and(|v| !v.is_empty()),
        })
        .collect();

    // Stable ordering by name for deterministic output
    zones.sort_by(|a, b| a.name.cmp(&b.name));
    let count = zones.len();

    Ok(ListZonesResult { zones, count })
}

// ─── list_scene ──────────────────────────────────────────────────────────────

/// Parameters for `list_scene` — no required fields.
#[derive(Debug, Deserialize, Default)]
pub struct ListSceneParams {}

/// A single tab entry in the list_scene response.
#[derive(Debug, Serialize)]
pub struct TabEntry {
    /// UUID of the tab.
    pub tab_id: String,
    /// Human-readable tab name.
    pub name: String,
    /// Display order.
    pub display_order: u32,
}

/// Response from `list_scene` (guest-restricted view).
///
/// Returns tab names and the zone registry only — not full tile topology.
/// This is intentionally limited to prevent guest agents from enumerating
/// the internal scene structure. Full topology is available to resident agents
/// via gRPC subscriptions.
#[derive(Debug, Serialize)]
pub struct ListSceneResult {
    /// All tabs in display order.
    pub tabs: Vec<TabEntry>,
    /// All registered zones (same as `list_zones`).
    pub zones: Vec<ZoneEntry>,
}

/// Return a restricted scene view: tab names and zone registry.
///
/// This is the guest-accessible variant of scene introspection. It does not
/// expose tile topology, node contents, lease state, or agent namespaces.
///
/// # Errors
/// - None (always succeeds; returns empty lists if scene is empty).
pub fn handle_list_scene(params: Value, scene: &SceneGraph) -> McpResult<ListSceneResult> {
    let _: ListSceneParams = parse_params(params)?;

    let mut tabs: Vec<TabEntry> = scene
        .tabs
        .values()
        .map(|t| TabEntry {
            tab_id: t.id.to_string(),
            name: t.name.clone(),
            display_order: t.display_order,
        })
        .collect();
    tabs.sort_by_key(|t| t.display_order);

    // Reuse list_zones logic for the zone portion
    let zones_result = handle_list_zones(Value::Null, scene)?;

    Ok(ListSceneResult {
        tabs,
        zones: zones_result.zones,
    })
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Deserialize tool parameters from a JSON value.
fn parse_params<T: for<'de> serde::Deserialize<'de>>(params: Value) -> McpResult<T> {
    // Treat null params as an empty object for tools with all-optional params
    let v = if params.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        params
    };
    serde_json::from_value(v).map_err(|e| McpError::InvalidParams(e.to_string()))
}

/// Parse a string as a [`SceneId`] (UUID).
fn parse_scene_id(s: &str) -> McpResult<SceneId> {
    uuid::Uuid::parse_str(s)
        .map(SceneId::from_uuid)
        .map_err(|e| McpError::InvalidId(format!("invalid UUID '{s}': {e}")))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tze_hud_scene::{graph::SceneGraph, types::{ZoneDefinition, GeometryPolicy, ZoneMediaType, RenderingPolicy, ContentionPolicy, LayerAttachment}, SceneId};

    fn scene_with_tab() -> (SceneGraph, SceneId) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).expect("create tab");
        (scene, tab_id)
    }

    // ── create_tab ──────────────────────────────────────────────────────────

    #[test]
    fn test_create_tab_basic() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result = handle_create_tab(json!({"name": "Dashboard"}), &mut scene).unwrap();
        assert_eq!(result.name, "Dashboard");
        assert_eq!(result.display_order, 0);
        assert_eq!(scene.tabs.len(), 1);
    }

    #[test]
    fn test_create_tab_explicit_order() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result =
            handle_create_tab(json!({"name": "Tab", "display_order": 5}), &mut scene).unwrap();
        assert_eq!(result.display_order, 5);
    }

    #[test]
    fn test_create_tab_auto_increments_order() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        handle_create_tab(json!({"name": "A", "display_order": 3}), &mut scene).unwrap();
        let r = handle_create_tab(json!({"name": "B"}), &mut scene).unwrap();
        assert_eq!(r.display_order, 4);
    }

    #[test]
    fn test_create_tab_empty_name_fails() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let err = handle_create_tab(json!({"name": ""}), &mut scene).unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }

    #[test]
    fn test_create_tab_duplicate_order_fails() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        handle_create_tab(json!({"name": "A", "display_order": 0}), &mut scene).unwrap();
        let err = handle_create_tab(json!({"name": "B", "display_order": 0}), &mut scene)
            .unwrap_err();
        assert!(matches!(err, McpError::SceneError(_)));
    }

    // ── create_tile ─────────────────────────────────────────────────────────

    #[test]
    fn test_create_tile_basic() {
        let (mut scene, _tab_id) = scene_with_tab();
        let result = handle_create_tile(
            json!({
                "namespace": "agent-1",
                "bounds": {"x": 0.0, "y": 0.0, "width": 400.0, "height": 300.0}
            }),
            &mut scene,
        )
        .unwrap();
        assert!(!result.tile_id.is_empty());
        assert_eq!(result.namespace, "agent-1");
        assert_eq!(scene.tile_count(), 1);
    }

    #[test]
    fn test_create_tile_explicit_tab() {
        let (mut scene, tab_id) = scene_with_tab();
        let result = handle_create_tile(
            json!({
                "tab_id": tab_id.to_string(),
                "namespace": "agent-1",
                "bounds": {"x": 0.0, "y": 0.0, "width": 200.0, "height": 200.0}
            }),
            &mut scene,
        )
        .unwrap();
        assert_eq!(result.tab_id, tab_id.to_string());
    }

    #[test]
    fn test_create_tile_no_active_tab_fails() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let err = handle_create_tile(
            json!({
                "namespace": "agent-1",
                "bounds": {"x": 0.0, "y": 0.0, "width": 200.0, "height": 200.0}
            }),
            &mut scene,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::NoActiveTab));
    }

    #[test]
    fn test_create_tile_invalid_bounds_fails() {
        let (mut scene, _) = scene_with_tab();
        let err = handle_create_tile(
            json!({
                "namespace": "agent-1",
                "bounds": {"x": 0.0, "y": 0.0, "width": 0.0, "height": 300.0}
            }),
            &mut scene,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }

    #[test]
    fn test_create_tile_empty_namespace_fails() {
        let (mut scene, _) = scene_with_tab();
        let err = handle_create_tile(
            json!({
                "namespace": "",
                "bounds": {"x": 0.0, "y": 0.0, "width": 200.0, "height": 200.0}
            }),
            &mut scene,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }

    #[test]
    fn test_create_tile_grants_lease() {
        let (mut scene, _) = scene_with_tab();
        handle_create_tile(
            json!({
                "namespace": "agent-1",
                "bounds": {"x": 0.0, "y": 0.0, "width": 200.0, "height": 200.0}
            }),
            &mut scene,
        )
        .unwrap();
        assert_eq!(scene.leases.len(), 1);
    }

    // ── set_content ─────────────────────────────────────────────────────────

    #[test]
    fn test_set_content_basic() {
        let (mut scene, _) = scene_with_tab();
        let tile = handle_create_tile(
            json!({
                "namespace": "agent-1",
                "bounds": {"x": 0.0, "y": 0.0, "width": 400.0, "height": 300.0}
            }),
            &mut scene,
        )
        .unwrap();

        let result = handle_set_content(
            json!({"tile_id": tile.tile_id, "content": "# Hello"}),
            &mut scene,
        )
        .unwrap();

        assert_eq!(result.tile_id, tile.tile_id);
        assert_eq!(result.content_len, 7);
        assert_eq!(scene.node_count(), 1);
    }

    #[test]
    fn test_set_content_replaces_existing() {
        let (mut scene, _) = scene_with_tab();
        let tile = handle_create_tile(
            json!({
                "namespace": "a",
                "bounds": {"x": 0.0, "y": 0.0, "width": 400.0, "height": 300.0}
            }),
            &mut scene,
        )
        .unwrap();

        handle_set_content(
            json!({"tile_id": tile.tile_id, "content": "First"}),
            &mut scene,
        )
        .unwrap();
        assert_eq!(scene.node_count(), 1);

        handle_set_content(
            json!({"tile_id": tile.tile_id, "content": "Second"}),
            &mut scene,
        )
        .unwrap();
        // Root replaced; still exactly 1 node
        assert_eq!(scene.node_count(), 1);
    }

    #[test]
    fn test_set_content_empty_content_fails() {
        let (mut scene, _) = scene_with_tab();
        let tile = handle_create_tile(
            json!({
                "namespace": "a",
                "bounds": {"x": 0.0, "y": 0.0, "width": 200.0, "height": 200.0}
            }),
            &mut scene,
        )
        .unwrap();
        let err = handle_set_content(
            json!({"tile_id": tile.tile_id, "content": ""}),
            &mut scene,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }

    #[test]
    fn test_set_content_invalid_tile_id_fails() {
        let (mut scene, _) = scene_with_tab();
        let err = handle_set_content(
            json!({"tile_id": "not-a-uuid", "content": "hello"}),
            &mut scene,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::InvalidId(_)));
    }

    #[test]
    fn test_set_content_nonexistent_tile_fails() {
        let (mut scene, _) = scene_with_tab();
        let fake_id = SceneId::new().to_string();
        let err = handle_set_content(
            json!({"tile_id": fake_id, "content": "hello"}),
            &mut scene,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::SceneError(_)));
    }

    // ── publish_to_zone ─────────────────────────────────────────────────────

    fn scene_with_zone() -> (SceneGraph, SceneId, String) {
        let (mut scene, tab_id) = scene_with_tab();
        let zone_name = "main-overlay".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "Primary overlay zone".to_string(),
                geometry_policy: GeometryPolicy::Relative { x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 0.1 },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::LatestWins,
                max_publishers: 4,
                transport_constraint: None,
                auto_clear_ms: None,
                ephemeral: false,
                layer_attachment: LayerAttachment::Content,
            },
        );
        (scene, tab_id, zone_name)
    }

    #[test]
    fn test_publish_to_zone_basic() {
        let (mut scene, _, zone) = scene_with_zone();
        let result = handle_publish_to_zone(
            json!({"zone_name": zone, "content": "## Status: OK"}),
            &mut scene,
        )
        .unwrap();
        assert_eq!(result.zone_name, zone);
        // Publishing goes to zone_registry.active_publishes; tiles are compositor-resolved.
        assert_eq!(scene.tile_count(), 0);
        assert_eq!(scene.node_count(), 0);
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(publishes.len(), 1);
        assert!(matches!(&publishes[0].content, tze_hud_scene::types::ZoneContent::StreamText(s) if s == "## Status: OK"));
    }

    #[test]
    fn test_publish_to_zone_with_ttl_us() {
        let (mut scene, _, zone) = scene_with_zone();
        let result = handle_publish_to_zone(
            json!({"zone_name": zone, "content": "hello", "ttl_us": 120_000_000u64}),
            &mut scene,
        )
        .unwrap();
        assert_eq!(result.ttl_us, 120_000_000u64);
    }

    #[test]
    fn test_publish_to_zone_with_merge_key() {
        let (mut scene, _, zone) = scene_with_zone();
        let result = handle_publish_to_zone(
            json!({"zone_name": zone, "content": "hello", "merge_key": "subtitle-main"}),
            &mut scene,
        )
        .unwrap();
        assert_eq!(result.merge_key.as_deref(), Some("subtitle-main"));
    }

    #[test]
    fn test_publish_to_zone_unknown_zone_fails() {
        let (mut scene, _, _) = scene_with_zone();
        let err = handle_publish_to_zone(
            json!({"zone_name": "does-not-exist", "content": "hi"}),
            &mut scene,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::ZoneNotFound(_)));
    }

    #[test]
    fn test_publish_to_zone_no_tab_succeeds() {
        // Zone publishing is global (not tab-scoped) in v1. No active tab is required.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone_name = "z".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "z".to_string(),
                geometry_policy: GeometryPolicy::Relative { x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 0.1 },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::LatestWins,
                max_publishers: 4,
                transport_constraint: None,
                auto_clear_ms: None,
                ephemeral: false,
                layer_attachment: LayerAttachment::Content,
            },
        );
        let result =
            handle_publish_to_zone(json!({"zone_name": zone_name, "content": "hi"}), &mut scene)
                .unwrap();
        assert_eq!(result.zone_name, zone_name);
        assert!(scene.zone_registry.active_publishes.contains_key(&zone_name));
    }

    #[test]
    fn test_publish_to_zone_contention_policy_latest_wins() {
        // scene_with_zone creates a LatestWins zone; a second publish must replace
        // the first (single record in active_publishes after both calls).
        let (mut scene, _, zone) = scene_with_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "first"}),
            &mut scene,
        )
        .unwrap();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "second"}),
            &mut scene,
        )
        .unwrap();
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(publishes.len(), 1, "LatestWins must replace old record");
        assert!(
            matches!(&publishes[0].content, tze_hud_scene::types::ZoneContent::StreamText(s) if s == "second"),
            "latest content must win"
        );
    }

    #[test]
    fn test_publish_to_zone_empty_content_fails() {
        let (mut scene, _, zone) = scene_with_zone();
        let err = handle_publish_to_zone(
            json!({"zone_name": zone, "content": ""}),
            &mut scene,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }

    // ── list_zones ──────────────────────────────────────────────────────────

    #[test]
    fn test_list_zones_empty() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let result = handle_list_zones(json!(null), &scene).unwrap();
        assert_eq!(result.count, 0);
        assert!(result.zones.is_empty());
    }

    #[test]
    fn test_list_zones_returns_registered() {
        let (scene, _, zone) = scene_with_zone();
        let result = handle_list_zones(json!(null), &scene).unwrap();
        assert_eq!(result.count, 1);
        assert_eq!(result.zones[0].name, zone);
    }

    #[test]
    fn test_list_zones_has_content_flag() {
        let (mut scene, _, zone) = scene_with_zone();
        // Before publishing: zone_registry.active_publishes is empty → no content
        let before = handle_list_zones(json!(null), &scene).unwrap();
        assert!(!before.zones[0].has_content);

        // After publishing: active_publishes contains a record → has_content = true
        // (namespace argument is used for the lease; the zone name drives the publish)
        handle_publish_to_zone(
            json!({"zone_name": zone.clone(), "content": "hi", "namespace": zone.clone()}),
            &mut scene,
        )
        .unwrap();
        let after = handle_list_zones(json!(null), &scene).unwrap();
        assert!(after.zones[0].has_content);
    }

    #[test]
    fn test_list_zones_sorted_by_name() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.create_tab("Main", 0).unwrap();
        for name in ["zebra", "alpha", "mango"] {
            scene.zone_registry.zones.insert(
                name.to_string(),
                ZoneDefinition {
                    id: SceneId::new(),
                    name: name.to_string(),
                    description: "".to_string(),
                    geometry_policy: GeometryPolicy::Relative { x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 0.1 },
                    accepted_media_types: vec![ZoneMediaType::StreamText],
                    rendering_policy: RenderingPolicy::default(),
                    contention_policy: ContentionPolicy::LatestWins,
                    max_publishers: 4,
                    transport_constraint: None,
                    auto_clear_ms: None,
                    ephemeral: false,
                    layer_attachment: LayerAttachment::Content,
                },
            );
        }
        let result = handle_list_zones(json!(null), &scene).unwrap();
        let names: Vec<&str> = result.zones.iter().map(|z| z.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mango", "zebra"]);
    }

    #[test]
    fn test_list_zones_invalid_params_fails() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let err = handle_list_zones(json!("unexpected-string"), &scene).unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }
}
