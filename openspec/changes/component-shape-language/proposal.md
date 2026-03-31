## Why

Zones and widgets define *behavioral* contracts (contention, TTL, geometry policy) and *structural* templates (SVG layers, parameter bindings), but they say nothing about *visual identity*. A subtitle zone declares "bottom 5%, centered, stream-text" — but not "white text, black outline, semi-transparent backdrop, readable over any background." A gauge widget declares "f32 fill_level bound to SVG height" — but not "rounded corners, consistent stroke width, color palette that matches the rest of the HUD."

This means every widget bundle and zone rendering policy is a visual island. Two independently-authored subtitle implementations will look different — different fonts, different outline techniques, different backdrop treatments. There is no shared design vocabulary, no way to say "use this HUD's visual identity" and have all components inherit a coherent look.

Worse, the concept of "a subtitle component" — a zone + rendering policy + visual identity that can be swapped as a unit — does not exist. If I want to replace my subtitle with yours, I must swap zone type definitions, rendering policies, possibly widget bundles, and hope the geometry and parameter schemas are compatible. There is no plug-and-play contract for "things that display subtitles."

We need a **component shape language**: a system of design tokens, component profiles, and visual authoring conventions that sits above zones and widgets, giving the HUD a coherent, professional visual identity while keeping implementations modular and swappable.

## What Changes

- Introduce **design tokens** — a structured vocabulary of visual primitives (color palettes, typography scales, spacing units, stroke/shadow conventions, backdrop treatments) that all zone rendering policies and widget SVGs can reference. Tokens are defined per-HUD (not per-component) so the entire display is visually coherent.
- Introduce **component profiles** — named contracts that define "what it means to be a subtitle" (or notification, or gauge, etc.) at the visual-semantic level: expected zone type, parameter schema, geometry expectations, visual identity requirements (readability constraints, contrast ratios, outline conventions). A component profile is the swappable unit — my subtitle-profile vs your subtitle-profile.
- Define **visual authoring conventions** for SVG widget bundles — how to reference design tokens in SVG templates, required readability guarantees (e.g., text must meet WCAG contrast against arbitrary backgrounds), standard outline/backdrop techniques, and how SVGs should degrade when tokens change.
- Define a **component registry** extension — how the runtime resolves "use this component profile for subtitles" from configuration, enabling plug-and-play swapping of visual implementations without changing agent code.

## Capabilities

### New Capabilities
- `component-shape-language`: Design token system, component profile contracts, visual authoring conventions, and registry extensions that give the HUD a coherent visual identity with swappable component implementations.

### Modified Capabilities
- `configuration`: Add design token definitions and component profile selection to HUD configuration.
- `widget-system`: Widget SVG bundles reference design tokens; widget types may declare conformance to a component profile.

## Impact

- **Scene types (`tze_hud_scene::types`)**: `RenderingPolicy` struct extended with 10 new `Option<T>` fields (font_family, font_weight, text_color, backdrop_opacity, outline_color, outline_width, margin_horizontal, margin_vertical, transition_in_ms, transition_out_ms). Backward-compatible via `#[serde(default)]`.
- **Configuration schema (`tze_hud_config`)**: New `[design_tokens]`, `[component_profile_bundles]`, `[component_profiles]` TOML sections. Token value parsing functions added.
- **Widget bundle loader (`tze_hud_widget`)**: SVG placeholder resolution (`{{token.key}}`) added before SVG parsing. New error codes for unresolved tokens and readability violations.
- **Compositor (`tze_hud_compositor`)**: Zone rendering path (`render_zone_content`) refactored to read RenderingPolicy fields instead of hardcoded per-content-type colors. 8-direction text outline rendering added. Backdrop opacity override. Transition fade-in/fade-out animation.
- **Runtime startup sequence**: Six new phases inserted (token loading, profile loading, profile selection, effective policy construction, readability validation, alert severity wiring).
- **Agent API**: No changes — agents still publish by zone/widget name. The shape language is transparent to publishers.
- **Bundle authors**: New authoring guide and validation rules for SVG bundles in component profiles (`data-role` attributes, document order, stroke requirements).
