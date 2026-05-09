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
//! This benchmark measures the retained `WidgetRenderPlan` CPU path without the
//! GPU texture upload step.
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
//! - `widget_rasterize/gauge_512x512_cold_parse` — full two-layer gauge at
//!   512×512, including retained-plan compilation and cold raster caches.
//! - `widget_rasterize/gauge_512x512_warm_identical_params` — retained plan,
//!   identical params, static + bound layer caches warm.
//! - `widget_rasterize/gauge_512x512_warm_parameter_changing` — retained plan,
//!   static/text caches warm, bound numeric/color params changing each iteration
//!   while text content stays stable.
//! - `widget_rasterize/gauge_512x512_warm_text_changing` — retained plan with
//!   non-text primitive segments warm and `text-content` changing each iteration.
//! - `widget_rasterize/gauge_128x128` — same widget at 128×128 for comparison.

use std::collections::HashMap;

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use tze_hud_compositor::widget::{
    WidgetRenderPlan, clear_widget_raster_caches, rasterize_widget_render_plan,
};
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

    group.bench_function(BenchmarkId::new("gauge_512x512", "cold_parse"), |b| {
        b.iter(|| {
            clear_widget_raster_caches();
            let bindings = gauge_fill_bindings();
            let layers: Vec<(&str, &[WidgetBinding])> =
                vec![(GAUGE_BACKGROUND_SVG, &[]), (GAUGE_FILL_SVG, &bindings)];
            let plan = WidgetRenderPlan::compile(&layers);
            black_box(rasterize_widget_render_plan(
                &plan,
                black_box(&constraints),
                black_box(&params),
                black_box(512),
                black_box(512),
            ))
        })
    });

    group.finish();
}

/// Two-layer gauge at 512×512 — warm path with identical params.
fn bench_gauge_512x512_warm_identical_params(c: &mut Criterion) {
    clear_widget_raster_caches();
    let params = gauge_params();
    let constraints = gauge_param_constraints();
    let bindings = gauge_fill_bindings();
    let layers: Vec<(&str, &[WidgetBinding])> =
        vec![(GAUGE_BACKGROUND_SVG, &[]), (GAUGE_FILL_SVG, &bindings)];
    let plan = WidgetRenderPlan::compile(&layers);
    let _ = rasterize_widget_render_plan(&plan, &constraints, &params, 512, 512);

    let mut group = c.benchmark_group("widget_rasterize");

    group.bench_function(
        BenchmarkId::new("gauge_512x512", "warm_identical_params"),
        |b| {
            b.iter(|| {
                black_box(rasterize_widget_render_plan(
                    black_box(&plan),
                    black_box(&constraints),
                    black_box(&params),
                    black_box(512),
                    black_box(512),
                ))
            })
        },
    );

    group.finish();
}

/// Two-layer gauge at 512×512 — retained plan with changing numeric/color params.
fn bench_gauge_512x512_warm_parameter_changing(c: &mut Criterion) {
    clear_widget_raster_caches();
    let constraints = gauge_param_constraints();
    let bindings = gauge_fill_bindings();
    let layers: Vec<(&str, &[WidgetBinding])> =
        vec![(GAUGE_BACKGROUND_SVG, &[]), (GAUGE_FILL_SVG, &bindings)];
    let plan = WidgetRenderPlan::compile(&layers);
    let warm_params = gauge_params();
    let _ = rasterize_widget_render_plan(&plan, &constraints, &warm_params, 512, 512);

    let mut group = c.benchmark_group("widget_rasterize");

    group.bench_function(
        BenchmarkId::new("gauge_512x512", "warm_parameter_changing"),
        |b| {
            let mut params = gauge_params();
            let mut i = 0u32;
            b.iter(|| {
                i = i.wrapping_add(1);
                let level = ((i % 997) as f32) / 996.0;
                *params.get_mut("level").expect("level param exists") =
                    WidgetParameterValue::F32(level);
                *params
                    .get_mut("fill_color")
                    .expect("fill_color param exists") = WidgetParameterValue::Color(
                    tze_hud_scene::types::Rgba::new(level, 0.706, 1.0 - level * 0.5, 1.0),
                );
                black_box(rasterize_widget_render_plan(
                    black_box(&plan),
                    black_box(&constraints),
                    black_box(&params),
                    black_box(512),
                    black_box(512),
                ))
            })
        },
    );

    group.finish();
}

/// Two-layer gauge at 512×512 — retained plan with changing text-content params.
fn bench_gauge_512x512_warm_text_changing(c: &mut Criterion) {
    clear_widget_raster_caches();
    let constraints = gauge_param_constraints();
    let bindings = gauge_fill_bindings();
    let layers: Vec<(&str, &[WidgetBinding])> =
        vec![(GAUGE_BACKGROUND_SVG, &[]), (GAUGE_FILL_SVG, &bindings)];
    let plan = WidgetRenderPlan::compile(&layers);
    let warm_params = gauge_params();
    let _ = rasterize_widget_render_plan(&plan, &constraints, &warm_params, 512, 512);

    let mut group = c.benchmark_group("widget_rasterize");

    group.bench_function(
        BenchmarkId::new("gauge_512x512", "warm_text_changing"),
        |b| {
            let mut params = gauge_params();
            let mut i = 0u32;
            b.iter(|| {
                i = i.wrapping_add(1);
                *params.get_mut("label").expect("label param exists") =
                    WidgetParameterValue::String(format!("load-{:03}", i % 997));
                black_box(rasterize_widget_render_plan(
                    black_box(&plan),
                    black_box(&constraints),
                    black_box(&params),
                    black_box(512),
                    black_box(512),
                ))
            })
        },
    );

    group.finish();
}

/// Two-layer gauge at 128×128 for scale comparison.
fn bench_gauge_128x128(c: &mut Criterion) {
    clear_widget_raster_caches();
    let params = gauge_params();
    let constraints = gauge_param_constraints();
    let bindings = gauge_fill_bindings();
    let layers: Vec<(&str, &[WidgetBinding])> =
        vec![(GAUGE_BACKGROUND_SVG, &[]), (GAUGE_FILL_SVG, &bindings)];
    let plan = WidgetRenderPlan::compile(&layers);
    let _ = rasterize_widget_render_plan(&plan, &constraints, &params, 128, 128);

    let mut group = c.benchmark_group("widget_rasterize");

    group.bench_function(
        BenchmarkId::new("gauge_128x128", "warm_identical_params"),
        |b| {
            b.iter(|| {
                black_box(rasterize_widget_render_plan(
                    black_box(&plan),
                    black_box(&constraints),
                    black_box(&params),
                    black_box(128),
                    black_box(128),
                ))
            })
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_gauge_512x512_cold,
    bench_gauge_512x512_warm_identical_params,
    bench_gauge_512x512_warm_parameter_changing,
    bench_gauge_512x512_warm_text_changing,
    bench_gauge_128x128,
);
criterion_main!(benches);
