# Widget System Specification

Domain: PRESENCE
Source: Proposal (widget-system change)
Depends on: scene-graph, session-protocol, configuration, resource-store

---

## ADDED Requirements

### Requirement: Widget Type Ontology
The widget system MUST follow a four-level ontology paralleling zones: Widget Type (schema + visual assets defining a parameterized visual template), Widget Instance (a widget type bound to a specific tab with geometry and layer attachment), Widget Publication (a set of parameter values submitted by an agent for a widget instance), and Widget Occupancy (the resolved render state after contention resolution). Each level MUST be independently identifiable and queryable. Widget Types SHALL be immutable after registration. Widget Instances SHALL reference exactly one Widget Type. Widget Publications SHALL reference exactly one Widget Instance. Widget Occupancy SHALL be derived from the set of active publications on an instance after applying the instance's contention policy.
Scope: v1-mandatory

#### Scenario: Widget type loaded from bundle
- **WHEN** the runtime loads a valid asset bundle containing a `widget.toml` manifest and SVG layers
- **THEN** the runtime MUST register a Widget Type with the declared name, parameter schema, and visual assets

#### Scenario: Widget instance created per tab
- **WHEN** configuration declares a widget instance of type "gauge" on tab "dashboard"
- **THEN** the runtime MUST create a Widget Instance binding the "gauge" Widget Type to the "dashboard" tab with the configured geometry and layer attachment

#### Scenario: Publication updates parameters
- **WHEN** an agent publishes parameter values `{ "level": 0.8, "label": "CPU" }` to widget "gauge"
- **THEN** the runtime MUST create a Widget Publication recording those parameter values, validated against the Widget Type's parameter schema

#### Scenario: Occupancy reflects current render
- **WHEN** two agents publish to a LatestWins widget and the second publication wins
- **THEN** the Widget Occupancy MUST reflect the parameter values from the second publication as the current render state

---

### Requirement: Widget Parameter Schema
Each Widget Type MUST declare a parameter schema: a list of named parameters each with type (f32, string, color, enum), default value, and optional constraints (min/max for f32, max_length for string, allowed_values for enum). The runtime MUST validate all published parameter values against the schema before accepting a publication. Unknown parameter names MUST be rejected with error code WIDGET_UNKNOWN_PARAMETER. Type mismatches MUST be rejected with error code WIDGET_PARAMETER_TYPE_MISMATCH. f32 values outside [min, max] MUST be clamped to the nearest bound (not rejected). NaN and infinity checks MUST be evaluated BEFORE range clamping. A NaN value MUST NOT be silently clamped to a bound. Parameters not included in a publication MUST retain their previous value, or the schema default if no prior publication exists.
Scope: v1-mandatory

#### Scenario: Valid publish accepted
- **WHEN** an agent publishes `{ "level": 0.75 }` to a widget whose schema declares `level` as f32 with min=0.0, max=1.0
- **THEN** the runtime MUST accept the publication and apply the parameter value

#### Scenario: Unknown parameter rejected
- **WHEN** an agent publishes `{ "bogus": 42 }` to a widget whose schema does not declare a parameter named "bogus"
- **THEN** the runtime MUST reject the publication with error code WIDGET_UNKNOWN_PARAMETER

#### Scenario: Type mismatch rejected
- **WHEN** an agent publishes `{ "level": "high" }` to a widget whose schema declares `level` as f32
- **THEN** the runtime MUST reject the publication with error code WIDGET_PARAMETER_TYPE_MISMATCH

#### Scenario: f32 value clamped to range
- **WHEN** an agent publishes `{ "level": 1.5 }` to a widget whose schema declares `level` as f32 with min=0.0, max=1.0
- **THEN** the runtime MUST clamp the value to 1.0 and accept the publication

---

