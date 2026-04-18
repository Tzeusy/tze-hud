# Session Protocol — Widget System Delta

Capability: session-protocol
Change: widget-system
Type: delta (MODIFIED + ADDED requirements)

---

## MODIFIED Requirements

### Requirement: Proto File Layout

The v1 protobuf definitions SHALL be organized into exactly three files under `crates/tze_hud_protocol/proto/`:

**`types.proto`** (`package tze_hud.protocol.v1`) — Geometry primitives, scene node types, mutation types, zone types, and widget types. SHALL contain (additions marked with [NEW]):
- Geometry: `Rect`, `Rgba`
- Node types: `NodeProto`, `SolidColorNodeProto`, `TextMarkdownNodeProto`, `HitRegionNodeProto`, `StaticImageNodeProto`, `ImageFitModeProto`
- Mutation types: `MutationProto`, `CreateTileMutation`, `SetTileRootMutation`
- Zone types: `ZoneContent`, `ZonePublishToken`, `ZoneDefinitionProto`, `ZoneRegistrySnapshotProto`, `ZonePublishRecordProto`, `NotificationPayload`, `StatusBarPayload`, `GeometryPolicyProto`, `RelativeGeometryPolicy`, `EdgeAnchoredGeometryPolicy`, `RenderingPolicyProto`, `DisplayEdge`, `TextAlignProto`, `ContentionPolicyProto`
- Zone mutation types: `PublishToZoneMutation`, `ClearZoneMutation`
- Zone query types: `ZoneRegistryRequest`, `ZoneRegistryResponse`
- [NEW] Widget types: `WidgetDefinitionProto`, `WidgetInstanceProto`, `WidgetParameterSchemaProto`, `WidgetParameterDeclarationProto`, `WidgetParamTypeProto`, `WidgetParameterValueProto`, `WidgetParamConstraintsProto`, `WidgetSvgLayerProto`, `WidgetBindingProto`, `WidgetBindingMappingProto`, `WidgetRegistrySnapshotProto`, `WidgetPublishRecordProto`, `WidgetOccupancyProto`
- [NEW] Widget mutation types: `PublishToWidgetMutation`, `ClearWidgetMutation`
- [NEW] Widget query types: `WidgetRegistryRequest`, `WidgetRegistryResponse`

**`events.proto`** — unchanged.

**`session.proto`** (`package tze_hud.protocol.v1.session`) — unchanged description plus:
- [NEW] Widget publishing: `WidgetPublish`, `WidgetPublishResult`

Source: proto file layout is part of the wire protocol contract
Scope: v1-mandatory

#### Scenario: types.proto contains widget types
- **WHEN** building the v1 protobuf package
- **THEN** `types.proto` SHALL define `WidgetDefinitionProto`, `WidgetInstanceProto`, `WidgetParameterSchemaProto`, `WidgetParameterDeclarationProto`, `WidgetParamTypeProto`, `WidgetParameterValueProto`, `WidgetParamConstraintsProto`, `WidgetSvgLayerProto`, `WidgetBindingProto`, `WidgetBindingMappingProto`, `WidgetRegistrySnapshotProto`, `WidgetPublishRecordProto`, `WidgetOccupancyProto`, `PublishToWidgetMutation`, `ClearWidgetMutation`, `WidgetRegistryRequest`, and `WidgetRegistryResponse` alongside existing zone and geometry types

#### Scenario: session.proto contains widget publish messages
- **WHEN** building the v1 protobuf package
- **THEN** `session.proto` SHALL define `WidgetPublish` and `WidgetPublishResult` message types alongside existing zone publish and session lifecycle messages

#### Scenario: events.proto unchanged
- **WHEN** building the v1 protobuf package with widget system additions
- **THEN** `events.proto` SHALL remain identical to its pre-widget-system state; no widget-related types SHALL be added to `events.proto`

---

### Requirement: ClientMessage and ServerMessage Envelopes

Every client-to-server message on the session stream SHALL be wrapped in a ClientMessage envelope, and every server-to-client message SHALL be wrapped in a ServerMessage envelope. Both envelopes SHALL contain: sequence (per-direction monotonically increasing, starting at 1), timestamp_wall_us (sender wall-clock, advisory only), and a oneof payload. ClientMessage oneof payload fields SHALL be allocated at 10-35 (session lifecycle at 10-12, agent operations at 20-33, widget publishing at 35). ServerMessage oneof payload fields SHALL be allocated at 10-47 (session lifecycle responses at 10-15, mutation/lease responses at 20-25, scene state and events at 30-36, zone/widget results at 39-47). Fields 50-99 in both envelopes SHALL be reserved for post-v1 use.
Source: RFC 0005 SS2.2, SS9.2
Scope: v1-mandatory

#### Scenario: widget_publish at field 35
- **WHEN** an agent sends a WidgetPublish message on the session stream
- **THEN** it SHALL be encoded as ClientMessage oneof payload field 35

