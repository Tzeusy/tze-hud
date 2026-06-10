//! Portal window management — resize affordances, focus-scoped hotkeys,
//! min/max clamping, and coalescible geometry snapshots.
//!
//! Implements §6b of `text-stream-portal-phase1/tasks.md` (amendment 2026-06-10):
//!
//! - §6b.1 Pointer-driven resize affordances on the portal frame (corner/edge
//!   capture regions, content layer) with local-first geometry feedback.
//! - §6b.2 Focus-scoped resize hotkeys (Ctrl+`+`/`=` grow, Ctrl+`-` shrink)
//!   with token-defined step; unfocused portals never consume them.
//! - §6b.3 Min/max clamping: legibility minimum from tokens; lease-bounds /
//!   scene-budget maximum passed by caller; no partially-clipped glyphs at any
//!   intermediate geometry (the overflow contract is not relaxed during resize).
//! - §6b.4 Coalescible state-stream geometry snapshots delivered to the
//!   owning adapter; gesture is authoritative over adapter publishes until
//!   gesture end.
//!
//! ## Invariants
//!
//! - Gesture authority: during an active resize gesture the adapter's
//!   `publish_geometry` requests MUST be dropped. Only `GeometrySnapshot`
//!   events emitted by this module are applied. This is enforced by the caller
//!   checking `PortalResizeState::gesture_active()`.
//! - Local feedback first: geometry updates happen immediately in the same
//!   frame as the pointer event — no adapter roundtrip.
//! - Token-defined bounds: `min_width_px`, `min_height_px`, and
//!   `resize_step_px` come exclusively from `PortalWindowTokens`, never from
//!   inline constants.
//! - Coalescible snapshots: `GeometrySnapshot` is a state-stream payload.
//!   The transport MAY deliver only the latest snapshot per adapter delivery
//!   window (latest-wins).
//!
//! ## Performance contract
//!
//! Each state update runs in O(1) with no allocations on the hot path.
//! Per the engineering bar: gesture recognizer update < 50 µs per event;
//! input to local ack < 4 ms.

use serde::{Deserialize, Serialize};

// ─── Token-resolved window geometry bounds ────────────────────────────────────

/// Token-resolved window geometry bounds for a portal.
///
/// Constructed from the portal token map at startup (or on hot-reload).
/// All numeric fields are already parsed — callers use these values directly.
///
/// The **max** bounds come from the lease/scene budget at call time, not from
/// tokens, and are passed into clamping helpers as arguments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortalWindowTokens {
    /// Legibility minimum width in pixels (§6b.3).
    pub min_width_px: f32,
    /// Legibility minimum height in pixels (§6b.3).
    pub min_height_px: f32,
    /// Pixels per Ctrl+`+`/`=`/`-` hotkey step (§6b.2).
    pub resize_step_px: f32,
    /// Width/height of the pointer capture region on frame edges/corners (§6b.1).
    pub affordance_px: f32,
}

impl Default for PortalWindowTokens {
    fn default() -> Self {
        // These defaults must match `portal_tokens::defaults::WINDOW_*` in
        // tze_hud_config. There is no compile-time link (the crates are
        // intentionally independent); update both when changing a default.
        Self {
            min_width_px: 240.0,
            min_height_px: 160.0,
            resize_step_px: 32.0,
            affordance_px: 8.0,
        }
    }
}

// ─── Resize edge / corner ─────────────────────────────────────────────────────

/// Which edge or corner of the portal frame is being resized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResizeEdge {
    /// Left edge — dragging changes x and width (width ↑ when pointer moves left).
    Left,
    /// Right edge — dragging changes width only.
    Right,
    /// Top edge — dragging changes y and height.
    Top,
    /// Bottom edge — dragging changes height only.
    Bottom,
    /// Top-left corner.
    TopLeft,
    /// Top-right corner.
    TopRight,
    /// Bottom-left corner.
    BottomLeft,
    /// Bottom-right corner.
    BottomRight,
}

