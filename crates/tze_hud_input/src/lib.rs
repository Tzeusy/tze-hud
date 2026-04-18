//! # tze_hud_input
//!
//! Input pipeline for tze_hud. Processes pointer events, performs hit-testing,
//! updates local feedback state (hover/pressed/focused), and dispatches events
//! to agents. Local feedback happens synchronously in < 4ms — no agent roundtrip.
//!
//! ## Module structure
//!
//! - `lib.rs` — `InputProcessor`, pointer event processing, `AgentDispatch`
//! - `focus_tree` — per-tab focus tree data structure and history
//! - `focus`      — focus manager (lifecycle, cycling, events, ring metadata)
//! - `keyboard`   — keyboard event types and dispatch (KeyDownEvent, KeyUpEvent,
//!   CharacterEvent) per RFC 0004 §7.4
//! - `command`    — abstract command input model (NAVIGATE_NEXT … SCROLL_DOWN)
//!   per RFC 0004 §10
//! - [`pointer`] — rich pointer event types (PointerDownEvent, ClickEvent, etc.)
//! - [`events`] — HitTestResult, RouteTarget, SceneLocalPatch, InputEnvelope, EventBatch
//! - [`hit_test`] — headless-testable hit-test pipeline
//! - [`dispatch`] — Stage 1+2 dispatch pipeline (DispatchProcessor)
//! - `local_feedback` — `LocalFeedbackStyle`, `ResolvedFeedbackStyle`, rollback tracker
//! - `scroll` — `ScrollConfig`, `ScrollState`, scroll-local-first processing
//! - `capture`    — pointer capture manager (RFC 0004 §2)
//! - [`envelope`] — full 19-variant `InputEnvelope` for the batching/coalescing pipeline
//! - [`event_queue`] — per-agent event queue with backpressure and transactional protection
//! - [`coalescing`] — in-place queue coalescing and per-frame `FrameCoalescer`
//! - [`batching`] — `EventBatchAssembler` for per-frame batch assembly
//!
//! ## Pointer Capture
//!
//! The [`capture`] module implements RFC 0004 §2 pointer capture semantics:
//! - Only one node holds capture globally at a time, per device.
//! - Capture can be explicit (CaptureRequest) or automatic (auto_capture=true).
//! - While captured, all events from the captured device route to the owning
//!   node, bypassing normal hit-testing.
//! - Capture is released by: explicit CaptureReleaseRequest, PointerUpEvent
//!   (when release_on_up=true), or runtime theft (Alt+Tab, lease revocation,
//!   tab switch).

pub mod batching;
pub mod coalescing;
pub mod envelope;
pub mod event_queue;

pub use batching::EventBatchAssembler;
pub use coalescing::{CoalesceResult, EventCoalescer, FrameCoalescer};
pub use envelope::{
    CharacterData,
    ClickData,
    CommandInputData,
    FocusGainedData,
    FocusLostData,
    GestureData,
    ImeCompositionEndData,
    // CaptureReleasedData and CaptureReleasedReason are defined in lib.rs
    // (from the pointer capture module); re-exporting envelope:: versions
    // would create duplicate definitions.
    ImeCompositionStartData,
    ImeCompositionUpdateData,
    KeyDownData,
    KeyUpData,
    PointerCancelData,
    PointerDownData,
    PointerEnterData,
    PointerLeaveData,
    PointerMoveData,
    PointerUpData,
    ScrollOffsetChangedData,
};
pub use event_queue::{AgentEventQueue, DEFAULT_QUEUE_DEPTH, HARD_CAP_QUEUE_DEPTH};

pub mod capture;
pub mod dispatch;
pub mod events;
pub mod hit_test;
pub mod local_feedback;
pub mod pointer;
pub mod scroll;

// Re-export core dispatch types at the crate root for convenience.
pub use dispatch::{DispatchOutcome, DispatchProcessor, build_agent_batch};
pub use events::{
    EventBatch, HitTestResult, InputEnvelope, LocalStateUpdate, RouteTarget, SceneLocalPatch,
    ScrollOffsetUpdate,
};
pub use hit_test::hit_test;
pub use local_feedback::{
    DEFAULT_FOCUS_RING_COLOR, DEFAULT_FOCUS_RING_WIDTH_PX, DEFAULT_HOVER_TINT,
    DEFAULT_PRESS_DARKEN, LocalFeedbackStyle, ROLLBACK_ANIMATION_MS, ResolvedFeedbackStyle,
    RollbackTracker,
};
pub use pointer::{
    CancelReason, ClickEvent, ContextMenuEvent, DoubleClickEvent, Modifiers, PointerButton,
    PointerCancelEvent, PointerDownEvent, PointerEnterEvent, PointerFields, PointerLeaveEvent,
    PointerMoveEvent, PointerUpEvent, RawPointerEvent, RawPointerEventKind,
};
pub use scroll::{
    ScrollConfig, ScrollEvent, ScrollOffsetChangedEvent, ScrollState, SetScrollOffsetRequest,
};

pub mod command;
pub mod drag;
pub mod focus;
pub mod focus_tree;
pub mod keyboard;

pub use command::{
    CommandAction, CommandDispatch, CommandInputEvent, CommandProcessor, CommandSource,
    RawCommandEvent,
};
pub use focus::{
    FocusGainedEvent, FocusLostEvent, FocusLostReason, FocusManager, FocusRequest, FocusResult,
    FocusRingBounds, FocusRingUpdate, FocusSource, FocusTransition,
};
pub use focus_tree::{FocusOwner, FocusTree};
pub use keyboard::{
    KeyboardDispatch, KeyboardDispatchKind, KeyboardModifiers, KeyboardProcessor,
    RawCharacterEvent, RawKeyDownEvent, RawKeyUpEvent,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::{ElementStore, ElementType, HitResult, NodeData, SceneId, ZoneInteractionKind};

pub use drag::{
    DEFAULT_SNAP_GRID_PCT, DRAG_HIGHLIGHT_BORDER_PX, DRAG_OPACITY_BOOST, DRAG_Z_ORDER_BOOST,
    DeviceDragState, DragConfig, DragEventOutcome, DragPhase, LONG_PRESS_MOVEMENT_TOLERANCE_DP,
    LONG_PRESS_POINTER_THRESHOLD_MS, LONG_PRESS_TOUCH_THRESHOLD_MS,
};
pub use tze_hud_scene::DragHandleElementKind;

/// Scroll step (pixels) applied per PgUp or PgDn keypress on a portal tile.
///
/// This is the keyboard scroll step for the OS keyboard path wired in hud-6bbe.
/// One PgUp/PgDn keypress scrolls 160px — approximately four line-scroll steps
/// (cf. wheel: LineDelta * 40px). Callers may override this constant when
/// computing the `delta_y` passed to [`InputProcessor::process_keyboard_scroll`].
pub const KEYBOARD_PAGE_SCROLL_PX: f32 = 160.0;

/// Raw pointer input event from the OS.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PointerEvent {
    pub x: f32,
    pub y: f32,
    pub kind: PointerEventKind,
    /// Device identifier — differentiates mouse, touch points, stylus, etc.
    /// Defaults to 0 (primary pointer device).
    #[serde(default)]
    pub device_id: u32,
    /// Monotonic timestamp (microseconds since process start).
    #[serde(skip)]
    pub timestamp: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerEventKind {
    Move,
    Down,
    Up,
}

/// Result of processing a pointer event — what changed locally.
#[derive(Clone, Debug)]
pub struct InputResult {
    /// Full hit-test result for this event.
    pub hit: HitResult,
    /// The interaction_id of the hit region (if a HitRegionNode was hit).
    pub interaction_id: Option<String>,
    /// Whether this was an activation (press then release on the same hit region).
    pub activated: bool,
    /// Time taken for local acknowledgement (microseconds).
    pub local_ack_us: u64,
    /// Time taken for hit-test (microseconds).
    pub hit_test_us: u64,
    /// Primary agent dispatch descriptor, if an event should be forwarded to an agent.
    pub dispatch: Option<AgentDispatch>,
    /// Additional agent dispatches to deliver after `dispatch` (in order).
    ///
    /// Used when a single pointer event produces multiple protocol-level events —
    /// for example, `PointerUp` + `CaptureReleased` when `release_on_up=true`.
    /// Callers MUST deliver all entries in `extra_dispatches` in order after `dispatch`.
    pub extra_dispatches: Vec<AgentDispatch>,
    /// Local state patch to forward to the compositor immediately.
    ///
    /// Non-empty when pressed/hovered/focused state changed. This patch MUST be
    /// sent to the compositor via the dedicated local-patch channel before the
    /// next frame to guarantee `input_to_next_present p99 < 33ms`.
    pub local_patch: SceneLocalPatch,
}

/// Information needed to dispatch this input event to the owning agent.
///
/// Callers (e.g. the runtime kernel) use this to call into the protocol layer
/// without the input crate needing a direct dependency on tze_hud_protocol.
#[derive(Clone, Debug)]
pub struct AgentDispatch {
    /// Namespace (agent name) of the tile owner.
    pub namespace: String,
    pub tile_id: SceneId,
    pub node_id: SceneId,
    pub interaction_id: String,
    /// Pointer position in tile-local coordinates.
    pub local_x: f32,
    pub local_y: f32,
    /// Pointer position in display-space coordinates.
    pub display_x: f32,
    pub display_y: f32,
    /// Device that generated this event.
    pub device_id: u32,
    pub kind: AgentDispatchKind,
    /// Populated when `kind == CaptureReleased`.
    pub capture_released_reason: Option<CaptureReleasedReason>,
}

/// Which type of input event to deliver to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentDispatchKind {
    PointerMove,
    PointerDown,
    PointerUp,
    PointerEnter,
    PointerLeave,
    /// Activation: press + release on the same hit region.
    Activated,
    /// Pointer capture was granted (response to CaptureRequest or auto-capture).
    CaptureGranted,
    /// Pointer capture request was denied (another node holds capture for this device).
    CaptureDenied,
    /// Pointer capture was released.  Carry `capture_released_reason` in AgentDispatch.
    CaptureReleased,
    /// Pointer interaction was cancelled (e.g., runtime capture theft via Alt+Tab).
    /// The agent MUST treat this as terminal — no further PointerUp/Activated expected.
    PointerCancel,
}

