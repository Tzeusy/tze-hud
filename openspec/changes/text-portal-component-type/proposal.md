## Why

RFC 0013 §7.2 and the text-stream-portals Promotion Scope Boundary permit, once the promotion evidence gate passes, exactly two things: a first-class portal surface (or node type) and a **`text-portal` component type contract**. The `text-stream-portal-phase1` change deliberately stopped short of authoring that component type — adding it before promotion would specify a first-class component contract for a surface that is still raw-tile assembly, exactly the premature-promotion risk RFC 0013 §7 guards against (see `text-stream-portal-phase1/design.md` §6).

The portal is the only HUD surface still styled outside the component-shape-language system: Phase 0 chose its raw-tile colors ad hoc in the adapter, and Phase 1 routed those values through resolved design tokens but had no first-class component type to consume `RenderingPolicy` like every other surface. The repo core rule is "never hardcode visuals — use design tokens and `RenderingPolicy`." Promotion is the point at which the portal must join that system.

This change authors the `text-portal` component type contract as a reviewed component-shape-language delta. It is the promotion-era (P1) styling contract: it enumerates every portal part, binds each to `RenderingPolicy`, and reuses existing canonical tokens. It deliberately does **not** canonicalize portal-specific token keys (that is P2, `hud-8691s`) and adds no runtime capability beyond what the §7.2 promotion already permits.

## What Changes

- **MODIFY** the `component-shape-language` "Component Type Contract" requirement so a promotion-era component type may govern a first-class multi-part portal surface (not just a single zone type), with a readability technique declared per text-bearing part. The six v1 zone-governing component types are unchanged.
- **ADD** the `text-portal` component type: name, governed surface, per-part readability, required tokens (existing canonical keys only), and informational geometry.
- **ADD** the Text-Portal Surface Part Model: the eight named parts (`frame`, `header`, `composer`, `transcript`, `divider`, `collapsed-card`, `capture-backstop`, `gesture-shield`) cross-mapped to the six Phase-0 raw tiles (`capture_backstop`, `frame`, `input_scroll`, `output_scroll`, `drag_shield`, `minimized_icon`) plus the frame-internal divider.
- **ADD** per-part `RenderingPolicy` field consumption, so the promoted portal styles itself through `RenderingPolicy` like every other component.
- **ADD** Text-Portal Readability Enforcement (`OpaqueBackdrop` for text-bearing parts, `None` for geometry-only parts).
- **ADD** Text-Portal Profile Styling and Promotion Scope Boundary, preserving every standing portal non-goal.

## Non-goals

- **No canonical token key changes.** This delta reuses existing canonical token keys verbatim. Portal-specific canonical keys (a `portal.*` namespace, collapsed/expanded transition-duration keys) are deferred to P2 (`hud-8691s`).
- **No excluded-scope additions.** No terminal emulation, no scene-graph transcript history, no chrome-layer portal UI, no dedicated portal transport or second portal stream, no runtime process ownership. Every RFC 0013 §7.2 non-goal stands.
- **No first-class portal-surface / node-type scene-mutation schema.** That is P3 (`hud-tc153`). This change is the styling contract only.
- **No Rust runtime code.** This is a docs/spec-only OpenSpec change.

## Capabilities

### Modified Capabilities

- `component-shape-language`: extend the Component Type Contract to allow promotion-era multi-part portal-surface governance, and add the `text-portal` component type, its part model, its per-part `RenderingPolicy` consumption, its readability enforcement, and its profile-styling scope boundary.

## Impact

- **Specs:** `openspec/specs/component-shape-language/spec.md` (one modified requirement, five added requirements) once this change is archived/synced.
- **Downstream (gated, separate changes):** P2 `hud-8691s` canonicalizes portal token keys against this contract; P3 `hud-tc153` defines the first-class portal-surface / node-type scene-mutation schema this styling contract decorates.
- **Runtime/agent code:** none in this change. Implementation of the contract is gated on the RFC 0013 §7.2 promotion gate passing; until then the same styling is expressible on the raw-tile pilot per RFC 0013 §7.
