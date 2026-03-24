//! Event structures for the dispatch pipeline.
//!
//! Implements:
//! - `HitTestResult`: outcome of hit-testing a display-space point
//! - `RouteTarget`: event routing decision from the event router
//! - `SceneLocalPatch`: local-state updates from Stage 2 (compositor-applied)
//! - `InputEnvelope` / `EventBatch`: wire container for agent delivery
//!   (spec §Requirement: Protobuf Schema for Input Events, lines 367-369)

use serde::{Deserialize, Serialize};
use tze_hud_scene::SceneId;

use crate::pointer::{
    ClickEvent, ContextMenuEvent, DoubleClickEvent, PointerCancelEvent, PointerDownEvent,
    PointerEnterEvent, PointerLeaveEvent, PointerMoveEvent, PointerUpEvent,
};

// ─── Hit-test result ─────────────────────────────────────────────────────────

/// The outcome of hitting a display-space point against the scene graph.
///
/// Hit-test traversal order per spec lines 263-265:
/// 1. Chrome layer first (always wins)
/// 2. Content tiles by z-order descending
/// 3. Within a tile, nodes in reverse tree order (last child first)
/// 4. First HitRegionNode whose bounds contain the point wins
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HitTestResult {
    /// A HitRegionNode was hit. Routes to that tile's lease owner.
    NodeHit {
        tile_id: SceneId,
        node_id: SceneId,
    },
    /// A tile was hit but no HitRegionNode was hit within it.
    /// Routes to the tile's lease owner with a null node_id.
    TileHit {
        tile_id: SceneId,
    },
    /// A chrome element was hit. Handled locally; no agent notification.
    ChromeHit {
        /// Opaque identifier for the chrome element (e.g., tab bar item ID).
        element_id: String,
    },
    /// No tile (or a passthrough tile) was hit.
    /// In overlay mode: passes through to the desktop.
    /// In fullscreen mode: discarded.
    Passthrough,
}

impl HitTestResult {
    /// Whether this hit requires agent notification.
    pub fn requires_agent_dispatch(&self) -> bool {
        matches!(self, HitTestResult::NodeHit { .. } | HitTestResult::TileHit { .. })
    }
}

// ─── Route target ─────────────────────────────────────────────────────────────

/// Resolved routing target for an event after hit-test.
///
/// Produced by the event router (spec §Requirement: Event Routing Resolution,
/// lines 315-317).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RouteTarget {
    /// Route to the lease owner of the given tile.
    Agent {
        /// Namespace (agent name) of the lease owner.
        namespace: String,
        /// The tile being addressed.
        tile_id: SceneId,
    },
    /// Chrome event; handled locally, no agent routing.
    ChromeLocal,
    /// Pass through to the desktop (overlay mode) or discard (fullscreen).
    Passthrough,
}

// ─── SceneLocalPatch ──────────────────────────────────────────────────────────

/// A per-node local state update, produced by Stage 2 (Local Feedback).
///
/// Spec §Requirement: Local Feedback Rendering via SceneLocalPatch (line 197).
/// Applied by the compositor in Stage 4 without lease validation or budget
/// checks.
///
/// The three runtime-owned boolean state bits:
/// - `pressed` — set on PointerDown, cleared on PointerUp (or rollback)
/// - `hovered` — set on PointerEnter, cleared on PointerLeave
/// - `focused` — set on focus acquisition, cleared on focus loss
///
/// `rollback=true` signals a 100ms reverse animation on agent explicit rejection
/// (spec §Local Feedback Rollback on Agent Rejection).
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
    /// (agent explicitly rejected the interaction).
    #[serde(default)]
    pub rollback: bool,
}

impl LocalStateUpdate {
    /// Construct a simple state update with no rollback.
    pub fn new(node_id: SceneId) -> Self {
        Self { node_id, pressed: None, hovered: None, focused: None, rollback: false }
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

/// A per-tile scroll offset update, produced by Stage 2.
///
/// Carries the **absolute** post-update scroll offset for the tile, per
/// spec §Local Feedback Rendering via SceneLocalPatch:
/// `ScrollOffsetUpdate(tile_id, offset_x, offset_y)`.
///
/// The `user_initiated` flag is used by `SceneLocalPatch::merge_from` to
/// enforce user-priority semantics when coalescing patches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollOffsetUpdate {
    /// The tile whose scroll offset changed.
    pub tile_id: SceneId,
    /// New absolute horizontal scroll offset (pixels from content origin).
    pub offset_x: f32,
    /// New absolute vertical scroll offset (pixels from content origin).
    pub offset_y: f32,
    /// Origin — `true` = user input, `false` = agent request.
    #[serde(default)]
    pub user_initiated: bool,
}

impl ScrollOffsetUpdate {
    /// Construct a user-initiated scroll offset update (absolute).
    pub fn from_user(tile_id: SceneId, offset_x: f32, offset_y: f32) -> Self {
        Self { tile_id, offset_x, offset_y, user_initiated: true }
    }

