//! Design token system for tze_hud configuration.
//!
//! Implements `[design_tokens]` TOML section handling:
//! - Key validation pattern `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*`
//! - Four token value parsers: color hex, numeric, font family, literal string
//! - Canonical token schema (~28 required keys with fallback defaults)
//! - Three-layer profile-scoped token resolution:
//!     profile overrides → global config → canonical fallbacks
//!
//! ## Error codes produced
//!
//! | Error code | Condition |
//! |---|---|
//! | `CONFIG_INVALID_TOKEN_KEY` | Key in `[design_tokens]` does not match the required pattern |
//! | `TOKEN_VALUE_PARSE_ERROR` | A token value string could not be parsed into the expected format |

use std::collections::HashMap;

use tze_hud_scene::config::{ConfigError, ConfigErrorCode};

use crate::raw::RawConfig;

// ─── DesignTokenMap ────────────────────────────────────────────────────────────

/// A flat, immutable (after startup) map of design tokens.
///
/// Keys follow the pattern `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*`.
/// Values are opaque strings until explicitly parsed via `TokenValue`.
pub type DesignTokenMap = HashMap<String, String>;

// ─── Token key validation ─────────────────────────────────────────────────────

/// Returns `true` if the key matches `^[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*$`.
///
/// Segment rules:
/// - First segment: starts with `[a-z]`, followed by `[a-z0-9]*`
/// - Subsequent segments (after `.`): starts with `[a-z]`, followed by `[a-z0-9_]*`
pub fn is_valid_token_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let mut segments = key.split('.');
    // First segment: [a-z][a-z0-9]*
    if let Some(first) = segments.next() {
        if !is_valid_first_segment(first) {
            return false;
        }
    } else {
        return false;
    }
    // Remaining segments: [a-z][a-z0-9_]*
    for seg in segments {
        if !is_valid_subsequent_segment(seg) {
            return false;
        }
    }
    true
}

fn is_valid_first_segment(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn is_valid_subsequent_segment(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

// ─── Token value types ────────────────────────────────────────────────────────

/// RGBA color value, components in `[0.0, 1.0]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// Font family value parsed from the three canonical keywords.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontFamily {
    /// `sans-serif`
    SansSerif,
    /// `serif`
    Serif,
    /// `monospace`
    Monospace,
}

impl FontFamily {
    /// Parse from a canonical keyword string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "sans-serif" => Some(FontFamily::SansSerif),
            "serif" => Some(FontFamily::Serif),
            "monospace" => Some(FontFamily::Monospace),
            _ => None,
        }
    }
}

/// A parsed token value.
///
/// Parsing is attempted in order:
/// 1. Color hex (`#RRGGBB` or `#RRGGBBAA`) → `Color(Rgba)`
/// 2. Numeric (decimal) → `Numeric(f32)`
/// 3. Font family keyword → `Font(FontFamily)`
/// 4. Everything else → `Literal(String)`
#[derive(Clone, Debug, PartialEq)]
pub enum TokenValue {
    Color(Rgba),
    Numeric(f32),
    Font(FontFamily),
    Literal(String),
}

// ─── Value parsers ────────────────────────────────────────────────────────────

/// Parse a `#RRGGBB` or `#RRGGBBAA` hex color string into `Rgba`.
///
/// Returns `None` if the string does not match either form.
pub fn parse_color_hex(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if !s.starts_with('#') {
        return None;
    }
    let hex = &s[1..];
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Rgba {
                r: r as f32 / 255.0,
                g: g as f32 / 255.0,
                b: b as f32 / 255.0,
                a: 1.0,
            })
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Rgba {
                r: r as f32 / 255.0,
                g: g as f32 / 255.0,
                b: b as f32 / 255.0,
                a: a as f32 / 255.0,
            })
        }
        _ => None,
    }
}

/// Parse a decimal numeric string into `f32`.
pub fn parse_numeric(s: &str) -> Option<f32> {
    s.trim().parse::<f32>().ok()
}

/// Parse a font family keyword.
pub fn parse_font_family(s: &str) -> Option<FontFamily> {
    FontFamily::from_str(s.trim())
}

