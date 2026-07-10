## MODIFIED Requirements

### Requirement: Component Type Contract
The specification defines **component types** — named contracts describing the visual-semantic identity of a class of HUD components. Each component type MUST declare:
1. A unique name (kebab-case)
2. The surface it governs. In v1 this is exactly one zone type per component type. A **promotion-era component type** (one whose first-class surface exists only after the relevant RFC promotion gate passes — currently only `text-portal`, gated on RFC 0013 §7.2) MAY instead govern a **first-class portal surface composed of named parts** rather than a single zone type; each styled part consumes `RenderingPolicy` like a zone does.
3. A **readability technique** requirement: one of `DualLayer` (backdrop + outline required), `OpaqueBackdrop` (backdrop with opacity >= threshold required), or `None` (no readability requirement). A multi-part surface declares a readability technique **per text-bearing part**; geometry-only parts declare `None`.
4. A list of specific canonical token keys that MUST be resolvable (from profile overrides, global tokens, or canonical fallbacks) for any active profile of this type
5. Informal geometry expectations (documented for profile authors; not validated at startup)

Component types are specification-defined constants — they are NOT user-configurable in v1. The v1 component types are: `subtitle`, `notification`, `status-bar`, `alert-banner`, `ambient-background`, and `pip`. The `text-portal` component type is defined **in addition to** the six v1 types and is recognized only after its RFC 0013 §7.2 promotion gate passes; it does not alter the six v1 zone-governing types.
Scope: v1-mandatory

#### Scenario: Component type declares governed zone
- **WHEN** the `subtitle` component type is defined
- **THEN** it MUST declare that it governs the `subtitle` zone type and MUST specify readability technique `DualLayer`

#### Scenario: Component type lists specific required tokens
- **WHEN** the `subtitle` component type is defined
- **THEN** it MUST list specific token keys: `color.text.primary`, `color.backdrop.default`, `opacity.backdrop.default`, `color.outline.default`, `typography.subtitle.family`, `typography.subtitle.size`, `typography.subtitle.weight`, `stroke.outline.width`

#### Scenario: All six v1 component types defined
- **WHEN** the runtime starts
- **THEN** the following component types MUST be recognized: `subtitle`, `notification`, `status-bar`, `alert-banner`, `ambient-background`, `pip`

#### Scenario: Promotion-era component type governs a multi-part surface
- **WHEN** the `text-portal` component type is defined after its RFC 0013 §7.2 promotion gate passes
- **THEN** it MUST govern a first-class portal surface composed of named parts (rather than a single zone type)
- **AND** each styled part MUST declare a readability technique and consume `RenderingPolicy`, while the six v1 zone-governing component types remain unchanged

---

## ADDED Requirements

### Requirement: Text-Portal Component Type
The `text-portal` component type defines the visual-semantic identity of the first-class text stream portal surface that RFC 0013 §7.2 promotion permits. It is the promotion-era successor to the Phase-0 raw-tile portal assembly, satisfying the Phase-1 contract that the portal "be styled through a `text-portal` component type contract defined by a separate component-shape-language delta, consuming `RenderingPolicy` fields like other component types." The component type MUST declare:

- **Name:** `text-portal` (kebab-case).
- **Governed surface:** the first-class portal surface (or node type) introduced by the RFC 0013 §7.2 promotion, expressed as the named parts in the Text-Portal Surface Part Model requirement — NOT a single zone type. Until the promotion surface exists, the same contract is expressible on the raw-tile pilot per RFC 0013 §7, and this component type is inert.
- **Readability technique:** declared **per text-bearing part** (see Text-Portal Readability Enforcement). The surface-level default for text-bearing parts is `OpaqueBackdrop`; geometry-only parts are `None`.
- **Required tokens** (all drawn from the existing Canonical Token Schema — this delta introduces NO new canonical token key): `color.text.primary`, `color.text.secondary`, `color.backdrop.default`, `color.border.default`, `color.outline.default`, `opacity.backdrop.opaque`, `typography.heading.family`, `typography.heading.size`, `typography.heading.weight`, `typography.body.family`, `typography.body.size`, `typography.body.weight`, `spacing.padding.medium`, `stroke.border.width`, `stroke.outline.width`.
- **Geometry expectation (informational):** a content-layer, lease-governed, movable and resizable two-pane surface (transcript pane + composer pane) with a header band and a collapsed-card state; corner/edge resize affordances and a pane divider. Geometry is governed by the portal surface's own bounds/lease, not validated against the component type.