/// Why pointer capture was released.
///
/// Carried in `AgentDispatch` when `kind == AgentDispatchKind::CaptureReleased`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureReleasedReason {
    /// The owning agent sent an explicit CaptureReleaseRequest.
    AgentReleased,
    /// PointerUpEvent arrived while `release_on_up=true`.
    PointerUp,
    /// Runtime revoked capture unconditionally (Alt+Tab, system notification,
    /// tab switch, or other system event).
    RuntimeRevoked,
    /// Agent lease was revoked; capture is implicitly released.
    LeaseRevoked,
}

/// A request from an agent to acquire pointer capture.
///
/// The runtime evaluates this after a PointerDownEvent has been dispatched.
/// Capture is only granted in response to PointerDown (spec lines 104-106).
#[derive(Clone, Debug)]
pub struct CaptureRequest {
    /// The tile whose node is requesting capture.
    pub tile_id: SceneId,
    /// The node requesting capture.
    pub node_id: SceneId,
    /// The pointer device to capture.
    pub device_id: u32,
}

/// A request from an agent to release pointer capture.
#[derive(Clone, Debug)]
pub struct CaptureReleaseRequest {
    /// The device whose capture should be released.
    pub device_id: u32,
}

/// The input processor. Tracks state across events for local feedback.
pub struct InputProcessor {
    /// Currently hovered node.
    current_hover: Option<(SceneId, SceneId)>, // (tile_id, node_id)
    /// Currently pressed node.
    current_press: Option<(SceneId, SceneId)>, // (tile_id, node_id)
    /// Currently hovered chrome drag handle interaction id.
    current_drag_handle_hover: Option<String>,
    /// Currently pressed chrome drag handle interaction id.
    current_drag_handle_press: Option<String>,
    /// Per-device long-press drag state.
    ///
    /// Keyed by `device_id`.  Each entry is independent so multi-touch devices
    /// can have simultaneous independent drag interactions on different handles.
    pub drag_states: HashMap<u32, DeviceDragState>,
    /// Drag configuration (snap grid, etc.).
    pub drag_config: DragConfig,
    /// Rollback animation tracker (agent-rejection-triggered).
    rollback_tracker: RollbackTracker,
    /// Pointer capture manager.
    pub capture: capture::PointerCaptureManager,
    /// Local-first tile scroll state.
    scroll_state: ScrollState,
}

impl InputProcessor {
    pub fn new() -> Self {
        Self {
            current_hover: None,
            current_press: None,
            current_drag_handle_hover: None,
            current_drag_handle_press: None,
            drag_states: HashMap::new(),
            drag_config: DragConfig::default(),
            rollback_tracker: RollbackTracker::new(),
            capture: capture::PointerCaptureManager::new(),
            scroll_state: ScrollState::new(),
        }
    }

    /// Queue an adapter-driven absolute scroll offset request.
    ///
    /// Requests are applied on `commit_scroll_updates`. If a user scroll for the
    /// same tile lands in the same frame, the user offset remains authoritative.
    pub fn queue_set_scroll_offset(&mut self, req: SetScrollOffsetRequest) {
        self.scroll_state.queue_agent_request(req);
    }

    /// Process a user-originated scroll event through the runtime-owned scroll path.
    ///
    /// Returns the changed tile and absolute offset when the event updates a
    /// scrollable tile. Returns `None` for passthrough / unscrollable hits.
    pub fn process_scroll_event(
        &mut self,
        event: &ScrollEvent,
        scene: &mut SceneGraph,
    ) -> Option<ScrollOffsetChangedEvent> {
        let hit = scene.hit_test(event.x, event.y);
        let tile_id = match hit {
            HitResult::NodeHit { tile_id, .. } | HitResult::TileHit { tile_id } => tile_id,
            _ => return None,
        };
        let config = scene.tile_scroll_config(tile_id)?;
        if !self.scroll_state.is_scrollable(tile_id) {
            self.scroll_state.register_tile(
                tile_id,
                ScrollConfig {
                    scrollable_x: config.scrollable_x,
                    scrollable_y: config.scrollable_y,
                    content_width: config.content_width,
                    content_height: config.content_height,
                },
            );
        }

        self.scroll_state
            .apply_user_scroll(tile_id, event.delta_x, event.delta_y)?;

        // Commit only this tile so local user feedback does not consume
        // pending adapter requests for other tiles.
        if !self.scroll_state.commit_tile_frame(tile_id) {
            return None;
        }
        let (offset_x, offset_y) = self.scroll_state.offset(tile_id);
        let _ = scene.set_tile_scroll_offset_local(tile_id, offset_x, offset_y);
        self.scroll_state.changed_event(tile_id)
    }

    /// Process a keyboard-originated scroll event through the runtime-owned scroll path.
    ///
    /// This is the OS keyboard input path for PgUp/PgDn on portal tiles.  It
    /// performs the same hit-test → scroll-offset update as
    /// [`process_scroll_event`], using the **cursor position** to resolve which
    /// tile receives the scroll.  Keyboard scroll uses a fixed page step
    /// ([`KEYBOARD_PAGE_SCROLL_PX`]) per keypress.
    ///
    /// Returns the changed tile and absolute offset when the event updates a
    /// scrollable tile. Returns `None` for passthrough / unscrollable hits.
    ///
    /// # Local-first invariant
    ///
    /// The offset is applied synchronously in < 4ms p99 on the main thread
    /// without waiting for any agent response, matching the wheel-scroll contract.
    pub fn process_keyboard_scroll(
        &mut self,
        cursor_x: f32,
        cursor_y: f32,
        delta_y: f32,
        scene: &mut SceneGraph,
    ) -> Option<ScrollOffsetChangedEvent> {
        self.process_scroll_event(
            &ScrollEvent {
                x: cursor_x,
                y: cursor_y,
                delta_x: 0.0,
                delta_y,
            },
            scene,
        )
    }

    /// Commit queued adapter-driven scroll requests and apply local offsets.
    pub fn commit_scroll_updates(
        &mut self,
        scene: &mut SceneGraph,
    ) -> Vec<ScrollOffsetChangedEvent> {
        let changed = self.scroll_state.commit_all_frames();
        let mut events = Vec::with_capacity(changed.len());
        for tile_id in changed {
            let (offset_x, offset_y) = self.scroll_state.offset(tile_id);
            let _ = scene.set_tile_scroll_offset_local(tile_id, offset_x, offset_y);
            if let Some(ev) = self.scroll_state.changed_event(tile_id) {
                events.push(ev);
            }
        }
        events
    }

    /// Apply an agent rejection for an in-progress interaction.
    ///
    /// Spec §Local Feedback Rollback: "If an agent explicitly rejects an
    /// interaction, the local feedback SHALL be reverted with a 100ms reverse
    /// animation. Rollback SHALL only occur on explicit agent rejection, not on
    /// agent latency or silence."
    ///
    /// Returns a `SceneLocalPatch` containing the rollback state update to
    /// forward to the compositor. The compositor drives the 100ms animation.
    pub fn apply_agent_rejection(
        &mut self,
        node_id: SceneId,
        scene: &mut SceneGraph,
    ) -> SceneLocalPatch {
        // Clear pressed state in the scene graph
        if let Some(state) = scene.hit_region_states.get_mut(&node_id) {
            state.pressed = false;
        }
        // Clear current_press if this node was being tracked
        if let Some((_, pressed_node)) = self.current_press {
            if pressed_node == node_id {
                self.current_press = None;
            }
        }
        // Begin rollback animation tracking
        self.rollback_tracker.begin_rollback(node_id);

        // Produce rollback patch for compositor
        let mut patch = SceneLocalPatch::new();
        patch.push_state(
            LocalStateUpdate::new(node_id)
                .with_pressed(false)
                .with_rollback(),
        );
        patch
    }

    /// Returns a reference to the rollback tracker (e.g. for compositor queries).
    pub fn rollback_tracker(&self) -> &RollbackTracker {
        &self.rollback_tracker
    }

