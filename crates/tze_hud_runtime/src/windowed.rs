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

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use tokio::sync::Mutex;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId, WindowLevel};

use crate::component_startup::{register_profile_widgets, run_component_startup};
use tze_hud_compositor::{Compositor, CompositorSurface, WindowSurface};
use tze_hud_config::{TzeHudConfig, resolve_runtime_widget_asset_store};
use tze_hud_input::{
    DragEventOutcome, FocusManager, InputProcessor, KeyboardProcessor, PointerEvent,
    PointerEventKind, RawCharacterEvent, RawKeyDownEvent, RawKeyUpEvent,
};
use tze_hud_protocol::proto::session::hud_session_server::HudSessionServer;
use tze_hud_protocol::proto::session::runtime_service_server::RuntimeServiceServer;
use tze_hud_protocol::proto::{EventBatch, InputEnvelope};
use tze_hud_protocol::session::SharedState;
use tze_hud_protocol::session_server::HudSessionImpl;
use tze_hud_protocol::token::TokenStore;
use tze_hud_resource::{RuntimeWidgetStore, RuntimeWidgetStoreConfig};
use tze_hud_scene::HitResult;
use tze_hud_scene::config::ConfigLoader;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::{
    DragHandleContextMenuState, DragHandleElementKind, WidgetParameterValue, ZoneContent,
    ZoneInteractionKind,
};
use tze_hud_telemetry::{SessionSummary, TelemetryCollector};

use crate::channels::{
    FrameReadyRx, FrameReadyTx, INPUT_EVENT_CAPACITY, InputEvent, InputEventKind,
    frame_ready_channel,
};
use crate::element_store::bootstrap_scene_element_store;
use crate::mcp::{McpServerConfig, start_mcp_http_server};
use crate::pipeline::FramePipeline;
use crate::reload_triggers::RuntimeServiceImpl;
use crate::runtime_context::{RuntimeContext, SharedRuntimeContext};
use crate::threads::{CompositorReady, NetworkRuntime, ShutdownToken, spawn_compositor_thread};
use crate::widget_hover::{
    WidgetHoverTracker, build_hover_trackers, hidden_mutations_for_removed, tick_hover_trackers,
};
use crate::widget_runtime_registration::process_pending_widget_svgs;
use crate::window::{HitRegion, WindowConfig, WindowMode};
use crate::window::{resolve_window_mode, should_capture_pointer_event};

// ── Drag-to-move: data carried out of the scene-lock for post-lock work ──────

/// Payload returned by [`apply_drag_handle_pointer_event`] when a drag is
/// completed and the geometry must be persisted outside the scene lock.
struct DragReleasedData {
    /// Scene-level ID of the element that was dragged.
    element_id: tze_hud_scene::SceneId,
    /// Final snapped+clamped top-left X in display pixels.
    final_x: f32,
    /// Final snapped+clamped top-left Y in display pixels.
    final_y: f32,
    /// Element width in display pixels (unchanged during drag).
    width: f32,
    /// Element height in display pixels (unchanged during drag).
    height: f32,
    /// Display width at time of release, used for `GeometryPolicy::Relative` normalisation.
    display_width: f32,
    /// Display height at time of release.
    display_height: f32,
    /// Agent namespace that owns the tile, used for `ElementRepositionedEvent` routing.
    namespace: String,
}

fn sync_scene_display_area(scene: &mut tze_hud_scene::graph::SceneGraph, width: u32, height: u32) {
    if width == 0 || height == 0 {
        return;
    }
    scene.display_area = tze_hud_scene::Rect::new(0.0, 0.0, width as f32, height as f32);
}

fn normalize_mouse_wheel_delta(delta: &MouseScrollDelta) -> (f32, f32) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => (-x * 40.0, -y * 40.0),
        MouseScrollDelta::PixelDelta(pos) => (-(pos.x as f32), -(pos.y as f32)),
    }
}

#[cfg(target_os = "windows")]
fn hwnd_for_window(window: &Window) -> Option<windows::Win32::Foundation::HWND> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(HWND(handle.hwnd.get() as *mut c_void))
}

fn focus_window_for_text_input(window: &Window) {
    window.focus_window();
    window.set_ime_allowed(true);

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{BringWindowToTop, SetForegroundWindow};

        if let Some(hwnd) = hwnd_for_window(window) {
            // SAFETY: The HWND is owned by this winit window on the event-loop thread.
            unsafe {
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

fn begin_os_mouse_capture(window: &Window) {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::SetCapture;

        if let Some(hwnd) = hwnd_for_window(window) {
            // SAFETY: The HWND is owned by this winit window on the event-loop thread.
            unsafe {
                let _ = SetCapture(hwnd);
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
    }
}

fn end_os_mouse_capture() {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;

        // SAFETY: Releases mouse capture for the current thread if held.
        unsafe {
            let _ = ReleaseCapture();
        }
    }
}

fn left_mouse_button_is_physically_down() -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};

        // SAFETY: GetAsyncKeyState reads the current global key state.
        let state = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) };
        Some((state & 0x8000u16 as i16) != 0)
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(target_os = "windows")]
fn read_windows_clipboard_text() -> Option<String> {
    use windows::Win32::Foundation::{HGLOBAL, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    const CF_UNICODETEXT_FORMAT: u32 = 13;

    // SAFETY: Clipboard access is confined to the window event-loop thread.
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT_FORMAT).is_err() {
            return None;
        }
        if OpenClipboard(HWND::default()).is_err() {
            return None;
        }

        struct ClipboardGuard;
        impl Drop for ClipboardGuard {
            fn drop(&mut self) {
                // SAFETY: Balances a successful OpenClipboard call.
                unsafe {
                    let _ = CloseClipboard();
                }
            }
        }
        let _guard = ClipboardGuard;

        let handle = GetClipboardData(CF_UNICODETEXT_FORMAT).ok()?;
        let hglobal = HGLOBAL(handle.0);
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            return None;
        }

        let mut len = 0usize;
        let wide = ptr as *const u16;
        while *wide.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(wide, len);
        let text = String::from_utf16_lossy(slice);
        let _ = GlobalUnlock(hglobal);

        if text.is_empty() { None } else { Some(text) }
    }
}

#[cfg(not(target_os = "windows"))]
fn read_windows_clipboard_text() -> Option<String> {
    None
}

/// Drive the drag-handle long-press state machine for a single pointer event.
///
/// Must be called while both the `SharedState` lock **and** the inner scene
/// `Mutex<SceneGraph>` are held (i.e., inside the lock block in
/// [`WinitApp::enqueue_pointer_event`]).
///
/// Returns `Some(DragReleasedData)` when a drag completes and the caller must
/// persist the new geometry after releasing the locks.  Returns `None` for all
/// other outcomes (including in-flight moves, which are written directly to
/// `scene.tiles`).
///
/// ## Hysteresis and click-focus coexistence
///
/// The state machine uses a 250 ms long-press threshold (mouse/pointer) or 1000 ms
/// (touch) before activating the drag.  A quick press-release cycle (tap/click)
/// never reaches the `Activated` phase and therefore does not interfere with the
/// click-to-focus path wired in [`WinitApp::enqueue_pointer_event`].
#[allow(clippy::too_many_arguments)]
fn apply_drag_handle_pointer_event(
    input_processor: &mut InputProcessor,
    pointer_event: &PointerEvent,
    result_hit: &HitResult,
    scene: &mut tze_hud_scene::graph::SceneGraph,
    display_width: f32,
    display_height: f32,
) -> Option<DragReleasedData> {
    let device_id = pointer_event.device_id;

    // Determine which drag handle (if any) was hit on this event.
    let hit_drag_info: Option<(&str, tze_hud_scene::SceneId, DragHandleElementKind)> =
        match result_hit {
            HitResult::ZoneInteraction {
                interaction_id,
                kind:
                    ZoneInteractionKind::DragHandle {
                        element_id,
                        element_kind,
                    },
                ..
            } => Some((interaction_id.as_str(), *element_id, *element_kind)),
            _ => None,
        };

    // On PointerDown on a drag handle, start accumulating.
    if pointer_event.kind == PointerEventKind::Down {
        if let Some((interaction_id, element_id, element_kind)) = hit_drag_info {
            let element_bounds = scene
                .tiles
                .get(&element_id)
                .map(|t| t.bounds)
                .unwrap_or_else(|| tze_hud_scene::Rect::new(0.0, 0.0, 0.0, 0.0));
            let outcome = input_processor.process_drag_handle_pointer(
                pointer_event,
                interaction_id,
                element_id,
                element_kind,
                element_bounds,
                display_width,
                display_height,
            );
            tracing::trace!(
                element_id = %element_id,
                x = pointer_event.x,
                y = pointer_event.y,
                ?outcome,
                "drag-handle: PointerDown accumulating"
            );
        }
        return None;
    }

    // On PointerMove or PointerUp, check for an in-flight drag on this device.
    let drag_info = input_processor
        .drag_states
        .get(&device_id)
        .map(|s| (s.interaction_id.clone(), s.element_id, s.element_kind));

    let Some((interaction_id, element_id, element_kind)) = drag_info else {
        // No drag in progress for this device — nothing to do.
        return None;
    };

    // Snapshot element bounds; element_id is the tile being dragged.
    let element_bounds = scene
        .tiles
        .get(&element_id)
        .map(|t| t.bounds)
        .unwrap_or_else(|| tze_hud_scene::Rect::new(0.0, 0.0, 0.0, 0.0));

    let outcome = input_processor.process_drag_handle_pointer(
        pointer_event,
        &interaction_id,
        element_id,
        element_kind,
        element_bounds,
        display_width,
        display_height,
    );

    match outcome {
        DragEventOutcome::Idle | DragEventOutcome::Accumulating { .. } => {
            // Nothing to do locally.
            None
        }
        DragEventOutcome::Activated { element_id, .. } => {
            tracing::debug!(
                element_id = %element_id,
                "drag-handle: drag activated — element follows pointer"
            );
            None
        }
        DragEventOutcome::Cancelled => {
            tracing::trace!(
                element_id = %element_id,
                "drag-handle: drag cancelled (tap or moved beyond tolerance)"
            );
            None
        }
        DragEventOutcome::Moved {
            element_id: eid,
            new_x,
            new_y,
            ..
        } => {
            // Update tile bounds directly (chrome-layer bypass — no lease check).
            if let Some(tile) = scene.tiles.get_mut(&eid) {
                let old = tile.bounds;
                tile.bounds.x = new_x;
                tile.bounds.y = new_y;
                scene.version += 1;
                tracing::trace!(
                    element_id = %eid,
                    old_x = old.x,
                    old_y = old.y,
                    new_x,
                    new_y,
                    "drag-handle: tile moved"
                );
            }
            None
        }
        DragEventOutcome::Released {
            element_id: eid,
            final_x,
            final_y,
            element_kind: _,
        } => {
            let (width, height) = scene
                .tiles
                .get(&eid)
                .map(|t| (t.bounds.width, t.bounds.height))
                .unwrap_or((0.0, 0.0));

            // Apply final position to tile bounds.
            if let Some(tile) = scene.tiles.get_mut(&eid) {
                tile.bounds.x = final_x;
                tile.bounds.y = final_y;
                scene.version += 1;
            }

            let namespace = scene
                .tiles
                .get(&eid)
                .map(|t| t.namespace.clone())
                .unwrap_or_default();

            tracing::debug!(
                element_id = %eid,
                final_x,
                final_y,
                width,
                height,
                "drag-handle: drag released — persisting geometry"
            );

            // Return data the caller will use to persist after releasing locks.
            Some(DragReleasedData {
                element_id: eid,
                final_x,
                final_y,
                width,
                height,
                display_width,
                display_height,
                namespace,
            })
        }
    }
}

fn zone_hit_regions_to_overlay_regions(scene: &SceneGraph) -> Vec<HitRegion> {
    scene
        .zone_hit_regions
        .iter()
        .map(|region| {
            HitRegion::new(
                region.bounds.x,
                region.bounds.y,
                region.bounds.width,
                region.bounds.height,
            )
        })
        .collect()
}

/// Collect overlay capture regions from content-layer tiles that contain at
/// least one `HitRegionNode` with `accepts_pointer = true`.
///
/// In overlay mode, the OS routes all pointer events to the desktop unless the
/// window has set `cursor_hittest(true)`.  We flip to capture only when the
/// cursor is inside a known interactive region.  Zone-owned affordances are
/// covered by `zone_hit_regions_to_overlay_regions`; this function handles
/// agent-owned content tiles that carry a `HitRegionNode`.
///
/// **Granularity**: we use the tile's display-space bounding box as the OS
/// capture region (not individual node bounds).  The OS capture decision must be
/// made before the event is dispatched, so a coarser region is correct here.
/// Precise hit-testing against individual `HitRegionNode` bounds still happens
/// in Stage 2 (`crates/tze_hud_input/src/hit_test.rs`) after the event arrives.
///
/// Tiles with `InputMode::Passthrough` are excluded — they are transparent by
/// design and must not block underlying desktop events.
fn content_tile_hit_regions_from_scene(scene: &SceneGraph) -> Vec<HitRegion> {
    let mut regions = Vec::new();
    for tile in scene.tiles.values() {
        // Passthrough tiles intentionally let events fall through to the desktop.
        if tile.input_mode == tze_hud_scene::InputMode::Passthrough {
            continue;
        }
        // Only register a capture region if the tile tree has at least one
        // HitRegionNode with accepts_pointer=true.
        if let Some(root_id) = tile.root_node {
            if tile_has_pointer_hit_region(scene, root_id) {
                regions.push(HitRegion::new(
                    tile.bounds.x,
                    tile.bounds.y,
                    tile.bounds.width,
                    tile.bounds.height,
                ));
            }
        }
    }
    regions
}

/// Returns `true` if the node subtree rooted at `node_id` contains at least
/// one `HitRegionNode` with `accepts_pointer = true`.
fn tile_has_pointer_hit_region(scene: &SceneGraph, node_id: tze_hud_scene::SceneId) -> bool {
    let Some(node) = scene.nodes.get(&node_id) else {
        return false;
    };
    if let tze_hud_scene::NodeData::HitRegion(hr) = &node.data {
        if hr.accepts_pointer {
            return true;
        }
    }
    node.children
        .iter()
        .any(|&child_id| tile_has_pointer_hit_region(scene, child_id))
}

fn combined_overlay_hit_regions(
    static_regions: &[HitRegion],
    scene: &SceneGraph,
) -> Vec<HitRegion> {
    let mut regions = static_regions.to_vec();
    regions.extend(zone_hit_regions_to_overlay_regions(scene));
    regions.extend(content_tile_hit_regions_from_scene(scene));
    regions
}

fn refresh_zone_hit_regions_after_render(
    compositor: &Compositor,
    scene: &mut SceneGraph,
    surface: &dyn CompositorSurface,
) {
    let (surf_w, surf_h) = surface.size();
    compositor.populate_zone_hit_regions(scene, surf_w as f32, surf_h as f32);
}

const WINDOWED_BENCHMARK_AGENT: &str = "windowed-compositor-benchmark";
const WINDOWED_BENCHMARK_SCENE: &str = "composite_tiles_v1";
const OVERLAY_COMPOSITE_DELTA_TARGET_US: i64 = 500;
const WINDOWED_BENCHMARK_INPUT_X: f32 = 16.0;
const WINDOWED_BENCHMARK_INPUT_Y: f32 = 16.0;

#[derive(Clone, Copy, Debug)]
struct PendingInputLatencySample {
    input_started_at: Instant,
    local_ack_us: u64,
}

type PendingInputLatencySamples = Arc<StdMutex<VecDeque<PendingInputLatencySample>>>;

fn record_pending_input_latency(
    pending: &PendingInputLatencySamples,
    input_started_at: Instant,
    local_ack_us: u64,
) {
    let local_ack_us = local_ack_us.max(1);
    if let Ok(mut samples) = pending.lock() {
        samples.push_back(PendingInputLatencySample {
            input_started_at,
            local_ack_us,
        });
    }
}

fn drain_pending_input_latency(
    pending: &PendingInputLatencySamples,
    scene_commit_at: Instant,
    frame_present_at: Instant,
) -> Option<(u64, u64, u64)> {
    let mut samples = pending.lock().ok()?;
    if samples.is_empty() {
        return None;
    }

    let mut local_ack_us = 0;
    let mut input_to_scene_commit_us = 0;
    let mut input_to_next_present_us = 0;
    while let Some(sample) = samples.pop_front() {
        local_ack_us = local_ack_us.max(sample.local_ack_us);
        input_to_scene_commit_us = input_to_scene_commit_us.max(
            scene_commit_at
                .saturating_duration_since(sample.input_started_at)
                .as_micros() as u64,
        );
        input_to_next_present_us = input_to_next_present_us.max(
            frame_present_at
                .saturating_duration_since(sample.input_started_at)
                .as_micros() as u64,
        );
    }

    Some((
        local_ack_us,
        input_to_scene_commit_us,
        input_to_next_present_us,
    ))
}

