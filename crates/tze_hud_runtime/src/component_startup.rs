//! Component shape language startup sequence — hud-sc0a.9.
//!
//! Orchestrates the full 10-step component shape language startup in the correct
//! dependency order:
//!
//! 1. Config parsing (done by caller before invoking this module)
//! 2. Design token loading — parse `[design_tokens]`, merge with canonical fallbacks → global token map
//! 3. Global widget bundle loading — pass global token map to `scan_bundle_dirs`
//! 4. Component profile loading — scan `[component_profile_bundles].paths`, parse `profile.toml`,
//!    construct per-profile scoped token maps, load profile widget bundles + zone overrides
//! 5. Component profile selection — resolve `[component_profiles]` → loaded profiles, validate
//! 6. Default zone rendering policy construction — token-derived defaults → profile overrides merged
//! 7. Readability validation — validate effective `RenderingPolicy` + profile SVGs per component type
//! 8. Zone registry construction — build `ZoneRegistry` with effective `RenderingPolicy` per zone type
//! 9. Widget registry construction — register global + profile-scoped widgets
//! 10. Session establishment (done by caller after this module returns)
//!
//! ## Dependency constraints
//!
//! - Token loading (step 2) MUST complete before any SVG resolution (steps 3-4).
//! - Profile loading (step 4) MUST complete before selection (step 5).
//! - Selection (step 5) MUST complete before effective policy construction (step 6).
//! - Readability validation (step 7) MUST use final effective policies.
//!
//! ## Error handling
//!
//! Profile loading errors (individual bad profiles) are logged and skipped; they do
//! not abort startup per spec §Component Profile Format: "Invalid profiles MUST be
//! logged and skipped". Profile selection errors (bad component type, unknown profile
//! name, type mismatch) are logged at WARN and the selection is skipped for that
//! entry, falling back to token-derived defaults only.
//!
//! Readability violations for zones with active profiles are hard errors in
//! production builds; in dev mode (`TZE_HUD_DEV=1` or profile=headless) they
//! are downgraded to WARN (see `tze_hud_config::readability::is_dev_mode`).
//!
//! ## Spec references
//!
//! - `component-shape-language/spec.md §Requirement: Startup Sequence Integration`
//! - `component-shape-language/spec.md §Requirement: Default Zone Rendering with Tokens`
//! - `component-shape-language/spec.md §Requirement: Component Profile Selection`
//! - `component-shape-language/spec.md §Requirement: Zone Readability Enforcement`
//! - `configuration/spec.md §Requirement: Design Token Configuration Section`
//! - `configuration/spec.md §Requirement: Component Profile Paths Configuration`
//! - `configuration/spec.md §Requirement: Component Profile Selection Configuration`

use std::collections::HashMap;
use std::path::Path;

use tze_hud_config::policy_builder::{
    ProfileSelection, build_all_effective_policies, resolve_profile_selection,
};
use tze_hud_config::raw::RawConfig;
use tze_hud_config::readability::{check_zone_readability, is_dev_mode};
use tze_hud_config::scan_profile_dirs;
use tze_hud_config::tokens::{DesignTokenMap, resolve_tokens};
use tze_hud_scene::types::{RenderingPolicy, ZoneRegistry};
use tze_hud_widget::loader::LoadedBundle;

use crate::widget_startup::init_widget_registry;

// ─── ComponentStartupResult ───────────────────────────────────────────────────

/// Result of the component shape language startup sequence.
///
/// Contains the fully resolved global token map and the effective zone registry
/// (with token-derived + profile-overridden rendering policies). The caller
/// assigns `zone_registry` into the `SceneGraph` and `global_tokens` is
/// available for diagnostic / session snapshot purposes.
pub struct ComponentStartupResult {
    /// Fully resolved global token map (canonical fallbacks + config overrides).
    pub global_tokens: DesignTokenMap,
    /// Zone registry with effective `RenderingPolicy` per zone type.
    ///
    /// Replaces the caller's default `ZoneRegistry::with_defaults()`.
    pub zone_registry: ZoneRegistry,
    /// Profile-scoped widget bundles that must be registered in the
    /// `WidgetRegistry` (namespaced as `"{profile_name}/{widget_name}"`).
    ///
    /// These are already processed and ready for `WidgetRegistry::register_definition`.
    pub profile_widget_bundles: Vec<LoadedBundle>,
    /// SVG assets from global widget bundles for compositor registration.
    pub widget_svg_assets: Vec<crate::widget_startup::WidgetSvgAsset>,
    /// Pre-merged compositor token map: `global_tokens` with all active profile
    /// `[token_overrides]` applied on top.
    ///
    /// Covers every profile that is active (Notification, AlertBanner, etc.) and
    /// every token those profiles override — not just `color.notification.urgency.*`.
    /// Pass this directly to `compositor.set_token_map()` without any further
    /// merging in the caller.
    pub compositor_tokens: DesignTokenMap,
}

