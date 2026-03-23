# Scene Graph Specification

Domain: FOUNDATION
Source RFC: 0001 (Scene Contract)

---

## ADDED Requirements

### Requirement: SceneId Identity
All live scene objects (tabs, tiles, nodes, leases, zones, sync groups) SHALL be identified by a `SceneId`, which MUST be a UUIDv7 (time-ordered). SceneId provides lexicographic sortability by creation time for sequence ordering and log correlation. When serialized, SceneId MUST be encoded as a 16-byte little-endian binary representation.
Source: RFC 0001 §1.1, §4.1
Scope: v1-mandatory

#### Scenario: SceneId generation
- **WHEN** a new scene object (tab, tile, node) is created
- **THEN** the runtime MUST assign it a UUIDv7 SceneId that is unique within the runtime instance and lexicographically sortable by creation time

#### Scenario: SceneId zero value
- **WHEN** a SceneId field contains all-zero bytes (`[0u8; 16]`)
- **THEN** the runtime MUST interpret it as "absent/null" (e.g., no root node, no sync group membership)

### Requirement: ResourceId Identity
Immutable uploaded resources (images, fonts, raw buffers) SHALL be identified by a `ResourceId`, which MUST be a BLAKE3 content hash stored as raw 32 bytes (256-bit binary). Two agents uploading the same content MUST receive the same ResourceId; the runtime SHALL store the resource once. Hex encoding is a display/debug concern only and MUST NOT appear on the wire or in storage.
Source: RFC 0001 §1.1
Scope: v1-mandatory

#### Scenario: Content deduplication
- **WHEN** two agents upload an identical PNG image
- **THEN** both MUST receive the same ResourceId and the runtime MUST store only one copy of the resource

### Requirement: Namespace Isolation
Every agent session SHALL be assigned a namespace on authentication. A TileId MUST belong to exactly one namespace. Agents MUST NOT reference tiles they do not own. NodeIds MUST be unique scene-globally, not just within a tile.
Source: RFC 0001 §1.2
Scope: v1-mandatory

#### Scenario: Cross-namespace tile access denied
- **WHEN** agent "weather-agent" (namespace "wtr") attempts to mutate a tile owned by namespace "cal"
- **THEN** the runtime MUST reject the mutation with a `CapabilityMissing` or `LeaseNotFound` validation error

#### Scenario: Resource sharing across namespaces
- **WHEN** agent "cal" references a ResourceId uploaded by agent "wtr"
- **THEN** the runtime MUST allow read access (default sharing policy: read-allowed, write-disallowed)

### Requirement: Scene Graph Hierarchy
The scene graph MUST be a tree rooted at a single Scene node with the hierarchy: Scene -> Tab[] -> Tile[] -> Node[]. A Scene SHALL have 0-256 Tab objects. A Tab SHALL have 0-1024 Tile objects. A Tile SHALL have 0-64 Node objects arranged as an acyclic tree. No TabId, TileId, or NodeId SHALL appear more than once in the scene graph.
Source: RFC 0001 §2.1
Scope: v1-mandatory

#### Scenario: Tab limit enforcement
- **WHEN** an agent attempts CreateTab and 256 tabs already exist
- **THEN** the runtime MUST reject the mutation with a `BudgetExceeded` validation error

#### Scenario: Tile limit enforcement
- **WHEN** an agent attempts CreateTile on a tab that already has 1024 tiles
- **THEN** the runtime MUST reject the mutation with a `BudgetExceeded` validation error

#### Scenario: Node limit enforcement
- **WHEN** an agent attempts InsertNode on a tile that already has 64 nodes
- **THEN** the runtime MUST reject the mutation with a `NodeCountExceeded` validation error

#### Scenario: Duplicate ID rejection
- **WHEN** an agent submits CreateTile with a TileId that already exists in the scene
- **THEN** the runtime MUST reject the batch with a `DuplicateId` validation error

