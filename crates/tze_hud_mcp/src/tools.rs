//! MCP tool implementations.
//!
//! Each function takes `params: serde_json::Value` and a mutable reference to
//! the shared scene state, and returns a [`McpResult`] with a serializable
//! response value.
//!
//! Tool naming follows the issue spec:
//! - `create_tab`        → `handle_create_tab`
//! - `create_tile`       → `handle_create_tile`
//! - `set_content`       → `handle_set_content`
//! - `dismiss`           → `handle_dismiss`
//! - `publish_to_zone`   → `handle_publish_to_zone`
//! - `list_zones`        → `handle_list_zones`
//! - `list_scene`        → `handle_list_scene`
//! - `publish_to_widget` → `handle_publish_to_widget`
//! - `list_widgets`      → `handle_list_widgets`
//! - `clear_widget`      → `handle_clear_widget`

use crate::{error::McpError, types::McpResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tze_hud_scene::{
    graph::SceneGraph,
    types::{
        Capability, FontFamily, Node, NodeData, NotificationPayload, Rect, Rgba, SceneId,
        StatusBarPayload, TextAlign, TextMarkdownNode, TextOverflow, WidgetParameterValue,
        ZoneContent,
    },
};

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
        return Err(McpError::InvalidParams(
            "name must be non-empty".to_string(),
        ));
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
        return Err(McpError::InvalidParams(
            "namespace must be non-empty".to_string(),
        ));
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
    ColorParams {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    }
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
        return Err(McpError::InvalidParams(
            "content must be non-empty".to_string(),
        ));
    }

    if p.font_size_px <= 0.0 {
        return Err(McpError::InvalidParams(
            "font_size_px must be > 0".to_string(),
        ));
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

    Ok(DismissResult { tile_id: p.tile_id })
}

// ─── publish_to_zone ─────────────────────────────────────────────────────────

/// Parameters for `publish_to_zone`.
#[derive(Debug, Deserialize)]
pub struct PublishToZoneParams {
    /// Name of the target zone (must exist in the zone registry).
    pub zone_name: String,
    /// Content to publish. Accepts either:
    /// - A plain string → interpreted as `StreamText`
    /// - A JSON object with a `"type"` field → dispatched to the matching
    ///   `ZoneContent` variant:
    ///   - `{"type":"stream_text","text":"..."}` → `StreamText`
    ///   - `{"type":"notification","text":"...","icon":"","urgency":1}` → `Notification`
    ///   - `{"type":"status_bar","entries":{"key":"val",...}}` → `StatusBar`
    ///   - `{"type":"solid_color","r":1.0,"g":0.0,"b":0.0,"a":1.0}` → `SolidColor`
    ///   - `{"type":"static_image","resource_id":"<hex>"}` → `StaticImage`
    pub content: Value,
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
    /// Byte-offset breakpoints for streaming word-by-word reveal (optional).
    ///
    /// Only meaningful when `content` is a `StreamText` publish (plain string or
    /// `{"type":"stream_text","text":"..."}`).  Breakpoints identify byte offsets
    /// in the UTF-8 text string at which the compositor pauses reveal.
    ///
    /// An empty array (the default) reveals the full text immediately.
    ///
    /// Per spec §Subtitle Streaming Word-by-Word Reveal:
    ///   - `[3, 9, 15]` for `"The quick brown"` → reveals "The", "The quick",
    ///     "The quick brown", then the full text.
    ///
    /// `u64` is used for platform-stable JSON serialization (rather than
    /// `usize`, which is architecture-dependent).
    #[serde(default)]
    pub breakpoints: Vec<u64>,
}

/// Parse the polymorphic `content` field into a `ZoneContent`.
///
/// - Plain string → `StreamText`
/// - Object with `"type"` → dispatched by variant name
fn parse_zone_content(content: &Value) -> Result<ZoneContent, McpError> {
    match content {
        Value::String(s) => Ok(ZoneContent::StreamText(s.clone())),
        Value::Object(obj) => {
            let type_str = obj
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    McpError::InvalidParams(
                        "object content must have a \"type\" field (one of: stream_text, notification, status_bar, solid_color, static_image)".to_string(),
                    )
                })?;
            match type_str {
                "stream_text" => {
                    let text = obj
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    Ok(ZoneContent::StreamText(text))
                }
                "notification" => {
                    let text = obj
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let icon = obj
                        .get("icon")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let urgency = obj.get("urgency").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                    let ttl_ms = obj.get("ttl_ms").and_then(|v| v.as_u64());
                    Ok(ZoneContent::Notification(NotificationPayload {
                        text,
                        icon,
                        urgency,
                        ttl_ms,
                    }))
                }
                "status_bar" => {
                    let entries: HashMap<String, String> = obj
                        .get("entries")
                        .and_then(|v| v.as_object())
                        .map(|m| {
                            m.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect()
                        })
                        .unwrap_or_default();
                    if entries.is_empty() {
                        return Err(McpError::InvalidParams(
                            "status_bar content must have a non-empty \"entries\" object"
                                .to_string(),
                        ));
                    }
                    Ok(ZoneContent::StatusBar(StatusBarPayload { entries }))
                }
                "solid_color" => {
                    let r = obj.get("r").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    let g = obj.get("g").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    let b = obj.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    let a = obj.get("a").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                    Ok(ZoneContent::SolidColor(Rgba { r, g, b, a }))
                }
                "static_image" => {
                    use tze_hud_scene::types::ResourceId;
                    let hex = obj
                        .get("resource_id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            McpError::InvalidParams(
                                "static_image content must have a \"resource_id\" field (hex-encoded 32-byte BLAKE3 hash)".to_string(),
                            )
                        })?;
                    // Decode hex without an external crate: parse pairs of chars as u8.
                    if hex.len() != 64 {
                        return Err(McpError::InvalidParams(format!(
                            "static_image \"resource_id\" must be 64 hex chars (32 bytes), got {}",
                            hex.len()
                        )));
                    }
                    let mut raw = [0u8; 32];
                    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
                        let hi = char::from(chunk[0]).to_digit(16);
                        let lo = char::from(chunk[1]).to_digit(16);
                        if let (Some(hi), Some(lo)) = (hi, lo) {
                            raw[i] = (hi * 16 + lo) as u8;
                        } else {
                            return Err(McpError::InvalidParams(format!(
                                "static_image \"resource_id\" is not valid hex: \"{hex}\""
                            )));
                        }
                    }
                    Ok(ZoneContent::StaticImage(ResourceId::from_bytes(raw)))
                }
                other => Err(McpError::InvalidParams(format!(
                    "unknown content type \"{other}\"; expected one of: stream_text, notification, status_bar, solid_color, static_image"
                ))),
            }
        }
        _ => Err(McpError::InvalidParams(
            "content must be a string or an object with a \"type\" field".to_string(),
        )),
    }
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
        return Err(McpError::InvalidParams(
            "zone_name must be non-empty".to_string(),
        ));
    }
    if p.content.is_null()
        || (p.content.is_string() && p.content.as_str().unwrap_or_default().is_empty())
    {
        return Err(McpError::InvalidParams(
            "content must be non-empty".to_string(),
        ));
    }

    // Validate zone exists before granting a lease, to fail fast on bad zone names.
    if !scene.zone_registry.zones.contains_key(&p.zone_name) {
        return Err(McpError::ZoneNotFound(p.zone_name));
    }

    // Parse the polymorphic content field into ZoneContent.
    let content = parse_zone_content(&p.content)?;

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

    // Validate that breakpoints are only used with StreamText content.
    // Breakpoints are a StreamText-specific feature; sending them alongside
    // other content types is a caller error.
    if !p.breakpoints.is_empty() && !matches!(content, ZoneContent::StreamText(_)) {
        return Err(McpError::InvalidParams(
            "breakpoints are only valid for StreamText content".to_string(),
        ));
    }

    // Delegate to the real zone engine. This enforces contention policy
    // (LatestWins / Stack / MergeByKey), validates accepted_media_types,
    // and stores the record in zone_registry.active_publishes.
    //
    // When breakpoints are provided (StreamText streaming reveal), use the
    // breakpoint-aware variant so the compositor can reveal text progressively.
    if !p.breakpoints.is_empty() {
        scene.publish_to_zone_with_lease_and_breakpoints(
            &p.zone_name,
            content,
            &p.namespace,
            p.merge_key.clone(),
            p.breakpoints,
        )?;
    } else {
        scene.publish_to_zone_with_lease(
            &p.zone_name,
            content,
            &p.namespace,
            p.merge_key.clone(),
        )?;
    }

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

// ─── publish_to_widget ───────────────────────────────────────────────────────

/// Parameters for `publish_to_widget`.
#[derive(Debug, Deserialize)]
pub struct PublishToWidgetParams {
    /// Widget instance name (instance_id or widget_type_name for single-instance).
    pub widget_name: String,
    /// Optional disambiguation: explicit instance_id when multiple instances of
    /// the same type exist on a tab. When provided, overrides `widget_name` for
    /// instance resolution.
    #[serde(default)]
    pub instance_id: Option<String>,
    /// Parameter values to publish. Keys are parameter names, values are typed.
    ///
    /// JSON type mapping:
    /// - f32 parameter → JSON number
    /// - string parameter → JSON string
    /// - color parameter → JSON object `{"r": number, "g": number, "b": number, "a": number}` (Note: currently parsed as f32 [0.0, 1.0], alignment to u8 planned)
    /// - enum parameter → JSON string
    pub params: HashMap<String, Value>,
    /// Transition duration in milliseconds (0 = instant). Defaults to 0.
    #[serde(default)]
    pub transition_ms: u32,
    /// Optional namespace (auto-derived from "mcp" if omitted).
    #[serde(default = "default_mcp_namespace")]
    pub namespace: String,
    /// TTL in microseconds (0 = use widget instance default). Defaults to 0.
    #[serde(default)]
    pub ttl_us: u64,
}

