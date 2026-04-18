## 1. Design Token System

- [ ] 1.1 Define `DesignTokenMap` type (`HashMap<String, String>`) and canonical token schema with all fallback values in a `tokens` module within `tze_hud_config`
- [ ] 1.2 Parse `[design_tokens]` from TOML into `DesignTokenMap`; validate key format `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*`; emit `CONFIG_INVALID_TOKEN_KEY` on violation
- [ ] 1.3 Implement token value parsing functions: `parse_color_hex(&str) -> Result<Rgba>`, `parse_numeric(&str) -> Result<f32>`, `parse_font_family(&str) -> FontFamily`; emit `TOKEN_VALUE_PARSE_ERROR` on failure
- [ ] 1.4 Implement three-layer token resolution: profile overrides → global config → canonical fallbacks; return scoped `DesignTokenMap` per resolution context
- [ ] 1.5 Add WARN-level logging when `FontFamily::Name(value)` fails to resolve in glyphon's font system
- [ ] 1.6 Write unit tests for token parsing, key validation, value format parsing (color hex 6/8 digit, numeric, font family keywords + named), fallback resolution, override precedence, parse error reporting

## 2. SVG Token Placeholder Resolution

- [ ] 2.1 Implement `{{token.key}}` placeholder scanner on raw SVG strings using regex `\{\{([a-z][a-z0-9]*(?:\.[a-z][a-z0-9_]*)*)\}\}`; single left-to-right pass, no recursive re-scanning
- [ ] 2.2 Implement `\{\{` / `\}\}` escape handling — replace before token scan, restore literals after substitution
- [ ] 2.3 Integrate placeholder resolution into widget bundle loader (`tze_hud_widget::loader`): resolve tokens after reading SVG bytes as string, before `parse_svg_dimensions()` and `collect_svg_element_ids()`; emit `WIDGET_BUNDLE_UNRESOLVED_TOKEN` for missing keys
- [ ] 2.4 Validate post-substitution SVG parseability; emit `WIDGET_BUNDLE_SVG_PARSE_ERROR` if token values produce invalid SVG
- [ ] 2.5 Write unit tests: single placeholder, multiple placeholders in one attribute, escaped braces, unresolved token, no-recursive substitution, placeholder inside style block, whitespace-in-braces rejected

## 3. Extended RenderingPolicy

- [ ] 3.1 Add new fields to `RenderingPolicy` in `tze_hud_scene::types`: `font_family`, `font_weight`, `text_color`, `backdrop_opacity`, `outline_color`, `outline_width`, `margin_horizontal`, `margin_vertical`, `transition_in_ms`, `transition_out_ms` — all `Option<T>` with `#[serde(default)]`
- [ ] 3.2 Add `FontFamily` import/mapping to `tze_hud_scene::types` (or re-export from glyphon)
- [ ] 3.3 Update existing `RenderingPolicy` consumers in `tze_hud_compositor` to handle new fields gracefully (unwrap_or fallbacks for all new Option fields)
- [ ] 3.4 Verify existing serialized `RenderingPolicy` deserializes correctly with new fields as `None` (backward compatibility)
- [ ] 3.5 Write unit tests for serde round-trip with old format (missing fields → None) and new format (all fields populated)

## 4. Default Zone Rendering with Tokens

- [ ] 4.1 Implement token-to-RenderingPolicy mapping for each built-in zone type: subtitle, notification, status_bar, alert_banner (per spec §Default Zone Rendering with Tokens)
- [ ] 4.2 Wire token-derived defaults into zone registry construction at startup — populate `None` RenderingPolicy fields from tokens, preserving explicit config values
- [ ] 4.3 Remove hardcoded per-content-type color branching in `render_zone_content()` — replace with RenderingPolicy field reads
- [ ] 4.4 Write integration test: start runtime with custom `[design_tokens]` colors → verify zone renders with token-derived colors (not hardcoded defaults)

## 5. Zone Text Outline Rendering

- [ ] 5.1 Implement 8-direction text outline in compositor: render text at 8 cardinal+diagonal offsets in `outline_color`, then render fill text on top in `text_color`
- [ ] 5.2 Integrate outline rendering into `render_zone_content()` — only when `outline_width > 0` and `outline_color` is `Some`
- [ ] 5.3 Implement backdrop opacity override: when `backdrop_opacity` is `Some`, use it as the backdrop quad's alpha regardless of `backdrop` color's alpha channel
- [ ] 5.4 Implement transition_in/transition_out opacity animation on zone composite quads (alpha ramp over configured ms duration)
- [ ] 5.5 Write visual regression tests: subtitle with outline, subtitle without outline, notification with opaque backdrop, transition fade-in/fade-out

