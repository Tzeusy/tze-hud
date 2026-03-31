## 1. Protobuf Definitions

- [ ] 1.1 Add widget types to `types.proto`: `WidgetDefinitionProto`, `WidgetInstanceProto`, `WidgetParameterSchemaProto`, `WidgetParameterDeclarationProto`, `WidgetParamTypeProto`, `WidgetParameterValueProto`, `WidgetParamConstraintsProto`, `WidgetSvgLayerProto`, `WidgetBindingProto`, `WidgetBindingMappingProto`, `WidgetRegistrySnapshotProto`, `WidgetPublishRecordProto`, `WidgetOccupancyProto`
- [ ] 1.2 Add widget mutation types to `types.proto`: `PublishToWidgetMutation`, `ClearWidgetMutation`
- [ ] 1.3 Add widget query types to `types.proto`: `WidgetRegistryRequest`, `WidgetRegistryResponse`
- [ ] 1.4 Add `WidgetPublish` message and `widget_publish` field (35) to `ClientMessage` in `session.proto`
- [ ] 1.5 Add `WidgetPublishResult` message and `widget_publish_result` field (47) to `ServerMessage` in `session.proto`
- [ ] 1.6 Run `protoc` and verify generated Rust code compiles cleanly

## 2. Scene Graph Types (tze_hud_scene)

- [ ] 2.1 Add `WidgetParamType` enum (F32, String, Color, Enum) and `WidgetParameterValue` enum
- [ ] 2.2 Add `WidgetParamConstraints` struct (min/max for f32, max_length for string, allowed_values for enum)
- [ ] 2.3 Add `WidgetParameterDeclaration` struct (name, param_type, default_value, constraints)
- [ ] 2.4 Add `WidgetBindingMapping` enum (Linear, Direct, Discrete) and `WidgetBinding` struct (param, target_element, target_attribute, mapping)
- [ ] 2.5 Add `WidgetSvgLayer` struct (svg_resource_id, bindings)
- [ ] 2.6 Add `WidgetDefinition` struct (id, name, description, parameter_schema, layers, default_geometry_policy, default_rendering_policy, default_contention_policy, ephemeral)
- [ ] 2.7 Add `WidgetInstance` struct (widget_type_name, tab_id, geometry_override, contention_override, current_params, instance_id)
- [ ] 2.8 Add `WidgetPublishRecord` struct (widget_name, publisher_namespace, params, published_at_wall_us, merge_key, expires_at_wall_us, transition_ms)
- [ ] 2.9 Add `WidgetOccupancy` struct (widget_name, tab_id, active_publications, occupant_count, effective_params)
- [ ] 2.10 Add `WidgetRegistry` struct (definitions, instances, active_publishes) with methods paralleling ZoneRegistry
- [ ] 2.11 Integrate WidgetRegistry into SceneGraph and SceneSnapshot serialization (deterministic BTreeMap ordering)
- [ ] 2.12 Add proto conversion impls (From<WidgetDefinition> for WidgetDefinitionProto, etc.)

## 3. Widget Asset Bundle Loader

- [ ] 3.1 Define `widget.toml` manifest schema: widget_type name, version, description, parameter_schema, layers with bindings
- [ ] 3.2 Implement bundle directory scanner: scan configured paths for subdirectories containing `widget.toml`
- [ ] 3.3 Implement manifest parser with structured error reporting (WIDGET_BUNDLE_NO_MANIFEST, WIDGET_BUNDLE_INVALID_MANIFEST, WIDGET_BUNDLE_DUPLICATE_TYPE)
- [ ] 3.4 Implement SVG file loading and parse validation (WIDGET_BUNDLE_MISSING_SVG, WIDGET_BUNDLE_SVG_PARSE_ERROR)
- [ ] 3.5 Implement parameter binding resolution: validate that all bindings reference valid parameter names and resolvable SVG element IDs / attributes (WIDGET_BINDING_UNRESOLVABLE)
- [ ] 3.6 Produce `WidgetDefinition` from parsed bundle and register SVG resources in the resource store

## 4. Resource Store: IMAGE_SVG Support

- [ ] 4.1 Add `IMAGE_SVG` variant to resource type enumeration
- [ ] 4.2 Implement SVG upload validation: well-formed XML with `<svg>` root element (no rasterization at upload time)
- [ ] 4.3 Implement SVG budget accounting: estimate rasterized size from viewBox/width/height (default 512x512, clamp 2048x2048), compute as `w * h * 4`
- [ ] 4.4 Add tests: valid SVG accepted, invalid XML rejected, non-SVG root rejected, budget estimation with/without viewBox

## 5. Configuration (tze_hud_config)

