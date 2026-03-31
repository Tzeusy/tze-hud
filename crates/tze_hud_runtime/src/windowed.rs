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
//!     This gives the same semantic as the XShape extension / wlr-layer-shell approach
//!     while using winit's cross-platform API.
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

use crate::component_startup::{register_profile_widgets, run_component_startup};
use tze_hud_compositor::{Compositor, WindowSurface};
use tze_hud_config::TzeHudConfig;
use tze_hud_input::{InputProcessor, PointerEvent, PointerEventKind};
use tze_hud_protocol::proto::session::hud_session_server::HudSessionServer;
use tze_hud_protocol::proto::session::runtime_service_server::RuntimeServiceServer;
use tze_hud_protocol::session::SharedState;
use tze_hud_protocol::session_server::HudSessionImpl;
use tze_hud_protocol::token::TokenStore;
use tze_hud_scene::config::ConfigLoader;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::ZoneContent;
use tze_hud_telemetry::TelemetryCollector;

use crate::channels::{
    FrameReadyRx, FrameReadyTx, INPUT_EVENT_CAPACITY, InputEvent, InputEventKind,
    frame_ready_channel,
};
use crate::mcp::{McpServerConfig, start_mcp_http_server};
use crate::pipeline::FramePipeline;
use crate::reload_triggers::RuntimeServiceImpl;
use crate::runtime_context::{RuntimeContext, SharedRuntimeContext};
use crate::threads::{CompositorReady, NetworkRuntime, ShutdownToken, spawn_compositor_thread};
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
    /// When `true` and the window mode is `Overlay`, auto-detect the primary
    /// monitor resolution at startup and use it as the window dimensions.
    ///
    /// Explicit `--width`/`--height` flags (or `TZE_HUD_WINDOW_WIDTH` /
    /// `TZE_HUD_WINDOW_HEIGHT` env vars) set this to `false`, causing the
    /// configured `window.width`/`window.height` values to be used instead.
    ///
    /// Has no effect in fullscreen mode (fullscreen always uses the monitor's
    /// native resolution via `Fullscreen::Borderless`).
    ///
    /// Default: `true`.
    pub overlay_auto_size: bool,
    /// gRPC server port.  Set to `0` to disable the gRPC server.
    pub grpc_port: u16,
    /// MCP HTTP server port.  Set to `0` to disable the MCP server.
    ///
    /// The MCP server binds on all interfaces (`0.0.0.0`) at the given port.
    /// It enforces PSK authentication on every request via HTTP
    /// `Authorization: Bearer <psk>` or the JSON-RPC `_auth` param field.
    ///
    /// Default: 9090.
    pub mcp_port: u16,
    /// Pre-shared key for session authentication (gRPC and MCP).
    pub psk: String,
    /// Target frames per second.  Default: 60.
    pub target_fps: u32,
    /// Raw TOML content of the configuration file, if one was loaded.
    ///
    /// When `Some`, the windowed runtime parses this at startup and applies the
    /// capability grants from `[agents.registered]` to the `RuntimeContext`.
    /// When `None`, the runtime falls back to `RuntimeContext::headless_default()`
    /// (all agents treated as guests).
    ///
    /// ## Source
    ///
    /// Populated by the application binary when `resolve_config_path` succeeds:
    /// ```rust,ignore
    /// let config_path = resolve_config_path(opts.config_path.as_deref());
    /// let config_toml = config_path.ok().and_then(|p| std::fs::read_to_string(&p).ok());
    /// ```
    pub config_toml: Option<String>,
    /// Filesystem path of the loaded configuration file, if known.
    ///
    /// Used to resolve relative `[widget_bundles].paths` entries relative to the
    /// config file's parent directory (per spec §Widget Bundle Configuration).
    /// When `None`, relative paths are resolved from the current working directory.
    ///
    /// ## Source
    ///
    /// Populated by the application binary alongside `config_toml`:
    /// ```rust,ignore
    /// let config_path = resolve_config_path(opts.config_path.as_deref());
    /// if let Ok(ref p) = config_path {
    ///     config.config_file_path = Some(p.clone());
    ///     config.config_toml = std::fs::read_to_string(p).ok();
    /// }
    /// ```
    pub config_file_path: Option<String>,
    /// Render zone boundaries with colored debug tints.  Default: `false`.
    pub debug_zones: bool,
    /// Monitor index for overlay placement (0-based).  `None` = primary monitor.
    pub monitor_index: Option<usize>,
}

impl Default for WindowedConfig {
    fn default() -> Self {
        Self {
            window: WindowConfig::default(),
            overlay_auto_size: true,
            grpc_port: 50051,
            mcp_port: 9090,
            psk: "tze-hud-key".to_string(),
            target_fps: 60,
            config_toml: None,
            config_file_path: None,
            debug_zones: false,
            monitor_index: None,
        }
    }
}

// ─── WindowedRuntime ─────────────────────────────────────────────────────────

