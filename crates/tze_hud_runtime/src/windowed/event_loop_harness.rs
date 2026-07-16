//! # event_loop_harness
//!
//! Headless test harness that drives the runtime's real event/window
//! state-machine — the code production runs inside winit's
//! [`ApplicationHandler`] — with synthetic events, WITHOUT constructing a real
//! `winit` window, a `wgpu` surface, or running the OS event loop (hud-nu0ea).
//!
//! ## Why this exists
//!
//! Several keyboard-drain tests historically had to *reconstruct* the
//! production drain loop (a hand-written `for _ in 0..limit` over a local
//! `VecDeque`, calling `InputProcessor::route_character_to_composer` directly)
//! because there was no way to construct a [`WinitApp`] without a live window /
//! GPU. That reconstruction can silently drift from the real
//! [`WinitApp::drain_pending_keyboard_events`] path (active-tab resolution →
//! inner-fn dispatch → [`restore_front_requeued_event`]).
//!
//! This harness closes that gap. The runtime's event/window state machine is
//! already decoupled from winit at the method boundary: the drain and the
//! `dispatch_*_event_inner` fns are methods on [`WinitApp`] that take the
//! runtime's *own* `PendingKeyboardEvent` type (never winit event types) and
//! never touch `ActiveEventLoop`. The only thing that previously blocked a test
//! from driving them was the inability to build a [`WindowedRuntimeState`]. The
//! [`WindowedRuntimeState::new_headless`] constructor below supplies an inert
//! but real state (no window, no GPU, no network servers), and
//! [`HeadlessEventLoopHarness`] wraps a real [`WinitApp`] around it so tests can
//! inject synthetic keyboard events and run the genuine production dispatch.
//!
//! ## Scope
//!
//! This is deliberately NOT an attempt to run a real headless `WinitApp` with a
//! GPU — that is the `cargo test -p tze_hud_compositor` llvmpipe pixel-readback
//! deadlock this harness is meant to avoid. It drives only the parts of the
//! state machine that are GPU-independent (input/keyboard dispatch, portal/
//! composer routing over the shared scene). The window, surface, and compositor
//! fields are left `None`.

use std::sync::Arc;

use tze_hud_input::{
    FocusManager, InputProcessor, KeyboardProcessor, PointerEvent, PointerEventKind,
};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::HitRegionNode;
use tze_hud_scene::{Capability, Node, NodeData, Rect, SceneId};

use super::WindowedRuntimeState;
use super::WinitApp;
use super::keyboard::PendingKeyboardEvent;

