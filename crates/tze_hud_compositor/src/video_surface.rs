//! VideoSurface state machine — compositor media-plane render state (v2 preview).
//!
//! Implements the `ADMITTED → STREAMING → (DEGRADED | PAUSED) → CLOSING → CLOSED`
//! state machine per E26 (signoff-packet.md) using the `statig` crate.  The
//! terminal `REVOKED` state is also modelled.
//!
//! # B11 semantics (signoff-packet.md §B11)
//!
//! > **On media drop while session survives**: media surface shows last frame
//! > with disconnection badge, session continues, control path stays alive.
//!
//! The `Paused` state carries this semantic: the compositor renders the last
//! decoded frame (held in [`VideoSurfaceEntry`]) with a disconnection-badge
//! overlay.  The session (lease + control path) is unaffected — only the media
//! plane signals the drop.
//!
//! # Integration
//!
//! The compositor owns a `VideoSurfaceMap` keyed by the `SceneId` carried in
//! `ZoneContent::VideoSurfaceRef`.  The runtime updates the state machine via
//! [`VideoSurfaceEntry::handle`] on media-plane events.  The renderer queries
//! [`VideoSurfaceEntry::render_state`] each frame to decide what to draw.
//!
//! # Spec references
//!
//! * signoff-packet.md §E26 (state machine, `statig` crate mandate)
//! * signoff-packet.md §B11 (media-drop while session survives)
//! * engineering-bar.md §1 (testing standards)
//! * engineering-bar.md §2 (frame-timing budgets)
//!
//! # v2_preview gate
//!
//! The full state machine, [`VideoSurfaceEntry`], and [`VideoSurfaceMap`]
//! implementation are compiled only when the `v2_preview` feature is active
//! per F27 (v2 code lands behind the feature flag after v1.0.0 ships).
//!
//! Outside `v2_preview`, consumers see only the stub [`VideoSurfaceMap`] type
//! (an empty wrapper) and the [`VideoRenderState`] enum (which always returns
//! `Placeholder` from the fallback constructor).

use tze_hud_scene::types::SceneId;

// Bring `statig::blocking` items into scope for the state machine impl.
// `Outcome`, `Transition`, `Handled`, `Super`, `IntoStateMachineExt` etc.
// are re-exported from `statig::blocking::*`.
#[cfg(feature = "v2_preview")]
use statig::blocking::*;

// ─── Render state (always public — queried by the renderer) ──────────────────

/// What the compositor should draw for a `ZoneContent::VideoSurfaceRef` zone.
///
/// Returned by [`VideoSurfaceEntry::render_state`] (or the stub) each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoRenderState {
    /// No decoded frame available yet — render a dark placeholder quad.
    Placeholder,
    /// Streaming normally — render the latest decoded frame.
    Streaming,
    /// Media plane dropped while session lives (B11) — render the last frame
    /// with a disconnection-badge overlay.
    LastFrameWithBadge,
    /// Surface is closed or revoked — render a dark placeholder quad.
    Closed,
}

// ─── v2_preview-gated implementation ─────────────────────────────────────────

/// A decoded video frame held as raw RGBA8 bytes.
///
/// In a full implementation the compositor would upload these bytes to a
/// `wgpu::Texture` (analogous to [`crate::renderer::ImageTextureEntry`]).
/// This type carries the minimum needed to test B11 state semantics and the
/// frame-timing path without requiring a live GStreamer pipeline.
///
/// Full GStreamer → wgpu texture upload is a follow-up task.
#[cfg(feature = "v2_preview")]
#[derive(Clone, Debug)]
pub struct VideoFrame {
    /// RGBA8 pixel data (`width × height × 4` bytes).
    pub rgba: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Monotonic wall-clock timestamp of this frame in microseconds.
    pub presented_at_us: u64,
}

/// Events that drive the `VideoSurface` state machine.
///
/// Per E26: the state machine is defined in RFC 0014 (media plane wire
/// protocol); these events mirror the approximate state set recorded in the
/// signoff packet.
#[cfg(feature = "v2_preview")]
#[derive(Debug)]
pub enum MediaEvent {
    /// Media plane admitted and decoder starting — `ADMITTED → STREAMING`.
    Admitted,
    /// A decoded frame arrived.
    FrameDecoded(VideoFrame),
    /// Network/decode hiccup, below teardown threshold — `STREAMING → DEGRADED`.
    Degraded,
    /// Recovered from a degraded episode — `DEGRADED → STREAMING`.
    Recovered,
    /// Media dropped while session survives (B11) — `STREAMING | DEGRADED → PAUSED`.
    MediaDropped,
    /// Session reconnected media plane — `PAUSED → STREAMING`.
    MediaReconnected,
    /// Graceful teardown requested — `* → CLOSING`.
    Close,
    /// Operator or budget watchdog hard-revoke — `* → REVOKED` (terminal).
    Revoke,
}

