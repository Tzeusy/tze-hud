# RFC 0001: Scene Contract

**Status:** Draft
**Issue:** rig-5vq.1
**Date:** 2026-03-22
**Authors:** tze_hud architecture team

---

## Summary

This RFC defines the Scene Contract — the authoritative data model specification for tze_hud. It covers the scene graph as a pure data structure (no GPU dependency), the full identity model, all mutation operations, the transaction pipeline, diff and snapshot formats, hit-testing semantics, the zone registry, and the protobuf schema.

The Scene Contract is the foundation on which every other RFC (Session/Protocol, Compositor, Lease, Input) depends. It must be right before those RFCs can be written.

---

## Motivation

tze_hud gives LLMs governed, performant presence on real screens. Every decision about how agents request tiles, how the compositor validates mutations, how reconnecting agents sync state, and how input is routed flows through the scene model. Without a precise contract:

- Agents have no stable surface to program against.
- The compositor cannot validate mutations deterministically.
- Tests cannot assert scene correctness without a GPU.
- Protocol versions cannot evolve without breaking changes.

The Scene Contract resolves all of these by specifying the scene graph as a pure data structure (DR-V1) with fully defined operations, invariants, and serialization formats.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| DR-V1: Scene separable from renderer | Scene graph is pure Rust data — no GPU types, no wgpu dependency. |
| DR-V3: Structured telemetry | Snapshot and diff are serializable; telemetry fields are defined here. |
| DR-V4: Deterministic test scenes | Scene is fully constructable and assertable in Layer 0 tests. |

---

## 1. Identity Model

### 1.1 Scheme Overview

tze_hud uses two ID classes:

| Class | Format | When Used |
|-------|--------|-----------|
| `SceneId` | UUIDv7 (time-ordered) | All live scene objects: tabs, tiles, nodes, leases, zones |
| `ResourceId` | BLAKE3 content hash (32 bytes, hex-encoded) | Immutable uploaded resources: images, fonts, raw buffers |

**UUIDv7** provides lexicographic sortability by creation time, useful for sequence ordering and log correlation without a separate timestamp field.

**Content-addressed ResourceId** provides deduplication at upload time: two agents uploading the same PNG get the same `ResourceId`; the runtime stores it once. Reference counting drives eviction (see §6).

```rust
/// Scene object ID — UUIDv7
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SceneId(uuid::Uuid);

impl SceneId {
    pub fn new() -> Self {
        SceneId(uuid::Uuid::now_v7())
    }
}

/// Immutable resource ID — BLAKE3 hex digest
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(String); // 64 hex chars
```

### 1.2 Namespace Isolation

Every agent session is assigned a **namespace** on authentication. Namespaces are string labels scoped to the runtime instance (e.g., `"weather-agent"`, `"calendar-agent"`).

Ownership hierarchy:

```
Runtime
└── Namespace  (agent identity boundary)
    ├── LeaseId  (governs access to tiles)
    │   └── TileId  (surface territory)
    │       └── NodeId  (content within tile)
    └── ResourceId  (uploaded assets; ref-counted globally)
```

**Cross-reference rules:**

1. A `TileId` belongs to exactly one namespace. Agents cannot reference tiles they do not own.
2. A `NodeId` belongs to exactly one `TileId`. Node IDs are unique scene-globally (not just within a tile) to support efficient diff addressing.
3. `ResourceId` is namespace-agnostic: resources are shared read-only across namespaces. An agent may reference a resource it did not upload if the runtime's sharing policy permits it (default: read-allowed, write-disallowed).
4. `ZoneId` is runtime-owned; agents do not create zones (in v1 — zone orchestration is a post-v1 feature). Both zone types and zone instances have `SceneId`s assigned by the runtime. Agents hold `ZonePublishToken` grants that permit publishing to a specific zone instance.
5. `LeaseId` scopes mutation rights to tiles. No tile operation is valid without a current lease on that tile's namespace.

### 1.3 ID Namespacing Diagram

```
┌─────────────────────────────────────────────────────────┐
│  Runtime                                                 │
│                                                          │
│  ┌───────────────────┐  ┌───────────────────┐           │
│  │ Namespace: "wtr"  │  │ Namespace: "cal"  │           │
│  │                   │  │                   │           │
│  │ lease-L1 ─► tile-T1  │  lease-L2 ─► tile-T2          │
│  │              node-N1 │              node-N3           │
│  │              node-N2 │              node-N4           │
│  └───────────────────┘  └───────────────────┘           │
│                                                          │
│  ┌────────────────────────────────────────────┐         │
│  │  Resource store (shared, content-addressed) │        │
│  │  res-abc123 ◄─ referenced by T1/N2         │         │
│  └────────────────────────────────────────────┘         │
│                                                          │
│  ┌────────────────────────────────────────────┐         │
│  │  Zone registry (runtime-owned)             │         │
│  │  types: subtitle, notification, …          │         │
│  │  instances: morning/subtitle, work/notif…  │         │
│  └────────────────────────────────────────────┘         │
└─────────────────────────────────────────────────────────┘
```

---

## 2. Scene Graph Structure

### 2.1 Hierarchy

The scene graph is a tree rooted at a single `Scene` node:

```
Scene
└── Tab[]  (ordered; one "active" tab at a time)
    └── Tile[]  (unordered set; z-order determines visual stack)
        └── Node[]  (ordered tree; composited front-to-back within tile)
```

**Formal tree invariants:**

1. A `Scene` has 0–256 `Tab` objects.
2. A `Tab` has 0–1024 `Tile` objects.
3. A `Tile` has 0–64 `Node` objects arranged as a tree (acyclic).
4. No `NodeId` appears more than once in the scene graph.
5. No `TileId` appears more than once in the scene graph.
6. No `TabId` appears more than once in the scene graph.

### 2.2 Tab

```rust
pub struct Tab {
    pub id: SceneId,
    pub name: String,           // Human-readable label; max 128 UTF-8 bytes
    pub display_order: u32,     // Determines tab bar ordering; unique per scene
    pub created_at_us: u64,     // UTC microseconds since Unix epoch (RFC 0003 §3.1)
}
```

**Invariants:**
- `display_order` values are unique across all tabs.
- `name` is non-empty.

### 2.3 Tile

```rust
pub struct Tile {
    pub id: SceneId,
    pub tab_id: SceneId,
    pub namespace: String,          // Owning agent namespace
    pub lease_id: SceneId,          // Current lease governing this tile

    // Geometry
    pub bounds: Rect,               // Position and size; must be within tab bounds
    pub z_order: u32,               // Higher = rendered on top within content layer

    // Visual
    pub opacity: f32,               // [0.0, 1.0]; 1.0 = fully opaque
    pub input_mode: InputMode,      // How input events are handled

    // Traffic / update semantics (from presence.md "Tiles are territories")
    pub latency_class: LatencyClass,   // Governs coalescing and scheduling priority
    pub update_policy: UpdatePolicy,   // How the compositor handles arriving content updates

    // Timing / coordination
    pub sync_group: Option<SceneId>, // Sync group membership; None = unsynchronized
    pub present_at_us: Option<u64>, // Scheduled presentation timestamp (μs, UTC); None = immediate (RFC 0003 §3.1)
    pub expires_at_us: Option<u64>, // Content expiry timestamp (μs, UTC); None = no auto-expiry (RFC 0003 §3.1)

    // Resource governance
    pub resource_budget: ResourceBudget,

    // Nodes
    pub root_node: Option<SceneId>, // Root of this tile's node tree; None = empty tile
}

pub struct Rect {
    pub x: f32,      // Left edge in logical pixels
    pub y: f32,      // Top edge in logical pixels
    pub width: f32,  // Must be > 0.0
    pub height: f32, // Must be > 0.0
}

pub enum InputMode {
    /// Events pass through to tiles below or to the desktop (overlay mode).
    Passthrough,
    /// Events are captured and forwarded to owning agent; no passthrough.
    Capture,
    /// Events are consumed locally (press states, focus) without agent forwarding.
    LocalOnly,
}

/// Governs how the compositor schedules and coalesces this tile's updates.
/// Corresponds to the four message classes in architecture.md §"Message classes".
pub enum LatencyClass {
    /// Transactional: reliable, ordered, acknowledged. For UI state changes.
    Transactional,
    /// State-stream: reliable, ordered, coalesced. For dashboard / continuous updates.
    StateStream,
    /// Ephemeral realtime: low-latency, droppable, latest-wins. For hover, cursor trails.
    EphemeralRealtime,
    /// Clocked media/cues: scheduled against media or display clock. For subtitles, AV sync.
    ClockedMedia,
}

/// How the compositor handles arriving content updates for this tile.
pub enum UpdatePolicy {
    /// Apply every update in order; never drop (required for transactional tiles).
    Ordered,
    /// Coalesce rapid updates; deliver only the most recent coherent view.
    Coalesce,
    /// Accept latest-wins; older updates are discarded if a newer one arrives.
    LatestWins,
}

/// Per-tile resource budget — embedded in every Tile and used during mutation validation.
/// This struct carries the per-tile limits enforced at scene-graph level.
///
/// NOTE on two-budget design: RFC 0008 §4 defines a separate `ResourceBudget` struct at the
/// *lease* level that includes per-session aggregate limits (`max_tiles`, `texture_bytes_total`,
/// `max_active_leases`, `max_concurrent_streams`) in addition to per-tile limits. These two structs
/// serve distinct purposes:
///   - `tze_scene::ResourceBudget` (this struct): per-tile limits embedded in Tile, enforced
///     during mutation validation in the scene graph layer.
///   - `tze_lease::ResourceBudget` (RFC 0008 §10 proto): lease-level budget carried in
///     LeaseRequest/LeaseResponse, governing aggregate session limits.
/// Implementations must not conflate the two. The lease-level budget populates the per-tile
/// budget at tile creation time; subsequent per-tile enforcement uses this struct.
pub struct ResourceBudget {
    pub texture_bytes: u64,    // Max texture memory for this tile's nodes
    pub update_rate_hz: f32,   // Max mutation rate (mutations/second)
    pub max_nodes: u8,         // Max nodes in tile tree; valid range [1, 64]; values > 64 are clamped to 64
                               // (u8 allows up to 255 by type but values above the scene hard cap are rejected)
}
```

**Tile invariants:**
1. `opacity` in `[0.0, 1.0]`.
2. `bounds` must be fully contained within the tab's display area (runtime-defined; typically the display resolution).
3. No two *agent-owned* tiles with the same `z_order` value on the same tab may both be non-passthrough and have overlapping bounds (exclusive-z conflict). Runtime-managed zone tiles are exempt from this check: they are pinned at z=MAX (content layer) or the chrome layer, which is outside the agent z_order space.
4. `width > 0.0` and `height > 0.0`.
5. `lease_id` must reference a currently-valid lease in the lease registry.
6. `resource_budget.max_nodes` in `[1, 64]`. The `u8` type allows values up to 255, but any value above 64 is rejected with `VALIDATION_ERROR_INVALID_FIELD_VALUE`.
7. `latency_class == ClockedMedia` requires `sync_group` to be `Some(_)` (clocked media tiles must belong to a sync group to be meaningful).
8. `update_policy` must be consistent with `latency_class`: `Transactional + Ordered`, `StateStream + Coalesce`, `EphemeralRealtime + LatestWins`, `ClockedMedia + Ordered` are the canonical pairings. Non-canonical pairings are accepted but generate a validation warning.

### 2.4 Node Types (V1)

All nodes share a common envelope:

```rust
pub struct Node {
    pub id: SceneId,
    pub children: Vec<SceneId>,  // Child node IDs; tree order determines compositing
    pub data: NodeData,
}

pub enum NodeData {
    SolidColor(SolidColorNode),
    TextMarkdown(TextMarkdownNode),
    StaticImage(StaticImageNode),
    HitRegion(HitRegionNode),
}
```

#### SolidColorNode

```rust
pub struct SolidColorNode {
    pub color: Rgba,   // [r, g, b, a] in [0.0, 1.0]
    pub bounds: Rect,  // Relative to tile origin
}

pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,  // Alpha; 0.0 = transparent, 1.0 = opaque
}
```

#### TextMarkdownNode

```rust
pub struct TextMarkdownNode {
    pub content: String,            // CommonMark markdown; max 65535 UTF-8 bytes
    pub bounds: Rect,               // Relative to tile origin
    pub font_size_px: f32,          // Must be > 0.0
    pub font_family: FontFamily,
    pub color: Rgba,
    pub background: Option<Rgba>,
    pub alignment: TextAlign,
    pub overflow: TextOverflow,
    pub present_at_us: Option<u64>, // Override tile-level present_at_us for this node (μs, UTC; RFC 0003 §3.1)
}

pub enum FontFamily {
    SystemSansSerif,
    SystemMonospace,
    SystemSerif,
    // Named font families added by compositor extension
}

pub enum TextAlign {
    Start,
    Center,
    End,
}

pub enum TextOverflow {
    Clip,
    Ellipsis,
    Scroll,   // V1: not yet interactive; deferred to post-v1
}
```

#### StaticImageNode

```rust
pub struct StaticImageNode {
    pub resource_id: ResourceId,    // Reference to uploaded image resource
    pub bounds: Rect,               // Relative to tile origin
    pub fit: ImageFit,
    pub present_at_us: Option<u64>, // Override tile-level present_at_us for this node (μs, UTC; RFC 0003 §3.1)
}

pub enum ImageFit {
    Contain,   // Scale to fit within bounds, preserve aspect ratio
    Cover,     // Scale to fill bounds, preserve aspect ratio, clip overflow
    Fill,      // Stretch to exactly fill bounds
    None,      // No scaling; clip to bounds
}
```

#### HitRegionNode

```rust
pub struct HitRegionNode {
    pub bounds: Rect,               // Relative to tile origin; the interactive area
    pub interaction_id: String,     // Agent-defined identifier; forwarded in input events
    pub accepts_focus: bool,        // Whether keyboard focus can land here
    pub accepts_pointer: bool,      // Whether pointer events are captured here

    // Input-model fields (see RFC 0004 §7.1 for full behavioral contract):
    pub auto_capture: bool,         // Acquire pointer capture automatically on PointerDown
    pub release_on_up: bool,        // Release capture on PointerUp (default: true)
    pub cursor_style: CursorStyle,  // Pointer cursor when hovering
    pub tooltip: Option<String>,    // Tooltip text shown after 500ms hover
    pub event_mask: EventMask,      // Which events this node receives
    pub accessibility: AccessibilityMetadata, // Agent-declared a11y properties
    pub local_style: LocalFeedbackStyle,      // Customizes press/hover/focus visuals
}
```

> **Note:** The four base fields (`bounds`, `interaction_id`, `accepts_focus`, `accepts_pointer`) are the scene-contract concern. The remaining fields are defined by RFC 0004 and are only relevant to the input subsystem. Implementations must treat all fields as part of the same message; the split is doctrinal, not structural.

`HitRegionNode` is the sole V1 interactive primitive. It has local state tracked by the compositor:

```rust
pub struct HitRegionLocalState {
    pub node_id: SceneId,
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
}
```

This local state drives immediate visual feedback (press states, focus rings) without waiting for agent acknowledgement.

### 2.5 Zone Registry

The zone registry is runtime-owned and loaded from configuration at startup. Agents cannot create zones in v1.

Zones have a **four-level ontology** (presence.md §"Zone anatomy") that prevents the common confusion of "is a zone the schema or the instance or the content?":

1. **Zone type** — the schema (accepted media types, contention policy, rendering policy defaults, privacy ceiling, interruption class, adjunct effects). Types are named identifiers, e.g. `"subtitle"`, `"notification"`.
2. **Zone instance** — a zone type bound into a specific tab with a geometry policy and a layer attachment. A "Morning" tab might have an instance of the `"notification"` type anchored to the top-right of the content layer. The same zone type can appear as multiple instances across different tabs.
3. **Publication** — one publish event into a zone instance: content payload, TTL, key (for merge-by-key zones), priority, privacy classification.
4. **Occupancy** — the runtime's resolved current state of a zone instance: what content is visible, which publications are active, what the effective geometry is after layout resolution. Agents can query occupancy but cannot set it directly.