impl WindowedRuntimeState {
    /// Build an inert `WindowedRuntimeState` for headless state-machine tests.
    ///
    /// Mirrors the field construction in [`super::WindowedRuntime::run`] but
    /// omits everything that needs a display, a GPU, or a live network runtime:
    ///
    /// - `window` / `window_surface` / `compositor` / `compositor_handle` — `None`.
    /// - `network_rt` / `network_handles` — no gRPC/MCP servers are spawned.
    /// - the broadcast/op channels (`element_repositioned_tx`, `input_event_tx`,
    ///   `portal_op_rx`, `safe_mode_exit_tx`, `resident_grpc_bridge`) — `None`.
    ///
    /// The `safe_mode_atomic` and `active_tab_mirror` `Arc`s are shared between
    /// the state and its embedded [`SharedState`] exactly as production does, so
    /// the lock-free keyboard-dispatch reads observe the same values a real run
    /// would.
    pub(super) fn new_headless() -> Self {
        use std::collections::{HashMap, HashSet, VecDeque};
        use std::sync::Mutex as StdMutex;
        use std::sync::atomic::AtomicBool;

        use tokio::sync::Mutex as TokioMutex;

        let safe_mode_atomic = Arc::new(AtomicBool::new(false));
        let active_tab_mirror: Arc<StdMutex<Option<SceneId>>> = Arc::new(StdMutex::new(None));

        let shared_state = Arc::new(TokioMutex::new(SharedStateBuilder::build(
            Arc::clone(&safe_mode_atomic),
            Arc::clone(&active_tab_mirror),
        )));

        let (frame_ready_tx, frame_ready_rx) = crate::channels::frame_ready_channel();
        // Input-capture / paste channels: the drain path never reads these, but
        // the fields require a receiver. Drop the senders — a disconnected
        // receiver is harmless here.
        let (_input_capture_tx, input_capture_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_paste_inject_tx, paste_inject_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        WindowedRuntimeState {
            config: super::WindowedConfig::default(),
            compositor_handle: None,
            network_rt: None,
            network_handles: Vec::new(),
            runtime_context: Arc::new(crate::runtime_context::RuntimeContext::headless_default()),
            _runtime_widget_store: None,
            fallback_unrestricted: true,
            shared_state,
            safe_mode_atomic,
            active_tab_mirror,
            safe_mode_exit_tx: None,
            chrome_state: Arc::new(std::sync::RwLock::new(crate::shell::ChromeState::new())),
            input_ring: Arc::new(StdMutex::new(VecDeque::new())),
            pending_input_latency: Arc::new(StdMutex::new(VecDeque::new())),
            frame_ready_rx,
            frame_ready_tx: Some(frame_ready_tx),
            frame_presented_tx: None,
            compositor: None,
            window_surface: None,
            input_processor: InputProcessor::new(),
            input_capture_rx,
            pending_input_capture_commands: VecDeque::new(),
            paste_inject_rx,
            focus_manager: FocusManager::new(),
            keyboard_processor: KeyboardProcessor::new(),
            telemetry: tze_hud_telemetry::TelemetryCollector::new(),
            pipeline: crate::pipeline::FramePipeline::new(),
            shutdown: crate::threads::ShutdownToken::new(),
            benchmark_failed: Arc::new(AtomicBool::new(false)),
            cursor_x: 0.0,
            cursor_y: 0.0,
            left_button_down: false,
            cursor_tracker: tze_hud_input::CursorIconTracker::new(),
            window: None,
            effective_mode: crate::window::WindowMode::Fullscreen,
            hit_regions: Vec::new(),
            static_hit_regions: Vec::new(),
            widget_hover_trackers: HashMap::new(),
            pending_mode_switch: None,
            pending_widget_svgs: Vec::new(),
            modifiers: winit::keyboard::ModifiersState::empty(),
            current_monitor_index: 0,
            global_tokens: HashMap::new(),
            element_repositioned_tx: None,
            input_event_tx: None,
            pending_blur_delivery_context: None,
            composer_pointer_drag_anchor: None,
            portal_resize_states: HashMap::new(),
            consumed_portal_resize_keydowns: HashSet::new(),
            keyboard_activation_nodes: HashMap::new(),
            consumed_command_keydowns: HashSet::new(),
            local_composer_state: Arc::new(StdMutex::new(None)),
            viewer_echo_queue: Arc::new(StdMutex::new(Vec::new())),
            focus_ring_owner_state: Arc::new(StdMutex::new(None)),
            resize_grip_hover_state: Arc::new(StdMutex::new(None)),
            composer_visual_layout: Arc::new(StdMutex::new(None)),
            portal_projection_driver: crate::portal_projection_driver::InProcessPortalDriver::new(),
            portal_op_rx: None,
            pending_keyboard_events: VecDeque::new(),
            interaction_feedback_lock_misses: std::sync::atomic::AtomicU64::new(0),
            resident_grpc_bridge: None,
            resident_grpc_input_rx: None,
        }
    }
}

/// Local builder for a minimal [`SharedState`], factored out only to keep the
/// long field list in [`WindowedRuntimeState::new_headless`] readable. Mirrors
/// [`super::test_support::make_shared_state`] but threads through the caller's
/// shared `safe_mode_atomic` / `active_tab_mirror` `Arc`s.
struct SharedStateBuilder;

impl SharedStateBuilder {
    fn build(
        safe_mode_atomic: Arc<std::sync::atomic::AtomicBool>,
        active_tab_mirror: Arc<std::sync::Mutex<Option<SceneId>>>,
    ) -> tze_hud_protocol::session::SharedState {
        use tokio::sync::Mutex as TokioMutex;
        tze_hud_protocol::session::SharedState {
            scene: Arc::new(TokioMutex::new(SceneGraph::new(1920.0, 1080.0))),
            sessions: tze_hud_protocol::session::SessionRegistry::new("test-psk"),
            resource_store: tze_hud_resource::ResourceStore::new(
                tze_hud_resource::ResourceStoreConfig::default(),
            ),
            widget_asset_store: tze_hud_protocol::session::WidgetAssetStore::default(),
            runtime_widget_store: None,
            element_store: tze_hud_scene::element_store::ElementStore::default(),
            element_store_path: None,
            safe_mode_atomic,
            active_tab_mirror,
            token_store: tze_hud_protocol::token::TokenStore::new(),
            freeze_active: false,
            degradation_level: tze_hud_protocol::session::RuntimeDegradationLevel::Normal,
            media_ingress_active: None,
            input_capture_tx: None,
            resolved_portal_tokens: std::collections::HashMap::new(),
        }
    }
}

/// Drives the real [`WinitApp`] event/window state machine headlessly.
///
/// Wraps a [`WinitApp`] built from [`WindowedRuntimeState::new_headless`] and
/// exposes the small surface a keyboard-drain test needs: install a focused
/// composer, inject synthetic `PendingKeyboardEvent`s, run the genuine
/// [`WinitApp::drain_pending_keyboard_events`], and observe the resulting
/// composer draft. The entire body of the drain — active-tab resolution via the
/// lock-free mirror, per-event inner-fn dispatch, and
/// `restore_front_requeued_event` — runs through production code.
pub(super) struct HeadlessEventLoopHarness {
    app: WinitApp,
}

impl HeadlessEventLoopHarness {
    /// Build a harness around an inert-but-real `WinitApp`.
    pub(super) fn new() -> Self {
        HeadlessEventLoopHarness {
            app: WinitApp {
                state: WindowedRuntimeState::new_headless(),
            },
        }
    }

