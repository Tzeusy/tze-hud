//! Dependency-safe bridge for runtime mutation-intake budget enforcement.

use std::sync::Arc;

use tze_hud_scene::types::{ResourceBudget, SceneId};

/// Stable outcome returned by the runtime-owned mutation budget enforcer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MutationBudgetDecision {
    Allow,
    Reject {
        error_code: &'static str,
        message: String,
    },
    Revoke {
        error_code: &'static str,
        message: String,
    },
}

/// Protocol-facing contract implemented by the runtime's `BudgetEnforcer`.
pub trait MutationBudgetEnforcer: Send + Sync {
    fn register_session(
        &self,
        session_id: SceneId,
        namespace: String,
        budget: ResourceBudget,
    );
    fn remove_session(&self, namespace: &str);
    fn check_mutation(
        &self,
        namespace: &str,
        delta_tiles: i32,
        delta_texture_bytes: i64,
        max_nodes_in_batch: u32,
    ) -> MutationBudgetDecision;
    fn apply_mutation_delta(
        &self,
        namespace: &str,
        delta_tiles: i32,
        delta_texture_bytes: i64,
    );
}

pub type SharedMutationBudgetEnforcer = Arc<dyn MutationBudgetEnforcer>;
