//! # windowed
//!
//! Windowed runtime — the production display path. Runs the full 8-stage frame
//! pipeline with a real `winit` window and `wgpu` swapchain.
//!
//! ## Architecture (spec §Thread Model, line 19)
//!
//! - **Main thread**: winit event loop, Stage 1 input drain, Stage 2 local
//!   feedback, surface.present() on `FrameReadySignal`.
//! - **Compositor thread**: Stages 3–7 (scene commit → GPU submit). Owns
//!   `wgpu::Device` and `wgpu::Queue` exclusively.
//! - **Network thread(s)**: Tokio runtime for gRPC and MCP.
//! - **Telemetry thread**: async structured emission.
//!
//! ## Window modes (spec §Window Modes, line 172)
//!
//! Two modes are supported, configured via `WindowedConfig::window.mode`:
//!
//! - **Fullscreen**: borderless fullscreen (`Fullscreen::Borderless`). The
//!   compositor owns the entire display with an opaque background. All input
//!   is captured (no passthrough).
//!
//! - **Overlay/HUD**: transparent, borderless, always-on-top window. Per-region
//!   input passthrough is implemented via `Window::set_cursor_hittest()`:
//!   - When the cursor is **inside** any active hit-region → `set_cursor_hittest(true)`
//!     (window captures the event).
//!   - When the cursor is **outside** all hit-regions → `set_cursor_hittest(false)`
//!     (event passes through to the desktop).
//!   This gives the same semantic as the XShape extension / wlr-layer-shell approach
//!   while using winit's cross-platform API.
//!
//! ## GNOME Wayland fallback (spec §Unsupported overlay fallback, line 185)
//!
//! `resolve_window_mode()` detects GNOME Wayland (no layer-shell) and falls back
//! to fullscreen with a startup warning logged.
//!
//! ## Runtime mode switching
//!
//! Mode switching is supported but disruptive (requires surface recreation, spec
//! line 173). Call `WinitApp::request_mode_switch()` — the event loop stores a
//! pending mode switch, tears down the existing window and compositor, and
//! re-initialises with the new mode on the next `RedrawRequested` event (where
//! the pending switch is detected before the frame is presented).
//!
//! ## Main thread event loop
//!
//! The winit event loop runs on the main thread (OS requirement on macOS).
//! On each `WindowEvent::RedrawRequested`, the main thread:
//! 1. Drains pending `PointerEvent` / `KeyboardEvent` from the input channel.
//! 2. Checks `FrameReadySignal` (tokio::sync::watch) for a compositor-ready signal.
//! 3. Calls `surface.get_current_texture()` then `surface_texture.present()` if
//!    a frame is ready.
//!
//! Input events are forwarded to the compositor thread via `input_tx` (ring buffer).
//!
//! ## Input integration
//!
//! winit `WindowEvent` → `PointerEvent` or `KeyboardEvent`:
//! - `CursorMoved`  → `PointerEvent { kind: Move, x, y }`
//! - `MouseInput`   → `PointerEvent { kind: Down | Up, x, y }`
//! - `KeyboardInput`→ `KeyboardEvent { key_code, logical_key, modifiers, pressed }`
//!
//! Per spec §Stage 1 Input Drain (line 72): "MUST drain all pending OS input
//! events, attach hardware timestamps, produce InputEvent records, enqueue to
//! InputEvent channel."
//!
//! ## FrameReadySignal and surface.present()
//!
//! The compositor thread sends `true` on `FrameReadyTx` after GPU submit.
//! The main thread loop detects the change (via `watch::Receiver::has_changed`)
//! and calls `present_pending_frame()`.
//!
//! Per spec §Compositor Thread Ownership (line 46): "The main thread SHALL hold
//! the surface handle and be the only thread that calls surface.present()."
//!
//! ## Window resize
//!
//! On `WindowEvent::Resized`, the main thread calls `surface.reconfigure()`.
//! The compositor thread picks up the new size on the next `surface.size()` call.

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId, WindowLevel};

use tze_hud_compositor::{Compositor, WindowSurface};
use tze_hud_input::{InputProcessor, PointerEvent, PointerEventKind};
use tze_hud_protocol::session::SharedState;
use tze_hud_protocol::token::TokenStore;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::ZoneContent;
use tze_hud_telemetry::TelemetryCollector;

use crate::channels::{
    frame_ready_channel, FrameReadyRx, FrameReadyTx, InputEvent, InputEventKind,
    INPUT_EVENT_CAPACITY,
};
use crate::pipeline::FramePipeline;
use crate::threads::{
    spawn_compositor_thread, CompositorReady, NetworkRuntime, ShutdownToken,
};
use crate::window::{HitRegion, WindowConfig, WindowMode};
use crate::window::{resolve_window_mode, should_capture_pointer_event};

// ─── WindowedConfig ──────────────────────────────────────────────────────────

/// Configuration for the windowed runtime.
#[derive(Debug, Clone)]
pub struct WindowedConfig {
    /// Window configuration (mode, dimensions, title).
    ///
    /// The `mode` field controls whether the runtime starts in fullscreen or
    /// overlay/HUD mode. Use `WindowMode::Fullscreen` (default) for the
    /// compositor to own the entire display, or `WindowMode::Overlay` for a
    /// transparent, borderless, always-on-top window with per-region input
    /// passthrough.
    pub window: WindowConfig,
    /// gRPC server port.  Set to `0` to disable the gRPC server.
    pub grpc_port: u16,
    /// Pre-shared key for session authentication.
    pub psk: String,
    /// Target frames per second.  Default: 60.
    pub target_fps: u32,
}

