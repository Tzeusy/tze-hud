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

**[CONSIDER]** `profile = "auto"` failure path now specified: falls back to `mobile` with a `WARN` log. *(Superseded by Round 6: the auto-detection fallback was changed to `headless`; `mobile` is not a v1 auto-detection target.)*

**No dimension below 3. Round 2 complete.**

---

### Round 3 — Cross-RFC Consistency (rig-5vq.33)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** security.md, privacy.md
**Related RFCs reviewed:** RFC 0005 (Session Protocol), RFC 0008 (Lease Governance), RFC 0009 (Policy Arbitration)

#### Doctrinal Alignment: 4/5
No doctrinal regressions from prior rounds.

#### Technical Robustness: 4/5
No new technical regressions found. Cross-references corrected; vocabulary clarified.

#### Cross-RFC Consistency: 4/5 (after fixes)

**[MUST-FIX → FIXED]** `redaction_style` appeared in `[chrome]` section (§2.8) — RFC 0009 §3.2 explicitly mandates it belongs exclusively in `[privacy]`. Removed from §2.8 `ChromeConfig` TOML example; added clarifying note that `ChromeConfig` must not contain this field.

**[MUST-FIX → FIXED]** RFC 0006 cross-references section was missing RFC 0005, RFC 0008, and RFC 0009. These RFCs were created after the original RFC 0006 draft, but RFC 0009 directly modifies RFC 0006 (redaction_style ownership) and RFC 0005/0008 define the capability vocabulary this RFC depends on. Added all three to the Cross-References section with precise interaction descriptions.

**[MUST-FIX → FIXED]** Capability identifier table (§6.3) was missing two capabilities defined in other RFCs: `read_telemetry` (RFC 0005 §7.1, required to subscribe to `telemetry_frames` category) and `lease:priority:<N>` (RFC 0008 §2.1, permits requesting lease priority above default). Both added to the table with authoritative RFC citations. Also added a note clarifying that RFC 0001's uppercase/colon-format capability names (`CREATE_TILE`, `zone:publish:`) are illustration-only; the canonical wire-format strings are the lowercase underscore forms in this table (established by RFC 0005 as the protocol source of truth per PR #42).

**[SHOULD-FIX → FIXED]** `reconnect_grace_secs` lacked cross-reference to the same parameter's definition in RFC 0005 §8 (`reconnect_grace_period_ms`, 30,000ms default) and RFC 0008 §3.3 ("orphan grace period"). Added inline comment noting the equivalence.

**No dimension below 3. Round 3 complete.**

---

### Round 4 — Final Hardening and Quantitative Verification (rig-5vq.34)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, v1.md, failure.md, validation.md

#### Doctrinal Alignment: 4/5
No doctrinal regressions from prior rounds. All architectural commitments remain faithfully implemented.

#### Technical Robustness: 4/5 (after fixes)

**[MUST-FIX → FIXED]** §2.1 required "at least one `[[tabs]]` entry" with no enforcement. Added `CONFIG_NO_TABS` validation rule to §2.4, added `ConfigNoTabs` variant to the `ConfigErrorCode` enum, and added the row to the Summary of Validation Error Codes table.

**[MUST-FIX → FIXED]** §5.3 `reserved_*_fraction` fields documented as float (0.0–1.0) but had no validation rule or error code. Added `CONFIG_INVALID_RESERVED_FRACTION` covering out-of-range individual values and horizontal/vertical sum ≥ 1.0 (which would leave zero space for agent tiles). Added to enum and summary table.

**[MUST-FIX → FIXED]** §6.2 agent budgets had no cross-profile validation. Per-agent `max_tiles`, `max_texture_mb`, and `max_update_hz` could silently exceed the active profile ceiling. Added `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE` validation rules, added to enum and summary table. Also corrected the `doorbell_agent` example `max_texture_mb = 512` (which exceeded the `full-display` profile ceiling of 2048 but was inconsistent with the mobile profile ceiling of 256) to `max_texture_mb = 128` with a clarifying comment.

**[SHOULD-FIX → FIXED]** §5.4 `tab_switch_on_event` unknown names were silently ignored with no documented behavior. Added explicit WARN log behavior for unrecognized event names (not a hard error, for forward compatibility with post-v1 custom events). An empty string is documented as valid with no warning.

**[SHOULD-FIX → FIXED]** §10 `DisplayProfileConfig.extends` doc comment listed only `full-display` and `mobile` as built-ins, omitting `headless`. Updated to clarify `headless` is a built-in but cannot be used as an `extends` base, pointing to `CONFIG_HEADLESS_NOT_EXTENDABLE`.

#### Cross-RFC Consistency: 5/5
No new inconsistencies. All prior-round fixes remain intact. Cross-RFC score raised to 5/5: all identified gaps are now closed.

**No dimension below 3. Round 4 complete. All scores ≥ 4.**

---

### Round 6 — Mobile Profile Doctrinal Alignment (rig-hix)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-hix
**Doctrine files reviewed:** v1.md §"Mobile", architecture.md §"Display profiles", mobile.md §"Two profiles, one model"

#### Finding: Mobile profile was active in v1, contradicting doctrine

**[MUST-FIX → FIXED]** RFC 0006 §3.1 listed `mobile` as a v1 built-in runtime profile alongside `full-display` and `headless`. RFC 0006 §3.3 provided a fully-specified mobile profile with concrete budget values. RFC 0006 §3.5 auto-detection fell back to `mobile` for low-resource hardware. This directly contradicts:
- v1.md §"Mobile": "No mobile build target. V1 is desktop/server only (Linux, Windows, macOS)."
- v1.md §"Mobile": "Mobile capability negotiation is designed into the API but not exercised."
- architecture.md §"Display profiles": "Mobile profiles are designed into the schema but not exercised until post-v1."

**Fixes applied:**

1. **§3.1 Profile table:** Added a `v1 Status` column. `mobile` is now documented as "Schema-reserved (post-v1)" with a cross-reference to §3.3. `full-display` and `headless` remain "Active". A prose note explains the doctrinal position.

2. **§3.3 Mobile profile:** Added a doctrinal callout at the top of the section stating that `mobile` is schema-reserved in v1, the Rust struct exists for forward compatibility, and `profile = "mobile"` at runtime emits `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` and refuses to start. Added a full error message example. Added a note that implementers must not test against mobile budget values in v1.

3. **§3.5 Profile negotiation:** Fixed the auto-detection fallback. "No local GPU or refresh < 60Hz" now falls back to `headless`, not `mobile`. Added an explicit rule: "`mobile` is never selected by auto-detection in v1." Added a note that `extends = "mobile"` is syntactically valid for custom profiles but does not activate absent mobile runtime paths.

