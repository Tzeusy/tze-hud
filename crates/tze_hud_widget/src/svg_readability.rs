//! Widget SVG readability convention validator — hud-sc0a.6.
//!
//! Scans widget SVG files for `data-role="text"` and `data-role="backdrop"`
//! elements and validates structural conventions required by the component
//! type's readability technique.
//!
//! Source: `component-shape-language/spec.md §Requirement: Widget SVG Readability Conventions`
//! and `§Requirement: SVG data-role Attribute Convention`.
//!
//! ## Conventions enforced
//!
//! 1. `data-role="backdrop"` elements MUST precede `data-role="text"` elements in
//!    document order (SVG painter's model — backdrop paints behind text).
//! 2. `data-role="text"` elements in **DualLayer** profiles MUST have both `fill`
//!    and `stroke` attributes.
//! 3. `data-role="text"` elements in **OpaqueBackdrop** profiles MUST have `fill`.
//!
//! Elements without `data-role` are not subject to these checks (decorative elements).
//!
//! ## Scope
//!
//! Only profile-scoped widget bundles are subject to readability checks.
//! Global widget bundles (not inside a profile directory) MUST NOT be checked.
//! The caller is responsible for passing only profile-scoped SVGs.

use quick_xml::Reader;
use quick_xml::events::Event;

// ─── Readability technique (mirrored from tze_hud_config) ────────────────────

/// Which readability technique applies for a component type.
///
/// Mirrors `tze_hud_config::component_types::ReadabilityTechnique` so that
/// `tze_hud_widget` does not need to depend on `tze_hud_config`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvgReadabilityTechnique {
    /// Subtitle-style: both `fill` and `stroke` required on `data-role="text"`.
    DualLayer,
    /// Notification-style: only `fill` required on `data-role="text"`.
    OpaqueBackdrop,
    /// No readability checks.
    None,
}

// ─── SVG element kind found in a scan pass ────────────────────────────────────

