//! # tze_hud_runtime
//!
//! Runtime kernel for tze_hud. Orchestrates the frame pipeline:
//! input drain → local feedback → mutation intake → scene commit →
//! render encode → GPU submit → telemetry emit.

pub mod headless;

pub use headless::HeadlessRuntime;