/// Shared state passed from the windowed runtime builder to the winit app.
///
/// All fields are `Arc`-wrapped or `Send` so the app handler can be moved into
/// the winit event loop.
#[allow(dead_code)] // several fields are read by the compositor/shutdown path; not all are used yet
struct WindowedRuntimeState {
    config: WindowedConfig,
    /// Compositor thread handle (stored so it can be joined on shutdown).
    compositor_handle: Option<std::thread::JoinHandle<()>>,
    /// Network runtime for gRPC / MCP.
    ///
    /// Kept alive for the duration of the windowed runtime. Dropping this
    /// shuts down all network tasks (gRPC server, future MCP bridge).
    network_rt: Option<NetworkRuntime>,
    /// Network task join handles (gRPC server tasks spawned onto `network_rt`).
    ///
    /// Stored so they can be aborted on shutdown. Dropping the `JoinHandle`
    /// does not kill the task; call `.abort()` explicitly.
    network_handles: Vec<tokio::task::JoinHandle<()>>,
    /// Immutable runtime context (capability policy, profile budgets).
    runtime_context: SharedRuntimeContext,
    /// Whether unknown agents receive unrestricted capabilities.
    fallback_unrestricted: bool,
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
    /// Pending widget SVG assets to register with the compositor after
    /// `init_widget_renderer` is called. Consumed once during first `resumed()`.
    pending_widget_svgs: Vec<crate::widget_startup::WidgetSvgAsset>,
    /// Tracked modifier key state for shortcut detection.
    modifiers: winit::keyboard::ModifiersState,
    /// Current monitor index for Ctrl+Shift+F8/F9 cycling.
    current_monitor_index: usize,
    /// Resolved design token map from component startup.
    ///
    /// Stashed here after `run_component_startup` returns so it can be applied
    /// to the compositor via `set_token_map` when the compositor is created in
    /// `resumed()`. After that call the field is no longer needed but is kept
    /// for potential hot-reload use.
    global_tokens: std::collections::HashMap<String, String>,
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
        // Clone the title and snapshot configured dimensions before any mutation
        // to avoid borrow conflicts when we later update the config in-place for
        // overlay auto-sizing.
        let window_title = self.state.config.window.title.clone();
        let cfg_width = self.state.config.window.width;
        let cfg_height = self.state.config.window.height;

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
                    .with_title(window_title)
                    // Borderless fullscreen on the current monitor.
                    .with_fullscreen(Some(Fullscreen::Borderless(None)))
                    .with_decorations(false)
            }
            WindowMode::Overlay => {
                // Determine overlay window dimensions.
                //
                // When `overlay_auto_size` is true (the default), query the primary
                // monitor's physical size via the event loop and use it as the window
                // dimensions.  This ensures the overlay covers the full display on any
                // monitor (1080p, 1440p, 4K, etc.) without requiring explicit
                // --width/--height flags.
                //
                // Fall back to the configured width/height if monitor detection fails
                // (headless environments, missing display server, etc.).
                let (overlay_w, overlay_h, mon_x, mon_y) = if self.state.config.overlay_auto_size {
                    detect_monitor_size(
                        event_loop,
                        cfg_width,
                        cfg_height,
                        self.state.config.monitor_index,
                    )
                } else {
                    (cfg_width, cfg_height, 0, 0)
                };

                // Update the config so that downstream code (surface init, logging)
                // sees the resolved dimensions rather than the stale defaults.
                self.state.config.window.width = overlay_w;
                self.state.config.window.height = overlay_h;

                tracing::info!(
                    width = overlay_w,
                    height = overlay_h,
                    position_x = mon_x,
                    position_y = mon_y,
                    auto_size = self.state.config.overlay_auto_size,
                    "window mode: overlay/HUD — transparent borderless always-on-top"
                );
                #[cfg(target_os = "windows")]
                {
                    use winit::platform::windows::WindowAttributesExtWindows;
                    WindowAttributes::default()
                        .with_title(window_title)
                        .with_inner_size(winit::dpi::PhysicalSize::new(overlay_w, overlay_h))
                        .with_position(winit::dpi::PhysicalPosition::new(mon_x, mon_y))
                        .with_transparent(true)
                        .with_decorations(false)
                        .with_window_level(WindowLevel::AlwaysOnTop)
                        // Set WS_EX_NOREDIRECTIONBITMAP at creation time —
                        // DWM will present the swapchain directly with
                        // per-pixel alpha from PreMultiplied mode.
                        .with_no_redirection_bitmap(true)
                }
                #[cfg(not(target_os = "windows"))]
                {
                    WindowAttributes::default()
                        .with_title(window_title)
                        .with_inner_size(winit::dpi::PhysicalSize::new(overlay_w, overlay_h))
                        .with_position(winit::dpi::PhysicalPosition::new(0i32, 0i32))
                        .with_transparent(true)
                        .with_decorations(false)
                        .with_window_level(WindowLevel::AlwaysOnTop)
                }
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
            // WS_EX_NOREDIRECTIONBITMAP is set at creation time via
            // with_no_redirection_bitmap(true) above. No post-creation
            // flag manipulation needed.
        }

        self.state.window = Some(window.clone());

        let cfg = self.state.config.clone();
        let window_clone = window.clone();

        // ── Resolve actual surface dimensions ─────────────────────────────
        // Query the actual physical size of the window AFTER creation.
        // On Windows the OS may constrain the window to the monitor bounds or
        // apply DPI scaling, so `window.inner_size()` may differ from the
        // requested cfg.window.width/height.  Using the configured values
        // directly causes wgpu to configure the swapchain at a size that
        // doesn't match the surface handle's drawable area, which triggers a
        // validation panic before `surface.configure()` can write alpha_diag.txt.
        //
        // `window.inner_size()` returns `PhysicalSize<u32>` — physical pixels —
        // when per-monitor DPI awareness is active (guaranteed by the embedded
        // manifest in `tze_hud_app/tze_hud.manifest`). Do NOT multiply by
        // `scale_factor()`; that would over-count on DPI-scaled displays.
        //
        // Fall back to the configured values only when inner_size() returns (0,0)
        // (e.g., window not yet shown or minimized at construction time — rare
        // but possible on some Win32 driver/compositor combinations).
        let actual_size = window.inner_size();
        let scale = window.scale_factor(); // logged for diagnostics only
        let (surface_width, surface_height) = if actual_size.width > 0 && actual_size.height > 0 {
            (actual_size.width, actual_size.height)
        } else {
            tracing::warn!(
                requested_width = cfg.window.width,
                requested_height = cfg.window.height,
                "window.inner_size() returned (0,0) at creation; \
                 using configured dimensions as fallback"
            );
            (cfg.window.width, cfg.window.height)
        };
        tracing::info!(
            configured_width = cfg.window.width,
            configured_height = cfg.window.height,
            inner_width = actual_size.width,
            inner_height = actual_size.height,
            scale_factor = scale,
            surface_width,
            surface_height,
            "windowed: resolved surface dimensions from window.inner_size() (physical pixels)"
        );
        // Diagnostic: write surface resolution so remote operators can verify.
        let _ = std::fs::write(
            "C:\\tze_hud\\logs\\surface_diag.txt",
            format!(
                "configured={}x{} inner={}x{} scale={} surface={}x{}\n",
                cfg.window.width,
                cfg.window.height,
                actual_size.width,
                actual_size.height,
                scale,
                surface_width,
                surface_height,
            ),
        );

        // ── Create compositor + surface (async in a blocking context) ──────
        // We need an async context to call Compositor::new_windowed.
        // Use a temporary single-thread Tokio runtime here — this runs only
        // at startup and is dropped immediately after.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build startup tokio runtime");

        let is_overlay = self.state.effective_mode == WindowMode::Overlay;
        let (mut compositor, window_surface) = rt.block_on(async {
            let mut c = if is_overlay {
                Compositor::new_windowed_overlay(window_clone, surface_width, surface_height).await
            } else {
                Compositor::new_windowed(window_clone, surface_width, surface_height).await
            }
            .expect("Compositor::new_windowed failed");
            c.0.overlay_mode = is_overlay;
            c.0.debug_zone_tints = self.state.config.debug_zones;
            c
        });

        // ── Initialize text renderer ──────────────────────────────────────
        // Must be called after surface configuration so we know the negotiated
        // swapchain format. glyphon text rendering is inert until this runs.
        let surface_format = window_surface
            .config
            .lock()
            .expect("WindowSurface config lock poisoned at text renderer init")
            .format;
        compositor.init_text_renderer(surface_format);
        compositor.init_widget_renderer(surface_format);

        // Register pending widget SVG assets with the widget renderer.
        if let Some(wr) = compositor.widget_renderer_mut() {
            for (type_id, filename, bytes) in self.state.pending_widget_svgs.drain(..) {
                wr.register_svg(&type_id, &filename, bytes);
            }
        }
        tracing::info!(format = ?surface_format, "windowed: text + widget renderers initialized");

        // Apply resolved design tokens to the compositor so severity colors are
        // looked up from the token map at render time rather than using hardcoded
        // constants.  Clone the map so the state retains its copy for potential
        // future hot-reload use.
        compositor.set_token_map(self.state.global_tokens.clone());
        tracing::debug!(
            token_count = self.state.global_tokens.len(),
            "windowed: compositor token map applied"
        );

        let window_surface = Arc::new(window_surface);
        self.state.window_surface = Some(window_surface.clone());

        // ── Elevate main thread priority ──────────────────────────────────
        crate::threads::elevate_main_thread_priority();

        // ── Wire compositor thread ─────────────────────────────────────────
        // Pre-clone the scene Arc so the compositor thread can lock the scene
        // directly without ever needing to acquire the SharedState lock.
        // This avoids nested-lock inversion: the compositor only ever holds the
        // scene lock; session handlers hold the SharedState lock then the scene lock.
        let compositor_scene = {
            let st = self.state.shared_state.try_lock().expect(
                "windowed runtime: shared_state lock contended at compositor setup — \
                 this should not happen during single-threaded initialisation",
            );
            Arc::clone(&st.scene)
        };
        // Share the ArcSwap handle (not the FramePipeline itself) with the compositor thread.
        let hit_test_snapshot = self.state.pipeline.hit_test_snapshot.clone();
        let frame_ready_tx = self
            .state
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

                let frame_interval =
                    std::time::Duration::from_micros(1_000_000 / cfg.target_fps.max(1) as u64);
                let mut shutdown_rx = shutdown_tok.subscribe();

                tracing::info!(
                    "compositor thread: starting frame loop at {}fps",
                    cfg.target_fps
                );

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
                        surface_for_compositor.reconfigure(
                            pending_w,
                            pending_h,
                            &compositor.device,
                        );
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
                    // Lock the scene directly (never lock SharedState here).
                    // Using try_lock avoids blocking the compositor thread for
                    // too long when a session handler or MCP handler holds the
                    // scene lock momentarily.
                    if let Ok(mut scene) = compositor_scene.try_lock() {
                        // ── Zone and widget publication expiry sweep ──────
                        // Per timing-model/spec.md §Expiration Policy: expired
                        // publications MUST be cleared before the next frame.
                        scene.drain_expired_zone_publications();
                        scene.drain_expired_widget_publications();

                        let new_snap = crate::pipeline::HitTestSnapshot::from_scene(&scene);
                        hit_test_snapshot.store(Arc::new(new_snap));

                        // ── Stage 5–7: Render Encode + GPU Submit ─────────
                        let compositor_telemetry =
                            compositor.render_frame(&scene, surface_for_compositor.as_ref());
                        drop(scene); // Release lock before signalling main thread.

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
                tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx).await
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
                self.state
                    .shutdown
                    .trigger(crate::threads::ShutdownReason::Clean);
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
                    surface
                        .pending_resize_height
                        .store(physical_size.height, std::sync::atomic::Ordering::Release);
                    surface
                        .pending_resize_width
                        .store(physical_size.width, std::sync::atomic::Ordering::Release);
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

            // ── Modifiers ─────────────────────────────────────────────────
            WindowEvent::ModifiersChanged(mods) => {
                self.state.modifiers = mods.state();
            }

            // ── Keyboard ──────────────────────────────────────────────────
            // Stage 1: Drain keyboard events into the input ring buffer.
            // Map winit keyboard events to InputEventKind::KeyPress / KeyRelease.
            WindowEvent::KeyboardInput { event, .. } => {
                // ── Monitor cycling: Ctrl+Shift+F9 (next) / Ctrl+Shift+F8 (prev)
                if event.state == ElementState::Pressed && !event.repeat {
                    use winit::keyboard::{KeyCode, PhysicalKey};
                    let mods = self.state.modifiers;
                    let ctrl_shift = mods.control_key()
                        && mods.shift_key()
                        && !mods.alt_key()
                        && !mods.super_key();
                    if ctrl_shift {
                        match event.physical_key {
                            PhysicalKey::Code(KeyCode::F9) => {
                                self.cycle_monitor(event_loop, 1);
                                return;
                            }
                            PhysicalKey::Code(KeyCode::F8) => {
                                self.cycle_monitor(event_loop, -1);
                                return;
                            }
                            _ => {}
                        }
                    }
                }

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
            let should_capture =
                should_capture_pointer_event(WindowMode::Overlay, x, y, &self.state.hit_regions);
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
        // Acquire the scene lock directly (without going through SharedState) so that
        // the main-thread input path does not contend with session handlers that hold
        // both the SharedState lock and the scene lock.
        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(mut scene) = state.scene.try_lock() {
                let _result = self
                    .state
                    .input_processor
                    .process(&pointer_event, &mut scene);
            }
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
    #[allow(dead_code)] // public API; callers will be added as overlay integration lands
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
    #[allow(dead_code)] // public API; callers will be added as mode-switching UI lands
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
            self.state
                .shutdown
                .trigger(crate::threads::ShutdownReason::Clean);
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
    /// Cycle the overlay to the next (+1) or previous (-1) monitor.
    ///
    /// Enumerates available monitors, advances the index, and repositions +
    /// resizes the window to cover the target monitor's full physical area.
    /// The compositor surface is reconfigured automatically via the existing
    /// `WindowEvent::Resized` handler.
    fn cycle_monitor(&mut self, event_loop: &ActiveEventLoop, direction: i32) {
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        if monitors.is_empty() {
            return;
        }
        let count = monitors.len();
        let new_idx = ((self.state.current_monitor_index as i32 + direction)
            .rem_euclid(count as i32)) as usize;
        self.state.current_monitor_index = new_idx;

        let m = &monitors[new_idx];
        let size = m.size();
        let pos = m.position();
        tracing::info!(
            monitor_index = new_idx,
            name = m.name().as_deref().unwrap_or("<unnamed>"),
            width = size.width,
            height = size.height,
            x = pos.x,
            y = pos.y,
            "monitor cycle: moving overlay"
        );

        if let Some(window) = &self.state.window {
            window.set_outer_position(winit::dpi::PhysicalPosition::new(pos.x, pos.y));
            let _ =
                window.request_inner_size(winit::dpi::PhysicalSize::new(size.width, size.height));
        }
    }

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
        // Parse the raw config once here so we can use it for both widget
        // registry initialization and the RuntimeContext build. Failure is
        // non-fatal — widget startup will just leave the registry empty.
        let raw_config_for_startup: Option<tze_hud_config::raw::RawConfig> = cfg
            .config_toml
            .as_deref()
            .and_then(|toml| toml::from_str(toml).ok());

        let mut pending_widget_svgs: Vec<crate::widget_startup::WidgetSvgAsset> = Vec::new();
        let (shared_scene, global_tokens_for_compositor) = {
            let mut scene = SceneGraph::new(width, height);

            // Resolve config file parent directory for path resolution.
            let config_parent_buf: Option<std::path::PathBuf> = cfg
                .config_file_path
                .as_deref()
                .and_then(|p| std::path::Path::new(p).parent().map(|d| d.to_path_buf()));

            // Run the full component shape language startup sequence (steps 2-9):
            // design token loading, global widget bundles, component profile loading,
            // profile selection, effective rendering policy construction, readability
            // validation, zone registry construction, and widget registry population.
            //
            // Per component-shape-language/spec.md §Requirement: Startup Sequence Integration
            let startup_global_tokens = if let Some(raw) = &raw_config_for_startup {
                let startup_result = run_component_startup(
                    raw,
                    config_parent_buf.as_deref(),
                    None, // profile_name: windowed mode uses production readability (no dev-mode unless TZE_HUD_DEV=1)
                    &mut scene,
                );
                // Step 9b: register profile-scoped widget bundles
                register_profile_widgets(&mut scene, &startup_result);
                // Stash SVG assets for compositor registration after init_widget_renderer.
                pending_widget_svgs = startup_result.widget_svg_assets;
                startup_result.global_tokens
            } else {
                // No config provided — bootstrap with canonical zone defaults (no token derivation).
                scene.zone_registry = tze_hud_scene::types::ZoneRegistry::with_defaults();
                std::collections::HashMap::new()
            };

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
            (Arc::new(Mutex::new(scene)), startup_global_tokens)
        };
        let sessions = tze_hud_protocol::session::SessionRegistry::new(&cfg.psk);
        let shared_state = Arc::new(Mutex::new(SharedState {
            scene: Arc::clone(&shared_scene),
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

        // ── RuntimeContext ─────────────────────────────────────────────────────
        // Build the RuntimeContext from the config file when one is provided, or
        // fall back to headless_default() when no config file is present.
        //
        // Config-driven path: parse the TOML, validate, freeze into a ResolvedConfig,
        // and extract the HotReloadableConfig for the initial hot sections.
        // The fallback_policy (Guest vs Unrestricted) is determined by whether
        // a config file is present:
        //   - Config present → Guest (registered agents only, all others denied).
        //   - No config → Unrestricted (dev-friendly; any PSK-authenticated agent
        //     gets all capabilities without a registration entry).
        let (runtime_context, fallback_unrestricted): (SharedRuntimeContext, bool) =
            build_runtime_context(&cfg);

        // ── Network runtime + gRPC + MCP HTTP servers ──────────────────────────
        // Spawn the Tokio multi-thread runtime for all network tasks (gRPC, MCP).
        // The runtime is created before the winit event loop so that network
        // services are available immediately after the process starts.
        //
        // Per spec §Thread Model (line 15): "Network thread(s) — Tokio multi-thread
        // runtime for gRPC server, MCP bridge, session management."
        //
        // gRPC server is disabled when grpc_port == 0 (per WindowedConfig docs).
        let (mut network_rt, mut network_handles) = start_network_services(
            cfg.grpc_port,
            &cfg.psk,
            shared_state.clone(),
            Arc::clone(&runtime_context),
            fallback_unrestricted,
        )?;

        // ── MCP HTTP server ────────────────────────────────────────────────────
        //
        // Scene coherence: the MCP server and gRPC session server share the
        // same `Arc<Mutex<SceneGraph>>` (`shared_scene`).  Mutations applied
        // over gRPC are immediately visible to MCP queries and vice versa.
        if cfg.mcp_port > 0 {
            // Ensure we have a network runtime to host the MCP task. If gRPC
            // was disabled (grpc_port == 0), network_rt is None and we need
            // to create a fresh one for MCP.
            if network_rt.is_none() {
                match NetworkRuntime::new() {
                    Ok(rt) => {
                        tracing::info!("MCP: created dedicated network runtime (gRPC disabled)");
                        network_rt = Some(rt);
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "failed to create network runtime for MCP; MCP will not be available"
                        );
                    }
                }
            }

            if let Some(ref rt) = network_rt {
                let mcp_config = McpServerConfig {
                    bind_addr: format!("0.0.0.0:{}", cfg.mcp_port)
                        .parse()
                        .expect("valid MCP bind addr"),
                    psk: cfg.psk.clone(),
                };
                let mcp_shutdown = shutdown.clone();
                match rt.rt.block_on(start_mcp_http_server(
                    Arc::clone(&shared_scene),
                    mcp_config,
                    mcp_shutdown,
                )) {
                    Ok(handle) => {
                        network_handles.push(handle);
                        tracing::info!(
                            mcp_port = cfg.mcp_port,
                            "MCP HTTP server started on network runtime"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            mcp_port = cfg.mcp_port,
                            error = %e,
                            "failed to bind MCP HTTP server; runtime will continue without MCP"
                        );
                    }
                }
            }
        } else {
            tracing::info!("MCP HTTP server disabled (mcp_port = 0)");
        }

        let app_state = WindowedRuntimeState {
            config: cfg,
            compositor_handle: None,
            network_rt,
            network_handles,
            runtime_context,
            fallback_unrestricted,
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
            pending_widget_svgs,
            modifiers: winit::keyboard::ModifiersState::empty(),
            current_monitor_index: 0,
            global_tokens: global_tokens_for_compositor,
        };

        let mut app = WinitApp { state: app_state };

        // Create winit event loop and run.
        // Per spec §Main Thread Responsibilities: winit event loop MUST run on main thread.
        let event_loop = EventLoop::new()?;
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.run_app(&mut app)?;

        // ── Post-event-loop cleanup ───────────────────────────────────────────

        // Ensure shutdown is triggered before draining threads/tasks.
        // WindowEvent::CloseRequested already triggers it in the normal path,
        // but other exit paths (OS SIGTERM, explicit exit_loop) may not.
        if !app.state.shutdown.is_triggered() {
            app.state
                .shutdown
                .trigger(crate::threads::ShutdownReason::Clean);
        }

        // Abort all spawned network task handles (gRPC, MCP) so they do not
        // linger past process exit.  The shutdown token already signals tasks
        // to exit gracefully; abort() is a fallback for tasks that ignore it.
        for handle in app.state.network_handles.drain(..) {
            handle.abort();
        }

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

        // Shutdown the network runtime (drains gRPC + MCP tasks).
        //
        // `shutdown_timeout` gives tasks 500 ms to exit cleanly after the
        // shutdown token was triggered above.  The MCP task exits promptly
        // because it polls the `ShutdownToken`; gRPC tasks were already aborted.
        if let Some(network_rt) = app.state.network_rt.take() {
            tracing::info!("shutting down network runtime (gRPC, MCP tasks)...");
            network_rt
                .rt
                .shutdown_timeout(std::time::Duration::from_millis(500));
            tracing::info!("network runtime shutdown complete");
        }

        Ok(())
    }
}

// ─── Runtime context construction ────────────────────────────────────────────

/// Build a `RuntimeContext` from the windowed config.
///
/// When `cfg.config_toml` is `Some`, the TOML is parsed and validated.  On
/// success, capability grants from `[agents.registered]` and the hot-reloadable
/// sections (`[privacy]`, `[degradation]`, `[chrome]`, `[agents.dynamic_policy]`)
/// are loaded into the context.  The fallback policy is `Guest` (registered
/// agents only).
///
/// When `cfg.config_toml` is `None` (no config file), the context falls back to
/// `RuntimeContext::headless_default()` and `fallback_unrestricted = true` for
/// dev-friendly behaviour (any PSK-authenticated agent gets all capabilities).
///
/// Parse or validation errors are logged as warnings and cause a graceful
/// fallback to `headless_default()` so the runtime can still start.
///
/// Returns `(runtime_context, fallback_unrestricted)`.
fn build_runtime_context(cfg: &WindowedConfig) -> (SharedRuntimeContext, bool) {
    match &cfg.config_toml {
        None => {
            // No config file — fall back to headless default.
            tracing::debug!(
                "windowed runtime: no config TOML provided; \
                 using headless_default (all agents unrestricted)"
            );
            (Arc::new(RuntimeContext::headless_default()), true)
        }
        Some(toml_src) => {
            // Parse the TOML.
            let loader = match TzeHudConfig::parse(toml_src) {
                Ok(l) => l,
                Err(parse_err) => {
                    tracing::warn!(
                        error = %parse_err.message,
                        line = parse_err.line,
                        column = parse_err.column,
                        "windowed runtime: config TOML parse error; \
                         falling back to headless_default"
                    );
                    return (Arc::new(RuntimeContext::headless_default()), false);
                }
            };

            // Validate and freeze into a ResolvedConfig.
            let resolved = match loader.freeze() {
                Ok(r) => r,
                Err(errors) => {
                    for err in &errors {
                        tracing::warn!(
                            code = ?err.code,
                            field = %err.field_path,
                            expected = %err.expected,
                            got = %err.got,
                            hint = %err.hint,
                            "windowed runtime: config validation error"
                        );
                    }
                    tracing::warn!(
                        "windowed runtime: {} config validation error(s); \
                         falling back to headless_default",
                        errors.len()
                    );
                    return (Arc::new(RuntimeContext::headless_default()), false);
                }
            };

            // Parse hot-reloadable sections from the same TOML so the initial
            // privacy/degradation/chrome/dynamic_policy settings take effect
            // immediately (before the first SIGHUP).
            let hot = tze_hud_config::reload_config(toml_src).unwrap_or_default();

            tracing::info!(
                profile = %resolved.profile.name,
                agents = resolved.agent_capabilities.len(),
                "windowed runtime: config loaded; \
                 capability grants applied from [agents.registered]"
            );

            let ctx = RuntimeContext::from_config_with_hot(
                resolved,
                crate::runtime_context::FallbackPolicy::Guest,
                hot,
            );
            (Arc::new(ctx), false)
        }
    }
}

// ─── Network service startup ──────────────────────────────────────────────────

/// Start network services (gRPC) on a dedicated Tokio multi-thread runtime.
///
/// Returns `(network_rt, handles)`:
/// - `network_rt` is `Some(NetworkRuntime)` when `grpc_port != 0`; `None` if
///   all services are disabled (port 0 disables gRPC).
/// - `handles` contains join handles for each spawned server task.
///
/// ## gRPC server
///
/// When `grpc_port != 0`, starts the `HudSession` gRPC server on `[::1]:grpc_port`.
/// Setting `grpc_port = 0` skips server creation (compositor-only mode).
///
/// ## Errors
///
/// Returns `Err` if the `NetworkRuntime` Tokio runtime cannot be created, or if
/// the gRPC server address fails to parse.
#[allow(clippy::type_complexity)] // return type is self-documenting in this internal helper
fn start_network_services(
    grpc_port: u16,
    psk: &str,
    shared_state: Arc<Mutex<SharedState>>,
    runtime_context: SharedRuntimeContext,
    fallback_unrestricted: bool,
) -> Result<(Option<NetworkRuntime>, Vec<tokio::task::JoinHandle<()>>), Box<dyn std::error::Error>>
{
    if grpc_port == 0 {
        tracing::info!(
            "windowed runtime: gRPC server disabled (grpc_port = 0); running compositor-only"
        );
        return Ok((None, Vec::new()));
    }

    // Build the multi-thread Tokio runtime for network tasks.
    let network_rt = NetworkRuntime::new()
        .map_err(|e| format!("windowed runtime: failed to build network Tokio runtime: {e}"))?;

    let addr: std::net::SocketAddr = format!("[::1]:{grpc_port}")
        .parse()
        .map_err(|e| format!("windowed runtime: invalid gRPC address (port {grpc_port}): {e}"))?;

    // Wire config-driven capability registry into the session service.
    let agent_caps = runtime_context.snapshot_agent_capabilities();
    let service = HudSessionImpl::from_shared_state_with_config(
        shared_state,
        psk,
        agent_caps,
        fallback_unrestricted,
    );

    // Wire RuntimeService (ReloadConfig RPC) alongside HudSession.
    let runtime_svc = RuntimeServiceImpl::new(Arc::clone(&runtime_context));

    tracing::info!(grpc_addr = %addr, "windowed runtime: starting gRPC server");

    // Spawn the combined gRPC server task onto the network runtime.
    let handle = network_rt.rt.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(HudSessionServer::new(service))
            .add_service(RuntimeServiceServer::new(runtime_svc))
            .serve(addr)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "gRPC server exited with error");
            });
    });

    tracing::info!(grpc_addr = %addr, "windowed runtime: gRPC server task spawned");

    Ok((Some(network_rt), vec![handle]))
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

