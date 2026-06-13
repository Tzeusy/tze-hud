# Component Profile Authoring Guide

Source: component-shape-language change

This guide explains how to create, configure, and validate component profiles
for tze_hud. A component profile is the swappable visual implementation unit
for a specific component type (subtitle, notification, etc.).

---

## Overview

A **component profile** is a directory that customises the visual rendering of
one component type. It may contain:

- A `profile.toml` manifest (required)
- `zones/` — per-zone rendering policy overrides (optional)
- `widgets/` — widget bundles scoped to this profile (optional)

Profiles are declared in `tze_hud.toml` under `[component_profile_bundles]`
and activated under `[component_profiles]`.

---

## Directory Structure

```
profiles/
  my-subtitles/
    profile.toml                        # required — manifest
    zones/
      subtitle.toml                     # optional — zone rendering overrides
    widgets/
      my-widget/
        widget.toml                     # optional — widget bundle manifest
        layer.svg                       # SVG layer file(s)
  my-notifications/
    profile.toml
    zones/
      notification-area.toml            # NOTE: "notification-area", not "notification"
```

The profile directory name is cosmetic. The authoritative profile name is the
`name` field in `profile.toml`.

---

## profile.toml Schema

```toml
# Required fields
name           = "my-subtitles"     # kebab-case; unique across all loaded profiles
version        = "1.0.0"            # semver
component_type = "subtitle"         # must be a valid v1 component type name

# Optional fields
description = "Cinematic subtitle style"  # defaults to ""

# Optional: profile-scoped token overrides.
# These values override global [design_tokens] ONLY for this profile's
# widget SVGs and zone rendering overrides. They do NOT affect other profiles,
# global widget bundles, or zones without an active profile.
[token_overrides]
"color.text.primary"     = "#FFFF00"   # yellow text for this profile only
"typography.subtitle.size" = "32"
```

### Valid component_type values (v1)

| component_type      | Governed zone      | Readability technique |
|---------------------|--------------------|-----------------------|
| `subtitle`          | `subtitle`         | DualLayer             |
| `notification`      | `notification-area`| OpaqueBackdrop        |
| `status-bar`        | `status_bar`       | OpaqueBackdrop        |
| `alert-banner`      | `alert_banner`     | OpaqueBackdrop        |
| `ambient-background`| `ambient_background`| None                 |
| `pip`               | `pip`              | None                  |

---

## Zone Override Files (zones/)

Each file in `zones/` overrides the `RenderingPolicy` for one zone type. The
filename **must exactly match the zone registry name** (not the component type
name). This is the most common authoring mistake:

| Component type | Correct override filename      | Wrong (rejected)      |
|----------------|--------------------------------|-----------------------|
| `notification` | `notification-area.toml`       | `notification.toml`   |
| `status-bar`   | `status_bar.toml`              | `status-bar.toml`     |
| `alert-banner` | `alert_banner.toml`            | `alert-banner.toml`   |
| `subtitle`     | `subtitle.toml`                | (no common mistake)   |

A profile for `subtitle` that contains `zones/notification-area.toml` will be
rejected at startup with `PROFILE_ZONE_OVERRIDE_MISMATCH`.

### Zone Override TOML Schema

All fields are optional. Omitted fields retain their token-derived default.

```toml
# Typography
font_family  = "monospace"             # or "{{typography.subtitle.family}}"
font_size_px = 32.0                    # pixels, or "{{typography.subtitle.size}}"
font_weight  = 600                     # CSS numeric 100-900

# Text rendering
text_color = "#FFFF00"                 # color hex, or "{{color.text.primary}}"
text_align = "center"                  # "start" | "center" | "end"

# Backdrop
backdrop_color   = "#000000"           # color hex, or "{{color.backdrop.default}}"
backdrop_opacity = 0.7                 # 0.0-1.0 float

# Outline (subtitle / DualLayer only)
outline_color = "#000000"              # color hex, or "{{color.outline.default}}"
outline_width = 2.0                    # pixels >= 1.0 for DualLayer

# Margins
margin_horizontal = 16.0              # pixels from left/right zone edges
margin_vertical   = 8.0               # pixels from top/bottom zone edges

# Transitions (opacity-only)
transition_in_ms  = 200               # ms; 0 = instant appear
transition_out_ms = 150               # ms; 0 = instant disappear
```

#### Field type rules

- **Color fields** (`text_color`, `backdrop_color`, `outline_color`): TOML
  string, parsed as `#RRGGBB` or `#RRGGBBAA` hex. Uppercase and lowercase
  both accepted.
