//! Thread-safe protocol bridge to the runtime-owned mutation budget enforcer.

use std::sync::Mutex;
use std::time::Instant;

use tze_hud_protocol::session_server::{
    MutationBudgetDecision, MutationBudgetEnforcer as MutationBudgetEnforcerContract,
};
use tze_hud_scene::types::{ResourceBudget, SceneId};

use crate::{BudgetCheckOutcome, BudgetEnforcer, NoopTelemetrySink};

/// Shared enforcement object used by production gRPC session handlers.
pub struct RuntimeMutationBudgetEnforcer {
    inner: Mutex<BudgetEnforcer>,
}

impl RuntimeMutationBudgetEnforcer {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BudgetEnforcer::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BudgetEnforcer> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for RuntimeMutationBudgetEnforcer {
    fn default() -> Self {
        Self::new()
    }
}

impl MutationBudgetEnforcerContract for RuntimeMutationBudgetEnforcer {
    fn register_session(
        &self,
        session_id: SceneId,
        namespace: String,
        budget: ResourceBudget,
    ) {
        self.lock().register_session(session_id, namespace, budget);
    }

    fn remove_session(&self, namespace: &str) {
        self.lock().remove_session(namespace);
    }

    fn check_mutation(
        &self,
        namespace: &str,
        delta_tiles: i32,
        delta_texture_bytes: i64,
        max_nodes_in_batch: u32,
    ) -> MutationBudgetDecision {
        let mut sink = NoopTelemetrySink;
        match self.lock().check_mutation(
            namespace,
            delta_tiles,
            delta_texture_bytes,
            max_nodes_in_batch,
            Instant::now(),
            &mut sink,
        ) {
            BudgetCheckOutcome::Allow => MutationBudgetDecision::Allow,
            BudgetCheckOutcome::Reject(violation) => MutationBudgetDecision::Reject {
                error_code: "RESOURCE_BUDGET_EXCEEDED",
                message: format!("{violation:?}"),
            },
            BudgetCheckOutcome::Revoke(violation) => MutationBudgetDecision::Revoke {
                error_code: "RESOURCE_BUDGET_CRITICAL",
                message: format!("{violation:?}"),
            },
        }
    }

    fn apply_mutation_delta(
        &self,
        namespace: &str,
        delta_tiles: i32,
        delta_texture_bytes: i64,
    ) {
        self.lock()
            .apply_mutation_delta(namespace, delta_tiles, delta_texture_bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_budget_rejects_mutation_above_tile_limit() {
        let enforcer = RuntimeMutationBudgetEnforcer::new();
        enforcer.register_session(
            SceneId::new(),
            "agent-a".to_string(),
            ResourceBudget {
                max_tiles: 1,
                ..ResourceBudget::default()
            },
        );
        enforcer.apply_mutation_delta("agent-a", 1, 0);

        assert!(matches!(
            enforcer.check_mutation("agent-a", 1, 0, 1),
            MutationBudgetDecision::Reject {
                error_code: "RESOURCE_BUDGET_EXCEEDED",
                ..
            }
        ));
    }
}
