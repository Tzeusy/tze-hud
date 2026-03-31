## Context

tze_hud has a well-defined three-tier publishing model: raw tiles (full control), zones (runtime-owned content surfaces), and widgets (parameterized SVG visuals). Each tier defines behavioral contracts — contention, TTL, geometry policies — but none defines *visual identity*. There is no shared vocabulary for colors, typography, stroke treatments, or backdrop styles. Widget bundles are visual islands. Zone rendering policies are hardcoded per zone type with no token indirection.

The result: two independently-authored "subtitle" implementations will look different and cannot be swapped without re-authoring zone types, rendering policies, and widget bundles. The HUD has no coherent visual identity, and there is no plug-and-play mechanism for component implementations.

### Current Implementation State

The codebase has ~100k lines of Rust across 15 crates with a production-grade rendering pipeline:

- **Zone rendering** uses `glyphon` for text shaping/layout and wgpu rectangle primitives for backdrops. Zone backgrounds are currently **hardcoded by content type** in `compositor/src/renderer.rs::render_zone_content()` — notifications get urgency-tinted backgrounds, stream-text gets dark blue-gray, etc. The `RenderingPolicy` struct exists but has only four optional fields (`font_size_px`, `backdrop`, `text_align`, `margin_px`) and is minimally wired into the actual rendering path.

- **Widget rendering** uses a completely separate path: SVG attribute manipulation on raw strings → `resvg::usvg::Tree` parsing → `tiny_skia::Pixmap` rasterization → `wgpu::Texture` upload → textured quad compositing. Widgets are cached per-instance with a dirty flag.

- **Text rendering** uses `glyphon` with `FontFamily` enum (`SansSerif`, `Monospace`, `Serif`), `Metrics::new(font_size, font_size * 1.4)` for line height, and `Wrap::Word` for word wrapping. TextItems carry pixel position, bounds, font size, font family, color (sRGB u8), alignment, and overflow mode.

These are **two fundamentally different rendering paths** and the spec must not conflate them.

### Constraints

- **V1 scope.** Zone types are static (loaded from config). Widget types are static (loaded from asset bundles). No runtime creation of either.
- **Performance.** Widget re-rasterization must stay under 2ms at 512×512. Token resolution must not add measurable overhead to the render path.
- **Agent transparency.** Agents publish by zone/widget name with typed values. The shape language must be invisible to the publishing API.
- **Two rendering paths.** Zone rendering uses glyphon text + wgpu rect quads. Widget rendering uses SVG + resvg + tiny-skia → wgpu textures. Tokens must feed into both, but through different mechanisms.
- **wgpu rect pipeline.** The current rect rendering path uses axis-aligned rectangles with flat RGBA colors. No rounded corners, gradients, or shadows in v1. `backdrop_radius` is deferred.

## Goals / Non-Goals

**Goals:**
- Define a **design token system** that gives all HUD visual elements a shared vocabulary (colors, typography, spacing, strokes, backdrops), with typed value parsing rules.
- Define **component profiles** as the unit of visual identity — named bundles that package zone rendering policies, widget asset bundles, and design token overrides into a swappable whole.
- Establish **SVG authoring conventions** (for widget bundles) and **rendering policy requirements** (for zone rendering) that ensure readability.
- Enable **plug-and-play** component swapping: change which subtitle profile is active without modifying agent code.
- Define how tokens flow into **both** rendering paths: glyphon/quad for zones, SVG/resvg for widgets.

**Non-Goals:**
- Runtime theme switching (token values are resolved at startup; hot-reload is post-v1).
- CSS or web rendering (tokens resolve into SVG attributes and `RenderingPolicy` fields, not CSS custom properties).
- Agent-visible design tokens (agents publish values; they never see or reference tokens).
- Custom font files in v1 (glyphon system fonts only; font bundles deferred).
- Animated SVG transitions beyond the existing parameter interpolation system.
- Rounded corners on zone backdrops (wgpu rect pipeline is axis-aligned; deferred to post-v1 when SDF or mesh-based rects are available).
- Display-profile-adaptive token values (same token values across all display profiles in v1; per-profile token overrides deferred).
- Nestable/theme-level profiles (per-component profiles only in v1; a "dark theme" that overrides all components is post-v1).

