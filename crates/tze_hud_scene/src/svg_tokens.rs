// ─── Token placeholder resolution ────────────────────────────────────────────

use std::collections::HashMap;

/// Resolve token mustache-style placeholders in SVG text.
///
/// # Syntax
///
/// Two equivalent placeholder forms are accepted:
///
/// - **Bare form** (preferred, per spec): `{{key}}` where `key` is a dotted
///   token path matching `[a-z][a-z0-9]*(?:\.[a-z][a-z0-9_]*)*`.  The key is
///   looked up directly in the token map.  Example: `{{color.text.primary}}`
///   looks up the key `color.text.primary`.
/// - **Prefixed form** (legacy, backward-compatible): `{{token.key}}` where
///   `key` matches the same pattern.  The `token.` prefix is stripped before
///   the map lookup, so `{{token.color.primary}}` also looks up `color.primary`.
///
/// Both forms perform no whitespace inside the braces and a single
/// left-to-right pass with no recursive re-scanning of substituted values.
///
/// # Escape sequences
///
/// Literal `{{` and `}}` can be written as `\{\{` / `\}\}` in the SVG source
/// (each brace individually backslash-escaped, per the spec).  These are
/// replaced with sentinels before scanning and restored afterwards, ensuring
/// they are never treated as placeholder delimiters.
///
/// # Errors
///
/// Returns `Err(token_key)` if a valid-syntax placeholder references a key not
/// present in `tokens`.  Unknown-syntax sequences (e.g. whitespace inside
/// braces) are passed through unchanged and never produce an error.
///
/// # Guarantees
///
/// - Single left-to-right pass: resolved substitution values are never
///   re-scanned for further placeholders.
/// - Placeholders in `<style>` blocks are resolved identically to any other
///   text content.
/// - UTF-8 safe: uses string-level `find` for all scanning; no raw byte casts.
pub fn resolve_token_placeholders(
    svg_text: &str,
    tokens: &HashMap<String, String>,
) -> Result<String, String> {
    // Fast path: most SVGs have no placeholders or escape sequences.
    // Skipping both checks avoids any allocation for the common case.
    if !svg_text.contains("{{") && !svg_text.contains("\\{\\{") {
        return Ok(svg_text.to_string());
    }

    // Sentinel strings that cannot appear in valid SVG/XML.
    const ESC_OPEN: &str = "\x00LBRACE\x00";
    const ESC_CLOSE: &str = "\x00RBRACE\x00";

    // Step 1: Replace escape sequences with sentinels.
    // The spec escape format is \{\{ / \}\} (each brace individually escaped).
    let work = svg_text
        .replace("\\{\\{", ESC_OPEN)
        .replace("\\}\\}", ESC_CLOSE);

    // Step 2: Single left-to-right scan using string-level find().
    // This is UTF-8 safe: `find` returns byte positions at character
    // boundaries; we only slice at those positions.
    //
    // XML comments (`<!-- ... -->`) are skipped verbatim: placeholders inside
    // comment regions are NOT resolved.  This allows SVG authors to include
    // documentation examples such as `<!-- use {{token.color.primary}} here -->`
    // without triggering resolution or spurious unresolved-token errors.
    let mut result = String::with_capacity(work.len());
    let mut remaining = work.as_str();
    // Once no `<!--` remains in `remaining`, no further comment checks are
    // needed.  This flag avoids the O(N·L) cost of re-scanning for `<!--` on
    // every placeholder iteration in comment-free inputs.
    let mut may_have_comments = remaining.contains("<!--");

    while let Some(open_pos) = remaining.find("{{") {
        // Before treating `{{` as a placeholder, check whether there is an XML
        // comment start (`<!--`) that appears before the next `{{` scan
        // position.  If so, emit that comment region verbatim before
        // processing the placeholder scan position.
        if may_have_comments {
            if let Some(comment_start) = remaining.find("<!--") {
                if comment_start < open_pos {
                    // A comment opens before the `{{`.  Emit everything up to
                    // and including the comment end marker (`-->`), then
                    // continue.
                    let comment_body_start = comment_start + 4; // skip past `<!--`
                    let comment_suffix = &remaining[comment_body_start..];
                    if let Some(comment_end_offset) = comment_suffix.find("-->") {
                        // Emit the entire comment (including delimiters) unchanged.
                        let comment_end_abs = comment_body_start + comment_end_offset + 3;
                        result.push_str(&remaining[..comment_end_abs]);
                        remaining = &remaining[comment_end_abs..];
                        // Re-check whether any further comments remain.
                        may_have_comments = remaining.contains("<!--");
                    } else {
                        // Unclosed comment — emit the rest of the input verbatim.
                        result.push_str(remaining);
                        remaining = "";
                        may_have_comments = false;
                    }
                    continue;
                }
            } else {
                may_have_comments = false;
            }
        }

        // Append everything before the `{{`.
        result.push_str(&remaining[..open_pos]);
        let after_open = &remaining[open_pos + 2..];

        // Find the matching `}}`.
        if let Some(close_offset) = after_open.find("}}") {
            let inner = &after_open[..close_offset];

            // Resolve the placeholder key using two accepted forms:
            //   1. Prefixed form: `{{token.<key>}}` — strip "token." prefix.
            //      If the stripped part is a valid key, use it.  If the stripped
            //      part is NOT a valid key (e.g. contains an underscore in the first
            //      segment), the prefixed-form prefix has precedence and the whole
            //      `{{...}}` is treated as unrecognised (passed through).
            //   2. Bare form:     `{{<key>}}`        — use the inner text directly,
            //      only when the text does NOT start with "token." (i.e. is not
            //      mistaken for an attempted-but-malformed prefixed form).
            // Both forms require the key to pass `is_valid_token_key`.
            let key_part = if let Some(stripped) = inner.strip_prefix("token.") {
                // Prefixed form: validate the part after "token.".
                if is_valid_token_key(stripped) {
                    Some(stripped)
                } else {
                    None
                }
            } else if is_valid_token_key(inner) {
                // Bare form: inner text is the key directly.
                Some(inner)
            } else {
                None
            };

            if let Some(key_part) = key_part {
                // Resolve against the token map.
                match tokens.get(key_part) {
                    Some(value) => {
                        result.push_str(value);
                        remaining = &after_open[close_offset + 2..]; // skip past `}}`
                        continue;
                    }
                    None => {
                        // Unresolved token — return the key as the error.
                        return Err(key_part.to_string());
                    }
                }
            }
            // Inner text didn't match token syntax — pass `{{` through literally
            // and advance past just the `{{` so the inner text is re-scanned.
            result.push_str("{{");
            remaining = after_open;
        } else {
            // No closing `}}` found — pass `{{` through and stop scanning.
            result.push_str("{{");
            remaining = after_open;
        }
    }

    // Append any remaining text after the last `{{` (or the whole string if
    // no `{{` was found).
    result.push_str(remaining);

    // Step 3: Restore sentinels to their literal form.
    let result = result.replace(ESC_OPEN, "{{").replace(ESC_CLOSE, "}}");

    Ok(result)
}

