# scene-graph Specification

## Purpose
The scene graph is the authoritative data model for tze_hud. It defines the scene as a pure data structure (no GPU dependency): the identity model (SceneId, ResourceId), the hierarchy (Scene → Tab → Tile → Node), all mutation operations and their validation pipeline, atomic batch semantics, hit-testing, the zone registry, snapshot serialization, and performance budgets. Every other subsystem (compositor, session protocol, lease governance, input) depends on this contract. Source: RFC 0001 (Scene Contract).

## Requirements

### Requirement: SceneId Identity
All live scene objects (tabs, tiles, nodes, leases, zones, sync groups) SHALL be identified by a `SceneId`, which MUST be a UUIDv7 (time-ordered). UUIDv7 is monotonically increasing by creation time, enabling sequence ordering and log correlation by UUID logical value. When serialized, SceneId MUST be encoded as a 16-byte little-endian binary representation (as returned by `Uuid::to_bytes_le`). Note: the LE wire encoding does not preserve byte-level lexicographic sort order; time ordering must be recovered by parsing the UUID, not by comparing raw wire bytes.
Source: RFC 0001 §1.1, §4.1
Scope: v1-mandatory

#### Scenario: SceneId generation
- **WHEN** a new scene object (tab, tile, node) is created
- **THEN** the runtime MUST assign it a UUIDv7 SceneId that is unique within the runtime instance and lexicographically sortable by creation time

#### Scenario: SceneId zero value
- **WHEN** a SceneId field contains all-zero bytes (`[0u8; 16]`)
- **THEN** the runtime MUST interpret it as "absent/null" (e.g., no root node, no sync group membership)

---

### Requirement: ResourceId Identity
Immutable uploaded resources (images, fonts, raw buffers) SHALL be identified by a `ResourceId`, which MUST be a BLAKE3 content hash stored as raw 32 bytes (256-bit binary). Two agents uploading the same content MUST receive the same ResourceId; the runtime SHALL store the resource once. Hex encoding is a display/debug concern only and MUST NOT appear on the wire or in storage.
Source: RFC 0001 §1.1
Scope: v1-mandatory

#### Scenario: Content deduplication
- **WHEN** two agents upload an identical PNG image
- **THEN** both MUST receive the same ResourceId and the runtime MUST store only one copy of the resource

---

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

---

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

---

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

---

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

---

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

---

### Requirement: Text Stream Portal Phase-0 Uses Raw Tiles
The `text-stream-portals` phase-0 pilot SHALL use agent-owned content-layer raw tiles with existing V1 node types. The pilot MUST NOT require a new scene node type before the capability is proven.
Source: RFC 0013 (Text Stream Portals)
Scope: v1-mandatory

#### Scenario: portal pilot tile stays below runtime-managed bands
- **WHEN** a resident portal pilot creates its surface as a raw tile
- **THEN** the tile SHALL use the normal agent-owned z-order band and remain below zone-reserved and widget-reserved runtime-managed tiles

---

### Requirement: V1 Node Types
The runtime MUST support four node types: SolidColorNode (with color and bounds), TextMarkdownNode (with CommonMark content up to 65535 UTF-8 bytes, font_size_px > 0.0, font family, base color, alignment, overflow, and optional `color_runs`), StaticImageNode (with ResourceId, bounds, fit mode), and HitRegionNode (with bounds, interaction_id, accepts_focus, accepts_pointer). Each node MUST have a SceneId and an ordered list of child SceneIds.
Source: RFC 0001 §2.4
Scope: v1-mandatory

#### Scenario: TextMarkdownNode content limit
- **WHEN** an agent submits a TextMarkdownNode with content exceeding 65535 UTF-8 bytes
- **THEN** the runtime MUST reject the mutation with `InvalidFieldValue`

#### Scenario: TextMarkdownNode with empty color_runs uses base color
- **WHEN** a TextMarkdownNode has `color_runs` empty (default)
- **THEN** the entire content MUST be rendered in the node's base `color`
- **AND** all existing behavior is preserved (full backward compatibility)

#### Scenario: TextMarkdownNode with color_runs applies inline styling
- **WHEN** a TextMarkdownNode has one or more `color_runs`
- **THEN** each run MUST color its `[start_byte, end_byte)` byte range in the run's color
- **AND** bytes not covered by any run MUST fall back to the base `color`
- **AND** when runs overlap, the last run in the list wins (last-writer-wins)

