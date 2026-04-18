# configuration Specification

## Purpose
TBD - created by archiving change component-shape-language. Update Purpose after archive.
## Requirements
### Requirement: Design Token Configuration Section
The configuration MUST support an optional `[design_tokens]` section containing a flat key-value map where keys are dotted token names (e.g., `"color.text.primary"`) and values are strings representing concrete visual primitives. The runtime MUST parse all entries as string key-value pairs. Keys MUST match the pattern `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*` (lowercase ASCII, dot-separated segments, underscores permitted after the first character of each segment). Invalid key format MUST produce `CONFIG_INVALID_TOKEN_KEY`. Values are opaque strings — the runtime stores them as-is; typed interpretation happens at token consumption time (RenderingPolicy field construction, SVG substitution). An absent `[design_tokens]` section MUST be treated as an empty map (canonical fallbacks apply for all standard tokens). Token values that fail typed parsing when consumed (e.g., `"not-a-color"` used as a color) MUST produce `TOKEN_VALUE_PARSE_ERROR` at startup.
Scope: v1-mandatory

#### Scenario: Valid design tokens parsed
- **WHEN** the configuration contains `[design_tokens]` with `"color.text.primary" = "#FFFFFF"` and `"typography.body.size" = "16"`
- **THEN** the runtime MUST load both tokens into the global token map as string key-value pairs

#### Scenario: Invalid token key rejected
- **WHEN** the configuration contains `[design_tokens]` with `"Color.Text.Primary" = "#FFFFFF"` (uppercase letters)
- **THEN** startup MUST fail with `CONFIG_INVALID_TOKEN_KEY` identifying the invalid key `"Color.Text.Primary"`

#### Scenario: Key with leading digit in segment rejected
- **WHEN** the configuration contains `[design_tokens]` with `"color.1invalid" = "#000000"`
- **THEN** startup MUST fail with `CONFIG_INVALID_TOKEN_KEY` because each segment must start with a lowercase letter

#### Scenario: Absent section treated as empty
- **WHEN** the configuration file contains no `[design_tokens]` section
- **THEN** the runtime MUST proceed with an empty user token map; canonical fallback values apply for all standard tokens

#### Scenario: Token value validated at consumption time
- **WHEN** `[design_tokens]` contains `"color.text.primary" = "not-a-color"` and the subtitle zone's default rendering attempts to parse it as a color
- **THEN** startup MUST fail with `TOKEN_VALUE_PARSE_ERROR` identifying the key, expected format "color hex (#RRGGBB or #RRGGBBAA)", and actual value

---

### Requirement: Component Profile Paths Configuration
The configuration MUST support an optional `[component_profile_bundles]` section with a `paths` field (array of directory paths). Each path MUST be resolved relative to the configuration file's parent directory (same resolution as `[widget_bundles].paths`). The runtime MUST scan each path for immediate subdirectories containing `profile.toml` manifests. Subdirectories without `profile.toml` MUST be silently skipped (they may be non-profile content). If a configured path does not exist, the runtime MUST produce `CONFIG_PROFILE_PATH_NOT_FOUND`. If `[component_profile_bundles]` is absent, the runtime MUST start with no loaded profiles (default token-derived rendering behavior for all component types). Duplicate profile names across all scanned directories MUST produce `CONFIG_PROFILE_DUPLICATE_NAME`.
Scope: v1-mandatory

#### Scenario: Valid profile path scanned
- **WHEN** `[component_profile_bundles].paths` includes `"./profiles"` and that directory contains subdirectories with valid `profile.toml` manifests
- **THEN** the runtime MUST load all valid profiles from those directories

#### Scenario: Path not found error
- **WHEN** `[component_profile_bundles].paths` includes `"./nonexistent"` and that directory does not exist relative to the configuration file
- **THEN** startup MUST fail with `CONFIG_PROFILE_PATH_NOT_FOUND` identifying the missing path and the resolved absolute path

#### Scenario: Absent section means no profiles
- **WHEN** the configuration file contains no `[component_profile_bundles]` section
- **THEN** the runtime MUST start with no loaded profiles; all component types use token-derived default rendering