/// Describes a `data-role`-tagged element found by the scanner.
#[derive(Debug, PartialEq, Eq)]
enum DataRoleElement {
    /// `data-role="backdrop"` element found at this document-order position.
    Backdrop { position: usize },
    /// `data-role="text"` element found at this position, with attribute flags.
    Text {
        position: usize,
        has_fill: bool,
        has_stroke: bool,
    },
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Validate a widget SVG for readability structural conventions.
///
/// Scans `svg_text` for elements with `data-role="backdrop"` or
/// `data-role="text"` attributes and checks:
///
/// - **Document order**: every `data-role="backdrop"` MUST appear before every
///   `data-role="text"` in document order (painter's model).
/// - **DualLayer**: each `data-role="text"` MUST have both `fill` and `stroke`.
/// - **OpaqueBackdrop**: each `data-role="text"` MUST have `fill`.
/// - **None**: no checks performed.
///
/// # Arguments
///
/// - `svg_text`: SVG source (after token placeholder resolution).
/// - `technique`: readability technique for this profile.
///
/// # Returns
///
/// `Ok(())` if all conventions are satisfied, or `Err(detail)` where `detail`
/// is a human-readable violation description.  The caller wraps this in a
/// [`crate::error::BundleError::ReadabilityConventionViolation`].
pub fn check_svg_readability(
    svg_text: &str,
    technique: SvgReadabilityTechnique,
) -> Result<(), String> {
    if technique == SvgReadabilityTechnique::None {
        return Ok(());
    }

    let elements = scan_data_role_elements(svg_text)?;

    // ── Document order check ──────────────────────────────────────────────────
    // Every backdrop must precede every text element.
    let last_backdrop_pos: Option<usize> = elements
        .iter()
        .filter_map(|e| match e {
            DataRoleElement::Backdrop { position } => Some(*position),
            _ => None,
        })
        .max();

    let first_text_pos: Option<usize> = elements
        .iter()
        .filter_map(|e| match e {
            DataRoleElement::Text { position, .. } => Some(*position),
            _ => None,
        })
        .min();

    if let (Some(last_bd), Some(first_txt)) = (last_backdrop_pos, first_text_pos) {
        if last_bd > first_txt {
            return Err(
                "data-role=\"backdrop\" element appears after data-role=\"text\" in document \
                 order; SVG uses the painter's model — backdrop must precede text so that text \
                 renders on top of the backdrop"
                    .to_string(),
            );
        }
    }

    // ── Per-text attribute checks ─────────────────────────────────────────────
    for element in &elements {
        if let DataRoleElement::Text {
            position: _,
            has_fill,
            has_stroke,
        } = element
        {
            match technique {
                SvgReadabilityTechnique::DualLayer => {
                    if !has_fill || !has_stroke {
                        let missing = match (has_fill, has_stroke) {
                            (false, false) => "fill and stroke",
                            (false, true) => "fill",
                            (true, false) => "stroke",
                            (true, true) => unreachable!(),
                        };
                        return Err(format!(
                            "data-role=\"text\" element is missing required attribute(s): \
                             {missing}; DualLayer profiles (e.g. subtitle) require both fill \
                             and stroke on text elements for readability"
                        ));
                    }
                }
                SvgReadabilityTechnique::OpaqueBackdrop => {
                    if !has_fill {
                        return Err(
                            "data-role=\"text\" element is missing required fill attribute; \
                             OpaqueBackdrop profiles require fill on text elements"
                                .to_string(),
                        );
                    }
                }
                SvgReadabilityTechnique::None => {}
            }
        }
    }

    Ok(())
}

// ─── Scanner ─────────────────────────────────────────────────────────────────

/// Scan SVG source for all `data-role`-tagged elements and collect their
/// position, type, and relevant attribute presence flags.
///
/// Uses quick-xml event-based parsing (same as `svg_ids::collect_svg_element_ids`).
/// `data-role` is a non-standard XML attribute; it is ignored by resvg but
/// consumed here before the SVG reaches the renderer.
fn scan_data_role_elements(svg_text: &str) -> Result<Vec<DataRoleElement>, String> {
    let mut elements: Vec<DataRoleElement> = Vec::new();
    let mut position: usize = 0;

    let mut reader = Reader::from_str(svg_text);
    reader.config_mut().trim_text(true);

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let mut data_role: Option<String> = None;
                let mut has_fill = false;
                let mut has_stroke = false;

                for attr in e.attributes().flatten() {
                    let local_name = attr.key.local_name();
                    let key = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    let val =
                        std::str::from_utf8(attr.value.as_ref()).unwrap_or("").trim().to_string();

                    match key {
                        "data-role" => {
                            data_role = Some(val);
                        }
                        "fill" => {
                            has_fill = true;
                        }
                        "stroke" => {
                            // Only count non-"none" stroke as present.
                            if val != "none" {
                                has_stroke = true;
                            }
                        }
                        _ => {}
                    }
                }

                if let Some(role) = data_role {
                    match role.as_str() {
                        "backdrop" => {
                            elements.push(DataRoleElement::Backdrop { position });
                        }
                        "text" => {
                            elements.push(DataRoleElement::Text {
                                position,
                                has_fill,
                                has_stroke,
                            });
                        }
                        _ => {
                            // Unknown data-role value — ignored.
                        }
                    }
                    position += 1;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    Ok(elements)
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── DualLayer (subtitle) pass ─────────────────────────────────────────────

    #[test]
    fn dual_layer_well_formed_passes() {
        // Scenario: Well-formed text SVG passes validation
        // backdrop precedes text, text has fill + stroke.
        // NOTE: raw strings use r##"..."## because SVG values contain "#" (e.g. fill="#000000").
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" opacity="0.6" width="200" height="50"/>
            <text data-role="text" fill="#FFFFFF" stroke="#000000" stroke-width="2">Subtitle</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(result.is_ok(), "well-formed DualLayer SVG should pass: {result:?}");
    }

    #[test]
    fn dual_layer_no_data_role_elements_passes() {
        // No data-role elements at all → nothing to validate.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect fill="#000000" width="200" height="50"/>
            <text fill="#FFFFFF">plain text, no data-role</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(result.is_ok(), "SVG without data-role elements should pass: {result:?}");
    }

    // ── DualLayer fail: missing stroke ────────────────────────────────────────

    #[test]
    fn dual_layer_text_without_stroke_fails() {
        // Scenario: Text without stroke in DualLayer profile
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" opacity="0.6" width="200" height="50"/>
            <text data-role="text" fill="#FFFFFF">No stroke here</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(
            result.is_err(),
            "DualLayer text missing stroke must fail; got: {result:?}"
        );
        let detail = result.unwrap_err();
        assert!(
            detail.contains("stroke"),
            "error detail must mention 'stroke': {detail}"
        );
    }

    #[test]
    fn dual_layer_text_without_fill_fails() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" width="200" height="50"/>
            <text data-role="text" stroke="#000000">No fill here</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(result.is_err(), "DualLayer text missing fill must fail");
        let detail = result.unwrap_err();
        assert!(detail.contains("fill"), "error detail must mention 'fill': {detail}");
    }

    #[test]
    fn dual_layer_text_without_fill_or_stroke_fails() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" width="200" height="50"/>
            <text data-role="text">Nothing at all</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(result.is_err(), "DualLayer text missing fill and stroke must fail");
        let detail = result.unwrap_err();
        assert!(
            detail.contains("fill") && detail.contains("stroke"),
            "error must mention both fill and stroke: {detail}"
        );
    }