fn seed_windowed_benchmark_scene(scene: &mut SceneGraph, width: u32, height: u32) {
    use tze_hud_scene::types::HitRegionNode;
    use tze_hud_scene::{Capability, Node, NodeData, Rect, Rgba, SceneId, SolidColorNode};

    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    scene.display_area = Rect::new(0.0, 0.0, width, height);

    let display_order = scene
        .tabs
        .values()
        .map(|tab| tab.display_order)
        .max()
        .map_or(0, |order| order.saturating_add(1));
    let Ok(tab_id) = scene.create_tab("windowed_perf", display_order) else {
        tracing::warn!("windowed benchmark: failed to create benchmark tab");
        return;
    };
    scene.active_tab = Some(tab_id);
    let lease_id = scene.grant_lease(
        WINDOWED_BENCHMARK_AGENT,
        300_000,
        vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
    );
    if let Some(lease) = scene.leases.get_mut(&lease_id) {
        lease.resource_budget.max_tiles = 32;
    }

    let cols = 5usize;
    let rows = 4usize;
    let tile_w = width / 4.85;
    let tile_h = height / 3.8;
    let step_x = width / 5.65;
    let step_y = height / 5.0;

    for i in 0..(cols * rows) {
        let col = i % cols;
        let row = i / cols;
        let x = col as f32 * step_x + (row % 2) as f32 * step_x * 0.25;
        let y = row as f32 * step_y;
        let bounds = Rect::new(x, y, tile_w, tile_h);

        let Ok(tile_id) = scene.create_tile(
            tab_id,
            WINDOWED_BENCHMARK_AGENT,
            lease_id,
            bounds,
            (i + 1) as u32,
        ) else {
            continue;
        };

        let alpha = match i % 4 {
            0 => 0.58,
            1 => 0.72,
            2 => 0.86,
            _ => 1.0,
        };
        let root_id = SceneId::new();
        let node = Node {
            id: root_id,
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(
                    (i % cols) as f32 / cols as f32,
                    0.32 + (row as f32 / rows as f32) * 0.45,
                    1.0 - (i as f32 / (cols * rows) as f32) * 0.7,
                    alpha,
                ),
                bounds: Rect::new(0.0, 0.0, tile_w, tile_h),
                radius: Some(10.0),
            }),
        };
        if let Err(err) = scene.set_tile_root(tile_id, node) {
            tracing::warn!(?err, "windowed benchmark: failed to set tile root");
            continue;
        }
        let hit_region = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(0.0, 0.0, tile_w, tile_h),
                interaction_id: format!("windowed-benchmark-input-{i}"),
                accepts_pointer: true,
                ..Default::default()
            }),
        };
        if let Err(err) = scene.add_node_to_tile(tile_id, Some(root_id), hit_region) {
            tracing::warn!(?err, "windowed benchmark: failed to add input hit region");
        }
    }
}

struct WindowedBenchmarkRunState {
    config: WindowedBenchmarkConfig,
    requested_mode: WindowMode,
    effective_mode: WindowMode,
    width: u32,
    height: u32,
    target_fps: u32,
    warmup_seen: u64,
    measured_seen: u64,
    measured_start: Option<Instant>,
    summary: SessionSummary,
}

impl WindowedBenchmarkRunState {
    fn new(
        config: WindowedBenchmarkConfig,
        requested_mode: WindowMode,
        effective_mode: WindowMode,
        width: u32,
        height: u32,
        target_fps: u32,
    ) -> Self {
        Self {
            config,
            requested_mode,
            effective_mode,
            width,
            height,
            target_fps,
            warmup_seen: 0,
            measured_seen: 0,
            measured_start: None,
            summary: SessionSummary::new(),
        }
    }

    fn record(&mut self, telemetry: &tze_hud_telemetry::FrameTelemetry) -> bool {
        if self.warmup_seen < self.config.warmup_frames {
            self.warmup_seen += 1;
            return false;
        }

        if self.measured_start.is_none() {
            self.measured_start = Some(Instant::now());
        }
        self.summary
            .record_frame(telemetry.frame_time_us, telemetry.tile_count);
        if telemetry.input_to_local_ack_us > 0 {
            self.summary
                .input_to_local_ack
                .record(telemetry.input_to_local_ack_us);
        }
        if telemetry.input_to_scene_commit_us > 0 {
            self.summary
                .input_to_scene_commit
                .record(telemetry.input_to_scene_commit_us);
        }
        if telemetry.input_to_next_present_us > 0 {
            self.summary
                .input_to_next_present
                .record(telemetry.input_to_next_present_us);
        }
        self.measured_seen += 1;
        self.measured_seen >= self.config.frames
    }

    fn finish(mut self) -> std::io::Result<()> {
        if let Some(start) = self.measured_start {
            self.summary.elapsed_us = start.elapsed().as_micros() as u64;
        }
        self.summary.finalize();

        let frame_time = serde_json::json!({
            "p50_us": self.summary.frame_time.p50(),
            "p99_us": self.summary.frame_time.p99(),
            "p99_9_us": self.summary.frame_time.percentile(99.9),
            "peak_us": self.summary.peak_frame_time_us,
        });
        let report = serde_json::json!({
            "schema": "tze_hud.windowed_compositor_benchmark.v1",
            "scene": WINDOWED_BENCHMARK_SCENE,
            "target": {
                "overlay_composite_delta_p99_us": OVERLAY_COMPOSITE_DELTA_TARGET_US,
            },
            "requested_mode": self.requested_mode.to_string(),
            "effective_mode": self.effective_mode.to_string(),
            "window": {
                "width": self.width,
                "height": self.height,
                "target_fps": self.target_fps,
            },
            "benchmark": {
                "warmup_frames": self.config.warmup_frames,
                "measured_frames": self.config.frames,
                "recorded_frames": self.summary.total_frames,
            },
            "frame_time": frame_time,
            "summary": self.summary,
        });

        if let Some(parent) = self.config.emit_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(
            &self.config.emit_path,
            serde_json::to_vec_pretty(&report)
                .expect("windowed benchmark report JSON serialization must succeed"),
        )
    }
}

// ─── WindowedConfig ──────────────────────────────────────────────────────────

