# RFC 0006: Configuration

**Status:** Draft
**Issue:** rig-5vq.9
**Date:** 2026-03-22
**Authors:** tze_hud architecture team

---

## Review History

### Round 1 — Doctrinal Alignment (rig-5vq.31)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, mobile.md, presence.md, privacy.md, security.md, failure.md

#### Doctrinal Alignment: 4/5
All MUST-FIX and SHOULD-FIX findings addressed. See PR #26 for details.

**[MUST-FIX → FIXED]** Added `headless` built-in profile (§3.4) — architecture.md requires "at least two built-in profiles: 'desktop' and 'headless'"; RFC originally only had `full-display` and `mobile`.

**[MUST-FIX → FIXED]** Fixed headless field names to match RFC 0002 §7 (`headless_width`/`headless_height`).

**[MUST-FIX → FIXED]** Added `DisplayProfileConfig` Rust struct (user-editable `[display_profile]` section with optional overrides).

**[MUST-FIX → FIXED]** Added `prefer_zones` and `upstream_precomposition` to `DisplayProfile` Rust struct.

**[SHOULD-FIX → FIXED]** §5.4 now cites the canonical 7-step policy evaluation order from architecture.md §"Policy arbitration".

**[SHOULD-FIX → FIXED]** Added `max_texture_mb` unit note (config is MiB; runtime uses bytes per RFC 0002 §4.3).

**[SHOULD-FIX → FIXED]** Added validation rules for `default_classification` and `default_viewer_class` including all five viewer classes.

**No dimension below 3. Round 1 complete.**

---

### Round 2 — Technical Architecture Scrutiny (rig-5vq.32)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, presence.md, security.md, privacy.md, failure.md, mobile.md

#### Doctrinal Alignment: 4/5 (unchanged from Round 1)
No doctrinal regressions found.

#### Technical Robustness: 4/5 (after fixes)

**[MUST-FIX → FIXED]** `ConfigError.code` was `&'static str`, not a typed enum. Introduced `ConfigErrorCode` enum; changed `ConfigError.code` to `ConfigErrorCode`. The enum is `JsonSchema`-derived, so the exported schema enumerates all valid codes — the summary table is no longer a separate source of truth that can drift.

**[MUST-FIX → FIXED]** `[display_profile].extends` and `[runtime].profile` could be set to different built-ins with no validation error, resulting in silent misconfiguration. Added `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE` validation rule in §2.3; §3.5 now specifies the resolution order and failure path for auto-detection.

**[MUST-FIX → FIXED]** Boolean capability escalation (`allow_background_zones`, `allow_chrome_zones`) was not guarded by the profile escalation rule, which only checked numeric fields. Added `CONFIG_PROFILE_CAPABILITY_ESCALATION` validation rule in §3.6.

**[SHOULD-FIX → FIXED]** Degradation threshold ordering was not validated. Added `CONFIG_DEGRADATION_THRESHOLD_ORDER` rule requiring frame-time thresholds and GPU-fraction thresholds to be monotonically non-decreasing across ladder steps.

**[SHOULD-FIX → FIXED]** `emit_schema = true` vs `--print-schema` semantics were ambiguous. §2.2 now documents that `emit_schema = true` continues running after emitting; `--print-schema` exits immediately; `--print-schema` takes precedence when both are set.

#### Cross-RFC Consistency: 4/5

**[CONSIDER]** `tab_switch_on_event` unknown names are silently accepted at load time; a `WARN` log entry is recommended for names not in the built-in event registry.

**[CONSIDER]** `profile = "auto"` failure path now specified: falls back to `mobile` with a `WARN` log.

**No dimension below 3. Round 2 complete.**

---

## Summary

This RFC defines the configuration system for tze_hud: the file format, schema, display profile definitions, zone geometry policies, tab and layout configuration, agent registration, and privacy/degradation policies. Configuration is the primary mechanism for declaring scenes, zones, and policies in v1 before dynamic orchestration exists, so it is a first-class, schema-validated, LLM-readable surface.

---

## Motivation

tze_hud is configuration-driven in v1. Zone definitions are static (see RFC 0001 §5 and v1.md scope). Tab layouts, default zones, quiet hours, viewer policies, degradation thresholds, and pre-registered agent identities are all declared in files on disk. Without a precise configuration contract:

- Agents cannot inspect or reason about the runtime's expected scene topology.
- Validation errors surface at runtime as cryptic failures rather than load-time structured errors.
- LLMs writing deployment configs have no schema surface to target.
- Display profile differences (Full Display Node vs. Mobile Presence Node) cannot be captured declaratively.

This RFC resolves all of these by specifying configuration as a validated, declarative, file-based surface with defined schema, structured load-time errors, and explicit semantics for every top-level section.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| DR-V1: Scene separable from renderer | Configuration declares scene topology; no GPU types appear in config |
| DR-V4: Deterministic test scenes | Test fixtures are valid config files; validation is pure, no runtime dependency |
| Configuration model (architecture.md) | Declarative, file-based, human-readable, validated at load time |

---

## Cross-References

- RFC 0001 (Scene Contract) — zone registry, tile identity, namespace model
- RFC 0002 (Runtime Kernel) — startup sequence, config load path, reload-on-signal
- RFC 0003 (Timing Model) — quiet hours time semantics, degradation timer thresholds
- RFC 0004 (Input) — tab switching policy (tab_switch_on_event)
- heart-and-soul/architecture.md — configuration model doctrine
- heart-and-soul/mobile.md — two profiles, one model
- heart-and-soul/presence.md — zones, geometry, layer attachment
- heart-and-soul/privacy.md — viewer classes, quiet hours, content classification
- heart-and-soul/failure.md — degradation axes and ladder
- heart-and-soul/security.md — agent authentication, capability scopes

---

## 1. File Format Decision

### 1.1 Candidate Evaluation

