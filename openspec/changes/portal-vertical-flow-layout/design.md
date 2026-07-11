# Design: portal-vertical-flow-layout

## Context

Verified (hud-26869, hud-txkbh investigations) that the compositor consumes each
node's `bounds.y` directly at draw time — no layout/resolve pre-pass — via twin
tree walks (`renderer/tile_render.rs::render_node`, `renderer/text.rs::collect_*`),
each computing `y = tile_y + node.bounds.y + margin`. There is no per-node layout
mode on the scene `Node` or `NodeProto`. A pure-CPU wrapped-line shaper already
exists (`text::composer_wrap_line_widths`), used by the viewer-echo line-count
pre-pass; height = `line_count * (font_size_px * LINE_HEIGHT_MULTIPLIER) +
vertical_padding`. The inter-section gap token `portal.spacing.section_gap_px`
(`PortalPartTokens::section_gap_px`) already exists.

## Goals / Non-Goals

**Goals**: a general, reusable, fully-testable-without-GPU resolution engine that
measures child heights and stacks them with a token gap; a spec contract for the
capability; provable backward-compatibility (default-off).

**Non-Goals**: the per-node layout-mode schema field, the render-site wiring, and
the projection per-turn transcript split (promotion-gated) — all follow-ups.

## Decisions

1. **Resolution engine now; schema + wiring + consumer later.** Adding a
   `layout` field to the scene `Node` struct is a ~300-site mechanical sweep
   (`Node` has no `Default`), and the field is inert until the promotion-gated
   per-turn split exists. Landing the engine first keeps every delivered piece
   verifiable and lets the schema/wiring land with their actual consumer.

2. **Reuse the render path's shaper for measurement.** `measure_child_height`
   calls `composer_wrap_line_widths` — the same shaper the compositor uses — so a
   measured row height equals the painted row height (no drift between the
   truncation/measurement path and the flow layout).

3. **Pure geometry core, separately tested.** `stack_offsets` /
   `flow_total_height` take `&[f32]` heights and a gap — no fonts — so the
   stacking math is unit-tested in isolation (empty, single, multi, clamps),
   independent of font availability, mirroring the existing
   `viewer_echo_divider_rects` pure-rects precedent.

4. **Gap is caller-supplied.** `resolve_vertical_flow` takes `gap: f32`; the
   caller passes `PortalPartTokens::section_gap_px`. The engine never invents
   spacing, satisfying the no-hardcoded-visuals rule.

5. **Defensive clamps.** Negative gap → 0; negative height → 0 contribution. A
   malformed input can never make the stack run backwards or overlap.

## Verification without GPU

The engine is entirely CPU: the pure core needs no fonts; the measurement bridge
uses `FontSystem::new()` (as existing `text.rs` measurement tests do). Ten unit
tests run under a filtered `cargo test -p tze_hud_compositor --lib vertical_flow::`
— the broad GPU suite is never invoked. Because production emits no flow nodes
yet, there is no render-path behavior change to GPU-verify in this PR; the
capability's correctness (measurement + stacking) is fully proven by the unit
tests.

## Risks / Trade-offs

- **[Inert until wired]** The engine has no production caller yet. → Accepted: it
  is the named prerequisite for the gated split, delivered so the split becomes a
  small follow-up once promotion is approved; the spec records the gate.
- **[Measurement drift]** If the render path's shaping diverges from
  `composer_wrap_line_widths`, measured heights would mismatch. → Mitigated by
  reusing the exact shared shaper; a divergence would already break the
  viewer-echo line-count pre-pass that depends on the same helper.

## Migration Plan

Spec-only delta + additive compositor module. Validate `--strict`, land with the
hud-txkbh engine, then sync + archive per the hud-hpuzp convention. Follow-ups:
the `Node`/`NodeProto` layout-mode field + render-site wiring, then the
promotion-gated per-turn transcript split consuming this engine.

## Open Questions

- Whether the eventual layout-mode marker lives on the scene `Node`, on
  `NodeProto` as an additive field, or is driven by the `PortalSurface` overlay
  (tile-keyed, avoids the `Node` sweep) — decide with the render-wiring follow-up.