4. **§3.6 Custom profiles:** Added a prose note to the `extends = "mobile"` example clarifying it is post-v1; in v1 it produces a custom profile that does not activate mobile-specific runtime paths.

5. **§4.2 Built-in policies:** Added a v1 note at the top of the section. All mobile-specific default parameter values are now annotated `(post-v1)` inline to prevent v1 implementers from writing tests against them.

6. **§4.3 Per-profile policy overrides:** Added a comment to the `profile_overrides.mobile` example block noting it is post-v1 and will never activate in v1 builds.

7. **§2.2 Validation rules:** Added a rule distinguishing `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` (mobile is recognized but refused) from `CONFIG_UNKNOWN_PROFILE` (name not known at all).

8. **§2.9 Error example:** Fixed `CONFIG_UNKNOWN_PROFILE` example — updated `Expected` to list `full-display, headless` (not `full-display, mobile`).

9. **§10 Rust types:** Added `ConfigMobileProfileNotExercised` variant to `ConfigErrorCode` enum with a doc comment.

10. **§10 `DisplayProfileConfig.extends` doc comment:** Added note that `extends = "mobile"` is valid syntax but post-v1 behaviorally.

11. **Summary of Validation Error Codes:** Added `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` row.

12. **§11.3 Test fixtures:** Renamed `mobile.toml` to `mobile-reserved.toml` and updated its description to test the error case, not mobile budget values.

**No doctrinal regressions. Round 6 complete.**

---

### Round 7 — Viewer ID Method Pipeline Design Note (rig-6x2)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-6x2
**Doctrine files reviewed:** privacy.md §"Viewer context"

#### Finding: `viewer_id_method` is a single string — pipeline design direction undocumented

**[DESIGN NOTE — P3]** RFC 0006 §7.1 defines `viewer_id_method` as a single string enum. In practice, deployments with multiple identification signals (face recognition as primary, phone proximity as fallback, explicit login as override) cannot express multi-signal identification under this schema. privacy.md §"Viewer context" explicitly states: "The runtime defines a trait for viewer identity and lets the deployment plug in what's appropriate." A pipeline model is the natural expression of that trait-based design.

Prior review feedback noted: "Let viewer identification be a pipeline of detectors instead of single `viewer_id_method` string."

This round documents the pipeline design direction and the migration path from the v1 single-string form. No v1 behavior is changed.

**Changes applied:**

1. **§7.1 Viewer Identification:** Expanded the `viewer_id_method` description with:
   - A note that single-string form is the v1 implementation.
   - A post-v1 design note showing the `[[privacy.viewer_detectors]]` pipeline syntax with `method`, `priority`, and `confidence_threshold` fields.
   - Pipeline semantics: detectors are evaluated in ascending priority order (lower number = higher priority); the first detector returning a confident identification wins; if no detector identifies a viewer, `default_viewer_class` applies.
   - A compatibility note: the single-string form is a convenience shorthand for single-detector deployments; a v1 parser treats `viewer_id_method = "face_recognition"` as a pipeline of length 1 with default threshold.
   - Validation rules for the detector array (post-v1): duplicate priorities, unknown method names, `confidence_threshold` out of range.

2. **§10 Rust types:** Added `ViewerDetectorConfig` struct and `ViewerIdConfig` enum (design note for post-v1) covering both the shorthand string form and the full pipeline array form.

3. **Open Questions:** Added Open Question 6 about timing of the pipeline promotion and whether a dedicated RFC should own the detector plugin contract.

**No v1 behavior changed. No doctrinal regressions. Round 7 complete.**

---

### Round 5 — Scene-Event Taxonomy (rig-f52)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-f52
**Doctrine files reviewed:** presence.md §"Inter-agent events", v1.md §"Advanced presence", architecture.md §"Policy arbitration"

#### Finding: RFC 0004 cross-reference was incorrect for `tab_switch_on_event`

**[MUST-FIX → FIXED]** §5.4 cited RFC 0004 (Input) as the source of scene-level event names. RFC 0004 owns pointer, touch, keyboard, focus, and gesture routing — not named domain events like `doorbell.ring` or `alert.fire`. The examples in §5.4 and the `emit_scene_event` capability in §6.3 reveal a scene-event bus contract that has no canonical definition in any RFC.

**Fixes applied:**

1. **Cross-References section:** Corrected RFC 0004 entry to note it owns input routing only and does not define the scene-event bus. Added presence.md §"Inter-agent events" (doctrine source) and v1.md §"Advanced presence" (v1 scope constraint) as explicit cross-references.

2. **§5.4 Tab Switching Policy:** Changed `(see RFC 0004)` to `(see §5.5)`. Changed the unknown-event-name WARN prose to reference `§5.5` instead of `RFC 0004 §scene-level events` (the latter section does not exist in RFC 0004).

3. **§5.5 Scene-Event Taxonomy (new section):** Defines the v1 scene-event contract inline, scoped to what v1.md permits:
   - Event naming convention: `<source>.<action>`, dotted lowercase hierarchy.
   - V1 event categories: system events (`system.*`, runtime-emitted, reserved prefix), scene topology events (`scene.*`, runtime-emitted), and agent-emittable named events (require `emit_scene_event:<name>` capability).
   - V1 event payload: `event_name`, `emitted_at_us`, `source_agent_id`.
   - Validation rules: `CONFIG_INVALID_EVENT_NAME` for malformed `tab_switch_on_event` values; `CONFIG_RESERVED_EVENT_PREFIX` for capability grants attempting to use `system.` or `scene.` prefix.

4. **§6.3 Capability Identifiers:** Updated `subscribe_scene_events` description to clarify it is the scene-event bus capability (not input). Updated `emit_scene_event` description to cite §5.5 naming convention and reserved prefix constraint. Added a boxed note distinguishing scene-event bus from input routing.

5. **§10 Rust types:** Added `ConfigInvalidEventName` and `ConfigReservedEventPrefix` to `ConfigErrorCode` enum.

6. **Open Question 5 (new):** Added open question about whether scene events should have a dedicated RFC 0010 as the contract grows post-v1.

7. **Summary of Validation Error Codes:** Added rows for `CONFIG_INVALID_EVENT_NAME` and `CONFIG_RESERVED_EVENT_PREFIX`.

**No doctrinal regressions. Round 5 complete.**

---

