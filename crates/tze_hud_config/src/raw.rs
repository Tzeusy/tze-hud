//! Raw TOML-deserialisable structs.
//!
//! These are the intermediate representations produced by `toml::from_str`.
//! They mirror the configuration file structure exactly and are deliberately
//! permissive — all fields except the structurally-required ones are `Option`
//! so that we can collect all missing/invalid-value errors in the validation
//! phase rather than failing at deserialisation.
//!
//! All of these types derive `schemars::JsonSchema` so that the `--print-schema`
//! feature can generate a full JSON Schema from them.

use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use std::collections::HashMap;

// ─── Helper: AnyValue for `includes` field ────────────────────────────────────

/// Wrapper that accepts any TOML value during deserialization.
///
/// Used exclusively for the `includes` field to detect its presence and
/// report a hard error (v1-reserved; post-v1 only).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AnyValue(pub toml::Value);

impl JsonSchema for AnyValue {
    fn schema_name() -> String {
        "AnyValue".to_string()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        // Represents any JSON value.
        schemars::schema::Schema::Bool(true)
    }
}

// ─── [runtime] ───────────────────────────────────────────────────────────────

/// `[runtime]` table — required.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawRuntime {
    /// Display profile name.  Must be present.
    pub profile: Option<String>,

    /// Write the JSON Schema at startup and continue running.
    #[serde(default)]
    pub emit_schema: bool,

    /// Virtual display width for headless mode (default 1920).
    pub headless_width: Option<u32>,

    /// Virtual display height for headless mode (default 1080).
    pub headless_height: Option<u32>,
}

// ─── [display_profile] ───────────────────────────────────────────────────────

/// `[display_profile]` table — optional; used for custom profiles that extend
/// a built-in.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawDisplayProfile {
    /// Built-in profile to extend (`full-display`, `mobile`; NOT `headless`).
    pub extends: Option<String>,

    /// Override: max tiles.
    pub max_tiles: Option<u32>,
    /// Override: max texture memory in MiB.
    pub max_texture_mb: Option<u32>,
    /// Override: max simultaneous agents.
    pub max_agents: Option<u32>,
    /// Override: max media streams.
    pub max_media_streams: Option<u32>,
    /// Override: max agent update Hz.
    pub max_agent_update_hz: Option<u32>,
    /// Override: target FPS.
    pub target_fps: Option<u32>,
    /// Override: minimum FPS.
    pub min_fps: Option<u32>,

    /// Override: allow background zones.
    pub allow_background_zones: Option<bool>,
    /// Override: allow chrome zones.
    pub allow_chrome_zones: Option<bool>,
}

// ─── [[tabs]] ────────────────────────────────────────────────────────────────

/// A single entry in the `[[tabs]]` array.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawTab {
    /// Human-readable tab name.  Must be unique.
    pub name: Option<String>,

    /// Whether this is the default tab.
    #[serde(default)]
    pub default_tab: bool,

    /// Default tile layout for this tab.
    pub default_layout: Option<String>,

    /// Scene event name that triggers an automatic switch to this tab.
    /// Empty string = no auto-switch.
    pub tab_switch_on_event: Option<String>,

    /// Layout fractions.
    pub layout: Option<RawTabLayout>,

    /// Zone types active on this tab.
    ///
    /// Each entry must be either a built-in zone type (see
    /// `zones::BUILTIN_ZONE_TYPES`) or a custom type defined in the
    /// `[zones]` section.  An unknown name produces `CONFIG_UNKNOWN_ZONE_TYPE`.
    #[serde(default)]
    pub zones: Vec<String>,
}

/// Layout fractions within a tab.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawTabLayout {
    pub reserved_top_fraction: Option<f64>,
    pub reserved_bottom_fraction: Option<f64>,
    pub reserved_left_fraction: Option<f64>,
    pub reserved_right_fraction: Option<f64>,
}

// ─── [degradation] ───────────────────────────────────────────────────────────