#### Scenario: widget_publish_result at field 47
- **WHEN** the runtime sends a WidgetPublishResult acknowledgement
- **THEN** it SHALL be encoded as ServerMessage oneof payload field 47

#### Scenario: existing fields unchanged
- **WHEN** the widget system fields are added to ClientMessage and ServerMessage
- **THEN** all existing field allocations SHALL remain at their current positions: ClientMessage session lifecycle at 10-12, agent operations at 20-33; ServerMessage session lifecycle responses at 10-15, mutation/lease responses at 20-25, scene state and events at 30-36, zone results at 39-46

---

## ADDED Requirements

### Requirement: Widget Publish Session Message

WidgetPublish SHALL be a ClientMessage payload at field 35 on the session stream. It MUST carry: widget_name (string), instance_id (string, optional — for disambiguating multiple instances of same type on a tab), params (repeated WidgetParameterValueProto — each with param_name and value), transition_ms (uint32, default 0), ttl_us (uint64, 0 = widget default), and merge_key (string, for MergeByKey widgets). Durable-widget publishes SHALL be transactional and receive WidgetPublishResult acknowledgement. Ephemeral-widget publishes SHALL be fire-and-forget (no WidgetPublishResult sent). WidgetPublish MUST require `publish_widget:<widget_name>` capability. The traffic class for widget publishes follows the same rules as zone publishes: durable = transactional, ephemeral = state-stream.
Source: widget-system proposal
Scope: v1-mandatory

#### Scenario: Durable widget publish receives ack
- **WHEN** an agent sends WidgetPublish targeting a durable widget with valid parameters
- **THEN** the runtime SHALL process the parameter values, apply them to the widget instance, and respond with WidgetPublishResult(accepted=true, widget_name=<name>)

#### Scenario: Ephemeral widget publish is fire-and-forget
- **WHEN** an agent sends WidgetPublish targeting an ephemeral widget
- **THEN** the runtime SHALL apply the parameter values but SHALL NOT send a WidgetPublishResult

#### Scenario: Missing capability rejected
- **WHEN** an agent sends WidgetPublish for widget "gauge_01" without the `publish_widget:gauge_01` capability
- **THEN** the runtime SHALL reject the publish with WidgetPublishResult(accepted=false, error={code=WIDGET_CAPABILITY_MISSING})

#### Scenario: Widget not found rejected
- **WHEN** an agent sends WidgetPublish with widget_name referencing a widget not present in the registry
- **THEN** the runtime SHALL reject the publish with WidgetPublishResult(accepted=false, error={code=WIDGET_NOT_FOUND})

---

### Requirement: Widget Publish Result

WidgetPublishResult SHALL be a ServerMessage payload at field 47. It MUST carry: accepted (bool), widget_name (string), and error (optional structured error with code and message). Error codes: WIDGET_NOT_FOUND, WIDGET_UNKNOWN_PARAMETER, WIDGET_PARAMETER_TYPE_MISMATCH, WIDGET_PARAMETER_INVALID_VALUE, WIDGET_CAPABILITY_MISSING. WidgetPublishResult SHALL only be sent for durable-widget publishes.
Source: widget-system proposal
Scope: v1-mandatory

#### Scenario: Accepted result
- **WHEN** the runtime successfully applies a durable widget publish
- **THEN** it SHALL send WidgetPublishResult(accepted=true, widget_name=<name>) with no error field

#### Scenario: Rejected with error code
- **WHEN** a durable widget publish includes a parameter name not declared in the widget's schema
- **THEN** the runtime SHALL send WidgetPublishResult(accepted=false, widget_name=<name>, error={code=WIDGET_UNKNOWN_PARAMETER, message="parameter '<param_name>' is not declared in widget '<widget_name>' schema"})

#### Scenario: No result for ephemeral
- **WHEN** an agent publishes to an ephemeral widget
- **THEN** the runtime SHALL NOT send a WidgetPublishResult regardless of whether the publish succeeded or failed

---

### Requirement: Widget Registry Query

WidgetRegistryRequest and WidgetRegistryResponse SHALL be defined as types in types.proto (not as session messages). Agents SHALL query the widget registry via the SceneSnapshot delivered at session establishment, which MUST include the full WidgetRegistrySnapshot. There SHALL be no separate widget registry query RPC in v1 — the snapshot is authoritative. MCP agents query via the `list_widgets` tool.
Source: widget-system proposal
Scope: v1-mandatory

#### Scenario: Scene snapshot includes widget registry
- **WHEN** a new agent session is established and the runtime sends SceneSnapshot
- **THEN** the SceneSnapshot SHALL include a WidgetRegistrySnapshot containing all registered widget instances with their definitions, parameter schemas, geometry policies, and current occupancy state

#### Scenario: No separate query RPC
- **WHEN** a resident agent needs to discover available widgets
- **THEN** it SHALL use the WidgetRegistrySnapshot from the SceneSnapshot received at session establishment; no separate WidgetRegistryRequest/WidgetRegistryResponse RPC exchange SHALL exist on the session stream in v1
