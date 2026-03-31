# RFC 0006: Configuration/Display Profiles — Round 2 Review

**Review Round:** 2 of 4
**Focus:** Technical Architecture Scrutiny
**Issue:** rig-5vq.32
**Date:** 2026-03-22
**Reviewer:** Beads worker agent
**Doctrine files reviewed:** architecture.md, presence.md, security.md, privacy.md, failure.md, mobile.md

---

## Doctrinal Alignment: 4/5

Round 1 fixed all doctrinal blockers (headless profile, field-name alignment with RFC 0002, policy evaluation order citation, viewer class enumeration). The RFC now faithfully implements:

- "Declarative, file-based, human-readable, validated at load time" (architecture.md §Configuration model)
- Two built-in profiles + headless (architecture.md §Display profiles)
- Quiet-hours and interruption classes (privacy.md)
- Zone geometry adapts to display profile (presence.md §Zone geometry adapts to the display profile)
- Degradation ladder ordered by impact (failure.md §Degradation axes)
- Agent capability grants, additive model, revocable (security.md §Capability scopes)
- "Fail closed" privacy default (privacy.md, RFC default_classification = "private")

No doctrinal regressions found. Score unchanged from round 1.

---

## Technical Robustness: 3/5

Five technical defects found. Three are MUST-FIX (type safety, silent misconfiguration, and incomplete validation). Two are SHOULD-FIX (missing range validation, schema-emit ambiguity).

---

## Cross-RFC Consistency: 4/5

One SHOULD-FIX and two CONSIDERs on cross-RFC boundary behavior.

---

## Actionable Findings

### [MUST-FIX 1] `ConfigError.code` is `&'static str`, not a typed enum

**Location:** §10 Rust types, `ConfigError` struct; §Summary of Validation Error Codes table.

**Problem:** The `code` field is declared as `&'static str`. This means:
- The compiler cannot enforce exhaustive matching over error codes at call sites.
- The Summary of Validation Error Codes table and the Rust type are two separate sources of truth that can drift silently.
- Tooling (schema export, JSON consumers) cannot enumerate valid codes without parsing the documentation.

**Required fix:** Define a `ConfigErrorCode` enum with one variant per code in the summary table. Change `ConfigError.code` to `ConfigErrorCode`. The error code table becomes derivable from `#[derive(Debug, Serialize, JsonSchema)]` on the enum, making the table a generated artifact rather than a manual one.

**Impact:** Non-trivial — requires updating both the type definition and the summary table to reference the enum. All construction sites of `ConfigError` must use `ConfigErrorCode::X` instead of a string literal.

---

### [MUST-FIX 2] `[display_profile].extends` and `[runtime].profile` are silently incoherent

**Location:** §2.3 `[display_profile]` section; §3.5 Profile Negotiation.

**Problem:** An operator can write:

```toml
[runtime]
profile = "full-display"    # selects built-in full-display profile

[display_profile]
extends = "mobile"          # overrides to extend mobile instead
max_tiles = 8
```

Per §3.5 step 3, `[display_profile].extends` triggers extension merge: "the named profile is loaded and then overridden with any fields present in `[display_profile]`." But step 1 says `[runtime].profile` selects the profile. When both are set and they name different built-ins, the RFC is ambiguous: does `extends` override `profile`, does `profile` win, or is the combination an error?

The current validation rules for `[runtime]` and `[display_profile]` do not cross-validate this pair. An operator who intends to extend `mobile` but accidentally leaves `profile = "full-display"` will silently get either the wrong profile or unpredictable merge behavior.

**Required fix:** Add a validation rule and a new error code:

- If `[display_profile].extends` is set AND `[runtime].profile` names a built-in AND they differ → `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE` with a hint explaining which wins and how to fix.
- Clarify in §3.5 the resolution order: when `extends` is set, the effective profile name (for gRPC handshake, logging, and budget enforcement) is the custom profile defined in `[display_profile]`, not `[runtime].profile`. The `profile` field then names the custom profile, and `extends` names the base.

