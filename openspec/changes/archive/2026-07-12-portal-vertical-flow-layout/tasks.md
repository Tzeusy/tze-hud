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
- [x] 3.4 hud-ysyis (P1, Codex-found on PR #1162, confirmed by reviewer-1149):
      `flow_child_height`'s markdown measurement always used the plain-text
      fallback (`markdown_tokens: None`), and had no measurement mode at all
      for `TextMarkdownNode`s carrying pixel-bearing `color_runs` (e.g. the
      hud-26869 per-turn role-attribution spans) — the render path forks on
      `markdown_node_has_pixel_runs` and paints such a node's RAW,
      un-stripped content via `from_text_markdown_node` (no markdown
      stripping, no token access — stripping/reflow would invalidate the
      color run's pinned byte offsets); the pre-existing measurement always
      took the stripped/token-agnostic path regardless, so an attributed
      transcript turn would measure wrong (line-count divergence,
      overlap/gaps). Fixed by replacing `FlowChild::markdown_tokens:
      Option<&MarkdownTokens>` with a 3-state `FlowContentMode` enum
      (`PlainText` / `Markdown(&MarkdownTokens)` / `RawWithColorRuns`) and a
      new `measure_raw_content_height` in `text.rs` reproducing
      `from_text_markdown_node`'s raw shaping (node's own `font_family`, not
      the hardcoded sans-serif `composer_wrap_line_widths` uses; the DEFAULT
      line-height multiplier, not token-resolved). `flow_child_height` now
      forks on `markdown_node_has_pixel_runs` exactly like the render path,
      and also threads a real `&MarkdownTokens` (new trailing parameter on
      `resolve_tile_flow_offsets`, additive/purely-appended — flagged to the
      coordinator for sequencing with the parallel hud-pd9bp render-site-wiring
      branch) into the common (non-attributed) case's margin AND text-height
      computation, replacing the plain-text fallback. 4 new unit tests
      (filtered, no GPU); verified the wiring-level test fails against a
      simulated pre-fix (always-plain-text, no fork) implementation before
      restoring the real fix.
- [x] 3.3 Render-site geometry substitution (bead hud-pd9bp): per-frame
      `prime_vertical_flow_layout` (renderer field `tile_flow_offsets`, mirrors
      `prime_viewer_echo_layout`; early-out + `bundled_font_system` measurement
      when a `VerticalFlow` node exists) resolves the child→y map; `render_node`
      substitutes `effective_y` at all geometry quads and `collect_text_items_from_node`
      folds `flow_dy = resolved_y − bounds.y` into the `from_text_markdown_*`
      `tile_y` so the constructor nets the resolved y. The ellipsis/truncation
      prime is NOT touched: `TruncationKey` is y-independent (content + box dims +
      font + viewport, no y), so flow placement cannot drift the truncation cache.
      Token gap via `resolve_section_gap_px` (`portal.spacing.section_gap_px`,
      default 8). Behavior-preserving for Absolute (empty map → `bounds.y`).
      Requirement: §Vertical-Flow Render Placement. Unit tests: gap resolver
      (default/override/malformed). PIXEL VERIFICATION of the stacked flow render
      remains hardware-gated (reference Windows, evidence pattern hud-2u5j7) — see
      4.2.

## 4. Follow-ups (separate beads)

- [ ] 4.1 [GATED] Projection per-turn transcript split consuming this engine —
      blocked on the Phase-1 Promotion Evidence Gate (owner/RFC promotion
      approval + refreshed live Windows evidence package). Per-turn attribution
      color runs from #1152 must survive the split.

## 5. Closeout

- [ ] 5.1 Sync + archive per hud-hpuzp convention once the PRs merge.
