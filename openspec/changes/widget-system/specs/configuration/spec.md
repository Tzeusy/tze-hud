# Configuration Specification (Widget System Delta)

Source: RFC 0006 (Configuration), widget-system proposal
Domain: GOVERNANCE

---

## MODIFIED Requirements

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

## ADDED Requirements

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