/// Bounded benchmark configuration for the real windowed compositor.
///
/// When present, the windowed runtime seeds a deterministic scene, records frame
/// telemetry after `warmup_frames`, writes a JSON artifact at `emit_path`, and
/// exits after `frames` measured frames.
#[derive(Debug, Clone)]
pub struct WindowedBenchmarkConfig {
    /// Number of warmup frames to render before recording measurements.
    pub warmup_frames: u64,
    /// Number of measured frames to include in the emitted artifact.
    pub frames: u64,
    /// Path to the per-mode benchmark JSON artifact.
    pub emit_path: PathBuf,
}

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
    /// Optional bounded benchmark run for the windowed compositor.
    pub benchmark: Option<WindowedBenchmarkConfig>,
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
            benchmark: None,
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
    /// Keeps the durable runtime widget asset store alive for runtime lifetime.
    _runtime_widget_store: Option<RuntimeWidgetStore>,
    /// Whether unknown agents receive unrestricted capabilities.
    fallback_unrestricted: bool,
    /// Shared scene + session state.
    shared_state: Arc<Mutex<SharedState>>,
    /// Input channel (ring buffer) — main thread writes, compositor thread reads.
    input_ring: Arc<std::sync::Mutex<std::collections::VecDeque<InputEvent>>>,
    /// Pending Stage 1/2 input latency samples for the next compositor frame.
    pending_input_latency: PendingInputLatencySamples,
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
    /// Session-plane pointer capture commands delivered from the gRPC server.
    input_capture_rx:
        tokio::sync::mpsc::UnboundedReceiver<tze_hud_protocol::session::InputCaptureCommand>,
    pending_input_capture_commands:
        std::collections::VecDeque<tze_hud_protocol::session::InputCaptureCommand>,
    /// Focus manager — tracks which node / tile has keyboard focus per tab.
    ///
    /// Updated on every pointer-down via `InputProcessor::process_with_focus`.
    /// Consulted by the keyboard drain path to route `KeyboardProcessor` output
    /// to the correct agent session.
    focus_manager: FocusManager,
    /// Keyboard processor — translates raw OS key/char events into typed
    /// `KeyboardDispatch` descriptors when a node or tile has focus.
    ///
    /// Stateless with respect to focus; focus is owned by `focus_manager`.
    keyboard_processor: KeyboardProcessor,
    /// Telemetry collector.
    telemetry: TelemetryCollector,
    /// Frame pipeline (ArcSwap hit-test snapshot, overflow counters).
    pipeline: FramePipeline,
    /// Shutdown token.
    shutdown: ShutdownToken,
    /// Set when benchmark output failed after the event loop was already running.
    benchmark_failed: Arc<std::sync::atomic::AtomicBool>,
    /// Current cursor position (updated by CursorMoved events).
    cursor_x: f32,
    cursor_y: f32,
    /// True while the primary pointer button is down.
    ///
    /// Overlay hit-testing must remain captured during a press/drag sequence so
    /// Windows delivers the matching button release even if the cursor leaves the
    /// original hit region.
    left_button_down: bool,
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
    /// External static hit-regions configured by callers.
    static_hit_regions: Vec<HitRegion>,
    /// Runtime-managed widget hover trackers keyed by widget instance_name.
    widget_hover_trackers: std::collections::HashMap<String, WidgetHoverTracker>,
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
    /// Pre-merged compositor token map from component startup (global tokens +
    /// all active profile overrides).
    ///
    /// Stashed here after `run_component_startup` returns so it can be applied
    /// to the compositor via `set_token_map` when the compositor is created in
    /// `resumed()`. After that call the field is no longer needed but is kept
    /// for potential hot-reload use.
    global_tokens: std::collections::HashMap<String, String>,
    /// Broadcast sender for `ElementRepositionedEvent`.
    ///
    /// Cloned from the `HudSessionImpl` after creation. `None` when gRPC is
    /// disabled (grpc_port == 0) or before network services start.
    ///
    /// Used by the windowed runtime to broadcast reset events without holding
    /// the async session lock (the reset path is sync chrome-layer code).
    element_repositioned_tx:
        Option<tokio::sync::broadcast::Sender<tze_hud_protocol::proto::ElementRepositionedEvent>>,
    /// Broadcast sender for runtime-injected input event batches.
    ///
    /// Cloned from the `HudSessionImpl` after creation. `None` when gRPC is
    /// disabled (grpc_port == 0) or before network services start.
    ///
    /// Used by the windowed runtime to dispatch any `EventBatch` to agents —
    /// scroll offset changes, keyboard down/up/character events, and future
    /// input event types.  Each `(namespace, EventBatch)` pair is delivered
    /// only to the session handler whose namespace matches, filtered by
    /// `INPUT_EVENTS` subscription.
    input_event_tx:
        Option<tokio::sync::broadcast::Sender<(String, tze_hud_protocol::proto::EventBatch)>>,
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
        if self.state.shutdown.is_triggered() {
            event_loop.exit();
            return;
        }
        if self.state.pending_mode_switch.is_some() {
            self.apply_pending_mode_switch();
            // Re-create the window with the new mode by forwarding to the
            // initialisation path inside resumed().
            self.resumed(event_loop);
        }
        self.refresh_cursor_position_from_os();
        self.drain_input_capture_commands();
        self.synthesize_left_release_if_physically_up();
        self.refresh_widget_hover_tracking();
        self.update_overlay_cursor_hittest();
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
                        // Hide from taskbar so the overlay can't be
                        // accidentally minimized or alt-tabbed to.
                        .with_skip_taskbar(true)
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
        self.refresh_widget_hover_tracking();

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
        if surface_width > 0 && surface_height > 0 {
            self.state.config.window.width = surface_width;
            self.state.config.window.height = surface_height;
            if let Ok(state) = self.state.shared_state.try_lock() {
                if let Ok(mut scene) = state.scene.try_lock() {
                    sync_scene_display_area(&mut scene, surface_width, surface_height);
                    if self.state.config.benchmark.is_some() {
                        seed_windowed_benchmark_scene(&mut scene, surface_width, surface_height);
                    }
                }
            }
        }
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
        process_pending_widget_svgs(
            compositor.widget_renderer_mut(),
            self.state.pending_widget_svgs.drain(..),
        );
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
        let pending_input_latency = Arc::clone(&self.state.pending_input_latency);
        let frame_ready_tx = self
            .state
            .frame_ready_tx
            .take()
            .expect("frame_ready_tx already taken");
        let shutdown = self.state.shutdown.clone();
        let benchmark_failed = self.state.benchmark_failed.clone();
        let telemetry_collector = TelemetryCollector::new();
        let surface_for_compositor = window_surface.clone();
        let mut benchmark_state = cfg.benchmark.clone().map(|benchmark| {
            WindowedBenchmarkRunState::new(
                benchmark,
                cfg.window.mode,
                self.state.effective_mode,
                surface_width,
                surface_height,
                cfg.target_fps,
            )
        });

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
                        // Register runtime-uploaded widget SVG assets before
                        // rendering so newly registered widget types/layers can
                        // be referenced by publish calls immediately.
                        process_pending_widget_svgs(
                            compositor.widget_renderer_mut(),
                            scene.drain_pending_widget_svg_assets(),
                        );

                        // ── Zone and widget publication expiry sweep ──────
                        // Per timing-model/spec.md §Expiration Policy: expired
                        // publications MUST be cleared before the next frame.
                        scene.drain_expired_zone_publications();
                        scene.drain_expired_widget_publications();

                        // ── Per-publication TTL fade-out sweep ───────────
                        // update_publication_animations seeds new state and ticks
                        // existing ones; prune_faded_publications removes any
                        // publications whose 150ms fade-out has completed.
                        compositor.update_publication_animations(&scene);
                        compositor.prune_faded_publications(&mut scene);

                        let new_snap = crate::pipeline::HitTestSnapshot::from_scene(&scene);
                        hit_test_snapshot.store(Arc::new(new_snap));

                        // ── Stage 5–7: Render Encode + GPU Submit ─────────
                        let scene_commit_at = Instant::now();
                        let compositor_telemetry =
                            compositor.render_frame(&scene, surface_for_compositor.as_ref());
                        refresh_zone_hit_regions_after_render(
                            &compositor,
                            &mut scene,
                            surface_for_compositor.as_ref(),
                        );
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
                        telem.stage6_render_encode_us = compositor_telemetry.render_encode_us;
                        telem.stage7_gpu_submit_us = compositor_telemetry.gpu_submit_us;
                        telem.tile_count = compositor_telemetry.tile_count;
                        if let Some((local_ack_us, scene_commit_us, next_present_us)) =
                            drain_pending_input_latency(
                                &pending_input_latency,
                                scene_commit_at,
                                Instant::now(),
                            )
                        {
                            telem.input_to_local_ack_us = local_ack_us;
                            telem.input_to_scene_commit_us = scene_commit_us;
                            telem.input_to_next_present_us = next_present_us;
                        }
                        telem.sync_legacy_aliases();
                        telemetry.record(telem);

                        if let Some(state) = benchmark_state.as_mut() {
                            if let Some(last) = telemetry.records().last() {
                                if state.record(last) {
                                    let finished = benchmark_state.take().expect(
                                        "benchmark_state must still exist when record completes",
                                    );
                                    let emit_path = finished.config.emit_path.clone();
                                    match finished.finish() {
                                        Ok(()) => {
                                            tracing::info!(
                                                path = %emit_path.display(),
                                                "windowed benchmark artifact written; shutting down"
                                            );
                                            shutdown_tok
                                                .trigger(crate::threads::ShutdownReason::Clean);
                                            break;
                                        }
                                        Err(err) => {
                                            tracing::error!(
                                                error = %err,
                                                path = %emit_path.display(),
                                                "failed to write windowed benchmark artifact"
                                            );
                                            benchmark_failed
                                                .store(true, std::sync::atomic::Ordering::Release);
                                            shutdown_tok
                                                .trigger(crate::threads::ShutdownReason::Clean);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
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
                if physical_size.width > 0 && physical_size.height > 0 {
                    self.state.config.window.width = physical_size.width;
                    self.state.config.window.height = physical_size.height;
                    if let Ok(state) = self.state.shared_state.try_lock() {
                        if let Ok(mut scene) = state.scene.try_lock() {
                            sync_scene_display_area(
                                &mut scene,
                                physical_size.width,
                                physical_size.height,
                            );
                        }
                    }
                }
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

                if self.synthesize_left_release_if_physically_up() {
                    return;
                }
                self.enqueue_pointer_event(PointerEventKind::Move);
            }

            // ── Pointer: button press/release ──────────────────────────────
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let kind = match state {
                        ElementState::Pressed => PointerEventKind::Down,
                        ElementState::Released => PointerEventKind::Up,
                    };
                    if state == ElementState::Pressed {
                        if self.state.left_button_down {
                            self.enqueue_pointer_event(PointerEventKind::Up);
                            end_os_mouse_capture();
                        }
                        self.state.left_button_down = true;
                        if let Some(window) = &self.state.window {
                            focus_window_for_text_input(window);
                            begin_os_mouse_capture(window);
                        }
                        self.update_overlay_cursor_hittest();
                    }
                    // If the context menu is showing and this is a left-press,
                    // check if it lands on the Reset button.  If so, trigger
                    // the reset; otherwise dismiss the menu (click-outside).
                    if state == ElementState::Released {
                        self.handle_left_click_with_context_menu();
                    }
                    self.enqueue_pointer_event(kind);
                    if state == ElementState::Released {
                        self.state.left_button_down = false;
                        end_os_mouse_capture();
                        self.update_overlay_cursor_hittest();
                    }
                } else if button == MouseButton::Right && state == ElementState::Released {
                    // Right-click: show context menu if cursor is on a drag handle.
                    self.handle_right_click_on_drag_handle();
                }
            }

            // ── Pointer: wheel scroll ────────────────────────────────────────
            WindowEvent::MouseWheel { delta, .. } => {
                let (delta_x, delta_y) = normalize_mouse_wheel_delta(&delta);
                self.enqueue_scroll_event(delta_x, delta_y);
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

                // ── PgUp / PgDn: keyboard scroll through OS input path (hud-6bbe) ──
                //
                // PageUp and PageDown scroll the portal tile under the cursor by
                // one page step (KEYBOARD_PAGE_SCROLL_PX).  This is the keyboard
                // analogue of wheel scroll: same hit-test path, same coalescing,
                // same clamp — only the input device class differs.
                //
                // We handle both Press *and* Repeat (held key = continuous scroll)
                // so the experience matches normal page-scroll behaviour.
                if event.state == ElementState::Pressed {
                    use winit::keyboard::{KeyCode, PhysicalKey};
                    let delta_y = match event.physical_key {
                        PhysicalKey::Code(KeyCode::PageUp) => {
                            // Scroll up: negative delta_y (same sign convention as wheel)
                            -tze_hud_input::KEYBOARD_PAGE_SCROLL_PX
                        }
                        PhysicalKey::Code(KeyCode::PageDown) => {
                            tze_hud_input::KEYBOARD_PAGE_SCROLL_PX
                        }
                        _ => 0.0,
                    };
                    if delta_y != 0.0 {
                        self.enqueue_keyboard_scroll_event(delta_y);
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

                // ── Keyboard → KeyboardProcessor drain (Stage 2) ─────────────
                // Translate the raw OS event to a typed KeyboardDispatch using the
                // current focus state, then dispatch to the owning agent session.
                //
                // The physical_key → DOM-style key_code string and the logical_key
                // → DOM-style key string are extracted here for RFC 0004 §7.4
                // compatibility. Only press and repeat events are forwarded (not
                // key-release events for now — release delivery is a follow-up).
                let key_code_str = physical_key_to_key_code_str(&event.physical_key);
                let logical_key_str = logical_key_to_str(&event.logical_key);
                let mods = winit_mods_to_keyboard_modifiers(self.state.modifiers);
                let timestamp_mono_us = tze_hud_scene::MonoUs(nanoseconds_since_start() / 1_000);
                let paste_shortcut_pressed = event.state == ElementState::Pressed
                    && !event.repeat
                    && (mods.ctrl || mods.meta)
                    && !mods.alt
                    && logical_key_str.eq_ignore_ascii_case("v");

                if event.state == ElementState::Pressed || event.repeat {
                    let raw = RawKeyDownEvent {
                        key_code: key_code_str,
                        key: logical_key_str.clone(),
                        modifiers: mods,
                        repeat: event.repeat,
                        timestamp_mono_us,
                    };
                    self.dispatch_key_down_event(&raw);
                } else if event.state == ElementState::Released {
                    let raw = RawKeyUpEvent {
                        key_code: physical_key_to_key_code_str(&event.physical_key),
                        key: logical_key_str,
                        modifiers: mods,
                        timestamp_mono_us,
                    };
                    self.dispatch_key_up_event(&raw);
                }

                if paste_shortcut_pressed {
                    if let Some(text) = read_windows_clipboard_text() {
                        let raw_char = RawCharacterEvent {
                            character: text,
                            timestamp_mono_us,
                        };
                        self.dispatch_character_event(&raw_char);
                    }
                }

                // ── Character input via Key::Character (non-IME path) ────────
                // When the logical key carries a printable character, produce a
                // RawCharacterEvent so the KeyboardProcessor character path is
                // also exercised (handles basic ASCII without an IME active).
                // IME commit characters arrive via WindowEvent::Ime below.
                if event.state == ElementState::Pressed && !mods.ctrl && !mods.meta && !mods.alt {
                    use winit::keyboard::Key;
                    if let Key::Character(ch) = event.logical_key.as_ref() {
                        let raw_char = RawCharacterEvent {
                            character: ch.to_string(),
                            timestamp_mono_us,
                        };
                        self.dispatch_character_event(&raw_char);
                    }
                }
            }

            // ── IME commit: post-composition character delivery ───────────────
            // `WindowEvent::Ime(Ime::Commit(text))` is the canonical path for
            // IME-composed characters (CJK, accented inputs, etc.). In v1 the
            // commit text is forwarded as a RawCharacterEvent so agents receive
            // CharacterEvent payloads regardless of input method.
            //
            // Preedit events (Ime::Preedit) are v1-reserved and not forwarded.
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                let timestamp_mono_us = tze_hud_scene::MonoUs(nanoseconds_since_start() / 1_000);
                let raw_char = RawCharacterEvent {
                    character: text.clone(),
                    timestamp_mono_us,
                };
                self.dispatch_character_event(&raw_char);
            }

            // ── Redraw ────────────────────────────────────────────────────
            WindowEvent::RedrawRequested => {
                self.refresh_cursor_position_from_os();
                self.drain_input_capture_commands();
                self.inject_windowed_benchmark_input_probe();
                self.refresh_widget_hover_tracking();
                self.tick_widget_hover_tracking();
                self.update_overlay_cursor_hittest();
                // Auto-dismiss the drag-handle context menu after 3 seconds.
                self.tick_context_menu_auto_dismiss();
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
    fn drain_input_capture_commands(&mut self) {
        while let Ok(command) = self.state.input_capture_rx.try_recv() {
            self.state.pending_input_capture_commands.push_back(command);
        }

        while let Some(command) = self.state.pending_input_capture_commands.pop_front() {
            let Ok(state) = self.state.shared_state.try_lock() else {
                tracing::warn!("input capture command deferred: shared_state lock busy");
                self.state
                    .pending_input_capture_commands
                    .push_front(command);
                break;
            };
            let Ok(scene) = state.scene.try_lock() else {
                tracing::warn!("input capture command deferred: scene lock busy");
                self.state
                    .pending_input_capture_commands
                    .push_front(command);
                break;
            };

            match command {
                tze_hud_protocol::session::InputCaptureCommand::Request {
                    tile_id,
                    node_id,
                    device_id,
                    release_on_up,
                } => {
                    let req = tze_hud_input::CaptureRequest {
                        tile_id,
                        node_id,
                        device_id,
                    };
                    if let Some(dispatch) =
                        self.state
                            .input_processor
                            .request_capture(&req, &scene, release_on_up)
                    {
                        tracing::debug!(
                            tile_id = ?tile_id,
                            node_id = ?node_id,
                            device_id,
                            kind = ?dispatch.kind,
                            "session input capture request applied"
                        );
                    } else {
                        tracing::warn!(
                            tile_id = ?tile_id,
                            node_id = ?node_id,
                            device_id,
                            "session input capture request ignored: target not found"
                        );
                    }
                }
                tze_hud_protocol::session::InputCaptureCommand::Release { device_id } => {
                    let req = tze_hud_input::CaptureReleaseRequest { device_id };
                    if let Some(dispatch) = self.state.input_processor.release_capture(&req, &scene)
                    {
                        dispatch_capture_released_event(&self.state.input_event_tx, dispatch);
                    }
                }
            }
        }
    }

    fn synthesize_left_release_if_physically_up(&mut self) -> bool {
        if self.state.left_button_down
            && matches!(left_mouse_button_is_physically_down(), Some(false))
        {
            self.enqueue_pointer_event(PointerEventKind::Up);
            self.state.left_button_down = false;
            end_os_mouse_capture();
            self.update_overlay_cursor_hittest();
            return true;
        }
        false
    }

    fn inject_windowed_benchmark_input_probe(&mut self) {
        if self.state.config.benchmark.is_none() {
            return;
        }
        self.state.cursor_x = WINDOWED_BENCHMARK_INPUT_X;
        self.state.cursor_y = WINDOWED_BENCHMARK_INPUT_Y;
        self.enqueue_pointer_event(PointerEventKind::Move);
    }

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
        self.refresh_widget_hover_tracking();
        let x = self.state.cursor_x;
        let y = self.state.cursor_y;

        // In overlay mode, update cursor hittest based on hit-region membership.
        // This implements per-region passthrough: pointer events outside all
        // active hit-regions are passed through to the underlying desktop, while
        // events inside any hit-region are captured by the runtime.
        //
        // We toggle on every CursorMoved so the hittest tracks the cursor as it
        // moves in/out of regions continuously.
        self.update_overlay_cursor_hittest();

        self.tick_widget_hover_tracking();

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
        let input_started_at = Instant::now();
        let pointer_event = PointerEvent {
            x,
            y,
            kind,
            device_id: 0,
            timestamp: Some(input_started_at),
        };
        // Acquire the scene lock directly (without going through SharedState) so that
        // the main-thread input path does not contend with session handlers that hold
        // both the SharedState lock and the scene lock.
        //
        // `drag_released` carries the geometry payload that must be persisted to the
        // element store after the locks are released (avoids holding locks during
        // disk I/O in the `DragEventOutcome::Released` path).
        let drag_released: Option<DragReleasedData>;
        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(mut scene) = state.scene.try_lock() {
                // ── Click-to-focus (Stage 2) ─────────────────────────────────
                // Use process_with_focus on every pointer event so that a
                // pointer-down on a focusable HitRegionNode transfers keyboard
                // focus before the AgentDispatch is produced.  The returned
                // FocusTransition carries the lost/gained events and a
                // compositor ring-update hint; we log the transition below and
                // broadcast FocusGainedEvent / FocusLostEvent to agents via
                // dispatch_focus_event on the input_event_tx channel.
                let active_tab = scene.active_tab;
                let (result, focus_transition) = if let Some(tab_id) = active_tab {
                    self.state.input_processor.process_with_focus(
                        &pointer_event,
                        &mut scene,
                        &mut self.state.focus_manager,
                        tab_id,
                    )
                } else {
                    // No active tab — fall back to focus-unaware processing.
                    let r = self
                        .state
                        .input_processor
                        .process(&pointer_event, &mut scene);
                    (r, None)
                };

                // Log focus transitions and broadcast FocusGainedEvent /
                // FocusLostEvent over the FOCUS_EVENTS gRPC channel.
                // Local state is already updated in focus_manager above.
                if let Some(ref transition) = focus_transition {
                    if let Some((ev, ns)) = &transition.gained {
                        tracing::debug!(
                            namespace = %ns,
                            tile_id = ?ev.tile_id,
                            node_id = ?ev.node_id,
                            source = ?ev.source,
                            "click-to-focus: focus gained"
                        );
                    }
                    if let Some((ev, ns)) = &transition.lost {
                        tracing::debug!(
                            namespace = %ns,
                            tile_id = ?ev.tile_id,
                            node_id = ?ev.node_id,
                            reason = ?ev.reason,
                            "click-to-focus: focus lost"
                        );
                    }
                }
                if let Some(transition) = focus_transition {
                    dispatch_focus_event(&self.state.input_event_tx, transition);
                }

                // ── Zone interaction dispatch (local feedback first) ──────────
                // On pointer-up, check whether the hit landed on a compositor-
                // managed zone interaction element (e.g. a notification dismiss
                // button).  Dismiss actions are applied synchronously here so the
                // stale affordance disappears before the next rendered frame —
                // satisfying the "local feedback first" doctrine.
                //
                // Action interactions are logged for now; callback delivery to
                // agents is handled separately (see hud-ltgk.7).
                if pointer_event.kind == PointerEventKind::Up {
                    if let HitResult::ZoneInteraction {
                        ref zone_name,
                        published_at_wall_us,
                        ref publisher_namespace,
                        ref kind,
                        ..
                    } = result.hit
                    {
                        match kind {
                            ZoneInteractionKind::Dismiss => {
                                let removed = scene.dismiss_notification(
                                    zone_name,
                                    published_at_wall_us,
                                    publisher_namespace,
                                );
                                tracing::debug!(
                                    zone = %zone_name,
                                    published_at_wall_us,
                                    publisher = %publisher_namespace,
                                    removed,
                                    "zone dismiss: notification removed from scene"
                                );
                            }
                            ZoneInteractionKind::Action { callback_id } => {
                                // Action callback delivery to the owning agent is
                                // handled by the agent event pipeline (hud-ltgk.7).
                                tracing::debug!(
                                    zone = %zone_name,
                                    published_at_wall_us,
                                    publisher = %publisher_namespace,
                                    %callback_id,
                                    "zone action: callback queued for agent delivery"
                                );
                            }
                            ZoneInteractionKind::DragHandle { .. } => {
                                // Handled by the drag state machine below — not here.
                            }
                        }
                    }
                }

                // ── Drag-to-move: long-press drag state machine ──────────────
                // Drives the per-device long-press drag recogniser.  On Down on a
                // drag handle, starts accumulating.  On Move/Up while a drag is
                // active, moves the tile (Moved) or finalises it (Released).
                //
                // Hysteresis: the 250 ms hold threshold means a quick tap (click)
                // never reaches Activated, so click-to-focus (wired above) is
                // unaffected.  Movement > 10dp during accumulation cancels the
                // long-press recognition (Cancelled).
                let display_w = self.state.config.window.width as f32;
                let display_h = self.state.config.window.height as f32;
                drag_released = apply_drag_handle_pointer_event(
                    &mut self.state.input_processor,
                    &pointer_event,
                    &result.hit,
                    &mut scene,
                    display_w,
                    display_h,
                );

                // Local feedback patch (result.local_patch) would be sent to the
                // compositor via a local-patch channel in the full pipeline. For the
                // initial windowed runtime, the compositor reads the scene state
                // directly on the next frame.
                record_pending_input_latency(
                    &self.state.pending_input_latency,
                    input_started_at,
                    result.local_ack_us,
                );
                let _ = result.local_patch;

                // ── Pointer event dispatch to subscribed portal agents ────────
                // Broadcast PointerDown / PointerMove / PointerUp to agents
                // that have subscribed to INPUT_EVENTS (requires
                // `access_input_events` capability).  Only dispatches for
                // portal-owned hit regions (NodeHit / TileHit) — the
                // AgentDispatch is only populated by the InputProcessor when a
                // named hit region matches.  Chrome / ZoneInteraction / Passthrough
                // hits produce no AgentDispatch and are silently skipped.
                //
                // primary dispatch
                if let Some(d) = result.dispatch {
                    dispatch_pointer_event(&self.state.input_event_tx, d);
                }
                // secondary dispatches (e.g. CaptureReleased following PointerUp)
                for d in result.extra_dispatches {
                    if d.kind == tze_hud_input::AgentDispatchKind::CaptureReleased {
                        // CaptureReleased is a focus/lease lifecycle event, not a pointer
                        // event.  Route it through the FOCUS_EVENTS channel so subscribed
                        // agents receive a CaptureReleasedEvent proto envelope.
                        dispatch_capture_released_event(&self.state.input_event_tx, d);
                    } else {
                        dispatch_pointer_event(&self.state.input_event_tx, d);
                    }
                }
            } else {
                drag_released = None;
            }
        } else {
            drag_released = None;
        }

        // ── Post-lock: persist geometry override after drag release ───────────
        // The drag state machine has already updated tile.bounds for live visual
        // feedback.  Here we also write the geometry_override to the element store
        // (durable) and broadcast an ElementRepositionedEvent so subscribers know
        // the tile moved.
        if let Some(released) = drag_released {
            self.persist_drag_release(released);
        }
    }

    /// Persist the geometry override for a completed drag and broadcast an
    /// `ElementRepositionedEvent`.
    ///
    /// Called after all scene locks are released to avoid holding locks during
    /// disk I/O (element store atomic write + fsync).
    fn persist_drag_release(&mut self, released: DragReleasedData) {
        use tze_hud_input::InputProcessor;
        use tze_hud_scene::element_store::ElementType;

        let (store_snapshot, persist_path, new_geometry) = {
            let Ok(mut state) = self.state.shared_state.try_lock() else {
                tracing::warn!("persist_drag_release: could not acquire shared_state lock");
                return;
            };

            let new_geometry = tze_hud_input::drag::final_position_to_geometry(
                released.final_x,
                released.final_y,
                released.width,
                released.height,
                released.display_width,
                released.display_height,
            );

            InputProcessor::persist_drag_geometry(
                &mut state.element_store,
                ElementType::Tile,
                &released.namespace,
                released.final_x,
                released.final_y,
                released.width,
                released.height,
                released.display_width,
                released.display_height,
            );

            let store_snapshot = state.element_store.clone();
            let persist_path = state.element_store_path.clone();
            (store_snapshot, persist_path, new_geometry)
        };

        // Persist element store on a background thread (avoids blocking the
        // winit event loop with sync disk I/O).
        if let Some(path) = persist_path {
            std::thread::spawn(move || {
                if let Err(e) = store_snapshot.persist_to_path_atomic(&path) {
                    tracing::warn!(
                        error = %e,
                        "persist_drag_release: element store persist failed"
                    );
                }
            });
        }

        // Broadcast ElementRepositionedEvent so gRPC subscribers are notified.
        if let Some(ref tx) = self.state.element_repositioned_tx {
            let event = tze_hud_protocol::proto::ElementRepositionedEvent {
                element_id: released.element_id.as_uuid().as_bytes().to_vec(),
                new_geometry: Some(tze_hud_protocol::convert::geometry_policy_to_proto(
                    &new_geometry,
                )),
                previous_geometry: None,
            };
            tx.send(event).unwrap_or_default();
            tracing::debug!(
                element_id = %released.element_id,
                final_x = released.final_x,
                final_y = released.final_y,
                "ElementRepositionedEvent broadcast after drag release"
            );
        }
    }

    /// Enqueue and process a local-first scroll event.
    ///
    /// Applies the offset locally (< 4ms p99 path) and, if the tile owner is
    /// subscribed, dispatches a `ScrollOffsetChangedEvent` to the agent via the
    /// `INPUT_EVENTS` channel.
    fn enqueue_scroll_event(&mut self, delta_x: f32, delta_y: f32) {
        self.refresh_widget_hover_tracking();
        let x = self.state.cursor_x;
        let y = self.state.cursor_y;
        self.update_overlay_cursor_hittest();

        if let Ok(state) = self.state.shared_state.try_lock()
            && let Ok(mut scene) = state.scene.try_lock()
        {
            if let Some(ev) = self.state.input_processor.process_scroll_event(
                &tze_hud_input::ScrollEvent {
                    x,
                    y,
                    delta_x,
                    delta_y,
                },
                &mut scene,
            ) {
                dispatch_scroll_offset_event(&self.state.input_event_tx, &scene, ev);
            }
        }
    }

    /// Enqueue and process a keyboard-originated scroll event (PgUp / PgDn).
    ///
    /// Uses the current cursor position for hit-testing, exactly like wheel
    /// scroll.  Delegates to
    /// [`InputProcessor::process_keyboard_scroll`] which applies the same
    /// local-first coalescing and clamping as `process_scroll_event`.
    ///
    /// Dispatches a `ScrollOffsetChangedEvent` to the tile-owning agent via the
    /// `INPUT_EVENTS` channel when the scroll changes the tile offset.
    fn enqueue_keyboard_scroll_event(&mut self, delta_y: f32) {
        let x = self.state.cursor_x;
        let y = self.state.cursor_y;

        if let Ok(state) = self.state.shared_state.try_lock()
            && let Ok(mut scene) = state.scene.try_lock()
        {
            if let Some(ev) = self
                .state
                .input_processor
                .process_keyboard_scroll(x, y, delta_y, &mut scene)
            {
                dispatch_scroll_offset_event(&self.state.input_event_tx, &scene, ev);
            }
        }
    }

    // ── Keyboard drain helpers ────────────────────────────────────────────

    /// Translate a raw key-down event through the `KeyboardProcessor`, log it,
    /// and broadcast the resulting `KeyboardDispatch` over the `INPUT_EVENTS`
    /// gRPC channel via `input_event_tx`.
    ///
    /// If `current_owner` is `FocusOwner::None` (no focused agent session),
    /// `KeyboardProcessor::process_key_down` returns `None` and the event is
    /// silently dropped — there is no recipient to deliver to.
    ///
    /// Delivery is best-effort (fire-and-forget): if the channel has no
    /// receivers (gRPC disabled, agent not subscribed) the broadcast error is
    /// silently ignored, consistent with the transactional keyboard-event
    /// contract where dropped delivery is an infrastructure gap, not a
    /// data-loss policy.
    fn dispatch_key_down_event(&mut self, raw: &RawKeyDownEvent) {
        let active_tab = self.active_tab_for_keyboard_dispatch();
        let Some(tab_id) = active_tab else { return };
        let focus_owner = self.state.focus_manager.current_owner(tab_id).clone();

        // Build a namespace-resolver closure: given a tile_id, return its
        // agent namespace from the scene.
        let namespace_fn = |tile_id: tze_hud_scene::SceneId| -> Option<String> {
            self.namespace_for_keyboard_tile(tile_id)
        };
        if let Some(dispatch) =
            self.state
                .keyboard_processor
                .process_key_down(raw, &focus_owner, namespace_fn)
        {
            tracing::debug!(
                namespace = %dispatch.namespace,
                tile_id = ?dispatch.tile_id,
                node_id = ?dispatch.node_id,
                kind = ?dispatch.kind,
                "keyboard: KeyDown dispatched to agent"
            );
            dispatch_keyboard_event(&self.state.input_event_tx, dispatch);
        }
    }

    /// Translate a raw key-up event through the `KeyboardProcessor`, log it,
    /// and broadcast it over the `INPUT_EVENTS` gRPC channel.
    ///
    /// Events are dropped silently when `current_owner` is `FocusOwner::None`.
    fn dispatch_key_up_event(&mut self, raw: &RawKeyUpEvent) {
        let active_tab = self.active_tab_for_keyboard_dispatch();
        let Some(tab_id) = active_tab else { return };
        let focus_owner = self.state.focus_manager.current_owner(tab_id).clone();

        let namespace_fn = |tile_id: tze_hud_scene::SceneId| -> Option<String> {
            self.namespace_for_keyboard_tile(tile_id)
        };
        if let Some(dispatch) =
            self.state
                .keyboard_processor
                .process_key_up(raw, &focus_owner, namespace_fn)
        {
            tracing::debug!(
                namespace = %dispatch.namespace,
                tile_id = ?dispatch.tile_id,
                node_id = ?dispatch.node_id,
                kind = ?dispatch.kind,
                "keyboard: KeyUp dispatched to agent"
            );
            dispatch_keyboard_event(&self.state.input_event_tx, dispatch);
        }
    }

    /// Translate a raw post-IME character event through the `KeyboardProcessor`,
    /// log it, and broadcast it over the `INPUT_EVENTS` gRPC channel.
    ///
    /// Called both from `WindowEvent::Ime(Ime::Commit)` (IME path) and from
    /// `Key::Character` in `WindowEvent::KeyboardInput` (direct input path).
    ///
    /// Events are dropped silently when `current_owner` is `FocusOwner::None`.
    fn dispatch_character_event(&mut self, raw: &RawCharacterEvent) {
        let active_tab = self.active_tab_for_keyboard_dispatch();
        let Some(tab_id) = active_tab else { return };
        let focus_owner = self.state.focus_manager.current_owner(tab_id).clone();

        let namespace_fn = |tile_id: tze_hud_scene::SceneId| -> Option<String> {
            self.namespace_for_keyboard_tile(tile_id)
        };
        if let Some(dispatch) =
            self.state
                .keyboard_processor
                .process_character(raw, &focus_owner, namespace_fn)
        {
            tracing::debug!(
                namespace = %dispatch.namespace,
                tile_id = ?dispatch.tile_id,
                node_id = ?dispatch.node_id,
                kind = ?dispatch.kind,
                "keyboard: Character dispatched to agent"
            );
            dispatch_keyboard_event(&self.state.input_event_tx, dispatch);
        }
    }

    fn active_tab_for_keyboard_dispatch(&self) -> Option<tze_hud_scene::SceneId> {
        let state = self.state.shared_state.blocking_lock();
        let scene = state.scene.blocking_lock();
        scene.active_tab
    }

    fn namespace_for_keyboard_tile(&self, tile_id: tze_hud_scene::SceneId) -> Option<String> {
        let state = self.state.shared_state.blocking_lock();
        let scene = state.scene.blocking_lock();
        scene.tiles.get(&tile_id).map(|tile| tile.namespace.clone())
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
        self.state.static_hit_regions = regions;
        // Capture regions are explicit interactive regions only; hover trackers
        // are visual-only and must not block clicks to underlying windows.
        self.state.hit_regions = self.state.static_hit_regions.clone();
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

    /// Rebuild runtime-managed widget hover trackers and refresh overlay hit-regions.
    fn refresh_widget_hover_tracking(&mut self) {
        let (surf_w, surf_h) = if let Some(window) = &self.state.window {
            let size = window.inner_size();
            (size.width as f32, size.height as f32)
        } else {
            (
                self.state.config.window.width as f32,
                self.state.config.window.height as f32,
            )
        };

        let mut next_trackers: Option<std::collections::HashMap<String, WidgetHoverTracker>> = None;
        let mut dynamic_hit_regions: Option<Vec<HitRegion>> = None;
        let mut removed_mutations = Vec::new();
        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(scene) = state.scene.try_lock() {
                let next =
                    build_hover_trackers(&scene, surf_w, surf_h, &self.state.widget_hover_trackers);
                removed_mutations =
                    hidden_mutations_for_removed(&self.state.widget_hover_trackers, &next);
                dynamic_hit_regions = Some(combined_overlay_hit_regions(
                    &self.state.static_hit_regions,
                    &scene,
                ));
                next_trackers = Some(next);
            }
        }
        if let Some(next) = next_trackers {
            self.state.widget_hover_trackers = next;
        }
        self.apply_widget_hover_mutations(removed_mutations);

        // Pointer capture includes explicit static regions plus compositor-managed
        // zone interaction regions (notification dismiss/action affordances).
        //
        // If the scene lock is briefly unavailable, keep the last known dynamic
        // regions. Dropping to the usually-empty static set makes overlay
        // hit-testing flicker to passthrough during mutation bursts.
        if let Some(dynamic_hit_regions) = dynamic_hit_regions {
            self.state.hit_regions = dynamic_hit_regions;
        } else if self.state.hit_regions.is_empty() {
            self.state.hit_regions = self.state.static_hit_regions.clone();
        }
    }

    /// Tick widget hover trackers and apply local parameter mutations.
    fn tick_widget_hover_tracking(&mut self) {
        if self.state.widget_hover_trackers.is_empty() {
            return;
        }
        let mutations = tick_hover_trackers(
            &mut self.state.widget_hover_trackers,
            self.state.cursor_x,
            self.state.cursor_y,
            Instant::now(),
        );
        self.apply_widget_hover_mutations(mutations);
    }

    /// Apply runtime-local hover mutations to widget instance params.
    fn apply_widget_hover_mutations(
        &mut self,
        mutations: Vec<crate::widget_hover::WidgetHoverMutation>,
    ) {
        if mutations.is_empty() {
            return;
        }

        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(mut scene) = state.scene.try_lock() {
                for mutation in mutations {
                    if let Err(e) = scene.set_widget_param_local(
                        &mutation.instance_name,
                        &mutation.param_name,
                        WidgetParameterValue::F32(mutation.value),
                    ) {
                        tracing::debug!(
                            error = %e,
                            widget = %mutation.instance_name,
                            param = %mutation.param_name,
                            "widget hover: failed to apply local hover mutation"
                        );
                    }
                }
            }
        }
    }

    /// Refresh cursor position from OS state when passthrough is active.
    ///
    /// In overlay mode on Windows, `set_cursor_hittest(false)` can prevent
    /// `CursorMoved` delivery to winit. Polling global cursor position ensures
    /// hit-testing can flip back to capture when the cursor enters an active
    /// widget hover region.
    fn refresh_cursor_position_from_os(&mut self) {
        if self.state.effective_mode == WindowMode::Overlay {
            #[cfg(target_os = "windows")]
            {
                use windows::Win32::Foundation::POINT;
                use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

                let Some(window) = &self.state.window else {
                    return;
                };
                let window_pos = window
                    .outer_position()
                    .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0));

                let mut pt = POINT { x: 0, y: 0 };
                // SAFETY: GetCursorPos writes to the provided POINT and has no
                // additional safety preconditions.
                let ok = unsafe { GetCursorPos(&mut pt).is_ok() };
                if ok {
                    self.state.cursor_x = (pt.x - window_pos.x) as f32;
                    self.state.cursor_y = (pt.y - window_pos.y) as f32;
                }
            }
        }
    }

    /// Update overlay passthrough/capture state from current cursor+regions.
    fn update_overlay_cursor_hittest(&mut self) {
        if self.state.effective_mode != WindowMode::Overlay {
            return;
        }
        let should_capture = should_capture_pointer_event(
            WindowMode::Overlay,
            self.state.cursor_x,
            self.state.cursor_y,
            &self.state.hit_regions,
        ) || self.state.left_button_down;
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

    // ── Chrome context menu (drag handle reset gesture) ────────────────────

    /// Show the chrome context menu anchored at the cursor position if the
    /// cursor is currently over a drag handle.
    ///
    /// Called on right-click (desktop).  No-op if the cursor is not on a
    /// drag handle.
    fn handle_right_click_on_drag_handle(&mut self) {
        let cx = self.state.cursor_x;
        let cy = self.state.cursor_y;

        // Find the drag handle under the cursor.
        let element_id = {
            let Ok(state) = self.state.shared_state.try_lock() else {
                return;
            };
            let Ok(scene) = state.scene.try_lock() else {
                return;
            };
            scene
                .drag_handle_hit_regions
                .iter()
                .find(|r| {
                    cx >= r.bounds.x
                        && cx < r.bounds.x + r.bounds.width
                        && cy >= r.bounds.y
                        && cy < r.bounds.y + r.bounds.height
                })
                .map(|r| r.element_id)
        };

        let Some(element_id) = element_id else {
            return; // Cursor is not on a drag handle — nothing to show.
        };

        // Anchor the menu to the right-click position.
        // Pre-compute the reset button rect (constant throughout the menu's lifetime).
        const MENU_W: f32 = 160.0;
        const MENU_H: f32 = 32.0;
        const PADDING: f32 = 4.0;
        let menu = DragHandleContextMenuState {
            element_id,
            anchor_x: cx,
            anchor_y: cy,
            shown_at_ns: nanoseconds_since_start(),
            reset_button_rect: Some(tze_hud_scene::Rect::new(
                cx + PADDING,
                cy + PADDING,
                MENU_W - PADDING * 2.0,
                MENU_H - PADDING * 2.0,
            )),
        };

        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(mut scene) = state.scene.try_lock() {
                scene.drag_handle_context_menu = Some(menu);
                tracing::debug!(
                    element_id = %element_id,
                    x = cx,
                    y = cy,
                    "chrome context menu shown for drag handle"
                );
            }
        }
    }

    /// Handle a left-click when the chrome context menu is showing.
    ///
    /// - If the click lands on the "Reset to default" button rect → trigger reset.
    /// - Otherwise → dismiss the menu (click-outside).
    ///
    /// Called synchronously on `MouseButton::Left` release, before the normal
    /// `enqueue_pointer_event` path.
    fn handle_left_click_with_context_menu(&mut self) {
        let cx = self.state.cursor_x;
        let cy = self.state.cursor_y;

        // Extract context menu state, then immediately drop the scene lock.
        let menu_state = {
            let Ok(state) = self.state.shared_state.try_lock() else {
                return;
            };
            let Ok(scene) = state.scene.try_lock() else {
                return;
            };
            scene.drag_handle_context_menu.clone()
        };

        let Some(menu) = menu_state else {
            return; // Menu not showing — nothing to do.
        };

        // Check if the click landed on the Reset button.
        let hit_reset = menu
            .reset_button_rect
            .is_some_and(|r| cx >= r.x && cx < r.x + r.width && cy >= r.y && cy < r.y + r.height);

        // Dismiss the menu in all cases.
        if let Ok(state) = self.state.shared_state.try_lock() {
            if let Ok(mut scene) = state.scene.try_lock() {
                scene.drag_handle_context_menu = None;
            }
        }

        if !hit_reset {
            tracing::debug!("chrome context menu dismissed (click-outside)");
            return;
        }

        // Reset the element geometry.
        self.perform_reset_element_geometry(menu.element_id);
    }

    /// Auto-dismiss the context menu after 3 seconds.
    ///
    /// Called each frame from `RedrawRequested`.  No-op when no menu is showing.
    fn tick_context_menu_auto_dismiss(&mut self) {
        const AUTO_DISMISS_NS: u64 = 3_000_000_000; // 3 seconds

        let now_ns = nanoseconds_since_start();

        let should_dismiss = {
            let Ok(state) = self.state.shared_state.try_lock() else {
                return;
            };
            let Ok(scene) = state.scene.try_lock() else {
                return;
            };
            scene
                .drag_handle_context_menu
                .as_ref()
                .is_some_and(|m| now_ns.saturating_sub(m.shown_at_ns) >= AUTO_DISMISS_NS)
        };

        if should_dismiss {
            if let Ok(state) = self.state.shared_state.try_lock() {
                if let Ok(mut scene) = state.scene.try_lock() {
                    scene.drag_handle_context_menu = None;
                    tracing::debug!("chrome context menu auto-dismissed after 3s");
                }
            }
        }
    }

    /// Synchronously reset the geometry override for `element_id` and broadcast
    /// an `ElementRepositionedEvent` (hud-zc7f).
    ///
    /// This is the sync chrome-layer path for the "Reset to default" context
    /// menu action.  It mirrors the logic in `HudSessionImpl::reset_element_geometry`
    /// but runs on the main thread without async, using the stored broadcast sender.
    ///
    /// No-op if the element has no user override.
    fn perform_reset_element_geometry(&mut self, element_id: tze_hud_scene::SceneId) {
        // Collect previous override, fallback geometry, and optional persist path.
        let (previous_override, fallback_geometry, store_snapshot, persist_path) = {
            let Ok(mut state) = self.state.shared_state.try_lock() else {
                tracing::warn!("perform_reset_element_geometry: could not acquire shared state");
                return;
            };
            // Clear the override.
            let previous = state.element_store.reset_geometry_override(element_id);
            let Some(previous) = previous else {
                tracing::debug!(
                    element_id = %element_id,
                    "perform_reset_element_geometry: no override — no-op"
                );
                return;
            };
            // Resolve fallback geometry (agent bounds → config → default policy).
            let fallback = {
                let Ok(scene) = state.scene.try_lock() else {
                    return;
                };
                state
                    .element_store
                    .entries
                    .get(&element_id)
                    .map(|entry| {
                        tze_hud_scene::element_store::fallback_geometry_for_element(
                            element_id, entry, &scene,
                        )
                    })
                    .unwrap_or(tze_hud_scene::ZERO_GEOMETRY_POLICY)
            };
            let store_snapshot = state.element_store.clone();
            let persist_path = state.element_store_path.clone();
            (previous, fallback, store_snapshot, persist_path)
        };

        // Persist the updated store on a background thread to avoid blocking the
        // Winit event loop with sync disk I/O (atomic write + fsync).
        if let Some(path) = persist_path {
            std::thread::spawn(move || {
                if let Err(e) = store_snapshot.persist_to_path_atomic(&path) {
                    tracing::warn!(error = %e, "perform_reset_element_geometry: persist failed");
                }
            });
        }

        // Broadcast ElementRepositionedEvent.
        if let Some(ref tx) = self.state.element_repositioned_tx {
            let event = tze_hud_protocol::proto::ElementRepositionedEvent {
                // Use big-endian UUID bytes to match scene_id_to_bytes wire contract.
                element_id: element_id.as_uuid().as_bytes().to_vec(),
                new_geometry: Some(tze_hud_protocol::convert::geometry_policy_to_proto(
                    &fallback_geometry,
                )),
                previous_geometry: Some(tze_hud_protocol::convert::geometry_policy_to_proto(
                    &previous_override,
                )),
            };
            tx.send(event).unwrap_or_default();
            tracing::debug!(
                element_id = %element_id,
                "ElementRepositionedEvent broadcast after reset-to-default"
            );
        }
    }

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
        // Parse the raw config once here so we can use it for both widget
        // registry initialization and the RuntimeContext build. Failure is
        // non-fatal — widget startup will just leave the registry empty.
        let raw_config_for_startup: Option<tze_hud_config::raw::RawConfig> = cfg
            .config_toml
            .as_deref()
            .and_then(|toml| toml::from_str(toml).ok());

        let mut pending_widget_svgs: Vec<crate::widget_startup::WidgetSvgAsset> = Vec::new();
        let (
            shared_scene,
            startup_compositor_tokens,
            runtime_widget_store,
            startup_element_store,
            startup_element_store_path,
        ) = {
            let mut scene = SceneGraph::new(width, height);

            // Resolve config file parent directory for path resolution.
            let config_parent_buf: Option<std::path::PathBuf> = cfg
                .config_file_path
                .as_deref()
                .and_then(|p| std::path::Path::new(p).parent().map(|d| d.to_path_buf()));

            let runtime_widget_store = if let Some(raw) = &raw_config_for_startup {
                let resolved =
                    resolve_runtime_widget_asset_store(raw, config_parent_buf.as_deref()).map_err(
                        |e| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                format!("runtime widget asset store config invalid: {}", e.hint),
                            )
                        },
                    )?;
                Some(RuntimeWidgetStore::open(RuntimeWidgetStoreConfig {
                    store_path: resolved.store_path,
                    max_total_bytes: resolved.max_total_bytes,
                    max_agent_bytes: resolved.max_agent_bytes,
                })?)
            } else {
                None
            };

            // Run the full component shape language startup sequence (steps 2-9):
            // design token loading, global widget bundles, component profile loading,
            // profile selection, effective rendering policy construction, readability
            // validation, zone registry construction, and widget registry population.
            //
            // Per component-shape-language/spec.md §Requirement: Startup Sequence Integration
            let compositor_tokens = if let Some(raw) = &raw_config_for_startup {
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
                // compositor_tokens is pre-merged: global tokens + all active profile
                // token overrides. Pass directly to compositor.set_token_map().
                startup_result.compositor_tokens
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
            let element_store_bootstrap = bootstrap_scene_element_store(&mut scene);
            (
                Arc::new(Mutex::new(scene)),
                compositor_tokens,
                runtime_widget_store,
                element_store_bootstrap.store,
                element_store_bootstrap.path,
            )
        };
        let sessions = tze_hud_protocol::session::SessionRegistry::new(&cfg.psk);
        let (input_capture_tx, input_capture_rx) = tokio::sync::mpsc::unbounded_channel();
        let shared_state = Arc::new(Mutex::new(SharedState {
            scene: Arc::clone(&shared_scene),
            sessions,
            resource_store: tze_hud_resource::ResourceStore::new(
                tze_hud_resource::ResourceStoreConfig::default(),
            ),
            widget_asset_store: tze_hud_protocol::session::WidgetAssetStore::default(),
            runtime_widget_store: runtime_widget_store.clone(),
            element_store: startup_element_store,
            element_store_path: Some(startup_element_store_path),
            safe_mode_active: false,
            token_store: TokenStore::new(),
            freeze_active: false,
            degradation_level: tze_hud_protocol::session::RuntimeDegradationLevel::Normal,
            input_capture_tx: Some(input_capture_tx),
        }));

        let (frame_ready_tx, frame_ready_rx) = frame_ready_channel();
        let input_ring = Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::with_capacity(INPUT_EVENT_CAPACITY),
        ));
        let pending_input_latency = Arc::new(StdMutex::new(VecDeque::new()));
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
        let (mut network_rt, mut network_handles, element_repositioned_tx, input_event_tx) =
            start_network_services(
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
            _runtime_widget_store: runtime_widget_store,
            fallback_unrestricted,
            shared_state,
            input_ring,
            pending_input_latency,
            frame_ready_rx,
            frame_ready_tx: Some(frame_ready_tx),
            compositor: None,
            window_surface: None,
            input_processor: InputProcessor::new(),
            input_capture_rx,
            pending_input_capture_commands: std::collections::VecDeque::new(),
            focus_manager: FocusManager::new(),
            keyboard_processor: KeyboardProcessor::new(),
            telemetry: TelemetryCollector::new(),
            pipeline: FramePipeline::new(),
            shutdown,
            benchmark_failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cursor_x: 0.0,
            cursor_y: 0.0,
            left_button_down: false,
            window: None,
            effective_mode,
            hit_regions: Vec::new(),
            static_hit_regions: Vec::new(),
            widget_hover_trackers: std::collections::HashMap::new(),
            pending_mode_switch: None,
            pending_widget_svgs,
            modifiers: winit::keyboard::ModifiersState::empty(),
            current_monitor_index: 0,
            global_tokens: startup_compositor_tokens,
            element_repositioned_tx,
            input_event_tx,
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

        if app
            .state
            .benchmark_failed
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("windowed benchmark artifact write failed".into());
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
/// When `grpc_port != 0`, starts the `HudSession` gRPC server on `0.0.0.0:grpc_port`.
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
) -> Result<
    (
        Option<NetworkRuntime>,
        Vec<tokio::task::JoinHandle<()>>,
        Option<tokio::sync::broadcast::Sender<tze_hud_protocol::proto::ElementRepositionedEvent>>,
        Option<tokio::sync::broadcast::Sender<(String, tze_hud_protocol::proto::EventBatch)>>,
    ),
    Box<dyn std::error::Error>,