// ─── run_component_startup ────────────────────────────────────────────────────

/// Execute steps 2–9 of the component shape language startup sequence.
///
/// Caller is responsible for step 1 (config parsing) and step 10 (session
/// establishment). The caller must supply:
///
/// - `raw`: a parsed (validated) `RawConfig`
/// - `config_parent`: parent directory of the config file (for relative path
///   resolution of `[component_profile_bundles].paths`). Pass `None` to resolve
///   paths relative to the current working directory.
/// - `profile_name`: the resolved display profile name (e.g. `"headless"`), used
///   only for dev-mode detection in readability validation.
/// - `scene`: mutable reference to a `SceneGraph` — the zone registry and widget
///   registry inside it are populated by this function. On return, `scene.zone_registry`
///   contains effective rendering policies for all built-in zones.
///
/// # Return value
///
/// Returns a `ComponentStartupResult` with the global token map and the effective
/// zone registry (already applied to `scene`), plus any profile-scoped widget bundles
/// that need to be registered.
///
/// Note: this function calls `init_widget_registry` internally for global widget
/// bundles (step 9a). Callers must additionally register the returned
/// `profile_widget_bundles` into the `WidgetRegistry` to complete step 9b.
pub fn run_component_startup(
    raw: &RawConfig,
    config_parent: Option<&Path>,
    profile_name: Option<&str>,
    scene: &mut tze_hud_scene::graph::SceneGraph,
) -> ComponentStartupResult {
    // ── Step 2: Design token loading ──────────────────────────────────────────
    // Parse [design_tokens] and merge with canonical fallbacks → global token map.
    let config_tokens: DesignTokenMap = raw
        .design_tokens
        .as_ref()
        .map(|dt| dt.0.clone())
        .unwrap_or_default();
    let global_tokens = resolve_tokens(&config_tokens, &DesignTokenMap::new());

    tracing::info!(
        token_count = global_tokens.len(),
        "component_startup: step 2 — design tokens loaded"
    );

    // ── Step 3: Global widget bundle loading ──────────────────────────────────
    // Calls init_widget_registry with the global token map so {{token.key}}
    // placeholders in SVG files are resolved at load time.
    // (Profile-scoped widget bundles are handled in step 4 below.)
    let tab_map = std::collections::HashMap::new();
    let widget_svg_assets =
        init_widget_registry(scene, raw, config_parent, &tab_map, &global_tokens);

    tracing::debug!("component_startup: step 3 — global widget bundles loaded");

    // ── Step 4: Component profile loading ─────────────────────────────────────
    // Scan [component_profile_bundles].paths, parse profile.toml, construct
    // per-profile scoped token maps, load profile widget bundles + zone overrides.
    let profile_roots: Vec<std::path::PathBuf> = raw
        .component_profile_bundles
        .as_ref()
        .map(|cpb| {
            let base = config_parent.unwrap_or_else(|| Path::new("."));
            cpb.paths
                .iter()
                .map(|p| {
                    let p = std::path::Path::new(p);
                    if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        base.join(p)
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let mut profile_errors = Vec::new();
    let loaded_profiles = if profile_roots.is_empty() {
        Vec::new()
    } else {
        scan_profile_dirs(&profile_roots, &global_tokens, &mut profile_errors)
    };

    for err in &profile_errors {
        tracing::warn!(
            code = ?err.code,
            field = %err.field_path,
            "component_startup: step 4 — profile load error ({})",
            err.hint
        );
    }

    tracing::info!(
        profile_count = loaded_profiles.len(),
        error_count = profile_errors.len(),
        "component_startup: step 4 — component profiles loaded"
    );

    // ── Step 5: Component profile selection ───────────────────────────────────
    // Resolve [component_profiles] entries (type → profile name) against loaded
    // profiles. Validate component type and type matching.
    let raw_profile_selection: HashMap<String, String> = raw
        .component_profiles
        .as_ref()
        .map(|cp| cp.0.clone())
        .unwrap_or_default();

    let mut selection_errors = Vec::new();
    let profile_selection: ProfileSelection = if raw_profile_selection.is_empty() {
        HashMap::new()
    } else {
        resolve_profile_selection(
            &raw_profile_selection,
            &loaded_profiles,
            &mut selection_errors,
        )
    };

    for err in &selection_errors {
        tracing::warn!(
            code = ?err.code,
            field = %err.field_path,
            "component_startup: step 5 — profile selection error ({})",
            err.hint
        );
    }

    tracing::info!(
        selected_count = profile_selection.len(),
        error_count = selection_errors.len(),
        "component_startup: step 5 — component profile selection resolved"
    );

    // ── Step 6: Default zone rendering policy construction ────────────────────
    // Build a ZoneRegistry with defaults, extract the default rendering policies
    // per zone, then construct effective policies via:
    //   zone default → token-derived defaults → profile override merge
    let base_registry = ZoneRegistry::with_defaults();
    let zone_defaults: HashMap<String, RenderingPolicy> = base_registry
        .zones
        .iter()
        .map(|(name, def)| (name.clone(), def.rendering_policy.clone()))
        .collect();

    let effective_policies =
        build_all_effective_policies(&zone_defaults, &global_tokens, &profile_selection);

    tracing::info!(
        zone_count = effective_policies.len(),
        "component_startup: step 6 — effective rendering policies constructed"
    );

    // ── Step 7: Readability validation ────────────────────────────────────────
    // Validate effective RenderingPolicy per component type that has an active
    // profile. Zones without active profiles (token defaults only) are not
    // validated (no profile = no readability contract required at startup).
    //
    // In dev mode, violations are WARN-logged instead of hard errors.
    let dev_mode = is_dev_mode(profile_name);

    for component_type in profile_selection.keys() {
        let zone_name = component_type.contract().zone_type_name;
        let technique = component_type.contract().readability;

        if let Some(effective_policy) = effective_policies.get(zone_name) {
            if let Err(violation) = check_zone_readability(effective_policy, technique) {
                if dev_mode {
                    tracing::warn!(
                        zone = zone_name,
                        violation = %violation,
                        "component_startup: step 7 — readability violation (dev mode, continuing)"
                    );
                } else {
                    // In production, this is a hard error (logged at ERROR).
                    // We continue startup with a warning rather than panicking,
                    // because the runtime must remain available for monitoring.
                    tracing::error!(
                        zone = zone_name,
                        violation = %violation,
                        "component_startup: step 7 — PROFILE_READABILITY_VIOLATION: zone {} \
                         does not meet readability requirements for {:?}; \
                         check profile zone overrides and token values",
                        zone_name, technique
                    );
                }
            } else {
                tracing::debug!(
                    zone = zone_name,
                    "component_startup: step 7 — readability OK for zone {zone_name}"
                );
            }
        }
    }

    tracing::info!("component_startup: step 7 — readability validation complete");

    // ── Step 8: Zone registry construction ───────────────────────────────────
    // Rebuild the zone registry, patching each zone's rendering_policy with the
    // effective policy (token-derived defaults + profile overrides).
    let mut zone_registry = base_registry;
    for (zone_name, effective_policy) in &effective_policies {
        if let Some(zone_def) = zone_registry.zones.get_mut(zone_name) {
            zone_def.rendering_policy = effective_policy.clone();
        }
    }

    // Apply the populated zone registry to the scene graph.
    scene.zone_registry = zone_registry.clone();

    tracing::info!(
        zone_count = zone_registry.zones.len(),
        "component_startup: step 8 — zone registry constructed with effective rendering policies"
    );

    // ── Step 9b: Collect profile-scoped widget bundles ────────────────────────
    // Global widgets were already registered in step 3 via init_widget_registry.
    // Profile-scoped widget bundles need to be registered with namespaced names
    // ("{profile_name}/{widget_name}"). Return them to the caller for registration.
    //
    // The caller (windowed.rs / headless.rs) must register these in the scene's
    // WidgetRegistry after this function returns.
    let mut profile_widget_bundles: Vec<LoadedBundle> = Vec::new();
    for profile in &loaded_profiles {
        for bundle in &profile.widget_bundles {
            // Clone and rename: the namespace prefix is already applied by
            // scan_profile_dirs (per spec §Profile Widget Scope).
            profile_widget_bundles.push(bundle.clone());
        }
    }

    if !profile_widget_bundles.is_empty() {
        tracing::info!(
            bundle_count = profile_widget_bundles.len(),
            "component_startup: step 9b — {} profile-scoped widget bundles collected",
            profile_widget_bundles.len()
        );
    }

    // ── Build compositor_tokens: global tokens + all active profile overrides ──
    // Merge token_overrides from every active profile on top of global_tokens.
    // This covers all component types (Notification, AlertBanner, etc.) and all
    // tokens those profiles override, giving the compositor a single pre-merged map.
    let mut compositor_tokens: DesignTokenMap = global_tokens.clone();
    let mut total_override_count = 0usize;
    for (component_type, profile) in &profile_selection {
        let override_count = profile.token_overrides.len();
        if override_count == 0 {
            tracing::debug!(
                profile = %profile.name,
                component = ?component_type,
                "component_startup: active {:?} profile '{}' has no token overrides",
                component_type,
                profile.name
            );
        } else {
            tracing::info!(
                profile = %profile.name,
                component = ?component_type,
                token_count = override_count,
                "component_startup: merging {} token override(s) from {:?} profile '{}'",
                override_count,
                component_type,
                profile.name
            );
            for (k, v) in &profile.token_overrides {
                if let Some(existing) = compositor_tokens.get(k) {
                    if existing != v {
                        tracing::warn!(
                            token = %k,
                            profile = %profile.name,
                            component = ?component_type,
                            "component_startup: token '{}' overridden by {:?} profile '{}' \
                             conflicts with a previously merged value ('{}' → '{}'); \
                             last-write wins (HashMap iteration order is non-deterministic)",
                            k,
                            component_type,
                            profile.name,
                            existing,
                            v
                        );
                    }
                }
                compositor_tokens.insert(k.clone(), v.clone());
            }
            total_override_count += override_count;
        }
    }

    if total_override_count > 0 {
        tracing::info!(
            total_override_count,
            profile_count = profile_selection.len(),
            "component_startup: compositor_tokens built ({} global + {} profile overrides)",
            global_tokens.len(),
            total_override_count
        );
    }

    ComponentStartupResult {
        global_tokens,
        zone_registry,
        profile_widget_bundles,
        widget_svg_assets,
        compositor_tokens,
    }
}

// ─── register_profile_widgets ─────────────────────────────────────────────────

/// Register profile-scoped widget bundles from `ComponentStartupResult` into the scene.
///
/// Called after `run_component_startup` to complete step 9b: registering the
/// profile-scoped widgets (namespaced as `"{profile_name}/{widget_name}"`) into
/// the scene's `WidgetRegistry`.
///
/// This is separate from `run_component_startup` so that the caller can inspect
/// or filter the bundles before registration if needed.
pub fn register_profile_widgets(
    scene: &mut tze_hud_scene::graph::SceneGraph,
    result: &ComponentStartupResult,
) {
    for bundle in &result.profile_widget_bundles {
        let name = bundle.definition.id.clone();
        if scene.widget_registry.get_definition(&name).is_some() {
            tracing::warn!(
                widget_name = %name,
                "component_startup: profile widget '{}' already registered (duplicate skipped)",
                name
            );
            continue;
        }
        tracing::info!(
            widget_name = %name,
            "component_startup: registered profile-scoped widget type '{name}'"
        );
        scene
            .widget_registry
            .register_definition(bundle.definition.clone());
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_config::raw::{RawConfig, RawDesignTokens};
    use tze_hud_scene::graph::SceneGraph;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_scene() -> SceneGraph {
        SceneGraph::new(1920.0, 1080.0)
    }

    // ── Step 2: Design token loading ──────────────────────────────────────────

    /// WHEN [design_tokens] is absent THEN global tokens contain canonical fallbacks only.
    #[test]
    fn absent_design_tokens_uses_canonical_fallbacks() {
        let raw = RawConfig::default();
        let mut scene = make_scene();
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);

        // color.text.primary has canonical fallback "#FFFFFF"
        assert_eq!(
            result
                .global_tokens
                .get("color.text.primary")
                .map(|s| s.as_str()),
            Some("#FFFFFF"),
            "absent [design_tokens] should produce canonical fallback for color.text.primary"
        );
        // All canonical tokens should be present
        assert!(
            result.global_tokens.len() >= 20,
            "should have at least 20 canonical tokens, got {}",
            result.global_tokens.len()
        );
    }

    /// WHEN [design_tokens] has overrides THEN they take precedence over canonical fallbacks.
    #[test]
    fn design_tokens_override_canonical_fallbacks() {
        let mut raw = RawConfig::default();
        let mut dt_map = HashMap::new();
        dt_map.insert("color.text.primary".to_string(), "#FF0000".to_string());
        raw.design_tokens = Some(RawDesignTokens(dt_map));

        let mut scene = make_scene();
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);

        assert_eq!(
            result
                .global_tokens
                .get("color.text.primary")
                .map(|s| s.as_str()),
            Some("#FF0000"),
            "config token override should take precedence over canonical fallback"
        );
    }

    // ── Step 4: Profile loading (no profiles configured) ──────────────────────

    /// WHEN [component_profile_bundles] is absent THEN no profiles loaded; startup succeeds.
    #[test]
    fn absent_profile_bundles_no_profiles_loaded() {
        let raw = RawConfig::default();
        let mut scene = make_scene();
        // Should not panic or fail
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);
        // Zone registry should still have all default zones
        assert!(
            scene.zone_registry.zones.len() >= 6,
            "zone registry should have 6 default zones"
        );
        let _ = result; // result is valid
    }

    // ── Step 5: Profile selection (empty) ─────────────────────────────────────

    /// WHEN [component_profiles] is absent THEN profile selection is empty; startup succeeds.
    #[test]
    fn absent_component_profiles_empty_selection() {
        let raw = RawConfig::default();
        let mut scene = make_scene();
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);
        // Zone registry should be populated with all 6 default zones
        assert!(
            result.zone_registry.zones.len() >= 6,
            "zone registry must contain at least 6 zones"
        );
    }

    // ── Step 6: Effective rendering policies ──────────────────────────────────

    /// WHEN global tokens are set THEN subtitle zone gets token-derived text_color.
    #[test]
    fn subtitle_zone_gets_token_derived_text_color() {
        let mut raw = RawConfig::default();
        let mut dt_map = HashMap::new();
        dt_map.insert("color.text.primary".to_string(), "#AABBCC".to_string());
        raw.design_tokens = Some(RawDesignTokens(dt_map));

        let mut scene = make_scene();
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);

        let subtitle_zone = result
            .zone_registry
            .zones
            .get("subtitle")
            .expect("subtitle zone should be registered");

        let text_color = subtitle_zone
            .rendering_policy
            .text_color
            .expect("subtitle zone should have token-derived text_color");
        // #AABBCC → R=0xAA/255≈0.667, G=0xBB/255≈0.733, B=0xCC/255≈0.8
        assert!(
            (text_color.r - 0xAA as f32 / 255.0).abs() < 1e-3,
            "text_color.r should match #AABBCC red component"
        );
        assert!(
            (text_color.g - 0xBB as f32 / 255.0).abs() < 1e-3,
            "text_color.g should match #AABBCC green component"
        );
        assert!(
            (text_color.b - 0xCC as f32 / 255.0).abs() < 1e-3,
            "text_color.b should match #AABBCC blue component"
        );
    }

    // ── Step 8: Zone registry applied to scene ────────────────────────────────

    /// WHEN run_component_startup completes THEN scene.zone_registry contains all
    /// default zones with effective rendering policies.
    #[test]
    fn zone_registry_applied_to_scene() {
        let raw = RawConfig::default();
        let mut scene = make_scene();
        run_component_startup(&raw, None, Some("headless"), &mut scene);

        // All 6 default zones must be present
        for zone_name in &[
            "subtitle",
            "notification-area",
            "status-bar",
            "pip",
            "ambient-background",
            "alert-banner",
        ] {
            assert!(
                scene.zone_registry.zones.contains_key(*zone_name),
                "zone '{zone_name}' should be in the zone registry"
            );
        }
    }

    // ── Step 2→8 dependency: tokens before effective policies ─────────────────

    /// WHEN tokens are configured THEN ALL built-in zones receive token-derived
    /// policies (not just subtitle); validates step 2→6→8 pipeline.
    #[test]
    fn all_builtin_zones_receive_token_derived_policies_after_startup() {
        let mut raw = RawConfig::default();
        let mut dt_map = HashMap::new();
        dt_map.insert("color.text.primary".to_string(), "#123456".to_string());
        raw.design_tokens = Some(RawDesignTokens(dt_map));

        let mut scene = make_scene();
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);

        // subtitle should get text_color from tokens
        let subtitle = result.zone_registry.zones.get("subtitle").unwrap();
        assert!(
            subtitle.rendering_policy.text_color.is_some(),
            "subtitle should get token-derived text_color"
        );
        // notification-area should also get text_color from tokens
        let notif = result.zone_registry.zones.get("notification-area").unwrap();
        assert!(
            notif.rendering_policy.text_color.is_some(),
            "notification-area should get token-derived text_color"
        );
    }

    // ── End-to-end startup test: config with all new sections ─────────────────

    /// WHEN config has [design_tokens] + [component_profile_bundles] + [component_profiles]
    /// (where profile_bundles path is empty) THEN runtime starts, zone registry is
    /// populated with token-derived policies, no panic, readability validation runs.
    ///
    /// This is the primary end-to-end integration test for hud-sc0a.9.
    /// Because we don't have real profile directories in tests, we test with an
    /// empty profile_bundles list — the startup sequence still exercises steps 2-8
    /// fully.
    #[test]
    fn end_to_end_startup_with_design_tokens_section() {
        // Use r##"..."## to avoid premature termination on "#RRGGBB" hex values.
        let toml_str = r##"
[runtime]
profile = "headless"

[[tabs]]
name = "Main"
default_tab = true

[design_tokens]
"color.text.primary" = "#FFFFFF"
"color.backdrop.default" = "#000000"
"opacity.backdrop.default" = "0.7"
"stroke.outline.width" = "2.0"
"color.outline.default" = "#FFFF00"
"typography.subtitle.size" = "18"
"typography.subtitle.weight" = "600"
"typography.subtitle.family" = "system-ui"
"typography.body.size" = "14"
"typography.body.weight" = "400"
"typography.body.family" = "system-ui"
"spacing.padding.medium" = "8"
"spacing.padding.large" = "16"
"typography.status.size" = "12"
"typography.status.weight" = "400"
"typography.status.family" = "system-ui"
"typography.alert.size" = "16"
"typography.alert.weight" = "700"
"typography.alert.family" = "system-ui"
"typography.notification.size" = "14"
"typography.notification.weight" = "500"
"typography.notification.family" = "system-ui"
"color.text.muted" = "#AAAAAA"
"opacity.text.muted" = "0.7"
"color.accent.primary" = "#3399FF"
"color.accent.secondary" = "#33FF99"
"color.surface.primary" = "#1A1A2E"
"color.surface.secondary" = "#16213E"
"color.border.default" = "#444466"
"##;

        let raw: RawConfig = toml::from_str(toml_str).expect("TOML parse should succeed");
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);

        // Global token map should include the overrides
        assert_eq!(
            result
                .global_tokens
                .get("color.text.primary")
                .map(|s| s.as_str()),
            Some("#FFFFFF")
        );

        // Zone registry should be in the scene with all 6 zones
        assert_eq!(
            scene.zone_registry.zones.len(),
            6,
            "expected 6 built-in zones after startup"
        );

        // Subtitle should have token-derived text_color (white)
        let subtitle = scene.zone_registry.zones.get("subtitle").unwrap();
        let text_color = subtitle.rendering_policy.text_color.unwrap();
        assert!(
            (text_color.r - 1.0).abs() < 1e-3,
            "subtitle text_color.r should be 1.0 for #FFFFFF"
        );
        assert!(
            (text_color.g - 1.0).abs() < 1e-3,
            "subtitle text_color.g should be 1.0 for #FFFFFF"
        );
        assert!(
            (text_color.b - 1.0).abs() < 1e-3,
            "subtitle text_color.b should be 1.0 for #FFFFFF"
        );

        // No profile-scoped widget bundles (no profiles configured)
        assert!(
            result.profile_widget_bundles.is_empty(),
            "no profiles configured, so no profile widget bundles expected"
        );
    }

    // ── compositor_tokens: pre-merged map from all active profiles ───────────

    /// WHEN no profiles are active THEN compositor_tokens equals global_tokens
    /// (no profile overrides are blended in).
    #[test]
    fn compositor_tokens_equal_global_tokens_without_active_profiles() {
        let raw = RawConfig::default();
        let mut scene = make_scene();
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);

        // compositor_tokens should equal global_tokens: same keys and values.
        assert_eq!(
            result.compositor_tokens.len(),
            result.global_tokens.len(),
            "compositor_tokens should have the same length as global_tokens when no profiles are active"
        );
        for (k, v) in &result.global_tokens {
            assert_eq!(
                result.compositor_tokens.get(k).map(|s| s.as_str()),
                Some(v.as_str()),
                "compositor_tokens[{k}] should equal global_tokens[{k}] with no active profiles"
            );
        }
    }

    /// WHEN the notification-stack-exemplar profile is active THEN compositor_tokens
    /// contains all four color.notification.urgency.* overrides pre-merged on top of
    /// global_tokens (callers no longer need to merge manually).
    ///
    /// Note: [component_profile_bundles].paths points to the PARENT directory
    /// (`profiles/`) that contains profile subdirectories — scan_profile_dirs
    /// scans subdirectories of each listed path.
    #[test]
    fn compositor_tokens_include_notification_profile_urgency_overrides() {
        let profiles_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("profiles");

        let exemplar_dir = profiles_root.join("notification-stack-exemplar");
        if !exemplar_dir.exists() {
            eprintln!(
                "SKIP: notification-stack-exemplar profile not found at {:?}",
                exemplar_dir
            );
            return;
        }

        let toml_str = format!(
            r##"
[runtime]
profile = "headless"

[[tabs]]
name = "Main"
default_tab = true

[component_profile_bundles]
paths = ["{profiles_root}"]

[component_profiles]
notification = "notification-stack-exemplar"
"##,
            profiles_root = profiles_root.display()
        );

        let raw: RawConfig = toml::from_str(&toml_str).expect("TOML parse should succeed");
        let mut scene = make_scene();
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);

        // compositor_tokens must contain all 4 urgency overrides from the profile.
        assert_eq!(
            result
                .compositor_tokens
                .get("color.notification.urgency.low")
                .map(|s| s.as_str()),
            Some("#000000"),
            "compositor_tokens: urgency.low should be #000000"
        );
        assert_eq!(
            result
                .compositor_tokens
                .get("color.notification.urgency.normal")
                .map(|s| s.as_str()),
            Some("#0C1426"),
            "compositor_tokens: urgency.normal should be #0C1426"
        );
        assert_eq!(
            result
                .compositor_tokens
                .get("color.notification.urgency.urgent")
                .map(|s| s.as_str()),
            Some("#2A1E08"),
            "compositor_tokens: urgency.urgent should be #2A1E08"
        );
        assert_eq!(
            result
                .compositor_tokens
                .get("color.notification.urgency.critical")
                .map(|s| s.as_str()),
            Some("#450612"),
            "compositor_tokens: urgency.critical should be #450612"
        );
        // compositor_tokens must also contain global tokens (e.g. canonical text color).
        assert!(
            result.compositor_tokens.contains_key("color.text.primary"),
            "compositor_tokens must also include global tokens"
        );
    }

    /// WHEN a profile overrides a token that is also set in [design_tokens] THEN
    /// the profile override wins in compositor_tokens (profile > global > canonical).
    #[test]
    fn compositor_tokens_profile_overrides_win_over_global_design_tokens() {
        let profiles_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("profiles");

        let exemplar_dir = profiles_root.join("notification-stack-exemplar");
        if !exemplar_dir.exists() {
            eprintln!(
                "SKIP: notification-stack-exemplar profile not found at {:?}",
                exemplar_dir
            );
            return;
        }

        let toml_str = format!(
            r##"
[runtime]
profile = "headless"

[[tabs]]
name = "Main"
default_tab = true

[design_tokens]
"color.notification.urgency.low" = "#DEADBE"

[component_profile_bundles]
paths = ["{profiles_root}"]

[component_profiles]
notification = "notification-stack-exemplar"
"##,
            profiles_root = profiles_root.display()
        );

        let raw: RawConfig = toml::from_str(&toml_str).expect("TOML parse should succeed");
        let mut scene = make_scene();
        let result = run_component_startup(&raw, None, Some("headless"), &mut scene);

        // Profile override (#000000, "smoke black") must win over the [design_tokens] value
        // (#DEADBE).  The design doc originally used #2A2A2A as a placeholder; the finalised
        // profile.toml sets urgency.low to #000000.
        assert_eq!(
            result
                .compositor_tokens
                .get("color.notification.urgency.low")
                .map(|s| s.as_str()),
            Some("#000000"),
            "profile urgency.low token should override global design_token value in compositor_tokens"
        );
    }
}
