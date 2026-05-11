# configuration Specification

Source: RFC 0006 (Configuration)
Domain: GOVERNANCE

## Purpose

The configuration capability defines how tze_hud reads, validates, and hot-reloads its TOML configuration file. It governs file format and resolution order, profile selection and budget enforcement, tab and zone layout, agent registration and capability grants, privacy and quiet-hours policy, degradation ladder ordering, and schema export. The configuration system is the governance entry point for the entire runtime: all structural and policy decisions are validated at startup before any port is bound or frame is rendered.

## Requirements

### Requirement: TOML Configuration Format
The runtime SHALL use TOML as its configuration file format. YAML, JSON, RON, and all other formats MUST NOT be accepted. The runtime MUST refuse to start if the configuration file is not valid TOML. Parse errors MUST include line and column numbers.
Source: RFC 0006 §1.2
Scope: v1-mandatory

#### Scenario: Valid TOML accepted
- **WHEN** the runtime is started with a syntactically valid TOML configuration file
- **THEN** the file is parsed without error and configuration proceeds to schema validation

#### Scenario: Non-TOML file rejected with line/column
- **WHEN** the runtime is started with a configuration file containing invalid TOML syntax
- **THEN** startup fails with a structured parse error including the line number and column number of the syntax error

---

### Requirement: Configuration File Resolution Order
The runtime SHALL search for configuration files in the following order, using the first found: (1) `--config <path>` CLI flag, (2) `$TZE_HUD_CONFIG` environment variable, (3) `./tze_hud.toml` in the current working directory, (4) `$XDG_CONFIG_HOME/tze_hud/config.toml` on Linux/macOS, (5) `%APPDATA%\tze_hud\config.toml` on Windows. The runtime MUST refuse to start if no configuration file is found, and the error message MUST list the searched paths.
Source: RFC 0006 §1.3
Scope: v1-mandatory

#### Scenario: CLI flag takes precedence
- **WHEN** the runtime is started with `--config /path/to/custom.toml` and a `tze_hud.toml` exists in the current directory
- **THEN** the runtime loads `/path/to/custom.toml` and ignores the current-directory file

#### Scenario: No config file found
- **WHEN** the runtime is started with no CLI flag, no environment variable, and no config file at any standard location
- **THEN** startup fails with an error message listing all paths that were searched

---

### Requirement: Minimal Valid Configuration
A minimal valid configuration file MUST contain a `[runtime]` section with a `profile` field and at least one `[[tabs]]` entry. All other sections MUST be optional and use documented defaults when absent.
Source: RFC 0006 §2.1, §2.4
Scope: v1-mandatory

#### Scenario: Minimal config accepted
- **WHEN** a config file contains only `[runtime]` with `profile = "full-display"` and one `[[tabs]]` entry with a `name`
- **THEN** the runtime starts successfully with all optional sections populated from defaults

#### Scenario: Missing tabs rejected
- **WHEN** a config file contains `[runtime]` but no `[[tabs]]` entries
- **THEN** startup fails with `CONFIG_NO_TABS` structured error

---

### Requirement: Display Profile full-display
The `full-display` built-in profile MUST define the following budget values: `max_tiles = 1024`, `max_texture_mb = 2048`, `max_agents = 16`, `target_fps = 60`, `min_fps = 30`, `max_media_streams = 4`, `max_agent_update_hz = 60`. The admission control for this profile MUST be "tight" (budget enforcement active).
Source: RFC 0006 §3.2
Scope: v1-mandatory

#### Scenario: full-display profile budget values
- **WHEN** the runtime starts with `profile = "full-display"` and no display_profile overrides
- **THEN** the effective profile has max_tiles=1024, max_texture_mb=2048, max_agents=16, target_fps=60, min_fps=30, max_media_streams=4, max_agent_update_hz=60

---