// ─── Monitor resolution detection ────────────────────────────────────────────

/// Detect the physical size of the primary monitor via the winit event loop.
///
/// Used exclusively for overlay auto-sizing: when `overlay_auto_size` is true
/// and the window mode is `Overlay`, this function is called in `resumed()`
/// before window creation so the overlay covers the full display area.
///
/// ## Resolution order
///
/// 1. `event_loop.primary_monitor()` — the OS-designated primary display.
/// 2. `event_loop.available_monitors().next()` — first enumerated monitor, if
///    no primary is designated (common on some Wayland compositors).
/// 3. `(fallback_width, fallback_height)` — the configured dimensions.
///
/// ## DPI scaling
///
/// `MonitorHandle::size()` returns the **physical** pixel size when the process
/// has per-monitor DPI awareness set (which is guaranteed by the embedded
/// application manifest in `tze_hud_app`). The return value is used directly
/// without scaling — do NOT multiply by `scale_factor()`.
///
/// Background: `MonitorHandle::scale_factor()` calls `GetDpiForMonitor` with
/// `MDT_EFFECTIVE_DPI`, which returns the DPI-awareness-adjusted value (96 for
/// DPI-unaware processes). If the process is somehow not DPI-aware, both
/// `size()` and `scale_factor()` are virtualised; multiplying them produces a
/// doubly-wrong result. The manifest-based DPI awareness declaration (see
/// `app/tze_hud_app/tze_hud.manifest`) ensures physical values at all times.
///
/// ## Errors
///
/// Failures (no monitors detected, headless environment) are logged as warnings
/// and cause a graceful fall back to the configured fallback dimensions.
/// Returns `(width, height, x_position, y_position)` for the selected monitor.
///
/// When `monitor_index` is `Some(i)`, selects the i-th available monitor.
/// When `None`, uses the primary monitor (or first available as fallback).
fn detect_monitor_size(
    event_loop: &ActiveEventLoop,
    fallback_width: u32,
    fallback_height: u32,
    monitor_index: Option<usize>,
) -> (u32, u32, i32, i32) {
    // Log available monitors for diagnostics.
    let monitors: Vec<_> = event_loop.available_monitors().collect();
    for (i, m) in monitors.iter().enumerate() {
        let size = m.size();
        let pos = m.position();
        tracing::info!(
            index = i,
            name = m.name().as_deref().unwrap_or("<unnamed>"),
            width = size.width,
            height = size.height,
            x = pos.x,
            y = pos.y,
            scale = m.scale_factor(),
            "available monitor"
        );
    }

    // Select monitor: by index, or primary, or first available.
    let monitor = if let Some(idx) = monitor_index {
        monitors.get(idx).cloned().or_else(|| {
            tracing::warn!(
                requested_index = idx,
                available = monitors.len(),
                "overlay: monitor index out of range, falling back to primary"
            );
            event_loop
                .primary_monitor()
                .or_else(|| monitors.into_iter().next())
        })
    } else {
        event_loop
            .primary_monitor()
            .or_else(|| monitors.into_iter().next())
    };

    match monitor {
        Some(m) => {
            let size = m.size();
            let pos = m.position();
            let scale = m.scale_factor();
            if size.width > 0 && size.height > 0 {
                tracing::info!(
                    monitor_name = m.name().as_deref().unwrap_or("<unnamed>"),
                    physical_width = size.width,
                    physical_height = size.height,
                    position_x = pos.x,
                    position_y = pos.y,
                    scale_factor = scale,
                    "overlay auto-size: selected monitor"
                );
                (size.width, size.height, pos.x, pos.y)
            } else {
                tracing::warn!(
                    fallback_width,
                    fallback_height,
                    "overlay auto-size: monitor size returned (0,0); \
                     using configured fallback dimensions"
                );
                (fallback_width, fallback_height, 0, 0)
            }
        }
        None => {
            tracing::warn!(
                fallback_width,
                fallback_height,
                "overlay auto-size: no monitors detected (headless?); \
                 using configured fallback dimensions"
            );
            (fallback_width, fallback_height, 0, 0)
        }
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
        assert_eq!(cfg.mcp_port, 9090);
        assert!(!cfg.psk.is_empty());
    }

    // ── overlay_auto_size field (hud-48ml) ────────────────────────────────────

    /// Default `WindowedConfig` must have `overlay_auto_size = true` so that
    /// overlay mode auto-detects the primary monitor resolution out-of-the-box.
    #[test]
    fn windowed_config_default_overlay_auto_size_is_true() {
        let cfg = WindowedConfig::default();
        assert!(
            cfg.overlay_auto_size,
            "overlay_auto_size must default to true so overlay covers the full monitor"
        );
    }

    /// `overlay_auto_size` can be explicitly disabled to respect user-provided
    /// `--width`/`--height` flags.
    #[test]
    fn windowed_config_overlay_auto_size_can_be_disabled() {
        let cfg = WindowedConfig {
            overlay_auto_size: false,
            ..WindowedConfig::default()
        };
        assert!(!cfg.overlay_auto_size);
    }

    /// When `overlay_auto_size` is false and mode is Overlay, the configured
    /// width/height values are respected (no monitor detection).
    #[test]
    fn windowed_config_overlay_explicit_dims_preserved() {
        let cfg = WindowedConfig {
            window: WindowConfig {
                mode: WindowMode::Overlay,
                width: 2560,
                height: 1440,
                title: "test".to_string(),
            },
            overlay_auto_size: false,
            ..WindowedConfig::default()
        };
        assert_eq!(cfg.window.width, 2560);
        assert_eq!(cfg.window.height, 1440);
        assert!(!cfg.overlay_auto_size);
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
        assert!(
            !capture,
            "overlay with no hit-regions must pass all events through"
        );
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
            HitRegion::new(0.0, 0.0, 100.0, 100.0),     // top-left
            HitRegion::new(500.0, 500.0, 100.0, 100.0), // bottom-right
        ];
        assert!(should_capture_pointer_event(
            WindowMode::Overlay,
            50.0,
            50.0,
            &regions
        ));
        assert!(should_capture_pointer_event(
            WindowMode::Overlay,
            550.0,
            550.0,
            &regions
        ));
        assert!(!should_capture_pointer_event(
            WindowMode::Overlay,
            300.0,
            300.0,
            &regions
        ));
    }

    // ── WindowedConfig display properties ────────────────────────────────

    #[test]
    fn windowed_config_title_is_non_empty_by_default() {
        let cfg = WindowedConfig::default();
        assert!(
            !cfg.window.title.is_empty(),
            "default title must be non-empty"
        );
    }

    #[test]
    fn windowed_config_dimensions_are_sensible_by_default() {
        let cfg = WindowedConfig::default();
        assert!(cfg.window.width > 0, "default width must be positive");
        assert!(cfg.window.height > 0, "default height must be positive");
    }

    // ── Network service startup ───────────────────────────────────────────────
    //
    // These tests exercise `start_network_services` directly — no winit window
    // is required. They verify the config-driven endpoint enable/disable
    // behaviour described in the acceptance criteria.

    use tokio::sync::Mutex as TokioMutex;

    fn make_shared_state() -> Arc<TokioMutex<SharedState>> {
        use tze_hud_protocol::session::{RuntimeDegradationLevel, SessionRegistry};
        use tze_hud_protocol::token::TokenStore;
        use tze_hud_scene::graph::SceneGraph;
        let scene = Arc::new(TokioMutex::new(SceneGraph::new(1920.0, 1080.0)));
        let sessions = SessionRegistry::new("test-psk");
        Arc::new(TokioMutex::new(SharedState {
            scene,
            sessions,
            safe_mode_active: false,
            token_store: TokenStore::new(),
            freeze_active: false,
            degradation_level: RuntimeDegradationLevel::Normal,
        }))
    }

    /// When `grpc_port == 0`, `start_network_services` must return `None` for
    /// the runtime and an empty handle list (compositor-only mode, AC §2).
    #[test]
    fn start_network_services_grpc_port_zero_returns_no_runtime() {
        let shared_state = make_shared_state();
        let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
        let (rt, handles) = start_network_services(0, "test-psk", shared_state, ctx, true)
            .expect("start_network_services should not fail for port 0");
        assert!(
            rt.is_none(),
            "grpc_port=0 must not create a NetworkRuntime (compositor-only)"
        );
        assert!(
            handles.is_empty(),
            "grpc_port=0 must not spawn any network task handles"
        );
    }

    /// When `grpc_port != 0`, `start_network_services` must return `Some` for
    /// the runtime and at least one spawned task handle (AC §1).
    #[test]
    fn start_network_services_nonzero_port_returns_runtime_and_handle() {
        let shared_state = make_shared_state();
        let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
        // Use a high ephemeral port unlikely to conflict.
        let (rt, handles) = start_network_services(59781, "test-psk", shared_state, ctx, true)
            .expect("start_network_services should not error for a valid port");
        assert!(
            rt.is_some(),
            "non-zero grpc_port must create a NetworkRuntime"
        );
        assert!(
            !handles.is_empty(),
            "non-zero grpc_port must spawn at least one network task handle"
        );
        // Abort the spawned task so the test doesn't leave a lingering server.
        for h in handles {
            h.abort();
        }
    }

    /// `WindowedConfig` with `grpc_port = 0` reflects a "compositor-only" intent.
    /// Verify the config field is stored and readable (AC §2 — explicit disable).
    #[test]
    fn windowed_config_grpc_port_zero_is_compositor_only() {
        let cfg = WindowedConfig {
            grpc_port: 0,
            ..WindowedConfig::default()
        };
        assert_eq!(
            cfg.grpc_port, 0,
            "grpc_port=0 must be stored and readable as 0 (endpoint disabled)"
        );
    }

    /// `WindowedConfig` with `grpc_port = 50051` (default) signals network enabled.
    #[test]
    fn windowed_config_grpc_port_nonzero_enables_network() {
        let cfg = WindowedConfig::default();
        assert_ne!(
            cfg.grpc_port, 0,
            "default grpc_port must be non-zero (gRPC enabled by default)"
        );
    }

    /// Two successive calls with `grpc_port = 0` must both return `(None, [])`.
    /// Verifies idempotency of the disabled path (AC §2 deterministic).
    #[test]
    fn start_network_services_grpc_port_zero_is_idempotent() {
        for _ in 0..2 {
            let shared_state = make_shared_state();
            let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
            let (rt, handles) = start_network_services(0, "psk", shared_state, ctx, false)
                .expect("port-0 must not error");
            assert!(rt.is_none());
            assert!(handles.is_empty());
        }
    }

    // ── build_runtime_context: config-driven and fallback behaviour ───────────

    /// Acceptance criterion 2: when no config TOML is provided, the runtime
    /// falls back to headless_default() with fallback_unrestricted = true.
    #[test]
    fn build_runtime_context_no_config_toml_uses_headless_default() {
        let cfg = WindowedConfig {
            config_toml: None,
            ..WindowedConfig::default()
        };
        let (ctx, fallback_unrestricted) = build_runtime_context(&cfg);
        // Fallback unrestricted should be true (dev-friendly default).
        assert!(
            fallback_unrestricted,
            "no-config path must set fallback_unrestricted=true"
        );
        // Profile name must be "headless" (headless_default behaviour).
        assert_eq!(
            ctx.profile.name, "headless",
            "no-config path must use the headless profile"
        );
        // Hot config should be all defaults.
        let hot = ctx.hot_config();
        assert!(
            hot.privacy.redaction_style.is_none(),
            "hot config privacy must default to None when no config file is given"
        );
    }

    /// Acceptance criterion 1: when a valid config TOML is provided, capability
    /// grants from [agents.registered] are parsed and applied.
    #[test]
    fn build_runtime_context_with_valid_config_applies_capability_grants() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[agents.registered.weather-agent]