/// Response from `publish_to_widget`.
#[derive(Debug, Serialize)]
pub struct PublishToWidgetResult {
    /// Widget instance name that was published to.
    pub widget_name: String,
    /// Whether the widget is durable (true) or ephemeral (false).
    pub durable: bool,
    /// Parameter names that were successfully applied.
    pub applied_params: Vec<String>,
}

/// Convert a JSON `Value` to a `WidgetParameterValue` for a given param type.
///
/// Returns `None` if the value cannot be coerced to the expected type.
fn json_to_widget_param_value(
    v: &Value,
    param_name: &str,
    scene: &SceneGraph,
    widget_name: &str,
) -> Result<(String, WidgetParameterValue), McpError> {
    use tze_hud_scene::types::WidgetParamType;

    // Look up the parameter declaration from the widget schema.
    let instance = scene
        .widget_registry
        .instances
        .get(widget_name)
        .ok_or_else(|| McpError::SceneError(format!("widget not found: {widget_name}")))?;

    let definition = scene
        .widget_registry
        .definitions
        .get(&instance.widget_type_name)
        .ok_or_else(|| {
            McpError::SceneError(format!(
                "widget type not found: {}",
                instance.widget_type_name
            ))
        })?;

    let decl = definition
        .parameter_schema
        .iter()
        .find(|d| d.name == param_name)
        .ok_or_else(|| {
            McpError::SceneError(format!(
                "parameter '{param_name}' is not declared in widget '{widget_name}' schema (WIDGET_UNKNOWN_PARAMETER)"
            ))
        })?;

    let typed_value = match decl.param_type {
        WidgetParamType::F32 => {
            let f = v.as_f64().ok_or_else(|| {
                McpError::SceneError(format!("parameter '{param_name}' must be a number (f32)"))
            })? as f32;
            WidgetParameterValue::F32(f)
        }
        WidgetParamType::String => {
            let s = v.as_str().ok_or_else(|| {
                McpError::SceneError(format!("parameter '{param_name}' must be a string"))
            })?;
            WidgetParameterValue::String(s.to_string())
        }
        WidgetParamType::Color => {
            let obj = v.as_object().ok_or_else(|| {
                McpError::SceneError(format!(
                    "parameter '{param_name}' must be a color object {{r, g, b, a}}"
                ))
            })?;
            let r = obj.get("r").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
            let g = obj.get("g").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
            let b = obj.get("b").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
            let a = obj.get("a").and_then(|x| x.as_f64()).unwrap_or(1.0) as f32;
            WidgetParameterValue::Color(Rgba { r, g, b, a })
        }
        WidgetParamType::Enum => {
            let s = v.as_str().ok_or_else(|| {
                McpError::SceneError(format!(
                    "parameter '{param_name}' must be a string (enum value)"
                ))
            })?;
            WidgetParameterValue::Enum(s.to_string())
        }
    };

    Ok((param_name.to_string(), typed_value))
}

/// Publish parameter values to a named widget instance.
///
/// This is the primary widget interaction tool. It requires the
/// `publish_widget:<widget_name>` capability on the calling session.
///
/// # Capability
///
/// The `publish_widget:<widget_name>` capability must be present in
/// `caller_capabilities`. If absent, the call is rejected with
/// `WIDGET_CAPABILITY_MISSING`.
///
/// # Parameter types
///
/// The `params` object maps parameter names to JSON values. The JSON type
/// must match the parameter's declared type in the widget schema:
/// - f32 → JSON number
/// - string → JSON string
/// - color → JSON object `{"r": u8, "g": u8, "b": u8, "a": u8}`
/// - enum → JSON string
///
/// # Errors
/// - `invalid_params` if `widget_name` is empty or params is missing.
/// - `scene_error` with `WIDGET_CAPABILITY_MISSING` if capability absent.
/// - `scene_error` with `WIDGET_NOT_FOUND` if widget instance unknown.
/// - `scene_error` with `WIDGET_UNKNOWN_PARAMETER` if param name not in schema.
/// - `scene_error` with `WIDGET_PARAMETER_TYPE_MISMATCH` if value type wrong.
/// - `scene_error` with `WIDGET_PARAMETER_INVALID_VALUE` if value invalid.
pub fn handle_publish_to_widget(
    params: Value,
    scene: &mut SceneGraph,
    caller_capabilities: &[String],
) -> McpResult<PublishToWidgetResult> {
    let p: PublishToWidgetParams = parse_params(params)?;

    if p.widget_name.trim().is_empty() {
        return Err(McpError::InvalidParams(
            "widget_name must be non-empty".to_string(),
        ));
    }

    // Resolve instance name: instance_id overrides widget_name when present.
    let resolved_name = p
        .instance_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&p.widget_name);

    // ── Capability gate (spec §Requirement: Widget Publishing via MCP) ────────
    let required_cap = format!("publish_widget:{}", p.widget_name);
    let has_cap = caller_capabilities.iter().any(|c| c == &required_cap);

    if !has_cap {
        return Err(McpError::SceneError(format!(
            "WIDGET_CAPABILITY_MISSING: missing capability '{required_cap}'"
        )));
    }

    // ── Validate widget exists ────────────────────────────────────────────────
    if !scene.widget_registry.instances.contains_key(resolved_name) {
        return Err(McpError::SceneError(format!(
            "WIDGET_NOT_FOUND: widget instance '{resolved_name}' not found"
        )));
    }

    // ── Convert JSON params to WidgetParameterValue map ───────────────────────
    let mut typed_params: HashMap<String, WidgetParameterValue> = HashMap::new();
    for (param_name, json_val) in &p.params {
        let (name, value) = json_to_widget_param_value(json_val, param_name, scene, resolved_name)?;
        typed_params.insert(name, value);
    }

    let applied_param_names: Vec<String> = typed_params.keys().cloned().collect();

    // ── Apply via scene graph (validates schema + contention policy) ──────────
    let is_durable = scene
        .publish_to_widget(
            resolved_name,
            typed_params,
            &p.namespace,
            None, // merge_key not supported in MCP v1
            p.transition_ms,
            None, // expires_at_wall_us from ttl_us (TTL conversion deferred)
        )
        .map_err(|e| {
            // Map ValidationErrors to WIDGET_* error codes in the message
            use tze_hud_scene::ValidationError;
            match &e {
                ValidationError::WidgetNotFound { .. } => {
                    McpError::SceneError(format!("WIDGET_NOT_FOUND: {e}"))
                }
                ValidationError::WidgetUnknownParameter { .. } => {
                    McpError::SceneError(format!("WIDGET_UNKNOWN_PARAMETER: {e}"))
                }
                ValidationError::WidgetParameterTypeMismatch { .. } => {
                    McpError::SceneError(format!("WIDGET_PARAMETER_TYPE_MISMATCH: {e}"))
                }
                ValidationError::WidgetParameterInvalidValue { .. } => {
                    McpError::SceneError(format!("WIDGET_PARAMETER_INVALID_VALUE: {e}"))
                }
                ValidationError::WidgetCapabilityMissing { .. } => {
                    McpError::SceneError(format!("WIDGET_CAPABILITY_MISSING: {e}"))
                }
                _ => McpError::SceneError(e.to_string()),
            }
        })?;

    Ok(PublishToWidgetResult {
        widget_name: resolved_name.to_string(),
        durable: is_durable,
        applied_params: applied_param_names,
    })
}

// ─── list_widgets ─────────────────────────────────────────────────────────────

/// Parameters for `list_widgets` — no required fields.
#[derive(Debug, Deserialize, Default)]
pub struct ListWidgetsParams {}

/// A parameter declaration entry in the list_widgets response.
#[derive(Debug, Serialize)]
pub struct WidgetParamEntry {
    /// Parameter name.
    pub name: String,
    /// Parameter type: "f32", "string", "color", or "enum".
    pub param_type: String,
    /// Constraints (present when non-default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub f32_min: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub f32_max: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_max_bytes: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub enum_allowed_values: Vec<String>,
}

/// A widget type entry in the list_widgets response.
#[derive(Debug, Serialize)]
pub struct WidgetTypeEntry {
    /// Unique widget type id (kebab-case).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Whether publishes to this widget type are fire-and-forget (no ack).
    pub ephemeral: bool,
    /// Parameter schema.
    pub parameter_schema: Vec<WidgetParamEntry>,
}

/// A widget instance entry in the list_widgets response.
#[derive(Debug, Serialize)]
pub struct WidgetInstanceEntry {
    /// Widget type name.
    pub widget_type: String,
    /// Instance addressing key.
    pub instance_name: String,
    /// Tab UUID this instance is bound to.
    pub tab_id: String,
    /// Current effective parameter values (from last publish or defaults).
    pub current_params: HashMap<String, Value>,
}

/// Response from `list_widgets`.
#[derive(Debug, Serialize)]
pub struct ListWidgetsResult {
    /// All registered widget types.
    pub widget_types: Vec<WidgetTypeEntry>,
    /// All widget instances with their current state.
    pub widget_instances: Vec<WidgetInstanceEntry>,
    /// Total widget type count.
    pub type_count: usize,
    /// Total instance count.
    pub instance_count: usize,
}