```rust
pub struct ZoneRegistry {
    pub zone_types: HashMap<String, ZoneType>,      // key = type name (e.g., "subtitle")
    pub zone_instances: Vec<ZoneInstance>,           // All instances across all tabs
}

/// Zone type — the schema layer of the four-level zone ontology.
/// Defines accepted media, contention policy, rendering policy defaults, etc.
/// The same type can be instantiated multiple times across different tabs.
pub struct ZoneType {
    pub id: SceneId,
    pub name: String,                                // Stable identifier, e.g. "subtitle"
    pub description: String,
    pub accepted_media_types: Vec<ZoneMediaType>,
    pub rendering_policy: RenderingPolicy,
    pub contention_policy: ContentionPolicy,
    pub transport_constraint: Option<TransportConstraint>,
    pub auto_clear_us: Option<u64>,                  // Auto-clear timeout (μs duration); None = no auto-clear
    pub default_privacy: Option<ContentClassification>, // Default if publication omits classification
}

/// Zone instance — a zone type bound into a specific tab.
/// Carries the geometry policy and layer attachment for this placement.
/// Multiple instances of the same zone type may exist in different tabs.
pub struct ZoneInstance {
    pub id: SceneId,
    pub zone_type_name: String,                      // References ZoneType by name
    pub tab_id: SceneId,                             // The tab this instance belongs to
    pub layer_attachment: ZoneLayerAttachment,       // Which compositor layer this instance attaches to
    pub geometry_policy: GeometryPolicy,
}

/// Zone occupancy — the runtime's resolved current state of a zone instance.
/// Read-only from the agent's perspective. Produced by the runtime after applying
/// contention policy to all active publications for this instance.
/// Agents can query occupancy but cannot set it directly (presence.md §"Zone anatomy").
pub struct ZoneOccupancy {
    pub instance_id: SceneId,                        // References ZoneInstance
    pub active_publications: Vec<ZonePublishRecord>, // Publications currently active
    pub effective_geometry: Option<ResolvedGeometry>, // Geometry after display-profile layout resolution
}

/// Resolved geometry — the runtime's concrete pixel-level placement for a zone instance
/// after the display profile and tab layout have been applied.
pub struct ResolvedGeometry {
    pub x_px: f32,
    pub y_px: f32,
    pub width_px: f32,
    pub height_px: f32,
}

/// Minimum z_order value reserved for runtime-managed zone tiles in the content layer.
/// Agent-owned tiles must have z_order < ZONE_TILE_Z_MIN. This reservation keeps
/// Content-layer zone tiles above all agent tiles without introducing a sub-layer.
/// The upper 2^31 values (0x8000_0000 ..= 0xFFFF_FFFF) are the zone-reserved band.
pub const ZONE_TILE_Z_MIN: u32 = 0x8000_0000;

/// Which compositor layer a zone instance attaches to (presence.md §"Layer attachment").
pub enum ZoneLayerAttachment {
    /// Behind all agent tiles; ambient-background zone.
    Background,
    /// Within the content layer's z-order space, at a reserved high z-order range
    /// (z_order >= ZONE_TILE_Z_MIN = 0x8000_0000). Agent-owned tiles must use z_order
    /// values below this threshold. This is not a separate sub-layer — zone tiles
    /// participate in the same content-layer z-order traversal as agent tiles; they
    /// simply occupy the upper reserved band. subtitle, notification, pip.
    Content,
    /// Above all agent content; rendered by runtime using zone's policy.
    /// alert-banner, status-bar. Agents publish data; runtime renders in chrome.
    Chrome,
}

pub enum GeometryPolicy {
    /// Percentage-based position relative to display area
    Relative {
        x_pct: f32,
        y_pct: f32,
        width_pct: f32,
        height_pct: f32,
    },
    /// Anchored to a display edge
    EdgeAnchored {
        edge: DisplayEdge,
        height_pct: f32,   // For top/bottom edges
        width_pct: f32,    // For left/right edges
        margin_px: f32,
    },
}

pub enum DisplayEdge { Top, Bottom, Left, Right }

pub enum ZoneMediaType {
    StreamText,         // Stream-text with optional breakpoints
    ShortTextWithIcon,  // Notification: text + icon + urgency
    KeyValuePairs,      // Status-bar: key-value map
    VideoSurfaceRef,    // Reference to a media surface (post-v1 media layer)
    StaticImage,        // Static image resource
    SolidColor,         // Solid color fill
}

pub struct RenderingPolicy {
    pub font_size_px: Option<f32>,
    pub backdrop: Option<Rgba>,
    pub text_align: Option<TextAlign>,
    pub margin_px: Option<f32>,
}

pub enum ContentionPolicy {
    /// Most recent publish replaces previous content
    LatestWins,
    /// Publishes accumulate as a stack; each auto-dismisses
    Stack { max_depth: u8 },
    /// Each publish includes a key; same key replaces, different keys coexist
    MergeByKey { max_keys: u8 },
    /// Only one occupant; new publish evicts current one
    Replace,
}

pub enum TransportConstraint {
    /// Content must arrive via gRPC session stream
    GrpcOnly,
    /// Content may arrive via MCP tool call
    McpAllowed,
    /// Content requires WebRTC media channel (post-v1)
    WebRtcRequired,
}

/// Privacy classification for zone publication content (presence.md §"Zone anatomy").
/// The privacy gate (RFC 0009 §2.3) uses this to determine redaction behavior.
/// Values match the viewer access matrix in RFC 0009 §2.3.
pub enum ContentClassification {
    /// Visible to all viewers including Unknown and Nobody.
    Public,
    /// Visible to Owner and HouseholdMember. Redacted for KnownGuest, Unknown, Nobody.
    Household,
    /// Visible to Owner only. Redacted for all others.
    Private,
    /// Visible to Owner only (same as Private but signals higher sensitivity).
    /// Runtime may apply additional redaction indicators.
    Sensitive,
}
```

**V1 scope note:** In v1, zone instances are static — loaded from configuration at startup, one instance per tab per zone type. The `ZoneOccupancy` struct and the `effective_geometry` field are defined here for full ontological correctness but their query API is deferred to post-v1. The runtime maintains occupancy state internally; v1 snapshots include active publications per instance (see §4.1) but do not expose `effective_geometry`.

**Zone-to-tile mapping:** The runtime creates and manages internal tiles for each active zone instance. Zone tiles are in a runtime-owned namespace. The `layer_attachment` field on `ZoneInstance` (see §2.5 struct) determines which compositor layer the zone's tile occupies. This RFC uses the three-layer model defined in architecture.md §"Layer stack" (background / content / chrome) — no fourth layer is introduced:
- `Background` zones render behind all agent tiles (ambient-background).
- `Content` zones are realized as runtime-managed tiles **within the content layer's z-order space**. Zone tiles occupy a reserved upper z-order band: `z_order >= ZONE_TILE_Z_MIN` where `ZONE_TILE_Z_MIN = 0x8000_0000`. Agent-assigned z_order values must remain below this threshold (enforced at transaction validation time, §3.3 Budget Check). Zone tiles therefore appear above all agent tiles in the content layer's traversal without requiring a separate sub-layer (subtitle, notification, pip).
- `Chrome` zones render above all content; agents publish data but the runtime renders it (alert-banner, status-bar).

Agent tiles cannot occlude Content-layer zone tiles because the reserved z-order band is enforced: no agent tile may be assigned a z_order value within the zone-reserved range. Chrome-layer zone tiles are entirely outside the z_order space of agent tiles.

**Contention policies:**

| Policy | Zones | Semantics |
|--------|-------|-----------|
| `LatestWins` | subtitle, ambient-background | Most recent publish replaces; no queue |
| `Stack` | notification | Accumulates; each auto-dismisses after timeout |
| `MergeByKey` | status-bar | Key-addressed; same key replaces, different coexist |
| `Replace` | pip (post-v1) | Single occupant; new publish evicts current |

---

### 2.6 Widget Registry

The widget registry is runtime-owned. Widget definitions may be introduced through two registration paths: (1) startup asset-bundle bootstrapping and (2) runtime SVG asset registration. It parallels the zone registry (§2.5) in structure and lifecycle.

In v1, agents do not mutate widget instances directly. They publish parameters/content to registered instances and may register new widget visual assets only through a dedicated runtime register/upload API with explicit capability grants.

Widgets have the same **four-level ontology** as zones (presence.md §"Widget anatomy"):

1. **Widget type** — the schema and visual assets (parameter declarations, SVG layers with parameter bindings, default geometry/contention/rendering policies). Types are named identifiers introduced via startup bundle load or runtime registration, e.g. `"gauge"`, `"progress-bar"`.
2. **Widget instance** — a widget type bound into a specific tab with a geometry policy and layer attachment. Declared in configuration under `[[tabs.widgets]]`. Multiple instances of the same type may exist on different tabs or on the same tab when disambiguated by `instance_id`.
3. **Publication** — one publish event into a widget instance: a set of typed parameter values (f32, string, color, or enum), TTL, optional merge key (for MergeByKey contention), and optional transition duration in milliseconds.
4. **Occupancy** — the runtime's resolved render state: effective parameter values after contention policy application. The compositor reads `effective_params` to determine current visual property values and re-rasterizes only when effective parameters change.

```rust
pub struct WidgetRegistry {
    pub definitions: BTreeMap<String, WidgetDefinition>,   // key = widget type id (e.g., "gauge")
    pub instances: Vec<WidgetInstance>,                     // All instances across all tabs
}

/// Widget type definition — the schema and visual asset layer of the four-level ontology.
pub struct WidgetDefinition {
    pub id: String,                                         // Kebab-case identifier, e.g. "gauge"
    pub name: String,                                       // Human-readable name
    pub description: String,
    pub parameter_schema: Vec<WidgetParameterDeclaration>,  // Ordered list of parameter declarations
    pub layers: Vec<WidgetSvgLayer>,                        // SVG layers with parameter bindings
    pub default_geometry_policy: GeometryPolicy,
    pub default_rendering_policy: RenderingPolicy,
    pub default_contention_policy: ContentionPolicy,
    pub ephemeral: bool,                                    // If true, publishes are fire-and-forget (no WidgetPublishResult)
}

/// Parameter declaration in a widget type's schema.
pub struct WidgetParameterDeclaration {
    pub name: String,                                       // Snake-case parameter identifier
    pub param_type: WidgetParamType,
    pub default_value: WidgetParameterValue,
    pub constraints: Option<WidgetParamConstraints>,        // Range (f32), max_length (string), allowed_values (enum)
}

pub enum WidgetParamType { F32, String, Color, Enum }

pub enum WidgetParameterValue {
    F32(f32),
    String(String),
    Color(Rgba),
    Enum(String),
}

pub struct WidgetParamConstraints {
    pub min: Option<f32>,               // f32 range lower bound
    pub max: Option<f32>,               // f32 range upper bound
    pub max_length: Option<u32>,        // String max byte length (default 1024)
    pub allowed_values: Vec<String>,    // Enum allowed values
}

/// SVG layer with parameter bindings.
pub struct WidgetSvgLayer {
    pub svg_resource_id: ResourceId,    // Content-addressed SVG resource
    pub bindings: Vec<WidgetBinding>,   // Parameter-to-attribute mappings
}

/// A single parameter binding within an SVG layer.
pub struct WidgetBinding {
    pub param: String,              // References WidgetParameterDeclaration.name
    pub target_element: String,     // SVG element id attribute
    pub target_attribute: String,   // SVG attribute name, or "text-content" for text node content
    pub mapping: WidgetBindingMapping,
}

pub enum WidgetBindingMapping {
    /// f32 parameter: linear map from [param.min, param.max] to [attr_min, attr_max]
    Linear { attr_min: f32, attr_max: f32 },
    /// string or color parameter: direct substitution into attribute value
    Direct,
    /// enum parameter: discrete lookup table from enum value to attribute value
    Discrete { values: BTreeMap<String, String> },
}

/// Widget instance — a widget type bound into a specific tab.
pub struct WidgetInstance {
    pub instance_name: String,                          // Addressing key for publish ops (explicit instance_id or widget_type_name)
    pub widget_type_name: String,                       // References WidgetDefinition.id
    pub tab_id: SceneId,
    pub geometry_override: Option<GeometryPolicy>,
    pub contention_override: Option<ContentionPolicy>,
    pub current_params: BTreeMap<String, WidgetParameterValue>,  // Current effective parameter values
}

/// Widget publish record — one publication into a widget instance.
pub struct WidgetPublishRecord {
    pub widget_name: String,            // Identifies the widget instance
    pub publisher_namespace: String,
    pub params: BTreeMap<String, WidgetParameterValue>,
    pub published_at_wall_us: u64,
    pub merge_key: Option<String>,      // For MergeByKey contention
    pub expires_at_wall_us: Option<u64>,
    pub transition_ms: u32,
}

/// Widget occupancy — resolved render state after contention policy application.
pub struct WidgetOccupancy {
    pub widget_name: String,
    pub tab_id: SceneId,
    pub active_publications: Vec<WidgetPublishRecord>,
    pub occupant_count: u32,
    pub effective_params: BTreeMap<String, WidgetParameterValue>,  // Compositor reads this for rendering
}

/// Minimum z_order value reserved for runtime-managed widget tiles in the content layer.
/// Widget tiles use z_order >= WIDGET_TILE_Z_MIN, which is above ZONE_TILE_Z_MIN (0x8000_0000).
/// This places widget tiles above zone tiles when they overlap spatially.
pub const WIDGET_TILE_Z_MIN: u32 = 0x9000_0000;
```

**Widget parameter binding model:** Each SVG layer in a widget definition declares bindings that map parameter names to SVG attributes. Three mapping types are supported:
- `Linear`: f32 parameters only. Maps the parameter's `[min, max]` range to an `[attr_min, attr_max]` SVG attribute value range via linear interpolation. Example: `fill_level` 0.0–1.0 maps to SVG `height` attribute 0–200 pixels.
- `Direct`: string and color parameters. The parameter value is substituted directly as the SVG attribute value. Color is serialized as `#RRGGBBAA` hex. String is placed verbatim.
- `Discrete`: enum parameters. A lookup table maps each allowed enum value to a specific SVG attribute value.

The special target attribute `"text-content"` replaces the text node content of a `<text>` or `<tspan>` SVG element (the character data), not an XML attribute.

**Widget parameter interpolation:** When `transition_ms > 0`, the compositor interpolates between old and new resolved SVG attribute values over the transition duration. f32 parameters use linear interpolation; color parameters use component-wise sRGB linear interpolation. String and enum parameters snap to the new value immediately at t=0 (no meaningful intermediate state). During interpolation, the compositor re-rasterizes at the display refresh rate (target_fps from the active display profile).

**Widget-to-tile mapping:** The runtime creates and manages internal tiles for each widget instance. Widget tiles are in a runtime-owned namespace. Widget tiles default to `input_mode = Passthrough` — they do not contain `HitRegionNode` children and do not capture input events. Input events pass through widget tiles to tiles beneath them.
- `Background` widget instances render behind all agent tiles.
- `Content` widget instances are realized as runtime-managed tiles at `z_order >= WIDGET_TILE_Z_MIN = 0x9000_0000`, above zone tiles.
- `Chrome` widget instances render above all agent content; the runtime renders them using the widget's visual assets.

**Widget asset format (bootstrapped + runtime):** Widget type definitions may be loaded from startup asset bundles (directories containing `widget.toml` + SVG files) and may also be registered at runtime via a dedicated upload/register API. The bundle path remains unchanged for bootstrapping: the runtime scans configured bundle directories at startup, parses manifest/schema/layers, and skips invalid bundles without halting startup. Runtime registration validates the uploaded SVG asset and type schema under the same structural rules before inserting into the registry.

**V1 scope note:** Widget instance bindings remain configuration-first in v1 (loaded at startup). The `WidgetOccupancy` struct is defined here for full ontological correctness. V1 snapshots include active widget publications per instance. Widget definitions may be bootstrapped from bundles and extended at runtime via register/upload. No built-in widget types ship with the runtime binary. An empty widget registry (no bundles configured and no runtime registrations yet) is valid.

**Widget registry in SceneSnapshot:** The scene snapshot (§4.1) includes a `WidgetRegistrySnapshot` alongside the `ZoneRegistrySnapshot`. It captures all widget definitions, all widget instances across all tabs, and all active widget publish records. Serialization uses `BTreeMap` ordering for determinism.

---

## 3. Transaction Model

### 3.1 Mutation Batch Format

An agent submits mutations as an atomic batch:

