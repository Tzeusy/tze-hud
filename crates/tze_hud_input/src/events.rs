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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalStateUpdate {
    pub node_id: SceneId,
    pub pressed: Option<bool>,
    pub hovered: Option<bool>,
    pub focused: Option<bool>,
}

/// A per-tile scroll offset update, produced by Stage 2.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollOffsetUpdate {
    pub tile_id: SceneId,
    pub offset_x: f32,
    pub offset_y: f32,
}

/// Batch of local state changes produced during Stage 2 Local Feedback.
///
/// Forwarded to the compositor via a dedicated channel (separate from the
/// MutationBatch channel). Applied in Stage 4 before render encoding.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneLocalPatch {
    pub node_updates: Vec<LocalStateUpdate>,
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

    /// Add a node state update.
    pub fn update_node(
        &mut self,
        node_id: SceneId,
        pressed: Option<bool>,
        hovered: Option<bool>,
        focused: Option<bool>,
    ) {
        self.node_updates.push(LocalStateUpdate { node_id, pressed, hovered, focused });
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
