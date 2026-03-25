//! Hot-reload support — rig-mop4.
//!
//! Implements spec `configuration/spec.md` requirements:
//!
//! - **Configuration Reload** (lines 263-274, v1-mandatory)
//!   SIGHUP and `RuntimeService.ReloadConfig` gRPC trigger a live reload.
//!   Hot-reloadable fields: `[privacy]`, `[degradation]`, `[chrome]`,
//!   `[agents.dynamic_policy]`.
//!   Frozen fields (require restart): `[runtime]`, `[[tabs]]`,
//!   `[agents.registered]`.
//!   On reload: entire config re-validated; validation errors returned without
//!   applying new config.
//!
//! ## Field Classification
//!
//! | Section                 | Reload behaviour |
//! |-------------------------|-----------------|
//! | `[runtime]`             | Frozen (restart required) |
//! | `[[tabs]]`              | Frozen (restart required) |
//! | `[agents.registered]`   | Frozen (restart required) |
//! | `[privacy]`             | Hot-reloadable |
//! | `[degradation]`         | Hot-reloadable |
//! | `[chrome]`              | Hot-reloadable |
//! | `[agents.dynamic_policy]` | Hot-reloadable |
//!
//! ## Design Note
//!
//! This module provides:
//! 1. `FieldClassification` enum — documents which sections are frozen/hot.
//! 2. `HotReloadableConfig` — the subset of config that survives a SIGHUP.
//! 3. `reload_config` — parses, validates, and returns a new `HotReloadableConfig`
//!    or the validation errors. Never mutates state; the caller applies the result.
//! 4. `SighupHandler` — a thin wrapper around the UNIX `SIGHUP` signal with a
//!    callback mechanism. On non-Unix targets this is a no-op stub.
//!
//! The actual state storage and application live outside this module (typically
//! in the runtime or a `Config` actor), because those involve async coordination
//! and are out of scope for the pure validation layer.

use tze_hud_scene::config::{ConfigError, ConfigErrorCode};

use crate::loader::TzeHudConfig;
use crate::raw::{RawChrome, RawDegradation, RawDynamicPolicy, RawPrivacy};

// ─── Field classification ──────────────────────────────────────────────────────

/// Whether a configuration section can be reloaded without a restart.
///
/// From spec §Configuration Reload (lines 263-264).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldClassification {
    /// Field is frozen at startup; changes require a full restart.
    Frozen,
    /// Field can be reloaded live via SIGHUP or `ReloadConfig` RPC.
    HotReloadable,
}

/// Returns the reload classification for a top-level configuration section.
///
/// `section_path` is the dotted section name (e.g., `"runtime"`, `"privacy"`).
pub fn section_classification(section_path: &str) -> FieldClassification {
    match section_path {
        // Hot-reloadable sections.
        "privacy" | "degradation" | "chrome" | "agents.dynamic_policy" => {
            FieldClassification::HotReloadable
        }
        // Everything else is frozen at startup.
        _ => FieldClassification::Frozen,
    }
}

// ─── Hot-reloadable config subset ─────────────────────────────────────────────

/// The subset of configuration that can be updated via a live reload.
///
/// Produced by `reload_config` when the new TOML is valid.
/// The caller is responsible for atomically applying this to the running state.
#[derive(Clone, Debug)]
pub struct HotReloadableConfig {
    /// Updated `[privacy]` section (or defaults if absent from new TOML).
    pub privacy: RawPrivacy,
    /// Updated `[degradation]` section (or defaults if absent).
    pub degradation: RawDegradation,
    /// Updated `[chrome]` section (or defaults if absent).
    pub chrome: RawChrome,
    /// Updated `[agents.dynamic_policy]` (or `None` if absent — disables dynamic agents).
    pub dynamic_policy: Option<RawDynamicPolicy>,
}

// ─── Reload entry point ────────────────────────────────────────────────────────