impl ResizeEdge {
    /// Whether this edge/corner affects the left boundary (and therefore x).
    pub fn affects_left(&self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    /// Whether this edge/corner affects the right boundary (and therefore width).
    pub fn affects_right(&self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    /// Whether this edge/corner affects the top boundary (and therefore y).
    pub fn affects_top(&self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    /// Whether this edge/corner affects the bottom boundary (and therefore height).
    pub fn affects_bottom(&self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

// ─── Portal geometry ──────────────────────────────────────────────────────────

/// Axis-aligned portal bounding rectangle in display pixels.
///
/// All fields are in display pixels from the top-left of the display.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortalRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PortalRect {
    /// Clamp the rect so width/height stay within `[min_*_px, max_*_px]` and
    /// the portal stays on-screen within `(display_w, display_h)`.
    ///
    /// When `affects_left`/`affects_top` would push the origin off-screen,
    /// the origin is clamped and the size is adjusted to keep the portal
    /// against the boundary.
    pub fn clamped(self, bounds: &ResizeBounds) -> Self {
        let w = self
            .width
            .clamp(bounds.tokens.min_width_px, bounds.max_width_px);
        let h = self
            .height
            .clamp(bounds.tokens.min_height_px, bounds.max_height_px);

        // Clamp origin so the portal stays fully on-screen.
        let x = self.x.clamp(0.0, (bounds.display_w - w).max(0.0));
        let y = self.y.clamp(0.0, (bounds.display_h - h).max(0.0));

        Self {
            x,
            y,
            width: w,
            height: h,
        }
    }
}

/// Groups clamping bounds for resize operations, reducing function argument count.
///
/// Passed by reference to avoid copying across every call site on the hot path.
/// Constructed once per frame (or on geometry change) from the token map and
/// the lease/scene budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeBounds {
    /// Resolved token bounds (min size, step, affordance).
    pub tokens: PortalWindowTokens,
    /// Lease/scene-budget maximum portal width in pixels (§6b.3).
    pub max_width_px: f32,
    /// Lease/scene-budget maximum portal height in pixels (§6b.3).
    pub max_height_px: f32,
    /// Display width in pixels (on-screen clamp).
    pub display_w: f32,
    /// Display height in pixels (on-screen clamp).
    pub display_h: f32,
}

// ─── Coalescible geometry snapshot (§6b.4) ────────────────────────────────────

/// Coalescible state-stream geometry snapshot delivered to the owning adapter.
///
/// Message class: **state-stream**. The transport MUST drop older snapshots
/// when a newer one arrives within the same delivery window (latest-wins).
///
/// The snapshot carries the portal's display-pixel bounds after clamping.
/// Adapter publishes that attempt to update the same portal while a gesture is
/// active MUST be rejected; the snapshot is the authoritative geometry until
/// gesture end.
///
/// Spec §6b.4: "gesture remains authoritative over adapter publishes until
/// gesture end."
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeometrySnapshot {
    /// Portal ID (opaque string key matching the adapter's portal attachment).
    /// In the raw-tile pilot this is the portal's `interaction_id`.
    pub portal_id_hash: u64,
    /// Final clamped portal bounds after this gesture step.
    pub rect: PortalRect,
    /// True during an active gesture (gesture is authoritative).
    /// False on gesture end (adapter may resume publishing).
    pub gesture_active: bool,
    /// Monotonic sequence counter — allows the adapter to detect skipped
    /// snapshots when the transport does not deliver every event.
    pub sequence: u64,
}

// ─── Pointer resize state machine (§6b.1) ─────────────────────────────────────

/// Phase of the per-device pointer resize interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizePhase {
    /// No resize in progress.
    Idle,
    /// Resize gesture active (pointer is held on an affordance and moving).
    Active,
}

/// Per-device pointer resize state.
///
/// One instance per active pointer device that has pressed an affordance region.
/// Instances are created on `PointerDown` on an affordance and removed on `PointerUp`.
#[derive(Clone, Debug)]
pub struct DeviceResizeState {
    /// Current phase.
    pub phase: ResizePhase,
    /// Which edge/corner is being dragged.
    pub edge: ResizeEdge,
    /// Display-space X where the pointer pressed down.
    pub press_x: f32,
    /// Display-space Y where the pointer pressed down.
    pub press_y: f32,
    /// Portal rect at the time the gesture started.
    pub initial_rect: PortalRect,
}

impl DeviceResizeState {
    /// Begin a resize gesture.
    pub fn new(edge: ResizeEdge, press_x: f32, press_y: f32, initial_rect: PortalRect) -> Self {
        Self {
            phase: ResizePhase::Active,
            edge,
            press_x,
            press_y,
            initial_rect,
        }
    }

    /// Compute the new rect given the current pointer position, clamped to bounds.
    ///
    /// Uses the initial rect and the total delta from the press origin so that
    /// floating-point error does not accumulate frame-by-frame.
    pub fn compute_rect(&self, pointer_x: f32, pointer_y: f32, bounds: &ResizeBounds) -> PortalRect {
        let dx = pointer_x - self.press_x;
        let dy = pointer_y - self.press_y;

        let mut x = self.initial_rect.x;
        let mut y = self.initial_rect.y;
        let mut w = self.initial_rect.width;
        let mut h = self.initial_rect.height;

        // Apply delta to the affected edges.
        if self.edge.affects_right() {
            w += dx;
        }
        if self.edge.affects_left() {
            x += dx;
            w -= dx;
        }
        if self.edge.affects_bottom() {
            h += dy;
        }
        if self.edge.affects_top() {
            y += dy;
            h -= dy;
        }

        PortalRect { x, y, width: w, height: h }.clamped(bounds)
    }
}

// ─── Resize event outcome ─────────────────────────────────────────────────────

/// Outcome of processing a pointer or hotkey event through the resize recognizer.
#[derive(Clone, Debug, PartialEq)]
pub enum ResizeOutcome {
    /// No resize action.
    Idle,
    /// Gesture started. Caller MUST mark the portal as gesture-authoritative.
    GestureStarted { snapshot: GeometrySnapshot },
    /// Gesture in progress — geometry changed. Emit the snapshot downstream.
    GestureUpdate { snapshot: GeometrySnapshot },
    /// Gesture ended (pointer up). Final geometry in snapshot. Caller MUST
    /// mark the portal as no longer gesture-authoritative.
    GestureEnded { snapshot: GeometrySnapshot },
    /// Hotkey resize applied (not a pointer gesture). Always gesture_active=false.
    HotkeyApplied { snapshot: GeometrySnapshot },
}

// ─── Portal resize state (per-portal) ────────────────────────────────────────

/// Per-portal resize management — tracks active pointer gestures and emits
/// coalescible geometry snapshots.
///
/// One instance per portal. Callers should hold this alongside the portal's
/// current geometry and call the appropriate handler on each input event.
#[derive(Debug, Default)]
pub struct PortalResizeState {
    /// Per-device resize gestures (usually only one device at a time).
    device_states: std::collections::HashMap<u32, DeviceResizeState>,
    /// Monotonic sequence counter for snapshots.
    sequence: u64,
    /// Hash of the portal ID (used in snapshots; callers set this at creation).
    portal_id_hash: u64,
}

impl PortalResizeState {
    /// Create a new per-portal resize state.
    ///
    /// `portal_id_hash` is a stable hash of the portal's opaque ID string,
    /// used to identify the snapshot's owner without copying a String on the
    /// hot path.
    pub fn new(portal_id_hash: u64) -> Self {
        Self {
            device_states: std::collections::HashMap::new(),
            sequence: 0,
            portal_id_hash,
        }
    }

    /// Returns true while any pointer device has an active resize gesture.
    ///
    /// When this is true, the caller MUST reject adapter geometry publishes
    /// (gesture is authoritative — §6b.4).
    pub fn gesture_active(&self) -> bool {
        self.device_states
            .values()
            .any(|s| s.phase == ResizePhase::Active)
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    fn snapshot(&mut self, rect: PortalRect, gesture_active: bool) -> GeometrySnapshot {
        GeometrySnapshot {
            portal_id_hash: self.portal_id_hash,
            rect,
            gesture_active,
            sequence: self.next_sequence(),
        }
    }

    /// Process a pointer-down event on a resize affordance.
    ///
    /// Returns `ResizeOutcome::GestureStarted` with the clamped initial rect.
    pub fn on_pointer_down(
        &mut self,
        device_id: u32,
        edge: ResizeEdge,
        press_x: f32,
        press_y: f32,
        current_rect: PortalRect,
        bounds: &ResizeBounds,
    ) -> ResizeOutcome {
        let initial = current_rect.clamped(bounds);
        self.device_states.insert(
            device_id,
            DeviceResizeState::new(edge, press_x, press_y, initial),
        );
        let snap = self.snapshot(initial, true);
        ResizeOutcome::GestureStarted { snapshot: snap }
    }

    /// Process a pointer-move event during an active resize gesture.
    ///
    /// Returns `ResizeOutcome::GestureUpdate` when a gesture is active for
    /// `device_id`, or `ResizeOutcome::Idle` when no gesture is in progress.
    pub fn on_pointer_move(
        &mut self,
        device_id: u32,
        pointer_x: f32,
        pointer_y: f32,
        bounds: &ResizeBounds,
    ) -> ResizeOutcome {
        let Some(state) = self.device_states.get(&device_id) else {
            return ResizeOutcome::Idle;
        };
        if state.phase != ResizePhase::Active {
            return ResizeOutcome::Idle;
        }
        let rect = state.compute_rect(pointer_x, pointer_y, bounds);
        let snap = self.snapshot(rect, true);
        ResizeOutcome::GestureUpdate { snapshot: snap }
    }

    /// Process a pointer-up event, ending any active resize gesture for `device_id`.
    ///
    /// Returns `ResizeOutcome::GestureEnded` with the final clamped rect, or
    /// `ResizeOutcome::Idle` if no gesture was active.
    pub fn on_pointer_up(
        &mut self,
        device_id: u32,
        pointer_x: f32,
        pointer_y: f32,
        bounds: &ResizeBounds,
    ) -> ResizeOutcome {
        let Some(state) = self.device_states.remove(&device_id) else {
            return ResizeOutcome::Idle;
        };
        let rect = state.compute_rect(pointer_x, pointer_y, bounds);
        // gesture_active reflects remaining devices, but this gesture is over
        let still_active = self.gesture_active();
        let snap = self.snapshot(rect, still_active);
        ResizeOutcome::GestureEnded { snapshot: snap }
    }
}

// ─── Focus-scoped hotkey resize (§6b.2) ──────────────────────────────────────

/// Outcome of a hotkey resize event.
#[derive(Clone, Debug, PartialEq)]
pub enum HotkeyResizeOutcome {
    /// The portal is not focused; hotkey was NOT consumed.
    NotFocused,
    /// Hotkey consumed and geometry updated. Caller MUST deliver snapshot.
    Applied { snapshot: GeometrySnapshot },
}

/// Direction of a hotkey resize step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyResizeDir {
    /// Grow (Ctrl+`+` or Ctrl+`=`).
    Grow,
    /// Shrink (Ctrl+`-`).
    Shrink,
}

impl HotkeyResizeDir {
    /// Parse from a DOM `KeyboardEvent.key` string.
    ///
    /// Returns `Some(Grow)` for `"+"` or `"="`, `Some(Shrink)` for `"-"`,
    /// `None` for anything else.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "+" | "=" => Some(Self::Grow),
            "-" => Some(Self::Shrink),
            _ => None,
        }
    }
}