### Requirement: Display Profile headless
The `headless` built-in profile MUST define the following budget values: `max_tiles = 256`, `max_texture_mb = 512`, `max_agents = 8`, `target_fps = 60`, `min_fps = 1`, `max_media_streams = 2`, `max_agent_update_hz = 30`. The headless profile MUST use an offscreen render target with no window. The `headless` profile MUST NOT be extendable via `[display_profile].extends`.
Source: RFC 0006 §3.4
Scope: v1-mandatory

#### Scenario: headless profile budget values
- **WHEN** the runtime starts with `profile = "headless"`
- **THEN** the effective profile has max_tiles=256, max_texture_mb=512, max_agents=8, target_fps=60, min_fps=1, max_media_streams=2, max_agent_update_hz=30 and renders to an offscreen surface

#### Scenario: headless not extendable
- **WHEN** a config file sets `[display_profile] extends = "headless"`
- **THEN** startup fails with `CONFIG_HEADLESS_NOT_EXTENDABLE` structured error

---

### Requirement: Mobile Profile Schema-Reserved
The `mobile` profile MUST be schema-reserved. Setting `profile = "mobile"` at runtime MUST produce the structured error `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` and the runtime MUST refuse to start. This is a hard startup error distinct from `CONFIG_UNKNOWN_PROFILE`. Setting `[display_profile].extends = "mobile"` MUST be syntactically valid (not a startup error) but MUST NOT activate any mobile runtime paths in v1; a custom profile that extends `mobile` produces a custom profile using the mobile budget values, but the Mobile Presence Node (MPN) display path is not exercised. Mobile capability negotiation is designed into the API (v1.md §"Mobile") but MUST NOT be implemented in v1.
Source: RFC 0006 §3.3, §3.6
Scope: v1-mandatory

#### Scenario: Mobile profile rejected at startup
- **WHEN** a config file sets `profile = "mobile"`
- **THEN** startup fails with `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` (not `CONFIG_UNKNOWN_PROFILE`) and a hint directing the operator to use `full-display` or `headless`

#### Scenario: Extends mobile is valid but post-v1
- **WHEN** a config file sets `[display_profile].extends = "mobile"` and `[runtime].profile = "custom"`
- **THEN** the configuration is accepted with the custom profile using mobile budget values, but no mobile-specific runtime paths are activated

---

### Requirement: Profile Auto-Detection
When `profile = "auto"`, the runtime MUST detect the environment in the following order: (1) headless if `$DISPLAY`/`$WAYLAND_DISPLAY` are unset, `/.dockerenv` exists, or wgpu reports software-only rendering; (2) `full-display` if VRAM > 4GB and refresh >= 60Hz; (3) abort with a structured error if neither condition matches. The `mobile` profile MUST never be selected by auto-detection in v1. An INFO log entry MUST name the detected signal for step 1.
Source: RFC 0006 §3.5
Scope: v1-mandatory

#### Scenario: Auto-detect headless in CI
- **WHEN** `profile = "auto"` and `$DISPLAY` is unset and `$WAYLAND_DISPLAY` is unset
- **THEN** the runtime selects `headless` and logs an INFO entry naming the detection signal

#### Scenario: Auto-detect full-display
- **WHEN** `profile = "auto"` and a GPU with >4GB VRAM is present and display refresh >= 60Hz
- **THEN** the runtime selects `full-display`

#### Scenario: Auto-detect failure requires explicit config
- **WHEN** `profile = "auto"` and a display is present but GPU VRAM is below 4GB
- **THEN** startup fails with a structured error instructing the operator to set an explicit `profile`

---

### Requirement: Profile Budget Escalation Prevention
Custom profiles that extend a built-in profile via `[display_profile].extends` MUST NOT exceed the base profile's budget values for `max_tiles`, `max_texture_mb`, `max_agents`, `max_media_streams`, and `max_agent_update_hz`. Boolean capability fields (`allow_background_zones`, `allow_chrome_zones`) MUST NOT be set to `true` if the base profile sets them `false`. Violations MUST produce `CONFIG_PROFILE_BUDGET_ESCALATION` or `CONFIG_PROFILE_CAPABILITY_ESCALATION` respectively.
Source: RFC 0006 §3.6
Scope: v1-mandatory

