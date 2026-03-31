//! # tze_hud_config
//!
//! TOML configuration loading and validation for tze_hud.
//!
//! This crate provides `TzeHudConfig`, the concrete implementation of the
//! `ConfigLoader` trait defined in `tze_hud_scene::config`.
//!
//! ## Scope
//!
//! ### rig-j90m (TOML schema and file loading)
//! - TOML Configuration Format (v1-mandatory)
//! - Configuration File Resolution Order (v1-mandatory)
//! - Minimal Valid Configuration (v1-mandatory)
//! - Structured Validation Error Collection (v1-mandatory)
//! - Tab Configuration Validation (v1-mandatory)
//! - Reserved Fraction Validation (v1-mandatory)
//! - FPS Range Validation (v1-mandatory)
//! - Degradation Threshold Ordering (v1-mandatory)
//! - Scene Event Naming Convention (v1-mandatory)
//! - Schema Export (v1-mandatory)
//! - Layered Config Composition guard (v1-reserved: hard error)
//!
//! ### rig-umgy (Display profile resolution)
//! - Display Profile full-display (v1-mandatory)
//! - Display Profile headless (v1-mandatory)
//! - Mobile Profile Schema-Reserved (v1-mandatory)
//! - Profile Auto-Detection (v1-mandatory)
//! - Profile Budget Escalation Prevention (v1-mandatory)
//! - Profile Extends Conflict Detection (v1-mandatory)
//! - Headless Virtual Display (v1-mandatory)
//!
//! ### rig-mop4 (Privacy, zone registry, agent registration, hot-reload)
//! - Privacy Configuration Defaults (v1-mandatory)
//! - Quiet Hours Configuration (v1-mandatory)
//! - Redaction Style Ownership (v1-mandatory)
//! - Zone Registry Configuration (v1-mandatory)
//! - Agent Registration with Per-Agent Budget Overrides (v1-mandatory)
//! - Dynamic Agent Policy (v1-mandatory)
//! - Authentication Secret Indirection (v1-mandatory)
//! - Configuration Reload (v1-mandatory)
//!
//! ## Does NOT include
//! - Capability vocabulary validation (rig-9yfh)
//!
//! ### hud-sc0a.4 (Component type contracts)
//! - Component Type Contract (v1-mandatory)
//! - V1 Component Type Definitions (v1-mandatory)
//! - Zone Name Reconciliation (v1-mandatory)
//!
//! ### hud-sc0a.5 (Component profile loader and zone override parser)
//! - Component Profile Format (v1-mandatory)
//! - Zone Rendering Override Schema (v1-mandatory)
//! - Profile-Scoped Token Resolution (v1-mandatory)
//! - Profile Widget Scope (v1-mandatory)
//! - Zone Name Reconciliation (v1-mandatory)

pub mod agents;
pub mod capability;
pub mod component_profiles;
pub mod component_types;
pub mod loader;
pub mod privacy;
pub mod profile;
pub mod raw;
pub mod readability;
pub mod reload;
pub mod resolver;
pub mod schema;
#[cfg(test)]
mod tests;
pub mod tokens;
pub mod widgets;
pub mod zones;

pub use agents::{
    AuthEnvWarning, check_agent_auth_env_vars, check_agent_auth_env_vars_with_lookup,
    dynamic_agents_allowed, validate_agents,
};
pub use component_profiles::{ComponentProfile, ZoneRenderingOverride, scan_profile_dirs};
pub use component_types::{ComponentType, ComponentTypeContract, ReadabilityTechnique};
pub use loader::TzeHudConfig;
pub use privacy::{QuietHoursAction, quiet_hours_action, validate_privacy};
pub use profile::{
    AutoDetectResult, HeadlessSignal, auto_detect_profile, resolve_headless_dimensions,
    resolve_profile, validate_display_profile,
};
pub use readability::{PolicySnapshot, ReadabilityViolation, check_zone_readability, is_dev_mode};
pub use reload::{
    FieldClassification, HotReloadableConfig, SighupHandler, reload_config, section_classification,
};
pub use resolver::resolve_config_path;
pub use schema::print_schema;
pub use widgets::{
    LoadedWidgetType, build_widget_instance, validate_widget_bundles, validate_widget_instances,
};
pub use zones::{BUILTIN_ZONE_TYPES, is_known_zone_type, validate_zone_type_ref, validate_zones};
