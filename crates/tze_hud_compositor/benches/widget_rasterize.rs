//! # Widget SVG re-rasterization benchmark — [hud-4kj0]
//!
//! Measures the CPU-only SVG rasterization path for a 512×512 widget using the
//! reference gauge fixture (background + fill layers, 4 bindings).
//!
//! ## Performance contract
//!
//! Per widget-system/spec.md §Requirement: Widget Compositor Rendering (task 12.11):
//! > Re-rasterization MUST complete in < 2ms for a 512×512 widget on reference hardware.
//!
//! This benchmark measures `rasterize_svg_layers` — the pure CPU path (SVG string
//! manipulation, `usvg` parsing, `resvg`/`tiny-skia` rendering, layer compositing)
//! without the GPU texture upload step.
//!
//! ## Thresholds
//!
//! | Context              | Threshold | Notes                                        |
//! |----------------------|-----------|----------------------------------------------|
//! | Reference hardware   | < 2ms     | Spec requirement (task 7.9, 12.11)           |
//! | CI (llvmpipe/any)    | < 500ms   | Lenient; headroom for debug builds + llvmpipe VMs |
//!
//! The CI assertion test uses the lenient 500ms threshold.  Run this Criterion
//! benchmark on target hardware to verify the 2ms spec requirement.
//!
//! ## Benchmark groups
//!
//! - `widget_rasterize/gauge_512x512_cold` — full two-layer gauge at 512×512,
//!   including SVG string manipulation and usvg parse on each iteration (cold path).
//! - `widget_rasterize/gauge_512x512_warm` — two-layer gauge at 512×512, SVG
//!   strings pre-built but `usvg` parse + rasterize measured (hot path).
//! - `widget_rasterize/gauge_128x128` — same widget at 128×128 for comparison.

use std::collections::HashMap;

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use tze_hud_compositor::widget::rasterize_svg_layers;
use tze_hud_scene::types::{WidgetBinding, WidgetBindingMapping, WidgetParameterValue};

// ─── Reference gauge SVG fixtures ────────────────────────────────────────────

const GAUGE_BACKGROUND_SVG: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 220" width="100" height="220">
  <rect id="frame" x="0" y="0" width="100" height="220" rx="6" ry="6"
        fill="#1a1a2e" stroke="#4a4a6a" stroke-width="2"/>
  <rect id="track" x="30" y="10" width="40" height="200" rx="3" ry="3"
        fill="#2a2a3e" stroke="#3a3a5a" stroke-width="1"/>
</svg>"##;

const GAUGE_FILL_SVG: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 220" width="100" height="220">
  <rect id="bar" x="30" y="10" width="40" height="0"
        fill="#00b4ff" rx="3" ry="3"/>
  <text id="label-text" x="50" y="215" text-anchor="middle"
        font-family="sans-serif" font-size="10" fill="#cccccc"></text>
  <circle id="indicator" cx="85" cy="15" r="6" fill="#00cc66"/>
</svg>"##;

// ─── Bindings matching the gauge fixture widget.toml ─────────────────────────

fn gauge_fill_bindings() -> Vec<WidgetBinding> {
    vec![
        WidgetBinding {
            param: "level".to_string(),
            target_element: "bar".to_string(),
            target_attribute: "height".to_string(),
            mapping: WidgetBindingMapping::Linear {
                attr_min: 0.0,
                attr_max: 200.0,
            },
        },
        WidgetBinding {
            param: "fill_color".to_string(),
            target_element: "bar".to_string(),
            target_attribute: "fill".to_string(),
            mapping: WidgetBindingMapping::Direct,
        },
        WidgetBinding {
            param: "label".to_string(),
            target_element: "label-text".to_string(),
            target_attribute: "text-content".to_string(),
            mapping: WidgetBindingMapping::Direct,
        },
        WidgetBinding {
            param: "severity".to_string(),
            target_element: "indicator".to_string(),
            target_attribute: "fill".to_string(),
            mapping: WidgetBindingMapping::Discrete {
                value_map: [
                    ("info".to_string(), "#00cc66".to_string()),
                    ("warning".to_string(), "#ffcc00".to_string()),
                    ("error".to_string(), "#ff3300".to_string()),
                ]
                .into_iter()
                .collect(),
            },
        },
    ]
}

fn gauge_params() -> HashMap<String, WidgetParameterValue> {
    [
        ("level".to_string(), WidgetParameterValue::F32(0.75)),
        (
            "fill_color".to_string(),
            WidgetParameterValue::Color(tze_hud_scene::types::Rgba::new(0.0, 0.706, 1.0, 1.0)),
        ),
        (
            "label".to_string(),
            WidgetParameterValue::String("75%".to_string()),
        ),
        (
            "severity".to_string(),
            WidgetParameterValue::Enum("info".to_string()),
        ),
    ]
    .into_iter()
    .collect()
}

fn gauge_param_constraints() -> HashMap<String, (f32, f32)> {
    [("level".to_string(), (0.0f32, 1.0f32))]
        .into_iter()
        .collect()
}

// ─── Benchmarks ───────────────────────────────────────────────────────────────

/// Two-layer gauge at 512×512 — cold path (bindings and SVG text built each iter).
fn bench_gauge_512x512_cold(c: &mut Criterion) {
    let params = gauge_params();
    let constraints = gauge_param_constraints();
    let mut group = c.benchmark_group("widget_rasterize");

    group.bench_function(BenchmarkId::new("gauge_512x512", "cold"), |b| {
        b.iter(|| {
            let bindings = gauge_fill_bindings();
            let layers: Vec<(&str, &[WidgetBinding])> =
                vec![(GAUGE_BACKGROUND_SVG, &[]), (GAUGE_FILL_SVG, &bindings)];
            black_box(rasterize_svg_layers(
                &layers,
                black_box(&constraints),
                black_box(&params),
                black_box(512),
                black_box(512),
            ))
        })
    });

    group.finish();
}

/// Two-layer gauge at 512×512 — warm path (bindings pre-built, only SVG parse + rasterize).
fn bench_gauge_512x512_warm(c: &mut Criterion) {
    let params = gauge_params();
    let constraints = gauge_param_constraints();
    let bindings = gauge_fill_bindings();
    let layers: Vec<(&str, &[WidgetBinding])> =
        vec![(GAUGE_BACKGROUND_SVG, &[]), (GAUGE_FILL_SVG, &bindings)];

    let mut group = c.benchmark_group("widget_rasterize");

    group.bench_function(BenchmarkId::new("gauge_512x512", "warm"), |b| {
        b.iter(|| {
            black_box(rasterize_svg_layers(
                black_box(&layers),
                black_box(&constraints),
                black_box(&params),
                black_box(512),
                black_box(512),
            ))
        })
    });

    group.finish();
}

/// Two-layer gauge at 128×128 for scale comparison.
fn bench_gauge_128x128(c: &mut Criterion) {
    let params = gauge_params();
    let constraints = gauge_param_constraints();
    let bindings = gauge_fill_bindings();
    let layers: Vec<(&str, &[WidgetBinding])> =
        vec![(GAUGE_BACKGROUND_SVG, &[]), (GAUGE_FILL_SVG, &bindings)];

    let mut group = c.benchmark_group("widget_rasterize");

    group.bench_function(BenchmarkId::new("gauge_128x128", "warm"), |b| {
        b.iter(|| {
            black_box(rasterize_svg_layers(
                black_box(&layers),
                black_box(&constraints),
                black_box(&params),
                black_box(128),
                black_box(128),
            ))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_gauge_512x512_cold,
    bench_gauge_512x512_warm,
    bench_gauge_128x128,
);
criterion_main!(benches);
