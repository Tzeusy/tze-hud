//! Textured draw commands, drag handle entries, UV fit-mode calculations,
//! and animation-state types used by the compositor render passes.
//!
//! Moved from `renderer.rs` banner 5 (`// ─── Image fit mode UV calculations ───`)
//! and the animation-state block that immediately follows it, by Step R-1 of
//! the renderer module split (hud-fgryk).  No logic was changed; only visibility
//! modifiers were added where Rust's module-privacy rules require them.
//!
//! Animation-state types (`ZoneAnimationState`, `PublicationAnimationState`,
//! `StreamRevealState`) are placed here rather than a separate `anim_types.rs`
//! because they follow immediately in the original file with no banner seam
//! between them, and the whole block (banner 5 + anim types) has no
//! `impl Compositor` dependencies — keeping the one-banner-per-PR invariant
//! while avoiding a file that would be <50 lines.

use tze_hud_scene::types::*;

// ─── Image fit mode UV calculations ─────────────────────────────────────────

/// A textured draw command collected during scene traversal.
///
/// These are collected separately from color quads because they use a
/// different vertex layout and render pipeline.
pub(super) struct TexturedDrawCmd {
    pub(super) resource_id: ResourceId,
    /// Pixel-space position and size of the destination rectangle.
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
    /// UV sub-rectangle within the texture: `[u_min, v_min, u_max, v_max]`.
    pub(super) uv_rect: [f32; 4],
    /// Per-vertex tint (opacity, fade, etc.).
    pub(super) tint: [f32; 4],
}

/// A draw command for a decoded video frame (v2 media plane, `v2_preview` only).
///
/// Collected by [`Compositor::collect_video_frame_cmds`] and consumed by
/// [`Compositor::encode_video_frame_pass`].  The bind group is looked up from
/// `video_frame_cache` keyed by `surface_id` at draw time.
///
/// Separate from [`TexturedDrawCmd`] because video surfaces are keyed by
/// [`tze_hud_scene::types::SceneId`] (not `ResourceId`) and come from a
/// different cache (`video_frame_cache` vs `image_texture_cache`).
#[cfg(feature = "v2_preview")]
pub(crate) struct VideoFrameDrawCmd {
    /// The `SceneId` of the `ZoneContent::VideoSurfaceRef` surface.
    pub(crate) surface_id: tze_hud_scene::types::SceneId,
    /// Pixel-space destination rectangle.
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
    /// Per-vertex tint (currently always opaque white — no fade animation
    /// on the video frame itself; badge is rendered by the chrome pass).
    pub(crate) tint: [f32; 4],
}

/// Collected runtime drag handle geometry + style for one visible element.
#[derive(Clone, Debug)]
pub(super) struct DragHandleEntry {
    pub(super) element_id: SceneId,
    pub(super) element_kind: DragHandleElementKind,
    /// Bounds of the drag handle affordance itself.
    pub(super) bounds: Rect,
    /// Full display-space bounds of the element being dragged.
    ///
    /// Used to emit the 2px highlight border during active drag.
    pub(super) element_bounds: Rect,
    pub(super) interaction_id: String,
    pub(super) style: DragHandleStyle,
}

