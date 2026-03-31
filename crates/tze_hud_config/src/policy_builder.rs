//! Token-to-RenderingPolicy mapper and effective policy constructor — hud-sc0a.7.
//!
//! Implements spec sections:
//! - `component-shape-language/spec.md §Requirement: Default Zone Rendering with Tokens`
//! - `component-shape-language/spec.md §Requirement: Component Profile Selection`
//!
//! ## Overview
//!
//! At startup, the runtime constructs an **effective `RenderingPolicy`** for each
//! built-in zone type following a three-layer merge:
//!
//! 1. **Zone defaults** — the existing `RenderingPolicy::default()` from zone registration.
//! 2. **Token-derived defaults** — per-zone-type token-to-field mappings applied to
//!    any field that is still `None` after layer 1.
//! 3. **Profile overrides** — zone rendering overrides from the active component profile
//!    (if any) merged on top.
//!
//! The result is immutable after startup.
//!
//! ## Error codes produced
//!
//! | Error code | Condition |
//! |---|---|
//! | `CONFIG_UNKNOWN_COMPONENT_TYPE` | A `[component_profiles]` key is not a known component type |
//! | `CONFIG_UNKNOWN_COMPONENT_PROFILE` | A `[component_profiles]` value doesn't match any loaded profile |
//! | `CONFIG_PROFILE_TYPE_MISMATCH` | A `[component_profiles]` entry maps a component type to a profile of a different type |

use std::collections::HashMap;

use tze_hud_scene::config::{ConfigError, ConfigErrorCode};
use tze_hud_scene::types::{FontFamily, RenderingPolicy, Rgba, TextAlign};

use crate::component_profiles::ComponentProfile;
use crate::component_types::ComponentType;
use crate::tokens::{DesignTokenMap, parse_color_hex, parse_font_family, parse_numeric};

// ─── Token lookup helpers ─────────────────────────────────────────────────────