| Format | Human-Editable | LLM-Parseable | Schema-Validatable | Comments | No Ambiguous Coercion |
|--------|---------------|---------------|-------------------|----------|----------------------|
| **TOML** | Excellent | Excellent | Via `toml-schema` / schemars | Yes (# comments) | Yes — all types explicit |
| **RON** | Good | Good | Via manual schema | Yes | Yes — Rust-native types |
| **YAML** | Good | Good | Via JSON Schema + yaml2json | Yes | **No** — `yes`/`no`/`on`/`off` bool coercion, bare numbers silently typed, Norway problem |
| **JSON5** | Good | Excellent | Via JSON Schema | Yes (// comments) | Yes |

### 1.2 Decision: TOML

**TOML is the configuration format for tze_hud.**

Rationale:

- **No ambiguous type coercion.** TOML's type system is explicit: a string is always quoted, an integer is never silently a boolean, there is no Norway problem. For a configuration system that produces structured validation errors, this correctness property is non-negotiable.
- **Human-editable.** TOML was designed for human-authored configuration files. Section headers map naturally to tze_hud's top-level concepts (`[runtime]`, `[display_profile]`, `[tabs.*]`).
- **LLM-parseable.** TOML's syntax is unambiguous and compact. LLMs generating or modifying config files make fewer mistakes than with YAML's whitespace-sensitive indentation.
- **Schema-validatable.** The Rust ecosystem has mature TOML parsing (`toml` crate) with `serde` integration. The runtime derives JSON Schema from its config types via `schemars` and exports it for tooling and LLM use.
- **Comments.** `#` comments are supported. Config files can self-document unusual settings.
- **RON** was considered as a Rust-native alternative. RON's syntax is less familiar to non-Rust developers and LLMs, and its schema story is weaker. The benefit does not outweigh TOML's broader literacy.
- **YAML** is rejected specifically for its coercion ambiguities. A config format that silently interprets `NO` as a boolean `false` is incompatible with the structured-error mandate.
- **JSON5** is a reasonable alternative but is less familiar than TOML for configuration use cases and lacks the `[section]` structure that maps cleanly to tze_hud's schema.

### 1.3 File Layout

The runtime looks for configuration in the following order (first found wins):

1. Path specified by `--config <path>` CLI flag
2. `$TZE_HUD_CONFIG` environment variable
3. `./tze_hud.toml` (current working directory)
4. `$XDG_CONFIG_HOME/tze_hud/config.toml` (Linux/macOS)
5. `%APPDATA%\tze_hud\config.toml` (Windows)

A config file is required. The runtime refuses to start without one. The error message includes the searched paths.

A minimal valid config file (using all defaults) is:

```toml
[runtime]
profile = "full-display"

[display_profile]
# Uses the built-in "full-display" profile — all defaults apply.

[[tabs]]
name = "default"
```

---

## 2. Configuration Schema

### 2.1 Top-Level Structure

```toml
[runtime]               # Process-level settings: profile, window mode, log level
[display_profile]       # Profile override/extension: resource budgets, frame rate caps
[[tabs]]                # Ordered list of tab definitions (TOML array-of-tables)
[zones]                 # Zone type registry: built-in + custom zone definitions
[agents]                # Pre-registered agent identities and capability grants
[privacy]               # Viewer classes, content classification defaults, quiet hours
[degradation]           # Thresholds, ladder steps, hysteresis
[chrome]                # Runtime chrome configuration: tab bar, indicators, prompts
```

Each section is optional except `[runtime]` and at least one `[[tabs]]` entry. Missing sections use documented defaults.

### 2.2 `[runtime]` Section

```toml
[runtime]
# Which display profile to use. One of: "full-display", "mobile", or a custom
# profile name defined in [display_profile]. Required.
profile = "full-display"

# Window mode. One of: "fullscreen", "overlay".
# Default: "fullscreen".
# "overlay" requires platform support (see architecture.md window model).
window_mode = "fullscreen"

# Bind address for the gRPC control plane.
# Default: "127.0.0.1:50051"
grpc_bind = "127.0.0.1:50051"

# Bind address for the MCP compatibility plane.
# Default: "127.0.0.1:50052"
mcp_bind = "127.0.0.1:50052"

# Log level. One of: "error", "warn", "info", "debug", "trace".
# Default: "info"
log_level = "info"

# Maximum agent reconnection grace period in seconds.
# If an agent disconnects and reconnects within this window, it may reclaim leases.
# Default: 30
reconnect_grace_secs = 30

# If true, the runtime writes a JSON schema to stdout at startup and then
# continues running (useful for logging pipelines that capture stdout).
# For non-interactive tooling use, prefer the --print-schema CLI flag, which
# prints the schema and exits immediately without starting the runtime.
# Default: false
emit_schema = false

# Virtual display dimensions for headless mode (profile = "headless").
# Ignored when running with a real display.
# Default: 1920 x 1080
# Note: RFC 0002 §7 uses the same field names: headless_width, headless_height.
headless_width = 1920
headless_height = 1080
```

**Validation rules:**
- `profile` must name a built-in profile or a profile defined in `[display_profile]`. Unknown profile → structured error `CONFIG_UNKNOWN_PROFILE`.
- `grpc_bind` and `mcp_bind` must be valid socket addresses. Invalid address → structured error `CONFIG_INVALID_ADDRESS`.
- `window_mode = "overlay"` on an unsupported platform emits a warning and falls back to `"fullscreen"` at runtime (not a startup error).

### 2.3 `[display_profile]` Section

See §3 for full profile definitions. This section either names a built-in profile or provides fields that override or extend one.

```toml
[display_profile]
# Extend a built-in profile. Optional; if omitted, [runtime].profile is used as-is.
extends = "full-display"

# Override specific budget fields (any field from the profile schema).
max_tiles = 512
max_agents = 8
target_fps = 30
```

**Validation rules:**
- If `[display_profile].extends` is set AND `[runtime].profile` names a different built-in, this is a configuration conflict. The operator almost certainly intended to name the custom profile (the result of extending the base) in `[runtime].profile`. → `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE` with a hint: "set `profile = \"<custom-name>\"` in `[runtime]` or remove the `extends` field."
- When `extends` is set without any custom name in `[runtime].profile`, the runtime internally names the result `"custom"` for logging; the gRPC handshake reports `"custom"`. Deployments should set `[runtime].profile` to a descriptive custom name to avoid ambiguity (see §3.6 for the correct pattern).

### 2.4 `[[tabs]]` Section

Array-of-tables. Each `[[tabs]]` entry defines one tab.

```toml
[[tabs]]
name = "Morning"                  # Required. Human-readable, also the routing key.
display_name = "Morning"          # Optional; defaults to `name`.
icon = "sun"                      # Optional icon name (runtime-defined icon set).
default_layout = "columns"        # "grid", "columns", "freeform". Default: "grid".
default_tab = true                # At most one tab may set this. Default: false.

# Zone instances active in this tab. See §4 for zone geometry policies.
[tabs.zones]
subtitle = { policy = "bottom_strip", layer = "content" }
notification = { policy = "top_right_stack", layer = "content" }
status_bar = { policy = "full_width_bar", layer = "chrome" }
ambient_background = { policy = "fullscreen_behind", layer = "background" }

# Tab switching policy: this tab becomes active when the named event fires.
# Event names are from RFC 0004 (Input) scene-level events.
# Optional.
tab_switch_on_event = "doorbell.ring"

# Layout constraints for this tab.
[tabs.layout]
min_tile_width_px = 120
min_tile_height_px = 80
max_tile_count = 64
# Reserved area per zone (agent tiles cannot overlap these).
# Expressed as fractional screen area (0.0–1.0).
reserved_bottom_fraction = 0.08  # for subtitle zone
reserved_top_fraction = 0.04     # for status_bar zone
```

**Validation rules:**
- `name` must be unique across all tabs. Duplicate → `CONFIG_DUPLICATE_TAB_NAME`.
- At most one tab may set `default_tab = true`. Multiple → `CONFIG_MULTIPLE_DEFAULT_TABS`.
- `default_layout` must be one of the enumerated values. Unknown value → `CONFIG_UNKNOWN_LAYOUT`.
- Each zone instance name in `[tabs.zones]` must correspond to a defined zone type in `[zones]` or a built-in zone. Unknown zone → `CONFIG_UNKNOWN_ZONE_TYPE`.
- `policy` must be a built-in policy name or a custom policy defined in `[zones]`. Unknown policy → `CONFIG_UNKNOWN_GEOMETRY_POLICY`.
- `layer` must be `"content"`, `"background"`, or `"chrome"`. Zone types have a default layer; overriding with an incompatible layer → `CONFIG_INCOMPATIBLE_ZONE_LAYER`.

### 2.5 `[zones]` Section

The zone registry. Built-in zone types (subtitle, notification, status_bar, pip, ambient_background, alert_banner) are always available. Custom zone types extend the registry.

```toml
[zones]
# Override a built-in zone type's rendering policy defaults.
[zones.subtitle]
default_privacy = "household"
timeout_secs = 5.0

# Define a custom zone type.
[zones.weather_ticker]
accepted_media_types = ["stream-text", "key-value"]
contention_policy = "merge-by-key"
default_privacy = "public"
interruption_class = "silent"
layer = "chrome"
timeout_secs = 0.0  # 0 = no automatic timeout
adjunct_effects = []
```

### 2.6 `[agents]` Section

Pre-registered agent identities. See §6 for full agent registration schema.

### 2.7 `[privacy]` Section

See §7 for privacy and degradation policy schemas.

### 2.8 `[chrome]` Section

```toml
[chrome]
# Show the tab bar.
# Default: true
show_tab_bar = true

# Position of the tab bar. One of: "top", "bottom".
# Default: "top"
tab_bar_position = "top"

# Show system indicators (connection state, agent count, degradation state).
# Default: true
show_system_indicators = true

# Show the "dismiss all" / "safe mode" override button.
# Default: true
show_override_controls = true

# Redaction placeholder style. One of: "pattern", "agent_name", "icon".
# Default: "pattern"
redaction_style = "pattern"
```

### 2.9 Structured Validation Errors

Every validation failure produces a structured error with:

- `code` — a stable string identifier (e.g., `CONFIG_UNKNOWN_PROFILE`)
- `field_path` — dotted path to the offending field (e.g., `"tabs[1].zones.subtitle.policy"`)
- `expected` — what type or value was expected (e.g., `"one of: bottom_strip, top_right_stack, ..."`)
- `got` — what was actually found (e.g., `"\"side_rail\""`)
- `hint` — a machine-readable correction suggestion (e.g., `"use policy = \"bottom_strip\" for a subtitle zone"`)

Multiple errors are collected before reporting. The runtime does not fail on the first error — it validates the entire config and returns all errors at once.

```
Error loading config: 3 validation error(s)

  [CONFIG_UNKNOWN_PROFILE]
  Field: runtime.profile
  Expected: one of: full-display, mobile
  Got: "wall-display"
  Hint: define a custom profile under [display_profile] or use a built-in name

  [CONFIG_DUPLICATE_TAB_NAME]
  Field: tabs[2].name
  Expected: unique tab name
  Got: "Morning" (already defined at tabs[0])
  Hint: choose a distinct name for each tab

  [CONFIG_UNKNOWN_ZONE_TYPE]
  Field: tabs[0].zones.news_ticker
  Expected: a zone type defined in [zones] or a built-in zone type
  Got: "news_ticker"
  Hint: define [zones.news_ticker] or use a built-in zone type name
```

---

## 3. Display Profile Definitions

### 3.1 Profile Architecture

A display profile is a named set of resource budgets, capability constraints, and rendering parameters. It shapes what the runtime permits — not what agents can request, but what the compositor will grant and enforce.

Profiles are not a fork. The scene model, API, and protocol are identical across profiles. A profile is a budget envelope. Agents negotiate within that envelope; the runtime enforces it.

**V1 built-in profiles:**

| Name | Purpose |
|------|---------|
| `full-display` | High-end local display (wall display, monitor, kiosk). GPU, persistent power. |
| `mobile` | Mobile Presence Node (phone, glasses-class). Thermal limits, variable network. |
| `headless` | CI/test, no window. Offscreen render target. Not for production deployments. |

The `"desktop"` alias used in early doctrine drafts maps to `full-display`. The `"headless"` profile is the v1 mechanism for CI testing (see architecture.md §"Display profiles": "V1 supports at least two built-in profiles: 'desktop' (high-end local display) and 'headless' (CI/test, no window)"). The RFC uses `full-display` as the canonical production name; `headless` is the third built-in to support the doctrinal requirement.

### 3.2 Built-in Profile: `full-display`

The Full Display Node profile. Targets wall displays, dedicated monitors, kiosks, and mirror displays with a local GPU and persistent power.

```toml
# Built-in profile definition (shown for documentation; not user-editable directly)
[profiles.full-display]
# Maximum simultaneous leased tiles.
max_tiles = 1024

# Maximum total texture memory (MB) across all agent-leased surfaces.
max_texture_mb = 2048

# Maximum concurrent resident + embodied agents.
max_agents = 16

# Target compositor frame rate (fps).
target_fps = 60

# Minimum guaranteed frame rate under load (fps).
# Below this, degradation is triggered automatically.
min_fps = 30

# Supported node types (all v1 node types).
allowed_node_types = [
  "solid_color",
  "text_markdown",
  "static_image",
  "hit_region",
]

# Supported window modes.
allowed_window_modes = ["fullscreen", "overlay"]

# Maximum concurrent WebRTC media streams (post-v1).
max_media_streams = 8

# Maximum agent update rate (Hz per agent, for state-stream class).
max_agent_update_hz = 60

# Whether background-layer zones are allowed.
allow_background_zones = true

# Whether chrome-layer zones are allowed.
allow_chrome_zones = true
```

### 3.3 Built-in Profile: `mobile`

The Mobile Presence Node profile. Targets high-end phones and smart-glasses-class devices with variable network, thermal limits, and tighter display budgets.

```toml
# Built-in profile definition (shown for documentation)
[profiles.mobile]
max_tiles = 32
max_texture_mb = 256
max_agents = 4
target_fps = 60           # Target 60fps when thermal allows
min_fps = 30              # Accept 30fps under thermal pressure
allowed_node_types = [
  "solid_color",
  "text_markdown",
  "static_image",
  "hit_region",
]
# Overlay mode is included for phone targets that support transparent windows
# (e.g., PiP overlay on Android, transparent floating window on iOS).
# On glasses-class devices, overlay is the primary rendering mode.
# Not all mobile platforms support overlay — the runtime falls back to fullscreen
# on platforms where overlay primitives are unavailable (see architecture.md
# §"Window model: two deployment modes", Promise boundary).
allowed_window_modes = ["fullscreen", "overlay"]
max_media_streams = 1     # One primary live stream
max_agent_update_hz = 30  # Coalesce more aggressively
allow_background_zones = false   # Background layer not available on mobile
allow_chrome_zones = true

# Mobile-specific: prefer zones over raw tiles (advisory to orchestrators).
# Zones abstract geometry which varies dramatically across mobile devices (see mobile.md).
prefer_zones = true

# Mobile-specific: upstream precomposition allowed (post-v1).
upstream_precomposition = false
```

**Note on `prefer_zones` and `upstream_precomposition`:** These are mobile-specific advisory fields surfaced in the profile's TOML documentation. They are included in the `DisplayProfile` Rust struct as optional boolean fields with `#[serde(default)]`. They do not affect the compositor's enforcement budget — they are hints to orchestrators and the upstream service respectively.

### 3.4 Built-in Profile: `headless`

The headless profile. Targets CI pipelines, integration tests, and offline rendering. Creates an offscreen texture surface — no window, no display server required. This is the third v1 built-in, satisfying the architecture.md doctrine: "V1 supports at least two built-in profiles: 'desktop' (high-end local display) and 'headless' (CI/test, no window)."

```toml
# Built-in profile definition (shown for documentation)
[profiles.headless]
max_tiles = 256
max_texture_mb = 512
max_agents = 8
target_fps = 60          # Software-driven; tokio::time::interval (see RFC 0002 §7)
min_fps = 1              # No vsync pressure; any frame rate is acceptable in CI
allowed_node_types = [
  "solid_color",
  "text_markdown",
  "static_image",
  "hit_region",
]
allowed_window_modes = ["fullscreen"]  # Window mode is ignored; surface is always offscreen
max_media_streams = 0    # No WebRTC in CI context (v1)
max_agent_update_hz = 60
allow_background_zones = true
allow_chrome_zones = true
```

This profile is selected by `profile = "headless"` in `[runtime]`. It cannot be extended via `[display_profile].extends` — headless is a terminal profile because its offscreen surface is a compile-time-invariant property (RF 0002 §7 headless mode).

**Validation rule:** If `profile = "headless"` and `window_mode` is set to anything other than `"fullscreen"`, emit a warning and ignore `window_mode` (the offscreen path does not use a window surface).

### 3.5 Profile Negotiation

When the runtime starts, it selects the active profile as follows:

1. **Explicit config**: if `[runtime].profile` names a profile, use it.
2. **Auto-detection** (if `profile = "auto"`): the runtime queries hardware capabilities and selects the closest built-in profile. Detection logic:
   - If a local GPU with > 4GB VRAM is present and display refresh >= 60Hz: `full-display`
   - If no local GPU or display refresh < 60Hz: `mobile`
   - If detection fails or hardware information is unavailable (virtualized environment, missing driver): fall back to `mobile` (most conservative), log a `WARN` with the detection output.
3. **Profile extension**: if `[display_profile].extends` is set, the named base profile is loaded and then overridden field-by-field with any fields present in `[display_profile]`. The result is the effective profile; `[runtime].profile` names this effective profile. For a custom-named profile, `[runtime].profile` must be set to the custom name (e.g., `"glasses-v1"`) and `[display_profile].extends` must name the built-in base. If `[display_profile].extends` is set and `[runtime].profile` names a *different* built-in, the configuration is rejected with `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE` (see §2.3).

The selected profile name is logged at startup and included in the runtime's gRPC handshake response so agents can inspect it.

### 3.6 Custom Profiles

A deployment can define a custom profile for specific hardware (e.g., a glasses device with unusual limits):

```toml
[runtime]
profile = "glasses-v1"

[display_profile]
extends = "mobile"
max_tiles = 8
max_texture_mb = 64
max_agents = 2
target_fps = 30
min_fps = 15
max_media_streams = 0       # No media in v1 glasses profile
allow_chrome_zones = false  # Minimal chrome on glasses
```

Custom profiles extend a built-in profile. Extending another custom profile is not supported (avoids chain resolution complexity). The `headless` built-in cannot be used as a base for custom profiles — it implies an offscreen render path that cannot be extended with windowed-display parameters.

**Validation rules:**
- Custom profile `extends` must name a built-in profile. Unknown base → `CONFIG_UNKNOWN_BASE_PROFILE`.
- `extends = "headless"` is not permitted for custom profiles. Attempt → `CONFIG_HEADLESS_NOT_EXTENDABLE`.
- Numeric overrides must not exceed the base profile's values (prevent budget escalation). Exceeding → `CONFIG_PROFILE_BUDGET_ESCALATION` with a note. Applies to: `max_tiles`, `max_texture_mb`, `max_agents`, `max_media_streams`, `max_agent_update_hz`.
- Boolean capability fields (`allow_background_zones`, `allow_chrome_zones`) may not be set to `true` if the base profile sets them `false`. Attempting to do so escalates a capability the base profile was designed to restrict. → `CONFIG_PROFILE_CAPABILITY_ESCALATION` with a note identifying the field. Note: advisory hint fields (`prefer_zones`, `upstream_precomposition`) are not subject to this rule — they may be freely overridden.
- `target_fps` must be >= `min_fps`. Violated → `CONFIG_INVALID_FPS_RANGE`.

---

## 4. Zone Geometry Policies

### 4.1 What a Geometry Policy Does

A geometry policy is a function:

```
fn resolve(zone_type, display_profile, active_zones, tab_layout) -> (Rect, Style)
```

Given the current display profile, the set of already-active zones, and the tab's layout constraints, it returns a screen-space rectangle and a style bundle (font scale, margins, opacity). Agents do not call this function — the runtime calls it when a zone instance is activated and when the display profile or layout changes.

This is the concrete mechanism by which "same scene model, different budgets" works for zone geometry (see heart-and-soul/mobile.md). The agent publishes to `"subtitle"` on both desktop and phone; the policy resolves a different `Rect` on each.

### 4.2 Built-in Policies

All built-in policies accept a `[zones.<name>.policy_overrides]` table for per-profile customization (see §4.3).

#### `bottom_strip`

A horizontal strip anchored to the bottom edge of the content area (below the status bar, above the OS taskbar if in overlay mode).

Default parameters:
- Height: `5%` of display height on `full-display`, `10%` on `mobile`
- Width: `100%` of display width minus horizontal margins
- Horizontal margins: `2%` on `full-display`, `1%` on `mobile`
- Vertical offset from bottom: `2%` on `full-display`, `3%` on `mobile`
- Background opacity: `0.75` (semi-transparent backdrop)
- Text scale: `1.0` on `full-display`, `1.4` on `mobile` (larger for glanceability)

Typical uses: subtitle zone, transcript strip.

#### `top_right_stack`

A vertically stacking list of cards anchored to the top-right corner. New cards push downward.

Default parameters:
- Card width: `20%` of display width on `full-display`, `80%` on `mobile` (full-width banner)
- Card max height: `8%` of display height per card
- Max visible cards: `5` on `full-display`, `2` on `mobile`
- Anchor: top-right corner, `2%` inset on each axis
- Stack direction: downward
- Auto-dismiss: card collapses after `timeout_secs` (configured per zone instance)

Typical uses: notification zone.

#### `full_width_bar`

A thin horizontal bar spanning the full display width. Attaches to the chrome layer.

Default parameters:
- Height: `3%` of display height on `full-display`, `4%` on `mobile`
- Position: top or bottom (determined by `chrome.tab_bar_position`; status bar takes the opposite edge)
- Content: horizontally scrolling key-value pairs
- Always visible: yes (chrome layer)

Typical uses: status-bar zone, tab bar.

#### `corner_anchored`

A floating surface anchored to a named corner of the content area, draggable within bounds.

Default parameters:
- Default anchor: `bottom_right`
- Default size: `20%` × `15%` of display on `full-display`; `30%` × `25%` on `mobile`
- Min size: `10%` × `8%`
- Max size: `40%` × `35%`
- Draggable: yes
- Resizable: within min/max bounds

Typical uses: pip (picture-in-picture) zone.

#### `fullscreen_behind`

Covers the entire display at the background layer (z-order behind all tiles and chrome).

Default parameters:
- Rect: full display bounds
- Layer: background (always — overriding to content or chrome is a validation error)
- No margins, no overlay

Typical uses: ambient-background zone.

#### `top_push`

A full-width bar at the top of the content area that pushes content tiles downward when active, and collapses (tiles return to original position) when dismissed.

The "push" mechanism works by dynamically adjusting `reserved_top_fraction` while the zone bar is active. The zone itself renders in the chrome layer (above all agent tiles), but its height is added to the content layer's reserved top area so agent tiles reflow below it rather than being occluded. When the zone dismisses, `reserved_top_fraction` returns to its configured value and tiles animate back to their prior positions. This is not the same as rendering in the content layer — the agent tiles move; the zone bar does not occlude them.

Default parameters:
- Height: `8%` of display height on `full-display`, `12%` on `mobile`
- Expansion: animated (smooth push) on `full-display`, instant on `mobile`
- Dismiss: swipe up or tap ×
- Layer: chrome (renders above the content layer; reserved area increase drives the tile reflow)

Typical uses: alert-banner zone, urgent notifications.

#### `audio_only`

A zero-size visual surface. The zone renders no visible content. Adjunct effects (sounds, haptics) are still fired. Used for audio-first fallback on smart glasses or during degradation.

Default parameters:
- Visual rect: 0 × 0
- Sound policy: inherited from zone type
- Haptic policy: inherited from zone type

Typical uses: subtitle fallback on glasses profile, notification sound-only mode.

### 4.3 Per-Profile Policy Overrides

Zone instances in a tab definition can override policy parameters per profile:

```toml
[[tabs]]
name = "Subtitles Demo"

[tabs.zones.subtitle]
policy = "bottom_strip"
layer = "content"

# On the mobile profile, use a taller strip.
[tabs.zones.subtitle.profile_overrides.mobile]
height_fraction = 0.15
text_scale = 1.6

# On a custom glasses profile, fall back to audio only.
[tabs.zones.subtitle.profile_overrides.glasses-v1]
policy = "audio_only"
```

If no override exists for the active profile, the policy's built-in defaults apply.

### 4.4 Zone Geometry in Headless Mode

In headless mode (CI, tests), zone geometry resolves against a virtual display. The default virtual display size is `1920x1080`. Tests may override it via:

```toml
[runtime]
headless_width = 1280
headless_height = 720
```

**Cross-reference:** RFC 0002 §7 names these fields `headless_width` and `headless_height` in the `RuntimeConfig` Rust struct. This RFC adopts the same names for consistency. Earlier drafts used `headless_display_width`/`headless_display_height` — those names are rejected.

All geometry fractions are computed against this virtual size. Tests can assert exact pixel coordinates for layout correctness.

---

## 5. Tab and Layout Configuration

### 5.1 Tab Definitions

A tab is a mode of the environment. It defines:
- Its name and display name
- Its default zone instances and their geometry policies
- Its layout mode for agent tiles
- Its tab-switching event trigger
- Its layout constraints

Full example:

```toml
[[tabs]]
name = "Morning"
display_name = "Morning"
icon = "sun"
default_layout = "columns"
default_tab = true
tab_switch_on_event = ""  # No automatic switch; stays until user or agent switches

[tabs.zones]
subtitle      = { policy = "bottom_strip",     layer = "content"    }
notification  = { policy = "top_right_stack",  layer = "content"    }
status_bar    = { policy = "full_width_bar",   layer = "chrome"     }
ambient_background = { policy = "fullscreen_behind", layer = "background" }

[tabs.layout]
min_tile_width_px = 120
min_tile_height_px = 80
max_tile_count = 64
reserved_bottom_fraction = 0.08
reserved_top_fraction = 0.04

[[tabs]]
name = "Security"
display_name = "Security"
icon = "shield"
default_layout = "grid"
tab_switch_on_event = "doorbell.ring"  # Auto-switch when doorbell fires

[tabs.zones]
subtitle     = { policy = "bottom_strip",    layer = "content" }
notification = { policy = "top_right_stack", layer = "content" }
status_bar   = { policy = "full_width_bar",  layer = "chrome"  }
alert_banner = { policy = "top_push",        layer = "chrome"  }

[tabs.layout]
min_tile_width_px = 200
min_tile_height_px = 150
max_tile_count = 16
reserved_bottom_fraction = 0.08
reserved_top_fraction = 0.04
```

### 5.2 Layout Modes

| Mode | Description |
|------|-------------|
| `grid` | Agent tiles snap to an invisible grid. Tiles can span multiple grid cells. Grid cell size derived from `min_tile_width_px` and `min_tile_height_px`. |
| `columns` | Display area divided into equal-width columns. Tiles placed top-to-bottom within columns. |
| `freeform` | Agent tiles can occupy any geometry within the non-reserved area. No snapping. The agent specifies exact coordinates as fractional display dimensions. |

The layout mode is advisory in v1: agents may request specific tile geometry in any mode; the mode shapes how the runtime resolves conflicts and allocates unclaimed space.

### 5.3 Layout Constraints

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `min_tile_width_px` | integer | 80 | Tiles narrower than this are rejected with `TILE_TOO_SMALL`. |
| `min_tile_height_px` | integer | 60 | Tiles shorter than this are rejected with `TILE_TOO_SMALL`. |
| `max_tile_count` | integer | 256 | Tiles beyond this limit are rejected with `TILE_LIMIT_EXCEEDED`. Applied per-tab. |
| `reserved_bottom_fraction` | float (0.0–1.0) | 0.0 | Fraction of display height at the bottom reserved for zones. Agent tiles cannot overlap this area. |
| `reserved_top_fraction` | float (0.0–1.0) | 0.0 | Fraction of display height at the top reserved for zones and chrome. |
| `reserved_left_fraction` | float (0.0–1.0) | 0.0 | Reserved left margin. |
| `reserved_right_fraction` | float (0.0–1.0) | 0.0 | Reserved right margin. |

Reserved fractions are additive with chrome layer zones. If `tab_bar_position = "top"` and `reserved_top_fraction = 0.04`, the effective top reserved area is `tab_bar_height + reserved_top_fraction`.

### 5.4 Tab Switching Policy

The `tab_switch_on_event` field names a scene-level event (see RFC 0004) that automatically activates the tab. This is the mechanism for interrupt-driven tab switching without agent involvement:

```toml
tab_switch_on_event = "doorbell.ring"    # Switch when doorbell rings
tab_switch_on_event = "alert.fire"       # Switch on fire alert
tab_switch_on_event = ""                 # No automatic switch (default)
```

Tab switches triggered by `tab_switch_on_event` are subject to the canonical policy evaluation order defined in architecture.md §"Policy arbitration" (steps 1–7: human override → capability gate → privacy/viewer gate → interruption policy → attention budget → zone contention → resource/degradation budget). The interruption class check (step 4) is what determines whether the tab switch fires during quiet hours: a `doorbell.ring` event carries `interruption_class = "urgent"` and therefore passes through quiet-hours gating. A `morning.routine` event with `interruption_class = "normal"` would be suppressed during quiet hours.

---

## 6. Agent Registration and Capability Grants

### 6.1 Two Registration Modes

**Pre-registered agents** are declared in config. Their identity, authentication method, and initial capability grants are defined at config load time. The runtime knows them before they connect.

**Dynamic agents** are not in config. When a dynamic agent connects, it must authenticate. Its capabilities are negotiated per-session based on the runtime's dynamic grant policy (defined in `[agents.dynamic_policy]`).

### 6.2 Pre-Registered Agent Schema

```toml
[agents.registered.weather_agent]
# Human-readable description. Used in prompts and logs.
description = "Weather display agent"

# Authentication method. One of: "psk", "unix_socket", "mtls", "oidc".
auth_method = "psk"

# For "psk": the name of an environment variable containing the key.
# The key itself is NEVER written into the config file.
auth_psk_env = "TZE_WEATHER_AGENT_KEY"

# For "unix_socket": the socket path the agent must connect from.
# auth_unix_socket_path = "/run/tze_hud/weather_agent.sock"

# For "mtls": the path to the agent's client certificate (PEM).
# auth_mtls_cert_path = "/etc/tze_hud/agents/weather_agent.crt"

# Presence level ceiling. One of: "guest", "resident", "embodied".
# The agent cannot escalate above this level regardless of what it requests.
presence_ceiling = "resident"

# Initial capability grants at connect time (before any user prompt).
# Uses the capability names from security.md.
capabilities = [
  "create_tiles",
  "modify_own_tiles",
  "subscribe_scene_events",
  "publish_zone:subtitle",
  "publish_zone:status_bar",
]

# Resource budget overrides (overrides the profile's per-agent defaults).
# max_texture_mb is in mebibytes (MiB). RFC 0002 §4.3 tracks this as bytes internally;
# the config layer converts: max_texture_mb × 1024 × 1024 = max_texture_bytes.
[agents.registered.weather_agent.budgets]
max_tiles = 4
max_texture_mb = 32
max_update_hz = 10

[agents.registered.doorbell_agent]
description = "Doorbell security agent"
auth_method = "psk"
auth_psk_env = "TZE_DOORBELL_AGENT_KEY"
presence_ceiling = "embodied"  # Will use WebRTC for camera feed (post-v1)
capabilities = [
  "create_tiles",
  "modify_own_tiles",
  "subscribe_scene_events",
  "publish_zone:notification",
  "publish_zone:alert_banner",
  "emit_scene_event:doorbell.ring",  # Can fire the doorbell tab-switch event
]

[agents.registered.doorbell_agent.budgets]
max_tiles = 2
max_texture_mb = 512  # Camera feed budget (post-v1)
max_update_hz = 60
```

### 6.3 Capability Identifiers

The capability grant list in each agent entry uses structured capability identifiers:

| Identifier | Description |
|------------|-------------|
| `create_tiles` | May request tile leases. |
| `modify_own_tiles` | May mutate content on own tiles. |
| `read_scene_topology` | May query the full scene topology, including other agents' lease metadata. |
| `subscribe_scene_events` | May subscribe to scene-level events (tab switches, agent joins/departs). |
| `overlay_privileges` | May request tiles with high z-order or overlay-level positions. |
| `access_input_events` | May receive input events forwarded from the runtime. |
| `high_priority_z_order` | May request z-order values in the top quartile. |
| `exceed_default_budgets` | May request budget overrides at session time (requires user prompt). |
| `publish_zone:<zone_name>` | May publish to the named zone. One grant per zone. `publish_zone:*` grants all zones. |
| `emit_scene_event:<event_name>` | May fire the named scene event (e.g., `doorbell.ring`). |

Capabilities not listed are not granted. There is no wildcard for tile/topology access — only `publish_zone:*` is supported as a wildcard form.

### 6.4 Dynamic Agent Policy

```toml
[agents.dynamic_policy]
# Whether to accept connections from agents not in [agents.registered].
# Default: false (all agents must be pre-registered).
allow_dynamic_agents = true

# Default capabilities granted to dynamic agents at connect time.
# Dynamic agents start with this set and may request more (subject to user prompt).
default_capabilities = [
  "create_tiles",
  "modify_own_tiles",
  "publish_zone:notification",
]

# Whether dynamic agents require a user prompt for elevated capabilities.
# Default: true
prompt_for_elevated_capabilities = true

# Presence level ceiling for dynamic agents.
# Default: "resident"
dynamic_presence_ceiling = "resident"

# Default resource budgets for dynamic agents.
[agents.dynamic_policy.default_budgets]
max_tiles = 8
max_texture_mb = 128
max_update_hz = 30
```

### 6.5 Authentication Notes

Secrets are **never** written into the config file. PSK-based auth references an environment variable name; the runtime reads the key from the environment at startup. If the environment variable is unset, the registration is logged as a warning and the agent cannot authenticate until the variable is set.

mTLS certificate paths point to files on disk, not inline PEM. This enables rotation without config reload.

OAuth2/OIDC configuration (issuer URL, client ID, required claims) is deferred to a dedicated `[agents.oidc]` section in a post-v1 RFC.

---

## 7. Privacy and Degradation Policies

### 7.1 Privacy Configuration

```toml
[privacy]
# Default content visibility classification when an agent does not declare one.
# One of: "public", "household", "private", "sensitive".
# Default: "private" (fail closed).
default_classification = "private"

# Default viewer class when no viewer is identified.
# One of: "owner", "household_member", "known_guest", "unknown", "nobody".
# Default: "unknown"
default_viewer_class = "unknown"

# Viewer identification method. One of: "none", "face_recognition", "proximity_badge",
# "phone_presence", "explicit_login". Pluggable; the runtime defines a trait.
# Default: "none" (viewer class always equals default_viewer_class).
viewer_id_method = "none"

# Redaction style for tiles whose classification exceeds the viewer's access.
# One of: "pattern", "agent_name", "icon", "blank".
# Default: "pattern"
redaction_style = "pattern"

# Multi-viewer policy: when multiple viewers with different access levels are present.
# One of: "most_restrictive", "least_restrictive".
# Default: "most_restrictive"
multi_viewer_policy = "most_restrictive"

# Whether the owner can override multi-viewer restriction with an explicit action.
# Default: true
owner_can_override_multi_viewer = true
```

#### Quiet Hours

```toml
[privacy.quiet_hours]
# Whether quiet hours are enabled.
# Default: false
enabled = true

# Quiet hours schedule. Array of time ranges.
[[privacy.quiet_hours.schedule]]
start = "22:00"  # 24-hour local time
end = "07:00"    # Wraps midnight automatically

# Days of week this schedule applies. Array of: "mon", "tue", "wed", "thu", "fri", "sat", "sun".
# Default: all days.
days = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]

# During quiet hours, which interruption classes are allowed through immediately.
# Content below this threshold is queued until quiet hours end.
# One of: "silent", "gentle", "normal", "urgent", "critical".
# Default: "urgent" (only urgent and critical pass through).
pass_through_class = "urgent"

# What the screen does during quiet hours.
# One of: "dim", "clock_only", "off".
# Default: "dim"
quiet_mode_display = "dim"

# Screen brightness during quiet hours (0.0–1.0).
# Default: 0.1
dim_level = 0.1
```

**Time format:** `HH:MM` in 24-hour local wall clock. Ranges that span midnight (e.g., `22:00`–`07:00`) are handled correctly. Multiple `[[privacy.quiet_hours.schedule]]` entries are unioned.

**Validation rules:**
- `default_classification` must be one of: `"public"`, `"household"`, `"private"`, `"sensitive"`. Unknown → `CONFIG_UNKNOWN_CLASSIFICATION`.
- `default_viewer_class` must be one of: `"owner"`, `"household_member"`, `"known_guest"`, `"unknown"`, `"nobody"`. The `"nobody"` class is valid and means "screen detects no viewer present" (see privacy.md). Unknown → `CONFIG_UNKNOWN_VIEWER_CLASS`.
- `start` and `end` must be valid `HH:MM` strings. Invalid → `CONFIG_INVALID_TIME`.
- `pass_through_class` must be one of the enumerated values. Unknown → `CONFIG_UNKNOWN_INTERRUPTION_CLASS`.

### 7.2 Degradation Configuration

```toml
[degradation]
# Whether automatic degradation is enabled.
# Default: true
enabled = true

# Polling interval for resource pressure sampling (milliseconds).
# Default: 1000
sample_interval_ms = 1000

# Hysteresis: a degradation level is not exited until pressure stays below the
# exit threshold for this many consecutive sample intervals.
# Default: 5
hysteresis_samples = 5
```

#### Degradation Thresholds

Each threshold defines the resource pressure level at which the corresponding degradation step activates.

```toml
[degradation.thresholds]
# Step 1: coalesce more aggressively.
# Triggers when frame time p99 exceeds this value (ms).
coalesce_frame_ms = 12.0

# Step 2: reduce media quality (post-v1 — video resolution/frame rate).
# Triggers when GPU memory utilization exceeds this fraction (0.0–1.0).
reduce_media_quality_gpu_fraction = 0.75

# Step 3: reduce concurrent streams (post-v1).
# Triggers when GPU memory utilization exceeds this fraction.
reduce_concurrent_streams_gpu_fraction = 0.85

# Step 4: simplify rendering (disable transitions, effects, blending).
# Triggers when frame time p99 exceeds this value (ms).
simplify_rendering_frame_ms = 14.0

# Step 5: shed tiles (collapse low-priority tiles).
# Triggers when frame time p99 exceeds this value (ms) for more than hysteresis_samples.
shed_tiles_frame_ms = 16.0

# Step 6: audio-first fallback (post-v1 — disable display pipeline, audio only).
# Triggers when frame time p99 exceeds this value (ms) for more than 3× hysteresis_samples.
audio_only_frame_ms = 20.0
```

#### Degradation Ladder Steps

Each step can be individually enabled/disabled and tuned:

```toml
[degradation.ladder]

[degradation.ladder.coalesce]
enabled = true
# Target update frequency after coalescing (Hz, per agent).
target_hz = 10

[degradation.ladder.reduce_media_quality]
enabled = true  # No-op in v1 (no media pipeline)
target_resolution_fraction = 0.5  # 50% of native resolution
target_fps = 30

[degradation.ladder.reduce_concurrent_streams]
enabled = true  # No-op in v1
max_streams_at_level = 1

[degradation.ladder.simplify_rendering]
enabled = true
disable_transitions = true
disable_alpha_blending = false  # Keep blending; disable animated effects
disable_effects = true

[degradation.ladder.shed_tiles]
enabled = true
# Tile priority field used for shedding order.
# One of: "z_order", "agent_priority", "last_updated".
# Default: "agent_priority"
shed_priority = "agent_priority"

[degradation.ladder.audio_only]
enabled = false  # Disabled by default; opt-in for headless/glasses deployments
```

**Validation rules:**
- Threshold values must be positive. Zero or negative → `CONFIG_INVALID_THRESHOLD`.
- `shed_priority` must be one of the enumerated values. Unknown → `CONFIG_UNKNOWN_SHED_PRIORITY`.
- Frame-time thresholds must be monotonically non-decreasing in ladder order: `coalesce_frame_ms` ≤ `simplify_rendering_frame_ms` ≤ `shed_tiles_frame_ms` ≤ `audio_only_frame_ms`. Violation → `CONFIG_DEGRADATION_THRESHOLD_ORDER` identifying the pair that is out of order. (Rationale: the ladder is designed lightest-first per failure.md §Degradation axes; out-of-order thresholds cause heavy steps to fire before lighter ones have been tried.)
- GPU fraction thresholds must be monotonically non-decreasing: `reduce_media_quality_gpu_fraction` ≤ `reduce_concurrent_streams_gpu_fraction`. Violation → `CONFIG_DEGRADATION_THRESHOLD_ORDER`.
- Ladder steps that reference post-v1 features (media, audio) are accepted in the schema but noted as no-ops in v1 logs at startup.

---

## 8. Schema Export

The runtime exports its full configuration JSON Schema in two ways:

### `--print-schema` CLI flag (recommended for tooling)

```bash
tze_hud --print-schema > tze_hud-config-schema.json
```

Prints the JSON Schema to stdout and **exits immediately**. The runtime does not start, does not bind ports, and does not initialize a GPU context. This is the safe, non-destructive path for CI pipelines, editors, and LLM tooling. If both `--print-schema` and `emit_schema = true` are present, `--print-schema` takes precedence.

### `emit_schema = true` in `[runtime]` (for logging pipelines)

When `emit_schema = true`, the runtime writes the JSON Schema to stdout **once at startup, then continues running**. Useful for log pipelines that capture startup output. Not suitable for tooling that expects an early-exit schema dump.

The schema is generated via `schemars` from the Rust config types, including `ConfigErrorCode` variants exported as a JSON Schema `enum` for tooling. The schema is stable within a major version and versioned with the runtime. LLMs writing config files should use this schema as their primary reference. The schema includes the error code for each validation constraint as a custom annotation (`x-error-code`).

---

## 9. Configuration Reload

The runtime supports live configuration reload for a subset of fields:

```bash
kill -HUP <runtime_pid>
```

Or via gRPC: `RuntimeService.ReloadConfig`.

**Hot-reloadable fields:**
- `[privacy]` — viewer classes, quiet hours, redaction style
- `[degradation]` — thresholds and ladder step settings
- `[chrome]` — display preferences (tab bar visibility, indicator display)
- `[agents.dynamic_policy]` — dynamic agent policy defaults

**Requires restart:**
- `[runtime]` — bind addresses, profile selection, window mode
- `[[tabs]]` — tab definitions (scene is ephemeral; tabs change the scene structure)
- `[agents.registered]` — pre-registered agent identities and capability grants

On reload, the runtime re-validates the config file. Validation errors are returned (via gRPC reload response) without applying the new config. The running config remains active. There is no partial reload.

---

## 10. Rust Types

The configuration is deserialized into these Rust types. These types are the authoritative schema; the TOML examples above are derived from them.

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Config {
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub display_profile: DisplayProfileConfig,
    #[serde(default)]
    pub tabs: Vec<TabConfig>,
    #[serde(default)]
    pub zones: ZoneRegistryConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub privacy: PrivacyConfig,
    #[serde(default)]
    pub degradation: DegradationConfig,
    #[serde(default)]
    pub chrome: ChromeConfig,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RuntimeConfig {
    pub profile: ProfileName,
    #[serde(default = "WindowMode::default")]
    pub window_mode: WindowMode,
    #[serde(default = "default_grpc_bind")]
    pub grpc_bind: SocketAddr,
    #[serde(default = "default_mcp_bind")]
    pub mcp_bind: SocketAddr,
    #[serde(default)]
    pub log_level: LogLevel,
    #[serde(default = "default_reconnect_grace")]
    pub reconnect_grace_secs: u32,
    #[serde(default)]
    pub emit_schema: bool,
    /// Virtual display width for headless mode (pixels). Default: 1920.
    /// Matches RFC 0002 §7 field name `headless_width`.
    #[serde(default = "default_headless_width")]
    pub headless_width: u32,
    /// Virtual display height for headless mode (pixels). Default: 1080.
    /// Matches RFC 0002 §7 field name `headless_height`.
    #[serde(default = "default_headless_height")]
    pub headless_height: u32,
}

/// A display profile: resource budgets and rendering constraints.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DisplayProfile {
    pub name: ProfileName,
    pub max_tiles: u32,
    /// Texture memory budget in mebibytes. Config uses MB for human-editability;
    /// the compositor converts to bytes internally. (RFC 0002 §4.3 uses `max_texture_bytes`.)
    pub max_texture_mb: u32,
    pub max_agents: u32,
    pub target_fps: u32,
    pub min_fps: u32,
    pub allowed_node_types: Vec<NodeTypeId>,
    pub allowed_window_modes: Vec<WindowMode>,
    pub max_media_streams: u32,
    pub max_agent_update_hz: u32,
    pub allow_background_zones: bool,
    pub allow_chrome_zones: bool,
    /// Advisory hint to orchestrators: prefer zones over raw tiles.
    /// Used in mobile profile. Does not affect budget enforcement.
    #[serde(default)]
    pub prefer_zones: bool,
    /// Allow upstream precomposition of certain layers (post-v1, mobile only).
    /// When false (default), composition is always local.
    #[serde(default)]
    pub upstream_precomposition: bool,
}

/// The user-editable `[display_profile]` section. Either names a built-in to use
/// as-is (via `[runtime].profile`) or extends one with field overrides.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct DisplayProfileConfig {
    /// If set, the named built-in profile is loaded and then the remaining fields
    /// in this section override it. Must name a built-in profile (`full-display` or `mobile`).
    /// Extending the `headless` profile or another custom profile is not supported.
    #[serde(default)]
    pub extends: Option<ProfileName>,
    // All DisplayProfile fields are optional overrides here.
    #[serde(default)]
    pub max_tiles: Option<u32>,
    #[serde(default)]
    pub max_texture_mb: Option<u32>,
    #[serde(default)]
    pub max_agents: Option<u32>,
    #[serde(default)]
    pub target_fps: Option<u32>,
    #[serde(default)]
    pub min_fps: Option<u32>,
    #[serde(default)]
    pub max_media_streams: Option<u32>,
    #[serde(default)]
    pub max_agent_update_hz: Option<u32>,
    #[serde(default)]
    pub allow_background_zones: Option<bool>,
    #[serde(default)]
    pub allow_chrome_zones: Option<bool>,
}

/// Typed validation error codes. Each variant corresponds to one entry in the
/// Summary of Validation Error Codes table. Using an enum (rather than `&'static str`)
/// ensures compile-time exhaustiveness at match sites and allows `schemars` to
/// enumerate all valid codes in the exported JSON Schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfigErrorCode {
    ConfigUnknownProfile,
    ConfigInvalidAddress,
    ConfigDuplicateTabName,
    ConfigMultipleDefaultTabs,
    ConfigUnknownLayout,
    ConfigUnknownZoneType,
    ConfigUnknownGeometryPolicy,
    ConfigIncompatibleZoneLayer,
    ConfigUnknownBaseProfile,
    ConfigHeadlessNotExtendable,
    ConfigProfileBudgetEscalation,
    ConfigProfileCapabilityEscalation,
    ConfigProfileExtendsConflictsWithProfile,
    ConfigInvalidFpsRange,
    ConfigUnknownClassification,
    ConfigUnknownViewerClass,
    ConfigInvalidTime,
    ConfigUnknownInterruptionClass,
    ConfigInvalidThreshold,
    ConfigUnknownShedPriority,
    ConfigDegradationThresholdOrder,
}

