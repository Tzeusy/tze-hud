//! # tze_hud_telemetry
//!
//! Structured telemetry for tze_hud. Per-frame timing, throughput,
//! and resource metrics emitted as machine-readable JSON.
//! Satisfies DR-V3: Structured telemetry.
//! Satisfies DR-V8: Soak and leak test resource monitoring.

pub mod record;
pub mod collector;
pub mod resource_monitor;
pub mod validation;

pub use record::{
    FrameTelemetry, SessionSummary, LatencyBucket,
    BudgetTier, BudgetViolationKind, BudgetViolationEvent, FrameTimeShedEvent,
    DegradationEvent, DegradationDirection,
};
pub use collector::{FrameRecorder, TelemetryCollector};
pub use resource_monitor::{
    AgentFootprint, GrowthRatios, ResourceMonitor, ResourceSnapshot, SPEC_GROWTH_TOLERANCE,
};
pub use validation::{
    AssertionOutcome, BudgetAssertion, CalibrationDimension, HardwareFactors, ValidationReport,
};