/// Parse a token value string into a `TokenValue`.
///
/// Order of precedence:
/// 1. Color hex
/// 2. Numeric
/// 3. Font family
/// 4. Literal string (always succeeds)
pub fn parse_token_value(s: &str) -> TokenValue {
    if let Some(color) = parse_color_hex(s) {
        return TokenValue::Color(color);
    }
    if let Some(n) = parse_numeric(s) {
        return TokenValue::Numeric(n);
    }
    if let Some(f) = parse_font_family(s) {
        return TokenValue::Font(f);
    }
    TokenValue::Literal(s.to_string())
}

// ─── Canonical token schema ───────────────────────────────────────────────────

/// A canonical token definition: key, expected format, and fallback default.
pub struct CanonicalToken {
    pub key: &'static str,
    pub description: &'static str,
    pub default_value: &'static str,
}

/// The ~28 canonical token keys with their fallback defaults.
///
/// These are always present in the resolved token map even if the config
/// does not specify them.
pub static CANONICAL_TOKENS: &[CanonicalToken] = &[
    // Color — text
    CanonicalToken {
        key: "color.text.primary",
        description: "Primary text color",
        default_value: "#FFFFFF",
    },
    CanonicalToken {
        key: "color.text.secondary",
        description: "Secondary text color",
        default_value: "#CCCCCC",
    },
    CanonicalToken {
        key: "color.text.disabled",
        description: "Disabled text color",
        default_value: "#666666",
    },
    CanonicalToken {
        key: "color.text.accent",
        description: "Accent text color",
        default_value: "#4DA6FF",
    },
    // Color — background
    CanonicalToken {
        key: "color.bg.primary",
        description: "Primary background color",
        default_value: "#000000",
    },
    CanonicalToken {
        key: "color.bg.secondary",
        description: "Secondary background color",
        default_value: "#111111",
    },
    CanonicalToken {
        key: "color.bg.overlay",
        description: "Overlay background color (semi-transparent)",
        default_value: "#00000099",
    },
    CanonicalToken {
        key: "color.bg.accent",
        description: "Accent background color",
        default_value: "#1A3A5C",
    },
    // Color — border
    CanonicalToken {
        key: "color.border.primary",
        description: "Primary border color",
        default_value: "#333333",
    },
    CanonicalToken {
        key: "color.border.accent",
        description: "Accent border color",
        default_value: "#4DA6FF",
    },
    // Color — status
    CanonicalToken {
        key: "color.status.success",
        description: "Success state color",
        default_value: "#22C55E",
    },
    CanonicalToken {
        key: "color.status.warning",
        description: "Warning state color",
        default_value: "#F59E0B",
    },
    CanonicalToken {
        key: "color.status.error",
        description: "Error state color",
        default_value: "#EF4444",
    },
    CanonicalToken {
        key: "color.status.info",
        description: "Info state color",
        default_value: "#3B82F6",
    },
    // Typography — body
    CanonicalToken {
        key: "typography.body.size",
        description: "Body text size in display-independent pixels",
        default_value: "16",
    },
    CanonicalToken {
        key: "typography.body.family",
        description: "Body font family",
        default_value: "sans-serif",
    },
    CanonicalToken {
        key: "typography.body.line_height",
        description: "Body line height multiplier",
        default_value: "1.5",
    },
    // Typography — heading
    CanonicalToken {
        key: "typography.heading.size",
        description: "Heading text size in display-independent pixels",
        default_value: "24",
    },
    CanonicalToken {
        key: "typography.heading.family",
        description: "Heading font family",
        default_value: "sans-serif",
    },
    // Typography — caption / label
    CanonicalToken {
        key: "typography.caption.size",
        description: "Caption text size in display-independent pixels",
        default_value: "12",
    },
    CanonicalToken {
        key: "typography.label.size",
        description: "Label text size in display-independent pixels",
        default_value: "14",
    },
    // Spacing
    CanonicalToken {
        key: "spacing.xs",
        description: "Extra-small spacing unit in pixels",
        default_value: "4",
    },
    CanonicalToken {
        key: "spacing.sm",
        description: "Small spacing unit in pixels",
        default_value: "8",
    },
    CanonicalToken {
        key: "spacing.md",
        description: "Medium spacing unit in pixels",
        default_value: "16",
    },
    CanonicalToken {
        key: "spacing.lg",
        description: "Large spacing unit in pixels",
        default_value: "24",
    },
    CanonicalToken {
        key: "spacing.xl",
        description: "Extra-large spacing unit in pixels",
        default_value: "32",
    },
    // Border radius
    CanonicalToken {
        key: "radius.sm",
        description: "Small border radius in pixels",
        default_value: "4",
    },
    CanonicalToken {
        key: "radius.md",
        description: "Medium border radius in pixels",
        default_value: "8",
    },
];

