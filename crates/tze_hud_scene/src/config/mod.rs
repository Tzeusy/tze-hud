//! Configuration loader trait for v1.
//!
//! Encodes the configuration specification from
//! `configuration/spec.md §Requirement: TOML Configuration Format`
//! and related requirements.  This module defines **only** the trait contract
//! and supporting types — no implementation is provided here.

// ─── Error Codes ─────────────────────────────────────────────────────────────

/// Stable configuration error codes.
///
/// From spec §Requirement: Structured Validation Error Collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigErrorCode {
    ParseError,
    NoTabs,
    DuplicateTabName,
    MultipleDefaultTabs,
    UnknownLayout,
    UnknownProfile,
    MobileProfileNotExercised,
    HeadlessNotExtendable,
    ProfileExtendsConflictsWithProfile,
    ProfileBudgetEscalation,
    ProfileCapabilityEscalation,
    UnknownZoneType,
    UnknownCapability,
    ReservedEventPrefix,
    InvalidEventName,
    UnknownClassification,
    UnknownViewerClass,
    UnknownInterruptionClass,
    AgentBudgetExceedsProfile,
    InvalidReservedFraction,
    InvalidFpsRange,
    DegradationThresholdOrder,
    ConfigIncludesNotSupported,
    /// `[widget_bundles].paths` entry does not exist on disk.
    WidgetBundlePathNotFound,
    /// `[[tabs.widgets]]` entry references a widget type not loaded from any bundle.
    UnknownWidgetType,
    /// `[[tabs.widgets]]` `initial_params` fails schema validation.
    WidgetInvalidInitialParams,
    /// Two bundles declare the same widget type name.
    WidgetBundleDuplicateType,
    /// A key in `[design_tokens]` does not match the required pattern.
    InvalidTokenKey,
    /// A token value string could not be parsed into the expected format.
    TokenValueParseError,
    /// A profile's `component_type` field does not match any known v1 component type.
    ProfileUnknownComponentType,
    /// Two profile directories declare the same profile name.
    ConfigProfileDuplicateName,
    /// A configured component profile bundle path does not exist on disk.
    ConfigProfilePathNotFound,
    /// A profile's zone override file governs a zone not owned by the profile's component type.
    ProfileZoneOverrideMismatch,
    /// A zone override field has an invalid value or type.
    ProfileInvalidZoneOverride,
    /// A `{{token.key}}` reference in a zone override field could not be resolved.
    ProfileUnresolvedToken,
    /// A profile's effective RenderingPolicy fails the component type's readability check.
    ///
    /// Wire code: `PROFILE_READABILITY_VIOLATION`
    ProfileReadabilityViolation,
    /// A `[component_profiles]` key is not a recognized v1 component type name.
    ConfigUnknownComponentType,
    /// A `[component_profiles]` value does not match any loaded profile.
    ConfigUnknownComponentProfile,
    /// A `[component_profiles]` entry maps a component type to a profile of a different type.
    ConfigProfileTypeMismatch,
    Other(String),
}

/// A single structured validation error.
///
/// From spec §Requirement: Structured Validation Error Collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub code: ConfigErrorCode,
    /// Dotted path to the offending field (e.g., `"runtime.profile"`).
    pub field_path: String,
    pub expected: String,
    pub got: String,
    /// Machine-readable correction suggestion.
    pub hint: String,
}

/// Parse error with line and column information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    /// 1-indexed line number of the error.
    pub line: u32,
    /// 1-indexed column number of the error.
    pub column: u32,
}

// ─── Built-in Profiles ────────────────────────────────────────────────────────

/// A resolved display profile with its budget values.
///
/// From spec §Requirement: Display Profile full-display and related.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayProfile {
    pub name: String,
    pub max_tiles: u32,
    pub max_texture_mb: u32,
    pub max_agents: u32,
    /// Maximum agent update rate in Hz (per-agent state-stream ceiling).
    ///
    /// Per-agent `max_update_hz` MUST NOT exceed this value.
    /// Violations produce `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE`.
    pub max_agent_update_hz: u32,
    pub target_fps: u32,
    pub min_fps: u32,
    pub allow_background_zones: bool,
    pub allow_chrome_zones: bool,
}

