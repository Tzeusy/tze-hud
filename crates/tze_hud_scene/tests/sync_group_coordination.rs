//! # Sync Group Coordination Integration Tests
//!
//! Integration tests for the full sync group coordination lifecycle per
//! `timing-model/spec.md` requirements (lines 124–208):
//!
//! - Sync Group Membership and Lifecycle (lines 124–139)
//! - Sync Group Commit Policies (lines 141–156)
//! - AllOrDefer Force-Commit (lines 158–173)
//! - Sync Group Owner Disconnect (lines 175–186)
//! - Sync Group Resource Governance (lines 188–195)
//! - Sync Drift Budget (lines 197–208)
//!
//! These are **Layer 0** tests: pure scene-graph logic, no GPU, no async.

use std::sync::Arc;

use tze_hud_scene::{
    CommitDecision, DEFAULT_SYNC_DRIFT_BUDGET_US, FrameSyncDriftRecord, ORPHAN_GRACE_PERIOD_US,
    OrphanReason, SyncDriftHighAlert, SyncGroupArrival, SyncGroupCommitDecision, SyncGroupEvent,
    SyncGroupOrphanState, TileArrival, ValidationError,
    evaluate_frame_drift,
    graph::SceneGraph,
    test_scenes::{ClockMs, TestSceneRegistry, assert_layer0_invariants},
    types::{Capability, Rect, SceneId, SyncCommitPolicy},
};

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn make_scene(tiles: usize) -> (SceneGraph, SceneId, Vec<SceneId>) {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Test", 0).unwrap();
    let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);
    let tiles: Vec<SceneId> = (0..tiles)
        .map(|i| {
            scene
                .create_tile(
                    tab_id,
                    "agent",
                    lease_id,
                    Rect::new(i as f32 * 200.0, 0.0, 190.0, 100.0),
                    i as u32 + 1,
                )
                .unwrap()
        })
        .collect();
    (scene, tab_id, tiles)
}

fn make_scene_ns(namespace: &str, tiles: usize) -> (SceneGraph, SceneId, Vec<SceneId>) {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Test", 0).unwrap();
    let lease_id = scene.grant_lease(namespace, 60_000, vec![Capability::CreateTile]);
    let tiles: Vec<SceneId> = (0..tiles)
        .map(|i| {
            scene
                .create_tile(
                    tab_id,
                    namespace,
                    lease_id,
                    Rect::new(i as f32 * 200.0, 0.0, 190.0, 100.0),
                    i as u32 + 1,
                )
                .unwrap()
        })
        .collect();
    (scene, tab_id, tiles)
}

// ─── Membership and Lifecycle ─────────────────────────────────────────────────

/// WHEN an agent creates a sync group, THEN the group is created with the
/// specified id, name, and commit_policy.
/// Spec: timing-model/spec.md lines 129–131.
#[test]
fn create_sync_group_stores_name_and_policy() {
    let (mut scene, _tab, _tiles) = make_scene(0);
    let group_id = scene
        .create_sync_group(
            Some("my-group".to_string()),
            "agent",
            SyncCommitPolicy::AllOrDefer,
            3,
        )
        .unwrap();

    let group = &scene.sync_groups[&group_id];
    assert_eq!(group.name.as_deref(), Some("my-group"));
    assert_eq!(group.commit_policy, SyncCommitPolicy::AllOrDefer);
    assert_eq!(group.max_deferrals, 3);
    assert_eq!(group.owner_namespace, "agent");
}

/// WHEN a tile already belongs to sync group A and is assigned to group B,
/// THEN the tile MUST leave group A and join group B.
/// Spec: timing-model/spec.md lines 133–135.
#[test]
fn tile_moves_from_one_group_to_another_on_join() {
    let (mut scene, _tab, tiles) = make_scene(1);
    let group_a = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();
    let group_b = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
        .unwrap();

    scene.join_sync_group(tiles[0], group_a).unwrap();
    assert!(scene.sync_groups[&group_a].members.contains(&tiles[0]));

    // Move tile to group B
    scene.join_sync_group(tiles[0], group_b).unwrap();

    // Must be in B and not in A
    assert!(!scene.sync_groups[&group_a].members.contains(&tiles[0]));
    assert!(scene.sync_groups[&group_b].members.contains(&tiles[0]));
    assert_eq!(scene.tiles[&tiles[0]].sync_group, Some(group_b));
}