    /// Process a pointer event against the scene graph, applying click-to-focus.
    ///
    /// Updates hit-region local state for immediate visual feedback.
    /// Returns the result including timing measurements and an optional
    /// `AgentDispatch` descriptor for forwarding the event to the owning agent.
    ///
    /// In this variant, click-to-focus is applied **before** the pointer
    /// event is forwarded to the agent, using the provided `focus_manager`
    /// and `tab_id`, per spec §1.2 (lines 27-29). The returned
    /// `FocusTransition` (if any) must be dispatched to agents before the
    /// `AgentDispatch` payload.
    pub fn process_with_focus(
        &mut self,
        event: &PointerEvent,
        scene: &mut SceneGraph,
        focus_manager: &mut FocusManager,
        tab_id: SceneId,
    ) -> (InputResult, Option<FocusTransition>) {
        let focus_transition = if event.kind == PointerEventKind::Down {
            let hit = scene.hit_test(event.x, event.y);
            if let HitResult::NodeHit {
                tile_id, node_id, ..
            } = hit
            {
                let transition = focus_manager.on_click(tab_id, tile_id, Some(node_id), scene);
                // Update focused local state in hit_region_states based on transition.
                // Clear the node that lost focus (if any) and set the one that gained.
                if let Some((lost_ev, _)) = &transition.lost {
                    if let Some(lost_node_id) = lost_ev.node_id {
                        if let Some(state) = scene.hit_region_states.get_mut(&lost_node_id) {
                            state.focused = false;
                        }
                    }
                }
                if let Some((gained_ev, _)) = &transition.gained {
                    if let Some(gained_node_id) = gained_ev.node_id {
                        if let Some(state) = scene.hit_region_states.get_mut(&gained_node_id) {
                            state.focused = true;
                        }
                    }
                }
                if transition.gained.is_some() || transition.lost.is_some() {
                    Some(transition)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let result = self.process(event, scene);
        (result, focus_transition)
    }

    /// Process a pointer event against the scene graph.
    ///
    /// Updates hit-region local state for immediate visual feedback.
    /// Returns the result including timing measurements, an optional
    /// `AgentDispatch` descriptor for forwarding the event to the owning agent,
    /// and a `SceneLocalPatch` to forward to the compositor immediately.
    ///
    /// ## Local feedback contract
    /// The `local_patch` in the returned `InputResult` MUST be forwarded to the
    /// compositor via the dedicated local-patch channel before the next frame.
    /// This ensures `input_to_next_present p99 < 33ms` and satisfies the
    /// doctrinal guarantee: "Local feedback first."
    ///
    /// ## Capture Routing
    ///
    /// If a node holds pointer capture for `event.device_id`, all pointer events
    /// from that device are routed to the capturing node, bypassing normal
    /// hit-testing.  (Spec lines 104-106, 110-111)
    pub fn process(&mut self, event: &PointerEvent, scene: &mut SceneGraph) -> InputResult {
        let start = Instant::now();

        let device_id = event.device_id;

        // ── Capture check: does this device have an active capture? ───────
        // If so, bypass hit-test and route to the capturing node.
        if let Some(capture_state) = self.capture.get(device_id) {
            let (cap_tile_id, cap_node_id) = (capture_state.tile_id, capture_state.node_id);
            let release_on_up = capture_state.release_on_up;

            // Resolve metadata for the capturing node
            let (captured_namespace, captured_interaction_id) =
                resolve_node_meta(scene, cap_tile_id, cap_node_id);
            // Pre-extract interaction_id string before the match may consume it.
            let cap_interaction_id_str = captured_interaction_id.clone().unwrap_or_default();

            let (local_x, local_y) = display_to_local(scene, cap_tile_id, event.x, event.y);
            let hit_test_us = 0; // capture bypasses hit-test

            let mut dispatches: Vec<AgentDispatch> = Vec::new();
            let mut capture_local_patch = SceneLocalPatch::new();

            match event.kind {
                PointerEventKind::Move => {
                    if let Some(ns) = captured_namespace {
                        dispatches.push(AgentDispatch {
                            namespace: ns,
                            tile_id: cap_tile_id,
                            node_id: cap_node_id,
                            interaction_id: captured_interaction_id.unwrap_or_default(),
                            local_x,
                            local_y,
                            display_x: event.x,
                            display_y: event.y,
                            device_id,
                            kind: AgentDispatchKind::PointerMove,
                            capture_released_reason: None,
                        });
                    }
                }
                PointerEventKind::Up => {
                    if release_on_up {
                        // Release capture first, then deliver PointerUp, then CaptureReleased
                        self.capture.release(device_id);

                        if let Some(state) = scene.hit_region_states.get_mut(&cap_node_id) {
                            state.pressed = false;
                        }
                        self.current_press = None;
                        // Emit pressed=false in local patch so compositor updates immediately
                        capture_local_patch
                            .push_state(LocalStateUpdate::new(cap_node_id).with_pressed(false));

                        if let Some(ns) = &captured_namespace {
                            // PointerUp
                            dispatches.push(AgentDispatch {
                                namespace: ns.clone(),
                                tile_id: cap_tile_id,
                                node_id: cap_node_id,
                                interaction_id: captured_interaction_id.clone().unwrap_or_default(),
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                device_id,
                                kind: AgentDispatchKind::PointerUp,
                                capture_released_reason: None,
                            });
                            // CaptureReleased(reason=POINTER_UP)
                            dispatches.push(AgentDispatch {
                                namespace: ns.clone(),
                                tile_id: cap_tile_id,
                                node_id: cap_node_id,
                                interaction_id: captured_interaction_id.unwrap_or_default(),
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                device_id,
                                kind: AgentDispatchKind::CaptureReleased,
                                capture_released_reason: Some(CaptureReleasedReason::PointerUp),
                            });
                        }
                    } else {
                        // Keep capture; just deliver PointerUp
                        if let Some(state) = scene.hit_region_states.get_mut(&cap_node_id) {
                            state.pressed = false;
                        }
                        self.current_press = None;
                        // Emit pressed=false in local patch
                        capture_local_patch
                            .push_state(LocalStateUpdate::new(cap_node_id).with_pressed(false));

                        if let Some(ns) = captured_namespace {
                            dispatches.push(AgentDispatch {
                                namespace: ns,
                                tile_id: cap_tile_id,
                                node_id: cap_node_id,
                                interaction_id: captured_interaction_id.unwrap_or_default(),
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                device_id,
                                kind: AgentDispatchKind::PointerUp,
                                capture_released_reason: None,
                            });
                        }
                    }
                }
                PointerEventKind::Down => {
                    // Capture already active — still deliver PointerDown to capturing node
                    if let Some(state) = scene.hit_region_states.get_mut(&cap_node_id) {
                        state.pressed = true;
                    }
                    // Emit pressed=true in local patch (mirrors non-capture path)
                    capture_local_patch
                        .push_state(LocalStateUpdate::new(cap_node_id).with_pressed(true));
                    if let Some(ns) = captured_namespace {
                        dispatches.push(AgentDispatch {
                            namespace: ns,
                            tile_id: cap_tile_id,
                            node_id: cap_node_id,
                            interaction_id: captured_interaction_id.unwrap_or_default(),
                            local_x,
                            local_y,
                            display_x: event.x,
                            display_y: event.y,
                            device_id,
                            kind: AgentDispatchKind::PointerDown,
                            capture_released_reason: None,
                        });
                    }
                }
            }

            let local_ack_us = start.elapsed().as_micros() as u64;
            let mut dispatches_iter = dispatches.into_iter();
            let dispatch = dispatches_iter.next();
            let extra_dispatches: Vec<AgentDispatch> = dispatches_iter.collect();
            let interaction_id_for_result = if cap_interaction_id_str.is_empty() {
                None
            } else {
                Some(cap_interaction_id_str.clone())
            };

            return InputResult {
                hit: HitResult::NodeHit {
                    tile_id: cap_tile_id,
                    node_id: cap_node_id,
                    interaction_id: cap_interaction_id_str,
                },
                interaction_id: interaction_id_for_result,
                activated: false,
                local_ack_us,
                hit_test_us,
                dispatch,
                extra_dispatches,
                local_patch: capture_local_patch,
            };
        }

        // ── Stage 2: Hit test ─────────────────────────────────────────────
        let hit_start = Instant::now();
        let hit = scene.hit_test(event.x, event.y);
        let hit_test_us = hit_start.elapsed().as_micros() as u64;

        let mut interaction_id: Option<String> = None;
        let mut activated = false;
        let mut dispatch: Option<AgentDispatch> = None;
        // Accumulate local state changes for the SceneLocalPatch
        let mut local_patch = SceneLocalPatch::new();

        // Decompose HitResult into (tile_id, node_id) where applicable.
        let (hit_tile_id, hit_node_id): (Option<SceneId>, Option<SceneId>) = match &hit {
            HitResult::NodeHit {
                tile_id,
                node_id,
                interaction_id: iid,
            } => {
                interaction_id = Some(iid.clone());
                (Some(*tile_id), Some(*node_id))
            }
            HitResult::TileHit { tile_id } => (Some(*tile_id), None),
            HitResult::Chrome { .. } | HitResult::Passthrough => (None, None),
            // ZoneInteraction hits are handled by the zone interaction layer,
            // not by the tile/node dispatch path.  No tile or node ID is associated.
            HitResult::ZoneInteraction {
                interaction_id: iid,
                ..
            } => {
                interaction_id = Some(iid.clone());
                (None, None)
            }
        };
        let hit_drag_handle_id: Option<String> = match &hit {
            HitResult::ZoneInteraction {
                interaction_id: iid,
                kind: ZoneInteractionKind::DragHandle { .. },
                ..
            } => Some(iid.clone()),
            _ => None,
        };

        // ── Stage 2: Update hover state ───────────────────────────────────
        let prev_hover_node = self.current_hover.map(|(_, n)| n);
        let new_hover_node = hit_node_id;

        if prev_hover_node != new_hover_node {
            // Un-hover the old node
            if let Some(old_id) = prev_hover_node {
                if let Some(state) = scene.hit_region_states.get_mut(&old_id) {
                    state.hovered = false;
                }
                // Emit local patch for hover-off
                local_patch.push_state(LocalStateUpdate::new(old_id).with_hovered(false));
                // Dispatch pointer_leave to previous owning agent
                if let Some((old_tile_id, _)) = self.current_hover {
                    if let Some(namespace) = tile_namespace(scene, old_tile_id) {
                        let leave_interaction_id = scene
                            .nodes
                            .get(&old_id)
                            .and_then(|n| {
                                if let NodeData::HitRegion(hr) = &n.data {
                                    Some(hr.interaction_id.clone())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default();
                        dispatch = Some(AgentDispatch {
                            namespace,
                            tile_id: old_tile_id,
                            node_id: old_id,
                            interaction_id: leave_interaction_id,
                            local_x: 0.0,
                            local_y: 0.0,
                            display_x: event.x,
                            display_y: event.y,
                            device_id,
                            kind: AgentDispatchKind::PointerLeave,
                            capture_released_reason: None,
                        });
                    }
                }
            }
            // Hover the new node
            if let Some(new_id) = new_hover_node {
                if let Some(state) = scene.hit_region_states.get_mut(&new_id) {
                    state.hovered = true;
                }
                // Emit local patch for hover-on
                local_patch.push_state(LocalStateUpdate::new(new_id).with_hovered(true));
                // Dispatch pointer_enter to new owning agent (overwrites leave above —
                // enter takes priority; the caller can queue both if needed)
                if let Some(tile_id) = hit_tile_id {
                    if let Some(namespace) = tile_namespace(scene, tile_id) {
                        let (local_x, local_y) = display_to_local(scene, tile_id, event.x, event.y);
                        dispatch = Some(AgentDispatch {
                            namespace,
                            tile_id,
                            node_id: new_id,
                            interaction_id: interaction_id.clone().unwrap_or_default(),
                            local_x,
                            local_y,
                            display_x: event.x,
                            display_y: event.y,
                            device_id,
                            kind: AgentDispatchKind::PointerEnter,
                            capture_released_reason: None,
                        });
                    }
                }
            }
            self.current_hover = hit_tile_id.and_then(|t| hit_node_id.map(|n| (t, n)));
        }

        // ── Stage 2: Update chrome drag-handle hover state ───────────────
        if self.current_drag_handle_hover != hit_drag_handle_id {
            if let Some(old) = self.current_drag_handle_hover.take() {
                scene.set_drag_handle_hovered(&old, false);
            }
            if let Some(new_id) = &hit_drag_handle_id {
                scene.set_drag_handle_hovered(new_id, true);
            }
            self.current_drag_handle_hover = hit_drag_handle_id.clone();
        }

        // ── Stage 2: Handle press/release ─────────────────────────────────
        match event.kind {
            PointerEventKind::Down => {
                if let Some(ref drag_id) = hit_drag_handle_id {
                    scene.set_drag_handle_pressed(drag_id, true);
                    self.current_drag_handle_press = Some(drag_id.clone());
                }
                if let (Some(tile_id), Some(node_id)) = (hit_tile_id, hit_node_id) {
                    if let Some(state) = scene.hit_region_states.get_mut(&node_id) {
                        state.pressed = true;
                    }
                    // Emit local patch for press-on — this is the critical 4ms path
                    local_patch.push_state(LocalStateUpdate::new(node_id).with_pressed(true));
                    self.current_press = Some((tile_id, node_id));

                    // ── Auto-capture: acquire capture automatically if auto_capture=true ─
                    let auto_cap = scene
                        .nodes
                        .get(&node_id)
                        .map(|n| {
                            if let NodeData::HitRegion(hr) = &n.data {
                                hr.auto_capture
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);

                    let release_on_up = scene
                        .nodes
                        .get(&node_id)
                        .map(|n| {
                            if let NodeData::HitRegion(hr) = &n.data {
                                hr.release_on_up
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);

                    if auto_cap {
                        // Try to acquire capture; succeeds unless another node already
                        // holds capture for this device (which shouldn't happen at Down
                        // if the pre-existing capture was already released or routed above).
                        let _ = self
                            .capture
                            .acquire(device_id, tile_id, node_id, release_on_up);
                    }

                    if let Some(namespace) = tile_namespace(scene, tile_id) {
                        let (local_x, local_y) = display_to_local(scene, tile_id, event.x, event.y);
                        dispatch = Some(AgentDispatch {
                            namespace,
                            tile_id,
                            node_id,
                            interaction_id: interaction_id.clone().unwrap_or_default(),
                            local_x,
                            local_y,
                            display_x: event.x,
                            display_y: event.y,
                            device_id,
                            kind: AgentDispatchKind::PointerDown,
                            capture_released_reason: None,
                        });
                    }
                }
            }
            PointerEventKind::Up => {
                if let Some(drag_id) = self.current_drag_handle_press.take() {
                    scene.set_drag_handle_pressed(&drag_id, false);
                }
                if let Some((pressed_tile_id, pressed_node_id)) = self.current_press.take() {
                    if let Some(state) = scene.hit_region_states.get_mut(&pressed_node_id) {
                        state.pressed = false;
                    }
                    // Emit local patch for press-off
                    local_patch
                        .push_state(LocalStateUpdate::new(pressed_node_id).with_pressed(false));
                    // Activation: press and release on the same node
                    if hit_node_id == Some(pressed_node_id) {
                        activated = true;
                        if let Some(namespace) = tile_namespace(scene, pressed_tile_id) {
                            let (local_x, local_y) =
                                display_to_local(scene, pressed_tile_id, event.x, event.y);
                            dispatch = Some(AgentDispatch {
                                namespace,
                                tile_id: pressed_tile_id,
                                node_id: pressed_node_id,
                                interaction_id: interaction_id.clone().unwrap_or_default(),
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                device_id,
                                kind: AgentDispatchKind::Activated,
                                capture_released_reason: None,
                            });
                        }
                    } else {
                        // Released outside the pressed node — dispatch pointer_up
                        if let Some(namespace) = tile_namespace(scene, pressed_tile_id) {
                            let (local_x, local_y) =
                                display_to_local(scene, pressed_tile_id, event.x, event.y);
                            let up_interaction_id = scene
                                .nodes
                                .get(&pressed_node_id)
                                .and_then(|n| {
                                    if let NodeData::HitRegion(hr) = &n.data {
                                        Some(hr.interaction_id.clone())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_default();
                            dispatch = Some(AgentDispatch {
                                namespace,
                                tile_id: pressed_tile_id,
                                node_id: pressed_node_id,
                                interaction_id: up_interaction_id,
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                device_id,
                                kind: AgentDispatchKind::PointerUp,
                                capture_released_reason: None,
                            });
                        }
                    }
                }
            }
            PointerEventKind::Move => {
                // If hovering, dispatch pointer_move (overrides enter/leave set above)
                if let (Some(tile_id), Some(node_id)) = (hit_tile_id, hit_node_id) {
                    if prev_hover_node == new_hover_node {
                        // Already hovering this node — plain move
                        if let Some(namespace) = tile_namespace(scene, tile_id) {
                            let (local_x, local_y) =
                                display_to_local(scene, tile_id, event.x, event.y);
                            dispatch = Some(AgentDispatch {
                                namespace,
                                tile_id,
                                node_id,
                                interaction_id: interaction_id.clone().unwrap_or_default(),
                                local_x,
                                local_y,
                                display_x: event.x,
                                display_y: event.y,
                                device_id,
                                kind: AgentDispatchKind::PointerMove,
                                capture_released_reason: None,
                            });
                        }
                    }
                    // If prev != new (handled above by enter/leave dispatch), no extra move
                }
            }
        }

        let local_ack_us = start.elapsed().as_micros() as u64;

        InputResult {
            hit,
            interaction_id,
            activated,
            local_ack_us,
            hit_test_us,
            dispatch,
            extra_dispatches: vec![],
            local_patch,
        }
    }

    /// Process an explicit capture request from an agent.
    ///
    /// Returns `Ok(AgentDispatch)` with `kind=CaptureGranted` if the capture was
    /// acquired, or `Ok(AgentDispatch)` with `kind=CaptureDenied` if another node
    /// already holds capture for the requested device.
    ///
    /// Per the spec (lines 104-106): capture can ONLY be granted in response to
    /// a PointerDownEvent.  The caller is responsible for only calling this after
    /// confirming a Down event just fired.
    pub fn request_capture(
        &mut self,
        req: &CaptureRequest,
        scene: &SceneGraph,
        release_on_up: bool,
    ) -> Option<AgentDispatch> {
        let namespace = tile_namespace(scene, req.tile_id)?;
        let interaction_id = scene
            .nodes
            .get(&req.node_id)
            .and_then(|n| {
                if let NodeData::HitRegion(hr) = &n.data {
                    Some(hr.interaction_id.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let kind = if self
            .capture
            .acquire(req.device_id, req.tile_id, req.node_id, release_on_up)
            .is_ok()
        {
            AgentDispatchKind::CaptureGranted
        } else {
            AgentDispatchKind::CaptureDenied
        };

        Some(AgentDispatch {
            namespace,
            tile_id: req.tile_id,
            node_id: req.node_id,
            interaction_id,
            local_x: 0.0,
            local_y: 0.0,
            display_x: 0.0,
            display_y: 0.0,
            device_id: req.device_id,
            kind,
            capture_released_reason: None,
        })
    }

    /// Process an explicit capture release request from an agent.
    ///
    /// If the named device has active capture, releases it and returns a
    /// `CaptureReleased(reason=AGENT_RELEASED)` dispatch.  Returns `None` if
    /// there was no active capture for the device.
    pub fn release_capture(
        &mut self,
        req: &CaptureReleaseRequest,
        scene: &SceneGraph,
    ) -> Option<AgentDispatch> {
        let state = self.capture.get(req.device_id)?;
        let (tile_id, node_id) = (state.tile_id, state.node_id);
        let namespace = tile_namespace(scene, tile_id)?;
        let interaction_id = scene
            .nodes
            .get(&node_id)
            .and_then(|n| {
                if let NodeData::HitRegion(hr) = &n.data {
                    Some(hr.interaction_id.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        self.capture.release(req.device_id);

        Some(AgentDispatch {
            namespace,
            tile_id,
            node_id,
            interaction_id,
            local_x: 0.0,
            local_y: 0.0,
            display_x: 0.0,
            display_y: 0.0,
            device_id: req.device_id,
            kind: AgentDispatchKind::CaptureReleased,
            capture_released_reason: Some(CaptureReleasedReason::AgentReleased),
        })
    }

    /// Runtime capture theft (Alt+Tab, system notification, tab switch, lease revocation).
    ///
    /// Per the spec (lines 130-132): sends PointerCancelEvent first, then
    /// CaptureReleasedEvent(reason).  Returns both dispatches in order, or an
    /// empty vec if no capture was active for the device.
    pub fn steal_capture(
        &mut self,
        device_id: u32,
        reason: CaptureReleasedReason,
        scene: &SceneGraph,
    ) -> Vec<AgentDispatch> {
        let state = match self.capture.get(device_id) {
            Some(s) => *s,
            None => return vec![],
        };
        let (tile_id, node_id) = (state.tile_id, state.node_id);
        let namespace = match tile_namespace(scene, tile_id) {
            Some(ns) => ns,
            None => {
                self.capture.release(device_id);
                return vec![];
            }
        };
        let interaction_id = scene
            .nodes
            .get(&node_id)
            .and_then(|n| {
                if let NodeData::HitRegion(hr) = &n.data {
                    Some(hr.interaction_id.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        self.capture.release(device_id);

        vec![
            // 1. PointerCancelEvent — agent MUST treat this as terminal
            AgentDispatch {
                namespace: namespace.clone(),
                tile_id,
                node_id,
                interaction_id: interaction_id.clone(),
                local_x: 0.0,
                local_y: 0.0,
                display_x: 0.0,
                display_y: 0.0,
                device_id,
                kind: AgentDispatchKind::PointerCancel,
                capture_released_reason: None,
            },
            // 2. CaptureReleasedEvent
            AgentDispatch {
                namespace,
                tile_id,
                node_id,
                interaction_id,
                local_x: 0.0,
                local_y: 0.0,
                display_x: 0.0,
                display_y: 0.0,
                device_id,
                kind: AgentDispatchKind::CaptureReleased,
                capture_released_reason: Some(reason),
            },
        ]
    }

    /// Process a pointer event that hit a chrome drag handle.
    ///
    /// This implements the compositor-internal long-press drag state machine
    /// (RFC 0004 §3.0 carve-out). Returns a [`DragEventOutcome`] that the
    /// caller MUST act on:
    ///
    /// - `Accumulating { progress }` — update visual progress on the handle.
    /// - `Activated` — apply visual feedback (z-boost, opacity, 2px border);
    ///   runtime MUST track the element and call `process_drag_move` on
    ///   subsequent `PointerMove` events for the same `device_id`.
    /// - `Moved { element_id, new_x, new_y }` — update the element's bounds.
    /// - `Released { element_id, final_x, final_y }` — persist the geometry.
    /// - `Cancelled` — clear visual feedback.
    /// - `Idle` — no action.
    ///
    /// ## Touch vs pointer thresholds
    ///
    /// `device_id == 0` (primary pointer / mouse) uses the 250 ms threshold.
    /// Any other `device_id` is treated as a touch contact and uses 1000 ms.
    #[allow(clippy::too_many_arguments)]
    pub fn process_drag_handle_pointer(
        &mut self,
        event: &PointerEvent,
        hit_interaction_id: &str,
        element_id: SceneId,
        element_kind: DragHandleElementKind,
        element_bounds: tze_hud_scene::Rect,
        display_width: f32,
        display_height: f32,
    ) -> DragEventOutcome {
        let device_id = event.device_id;

        // Select threshold: primary pointer = 250ms, touch = 1000ms.
        let threshold_ms = if device_id == 0 {
            drag::LONG_PRESS_POINTER_THRESHOLD_MS
        } else {
            drag::LONG_PRESS_TOUCH_THRESHOLD_MS
        };

        match event.kind {
            PointerEventKind::Down => {
                // Start accumulating for this device.
                let state = DeviceDragState::new(
                    hit_interaction_id.to_string(),
                    element_id,
                    element_kind,
                    event.x,
                    event.y,
                    threshold_ms,
                );
                self.drag_states.insert(device_id, state);
                DragEventOutcome::Accumulating { progress: 0.0 }
            }
            PointerEventKind::Move => {
                let Some(state) = self.drag_states.get_mut(&device_id) else {
                    return DragEventOutcome::Idle;
                };
                match state.phase {
                    DragPhase::Idle => DragEventOutcome::Idle,
                    DragPhase::Accumulating => {
                        // Check movement cancellation.
                        if state.has_exceeded_movement_tolerance(event.x, event.y) {
                            let _ = self.drag_states.remove(&device_id);
                            return DragEventOutcome::Cancelled;
                        }
                        // Check threshold.
                        if state.is_threshold_met() {
                            // Activate drag: record grab offset.
                            state.phase = DragPhase::Activated;
                            state.grab_offset_x = event.x - element_bounds.x;
                            state.grab_offset_y = event.y - element_bounds.y;
                            return DragEventOutcome::Activated {
                                element_id: state.element_id,
                                element_kind: state.element_kind,
                            };
                        }
                        // Accumulating — update progress.
                        let progress = state.progress();
                        state.last_progress = progress;
                        DragEventOutcome::Accumulating { progress }
                    }
                    DragPhase::Activated => {
                        // Compute raw new top-left.
                        let raw_x = event.x - state.grab_offset_x;
                        let raw_y = event.y - state.grab_offset_y;
                        let (new_x, new_y) = drag::quantise_and_clamp(
                            raw_x,
                            raw_y,
                            element_bounds.width,
                            element_bounds.height,
                            display_width,
                            display_height,
                            self.drag_config.snap_grid_pct,
                        );
                        DragEventOutcome::Moved {
                            element_id: state.element_id,
                            element_kind: state.element_kind,
                            new_x,
                            new_y,
                        }
                    }
                }
            }
            PointerEventKind::Up => {
                let Some(state) = self.drag_states.remove(&device_id) else {
                    return DragEventOutcome::Idle;
                };
                match state.phase {
                    DragPhase::Idle | DragPhase::Accumulating => {
                        // Short press (tap) — not a drag; no geometry change.
                        DragEventOutcome::Cancelled
                    }
                    DragPhase::Activated => {
                        let raw_x = event.x - state.grab_offset_x;
                        let raw_y = event.y - state.grab_offset_y;
                        let (final_x, final_y) = drag::quantise_and_clamp(
                            raw_x,
                            raw_y,
                            element_bounds.width,
                            element_bounds.height,
                            display_width,
                            display_height,
                            self.drag_config.snap_grid_pct,
                        );
                        DragEventOutcome::Released {
                            element_id: state.element_id,
                            element_kind: state.element_kind,
                            final_x,
                            final_y,
                        }
                    }
                }
            }
        }
    }

    /// Check the long-press progress for a device during `Accumulating` phase.
    ///
    /// Returns the progress value (0.0–1.0) and whether the threshold has been
    /// met. Returns `None` if there is no drag state for the device.
    ///
    /// Called by the compositor or runtime loop to poll accumulation state
    /// (e.g. to drive a progress indicator on the drag handle).
    pub fn drag_accumulation_progress(&self, device_id: u32) -> Option<f32> {
        let state = self.drag_states.get(&device_id)?;
        if state.phase == DragPhase::Accumulating {
            Some(state.progress())
        } else {
            None
        }
    }

    /// Returns the element_id and element_kind of the currently active drag for
    /// a given device, or `None` if no drag is active.
    ///
    /// The compositor uses this to decide whether to apply visual feedback
    /// (z-order boost, opacity, 2px highlight border) to the element.
    pub fn active_drag_element(&self, device_id: u32) -> Option<(SceneId, DragHandleElementKind)> {
        let state = self.drag_states.get(&device_id)?;
        if state.phase == DragPhase::Activated {
            Some((state.element_id, state.element_kind))
        } else {
            None
        }
    }

    /// Persist the final drag geometry to the element store.
    ///
    /// This is a thin wrapper around [`drag::persist_geometry_override`] for
    /// callers that have already resolved the `interaction_key` and know the
    /// element type.
    ///
    /// The caller (windowed/headless runtime) MUST call this on
    /// [`DragEventOutcome::Released`], then atomically save the element store
    /// to disk.
    #[allow(clippy::too_many_arguments)]
    pub fn persist_drag_geometry(
        store: &mut ElementStore,
        element_type: ElementType,
        interaction_key: &str,
        final_x: f32,
        final_y: f32,
        width: f32,
        height: f32,
        display_width: f32,
        display_height: f32,
    ) {
        let geometry = drag::final_position_to_geometry(
            final_x,
            final_y,
            width,
            height,
            display_width,
            display_height,
        );
        drag::persist_geometry_override(store, element_type, interaction_key, geometry);
    }
}

impl Default for InputProcessor {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Get the namespace (agent name) of the tile owner, or None if tile not found.
fn tile_namespace(scene: &SceneGraph, tile_id: SceneId) -> Option<String> {
    scene.tiles.get(&tile_id).map(|t| t.namespace.clone())
}

/// Convert display-space coordinates to tile-local coordinates.
fn display_to_local(scene: &SceneGraph, tile_id: SceneId, x: f32, y: f32) -> (f32, f32) {
    if let Some(tile) = scene.tiles.get(&tile_id) {
        (x - tile.bounds.x, y - tile.bounds.y)
    } else {
        (x, y)
    }
}

/// Resolve the namespace and interaction_id for a tile/node pair.
fn resolve_node_meta(
    scene: &SceneGraph,
    tile_id: SceneId,
    node_id: SceneId,
) -> (Option<String>, Option<String>) {
    let namespace = tile_namespace(scene, tile_id);
    let interaction_id = scene.nodes.get(&node_id).and_then(|n| {
        if let NodeData::HitRegion(hr) = &n.data {
            Some(hr.interaction_id.clone())
        } else {
            None
        }
    });
    (namespace, interaction_id)
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::*;

    fn setup_scene_with_hit_region() -> (SceneGraph, SceneId, SceneId) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        let tile_id = scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();

        let hr_node_id = SceneId::new();
        let hit_node = Node {
            id: hr_node_id,
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(50.0, 50.0, 200.0, 100.0),
                interaction_id: "test-button".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };
        scene.set_tile_root(tile_id, hit_node).unwrap();

        (scene, tile_id, hr_node_id)
    }

    fn setup_scrollable_scene() -> (SceneGraph, SceneId) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(100.0, 100.0, 400.0, 300.0),
                1,
            )
            .unwrap();
        scene
            .register_tile_scroll_config(tile_id, tze_hud_scene::TileScrollConfig::vertical())
            .unwrap();
        (scene, tile_id)
    }

    #[test]
    fn test_process_scroll_event_updates_scene_local_offset() {
        let (mut scene, tile_id) = setup_scrollable_scene();
        let mut processor = InputProcessor::new();

        let changed = processor.process_scroll_event(
            &ScrollEvent {
                x: 150.0,
                y: 150.0,
                delta_x: 0.0,
                delta_y: 24.0,
            },
            &mut scene,
        );

        assert!(changed.is_some(), "scroll on a configured tile must update");
        let (_, offset_y) = scene.tile_scroll_offset_local(tile_id);
        assert!(
            (offset_y - 24.0).abs() < f32::EPSILON,
            "expected local offset_y=24.0, got {offset_y}"
        );
    }

    #[test]
    fn test_user_scroll_is_authoritative_over_queued_adapter_offset() {
        let (mut scene, tile_id) = setup_scrollable_scene();
        let mut processor = InputProcessor::new();

        processor.queue_set_scroll_offset(SetScrollOffsetRequest {
            tile_id,
            offset_x: 0.0,
            offset_y: 200.0,
        });

        let changed = processor.process_scroll_event(
            &ScrollEvent {
                x: 150.0,
                y: 150.0,
                delta_x: 0.0,
                delta_y: 18.0,
            },
            &mut scene,
        );

        assert!(changed.is_some(), "user scroll should still update offset");
        let (_, offset_y) = scene.tile_scroll_offset_local(tile_id);
        assert!(
            (offset_y - 18.0).abs() < f32::EPSILON,
            "user scroll must win over queued adapter request, got {offset_y}"
        );
    }

    #[test]
    fn test_hover_state_updates() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Move pointer over hit region (tile at 100,100; region at 50,50 within)
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(result.hit.is_some());
        assert_eq!(result.interaction_id, Some("test-button".to_string()));
        assert!(scene.hit_region_states[&hr_node_id].hovered);

        // Move pointer away
        let result = processor.process(
            &PointerEvent {
                x: 10.0,
                y: 10.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(result.hit.is_none());
        assert!(!scene.hit_region_states[&hr_node_id].hovered);
    }

    #[test]
    fn test_press_and_activate() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Press on hit region
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(scene.hit_region_states[&hr_node_id].pressed);
        assert!(!result.activated);

        // Release on hit region — should activate
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Up,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(!scene.hit_region_states[&hr_node_id].pressed);
        assert!(result.activated);
        assert_eq!(result.interaction_id, Some("test-button".to_string()));
    }

    #[test]
    fn test_local_ack_under_4ms() {
        use tze_hud_scene::calibration::{budgets, test_budget};

        let (mut scene, _, _) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        let ack_budget = test_budget(budgets::INPUT_ACK_BUDGET_US);
        let hit_budget = test_budget(budgets::HIT_TEST_BUDGET_US);

        // local_ack should be within calibrated budget (hardware-normalized)
        assert!(
            result.local_ack_us < ack_budget,
            "local_ack_us was {}us, calibrated budget is {}us (base: {}us)",
            result.local_ack_us,
            ack_budget,
            budgets::INPUT_ACK_BUDGET_US,
        );
        // hit_test should be within calibrated budget
        assert!(
            result.hit_test_us < hit_budget,
            "hit_test_us was {}us, calibrated budget is {}us (base: {}us)",
            result.hit_test_us,
            hit_budget,
            budgets::HIT_TEST_BUDGET_US,
        );
    }

    // ── New tests for AgentDispatch ─────────────────────────────────────

    #[test]
    fn test_dispatch_pointer_enter_on_first_hover() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // First move onto the hit region — should produce PointerEnter dispatch
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("expected AgentDispatch on enter");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerEnter);
        assert_eq!(dispatch.namespace, "test");
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert_eq!(dispatch.interaction_id, "test-button");
        // local coords: display(200,180) - tile_origin(100,100) = (100,80)
        assert!((dispatch.local_x - 100.0).abs() < 0.01);
        assert!((dispatch.local_y - 80.0).abs() < 0.01);
    }

    #[test]
    fn test_dispatch_pointer_leave_on_exit() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Enter
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        // Leave
        let result = processor.process(
            &PointerEvent {
                x: 10.0,
                y: 10.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("expected AgentDispatch on leave");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerLeave);
        assert_eq!(dispatch.namespace, "test");
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert_eq!(dispatch.interaction_id, "test-button");
    }

    #[test]
    fn test_dispatch_pointer_down() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("expected AgentDispatch on down");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerDown);
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
    }

    #[test]
    fn test_dispatch_activated_on_press_release() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Down
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        // Up on same node — Activated
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Up,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(result.activated);
        let dispatch = result
            .dispatch
            .expect("expected AgentDispatch on activation");
        assert_eq!(dispatch.kind, AgentDispatchKind::Activated);
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert_eq!(dispatch.interaction_id, "test-button");
    }

    #[test]
    fn test_dispatch_pointer_up_outside_pressed_node() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Press inside hit region
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        // Release outside — PointerUp (not Activated)
        let result = processor.process(
            &PointerEvent {
                x: 10.0,
                y: 10.0,
                kind: PointerEventKind::Up,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(!result.activated);
        let dispatch = result
            .dispatch
            .expect("expected AgentDispatch on up-outside");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerUp);
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert_eq!(dispatch.interaction_id, "test-button");
    }

    #[test]
    fn test_no_dispatch_when_no_hit() {
        let (mut scene, _, _) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        let result = processor.process(
            &PointerEvent {
                x: 5.0,
                y: 5.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(result.hit.is_none());
        assert!(result.dispatch.is_none());
    }

    #[test]
    fn test_dispatch_move_while_hovering() {
        let (mut scene, tile_id, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Enter
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        // Move within the same hit region — PointerMove
        let result = processor.process(
            &PointerEvent {
                x: 210.0,
                y: 185.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        let dispatch = result.dispatch.expect("expected AgentDispatch on move");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerMove);
        assert_eq!(dispatch.tile_id, tile_id);
        assert_eq!(dispatch.node_id, hr_node_id);
        assert!((dispatch.local_x - 110.0).abs() < 0.01);
        assert!((dispatch.local_y - 85.0).abs() < 0.01);
    }

    // ── SceneLocalPatch integration tests ────────────────────────────────

    #[test]
    fn test_local_patch_produced_on_pointer_down() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        // SceneLocalPatch must contain a pressed=true update for the hit node
        assert!(
            !result.local_patch.is_empty(),
            "local_patch should not be empty after Down"
        );
        let pressed_update = result
            .local_patch
            .node_updates
            .iter()
            .find(|u| u.node_id == hr_node_id && u.pressed.is_some())
            .expect("expected pressed state update for hr_node_id");
        assert_eq!(pressed_update.pressed, Some(true));
        assert!(!pressed_update.rollback);
    }

    #[test]
    fn test_local_patch_produced_on_pointer_up() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Press first
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        // Up
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Up,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(
            !result.local_patch.is_empty(),
            "local_patch should not be empty after Up"
        );
        let state_update = result
            .local_patch
            .node_updates
            .iter()
            .find(|u| u.node_id == hr_node_id)
            .expect("expected state update for hr_node_id");
        assert_eq!(state_update.pressed, Some(false));
    }

    #[test]
    fn test_local_patch_hover_on_enter() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(
            !result.local_patch.is_empty(),
            "local_patch should contain hover update"
        );
        let state_update = result
            .local_patch
            .node_updates
            .iter()
            .find(|u| u.node_id == hr_node_id)
            .expect("expected state update for hr_node_id");
        assert_eq!(state_update.hovered, Some(true));
    }

    #[test]
    fn test_local_patch_hover_off_on_leave() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Enter
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        // Leave
        let result = processor.process(
            &PointerEvent {
                x: 5.0,
                y: 5.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(
            !result.local_patch.is_empty(),
            "local_patch should contain hover-off update"
        );
        let state_update = result
            .local_patch
            .node_updates
            .iter()
            .find(|u| u.node_id == hr_node_id)
            .expect("expected state update for hr_node_id");
        assert_eq!(state_update.hovered, Some(false));
    }

    #[test]
    fn test_local_patch_empty_when_no_state_change() {
        let (mut scene, _, _) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Move in empty space — no hit, no state change
        let result = processor.process(
            &PointerEvent {
                x: 5.0,
                y: 5.0,
                kind: PointerEventKind::Move,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(
            result.local_patch.is_empty(),
            "no state changed, patch should be empty"
        );
    }

    #[test]
    fn test_apply_agent_rejection_produces_rollback_patch() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Press to set up pressed state
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );
        assert!(scene.hit_region_states[&hr_node_id].pressed);

        // Agent rejects the interaction
        let rollback_patch = processor.apply_agent_rejection(hr_node_id, &mut scene);

        // Pressed state cleared in scene graph immediately
        assert!(!scene.hit_region_states[&hr_node_id].pressed);

        // Patch contains rollback=true state update
        assert!(!rollback_patch.is_empty());
        let update = rollback_patch
            .node_updates
            .iter()
            .find(|u| u.node_id == hr_node_id)
            .expect("expected rollback state update");
        assert_eq!(update.pressed, Some(false));
        assert!(update.rollback, "rollback flag must be set");

        // Rollback tracker should record the animation
        assert!(processor.rollback_tracker().is_rolling_back(hr_node_id));
    }

    #[test]
    fn test_agent_silence_does_not_rollback() {
        let (mut scene, _, hr_node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // Press — starts pressed
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id: 0,
                timestamp: None,
            },
            &mut scene,
        );
        assert!(scene.hit_region_states[&hr_node_id].pressed);

        // Agent does NOT respond (silence) — pressed remains true
        // (no apply_agent_rejection called)
        assert!(
            scene.hit_region_states[&hr_node_id].pressed,
            "pressed should remain true on agent silence per spec"
        );
        assert!(
            !processor.rollback_tracker().is_rolling_back(hr_node_id),
            "rollback should NOT be triggered by agent silence"
        );
    }

    // ── Pointer Capture Protocol Tests ─────────────────────────────────
    // These tests cover the acceptance scenarios from issue rig-vzf0.

    /// Helper: build a scene with TWO tiles, each with a hit region.
    /// Tile T1: bounds (100,100,300,200), hit region (0,0,300,200) — "node-t1"
    /// Tile T2: bounds (500,100,300,200), hit region (0,0,300,200) — "node-t2"
    fn setup_two_tile_scene() -> (SceneGraph, SceneId, SceneId, SceneId, SceneId) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease1 = scene.grant_lease("agent1", 60_000, vec![Capability::CreateTile]);
        let lease2 = scene.grant_lease("agent2", 60_000, vec![Capability::CreateTile]);

        let t1 = scene
            .create_tile(
                tab_id,
                "agent1",
                lease1,
                Rect::new(100.0, 100.0, 300.0, 200.0),
                1,
            )
            .unwrap();
        let n1 = SceneId::new();
        scene
            .set_tile_root(
                t1,
                Node {
                    id: n1,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 300.0, 200.0),
                        interaction_id: "node-t1".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let t2 = scene
            .create_tile(
                tab_id,
                "agent2",
                lease2,
                Rect::new(500.0, 100.0, 300.0, 200.0),
                2,
            )
            .unwrap();
        let n2 = SceneId::new();
        scene
            .set_tile_root(
                t2,
                Node {
                    id: n2,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 300.0, 200.0),
                        interaction_id: "node-t2".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        (scene, t1, n1, t2, n2)
    }

    /// WHEN node N1 in tile T1 holds pointer capture for device D1 and the pointer
    /// moves outside T1's bounds THEN all pointer events from D1 are routed to N1
    /// regardless of which tile the pointer is visually over. (spec line 110)
    #[test]
    fn test_capture_routes_events_outside_capturing_tile_bounds() {
        let (mut scene, t1, n1, _t2, _n2) = setup_two_tile_scene();
        let mut processor = InputProcessor::new();
        let device_id = 0u32;

        // Press on T1 to establish press state
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Down,
                device_id,
                timestamp: None,
            },
            &mut scene,
        );

        // Acquire capture for N1/T1 for device 0
        let req = CaptureRequest {
            tile_id: t1,
            node_id: n1,
            device_id,
        };
        let response = processor.request_capture(&req, &scene, false).unwrap();
        assert_eq!(response.kind, AgentDispatchKind::CaptureGranted);
        assert!(processor.capture.is_captured(device_id));

        // Move pointer to T2's territory (x=600, which is inside T2 bounds at 500-800)
        let result = processor.process(
            &PointerEvent {
                x: 600.0,
                y: 180.0,
                kind: PointerEventKind::Move,
                device_id,
                timestamp: None,
            },
            &mut scene,
        );

        // Event MUST be routed to N1 (T1's owner), not T2
        let dispatch = result.dispatch.expect("should dispatch during capture");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerMove);
        assert_eq!(
            dispatch.tile_id, t1,
            "captured events must route to capturing tile"
        );
        assert_eq!(
            dispatch.node_id, n1,
            "captured events must route to capturing node"
        );
        assert_eq!(dispatch.namespace, "agent1");
    }

    /// WHEN node N1 holds capture for device D1 and node N2 requests capture for
    /// the same device D1 THEN the runtime responds with CaptureResponse(result=DENIED).
    /// (spec line 114)
    #[test]
    fn test_capture_denied_when_device_already_captured() {
        let (scene, t1, n1, t2, n2) = setup_two_tile_scene();
        let mut processor = InputProcessor::new();
        let device_id = 0u32;

        // N1 acquires capture
        let req1 = CaptureRequest {
            tile_id: t1,
            node_id: n1,
            device_id,
        };
        let response1 = processor.request_capture(&req1, &scene, false).unwrap();
        assert_eq!(response1.kind, AgentDispatchKind::CaptureGranted);

        // N2 tries to capture the same device
        let req2 = CaptureRequest {
            tile_id: t2,
            node_id: n2,
            device_id,
        };
        let response2 = processor.request_capture(&req2, &scene, false).unwrap();
        assert_eq!(
            response2.kind,
            AgentDispatchKind::CaptureDenied,
            "second capture request for same device must be denied"
        );

        // N1 still holds capture
        let state = processor.capture.get(device_id).unwrap();
        assert_eq!(state.node_id, n1, "original capture must remain");
    }

    /// WHEN a node with release_on_up=true holds capture and a PointerUpEvent arrives
    /// for the captured device THEN capture is released and
    /// CaptureReleasedEvent(reason=POINTER_UP) is dispatched. (spec line 125)
    #[test]
    fn test_capture_released_on_pointer_up_when_release_on_up_true() {
        let (mut scene, t1, n1, _t2, _n2) = setup_two_tile_scene();
        let mut processor = InputProcessor::new();
        let device_id = 0u32;

        // Acquire capture with release_on_up=true
        let req = CaptureRequest {
            tile_id: t1,
            node_id: n1,
            device_id,
        };
        let response = processor.request_capture(&req, &scene, true).unwrap();
        assert_eq!(response.kind, AgentDispatchKind::CaptureGranted);
        assert!(processor.capture.get(device_id).unwrap().release_on_up);

        // Send PointerUp — should release capture
        let result = processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Up,
                device_id,
                timestamp: None,
            },
            &mut scene,
        );

        // Capture should be released
        assert!(
            !processor.capture.is_captured(device_id),
            "capture must be released on PointerUp"
        );

        // The primary dispatch returned should be PointerUp
        let dispatch = result.dispatch.expect("should dispatch on up");
        assert_eq!(dispatch.kind, AgentDispatchKind::PointerUp);
        assert_eq!(dispatch.tile_id, t1);

        // The secondary dispatch (extra_dispatches) must contain CaptureReleased(POINTER_UP)
        // per spec line 125: "CaptureReleasedEvent(reason=POINTER_UP) SHALL be dispatched"
        assert_eq!(
            result.extra_dispatches.len(),
            1,
            "CaptureReleased must be delivered as extra_dispatch after PointerUp"
        );
        let cap_released = &result.extra_dispatches[0];
        assert_eq!(cap_released.kind, AgentDispatchKind::CaptureReleased);
        assert_eq!(
            cap_released.capture_released_reason,
            Some(CaptureReleasedReason::PointerUp)
        );
        assert_eq!(cap_released.tile_id, t1);
    }

    /// WHEN a node holds pointer capture and the user presses Alt+Tab THEN the runtime
    /// sends PointerCancelEvent to the capturing node, followed by
    /// CaptureReleasedEvent(reason=RUNTIME_REVOKED). (spec line 136)
    #[test]
    fn test_capture_theft_sends_cancel_then_released() {
        let (scene, t1, n1, _t2, _n2) = setup_two_tile_scene();
        let mut processor = InputProcessor::new();
        let device_id = 0u32;

        // Acquire capture
        let req = CaptureRequest {
            tile_id: t1,
            node_id: n1,
            device_id,
        };
        processor.request_capture(&req, &scene, false).unwrap();

        // Runtime steals capture (Alt+Tab scenario)
        let dispatches =
            processor.steal_capture(device_id, CaptureReleasedReason::RuntimeRevoked, &scene);

        assert_eq!(
            dispatches.len(),
            2,
            "theft must produce exactly 2 dispatches"
        );

        // First: PointerCancelEvent
        assert_eq!(
            dispatches[0].kind,
            AgentDispatchKind::PointerCancel,
            "first dispatch must be PointerCancelEvent"
        );
        assert_eq!(dispatches[0].tile_id, t1);
        assert_eq!(dispatches[0].node_id, n1);

        // Second: CaptureReleasedEvent(reason=RUNTIME_REVOKED)
        assert_eq!(
            dispatches[1].kind,
            AgentDispatchKind::CaptureReleased,
            "second dispatch must be CaptureReleasedEvent"
        );
        assert_eq!(
            dispatches[1].capture_released_reason,
            Some(CaptureReleasedReason::RuntimeRevoked),
        );

        // Capture is cleared
        assert!(!processor.capture.is_captured(device_id));
    }

    /// WHEN PointerDownEvent hits a HitRegionNode with auto_capture=true THEN the
    /// runtime acquires capture for that node and device without the agent sending
    /// CaptureRequest. (spec line 147)
    #[test]
    fn test_auto_capture_on_pointer_down() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease = scene.grant_lease("agent1", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent1",
                lease,
                Rect::new(100.0, 100.0, 300.0, 200.0),
                1,
            )
            .unwrap();
        let node_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: node_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 300.0, 200.0),
                        interaction_id: "auto-cap-button".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        auto_capture: true,
                        release_on_up: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let mut processor = InputProcessor::new();
        let device_id = 0u32;

        // Before: no capture
        assert!(!processor.capture.is_captured(device_id));

        // PointerDown on auto_capture=true node
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 200.0,
                kind: PointerEventKind::Down,
                device_id,
                timestamp: None,
            },
            &mut scene,
        );

        // Capture must be automatically acquired
        assert!(
            processor.capture.is_captured(device_id),
            "auto_capture=true must acquire capture on PointerDown without explicit request"
        );
        let state = processor.capture.get(device_id).unwrap();
        assert_eq!(state.tile_id, tile_id);
        assert_eq!(state.node_id, node_id);
        assert!(
            state.release_on_up,
            "release_on_up from HitRegionNode must be preserved"
        );
    }

    /// WHEN node holds capture and agent sends explicit CaptureReleaseRequest THEN
    /// capture is released with reason=AGENT_RELEASED. (spec line 120)
    #[test]
    fn test_explicit_capture_release_by_agent() {
        let (scene, t1, n1, _t2, _n2) = setup_two_tile_scene();
        let mut processor = InputProcessor::new();
        let device_id = 0u32;

        // Acquire capture
        let req = CaptureRequest {
            tile_id: t1,
            node_id: n1,
            device_id,
        };
        processor.request_capture(&req, &scene, false).unwrap();

        // Agent releases explicitly
        let release_req = CaptureReleaseRequest { device_id };
        let dispatch = processor
            .release_capture(&release_req, &scene)
            .expect("release must produce dispatch when capture was active");

        assert_eq!(dispatch.kind, AgentDispatchKind::CaptureReleased);
        assert_eq!(
            dispatch.capture_released_reason,
            Some(CaptureReleasedReason::AgentReleased)
        );
        assert_eq!(dispatch.tile_id, t1);
        assert_eq!(dispatch.node_id, n1);
        assert!(!processor.capture.is_captured(device_id));
    }

    /// WHEN capture is NOT active and agent sends CaptureReleaseRequest THEN
    /// release_capture returns None (no spurious events).
    #[test]
    fn test_release_when_no_capture_returns_none() {
        let (scene, _t1, _n1, _t2, _n2) = setup_two_tile_scene();
        let mut processor = InputProcessor::new();
        let result = processor.release_capture(&CaptureReleaseRequest { device_id: 0 }, &scene);
        assert!(
            result.is_none(),
            "releasing when no capture must return None"
        );
    }

    /// WHEN two different devices are captured by different nodes THEN releasing one
    /// does NOT affect the other.
    #[test]
    fn test_two_devices_captured_independently() {
        let (_scene, t1, n1, t2, n2) = setup_two_tile_scene();
        let mut processor = InputProcessor::new();

        processor.capture.acquire(0, t1, n1, false).unwrap();
        processor.capture.acquire(1, t2, n2, true).unwrap();

        assert_eq!(processor.capture.active_count(), 2);

        processor.capture.release(0);
        assert!(!processor.capture.is_captured(0));
        assert!(
            processor.capture.is_captured(1),
            "releasing device 0 must not affect device 1"
        );
    }

    /// PointerUp with release_on_up=FALSE keeps capture active.
    #[test]
    fn test_capture_not_released_on_up_when_flag_false() {
        let (mut scene, t1, n1, _t2, _n2) = setup_two_tile_scene();
        let mut processor = InputProcessor::new();
        let device_id = 0u32;

        // Acquire with release_on_up=false
        let req = CaptureRequest {
            tile_id: t1,
            node_id: n1,
            device_id,
        };
        processor.request_capture(&req, &scene, false).unwrap();

        // Send PointerUp — capture should remain
        processor.process(
            &PointerEvent {
                x: 200.0,
                y: 180.0,
                kind: PointerEventKind::Up,
                device_id,
                timestamp: None,
            },
            &mut scene,
        );

        assert!(
            processor.capture.is_captured(device_id),
            "capture must remain when release_on_up=false"
        );
    }

    // ── Keyboard scroll (hud-6bbe) ────────────────────────────────────────────

    /// PgDn (positive delta_y) scrolls down on a scrollable portal tile.
    #[test]
    fn test_process_keyboard_scroll_pgdn_scrolls_down() {
        let (mut scene, tile_id) = setup_scrollable_scene();
        let mut processor = InputProcessor::new();

        // Cursor is inside the tile (tile bounds: 100,100 → 500,400).
        let ev = processor.process_keyboard_scroll(150.0, 150.0, KEYBOARD_PAGE_SCROLL_PX, &mut scene);

        assert!(ev.is_some(), "PgDn must return a changed event for a scrollable tile");
        let (_, offset_y) = scene.tile_scroll_offset_local(tile_id);
        assert!(
            (offset_y - KEYBOARD_PAGE_SCROLL_PX).abs() < 1e-4,
            "offset_y must equal KEYBOARD_PAGE_SCROLL_PX after PgDn; got {offset_y}"
        );
    }

    /// PgUp (negative delta_y) scrolls up — clamped to 0 when already at top.
    #[test]
    fn test_process_keyboard_scroll_pgup_clamped_at_zero() {
        let (mut scene, _tile_id) = setup_scrollable_scene();
        let mut processor = InputProcessor::new();

        // Starting at offset 0, PgUp should clamp to 0 (no-op on offset).
        let ev = processor.process_keyboard_scroll(150.0, 150.0, -KEYBOARD_PAGE_SCROLL_PX, &mut scene);

        // Scroll event fires (tile is scrollable and hit), but offset stays 0.
        // queue_user_scroll sets dirty=true before clamping, so commit_frame returns
        // true and process_scroll_event returns Some even when the clamped offset is
        // unchanged.  The return value is not load-bearing for correctness here.
        let (_, offset_y) = scene.tile_scroll_offset_local(_tile_id);
        assert_eq!(
            offset_y, 0.0,
            "PgUp at zero offset must result in exactly 0.0 scroll offset; got {offset_y}"
        );
        let _ = ev; // return value is Some (dirty flag set before clamp) — not asserted
    }

    /// PgUp after PgDn returns toward origin.
    #[test]
    fn test_process_keyboard_scroll_pgdn_then_pgup() {
        let (mut scene, tile_id) = setup_scrollable_scene();
        let mut processor = InputProcessor::new();

        // Scroll down by two page steps.
        processor.process_keyboard_scroll(150.0, 150.0, KEYBOARD_PAGE_SCROLL_PX, &mut scene);
        processor.process_keyboard_scroll(150.0, 150.0, KEYBOARD_PAGE_SCROLL_PX, &mut scene);

        let (_, before) = scene.tile_scroll_offset_local(tile_id);
        assert!((before - 2.0 * KEYBOARD_PAGE_SCROLL_PX).abs() < 1e-4,
            "expected 2x page scroll; got {before}");

        // Scroll back up by one page step.
        processor.process_keyboard_scroll(150.0, 150.0, -KEYBOARD_PAGE_SCROLL_PX, &mut scene);

        let (_, after) = scene.tile_scroll_offset_local(tile_id);
        assert!(
            (after - KEYBOARD_PAGE_SCROLL_PX).abs() < 1e-4,
            "one PgUp from 2x should leave offset at 1x page; got {after}"
        );
    }

    /// Keyboard scroll outside all tiles returns None (passthrough).
    #[test]
    fn test_process_keyboard_scroll_no_tile_hit_returns_none() {
        let (mut scene, _tile_id) = setup_scrollable_scene();
        let mut processor = InputProcessor::new();

        // Cursor far outside the tile (tile: 100,100 → 500,400).
        let ev = processor.process_keyboard_scroll(10.0, 10.0, KEYBOARD_PAGE_SCROLL_PX, &mut scene);
        assert!(ev.is_none(), "keyboard scroll outside tile bounds must return None");
    }

    /// Keyboard scroll on a non-scrollable tile returns None.
    #[test]
    fn test_process_keyboard_scroll_non_scrollable_tile_returns_none() {
        let (mut scene, _tile_id, _node_id) = setup_scene_with_hit_region();
        let mut processor = InputProcessor::new();

        // The scene_with_hit_region tile has no scroll config registered.
        let ev = processor.process_keyboard_scroll(200.0, 180.0, KEYBOARD_PAGE_SCROLL_PX, &mut scene);
        assert!(ev.is_none(), "keyboard scroll on a non-scrollable tile must return None");
    }

    /// Keyboard scroll coalesces with wheel scroll under the same frame — user wins.
    #[test]
    fn test_keyboard_scroll_coalesces_like_wheel_scroll() {
        let (mut scene, tile_id) = setup_scrollable_scene();
        let mut processor = InputProcessor::new();

        // Wheel scroll first.
        processor.process_scroll_event(
            &ScrollEvent { x: 150.0, y: 150.0, delta_x: 0.0, delta_y: 40.0 },
            &mut scene,
        );
        // Then keyboard scroll (same frame — no commit between).
        processor.process_keyboard_scroll(150.0, 150.0, KEYBOARD_PAGE_SCROLL_PX, &mut scene);

        let (_, offset_y) = scene.tile_scroll_offset_local(tile_id);
        let expected = 40.0 + KEYBOARD_PAGE_SCROLL_PX;
        assert!(
            (offset_y - expected).abs() < 1e-4,
            "combined wheel + keyboard scroll must accumulate; expected {expected}, got {offset_y}"
        );
    }
}
