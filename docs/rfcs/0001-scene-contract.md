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
4. `ZoneId` is runtime-owned; agents do not create zones (in v1 — zone orchestration is a post-v1 feature). Agents hold `ZonePublishToken` grants that permit publishing to a specific zone.
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
│  │  zone-subtitle, zone-notification, ...     │         │
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
    pub created_at: u64,        // Unix timestamp, milliseconds
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
    pub present_at: Option<u64>,    // Scheduled presentation timestamp (ms); None = immediate
    pub expires_at: Option<u64>,    // Content expiry timestamp (ms); None = no auto-expiry

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

pub struct ResourceBudget {
    pub texture_bytes: u64,    // Max texture memory for this tile's nodes
    pub update_rate_hz: f32,   // Max mutation rate (mutations/second)
    pub max_nodes: u8,         // Max nodes in tile tree (default 64)
}
```

**Tile invariants:**
1. `opacity` in `[0.0, 1.0]`.
2. `bounds` must be fully contained within the tab's display area (runtime-defined; typically the display resolution).
3. No two tiles with the same `z_order` value on the same tab may both be non-passthrough and have overlapping bounds (exclusive-z conflict).
4. `width > 0.0` and `height > 0.0`.
5. `lease_id` must reference a currently-valid lease in the lease registry.
6. `resource_budget.max_nodes <= 64`.
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
    pub present_at: Option<u64>,    // Override tile-level present_at for this node
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
    pub present_at: Option<u64>,
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

```rust
pub struct ZoneRegistry {
    pub zones: HashMap<String, ZoneDefinition>,  // key = zone name (e.g., "subtitle")
}

pub struct ZoneDefinition {
    pub id: SceneId,
    pub name: String,
    pub description: String,
    pub layer_attachment: ZoneLayerAttachment,  // Which compositor layer this zone attaches to
    pub geometry_policy: GeometryPolicy,
    pub accepted_media_types: Vec<ZoneMediaType>,
    pub rendering_policy: RenderingPolicy,
    pub contention_policy: ContentionPolicy,
    pub transport_constraint: Option<TransportConstraint>,
    pub auto_clear_ms: Option<u64>,  // Auto-clear timeout; None = no auto-clear
}

/// Which compositor layer a zone instance attaches to (presence.md §"Layer attachment").
pub enum ZoneLayerAttachment {
    /// Behind all agent tiles; ambient-background zone.
    Background,
    /// Among agent tiles; z-order is pinned by runtime above all agent tiles.
    /// subtitle, notification, pip.
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
```

**Zone-to-tile mapping:** The runtime creates and manages internal tiles for each active zone. Zone tiles are in a runtime-owned namespace. The `layer_attachment` field on `ZoneDefinition` (see §2.5 struct) determines which compositor layer the zone's tile occupies:
- `Background` zones render behind all agent tiles (ambient-background).
- `Content` zones render among agent tiles at a pinned z_order above all agent-controlled z_order values (subtitle, notification, pip).
- `Chrome` zones render above all content; agents publish data but the runtime renders it (alert-banner, status-bar).

Agent tiles cannot occlude Content-layer zone tiles (as zone tiles are pinned at the highest z_order in the content layer). Chrome-layer zone tiles are entirely outside the z_order space of agent tiles.

**Contention policies:**

| Policy | Zones | Semantics |
|--------|-------|-----------|
| `LatestWins` | subtitle, ambient-background | Most recent publish replaces; no queue |
| `Stack` | notification | Accumulates; each auto-dismisses after timeout |
| `MergeByKey` | status-bar | Key-addressed; same key replaces, different coexist |
| `Replace` | pip (post-v1) | Single occupant; new publish evicts current |

---

## 3. Transaction Model

### 3.1 Mutation Batch Format

An agent submits mutations as an atomic batch:

```rust
pub struct MutationBatch {
    pub batch_id: SceneId,          // Agent-assigned; used in error responses
    pub agent_namespace: String,
    pub mutations: Vec<SceneMutation>,
    pub present_at: Option<u64>,    // Apply in one frame at or after this time
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
    UpdateTileExpiry { tile_id: SceneId, expires_at: Option<u64> },
    DeleteTile { tile_id: SceneId },

    // Node operations
    SetTileRoot { tile_id: SceneId, node: Node },
    InsertNode { tile_id: SceneId, parent_id: Option<SceneId>, node: Node },
    ReplaceNode { node_id: SceneId, node: Node },
    UpdateNodeBounds { node_id: SceneId, bounds: Rect },
    RemoveNode { node_id: SceneId },

