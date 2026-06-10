# Pixel Readback (llvmpipe) CI Job — Flake Investigation

**Issue:** hud-bylxu  
**Date:** 2026-06-10  
**Author:** Beads Worker (agent/hud-bylxu)

---

## Summary

The "pixel readback (GPU / llvmpipe, informational)" CI job is failing
**consistently** (not randomly) on all PRs merged after PR #664. The failure
is caused by a regression in `from_text_markdown_cached` (introduced by
PR #664) that silently drops `node.color_runs` when the markdown cache path
is taken. The test `test_color_runs_red_error_text_rendered` fails because its
`color_runs`-based red text run is never applied to the compositor.

**Status:** Fixed in this PR. The root cause was a code defect, not an
environmental flake.

---

## Flake Rate / Failure History

| PR | Type | pixel readback result |
|----|------|-----------------------|
| #662 | docs only | **PASS** (51/51 tests) |
| #666 | docs only | **FAIL** (50/51) — first failure |
| #669 | feature | **FAIL** (50/51) |
| #670 | feature | **FAIL** (50/51) |
| #671 | feature | **FAIL** (50/51) |

Flake rate from this window: **4/5 runs fail** (80%). All failures are
on runs after PR #664 was merged. PR #662 passed on code that predates #664.

**This is not a nondeterministic environmental flake.** It is a deterministic
regression that fails on every run after PR #664.

---

## Failure Signature

All failing runs produce the same panic:

```
thread 'test_color_runs_red_error_text_rendered' panicked at
crates/tze_hud_runtime/tests/pixel_readback.rs:1193:5:
color_runs: expected at least one strongly red-dominant pixel in the
'ERROR' region (scan [55..250) × [405..595)).
This indicates the red color run was not applied by the compositor.
```

Every failing run: `test result: FAILED. 50 passed; 1 failed; 0 ignored`

Run URLs:
- PR #666 (docs only): https://github.com/Tzeusy/tze-hud/actions/runs/27260065426/job/80504035936
- PR #669: https://github.com/Tzeusy/tze-hud/actions/runs/27265917804/job/80523583415
- PR #670: https://github.com/Tzeusy/tze-hud/actions/runs/27269941118/job/80537287218
- PR #671: https://github.com/Tzeusy/tze-hud/actions/runs/27269693734/job/80536407827

---

## Root Cause

### Introducing commit

PR #664 (commit `2e803a5f`): *"feat: Phase-1 markdown subset parse-on-commit
cache [hud-5jbra.2]"*

This PR introduced `from_text_markdown_cached` in `crates/tze_hud_compositor/src/text.rs`
and changed `collect_text_items_from_node` in `renderer.rs` to call it on
every markdown cache hit.

### The defect

`from_text_markdown_cached` builds a `TextItem` from the markdown parse
cache. It uses `parsed.plain_text` (the Markdown-stripped version of the
content) as the text base, and `parsed.spans` to build `styled_runs`. However,
it **unconditionally drops `node.color_runs`**:

```rust
// from_text_markdown_cached — line 920 (before fix)
color_runs: Box::default(),  // <-- node.color_runs silently discarded
styled_runs,
```

### Why the test fails

The test `test_color_runs_red_error_text_rendered` constructs a
`TextMarkdownNode` with:
- `content: "ERROR rest of the text"` (no Markdown constructs)
- `color_runs: [TextColorRun { start_byte: 0, end_byte: 5, color: RED }]`

At render time:
1. `prime_markdown_cache` is called, parses `"ERROR rest of the text"` — a
   plain string with no Markdown markup. `parsed.spans` is empty.
2. `collect_text_items_from_node` finds a cache hit and calls
   `from_text_markdown_cached`.
3. `from_text_markdown_cached` builds `styled_runs` from empty `parsed.spans`
   → `styled_runs` is empty.
4. `color_runs` from the node is **dropped** (`Box::default()`).
5. The renderer falls through to the uniform-base-color path (no color runs
   applied) → all text renders as the base white color.
6. The pixel scan finds no red-dominant pixels → assertion fails.

### Why PR #662 passed

PR #662 ran on main *before* PR #664 was merged. The markdown cache did not
exist at that point. `from_text_markdown_node` was used unconditionally and
correctly preserved `color_runs`.

### Why the same docs-only PR #666 fails

PR #666 is docs-only but its CI runs against the merged `main` which now
includes #664. The markdown cache exists, hits for "ERROR rest of the text",
and the defect triggers.

---

## Fix

### `crates/tze_hud_compositor/src/renderer.rs`

In `collect_text_items_from_node` and `collect_ellipsis_text_items_from_node`,
guard the cache path with a `color_runs.is_empty()` check:

```rust
// Before (defect):
let item = if let Some(parsed) = self.markdown_cache.get_by_key(&content_key) {
    TextItem::from_text_markdown_cached(tm, ...)
} else {
    TextItem::from_text_markdown_node(tm, ...)
};

// After (fix):
let item = if tm.color_runs.is_empty() {
    if let Some(parsed) = self.markdown_cache.get_by_key(&content_key) {
        TextItem::from_text_markdown_cached(tm, ...)
    } else {
        TextItem::from_text_markdown_node(tm, ...)
    }
} else {
    // color_runs present: cache path drops color_runs (byte offsets are
    // against raw content, not stripped plain_text). Use legacy path.
    TextItem::from_text_markdown_node(tm, ...)
};
```

### `crates/tze_hud_compositor/src/text.rs`

Added a `# color_runs incompatibility` doc note to `from_text_markdown_cached`
making the contract explicit: callers must not use this constructor when
`node.color_runs` is non-empty.

---

## Why This Looked Like a Flake

- The test is in the **informational** (non-blocking) job.
- PR #662 passed and PR #666 failed on the same day, giving the appearance
  of environmental nondeterminism.
- The actual cause was a code change (PR #664) merged between the two runs.
- No true environmental flakiness was observed in the CI logs; all other 50
  tests pass consistently across all runs.

---

## Recommendations

1. **This PR fixes the deterministic failure.** The `test_color_runs_red_error_text_rendered`
   test should pass after this change.

2. **Add a unit test for `from_text_markdown_cached` with non-empty
   `color_runs`.** The test should assert that callers with `color_runs` must
   not use the cached path, or alternatively add a debug-mode assertion to
   `from_text_markdown_cached` that panics if `color_runs` is non-empty.

3. **Consider making the pixel readback job a blocking gate** once the test
   is confirmed stable. The `continue-on-error: true` flag in the CI workflow
   was the reason this regression went unnoticed for multiple PRs.