## Decisions

### 1. Design tokens are a flat TOML table with typed value parsing

**Decision:** Design tokens are declared in a single `[design_tokens]` TOML section as a flat string-to-string map with dotted namespace conventions. Values are always strings in TOML, but the spec defines **four concrete value formats** that consumers parse on demand:

| Format | Pattern | Example | Used for |
|--------|---------|---------|----------|
| Color hex | `#RRGGBB` or `#RRGGBBAA` | `"#FFFFFF"`, `"#00000099"` | fill, stroke, backdrop colors |
| Numeric | Decimal integer or float | `"16"`, `"0.6"`, `"2.5"` | font sizes, stroke widths, opacity, spacing |
| Font family | Keyword or name | `"system-ui"`, `"monospace"`, `"serif"` | glyphon `FontFamily` mapping |
| Literal string | Any other string | `"bold"`, `"center"` | enumerated values |

Token values are stored as raw strings. Parsing happens at consumption time — when a rendering policy field or SVG placeholder needs a concrete typed value. Invalid format (e.g., `"#ZZZZZZ"` where a color is expected) produces a validation error at startup, not at render time.

**Rationale:** Keeping tokens as strings in storage avoids a type system in the token map itself. Typed parsing at consumption boundaries is simpler and mirrors how TOML already works (everything is a string until you interpret it). The four formats cover all v1 rendering needs without inventing a type language.

### 2. Font family tokens map to `tze_hud_scene::types::FontFamily` enum (v1: keywords only)

**Decision:** Typography family tokens accept exactly three keyword values that map to the existing `FontFamily` enum:

| Token value | Rust enum variant |
|-------------|-----------------|
| `"system-ui"` or `"sans-serif"` | `FontFamily::SystemSansSerif` |
| `"monospace"` | `FontFamily::SystemMonospace` |
| `"serif"` | `FontFamily::SystemSerif` |

Any other string value is rejected with `TOKEN_VALUE_PARSE_ERROR` at startup.

**Rationale:** The existing `FontFamily` enum derives `Copy` (and `Clone, Debug, Default, PartialEq, Eq`). Adding a `Named(String)` variant would require removing the `Copy` derive since `String` is not `Copy`. This cascades to every struct that contains `FontFamily` (including `RenderingPolicy`, `TextItem`, and potentially `SceneGraphSnapshot` serialization). The three system keywords cover all v1 HUD use cases. Named font families are deferred to post-v1 when a proper enum migration can be planned.

**Alternatives considered:**
- *Immediate `Named(String)` variant*: Breaks `Copy` derive, cascading changes across 5+ crates. Not justified for v1 where system fonts suffice.
- *`FontFamily` as `String` everywhere*: Loses type safety. The enum is the right abstraction; it just needs a non-Copy variant later.

### 3. SVG templates reference tokens via `{{token.name}}` mustache placeholders

**Decision:** Widget SVG files may contain `{{token.key}}` placeholders in attribute values and text content. The exact pattern is: `\{\{([a-z][a-z0-9]*(?:\.[a-z][a-z0-9_]*)*)\}\}` — opening `{{`, a valid dotted token key, closing `}}`. Whitespace between braces and key is NOT permitted (`{{ color.text.primary }}` is NOT a valid placeholder). Multiple placeholders in a single attribute value ARE permitted (e.g., `transform="translate({{spacing.unit}}, {{spacing.unit}})"`).

The runtime resolves all placeholders at bundle load time by text substitution on the raw SVG string, before any XML/SVG parsing. This means zero runtime overhead — resolved SVGs are retained as immutable strings for the lifecycle.

**Rationale:** Strict pattern matching (no whitespace, exact key format) avoids ambiguity. Pre-parse text substitution is the simplest mechanism and avoids coupling with resvg's XML processing.

**Alternatives considered:**
- *CSS custom properties in SVG*: `fill="var(--color-text-primary)"` — elegant but resvg's CSS variable support is incomplete. Would create a hard dependency on resvg feature parity.
- *Lenient whitespace*: `{{ color.text.primary }}` — adds regex complexity for no user benefit. Authors copy token keys from documentation; whitespace variations are just bugs.

### 4. Zone rendering consumes tokens through RenderingPolicy fields, not SVG

