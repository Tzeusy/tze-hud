//! # tze_hud_config
//!
//! TOML configuration loading and validation for tze_hud.
//!
//! This crate provides `TzeHudConfig`, the concrete implementation of the
//! `ConfigLoader` trait defined in `tze_hud_scene::config`.
//!
//! ## Scope (per spec §rig-j90m)
//!
//! Implements:
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
//! Does NOT include:
//! - Display profile resolution (rig-umgy)
//! - Capability vocabulary validation (rig-9yfh)
//! - Privacy, zone registry, agent registration (rig-mop4)
//! - Hot-reload (separate bead)

pub mod loader;
pub mod raw;
pub mod resolver;
pub mod schema;
#[cfg(test)]
mod tests;

pub use loader::TzeHudConfig;
pub use resolver::resolve_config_path;
pub use schema::print_schema;
