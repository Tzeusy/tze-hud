//! Core types for the scene graph, following RFC 0001.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── IDs ────────────────────────────────────────────────────────────────────

/// Scene object ID — UUIDv7 (time-ordered, 16 bytes).
///
/// # Wire format
/// Serialized as 16 raw bytes in little-endian UUID byte order (as returned by
/// [`Uuid::to_bytes_le`]). The all-zero value (`[0u8; 16]`) is the null/absent
/// sentinel per RFC 0001 §1.1.
///
/// # Invariants
/// - `size_of::<SceneId>() == 16`
/// - Lexicographic sort order == creation-time order (UUIDv7 property)
/// - `SceneId::null().is_null() == true`
/// - `SceneId::new().is_null() == false` (freshly-generated IDs are never null)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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

    /// Null/zero ID used as the "absent" sentinel (RFC 0001 §1.1).
    ///
    /// When encoded as a protobuf `bytes` field, this value serializes to 16
    /// zero bytes. Note that in proto3 an unset `bytes` field defaults to an
    /// empty vector (length 0), not 16 zero bytes; callers must explicitly
    /// handle the empty-bytes case (e.g., `proto_to_scene_id` returns `None`
    /// for empty input) and decide whether to treat it as this sentinel.
    pub fn null() -> Self {
        SceneId(Uuid::nil())
    }

    /// Returns `true` if this is the null/absent sentinel (`[0u8; 16]`).
    pub fn is_null(&self) -> bool {
        self.0.is_nil()
    }

    /// Nil/zero ID used as "none" sentinel in protobuf.
    ///
    /// Alias for [`Self::null`]; prefer `null()`/`is_null()` in new code.
    #[inline]
    pub fn nil() -> Self {
        Self::null()
    }

    /// Returns `true` if this is the nil/zero sentinel.
    ///
    /// Alias for [`Self::is_null`]; prefer `is_null()` in new code.
    #[inline]
    pub fn is_nil(&self) -> bool {
        self.is_null()
    }

    /// Serialize to 16 bytes in little-endian UUID byte order.
    ///
    /// Used for protobuf `bytes` fields. The encoding is stable and matches
    /// the wire contract from RFC 0001 §4.1.
    pub fn to_bytes_le(&self) -> [u8; 16] {
        self.0.to_bytes_le()
    }

    /// Deserialize from 16 bytes in little-endian UUID byte order.
    ///
    /// Returns `None` if the slice is not exactly 16 bytes.
    pub fn from_bytes_le(bytes: &[u8]) -> Option<Self> {
        let arr: [u8; 16] = bytes.try_into().ok()?;
        Some(SceneId(Uuid::from_bytes_le(arr)))
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

// ─── ResourceId ──────────────────────────────────────────────────────────────

/// Content-addressed resource identity — 32-byte BLAKE3 hash.
///
/// Two agents uploading identical content MUST receive the same `ResourceId`;
/// the runtime stores the resource once (RFC 0001 §1.1).
///
/// # Wire format
/// Stored and transmitted as raw 32 bytes. Hex encoding is a display/debug
/// concern only and MUST NOT appear on the wire or in storage.
///
/// # Invariants
/// - `size_of::<ResourceId>() == 32`
/// - Equality is byte equality — no normalisation
/// - `ResourceId::of(bytes) == ResourceId::of(same_bytes)` always
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResourceId([u8; 32]);

impl ResourceId {
    /// Compute the `ResourceId` for a byte payload using BLAKE3.
    pub fn of(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        ResourceId(*hash.as_bytes())
    }

    /// Wrap a raw 32-byte array directly (for deserialization / testing).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        ResourceId(bytes)
    }

    /// Return the raw 32-byte hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Try to construct from a byte slice.
    ///
    /// Returns `None` if the slice is not exactly 32 bytes.
    pub fn from_slice(slice: &[u8]) -> Option<Self> {
        let arr: [u8; 32] = slice.try_into().ok()?;
        Some(ResourceId(arr))
    }

    /// Return a lowercase hex string for display / logging only.
    ///
    /// MUST NOT be used on the wire or in storage.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
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

/// How image content is fitted within the node's bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFitMode {
    /// Scale uniformly so the entire image is visible; may leave letterbox bars.
    Contain,
    /// Scale uniformly to cover the entire bounds; may crop the image.
    Cover,
    /// Stretch non-uniformly to fill bounds exactly.
    Fill,
    /// Like Contain but never scale up; display at native size if smaller than bounds.
    ScaleDown,
}

impl Default for ImageFitMode {
    fn default() -> Self {
        ImageFitMode::Contain
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputMode {
    Passthrough,
    #[default]
    Capture,
    LocalOnly,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontFamily {
    #[default]
    SystemSansSerif,
    SystemMonospace,
    SystemSerif,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

// ─── Scene Objects ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: SceneId,
    pub name: String,
    pub display_order: u32,
    pub created_at_ms: u64,
    /// Optional bare event name that triggers automatic tab activation.
    ///
    /// Per scene-events/spec.md §9.1–§9.4 (Requirement: tab_switch_on_event Contract):
    /// - Names a scene-level event that triggers automatic activation of this tab.
    /// - Agent events match against the bare name (before namespace prefixing) for
    ///   agent-independence: "doorbell.ring" fires for ANY agent emitting "doorbell.ring".
    /// - System events (system.* prefix) are excluded from matching.
    /// - Triggered switch is subject to attention filtering (quiet hours, attention budget).
    /// - Successful switch generates ActiveTabChangedEvent (event_type "scene.tab.active_changed").
    ///
    /// Set to `None` to disable event-triggered tab switching for this tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_switch_on_event: Option<String>,
}

/// Per-agent resource envelope enforced by the budget enforcement ladder.
///
/// All four dimensions are checked at mutation intake (Stage 3). A mutation batch
/// that would push any dimension over budget is rejected whole (all-or-nothing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceBudget {
    /// Maximum number of tiles this agent may hold simultaneously.
    pub max_tiles: u32,
    /// Maximum texture memory across all tiles, in bytes.
    pub max_texture_bytes: u64,
    /// Maximum scene mutation rate in Hz (sliding window over last 1 second).
    pub max_update_rate_hz: f32,
    /// Maximum nodes per individual tile.
    pub max_nodes_per_tile: u32,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_tiles: 8,
            max_texture_bytes: 256 * 1024 * 1024, // 256 MiB
            max_update_rate_hz: 30.0,
            max_nodes_per_tile: 32,
        }
    }
}

/// A dimension in which an agent has violated its resource budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BudgetViolation {
    /// Agent holds more tiles than `max_tiles`.
    TileCountExceeded {
        current: u32,
        limit: u32,
    },
    /// Texture memory across all tiles exceeds `max_texture_bytes`.
    TextureMemoryExceeded {
        current_bytes: u64,
        limit_bytes: u64,
    },
    /// Scene mutation rate exceeds `max_update_rate_hz`.
    UpdateRateExceeded {
        current_hz: f32,
        limit_hz: f32,
    },
    /// A single tile contains more nodes than `max_nodes_per_tile`.
    NodeCountPerTileExceeded {
        tile_id_hint: String,
        current: u32,
        limit: u32,
    },
    /// Mutation would push texture memory past the absolute hard maximum.
    /// This is a critical violation — session is revoked immediately.
    CriticalTextureOomAttempt {
        requested_bytes: u64,
        hard_max_bytes: u64,
    },
    /// Session has accumulated too many protocol invariant violations.
    /// This is a critical violation — session is revoked immediately.
    RepeatedInvariantViolations {
        count: u32,
    },
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
    /// Visual overlay hint for the compositor.
    ///
    /// Set by the scene graph in response to lease state changes.
    /// The compositor renders the indicated badge/overlay within 1 frame
    /// of this field being set (spec line 133).
    #[serde(default)]
    pub visual_hint: crate::lease::TileVisualHint,
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
    StaticImage(StaticImageNode),
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