/// Per-surface compositor state including the `statig` state machine.
///
/// Owns the `statig`-generated `InitializedStateMachine<VideoSurface>` and
/// the last decoded frame retained for B11 rendering.
#[cfg(feature = "v2_preview")]
pub struct VideoSurfaceEntry {
    /// The initialized `statig` blocking state machine for this surface.
    machine: statig::blocking::InitializedStateMachine<VideoSurface>,
    /// The last successfully decoded frame.  `None` until the first frame
    /// arrives.  Retained across `Paused` so the compositor can render it
    /// with a disconnection badge (B11).
    pub last_frame: Option<VideoFrame>,
}

#[cfg(feature = "v2_preview")]
impl VideoSurfaceEntry {
    /// Create a new entry in `Admitted` state (decoder starting).
    pub fn new() -> Self {
        let machine = VideoSurface.uninitialized_state_machine().init();
        Self {
            machine,
            last_frame: None,
        }
    }

    /// Deliver an event to the state machine.
    ///
    /// If the event carries a `FrameDecoded` payload, the frame is captured
    /// into `last_frame` before the state transition so it is available for
    /// rendering in subsequent states (including `Paused` per B11).
    pub fn handle(&mut self, event: &MediaEvent) {
        // Capture frame before mutating state.
        if let MediaEvent::FrameDecoded(frame) = event {
            self.last_frame = Some(frame.clone());
        }
        self.machine.handle(event);
    }

    /// What the renderer should draw this frame.
    ///
    /// Maps the current state machine state to a [`VideoRenderState`] variant
    /// that the render path can act on without knowing `statig` internals.
    pub fn render_state(&self) -> VideoRenderState {
        match self.machine.state() {
            State::Admitted {} => VideoRenderState::Placeholder,
            State::Streaming {} | State::Degraded {} => VideoRenderState::Streaming,
            State::Paused {} => VideoRenderState::LastFrameWithBadge,
            State::Closing {} | State::Closed {} | State::Revoked {} => VideoRenderState::Closed,
        }
    }

    /// Whether a disconnection badge overlay should be rendered this frame.
    ///
    /// Convenience wrapper over [`render_state`][Self::render_state].
    #[inline]
    pub fn needs_disconnection_badge(&self) -> bool {
        self.render_state() == VideoRenderState::LastFrameWithBadge
    }
}

#[cfg(feature = "v2_preview")]
impl Default for VideoSurfaceEntry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── statig state machine definition ─────────────────────────────────────────

/// State machine shared-storage (per-surface, currently empty).
///
/// The `statig` `#[state_machine]` attribute generates the `State` enum and
/// the dispatch infrastructure.  Shared-storage fields can be added here
/// later (e.g. `decode_drop_count: u32`) without changing the state trait
/// surface.
///
/// State derivations include `Debug` and `PartialEq` so tests can assert on
/// state values without needing to match variants manually.
#[cfg(feature = "v2_preview")]
#[derive(Default)]
pub struct VideoSurface;

#[cfg(feature = "v2_preview")]
#[statig::state_machine(initial = "State::admitted()", state(derive(Debug, PartialEq, Eq)))]
impl VideoSurface {
    /// ADMITTED — decoder admitted by the runtime, waiting for first frame.
    ///
    /// Valid transitions:
    /// - `Admitted` → `Streaming`   (runtime signals media-plane ready)
    /// - `Admitted` → `Closing`     (graceful close before any frame)
    /// - `Admitted` → `Revoked`     (operator hard-revoke during startup)
    #[state]
    fn admitted(&mut self, event: &MediaEvent) -> Outcome<State> {
        match event {
            MediaEvent::Admitted => Transition(State::streaming()),
            MediaEvent::Close => Transition(State::closing()),
            MediaEvent::Revoke => Transition(State::revoked()),
            _ => Super,
        }
    }