    // Zone publishing
    PublishToZone {
        zone_name: String,
        content: ZoneContent,
        publish_token: ZonePublishToken,
        expires_at_ms: Option<u64>,   // Per-publish TTL; None = use zone auto_clear_ms
        publish_key: Option<String>,  // Key for MergeByKey zones; None for other policies
    },
    ClearZone { zone_name: String, publish_token: ZonePublishToken },
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

```
mutation targets tile T
  → T.namespace == batch.agent_namespace        (agent owns tile)
  → lease_registry.get(T.lease_id).is_valid()  (lease not expired or revoked)
  → lease has WRITE_SCENE capability
```

Zone publish mutations require `ZonePublishToken` embedded in the mutation; the token is validated against the capability registry.

#### Budget Check

For `CreateTile`:
- `agent.active_leases.len() < agent.max_leases`

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
    RateLimitExceeded { limit_hz: f32, current_hz: f32 },
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
    pub timestamp_ms: u64,                  // Wall clock at snapshot time
    pub tabs: Vec<Tab>,                     // Ordered by display_order
    pub tiles: Vec<Tile>,                   // All tiles across all tabs
    pub nodes: Vec<Node>,                   // All nodes across all tiles
    pub zone_registry: ZoneRegistrySnapshot,
    pub active_tab: Option<SceneId>,
    pub checksum: [u8; 32],                 // BLAKE3 of canonical serialization
}

pub struct ZoneRegistrySnapshot {
    pub zones: Vec<ZoneDefinition>,
    pub active_publishes: Vec<ZonePublishRecord>,
}

pub struct ZonePublishRecord {
    pub zone_name: String,
    pub publisher_namespace: String,
    pub content: ZoneContent,
    pub published_at_ms: u64,
    pub expires_at_ms: Option<u64>,  // Publication TTL; None = governed by zone auto_clear_ms
    pub publish_key: Option<String>, // Key for MergeByKey zones; None for other contention policies
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
| Zone registry | Zone definitions, geometry/rendering/contention policies | Config file |
| User preferences | Quiet hours, safe mode config, display profiles | Config file |
| Capability grants | Per-agent capability scope definitions | Config file |
| Uploaded resources | Image, font, buffer resources (content-addressed) | Blob store (filesystem) |

Durable state is written to disk on change; it is not part of the scene graph serialization.

### 6.2 Ephemeral State (lost on restart)

Ephemeral state lives entirely in memory. After a restart, agents must re-establish sessions and re-create scene content.

| Category | Notes |
|----------|-------|
| Scene graph | All tabs, tiles, nodes are recreated by agents after reconnect |
| Active leases | All leases expire on restart; agents re-request |
| Live zone publishes | Zone content is cleared on restart (tabs and zone definitions persist) |
| gRPC sessions | All sessions disconnected; agents must reconnect |
| Hit-region local state | Hover/press/focus state is reset |
| WAL / diff history | Lost on restart (no durable replay) |
| Performance telemetry | Per-session metrics discarded |

**Design rationale:** Making the scene graph ephemeral simplifies the correctness model dramatically. Agents are expected to re-create their scene state on reconnect. The lease governance model ensures they can do so within their granted capability scope. Durable resources (images, fonts) survive because they are content-addressed and independent of scene graph state.

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
  uint64   created_at_ms = 4;
}

message ResourceBudget {
  uint64 texture_bytes  = 1;
  float  update_rate_hz = 2;
  uint32 max_nodes      = 3;
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
  uint64         present_at_ms   = 10;  // 0 = immediate
  uint64         expires_at_ms   = 11;  // 0 = no expiry
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
  uint64       present_at_ms = 9;  // 0 = use tile-level present_at
}

message StaticImageNode {
  ResourceId resource_id  = 1;
  Rect       bounds       = 2;
  ImageFit   fit          = 3;
  uint64     present_at_ms = 4; // 0 = use tile-level present_at
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
message UpdateTileExpiryMutation  { SceneId tile_id = 1; uint64 expires_at_ms = 2; }
message DeleteTileMutation   { SceneId tile_id = 1; }

message SetTileRootMutation  { SceneId tile_id = 1; Node node = 2; }
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

message PublishToZoneMutation {
  string           zone_name     = 1;
  ZoneContent      content       = 2;
  ZonePublishToken publish_token = 3;
  uint64           expires_at_ms = 4;   // 0 = use zone auto_clear_ms; >0 = per-publish TTL
  string           publish_key   = 5;   // Non-empty only for MergeByKey zones
}

message ClearZoneMutation {
  string           zone_name     = 1;
  ZonePublishToken publish_token = 2;
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
  }
}

message MutationBatch {
  SceneId            batch_id         = 1;
  string             agent_namespace  = 2;
  repeated SceneMutation mutations    = 3;
  uint64             present_at_ms    = 4;  // 0 = immediate
  uint64             sequence_hint    = 5;  // 0 = no hint
}

// ─── Responses ──────────────────────────────────────────────────────────────

message BatchCommitted {
  SceneId batch_id       = 1;
  uint64  sequence       = 2;
  uint64  committed_at_ms = 3;
}

message MutationValidationError {
  uint32               mutation_index  = 1;
  string               mutation_type   = 2;
  ValidationErrorCode  code            = 3;
  string               message         = 4;
  string               context_json    = 5;   // JSON object: {field, value, constraint}
  string               correction_hint = 6;   // JSON object or empty
}

message BatchRejected {
  SceneId                    batch_id = 1;
  oneof error {
    string                   parse_error   = 2;
    MutationValidationError  validation    = 3;
    string                   rate_limited  = 4;
    string                   batch_too_large = 5;
  }
}

// ─── Snapshots and Diffs ────────────────────────────────────────────────────

message SceneSnapshot {
  uint64             sequence       = 1;
  uint64             timestamp_ms   = 2;
  repeated Tab       tabs           = 3;
  repeated Tile      tiles          = 4;
  repeated Node      nodes          = 5;
  ZoneRegistrySnapshot zone_registry = 6;
  SceneId            active_tab     = 7;   // Zero = no active tab
  bytes              checksum       = 8;   // BLAKE3, 32 bytes
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

message ZoneDefinitionProto {
  SceneId                    id                   = 1;
  string                     name                 = 2;
  string                     description          = 3;
  GeometryPolicyProto        geometry_policy      = 4;
  repeated ZoneMediaType     accepted_media_types = 5;
  RenderingPolicyProto       rendering_policy     = 6;
  ContentionPolicy           contention_policy    = 7;
  uint64                     auto_clear_ms        = 8;   // 0 = no auto-clear
  ZoneLayerAttachment        layer_attachment     = 9;   // Which compositor layer this zone attaches to
}

message ZonePublishRecordProto {
  string      zone_name           = 1;
  string      publisher_namespace = 2;
  ZoneContent content             = 3;
  uint64      published_at_ms     = 4;
  uint64      expires_at_ms       = 5;   // 0 = governed by zone auto_clear_ms
  string      publish_key         = 6;   // Non-empty only for MergeByKey zones
}

message ZoneRegistrySnapshot {
  repeated ZoneDefinitionProto    zones           = 1;
  repeated ZonePublishRecordProto active_publishes = 2;
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
  uint64       expires_at_ms = 7;   // 0 = not changed
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
  string                  zone_name = 1;
  ZonePublishRecordProto  record    = 2;   // Empty = zone cleared
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
2. All `uint64` timestamp fields use 0 to represent "not set" (zero is never a valid real timestamp in this system; the runtime started after 2025).
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

**Memory budget (target):** < 1KB per tile excluding texture data. Approximation:

```
Tile struct:     ~200 bytes
Per node avg:    ~150 bytes (varies by node type; TextMarkdownNode has variable content)
64 nodes/tile:   ~9.6 KB (at average; content excluded)
```

The 1KB/tile target excludes node content (markdown text, texture references) which is accounted via `ResourceBudget.texture_bytes`. Structural overhead (IDs, bounds, metadata) is the bounded quantity.

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
║  │  │ zone: "subtitle" z=MAX         │                            ║
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
║                    │  - CREATE_TILE          │           ║
║                    │  - WRITE_SCENE          │           ║
║                    │  - zone:publish:subtitle│           ║
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
| Memory per tile | < 1KB structural overhead (excl. texture data) | Rust `size_of` + heap alloc accounting |
| Max scene | 256 tabs × 1024 tiles/tab × 64 nodes/tile | Hard limit enforced in validation |
| Sequence number | Monotonic u64; wraps after ~1.8×10^19 | Never wraps in practice |

Hardware reference: single core at 3GHz equivalent (normalized; see validation.md calibration).

---

## 11. Open Questions

1. **Zone geometry policy config format:** The wire format is now typed protobuf (`GeometryPolicyProto`, `RenderingPolicyProto`). The config file format (TOML/YAML/JSON used for zone registry at startup) is a separate concern: it should be human-editable and deserializable into the Rust `GeometryPolicy` enum. Recommendation: TOML for authoring, convert to proto for wire. Defer config file schema to the Config/Setup RFC.

2. **WAL retention policy (post-v1):** 1000 batches or 60s, whichever is smaller, is a starting point for the deferred incremental diff extension. In v1, the WAL is used only for sequence ordering within the commit pipeline; agents reconnect via full snapshot and the WAL does not need to be queried externally. Revisit when incremental diff is implemented.

3. **Snapshot checksum coverage:** Should `SceneSnapshot.checksum` cover the full serialization including zone state, or only the scene graph (tabs/tiles/nodes)? Recommendation: full serialization for integrity; exclude volatile fields like `timestamp_ms`.

4. **`#[no_std]` compatibility:** The `SceneId` (UUIDv7) constructor requires a clock source not available in no_std. Options: (a) accept that scene graph construction requires std, (b) inject a clock trait, (c) make `new()` require a timestamp argument. Recommendation: (b) inject a clock trait for test/embedded flexibility.

5. **Tile bounds reference frame:** The spec says tile bounds are in "logical pixels relative to the tab's display area." The compositor must define what "logical pixel" means across display profiles (HiDPI, scaling). The Compositor RFC must define the coordinate space and DPI contract.

6. **Zone publish token wire format:** The `ZonePublishToken` is currently an opaque bytes field. The Session/Protocol RFC must define how tokens are issued during auth and their expiry semantics.

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
├── mod zone       — ZoneDefinition, ZoneRegistry, ZoneContent
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

*End of RFC 0001.*