### Requirement: Tab CRUD Operations
The runtime MUST support tab lifecycle mutations: CreateTab (with id, name, display_order), DeleteTab, RenameTab, ReorderTab, and SwitchActiveTab. Tab name MUST be non-empty and at most 128 UTF-8 bytes. Display_order values MUST be unique across all tabs. Tab mutations MUST require the `manage_tabs` capability.
Source: RFC 0001 §2.2, §3.1, §3.3
Scope: v1-mandatory

#### Scenario: Create and switch tab
- **WHEN** an agent with `manage_tabs` capability submits CreateTab followed by SwitchActiveTab
- **THEN** the new tab MUST be created with the specified name and display_order, and MUST become the active tab

#### Scenario: Tab rename
- **WHEN** an agent submits RenameTab with a new name of 100 UTF-8 bytes
- **THEN** the tab name MUST be updated

#### Scenario: Tab name too long
- **WHEN** an agent submits CreateTab with a name exceeding 128 UTF-8 bytes
- **THEN** the runtime MUST reject the mutation with `InvalidFieldValue`

#### Scenario: Tab mutation without capability
- **WHEN** an agent without `manage_tabs` capability submits CreateTab
- **THEN** the runtime MUST reject the mutation with `CapabilityMissing`

### Requirement: Tile CRUD Operations
The runtime MUST support tile lifecycle mutations: CreateTile, UpdateTileBounds, UpdateTileZOrder, UpdateTileOpacity, UpdateTileInputMode, UpdateTileSyncGroup, UpdateTileExpiry, and DeleteTile. Tile operations MUST require a valid lease on the tile's namespace. CreateTile MUST additionally require `create_tiles` capability. All tile mutations MUST require `modify_own_tiles` capability.
Source: RFC 0001 §2.3, §3.1, §3.3
Scope: v1-mandatory

#### Scenario: Create tile with valid lease
- **WHEN** an agent with `create_tiles` and `modify_own_tiles` capabilities and a valid lease submits CreateTile
- **THEN** the tile MUST be created with the specified bounds, z_order, and opacity

#### Scenario: Tile mutation with expired lease
- **WHEN** an agent submits UpdateTileBounds but the tile's lease has expired
- **THEN** the runtime MUST reject the mutation with `LeaseExpired`

#### Scenario: Delete tile
- **WHEN** an agent submits DeleteTile for a tile it owns with a valid lease
- **THEN** the tile and all its nodes MUST be removed from the scene graph

### Requirement: Tile Field Invariants
Tile opacity MUST be in [0.0, 1.0]. Tile bounds width and height MUST be > 0.0. Tile bounds MUST be fully contained within the tab's display area. Agent-owned tiles MUST have z_order < ZONE_TILE_Z_MIN (0x8000_0000). The resource_budget.max_nodes MUST be in [1, 64]; values above 64 MUST be rejected with VALIDATION_ERROR_INVALID_FIELD_VALUE.
Source: RFC 0001 §2.3
Scope: v1-mandatory

#### Scenario: Opacity out of range
- **WHEN** an agent submits UpdateTileOpacity with opacity = 1.5
- **THEN** the runtime MUST reject the mutation with `InvalidFieldValue`

#### Scenario: Zero-size bounds
- **WHEN** an agent submits CreateTile with width = 0.0
- **THEN** the runtime MUST reject the mutation with `BoundsOutOfRange`

#### Scenario: Bounds outside tab area
- **WHEN** an agent submits UpdateTileBounds with x + width exceeding the tab display width
- **THEN** the runtime MUST reject the mutation with `BoundsOutOfRange`

#### Scenario: Z-order in reserved zone band
- **WHEN** an agent submits CreateTile with z_order = 0x8000_0000
- **THEN** the runtime MUST reject the mutation with `InvalidFieldValue` (z_order >= ZONE_TILE_Z_MIN is reserved for runtime-managed zone tiles)