#### Scenario: Invalid color_run byte range rejected
- **WHEN** an agent submits a TextMarkdownNode with a `color_run` where `start_byte >= end_byte`
- **THEN** the runtime MUST reject the mutation with `InvalidFieldValue`

#### Scenario: Out-of-range color_run byte offset rejected
- **WHEN** an agent submits a TextMarkdownNode with a `color_run` where `end_byte > content.len()`
- **THEN** the runtime MUST reject the mutation with `InvalidFieldValue`

#### Scenario: Non-UTF-8-boundary color_run offset rejected
- **WHEN** an agent submits a TextMarkdownNode with a `color_run` where `start_byte` or `end_byte` is not on a UTF-8 character boundary
- **THEN** the runtime MUST reject the mutation with `InvalidFieldValue`

#### Scenario: StaticImageNode with unknown resource
- **WHEN** an agent submits a StaticImageNode referencing a ResourceId not known to the runtime
- **THEN** the runtime MUST reject the mutation with `ResourceNotFound`

#### Scenario: HitRegionNode local state
- **WHEN** a pointer hovers over a HitRegionNode with accepts_pointer = true
- **THEN** the runtime MUST update the node's local hovered state for immediate visual feedback without waiting for agent acknowledgement

---

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

---

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

---

### Requirement: Transaction Concurrency
The scene graph MUST have a single writer lock. One batch commits at a time. Batches from the same agent MUST commit in submission order. Each batch's mutations MUST be contiguous (no interleaving within a batch). A monotonically increasing u64 sequence_number MUST be assigned to each commit and is the canonical ordering token for scene history.
Source: RFC 0001 §3.5
Scope: v1-mandatory

#### Scenario: Sequential batch ordering
- **WHEN** agent A submits batches B1 and B2 in order
- **THEN** the runtime MUST commit B1 before B2, and B1.sequence_number < B2.sequence_number

---

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

---

### Requirement: Zone Publishing
Agents MUST publish to zones via PublishToZoneMutation, specifying a zone_name (type name), content, publish_token, optional expires_at_wall_us, optional publish_key (for MergeByKey zones), and optional content_classification. The runtime MUST resolve zone_name to the ZoneInstance for the agent's active tab. Publication MUST require `publish_zone:<zone_name>` capability (RFC 0006 §6.3 canonical wire-format name) and a valid ZonePublishToken. ClearZone MUST clear all publications by the agent in the specified zone.
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

---

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

---

### Requirement: Zone Media Types V1
The runtime MUST support five mandatory zone media types in v1: StreamText, ShortTextWithIcon, KeyValuePairs, StaticImage, and SolidColor. Support for VideoSurfaceRef is deferred to the post-v1 media layer. Zone publish content MUST match one of the zone type's accepted_media_types.
Source: RFC 0001 §2.5
Scope: v1-mandatory

#### Scenario: Media type mismatch
- **WHEN** an agent publishes a KeyValuePairs payload to a zone that only accepts StreamText
- **THEN** the runtime MUST reject with `ZoneMediaTypeMismatch`

---

### Requirement: Zone Layer Attachment
Each zone instance MUST declare a layer_attachment: Background (behind all agent tiles), Content (within the content layer z-order space at z_order >= ZONE_TILE_Z_MIN = 0x8000_0000), or Chrome (above all agent content, rendered by runtime). Content-layer zone tiles MUST participate in the same z-order traversal as agent tiles but in the reserved upper band.
Source: RFC 0001 §2.5
Scope: v1-mandatory

#### Scenario: Content zone tile z-order
- **WHEN** a Content-layer zone instance is active
- **THEN** the runtime MUST create a zone tile with z_order >= 0x8000_0000 that appears above all agent-owned tiles in the content layer

---

### Requirement: Widget Registry in Scene Graph
The scene graph MUST contain a WidgetRegistry alongside the existing ZoneRegistry. The WidgetRegistry MUST store: widget type definitions (WidgetDefinition), widget instances per tab (WidgetInstance), active publications (WidgetPublishRecord), and resolved occupancy (WidgetOccupancy). WidgetRegistry MUST be accessible from the SceneGraph root. The WidgetRegistry parallels ZoneRegistry in structure: runtime-owned, loaded from configuration at startup, not agent-created in v1.
Source: RFC 0001 §2.5 (zone registry pattern), widget-system proposal
Scope: v1-mandatory