#### Scenario: Numeric budget escalation rejected
- **WHEN** a custom profile extends `full-display` and sets `max_tiles = 2048` (exceeding base 1024)
- **THEN** startup fails with `CONFIG_PROFILE_BUDGET_ESCALATION` identifying the offending field

#### Scenario: Boolean capability escalation rejected
- **WHEN** a custom profile extends a base that sets `allow_background_zones = false` and the override sets `allow_background_zones = true`
- **THEN** startup fails with `CONFIG_PROFILE_CAPABILITY_ESCALATION`

---

### Requirement: Profile Extends Conflict Detection
If `[display_profile].extends` is set AND `[runtime].profile` names a different built-in profile, the runtime MUST reject the configuration with `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE`.
Source: RFC 0006 §2.3
Scope: v1-mandatory

#### Scenario: Conflicting extends and profile
- **WHEN** `[runtime].profile = "full-display"` and `[display_profile].extends = "headless"`
- **THEN** startup fails with `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE`

---

### Requirement: Agent Registration with Per-Agent Budget Overrides
Pre-registered agents MUST be declarable in `[agents.registered]` with identity, authentication method, presence level ceiling, capability grants, and per-agent resource budget overrides. Per-agent `max_tiles` MUST NOT exceed the active profile's `max_tiles`; per-agent `max_texture_mb` MUST NOT exceed the profile's `max_texture_mb`; per-agent `max_update_hz` MUST NOT exceed the profile's `max_agent_update_hz`. Violations MUST produce `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE`.
Source: RFC 0006 §6.2
Scope: v1-mandatory

#### Scenario: Agent budget within profile ceiling
- **WHEN** a pre-registered agent sets `max_tiles = 4` and the active profile has `max_tiles = 1024`
- **THEN** the configuration is accepted

#### Scenario: Agent budget exceeds profile ceiling
- **WHEN** a pre-registered agent sets `max_tiles = 2048` and the active profile has `max_tiles = 1024`
- **THEN** startup fails with `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE` identifying the agent, field, and ceiling

---

### Requirement: Structured Validation Error Collection
The runtime MUST collect all validation errors before reporting. It MUST NOT fail on the first error. Each validation error MUST include: `code` (a stable string identifier from the `ConfigErrorCode` enum), `field_path` (dotted path to the offending field), `expected` (what was expected), `got` (what was found), and `hint` (a machine-readable correction suggestion).
Source: RFC 0006 §2.9
Scope: v1-mandatory

#### Scenario: Multiple errors reported together
- **WHEN** a config file has an unknown profile, a duplicate tab name, and an unknown zone type
- **THEN** all three errors are collected and reported in a single validation result, each with code, field_path, expected, got, and hint

---

### Requirement: Tab Configuration Validation
The `tabs` array MUST contain at least one entry (`CONFIG_NO_TABS`). Tab names MUST be unique across all tabs (`CONFIG_DUPLICATE_TAB_NAME`). At most one tab MAY set `default_tab = true` (`CONFIG_MULTIPLE_DEFAULT_TABS`). The `default_layout` MUST be one of `grid`, `columns`, or `freeform` (`CONFIG_UNKNOWN_LAYOUT`).
Source: RFC 0006 §2.4
Scope: v1-mandatory

#### Scenario: Duplicate tab names rejected
- **WHEN** two tabs share the name `"Morning"`
- **THEN** startup fails with `CONFIG_DUPLICATE_TAB_NAME` identifying the second tab

#### Scenario: Multiple default tabs rejected
- **WHEN** two tabs both set `default_tab = true`
- **THEN** startup fails with `CONFIG_MULTIPLE_DEFAULT_TABS`

---

### Requirement: Reserved Fraction Validation
Each `reserved_*_fraction` in `[tabs.layout]` MUST be in the range [0.0, 1.0]. The sum `reserved_top_fraction + reserved_bottom_fraction` MUST be strictly less than 1.0. The sum `reserved_left_fraction + reserved_right_fraction` MUST be strictly less than 1.0. Violations MUST produce `CONFIG_INVALID_RESERVED_FRACTION`.
Source: RFC 0006 §5.3
Scope: v1-mandatory