/// Cursor style hint forwarded to the host OS pointer layer.
///
/// The runtime updates the system cursor to the resolved style whenever
/// the pointer hovers over a HitRegionNode — no agent roundtrip required.
/// Source: RFC 0004 §7.1, input-model/spec.md line 249.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorStyle {
    /// Platform default arrow cursor.
    #[default]
    Default,
    /// Text-insertion I-beam.
    Text,
    /// Pointer (pointing hand) — conventional for links/buttons.
    Pointer,
    /// Move cursor (four-directional arrows).
    Move,
    /// Crosshair for precision targeting.
    Crosshair,
    /// Grab (open hand).
    Grab,
    /// Grabbing (closed hand).
    Grabbing,
    /// Not-allowed / forbidden.
    NotAllowed,
    /// Resize — north-south.
    ResizeNS,
    /// Resize — east-west.
    ResizeEW,
    /// Resize — northwest-southeast diagonal.
    ResizeNWSE,
    /// Resize — northeast-southwest diagonal.
    ResizeNESW,
}

/// Event delivery filter mask for a HitRegionNode.
///
/// **Data model only in v1.** This struct carries the mask values set by the
/// owning agent; the filtering logic (suppressing event delivery when a flag is
/// `false`) is implemented in the input-dispatch layer (input model epic,
/// post-v1 or separate bead).  Until that layer is wired, all event types
/// reach the agent regardless of this mask.
///
/// When the filtering layer is active, a flag of `false` suppresses the
/// corresponding event type before it reaches the owning agent's EventBatch,
/// saving agent bandwidth.  All flags default to `true`.
///
/// The runtime still performs hit-testing and local-state updates regardless
/// of mask values.  `event_mask` controls agent delivery only — it is never
/// consulted by the hit-test spatial query.
///
/// Source: RFC 0004 §7.1, input-model/spec.md lines 249, 253-255.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMask {
    /// Deliver PointerDownEvent to the owning agent.
    pub pointer_down: bool,
    /// Deliver PointerUpEvent to the owning agent.
    pub pointer_up: bool,
    /// Deliver PointerMoveEvent to the owning agent.
    pub pointer_move: bool,
    /// Deliver PointerEnterEvent to the owning agent.
    pub pointer_enter: bool,
    /// Deliver PointerLeaveEvent to the owning agent.
    pub pointer_leave: bool,
    /// Deliver ClickEvent to the owning agent.
    pub click: bool,
    /// Deliver DoubleClickEvent to the owning agent.
    pub double_click: bool,
    /// Deliver ContextMenuEvent to the owning agent.
    pub context_menu: bool,
    /// Deliver KeyDownEvent / KeyUpEvent / CharacterEvent to the owning agent.
    pub keyboard: bool,
}

impl Default for EventMask {
    /// All event types delivered by default.
    fn default() -> Self {
        Self {
            pointer_down: true,
            pointer_up: true,
            pointer_move: true,
            pointer_enter: true,
            pointer_leave: true,
            click: true,
            double_click: true,
            context_menu: true,
            keyboard: true,
        }
    }
}

/// ARIA-compatible accessibility metadata for a HitRegionNode.
///
/// Enables screen readers and assistive technologies to understand the
/// interactive element's role, label, and state.
/// Source: RFC 0004 §7.1, input-model/spec.md line 249.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityMeta {
    /// ARIA role string (e.g., "button", "link", "checkbox").
    /// Empty string means no explicit role — inferred from context.
    pub role: String,
    /// Accessible label.  Used by screen readers as the element's name.
    pub label: String,
    /// ARIA description (longer contextual hint, supplemental to label).
    pub description: String,
    /// Whether the element is currently disabled from an accessibility standpoint.
    pub disabled: bool,
}

/// Per-node visual style overrides applied locally without an agent roundtrip.
///
/// These are compositor-managed overrides — the agent provides the values; the
/// compositor applies them in real time based on `HitRegionLocalState`.
/// Source: RFC 0004 §7.1, input-model/spec.md line 249.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalStyle {
    /// Highlight tint applied while the pointer hovers over the node.
    /// `None` means no hover highlight.
    pub hover_tint: Option<Rgba>,
    /// Highlight tint applied while the node is pressed / active.
    pub pressed_tint: Option<Rgba>,
    /// Highlight tint applied while the node has keyboard focus.
    pub focus_outline_color: Option<Rgba>,
}

/// Hit-test spatial query result.
///
/// Returned by [`SceneGraph::hit_test`].  Represents the outcome of mapping a
/// 2D display-coordinate point to the deepest interactive scene element per the
/// traversal contract:
///
/// 1. Chrome layer tiles (lease priority 0) checked first.
/// 2. Content layer tiles in z-order descending; passthrough tiles skipped.
/// 3. Within each tile, nodes in reverse tree order (last sibling first, depth-first).
/// 4. Only [`NodeData::HitRegion`] nodes whose `bounds` contain the point qualify.
///
/// Source: RFC 0001 §5.1-5.2, scene-graph/spec.md lines 250-265,
///         input-model/spec.md lines 263-274.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitResult {
    /// A [`NodeData::HitRegion`] node was hit.
    ///
    /// This is the most specific result: a named interactive region on a tile
    /// accepted the point.
    NodeHit {
        /// The tile that owns the hit node.
        tile_id: SceneId,
        /// The `HitRegionNode`'s scene ID.
        node_id: SceneId,
        /// The `interaction_id` string from the `HitRegionNode` — forwarded in
        /// all events dispatched from this hit.
        interaction_id: String,
    },
    /// A tile was hit but no `HitRegionNode` within it accepted the point.
    ///
    /// The tile itself absorbs the event (input_mode != Passthrough).
    TileHit {
        /// The tile that absorbed the point.
        tile_id: SceneId,
    },
    /// The point landed on a passthrough tile (or all tiles are passthrough at
    /// this coordinate).
    ///
    /// The event should be forwarded to the desktop in overlay mode or discarded
    /// in fullscreen.
    Passthrough,
    /// The point hit a chrome-layer element (lease priority 0).
    ///
    /// Chrome always wins; no content-layer tile receives the event.
    Chrome {
        /// The scene ID of the chrome tile (or node, for chrome HitRegionNodes).
        element_id: SceneId,
    },
}