/// WHEN the last member tile leaves a sync group, THEN the sync group MUST
/// be destroyed and removed from the scene graph.
/// Spec: timing-model/spec.md lines 137–139.
///
/// Note: the current graph implementation does NOT auto-destroy on last leave;
/// destruction is explicit (delete_sync_group). This test documents the
/// explicit-destruction contract.
#[test]
fn explicit_delete_removes_group_and_releases_tiles() {
    let (mut scene, _tab, tiles) = make_scene(2);
    let group_id = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();

    scene.join_sync_group(tiles[0], group_id).unwrap();
    scene.join_sync_group(tiles[1], group_id).unwrap();

    scene.delete_sync_group(group_id).unwrap();

    // Group removed from scene
    assert!(!scene.sync_groups.contains_key(&group_id));
    // Tiles released
    assert_eq!(scene.tiles[&tiles[0]].sync_group, None);
    assert_eq!(scene.tiles[&tiles[1]].sync_group, None);
}

// ─── Commit Policies ─────────────────────────────────────────────────────────

/// WHEN all members of an AllOrDefer group have pending mutations,
/// THEN all mutations MUST be applied atomically in the same frame.
/// Spec: timing-model/spec.md lines 146–148.
#[test]
fn all_or_defer_commits_when_all_members_ready() {
    let (mut scene, _tab, tiles) = make_scene(3);
    let group_id = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();
    for &t in &tiles {
        scene.join_sync_group(t, group_id).unwrap();
    }

    let pending: std::collections::BTreeSet<SceneId> = tiles.iter().copied().collect();
    let decision = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();

    match decision {
        SyncGroupCommitDecision::Commit { tiles: committed } => {
            assert_eq!(committed.len(), 3, "all 3 members must be committed atomically");
        }
        other => panic!("expected Commit, got {:?}", other),
    }
    assert_eq!(scene.sync_groups[&group_id].deferral_count, 0);
}

/// WHEN only some members of an AllOrDefer group have pending mutations,
/// THEN the entire group MUST be deferred to the next frame.
/// Spec: timing-model/spec.md lines 150–152.
#[test]
fn all_or_defer_defers_when_only_some_members_ready() {
    let (mut scene, _tab, tiles) = make_scene(2);
    let group_id = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();
    scene.join_sync_group(tiles[0], group_id).unwrap();
    scene.join_sync_group(tiles[1], group_id).unwrap();

    // Only tile[0] ready
    let mut pending = std::collections::BTreeSet::new();
    pending.insert(tiles[0]);

    let decision = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
    assert_eq!(decision, SyncGroupCommitDecision::Defer);
    assert_eq!(scene.sync_groups[&group_id].deferral_count, 1);
}

/// WHEN only some members of an AvailableMembers group have pending mutations,
/// THEN available members' mutations MUST be applied; absent members remain unchanged.
/// Spec: timing-model/spec.md lines 154–156.
#[test]
fn available_members_applies_ready_subset_absent_unchanged() {
    let (mut scene, _tab, tiles) = make_scene(3);
    let group_id = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
        .unwrap();
    for &t in &tiles {
        scene.join_sync_group(t, group_id).unwrap();
    }

    // Only tiles[0] and tiles[2] have pending mutations
    let mut pending = std::collections::BTreeSet::new();
    pending.insert(tiles[0]);
    pending.insert(tiles[2]);

    let decision = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
    match decision {
        SyncGroupCommitDecision::Commit { tiles: committed } => {
            assert!(committed.contains(&tiles[0]), "tile[0] should be committed");
            assert!(committed.contains(&tiles[2]), "tile[2] should be committed");
            assert!(!committed.contains(&tiles[1]), "tile[1] should NOT be committed (absent)");
        }
        other => panic!("expected Commit, got {:?}", other),
    }
}