/// Convert a `WidgetParameterValue` to a JSON `Value` for the list_widgets response.
fn widget_param_value_to_json(v: &WidgetParameterValue) -> Value {
    match v {
        WidgetParameterValue::F32(f) => Value::from(*f as f64),
        WidgetParameterValue::String(s) => Value::String(s.clone()),
        WidgetParameterValue::Color(c) => serde_json::json!({
            "r": c.r, "g": c.g, "b": c.b, "a": c.a
        }),
        WidgetParameterValue::Enum(e) => Value::String(e.clone()),
    }
}

/// List all registered widget types and their instances with current parameter values.
///
/// Returns an empty result set if no widget bundles are configured. This tool
/// is guest-accessible and requires no special capability.
///
/// # Errors
/// - None (always succeeds; returns empty lists if registry is empty).
pub fn handle_list_widgets(params: Value, scene: &SceneGraph) -> McpResult<ListWidgetsResult> {
    let _: ListWidgetsParams = parse_params(params)?;

    // ── Widget types ─────────────────────────────────────────────────────────
    use tze_hud_scene::types::WidgetParamType;

    let mut widget_types: Vec<WidgetTypeEntry> = scene
        .widget_registry
        .definitions
        .values()
        .map(|def| {
            let parameter_schema = def
                .parameter_schema
                .iter()
                .map(|decl| {
                    let param_type = match decl.param_type {
                        WidgetParamType::F32 => "f32",
                        WidgetParamType::String => "string",
                        WidgetParamType::Color => "color",
                        WidgetParamType::Enum => "enum",
                    }
                    .to_string();
                    let (f32_min, f32_max, string_max_bytes, enum_allowed_values) =
                        if let Some(c) = &decl.constraints {
                            (
                                c.f32_min,
                                c.f32_max,
                                c.string_max_bytes,
                                c.enum_allowed_values.clone(),
                            )
                        } else {
                            (None, None, None, vec![])
                        };
                    WidgetParamEntry {
                        name: decl.name.clone(),
                        param_type,
                        f32_min,
                        f32_max,
                        string_max_bytes,
                        enum_allowed_values,
                    }
                })
                .collect();
            WidgetTypeEntry {
                id: def.id.clone(),
                name: def.name.clone(),
                description: def.description.clone(),
                ephemeral: def.ephemeral,
                parameter_schema,
            }
        })
        .collect();

    // Stable ordering by id
    widget_types.sort_by(|a, b| a.id.cmp(&b.id));

    // ── Widget instances ──────────────────────────────────────────────────────
    let mut widget_instances: Vec<WidgetInstanceEntry> = scene
        .widget_registry
        .instances
        .values()
        .map(|inst| {
            let current_params: HashMap<String, Value> = inst
                .current_params
                .iter()
                .map(|(k, v)| (k.clone(), widget_param_value_to_json(v)))
                .collect();
            WidgetInstanceEntry {
                widget_type: inst.widget_type_name.clone(),
                instance_name: inst.instance_name.clone(),
                tab_id: inst.tab_id.to_string(),
                current_params,
            }
        })
        .collect();

    // Stable ordering by instance_name
    widget_instances.sort_by(|a, b| a.instance_name.cmp(&b.instance_name));

    let type_count = widget_types.len();
    let instance_count = widget_instances.len();

    Ok(ListWidgetsResult {
        widget_types,
        widget_instances,
        type_count,
        instance_count,
    })
}

// ─── clear_widget ─────────────────────────────────────────────────────────────

/// Parameters for `clear_widget`.
#[derive(Debug, Deserialize)]
pub struct ClearWidgetParams {
    /// Widget instance name (addressing key).
    pub widget_name: String,
    /// Agent namespace performing the clear. Defaults to "" (cleared publications
    /// belonging to the namespace are removed).
    #[serde(default)]
    pub namespace: String,
    /// Optional disambiguation when multiple instances share the same name.
    #[serde(default)]
    pub instance_id: Option<String>,
}

/// Response from `clear_widget`.
#[derive(Debug, Serialize)]
pub struct ClearWidgetResult {
    /// Resolved widget instance name.
    pub widget_name: String,
    /// True — the operation always succeeds or returns an error.
    pub cleared: bool,
}