/// `[degradation]` table — optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawDegradation {
    // Frame-time thresholds (ms) — must be monotonically non-decreasing.
    pub coalesce_frame_ms: Option<f64>,
    pub simplify_rendering_frame_ms: Option<f64>,
    pub shed_tiles_frame_ms: Option<f64>,
    pub audio_only_frame_ms: Option<f64>,

    // GPU fraction thresholds — must be monotonically non-decreasing.
    pub reduce_media_quality_gpu_fraction: Option<f64>,
    pub reduce_concurrent_streams_gpu_fraction: Option<f64>,
}

// ─── [privacy] ───────────────────────────────────────────────────────────────

/// `[privacy]` table — optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawPrivacy {
    pub default_classification: Option<String>,
    pub default_viewer_class: Option<String>,
    pub viewer_id_method: Option<String>,
    pub redaction_style: Option<String>,
    pub multi_viewer_policy: Option<String>,

    pub quiet_hours: Option<RawQuietHours>,
}

/// `[privacy.quiet_hours]` — optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawQuietHours {
    #[serde(default)]
    pub enabled: bool,
    pub pass_through_class: Option<String>,
    pub quiet_mode_display: Option<String>,
    pub schedule: Option<Vec<RawQuietHoursSchedule>>,
}

/// A single time-range entry in `[[privacy.quiet_hours.schedule]]`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawQuietHoursSchedule {
    pub start: Option<String>,
    pub end: Option<String>,
    pub days: Option<Vec<String>>,
}

// ─── [chrome] ────────────────────────────────────────────────────────────────

/// `[chrome]` table — optional.
///
/// NOTE: `redaction_style` MUST NOT appear here (spec §Requirement: Redaction
/// Style Ownership).  We use a deny-unknown-fields variant in validation rather
/// than here to give a better error message.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawChrome {
    // Intentionally minimal — chrome options are spec-reserved for now.
    // Future fields added here when the chrome spec matures.
}

// ─── [zones] ─────────────────────────────────────────────────────────────────

/// `[zones]` table — optional.  Custom zone type definitions.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawZones(pub HashMap<String, RawZoneType>);

/// A single custom zone type definition.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawZoneType {
    pub policy: Option<String>,
    pub layer: Option<String>,
}

// ─── [agents] ────────────────────────────────────────────────────────────────

/// `[agents]` table — optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawAgents {
    pub registered: Option<HashMap<String, RawRegisteredAgent>>,
    pub dynamic_policy: Option<RawDynamicPolicy>,
}

/// A single pre-registered agent entry under `[agents.registered.<name>]`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawRegisteredAgent {
    pub capabilities: Option<Vec<String>>,
    pub auth_psk_env: Option<String>,
    pub max_tiles: Option<u32>,
    pub max_texture_mb: Option<u32>,
    pub max_update_hz: Option<u32>,
}

/// `[agents.dynamic_policy]` — optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawDynamicPolicy {
    #[serde(default)]
    pub allow_dynamic_agents: bool,
    pub default_capabilities: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub prompt_for_elevated_capabilities: bool,
    pub dynamic_presence_ceiling: Option<String>,
}

fn default_true() -> bool {
    true
}

// ─── Top-level document ──────────────────────────────────────────────────────

/// The top-level TOML document.
///
/// All sections are optional to allow maximum error collection; the validator
/// enforces required fields.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RawConfig {
    /// `includes` is v1-reserved.  Presence must produce a hard error.
    /// Accepts any value — detection of presence triggers the error in validation.
    pub includes: Option<AnyValue>,

    pub runtime: Option<RawRuntime>,
    pub display_profile: Option<RawDisplayProfile>,

    #[serde(default)]
    pub tabs: Vec<RawTab>,

    pub degradation: Option<RawDegradation>,
    pub privacy: Option<RawPrivacy>,
    pub chrome: Option<RawChrome>,
    pub zones: Option<RawZones>,
    pub agents: Option<RawAgents>,
}