### Requirement: V1 Node Types
The runtime MUST support four node types: SolidColorNode (with color and bounds), TextMarkdownNode (with CommonMark content up to 65535 UTF-8 bytes, font_size_px > 0.0, font family, color, alignment, overflow), StaticImageNode (with ResourceId, bounds, fit mode), and HitRegionNode (with bounds, interaction_id, accepts_focus, accepts_pointer). Each node MUST have a SceneId and an ordered list of child SceneIds.
Source: RFC 0001 §2.4
Scope: v1-mandatory

#### Scenario: TextMarkdownNode content limit
- **WHEN** an agent submits a TextMarkdownNode with content exceeding 65535 UTF-8 bytes
- **THEN** the runtime MUST reject the mutation with `InvalidFieldValue`

#### Scenario: StaticImageNode with unknown resource
- **WHEN** an agent submits a StaticImageNode referencing a ResourceId not known to the runtime
- **THEN** the runtime MUST reject the mutation with `ResourceNotFound`

#### Scenario: HitRegionNode local state
- **WHEN** a pointer hovers over a HitRegionNode with accepts_pointer = true
- **THEN** the runtime MUST update the node's local hovered state for immediate visual feedback without waiting for agent acknowledgement

### Requirement: Atomic Batch Mutations
Mutations MUST be submitted as atomic batches (MutationBatch). A batch MUST have a batch_id (SceneId). Maximum batch size SHALL be 1000 mutations. The entire batch MUST be validated and committed atomically: if any single mutation fails validation, the entire batch MUST be rejected with no partial application. The agent_namespace MUST NOT be carried in the batch; it MUST be derived from the authenticated session context.
Source: RFC 0001 §3.1, §3.2
Scope: v1-mandatory

#### Scenario: All-or-nothing batch rejection
- **WHEN** an agent submits a batch of 5 mutations and mutation 3 has an invalid bounds value
- **THEN** the runtime MUST reject the entire batch (all 5 mutations) and report the error at mutation_index=2

#### Scenario: Batch size exceeded
- **WHEN** an agent submits a batch with 1001 mutations
- **THEN** the runtime MUST reject the batch with `BatchSizeExceeded { max: 1000, got: 1001 }`

#### Scenario: Agent namespace not trusted from client
- **WHEN** a batch is received over gRPC
- **THEN** the runtime MUST derive the agent namespace from the authenticated session context, never from a client-supplied field in the batch

### Requirement: Transaction Validation Pipeline
Each mutation in a batch MUST pass through five ordered validation checks: (1) Lease check, (2) Budget check, (3) Bounds check, (4) Type check, (5) Invariant check (post-mutation simulation). Validation MUST verify no TileId collision, no NodeId collision, no acyclic-tree violation, no exclusive z-order conflict, and all internal references are valid.
Source: RFC 0001 §3.2, §3.3
Scope: v1-mandatory

#### Scenario: Lease check before budget check
- **WHEN** an agent submits a mutation targeting a tile with an expired lease
- **THEN** the runtime MUST reject with `LeaseExpired` before evaluating budget or bounds checks

#### Scenario: Post-mutation invariant simulation
- **WHEN** a batch of mutations would introduce an acyclic-tree violation in a tile's node tree
- **THEN** the runtime MUST reject the batch with `CycleDetected`

#### Scenario: Exclusive z-order conflict
- **WHEN** two non-passthrough tiles on the same tab would share the same z_order with overlapping bounds after a batch is applied
- **THEN** the runtime MUST reject the batch with `ZOrderConflict`

### Requirement: Transaction Concurrency
The scene graph MUST have a single writer lock. One batch commits at a time. Batches from the same agent MUST commit in submission order. Each batch's mutations MUST be contiguous (no interleaving within a batch). A monotonically increasing u64 sequence_number MUST be assigned to each commit and is the canonical ordering token for scene history.
Source: RFC 0001 §3.5
Scope: v1-mandatory

#### Scenario: Sequential batch ordering
- **WHEN** agent A submits batches B1 and B2 in order
- **THEN** the runtime MUST commit B1 before B2, and B1.sequence_number < B2.sequence_number