impl Default for WindowedConfig {
    fn default() -> Self {
        Self {
            window: WindowConfig::default(),
            grpc_port: 50051,
            psk: "tze-hud-key".to_string(),
            target_fps: 60,
        }
    }
}

// ─── WindowedRuntime ─────────────────────────────────────────────────────────

/// Shared state passed from the windowed runtime builder to the winit app.
///
/// All fields are `Arc`-wrapped or `Send` so the app handler can be moved into
/// the winit event loop.
struct WindowedRuntimeState {
    config: WindowedConfig,
    /// Compositor thread handle (stored so it can be joined on shutdown).
    compositor_handle: Option<std::thread::JoinHandle<()>>,
    /// Network runtime for gRPC / MCP.
    network_rt: Option<NetworkRuntime>,
    /// Shared scene + session state.
    shared_state: Arc<Mutex<SharedState>>,
    /// Input channel (ring buffer) — main thread writes, compositor thread reads.
    input_ring: Arc<std::sync::Mutex<std::collections::VecDeque<InputEvent>>>,
    /// Frame-ready signal: compositor → main thread.
    frame_ready_rx: FrameReadyRx,
    /// Frame-ready sender (compositor thread will own this; stored here until
    /// the compositor thread is spawned and takes it).
    frame_ready_tx: Option<FrameReadyTx>,
    /// Compositor and surface (Some until compositor thread is spawned and takes the compositor).
    compositor: Option<Compositor>,
    /// The window surface (main thread owns this for the lifetime of the window).
    window_surface: Option<Arc<WindowSurface>>,
    /// Input processor for local feedback.
    input_processor: InputProcessor,
    /// Telemetry collector.
    telemetry: TelemetryCollector,
    /// Frame pipeline (ArcSwap hit-test snapshot, overflow counters).
    pipeline: FramePipeline,
    /// Shutdown token.
    shutdown: ShutdownToken,
    /// Current cursor position (updated by CursorMoved events).
    cursor_x: f32,
    cursor_y: f32,
    /// Winit window handle (Some after window is created).
    window: Option<Arc<Window>>,
    /// Effective window mode after platform fallback resolution.
    ///
    /// This may differ from `config.window.mode` if an overlay-to-fullscreen
    /// fallback occurred (e.g., GNOME Wayland with no layer-shell).
    effective_mode: WindowMode,
    /// Active hit-regions for overlay input passthrough.
    ///
    /// In overlay mode, the cursor hittest is toggled on/off per frame based
    /// on whether the cursor is inside any of these regions.  Empty means all
    /// events pass through.
    hit_regions: Vec<HitRegion>,
    /// Pending mode switch requested at runtime (disruptive — triggers surface
    /// recreation on the next event loop tick).
    pending_mode_switch: Option<WindowMode>,
}

// ─── WinitApp ────────────────────────────────────────────────────────────────

/// `ApplicationHandler` implementation for winit 0.30 event loop.
///
/// The main thread creates the window in `resumed()`, initialises the
/// compositor + surface, spawns the compositor thread, then processes
/// window events on every `window_event()` call.
struct WinitApp {
    state: WindowedRuntimeState,
}

impl ApplicationHandler for WinitApp {
    /// Called by winit when the event loop has processed all pending events for
    /// the current iteration.  We use this to apply any pending mode switch:
    /// tearing down the current window/compositor and re-initialising with the
    /// new mode is safe here because no window events are in flight.
    ///
    /// Note: `resumed()` is a *lifecycle* callback (initial app start / app
    /// resume after suspension) and is NOT triggered by `window.request_redraw()`.
    /// Pending mode switches must therefore be handled here in `about_to_wait`
    /// rather than in `resumed()`.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.pending_mode_switch.is_some() {
            self.apply_pending_mode_switch();
            // Re-create the window with the new mode by forwarding to the
            // initialisation path inside resumed().
            self.resumed(event_loop);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.window.is_some() {
            return; // Already initialised.
        }

        // ── Create winit window ────────────────────────────────────────────
        let cfg = &self.state.config.window;