- [ ] 5.1 Add `[widget_bundles]` TOML section with `paths` field (array of directory paths, resolved relative to config file)
- [ ] 5.2 Add `[[tabs.widgets]]` TOML section with `type`, optional `instance_id`, optional `geometry`, optional `contention`, optional `initial_params`
- [ ] 5.3 Implement CONFIG_WIDGET_BUNDLE_PATH_NOT_FOUND validation (configured path doesn't exist)
- [ ] 5.4 Implement CONFIG_UNKNOWN_WIDGET_TYPE validation (tab references unloaded widget type)
- [ ] 5.5 Implement CONFIG_WIDGET_INVALID_INITIAL_PARAMS validation (initial_params fail schema check)
- [ ] 5.6 Implement CONFIG_WIDGET_BUNDLE_DUPLICATE_TYPE validation (same type name from two bundles)
- [ ] 5.7 Add `publish_widget:<name>` and `publish_widget:*` to the canonical capability vocabulary
- [ ] 5.8 Add tests: valid config, each error case, absent widget_bundles section (empty registry)

## 6. Widget Registry Runtime Integration

- [ ] 6.1 Wire bundle loader into runtime startup: scan bundles → register definitions → create instances per tab from config
- [ ] 6.2 Implement widget parameter validation on publish: schema check, type check, f32 clamping, NaN/infinity rejection
- [ ] 6.3 Implement widget contention policies (reuse zone contention: LatestWins, Stack, MergeByKey, Replace)
- [ ] 6.4 Implement widget TTL/expiry (auto_clear_ms, expires_at_wall_us)
- [ ] 6.5 Implement widget occupancy resolution: apply contention policy to active publications, compute effective_params
- [ ] 6.6 Include widget registry in SceneSnapshot delivered at session establishment

## 7. Compositor: SVG Layer Rendering Pipeline

- [ ] 7.1 Add resvg dependency and integrate SVG tree parsing at widget type registration
- [ ] 7.2 Implement parameter binding resolution: apply current parameter values to SVG tree (modify attributes per binding mappings)
- [ ] 7.3 Implement SVG rasterization to GPU texture via resvg at the widget instance's geometry size
- [ ] 7.4 Implement texture caching: reuse rasterized texture between frames when parameters unchanged
- [ ] 7.5 Implement re-rasterization on parameter change (only when params differ from cached)
- [ ] 7.6 Implement parameter interpolation: linear for f32, component-wise sRGB for color, snap for string/enum
- [ ] 7.7 Implement transition_ms animation: interpolate at target_fps, re-rasterize each frame during transition
- [ ] 7.8 Composite widget textures into scene at correct layer attachment and z-order (>= WIDGET_TILE_Z_MIN)
- [ ] 7.9 Performance validation: re-rasterization < 2ms for 512x512 widget on reference hardware

## 8. Session Protocol: Widget Publish Handler

- [ ] 8.1 Handle `WidgetPublish` (ClientMessage field 35): parse params, validate against widget schema, resolve to widget instance
- [ ] 8.2 Enforce `publish_widget:<widget_name>` capability check
- [ ] 8.3 Route durable widget publishes through transactional path (send WidgetPublishResult)
- [ ] 8.4 Route ephemeral widget publishes through fire-and-forget path (no result)
- [ ] 8.5 Implement WidgetPublishResult error codes: WIDGET_NOT_FOUND, WIDGET_UNKNOWN_PARAMETER, WIDGET_PARAMETER_TYPE_MISMATCH, WIDGET_PARAMETER_INVALID_VALUE, WIDGET_CAPABILITY_MISSING

## 9. MCP Tools

- [ ] 9.1 Add `publish_to_widget` MCP tool: widget_name, params, transition_ms, namespace, ttl_us
- [ ] 9.2 Add `list_widgets` MCP tool: return all widget types (name, description, parameter_schema) and instances per tab
- [ ] 9.3 Wire publish_to_widget to the widget registry publish path (same as gRPC but via MCP lease)
- [ ] 9.4 Add capability grant for `publish_widget:<name>` on MCP guest sessions (paralleling zone publish)

## 10. Heart-and-Soul Doctrine Updates

- [x] 10.1 Update `about/heart-and-soul/architecture.md`: add widgets as third publishing abstraction in protocol planes section, add widget compositor pipeline description in compositing model section
- [x] 10.2 Update `about/heart-and-soul/presence.md`: add "Widgets: parameterized visual publishing" section after zones, covering widget anatomy, how agents use widgets, relationship to zones and raw tiles
- [x] 10.3 Update `about/heart-and-soul/v1.md`: add widgets to "V1 ships" scene model section, add widget-specific success criterion

## 11. RFC / Law-and-Lore Updates

- [x] 11.1 Update `about/law-and-lore/rfcs/0001-scene-contract.md`: add widget ontology (type, instance, publication, occupancy) paralleling zone ontology, add WidgetRegistry to scene graph
- [x] 11.2 Update `about/law-and-lore/rfcs/0005-session-protocol.md`: add WidgetPublish/WidgetPublishResult message definitions, field allocations (35/47), traffic class assignment
- [x] 11.3 Update `about/law-and-lore/rfcs/0006-configuration.md`: add [widget_bundles] and [[tabs.widgets]] config sections, add publish_widget capability vocabulary
- [x] 11.4 Update `about/law-and-lore/rfcs/0011-resource-store.md`: add IMAGE_SVG resource type, SVG validation rules, budget accounting

## 12. Tests

- [ ] 12.1 Unit tests: WidgetParameterValue validation (f32 clamping, NaN rejection, type mismatch, enum validation)
- [ ] 12.2 Unit tests: widget asset bundle parser (valid bundle, each error case)
- [ ] 12.3 Unit tests: parameter binding resolution (linear, direct, discrete mappings)
- [ ] 12.4 Unit tests: widget contention policies (LatestWins, Stack, MergeByKey, Replace)
- [ ] 12.5 Unit tests: widget registry (definition registration, instance creation, publish, occupancy resolution)
- [ ] 12.6 Integration tests: MCP publish_to_widget end-to-end (publish params, verify occupancy)
- [ ] 12.7 Integration tests: gRPC WidgetPublish end-to-end (durable ack, ephemeral fire-and-forget)
- [ ] 12.8 Integration tests: widget in SceneSnapshot (session establishment includes widget registry)
- [ ] 12.9 Compositor tests: SVG rasterization correctness (render known SVG, compare output texture)
- [ ] 12.10 Compositor tests: parameter interpolation (verify intermediate values during transition)
- [x] 12.11a Performance tests: re-rasterization < 2ms for 512x512 (CI benchmark + lenient CI assertion)
- [ ] 12.11b Performance tests: SceneSnapshot < 1ms with widgets
- [ ] 12.12 Configuration tests: valid widget config, each CONFIG_WIDGET_* error case