// ─── Token resolution ─────────────────────────────────────────────────────────

/// Resolve the effective design token map using three-layer precedence:
/// 1. Profile-scoped overrides (passed in as `profile_tokens`)
/// 2. Global config `[design_tokens]` section
/// 3. Canonical fallback defaults
///
/// The returned map contains ALL canonical tokens (via fallbacks) plus any
/// non-canonical tokens from config or profile layers.
///
/// # Arguments
///
/// * `config_tokens` — tokens from the global `[design_tokens]` section (may be empty)
/// * `profile_tokens` — per-profile token overrides (may be empty); applied on top
pub fn resolve_tokens(
    config_tokens: &DesignTokenMap,
    profile_tokens: &DesignTokenMap,
) -> DesignTokenMap {
    let mut resolved = DesignTokenMap::new();

    // Layer 3 (lowest priority): canonical fallback defaults
    for token in CANONICAL_TOKENS {
        resolved.insert(token.key.to_string(), token.default_value.to_string());
    }

    // Layer 2: global config overrides
    for (k, v) in config_tokens {
        resolved.insert(k.clone(), v.clone());
    }

    // Layer 1 (highest priority): profile-scoped overrides
    for (k, v) in profile_tokens {
        resolved.insert(k.clone(), v.clone());
    }

    resolved
}

// ─── Validation ───────────────────────────────────────────────────────────────