```rust
pub struct MutationBatch {
    pub batch_id: SceneId,          // Agent-assigned; used in error responses
    // agent_namespace is NOT carried in the batch; it is derived from the
    // authenticated session context by the runtime.  See security.md §'Authentication'
    // and §'Capability scopes'.  Clients must never supply their own identity per-message.
    pub mutations: Vec<SceneMutation>,
    pub present_at_us: Option<u64>, // Apply in one frame at or after this time (μs, UTC; RFC 0003 §3.1)
    pub sequence_hint: Option<u64>, // Agent's local sequence number; for ordering hints
}

pub enum SceneMutation {
    // Tab operations
    CreateTab { id: SceneId, name: String, display_order: u32 },
    DeleteTab { tab_id: SceneId },
    RenameTab { tab_id: SceneId, name: String },
    ReorderTab { tab_id: SceneId, display_order: u32 },
    SwitchActiveTab { tab_id: SceneId },

    // Tile operations
    CreateTile { tile: Tile },
    UpdateTileBounds { tile_id: SceneId, bounds: Rect },
    UpdateTileZOrder { tile_id: SceneId, z_order: u32 },
    UpdateTileOpacity { tile_id: SceneId, opacity: f32 },
    UpdateTileInputMode { tile_id: SceneId, input_mode: InputMode },
    UpdateTileSyncGroup { tile_id: SceneId, sync_group: Option<SceneId> },
    UpdateTileExpiry { tile_id: SceneId, expires_at_us: Option<u64> },
    DeleteTile { tile_id: SceneId },

    // Node operations
    SetTileRoot { tile_id: SceneId, node: Node },
    InsertNode { tile_id: SceneId, parent_id: Option<SceneId>, node: Node },
    ReplaceNode { node_id: SceneId, node: Node },
    UpdateNodeBounds { node_id: SceneId, bounds: Rect },
    RemoveNode { node_id: SceneId },

    // Zone publishing
    PublishToZone {
        // zone_name identifies the zone type name (e.g. "subtitle"). The runtime resolves
        // this to the ZoneInstance for the publishing agent's active tab. Per presence.md
        // §"Two APIs", agents publish by zone type name with minimal context; the runtime
        // resolves the type name to the correct ZoneInstance internally.
        zone_name: String,
        content: ZoneContent,
        publish_token: ZonePublishToken,
        expires_at_us: Option<u64>,   // Per-publish TTL (μs, UTC); None = use zone auto_clear_us
        publish_key: Option<String>,  // Key for MergeByKey zones; None for other policies
        // Per-publication privacy classification (presence.md §"Zone anatomy" — publications
        // carry "privacy classification" as a first-class field). The runtime's privacy gate
        // (RFC 0009 §2.3) evaluates this against the current viewer class and may redact.
        // None = inherit the zone's default_privacy classification from configuration.
        // Effective classification = max(declared, zone_default_classification) — an agent
        // cannot escalate visibility beyond the zone's ceiling (RFC 0009 §2.3).
        content_classification: Option<ContentClassification>,
    },
    ClearZone { zone_name: String, publish_token: ZonePublishToken },

    // Sync group lifecycle (RFC 0003 §7.2). Must be used before tiles can be assigned to the group.
    // CreateSyncGroup: group ID is embedded in config.id (SyncGroupConfig.id, RFC 0003 §7.1).
    CreateSyncGroup { config: SyncGroupConfig }, // config.id is the group SceneId; no separate id field
    DeleteSyncGroup { sync_group_id: SceneId },  // member tiles remain, unlinked
}
```

### 3.2 Transaction Pipeline

```
Agent submits MutationBatch
         │
         ▼
┌─────────────────────────────────────────────────────────────┐
│  Stage: Parse + Deserialize                                  │
│  - Deserialize protobuf                                     │
│  - Validate structural integrity (no nulls where required)  │
│  - Max batch size: 1000 mutations                           │
└───────────────────────────┬─────────────────────────────────┘
                            │ fail → BatchRejected(ParseError)
                            ▼
┌─────────────────────────────────────────────────────────────┐
│  Validate: Per-mutation checks (all-or-nothing)             │
│                                                             │
│  For each mutation in order:                                │
│  1. Lease check     — agent holds valid lease for target    │
│  2. Budget check    — mutation within resource budget       │
│  3. Bounds check    — new geometry within tab area          │
│  4. Type check      — field values within valid ranges      │
│  5. Invariant check — post-mutation state satisfies         │
│                       all scene invariants                  │
│                                                             │
│  Any failure → entire batch rejected; no partial apply     │
└───────────────────────────┬─────────────────────────────────┘
                            │ any fail → BatchRejected(ValidationError)
                            │ all pass ↓
                            ▼
┌─────────────────────────────────────────────────────────────┐
│  Commit: Atomic application in one frame                    │
│  - Acquire write lock on scene graph                        │
│  - Apply all mutations in batch order                       │
│  - Increment global scene sequence number (monotonic u64)  │
│  - Release write lock                                       │
│  - Emit BatchCommitted(batch_id, sequence_number)           │
│  (WAL append deferred to post-v1; see §4.2)                 │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
                     Compositor picks up
                     next frame delta
```

**Pipeline performance requirements:**
- Validation: < 200μs per batch of 10 mutations
- Commit (lock-acquire through lock-release): < 50μs for 10 mutations
- Full path (parse → validate → commit): < 300μs p99 for 10 mutations on reference hardware

### 3.3 Validation Rules

#### Lease Check

Tab mutations (`CreateTab`, `DeleteTab`, `RenameTab`, `ReorderTab`, `SwitchActiveTab`) require the `manage_tabs` capability (canonical name per RFC 0006 §6.3). Tab operations are not gated by a lease — they are capability-gated only, because tabs are not agent-owned resources.

```
tab mutation (CreateTab / DeleteTab / RenameTab / ReorderTab / SwitchActiveTab)
  → agent holds manage_tabs capability          (canonical name per RFC 0006 §6.3; earlier drafts used MANAGE_TABS — that identifier is non-canonical)
```

Tile mutations require a valid lease:

```
mutation targets tile T
  → T.namespace == session.agent_namespace      (agent owns tile; namespace from authenticated session, not batch payload)
  → lease_registry.get(T.lease_id).is_valid()  (lease not expired or revoked; per RFC 0008 §11 clarification: only ACTIVE leases are valid)
  → agent holds modify_own_tiles capability     (canonical name per RFC 0006 §6.3; earlier drafts used WRITE_SCENE — that identifier is non-canonical)
```

`CreateTile` also requires the `create_tiles` capability in addition to `modify_own_tiles`.

Zone publish mutations require `ZonePublishToken` embedded in the mutation; the token is validated against the capability registry. The agent must also hold `publish_zone:<zone_name>` capability (RFC 0006 §6.3). The runtime resolves `zone_name` (a zone type name) to the `ZoneInstance` for the agent's active tab; the mutation is rejected with `ZoneNotFound` if no such instance exists in the active tab.

Sync group mutations (`CreateSyncGroup`, `DeleteSyncGroup`) require the `manage_sync_groups` capability. A sync group is a scene-level object not tied to a specific tile, so it is capability-gated rather than lease-gated.

> **Capability name note:** The canonical capability naming scheme uses `snake_case` for all identifiers (RFC 0006 §6.3). `CREATE_TILE`, `WRITE_SCENE`, and `MANAGE_TABS` are earlier informal names used in some RFC diagrams and examples. The authoritative names are `create_tiles`, `modify_own_tiles`, and `manage_tabs` respectively.

#### Budget Check

For `CreateTile`:
- `agent.active_leases.len() < agent.max_leases`
- `tile.z_order < ZONE_TILE_Z_MIN` (= `0x8000_0000`): agent-owned tiles must not use the zone-reserved z-order band (see §2.5 Zone-to-tile mapping). Violations are rejected with `INVALID_Z_ORDER`.

For `InsertNode` / `ReplaceNode`:
- `tile.nodes.len() < tile.resource_budget.max_nodes`
- `new_node.estimated_texture_bytes() + tile.current_texture_bytes <= tile.resource_budget.texture_bytes`

For any mutation:
- Mutation rate for this agent in the current second < `agent.update_rate_hz_budget`

#### Bounds Check

For `CreateTile` / `UpdateTileBounds`:
- `tile.bounds.x >= 0.0`
- `tile.bounds.y >= 0.0`
- `tile.bounds.x + tile.bounds.width <= tab_display_width`
- `tile.bounds.y + tile.bounds.height <= tab_display_height`
- `tile.bounds.width > 0.0`
- `tile.bounds.height > 0.0`

For `UpdateNodeBounds` / node creation:
- Node bounds are relative to tile origin; no constraint on node bounds exceeding tile size (nodes may be clipped).

#### Type Check

- `opacity` in `[0.0, 1.0]`
- `z_order`: valid u32
- `font_size_px > 0.0`
- `content` in `TextMarkdownNode` must be valid UTF-8 and ≤ 65535 bytes
- `ResourceId` references a resource known to the runtime

#### Invariant Check (post-mutation simulation)

Before committing, the validator simulates the full post-batch state and checks:

1. No `TileId` collision in scene.
2. No `NodeId` collision in scene.
3. No acyclic-tree violation in any tile's node tree.
4. No exclusive z-order conflict: no two non-passthrough tiles on same tab share `z_order` with overlapping bounds.
5. All `TileId` references in `tab.tiles` have corresponding `Tile` records.
6. All `NodeId` references in `node.children` have corresponding `Node` records within the same tile.

### 3.4 Rejection Response

```rust
pub struct BatchRejected {
    pub batch_id: SceneId,
    pub error: BatchError,
}

pub enum BatchError {
    ParseError { message: String },
    MutationValidationError {
        mutation_index: usize,    // 0-based index into batch.mutations
        mutation_type: String,    // e.g., "CreateTile"
        code: ValidationErrorCode,
        message: String,          // Human-readable explanation
        context: serde_json::Value, // Machine-readable: {field, value, constraint}
        correction_hint: Option<serde_json::Value>,
    },
    BatchSizeExceeded { max: usize, got: usize },
    RateLimitExceeded {
        limit_hz: f32,   // Agent's allowed rate budget (mutations/second)
        current_hz: f32, // Observed rate that triggered the limit
    },
}

pub enum ValidationErrorCode {
    LeaseExpired,
    LeaseNotFound,
    CapabilityMissing,
    BudgetExceeded,
    BoundsOutOfRange,
    ZOrderConflict,
    NodeCountExceeded,
    InvalidFieldValue,
    DuplicateId,
    CycleDetected,
    ResourceNotFound,
    ZonePublishTokenInvalid,
    ZoneMediaTypeMismatch,
    TabNotFound,
    TileNotFound,
    NodeNotFound,
}
```

All error codes are stable across minor versions. The `context` and `correction_hint` fields provide agent-readable remediation guidance.

### 3.5 Concurrency Model

- The scene graph has a single writer lock. One batch commits at a time.
- Multiple batches arriving in the same frame are serialized by arrival order at the runtime's ingestion queue.
- Batches from the same agent are guaranteed to commit in submission order.
- Batches from different agents may interleave, but each batch's mutations are contiguous (no interleaving within a batch).
- The committed `sequence_number` is monotonically increasing; it is the canonical ordering token for scene history.

---

## 4. Snapshots and Diffs

### 4.1 Full Scene Snapshot

A snapshot is a complete, deterministic serialization of the scene graph at a specific sequence number.

```rust
pub struct SceneSnapshot {
    pub sequence: u64,                      // Commit sequence at time of snapshot
    pub timestamp_us: u64,                  // Wall clock at snapshot time (μs, UTC; RFC 0003 §3.1)
    pub tabs: Vec<Tab>,                     // Ordered by display_order
    pub tiles: Vec<Tile>,                   // All tiles across all tabs
    pub nodes: Vec<Node>,                   // All nodes across all tiles
    pub zone_registry: ZoneRegistrySnapshot,
    pub widget_registry: WidgetRegistrySnapshot,
    pub active_tab: Option<SceneId>,
    pub checksum: [u8; 32],                 // BLAKE3 of canonical serialization
}

/// Zone registry snapshot — captures all four levels of the zone ontology.
/// Zone types and instances are stable (loaded from config in v1).
/// Active publications and occupancy reflect live runtime state.
pub struct ZoneRegistrySnapshot {
    pub zone_types: Vec<ZoneType>,             // All zone type schemas
    pub zone_instances: Vec<ZoneInstance>,     // All zone instances across all tabs
    pub active_publishes: Vec<ZonePublishRecord>, // Active publications, keyed by instance_id
    // Note: ZoneOccupancy.effective_geometry is internal in v1; occupancy is reconstructable
    // from active_publishes grouped by instance_id.
}

/// Widget registry snapshot — captures all four levels of the widget ontology.
/// Widget types (loaded from asset bundles) and instances (from config) are stable in v1.
/// Active publications reflect live runtime state.
/// Serialization uses BTreeMap ordering for checksum determinism.
pub struct WidgetRegistrySnapshot {
    pub definitions: Vec<WidgetDefinition>,         // All widget type definitions
    pub instances: Vec<WidgetInstance>,             // All widget instances across all tabs
    pub active_publishes: Vec<WidgetPublishRecord>, // Active publications, keyed by widget_name + publisher
}

pub struct ZonePublishRecord {
    pub instance_id: SceneId,                  // References ZoneInstance.id
    pub zone_type_name: String,                // Zone type name (for readability; resolved via instance)
    pub publisher_namespace: String,
    pub content: ZoneContent,
    pub published_at_us: u64,                  // UTC microseconds since Unix epoch (RFC 0003 §3.1)
    pub expires_at_us: Option<u64>,            // Publication TTL (μs, UTC); None = governed by zone auto_clear_us
    pub publish_key: Option<String>,           // Key for MergeByKey zones; None for other contention policies
    // Per-publication privacy classification (presence.md §"Zone anatomy"; RFC 0009 §2.3).
    // Preserved in snapshot so that reconnecting agents and the privacy gate can reconstruct
    // the effective classification of each active zone publication after reconnect.
    // None = publication inherited the zone's default classification.
    pub content_classification: Option<ContentClassification>,
}
```

**Serialization properties:**
- Fields serialized in deterministic order (no HashMap; use BTreeMap for any map types in snapshot).
- Floating-point values serialized to 6 significant decimal digits in text format; binary format uses IEEE 754 little-endian.
- `SceneId` serialized as 16-byte little-endian UUID.
- `ResourceId` serialized as 32-byte BLAKE3 digest.

**Performance requirement:** < 1ms to serialize a 100-tile scene (10 nodes/tile average = 1000 nodes total) on reference hardware (measured as protobuf encoding time on a single core at 3GHz equivalent).

### 4.2 Incremental Diff (Future Extension — Deferred from V1)

> **V1 scope note:** v1.md explicitly defers "resumable state sync". In v1, reconnecting agents always receive a full snapshot (§4.1) — there is no incremental diff path. The diff infrastructure described below (WAL, SceneDiff, DiffOp, branching reconnect logic) is documented here as a planned future extension only. Do not implement it in v1.

Incremental diff is deferred because:
- Reconnect-via-snapshot is simpler to implement and test correctly.
- WAL retention and coalescing add complexity and memory pressure that is unjustified before the core scene model is proven.
- The full snapshot path is required regardless (for cold reconnects), so v1 ships only the full snapshot path.

**V1 reconnect behavior:** When an agent reconnects, the runtime always sends a full snapshot regardless of how recently the agent was connected. The agent discards its prior scene state, applies the snapshot, and resumes from the snapshot's sequence number.

**Post-v1 (incremental diff):** Once the core compositor and lease system are stable, the runtime may add a WAL-backed incremental diff path. Reconnecting agents with a recent sequence number would receive only the delta. This is a protocol-compatible addition: the reconnect RPC can offer both snapshot and diff modes, selected by negotiation. Implementation is deferred.

---

## 5. Hit-Testing

### 5.1 Spatial Query

Hit-testing maps a 2D point to the deepest interactive element at that point. It is used for input routing (pointer, touch) and scene inspection.

```rust
pub struct HitTestQuery {
    pub tab_id: SceneId,
    pub point: Point2D,     // Display-space coordinates
}

pub struct Point2D {
    pub x: f32,
    pub y: f32,
}

pub struct HitTestResult {
    pub kind: HitTestKind,
}

pub enum HitTestKind {
    /// Hit a specific node within a tile
    NodeHit {
        tile_id: SceneId,
        node_id: SceneId,
        local_coords: Point2D,   // Coordinate relative to node bounds origin
        tile_local_coords: Point2D, // Coordinate relative to tile origin
    },
    /// Hit a tile but no interactive node within it
    TileHit {
        tile_id: SceneId,
        local_coords: Point2D,   // Coordinate relative to tile origin
    },
    /// No tile hit; passes through to desktop (overlay mode) or tab background
    Passthrough,
    /// Hit the chrome layer (runtime UI)
    Chrome { element: ChromeElement },
}

pub enum ChromeElement {
    TabBar,
    SystemIndicator,
    OverrideControl,
    DisconnectionBadge { agent_namespace: String },
}
```

**Performance requirement:** < 100μs for a single point query against 50 tiles.

### 5.2 Traversal Order

Hit-testing traverses the layer stack in top-to-bottom (front-to-back) order:

```
1. Chrome layer (always wins)
   → if point is in a chrome element → return Chrome hit

2. Content layer: tiles ordered by z_order descending (highest z first)
   For each tile T (highest z_order first):
     a. If T.input_mode == Passthrough → skip
     b. If point not in T.bounds → skip
     c. Traverse T's node tree in reverse tree order (last child first → root):
        For each node N:
          i.  If N is HitRegionNode and N.bounds contains point → return NodeHit
          ii. Continue to next node
     d. If no node hit but point is in T.bounds → return TileHit(T)

3. If no tile hit → return Passthrough
```

