# Configuration Specification (Component Shape Language Delta)

Source: RFC 0006 (Configuration), component-shape-language proposal
Domain: GOVERNANCE

---

## ADDED Requirements

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