Add `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE` to the error code summary table.

---

### [MUST-FIX 3] Boolean capability escalation not blocked by profile budget escalation rule

**Location:** §3.6 Custom Profiles, validation rules.

**Problem:** The escalation validation rule states: "Numeric overrides must not exceed the base profile's values (prevent escalation)." This correctly blocks increasing `max_tiles`, `max_texture_mb`, etc. However, it does not address boolean capability escalation:

- The `mobile` built-in has `allow_background_zones = false`.
- A custom profile extending `mobile` could set `allow_background_zones = true`, re-enabling a capability the mobile profile explicitly disabled.
- This is capability escalation, not budget escalation — but it is still escalation against the base profile's security boundary.

For `allow_background_zones` and `allow_chrome_zones`, `false` in the base is a security/capability ceiling, not a default preference. A deployment targeting glasses-class hardware that extends `mobile` with `allow_background_zones = true` gets a capability the base profile was designed to prevent.

**Required fix:** Extend the validation rule to cover boolean capability fields:

- If a custom profile sets `allow_background_zones = true` and the base profile has `allow_background_zones = false` → `CONFIG_PROFILE_CAPABILITY_ESCALATION` (a new, distinct code from `CONFIG_PROFILE_BUDGET_ESCALATION`).
- Same for `allow_chrome_zones`.
- Note: `prefer_zones` and `upstream_precomposition` are advisory hints, not capability gates — those boolean fields may be freely overridden.

Add `CONFIG_PROFILE_CAPABILITY_ESCALATION` to the error code summary table.

---

### [SHOULD-FIX 1] Degradation threshold ordering is not validated

**Location:** §7.2 Degradation Configuration, `[degradation.thresholds]`.

**Problem:** The degradation ladder (failure.md §Degradation axes) is "ordered by impact, lightest first." The RFC correctly assigns frame-time thresholds in ascending order in the example (`coalesce_frame_ms = 12.0`, `simplify_rendering_frame_ms = 14.0`, `shed_tiles_frame_ms = 16.0`, `audio_only_frame_ms = 20.0`). However, no validation rule enforces this ordering. An operator can configure:

```toml
[degradation.thresholds]
coalesce_frame_ms = 20.0         # step 1 threshold higher than shed_tiles
shed_tiles_frame_ms = 12.0       # step 5 triggers before step 1
```

This inverts the ladder: shed_tiles would fire before coalesce, causing aggressive content loss before the lighter-touch response is tried. No error is raised at load time.

**Required fix:** Add a validation rule after all thresholds are parsed:

- `coalesce_frame_ms` ≤ `simplify_rendering_frame_ms` ≤ `shed_tiles_frame_ms` ≤ `audio_only_frame_ms`. Violation → `CONFIG_DEGRADATION_THRESHOLD_ORDER`.
- Same for GPU fraction thresholds: `reduce_media_quality_gpu_fraction` ≤ `reduce_concurrent_streams_gpu_fraction`.

Add `CONFIG_DEGRADATION_THRESHOLD_ORDER` to the error code summary table.

---

### [SHOULD-FIX 2] `emit_schema` in `[runtime]` vs. `--print-schema` CLI flag: behavior on conflict undefined

**Location:** §2.2 `[runtime]` section (`emit_schema` field); §8 Schema Export.

**Problem:** §2.2 says `emit_schema = true` causes the runtime to "export a JSON schema to stdout at startup." §8 says `tze_hud --print-schema > schema.json` is the recommended usage. These are two separate mechanisms for the same output. The RFC does not specify:

1. Whether `--print-schema` implies an early exit (prints schema and exits) or whether the runtime continues starting up.
2. Whether `emit_schema = true` implies the runtime continues starting up after printing (allowing normal operation alongside schema output) or exits immediately.
3. What happens when both `emit_schema = true` and `--print-schema` are used simultaneously.

For tooling, `--print-schema` without early-exit means a caller cannot rely on `tze_hud --print-schema > schema.json` being a safe, non-destructive operation (it might start binding ports, creating GPU contexts, etc.).