    /// STREAMING — live frames arriving normally.
    ///
    /// Valid transitions:
    /// - `Streaming` → `Degraded`   (decode hiccup, below teardown threshold)
    /// - `Streaming` → `Paused`     (media drop, session survives — B11)
    /// - `Streaming` → `Closing`    (graceful teardown)
    /// - `Streaming` → `Revoked`    (operator hard-revoke)
    ///
    /// `FrameDecoded` events are captured by `VideoSurfaceEntry::handle` before
    /// dispatch; the state machine handles them as no-ops.
    #[state]
    fn streaming(&mut self, event: &MediaEvent) -> Outcome<State> {
        match event {
            MediaEvent::Degraded => Transition(State::degraded()),
            MediaEvent::MediaDropped => Transition(State::paused()),
            MediaEvent::Close => Transition(State::closing()),
            MediaEvent::Revoke => Transition(State::revoked()),
            MediaEvent::FrameDecoded(_) => Handled,
            _ => Super,
        }
    }

    /// DEGRADED — transient quality issue; frames still arriving but
    /// sub-nominal.  Renders like `Streaming` (no badge).
    ///
    /// Valid transitions:
    /// - `Degraded` → `Streaming`   (quality recovered)
    /// - `Degraded` → `Paused`      (media drop while degraded — B11)
    /// - `Degraded` → `Closing`     (graceful teardown)
    /// - `Degraded` → `Revoked`     (operator hard-revoke)
    #[state]
    fn degraded(&mut self, event: &MediaEvent) -> Outcome<State> {
        match event {
            MediaEvent::Recovered => Transition(State::streaming()),
            MediaEvent::MediaDropped => Transition(State::paused()),
            MediaEvent::Close => Transition(State::closing()),
            MediaEvent::Revoke => Transition(State::revoked()),
            MediaEvent::FrameDecoded(_) => Handled,
            _ => Super,
        }
    }

    /// PAUSED — media plane dropped while session survives (B11).
    ///
    /// The compositor renders the **last decoded frame** (held in
    /// `VideoSurfaceEntry.last_frame`) with a disconnection-badge overlay.
    /// The session / control path is unaffected.
    ///
    /// Valid transitions:
    /// - `Paused` → `Streaming`     (media plane reconnected)
    /// - `Paused` → `Closing`       (graceful teardown)
    /// - `Paused` → `Revoked`       (operator hard-revoke)
    #[state]
    fn paused(&mut self, event: &MediaEvent) -> Outcome<State> {
        match event {
            MediaEvent::MediaReconnected | MediaEvent::Admitted => Transition(State::streaming()),
            MediaEvent::Close => Transition(State::closing()),
            MediaEvent::Revoke => Transition(State::revoked()),
            _ => Super,
        }
    }

    /// CLOSING — graceful teardown in progress.
    ///
    /// Valid transitions:
    /// - `Closing` → `Closed`       (teardown complete: second `Close` event)
    /// - `Closing` → `Revoked`      (operator hard-revoke overtakes graceful close)
    ///
    /// All other events are absorbed (teardown is in progress; no side-effects).
    #[state]
    fn closing(&mut self, event: &MediaEvent) -> Outcome<State> {
        match event {
            MediaEvent::Close => Transition(State::closed()),
            MediaEvent::Revoke => Transition(State::revoked()),
            _ => Handled,
        }
    }

    /// CLOSED — terminal.  No further transitions allowed.
    ///
    /// Any event delivered after `Closed` is silently absorbed.
    #[state]
    fn closed(&mut self, event: &MediaEvent) -> Outcome<State> {
        let _ = event;
        Handled
    }

    /// REVOKED — terminal hard-revoke by operator or budget watchdog.
    ///
    /// Any event delivered after `Revoked` is silently absorbed.
    #[state]
    fn revoked(&mut self, event: &MediaEvent) -> Outcome<State> {
        let _ = event;
        Handled
    }
}

// ─── VideoSurfaceMap (v2_preview) ────────────────────────────────────────────

/// Per-compositor map from `SceneId` → [`VideoSurfaceEntry`].
///
/// The compositor owns one `VideoSurfaceMap` for all live video surfaces.
/// Entries are created when a `ZoneContent::VideoSurfaceRef` zone is first
/// seen and removed when the surface transitions to a terminal state.
///
/// Outside `v2_preview`, this is a zero-cost empty type with the same public
/// interface so the renderer can call `render_state_for` unconditionally.
#[cfg(feature = "v2_preview")]
pub struct VideoSurfaceMap {
    entries: std::collections::HashMap<SceneId, VideoSurfaceEntry>,
}

