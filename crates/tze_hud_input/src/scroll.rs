//! Scroll local feedback — compositor-maintained, agent-free, < 4ms p99.
//!
//! Scroll is a local-first operation: the compositor maintains a scroll offset
//! per scrollable tile and updates it in the same frame the scroll event arrives,
//! without waiting for any agent response. Agents are notified asynchronously via
//! `ScrollOffsetChangedEvent` (non-transactional, coalesced).
//!
//! # Opt-in via ScrollConfig
//!
//! Tiles opt in to scroll behavior by attaching a `ScrollConfig`. A tile without
//! `ScrollConfig` ignores scroll events.
//!
//! # Priority rule
//!
//! If a user scroll event and an agent `SetScrollOffsetRequest` arrive in the same
//! frame, the **user scroll takes priority** and the agent request is
//! discarded. This is enforced in `ScrollTileState::commit_frame` and
//! `ScrollState::commit_all_frames`.
//!
//! # Latency invariant
//!
//! Scroll latency budget = input_to_local_ack p99 < 4ms — same as press state.
//! The scroll path executes entirely on the main thread with no locks or async.

use serde::{Deserialize, Serialize};
use tze_hud_scene::SceneId;

// ─── ScrollConfig ─────────────────────────────────────────────────────────────

/// Scroll behavior configuration for a scrollable tile.
///
/// Attached to a tile (keyed by tile_id in `ScrollState`). A tile without
/// `ScrollConfig` is not scrollable and scroll events pass through.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollConfig {
    /// Whether the tile scrolls horizontally.
    pub scrollable_x: bool,
    /// Whether the tile scrolls vertically.
    pub scrollable_y: bool,
    /// Content width in pixels (used to clamp scroll offset).
    /// `None` = no clamping in x.
    pub content_width: Option<f32>,
    /// Content height in pixels (used to clamp scroll offset).
    /// `None` = no clamping in y.
    pub content_height: Option<f32>,
}

impl ScrollConfig {
    /// Convenience constructor for a vertically-scrollable tile.
    pub fn vertical() -> Self {
        Self { scrollable_x: false, scrollable_y: true, content_width: None, content_height: None }
    }

    /// Convenience constructor for a horizontally-scrollable tile.
    pub fn horizontal() -> Self {
        Self { scrollable_x: true, scrollable_y: false, content_width: None, content_height: None }
    }

    /// Convenience constructor for a tile that scrolls in both directions.
    pub fn both() -> Self {
        Self { scrollable_x: true, scrollable_y: true, content_width: None, content_height: None }
    }
}

// ─── ScrollEvent ──────────────────────────────────────────────────────────────

/// A raw scroll input event from the OS.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScrollEvent {
    /// X position of the pointer when the scroll occurred (display-space).
    pub x: f32,
    /// Y position of the pointer when the scroll occurred (display-space).
    pub y: f32,
    /// Horizontal scroll delta (pixels, positive = scroll right).
    pub delta_x: f32,
    /// Vertical scroll delta (pixels, positive = scroll down).
    pub delta_y: f32,
}

// ─── SetScrollOffsetRequest ───────────────────────────────────────────────────

/// An agent request to programmatically set the scroll offset of a tile.
///
/// If a user scroll event and a `SetScrollOffsetRequest` arrive in the same
/// frame, the user scroll takes priority and this request is discarded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetScrollOffsetRequest {
    pub tile_id: SceneId,
    /// Absolute offset_x to set (pixels from content origin).
    pub offset_x: f32,
    /// Absolute offset_y to set (pixels from content origin).
    pub offset_y: f32,
}

// ─── ScrollOffsetChangedEvent ─────────────────────────────────────────────────

/// Async notification sent to agents after scroll offset changes.
///
/// This event is non-transactional and coalesced: if many scroll events arrive
/// between agent polling cycles, only the final offset is delivered.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScrollOffsetChangedEvent {
    pub tile_id: SceneId,
    /// Current absolute scroll offset_x (pixels from content origin).
    pub offset_x: f32,
    /// Current absolute scroll offset_y (pixels from content origin).
    pub offset_y: f32,
}

// ─── ScrollTileState ──────────────────────────────────────────────────────────

