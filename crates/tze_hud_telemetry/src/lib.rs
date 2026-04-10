//! # tze_hud_telemetry
//!
//! Structured telemetry for tze_hud. Per-frame timing, throughput,
//! and resource metrics emitted as machine-readable JSON.
//! Satisfies DR-V3: Structured telemetry.
//! Satisfies DR-V8: Soak and leak test resource monitoring.

pub mod collector;
pub mod record;
pub mod resource_monitor;
pub mod validation;

pub use collector::{FrameRecorder, TelemetryCollector};
pub use record::{
    BudgetTier, BudgetViolationEvent, BudgetViolationKind, CalibrationStatus, DegradationDirection,
    DegradationEvent, FrameTelemetry, FrameTimeShedEvent, LatencyBucket, SessionSummary,
};
pub use resource_monitor::{
    AgentFootprint, GrowthRatios, ResourceMonitor, ResourceSnapshot, SPEC_GROWTH_TOLERANCE,
};
pub use validation::{
    AssertionOutcome, BudgetAssertion, CalibrationDimension, HardwareFactors, ValidationReport,
};