    /// Construct an agent-requested scroll offset update (absolute).
    pub fn from_agent(tile_id: SceneId, offset_x: f32, offset_y: f32) -> Self {
        Self { tile_id, offset_x, offset_y, user_initiated: false }
    }
}

/// Batch of local state changes produced during Stage 2 Local Feedback.
///
/// Forwarded to the compositor via a dedicated channel (separate from the
/// MutationBatch channel). Applied in Stage 4 before render encoding without
/// lease validation or budget checks.
///
/// ## Latency invariant
/// Must be produced within 1ms of the input event (combined Stage 1+2 budget).
/// The compositor must apply it before the next frame (< 33ms guarantee).
///
/// ## Channel semantics
/// The channel is bounded; if the compositor is behind, patches may be coalesced
/// via `merge_from`. Since local state is idempotent (last-write-wins),
/// coalescing is lossless.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneLocalPatch {
    /// Per-node state updates (pressed, hovered, focused).
    pub node_updates: Vec<LocalStateUpdate>,
    /// Per-tile scroll offset updates.
    pub scroll_updates: Vec<ScrollOffsetUpdate>,
}

impl SceneLocalPatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if this patch contains no changes.
    pub fn is_empty(&self) -> bool {
        self.node_updates.is_empty() && self.scroll_updates.is_empty()
    }

    /// Add a node state update (builder-friendly alias for push_state).
    pub fn push_state(&mut self, update: LocalStateUpdate) {
        self.node_updates.push(update);
    }

    /// Add a scroll offset update.
    pub fn push_scroll(&mut self, update: ScrollOffsetUpdate) {
        self.scroll_updates.push(update);
    }

    /// Add a node state update (convenience form).
    pub fn update_node(
        &mut self,
        node_id: SceneId,
        pressed: Option<bool>,
        hovered: Option<bool>,
        focused: Option<bool>,
    ) {
        self.node_updates.push(LocalStateUpdate { node_id, pressed, hovered, focused, rollback: false });
    }

    /// Merge another patch into this one (in-place coalescing).
    ///
    /// For state updates: the incoming update for a `node_id` replaces any
    /// existing entry for the same `node_id` (last-write-wins per node).
    ///
    /// For scroll updates: per-tile coalescing with user-priority semantics.
    /// Since offsets are absolute, same-origin updates follow last-write-wins:
    /// - Existing **agent** + incoming **user**: agent discarded, user wins.
    /// - Existing **user** + incoming **agent**: agent dropped.
    /// - Same origin: last-write-wins on absolute offsets.
    pub fn merge_from(&mut self, other: SceneLocalPatch) {
        // State updates: last-write-wins per node_id.
        for incoming in other.node_updates {
            self.node_updates.retain(|u| u.node_id != incoming.node_id);
            self.node_updates.push(incoming);
        }
        // Scroll updates: coalesce per tile_id with user-priority.
        for incoming in other.scroll_updates {
            if let Some(existing) = self.scroll_updates.iter_mut().find(|u| u.tile_id == incoming.tile_id) {
                match (existing.user_initiated, incoming.user_initiated) {
                    (false, true) => { *existing = incoming; }
                    (true, false) => {}
                    _ => { *existing = incoming; }
                }
            } else {
                self.scroll_updates.push(incoming);
            }
        }
    }
}

// ─── InputEnvelope ───────────────────────────────────────────────────────────

/// A single input event in its typed form.
///
/// This is the Rust-native equivalent of the 22-variant protobuf `InputEnvelope`
/// oneof defined in events.proto (spec lines 367-369). The v1-mandatory pointer
/// variants are present; v1-reserved and post-v1 variants are listed for schema
/// completeness but marked as placeholders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputEnvelope {
    // ── v1-mandatory pointer events ────────────────────────────────────────
    PointerDown(PointerDownEvent),
    PointerUp(PointerUpEvent),
    PointerMove(PointerMoveEvent),
    PointerEnter(PointerEnterEvent),
    PointerLeave(PointerLeaveEvent),
    Click(ClickEvent),
    DoubleClick(DoubleClickEvent),
    ContextMenu(ContextMenuEvent),
    PointerCancel(PointerCancelEvent),
}

impl InputEnvelope {
    /// Extract the timestamp_mono_us from whatever event variant is wrapped.
    pub fn timestamp_mono_us(&self) -> u64 {
        match self {
            InputEnvelope::PointerDown(e) => e.fields.timestamp_mono_us,
            InputEnvelope::PointerUp(e) => e.fields.timestamp_mono_us,
            InputEnvelope::PointerMove(e) => e.fields.timestamp_mono_us,
            InputEnvelope::PointerEnter(e) => e.fields.timestamp_mono_us,
            InputEnvelope::PointerLeave(e) => e.fields.timestamp_mono_us,
            InputEnvelope::Click(e) => e.fields.timestamp_mono_us,
            InputEnvelope::DoubleClick(e) => e.fields.timestamp_mono_us,
            InputEnvelope::ContextMenu(e) => e.fields.timestamp_mono_us,
            InputEnvelope::PointerCancel(e) => e.fields.timestamp_mono_us,
        }
    }