#### Scenario: Duplicate profile name rejected
- **WHEN** two profile directories both contain a profile named `"my-subtitles"`
- **THEN** startup MUST fail with `CONFIG_PROFILE_DUPLICATE_NAME` identifying the duplicate name and both directory paths

#### Scenario: Subdirectory without profile.toml silently skipped
- **WHEN** `[component_profile_bundles].paths` includes `"./profiles"` and that directory contains a subdirectory `tmp/` without a `profile.toml`
- **THEN** the runtime MUST silently skip `tmp/` without logging an error

---

### Requirement: Component Profile Selection Configuration
The configuration MUST support an optional `[component_profiles]` section mapping component type names to active profile names. Each key MUST be a valid v1 component type name: `subtitle`, `notification`, `status-bar`, `alert-banner`, `ambient-background`, `pip`. Each value MUST be a string matching a loaded profile name. Unknown component type keys MUST produce `CONFIG_UNKNOWN_COMPONENT_TYPE`. Unknown profile names MUST produce `CONFIG_UNKNOWN_COMPONENT_PROFILE`. Profile-component type mismatches (profile's `component_type` does not match the key) MUST produce `CONFIG_PROFILE_TYPE_MISMATCH`. An absent `[component_profiles]` section MUST be valid — all component types use token-derived default rendering. Multiple entries MAY reference different profiles for different component types. The same profile MUST NOT be assigned to multiple component types (this is structurally impossible since profiles declare exactly one `component_type`).
Scope: v1-mandatory

#### Scenario: Valid profile selection
- **WHEN** `[component_profiles]` contains `subtitle = "my-subtitles"` and the `"my-subtitles"` profile is loaded with `component_type = "subtitle"`
- **THEN** the runtime MUST activate the profile for the subtitle component type

#### Scenario: Multiple component types configured
- **WHEN** `[component_profiles]` contains `subtitle = "cinematic-subs"` and `notification = "clean-notifs"` and both profiles are loaded with matching component types
- **THEN** the runtime MUST activate each profile for its respective component type independently

#### Scenario: Unknown component type key rejected
- **WHEN** `[component_profiles]` contains `hologram = "my-hologram"`
- **THEN** startup MUST fail with `CONFIG_UNKNOWN_COMPONENT_TYPE` identifying `"hologram"` and listing the valid component type names

#### Scenario: Unknown profile name rejected
- **WHEN** `[component_profiles]` contains `subtitle = "nonexistent"`
- **THEN** startup MUST fail with `CONFIG_UNKNOWN_COMPONENT_PROFILE` identifying `"nonexistent"` and listing the loaded profile names

#### Scenario: Type mismatch rejected
- **WHEN** `[component_profiles]` maps `subtitle = "clean-notifs"` but `"clean-notifs"` has `component_type = "notification"`
- **THEN** startup MUST fail with `CONFIG_PROFILE_TYPE_MISMATCH` identifying the key `subtitle`, the profile `"clean-notifs"`, and its actual component type `"notification"`

#### Scenario: Absent section uses defaults
- **WHEN** the configuration file contains no `[component_profiles]` section
- **THEN** all component types MUST use their token-derived default rendering behavior

---

### Requirement: Runtime Widget Asset Store Configuration
The configuration MUST support an optional `[widget_runtime_assets]` section that controls durable storage for runtime-registered widget SVG assets. Supported keys:
- `store_path` (string, optional): root directory for durable content-addressed widget SVG blobs and index files. Relative paths MUST resolve against the configuration file parent directory.
- `max_total_bytes` (u64, optional): global durable footprint ceiling.
- `max_agent_bytes` (u64, optional): per-agent durable footprint ceiling.

If `[widget_runtime_assets]` is absent, the runtime MUST use a platform-default cache location and built-in default limits. Runtime widget asset settings are startup-frozen in v1: hot reload MUST NOT mutate the active store path or budget ceilings.

Validation rules:
- Unwritable/uncreatable `store_path` MUST fail startup with `CONFIG_WIDGET_ASSET_STORE_UNWRITABLE`.
- `max_agent_bytes` greater than `max_total_bytes` MUST fail with `CONFIG_WIDGET_ASSET_BUDGET_INVALID`.
- Negative values are invalid (parse failure); `0` means "unbounded" only when explicitly configured.
Source: RFC 0006 §2.6a, RFC 0011 §9.1
Scope: v1-mandatory

#### Scenario: Explicit runtime widget asset store path
- **WHEN** configuration sets `[widget_runtime_assets].store_path = "./runtime_widget_assets"`
- **THEN** the runtime MUST resolve the path relative to the config directory and initialize the durable widget asset store there

#### Scenario: Budget relationship validation
- **WHEN** configuration sets `max_total_bytes = 1048576` and `max_agent_bytes = 2097152`
- **THEN** startup MUST fail with `CONFIG_WIDGET_ASSET_BUDGET_INVALID`

#### Scenario: Hot reload does not mutate widget runtime asset path
- **WHEN** a hot-reload config payload changes `[widget_runtime_assets].store_path`
- **THEN** the runtime MUST keep the existing active store path, log a warning, and require restart for the new path to take effect

---

### Requirement: Zone Registry Configuration
The zone registry MUST include built-in zone types: `subtitle`, `notification`, `status_bar`, `pip`, `ambient_background`, and `alert_banner`. Custom zone types MUST be definable via the `[zones]` section. Zone instances in `[tabs.zones]` MUST reference a defined or built-in zone type; unknown zone types MUST produce `CONFIG_UNKNOWN_ZONE_TYPE`. Widget types MUST be loaded from asset bundles in directories specified by `[widget_bundles].paths` (array of directory paths). Widget instances in `[tabs.widgets]` MUST reference a loaded widget type; unknown widget types MUST produce `CONFIG_UNKNOWN_WIDGET_TYPE`.
Source: RFC 0006 §2.5
Scope: v1-mandatory

#### Scenario: Built-in zone types available
- **WHEN** a tab defines `subtitle = { policy = "bottom_strip", layer = "content" }` without a custom `[zones.subtitle]` entry
- **THEN** the built-in `subtitle` zone type is used

#### Scenario: Unknown zone type rejected
- **WHEN** a tab references a zone type `news_ticker` that is not defined in `[zones]` and is not a built-in
- **THEN** startup fails with `CONFIG_UNKNOWN_ZONE_TYPE`

#### Scenario: Widget bundles loaded from configured paths
- **WHEN** `[widget_bundles].paths` includes `"./widgets"` and that directory contains subdirectories with valid `widget.toml` manifests
- **THEN** the runtime loads all widget type definitions from those bundles and they are available for use in `[tabs.widgets]`

#### Scenario: Unknown widget type rejected
- **WHEN** a tab's `[[tabs.widgets]]` entry references `type = "speedometer"` but no loaded widget bundle defines a `speedometer` type
- **THEN** startup fails with `CONFIG_UNKNOWN_WIDGET_TYPE`

#### Scenario: No widget_bundles section is valid
- **WHEN** the configuration file contains no `[widget_bundles]` section
- **THEN** the runtime starts successfully with an empty widget registry and no widget types available

---

### Requirement: Capability Vocabulary
Add to the canonical capability vocabulary: `publish_widget:<widget_name>` (parameterized, grants publish access to a specific widget), `publish_widget:*` (wildcard, grants publish access to all widgets). These follow the same pattern as `publish_zone:<zone_name>` and `publish_zone:*`. Any non-canonical capability name MUST be rejected with CONFIG_UNKNOWN_CAPABILITY.
Source: RFC 0006 §6.3
Scope: v1-mandatory

#### Scenario: publish_widget:gauge accepted
- **WHEN** an agent's capabilities include `"publish_widget:gauge"`
- **THEN** the configuration is accepted and the agent is granted publish access to the `gauge` widget

#### Scenario: publish_widget:* accepted
- **WHEN** an agent's capabilities include `"publish_widget:*"`
- **THEN** the configuration is accepted and the agent is granted publish access to all widget types

#### Scenario: Non-canonical widget capability rejected
- **WHEN** an agent's capabilities include a non-canonical name such as `"widget_publish:gauge"` or `"publishWidget:gauge"`
- **THEN** startup fails with `CONFIG_UNKNOWN_CAPABILITY` identifying the unrecognized capability name

---

### Requirement: Widget Bundle Configuration
The configuration MUST support an optional `[widget_bundles]` section with a `paths` field (array of directory paths). Each path MUST be resolved relative to the configuration file's parent directory. The runtime MUST scan each path for subdirectories containing `widget.toml` manifests. If a configured path does not exist, the runtime MUST produce `CONFIG_WIDGET_BUNDLE_PATH_NOT_FOUND`. If `[widget_bundles]` is absent, the runtime MUST start with an empty widget registry (widgets are optional). Duplicate widget type names across bundles MUST produce `CONFIG_WIDGET_BUNDLE_DUPLICATE_TYPE`.
Source: widget-system proposal
Scope: v1-mandatory

#### Scenario: Valid bundle path scanned
- **WHEN** `[widget_bundles].paths` includes `"./my-widgets"` and that directory exists with subdirectories containing `widget.toml` manifests
- **THEN** the runtime scans each subdirectory, loads the manifests, and registers the widget types in the widget registry

#### Scenario: Path not found error
- **WHEN** `[widget_bundles].paths` includes `"./nonexistent-dir"` and that directory does not exist relative to the configuration file
- **THEN** startup fails with `CONFIG_WIDGET_BUNDLE_PATH_NOT_FOUND` identifying the missing path

#### Scenario: Absent section means no widgets
- **WHEN** the configuration file does not contain a `[widget_bundles]` section
- **THEN** the runtime starts with an empty widget registry and no widget types are available; this is not an error

#### Scenario: Duplicate type name across bundles rejected
- **WHEN** two different bundle directories each contain a widget type named `gauge`
- **THEN** startup fails with `CONFIG_WIDGET_BUNDLE_DUPLICATE_TYPE` identifying the duplicate name and the conflicting bundle paths

---

### Requirement: Widget Instance Configuration
Widget instances MUST be declarable per tab via `[[tabs.widgets]]` entries. Each entry MUST specify: `type` (string, references a loaded widget type name), optional `geometry` override (inline geometry policy), and optional `initial_params` (inline table mapping parameter names to values). The runtime MUST validate `initial_params` against the widget type's parameter schema at startup. Invalid initial parameters MUST produce `CONFIG_WIDGET_INVALID_INITIAL_PARAMS`. Multiple instances of the same widget type on the same tab MUST be disambiguated by an optional `instance_id` field. The resulting instance_name (used for publish targeting) SHALL be the `instance_id` if provided, otherwise the widget `type` name. instance_name MUST be unique per tab.
Source: widget-system proposal
Scope: v1-mandatory

#### Scenario: Widget instance with geometry override
- **WHEN** a `[[tabs.widgets]]` entry specifies `type = "gauge"` and includes a `geometry` override with custom position and size
- **THEN** the widget instance is created with the overridden geometry policy instead of the widget type's default geometry

#### Scenario: initial_params validated
- **WHEN** a `[[tabs.widgets]]` entry specifies `type = "gauge"` with `initial_params = { value = 0.75, label = "CPU" }` and the gauge widget's parameter schema accepts `value` (float 0.0-1.0) and `label` (string)
- **THEN** the configuration is accepted and the widget instance starts with those parameter values

#### Scenario: Invalid initial param rejected
- **WHEN** a `[[tabs.widgets]]` entry specifies `initial_params = { value = "not_a_number" }` but the widget type's schema defines `value` as a float
- **THEN** startup fails with `CONFIG_WIDGET_INVALID_INITIAL_PARAMS` identifying the parameter name, expected type, and actual value

#### Scenario: Multiple instances with instance_id
- **WHEN** a tab declares two `[[tabs.widgets]]` entries both with `type = "gauge"`, one with `instance_id = "cpu_gauge"` and one with `instance_id = "mem_gauge"`
- **THEN** the configuration is accepted and both widget instances are created with distinct identities

#### Scenario: Missing instance_id for duplicate type rejected
- **WHEN** a tab declares two `[[tabs.widgets]]` entries both with `type = "gauge"` and neither specifies an `instance_id`
- **THEN** startup fails with a structured error indicating that duplicate widget types on the same tab require an `instance_id` for disambiguation
