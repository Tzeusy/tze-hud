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
//! | [`relative`] | Relative Scheduling Primitives (after_us, frames_from_now, next_frame) |
//! | [`pending_queue`] | Presentation Deadline, Session Close Pending Queue Flush |
//! | [`expiration`] | Expiration Policy |
//! | [`staleness`] | Staleness Indicators |
//! | [`drift`] | Clock Drift Detection/Correction/Enforcement, ClockSync RPC |
//! | [`config`] | Timing Configuration |
//! | [`sync_group`] | Sync Group Membership and Lifecycle, Owner Disconnect |
//! | [`sync_commit`] | Sync Group Commit Policies, AllOrDefer Force-Commit |
//! | [`sync_drift`] | Sync Drift Budget |
//!
//! See [`domains`] for the primary clock-domain types: [`WallUs`], [`MonoUs`],
//! [`DurationUs`], and [`ClockOffset`].
//!
//! Sub-modules implement sync group coordination per
//! `timing-model/spec.md` §Sync Group requirements (lines 124–208):
//!
//! - [`sync_group`] — membership, lifecycle, orphan state, ownership checks
//! - [`sync_commit`] — AllOrDefer / AvailableMembers evaluation, force-commit
//! - [`sync_drift`] — drift budget tracking and telemetry

pub mod config;
pub mod domains;
pub mod drift;
pub mod errors;
pub mod expiration;
pub mod hints;
pub mod pending_queue;
pub mod relative;
pub mod scheduling;
pub mod staleness;
pub mod sync_commit;
pub mod sync_drift;
pub mod sync_group;

pub use config::{TimingConfig, TimingConfigError};
pub use domains::{ClockOffset, DurationUs, MonoUs, WallUs};
pub use drift::{
    CLOCK_DRIFT_WINDOW_SIZE, ClockDriftEstimator, ClockSyncRequest, ClockSyncResponse,
    DEFAULT_CLOCK_JUMP_DETECTION_US, SessionClockSync, VsyncSyncPoint, handle_clock_sync,
};
pub use errors::{TimingError, TimingWarning};
pub use expiration::{ExpirationEntry, ExpirationHeap};
pub use hints::{DeliveryPolicy, MessageClass, Schedule, TimingHints};
pub use pending_queue::{PendingEntry, PendingQueue};
pub use relative::{
    IntakeContext, resolve_after_us, resolve_frames_from_now, resolve_next_frame, resolve_schedule,
};
pub use scheduling::{
    CLOCK_SKEW_EXCESSIVE_THRESHOLD_US, CLOCK_SKEW_HIGH_THRESHOLD_US,
    DEFAULT_MAX_FUTURE_SCHEDULE_US, TIMESTAMP_TOO_OLD_THRESHOLD_US, TimestampValidationInput,
    is_in_scope_for_frame, validate_timing_hints,
};
pub use staleness::TileStaleness;
pub use sync_commit::{CommitDecision, apply_decision, evaluate_commit};
pub use sync_drift::{
    DEFAULT_SYNC_DRIFT_BUDGET_US, FrameSyncDriftRecord, SyncDriftHighAlert, SyncGroupArrival,
    TileArrival, compute_spread, evaluate_frame_drift,
};
pub use sync_group::{
    ORPHAN_GRACE_PERIOD_US, OrphanReason, SyncGroupEvent, SyncGroupOrphanState,
    check_sync_group_ownership,
};