### Round 6 — Headless Auto-Detection Branch (rig-6hz)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-6hz
**Doctrine files reviewed:** v1.md §"V1 must prove", architecture.md §"Display profiles"

#### Finding: §3.5 auto-detection had no headless branch

**[MUST-FIX → FIXED]** §3.5 auto-detection had two branches (`full-display` and `mobile`) with no headless path. In CI or server environments (no display server, software-only rendering, or container environment), detection fell through to `mobile` — semantically incorrect for headless operation, and `mobile` is not exercised in v1.

**Fixes applied:**

1. **§3.5 Auto-detection rewritten as three ordered steps:**
   - Step 1 (headless check, evaluated first): selects `headless` if `$DISPLAY`/`$WAYLAND_DISPLAY` are unset on Linux/macOS, Win32 display is unreachable on Windows, `/.dockerenv` exists (container hint), or wgpu reports software-only rendering. Logs an `INFO` entry naming the detected signal.
   - Step 2 (full-display check): selects `full-display` if GPU VRAM > 4 GB and display refresh >= 60 Hz (unchanged from prior spec).
   - Step 3 (failure): if neither condition matches, log `WARN` and abort — requires explicit `profile =`. Mobile is no longer a silent auto-detection fallback.

2. **CI guidance note added** in §3.5: CI pipelines should set `profile = "headless"` explicitly; auto-detection will do the right thing when no display is present, but explicit config is more robust across environments with virtual displays.

3. **`mobile` reservation note added** in §3.5: explains why `mobile` is excluded from auto-detection (cannot distinguish mobile hardware from degraded desktop without device-class signals unavailable at startup).

4. **Open Question 2 updated** to reflect the headless detection signals and the broader set of heuristics now included.

**No doctrinal regressions. Round 6 complete.**

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
- RFC 0004 (Input) — input routing, focus, gesture model; note: RFC 0004 does **not** define the scene-event bus — see §5.4 and §5.5 of this RFC
- RFC 0005 (Session Protocol) — canonical capability identifier vocabulary (§7.1), `reconnect_grace_period_ms` (§8), `subscribe_scene_events` / `emit_scene_event` capability grants
- RFC 0008 (Lease Governance) — capability scope format, `lease:priority:<N>` capability, reconnect grace period semantics
- RFC 0009 (Policy Arbitration) — `redaction_style` ownership (§3.2 mandates `[privacy]` as canonical section), arbitration stack (§1) that this config feeds
- heart-and-soul/architecture.md — configuration model doctrine, policy arbitration order
- heart-and-soul/mobile.md — two profiles, one model
- heart-and-soul/presence.md — zones, geometry, layer attachment; §"Inter-agent events" is the doctrine source for the scene-event bus
- heart-and-soul/privacy.md — viewer classes, quiet hours, content classification
- heart-and-soul/failure.md — degradation axes and ladder
- heart-and-soul/security.md — agent authentication, capability scopes
- heart-and-soul/v1.md — §"Advanced presence": "No inter-agent event bus beyond basic scene topology changes" — defines v1 scene-event scope

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
# Which display profile to use. One of: "full-display", "headless", or a custom
# profile name defined in [display_profile]. Required.
# Note: "mobile" is recognized but is not a valid v1 runtime target; it produces
# CONFIG_MOBILE_PROFILE_NOT_EXERCISED. See §3.3.
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
# Default: 30 (= 30,000ms; matches RFC 0005 §8 `reconnect_grace_period_ms` default)
# Cross-reference: RFC 0008 §3.3 calls this the "orphan grace period"; value is consistent.
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
- `profile = "mobile"` is a hard error in v1 → structured error `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` (see §3.3). This is distinct from `CONFIG_UNKNOWN_PROFILE` — `mobile` is a recognized name but is not a v1 runtime target.
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
# Event names follow the scene-event taxonomy defined in §5.5 (<source>.<action>).
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
- The `tabs` array must contain at least one entry. Empty array → `CONFIG_NO_TABS` with hint: "add at least one `[[tabs]]` entry."
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
```

**Note:** `redaction_style` is NOT a chrome configuration field. It is a privacy policy field and belongs exclusively in `[privacy]` (see §7.1 and RFC 0009 §3.2). The `ChromeConfig` Rust struct must not contain a `redaction_style` field.

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
  Expected: one of: full-display, headless, or a custom profile name defined in [display_profile]
  Got: "wall-display"
  Hint: define a custom profile under [display_profile] or use a built-in name
  Note: the mobile profile is schema-reserved; use CONFIG_MOBILE_PROFILE_NOT_EXERCISED for that case

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

| Name | v1 Status | Purpose |
|------|-----------|---------|
| `full-display` | Active | High-end local display (wall display, monitor, kiosk). GPU, persistent power. |
| `headless` | Active | CI/test, no window. Offscreen render target. Not for production deployments. |
| `mobile` | Schema-reserved (post-v1) | Mobile Presence Node (phone, glasses-class). Designed into the schema; **not exercised in v1**. See §3.3. |

The `"desktop"` alias used in early doctrine drafts maps to `full-display`. The `"headless"` profile is the v1 mechanism for CI testing (see architecture.md §"Display profiles": "V1 supports at least two built-in profiles: 'desktop' (high-end local display) and 'headless' (CI/test, no window)"). The RFC uses `full-display` as the canonical production name; `headless` is the third built-in to support the doctrinal requirement.

The `mobile` profile is designed into the schema for forward compatibility (see v1.md §"Mobile": "Mobile capability negotiation is designed into the API but not exercised"; architecture.md §"Display profiles": "Mobile profiles are designed into the schema but not exercised until post-v1"). V1 is desktop/server only. Setting `profile = "mobile"` at runtime is not silently accepted — see §3.3 for the v1 enforcement rule.

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

> **V1 doctrinal position:** The `mobile` profile is **schema-reserved in v1**. The Rust struct exists and the TOML schema documents it for forward compatibility, but `profile = "mobile"` at runtime emits a structured warning `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` and the runtime refuses to start. V1 is desktop/server only (Linux, Windows, macOS). See v1.md §"Mobile" and architecture.md §"Display profiles". Mobile capability negotiation is designed into the API but not exercised until post-v1.

The Mobile Presence Node profile. Targets high-end phones and smart-glasses-class devices with variable network, thermal limits, and tighter display budgets. The schema is defined here to document the design intent and for use by custom profiles that `extends = "mobile"` in post-v1 deployments. **Implementers must not test against these budget values in v1** — the profile is not activated by the v1 runtime.

```toml
# Built-in profile definition (shown for documentation; post-v1 target)
# IMPORTANT: profile = "mobile" is not valid in v1. The runtime will emit
# CONFIG_MOBILE_PROFILE_NOT_EXERCISED and refuse to start.
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
max_media_streams = 1     # One primary live stream (post-v1)
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

