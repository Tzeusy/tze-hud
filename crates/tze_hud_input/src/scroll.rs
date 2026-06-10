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
        Self {
            scrollable_x: false,
            scrollable_y: true,
            content_width: None,
            content_height: None,
        }
    }

    /// Convenience constructor for a horizontally-scrollable tile.
    pub fn horizontal() -> Self {
        Self {
            scrollable_x: true,
            scrollable_y: false,
            content_width: None,
            content_height: None,
        }
    }

    /// Convenience constructor for a tile that scrolls in both directions.
    pub fn both() -> Self {
        Self {
            scrollable_x: true,
            scrollable_y: true,
            content_width: None,
            content_height: None,
        }
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

// ─── FollowTailAnchor ─────────────────────────────────────────────────────────

/// The scroll-anchor state for a streaming transcript tile.
///
/// This type encodes the spec task 3.2 / 3.3 contract for the follow-tail
/// scroll model:
///
/// - **`AtTail`** — the viewport is currently at the tail of the content.
///   When new content is appended, the scroll offset advances by exactly N
///   whole lines (spec task 3.2: "follow-tail advances by whole lines").
///
/// - **`ScrolledBack`** — the user has scrolled back from the tail.  When new
///   content is appended, the scroll offset is **not changed** (spec task 3.3:
///   "append does not disturb a scrolled-back viewport").
///
/// # Transition rules
///
/// | Event | Before | After |
/// |---|---|---|
/// | User scrolls down to tail | any | `AtTail` |
/// | User scrolls back (up) | `AtTail` | `ScrolledBack` |
/// | Content appended at tail | `AtTail` | `AtTail` (offset updated) |
/// | Content appended at tail | `ScrolledBack` | `ScrolledBack` (offset unchanged) |
/// | Tile registered / reset | — | `AtTail` (default: new tiles start at tail) |
///
/// # Usage
///
/// `FollowTailAnchor` is stored in [`ScrollTileState`] alongside the existing
/// offset fields.  It is updated by [`ScrollTileState::queue_user_scroll`] and
/// consumed by [`ScrollTileState::notify_content_appended`].
#[derive(Clone, Debug, Copy, PartialEq, Eq, Default)]
pub enum FollowTailAnchor {
    /// The viewport is at the tail of the content (default for new tiles).
    #[default]
    AtTail,
    /// The user has scrolled back from the tail.
    ScrolledBack,
}

/// Compute the follow-tail scroll offset for a tile when content is appended.
///
/// Given the previous and new `content_height`, `viewport_height`, and
/// `line_height`, returns the offset that keeps the viewport at the tail after
/// the append, advancing by **whole lines only**.
///
/// If the new content height does not add at least one full line, no change is
/// made (returns the current offset unchanged).  This ensures the "whole-line
/// advancement" invariant from spec task 3.2.
///
/// # Parameters
///
/// - `current_offset_y` — the current scroll offset (pixels from content origin).
/// - `old_content_height` — content height before the append (pixels).
/// - `new_content_height` — content height after the append (pixels).
/// - `viewport_height` — visible tile height (pixels).
/// - `line_height` — line height (pixels); used to quantise the advancement.
///
/// # Returns
///
/// The new `offset_y` value (may equal `current_offset_y` if no whole line was
/// added, or be clamped to `new_content_height - viewport_height` at the tail).
pub fn follow_tail_offset(
    current_offset_y: f32,
    old_content_height: f32,
    new_content_height: f32,
    viewport_height: f32,
    line_height: f32,
) -> f32 {
    if line_height <= 0.0 || viewport_height <= 0.0 {
        return current_offset_y;
    }

    // New lines added (as a count of whole lines).
    // Use a small tolerance (1/32 of a line) when rounding to defend against
    // floating-point representation errors: e.g. `5.0_f32 * 22.4_f32 = 112.0`
    // and `6.0_f32 * 22.4_f32 = 134.39999...`, so `delta = 22.39999...` which
    // would floor-divide to 0 without the tolerance bump.
    let delta_px = new_content_height - old_content_height;
    if delta_px < line_height * 0.5 {
        // Less than half a line was added — not yet a whole line, no advancement.
        return current_offset_y;
    }
    let tolerance = line_height / 32.0;
    let new_lines = ((delta_px + tolerance) / line_height).floor() as u32;
    if new_lines == 0 {
        return current_offset_y;
    }

    // Advance by exactly `new_lines` whole lines.
    let advanced = current_offset_y + new_lines as f32 * line_height;

    // Clamp to the tail (new_content_height − viewport_height).
    // A tile whose content is shorter than the viewport has no scrollable range;
    // the max meaningful offset is 0.
    let tail_offset = (new_content_height - viewport_height).max(0.0);
    advanced.min(tail_offset)
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
    /// Follow-tail anchor state for streaming transcript tiles.
    ///
    /// Defaults to `AtTail` for all new tiles: a freshly created tile starts
    /// with the viewport at the tail of the content.  Transitions to
    /// `ScrolledBack` when the user scrolls up, and back to `AtTail` when the
    /// user scrolls to the tail again.
    pub follow_tail: FollowTailAnchor,
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
            follow_tail: FollowTailAnchor::AtTail,
        }
    }

    /// Queue a user scroll delta for this frame.
    ///
    /// Updates the follow-tail anchor: a positive y-delta (scroll down) that
    /// brings the viewport to the tail transitions the anchor back to `AtTail`;
    /// any upward scroll transitions it to `ScrolledBack`.
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
            // Update follow-tail anchor after clamping so we can compare against
            // the tail boundary.
            self.update_follow_tail_anchor_after_user_scroll(delta_y);
        }
    }

    /// Update the follow-tail anchor after a user scroll gesture.
    ///
    /// If the user scrolled backward (negative delta_y = up), transition to
    /// `ScrolledBack`.  If they scrolled forward (positive delta_y = down) and
    /// are now at the tail, transition back to `AtTail`.
    fn update_follow_tail_anchor_after_user_scroll(&mut self, delta_y: f32) {
        if let Some(config) = &self.config {
            if delta_y < 0.0 {
                // Scrolled up — user moved away from tail.
                self.follow_tail = FollowTailAnchor::ScrolledBack;
            } else if delta_y > 0.0 {
                // Scrolled down — check if we reached the tail.
                let tail_offset = config.content_height.map(|ch| ch.max(0.0)).unwrap_or(0.0);
                // If there is no content_height bound (free scroll), or we are at
                // or beyond the tail offset, mark as AtTail.
                if config.content_height.is_none() || self.offset_y >= tail_offset {
                    self.follow_tail = FollowTailAnchor::AtTail;
                }
            }
        }
    }

    /// Notify this tile that new content has been appended (e.g. a streaming
    /// transcript received new lines).
    ///
    /// Implements spec task 3.2 ("follow-tail advances by whole lines") and
    /// task 3.3 ("append does not disturb a scrolled-back viewport"):
    ///
    /// - When `self.follow_tail == AtTail`, the scroll offset advances by
    ///   whole lines to track the new tail.
    /// - When `self.follow_tail == ScrolledBack`, the scroll offset is
    ///   unchanged; only `content_height` is updated.
    ///
    /// Also updates `config.content_height` to the new value so that future
    /// scroll clamping reflects the extended content.
    ///
    /// # Parameters
    ///
    /// - `new_content_height` — total content height (pixels) after the append.
    /// - `viewport_height` — visible tile height (pixels); needed for the
    ///   whole-line advancement calculation.
    /// - `line_height` — line height (pixels); used to quantise advancement.
    ///
    /// # Returns
    ///
    /// `true` if the scroll offset changed (dirty), `false` if the anchor was
    /// `ScrolledBack` and the offset was left unchanged.
    pub fn notify_content_appended(
        &mut self,
        new_content_height: f32,
        viewport_height: f32,
        line_height: f32,
    ) -> bool {
        let old_content_height = self
            .config
            .as_ref()
            .and_then(|c| c.content_height)
            .unwrap_or(0.0);

        // Update content_height in config regardless of anchor state so that
        // future scroll clamping uses the correct boundary.
        if let Some(config) = &mut self.config {
            config.content_height = Some(new_content_height);
        }

        match self.follow_tail {
            FollowTailAnchor::ScrolledBack => {
                // Task 3.3: do NOT disturb the scrolled-back viewport.
                false
            }
            FollowTailAnchor::AtTail => {
                // Task 3.2: advance by whole lines.
                let new_offset = follow_tail_offset(
                    self.offset_y,
                    old_content_height,
                    new_content_height,
                    viewport_height,
                    line_height,
                );
                if (new_offset - self.offset_y).abs() > f32::EPSILON {
                    self.offset_y = new_offset;
                    self.dirty = true;
                    true
                } else {
                    false
                }
            }
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
    pending_unregistered_requests: std::collections::HashMap<SceneId, SetScrollOffsetRequest>,
}

impl ScrollState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tile as scrollable with the given configuration.
    pub fn register_tile(&mut self, tile_id: SceneId, config: ScrollConfig) {
        let mut state = ScrollTileState::new(config);
        if let Some(req) = self.pending_unregistered_requests.remove(&tile_id) {
            state.queue_agent_request(req);
        }
        self.tiles.insert(tile_id, state);
    }

    /// Unregister a tile (e.g. tile destroyed).
    pub fn unregister_tile(&mut self, tile_id: SceneId) {
        self.tiles.remove(&tile_id);
        self.pending_unregistered_requests.remove(&tile_id);
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
        } else {
            self.pending_unregistered_requests.insert(req.tile_id, req);
        }
    }

    /// Commit a single tile frame and report if its offset changed.
    pub fn commit_tile_frame(&mut self, tile_id: SceneId) -> bool {
        self.tiles
            .get_mut(&tile_id)
            .map(ScrollTileState::commit_frame)
            .unwrap_or(false)
    }

    /// Commit all pending frames and return a list of tile IDs whose offsets
    /// actually changed (to be included in `ScrollOffsetChangedEvent`s for agents).
    pub fn commit_all_frames(&mut self) -> Vec<SceneId> {
        self.tiles
            .iter_mut()
            .filter_map(|(tile_id, state)| {
                if state.commit_frame() {
                    Some(*tile_id)
                } else {
                    None
                }
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

    /// Notify a tile that content has been appended (e.g. new streaming lines).
    ///
    /// Implements spec task 3.2 / 3.3 at the registry level:
    /// - `AtTail` tiles advance their scroll offset by whole lines.
    /// - `ScrolledBack` tiles have their offset left unchanged.
    ///
    /// Returns `true` if the offset actually changed, `false` otherwise.
    /// No-op if the tile is not registered.
    pub fn notify_content_appended(
        &mut self,
        tile_id: SceneId,
        new_content_height: f32,
        viewport_height: f32,
        line_height: f32,
    ) -> bool {
        self.tiles
            .get_mut(&tile_id)
            .map(|s| s.notify_content_appended(new_content_height, viewport_height, line_height))
            .unwrap_or(false)
    }

    /// Return the current follow-tail anchor state for a tile.
    ///
    /// Returns `AtTail` (the default) if the tile is not registered.
    pub fn follow_tail_anchor(&self, tile_id: SceneId) -> FollowTailAnchor {
        self.tiles
            .get(&tile_id)
            .map(|s| s.follow_tail)
            .unwrap_or(FollowTailAnchor::AtTail)
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
        assert!(
            (state.offset_y - 20.0).abs() < f32::EPSILON,
            "expected 20.0 got {}",
            state.offset_y
        );
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
        assert!(
            state.offset_y <= 200.0,
            "offset_y {} should be clamped to 200",
            state.offset_y
        );
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
        assert!(
            (offset_y - 25.0).abs() < f32::EPSILON,
            "user scroll should win, got {offset_y}"
        );
    }

    #[test]
    fn test_queue_request_before_registration_is_applied_after_register() {
        let tile_id = SceneId::new();
        let mut scroll = ScrollState::new();

        scroll.queue_agent_request(SetScrollOffsetRequest {
            tile_id,
            offset_x: 0.0,
            offset_y: 120.0,
        });

        scroll.register_tile(tile_id, ScrollConfig::vertical());
        let changed = scroll.commit_all_frames();
        assert!(
            changed.contains(&tile_id),
            "tile with queued request should be reported as changed"
        );
        let (_, offset_y) = scroll.offset(tile_id);
        assert!(
            (offset_y - 120.0).abs() < f32::EPSILON,
            "queued request should apply after registration, got {offset_y}"
        );
    }

    #[test]
    fn test_commit_tile_frame_does_not_consume_other_tile_updates() {
        let tile_a = SceneId::new();
        let tile_b = SceneId::new();
        let mut scroll = ScrollState::new();
        scroll.register_tile(tile_a, ScrollConfig::vertical());
        scroll.register_tile(tile_b, ScrollConfig::vertical());
        scroll.queue_agent_request(SetScrollOffsetRequest {
            tile_id: tile_b,
            offset_x: 0.0,
            offset_y: 55.0,
        });

        assert!(!scroll.commit_tile_frame(tile_a));

        let changed = scroll.commit_all_frames();
        assert!(
            changed.contains(&tile_b),
            "tile_b update must remain pending until its frame is committed"
        );
        let (_, offset_y) = scroll.offset(tile_b);
        assert!((offset_y - 55.0).abs() < f32::EPSILON);
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
        use std::time::Instant;
        use tze_hud_scene::calibration::{budgets, test_budget};

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

    // ── Spec task 3.2 — follow-tail advances by whole lines ──────────────────

    /// A tile starting at the tail should advance its offset by exactly N whole
    /// lines when content is appended, never by a fractional line.
    ///
    /// Spec task 3.2: "follow-tail advances by whole lines"
    #[test]
    fn follow_tail_advances_by_whole_lines_on_append() {
        let tile_id = SceneId::new();
        let line_h = 20.0_f32;
        let viewport_h = 100.0_f32; // 5 visible lines
        let mut scroll = ScrollState::new();

        // Register with content_height = 5 lines initially (viewport is full).
        let config = ScrollConfig {
            scrollable_y: true,
            scrollable_x: false,
            content_width: None,
            content_height: Some(5.0 * line_h), // 100px
        };
        scroll.register_tile(tile_id, config);

        // Tile starts at AtTail with offset_y = 0 (content fits in viewport).
        assert_eq!(scroll.follow_tail_anchor(tile_id), FollowTailAnchor::AtTail);

        // Append 3 more lines: content grows from 100px to 160px.
        let new_content_height = 8.0 * line_h; // 160px
        let changed =
            scroll.notify_content_appended(tile_id, new_content_height, viewport_h, line_h);

        assert!(
            changed,
            "offset should have changed when at tail and content grew"
        );

        let (_, offset_y) = scroll.offset(tile_id);
        // Expected new offset: 160 - 100 = 60px (exactly 3 line-heights).
        // The viewport shows lines 3-7 (0-indexed), bottom-aligned to content end.
        assert!(
            (offset_y - 60.0).abs() < f32::EPSILON,
            "follow-tail should advance to offset 60.0 (3 new lines × 20px); got {offset_y}"
        );

        // Advancement is always a whole multiple of line_height.
        assert_eq!(
            (offset_y / line_h).fract(),
            0.0,
            "offset_y must be a whole multiple of line_height; got {offset_y}"
        );

        // Tile remains at tail anchor after content append.
        assert_eq!(
            scroll.follow_tail_anchor(tile_id),
            FollowTailAnchor::AtTail,
            "anchor must remain AtTail after follow-tail advancement"
        );
    }

    /// Single-line append from a follow-tail position advances by exactly one
    /// whole line.
    #[test]
    fn follow_tail_single_line_append_advances_exactly_one_line() {
        let tile_id = SceneId::new();
        let line_h = 22.4_f32;
        let viewport_h = 5.0 * line_h;
        let mut scroll = ScrollState::new();

        // Start: content exactly fills 5 lines; offset = 0 (no scrollable range).
        let config = ScrollConfig {
            scrollable_y: true,
            scrollable_x: false,
            content_width: None,
            content_height: Some(5.0 * line_h),
        };
        scroll.register_tile(tile_id, config);

        // Append exactly 1 line.
        let new_content = 6.0 * line_h;
        scroll.notify_content_appended(tile_id, new_content, viewport_h, line_h);

        let (_, offset_y) = scroll.offset(tile_id);
        // offset_y should be 1 × line_h to keep the 6th line visible.
        let expected = line_h; // one line height = 22.4px
        assert!(
            (offset_y - expected).abs() < 0.01,
            "single-line append must advance by exactly 1 × line_height ({expected:.2}px); \
             got {offset_y:.2}"
        );
    }

    // ── Spec task 3.3 — append does not disturb a scrolled-back viewport ─────

    /// When the user has scrolled back from the tail, appending new content
    /// must NOT change the scroll offset.
    ///
    /// Spec task 3.3: "append stability for scrolled-back viewports"
    #[test]
    fn scrolled_back_append_does_not_disturb_viewport() {
        let tile_id = SceneId::new();
        let line_h = 20.0_f32;
        let viewport_h = 100.0_f32; // 5 visible lines
        let mut scroll = ScrollState::new();

        // Content: 20 lines = 400px.
        let config = ScrollConfig {
            scrollable_y: true,
            scrollable_x: false,
            content_width: None,
            content_height: Some(20.0 * line_h),
        };
        scroll.register_tile(tile_id, config);

        // Move viewport to the tail first (scroll down to end).
        scroll.apply_user_scroll(tile_id, 0.0, 300.0); // scroll to offset 300 (lines 15–20)
        assert_eq!(
            scroll.follow_tail_anchor(tile_id),
            FollowTailAnchor::AtTail,
            "after scrolling to tail offset should be AtTail"
        );

        // Now scroll back up.
        scroll.apply_user_scroll(tile_id, 0.0, -120.0); // back up 6 lines to offset 180
        assert_eq!(
            scroll.follow_tail_anchor(tile_id),
            FollowTailAnchor::ScrolledBack,
            "after scrolling up the anchor must be ScrolledBack"
        );
        let (_, offset_before) = scroll.offset(tile_id);

        // Append 5 more lines.
        let new_content = 25.0 * line_h;
        let changed = scroll.notify_content_appended(tile_id, new_content, viewport_h, line_h);

        assert!(
            !changed,
            "append must not dirty the offset when ScrolledBack"
        );

        let (_, offset_after) = scroll.offset(tile_id);
        assert!(
            (offset_before - offset_after).abs() < f32::EPSILON,
            "scrolled-back append must not change offset_y; before={offset_before} after={offset_after}"
        );

        // Anchor remains ScrolledBack.
        assert_eq!(
            scroll.follow_tail_anchor(tile_id),
            FollowTailAnchor::ScrolledBack,
            "anchor must remain ScrolledBack after content append"
        );
    }

    /// After scrolling back and then scrolling back to the tail, the anchor
    /// transitions back to AtTail and follow-tail behaviour resumes.
    #[test]
    fn scrolled_back_then_scroll_to_tail_resumes_follow_tail() {
        let tile_id = SceneId::new();
        let line_h = 20.0_f32;
        let content_h = 20.0 * line_h; // 400px
        let mut scroll = ScrollState::new();

        let config = ScrollConfig {
            scrollable_y: true,
            scrollable_x: false,
            content_width: None,
            content_height: Some(content_h),
        };
        scroll.register_tile(tile_id, config);

        // Scroll to tail.
        scroll.apply_user_scroll(tile_id, 0.0, content_h);
        assert_eq!(scroll.follow_tail_anchor(tile_id), FollowTailAnchor::AtTail);

        // Scroll back.
        scroll.apply_user_scroll(tile_id, 0.0, -60.0);
        assert_eq!(
            scroll.follow_tail_anchor(tile_id),
            FollowTailAnchor::ScrolledBack
        );

        // Scroll back to tail.
        scroll.apply_user_scroll(tile_id, 0.0, 300.0); // forward past the tail
        assert_eq!(
            scroll.follow_tail_anchor(tile_id),
            FollowTailAnchor::AtTail,
            "scrolling back to tail must restore AtTail anchor"
        );
    }

    // ── follow_tail_offset unit tests ─────────────────────────────────────────

    #[test]
    fn follow_tail_offset_zero_delta_returns_unchanged() {
        // No new content: no advancement.
        let result = follow_tail_offset(50.0, 200.0, 200.0, 100.0, 20.0);
        assert!((result - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn follow_tail_offset_one_line_advance() {
        // 1 new line (20px) added; advance by exactly 20px.
        let result = follow_tail_offset(0.0, 100.0, 120.0, 100.0, 20.0);
        assert!(
            (result - 20.0).abs() < f32::EPSILON,
            "expected 20.0 got {result}"
        );
    }

    #[test]
    fn follow_tail_offset_clamped_to_tail() {
        // Many lines added but we clamp to (new_content - viewport).
        let result = follow_tail_offset(0.0, 100.0, 500.0, 100.0, 20.0);
        // tail = 500 - 100 = 400; advanced = 0 + (500-100)/20*20 = 400; min(400, 400) = 400.
        assert!(
            (result - 400.0).abs() < f32::EPSILON,
            "expected 400.0 got {result}"
        );
    }

    #[test]
    fn follow_tail_offset_fractional_line_below_threshold_unchanged() {
        // 9px added; line_height = 20px; 9 < 10 (0.5 * 20) => no advancement.
        let result = follow_tail_offset(0.0, 100.0, 109.0, 100.0, 20.0);
        assert!(
            (result - 0.0).abs() < f32::EPSILON,
            "expected 0.0 got {result}"
        );
    }

    #[test]
    fn follow_tail_offset_zero_line_height_returns_unchanged() {
        let result = follow_tail_offset(50.0, 100.0, 200.0, 100.0, 0.0);
        assert!((result - 50.0).abs() < f32::EPSILON);
    }
}