**Zone tiles in the content layer:** Content-layer zone tiles (subtitle, notification, pip) are runtime-owned tiles with `z_order >= ZONE_TILE_Z_MIN = 0x8000_0000`. They participate in the same traversal as agent tiles (step 2 above) — no special-casing is required. Because their z_order values exceed all agent-owned tiles (which are validated to have z_order < ZONE_TILE_Z_MIN), zone tiles are encountered first in the descending traversal. Zone tile `input_mode` is `Passthrough` by default, meaning they do not intercept pointer events unless a zone explicitly declares interactive capability. Chrome-layer zones (status-bar, alert-banner) are handled in step 1 by the chrome layer check.

**Diagram:**

```
Input point P
     │
     ▼
Chrome layer?   ──yes──► Chrome hit
     │ no
     ▼
Tiles (z descending)
     │
     ├── Zone tile z=0x8000_0003 (subtitle): Passthrough mode? → skip
     ├── Tile z=10: Passthrough mode? → skip
     ├── Tile z=8:  P in bounds?      → yes
     │              Nodes (reverse tree order):
     │              ├── HitRegion N3: P in N3.bounds? → yes → NodeHit(T, N3)
     │              └── (not reached)
     └── (not reached)
```

### 5.3 Hit-Test Result Usage

| Result | Input Routing Action |
|--------|---------------------|
| `Chrome` | Runtime handles locally; agent not notified |
| `NodeHit` | Runtime updates local state (hover/press), forwards event to tile's owning agent |
| `TileHit` | Event forwarded to tile's owning agent (no node-level local state) |
| `Passthrough` | Overlay mode: event passed to desktop; fullscreen mode: discarded |

---

## 6. Durable vs Ephemeral State

### 6.1 Durable State (survives restart)

Durable state is stored on disk and reloaded at runtime startup.

| Category | Contents | Storage |
|----------|----------|---------|
| Agent registrations | Agent identity, auth credentials (hashed), default capability grants | Config file |
| Tab configuration | Tab names, display order, default layouts | Config file |
| Zone registry | Zone types (schemas) and zone instances (tab bindings, geometry, layer attachment) | Config file |
| User preferences | Quiet hours, safe mode config, display profiles | Config file |
| Capability grants | Per-agent capability scope definitions | Config file |
| Runtime widget SVG assets | Runtime-registered widget SVG blobs (content-addressed) | Runtime-managed local asset store (filesystem) |

Durable state is written to disk on change; it is not part of the scene graph serialization.

### 6.2 Ephemeral State (lost on restart)

Ephemeral state lives entirely in memory. After a restart, agents must re-establish sessions and re-create scene content.

| Category | Notes |
|----------|-------|
| Scene graph | All tabs, tiles, nodes are recreated by agents after reconnect |
| Active leases | All leases expire on restart; agents re-request |
| Live zone publishes | Zone content is cleared on restart (tabs, zone types, and zone instances persist) |
| gRPC sessions | All sessions disconnected; agents must reconnect |
| Hit-region local state | Hover/press/focus state is reset |
| WAL / diff history | Lost on restart (no durable replay) |
| Performance telemetry | Per-session metrics discarded |

**Design rationale:** Making the scene graph ephemeral simplifies the correctness model dramatically. Agents are expected to re-create their scene state on reconnect. The lease governance model ensures they can do so within their granted capability scope. In v1, scene-node resources remain ephemeral, while runtime-registered widget SVG assets are carved out as durable infrastructure to preserve two-stage publish efficiency across restarts.

---

## 7. Protobuf Schema

### 7.1 scene.proto