/// Convert a tokens-module `Rgba` to the scene-types `Rgba`.
fn tokens_color_to_scene(c: crate::tokens::Rgba) -> Rgba {
    Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

/// Look up a token as `Rgba`. Returns `None` if the key is absent or unparseable.
fn token_color(tokens: &DesignTokenMap, key: &str) -> Option<Rgba> {
    tokens
        .get(key)
        .and_then(|v| parse_color_hex(v))
        .map(tokens_color_to_scene)
}

/// Look up a token as `f32`. Returns `None` if absent or unparseable.
fn token_f32(tokens: &DesignTokenMap, key: &str) -> Option<f32> {
    tokens.get(key).and_then(|v| parse_numeric(v))
}

/// Look up a token as `u16`. Returns `None` if absent or unparseable.
fn token_u16(tokens: &DesignTokenMap, key: &str) -> Option<u16> {
    tokens
        .get(key)
        .and_then(|v| parse_numeric(v))
        .map(|n| n as u16)
}

/// Look up a token as `FontFamily`. Returns `None` if absent or unparseable.
fn token_font_family(tokens: &DesignTokenMap, key: &str) -> Option<FontFamily> {
    tokens.get(key).and_then(|v| parse_font_family(v))
}

// ─── Token-to-RenderingPolicy mapper ──────────────────────────────────────────

/// Apply token-derived defaults for the `subtitle` zone to `policy`.
///
/// Populates `None` fields only — explicit (non-`None`) values are left untouched.
///
/// Token mappings (per spec §Requirement: Default Zone Rendering with Tokens):
/// - `text_color` ← `color.text.primary`
/// - `font_family` ← `typography.subtitle.family`
/// - `font_size_px` ← `typography.subtitle.size`
/// - `font_weight` ← `typography.subtitle.weight`
/// - `backdrop` ← `color.backdrop.default`
/// - `backdrop_opacity` ← `opacity.backdrop.default`
/// - `outline_color` ← `color.outline.default`
/// - `outline_width` ← `stroke.outline.width`
/// - `text_align` ← `Center` (hardcoded, not token-driven)
/// - `margin_vertical` ← `spacing.padding.medium`
pub fn apply_subtitle_token_defaults(policy: &mut RenderingPolicy, tokens: &DesignTokenMap) {
    if policy.text_color.is_none() {
        policy.text_color = token_color(tokens, "color.text.primary");
    }
    if policy.font_family.is_none() {
        policy.font_family = token_font_family(tokens, "typography.subtitle.family");
    }
    if policy.font_size_px.is_none() {
        policy.font_size_px = token_f32(tokens, "typography.subtitle.size");
    }
    if policy.font_weight.is_none() {
        policy.font_weight = token_u16(tokens, "typography.subtitle.weight");
    }
    if policy.backdrop.is_none() {
        policy.backdrop = token_color(tokens, "color.backdrop.default");
    }
    if policy.backdrop_opacity.is_none() {
        policy.backdrop_opacity = token_f32(tokens, "opacity.backdrop.default");
    }
    if policy.outline_color.is_none() {
        policy.outline_color = token_color(tokens, "color.outline.default");
    }
    if policy.outline_width.is_none() {
        policy.outline_width = token_f32(tokens, "stroke.outline.width");
    }
    // text_align: hardcoded default (Center), not token-driven
    if policy.text_align.is_none() {
        policy.text_align = Some(TextAlign::Center);
    }
    if policy.margin_vertical.is_none() {
        policy.margin_vertical = token_f32(tokens, "spacing.padding.medium");
    }
}

/// Apply token-derived defaults for the `notification-area` zone to `policy`.
///
/// Token mappings:
/// - `text_color` ← `color.text.primary`
/// - `font_family` ← `typography.body.family`
/// - `font_size_px` ← `typography.body.size`
/// - `font_weight` ← `typography.body.weight`
/// - `backdrop` ← `color.backdrop.default`
/// - `backdrop_opacity` ← `opacity.backdrop.opaque`
/// - `outline_color` ← `None` (no outline for notifications; spec says explicitly None)
/// - `margin_horizontal` ← `spacing.padding.medium`
/// - `margin_vertical` ← `spacing.padding.medium`
pub fn apply_notification_area_token_defaults(
    policy: &mut RenderingPolicy,
    tokens: &DesignTokenMap,
) {
    if policy.text_color.is_none() {
        policy.text_color = token_color(tokens, "color.text.primary");
    }
    if policy.font_family.is_none() {
        policy.font_family = token_font_family(tokens, "typography.body.family");
    }
    if policy.font_size_px.is_none() {
        policy.font_size_px = token_f32(tokens, "typography.body.size");
    }
    if policy.font_weight.is_none() {
        policy.font_weight = token_u16(tokens, "typography.body.weight");
    }
    if policy.backdrop.is_none() {
        policy.backdrop = token_color(tokens, "color.backdrop.default");
    }
    if policy.backdrop_opacity.is_none() {
        policy.backdrop_opacity = token_f32(tokens, "opacity.backdrop.opaque");
    }
    // outline_color: explicitly None for notifications — not token-driven
    if policy.margin_horizontal.is_none() {
        policy.margin_horizontal = token_f32(tokens, "spacing.padding.medium");
    }
    if policy.margin_vertical.is_none() {
        policy.margin_vertical = token_f32(tokens, "spacing.padding.medium");
    }
}

/// Apply token-derived defaults for the `status-bar` zone to `policy`.
///
/// Token mappings:
/// - `text_color` ← `color.text.secondary`
/// - `font_family` ← `typography.body.family`
/// - `font_size_px` ← `typography.body.size`
/// - `backdrop` ← `color.backdrop.default`
/// - `backdrop_opacity` ← `opacity.backdrop.opaque`
pub fn apply_status_bar_token_defaults(policy: &mut RenderingPolicy, tokens: &DesignTokenMap) {
    if policy.text_color.is_none() {
        policy.text_color = token_color(tokens, "color.text.secondary");
    }
    if policy.font_family.is_none() {
        policy.font_family = token_font_family(tokens, "typography.body.family");
    }
    if policy.font_size_px.is_none() {
        policy.font_size_px = token_f32(tokens, "typography.body.size");
    }
    if policy.backdrop.is_none() {
        policy.backdrop = token_color(tokens, "color.backdrop.default");
    }
    if policy.backdrop_opacity.is_none() {
        policy.backdrop_opacity = token_f32(tokens, "opacity.backdrop.opaque");
    }
}

/// Apply token-derived defaults for the `alert-banner` zone to `policy`.
///
/// Token mappings:
/// - `text_color` ← `color.text.primary`
/// - `font_family` ← `typography.heading.family`
/// - `font_size_px` ← `typography.heading.size`
/// - `font_weight` ← `typography.heading.weight`
/// - `backdrop` ← `color.backdrop.default`
/// - `backdrop_opacity` ← `opacity.backdrop.opaque`
pub fn apply_alert_banner_token_defaults(policy: &mut RenderingPolicy, tokens: &DesignTokenMap) {
    if policy.text_color.is_none() {
        policy.text_color = token_color(tokens, "color.text.primary");
    }
    if policy.font_family.is_none() {
        policy.font_family = token_font_family(tokens, "typography.heading.family");
    }
    if policy.font_size_px.is_none() {
        policy.font_size_px = token_f32(tokens, "typography.heading.size");
    }
    if policy.font_weight.is_none() {
        policy.font_weight = token_u16(tokens, "typography.heading.weight");
    }
    if policy.backdrop.is_none() {
        policy.backdrop = token_color(tokens, "color.backdrop.default");
    }
    if policy.backdrop_opacity.is_none() {
        policy.backdrop_opacity = token_f32(tokens, "opacity.backdrop.opaque");
    }
}

/// Apply token-derived defaults to `policy` for the given zone type name.
///
/// For zone types that have no token-driven defaults (`ambient-background`, `pip`),
/// this is a no-op.
pub fn apply_token_defaults_for_zone(
    zone_name: &str,
    policy: &mut RenderingPolicy,
    tokens: &DesignTokenMap,
) {
    match zone_name {
        "subtitle" => apply_subtitle_token_defaults(policy, tokens),
        "notification-area" => apply_notification_area_token_defaults(policy, tokens),
        "status-bar" => apply_status_bar_token_defaults(policy, tokens),
        "alert-banner" => apply_alert_banner_token_defaults(policy, tokens),
        // ambient-background, pip: no token-driven rendering policy fields
        _ => {}
    }
}

// ─── Profile overrides → RenderingPolicy merge ───────────────────────────────

/// Merge a `ZoneRenderingOverride` on top of an existing `RenderingPolicy`.
///
/// Override fields (when `Some`) replace the corresponding policy fields.
/// `None` override fields leave the policy field unchanged.
///
/// Color strings in the override are expected to be already resolved
/// (no `{{token.key}}` references remain after `scan_profile_dirs` parsing).
pub fn merge_zone_override(
    policy: &mut RenderingPolicy,
    override_: &crate::component_profiles::ZoneRenderingOverride,
) {
    if let Some(ref ff_str) = override_.font_family {
        if let Some(ff) = parse_font_family(ff_str) {
            policy.font_family = Some(ff);
        }
    }
    if let Some(sz) = override_.font_size_px {
        policy.font_size_px = Some(sz);
    }
    if let Some(fw) = override_.font_weight {
        policy.font_weight = Some(fw as u16);
    }
    if let Some(ref color_str) = override_.text_color {
        if let Some(c) = parse_color_hex(color_str) {
            policy.text_color = Some(tokens_color_to_scene(c));
        }
    }
    if let Some(ref align_str) = override_.text_align {
        policy.text_align = match align_str.as_str() {
            "start" => Some(TextAlign::Start),
            "center" => Some(TextAlign::Center),
            "end" => Some(TextAlign::End),
            _ => policy.text_align,
        };
    }
    if let Some(ref color_str) = override_.backdrop_color {
        if let Some(c) = parse_color_hex(color_str) {
            policy.backdrop = Some(tokens_color_to_scene(c));
        }
    }
    if let Some(op) = override_.backdrop_opacity {
        policy.backdrop_opacity = Some(op);
    }
    if let Some(ref color_str) = override_.outline_color {
        if let Some(c) = parse_color_hex(color_str) {
            policy.outline_color = Some(tokens_color_to_scene(c));
        }
    }
    if let Some(w) = override_.outline_width {
        policy.outline_width = Some(w);
    }
    if let Some(mh) = override_.margin_horizontal {
        policy.margin_horizontal = Some(mh);
    }
    if let Some(mv) = override_.margin_vertical {
        policy.margin_vertical = Some(mv);
    }
    if let Some(t) = override_.transition_in_ms {
        policy.transition_in_ms = Some(t);
    }
    if let Some(t) = override_.transition_out_ms {
        policy.transition_out_ms = Some(t);
    }
}

// ─── Profile selection resolver ───────────────────────────────────────────────

/// A resolved profile selection: component type → loaded ComponentProfile.
pub type ProfileSelection = HashMap<ComponentType, ComponentProfile>;

/// Validate and resolve `[component_profiles]` entries against the loaded profiles.
///
/// For each entry in `raw_profiles` (component type name → profile name):
/// 1. Validate the component type name against v1 component types.
/// 2. Look up the profile by name in `loaded_profiles`.
/// 3. Validate that the profile's `component_type` matches the key.
///
/// On success, returns a `ProfileSelection` mapping component types to profiles.
/// Errors are appended to `errors`; if any errors occur, the returned map may
/// be partial (used only for logging; callers should check `errors` before using).
pub fn resolve_profile_selection(
    raw_profiles: &HashMap<String, String>,
    loaded_profiles: &[ComponentProfile],
    errors: &mut Vec<ConfigError>,
) -> ProfileSelection {
    let mut selection = ProfileSelection::new();

    for (ct_name, profile_name) in raw_profiles {
        // Step 1: validate component type name.
        let component_type = match ComponentType::from_name(ct_name) {
            Some(ct) => ct,
            None => {
                errors.push(ConfigError {
                    code: ConfigErrorCode::ConfigUnknownComponentType,
                    field_path: format!("component_profiles.{ct_name}"),
                    expected: "a recognized v1 component type name (e.g. 'subtitle', 'notification', 'status-bar', 'alert-banner', 'ambient-background', 'pip')".into(),
                    got: ct_name.clone(),
                    hint: format!(
                        "'{ct_name}' is not a recognized v1 component type; \
                         valid names are: subtitle, notification, status-bar, \
                         alert-banner, ambient-background, pip"
                    ),
                });
                continue;
            }
        };

        // Step 2: look up profile by name.
        let profile = match loaded_profiles.iter().find(|p| p.name == *profile_name) {
            Some(p) => p,
            None => {
                errors.push(ConfigError {
                    code: ConfigErrorCode::ConfigUnknownComponentProfile,
                    field_path: format!("component_profiles.{ct_name}"),
                    expected: format!("a loaded profile named '{profile_name}'"),
                    got: profile_name.clone(),
                    hint: format!(
                        "profile '{profile_name}' not found among loaded profiles; \
                         check [component_profile_bundles].paths and verify the \
                         profile directory contains a valid profile.toml"
                    ),
                });
                continue;
            }
        };

        // Step 3: validate component type match.
        if profile.component_type != component_type {
            let expected_type_name = component_type.contract().name;
            let actual_type_name = profile.component_type.contract().name;
            errors.push(ConfigError {
                code: ConfigErrorCode::ConfigProfileTypeMismatch,
                field_path: format!("component_profiles.{ct_name}"),
                expected: format!("a profile with component_type = '{expected_type_name}'"),
                got: format!("profile '{profile_name}' has component_type = '{actual_type_name}'"),
                hint: format!(
                    "component_profiles.{ct_name} = '{profile_name}' is invalid because \
                     profile '{profile_name}' implements '{actual_type_name}', not \
                     '{expected_type_name}'; use a '{expected_type_name}' profile here"
                ),
            });
            continue;
        }

        selection.insert(component_type, profile.clone());
    }

    selection
}

// ─── Effective policy constructor ─────────────────────────────────────────────

/// Construct the effective `RenderingPolicy` for a zone type.
///
/// Merge order (lowest → highest priority):
/// 1. Zone type default policy (passed in as `zone_default`)
/// 2. Token-derived defaults (from `tokens`)
/// 3. Active profile zone override (if any)
///
/// The `zone_name` is the zone registry name (e.g., `"subtitle"`,
/// `"notification-area"`, `"status-bar"`, `"alert-banner"`).
///
/// If `active_profile` is `None` (no profile selected for this component type),
/// only layers 1 and 2 are applied.
pub fn build_effective_policy(
    zone_name: &str,
    zone_default: &RenderingPolicy,
    tokens: &DesignTokenMap,
    active_profile: Option<&ComponentProfile>,
) -> RenderingPolicy {
    // Start from the zone's current policy (layer 1).
    let mut policy = zone_default.clone();

    // Layer 2: token-derived defaults (populate None fields only).
    apply_token_defaults_for_zone(zone_name, &mut policy, tokens);

    // Layer 3: profile zone override (if any).
    if let Some(profile) = active_profile {
        if let Some(zone_override) = profile.zone_overrides.get(zone_name) {
            merge_zone_override(&mut policy, zone_override);
        }
    }

    policy
}

/// Build effective rendering policies for all built-in zone types.
///
/// Returns a `HashMap<zone_name, effective_RenderingPolicy>` that can be used
/// to patch `ZoneRegistry::with_defaults()` after construction.
///
/// The `profile_selection` maps component types to their active profiles.
/// Zone types with no active profile receive token-derived defaults only.
pub fn build_all_effective_policies(
    zone_defaults: &HashMap<String, RenderingPolicy>,
    tokens: &DesignTokenMap,
    profile_selection: &ProfileSelection,
) -> HashMap<String, RenderingPolicy> {
    // Map: component type zone_type_name → active profile
    let profile_by_zone: HashMap<&str, &ComponentProfile> = profile_selection
        .iter()
        .map(|(ct, profile)| (ct.contract().zone_type_name, profile))
        .collect();

    let mut result = HashMap::new();

    for (zone_name, zone_default) in zone_defaults {
        let active_profile = profile_by_zone.get(zone_name.as_str()).copied();
        let effective = build_effective_policy(zone_name, zone_default, tokens, active_profile);
        result.insert(zone_name.clone(), effective);
    }

    result
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{CANONICAL_TOKENS, resolve_tokens};

    fn default_tokens() -> DesignTokenMap {
        resolve_tokens(&DesignTokenMap::new(), &DesignTokenMap::new())
    }

    // ── apply_token_defaults_for_zone ─────────────────────────────────────────

    #[test]
    fn test_subtitle_token_defaults_populate_none_fields() {
        let tokens = default_tokens();
        let mut policy = RenderingPolicy::default();
        apply_token_defaults_for_zone("subtitle", &mut policy, &tokens);

        // text_color should be populated from color.text.primary = "#FFFFFF"
        assert!(
            policy.text_color.is_some(),
            "text_color should be set from tokens"
        );
        let tc = policy.text_color.unwrap();
        assert!(
            (tc.r - 1.0).abs() < 1e-4,
            "text_color.r should be 1.0 for #FFFFFF"
        );
        assert!(
            (tc.g - 1.0).abs() < 1e-4,
            "text_color.g should be 1.0 for #FFFFFF"
        );
        assert!(
            (tc.b - 1.0).abs() < 1e-4,
            "text_color.b should be 1.0 for #FFFFFF"
        );

        // font_family should be set
        assert!(policy.font_family.is_some());
        assert_eq!(policy.font_family.unwrap(), FontFamily::SystemSansSerif);

        // text_align should be Center (hardcoded)
        assert_eq!(policy.text_align, Some(TextAlign::Center));

        // backdrop should be set
        assert!(policy.backdrop.is_some());

        // outline_color should be set
        assert!(policy.outline_color.is_some());
    }

    #[test]
    fn test_token_defaults_do_not_overwrite_existing_values() {
        let tokens = default_tokens();
        let mut policy = RenderingPolicy::default();

        // Pre-set font_size_px to 32.0 (explicit config value)
        policy.font_size_px = Some(32.0);

        apply_token_defaults_for_zone("subtitle", &mut policy, &tokens);

        // font_size_px should remain 32.0, not the canonical default of 28
        assert_eq!(
            policy.font_size_px,
            Some(32.0),
            "explicit value must not be overwritten by token default"
        );
    }

    #[test]
    fn test_custom_color_token_reflected_in_policy() {
        // spec: WHEN color.text.primary = "#00FF00" THEN text_color = Rgba(0,1,0,1)
        let mut config_tokens = DesignTokenMap::new();
        config_tokens.insert("color.text.primary".to_string(), "#00FF00".to_string());
        let tokens = resolve_tokens(&config_tokens, &DesignTokenMap::new());

        let mut policy = RenderingPolicy::default();
        apply_token_defaults_for_zone("subtitle", &mut policy, &tokens);

        let tc = policy.text_color.expect("text_color should be set");
        assert!((tc.r).abs() < 1e-4, "r should be 0 for #00FF00");
        assert!((tc.g - 1.0).abs() < 1e-4, "g should be 1.0 for #00FF00");
        assert!((tc.b).abs() < 1e-4, "b should be 0 for #00FF00");
    }

    #[test]
    fn test_notification_area_token_defaults() {
        let tokens = default_tokens();
        let mut policy = RenderingPolicy::default();
        apply_token_defaults_for_zone("notification-area", &mut policy, &tokens);

        assert!(policy.text_color.is_some());
        assert!(policy.font_family.is_some());
        assert!(policy.font_size_px.is_some());
        assert!(policy.backdrop.is_some());
        assert!(policy.backdrop_opacity.is_some());
        // No outline for notifications (spec: outline_color ← None)
        assert!(
            policy.outline_color.is_none(),
            "notification-area must NOT have outline_color set from tokens"
        );
        assert!(policy.margin_horizontal.is_some());
        assert!(policy.margin_vertical.is_some());
    }

    #[test]
    fn test_status_bar_token_defaults() {
        let tokens = default_tokens();
        let mut policy = RenderingPolicy::default();
        apply_token_defaults_for_zone("status-bar", &mut policy, &tokens);

        assert!(policy.text_color.is_some());
        assert!(policy.font_family.is_some());
        assert!(policy.font_size_px.is_some());
        assert!(policy.backdrop.is_some());
        assert!(policy.backdrop_opacity.is_some());
    }

    #[test]
    fn test_alert_banner_token_defaults() {
        let tokens = default_tokens();
        let mut policy = RenderingPolicy::default();
        apply_token_defaults_for_zone("alert-banner", &mut policy, &tokens);

        assert!(policy.text_color.is_some());
        assert!(policy.font_family.is_some());
        assert!(policy.font_size_px.is_some());
        assert!(policy.font_weight.is_some());
        assert!(policy.backdrop.is_some());
        assert!(policy.backdrop_opacity.is_some());
    }

    #[test]
    fn test_ambient_background_no_token_defaults() {
        let tokens = default_tokens();
        let mut policy = RenderingPolicy::default();
        apply_token_defaults_for_zone("ambient-background", &mut policy, &tokens);

        // ambient-background has no token-driven rendering policy fields
        assert!(policy.text_color.is_none());
        assert!(policy.font_family.is_none());
        assert!(policy.backdrop.is_none());
    }

    #[test]
    fn test_pip_no_token_defaults() {
        let tokens = default_tokens();
        let mut policy = RenderingPolicy::default();
        apply_token_defaults_for_zone("pip", &mut policy, &tokens);

        // pip has no token-driven rendering policy fields
        assert!(policy.text_color.is_none());
        assert!(policy.font_family.is_none());
    }

    // ── build_effective_policy ────────────────────────────────────────────────

    #[test]
    fn test_build_effective_policy_no_profile() {
        let tokens = default_tokens();
        let zone_default = RenderingPolicy::default();
        let policy = build_effective_policy("subtitle", &zone_default, &tokens, None);

        // Should have token-derived defaults
        assert!(policy.text_color.is_some());
        assert!(policy.font_family.is_some());
        assert_eq!(policy.text_align, Some(TextAlign::Center));
    }

    #[test]
    fn test_build_effective_policy_absent_zone_type_is_noop() {
        let tokens = default_tokens();
        let zone_default = RenderingPolicy::default();
        let policy = build_effective_policy("ambient-background", &zone_default, &tokens, None);

        // ambient-background gets no token defaults
        assert_eq!(policy, RenderingPolicy::default());
    }

    // ── resolve_profile_selection ─────────────────────────────────────────────

    #[test]
    fn test_resolve_profile_selection_empty_config() {
        let mut errors = Vec::new();
        let selection = resolve_profile_selection(&HashMap::new(), &[], &mut errors);
        assert!(errors.is_empty());
        assert!(selection.is_empty());
    }

    #[test]
    fn test_resolve_profile_selection_unknown_component_type() {
        let mut raw = HashMap::new();
        raw.insert("not-a-type".to_string(), "some-profile".to_string());
        let mut errors = Vec::new();
        let selection = resolve_profile_selection(&raw, &[], &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(
            matches!(errors[0].code, ConfigErrorCode::ConfigUnknownComponentType),
            "expected ConfigUnknownComponentType, got {:?}",
            errors[0].code
        );
        assert!(selection.is_empty());
    }

    #[test]
    fn test_resolve_profile_selection_unknown_profile() {
        let mut raw = HashMap::new();
        raw.insert("subtitle".to_string(), "nonexistent-profile".to_string());
        let mut errors = Vec::new();
        let selection = resolve_profile_selection(&raw, &[], &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(
            matches!(
                errors[0].code,
                ConfigErrorCode::ConfigUnknownComponentProfile
            ),
            "expected ConfigUnknownComponentProfile, got {:?}",
            errors[0].code
        );
        assert!(selection.is_empty());
    }
}
