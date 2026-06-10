//! # Markdown cache prime benchmark — [hud-gpqde]
//!
//! Verifies the two key contract properties from task 2.5 of the Text Stream
//! Portal Phase-1 spec:
//!
//! 1. **Zero per-frame parse cost** — a cache hit (`MarkdownCache::get_by_key`
//!    on already-primed content) completes in O(32-byte map lookup) time.
//!    CI asserts < 10 µs per lookup.
//!
//! 2. **Stage-budget compliance on commit** — parsing a 65535-byte markdown
//!    payload (the module-stated content ceiling) via `MarkdownCache::prime`
//!    completes within the Stage 3/4 combined budget of **< 1 ms p99** per
//!    RFC 0003 §5.1 and `about/craft-and-care/engineering-bar.md` §2.
//!    CI uses a lenient ceiling of 500 ms to tolerate debug builds, software
//!    rasterisers, and headless VMs.
//!
//! ## Benchmark groups
//!
//! - `markdown_cache/cache_hit`          — `get_by_key` on warm cached content
//! - `markdown_cache/parse_commit_64kB`  — `prime` for 65535-byte new content
//! - `markdown_cache/compute_key`        — BLAKE3 hash of 65535 bytes (baseline)

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use tze_hud_compositor::{MarkdownCache, MarkdownTokens};

// ─── Fixtures ────────────────────────────────────────────────────────────────

/// ~45 bytes: one repetition unit for building markdown-heavy inputs.
const MD_UNIT: &str = "**Bold text** and *italic* with `code` and a [link](https://example.com). ";

/// Build a markdown string of approximately `target_bytes`.
fn markdown_content(target_bytes: usize) -> String {
    // Mix in some heading structure and lists to exercise more parse paths.
    let heading = "# Section heading\n\n";
    let list_item = "- List item with **bold** and `code`\n";
    let mut out = String::with_capacity(target_bytes);
    // Prepend a heading and a few list items, then fill with prose.
    out.push_str(heading);
    while out.len() < target_bytes / 4 {
        out.push_str(list_item);
    }
    while out.len() < target_bytes.saturating_sub(MD_UNIT.len()) {
        out.push_str(MD_UNIT);
    }
    // Pad to exactly target_bytes if needed.
    while out.len() < target_bytes {
        out.push(' ');
    }
    out.truncate(target_bytes);
    out
}

// ─── Benchmark: cache hit — zero per-frame parse cost ─────────────────────

/// Benchmark a warm `get_by_key` lookup: should be a 32-byte HashMap read.
///
/// Contract: < 10 µs per lookup (essentially O(1) constant time).
/// This verifies that the per-frame markdown render path incurs **zero**
/// parse work for unchanged content.
fn bench_cache_hit(c: &mut Criterion) {
    let content = markdown_content(8 * 1024); // 8 KiB — representative size
    let tokens = MarkdownTokens::default();

    let mut cache = MarkdownCache::new();
    // Prime once — this parses the content and populates the cache.
    cache.prime(&content, &tokens);

    // Precompute the key exactly as the frame path does (via node_key_cache).
    let key = MarkdownCache::compute_key(&content);

    let mut group = c.benchmark_group("markdown_cache");
    group.bench_function(BenchmarkId::new("cache_hit", "8KiB_warm"), |b| {
        b.iter(|| {
            // This is the hot frame-path operation: O(32-byte map lookup).
            black_box(cache.get_by_key(black_box(&key)))
        })
    });
    group.finish();
}

// ─── Benchmark: commit-frame parse of 65535-byte content ─────────────────

/// Benchmark `prime` for a 65535-byte payload: the Phase-1 content ceiling.
///
/// Contract: < 1 ms (Stage 3/4 combined budget).  CI uses 500 ms ceiling to
/// tolerate debug builds.
///
/// This simulates the worst-case scenario where a streaming agent appends to
/// a text node and the full content must be parsed exactly once at commit time.
fn bench_parse_commit_64kb(c: &mut Criterion) {
    let content = markdown_content(65535);
    let tokens = MarkdownTokens::default();

    let mut group = c.benchmark_group("markdown_cache");
    group.bench_function(BenchmarkId::new("parse_commit_64kB", "cold"), |b| {
        b.iter(|| {
            // Use a fresh cache each iteration so this always exercises the
            // cold-parse path, not the hit path.  Drop the returned reference
            // immediately (we're measuring parse time, not allocation).
            let mut cache = MarkdownCache::new();
            // `prime` returns &ParsedMarkdown referencing `cache`; the borrow
            // ends when we convert to a raw pointer (for black_box) and never
            // dereference it.  The ParsedMarkdown is owned by `cache` and is
            // dropped at end of iteration.
            let parsed = cache.prime(black_box(&content), black_box(&tokens));
            // Extract the plain_text length so the optimizer can't elide the work.
            black_box(parsed.plain_text.len())
        })
    });
    group.finish();
}

// ─── Benchmark: BLAKE3 compute_key for 65535 bytes ───────────────────────

/// Benchmark the BLAKE3 content key computation for 65535 bytes.
///
/// This is the baseline cost that was being paid EVERY FRAME per
/// TextMarkdownNode before hud-gpqde.  After hud-gpqde the key is computed
/// once at prime time and stored in node_key_cache.
fn bench_compute_key_64kb(c: &mut Criterion) {
    let content = markdown_content(65535);

    let mut group = c.benchmark_group("markdown_cache");
    group.bench_function(BenchmarkId::new("compute_key", "64kB"), |b| {
        b.iter(|| black_box(MarkdownCache::compute_key(black_box(&content))))
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_cache_hit,
    bench_parse_commit_64kb,
    bench_compute_key_64kb,
);
criterion_main!(benches);