        // Build window attributes based on the effective window mode.
        //
        // Fullscreen: borderless fullscreen — compositor owns the entire display
        //   with an opaque background. All input captured. Spec §Fullscreen mode
        //   (line 177).
        //
        // Overlay: transparent, borderless, always-on-top window with per-region
        //   input passthrough via set_cursor_hittest(). Spec §Overlay click-through
        //   (line 181).
        let attrs = match self.state.effective_mode {
            WindowMode::Fullscreen => {
                tracing::info!(
                    "window mode: fullscreen (borderless) — compositor owns display, all input captured"
                );
                WindowAttributes::default()
                    .with_title(cfg.title.clone())
                    // Borderless fullscreen on the current monitor.
                    .with_fullscreen(Some(Fullscreen::Borderless(None)))
                    .with_decorations(false)
            }
            WindowMode::Overlay => {
                tracing::info!(
                    "window mode: overlay/HUD — transparent borderless always-on-top"
                );
                WindowAttributes::default()
                    .with_title(cfg.title.clone())
                    .with_inner_size(winit::dpi::PhysicalSize::new(cfg.width, cfg.height))
                    // Transparent so the desktop shows through non-opaque pixels.
                    .with_transparent(true)
                    // No title bar / frame — pure overlay surface.
                    .with_decorations(false)
                    // Always on top of other windows, including normal desktop windows.
                    .with_window_level(WindowLevel::AlwaysOnTop)
            }
        };

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                tracing::error!("failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        // In overlay mode, initialise cursor hittest to false so all pointer
        // events pass through to the desktop until the cursor enters a
        // hit-region.  The hittest is toggled per-frame in enqueue_pointer_event()
        // per spec §Overlay click-through (line 181).
        if self.state.effective_mode == WindowMode::Overlay {
            if let Err(e) = window.set_cursor_hittest(false) {
                tracing::warn!(
                    error = %e,
                    "overlay mode: set_cursor_hittest(false) failed — passthrough \
                     may not work on this platform/compositor"
                );
            }
        }

        self.state.window = Some(window.clone());

        let cfg = self.state.config.clone();
        let window_clone = window.clone();

        // ── Create compositor + surface (async in a blocking context) ──────
        // We need an async context to call Compositor::new_windowed.
        // Use a temporary single-thread Tokio runtime here — this runs only
        // at startup and is dropped immediately after.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build startup tokio runtime");

        let (compositor, window_surface) = rt.block_on(async {
            Compositor::new_windowed(
                window_clone,
                cfg.window.width,
                cfg.window.height,
            )
            .await
            .expect("Compositor::new_windowed failed")
        });

        let window_surface = Arc::new(window_surface);
        self.state.window_surface = Some(window_surface.clone());

        // ── Elevate main thread priority ──────────────────────────────────
        crate::threads::elevate_main_thread_priority();

        // ── Wire compositor thread ─────────────────────────────────────────
        let shared_state = self.state.shared_state.clone();
        let input_ring = self.state.input_ring.clone();
        // Share the ArcSwap handle (not the FramePipeline itself) with the compositor thread.
        let hit_test_snapshot = self.state.pipeline.hit_test_snapshot.clone();
        let frame_ready_tx = self.state
            .frame_ready_tx
            .take()
            .expect("frame_ready_tx already taken");
        let shutdown = self.state.shutdown.clone();
        let telemetry_collector = TelemetryCollector::new();
        let surface_for_compositor = window_surface.clone();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

        let compositor_handle = spawn_compositor_thread(
            shutdown.clone(),
            ready_tx,
            move |shutdown_tok, comp_ready| {
                // Signal ready immediately (compositor thread setup is synchronous).
                let _ = comp_ready.send(CompositorReady { ok: true });

                let mut compositor = compositor;
                let mut telemetry = telemetry_collector;

                let frame_interval = std::time::Duration::from_micros(
                    1_000_000 / cfg.target_fps.max(1) as u64,
                );
                let mut shutdown_rx = shutdown_tok.subscribe();

                tracing::info!("compositor thread: starting frame loop at {}fps", cfg.target_fps);

                loop {
                    // Check for shutdown.
                    match shutdown_rx.try_recv() {
                        Ok(_) => {
                            tracing::info!("compositor thread: shutdown received");
                            break;
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                            tracing::info!("compositor thread: shutdown channel closed");
                            break;
                        }
                        Err(_) => {} // Lagged or empty — continue.
                    }
                    if shutdown_tok.is_triggered() {
                        break;
                    }

                    let frame_start = Instant::now();

                    // ── Resize check ───────────────────────────────────────
                    // The main thread writes pending_resize_width/height on
                    // WindowEvent::Resized. We detect and apply it here because
                    // the compositor thread owns the wgpu::Device required by
                    // surface.reconfigure().
                    //
                    // Read width last (it was written last by the main thread)
                    // to avoid a torn read: if the main thread is mid-write we
                    // will see the old width and skip this cycle; the resize
                    // will be applied on the next frame instead.
                    let pending_w = surface_for_compositor
                        .pending_resize_width
                        .load(std::sync::atomic::Ordering::Acquire);
                    let pending_h = surface_for_compositor
                        .pending_resize_height
                        .load(std::sync::atomic::Ordering::Acquire);
                    if pending_w > 0 && pending_h > 0 {
                        surface_for_compositor.reconfigure(pending_w, pending_h, &compositor.device);
                        // Reset pending resize (store 0 to signal "handled").
                        surface_for_compositor
                            .pending_resize_width
                            .store(0, std::sync::atomic::Ordering::Release);
                        surface_for_compositor
                            .pending_resize_height
                            .store(0, std::sync::atomic::Ordering::Release);
                        // Update compositor's cached dimensions.
                        compositor.width = pending_w;
                        compositor.height = pending_h;
                    }

                    // ── Stage 3: Mutation Intake ───────────────────────────
                    // (placeholder — real mutations come via gRPC session)

                    // ── Stage 4: Scene Commit + HitTest Snapshot ──────────
                    let state_guard = {
                        // Use a lightweight approach: try to acquire lock without blocking
                        // the compositor thread for too long.
                        // In production this would use a dedicated mutation intake channel.
                        match shared_state.try_lock() {
                            Ok(g) => Some(g),
                            Err(_) => None,
                        }
                    };

                    if let Some(state) = state_guard {
                        let new_snap = crate::pipeline::HitTestSnapshot::from_scene(&state.scene);
                        hit_test_snapshot.store(Arc::new(new_snap));

                        // ── Stage 5–7: Render Encode + GPU Submit ─────────
                        let compositor_telemetry = compositor.render_frame(
                            &state.scene,
                            surface_for_compositor.as_ref(),
                        );
                        drop(state); // Release lock before signalling main thread.

                        // ── Signal main thread to present ─────────────────
                        // Per spec §Compositor Thread Ownership (line 55):
                        // "compositor thread MUST signal the main thread via
                        // FrameReadySignal, and only the main thread SHALL call
                        // surface.present()."
                        let _ = frame_ready_tx.send(true);

                        // Telemetry emit (Stage 8)
                        let mut telem = tze_hud_telemetry::FrameTelemetry::new(
                            compositor_telemetry.frame_number,
                        );
                        telem.frame_time_us = frame_start.elapsed().as_micros() as u64;
                        telem.render_encode_us = compositor_telemetry.render_encode_us;
                        telem.gpu_submit_us = compositor_telemetry.gpu_submit_us;
                        telem.tile_count = compositor_telemetry.tile_count;
                        telemetry.record(telem);
                    }

                    // Frame rate control.
                    let elapsed = frame_start.elapsed();
                    if elapsed < frame_interval {
                        std::thread::sleep(frame_interval - elapsed);
                    }
                }

                tracing::info!("compositor thread: frame loop exited");
            },
        );

        self.state.compositor_handle = Some(compositor_handle);

        // Wait for the compositor thread to signal ready (with timeout).
        let tmp_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("startup runtime 2");
        let compositor_ok = tmp_rt
            .block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    ready_rx,
                )
                .await
            })
            .ok()
            .and_then(|r| r.ok())
            .map(|r| r.ok)
            .unwrap_or(false);

        if !compositor_ok {
            tracing::warn!("compositor thread did not signal ready in time");
        } else {
            tracing::info!("windowed runtime initialised successfully");
        }

        // Request first frame.
        window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            // ── Close ──────────────────────────────────────────────────────
            WindowEvent::CloseRequested => {
                tracing::info!("main thread: window close requested");
                self.state.shutdown.trigger(crate::threads::ShutdownReason::Clean);
                event_loop.exit();
            }

            // ── Resize ─────────────────────────────────────────────────────
            WindowEvent::Resized(physical_size) => {
                if let Some(surface) = &self.state.window_surface {
                    tracing::info!(
                        width = physical_size.width,
                        height = physical_size.height,
                        "main thread: window resized — signalling compositor for reconfiguration"
                    );
                    // Signal the compositor thread to reconfigure the surface.
                    // The compositor thread owns the wgpu::Device and is the
                    // only thread that can safely call surface.configure().
                    //
                    // We write the new dimensions atomically. The compositor
                    // thread reads `pending_resize_width/height` at the start of
                    // each frame cycle, calls `surface.reconfigure()` when
                    // non-zero, and resets both fields to 0.
                    //
                    // Write height first so the compositor never sees a
                    // partially-updated pair (width updated, height stale).
                    surface.pending_resize_height.store(
                        physical_size.height,
                        std::sync::atomic::Ordering::Release,
                    );
                    surface.pending_resize_width.store(
                        physical_size.width,
                        std::sync::atomic::Ordering::Release,
                    );
                }
            }

            // ── Pointer: cursor moved ──────────────────────────────────────
            // Stage 1: Drain OS input event → InputEvent ring buffer.
            // Stage 2: Apply local feedback.
            WindowEvent::CursorMoved { position, .. } => {
                self.state.cursor_x = position.x as f32;
                self.state.cursor_y = position.y as f32;

                self.enqueue_pointer_event(PointerEventKind::Move);
            }

            // ── Pointer: button press/release ──────────────────────────────
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let kind = match state {
                        ElementState::Pressed => PointerEventKind::Down,
                        ElementState::Released => PointerEventKind::Up,
                    };
                    self.enqueue_pointer_event(kind);
                }
            }

            // ── Keyboard ──────────────────────────────────────────────────
            // Stage 1: Drain keyboard events into the input ring buffer.
            // Map winit keyboard events to InputEventKind::KeyPress / KeyRelease.
            WindowEvent::KeyboardInput { event, .. } => {
                // Extract a u32 key code from the physical key for the channel type.
                let key_u32 = physical_key_to_u32(&event.physical_key);
                let input_event = InputEvent {
                    timestamp_ns: nanoseconds_since_start(),
                    kind: if event.state == ElementState::Pressed {
                        InputEventKind::KeyPress { key: key_u32 }
                    } else {
                        InputEventKind::KeyRelease { key: key_u32 }
                    },
                };
                enqueue_input(&self.state.input_ring, input_event);
            }

            // ── Redraw ────────────────────────────────────────────────────
            WindowEvent::RedrawRequested => {
                // Stage 1/2 bookkeeping: check frame-ready signal and present.
                self.maybe_present_frame();

                // Request next redraw for continuous rendering.
                if let Some(window) = &self.state.window {
                    window.request_redraw();
                }
            }

            _ => {}
        }
    }
}

