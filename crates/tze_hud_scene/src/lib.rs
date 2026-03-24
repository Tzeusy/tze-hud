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
pub use timing::{ClockOffset, DurationUs, MonoUs, WallUs};
pub use types::*;
pub use graph::{SceneGraph, SyncGroupCommitDecision};
pub use mutation::{MutationBatch, MutationResult, SceneMutation};
pub use diff::{SceneDiff, DiffEntry};
pub use validation::ValidationError;
pub use test_scenes::{
    ClockMs, SceneSpec, TestSceneRegistry, InvariantViolation, SceneGraphTestExt,
    assert_layer0_invariants,
};
pub use calibration::{test_budget, CalibrationResult};