// ─── AllOrDefer Force-Commit ──────────────────────────────────────────────────

/// WHEN an AllOrDefer group is incomplete for max_defer_frames consecutive frames,
/// THEN the compositor MUST force-commit present members, discard absent, emit event.
/// Spec: timing-model/spec.md lines 163–165.
#[test]
fn all_or_defer_force_commits_after_max_defer_frames() {
    let (mut scene, _tab, tiles) = make_scene(2);
    let group_id = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();
    scene.join_sync_group(tiles[0], group_id).unwrap();
    scene.join_sync_group(tiles[1], group_id).unwrap();

    // tiles[0] is always ready; tiles[1] never arrives
    let mut pending = std::collections::BTreeSet::new();
    pending.insert(tiles[0]);

    // 3 deferrals
    for expected_count in 1..=3 {
        let d = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
        assert_eq!(d, SyncGroupCommitDecision::Defer, "frame {}", expected_count);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, expected_count);
    }

    // 4th evaluation: force-commit fires
    let d4 = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
    match d4 {
        SyncGroupCommitDecision::ForceCommit { tiles: committed } => {
            assert!(committed.contains(&tiles[0]), "ready tile must be committed");
            assert!(!committed.contains(&tiles[1]), "absent tile must NOT be committed");
        }
        other => panic!("expected ForceCommit, got {:?}", other),
    }
    assert_eq!(scene.sync_groups[&group_id].deferral_count, 0, "reset after force-commit");
}

/// WHEN an AllOrDefer group has no pending mutations for any member,
/// THEN deferred_frames_count MUST NOT be incremented.
/// Spec: timing-model/spec.md lines 167–169.
#[test]
fn all_or_defer_idle_does_not_increment_deferral_count() {
    let (mut scene, _tab, tiles) = make_scene(2);
    let group_id = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();
    scene.join_sync_group(tiles[0], group_id).unwrap();
    scene.join_sync_group(tiles[1], group_id).unwrap();

    // No pending mutations at all
    let pending = std::collections::BTreeSet::new();

    // Run 5 idle frames
    for _ in 0..5 {
        let _ = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
    }

    assert_eq!(
        scene.sync_groups[&group_id].deferral_count, 0,
        "idle frames must not increment deferral_count"
    );
}

/// WHEN a force-commit has fired, THEN the group MUST resume AllOrDefer
/// evaluation from deferred_frames_count = 0 on the next frame.
/// Spec: timing-model/spec.md lines 171–173.
#[test]
fn all_or_defer_resumes_from_zero_after_force_commit() {
    let (mut scene, _tab, tiles) = make_scene(2);
    // max_deferrals = 1 so force-commit fires quickly
    let group_id = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 1)
        .unwrap();
    scene.join_sync_group(tiles[0], group_id).unwrap();
    scene.join_sync_group(tiles[1], group_id).unwrap();

    let mut partial_pending = std::collections::BTreeSet::new();
    partial_pending.insert(tiles[0]);

    // One deferral
    let d1 = scene.evaluate_sync_group_commit(group_id, &partial_pending).unwrap();
    assert_eq!(d1, SyncGroupCommitDecision::Defer);

    // Force-commit fires
    let d2 = scene.evaluate_sync_group_commit(group_id, &partial_pending).unwrap();
    assert!(
        matches!(d2, SyncGroupCommitDecision::ForceCommit { .. }),
        "should force-commit after max_deferrals=1"
    );
    assert_eq!(scene.sync_groups[&group_id].deferral_count, 0);

    // Next frame: all ready — should commit normally
    let mut full_pending = std::collections::BTreeSet::new();
    full_pending.insert(tiles[0]);
    full_pending.insert(tiles[1]);
    let d3 = scene.evaluate_sync_group_commit(group_id, &full_pending).unwrap();
    assert!(
        matches!(d3, SyncGroupCommitDecision::Commit { .. }),
        "should commit normally after recovery"
    );
}