#### Scenario: Vertical reserved fractions sum to 1.0
- **WHEN** `reserved_top_fraction = 0.5` and `reserved_bottom_fraction = 0.5`
- **THEN** startup fails with `CONFIG_INVALID_RESERVED_FRACTION` with a hint that no vertical space remains for agent tiles

---

### Requirement: FPS Range Validation
The `target_fps` value MUST be greater than or equal to `min_fps` in any profile. Violation MUST produce `CONFIG_INVALID_FPS_RANGE`.
Source: RFC 0006 §3.6
Scope: v1-mandatory

#### Scenario: target_fps below min_fps
- **WHEN** a profile sets `target_fps = 15` and `min_fps = 30`
- **THEN** startup fails with `CONFIG_INVALID_FPS_RANGE`

---

### Requirement: Degradation Threshold Ordering
Frame-time thresholds in the degradation ladder MUST be monotonically non-decreasing: `coalesce_frame_ms <= simplify_rendering_frame_ms <= shed_tiles_frame_ms <= audio_only_frame_ms`. GPU fraction thresholds MUST be monotonically non-decreasing: `reduce_media_quality_gpu_fraction <= reduce_concurrent_streams_gpu_fraction`. Violations MUST produce `CONFIG_DEGRADATION_THRESHOLD_ORDER`.
Source: RFC 0006 §7.2
Scope: v1-mandatory

#### Scenario: Out-of-order frame-time thresholds
- **WHEN** `shed_tiles_frame_ms = 12.0` and `coalesce_frame_ms = 14.0`
- **THEN** startup fails with `CONFIG_DEGRADATION_THRESHOLD_ORDER` identifying the pair that is out of order

---

### Requirement: Privacy Configuration Defaults
The `[privacy]` section MUST support `default_classification` (one of `public`, `household`, `private`, `sensitive`; default: `private`), `default_viewer_class` (one of `owner`, `household_member`, `known_guest`, `unknown`, `nobody`; default: `unknown`), `viewer_id_method` (default: `none`), `redaction_style` (one of `pattern`, `blank`; default: `pattern`), and `multi_viewer_policy` (one of `most_restrictive`, `least_restrictive`; default: `most_restrictive`). Invalid values MUST produce the appropriate `CONFIG_UNKNOWN_*` error code.
Source: RFC 0006 §7.1
Scope: v1-mandatory

#### Scenario: Unknown classification rejected
- **WHEN** `default_classification = "top_secret"`
- **THEN** startup fails with `CONFIG_UNKNOWN_CLASSIFICATION`

#### Scenario: Unknown viewer class rejected
- **WHEN** `default_viewer_class = "admin"`
- **THEN** startup fails with `CONFIG_UNKNOWN_VIEWER_CLASS`

---

### Requirement: Quiet Hours Configuration
The `[privacy.quiet_hours]` section MUST support `enabled` (default: `false`), a `[[privacy.quiet_hours.schedule]]` array of time ranges with `start` (HH:MM 24-hour), `end` (HH:MM 24-hour, wraps midnight), and optional `days` (array of `"mon"`..`"sun"`; default: all days), `pass_through_class` (one of `CRITICAL`, `HIGH`, `NORMAL`, `LOW`, `SILENT`; default: `HIGH`; values use the canonical `InterruptionClass` enum from RFC 0010 §3.1), and `quiet_mode_display` (one of `"dim"`, `"clock_only"`, `"off"`; default: `"dim"`). `pass_through_class` specifies the minimum interruption class that passes through immediately during quiet hours. `CRITICAL` always passes through regardless of this setting; specifying `CRITICAL` as the threshold is valid (meaning only CRITICAL passes — all others queued or discarded). Classes below the configured threshold are deferred, except `SILENT` which is always unaffected (invisible by definition and carries no user-visible signal to defer): NORMAL is queued and delivered when quiet hours end; LOW is discarded (too stale to be useful); SILENT is never queued or discarded regardless of the threshold. Invalid `pass_through_class` values MUST produce `CONFIG_UNKNOWN_INTERRUPTION_CLASS`. Note: RFC 0006 originally used doctrine names (`urgent`, `gentle`) for `pass_through_class`; the canonical wire values are the InterruptionClass enum names (`CRITICAL`, `HIGH`, `LOW`, etc.) as established by RFC 0010 §3.1.
Source: RFC 0006 §7.1, RFC 0010 §3.1, §4.2
Scope: v1-mandatory