impl WinitApp {
    /// Enqueue a pointer event into the input ring buffer.
    ///
    /// Maps a `PointerEventKind` to the corresponding `InputEventKind` variant
    /// understood by the channel topology and compositor pipeline.
    ///
    /// In overlay mode, dynamically toggles cursor hittest based on whether the
    /// cursor is inside any active hit-region (spec §Overlay click-through, line 181):
    /// - Inside a hit-region → `set_cursor_hittest(true)` (window captures events).
    /// - Outside all hit-regions → `set_cursor_hittest(false)` (events pass through).
    fn enqueue_pointer_event(&mut self, kind: PointerEventKind) {
        let x = self.state.cursor_x;
        let y = self.state.cursor_y;

        // In overlay mode, update cursor hittest based on hit-region membership.
        // This implements per-region passthrough: pointer events outside all
        // active hit-regions are passed through to the underlying desktop, while
        // events inside any hit-region are captured by the runtime.
        //
        // We toggle on every CursorMoved so the hittest tracks the cursor as it
        // moves in/out of regions continuously.
        if self.state.effective_mode == WindowMode::Overlay {
            let should_capture = should_capture_pointer_event(
                WindowMode::Overlay,
                x,
                y,
                &self.state.hit_regions,
            );
            if let Some(window) = &self.state.window {
                if let Err(e) = window.set_cursor_hittest(should_capture) {
                    tracing::trace!(
                        error = %e,
                        capture = should_capture,
                        "overlay: set_cursor_hittest failed"
                    );
                }
            }
        }

        let channel_kind = match kind {
            PointerEventKind::Move => InputEventKind::PointerMove { x, y },
            PointerEventKind::Down => InputEventKind::PointerPress { x, y, button: 0 },
            PointerEventKind::Up => InputEventKind::PointerRelease { x, y, button: 0 },
        };
        let input_event = InputEvent {
            timestamp_ns: nanoseconds_since_start(),
            kind: channel_kind,
        };
        enqueue_input(&self.state.input_ring, input_event);

        // Also feed the InputProcessor for local feedback (Stage 2).
        // This happens synchronously on the main thread per spec §Stage 2.
        let pointer_event = PointerEvent {
            x,
            y,
            kind,
            device_id: 0,
            timestamp: Some(Instant::now()),
        };
        if let Ok(mut state) = self.state.shared_state.try_lock() {
            let _result = self.state.input_processor.process(&pointer_event, &mut state.scene);
            // Local feedback patch (_result.local_patch) would be sent to the
            // compositor via a local-patch channel in the full pipeline. For the
            // initial windowed runtime, the compositor reads the scene state
            // directly on the next frame.
        }
    }

