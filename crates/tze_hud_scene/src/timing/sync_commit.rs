//! Sync group commit policy evaluation.
//!
//! This module implements the two v1 commit policies for sync groups and the
//! force-commit mechanism. It is called by the compositor at Stage 4
//! (Scene Commit) once per sync group per frame.
//!
//! # Spec alignment
//!
//! - Sync Group Commit Policies (timing-model/spec.md lines 141–156)
//! - AllOrDefer Force-Commit (timing-model/spec.md lines 158–173)
//!
//! # Key correctness invariant
//!
//! `deferred_frames_count` (called `deferral_count` on `SyncGroup`) MUST only
//! increment when **at least one member has a pending mutation AND at least one
//! is absent**. When the group is idle (zero pending mutations), the counter
//! MUST NOT change.
//!
//! Spec reference: lines 159 and 167–169.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::types::{SceneId, SyncCommitPolicy, SyncGroup};

use super::sync_group::SyncGroupEvent;

/// Decision returned by [`evaluate_commit`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CommitDecision {
    /// Commit the listed tiles' pending mutations this frame.
    ///
    /// For `AllOrDefer`: all members are present-and-ready.
    /// For `AvailableMembers`: the ready subset (may be empty or partial).
    Commit { tiles: Vec<SceneId> },

    /// Defer the entire group to the next frame.
    ///
    /// Only produced by `AllOrDefer` when some members are absent and
    /// `deferral_count < max_deferrals`.
    Defer,

    /// Force-commit with the present-and-ready members after exhausting
    /// `max_deferrals` consecutive incomplete frames.
    ///
    /// Carries the emitted `SyncGroupForceCommitEvent` so the caller can
    /// route it to the event bus.
    ForceCommit {
        /// Tiles whose pending mutations were applied.
        committed_tiles: Vec<SceneId>,
        /// Tiles whose deferred mutations were discarded.
        discarded_tiles: Vec<SceneId>,
        /// The event that must be delivered to subscribers.
        event: SyncGroupEvent,
    },
}

/// Evaluate a sync group's commit policy given the set of tiles that have
/// pending mutations in this frame's intake window.
///
/// This function is **pure** — it does not mutate `group`. The caller is
/// responsible for applying the returned [`CommitDecision`] (increment
/// `deferral_count`, reset it, etc.) via [`apply_decision`].
///
/// # Arguments
///
/// * `group` — The sync group to evaluate.
/// * `tiles_with_pending` — Tile IDs that have at least one pending mutation
///   ready to commit this frame.
///
/// # Returns
///
/// A [`CommitDecision`] describing what to do. The caller must call
/// [`apply_decision`] to materialise the state changes on `group`.
pub fn evaluate_commit(
    group: &SyncGroup,
    tiles_with_pending: &BTreeSet<SceneId>,
) -> CommitDecision {
    match group.commit_policy {
        SyncCommitPolicy::AvailableMembers => {
            // Apply whatever subset has pending mutations — never defers.
            let ready: Vec<SceneId> = group
                .members
                .iter()
                .filter(|id| tiles_with_pending.contains(id))
                .copied()
                .collect();
            CommitDecision::Commit { tiles: ready }
        }

        SyncCommitPolicy::AllOrDefer => {
            let any_pending = group.members.iter().any(|id| tiles_with_pending.contains(id));

            if !any_pending {
                // Idle frame: no member has a pending mutation.
                // deferred_frames_count MUST NOT increment (spec lines 167–169).
                // Return Commit with empty list so the compositor does nothing.
                return CommitDecision::Commit { tiles: vec![] };
            }

            let all_ready = group.members.iter().all(|id| tiles_with_pending.contains(id));

            if all_ready {
                // All members are present-and-ready — commit atomically.
                let tiles: Vec<SceneId> = group.members.iter().copied().collect();
                CommitDecision::Commit { tiles }
            } else if group.deferral_count < group.max_deferrals {
                // Some members absent; still within the deferral budget.
                CommitDecision::Defer
            } else {
                // Exhausted max_deferrals — force-commit present members.
                let (committed, discarded): (Vec<SceneId>, Vec<SceneId>) = group
                    .members
                    .iter()
                    .copied()
                    .partition(|id| tiles_with_pending.contains(id));
                let event = SyncGroupEvent::ForceCommit {
                    group_id: group.id,
                    committed_tiles: committed.clone(),
                    discarded_tiles: discarded.clone(),
                };
                CommitDecision::ForceCommit {
                    committed_tiles: committed,
                    discarded_tiles: discarded,
                    event,
                }
            }
        }
    }
}