impl DisplayProfile {
    /// Returns the `full-display` profile defaults.
    pub fn full_display() -> Self {
        DisplayProfile {
            name: "full-display".into(),
            max_tiles: 1024,
            max_texture_mb: 2048,
            max_agents: 16,
            max_agent_update_hz: 60,
            target_fps: 60,
            min_fps: 30,
            allow_background_zones: true,
            allow_chrome_zones: true,
        }
    }

    /// Returns the `headless` profile defaults.
    pub fn headless() -> Self {
        DisplayProfile {
            name: "headless".into(),
            max_tiles: 256,
            max_texture_mb: 512,
            max_agents: 8,
            max_agent_update_hz: 60,
            target_fps: 60,
            min_fps: 1,
            allow_background_zones: false,
            allow_chrome_zones: false,
        }
    }
}

// ─── Resolved Config ──────────────────────────────────────────────────────────

/// A fully validated, frozen configuration.
///
/// Returned by `ConfigLoader::freeze()`.
#[derive(Clone, Debug)]
pub struct ResolvedConfig {
    pub profile: DisplayProfile,
    pub tab_names: Vec<String>,
    pub agent_capabilities: std::collections::HashMap<String, Vec<String>>,
    /// Sourced TOML file path.
    pub source_path: Option<String>,
}

// ─── ConfigLoader Trait ───────────────────────────────────────────────────────

/// Trait encoding the configuration loading and validation contract.
///
/// Implementations must:
/// - Accept only TOML with parse errors including line/column.
/// - Search configuration file chain (CLI → env → cwd → XDG) in order.
/// - Enforce built-in profile budget values exactly.
/// - Reject `profile = "mobile"` with `CONFIG_MOBILE_PROFILE_NOT_EXERCISED`.
/// - Prevent budget escalation in custom profiles.
/// - Validate capability names against the canonical v1 vocabulary.
/// - Collect ALL validation errors before reporting.
/// - Reject `includes` fields (post-v1 reserved).
pub trait ConfigLoader {
    /// Parse a TOML configuration string.
    ///
    /// Returns `Err(ParseError)` with line and column if the TOML is invalid.
    fn parse(toml_src: &str) -> Result<Self, ParseError>
    where
        Self: Sized;

    /// Apply normalisation rules (resolve profile, fill defaults).
    fn normalize(&mut self);

    /// Validate all fields.  Returns ALL validation errors (never stops at first).
    fn validate(&self) -> Vec<ConfigError>;

    /// Freeze the validated config into a `ResolvedConfig`.
    ///
    /// Returns `Err` (with all errors) if validation fails.
    fn freeze(self) -> Result<ResolvedConfig, Vec<ConfigError>>;

    /// Resolve the file path to use according to the search chain:
    /// (1) `cli_path`, (2) `$TZE_HUD_CONFIG` env, (3) `./tze_hud.toml`,
    /// (4) XDG config.
    ///
    /// Returns `Ok(path)` for the first path found; returns `Err(searched_paths)` when
    /// no file is found, where `searched_paths` is the ordered list of paths that were tried.
    fn resolve_config_path(cli_path: Option<&str>) -> Result<String, Vec<String>>
    where
        Self: Sized;

