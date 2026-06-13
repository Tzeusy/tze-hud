//! Bundled font assets for tze_hud.
//!
//! # Why bundled fonts?
//!
//! [`glyphon::FontSystem::new`] calls `db.load_system_fonts()` which scans the
//! OS font directory at runtime.  This is:
//!
//! - **Fragile on kiosk/minimal hosts** — headless servers, Windows Nano,
//!   container images, and similar environments may have zero or very few fonts
//!   installed.
//! - **Slow** — system font discovery takes up to 1 s on debug builds.
//! - **Non-deterministic** — the fonts available (and therefore glyph metrics,
//!   layout widths, and test pass/fail behaviour) vary by host.
//!
//! Instead, we use [`glyphon::FontSystem::new_with_locale_and_db`] with a
//! fontdb [`Database`][glyphon::fontdb::Database] pre-loaded with a fixed set
//! of OFL/permissive-licensed fonts embedded at compile time via
//! `include_bytes!`.  System fonts are **not** loaded.  Agent-uploaded fonts
//! are still supported via [`crate::text::TextRasterizer::load_font_bytes`].
//!
//! Note: [`glyphon::FontSystem::new_with_fonts`] loads system fonts *in
//! addition to* the supplied sources, so it cannot be used for system-font-free
//! operation.  Use [`bundled_font_system`] instead.
//!
//! # Font selection
//!
//! Three families cover all [`tze_hud_scene::types::FontFamily`] variants:
//!
//! | Family | Face(s) bundled | Used for |
//! |--------|----------------|----------|
//! | **DejaVu Sans** | Regular, Bold, Oblique, BoldOblique | `FontFamily::SystemSansSerif` |
//! | **DejaVu Sans Mono** | Regular, Bold, Oblique, BoldOblique | `FontFamily::SystemMonospace` |
//! | **DejaVu Serif** | Regular, Bold | `FontFamily::SystemSerif` |
//!
//! # License
//!
//! DejaVu fonts are derived from the Bitstream Vera fonts and are released
//! under a permissive license (redistribution and use as part of a larger
//! package is explicitly permitted).  The full license text is in
//! `fonts/dejavu/LICENSE` and is reproduced in this crate's README.
//!
//! Source: <https://dejavu-fonts.github.io/>

use glyphon::{FontSystem, fontdb};

// ── Embedded font bytes ───────────────────────────────────────────────────────

/// DejaVu Sans — regular weight, upright.
///
/// Maps to `FontFamily::SystemSansSerif` + weight 400.
static DEJAVU_SANS: &[u8] = include_bytes!("../fonts/dejavu/DejaVuSans.ttf");

/// DejaVu Sans — bold weight, upright.
///
/// Maps to `FontFamily::SystemSansSerif` + weight 700.
static DEJAVU_SANS_BOLD: &[u8] = include_bytes!("../fonts/dejavu/DejaVuSans-Bold.ttf");

/// DejaVu Sans — regular weight, oblique.
///
/// Maps to `FontFamily::SystemSansSerif` + italic style.
static DEJAVU_SANS_OBLIQUE: &[u8] = include_bytes!("../fonts/dejavu/DejaVuSans-Oblique.ttf");

/// DejaVu Sans — bold weight, oblique.
///
/// Maps to `FontFamily::SystemSansSerif` + bold + italic.
static DEJAVU_SANS_BOLD_OBLIQUE: &[u8] =
    include_bytes!("../fonts/dejavu/DejaVuSans-BoldOblique.ttf");

/// DejaVu Sans Mono — regular weight, upright.
///
/// Maps to `FontFamily::SystemMonospace` + weight 400.
static DEJAVU_SANS_MONO: &[u8] = include_bytes!("../fonts/dejavu/DejaVuSansMono.ttf");

/// DejaVu Sans Mono — bold weight, upright.
///
/// Maps to `FontFamily::SystemMonospace` + weight 700.
static DEJAVU_SANS_MONO_BOLD: &[u8] = include_bytes!("../fonts/dejavu/DejaVuSansMono-Bold.ttf");

/// DejaVu Sans Mono — regular weight, oblique.
///
/// Maps to `FontFamily::SystemMonospace` + italic style.
static DEJAVU_SANS_MONO_OBLIQUE: &[u8] =
    include_bytes!("../fonts/dejavu/DejaVuSansMono-Oblique.ttf");