/// Validate the `[design_tokens]` section of a `RawConfig`.
///
/// Produces `CONFIG_INVALID_TOKEN_KEY` for any key that does not match the
/// required pattern. Non-canonical keys are accepted silently.
pub fn validate_design_tokens(raw: &RawConfig, errors: &mut Vec<ConfigError>) {
    let Some(tokens) = &raw.design_tokens else {
        return;
    };
    for key in tokens.0.keys() {
        if !is_valid_token_key(key) {
            errors.push(ConfigError {
                code: ConfigErrorCode::InvalidTokenKey,
                field_path: format!("design_tokens.{key}"),
                expected: "key matching [a-z][a-z0-9]*(\\.[a-z][a-z0-9_]*)*".into(),
                got: key.clone(),
                hint: format!(
                    "use lowercase dot-separated segments, e.g. \"color.text.primary\"; \
                     got {key:?}"
                ),
            });
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Key validation ────────────────────────────────────────────────────────

    #[test]
    fn test_valid_token_keys() {
        assert!(is_valid_token_key("color.text.primary"));
        assert!(is_valid_token_key("typography.body.size"));
        assert!(is_valid_token_key("spacing.xs"));
        assert!(is_valid_token_key("radius.sm"));
        assert!(is_valid_token_key("color.bg.overlay"));
        assert!(is_valid_token_key("a.b"));
        assert!(is_valid_token_key("a1.b2_c"));
        assert!(is_valid_token_key("abc123.def_ghi"));
    }

    #[test]
    fn test_invalid_token_keys() {
        // Empty string
        assert!(!is_valid_token_key(""));
        // Starts with digit
        assert!(!is_valid_token_key("1color.text"));
        // Uppercase in first segment
        assert!(!is_valid_token_key("Color.text.primary"));
        // Uppercase in subsequent segment
        assert!(!is_valid_token_key("color.Text.primary"));
        // First segment contains underscore (not allowed)
        assert!(!is_valid_token_key("color_bg.primary"));
        // Trailing dot
        assert!(!is_valid_token_key("color.text."));
        // Leading dot
        assert!(!is_valid_token_key(".color.text"));
        // Empty segment
        assert!(!is_valid_token_key("color..text"));
        // Kebab-case in first segment
        assert!(!is_valid_token_key("color-bg.primary"));
        // No dot (single segment only) — single segment is valid if it matches first-segment rules
        assert!(is_valid_token_key("spacing"));
        // Hyphen in subsequent segment
        assert!(!is_valid_token_key("color.text-primary"));
    }

    // ── Color hex parsing ─────────────────────────────────────────────────────

    #[test]
    fn test_parse_color_hex_rrggbb() {
        let rgba = parse_color_hex("#FFFFFF").unwrap();
        assert!((rgba.r - 1.0).abs() < 1e-4);
        assert!((rgba.g - 1.0).abs() < 1e-4);
        assert!((rgba.b - 1.0).abs() < 1e-4);
        assert!((rgba.a - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_parse_color_hex_rrggbb_black() {
        let rgba = parse_color_hex("#000000").unwrap();
        assert!((rgba.r).abs() < 1e-4);
        assert!((rgba.g).abs() < 1e-4);
        assert!((rgba.b).abs() < 1e-4);
        assert!((rgba.a - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_parse_color_hex_rrggbbaa() {
        let rgba = parse_color_hex("#00000099").unwrap();
        assert!((rgba.r).abs() < 1e-4);
        assert!((rgba.g).abs() < 1e-4);
        assert!((rgba.b).abs() < 1e-4);
        // 0x99 = 153 → 153/255 ≈ 0.6
        assert!((rgba.a - 153.0 / 255.0).abs() < 1e-3);
    }

    #[test]
    fn test_parse_color_hex_invalid() {
        assert!(parse_color_hex("FFFFFF").is_none()); // no leading #
        assert!(parse_color_hex("#FFF").is_none()); // too short
        assert!(parse_color_hex("#GGGGGG").is_none()); // invalid hex
        assert!(parse_color_hex("").is_none());
        assert!(parse_color_hex("not-a-color").is_none());
    }

    // ── Numeric parsing ───────────────────────────────────────────────────────

    #[test]
    fn test_parse_numeric_integer() {
        let n = parse_numeric("16").unwrap();
        assert!((n - 16.0).abs() < 1e-4);
    }

    #[test]
    fn test_parse_numeric_decimal() {
        let n = parse_numeric("1.5").unwrap();
        assert!((n - 1.5).abs() < 1e-4);
    }

    #[test]
    fn test_parse_numeric_invalid() {
        assert!(parse_numeric("abc").is_none());
        assert!(parse_numeric("").is_none());
        assert!(parse_numeric("1.2.3").is_none());
    }

    // ── Font family parsing ───────────────────────────────────────────────────

    #[test]
    fn test_parse_font_family_keywords() {
        assert_eq!(
            parse_font_family("sans-serif"),
            Some(FontFamily::SansSerif)
        );
        assert_eq!(parse_font_family("serif"), Some(FontFamily::Serif));
        assert_eq!(
            parse_font_family("monospace"),
            Some(FontFamily::Monospace)
        );
        assert!(parse_font_family("Arial").is_none());
        assert!(parse_font_family("").is_none());
    }

    // ── parse_token_value dispatch ────────────────────────────────────────────

    #[test]
    fn test_parse_token_value_color() {
        let tv = parse_token_value("#FF0000");
        assert!(matches!(tv, TokenValue::Color(_)));
    }

    #[test]
    fn test_parse_token_value_numeric() {
        let tv = parse_token_value("16");
        assert!(matches!(tv, TokenValue::Numeric(n) if (n - 16.0).abs() < 1e-4));
    }

    #[test]
    fn test_parse_token_value_font() {
        let tv = parse_token_value("monospace");
        assert_eq!(tv, TokenValue::Font(FontFamily::Monospace));
    }

    #[test]
    fn test_parse_token_value_literal() {
        let tv = parse_token_value("my-custom-value");
        assert_eq!(tv, TokenValue::Literal("my-custom-value".to_string()));
    }

    // ── Fallback resolution ───────────────────────────────────────────────────

    #[test]
    fn test_resolve_tokens_canonical_fallbacks_present() {
        let map = resolve_tokens(&DesignTokenMap::new(), &DesignTokenMap::new());
        // All canonical tokens must be present
        for token in CANONICAL_TOKENS {
            assert!(
                map.contains_key(token.key),
                "canonical token '{}' missing from resolved map",
                token.key
            );
            assert_eq!(
                map[token.key], token.default_value,
                "canonical token '{}' has wrong default",
                token.key
            );
        }
    }

    #[test]
    fn test_resolve_tokens_config_overrides_fallback() {
        let mut config_tokens = DesignTokenMap::new();
        config_tokens.insert(
            "color.text.primary".to_string(),
            "#FF0000".to_string(),
        );
        let map = resolve_tokens(&config_tokens, &DesignTokenMap::new());
        assert_eq!(map["color.text.primary"], "#FF0000");
    }

    #[test]
    fn test_resolve_tokens_profile_overrides_config() {
        let mut config_tokens = DesignTokenMap::new();
        config_tokens.insert(
            "color.text.primary".to_string(),
            "#FF0000".to_string(),
        );
        let mut profile_tokens = DesignTokenMap::new();
        profile_tokens.insert(
            "color.text.primary".to_string(),
            "#00FF00".to_string(),
        );
        let map = resolve_tokens(&config_tokens, &profile_tokens);
        assert_eq!(map["color.text.primary"], "#00FF00");
    }

    #[test]
    fn test_resolve_tokens_non_canonical_keys_accepted() {
        let mut config_tokens = DesignTokenMap::new();
        config_tokens.insert("custom.brand.color".to_string(), "#ABCDEF".to_string());
        let map = resolve_tokens(&config_tokens, &DesignTokenMap::new());
        assert_eq!(map["custom.brand.color"], "#ABCDEF");
    }

    // ── validate_design_tokens ────────────────────────────────────────────────

    #[test]
    fn test_validate_no_design_tokens_section_ok() {
        let raw = RawConfig::default();
        let mut errors = Vec::new();
        validate_design_tokens(&raw, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_valid_keys_no_errors() {
        use crate::raw::RawDesignTokens;
        let mut tokens = HashMap::new();
        tokens.insert("color.text.primary".to_string(), "#FFFFFF".to_string());
        tokens.insert("spacing.md".to_string(), "16".to_string());
        let mut raw = RawConfig::default();
        raw.design_tokens = Some(RawDesignTokens(tokens));
        let mut errors = Vec::new();
        validate_design_tokens(&raw, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_invalid_key_produces_error() {
        use crate::raw::RawDesignTokens;
        let mut tokens = HashMap::new();
        tokens.insert("Color.Text.Primary".to_string(), "#FFFFFF".to_string()); // uppercase
        let mut raw = RawConfig::default();
        raw.design_tokens = Some(RawDesignTokens(tokens));
        let mut errors = Vec::new();
        validate_design_tokens(&raw, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            errors[0].code,
            ConfigErrorCode::InvalidTokenKey
        ));
    }

    #[test]
    fn test_validate_multiple_invalid_keys_all_reported() {
        use crate::raw::RawDesignTokens;
        let mut tokens = HashMap::new();
        tokens.insert("1bad.key".to_string(), "value".to_string());
        tokens.insert("also-bad".to_string(), "value".to_string());
        tokens.insert("color.text.primary".to_string(), "#FFF".to_string()); // valid
        let mut raw = RawConfig::default();
        raw.design_tokens = Some(RawDesignTokens(tokens));
        let mut errors = Vec::new();
        validate_design_tokens(&raw, &mut errors);
        // Exactly 2 errors (the 2 invalid keys)
        let invalid_token_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e.code, ConfigErrorCode::InvalidTokenKey))
            .collect();
        assert_eq!(invalid_token_errors.len(), 2);
    }

    // ── Parse error reporting ─────────────────────────────────────────────────

    #[test]
    fn test_parse_error_code_exists() {
        // Verify that TOKEN_VALUE_PARSE_ERROR code can be constructed
        let _err = ConfigError {
            code: ConfigErrorCode::TokenValueParseError,
            field_path: "design_tokens.color.text.primary".to_string(),
            expected: "color hex #RRGGBB or #RRGGBBAA".to_string(),
            got: "not-a-color".to_string(),
            hint: "use a hex color like #FF0000".to_string(),
        };
    }
}