**V1 runtime enforcement rule:** If `[runtime].profile = "mobile"` is set, the config validator emits a structured error (not a warning) `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` and refuses to start:

```
Error loading config: 1 validation error(s)

  [CONFIG_MOBILE_PROFILE_NOT_EXERCISED]
  Field: runtime.profile
  Expected: one of: full-display, headless, or a custom profile name
  Got: "mobile"
  Hint: the mobile profile is schema-reserved for post-v1. V1 is desktop/server only.
        Use profile = "full-display" for production or profile = "headless" for CI.
        To define a mobile-inspired custom profile, use [display_profile] extends = "mobile"
        with a distinct custom name — but note this custom profile will also not activate
        mobile-specific runtime paths that do not exist in v1.
```

This is a hard startup error, not a degraded run. Operators must not ship mobile configs against v1 builds.

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

1. **Explicit config**: if `[runtime].profile` names a profile, use it. Note that `profile = "mobile"` is a hard startup error in v1 (see §3.3; `CONFIG_MOBILE_PROFILE_NOT_EXERCISED`).
2. **Auto-detection** (if `profile = "auto"`): the runtime queries environment and hardware capabilities and selects the closest built-in profile. Detection runs in the following order — the first matching branch wins:

   **Step 1 — Headless environment check (evaluated first):**
   Select `headless` if any of the following signals are present:
   - On Linux/macOS: `$DISPLAY` is unset or empty **and** `$WAYLAND_DISPLAY` is unset or empty (no X11 or Wayland session).
   - On Windows: no Win32 display server is reachable (e.g., `GetSystemMetrics(SM_CXSCREEN)` returns 0 or the call fails).
   - Container hint: `/.dockerenv` exists (process is running inside a Docker container). This is an additional headless hint, not a sufficient condition on its own — a Docker container with a forwarded display is still treated as headless unless an explicit `profile =` is set.
   - Software-only rendering: the wgpu adapter reports no hardware GPU and falls back to a software renderer (Mesa llvmpipe, WARP, or equivalent).

   When this branch fires, log an `INFO` entry naming the detected signal (e.g., `"auto-detect: headless — $DISPLAY unset"`).

   **Step 2 — Full-display check:**
   Select `full-display` if a local GPU with > 4 GB VRAM is present **and** the primary display refresh rate is >= 60 Hz.

   **Step 3 — Detection failure:**
   If neither headless nor full-display conditions are met (e.g., display present but GPU VRAM below threshold, or hardware information is partially unavailable): log a `WARN` with the detection output and abort startup with a structured error. The operator must set an explicit `profile =` value.

   > **Note:** `mobile` is schema-reserved but **never selected by auto-detection in v1**. Auto-detection cannot reliably distinguish a mobile hardware environment from a degraded desktop environment without device-class signals (screen DPI, touch capability) that are unavailable at startup on all supported platforms. Any code path that would select `mobile` must instead select `headless` and log a clear warning.

   > **CI guidance:** CI pipelines should set `profile = "headless"` explicitly in their config rather than relying on auto-detection. Auto-detection will select `headless` correctly when no display server is present, but explicit configuration is more robust across CI environments (some runners expose a virtual display). See §3.4 for the headless profile definition.
3. **Profile extension**: if `[display_profile].extends` is set, the named base profile is loaded and then overridden field-by-field with any fields present in `[display_profile]`. The result is the effective profile; `[runtime].profile` names this effective profile. For a custom-named profile, `[runtime].profile` must be set to the custom name (e.g., `"glasses-v1"`) and `[display_profile].extends` must name the built-in base. If `[display_profile].extends` is set and `[runtime].profile` names a *different* built-in, the configuration is rejected with `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE` (see §2.3).
   - Note: `extends = "mobile"` is permitted for custom profiles (the `mobile` schema is available as a base), but the resulting custom profile does not activate any mobile-specific runtime paths that do not exist in v1. Operators must be aware that custom profiles derived from `mobile` are schema-compatible but behaviorally equivalent to `headless` in areas where mobile runtime paths are absent.

The selected profile name is logged at startup and included in the runtime's gRPC handshake response so agents can inspect it.

### 3.6 Custom Profiles

A deployment can define a custom profile for specific hardware (e.g., a glasses device with unusual limits):

```toml
[runtime]
profile = "glasses-v1"

[display_profile]
extends = "mobile"   # post-v1: mobile schema is used as the base; see note below
max_tiles = 8
max_texture_mb = 64
max_agents = 2
target_fps = 30
min_fps = 15
max_media_streams = 0       # No media in v1 glasses profile
allow_chrome_zones = false  # Minimal chrome on glasses
```

**Note on `extends = "mobile"` in v1:** This custom profile example is shown for forward-compatibility documentation. In v1, a custom profile with `extends = "mobile"` is syntactically valid (the `mobile` schema is available as a base), but the resulting profile does not activate any mobile-specific runtime paths that are absent in v1. Operators targeting a constrained v1 deployment with mobile-like budgets should instead extend `full-display` and set conservative budget values, or use `profile = "headless"` for display-less environments. The `extends = "mobile"` form is intended for post-v1 use when the mobile runtime paths are implemented.

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

> **V1 note on mobile parameters:** The policy definitions below include per-profile default values for the `mobile` profile. These are documented to preserve design intent and support post-v1 implementation. In v1, the `mobile` profile is schema-reserved and not exercised (see §3.3). V1 implementers must not write code that tests geometry against these mobile values — they are reference documentation for post-v1.

#### `bottom_strip`

A horizontal strip anchored to the bottom edge of the content area (below the status bar, above the OS taskbar if in overlay mode).

Default parameters:
- Height: `5%` of display height on `full-display`, `10%` on `mobile` *(post-v1)*
- Width: `100%` of display width minus horizontal margins
- Horizontal margins: `2%` on `full-display`, `1%` on `mobile` *(post-v1)*
- Vertical offset from bottom: `2%` on `full-display`, `3%` on `mobile` *(post-v1)*
- Background opacity: `0.75` (semi-transparent backdrop)
- Text scale: `1.0` on `full-display`, `1.4` on `mobile` *(post-v1, larger for glanceability)*

Typical uses: subtitle zone, transcript strip.

#### `top_right_stack`

A vertically stacking list of cards anchored to the top-right corner. New cards push downward.

