# hud-6fmax — styled_runs per-frame allocation measurement

- Date: 2026-07-11
- Bead: `hud-6fmax` (`collect_composer_text_item` / `collect_text_items` construct a
  fresh `Box<[StyledRunItem]>` every frame even when content is unchanged; the
  hud-991cj shape_cache only dedupes the expensive `shape_until_scroll` call, not
  this metadata allocation), child of `hud-wse80`, discovered from the
  `hud-0yrix` audit (`perf-styled-runs-vec-per-frame [S3]`)
- Base commit: `6213c7e3` (origin/main, includes `#1148` — the `hud-u4lq2`
  MarkdownPrimer internal-epoch fix)
- Host: sandbox with NVIDIA GeForce RTX 4080 (real GPU, not llvmpipe)
- Approach: craft-and-care performance-bead protocol (`about/craft-and-care/engineering-bar.md`
  §4.10) — measure first, empirically, in release mode, against the exact code
  paths named in the bead, before deciding fix vs. no-fix

## Background

Two call sites were named in the bead:

1. `Compositor::collect_composer_text_item` (`renderer/tile_render.rs:1315`) —
   builds a `Box<[StyledRunItem]>` for the composer draft's selection
   highlight run.
2. `TextItem::from_text_markdown_cached` (`src/text.rs:2157`, invoked from
   `collect_text_items_from_node` in `renderer/text.rs:1306` on **every**
   `TextMarkdown` node, every frame, once the markdown cache is warm) — builds
   a `Box<[StyledRunItem]>` from `ParsedMarkdown::spans` on every call, even
   though `parsed.spans` itself is immutable, cached, per-content data (only
   `plain_text: Arc<str>` is Arc-shared today; `spans: Vec<StyledSpan>` is
   walked, filtered, and color-converted into a brand-new `Vec` → boxed slice
   on every single frame for every visible markdown node).

**(1) is not the real cost.** `collect_composer_text_item`'s styled_runs is
`Box::new([])` (no selection) or a 1-element box (active selection). An empty
`Box<[T]>`/`Vec<T>` never calls the global allocator in Rust (zero-size
allocations use a dangling pointer per the `Vec`/`Box` internals) — so the
common no-selection frame allocates **zero bytes** here already, and the
1-element case during an active text selection is negligible.