    /// Install a single-tab scene containing a focused composer region and seed
    /// the lock-free `active_tab_mirror`, so the keyboard-drain path routes
    /// character events into the composer draft. Returns the active tab id.
    ///
    /// Focus is acquired through the same production path a pointer-down would
    /// use: [`InputProcessor::process_with_focus`] on the harness's *own*
    /// `input_processor` and `focus_manager` (the exact fields the drain later
    /// reads).
    pub(super) fn focus_composer(&mut self) -> SceneId {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 800.0, 600.0),
                1,
            )
            .unwrap();
        let composer_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    layout: Default::default(),
                    id: composer_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 800.0, 60.0),
                        interaction_id: "composer-input".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        accepts_composer_input: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        // Focus the composer via the production pointer-down focus path, using
        // the harness's own input_processor + focus_manager (disjoint field
        // borrows on `state`).
        self.app.state.focus_manager.add_tab(tab_id);
        let pointer = PointerEvent {
            x: 10.0,
            y: 10.0,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };
        self.app.state.input_processor.process_with_focus(
            &pointer,
            &mut scene,
            &mut self.app.state.focus_manager,
            tab_id,
        );
        assert!(
            self.app.state.input_processor.is_composer_active(),
            "composer must be active after focusing the composer region"
        );

        // Install the focused scene into shared_state and seed the mirror, the
        // way the post-apply_batch refresh does in production. Sync test → no
        // Tokio runtime is entered, so blocking_lock is safe.
        {
            let shared = self.app.state.shared_state.blocking_lock();
            *shared.scene.blocking_lock() = scene;
        }
        *self.app.state.active_tab_mirror.lock().unwrap() = Some(tab_id);
        tab_id
    }

    /// Enqueue a synthetic keyboard event onto the runtime's pending queue,
    /// exactly as the winit event handler does when a dispatch is deferred.
    pub(super) fn enqueue(&mut self, event: PendingKeyboardEvent) {
        self.app.state.pending_keyboard_events.push_back(event);
    }

    /// Number of events still pending (undrained).
    pub(super) fn pending_len(&self) -> usize {
        self.app.state.pending_keyboard_events.len()
    }

    /// Peek the front pending event (for FIFO-ordering assertions).
    pub(super) fn front_pending(&self) -> Option<&PendingKeyboardEvent> {
        self.app.state.pending_keyboard_events.front()
    }

    /// Run the genuine production drain over the pending queue.
    pub(super) fn drain(&mut self) {
        self.app.drain_pending_keyboard_events();
    }

    /// Current composer draft text, if a composer is active.
    pub(super) fn composer_draft(&self) -> Option<String> {
        self.app
            .state
            .input_processor
            .composer_draft_snapshot()
            .map(|(text, ..)| text)
    }

    /// A clone of the lock-free `active_tab_mirror` handle, so a test can hold
    /// its guard to simulate mirror contention.
    pub(super) fn active_tab_mirror(&self) -> Arc<std::sync::Mutex<Option<SceneId>>> {
        Arc::clone(&self.app.state.active_tab_mirror)
    }

    /// A clone of the `shared_state` handle, so a test can hold its guard to
    /// simulate scene/shared-state lock contention (the busy-defer path).
    pub(super) fn shared_state(
        &self,
    ) -> Arc<tokio::sync::Mutex<tze_hud_protocol::session::SharedState>> {
        Arc::clone(&self.app.state.shared_state)
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_input::{KeyboardModifiers, RawCharacterEvent, RawKeyDownEvent, RawKeyUpEvent};
    use tze_hud_protocol::proto::input_envelope::Event as ProtoInputEvent;
    use tze_hud_protocol::proto::{CommandAction, CommandSource};
    use tze_hud_scene::MonoUs;

    fn character(ch: &str, ts: u64) -> PendingKeyboardEvent {
        PendingKeyboardEvent::Character(RawCharacterEvent {
            character: ch.to_string(),
            timestamp_mono_us: MonoUs(ts),
        })
    }

    fn key_down(key_code: &str, key: &str, ts: u64) -> PendingKeyboardEvent {
        PendingKeyboardEvent::KeyDown(RawKeyDownEvent {
            key_code: key_code.to_string(),
            key: key.to_string(),
            modifiers: KeyboardModifiers::NONE,
            repeat: false,
            timestamp_mono_us: MonoUs(ts),
        })
    }

    fn key_up(key_code: &str, key: &str, ts: u64) -> PendingKeyboardEvent {
        PendingKeyboardEvent::KeyUp(RawKeyUpEvent {
            key_code: key_code.to_string(),
            key: key.to_string(),
            modifiers: KeyboardModifiers::NONE,
            timestamp_mono_us: MonoUs(ts),
        })
    }

    /// Install one ordinary focused button (not a portal composer/control) and
    /// return its ids plus a receiver for the runtime's real input-event channel.
    fn install_button(
        harness: &mut HeadlessEventLoopHarness,
        initially_focused: bool,
    ) -> (
        SceneId,
        SceneId,
        tokio::sync::broadcast::Receiver<(String, tze_hud_protocol::proto::EventBatch)>,
    ) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease(
            "command-agent",
            60_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile(
                tab_id,
                "command-agent",
                lease_id,
                Rect::new(0.0, 0.0, 400.0, 200.0),
                1,
            )
            .unwrap();
        let node_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    layout: Default::default(),
                    id: node_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(10.0, 10.0, 160.0, 48.0),
                        interaction_id: "primary-action".to_string(),
                        accepts_focus: true,
                        accepts_pointer: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        harness.app.state.focus_manager.add_tab(tab_id);
        if initially_focused {
            // Use the direct command-focus helper so no PointerDown pre-sets
            // `pressed`; ACTIVATE must be the operation that changes it.
            let _ = harness
                .app
                .state
                .focus_manager
                .focus_node_via_command(tab_id, tile_id, node_id, &scene);
        }

        {
            let shared = harness.app.state.shared_state.blocking_lock();
            *shared.scene.blocking_lock() = scene;
        }
        *harness.app.state.active_tab_mirror.lock().unwrap() = Some(tab_id);

        let (tx, rx) = tokio::sync::broadcast::channel(16);
        harness.app.state.input_event_tx = Some(tx);
        (tile_id, node_id, rx)
    }

    fn received_events(
        rx: &mut tokio::sync::broadcast::Receiver<(String, tze_hud_protocol::proto::EventBatch)>,
    ) -> Vec<(String, ProtoInputEvent)> {
        let mut events = Vec::new();
        while let Ok((namespace, batch)) = rx.try_recv() {
            events.extend(
                batch
                    .events
                    .into_iter()
                    .filter_map(|envelope| envelope.event)
                    .map(|event| (namespace.clone(), event)),
            );
        }
        events
    }

    /// RFC 0004 §10 production proof: keyboard is a concrete pointer-free
    /// command source, not merely a library/test fixture. Drive the real pending
    /// keyboard drain and require ACTIVATE to emerge on the runtime broadcast.
    #[test]
    fn real_keyboard_drain_dispatches_activate_command_and_local_feedback() {
        let mut harness = HeadlessEventLoopHarness::new();
        let (tile_id, node_id, mut rx) = install_button(&mut harness, true);

        harness.enqueue(key_down("Enter", "Enter", 1_000));
        harness.drain();

        let events = received_events(&mut rx);
        let command = events.iter().find_map(|(namespace, event)| match event {
            ProtoInputEvent::CommandInput(command) => Some((namespace, command)),
            _ => None,
        });
        let (namespace, command) = command.unwrap_or_else(|| {
            panic!("production keyboard drain must emit CommandInputEvent; got {events:?}")
        });
        assert_eq!(namespace, "command-agent");
        assert_eq!(command.tile_id, tile_id.as_uuid().as_bytes());
        assert_eq!(command.node_id, node_id.as_uuid().as_bytes());
        assert_eq!(command.interaction_id, "primary-action");
        assert_eq!(command.action, CommandAction::Activate as i32);
        assert_eq!(command.source, CommandSource::Keyboard as i32);

        let shared = harness.app.state.shared_state.blocking_lock();
        let scene = shared.scene.blocking_lock();
        assert!(
            scene
                .hit_region_states
                .get(&node_id)
                .is_some_and(|state| state.pressed),
            "ACTIVATE must set local pressed feedback before agent delivery"
        );
        drop(scene);
        drop(shared);

        harness.enqueue(key_up("Enter", "Enter", 2_000));
        harness.drain();

        let shared = harness.app.state.shared_state.blocking_lock();
        let scene = shared.scene.blocking_lock();
        assert!(
            scene
                .hit_region_states
                .get(&node_id)
                .is_some_and(|state| !state.pressed),
            "matching activation KeyUp must clear local pressed feedback"
        );
    }

    /// A command binding replaces the raw key sequence, not just the
    /// key-down. A focused agent must never receive a raw release without the
    /// matching raw press after the Context Menu key was translated to CONTEXT.
    #[test]
    fn command_binding_swallows_matching_raw_key_up() {
        let mut harness = HeadlessEventLoopHarness::new();
        let (_tile_id, _node_id, mut rx) = install_button(&mut harness, true);

        harness.enqueue(key_down("ContextMenu", "ContextMenu", 1_000));
        harness.enqueue(key_up("ContextMenu", "ContextMenu", 1_100));
        harness.drain();

        let events = received_events(&mut rx);
        assert!(
            events
                .iter()
                .any(|(_, event)| matches!(event, ProtoInputEvent::CommandInput(command) if command.action == CommandAction::Context as i32)),
            "ContextMenu must be translated to the CONTEXT command; got {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|(_, event)| matches!(event, ProtoInputEvent::KeyUp(_))),
            "a command binding must swallow its matching raw KeyUp; got {events:?}"
        );
    }

    /// Bare Tab already moved focus in production, but it skipped the abstract
    /// command pipeline. Require the real drain to deliver NAVIGATE_NEXT to the
    /// newly focused owner after local focus movement.
    #[test]
    fn real_keyboard_drain_dispatches_navigate_next_to_new_focus_owner() {
        let mut harness = HeadlessEventLoopHarness::new();
        let (_tile_id, node_id, mut rx) = install_button(&mut harness, false);

        harness.enqueue(key_down("Tab", "Tab", 1_000));
        harness.drain();

        assert_eq!(
            harness
                .app
                .state
                .focus_manager
                .current_owner(
                    *harness
                        .app
                        .state
                        .active_tab_mirror
                        .lock()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                )
                .node_id(),
            Some(node_id),
            "Tab must move focus locally before command delivery"
        );
        let events = received_events(&mut rx);
        let command = events.iter().find_map(|(_, event)| match event {
            ProtoInputEvent::CommandInput(command) => Some(command),
            _ => None,
        });
        let command = command.unwrap_or_else(|| {
            panic!("Tab must emit NAVIGATE_NEXT CommandInputEvent; got {events:?}")
        });
        assert_eq!(command.action, CommandAction::NavigateNext as i32);
        assert_eq!(command.source, CommandSource::Keyboard as i32);
        assert_eq!(command.node_id, node_id.as_uuid().as_bytes());
    }

    /// Composer-less Escape retains the existing focus-recovery behavior while
    /// also delivering the RFC 0004 CANCEL command to the owner that had focus.
    #[test]
    fn real_keyboard_drain_dispatches_cancel_before_focus_owner_is_lost() {
        let mut harness = HeadlessEventLoopHarness::new();
        let (_tile_id, node_id, mut rx) = install_button(&mut harness, true);

        harness.enqueue(key_down("Escape", "Escape", 1_000));
        harness.drain();

        let tab_id = harness
            .app
            .state
            .active_tab_mirror
            .lock()
            .unwrap()
            .expect("test scene has an active tab");
        assert_eq!(
            *harness.app.state.focus_manager.current_owner(tab_id),
            tze_hud_input::FocusOwner::None,
            "Escape recovery must still clear the composer-less focus stop"
        );

        let events = received_events(&mut rx);
        let command = events.iter().find_map(|(_, event)| match event {
            ProtoInputEvent::CommandInput(command) => Some(command),
            _ => None,
        });
        let command = command.unwrap_or_else(|| {
            panic!("Escape must emit CANCEL before its focus target is lost; got {events:?}")
        });
        assert_eq!(command.action, CommandAction::Cancel as i32);
        assert_eq!(command.source, CommandSource::Keyboard as i32);
        assert_eq!(command.node_id, node_id.as_uuid().as_bytes());
    }

    /// hud-nu0ea headline: the keyboard-drain full path runs end-to-end through
    /// production dispatch — NOT a reconstructed closure.
    ///
    /// A focused composer plus a pile of queued character events is drained by
    /// the real [`WinitApp::drain_pending_keyboard_events`]. That method resolves
    /// the active tab from the lock-free mirror, pops each event, and routes it
    /// through `dispatch_character_event_inner` (the composer intercept). We
    /// assert the queue fully drains AND every keystroke landed in the real
    /// `InputProcessor` composer draft — the exact end-to-end behavior the old
    /// hand-reconstructed drain loop only *modeled*.
    #[test]
    fn drain_routes_characters_through_real_dispatch_into_composer_draft() {
        let mut harness = HeadlessEventLoopHarness::new();
        harness.focus_composer();

        for (i, ch) in ["h", "e", "l", "l", "o"].into_iter().enumerate() {
            harness.enqueue(character(ch, (i as u64 + 1) * 1_000));
        }
        assert_eq!(harness.pending_len(), 5, "precondition: 5 events queued");

        harness.drain();

        assert_eq!(
            harness.pending_len(),
            0,
            "the real drain must fully empty the queue (no front→back rotation)"
        );
        assert_eq!(
            harness.composer_draft().as_deref(),
            Some("hello"),
            "every drained keystroke must be applied to the composer draft via \
             the real dispatch path"
        );
    }

    /// The real drain must stop immediately — popping nothing — when the
    /// lock-free `active_tab_mirror` is contended, preserving strict FIFO order
    /// across the `about_to_wait` boundary.
    ///
    /// This exercises the `active_tab_for_keyboard_dispatch().is_none()` break at
    /// the top of the genuine drain loop. `std::sync::Mutex::try_lock` returns
    /// `WouldBlock` while a guard is held (even on the same thread), so holding
    /// the mirror guard here forces that busy branch without a second thread.
    #[test]
    fn drain_breaks_without_popping_when_active_tab_mirror_is_busy() {
        let mut harness = HeadlessEventLoopHarness::new();
        harness.focus_composer();
        harness.enqueue(character("a", 1_000));
        harness.enqueue(character("b", 2_000));

        let mirror = harness.active_tab_mirror();
        let guard = mirror.try_lock().expect("mirror must be free before drain");

        harness.drain();

        drop(guard);
        assert_eq!(
            harness.pending_len(),
            2,
            "no event may be popped while the active_tab mirror is busy"
        );
        // Draft untouched — nothing was dispatched.
        assert_eq!(
            harness.composer_draft().as_deref(),
            Some(""),
            "no keystroke may reach the composer draft while the mirror is busy"
        );
    }

    /// When inner dispatch defers the popped event (a required shared-state/scene
    /// lock was busy), the real drain must restore that event to the FRONT of the
    /// queue and break — never let a later event overtake it.
    ///
    /// The composer intercept in `dispatch_character_event_inner` resolves its
    /// delivery context via `namespace_for_keyboard_tile`, which `try_lock`s
    /// `shared_state`. Holding that guard forces `ComposerDeliveryContextLookup::
    /// Busy`, so the inner fn pushes the popped event to the tail;
    /// `restore_front_requeued_event` inside the genuine drain then detects the
    /// growth, moves it back to the front, and stops. This is the real-path
    /// analogue of the previously reconstructed `restore_front_requeued_event`
    /// closure test.
    #[test]
    fn drain_restores_requeued_event_to_front_when_delivery_context_busy() {
        let mut harness = HeadlessEventLoopHarness::new();
        harness.focus_composer();
        harness.enqueue(character("x", 1_000));
        harness.enqueue(character("y", 2_000));

        // Hold shared_state so namespace_for_keyboard_tile → Busy inside the
        // inner composer dispatch. try_lock is non-reentrant, so holding the
        // guard on this thread is sufficient.
        let shared = harness.shared_state();
        let guard = shared
            .try_lock()
            .expect("shared_state must be free before drain");

        harness.drain();

        drop(guard);

        // The popped-then-deferred "x" must be back at the front, ahead of "y",
        // and nothing may have been consumed into the draft.
        assert_eq!(
            harness.pending_len(),
            2,
            "the deferred event must be restored, not dropped"
        );
        match harness.front_pending() {
            Some(PendingKeyboardEvent::Character(raw)) => assert_eq!(
                raw.character, "x",
                "the deferred event must be restored to the FRONT (FIFO preserved)"
            ),
            other => panic!("expected Character(\"x\") at front, got {other:?}"),
        }
        assert_eq!(
            harness.composer_draft().as_deref(),
            Some(""),
            "a busy-deferred keystroke must not mutate the draft"
        );
    }
}
