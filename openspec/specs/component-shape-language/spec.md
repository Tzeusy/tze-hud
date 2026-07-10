# component-shape-language Specification
Status: implemented

## Purpose
Defines the v1 component shape language for token-driven visual identity, rendering-policy resolution, component profile loading, readability validation, and SVG token substitution used by zones and widgets.

Implementation: crates/tze_hud_config/src/tokens.rs; crates/tze_hud_config/src/component_profiles.rs; crates/tze_hud_config/src/readability.rs; crates/tze_hud_scene/src/svg_tokens.rs; crates/tze_hud_widget/src/svg_readability.rs; crates/tze_hud_runtime/src/component_startup.rs; crates/tze_hud_config/src/portal_tokens.rs; crates/tze_hud_runtime/src/portal_tokens.rs (text-portal `portal.*` namespace, hud-8691s)
## Requirements
### Requirement: Design Token System
The runtime MUST support a design token system — a flat key-value map of named visual primitives loaded from the `[design_tokens]` configuration section. Each token MUST have a dotted namespace key (e.g., `color.text.primary`) and a string value representing a concrete visual primitive. Token keys MUST match the pattern `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*` (lowercase ASCII, dot-separated segments, underscores permitted after the first character of each segment). Token values are stored as raw strings; typed interpretation is deferred to consumption time (see Token Value Parsing requirement). Tokens MUST be resolved at startup into concrete values. Token values MUST NOT reference other tokens (no aliases or computed values in v1). The token map MUST be immutable after startup. An empty or absent `[design_tokens]` section MUST be valid — the runtime SHALL use hardcoded fallback values for all canonical tokens.
Scope: v1-mandatory

#### Scenario: Tokens loaded from configuration
- **WHEN** the configuration contains `[design_tokens]` with entries like `"color.text.primary" = "#FFFFFF"` and `"typography.body.size" = "16"`
- **THEN** the runtime MUST load these tokens into a flat `HashMap<String, String>` accessible by key at startup

#### Scenario: Empty tokens section uses fallbacks
- **WHEN** the configuration contains an empty `[design_tokens]` section or no `[design_tokens]` section
- **THEN** the runtime MUST use hardcoded fallback values for all canonical token keys

#### Scenario: Token values are immutable after startup
- **WHEN** the runtime has completed startup and token resolution
- **THEN** no API, configuration reload, or runtime operation SHALL modify token values until the next restart

#### Scenario: Token with underscore in segment
- **WHEN** `[design_tokens]` contains `"color.fill_accent" = "#4A9EFF"`
- **THEN** the runtime MUST accept the key (underscores are valid after the first character of a segment)

---

### Requirement: Token Value Formats and Parsing
Design token values are stored as strings. Consumers (rendering policy resolution, SVG placeholder substitution) MUST interpret token values according to four concrete formats. Parsing MUST be performed at startup during rendering policy construction and SVG placeholder resolution — never at render time. A token value that fails to parse in its expected format MUST produce `TOKEN_VALUE_PARSE_ERROR` identifying the token key, expected format, and actual value.

**Color hex format:** A `#` followed by exactly 6 or 8 hexadecimal digits (case-insensitive). `#RRGGBB` is interpreted as RGBA with alpha = 255. `#RRGGBBAA` includes explicit alpha. Examples: `"#FFFFFF"` → Rgba(1.0, 1.0, 1.0, 1.0), `"#00000099"` → Rgba(0.0, 0.0, 0.0, 0.6). Parsing: each pair of hex digits is converted to a u8 [0, 255], then normalized to f32 [0.0, 1.0] by dividing by 255.0.

**Numeric format:** A decimal integer or floating-point number. Leading/trailing whitespace MUST NOT be permitted. Examples: `"16"` → 16.0f32, `"0.6"` → 0.6f32, `"2.5"` → 2.5f32. Negative values ARE permitted (e.g., `"-8"` for negative offsets). NaN and infinity strings MUST be rejected.

**Font family format:** One of three keywords that map to the `FontFamily` enum defined in `tze_hud_scene::types`:
- `"system-ui"` or `"sans-serif"` → `FontFamily::SystemSansSerif`
- `"monospace"` → `FontFamily::SystemMonospace`
- `"serif"` → `FontFamily::SystemSerif`

V1 supports only these three system font keywords. Named/custom font families (e.g., `"Fira Code"`) are NOT supported in v1 because the current `FontFamily` enum derives `Copy` and has no `Name(String)` variant. Named font support requires an enum redesign (removing `Copy`, adding a `Named(String)` variant) and is deferred to post-v1. Any font family token value that is not one of the three recognized keywords MUST produce `TOKEN_VALUE_PARSE_ERROR` at startup.

**Literal string format:** Any string value not parsed as the above types. Used for enumerated values (e.g., font weights as `"400"`, `"700"` which are parsed as numeric when consumed as `u16`).
Scope: v1-mandatory

#### Scenario: Color hex parsed to Rgba
- **WHEN** a rendering policy field consumes token `color.text.primary` with value `"#FFFFFF"`
- **THEN** the value MUST parse to `Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }`

#### Scenario: Color hex with alpha
- **WHEN** a rendering policy field consumes token `color.backdrop.default` with value `"#00000099"`
- **THEN** the value MUST parse to `Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.6 }` (153/255 ≈ 0.6)

#### Scenario: Numeric parsed to f32
- **WHEN** a rendering policy field consumes token `typography.body.size` with value `"16"`
- **THEN** the value MUST parse to 16.0f32

#### Scenario: Invalid color rejected at startup
- **WHEN** token `color.text.primary` has value `"not-a-color"` and is consumed as a color
- **THEN** startup MUST fail with `TOKEN_VALUE_PARSE_ERROR` identifying the key, expected format "color hex", and actual value

#### Scenario: Font family keyword mapping
- **WHEN** token `typography.body.family` has value `"monospace"`
- **THEN** the value MUST map to `FontFamily::SystemMonospace` (the `tze_hud_scene::types::FontFamily` enum variant)

#### Scenario: Unrecognized font family rejected
- **WHEN** token `typography.body.family` has value `"Fira Code"` (a named font, not a v1 keyword)
- **THEN** startup MUST fail with `TOKEN_VALUE_PARSE_ERROR` identifying the key, expected format "font family keyword (system-ui, sans-serif, monospace, serif)", and actual value `"Fira Code"`

---

### Requirement: Canonical Token Schema
The runtime MUST define a canonical token schema with the following required keys. A hardcoded fallback value MUST be provided for every canonical key. Operators MAY override any canonical token in `[design_tokens]`. Non-canonical keys (outside this schema) MUST be accepted without error — they are available for profile-specific use.

**Color tokens:**
- `color.text.primary` — primary text color (fallback: `"#FFFFFF"`)
- `color.text.secondary` — secondary/muted text color (fallback: `"#B0B0B0"`)
- `color.text.accent` — accent/highlight text color (fallback: `"#4A9EFF"`)
- `color.backdrop.default` — default backdrop fill color (fallback: `"#000000"`)
- `color.outline.default` — default text outline/stroke color (fallback: `"#000000"`)
- `color.border.default` — default border/frame color (fallback: `"#333333"`)
- `color.severity.info` — info severity indicator (fallback: `"#4A9EFF"`)
- `color.severity.warning` — warning severity indicator (fallback: `"#FFB800"`)
- `color.severity.error` — error severity indicator (fallback: `"#FF4444"`)
- `color.severity.critical` — critical severity indicator (fallback: `"#FF0000"`)

**Opacity tokens (numeric, 0.0–1.0):**
- `opacity.backdrop.default` — default backdrop opacity (fallback: `"0.6"`)
- `opacity.backdrop.opaque` — opaque backdrop opacity threshold (fallback: `"0.9"`)

**Typography tokens:**
- `typography.body.family` — body text font family (fallback: `"system-ui"`)
- `typography.body.size` — body text size in pixels (fallback: `"16"`)
- `typography.body.weight` — body text weight, CSS numeric (fallback: `"400"`)
- `typography.heading.family` — heading font family (fallback: `"system-ui"`)
- `typography.heading.size` — heading text size in pixels (fallback: `"24"`)
- `typography.heading.weight` — heading weight (fallback: `"700"`)
- `typography.subtitle.family` — subtitle font family (fallback: `"system-ui"`)
- `typography.subtitle.size` — subtitle text size in pixels (fallback: `"28"`)
- `typography.subtitle.weight` — subtitle weight (fallback: `"600"`)

**Spacing tokens (numeric, pixels):**
- `spacing.unit` — base spacing unit (fallback: `"8"`)
- `spacing.padding.small` — small internal padding (fallback: `"4"`)
- `spacing.padding.medium` — medium internal padding (fallback: `"8"`)
- `spacing.padding.large` — large internal padding (fallback: `"16"`)

**Stroke tokens (numeric, pixels):**
- `stroke.outline.width` — text outline stroke width (fallback: `"2"`)
- `stroke.border.width` — border/frame stroke width (fallback: `"1"`)

Token values are the same across all display profiles in v1. Per-display-profile token overrides are deferred to post-v1.
Scope: v1-mandatory

#### Scenario: All canonical tokens have fallbacks
- **WHEN** the runtime starts with an empty `[design_tokens]` section
- **THEN** every canonical token key MUST resolve to its documented fallback value

#### Scenario: Operator overrides canonical token
- **WHEN** `[design_tokens]` contains `"color.text.primary" = "#00FF00"`
- **THEN** the runtime MUST use `"#00FF00"` as the resolved value for `color.text.primary` instead of the fallback

#### Scenario: Non-canonical tokens accepted
- **WHEN** `[design_tokens]` contains `"custom.my_profile.highlight" = "#FF00FF"`
- **THEN** the runtime MUST accept and store the token without error; it is available to profiles and SVG templates that reference it

#### Scenario: Canonical token parsed at startup
- **WHEN** the runtime starts and constructs default rendering policies for built-in zone types
- **THEN** each canonical token MUST be parsed into its typed representation (Rgba, f32, FontFamily) and any parse failure MUST produce `TOKEN_VALUE_PARSE_ERROR` before the first frame renders

---

### Requirement: Two Rendering Paths for Token Consumption
Tokens feed into two distinct rendering paths. The spec MUST NOT conflate them:

**Zone rendering path (glyphon + wgpu quads):** Zones render text via `glyphon` (font shaping, glyph rasterization) and backdrops/outlines via wgpu rectangle primitives. Tokens flow into this path through **RenderingPolicy fields** — the extended `RenderingPolicy` struct attached to each `ZoneDefinition`. Token values are parsed into typed fields (`Rgba`, `f32`, `FontFamily`) at startup. The compositor reads these fields during `render_zone_content()`.