Default parameters:
- Card width: `20%` of display width on `full-display`, `80%` on `mobile` *(post-v1, full-width banner)*
- Card max height: `8%` of display height per card
- Max visible cards: `5` on `full-display`, `2` on `mobile` *(post-v1)*
- Anchor: top-right corner, `2%` inset on each axis
- Stack direction: downward
- Auto-dismiss: card collapses after `timeout_secs` (configured per zone instance)

Typical uses: notification zone.

#### `full_width_bar`

A thin horizontal bar spanning the full display width. Attaches to the chrome layer.

Default parameters:
- Height: `3%` of display height on `full-display`, `4%` on `mobile` *(post-v1)*
- Position: top or bottom (determined by `chrome.tab_bar_position`; status bar takes the opposite edge)
- Content: horizontally scrolling key-value pairs
- Always visible: yes (chrome layer)

Typical uses: status-bar zone, tab bar.

#### `corner_anchored`

A floating surface anchored to a named corner of the content area, draggable within bounds.

Default parameters:
- Default anchor: `bottom_right`
- Default size: `20%` × `15%` of display on `full-display`; `30%` × `25%` on `mobile` *(post-v1)*
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
- Height: `8%` of display height on `full-display`, `12%` on `mobile` *(post-v1)*
- Expansion: animated (smooth push) on `full-display`, instant on `mobile` *(post-v1)*
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
# NOTE: profile_overrides.mobile is post-v1. The mobile profile is schema-reserved
# and not exercised in v1 (see §3.3). This override block is valid TOML and will be
# parsed without error, but it will never activate in v1 builds.
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

**Validation rules:**
- Each `reserved_*_fraction` must be in the range [0.0, 1.0]. Values outside this range → `CONFIG_INVALID_RESERVED_FRACTION` identifying the field and the out-of-range value.
- `reserved_top_fraction + reserved_bottom_fraction` must be strictly less than 1.0. A sum of exactly 1.0 or greater leaves zero vertical space for agent tiles. Violation → `CONFIG_INVALID_RESERVED_FRACTION` with hint: "vertical reserved fractions sum to <value>, leaving no room for agent tiles; reduce `reserved_top_fraction` or `reserved_bottom_fraction`."
- `reserved_left_fraction + reserved_right_fraction` must be strictly less than 1.0. Same rationale. Violation → `CONFIG_INVALID_RESERVED_FRACTION` with equivalent hint.

### 5.4 Tab Switching Policy

The `tab_switch_on_event` field names a scene-level event (see §5.5) that automatically activates the tab. This is the mechanism for interrupt-driven tab switching without agent involvement:

```toml
tab_switch_on_event = "doorbell.ring"    # Switch when doorbell rings
tab_switch_on_event = "alert.fire"       # Switch on fire alert
tab_switch_on_event = ""                 # No automatic switch (default)
```

Tab switches triggered by `tab_switch_on_event` are subject to the canonical policy evaluation order defined in architecture.md §"Policy arbitration" (steps 1–7: human override → capability gate → privacy/viewer gate → interruption policy → attention budget → zone contention → resource/degradation budget). The interruption class check (step 4) is what determines whether the tab switch fires during quiet hours: a `doorbell.ring` event carries `interruption_class = "urgent"` and therefore passes through quiet-hours gating. A `morning.routine` event with `interruption_class = "normal"` would be suppressed during quiet hours.

**Unknown event names:** If `tab_switch_on_event` names an event not found in the v1 scene-event taxonomy (§5.5), the runtime accepts the configuration without error and emits a `WARN` log entry at startup: `tab_switch_on_event '<name>' in tab '<tab>': event name not recognized; this tab will never auto-switch until this event is registered`. This is not a hard validation error — custom events may be registered at runtime in post-v1 deployments, and treating an unrecognized event name as an error would block forward compatibility. An empty string (`""`) is a valid value meaning "no automatic switch" and does not generate a warning.

### 5.5 Scene-Event Taxonomy

> **Doctrine:** presence.md §"Inter-agent events" establishes that "agents can subscribe to a shared event bus for coarse-grained coordination signals." These are scene-level events, not input events (pointer, touch, keyboard, gesture) — those belong to RFC 0004. The v1 scope constraint comes from v1.md §"Advanced presence": "No inter-agent event bus beyond basic scene topology changes."

#### Event Naming Convention

Scene events use a dotted hierarchy: `<source>.<action>`. Both segments use lowercase letters, digits, and underscores. No whitespace, no uppercase.

Examples: `doorbell.ring`, `alert.fire`, `system.agent_connected`.

The source segment identifies who or what generated the event. The action segment identifies what happened. A two-segment name is required; deeper nesting (e.g., `sensor.door.open`) is reserved for post-v1.

#### V1 Event Categories

Three categories of scene events exist in v1:

**1. System events** (runtime-emitted, `system.*` prefix, reserved for runtime use):

| Event name | Trigger | Interruption class |
|---|---|---|
| `system.agent_connected` | An agent completes `SessionEstablished` | `normal` |
| `system.agent_disconnected` | An agent session closes or expires | `normal` |
| `system.degradation_entered` | Runtime enters a degradation level | `normal` |
| `system.degradation_exited` | Runtime exits a degradation level | `normal` |

The `system.*` prefix is reserved. Agents may not emit events with names starting with `system.` — such capability grants are rejected at load time (see validation rules below).

**2. Scene topology events** (runtime-emitted, delivered via `subscribe_scene_events` capability):

These are coarse-grained state changes that agents can subscribe to (RFC 0005 §7.1 `scene_topology` category):

| Event name | Trigger |
|---|---|
| `scene.tab_switched` | The active tab changed |
| `scene.tile_created` | A tile was added to the scene |
| `scene.tile_destroyed` | A tile was removed from the scene |

Scene topology events are emitted by the runtime and are not agent-emittable.

**3. Agent-emittable named events**:

Agents can fire arbitrary named events by holding the `emit_scene_event:<name>` capability (§6.3). These events are named by the agent and correspond to domain-specific triggers: `doorbell.ring`, `alert.fire`, `morning.routine`, etc. An agent cannot emit an event it does not hold a specific capability for.

Agent-emittable event names must conform to the naming convention above (`<source>.<action>`) and must not use the `system.` or `scene.` prefix.

#### Event Payload (V1)

All v1 scene events carry a minimal fixed payload:

| Field | Type | Description |
|---|---|---|
| `event_name` | string | Canonical dotted event name (e.g., `doorbell.ring`) |
| `emitted_at_us` | int64 | UTC microseconds when the event was fired (RFC 0003 clock domain) |
| `source_agent_id` | SceneId or null | Agent that emitted the event; null for runtime-emitted events |