#[cfg(feature = "v2_preview")]
impl VideoSurfaceMap {
    /// Create an empty map.
    pub fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
        }
    }

    /// Ensure an entry exists for `surface_id`, creating it in `Admitted`
    /// state if absent.
    pub fn ensure(&mut self, surface_id: SceneId) -> &mut VideoSurfaceEntry {
        self.entries.entry(surface_id).or_default()
    }

    /// Look up an existing entry without creating one.
    pub fn get(&self, surface_id: &SceneId) -> Option<&VideoSurfaceEntry> {
        self.entries.get(surface_id)
    }

    /// Deliver an event to the named surface.
    ///
    /// No-op if the surface is not tracked — the caller should call
    /// [`ensure`][Self::ensure] first to register a new surface.
    pub fn handle(&mut self, surface_id: SceneId, event: &MediaEvent) {
        if let Some(entry) = self.entries.get_mut(&surface_id) {
            entry.handle(event);
        }
    }

    /// Remove surfaces that have reached a terminal render state (`Closed`).
    ///
    /// Call once per frame before rendering to avoid accumulating dead entries.
    /// Revoked surfaces also render as `Closed` and are pruned here.
    pub fn prune_terminal(&mut self) {
        self.entries
            .retain(|_, entry| entry.render_state() != VideoRenderState::Closed);
    }

    /// Render state for a surface, defaulting to `Placeholder` if unknown.
    pub fn render_state_for(&self, surface_id: &SceneId) -> VideoRenderState {
        self.entries
            .get(surface_id)
            .map(|e| e.render_state())
            .unwrap_or(VideoRenderState::Placeholder)
    }
}

#[cfg(feature = "v2_preview")]
impl Default for VideoSurfaceMap {
    fn default() -> Self {
        Self::new()
    }
}

// ─── v1 fallback stub (no v2_preview) ────────────────────────────────────────

/// Fallback no-op `VideoSurfaceMap` for v1 builds (no `v2_preview` feature).
///
/// Provides the same public interface so the renderer can call
/// `render_state_for` unconditionally, returning `Placeholder` in v1.
#[cfg(not(feature = "v2_preview"))]
pub struct VideoSurfaceMap;

#[cfg(not(feature = "v2_preview"))]
impl VideoSurfaceMap {
    /// Create an empty (stub) map.
    pub fn new() -> Self {
        Self
    }

    /// Always returns `Placeholder` in v1 builds.
    pub fn render_state_for(&self, _surface_id: &SceneId) -> VideoRenderState {
        VideoRenderState::Placeholder
    }

    /// No-op in v1 builds.
    pub fn prune_terminal(&mut self) {}
}

#[cfg(not(feature = "v2_preview"))]
impl Default for VideoSurfaceMap {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── B11: media-drop-while-session-survives ────────────────────────────────

    /// B11 — On media drop while session survives: last frame + badge,
    /// session continues, control path alive.
    ///
    /// Scenario:
    ///   ADMITTED → STREAMING (first frame) → PAUSED (media drop)
    ///   ⟹ render_state = LastFrameWithBadge
    ///   ⟹ last_frame retained
    ///
    /// Per engineering-bar.md §1: invariant-based test (not point-value assert).
    #[test]
    #[cfg(feature = "v2_preview")]
    fn b11_media_drop_while_streaming_shows_last_frame_with_badge() {
        let mut entry = VideoSurfaceEntry::new();
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Placeholder,
            "initial state must be Placeholder (no frame yet)"
        );