### Requirement: Zone Registry
The zone registry MUST be runtime-owned and loaded from configuration at startup. Agents MUST NOT create zones in v1. The zone ontology MUST have four levels: zone type (schema), zone instance (type bound to tab), publication (publish event), and occupancy (resolved state). Zone instances MUST be static in v1 (loaded from config, one instance per tab per zone type).
Source: RFC 0001 §2.5
Scope: v1-mandatory

#### Scenario: Zone type loaded from config
- **WHEN** the runtime starts with a config defining zone type "subtitle"
- **THEN** the zone registry MUST contain a ZoneType named "subtitle" with its accepted_media_types, contention_policy, and rendering_policy

#### Scenario: Agent cannot create zones
- **WHEN** an agent attempts to create a new zone type or zone instance
- **THEN** the runtime MUST reject the operation (no mutation exists for zone creation in v1)

### Requirement: Zone Publishing
Agents MUST publish to zones via PublishToZoneMutation, specifying a zone_name (type name), content, publish_token, optional expires_at_us, optional publish_key (for MergeByKey zones), and optional content_classification. The runtime MUST resolve zone_name to the ZoneInstance for the agent's active tab. Publication MUST require `publish_zone:<zone_name>` capability (RFC 0006 §6.3 canonical wire-format name) and a valid ZonePublishToken. ClearZone MUST clear all publications by the agent in the specified zone.
Source: RFC 0001 §3.1, §3.3
Scope: v1-mandatory

#### Scenario: Publish to subtitle zone
- **WHEN** an agent with `publish_zone:subtitle` capability and a valid ZonePublishToken submits PublishToZoneMutation with zone_name="subtitle"
- **THEN** the runtime MUST resolve the zone type to the subtitle ZoneInstance in the agent's active tab and publish the content

#### Scenario: Zone not found
- **WHEN** an agent publishes to zone_name="nonexistent" and no such zone type exists for the active tab
- **THEN** the runtime MUST reject with `ZoneNotFound`

#### Scenario: Invalid publish token
- **WHEN** an agent submits PublishToZoneMutation with an invalid or expired ZonePublishToken
- **THEN** the runtime MUST reject with `ZonePublishTokenInvalid`

### Requirement: Zone Contention Policies
The runtime MUST support four contention policies for zone instances: LatestWins (most recent publish replaces previous), Stack (publishes accumulate; each auto-dismisses, with configurable max_depth), MergeByKey (same key replaces, different keys coexist, with configurable max_keys), and Replace (single occupant; new publish evicts current).
Source: RFC 0001 §2.5
Scope: v1-mandatory

#### Scenario: LatestWins contention
- **WHEN** two agents publish to a LatestWins zone
- **THEN** only the most recent publication MUST be visible; the previous one is replaced

#### Scenario: MergeByKey contention
- **WHEN** agent A publishes with publish_key="temp" and agent B publishes with publish_key="humidity" to a MergeByKey zone
- **THEN** both publications MUST coexist

#### Scenario: Stack max depth
- **WHEN** publications exceed a Stack zone's max_depth
- **THEN** the oldest publication MUST be evicted

### Requirement: Zone Media Types V1
The runtime MUST support six zone media types: five v1-mandatory (StreamText, ShortTextWithIcon, KeyValuePairs, StaticImage, SolidColor) and one post-v1 (VideoSurfaceRef, deferred to post-v1 media layer). Zone publish content MUST match one of the zone type's accepted_media_types.
Source: RFC 0001 §2.5
Scope: v1-mandatory

#### Scenario: Media type mismatch
- **WHEN** an agent publishes a KeyValuePairs payload to a zone that only accepts StreamText
- **THEN** the runtime MUST reject with `ZoneMediaTypeMismatch`

