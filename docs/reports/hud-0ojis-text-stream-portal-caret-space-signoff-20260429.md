# Text Stream Portal Caret and Space Sign-Off - hud-0ojis

Date: 2026-04-29
Branch: `agent/hud-0ojis`

## Scope

Close the focused polish bead for the text stream portal composer:

- caret x-position near right-edge visual line ends after long markdown-like paste;
- Space insertion after the input path switched normal printable text to runtime
  `Character` events with a Space-only `KeyDown` fallback.

This sign-off is scoped to deterministic composer layout/input behavior. It
does not close the broader Text Stream Portal exemplar UX refinement track.

## Resolution

The caret model now uses the same monospace advance constant for explicit wrap
calculation and caret placement:

- `COMPOSER_WRAP_CHAR_W = INPUT_FONT * 0.57`
- `COMPOSER_CARET_CHAR_W = COMPOSER_WRAP_CHAR_W`

That removes the prior one-character-looking lag when the explicit wrap model
placed the caret at a visual line end but the caret advance used a narrower
constant than the wrap advance.

The key-down printable fallback remains deliberately narrow:

- `Space` inserts `" "` and records a pending key echo.
- Non-Space printable key-downs return `None`; normal printable input must come
  from runtime `Character` events.

## Evidence

The focused regression coverage lives in
`.claude/skills/user-test/tests/test_text_stream_portal_exemplar.py`:

- `test_caret_advance_matches_explicit_wrap_advance_at_line_end`
- `test_space_is_the_only_printable_key_down_fallback`

The live exemplar script also includes `--self-test`, which exercises:

- `hello world` Space/fallback layout,
- wrapped markdown-like paste with no trailing whitespace visual rows,
- long unbroken word wrapping with caret bounded inside the wrap area.

## Verification

Commands run on `agent/hud-0ojis` after merging current `origin/main` into the
worker branch:

```bash
python3 -m py_compile .claude/skills/user-test/scripts/text_stream_portal_exemplar.py
python3 .claude/skills/user-test/tests/test_text_stream_portal_exemplar.py
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py --self-test
```

Result: all passed.

Live Windows validation was not rerun for this bead because the remaining risk
is covered by deterministic composer layout/input tests and the `composer-smoke`
live phase added to the user-test harness. Use `--phases composer-smoke` for the
next operator visual pass.
