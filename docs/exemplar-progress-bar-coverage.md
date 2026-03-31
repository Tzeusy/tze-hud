# Exemplar Progress Bar — Spec-to-Code Coverage Report

**Issue**: hud-d9jr.5  
**Date**: 2026-03-31  
**Spec**: `openspec/changes/exemplar-progress-bar/specs/exemplar-progress-bar/spec.md`  
**Tasks ref**: `openspec/changes/exemplar-progress-bar/tasks.md`

---

## Summary

The spec defines 7 requirements and 26 scenarios. The underlying infrastructure
(widget loader, binding resolution, compositor rasterization, interpolation,
config validation, MCP publish_to_widget) is **fully implemented and correct**.
The gap is entirely in exemplar-specific deliverables: no `progress-bar` bundle
exists, no instance is wired into the default config, and no MCP test fixtures or
user-test integration have been created.

| Category | Implemented | Partial | Missing |
|---|---|---|---|
| Bundle infrastructure (loader, format, errors) | Yes | — | — |
| Bundle asset files (widget.toml, track.svg, fill.svg) | — | — | **Yes** |
| Config registration (widget_bundle_paths, [[tabs.widgets]]) | — | — | **Yes** |
| MCP test fixtures (progress-bar-step.json, color-sweep.json) | — | — | **Yes** |
| User-test integration (SKILL.md, named scenario) | — | — | **Yes** |
| Parameter schema handling (f32 clamp, string, color direct) | Yes | — | — |
| Binding resolution (linear, direct, text-content) | Yes | — | — |
| Parameter interpolation (f32 linear, color component-wise, string snap) | Yes | — | — |
| Rasterization pipeline (<2ms budget, dirty cache, texture reuse) | Yes | — | — |
| SVG token placeholder resolution | Yes | — | — |

**Overall status: infrastructure ready, exemplar deliverables not yet created.**

---

## Requirement 1: Progress Bar Asset Bundle Structure

**Status: NOT IMPLEMENTED**

The spec requires a bundle directory at `assets/widget_bundles/progress-bar/`
(tasks.md task 1.2 calls for `assets/widget_bundles/progress-bar/`; the spec
names the directory `progress-bar`) containing `widget.toml`, `track.svg`, and
`fill.svg`.

### Evidence of infrastructure readiness

- `crates/tze_hud_widget/src/loader.rs`: `scan_bundle_dirs` / `load_bundle_dir_inner`
  correctly loads TOML manifests, validates required fields (`name`, `version`),
  checks SVG file presence, and emits `WIDGET_BUNDLE_MISSING_SVG` when an SVG
  layer is absent.