### Requirement: Zone Layer Attachment
Each zone instance MUST declare a layer_attachment: Background (behind all agent tiles), Content (within the content layer z-order space at z_order >= ZONE_TILE_Z_MIN = 0x8000_0000), or Chrome (above all agent content, rendered by runtime). Content-layer zone tiles MUST participate in the same z-order traversal as agent tiles but in the reserved upper band.
Source: RFC 0001 §2.5
Scope: v1-mandatory

#### Scenario: Content zone tile z-order
- **WHEN** a Content-layer zone instance is active
- **THEN** the runtime MUST create a zone tile with z_order >= 0x8000_0000 that appears above all agent-owned tiles in the content layer

### Requirement: Hit-Testing Contract
Hit-testing MUST map a 2D point to the deepest interactive element. Traversal order: (1) Chrome layer first, (2) Content layer tiles by z_order descending, (3) within each tile, reverse tree order of nodes. Passthrough tiles MUST be skipped. HitRegionNodes MUST be the interactive primitive. The hit-test MUST return one of: NodeHit, TileHit, Passthrough, or Chrome.
Source: RFC 0001 §5.1, §5.2
Scope: v1-mandatory

#### Scenario: Chrome layer always wins
- **WHEN** a pointer event lands on a chrome element (tab bar, system indicator)
- **THEN** the hit-test MUST return ChromeHit regardless of underlying tiles

#### Scenario: Highest-z tile hit
- **WHEN** two non-passthrough tiles overlap and the pointer lands in the overlap region
- **THEN** the hit-test MUST return a hit for the tile with the higher z_order

#### Scenario: Passthrough tile skipped
- **WHEN** a pointer event lands on a tile with input_mode = Passthrough
- **THEN** the hit-test MUST skip that tile and test the next tile in z-order

### Requirement: Hit-Test Performance
Hit-testing MUST complete in < 100us for a single point query against 50 tiles, measured as pure Rust execution with no GPU involvement.
Source: RFC 0001 §5.1, §10
Scope: v1-mandatory

#### Scenario: Hit-test latency benchmark
- **WHEN** a hit-test query is executed against a scene with 50 tiles
- **THEN** the query MUST complete in less than 100us on reference hardware (single core at 3GHz equivalent)

### Requirement: Scene Snapshot Serialization
A full scene snapshot MUST be a complete, deterministic serialization of the scene graph at a specific sequence number. It MUST include: sequence, snapshot_wall_us (UTC wall-clock timestamp of the snapshot), snapshot_mono_us (monotonic timestamp of the snapshot), all tabs (ordered by display_order), all tiles, all nodes, zone_registry (types, instances, active publishes), active_tab, and a BLAKE3 checksum. Fields MUST be serialized in deterministic order (BTreeMap, not HashMap). Serialization MUST complete in < 1ms for a 100-tile scene with 1000 nodes total.
Source: RFC 0001 §4.1, §10
Scope: v1-mandatory

#### Scenario: Snapshot determinism
- **WHEN** two snapshots are taken of identical scene state
- **THEN** both MUST produce identical serialization bytes and identical BLAKE3 checksums

#### Scenario: Snapshot performance
- **WHEN** a snapshot is serialized for a scene with 100 tiles and 1000 nodes
- **THEN** serialization MUST complete in < 1ms (protobuf encode, single core, reference hardware)

### Requirement: Transaction Validation Performance
Transaction validation MUST complete in < 200us per batch of 10 mutations. Commit (lock-acquire through lock-release) MUST complete in < 50us for 10 mutations. Full path (parse, validate, commit) MUST complete in < 300us p99 for 10 mutations on reference hardware.
Source: RFC 0001 §3.2, §10
Scope: v1-mandatory

#### Scenario: Validation latency
- **WHEN** a batch of 10 mutations is submitted for validation
- **THEN** validation MUST complete in < 200us on reference hardware

#### Scenario: Commit latency
- **WHEN** a validated batch of 10 mutations is committed
- **THEN** the commit (lock-acquire through lock-release) MUST complete in < 50us