    /// Returns `true` if the capability name is in the canonical v1 vocabulary.
    ///
    /// From spec §Requirement: Capability Vocabulary.
    fn is_known_capability(name: &str) -> bool
    where
        Self: Sized;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Canonical v1 capability names (from spec §Requirement: Capability Vocabulary).
pub const CANONICAL_CAPABILITIES: &[&str] = &[
    "create_tiles",
    "modify_own_tiles",
    "manage_tabs",
    "manage_sync_groups",
    "upload_resource",
    "read_scene_topology",
    "subscribe_scene_events",
    "overlay_privileges",
    "access_input_events",
    "high_priority_z_order",
    "exceed_default_budgets",
    "read_telemetry",
    "resident_mcp",
    // Parameterized — wildcard or specific zone.
    "publish_zone:*",
    // Parameterized — wildcard or specific widget.
    "publish_widget:*",
    // "publish_zone:<zone_name>", "publish_widget:<widget_name>",
    // "emit_scene_event:<name>", and "lease:priority:<N>"
    // are validated by prefix pattern, not exact match.
];

/// Returns `true` if `name` is a valid v1 capability per the canonical vocabulary.
///
/// Parameterized forms are validated:
/// - `publish_zone:<name>`: suffix must be non-empty.
/// - `emit_scene_event:<event>`: suffix must be non-empty and must not start with reserved
///   prefixes (`scene.` or `system.`).
/// - `lease:priority:<N>`: suffix must be a non-empty numeric value.
pub fn is_canonical_capability(name: &str) -> bool {
    // Exact matches.
    if CANONICAL_CAPABILITIES.contains(&name) {
        return true;
    }
    // Parameterized forms with validation.
    if let Some(zone_name) = name.strip_prefix("publish_zone:") {
        return !zone_name.is_empty();
    }
    if let Some(widget_name) = name.strip_prefix("publish_widget:") {
        return !widget_name.is_empty();
    }
    if let Some(event_name) = name.strip_prefix("emit_scene_event:") {
        if event_name.is_empty() {
            return false;
        }
        // Reserved event prefixes must not be usable as capability grants.
        if event_name.starts_with("scene.") || event_name.starts_with("system.") {
            return false;
        }
        return true;
    }
    if let Some(priority_str) = name.strip_prefix("lease:priority:") {
        if priority_str.is_empty() {
            return false;
        }
        return priority_str.parse::<u32>().is_ok();
    }
    false
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;

    // ── 1. TOML parsing ───────────────────────────────────────────────────────

    /// WHEN valid TOML provided THEN parse succeeds.
    pub fn test_valid_toml_accepted<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
"#;
        let result = L::parse(toml);
        assert!(result.is_ok(), "valid TOML should be accepted");
    }

    /// WHEN TOML has syntax error THEN parse error includes line and column.
    pub fn test_parse_error_includes_line_column<L: ConfigLoader>() {
        let bad_toml = "this is not = valid toml [\n";
        let result = L::parse(bad_toml);
        match result {
            Err(ParseError { line, column, .. }) => {
                assert!(line >= 1, "line should be >= 1, got {line}");
                assert!(column >= 1, "column should be >= 1, got {column}");
            }
            Ok(_) => panic!("invalid TOML should have failed to parse"),
        }
    }

    // ── 2. Mobile profile rejection ───────────────────────────────────────────

    /// WHEN profile = "mobile" THEN rejected with CONFIG_MOBILE_PROFILE_NOT_EXERCISED.
    pub fn test_mobile_profile_rejected<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "mobile"

[[tabs]]
name = "Main"
"#;
        let loader = L::parse(toml).expect("parse should succeed even with mobile profile");
        let errors = loader.validate();
        let has_mobile_error = errors
            .iter()
            .any(|e| matches!(e.code, ConfigErrorCode::MobileProfileNotExercised));
        assert!(
            has_mobile_error,
            "mobile profile should produce CONFIG_MOBILE_PROFILE_NOT_EXERCISED"
        );
    }

    // ── 3. Budget escalation prevention ───────────────────────────────────────

    /// WHEN custom profile exceeds base max_tiles THEN CONFIG_PROFILE_BUDGET_ESCALATION.
    pub fn test_budget_escalation_rejected<L: ConfigLoader>() {
        // Custom profile extending full-display with max_tiles > 1024.
        let toml = r#"
[runtime]
profile = "custom"

[display_profile]
extends = "full-display"
max_tiles = 2048

[[tabs]]
name = "Main"
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let errors = loader.validate();
        let has_escalation = errors
            .iter()
            .any(|e| matches!(e.code, ConfigErrorCode::ProfileBudgetEscalation));
        assert!(
            has_escalation,
            "max_tiles=2048 exceeding base 1024 should produce BUDGET_ESCALATION"
        );
    }

    /// WHEN custom profile exceeds base boolean capability THEN CONFIG_PROFILE_CAPABILITY_ESCALATION.
    pub fn test_capability_escalation_rejected<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "custom"