/// HitRegionNode is the sole interactive primitive in v1.
///
/// It defines a rectangular interactive region within a tile.  The runtime
/// performs hit-testing against `bounds` (tile-local coordinates) during
/// Stage 2 of the input dispatch pipeline.
///
/// # Local feedback
/// The runtime updates `HitRegionLocalState` (`hovered`, `pressed`) immediately
/// on hit, without waiting for the owning agent to acknowledge.  This satisfies
/// the "local feedback first" doctrine.
///
/// # Event filtering
/// `event_mask` controls which event types are forwarded to the owning agent.
/// The runtime always performs the spatial query and local-state update;
/// `event_mask` only suppresses agent delivery.
///
/// Source: RFC 0004 §7.1, RFC 0001 §2.4, input-model/spec.md lines 248-259.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitRegionNode {
    /// Bounding rectangle in tile-local coordinates (origin = tile top-left).
    pub bounds: Rect,
    /// Agent-defined interaction identifier forwarded in all events from this node.
    /// Must be non-empty; the runtime treats an empty string as "unnamed".
    pub interaction_id: String,
    /// Whether this node participates in keyboard focus cycling.
    pub accepts_focus: bool,
    /// Whether this node accepts pointer events (hit-test only qualifies nodes
    /// where `accepts_pointer = true`).
    pub accepts_pointer: bool,
    /// When `true`, the runtime automatically acquires pointer capture for this
    /// node on PointerDownEvent without requiring an explicit CaptureRequest.
    /// Source: RFC 0004 §7.1 / input-model/spec.md line 142.
    #[serde(default)]
    pub auto_capture: bool,
    /// When `true`, pointer capture is released automatically on PointerUpEvent.
    /// Source: RFC 0004 §7.1 / input-model/spec.md line 120.
    #[serde(default)]
    pub release_on_up: bool,
    /// Cursor style hint shown while the pointer hovers over this node.
    #[serde(default)]
    pub cursor_style: CursorStyle,
    /// Tooltip text shown after the pointer has hovered for 500 ms.
    /// `None` means no tooltip.
    #[serde(default)]
    pub tooltip: Option<String>,
    /// Per-event-type delivery filter.  All events enabled by default.
    #[serde(default)]
    pub event_mask: EventMask,
    /// ARIA-compatible accessibility metadata.
    ///
    /// Boxed to keep `HitRegionNode` (and therefore `Node`) within the
    /// 150-byte struct budget (scene-graph/spec.md line 302, RFC 0001 §8).
    /// `AccessibilityMeta` is zero by default; boxing costs only 8 bytes when empty.
    #[serde(default)]
    pub accessibility: Box<AccessibilityMeta>,
    /// Compositor-applied visual style overrides for hover/press/focus states.
    ///
    /// Boxed to stay within the 150-byte `Node` struct budget
    /// (scene-graph/spec.md line 302, RFC 0001 §8).
    #[serde(default)]
    pub local_style: Box<LocalStyle>,
}

impl HitResult {
    /// Returns `true` if the point hit any interactive element (node, tile, or chrome).
    ///
    /// Returns `false` only for [`HitResult::Passthrough`].
    pub fn is_some(&self) -> bool {
        !matches!(self, HitResult::Passthrough)
    }

    /// Returns `true` if the point did not hit any interactive element.
    ///
    /// Equivalent to `!self.is_some()`.
    pub fn is_none(&self) -> bool {
        matches!(self, HitResult::Passthrough)
    }

    /// Returns `true` if this is a [`HitResult::NodeHit`].
    pub fn is_node_hit(&self) -> bool {
        matches!(self, HitResult::NodeHit { .. })
    }

    /// Returns `true` if this is a [`HitResult::Chrome`] hit.
    pub fn is_chrome(&self) -> bool {
        matches!(self, HitResult::Chrome { .. })
    }

    /// Extract the `(tile_id, node_id)` pair for `NodeHit` results.
    ///
    /// Returns `None` for all other variants.
    pub fn node_hit_ids(&self) -> Option<(SceneId, SceneId)> {
        if let HitResult::NodeHit { tile_id, node_id, .. } = self {
            Some((*tile_id, *node_id))
        } else {
            None
        }
    }

    /// Extract the tile_id for `NodeHit` or `TileHit` results.
    ///
    /// Returns `None` for `Chrome` and `Passthrough`.
    pub fn tile_id(&self) -> Option<SceneId> {
        match self {
            HitResult::NodeHit { tile_id, .. } | HitResult::TileHit { tile_id } => {
                Some(*tile_id)
            }
            _ => None,
        }
    }
}

impl Default for HitRegionNode {
    fn default() -> Self {
        Self {
            bounds: Rect::new(0.0, 0.0, 0.0, 0.0),
            interaction_id: String::new(),
            accepts_focus: false,
            accepts_pointer: false,
            auto_capture: false,
            release_on_up: false,
            cursor_style: CursorStyle::Default,
            tooltip: None,
            event_mask: EventMask::default(),
            accessibility: Box::default(),
            local_style: Box::default(),
        }
    }
}

/// A static image node that references a resource by its content-addressed identity.
///
/// Per resource-store/spec.md §Requirement: Ephemeral Storage in V1 (lines 244-246),
/// scene snapshots reference resources by `ResourceId` only — blob data is NOT
/// embedded.  On restart, the resource store is empty; agents must re-upload
/// referenced resources before the scene can fully render.
///
/// The `decoded_bytes` field is set at mutation time (from the resource store
/// record) so that the scene graph can enforce texture budget limits without
/// holding raw pixel data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaticImageNode {
    /// Content-addressed identity of the image resource (BLAKE3 hash of raw bytes).
    ///
    /// This is the only reference to the backing data.  The blob itself lives in
    /// the runtime's ephemeral resource store, not in the scene graph.
    pub resource_id: ResourceId,
    /// Width of the image in pixels (metadata from upload).
    pub width: u32,
    /// Height of the image in pixels (metadata from upload).
    pub height: u32,
    /// Decoded in-memory size in bytes, recorded at mutation time for budget
    /// accounting.  Does NOT store the actual pixels.
    ///
    /// Using `u64` to match scene budget accounting types (`ResourceBudget.max_texture_bytes`)
    /// and the protobuf wire type, avoiding lossy casts on 32-bit targets.
    pub decoded_bytes: u64,
    /// How the image is fitted within `bounds`.
    pub fit_mode: ImageFitMode,
    /// Position and size within the parent tile (in tile-local coordinates).
    pub bounds: Rect,
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

// ─── Lease state machine (RFC 0008) ──────────────────────────────────────────

/// Lease lifecycle state per RFC 0008 SS3.
///
/// All 8 canonical states from the spec are present.
/// Terminal states (no further transitions): `Denied`, `Revoked`, `Expired`, `Released`.
/// Non-terminal: `Requested`, `Active`, `Suspended`, `Orphaned`.
///
/// `Disconnected` is a deprecated alias for `Orphaned`; new code should use `Orphaned`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaseState {
    /// Lease request received; runtime evaluating. No mutations allowed.
    Requested,
    /// Lease valid — agent holds mutation rights.
    Active,
    /// Lease suspended (safe mode) — mutations blocked, state and tiles preserved.
    Suspended,
    /// Session disconnected — within reconnect grace period. Tiles frozen.
    /// Canonical name per spec. Replaces `Disconnected`.
    Orphaned,
    /// Agent disconnected — legacy alias for `Orphaned`. Kept for backward compat.
    Disconnected,
    /// Lease request rejected — terminal; agent must submit a new request.
    Denied,
    /// Lease revoked — state destroyed. Terminal.
    Revoked,
    /// Lease expired (TTL exceeded) — state destroyed. Terminal.
    Expired,
    /// Agent voluntarily released lease — state destroyed. Terminal.
    Released,
}

impl LeaseState {
    /// Whether this state is terminal (no further transitions possible).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            LeaseState::Denied | LeaseState::Revoked | LeaseState::Expired | LeaseState::Released
        )
    }
}

/// Renewal policy per RFC 0008 SS1.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenewalPolicy {
    /// Agent must explicitly renew before TTL expires.
    Manual,
    /// Runtime auto-renews at 75% TTL elapsed.
    AutoRenew,
    /// No renewal; expires at TTL.
    OneShot,
}

impl Default for RenewalPolicy {
    fn default() -> Self {
        RenewalPolicy::Manual
    }
}