This `text-portal` contract's required-tokens list reuses existing canonical keys verbatim and introduces no new canonical key. Portal-specific token **canonicalization** was subsequently delivered by the follow-up change (`hud-8691s`, Promotion P2), which shipped a profile-scoped `portal.*` token namespace of 59 keys (`crates/tze_hud_config/src/portal_tokens.rs` — the `PORTAL_TOKEN_*` consts and their `PORTAL_TOKEN_DEFAULT_STRINGS` defaults), resolved via profile-scoped overrides with canonical-token fallbacks. The required-tokens list above intentionally stays on the pre-existing canonical keys so the promotion-era styling contract is expressible independently of the `portal.*` namespace.
Scope: post-v1

#### Scenario: text-portal declares its governed surface and required tokens
- **WHEN** the `text-portal` component type is defined
- **THEN** it MUST declare that it governs the first-class portal surface expressed as named parts (not a single zone type)
- **AND** every required token key it lists MUST already exist in the Canonical Token Schema, with no new canonical key introduced by this delta

#### Scenario: text-portal does not alter the six v1 component types
- **WHEN** the runtime resolves component types with `text-portal` defined
- **THEN** the six v1 component types (`subtitle`, `notification`, `status-bar`, `alert-banner`, `ambient-background`, `pip`) MUST remain defined exactly as before
- **AND** `text-portal` MUST be recognized only after its RFC 0013 §7.2 promotion gate passes

#### Scenario: portal token canonicalization delivered by P2
- **WHEN** an author asks whether this delta introduces a portal-namespaced canonical token key (e.g. a `portal.*` namespace)
- **THEN** this delta MUST introduce no new canonical key — the `text-portal` required-tokens list references only pre-existing canonical keys
- **AND** portal-specific canonicalization was delivered by the P2 follow-up change `hud-8691s`, which shipped a 59-key profile-scoped `portal.*` token namespace (`crates/tze_hud_config/src/portal_tokens.rs`), resolved via profile-scoped overrides with canonical-token fallbacks

---

### Requirement: Text-Portal Surface Part Model
The `text-portal` component type styles a fixed set of named **parts**. The part set MUST enumerate every visual region of the portal surface and MUST cross-map to the Phase-0 raw-tile assembly so that promotion preserves, rather than redefines, the proven layout. The named parts and their raw-tile expression are:

| Part | Phase-0 raw-tile expression | Text-bearing | Readability |
|---|---|---|---|
| `frame` | `frame` tile — surface backdrop, outer border, and footer/status chrome | partial (footer meta) | OpaqueBackdrop |
| `header` | header band within the `frame` tile — title + subtitle; also the move/drag handle | yes | OpaqueBackdrop |
| `composer` | `input_scroll` tile — bounded draft text, caret, and selection | yes | OpaqueBackdrop |
| `transcript` | `output_scroll` tile — markdown-rendered transcript window | yes | OpaqueBackdrop |
| `divider` | vertical divider sub-element of the `frame` tile — pane split and resize handle | no | None |
| `collapsed-card` | `minimized_icon` tile — collapsed/minimized representation | yes | OpaqueBackdrop |
| `capture-backstop` | `capture_backstop` tile — full-bounds input/redaction backstop beneath the surface | no | None |
| `gesture-shield` | `drag_shield` tile — transient move/resize gesture capture; hosts the scroll-position indicator | no | None |

Every part MUST be styled exclusively from resolved design-token values (via `RenderingPolicy` for text-bearing and backdrop styling, and via the existing border-token pattern for non-text strokes); no part may carry hardcoded compositor colors, typography, spacing, or strokes. The eight parts MUST account for all six Phase-0 portal tiles (`capture_backstop`, `frame`, `input_scroll`, `output_scroll`, `drag_shield`, `minimized_icon`) plus the frame-internal `divider` sub-element. This part model adds no new portal capability: it is the styling decomposition of the surface that RFC 0013 §7.2 promotes, and it preserves every standing portal non-goal (see Text-Portal Profile Styling and Promotion Scope Boundary).
Scope: post-v1

#### Scenario: every portal part is enumerated and token-styled
- **WHEN** the `text-portal` component type is rendered
- **THEN** the `frame`, `header`, `composer`, `transcript`, `divider`, `collapsed-card`, `capture-backstop`, and `gesture-shield` parts MUST each resolve their styling from design tokens
- **AND** no part MUST carry a hardcoded compositor color, typography, spacing, or stroke value

