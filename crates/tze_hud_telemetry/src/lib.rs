//! # tze_hud_telemetry
//!
//! Structured telemetry for tze_hud. Per-frame timing, throughput,
//! and resource metrics emitted as machine-readable JSON.
//! Satisfies DR-V3: Structured telemetry.
//! Satisfies DR-V8: Soak and leak test resource monitoring.

pub mod collector;
pub mod publish_load;
pub mod record;
pub mod resource_monitor;
pub mod validation;

pub use collector::{FrameRecorder, TelemetryCollector};
pub use publish_load::{
    ByteAccountingMode, PublishLoadArtifact, PublishLoadCalibrationStatus, PublishLoadIdentity,
    PublishLoadMetrics, PublishLoadMode, PublishLoadThresholds, PublishLoadTraceability,
    PublishLoadTransport, PublishLoadVerdict,
};
pub use record::{
    BudgetTier, BudgetViolationEvent, BudgetViolationKind, CalibrationStatus, DegradationDirection,
    DegradationEvent, FrameTelemetry, FrameTimeShedEvent, LatencyBucket, SessionSummary,
};
pub use resource_monitor::{
    AgentFootprint, GrowthRatios, ResourceMonitor, ResourceSnapshot, SPEC_GROWTH_TOLERANCE,
};
pub use validation::{
    AssertionOutcome, BudgetAssertion, CalibrationDimension, HardwareFactors,
    MutationPathLatencyConformance, POLICY_MUTATION_EVAL_BUDGET_US, ValidationReport,
    evaluate_policy_mutation_latency_conformance,
};
