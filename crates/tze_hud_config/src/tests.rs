//! Integration tests for `TzeHudConfig`.
//!
//! Each test corresponds to a WHEN/THEN scenario from the issue acceptance
//! criteria (rig-j90m) and `configuration/spec.md`.
//!
//! NOTE: `tze_hud_scene::config::tests` is `#[cfg(test)]`-gated inside its own
//! crate and not accessible from here as a module. We inline equivalent test
//! logic directly rather than re-exporting.

use tze_hud_scene::config::{ConfigErrorCode, ConfigLoader, ParseError};
use crate::loader::TzeHudConfig;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn parse_ok(toml: &str) -> TzeHudConfig {
    TzeHudConfig::parse(toml).expect("parse should succeed for this TOML")
}

// ── Spec §TOML Configuration Format ──────────────────────────────────────────

/// WHEN valid TOML provided THEN parse succeeds.
#[test]
fn spec_valid_toml_accepted() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
"#;
    let result = TzeHudConfig::parse(toml);
    assert!(result.is_ok(), "valid TOML should be accepted");
}

/// WHEN invalid TOML THEN parse error includes line and column.
#[test]
fn spec_parse_error_includes_line_column() {
    let bad_toml = "this is not = valid toml [\n";
    let result = TzeHudConfig::parse(bad_toml);
    match result {
        Err(ParseError { line, column, .. }) => {
            assert!(line >= 1, "line should be >= 1, got {line}");
            assert!(column >= 1, "column should be >= 1, got {column}");
        }
        Ok(_) => panic!("invalid TOML should have failed to parse"),
    }
}

/// WHEN TOML has a syntax error THEN ParseError.line >= 1 and .column >= 1.
#[test]
fn spec_parse_error_line_column_are_one_indexed() {
    let bad_toml = "not = valid [\n";
    let result = TzeHudConfig::parse(bad_toml);
    match result {
        Err(ParseError { line, column, .. }) => {
            assert!(line >= 1, "line must be >= 1, got {line}");
            assert!(column >= 1, "column must be >= 1, got {column}");
        }
        Ok(_) => panic!("invalid TOML should fail"),
    }
}

// ── Spec §Configuration File Resolution Order ─────────────────────────────────

/// WHEN no config file found at any location THEN Err lists searched paths.
#[test]
fn spec_no_config_found_lists_searched_paths() {
    let result = TzeHudConfig::resolve_config_path(Some("/tmp/tze_hud_no_such_file_j90m_test.toml"));
    match result {
        Err(paths) => {
            assert!(!paths.is_empty(), "searched paths must be listed");
        }
        Ok(_) => panic!("should not have found a non-existent file"),
    }
}

/// WHEN --config /path/to/custom.toml specified THEN only that path is used.
#[test]
fn spec_cli_path_takes_precedence() {
    let dir = std::env::temp_dir();
    let cli_file = dir.join("tze_hud_j90m_cli_precedence.toml");
    std::fs::write(&cli_file, b"[runtime]\nprofile = \"headless\"\n[[tabs]]\nname = \"T\"\n")
        .unwrap();

    let result = TzeHudConfig::resolve_config_path(Some(cli_file.to_str().unwrap()));
    assert!(result.is_ok(), "CLI path should be found");
    assert_eq!(result.unwrap(), cli_file.to_string_lossy().as_ref());

    let _ = std::fs::remove_file(&cli_file);
}

// ── Spec §Minimal Valid Configuration ─────────────────────────────────────────

/// WHEN minimal config (runtime + one tab) THEN freeze succeeds.
#[test]
fn spec_minimal_config_accepted() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Home"
"#;
    let loader = parse_ok(toml);
    let resolved = loader.freeze();
    assert!(resolved.is_ok(), "minimal config should freeze successfully");
    let config = resolved.unwrap();
    assert_eq!(config.tab_names, vec!["Home".to_string()]);
}

/// WHEN config has [runtime] but no [[tabs]] THEN CONFIG_NO_TABS.
#[test]
fn spec_missing_tabs_rejected_with_config_no_tabs() {
    let toml = r#"
[runtime]
profile = "full-display"
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let has_no_tabs = errors.iter().any(|e| matches!(e.code, ConfigErrorCode::NoTabs));
    assert!(has_no_tabs, "no [[tabs]] should produce CONFIG_NO_TABS");
}

// ── Spec §Layered Config Composition (v1-reserved) ────────────────────────────

