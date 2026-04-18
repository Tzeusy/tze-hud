# scene-graph Specification

## Purpose
TBD - created by archiving change text-stream-portals. Update Purpose after archive.
## Requirements
### Requirement: Text Stream Portal Phase-0 Uses Raw Tiles

The `text-stream-portals` phase-0 pilot SHALL use agent-owned content-layer raw tiles with existing V1 node types. The pilot MUST NOT require a new scene node type before the capability is proven.

#### Scenario: portal pilot tile stays below runtime-managed bands

- **WHEN** a resident portal pilot creates its surface as a raw tile
- **THEN** the tile SHALL use the normal agent-owned z-order band and remain below zone-reserved and widget-reserved runtime-managed tiles

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

### Requirement: Scene Snapshot Serialization
A full scene snapshot MUST be a complete, deterministic serialization of the scene graph at a specific sequence number. It MUST include: sequence, snapshot_wall_us (UTC wall-clock timestamp of the snapshot), snapshot_mono_us (monotonic timestamp of the snapshot), all tabs (ordered by display_order), all tiles, all nodes, zone_registry (types, instances, active publishes), widget_registry (types, instances, active publishes), active_tab, and a BLAKE3 checksum. Fields MUST be serialized in deterministic order (BTreeMap, not HashMap). Serialization MUST complete in < 1ms for a 100-tile scene with 1000 nodes total.
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