/// Apply the state changes implied by a [`CommitDecision`] to the sync group.
///
/// Must be called immediately after [`evaluate_commit`] returns so that
/// `deferral_count` is kept in sync.
///
/// # Returns
///
/// `true` if the group's `deferral_count` was modified.
pub fn apply_decision(group: &mut SyncGroup, decision: &CommitDecision) -> bool {
    match decision {
        CommitDecision::Commit { .. } => {
            if group.deferral_count != 0 {
                group.deferral_count = 0;
                true
            } else {
                false
            }
        }
        CommitDecision::Defer => {
            group.deferral_count += 1;
            true
        }
        CommitDecision::ForceCommit { .. } => {
            group.deferral_count = 0;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SyncCommitPolicy, SyncGroup, SyncGroupId};

    fn make_group(policy: SyncCommitPolicy, max_deferrals: u32, members: &[SceneId]) -> SyncGroup {
        let id: SyncGroupId = SceneId::new();
        let mut g = SyncGroup::new(id, None, "agent".to_string(), policy, max_deferrals, 0);
        for m in members {
            g.members.insert(*m);
        }
        g
    }

    // ── AvailableMembers ─────────────────────────────────────────────────────

    #[test]
    fn available_members_all_ready_commits_all() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let group = make_group(SyncCommitPolicy::AvailableMembers, 0, &[t1, t2]);

        let mut pending = BTreeSet::new();
        pending.insert(t1);
        pending.insert(t2);

        let decision = evaluate_commit(&group, &pending);
        match decision {
            CommitDecision::Commit { tiles } => {
                assert_eq!(tiles.len(), 2);
            }
            other => panic!("expected Commit, got {:?}", other),
        }
    }

    #[test]
    fn available_members_partial_ready_commits_subset() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let group = make_group(SyncCommitPolicy::AvailableMembers, 0, &[t1, t2]);

        let mut pending = BTreeSet::new();
        pending.insert(t1);
        // t2 is absent

        let decision = evaluate_commit(&group, &pending);
        match decision {
            CommitDecision::Commit { tiles } => {
                assert_eq!(tiles, vec![t1]);
            }
            other => panic!("expected Commit, got {:?}", other),
        }
    }

    #[test]
    fn available_members_none_ready_commits_empty() {
        let t1 = SceneId::new();
        let group = make_group(SyncCommitPolicy::AvailableMembers, 0, &[t1]);
        let pending = BTreeSet::new();

        let decision = evaluate_commit(&group, &pending);
        match decision {
            CommitDecision::Commit { tiles } => {
                assert!(tiles.is_empty());
            }
            other => panic!("expected Commit, got {:?}", other),
        }
    }

    // ── AllOrDefer: all ready ────────────────────────────────────────────────

    #[test]
    fn all_or_defer_all_ready_commits_atomically() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let group = make_group(SyncCommitPolicy::AllOrDefer, 3, &[t1, t2]);

        let mut pending = BTreeSet::new();
        pending.insert(t1);
        pending.insert(t2);

        let decision = evaluate_commit(&group, &pending);
        match decision {
            CommitDecision::Commit { tiles } => {
                assert_eq!(tiles.len(), 2);
                assert!(tiles.contains(&t1));
                assert!(tiles.contains(&t2));
            }
            other => panic!("expected Commit, got {:?}", other),
        }
    }

    // ── AllOrDefer: idle (no pending) ────────────────────────────────────────

    #[test]
    fn all_or_defer_idle_does_not_increment_deferral_count() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let mut group = make_group(SyncCommitPolicy::AllOrDefer, 3, &[t1, t2]);
        group.deferral_count = 0;

        // No pending mutations at all
        let pending = BTreeSet::new();
        let decision = evaluate_commit(&group, &pending);

        // Should return Commit with empty tiles (idle path)
        match &decision {
            CommitDecision::Commit { tiles } => {
                assert!(tiles.is_empty(), "idle frame should produce empty Commit");
            }
            other => panic!("expected empty Commit, got {:?}", other),
        }

        // apply_decision should NOT increment deferral_count
        apply_decision(&mut group, &decision);
        assert_eq!(group.deferral_count, 0, "idle frame must not increment deferral_count");
    }

    // ── AllOrDefer: incomplete (some pending) ────────────────────────────────

    #[test]
    fn all_or_defer_incomplete_defers_and_increments_count() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let mut group = make_group(SyncCommitPolicy::AllOrDefer, 3, &[t1, t2]);

        let mut pending = BTreeSet::new();
        pending.insert(t1); // t2 absent

        let decision = evaluate_commit(&group, &pending);
        assert_eq!(decision, CommitDecision::Defer);

        apply_decision(&mut group, &decision);
        assert_eq!(group.deferral_count, 1);
    }

    #[test]
    fn all_or_defer_defers_up_to_max_then_force_commits() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let mut group = make_group(SyncCommitPolicy::AllOrDefer, 2, &[t1, t2]);

        let mut pending = BTreeSet::new();
        pending.insert(t1); // t2 always absent

        // Frame 1: defer
        let d1 = evaluate_commit(&group, &pending);
        assert_eq!(d1, CommitDecision::Defer);
        apply_decision(&mut group, &d1);
        assert_eq!(group.deferral_count, 1);

        // Frame 2: defer
        let d2 = evaluate_commit(&group, &pending);
        assert_eq!(d2, CommitDecision::Defer);
        apply_decision(&mut group, &d2);
        assert_eq!(group.deferral_count, 2);

        // Frame 3: deferral_count == max_deferrals → force-commit
        let d3 = evaluate_commit(&group, &pending);
        match &d3 {
            CommitDecision::ForceCommit {
                committed_tiles,
                discarded_tiles,
                ..
            } => {
                assert!(committed_tiles.contains(&t1), "t1 should be committed");
                assert!(discarded_tiles.contains(&t2), "t2 should be discarded");
            }
            other => panic!("expected ForceCommit, got {:?}", other),
        }
        apply_decision(&mut group, &d3);
        assert_eq!(group.deferral_count, 0, "force-commit resets deferral_count");
    }

    // ── AllOrDefer: post-force-commit recovery ───────────────────────────────

    #[test]
    fn all_or_defer_resumes_from_zero_after_force_commit() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let mut group = make_group(SyncCommitPolicy::AllOrDefer, 1, &[t1, t2]);

        // One deferral to exhaust budget
        let mut pending = BTreeSet::new();
        pending.insert(t1);
        let d1 = evaluate_commit(&group, &pending);
        apply_decision(&mut group, &d1);

        // Force-commit fires
        let d2 = evaluate_commit(&group, &pending);
        assert!(matches!(d2, CommitDecision::ForceCommit { .. }));
        apply_decision(&mut group, &d2);
        assert_eq!(group.deferral_count, 0, "should be reset after force-commit");

        // Next frame with all members ready — should commit normally
        pending.insert(t2);
        let d3 = evaluate_commit(&group, &pending);
        match d3 {
            CommitDecision::Commit { tiles } => {
                assert_eq!(tiles.len(), 2);
            }
            other => panic!("expected Commit after recovery, got {:?}", other),
        }
    }

    // ── ForceCommit event ────────────────────────────────────────────────────

    #[test]
    fn force_commit_emits_sync_group_force_commit_event() {
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let mut group = make_group(SyncCommitPolicy::AllOrDefer, 1, &[t1, t2]);

        // Exhaust deferrals
        let mut pending = BTreeSet::new();
        pending.insert(t1);

        let d1 = evaluate_commit(&group, &pending);
        apply_decision(&mut group, &d1);

        let d2 = evaluate_commit(&group, &pending);
        match d2 {
            CommitDecision::ForceCommit { event, .. } => {
                assert!(
                    matches!(event, SyncGroupEvent::ForceCommit { .. }),
                    "force-commit must emit SyncGroupForceCommitEvent"
                );
            }
            other => panic!("expected ForceCommit, got {:?}", other),
        }
    }

    // ── apply_decision: no side-effect when already at zero ─────────────────

    #[test]
    fn apply_commit_when_count_already_zero_returns_false() {
        let mut group = make_group(SyncCommitPolicy::AllOrDefer, 3, &[]);
        group.deferral_count = 0;
        let changed = apply_decision(&mut group, &CommitDecision::Commit { tiles: vec![] });
        assert!(!changed);
        assert_eq!(group.deferral_count, 0);
    }
}
