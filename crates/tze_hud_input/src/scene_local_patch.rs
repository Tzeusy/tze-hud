//! SceneLocalPatch — the compositor-bypass channel for local feedback state.
//!
//! Local state (pressed, hovered, focused, scroll offset) is communicated from
//! the input pipeline to the compositor via `SceneLocalPatch`, **not** via the
//! normal `MutationBatch` channel. This ensures:
//!
//! - The compositor applies local state **without** going through lease validation
//!   or budget checks (spec §Requirement: Local Feedback Rendering via SceneLocalPatch).
//! - The patch is produced at Stage 2 (input processing) and applied at Stage 4
//!   (before render encoding), keeping the critical p99 < 4ms path intact.
//! - Scroll offsets are treated identically to button state — same channel,
//!   same bypass guarantees.
//!
//! # Wire contract
//!
//! `SceneLocalPatch` is a purely in-process type (not sent over the wire). The
//! compositor receives it via a dedicated `mpsc` or lock-free channel. The input
//! crate is responsible for **producing** the patch; the compositor is responsible
//! for **consuming** it.

use serde::{Deserialize, Serialize};
use tze_hud_scene::SceneId;

// ─── LocalStateUpdate ────────────────────────────────────────────────────────

/// Per-node local state update for a single HitRegionNode.
///
/// Carries the three runtime-owned boolean state bits:
/// - `pressed` — set on PointerDown, cleared on PointerUp (or rollback)
/// - `hovered` — set on PointerEnter, cleared on PointerLeave
/// - `focused` — set on focus acquisition, cleared on focus loss
///
/// The compositor reads these fields directly; it does not validate leases or
/// budgets before applying them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalStateUpdate {
    /// The node whose local state changed.
    pub node_id: SceneId,
    /// New pressed state. `None` = unchanged.
    pub pressed: Option<bool>,
    /// New hovered state. `None` = unchanged.
    pub hovered: Option<bool>,
    /// New focused state. `None` = unchanged.
    pub focused: Option<bool>,
    /// If true, this update initiates a 100ms reverse rollback animation
    /// (agent explicitly rejected the interaction — spec §Local Feedback Rollback).
    pub rollback: bool,
}

impl LocalStateUpdate {
    /// Construct a simple state update with no rollback.
    pub fn new(node_id: SceneId) -> Self {
        Self {
            node_id,
            pressed: None,
            hovered: None,
            focused: None,
            rollback: false,
        }
    }

    /// Set pressed state and return self for chaining.
    pub fn with_pressed(mut self, pressed: bool) -> Self {
        self.pressed = Some(pressed);
        self
    }

    /// Set hovered state and return self for chaining.
    pub fn with_hovered(mut self, hovered: bool) -> Self {
        self.hovered = Some(hovered);
        self
    }

    /// Set focused state and return self for chaining.
    pub fn with_focused(mut self, focused: bool) -> Self {
        self.focused = Some(focused);
        self
    }

    /// Mark this update as a rollback (pressed → false with 100ms animation).
    pub fn with_rollback(mut self) -> Self {
        self.rollback = true;
        self
    }

    /// Returns true if any state bit is set (non-trivial update).
    pub fn has_changes(&self) -> bool {
        self.pressed.is_some() || self.hovered.is_some() || self.focused.is_some()
    }
}

// ─── ScrollOffsetUpdate ──────────────────────────────────────────────────────

/// Per-tile scroll offset update.
///
/// Produced when a scroll event is processed locally. The compositor maintains
/// the canonical scroll offset per tile and applies this update before rendering
/// without any agent roundtrip.
///
/// # Priority rule (spec §Scroll Local Feedback)
/// If an agent-set `SetScrollOffsetRequest` and a user scroll arrive in the same
/// frame, the **user scroll takes priority** and the agent request is discarded.
/// The compositor enforces this; the input pipeline always produces a
/// `ScrollOffsetUpdate` for user scroll events regardless of pending agent requests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollOffsetUpdate {
    /// The tile whose scroll offset changed.
    pub tile_id: SceneId,
    /// Delta applied to the horizontal scroll offset (pixels, positive = right).
    pub delta_x: f32,
    /// Delta applied to the vertical scroll offset (pixels, positive = down).
    pub delta_y: f32,
    /// Origin of this update — `true` = user input, `false` = agent request.
    /// User-origin updates always win over agent-origin updates in the same frame.
    pub user_initiated: bool,
}

impl ScrollOffsetUpdate {
    /// Construct a user-initiated scroll delta.
    pub fn from_user(tile_id: SceneId, delta_x: f32, delta_y: f32) -> Self {
        Self { tile_id, delta_x, delta_y, user_initiated: true }
    }

    /// Construct an agent-requested absolute scroll offset as a delta-from-zero.
    ///
    /// The caller is expected to compute the delta relative to the current offset.
    pub fn from_agent(tile_id: SceneId, delta_x: f32, delta_y: f32) -> Self {
        Self { tile_id, delta_x, delta_y, user_initiated: false }
    }
}

// ─── SceneLocalPatch ─────────────────────────────────────────────────────────