**Widget rendering path (SVG + resvg):** Widgets render via SVG templates with parameter bindings. Tokens flow into this path through **mustache placeholder substitution** — `{{token.key}}` patterns in raw SVG strings are replaced with concrete token values at bundle load time, before SVG parsing. The resolved SVG string is then parsed, rasterized, and cached as a GPU texture.

Readability validation is path-specific: RenderingPolicy field checks for zones, SVG structural analysis for widgets. A component profile MAY include assets for both paths (zone rendering overrides AND widget bundles) or either path independently.
Scope: v1-mandatory

#### Scenario: Subtitle zone renders via glyphon, not SVG
- **WHEN** the subtitle zone renders stream-text content
- **THEN** the compositor MUST use the zone's `RenderingPolicy` fields to drive glyphon text layout and wgpu rectangle backdrop rendering — it MUST NOT involve SVG parsing or resvg rasterization

#### Scenario: Subtitle widget renders via SVG
- **WHEN** a subtitle component profile includes a widget bundle for enhanced subtitle visuals (e.g., word-highlight overlay)
- **THEN** the compositor MUST render that widget via the standard SVG → resvg → texture pipeline with token-resolved SVG templates

#### Scenario: Tokens used in both paths simultaneously
- **WHEN** a subtitle profile provides zone rendering overrides (RenderingPolicy) AND widget bundles (SVG), and both reference `color.text.primary`
- **THEN** the zone rendering path MUST receive the parsed `Rgba` value in the `text_color` field, AND the widget SVG MUST have `{{color.text.primary}}` substituted with the hex string `"#FFFFFF"` — both consuming the same token through their respective mechanisms

---

### Requirement: Extended RenderingPolicy
The `RenderingPolicy` struct (defined in `tze_hud_scene::types`) MUST be extended with the following fields. All new fields MUST be `Option<T>` with `#[serde(default)]` for backward-compatible deserialization. Existing fields (`font_size_px`, `backdrop`, `text_align`, `margin_px`) MUST be preserved without modification.