/// Parse and validate a new TOML configuration string for a live reload.
///
/// On success, returns the `HotReloadableConfig` extracted from the new TOML.
/// On failure (parse error or validation errors), returns `Err` with the errors.
///
/// The running configuration is NEVER modified by this function — the caller
/// applies the result only if `Ok(...)` is returned.
///
/// Per spec §Configuration Reload:
/// > On reload, the runtime MUST re-validate the entire config; validation errors
/// > MUST be returned without applying the new config.
pub fn reload_config(new_toml: &str) -> Result<HotReloadableConfig, Vec<ConfigError>> {
    use tze_hud_scene::config::ConfigLoader;

    // Step 1: parse TOML.
    let loader = TzeHudConfig::parse(new_toml).map_err(|parse_err| {
        vec![ConfigError {
            code: ConfigErrorCode::ParseError,
            field_path: String::new(),
            expected: "valid TOML".into(),
            got: parse_err.message.clone(),
            hint: format!(
                "fix the TOML syntax error at line {}, column {}",
                parse_err.line, parse_err.column
            ),
        }]
    })?;

    // Step 2: validate the entire config.
    let errors = loader.validate();
    if !errors.is_empty() {
        return Err(errors);
    }

    // Step 3: extract the hot-reloadable subset.
    let raw = loader.into_raw();
    let hot = HotReloadableConfig {
        privacy: raw.privacy.unwrap_or_default(),
        degradation: raw.degradation.unwrap_or_default(),
        chrome: raw.chrome.unwrap_or_default(),
        dynamic_policy: raw.agents.and_then(|a| a.dynamic_policy),
    };

    Ok(hot)
}

// ─── SIGHUP handler ───────────────────────────────────────────────────────────

/// A callback invoked when SIGHUP is received.
///
/// The callback receives the path to the config file to reload.
/// It is responsible for calling `reload_config` and applying the result.
pub type SighupCallback = Box<dyn Fn(&str) + Send + Sync>;

/// SIGHUP reload coordinator.
///
/// This struct **does not** install a real OS signal handler. It stores the
/// config file path and exposes `trigger_reload()` for programmatic invocation.
///
/// Production runtimes MUST integrate with `tokio::signal::unix::signal(SignalKind::hangup())`
/// to receive OS SIGHUP signals asynchronously. When the signal fires, call
/// `trigger_reload()` on this struct from the signal handling task.
///
/// The separation keeps signal delivery (async runtime concern) separate from
/// config parsing/validation (pure logic concern), making both independently
/// testable.
pub struct SighupHandler {
    config_path: String,
}

impl SighupHandler {
    /// Create a new SIGHUP handler for the given config file path.
    pub fn new(config_path: impl Into<String>) -> Self {
        SighupHandler {
            config_path: config_path.into(),
        }
    }

    /// Returns the config path registered with this handler.
    pub fn config_path(&self) -> &str {
        &self.config_path
    }

    /// Simulate a SIGHUP reload by loading the config from `config_path` and
    /// calling `callback` with the new `HotReloadableConfig`.
    ///
    /// Returns `Ok(())` if reload succeeded, `Err(errors)` if validation failed.
    ///
    /// In tests, call this directly instead of sending a real SIGHUP.
    pub fn trigger_reload(
        &self,
        on_success: impl FnOnce(HotReloadableConfig),
    ) -> Result<(), Vec<ConfigError>> {
        let toml_src = std::fs::read_to_string(&self.config_path).map_err(|io_err| {
            vec![ConfigError {
                code: ConfigErrorCode::Other("CONFIG_RELOAD_IO_ERROR".into()),
                field_path: "config_path".into(),
                expected: "readable config file".into(),
                got: format!("{io_err}"),
                hint: format!("ensure {:?} exists and is readable", self.config_path),
            }]
        })?;

        let hot = reload_config(&toml_src)?;
        on_success(hot);
        Ok(())
    }
}

// ─── TzeHudConfig accessor ────────────────────────────────────────────────────
//
// We need access to the raw config to extract hot-reloadable fields.
// Add a `into_raw` method to `TzeHudConfig` via a dedicated trait.

/// Extension that exposes the inner `RawConfig` for extraction.
///
/// Intentionally crate-private — `reload_config` is the public API for reload.
/// External callers have no need to access the raw TOML representation directly.
pub(crate) trait IntoRaw {
    fn into_raw(self) -> crate::raw::RawConfig;
}

impl IntoRaw for TzeHudConfig {
    fn into_raw(self) -> crate::raw::RawConfig {
        self.raw
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::config::ConfigLoader;

    fn minimal_valid_toml() -> &'static str {
        r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
"#
    }

    // ── Field classification ──────────────────────────────────────────────────

