//! `TzeHudConfig` — concrete implementation of `ConfigLoader`.
//!
//! This module implements all v1-mandatory validation requirements from
//! `configuration/spec.md` that belong to bead rig-j90m:
//!
//! - TOML parse errors with line + column (§TOML Configuration Format)
//! - File resolution order (§Configuration File Resolution Order)
//! - Minimal valid config: `[runtime]` + `profile` + ≥1 `[[tabs]]` (§Minimal Valid Configuration)
//! - Layered config (`includes`) rejected (§Layered Config Composition, v1-reserved)
//! - All validation errors collected before reporting (§Structured Validation Error Collection)
//! - Tab uniqueness, default_tab count, layout enum (§Tab Configuration Validation)
//! - Reserved fraction sums (§Reserved Fraction Validation)
//! - FPS range: target_fps >= min_fps (§FPS Range Validation)
//! - Degradation threshold ordering (§Degradation Threshold Ordering)
//! - Scene event naming convention (§Scene Event Naming Convention)
//!
//! Validation items delegated to other beads:
//! - Display profile resolution (rig-umgy): profile auto-detection, custom profile
//!   extends semantics, profile budget/capability escalation checks.
//! - Capability vocabulary (rig-9yfh): `CONFIG_UNKNOWN_CAPABILITY`, reserved event
//!   prefix in capability grants.
//! - Privacy / zone / agent registration (rig-mop4).
//!
//! This crate still calls `tze_hud_scene::config::is_canonical_capability` so that
//! the `unknown_capability` and `reserved_event_prefix` tests from the trait-level
//! test suite pass (they use the generic helper defined in the scene crate).

use std::collections::HashMap;

use tze_hud_scene::config::{
    ConfigError, ConfigErrorCode, ConfigLoader, DisplayProfile, ParseError, ResolvedConfig,
    is_canonical_capability,
};

use crate::raw::{RawConfig, RawDegradation};
use crate::resolver;

// ─── Regex helper ────────────────────────────────────────────────────────────