#### Scenario: Widget registry exists in scene graph
- **WHEN** the runtime starts with widget configuration
- **THEN** the SceneGraph root MUST contain a WidgetRegistry alongside the ZoneRegistry
- **AND** the WidgetRegistry MUST be queryable for widget type definitions and widget instances

#### Scenario: Widget registry contains types and instances
- **WHEN** the runtime loads a widget bundle defining widget type "gauge" and config declares a gauge instance on tab "main"
- **THEN** the WidgetRegistry MUST contain a WidgetDefinition with id "gauge"
- **AND** the WidgetRegistry MUST contain a WidgetInstance binding "gauge" to the "main" tab

---

### Requirement: Widget Type Definition
WidgetDefinition MUST contain: id (string, kebab-case), name (string, human-readable), description (string), parameter_schema (Vec of WidgetParameterDeclaration), layers (Vec of WidgetSvgLayer with parameter bindings), default_geometry_policy (GeometryPolicy), default_rendering_policy (RenderingPolicy), default_contention_policy (ContentionPolicy), and ephemeral (bool). WidgetParameterDeclaration MUST contain: name (string), param_type (WidgetParamType enum: F32, String, Color, Enum), default_value (WidgetParameterValue), constraints (optional WidgetParamConstraints). Widget type ids MUST be unique within the WidgetRegistry. The id field MUST match the pattern `[a-z][a-z0-9-]*` (kebab-case, starting with lowercase letter).
Source: RFC 0001 §2.5 (zone type pattern), widget-system proposal
Scope: v1-mandatory

#### Scenario: Widget definition with full schema
- **WHEN** a widget bundle is loaded containing a "gauge" widget definition with two parameters (value: F32 with range [0.0, 1.0], label: String) and two SVG layers
- **THEN** the WidgetDefinition MUST be stored in the WidgetRegistry with id "gauge", the full parameter_schema with both declarations, both WidgetSvgLayer entries, and all default policies

#### Scenario: Parameter declaration validated
- **WHEN** a widget bundle declares a WidgetParameterDeclaration with param_type F32 and default_value String("hello")
- **THEN** the runtime MUST reject the widget bundle with a configuration error indicating the default_value type does not match param_type

#### Scenario: Widget type id uniqueness
- **WHEN** two widget bundles both declare a widget type with id "gauge"
- **THEN** the runtime MUST reject the configuration with a duplicate widget type error

#### Scenario: Widget type id format validation
- **WHEN** a widget bundle declares a widget type with id "My Gauge!"
- **THEN** the runtime MUST reject the widget bundle with an invalid id format error

---

### Requirement: Widget Instance in Scene Graph
WidgetInstance MUST contain: widget_type_name (string, references WidgetDefinition.id), tab_id (SceneId), geometry_override (Option<GeometryPolicy>), contention_override (Option<ContentionPolicy>), and current_params (HashMap<String, WidgetParameterValue>). Widget instances MUST be unique per (widget_type_name, tab_id) pair unless explicit instance_id disambiguation is provided. The widget_type_name MUST reference an existing WidgetDefinition.id in the WidgetRegistry. Widget instances are static in v1 (loaded from config, not agent-created). The instance_name field MUST be computed as: the explicit `instance_id` if provided in configuration, otherwise the `widget_type_name`. instance_name MUST be unique within a tab and serves as the addressing key for widget publish operations.
Source: RFC 0001 §2.5 (zone instance pattern), widget-system proposal
Scope: v1-mandatory

#### Scenario: Widget instance bound to tab
- **WHEN** configuration declares a widget instance of type "gauge" on tab "main"
- **THEN** the WidgetRegistry MUST contain a WidgetInstance with widget_type_name "gauge" and tab_id matching the "main" tab's SceneId
- **AND** current_params MUST be initialized to the WidgetDefinition's default parameter values

#### Scenario: Geometry override applied
- **WHEN** configuration declares a widget instance with a geometry_override specifying explicit bounds
- **THEN** the WidgetInstance MUST use the geometry_override instead of the WidgetDefinition's default_geometry_policy for layout computation