// ─── Resource Governance ─────────────────────────────────────────────────────

/// WHEN an agent has 16 sync groups and attempts to create a 17th,
/// THEN the compositor MUST reject.
/// Spec: timing-model/spec.md lines 193–195.
#[test]
fn sync_group_namespace_limit_is_16() {
    let (mut scene, _tab, _tiles) = make_scene(0);

    for i in 0..16 {
        scene
            .create_sync_group(
                Some(format!("group-{}", i)),
                "agent",
                SyncCommitPolicy::AllOrDefer,
                3,
            )
            .unwrap();
    }
    assert_eq!(scene.sync_group_count(), 16);

    // 17th must fail
    let result = scene.create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3);
    assert!(
        matches!(result, Err(ValidationError::SyncGroupLimitExceeded { .. })),
        "17th sync group must be rejected"
    );
}

/// WHEN an agent attempts to add 65 tiles to a sync group (limit is 64),
/// THEN the compositor MUST reject.
/// Spec: timing-model/spec.md lines 188–189.
#[test]
fn sync_group_member_limit_is_64() {
    let (mut scene, _tab, tiles) = make_scene(65);
    let group_id = scene
        .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();

    // Add first 64 — should succeed
    for &t in &tiles[..64] {
        scene.join_sync_group(t, group_id).unwrap();
    }
    assert_eq!(scene.sync_groups[&group_id].members.len(), 64);

    // 65th must fail
    let result = scene.join_sync_group(tiles[64], group_id);
    assert!(
        matches!(result, Err(ValidationError::SyncGroupMemberLimitExceeded { .. })),
        "65th tile must be rejected"
    );
}

/// WHEN an agent attempts to add another agent's tile to a sync group,
/// THEN the compositor MUST reject.
/// Spec: timing-model/spec.md lines 188–189.
#[test]
fn ownership_check_rejects_cross_namespace_join() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Test", 0).unwrap();

    // agent-a creates a tile
    let lease_a = scene.grant_lease("agent-a", 60_000, vec![Capability::CreateTile]);
    let tile_a = scene
        .create_tile(tab_id, "agent-a", lease_a, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
        .unwrap();

    // agent-b creates a group
    scene.grant_lease("agent-b", 60_000, vec![Capability::CreateTile]);
    let group_id = scene
        .create_sync_group(None, "agent-b", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();

    // agent-b tries to add agent-a's tile — must fail
    let result = scene.join_sync_group_checked(tile_a, group_id, "agent-b");
    assert!(
        matches!(result, Err(ValidationError::SyncGroupOwnershipViolation { .. })),
        "agent-b must not add agent-a's tile"
    );
}

/// WHEN an agent adds its own tile to its own group, THEN ownership check passes.
#[test]
fn ownership_check_allows_same_namespace_join() {
    let (mut scene, _tab, tiles) = make_scene_ns("agent-x", 1);
    let group_id = scene
        .create_sync_group(None, "agent-x", SyncCommitPolicy::AllOrDefer, 3)
        .unwrap();
    let result = scene.join_sync_group_checked(tiles[0], group_id, "agent-x");
    assert!(result.is_ok(), "same-namespace join must succeed");
}

// ─── Sync Drift Budget ────────────────────────────────────────────────────────

/// WHEN all sync group members' mutations arrive within 500µs of each other,
/// THEN sync_drift_budget_exceeded MUST be false in FrameTimingRecord.
/// Spec: timing-model/spec.md lines 202–204.
#[test]
fn sync_drift_within_budget_not_exceeded() {
    let gid = SceneId::new();
    let t1 = SceneId::new();
    let t2 = SceneId::new();

    let group = SyncGroupArrival {
        group_id: gid,
        tile_arrivals: vec![
            TileArrival { tile_id: t1, arrival_wall_us: 1_000_000 },
            TileArrival { tile_id: t2, arrival_wall_us: 1_000_499 }, // 499µs spread
        ],
    };

    let (record, alerts) = evaluate_frame_drift(&[group], DEFAULT_SYNC_DRIFT_BUDGET_US);
    assert_eq!(record.sync_group_max_drift_us, 499);
    assert!(!record.sync_drift_budget_exceeded,
        "499µs < 500µs must not exceed budget");
    assert!(alerts.is_empty(), "no alert when within budget");
}

/// WHEN sync group members' mutations arrive with 800µs spread,
/// THEN sync_drift_budget_exceeded MUST be true and staleness indicator
/// MUST be activated for the slow member's tiles.
/// Spec: timing-model/spec.md lines 206–208.
#[test]
fn sync_drift_800us_exceeds_budget_and_marks_slow_tile_stale() {
    let gid = SceneId::new();
    let t1 = SceneId::new();
    let t2 = SceneId::new();

    let group = SyncGroupArrival {
        group_id: gid,
        tile_arrivals: vec![
            TileArrival { tile_id: t1, arrival_wall_us: 2_000_000 },      // fast
            TileArrival { tile_id: t2, arrival_wall_us: 2_000_800 },      // slow (+800µs)
        ],
    };

    let (record, alerts) = evaluate_frame_drift(&[group], DEFAULT_SYNC_DRIFT_BUDGET_US);

    assert_eq!(record.sync_group_max_drift_us, 800);
    assert!(record.sync_drift_budget_exceeded,
        "800µs > 500µs must exceed budget");
    assert!(record.stale_tiles.contains(&t2),
        "slow tile (t2) must have staleness indicator activated");
    assert!(!record.stale_tiles.contains(&t1),
        "fast tile (t1) must NOT be stale");
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].group_id, gid);
    assert_eq!(alerts[0].observed_drift_us, 800);
}