[display_profile]
extends = "headless"
allow_background_zones = true

[[tabs]]
name = "Main"
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let errors = loader.validate();
        let has_cap_escalation = errors
            .iter()
            .any(|e| matches!(e.code, ConfigErrorCode::ProfileCapabilityEscalation));
        assert!(
            has_cap_escalation,
            "allow_background_zones=true over headless base should be CAPABILITY_ESCALATION"
        );
    }

    // ── 4. Capability vocabulary validation ────────────────────────────────────

    /// WHEN unknown capability in agent config THEN CONFIG_UNKNOWN_CAPABILITY.
    pub fn test_unknown_capability_rejected<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[agents.registered.agent_a]
capabilities = ["createTiles"]
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let errors = loader.validate();
        let has_cap_error = errors
            .iter()
            .any(|e| matches!(e.code, ConfigErrorCode::UnknownCapability));
        assert!(
            has_cap_error,
            "non-canonical capability 'createTiles' should produce UNKNOWN_CAPABILITY"
        );
    }

    /// WHEN valid canonical capability names used THEN no error.
    #[test]
    fn test_canonical_capability_names_valid() {
        assert!(is_canonical_capability("create_tiles"));
        assert!(is_canonical_capability("read_scene_topology"));
        assert!(is_canonical_capability("access_input_events"));
        assert!(is_canonical_capability("publish_zone:subtitle"));
        assert!(is_canonical_capability("emit_scene_event:doorbell.ring"));
        assert!(is_canonical_capability("lease:priority:1"));
        assert!(is_canonical_capability("resident_mcp"));
    }

    /// WHEN non-canonical capability names used THEN validation rejects them.
    #[test]
    fn test_non_canonical_capability_names_invalid() {
        // Pre-Round-14 names.
        assert!(
            !is_canonical_capability("read_scene"),
            "read_scene is pre-Round-14"
        );
        assert!(
            !is_canonical_capability("receive_input"),
            "receive_input is pre-Round-14"
        );
        assert!(
            !is_canonical_capability("zone_publish:subtitle"),
            "zone_publish is pre-Round-14"
        );
        // Wrong case / format.
        assert!(
            !is_canonical_capability("CREATE_TILE"),
            "uppercase not allowed"
        );
        assert!(
            !is_canonical_capability("create-tiles"),
            "kebab-case not allowed"
        );
    }

    // ── 5. Config includes rejected ────────────────────────────────────────────

    /// WHEN config includes used THEN rejected (v1 does not support includes).
    pub fn test_config_includes_rejected<L: ConfigLoader>() {
        let toml = r#"
includes = "/etc/tze_hud/base.toml"

[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let errors = loader.validate();
        let has_includes_error = errors
            .iter()
            .any(|e| matches!(e.code, ConfigErrorCode::ConfigIncludesNotSupported));
        assert!(
            has_includes_error,
            "includes field should produce CONFIG_INCLUDES_NOT_SUPPORTED"
        );
    }

    // ── 6. Minimal valid config ────────────────────────────────────────────────

    /// WHEN minimal config provided (runtime + one tab) THEN freeze succeeds.
    pub fn test_minimal_config_accepted<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Home"
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let resolved = loader.freeze();
        assert!(
            resolved.is_ok(),
            "minimal config should freeze successfully"
        );
        let config = resolved.unwrap();
        assert_eq!(config.tab_names, vec!["Home".to_string()]);
    }

    /// WHEN config missing tabs THEN CONFIG_NO_TABS.
    pub fn test_missing_tabs_rejected<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "full-display"
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let errors = loader.validate();
        let has_no_tabs = errors
            .iter()
            .any(|e| matches!(e.code, ConfigErrorCode::NoTabs));
        assert!(has_no_tabs, "no [[tabs]] should produce CONFIG_NO_TABS");
    }

    // ── 7. Full-display profile defaults ──────────────────────────────────────

    /// WHEN profile = "full-display" THEN correct budget values resolved.
    #[test]
    fn test_full_display_profile_budget_values() {
        let p = DisplayProfile::full_display();
        assert_eq!(p.max_tiles, 1024);
        assert_eq!(p.max_texture_mb, 2048);
        assert_eq!(p.max_agents, 16);
        assert_eq!(p.target_fps, 60);
        assert_eq!(p.min_fps, 30);
    }

    /// WHEN profile = "headless" THEN correct budget values resolved.
    #[test]
    fn test_headless_profile_budget_values() {
        let p = DisplayProfile::headless();
        assert_eq!(p.max_tiles, 256);
        assert_eq!(p.max_texture_mb, 512);
        assert_eq!(p.max_agents, 8);
        assert_eq!(p.target_fps, 60);
        assert_eq!(p.min_fps, 1);
    }

    // ── 8. Multiple errors collected ──────────────────────────────────────────

    /// WHEN config has unknown profile AND duplicate tab name THEN both errors reported.
    pub fn test_multiple_errors_collected<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "totally_unknown_profile"