/// WHEN config has `includes` field THEN startup error (post-v1 reserved).
#[test]
fn spec_includes_field_rejected() {
    let toml = r#"
includes = "/etc/tze_hud/base.toml"

[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let has_includes_error = errors.iter().any(|e| {
        matches!(e.code, ConfigErrorCode::ConfigIncludesNotSupported)
    });
    assert!(has_includes_error, "includes field should produce CONFIG_INCLUDES_NOT_SUPPORTED");
}

// ── Spec §Structured Validation Error Collection ──────────────────────────────

/// WHEN multiple validation errors exist THEN all are reported together.
#[test]
fn spec_multiple_errors_collected() {
    let toml = r#"
[runtime]
profile = "totally_unknown_profile"

[[tabs]]
name = "Dup"

[[tabs]]
name = "Dup"
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    // Should have at least: UNKNOWN_PROFILE and DUPLICATE_TAB_NAME.
    assert!(
        errors.len() >= 2,
        "should collect multiple errors, got: {:?}",
        errors.iter().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── Spec §Tab Configuration Validation ───────────────────────────────────────

/// WHEN two tabs share name "Morning" THEN CONFIG_DUPLICATE_TAB_NAME.
#[test]
fn spec_duplicate_tab_name_rejected() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Morning"

[[tabs]]
name = "Morning"
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let dup_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e.code, ConfigErrorCode::DuplicateTabName))
        .collect();
    assert!(!dup_errors.is_empty(), "should have CONFIG_DUPLICATE_TAB_NAME error");
    assert!(
        dup_errors[0].field_path.contains("tabs"),
        "field_path should reference tabs"
    );
}

/// WHEN two tabs both set default_tab = true THEN CONFIG_MULTIPLE_DEFAULT_TABS.
#[test]
fn spec_multiple_default_tabs_rejected() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "A"
default_tab = true

[[tabs]]
name = "B"
default_tab = true
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    assert!(
        errors.iter().any(|e| matches!(e.code, ConfigErrorCode::MultipleDefaultTabs)),
        "should have CONFIG_MULTIPLE_DEFAULT_TABS error, got: {:?}",
        errors.iter().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── Spec §Reserved Fraction Validation ───────────────────────────────────────

/// WHEN reserved_top + reserved_bottom >= 1.0 THEN CONFIG_INVALID_RESERVED_FRACTION.
#[test]
fn spec_reserved_fractions_sum_to_one_rejected() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[tabs.layout]
reserved_top_fraction = 0.5
reserved_bottom_fraction = 0.5
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    assert!(
        errors.iter().any(|e| matches!(e.code, ConfigErrorCode::InvalidReservedFraction)),
        "reserved_top + reserved_bottom = 1.0 should produce CONFIG_INVALID_RESERVED_FRACTION, got: {:?}",
        errors.iter().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── Spec §FPS Range Validation ────────────────────────────────────────────────

/// WHEN target_fps < min_fps THEN CONFIG_INVALID_FPS_RANGE.
#[test]
fn spec_fps_range_target_below_min_rejected() {
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
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let has_fps_error = errors.iter().any(|e| matches!(e.code, ConfigErrorCode::InvalidFpsRange));
    assert!(has_fps_error, "target_fps < min_fps should produce CONFIG_INVALID_FPS_RANGE");
}

// ── Spec §Degradation Threshold Ordering ──────────────────────────────────────

/// WHEN degradation thresholds are out of order THEN CONFIG_DEGRADATION_THRESHOLD_ORDER.
#[test]
fn spec_degradation_out_of_order_rejected() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[degradation]
shed_tiles_frame_ms = 12.0
coalesce_frame_ms = 14.0
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    assert!(
        errors.iter().any(|e| matches!(e.code, ConfigErrorCode::DegradationThresholdOrder)),
        "out-of-order thresholds should produce CONFIG_DEGRADATION_THRESHOLD_ORDER, got: {:?}",
        errors.iter().map(|e| &e.code).collect::<Vec<_>>()
    );
}

// ── Spec §Scene Event Naming Convention ──────────────────────────────────────

/// WHEN tab_switch_on_event = "doorbell.ring" THEN accepted.
#[test]
fn spec_valid_event_name_accepted() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
tab_switch_on_event = "doorbell.ring"
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let event_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e.code, ConfigErrorCode::InvalidEventName))
        .collect();
    assert!(event_errors.is_empty(), "valid event name should not produce errors");
}

/// WHEN tab_switch_on_event = "Doorbell-Ring" THEN CONFIG_INVALID_EVENT_NAME.
#[test]
fn spec_invalid_event_name_rejected() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
tab_switch_on_event = "Doorbell-Ring"
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    assert!(
        errors.iter().any(|e| matches!(e.code, ConfigErrorCode::InvalidEventName)),
        "invalid event name should produce CONFIG_INVALID_EVENT_NAME, got: {:?}",
        errors.iter().map(|e| &e.code).collect::<Vec<_>>()
    );
}