/// Compute the UV rectangle and destination rectangle for a given fit mode.
///
/// Returns `(dest_x, dest_y, dest_w, dest_h, uv_rect)` where:
/// - `(dest_x, dest_y, dest_w, dest_h)` is the pixel-space quad to render
/// - `uv_rect` is `[u_min, v_min, u_max, v_max]` within the texture
///
/// All fit modes assume the texture contains the full image at `(img_w, img_h)`.
pub(super) fn compute_fit_mode(
    fit_mode: ImageFitMode,
    // Destination bounds in pixel space
    dest_x: f32,
    dest_y: f32,
    dest_w: f32,
    dest_h: f32,
    // Source image dimensions
    img_w: u32,
    img_h: u32,
) -> (f32, f32, f32, f32, [f32; 4]) {
    let iw = img_w as f32;
    let ih = img_h as f32;

    if iw <= 0.0 || ih <= 0.0 || dest_w <= 0.0 || dest_h <= 0.0 {
        return (dest_x, dest_y, dest_w, dest_h, [0.0, 0.0, 1.0, 1.0]);
    }

    let src_aspect = iw / ih;
    let dst_aspect = dest_w / dest_h;

    match fit_mode {
        ImageFitMode::Fill => {
            // Stretch to fill — full UV, full dest
            (dest_x, dest_y, dest_w, dest_h, [0.0, 0.0, 1.0, 1.0])
        }
        ImageFitMode::Contain => {
            // Scale uniformly so the entire image is visible; letterbox bars.
            let (rw, rh) = if src_aspect > dst_aspect {
                // Image is wider: fit width, letterbox top/bottom
                let rw = dest_w;
                let rh = dest_w / src_aspect;
                (rw, rh)
            } else {
                // Image is taller: fit height, letterbox left/right
                let rh = dest_h;
                let rw = dest_h * src_aspect;
                (rw, rh)
            };
            let rx = dest_x + (dest_w - rw) * 0.5;
            let ry = dest_y + (dest_h - rh) * 0.5;
            (rx, ry, rw, rh, [0.0, 0.0, 1.0, 1.0])
        }
        ImageFitMode::Cover => {
            // Scale uniformly to cover the entire dest; crop the excess via UV.
            let (u_min, v_min, u_max, v_max) = if src_aspect > dst_aspect {
                // Image is wider than dest: crop horizontal
                let visible_fraction = dst_aspect / src_aspect;
                let u_offset = (1.0 - visible_fraction) * 0.5;
                (u_offset, 0.0, u_offset + visible_fraction, 1.0)
            } else {
                // Image is taller than dest: crop vertical
                let visible_fraction = src_aspect / dst_aspect;
                let v_offset = (1.0 - visible_fraction) * 0.5;
                (0.0, v_offset, 1.0, v_offset + visible_fraction)
            };
            (dest_x, dest_y, dest_w, dest_h, [u_min, v_min, u_max, v_max])
        }
        ImageFitMode::ScaleDown => {
            // Like Contain but never scale up.
            if iw <= dest_w && ih <= dest_h {
                // Image fits at native size — center it.
                let rx = dest_x + (dest_w - iw) * 0.5;
                let ry = dest_y + (dest_h - ih) * 0.5;
                (rx, ry, iw, ih, [0.0, 0.0, 1.0, 1.0])
            } else {
                // Image is larger — use Contain logic.
                let (rw, rh) = if src_aspect > dst_aspect {
                    (dest_w, dest_w / src_aspect)
                } else {
                    (dest_h * src_aspect, dest_h)
                };
                let rx = dest_x + (dest_w - rw) * 0.5;
                let ry = dest_y + (dest_h - rh) * 0.5;
                (rx, ry, rw, rh, [0.0, 0.0, 1.0, 1.0])
            }
        }
    }
}

// ─── Animation-state types ───────────────────────────────────────────────────
//
// These types follow the image-fit-mode block in the original renderer.rs with
// no banner seam between them. They are included here (not in a separate
// anim_types.rs) because: (a) the whole block has no impl Compositor deps,
// (b) a dedicated file would be <60 lines, and (c) they will move together to
// animation.rs in step R-5 when the impl Compositor animation methods move.

/// Per-zone opacity animation state.
///
/// Tracks a fade-in or fade-out transition for a single zone.
/// When no transition is active, the zone is at full opacity (1.0).
///
/// Modeled after `WidgetAnimationState` in `crate::widget`.
pub struct ZoneAnimationState {
    /// Wall-clock time when the transition started.
    pub transition_start: std::time::Instant,
    /// Duration of the transition in milliseconds.
    pub duration_ms: u32,
    /// Opacity at the start of the transition.
    pub from_opacity: f32,
    /// Target opacity at the end of the transition (0.0 = fade-out, 1.0 = fade-in).
    pub target_opacity: f32,
}

impl ZoneAnimationState {
    /// Create a fade-in state (opacity 0 → 1) with the given duration.
    pub fn fade_in(duration_ms: u32) -> Self {
        Self {
            transition_start: std::time::Instant::now(),
            duration_ms,
            from_opacity: 0.0,
            target_opacity: 1.0,
        }
    }

