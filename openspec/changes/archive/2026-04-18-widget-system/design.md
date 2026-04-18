## Context

The zone system gives agents a fire-and-forget publishing surface: an agent calls `publish_to_zone("subtitle", content)` and the runtime handles rendering, contention, TTL, and cleanup. Zones accept raw content — text, key-value pairs, colors, images — and the runtime renders that content directly. This works well for information display, but it leaves a gap: there is no way for an agent to publish a *value* and have the runtime render a *visual interpretation* of that value.

Consider a temperature gauge. Today, an agent wanting to display "78% capacity" as a filling bar must drop to the full tile + node tree API: create a tile, compute a SolidColorNode at the right height, add a TextMarkdownNode for the label, manage z-order and bounds arithmetic, and keep it all alive with lease renewals. This is the exact "LLM in the frame loop" pattern the architecture forbids — the agent is computing geometry, not declaring intent.

Widgets fill this gap by extending the zone pattern to parameterized visuals. A widget is a runtime-owned visual template (SVG layers with parameter bindings) that agents parameterize with simple values (numbers, strings, colors, enums). The runtime owns the visual assets, renders at compositor frame rate, and smoothly interpolates between parameter states. The agent publishes `{ fill_level: 0.78, label: "Capacity" }` — the runtime renders a gauge.

The zone infrastructure already solves runtime-owned rendering, contention, geometry, capability governance, and TTL. Widgets reuse all of it, adding only the SVG visual language and parameter binding model. No new architectural primitives are introduced.

## Goals / Non-Goals

**Goals:**

- Parameterized visual publishing: agents publish typed parameter values; the runtime renders visual interpretations via SVG templates with smooth transitions between states
- Modular, extensible widget definitions: widget visual assets are user-authorable asset bundles loaded at startup, not hardcoded into the runtime binary
- Smooth transitions: the compositor interpolates between old and new parameter values over a configurable `transition_ms` duration
- Same governance model as zones: widgets reuse the existing ContestionPolicy, GeometryPolicy, LayerAttachment, TTL, and capability model without forking the governance path

**Non-Goals:**

- Conditional visuals: no if/else, no show/hide based on parameter values, no template conditionals. Parameter bindings are pure value mappings.
- Widget-in-widget composition: widgets are atomic leaf elements. Compose by placing multiple widget instances in a tab.
- Turing-complete scripting: no expression evaluator, no JavaScript, no Lua. The binding model is declarative mappings, not a programming language.
- Custom shaders: widget visuals are SVG, not GPU shader programs. Custom render pipelines are not in scope.
- Animated SVG (SMIL): no `<animate>`, no `<animateTransform>`, no `<set>`. Animation comes from parameter interpolation driven by agent publishes, not from SVG-internal animation primitives.
- Runtime widget creation by agents: agents cannot define new widget types or upload widget asset bundles. Widget types are declared in configuration and loaded at startup.

## Decisions

### D1: SVG layers with parameter bindings as the visual language

**Choice:** Widget visuals are one or more SVG files (layers) with parameter bindings that map agent-published values to SVG attributes. The compositor rasterizes the SVG to a texture using resvg (a Rust SVG rendering library), re-rasterizing only when parameters change.

**Rationale:** SVG is the right abstraction level for parameterized 2D visuals:

- **Declarative and resolution-independent.** SVG scales cleanly to any widget geometry without pixelation. A gauge defined at 100x100 renders crisply at 512x512.
- **Mature tooling.** Designers author SVG in Illustrator, Figma, Inkscape. Widget bundles are authorable by humans without Rust or shader expertise.
- **Inspectable.** SVG is XML — parameter bindings can be verified by reading the manifest and cross-referencing SVG attributes. No opaque binary formats.
- **Bounded complexity.** By restricting to a practical SVG subset (paths, shapes, text, gradients, transforms, clip-paths) and forbidding scripting, filters, and animation, the rendering cost is predictable and the security surface is minimal.
- **resvg is production-quality.** Pure Rust, no system dependencies, renders the SVG static subset faithfully. Used in production by Typst, Zed, and others.

**Alternatives considered:**

