# Widget SVG Rasterization Evidence - hud-8qkr0

Date: 2026-05-09

## Context

The Windows baseline from `hud-1753c` measured the Criterion
`widget_rasterize/gauge_512x512` means at:

- cold: 3.679 ms
- warm: 4.775 ms

That misses both the Windows-first <= 1 ms target and the older v1 <= 2 ms
target for a 512x512 widget re-rasterization.

## Focused Change

The compositor raster path previously reparsed and rerasterized every SVG layer
on each widget re-raster, including layers with no parameter bindings. For the
reference gauge, the static background layer has no bindings and is identical
across all parameter updates.

This change adds a small process-local CPU pixmap cache for static no-binding
SVG layers keyed by SVG content and target pixel size. Dynamic bound layers are
still rendered from current parameters on every re-raster.

The runtime upload path also now borrows registered SVG bytes instead of cloning
each layer payload before converting it to UTF-8 text.

## Local Linux Measurements

Command:

```bash
cargo bench -p tze_hud_compositor --bench widget_rasterize -- --sample-size 20 --measurement-time 2
```

Before this change on the worker machine:

- `gauge_512x512/cold`: 2.3241 ms mean
- `gauge_512x512/warm`: 2.8580 ms mean
- `gauge_128x128/warm`: 900.16 us mean

After this change on the same worker machine:

- `gauge_512x512/cold`: 2.4811 ms mean, no material improvement
- `gauge_512x512/warm`: 1.9362 ms mean, roughly 32.3% better than the
  worker-machine baseline
- `gauge_128x128/warm`: 716.27 us mean, roughly 20.4% better than the
  worker-machine baseline

## Windows Validation

The Windows host `tzehouse-windows.parrot-hen.ts.net` was reachable as both
`hudbot` and `tzeus`, but neither account exposed `cargo` in PATH. The host
contains the prior `C:\tze_hud\benchmark_hud1753c.exe` artifact, not a usable
source checkout plus Rust toolchain for rebuilding this branch. I did not run
that old executable because it would only re-measure the previous baseline.

## Interpretation

The static-layer cache is useful but insufficient. It removes repeated work for
unchanged widget chrome/background layers, but the bound gauge fill layer still
pays the dominant cost: SVG string mutation, `usvg` parse, text layout/shaping,
and `resvg`/`tiny-skia` rasterization for a 512x512 target.

The current `usvg::Tree` API exposes renderable nodes read-only, so the obvious
"parse once, mutate attributes in-place" optimization is not available through
public fields. Hitting <= 1 ms likely requires decomposing the widget renderer
into a retained draw plan at registration time, or splitting static text/chrome
from cheap numeric primitives so level/color changes do not reparse and relayout
the full SVG.

## Recommended Follow-ups

1. Add a retained widget render-plan path for the supported v1 SVG primitive
   subset used by shipped widgets: rect, circle, path, text, solid fill/stroke.
   Bindings should update typed primitive fields directly, then rasterize without
   XML parse.
2. Split text-content bindings into their own cached text layer when the text
   parameter has not changed; level/color animation should not relayout glyphs.
3. Change the Criterion warm benchmark to include a parameter-changing case, so
   it measures actual re-rasterization rather than repeated identical parameters
   that the runtime texture cache normally skips.
4. Re-run the gauge benchmark on the Windows reference host after the retained
   draw-plan work lands; the current static-cache fix alone is not expected to
   satisfy the <= 1 ms Windows-first target.