```protobuf
syntax = "proto3";
package tze_hud.scene.v1;

import "google/protobuf/timestamp.proto";

// ─── IDs ────────────────────────────────────────────────────────────────────

// UUIDv7: 16-byte little-endian binary representation
message SceneId {
  bytes bytes = 1;  // Must be exactly 16 bytes
}

// BLAKE3 content hash: 32 bytes
message ResourceId {
  bytes bytes = 1;  // Must be exactly 32 bytes
}

// ─── Geometry ───────────────────────────────────────────────────────────────

message Rect {
  float x      = 1;
  float y      = 2;
  float width  = 3;
  float height = 4;
}

message Point2D {
  float x = 1;
  float y = 2;
}

message Rgba {
  float r = 1;
  float g = 2;
  float b = 3;
  float a = 4;
}

// ─── Enums ──────────────────────────────────────────────────────────────────

enum InputMode {
  INPUT_MODE_UNSPECIFIED = 0;
  INPUT_MODE_PASSTHROUGH = 1;
  INPUT_MODE_CAPTURE     = 2;
  INPUT_MODE_LOCAL_ONLY  = 3;
}

enum FontFamily {
  FONT_FAMILY_UNSPECIFIED    = 0;
  FONT_FAMILY_SYSTEM_SANS    = 1;
  FONT_FAMILY_SYSTEM_MONO    = 2;
  FONT_FAMILY_SYSTEM_SERIF   = 3;
}

enum TextAlign {
  TEXT_ALIGN_UNSPECIFIED = 0;
  TEXT_ALIGN_START       = 1;
  TEXT_ALIGN_CENTER      = 2;
  TEXT_ALIGN_END         = 3;
}

enum TextOverflow {
  TEXT_OVERFLOW_UNSPECIFIED = 0;
  TEXT_OVERFLOW_CLIP        = 1;
  TEXT_OVERFLOW_ELLIPSIS    = 2;
  TEXT_OVERFLOW_SCROLL      = 3;
}

enum ImageFit {
  IMAGE_FIT_UNSPECIFIED = 0;
  IMAGE_FIT_CONTAIN     = 1;
  IMAGE_FIT_COVER       = 2;
  IMAGE_FIT_FILL        = 3;
  IMAGE_FIT_NONE        = 4;
}

enum ContentionPolicy {
  CONTENTION_POLICY_UNSPECIFIED  = 0;
  CONTENTION_POLICY_LATEST_WINS  = 1;
  CONTENTION_POLICY_STACK        = 2;
  CONTENTION_POLICY_MERGE_BY_KEY = 3;
  CONTENTION_POLICY_REPLACE      = 4;
}

enum ValidationErrorCode {
  VALIDATION_ERROR_UNSPECIFIED           = 0;
  VALIDATION_ERROR_LEASE_EXPIRED         = 1;
  VALIDATION_ERROR_LEASE_NOT_FOUND       = 2;
  VALIDATION_ERROR_CAPABILITY_MISSING    = 3;
  VALIDATION_ERROR_BUDGET_EXCEEDED       = 4;
  VALIDATION_ERROR_BOUNDS_OUT_OF_RANGE   = 5;
  VALIDATION_ERROR_ZORDER_CONFLICT       = 6;
  VALIDATION_ERROR_NODE_COUNT_EXCEEDED   = 7;
  VALIDATION_ERROR_INVALID_FIELD_VALUE   = 8;
  VALIDATION_ERROR_DUPLICATE_ID          = 9;
  VALIDATION_ERROR_CYCLE_DETECTED        = 10;
  VALIDATION_ERROR_RESOURCE_NOT_FOUND    = 11;
  VALIDATION_ERROR_ZONE_TOKEN_INVALID    = 12;
  VALIDATION_ERROR_ZONE_TYPE_MISMATCH    = 13;
  VALIDATION_ERROR_TAB_NOT_FOUND         = 14;
  VALIDATION_ERROR_TILE_NOT_FOUND        = 15;
  VALIDATION_ERROR_NODE_NOT_FOUND        = 16;
}

// ─── Scene Objects ──────────────────────────────────────────────────────────

message Tab {
  SceneId  id            = 1;
  string   name          = 2;
  uint32   display_order = 3;
  uint64   created_at_us = 4;   // UTC μs since Unix epoch (RFC 0003 §3.1); 0 = not set
}

// Per-tile resource budget embedded in Tile messages.
// NOTE: This is the scene-graph-level per-tile budget. It is distinct from the
// lease-level ResourceBudget defined in RFC 0008 §10 (tze.lease.v1.ResourceBudget),
// which includes per-session aggregate limits (max_tiles, texture_bytes_total,
// max_active_leases, max_concurrent_streams). Do not conflate the two; they are
// in separate proto packages (tze_hud.scene.v1 vs tze.lease.v1).
message ResourceBudget {
  uint64 texture_bytes  = 1;   // Max texture bytes for this tile's nodes
  float  update_rate_hz = 2;   // Max mutations/second for this tile
  uint32 max_nodes      = 3;   // Max nodes in tile tree [1, 64]
}

enum LatencyClass {
  LATENCY_CLASS_UNSPECIFIED        = 0;
  LATENCY_CLASS_TRANSACTIONAL      = 1;
  LATENCY_CLASS_STATE_STREAM       = 2;
  LATENCY_CLASS_EPHEMERAL_REALTIME = 3;
  LATENCY_CLASS_CLOCKED_MEDIA      = 4;
}

enum UpdatePolicy {
  UPDATE_POLICY_UNSPECIFIED = 0;
  UPDATE_POLICY_ORDERED     = 1;
  UPDATE_POLICY_COALESCE    = 2;
  UPDATE_POLICY_LATEST_WINS = 3;
}

message Tile {
  SceneId        id              = 1;
  SceneId        tab_id          = 2;
  string         namespace       = 3;
  SceneId        lease_id        = 4;
  Rect           bounds          = 5;
  uint32         z_order         = 6;
  float          opacity         = 7;
  InputMode      input_mode      = 8;
  SceneId        sync_group      = 9;   // Zero value = not in a sync group
  uint64         present_at_us   = 10;  // UTC μs; 0 = immediate (RFC 0003 §3.1)
  uint64         expires_at_us   = 11;  // UTC μs; 0 = no expiry (RFC 0003 §3.1)
  ResourceBudget resource_budget = 12;
  SceneId        root_node       = 13;  // Zero value = empty tile
  LatencyClass   latency_class   = 14;  // Default: STATE_STREAM if unspecified
  UpdatePolicy   update_policy   = 15;  // Default: COALESCE if unspecified
}

// ─── Nodes ──────────────────────────────────────────────────────────────────

message SolidColorNode {
  Rgba color  = 1;
  Rect bounds = 2;
}

message TextMarkdownNode {
  string       content       = 1;
  Rect         bounds        = 2;
  float        font_size_px  = 3;
  FontFamily   font_family   = 4;
  Rgba         color         = 5;
  Rgba         background    = 6;  // Zero alpha = transparent
  TextAlign    alignment     = 7;
  TextOverflow overflow      = 8;
  uint64       present_at_us = 9;  // UTC μs; 0 = use tile-level present_at_us (RFC 0003 §3.1)
}

message StaticImageNode {
  ResourceId resource_id  = 1;
  Rect       bounds       = 2;
  ImageFit   fit          = 3;
  uint64     present_at_us = 4; // UTC μs; 0 = use tile-level present_at_us (RFC 0003 §3.1)
}

// HitRegionNode: base fields defined here; input-model fields defined in RFC 0004.
// Implementations use a single unified message with all fields populated.
message HitRegionNode {
  Rect   bounds          = 1;
  string interaction_id  = 2;  // Forwarded in input events for agent correlation
  bool   accepts_focus   = 3;
  bool   accepts_pointer = 4;
  // Fields 5–11: see RFC 0004 §7.1 for the behavioral contract of these input-model fields
  bool                auto_capture    = 5;
  bool                release_on_up   = 6;
  CursorStyle         cursor_style    = 7;
  string              tooltip         = 8;
  EventMaskConfig     event_mask      = 9;
  AccessibilityConfig accessibility   = 10;
  LocalStyleConfig    local_style     = 11;
}

message Node {
  SceneId        id       = 1;
  repeated SceneId children = 2;
  oneof data {
    SolidColorNode   solid_color   = 10;
    TextMarkdownNode text_markdown = 11;
    StaticImageNode  static_image  = 12;
    HitRegionNode    hit_region    = 13;
  }
}

// ─── Mutations ──────────────────────────────────────────────────────────────

message CreateTabMutation    { SceneId id = 1; string name = 2; uint32 display_order = 3; }
message DeleteTabMutation    { SceneId tab_id = 1; }
message RenameTabMutation    { SceneId tab_id = 1; string name = 2; }
message ReorderTabMutation   { SceneId tab_id = 1; uint32 display_order = 2; }
message SwitchActiveTabMutation { SceneId tab_id = 1; }

message CreateTileMutation   { Tile tile = 1; }
message UpdateTileBoundsMutation  { SceneId tile_id = 1; Rect bounds = 2; }
message UpdateTileZOrderMutation  { SceneId tile_id = 1; uint32 z_order = 2; }
message UpdateTileOpacityMutation { SceneId tile_id = 1; float opacity = 2; }
message UpdateTileInputModeMutation { SceneId tile_id = 1; InputMode input_mode = 2; }
message UpdateTileSyncGroupMutation { SceneId tile_id = 1; SceneId sync_group = 2; }
message UpdateTileExpiryMutation  { SceneId tile_id = 1; uint64 expires_at_us = 2; }  // UTC μs; 0 = clear expiry
message DeleteTileMutation   { SceneId tile_id = 1; }

message SetTileRootMutation  { SceneId tile_id = 1; Node node = 2; }
// InsertNodeMutation: parent_id zero bytes = "no parent" (node becomes the tile root, equivalent
// to SetTileRoot). Use SetTileRootMutation if the tile has no existing root; use InsertNode with
// zero parent_id to atomically replace the root in a batch. Non-zero parent_id = add as child.
message InsertNodeMutation   { SceneId tile_id = 1; SceneId parent_id = 2; Node node = 3; }
message ReplaceNodeMutation  { SceneId node_id = 1; Node node = 2; }
message UpdateNodeBoundsMutation { SceneId node_id = 1; Rect bounds = 2; }
message RemoveNodeMutation   { SceneId node_id = 1; }

message ZoneContent {
  oneof payload {
    string          stream_text          = 1;
    NotificationPayload notification     = 2;
    StatusBarPayload    status_bar       = 3;
    ResourceId          static_image     = 4;
    Rgba                solid_color      = 5;
  }
}

enum NotificationUrgency {
  NOTIFICATION_URGENCY_UNSPECIFIED = 0;
  NOTIFICATION_URGENCY_LOW         = 1;
  NOTIFICATION_URGENCY_NORMAL      = 2;
  NOTIFICATION_URGENCY_URGENT      = 3;
  NOTIFICATION_URGENCY_CRITICAL    = 4;
}

message NotificationPayload {
  string               text    = 1;
  string               icon    = 2;   // Resource name or empty
  NotificationUrgency  urgency = 3;
}

message StatusBarPayload {
  map<string, string> entries = 1;  // key → display string
}

message ZonePublishToken {
  bytes token = 1;  // Opaque capability token issued at session auth
}

// Content privacy classification for zone publications.
// Used by the privacy gate (RFC 0009 §2.3) to determine redaction behavior.
// Corresponds to the ContentClassification Rust enum in §2.5.
// UNSPECIFIED = inherit zone's default_privacy from configuration.
enum ContentClassification {
  CONTENT_CLASSIFICATION_UNSPECIFIED = 0;   // Inherit zone default
  CONTENT_CLASSIFICATION_PUBLIC      = 1;   // All viewers
  CONTENT_CLASSIFICATION_HOUSEHOLD   = 2;   // Owner + HouseholdMember
  CONTENT_CLASSIFICATION_PRIVATE     = 3;   // Owner only
  CONTENT_CLASSIFICATION_SENSITIVE   = 4;   // Owner only; higher-sensitivity signal
}

// zone_name is a zone type name (e.g. "subtitle"). The runtime resolves it to the
// ZoneInstance for the publishing agent's active tab (see §2.5 and §3.1 comments).
// Rejected with ZoneNotFound if no instance for this type exists in the active tab.
message PublishToZoneMutation {
  string                  zone_name              = 1;   // Zone type name; resolved to ZoneInstance by runtime
  ZoneContent             content                = 2;
  ZonePublishToken        publish_token          = 3;
  uint64                  expires_at_us          = 4;   // UTC μs; 0 = use zone auto_clear_us; >0 = per-publish TTL (RFC 0003 §3.1)
  string                  publish_key            = 5;   // Non-empty only for MergeByKey zones
  ContentClassification   content_classification = 6;   // UNSPECIFIED = inherit zone type default (see §2.5)
}

message ClearZoneMutation {
  string           zone_name     = 1;   // Zone type name; resolved to ZoneInstance by runtime
  ZonePublishToken publish_token = 2;
}

// Sync group lifecycle mutations (defined by RFC 0003 §7.2; consumed here in scene.proto).
// Sync group creation/deletion is an explicit scene operation: a group must be created before
// tiles can be assigned to it via UpdateTileSyncGroupMutation. Groups are deleted when the last
// member tile is removed or via an explicit DeleteSyncGroupMutation.
//
// SyncGroupConfig is defined in timing.proto (RFC 0003 §7.1) and imported here.
// The group ID is embedded in config.id (SyncGroupConfig field 1). There is no
// separate top-level `id` field — use config.id. This is the canonical cross-RFC form;
// RFC 0003 §7.1 CreateSyncGroupMutation matches this definition exactly.
message CreateSyncGroupMutation {
  SyncGroupConfig config = 1;  // Full group configuration (RFC 0003 §7.1); config.id is the group SceneId
}

message DeleteSyncGroupMutation {
  SceneId sync_group_id = 1;  // The sync group to delete; member tiles remain, unlinked
}

message SceneMutation {
  oneof mutation {
    CreateTabMutation         create_tab          = 1;
    DeleteTabMutation         delete_tab          = 2;
    RenameTabMutation         rename_tab          = 3;
    ReorderTabMutation        reorder_tab         = 4;
    SwitchActiveTabMutation   switch_active_tab   = 5;
    CreateTileMutation        create_tile         = 6;
    UpdateTileBoundsMutation  update_tile_bounds  = 7;
    UpdateTileZOrderMutation  update_tile_zorder  = 8;
    UpdateTileOpacityMutation update_tile_opacity = 9;
    UpdateTileInputModeMutation update_tile_input_mode = 10;
    UpdateTileSyncGroupMutation update_tile_sync_group = 11;
    UpdateTileExpiryMutation  update_tile_expiry  = 12;
    DeleteTileMutation        delete_tile         = 13;
    SetTileRootMutation       set_tile_root       = 14;
    InsertNodeMutation        insert_node         = 15;
    ReplaceNodeMutation       replace_node        = 16;
    UpdateNodeBoundsMutation  update_node_bounds  = 17;
    RemoveNodeMutation        remove_node         = 18;
    PublishToZoneMutation     publish_to_zone     = 19;
    ClearZoneMutation         clear_zone          = 20;
    CreateSyncGroupMutation   create_sync_group   = 21;  // RFC 0003 §7.2
    DeleteSyncGroupMutation   delete_sync_group   = 22;  // RFC 0003 §7.2
  }
}

message MutationBatch {
  SceneId            batch_id         = 1;
  // Field 2 (agent_namespace) is reserved; the server derives the agent namespace
  // from the authenticated session context and must never trust a client-supplied
  // identity field.  See security.md §'Authentication' and §'Capability scopes'.
  reserved 2;
  reserved "agent_namespace";
  repeated SceneMutation mutations    = 3;
  uint64             present_at_us    = 4;  // UTC μs; 0 = immediate (RFC 0003 §3.1)
  uint64             sequence_hint    = 5;  // 0 = no hint
}

// ─── Responses ──────────────────────────────────────────────────────────────

message BatchCommitted {
  SceneId batch_id       = 1;
  uint64  sequence       = 2;
  uint64  committed_at_us = 3;   // UTC μs; frame commit wall clock (RFC 0003 §3.1)
}

message MutationValidationError {
  uint32               mutation_index  = 1;
  string               mutation_type   = 2;
  ValidationErrorCode  code            = 3;
  string               message         = 4;
  string               context_json    = 5;   // JSON object: {field, value, constraint}
  string               correction_hint = 6;   // JSON object or empty
}

message RateLimitExceededError {
  float  limit_hz   = 1;   // Agent's allowed rate budget (mutations/second)
  float  current_hz = 2;   // Observed rate that exceeded the limit
}

message BatchSizeExceededError {
  uint32 max = 1;   // Maximum allowed mutations per batch
  uint32 got = 2;   // Actual number of mutations in the rejected batch
}

message BatchRejected {
  SceneId                    batch_id = 1;
  oneof error {
    string                   parse_error       = 2;
    MutationValidationError  validation        = 3;
    RateLimitExceededError   rate_limited      = 4;   // Structured rate-limit error (matches Rust BatchError::RateLimitExceeded)
    BatchSizeExceededError   batch_too_large   = 5;   // Structured size error (matches Rust BatchError::BatchSizeExceeded)
  }
}

// ─── Snapshots and Diffs ────────────────────────────────────────────────────

// SceneSnapshot: flat lists for deterministic serialization (required for checksum stability).
// Tree reconstruction: tiles reference their tab via tile.tab_id. Node tree is reconstructed by
// indexing nodes by id and walking Node.children recursively from each tile's root_node.
// See Rust SceneSnapshot doc comment in §4.1 for the canonical reconstruction algorithm.
message SceneSnapshot {
  uint64             sequence         = 1;
  uint64             timestamp_us     = 2;   // UTC μs; snapshot wall clock (RFC 0003 §3.1)
  repeated Tab       tabs             = 3;
  repeated Tile      tiles            = 4;   // All tiles; use tile.tab_id to group by tab
  repeated Node      nodes            = 5;   // All nodes; use Node.children for tree structure
  ZoneRegistrySnapshot zone_registry  = 6;
  SceneId            active_tab       = 7;   // Zero = no active tab
  bytes              checksum         = 8;   // BLAKE3, 32 bytes
  WidgetRegistrySnapshotProto widget_registry = 9;  // Widget types, instances, and active publications
}

// Geometry policy variants for zone placement.
// Corresponds to the GeometryPolicy Rust enum.
message RelativeGeometryPolicy {
  float x_pct      = 1;
  float y_pct      = 2;
  float width_pct  = 3;
  float height_pct = 4;
}

enum DisplayEdge {
  DISPLAY_EDGE_UNSPECIFIED = 0;
  DISPLAY_EDGE_TOP         = 1;
  DISPLAY_EDGE_BOTTOM      = 2;
  DISPLAY_EDGE_LEFT        = 3;
  DISPLAY_EDGE_RIGHT       = 4;
}

message EdgeAnchoredGeometryPolicy {
  DisplayEdge edge       = 1;
  float       height_pct = 2;   // Used for top/bottom edges
  float       width_pct  = 3;   // Used for left/right edges
  float       margin_px  = 4;
}

message GeometryPolicyProto {
  oneof policy {
    RelativeGeometryPolicy    relative     = 1;
    EdgeAnchoredGeometryPolicy edge_anchored = 2;
  }
}

// Rendering policy for zone content presentation.
// All fields are optional; absent = compositor default.
message RenderingPolicyProto {
  float     font_size_px = 1;   // 0.0 = not set (use compositor default)
  Rgba      backdrop     = 2;   // Zero alpha = not set
  TextAlign text_align   = 3;   // UNSPECIFIED = not set
  float     margin_px    = 4;   // 0.0 = not set
}

enum ZoneMediaType {
  ZONE_MEDIA_TYPE_UNSPECIFIED        = 0;
  ZONE_MEDIA_TYPE_STREAM_TEXT        = 1;
  ZONE_MEDIA_TYPE_SHORT_TEXT_ICON    = 2;
  ZONE_MEDIA_TYPE_KEY_VALUE_PAIRS    = 3;
  ZONE_MEDIA_TYPE_VIDEO_SURFACE_REF  = 4;
  ZONE_MEDIA_TYPE_STATIC_IMAGE       = 5;
  ZONE_MEDIA_TYPE_SOLID_COLOR        = 6;
}

enum ZoneLayerAttachment {
  ZONE_LAYER_UNSPECIFIED = 0;
  ZONE_LAYER_BACKGROUND  = 1;   // Behind all agent tiles
  ZONE_LAYER_CONTENT     = 2;   // Among agent tiles; z-order pinned by runtime
  ZONE_LAYER_CHROME      = 3;   // Above all agent tiles; runtime-rendered
}

// Zone type — the schema layer of the four-level zone ontology (presence.md §"Zone anatomy").
// Defines accepted media, contention policy, rendering defaults.
// Corresponds to the ZoneType Rust struct in §2.5.
message ZoneTypeProto {
  SceneId                    id                   = 1;
  string                     name                 = 2;   // Stable type name, e.g. "subtitle"
  string                     description          = 3;
  repeated ZoneMediaType     accepted_media_types = 4;
  RenderingPolicyProto       rendering_policy     = 5;
  ContentionPolicy           contention_policy    = 6;
  uint64                     auto_clear_us        = 7;   // μs duration; 0 = no auto-clear
  ContentClassification      default_privacy      = 8;   // UNSPECIFIED = no default (publication must declare)
}

// Zone instance — a zone type bound into a specific tab (presence.md §"Zone anatomy").
// Multiple instances of the same zone type may exist across different tabs.
// Corresponds to the ZoneInstance Rust struct in §2.5.
message ZoneInstanceProto {
  SceneId                    id               = 1;
  string                     zone_type_name   = 2;   // References ZoneTypeProto.name
  SceneId                    tab_id           = 3;   // The tab this instance belongs to
  ZoneLayerAttachment        layer_attachment = 4;   // Which compositor layer this instance attaches to
  GeometryPolicyProto        geometry_policy  = 5;
}

// Zone occupancy — the runtime's resolved current state of a zone instance.
// Read-only from the agent's perspective (presence.md §"Zone anatomy").
// Corresponds to the ZoneOccupancy Rust struct in §2.5.
// In v1, effective_geometry is not included in snapshots (deferred to post-v1 query API).
message ZoneOccupancyProto {
  SceneId                         instance_id          = 1;   // References ZoneInstanceProto.id
  repeated ZonePublishRecordProto active_publications  = 2;   // Currently active publications
  // effective_geometry: deferred to post-v1 occupancy query API
}

// Resolved pixel-level geometry for a zone instance (post-v1 occupancy query API).
message ResolvedGeometryProto {
  float x_px      = 1;
  float y_px      = 2;
  float width_px  = 3;
  float height_px = 4;
}

message ZonePublishRecordProto {
  SceneId                 instance_id            = 1;   // References ZoneInstanceProto.id
  string                  zone_type_name         = 2;   // Zone type name (readability; resolved via instance)
  string                  publisher_namespace    = 3;
  ZoneContent             content                = 4;
  uint64                  published_at_us        = 5;   // UTC μs since Unix epoch (RFC 0003 §3.1)
  uint64                  expires_at_us          = 6;   // UTC μs; 0 = governed by zone auto_clear_us (RFC 0003 §3.1)
  string                  publish_key            = 7;   // Non-empty only for MergeByKey zones
  ContentClassification   content_classification = 8;   // UNSPECIFIED = publication inherited zone default
  // Field 8 propagates per-publication privacy classification from PublishToZoneMutation
  // into the snapshot record so that reconnecting agents and the privacy gate can
  // reconstruct effective classification state after reconnect.
  // Reference: presence.md §"Zone anatomy"; RFC 0009 §2.3.
}

// Zone registry snapshot — all four levels of the zone ontology.
// Zone types and instances are stable (config-loaded in v1).
// Active publications reflect live runtime state.
message ZoneRegistrySnapshot {
  repeated ZoneTypeProto          zone_types       = 1;
  repeated ZoneInstanceProto      zone_instances   = 2;
  repeated ZonePublishRecordProto active_publishes = 3;
}

// ─── Widget registry proto types ────────────────────────────────────────────
// Corresponds to §2.6 Widget Registry Rust types.
// Defined in types.proto (tze_hud.protocol.v1) alongside zone types.

enum WidgetParamTypeProto {
  WIDGET_PARAM_TYPE_UNSPECIFIED = 0;
  WIDGET_PARAM_TYPE_F32         = 1;
  WIDGET_PARAM_TYPE_STRING      = 2;
  WIDGET_PARAM_TYPE_COLOR       = 3;
  WIDGET_PARAM_TYPE_ENUM        = 4;
}

message WidgetParamConstraintsProto {
  float           min             = 1;   // f32 range lower bound; 0.0 = not set
  float           max             = 2;   // f32 range upper bound; 0.0 = not set
  uint32          max_length      = 3;   // String max byte length; 0 = use default (1024)
  repeated string allowed_values  = 4;   // Enum allowed values
}

message WidgetParameterValueProto {
  oneof value {
    float  f32_value    = 1;
    string string_value = 2;
    Rgba   color_value  = 3;
    string enum_value   = 4;
  }
}

message WidgetParameterDeclarationProto {
  string                       name          = 1;   // Snake-case parameter identifier
  WidgetParamTypeProto         param_type    = 2;
  WidgetParameterValueProto    default_value = 3;
  WidgetParamConstraintsProto  constraints   = 4;   // Absent = no constraints beyond type
}

message WidgetBindingMappingProto {
  oneof mapping {
    LinearMappingProto   linear   = 1;
    bool                 direct   = 2;   // true = direct substitution
    DiscreteMappingProto discrete = 3;
  }
}

message LinearMappingProto {
  float attr_min = 1;
  float attr_max = 2;
}

message DiscreteMappingProto {
  map<string, string> values = 1;   // enum value → SVG attribute value
}

message WidgetBindingProto {
  string                  param            = 1;   // References WidgetParameterDeclarationProto.name
  string                  target_element   = 2;   // SVG element id attribute
  string                  target_attribute = 3;   // SVG attribute name, or "text-content"
  WidgetBindingMappingProto mapping         = 4;
}

message WidgetSvgLayerProto {
  string                   svg_resource_id = 1;   // BLAKE3 hex ResourceId of the SVG asset
  repeated WidgetBindingProto bindings     = 2;
}

message WidgetDefinitionProto {
  string                            id                        = 1;   // Kebab-case type identifier
  string                            name                      = 2;
  string                            description               = 3;
  repeated WidgetParameterDeclarationProto parameter_schema   = 4;
  repeated WidgetSvgLayerProto      layers                    = 5;
  GeometryPolicyProto               default_geometry_policy   = 6;
  RenderingPolicyProto              default_rendering_policy  = 7;
  ContentionPolicyProto             default_contention_policy = 8;
  bool                              ephemeral                 = 9;
}

message WidgetInstanceProto {
  string              instance_name      = 1;   // Addressing key (explicit instance_id or widget_type_name)
  string              widget_type_name   = 2;   // References WidgetDefinitionProto.id
  SceneId             tab_id             = 3;
  GeometryPolicyProto geometry_override  = 4;   // Absent = use widget type default
  ContentionPolicyProto contention_override = 5; // Absent = use widget type default
  map<string, WidgetParameterValueProto> current_params = 6;
}

message WidgetPublishRecordProto {
  string                                 widget_name          = 1;
  string                                 publisher_namespace  = 2;
  map<string, WidgetParameterValueProto> params               = 3;
  uint64                                 published_at_wall_us = 4;
  string                                 merge_key            = 5;   // Empty = not a MergeByKey publish
  uint64                                 expires_at_wall_us   = 6;   // 0 = use widget default TTL
  uint32                                 transition_ms        = 7;
}

message WidgetOccupancyProto {
  string                                 widget_name        = 1;
  SceneId                                tab_id             = 2;
  repeated WidgetPublishRecordProto      active_publications = 3;
  uint32                                 occupant_count     = 4;
  map<string, WidgetParameterValueProto> effective_params   = 5;
}

// Widget registry snapshot — all four levels of the widget ontology.
// Definitions (loaded from asset bundles) and instances (from config) are stable in v1.
// Active publications reflect live runtime state.
// All maps use BTreeMap ordering in the Rust implementation for checksum determinism.
message WidgetRegistrySnapshotProto {
  repeated WidgetDefinitionProto      definitions      = 1;
  repeated WidgetInstanceProto        instances        = 2;
  repeated WidgetPublishRecordProto   active_publishes = 3;
}

// Widget mutation types — used within MutationBatch (§3.1).
// These parallel PublishToZoneMutation and ClearZoneMutation.
message PublishToWidgetMutation {
  string                                 widget_name   = 1;   // Widget instance name
  string                                 instance_id   = 2;   // Disambiguation when multiple instances of same type exist on tab
  map<string, WidgetParameterValueProto> params        = 3;
  uint32                                 transition_ms = 4;   // 0 = snap immediately
  uint64                                 ttl_us        = 5;   // 0 = use widget instance default
  string                                 merge_key     = 6;   // For MergeByKey contention; empty otherwise
}

message ClearWidgetMutation {
  string widget_name = 1;   // Widget instance name
  string instance_id = 2;   // Optional disambiguation
}

// Widget registry query types — defined here for use in scene discovery.
// Agents query the widget registry via SceneSnapshot.widget_registry at session establishment;
// there is no separate widget registry query RPC in v1. MCP agents query via list_widgets.
message WidgetRegistryRequest {
  // No parameters in v1 — returns all widget definitions and instances.
}

message WidgetRegistryResponse {
  WidgetRegistrySnapshotProto registry = 1;
}

// Typed partial-update messages for incremental diff ops.
// All fields are optional; absent field = not changed in this diff.
// (Used by DiffOp — part of the deferred incremental diff extension.)
message TabPatch {
  SceneId tab_id        = 1;
  string  name          = 2;   // Empty = not changed
  uint32  display_order = 3;   // 0 = not changed (display_order is never 0 in practice)
}

message TilePatch {
  SceneId      tile_id       = 1;
  Rect         bounds        = 2;   // Absent = not changed
  uint32       z_order       = 3;   // 0 = not changed (use has_z_order wrapper field in impl)
  float        opacity       = 4;   // 0.0 = not changed (use has_opacity wrapper field in impl)
  InputMode    input_mode    = 5;   // UNSPECIFIED = not changed
  SceneId      sync_group    = 6;   // Zero bytes = not changed
  uint64       expires_at_us = 7;   // 0 = not changed (UTC μs; RFC 0003 §3.1)
  LatencyClass latency_class = 8;   // UNSPECIFIED = not changed
  UpdatePolicy update_policy = 9;   // UNSPECIFIED = not changed
}

message NodePatch {
  SceneId node_id        = 1;
  Rect    bounds         = 2;   // Absent = not changed; applies to node's primary bounds field
  // Note: full node type replacement uses NodeAdded+NodeRemoved, not NodePatch.
  // NodePatch covers bounds-only updates on existing nodes without changing node type.
}

message DiffOp {
  oneof op {
    Tab      tab_added            = 1;
    SceneId  tab_removed          = 2;
    TabPatch tab_modified         = 3;
    Tile     tile_added           = 4;
    SceneId  tile_removed         = 5;
    TilePatch tile_modified       = 6;
    NodeAddedDiff node_added      = 7;
    SceneId  node_removed         = 8;
    NodePatch node_modified       = 9;
    ZonePublishChanged zone_publish_changed = 10;
    uint64   sequence_marker      = 11;
  }
}

message NodeAddedDiff {
  SceneId tile_id   = 1;
  SceneId parent_id = 2;  // Zero = root node
  Node    node      = 3;
}

message ZonePublishChanged {
  SceneId                 instance_id = 1;   // References ZoneInstanceProto.id
  ZonePublishRecordProto  record      = 2;   // Empty = zone instance cleared
}

message SceneDiff {
  uint64            from_sequence = 1;
  uint64            to_sequence   = 2;
  repeated DiffOp   ops           = 3;
}

// ─── Hit Testing ────────────────────────────────────────────────────────────

message HitTestQuery {
  SceneId tab_id = 1;
  Point2D point  = 2;
}

message HitTestResult {
  oneof kind {
    NodeHitResult      node_hit   = 1;
    TileHitResult      tile_hit   = 2;
    PassthroughResult  passthrough = 3;
    ChromeHitResult    chrome_hit = 4;
  }
}

message NodeHitResult {
  SceneId tile_id          = 1;
  SceneId node_id          = 2;
  Point2D local_coords     = 3;
  Point2D tile_local_coords = 4;
}

message TileHitResult {
  SceneId tile_id      = 1;
  Point2D local_coords = 2;
}

message PassthroughResult {}

message ChromeHitResult {
  string element = 1;  // ChromeElement name
}
```