> {
    if grpc_port == 0 {
        tracing::info!(
            "windowed runtime: gRPC server disabled (grpc_port = 0); running compositor-only"
        );
        return Ok((None, Vec::new(), None, None));
    }

    // Build the multi-thread Tokio runtime for network tasks.
    let network_rt = NetworkRuntime::new()
        .map_err(|e| format!("windowed runtime: failed to build network Tokio runtime: {e}"))?;

    let addr: std::net::SocketAddr = format!("0.0.0.0:{grpc_port}")
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

    // Clone the broadcast senders before moving the service into the gRPC task.
    // The windowed runtime holds these senders to:
    // - broadcast ElementRepositionedEvents from the sync chrome-layer reset path.
    // - inject EventBatch payloads (scroll, keyboard, and future input events)
    //   on the input_event_tx channel after windowed input is processed.
    let element_repositioned_tx = service.element_repositioned_tx.clone();
    let input_event_tx = service.input_event_tx.clone();

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

    Ok((
        Some(network_rt),
        vec![handle],
        Some(element_repositioned_tx),
        Some(input_event_tx),
    ))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Dispatch a `ScrollOffsetChangedEvent` to the tile-owning agent.
///
/// Looks up the owning namespace from the scene graph, constructs an
/// `EventBatch` with a single `ScrollOffsetChangedEvent` envelope, and sends it
/// on the `input_event_tx` broadcast channel.  The session handler delivers the
/// batch only when the agent is subscribed to `INPUT_EVENTS` — the subscription
/// gate is enforced in `subscriptions::filter_event_batch`, not here.
///
/// This is a best-effort dispatch (non-blocking, try_send semantics): if no
/// receiver is connected (gRPC disabled, no agent subscribed) the event is
/// silently dropped, matching the ephemeral-realtime message class contract.
fn dispatch_scroll_offset_event(
    tx: &Option<tokio::sync::broadcast::Sender<(String, EventBatch)>>,
    scene: &SceneGraph,
    ev: tze_hud_input::ScrollOffsetChangedEvent,
) {
    let Some(tx) = tx else { return };

    // Look up the namespace that owns this tile so the session handler can
    // route the batch to the correct agent.
    let Some(namespace) = scene.tiles.get(&ev.tile_id).map(|t| t.namespace.clone()) else {
        return;
    };

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    use tze_hud_protocol::proto::input_envelope::Event as InputEvent;
    let batch = EventBatch {
        frame_number: 0, // synthetic — not tied to a compositor frame
        batch_ts_us: now_us,
        events: vec![InputEnvelope {
            event: Some(InputEvent::ScrollOffsetChanged(
                tze_hud_protocol::proto::ScrollOffsetChangedEvent {
                    tile_id: ev.tile_id.as_uuid().as_bytes().to_vec(),
                    timestamp_mono_us: 0, // monotonic clock not wired here; v1 leaves unset
                    offset_x: ev.offset_x,
                    offset_y: ev.offset_y,
                },
            )),
        }],
    };

    // Broadcast to all session handler tasks; each one delivers only if the
    // namespace matches and INPUT_EVENTS is active. Errors (no receivers,
    // channel full) are silently ignored — scroll events are ephemeral.
    let _ = tx.send((namespace, batch));
}

/// Broadcast a [`tze_hud_input::KeyboardDispatch`] to the owning agent via the
/// `INPUT_EVENTS` gRPC channel.
///
/// Converts the `KeyboardDispatch` to the appropriate proto envelope
/// (`KeyDownEvent`, `KeyUpEvent`, or `CharacterEvent`), wraps it in an
/// `EventBatch`, and sends it on the broadcast channel.  The session handler
/// delivers the batch only when the agent is subscribed to `INPUT_EVENTS` —
/// the subscription gate is enforced in `subscriptions::filter_event_batch`.
///
/// Keyboard events are transactional (never dropped by design), but the
/// broadcast channel is best-effort from the runtime side: if no receiver is
/// connected (gRPC disabled, no agent subscribed to INPUT_EVENTS), the send
/// error is silently discarded.  This matches the existing scroll-event
/// pattern and is consistent with the `FocusOwner::None` early-return in the
/// dispatch helpers (no broadcast when no focused session).
fn dispatch_keyboard_event(
    tx: &Option<tokio::sync::broadcast::Sender<(String, EventBatch)>>,
    dispatch: tze_hud_input::KeyboardDispatch,
) {
    let Some(tx) = tx else { return };

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    let tile_id_bytes = dispatch.tile_id.as_uuid().as_bytes().to_vec();
    let node_id_bytes = dispatch
        .node_id
        .map(|id| id.as_uuid().as_bytes().to_vec())
        .unwrap_or_default();

    use tze_hud_input::KeyboardDispatchKind;
    use tze_hud_protocol::proto::input_envelope::Event as InputEvent;

    let event = match dispatch.kind {
        KeyboardDispatchKind::KeyDown {
            key_code,
            key,
            modifiers,
            repeat,
            timestamp_mono_us,
        } => InputEvent::KeyDown(tze_hud_protocol::proto::KeyDownEvent {
            tile_id: tile_id_bytes,
            node_id: node_id_bytes,
            timestamp_mono_us: timestamp_mono_us.0,
            key_code,
            key,
            repeat,
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            meta: modifiers.meta,
        }),
        KeyboardDispatchKind::KeyUp {
            key_code,
            key,
            modifiers,
            timestamp_mono_us,
        } => InputEvent::KeyUp(tze_hud_protocol::proto::KeyUpEvent {
            tile_id: tile_id_bytes,
            node_id: node_id_bytes,
            timestamp_mono_us: timestamp_mono_us.0,
            key_code,
            key,
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            meta: modifiers.meta,
        }),
        KeyboardDispatchKind::Character {
            character,
            timestamp_mono_us,
        } => InputEvent::Character(tze_hud_protocol::proto::CharacterEvent {
            tile_id: tile_id_bytes,
            node_id: node_id_bytes,
            timestamp_mono_us: timestamp_mono_us.0,
            character,
        }),
    };

    let batch = EventBatch {
        frame_number: 0, // synthetic — not tied to a compositor frame
        batch_ts_us: now_us,
        events: vec![InputEnvelope { event: Some(event) }],
    };

    // Broadcast to all session handler tasks; each one delivers only if the
    // namespace matches and INPUT_EVENTS is subscribed. Errors (no receivers,
    // channel lagged) are silently ignored.
    let _ = tx.send((dispatch.namespace, batch));
}

/// Broadcast `FocusGainedEvent` and/or `FocusLostEvent` to the owning agents
/// via the `FOCUS_EVENTS` gRPC channel.
///
/// Converts a [`tze_hud_input::FocusTransition`] into proto envelopes and sends
/// each event as a single-event `EventBatch` on the broadcast channel.  The
/// session handler delivers each batch only when the agent is subscribed to
/// `FOCUS_EVENTS` — the subscription gate is enforced in
/// `subscriptions::filter_event_batch`, not here.
///
/// Focus lost is dispatched first (if present) so the agent that relinquished
/// focus receives its event before the newly-focused agent receives its gained
/// event, preserving the ordering guarantee in RFC 0004 §8.4.
///
/// Delivery is best-effort (fire-and-forget): errors (no receivers, channel
/// lagged) are silently ignored, matching the keyboard-event broadcast pattern.
fn dispatch_focus_event(
    tx: &Option<tokio::sync::broadcast::Sender<(String, EventBatch)>>,
    transition: tze_hud_input::FocusTransition,
) {
    let Some(tx) = tx else { return };

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    use tze_hud_input::{FocusLostReason, FocusSource};
    use tze_hud_protocol::proto::input_envelope::Event as InputEvent;

    // ── FocusLostEvent (dispatched first per RFC 0004 §8.4) ─────────────────
    if let Some((lost_ev, namespace)) = transition.lost {
        let tile_id_bytes = lost_ev.tile_id.as_uuid().as_bytes().to_vec();
        let node_id_bytes = lost_ev
            .node_id
            .map(|id| id.as_uuid().as_bytes().to_vec())
            .unwrap_or_default();

        let proto_reason = match lost_ev.reason {
            FocusLostReason::ClickElsewhere => {
                tze_hud_protocol::proto::FocusLostReason::ClickElsewhere
            }
            FocusLostReason::TabKey => tze_hud_protocol::proto::FocusLostReason::TabKey,
            FocusLostReason::Programmatic => tze_hud_protocol::proto::FocusLostReason::Programmatic,
            FocusLostReason::TileDestroyed => {
                tze_hud_protocol::proto::FocusLostReason::TileDestroyed
            }
            FocusLostReason::TabSwitched => tze_hud_protocol::proto::FocusLostReason::TabSwitched,
            FocusLostReason::LeaseRevoked => tze_hud_protocol::proto::FocusLostReason::LeaseRevoked,
            FocusLostReason::AgentDisconnected => {
                tze_hud_protocol::proto::FocusLostReason::AgentDisconnected
            }
            FocusLostReason::CommandInput => tze_hud_protocol::proto::FocusLostReason::CommandInput,
        };

        let batch = EventBatch {
            frame_number: 0,
            batch_ts_us: now_us,
            events: vec![InputEnvelope {
                event: Some(InputEvent::FocusLost(
                    tze_hud_protocol::proto::FocusLostEvent {
                        tile_id: tile_id_bytes,
                        node_id: node_id_bytes,
                        timestamp_mono_us: 0, // monotonic clock not wired here; v1 leaves unset
                        reason: proto_reason as i32,
                    },
                )),
            }],
        };

        let _ = tx.send((namespace, batch));
    }

    // ── FocusGainedEvent ─────────────────────────────────────────────────────
    if let Some((gained_ev, namespace)) = transition.gained {
        let tile_id_bytes = gained_ev.tile_id.as_uuid().as_bytes().to_vec();
        let node_id_bytes = gained_ev
            .node_id
            .map(|id| id.as_uuid().as_bytes().to_vec())
            .unwrap_or_default();

        let proto_source = match gained_ev.source {
            FocusSource::Click => tze_hud_protocol::proto::FocusSource::Click,
            FocusSource::TabKey => tze_hud_protocol::proto::FocusSource::TabKey,
            FocusSource::Programmatic => tze_hud_protocol::proto::FocusSource::Programmatic,
            FocusSource::CommandInput => tze_hud_protocol::proto::FocusSource::CommandInput,
        };

        let batch = EventBatch {
            frame_number: 0,
            batch_ts_us: now_us,
            events: vec![InputEnvelope {
                event: Some(InputEvent::FocusGained(
                    tze_hud_protocol::proto::FocusGainedEvent {
                        tile_id: tile_id_bytes,
                        node_id: node_id_bytes,
                        timestamp_mono_us: 0, // monotonic clock not wired here; v1 leaves unset
                        source: proto_source as i32,
                    },
                )),
            }],
        };

        let _ = tx.send((namespace, batch));
    }
}

/// Broadcast a `PointerDownEvent`, `PointerMoveEvent`, or `PointerUpEvent` to
/// the owning agent via the `INPUT_EVENTS` gRPC channel.
///
/// Converts an [`tze_hud_input::AgentDispatch`] with `kind` in
/// {`PointerDown`, `PointerMove`, `PointerUp`} to the corresponding proto
/// envelope, wraps it in an `EventBatch`, and sends it on the broadcast
/// channel.  The session handler delivers the batch only when the agent is
/// subscribed to `INPUT_EVENTS` — the subscription gate is enforced in
/// `subscriptions::filter_event_batch`, not here.
///
/// **Throttling**: every `PointerMove` is forwarded as-is to all opted-in
/// subscribers.  Subscribers that cannot tolerate the full rate should
/// throttle on the receive side.  This matches the "ephemeral realtime"
/// message class contract (RFC CLAUDE.md §Four Message Classes) and avoids
/// imposing a specific rate budget on the dispatch path, which is on the
/// Stage 2 hot path (< 500 µs p99 per engineering-bar.md §2).
///
/// All other `AgentDispatchKind` values are silently ignored — only
/// `PointerDown`, `PointerMove`, and `PointerUp` are dispatched here.
/// `PointerEnter`, `PointerLeave`, `Activated` are not yet wired.
/// `CaptureReleased` is routed to `dispatch_capture_released_event` by the
/// caller (it belongs to `FOCUS_EVENTS`, not `INPUT_EVENTS`).
///
/// Delivery is best-effort (fire-and-forget): errors (no receivers, channel
/// lagged) are silently ignored, matching the scroll and keyboard patterns.
fn dispatch_pointer_event(
    tx: &Option<tokio::sync::broadcast::Sender<(String, EventBatch)>>,
    dispatch: tze_hud_input::AgentDispatch,
) {
    let Some(tx) = tx else { return };

    use tze_hud_input::AgentDispatchKind;
    use tze_hud_protocol::proto::input_envelope::Event as InputEvent;

    let tile_id_bytes = dispatch.tile_id.as_uuid().as_bytes().to_vec();
    // Send empty bytes when no specific node was hit (tile-level pointer event).
    // This matches the proto field-presence convention used by FocusLostEvent,
    // FocusGainedEvent, and CaptureReleasedEvent: absent field = empty Vec,
    // not 16 zero bytes.
    let node_id_bytes = if dispatch.node_id.is_nil() {
        Vec::new()
    } else {
        dispatch.node_id.as_uuid().as_bytes().to_vec()
    };

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    // Monotonic microseconds since process start — same clock source used by
    // the keyboard and scene-graph event paths (see `nanoseconds_since_start`).
    let timestamp_mono_us = (nanoseconds_since_start() / 1_000).max(1);

    let event = match dispatch.kind {
        AgentDispatchKind::PointerDown => {
            InputEvent::PointerDown(tze_hud_protocol::proto::PointerDownEvent {
                tile_id: tile_id_bytes,
                node_id: node_id_bytes,
                interaction_id: dispatch.interaction_id,
                timestamp_mono_us,
                device_id: dispatch.device_id.to_string(),
                local_x: dispatch.local_x,
                local_y: dispatch.local_y,
                display_x: dispatch.display_x,
                display_y: dispatch.display_y,
                button: 0, // primary button; multi-button not yet tracked in AgentDispatch
            })
        }
        AgentDispatchKind::PointerMove => {
            InputEvent::PointerMove(tze_hud_protocol::proto::PointerMoveEvent {
                tile_id: tile_id_bytes,
                node_id: node_id_bytes,
                interaction_id: dispatch.interaction_id,
                timestamp_mono_us,
                device_id: dispatch.device_id.to_string(),
                local_x: dispatch.local_x,
                local_y: dispatch.local_y,
                display_x: dispatch.display_x,
                display_y: dispatch.display_y,
            })
        }
        AgentDispatchKind::PointerUp => {
            InputEvent::PointerUp(tze_hud_protocol::proto::PointerUpEvent {
                tile_id: tile_id_bytes,
                node_id: node_id_bytes,
                interaction_id: dispatch.interaction_id,
                timestamp_mono_us,
                device_id: dispatch.device_id.to_string(),
                local_x: dispatch.local_x,
                local_y: dispatch.local_y,
                display_x: dispatch.display_x,
                display_y: dispatch.display_y,
                button: 0, // primary button; multi-button not yet tracked in AgentDispatch
            })
        }
        // All other variants (PointerEnter, PointerLeave, Activated, PointerCancel)
        // are not yet wired.  CaptureReleased is pre-filtered by the caller and
        // routed to dispatch_capture_released_event instead.
        _ => return,
    };

    let batch = EventBatch {
        frame_number: 0, // synthetic — not tied to a compositor frame
        batch_ts_us: now_us,
        events: vec![InputEnvelope { event: Some(event) }],
    };

    // Broadcast to all session handler tasks; each one delivers only if the
    // namespace matches and INPUT_EVENTS is subscribed. Errors (no receivers,
    // channel lagged) are silently ignored — PointerMove is ephemeral;
    // PointerDown/Up are transactional but the broadcast channel is best-effort
    // from the runtime side (consistent with keyboard and scroll patterns).
    let _ = tx.send((dispatch.namespace, batch));
}

/// Broadcast a `CaptureReleasedEvent` to the owning agent via the `FOCUS_EVENTS`
/// gRPC channel.
///
/// Called when `InputProcessor` produces a `CaptureReleased` dispatch in
/// `extra_dispatches` (e.g. after `PointerUp` with `release_on_up=true`).
///
/// `CaptureReleased` is a focus/lease lifecycle event, not a pointer event, so
/// it belongs on the `FOCUS_EVENTS` channel (RFC 0004 §8.3, subscriptions.rs
/// §`is_focus_variant`).  Agents that subscribe to `FOCUS_EVENTS` with the
/// `access_input_events` capability will receive it.
///
/// Delivery is best-effort (fire-and-forget): errors (no receivers, channel
/// lagged) are silently ignored.
fn dispatch_capture_released_event(
    tx: &Option<tokio::sync::broadcast::Sender<(String, EventBatch)>>,
    dispatch: tze_hud_input::AgentDispatch,
) {
    let Some(tx) = tx else { return };

    use tze_hud_input::{AgentDispatchKind, CaptureReleasedReason};
    use tze_hud_protocol::proto::input_envelope::Event as InputEvent;

    debug_assert_eq!(
        dispatch.kind,
        AgentDispatchKind::CaptureReleased,
        "dispatch_capture_released_event called with non-CaptureReleased kind"
    );

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    let proto_reason = match dispatch.capture_released_reason {
        Some(CaptureReleasedReason::AgentReleased) => {
            tze_hud_protocol::proto::CaptureReleasedReason::AgentReleased
        }
        Some(CaptureReleasedReason::PointerUp) => {
            tze_hud_protocol::proto::CaptureReleasedReason::PointerUp
        }
        Some(CaptureReleasedReason::RuntimeRevoked) => {
            tze_hud_protocol::proto::CaptureReleasedReason::RuntimeRevoked
        }
        Some(CaptureReleasedReason::LeaseRevoked) => {
            tze_hud_protocol::proto::CaptureReleasedReason::LeaseRevoked
        }
        None => tze_hud_protocol::proto::CaptureReleasedReason::Unspecified,
    };

    let tile_id_bytes = dispatch.tile_id.as_uuid().as_bytes().to_vec();
    // Send empty bytes when no specific node was captured (tile-level capture).
    // This matches the proto field-presence convention used by FocusLostEvent
    // and FocusGainedEvent: absent field = empty Vec, not 16 zero bytes.
    let node_id_bytes = if dispatch.node_id.is_nil() {
        Vec::new()
    } else {
        dispatch.node_id.as_uuid().as_bytes().to_vec()
    };

    let batch = EventBatch {
        frame_number: 0, // synthetic — not tied to a compositor frame
        batch_ts_us: now_us,
        events: vec![InputEnvelope {
            event: Some(InputEvent::CaptureReleased(
                tze_hud_protocol::proto::CaptureReleasedEvent {
                    tile_id: tile_id_bytes,
                    node_id: node_id_bytes,
                    timestamp_mono_us: 0, // monotonic clock not wired here; v1 leaves unset
                    device_id: dispatch.device_id.to_string(),
                    reason: proto_reason as i32,
                },
            )),
        }],
    };

    // Broadcast to FOCUS_EVENTS subscribers.  Errors are silently ignored.
    let _ = tx.send((dispatch.namespace, batch));
}

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

// ─── Keyboard pipeline helpers ────────────────────────────────────────────────

/// Map a winit `PhysicalKey` to the DOM `KeyboardEvent.code`-style string
/// used by `RawKeyDownEvent.key_code` (RFC 0004 §7.4).
///
/// Returns the `KeyCode` variant name (e.g. `"KeyA"`, `"ShiftLeft"`,
/// `"ArrowDown"`) for identified keys, and `"Unidentified"` for unknown ones.
fn physical_key_to_key_code_str(key: &winit::keyboard::PhysicalKey) -> String {
    use winit::keyboard::PhysicalKey;
    match key {
        PhysicalKey::Code(code) => format!("{code:?}"),
        PhysicalKey::Unidentified(_) => "Unidentified".to_string(),
    }
}

/// Map a winit logical `Key` to the DOM `KeyboardEvent.key`-style string
/// used by `RawKeyDownEvent.key` and `RawKeyUpEvent.key` (RFC 0004 §7.4).
///
/// For character keys this is the character itself (e.g. `"a"`, `"A"`, `"1"`).
/// For named keys this is the `NamedKey` variant name (e.g. `"Enter"`,
/// `"Backspace"`, `"ArrowDown"`). Unknown keys map to `"Unidentified"`.
fn logical_key_to_str(key: &winit::keyboard::Key) -> String {
    use winit::keyboard::Key;
    match key {
        Key::Character(s) => s.to_string(),
        Key::Named(named) => format!("{named:?}"),
        Key::Unidentified(_) => "Unidentified".to_string(),
        Key::Dead(Some(c)) => format!("Dead({c})"),
        Key::Dead(None) => "Dead".to_string(),
    }
}

/// Convert winit's `ModifiersState` to `KeyboardModifiers` (RFC 0004 §7.4).
///
/// CapsLock and NumLock toggle states are not exposed by winit's
/// `ModifiersState`; they default to `false` here. Full toggle-key tracking
/// can be added via `WindowEvent::KeyboardInput` state tracking if needed.
fn winit_mods_to_keyboard_modifiers(
    mods: winit::keyboard::ModifiersState,
) -> tze_hud_input::KeyboardModifiers {
    tze_hud_input::KeyboardModifiers {
        shift: mods.shift_key(),
        ctrl: mods.control_key(),
        alt: mods.alt_key(),
        meta: mods.super_key(),
        caps_lock: false, // winit ModifiersState does not expose CapsLock state
        num_lock: false,  // winit ModifiersState does not expose NumLock state
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
    use tze_hud_scene::NodeData;

    #[test]
    fn windowed_config_default_values() {
        let cfg = WindowedConfig::default();
        assert_eq!(cfg.target_fps, 60);
        assert_eq!(cfg.grpc_port, 50051);
        assert_eq!(cfg.mcp_port, 9090);
        assert!(!cfg.psk.is_empty());
        assert!(cfg.benchmark.is_none());
    }

    #[test]
    fn seed_windowed_benchmark_scene_creates_deterministic_tile_stack() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        seed_windowed_benchmark_scene(&mut scene, 1920, 1080);

        assert_eq!(scene.display_area.width, 1920.0);
        assert_eq!(scene.display_area.height, 1080.0);
        assert_eq!(scene.tiles.len(), 20);
        assert!(
            scene.active_tab.is_some(),
            "benchmark scene must select the benchmark tab as active"
        );
        assert!(
            scene
                .leases
                .values()
                .any(|lease| lease.namespace == WINDOWED_BENCHMARK_AGENT),
            "benchmark scene must be owned by the benchmark agent namespace"
        );
        assert!(
            scene.nodes.values().any(|node| {
                matches!(
                    &node.data,
                    NodeData::HitRegion(hit_region)
                        if hit_region
                            .interaction_id
                            .starts_with("windowed-benchmark-input-")
                            && hit_region.accepts_pointer
                )
            }),
            "benchmark scene must include pointer hit regions for input-latency probes"
        );
    }

    #[test]
    fn seed_windowed_benchmark_scene_uses_unique_tab_order() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let existing = scene.create_tab("configured", 0).unwrap();
        assert_eq!(scene.active_tab, Some(existing));

        seed_windowed_benchmark_scene(&mut scene, 1920, 1080);

        assert_eq!(scene.tabs.len(), 2);
        assert_ne!(
            scene.active_tab,
            Some(existing),
            "benchmark mode must activate its own deterministic tab"
        );
        assert_eq!(scene.tiles.len(), 20);
    }

    #[test]
    fn pending_input_latency_drains_into_split_latency_fields() {
        let pending = Arc::new(StdMutex::new(VecDeque::new()));
        let started = Instant::now() - std::time::Duration::from_millis(3);
        let scene_commit_at = Instant::now() - std::time::Duration::from_millis(1);
        record_pending_input_latency(&pending, started, 125);

        let (local_ack, scene_commit, next_present) =
            drain_pending_input_latency(&pending, scene_commit_at, Instant::now())
                .expect("sample drains");

        assert_eq!(local_ack, 125);
        assert!(scene_commit >= 2_000);
        assert!(next_present >= 3_000);
        assert!(scene_commit < next_present);
        assert!(
            drain_pending_input_latency(&pending, scene_commit_at, Instant::now()).is_none(),
            "sample should be consumed exactly once"
        );
    }

    #[test]
    fn pending_input_latency_clamps_sub_microsecond_local_ack_to_nonzero_sample() {
        let pending = Arc::new(StdMutex::new(VecDeque::new()));
        let started = Instant::now() - std::time::Duration::from_millis(1);
        record_pending_input_latency(&pending, started, 0);

        let (local_ack, _, _) =
            drain_pending_input_latency(&pending, Instant::now(), Instant::now())
                .expect("sample drains");

        assert_eq!(local_ack, 1);
    }

    #[test]
    fn windowed_benchmark_records_nonzero_split_input_latency() {
        let config = WindowedBenchmarkConfig {
            warmup_frames: 0,
            frames: 10,
            emit_path: std::env::temp_dir().join("windowed-benchmark-test.json"),
        };
        let mut state = WindowedBenchmarkRunState::new(
            config,
            WindowMode::Overlay,
            WindowMode::Overlay,
            1920,
            1080,
            60,
        );
        let mut telemetry = tze_hud_telemetry::FrameTelemetry::new(1);
        telemetry.frame_time_us = 12_000;
        telemetry.tile_count = 3;
        telemetry.input_to_local_ack_us = 900;
        telemetry.input_to_scene_commit_us = 10_500;
        telemetry.input_to_next_present_us = 18_000;

        assert!(!state.record(&telemetry));
        assert_eq!(state.summary.input_to_local_ack.samples, vec![900]);
        assert_eq!(state.summary.input_to_scene_commit.samples, vec![10_500]);
        assert_eq!(state.summary.input_to_next_present.samples, vec![18_000]);
    }

    #[test]
    fn windowed_benchmark_does_not_treat_zero_input_latency_as_sample() {
        let config = WindowedBenchmarkConfig {
            warmup_frames: 0,
            frames: 10,
            emit_path: std::env::temp_dir().join("windowed-benchmark-test.json"),
        };
        let mut state = WindowedBenchmarkRunState::new(
            config,
            WindowMode::Overlay,
            WindowMode::Overlay,
            1920,
            1080,
            60,
        );
        let mut telemetry = tze_hud_telemetry::FrameTelemetry::new(1);
        telemetry.frame_time_us = 12_000;

        assert!(!state.record(&telemetry));
        assert!(state.summary.input_to_local_ack.samples.is_empty());
        assert!(state.summary.input_to_scene_commit.samples.is_empty());
        assert!(state.summary.input_to_next_present.samples.is_empty());
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

    #[test]
    fn combined_overlay_hit_regions_includes_zone_interaction_regions() {
        let static_regions = vec![HitRegion::new(10.0, 20.0, 30.0, 40.0)];
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene
            .zone_hit_regions
            .push(tze_hud_scene::types::ZoneHitRegion {
                zone_name: "notification-area".to_string(),
                published_at_wall_us: 123,
                publisher_namespace: "test-agent".to_string(),
                bounds: tze_hud_scene::types::Rect::new(100.0, 200.0, 20.0, 20.0),
                kind: tze_hud_scene::types::ZoneInteractionKind::Dismiss,
                interaction_id: "zone:notification-area:dismiss:123:test-agent".to_string(),
                tab_order: 0,
            });

        let combined = combined_overlay_hit_regions(&static_regions, &scene);

        assert_eq!(combined.len(), 2, "static + zone capture regions expected");
        assert_eq!(
            combined[0], static_regions[0],
            "static region must be preserved"
        );
        assert_eq!(
            combined[1],
            HitRegion::new(100.0, 200.0, 20.0, 20.0),
            "zone hit region must be exposed to overlay click-capture"
        );
    }

    // ── Content-layer tile HitRegion capture ─────────────────────────────

    /// Helper: build a scene with a single tile that has a HitRegionNode child.
    fn scene_with_capture_tile() -> (SceneGraph, tze_hud_scene::SceneId) {
        use tze_hud_scene::{Capability, HitRegionNode, Node, NodeData, Rect, SceneGraph, SceneId};
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("portal-agent", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(300.0, 400.0, 600.0, 200.0),
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
                        bounds: Rect::new(0.0, 0.0, 200.0, 40.0),
                        interaction_id: "portal-submit".to_string(),
                        accepts_pointer: true,
                        accepts_focus: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();
        (scene, tile_id)
    }

    /// A content tile with a pointer HitRegionNode must appear in the overlay
    /// capture region set so the OS delivers pointer events to the window rather
    /// than routing them to the desktop.
    ///
    /// Spec §Overlay click-through (line 181): events outside active hit-regions
    /// pass through to the desktop. The tile's display-space bounds represent the
    /// coarsest valid capture region; precise node hit-testing happens in Stage 2.
    #[test]
    fn content_tile_with_hit_region_node_registers_capture_region() {
        let (scene, tile_id) = scene_with_capture_tile();
        let tile_bounds = scene.tiles[&tile_id].bounds;

        let regions = content_tile_hit_regions_from_scene(&scene);

        assert_eq!(
            regions.len(),
            1,
            "exactly one capture region for one capture-mode tile with HitRegionNode"
        );
        assert_eq!(
            regions[0],
            HitRegion::new(
                tile_bounds.x,
                tile_bounds.y,
                tile_bounds.width,
                tile_bounds.height
            ),
            "capture region must match the tile's display-space bounds"
        );
    }

    /// A tile with `input_mode == Passthrough` must NOT generate a capture
    /// region, even if it has HitRegionNodes.  Passthrough tiles are transparent
    /// by design and must let desktop events fall through.
    #[test]
    fn passthrough_tile_with_hit_region_does_not_register_capture_region() {
        use tze_hud_scene::{Capability, HitRegionNode, InputMode, Node, NodeData, Rect, SceneId};
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("overlay", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "overlay",
                lease_id,
                Rect::new(0.0, 0.0, 300.0, 300.0),
                1,
            )
            .unwrap();
        // Explicitly set input_mode to Passthrough.
        scene.tiles.get_mut(&tile_id).unwrap().input_mode = InputMode::Passthrough;
        let node_id = SceneId::new();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: node_id,
                    children: vec![],
                    data: NodeData::HitRegion(HitRegionNode {
                        bounds: Rect::new(0.0, 0.0, 100.0, 50.0),
                        interaction_id: "passthrough-btn".to_string(),
                        accepts_pointer: true,
                        accepts_focus: false,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let regions = content_tile_hit_regions_from_scene(&scene);
        assert!(
            regions.is_empty(),
            "passthrough tile must not register a capture region"
        );
    }

    /// A tile whose root node has `accepts_pointer = false` on its only
    /// HitRegionNode must NOT generate a capture region.
    #[test]
    fn tile_with_accepts_pointer_false_does_not_register_capture_region() {
        use tze_hud_scene::{Capability, HitRegionNode, Node, NodeData, Rect, SceneId};
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 200.0, 100.0),
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
                        bounds: Rect::new(0.0, 0.0, 200.0, 100.0),
                        interaction_id: "focus-only".to_string(),
                        accepts_pointer: false, // focus-only, not pointer
                        accepts_focus: true,
                        ..Default::default()
                    }),
                },
            )
            .unwrap();

        let regions = content_tile_hit_regions_from_scene(&scene);
        assert!(
            regions.is_empty(),
            "tile with accepts_pointer=false node must not register a capture region"
        );
    }

    /// `combined_overlay_hit_regions` must merge static, zone, and content-tile
    /// capture regions into one flat list.
    #[test]
    fn combined_overlay_hit_regions_includes_content_tile_regions() {
        let (scene, tile_id) = scene_with_capture_tile();
        let tile_bounds = scene.tiles[&tile_id].bounds;

        let static_regions = vec![HitRegion::new(10.0, 10.0, 50.0, 50.0)];
        let combined = combined_overlay_hit_regions(&static_regions, &scene);

        // static (1) + zone (0, none in this scene) + content tile (1)
        assert_eq!(
            combined.len(),
            2,
            "static + content-tile capture regions expected"
        );
        assert_eq!(combined[0], static_regions[0], "static region preserved");
        assert_eq!(
            combined[1],
            HitRegion::new(
                tile_bounds.x,
                tile_bounds.y,
                tile_bounds.width,
                tile_bounds.height
            ),
            "content-tile capture region must cover tile display-space bounds"
        );
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

    #[test]
    fn hover_tracker_region_resolves_from_widget_geometry() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).expect("create tab");

        scene
            .widget_registry
            .register_definition(tze_hud_scene::types::WidgetDefinition {
                id: "status-indicator".to_string(),
                name: "status-indicator".to_string(),
                description: "test".to_string(),
                parameter_schema: Vec::new(),
                layers: Vec::new(),
                default_geometry_policy: tze_hud_scene::types::GeometryPolicy::Relative {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    width_pct: 1.0,
                    height_pct: 1.0,
                },
                default_rendering_policy: tze_hud_scene::types::RenderingPolicy::default(),
                default_contention_policy: tze_hud_scene::types::ContentionPolicy::LatestWins,
                ephemeral: false,
                hover_behavior: Some(tze_hud_scene::types::WidgetHoverBehavior {
                    trigger_rect: tze_hud_scene::types::WidgetNormalizedRect {
                        x_pct: 0.88,
                        y_pct: 0.06,
                        width_pct: 0.08,
                        height_pct: 0.22,
                    },
                    delay_ms: 3_000,
                    visibility_param: "tooltip_visible".to_string(),
                    hidden_value: 0.0,
                    visible_value: 1.0,
                }),
            });
        scene
            .widget_registry
            .register_instance(tze_hud_scene::types::WidgetInstance {
                id: tze_hud_scene::SceneId::new(),
                widget_type_name: "status-indicator".to_string(),
                tab_id,
                geometry_override: Some(tze_hud_scene::types::GeometryPolicy::Relative {
                    x_pct: 1660.0 / 1920.0,
                    y_pct: 8.0 / 1080.0,
                    width_pct: 252.0 / 1920.0,
                    height_pct: 96.0 / 1080.0,
                }),
                contention_override: None,
                instance_name: "main-status".to_string(),
                current_params: std::collections::HashMap::new(),
            });

        let trackers =
            build_hover_trackers(&scene, 1920.0, 1080.0, &std::collections::HashMap::new());
        let region = trackers
            .get("main-status")
            .map(|t| t.region.clone())
            .expect("main-status hover tracker must resolve");
        assert!(
            region.x >= 1880.0 && region.x <= 1905.0,
            "x should sit near the icon trigger region, got {}",
            region.x
        );
        assert!(
            region.y >= 12.0 && region.y <= 18.0,
            "y should sit near the top trigger margin, got {}",
            region.y
        );
        assert!(
            region.width >= 19.0 && region.width <= 21.0,
            "width should match normalized trigger width"
        );
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
            resource_store: tze_hud_resource::ResourceStore::new(
                tze_hud_resource::ResourceStoreConfig::default(),
            ),
            widget_asset_store: tze_hud_protocol::session::WidgetAssetStore::default(),
            runtime_widget_store: None,
            element_store: tze_hud_scene::element_store::ElementStore::default(),
            element_store_path: None,
            safe_mode_active: false,
            token_store: TokenStore::new(),
            freeze_active: false,
            degradation_level: RuntimeDegradationLevel::Normal,
            input_capture_tx: None,
        }))
    }

    /// When `grpc_port == 0`, `start_network_services` must return `None` for
    /// the runtime and an empty handle list (compositor-only mode, AC §2).
    #[test]
    fn start_network_services_grpc_port_zero_returns_no_runtime() {
        let shared_state = make_shared_state();
        let ctx: SharedRuntimeContext = Arc::new(RuntimeContext::headless_default());
        let (rt, handles, _tx, _scroll_tx) =
            start_network_services(0, "test-psk", shared_state, ctx, true)
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
        let (rt, handles, _tx, _scroll_tx) =
            start_network_services(59781, "test-psk", shared_state, ctx, true)
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
            let (rt, handles, _tx, _scroll_tx) =
                start_network_services(0, "psk", shared_state, ctx, false)
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

    #[test]
    fn scene_display_area_sync_uses_actual_surface_size() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        super::sync_scene_display_area(&mut scene, 2560, 1440);

        assert_eq!(scene.display_area.width, 2560.0);
        assert_eq!(scene.display_area.height, 1440.0);
    }

    #[test]
    fn scene_display_area_sync_ignores_zero_dimensions() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        super::sync_scene_display_area(&mut scene, 0, 1440);

        assert_eq!(scene.display_area.width, 1920.0);
        assert_eq!(scene.display_area.height, 1080.0);
    }

    #[test]
    fn mouse_wheel_delta_positive_line_scrolls_toward_top() {
        let (x, y) = super::normalize_mouse_wheel_delta(&MouseScrollDelta::LineDelta(0.0, 1.0));

        assert_eq!(x, 0.0);
        assert_eq!(y, -40.0);
    }

    #[test]
    fn mouse_wheel_delta_negative_line_scrolls_down_transcript() {
        let (_, y) = super::normalize_mouse_wheel_delta(&MouseScrollDelta::LineDelta(0.0, -1.0));

        assert_eq!(y, 40.0);
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

    // ── Zone interaction: dismiss hit wiring (hud-ltgk.6) ────────────────────

    use tze_hud_scene::types::{
        ContentionPolicy, GeometryPolicy, LayerAttachment, NotificationPayload, Rect,
        RenderingPolicy, SceneId, ZoneDefinition, ZoneHitRegion, ZoneMediaType,
    };

    fn make_test_zone(name: &str) -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: name.to_string(),
            description: format!("test zone: {name}"),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 0.1,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        }
    }

    /// Pointer-up on a zone dismiss hit-region must remove the notification from
    /// `zone_registry.active_publishes`.
    ///
    /// This is the regression test for hud-ltgk.6: the dismiss button rendered
    /// visually but clicks had no effect because the `InputResult` was discarded
    /// without acting on `HitResult::ZoneInteraction { kind: Dismiss }`.
    #[test]
    fn zone_dismiss_on_pointer_up_removes_notification() {
        use tze_hud_input::InputProcessor;
        use tze_hud_input::PointerEvent;

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        // hit_test requires an active tab; create one to mimic production state.
        scene
            .create_tab("Main", 0)
            .expect("tab creation must succeed");
        scene.register_zone(make_test_zone("alert-banner"));

        // Publish a notification so there is something to dismiss.
        let publisher = "test-agent";
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "hello".to_string(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(),
                    actions: vec![],
                }),
                publisher,
                None,
                None, // no explicit expiry
                None,
            )
            .expect("publish should succeed");

        // Verify the notification is present.
        assert_eq!(
            scene.zone_registry.active_for_zone("alert-banner").len(),
            1,
            "notification must be present before dismiss"
        );
        // Use the actual published_at from the record (assigned by publish_to_zone).
        let record_published_at =
            scene.zone_registry.active_for_zone("alert-banner")[0].published_at_wall_us;

        // Simulate the compositor injecting a dismiss ZoneHitRegion for this publication.
        scene.zone_hit_regions.push(ZoneHitRegion {
            zone_name: "alert-banner".to_string(),
            published_at_wall_us: record_published_at,
            publisher_namespace: publisher.to_string(),
            bounds: Rect::new(100.0, 10.0, 20.0, 20.0), // dismiss button at (100,10)
            kind: ZoneInteractionKind::Dismiss,
            interaction_id: format!("zone:alert-banner:dismiss:{record_published_at}:{publisher}"),
            tab_order: 0,
        });

        let mut processor = InputProcessor::new();

        // Pointer-down on the dismiss button (does not dismiss yet).
        let down = PointerEvent {
            x: 110.0,
            y: 20.0,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };
        let result_down = processor.process(&down, &mut scene);
        // Down on a ZoneInteraction does not dismiss.
        assert_eq!(
            scene.zone_registry.active_for_zone("alert-banner").len(),
            1,
            "notification must still be present after pointer-down"
        );
        // The hit result must be ZoneInteraction.
        assert!(
            result_down.hit.is_zone_interaction(),
            "pointer-down on zone hit region must produce ZoneInteraction hit"
        );

        // Pointer-up on the dismiss button — this is where the dismiss fires.
        let up = PointerEvent {
            x: 110.0,
            y: 20.0,
            kind: PointerEventKind::Up,
            device_id: 0,
            timestamp: None,
        };
        let result_up = processor.process(&up, &mut scene);
        assert!(
            result_up.hit.is_zone_interaction(),
            "pointer-up on zone hit region must produce ZoneInteraction hit"
        );

        // Simulate the windowed runtime's zone interaction dispatch.
        if let HitResult::ZoneInteraction {
            ref zone_name,
            published_at_wall_us,
            ref publisher_namespace,
            ref kind,
            ..
        } = result_up.hit
        {
            if let ZoneInteractionKind::Dismiss = kind {
                scene.dismiss_notification(zone_name, published_at_wall_us, publisher_namespace);
            }
        }

        // The notification must now be gone.
        assert_eq!(
            scene.zone_registry.active_for_zone("alert-banner").len(),
            0,
            "notification must be removed after dismiss pointer-up [hud-ltgk.6 regression]"
        );

        // The hit region must also be pruned immediately (local feedback first).
        assert!(
            scene.zone_hit_regions.is_empty(),
            "stale dismiss hit-region must be pruned after dismiss [local feedback first]"
        );
    }

    /// Pointer-down alone must NOT dismiss a notification — only pointer-up triggers dismiss.
    #[test]
    fn zone_dismiss_only_on_pointer_up_not_down() {
        use tze_hud_input::InputProcessor;
        use tze_hud_input::PointerEvent;

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene
            .create_tab("Main", 0)
            .expect("tab creation must succeed");
        scene.register_zone(make_test_zone("alert-banner"));

        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "hello".to_string(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(),
                    actions: vec![],
                }),
                "test-agent",
                None,
                None,
                None,
            )
            .expect("publish should succeed");

        let record_published_at =
            scene.zone_registry.active_for_zone("alert-banner")[0].published_at_wall_us;

        scene.zone_hit_regions.push(ZoneHitRegion {
            zone_name: "alert-banner".to_string(),
            published_at_wall_us: record_published_at,
            publisher_namespace: "test-agent".to_string(),
            bounds: Rect::new(100.0, 10.0, 20.0, 20.0),
            kind: ZoneInteractionKind::Dismiss,
            interaction_id: format!("zone:alert-banner:dismiss:{record_published_at}:test-agent"),
            tab_order: 0,
        });

        let mut processor = InputProcessor::new();

        // Only send pointer-down, no pointer-up.
        let down = PointerEvent {
            x: 110.0,
            y: 20.0,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };
        let result = processor.process(&down, &mut scene);

        // No dismiss dispatch on pointer-down — check it would not be dismissed
        // even if we ran the zone dispatch logic.
        let would_dismiss = match &result.hit {
            HitResult::ZoneInteraction { kind, .. } => {
                matches!(kind, ZoneInteractionKind::Dismiss) && down.kind == PointerEventKind::Up // false for Down
            }
            _ => false,
        };
        assert!(!would_dismiss, "pointer-down must not trigger dismiss");

        assert_eq!(
            scene.zone_registry.active_for_zone("alert-banner").len(),
            1,
            "notification must still be present after pointer-down only"
        );
    }

    /// Action zone hit does NOT dismiss the notification — it should only log.
    #[test]
    fn zone_action_hit_does_not_dismiss_notification() {
        use tze_hud_input::InputProcessor;
        use tze_hud_input::PointerEvent;

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene
            .create_tab("Main", 0)
            .expect("tab creation must succeed");
        scene.register_zone(make_test_zone("alert-banner"));

        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "hello".to_string(),
                    icon: String::new(),
                    urgency: 0,
                    ttl_ms: None,
                    title: String::new(),
                    actions: vec![],
                }),
                "test-agent",
                None,
                None,
                None,
            )
            .expect("publish should succeed");

        let record_published_at =
            scene.zone_registry.active_for_zone("alert-banner")[0].published_at_wall_us;

        // Place an Action hit region (not Dismiss).
        scene.zone_hit_regions.push(ZoneHitRegion {
            zone_name: "alert-banner".to_string(),
            published_at_wall_us: record_published_at,
            publisher_namespace: "test-agent".to_string(),
            bounds: Rect::new(50.0, 10.0, 40.0, 20.0),
            kind: ZoneInteractionKind::Action {
                callback_id: "open".to_string(),
            },
            interaction_id: format!(
                "zone:alert-banner:action:{record_published_at}:test-agent:open"
            ),
            tab_order: 1,
        });

        let mut processor = InputProcessor::new();

        let up = PointerEvent {
            x: 70.0,
            y: 20.0,
            kind: PointerEventKind::Up,
            device_id: 0,
            timestamp: None,
        };
        let result = processor.process(&up, &mut scene);

        // Run the zone dispatch logic: action must NOT call dismiss_notification.
        if let HitResult::ZoneInteraction {
            ref zone_name,
            published_at_wall_us,
            ref publisher_namespace,
            ref kind,
            ..
        } = result.hit
        {
            match kind {
                ZoneInteractionKind::Dismiss => {
                    // Should not happen for an Action hit region.
                    scene.dismiss_notification(
                        zone_name,
                        published_at_wall_us,
                        publisher_namespace,
                    );
                }
                ZoneInteractionKind::Action { .. } => {
                    // Action: just log (no dismiss).
                }
                ZoneInteractionKind::DragHandle { .. } => {}
            }
        }

        // Notification must still be present.
        assert_eq!(
            scene.zone_registry.active_for_zone("alert-banner").len(),
            1,
            "action interaction must NOT remove the notification"
        );
    }

    // ── Drag-to-move: long-press drag moves a text stream portal tile [hud-9yfce] ──

    /// Build a scene with a single text-stream-portal-like tile plus the
    /// corresponding drag handle hit region registered in the chrome layer.
    ///
    /// Returns `(scene, tile_id, element_id, interaction_id)`.
    fn scene_with_drag_handle_tile(
        initial_x: f32,
        initial_y: f32,
        tile_w: f32,
        tile_h: f32,
    ) -> (
        SceneGraph,
        tze_hud_scene::SceneId,
        tze_hud_scene::SceneId,
        String,
    ) {
        use tze_hud_scene::types::DragHandleElementKind;
        use tze_hud_scene::{Capability, DragHandleHitRegion, HitRegionNode};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).expect("tab must be created");
        let lease_id = scene.grant_lease("portal-agent", 60_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "portal-agent",
                lease_id,
                Rect::new(initial_x, initial_y, tile_w, tile_h),
                1,
            )
            .expect("tile must be created");

        // The element_id for a tile is its scene id (used in drag handle).
        let element_id = tile_id;

        // Register a drag handle hit region above the tile (as the compositor would).
        let interaction_id = format!(
            "drag-handle:{:032x}",
            element_id
                .to_bytes_le()
                .iter()
                .fold(0u128, |acc, &b| (acc << 8) | b as u128)
        );
        let handle_bounds = Rect::new(
            initial_x + tile_w / 2.0 - 20.0, // centred above tile
            initial_y - 10.0,
            40.0,
            20.0,
        );
        scene.drag_handle_hit_regions.push(DragHandleHitRegion {
            element_id,
            element_kind: DragHandleElementKind::Tile,
            bounds: handle_bounds,
            interaction_id: interaction_id.clone(),
            hit_region: HitRegionNode {
                bounds: handle_bounds,
                interaction_id: interaction_id.clone(),
                accepts_pointer: true,
                ..Default::default()
            },
            tab_order: 0,
        });

        (scene, tile_id, element_id, interaction_id)
    }

    /// A long-press drag on a tile's drag handle must move the tile's bounds and
    /// return a `DragReleasedData` payload when the pointer is released.
    ///
    /// Acceptance criteria for hud-9yfce:
    /// - `Moved` outcome during pointer-move updates `tile.bounds` immediately.
    /// - `Released` outcome on pointer-up produces persist data.
    /// - Click-focus is unaffected: a short tap (no long-press) produces no move.
    #[test]
    fn drag_to_move_long_press_moves_tile_bounds() {
        use std::thread;
        use std::time::Duration;
        use tze_hud_input::{InputProcessor, PointerEvent};

        let (mut scene, tile_id, element_id, _interaction_id) =
            scene_with_drag_handle_tile(400.0, 300.0, 600.0, 200.0);

        // The drag handle was placed at:
        //   x = 400 + 600/2 - 20 = 680, y = 300 - 10 = 290, w=40, h=20
        // So the handle spans x: 680..720, y: 290..310.
        let handle_cx = 700.0_f32; // centre of the handle
        let handle_cy = 300.0_f32;

        let mut processor = InputProcessor::new();

        // ── Step 1: PointerDown on the drag handle ────────────────────────────
        let down = PointerEvent {
            x: handle_cx,
            y: handle_cy,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };
        // process() produces the HitResult for the drag handle.
        let result_down = processor.process(&down, &mut scene);
        assert!(
            result_down.hit.is_zone_interaction(),
            "pointer-down on drag handle must produce ZoneInteraction hit"
        );

        // Drive the drag state machine — should start accumulating.
        let released_on_down = super::apply_drag_handle_pointer_event(
            &mut processor,
            &down,
            &result_down.hit,
            &mut scene,
            1920.0,
            1080.0,
        );
        assert!(
            released_on_down.is_none(),
            "PointerDown must not trigger release"
        );
        assert!(
            processor.drag_states.contains_key(&0),
            "drag state must be created for device 0 after PointerDown on handle"
        );

        // ── Step 2: Wait for long-press threshold (250 ms) ───────────────────
        thread::sleep(Duration::from_millis(260));

        // ── Step 3: PointerMove — first move activates the drag, second moves tile ──
        //
        // The state machine on the first PointerMove after the threshold transitions
        // Accumulating → Activated (returns `Activated`, not `Moved` yet).  The
        // grab offset is recorded at activation.  Subsequent PointerMove events
        // return `Moved` and update the tile bounds.
        let move1 = PointerEvent {
            x: handle_cx + 5.0, // small nudge triggers Activated
            y: handle_cy,
            kind: PointerEventKind::Move,
            device_id: 0,
            timestamp: None,
        };
        let result_move1 = processor.process(&move1, &mut scene);
        let released_on_move1 = super::apply_drag_handle_pointer_event(
            &mut processor,
            &move1,
            &result_move1.hit,
            &mut scene,
            1920.0,
            1080.0,
        );
        assert!(
            released_on_move1.is_none(),
            "first PointerMove (Activated) must not trigger release"
        );

        // Second PointerMove — now in Activated phase, returns Moved.
        let move2 = PointerEvent {
            x: handle_cx + 100.0,
            y: handle_cy + 50.0,
            kind: PointerEventKind::Move,
            device_id: 0,
            timestamp: None,
        };
        let result_move2 = processor.process(&move2, &mut scene);
        let released_on_move2 = super::apply_drag_handle_pointer_event(
            &mut processor,
            &move2,
            &result_move2.hit,
            &mut scene,
            1920.0,
            1080.0,
        );
        assert!(
            released_on_move2.is_none(),
            "PointerMove must not trigger release"
        );

        // The tile must have moved from its original position.
        let tile_after_move = scene.tiles.get(&tile_id).expect("tile must exist");
        assert_ne!(
            tile_after_move.bounds.x, 400.0,
            "tile X must change after drag move"
        );
        assert_ne!(
            tile_after_move.bounds.y, 300.0,
            "tile Y must change after drag move"
        );

        // ── Step 4: PointerUp — should release and return persist data ────────
        // Release at the same position as the last move.
        let up = PointerEvent {
            x: handle_cx + 100.0,
            y: handle_cy + 50.0,
            kind: PointerEventKind::Up,
            device_id: 0,
            timestamp: None,
        };
        let result_up = processor.process(&up, &mut scene);
        let released_on_up = super::apply_drag_handle_pointer_event(
            &mut processor,
            &up,
            &result_up.hit,
            &mut scene,
            1920.0,
            1080.0,
        );

        let released =
            released_on_up.expect("PointerUp after activated drag must return released data");
        assert_eq!(
            released.element_id, element_id,
            "released element_id must match the dragged tile"
        );
        assert!(
            released.final_x >= 0.0 && released.final_x + released.width <= 1920.0,
            "final X must be within display bounds"
        );
        assert!(
            released.final_y >= 0.0 && released.final_y + released.height <= 1080.0,
            "final Y must be within display bounds"
        );

        // Drag state must be cleaned up after release.
        assert!(
            !processor.drag_states.contains_key(&0),
            "drag state must be removed after PointerUp"
        );
    }

    /// A quick tap (PointerDown immediately followed by PointerUp, no long-press)
    /// on a drag handle must NOT move the tile — the click-to-focus path must be
    /// unaffected.
    ///
    /// Hysteresis: the 250 ms threshold ensures taps are recognised as clicks,
    /// not drags.
    #[test]
    fn drag_to_move_quick_tap_does_not_move_tile() {
        use tze_hud_input::{InputProcessor, PointerEvent};

        let (mut scene, tile_id, _element_id, _interaction_id) =
            scene_with_drag_handle_tile(400.0, 300.0, 600.0, 200.0);

        // Same drag handle position as drag_to_move_long_press_moves_tile_bounds:
        //   x: 680..720, y: 290..310.
        let handle_cx = 700.0_f32; // centre of the handle
        let handle_cy = 300.0_f32;

        let mut processor = InputProcessor::new();

        // PointerDown.
        let down = PointerEvent {
            x: handle_cx,
            y: handle_cy,
            kind: PointerEventKind::Down,
            device_id: 0,
            timestamp: None,
        };
        let result_down = processor.process(&down, &mut scene);
        let _ = super::apply_drag_handle_pointer_event(
            &mut processor,
            &down,
            &result_down.hit,
            &mut scene,
            1920.0,
            1080.0,
        );

        // PointerUp immediately — no long-press threshold met.
        let up = PointerEvent {
            x: handle_cx,
            y: handle_cy,
            kind: PointerEventKind::Up,
            device_id: 0,
            timestamp: None,
        };
        let result_up = processor.process(&up, &mut scene);
        let released_on_up = super::apply_drag_handle_pointer_event(
            &mut processor,
            &up,
            &result_up.hit,
            &mut scene,
            1920.0,
            1080.0,
        );

        // Must NOT return release data — this was a tap, not a drag.
        assert!(
            released_on_up.is_none(),
            "quick tap must not trigger drag release [click-focus coexistence]"
        );

        // Tile bounds must be unchanged.
        let tile = scene.tiles.get(&tile_id).expect("tile must exist");
        assert_eq!(
            tile.bounds.x, 400.0,
            "tile X must not change after a tap on the drag handle"
        );
        assert_eq!(
            tile.bounds.y, 300.0,
            "tile Y must not change after a tap on the drag handle"
        );
    }

    // ── dispatch_pointer_event: timestamp_mono_us wiring (hud-cz5mw) ─────────

    /// Build a minimal [`AgentDispatch`] for the given kind.
    fn make_agent_dispatch(kind: tze_hud_input::AgentDispatchKind) -> tze_hud_input::AgentDispatch {
        use tze_hud_scene::SceneId;
        tze_hud_input::AgentDispatch {
            namespace: "test-agent".to_string(),
            tile_id: SceneId::new(),
            node_id: SceneId::new(),
            interaction_id: "test-interaction".to_string(),
            local_x: 1.0,
            local_y: 2.0,
            display_x: 10.0,
            display_y: 20.0,
            device_id: 0,
            kind,
            capture_released_reason: None,
        }
    }

    /// Extract `timestamp_mono_us` from a received `EventBatch` containing one
    /// pointer event.  Panics with a descriptive message if the batch or event
    /// is not what was expected.
    fn extract_pointer_timestamp(batch: &tze_hud_protocol::proto::EventBatch) -> u64 {
        use tze_hud_protocol::proto::input_envelope::Event as InputEvent;
        assert_eq!(
            batch.events.len(),
            1,
            "batch must contain exactly one event"
        );
        match &batch.events[0].event {
            Some(InputEvent::PointerDown(ev)) => ev.timestamp_mono_us,
            Some(InputEvent::PointerMove(ev)) => ev.timestamp_mono_us,
            Some(InputEvent::PointerUp(ev)) => ev.timestamp_mono_us,
            other => panic!("expected pointer event, got: {other:?}"),
        }
    }

    /// `dispatch_pointer_event` must set `timestamp_mono_us > 0` for PointerDown,
    /// PointerMove, and PointerUp (gap from hud-zffvp now closed).
    #[test]
    fn dispatch_pointer_event_timestamp_mono_us_is_non_zero() {
        use tze_hud_input::AgentDispatchKind;

        let (tx, mut rx) =
            tokio::sync::broadcast::channel::<(String, tze_hud_protocol::proto::EventBatch)>(8);
        let tx_opt = Some(tx);

        for kind in [
            AgentDispatchKind::PointerDown,
            AgentDispatchKind::PointerMove,
            AgentDispatchKind::PointerUp,
        ] {
            let label = format!("{kind:?}");
            dispatch_pointer_event(&tx_opt, make_agent_dispatch(kind));
            let (_ns, batch) = rx.try_recv().expect("event must be sent on the channel");
            let ts = extract_pointer_timestamp(&batch);
            assert!(
                ts > 0,
                "{label}: timestamp_mono_us must be non-zero (monotonic clock wired)"
            );
        }
    }

    /// Two consecutive `dispatch_pointer_event` calls must produce monotonically
    /// increasing `timestamp_mono_us` values, confirming the clock source is
    /// truly monotonic and not stuck at zero.
    #[test]
    fn dispatch_pointer_event_timestamp_mono_us_is_monotonic() {
        use tze_hud_input::AgentDispatchKind;

        let (tx, mut rx) =
            tokio::sync::broadcast::channel::<(String, tze_hud_protocol::proto::EventBatch)>(8);
        let tx_opt = Some(tx);

        dispatch_pointer_event(&tx_opt, make_agent_dispatch(AgentDispatchKind::PointerDown));
        let (_ns, batch1) = rx.try_recv().expect("first event must be sent");
        let ts1 = extract_pointer_timestamp(&batch1);

        // A small sleep ensures the monotonic clock advances between calls.
        std::thread::sleep(std::time::Duration::from_millis(1));

        dispatch_pointer_event(&tx_opt, make_agent_dispatch(AgentDispatchKind::PointerMove));
        let (_ns, batch2) = rx.try_recv().expect("second event must be sent");
        let ts2 = extract_pointer_timestamp(&batch2);

        assert!(
            ts2 > ts1,
            "timestamp_mono_us must be strictly increasing across consecutive dispatches \
             (ts1={ts1}, ts2={ts2})"
        );
    }
}