### Requirement: Widget Parameter Types
V1 SHALL support exactly four parameter types: f32 (IEEE 754 32-bit float, range-clamped to [min, max]), string (UTF-8, max 1024 bytes by default, overridable by max_length constraint), color (RGBA as 4x u8, range [0, 255] per component), and enum (string value from a declared allowed_values set). NaN and infinity f32 values MUST be rejected with error code WIDGET_PARAMETER_INVALID_VALUE. String values exceeding max_length MUST be rejected with WIDGET_PARAMETER_INVALID_VALUE. Color components outside [0, 255] MUST be rejected with WIDGET_PARAMETER_INVALID_VALUE. Enum values not in allowed_values MUST be rejected with WIDGET_PARAMETER_INVALID_VALUE.
Scope: v1-mandatory

#### Scenario: f32 parameter validated
- **WHEN** an agent publishes an f32 parameter with value 0.5 and the schema declares min=0.0, max=1.0
- **THEN** the runtime MUST accept the value without modification

#### Scenario: f32 NaN rejected
- **WHEN** an agent publishes an f32 parameter with value NaN
- **THEN** the runtime MUST reject the publication with error code WIDGET_PARAMETER_INVALID_VALUE

#### Scenario: f32 infinity rejected
- **WHEN** an agent publishes an f32 parameter with value positive infinity
- **THEN** the runtime MUST reject the publication with error code WIDGET_PARAMETER_INVALID_VALUE

#### Scenario: String parameter validated
- **WHEN** an agent publishes a string parameter with a valid UTF-8 value of 500 bytes and max_length is 1024
- **THEN** the runtime MUST accept the value

#### Scenario: String exceeds max_length
- **WHEN** an agent publishes a string parameter with a 2000-byte UTF-8 value and max_length is 1024
- **THEN** the runtime MUST reject the publication with error code WIDGET_PARAMETER_INVALID_VALUE

#### Scenario: Color parameter validated
- **WHEN** an agent publishes a color parameter with value [255, 128, 0, 255]
- **THEN** the runtime MUST accept the value as a valid RGBA color

#### Scenario: Enum parameter validated
- **WHEN** an agent publishes an enum parameter with value "warning" and allowed_values is ["info", "warning", "error"]
- **THEN** the runtime MUST accept the value

#### Scenario: Enum value not in allowed set
- **WHEN** an agent publishes an enum parameter with value "critical" and allowed_values is ["info", "warning", "error"]
- **THEN** the runtime MUST reject the publication with error code WIDGET_PARAMETER_INVALID_VALUE

---

### Requirement: Widget Asset Bundle Format
Widget type definitions MUST be loaded from asset bundles — directories containing a `widget.toml` manifest and one or more SVG files. The manifest MUST declare: `name` (kebab-case, unique across all loaded bundles), `version` (semver string), `description` (human-readable string), `parameter_schema` (array of parameter declarations, each with name, type, default, and optional constraints), `layers` (ordered array of SVG layer references with parameter bindings), and optional `default_geometry` (Rect), `default_rendering_policy`, `default_contention_policy`. The runtime MUST scan all configured bundle directories at startup. The runtime MUST reject bundles with the following error codes: missing `widget.toml` manifest (WIDGET_BUNDLE_NO_MANIFEST), invalid TOML syntax or missing required fields (WIDGET_BUNDLE_INVALID_MANIFEST), duplicate widget type name across loaded bundles (WIDGET_BUNDLE_DUPLICATE_TYPE), SVG file referenced in layers but not present in the bundle directory (WIDGET_BUNDLE_MISSING_SVG), SVG file that fails to parse (WIDGET_BUNDLE_SVG_PARSE_ERROR). A rejected bundle MUST NOT prevent other valid bundles from loading; the runtime SHALL log the error and continue.
Scope: v1-mandatory

#### Scenario: Valid bundle loaded
- **WHEN** the runtime scans a bundle directory containing a valid `widget.toml` with name "gauge", version "1.0.0", parameter schema, layers referencing existing SVG files, and all SVGs parse successfully
- **THEN** the runtime MUST register a Widget Type named "gauge" with the declared schema and visual assets

#### Scenario: Missing manifest rejected
- **WHEN** the runtime scans a bundle directory that contains SVG files but no `widget.toml`
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_NO_MANIFEST and log the error