- **Numeric fields** (`font_size_px`, `backdrop_opacity`, etc.): TOML float or
  integer, or a `"{{token.key}}"` reference that resolves to a numeric string.
- **`text_align`**: must be one of `"start"`, `"center"`, `"end"`. Any other
  value produces `PROFILE_INVALID_ZONE_OVERRIDE`.
- **`font_family`**: one of `"system-ui"`, `"sans-serif"`, `"monospace"`,
  `"serif"`. Named/custom font families are not supported in v1.
- **`transition_in_ms` / `transition_out_ms`**: TOML integer (u32).

---

## Token Reference Syntax

Use `{{token.key}}` in zone override field values and SVG attribute values to
reference design tokens. The placeholder is resolved at load time against this
priority order:

1. Profile's `[token_overrides]` (highest priority)
2. Global `[design_tokens]` from `tze_hud.toml`
3. Canonical fallback values (lowest priority)

Rules:
- No whitespace between `{{` and the key, or between the key and `}}`.
  `{{ color.text.primary }}` is NOT a valid placeholder — it passes through
  as literal text.
- Multiple placeholders may appear in a single SVG attribute value.
- Substituted values are NOT re-scanned (no recursive resolution).
- To include a literal `{{` or `}}` in SVG content, escape as `\{\{` or `\}\}`.
- An unresolvable token key produces `WIDGET_BUNDLE_UNRESOLVED_TOKEN` or
  `PROFILE_UNRESOLVED_TOKEN` and rejects the profile.

### Canonical token keys (most useful for profiles)

```
color.text.primary          #FFFFFF     — primary text color
color.text.secondary        #B0B0B0     — secondary/muted text
color.text.accent           #4A9EFF     — accent/highlight
color.backdrop.default      #000000     — default backdrop fill
color.outline.default       #000000     — text outline stroke
color.border.default        #333333     — border/frame color
opacity.backdrop.default    0.6         — default backdrop opacity
opacity.backdrop.opaque     0.9         — opaque backdrop threshold
typography.body.family      system-ui
typography.body.size        16          — px
typography.body.weight      400
typography.subtitle.family  system-ui
typography.subtitle.size    28          — px
typography.subtitle.weight  600
typography.heading.family   system-ui
typography.heading.size     24          — px
typography.heading.weight   700
spacing.padding.medium      8           — px
stroke.outline.width        2           — px
stroke.border.width         1           — px
```

---

## SVG Conventions (widgets/)

Widget SVG files in a profile's `widgets/` directory must follow these
conventions for readability validation to pass.

### data-role attributes

Elements that play a structural role must be annotated:

- `data-role="backdrop"` — backdrop rectangle or shape
- `data-role="text"` — visible text element subject to readability checks

Elements without `data-role` are decorative and skipped by readability
validation (borders, icons, dividers, etc.).

### Document order (painter's model)

SVG renders in document order — later elements paint over earlier ones.
**Backdrop elements MUST precede their associated text elements** in document
order, otherwise text would be hidden under the backdrop.

```xml
<!-- CORRECT: backdrop first, then text -->
<rect data-role="backdrop" fill="#000000" opacity="0.6" .../>
<text data-role="text" fill="#FFFFFF" stroke="#000000" ...>Hello</text>

<!-- WRONG: text first — backdrop renders on top of text -->
<text data-role="text" ...>Hello</text>
<rect data-role="backdrop" .../>
```

Violation produces `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION`.

### Stroke requirement for DualLayer profiles (subtitle)

For profiles with `component_type = "subtitle"` (DualLayer readability),
every `data-role="text"` element MUST have both `fill` and `stroke`
attributes. The `stroke-width` value (after token resolution) MUST be >= 1.0.

```xml
<!-- CORRECT for DualLayer: fill + stroke + stroke-width present -->
<text
  data-role="text"
  fill="{{color.text.primary}}"
  stroke="{{color.outline.default}}"
  stroke-width="{{stroke.outline.width}}"
  paint-order="stroke fill">...</text>

<!-- WRONG for DualLayer: missing stroke -->
<text data-role="text" fill="#FFFFFF">...</text>
```

Use `paint-order="stroke fill"` so the stroke renders behind the fill, giving
a cleaner outline effect.

### OpaqueBackdrop profiles (notification, status-bar, alert-banner)

Stroke on `data-role="text"` elements is optional. Only `fill` is required.

```xml
<!-- CORRECT for OpaqueBackdrop: fill without stroke is fine -->
<text data-role="text" fill="{{color.text.primary}}">...</text>
```

### Token placeholders in SVG

Use `{{token.key}}` in any SVG attribute value or text content. Resolution
happens at bundle load time (startup), never at render time.