### 7.2 Schema Constraints and Wire Format Notes

1. All `SceneId` fields use the zero value (`[0u8; 16]`) to represent "absent/null" (e.g., `root_node = 0` means no root node; `sync_group = 0` means not in a sync group).
2. All `uint64` timestamp fields use 0 to represent "not set" (zero is never a valid real timestamp in this system; the runtime started after 2025). All timestamp fields use **microsecond (μs) resolution** per RFC 0003 §3.1.
3. `bytes checksum` in `SceneSnapshot` is exactly 32 bytes or empty (not yet computed).
4. Mutation field numbers are stable; fields are never renumbered. New mutations are added with new field numbers.
5. Unknown fields in messages are preserved (proto3 semantics). Agents must not fail on unknown fields.
6. The `context_json` and `correction_hint` in `MutationValidationError` are UTF-8 JSON strings to allow structured context without defining a separate message per error type.

---

## 8. Scene Graph Capacity Limits

| Dimension | V1 Limit | Notes |
|-----------|----------|-------|
| Tabs per scene | 256 | Enforced at `CreateTab` validation |
| Tiles per tab | 1024 | Enforced at `CreateTile` validation |
| Nodes per tile | 64 | Set per-tile in `ResourceBudget`; hard cap |
| Max batch size | 1000 mutations | Rejected with `BatchSizeExceeded` |
| Max markdown content | 65535 bytes | UTF-8 encoded |
| Tab name length | 128 bytes | UTF-8 encoded |
| Zone name length | 64 bytes | ASCII only (zone names are identifiers) |
| Agent namespace length | 128 bytes | UTF-8 encoded |
| Interaction ID length | 256 bytes | UTF-8 encoded |

**Memory budget (targets):** Two separate budgets govern structural overhead (content payloads are accounted via `ResourceBudget.texture_bytes` and excluded from both):

| Budget | Target | What it covers |
|--------|--------|----------------|
| Per-tile struct | < 200 bytes | `Tile` fields: ID, bounds, opacity, z_order, lease ID, flags, metadata — no nodes |
| Per-node struct | < 150 bytes | Node fields: ID, type tag, bounds, enum discriminants, parent/child IDs — no content payloads |

Approximation at max capacity:

```
Tile struct:             ~200 bytes
Per node avg:            ~150 bytes (structural fields; TextMarkdownNode content excluded)
64 nodes/tile (max):     ~9.6 KB structural node overhead
Total at max_nodes=64:   ~9.8 KB per tile (tile struct + full node tree, content excluded)
```

These are independent budgets: an empty tile costs ~200 bytes; each additional node adds ~150 bytes. The two-budget model lets implementors bound memory linearly against actual node count rather than reserving worst-case allocation upfront.

---

## 9. Diagrams

### 9.1 Scene Graph Hierarchy (Tab → Tile → Node with Zone Overlay)

```
╔══════════════════════════════════════════════════════════════════╗
║  SCENE                                                           ║
║                                                                  ║
║  ┌─────────────────────────────┐  ┌────────────────────────────┐ ║
║  │ TAB: "Morning"              │  │ TAB: "Work"                │ ║
║  │ display_order=0 [ACTIVE]    │  │ display_order=1            │ ║
║  │                             │  └────────────────────────────┘ ║
║  │  ┌─────────────────────┐    │                                  ║
║  │  │ TILE z=5 (weather)  │    │                                  ║
║  │  │ bounds: 0,0,400,200 │    │                                  ║
║  │  │ ns: "weather-agent" │    │                                  ║
║  │  │                     │    │                                  ║
║  │  │  [NODE: SolidColor] │    │                                  ║
║  │  │  [NODE: TextMd    ] │    │                                  ║
║  │  │    └─[NODE: HitRgn]│    │                                  ║
║  │  └─────────────────────┘    │                                  ║
║  │                             │                                  ║
║  │  ┌─────────────────────┐    │                                  ║
║  │  │ TILE z=3 (calendar) │    │                                  ║
║  │  │ bounds: 420,0,380,  │    │                                  ║
║  │  │         300         │    │                                  ║
║  │  │ ns: "cal-agent"     │    │                                  ║
║  │  │                     │    │                                  ║
║  │  │  [NODE: StaticImg ] │    │                                  ║
║  │  │  [NODE: TextMd    ] │    │                                  ║
║  │  └─────────────────────┘    │                                  ║
║  │                             │                                  ║
║  │  ┌────────────────────────────────┐                            ║
║  │  │ ZONE TILE (runtime-owned)      │  ← auto-managed by runtime ║
║  │  │ instance: "Morning"/"subtitle" │  ← ZoneInstance (tab+type) ║
║  │  │ z=0x8000_0000 (reserved band)  │  ← within content layer   ║
║  │  │ bounds: 0,540,800,60           │                            ║
║  │  │  [NODE: TextMd (zone content)] │                            ║
║  │  └────────────────────────────────┘                            ║
║  └─────────────────────────────┘                                  ║
╚══════════════════════════════════════════════════════════════════╝

CHROME LAYER (above all content — always on top)
┌──────────────────────────────────────────────────────────────────┐
│  [TAB BAR] [SYSTEM INDICATORS] [OVERRIDE CONTROLS]              │
└──────────────────────────────────────────────────────────────────┘
```

**Layer note:** The subtitle zone tile shown above is still within the **content layer** (the second of three layers). The three-layer model from architecture.md §"Layer stack" is preserved: background / content / chrome. Content-layer zone tiles occupy a reserved z-order band (`z_order >= ZONE_TILE_Z_MIN = 0x8000_0000`) at the top of the content layer's z-order space. Agent-owned tiles are validated to use z_order values below this threshold. No fourth layer exists — zone tiles are realized as ordinary tiles with runtime-enforced z-order governance.

### 9.2 Transaction Pipeline

```
Agent
  │
  │  MutationBatch{batch_id, mutations[...]}
  ▼
┌──────────────────────────────────────────────────────┐
│  STAGE: Parse + Deserialize                          │
│  ─ protobuf decode                                   │
│  ─ structural null checks                            │
│  ─ batch size check (≤ 1000)                         │
└─────────────────────────┬────────────────────────────┘
                          │ ParseError?
                          ├──────────────────────────► BatchRejected(ParseError)
                          ▼
┌──────────────────────────────────────────────────────┐
│  VALIDATE: All-or-nothing per-mutation checks        │
│  For mutation[i]:                                    │
│    1. Lease check                                    │
│    2. Budget check                                   │
│    3. Bounds check                                   │
│    4. Type check                                     │
│    5. Post-batch invariant simulation                │
└─────────────────────────┬────────────────────────────┘
                          │ Any failure?
                          ├──────────────────────────► BatchRejected(ValidationError{
                          │                               mutation_index: i,
                          │                               code, message,
                          │                               context, hint})
                          ▼ All pass
┌──────────────────────────────────────────────────────┐
│  COMMIT: Atomic                                      │
│  ─ acquire write lock                                │
│  ─ apply mutations[0..n] in order                    │
│  ─ increment sequence_number                         │
│  ─ release write lock                                │
│  (WAL append deferred to post-v1; see §4.2)          │
└─────────────────────────┬────────────────────────────┘
                          ▼
                BatchCommitted{batch_id, sequence}
                          │
                          ▼
              ┌───────────────────────┐
              │  COMPOSITOR picks up  │
              │  frame delta at next  │
              │  vsync boundary       │
              └───────────────────────┘
                          │
                          ▼
              ┌───────────────────────┐
              │     PRESENT           │
              │  Frame rendered and   │
              │  displayed            │
              └───────────────────────┘
```

### 9.3 Hit-Test Traversal Order

```
Input Point P = (x, y) in tab display space
                    │
                    ▼
        ┌──────────────────────┐
        │  Chrome layer check  │
        │  (always first)      │
        └──────────┬───────────┘
                   │ P in chrome element?
            yes ◄──┤
    ChromeHit       │ no
                    ▼
        ┌──────────────────────────────────────────┐
        │  Iterate tiles: z_order descending       │
        │  (z=1024 first → z=0 last)               │
        └──────────────────────┬───────────────────┘
                               │
              ┌────────────────▼───────────────────┐
              │  Tile T (current, highest-z first)  │
              │                                     │
              │  1. T.input_mode == Passthrough?    │
              │     yes → skip to next tile         │
              │                                     │
              │  2. P in T.bounds?                  │
              │     no  → skip to next tile         │
              │                                     │
              │  3. Traverse T.nodes reverse order  │
              │     (last child → root):            │
              │     For each HitRegionNode N:        │
              │       P in N.bounds?                │
              │         yes → NodeHit(T, N,         │
              │                 local_coords)       │
              │                  STOP               │
              │     No HitRegion matched →           │
              │       TileHit(T, local_coords) STOP │
              └────────────────────────────────────┘
                    │
                    │ No tile matched
                    ▼
                Passthrough
```

### 9.4 ID Namespace Isolation Model

```
╔══════════════════════════════════════════════════════════╗
║  RUNTIME IDENTITY BOUNDARY                              ║
║                                                          ║
║  ┌───────────────────────────────────────────────────┐  ║
║  │  Session Auth                                     │  ║
║  │  identity="weather-agent" → namespace="wtr"       │  ║
║  └─────────────────────────────┬─────────────────────┘  ║
║                                │ grants                  ║
║                    ┌───────────▼────────────┐           ║
║                    │  Capability Grants     │           ║
║                    │  - create_tiles         │           ║
║                    │  - modify_own_tiles     │           ║
║                    │  - publish_zone:subtitle│           ║
║                    └───────────┬────────────┘           ║
║                                │ scopes                  ║
║              ┌─────────────────▼──────────────────────┐ ║
║              │  Namespace: "wtr"                      │ ║
║              │                                        │ ║
║              │  LeaseId: L-a1b2 (TTL: 300s)           │ ║
║              │  └── TileId: T-c3d4                    │ ║
║              │       ├── NodeId: N-e5f6 (SolidColor)  │ ║
║              │       └── NodeId: N-g7h8 (TextMd)      │ ║
║              │                                        │ ║
║              │  LeaseId: L-i9j0 (TTL: 60s)            │ ║
║              │  └── TileId: T-k1l2                    │ ║
║              │       └── NodeId: N-m3n4 (HitRegion)   │ ║
║              └────────────────────────────────────────┘ ║
║                                                          ║
║              ┌─────────────────────────────────────────┐║
║              │  Namespace: "cal"   (different agent)   │║
║              │  ── cannot read/write "wtr" tiles ──    │║
║              └─────────────────────────────────────────┘║
║                                                          ║
║  ┌──────────────────────────────────────────────────┐   ║
║  │  Resource Store (shared, content-addressed)      │   ║
║  │  res-blake3hash1 ← referenced by N-g7h8          │   ║
║  │  res-blake3hash2 ← referenced by cal's node      │   ║
║  │  (readable by any namespace; write = upload only) │   ║
║  └──────────────────────────────────────────────────┘   ║
╚══════════════════════════════════════════════════════════╝
```

---

## 10. Quantitative Requirements Summary

| Metric | Requirement | Measurement Method |
|--------|-------------|-------------------|
| Snapshot serialization | < 1ms for 100-tile / 1000-node scene | Protobuf encode, single core, reference hw |
| Diff computation | < 500μs for typical frame delta (10–30 mutations) — **post-v1** | WAL walk + coalesce, single core |
| Hit-test | < 100μs for single point query on 50 tiles | Pure Rust, no GPU, Layer 0 benchmark |
| Transaction validation | < 200μs per batch of 10 mutations | Validation stage only, excludes commit |
| Tile struct overhead | < 200 bytes (excl. texture data, excl. nodes) | Rust `size_of::<Tile>()` + metadata alloc |
| Node struct overhead | < 150 bytes per node (excl. content payloads) | Rust `size_of::<Node>()` + ID alloc |
| Max tile memory (64 nodes) | ~9.8 KB structural (excl. content/textures) | Tile struct + 64 × node struct |
| Max scene | 256 tabs × 1024 tiles/tab × 64 nodes/tile | Hard limit enforced in validation |
| Sequence number | Monotonic u64; wraps after ~1.8×10^19 | Never wraps in practice |

Hardware reference: single core at 3GHz equivalent (normalized; see validation.md calibration).

---

## 11. Open Questions

1. **Zone geometry policy config format:** The wire format is now typed protobuf (`GeometryPolicyProto`, `RenderingPolicyProto`). The config file format (TOML/YAML/JSON used for zone registry at startup) is a separate concern: it should be human-editable and deserializable into the Rust `GeometryPolicy` enum. Recommendation: TOML for authoring, convert to proto for wire. Defer config file schema to the Config/Setup RFC.

2. **WAL retention policy (post-v1):** 1000 batches or 60s, whichever is smaller, is a starting point for the deferred incremental diff extension. In v1, the WAL is used only for sequence ordering within the commit pipeline; agents reconnect via full snapshot and the WAL does not need to be queried externally. Revisit when incremental diff is implemented.

3. **Snapshot checksum coverage:** ~~Recommendation~~ **Normative decision (Round 4):** `SceneSnapshot.checksum` covers the full serialization — tabs, tiles, nodes, zone registry (zone types + zone instances + active publishes) — in that canonical order, excluding only `timestamp_us` and `checksum` itself. The canonical serialization uses protobuf binary encoding with fields in tag-ascending order. This is normative: implementations that compute the checksum differently will produce divergent values and fail reconnect validation. The checksum is computed over the same proto binary that would be transmitted on the wire.

4. **`#[no_std]` compatibility:** The `SceneId` (UUIDv7) constructor requires a clock source not available in no_std. Options: (a) accept that scene graph construction requires std, (b) inject a clock trait, (c) make `new()` require a timestamp argument. Recommendation: (b) inject a clock trait for test/embedded flexibility.