#### Scenario: Quiet hours enabled with HIGH pass-through
- **WHEN** `[privacy.quiet_hours] enabled = true` and `pass_through_class = "HIGH"`
- **THEN** during quiet hours: CRITICAL and HIGH interruptions pass through immediately; NORMAL interruptions are queued until quiet hours end; LOW interruptions are discarded; SILENT interruptions are unaffected (not queued)

#### Scenario: Invalid pass_through_class rejected
- **WHEN** `pass_through_class = "urgent"` (doctrine name, not canonical enum name)
- **THEN** startup fails with `CONFIG_UNKNOWN_INTERRUPTION_CLASS` with a hint suggesting the canonical name `HIGH`

---

### Requirement: Schema Export
The runtime MUST support `--print-schema` CLI flag which prints the full configuration JSON Schema to stdout and exits immediately without starting the runtime. The runtime MUST also support `emit_schema = true` in `[runtime]` which writes the schema at startup and continues running. `--print-schema` MUST take precedence when both are set.
Source: RFC 0006 §8
Scope: v1-mandatory

#### Scenario: Print schema and exit
- **WHEN** the runtime is invoked with `--print-schema`
- **THEN** a valid JSON Schema is written to stdout and the process exits with code 0 without binding any ports

#### Scenario: Print-schema precedence
- **WHEN** the runtime is invoked with `--print-schema` and the config has `emit_schema = true`
- **THEN** `--print-schema` behavior takes precedence (exit after print)

---

### Requirement: Redaction Style Ownership
The `redaction_style` field MUST exist exclusively in the `[privacy]` configuration section. The `[chrome]` section and its corresponding `ChromeConfig` Rust struct MUST NOT contain a `redaction_style` field. Any presence of `redaction_style` in `[chrome]` is a configuration error.
Source: RFC 0006 §2.8, RFC 0009 §5.2
Scope: v1-mandatory

#### Scenario: Redaction style in privacy section
- **WHEN** `[privacy].redaction_style = "pattern"` is set and `[chrome]` does not contain `redaction_style`
- **THEN** the configuration is accepted and redaction uses the privacy section value

---

### Requirement: Configuration Reload
The runtime MUST support live configuration reload via `SIGHUP` or `RuntimeService.ReloadConfig` gRPC call. Hot-reloadable fields SHALL include `[privacy]`, `[degradation]`, `[chrome]`, and `[agents.dynamic_policy]`. Fields requiring restart SHALL include `[runtime]`, `[[tabs]]`, and `[agents.registered]`. On reload, the runtime MUST re-validate the entire config; validation errors MUST be returned without applying the new config.
Source: RFC 0006 §9
Scope: v1-mandatory

#### Scenario: Hot-reload of privacy settings
- **WHEN** the runtime receives SIGHUP and the updated config changes `[privacy].redaction_style` from `"pattern"` to `"blank"`
- **THEN** the new redaction style takes effect without restart

#### Scenario: Reload validation failure
- **WHEN** the runtime receives SIGHUP and the updated config has validation errors
- **THEN** the errors are returned and the running configuration remains unchanged

---

### Requirement: Headless Virtual Display
In headless mode, zone geometry MUST resolve against a virtual display with configurable dimensions via `headless_width` and `headless_height` in `[runtime]`. Defaults MUST be 1920x1080. The field names MUST match RFC 0002 §7 (`headless_width`, `headless_height`).
Source: RFC 0006 §4.4
Scope: v1-mandatory