/// Apply a focus-scoped hotkey resize step to the current rect.
///
/// # Arguments
///
/// * `focused` — true if the portal holds keyboard focus. If false, the
///   hotkey is NOT consumed and `HotkeyResizeOutcome::NotFocused` is returned.
/// * `dir` — grow or shrink direction.
/// * `current_rect` — portal bounds before the step.
/// * `bounds` — clamping bounds (tokens, max size, display size).
/// * `state` — portal resize state (for sequence counter).
///
/// Grows/shrinks both width and height symmetrically by `resize_step_px`
/// (centred on the existing rect, so the centre stays roughly fixed).
/// The result is clamped by `clamped()` to satisfy the overflow contract.
pub fn apply_hotkey_resize(
    focused: bool,
    dir: HotkeyResizeDir,
    current_rect: PortalRect,
    bounds: &ResizeBounds,
    state: &mut PortalResizeState,
) -> HotkeyResizeOutcome {
    if !focused {
        return HotkeyResizeOutcome::NotFocused;
    }

    let step = bounds.tokens.resize_step_px;
    let delta = match dir {
        HotkeyResizeDir::Grow => step,
        HotkeyResizeDir::Shrink => -step,
    };

    // Grow/shrink symmetrically: shift origin by half-step in each direction.
    let new_w = current_rect.width + delta;
    let new_h = current_rect.height + delta;
    let new_x = current_rect.x - delta / 2.0;
    let new_y = current_rect.y - delta / 2.0;

    let new_rect = PortalRect {
        x: new_x,
        y: new_y,
        width: new_w,
        height: new_h,
    }
    .clamped(bounds);

    // Advance sequence only if clamping actually changed geometry.
    // (e.g., already at min when shrinking — still emit a snapshot so the
    // adapter can detect the attempt, but do not advance the sequence.)
    let changed = (new_rect.x - current_rect.x).abs() > f32::EPSILON
        || (new_rect.y - current_rect.y).abs() > f32::EPSILON
        || (new_rect.width - current_rect.width).abs() > f32::EPSILON
        || (new_rect.height - current_rect.height).abs() > f32::EPSILON;

    let snap = GeometrySnapshot {
        portal_id_hash: state.portal_id_hash,
        rect: new_rect,
        gesture_active: false,
        sequence: if changed { state.next_sequence() } else { state.sequence },
    };
    HotkeyResizeOutcome::Applied { snapshot: snap }
}