5. **Tile bounds reference frame:** The spec says tile bounds are in "logical pixels relative to the tab's display area." The compositor must define what "logical pixel" means across display profiles (HiDPI, scaling). The Compositor RFC must define the coordinate space and DPI contract.

6. **Zone publish token wire format:** The `ZonePublishToken` is an opaque bytes field intentionally. The Session/Protocol RFC (RFC 0005) defines how tokens are issued during session auth and their expiry semantics. **Normative expectation for RFC 0001 (Round 4):** A `ZonePublishToken` is issued by the runtime as part of capability grants at session authentication time (RFC 0006 §6.3 grants `publish_zone:<zone_name>`, where `zone_name` is a zone type name, which causes the runtime to issue a token for publications to instances of that zone type). The token is bound to the session: it is invalidated when the session ends or when the `publish_zone:<zone_name>` capability is revoked mid-session. Tokens are not transferable between sessions. RFC 0005 must define the token issuance protocol; this RFC provides the contract: the token embeds a session-scoped, zone-type-scoped authorization proof that the runtime can verify without a round-trip. The exact encoding (HMAC, JWT, or opaque blob) is an RFC 0005 implementation decision.

---

## 12. Rust Module Structure (Informational)

Anticipated module layout when implementation begins:

```
crate: tze_scene (no GPU dependency)
├── mod id         — SceneId, ResourceId
├── mod types      — Rect, Point2D, Rgba, enums
├── mod tab        — Tab
├── mod tile       — Tile, ResourceBudget, InputMode
├── mod node       — Node, NodeData, all node type structs
├── mod zone       — ZoneType, ZoneInstance, ZoneOccupancy, ZoneRegistry, ZoneContent
├── mod mutation   — SceneMutation, MutationBatch
├── mod validate   — Validator, ValidationError, all rule checks
├── mod scene      — Scene (root), transaction pipeline, WAL
├── mod snapshot   — SceneSnapshot, serialization
├── mod diff       — SceneDiff, DiffOp, diff computation  (post-v1; v1 ships snapshot only)
├── mod hit_test   — HitTestQuery, HitTestResult, traversal
└── mod proto      — prost-generated types from scene.proto
```

`tze_scene` has no dependency on `wgpu`, `winit`, `tokio`, or any I/O runtime. It is the pure logic layer satisfying DR-V1.

---

## 13. Related RFCs

| RFC | Depends On | Topic |
|-----|-----------|-------|
| RFC 0001 (this) | — | Scene Contract: data model, mutations, hit-test |
| RFC 0002 | 0001 | Runtime Kernel: process architecture, thread model, frame pipeline, admission control, degradation |
| RFC 0003 | 0001, 0002 | Timing Model: clock domains, sync groups, timestamp semantics, drift rules |
| RFC 0004 | 0001, 0002, 0003 | Input Model: pointer/touch model, focus, gesture arbitration, IME, accessibility |
| RFC 0005 | 0001, 0002, 0003, 0004 | Session/Protocol: gRPC API, session lifecycle, MCP mapping |
| RFC 0006 | 0001 | Configuration: config file schema, display profiles, zone registry startup format |
| RFC 0007 | 0001, 0002 | System Shell: chrome layer UI, tab bar, override controls, privilege prompts |

---

## 14. Review Record

### Round 1 — Doctrinal Alignment Deep-Dive (2026-03-22)

**Reviewer:** rig-5vq.11 agent worker
**Focus:** Completeness — does the RFC cover every doctrine section it cites? Are doctrine commitments silently dropped?
**Doctrine read:** presence.md, architecture.md, security.md, validation.md, v1.md, failure.md

---

#### Doctrinal Alignment Score: 4/5

The RFC faithfully implements the core doctrine structure (scene hierarchy, transaction atomicity, lease-scoped namespaces, zone publishing model, hit-test priority order, durable vs. ephemeral state split, DR-V1 through DR-V4 traceability). Quantitative requirements are traceable to specific passages.

**Gaps that reduced score from 5:**

- `latency_class` and `update_policy` are explicit tile properties in presence.md ("Tiles are territories with … update policy … latency class") but were absent from the `Tile` struct and proto. **Fixed in this round.**
- `ZoneDefinition` lacked `layer_attachment` despite presence.md's "Layer attachment" subsection making it a first-class part of zone anatomy. The zone-to-tile mapping note referenced it informally but the data structure did not model it. **Fixed in this round.**
- Zone publications (`ZonePublishRecord`) lacked `expires_at_ms` and `publish_key` despite presence.md explicitly listing "TTL" and "key (for merge-by-key zones)" as publication fields. **Fixed in this round.**
- The commit pipeline diagram had an "append to WAL" step that contradicted v1.md's explicit deferral of the WAL-backed diff path. **Fixed in this round.**
- `ZoneDefinitionProto.accepted_media_types` used `repeated string` (untyped) instead of the typed `ZoneMediaType` enum, violating the no-JSON/no-untyped-strings-on-hot-paths principle. **Fixed in this round.**

---

#### Technical Robustness Score: 4/5

Data structures are correct and well-specified. The transaction pipeline is sound: all-or-nothing semantics, ordered validation steps, monotonic sequence numbers, single writer lock with correct concurrency properties. Performance budgets are quantified and hardware-normalized per validation.md. Hit-test algorithm is correct and complete. Protobuf schema is well-formed with proper zero-value semantics documented.

**Minor issues noted:**

- `NotificationPayload.urgency` was `uint32` (magic numbers). Replaced with typed `NotificationUrgency` enum for API clarity and wire safety. **Fixed in this round.**
- Tile invariants were incomplete: the semantics of `latency_class + ClockedMedia` requiring a sync_group, and the canonical `latency_class + update_policy` pairings, were not documented. **Fixed in this round.**

**Items deferred to later rounds or design:**

- `#[no_std]` compatibility for `SceneId::new()` (noted in Open Questions §11.4) — architectural choice needed.
- Tile bounds reference frame / logical pixel definition (§11.5) — deferred to Compositor RFC.

---

#### Cross-RFC Consistency Score: 3/5 (pre-fix) → 4/5 (post-fix)

The §13 Related RFCs table was wrong: it listed the old issue-description numbering (RFC 0002 as "Session/Protocol") rather than the actual RFC numbers (RFC 0002 = Runtime Kernel, RFC 0003 = Timing, RFC 0004 = Input, RFC 0005 = Session/Protocol). This was a documentation error, not a semantic contradiction, but it would have confused implementors integrating across RFCs. **Fixed in this round.**

RFC 0004 (Input) references RFC 0001 §5 (hit-test) correctly. RFC 0005 (Session/Protocol) imports scene types from RFC 0001. No type contradictions detected in the portions read. `LatencyClass` and `UpdatePolicy` enums are new to this RFC and must be consumed by RFC 0002 (Runtime Kernel) and RFC 0003 (Timing) — those RFCs should reference this RFC's definitions rather than define their own.

---

#### Actionable Findings Summary

| # | Severity | Location | Finding | Status |
|---|----------|----------|---------|--------|
| 1 | MUST-FIX | §13 Related RFCs | Wrong RFC numbers; old numbering scheme | Fixed |
| 2 | MUST-FIX | §2.3 Tile, §7.1 proto | `latency_class` and `update_policy` absent from tile despite presence.md mandate | Fixed |
| 3 | MUST-FIX | §3.2 / §9.2 commit diagram | "append to WAL" in v1 commit path contradicts §4.2 WAL deferral | Fixed |
| 4 | MUST-FIX | §7.1 `ZoneDefinitionProto` | `accepted_media_types` is `repeated string`; should be typed `ZoneMediaType` enum | Fixed |
| 5 | SHOULD-FIX | §2.5 `ZoneDefinition`, §7.1 | `layer_attachment` absent; presence.md "Layer attachment" is first-class zone anatomy | Fixed |
| 6 | SHOULD-FIX | §7.1 `NotificationPayload` | `urgency` is raw `uint32`; should be typed enum | Fixed |
| 7 | SHOULD-FIX | §4.1 `ZonePublishRecord` | Missing `expires_at_ms` and `publish_key`; presence.md lists TTL and key as publication fields | Fixed |
| 8 | CONSIDER | §2.3 Tile invariants | Canonical `latency_class + update_policy` pairings not documented | Fixed (added as invariant note) |

---

*Review round 1 complete. All MUST-FIX and SHOULD-FIX items addressed. No dimension scored below 3.*

---

### Round 2 — Technical Architecture Scrutiny (2026-03-22)

**Reviewer:** rig-5vq.12 agent worker
**Focus:** Technical rigor — would a senior systems architect find holes? Data structure choices, concurrency safety, protocol correctness, error handling completeness, platform-specific feasibility, performance budget realism, API ergonomics, edge cases in state machines, cross-RFC interaction.
**Doctrine read:** vision.md, v1.md, architecture.md, presence.md, failure.md, validation.md

---

#### Doctrinal Alignment Score: 4/5

No new doctrinal gaps found — round 1 fixes held. The RFC correctly models all doctrine-mandated structures. Score unchanged from round 1.

---

#### Technical Robustness Score: 4/5

The data model is sound, the transaction pipeline is well-specified, and the concurrency model (single writer lock, serialized batches, monotonic sequence) is correct. Performance budgets are quantified. Hit-test algorithm is complete.

**Issues found and fixed in this round:**

1. **Timestamp unit mismatch (cross-RFC):** RFC 0001 used millisecond-resolution (`_ms`) timestamp fields throughout Rust structs and the proto schema. RFC 0003 §3.1 and §7.2 establish microsecond (`_us`) as the authoritative resolution for all timing fields in the system. RFC 0003 round 1 explicitly tracked "A formal RFC 0001 amendment is tracked separately." This amendment is applied in this round. All `_ms` timestamp fields renamed to `_us`; units changed to UTC microseconds. **Fixed.**

2. **`BatchRejected` error variants used unstructured `string` for structured errors:** `rate_limited` and `batch_too_large` in the proto `BatchRejected.oneof` were typed as `string` (unstructured), losing the `limit_hz/current_hz` and `max/got` fields present in the Rust `BatchError` enum. The architecture.md error model requires structured, machine-readable errors with context fields. Added typed `RateLimitExceededError` and `BatchSizeExceededError` messages. **Fixed.**

3. **`SceneMutation` oneof missing sync group lifecycle mutations:** RFC 0003 §7.2 specifies that `CreateSyncGroupMutation` (field 21) and `DeleteSyncGroupMutation` (field 22) must be added to the `SceneMutation` oneof because sync group creation is now an explicit scene operation, not an implicit side effect. These mutations were absent from RFC 0001's proto. Added with corresponding Rust variants. **Fixed.**

4. **`SceneSnapshot` flat `Vec` lacked reconstruction documentation:** The snapshot uses flat `Vec<Tile>` and `Vec<Node>` for deterministic serialization, but the reconstruction algorithm (how to reassemble the tree from the flat lists) was undocumented. Agents reconnecting via snapshot need this. Added reconstruction comment to both the Rust struct and proto message. **Fixed.**

5. **`InsertNodeMutation.parent_id` zero-value semantics undocumented in proto:** The Rust type is `Option<SceneId>` (None = root), but the proto uses a bare `SceneId` with zero-bytes = "no parent" convention. This creates a footgun: implementors may not know that a zero `parent_id` makes the node the tile root rather than being an invalid/unset ID. Added explicit comment. **Fixed.**

**Issues fixed (SHOULD-FIX):**

6. **`ResourceBudget.max_nodes` type admits invalid values:** `u8` allows 255, but the hard cap is 64. The type and invariant are now explicit: valid range is `[1, 64]`; values above 64 are rejected with `VALIDATION_ERROR_INVALID_FIELD_VALUE`. **Fixed.**

7. **Exclusive-z conflict invariant silently included zone tiles:** Invariant 3 said "no two tiles" but runtime-managed zone tiles are pinned at z=MAX (content layer) and are exempt from agent z_order conflict checks. Clarified to "agent-owned tiles" with an explicit note about zone tile exemption. **Fixed.**

---

#### Cross-RFC Consistency Score: 4/5

No new cross-RFC contradictions found beyond the already-tracked timestamp migration. That migration is now applied (see Technical Robustness finding 1). The sync group mutation additions (finding 3) resolve the outstanding cross-reference from RFC 0003 §7.2. Score unchanged from post-fix round 1.

**Remaining open cross-RFC items (deferred to later rounds or future RFCs):**
- `#[no_std]` compatibility for `SceneId::new()` (§11.4) — clock trait injection required; deferred to implementation.
- Tile bounds logical-pixel definition (§11.5) — deferred to Compositor RFC.
- `SyncGroupConfig` serialization format for `CreateSyncGroupMutation.config` — defined in RFC 0003 §7.1; implementors should use that definition.

---

#### Actionable Findings Summary — Round 2

| # | Severity | Location | Finding | Status |
|---|----------|----------|---------|--------|
| R2-1 | MUST-FIX | All `_ms` timestamp fields | Timestamp unit mismatch with RFC 0003 (ms vs μs); amendment tracked in RFC 0003 §7.2 | Fixed |
| R2-2 | MUST-FIX | §3.4 `BatchRejected` proto | `rate_limited` and `batch_too_large` are unstructured `string`; should be typed error messages | Fixed |
| R2-3 | MUST-FIX | §7.1 `SceneMutation` oneof | `CreateSyncGroupMutation` (field 21) and `DeleteSyncGroupMutation` (field 22) absent; required by RFC 0003 §7.2 | Fixed |
| R2-4 | MUST-FIX | §4.1 `SceneSnapshot` / §7.1 proto | Flat Vec layout undocumented — agents need reconstruction algorithm to use snapshot | Fixed |
| R2-5 | MUST-FIX | §7.1 `InsertNodeMutation` | `parent_id` zero-value semantics (= tile root) not documented in proto; footgun for implementors | Fixed |
| R2-6 | SHOULD-FIX | §2.3 `ResourceBudget.max_nodes` | `u8` type admits values 65–255 which are invalid; invariant and comment clarified | Fixed |
| R2-7 | SHOULD-FIX | §2.3 Tile invariant 3 | Exclusive-z conflict invariant incorrectly included runtime-managed zone tiles | Fixed |

---

*Review round 2 complete. All MUST-FIX and SHOULD-FIX items addressed. No dimension scored below 3.*

---

### Cross-RFC Fix — CreateSyncGroupMutation Alignment (rig-5vq.21)

**Date:** 2026-03-22

**[MUST-FIX → FIXED]** `CreateSyncGroupMutation` had two fields (`SceneId id = 1; bytes config = 2;`) while RFC 0003 §7.1 defines it as one field (`SyncGroupConfig config = 1;`). The RFC 0001 form was an early draft artifact predating RFC 0003's `SyncGroupConfig` message. The forms were wire-incompatible (different field numbers/types). Fixed: RFC 0001 §7.1 proto and Rust enum variant updated to match RFC 0003 canonical form — `SyncGroupConfig config = 1` (group ID is `config.id`). No change to `DeleteSyncGroupMutation` (both RFCs already used `sync_group_id = 1`).

### Round 3 — Cross-RFC Consistency and Integration (2026-03-22)

**Reviewer:** rig-5vq.13 agent worker
**Focus:** Cross-RFC coherence — do shared types match? Do capability names align? Are there contradictory requirements? Checks RFC 0008 (Lease Governance) and RFC 0009 (Policy & Arbitration) integration.
**Doctrine read:** presence.md (zone anatomy, publications), architecture.md (capability model), security.md (capability scopes)

---

#### Doctrinal Alignment Score: 4/5

No new doctrinal regressions. Prior round fixes held. Score unchanged.

---

#### Technical Robustness Score: 4/5

No new technical regressions. Score unchanged.

---

#### Cross-RFC Consistency Score: 4/5 (up from initial 3/5 with fixes applied)

Several cross-RFC consistency issues were found and fixed in this round:

**Issues found and fixed:**

1. **`PublishToZoneMutation` lacked `content_classification` field (MUST-FIX → Fixed):** presence.md §"Zone anatomy" explicitly lists "privacy classification" as a first-class field in zone publications. RFC 0009 §2.3 (privacy gate) references `VisibilityClassification` as the per-publication privacy input, but RFC 0001's `PublishToZoneMutation` had no such field — agents had no wire mechanism to declare content classification. Added `ContentClassification` enum and `content_classification` field (field 6) to `PublishToZoneMutation`. See §2.5 and §7.1.