**Decision:** Zones render via glyphon text and wgpu rectangle quads — they do NOT use SVG. Tokens flow into zone rendering through an **extended RenderingPolicy** struct. The existing four fields are preserved; new fields are added. When a component profile is active, its zone rendering overrides are merged onto the zone type's default `RenderingPolicy`. When no profile is active, global tokens populate the default `RenderingPolicy` at startup.

The extended `RenderingPolicy`:
```rust
pub struct RenderingPolicy {
    // --- Existing fields (proto fields 1–4) ---
    pub font_size_px: Option<f32>,       // proto field 1
    pub backdrop: Option<Rgba>,          // proto field 2
    pub text_align: Option<TextAlign>,   // proto field 3
    pub margin_px: Option<f32>,          // proto field 4
    // --- New fields (proto fields 5–14) ---
    pub font_family: Option<FontFamily>, // proto field 5 (v1: SystemSansSerif/SystemMonospace/SystemSerif only)
    pub font_weight: Option<u16>,        // proto field 6; CSS numeric weight: 100–900
    pub text_color: Option<Rgba>,        // proto field 7
    pub backdrop_opacity: Option<f32>,   // proto field 8; 0.0–1.0; overrides backdrop.a; ignored when backdrop is None
    pub outline_color: Option<Rgba>,     // proto field 9
    pub outline_width: Option<f32>,      // proto field 10; px; 0 = no outline
    pub margin_horizontal: Option<f32>,  // proto field 11; overrides margin_px for horizontal
    pub margin_vertical: Option<f32>,    // proto field 12; overrides margin_px for vertical
    pub transition_in_ms: Option<u32>,   // proto field 13; fade-in duration (opacity 0→1)
    pub transition_out_ms: Option<u32>,  // proto field 14; fade-out duration (opacity 1→0)
}
```

This is a data-driven change to an existing struct. All new fields are `Option<T>` with `#[serde(default)]` for backward-compatible deserialization of existing snapshots. The protobuf schema (`types.proto`) must be extended with fields 5–14 using the same sentinel-value pattern as existing fields. The compositor reads these fields during `render_zone_content()` instead of using hardcoded values — this requires refactoring the rendering path and the `TextItem` factory methods.

**Rationale:** Zone rendering is glyphon + rects, not SVG. Driving it through struct fields is natural and keeps the rendering path simple. The compositor reads `policy.text_color.unwrap_or(default_color)` — no SVG parsing, no token resolution at render time. All token→field resolution happens once at startup.

**Alternatives considered:**
- *SVG-based zone rendering*: Replace glyphon text with SVG templates for zones. This would unify the rendering path but is a massive architectural change — glyphon is purpose-built for fast text shaping, and SVG rasterization is ~10x slower for plain text. Not justified.
- *Separate zone styling struct*: Instead of extending `RenderingPolicy`, create a new `ZoneStyle` type. Increases type proliferation without benefit — `RenderingPolicy` already exists and is attached to `ZoneDefinition`.

### 5. Component profiles are directories, not single files

**Decision:** A component profile is a directory containing:
```
profiles/
  my-subtitles/
    profile.toml          # metadata + token overrides
    widgets/              # widget asset bundles (standard format)
      subtitle-text/
        widget.toml
        backdrop.svg
        text.svg
    zones/                # zone rendering policy overrides
      subtitle.toml       # rendering policy for the subtitle zone type
```

Widget bundles within profiles follow the **exact same format** as global widget bundles — `widget.toml` + SVG files at the bundle root. There is no `layers/` subdirectory.

Profile-scoped widget bundles are namespaced to avoid collision with global bundles: a widget named `"gauge"` inside profile `"my-subtitles"` is registered as `"my-subtitles/gauge"` in the widget registry. Agents reference profile widgets by their namespaced name.

### 6. Component types define the swappable contract

