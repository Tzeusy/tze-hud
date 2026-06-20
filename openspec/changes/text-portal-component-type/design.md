# Design — Text-Portal Component Type

## 1. Scope and sequencing

This change is **P1** of the portal promotion track (epic `hud-g1ena`). It is a docs/spec-only delta to `component-shape-language`. It is the styling contract; the two changes it gates are intentionally out of scope:

- **P2 (`hud-8691s`):** canonicalize portal token keys. This delta deliberately reuses existing canonical keys and changes none, so P2 can introduce a `portal.*` namespace and transition-duration keys against a stable, reviewed part model.
- **P3 (`hud-tc153`):** first-class portal-surface / node-type scene-mutation schema. This delta describes how the surface is *styled*, not the protobuf/scene-mutation additions the surface *is*.

Sequencing this styling contract first lets P2 and P3 build on a fixed enumeration of parts and token consumption rather than inventing both at once.

## 2. Why a multi-part surface, not a single zone

The six v1 component types each govern exactly one zone type with one effective `RenderingPolicy`. The portal is not a single zone — Phase 0 proved it as a six-tile raw assembly (`capture_backstop`, `frame`, `input_scroll`, `output_scroll`, `drag_shield`, `minimized_icon`) plus a frame-internal divider. Promotion does not collapse that into one zone; it names the regions and gives each one token-driven styling. So the Component Type Contract is extended (minimally) to allow a promotion-era component type to govern a **named multi-part surface**, with readability declared per text-bearing part. The v1 single-zone rule is preserved verbatim; the multi-part path is an explicitly promotion-gated addition.

## 3. Part enumeration and the raw-tile cross-map

Every named part traces to a concrete Phase-0 tile so promotion preserves the proven layout instead of redefining it:

| Part | Raw tile | Role |
|---|---|---|
| `frame` | `frame` | surface backdrop, outer border, footer/status chrome |
| `header` | `frame` (band) | title + subtitle; move/drag handle |
| `composer` | `input_scroll` | bounded draft text, caret, selection |
| `transcript` | `output_scroll` | markdown-rendered transcript window |
| `divider` | `frame` (sub-element) | pane split + resize handle |
| `collapsed-card` | `minimized_icon` | collapsed/minimized representation |
| `capture-backstop` | `capture_backstop` | full-bounds input/redaction backstop |
| `gesture-shield` | `drag_shield` | transient move/resize capture; scroll-position indicator |

This is verified against the exemplar's `PortalTiles` dataclass and tile-creation in `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py` (the divider is built inside the frame nodes, hence a frame sub-element). All six tiles plus the divider are covered.

## 4. RenderingPolicy binding

Text-bearing parts consume `RenderingPolicy` text fields (`text_color`, `font_family`, `font_size_px`, `font_weight`, `backdrop`, `backdrop_opacity`, margins) like zones. Non-text strokes (frame border, divider, scroll indicator) follow the **existing border-token pattern** the `notification` component type already uses (`color.border.default` + `stroke.border.width` rendered as edge quads) — `RenderingPolicy` outline fields style text, not chrome, so reusing the notification border pattern avoids inventing a new field. Collapsed↔expanded transitions reuse `transition_in_ms`/`transition_out_ms` on the existing zone-transition opacity mechanics; no new animation system, and the transition is redaction-safe by construction.

## 5. Readability

Every text-bearing part requires `OpaqueBackdrop` (`backdrop_opacity >= 0.8`) — the portal panes are opaque, matching `notification`/`status-bar`. Outline (`DualLayer`) is allowed but not required, since legibility comes from the opaque pane rather than overlay-on-arbitrary-background like subtitles. Geometry-only parts are `None`. Validation reuses the existing Zone Readability Enforcement machinery and the dev-build WARN relaxation.

## 6. Token discipline

All required tokens are existing canonical keys (`color.text.*`, `color.backdrop.default`, `color.border.default`, `color.outline.default`, `opacity.backdrop.opaque`, `typography.heading.*`, `typography.body.*`, `spacing.padding.medium`, `stroke.border.width`, `stroke.outline.width`). No new canonical key is introduced — that is P2's job. This keeps P1 a pure contract decomposition with zero token-schema churn.

## 7. Non-goal preservation

The contract restates and binds itself to RFC 0013 §7.2 non-goals: no terminal semantics, no scene-graph transcript history, no chrome-layer UI, no dedicated transport/second stream, no runtime process ownership. The styled surface stays lease-governed, content-layer, redactable, attention-subordinate. The change grants no capability beyond what §7.2 promotion already permits.
