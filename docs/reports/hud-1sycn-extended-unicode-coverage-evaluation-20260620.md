# Extended Unicode Coverage Evaluation — Bundled Fonts

**Bead:** hud-1sycn (from PR #802 / hud-bq0gl.11 review)
**Date:** 2026-06-20
**Type:** Evaluation / recommendation (docs-only — **no font-bundling change**)
**Related (out of scope):** hud-hkle2 (Shaping::Basic → Shaping::Advanced migration)

## Summary

The compositor ships a fixed, system-font-free font set: ten DejaVu faces
embedded via `include_bytes!`. This is deliberate (deterministic layout, robust
on kiosk/headless hosts) and should not be reverted. However, the bundled set
covers only Latin / Greek / Cyrillic / Armenian / Georgian / Hebrew / Arabic and
a handful of symbol blocks. **CJK (Han, Hiragana, Katakana, Hangul), Indic
(Devanagari), Thai, and color emoji are not covered**, and **DejaVu Serif ships
no Italic/BoldItalic faces**, so serif italic is faux-synthesized (skewed
upright).

Critically, because the `FontSystem` loads **only** DejaVu (no system fonts),
there is **no fallback font** for uncovered codepoints: missing glyphs resolve to
`.notdef` (tofu boxes), not to a substitute typeface. The PR-review phrasing
"silent fallback" overstates the graceful case — in practice an agent that
publishes Japanese or an emoji today gets boxes.

**Recommendation for v1: do nothing (Option C).** Defer bundling until the
compositor sees real HUD sessions that actually demand non-Latin scripts (the
deferral condition already noted on the bead). When that demand materializes,
prefer **Option A (bundle a Noto CJK subset)** over system-font opt-in, because
system fonts reintroduce exactly the non-determinism the bundled-font design
removed. Track any chosen follow-up as a new bead.

## 1. What ships today, and where

### Font files

Ten TTF faces under
[`crates/tze_hud_compositor/fonts/dejavu/`](../../crates/tze_hud_compositor/fonts/dejavu/)
(total ≈ 4.6 MiB on disk; ≈ 3.8 MiB embedded per the doc comment), plus
`LICENSE`:

| File | On-disk size | Family / style |
|---|---:|---|
| `DejaVuSans.ttf` | 757 KiB | Sans Regular |
| `DejaVuSans-Bold.ttf` | 705 KiB | Sans Bold |
| `DejaVuSans-Oblique.ttf` | 635 KiB | Sans Oblique |
| `DejaVuSans-BoldOblique.ttf` | 643 KiB | Sans Bold Oblique |
| `DejaVuSansMono.ttf` | 340 KiB | Mono Regular |
| `DejaVuSansMono-Bold.ttf` | 332 KiB | Mono Bold |
| `DejaVuSansMono-Oblique.ttf` | 252 KiB | Mono Oblique |
| `DejaVuSansMono-BoldOblique.ttf` | 254 KiB | Mono Bold Oblique |
| `DejaVuSerif.ttf` | 380 KiB | Serif Regular |
| `DejaVuSerif-Bold.ttf` | 356 KiB | Serif Bold |

Note the asymmetry: Sans and Sans Mono each ship 4 faces (Regular/Bold ×
upright/oblique); **Serif ships only Regular + Bold — no Italic or BoldItalic.**

### Loading code

[`crates/tze_hud_compositor/src/fonts.rs`](../../crates/tze_hud_compositor/src/fonts.rs):

- `include_bytes!` of each face — `fonts.rs:51`–`fonts.rs:99`.
- `BUNDLED_FACES: [&[u8]; 10]` — `fonts.rs:107`.
- `bundled_font_system()` — `fonts.rs:133`. Builds a `fontdb::Database`, loads
  **only** the 10 bundled sources (`fonts.rs:152`–`fonts.rs:154`), maps the
  generic families (`fonts.rs:157`–`fonts.rs:159`), and constructs the
  `FontSystem` via `FontSystem::new_with_locale_and_db` (`fonts.rs:161`) —
  deliberately bypassing `db.load_system_fonts()` so **no OS fonts are present**
  (see module doc `fonts.rs:5`–`fonts.rs:23`).
- `BUNDLED_FONT_FACE_COUNT = 10` and the "no system fonts leaked" guard test
  (`fonts.rs:187`, `fonts.rs:274`–`fonts.rs:288`).

### Family mapping

The scene API exposes exactly three families
([`crates/tze_hud_scene/src/types.rs:280`](../../crates/tze_hud_scene/src/types.rs)):
`SystemSansSerif`, `SystemMonospace`, `SystemSerif` → DejaVu Sans / Sans Mono /
Serif respectively.

### Shaping path

All production shaping already uses `Shaping::Advanced` (rustybuzz) in
[`text.rs`](../../crates/tze_hud_compositor/src/text.rs) (e.g. `text.rs:659`,
`text.rs:705`, `text.rs:770`) and
[`overflow.rs`](../../crates/tze_hud_compositor/src/overflow.rs) (e.g.
`overflow.rs:615`, `overflow.rs:745`). (The Basic→Advanced migration item
hud-hkle2 concerns remaining call sites and is out of scope here.) Shaping does
not change coverage: rustybuzz can only shape glyphs the face actually contains.

### Agent-uploaded fonts

Agents can side-load arbitrary TTF/OTF at runtime via
`TextRasterizer::load_font_bytes`
([`renderer/image_cache.rs:224`](../../crates/tze_hud_compositor/src/renderer/image_cache.rs)),
which adds the face to the live `FontSystem`. This is the **existing escape
hatch**: a session needing CJK today could upload its own font. It does not solve
the default-coverage problem (every session would have to ship its own font), but
it bounds the severity.

## 2. Coverage matrix (measured)

Measured directly from the shipped TTF `cmap` tables with fontTools 4.53.0 on
2026-06-20 (probe = one representative codepoint per block; ✓ = glyph present,
✗ = `.notdef`/tofu). Glyph counts: DejaVu Sans 6253, Sans Mono 3377, Serif 3528.

| Script / range | Probe | Sans | Sans Mono | Serif |
|---|---|:--:|:--:|:--:|
| Latin / Latin-1 | A, é | ✓ | ✓ | ✓ |
| Greek | α U+03B1 | ✓ | ✓ | ✓ |
| Cyrillic | Я U+042F | ✓ | ✓ | ✓ |
| Armenian | U+0531 | ✓ | ✓ | ✓ |
| Georgian | U+10D0 | ✓ | ✓ | ✓ |
| Hebrew | א U+05D0 | ✓ | ✗ | ✗ |
| Arabic | ع U+0639 | ✓ | ✓ | ✗ |
| Thai | ก U+0E01 | ✗ | ✗ | ✗ |
| Devanagari (Indic) | क U+0915 | ✗ | ✗ | ✗ |
| **CJK Han** | 漢 U+6F22 | ✗ | ✗ | ✗ |
| **Hiragana** | あ U+3042 | ✗ | ✗ | ✗ |
| **Katakana** | ア U+30A2 | ✗ | ✗ | ✗ |
| **Hangul** | 가 U+AC00 | ✗ | ✗ | ✗ |
| **Color emoji** | 😀 U+1F600 | △ | ✗ | ✗ |
| Heart dingbat | ❤ U+2764 | ✓ | ✓ | ✗ |
| Check mark | ✓ U+2713 | ✓ | ✓ | ✗ |
| Box drawing | ─ U+2500 | ✓ | ✓ | ✓ |
| Braille | ⠁ U+2801 | ✓ | ✗ | ✓ |

△ = a monochrome outline glyph exists in DejaVu Sans, but it is **not color
emoji** (no `COLR`/`CBDT`/`sbix` tables); it renders as a flat black-and-white
shape, never the colored pictograph users expect.

### Style coverage (italic / oblique)

| Family | Regular | Bold | Italic/Oblique | Bold Italic/Oblique |
|---|:--:|:--:|:--:|:--:|
| Sans | ✓ real | ✓ real | ✓ real (Oblique) | ✓ real (Oblique) |
| Sans Mono | ✓ real | ✓ real | ✓ real (Oblique) | ✓ real (Oblique) |
| **Serif** | ✓ real | ✓ real | ✗ **synthesized** | ✗ **synthesized** |

The Sans/Mono oblique faces carry the italic style bit (`head.macStyle`
verified). DejaVu Serif has **no upstream Italic/BoldItalic** — none exists in the
DejaVu project — so a serif-italic request is satisfied by FontDB/cosmic-text
**faux-obliquing** the upright Serif (a shear transform), which is visually
inferior to a true italic (no cursive letterforms, distorted curves).

### Failure mode for uncovered codepoints

Because `bundled_font_system()` loads only DejaVu, there is **no fallback face**
in the database. cosmic-text's fallback search finds nothing for an uncovered
codepoint and emits the `.notdef` glyph → a **tofu box**, not a substituted
typeface. So the practical outcome for CJK/emoji/Thai/Devanagari today is visible
boxes, not graceful degradation.

## 3. Options & tradeoffs

### Option A — Bundle a Noto CJK subset (preferred *when* coverage is needed)

Add a CJK-capable face to `BUNDLED_FACES` and register it in
`bundled_font_system()`. Because shaping already runs `Shaping::Advanced` and the
DB drives fallback, an added face is picked up automatically for codepoints
DejaVu lacks — no call-site changes.

**Which subset.** Full pan-CJK is large; scope to the actual target:

| Candidate | Scope | Approx. footprint (embedded) | Notes |
|---|---|---:|---|
| Noto Sans CJK **JP** (single-language OTF/OTC, one weight) | JP kana + JIS Han | ~5–8 MiB/weight | Covers Japanese; Han subset also renders most common Chinese. Per-language builds avoid the full 16 MiB+ pan-CJK. |
| Noto Sans **SC / TC / KR** (per-language) | Simplified / Traditional / Korean | ~5–9 MiB/weight each | Pick by audience; Han glyph shapes differ per locale. |
| Noto Sans CJK (pan-CJK OTC, all langs) | JP+SC+TC+KR | ~16–40 MiB depending on weights | One file, simplest, but heavy in-binary. |
| **`fontTools` static subset** of any of the above | only the codepoints we commit to support | tunable, can hit ~1–3 MiB | Best size/coverage tradeoff; adds a build step + a "supported set" contract to maintain. |

**Emoji is separate from CJK.** Noto CJK does **not** provide color emoji. Color
emoji would need Noto Color Emoji (`CBDT`/`COLR` ≈ 9–24 MiB) **and** glyphon/wgpu
support for color bitmap/COLR glyph rasterization, which is a larger renderer
question — recommend keeping emoji out of v1 scope entirely.

**Serif italic** can be closed independently and cheaply by sourcing a true
serif-italic face (DejaVu has none; a Noto Serif Italic or other OFL serif italic
would be required) — but only if real sessions request serif italic.

**Licensing.** Noto fonts are **SIL OFL 1.1** — redistribution/embedding in a
larger work is permitted with the license file included, fully compatible with
the existing bundled-font approach (DejaVu ships under its own permissive
Bitstream-Vera-derived license, already reproduced at
`fonts/dejavu/LICENSE`). Add the Noto `LICENSE` alongside it and note it in the
crate docs.

**Cost.** Binary size grows by the subset footprint; `include_bytes!` means it is
always resident even for Latin-only sessions (the doc comment at `fonts.rs:171`
already acknowledges all faces are unconditionally embedded). A subset build step
adds maintenance. Deterministic-layout guarantee is preserved.

### Option B — System-font opt-in

Add a config flag that, when set, augments the bundled DB with system fonts
(e.g. `FontSystem::new_with_fonts`, which loads system fonts *in addition to*
supplied sources — already noted as a known behavior at `fonts.rs:21`). Off by
default.

**Pros:** zero binary growth; broad coverage on full desktop OSes; no font
sourcing/licensing burden.
**Cons:** reintroduces exactly the non-determinism the bundled design removed
(`fonts.rs:8`–`fonts.rs:13`) — glyph metrics, layout widths, and therefore the
overflow/truncation tests become host-dependent; useless on the kiosk/headless/
Nano/container hosts that motivated bundling (those have no CJK fonts either);
adds the ~1 s system-font-scan cost. **Not recommended as the primary path**;
acceptable only as an explicit, off-by-default "desktop convenience" toggle,
never the default.

### Option C — Do nothing for v1 (recommended now)

Keep the current set. Latin/Greek/Cyrillic/Hebrew/Arabic + symbols cover the
near-term agent-presence content; the `load_font_bytes` upload path
(`image_cache.rs:224`) is the escape hatch for a session that genuinely needs
CJK before bundling lands.

**Pros:** no binary growth, no new licensing/build surface, preserves
determinism, matches the bead's stated deferral ("once the compositor sees real
HUD sessions"). **Cons:** CJK/emoji/Thai/Devanagari render as tofu and serif
italic is faux until A or B lands; the failure is silent at the API layer (no
warning when an agent publishes uncovered text).

**Low-cost hardening compatible with C:** emit a `tracing` warning (or a
per-zone diagnostic) when shaped output contains `.notdef` glyphs, so uncovered
text is observable instead of silently boxed. File as a small follow-up if
desired.

## Recommendation

1. **v1: Option C (do nothing).** Defer bundling per the existing condition.
2. **When real sessions need it: Option A**, scoped to the actual target
   language as a `fontTools` subset (size/coverage sweet spot), OFL license file
   included. Treat **emoji** and **serif italic** as independent, separately
   scoped follow-ups (emoji additionally gated on color-glyph renderer support).
3. **Avoid Option B as the default**; if added at all, make it an explicit
   off-by-default desktop toggle.
4. Optionally add `.notdef`/tofu telemetry so uncovered-text incidents are
   visible without waiting for a user report.

Out of scope: hud-hkle2 (Shaping::Basic → Advanced migration). All new bundling
or telemetry work should be filed as new beads; this deliverable is evaluation
only.