/// Lease caps violation error.
#[derive(Clone, Debug, PartialEq)]
pub enum CapsError {
    /// Runtime-wide lease limit (64) exceeded — spec §Requirement: Lease Caps.
    MaxRuntimeLeasesExceeded { current: usize, limit: usize },
    /// Per-session lease hard limit (64) exceeded — spec §Requirement: Lease Caps.
    MaxSessionLeasesExceeded { current: usize, limit: usize },
    /// Tile-per-lease limit (64) exceeded — spec §Requirement: Lease Caps.
    MaxTilesPerLeaseExceeded { current: u32, limit: u32 },
    /// Node-per-tile limit (64) exceeded — spec §Requirement: Lease Caps.
    MaxNodesPerTileExceeded { current: u32, limit: u32 },
}

impl std::fmt::Display for CapsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapsError::MaxRuntimeLeasesExceeded { current, limit } => {
                write!(f, "MAX_RUNTIME_LEASES_EXCEEDED: {} / {}", current, limit)
            }
            CapsError::MaxSessionLeasesExceeded { current, limit } => {
                write!(f, "MAX_SESSION_LEASES_EXCEEDED: {} / {}", current, limit)
            }
            CapsError::MaxTilesPerLeaseExceeded { current, limit } => {
                write!(f, "MAX_TILES_PER_LEASE_EXCEEDED: {} / {}", current, limit)
            }
            CapsError::MaxNodesPerTileExceeded { current, limit } => {
                write!(f, "MAX_NODES_PER_TILE_EXCEEDED: {} / {}", current, limit)
            }
        }
    }
}

/// Error type for lease state transitions.
#[derive(Clone, Debug, PartialEq)]
pub enum LeaseError {
    /// Attempted an invalid state transition.
    InvalidTransition { from: LeaseState, to: LeaseState },
    /// Lease not found in the scene graph.
    LeaseNotFound(SceneId),
    /// Lease exists but is not in Active state.
    LeaseNotActive(SceneId),
    /// Mutation would exceed the lease's resource budget.
    BudgetExceeded(BudgetError),
    /// Lease caps exceeded (runtime-wide or per-session).
    CapsExceeded(CapsError),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeaseError::InvalidTransition { from, to } => {
                write!(f, "invalid lease transition: {:?} -> {:?}", from, to)
            }
            LeaseError::LeaseNotFound(id) => write!(f, "lease not found: {}", id),
            LeaseError::LeaseNotActive(id) => write!(f, "lease not active: {}", id),
            LeaseError::BudgetExceeded(e) => write!(f, "budget exceeded: {}", e),
            LeaseError::CapsExceeded(e) => write!(f, "caps exceeded: {}", e),
        }
    }
}

impl std::error::Error for LeaseError {}

/// Error returned when a mutation batch would exceed budget limits.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetError {
    /// Which resource dimension was exceeded (e.g. "tiles", "texture_bytes").
    pub resource: String,
    /// Current usage before the mutation.
    pub current: u64,
    /// The configured limit.
    pub limit: u64,
    /// How much the mutation batch would add.
    pub requested: u64,
}

impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: current={}, limit={}, requested={}",
            self.resource, self.current, self.limit, self.requested
        )
    }
}

/// Current resource usage for a lease.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceUsage {
    /// Number of tiles owned by this lease.
    pub tiles: u32,
    /// Total texture bytes across all tiles.
    pub texture_bytes: u64,
    /// Node count per tile (tile_id -> count).
    pub nodes_per_tile: HashMap<SceneId, u32>,
}

/// Information about an expired or cleaned-up lease.
#[derive(Clone, Debug, PartialEq)]
pub struct LeaseExpiry {
    /// The lease ID that was expired/cleaned up.
    pub lease_id: SceneId,
    /// The terminal state it entered.
    pub terminal_state: LeaseState,
    /// Tile IDs that were removed as a result.
    pub removed_tiles: Vec<SceneId>,
}

// ─── Lease ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lease {
    /// UUIDv7 lease identifier (time-ordered, SceneId type). Assigned at grant time.
    pub id: SceneId,
    /// Agent identity string (namespace). Established at session auth.
    pub namespace: String,
    /// Parent session identifier. Lease is invalidated if session is revoked.
    pub session_id: SceneId,
    pub state: LeaseState,
    /// Priority: 0=system/chrome (reserved), 1=high, 2=normal (default), 3=low, 4+=background.
    /// Per RFC 0008 SS2.
    pub priority: u8,
    /// Wall-clock grant timestamp in milliseconds since Unix epoch (RFC 0003 wall-clock domain).
    /// Corresponds to `granted_at_wall_us / 1000` in the wire protocol.
    pub granted_at_ms: u64,
    pub ttl_ms: u64,
    pub renewal_policy: RenewalPolicy,
    pub capabilities: Vec<Capability>,
    pub resource_budget: ResourceBudget,
    // Suspension tracking
    /// Timestamp when the lease was suspended (ms since epoch).
    pub suspended_at_ms: Option<u64>,
    /// TTL remaining at the moment of suspension (ms).
    pub ttl_remaining_at_suspend_ms: Option<u64>,
    // Orphan/disconnect tracking
    /// Timestamp when the agent disconnected (ms since epoch).
    pub disconnected_at_ms: Option<u64>,
    /// Grace period before an orphaned lease is cleaned up (ms). Default 30_000.
    pub grace_period_ms: u64,
}

/// Agent capabilities that govern what mutations are permitted.
///
/// Canonical names per configuration/spec.md §Requirement: Capability Vocabulary.
/// RFC 0001 §3.1, §3.3 defines the canonical capability names.
///
/// The `String`-bearing variants (`PublishZone`, `EmitSceneEvent`) carry their
/// parameterized argument (zone name or event name).
///
/// Legacy variants (`CreateTile`, `UpdateTile`, …) are retained for backward
/// compatibility; new code should use the canonical-name variants where available.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    // ── Legacy / backward-compat variants ────────────────────────────────────
    /// Legacy: equivalent to `CreateTiles`.
    CreateTile,
    /// Legacy: equivalent to `ModifyOwnTiles`.
    UpdateTile,
    DeleteTile,
    CreateNode,
    UpdateNode,
    DeleteNode,
    /// Legacy: equivalent to `AccessInputEvents`.
    ReceiveInput,

    // ── Canonical v1 capability vocabulary ────────────────────────────────────
    /// `create_tiles` — agent may create tiles.
    CreateTiles,
    /// `modify_own_tiles` — agent may mutate tiles it owns.
    ModifyOwnTiles,
    /// `manage_tabs` — agent may create/switch tabs.
    ManageTabs,
    /// `manage_sync_groups` — agent may create/manage sync groups.
    ManageSyncGroups,
    /// `upload_resource` — agent may upload resources.
    UploadResource,
    /// `read_scene_topology` — agent may read the scene graph topology.
    ReadSceneTopology,
    /// `subscribe_scene_events` — agent may subscribe to scene events.
    SubscribeSceneEvents,
    /// `overlay_privileges` — agent may use overlay/chrome privileges.
    OverlayPrivileges,
    /// `access_input_events` — agent may receive input events.
    AccessInputEvents,
    /// `high_priority_z_order` — agent may request high z-order tiles.
    HighPriorityZOrder,
    /// `exceed_default_budgets` — agent may exceed default resource budgets.
    ExceedDefaultBudgets,
    /// `read_telemetry` — agent may read telemetry data.
    ReadTelemetry,
    /// `publish_zone:<zone_name>` or `publish_zone:*` — agent may publish to a zone.
    PublishZone(String),
    /// `emit_scene_event:<event_name>` — agent may emit a named scene event.
    EmitSceneEvent(String),
    /// `resident_mcp` — agent is a resident MCP agent.
    ResidentMcp,
    /// `lease:priority:1` — agent may request lease priority 1 (high).
    LeasePriority1,
}