Post-v1 event payloads may carry additional structured fields. V1 recipients must ignore unknown fields.

#### Validation Rules for `tab_switch_on_event`

- The field value must be either the empty string (`""`) or a valid dotted event name matching `^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$`. Values that fail this pattern → `CONFIG_INVALID_EVENT_NAME` with hint: "event name must follow the `<source>.<action>` dotted hierarchy".

#### Validation Rules for `emit_scene_event:<name>` Capabilities (§6.3)

- The `<name>` segment in `emit_scene_event:<name>` must be a valid dotted event name.
- Capability grants with names starting `emit_scene_event:system.` are rejected at config load: `CONFIG_RESERVED_EVENT_PREFIX` with hint: "the `system.` prefix is reserved for runtime-emitted events; agents may not emit events with this prefix".
- Capability grants with names starting `emit_scene_event:scene.` are also rejected: `CONFIG_RESERVED_EVENT_PREFIX` with equivalent hint.

These validation rules are added to the `ConfigErrorCode` enum (§10) and the Summary of Validation Error Codes table.

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
  "zone_publish:subtitle",
  "zone_publish:status_bar",
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
  "zone_publish:notification",
  "zone_publish:alert_banner",
  "emit_scene_event:doorbell.ring",  # Can fire the doorbell tab-switch event
]

[agents.registered.doorbell_agent.budgets]
max_tiles = 2
max_texture_mb = 128  # v1 snapshot budget; camera feed (post-v1) will require a higher ceiling
max_update_hz = 60
```

**Validation rules for agent budgets:**
- Per-agent `max_tiles` may not exceed the active profile's `max_tiles` ceiling. Violation → `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE` identifying the agent name, the field, the configured value, and the profile ceiling.
- Per-agent `max_texture_mb` may not exceed the active profile's `max_texture_mb` ceiling. Same error code.
- Per-agent `max_update_hz` may not exceed the active profile's `max_agent_update_hz` ceiling. Same error code.
- These rules apply to both `[agents.registered.<name>.budgets]` and `[agents.dynamic_policy.default_budgets]`.

Note: per-agent budgets are sub-allocations of the profile total. An agent claiming more than the entire profile budget is always misconfigured; the runtime's admission control (RFC 0002 §4.3) would reject it at session time anyway — catching it at config load is strictly better for operator feedback.

### 6.3 Capability Identifiers

The capability grant list in each agent entry uses structured capability identifiers. **These names are the wire-format canonical names as defined by RFC 0005 §7.1 and RFC 0008 §2.1.** RFC 0001 diagrams use an older uppercase form (`CREATE_TILE`, `WRITE_SCENE`) and a colon-delimited zone format (`zone:publish:<zone>`) for illustration purposes; the canonical runtime strings are the lowercase underscore forms in the table below.

| Identifier | Description | Authoritative RFC |
|------------|-------------|------------------|
| `create_tiles` | May request tile leases. | RFC 0005, RFC 0008 §3.3 |
| `modify_own_tiles` | May mutate content on own tiles. | RFC 0005 |
| `read_scene` | May query the full scene topology, including other agents' lease metadata. | RFC 0005 §7.1 |
| `subscribe_scene_events` | May subscribe to scene-level events on the scene-event bus (system events, scene topology events, agent-emittable named events — see §5.5). This is **not** an input-event subscription; input routing uses `receive_input`. | RFC 0005 §7.1 |
| `overlay_privileges` | May request tiles with high z-order or overlay-level positions. | RFC 0005 |
| `receive_input` | May receive input events forwarded from the runtime (pointer, touch, keyboard, gesture per RFC 0004). This is **not** the scene-event bus; use `subscribe_scene_events` for named scene events. | RFC 0005 §7.1 |
| `high_priority_z_order` | May request z-order values in the top quartile. | RFC 0005 |
| `exceed_default_budgets` | May request budget overrides at session time (requires user prompt). | RFC 0005 |
| `read_telemetry` | May subscribe to `telemetry_frames` events (runtime performance samples). | RFC 0005 §7.1 |
| `zone_publish:<zone_name>` | May publish to the named zone. One grant per zone. `zone_publish:*` grants all zones. | RFC 0005 §7.1 |
| `emit_scene_event:<event_name>` | May fire the named scene event on the scene-event bus (e.g., `emit_scene_event:doorbell.ring`). The `<event_name>` must follow the `<source>.<action>` naming convention and must not use the `system.` or `scene.` reserved prefix (§5.5). | RFC 0005 |
| `lease:priority:<N>` | May request lease priority N or lower (0=Critical, 4=Speculative). See RFC 0008 §2.1. | RFC 0008 §2.1 |

Capabilities not listed are not granted. Tile and topology access is controlled by the individual capabilities `create_tiles`, `modify_own_tiles`, and `read_scene` — these must be listed explicitly. Only `zone_publish:*` is supported as a wildcard form.

> **Scene-event bus vs. input routing:** `subscribe_scene_events` / `emit_scene_event` operate on the scene-event bus defined in §5.5 (presence.md §"Inter-agent events" doctrine). They are distinct from input events (RFC 0004): `doorbell.ring` is a device/domain event, not a pointer or keyboard event. Granting an agent `receive_input` does not give it access to scene events, and granting `subscribe_scene_events` does not forward any input.

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
  "zone_publish:notification",
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

# Viewer identification method (v1 form — single string).
# One of: "none", "face_recognition", "proximity_badge", "phone_presence", "explicit_login".
# Pluggable; the runtime defines a trait for viewer identity (see privacy.md §"Viewer context").
# Default: "none" (viewer class always equals default_viewer_class).
#
# POST-V1 DESIGN DIRECTION — Detector Pipeline:
# viewer_id_method will become a pipeline of ordered detector configurations.
# The single-string form above remains valid as a convenience shorthand (pipeline of length 1).
# See §7.1 "Viewer Identification Pipeline (Post-V1 Design Note)" for the full syntax.
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

#### Viewer Identification Pipeline (Post-V1 Design Note)

> **This section is a design note only.** The v1 implementation uses the single `viewer_id_method` string above. The pipeline syntax below is forward-looking doctrine to prevent locking the config schema into a single-detector architecture. See Open Question 6 and Round 7 review history.

In v1, `viewer_id_method` accepts a single method name (or `"none"`). The runtime instantiates one detector and uses its output directly.

**Post-v1 direction:** `viewer_id_method` will be superseded by `[[privacy.viewer_detectors]]`, an ordered array of detector configurations. The single-string form is retained as a convenience shorthand that expands to a one-entry pipeline with default threshold settings — ensuring backward compatibility for simple deployments.

**Post-v1 pipeline syntax:**

```toml
# Explicit login always wins (priority 0 = highest).
[[privacy.viewer_detectors]]
method = "explicit_login"
priority = 0