```xml
<rect
  data-role="backdrop"
  fill="{{color.backdrop.default}}"
  opacity="{{opacity.backdrop.default}}"/>
<text
  data-role="text"
  font-family="{{typography.subtitle.family}}"
  font-size="{{typography.subtitle.size}}"
  fill="{{color.text.primary}}"
  stroke="{{color.outline.default}}"
  stroke-width="{{stroke.outline.width}}">...</text>
```

---

## Readability Requirements by Component Type

### DualLayer (subtitle)

All of the following must be satisfied for the zone's effective RenderingPolicy:

1. `backdrop_color` is set and not fully transparent
2. `backdrop_opacity` >= 0.3
3. `outline_color` is set
4. `outline_width` >= 1.0

For widget SVGs:
- `data-role="backdrop"` must precede `data-role="text"` in document order
- `data-role="text"` elements must have `stroke` + `stroke-width` >= 1.0

### OpaqueBackdrop (notification, status-bar, alert-banner)

All of the following must be satisfied:

1. `backdrop_color` is set and not fully transparent
2. `backdrop_opacity` >= 0.8

For widget SVGs:
- `data-role="backdrop"` must precede `data-role="text"` in document order
- Stroke on text is optional

### None (ambient-background, pip)

No readability validation is performed.

---

## Dev Mode vs Production Validation

| Mode | Readability failure | Profile result |
|------|---------------------|----------------|
| Production | Hard error (`PROFILE_READABILITY_VIOLATION`) | Startup fails if profile is referenced |
| Dev (`TZE_HUD_DEV=1` or `profile = "headless"`) | WARN log | Profile loads; rendering may be wrong |

Set `TZE_HUD_DEV=1` in the environment during iterative authoring to iterate
without restarting on every readability check failure.

An unreferenced profile that fails validation is logged and skipped in both
modes — it does not prevent startup.

---

## Example: tze_hud.toml Configuration Snippet

```toml
# Global design tokens — override canonical fallbacks for all zones and profiles
[design_tokens]
"color.text.primary"       = "#FFFFFF"
"color.backdrop.default"   = "#000000"
"opacity.backdrop.default" = "0.6"
"color.outline.default"    = "#000000"
"stroke.outline.width"     = "2"
"typography.subtitle.size" = "28"
"typography.body.size"     = "16"
"color.border.default"     = "#333333"

# Directories to scan for component profile directories.
# Each entry is resolved relative to this config file's parent directory.
# The runtime scans each path for immediate subdirectories containing profile.toml.
[component_profile_bundles]
paths = ["./profiles"]

# Activate a specific profile for each component type.
# Keys must be valid v1 component type names.
# Values must be profile names loaded from the paths above.
# Omitted component types use token-derived default rendering.
[component_profiles]
subtitle     = "reference-subtitle"
notification = "notification-stack-exemplar"
```

### Resolution order for token references in the active `reference-subtitle` profile

When the profile's `zones/subtitle.toml` contains `text_color = "{{color.text.primary}}"`:

1. Checks `reference-subtitle`'s `[token_overrides]` → found `"#FFFFFF"`
2. (If not found) checks global `[design_tokens]` in `tze_hud.toml`
3. (If not found) uses canonical fallback `"#FFFFFF"`

Profile-scoped overrides do NOT leak to other profiles or global zones.

---

## Common Mistakes

| Mistake | Error | Fix |
|---------|-------|-----|
| Wrong zone override filename (`notification.toml`) | `PROFILE_ZONE_OVERRIDE_MISMATCH` | Use zone registry name (`notification-area.toml`) |
| Whitespace inside `{{` ... `}}` | Placeholder not resolved (literal text) | Remove whitespace: `{{color.text.primary}}` |
| Missing `stroke` on DualLayer text element | `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION` | Add `stroke` + `stroke-width` >= 1.0 |
| Backdrop after text in SVG document order | `WIDGET_BUNDLE_READABILITY_CONVENTION_VIOLATION` | Move `data-role="backdrop"` element before `data-role="text"` |
| `backdrop_opacity = 0.5` on notification profile | `PROFILE_READABILITY_VIOLATION` (OpaqueBackdrop requires >= 0.8) | Increase to >= 0.8 |
| Referencing an unloaded profile name | `CONFIG_UNKNOWN_COMPONENT_PROFILE` at startup | Ensure path is in `[component_profile_bundles].paths` |
| Named font family (e.g. `"Fira Code"`) | `TOKEN_VALUE_PARSE_ERROR` at startup | Use `"monospace"`, `"system-ui"`, `"sans-serif"`, or `"serif"` |
