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

## 3. Schema field + pre-pass resolver (bead hud-yfj8u — stacked PR)

- [x] 3.1 Per-node layout-mode field: scene `Node.layout: NodeLayout` (additive,
      default `Absolute`; ~360 construction sites swept, `cargo build --all-targets`
      proving completeness) + `NodeProto.layout` wire field (additive, byte-compat)
      + `convert` round-trip (round-trip unit test) + budget (Node budget 150→160,
      scene-graph §Struct Overhead Budgets delta). No new validation rule (any
      layout value valid; whole-subtree validation unaffected).
- [x] 3.2 Compositor pre-pass resolver `resolve_tile_flow_offsets` (measures each
      flow parent's children, stacks from the parent top with the token gap →
      `SceneId → y` map). Behavior-preserving: empty map for all-Absolute scenes.
      Unit-tested (empty-for-absolute + stacks-from-parent-top).
- [ ] 3.3 Render-site geometry substitution: thread the resolved-y map into
      `renderer/tile_render.rs` render_node (~2037) and `renderer/text.rs`
      from_text_markdown_node (~2075) / from_text_markdown_cached (~2200) / the
      ellipsis twin (~1969), substituting the flow-y for `bounds.y`. Deferred to a
      live-hardware evidence bead (hud-yfj8u live-verify scope): the substitution
      is behavior-preserving for the absolute path (empty map), but pixel
      correctness of the stacked flow render requires GPU/reference-Windows
      verification, which cannot run in this lane.

## 4. Follow-ups (separate beads)

- [ ] 4.1 [GATED] Projection per-turn transcript split consuming this engine —
      blocked on the Phase-1 Promotion Evidence Gate (owner/RFC promotion
      approval + refreshed live Windows evidence package). Per-turn attribution
      color runs from #1152 must survive the split.

## 5. Closeout

- [ ] 5.1 Sync + archive per hud-hpuzp convention once the PRs merge.
