//! Clock-domain timing types and TimingHints for tze_hud.
//!
//! ## Modules
//!
//! | Module | Spec requirement |
//! |---|---|
//! | [`domains`] | Clock Domain Separation, Clock Domain Naming Convention |
//! | [`hints`] | Timing Fields on Payloads, Message Class Typed Enum |
//! | [`errors`] | Timestamp Validation (error codes) |
//! | [`scheduling`] | Timestamp Validation (logic), Frame Quantization |
//! | [`pending_queue`] | Presentation Deadline, Session Close Pending Queue Flush |
//! | [`expiration`] | Expiration Policy |
//! | [`staleness`] | Staleness Indicators |
//! | [`drift`] | Clock Drift Detection/Correction/Enforcement, ClockSync RPC |
//! | [`config`] | Timing Configuration |
//!
//! See [`domains`] for the primary clock-domain types: [`WallUs`], [`MonoUs`],
//! [`DurationUs`], and [`ClockOffset`].

pub mod domains;
pub mod hints;
pub mod errors;
pub mod scheduling;
pub mod pending_queue;
pub mod expiration;
pub mod staleness;
pub mod drift;
pub mod config;

pub use domains::{ClockOffset, DurationUs, MonoUs, WallUs};
pub use hints::{DeliveryPolicy, MessageClass, Schedule, TimingHints};
pub use errors::{TimingError, TimingWarning};
pub use scheduling::{
    is_in_scope_for_frame, validate_timing_hints, TimestampValidationInput,
    CLOCK_SKEW_EXCESSIVE_THRESHOLD_US, CLOCK_SKEW_HIGH_THRESHOLD_US,
    DEFAULT_MAX_FUTURE_SCHEDULE_US, TIMESTAMP_TOO_OLD_THRESHOLD_US,
};
pub use pending_queue::{PendingEntry, PendingQueue};
pub use expiration::{ExpirationEntry, ExpirationHeap};
pub use staleness::TileStaleness;
pub use drift::{
    handle_clock_sync, ClockDriftEstimator, ClockSyncRequest, ClockSyncResponse,
    SessionClockSync, VsyncSyncPoint, CLOCK_DRIFT_WINDOW_SIZE, DEFAULT_CLOCK_JUMP_DETECTION_US,
};
pub use config::{TimingConfig, TimingConfigError};