/// WHEN tab_switch_on_event = "" THEN accepted with no warning.
#[test]
fn spec_empty_event_name_accepted_no_warning() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
tab_switch_on_event = ""
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let event_errors: Vec<_> = errors
        .iter()
        .filter(|e| matches!(e.code, ConfigErrorCode::InvalidEventName))
        .collect();
    assert!(event_errors.is_empty(), "empty event name must not produce an error");
}

// ── Spec §Schema Export ───────────────────────────────────────────────────────

/// WHEN schema_value() is called THEN valid JSON Schema returned.
#[test]
fn spec_schema_export_produces_valid_json_schema() {
    let schema = crate::schema::schema_value();
    assert!(schema.is_object(), "schema must be a JSON object");
}

// ── Spec §Mobile Profile ──────────────────────────────────────────────────────

/// WHEN profile = "mobile" THEN CONFIG_MOBILE_PROFILE_NOT_EXERCISED.
#[test]
fn spec_mobile_profile_rejected() {
    let toml = r#"
[runtime]
profile = "mobile"

[[tabs]]
name = "Main"
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let has_mobile_error = errors.iter().any(|e| {
        matches!(e.code, ConfigErrorCode::MobileProfileNotExercised)
    });
    assert!(has_mobile_error, "mobile profile should produce CONFIG_MOBILE_PROFILE_NOT_EXERCISED");
}

// ── Spec §Capability Vocabulary ───────────────────────────────────────────────

/// WHEN non-canonical capability in agent config THEN CONFIG_UNKNOWN_CAPABILITY.
#[test]
fn spec_unknown_capability_rejected() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[agents.registered.agent_a]
capabilities = ["createTiles"]
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let has_cap_error = errors.iter().any(|e| {
        matches!(e.code, ConfigErrorCode::UnknownCapability)
    });
    assert!(has_cap_error, "non-canonical capability 'createTiles' should produce UNKNOWN_CAPABILITY");
}

/// WHEN emit_scene_event:system.shutdown used THEN CONFIG_RESERVED_EVENT_PREFIX.
#[test]
fn spec_reserved_event_prefix_in_capability_rejected() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[agents.registered.agent_a]
capabilities = ["emit_scene_event:system.shutdown"]
"#;
    let loader = parse_ok(toml);
    let errors = loader.validate();
    let has_reserved = errors.iter().any(|e| {
        matches!(e.code, ConfigErrorCode::ReservedEventPrefix)
    });
    assert!(has_reserved, "emit_scene_event:system.* should produce CONFIG_RESERVED_EVENT_PREFIX");
}

// ── Spec §Profile Budget / Capability Escalation (delegated) ─────────────────

/// budget escalation check is delegated to rig-umgy
#[test]
#[ignore = "profile budget escalation check delegated to rig-umgy"]
fn spec_budget_escalation_rejected() {}

/// capability escalation check is delegated to rig-umgy
#[test]
#[ignore = "profile capability escalation check delegated to rig-umgy"]
fn spec_capability_escalation_rejected() {}

// ── freeze / ResolvedConfig ───────────────────────────────────────────────────

/// WHEN minimal config frozen THEN tab_names contains the tab.
#[test]
fn spec_freeze_populates_tab_names() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Dashboard"
"#;
    let loader = parse_ok(toml);
    let resolved = loader.freeze().expect("freeze should succeed");
    assert_eq!(resolved.tab_names, vec!["Dashboard".to_string()]);
}

/// WHEN profile = "headless" THEN resolved profile has headless budget.
#[test]
fn spec_freeze_headless_profile_budget_values() {
    let toml = r#"
[runtime]
profile = "headless"

[[tabs]]
name = "T"
"#;
    let loader = parse_ok(toml);
    let resolved = loader.freeze().expect("freeze should succeed");
    assert_eq!(resolved.profile.max_tiles, 256);
    assert_eq!(resolved.profile.max_texture_mb, 512);
    assert_eq!(resolved.profile.max_agents, 8);
}

/// WHEN config has validation errors THEN freeze returns Err.
#[test]
fn spec_freeze_returns_err_on_validation_errors() {
    let toml = r#"
[runtime]
profile = "full-display"
"#;
    let loader = parse_ok(toml);
    let result = loader.freeze();
    assert!(result.is_err(), "freeze should fail when there are no tabs");
}

/// WHEN profile = "full-display" THEN resolved profile has correct budget values.
#[test]
fn spec_full_display_profile_budget_values() {
    let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "T"
"#;
    let loader = parse_ok(toml);
    let resolved = loader.freeze().expect("freeze should succeed");
    assert_eq!(resolved.profile.max_tiles, 1024);
    assert_eq!(resolved.profile.max_texture_mb, 2048);
    assert_eq!(resolved.profile.max_agents, 16);
    assert_eq!(resolved.profile.target_fps, 60);
    assert_eq!(resolved.profile.min_fps, 30);
}