/// A validation error with structured fields.
#[derive(Debug, Serialize)]
pub struct ConfigError {
    /// Stable, typed error code. Use `ConfigErrorCode` variants, not raw strings.
    pub code: ConfigErrorCode,
    pub field_path: String,      // e.g., "runtime.profile"
    pub expected: String,
    pub got: String,
    pub hint: String,
}
```

Full type definitions for `TabConfig`, `ZoneInstanceConfig`, `AgentRegistrationConfig`, `PrivacyConfig`, `DegradationConfig`, `ChromeConfig`, and their sub-types follow the patterns shown above. These types derive `serde::Deserialize`, `serde::Serialize`, and `schemars::JsonSchema`. All optional fields use `#[serde(default)]` with documented default values.

---

## 11. Implementation Notes

### 11.1 Config Crate

Configuration parsing and validation lives in a standalone crate `crates/tze-config`. It has no dependency on the compositor, scene graph, or GPU. This enables:
- Pure unit tests for validation logic (no runtime needed)
- Schema export without starting the runtime
- LLM-driven config generation and validation tooling

### 11.2 Load Sequence

1. CLI / environment variable resolution to config file path
2. File read (structured I/O error if file not found or not readable)
3. TOML parse (syntax error with line/column if parse fails)
4. Schema validation (all errors collected, then returned together)
5. Profile resolution (built-in lookup or extension merge)
6. Defaults injection for missing optional fields
7. Delivery of `Config` struct to runtime