#### Scenario: Duplicate instance without disambiguation
- **WHEN** configuration declares two widget instances of type "gauge" on the same tab without distinct instance_id values
- **THEN** the runtime MUST reject the configuration with a duplicate widget instance error

#### Scenario: Widget type reference validation
- **WHEN** configuration declares a widget instance referencing widget_type_name "nonexistent"
- **THEN** the runtime MUST reject the configuration with a widget type not found error

---

### Requirement: Widget Publish Record
WidgetPublishRecord MUST contain: widget_name (string), publisher_namespace (string), params (HashMap<String, WidgetParameterValue>), published_at_wall_us (u64), merge_key (Option<String>), expires_at_wall_us (Option<u64>), transition_ms (u32). This parallels ZonePublishRecord. The widget_name MUST reference an existing WidgetDefinition.id. The params keys MUST be a subset of the widget's parameter_schema names. Each param value MUST match its declared type.
Source: RFC 0001 §2.5 (zone publish record pattern), widget-system proposal
Scope: v1-mandatory

#### Scenario: Publish record stored on publish
- **WHEN** an agent publishes params {value: 0.8, label: "CPU"} to widget "gauge" with transition_ms 300
- **THEN** a WidgetPublishRecord MUST be created with the agent's namespace, the supplied params, the current wall-clock timestamp as published_at_wall_us, and transition_ms 300

#### Scenario: TTL expiry
- **WHEN** a WidgetPublishRecord has expires_at_wall_us set to T and the current wall-clock time exceeds T
- **THEN** the runtime MUST remove the expired WidgetPublishRecord from the widget's active publications and recompute widget occupancy

#### Scenario: Unknown parameter rejected
- **WHEN** an agent publishes params {unknown_param: 42} to widget "gauge" and "unknown_param" is not in the gauge's parameter_schema
- **THEN** the runtime MUST reject the publication with a parameter validation error

#### Scenario: Parameter type mismatch rejected
- **WHEN** an agent publishes params {value: "not a number"} to widget "gauge" where "value" is declared as F32
- **THEN** the runtime MUST reject the publication with a parameter type mismatch error

---

### Requirement: Widget Occupancy
WidgetOccupancy MUST contain: widget_name (string), tab_id (SceneId), active_publications (Vec<WidgetPublishRecord>), occupant_count (u32), effective_params (HashMap<String, WidgetParameterValue>) -- the resolved parameter values after contention policy application. WidgetOccupancy is the resolved render state: the compositor reads effective_params to determine current visual property values. When no publications are active, effective_params MUST fall back to the WidgetDefinition's default parameter values.
Source: RFC 0001 §2.5 (zone occupancy pattern), widget-system proposal
Scope: v1-mandatory

#### Scenario: Occupancy reflects latest-wins
- **WHEN** widget "gauge" has contention policy LatestWins and agent A publishes {value: 0.3} followed by agent B publishing {value: 0.8}
- **THEN** the WidgetOccupancy effective_params MUST contain {value: 0.8} from agent B's publication
- **AND** occupant_count MUST be 1

#### Scenario: Occupancy reflects merge-by-key
- **WHEN** widget "gauge" has contention policy MergeByKey and agent A publishes with merge_key "cpu" params {value: 0.6} and agent B publishes with merge_key "memory" params {value: 0.9}
- **THEN** the WidgetOccupancy active_publications MUST contain both records
- **AND** occupant_count MUST be 2
- **AND** effective_params MUST reflect the merged result of both publications

#### Scenario: Occupancy falls back to defaults
- **WHEN** widget "gauge" has no active publications (all expired or cleared)
- **THEN** the WidgetOccupancy effective_params MUST equal the WidgetDefinition's default parameter values
- **AND** occupant_count MUST be 0

---

### Requirement: WidgetParameterValue Type
WidgetParameterValue MUST be an enum with variants: F32(f32), String(String), Color(Rgba), Enum(String). The value MUST match the parameter's declared type in the widget's parameter_schema. F32 values MUST be finite (no NaN, no infinity). String values MUST be at most 1024 UTF-8 bytes. Enum values MUST match one of the allowed values declared in the parameter's WidgetParamConstraints.
Source: widget-system proposal
Scope: v1-mandatory

