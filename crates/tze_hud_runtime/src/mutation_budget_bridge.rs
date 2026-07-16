//! Thread-safe protocol bridge to the runtime-owned mutation budget enforcer.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use tze_hud_protocol::session_server::{
    MutationBudgetDecision, MutationBudgetEnforcer as MutationBudgetEnforcerContract,
};
use tze_hud_scene::types::{ResourceBudget, SceneId};

use crate::{BudgetCheckOutcome, BudgetEnforcer, NoopTelemetrySink};

/// Shared enforcement object used by production gRPC session handlers.
pub struct RuntimeMutationBudgetEnforcer {
    inner: Mutex<AggregateBudgetState>,
}

struct AggregateBudgetState {
    enforcer: BudgetEnforcer,
    sessions: HashMap<SceneId, (String, bool)>,
    resident_sessions: u32,
    leased_tiles: u32,
    leased_texture_bytes: u64,
    max_resident_sessions: u32,
    max_leased_tiles: u32,
    max_leased_texture_bytes: u64,
}

impl RuntimeMutationBudgetEnforcer {
    pub fn new() -> Self {
        Self::with_limits(u32::MAX, u32::MAX, u64::MAX)
    }

    pub fn with_limits(
        max_resident_sessions: u32,
        max_leased_tiles: u32,
        max_leased_texture_bytes: u64,
    ) -> Self {
        Self {
            inner: Mutex::new(AggregateBudgetState {
                enforcer: BudgetEnforcer::new(),
                sessions: HashMap::new(),
                resident_sessions: 0,
                leased_tiles: 0,
                leased_texture_bytes: 0,
                max_resident_sessions,
                max_leased_tiles,
                max_leased_texture_bytes,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AggregateBudgetState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        resident: bool,
    ) -> MutationBudgetDecision {
        let mut state = self.lock();
        if resident && state.resident_sessions >= state.max_resident_sessions {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_EXHAUSTED",
                message: format!(
                    "resident_sessions current={} limit={}",
                    state.resident_sessions, state.max_resident_sessions
                ),
            };
        }
        state
            .enforcer
            .register_session(session_id, namespace.clone(), budget);
        state.sessions.insert(session_id, (namespace, resident));
        if resident {
            state.resident_sessions = state.resident_sessions.saturating_add(1);
        }
        MutationBudgetDecision::Allow
    }

    fn remove_session(&self, namespace: &str) {
        let mut state = self.lock();
        let removed: Vec<SceneId> = state
            .sessions
            .iter()
            .filter_map(|(id, (name, _))| (name == namespace).then_some(*id))
            .collect();
        for id in removed {
            if let Some((_, resident)) = state.sessions.remove(&id)
                && resident
            {
                state.resident_sessions = state.resident_sessions.saturating_sub(1);
            }
        }
        state.enforcer.remove_session(namespace);
    }

    fn reserve_mutation(
        &self,
        namespace: &str,
        delta_tiles: i32,
        delta_texture_bytes: i64,
        max_nodes_in_batch: u32,
    ) -> MutationBudgetDecision {
        let mut state = self.lock();
        let proposed_tiles = if delta_tiles >= 0 {
            state.leased_tiles.saturating_add(delta_tiles as u32)
        } else {
            state.leased_tiles.saturating_sub((-delta_tiles) as u32)
        };
        if proposed_tiles > state.max_leased_tiles {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_EXHAUSTED",
                message: format!(
                    "leased_tiles current={} requested_delta={} limit={}",
                    state.leased_tiles, delta_tiles, state.max_leased_tiles
                ),
            };
        }
        let proposed_texture = if delta_texture_bytes >= 0 {
            state
                .leased_texture_bytes
                .saturating_add(delta_texture_bytes as u64)
        } else {
            state
                .leased_texture_bytes
                .saturating_sub((-delta_texture_bytes) as u64)
        };
        if proposed_texture > state.max_leased_texture_bytes {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_EXHAUSTED",
                message: format!(
                    "agent_leased_texture_bytes current={} requested_delta={} limit={}",
                    state.leased_texture_bytes, delta_texture_bytes, state.max_leased_texture_bytes
                ),
            };
        }

        let mut sink = NoopTelemetrySink;
        match state.enforcer.check_mutation(
            namespace,
            delta_tiles,
            delta_texture_bytes,
            max_nodes_in_batch,
            Instant::now(),
            &mut sink,
        ) {
            BudgetCheckOutcome::Allow => {
                state
                    .enforcer
                    .apply_mutation_delta(namespace, delta_tiles, delta_texture_bytes);
                state.leased_tiles = proposed_tiles;
                state.leased_texture_bytes = proposed_texture;
                MutationBudgetDecision::Allow
            }
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

    fn rollback_mutation(&self, namespace: &str, delta_tiles: i32, delta_texture_bytes: i64) {
        let mut state = self.lock();
        state.enforcer.apply_mutation_delta(
            namespace,
            delta_tiles.saturating_neg(),
            delta_texture_bytes.saturating_neg(),
        );
        if delta_tiles >= 0 {
            state.leased_tiles = state.leased_tiles.saturating_sub(delta_tiles as u32);
        } else {
            state.leased_tiles = state.leased_tiles.saturating_add((-delta_tiles) as u32);
        }
        if delta_texture_bytes >= 0 {
            state.leased_texture_bytes = state
                .leased_texture_bytes
                .saturating_sub(delta_texture_bytes as u64);
        } else {
            state.leased_texture_bytes = state
                .leased_texture_bytes
                .saturating_add((-delta_texture_bytes) as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_budget_rejects_mutation_above_tile_limit() {
        let enforcer = RuntimeMutationBudgetEnforcer::new();
        assert_eq!(
            enforcer.register_session(
                SceneId::new(),
                "agent-a".to_string(),
                ResourceBudget {
                    max_tiles: 1,
                    ..ResourceBudget::default()
                },
                true,
            ),
            MutationBudgetDecision::Allow
        );
        assert_eq!(
            enforcer.reserve_mutation("agent-a", 1, 0, 1),
            MutationBudgetDecision::Allow
        );

        assert!(matches!(
            enforcer.reserve_mutation("agent-a", 1, 0, 1),
            MutationBudgetDecision::Reject {
                error_code: "RESOURCE_BUDGET_EXCEEDED",
                ..
            }
        ));
    }

    #[test]
    fn aggregate_limits_are_atomic_across_agents() {
        let enforcer = RuntimeMutationBudgetEnforcer::with_limits(2, 2, 100);
        for name in ["agent-a", "agent-b"] {
            assert_eq!(
                enforcer.register_session(
                    SceneId::new(),
                    name.to_string(),
                    ResourceBudget {
                        max_tiles: 8,
                        max_texture_bytes: 100,
                        ..ResourceBudget::default()
                    },
                    true,
                ),
                MutationBudgetDecision::Allow
            );
        }
        assert!(matches!(
            enforcer.register_session(
                SceneId::new(),
                "agent-c".to_string(),
                ResourceBudget::default(),
                true,
            ),
            MutationBudgetDecision::Reject { message, .. } if message.contains("resident_sessions")
        ));

        assert_eq!(
            enforcer.reserve_mutation("agent-a", 1, 60, 1),
            MutationBudgetDecision::Allow
        );
        assert!(matches!(
            enforcer.reserve_mutation("agent-b", 2, 0, 1),
            MutationBudgetDecision::Reject { message, .. } if message.contains("leased_tiles")
        ));
        assert!(matches!(
            enforcer.reserve_mutation("agent-b", 1, 50, 1),
            MutationBudgetDecision::Reject { message, .. } if message.contains("agent_leased_texture_bytes")
        ));
        assert_eq!(
            enforcer.reserve_mutation("agent-b", 1, 40, 1),
            MutationBudgetDecision::Allow
        );
    }
}