    /// Whether this is a transactional event that MUST NOT be coalesced or
    /// dropped (spec §Requirement: Event Coalescing Under Backpressure,
    /// line 342).
    pub fn is_transactional(&self) -> bool {
        matches!(
            self,
            InputEnvelope::PointerDown(_)
                | InputEnvelope::PointerUp(_)
                | InputEnvelope::Click(_)
                | InputEnvelope::DoubleClick(_)
                | InputEnvelope::ContextMenu(_)
                | InputEnvelope::PointerCancel(_)
        )
    }
}

// ─── EventBatch ──────────────────────────────────────────────────────────────

/// A batch of input events for a single agent, for a single frame.
///
/// Spec §Requirement: Event Serialization and Batching (line 331):
/// - Multiple events for the same agent within a single frame are batched.
/// - Events within a batch are ordered by `timestamp_mono_us` ascending.
/// - Delivered as a single SessionMessage (field 34) on the agent's gRPC stream.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EventBatch {
    /// Frame number from the compositor, for ordering and dedup.
    pub frame_number: u64,
    /// Batch creation timestamp in the monotonic domain (microseconds).
    pub batch_ts_us: u64,
    /// Events in this batch, ordered by `timestamp_mono_us` ascending.
    pub events: Vec<InputEnvelope>,
}

impl EventBatch {
    pub fn new(frame_number: u64, batch_ts_us: u64) -> Self {
        Self { frame_number, batch_ts_us, events: Vec::new() }
    }

    /// Add an event to the batch. Maintains ascending timestamp order.
    pub fn push(&mut self, event: InputEnvelope) {
        let ts = event.timestamp_mono_us();
        let pos = self.events.partition_point(|e| e.timestamp_mono_us() <= ts);
        self.events.insert(pos, event);
    }

    /// Whether this batch contains no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pointer::{Modifiers, PointerFields};

    fn make_fields(ts: u64) -> PointerFields {
        PointerFields {
            tile_id: SceneId::new(),
            node_id: SceneId::new(),
            interaction_id: "btn".to_string(),
            device_id: 1,
            local_x: 0.0,
            local_y: 0.0,
            display_x: 0.0,
            display_y: 0.0,
            modifiers: Modifiers::NONE,
            timestamp_mono_us: ts,
        }
    }

    #[test]
    fn hit_test_result_requires_dispatch() {
        assert!(HitTestResult::NodeHit { tile_id: SceneId::new(), node_id: SceneId::new() }
            .requires_agent_dispatch());
        assert!(HitTestResult::TileHit { tile_id: SceneId::new() }.requires_agent_dispatch());
        assert!(!HitTestResult::ChromeHit { element_id: "tab-bar".to_string() }
            .requires_agent_dispatch());
        assert!(!HitTestResult::Passthrough.requires_agent_dispatch());
    }

    #[test]
    fn scene_local_patch_empty_by_default() {
        let patch = SceneLocalPatch::new();
        assert!(patch.is_empty());
    }

    #[test]
    fn scene_local_patch_update_node() {
        let mut patch = SceneLocalPatch::new();
        let node_id = SceneId::new();
        patch.update_node(node_id, Some(true), None, None);
        assert!(!patch.is_empty());
        assert_eq!(patch.node_updates[0].node_id, node_id);
        assert_eq!(patch.node_updates[0].pressed, Some(true));
        assert_eq!(patch.node_updates[0].hovered, None);
    }

    #[test]
    fn event_batch_ordered_by_timestamp() {
        let mut batch = EventBatch::new(1, 1000);
        // Insert in reverse order
        batch.push(InputEnvelope::PointerMove(PointerMoveEvent {
            fields: make_fields(300),
        }));
        batch.push(InputEnvelope::PointerDown(PointerDownEvent {
            fields: make_fields(100),
            button: crate::pointer::PointerButton::Primary,
        }));
        batch.push(InputEnvelope::PointerUp(PointerUpEvent {
            fields: make_fields(200),
            button: crate::pointer::PointerButton::Primary,
        }));

        let timestamps: Vec<u64> = batch.events.iter().map(|e| e.timestamp_mono_us()).collect();
        assert_eq!(timestamps, vec![100, 200, 300], "events must be ordered by timestamp");
    }

    #[test]
    fn transactional_events_identified_correctly() {
        let down = InputEnvelope::PointerDown(PointerDownEvent {
            fields: make_fields(0),
            button: crate::pointer::PointerButton::Primary,
        });
        let move_evt = InputEnvelope::PointerMove(PointerMoveEvent { fields: make_fields(0) });
        assert!(down.is_transactional(), "PointerDown must be transactional");
        assert!(!move_evt.is_transactional(), "PointerMove must NOT be transactional");
    }

    #[test]
    fn context_menu_is_transactional() {
        let evt = InputEnvelope::ContextMenu(ContextMenuEvent { fields: make_fields(0) });
        assert!(evt.is_transactional(), "ContextMenuEvent must be transactional");
    }
}