/// DejaVu Sans Mono — bold weight, oblique.
///
/// Maps to `FontFamily::SystemMonospace` + bold + italic.
static DEJAVU_SANS_MONO_BOLD_OBLIQUE: &[u8] =
    include_bytes!("../fonts/dejavu/DejaVuSansMono-BoldOblique.ttf");

/// DejaVu Serif — regular weight, upright.
///
/// Maps to `FontFamily::SystemSerif` + weight 400.
static DEJAVU_SERIF: &[u8] = include_bytes!("../fonts/dejavu/DejaVuSerif.ttf");

/// DejaVu Serif — bold weight, upright.
///
/// Maps to `FontFamily::SystemSerif` + weight 700.
static DEJAVU_SERIF_BOLD: &[u8] = include_bytes!("../fonts/dejavu/DejaVuSerif-Bold.ttf");

// ── Public API ────────────────────────────────────────────────────────────────

/// The raw `&'static [u8]` slices for all bundled font faces.
///
/// Stored as a `static` so that `bundled_font_sources` can borrow it for
/// `'static` and hand each slice to `Arc::new` without a temporary lifetime.
static BUNDLED_FACES: [&[u8]; BUNDLED_FONT_FACE_COUNT] = [
    DEJAVU_SANS,
    DEJAVU_SANS_BOLD,
    DEJAVU_SANS_OBLIQUE,
    DEJAVU_SANS_BOLD_OBLIQUE,
    DEJAVU_SANS_MONO,
    DEJAVU_SANS_MONO_BOLD,
    DEJAVU_SANS_MONO_OBLIQUE,
    DEJAVU_SANS_MONO_BOLD_OBLIQUE,
    DEJAVU_SERIF,
    DEJAVU_SERIF_BOLD,
];

/// Build a self-contained [`FontSystem`] loaded with only the bundled DejaVu
/// font faces — no system fonts are loaded.
///
/// Uses [`FontSystem::new_with_locale_and_db`] to bypass the internal
/// `db.load_system_fonts()` call that [`FontSystem::new_with_fonts`] always
/// performs.  The returned system is fully deterministic: the same ten faces
/// are present on every host regardless of OS-installed fonts.
///
/// Family mappings are set so that [`glyphon::Family::SansSerif`],
/// [`glyphon::Family::Monospace`], and [`glyphon::Family::Serif`] resolve to
/// DejaVu Sans, DejaVu Sans Mono, and DejaVu Serif respectively.
///
/// [`FontFamily`]: tze_hud_scene::types::FontFamily
pub fn bundled_font_system() -> FontSystem {
    // Use the system locale when available; fall back to "en-US" if the
    // platform locale cannot be detected.  We do NOT call
    // `sys_locale::get_locale()` directly to avoid adding a direct dep —
    // cosmic-text (re-exported via glyphon) already carries sys-locale as a
    // transitive dep, but the crate is not in our direct dep list.
    // `FontSystem::new_with_locale_and_db` accepts any locale string; "en-US"
    // is the same fallback cosmic-text uses internally.
    let locale = std::env::var("LANG")
        .ok()
        .and_then(|v| {
            let v = v.replace('_', "-");
            let v = v.split('.').next().unwrap_or("").to_owned();
            if v.is_empty() { None } else { Some(v) }
        })
        .unwrap_or_else(|| String::from("en-US"));
    let mut db = fontdb::Database::new();

    // Load only our bundled faces — no system font scan.
    for bytes in BUNDLED_FACES.iter().copied() {
        db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(bytes)));
    }

    // Map generic families to the bundled DejaVu names.
    db.set_sans_serif_family("DejaVu Sans");
    db.set_monospace_family("DejaVu Sans Mono");
    db.set_serif_family("DejaVu Serif");

    FontSystem::new_with_locale_and_db(locale, db)
}

/// Return all bundled font faces as [`fontdb::Source::Binary`] sources.
///
/// Useful when you need to add the bundled faces to an existing
/// [`fontdb::Database`].  Note that [`FontSystem::new_with_fonts`] loads
/// system fonts *before* these sources, so prefer [`bundled_font_system`]
/// when building a system-font-free [`FontSystem`].
///
/// All ten sources are returned regardless of which [`FontFamily`] variants
/// are actually used at runtime; the compiler embeds all ten byte slices in
/// the binary (total ≈ 3.8 MiB) so no conditional loading is needed.
///
/// [`FontFamily`]: tze_hud_scene::types::FontFamily
pub fn bundled_font_sources() -> impl Iterator<Item = fontdb::Source> {
    BUNDLED_FACES
        .iter()
        .copied()
        .map(|bytes| fontdb::Source::Binary(std::sync::Arc::new(bytes)))
}

