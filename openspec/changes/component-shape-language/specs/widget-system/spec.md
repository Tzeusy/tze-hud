# Widget System Specification (Component Shape Language Delta)

Domain: PRESENCE
Source: Proposal (component-shape-language change)
Depends on: component-shape-language, configuration

---

## MODIFIED Requirements

### Requirement: Widget Asset Bundle Format
Widget type definitions MUST be loaded from asset bundles — directories containing a `widget.toml` manifest and one or more SVG files. The manifest MUST declare: `name` (kebab-case matching `[a-z][a-z0-9-]*`, unique across all loaded bundles), `version` (semver string), `description` (human-readable string), `parameter_schema` (array of parameter declarations, each with name, type, default, and optional constraints), `layers` (ordered array of SVG layer references with parameter bindings), and optional `default_geometry` (Rect), `default_rendering_policy`, `default_contention_policy`. The manifest MAY declare an optional `component_type` field (string, references a v1 component type name) indicating that this widget bundle is designed to serve as part of a component profile. The `component_type` field is informational for standalone (global) bundles — it does NOT trigger readability validation on global bundles. Readability convention validation only applies to widget bundles loaded as part of a component profile directory (see Component Shape Language spec §Widget SVG Readability Conventions).

The runtime MUST scan all configured bundle directories at startup. SVG files in the bundle MAY contain `{{token.key}}` mustache placeholders matching the pattern `\{\{([a-z][a-z0-9]*(?:\.[a-z][a-z0-9_]*)*)\}\}`. The runtime MUST resolve all placeholders by text substitution against the applicable design token map BEFORE SVG parsing:
- For global bundles: substitute against the global token map (user tokens merged with canonical fallbacks).
- For profile-scoped bundles: substitute against the profile's scoped token map (profile overrides → global → canonical fallbacks).

Substitution is a single left-to-right pass; substituted values are NOT re-scanned for further placeholders. Multiple placeholders in a single attribute value are supported. Literal `{{` sequences in SVG content that should NOT be treated as placeholders MUST be escaped as `\{\{`.

The runtime MUST reject bundles with the following error codes:
- `WIDGET_BUNDLE_NO_MANIFEST` — no `widget.toml` in the bundle directory
- `WIDGET_BUNDLE_INVALID_MANIFEST` — TOML parse error or missing required field
- `WIDGET_BUNDLE_INVALID_NAME` — name does not match `[a-z][a-z0-9-]*`
- `WIDGET_BUNDLE_DUPLICATE_TYPE` — duplicate name across loaded bundles (note: profile-scoped bundles are namespaced as `"{profile}/{name}"` and do not collide with global bundles)
- `WIDGET_BUNDLE_MISSING_SVG` — SVG file referenced in layers not found in bundle directory
- `WIDGET_BUNDLE_SVG_PARSE_ERROR` — SVG fails to parse after token placeholder resolution
- `WIDGET_BUNDLE_UNRESOLVED_TOKEN` — an `{{token.key}}` placeholder references a key not in the applicable token map. Error MUST include: bundle path, SVG file name, unresolved token key
- `WIDGET_BINDING_UNRESOLVABLE` — a binding references a nonexistent parameter, SVG element, or uses an incompatible mapping type
- `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION` — (profile-scoped bundles only) an SVG violates readability conventions for its component type. Error MUST include: bundle path, SVG file name, violation description

A rejected bundle MUST NOT prevent other valid bundles from loading; the runtime SHALL log the error at WARN level and continue.
Scope: v1-mandatory

#### Scenario: Valid bundle with token placeholders loaded
- **WHEN** the runtime scans a bundle directory containing a valid `widget.toml` with name "gauge", SVG files containing `fill="{{color.text.primary}}"`, and the global token map resolves `color.text.primary` to `"#FFFFFF"`
- **THEN** the runtime MUST substitute the placeholder to `fill="#FFFFFF"`, parse the resulting SVG, and register the Widget Type named "gauge"

#### Scenario: Missing manifest rejected
- **WHEN** the runtime scans a bundle directory that contains SVG files but no `widget.toml`
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_NO_MANIFEST and log the error

#### Scenario: Invalid manifest rejected
- **WHEN** the runtime scans a bundle directory with a `widget.toml` that has invalid TOML syntax
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_INVALID_MANIFEST and log the error

#### Scenario: Duplicate type name rejected for global bundles
- **WHEN** two global bundle directories both declare widget type name "gauge"
- **THEN** the runtime MUST reject the second bundle with error code WIDGET_BUNDLE_DUPLICATE_TYPE and log the error

#### Scenario: Profile-scoped bundle namespaced — no collision with global
- **WHEN** a global bundle declares name "gauge" and a profile "my-subtitles" also contains a bundle named "gauge"
- **THEN** the profile bundle MUST be registered as "my-subtitles/gauge" and MUST NOT collide with the global "gauge" bundle

#### Scenario: Missing SVG file rejected
- **WHEN** a `widget.toml` references layer file "fill.svg" but the file does not exist in the bundle directory
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_MISSING_SVG and log the error

#### Scenario: SVG parse failure after token resolution rejected
- **WHEN** a bundle's SVG, after token placeholder resolution, is not valid SVG (e.g., a resolved token value contains raw `<` characters)
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_SVG_PARSE_ERROR and log the error

#### Scenario: Unresolved token in SVG rejected
- **WHEN** a bundle SVG contains `fill="{{color.nonexistent}}"` and no token with that key exists in the applicable token map
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_UNRESOLVED_TOKEN, logging the bundle path, SVG file, and token key

#### Scenario: Readability violation in profile-scoped text widget
- **WHEN** a widget bundle inside a subtitle profile directory has SVG text elements with `data-role="text"` but missing `stroke` attribute
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION

#### Scenario: Global bundle with component_type NOT subject to readability check
- **WHEN** a global widget bundle (in `[widget_bundles].paths`) has `component_type = "subtitle"` in its manifest but its SVGs lack `data-role` attributes
- **THEN** the runtime MUST load the bundle without readability validation — the `component_type` field is informational for global bundles

#### Scenario: SVG with multiple placeholders in one attribute
- **WHEN** a bundle SVG contains `viewBox="0 0 {{spacing.unit}} {{spacing.unit}}"` and `spacing.unit` resolves to `"100"`
- **THEN** the runtime MUST substitute both placeholders, producing `viewBox="0 0 100 100"`
