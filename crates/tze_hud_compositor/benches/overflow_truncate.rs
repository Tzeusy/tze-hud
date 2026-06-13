//! # Overflow / ellipsis truncation benchmark — [hud-alq0x]
//!
//! Measures [`truncate_for_ellipsis`] across three representative input sizes:
//!
//! | Size  | Content                     | Measured p99 (release, warm) |
//! |-------|-----------------------------|-------------------------------|
//! | Small | ~540 B / 5-visible-line box | < 1 ms                        |
//! | Med   | ~8 KiB paragraph            | ~10 ms                        |
//! | Large | ~64 KiB (module ceiling)    | ~94 ms                        |
//!
//! ## Performance contract
//!
//! These are the raw per-call costs of `truncate_for_ellipsis` (uncached).
//! In production the Stage-5 (Layout Resolve) budget of **< 1 ms p99** per
//! RFC 0003 §5.1 / `about/craft-and-care/engineering-bar.md` §2 is met
//! because `TruncationCache` (hud-wgq7j) ensures that `truncate_for_ellipsis`
//! is called **at most once per content/geometry change** (at commit time, off
//! the render path).  The per-frame cost is then a single O(1) cache lookup.
//!
//! The old O(n²) implementation exceeded even these raw-call figures dramatically:
//!   - 540 B / 5-line box: ~10.4 ms (measured release mode)
//!   - 8 KiB paragraph:    ~4,522 ms
//!   - 64 KiB:             did not finish in 12 minutes
//!
//! The current O(n log n) binary-search implementation is measured above.
//! CI uses a lenient ceiling of 500 ms to tolerate debug builds, software
//! rasterisers, and headless VMs.  [hud-v2z6u]
//!
//! ## Benchmark groups
//!
//! - `overflow_truncate/small_540B`   — ~540 B repeated prose, 5-line box
//! - `overflow_truncate/medium_8KiB`  — ~8 KiB paragraph, single-line box
//! - `overflow_truncate/large_64KiB`  — ~64 KiB paragraph, single-line box

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use glyphon::{Attrs, FontSystem};
use tze_hud_compositor::overflow::truncate_for_ellipsis;

// ─── Fixtures ────────────────────────────────────────────────────────────────

/// ~45 bytes: one repetition unit for building multi-KB inputs.
const PROSE_UNIT: &str = "The quick brown fox jumps over the lazy dog. ";

/// Build a prose string of approximately `target_bytes` in length.
fn prose(target_bytes: usize) -> String {
    let repeats = (target_bytes / PROSE_UNIT.len()).max(1);
    PROSE_UNIT.repeat(repeats)
}

// ─── Benchmarks ──────────────────────────────────────────────────────────────

/// ~540 B prose in a 400×(5-line) box — the "transcript widget" scenario.
fn bench_small_540b(c: &mut Criterion) {
    let content = prose(540);
    let font_size = 14.0_f32;
    let line_h = font_size * 1.4;
    let bounds_w = 400.0_f32;
    let bounds_h = line_h * 5.0;

    let mut fs = FontSystem::new();
    let attrs = Attrs::new();

    // Warm up the font system (loads system fonts on first call).
    let _ = truncate_for_ellipsis(
        &content, attrs, bounds_w, bounds_h, font_size, line_h, &mut fs,
    );

    let mut group = c.benchmark_group("overflow_truncate");
    group.bench_function(BenchmarkId::new("small_540B", "warm"), |b| {
        b.iter(|| {
            black_box(truncate_for_ellipsis(
                black_box(&content),
                attrs,
                black_box(bounds_w),
                black_box(bounds_h),
                black_box(font_size),
                black_box(line_h),
                &mut fs,
            ))
        })
    });
    group.finish();
}

/// ~8 KiB paragraph in a narrow 300×(1-line) box — forces deep truncation.
fn bench_medium_8kib(c: &mut Criterion) {
    let content = prose(8 * 1024);
    let font_size = 14.0_f32;
    let line_h = font_size * 1.4;
    let bounds_w = 300.0_f32;
    let bounds_h = line_h + 1.0; // single visible line

    let mut fs = FontSystem::new();
    let attrs = Attrs::new();

    let _ = truncate_for_ellipsis(
        &content, attrs, bounds_w, bounds_h, font_size, line_h, &mut fs,
    );

    let mut group = c.benchmark_group("overflow_truncate");
    group.bench_function(BenchmarkId::new("medium_8KiB", "warm"), |b| {
        b.iter(|| {
            black_box(truncate_for_ellipsis(
                black_box(&content),
                attrs,
                black_box(bounds_w),
                black_box(bounds_h),
                black_box(font_size),
                black_box(line_h),
                &mut fs,
            ))
        })
    });
    group.finish();
}

/// ~64 KiB paragraph in a narrow 300×(1-line) box — the module-stated ceiling.
/// The old implementation did not finish in 12 minutes for this input.
fn bench_large_64kib(c: &mut Criterion) {
    let content = prose(64 * 1024);
    let font_size = 14.0_f32;
    let line_h = font_size * 1.4;
    let bounds_w = 300.0_f32;
    let bounds_h = line_h + 1.0; // single visible line

    let mut fs = FontSystem::new();
    let attrs = Attrs::new();

    let _ = truncate_for_ellipsis(
        &content, attrs, bounds_w, bounds_h, font_size, line_h, &mut fs,
    );

    let mut group = c.benchmark_group("overflow_truncate");
    group.bench_function(BenchmarkId::new("large_64KiB", "warm"), |b| {
        b.iter(|| {
            black_box(truncate_for_ellipsis(
                black_box(&content),
                attrs,
                black_box(bounds_w),
                black_box(bounds_h),
                black_box(font_size),
                black_box(line_h),
                &mut fs,
            ))
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_small_540b,
    bench_medium_8kib,
    bench_large_64kib
);
criterion_main!(benches);