#### Scenario: part model cross-maps to the six-tile raw assembly
- **WHEN** the named parts are mapped to the Phase-0 raw-tile assembly
- **THEN** the six tiles `capture_backstop`, `frame`, `input_scroll`, `output_scroll`, `drag_shield`, and `minimized_icon` MUST each be covered by exactly one or more named parts
- **AND** the `divider` part MUST map to the frame-internal pane-split/resize sub-element of the `frame` tile

#### Scenario: geometry-only parts carry no text styling
- **WHEN** the `capture-backstop`, `gesture-shield`, or `divider` part is styled
- **THEN** it MUST consume only backdrop/border/indicator styling (no text color or typography)
- **AND** its readability technique MUST be `None`

---

### Requirement: Text-Portal Part RenderingPolicy Consumption
Each text-bearing or backdrop-bearing part of the `text-portal` surface MUST consume `RenderingPolicy` fields (from the Extended RenderingPolicy requirement) exactly as zones do — the portal is NOT exempt from the "never hardcode visuals" rule. The per-part consumption is:

- **`frame`:** `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`. The outer border uses `color.border.default` + `stroke.border.width` via the same compositor border-token pattern the `notification` component type uses (rendered as edge quads), since `RenderingPolicy` outline fields style text, not frame chrome. Footer/status text consumes `text_color` ← `color.text.secondary`, `font_family`/`font_size_px` ← `typography.body.*`.
- **`header`:** `text_color` ← `color.text.primary`; `font_family`/`font_size_px`/`font_weight` ← `typography.heading.*` for the title and `typography.body.*` for the subtitle; `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`; `margin_horizontal`/`margin_vertical` ← `spacing.padding.medium`.
- **`composer`:** `text_color` ← `color.text.primary`; `font_family`/`font_size_px`/`font_weight` ← `typography.body.*`; `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`; `margin_horizontal`/`margin_vertical` ← `spacing.padding.medium`. The caret and selection are geometry rendered locally per the input model; their color follows `text_color`.
- **`transcript`:** `text_color` ← `color.text.primary`; `font_family`/`font_size_px`/`font_weight` ← `typography.body.*`; `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`; `margin_horizontal`/`margin_vertical` ← `spacing.padding.medium`. The Phase-1 markdown subset styling (heading scale, emphasis, inline/fenced code, list indentation, link treatment) resolves from the same token set per the Phase-1 Markdown Rendering Subset requirement and MUST NOT introduce hardcoded styling.
- **`collapsed-card`:** `text_color` ← `color.text.secondary`; `font_family`/`font_size_px` ← `typography.body.*`; `backdrop` ← `color.backdrop.default`; `backdrop_opacity` ← `opacity.backdrop.opaque`; border via `color.border.default` + `stroke.border.width`.
- **`divider` / `gesture-shield` scroll-position indicator:** stroke/fill from `color.border.default` (or `color.text.secondary` for the indicator) + `stroke.border.width`; geometry only.
- **`capture-backstop`:** when the viewer's policy redacts the portal, this part renders the neutral redaction treatment from `color.backdrop.default` + `opacity.backdrop.opaque`; it carries no text.

Collapsed↔expanded state transitions MUST use `RenderingPolicy.transition_in_ms` / `transition_out_ms` built on the existing zone-transition opacity mechanics; the durations are token-derived once portal token keys are canonicalized (P2), and until then use the existing zone-transition defaults. A transition MUST NOT reveal transcript content past the active redaction policy at any frame.
Scope: post-v1

#### Scenario: header and transcript consume RenderingPolicy text fields
- **WHEN** the `header` and `transcript` parts render text
- **THEN** each MUST construct its text from `RenderingPolicy` fields (`text_color`, `font_family`, `font_size_px`, `font_weight`) resolved from the listed canonical tokens
- **AND** neither part MUST read a hardcoded font, size, weight, or color

#### Scenario: frame border follows the existing border-token pattern
- **WHEN** the `frame` part renders its outer border
- **THEN** the border color MUST come from `color.border.default` and the border width from `stroke.border.width`, rendered as edge quads in the same manner as the `notification` component type
- **AND** the `frame` part's surface backdrop MUST come from `RenderingPolicy.backdrop` / `backdrop_opacity`

