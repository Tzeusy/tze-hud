//! # tze_hud_scene
//!
//! Pure scene graph data model for tze_hud. No GPU dependency.
//! Satisfies DR-V1: Scene model separable from renderer.
//!
//! The scene graph is a tree: Scene → Tab[] → Tile[] → Node[].
//! All types are constructable, mutable, queryable, serializable,
//! and assertable without any GPU context.

pub mod types;
pub mod graph;
pub mod mutation;
pub mod diff;
pub mod validation;

pub use types::*;
pub use graph::SceneGraph;
pub use mutation::{MutationBatch, SceneMutation};
pub use diff::{SceneDiff, DiffEntry};
pub use validation::ValidationError;