    // ── DualLayer fail: document order violation ───────────────────────────────

    #[test]
    fn dual_layer_backdrop_after_text_fails() {
        // Scenario: Backdrop after text rejected
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <text data-role="text" fill="#FFFFFF" stroke="#000000">First</text>
            <rect data-role="backdrop" fill="#000000" width="200" height="50"/>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(
            result.is_err(),
            "backdrop after text must fail with DualLayer; got: {result:?}"
        );
        let detail = result.unwrap_err();
        assert!(
            detail.contains("painter") || detail.contains("document order") || detail.contains("backdrop"),
            "error must mention document order / painter's model: {detail}"
        );
    }

    // ── OpaqueBackdrop (notification) pass ────────────────────────────────────

    #[test]
    fn opaque_backdrop_text_with_fill_passes() {
        // Scenario: Text without stroke in OpaqueBackdrop profile passes.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" opacity="0.9" width="200" height="50"/>
            <text data-role="text" fill="#FFFFFF">No stroke needed</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::OpaqueBackdrop);
        assert!(
            result.is_ok(),
            "OpaqueBackdrop text with fill (no stroke) should pass: {result:?}"
        );
    }

    #[test]
    fn opaque_backdrop_text_with_fill_and_stroke_passes() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" opacity="0.9" width="200" height="50"/>
            <text data-role="text" fill="#FFFFFF" stroke="#333333">With stroke too</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::OpaqueBackdrop);
        assert!(result.is_ok(), "OpaqueBackdrop text with fill+stroke should pass: {result:?}");
    }

    // ── OpaqueBackdrop fail: missing fill ─────────────────────────────────────

    #[test]
    fn opaque_backdrop_text_without_fill_fails() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" opacity="0.9" width="200" height="50"/>
            <text data-role="text">No fill attribute</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::OpaqueBackdrop);
        assert!(result.is_err(), "OpaqueBackdrop text without fill must fail");
        let detail = result.unwrap_err();
        assert!(detail.contains("fill"), "error must mention fill: {detail}");
    }

    // ── OpaqueBackdrop: document order still enforced ─────────────────────────

    #[test]
    fn opaque_backdrop_backdrop_after_text_fails() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <text data-role="text" fill="#FFFFFF">First</text>
            <rect data-role="backdrop" fill="#000000" width="200" height="50"/>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::OpaqueBackdrop);
        assert!(result.is_err(), "backdrop after text must fail for OpaqueBackdrop too");
    }

    // ── None technique: no checks ─────────────────────────────────────────────

    #[test]
    fn none_technique_skips_all_checks() {
        // Even a malformed SVG passes when technique is None.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <text data-role="text">No fill, no stroke</text>
            <rect data-role="backdrop" fill="#000000"/>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::None);
        assert!(result.is_ok(), "None technique must skip all readability checks");
    }

    // ── Text without data-role is ignored ─────────────────────────────────────

    #[test]
    fn text_without_data_role_is_ignored_in_dual_layer() {
        // Scenario: Text without data-role skips validation.
        // Plain <text> elements (no data-role) must not be subject to checks.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <text fill="#FF0000">decorative label</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(
            result.is_ok(),
            "text without data-role should not be subject to readability checks: {result:?}"
        );
    }

    // ── stroke="none" does not count as having stroke ─────────────────────────

    #[test]
    fn dual_layer_stroke_none_counts_as_missing() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" width="200" height="50"/>
            <text data-role="text" fill="#FFFFFF" stroke="none">Explicit none</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(
            result.is_err(),
            "stroke=\"none\" must count as missing stroke in DualLayer"
        );
    }

    // ── Multiple text elements — all must pass ────────────────────────────────

    #[test]
    fn dual_layer_all_text_elements_must_pass() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect data-role="backdrop" fill="#000000" width="200" height="50"/>
            <text data-role="text" fill="#FFFFFF" stroke="#000000">Good</text>
            <text data-role="text" fill="#FFFFFF">Missing stroke here</text>
        </svg>"##;
        let result = check_svg_readability(svg, SvgReadabilityTechnique::DualLayer);
        assert!(result.is_err(), "any text failing DualLayer check should fail the whole SVG");
    }
}