    /// Update the active hit-regions for overlay input passthrough.
    ///
    /// Replaces the current hit-region set.  The new regions take effect on the
    /// next `CursorMoved` event.
    ///
    /// No-op in fullscreen mode (all events are always captured; hit-regions
    /// are not consulted).
    pub fn set_hit_regions(&mut self, regions: Vec<HitRegion>) {
        if self.state.effective_mode == WindowMode::Fullscreen {
            return; // Hit-regions unused in fullscreen mode.
        }
        self.state.hit_regions = regions;
    }

    /// Request a runtime mode switch (disruptive — triggers surface recreation).
    ///
    /// The switch is deferred to the next `about_to_wait` callback, where
    /// `apply_pending_mode_switch()` tears down the current window/compositor
    /// and `resumed()` re-creates them with the new mode.
    ///
    /// Per spec §Window Modes (line 173): "Runtime mode switching MUST be
    /// supported but is a disruptive operation requiring surface recreation."
    pub fn request_mode_switch(&mut self, new_mode: WindowMode) {
        if new_mode == self.state.effective_mode {
            tracing::debug!(mode = %new_mode, "mode switch no-op: already in requested mode");
            return;
        }
        tracing::info!(
            current = %self.state.effective_mode,
            requested = %new_mode,
            "runtime mode switch requested — surface recreation will occur on next about_to_wait"
        );
        self.state.pending_mode_switch = Some(new_mode);
        // request_redraw() ensures the event loop stays active (Poll mode),
        // so about_to_wait fires promptly after the current event batch.
        if let Some(window) = &self.state.window {
            window.request_redraw();
        }
    }

