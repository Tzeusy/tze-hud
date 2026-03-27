//! Sync group membership, lifecycle, and orphan state.
//!
//! This module provides types and logic for managing sync group lifecycle
//! beyond basic CRUD: orphan tracking, grace-period destruction, and
//! reconnect cancellation.
//!
//! # Spec alignment
//!
//! - Sync Group Membership and Lifecycle (timing-model/spec.md lines 124–139)
//! - Sync Group Owner Disconnect (timing-model/spec.md lines 175–186)
//! - Sync Group Resource Governance (timing-model/spec.md lines 188–195)

use serde::{Deserialize, Serialize};

use crate::timing::domains::{DurationUs, WallUs};
use crate::types::SceneId;

/// Grace period after an owner disconnect before a sync group is destroyed.
///
/// Spec: timing-model/spec.md line 176 — "destroy the group after a 5-second
/// grace period".
pub const ORPHAN_GRACE_PERIOD_US: DurationUs = DurationUs(5_000_000); // 5 seconds

/// Reasons a sync group can enter the orphaned state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrphanReason {
    /// The agent session that created the sync group closed.
    OwnerSessionClosed,
}

/// Runtime orphan state associated with a sync group whose owner has disconnected.
///
/// The compositor attaches this to a sync group when the owning namespace
/// session closes. If the owner reconnects within [`ORPHAN_GRACE_PERIOD_US`],
/// `SyncGroupOrphanState::cancel_destruction` removes the orphan state.
/// Otherwise, after the grace period the group is destroyed.
///
/// Spec: timing-model/spec.md lines 175–186.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncGroupOrphanState {
    /// The group that has been orphaned.
    pub group_id: SceneId,
    /// Owner namespace (for reconnect matching).
    pub owner_namespace: String,
    /// Wall-clock time when the grace period expires.
    pub destroy_after_wall_us: WallUs,
    /// Why this group became orphaned.
    pub reason: OrphanReason,
}

impl SyncGroupOrphanState {
    /// Create a new orphan state record.
    ///
    /// `orphaned_at_wall_us` — the wall-clock time of the disconnect event.
    pub fn new(
        group_id: SceneId,
        owner_namespace: String,
        orphaned_at_wall_us: WallUs,
        reason: OrphanReason,
    ) -> Self {
        Self {
            group_id,
            owner_namespace,
            destroy_after_wall_us: ORPHAN_GRACE_PERIOD_US.after_wall(orphaned_at_wall_us),
            reason,
        }
    }

    /// Returns `true` if the grace period has elapsed.
    ///
    /// `now_wall_us` — current wall-clock time.
    pub fn grace_expired(&self, now_wall_us: WallUs) -> bool {
        now_wall_us >= self.destroy_after_wall_us
    }
}

/// Events emitted by sync group lifecycle operations.
///
/// These are returned by methods that trigger lifecycle transitions so callers
/// can route them into the event bus without this crate depending on the full
/// event taxonomy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SyncGroupEvent {
    /// Emitted when an AllOrDefer sync group exceeds `max_defer_frames` and
    /// the compositor applies a force-commit.
    ///
    /// Spec: timing-model/spec.md lines 163–165.
    ForceCommit {
        /// ID of the group that was force-committed.
        group_id: SceneId,
        /// Tile IDs that were committed (present-and-ready members).
        committed_tiles: Vec<SceneId>,
        /// Tile IDs whose deferred mutations were discarded (absent members).
        discarded_tiles: Vec<SceneId>,
    },

    /// Emitted when a sync group's owner session closes.
    ///
    /// The group enters a 5-second grace period before destruction.
    /// Spec: timing-model/spec.md lines 180–182.
    Orphaned {
        /// ID of the orphaned group.
        group_id: SceneId,
        /// Owner namespace that disconnected.
        owner_namespace: String,
        /// Wall-clock time when the grace period ends.
        destroy_after_wall_us: WallUs,
    },

    /// Emitted when a previously orphaned sync group is reclaimed because the
    /// owner reconnected within the grace period.
    ///
    /// Spec: timing-model/spec.md lines 184–186.
    OrphanCancelled {
        /// ID of the group whose destruction was cancelled.
        group_id: SceneId,
        /// Owner namespace that reconnected.
        owner_namespace: String,
    },
}