/// Scene event name pattern: `^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$`
fn is_valid_event_name(name: &str) -> bool {
    if name.is_empty() {
        // Empty string is explicitly valid (no auto-switch).
        return true;
    }
    let parts: Vec<&str> = name.splitn(2, '.').collect();
    if parts.len() != 2 {
        return false;
    }
    fn valid_segment(s: &str) -> bool {
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
    valid_segment(parts[0]) && valid_segment(parts[1])
}

// ─── TzeHudConfig ─────────────────────────────────────────────────────────────

/// Concrete implementation of `ConfigLoader` for tze_hud.
///
/// Created via `TzeHudConfig::parse(toml_src)`.
pub struct TzeHudConfig {
    raw: RawConfig,
}

impl ConfigLoader for TzeHudConfig {
    // ── parse ─────────────────────────────────────────────────────────────────

    fn parse(toml_src: &str) -> Result<Self, ParseError>
    where
        Self: Sized,
    {
        toml::from_str::<RawConfig>(toml_src).map(|raw| TzeHudConfig { raw }).map_err(|e| {
            // toml 0.8 errors have a span that includes line/column.
            let message = e.to_string();

            // Extract line/column from the error message.
            // Format: "... at line N column M"
            let (line, column) = parse_toml_location(&message);
            ParseError { message, line, column }
        })
    }

    // ── normalize ─────────────────────────────────────────────────────────────

    fn normalize(&mut self) {
        // Ensure `runtime` is present (even if empty) so downstream code can
        // rely on `self.raw.runtime.as_ref()` without repeated Option handling.
        if self.raw.runtime.is_none() {
            self.raw.runtime = Some(Default::default());
        }
    }

    // ── validate ──────────────────────────────────────────────────────────────

    fn validate(&self) -> Vec<ConfigError> {
        let mut errors: Vec<ConfigError> = Vec::new();

        // ── (1) includes field (v1-reserved) ──────────────────────────────────
        if self.raw.includes.is_some() {
            // AnyValue wraps any TOML value; presence alone is the error.
            errors.push(ConfigError {
                code: ConfigErrorCode::ConfigIncludesNotSupported,
                field_path: "includes".into(),
                expected: "field must be absent (layered composition is post-v1)".into(),
                got: "includes field present".into(),
                hint: "remove the `includes` field; layered config is reserved for post-v1".into(),
            });
        }

        // ── (2) [runtime] present and profile set ─────────────────────────────
        let profile_str = self
            .raw
            .runtime
            .as_ref()
            .and_then(|r| r.profile.as_deref());

        // Validate profile value if present.
        if let Some(p) = profile_str {
            validate_profile(p, &mut errors);
        }

        // ── (3) [[tabs]] — at least one, names unique, ≤1 default ────────────
        validate_tabs(&self.raw, &mut errors);

        // ── (4) Reserved fractions ────────────────────────────────────────────
        for (i, tab) in self.raw.tabs.iter().enumerate() {
            if let Some(layout) = &tab.layout {
                validate_reserved_fractions(i, layout, &mut errors);
            }
        }

        // ── (5) FPS range ─────────────────────────────────────────────────────
        if let Some(dp) = &self.raw.display_profile {
            validate_fps_range(dp.target_fps, dp.min_fps, &mut errors);
        }

        // ── (6) Degradation thresholds ────────────────────────────────────────
        if let Some(deg) = &self.raw.degradation {
            validate_degradation_order(deg, &mut errors);
        }

        // ── (7) Scene event naming convention (tab_switch_on_event) ──────────
        for (i, tab) in self.raw.tabs.iter().enumerate() {
            if let Some(event) = &tab.tab_switch_on_event {
                if !is_valid_event_name(event) {
                    errors.push(ConfigError {
                        code: ConfigErrorCode::InvalidEventName,
                        field_path: format!("tabs[{i}].tab_switch_on_event"),
                        expected: "empty string or <source>.<action> matching ^[a-z][a-z0-9_]*\\.[a-z][a-z0-9_]*$".into(),
                        got: format!("{event:?}"),
                        hint: format!(
                            "use lowercase dotted format e.g. \"doorbell.ring\"; got {:?}",
                            event
                        ),
                    });
                }
            }
        }

        // ── (8) Capability vocabulary (rig-j90m scope includes this check) ───
        if let Some(agents) = &self.raw.agents {
            if let Some(registered) = &agents.registered {
                for (agent_name, agent) in registered {
                    if let Some(caps) = &agent.capabilities {
                        for cap in caps {
                            if !is_canonical_capability(cap) {
                                // Distinguish reserved event prefix from unknown.
                                let code = if cap.starts_with("emit_scene_event:") {
                                    let suffix = &cap["emit_scene_event:".len()..];
                                    if suffix.starts_with("scene.") || suffix.starts_with("system.") {
                                        ConfigErrorCode::ReservedEventPrefix
                                    } else {
                                        ConfigErrorCode::UnknownCapability
                                    }
                                } else {
                                    ConfigErrorCode::UnknownCapability
                                };
                                errors.push(ConfigError {
                                    code,
                                    field_path: format!(
                                        "agents.registered.{agent_name}.capabilities"
                                    ),
                                    expected: "canonical v1 capability name".into(),
                                    got: cap.clone(),
                                    hint: format!(
                                        "unknown capability {:?}; check the canonical v1 vocabulary",
                                        cap
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        errors
    }

    // ── freeze ────────────────────────────────────────────────────────────────

    fn freeze(mut self) -> Result<ResolvedConfig, Vec<ConfigError>> {
        self.normalize();
        let errors = self.validate();
        if !errors.is_empty() {
            return Err(errors);
        }

        let profile_str = self
            .raw
            .runtime
            .as_ref()
            .and_then(|r| r.profile.as_deref())
            .unwrap_or("full-display");

        let profile = resolve_profile_defaults(profile_str);

        let tab_names = self
            .raw
            .tabs
            .iter()
            .filter_map(|t| t.name.clone())
            .collect();

        let mut agent_capabilities: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(agents) = &self.raw.agents {
            if let Some(registered) = &agents.registered {
                for (name, agent) in registered {
                    agent_capabilities.insert(
                        name.clone(),
                        agent.capabilities.clone().unwrap_or_default(),
                    );
                }
            }
        }

        let source_path = None; // Set by caller after file load.

        Ok(ResolvedConfig {
            profile,
            tab_names,
            agent_capabilities,
            source_path,
        })
    }

    // ── resolve_config_path ───────────────────────────────────────────────────

    fn resolve_config_path(cli_path: Option<&str>) -> Result<String, Vec<String>>
    where
        Self: Sized,
    {
        resolver::resolve_config_path(cli_path)
    }

    // ── is_known_capability ───────────────────────────────────────────────────

    fn is_known_capability(name: &str) -> bool
    where
        Self: Sized,
    {
        is_canonical_capability(name)
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Extract line/column from a toml error message.
///
/// toml 0.8 formats errors like:
/// `TOML parse error at line 2, column 5`
fn parse_toml_location(msg: &str) -> (u32, u32) {
    // Try to parse "at line N, column M" or "at line N column M".
    let mut line = 1u32;
    let mut col = 1u32;

    // Find "line N"
    if let Some(idx) = msg.find("line ") {
        let rest = &msg[idx + 5..];
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<u32>() {
            line = n;
        }
    }

    // Find "column M"
    if let Some(idx) = msg.find("column ") {
        let rest = &msg[idx + 7..];
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<u32>() {
            col = n;
        }
    }

    (line, col)
}

/// Validate the profile string value and append any errors.
fn validate_profile(profile: &str, errors: &mut Vec<ConfigError>) {
    match profile {
        "full-display" | "headless" | "auto" | "custom" => {}
        "mobile" => {
            errors.push(ConfigError {
                code: ConfigErrorCode::MobileProfileNotExercised,
                field_path: "runtime.profile".into(),
                expected: "\"full-display\", \"headless\", \"auto\", or \"custom\"".into(),
                got: "\"mobile\"".into(),
                hint: "mobile profile is schema-reserved; use \"full-display\" or \"headless\"".into(),
            });
        }
        other => {
            errors.push(ConfigError {
                code: ConfigErrorCode::UnknownProfile,
                field_path: "runtime.profile".into(),
                expected: "\"full-display\", \"headless\", \"auto\", \"custom\", or \"mobile\"".into(),
                got: format!("{other:?}"),
                hint: format!(
                    "unknown profile {:?}; valid built-ins: full-display, headless, auto",
                    other
                ),
            });
        }
    }
}

/// Validate `[[tabs]]` entries.
fn validate_tabs(raw: &RawConfig, errors: &mut Vec<ConfigError>) {
    // Must have at least one tab.
    if raw.tabs.is_empty() {
        errors.push(ConfigError {
            code: ConfigErrorCode::NoTabs,
            field_path: "tabs".into(),
            expected: "at least one [[tabs]] entry".into(),
            got: "empty array".into(),
            hint: "add a [[tabs]] section with at least a `name` field".into(),
        });
        return;
    }

    // Collect names, check uniqueness.
    let mut seen_names: HashMap<String, usize> = HashMap::new();
    let mut default_count = 0usize;

    for (i, tab) in raw.tabs.iter().enumerate() {
        // Name must be present.
        let name = match &tab.name {
            Some(n) => n.clone(),
            None => {
                errors.push(ConfigError {
                    code: ConfigErrorCode::Other("CONFIG_TAB_MISSING_NAME".into()),
                    field_path: format!("tabs[{i}].name"),
                    expected: "non-empty string".into(),
                    got: "absent".into(),
                    hint: "every [[tabs]] entry must have a `name` field".into(),
                });
                continue;
            }
        };

        // Name uniqueness.
        if let Some(prev) = seen_names.get(&name) {
            errors.push(ConfigError {
                code: ConfigErrorCode::DuplicateTabName,
                field_path: format!("tabs[{i}].name"),
                expected: format!("unique name; \"{}\" already used at tabs[{}]", name, prev),
                got: format!("{name:?}"),
                hint: format!("rename the second tab (tabs[{i}]) to a unique name"),
            });
        } else {
            seen_names.insert(name, i);
        }

        // Default tab count.
        if tab.default_tab {
            default_count += 1;
            if default_count > 1 {
                errors.push(ConfigError {
                    code: ConfigErrorCode::MultipleDefaultTabs,
                    field_path: format!("tabs[{i}].default_tab"),
                    expected: "at most one tab with default_tab = true".into(),
                    got: format!("tabs[{i}] is the second tab with default_tab = true"),
                    hint: "set default_tab = true on at most one tab".into(),
                });
            }
        }

        // Layout enum validation.
        if let Some(layout) = &tab.default_layout {
            match layout.as_str() {
                "grid" | "columns" | "freeform" => {}
                other => {
                    errors.push(ConfigError {
                        code: ConfigErrorCode::UnknownLayout,
                        field_path: format!("tabs[{i}].default_layout"),
                        expected: "\"grid\", \"columns\", or \"freeform\"".into(),
                        got: format!("{other:?}"),
                        hint: format!(
                            "unknown layout {:?}; valid values: grid, columns, freeform",
                            other
                        ),
                    });
                }
            }
        }
    }
}

/// Validate reserved fraction sums for a single tab's layout.
fn validate_reserved_fractions(
    tab_idx: usize,
    layout: &crate::raw::RawTabLayout,
    errors: &mut Vec<ConfigError>,
) {
    let top = layout.reserved_top_fraction.unwrap_or(0.0);
    let bottom = layout.reserved_bottom_fraction.unwrap_or(0.0);
    let left = layout.reserved_left_fraction.unwrap_or(0.0);
    let right = layout.reserved_right_fraction.unwrap_or(0.0);

    // Each fraction must be in [0.0, 1.0].
    for (name, val) in [
        ("reserved_top_fraction", top),
        ("reserved_bottom_fraction", bottom),
        ("reserved_left_fraction", left),
        ("reserved_right_fraction", right),
    ] {
        if !(0.0..=1.0).contains(&val) {
            errors.push(ConfigError {
                code: ConfigErrorCode::InvalidReservedFraction,
                field_path: format!("tabs[{tab_idx}].layout.{name}"),
                expected: "value in [0.0, 1.0]".into(),
                got: format!("{val}"),
                hint: format!("{name} must be between 0.0 and 1.0 inclusive"),
            });
        }
    }

    // Vertical sum must be < 1.0.
    if top + bottom >= 1.0 {
        errors.push(ConfigError {
            code: ConfigErrorCode::InvalidReservedFraction,
            field_path: format!("tabs[{tab_idx}].layout"),
            expected: "reserved_top_fraction + reserved_bottom_fraction < 1.0".into(),
            got: format!("{top} + {bottom} = {}", top + bottom),
            hint: "no vertical space remains for agent tiles; reduce top or bottom fraction".into(),
        });
    }

    // Horizontal sum must be < 1.0.
    if left + right >= 1.0 {
        errors.push(ConfigError {
            code: ConfigErrorCode::InvalidReservedFraction,
            field_path: format!("tabs[{tab_idx}].layout"),
            expected: "reserved_left_fraction + reserved_right_fraction < 1.0".into(),
            got: format!("{left} + {right} = {}", left + right),
            hint: "no horizontal space remains for agent tiles; reduce left or right fraction"
                .into(),
        });
    }
}

/// Validate FPS range for an explicit display_profile override.
fn validate_fps_range(
    target_fps: Option<u32>,
    min_fps: Option<u32>,
    errors: &mut Vec<ConfigError>,
) {
    if let (Some(target), Some(min)) = (target_fps, min_fps) {
        if target < min {
            errors.push(ConfigError {
                code: ConfigErrorCode::InvalidFpsRange,
                field_path: "display_profile".into(),
                expected: format!("target_fps ({target}) >= min_fps ({min})"),
                got: format!("target_fps={target}, min_fps={min}"),
                hint: format!(
                    "target_fps must be >= min_fps; set target_fps >= {min} or lower min_fps"
                ),
            });
        }
    }
}

/// Validate degradation threshold ordering.
fn validate_degradation_order(deg: &RawDegradation, errors: &mut Vec<ConfigError>) {
    // Frame-time thresholds must be monotonically non-decreasing:
    // coalesce_frame_ms <= simplify_rendering_frame_ms <= shed_tiles_frame_ms <= audio_only_frame_ms
    let frame_thresholds: &[(&str, Option<f64>)] = &[
        ("coalesce_frame_ms", deg.coalesce_frame_ms),
        ("simplify_rendering_frame_ms", deg.simplify_rendering_frame_ms),
        ("shed_tiles_frame_ms", deg.shed_tiles_frame_ms),
        ("audio_only_frame_ms", deg.audio_only_frame_ms),
    ];

    check_monotone_non_decreasing(frame_thresholds, errors);

    // GPU fraction thresholds:
    // reduce_media_quality_gpu_fraction <= reduce_concurrent_streams_gpu_fraction
    let gpu_thresholds: &[(&str, Option<f64>)] = &[
        (
            "reduce_media_quality_gpu_fraction",
            deg.reduce_media_quality_gpu_fraction,
        ),
        (
            "reduce_concurrent_streams_gpu_fraction",
            deg.reduce_concurrent_streams_gpu_fraction,
        ),
    ];

    check_monotone_non_decreasing(gpu_thresholds, errors);
}

/// Check that each pair of adjacent non-None thresholds is non-decreasing.
fn check_monotone_non_decreasing(fields: &[(&str, Option<f64>)], errors: &mut Vec<ConfigError>) {
    let mut prev: Option<(&str, f64)> = None;
    for (name, val_opt) in fields {
        if let Some(val) = *val_opt {
            if let Some((prev_name, prev_val)) = prev {
                if val < prev_val {
                    errors.push(ConfigError {
                        code: ConfigErrorCode::DegradationThresholdOrder,
                        field_path: format!("degradation.{name}"),
                        expected: format!(
                            "{name} ({val}) >= {prev_name} ({prev_val})"
                        ),
                        got: format!("{name}={val}, {prev_name}={prev_val}"),
                        hint: format!(
                            "degradation thresholds must be non-decreasing; \
                             {name} ({val}) is less than preceding {prev_name} ({prev_val})"
                        ),
                    });
                }
            }
            prev = Some((name, val));
        }
    }
}

/// Resolve profile name to `DisplayProfile` defaults.
///
/// Full profile resolution (auto-detection, custom extends) is handled by rig-umgy.
/// Here we just map the built-in names.
fn resolve_profile_defaults(profile: &str) -> DisplayProfile {
    match profile {
        "headless" => DisplayProfile::headless(),
        _ => DisplayProfile::full_display(),
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod unit_tests {
    use super::*;

    // ── parse ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_error_location_extraction_with_line_col() {
        // A message containing "line 3, column 7"
        let msg = "TOML parse error at line 3, column 7\n  --> details";
        let (l, c) = parse_toml_location(msg);
        assert_eq!(l, 3);
        assert_eq!(c, 7);
    }

    #[test]
    fn test_parse_error_location_fallback_when_not_found() {
        let msg = "some error without location info";
        let (l, c) = parse_toml_location(msg);
        assert_eq!(l, 1, "line should default to 1");
        assert_eq!(c, 1, "column should default to 1");
    }

    // ── event name validation ─────────────────────────────────────────────────

    #[test]
    fn test_event_name_valid() {
        assert!(is_valid_event_name("doorbell.ring"));
        assert!(is_valid_event_name("door_bell.ring_now"));
        assert!(is_valid_event_name("src123.act456"));
    }

    #[test]
    fn test_event_name_empty_is_valid() {
        assert!(is_valid_event_name(""), "empty string must be valid");
    }

    #[test]
    fn test_event_name_invalid_patterns() {
        assert!(!is_valid_event_name("Doorbell-Ring"), "uppercase not allowed");
        assert!(!is_valid_event_name("doorbell"), "no dot");
        assert!(!is_valid_event_name(".ring"), "empty source");
        assert!(!is_valid_event_name("doorbell."), "empty action");
        assert!(!is_valid_event_name("1doorbell.ring"), "starts with digit");
        assert!(!is_valid_event_name("door.bell.ring"), "more than one dot");
    }

    // ── profile validation ────────────────────────────────────────────────────

    #[test]
    fn test_validate_profile_mobile_gives_mobile_error() {
        let mut errors = Vec::new();
        validate_profile("mobile", &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0].code, ConfigErrorCode::MobileProfileNotExercised));
    }

    #[test]
    fn test_validate_profile_unknown_gives_unknown_error() {
        let mut errors = Vec::new();
        validate_profile("totally_unknown", &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0].code, ConfigErrorCode::UnknownProfile));
    }

    #[test]
    fn test_validate_profile_known_profiles_no_error() {
        for p in &["full-display", "headless", "auto", "custom"] {
            let mut errors = Vec::new();
            validate_profile(p, &mut errors);
            assert!(errors.is_empty(), "profile {:?} should not produce errors", p);
        }
    }

    // ── tab validation ────────────────────────────────────────────────────────

    #[test]
    fn test_no_tabs_produces_no_tabs_error() {
        let raw = RawConfig::default();
        let mut errors = Vec::new();
        validate_tabs(&raw, &mut errors);
        assert!(errors.iter().any(|e| matches!(e.code, ConfigErrorCode::NoTabs)));
    }

    #[test]
    fn test_duplicate_tab_name_produces_error() {
        let mut raw = RawConfig::default();
        raw.tabs.push(crate::raw::RawTab {
            name: Some("Home".into()),
            ..Default::default()
        });
        raw.tabs.push(crate::raw::RawTab {
            name: Some("Home".into()),
            ..Default::default()
        });
        let mut errors = Vec::new();
        validate_tabs(&raw, &mut errors);
        assert!(
            errors.iter().any(|e| matches!(e.code, ConfigErrorCode::DuplicateTabName)),
            "duplicate tab name should produce error"
        );
    }

    #[test]
    fn test_multiple_default_tabs_produces_error() {
        let mut raw = RawConfig::default();
        raw.tabs.push(crate::raw::RawTab {
            name: Some("A".into()),
            default_tab: true,
            ..Default::default()
        });
        raw.tabs.push(crate::raw::RawTab {
            name: Some("B".into()),
            default_tab: true,
            ..Default::default()
        });
        let mut errors = Vec::new();
        validate_tabs(&raw, &mut errors);
        assert!(
            errors.iter().any(|e| matches!(e.code, ConfigErrorCode::MultipleDefaultTabs)),
            "multiple default_tab=true should produce error"
        );
    }

    // ── reserved fractions ────────────────────────────────────────────────────

    #[test]
    fn test_reserved_fractions_sum_to_one_invalid() {
        let layout = crate::raw::RawTabLayout {
            reserved_top_fraction: Some(0.5),
            reserved_bottom_fraction: Some(0.5),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_reserved_fractions(0, &layout, &mut errors);
        assert!(
            errors.iter().any(|e| matches!(e.code, ConfigErrorCode::InvalidReservedFraction)),
            "top + bottom = 1.0 should be invalid"
        );
    }

    #[test]
    fn test_reserved_fractions_valid_no_error() {
        let layout = crate::raw::RawTabLayout {
            reserved_top_fraction: Some(0.1),
            reserved_bottom_fraction: Some(0.1),
            reserved_left_fraction: Some(0.0),
            reserved_right_fraction: Some(0.0),
        };
        let mut errors = Vec::new();
        validate_reserved_fractions(0, &layout, &mut errors);
        assert!(errors.is_empty(), "valid fractions should not produce errors");
    }

    // ── FPS range ─────────────────────────────────────────────────────────────

    #[test]
    fn test_fps_range_target_below_min_invalid() {
        let mut errors = Vec::new();
        validate_fps_range(Some(15), Some(30), &mut errors);
        assert!(errors.iter().any(|e| matches!(e.code, ConfigErrorCode::InvalidFpsRange)));
    }

    #[test]
    fn test_fps_range_target_equals_min_valid() {
        let mut errors = Vec::new();
        validate_fps_range(Some(30), Some(30), &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_fps_range_target_above_min_valid() {
        let mut errors = Vec::new();
        validate_fps_range(Some(60), Some(30), &mut errors);
        assert!(errors.is_empty());
    }

    // ── degradation order ────────────────────────────────────────────────────

    #[test]
    fn test_degradation_out_of_order_produces_error() {
        let deg = RawDegradation {
            coalesce_frame_ms: Some(14.0),
            shed_tiles_frame_ms: Some(12.0),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_degradation_order(&deg, &mut errors);
        assert!(
            errors.iter().any(|e| matches!(e.code, ConfigErrorCode::DegradationThresholdOrder)),
            "out-of-order thresholds should produce error"
        );
    }

    #[test]
    fn test_degradation_in_order_no_error() {
        let deg = RawDegradation {
            coalesce_frame_ms: Some(10.0),
            simplify_rendering_frame_ms: Some(12.0),
            shed_tiles_frame_ms: Some(14.0),
            audio_only_frame_ms: Some(20.0),
            reduce_media_quality_gpu_fraction: Some(0.7),
            reduce_concurrent_streams_gpu_fraction: Some(0.9),
        };
        let mut errors = Vec::new();
        validate_degradation_order(&deg, &mut errors);
        assert!(errors.is_empty(), "in-order thresholds should not produce errors");
    }
}