**Decision:** A **component type** is a named contract that declares:
- Which zone type(s) it governs
- A list of **specific** canonical token keys it requires (not wildcards/namespaces)
- Visual identity requirements: a **readability technique** enum (`DualLayer`, `OpaqueBackdrop`, `None`)
- Geometry expectations (informal; not validated — geometry is the zone type's responsibility)

Component types are defined in the spec. They are NOT user-configurable in v1.

**Rationale:** Previous draft used "required token namespaces" (e.g., `color.subtitle.*`) which is fuzzy — the canonical schema doesn't define `color.subtitle.*` tokens. Listing specific required token keys is unambiguous and directly testable.

### 7. Readability enforcement is split by rendering path

**Decision:** Readability requirements differ by rendering path:

- **Zone rendering (glyphon/quad path):** Validated via RenderingPolicy fields. The subtitle component type requires `backdrop` color + `backdrop_opacity` >= 0.3 + `outline_width` >= 1.0. Notification requires `backdrop` color + `backdrop_opacity` >= 0.8. These are field-level checks on the effective `RenderingPolicy` at startup.

- **Widget rendering (SVG/resvg path):** Validated via SVG structural analysis. Text-bearing SVGs in profiles with readability requirements MUST have `data-role="backdrop"` and `data-role="text"` elements in correct document order, text elements MUST have `stroke` attributes when outline is required.

These are two separate validation passes, not one conflated check.

**Rationale:** The previous draft applied SVG structural validation to zone rendering, which doesn't make sense — zones don't use SVG. Splitting validation by rendering path maps directly to the implementation.

### 8. Readability validation is a hard gate in production, warning in dev builds

**Decision:** Bundles/profiles failing readability validation are **rejected** in production builds (non-zero exit if referenced in config). In development builds (`profile = "headless"` or `TZE_HUD_DEV=1`), readability violations are logged at WARN level but the bundle/profile is loaded. This enables iterative authoring without startup failures on every SVG edit.

**Rationale:** Hard gates are correct for production — invisible subtitles are a real user-facing bug. But requiring perfect readability during iterative SVG authoring creates friction. Development builds are already a relaxed validation context (headless, no GPU required).

## Risks / Trade-offs

**[Token placeholder collision]** → SVG content containing literal `{{` text would be incorrectly treated as token references. Mitigation: escape syntax `\{\{` for literal braces. Low risk — SVG attribute values rarely contain double-brace patterns.

**[Readability validation false negatives]** → The structural check (presence of stroke/backdrop) does not guarantee actual WCAG compliance — a 1px transparent stroke technically satisfies field-level checks. Mitigation: minimum thresholds are specified in the component type contract (outline_width >= 1.0, backdrop_opacity >= 0.3 for subtitle). The structural check is a minimum bar, not a proof of visual compliance.

**[RenderingPolicy struct growth]** → Adding fields to RenderingPolicy increases the struct size and may require migration of serialized snapshots. Mitigation: all new fields are `Option<T>` with `#[serde(default)]` — existing serialized data deserializes correctly with new fields as `None`. No migration needed.

**[Profile widget naming]** → Namespacing profile widgets as `"profile-name/widget-name"` means agents must know the profile name to publish to profile-specific widgets. Mitigation: this is intentional — profile widgets are internal to the profile's rendering, not exposed as agent-facing publishing surfaces. If a profile needs an agent-publishable widget, it should be a global widget bundle, not profile-scoped.

**[Font family v1 limitation]** → Only three system keywords are supported. Operators wanting custom fonts must wait for post-v1 enum migration. Mitigation: the three keywords (sans-serif, monospace, serif) cover all common HUD use cases. Custom fonts are a polish feature, not a functional requirement.

**[Protobuf wire format growth]** → Each new RenderingPolicy field adds to the protobuf message size in SceneGraphSnapshot. Agents receive these snapshots at session establishment. Mitigation: new fields use proto3 defaults (omitted when zero/None) — no wire overhead for unused fields. The 10 new fields add at most ~80 bytes per zone definition when all fields are set.

**[Zone name inconsistency]** → The zone registry uses hyphenated names (`"notification-area"`, `"status-bar"`) while config validation constants use underscored/shortened names (`"notification"`, `"status_bar"`). Profile zone override files must use registry names. Mitigation: documented in spec. The naming inconsistency is pre-existing and should be reconciled in a separate change.

**[No rounded backdrop corners in v1]** → `backdrop_radius` is listed as a future field but the wgpu rect pipeline uses axis-aligned rectangles. Mitigation: explicitly deferred. The field is not included in the v1 RenderingPolicy extension. SDF or mesh-based rounded rects can be added post-v1.
