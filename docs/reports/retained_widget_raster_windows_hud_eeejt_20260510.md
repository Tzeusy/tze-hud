# Retained Widget Raster Benchmark - hud-eeejt

Date: 2026-05-10 UTC
Issue: `hud-eeejt`
Reference host: `TzeHouse` (`tzehouse-windows.parrot-hen.ts.net`)

## Purpose

Rerun the retained widget raster benchmark from PR #639 on the Windows
reference host with a text-changing label/readout case. The prior retained-plan
evidence separated cold parse, warm identical params, and warm numeric/color
parameter changes locally; this run verifies the Windows path and includes
`text-content` changes so text binding/cache behavior is measured directly.

## Run Shape

The Windows reference host does not expose `cargo` or `rustc` on PATH for the
SSH user. The benchmark executable was cross-built from this worktree and copied
to the host:

```bash
cargo bench -p tze_hud_compositor --bench widget_rasterize \
  --target x86_64-pc-windows-gnu --no-run
scp target/x86_64-pc-windows-gnu/release/deps/widget_rasterize-d946571f82217fad.exe \
  tzeus@tzehouse-windows.parrot-hen.ts.net:C:/tze_hud/perf/hud-eeejt/widget_rasterize.exe
```

The benchmark then ran on TzeHouse:

```cmd
cd /d C:\tze_hud\perf\hud-eeejt
widget_rasterize.exe --bench --sample-size 20 --warm-up-time 1 --measurement-time 5
```

Artifacts:

- `docs/reports/artifacts/hud-eeejt-retained-widget-raster-20260510/widget_rasterize_stdout.txt`
- `docs/reports/artifacts/hud-eeejt-retained-widget-raster-20260510/widget_rasterize_summary.json`

## Results

Criterion slope estimates from `widget_rasterize_summary.json`:

| Benchmark | Lower | Point | Upper | Proposed target |
|---|---:|---:|---:|---:|
| `gauge_512x512_cold_parse` | 4.555 ms | 5.062 ms | 5.338 ms | n/a |
| `gauge_512x512_warm_identical_params` | 0.313 ms | 0.323 ms | 0.332 ms | <= 1.0 ms |
| `gauge_512x512_warm_parameter_changing` | 3.530 ms | 4.021 ms | 4.309 ms | <= 1.0 ms |
| `gauge_512x512_warm_text_changing` | 4.792 ms | 5.726 ms | 6.261 ms | <= 1.0 ms |
| `gauge_128x128_warm_identical_params` | 0.0047 ms | 0.0048 ms | 0.0049 ms | n/a |

## Interpretation

The retained composed-widget cache works for unchanged parameters: the 512x512
warm identical-params case is well below 1 ms on TzeHouse. Actual
re-rasterization remains above the proposed Windows-first 1 ms target on the
reference host.

The text-changing case is now measured and is materially slower than the
numeric/color-changing retained primitive path:

- `warm_parameter_changing`: 4.021 ms point estimate
- `warm_text_changing`: 5.726 ms point estimate

This confirms the review concern from PR #639: numeric/color updates alone do
not characterize label/readout binding cost. Text updates still route through
the text raster path and need separate optimization or a revised budget.

## Follow-Up Resolution

The suggested follow-up became `hud-vzvna` and landed in PR #648. The PR #648
TzeHouse rerun reported:

| Benchmark | TzeHouse point estimate | TzeHouse upper estimate | Proposed target |
|---|---:|---:|---:|
| `warm_parameter_changing` | 0.429 ms | 0.514 ms | <= 1.0 ms |
| `warm_text_changing` | 0.585 ms | 0.788 ms | <= 1.0 ms |

That resolves the changing-path widget raster miss identified by this report.
The full Windows release gate still depends on the later `hud-nfl7n` live soak.

## Follow-Up Input

Suggested follow-up bead for the coordinator:

```json
{
  "title": "Reduce retained widget text-changing raster cost on TzeHouse",
  "type": "task",
  "priority": 1,
  "depends_on": "hud-eeejt",
  "rationale": "Reference-host Criterion run measured gauge_512x512 warm_text_changing at 5.726 ms point / 6.261 ms upper, above the proposed <=1.0 ms Windows-first target. Numeric/color re-raster also remains above target at 4.021 ms point / 4.309 ms upper."
}
```