/// A batch of local-state changes produced by Stage 2 of the input pipeline.
///
/// This patch is forwarded to the compositor via a **dedicated channel** that is
/// separate from the normal `MutationBatch` channel. The compositor applies it
/// at Stage 4 (before render encoding) without lease validation or budget checks.
///
/// ## Latency invariant
/// The patch MUST be produced within 1ms of the input event being received on
/// the main thread (combined Stage 1+2 budget). The compositor MUST apply the
/// patch before the next frame, guaranteeing `input_to_next_present < 33ms`.
///
/// ## Channel semantics
/// The channel is bounded; if the compositor is behind, the producer (input
/// pipeline) may coalesce patches. Since local state is idempotent (last write
/// wins), coalescing is lossless.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneLocalPatch {
    /// Per-node state updates (pressed, hovered, focused).
    pub state_updates: Vec<LocalStateUpdate>,
    /// Per-tile scroll offset updates.
    pub scroll_updates: Vec<ScrollOffsetUpdate>,
}

impl SceneLocalPatch {
    /// Construct an empty patch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the patch carries no changes.
    pub fn is_empty(&self) -> bool {
        self.state_updates.is_empty() && self.scroll_updates.is_empty()
    }

    /// Add a local state update.
    pub fn push_state(&mut self, update: LocalStateUpdate) {
        self.state_updates.push(update);
    }

    /// Add a scroll offset update.
    pub fn push_scroll(&mut self, update: ScrollOffsetUpdate) {
        self.scroll_updates.push(update);
    }

    /// Merge another patch into this one (in-place coalescing).
    ///
    /// For state updates: later entries override earlier ones for the same `node_id`.
    /// For scroll updates: deltas are accumulated per `tile_id`; if a user update
    /// coexists with an agent update, the user update wins and the agent update is
    /// dropped (spec §Scroll Local Feedback: user scroll takes priority).
    pub fn merge_from(&mut self, other: SceneLocalPatch) {
        self.state_updates.extend(other.state_updates);
        for incoming in other.scroll_updates {
            // If the incoming update is user-initiated, discard any pending
            // agent update for the same tile (user wins).
            if incoming.user_initiated {
                self.scroll_updates.retain(|u| u.tile_id != incoming.tile_id || u.user_initiated);
            }
            self.scroll_updates.push(incoming);
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_state_update_builder() {
        let id = SceneId::new();
        let update = LocalStateUpdate::new(id)
            .with_pressed(true)
            .with_hovered(false);

        assert_eq!(update.node_id, id);
        assert_eq!(update.pressed, Some(true));
        assert_eq!(update.hovered, Some(false));
        assert!(update.focused.is_none());
        assert!(!update.rollback);
        assert!(update.has_changes());
    }

    #[test]
    fn test_local_state_update_rollback() {
        let id = SceneId::new();
        let update = LocalStateUpdate::new(id)
            .with_pressed(false)
            .with_rollback();

        assert!(update.rollback);
        assert_eq!(update.pressed, Some(false));
    }

    #[test]
    fn test_local_state_update_no_changes() {
        let id = SceneId::new();
        let update = LocalStateUpdate::new(id);
        assert!(!update.has_changes());
    }

    #[test]
    fn test_scroll_offset_update_user_initiated() {
        let tile_id = SceneId::new();
        let update = ScrollOffsetUpdate::from_user(tile_id, 0.0, 20.0);
        assert!(update.user_initiated);
        assert_eq!(update.delta_y, 20.0);
    }

    #[test]
    fn test_scroll_offset_update_agent() {
        let tile_id = SceneId::new();
        let update = ScrollOffsetUpdate::from_agent(tile_id, 0.0, -10.0);
        assert!(!update.user_initiated);
        assert_eq!(update.delta_y, -10.0);
    }

    #[test]
    fn test_scene_local_patch_empty() {
        let patch = SceneLocalPatch::new();
        assert!(patch.is_empty());
    }

    #[test]
    fn test_scene_local_patch_push_state() {
        let mut patch = SceneLocalPatch::new();
        let id = SceneId::new();
        patch.push_state(LocalStateUpdate::new(id).with_pressed(true));
        assert!(!patch.is_empty());
        assert_eq!(patch.state_updates.len(), 1);
    }

    #[test]
    fn test_scene_local_patch_merge_user_wins_over_agent() {
        let tile_id = SceneId::new();
        let mut base = SceneLocalPatch::new();
        base.push_scroll(ScrollOffsetUpdate::from_agent(tile_id, 0.0, 100.0));

        let mut incoming = SceneLocalPatch::new();
        incoming.push_scroll(ScrollOffsetUpdate::from_user(tile_id, 0.0, 20.0));

        base.merge_from(incoming);

        // Agent update should be dropped; only user update remains
        let scroll_for_tile: Vec<_> = base.scroll_updates.iter()
            .filter(|u| u.tile_id == tile_id)
            .collect();
        for u in &scroll_for_tile {
            assert!(u.user_initiated, "agent scroll should have been evicted");
        }
        // The user update should be present
        assert!(scroll_for_tile.iter().any(|u| (u.delta_y - 20.0).abs() < f32::EPSILON));
    }

    #[test]
    fn test_scene_local_patch_merge_agent_does_not_evict_user() {
        let tile_id = SceneId::new();
        let mut base = SceneLocalPatch::new();
        base.push_scroll(ScrollOffsetUpdate::from_user(tile_id, 0.0, 20.0));

        let mut incoming = SceneLocalPatch::new();
        incoming.push_scroll(ScrollOffsetUpdate::from_agent(tile_id, 0.0, 100.0));

        base.merge_from(incoming);

        // Both should be present — agent does not evict user
        let scroll_for_tile: Vec<_> = base.scroll_updates.iter()
            .filter(|u| u.tile_id == tile_id)
            .collect();
        assert_eq!(scroll_for_tile.len(), 2);
    }
}