/// Validates that an agent is allowed to add a tile to a sync group.
///
/// An agent MUST NOT place another agent's tiles into a sync group.
///
/// # Arguments
///
/// * `agent_namespace` — The namespace of the agent attempting the operation.
/// * `tile_namespace` — The namespace that owns the tile.
/// * `group_namespace` — The namespace that owns the sync group.
///
/// # Returns
///
/// `Ok(())` if the operation is permitted. `Err` with a descriptive message
/// otherwise.
///
/// Spec: timing-model/spec.md lines 188–189.
pub fn check_sync_group_ownership(
    agent_namespace: &str,
    tile_namespace: &str,
    group_namespace: &str,
) -> Result<(), String> {
    if tile_namespace != agent_namespace {
        return Err(format!(
            "agent '{}' is not permitted to add tile owned by '{}' to a sync group",
            agent_namespace, tile_namespace
        ));
    }
    if group_namespace != agent_namespace {
        return Err(format!(
            "agent '{}' is not permitted to modify sync group owned by '{}'",
            agent_namespace, group_namespace
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── OrphanState ─────────────────────────────────────────────────────────

    #[test]
    fn orphan_state_grace_not_expired_before_deadline() {
        let orphaned_at = WallUs(10_000_000);
        let state = SyncGroupOrphanState::new(
            SceneId::new(),
            "agent.alpha".to_string(),
            orphaned_at,
            OrphanReason::OwnerSessionClosed,
        );
        // 1 second into grace period — not yet expired
        assert!(!state.grace_expired(WallUs(orphaned_at.0 + 1_000_000)));
    }

    #[test]
    fn orphan_state_grace_expired_at_deadline() {
        let orphaned_at = WallUs(10_000_000);
        let state = SyncGroupOrphanState::new(
            SceneId::new(),
            "agent.alpha".to_string(),
            orphaned_at,
            OrphanReason::OwnerSessionClosed,
        );
        // Exactly at the deadline
        assert!(state.grace_expired(WallUs(orphaned_at.0 + ORPHAN_GRACE_PERIOD_US.0)));
    }

    #[test]
    fn orphan_state_grace_expired_after_deadline() {
        let orphaned_at = WallUs(10_000_000);
        let state = SyncGroupOrphanState::new(
            SceneId::new(),
            "agent.beta".to_string(),
            orphaned_at,
            OrphanReason::OwnerSessionClosed,
        );
        assert!(state.grace_expired(WallUs(orphaned_at.0 + ORPHAN_GRACE_PERIOD_US.0 + 1)));
    }

    #[test]
    fn orphan_grace_period_is_5_seconds() {
        assert_eq!(ORPHAN_GRACE_PERIOD_US.0, 5_000_000);
    }

    // ── Ownership check ──────────────────────────────────────────────────────

    #[test]
    fn ownership_check_same_namespace_ok() {
        assert!(check_sync_group_ownership("agent.a", "agent.a", "agent.a").is_ok());
    }

    #[test]
    fn ownership_check_cross_tile_namespace_rejected() {
        let result = check_sync_group_ownership("agent.a", "agent.b", "agent.a");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("agent.b"),
            "error should name the foreign tile namespace"
        );
    }

    #[test]
    fn ownership_check_cross_group_namespace_rejected() {
        let result = check_sync_group_ownership("agent.a", "agent.a", "agent.b");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("agent.b"),
            "error should name the foreign group namespace"
        );
    }

    // ── SyncGroupEvent ────────────────────────────────────────────────────────

    #[test]
    fn force_commit_event_fields() {
        let gid = SceneId::new();
        let t1 = SceneId::new();
        let t2 = SceneId::new();
        let ev = SyncGroupEvent::ForceCommit {
            group_id: gid,
            committed_tiles: vec![t1],
            discarded_tiles: vec![t2],
        };
        if let SyncGroupEvent::ForceCommit {
            group_id,
            committed_tiles,
            discarded_tiles,
        } = ev
        {
            assert_eq!(group_id, gid);
            assert_eq!(committed_tiles, vec![t1]);
            assert_eq!(discarded_tiles, vec![t2]);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn orphaned_event_fields() {
        let gid = SceneId::new();
        let ev = SyncGroupEvent::Orphaned {
            group_id: gid,
            owner_namespace: "agent.x".to_string(),
            destroy_after_wall_us: WallUs(15_000_000),
        };
        if let SyncGroupEvent::Orphaned {
            group_id,
            owner_namespace,
            destroy_after_wall_us,
        } = ev
        {
            assert_eq!(group_id, gid);
            assert_eq!(owner_namespace, "agent.x");
            assert_eq!(destroy_after_wall_us, WallUs(15_000_000));
        } else {
            panic!("wrong variant");
        }
    }
}