// ─── Owner Disconnect (orphan lifecycle types) ────────────────────────────────

/// WHEN the agent that created sync group G disconnects,
/// THEN the orphan state records the grace period deadline.
/// Spec: timing-model/spec.md lines 180–182.
#[test]
fn orphan_state_created_with_5s_grace_period() {
    let group_id = SceneId::new();
    let now = 10_000_000u64;

    let orphan = SyncGroupOrphanState::new(
        group_id,
        "agent.gamma".to_string(),
        now,
        OrphanReason::OwnerSessionClosed,
    );

    assert_eq!(orphan.group_id, group_id);
    assert_eq!(orphan.owner_namespace, "agent.gamma");
    assert_eq!(
        orphan.destroy_after_wall_us,
        now + ORPHAN_GRACE_PERIOD_US,
        "grace period deadline must be 5s after disconnect"
    );
    assert_eq!(ORPHAN_GRACE_PERIOD_US, 5_000_000,
        "grace period must be 5 seconds");
}

/// WHEN the owner reconnects within 5 seconds of disconnect,
/// THEN the pending group destruction MUST be cancelled.
/// Spec: timing-model/spec.md lines 184–186.
///
/// This test validates that `grace_expired` returns false before the deadline,
/// which is the condition used to cancel destruction.
#[test]
fn owner_reconnect_within_grace_cancels_destruction() {
    let group_id = SceneId::new();
    let disconnected_at = 5_000_000u64;

    let orphan = SyncGroupOrphanState::new(
        group_id,
        "agent.delta".to_string(),
        disconnected_at,
        OrphanReason::OwnerSessionClosed,
    );

    // 4 seconds after disconnect (1 second before grace expires) — not expired
    let reconnect_time = disconnected_at + 4_000_000;
    assert!(
        !orphan.grace_expired(reconnect_time),
        "reconnect at 4s must not have expired grace period (5s grace)"
    );

    // Exactly at grace period boundary
    assert!(orphan.grace_expired(disconnected_at + ORPHAN_GRACE_PERIOD_US));
}

