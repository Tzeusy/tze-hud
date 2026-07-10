# Tasks — Text-Portal Component Type

This is a docs/spec-only change. No Rust runtime code. Implementation of the contract is gated on the RFC 0013 §7.2 promotion gate passing (`text-stream-portal-phase1` section 7); until then the same styling is expressible on the raw-tile pilot.

## 1. Contract and review

- [x] 1.1 Validate this OpenSpec change with `openspec validate text-portal-component-type --strict`
- [x] 1.2 Review doctrine alignment against CLAUDE.md core rules (never hardcode visuals — use design tokens and `RenderingPolicy`; visual identity is modular) and RFC 0013 §7.2
- [x] 1.3 Confirm no canonical token key is added or changed (portal token canonicalization is P2, `hud-8691s`)
- [x] 1.4 Confirm no excluded-scope item is introduced (no terminal emulation, scene-graph transcript history, chrome portal UI, dedicated transport/second stream, or runtime process ownership)

## 2. Component type contract

- [x] 2.1 MODIFY the `component-shape-language` Component Type Contract to permit promotion-era multi-part portal-surface governance with per-text-bearing-part readability, preserving the six v1 zone-governing types
- [x] 2.2 ADD the `text-portal` component type: name, governed surface, required tokens (existing canonical keys only), per-part readability, informational geometry

## 3. Part model and RenderingPolicy

- [x] 3.1 ADD the Text-Portal Surface Part Model enumerating all eight parts and cross-mapping them to the six Phase-0 tiles plus the frame-internal divider
- [x] 3.2 ADD per-part `RenderingPolicy` consumption (text fields for text-bearing parts; border-token pattern for non-text strokes; transition fields for collapsed↔expanded)
- [x] 3.3 ADD Text-Portal Readability Enforcement (`OpaqueBackdrop` for text-bearing parts, `None` for geometry-only parts)
- [x] 3.4 ADD Text-Portal Profile Styling and Promotion Scope Boundary preserving all standing portal non-goals

## 4. Gated follow-ups (out of scope here)

- [x] 4.1 (P2 `hud-8691s`) Canonicalize portal token keys against this part model — shipped: 59-key `portal.*` namespace in `crates/tze_hud_config/src/portal_tokens.rs`
- [x] 4.2 (P3 `hud-tc153`) Define the first-class portal-surface / node-type scene-mutation schema this styling contract decorates — shipped (#1092)