#### Scenario: F32 value matches F32 parameter
- **WHEN** an agent publishes WidgetParameterValue::F32(0.75) for a parameter declared as param_type F32
- **THEN** the runtime MUST accept the value

#### Scenario: Type mismatch rejected
- **WHEN** an agent publishes WidgetParameterValue::String("hello") for a parameter declared as param_type F32
- **THEN** the runtime MUST reject the publication with a parameter type mismatch error

#### Scenario: Non-finite F32 rejected
- **WHEN** an agent publishes WidgetParameterValue::F32(f32::NAN) for a parameter declared as param_type F32
- **THEN** the runtime MUST reject the value with an invalid parameter value error

#### Scenario: Enum value validated against constraints
- **WHEN** a parameter declares param_type Enum with allowed values ["low", "medium", "high"] and an agent publishes WidgetParameterValue::Enum("critical")
- **THEN** the runtime MUST reject the value with an enum value not in allowed set error

---

### Requirement: Hit-Testing Contract
Hit-testing MUST map a 2D point to the deepest interactive element. Traversal order: (1) Chrome drag-handle hit regions and Zone hit regions (runtime-managed zone affordances such as dismiss/action buttons), (2) Chrome layer tiles, (3) Content layer tiles sorted by (z_order DESC), (4) within each tile, sorted by (tree_order DESC) of nodes. Passthrough tiles MUST be skipped. HitRegionNodes MUST be the interactive primitive. The hit-test MUST return one of: NodeHit, TileHit, Passthrough, Chrome, or ZoneInteraction.
Source: RFC 0001 §5.1, §5.2
Scope: v1-mandatory

#### Scenario: Chrome layer always wins
- **WHEN** a pointer event lands on a chrome element (tab bar, system indicator)
- **THEN** the hit-test MUST return Chrome regardless of underlying tiles

#### Scenario: Highest-z tile hit
- **WHEN** two non-passthrough tiles overlap and the pointer lands in the overlap region
- **THEN** the hit-test MUST return a hit for the tile with the higher z_order

#### Scenario: Passthrough tile skipped
- **WHEN** a pointer event lands on a tile with input_mode = Passthrough
- **THEN** the hit-test MUST skip that tile and test the next tile in z-order

---

### Requirement: Hit-Test Performance
Hit-testing MUST complete in < 100us for a single point query against 50 tiles, measured as pure Rust execution with no GPU involvement.
Source: RFC 0001 §5.1, §10
Scope: v1-mandatory

#### Scenario: Hit-test latency benchmark
- **WHEN** a hit-test query is executed against a scene with 50 tiles
- **THEN** the query MUST complete in less than 100us on reference hardware (single core at 3GHz equivalent)

---

### Requirement: Scene Snapshot Serialization
A full scene snapshot MUST be a complete, deterministic serialization of the scene graph at a specific sequence number. It MUST include: sequence, snapshot_wall_us (UTC wall-clock timestamp of the snapshot), snapshot_mono_us (monotonic timestamp of the snapshot), all tabs (ordered by display_order), all tiles, all nodes, zone_registry (types, active publishes; zone_instances is intentionally empty in v1 because instance binding is implicit — one per tab per zone type — and is not stored explicitly), widget_registry (types, instances, active publishes), active_tab, and a BLAKE3 checksum. Fields MUST be serialized in deterministic order (BTreeMap, not HashMap). Serialization MUST complete in < 1ms for a 100-tile scene with 1000 nodes total.
Source: RFC 0001 §4.1, §10
Scope: v1-mandatory

#### Scenario: Snapshot determinism
- **WHEN** two snapshots are taken of identical scene state
- **THEN** both MUST produce identical serialization bytes and identical BLAKE3 checksums

#### Scenario: Snapshot performance
- **WHEN** a snapshot is serialized for a scene with 100 tiles and 1000 nodes
- **THEN** serialization MUST complete in < 1ms (protobuf encode, single core, reference hardware)

#### Scenario: Snapshot includes widget registry
- **WHEN** a scene snapshot is serialized and the scene contains widget definitions and active widget publications
- **THEN** the snapshot MUST include a widget_registry field containing all WidgetDefinition entries, all WidgetInstance entries, and all active WidgetPublishRecord entries
- **AND** widget_registry data MUST be serialized in deterministic order (BTreeMap)