#### Scenario: Custom headless dimensions
- **WHEN** `profile = "headless"` and `headless_width = 1280` and `headless_height = 720`
- **THEN** zone geometry fractions compute against a 1280x720 virtual surface

---

### Requirement: Scene Event Naming Convention
The `tab_switch_on_event` field accepts either an empty string (meaning no automatic switch) or a non-empty event name. An empty string MUST be valid and MUST NOT generate a warning. Non-empty scene event names MUST follow the `<source>.<action>` dotted hierarchy pattern matching `^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$`. Non-empty names that do not match this pattern MUST produce `CONFIG_INVALID_EVENT_NAME`. Unrecognized (but validly formatted) non-empty event names MUST be accepted with a WARN log.
Source: RFC 0006 §5.5
Scope: v1-mandatory

#### Scenario: Valid event name accepted
- **WHEN** `tab_switch_on_event = "doorbell.ring"`
- **THEN** the configuration is accepted

#### Scenario: Invalid event name pattern rejected
- **WHEN** `tab_switch_on_event = "Doorbell-Ring"`
- **THEN** startup fails with `CONFIG_INVALID_EVENT_NAME`

#### Scenario: Empty string valid without warning
- **WHEN** `tab_switch_on_event = ""`
- **THEN** the configuration is accepted with no warning

---

### Requirement: Dynamic Agent Policy
The `[agents.dynamic_policy]` section MUST support `allow_dynamic_agents` (default: `false`), `default_capabilities`, `prompt_for_elevated_capabilities` (default: `true`), and `dynamic_presence_ceiling` (default: `"resident"`). Dynamic agent default budgets MUST be subject to the same profile ceiling validation as pre-registered agent budgets.
Source: RFC 0006 §6.4
Scope: v1-mandatory

#### Scenario: Dynamic agents disabled by default
- **WHEN** no `[agents.dynamic_policy]` section is present
- **THEN** connections from unregistered agents are rejected

---

### Requirement: Authentication Secret Indirection
Agent authentication secrets MUST never be stored directly in the configuration file. PSK-based authentication MUST reference an environment variable name via `auth_psk_env`. If the referenced environment variable is unset at startup, the runtime MUST log a warning and the agent MUST NOT be authenticable until the variable is set.
Source: RFC 0006 §6.5
Scope: v1-mandatory

#### Scenario: PSK from environment variable
- **WHEN** an agent sets `auth_psk_env = "AGENT_KEY"` and the environment variable `AGENT_KEY` is set
- **THEN** the agent can authenticate using the PSK from that variable

#### Scenario: Unset PSK environment variable
- **WHEN** an agent sets `auth_psk_env = "AGENT_KEY"` and the environment variable `AGENT_KEY` is not set
- **THEN** a warning is logged and the agent cannot authenticate

---

### Requirement: Layered Config Composition
Layered configuration composition via an `includes` field is schema-reserved for post-v1. V1 MUST use the single-file model. Any `includes` field present in a v1 config MUST produce a startup error with a message indicating that layered composition MUST NOT be implemented in v1.
Source: RFC 0006 §1.4
Scope: v1-reserved

#### Scenario: Includes field in v1
- **WHEN** a v1 config file contains `includes = "/etc/tze_hud/base.toml"`
- **THEN** the runtime produces a startup error indicating that layered composition MUST NOT be implemented in v1

---

### Requirement: Viewer Identification Pipeline
The viewer identification pipeline (`[[privacy.viewer_detectors]]`) is a post-v1 design direction. V1 MUST use the single `viewer_id_method` string form. The pipeline syntax MUST NOT be exercised in v1.
Source: RFC 0006 §7.1
Scope: post-v1

#### Scenario: Single viewer_id_method in v1
- **WHEN** `viewer_id_method = "face_recognition"` is set
- **THEN** a single detector pipeline of length 1 is used