# Face recognition as primary biometric detector.
[[privacy.viewer_detectors]]
method = "face_recognition"
priority = 1
confidence_threshold = 0.85  # 0.0–1.0; default: 0.75

# Phone proximity as fallback when face recognition is unavailable or inconclusive.
[[privacy.viewer_detectors]]
method = "phone_presence"
priority = 2
```

**Pipeline semantics:**

1. Each detector is assigned an integer `priority` (lower number = higher priority); `priority` defines ranking, not evaluation order.
2. If one or more detectors return an identification result with confidence ≥ `confidence_threshold`, the result from the highest‑priority detector (lowest `priority` value) wins; its viewer class assignment is used.
3. If no detector produces a confident identification, `default_viewer_class` applies.
4. The `"explicit_login"` method always produces confidence 1.0 (binary: logged in or not) and should be given the highest priority (lowest number) when present.
5. Multiple detectors may run concurrently; priority governs which successful result is accepted, not evaluation order or wall‑clock completion time.

**Compatibility rules:**

- `viewer_id_method = "face_recognition"` (v1 string form) is equivalent to:
  ```toml
  [[privacy.viewer_detectors]]
  method = "face_recognition"
  priority = 0
  confidence_threshold = 0.75
  ```
- A config that supplies both `viewer_id_method` and `[[privacy.viewer_detectors]]` is a post-v1 validation error: `CONFIG_VIEWER_ID_AMBIGUOUS`.
- The `"none"` method in the pipeline is only valid as a sole entry (pipeline of length 1, equivalent to `viewer_id_method = "none"`). Including `"none"` alongside other detectors is a validation error: `CONFIG_VIEWER_ID_NONE_NOT_MIXABLE`.

**Post-v1 validation rules (not enforced in v1):**

- Each entry's `method` must be one of the recognized detector types. Unknown → `CONFIG_UNKNOWN_VIEWER_METHOD`.
- `priority` values must be unique across all entries in the pipeline. Duplicate priorities → `CONFIG_VIEWER_ID_DUPLICATE_PRIORITY`.
- `confidence_threshold`, if set, must be in `[0.0, 1.0]`. Out of range → `CONFIG_INVALID_CONFIDENCE_THRESHOLD`.
- If neither `viewer_id_method` nor any `[[privacy.viewer_detectors]]` entries are present, the runtime normalizes this to `viewer_id_method = "none"` and emits a WARN log.

**Doctrinal alignment:** privacy.md §"Viewer context" states: "The runtime does not require a specific identification mechanism — it defines a trait for viewer identity and lets the deployment plug in what's appropriate." The pipeline model is the direct config-level expression of this trait-based architecture: each `[[privacy.viewer_detectors]]` entry is a registered implementation of the `ViewerIdentitySource` trait, evaluated in deployment-defined priority order.

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
    /// in this section override it. May name `"full-display"` or `"mobile"`.
    /// The `"headless"` profile is a recognized built-in but may not be used as an
    /// `extends` base — see `CONFIG_HEADLESS_NOT_EXTENDABLE`. Extending another
    /// custom profile is also not supported.
    ///
    /// Note: `extends = "mobile"` is valid syntax in v1 but the resulting custom
    /// profile does not activate any mobile-specific runtime paths absent in v1.
    /// The mobile schema is available as a budget reference for post-v1 deployments.
    /// See RFC 0006 §3.6 and v1.md §"Mobile".
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
    /// The `mobile` profile is schema-reserved and not exercised in v1.
    /// Setting `profile = "mobile"` in `[runtime]` is a hard startup error.
    /// See RFC 0006 §3.3 and v1.md §"Mobile".
    ConfigMobileProfileNotExercised,
    ConfigInvalidAddress,
    ConfigNoTabs,
    ConfigDuplicateTabName,
    ConfigMultipleDefaultTabs,
    ConfigUnknownLayout,
    ConfigUnknownZoneType,
    ConfigUnknownGeometryPolicy,
    ConfigIncompatibleZoneLayer,
    ConfigInvalidReservedFraction,
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
    ConfigAgentBudgetExceedsProfile,
    ConfigInvalidEventName,
    ConfigReservedEventPrefix,
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

### Viewer Identification Pipeline Types (Post-V1 Design Note)

> **These types are a design note only.** In v1, `PrivacyConfig` holds a single `viewer_id_method: ViewerIdMethod` (enum) field. The types below document the intended post-v1 *pipeline* representation, where `viewer_id_method` becomes a structured list of detectors. See §7.1 "Viewer Identification Pipeline (Post-V1 Design Note)" for the config syntax.

```rust
/// A single viewer identification detector in the pipeline.
/// Each entry maps to one implementation of the `ViewerIdentitySource` trait.
///
/// Post-v1. In v1, `PrivacyConfig.viewer_id_method` is a single `ViewerIdMethod` enum value, not a pipeline.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ViewerDetectorConfig {
    /// Detector type. One of: "none", "face_recognition", "proximity_badge",
    /// "phone_presence", "explicit_login".
    pub method: ViewerIdMethod,

    /// Evaluation priority. Lower number = higher priority. Must be unique within
    /// the pipeline. The first detector returning confidence >= `confidence_threshold`
    /// wins; lower-priority detectors are not consulted.
    pub priority: u32,

    /// Minimum confidence score [0.0, 1.0] for this detector's result to be accepted.
    /// Default: 0.75. The "explicit_login" method always returns confidence 1.0
    /// (binary outcome) and does not require this field.
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f32,
}

/// The recognized viewer identification methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewerIdMethod {
    /// No identification; viewer class is always `default_viewer_class`.
    None,
    /// Camera-based face recognition. Requires hardware camera and face recognition
    /// plugin. Confidence is the recognition model's score for the top match.
    FaceRecognition,
    /// Bluetooth/NFC proximity badge detection. Confidence is binary (detected or not).
    ProximityBadge,
    /// Phone presence detection (Bluetooth LE, Wi-Fi association, or similar).
    /// Confidence is binary (device detected or not).
    PhonePresence,
    /// Explicit PIN, password, or biometric login via the runtime's login screen.
    /// Always returns confidence 1.0. Recommended for highest-priority slot.
    ExplicitLogin,
}