    /// Tear down the current window/compositor and apply a pending mode switch.
    ///
    /// Called from `about_to_wait` when `pending_mode_switch` is `Some`.
    /// After this returns, `self.state.window` is `None` so that `resumed()`
    /// will re-create the window with the new effective mode.
    fn apply_pending_mode_switch(&mut self) {
        let new_mode = match self.state.pending_mode_switch.take() {
            Some(m) => m,
            None => return,
        };

        tracing::info!(
            old_mode = %self.state.effective_mode,
            new_mode = %new_mode,
            "runtime mode switch: tearing down existing window for surface recreation"
        );

        // Join the compositor thread before destroying the surface.
        if let Some(handle) = self.state.compositor_handle.take() {
            self.state.shutdown.trigger(crate::threads::ShutdownReason::Clean);
            let _ = handle.join();
        }

        // Drop the surface and window handles.
        self.state.window_surface = None;
        self.state.window = None;

        // Re-create the shutdown token for the new session.
        self.state.shutdown = ShutdownToken::new();

        // Re-create the frame-ready channel.
        let (new_tx, new_rx) = frame_ready_channel();
        self.state.frame_ready_tx = Some(new_tx);
        self.state.frame_ready_rx = new_rx;

        // Apply the new mode (with platform fallback check).
        // resolve_window_mode() emits the fallback warning internally;
        // no duplicate logging needed here.
        let (resolved_mode, _) = resolve_window_mode(new_mode);
        self.state.effective_mode = resolved_mode;
        self.state.config.window.mode = resolved_mode;
    }

    /// Check the `FrameReadySignal` and present the frame if the compositor
    /// has signalled one.
    ///
    /// Per spec §Compositor Thread Ownership (line 54-55):
    /// "WHEN a frame is ready for presentation THEN the compositor thread MUST
    /// signal the main thread via FrameReadySignal, and only the main thread
    /// SHALL call surface.present()."
    ///
    /// The compositor thread stores the rendered `SurfaceTexture` in
    /// `WindowSurface::pending_texture` during `acquire_frame()`. This method
    /// retrieves that exact texture via `take_pending_texture()` and calls
    /// `SurfaceTexture::present()` on it — satisfying the macOS/Metal requirement
    /// that `present()` runs on the main thread, and ensuring we present the
    /// texture the compositor actually rendered into.
    fn maybe_present_frame(&mut self) {
        if self.state.frame_ready_rx.has_changed().unwrap_or(false) {
            // Acknowledge the signal.
            let _ = self.state.frame_ready_rx.borrow_and_update();

            if let Some(surface) = &self.state.window_surface {
                // Present under the same mutex that guards pending swapchain
                // ownership so acquire/present cannot interleave into a
                // double-acquire validation error on some backends.
                if !surface.present_pending_texture() {
                    // FrameReady signal fired but no texture is pending —
                    // this can happen if acquire_frame() failed on the
                    // compositor thread (error already logged there).
                    tracing::debug!(
                        "main thread: FrameReady received but no pending texture; \
                         compositor likely skipped frame due to surface error"
                    );
                }
            }
        }
    }
}

// ─── WindowedRuntime ─────────────────────────────────────────────────────────

/// Entry point for the windowed runtime.
///
/// Owns all windowed runtime state. Call `run()` to hand control to the
/// winit event loop (this call blocks until the window is closed).
pub struct WindowedRuntime {
    config: WindowedConfig,
}

impl WindowedRuntime {
    /// Create a new `WindowedRuntime` with the given config.
    pub fn new(config: WindowedConfig) -> Self {
        Self { config }
    }

    /// Run the windowed runtime event loop.
    ///
    /// This is a **blocking call** that runs on the main thread until the window
    /// is closed or a shutdown signal is received. It creates the winit event
    /// loop, initialises the window + compositor + surface, spawns the compositor
    /// thread, and enters the winit event loop.
    ///
    /// Per spec §Main Thread Responsibilities (line 33): "The main thread MUST
    /// run the winit event loop."
    ///
    /// # Errors
    ///
    /// Returns an error if the winit event loop or window creation fails.
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let cfg = self.config;

        // Resolve the effective window mode, applying platform fallback checks.
        // Spec §Unsupported overlay fallback (line 185): if overlay is requested
        // on GNOME Wayland (no layer-shell), fall back to fullscreen with a
        // startup warning.  resolve_window_mode() emits the warning internally
        // when a fallback occurs; no additional logging needed here.
        let (effective_mode, _fallback_reason) = resolve_window_mode(cfg.window.mode);

        // Build shared state (scene + sessions).
        let width = cfg.window.width as f32;
        let height = cfg.window.height as f32;
        let mut scene = SceneGraph::new(width, height);
        if std::env::var("TZE_HUD_SIM_SUBTITLES").as_deref() == Ok("1") {
            let samples = [
                "Subtitle demo: systems online.",
                "Subtitle demo: compositor stable.",
                "Subtitle demo: overlay path verified.",
            ];
            for line in samples {
                if let Err(e) = scene.publish_to_zone(
                    "subtitle",
                    ZoneContent::StreamText(line.to_string()),
                    "hudbot-sim",
                    None,
                    None,
                    None,
                ) {
                    tracing::warn!(error = %e, "failed to seed subtitle demo line");
                }
            }
        }
        let sessions = tze_hud_protocol::session::SessionRegistry::new(&cfg.psk);
        let shared_state = Arc::new(Mutex::new(SharedState {
            scene,
            sessions,
            safe_mode_active: false,
            token_store: TokenStore::new(),
            freeze_active: false,
            degradation_level: tze_hud_protocol::session::RuntimeDegradationLevel::Normal,
        }));