## 6. Component Type Contracts

- [ ] 6.1 Define `ComponentType` enum with six v1 variants: `Subtitle`, `Notification`, `StatusBar`, `AlertBanner`, `AmbientBackground`, `Pip`
- [ ] 6.2 Define `ReadabilityTechnique` enum: `DualLayer`, `OpaqueBackdrop`, `None`
- [ ] 6.3 Define `ComponentTypeContract` struct: governed zone type name, readability technique, required token keys (Vec<&str>), geometry expectation (informational string)
- [ ] 6.4 Implement static `contract()` method on `ComponentType` returning the full contract per spec §V1 Component Type Definitions
- [ ] 6.5 Write unit tests verifying each component type's contract fields and required token key lists

## 7. Component Profile Loader

- [ ] 7.1 Define `ComponentProfile` struct: name, version, description, component_type, token_overrides (`HashMap<String, String>`), loaded widget bundles, zone rendering overrides
- [ ] 7.2 Define `ZoneRenderingOverride` struct with all overridable fields as `Option<T>` per spec §Zone Rendering Override Schema
- [ ] 7.3 Implement profile directory scanner: find `profile.toml` manifests in configured paths, parse manifests with serde, validate required fields (name, version, component_type)
- [ ] 7.4 Emit `PROFILE_UNKNOWN_COMPONENT_TYPE` for profiles declaring unknown component types
- [ ] 7.5 Emit `CONFIG_PROFILE_DUPLICATE_NAME` for duplicate profile names across directories
- [ ] 7.6 Emit `CONFIG_PROFILE_PATH_NOT_FOUND` for configured paths that don't exist
- [ ] 7.7 Load profile-scoped widget bundles from `widgets/` subdirectory with namespaced names (`"{profile_name}/{widget_name}"`) and profile-scoped token resolution
- [ ] 7.8 Parse zone rendering override TOML files from `zones/` subdirectory; validate field types and ranges; resolve `{{token.key}}` references in override values; emit `PROFILE_ZONE_OVERRIDE_MISMATCH` for overrides on zone types not governed by the profile's component type
- [ ] 7.9 Emit `PROFILE_INVALID_ZONE_OVERRIDE` for invalid field values, `PROFILE_UNRESOLVED_TOKEN` for unresolvable token references
- [ ] 7.10 Write integration tests: valid profile load, missing fields, unknown type, duplicate names, zone override mismatch, token references in overrides

## 8. Readability Validation

- [ ] 8.1 Implement zone readability validator: check effective RenderingPolicy fields against component type readability technique (DualLayer checks backdrop+opacity+outline, OpaqueBackdrop checks backdrop+opacity)
- [ ] 8.2 Implement SVG readability validator for profile-scoped widget bundles: scan for `data-role="backdrop"` and `data-role="text"` elements, verify document order, check stroke attributes on text elements in DualLayer profiles
- [ ] 8.3 Emit `PROFILE_READABILITY_VIOLATION` for zone rendering failures, `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION` for SVG structural failures
- [ ] 8.4 Implement dev-mode gate: when `TZE_HUD_DEV=1` or `profile = "headless"`, log readability violations as WARN instead of hard rejection
- [ ] 8.5 Write unit tests: DualLayer pass/fail (with/without outline, with/without backdrop), OpaqueBackdrop pass/fail (opacity threshold), SVG structural checks (correct/incorrect document order, missing data-role, missing stroke)

## 9. Profile Selection and Activation

- [ ] 9.1 Parse `[component_profile_bundles]` config section with `paths` array; resolve paths relative to config file parent directory
- [ ] 9.2 Parse `[component_profiles]` config section; validate keys against v1 component type names, validate values against loaded profile names
- [ ] 9.3 Emit `CONFIG_UNKNOWN_COMPONENT_TYPE`, `CONFIG_UNKNOWN_COMPONENT_PROFILE`, `CONFIG_PROFILE_TYPE_MISMATCH` for respective violations
- [ ] 9.4 Construct effective RenderingPolicy per zone: token-derived defaults → active profile zone overrides merged on top
- [ ] 9.5 Register profile-scoped widgets in WidgetRegistry with namespaced names
- [ ] 9.6 Wire effective rendering policies into ZoneRegistry at zone registration time
- [ ] 9.7 Write integration tests: profile selection valid, unknown profile, type mismatch, absent section defaults, effective policy construction with and without profile active