/// Current scroll state for a single tile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollTileState {
    /// Current horizontal scroll offset (pixels from content origin).
    pub offset_x: f32,
    /// Current vertical scroll offset (pixels from content origin).
    pub offset_y: f32,
    /// Scroll configuration (None = not scrollable).
    pub config: Option<ScrollConfig>,
    /// Pending agent `SetScrollOffsetRequest` for this tile, if any.
    /// Cleared each frame after applying (or discarding due to user scroll).
    pending_agent_request: Option<SetScrollOffsetRequest>,
    /// Whether a user scroll was received this frame.
    user_scroll_this_frame: bool,
    /// Whether the offset changed this frame (set by queue_user_scroll or commit_frame).
    dirty: bool,
}

impl ScrollTileState {
    pub fn new(config: ScrollConfig) -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            config: Some(config),
            pending_agent_request: None,
            user_scroll_this_frame: false,
            dirty: false,
        }
    }

    /// Queue a user scroll delta for this frame.
    pub fn queue_user_scroll(&mut self, delta_x: f32, delta_y: f32) {
        if let Some(config) = &self.config {
            if config.scrollable_x {
                self.offset_x += delta_x;
            }
            if config.scrollable_y {
                self.offset_y += delta_y;
            }
            self.user_scroll_this_frame = true;
            self.dirty = true;
            self.clamp_offsets();
        }
    }

    /// Queue an agent `SetScrollOffsetRequest`.
    ///
    /// Will be applied only if no user scroll event arrives this frame.
    pub fn queue_agent_request(&mut self, req: SetScrollOffsetRequest) {
        self.pending_agent_request = Some(req);
    }

    /// Commit the frame: apply pending agent request (if no user scroll this frame),
    /// then clear per-frame state. Returns `true` if the offset changed this frame.
    pub fn commit_frame(&mut self) -> bool {
        if !self.user_scroll_this_frame {
            // No user scroll — apply pending agent request if present
            if let Some(req) = self.pending_agent_request.take() {
                let before_x = self.offset_x;
                let before_y = self.offset_y;
                if let Some(config) = &self.config {
                    if config.scrollable_x {
                        self.offset_x = req.offset_x;
                    }
                    if config.scrollable_y {
                        self.offset_y = req.offset_y;
                    }
                    self.clamp_offsets();
                }
                if (self.offset_x - before_x).abs() > f32::EPSILON
                    || (self.offset_y - before_y).abs() > f32::EPSILON
                {
                    self.dirty = true;
                }
            }
        } else {
            // User scroll wins: discard pending agent request (spec: user takes priority)
            self.pending_agent_request = None;
        }

        let changed = self.dirty;
        self.user_scroll_this_frame = false;
        self.dirty = false;
        changed
    }

    /// Clamp offsets to [0, content_size] range.
    ///
    /// `content_width`/`content_height` in `ScrollConfig` represent the maximum
    /// scroll offset (i.e. the content boundary), not viewport-subtracted values.
    /// If viewport-aware clamping is needed in the future, `ScrollConfig` must
    /// carry viewport dimensions and this method updated accordingly.
    fn clamp_offsets(&mut self) {
        if let Some(config) = &self.config {
            self.offset_x = self.offset_x.max(0.0);
            self.offset_y = self.offset_y.max(0.0);
            if let Some(cw) = config.content_width {
                self.offset_x = self.offset_x.min(cw.max(0.0));
            }
            if let Some(ch) = config.content_height {
                self.offset_y = self.offset_y.min(ch.max(0.0));
            }
        }
    }

    /// Returns a `ScrollOffsetChangedEvent` for notifying agents.
    pub fn changed_event(&self, tile_id: SceneId) -> ScrollOffsetChangedEvent {
        ScrollOffsetChangedEvent {
            tile_id,
            offset_x: self.offset_x,
            offset_y: self.offset_y,
        }
    }
}

// ─── ScrollState ──────────────────────────────────────────────────────────────

/// Scroll state registry for all scrollable tiles.
///
/// Owned by the local scroll subsystem (compositor or input kernel). Scroll
/// events are applied here to update per-tile offsets; the caller is responsible
/// for encoding changed offsets as `ScrollOffsetUpdate` entries in the
/// `SceneLocalPatch` and for emitting `ScrollOffsetChangedEvent`s to agents.
#[derive(Default)]
pub struct ScrollState {
    tiles: std::collections::HashMap<SceneId, ScrollTileState>,
}