#### Scenario: Invalid manifest rejected
- **WHEN** the runtime scans a bundle directory with a `widget.toml` that has invalid TOML syntax
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_INVALID_MANIFEST and log the error

#### Scenario: Duplicate type name rejected
- **WHEN** two bundle directories both declare widget type name "gauge"
- **THEN** the runtime MUST reject the second bundle with error code WIDGET_BUNDLE_DUPLICATE_TYPE and log the error

#### Scenario: Missing SVG file rejected
- **WHEN** a `widget.toml` references layer file "fill.svg" but the file does not exist in the bundle directory
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_MISSING_SVG and log the error

#### Scenario: SVG parse failure rejected
- **WHEN** a bundle contains a file "fill.svg" that is not valid SVG
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_SVG_PARSE_ERROR and log the error

---

### Requirement: SVG Layer Parameter Bindings
Each SVG layer in a widget bundle MUST declare parameter bindings: mappings from parameter names to SVG element attributes. A binding MUST specify: `param` (parameter name from the widget's parameter schema), `target_element` (SVG element ID within the layer's SVG file), `target_attribute` (SVG attribute name — e.g., `height`, `fill`, `opacity`, `transform`, `d`), and `mapping` (how the parameter value maps to the attribute value). The runtime MUST support three mapping types: `linear` (for f32 parameters, maps the parameter's [min, max] range to an [attr_min, attr_max] attribute value range using linear interpolation), `direct` (parameter value used as-is for string and color parameters — string becomes the attribute value, color becomes an SVG color string), and `discrete` (for enum parameters, maps each enum value to a specific attribute value via a lookup table). The special target_attribute value `text-content` SHALL be supported as a synthetic binding target. When target_attribute is `text-content`, the binding SHALL replace the text node content of the target SVG element (the character data inside a `<text>` or `<tspan>` element), not an XML attribute. This is the only non-attribute binding target in v1. The runtime MUST resolve all bindings at widget type registration time. Bindings referencing a nonexistent parameter name, a nonexistent SVG element ID, or an incompatible mapping type for the parameter type MUST be rejected with error code WIDGET_BINDING_UNRESOLVABLE.
Scope: v1-mandatory

#### Scenario: Linear mapping applied
- **WHEN** a widget has f32 parameter "level" with min=0.0, max=1.0, and a linear binding maps it to SVG element "bar" attribute "height" with attr_min=0, attr_max=200
- **THEN** publishing level=0.5 MUST set the "height" attribute of SVG element "bar" to 100

#### Scenario: Direct mapping applied
- **WHEN** a widget has color parameter "fill_color" with a direct binding to SVG element "bg" attribute "fill"
- **THEN** publishing fill_color=[255, 0, 0, 255] MUST set the "fill" attribute of SVG element "bg" to the corresponding SVG color representation

#### Scenario: Discrete mapping applied
- **WHEN** a widget has enum parameter "severity" with allowed_values ["info", "warning", "error"] and a discrete binding maps "info" to "#00ff00", "warning" to "#ffff00", "error" to "#ff0000" on SVG element "indicator" attribute "fill"
- **THEN** publishing severity="warning" MUST set the "fill" attribute of SVG element "indicator" to "#ffff00"

#### Scenario: Unresolvable binding rejected
- **WHEN** a widget bundle declares a binding referencing parameter "nonexistent" which is not in the parameter schema
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BINDING_UNRESOLVABLE

---

### Requirement: Widget Registry
The runtime MUST maintain a WidgetRegistry parallel to ZoneRegistry. The registry MUST be populated at startup from asset bundles (widget types) and configuration (widget instances). The registry MUST be queryable by agents via `list_widgets` (MCP tool) and via the WidgetRegistrySnapshot included in SceneSnapshot at session establishment. There SHALL be no separate widget registry query RPC in v1. The registry MUST expose: available widget types (name, version, description, parameter schema), widget instances per tab (widget type, geometry, current parameter values), and widget occupancy (current render state per instance). Agents MUST NOT create, modify, or delete widget types at runtime in v1. The registry MUST be read-only from the agent perspective.
Scope: v1-mandatory

#### Scenario: Registry populated at startup
- **WHEN** the runtime starts with two configured widget bundles ("gauge" and "progress-bar") and configuration declaring instances on tabs
- **THEN** the WidgetRegistry MUST contain both widget types and all declared widget instances

#### Scenario: Agent queries registry via MCP
- **WHEN** an agent calls the `list_widgets` MCP tool
- **THEN** the runtime MUST return all registered widget types with their names, descriptions, and parameter schemas, plus all widget instances with their tab bindings and current parameter values

#### Scenario: Widget registry included in scene snapshot
- **WHEN** an agent establishes a session and receives a SceneSnapshot
- **THEN** the SceneSnapshot MUST include a WidgetRegistrySnapshot containing all widget types and instances

#### Scenario: Agent cannot create widget types
- **WHEN** an agent attempts to register a new widget type at runtime
- **THEN** the runtime MUST reject the operation (no mutation or RPC exists for widget type creation in v1)

---

### Requirement: Widget Publishing via MCP
The MCP tool surface MUST include `publish_to_widget` as a guest tool. The tool MUST accept the following parameters: `widget_name` (string, required — identifies the widget type by name), `instance_id` (string, optional — when multiple instances of the same widget type exist on a tab, `instance_id` MUST be provided to disambiguate; when only one instance of the type exists, `instance_id` MAY be omitted), `params` (object mapping parameter names to values, required), `transition_ms` (u32, optional, default 0 — instant application), `namespace` (string, optional, auto-derived from session if omitted), and `ttl_us` (u64, optional, 0 = use widget instance default). The tool MUST validate `params` against the widget's parameter schema before publishing. The tool MUST require the `publish_widget:<widget_name>` capability on the calling session. The tool MUST return immediately for ephemeral widgets (fire-and-forget) and return an acknowledgement for durable widgets. The `widget_name` parameter identifies a widget instance by its configured instance name. If the instance was declared with an explicit `instance_id`, that id is the `widget_name`. If no `instance_id` was set, the widget type name is used as the instance name (valid only when a single instance of that type exists on the tab). The MCP tool surface MUST also include `list_widgets` as a guest tool that returns all registered widget types and their instances with current parameter values.
Scope: v1-mandatory

#### Scenario: Successful publish via MCP
- **WHEN** an agent with `publish_widget:gauge` capability calls `publish_to_widget` with widget_name="gauge" and params={"level": 0.8}
- **THEN** the runtime MUST validate the parameters, publish the values to the gauge widget on the agent's active tab, and return success

#### Scenario: Schema validation failure via MCP
- **WHEN** an agent calls `publish_to_widget` with params={"nonexistent": 42} for a widget that does not declare parameter "nonexistent"
- **THEN** the tool MUST return an error with code WIDGET_UNKNOWN_PARAMETER

#### Scenario: Missing capability via MCP
- **WHEN** an agent without `publish_widget:gauge` capability calls `publish_to_widget` with widget_name="gauge"
- **THEN** the tool MUST return an error indicating insufficient capabilities

#### Scenario: Publish to specific instance when multiple exist
- **WHEN** a tab has two instances of widget type "gauge" with instance_ids "cpu-gauge" and "mem-gauge", and an agent calls `publish_to_widget` with widget_name="gauge" and instance_id="cpu-gauge"
- **THEN** the runtime MUST publish the parameters to the "cpu-gauge" instance only

#### Scenario: list_widgets returns registry
- **WHEN** an agent calls the `list_widgets` MCP tool
- **THEN** the tool MUST return all registered widget types (name, description, parameter schema) and all widget instances (tab, geometry, current parameters)

---

### Requirement: Widget Publishing via gRPC
WidgetPublish SHALL be a ClientMessage payload at field 35 on the bidirectional session stream. It MUST carry: widget_name (string), instance_id (string), params (repeated WidgetParameterValue — each containing parameter name and typed value), transition_ms (uint32), ttl_us (uint64), and merge_key (string, optional — used only for MergeByKey contention policy). Durable-widget publishes SHALL be transactional: the runtime MUST send a WidgetPublishResult (ServerMessage field 47) containing accepted (bool), widget_name (string), and error details if rejected. Ephemeral-widget publishes SHALL be fire-and-forget: the runtime MUST NOT send a WidgetPublishResult. The `widget_name` parameter identifies a widget instance by its configured instance name. If the instance was declared with an explicit `instance_id`, that id is the `widget_name`. If no `instance_id` was set, the widget type name is used as the instance name (valid only when a single instance of that type exists on the tab). The runtime MUST validate all parameters against the widget's schema before applying the publication.
Scope: v1-mandatory

#### Scenario: Durable widget publish acknowledged
- **WHEN** an agent sends a WidgetPublish for a durable widget with valid parameters on the session stream
- **THEN** the runtime MUST apply the publication and send a WidgetPublishResult with accepted=true

#### Scenario: Ephemeral widget publish fire-and-forget
- **WHEN** an agent sends a WidgetPublish for an ephemeral widget with valid parameters on the session stream
- **THEN** the runtime MUST apply the publication and MUST NOT send a WidgetPublishResult

#### Scenario: Rejected publish returns error
- **WHEN** an agent sends a WidgetPublish with an unknown parameter name
- **THEN** the runtime MUST send a WidgetPublishResult with accepted=false and error code WIDGET_UNKNOWN_PARAMETER

#### Scenario: Publish to specific instance when multiple exist
- **WHEN** a tab has two instances of widget type "gauge" with instance_ids "cpu-gauge" and "mem-gauge", and an agent sends a WidgetPublish with widget_name="gauge" and instance_id="cpu-gauge"
- **THEN** the runtime MUST publish the parameters to the "cpu-gauge" instance only

---

### Requirement: Clear Widget
ClearWidgetMutation SHALL clear all publications by the calling agent in the specified widget instance. It MUST carry: widget_name (string), instance_id (string, optional). After clearing, the widget's effective_params MUST fall back to remaining publications (if any) or to the WidgetDefinition's default parameter values (if none remain). ClearWidget MUST require `publish_widget:<widget_name>` capability (same as publish). ClearWidget is available via gRPC only (no MCP tool in v1).
Scope: v1-mandatory

#### Scenario: Successful clear reverts to defaults
- **WHEN** an agent that is the sole publisher on a widget instance sends a ClearWidgetMutation for that instance
- **THEN** the runtime MUST remove the agent's publication and the widget's effective_params MUST revert to the WidgetDefinition's default parameter values

#### Scenario: Clear with other publishers leaves their publications
- **WHEN** an agent sends a ClearWidgetMutation for a widget instance that has publications from other agents
- **THEN** the runtime MUST remove only the calling agent's publication; the widget's effective_params MUST be derived from the remaining publications

#### Scenario: Clear without capability rejected
- **WHEN** an agent without `publish_widget:gauge` capability sends a ClearWidgetMutation for widget "gauge"
- **THEN** the runtime MUST reject the mutation with a capability error

---

### Requirement: Widget Compositor Rendering
The compositor MUST render widgets through the following pipeline: (1) load SVG layers from the asset bundle at widget type registration, (2) parse each SVG into a retained representation using resvg, (3) on parameter publication, apply parameter bindings to produce a modified SVG tree, (4) rasterize the modified SVG tree to a GPU texture at the widget instance's geometry size, (5) composite the texture into the scene at the widget instance's layer attachment and z-order. Re-rasterization MUST only occur when parameter values change or the widget instance's geometry is resized. Cached GPU textures MUST be reused between frames when parameters are unchanged. Re-rasterization MUST complete in less than 2ms for a 512x512 pixel widget on reference hardware (single core at 3GHz equivalent, integrated GPU).
Scope: v1-mandatory

#### Scenario: Initial render on startup
- **WHEN** a widget instance is created at startup with default parameter values
- **THEN** the compositor MUST rasterize the widget's SVG layers with default parameter bindings applied and composite the resulting texture into the scene

#### Scenario: Parameter update triggers re-rasterize
- **WHEN** an agent publishes new parameter values to a widget
- **THEN** the compositor MUST apply the new bindings to the SVG tree, re-rasterize to a GPU texture, and composite the updated texture in the next frame

#### Scenario: Unchanged parameters reuse cache
- **WHEN** a frame is rendered and no parameters have changed on a widget since the last rasterization
- **THEN** the compositor MUST reuse the cached GPU texture without re-rasterizing

#### Scenario: Re-rasterization performance budget
- **WHEN** a 512x512 pixel widget is re-rasterized after a parameter change
- **THEN** the re-rasterization MUST complete in less than 2ms on reference hardware

---

### Requirement: Widget Parameter Interpolation
When a widget publication includes transition_ms > 0, the compositor MUST interpolate between old and new parameter values over the specified duration. f32 parameters MUST use linear interpolation: value(t) = old + (new - old) * (elapsed_ms / transition_ms), clamped to [0.0, 1.0] normalized time. Color parameters MUST use component-wise linear interpolation in sRGB space (each of R, G, B, A interpolated independently as f32 then rounded to u8). String parameters MUST snap to the new value at t=0 (no interpolation — the new string is applied immediately). Enum parameters MUST snap to the new value at t=0 (no interpolation — the new enum value is applied immediately). During interpolation, the compositor MUST re-rasterize at the display refresh rate (target_fps from the active display profile). When transition_ms = 0, the new values MUST be applied immediately with a single re-rasterize.
Scope: v1-mandatory

#### Scenario: f32 parameter interpolation
- **WHEN** an agent publishes level=1.0 with transition_ms=500 to a widget currently at level=0.0
- **THEN** the compositor MUST render intermediate values (e.g., level=0.5 at 250ms) using linear interpolation over 500ms

#### Scenario: Color parameter interpolation
- **WHEN** an agent publishes fill_color=[255, 0, 0, 255] with transition_ms=300 to a widget currently at fill_color=[0, 0, 255, 255]
- **THEN** the compositor MUST interpolate each RGBA component independently in sRGB space over 300ms

#### Scenario: String parameter snaps immediately
- **WHEN** an agent publishes label="New Label" with transition_ms=1000 to a widget currently at label="Old Label"
- **THEN** the compositor MUST apply "New Label" immediately at the start of the transition (t=0), not interpolate character-by-character

#### Scenario: Enum parameter snaps immediately
- **WHEN** an agent publishes severity="error" with transition_ms=500 to a widget currently at severity="info"
- **THEN** the compositor MUST apply "error" immediately at the start of the transition (t=0)

#### Scenario: Zero transition applies immediately
- **WHEN** an agent publishes parameters with transition_ms=0
- **THEN** the compositor MUST apply the new values immediately with a single re-rasterize in the next frame

---

### Requirement: Widget Contention and Governance
Widgets MUST reuse the same contention policies as zones: LatestWins (most recent publication replaces previous), Stack (publications accumulate with configurable max_depth), MergeByKey (same key replaces, different keys coexist, with configurable max_keys), and Replace (single occupant, new publication evicts current). Widget publishing MUST require the `publish_widget:<widget_name>` capability, paralleling the `publish_zone:<zone_name>` capability pattern. Widget instances MUST attach to compositor layers using the same LayerAttachment model as zones: Background (behind all agent tiles), Content (within the content layer z-order space), or Chrome (above all agent content, rendered by runtime). Widget tiles MUST use z_order >= WIDGET_TILE_Z_MIN (0x9000_0000), which is above ZONE_TILE_Z_MIN (0x8000_0000). This places widget tiles above zone tiles when they overlap spatially. For MergeByKey widgets, each merge_key identifies a separate publication with its own parameter values. The widget's effective_params SHALL be computed by starting from the WidgetDefinition's default parameter values, then applying each active publication's params in merge_key lexicographic order, with later keys overwriting earlier keys for overlapping parameter names. This means MergeByKey on widgets is a parameter-level merge where each publisher can own a subset of parameters via distinct keys.
Scope: v1-mandatory

#### Scenario: LatestWins contention on widget
- **WHEN** two agents publish to the same LatestWins widget instance
- **THEN** only the most recent publication's parameter values MUST be rendered; the previous publication is replaced

#### Scenario: Capability required for widget publish
- **WHEN** an agent without `publish_widget:gauge` capability attempts to publish to widget "gauge"
- **THEN** the runtime MUST reject the publication with a capability error

#### Scenario: Chrome layer attachment
- **WHEN** a widget instance is configured with LayerAttachment::Chrome
- **THEN** the widget's tile MUST be rendered above all agent-owned content, in the chrome layer

#### Scenario: MergeByKey merges parameters from multiple publishers
- **WHEN** two agents publish to a MergeByKey widget with merge_keys "alpha" (params: {"level": 0.5}) and "beta" (params: {"label": "CPU"}), and the widget's defaults are {"level": 0.0, "label": "default"}
- **THEN** the widget's effective_params MUST be computed as defaults merged with "alpha" then "beta" in lexicographic order, yielding {"level": 0.5, "label": "CPU"}

---

### Requirement: Widget Instance Lifecycle
Widget instances MUST be created at startup from configuration (static declaration, paralleling zone instances in v1). Each widget instance MUST be bound to exactly one tab. Multiple instances of the same widget type MAY exist on different tabs or on the same tab with different geometry. Widget instances MUST be included in SceneSnapshot serialization alongside zone instances. Widget instance creation, modification, and deletion by agents MUST NOT be supported in v1; all widget instances are runtime-managed.
Scope: v1-mandatory

#### Scenario: Static instance from configuration
- **WHEN** configuration declares a widget instance of type "progress-bar" on tab "build" with geometry {x: 10, y: 400, width: 300, height: 20}
- **THEN** the runtime MUST create the widget instance at startup with the specified type, tab binding, and geometry

#### Scenario: Widget instance in scene snapshot
- **WHEN** a scene snapshot is serialized
- **THEN** the snapshot MUST include all widget instances with their type, tab, geometry, layer attachment, and current parameter values

#### Scenario: Agent cannot create widget instance
- **WHEN** an agent attempts to create a new widget instance at runtime
- **THEN** the runtime MUST reject the operation (no mutation or RPC exists for widget instance creation in v1)

---

### Requirement: Built-in Widget Types
V1 MUST NOT ship built-in widget types. Unlike zones (which define six built-in types), all widget types MUST come from user-authored asset bundles loaded at startup. The runtime MUST function correctly with zero widget bundles configured — in this case, the WidgetRegistry SHALL be empty, `list_widgets` SHALL return an empty list, and `publish_to_widget` calls SHALL be rejected with WIDGET_TYPE_NOT_FOUND.
Scope: v1-mandatory

#### Scenario: No widgets configured
- **WHEN** the runtime starts with no widget bundle directories configured
- **THEN** the WidgetRegistry MUST be empty and `list_widgets` MUST return an empty result

#### Scenario: Runtime starts without widget bundles
- **WHEN** the runtime starts with widget bundle directories configured but all directories are empty or contain only invalid bundles
- **THEN** the runtime MUST start successfully with an empty WidgetRegistry and log warnings for each rejected bundle

---

### Requirement: Widget Input Mode
Widget tiles MUST default to input_mode = Passthrough. Widget tiles SHALL NOT contain interactive nodes (HitRegionNode) in v1. Input events landing on a widget tile's geometry MUST pass through to the next tile in z-order. This makes widgets display-only visual indicators that do not capture input.
Scope: v1-mandatory

#### Scenario: Pointer passes through widget tile
- **WHEN** a pointer event lands on the geometry of a widget tile that overlaps an agent-owned content tile below it in z-order
- **THEN** the input system MUST pass the event through the widget tile to the content tile beneath it

#### Scenario: Widget tile has no hit regions
- **WHEN** a widget tile is rendered in the scene graph
- **THEN** the tile MUST NOT contain any HitRegionNode children and MUST have input_mode set to Passthrough