---

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
Capability identifiers in agent grants MUST use the canonical `snake_case` wire-format names. The capability vocabulary defined in this specification is CANONICAL — all other specs, protocol handlers, example code, and MCP tool implementations MUST use these exact names when referencing capabilities. No synonyms, aliases, or legacy names are permitted in v1. The runtime MUST recognize all v1 capabilities: `create_tiles`, `modify_own_tiles`, `manage_tabs`, `manage_sync_groups`, `upload_resource`, `register_widget_asset`, `read_scene_topology`, `subscribe_scene_events`, `overlay_privileges`, `access_input_events`, `high_priority_z_order`, `exceed_default_budgets`, `read_telemetry`, `media_ingress`, `publish_zone:<zone_name>`, `publish_zone:*`, `emit_scene_event:<event_name>`, `resident_mcp`, `lease:priority:<N>`, `publish_widget:<widget_name>`, and `publish_widget:*`. Any capability name not in this canonical list (including misspellings, camelCase variants, or legacy names) MUST be rejected. Parameterized capability grants using the `system.` or `scene.` prefix for `emit_scene_event` MUST be rejected with `CONFIG_RESERVED_EVENT_PREFIX`. Note: RFC 0009 §8.1 contains older names (`read_scene`, `receive_input`, `zone_publish`) superseded by RFC 0005 Round 14 (rig-b2s); RFC 0006 §6.3 (this requirement) is the canonical authority.
Source: RFC 0006 §6.3, RFC 0005 Round 14
Scope: v1-mandatory

#### Scenario: Valid capability grants accepted
- **WHEN** an agent's capabilities include `["create_tiles", "publish_zone:subtitle", "emit_scene_event:doorbell.ring"]`
- **THEN** the configuration is accepted

#### Scenario: Reserved event prefix rejected
- **WHEN** an agent's capabilities include `"emit_scene_event:system.shutdown"`
- **THEN** startup fails with `CONFIG_RESERVED_EVENT_PREFIX`

#### Scenario: Non-canonical capability name rejected
- **WHEN** an agent's capabilities include a non-canonical name such as `"createTiles"`, `"create-tiles"`, `"tile_create"`, or any other name not in the canonical v1 vocabulary
- **THEN** startup fails with `CONFIG_UNKNOWN_CAPABILITY` identifying the unrecognized capability name and providing a hint with the closest canonical match (e.g., `{"unknown": "createTiles", "hint": "did you mean create_tiles?"}`)

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

### Requirement: Windows Media Ingress Configuration
Windows media ingress MUST be disabled by default. The runtime MUST only enable media ingress when `[media_ingress]` explicitly sets `enabled = true`, `approved_zone = "media-pip"`, `max_active_streams = 1`, a non-empty `default_classification`, explicit `operator_disabled` state, and fixed absolute `geometry` (`x`, `y`, `width`, `height`). The approved zone MUST be content-layer, MUST accept only `VideoSurfaceRef`, MUST use `transport_constraint = WebRtcRequired`, and MUST NOT be inferred from existing default zones such as `pip` or `ambient-background`. Authenticated resident/local producers MUST receive the canonical `media_ingress` capability explicitly through `[agents.registered]`.
Source: `openspec/changes/windows-media-ingress-exemplar/specs/configuration/spec.md`
Scope: windows-media-ingress-exemplar only

#### Scenario: media ingress absent remains disabled
- **WHEN** the configuration has no `[media_ingress]` table
- **THEN** media ingress is disabled and no approved media zone is registered

#### Scenario: explicit media-pip configuration accepted
- **WHEN** `[media_ingress]` enables `media-pip` with fixed geometry, `max_active_streams = 1`, default classification, and operator-disabled state
- **THEN** the resolved config records the approved Windows media-ingress surface
- **AND** only agents explicitly granted `media_ingress` can request media admission

#### Scenario: non-approved media zone rejected
- **WHEN** `[media_ingress]` names `pip`, `ambient-background`, or any zone other than `media-pip`
- **THEN** startup fails with `CONFIG_INVALID_MEDIA_INGRESS`

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