impl Lease {
    /// Check if the lease has expired based on effective TTL elapsed.
    ///
    /// Accounts for suspension: time spent in Suspended state does not count
    /// toward TTL consumption (RFC 0008 SS4.3).
    pub fn is_expired(&self, now_ms: u64) -> bool {
        match self.state {
            // Terminal states are already past expiry semantics.
            LeaseState::Denied | LeaseState::Revoked | LeaseState::Expired | LeaseState::Released => true,
            // When suspended, TTL clock is paused — not expired.
            LeaseState::Suspended => false,
            // When orphaned/disconnected, TTL continues.
            // (Grace period handles cleanup separately.)
            LeaseState::Orphaned
            | LeaseState::Disconnected
            | LeaseState::Active
            | LeaseState::Requested => {
                self.effective_remaining_ms(now_ms) == 0
            }
        }
    }

    /// Check whether this lease grants the requested capability.
    ///
    /// The v1-canonical capabilities (`ManageTabs`, `CreateTiles`, `ModifyOwnTiles`) are
    /// checked exactly. Legacy broad variants act as aliases:
    ///
    /// - `CreateTile` covers `CreateTiles` + `ModifyOwnTiles` (legacy: create implied mutate)
    /// - `UpdateTile` covers `ModifyOwnTiles`
    /// - `DeleteTile` covers `ModifyOwnTiles`
    ///
    /// This ensures test code using the legacy variant names is not broken by the introduction
    /// of the v1-canonical names.
    pub fn has_capability(&self, cap: Capability) -> bool {
        if self.capabilities.contains(&cap) {
            return true;
        }
        // Legacy capability aliases
        match cap {
            Capability::CreateTiles => {
                self.capabilities.contains(&Capability::CreateTile)
            }
            Capability::ModifyOwnTiles => {
                self.capabilities.contains(&Capability::CreateTile)
                    || self.capabilities.contains(&Capability::UpdateTile)
                    || self.capabilities.contains(&Capability::DeleteTile)
            }
            _ => false,
        }
    }

    /// Remaining TTL in milliseconds (0 if expired).
    ///
    /// If the lease was previously suspended, the suspension duration is
    /// deducted so that the effective TTL is preserved across suspend/resume.
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.effective_remaining_ms(now_ms)
    }

    /// Effective remaining TTL accounting for suspension pauses.
    fn effective_remaining_ms(&self, now_ms: u64) -> u64 {
        match self.state {
            LeaseState::Suspended => {
                // TTL frozen at the value saved when suspension started.
                self.ttl_remaining_at_suspend_ms.unwrap_or(0)
            }
            _ => {
                let expires = self.granted_at_ms + self.ttl_ms;
                expires.saturating_sub(now_ms)
            }
        }
    }

    // ─── State transition methods ────────────────────────────────────────

    /// Transition Active -> Suspended (safe mode entry).
    ///
    /// Pauses the TTL clock and records suspension timestamp.
    pub fn suspend(&mut self, now_ms: u64) -> Result<(), LeaseError> {
        if self.state != LeaseState::Active {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Suspended,
            });
        }
        let remaining = self.effective_remaining_ms(now_ms);
        self.suspended_at_ms = Some(now_ms);
        self.ttl_remaining_at_suspend_ms = Some(remaining);
        self.state = LeaseState::Suspended;
        Ok(())
    }

    /// Transition Suspended -> Active (safe mode exit).
    ///
    /// Resumes the TTL clock. The `granted_at_ms` and `ttl_ms` are adjusted
    /// so that the remaining TTL equals what was saved at suspension time.
    pub fn resume(&mut self, now_ms: u64) -> Result<(), LeaseError> {
        if self.state != LeaseState::Suspended {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        // Restore TTL: set granted_at_ms so that granted_at_ms + ttl_ms
        // equals now_ms + remaining.
        if let Some(remaining) = self.ttl_remaining_at_suspend_ms {
            self.granted_at_ms = now_ms;
            self.ttl_ms = remaining;
        }
        self.suspended_at_ms = None;
        self.ttl_remaining_at_suspend_ms = None;
        self.state = LeaseState::Active;
        Ok(())
    }

    /// Transition Active -> Orphaned (agent disconnect).
    ///
    /// Starts the grace period. TTL continues running.
    /// Only `Active` is accepted as source state; any other state returns `InvalidTransition`.
    pub fn disconnect(&mut self, now_ms: u64) -> Result<(), LeaseError> {
        if self.state != LeaseState::Active {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Orphaned,
            });
        }
        self.disconnected_at_ms = Some(now_ms);
        self.state = LeaseState::Orphaned;
        Ok(())
    }

    /// Transition Orphaned (or legacy Disconnected) -> Active (agent reconnect within grace period).
    pub fn reconnect(&mut self, now_ms: u64) -> Result<(), LeaseError> {
        if self.state != LeaseState::Orphaned && self.state != LeaseState::Disconnected {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        // Check that grace period has not expired.
        if self.check_grace_expired(now_ms) {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        self.disconnected_at_ms = None;
        self.state = LeaseState::Active;
        Ok(())
    }

    /// Transition any non-terminal state -> Revoked.
    pub fn revoke(&mut self) -> Result<(), LeaseError> {
        if self.state.is_terminal() {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Revoked,
            });
        }
        self.state = LeaseState::Revoked;
        Ok(())
    }

    /// Whether the lease is currently in Active state.
    pub fn is_active(&self) -> bool {
        self.state == LeaseState::Active
    }

    /// Whether mutations are allowed. Only Active state permits mutations.
    pub fn is_mutations_allowed(&self) -> bool {
        self.state == LeaseState::Active
    }

    /// Check if the grace period has expired for an orphaned/disconnected lease.
    pub fn check_grace_expired(&self, now_ms: u64) -> bool {
        match (self.state, self.disconnected_at_ms) {
            (LeaseState::Orphaned, Some(disc_at))
            | (LeaseState::Disconnected, Some(disc_at)) => {
                now_ms >= disc_at + self.grace_period_ms
            }
            _ => false,
        }
    }

    /// Check if a suspended lease has exceeded the maximum suspension time.
    pub fn check_suspension_expired(&self, now_ms: u64, max_suspend_ms: u64) -> bool {
        match (self.state, self.suspended_at_ms) {
            (LeaseState::Suspended, Some(susp_at)) => {
                now_ms >= susp_at + max_suspend_ms
            }
            _ => false,
        }
    }
}

// ─── Sync Groups ────────────────────────────────────────────────────────────

/// Type alias for sync group IDs (they are just SceneIds).
pub type SyncGroupId = SceneId;

/// Commit policy for a sync group.
///
/// See RFC 0003 §2.2 for full semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncCommitPolicy {
    /// All members must have a pending mutation before any are applied.
    /// If not all members are ready when Stage 4 runs, the group is deferred.
    /// After `max_deferrals` consecutive deferrals the available members are
    /// force-committed and a telemetry event is emitted.
    #[default]
    AllOrDefer,

    /// Apply whatever subset of members have pending mutations this frame.
    /// Members without pending mutations are implicitly "unchanged".
    AvailableMembers,
}

