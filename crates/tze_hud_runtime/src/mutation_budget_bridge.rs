//! Thread-safe protocol bridge to the runtime-owned mutation budget enforcer.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use tze_hud_protocol::session_server::{
    MutationBudgetDecision, MutationBudgetEnforcer as MutationBudgetEnforcerContract,
    MutationBudgetUsage,
};
use tze_hud_scene::types::{ResourceBudget, SceneId};

use crate::{BudgetCheckOutcome, BudgetEnforcer, NoopTelemetrySink};

/// Shared enforcement object used by production gRPC session handlers.
pub struct RuntimeMutationBudgetEnforcer {
    inner: Mutex<AggregateBudgetState>,
}

struct AggregateBudgetState {
    enforcer: BudgetEnforcer,
    sessions: HashMap<SceneId, SessionBudgetState>,
    resident_sessions: u32,
    guest_sessions: u32,
    leased_tiles: u32,
    leased_texture_bytes: u64,
    max_resident_sessions: u32,
    max_guest_sessions: u32,
    max_leased_tiles: u32,
    max_leased_texture_bytes: u64,
}

struct SessionBudgetState {
    budget_key: String,
    resident: bool,
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
        Self::with_session_limits(
            max_resident_sessions,
            u32::try_from(crate::admission::DEFAULT_MAX_GUEST_SESSIONS).unwrap_or(u32::MAX),
            max_leased_tiles,
            max_leased_texture_bytes,
        )
    }

    pub fn with_session_limits(
        max_resident_sessions: u32,
        max_guest_sessions: u32,
        max_leased_tiles: u32,
        max_leased_texture_bytes: u64,
    ) -> Self {
        Self {
            inner: Mutex::new(AggregateBudgetState {
                enforcer: BudgetEnforcer::new(),
                sessions: HashMap::new(),
                resident_sessions: 0,
                guest_sessions: 0,
                leased_tiles: 0,
                leased_texture_bytes: 0,
                max_resident_sessions,
                max_guest_sessions,
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
        initial_usage: MutationBudgetUsage,
    ) -> MutationBudgetDecision {
        let mut state = self.lock();
        if state.sessions.contains_key(&session_id) {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_EXHAUSTED",
                message: format!("session_id {session_id} is already registered"),
            };
        }
        if resident && state.resident_sessions >= state.max_resident_sessions {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_EXHAUSTED",
                message: format!(
                    "resident_sessions current={} limit={}",
                    state.resident_sessions, state.max_resident_sessions
                ),
            };
        }
        if !resident && state.guest_sessions >= state.max_guest_sessions {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_EXHAUSTED",
                message: format!(
                    "guest_sessions current={} limit={}",
                    state.guest_sessions, state.max_guest_sessions
                ),
            };
        }
        let proposed_tiles = state.leased_tiles.saturating_add(initial_usage.tiles);
        if proposed_tiles > state.max_leased_tiles {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_EXHAUSTED",
                message: format!(
                    "leased_tiles current={} restored={} limit={}",
                    state.leased_tiles, initial_usage.tiles, state.max_leased_tiles
                ),
            };
        }
        let proposed_texture = state
            .leased_texture_bytes
            .saturating_add(initial_usage.texture_bytes);
        if proposed_texture > state.max_leased_texture_bytes {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_EXHAUSTED",
                message: format!(
                    "agent_leased_texture_bytes current={} restored={} limit={}",
                    state.leased_texture_bytes,
                    initial_usage.texture_bytes,
                    state.max_leased_texture_bytes
                ),
            };
        }
        let budget_key = format!("{namespace}@{session_id}");
        state
            .enforcer
            .register_session(session_id, budget_key.clone(), budget);
        let mut sink = NoopTelemetrySink;
        let restored_texture = i64::try_from(initial_usage.texture_bytes).unwrap_or(i64::MAX);
        match state.enforcer.check_mutation(
            &budget_key,
            i32::try_from(initial_usage.tiles).unwrap_or(i32::MAX),
            restored_texture,
            0,
            Instant::now(),
            &mut sink,
        ) {
            BudgetCheckOutcome::Allow => state.enforcer.apply_mutation_delta(
                &budget_key,
                i32::try_from(initial_usage.tiles).unwrap_or(i32::MAX),
                restored_texture,
            ),
            BudgetCheckOutcome::Reject(violation) => {
                state.enforcer.remove_session(&budget_key);
                return MutationBudgetDecision::Reject {
                    error_code: "RESOURCE_BUDGET_EXCEEDED",
                    message: format!("restored session usage rejected: {violation:?}"),
                };
            }
            BudgetCheckOutcome::Revoke(violation) => {
                state.enforcer.remove_session(&budget_key);
                return MutationBudgetDecision::Revoke {
                    error_code: "RESOURCE_BUDGET_CRITICAL",
                    message: format!("restored session usage revoked: {violation:?}"),
                };
            }
        }
        state.sessions.insert(
            session_id,
            SessionBudgetState {
                budget_key,
                resident,
            },
        );
        if resident {
            state.resident_sessions = state.resident_sessions.saturating_add(1);
        } else {
            state.guest_sessions = state.guest_sessions.saturating_add(1);
        }
        state.leased_tiles = proposed_tiles;
        state.leased_texture_bytes = proposed_texture;
        MutationBudgetDecision::Allow
    }

    fn remove_session(&self, session_id: SceneId) {
        let mut state = self.lock();
        let Some(session) = state.sessions.remove(&session_id) else {
            return;
        };
        if let Some((tiles, texture_bytes)) = state
            .enforcer
            .agent_state(&session.budget_key)
            .map(|usage| (usage.tile_count, usage.texture_bytes_used))
        {
            state.leased_tiles = state.leased_tiles.saturating_sub(tiles);
            state.leased_texture_bytes = state.leased_texture_bytes.saturating_sub(texture_bytes);
        }
        if session.resident {
            state.resident_sessions = state.resident_sessions.saturating_sub(1);
        } else {
            state.guest_sessions = state.guest_sessions.saturating_sub(1);
        }
        state.enforcer.remove_session(&session.budget_key);
    }

    fn reserve_mutation(
        &self,
        session_id: SceneId,
        delta_tiles: i32,
        delta_texture_bytes: i64,
        max_nodes_in_batch: u32,
    ) -> MutationBudgetDecision {
        let mut state = self.lock();
        let Some(budget_key) = state
            .sessions
            .get(&session_id)
            .map(|session| session.budget_key.clone())
        else {
            return MutationBudgetDecision::Reject {
                error_code: "RESOURCE_BUDGET_SESSION_UNKNOWN",
                message: format!("session_id {session_id} is not registered"),
            };
        };
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
            &budget_key,
            delta_tiles,
            delta_texture_bytes,
            max_nodes_in_batch,
            Instant::now(),
            &mut sink,
        ) {
            BudgetCheckOutcome::Allow => {
                state
                    .enforcer
                    .apply_mutation_delta(&budget_key, delta_tiles, delta_texture_bytes);
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

    fn rollback_mutation(&self, session_id: SceneId, delta_tiles: i32, delta_texture_bytes: i64) {
        let mut state = self.lock();
        let Some(budget_key) = state
            .sessions
            .get(&session_id)
            .map(|session| session.budget_key.clone())
        else {
            return;
        };
        state.enforcer.apply_mutation_delta(
            &budget_key,
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
        let session_id = SceneId::new();
        assert_eq!(
            enforcer.register_session(
                session_id,
                "agent-a".to_string(),
                ResourceBudget {
                    max_tiles: 1,
                    ..ResourceBudget::default()
                },
                true,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Allow
        );
        assert_eq!(
            enforcer.reserve_mutation(session_id, 1, 0, 1),
            MutationBudgetDecision::Allow
        );

        assert!(matches!(
            enforcer.reserve_mutation(session_id, 1, 0, 1),
            MutationBudgetDecision::Reject {
                error_code: "RESOURCE_BUDGET_EXCEEDED",
                ..
            }
        ));
    }

    #[test]
    fn aggregate_limits_are_atomic_across_agents() {
        let enforcer = RuntimeMutationBudgetEnforcer::with_limits(2, 2, 100);
        let agent_a = SceneId::new();
        let agent_b = SceneId::new();
        for (session_id, name) in [(agent_a, "agent-a"), (agent_b, "agent-b")] {
            assert_eq!(
                enforcer.register_session(
                    session_id,
                    name.to_string(),
                    ResourceBudget {
                        max_tiles: 8,
                        max_texture_bytes: 100,
                        ..ResourceBudget::default()
                    },
                    true,
                    MutationBudgetUsage::default(),
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
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Reject { message, .. } if message.contains("resident_sessions")
        ));

        assert_eq!(
            enforcer.reserve_mutation(agent_a, 1, 60, 1),
            MutationBudgetDecision::Allow
        );
        assert!(matches!(
            enforcer.reserve_mutation(agent_b, 2, 0, 1),
            MutationBudgetDecision::Reject { message, .. } if message.contains("leased_tiles")
        ));
        assert!(matches!(
            enforcer.reserve_mutation(agent_b, 1, 50, 1),
            MutationBudgetDecision::Reject { message, .. } if message.contains("agent_leased_texture_bytes")
        ));
        assert_eq!(
            enforcer.reserve_mutation(agent_b, 1, 40, 1),
            MutationBudgetDecision::Allow
        );
    }

    #[test]
    fn removing_session_releases_its_aggregate_tile_and_texture_usage() {
        let enforcer = RuntimeMutationBudgetEnforcer::with_limits(2, 2, 100);
        let agent_a = SceneId::new();
        assert_eq!(
            enforcer.register_session(
                agent_a,
                "agent-a".to_string(),
                ResourceBudget {
                    max_tiles: 2,
                    max_texture_bytes: 100,
                    ..ResourceBudget::default()
                },
                true,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Allow
        );
        assert_eq!(
            enforcer.reserve_mutation(agent_a, 2, 100, 1),
            MutationBudgetDecision::Allow
        );

        enforcer.remove_session(agent_a);

        let agent_b = SceneId::new();
        assert_eq!(
            enforcer.register_session(
                agent_b,
                "agent-b".to_string(),
                ResourceBudget {
                    max_tiles: 2,
                    max_texture_bytes: 100,
                    ..ResourceBudget::default()
                },
                true,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Allow
        );
        assert_eq!(
            enforcer.reserve_mutation(agent_b, 2, 100, 1),
            MutationBudgetDecision::Allow,
            "disconnect cleanup must not leak aggregate usage"
        );
    }

    #[test]
    fn resumed_session_restores_usage_before_accepting_new_mutations() {
        let enforcer = RuntimeMutationBudgetEnforcer::with_limits(1, 2, 100);
        let resumed = SceneId::new();
        assert_eq!(
            enforcer.register_session(
                resumed,
                "agent-a".to_string(),
                ResourceBudget {
                    max_tiles: 2,
                    max_texture_bytes: 100,
                    ..ResourceBudget::default()
                },
                true,
                MutationBudgetUsage {
                    tiles: 1,
                    texture_bytes: 60,
                },
            ),
            MutationBudgetDecision::Allow
        );
        assert!(matches!(
            enforcer.reserve_mutation(resumed, 1, 50, 1),
            MutationBudgetDecision::Reject { message, .. }
                if message.contains("agent_leased_texture_bytes")
        ));
        assert_eq!(
            enforcer.reserve_mutation(resumed, 1, 40, 1),
            MutationBudgetDecision::Allow
        );
    }

    #[test]
    fn removing_one_of_two_same_namespace_sessions_preserves_the_other() {
        let enforcer = RuntimeMutationBudgetEnforcer::with_limits(2, 2, 100);
        let first = SceneId::new();
        let second = SceneId::new();
        for session_id in [first, second] {
            assert_eq!(
                enforcer.register_session(
                    session_id,
                    "same-agent".to_string(),
                    ResourceBudget {
                        max_tiles: 2,
                        max_texture_bytes: 100,
                        ..ResourceBudget::default()
                    },
                    true,
                    MutationBudgetUsage::default(),
                ),
                MutationBudgetDecision::Allow
            );
        }
        assert_eq!(
            enforcer.reserve_mutation(second, 1, 20, 1),
            MutationBudgetDecision::Allow
        );

        enforcer.remove_session(first);

        assert_eq!(
            enforcer.reserve_mutation(second, 1, 20, 1),
            MutationBudgetDecision::Allow
        );
        assert_eq!(
            enforcer.register_session(
                SceneId::new(),
                "third".to_string(),
                ResourceBudget::default(),
                true,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Allow
        );
        assert!(matches!(
            enforcer.register_session(
                SceneId::new(),
                "fourth".to_string(),
                ResourceBudget::default(),
                true,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Reject { message, .. }
                if message.contains("resident_sessions")
        ));
    }

    #[test]
    fn guest_sessions_use_a_separate_enforced_pool() {
        let enforcer = RuntimeMutationBudgetEnforcer::with_session_limits(1, 1, 2, 100);
        let resident = SceneId::new();
        let guest = SceneId::new();
        assert_eq!(
            enforcer.register_session(
                resident,
                "resident".to_string(),
                ResourceBudget::default(),
                true,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Allow
        );
        assert_eq!(
            enforcer.register_session(
                guest,
                "guest".to_string(),
                ResourceBudget::default(),
                false,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Allow
        );
        assert!(matches!(
            enforcer.register_session(
                SceneId::new(),
                "guest-2".to_string(),
                ResourceBudget::default(),
                false,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Reject { message, .. }
                if message.contains("guest_sessions")
        ));

        enforcer.remove_session(guest);
        assert_eq!(
            enforcer.register_session(
                SceneId::new(),
                "guest-2".to_string(),
                ResourceBudget::default(),
                false,
                MutationBudgetUsage::default(),
            ),
            MutationBudgetDecision::Allow
        );
    }
}