#### Scenario: collapsed-to-expanded transition is opacity-only and redaction-safe
- **WHEN** a collapsed `text-portal` expands for a viewer not permitted to see its transcript
- **THEN** the transition MUST animate via `RenderingPolicy.transition_in_ms` on the existing zone-transition opacity mechanics
- **AND** every frame of the transition MUST show the neutral redaction treatment in place of transcript content

---

### Requirement: Text-Portal Readability Enforcement
The text-bearing parts of the `text-portal` surface (`header`, `composer`, `transcript`, `collapsed-card`, and the `frame` footer text) MUST enforce readability through validation of each part's effective `RenderingPolicy` at startup, exactly as the Zone Readability Enforcement requirement validates zones. Each text-bearing part's required technique is `OpaqueBackdrop`: the effective `RenderingPolicy` MUST satisfy `backdrop` is `Some(color)` (not fully transparent) and `backdrop_opacity` is `Some(v)` where `v >= 0.8`. The geometry-only parts (`capture-backstop`, `gesture-shield`, `divider`) require `None`. A profile MAY additionally apply `outline_color` + `outline_width` to a text-bearing part for extra legibility, but outline is not required for `text-portal`. Failure MUST produce `PROFILE_READABILITY_VIOLATION` identifying the component type `text-portal`, the failing part, the failing check, and the actual field values. Development-build relaxation (WARN instead of hard error under `profile = "headless"` or `TZE_HUD_DEV=1`) applies to `text-portal` exactly as it does to the v1 component types.
Scope: post-v1

#### Scenario: opaque transcript pane passes readability
- **WHEN** the `transcript` part's effective `RenderingPolicy` has `backdrop = Some(color.backdrop.default)` and `backdrop_opacity = Some(0.9)`
- **THEN** the `OpaqueBackdrop` readability check for that part MUST pass

#### Scenario: translucent composer fails readability
- **WHEN** the `composer` part's effective `RenderingPolicy` has `backdrop = Some(color)` but `backdrop_opacity = Some(0.5)`
- **THEN** the readability check MUST fail with `PROFILE_READABILITY_VIOLATION` identifying component type `text-portal`, the `composer` part, and "OpaqueBackdrop: backdrop_opacity must be >= 0.8, got 0.5"

#### Scenario: geometry-only part skips readability
- **WHEN** the `capture-backstop` or `gesture-shield` part is validated
- **THEN** no readability check MUST be performed for that part (technique `None`)

---

### Requirement: Text-Portal Profile Styling and Promotion Scope Boundary
A component profile with `component_type = "text-portal"` MUST be able to reskin the portal surface — every named part — purely through `[token_overrides]` and zone-style rendering overrides, without changing adapter logic or runtime behavior, consistent with the Component Profile Format and Zone Rendering Override Schema requirements. The text-portal profile MUST follow the same loading, scoped-token-resolution, and validation rules as v1 component-type profiles.

Defining the `text-portal` component type MUST NOT change any standing portal non-goal established by RFC 0013 §7.2 and the text-stream-portals Promotion Scope Boundary. Specifically, this component type MUST NOT add or imply: terminal emulation (VT100/ANSI cursor addressing, alternate screen, PTY hosting), full transcript history materialized in the scene graph, chrome-layer portal UI, a dedicated portal transport or a second long-lived portal stream, or runtime ownership of external process lifecycles. The styled surface MUST remain lease-governed, content-layer, redactable, and subordinate to the attention model exactly as the raw-tile pilot is. This delta is a styling-contract decomposition only; it grants no new runtime capability beyond what the RFC 0013 §7.2 promotion already permits.
Scope: post-v1

#### Scenario: profile reskins the portal without code changes
- **WHEN** an operator activates a `text-portal` profile whose `[token_overrides]` change portal-relevant colors, typography, spacing, and strokes
- **THEN** the `frame`, `header`, `composer`, `transcript`, `divider`, and `collapsed-card` parts MUST reflect the new token values on re-render
- **AND** no adapter logic or runtime behavior MUST change to achieve the reskin

#### Scenario: component type adds no excluded-scope capability
- **WHEN** the `text-portal` component type is reviewed against the portal non-goals
- **THEN** it MUST NOT introduce terminal emulation, scene-graph transcript history, chrome-layer portal UI, a dedicated portal transport or second portal stream, or runtime process ownership
- **AND** the styled surface MUST remain lease-governed, content-layer, redactable, and attention-subordinate