/// A sync group is a named set of tiles whose mutations must be applied
/// atomically in the same frame.
///
/// See RFC 0003 §2 for the full specification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncGroup {
    /// Unique identifier (UUIDv7).
    pub id: SyncGroupId,
    /// Optional human-readable label (max 128 UTF-8 bytes).
    pub name: Option<String>,
    /// Namespace that created this group.
    pub owner_namespace: String,
    /// Tile IDs currently in the group.
    pub members: std::collections::BTreeSet<SceneId>,
    /// Wall-clock creation time (UTC microseconds since Unix epoch).
    pub created_at_us: u64,
    /// Commit policy.
    pub commit_policy: SyncCommitPolicy,
    /// Maximum number of consecutive deferral frames before a force-commit.
    /// Only relevant when `commit_policy == AllOrDefer`. Default: 3.
    pub max_deferrals: u32,
    /// Current consecutive deferral count (runtime state — not part of
    /// the authoritative scene snapshot, but carried in the struct for
    /// simplicity in the scene crate).
    #[serde(default)]
    pub deferral_count: u32,
}

impl SyncGroup {
    pub fn new(
        id: SyncGroupId,
        name: Option<String>,
        owner_namespace: String,
        commit_policy: SyncCommitPolicy,
        max_deferrals: u32,
        created_at_us: u64,
    ) -> Self {
        Self {
            id,
            name,
            owner_namespace,
            members: std::collections::BTreeSet::new(),
            created_at_us,
            commit_policy,
            max_deferrals,
            deferral_count: 0,
        }
    }
}

// ─── Zone types ─────────────────────────────────────────────────────────────

/// Minimum z-order for Content-layer zone tiles (= 0x8000_0000).
///
/// Content-layer zone tiles must participate in the same z-order traversal as
/// agent tiles but in the reserved upper band (≥ ZONE_TILE_Z_MIN). Agent tiles
/// must use z_order values below this constant.
///
/// Per scene-graph/spec.md §Requirement: Zone Layer Attachment.
pub const ZONE_TILE_Z_MIN: u32 = 0x8000_0000;

/// Layer attachment for a zone instance — determines rendering order.
///
/// Per RFC 0001 §2.5 and scene-graph/spec.md line 241.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerAttachment {
    /// Rendered behind all agent tiles (below content layer).
    Background,
    /// Rendered within the content layer z-order space at
    /// z_order >= [`ZONE_TILE_Z_MIN`].
    Content,
    /// Rendered above all agent content; managed by runtime chrome rendering.
    Chrome,
}

/// Display edge for edge-anchored zone geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayEdge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Geometry policy — how a zone is positioned on the display.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum GeometryPolicy {
    /// Percentage-based position relative to display area.
    Relative {
        x_pct: f32,
        y_pct: f32,
        width_pct: f32,
        height_pct: f32,
    },
    /// Anchored to a display edge.
    EdgeAnchored {
        edge: DisplayEdge,
        /// Used for Top/Bottom edges.
        height_pct: f32,
        /// Used for Left/Right edges.
        width_pct: f32,
        margin_px: f32,
    },
}

/// Media types that can be published to a zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneMediaType {
    /// Stream-text with optional breakpoints.
    StreamText,
    /// Notification: text + icon + urgency.
    ShortTextWithIcon,
    /// Status-bar: key-value map.
    KeyValuePairs,
    /// Reference to a media surface (post-v1 media layer).
    VideoSurfaceRef,
    /// Static image resource.
    StaticImage,
    /// Solid color fill.
    SolidColor,
}

/// Rendering policy — how content is presented in the zone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RenderingPolicy {
    pub font_size_px: Option<f32>,
    pub backdrop: Option<Rgba>,
    pub text_align: Option<TextAlign>,
    pub margin_px: Option<f32>,
}

/// Contention policy — what happens when multiple agents publish to the same zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentionPolicy {
    /// Most recent publish replaces previous content.
    LatestWins,
    /// Publishes accumulate as a stack; each auto-dismisses.
    Stack { max_depth: u8 },
    /// Each publish includes a key; same key replaces, different keys coexist.
    MergeByKey { max_keys: u8 },
    /// Only one occupant; new publish evicts current one.
    Replace,
}

/// Transport constraint for zone publishing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportConstraint {
    /// Content must arrive via gRPC session stream.
    GrpcOnly,
    /// Content may arrive via MCP tool call.
    McpAllowed,
    /// Content requires WebRTC media channel (post-v1).
    WebRtcRequired,
}

/// Full zone definition per RFC 0001 §2.5.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneDefinition {
    pub id: SceneId,
    pub name: String,
    pub description: String,
    pub geometry_policy: GeometryPolicy,
    pub accepted_media_types: Vec<ZoneMediaType>,
    pub rendering_policy: RenderingPolicy,
    pub contention_policy: ContentionPolicy,
    pub max_publishers: u32,
    pub transport_constraint: Option<TransportConstraint>,
    /// Auto-clear timeout in milliseconds; None = no auto-clear.
    pub auto_clear_ms: Option<u64>,
    /// When true, publishes to this zone are fire-and-forget (no ZonePublishResult).
    /// When false (default), publishes are transactional and receive a ZonePublishResult.
    /// Per RFC 0005 §3.1, §8.6.
    #[serde(default)]
    pub ephemeral: bool,
    /// Layer attachment — determines rendering order and z-space.
    /// Defaults to [`LayerAttachment::Content`] if not specified.
    pub layer_attachment: LayerAttachment,
}

// ─── Zone publish token ──────────────────────────────────────────────────────

/// Opaque capability token that authorizes publishing to a specific zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZonePublishToken {
    /// Opaque bytes issued at session auth.
    pub token: Vec<u8>,
}

// ─── Zone content ────────────────────────────────────────────────────────────

/// Notification payload: text + optional icon + urgency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPayload {
    pub text: String,
    /// Resource name or empty string.
    pub icon: String,
    /// 0=low, 1=normal, 2=urgent, 3=critical.
    pub urgency: u32,
}

/// Status-bar payload: key → display string map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusBarPayload {
    pub entries: HashMap<String, String>,
}

/// Content that can be published to a zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ZoneContent {
    StreamText(String),
    Notification(NotificationPayload),
    StatusBar(StatusBarPayload),
    SolidColor(Rgba),
    /// Static image reference (v1-mandatory: content-addressed resource).
    StaticImage(ResourceId),
    /// Video surface reference (post-v1; schema defined, rendering deferred).
    VideoSurfaceRef(SceneId),
}

// ─── Zone publish records ────────────────────────────────────────────────────

/// Record of a single publish event into a zone.
///
/// This is the publication event (third level of the zone ontology:
/// ZoneType → ZoneInstance → ZonePublication → ZoneOccupancy).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZonePublishRecord {
    pub zone_name: String,
    pub publisher_namespace: String,
    pub content: ZoneContent,
    pub published_at_ms: u64,
    /// For MergeByKey contention: the key under which this record is stored.
    pub merge_key: Option<String>,
    /// Optional wall-clock expiry timestamp (microseconds since epoch).
    /// When present, the runtime MUST clear this publication at or before this time.
    /// None = no expiry (publication lives until explicitly cleared or zone cleared).
    pub expires_at_wall_us: Option<u64>,
    /// Optional content classification tag (e.g., "public", "private", "pii").
    /// Used by policy and redaction layers; treated as opaque by the scene graph.
    pub content_classification: Option<String>,
}