    /// Create a fade-in state starting from `from_opacity` rather than 0.
    ///
    /// Used for **transition interrupt semantics**: when a new publish arrives
    /// during an active fade-out, the fade-out is cancelled and a fade-in begins
    /// from the current composite opacity (not from zero).  This prevents blank
    /// frames during rapid replacement.
    ///
    /// Per spec §Subtitle Contention Policy — Latest Wins, Transition interrupt
    /// semantics note: "the fade-out MUST be cancelled immediately and the new
    /// content MUST begin its transition_in_ms fade-in from the current composite
    /// opacity (not from zero)."
    pub fn fade_in_from(duration_ms: u32, from_opacity: f32) -> Self {
        Self {
            transition_start: std::time::Instant::now(),
            duration_ms,
            from_opacity: from_opacity.clamp(0.0, 1.0),
            target_opacity: 1.0,
        }
    }

    /// Create a fade-out state (opacity 1 → 0) with the given duration.
    pub fn fade_out(duration_ms: u32) -> Self {
        Self {
            transition_start: std::time::Instant::now(),
            duration_ms,
            from_opacity: 1.0,
            target_opacity: 0.0,
        }
    }

    /// Compute the current interpolated opacity.
    ///
    /// Returns `target_opacity` once the transition has elapsed.
    pub fn current_opacity(&self) -> f32 {
        if self.duration_ms == 0 {
            return self.target_opacity;
        }
        let elapsed_ms = self.transition_start.elapsed().as_millis() as f32;
        let t = (elapsed_ms / self.duration_ms as f32).clamp(0.0, 1.0);
        self.from_opacity + (self.target_opacity - self.from_opacity) * t
    }

    /// Returns `true` if the transition has fully completed.
    pub fn is_complete(&self) -> bool {
        self.transition_start.elapsed().as_millis() >= self.duration_ms as u128
    }
}

/// Stable key that uniquely identifies a single publication within a zone.
///
/// Composed of `(published_at_wall_us, publisher_namespace)`.  Because the
/// scene graph assigns `published_at_wall_us` from a monotonic wall clock, the
/// combination is unique per-zone across all practical usage.
pub(super) type PubKey = (u64, String);

/// Per-publication opacity animation state for TTL-based fade-out.
///
/// Unlike [`ZoneAnimationState`] (which tracks zone-wide fade-in/fade-out),
/// `PublicationAnimationState` tracks the lifecycle of **one** publication
/// within a Stack zone.  Each notification gets its own instance.
///
/// Lifecycle:
/// 1. Created when the compositor first sees the publication.  `fade_start` is
///    `None` (publication is fully visible at opacity 1.0).
/// 2. When `first_seen.elapsed() >= ttl_ms`, the compositor sets
///    `fade_start = Some(Instant::now())` to begin the 150 ms fade-out.
/// 3. While fading: `current_opacity()` interpolates from 1.0 → 0.0 over
///    `NOTIFICATION_FADE_OUT_MS` ms.
/// 4. When `is_fade_complete()` returns `true`, the publication is removed
///    from `SceneGraph::zone_registry::active_publishes`.
///
/// The effective `ttl_ms` is derived by [`Compositor::publication_ttl_ms`] with
/// this priority order:
/// - `ZonePublishRecord.expires_at_wall_us` (urgency-derived, highest priority)
/// - `NotificationPayload.ttl_ms`
/// - Zone `auto_clear_ms` / `NOTIFICATION_DEFAULT_TTL_MS` (fallback)
pub struct PublicationAnimationState {
    /// Wall-clock instant when the compositor first rendered this publication.
    pub first_seen: std::time::Instant,
    /// Effective TTL in milliseconds.  Fade-out begins once this many ms
    /// have elapsed since `first_seen`.
    pub ttl_ms: u64,
    /// Instant when the fade-out transition started.  `None` means the
    /// publication is still fully visible (TTL has not yet expired).
    pub fade_start: Option<std::time::Instant>,
    /// Fade-out duration in milliseconds (always 150 for notifications).
    pub fade_duration_ms: u32,
}

/// Duration of the per-notification fade-out transition (ms).
pub(super) const NOTIFICATION_FADE_OUT_MS: u32 = 150;

/// Default TTL used when no per-publication TTL is set and the zone has no
/// `auto_clear_ms`.  Matches the notification-area zone default (8 000 ms).
pub(super) const NOTIFICATION_DEFAULT_TTL_MS: u64 = 8_000;