/// Returns `true` if `key` matches `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*`.
///
/// Pattern breakdown:
/// - First segment: `[a-z][a-z0-9]*` — lowercase letter followed by lowercase
///   letters and digits only (no underscores).
/// - Each additional segment: `[a-z][a-z0-9_]*` — lowercase letter followed by
///   lowercase letters, digits, and underscores.
///
/// This is the allowed token key syntax for both the bare `{{key}}` form and
/// the prefixed `{{token.<key>}}` form of SVG token placeholders.
pub fn is_valid_token_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    for (i, segment) in key.split('.').enumerate() {
        let mut chars = segment.chars();
        // Each segment must start with a lowercase ASCII letter.
        if !matches!(chars.next(), Some(c) if c.is_ascii_lowercase()) {
            return false;
        }
        // Remaining chars: letters or digits for all segments;
        // underscores additionally allowed in non-first segments.
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || (i > 0 && c == '_')) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{is_valid_token_key, resolve_token_placeholders};
    use std::collections::HashMap;

    fn token_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn no_placeholders_passthrough() {
        let tokens = token_map(&[]);
        let input = r##"<rect fill="#B0B0B0"/>"##;
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn single_placeholder_substituted() {
        let tokens = token_map(&[("color.primary", "#ff0000")]);
        let input = r##"<rect fill="{{token.color.primary}}"/>"##;
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, r##"<rect fill="#ff0000"/>"##);
    }

    #[test]
    fn bare_form_substituted() {
        let tokens = token_map(&[("color.primary", "#ff0000")]);
        let input = r##"<rect fill="{{color.primary}}"/>"##;
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, r##"<rect fill="#ff0000"/>"##);
    }

    #[test]
    fn unresolved_token_returns_err() {
        let tokens = token_map(&[]);
        let input = r##"<rect fill="{{color.missing}}"/>"##;
        let err = resolve_token_placeholders(input, &tokens).unwrap_err();
        assert_eq!(err, "color.missing");
    }

    #[test]
    fn is_valid_token_key_basic() {
        assert!(is_valid_token_key("color"));
        assert!(is_valid_token_key("color.primary"));
        assert!(is_valid_token_key("color.text.primary"));
        assert!(!is_valid_token_key(""));
        assert!(!is_valid_token_key("Color"));
        assert!(!is_valid_token_key("color_primary")); // underscore in first segment
    }
}