// ─── Affordance hit test (§6b.1) ──────────────────────────────────────────────

/// Determine which resize edge or corner a pointer position hits, given the
/// portal rect and the affordance region width.
///
/// Returns `None` if the pointer is not in any affordance region.
/// Corner affordances take priority over edge affordances (corners are at
/// the intersection of two edge capture regions).
///
/// The content layer is inside the affordance capture regions; pointer events
/// within the inner content area do not trigger resize.
pub fn hit_affordance(
    pointer_x: f32,
    pointer_y: f32,
    rect: &PortalRect,
    affordance_px: f32,
) -> Option<ResizeEdge> {
    let in_left = pointer_x >= rect.x && pointer_x < rect.x + affordance_px;
    let in_right =
        pointer_x > rect.x + rect.width - affordance_px && pointer_x <= rect.x + rect.width;
    let in_top = pointer_y >= rect.y && pointer_y < rect.y + affordance_px;
    let in_bottom =
        pointer_y > rect.y + rect.height - affordance_px && pointer_y <= rect.y + rect.height;

    // Must be within the portal rect at all
    let in_rect = pointer_x >= rect.x
        && pointer_x <= rect.x + rect.width
        && pointer_y >= rect.y
        && pointer_y <= rect.y + rect.height;

    if !in_rect {
        return None;
    }

    // Corners first (intersection of two edge bands)
    match (in_top, in_bottom, in_left, in_right) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (true, _, _, true) => Some(ResizeEdge::TopRight),
        (_, true, true, _) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (true, _, _, _) => Some(ResizeEdge::Top),
        (_, true, _, _) => Some(ResizeEdge::Bottom),
        (_, _, true, _) => Some(ResizeEdge::Left),
        (_, _, _, true) => Some(ResizeEdge::Right),
        _ => None, // inside content area
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_tokens() -> PortalWindowTokens {
        PortalWindowTokens::default()
    }

    fn default_bounds() -> ResizeBounds {
        let tokens = default_tokens();
        ResizeBounds {
            tokens,
            max_width_px: 2000.0,
            max_height_px: 2000.0,
            display_w: 3840.0,
            display_h: 2160.0,
        }
    }

    // ─── PortalRect::clamped ───────────────────────────────────────────────

    #[test]
    fn clamped_rect_within_bounds_unchanged() {
        let bounds = default_bounds();
        let r = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let c = r.clamped(&bounds);
        assert_eq!(c, r, "rect within bounds must be unchanged by clamping");
    }

    #[test]
    fn clamped_rect_enforces_min_width() {
        let bounds = default_bounds(); // min_width = 240
        let r = PortalRect { x: 0.0, y: 0.0, width: 50.0, height: 300.0 };
        let c = r.clamped(&bounds);
        assert!(
            c.width >= bounds.tokens.min_width_px,
            "width must be at least min_width_px after clamping"
        );
    }

    #[test]
    fn clamped_rect_enforces_min_height() {
        let bounds = default_bounds(); // min_height = 160
        let r = PortalRect { x: 0.0, y: 0.0, width: 400.0, height: 50.0 };
        let c = r.clamped(&bounds);
        assert!(
            c.height >= bounds.tokens.min_height_px,
            "height must be at least min_height_px after clamping"
        );
    }

    #[test]
    fn clamped_rect_stays_on_screen() {
        let bounds = default_bounds();
        // rect extends beyond right and bottom edges
        let r = PortalRect { x: 3700.0, y: 2000.0, width: 400.0, height: 300.0 };
        let c = r.clamped(&bounds);
        assert!(
            c.x + c.width <= bounds.display_w + f32::EPSILON,
            "right edge must stay on screen"
        );
        assert!(
            c.y + c.height <= bounds.display_h + f32::EPSILON,
            "bottom edge must stay on screen"
        );
    }

    #[test]
    fn clamped_rect_enforces_max_bounds() {
        let tokens = default_tokens();
        let bounds = ResizeBounds {
            tokens,
            max_width_px: 600.0,
            max_height_px: 400.0,
            display_w: 3840.0,
            display_h: 2160.0,
        };
        let r = PortalRect { x: 0.0, y: 0.0, width: 900.0, height: 700.0 };
        let c = r.clamped(&bounds);
        assert!(
            c.width <= bounds.max_width_px,
            "width must be at most max_width after clamping"
        );
        assert!(
            c.height <= bounds.max_height_px,
            "height must be at most max_height after clamping"
        );
    }

    // ─── ResizeEdge helpers ────────────────────────────────────────────────

    #[test]
    fn resize_edge_affects_correct_sides() {
        assert!(ResizeEdge::Left.affects_left());
        assert!(!ResizeEdge::Left.affects_right());
        assert!(ResizeEdge::Right.affects_right());
        assert!(!ResizeEdge::Right.affects_left());
        assert!(ResizeEdge::TopLeft.affects_left());
        assert!(ResizeEdge::TopLeft.affects_top());
        assert!(!ResizeEdge::TopLeft.affects_right());
        assert!(!ResizeEdge::TopLeft.affects_bottom());
        assert!(ResizeEdge::BottomRight.affects_right());
        assert!(ResizeEdge::BottomRight.affects_bottom());
    }

    // ─── DeviceResizeState::compute_rect ─────────────────────────────────

    #[test]
    fn right_edge_resize_grows_width_rightward() {
        let bounds = default_bounds();
        let initial = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let state = DeviceResizeState::new(ResizeEdge::Right, 500.0, 250.0, initial);
        // drag 50px to the right
        let result = state.compute_rect(550.0, 250.0, &bounds);
        assert!(
            (result.width - 450.0).abs() < 1.0,
            "right-drag 50px must grow width by 50px"
        );
        assert!(
            (result.x - initial.x).abs() < 1.0,
            "right-drag must not change x"
        );
    }

    #[test]
    fn left_edge_resize_grows_width_leftward() {
        let bounds = default_bounds();
        let initial = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let state = DeviceResizeState::new(ResizeEdge::Left, 100.0, 250.0, initial);
        // drag 50px to the left
        let result = state.compute_rect(50.0, 250.0, &bounds);
        assert!(
            (result.width - 450.0).abs() < 1.0,
            "left-drag 50px must grow width by 50px"
        );
        assert!(
            (result.x - 50.0).abs() < 1.0,
            "left-drag must move x leftward"
        );
    }

    #[test]
    fn resize_clamps_to_min_width_when_dragging_too_far() {
        let bounds = default_bounds(); // min_width = 240
        let initial = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let state = DeviceResizeState::new(ResizeEdge::Right, 500.0, 250.0, initial);
        // drag 400px to the left (would make width=-100)
        let result = state.compute_rect(100.0, 250.0, &bounds);
        assert!(
            result.width >= bounds.tokens.min_width_px,
            "resize past minimum must clamp to min_width_px (no clipped glyphs)"
        );
    }

    #[test]
    fn resize_delta_uses_initial_rect_not_accumulated() {
        // Verifies that compute_rect always uses the initial rect + total delta,
        // not the accumulated delta — preventing floating-point drift.
        let bounds = default_bounds();
        let initial = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let state = DeviceResizeState::new(ResizeEdge::Right, 500.0, 250.0, initial);

        let result_a = state.compute_rect(550.0, 250.0, &bounds);
        let result_b = state.compute_rect(600.0, 250.0, &bounds);

        // Each call uses the initial rect, so result_b should be 50px wider than result_a
        assert!(
            (result_b.width - result_a.width - 50.0).abs() < 1.0,
            "second compute must be 50px wider than first (no accumulated drift)"
        );
    }

    // ─── PortalResizeState gesture lifecycle ──────────────────────────────

    #[test]
    fn gesture_not_active_by_default() {
        let state = PortalResizeState::new(0xdeadbeef);
        assert!(
            !state.gesture_active(),
            "no gesture should be active at construction"
        );
    }

    #[test]
    fn gesture_active_after_pointer_down() {
        let mut state = PortalResizeState::new(0xdeadbeef);
        let bounds = default_bounds();
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let outcome = state.on_pointer_down(1, ResizeEdge::Right, 500.0, 250.0, rect, &bounds);
        assert!(state.gesture_active(), "gesture must be active after pointer-down");
        matches!(outcome, ResizeOutcome::GestureStarted { .. });
    }

    #[test]
    fn gesture_inactive_after_pointer_up() {
        let mut state = PortalResizeState::new(0xdeadbeef);
        let bounds = default_bounds();
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        state.on_pointer_down(1, ResizeEdge::Right, 500.0, 250.0, rect, &bounds);
        let outcome = state.on_pointer_up(1, 550.0, 250.0, &bounds);
        assert!(
            !state.gesture_active(),
            "gesture must be inactive after pointer-up"
        );
        matches!(outcome, ResizeOutcome::GestureEnded { .. });
    }

    #[test]
    fn snapshot_sequence_is_monotonically_increasing() {
        let mut state = PortalResizeState::new(0xdeadbeef);
        let bounds = default_bounds();
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };

        let start = state.on_pointer_down(1, ResizeEdge::Right, 500.0, 250.0, rect, &bounds);
        let mid = state.on_pointer_move(1, 520.0, 250.0, &bounds);
        let end = state.on_pointer_up(1, 540.0, 250.0, &bounds);

        let seq_start = match start {
            ResizeOutcome::GestureStarted { ref snapshot } => snapshot.sequence,
            _ => panic!("expected GestureStarted"),
        };
        let seq_mid = match mid {
            ResizeOutcome::GestureUpdate { ref snapshot } => snapshot.sequence,
            _ => panic!("expected GestureUpdate"),
        };
        let seq_end = match end {
            ResizeOutcome::GestureEnded { ref snapshot } => snapshot.sequence,
            _ => panic!("expected GestureEnded"),
        };

        assert!(
            seq_start < seq_mid,
            "sequence must be monotonically increasing: start < mid"
        );
        assert!(
            seq_mid < seq_end,
            "sequence must be monotonically increasing: mid < end"
        );
    }

    #[test]
    fn gesture_snapshot_has_gesture_active_false_on_end() {
        let mut state = PortalResizeState::new(0xdeadbeef);
        let bounds = default_bounds();
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };

        state.on_pointer_down(1, ResizeEdge::Right, 500.0, 250.0, rect, &bounds);
        let end = state.on_pointer_up(1, 550.0, 250.0, &bounds);

        let snap = match end {
            ResizeOutcome::GestureEnded { snapshot } => snapshot,
            _ => panic!("expected GestureEnded"),
        };
        assert!(
            !snap.gesture_active,
            "gesture_active must be false in GestureEnded snapshot"
        );
    }

    #[test]
    fn move_event_without_active_gesture_returns_idle() {
        let mut state = PortalResizeState::new(0xdeadbeef);
        let bounds = default_bounds();
        let outcome = state.on_pointer_move(1, 500.0, 250.0, &bounds);
        assert_eq!(
            outcome,
            ResizeOutcome::Idle,
            "move without active gesture must return Idle"
        );
    }

    // ─── Hotkey resize ────────────────────────────────────────────────────

    #[test]
    fn hotkey_grow_increases_size() {
        let bounds = default_bounds(); // step = 32px
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let mut state = PortalResizeState::new(0xdeadbeef);

        let result = apply_hotkey_resize(true, HotkeyResizeDir::Grow, rect, &bounds, &mut state);

        let snap = match result {
            HotkeyResizeOutcome::Applied { snapshot } => snapshot,
            _ => panic!("expected Applied"),
        };
        assert!(snap.rect.width > rect.width, "grow must increase width");
        assert!(snap.rect.height > rect.height, "grow must increase height");
        assert!(
            !snap.gesture_active,
            "hotkey resize must never set gesture_active"
        );
    }

    #[test]
    fn hotkey_shrink_decreases_size() {
        let bounds = default_bounds();
        let rect = PortalRect { x: 100.0, y: 100.0, width: 600.0, height: 400.0 };
        let mut state = PortalResizeState::new(0xdeadbeef);

        let result =
            apply_hotkey_resize(true, HotkeyResizeDir::Shrink, rect, &bounds, &mut state);

        let snap = match result {
            HotkeyResizeOutcome::Applied { snapshot } => snapshot,
            _ => panic!("expected Applied"),
        };
        assert!(snap.rect.width < rect.width, "shrink must decrease width");
        assert!(snap.rect.height < rect.height, "shrink must decrease height");
    }

    #[test]
    fn hotkey_not_consumed_when_portal_not_focused() {
        let bounds = default_bounds();
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let mut state = PortalResizeState::new(0xdeadbeef);

        let result =
            apply_hotkey_resize(false, HotkeyResizeDir::Grow, rect, &bounds, &mut state);
        assert_eq!(
            result,
            HotkeyResizeOutcome::NotFocused,
            "hotkey must not be consumed by unfocused portal"
        );
    }

    #[test]
    fn hotkey_shrink_clamped_to_min_at_boundary() {
        let bounds = default_bounds(); // min=240x160, step=32
        // Portal already at or near minimum size
        let rect = PortalRect {
            x: 100.0,
            y: 100.0,
            width: bounds.tokens.min_width_px,
            height: bounds.tokens.min_height_px,
        };
        let mut state = PortalResizeState::new(0xdeadbeef);

        let result =
            apply_hotkey_resize(true, HotkeyResizeDir::Shrink, rect, &bounds, &mut state);

        let snap = match result {
            HotkeyResizeOutcome::Applied { snapshot } => snapshot,
            _ => panic!("expected Applied"),
        };
        assert!(
            snap.rect.width >= bounds.tokens.min_width_px,
            "shrink at minimum must clamp to min_width (no clipped glyphs)"
        );
        assert!(
            snap.rect.height >= bounds.tokens.min_height_px,
            "shrink at minimum must clamp to min_height (no clipped glyphs)"
        );
    }

    #[test]
    fn hotkey_key_parser_from_key() {
        assert_eq!(HotkeyResizeDir::from_key("+"), Some(HotkeyResizeDir::Grow));
        assert_eq!(HotkeyResizeDir::from_key("="), Some(HotkeyResizeDir::Grow));
        assert_eq!(HotkeyResizeDir::from_key("-"), Some(HotkeyResizeDir::Shrink));
        assert_eq!(HotkeyResizeDir::from_key("a"), None);
        assert_eq!(HotkeyResizeDir::from_key("Enter"), None);
    }

    // ─── Affordance hit test ───────────────────────────────────────────────

    #[test]
    fn hit_affordance_right_edge() {
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let affordance = 8.0;
        // right edge: x=492..500
        let edge = hit_affordance(496.0, 250.0, &rect, affordance);
        assert_eq!(edge, Some(ResizeEdge::Right));
    }

    #[test]
    fn hit_affordance_left_edge() {
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let affordance = 8.0;
        // left edge: x=100..108
        let edge = hit_affordance(104.0, 250.0, &rect, affordance);
        assert_eq!(edge, Some(ResizeEdge::Left));
    }

    #[test]
    fn hit_affordance_top_edge() {
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let affordance = 8.0;
        let edge = hit_affordance(300.0, 104.0, &rect, affordance);
        assert_eq!(edge, Some(ResizeEdge::Top));
    }

    #[test]
    fn hit_affordance_bottom_edge() {
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let affordance = 8.0;
        // bottom edge: y=392..400
        let edge = hit_affordance(300.0, 396.0, &rect, affordance);
        assert_eq!(edge, Some(ResizeEdge::Bottom));
    }

    #[test]
    fn hit_affordance_top_left_corner() {
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let affordance = 8.0;
        let edge = hit_affordance(104.0, 104.0, &rect, affordance);
        assert_eq!(edge, Some(ResizeEdge::TopLeft));
    }

    #[test]
    fn hit_affordance_bottom_right_corner() {
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let affordance = 8.0;
        let edge = hit_affordance(496.0, 396.0, &rect, affordance);
        assert_eq!(edge, Some(ResizeEdge::BottomRight));
    }

    #[test]
    fn hit_affordance_content_area_returns_none() {
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let affordance = 8.0;
        // centre of the content area
        let edge = hit_affordance(300.0, 250.0, &rect, affordance);
        assert_eq!(edge, None, "content area must not hit any affordance");
    }

    #[test]
    fn hit_affordance_outside_rect_returns_none() {
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };
        let affordance = 8.0;
        let edge = hit_affordance(50.0, 250.0, &rect, affordance);
        assert_eq!(edge, None, "pointer outside rect must not hit any affordance");
    }

    // ─── Adapter authority (§6b.4) ────────────────────────────────────────

    /// During an active gesture, `gesture_active()` returns true, which the
    /// caller uses to reject adapter geometry publishes.
    #[test]
    fn adapter_publishes_must_be_rejected_during_gesture() {
        let mut state = PortalResizeState::new(0xdeadbeef);
        let bounds = default_bounds();
        let rect = PortalRect { x: 100.0, y: 100.0, width: 400.0, height: 300.0 };

        // Before gesture: adapter may publish
        assert!(
            !state.gesture_active(),
            "adapter may publish before gesture starts"
        );

        state.on_pointer_down(1, ResizeEdge::Bottom, 300.0, 400.0, rect, &bounds);

        // During gesture: adapter must NOT publish
        assert!(
            state.gesture_active(),
            "adapter must be blocked during active gesture (gesture is authoritative)"
        );

        state.on_pointer_up(1, 300.0, 450.0, &bounds);

        // After gesture: adapter may publish again
        assert!(
            !state.gesture_active(),
            "adapter may publish after gesture ends"
        );
    }
}