#### Scenario: Snapshot determinism with widgets
- **WHEN** two snapshots are taken of identical scene state including widget registry state
- **THEN** both MUST produce identical serialization bytes and identical BLAKE3 checksums, including the widget_registry portion

---

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

---

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

---

### Requirement: Batch Rejection Response
When a batch is rejected, the runtime MUST return a BatchRejected response containing the batch_id and a structured error. Validation errors MUST include: mutation_index (0-based), mutation_type, ValidationErrorCode, human-readable message, machine-readable context (JSON), and optional correction_hint (JSON). All error codes MUST be stable across minor versions.
Source: RFC 0001 §3.4
Scope: v1-mandatory

#### Scenario: Structured validation error
- **WHEN** a mutation fails bounds validation
- **THEN** the rejection MUST include the mutation_index, code=BoundsOutOfRange, a human-readable message, and a context JSON object identifying the field, value, and constraint

---

### Requirement: Ephemeral Scene State
The scene graph (tabs, tiles, nodes), active leases, live zone publishes, gRPC sessions, hit-region local state, and performance telemetry MUST be ephemeral (lost on restart). After restart, agents MUST re-establish sessions and re-create scene content. Resources (images, fonts) are also ephemeral in v1: they are content-addressed and deduplicated by BLAKE3 hash, but all stored resources are lost on restart. Persistence of resources is deferred to post-v1 (see Resource Store RFC 0011 §4.1).
Source: RFC 0001 §6; RFC 0011 §4.1
Scope: v1-mandatory

#### Scenario: Scene lost on restart
- **WHEN** the runtime process restarts
- **THEN** the scene graph MUST be empty; agents MUST reconnect and re-create their tiles and nodes

---

### Requirement: V1 Reconnect via Full Snapshot
In v1, when an agent reconnects, the runtime SHALL always send a full SceneSnapshot regardless of how recently the agent was connected. The agent MUST discard its prior scene state, apply the snapshot, and resume from the snapshot's sequence number.
Source: RFC 0001 §4.2
Scope: v1-mandatory

#### Scenario: Reconnecting agent receives snapshot
- **WHEN** an agent reconnects after a disconnection
- **THEN** the runtime MUST send a full SceneSnapshot; incremental diff is not available in v1

---

### Requirement: Incremental Diff (Deferred)
Incremental diff (WAL-backed delta sync for reconnecting agents), including SceneDiff, DiffOp, and branching reconnect logic, is deferred to post-v1. V1 ships snapshot-only reconnection. Implementations SHALL provide a full snapshot instead of incremental diff when requested in v1.
Source: RFC 0001 §4.2
Scope: post-v1

#### Scenario: Incremental diff not available
- **WHEN** an agent requests incremental diff in v1
- **THEN** the runtime MUST provide a full snapshot instead

---

### Requirement: VideoSurfaceRef and WebRtcRequired (Deferred)
The VideoSurfaceRef zone media type and the WebRtcRequired transport constraint are deferred to the post-v1 media layer. Implementations SHALL treat VideoSurfaceRef as unsupported in v1.
Narrow exception: the accepted `openspec/changes/windows-media-ingress-exemplar/` change may enable `VideoSurfaceRef` only for the explicitly configured Windows `media-pip` zone. Existing default zones such as `pip` and `ambient-background` MUST NOT implicitly accept `VideoSurfaceRef`.
Source: RFC 0001 §2.5
Scope: post-v1

#### Scenario: VideoSurfaceRef not supported
- **WHEN** a zone type with VideoSurfaceRef is configured in v1
- **THEN** the runtime MAY accept the configuration but MUST NOT attempt to render video surface content

---

### Requirement: Zone Occupancy Query API (Deferred)
The ZoneOccupancy effective_geometry field and the occupancy query API are deferred to post-v1. In v1, the runtime maintains occupancy state internally; snapshots include active publications per instance but do not expose effective_geometry. Implementations SHALL omit effective_geometry from v1 snapshots.
Source: RFC 0001 §2.5
Scope: post-v1

#### Scenario: Effective geometry not in v1 snapshot
- **WHEN** a scene snapshot is serialized in v1
- **THEN** the snapshot MUST include active zone publications but MUST NOT include effective_geometry data
