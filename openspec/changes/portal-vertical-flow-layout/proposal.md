# Proposal: portal-vertical-flow-layout

## Why

The conversational turn model (hud-26869 / portal-turn-model-attribution) shipped
per-turn *attribution* within the single-blob transcript node but deferred the
structural *per-turn node split* — one `TextMarkdownNode` per turn — with a
documented prerequisite: the compositor plots every node at its own explicit
`bounds.y` with **no vertical flow/stack layout**, and the projection layer has
no font metrics to measure wrapped turn heights, so per-turn child nodes would
all paint at `y = 0` and overlap. PR #1149 (hud-ga4md) shipped atomic
`NodeProto.children` subtree *materialization*, but not per-turn *layout*.

This change adds the missing capability: a runtime-side **vertical-flow layout
resolution engine** that measures each child's wrapped rendered height (reusing
the compositor's existing CPU wrapped-line shaper, so measured height equals
painted height) and stacks children with a token-driven inter-child gap. It runs
in the runtime/compositor — never in the model — so a publisher that cannot
measure text (the projection layer) can emit a stacked transcript body and have
the runtime resolve positions.

The engine is the enabler; the per-turn transcript **node split** that would
drive it in production is promotion-era work and remains gated on the
§Phase-1 Promotion Evidence Gate (a refreshed live Windows evidence package that
has not been produced). Until that gate passes, the raw-tile pilot's single-node
transcript stays authoritative, and this capability is exercised as the runtime
resolution engine a future split will consume.

## What Changes

- **Vertical-flow layout resolution**: the runtime SHALL be able to lay out a
  stacked sequence of child nodes by measuring each child's wrapped rendered
  height and positioning each subsequent child directly below the previous one
  plus an inter-child gap.
- **Runtime-resolved, model-out-of-loop**: layout resolution runs in the
  runtime/compositor; the owning model/adapter never computes child positions.
- **Token-driven spacing**: the inter-child gap is sourced from a design token
  (`portal.spacing.section_gap_px`), never a hardcoded compositor value.
- **Additive, default-off**: a node whose children are not marked for flow keeps
  being positioned by each child's explicit bounds exactly as before — the
  single-node transcript path is byte-identical.
- **Split stays gated**: materializing the OUTPUT transcript as one scene node
  per turn on top of this layout remains gated on the §Phase-1 Promotion
  Evidence Gate.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `text-stream-portals`: ADDS a "Vertical-Flow Transcript Layout" requirement —
  the runtime resolution engine that a promotion-era per-turn transcript node
  split will consume — consistent with §Phase-0 Raw-Tile Pilot (existing node
  types), §Portal Component Profile Styling (token-driven), §Promotion Scope
  Boundary, and §Phase-1 Promotion Evidence Gate (the consumer stays gated).

## Impact

- **Spec**: delta on `text-stream-portals`.
- **Code** (bead hud-txkbh, this PR): `crates/tze_hud_compositor/src/vertical_flow.rs`
  — the pure geometry core (`stack_offsets` / `flow_total_height`) + measurement
  bridge (`measure_child_height` / `resolve_vertical_flow`), reusing
  `crate::text::composer_wrap_line_widths`.
- **Deferred to follow-ups** (out of scope here): the per-node layout-mode schema
  field on the scene `Node` (~300 construction sites) and `NodeProto`, the
  render-site wiring that substitutes the resolved flow-y at draw time, and the
  projection per-turn transcript split (promotion-gated).
- **Non-goals**: per-turn scene nodes in production (gated); alignment/bubble
  styling; terminal emulation; scene-graph transcript history.