/// Clear all publications by the calling agent on the specified widget instance.
///
/// Mirrors `clear_zone` semantics: removes only the calling agent's publications.
/// If no publications exist for the publisher this is a no-op (still succeeds).
/// When all publishers have been cleared the widget reverts to its default params.
///
/// # Capability
///
/// The `publish_widget:<widget_name>` capability must be present in
/// `caller_capabilities`. Agents may only clear their own publications.
///
/// # Errors
/// - `invalid_params` if `widget_name` is empty.
/// - `scene_error` with `WIDGET_CAPABILITY_MISSING` if capability absent.
/// - `scene_error` with `WIDGET_NOT_FOUND` if widget instance unknown.
pub fn handle_clear_widget(
    params: Value,
    scene: &mut SceneGraph,
    caller_capabilities: &[String],
) -> McpResult<ClearWidgetResult> {
    let p: ClearWidgetParams = parse_params(params)?;

    if p.widget_name.trim().is_empty() {
        return Err(McpError::InvalidParams(
            "widget_name must be non-empty".to_string(),
        ));
    }

    // Resolve instance name: instance_id overrides widget_name when present.
    let resolved_name = p
        .instance_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&p.widget_name);

    // ── Capability gate (mirrors publish_to_widget) ───────────────────────────
    let required_cap = format!("publish_widget:{}", p.widget_name);
    let has_cap = caller_capabilities.iter().any(|c| c == &required_cap);

    if !has_cap {
        return Err(McpError::SceneError(format!(
            "WIDGET_CAPABILITY_MISSING: missing capability '{required_cap}'"
        )));
    }

    // ── Delegate to scene graph ───────────────────────────────────────────────
    scene
        .clear_widget_for_publisher(resolved_name, &p.namespace)
        .map_err(|e| {
            use tze_hud_scene::ValidationError;
            match &e {
                ValidationError::WidgetNotFound { .. } => {
                    McpError::SceneError(format!("WIDGET_NOT_FOUND: {e}"))
                }
                _ => McpError::SceneError(e.to_string()),
            }
        })?;

    Ok(ClearWidgetResult {
        widget_name: resolved_name.to_string(),
        cleared: true,
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
    use tze_hud_scene::{
        SceneId,
        graph::SceneGraph,
        types::{
            ContentionPolicy, GeometryPolicy, LayerAttachment, RenderingPolicy, ZoneDefinition,
            ZoneMediaType,
        },
    };

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
        let err =
            handle_create_tab(json!({"name": "B", "display_order": 0}), &mut scene).unwrap_err();
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
        let err = handle_set_content(json!({"tile_id": tile.tile_id, "content": ""}), &mut scene)
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
        let err = handle_set_content(json!({"tile_id": fake_id, "content": "hello"}), &mut scene)
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
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 1.0,
                    height_pct: 0.1,
                },
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
        assert!(
            matches!(&publishes[0].content, tze_hud_scene::types::ZoneContent::StreamText(s) if s == "## Status: OK")
        );
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
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 1.0,
                    height_pct: 0.1,
                },
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
        assert!(
            scene
                .zone_registry
                .active_publishes
                .contains_key(&zone_name)
        );
    }

    #[test]
    fn test_publish_to_zone_contention_policy_latest_wins() {
        // scene_with_zone creates a LatestWins zone; a second publish must replace
        // the first (single record in active_publishes after both calls).
        let (mut scene, _, zone) = scene_with_zone();
        handle_publish_to_zone(json!({"zone_name": zone, "content": "first"}), &mut scene).unwrap();
        handle_publish_to_zone(json!({"zone_name": zone, "content": "second"}), &mut scene)
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
        let err = handle_publish_to_zone(json!({"zone_name": zone, "content": ""}), &mut scene)
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
                    geometry_policy: GeometryPolicy::Relative {
                        x_pct: 0.0,
                        y_pct: 0.0,
                        width_pct: 1.0,
                        height_pct: 0.1,
                    },
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

    // ── Contention policy: Stack ─────────────────────────────────────────────

    /// Build a scene with a Stack zone (max_depth=3).
    fn scene_with_stack_zone() -> (SceneGraph, String) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone_name = "notif".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "Stack zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.75,
                    y_pct: 0.0,
                    width_pct: 0.25,
                    height_pct: 0.30,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::Stack { max_depth: 3 },
                max_publishers: 8,
                transport_constraint: None,
                auto_clear_ms: None,
                ephemeral: false,
                layer_attachment: LayerAttachment::Content,
            },
        );
        (scene, zone_name)
    }

    #[test]
    fn test_contention_stack_accumulates_records() {
        let (mut scene, zone) = scene_with_stack_zone();
        // Three publishes — all should accumulate in the stack.
        for i in 1..=3u32 {
            handle_publish_to_zone(
                json!({"zone_name": zone, "content": format!("msg-{i}"), "namespace": format!("agent-{i}")}),
                &mut scene,
            )
            .unwrap();
        }
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(
            publishes.len(),
            3,
            "Stack zone must accumulate all records up to max_depth"
        );
    }

    #[test]
    fn test_contention_stack_trims_oldest_when_max_depth_exceeded() {
        let (mut scene, zone) = scene_with_stack_zone();
        // Publish 4 items to a max_depth=3 stack (different namespaces to avoid publisher limit).
        for i in 1..=4u32 {
            handle_publish_to_zone(
                json!({"zone_name": zone, "content": format!("msg-{i}"), "namespace": format!("agent-{i}")}),
                &mut scene,
            )
            .unwrap();
        }
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(
            publishes.len(),
            3,
            "Stack must trim oldest when max_depth exceeded"
        );
        // Oldest (msg-1) should be gone; most recent (msg-4) should be present.
        assert!(
            publishes
                .iter()
                .all(|r| r.content
                    != tze_hud_scene::types::ZoneContent::StreamText("msg-1".to_string())),
            "oldest record must be evicted when stack overflows"
        );
        assert!(
            publishes
                .iter()
                .any(|r| r.content
                    == tze_hud_scene::types::ZoneContent::StreamText("msg-4".to_string())),
            "newest record must survive stack trim"
        );
    }

    #[test]
    fn test_contention_stack_no_tiles_created() {
        // Zone publishes must never create tiles directly — compositor handles rendering.
        let (mut scene, zone) = scene_with_stack_zone();
        for i in 1..=3u32 {
            handle_publish_to_zone(
                json!({"zone_name": zone, "content": format!("item-{i}"), "namespace": format!("ns-{i}")}),
                &mut scene,
            )
            .unwrap();
        }
        assert_eq!(
            scene.tile_count(),
            0,
            "Stack zone publishes must not create tiles"
        );
        assert_eq!(
            scene.node_count(),
            0,
            "Stack zone publishes must not create nodes"
        );
    }

    // ── Contention policy: Replace ───────────────────────────────────────────

    /// Build a scene with a Replace zone (single occupant).
    fn scene_with_replace_zone() -> (SceneGraph, String) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone_name = "pip".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "Replace zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 0.5,
                    height_pct: 0.5,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::Replace,
                max_publishers: 1,
                transport_constraint: None,
                auto_clear_ms: None,
                ephemeral: false,
                layer_attachment: LayerAttachment::Content,
            },
        );
        (scene, zone_name)
    }

    #[test]
    fn test_contention_replace_evicts_current_occupant() {
        let (mut scene, zone) = scene_with_replace_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "first-occupant", "namespace": "agent-a"}),
            &mut scene,
        )
        .unwrap();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "second-occupant", "namespace": "agent-b"}),
            &mut scene,
        )
        .unwrap();
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(
            publishes.len(),
            1,
            "Replace zone must hold exactly one record"
        );
        assert!(
            matches!(&publishes[0].content, tze_hud_scene::types::ZoneContent::StreamText(s) if s == "second-occupant"),
            "Replace must evict first and install second occupant"
        );
    }

    #[test]
    fn test_contention_replace_no_tiles_created() {
        let (mut scene, zone) = scene_with_replace_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "occupant", "namespace": "agent-x"}),
            &mut scene,
        )
        .unwrap();
        assert_eq!(
            scene.tile_count(),
            0,
            "Replace zone publishes must not create tiles"
        );
    }

    // ── Contention policy: MergeByKey ────────────────────────────────────────

    /// Build a scene with a MergeByKey zone (max_keys=4).
    fn scene_with_merge_zone() -> (SceneGraph, String) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone_name = "status".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "MergeByKey zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.95,
                    width_pct: 1.0,
                    height_pct: 0.05,
                },
                accepted_media_types: vec![ZoneMediaType::StreamText],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::MergeByKey { max_keys: 4 },
                max_publishers: 16,
                transport_constraint: None,
                auto_clear_ms: None,
                ephemeral: false,
                layer_attachment: LayerAttachment::Content,
            },
        );
        (scene, zone_name)
    }

    #[test]
    fn test_contention_merge_by_key_same_key_replaces() {
        let (mut scene, zone) = scene_with_merge_zone();
        // Publish twice with the same merge_key — must stay at 1 record.
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "v1", "merge_key": "cpu", "namespace": "agent-a"}),
            &mut scene,
        )
        .unwrap();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "v2", "merge_key": "cpu", "namespace": "agent-a"}),
            &mut scene,
        )
        .unwrap();
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(publishes.len(), 1, "Same merge_key must replace old record");
        assert!(
            matches!(&publishes[0].content, tze_hud_scene::types::ZoneContent::StreamText(s) if s == "v2"),
            "latest content must win for same merge_key"
        );
    }

    #[test]
    fn test_contention_merge_by_key_different_keys_coexist() {
        let (mut scene, zone) = scene_with_merge_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "cpu-data", "merge_key": "cpu", "namespace": "agent-a"}),
            &mut scene,
        )
        .unwrap();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "mem-data", "merge_key": "mem", "namespace": "agent-b"}),
            &mut scene,
        )
        .unwrap();
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(
            publishes.len(),
            2,
            "Different merge_keys must coexist in zone"
        );
    }

    #[test]
    fn test_contention_merge_by_key_max_keys_evicts_oldest() {
        // When a MergeByKey zone is at max_keys capacity, publishing a new
        // distinct key must succeed by evicting the oldest entry (index 0).
        // Spec: openspec/changes/exemplar-status-bar/tasks.md §2.5
        let (mut scene, zone) = scene_with_merge_zone(); // max_keys = 4
        // Fill all 4 key slots; key-0 is inserted first (oldest).
        for i in 0..4u32 {
            handle_publish_to_zone(
                json!({"zone_name": zone, "content": format!("val-{i}"), "merge_key": format!("key-{i}"), "namespace": format!("agent-{i}")}),
                &mut scene,
            )
            .unwrap();
        }
        // 5th distinct key must SUCCEED — evicting the oldest entry.
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "overflow", "merge_key": "key-overflow", "namespace": "agent-x"}),
            &mut scene,
        )
        .expect("5th key must succeed: oldest evicted, max_keys remain");

        // Zone must retain exactly max_keys (4) publications.
        let pubs = scene.zone_registry.active_for_zone(&zone);
        assert_eq!(
            pubs.len(),
            4,
            "zone must retain exactly 4 publications after eviction"
        );
        // The oldest key ("key-0") must have been evicted.
        assert!(
            !pubs.iter().any(|r| r.merge_key.as_deref() == Some("key-0")),
            "key-0 (oldest) must have been evicted"
        );
        // The new key must be present.
        assert!(
            pubs.iter()
                .any(|r| r.merge_key.as_deref() == Some("key-overflow")),
            "key-overflow must be present after eviction"
        );
    }

    #[test]
    fn test_contention_merge_by_key_no_tiles_created() {
        let (mut scene, zone) = scene_with_merge_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "data", "merge_key": "k1", "namespace": "agent-a"}),
            &mut scene,
        )
        .unwrap();
        assert_eq!(
            scene.tile_count(),
            0,
            "MergeByKey zone publishes must not create tiles"
        );
    }

    // ── Media-type rejection ─────────────────────────────────────────────────

    #[test]
    fn test_media_type_rejected_for_wrong_type_zone() {
        // Build a zone that only accepts ShortTextWithIcon (not StreamText).
        // A plain string content is parsed as StreamText, so this must be rejected.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone_name = "notif-only".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "Notification-only zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.75,
                    y_pct: 0.0,
                    width_pct: 0.25,
                    height_pct: 0.20,
                },
                accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::Stack { max_depth: 8 },
                max_publishers: 16,
                transport_constraint: None,
                auto_clear_ms: None,
                ephemeral: false,
                layer_attachment: LayerAttachment::Content,
            },
        );
        // A plain string produces ZoneContent::StreamText.
        // ShortTextWithIcon zone must reject it.
        let err = handle_publish_to_zone(
            json!({"zone_name": zone_name, "content": "hello"}),
            &mut scene,
        )
        .unwrap_err();
        assert!(
            matches!(err, McpError::SceneError(_)),
            "StreamText publish to ShortTextWithIcon-only zone must return SceneError (media type mismatch), got: {err:?}"
        );
    }

    #[test]
    fn test_media_type_accepted_for_matching_zone() {
        // StreamText zone must accept StreamText content.
        let (mut scene, _, zone) = scene_with_zone();
        let result = handle_publish_to_zone(
            json!({"zone_name": zone, "content": "valid stream text"}),
            &mut scene,
        );
        assert!(
            result.is_ok(),
            "StreamText content must be accepted by StreamText zone"
        );
    }

    // ── Occupancy reporting (list_zones has_content accuracy) ────────────────

    #[test]
    fn test_has_content_false_before_publish() {
        let (scene, _, zone) = scene_with_zone();
        let result = handle_list_zones(json!(null), &scene).unwrap();
        let entry = result.zones.iter().find(|z| z.name == zone).unwrap();
        assert!(
            !entry.has_content,
            "has_content must be false before any publish"
        );
    }

    #[test]
    fn test_has_content_true_after_publish() {
        let (mut scene, _, zone) = scene_with_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone.clone(), "content": "occupying content"}),
            &mut scene,
        )
        .unwrap();
        let result = handle_list_zones(json!(null), &scene).unwrap();
        let entry = result.zones.iter().find(|z| z.name == zone).unwrap();
        assert!(
            entry.has_content,
            "has_content must be true after successful publish"
        );
    }

    #[test]
    fn test_has_content_after_replace_policy_publish() {
        let (mut scene, zone) = scene_with_replace_zone();
        // Two publishes with Replace policy — one record remains; has_content = true.
        handle_publish_to_zone(
            json!({"zone_name": zone.clone(), "content": "first", "namespace": "a"}),
            &mut scene,
        )
        .unwrap();
        handle_publish_to_zone(
            json!({"zone_name": zone.clone(), "content": "second", "namespace": "b"}),
            &mut scene,
        )
        .unwrap();
        let result = handle_list_zones(json!(null), &scene).unwrap();
        let entry = result.zones.iter().find(|z| z.name == zone).unwrap();
        assert!(
            entry.has_content,
            "has_content must be true after Replace publish"
        );
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(
            publishes.len(),
            1,
            "Replace must maintain exactly one record"
        );
    }

    #[test]
    fn test_has_content_false_after_zone_cleared() {
        let (mut scene, _, zone) = scene_with_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone.clone(), "content": "something"}),
            &mut scene,
        )
        .unwrap();
        // Manually clear zone (as runtime would do on lease expiry / eviction).
        scene.clear_zone(&zone).unwrap();
        let result = handle_list_zones(json!(null), &scene).unwrap();
        let entry = result.zones.iter().find(|z| z.name == zone).unwrap();
        assert!(
            !entry.has_content,
            "has_content must be false after zone is cleared"
        );
    }

    #[test]
    fn test_has_content_stack_zone_reflects_occupancy() {
        let (mut scene, zone) = scene_with_stack_zone();
        // Stack with 2 items — has_content = true.
        for i in 1..=2u32 {
            handle_publish_to_zone(
                json!({"zone_name": zone.clone(), "content": format!("item-{i}"), "namespace": format!("agent-{i}")}),
                &mut scene,
            )
            .unwrap();
        }
        let result = handle_list_zones(json!(null), &scene).unwrap();
        let entry = result.zones.iter().find(|z| z.name == zone).unwrap();
        assert!(
            entry.has_content,
            "has_content must be true when Stack zone has entries"
        );
    }

    #[test]
    fn test_has_content_merge_zone_with_multiple_keys() {
        let (mut scene, zone) = scene_with_merge_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone.clone(), "content": "data-a", "merge_key": "alpha", "namespace": "agent-a"}),
            &mut scene,
        )
        .unwrap();
        handle_publish_to_zone(
            json!({"zone_name": zone.clone(), "content": "data-b", "merge_key": "beta", "namespace": "agent-b"}),
            &mut scene,
        )
        .unwrap();
        let result = handle_list_zones(json!(null), &scene).unwrap();
        let entry = result.zones.iter().find(|z| z.name == zone).unwrap();
        assert!(
            entry.has_content,
            "has_content must be true when MergeByKey zone has keyed records"
        );
    }

    // ── Expiry/cleanup after lease loss ─────────────────────────────────────

    #[test]
    fn test_zone_publishes_cleared_on_lease_revoke() {
        // Publish to a zone, then explicitly revoke the lease and verify that
        // active_publishes for this namespace are cleaned up.
        let (mut scene, _, zone) = scene_with_zone();

        // publish_to_zone grants a lease internally; but we need the lease_id to revoke.
        // Grant a lease manually, then use publish_to_zone (bypassing the MCP handler
        // since we need direct SceneGraph access to revoke the lease by ID).
        use tze_hud_scene::types::{Capability, ZoneContent};
        let ns = "agent-expiry";
        let lease_id = scene.grant_lease(ns, 60_000, vec![Capability::PublishZone(zone.clone())]);
        scene
            .publish_to_zone(
                &zone,
                ZoneContent::StreamText("expiring content".to_string()),
                ns,
                None,
                None,
                None,
            )
            .unwrap();

        // Confirm content is present
        assert!(
            scene
                .zone_registry
                .active_publishes
                .get(&zone)
                .is_some_and(|v| !v.is_empty()),
            "zone must have content before lease revoke"
        );

        // Revoke the lease — spec §Requirement: Lease Revocation Clears Zone Publications
        scene.revoke_lease(lease_id).unwrap();

        // All publications for this namespace must be gone.
        let remaining = scene.zone_registry.active_publishes.get(&zone);
        assert!(
            remaining.is_none_or(|v| v.is_empty()),
            "zone publications must be cleared when lease is revoked"
        );
    }

    #[test]
    fn test_list_zones_has_content_false_after_lease_revoke() {
        // After lease revoke, list_zones must report has_content = false.
        let (mut scene, _, zone) = scene_with_zone();
        use tze_hud_scene::types::{Capability, ZoneContent};
        let ns = "agent-expiry-2";
        let lease_id = scene.grant_lease(ns, 60_000, vec![Capability::PublishZone(zone.clone())]);
        scene
            .publish_to_zone(
                &zone,
                ZoneContent::StreamText("content".to_string()),
                ns,
                None,
                None,
                None,
            )
            .unwrap();
        scene.revoke_lease(lease_id).unwrap();

        let result = handle_list_zones(json!(null), &scene).unwrap();
        let entry = result.zones.iter().find(|z| z.name == zone).unwrap();
        assert!(
            !entry.has_content,
            "list_zones has_content must be false after lease revoke clears zone publications"
        );
    }

    #[test]
    fn test_zone_publish_fails_without_active_lease() {
        // After lease revoke, publish_to_zone_with_lease must fail.
        let (mut scene, _, zone) = scene_with_zone();
        use tze_hud_scene::types::{Capability, ZoneContent};
        let ns = "agent-gone";
        let lease_id = scene.grant_lease(ns, 60_000, vec![Capability::PublishZone(zone.clone())]);
        scene.revoke_lease(lease_id).unwrap();

        // Now publish_to_zone_with_lease must reject (no active lease).
        let result = scene.publish_to_zone_with_lease(
            &zone,
            ZoneContent::StreamText("should fail".to_string()),
            ns,
            None,
        );
        assert!(
            result.is_err(),
            "publish must be rejected after lease revoke"
        );
    }

    // ── Guest vs resident capability gates (additional coverage) ────────────

    #[test]
    fn test_publish_to_zone_is_guest_accessible() {
        // publish_to_zone is a guest tool; callers without resident_mcp must succeed.
        let (mut scene, _, zone) = scene_with_zone();
        let result = handle_publish_to_zone(
            json!({"zone_name": zone, "content": "guest-publish"}),
            &mut scene,
        );
        assert!(
            result.is_ok(),
            "publish_to_zone must be callable without resident_mcp capability"
        );
    }

    #[test]
    fn test_list_zones_is_guest_accessible() {
        // list_zones is a guest tool.
        let (scene, _, _) = scene_with_zone();
        let result = handle_list_zones(json!(null), &scene);
        assert!(
            result.is_ok(),
            "list_zones must be callable without resident_mcp capability"
        );
    }

    #[test]
    fn test_list_scene_is_guest_accessible() {
        // list_scene is a guest tool.
        let (scene, _, _) = scene_with_zone();
        let result = handle_list_scene(json!(null), &scene);
        assert!(
            result.is_ok(),
            "list_scene must be callable without resident_mcp capability"
        );
    }

    // ── No shortcut tile-creation path for zone publishing ───────────────────

    #[test]
    fn test_publish_to_zone_latest_wins_no_tiles() {
        // LatestWins zone must never create tiles or nodes.
        let (mut scene, _, zone) = scene_with_zone();
        handle_publish_to_zone(
            json!({"zone_name": zone, "content": "# HUD content"}),
            &mut scene,
        )
        .unwrap();
        assert_eq!(
            scene.tile_count(),
            0,
            "LatestWins publish must not create tiles"
        );
        assert_eq!(
            scene.node_count(),
            0,
            "LatestWins publish must not create nodes"
        );
    }

    #[test]
    fn test_publish_to_zone_second_publish_no_additional_tiles() {
        // Even repeated publishing must never accumulate tiles.
        let (mut scene, _, zone) = scene_with_zone();
        for i in 1..=5u32 {
            handle_publish_to_zone(
                json!({"zone_name": zone, "content": format!("update-{i}")}),
                &mut scene,
            )
            .unwrap();
        }
        assert_eq!(
            scene.tile_count(),
            0,
            "repeated publishes must never create tiles (compositor resolves zone → tile)"
        );
    }

    #[test]
    fn test_create_tile_does_not_bypass_zone_policy() {
        // create_tile is a resident tool that creates a raw tile, NOT a zone publish.
        // It should not interfere with zone occupancy: publishing to a zone after
        // creating a raw tile must still see tile_count=1, zone publishes=1 separately.
        let (mut scene, tab_id) = scene_with_tab();
        let zone_name = "z2".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "test zone".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 1.0,
                    height_pct: 0.1,
                },
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

        // Create a raw tile (resident operation — simulated here at SceneGraph level).
        use tze_hud_scene::types::{Capability, Rect};
        let lease_id = scene.grant_lease("resident-agent", 60_000, vec![Capability::CreateTile]);
        scene
            .create_tile(
                tab_id,
                "resident-agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 100.0),
                1,
            )
            .unwrap();
        assert_eq!(
            scene.tile_count(),
            1,
            "raw tile must exist after create_tile"
        );

        // Now publish to zone — must not add more tiles.
        handle_publish_to_zone(
            json!({"zone_name": zone_name, "content": "zone content"}),
            &mut scene,
        )
        .unwrap();

        assert_eq!(
            scene.tile_count(),
            1,
            "zone publish must not add extra tiles on top of existing raw tiles"
        );
        // Zone must have a record, but it's distinct from the raw tile.
        assert!(
            scene
                .zone_registry
                .active_publishes
                .get(&zone_name)
                .is_some_and(|v| !v.is_empty()),
            "zone publish record must be created independently of raw tile"
        );
    }

    // ── publish_to_widget ─────────────────────────────────────────────────────

    /// Build a scene pre-populated with a "gauge" widget type and instance.
    fn scene_with_widget() -> (SceneGraph, SceneId) {
        use tze_hud_scene::types::{
            ContentionPolicy as CP, GeometryPolicy, RenderingPolicy, WidgetDefinition,
            WidgetInstance, WidgetParamType, WidgetParameterDeclaration, WidgetParameterValue,
        };

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).expect("create tab");

        // Register "gauge" widget type with one f32 param "level".
        scene.widget_registry.register_definition(WidgetDefinition {
            id: "gauge".to_string(),
            name: "Gauge".to_string(),
            description: "Gauge widget".to_string(),
            parameter_schema: vec![WidgetParameterDeclaration {
                name: "level".to_string(),
                param_type: WidgetParamType::F32,
                default_value: WidgetParameterValue::F32(0.0),
                constraints: None,
            }],
            layers: vec![],
            default_geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 0.2,
                height_pct: 0.1,
            },
            default_rendering_policy: RenderingPolicy::default(),
            default_contention_policy: CP::LatestWins,
            ephemeral: false,
        });

        scene.widget_registry.register_instance(WidgetInstance {
            widget_type_name: "gauge".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "gauge".to_string(),
            current_params: std::collections::HashMap::new(),
        });

        (scene, tab_id)
    }

    #[test]
    fn test_publish_to_widget_missing_capability_rejected() {
        let (mut scene, _) = scene_with_widget();
        // No capabilities granted.
        let err = handle_publish_to_widget(
            json!({"widget_name": "gauge", "params": {"level": 0.5}}),
            &mut scene,
            &[],
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::SceneError(msg) if msg.contains("WIDGET_CAPABILITY_MISSING")),
            "expected WIDGET_CAPABILITY_MISSING, got: {err:?}"
        );
    }

    #[test]
    fn test_publish_to_widget_not_found() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:nonexistent".to_string()];
        let err = handle_publish_to_widget(
            json!({"widget_name": "nonexistent", "params": {}}),
            &mut scene,
            &caps,
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::SceneError(msg) if msg.contains("WIDGET_NOT_FOUND")),
            "expected WIDGET_NOT_FOUND, got: {err:?}"
        );
    }

    #[test]
    fn test_publish_to_widget_unknown_parameter() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];
        let err = handle_publish_to_widget(
            json!({"widget_name": "gauge", "params": {"bogus": 1.0}}),
            &mut scene,
            &caps,
        )
        .unwrap_err();
        // json_to_widget_param_value reports unknown param in its own message format.
        // handle_publish_to_widget does not run scene.publish_to_widget in this case
        // because the schema lookup fails first in json_to_widget_param_value.
        let msg = format!("{err:?}");
        assert!(
            msg.contains("WIDGET_UNKNOWN_PARAMETER") || msg.contains("not declared"),
            "expected unknown parameter error, got: {msg}"
        );
    }

    #[test]
    fn test_publish_to_widget_type_mismatch() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];
        // "level" is f32 but we pass a string.
        let err = handle_publish_to_widget(
            json!({"widget_name": "gauge", "params": {"level": "not-a-number"}}),
            &mut scene,
            &caps,
        )
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("f32") || msg.contains("number") || msg.contains("type"),
            "expected type mismatch error, got: {msg}"
        );
    }

    #[test]
    fn test_publish_to_widget_durable_succeeds() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];
        let result = handle_publish_to_widget(
            json!({"widget_name": "gauge", "params": {"level": 0.75}}),
            &mut scene,
            &caps,
        )
        .unwrap();
        assert_eq!(result.widget_name, "gauge");
        assert!(result.durable, "gauge is a durable widget type");
        assert!(result.applied_params.contains(&"level".to_string()));
    }

    #[test]
    fn test_publish_to_widget_empty_params_succeeds() {
        // An empty params map is valid — zero fields to update is fine.
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];
        let result = handle_publish_to_widget(
            json!({"widget_name": "gauge", "params": {}}),
            &mut scene,
            &caps,
        )
        .unwrap();
        assert_eq!(result.widget_name, "gauge");
        assert!(result.applied_params.is_empty());
    }

    #[test]
    fn test_publish_to_widget_empty_widget_name_rejected() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];
        let err =
            handle_publish_to_widget(json!({"widget_name": "", "params": {}}), &mut scene, &caps)
                .unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }

    // ── list_widgets ──────────────────────────────────────────────────────────

    #[test]
    fn test_list_widgets_empty_scene() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let result = handle_list_widgets(json!({}), &scene).unwrap();
        assert_eq!(result.type_count, 0);
        assert_eq!(result.instance_count, 0);
        assert!(result.widget_types.is_empty());
        assert!(result.widget_instances.is_empty());
    }

    #[test]
    fn test_list_widgets_returns_registered_type_and_instance() {
        let (scene, tab_id) = scene_with_widget();
        let result = handle_list_widgets(json!({}), &scene).unwrap();

        assert_eq!(result.type_count, 1);
        assert_eq!(result.instance_count, 1);

        let ty = &result.widget_types[0];
        assert_eq!(ty.id, "gauge");
        assert_eq!(ty.name, "Gauge");
        assert!(!ty.ephemeral);
        assert_eq!(ty.parameter_schema.len(), 1);
        assert_eq!(ty.parameter_schema[0].name, "level");
        assert_eq!(ty.parameter_schema[0].param_type, "f32");

        let inst = &result.widget_instances[0];
        assert_eq!(inst.instance_name, "gauge");
        assert_eq!(inst.widget_type, "gauge");
        assert_eq!(inst.tab_id, tab_id.to_string());
        assert!(inst.current_params.is_empty(), "no params published yet");
    }

    #[test]
    fn test_list_widgets_current_params_reflect_last_publish() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];
        handle_publish_to_widget(
            json!({"widget_name": "gauge", "params": {"level": 0.5}}),
            &mut scene,
            &caps,
        )
        .unwrap();

        let result = handle_list_widgets(json!({}), &scene).unwrap();
        let inst = &result.widget_instances[0];
        assert!(
            inst.current_params.contains_key("level"),
            "published param must appear in current_params"
        );
        // The f32 value 0.5 should be representable as a JSON number.
        let level_val = inst.current_params.get("level").unwrap();
        assert!(level_val.as_f64().is_some(), "level must be a JSON number");
    }

    #[test]
    fn test_list_widgets_is_guest_accessible() {
        // list_widgets must not require a resident capability — a guest-level
        // caller (no capabilities) must be able to call it without error.
        let (scene, _) = scene_with_widget();
        // No caller_capabilities needed — list_widgets takes no caps param.
        let result = handle_list_widgets(json!(null), &scene).unwrap();
        // Succeeds and returns data — confirms guest access works.
        assert_eq!(result.type_count, 1);
    }

    #[test]
    fn test_publish_to_widget_is_capability_gated() {
        // publish_to_widget without the right capability → WIDGET_CAPABILITY_MISSING.
        // Confirms the tool performs its own gate and does not require a separate
        // outer access check (i.e., the guest classification is correct: the tool
        // handles its own capability gating internally).
        let (mut scene, _) = scene_with_widget();
        let err = handle_publish_to_widget(
            json!({"widget_name": "gauge", "params": {"level": 0.5}}),
            &mut scene,
            &[], // no capabilities
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::SceneError(m) if m.contains("WIDGET_CAPABILITY_MISSING")),
            "expected WIDGET_CAPABILITY_MISSING, got: {err:?}"
        );
    }

    // ── clear_widget ──────────────────────────────────────────────────────────

    #[test]
    fn test_clear_widget_removes_own_publications() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];

        // Publish first
        handle_publish_to_widget(
            json!({"widget_name": "gauge", "namespace": "agent.a", "params": {"level": 0.8}}),
            &mut scene,
            &caps,
        )
        .unwrap();
        assert_eq!(scene.widget_registry.active_for_widget("gauge").len(), 1);

        // Clear
        let result = handle_clear_widget(
            json!({"widget_name": "gauge", "namespace": "agent.a"}),
            &mut scene,
            &caps,
        )
        .unwrap();
        assert_eq!(result.widget_name, "gauge");
        assert!(result.cleared);
        assert_eq!(
            scene.widget_registry.active_for_widget("gauge").len(),
            0,
            "agent.a's publication should be cleared"
        );
    }

    #[test]
    fn test_clear_widget_missing_capability_rejected() {
        let (mut scene, _) = scene_with_widget();
        let err = handle_clear_widget(
            json!({"widget_name": "gauge", "namespace": "agent.a"}),
            &mut scene,
            &[], // no capabilities
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::SceneError(m) if m.contains("WIDGET_CAPABILITY_MISSING")),
            "expected WIDGET_CAPABILITY_MISSING, got: {err:?}"
        );
    }

    #[test]
    fn test_clear_widget_not_found() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:nonexistent".to_string()];
        let err = handle_clear_widget(
            json!({"widget_name": "nonexistent", "namespace": "agent.a"}),
            &mut scene,
            &caps,
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::SceneError(m) if m.contains("WIDGET_NOT_FOUND")),
            "expected WIDGET_NOT_FOUND, got: {err:?}"
        );
    }

    #[test]
    fn test_clear_widget_empty_name_rejected() {
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];
        let err = handle_clear_widget(
            json!({"widget_name": "", "namespace": "agent.a"}),
            &mut scene,
            &caps,
        )
        .unwrap_err();
        assert!(matches!(err, McpError::InvalidParams(_)));
    }

    #[test]
    fn test_clear_widget_noop_when_no_publications() {
        // clear_widget with no prior publications should succeed silently.
        let (mut scene, _) = scene_with_widget();
        let caps = vec!["publish_widget:gauge".to_string()];
        let result = handle_clear_widget(
            json!({"widget_name": "gauge", "namespace": "agent.nobody"}),
            &mut scene,
            &caps,
        )
        .unwrap();
        assert!(result.cleared);
    }

    // ── parse_zone_content: static_image ────────────────────────────────────

    /// Build a scene with a zone that accepts StaticImage media type.
    fn scene_with_static_image_zone() -> (SceneGraph, String) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone_name = "pip".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "Picture-in-picture zone (accepts static images)".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 0.25,
                    height_pct: 0.25,
                },
                accepted_media_types: vec![ZoneMediaType::StaticImage],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::Replace,
                max_publishers: 1,
                transport_constraint: None,
                auto_clear_ms: None,
                ephemeral: false,
                layer_attachment: LayerAttachment::Content,
            },
        );
        (scene, zone_name)
    }

    #[test]
    fn test_parse_zone_content_static_image_valid_hex() {
        // A 64-char lowercase hex string must be accepted and produce StaticImage.
        let (mut scene, zone) = scene_with_static_image_zone();
        // blake3::hash(b"test") as a 64-char hex string.
        let hex = "4878ca0425c739fa427f7eda20fe845f6b2f46ba5fe5ac7d6b85add8db6bb08f"; // blake3 of "test"
        let result = handle_publish_to_zone(
            json!({"zone_name": zone, "content": {"type": "static_image", "resource_id": hex}}),
            &mut scene,
        )
        .unwrap();
        assert_eq!(result.zone_name, zone);
        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(publishes.len(), 1);
        // Verify the correct variant was produced and that the hex was decoded correctly.
        if let tze_hud_scene::types::ZoneContent::StaticImage(resource_id) = &publishes[0].content {
            let expected: [u8; 32] = [
                0x48, 0x78, 0xca, 0x04, 0x25, 0xc7, 0x39, 0xfa, 0x42, 0x7f, 0x7e, 0xda, 0x20, 0xfe,
                0x84, 0x5f, 0x6b, 0x2f, 0x46, 0xba, 0x5f, 0xe5, 0xac, 0x7d, 0x6b, 0x85, 0xad, 0xd8,
                0xdb, 0x6b, 0xb0, 0x8f,
            ];
            assert_eq!(
                resource_id.as_bytes(),
                &expected,
                "parsed ResourceId bytes must match expected value for provided hex"
            );
        } else {
            panic!(
                "static_image content must produce ZoneContent::StaticImage, got: {:?}",
                &publishes[0].content
            );
        }
    }

    #[test]
    fn test_parse_zone_content_static_image_uppercase_hex_accepted() {
        // Uppercase hex must also be accepted.
        let (mut scene, zone) = scene_with_static_image_zone();
        let hex = "4878CA0425C739FA427F7EDA20FE845F6B2F46BA5FE5AC7D6B85ADD8DB6BB08F";
        let result = handle_publish_to_zone(
            json!({"zone_name": zone, "content": {"type": "static_image", "resource_id": hex}}),
            &mut scene,
        );
        // Uppercase hex is valid (A-F are recognized by to_digit(16)).
        assert!(result.is_ok(), "uppercase hex must be accepted: {result:?}");
    }

    #[test]
    fn test_parse_zone_content_static_image_missing_resource_id_rejected() {
        // Missing resource_id field must return InvalidParams.
        let (mut scene, zone) = scene_with_static_image_zone();
        let err = handle_publish_to_zone(
            json!({"zone_name": zone, "content": {"type": "static_image"}}),
            &mut scene,
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::InvalidParams(msg) if msg.contains("resource_id")),
            "expected InvalidParams about resource_id, got: {err:?}"
        );
    }

    #[test]
    fn test_parse_zone_content_static_image_wrong_length_rejected() {
        // A hex string that is not exactly 64 chars must return InvalidParams.
        let (mut scene, zone) = scene_with_static_image_zone();
        let err = handle_publish_to_zone(
            json!({"zone_name": zone, "content": {"type": "static_image", "resource_id": "deadbeef"}}),
            &mut scene,
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::InvalidParams(msg) if msg.contains("64 hex chars") || msg.contains("64")),
            "expected InvalidParams about length, got: {err:?}"
        );
    }

    #[test]
    fn test_parse_zone_content_static_image_invalid_hex_chars_rejected() {
        // Non-hex characters must return InvalidParams.
        let (mut scene, zone) = scene_with_static_image_zone();
        // 64 chars with "XXXX" at the end, which are not valid hex digits.
        let bad_hex = "4878ca0425c739fa427f7eda20fe845f6b2f46ba5fe5ac7d6b85add8db6bXXXX";
        let err = handle_publish_to_zone(
            json!({"zone_name": zone, "content": {"type": "static_image", "resource_id": bad_hex}}),
            &mut scene,
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::InvalidParams(msg) if msg.contains("not valid hex") || msg.contains("hex")),
            "expected InvalidParams about invalid hex, got: {err:?}"
        );
    }

    #[test]
    fn test_parse_zone_content_unknown_type_error_includes_static_image() {
        // The error message for unknown content type must list static_image as valid.
        let (mut scene, zone) = scene_with_static_image_zone();
        let err = handle_publish_to_zone(
            json!({"zone_name": zone, "content": {"type": "bogus_type"}}),
            &mut scene,
        )
        .unwrap_err();
        assert!(
            matches!(&err, McpError::InvalidParams(msg) if msg.contains("static_image")),
            "error for unknown type must mention static_image, got: {err:?}"
        );
    }

    // ── Notification stack exemplar — MCP integration tests ─────────────────
    //
    // These 5 tests exercise the notification-area zone (max_depth=5,
    // auto_clear_ms=8000, Stack contention policy) via the MCP
    // `publish_to_zone` path, verifying the scenarios required by
    // openspec/changes/exemplar-notification/specs/exemplar-notification/spec.md
    // §Requirement: Notification Exemplar MCP Integration Test.

    /// Build a scene with the canonical notification-area zone (max_depth=5,
    /// ShortTextWithIcon, Stack contention, auto_clear_ms=8000).
    fn scene_with_notification_area() -> (SceneGraph, String) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone_name = "notification-area".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "Notification overlay area".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.75,
                    y_pct: 0.02,
                    width_pct: 0.24,
                    height_pct: 0.30,
                },
                accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::Stack { max_depth: 5 },
                max_publishers: 16,
                transport_constraint: None,
                auto_clear_ms: Some(8_000),
                ephemeral: false,
                layer_attachment: LayerAttachment::Chrome,
            },
        );
        (scene, zone_name)
    }

    /// Build a notification-area-backed scene using an injectable TestClock.
    ///
    /// The TestClock allows deterministic TTL expiry tests without real sleeps.
    fn scene_with_notification_area_and_clock() -> (SceneGraph, tze_hud_scene::TestClock, String) {
        use std::sync::Arc;
        let clock = tze_hud_scene::TestClock::new(1_000); // start at 1 second
        let mut scene = SceneGraph::new_with_clock(1920.0, 1080.0, Arc::new(clock.clone()));
        let zone_name = "notification-area".to_string();
        scene.zone_registry.zones.insert(
            zone_name.clone(),
            ZoneDefinition {
                id: SceneId::new(),
                name: zone_name.clone(),
                description: "Notification overlay area".to_string(),
                geometry_policy: GeometryPolicy::Relative {
                    x_pct: 0.75,
                    y_pct: 0.02,
                    width_pct: 0.24,
                    height_pct: 0.30,
                },
                accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
                rendering_policy: RenderingPolicy::default(),
                contention_policy: ContentionPolicy::Stack { max_depth: 5 },
                max_publishers: 16,
                transport_constraint: None,
                auto_clear_ms: Some(8_000),
                ephemeral: false,
                layer_attachment: LayerAttachment::Chrome,
            },
        );
        (scene, clock, zone_name)
    }

    /// Test 1: Multi-agent stack ordering.
    ///
    /// Three agents publish notifications to the notification-area zone.
    /// Verifies that:
    /// - All 3 records accumulate in active_publishes (Stack policy).
    /// - Arrival order is preserved: alpha first (index 0), then beta, then gamma (index 2, newest).
    /// - The newest publication (gamma) is at the end of the Vec (rendered at top per spec).
    #[test]
    fn test_notification_stack_multi_agent_arrival_order() {
        let (mut scene, zone) = scene_with_notification_area();

        // Three agents publish in order: alpha → beta → gamma.
        handle_publish_to_zone(
            json!({
                "zone_name": zone,
                "namespace": "alpha",
                "content": {"type": "notification", "text": "System idle", "icon": "", "urgency": 0}
            }),
            &mut scene,
        )
        .unwrap();
        handle_publish_to_zone(
            json!({
                "zone_name": zone,
                "namespace": "beta",
                "content": {"type": "notification", "text": "Update available", "icon": "update", "urgency": 1}
            }),
            &mut scene,
        )
        .unwrap();
        handle_publish_to_zone(
            json!({
                "zone_name": zone,
                "namespace": "gamma",
                "content": {"type": "notification", "text": "Security alert", "icon": "shield", "urgency": 3}
            }),
            &mut scene,
        )
        .unwrap();

        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(
            publishes.len(),
            3,
            "all 3 agent notifications must be present in the stack"
        );

        // Slot assignment: oldest at index 0, newest at index 2 (rendered at top).
        // Spec §Three notifications stack vertically newest-on-top: gamma (newest) at top.
        if let tze_hud_scene::types::ZoneContent::Notification(n) = &publishes[0].content {
            assert_eq!(
                n.text, "System idle",
                "alpha (oldest) must be at slot index 0"
            );
        } else {
            panic!("expected Notification at index 0, got {:?}", &publishes[0].content);
        }
        if let tze_hud_scene::types::ZoneContent::Notification(n) = &publishes[2].content {
            assert_eq!(
                n.text, "Security alert",
                "gamma (newest) must be at slot index 2"
            );
        } else {
            panic!("expected Notification at index 2, got {:?}", &publishes[2].content);
        }

        // Publisher namespaces must reflect each agent's identity.
        assert_eq!(publishes[0].publisher_namespace, "alpha");
        assert_eq!(publishes[1].publisher_namespace, "beta");
        assert_eq!(publishes[2].publisher_namespace, "gamma");
    }

    /// Test 2: Max depth eviction.
    ///
    /// Publishing 6 notifications to a max_depth=5 zone must evict the oldest
    /// (first) record immediately with no fade-out, leaving exactly 5 records.
    /// Spec §Sixth notification evicts oldest and §Evicted notification has no fade-out.
    #[test]
    fn test_notification_stack_max_depth_eviction() {
        let (mut scene, zone) = scene_with_notification_area();

        // Publish 6 notifications from 6 distinct agents.
        for i in 1..=6u32 {
            handle_publish_to_zone(
                json!({
                    "zone_name": zone,
                    "namespace": format!("agent-{i}"),
                    "content": {
                        "type": "notification",
                        "text": format!("notification-{i}"),
                        "icon": "",
                        "urgency": 1
                    }
                }),
                &mut scene,
            )
            .unwrap();
        }

        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(
            publishes.len(),
            5,
            "max_depth=5: only 5 newest notifications must remain after 6th publish"
        );

        // The oldest (notification-1) must be evicted.
        let has_first = publishes.iter().any(|r| {
            matches!(&r.content, tze_hud_scene::types::ZoneContent::Notification(n) if n.text == "notification-1")
        });
        assert!(!has_first, "notification-1 (oldest) must be evicted");

        // The newest (notification-6) must be present at the end.
        let last = &publishes[4];
        if let tze_hud_scene::types::ZoneContent::Notification(n) = &last.content {
            assert_eq!(n.text, "notification-6", "notification-6 (newest) must be at end");
        } else {
            panic!("expected Notification at index 4, got {:?}", last.content);
        }

        // The 5 surviving records must be notifications 2 through 6.
        let texts: Vec<&str> = publishes
            .iter()
            .filter_map(|r| {
                if let tze_hud_scene::types::ZoneContent::Notification(n) = &r.content {
                    Some(n.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            texts,
            vec!["notification-2", "notification-3", "notification-4", "notification-5", "notification-6"],
            "surviving records must be 2-6 in arrival order"
        );
    }

    /// Test 3: TTL auto-dismiss.
    ///
    /// A notification published with urgency=0 receives an auto-dismiss expiry of
    /// NOTIFICATION_TTL_INFO_US (8 s) from the scene graph. Advancing the clock
    /// past that expiry and calling drain_expired_zone_publications must remove it.
    ///
    /// Spec §Notification auto-dismisses after 8 seconds and
    ///      §Notification removed after fade-out completes.
    #[test]
    fn test_notification_stack_ttl_auto_dismiss() {
        let (mut scene, clock, zone) = scene_with_notification_area_and_clock();

        // Publish a low-urgency notification (urgency=0 → 8s auto-dismiss).
        handle_publish_to_zone(
            json!({
                "zone_name": zone,
                "namespace": "agent-ttl",
                "content": {
                    "type": "notification",
                    "text": "Will expire",
                    "icon": "",
                    "urgency": 0
                }
            }),
            &mut scene,
        )
        .unwrap();

        // Confirm publication is present before expiry.
        let before = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(before.len(), 1, "notification must be present before TTL expires");
        assert!(
            before[0].expires_at_wall_us.is_some(),
            "urgency=0 notification must have an auto-dismiss expiry set"
        );

        // Advance clock to just before expiry — publication must still be present.
        clock.advance(SceneGraph::NOTIFICATION_TTL_INFO_US / 1_000 - 1);
        let removed_early = scene.drain_expired_zone_publications();
        assert_eq!(removed_early, 0, "publication must not be removed before TTL expires");
        let still_present = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(still_present.len(), 1, "notification must still be present 1ms before TTL");

        // Advance clock 2ms past expiry (now = 1000 + 8000 + 1ms = past TTL+fade).
        clock.advance(2);
        let removed = scene.drain_expired_zone_publications();
        assert_eq!(removed, 1, "one notification must be removed after TTL expires");

        // Zone must have no active publications.
        let after = scene.zone_registry.active_publishes.get(&zone);
        assert!(
            after.is_none() || after.unwrap().is_empty(),
            "notification-area must be empty after TTL expiry and drain"
        );
    }

    /// Test 4: Urgency backdrop colors.
    ///
    /// Publishes notifications with urgency 0, 2, and 3, and verifies that the
    /// urgency values are correctly preserved in active_publishes (the renderer
    /// maps urgency → backdrop color; the MCP/scene layer preserves urgency as-is).
    ///
    /// Also verifies that urgency=5 is stored in the payload without error (clamping
    /// to critical=3 is the compositor's responsibility at render time, not the scene
    /// graph's, so urgency=5 is accepted and stored unchanged).
    ///
    /// Spec §Urgency-Tinted Notification Backdrops and §Out-of-range urgency clamped to critical.
    #[test]
    fn test_notification_stack_urgency_backdrop_colors() {
        let (mut scene, zone) = scene_with_notification_area();

        // Publish urgency 0 (low → #2A2A2A backdrop), urgency 2 (urgent → #8B6914),
        // urgency 3 (critical → #8B1A1A), and urgency 5 (out-of-range → clamped to critical
        // at render time; stored as 5 in the record).
        for (ns, urgency, label) in &[
            ("agent-low", 0u32, "low"),
            ("agent-urgent", 2u32, "urgent"),
            ("agent-critical", 3u32, "critical"),
            ("agent-oob", 5u32, "out-of-range"),
        ] {
            handle_publish_to_zone(
                json!({
                    "zone_name": zone,
                    "namespace": ns,
                    "content": {
                        "type": "notification",
                        "text": format!("urgency-{label}"),
                        "icon": "",
                        "urgency": urgency
                    }
                }),
                &mut scene,
            )
            .unwrap_or_else(|e| panic!("publish urgency={urgency} failed: {e:?}"));
        }

        let publishes = scene.zone_registry.active_publishes.get(&zone).unwrap();
        assert_eq!(publishes.len(), 4, "all 4 urgency-level notifications must be present");

        // Verify each urgency value is stored unchanged in the payload.
        let urgency_by_ns: std::collections::HashMap<&str, u32> = publishes
            .iter()
            .filter_map(|r| {
                if let tze_hud_scene::types::ZoneContent::Notification(n) = &r.content {
                    Some((r.publisher_namespace.as_str(), n.urgency))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(urgency_by_ns["agent-low"], 0, "urgency=0 (low) must be preserved");
        assert_eq!(urgency_by_ns["agent-urgent"], 2, "urgency=2 (urgent) must be preserved");
        assert_eq!(urgency_by_ns["agent-critical"], 3, "urgency=3 (critical) must be preserved");
        // urgency=5 is stored as-is; the compositor clamps it to 3 at render time.
        assert_eq!(
            urgency_by_ns["agent-oob"],
            5,
            "urgency=5 (out-of-range) must be stored as-is in the record; compositor clamps to critical=3"
        );
    }

    /// Test 5: Agent independence.
    ///
    /// When one agent's notification TTL expires and is drained, the other agent's
    /// notification must remain unaffected in the stack.
    ///
    /// Spec §Agents do not interfere with each other's notifications.
    #[test]
    fn test_notification_stack_agent_independence() {
        let (mut scene, clock, zone) = scene_with_notification_area_and_clock();

        // Agent-alpha publishes urgency=0 (8s TTL → expires at now+8000ms).
        handle_publish_to_zone(
            json!({
                "zone_name": zone,
                "namespace": "agent-alpha",
                "content": {
                    "type": "notification",
                    "text": "Alpha message",
                    "icon": "",
                    "urgency": 0
                }
            }),
            &mut scene,
        )
        .unwrap();

        // Advance 1ms so beta's published_at_wall_us is distinct from alpha's.
        clock.advance(1);

        // Agent-beta publishes urgency=3 (critical → 30s TTL → expires at now+30000ms).
        handle_publish_to_zone(
            json!({
                "zone_name": zone,
                "namespace": "agent-beta",
                "content": {
                    "type": "notification",
                    "text": "Beta message",
                    "icon": "",
                    "urgency": 3
                }
            }),
            &mut scene,
        )
        .unwrap();

        // Both notifications must be present before any TTL expires.
        let before = scene.zone_registry.active_for_zone(&zone);
        assert_eq!(before.len(), 2, "both notifications must be active before any expiry");

        // Advance clock past alpha's 8s TTL (urgency=0 → NOTIFICATION_TTL_INFO_US).
        // Beta's 30s TTL (urgency=3 → NOTIFICATION_TTL_CRITICAL_US) must not have expired.
        clock.advance(SceneGraph::NOTIFICATION_TTL_INFO_US / 1_000 + 500); // +8500ms
        let removed = scene.drain_expired_zone_publications();
        assert_eq!(removed, 1, "only alpha's notification must be removed at t=8500ms");

        // Beta's notification must remain unaffected.
        let after = scene.zone_registry.active_for_zone(&zone);
        assert_eq!(after.len(), 1, "beta's notification must survive alpha's TTL expiry");
        if let tze_hud_scene::types::ZoneContent::Notification(n) = &after[0].content {
            assert_eq!(n.text, "Beta message", "surviving notification must be beta's");
            assert_eq!(
                after[0].publisher_namespace, "agent-beta",
                "surviving record must belong to agent-beta"
            );
        } else {
            panic!("expected Notification, got {:?}", after[0].content);
        }
    }
}