Steps 3–6 are pure functions. Step 7 is the only point where the runtime takes ownership.

### 11.3 Test Fixtures

`tests/config/` contains:
- `minimal.toml` — one tab, all defaults, must parse without errors
- `full-display.toml` — comprehensive full-display config exercising all sections
- `mobile.toml` — mobile profile with per-profile policy overrides
- `invalid/*.toml` — one fixture per error code, asserting the exact structured error
- `reload/` — fixtures for testing hot-reload behavior

---

## Open Questions

1. **Custom zone type discovery.** Should the runtime enumerate custom zone types in the `list_zones` gRPC/MCP response? Currently yes (all zones, built-in and custom, are listed). This exposes deployment-specific configuration to agents, which is intentional — agents need to discover what zones are available.

2. **Profile auto-detection heuristics.** The auto-detection criteria in §3.5 (GPU VRAM threshold, display refresh rate) may need tuning once real mobile hardware targets are defined. The heuristics are conservative defaults; deployments should prefer explicit `profile =` settings.

3. **Tab order persistence.** Tab order is defined by the order of `[[tabs]]` entries in the config. If the user reorders tabs via the UI (a future feature), should that reordering persist across restarts? For v1, tab order is config-only. Persistence is deferred.

4. **Secret management for PSK keys.** The current design reads keys from environment variables. Deployments with more complex secret management needs (Vault, key files, etc.) will need a post-v1 extension. The auth trait is designed to support this.