        let (frame_ready_tx, frame_ready_rx) = frame_ready_channel();
        let input_ring = Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::with_capacity(INPUT_EVENT_CAPACITY),
        ));
        let shutdown = ShutdownToken::new();

        let app_state = WindowedRuntimeState {
            config: cfg,
            compositor_handle: None,
            network_rt: None,
            shared_state,
            input_ring,
            frame_ready_rx,
            frame_ready_tx: Some(frame_ready_tx),
            compositor: None,
            window_surface: None,
            input_processor: InputProcessor::new(),
            telemetry: TelemetryCollector::new(),
            pipeline: FramePipeline::new(),
            shutdown,
            cursor_x: 0.0,
            cursor_y: 0.0,
            window: None,
            effective_mode,
            hit_regions: Vec::new(),
            pending_mode_switch: None,
        };

        let mut app = WinitApp { state: app_state };

        // Create winit event loop and run.
        // Per spec §Main Thread Responsibilities: winit event loop MUST run on main thread.
        let event_loop = EventLoop::new()?;
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.run_app(&mut app)?;

        // Cleanly join the compositor thread after the event loop exits.
        //
        // Without this, the compositor thread is detached (JoinHandle drop ≠
        // join) and may still be running GPU work during process teardown,
        // leading to device-lost errors or use-after-free in wgpu internals.
        //
        // The shutdown token was already triggered via CloseRequested
        // (WindowEvent::CloseRequested calls shutdown.trigger()), so the
        // compositor frame loop should exit promptly.
        if let Some(handle) = app.state.compositor_handle.take() {
            tracing::info!("waiting for compositor thread to exit...");
            if let Err(e) = handle.join() {
                tracing::error!("compositor thread panicked: {e:?}");
            } else {
                tracing::info!("compositor thread exited cleanly");
            }
        }

        Ok(())
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Push an `InputEvent` into the ring buffer, dropping the oldest if full.
fn enqueue_input(
    ring: &Arc<std::sync::Mutex<std::collections::VecDeque<InputEvent>>>,
    event: InputEvent,
) {
    if let Ok(mut q) = ring.lock() {
        if q.len() >= INPUT_EVENT_CAPACITY {
            q.pop_front(); // Drop oldest to make room.
        }
        q.push_back(event);
    }
}

/// Monotonic nanosecond timestamp for `InputEvent.timestamp_ns`.
///
/// Uses process-relative time so values are comparable within a session.
fn nanoseconds_since_start() -> u64 {
    // Use std::time::Instant for monotonic timing.
    // We store the process start time lazily and subtract.
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}

/// Map a winit `PhysicalKey` to a compact u32 key code.
///
/// This is a best-effort mapping for the `InputEventKind::KeyPress/KeyRelease`
/// channel type. The full keyboard pipeline uses `tze_hud_input::KeyboardProcessor`
/// for richer key event data.
fn physical_key_to_u32(key: &winit::keyboard::PhysicalKey) -> u32 {
    use winit::keyboard::PhysicalKey;
    match key {
        PhysicalKey::Code(code) => *code as u32,
        PhysicalKey::Unidentified(_) => 0,
    }
}

