//! # tze_hud_scene
//!
//! Pure scene graph data model for tze_hud. No GPU dependency.
//! Satisfies DR-V1: Scene model separable from renderer.
//!
//! The scene graph is a tree: Scene → Tab[] → Tile[] → Node[].
//! All types are constructable, mutable, queryable, serializable,
//! and assertable without any GPU context.

pub mod clock;
pub mod timing;
pub mod types;
pub mod graph;
pub mod mutation;
pub mod diff;
pub mod validation;
pub mod test_scenes;
pub mod calibration;

// ── v1 subsystem trait contracts ─────────────────────────────────────────────
pub mod lease;
pub mod policy;
pub mod events;
pub mod resource;
pub mod config;

pub use clock::{Clock, SimulatedClock, SystemClock, TestClock};
pub use timing::{
    ClockOffset, DurationUs, MonoUs, WallUs,
    // TimingHints and supporting types
    DeliveryPolicy, MessageClass, Schedule, TimingHints,
    // Errors and warnings
    TimingError, TimingWarning,
    // Scheduling helpers
    is_in_scope_for_frame, validate_timing_hints, TimestampValidationInput,
    // Pending queue
    PendingEntry, PendingQueue,
    // Expiration heap
    ExpirationEntry, ExpirationHeap,
    // Staleness
    TileStaleness,
    // Drift detection / ClockSync
    handle_clock_sync, ClockDriftEstimator, ClockSyncRequest, ClockSyncResponse,
    SessionClockSync, VsyncSyncPoint,
    // Timing configuration
    TimingConfig, TimingConfigError,
};
pub use types::*;
pub use graph::{
    SceneGraph, SyncGroupCommitDecision,
    // RFC 0001 §2.1 scene-level capacity constants
    MAX_TABS, MAX_TILES_PER_TAB, MAX_NODES_PER_TILE, MAX_TAB_NAME_BYTES, MAX_MARKDOWN_BYTES,
    // RFC 0001 §2.3 zone band reservation
    ZONE_TILE_Z_MIN,
    // Node data validation
    validate_text_markdown_node_data,
};
pub use mutation::{MutationBatch, MutationResult, SceneMutation, TimingHints, MAX_BATCH_SIZE};
pub use diff::{SceneDiff, DiffEntry};
pub use validation::{
    BatchRejected, BatchValidationError, ValidationError, ValidationErrorCode,
};
pub use test_scenes::{
    ClockMs, SceneSpec, TestSceneRegistry, InvariantViolation, SceneGraphTestExt,
    assert_layer0_invariants,
};
pub use calibration::{test_budget, CalibrationResult};

// ── Lease governance public API ───────────────────────────────────────────────
pub use lease::priority::{
    clamp_requested_priority, shed_count_for_level4, shedding_order,
    TileSortKey, TileSheddingEntry,
    PRIORITY_DEFAULT, PRIORITY_HIGH, PRIORITY_SYSTEM,
};
pub use lease::capability::{
    check_zone_publish, has_publish_zone_capability, should_clear_on_revoke,
    ZonePublishError,
};
pub use lease::degradation::{
    DegradationLevel, DegradationTracker, FrameTimeWindow,
    ENTRY_THRESHOLD_MS, ENTRY_WINDOW_FRAMES,
    RECOVERY_THRESHOLD_MS, RECOVERY_WINDOW_FRAMES,
};
