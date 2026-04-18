## Why

Zones solve the "LLM publishes content with one call" problem, but they only accept raw content (text, KV pairs, colors, images). There is no way for an agent to publish a *value* and have the runtime render a *visual interpretation* of that value — a gauge filling to 80%, a progress bar, a temperature dial. Today, an agent wanting such visuals must drop to the full tile + node tree API: create a tile, compute geometry, build node trees, manage layout. This complexity gap between "publish a string" and "render a parameterized visual" is unnecessarily wide.

Widgets fill this gap: runtime-owned visual templates that agents parameterize with simple values (numbers, strings, enums). The runtime owns the visual assets, renders at 60fps, and smoothly animates between parameter states. Agents publish parameters — the runtime makes them beautiful.

This belongs in v1 because the zone system already proves the pattern (runtime-owned rendering, agent-owned content) and widgets extend it to parameterized visuals without new architectural primitives. Deferring it means agents will inevitably build poor widget-like behavior by compositing images — creating exactly the kind of "LLM in the frame loop" pattern the architecture forbids.

## What Changes

- **New concept: Widget.** A four-level ontology paralleling zones: widget type (schema + visual assets), widget instance (type bound to tab), widget publication (parameter values), widget occupancy (resolved render state).
- **New MCP tool: `publish_to_widget`.** Agents publish named parameters to a widget by name. Zero scene context required. Same fire-and-forget pattern as zone publishing.
- **New gRPC message: `WidgetPublish`.** Multiplexed on the existing bidirectional session stream. Same pattern as `ZonePublish`.
- **New protobuf types.** `WidgetDefinitionProto`, `WidgetInstanceProto`, `WidgetPublishMutation`, `WidgetParameterSchema`, `WidgetParameterValue`.
- **Widget asset bundle system.** Modular, extensible widget definitions loaded from asset bundles at startup. Each bundle contains: SVG layer definitions with parameter bindings, parameter schema (names, types, ranges), default geometry and rendering policy. User-authorable — not hardcoded.
- **Compositor: SVG layer renderer with parameter interpolation.** The compositor loads widget visual assets (SVG layers), binds parameter values to visual properties (fill height, rotation, color stops, opacity), and interpolates smoothly between old and new values on parameter update.
- **Widget registry.** Parallel to zone registry. Loaded from configuration + asset bundles at startup. Discoverable by agents via `list_widgets`.
- **New resource type: IMAGE_SVG.** SVG added to the resource store's supported type enumeration for widget asset bundles.
- **Configuration extensions.** `[[widgets]]` TOML section for declaring widget instances per tab, and `[widget_bundles]` for specifying asset bundle directories.

### Constraints

- Widgets are **atomic leaf elements** — no widget-in-widget composition. Compose by placing multiple widgets in a tab.
- Visual language is **SVG layers with parameter bindings** — no conditional logic, no scripting, no Turing-complete expressions. Parameter → visual property mapping only.
- Widgets reuse existing infrastructure: same contention policies as zones, same geometry policies, same layer attachment model, same capability/lease governance.

## Capabilities

### New Capabilities
- `widget-system`: Core widget system — widget type schema, parameter schema, asset bundle format, widget registry, widget instance lifecycle, parameter publishing, compositor SVG layer rendering with parameter interpolation, and MCP/gRPC publishing surface.

### Modified Capabilities
- `scene-graph`: Add WidgetDefinition, WidgetInstance, WidgetOccupancy to the scene graph type hierarchy. Add widget registry parallel to zone registry. Add WIDGET_TILE_Z_MIN constant for runtime-managed widget tiles.
- `session-protocol`: Add WidgetPublish client message and WidgetPublishResult server message to the bidirectional session stream.
- `configuration`: Add `[[widgets]]` and `[widget_bundles]` TOML configuration sections for widget instance declarations and asset bundle paths.
- `resource-store`: Add IMAGE_SVG to the v1 resource type enumeration for widget visual assets.

## Impact

- **Protobuf**: New message types in `types.proto` (widget definitions, parameter schema, parameter values) and `session.proto` (widget publish/result). No breaking changes to existing messages.
- **Scene graph crate**: New `WidgetRegistry` parallel to `ZoneRegistry`. New types: `WidgetDefinition`, `WidgetInstance`, `WidgetPublishRecord`, `WidgetOccupancy`, `WidgetParameterSchema`, `WidgetParameterValue`.
- **Compositor**: New SVG layer rendering pipeline. Must load SVG assets, parse parameter bindings, bind values to visual properties, interpolate between states. This is new rendering capability — the compositor currently only renders solid colors, text, images, and hit regions.
- **MCP crate**: New `publish_to_widget` and `list_widgets` tools.
- **Configuration crate**: New TOML sections and validation for widget declarations and asset bundle paths.
- **Resource store**: SVG resource type support (decode validation, storage).
- **Heart-and-soul doctrine**: Widgets documented as the third publishing abstraction alongside zones and raw tiles. Architecture.md updated with widget compositor pipeline. Presence.md updated with widget publishing flow.
- **RFCs**: Scene contract RFC extended with widget ontology. Session protocol RFC extended with widget messages. Configuration RFC extended with widget config sections. New or extended resource store RFC for SVG support.