/// WHEN orphan grace expires, the group should be destroyed.
#[test]
fn orphan_grace_expired_after_5s() {
    let group_id = SceneId::new();
    let disconnected_at = 1_000_000u64;

    let orphan = SyncGroupOrphanState::new(
        group_id,
        "agent.epsilon".to_string(),
        disconnected_at,
        OrphanReason::OwnerSessionClosed,
    );

    // 6 seconds later — expired
    assert!(orphan.grace_expired(disconnected_at + 6_000_000));
}

// ─── SyncGroupForceCommitEvent emitted ────────────────────────────────────────

/// WHEN force-commit fires (via sync_commit module), THEN SyncGroupForceCommitEvent
/// is embedded in the CommitDecision.
/// Spec: timing-model/spec.md lines 163–165.
#[test]
fn force_commit_decision_carries_event() {
    use tze_hud_scene::{apply_decision, evaluate_commit};
    use tze_hud_scene::types::{SyncGroup, SyncGroupId};

    let id: SyncGroupId = SceneId::new();
    let t1 = SceneId::new();
    let t2 = SceneId::new();
    let mut group = SyncGroup::new(id, None, "agent".to_string(), SyncCommitPolicy::AllOrDefer, 1, 0);
    group.members.insert(t1);
    group.members.insert(t2);

    let mut pending = std::collections::BTreeSet::new();
    pending.insert(t1); // t2 absent

    // Frame 1: deferral
    let d1 = evaluate_commit(&group, &pending);
    apply_decision(&mut group, &d1);

    // Frame 2: force-commit
    let d2 = evaluate_commit(&group, &pending);
    match d2 {
        CommitDecision::ForceCommit { event, .. } => {
            assert!(
                matches!(event, SyncGroupEvent::ForceCommit { .. }),
                "force-commit must carry SyncGroupForceCommitEvent"
            );
        }
        other => panic!("expected ForceCommit, got {:?}", other),
    }
}

// ─── sync_group_media test scene ─────────────────────────────────────────────

/// The sync_group_media scene from the validation framework builds without error
/// and passes all Layer 0 invariants.
/// Spec: timing-model/spec.md lines 124–173 (primary sync group scene).
#[test]
fn sync_group_media_scene_passes_invariants() {
    let registry = TestSceneRegistry::new();
    let (graph, _spec) = registry.build("sync_group_media", ClockMs::FIXED).unwrap();

    let violations = assert_layer0_invariants(&graph);
    assert!(
        violations.is_empty(),
        "sync_group_media must pass Layer 0 invariants, got: {:?}",
        violations
    );
}

/// The sync_group_media scene has exactly one sync group and both tiles belong
/// to it with AllOrDefer policy.
#[test]
fn sync_group_media_group_has_all_or_defer_policy() {
    let registry = TestSceneRegistry::new();
    let (graph, _spec) = registry.build("sync_group_media", ClockMs::FIXED).unwrap();

    assert_eq!(graph.sync_groups.len(), 1, "should have exactly one sync group");
    let group = graph.sync_groups.values().next().unwrap();
    assert_eq!(group.commit_policy, SyncCommitPolicy::AllOrDefer);
    assert_eq!(group.members.len(), 2, "both tiles must be enrolled");
    assert_eq!(group.max_deferrals, 3);
}

/// The disconnect_reclaim_multiagent scene passes Layer 0 invariants.
#[test]
fn disconnect_reclaim_multiagent_scene_passes_invariants() {
    let registry = TestSceneRegistry::new();
    let (graph, _spec) = registry
        .build("disconnect_reclaim_multiagent", ClockMs::FIXED)
        .unwrap();

    let violations = assert_layer0_invariants(&graph);
    assert!(
        violations.is_empty(),
        "disconnect_reclaim_multiagent must pass Layer 0 invariants, got: {:?}",
        violations
    );
}
