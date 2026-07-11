# Tasks: portal-vertical-flow-layout

## 1. Spec delta (this change's deliverable)

- [x] 1.1 Author delta: Vertical-Flow Transcript Layout requirement
- [x] 1.2 `openspec validate portal-vertical-flow-layout --strict` passes
- [ ] 1.3 Commit + push (bead hud-txkbh branch → PR)

## 2. Implementation (bead hud-txkbh — this PR)

- [x] 2.1 `crates/tze_hud_compositor/src/vertical_flow.rs`: pure geometry core
      (`stack_offsets` / `flow_total_height`).
- [x] 2.2 Measurement bridge (`measure_child_height` / `resolve_vertical_flow`)
      reusing `text::composer_wrap_line_widths` for plain-text children; token
      gap supplied by caller.
- [x] 2.3 Register `pub mod vertical_flow` in `lib.rs`.
- [x] 2.4 Unit tests (pure math + CPU font measurement + full-resolve
      demonstration); run filtered, never the broad GPU suite.
- [x] 2.5 hud-3xdlf (P1, Codex-found on PR #1161, confirmed by reviewer-1149):
      `measure_child_height` originally measured markdown `TextMarkdownNode`
      content the same way as plain text (raw source, uniform font size, the
      `LINE_HEIGHT_MULTIPLIER` constant) — three real divergences from what
      `TextItem::from_text_markdown_cached` actually paints (stripped-vs-raw
      text, heading `size_scale`, token-resolved line-height). Fixed by adding
      `FlowChild::markdown_tokens: Option<&MarkdownTokens>` and a markdown
      branch that reproduces the render path's parse+shape via a shared helper
      (`markdown_spans_to_styled_runs`, factored out of
      `from_text_markdown_cached` so both build the identical run set) and
      `measure_markdown_content_height`, reading total height back from
      glyphon's actual per-line layout rather than a `line_count * constant`
      product. 7 new markdown-focused unit tests (filtered, no GPU); verified
      each precision assertion actually fails against a simulated pre-fix
      implementation before restoring the real fix.

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