[[tabs]]
name = "Dup"

[[tabs]]
name = "Dup"
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let errors = loader.validate();
        // Should have at least: UNKNOWN_PROFILE and DUPLICATE_TAB_NAME.
        assert!(
            errors.len() >= 2,
            "should collect multiple errors, got: {:?}",
            errors.iter().map(|e| &e.code).collect::<Vec<_>>()
        );
    }

    // ── 9. FPS range validation ────────────────────────────────────────────────

    /// WHEN target_fps < min_fps THEN CONFIG_INVALID_FPS_RANGE.
    pub fn test_fps_range_invalid<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "custom"

[display_profile]
extends = "full-display"
target_fps = 15
min_fps = 30

[[tabs]]
name = "Main"
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let errors = loader.validate();
        let has_fps_error = errors
            .iter()
            .any(|e| matches!(e.code, ConfigErrorCode::InvalidFpsRange));
        assert!(
            has_fps_error,
            "target_fps < min_fps should produce CONFIG_INVALID_FPS_RANGE"
        );
    }

    // ── 10. Reserved event prefix in capability ────────────────────────────────

    /// WHEN emit_scene_event:system.shutdown used in agent capabilities THEN rejected.
    pub fn test_reserved_event_prefix_in_capability_rejected<L: ConfigLoader>() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[agents.registered.agent_a]
capabilities = ["emit_scene_event:system.shutdown"]
"#;
        let loader = L::parse(toml).expect("parse should succeed");
        let errors = loader.validate();
        let has_reserved = errors
            .iter()
            .any(|e| matches!(e.code, ConfigErrorCode::ReservedEventPrefix));
        assert!(
            has_reserved,
            "emit_scene_event:system.* should produce CONFIG_RESERVED_EVENT_PREFIX"
        );
    }

    // ── Compile-time generic check ────────────────────────────────────────────

    #[test]
    #[ignore = "no implementation yet"]
    fn test_config_loader_generic_compile_check() {
        fn use_loader<L: ConfigLoader>() {
            let result =
                L::parse("[runtime]\nprofile = \"full-display\"\n[[tabs]]\nname = \"T\"\n");
            let _ = result;
        }
        // Call use_loader::<ConcreteImpl>() once an impl exists.
    }
}
