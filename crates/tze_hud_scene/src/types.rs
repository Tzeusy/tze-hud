//! Core types for the scene graph, following RFC 0001.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── IDs ────────────────────────────────────────────────────────────────────

/// Scene object ID — UUIDv7 (time-ordered).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SceneId(Uuid);

impl SceneId {
    pub fn new() -> Self {
        SceneId(Uuid::now_v7())
    }

    /// Create from raw UUID (for testing / deserialization).
    pub fn from_uuid(uuid: Uuid) -> Self {
        SceneId(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Nil/zero ID used as "none" sentinel in protobuf.
    pub fn nil() -> Self {
        SceneId(Uuid::nil())
    }

    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
}

impl Default for SceneId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SceneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ─── Geometry ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    /// Check if this rect is fully contained within `outer`.
    pub fn is_within(&self, outer: &Rect) -> bool {
        self.x >= outer.x
            && self.y >= outer.y
            && self.x + self.width <= outer.x + outer.width
            && self.y + self.height <= outer.y + outer.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const TRANSPARENT: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Convert to [f32; 4] for GPU upload.
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

// ─── Enums ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputMode {
    Passthrough,
    Capture,
    LocalOnly,
}

impl Default for InputMode {
    fn default() -> Self {
        InputMode::Capture
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontFamily {
    SystemSansSerif,
    SystemMonospace,
    SystemSerif,
}

impl Default for FontFamily {
    fn default() -> Self {
        FontFamily::SystemSansSerif
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Start,
    Center,
    End,
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign::Start
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextOverflow {
    Clip,
    Ellipsis,
}

impl Default for TextOverflow {
    fn default() -> Self {
        TextOverflow::Clip
    }
}

// ─── Scene Objects ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: SceneId,
    pub name: String,
    pub display_order: u32,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceBudget {
    pub texture_bytes: u64,
    pub update_rate_hz: f32,
    pub max_nodes: u8,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            texture_bytes: 16 * 1024 * 1024, // 16 MiB
            update_rate_hz: 30.0,
            max_nodes: 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub id: SceneId,
    pub tab_id: SceneId,
    pub namespace: String,
    pub lease_id: SceneId,
    pub bounds: Rect,
    pub z_order: u32,
    pub opacity: f32,
    pub input_mode: InputMode,
    pub sync_group: Option<SceneId>,
    pub present_at: Option<u64>,
    pub expires_at: Option<u64>,
    pub resource_budget: ResourceBudget,
    pub root_node: Option<SceneId>,
}

// ─── Nodes ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: SceneId,
    pub children: Vec<SceneId>,
    pub data: NodeData,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeData {
    SolidColor(SolidColorNode),
    TextMarkdown(TextMarkdownNode),
    HitRegion(HitRegionNode),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolidColorNode {
    pub color: Rgba,
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextMarkdownNode {
    pub content: String,
    pub bounds: Rect,
    pub font_size_px: f32,
    pub font_family: FontFamily,
    pub color: Rgba,
    pub background: Option<Rgba>,
    pub alignment: TextAlign,
    pub overflow: TextOverflow,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitRegionNode {
    pub bounds: Rect,
    pub interaction_id: String,
    pub accepts_focus: bool,
    pub accepts_pointer: bool,
}

// ─── Hit Region Local State (compositor-managed) ────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitRegionLocalState {
    pub node_id: SceneId,
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
}

impl HitRegionLocalState {
    pub fn new(node_id: SceneId) -> Self {
        Self {
            node_id,
            hovered: false,
            pressed: false,
            focused: false,
        }
    }
}

// ─── Lease (minimal for vertical slice) ─────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lease {
    pub id: SceneId,
    pub namespace: String,
    pub granted_at_ms: u64,
    pub ttl_ms: u64,
    pub capabilities: Vec<Capability>,
    pub resource_budget: ResourceBudget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    CreateTile,
    UpdateTile,
    DeleteTile,
    CreateNode,
    UpdateNode,
    DeleteNode,
    ReceiveInput,
}

impl Lease {
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms > self.granted_at_ms + self.ttl_ms
    }

    pub fn has_capability(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Remaining TTL in milliseconds (0 if expired).
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        let expires = self.granted_at_ms + self.ttl_ms;
        expires.saturating_sub(now_ms)
    }
}

// ─── Zone (minimal, from config) ────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneDefinition {
    pub id: SceneId,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneRegistry {
    pub zones: HashMap<String, ZoneDefinition>,
}

impl ZoneRegistry {
    pub fn new() -> Self {
        Self {
            zones: HashMap::new(),
        }
    }
}

impl Default for ZoneRegistry {
    fn default() -> Self {
        Self::new()
    }
}