## 10. Startup Sequence Integration

- [ ] 10.1 Add design token loading phase to startup: after config parse, before widget bundle loading
- [ ] 10.2 Wire global token map into existing widget bundle loader (pass token map to `scan_bundle_dirs`)
- [ ] 10.3 Add profile loading phase: after token and widget bundle loading, before zone/widget registry construction
- [ ] 10.4 Add profile selection and effective policy construction phase: after profile loading, before readability validation
- [ ] 10.5 Add readability validation phase: after effective policy construction, before zone registry construction
- [ ] 10.6 Update startup validation order to match spec sequence: tokens → global bundles → profiles → selection → effective policies → readability → registries
- [ ] 10.7 Write end-to-end startup test: config with tokens + profiles + widget bundles → runtime initializes with correct rendering policies → readability passes

## 11. Alert Banner Urgency-to-Severity Mapping

- [ ] 11.1 Implement urgency-to-severity-token mapping in compositor for alert-banner zone: urgency 0,1 → `color.severity.info`, urgency 2 → `color.severity.warning`, urgency 3 → `color.severity.critical`
- [ ] 11.2 Apply mapping only when content is `ZoneContent::Notification(payload)` on alert-banner zone; use default `backdrop` color for other content types
- [ ] 11.3 Ensure notification-area zone does NOT use urgency-to-severity mapping — it uses its RenderingPolicy `backdrop` color directly
- [ ] 11.4 Write unit test: alert-banner with each urgency level renders with correct severity token color; notification-area with same urgency uses its own backdrop

## 12. Protobuf Wire Format Extension

- [ ] 12.1 Add proto fields 5–14 to `RenderingPolicyProto` in `types.proto` for the 10 new RenderingPolicy fields
- [ ] 12.2 Update `rendering_policy_to_proto` and `proto_to_rendering_policy` in `convert.rs` with sentinel value handling for new fields
- [ ] 12.3 Run protobuf codegen and verify compilation
- [ ] 12.4 Update `roundtrip.rs` test to cover new fields (serialization round-trip with all fields populated AND with all fields None)
- [ ] 12.5 Verify backward compatibility: deserialize a snapshot from the pre-extension format → new fields must be None

## 13. Hot-Reload Classification

- [ ] 13.1 Add `design_tokens`, `component_profile_bundles`, `component_profiles` to frozen-field classification in `tze_hud_config::reload::section_classification()`
- [ ] 13.2 Update `HotReloadableConfig` to NOT include the new sections
- [ ] 13.3 Emit WARN on SIGHUP reload when frozen sections have changed: "requires restart to take effect"
- [ ] 13.4 Write unit test: verify section_classification returns Frozen for the three new sections

## 14. RawConfig Extension

- [ ] 14.1 Add `RawDesignTokens` struct (newtype over `HashMap<String, String>`) and `design_tokens: Option<RawDesignTokens>` field to `RawConfig`
- [ ] 14.2 Add `RawComponentProfileBundles` struct with `paths: Vec<String>` field and add to `RawConfig`
- [ ] 14.3 Add `RawComponentProfiles` struct (newtype over `HashMap<String, String>` mapping component type → profile name) and add to `RawConfig`
- [ ] 14.4 Update JSON schema export (`schema.rs`) to include the three new config sections

## 15. Reference Implementations and Documentation

- [ ] 15.1 Author a reference `subtitle` component profile: `profile.toml`, zone override `zones/subtitle.toml` with token references, optional widget bundle with backdrop + outlined text SVG using token placeholders and `data-role` attributes
- [ ] 15.2 Author a reference `notification` component profile: opaque backdrop, border, token-based styling
- [ ] 15.3 Write component profile authoring guide: directory structure, profile.toml schema, SVG conventions (data-role attributes, document order, stroke requirements), token reference syntax, readability requirements per component type, dev vs production validation modes, zone override file naming (must use zone registry names like `notification-area.toml`)
- [ ] 15.4 Add example configuration showing `[design_tokens]`, `[component_profile_bundles]`, and `[component_profiles]` sections working together