impl ScrollState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tile as scrollable with the given configuration.
    pub fn register_tile(&mut self, tile_id: SceneId, config: ScrollConfig) {
        self.tiles.insert(tile_id, ScrollTileState::new(config));
    }

    /// Unregister a tile (e.g. tile destroyed).
    pub fn unregister_tile(&mut self, tile_id: SceneId) {
        self.tiles.remove(&tile_id);
    }

    /// Returns true if a tile is registered as scrollable.
    pub fn is_scrollable(&self, tile_id: SceneId) -> bool {
        self.tiles.contains_key(&tile_id)
    }

    /// Process a user scroll event for a specific tile.
    ///
    /// Returns the scroll deltas actually applied (respecting `ScrollConfig`
    /// axis locks), or `None` if the tile is not scrollable.
    pub fn apply_user_scroll(
        &mut self,
        tile_id: SceneId,
        delta_x: f32,
        delta_y: f32,
    ) -> Option<(f32, f32)> {
        let state = self.tiles.get_mut(&tile_id)?;
        let before_x = state.offset_x;
        let before_y = state.offset_y;
        state.queue_user_scroll(delta_x, delta_y);
        Some((state.offset_x - before_x, state.offset_y - before_y))
    }

    /// Queue an agent `SetScrollOffsetRequest`.
    ///
    /// The request will be applied at `commit_frame` unless a user scroll
    /// arrives in the same frame.
    pub fn queue_agent_request(&mut self, req: SetScrollOffsetRequest) {
        if let Some(state) = self.tiles.get_mut(&req.tile_id) {
            state.queue_agent_request(req);
        }
    }

    /// Commit all pending frames and return a list of tile IDs whose offsets
    /// actually changed (to be included in `ScrollOffsetChangedEvent`s for agents).
    pub fn commit_all_frames(&mut self) -> Vec<SceneId> {
        self.tiles
            .iter_mut()
            .filter_map(|(tile_id, state)| {
                if state.commit_frame() { Some(*tile_id) } else { None }
            })
            .collect()
    }

    /// Get the current scroll offset for a tile, or `(0.0, 0.0)` if not found.
    pub fn offset(&self, tile_id: SceneId) -> (f32, f32) {
        self.tiles
            .get(&tile_id)
            .map(|s| (s.offset_x, s.offset_y))
            .unwrap_or((0.0, 0.0))
    }

    /// Get a `ScrollOffsetChangedEvent` for a tile (for agent notification).
    pub fn changed_event(&self, tile_id: SceneId) -> Option<ScrollOffsetChangedEvent> {
        self.tiles.get(&tile_id).map(|s| s.changed_event(tile_id))
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_vertical_updates_offset_y() {
        let mut state = ScrollTileState::new(ScrollConfig::vertical());
        state.queue_user_scroll(0.0, 30.0);
        assert!((state.offset_y - 30.0).abs() < f32::EPSILON);
        assert!((state.offset_x).abs() < f32::EPSILON); // x unchanged
    }

    #[test]
    fn test_scroll_horizontal_updates_offset_x() {
        let mut state = ScrollTileState::new(ScrollConfig::horizontal());
        state.queue_user_scroll(15.0, 0.0);
        assert!((state.offset_x - 15.0).abs() < f32::EPSILON);
        assert!((state.offset_y).abs() < f32::EPSILON);
    }

    #[test]
    fn test_scroll_axis_lock_ignores_locked_axis() {
        let mut state = ScrollTileState::new(ScrollConfig::vertical());
        state.queue_user_scroll(50.0, 20.0); // x should be ignored
        assert!((state.offset_x).abs() < f32::EPSILON);
        assert!((state.offset_y - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_user_scroll_takes_priority_over_agent_request() {
        let tile_id = SceneId::new();
        let mut state = ScrollTileState::new(ScrollConfig::vertical());

        // Queue agent request first
        state.queue_agent_request(SetScrollOffsetRequest {
            tile_id,
            offset_x: 0.0,
            offset_y: 999.0,
        });

        // Then a user scroll arrives in the same frame
        state.queue_user_scroll(0.0, 20.0);

        state.commit_frame();

        // User scroll (20.0) wins; agent request (999.0) discarded
        assert!((state.offset_y - 20.0).abs() < f32::EPSILON, "expected 20.0 got {}", state.offset_y);
    }

    #[test]
    fn test_agent_request_applied_when_no_user_scroll() {
        let tile_id = SceneId::new();
        let mut state = ScrollTileState::new(ScrollConfig::vertical());

        state.queue_agent_request(SetScrollOffsetRequest {
            tile_id,
            offset_x: 0.0,
            offset_y: 300.0,
        });

        state.commit_frame();

        // No user scroll — agent request applied
        assert!((state.offset_y - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_content_height_clamp() {
        let config = ScrollConfig {
            scrollable_y: true,
            scrollable_x: false,
            content_width: None,
            content_height: Some(200.0),
        };
        let mut state = ScrollTileState::new(config);
        state.queue_user_scroll(0.0, 999.0);
        assert!(state.offset_y <= 200.0, "offset_y {} should be clamped to 200", state.offset_y);
    }

    #[test]
    fn test_scroll_offset_no_negative() {
        let mut state = ScrollTileState::new(ScrollConfig::vertical());
        state.queue_user_scroll(0.0, -50.0); // negative delta → clamp to 0
        assert!(state.offset_y >= 0.0);
    }

    #[test]
    fn test_scroll_state_register_and_apply() {
        let tile_id = SceneId::new();
        let mut scroll = ScrollState::new();
        scroll.register_tile(tile_id, ScrollConfig::vertical());

        assert!(scroll.is_scrollable(tile_id));

        let delta = scroll.apply_user_scroll(tile_id, 0.0, 50.0);
        assert!(delta.is_some());
        let (dx, dy) = delta.unwrap();
        assert!((dx).abs() < f32::EPSILON);
        assert!((dy - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_scroll_state_non_scrollable_tile_returns_none() {
        let tile_id = SceneId::new();
        let mut scroll = ScrollState::new();
        // Not registered
        let delta = scroll.apply_user_scroll(tile_id, 0.0, 50.0);
        assert!(delta.is_none());
    }

    #[test]
    fn test_scroll_state_commit_reports_changed_tiles() {
        let tile_a = SceneId::new();
        let tile_b = SceneId::new();
        let mut scroll = ScrollState::new();
        scroll.register_tile(tile_a, ScrollConfig::vertical());
        scroll.register_tile(tile_b, ScrollConfig::horizontal());

        // Only scroll tile_a
        scroll.apply_user_scroll(tile_a, 0.0, 10.0);

        let changed = scroll.commit_all_frames();
        assert!(changed.contains(&tile_a), "tile_a should be changed");
        // tile_b did not scroll, so it should not be in changed (unless it had a pending agent req)
        // (tile_b had no scroll, so unchanged)
        assert!(!changed.contains(&tile_b), "tile_b was not scrolled");
    }

    #[test]
    fn test_scroll_state_user_wins_over_queued_agent_request() {
        let tile_id = SceneId::new();
        let mut scroll = ScrollState::new();
        scroll.register_tile(tile_id, ScrollConfig::vertical());

        scroll.queue_agent_request(SetScrollOffsetRequest {
            tile_id,
            offset_x: 0.0,
            offset_y: 999.0,
        });
        scroll.apply_user_scroll(tile_id, 0.0, 25.0);
        scroll.commit_all_frames();

        let (_, offset_y) = scroll.offset(tile_id);
        assert!((offset_y - 25.0).abs() < f32::EPSILON, "user scroll should win, got {offset_y}");
    }

    #[test]
    fn test_scroll_changed_event_contains_current_offset() {
        let tile_id = SceneId::new();
        let mut scroll = ScrollState::new();
        scroll.register_tile(tile_id, ScrollConfig::vertical());
        scroll.apply_user_scroll(tile_id, 0.0, 42.0);

        let event = scroll.changed_event(tile_id);
        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.tile_id, tile_id);
        assert!((event.offset_y - 42.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_input_to_local_ack_p99_within_budget_scroll() {
        use tze_hud_scene::calibration::{test_budget, budgets};
        use std::time::Instant;

        let tile_id = SceneId::new();
        let mut scroll = ScrollState::new();
        scroll.register_tile(tile_id, ScrollConfig::vertical());

        let start = Instant::now();
        scroll.apply_user_scroll(tile_id, 0.0, 30.0);
        let elapsed_us = start.elapsed().as_micros() as u64;

        let budget = test_budget(budgets::INPUT_ACK_BUDGET_US);
        assert!(
            elapsed_us < budget,
            "scroll local_ack_us={elapsed_us}us exceeded calibrated budget {budget}us",
        );
    }
}
