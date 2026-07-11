# Tasks: portal-vertical-flow-layout

## 1. Spec delta (this change's deliverable)

- [x] 1.1 Author delta: Vertical-Flow Transcript Layout requirement
- [x] 1.2 `openspec validate portal-vertical-flow-layout --strict` passes
- [ ] 1.3 Commit + push (bead hud-txkbh branch → PR)

## 2. Implementation (bead hud-txkbh — this PR)

- [x] 2.1 `crates/tze_hud_compositor/src/vertical_flow.rs`: pure geometry core
      (`stack_offsets` / `flow_total_height`).
- [x] 2.2 Measurement bridge (`measure_child_height` / `resolve_vertical_flow`)
      reusing `text::composer_wrap_line_widths`; token gap supplied by caller.
- [x] 2.3 Register `pub mod vertical_flow` in `lib.rs`.
- [x] 2.4 Unit tests (pure math + CPU font measurement + full-resolve
      demonstration); run filtered, never the broad GPU suite.

## 3. Follow-ups (out of scope here — separate beads)

- [ ] 3.1 Per-node layout-mode schema field on scene `Node` (+ `Default`) and
      `NodeProto` (additive, default-off), convert round-trip, validation, budget.
- [ ] 3.2 Render-site wiring: resolve flow offsets once per flow parent (pre-pass
      keyed `SceneId → y`, precedent: viewer-echo prime) and substitute the
      resolved flow-y at the geometry sites (`renderer/text.rs` from_text_markdown_*,
      `renderer/tile_render.rs` render_node, and the ellipsis twin). GPU-verify.
- [ ] 3.3 [GATED] Projection per-turn transcript split consuming this engine —
      blocked on the Phase-1 Promotion Evidence Gate (owner/RFC promotion
      approval + refreshed live Windows evidence package). Per-turn attribution
      color runs from #1152 must survive the split.

## 4. Closeout

- [ ] 4.1 Sync + archive per hud-hpuzp convention once the PR merges.