/// Convert a winit `Key` (logical key) to a string for debug/logging.
#[allow(dead_code)]
fn winit_logical_to_str(key: &winit::keyboard::Key) -> String {
    match key {
        winit::keyboard::Key::Character(s) => s.to_string(),
        winit::keyboard::Key::Named(named) => format!("{named:?}"),
        winit::keyboard::Key::Unidentified(native) => format!("Unidentified({native:?})"),
        winit::keyboard::Key::Dead(Some(c)) => format!("Dead({c})"),
        winit::keyboard::Key::Dead(None) => "Dead".to_string(),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windowed_config_default_values() {
        let cfg = WindowedConfig::default();
        assert_eq!(cfg.target_fps, 60);
        assert_eq!(cfg.grpc_port, 50051);
        assert!(!cfg.psk.is_empty());
    }

    #[test]
    fn enqueue_input_drops_oldest_when_full() {
        let ring = Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::with_capacity(INPUT_EVENT_CAPACITY),
        ));
        // Fill beyond capacity.
        for i in 0..INPUT_EVENT_CAPACITY + 10 {
            let event = InputEvent {
                timestamp_ns: i as u64,
                kind: InputEventKind::KeyPress { key: 0 },
            };
            enqueue_input(&ring, event);
        }
        let q = ring.lock().unwrap();
        assert_eq!(
            q.len(),
            INPUT_EVENT_CAPACITY,
            "ring buffer should never exceed capacity"
        );
        // The oldest entry was dropped; the newest should have timestamp
        // INPUT_EVENT_CAPACITY + 9.
        let last = q.back().unwrap();
        assert_eq!(
            last.timestamp_ns,
            (INPUT_EVENT_CAPACITY + 9) as u64,
            "most recent event should be at the back"
        );
    }

    #[test]
    fn nanoseconds_since_start_is_monotonic() {
        let t1 = nanoseconds_since_start();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let t2 = nanoseconds_since_start();
        assert!(t2 > t1, "timestamps must be monotonically increasing");
    }

    #[test]
    fn winit_logical_to_str_character() {
        use winit::keyboard::Key;
        let key = Key::Character("a".into());
        assert_eq!(winit_logical_to_str(&key), "a");
    }

    #[test]
    fn winit_logical_to_str_dead() {
        use winit::keyboard::Key;
        let key = Key::Dead(Some('´'));
        let s = winit_logical_to_str(&key);
        assert!(s.starts_with("Dead"));
    }

    // ── Window mode configuration ─────────────────────────────────────────

    #[test]
    fn windowed_config_default_mode_is_fullscreen() {
        let cfg = WindowedConfig::default();
        assert_eq!(
            cfg.window.mode,
            WindowMode::Fullscreen,
            "default mode must be fullscreen (spec §Window Modes)"
        );
    }

    #[test]
    fn windowed_config_overlay_mode_can_be_set() {
        let cfg = WindowedConfig {
            window: WindowConfig {
                mode: WindowMode::Overlay,
                width: 1280,
                height: 720,
                title: "test-overlay".to_string(),
            },
            ..WindowedConfig::default()
        };
        assert_eq!(cfg.window.mode, WindowMode::Overlay);
    }

    // ── resolve_window_mode integration ──────────────────────────────────

    /// Verify that resolve_window_mode is called correctly for fullscreen
    /// (no fallback should ever occur for fullscreen).
    #[test]
    fn resolve_fullscreen_config_produces_fullscreen() {
        let (mode, reason) = resolve_window_mode(WindowMode::Fullscreen);
        assert_eq!(mode, WindowMode::Fullscreen);
        assert!(reason.is_none(), "fullscreen must never trigger a fallback");
    }

    /// Verify that resolve_window_mode for overlay either returns Overlay
    /// (if supported) or falls back to Fullscreen (GNOME Wayland), but never
    /// panics and always produces a valid mode.
    #[test]
    fn resolve_overlay_config_is_always_valid() {
        let (mode, _reason) = resolve_window_mode(WindowMode::Overlay);
        assert!(
            mode == WindowMode::Overlay || mode == WindowMode::Fullscreen,
            "resolved mode must be Overlay or Fullscreen, got: {mode}"
        );
    }

    // ── Overlay passthrough logic (no window required) ────────────────────

    /// In fullscreen mode, should_capture_pointer_event must return true
    /// regardless of cursor position or hit-regions (spec §Fullscreen mode,
    /// line 177: "all input captured").
    #[test]
    fn fullscreen_captures_pointer_outside_any_hit_region() {
        // No hit-regions at all.
        let capture = should_capture_pointer_event(WindowMode::Fullscreen, 9000.0, 9000.0, &[]);
        assert!(capture, "fullscreen must capture all pointer events");
    }

    #[test]
    fn fullscreen_captures_pointer_even_with_regions_present() {
        let regions = vec![HitRegion::new(0.0, 0.0, 100.0, 100.0)];
        // Cursor is far outside the region — fullscreen still captures.
        let capture =
            should_capture_pointer_event(WindowMode::Fullscreen, 9000.0, 9000.0, &regions);
        assert!(capture);
    }

    /// In overlay mode with no hit-regions, ALL pointer events must pass through
    /// (spec §Overlay click-through, line 181).
    #[test]
    fn overlay_no_hit_regions_passes_through_all_events() {
        let capture = should_capture_pointer_event(WindowMode::Overlay, 500.0, 500.0, &[]);
        assert!(!capture, "overlay with no hit-regions must pass all events through");
    }

    /// In overlay mode, cursor inside a hit-region is captured.
    #[test]
    fn overlay_cursor_inside_hit_region_is_captured() {
        let regions = vec![HitRegion::new(100.0, 100.0, 200.0, 150.0)];
        let capture = should_capture_pointer_event(WindowMode::Overlay, 150.0, 150.0, &regions);
        assert!(capture, "cursor inside hit-region must be captured");
    }

    /// In overlay mode, cursor outside all hit-regions passes through.
    #[test]
    fn overlay_cursor_outside_all_hit_regions_passes_through() {
        let regions = vec![HitRegion::new(100.0, 100.0, 200.0, 150.0)];
        // Cursor at (50, 50) is outside the region.
        let capture = should_capture_pointer_event(WindowMode::Overlay, 50.0, 50.0, &regions);
        assert!(!capture, "cursor outside all hit-regions must pass through");
    }

    /// In overlay mode, the union of multiple hit-regions is used.
    #[test]
    fn overlay_multiple_hit_regions_union_semantics() {
        let regions = vec![
            HitRegion::new(0.0, 0.0, 100.0, 100.0),    // top-left
            HitRegion::new(500.0, 500.0, 100.0, 100.0), // bottom-right
        ];
        assert!(should_capture_pointer_event(WindowMode::Overlay, 50.0, 50.0, &regions));
        assert!(should_capture_pointer_event(WindowMode::Overlay, 550.0, 550.0, &regions));
        assert!(!should_capture_pointer_event(WindowMode::Overlay, 300.0, 300.0, &regions));
    }

    // ── WindowedConfig display properties ────────────────────────────────

    #[test]
    fn windowed_config_title_is_non_empty_by_default() {
        let cfg = WindowedConfig::default();
        assert!(!cfg.window.title.is_empty(), "default title must be non-empty");
    }

    #[test]
    fn windowed_config_dimensions_are_sensible_by_default() {
        let cfg = WindowedConfig::default();
        assert!(cfg.window.width > 0, "default width must be positive");
        assert!(cfg.window.height > 0, "default height must be positive");
    }
}