- `crates/tze_hud_widget/src/error.rs`: `BundleError::MissingSvg` implements
  `WIDGET_BUNDLE_MISSING_SVG` wire code (covers scenario "Bundle rejected when
  SVG file missing").
- `crates/tze_hud_widget/tests/bundle_loader.rs`: The `gauge` fixture exercises
  the same manifest structure that `progress-bar` would use.

### Gaps

| Scenario | Status | Gap |
|---|---|---|
| Bundle loads successfully with default tokens | NOT TESTED | `assets/widget_bundles/progress-bar/` directory absent |
| Bundle rejected when SVG file missing | COVERED BY INFRA | error code path exists, but no progress-bar-specific test |

### Missing files

- `assets/widget_bundles/progress-bar/widget.toml` — manifest with name, version, description, parameter_schema (3 params), layers array
- `assets/widget_bundles/progress-bar/track.svg` — static rounded-rect background
- `assets/widget_bundles/progress-bar/fill.svg` — dynamic fill bar and label

---

## Requirement 2: Progress Bar Parameter Schema

**Status: INFRASTRUCTURE IMPLEMENTED, BUNDLE NOT CREATED**

The loader handles all three required parameter types:

- `f32` with `f32_min`/`f32_max` constraints: parsed in `parse_parameter_schema` → `WidgetParamConstraints::f32_min/f32_max`
- `string` with `string_max_bytes`: handled in `WidgetParamConstraints::string_max_bytes`
- `color` with RGBA `[r,g,b,a]` default: parsed via `parse_default_value` → `WidgetParameterValue::Color(Rgba::new(...))`

Clamping is implemented in `crates/tze_hud_scene/src/graph.rs:publish_to_widget`
(line ~2447–2454): f32 values outside `[min, max]` are silently clamped to range
rather than rejected, matching the spec requirement exactly.

### Gaps

| Scenario | Status | Gap |
|---|---|---|
| Publish valid progress value (0.75 → 75% fill) | INFRA READY | No bundle, no instance |
| Progress value clamped to range (1.5 → 1.0) | INFRA READY | No bundle, no instance |
| Label published as text-content | INFRA READY | No bundle, no instance |
| Empty label shows no text | INFRA READY | No bundle, no instance |
| Fill color overrides token default | INFRA READY | No bundle, no instance |

All five scenarios depend on the bundle being created and an instance being
registered. The clamping path at `graph.rs:publish_to_widget` is tested via
the gauge fixture (`widget_publish_f32_above_max_clamped`) but not for a
progress-bar-specific `progress` parameter.

---

## Requirement 3: Progress Bar SVG Track Layer

**Status: NOT IMPLEMENTED**

`track.svg` does not exist. The spec requires:

- `viewBox="0 0 300 20"`
- `<rect id="track-bg" x="0" y="0" width="300" height="20" rx="10" ry="10" fill="{{token.color.backdrop.default}}" fill-opacity="0.3"/>`
- Token placeholder `{{token.color.backdrop.default}}` resolved at bundle load time

### Infrastructure readiness

- Token placeholder resolution is implemented in `loader.rs:resolve_token_placeholders`
  (called in `load_bundle_dir_inner` at step 5b-post, before SVG parse).
- `BundleError::UnresolvedToken` is implemented for missing token keys.
- The canonical token `color.backdrop.default` has a hardcoded fallback `#000000`
  per the design.md risk note.

### Gaps

| Scenario | Status | Gap |
|---|---|---|
| Track renders with token-resolved backdrop color | NOT IMPLEMENTED | track.svg absent |
| Track renders with operator-overridden backdrop color | NOT IMPLEMENTED | track.svg absent; no test |
| Track has rounded end-caps | NOT IMPLEMENTED | track.svg absent |

---

## Requirement 4: Progress Bar SVG Fill Layer

**Status: NOT IMPLEMENTED**

`fill.svg` does not exist. The spec requires:

- `viewBox="0 0 300 20"`
- `<rect id="fill-bar" x="2" y="2" width="0" height="16" rx="8" ry="8" fill="{{token.color.text.accent}}"/>`
- `<text id="label-text" x="150" y="10" text-anchor="middle" dominant-baseline="central" font-size="11" fill="{{token.color.text.primary}}"></text>`

### Infrastructure readiness

- `apply_svg_attribute` in `crates/tze_hud_compositor/src/widget.rs` handles
  attribute replacement for SVG elements identified by `id`.
- `apply_svg_text_content` handles the text-content synthetic target (replaces
  content between `<tag>` and `</tag>`).
- `resolve_binding_value` computes the correct width attribute for a linear
  binding: `attr_min + normalized * (attr_max - attr_min)` — at `progress=0.5`,
  `attr_min=0`, `attr_max=296`: result is `148`.
- `rgba_to_svg_color` converts `WidgetParameterValue::Color` to hex or rgba()
  string for direct bindings.

### Gaps

| Scenario | Status | Gap |
|---|---|---|
| Fill bar width tracks progress linearly (0.5 → width=148) | NOT IMPLEMENTED | fill.svg absent |
| Fill bar at zero progress (width=0) | NOT IMPLEMENTED | fill.svg absent |
| Fill bar at full progress (width=296) | NOT IMPLEMENTED | fill.svg absent |
| Fill color uses token-resolved default (#4A9EFF) | NOT IMPLEMENTED | fill.svg absent |
| Label text renders centered | NOT IMPLEMENTED | fill.svg absent |
| Label text uses primary color token | NOT IMPLEMENTED | fill.svg absent |

---

## Requirement 5: Progress Bar Parameter Bindings

**Status: INFRASTRUCTURE IMPLEMENTED, BUNDLE NOT CREATED**

The widget.toml binding spec requires three bindings on layer 1 (fill.svg):

1. Linear: `progress` → `fill-bar.width` with `attr_min=0`, `attr_max=296`
2. Direct: `fill_color` → `fill-bar.fill`
3. Text-content: `label` → `label-text.text-content`

All three binding types are implemented in `loader.rs:resolve_bindings` and
`parse_binding_mapping`:

- `"linear"` mapping: validated for F32 only; extracts `attr_min`/`attr_max`
- `"direct"` mapping: validated for String and Color only
- `"discrete"` mapping: validated for Enum only (not needed for progress-bar)

Text-content binding uses `target_attribute = "text-content"` which is the
synthetic target string intercepted by `apply_svg_attribute` → `apply_svg_text_content`.

`WIDGET_BINDING_UNRESOLVABLE` error code is implemented for:
- Nonexistent parameter name in binding
- Nonexistent SVG element ID in binding
- Type-incompatible mapping (e.g. linear on a string param)

### Gaps

| Scenario | Status | Gap |
|---|---|---|
| Linear binding resolves at registration | NOT TESTED | No progress-bar bundle or test fixture |
| Direct color binding resolves at registration | NOT TESTED | No progress-bar bundle or test fixture |
| Text-content binding resolves at registration | NOT TESTED | No progress-bar bundle or test fixture |
| Binding to missing element rejected | COVERED BY INFRA | `BindingUnresolvable` tested generically, not for progress-bar |

Note: There is a subtle mismatch in binding type for the text-content binding.
The spec says the binding type is `"text-content"` but the existing
infrastructure only recognizes `"linear"`, `"direct"`, and `"discrete"` as
mapping strings. Text-content is handled as a **synthetic target attribute**
(the string `"text-content"` as `target_attribute`) with mapping type `"direct"`.
The gauge bundle and all tests confirm this: `label` uses `mapping = "direct"`
and `target_attribute = "text-content"`. The tasks.md confirms the same
convention. The spec's description of "text-content binding" is a capability
description, not a separate `mapping` type string. **No gap here once the bundle
is authored correctly.**

---

## Requirement 6: Progress Bar Interpolation Behavior

**Status: INFRASTRUCTURE IMPLEMENTED, NOT TESTED FOR PROGRESS-BAR**

The spec requires:
- `progress` (f32): linear interpolation
- `label` (string): snap at t=0
- `fill_color` (color): component-wise linear sRGB interpolation

All three behaviors are implemented in `crates/tze_hud_compositor/src/widget.rs:interpolate_param`:

```rust
// F32 → linear: a + (b - a) * t
// Color → component-wise: ca.r + (cb.r - ca.r) * t (etc.)
// String/Enum → snap: returns new.clone()
```

`compute_transition_t` handles the `transition_ms > 0` case and snaps under
`DegradationLevel::Significant` or higher.

`WidgetAnimationState` tracks `start`, `duration_ms`, `from_params`, `to_params`
for in-flight transitions.

### Gaps

| Scenario | Status | Gap |
|---|---|---|
| Smooth progress animation (0→1.0 over 200ms, width=148 at 100ms) | NOT TESTED | No progress-bar bundle; gauge tests cover f32 interpolation generically |
| Label snaps during transition (label="75%" immediate, fill animates) | NOT TESTED | No progress-bar bundle |
| Color interpolates during transition (fill_color component-wise over 300ms) | NOT TESTED | No progress-bar bundle |
| Zero transition applies immediately (transition_ms=0 → next-frame) | INFRA READY | Covered generically; no progress-bar-specific test |

---

## Requirement 7: Progress Bar Rendering Performance

**Status: INFRASTRUCTURE IMPLEMENTED, NOT TESTED AT 300x20 GEOMETRY**

The spec requires re-rasterization < 2ms at 300x20 on reference hardware.

The CI budget test in `crates/tze_hud_compositor/tests/widget_rasterize_budget.rs`
tests the gauge at 512x512 with a 500ms CI threshold and references the 2ms spec
target. The Criterion benchmark in `benches/widget_rasterize.rs` enforces the 2ms
target on release hardware.

The progress-bar's 300x20 geometry is far simpler than the 512x512 gauge fixture
(two rects and one text element vs. the gauge's multi-element SVG), so it will
trivially pass the 2ms budget. However, there is no dedicated benchmark or test
for the progress-bar geometry.

### Gaps

| Scenario | Status | Gap |
|---|---|---|
| Re-rasterization under budget (<2ms at 300x20) | NO DEDICATED TEST | Gauge benchmark covers general case; no progress-bar fixture |
| Cached texture reused when unchanged | INFRA READY | `WidgetTextureEntry::dirty` flag + `last_rendered_params` implements this; no progress-bar test |

---

## Requirement 8: Progress Bar MCP Test Fixtures

**Status: NOT IMPLEMENTED**

The spec requires fixture files in `.claude/skills/user-test/scripts/`:

1. `progress-bar-step.json` — 7-step publish sequence (progress 0→0.25→0.5→0.75→1.0, then color override, then reset)
2. `progress-bar-color-sweep.json` — 4-step color cycling fixture (tasks.md task 3.2)

The existing scripts directory contains gauge and subtitle fixtures but **no
progress-bar fixtures**.

| Scenario | Status | Gap |
|---|---|---|
| Full test fixture sequence executes without errors | NOT IMPLEMENTED | progress-bar-step.json absent |
| Transition produces visible animation between steps | NOT IMPLEMENTED | progress-bar-step.json absent |

---

## Requirement 9: Progress Bar User-Test Integration

**Status: NOT IMPLEMENTED**

The spec requires:

1. A named user-test scenario `progress-bar-widget`
2. Documentation in `.claude/skills/user-test/SKILL.md` for the progress-bar
   test scenario
3. Visual acceptance criteria for each step

The current `SKILL.md` covers only zone publishes and the gauge cycling test
(Step 5). There is no `progress-bar-widget` scenario section.

| Scenario | Status | Gap |
|---|---|---|
| User-test discovers registered widget (list_widgets) | INFRA READY | No bundle registered; list_widgets returns empty |
| User-test publishes progress sequence (0.0→1.0) | NOT IMPLEMENTED | Fixture absent; no SKILL.md section |

---

## Complete Gap List

The following items from `tasks.md` have not been implemented:

| Task | Description |
|---|---|
| 1.1 | Create `assets/widget_bundles/progress-bar/` directory |
| 1.2 | Create `widget.toml` with name, version, description, 3-param schema, 2-layer array |
| 1.3 | Create `track.svg` with viewBox="0 0 300 20", `<rect id="track-bg">` with token placeholder |
| 1.4 | Create `fill.svg` with viewBox="0 0 300 20", `<rect id="fill-bar">` and `<text id="label-text">` with token placeholders |
| 1.5 | Add `assets/widget_bundles/progress-bar` to `[widget_bundle_paths]` in default config |
| 1.6 | Verify bundle loads at startup (no WIDGET_BUNDLE_* errors, list_widgets returns "progress-bar") |
| 2.1 | Add `[[tabs.widgets]]` entry for progress-bar with geometry `{x=10, y=400, width=300, height=20}` |
| 2.2 | Verify widget instance appears in SceneSnapshot via list_widgets |
| 3.1 | Create `.claude/skills/user-test/scripts/progress-bar-step.json` (7-step fixture) |
| 3.2 | Create `.claude/skills/user-test/scripts/progress-bar-color-sweep.json` (4-step color cycling) |
| 3.3 | Verify fixture JSON parses correctly |
| 4.1 | Add progress-bar-widget scenario to `.claude/skills/user-test/SKILL.md` |
| 4.2 | Register `progress-bar-step.json` as a named test group in user-test skill |
| 5.1–5.5 | End-to-end validation (runtime, fixtures, cross-machine user-test) |

---

## Infrastructure Correctness Notes

The following potential issues were noticed during the audit but are NOT bugs —
they are design decisions confirmed by the codebase and tasks.md:

1. **`widget_bundle_paths` vs `widget_bundles.paths`**: The config raw struct
   (`tze_hud_config/src/raw.rs`) uses `[widget_bundles]` with a `paths` array.
   Tasks.md task 1.5 says "add to `[widget_bundle_paths]` array" — this appears
   to be a shorthand for the `[widget_bundles].paths` key. Not a spec gap, but
   the tasks.md wording could be clearer.

2. **Default config location**: There is no single "default.toml" in the repo
   root. The only runtime config is `examples/vertical_slice/config/production.toml`
   which does not yet declare any `[widget_bundles]` section. Tasks 1.5 and 2.1
   need a target config file to be identified or created.

3. **Token `color.backdrop.default` fallback**: The design.md states this resolves
   to `#000000`. The loader's `scan_bundle_dirs` passes a `tokens` map from the
   runtime; the canonical fallbacks must be registered before bundle scanning.
   This is an integration concern, not a gap in the loader itself.

4. **`color.text.accent` default**: The spec's `fill_color` schema default of
   `[74, 158, 255, 255]` matches the canonical `#4A9EFF` fallback documented in
   the design. The SVG template uses `{{token.color.text.accent}}` for the visual
   default, while the parameter default is used only when `fill_color` is
   explicitly published without a value. This dual-default behavior is correct per
   spec §Note on fill_color default interaction.