**Required fix:** Clarify in §2.2 and §8:

- `--print-schema`: prints the JSON Schema to stdout and **exits immediately** (does not start the runtime). Safe for tooling and CI.
- `emit_schema = true`: the runtime writes the JSON Schema to stdout **at startup, then continues running** (useful for logging). Not a substitute for `--print-schema` in tooling pipelines.
- When both are used: `--print-schema` takes precedence and exits immediately.

---

### [CONSIDER 1] `tab_switch_on_event` names an event string with no validation against event registry

**Location:** §5.4 Tab Switching Policy.

**Problem:** `tab_switch_on_event = "doorbell.ring"` is a freeform string referencing a scene-level event from RFC 0004. There is no validation at config load time that this event name exists or is reachable. Operators who make a typo (`"doorbell.rings"`) get a tab that silently never auto-switches — no error, no warning.

**Recommendation:** Add a note that unknown event names are silently accepted at load time (since the event registry may not be fully known at config-load time, e.g., dynamic events) but the runtime emits a `WARN` log entry at startup for `tab_switch_on_event` names that are not in the built-in event registry. This is not a validation error — it's a diagnostic aid.

---

### [CONSIDER 2] `profile = "auto"` failure path is unspecified

**Location:** §3.5 Profile Negotiation, step 2 (auto-detection).

**Problem:** §3.5 defines auto-detection heuristics (GPU VRAM > 4GB and refresh ≥ 60Hz → `full-display`, else `mobile`). It does not specify what happens if detection fails: for example, if the GPU query itself throws an error, if VRAM cannot be read (virtualized environment, missing driver), or if neither heuristic condition matches exactly.

**Recommendation:** Add a sentence: if auto-detection encounters an error or an ambiguous result, the runtime falls back to `mobile` (most conservative) and logs a warning with the detection output. A future extension could use `profile = "auto:full-display"` syntax to specify a fallback.

---

### [CONSIDER 3] Quiet hours timezone handling is implicit

**Location:** §7.1 Privacy Configuration, quiet hours.

**Problem:** The RFC specifies `start`/`end` as `HH:MM` in "24-hour local wall clock." In practice, "local" is ambiguous for:
- A headless display node running in a server environment where `TZ` is set to UTC.
- Deployments across DST transitions (22:00 local in summer vs. winter may differ by one hour of real-time behavior).

**Recommendation:** Add a note clarifying that quiet hours use the system's local timezone (`TZ` environment variable or OS timezone setting) and that DST transitions are handled by the OS clock — the schedule is evaluated against local wall clock, not UTC. Operators in UTC-configured environments should set `TZ` appropriately. This is not a blocker, but the current silence is likely to cause support confusion.

---

## Summary of Changes Applied (MUST-FIX)

All three MUST-FIX findings are addressed in the RFC below:

1. **[MUST-FIX 1]** `ConfigErrorCode` typed enum introduced; `ConfigError.code` changed to `ConfigErrorCode`; summary table updated.
2. **[MUST-FIX 2]** New validation rule for `extends`/`profile` incoherence; new error code `CONFIG_PROFILE_EXTENDS_CONFLICTS_WITH_PROFILE`; §3.5 clarified.
3. **[MUST-FIX 3]** Boolean capability escalation rule added to §3.6; new error code `CONFIG_PROFILE_CAPABILITY_ESCALATION`; summary table updated.

SHOULD-FIX items:
- **[SHOULD-FIX 1]** Degradation threshold ordering validation added; new error code `CONFIG_DEGRADATION_THRESHOLD_ORDER`.
- **[SHOULD-FIX 2]** `emit_schema` vs. `--print-schema` semantics clarified in §2.2 and §8.

---

**Round 2 scores:**
- Doctrinal Alignment: 4/5
- Technical Robustness: 4/5 (after fixes applied)
- Cross-RFC Consistency: 4/5

No dimension below 3. Round 2 complete.