/// Type alias for `ZonePublishRecord` — the canonical name for the publication
/// level of the four-level zone ontology.
///
/// Ontology levels:
/// 1. `ZoneDefinition` — the zone type (schema, accepted media, contention policy)
/// 2. `ZoneInstance` — zone type bound to a specific tab
/// 3. `ZonePublication` (= `ZonePublishRecord`) — a single publish event
/// 4. `ZoneOccupancy` — resolved state after applying contention policy
pub type ZonePublication = ZonePublishRecord;

/// A zone instance — zone type bound to a specific tab.
///
/// In v1, zone instances are static (loaded from config; one instance per tab
/// per zone type). Agents MUST NOT create zone instances.
///
/// Per scene-graph/spec.md §Requirement: Zone Registry (line 185).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneInstance {
    /// The zone type (definition) this instance belongs to.
    pub zone_type_name: String,
    /// The tab this instance is bound to.
    pub tab_id: SceneId,
    /// Instance-level geometry override (None = use zone type's geometry_policy).
    pub geometry_override: Option<GeometryPolicy>,
}

/// Zone occupancy — the resolved state after applying the contention policy.
///
/// This is the fourth level of the zone ontology. In v1, effective_geometry
/// is NOT exposed (deferred to post-v1 per spec line 360).
///
/// Per scene-graph/spec.md §Requirement: Zone Occupancy Query API (line 360,
/// post-v1), and §Requirement: Zone Registry (line 185, v1-mandatory).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneOccupancy {
    pub zone_name: String,
    pub tab_id: SceneId,
    /// Active publications after applying contention policy.
    pub active_publications: Vec<ZonePublishRecord>,
    /// Occupant count after contention resolution.
    pub occupant_count: u32,
    // NOTE: effective_geometry intentionally absent in v1 (post-v1 per spec line 360).
}

// ─── Zone registry ───────────────────────────────────────────────────────────

/// Snapshot of the zone registry (all zones + active publishes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneRegistrySnapshot {
    pub zones: Vec<ZoneDefinition>,
    pub active_publishes: Vec<ZonePublishRecord>,
}

/// Runtime-owned zone registry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneRegistry {
    /// Zone definitions keyed by zone name.
    pub zones: HashMap<String, ZoneDefinition>,
    /// Active publishes per zone name.
    /// For LatestWins/Replace: at most one entry per zone.
    /// For Stack: ordered oldest-first, bounded by max_depth.
    /// For MergeByKey: keyed by merge_key, bounded by max_keys.
    pub active_publishes: HashMap<String, Vec<ZonePublishRecord>>,
}

impl ZoneRegistry {
    pub fn new() -> Self {
        Self {
            zones: HashMap::new(),
            active_publishes: HashMap::new(),
        }
    }

    /// Create a registry pre-populated with the default v1 zones.
    ///
    /// V1 zone set (scene-graph/spec.md §Implementation Details):
    /// subtitle, notification-area, status-bar, pip, ambient-background, alert-banner.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();