**New fields:**
- `font_family: Option<FontFamily>` — glyphon font family for zone text. When `None`, falls back to `FontFamily::SystemSansSerif`.
- `font_weight: Option<u16>` — CSS numeric font weight (100–900). When `None`, falls back to 400. Values outside [100, 900] MUST be clamped.
- `text_color: Option<Rgba>` — explicit text color for zone rendering. When `None`, the compositor uses its existing per-content-type defaults.
- `backdrop_opacity: Option<f32>` — opacity multiplier for `backdrop` color, range [0.0, 1.0]. When `None`, the `backdrop` color's alpha channel is used as-is. When set, it overrides the backdrop color's alpha: `effective_alpha = backdrop_opacity`.
- `outline_color: Option<Rgba>` — text outline color for zone text rendering. When `None`, no text outline is rendered.
- `outline_width: Option<f32>` — text outline stroke width in pixels. When `None` or 0.0, no text outline is rendered. Glyphon does not natively support text outline; the compositor MUST implement outline by rendering the text twice: first with `outline_color` at +/-1px offsets in 4 or 8 directions (cardinal or cardinal+diagonal), then the fill text on top.
- `margin_horizontal: Option<f32>` — horizontal margin in pixels, overriding `margin_px` for the horizontal axis. When `None`, falls back to `margin_px`.
- `margin_vertical: Option<f32>` — vertical margin in pixels, overriding `margin_px` for the vertical axis. When `None`, falls back to `margin_px`.
- `transition_in_ms: Option<u32>` — fade-in duration when zone content appears (opacity 0 → 1). When `None` or 0, content appears instantly. Animation is opacity-only (alpha ramp on the zone's composite quad).
- `transition_out_ms: Option<u32>` — fade-out duration when zone content is cleared or replaced (opacity 1 → 0). When `None` or 0, content disappears instantly.

**Existing field clarification:**
- `backdrop: Option<Rgba>` — the backdrop rectangle color. When combined with `backdrop_opacity`, effective rendering is: `Rgba { r: backdrop.r, g: backdrop.g, b: backdrop.b, a: backdrop_opacity }`. When `backdrop_opacity` is `None`, uses `backdrop.a` as-is. When `backdrop` is `None`, no backdrop quad is rendered regardless of `backdrop_opacity` value — `backdrop_opacity` has no effect without a `backdrop` color.

**Protobuf wire format:** Each new field MUST be added to `RenderingPolicyProto` in `types.proto` using the next available field numbers (5–14). The existing fields (font_size_px=1, backdrop=2, text_align=3, margin_px=4) MUST NOT be renumbered. Proto conversion logic in `convert.rs` MUST use sentinel values for `Option` mapping (0.0 for unused floats, enum UNSPECIFIED for unused enums) consistent with the existing pattern.

**Compositor refactoring note:** The current compositor (`render_zone_content()`) does NOT read `RenderingPolicy` fields — zone backgrounds are hardcoded per `ZoneContent` variant. This spec requires the compositor to be refactored: (1) construct `TextItem` from RenderingPolicy fields (font_family, font_size_px, text_color, text_align), replacing the current factory methods that hardcode these values; (2) render backdrop quads using RenderingPolicy.backdrop/backdrop_opacity instead of content-type color matching; (3) implement outline rendering as a new capability. The `TextItem::from_zone_stream_text()` and `TextItem::from_zone_notification()` factory methods MUST be updated to accept RenderingPolicy (or the relevant fields) instead of hardcoded parameters.

**Performance note for outline rendering:** The 8-direction outline technique renders text 9 times per frame (8 offset passes + 1 fill pass). For subtitle-sized text at 28px this is acceptable. For zones with large text volumes (notification stacks with many items), the performance cost scales linearly with text count. The compositor SHOULD skip outline passes when `outline_width` is `None` or 0.0 to avoid any overhead on zones that don't use outlines.

**Zone transition animation state:** Transition fade-in/out requires per-zone animation tracking. The compositor MUST maintain a `ZoneAnimationState` per zone instance (parallel to the existing `WidgetAnimationState` pattern used for widget parameter interpolation) with fields: `opacity: f32`, `target_opacity: f32`, `transition_start: Instant`, `duration_ms: u32`. When no transition is active, the opacity is 1.0 and no animation state is allocated.
Scope: v1-mandatory

#### Scenario: Zone text uses font_family from RenderingPolicy
- **WHEN** a zone's effective `RenderingPolicy` has `font_family = Some(FontFamily::SystemMonospace)` and `font_size_px = Some(24.0)`
- **THEN** the compositor MUST construct a glyphon `TextItem` with `font_family: FontFamily::SystemMonospace` and `font_size_px: 24.0`

#### Scenario: Text outline rendered via multi-pass
- **WHEN** a zone's effective `RenderingPolicy` has `outline_color = Some(Rgba::BLACK)` and `outline_width = Some(2.0)`
- **THEN** the compositor MUST render the text in `outline_color` at pixel offsets [(-2,0), (2,0), (0,-2), (0,2), (-2,-2), (2,-2), (-2,2), (2,2)] (8-direction outline), then render the fill text on top in `text_color`

#### Scenario: Backdrop opacity override
- **WHEN** a zone's effective `RenderingPolicy` has `backdrop = Some(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 })` and `backdrop_opacity = Some(0.6)`
- **THEN** the compositor MUST render the backdrop rectangle with `Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.6 }` — the `backdrop_opacity` overrides the color's alpha

#### Scenario: Transition fade-in on zone publish
- **WHEN** a zone's effective `RenderingPolicy` has `transition_in_ms = Some(200)` and content is published to the zone
- **THEN** the compositor MUST animate the zone's composite quad opacity from 0.0 to 1.0 over 200ms

#### Scenario: New fields default to None
- **WHEN** an existing `RenderingPolicy` is deserialized from a snapshot that predates this spec extension
- **THEN** all new fields MUST deserialize as `None` and the compositor MUST use fallback behavior for each

---

### Requirement: Default Zone Rendering with Tokens
When no component profile is active for a zone's component type, the runtime MUST still apply global design tokens to the zone's default `RenderingPolicy` at startup. For each built-in zone type, the following token-to-field mappings MUST be applied at startup (populating `None` fields only — explicit config values take precedence):

**subtitle zone defaults:**
- `text_color` ← `color.text.primary` (parsed as Rgba)
- `font_family` ← `typography.subtitle.family` (parsed as FontFamily)
- `font_size_px` ← `typography.subtitle.size` (parsed as f32)
- `font_weight` ← `typography.subtitle.weight` (parsed as u16)
- `backdrop` ← `color.backdrop.default` (parsed as Rgba)
- `backdrop_opacity` ← `opacity.backdrop.default` (parsed as f32)
- `outline_color` ← `color.outline.default` (parsed as Rgba)
- `outline_width` ← `stroke.outline.width` (parsed as f32)
- `text_align` ← `Center` (hardcoded default, not token-driven)
- `margin_vertical` ← `spacing.padding.medium` (parsed as f32)

**notification-area zone defaults:**
- `text_color` ← `color.text.primary`
- `font_family` ← `typography.body.family`
- `font_size_px` ← `typography.body.size`
- `font_weight` ← `typography.body.weight`
- `backdrop` ← `color.backdrop.default`
- `backdrop_opacity` ← `opacity.backdrop.opaque`
- `outline_color` ← `None` (no outline for notifications)
- `margin_horizontal` ← `spacing.padding.medium`
- `margin_vertical` ← `spacing.padding.medium`

**status_bar zone defaults:**
- `text_color` ← `color.text.secondary`
- `font_family` ← `typography.body.family`
- `font_size_px` ← `typography.body.size`
- `backdrop` ← `color.backdrop.default`
- `backdrop_opacity` ← `opacity.backdrop.opaque`

**alert_banner zone defaults:**
- `text_color` ← `color.text.primary`
- `font_family` ← `typography.heading.family`
- `font_size_px` ← `typography.heading.size`
- `font_weight` ← `typography.heading.weight`
- `backdrop` ← `color.backdrop.default` (static default; overridden at render time by urgency-to-severity mapping when the publication carries a `NotificationPayload` with urgency > 0)
- `backdrop_opacity` ← `opacity.backdrop.opaque`

**ambient_background:** No token-driven rendering policy fields (content is image/color).

**pip:** `border_color` rendering is post-v1 (no border rendering in current wgpu pipeline for video surfaces).

This mapping replaces the hardcoded per-content-type color logic in the current compositor. The compositor MUST read effective RenderingPolicy fields instead of branching on `ZoneContent` variant.
Scope: v1-mandatory

#### Scenario: Subtitle renders with token-derived colors
- **WHEN** the runtime starts with `color.text.primary = "#00FF00"` in `[design_tokens]` and no subtitle component profile is active
- **THEN** the subtitle zone's effective `RenderingPolicy` MUST have `text_color = Rgba(0.0, 1.0, 0.0, 1.0)` and subtitle text MUST render in green

#### Scenario: Token-derived defaults populated only for None fields
- **WHEN** a zone definition explicitly sets `font_size_px = Some(32.0)` in its definition, and the canonical token `typography.subtitle.size` is `"28"`
- **THEN** the effective `font_size_px` MUST remain 32.0 — the explicit value takes precedence over the token-derived default

#### Scenario: No hardcoded content-type color branching
- **WHEN** the compositor renders a notification zone
- **THEN** the backdrop color MUST come from the zone's effective `RenderingPolicy.backdrop` field, NOT from a match on `ZoneContent::Notification` variant

---

### Requirement: SVG Token Placeholder Resolution
Widget SVG files MAY contain `{{token.key}}` mustache-style placeholders in attribute values and text content. The exact placeholder pattern MUST be: opening `{{`, immediately followed by a valid token key matching `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*`, immediately followed by closing `}}`. No whitespace is permitted between the braces and the key. Multiple placeholders MAY appear in a single attribute value (e.g., `transform="translate({{spacing.unit}}, {{spacing.unit}})"`). The runtime MUST resolve all placeholders at widget-asset ingest time by substituting the token's concrete string value from the design token map. Ingest applies to both startup bundle/profile loading and runtime widget asset registration. Substitution is a single-pass left-to-right scan; substituted values are NOT re-scanned for further placeholders. Unresolved placeholders (token key not found in the map) MUST reject the asset (`WIDGET_BUNDLE_UNRESOLVED_TOKEN` for startup bundle/profile loads, `WIDGET_ASSET_INVALID_SVG` for runtime registrations). Literal double-brace sequences in SVG content MUST be escaped as `\{\{` and `\}\}`. After token substitution, the resulting SVG MUST be validated for parse correctness; SVG parse failure after substitution MUST produce `WIDGET_BUNDLE_SVG_PARSE_ERROR` (startup bundle/profile loads) or `WIDGET_ASSET_INVALID_SVG` (runtime registrations). Token resolution MUST occur exactly once per asset ingest event (startup load or runtime registration), and resolved SVGs are retained for the runtime lifecycle. Placeholders inside SVG `<style>` blocks and CDATA sections MUST be resolved identically to attribute values (the substitution operates on the raw text, before any XML parsing).
Scope: v1-mandatory

#### Scenario: Token placeholder resolved in SVG attribute
- **WHEN** a widget SVG contains `fill="{{color.text.primary}}"` and the design token `color.text.primary` resolves to `"#FFFFFF"`
- **THEN** the runtime MUST substitute the placeholder, producing `fill="#FFFFFF"` in the raw SVG string before parsing

#### Scenario: Multiple placeholders in one attribute
- **WHEN** a widget SVG contains `transform="translate({{spacing.unit}}, {{spacing.padding.small}})"`
- **THEN** the runtime MUST substitute both placeholders independently, producing `transform="translate(8, 4)"` with default token values

#### Scenario: Unresolved token rejects bundle
- **WHEN** a widget SVG contains `fill="{{color.nonexistent}}"` and no token with key `color.nonexistent` exists
- **THEN** the runtime MUST reject the bundle with error code `WIDGET_BUNDLE_UNRESOLVED_TOKEN` identifying the unresolved key and the SVG file

#### Scenario: Escaped braces preserved
- **WHEN** a widget SVG contains `data-label="\{\{not a token\}\}"`
- **THEN** the runtime MUST NOT treat the escaped braces as a token reference and MUST produce the literal `{{not a token}}` in the resolved SVG

#### Scenario: Whitespace inside braces is NOT a valid placeholder
- **WHEN** a widget SVG contains `fill="{{ color.text.primary }}"`
- **THEN** the runtime MUST NOT treat this as a placeholder (the leading space after `{{` breaks the pattern); the SVG is passed through as-is

#### Scenario: No recursive substitution
- **WHEN** token `custom.redirect` has value `"{{color.text.primary}}"` and a widget SVG contains `fill="{{custom.redirect}}"`
- **THEN** the runtime MUST substitute to produce `fill="{{color.text.primary}}"` literally — it MUST NOT re-scan and resolve the inner placeholder

#### Scenario: Post-substitution SVG validation
- **WHEN** token substitution produces an SVG string that is not valid SVG (e.g., a token value contains unescaped `<` characters)
- **THEN** the runtime MUST reject the bundle with error code `WIDGET_BUNDLE_SVG_PARSE_ERROR`

#### Scenario: Placeholder inside SVG style block
- **WHEN** a widget SVG contains `<style>.label { fill: {{color.text.primary}}; }</style>`
- **THEN** the runtime MUST resolve the placeholder within the style block, producing `<style>.label { fill: #FFFFFF; }</style>`

#### Scenario: Runtime-registered SVG resolves placeholders with same rules
- **WHEN** a runtime widget asset registration uploads an SVG containing `fill="{{color.text.primary}}"`
- **THEN** the runtime MUST apply the same single-pass token substitution and SVG-parse validation rules used for startup bundle assets before accepting registration

---

### Requirement: Component Type Contract
The specification defines **component types** — named contracts describing the visual-semantic identity of a class of HUD components. Each component type MUST declare:
1. A unique name (kebab-case)
2. The surface it governs. In v1 this is exactly one zone type per component type. A **promotion-era component type** (one whose first-class surface exists only after the relevant RFC promotion gate passes — currently only `text-portal`, gated on RFC 0013 §7.2) MAY instead govern a **first-class portal surface composed of named parts** rather than a single zone type; each styled part consumes `RenderingPolicy` like a zone does.
3. A **readability technique** requirement: one of `DualLayer` (backdrop + outline required), `OpaqueBackdrop` (backdrop with opacity >= threshold required), or `None` (no readability requirement). A multi-part surface declares a readability technique **per text-bearing part**; geometry-only parts declare `None`.
4. A list of specific canonical token keys that MUST be resolvable (from profile overrides, global tokens, or canonical fallbacks) for any active profile of this type
5. Informal geometry expectations (documented for profile authors; not validated at startup)

Component types are specification-defined constants — they are NOT user-configurable in v1. The v1 component types are: `subtitle`, `notification`, `status-bar`, `alert-banner`, `ambient-background`, and `pip`. The `text-portal` component type is defined **in addition to** the six v1 types and is recognized only after its RFC 0013 §7.2 promotion gate passes; it does not alter the six v1 zone-governing types.
Scope: v1-mandatory

#### Scenario: Component type declares governed zone
- **WHEN** the `subtitle` component type is defined
- **THEN** it MUST declare that it governs the `subtitle` zone type and MUST specify readability technique `DualLayer`

#### Scenario: Component type lists specific required tokens
- **WHEN** the `subtitle` component type is defined
- **THEN** it MUST list specific token keys: `color.text.primary`, `color.backdrop.default`, `opacity.backdrop.default`, `color.outline.default`, `typography.subtitle.family`, `typography.subtitle.size`, `typography.subtitle.weight`, `stroke.outline.width`

#### Scenario: All six v1 component types defined
- **WHEN** the runtime starts
- **THEN** the following component types MUST be recognized: `subtitle`, `notification`, `status-bar`, `alert-banner`, `ambient-background`, `pip`

#### Scenario: Promotion-era component type governs a multi-part surface
- **WHEN** the `text-portal` component type is defined after its RFC 0013 §7.2 promotion gate passes
- **THEN** it MUST govern a first-class portal surface composed of named parts (rather than a single zone type)
- **AND** each styled part MUST declare a readability technique and consume `RenderingPolicy`, while the six v1 zone-governing component types remain unchanged

---

### Requirement: V1 Component Type Definitions
The following component types MUST be defined with these contracts:

**subtitle**
- Governs: `subtitle` zone type
- Readability: `DualLayer` — RenderingPolicy MUST have `backdrop` color set, `backdrop_opacity` >= 0.3, `outline_color` set, and `outline_width` >= 1.0. Widget SVGs (if any) MUST have `data-role="backdrop"` + `data-role="text"` elements with text stroke.
- Required tokens: `color.text.primary`, `color.backdrop.default`, `opacity.backdrop.default`, `color.outline.default`, `typography.subtitle.family`, `typography.subtitle.size`, `typography.subtitle.weight`, `stroke.outline.width`
- Geometry expectation: bottom-center, 5–15% screen height, centered text. This is informational — geometry is governed by the zone type's `GeometryPolicy`, not validated against the component type.
- Rendering notes: text rendered via 8-direction outline (outline_color at offsets) + fill (text_color) over a semi-transparent backdrop quad. The backdrop quad spans the zone geometry minus margins.

**notification**
- Governs: `notification-area` zone type (note: the zone registry name is `"notification-area"`, though the config validation constant uses `"notification"` — see Zone Name Reconciliation below)
- Readability: `OpaqueBackdrop` — RenderingPolicy MUST have `backdrop` color set and `backdrop_opacity` >= 0.8. No outline requirement.
- Required tokens: `color.text.primary`, `color.backdrop.default`, `opacity.backdrop.opaque`, `color.border.default`, `typography.body.family`, `typography.body.size`, `typography.body.weight`, `spacing.padding.medium`, `stroke.border.width`
- Geometry expectation: top-right corner, stacked vertically, auto-dismisses.
- Rendering notes: each notification is a backdrop quad with 1px border (`color.border.default`) containing left-aligned text. The border is rendered as four thin quads along the edges of the backdrop.

**status-bar**
- Governs: `status_bar` zone type
- Readability: `OpaqueBackdrop` — chrome layer, background is controlled by runtime.
- Required tokens: `color.text.secondary`, `color.backdrop.default`, `opacity.backdrop.opaque`, `typography.body.family`, `typography.body.size`
- Geometry expectation: full-width strip, top or bottom edge.

**alert-banner**
- Governs: `alert_banner` zone type
- Readability: `OpaqueBackdrop`
- Required tokens: `color.text.primary`, `color.backdrop.default`, `opacity.backdrop.opaque`, `color.severity.info`, `color.severity.warning`, `color.severity.error`, `color.severity.critical`, `typography.heading.family`, `typography.heading.size`, `typography.heading.weight`
- Geometry expectation: full-width horizontal bar.
- Rendering notes: alert-banner currently accepts `ShortTextWithIcon` media type, which carries an `urgency: u32` field (0=low, 1=normal, 2=urgent, 3=critical) on `NotificationPayload`. The compositor MUST map urgency levels to severity tokens for backdrop color: urgency 0 or 1 → `color.severity.info`, urgency 2 → `color.severity.warning`, urgency 3 → `color.severity.critical`. `color.severity.error` is used when an explicit severity enum is available (post-v1; requires a dedicated `AlertPayload` content type with a severity field). The `color.backdrop.default` is the fallback when the publication has no urgency/severity or when the content type is not `Notification`. Text color is always `color.text.primary` for contrast against severity-colored backdrops.

**ambient-background**
- Governs: `ambient_background` zone type
- Readability: `None` (no text content)
- Required tokens: none
- Geometry expectation: full-screen background layer.

**pip**
- Governs: `pip` zone type
- Readability: `None` (video surface)
- Required tokens: `color.border.default`, `stroke.border.width` (reserved for post-v1 border rendering)
- Geometry expectation: corner-anchored, resizable within bounds.
Scope: v1-mandatory

#### Scenario: Subtitle readability validated on RenderingPolicy
- **WHEN** the effective RenderingPolicy for the subtitle zone has `backdrop = Some(Rgba::BLACK)`, `backdrop_opacity = Some(0.6)`, `outline_color = Some(Rgba::BLACK)`, `outline_width = Some(2.0)`
- **THEN** the DualLayer readability check MUST pass (backdrop present with opacity >= 0.3, outline present with width >= 1.0)

#### Scenario: Subtitle fails DualLayer without outline
- **WHEN** the effective RenderingPolicy for the subtitle zone has `backdrop` and `backdrop_opacity` set but `outline_color = None` and `outline_width = None`
- **THEN** the DualLayer readability check MUST fail with `PROFILE_READABILITY_VIOLATION` because outline is missing

#### Scenario: Notification readability validated on RenderingPolicy
- **WHEN** the effective RenderingPolicy for the notification zone has `backdrop = Some(Rgba::BLACK)` and `backdrop_opacity = Some(0.9)`
- **THEN** the OpaqueBackdrop readability check MUST pass (backdrop present with opacity >= 0.8)

#### Scenario: Alert banner backdrop derived from urgency-to-severity mapping
- **WHEN** a notification with urgency=2 (urgent) is published to the alert-banner zone and `color.severity.warning` resolves to `"#FFB800"`
- **THEN** the compositor MUST map urgency 2 → `color.severity.warning` and render the banner's backdrop quad with color Rgba from `"#FFB800"` at the zone's `backdrop_opacity`

#### Scenario: Ambient background has no readability check
- **WHEN** the runtime validates the ambient-background component type
- **THEN** no readability check is performed and no RenderingPolicy text/outline fields are required

---

### Requirement: Component Profile Format
A component profile MUST be a directory containing a `profile.toml` manifest and optional subdirectories for widget bundles (`widgets/`) and zone rendering overrides (`zones/`).

**profile.toml schema:**
```toml
name = "my-subtitles"           # kebab-case, unique across all loaded profiles
version = "1.0.0"               # semver
description = "Cinematic style" # human-readable

component_type = "subtitle"     # references a v1 component type name

[token_overrides]               # optional; scoped to this profile's assets
"color.text.primary" = "#FFFF00"
"typography.subtitle.size" = "32"
```

Required fields: `name`, `version`, `component_type`. Optional fields: `description` (defaults to `""`), `token_overrides` (defaults to empty map).

**widgets/ subdirectory:** Contains zero or more widget bundle directories, each following the standard widget asset bundle format (`widget.toml` + SVG files at the bundle root). Profile-scoped widget bundles are registered in the `WidgetRegistry` with a namespaced name: `"{profile_name}/{widget_name}"`. This prevents name collision with global widget bundles and other profiles. Agents referencing profile widgets MUST use the namespaced name.

**zones/ subdirectory:** Contains zero or more TOML files named `{zone_type_name}.toml` (e.g., `subtitle.toml`). Each file declares rendering policy overrides for that zone type. The zone type name MUST match a zone type governed by the profile's `component_type`. A profile for `subtitle` component type MUST NOT contain `zones/notification.toml` — this MUST produce `PROFILE_ZONE_OVERRIDE_MISMATCH`.

The runtime MUST scan profile directories specified in configuration. Invalid profiles MUST be logged and skipped (not halt startup). A profile with a `component_type` that does not match any defined component type MUST be rejected with `PROFILE_UNKNOWN_COMPONENT_TYPE`.
Scope: v1-mandatory

#### Scenario: Valid profile loaded
- **WHEN** the runtime scans a profile directory containing a valid `profile.toml` with `component_type = "subtitle"` and a `widgets/` directory with valid bundles
- **THEN** the profile MUST be registered and available for selection in configuration

#### Scenario: Unknown component type rejected
- **WHEN** a profile's `profile.toml` declares `component_type = "hologram"` and no `hologram` component type exists
- **THEN** the profile MUST be rejected with `PROFILE_UNKNOWN_COMPONENT_TYPE` and logged; other profiles continue loading

#### Scenario: Token overrides scoped to profile
- **WHEN** a profile declares `[token_overrides]` with `"color.text.primary" = "#00FF00"` and the global token for `color.text.primary` is `"#FFFFFF"`
- **THEN** the profile's widget SVGs and zone rendering overrides MUST resolve `color.text.primary` to `"#00FF00"`; all other profiles and global rendering MUST continue using `"#FFFFFF"`

#### Scenario: Invalid profile does not halt startup
- **WHEN** a profile directory contains a `profile.toml` with missing required field `component_type`
- **THEN** the profile MUST be skipped with a logged warning; the runtime MUST continue loading other profiles and start successfully

#### Scenario: Profile widget namespaced in registry
- **WHEN** profile `"my-subtitles"` contains `widgets/highlight/widget.toml` with `name = "highlight"`
- **THEN** the widget MUST be registered as `"my-subtitles/highlight"` in the WidgetRegistry

#### Scenario: Zone override for wrong zone type rejected
- **WHEN** a profile with `component_type = "subtitle"` contains `zones/notification.toml`
- **THEN** the profile MUST be rejected with `PROFILE_ZONE_OVERRIDE_MISMATCH` identifying the zone type and the profile's component type

---

### Requirement: Zone Rendering Override Schema
Zone rendering overrides in a profile's `zones/{zone_type}.toml` file MUST use the following TOML schema. Each field is optional; omitted fields retain the zone type's default value (which may itself be token-derived from the Default Zone Rendering with Tokens requirement). Each field value MAY be a literal value OR a `{{token.key}}` reference that is resolved against the profile's scoped token map at load time.

#### Scenario: Zone override file may contain literal and token values
- **WHEN** a profile provides `zones/subtitle.toml` with both literal values and `{{token.key}}` references
- **THEN** the runtime MUST parse literal values directly and resolve token references against the profile's scoped token map before merging the overrides onto the effective `RenderingPolicy`

**TOML schema:**
```toml
# Typography
font_family = "monospace"             # or "{{typography.subtitle.family}}"
font_size_px = 32.0                   # pixels, or "{{typography.subtitle.size}}"
font_weight = 600                     # CSS numeric 100–900

# Text
text_color = "#FFFF00"                # color hex, or "{{color.text.primary}}"
text_align = "center"                 # "start" | "center" | "end"

# Backdrop
backdrop_color = "#000000"            # color hex, or "{{color.backdrop.default}}"
backdrop_opacity = 0.7                # 0.0–1.0

# Outline
outline_color = "#000000"             # color hex, or "{{color.outline.default}}"
outline_width = 2.0                   # pixels

# Margins
margin_horizontal = 16.0              # pixels
margin_vertical = 8.0                 # pixels

# Transitions
transition_in_ms = 200                # milliseconds
transition_out_ms = 150               # milliseconds
```

**Field type rules:**
- Color fields (`text_color`, `backdrop_color`, `outline_color`): string value parsed as color hex per Token Value Formats requirement. TOML string type.
- Numeric fields (`font_size_px`, `font_weight`, `backdrop_opacity`, `outline_width`, `margin_horizontal`, `margin_vertical`): TOML float or integer, or string with `{{token}}` reference that resolves to a numeric value.
- Enum fields (`text_align`): string, must be one of `"start"`, `"center"`, `"end"`. Invalid values MUST produce `PROFILE_INVALID_ZONE_OVERRIDE` identifying the field, expected values, and actual value.
- Font family (`font_family`): string, parsed per font family format in Token Value Formats.
- Transition fields (`transition_in_ms`, `transition_out_ms`): TOML integer (u32).

When a field contains a `{{token.key}}` reference, the reference MUST be resolved against the profile's scoped token map (profile overrides → global → canonical fallbacks). Unresolved references MUST produce `PROFILE_UNRESOLVED_TOKEN` and reject the profile.

The resolved override values are merged onto the zone type's effective `RenderingPolicy` at startup. The merge is field-by-field: each non-null override field replaces the corresponding `RenderingPolicy` field.
Scope: v1-mandatory

#### Scenario: Profile overrides subtitle font via token reference
- **WHEN** the active subtitle profile contains `zones/subtitle.toml` with `font_family = "{{typography.subtitle.family}}"` and the profile's scoped token resolves `typography.subtitle.family` to `"monospace"`
- **THEN** the subtitle zone's effective `RenderingPolicy` MUST have `font_family = Some(FontFamily::SystemMonospace)`

#### Scenario: Profile overrides with literal value
- **WHEN** `zones/subtitle.toml` contains `backdrop_opacity = 0.8`
- **THEN** the subtitle zone's effective `RenderingPolicy` MUST have `backdrop_opacity = Some(0.8)`

#### Scenario: Partial override preserves token-derived defaults
- **WHEN** `zones/subtitle.toml` overrides only `backdrop_opacity = 0.8` and does not specify `text_color`
- **THEN** the subtitle zone MUST use opacity 0.8 for the backdrop and retain the token-derived default for `text_color` (from `color.text.primary`)

#### Scenario: Missing zone override file uses defaults
- **WHEN** an active profile does not contain a `zones/subtitle.toml` file
- **THEN** the subtitle zone MUST use its token-derived default rendering policy (no profile overrides applied)

#### Scenario: Invalid text_align value rejected
- **WHEN** `zones/subtitle.toml` contains `text_align = "middle"`
- **THEN** the profile MUST be rejected with `PROFILE_INVALID_ZONE_OVERRIDE` identifying the field `text_align`, expected values `["start", "center", "end"]`, and actual value `"middle"`

#### Scenario: Token reference in zone override resolved
- **WHEN** `zones/subtitle.toml` contains `text_color = "{{color.text.accent}}"` and the profile's scoped token map resolves `color.text.accent` to `"#4A9EFF"`
- **THEN** the override MUST set `text_color = Rgba(0.29, 0.62, 1.0, 1.0)`

---

### Requirement: Component Profile Selection
The configuration MUST support a `[component_profiles]` section mapping component type names to active profile names. Each entry binds a component type to its active implementation. If no profile is specified for a component type, the runtime MUST use the default rendering behavior (token-derived zone rendering policies, no profile-specific widgets, no profile token overrides). If a specified profile name does not match any loaded profile, startup MUST fail with `CONFIG_UNKNOWN_COMPONENT_PROFILE`. If a specified profile's `component_type` does not match the component type it is assigned to, startup MUST fail with `CONFIG_PROFILE_TYPE_MISMATCH`. Profile selection is immutable after startup — no runtime switching in v1. Nestable/theme-level profiles (a single profile overriding all component types) are NOT supported in v1.
Scope: v1-mandatory

#### Scenario: Profile selected for subtitle
- **WHEN** configuration contains `[component_profiles]` with `subtitle = "my-subtitles"` and a profile named `"my-subtitles"` with `component_type = "subtitle"` is loaded
- **THEN** the runtime MUST use the `"my-subtitles"` profile's zone rendering overrides, widget bundles, and token overrides for the subtitle zone

#### Scenario: No profile uses token-derived defaults
- **WHEN** configuration contains no `[component_profiles]` section or omits the `notification` component type
- **THEN** the runtime MUST use the default rendering behavior for notifications (token-derived RenderingPolicy fields from Default Zone Rendering with Tokens requirement)

#### Scenario: Unknown profile name rejected
- **WHEN** `[component_profiles]` specifies `subtitle = "nonexistent-profile"` and no profile with that name was loaded
- **THEN** startup MUST fail with `CONFIG_UNKNOWN_COMPONENT_PROFILE` identifying the profile name

#### Scenario: Profile type mismatch rejected
- **WHEN** `[component_profiles]` specifies `subtitle = "clean-notifs"` but the `"clean-notifs"` profile has `component_type = "notification"`
- **THEN** startup MUST fail with `CONFIG_PROFILE_TYPE_MISMATCH` indicating that a notification profile cannot serve as a subtitle implementation

---

### Requirement: Zone Readability Enforcement
All text-bearing component types (`subtitle`, `notification`, `status-bar`, `alert-banner`) MUST enforce readability through validation of the zone's effective `RenderingPolicy` at startup. The runtime MUST check the effective policy (after token-derived defaults AND profile overrides are applied) against the component type's readability technique:

**DualLayer (subtitle):** The effective RenderingPolicy MUST satisfy ALL of:
- `backdrop` is `Some(color)` where color is not fully transparent
- `backdrop_opacity` is `Some(v)` where v >= 0.3
- `outline_color` is `Some(color)` where color differs from `text_color` (if both are set)
- `outline_width` is `Some(v)` where v >= 1.0

**OpaqueBackdrop (notification, status-bar, alert-banner):** The effective RenderingPolicy MUST satisfy ALL of:
- `backdrop` is `Some(color)` where color is not fully transparent
- `backdrop_opacity` is `Some(v)` where v >= 0.8

**None (ambient-background, pip):** No validation.

Failure MUST produce `PROFILE_READABILITY_VIOLATION` with the component type name, the failing check, and the actual field values. In production builds, referenced profiles that fail readability are a hard startup error. In development builds (`profile = "headless"` OR environment variable `TZE_HUD_DEV=1`), readability violations are logged at WARN level and the profile is loaded — this enables iterative authoring without startup failures.
Scope: v1-mandatory

#### Scenario: Subtitle DualLayer passes
- **WHEN** the subtitle zone's effective RenderingPolicy has `backdrop = Some(Rgba::BLACK)`, `backdrop_opacity = Some(0.6)`, `outline_color = Some(Rgba::BLACK)`, `outline_width = Some(2.0)`, and `text_color = Some(Rgba::WHITE)`
- **THEN** the DualLayer readability check MUST pass

#### Scenario: Subtitle DualLayer fails — no outline
- **WHEN** the subtitle zone's effective RenderingPolicy has `backdrop` and `backdrop_opacity` set but `outline_width = None`
- **THEN** the readability check MUST fail with `PROFILE_READABILITY_VIOLATION` identifying "DualLayer: outline_width must be >= 1.0, got None"

#### Scenario: Notification OpaqueBackdrop fails — low opacity
- **WHEN** the notification zone's effective RenderingPolicy has `backdrop = Some(Rgba::BLACK)` and `backdrop_opacity = Some(0.5)`
- **THEN** the readability check MUST fail with `PROFILE_READABILITY_VIOLATION` identifying "OpaqueBackdrop: backdrop_opacity must be >= 0.8, got 0.5"

#### Scenario: Dev build allows readability violation with warning
- **WHEN** the runtime starts with `TZE_HUD_DEV=1` and a referenced subtitle profile has `outline_width = None`
- **THEN** the runtime MUST log a WARN with the readability violation details and MUST load the profile without failing startup

---

### Requirement: Widget SVG Readability Conventions
Widget SVG bundles that are part of a component profile with a text-bearing component type (`DualLayer` or `OpaqueBackdrop` readability) and that contain visible text elements MUST follow these structural conventions:

1. **Text elements MUST use `data-role="text"` attribute.** This identifies text elements for the validator. Elements with `data-role="text"` are subject to readability checks.
2. **Backdrop elements MUST use `data-role="backdrop"` attribute.** This identifies backdrop rectangles for the validator.
3. **Backdrop elements MUST precede their associated text elements in SVG document order.** SVG uses the painter's model (later elements paint over earlier ones); backdrops must be earlier in document order so text renders on top.
4. **Text elements in `DualLayer` profiles MUST have both `fill` and `stroke` attributes.** The `stroke` attribute provides the outline. `stroke-width` MUST be present and its value (after token resolution) MUST be >= 1.0.
5. **Text elements in `OpaqueBackdrop` profiles MUST have a `fill` attribute.** Stroke/outline is optional.

Elements without `data-role` attributes are not subject to readability validation. This allows decorative SVG elements (borders, dividers, icons) to coexist without triggering false positives.

Validation occurs at bundle load time after token placeholder resolution. Violations MUST produce `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION`. In development builds, violations are logged as WARN and the bundle is loaded.

Widgets that are NOT part of a component profile (global widget bundles without `component_type` in their manifest) are NOT subject to readability convention checks — they are generic parameterized visuals with no readability contract.
Scope: v1-mandatory

#### Scenario: Well-formed text SVG passes validation
- **WHEN** a widget SVG in a subtitle profile contains `<rect data-role="backdrop" fill="#000000" opacity="0.6" .../>` followed by `<text data-role="text" fill="#FFFFFF" stroke="#000000" stroke-width="2" ...>Subtitle</text>`
- **THEN** the bundle MUST pass readability convention validation

#### Scenario: Text without data-role skips validation
- **WHEN** a widget SVG contains `<text fill="#FF0000">decorative label</text>` without `data-role="text"`
- **THEN** the element MUST NOT be subject to readability checks (it is treated as decoration)

#### Scenario: Backdrop after text rejected
- **WHEN** a widget SVG contains `<text data-role="text" .../>` followed by `<rect data-role="backdrop" .../>`
- **THEN** the bundle MUST be rejected with `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION` because SVG paints in document order — the backdrop would paint over the text

#### Scenario: Text without stroke in DualLayer profile
- **WHEN** a widget SVG in a subtitle profile contains `<text data-role="text" fill="#FFFFFF">` with no `stroke` attribute
- **THEN** the bundle MUST be rejected with `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION` because subtitle requires DualLayer (stroke/outline mandatory on text)

#### Scenario: Text without stroke in OpaqueBackdrop profile passes
- **WHEN** a widget SVG in a notification profile contains `<text data-role="text" fill="#FFFFFF">` with no `stroke` attribute but a preceding `<rect data-role="backdrop" opacity="0.9" .../>`
- **THEN** the bundle MUST pass readability convention validation — OpaqueBackdrop does not require text stroke

#### Scenario: Global widget bundle not subject to readability checks
- **WHEN** a global widget bundle (not in any profile directory) has `component_type = "subtitle"` but its SVGs lack `data-role` attributes
- **THEN** the bundle MUST NOT be subject to readability convention validation — readability enforcement applies only to profile-scoped bundles

---

### Requirement: Profile-Scoped Token Resolution
When a component profile declares `[token_overrides]`, those overrides MUST apply exclusively to the profile's own widget SVG templates and zone rendering overrides. Token resolution order MUST be: (1) profile `[token_overrides]` (highest priority), (2) global `[design_tokens]` from configuration, (3) canonical fallback values (lowest priority). This resolution MUST happen at load time — the resolved concrete values are baked into the profile's SVG templates and rendering policy fields. No runtime re-resolution. Profile token overrides MUST NOT affect other profiles, global widget bundles, or the default zone rendering for component types without an active profile.
Scope: v1-mandatory

#### Scenario: Profile override takes precedence
- **WHEN** a profile declares `[token_overrides]` with `"color.text.primary" = "#FF0000"` and global tokens set `color.text.primary = "#FFFFFF"`
- **THEN** the profile's SVG placeholders and zone rendering override fields referencing `color.text.primary` MUST resolve to `"#FF0000"`

#### Scenario: Unoverridden tokens fall through to global
- **WHEN** a profile does not override `typography.subtitle.size` but global tokens set it to `"32"`
- **THEN** the profile's references to `typography.subtitle.size` MUST resolve to `"32"`

#### Scenario: Profile override does not leak to other profiles
- **WHEN** profile A overrides `color.text.primary = "#FF0000"` and profile B does not override it
- **THEN** profile B's token references MUST resolve `color.text.primary` from global tokens, not from profile A's override

#### Scenario: Profile override does not affect default rendering
- **WHEN** profile A (for subtitle) overrides `color.text.primary = "#FF0000"`, and the notification component type has no active profile
- **THEN** the notification zone's default rendering MUST use the global/canonical `color.text.primary` value, not profile A's override

---

### Requirement: Profile Validation at Startup
The runtime MUST validate all loaded component profiles at startup. Validation MUST execute in the following order:
1. **Manifest validation:** `profile.toml` has all required fields, `component_type` references a known v1 type, `name` is kebab-case and unique.
2. **Token resolution:** Profile-scoped token map is constructed (profile overrides merged with global tokens and canonical fallbacks). All required tokens for the component type are resolvable.
3. **Zone override validation:** Zone override files reference only zone types governed by the profile's component type. Override field values are valid types and ranges. Token references in override fields are resolvable.
4. **Widget bundle validation:** Profile-scoped widget bundles are loaded with the profile's scoped token map. SVG placeholders are resolved. SVGs parse after resolution.
5. **Readability validation:** Zone rendering readability (RenderingPolicy field checks) and widget SVG readability (structural checks) per the component type's readability technique.

Validation failures MUST reject the profile with a specific error code and log the failure. Rejected profiles MUST NOT prevent startup if no `[component_profiles]` entry references them. If a `[component_profiles]` entry references a rejected profile, startup MUST fail with the profile's validation error.
Scope: v1-mandatory

#### Scenario: All validations pass
- **WHEN** a profile passes manifest, token, zone override, widget bundle, and readability validation
- **THEN** the profile MUST be registered as available for selection

#### Scenario: Unreferenced rejected profile allows startup
- **WHEN** a profile fails validation but no `[component_profiles]` entry references it
- **THEN** the runtime MUST log the failure and continue startup; the profile is unavailable but does not block the system

#### Scenario: Referenced rejected profile blocks startup
- **WHEN** a profile fails validation and a `[component_profiles]` entry references it
- **THEN** startup MUST fail with the profile's validation error and the configuration entry that references it

#### Scenario: Validation order catches manifest errors before token resolution
- **WHEN** a profile has `component_type = "hologram"` (unknown) and also has unresolvable token references in zone overrides
- **THEN** the profile MUST be rejected with `PROFILE_UNKNOWN_COMPONENT_TYPE` (step 1) — it MUST NOT proceed to token resolution (step 2)

---

### Requirement: Startup Sequence Integration
The component shape language system MUST integrate into the runtime startup sequence in the following order relative to existing startup phases:

1. **Configuration parsing** (existing) — parse TOML, validate schema
2. **Design token loading** (NEW) — parse `[design_tokens]`, merge with canonical fallbacks to produce the global token map
3. **Global widget bundle loading** (existing, MODIFIED) — scan `[widget_bundles].paths`, resolve `{{token}}` placeholders in SVGs against global token map, validate SVGs
4. **Component profile loading** (NEW) — scan `[component_profile_bundles].paths`, parse `profile.toml` manifests, construct per-profile scoped token maps, load profile widget bundles (with scoped tokens), parse zone rendering overrides (with scoped tokens)
5. **Component profile selection** (NEW) — resolve `[component_profiles]` entries to loaded profiles, validate type matches
6. **Default zone rendering policy construction** (NEW) — for each built-in zone type, construct effective `RenderingPolicy` by: start with zone type defaults → apply token-derived defaults → apply active profile zone overrides (if any)
7. **Readability validation** (NEW) — validate effective RenderingPolicy per component type readability technique, validate profile widget SVGs per readability conventions
8. **Zone registry construction** (existing, MODIFIED) — build `ZoneRegistry` with effective `RenderingPolicy` per zone type
9. **Widget registry construction** (existing, MODIFIED) — register global + profile-scoped widgets
10. **Session establishment** (existing) — scene snapshot includes token-resolved rendering policies

Token loading (step 2) MUST complete before any SVG placeholder resolution (steps 3–4). Profile loading (step 4) MUST complete before profile selection (step 5). Selection (step 5) MUST complete before effective policy construction (step 6). Readability validation (step 7) MUST use the final effective policies.
Scope: v1-mandatory

#### Scenario: Token loaded before SVG resolution
- **WHEN** the runtime starts with `[design_tokens]` containing `"color.text.primary" = "#00FF00"` and a widget SVG containing `fill="{{color.text.primary}}"`
- **THEN** the token map MUST be fully constructed before the widget bundle loader resolves SVG placeholders, producing `fill="#00FF00"`

#### Scenario: Profile loaded before selection
- **WHEN** `[component_profiles]` references profile `"my-subtitles"` and the profile directory is in `[component_profile_bundles].paths`
- **THEN** all profiles in configured paths MUST be loaded and validated before the runtime attempts to resolve `"my-subtitles"` from the loaded profile set

#### Scenario: Effective policy constructed after selection
- **WHEN** profile `"my-subtitles"` is selected for subtitle and its `zones/subtitle.toml` overrides `backdrop_opacity = 0.8`
- **THEN** the effective RenderingPolicy for the subtitle zone MUST include `backdrop_opacity = 0.8` merged on top of the token-derived default — this MUST happen before readability validation

---

### Requirement: Error Code Catalog
The following error codes are introduced by the component shape language system. Each code MUST be a unique, stable string identifier used in structured errors. Error codes MUST NOT be reused across different failure conditions.

**Token errors:**
- `TOKEN_VALUE_PARSE_ERROR` — a token value failed to parse in its expected format (color, numeric, font family). Includes: token key, expected format, actual value.
- `CONFIG_INVALID_TOKEN_KEY` — a `[design_tokens]` key does not match the required pattern. Includes: the invalid key.

**Profile errors:**
- `PROFILE_UNKNOWN_COMPONENT_TYPE` — a profile's `component_type` does not match any v1 component type. Includes: profile name, declared component type.
- `PROFILE_READABILITY_VIOLATION` — a profile's effective rendering fails the component type's readability check. Includes: profile name, component type, failing check description, actual field values.
- `PROFILE_ZONE_OVERRIDE_MISMATCH` — a profile contains a zone override file for a zone type not governed by the profile's component type. Includes: profile name, zone type, component type.
- `PROFILE_INVALID_ZONE_OVERRIDE` — a zone override field has an invalid value or type. Includes: profile name, zone type, field name, expected type/values, actual value.
- `PROFILE_UNRESOLVED_TOKEN` — a `{{token.key}}` reference in a zone override field could not be resolved. Includes: profile name, zone type, field name, token key.

**Configuration errors:**
- `CONFIG_PROFILE_PATH_NOT_FOUND` — a configured profile bundle path does not exist. Includes: the path.
- `CONFIG_PROFILE_DUPLICATE_NAME` — two profile directories declare the same profile name. Includes: name, both paths.
- `CONFIG_UNKNOWN_COMPONENT_TYPE` — a `[component_profiles]` key is not a recognized component type name. Includes: the key.
- `CONFIG_UNKNOWN_COMPONENT_PROFILE` — a `[component_profiles]` value does not match any loaded profile. Includes: the profile name.
- `CONFIG_PROFILE_TYPE_MISMATCH` — a `[component_profiles]` entry maps a component type to a profile of a different component type. Includes: component type key, profile name, profile's actual component type.

**Widget bundle errors (additions to existing catalog):**
- `WIDGET_BUNDLE_UNRESOLVED_TOKEN` — an SVG contains a `{{token.key}}` placeholder that cannot be resolved. Includes: bundle path, SVG file, token key.
- `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION` — an SVG in a profile-scoped bundle violates readability conventions. Includes: bundle path, SVG file, violation description.
Scope: v1-mandatory

#### Scenario: Error code uniqueness
- **WHEN** any error in the component shape language system is produced
- **THEN** its error code MUST be one of the codes listed in this catalog, and the code MUST uniquely identify the failure category

#### Scenario: Error includes diagnostic context
- **WHEN** `PROFILE_READABILITY_VIOLATION` is produced for a subtitle profile missing outline
- **THEN** the error MUST include the profile name, the component type "subtitle", the check "DualLayer", and the failing condition "outline_width must be >= 1.0, got None"

---

### Requirement: Profile Widget Scope
Widget bundles inside a component profile's `widgets/` directory are **compositor-internal assets** — they are used by the runtime to enhance the visual rendering of the governed zone, not as agent-facing publishing surfaces. Profile-scoped widgets are registered in the `WidgetRegistry` with namespaced names (`"{profile_name}/{widget_name}"`) for internal tracking and scene snapshot inclusion, but agents SHOULD NOT publish to them. Profile widgets are driven by the compositor's zone rendering logic, which may create and update internal widget instances to enhance zone visuals (e.g., a subtitle profile might use a widget for a styled word-highlight overlay that the compositor updates in sync with subtitle stream-text publications). If an agent publishes to a profile-scoped widget by name, the publish MUST be accepted (it is a valid widget instance) but the compositor's internal updates may overwrite the agent's values on the next zone publication. This is not a conflict — it is the expected behavior: profile widgets serve the zone rendering, not agent authoring.
Scope: v1-mandatory

#### Scenario: Profile widget registered but compositor-driven
- **WHEN** profile `"my-subtitles"` contains `widgets/highlight/widget.toml` and the profile is activated
- **THEN** the widget MUST be registered as `"my-subtitles/highlight"` in the WidgetRegistry and the compositor MAY create an internal widget instance bound to the subtitle zone's tab

#### Scenario: Agent publish to profile widget accepted but overwritten
- **WHEN** an agent publishes parameters to `"my-subtitles/highlight"` and the compositor subsequently updates the widget based on a new subtitle publication
- **THEN** the agent's publication MUST be accepted, but the compositor's update replaces it — this is expected behavior

---

### Requirement: Hot-Reload Classification
The `[design_tokens]`, `[component_profile_bundles]`, and `[component_profiles]` configuration sections MUST be classified as **frozen** (not hot-reloadable). These sections are resolved at startup and their values are baked into RenderingPolicy fields, SVG templates, and zone/widget registries. Changing them requires a runtime restart. The `HotReloadableConfig` struct and `section_classification()` function in `tze_hud_config::reload` MUST explicitly exclude these sections from the hot-reloadable set. A SIGHUP-triggered config reload that detects changes in frozen sections MUST log a WARN indicating that a restart is required for the changes to take effect — it MUST NOT apply partial updates.
Scope: v1-mandatory

#### Scenario: Token change on reload produces warning
- **WHEN** the runtime receives SIGHUP and the reloaded config file has a different `[design_tokens]` section
- **THEN** the runtime MUST log a WARN: "design_tokens changed but requires restart to take effect" and MUST NOT modify the active token map

#### Scenario: Profile change on reload produces warning
- **WHEN** the runtime receives SIGHUP and the reloaded config file has a different `[component_profiles]` section
- **THEN** the runtime MUST log a WARN and MUST NOT modify the active profile selection

---

### Requirement: Zone Name Reconciliation
Profile authors MUST use zone registry names (not config constant names) when naming zone override files. The codebase has a pre-existing inconsistency between zone names in the zone registry (`with_defaults()`) and the config validation constants (`BUILTIN_ZONE_TYPES`):

| Zone Registry Name | Config Constant | Component Type Name |
|---|---|---|
| `"status-bar"` | `"status_bar"` | `status-bar` |
| `"notification-area"` | `"notification"` | `notification` |
| `"subtitle"` | `"subtitle"` | `subtitle` |
| `"pip"` | `"pip"` | `pip` |
| `"ambient-background"` | `"ambient_background"` | `ambient-background` |
| `"alert-banner"` | `"alert_banner"` | `alert-banner` |

Component type names in this spec use kebab-case (matching zone registry names). The `[component_profiles]` configuration section accepts component type names in kebab-case. Zone rendering override files inside profiles (`zones/{name}.toml`) MUST use the zone registry name (e.g., `zones/notification-area.toml`, NOT `zones/notification.toml`). The config/registry naming discrepancy is a pre-existing issue outside the scope of this change but MUST be documented here so profile authors use the correct names. The runtime MUST accept the zone registry name for override file matching.
Scope: v1-mandatory

#### Scenario: Zone override file uses registry name
- **WHEN** a subtitle profile contains `zones/subtitle.toml`
- **THEN** the override MUST be matched to the `"subtitle"` zone definition (registry name matches)

#### Scenario: Zone override file uses wrong name form
- **WHEN** a notification profile contains `zones/notification.toml` (config constant form) instead of `zones/notification-area.toml` (registry name)
- **THEN** the override MUST NOT match any zone definition and the runtime MUST produce `PROFILE_ZONE_OVERRIDE_MISMATCH`

---

### Requirement: SVG data-role Attribute Convention
The `data-role` attributes (`data-role="backdrop"`, `data-role="text"`) used for readability validation are **custom attributes**, not part of the SVG specification. They are valid XML attributes but are not recognized by SVG renderers. The runtime's readability validator scans SVG elements using `quick-xml` parsing (the same XML scanner used by `tze_hud_widget::svg_ids::collect_svg_element_ids()`) BEFORE the SVG is passed to `resvg` for rasterization. `resvg` ignores unknown attributes, so `data-role` attributes do not affect rendering. The validator MUST use `quick-xml` event-based parsing to find elements with `data-role` attributes — it MUST NOT require `resvg` to understand or preserve these attributes.
Scope: v1-mandatory

#### Scenario: data-role attributes ignored by resvg
- **WHEN** a widget SVG contains `<rect data-role="backdrop" fill="#000000" .../>` and is rasterized by resvg
- **THEN** resvg MUST render the rectangle normally (the `data-role` attribute is ignored); the attribute is only consumed by the readability validator during bundle loading

#### Scenario: Readability validator uses quick-xml, not resvg
- **WHEN** the readability validator scans a widget SVG for `data-role` elements
- **THEN** it MUST use `quick-xml` event-based XML parsing (matching the existing `collect_svg_element_ids()` pattern) to find elements — it MUST NOT depend on resvg's tree representation

---

### Requirement: Notification Urgency-to-Severity Token Mapping
The alert-banner zone renders backdrop colors based on the publication's urgency level (carried in `NotificationPayload.urgency: u32`). The compositor MUST map urgency values to severity color tokens as follows:

| Urgency value | Urgency level | Severity token |
|---|---|---|
| 0 | low | `color.severity.info` |
| 1 | normal | `color.severity.info` |
| 2 | urgent | `color.severity.warning` |
| 3 | critical | `color.severity.critical` |

This mapping applies only to the alert-banner zone type when the published content is `ZoneContent::Notification(payload)`. For all other content types published to alert-banner, the zone's default `backdrop` color is used. The `color.severity.error` token is reserved for a future `AlertPayload` content type that carries an explicit severity enum (post-v1). The notification-area zone does NOT use this mapping — notification urgency tinting is replaced entirely by the RenderingPolicy `backdrop` color from the notification component type's token-derived defaults or profile override.
Scope: v1-mandatory

#### Scenario: Alert-banner urgency 2 maps to warning color
- **WHEN** a `NotificationPayload` with `urgency = 2` is published to the alert-banner zone and `color.severity.warning` resolves to `"#FFB800"`
- **THEN** the compositor MUST render the alert-banner backdrop with Rgba from `"#FFB800"`

#### Scenario: Alert-banner urgency 3 maps to critical color
- **WHEN** a `NotificationPayload` with `urgency = 3` is published to the alert-banner zone and `color.severity.critical` resolves to `"#FF0000"`
- **THEN** the compositor MUST render the alert-banner backdrop with Rgba from `"#FF0000"`

#### Scenario: Alert-banner non-notification content uses default backdrop
- **WHEN** a `ZoneContent::StreamText` is published to the alert-banner zone
- **THEN** the compositor MUST use the zone's default `backdrop` color from RenderingPolicy (not severity mapping)

#### Scenario: Notification-area zone does NOT use urgency-to-severity mapping
- **WHEN** a `NotificationPayload` with `urgency = 3` is published to the notification-area zone
- **THEN** the compositor MUST use the notification-area zone's RenderingPolicy `backdrop` color — it MUST NOT apply urgency-to-severity color mapping (that is alert-banner-specific)

### Requirement: Text-Portal Component Type
The `text-portal` component type defines the visual-semantic identity of the first-class text stream portal surface that RFC 0013 §7.2 promotion permits. It is the promotion-era successor to the Phase-0 raw-tile portal assembly, satisfying the Phase-1 contract that the portal "be styled through a `text-portal` component type contract defined by a separate component-shape-language delta, consuming `RenderingPolicy` fields like other component types." The component type MUST declare:

- **Name:** `text-portal` (kebab-case).
- **Governed surface:** the first-class portal surface (or node type) introduced by the RFC 0013 §7.2 promotion, expressed as the named parts in the Text-Portal Surface Part Model requirement — NOT a single zone type. Until the promotion surface exists, the same contract is expressible on the raw-tile pilot per RFC 0013 §7, and this component type is inert.
- **Readability technique:** declared **per text-bearing part** (see Text-Portal Readability Enforcement). The surface-level default for text-bearing parts is `OpaqueBackdrop`; geometry-only parts are `None`.
- **Required tokens** (all drawn from the existing Canonical Token Schema — this delta introduces NO new canonical token key): `color.text.primary`, `color.text.secondary`, `color.backdrop.default`, `color.border.default`, `color.outline.default`, `opacity.backdrop.opaque`, `typography.heading.family`, `typography.heading.size`, `typography.heading.weight`, `typography.body.family`, `typography.body.size`, `typography.body.weight`, `spacing.padding.medium`, `stroke.border.width`, `stroke.outline.width`.
- **Geometry expectation (informational):** a content-layer, lease-governed, movable and resizable two-pane surface (transcript pane + composer pane) with a header band and a collapsed-card state; corner/edge resize affordances and a pane divider. Geometry is governed by the portal surface's own bounds/lease, not validated against the component type.

This `text-portal` contract's required-tokens list reuses existing canonical keys verbatim and introduces no new canonical key. Portal-specific token **canonicalization** was subsequently delivered by the follow-up change (`hud-8691s`, Promotion P2), which shipped a profile-scoped `portal.*` token namespace of 59 keys (`crates/tze_hud_config/src/portal_tokens.rs` — the `PORTAL_TOKEN_*` consts and their `PORTAL_TOKEN_DEFAULT_STRINGS` defaults), resolved via profile-scoped overrides with canonical-token fallbacks. The required-tokens list above intentionally stays on the pre-existing canonical keys so the promotion-era styling contract is expressible independently of the `portal.*` namespace.

**Runtime registration status (wired, hud-m4xay / reconciliation F4):** the runtime loader now recognizes `text-portal` — it is present in `ComponentType::ALL` / `ComponentType::from_name` (`crates/tze_hud_config/src/component_types.rs`), so a conforming `component_type = "text-portal"` profile loads like any v1 profile, and the per-part `OpaqueBackdrop`/`None` readability enforcement below has a runtime validation path in `run_component_startup` step 7 (`crates/tze_hud_runtime/src/component_startup.rs`, `enforce_text_portal_readability`), which resolves each text-bearing part's effective backdrop from `color.backdrop.default` + `opacity.backdrop.opaque` and hard-errors (dev-mode WARN) on a translucent backdrop, exactly as the zone readability loop does. The per-part technique split is sourced from `PortalPartKind::is_text_bearing` via `ComponentType::text_portal_part_readability`. This realizes the "recognized after the RFC 0013 §7.2 gate" clause (reconciliation finding F4, `docs/reports/promotion-epic-reconciliation-20260711.md`).
Scope: post-v1

#### Scenario: text-portal declares its governed surface and required tokens
- **WHEN** the `text-portal` component type is defined
- **THEN** it MUST declare that it governs the first-class portal surface expressed as named parts (not a single zone type)
- **AND** every required token key it lists MUST already exist in the Canonical Token Schema, with no new canonical key introduced by this delta

#### Scenario: text-portal does not alter the six v1 component types
- **WHEN** the runtime resolves component types with `text-portal` defined
- **THEN** the six v1 component types (`subtitle`, `notification`, `status-bar`, `alert-banner`, `ambient-background`, `pip`) MUST remain defined exactly as before
- **AND** `text-portal` MUST be recognized only after its RFC 0013 §7.2 promotion gate passes

#### Scenario: portal token canonicalization delivered by P2
- **WHEN** an author asks whether this delta introduces a portal-namespaced canonical token key (e.g. a `portal.*` namespace)
- **THEN** this delta MUST introduce no new canonical key — the `text-portal` required-tokens list references only pre-existing canonical keys
- **AND** portal-specific canonicalization was delivered by the P2 follow-up change `hud-8691s`, which shipped a 59-key profile-scoped `portal.*` token namespace (`crates/tze_hud_config/src/portal_tokens.rs`), resolved via profile-scoped overrides with canonical-token fallbacks

---

### Requirement: Text-Portal Surface Part Model
The `text-portal` component type styles a fixed set of named **parts**. The part set MUST enumerate every visual region of the portal surface and MUST cross-map to the Phase-0 raw-tile assembly so that promotion preserves, rather than redefines, the proven layout. The named parts and their raw-tile expression are:

| Part | Phase-0 raw-tile expression | Text-bearing | Readability |
|---|---|---|---|
| `frame` | `frame` tile — surface backdrop, outer border, and footer/status chrome | partial (footer meta) | OpaqueBackdrop |
| `header` | header band within the `frame` tile — title + subtitle; also the move/drag handle | yes | OpaqueBackdrop |
| `composer` | `input_scroll` tile — bounded draft text, caret, and selection | yes | OpaqueBackdrop |
| `transcript` | `output_scroll` tile — markdown-rendered transcript window | yes | OpaqueBackdrop |
| `divider` | vertical divider sub-element of the `frame` tile — pane split and resize handle | no | None |
| `collapsed-card` | `minimized_icon` tile — collapsed/minimized representation | yes | OpaqueBackdrop |
| `capture-backstop` | `capture_backstop` tile — full-bounds input/redaction backstop beneath the surface | no | None |
| `gesture-shield` | `drag_shield` tile — transient move/resize gesture capture; hosts the scroll-position indicator | no | None |

Every part MUST be styled exclusively from resolved design-token values (via `RenderingPolicy` for text-bearing and backdrop styling, and via the existing border-token pattern for non-text strokes); no part may carry hardcoded compositor colors, typography, spacing, or strokes. The eight parts MUST account for all six Phase-0 portal tiles (`capture_backstop`, `frame`, `input_scroll`, `output_scroll`, `drag_shield`, `minimized_icon`) plus the frame-internal `divider` sub-element. This part model adds no new portal capability: it is the styling decomposition of the surface that RFC 0013 §7.2 promotes, and it preserves every standing portal non-goal (see Text-Portal Profile Styling and Promotion Scope Boundary).
Scope: post-v1

#### Scenario: every portal part is enumerated and token-styled
- **WHEN** the `text-portal` component type is rendered
- **THEN** the `frame`, `header`, `composer`, `transcript`, `divider`, `collapsed-card`, `capture-backstop`, and `gesture-shield` parts MUST each resolve their styling from design tokens
- **AND** no part MUST carry a hardcoded compositor color, typography, spacing, or stroke value

#### Scenario: part model cross-maps to the six-tile raw assembly
- **WHEN** the named parts are mapped to the Phase-0 raw-tile assembly
- **THEN** the six tiles `capture_backstop`, `frame`, `input_scroll`, `output_scroll`, `drag_shield`, and `minimized_icon` MUST each be covered by exactly one or more named parts
- **AND** the `divider` part MUST map to the frame-internal pane-split/resize sub-element of the `frame` tile

#### Scenario: geometry-only parts carry no text styling
- **WHEN** the `capture-backstop`, `gesture-shield`, or `divider` part is styled
- **THEN** it MUST consume only backdrop/border/indicator styling (no text color or typography)
- **AND** its readability technique MUST be `None`

---

### Requirement: Text-Portal Part RenderingPolicy Consumption
Each text-bearing or backdrop-bearing part of the `text-portal` surface MUST consume `RenderingPolicy` fields (from the Extended RenderingPolicy requirement) exactly as zones do — the portal is NOT exempt from the "never hardcode visuals" rule. The per-part consumption is:

- **`frame`:** `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`. The outer border uses `color.border.default` + `stroke.border.width` via the same compositor border-token pattern the `notification` component type uses (rendered as edge quads), since `RenderingPolicy` outline fields style text, not frame chrome. Footer/status text consumes `text_color` ← `color.text.secondary`, `font_family`/`font_size_px` ← `typography.body.*`.
- **`header`:** `text_color` ← `color.text.primary`; `font_family`/`font_size_px`/`font_weight` ← `typography.heading.*` for the title and `typography.body.*` for the subtitle; `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`; `margin_horizontal`/`margin_vertical` ← `spacing.padding.medium`.
- **`composer`:** `text_color` ← `color.text.primary`; `font_family`/`font_size_px`/`font_weight` ← `typography.body.*`; `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`; `margin_horizontal`/`margin_vertical` ← `spacing.padding.medium`. The caret and selection are geometry rendered locally per the input model; their color follows `text_color`.
- **`transcript`:** `text_color` ← `color.text.primary`; `font_family`/`font_size_px`/`font_weight` ← `typography.body.*`; `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`; `margin_horizontal`/`margin_vertical` ← `spacing.padding.medium`. The Phase-1 markdown subset styling (heading scale, emphasis, inline/fenced code, list indentation, link treatment) resolves from the same token set per the Phase-1 Markdown Rendering Subset requirement and MUST NOT introduce hardcoded styling.
- **`collapsed-card`:** `text_color` ← `color.text.secondary`; `font_family`/`font_size_px` ← `typography.body.*`; `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`; border via `color.border.default` + `stroke.border.width`.
- **`divider` / `gesture-shield` scroll-position indicator:** stroke/fill from `color.border.default` (or `color.text.secondary` for the indicator) + `stroke.border.width`; geometry only.
- **`capture-backstop`:** when the viewer's policy redacts the portal, this part renders the neutral redaction treatment from `color.backdrop.default` + `opacity.backdrop.opaque`; it carries no text.

Collapsed↔expanded state transitions MUST use `RenderingPolicy.transition_in_ms` / `transition_out_ms` built on the existing zone-transition opacity mechanics; the durations are token-derived once portal token keys are canonicalized (P2), and until then use the existing zone-transition defaults. A transition MUST NOT reveal transcript content past the active redaction policy at any frame.
Scope: post-v1

#### Scenario: header and transcript consume RenderingPolicy text fields
- **WHEN** the `header` and `transcript` parts render text
- **THEN** each MUST construct its text from `RenderingPolicy` fields (`text_color`, `font_family`, `font_size_px`, `font_weight`) resolved from the listed canonical tokens
- **AND** neither part MUST read a hardcoded font, size, weight, or color

#### Scenario: frame border follows the existing border-token pattern
- **WHEN** the `frame` part renders its outer border
- **THEN** the border color MUST come from `color.border.default` and the border width from `stroke.border.width`, rendered as edge quads in the same manner as the `notification` component type
- **AND** the `frame` part's surface backdrop MUST come from `RenderingPolicy.backdrop` / `backdrop_opacity`

#### Scenario: collapsed-to-expanded transition is opacity-only and redaction-safe
- **WHEN** a collapsed `text-portal` expands for a viewer not permitted to see its transcript
- **THEN** the transition MUST animate via `RenderingPolicy.transition_in_ms` on the existing zone-transition opacity mechanics
- **AND** every frame of the transition MUST show the neutral redaction treatment in place of transcript content

---

### Requirement: Text-Portal Readability Enforcement
The text-bearing parts of the `text-portal` surface (`header`, `composer`, `transcript`, `collapsed-card`, and the `frame` footer text) MUST enforce readability through validation of each part's effective `RenderingPolicy` at startup, exactly as the Zone Readability Enforcement requirement validates zones. Each text-bearing part's required technique is `OpaqueBackdrop`: the effective `RenderingPolicy` MUST satisfy `backdrop` is `Some(color)` (not fully transparent) and `backdrop_opacity` is `Some(v)` where `v >= 0.8`. The geometry-only parts (`capture-backstop`, `gesture-shield`, `divider`) require `None`. A profile MAY additionally apply `outline_color` + `outline_width` to a text-bearing part for extra legibility, but outline is not required for `text-portal`. Failure MUST produce `PROFILE_READABILITY_VIOLATION` identifying the component type `text-portal`, the failing part, the failing check, and the actual field values. Development-build relaxation (WARN instead of hard error under `profile = "headless"` or `TZE_HUD_DEV=1`) applies to `text-portal` exactly as it does to the v1 component types.
Scope: post-v1

#### Scenario: opaque transcript pane passes readability
- **WHEN** the `transcript` part's effective `RenderingPolicy` has `backdrop = Some(color.backdrop.default)` and `backdrop_opacity = Some(0.9)`
- **THEN** the `OpaqueBackdrop` readability check for that part MUST pass

#### Scenario: translucent composer fails readability
- **WHEN** the `composer` part's effective `RenderingPolicy` has `backdrop = Some(color)` but `backdrop_opacity = Some(0.5)`
- **THEN** the readability check MUST fail with `PROFILE_READABILITY_VIOLATION` identifying component type `text-portal`, the `composer` part, and "OpaqueBackdrop: backdrop_opacity must be >= 0.8, got 0.5"

#### Scenario: geometry-only part skips readability
- **WHEN** the `capture-backstop` or `gesture-shield` part is validated
- **THEN** no readability check MUST be performed for that part (technique `None`)

---

### Requirement: Text-Portal Profile Styling and Promotion Scope Boundary
A component profile with `component_type = "text-portal"` MUST be able to reskin the portal surface — every named part — purely through `[token_overrides]` and zone-style rendering overrides, without changing adapter logic or runtime behavior, consistent with the Component Profile Format and Zone Rendering Override Schema requirements. The text-portal profile MUST follow the same loading, scoped-token-resolution, and validation rules as v1 component-type profiles.

Defining the `text-portal` component type MUST NOT change any standing portal non-goal established by RFC 0013 §7.2 and the text-stream-portals Promotion Scope Boundary. Specifically, this component type MUST NOT add or imply: terminal emulation (VT100/ANSI cursor addressing, alternate screen, PTY hosting), full transcript history materialized in the scene graph, chrome-layer portal UI, a dedicated portal transport or a second long-lived portal stream, or runtime ownership of external process lifecycles. The styled surface MUST remain lease-governed, content-layer, redactable, and subordinate to the attention model exactly as the raw-tile pilot is. This delta is a styling-contract decomposition only; it grants no new runtime capability beyond what the RFC 0013 §7.2 promotion already permits.
Scope: post-v1

#### Scenario: profile reskins the portal without code changes
- **WHEN** an operator activates a `text-portal` profile whose `[token_overrides]` change portal-relevant colors, typography, spacing, and strokes
- **THEN** the `frame`, `header`, `composer`, `transcript`, `divider`, and `collapsed-card` parts MUST reflect the new token values on re-render
- **AND** no adapter logic or runtime behavior MUST change to achieve the reskin

#### Scenario: component type adds no excluded-scope capability
- **WHEN** the `text-portal` component type is reviewed against the portal non-goals
- **THEN** it MUST NOT introduce terminal emulation, scene-graph transcript history, chrome-layer portal UI, a dedicated portal transport or second portal stream, or runtime process ownership
- **AND** the styled surface MUST remain lease-governed, content-layer, redactable, and attention-subordinate