---

## Summary of Validation Error Codes

| Code | Section | Trigger |
|------|---------|---------|
| `CONFIG_UNKNOWN_PROFILE` | §2.2 | `runtime.profile` names an unknown profile |
| `CONFIG_INVALID_ADDRESS` | §2.2 | `grpc_bind` or `mcp_bind` is not a valid socket address |
| `CONFIG_DUPLICATE_TAB_NAME` | §2.4 | Two `[[tabs]]` entries share a name |
| `CONFIG_MULTIPLE_DEFAULT_TABS` | §2.4 | More than one tab sets `default_tab = true` |
| `CONFIG_UNKNOWN_LAYOUT` | §2.4 | `default_layout` is not a known layout mode |
| `CONFIG_UNKNOWN_ZONE_TYPE` | §2.4 | A zone instance references an undefined zone type |
| `CONFIG_UNKNOWN_GEOMETRY_POLICY` | §2.4 | A zone instance references an undefined geometry policy |
| `CONFIG_INCOMPATIBLE_ZONE_LAYER` | §2.4 | Zone layer override is incompatible with zone type |
| `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE` | §2.3, §3.5 | `[display_profile].extends` names a different built-in than `[runtime].profile` |
| `CONFIG_UNKNOWN_BASE_PROFILE` | §3.6 | Custom profile `extends` an unknown built-in |
| `CONFIG_HEADLESS_NOT_EXTENDABLE` | §3.6 | Custom profile attempts `extends = "headless"` |
| `CONFIG_PROFILE_BUDGET_ESCALATION` | §3.6 | Custom profile numeric override exceeds base profile value |
| `CONFIG_PROFILE_CAPABILITY_ESCALATION` | §3.6 | Custom profile enables a boolean capability the base profile disables (`allow_background_zones`, `allow_chrome_zones`) |
| `CONFIG_INVALID_FPS_RANGE` | §3.6 | `target_fps < min_fps` |
| `CONFIG_UNKNOWN_CLASSIFICATION` | §7.1 | Unknown `default_classification` value |
| `CONFIG_UNKNOWN_VIEWER_CLASS` | §7.1 | Unknown `default_viewer_class` value |
| `CONFIG_INVALID_TIME` | §7.1 | Quiet hours `start` or `end` is not valid `HH:MM` |
| `CONFIG_UNKNOWN_INTERRUPTION_CLASS` | §7.1 | Unknown `pass_through_class` value |
| `CONFIG_INVALID_THRESHOLD` | §7.2 | Degradation threshold is zero or negative |
| `CONFIG_UNKNOWN_SHED_PRIORITY` | §7.2 | Unknown `shed_priority` value |
| `CONFIG_DEGRADATION_THRESHOLD_ORDER` | §7.2 | Degradation thresholds are not monotonically non-decreasing (heavier step fires before lighter step) |