    #[test]
    fn test_frozen_sections_classified_correctly() {
        assert_eq!(section_classification("runtime"), FieldClassification::Frozen);
        assert_eq!(section_classification("tabs"), FieldClassification::Frozen);
        assert_eq!(section_classification("agents.registered"), FieldClassification::Frozen);
        assert_eq!(section_classification("display_profile"), FieldClassification::Frozen);
        assert_eq!(section_classification("includes"), FieldClassification::Frozen);
    }

    #[test]
    fn test_hot_reloadable_sections_classified_correctly() {
        assert_eq!(section_classification("privacy"), FieldClassification::HotReloadable);
        assert_eq!(section_classification("degradation"), FieldClassification::HotReloadable);
        assert_eq!(section_classification("chrome"), FieldClassification::HotReloadable);
        assert_eq!(section_classification("agents.dynamic_policy"), FieldClassification::HotReloadable);
    }

    // ── reload_config ─────────────────────────────────────────────────────────

    #[test]
    fn test_reload_config_valid_toml_returns_hot_config() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[privacy]
redaction_style = "blank"
"#;
        let result = reload_config(toml);
        assert!(result.is_ok(), "valid config should reload successfully, got: {:?}", result);
        let hot = result.unwrap();
        // Privacy redaction_style should be reflected.
        assert_eq!(hot.privacy.redaction_style, Some("blank".into()));
    }

    #[test]
    fn test_reload_config_invalid_toml_returns_errors() {
        let bad_toml = "this is not valid toml [\n";
        let result = reload_config(bad_toml);
        assert!(result.is_err(), "invalid TOML should return errors");
        let errors = result.unwrap_err();
        assert!(!errors.is_empty());
        assert!(matches!(errors[0].code, ConfigErrorCode::ParseError));
    }

    #[test]
    fn test_reload_config_validation_error_returns_errors_without_applying() {
        // Spec scenario: SIGHUP with validation errors → errors returned, running
        // config unchanged.
        let bad_config = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Tab1"

[privacy]
default_classification = "top_secret"
"#;
        let result = reload_config(bad_config);
        assert!(result.is_err(), "validation error should be returned on reload");
        let errors = result.unwrap_err();
        assert!(
            errors.iter().any(|e| matches!(e.code, ConfigErrorCode::UnknownClassification)),
            "should return CONFIG_UNKNOWN_CLASSIFICATION, got: {:?}", errors
        );
    }

    #[test]
    fn test_reload_config_privacy_redaction_style_change() {
        // Spec scenario: SIGHUP with redaction_style changed from "pattern" to "blank"
        // → new style takes effect without restart.
        let toml_v2 = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[privacy]
redaction_style = "blank"
"#;
        let hot = reload_config(toml_v2).expect("reload should succeed");
        assert_eq!(hot.privacy.redaction_style, Some("blank".into()));
    }

    #[test]
    fn test_reload_config_missing_optional_sections_use_defaults() {
        // When [privacy] absent from new TOML, defaults applied.
        let hot = reload_config(minimal_valid_toml()).expect("reload should succeed");
        // Privacy should default (all None).
        assert!(hot.privacy.default_classification.is_none());
        assert!(hot.privacy.redaction_style.is_none());
        // Dynamic policy absent → None.
        assert!(hot.dynamic_policy.is_none());
    }

    // ── SighupHandler ─────────────────────────────────────────────────────────

    #[test]
    fn test_sighup_handler_config_path() {
        let handler = SighupHandler::new("/etc/tze_hud/config.toml");
        assert_eq!(handler.config_path(), "/etc/tze_hud/config.toml");
    }

    #[test]
    fn test_sighup_handler_trigger_reload_missing_file_returns_error() {
        let handler = SighupHandler::new("/tmp/tze_hud_no_such_file_reload_mop4.toml");
        let result = handler.trigger_reload(|_hot| {
            panic!("callback should not be called on missing file");
        });
        assert!(result.is_err(), "missing file should produce IO error");
        let errors = result.unwrap_err();
        assert!(!errors.is_empty());
        assert!(
            matches!(errors[0].code, ConfigErrorCode::Other(_)),
            "expected IO error code, got: {:?}", errors[0].code
        );
    }

    #[test]
    fn test_into_raw_exposes_raw_config() {
        let toml = minimal_valid_toml();
        let loader = TzeHudConfig::parse(toml).unwrap();
        let raw = loader.into_raw();
        assert!(raw.runtime.is_some(), "runtime should be present");
        assert!(!raw.tabs.is_empty(), "tabs should be present");
    }
}