impl PublicationAnimationState {
    /// Create a new state for a freshly-seen publication.
    pub fn new(ttl_ms: u64) -> Self {
        Self {
            first_seen: std::time::Instant::now(),
            ttl_ms,
            fade_start: None,
            fade_duration_ms: NOTIFICATION_FADE_OUT_MS,
        }
    }

    /// Check whether the TTL has expired and start the fade if so.
    ///
    /// Must be called once per frame per publication.  Idempotent after the
    /// fade has started.
    pub fn tick(&mut self) {
        if self.fade_start.is_none() && self.first_seen.elapsed().as_millis() as u64 >= self.ttl_ms
        {
            self.fade_start = Some(std::time::Instant::now());
        }
    }

    /// Returns the current effective opacity for this publication (0.0–1.0).
    ///
    /// Before fade: 1.0.
    /// During fade: linear interpolation from 1.0 → 0.0.
    /// After fade: 0.0.
    pub fn current_opacity(&self) -> f32 {
        let Some(start) = self.fade_start else {
            return 1.0;
        };
        if self.fade_duration_ms == 0 {
            return 0.0;
        }
        let elapsed_ms = start.elapsed().as_millis() as f32;
        let t = (elapsed_ms / self.fade_duration_ms as f32).clamp(0.0, 1.0);
        1.0 - t
    }

    /// Returns `true` when the fade-out transition has fully completed.
    pub fn is_fade_complete(&self) -> bool {
        let Some(start) = self.fade_start else {
            return false;
        };
        start.elapsed().as_millis() >= self.fade_duration_ms as u128
    }
}

/// How many frames to display each breakpoint segment before advancing.
///
/// At 60fps, 10 frames ≈ 167ms dwell per word — comfortably perceptible as
/// word-by-word reveal without feeling sluggish.
pub(super) const STREAM_REVEAL_FRAMES_PER_SEGMENT: u32 = 10;

/// Per-zone streaming reveal state.
///
/// Tracks progressive text reveal for `StreamText` publications that include
/// breakpoints.  Each frame, `advance()` is called; when the dwell counter
/// reaches [`STREAM_REVEAL_FRAMES_PER_SEGMENT`] the compositor moves to the
/// next breakpoint.
///
/// The `pub_key` ties this state to a specific publication.  When the
/// latest-wins publication changes, the old state is discarded and a new one
/// starts from breakpoint index 0.
pub struct StreamRevealState {
    /// The publication this state tracks.
    pub pub_key: PubKey,
    /// Byte-offset breakpoints copied from the publication record.
    /// Expected to be non-decreasing (callers should validate before constructing),
    /// but not enforced here — the compositor is safe regardless of order.
    pub breakpoints: Vec<usize>,
    /// Index into `breakpoints` of the currently-visible segment boundary.
    /// A value of `breakpoints.len()` means the full text is visible.
    pub segment_idx: usize,
    /// Frame counter within the current segment.
    pub frames_in_segment: u32,
}

impl StreamRevealState {
    /// Create reveal state for a new publication.
    ///
    /// Accepts `Vec<u64>` breakpoints (wire format from `ZonePublishRecord`) and
    /// converts to `Vec<usize>` for internal indexing arithmetic.
    pub fn new(pub_key: PubKey, breakpoints: Vec<u64>) -> Self {
        Self {
            pub_key,
            breakpoints: breakpoints.into_iter().map(|b| b as usize).collect(),
            segment_idx: 0,
            frames_in_segment: 0,
        }
    }

    /// Return the byte offset up to which text should be visible this frame.
    ///
    /// Returns `usize::MAX` (reveal all) when no breakpoints are set or all
    /// segments have been revealed.
    pub fn visible_byte_offset(&self) -> usize {
        if self.breakpoints.is_empty() || self.segment_idx >= self.breakpoints.len() {
            usize::MAX
        } else {
            self.breakpoints[self.segment_idx]
        }
    }

    /// Advance the reveal state by one frame.
    ///
    /// Returns `true` if the reveal is still in progress (more segments remain).
    pub fn advance(&mut self) -> bool {
        if self.segment_idx >= self.breakpoints.len() {
            return false; // already fully revealed
        }
        self.frames_in_segment += 1;
        if self.frames_in_segment >= STREAM_REVEAL_FRAMES_PER_SEGMENT {
            self.frames_in_segment = 0;
            self.segment_idx += 1;
        }
        self.segment_idx < self.breakpoints.len()
    }
}