2. **Capability name inconsistency — non-canonical identifiers in §3.3 diagram and §9.4 (MUST-FIX → Fixed):** RFC 0001 used `CREATE_TILE`, `WRITE_SCENE`, `zone:publish:subtitle` (colon-separated) in its code and diagrams. RFC 0006 §6.3 (as amended by RFC 0005 Round 14) defines the authoritative `snake_case` scheme: `create_tiles`, `modify_own_tiles`, `publish_zone:<zone_name>`. These identifiers must be consistent for config validation, audit logging, and capability grant enforcement to work correctly. Updated §3.3 lease check, §9.4 diagram. Added normative note clarifying the naming convention.

3. **`ResourceBudget` two-struct confusion (SHOULD-FIX → Fixed):** RFC 0001 defines `ResourceBudget` with 3 per-tile fields; RFC 0008 §4/§10 defines a separate `ResourceBudget` with 7 lease-level fields (different field names). These serve genuinely different purposes but the identical struct name across different proto packages creates implementor confusion. Added explicit doc-comment to the Rust struct and proto `message ResourceBudget` in §7.1 explaining the two-budget design, their packages (`tze_hud.scene.v1` vs `tze.lease.v1`), and their relationship.

**Remaining known cross-RFC items (not blocking):**
- RFC 0008 §11 errata for RFC 0001 §3.3 (lease validity: only ACTIVE leases are valid) incorporated into §3.3 note above.
- `SyncGroupConfig` serialization format: RFC 0003 §7.1 is authoritative; already noted in §11.7.

---

#### Actionable Findings Summary — Round 3

| # | Severity | Location | Finding | Status |
|---|----------|----------|---------|--------|
| R3-1 | MUST-FIX | §2.5 `PublishToZone`, §7.1 proto | `content_classification` absent; presence.md mandates privacy classification on zone publications; RFC 0009 §2.3 references it | Fixed |
| R3-2 | MUST-FIX | §3.3 lease check, §9.4 diagram | Non-canonical capability names (`CREATE_TILE`, `WRITE_SCENE`, `zone:publish:`) contradict RFC 0006 §6.3 canonical `snake_case` scheme | Fixed |
| R3-3 | SHOULD-FIX | §2.3 `ResourceBudget`, §7.1 proto | Two `ResourceBudget` structs with same name across RFC 0001 and RFC 0008 create implementor confusion; no documentation of split | Fixed |

---

*Review round 3 complete. All MUST-FIX and SHOULD-FIX items addressed. No dimension scored below 3.*

---

### Round 4 — Final Hardening and Quantitative Verification (2026-03-22)

**Reviewer:** rig-5vq.14 agent worker
**Focus:** Final shipping readiness — quantitative verification, wire format completeness, state machine exhaustiveness, zero-ambiguity for implementors.
**Doctrine read:** presence.md, architecture.md, security.md, validation.md, v1.md
**RFCs consulted:** 0001 (this), 0003, 0005, 0006, 0008, 0009

---

#### Doctrinal Alignment: 5/5

All prior round fixes held. The RFC faithfully implements the full doctrine mandate:

- Scene hierarchy (Tab → Tile → Node) correctly models presence.md §"Tabs and tiles"
- Zone anatomy (type → instance → publication → occupancy) is fully represented in §2.5
- `content_classification` on zone publications (presence.md §"Zone anatomy", presence.md §"Publication") is present in `PublishToZoneMutation` (added R3) and now also in `ZonePublishRecord` (added R4 — see MUST-FIX R4-1 below)
- Hit-test priority (Chrome → Content tiles by z_order descending → Passthrough) matches architecture.md §"Compositing model" and §"Policy arbitration" Step 1
- Four message classes are represented by `LatencyClass` and `UpdatePolicy` enums
- Transaction atomicity (no partial apply) matches presence.md §"Scene mutations are atomic"
- `DR-V1` through `DR-V4` are explicitly satisfied (§ "Design Requirements Satisfied")
- Durable vs. ephemeral state split matches architecture.md §"Resource lifecycle" and failure.md
- Screen sovereignty enforced: `agent_namespace` is server-derived, not client-supplied (`reserved 2` in `MutationBatch`)

No doctrinal gaps found after R4 fixes are applied.

---

#### Technical Robustness: 5/5

**Quantitative requirements: all present and traceable**

| Metric | Stated Requirement | Source |
|--------|-------------------|--------|
| Snapshot serialization | < 1ms for 100 tiles / 1000 nodes | §10, §4.1 |
| Hit-test | < 100μs for 50 tiles | §5.1, §10 |
| Validation | < 200μs per 10-mutation batch | §3.2, §10 |
| Commit (lock→lock) | < 50μs for 10 mutations | §3.2 |
| Full pipeline p99 | < 300μs for 10 mutations | §3.2 |
| Memory per tile | < 1KB structural overhead | §8 |
| Max scene | 256 tabs × 1024 tiles × 64 nodes | §2.1, §8 |
| Max markdown | 65535 UTF-8 bytes | §2.4, §8 |

All budgets have stated hardware reference ("single core at 3GHz equivalent, normalized per validation.md calibration"). Budget basis is defensible: validation of 10 mutations at 200μs is consistent with O(n) checks on small integer structures with no IO.

**Performance budget arithmetic check:**
- 10 mutations × 200μs = 2ms per second per agent for validation only, well within a 16.6ms frame budget shared across multiple agents
- 50μs commit lock per batch × 60fps × up to N simultaneous agents: at 3 agents each submitting 1 batch/frame, commit latency is 150μs/frame — still < 1% of the 16.6ms budget

**Wire format completeness:**
- All enums have UNSPECIFIED = 0 default (proto3 best practice)
- Zero-value conventions are documented for SceneId (absent = zero bytes), timestamps (0 = not set), and sync_group (0 = no group)
- `agent_namespace` is `reserved 2` with a security comment
- `BatchRejected` has typed structured error messages for all four error classes (ParseError, ValidationError, RateLimitExceeded, BatchSizeExceeded)
- `ValidationErrorCode` enum covers all 16 failure modes across all mutation types

**State machine completeness:**
- HitRegionLocalState (hovered, pressed, focused) is defined but transitions are fully specified in RFC 0004 §7.1 — cross-reference is explicit
- Lease state machine (ACTIVE → SUSPENDED → REVOKED) is fully specified in RFC 0008 §3 — cross-reference in §3.3 is present
- Tab lifecycle: no explicit state machine needed — tabs are either present or absent; `display_order` uniqueness invariant is specified
- Tile lifecycle: created via lease, deleted on lease revoke or explicit mutation; lease validity check in §3.3 closes the loop

**Edge case coverage (verified against §3.3 and §2.1–2.4):**
- Node cycle detection: invariant check §3.3.5 (post-batch invariant simulation step 3)
- Duplicate ID collision: invariants §3.3.5 steps 1–2
- Empty tile (no root node): allowed by design, `root_node = None`
- Max nodes exceeded: `VALIDATION_ERROR_NODE_COUNT_EXCEEDED`
- Bounds out of display area: `VALIDATION_ERROR_BOUNDS_OUT_OF_RANGE`
- Wrong media type for zone: `VALIDATION_ERROR_ZONE_TYPE_MISMATCH`
- Expired lease: `VALIDATION_ERROR_LEASE_EXPIRED` vs `VALIDATION_ERROR_LEASE_NOT_FOUND` (both exist)

**Issues found and fixed in this round:**

1. **`ZonePublishRecord` / `ZonePublishRecordProto` missing `content_classification` (MUST-FIX R4-1 → Fixed):** Added `content_classification` field to `PublishToZoneMutation` in R3, but the same field was not added to `ZonePublishRecord` (Rust snapshot struct §4.1) or `ZonePublishRecordProto` (proto §7.1). The snapshot is the reconnect mechanism — an agent reconnecting via full snapshot must reconstruct the active privacy classification of each zone publication. Without this field in the record, the privacy gate (RFC 0009 §2.3) cannot correctly enforce redaction on publication content that was classified at publish time. **Fixed: added `content_classification: Option<ContentClassification>` to the Rust struct and `content_classification: ContentClassification = 7` to the proto.**

2. **Tab mutation validation rules absent from §3.3 (MUST-FIX R4-2 → Fixed):** The Validation Rules section (§3.3) specifies the Lease Check for tile mutations and zone publications, but the five tab mutations (`CreateTab`, `DeleteTab`, `RenameTab`, `ReorderTab`, `SwitchActiveTab`) had no stated capability requirement. The capability `manage_tabs` was mentioned only in a capability-name note (not a validation rule). An implementor cannot build the tab validation path from this RFC alone — they would have to guess which capability is required, or worse, allow unauthenticated tab mutation. Similarly, `CreateSyncGroup`/`DeleteSyncGroup` had no validation rule. **Fixed: added explicit capability check rules for tab mutations (`manage_tabs`) and sync group mutations (`manage_sync_groups`) with a note that these are capability-gated rather than lease-gated because tabs and sync groups are not agent-owned via the lease system.**

3. **Open Question §11.3 snapshot checksum coverage resolved to normative decision (SHOULD-FIX R4-3 → Fixed):** For a final-round RFC, leaving checksum coverage as an open question means two implementations will compute different checksums for the same snapshot, causing false reconnect validation failures. **Fixed: promoted to normative decision — checksum covers full serialization (tabs + tiles + nodes + zone registry) excluding `timestamp_us` and `checksum` itself, using protobuf binary encoding with fields in tag-ascending order.**

4. **Open Question §11.6 `ZonePublishToken` expiry semantics promoted to normative expectation (SHOULD-FIX R4-4 → Fixed):** The token was "opaque bytes" with no stated behavior contract, creating a circular dependency between RFC 0001 (which defines the token field) and RFC 0005 (which was supposed to define issuance). **Fixed: added a normative expectation — token is session-scoped and zone-scoped, invalidated on session end or capability revocation, not transferable between sessions. RFC 0005 defines the encoding; this RFC defines the semantic contract.**

---

#### Cross-RFC Consistency: 5/5

All cross-RFC inconsistencies found in rounds 1–3 have been resolved. No new contradictions found.

**Verified cross-RFC alignments:**

- `ContentClassification` enum values match RFC 0009 §2.3 viewer access matrix (Public, Household, Private, Sensitive)
- `ValidationErrorCode` covers all error conditions that RFC 0005 §5.x (error response model) expects to surface
- `LatencyClass` and `UpdatePolicy` canonical pairings in §2.3 are consistent with RFC 0002 (Runtime Kernel) frame pipeline stages
- `SyncGroupConfig` in `CreateSyncGroupMutation` defers to RFC 0003 §7.1 — cross-reference is explicit
- Capability names in §3.3 and §9.4 use canonical `snake_case` (RFC 0006 §6.3, as amended by RFC 0005 Round 14): `create_tiles`, `modify_own_tiles`, `manage_tabs`, `publish_zone:<name>`
- `manage_sync_groups` capability (new in R4) should be added to RFC 0006 §6.3 canonical capability table — noted as a discovered item for RFC 0006

---

#### Actionable Findings Summary — Round 4

| # | Severity | Location | Finding | Status |
|---|----------|----------|---------|--------|
| R4-1 | MUST-FIX | §4.1 `ZonePublishRecord`, §7.1 `ZonePublishRecordProto` | `content_classification` absent from snapshot record; added to mutation in R3 but not propagated to snapshot/diff; privacy gate cannot reconstruct classification on reconnect | Fixed |
| R4-2 | MUST-FIX | §3.3 Validation Rules | Tab mutations (`CreateTab` etc.) and sync group mutations (`CreateSyncGroup` etc.) have no stated capability requirement; implementors cannot build the validation path | Fixed |
| R4-3 | SHOULD-FIX | §11.3 Open Questions | Snapshot checksum coverage was an open question; for final-round RFC this must be normative; ambiguity causes divergent checksum implementations | Fixed |
| R4-4 | SHOULD-FIX | §11.6 Open Questions | `ZonePublishToken` expiry semantics deferred entirely to RFC 0005; creates circular dependency; needs normative expectation in this RFC | Fixed |

---

#### Discovered Issue for RFC 0006

The capability `manage_sync_groups` was introduced in the R4 validation rules fix for sync group mutations. RFC 0006 §6.3 canonical capability table does not list this capability. RFC 0006 should add `manage_sync_groups` to its capability table for consistency. This is out of scope for this RFC but should be tracked.

---

#### Final Scores

| Dimension | Score | Rationale |
|-----------|-------|-----------|
| Doctrinal Alignment | **5/5** | All doctrine commitments faithfully implemented. All prior gaps fixed and held. R4 fix for `content_classification` in snapshot closes the last doctrinal gap. |
| Technical Robustness | **5/5** | All quantitative requirements present with hardware reference. Wire format complete and unambiguous. State machines exhaustive or explicitly cross-referenced. All edge cases covered. Tab/sync-group validation rules now complete. |
| Cross-RFC Consistency | **5/5** | No remaining contradictions. All shared types align. R4 normative decisions on checksum and token semantics close the last open questions that would have caused inter-RFC divergence. |

All dimensions ≥ 4. All dimensions ≥ 3. Round 4 (final) is complete. The Scene Contract RFC is ready for implementation.

---

*Review round 4 complete. All MUST-FIX and SHOULD-FIX items addressed. All dimensions scored ≥ 4. RFC is implementation-ready.*

---

### Zone Ontology Alignment — rig-cu4 (2026-03-22)

**Context:** External review (Round 4, scored 8.1/10) identified that RFC 0001's zone model was thinner than presence.md §"Zone anatomy" specifies. `ZoneDefinition` conflated the type-schema and the instance-placement layers, and occupancy had no data structure.

**Changes made:**

- **§1.2 Namespace Isolation:** Updated `ZoneId` note to distinguish zone types vs zone instances; tokens are now described as scoped to zone type name with instance resolution by the runtime.
- **§1.3 ID Namespacing Diagram:** Updated zone registry label to show types and instances separately.
- **§2.5 Zone Registry:** Split `ZoneDefinition` into three new structs following the four-level ontology:
  - `ZoneType` — schema layer: accepted media types, contention policy, rendering defaults, privacy ceiling, transport constraint.
  - `ZoneInstance` — placement layer: type reference, tab binding, layer attachment, geometry policy. Multiple instances of the same type can exist across different tabs.
  - `ZoneOccupancy` — resolved state layer: active publications, effective geometry. Agents can query but cannot set occupancy. Query API deferred to post-v1; struct defined now for ontological completeness.
  - `ZoneRegistry` updated to `HashMap<String, ZoneType>` + `Vec<ZoneInstance>`.
  - Added `ResolvedGeometry` for future occupancy query support.
  - Added V1 scope note: instances are static (config-loaded), occupancy query deferred.
- **§2.5 Zone-to-tile mapping:** Updated to reference `ZoneInstance.layer_attachment` rather than `ZoneDefinition`.
- **§3.1 PublishToZone mutation:** Added comment clarifying `zone_name` is a zone type name that the runtime resolves to the active tab's `ZoneInstance`.
- **§3.3 Lease Check:** Added note that `PublishToZone` is rejected with `ZoneNotFound` if no instance of the named type exists in the active tab.
- **§4.1 `ZoneRegistrySnapshot`:** Expanded to include `zone_types: Vec<ZoneType>`, `zone_instances: Vec<ZoneInstance>`, and `active_publishes: Vec<ZonePublishRecord>`. `ZonePublishRecord` updated: `zone_name` field replaced by `instance_id: SceneId` + `zone_type_name: String` (for readability).
- **§6.1 Durable State:** Updated zone registry description to "zone types and zone instances."
- **§6.2 Ephemeral State:** Updated to clarify zone types and instances persist; only publications are cleared.
- **§7.1 Protobuf:** Replaced `ZoneDefinitionProto` with `ZoneTypeProto` + `ZoneInstanceProto` + `ZoneOccupancyProto` + `ResolvedGeometryProto`. Updated `ZonePublishRecordProto` (`instance_id` field 1, `zone_type_name` field 2; renumbered content fields accordingly). Updated `ZoneRegistrySnapshot` to include `zone_types`, `zone_instances`, `active_publishes`. Updated `ZonePublishChanged` diff op to use `instance_id` instead of `zone_name`. Updated `PublishToZoneMutation` and `ClearZoneMutation` comments.
- **§9.1 Diagram:** Updated zone tile label to show `ZoneInstance` (tab + type).
- **§11 Open Questions:** Updated checksum coverage (§11.3) and zone token note (§11.6) to reference new type/instance terminology.
- **§12 Module Structure:** Updated `mod zone` export list.

**Invariants preserved:**
- Agents still publish by zone type name (present.md §"Two APIs" — publish by name, no scene context needed). The runtime resolves the type name to the correct `ZoneInstance` for the agent's active tab.
- The three-layer compositor model (background/content/chrome) is unchanged.
- The z-order reservation (`ZONE_TILE_Z_MIN = 0x8000_0000`) is unchanged.
- All v1 scope constraints preserved; `ZoneOccupancy` is defined but its query API is explicitly deferred.

*End of RFC 0001.*