        // 1. status-bar: edge-anchored bottom, MergeByKey, Chrome layer
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_string(),
            description: "Status bar at the bottom of the display".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 32 },
            max_publishers: 16,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // 2. notification-area: top-right relative, Stack, Chrome layer
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_string(),
            description: "Notification overlay area".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 16,
            transport_constraint: None,
            auto_clear_ms: Some(5_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // 3. subtitle: edge-anchored bottom, LatestWins, Content layer
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_string(),
            description: "Subtitle / caption overlay".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 48.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // 4. pip: picture-in-picture, Relative geometry, Replace, Content layer
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "pip".to_string(),
            description: "Picture-in-picture overlay zone".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.70,
                width_pct: 0.22,
                height_pct: 0.26,
            },
            accepted_media_types: vec![
                ZoneMediaType::SolidColor,
                ZoneMediaType::StaticImage,
                ZoneMediaType::VideoSurfaceRef,
            ],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // 5. ambient-background: full-screen, Replace, Background layer
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "ambient-background".to_string(),
            description: "Ambient background zone — full display, behind all content".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 1.0,
            },
            accepted_media_types: vec![
                ZoneMediaType::SolidColor,
                ZoneMediaType::StaticImage,
                ZoneMediaType::VideoSurfaceRef,
            ],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Background,
        });

        // 6. alert-banner: edge-anchored top, Replace, Chrome layer
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_string(),
            description: "Alert banner — top edge, single occupant".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.05,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![
                ZoneMediaType::ShortTextWithIcon,
                ZoneMediaType::StreamText,
            ],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: Some(10_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        registry
    }

    /// Register a zone definition. Overwrites any existing definition with the same name.
    pub fn register(&mut self, zone: ZoneDefinition) {
        self.zones.insert(zone.name.clone(), zone);
    }

    /// Remove a zone definition by name. Returns the removed definition if present.
    pub fn unregister(&mut self, name: &str) -> Option<ZoneDefinition> {
        self.active_publishes.remove(name);
        self.zones.remove(name)
    }

    /// Look up a zone by name.
    pub fn get_by_name(&self, name: &str) -> Option<&ZoneDefinition> {
        self.zones.get(name)
    }

    /// Query zones that accept a given media type.
    pub fn zones_accepting(&self, media_type: ZoneMediaType) -> Vec<&ZoneDefinition> {
        self.zones
            .values()
            .filter(|z| z.accepted_media_types.contains(&media_type))
            .collect()
    }

    /// Return all zone definitions.
    pub fn all_zones(&self) -> Vec<&ZoneDefinition> {
        self.zones.values().collect()
    }

    /// Get the current active publish(es) for a zone.
    pub fn active_for_zone(&self, zone_name: &str) -> &[ZonePublishRecord] {
        self.active_publishes
            .get(zone_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Query occupancy for a zone instance (resolved state after contention policy).
    ///
    /// In v1, `effective_geometry` is not exposed (deferred to post-v1).
    /// Returns `None` if the zone is not found.
    pub fn get_occupancy(&self, zone_name: &str, tab_id: SceneId) -> Option<ZoneOccupancy> {
        let _zone = self.zones.get(zone_name)?;
        let pubs = self.active_publishes
            .get(zone_name)
            .map(|v| v.clone())
            .unwrap_or_default();
        let occupant_count = pubs.len() as u32;
        Some(ZoneOccupancy {
            zone_name: zone_name.to_string(),
            tab_id,
            active_publications: pubs,
            occupant_count,
        })
    }

    /// Query zones by layer attachment.
    pub fn zones_with_attachment(&self, attachment: LayerAttachment) -> Vec<&ZoneDefinition> {
        self.zones
            .values()
            .filter(|z| z.layer_attachment == attachment)
            .collect()
    }

    /// Snapshot the registry (all definitions + all active publishes).
    pub fn snapshot(&self) -> ZoneRegistrySnapshot {
        ZoneRegistrySnapshot {
            zones: self.zones.values().cloned().collect(),
            active_publishes: self
                .active_publishes
                .values()
                .flat_map(|v| v.iter().cloned())
                .collect(),
        }
    }
}

impl Default for ZoneRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    // ── SceneId size invariant ────────────────────────────────────────────────

    #[test]
    fn scene_id_size_is_16_bytes() {
        assert_eq!(size_of::<SceneId>(), 16, "SceneId must be exactly 16 bytes");
    }

    // ── SceneId null sentinel ─────────────────────────────────────────────────

    #[test]
    fn scene_id_null_is_all_zeros() {
        let null = SceneId::null();
        assert!(null.is_null(), "SceneId::null() must report is_null() == true");
        assert_eq!(null.to_bytes_le(), [0u8; 16], "null SceneId must serialize to 16 zero bytes");
    }

    #[test]
    fn scene_id_new_is_never_null() {
        let id = SceneId::new();
        assert!(!id.is_null(), "freshly-generated SceneId must not be null");
    }

    #[test]
    fn scene_id_nil_aliases_null() {
        assert_eq!(SceneId::nil(), SceneId::null());
        assert!(SceneId::nil().is_nil());
    }

    // ── SceneId byte round-trip ───────────────────────────────────────────────

    #[test]
    fn scene_id_bytes_le_round_trip() {
        let id = SceneId::new();
        let bytes = id.to_bytes_le();
        let restored = SceneId::from_bytes_le(&bytes).expect("must decode 16 bytes");
        assert_eq!(id, restored, "SceneId bytes LE round-trip must be lossless");
    }

    #[test]
    fn scene_id_from_bytes_le_rejects_wrong_length() {
        assert!(SceneId::from_bytes_le(&[0u8; 15]).is_none());
        assert!(SceneId::from_bytes_le(&[0u8; 17]).is_none());
        assert!(SceneId::from_bytes_le(&[]).is_none());
    }

    #[test]
    fn scene_id_null_round_trips_via_bytes() {
        let null = SceneId::null();
        let bytes = null.to_bytes_le();
        let restored = SceneId::from_bytes_le(&bytes).unwrap();
        assert!(restored.is_null());
    }

    // ── SceneId lexicographic / monotonicity ─────────────────────────────────

    #[test]
    fn scene_id_monotonic_small_batch() {
        // Generate a small batch synchronously and verify they're non-decreasing.
        // (A full 10,000-ID property test is in the proptest suite below.)
        let ids: Vec<SceneId> = (0..64).map(|_| SceneId::new()).collect();
        for w in ids.windows(2) {
            assert!(
                w[0] <= w[1],
                "SceneId sequence must be non-decreasing: {:?} > {:?}",
                w[0],
                w[1]
            );
        }
    }

    // ── ResourceId size invariant ─────────────────────────────────────────────

    #[test]
    fn resource_id_size_is_32_bytes() {
        assert_eq!(size_of::<ResourceId>(), 32, "ResourceId must be exactly 32 bytes");
    }

    // ── ResourceId content deduplication ─────────────────────────────────────

    #[test]
    fn resource_id_same_content_same_id() {
        let data = b"hello world";
        let id1 = ResourceId::of(data);
        let id2 = ResourceId::of(data);
        assert_eq!(id1, id2, "identical content must produce the same ResourceId");
    }

    #[test]
    fn resource_id_different_content_different_id() {
        let id1 = ResourceId::of(b"foo");
        let id2 = ResourceId::of(b"bar");
        assert_ne!(id1, id2, "different content must produce different ResourceIds");
    }

    #[test]
    fn resource_id_empty_content() {
        let id = ResourceId::of(b"");
        assert_eq!(id.as_bytes().len(), 32);
    }

    // ── ResourceId byte round-trip ────────────────────────────────────────────

    #[test]
    fn resource_id_from_bytes_round_trip() {
        let id = ResourceId::of(b"round-trip test payload");
        let bytes = *id.as_bytes();
        let restored = ResourceId::from_bytes(bytes);
        assert_eq!(id, restored);
    }

    #[test]
    fn resource_id_from_slice_round_trip() {
        let id = ResourceId::of(b"slice round-trip");
        let restored = ResourceId::from_slice(id.as_bytes()).expect("must accept 32-byte slice");
        assert_eq!(id, restored);
    }

    #[test]
    fn resource_id_from_slice_rejects_wrong_length() {
        assert!(ResourceId::from_slice(&[0u8; 31]).is_none());
        assert!(ResourceId::from_slice(&[0u8; 33]).is_none());
        assert!(ResourceId::from_slice(&[]).is_none());
    }

    // ── ResourceId display / hex is debug-only ────────────────────────────────

    #[test]
    fn resource_id_to_hex_is_64_chars() {
        let id = ResourceId::of(b"hex display test");
        let hex = id.to_hex();
        assert_eq!(hex.len(), 64, "hex of 32-byte hash must be 64 chars");
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()), "must be valid hex");
    }

    // ── Layer 0 identity invariant check helper ───────────────────────────────

    /// Validates the core Layer 0 identity invariants for `SceneId` and `ResourceId`.
    /// This function mirrors what `assert_layer0_invariants` checks at the graph level
    /// but focuses on the type-level contracts.
    pub fn assert_identity_invariants() -> Vec<String> {
        let mut violations = Vec::new();

        if size_of::<SceneId>() != 16 {
            violations.push(format!("SceneId size {} != 16", size_of::<SceneId>()));
        }
        if size_of::<ResourceId>() != 32 {
            violations.push(format!("ResourceId size {} != 32", size_of::<ResourceId>()));
        }
        if !SceneId::null().is_null() {
            violations.push("SceneId::null() does not report is_null()".into());
        }
        if SceneId::new().is_null() {
            violations.push("freshly-generated SceneId reports is_null()".into());
        }
        let id = ResourceId::of(b"test");
        if ResourceId::of(b"test") != id {
            violations.push("ResourceId deduplication failed".into());
        }

        violations
    }

    #[test]
    fn layer0_identity_invariants_pass() {
        let violations = assert_identity_invariants();
        assert!(violations.is_empty(), "Layer 0 identity violations: {:?}", violations);
    }
}

// ─── Property tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Generates 10,000 SceneIds and asserts they are monotonically non-decreasing.
    ///
    /// UUIDv7 guarantees creation-time ordering via a monotonic counter within the
    /// same millisecond, so lexicographic sort == chronological sort.
    #[test]
    fn scene_id_monotonic_10k() {
        let ids: Vec<SceneId> = (0..10_000).map(|_| SceneId::new()).collect();
        for w in ids.windows(2) {
            assert!(
                w[0] <= w[1],
                "SceneId not monotonically non-decreasing: {:?} > {:?}",
                w[0],
                w[1]
            );
        }
    }

    proptest! {
        /// Verifies that any 16-byte input round-trips through SceneId bytes LE encoding.
        #[test]
        fn scene_id_bytes_le_roundtrip_arb(raw in proptest::array::uniform16(0u8..)) {
            // from_bytes_le -> to_bytes_le must be identity
            let id = SceneId::from_bytes_le(&raw).expect("uniform16 is always 16 bytes");
            prop_assert_eq!(id.to_bytes_le(), raw);
        }

        /// Verifies that any 32-byte slice round-trips through ResourceId.
        #[test]
        fn resource_id_bytes_roundtrip_arb(raw in proptest::array::uniform32(0u8..)) {
            let id = ResourceId::from_bytes(raw);
            prop_assert_eq!(*id.as_bytes(), raw);
        }

        /// Verifies BLAKE3 determinism: same input always produces the same ResourceId.
        #[test]
        fn resource_id_deterministic(data in proptest::collection::vec(0u8.., 0..1024)) {
            let id1 = ResourceId::of(&data);
            let id2 = ResourceId::of(&data);
            prop_assert_eq!(id1, id2);
        }

        /// Verifies that distinct inputs produce distinct ResourceIds (collision resistance).
        #[test]
        fn resource_id_distinct_inputs_distinct_ids(
            a in proptest::collection::vec(0u8.., 1..512),
            b in proptest::collection::vec(0u8.., 1..512),
        ) {
            // Only assert when inputs differ
            if a != b {
                let id_a = ResourceId::of(&a);
                let id_b = ResourceId::of(&b);
                prop_assert_ne!(id_a, id_b, "distinct content must yield distinct ResourceIds");
            }
        }
    }
}