        entry.handle(&MediaEvent::Admitted);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Streaming,
            "Admitted must transition to Streaming"
        );

        let frame = VideoFrame {
            rgba: vec![0xAB, 0xCD, 0xEF, 0xFF],
            width: 1,
            height: 1,
            presented_at_us: 1_000_000,
        };
        entry.handle(&MediaEvent::FrameDecoded(frame));
        assert_eq!(entry.render_state(), VideoRenderState::Streaming);
        assert!(
            entry.last_frame.is_some(),
            "last_frame must be set after FrameDecoded"
        );

        entry.handle(&MediaEvent::MediaDropped);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::LastFrameWithBadge,
            "B11: media drop must show LastFrameWithBadge"
        );
        assert!(
            entry.last_frame.is_some(),
            "B11: last_frame must be retained on media drop (session continues)"
        );
        assert!(
            entry.needs_disconnection_badge(),
            "B11: needs_disconnection_badge must be true in Paused state"
        );
    }

    /// B11 reconnect: after media drop, a reconnect clears the badge and
    /// resumes streaming.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn b11_reconnect_clears_badge_and_resumes_streaming() {
        let mut entry = VideoSurfaceEntry::new();
        entry.handle(&MediaEvent::Admitted);
        entry.handle(&MediaEvent::FrameDecoded(VideoFrame {
            rgba: vec![0u8; 4],
            width: 1,
            height: 1,
            presented_at_us: 1,
        }));
        entry.handle(&MediaEvent::MediaDropped);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::LastFrameWithBadge,
            "prerequisite: must be in badge state before reconnect"
        );

        entry.handle(&MediaEvent::MediaReconnected);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Streaming,
            "reconnect must resume Streaming and clear badge"
        );
        assert!(
            !entry.needs_disconnection_badge(),
            "reconnect must clear the disconnection badge"
        );
    }

    // ── Deterministic teardown ────────────────────────────────────────────────

    /// Graceful teardown: STREAMING → CLOSING → CLOSED (terminal).
    ///
    /// The teardown sequence is deterministic: a `Close` event moves to
    /// `Closing`, and a second `Close` completes the teardown to `Closed`.
    /// No race conditions or partial states.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn graceful_teardown_streaming_to_closed() {
        let mut entry = VideoSurfaceEntry::new();
        entry.handle(&MediaEvent::Admitted);
        assert_eq!(entry.render_state(), VideoRenderState::Streaming);

        entry.handle(&MediaEvent::Close);
        // First Close: Streaming → Closing (renders as Closed to protect callers).
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Closed,
            "Closing state must render as Closed (dark placeholder)"
        );

        // Second Close: Closing → Closed (terminal).
        entry.handle(&MediaEvent::Close);
        assert_eq!(entry.render_state(), VideoRenderState::Closed);

        // Terminal: further events are no-ops.
        entry.handle(&MediaEvent::Admitted);
        entry.handle(&MediaEvent::MediaReconnected);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Closed,
            "Closed is terminal — no further transitions"
        );
    }

    /// PAUSED → CLOSING → CLOSED: session tears down while badge is visible.
    ///
    /// Validates B11 teardown path: media is dropped (badge visible), then
    /// the session itself closes.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn teardown_from_paused_state_is_deterministic() {
        let mut entry = VideoSurfaceEntry::new();
        entry.handle(&MediaEvent::Admitted);
        entry.handle(&MediaEvent::MediaDropped);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::LastFrameWithBadge,
            "prerequisite: badge must be visible"
        );

        entry.handle(&MediaEvent::Close); // Paused → Closing
        entry.handle(&MediaEvent::Close); // Closing → Closed
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Closed,
            "PAUSED → CLOSING → CLOSED must be deterministic"
        );
    }

    /// Operator hard-revoke from any state transitions to Revoked (terminal).
    ///
    /// Per E25 degradation ladder: operator-manual revoke is always available.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn revoke_from_streaming_is_terminal() {
        let mut entry = VideoSurfaceEntry::new();
        entry.handle(&MediaEvent::Admitted);
        assert_eq!(entry.render_state(), VideoRenderState::Streaming);

        entry.handle(&MediaEvent::Revoke);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Closed,
            "Revoked renders as Closed (dark placeholder)"
        );

        // No further transitions — Revoked is terminal.
        entry.handle(&MediaEvent::MediaReconnected);
        entry.handle(&MediaEvent::Admitted);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Closed,
            "Revoked is terminal — no further transitions"
        );
    }

    /// Revoke from Paused state: badge visible, then hard revoke comes in.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn revoke_from_paused_is_terminal() {
        let mut entry = VideoSurfaceEntry::new();
        entry.handle(&MediaEvent::Admitted);
        entry.handle(&MediaEvent::MediaDropped);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::LastFrameWithBadge,
            "prerequisite: badge must be visible"
        );

        entry.handle(&MediaEvent::Revoke);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Closed,
            "Revoked from Paused must immediately render as Closed"
        );
    }

    // ── DEGRADED transitions ──────────────────────────────────────────────────

    /// STREAMING → DEGRADED → STREAMING (recovery): no badge during degradation.
    ///
    /// Validates that degradation does NOT trigger the disconnection badge —
    /// only `MediaDropped` (B11) does.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn degraded_renders_without_badge() {
        let mut entry = VideoSurfaceEntry::new();
        entry.handle(&MediaEvent::Admitted);
        entry.handle(&MediaEvent::Degraded);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Streaming,
            "DEGRADED renders as Streaming (no badge — not a media drop)"
        );
        assert!(
            !entry.needs_disconnection_badge(),
            "DEGRADED must not show disconnection badge"
        );

        entry.handle(&MediaEvent::Recovered);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Streaming,
            "Recovery from DEGRADED must resume Streaming"
        );
    }

    /// DEGRADED → PAUSED: media drop while degraded still triggers B11.
    ///
    /// A media drop is a media drop regardless of whether we were in DEGRADED
    /// or STREAMING.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn media_drop_while_degraded_triggers_b11() {
        let mut entry = VideoSurfaceEntry::new();
        entry.handle(&MediaEvent::Admitted);
        entry.handle(&MediaEvent::FrameDecoded(VideoFrame {
            rgba: vec![0u8; 4],
            width: 1,
            height: 1,
            presented_at_us: 2,
        }));
        entry.handle(&MediaEvent::Degraded);
        assert_eq!(entry.render_state(), VideoRenderState::Streaming);

        entry.handle(&MediaEvent::MediaDropped);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::LastFrameWithBadge,
            "DEGRADED → PAUSED must still show last frame + badge (B11)"
        );
        assert!(
            entry.last_frame.is_some(),
            "last_frame must be retained through degraded state"
        );
    }

    // ── VideoSurfaceMap ───────────────────────────────────────────────────────

    /// Map::ensure creates a new entry in Placeholder state.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn map_ensure_creates_placeholder_entry() {
        let mut map = VideoSurfaceMap::new();
        let id = SceneId::new();
        let entry = map.ensure(id);
        assert_eq!(
            entry.render_state(),
            VideoRenderState::Placeholder,
            "newly created entry must start in Placeholder (Admitted) state"
        );
    }

    /// Map::render_state_for returns Placeholder for unknown surface.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn map_render_state_unknown_surface_is_placeholder() {
        let map = VideoSurfaceMap::new();
        let id = SceneId::new();
        assert_eq!(
            map.render_state_for(&id),
            VideoRenderState::Placeholder,
            "unknown surface must return Placeholder"
        );
    }

    /// Map::handle delivers events to the correct entry.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn map_handle_delivers_event_to_entry() {
        let mut map = VideoSurfaceMap::new();
        let id = SceneId::new();
        map.ensure(id);
        map.handle(id, &MediaEvent::Admitted);
        assert_eq!(map.render_state_for(&id), VideoRenderState::Streaming);

        map.handle(id, &MediaEvent::MediaDropped);
        assert_eq!(
            map.render_state_for(&id),
            VideoRenderState::LastFrameWithBadge,
            "map must propagate MediaDropped → LastFrameWithBadge"
        );
    }

    /// Map::prune_terminal removes Closed entries.
    #[test]
    #[cfg(feature = "v2_preview")]
    fn map_prune_removes_closed_entries() {
        let mut map = VideoSurfaceMap::new();
        let id = SceneId::new();
        map.ensure(id);
        map.handle(id, &MediaEvent::Admitted);
        map.handle(id, &MediaEvent::Close); // → Closing
        map.handle(id, &MediaEvent::Close); // → Closed
        assert_eq!(
            map.render_state_for(&id),
            VideoRenderState::Closed,
            "prerequisite: entry must be Closed before pruning"
        );
        map.prune_terminal();
        // After pruning, the entry is gone → back to Placeholder.
        assert_eq!(
            map.render_state_for(&id),
            VideoRenderState::Placeholder,
            "pruned entry must return Placeholder (unknown surface)"
        );
    }

    // ── v1 fallback stub ──────────────────────────────────────────────────────

    /// Fallback stub always returns Placeholder in v1 builds.
    #[test]
    #[cfg(not(feature = "v2_preview"))]
    fn fallback_stub_returns_placeholder() {
        let map = VideoSurfaceMap::new();
        let id = SceneId::new();
        assert_eq!(
            map.render_state_for(&id),
            VideoRenderState::Placeholder,
            "v1 stub must always return Placeholder"
        );
    }
}