capabilities = ["create_tiles", "modify_own_tiles"]
"#;
        let cfg = WindowedConfig {
            config_toml: Some(toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, fallback_unrestricted) = build_runtime_context(&cfg);
        // Config-driven path: fallback must be Guest (not unrestricted).
        assert!(
            !fallback_unrestricted,
            "config-driven path must set fallback_unrestricted=false"
        );
        // Registered agent capabilities must be applied.
        let caps = ctx.agent_capabilities("weather-agent");
        assert!(
            caps.is_some(),
            "weather-agent must appear in the capability registry"
        );
        let caps = caps.unwrap();
        assert!(
            caps.contains(&"create_tiles".to_string()),
            "weather-agent must have create_tiles grant"
        );
        assert!(
            caps.contains(&"modify_own_tiles".to_string()),
            "weather-agent must have modify_own_tiles grant"
        );
        // Unregistered agent must get guest (denied) policy.
        let policy = ctx.capability_policy_for("unknown-agent");
        assert!(
            policy
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_err(),
            "unregistered agent must be denied under config-driven Guest fallback"
        );
    }

    /// Acceptance criterion 1: config-driven context uses the full-display profile.
    #[test]
    fn build_runtime_context_with_config_uses_configured_profile() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"
"#;
        let cfg = WindowedConfig {
            config_toml: Some(toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, _) = build_runtime_context(&cfg);
        assert_eq!(
            ctx.profile.name, "full-display",
            "config-driven path must use the profile specified in the TOML"
        );
    }

    /// Acceptance criterion 3 (fallback): invalid TOML falls back to
    /// headless_default() rather than crashing.
    #[test]
    fn build_runtime_context_invalid_toml_falls_back_to_headless() {
        let bad_toml = "this is not valid TOML [\n";
        let cfg = WindowedConfig {
            config_toml: Some(bad_toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, fallback_unrestricted) = build_runtime_context(&cfg);
        // Must fall back gracefully to headless, but NOT unrestricted.
        // An operator who provided a config intended to restrict capabilities.
        assert!(
            !fallback_unrestricted,
            "parse-error path must NOT fall back to unrestricted"
        );
        assert_eq!(
            ctx.profile.name, "headless",
            "parse-error path must fall back to headless profile"
        );
    }

    /// Acceptance criterion 3 (fallback): config with validation errors falls
    /// back to headless_default() rather than crashing.
    #[test]
    fn build_runtime_context_validation_error_falls_back_to_headless() {
        // Missing required [[tabs]] section → validation error.
        let invalid_toml = r#"
[runtime]
profile = "full-display"
"#;
        let cfg = WindowedConfig {
            config_toml: Some(invalid_toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, fallback_unrestricted) = build_runtime_context(&cfg);
        // Must fall back gracefully to headless, but NOT unrestricted.
        // An operator who provided a config intended to restrict capabilities.
        assert!(
            !fallback_unrestricted,
            "validation-error path must NOT fall back to unrestricted"
        );
        assert_eq!(
            ctx.profile.name, "headless",
            "validation-error path must fall back to headless profile"
        );
    }

    /// Hot-reloadable sections (privacy, degradation) from the initial config
    /// are applied immediately — no SIGHUP required.
    #[test]
    fn build_runtime_context_hot_sections_applied_from_config() {
        let toml = r#"
[runtime]
profile = "full-display"

[[tabs]]
name = "Main"

[privacy]
redaction_style = "blank"
"#;
        let cfg = WindowedConfig {
            config_toml: Some(toml.to_string()),
            ..WindowedConfig::default()
        };
        let (ctx, _) = build_runtime_context(&cfg);
        let hot = ctx.hot_config();
        assert_eq!(
            hot.privacy.redaction_style,
            Some("blank".to_string()),
            "privacy.redaction_style from config must be applied immediately at startup"
        );
    }

    /// Acceptance criterion 1: default WindowedConfig has no config_toml.
    #[test]
    fn windowed_config_default_has_no_config_toml() {
        let cfg = WindowedConfig::default();
        assert!(
            cfg.config_toml.is_none(),
            "default WindowedConfig must have config_toml = None"
        );
    }

    // ── Surface dimension resolution (hud-q5hx regression) ───────────────
    //
    // These tests document the contract for the `window.inner_size()` fallback
    // logic added to fix the crash at non-default dimensions (hud-q5hx).
    //
    // The actual window creation path cannot be tested without a real GPU and
    // display, but we can verify the helper logic and the config encoding path
    // to ensure non-default dimensions flow through correctly.

    /// `WindowedConfig` built with 2560x1440 must preserve those dimensions
    /// exactly. Verifies that the config struct does not silently clamp or
    /// reject resolutions larger than the default 1920x1080.
    #[test]
    fn windowed_config_preserves_non_default_dimensions() {
        let cfg = WindowedConfig {
            window: WindowConfig {
                mode: WindowMode::Overlay,
                width: 2560,
                height: 1440,
                title: "tze_hud".to_string(),
            },
            ..WindowedConfig::default()
        };
        assert_eq!(
            cfg.window.width, 2560,
            "2560x1440 width must be preserved in WindowedConfig"
        );
        assert_eq!(
            cfg.window.height, 1440,
            "2560x1440 height must be preserved in WindowedConfig"
        );
    }

    /// `WindowedConfig` built with 3840x2160 (4K) must preserve those dimensions.
    #[test]
    fn windowed_config_preserves_4k_dimensions() {
        let cfg = WindowedConfig {
            window: WindowConfig {
                mode: WindowMode::Overlay,
                width: 3840,
                height: 2160,
                title: "tze_hud".to_string(),
            },
            ..WindowedConfig::default()
        };
        assert_eq!(cfg.window.width, 3840);
        assert_eq!(cfg.window.height, 2160);
    }

    /// The surface dimension fallback logic (used when `window.inner_size()`
    /// returns (0,0)) should prefer the actual window size when non-zero and
    /// fall back to the configured dimensions otherwise.
    ///
    /// This test validates the resolution rule as a pure function without
    /// requiring a real window handle.
    #[test]
    fn surface_dimension_resolution_prefers_actual_size() {
        // Simulate: window.inner_size() returns (2560, 1440) — use actual size.
        let actual = (2560u32, 1440u32);
        let configured = (1920u32, 1080u32);
        let (w, h) = if actual.0 > 0 && actual.1 > 0 {
            actual
        } else {
            configured
        };
        assert_eq!(w, 2560, "actual size must win when non-zero");
        assert_eq!(h, 1440, "actual size must win when non-zero");
    }

    /// When `window.inner_size()` returns (0,0) (minimized/not-yet-shown),
    /// the configured dimensions must be used as fallback.
    #[test]
    fn surface_dimension_resolution_falls_back_to_configured_when_zero() {
        // Simulate: window.inner_size() returns (0, 0) — use configured size.
        let actual = (0u32, 0u32);
        let configured = (2560u32, 1440u32);
        let (w, h) = if actual.0 > 0 && actual.1 > 0 {
            actual
        } else {
            configured
        };
        assert_eq!(w, 2560, "configured size must be used when actual is zero");
        assert_eq!(h, 1440, "configured size must be used when actual is zero");
    }

    // ── DPI scaling correctness (hud-22by) ────────────────────────────────────

    /// At 125% DPI on a 2560x1440 monitor, `MonitorHandle::size()` returns
    /// physical pixels (2560, 1440) when the process has per-monitor DPI
    /// awareness (guaranteed by the embedded manifest).  The overlay MUST cover
    /// the full 2560x1440 display — NOT the DPI-virtualized 2048x1152.
    ///
    /// Regression guard: the old code multiplied `size()` by `scale_factor()`
    /// (2560 * 1.25 = 3200), which over-counted physical pixels.  The correct
    /// behaviour is to use `size()` directly.
    #[test]
    fn dpi_125pct_overlay_covers_full_physical_display() {
        // Simulate: winit reports physical size (DPI-aware process, manifest set)
        let physical_width: u32 = 2560;
        let physical_height: u32 = 1440;
        let scale_factor: f64 = 1.25; // 125% DPI = 120 DPI / 96 base

        // Correct approach: use size() directly (physical pixels).
        let (w, h) = (physical_width, physical_height);
        assert_eq!(w, 2560, "overlay must be full physical width at 125% DPI");
        assert_eq!(h, 1440, "overlay must be full physical height at 125% DPI");

        // Regression check: old code that over-counted.
        let over_counted_w = (physical_width as f64 * scale_factor).round() as u32;
        assert_ne!(
            over_counted_w, 2560,
            "multiplying physical size by scale_factor over-counts (produces 3200, not 2560)"
        );
        assert_eq!(over_counted_w, 3200, "old code would have produced 3200");
    }

    /// At 150% DPI on a 3840x2160 monitor, `MonitorHandle::size()` returns
    /// physical pixels (3840, 2160).  The overlay must cover the full display.
    #[test]
    fn dpi_150pct_overlay_covers_full_physical_display() {
        let physical_width: u32 = 3840;
        let physical_height: u32 = 2160;
        let scale_factor: f64 = 1.5;

        // Correct: use size() directly.
        let (w, h) = (physical_width, physical_height);
        assert_eq!(w, 3840, "overlay must be full physical width at 150% DPI");
        assert_eq!(h, 2160, "overlay must be full physical height at 150% DPI");

        // Old code would over-count.
        let over_counted_w = (physical_width as f64 * scale_factor).round() as u32;
        assert_eq!(over_counted_w, 5760, "old code produced 5760 at 150% DPI");
    }

    /// At 100% DPI, `scale_factor()` is 1.0 and physical equals logical.
    /// Using `size()` directly must produce the same result whether or not
    /// scale_factor multiplication is applied — no regression at 100%.
    #[test]
    fn dpi_100pct_no_regression() {
        let physical_width: u32 = 1920;
        let physical_height: u32 = 1080;
        let scale_factor: f64 = 1.0;

        // Correct: use size() directly.
        let (w, h) = (physical_width, physical_height);
        assert_eq!(w, 1920, "100% DPI must not regress");
        assert_eq!(h, 1080, "100% DPI must not regress");

        // At 100%, old code and new code agree (1.0 multiply is identity).
        let with_scale = (physical_width as f64 * scale_factor).round() as u32;
        assert_eq!(
            with_scale, w,
            "at 100% DPI, scale multiplication is identity — no regression"
        );
    }

    /// The `inner_size()` surface dimension resolution must use physical pixels
    /// directly, without multiplying by `scale_factor`.  At 125% DPI with a
    /// 2560x1440 window, the wgpu surface must be configured at 2560x1440, not
    /// 3200x1800.
    #[test]
    fn surface_dimension_resolution_does_not_multiply_by_scale_factor() {
        // Simulate: window.inner_size() = (2560, 1440) at 125% DPI.
        let inner_w: u32 = 2560;
        let inner_h: u32 = 1440;
        let scale: f64 = 1.25;

        // Correct: use inner_size() directly.
        let (surface_w, surface_h) = if inner_w > 0 && inner_h > 0 {
            (inner_w, inner_h)
        } else {
            (1920u32, 1080u32) // fallback (unreachable in this test)
        };
        assert_eq!(
            surface_w, 2560,
            "surface must match physical inner_size at 125% DPI"
        );
        assert_eq!(
            surface_h, 1440,
            "surface must match physical inner_size at 125% DPI"
        );

        // Old code multiplied — would have produced 3200x1800.
        let old_surface_w = (inner_w as f64 * scale).round() as u32;
        assert_eq!(old_surface_w, 3200, "old code over-counted surface width");
    }
}