/// This type is intended to be used as the value type of a single configuration
/// field (e.g., `viewer_id_method`) whose syntax evolves over time. In v1, that
/// field contains a single `ViewerIdMethod` enum value. Post-v1, it may instead
/// contain a full detector pipeline (`Vec<ViewerDetectorConfig>`).
///
/// This untagged union allows the post-v1 parser to accept both forms and
/// normalize them internally.
///
/// The single-method form is normalized to a one-entry pipeline on load:
/// ```
/// viewer_id_method = "face_recognition"
/// →
/// ViewerIdConfig::Pipeline(vec![ViewerDetectorConfig {
///     method: ViewerIdMethod::FaceRecognition,
///     priority: 0,
///     confidence_threshold: 0.75,
/// }])
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ViewerIdConfig {
    /// V1 form: single method string. Shorthand for a one-entry pipeline.
    Single(ViewerIdMethod),
    /// Post-v1 form: ordered pipeline of detector configurations.
    Pipeline(Vec<ViewerDetectorConfig>),
}

fn default_confidence_threshold() -> f32 { 0.75 }
```

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
- `mobile-reserved.toml` — asserts that `profile = "mobile"` produces `CONFIG_MOBILE_PROFILE_NOT_EXERCISED`; does not test mobile budget values (which are post-v1)
- `invalid/*.toml` — one fixture per error code, asserting the exact structured error
- `reload/` — fixtures for testing hot-reload behavior

---

## Open Questions

1. **Custom zone type discovery.** Should the runtime enumerate custom zone types in the `list_zones` gRPC/MCP response? Currently yes (all zones, built-in and custom, are listed). This exposes deployment-specific configuration to agents, which is intentional — agents need to discover what zones are available.

2. **Profile auto-detection heuristics.** The auto-detection criteria in §3.5 (GPU VRAM threshold, display refresh rate for `full-display`; environment variable and `/.dockerenv` signals for `headless`) are v1-only heuristics. They will need revisiting when the mobile runtime is implemented in post-v1 (the current logic must not silently select `mobile`). The headless signals are conservative and enumerated (not exhaustive); future container runtimes or novel CI environments may require additional probes. Deployments should prefer explicit `profile =` settings whenever the target environment is known at deploy time.

3. **Tab order persistence.** Tab order is defined by the order of `[[tabs]]` entries in the config. If the user reorders tabs via the UI (a future feature), should that reordering persist across restarts? For v1, tab order is config-only. Persistence is deferred.

4. **Secret management for PSK keys.** The current design reads keys from environment variables. Deployments with more complex secret management needs (Vault, key files, etc.) will need a post-v1 extension. The auth trait is designed to support this.

5. **Scene events as a dedicated RFC.** §5.5 defines the v1 scene-event taxonomy inline because v1.md constrains scene events to basic topology changes plus agent-emittable named events. As post-v1 deployments add richer event routing (event filters, replay, subscriptions by prefix, custom payload schemas), the scene-event contract may outgrow a config section. Consider a dedicated RFC 0010 (Scene Event Bus) that owns the namespace registry, event payload schema, delivery guarantees, and subscription model. RFC 0006 would then cross-reference it for `tab_switch_on_event` semantics, as RFC 0004 owns input routing today.

6. **Viewer identification pipeline promotion.** §7.1 documents the `[[privacy.viewer_detectors]]` pipeline syntax as a post-v1 design note. Two open questions remain for the promotion milestone:
   - **When?** The pipeline syntax should be promoted to a first-class schema field as soon as any deployment needs more than one detector. It should not wait for a major version bump. The proposed trigger: when a second detector type ships a plugin implementation, the pipeline config is promoted from "design note" to "supported" in the same release.
   - **Plugin contract.** The `ViewerIdentitySource` trait needs a dedicated RFC (or an appendix to an existing privacy/security RFC) that specifies the plugin interface: confidence score semantics, async evaluation contract, error handling when a detector is unavailable, and how the runtime surfaces detector health in telemetry. Without a stable plugin contract, third-party detector implementations cannot be guaranteed interoperable with the pipeline runtime. Consider whether this belongs in a dedicated RFC 0011 (Viewer Identity Plugin Contract) or as a §7 extension in this RFC.

---

## Summary of Validation Error Codes

| Code | Section | Trigger |
|------|---------|---------|
| `CONFIG_UNKNOWN_PROFILE` | §2.2 | `runtime.profile` names an unknown profile |
| `CONFIG_MOBILE_PROFILE_NOT_EXERCISED` | §2.2, §3.3 | `runtime.profile = "mobile"` — mobile is schema-reserved and not a v1 runtime target |
| `CONFIG_INVALID_ADDRESS` | §2.2 | `grpc_bind` or `mcp_bind` is not a valid socket address |
| `CONFIG_NO_TABS` | §2.4 | `tabs` array is empty (at least one `[[tabs]]` entry required) |
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
| `CONFIG_INVALID_RESERVED_FRACTION` | §5.3 | A `reserved_*_fraction` is outside [0.0, 1.0] or the sum of vertical/horizontal reserved fractions ≥ 1.0 |
| `CONFIG_UNKNOWN_CLASSIFICATION` | §7.1 | Unknown `default_classification` value |
| `CONFIG_UNKNOWN_VIEWER_CLASS` | §7.1 | Unknown `default_viewer_class` value |
| `CONFIG_INVALID_TIME` | §7.1 | Quiet hours `start` or `end` is not valid `HH:MM` |
| `CONFIG_UNKNOWN_INTERRUPTION_CLASS` | §7.1 | Unknown `pass_through_class` value |
| `CONFIG_INVALID_THRESHOLD` | §7.2 | Degradation threshold is zero or negative |
| `CONFIG_UNKNOWN_SHED_PRIORITY` | §7.2 | Unknown `shed_priority` value |
| `CONFIG_DEGRADATION_THRESHOLD_ORDER` | §7.2 | Degradation thresholds are not monotonically non-decreasing (heavier step fires before lighter step) |
| `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE` | §6.2 | Per-agent budget override (`max_tiles`, `max_texture_mb`, `max_update_hz`) exceeds the active profile's ceiling |
| `CONFIG_INVALID_EVENT_NAME` | §5.4, §5.5 | `tab_switch_on_event` value is not empty and does not match `^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$` |
| `CONFIG_RESERVED_EVENT_PREFIX` | §5.5, §6.3 | `emit_scene_event:<name>` capability grant uses the `system.` or `scene.` reserved prefix |
