# hud-ko552 — steady-state re-measurement after hud-u4lq2 fix (PR #1148)

- Date: 2026-07-11
- Bead: `hud-ko552` (per-tile damage tracking), blocked by coordinator `[decision]` on `hud-u4lq2` (MarkdownPrimer stale-clobber cache bug), pending this re-measure
- Base commit: `6213c7e3` (origin/main, includes `#1148` — the `hud-u4lq2` primer-internal-epoch fix)
- Host: sandbox with NVIDIA GeForce RTX 4080 (driver 570.148.08) — real GPU, not llvmpipe
- Harness: `crates/tze_hud_compositor/src/renderer/tests.rs::scroll_reshape_bench_hud991cj` (existing, unmodified), run headless (`render_frame_headless`, no pixel readback)
- GPU-gated: confirmed **actually ran** (not silently no-op'd by `require_gpu!`) — non-empty `eprintln!` output with real timings and passing assertions below

## Background

`hud-ko552` covers per-tile damage tracking (composer keystrokes / isolated
part changes reprocess the whole portal every frame — the frame pipeline has
no dirty-region concept). The coordinator's `[decision]` deliberately blocked
implementation on `hud-u4lq2` first: rev-5hfn4's original measurement
(instrumenting this same `scroll_reshape_bench_hud991cj` test) traced the
entire **5-15ms/frame steady-state `collect_text_items` cost** to a
`MarkdownPrimer` stale-clobber bug — every frame took the
"documented-as-rare" synchronous inline-reparse fallback for the 64KiB
transcript scenario because the primer's background-parsed snapshot never
landed. That bug is a confounder: it would have made *any* frame-path work
(damage-tracked or not) pay a full markdown re-parse per frame, so measuring
against it would justify damage tracking for the wrong reason.

`hud-u4lq2` is now fixed and merged (PR #1148, primer-internal epoch instead
of trusting the caller's scene-relative version). This report re-runs the
prescribed re-measure per the decision note.

## Budgets (per `about/craft-and-care/engineering-bar.md` §2)

| Budget | Value |
|---|---|
| Total frame time (p99) | < 16.6ms |
| Stage 6: Render Encode (p99) | < 4ms — `collect_text_items` runs inside this stage |

## Measurement 1 — full-frame bench (`scroll_reshape_bench_hud991cj`, unmodified)

Two consecutive runs (sandbox has scheduling variance; both are well inside budget):

```
cargo test -p tze_hud_compositor --lib scroll_reshape_bench_hud991cj -- --nocapture
```

Run A:
```
hud-991cj bench [small] bytes=540   | rest: 0 reshapes 3.845 ms/frame | scroll: 0 reshapes 1.964 ms/frame | append: 1 reshape
hud-991cj bench [64KiB] bytes=64800 | rest: 0 reshapes 3.020 ms/frame | scroll: 0 reshapes 1.758 ms/frame | append: 1 reshape
```

Run B:
```
hud-991cj bench [small] bytes=540   | rest: 0 reshapes 0.919 ms/frame | scroll: 0 reshapes 0.636 ms/frame | append: 1 reshape
hud-991cj bench [64KiB] bytes=64800 | rest: 0 reshapes 0.894 ms/frame | scroll: 0 reshapes 0.900 ms/frame | append: 1 reshape
```

Both runs: all 6 assertions pass (0 re-shapes on REST/SCROLL, exactly 1 on
APPEND, for both content sizes). This is the **whole `render_frame_headless`
pipeline** (layout resolve + render encode + headless GPU submit), i.e. an
upper bound that includes far more than `collect_text_items` alone — and it
already sits at 0.6-3.8ms/frame, comfortably under both the 4ms Stage-6 and
16.6ms total-frame budgets, for the exact 64KiB scenario that previously
showed 5-15ms/frame.

## Measurement 2 — isolated `collect_text_items` cost

To isolate the specific cost the original bead/decision attributed the
5-15ms/frame to (`collect_text_items_from_node`'s markdown-cache-miss
fallback), a temporary local bench was added to
`renderer::tests` calling `Compositor::collect_text_items` directly (bypassing
layout/encode/GPU submit entirely), covering REST, SCROLL, and a
**post-append** case (one single-part content change, then re-measured after
resettling — the composer-keystroke-style single-part-change shape the bead
describes). It was run locally, the numbers below were captured, and the
instrumentation was then **reverted** (`git checkout --`) so this stays a
docs-only change — no test/prod code was added or modified on this branch.

```
hud-ko552 ISOLATED collect_text_items bench [small] bytes=540   | rest: 0.0054 ms/call | scroll: 0.0058 ms/call | post-append-settled: 0.0052 ms/call | markdown_cache_misses_total=0
hud-ko552 ISOLATED collect_text_items bench [64KiB] bytes=64800 | rest: 0.0049 ms/call | scroll: 0.0053 ms/call | post-append-settled: 0.0053 ms/call | markdown_cache_misses_total=2
```

`collect_text_items` alone costs **~0.005ms/call (~5 microseconds)** for both
the small and 64KiB transcript, REST and SCROLL alike — roughly **1000-3000x**
below the previously measured 5-15ms/frame, and the *total* markdown-cache
miss count across the entire bench (spanning two scenarios' prime+append
cycles, ~210 calls) is only 2 — both from legitimate one-time re-primes after
content changed, not per-frame misses. This directly confirms rev-5hfn4's
diagnosis: the 5-15ms/frame cost was **entirely** the `hud-u4lq2` cache bug,
not an inherent cost of walking the whole portal every frame.

### Composer-keystroke framing

`collect_text_items` has no damage-tracking today — it always walks every
visible tile/part in the scene regardless of which single part changed, so a
composer keystroke's cost profile is identical to the measured REST case
(there is no cheaper path to compare against). That cost is now ~5µs
isolated / 0.6-3.8ms full-pipeline for a 64KiB transcript — i.e. walking the
*entire* portal on every keystroke is not observable against either budget.

## Verdict

**No whole-portal-reprocess cost survives that justifies per-tile damage
tracking.** The 5-15ms/frame steady-state cost that originally motivated
`hud-ko552` was fully attributable to the `hud-u4lq2` MarkdownPrimer cache bug
(now fixed). With that fixed, the actual cost of reprocessing the whole
portal every frame — the thing damage tracking would eliminate — is ~5µs
isolated (`collect_text_items`) and 0.6-3.8ms end-to-end, roughly 3 orders of
magnitude under the 4ms Stage-6 budget and comfortably under the 16.6ms
total-frame budget, for the largest transcript size the bench models
(64KiB, just under `MAX_MARKDOWN_BYTES`).

Recommendation: **close `hud-ko552` with this evidence**, rather than
implementing per-tile damage tracking now. The decision note's risk framing
(L effort, 57 scene-graph mutation sites needing instrumentation in a
foundational crate, real risk of a missed site causing silent stale
rendering) is not offset by any measured benefit — there is no surviving cost
to optimize away. If a future workload profile changes this picture (e.g.
substantially larger transcripts, many concurrently-visible portal tiles, or
a lower-power target device where even microsecond-scale per-tile walks
matter), the per-tile-tracking investment question should be re-opened with
fresh data at that time — this report does not attempt to predict that.

## Reproduction

```bash
cd crates/tze_hud_compositor  # or run from repo root with -p
cargo test -p tze_hud_compositor --lib scroll_reshape_bench_hud991cj -- --nocapture
```

The isolated `collect_text_items`-only bench used for Measurement 2 was not
committed (kept this branch docs-only); to reproduce, add a test in
`renderer::tests` that calls `compositor.collect_text_items(&scene, w, h)`
directly in a timed loop instead of `render_frame_headless`, following the
same scene setup as `scroll_reshape_bench_hud991cj` (both are `pub(super)`
and `renderer::tests` is a child module of `renderer`, so both are directly
callable from a test in that file).
