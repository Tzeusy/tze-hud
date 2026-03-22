//! # tze_hud_telemetry
//!
//! Structured telemetry for tze_hud. Per-frame timing, throughput,
//! and resource metrics emitted as machine-readable JSON.
//! Satisfies DR-V3: Structured telemetry.

pub mod record;
pub mod collector;

pub use record::{
    FrameTelemetry, SessionSummary, LatencyBucket,
    BudgetTier, BudgetViolationKind, BudgetViolationEvent, FrameTimeShedEvent,
};
pub use collector::{FrameRecorder, TelemetryCollector};