/// Number of font faces bundled at compile time.
///
/// Used in startup telemetry and tests to confirm the bundled font set is
/// intact.
pub const BUNDLED_FONT_FACE_COUNT: usize = 10;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Confirm that `bundled_font_system()` produces at least
    /// [`BUNDLED_FONT_FACE_COUNT`] faces in the database.
    ///
    /// This is a regression guard: if a font file is accidentally removed from
    /// the `fonts/` directory, `include_bytes!` fails at compile time. This
    /// test confirms that fontdb accepts and parses each file at runtime too.
    #[test]
    fn bundled_fonts_load_into_font_system() {
        let fs = bundled_font_system();
        let face_count = fs.db().faces().count();
        // Each TTF file should produce at least one face.
        assert!(
            face_count >= BUNDLED_FONT_FACE_COUNT,
            "expected at least {BUNDLED_FONT_FACE_COUNT} faces, got {face_count}"
        );
    }

    /// Confirm that the font system can resolve a sans-serif, monospace, and
    /// serif family using only the bundled faces (no system font fallback).
    #[test]
    fn bundled_font_families_are_queryable() {
        use glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Wrap};

        // bundled_font_system() already sets the family mappings.
        let mut fs = bundled_font_system();

        for family in [Family::SansSerif, Family::Monospace, Family::Serif] {
            let attrs = Attrs::new().family(family);
            let mut buf = Buffer::new(&mut fs, Metrics::new(16.0, 22.0));
            buf.set_size(&mut fs, Some(400.0), Some(200.0));
            buf.set_wrap(&mut fs, Wrap::Word);
            buf.set_text(&mut fs, "Hello world", attrs, Shaping::Advanced);
            buf.shape_until_scroll(&mut fs, false);

            // Confirm at least one layout run was produced (font found and
            // shaped successfully).
            let run_count: usize = buf
                .layout_runs()
                .map(|r| {
                    // Count non-empty runs (at least one glyph).
                    if r.glyphs.is_empty() { 0 } else { 1 }
                })
                .sum();
            assert!(
                run_count >= 1,
                "expected at least one layout run for family {family:?}, got 0"
            );
        }
    }

    /// Confirm that `Shaping::Advanced` is usable with bundled fonts —
    /// rustybuzz is linked and produces shaped glyphs without panicking.
    ///
    /// This test is part of the Part B evaluation (hud-bq0gl.11): verifying
    /// that `Shaping::Advanced` works correctly with the bundled font set
    /// before any migration of production call sites.
    #[test]
    fn shaping_advanced_works_with_bundled_fonts() {
        use glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Wrap};

        let mut fs = bundled_font_system();

        // LTR Latin — baseline case.
        let attrs = Attrs::new().family(Family::SansSerif);
        let mut buf = Buffer::new(&mut fs, Metrics::new(16.0, 22.0));
        buf.set_size(&mut fs, Some(400.0), Some(200.0));
        buf.set_wrap(&mut fs, Wrap::Word);
        buf.set_text(&mut fs, "Advanced shaping works", attrs, Shaping::Advanced);
        buf.shape_until_scroll(&mut fs, false);

        let glyph_count: usize = buf.layout_runs().map(|r| r.glyphs.len()).sum();
        assert!(
            glyph_count >= 1,
            "Shaping::Advanced produced no glyphs for Latin text"
        );
    }

    /// Confirm that `bundled_font_system()` exposes exactly the faces we
    /// embedded and no more (i.e. system fonts are NOT loaded).
    #[test]
    fn bundled_font_system_contains_no_system_fonts() {
        let fs = bundled_font_system();
        let face_count = fs.db().faces().count();
        // We embed exactly 10 TTF files. If system fonts leaked in, this count
        // would typically be much higher (50–500 faces on a desktop OS).
        // Allow a small margin: fontdb may produce multiple faces per TTF (e.g.
        // variable-font axes), so we allow up to 3× per file.
        assert!(
            face_count <= BUNDLED_FONT_FACE_COUNT * 3,
            "unexpected face count {face_count}; system fonts may have leaked in \
             (expected at most {} faces for {BUNDLED_FONT_FACE_COUNT} files)",
            BUNDLED_FONT_FACE_COUNT * 3,
        );
    }
}