**(2) is the real candidate.** It runs unconditionally on the steady-state hot
path (comment in `text.rs:2179` even calls it out: "This is the hot per-frame
path"), and its cost scales with `parsed.spans.len()` — i.e. with markdown
styling density (bold/italic/code spans), not just byte count.

### Why the existing hud-ko552 bench doesn't cover this

`crates/tze_hud_compositor/src/renderer/tests.rs::scroll_reshape_bench_hud991cj`
(the harness `docs/evidence/text-stream-portals/hud-ko552-remeasure-post-u4lq2-20260711.md`,
PR #1150, used to re-measure the whole `collect_text_items` path at ~5µs/call)
uses **plain, unstyled** transcript content ("The quick brown fox...", no
markdown emphasis, `color_runs: Box::default()`). For that content,
`ParsedMarkdown::spans` is empty, so the `styled_runs` boxed-slice build is a
zero-allocation no-op — that bench cannot see the cost this bead is about.
This measurement adds markdown-styled content to isolate it.

## Budgets (per `about/craft-and-care/engineering-bar.md` §2)

| Budget | Value |
|---|---|
| Total frame time (p99) | < 16.6ms |
| Stage 6: Render Encode (p99) | < 4ms — `collect_text_items` runs inside this stage |

## Methodology

A temporary (not committed — reverted after capturing numbers, mirroring the
hud-ko552 Measurement-2 methodology) bench was added to `renderer::tests`,
calling `Compositor::collect_text_items` directly (bypassing layout/encode/GPU
submit — isolates exactly the collection path named in the bead). Three
byte-identical (81-byte line, repeated) content variants were compared at two
sizes:

- **bold**: every word individually wrapped in `**...**` — worst-case span
  density (1 `StyledSpan` per word).
- **sparse**: one bolded word per line — a more realistic "emphasize a key
  term" chat/doc-markdown density (~11% of words).
- **plain**: same byte length, no markdown emphasis — zero spans, the control.

Content sizes: **small** (12 lines, ~972 bytes) and **64KiB-ish** (800 lines,
64800 bytes, just under the 65535-byte `TextMarkdownNode` content cap).

For each variant/size, the scene was primed (`prime_markdown_cache`), then the
test **polled `MarkdownCache::get_by_key` until the entry landed** before
timing. This step is necessary and was not optional: payloads over
`INLINE_PARSE_BYTE_THRESHOLD` (4096 bytes, `markdown.rs:782`) parse
asynchronously on the `MarkdownPrimer`'s background thread. Timing
immediately after `prime_markdown_cache` — without waiting for the snapshot to
land — measures the one-time **liveness-fallback inline re-parse** (the
per-frame cache-miss branch that covers the race window), not the intended
steady-state cached path; an early draft of this bench made exactly that
mistake and reported spans=0 / inflated timings for the 64KiB case before the
poll was added.

200 iterations of `collect_text_items` were timed per variant/size,
`--release`, with a real GPU adapter (`require_gpu!` confirmed non-skip).

## Results

| Scenario | Bytes | Spans | `collect_text_items` |
|---|---|---|---|
| small / bold (every word) | 972 | 108 | 2.2 µs/call |
| small / sparse (1 word/line) | 972 | 12 | 0.6 µs/call |
| small / plain | 972 | 0 | 0.3 µs/call |
| 64KiB-ish / bold (every word) | 64,800 | 7,200 | 95.0 µs/call |
| 64KiB-ish / sparse (1 word/line) | 64,800 | 800 | 11.8 µs/call |
| 64KiB-ish / plain | 64,800 | 0 | 0.3 µs/call |

The plain baseline (0 spans) is flat at ~0.3µs/call regardless of content size
— confirming `plain_text: Arc<str>` sharing works as documented (no per-byte
cost from the Arc clone) and that the *rest* of `collect_text_items` (tile
iteration, opacity/scroll bookkeeping, `Vec<TextItem>` push) is not the
concern here. The delta between bold/sparse and plain is attributable to the
`styled_runs` rebuild, and scales linearly with span count: ~13–15ns/span
consistently across both content sizes (108 spans → 1.9µs delta ≈ 17ns/span;
7200 spans → 94.7µs delta ≈ 13ns/span; 800 spans → 11.5µs delta ≈ 14ns/span).

## Verdict

**Real, measurable, linearly-scaling cost — but comfortably under budget for
both realistic and pathological content, so no fix is warranted at this
priority.**

- Realistic markdown density (**sparse**, ~11% of words styled — a plausible
  upper bound for chat/doc-style "bold a key term" usage) costs **11.8µs** for
  a full 64KiB transcript: **0.3%** of the 4ms Stage-6 budget, **0.07%** of the
  16.6ms total-frame budget. This is noise.
- Even the deliberately pathological worst case (**bold**, literally every
  word individually emphasized — not a realistic transcript) costs **95µs**:
  **2.4%** of the Stage-6 budget, **0.57%** of the total-frame budget. Still
  comfortably inside both budgets, for one visible transcript node; a portal
  typically has one transcript tile plus a composer, not many concurrent
  heavily-styled nodes.
- Unlike hud-ko552 (where the *entire* prior 5-15ms/frame cost turned out to
  be attributable to a since-fixed cache bug, leaving ~zero residual cost),
  this cost is real and does scale with content — it is not "noise" in the
  literal sense. But it does not approach either performance budget for any
  content shape this product plausibly renders, so implementing the
  Arc-sharing fix described in the bead (mirroring `plain_text: Arc<str>` —
  precompute `styled_runs` once in `ParsedMarkdown` at parse time as
  `Arc<[StyledRunItem]>` and change `TextItem::styled_runs` from
  `Box<[StyledRunItem]>` to `Arc<[StyledRunItem]>`) is not justified right
  now. That refactor touches the `TextItem` struct definition, several
  `styled_runs = ....into_boxed_slice()` mutation sites (`text.rs`'s reveal-fade
  and cursor-accent rewrites, which have to stay owned/mutable — they respond
  to per-frame cursor/reveal state, not just cached parse output), the
  composer's own `Box::new([...])` construction sites in `tile_render.rs`, and
  every call site per the engineering-bar §4 review standard #9 (production
  call-site coverage for shared-symbol changes) — nontrivial surface for a
  cost that is 40-170x under budget even in the worst case measured here.

**Recommendation: close `hud-6fmax` as low-priority/no-fix with this
evidence**, consistent with the bead's own framing ("Low-severity... or
measuring first"). If a future feature drives much higher styled-span density
per node (e.g. heavy syntax-highlighted code blocks, diff-style per-character
styling, or many concurrently visible heavily-formatted tiles), re-open with
fresh numbers against that shape — this report does not attempt to predict
that.

## Reproduction

The isolated bench was not committed (kept this change docs-only, mirroring
hud-ko552's Measurement 2). To reproduce, add a test to
`crates/tze_hud_compositor/src/renderer/tests.rs` (a child module of
`renderer`, so `Compositor::collect_text_items` — `pub(super)` — is directly
callable) that:

1. Builds three `TextMarkdownNode` scenes with byte-identical repeated lines:
   one with every word wrapped in `**...**`, one with a single bolded word per
   line, one plain (same total bytes).
2. Registers a vertical `TileScrollConfig` on the tile (portal-scope markdown
   tokens) and calls `compositor.prime_markdown_cache(&scene)`.
3. **Polls** `compositor.markdown_cache().get_by_key(&key)` (with
   `MarkdownCache::compute_key(&content, &compositor.markdown_tokens)`) until
   `Some`, sleeping briefly between attempts — content over 4096 bytes
   (`INLINE_PARSE_BYTE_THRESHOLD`) parses on the `MarkdownPrimer` background
   thread and the cache entry is not necessarily present immediately after
   `prime_markdown_cache` returns.
4. Times `compositor.collect_text_items(&scene, w, h)` in a loop (200
   iterations was enough for a stable reading in `--release`).

Run with a real GPU adapter available:

```bash
cargo test -p tze_hud_compositor --release --lib <bench_test_name> -- --nocapture
```