- *Custom DSL (JSON/TOML scene description):* Would require inventing a visual language, building a renderer, and teaching it to designers. SVG is a better starting point — it is already a visual language with tooling.
- *HTML/CSS via embedded browser:* Violates the "browser is not the main renderer" anti-pattern. Introduces a WebView dependency, unpredictable layout timing, and a massive security surface. Not acceptable for the hot path.
- *Canvas API:* Imperative drawing is the wrong abstraction — it puts code in the rendering path. SVG is declarative.
- *WebGL/wgpu shaders:* Too low-level for parameterized templates. Writing a gauge as a fragment shader is possible but not authorable by non-engineers, and debugging shader logic is far harder than inspecting SVG markup.

### D2: Widget asset bundle format

**Choice:** A widget asset bundle is a directory on disk containing:

1. **`widget.toml`** — The manifest. Declares:
   - `widget_type`: String identifier (e.g., `"gauge"`, `"progress_bar"`, `"temperature_dial"`)
   - `version`: Semantic version string (informational in v1; no compatibility enforcement)
   - `description`: Human-readable description
   - `default_geometry`: Optional GeometryPolicy override
   - `[[parameters]]`: Array of parameter schema entries, each with:
     - `name`: Parameter identifier (snake_case, unique within the widget type)
     - `type`: One of `f32`, `string`, `color`, `enum`
     - `default`: Default value (used when no publication exists)
     - `min`, `max`: Range bounds (f32 only)
     - `choices`: Allowed values (enum only)
   - `[[layers]]`: Array of SVG layer entries, each with:
     - `file`: Relative path to SVG file within the bundle directory
     - `z_order`: Layer ordering (lower = further back)
     - `[[layers.bindings]]`: Array of parameter-to-SVG-attribute mappings:
       - `param`: Parameter name (must reference a declared parameter)
       - `svg_element_id`: SVG element ID (the `id` attribute in the SVG)
       - `svg_attribute`: SVG attribute to bind (e.g., `height`, `fill`, `transform`, `text-content`)
       - `map_min`, `map_max`: SVG attribute range for f32 interpolation (e.g., `map_min = "0"`, `map_max = "200"` maps param min..max to SVG attribute 0..200)

2. **SVG files** — One or more `.svg` files referenced by the manifest's layer entries.

**Rationale:** A directory-based format is the simplest thing that works:

- No archive format to parse (no zip, no tar, no custom binary container)
- Files are individually inspectable, diffable, and editable
- Standard filesystem permissions apply
- Glob-friendly for CI validation (`widget_bundles/*/widget.toml`)

The manifest is TOML because the rest of the configuration system is TOML (RFC 0006). No format mixing.