### Requirement: Struct Overhead Budgets
Per-tile struct overhead MUST be < 200 bytes (excluding texture data and nodes). Per-node struct overhead MUST be < 150 bytes (excluding content payloads). At maximum capacity (64 nodes per tile), total structural overhead per tile MUST be approximately 9.8 KB (tile struct + 64 node structs, content excluded).
Source: RFC 0001 §8, §10
Scope: v1-mandatory

#### Scenario: Tile struct size
- **WHEN** `size_of::<Tile>()` plus metadata allocation is measured
- **THEN** it MUST be less than 200 bytes

#### Scenario: Node struct size
- **WHEN** `size_of::<Node>()` plus ID allocation is measured
- **THEN** it MUST be less than 150 bytes

### Requirement: Batch Rejection Response
When a batch is rejected, the runtime MUST return a BatchRejected response containing the batch_id and a structured error. Validation errors MUST include: mutation_index (0-based), mutation_type, ValidationErrorCode, human-readable message, machine-readable context (JSON), and optional correction_hint (JSON). All error codes MUST be stable across minor versions.
Source: RFC 0001 §3.4
Scope: v1-mandatory

#### Scenario: Structured validation error
- **WHEN** a mutation fails bounds validation
- **THEN** the rejection MUST include the mutation_index, code=BoundsOutOfRange, a human-readable message, and a context JSON object identifying the field, value, and constraint

### Requirement: Ephemeral Scene State
The scene graph (tabs, tiles, nodes), active leases, live zone publishes, gRPC sessions, hit-region local state, and performance telemetry MUST be ephemeral (lost on restart). After restart, agents MUST re-establish sessions and re-create scene content. Durable resources (images, fonts) SHALL survive restart because they are content-addressed and independent of scene graph state.
Source: RFC 0001 §6
Scope: v1-mandatory

#### Scenario: Scene lost on restart
- **WHEN** the runtime process restarts
- **THEN** the scene graph MUST be empty; agents MUST reconnect and re-create their tiles and nodes

### Requirement: V1 Reconnect via Full Snapshot
In v1, when an agent reconnects, the runtime SHALL always send a full SceneSnapshot regardless of how recently the agent was connected. The agent MUST discard its prior scene state, apply the snapshot, and resume from the snapshot's sequence number.
Source: RFC 0001 §4.2
Scope: v1-mandatory

#### Scenario: Reconnecting agent receives snapshot
- **WHEN** an agent reconnects after a disconnection
- **THEN** the runtime MUST send a full SceneSnapshot; incremental diff is not available in v1

### Requirement: Incremental Diff (Deferred)
Incremental diff (WAL-backed delta sync for reconnecting agents), including SceneDiff, DiffOp, and branching reconnect logic, is deferred to post-v1. V1 ships snapshot-only reconnection.
Source: RFC 0001 §4.2
Scope: post-v1

#### Scenario: Incremental diff not available
- **WHEN** an agent requests incremental diff in v1
- **THEN** the runtime MUST provide a full snapshot instead

### Requirement: VideoSurfaceRef and WebRtcRequired (Deferred)
The VideoSurfaceRef zone media type and the WebRtcRequired transport constraint are deferred to the post-v1 media layer.
Source: RFC 0001 §2.5
Scope: post-v1

#### Scenario: VideoSurfaceRef not supported
- **WHEN** a zone type with VideoSurfaceRef is configured in v1
- **THEN** the runtime MAY accept the configuration but MUST NOT attempt to render video surface content

### Requirement: Zone Occupancy Query API (Deferred)
The ZoneOccupancy effective_geometry field and the occupancy query API are deferred to post-v1. In v1, the runtime maintains occupancy state internally; snapshots include active publications per instance but do not expose effective_geometry.
Source: RFC 0001 §2.5
Scope: post-v1

#### Scenario: Effective geometry not in v1 snapshot
- **WHEN** a scene snapshot is serialized in v1
- **THEN** the snapshot MUST include active zone publications but MUST NOT include effective_geometry data