**Alternative considered:** Single-file archive (zip with embedded manifest). Rejected for v1 because it adds a decompression dependency, complicates debugging (can't `cat` a layer SVG), and the portability benefit doesn't matter when bundles live on the local filesystem next to the config file.

### D3: Parameter binding model

**Choice:** Each SVG layer declares bindings that map parameter names to SVG attributes. The binding model supports four parameter types with type-specific resolution:

| Type | Wire value | Binding behavior |
|------|-----------|-----------------|
| `f32` | 32-bit float, clamped to [min, max] | Linear interpolation: param value in [min, max] maps to SVG attribute value in [map_min, map_max]. Example: `fill_level` 0.0..1.0 maps to `height` 0..200 pixels. |
| `string` | UTF-8, max 1024 bytes | Direct substitution into SVG `text-content` (the text node content of the target element). Snap on change (no interpolation). |
| `color` | RGBA (4x u8) | Component-wise linear interpolation in sRGB space. The SVG attribute receives `#RRGGBBAA` hex. |
| `enum` | String from `choices` set | Direct substitution into the bound SVG attribute. Snap on change (no interpolation). Invalid values are rejected at publish time. |

Binding expressions are pure mappings, not expressions. There is no `if`, no arithmetic, no string formatting. The compositor resolves bindings at render time by:

1. Clamping f32 values to [min, max]
2. Computing the linear interpolation factor `t = (value - min) / (max - min)`
3. Mapping to SVG attribute value: `svg_value = map_min + t * (map_max - map_min)` (for numeric SVG attributes; parsed from the map_min/map_max strings)
4. Replacing the SVG attribute value in the retained SVG tree
5. Re-rasterizing the modified SVG to a texture

When `transition_ms > 0`, the compositor interpolates between old and new resolved values over the transition duration. f32 and color parameters interpolate smoothly (linear). String and enum parameters snap immediately at the start of the transition (they have no meaningful intermediate values).

**Rationale:** Linear interpolation between min/max is the simplest model that produces smooth visual transitions for gauges, progress bars, dials, and similar parameterized visuals. It avoids the complexity of expression evaluation while covering the dominant use case. Non-linear mappings (easing curves, logarithmic scales) are deliberately deferred — they can be added post-v1 as additional mapping functions without changing the binding schema.

**Alternative considered:** Expression language (e.g., `svg_height = fill_level * 200 + 10`). Rejected because any expression language, no matter how small, eventually grows toward Turing-completeness. The linear mapping model is intentionally inexpressive to keep widget bundles auditable and the compositor binding logic trivial.

### D4: Reuse zone infrastructure

**Choice:** Widgets are built on top of the zone infrastructure, not beside it. Specifically:

- **ContestionPolicy:** Widget instances declare a contention policy (LatestWins, Replace, Stack, MergeByKey) that governs how multiple agents publishing to the same widget interact. Same four policies, same semantics.
- **GeometryPolicy:** Widget instances declare geometry using the same `GeometryPolicyProto` (relative or edge-anchored) that zones use. The compositor resolves widget geometry through the same pipeline.
- **LayerAttachment:** Widget instances declare Background, Content, or Chrome attachment, same as zones. Content-layer widgets get tiles in the z >= WIDGET_TILE_Z_MIN band.
- **TTL:** Widget publications carry `ttl_us` with the same semantics as zone publications. Zero means widget-type default.
- **Capability model:** Publishing to a widget requires `publish_widget:<widget_name>` capability, structurally identical to `publish_zone:<zone_name>`. The capability vocabulary extends naturally.
- **Widget registry:** Parallel to `ZoneRegistry`. Loaded from configuration at startup. Agents discover available widgets via `list_widgets`. Same lifecycle: static in v1, no runtime creation.

Internally, the widget registry holds `WidgetDefinition` (type + parameter schema + SVG assets), `WidgetInstance` (definition bound to a tab), `WidgetPublication` (parameter values from an agent), and `WidgetOccupancy` (resolved render state with rasterized texture). This four-level ontology mirrors the zone ontology exactly.

**Rationale:** Zones proved that runtime-owned rendering with agent-owned content works. The governance model (contention, TTL, capabilities, layer attachment) is already designed, tested, and understood. Forking it for widgets would double the governance surface with no benefit. By reusing zone infrastructure, widgets inherit all the safety properties — budget enforcement, lease governance, graceful degradation — without re-deriving them.

**Alternative considered:** Separate widget subsystem with its own contention, TTL, and capability model. Rejected because the governance concerns are identical. The only difference between a zone and a widget is the rendering pipeline (raw content vs. parameterized SVG), not the lifecycle or arbitration logic.

### D5: Compositor SVG rendering pipeline

**Choice:** The compositor manages widget visuals through a five-stage pipeline:

1. **Registration (startup).** For each widget type declared in configuration, the compositor loads the widget asset bundle, parses the `widget.toml` manifest, validates the parameter schema, loads SVG files from disk, and parses each SVG into a retained `usvg::Tree` (resvg's SVG document model). Parsing errors at this stage are fatal startup errors.

2. **Instance binding (startup).** For each `[[widgets]]` entry in the tab configuration, the compositor creates a `WidgetInstance` linking the widget type to a tab, applying any geometry overrides. The instance starts with default parameter values from the schema.

3. **Publication (runtime).** When a `WidgetPublish` message arrives (via gRPC or MCP), the compositor:
   - Validates the widget name resolves to a registered instance on the agent's active tab
   - Validates each parameter value against the schema (type, range, enum membership)
   - Applies contention policy to determine the effective publication
   - Records the publication in the widget registry

4. **Binding resolution (render).** On each frame where a widget's parameters have changed, the compositor:
   - Clones the widget type's base `usvg::Tree` (or modifies in-place with restoration)
   - Applies each binding: locates the SVG element by `svg_element_id`, sets the `svg_attribute` to the resolved value (interpolated if transition is in progress)
   - Rasterizes the modified SVG to an RGBA texture via `resvg::render()` at the widget instance's pixel dimensions
   - Under degradation level RENDERING_SIMPLIFIED or higher, the compositor MAY skip interpolation and snap to final parameter values immediately, reducing re-rasterization to at most once per parameter change rather than once per frame during transitions.
   - Uploads the texture to the GPU as a standard wgpu texture

5. **Caching.** The rasterized texture is cached and reused on subsequent frames until parameters change again. Widgets whose parameters have not changed since the last rasterization skip stages 3-4 entirely and reuse the cached texture. The compositor tracks a dirty flag per widget instance.

**Performance budget:** Re-rasterization must complete in less than 2ms for a 512x512 widget on a single core. resvg benchmarks at roughly 10-50ms for complex full-page SVGs, but widget SVGs are small (a gauge is typically 5-20 SVG elements, not hundreds). For simple widget templates, resvg rasterizes well under 1ms at 512x512. The 2ms budget provides headroom.

**Rationale:** CPU-based SVG rasterization via resvg is the pragmatic choice for v1:

- resvg is a mature, pure-Rust, no-system-dependency SVG renderer. It integrates naturally with the existing Rust + wgpu compositor.
- Widget parameter changes are infrequent (agents publish values, not animation frames). Re-rasterizing on parameter change amortizes the CPU cost over many display frames.
- The rasterized texture is a standard GPU texture — the compositor composits it identically to StaticImageNode textures. No new GPU pipeline is needed.
- GPU-accelerated SVG rendering (e.g., vello, lyon + wgpu) could replace resvg post-v1 if profiling shows CPU rasterization is a bottleneck. The pipeline stages above remain the same; only stage 4's rasterizer changes.

**Alternative considered:** Direct GPU rendering of SVG paths (tessellation + GPU draw calls per widget per frame). Rejected for v1 because it requires a GPU vector graphics pipeline (vello or lyon), which is a significant integration effort for a rendering path that executes infrequently (only on parameter change). The texture-cache approach is simpler and sufficient.

### D6: MCP tool surface

**Choice:** Two new guest MCP tools, mirroring the zone tools exactly:

**`publish_to_widget`**
```json
{
  "widget_name": "gauge",
  "params": {
    "fill_level": 0.78,
    "label": "Capacity",
    "fill_color": [66, 133, 244, 255]
  },
  "transition_ms": 300
}
```
Publishes parameter values to a named widget on the agent's active tab. Returns success/failure with structured error on validation failure (unknown widget, type mismatch, out-of-range value).

**`list_widgets`**
```json
[
  {
    "name": "gauge",
    "widget_type": "gauge",
    "parameters": [
      {"name": "fill_level", "type": "f32", "default": 0.0, "min": 0.0, "max": 1.0},
      {"name": "label", "type": "string", "default": ""},
      {"name": "fill_color", "type": "color", "default": [200, 200, 200, 255]}
    ]
  }
]
```
Returns all widget instances available on the agent's active tab, including their parameter schemas. Agents use this to discover what widgets exist and what parameters they accept.

Both tools are guest-accessible. `publish_to_widget` requires `publish_widget:<widget_name>` capability. `list_widgets` has no capability requirement (same as `list_zones`).

**Rationale:** The MCP surface mirrors zone tools for consistency. An agent that knows how to use `publish_to_zone` already understands the pattern — `publish_to_widget` follows the same fire-and-forget model with the same error semantics. `list_widgets` provides discoverability so agents never need to hardcode widget names or parameter schemas.

**Alternative considered:** Exposing widget publishing only via gRPC (no MCP tool). Rejected because the primary widget use case is guest agents publishing simple values — exactly the MCP guest pattern. Requiring gRPC for widget publishing would force agents to establish full resident sessions for a fire-and-forget operation.

### D7: gRPC wire format

**Choice:** Widget publishing on the bidirectional session stream uses two new messages:

**`WidgetPublish`** — ClientMessage oneof field **35**
```protobuf
message WidgetPublish {
  string widget_name                    = 1;  // Target widget instance name
  repeated WidgetParameterValue params  = 2;  // Parameter values to publish
  uint32 transition_ms                  = 3;  // Interpolation duration; 0 = snap
  string merge_key                      = 4;  // For MergeByKey contention; empty otherwise
  string instance_id                    = 5;  // For disambiguating multiple instances of same type; empty if unique
}

message WidgetParameterValue {
  string param_name = 1;
  oneof value {
    float   f32_value    = 2;
    string  string_value = 3;
    Rgba    color_value  = 4;
    string  enum_value   = 5;
  }
}
```

**`WidgetPublishResult`** — ServerMessage oneof field **47**
```protobuf
message WidgetPublishResult {
  uint64 request_sequence = 1;  // Echo of the ClientMessage sequence that triggered this
  bool   accepted         = 2;
  string error_code       = 3;  // Machine-readable error code (if accepted=false)
  string error_message    = 4;  // Human-readable reason (if accepted=false)
}
```

The transactional/ephemeral split follows zones: widget instances declared `ephemeral = true` in their definition are fire-and-forget (no WidgetPublishResult); non-ephemeral widget publishes receive acknowledgement.

**Field number justification:** ClientMessage currently uses fields 10-33. Field 34 is available (no existing allocation). Field 35 is chosen to leave field 34 as a buffer for potential future zone extensions, maintaining a clean separation. ServerMessage currently uses fields 10-46 (with 22 and 32 reserved). Field 47 is the next available field after the current maximum (46).

**Rationale:** Same multiplexing pattern as `ZonePublish`/`ZonePublishResult`. Widget messages ride the existing bidirectional session stream with no new RPCs or services. The `WidgetParameterValue` uses a typed oneof to ensure wire-level type safety — the compositor can reject type mismatches before attempting binding resolution.

### D8: Configuration

**Choice:** Two new TOML configuration sections:

**`[widget_bundles]`** — Specifies where to find widget asset bundles:
```toml
[widget_bundles]
paths = ["./widgets", "/usr/share/tze_hud/widgets"]
```
The runtime scans each path for directories containing `widget.toml`. Each valid directory is loaded as a widget type. Duplicate widget type names across paths produce `CONFIG_DUPLICATE_WIDGET_TYPE`. Missing or unparseable `widget.toml` produces `CONFIG_INVALID_WIDGET_BUNDLE`. Empty SVG files or SVG files that fail resvg parsing produce `CONFIG_WIDGET_SVG_PARSE_ERROR`. Unknown widget types referenced in `[[widgets]]` produce `CONFIG_UNKNOWN_WIDGET_TYPE`.

**`[[widgets]]`** — Per-tab widget instance declarations (inside `[[tabs]]`):
```toml
[[tabs]]
name = "Dashboard"

  [[tabs.widgets]]
  name = "cpu_gauge"
  type = "gauge"
  layer = "content"
  contention = "latest_wins"

  [tabs.widgets.geometry]
  x_pct = 0.7
  y_pct = 0.1
  width_pct = 0.25
  height_pct = 0.25

  [tabs.widgets.defaults]
  fill_level = 0.0
  label = "CPU"
  fill_color = [66, 133, 244, 255]
```

Widget instance `name` must be unique within a tab (same constraint as zone instance names). The `type` must reference a widget type loaded from `[widget_bundles]`. The `defaults` table provides initial parameter values that override the widget type's schema defaults.

Configuration validation follows the same structured error collection model as the rest of the config system (RFC 0006 §2.9): all validation errors are collected and reported together, each with `code`, `field_path`, `expected`, `got`, and `hint`.

**New config error codes:**
- `CONFIG_UNKNOWN_WIDGET_TYPE` — `[[tabs.widgets]]` references a type not found in any widget bundle
- `CONFIG_DUPLICATE_WIDGET_TYPE` — Two bundles declare the same widget type name
- `CONFIG_INVALID_WIDGET_BUNDLE` — `widget.toml` is missing, unparseable, or fails schema validation
- `CONFIG_WIDGET_SVG_PARSE_ERROR` — An SVG file referenced by a widget manifest fails resvg parsing
- `CONFIG_DUPLICATE_WIDGET_NAME` — Two widget instances on the same tab share a name
- `CONFIG_WIDGET_PARAM_TYPE_MISMATCH` — A default value in config does not match the parameter's declared type
- `CONFIG_WIDGET_PARAM_OUT_OF_RANGE` — A default f32 value is outside the parameter's [min, max] range
- `CONFIG_WIDGET_UNKNOWN_PARAM` — A default references a parameter name not in the widget type's schema

**Rationale:** The configuration structure mirrors zones: `[widget_bundles]` parallels the implicit zone type registry, `[[tabs.widgets]]` parallels `[[tabs.zones]]`. Configuration is the only way to declare widget instances in v1 — no runtime creation, no dynamic widget types.

**Alternative considered:** Widget definitions inline in the TOML config (SVG content as multiline strings). Rejected because SVG files can be large, inline TOML multiline strings are fragile for XML, and separating the manifest from the SVG files enables standard SVG tooling.

## Risks / Trade-offs

**SVG rasterization performance.** resvg is CPU-based. For a 512x512 widget with a simple SVG template (5-20 elements), rasterization is well under 2ms. However, a widget with a complex SVG (hundreds of paths, heavy gradients, many clip-paths) could exceed the 2ms budget. Mitigation: (1) re-rasterize only on parameter change, not every frame; (2) cache the rasterized texture; (3) document recommended SVG complexity limits in the widget authoring guide; (4) log a warning when rasterization exceeds 2ms so authors can simplify their SVGs. Post-v1, GPU-accelerated rasterization (vello) can replace resvg if profiling shows CPU rasterization is the bottleneck.

**SVG subset.** Full SVG 1.1 is enormous. tze_hud supports the practical subset that resvg renders: paths, basic shapes (rect, circle, ellipse, line, polyline, polygon), text (single-line and multiline), gradients (linear, radial), transforms, clip-paths, masks, and markers. Not supported: filters (`<feGaussianBlur>`, etc.), animations (SMIL), scripting (`<script>`), `<foreignObject>`, CSS `@import`, and `<use>` with external references. Widget authors who use unsupported features will see a startup parse error or a silently missing element. Mitigation: document the supported subset; validate SVGs at bundle load time; provide clear error messages.

**Parameter interpolation complexity.** Linear interpolation works for f32 (smooth gauge fill) and color (smooth color transitions). String and enum parameters snap immediately (no meaningful intermediate value between "Active" and "Inactive"). This is a deliberate simplification. Non-linear easing (ease-in-out, spring physics) is deferred to post-v1. For v1, `transition_ms` with linear interpolation covers the dominant use case (gauges, progress bars, dials) without introducing an easing curve vocabulary.

**Asset bundle versioning.** V1 does not version widget bundles. If a bundle's SVG files or manifest change on disk while the runtime is running, the runtime does not detect or reload them. The runtime must be restarted to pick up bundle changes. Hot-reload of widget asset bundles is deferred to post-v1. Mitigation: widget bundles are typically stable assets authored during development, not runtime-modified files. The restart requirement is acceptable for v1.

**IMAGE_SVG in the resource store.** Adding IMAGE_SVG to the resource type enumeration introduces a resource type that is fundamentally different from the existing raster types (PNG, JPEG, RGBA8). SVG is vector, not raster. The resource store validates that the SVG content parses correctly (via resvg) but does not decode it to pixels at upload time — unlike PNG/JPEG, where decode validation produces a raster image. Budget accounting uses the estimated rasterized size at the widget instance's geometry dimensions (width_px * height_px * 4 bytes for RGBA8), not the SVG file size. This means an SVG resource's budget impact depends on where it is used, which is a departure from the raster model where budget impact is intrinsic to the resource. Mitigation: SVG resources are only used internally by the widget system (agents do not directly reference SVG ResourceIds in nodes), so the variable-size accounting is contained within the widget pipeline.

**Widget input passthrough.** Widget tiles default to input_mode = Passthrough, meaning input events pass through to underlying tiles. This is intentional: widgets are visual indicators, not interactive surfaces. If a future version needs interactive widgets (clickable gauges, draggable sliders), the widget system would need to support HitRegionNode embedding and input event routing to the publishing agent. For v1, widgets are display-only.

**Widget tile z-order.** Widget tiles occupy the z-order band at z >= WIDGET_TILE_Z_MIN, parallel to but separate from ZONE_TILE_Z_MIN. Both are in the reserved upper band (z >= 0x8000_0000). The exact allocation is: ZONE_TILE_Z_MIN = 0x8000_0000, WIDGET_TILE_Z_MIN = 0x9000_0000. This leaves room for 268 million zone z-order slots and 1.8 billion widget z-order slots. If a widget and a zone overlap spatially, the widget renders above the zone by default (higher z-order band). This ordering is intentional: widgets are typically foreground indicators, zones are typically background/chrome.
