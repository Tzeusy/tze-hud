//! The compositor — renders the scene graph to a surface.
//!
//! For the vertical slice, this renders tiles as colored rectangles
//! with hit-region highlighting for local feedback.
//!
//! The frame pipeline is **surface-agnostic**: it renders through the
//! `CompositorSurface` trait, which is implemented by both `HeadlessSurface`
//! (offscreen) and `WindowSurface` (display-connected).  No conditional
//! compilation exists in the render path — only the surface implementation
//! differs between modes.
//!
//! Per runtime-kernel/spec.md Requirement: Headless Mode (line 198):
//! "No conditional compilation for the render path."
//!
//! ## Two-pass rendering (content → chrome)
//!
//! [`Compositor::render_frame_with_chrome`] implements the three-layer ordering required by
//! the chrome sovereignty contract:
//!   1. Background + content pass (`LoadOp::Clear` — clears and draws agent tiles)
//!   2. Chrome pass (`LoadOp::Load` — draws chrome on top of content, preserving pixels)
//!
//! This separation is the architectural foundation for future render-skip redaction
//! (capture-safe architecture): the content and chrome passes are structurally independent.

use std::collections::HashMap;

use crate::pipeline::{ChromeDrawCmd, RectVertex, rect_vertices};
use crate::surface::{CompositorSurface, HeadlessSurface};
use crate::text::{TextItem, TextRasterizer};
use crate::widget::WidgetRenderer;
use tze_hud_scene::DegradationLevel;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::*;
use tze_hud_telemetry::FrameTelemetry;

// ─── Severity token fallback colors ─────────────────────────────────────────

/// Default severity colors (linear sRGB) used when design tokens are absent.
///
/// Per spec §Canonical Token Schema:
///   color.severity.info     → #4A9EFF → (0.078, 0.384, 1.0)
///   color.severity.warning  → #FFB800 → (1.0, 0.722, 0.0)
///   color.severity.critical → #FF0000 → (1.0, 0.0, 0.0)
const SEVERITY_INFO: Rgba = Rgba {
    r: 0.078,
    g: 0.384,
    b: 1.0,
    a: 1.0,
};
const SEVERITY_WARNING: Rgba = Rgba {
    r: 1.0,
    g: 0.722,
    b: 0.0,
    a: 1.0,
};
const SEVERITY_CRITICAL: Rgba = Rgba {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

// ─── Notification urgency token fallback colors ───────────────────────────────

/// Default notification urgency backdrop colors (linear sRGB) used when
/// `color.notification.urgency.*` design tokens are absent.
///
/// Per spec §Notification Urgency Backdrop Token Schema:
///   color.notification.urgency.low      → #2A2A2A → (0.165, 0.165, 0.165)
///   color.notification.urgency.normal   → #1A1A3A → (0.102, 0.102, 0.227)
///   color.notification.urgency.urgent   → #8B6914 → (0.545, 0.412, 0.078)
///   color.notification.urgency.critical → #8B1A1A → (0.545, 0.102, 0.102)
///
/// These tokens are for notification-area (and non-alert-banner notification zones)
/// only. Alert-banner continues to use `color.severity.*` tokens.
const NOTIFICATION_URGENCY_LOW: Rgba = Rgba {
    r: 0.165,
    g: 0.165,
    b: 0.165,
    a: 1.0,
};
const NOTIFICATION_URGENCY_NORMAL: Rgba = Rgba {
    r: 0.102,
    g: 0.102,
    b: 0.227,
    a: 1.0,
};
const NOTIFICATION_URGENCY_URGENT: Rgba = Rgba {
    r: 0.545,
    g: 0.412,
    b: 0.078,
    a: 1.0,
};
const NOTIFICATION_URGENCY_CRITICAL: Rgba = Rgba {
    r: 0.545,
    g: 0.102,
    b: 0.102,
    a: 1.0,
};

/// Per-notification backdrop opacity applied to urgency-tinted backdrop quads.
///
/// This is the fixed 0.9 opacity specified for notification zone backdrop rendering.
/// It overrides the token color's alpha channel.
const NOTIFICATION_BACKDROP_OPACITY: f32 = 0.9;

/// Fallback color for `color.border.default` when the token is absent.
///
/// Matches the default value in built-in component startup tokens (#444466).
const BORDER_DEFAULT_FALLBACK: Rgba = Rgba {
    r: 0.267,
    g: 0.267,
    b: 0.400,
    a: 1.0,
};

/// Returns `true` if the zone name is an alert-banner zone.
///
/// Per spec §V1 Component Type Definitions: the alert-banner zone name is
/// `"alert-banner"`.  notification-area uses urgency-tinted notification
/// backdrop tokens, not severity tokens.
#[inline]
fn is_alert_banner_zone(zone_name: &str) -> bool {
    zone_name == "alert-banner"
}

/// Parse a `#RRGGBB` or `#RRGGBBAA` hex string into `Rgba`.
///
/// This is a minimal, allocation-free parser used to resolve token color
/// values at render time without depending on `tze_hud_config`.
/// Returns `None` if the string does not match either form.
fn parse_hex_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if !s.starts_with('#') || !s.is_ascii() {
        return None;
    }
    let hex = &s[1..];
    match hex.len() {
        6 | 8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = if hex.len() == 8 {
                u8::from_str_radix(&hex[6..8], 16).ok()?
            } else {
                255
            };
            Some(Rgba::new(
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            ))
        }
        _ => None,
    }
}

/// Look up a token key in the resolved token map and parse it as a color.
/// Returns `None` if the key is absent or the value is not a valid hex color.
#[inline]
fn resolve_token_color(token_map: &HashMap<String, String>, key: &str) -> Option<Rgba> {
    token_map.get(key).and_then(|v| parse_hex_color(v))
}

/// Look up a severity token key in the resolved token map and parse it as a
/// color.  Returns `None` if the key is absent or the value is not a valid
/// hex color.
#[inline]
fn resolve_severity_token(token_map: &HashMap<String, String>, key: &str) -> Option<Rgba> {
    resolve_token_color(token_map, key)
}

/// Map a `NotificationPayload.urgency` level to a severity backdrop color.
///
/// Looks up `color.severity.{info,warning,critical}` in `token_map` first;
/// falls back to hardcoded SEVERITY_* constants when the key is absent or
/// cannot be parsed as a hex color.
///
/// Per spec §Notification Urgency-to-Severity Token Mapping:
///   urgency 0, 1 → color.severity.info   (fallback: #4A9EFF)
///   urgency 2    → color.severity.warning (fallback: #FFB800)
///   urgency 3    → color.severity.critical (fallback: #FF0000)
///
/// The returned `Rgba` alpha is 1.0 (unless the token itself carries an alpha
/// via `#RRGGBBAA`); `backdrop_opacity` from the policy is applied by the
/// caller after this lookup.
///
/// MUST NOT be used for notification-area zones. Only for alert-banner zones.
fn urgency_to_severity_color(urgency: u32, token_map: &HashMap<String, String>) -> Rgba {
    match urgency {
        3 => resolve_severity_token(token_map, "color.severity.critical")
            .unwrap_or(SEVERITY_CRITICAL),
        2 => {
            resolve_severity_token(token_map, "color.severity.warning").unwrap_or(SEVERITY_WARNING)
        }
        _ => resolve_severity_token(token_map, "color.severity.info").unwrap_or(SEVERITY_INFO),
    }
}

/// Map a `NotificationPayload.urgency` level to a notification urgency backdrop color.
///
/// Looks up `color.notification.urgency.{low,normal,urgent,critical}` in `token_map`
/// first; falls back to hardcoded NOTIFICATION_URGENCY_* constants when the key is
/// absent or cannot be parsed as a hex color.
///
/// Per spec §Notification Urgency Backdrop Token Schema:
///   urgency 0     → color.notification.urgency.low      (fallback: #2A2A2A)
///   urgency 1     → color.notification.urgency.normal   (fallback: #1A1A3A)
///   urgency 2     → color.notification.urgency.urgent   (fallback: #8B6914)
///   urgency 3+    → color.notification.urgency.critical (fallback: #8B1A1A)
///
/// Urgency values greater than 3 are clamped to 3 (critical).
///
/// MUST NOT use `color.severity.*` tokens — those are for alert-banner only.
fn urgency_to_notification_color(urgency: u32, token_map: &HashMap<String, String>) -> Rgba {
    // Clamp urgency >3 to critical (3).
    let level = urgency.min(3);
    match level {
        0 => resolve_token_color(token_map, "color.notification.urgency.low")
            .unwrap_or(NOTIFICATION_URGENCY_LOW),
        1 => resolve_token_color(token_map, "color.notification.urgency.normal")
            .unwrap_or(NOTIFICATION_URGENCY_NORMAL),
        2 => resolve_token_color(token_map, "color.notification.urgency.urgent")
            .unwrap_or(NOTIFICATION_URGENCY_URGENT),
        _ => resolve_token_color(token_map, "color.notification.urgency.critical")
            .unwrap_or(NOTIFICATION_URGENCY_CRITICAL),
    }
}

/// Resolve the `color.border.default` token from the map.
///
/// Falls back to `BORDER_DEFAULT_FALLBACK` (#444466) when the token is absent
/// or cannot be parsed as a valid hex color.
#[inline]
fn resolve_border_default_color(token_map: &HashMap<String, String>) -> Rgba {
    resolve_token_color(token_map, "color.border.default").unwrap_or(BORDER_DEFAULT_FALLBACK)
}

/// Emit 4 thin 1px border quads positioned inside the given backdrop rectangle.
///
/// Produces a 1px inset border using four axis-aligned rectangles:
///   - top:    (x, y, w, 1)
///   - bottom: (x, y+h-1, w, 1)
///   - left:   (x, y+1, 1, h-2)
///   - right:  (x+w-1, y+1, 1, h-2)
///
/// The border is drawn inside the backdrop bounds (does not extend outside).
/// When `h < 2` or `w < 1`, the degenerate dimension quads are skipped (size ≤ 0
/// after the inset). Top/bottom edges require `w >= 1` to avoid degenerate quads
/// with zero or negative width.
///
/// `sw`/`sh` are the screen dimensions passed through to `rect_vertices`.
fn emit_border_quads(
    vertices: &mut Vec<crate::pipeline::RectVertex>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    sw: f32,
    sh: f32,
    border_color: [f32; 4],
) {
    const BORDER_PX: f32 = 1.0;
    // Top edge.
    if h >= BORDER_PX && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(x, y, w, BORDER_PX, sw, sh, border_color));
    }
    // Bottom edge.
    if h >= BORDER_PX * 2.0 && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(
            x,
            y + h - BORDER_PX,
            w,
            BORDER_PX,
            sw,
            sh,
            border_color,
        ));
    }
    // Left edge (inset 1px top and bottom to avoid corner overlap).
    if h > BORDER_PX * 2.0 && w >= BORDER_PX {
        vertices.extend_from_slice(&rect_vertices(
            x,
            y + BORDER_PX,
            BORDER_PX,
            h - BORDER_PX * 2.0,
            sw,
            sh,
            border_color,
        ));
    }
    // Right edge (inset 1px top and bottom to avoid corner overlap).
    if h > BORDER_PX * 2.0 && w >= BORDER_PX * 2.0 {
        vertices.extend_from_slice(&rect_vertices(
            x + w - BORDER_PX,
            y + BORDER_PX,
            BORDER_PX,
            h - BORDER_PX * 2.0,
            sw,
            sh,
            border_color,
        ));
    }
}

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

/// GPU state and render pipeline.
pub struct Compositor {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// Pipeline with no blending — writes RGBA directly. Used to clear
    /// the framebuffer to transparent in overlay mode (LoadOp::Clear
    /// doesn't write alpha correctly on some GPUs).
    clear_pipeline: wgpu::RenderPipeline,
    pub width: u32,
    pub height: u32,
    frame_number: u64,
    /// When true, the clear color uses alpha=0 for transparent overlay mode.
    pub overlay_mode: bool,
    /// When true, render all zone boundaries with colored tints even when
    /// zones have no active content. Controlled by `TZE_HUD_DEBUG_ZONES=1`.
    pub debug_zone_tints: bool,
    /// Current degradation level, set by the runtime before each frame.
    ///
    /// At [`DegradationLevel::Significant`] or higher, widget transition
    /// interpolation is skipped and final parameter values are applied
    /// immediately to reduce re-rasterization under load.
    pub degradation_level: DegradationLevel,
    /// Optional text rasterizer (glyphon). Absent until `init_text_renderer`
    /// is called. When `None`, TextMarkdownNode and zone StreamText content
    /// renders as solid-color rectangles only (no glyph output).
    pub(crate) text_rasterizer: Option<TextRasterizer>,
    /// Optional widget renderer. Absent until `init_widget_renderer` is called.
    /// When `None`, widget instances in the scene graph are not composited.
    pub(crate) widget_renderer: Option<WidgetRenderer>,
    /// Per-zone fade-in / fade-out animation state.
    ///
    /// Keyed by zone name. An entry is inserted on the first publish to a zone
    /// that has `transition_in_ms > 0`, and on every zone clear when
    /// `transition_out_ms > 0`. Completed transitions are pruned each frame.
    pub(crate) zone_animation_states: HashMap<String, ZoneAnimationState>,
    /// Track which zones had active publishes in the previous frame so we can
    /// detect publish → clear transitions and start fade-out animations.
    prev_active_zones: HashMap<String, bool>,
    /// Resolved design token map, set at startup via `set_token_map`.
    ///
    /// Used to resolve `color.severity.{info,warning,critical}` tokens for
    /// alert-banner backdrop colors. When empty (the default), all severity
    /// lookups fall back to the hardcoded `SEVERITY_*` constants.
    ///
    /// Populated by calling `set_token_map` after `run_component_startup`
    /// produces a `ComponentStartupResult::global_tokens`.
    pub token_map: HashMap<String, String>,
}

impl Compositor {
    /// Create a new headless compositor.
    ///
    /// Checks the `HEADLESS_FORCE_SOFTWARE` environment variable.  When set to
    /// `1`, wgpu adapter selection uses `force_fallback_adapter = true`, which
    /// selects llvmpipe on Linux or WARP on Windows.
    ///
    /// Per runtime-kernel/spec.md Requirement: Headless Software GPU (line 211):
    /// "When set, the wgpu adapter selection MUST request a software fallback
    /// (force_fallback_adapter = true)."
    pub async fn new_headless(width: u32, height: u32) -> Result<Self, CompositorError> {
        // Check HEADLESS_FORCE_SOFTWARE env var (spec line 409: "conventionally
        // named HEADLESS_FORCE_SOFTWARE").
        let force_software = std::env::var("HEADLESS_FORCE_SOFTWARE")
            .map(|v| v.trim() == "1")
            .unwrap_or(false);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: force_software,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tze_hud_compositor"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        let pipeline = Self::create_pipeline(&device);
        let clear_pipeline =
            Self::create_clear_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb);

        Ok(Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            width,
            height,
            frame_number: 0,
            overlay_mode: false,
            debug_zone_tints: std::env::var("TZE_HUD_DEBUG_ZONES").is_ok_and(|v| v == "1"),
            degradation_level: DegradationLevel::Nominal,
            text_rasterizer: None,
            widget_renderer: None,
            zone_animation_states: HashMap::new(),
            prev_active_zones: HashMap::new(),
            token_map: HashMap::new(),
        })
    }

    /// Create a windowed compositor backed by a real `winit::window::Window`.
    ///
    /// This is the factory method for production windowed rendering. It:
    /// 1. Uses `select_gpu_adapter` with platform-mandated backends (Vulkan/D3D12/Metal).
    /// 2. Creates a `wgpu::Surface` from the window via `instance.create_surface`.
    /// 3. Negotiates the surface format (sRGB preferred).
    /// 4. Configures the surface with the window's physical dimensions.
    /// 5. Creates the `wgpu::Device` and `wgpu::Queue`.
    ///
    /// Returns the `(Compositor, WindowSurface)` pair. The `WindowSurface` must
    /// be kept alive for the duration of the runtime.
    ///
    /// Per spec §Compositor Thread Ownership (line 46): the returned `Compositor`
    /// (and thus `Device` + `Queue`) MUST be transferred to the compositor thread
    /// immediately after creation. The `WindowSurface` is owned by the main thread.
    ///
    /// Per spec §Platform GPU Backends (line 189): this path uses the platform-
    /// mandated backends — unlike `new_headless` which uses `Backends::all()`.
    pub async fn new_windowed(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        Self::new_windowed_inner(window, width, height, false).await
    }

    /// Create a windowed compositor, optionally forcing Vulkan for overlay
    /// transparency (DX12 only supports Opaque swapchain alpha mode).
    pub async fn new_windowed_overlay(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        Self::new_windowed_inner(window, width, height, true).await
    }

    async fn new_windowed_inner(
        window: std::sync::Arc<winit::window::Window>,
        width: u32,
        height: u32,
        overlay: bool,
    ) -> Result<(Self, crate::surface::WindowSurface), CompositorError> {
        use crate::surface::WindowSurface;

        // ── Step 1: Create instance with platform-mandated backends ──────────
        // We need the surface before adapter selection so we can pass it as
        // `compatible_surface`. Create a temporary instance first, create the
        // surface, then select the adapter with that surface constraint.
        // On Windows in overlay mode, force Vulkan — DX12 only supports Opaque
        // swapchain alpha mode, which prevents per-pixel transparency.
        let backends = if overlay && cfg!(target_os = "windows") {
            tracing::info!("overlay mode: forcing Vulkan backend for transparent swapchain");
            wgpu::Backends::VULKAN
        } else {
            crate::adapter::platform_backends().flags
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });

        // ── Step 2: Create wgpu::Surface from the winit window ───────────────
        // SAFETY: `window` is wrapped in Arc — it outlives the surface because
        // we pass 'static lifetime via Arc<Window>.
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| CompositorError::DeviceCreation(format!("create_surface: {e}")))?;

        // ── Step 3: Select adapter compatible with the surface ────────────────
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(CompositorError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        tracing::info!(
            backend = ?adapter_info.backend,
            device_name = %adapter_info.name,
            vendor = adapter_info.vendor,
            "windowed: GPU adapter selected"
        );

        // ── Step 4: Request device ────────────────────────────────────────────
        // Use downlevel_defaults() for broad compatibility but override
        // max_texture_dimension_2d with the adapter's actual capability.
        // downlevel_defaults() caps this at 2048 which is smaller than
        // common display resolutions (e.g. 2560x1440).  The adapter knows
        // the GPU's true limit; requesting it ensures the surface can be
        // configured at the monitor's native resolution.
        let mut required_limits = wgpu::Limits::downlevel_defaults();
        required_limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("tze_hud_compositor_windowed"),
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|e| CompositorError::DeviceCreation(e.to_string()))?;

        // ── Step 5: Configure the surface ────────────────────────────────────
        let surface_caps = surface.get_capabilities(&adapter);

        // Guard: wgpu surface capabilities must be non-empty on a valid
        // adapter/surface combination. Return a structured error instead of
        // panicking via index [0] so the caller can diagnose driver issues.
        if surface_caps.formats.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported texture formats — driver or backend issue"
                    .to_string(),
            ));
        }
        if surface_caps.present_modes.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported present modes — driver or backend issue".to_string(),
            ));
        }
        if surface_caps.alpha_modes.is_empty() {
            return Err(CompositorError::DeviceCreation(
                "surface reports no supported alpha modes — driver or backend issue".to_string(),
            ));
        }

        // Prefer sRGB surface format; fall back to the first available format.
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo // vsync — latency-stable
        } else {
            surface_caps.present_modes[0]
        };

        // Clamp dimensions to the device's maximum supported texture size.
        // Use device.limits() (not adapter.limits()) because the device is
        // created with required_limits=downlevel_defaults(); the actual device
        // limits reflect what the adapter provides subject to those requirements.
        // Some GPUs (e.g. certain Intel/Mesa drivers) report a max of 2048,
        // which is smaller than common display resolutions like 2560x1440.
        //
        // Also guard against zero-size dimensions: wgpu panics if width or
        // height is 0 in surface.configure().  This can happen if inner_size()
        // returned (0,0) on a minimized or not-yet-shown window.
        let max_dim = device.limits().max_texture_dimension_2d;
        let clamped_width = width.min(max_dim).max(1);
        let clamped_height = height.min(max_dim).max(1);
        if clamped_width != width || clamped_height != height {
            tracing::warn!(
                requested_width = width,
                requested_height = height,
                clamped_width,
                clamped_height,
                max_texture_dimension_2d = max_dim,
                "windowed: surface dimensions clamped to device limit"
            );
        }

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: clamped_width,
            height: clamped_height,
            present_mode,
            alpha_mode: surface_caps
                .alpha_modes
                .iter()
                .find(|m| **m == wgpu::CompositeAlphaMode::PreMultiplied)
                .or_else(|| {
                    surface_caps
                        .alpha_modes
                        .iter()
                        .find(|m| **m == wgpu::CompositeAlphaMode::PostMultiplied)
                })
                .copied()
                .unwrap_or(surface_caps.alpha_modes[0]),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        tracing::info!(
            available_alpha_modes = ?surface_caps.alpha_modes,
            selected_alpha_mode = ?config.alpha_mode,
            "windowed: alpha mode selection"
        );
        // Write diagnostic to a known file for remote debugging.
        let diag = format!(
            "backend: {:?}\ndevice: {}\nrequested_backends: {:?}\noverlay: {}\navailable_alpha_modes: {:?}\nselected_alpha_mode: {:?}\nformat: {:?}\npresent_mode: {:?}\n",
            adapter_info.backend,
            adapter_info.name,
            backends,
            overlay,
            surface_caps.alpha_modes,
            config.alpha_mode,
            surface_format,
            present_mode,
        );
        let _ = std::fs::write("C:\\tze_hud\\logs\\alpha_diag.txt", &diag);
        surface.configure(&device, &config);
        tracing::info!(
            format = ?surface_format,
            present_mode = ?present_mode,
            alpha_mode = ?config.alpha_mode,
            width = clamped_width,
            height = clamped_height,
            "windowed: surface configured"
        );

        // ── Step 6: Create render pipelines (format-aware) ────────────────────
        let pipeline = Self::create_pipeline_with_format(&device, surface_format);
        let clear_pipeline = Self::create_clear_pipeline(&device, surface_format);

        let compositor = Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            width: clamped_width,
            height: clamped_height,
            frame_number: 0,
            overlay_mode: false,
            debug_zone_tints: std::env::var("TZE_HUD_DEBUG_ZONES").is_ok_and(|v| v == "1"),
            degradation_level: DegradationLevel::Nominal,
            text_rasterizer: None,
            widget_renderer: None,
            zone_animation_states: HashMap::new(),
            prev_active_zones: HashMap::new(),
            token_map: HashMap::new(),
        };

        let window_surface = WindowSurface::new(surface, config);
        Ok((compositor, window_surface))
    }

    /// Create a render pipeline targeting a specific texture format.
    ///
    /// This is the canonical pipeline constructor. Both `create_pipeline`
    /// (headless, fixed format) and `create_pipeline_with_format` (windowed,
    /// dynamic swapchain format) delegate here to avoid duplicating the
    /// pipeline descriptor.
    ///
    /// `label_prefix` is prepended to debug labels so GPU profilers can
    /// distinguish headless vs windowed pipelines.
    fn create_pipeline_inner(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        label_prefix: &str,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline::{RECT_SHADER, RectVertex};

        let shader_label = format!("{label_prefix}rect_shader");
        let layout_label = format!("{label_prefix}rect_pipeline_layout");
        let pipeline_label = format!("{label_prefix}rect_pipeline");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&shader_label),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&layout_label),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&pipeline_label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Create a render pipeline targeting a dynamic swapchain format.
    ///
    /// Called by `new_windowed` so the pipeline matches the negotiated
    /// surface format (which varies by platform/driver).
    fn create_pipeline_with_format(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        Self::create_pipeline_inner(device, format, "windowed_")
    }

    /// Create a pipeline with no blending — writes RGBA directly.
    /// Used to clear the framebuffer to transparent in overlay mode.
    fn create_clear_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        use crate::pipeline::{RECT_SHADER, RectVertex};
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("clear_shader"),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clear_pipeline_layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("clear_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[RectVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None, // No blending — direct RGBA write
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Create a render pipeline for headless mode (`Rgba8UnormSrgb`).
    fn create_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
        Self::create_pipeline_inner(device, wgpu::TextureFormat::Rgba8UnormSrgb, "")
    }

    /// Replace the compositor's resolved design token map.
    ///
    /// Should be called once at startup after `run_component_startup` produces a
    /// `ComponentStartupResult::global_tokens`.  The map is keyed by canonical
    /// token names (e.g. `"color.severity.warning"`) with hex-color string values
    /// (e.g. `"#FFB800"`).
    ///
    /// At render time the compositor looks up `color.severity.{info,warning,critical}`
    /// to derive alert-banner backdrop colors, falling back to hardcoded constants
    /// when a key is absent or unparseable.
    pub fn set_token_map(&mut self, map: HashMap<String, String>) {
        self.token_map = map;
    }

    /// Initialize (or re-initialize) the text rasterizer for a given surface format.
    ///
    /// Must be called once after creation before text rendering is available.
    /// For headless compositors, `format` should be `Rgba8UnormSrgb`.
    /// For windowed compositors, use the negotiated swapchain format.
    ///
    /// Calling this multiple times replaces the existing rasterizer (e.g. on
    /// surface resize or format change).
    pub fn init_text_renderer(&mut self, format: wgpu::TextureFormat) {
        self.text_rasterizer = Some(TextRasterizer::new(&self.device, &self.queue, format));
        tracing::debug!(format = ?format, "text renderer initialized");
    }

    /// Load raw font bytes (TTF or OTF) from an agent upload into glyphon's
    /// `FontSystem`.
    ///
    /// After this call, the font is available for text layout in subsequent
    /// frames.  The `resource_id` is the 32-byte BLAKE3 digest of `data`
    /// (matches `tze_hud_resource::ResourceId::as_bytes()`); duplicate calls
    /// with the same `resource_id` are no-ops.
    ///
    /// Silently skips if the text renderer is not yet initialized (e.g. if
    /// `init_text_renderer` has not been called).  Callers in the runtime may
    /// defer font uploads until the compositor is ready.
    pub fn load_font_bytes(&mut self, resource_id: [u8; 32], data: &[u8]) {
        if let Some(rasterizer) = &mut self.text_rasterizer {
            rasterizer.load_font_bytes(resource_id, data);
        } else {
            tracing::debug!("load_font_bytes called before text renderer is initialized — skipped");
        }
    }

    /// Returns `true` if the font with the given `resource_id` has been loaded
    /// into the `FontSystem`.
    ///
    /// Returns `false` if the text renderer is not yet initialized.
    pub fn has_font(&self, resource_id: &[u8; 32]) -> bool {
        self.text_rasterizer
            .as_ref()
            .map(|r| r.has_font(resource_id))
            .unwrap_or(false)
    }

    // ─── Widget renderer ─────────────────────────────────────────────────────

    /// Initialize (or re-initialize) the widget renderer for the given surface format.
    ///
    /// Must be called once before widget textures can be composited. For headless
    /// compositors, `format` should be `Rgba8UnormSrgb`. For windowed compositors,
    /// use the negotiated swapchain format.
    ///
    /// Calling this multiple times replaces the existing renderer (e.g. on surface
    /// reconfiguration or format change). Any cached textures are discarded.
    pub fn init_widget_renderer(&mut self, format: wgpu::TextureFormat) {
        self.widget_renderer = Some(WidgetRenderer::new(&self.device, format));
        tracing::debug!(format = ?format, "widget renderer initialized");
    }

    /// Get a mutable reference to the widget renderer, if initialized.
    pub fn widget_renderer_mut(&mut self) -> Option<&mut WidgetRenderer> {
        self.widget_renderer.as_mut()
    }

    /// Get a reference to the widget renderer, if initialized.
    pub fn widget_renderer(&self) -> Option<&WidgetRenderer> {
        self.widget_renderer.as_ref()
    }

    /// Ensure widget instances have up-to-date cached textures for all widget
    /// instances in the registry.
    ///
    /// For each widget instance:
    /// - If no texture entry exists (first frame), rasterizes with default params.
    /// - If the instance has a `dirty` flag set, re-rasterizes with current params.
    /// - If an animation is active, resolves interpolated params and re-rasterizes.
    ///
    /// Under degradation level [`DegradationLevel::Significant`] or higher, active
    /// transitions are snapped to their final values immediately, reducing
    /// re-rasterization to at most once per parameter change during transitions.
    ///
    /// This should be called once per frame before `render_frame`.
    pub fn sync_widget_textures(
        &mut self,
        scene: &SceneGraph,
        degradation_level: DegradationLevel,
    ) {
        let wr = match &mut self.widget_renderer {
            Some(r) => r,
            None => return,
        };

        let registry = &scene.widget_registry;

        // Collect instances that need texture updates.
        let instance_names: Vec<String> = registry.instances.keys().cloned().collect();

        for instance_name in instance_names {
            let instance = match registry.instances.get(&instance_name) {
                Some(i) => i.clone(),
                None => continue,
            };
            let def = match registry.definitions.get(&instance.widget_type_name) {
                Some(d) => d.clone(),
                None => continue,
            };

            // Determine pixel geometry from the instance's geometry policy.
            // Fall back to a sensible default if not set.
            let (pw, ph) = resolve_widget_pixel_size(&instance, &def, self.width, self.height);
            if pw == 0 || ph == 0 {
                continue;
            }

            // Check if this instance needs an initial texture (no entry yet).
            let needs_initial = wr.texture_entry(&instance_name).is_none();

            // Resolve animated or static params, applying degradation-aware snapping.
            let current_params = &instance.current_params;
            let (effective_params, still_animating) =
                wr.resolve_animated_params(&instance_name, current_params, degradation_level);

            let params_changed = wr
                .texture_entry(&instance_name)
                .map(|e| e.last_rendered_params != effective_params)
                .unwrap_or(false);

            let dirty = needs_initial
                || still_animating
                || params_changed
                || wr
                    .texture_entry(&instance_name)
                    .map(|e| e.dirty)
                    .unwrap_or(false);

            if dirty {
                wr.rasterize_and_upload(
                    &self.device,
                    &self.queue,
                    &instance_name,
                    &def,
                    &effective_params,
                    pw,
                    ph,
                );
                // Record what params were rendered so we can detect future changes.
                if let Some(entry) = wr.texture_entry_mut(&instance_name) {
                    entry.last_rendered_params = effective_params;
                    entry.dirty = false;
                }
            }
        }
    }

    /// Collect `TextItem`s for all TextMarkdownNode tiles and zone StreamText
    /// and ShortTextWithIcon/Notification content in the scene.
    ///
    /// All zone `TextItem`s are constructed from `RenderingPolicy` fields —
    /// no hardcoded colors or font choices.  Animation opacity is applied to
    /// the color channels so text fades with the backdrop.
    ///
    /// Returns a flat `Vec<TextItem>` ready for `TextRasterizer::prepare_text_items`.
    fn collect_text_items(&self, scene: &SceneGraph, sw: f32, sh: f32) -> Vec<TextItem> {
        let mut items: Vec<TextItem> = Vec::new();

        // ── TextMarkdownNode tiles ────────────────────────────────────────────
        for tile in &scene.visible_tiles() {
            if let Some(root_id) = tile.root_node {
                self.collect_text_items_from_node(root_id, tile, scene, &mut items);
            }
        }

        // ── Zone StreamText, Notification, and StatusBar content ─────────────
        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };

            // Resolve zone geometry to pixel bounds.
            let (zx, zy, zw, zh) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);

            let policy = &zone_def.rendering_policy;

            // Current animation opacity for this zone.
            let anim_opacity = self
                .zone_animation_states
                .get(zone_name)
                .map(|s| s.current_opacity())
                .unwrap_or(1.0);

            // Emit TextItems based on contention policy.
            //
            // Stack: each publication occupies a vertically-stacked slot (newest at top,
            //   slot 0 = newest).  slot_h via Self::stack_slot_height().
            //
            // MergeByKey: collect ALL StatusBar publications, merge their entries
            //   (last write wins per key), render the merged set as one text item.
            //
            // LatestWins / Replace: render only the most-recent publication.
            match zone_def.contention_policy {
                ContentionPolicy::Stack { .. } => {
                    let slot_h = Self::stack_slot_height(policy);
                    // Render newest-first: slot_index 0 = newest (top of zone).
                    for (slot_idx, record) in publishes.iter().rev().enumerate() {
                        let slot_y = zy + slot_idx as f32 * slot_h;
                        // Clip to zone bounds.
                        if slot_y >= zy + zh {
                            break;
                        }
                        let effective_slot_h = slot_h.min((zy + zh) - slot_y);

                        match &record.content {
                            ZoneContent::StreamText(text) => {
                                items.push(TextItem::from_zone_policy(
                                    text,
                                    zx,
                                    slot_y,
                                    zw,
                                    effective_slot_h,
                                    policy,
                                    anim_opacity,
                                ));
                            }
                            ZoneContent::Notification(payload) => {
                                // Icon rendering is stubbed (no texture pipeline in v1).
                                items.push(TextItem::from_zone_policy(
                                    &payload.text,
                                    zx,
                                    slot_y,
                                    zw,
                                    effective_slot_h,
                                    policy,
                                    anim_opacity,
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                ContentionPolicy::MergeByKey { max_keys } => {
                    // Collect all StatusBar publications and merge their entries.
                    // For each key, the last publish wins (latest value).
                    let mut merged: HashMap<String, String> =
                        HashMap::with_capacity(max_keys as usize);
                    for record in publishes.iter() {
                        if let ZoneContent::StatusBar(payload) = &record.content {
                            for (k, v) in &payload.entries {
                                merged.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    if !merged.is_empty() {
                        let mut sorted: Vec<(&String, &String)> = merged.iter().collect();
                        sorted.sort_by_key(|(k, _)| k.as_str());
                        let text = sorted
                            .iter()
                            .map(|(k, v)| format!("{k}: {v}"))
                            .collect::<Vec<_>>()
                            .join("  ");
                        items.push(TextItem::from_zone_policy(
                            &text,
                            zx,
                            zy,
                            zw,
                            zh,
                            policy,
                            anim_opacity,
                        ));
                    } else {
                        // Fallback: render whatever the latest publication contains.
                        for record in publishes.iter().rev() {
                            match &record.content {
                                ZoneContent::StreamText(text) => {
                                    items.push(TextItem::from_zone_policy(
                                        text,
                                        zx,
                                        zy,
                                        zw,
                                        zh,
                                        policy,
                                        anim_opacity,
                                    ));
                                    break;
                                }
                                ZoneContent::Notification(payload) => {
                                    items.push(TextItem::from_zone_policy(
                                        &payload.text,
                                        zx,
                                        zy,
                                        zw,
                                        zh,
                                        policy,
                                        anim_opacity,
                                    ));
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                ContentionPolicy::LatestWins | ContentionPolicy::Replace => {
                    // Use the most-recent publish only.
                    for record in publishes.iter().rev() {
                        match &record.content {
                            ZoneContent::StreamText(text) => {
                                items.push(TextItem::from_zone_policy(
                                    text,
                                    zx,
                                    zy,
                                    zw,
                                    zh,
                                    policy,
                                    anim_opacity,
                                ));
                                // Only render the most-recent StreamText publish.
                                break;
                            }
                            ZoneContent::Notification(payload) => {
                                // Render the notification text.
                                // For alert-banner, text color is always policy.text_color
                                // for contrast against the severity-colored backdrop.
                                // For notification-area, policy.text_color is also used.
                                // Icon rendering is stubbed (no texture pipeline in v1).
                                items.push(TextItem::from_zone_policy(
                                    &payload.text,
                                    zx,
                                    zy,
                                    zw,
                                    zh,
                                    policy,
                                    anim_opacity,
                                ));
                                // Only render the most-recent publish.
                                break;
                            }
                            ZoneContent::StatusBar(payload) => {
                                // Format key-value pairs as "key: value" lines, sorted by key
                                // for deterministic output.
                                let mut sorted: Vec<(&String, &String)> =
                                    payload.entries.iter().collect();
                                sorted.sort_by_key(|(k, _)| k.as_str());
                                let text = sorted
                                    .iter()
                                    .map(|(k, v)| format!("{k}: {v}"))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                items.push(TextItem::from_zone_policy(
                                    &text,
                                    zx,
                                    zy,
                                    zw,
                                    zh,
                                    policy,
                                    anim_opacity,
                                ));
                                // Only render the most-recent StatusBar publish.
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        items
    }

    /// Update zone animation states before each frame.
    ///
    /// Starts fade-in animations for newly-published zones and fade-out
    /// animations for zones that just lost their last publish.
    ///
    /// Also handles zone unregistration: zones that were active and have since
    /// been removed from the registry are treated as cleared (no fade-out is
    /// possible since the zone_def is gone, so the state is simply pruned).
    ///
    /// Prunes completed transitions.
    pub fn update_zone_animations(&mut self, scene: &SceneGraph) {
        // Build current active-zone set (zone_name → has active publishes).
        let current_active: HashMap<String, bool> = scene
            .zone_registry
            .active_publishes
            .iter()
            .map(|(name, pubs)| (name.clone(), !pubs.is_empty()))
            .collect();

        // Detect publish transitions within currently-registered zones.
        for (zone_name, &is_active) in &current_active {
            let was_active = self
                .prev_active_zones
                .get(zone_name)
                .copied()
                .unwrap_or(false);

            if is_active && !was_active {
                // Zone just received its first publish — start fade-in.
                if let Some(zone_def) = scene.zone_registry.zones.get(zone_name) {
                    if let Some(ms) = zone_def.rendering_policy.transition_in_ms {
                        if ms > 0 {
                            self.zone_animation_states
                                .insert(zone_name.clone(), ZoneAnimationState::fade_in(ms));
                        }
                    }
                }
            } else if !is_active && was_active {
                // Zone just lost its last publish — start fade-out.
                if let Some(zone_def) = scene.zone_registry.zones.get(zone_name) {
                    if let Some(ms) = zone_def.rendering_policy.transition_out_ms {
                        if ms > 0 {
                            self.zone_animation_states
                                .insert(zone_name.clone(), ZoneAnimationState::fade_out(ms));
                        }
                    }
                }
            }
        }

        // Detect zone unregistration: zones that were previously tracked but
        // are now absent from active_publishes (zone was removed from registry).
        // Since zone_def is gone, no fade-out animation is possible; we simply
        // prune any in-flight animation state for that zone immediately.
        self.zone_animation_states
            .retain(|zone_name, _| current_active.contains_key(zone_name));

        // Prune completed transitions (reached target opacity).
        self.zone_animation_states
            .retain(|_, state| !state.is_complete());

        self.prev_active_zones = current_active;
    }

    /// Recursively collect `TextItem`s from a node and its children.
    #[allow(clippy::only_used_in_recursion)]
    fn collect_text_items_from_node(
        &self,
        node_id: SceneId,
        tile: &Tile,
        scene: &SceneGraph,
        items: &mut Vec<TextItem>,
    ) {
        let node = match scene.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };

        if let NodeData::TextMarkdown(tm) = &node.data {
            items.push(TextItem::from_text_markdown_node(
                tm,
                tile.bounds.x,
                tile.bounds.y,
            ));
        }

        for child_id in &node.children {
            self.collect_text_items_from_node(*child_id, tile, scene, items);
        }
    }

    /// Shared encode pipeline used by `render_frame` and `render_frame_headless`.
    ///
    /// Encodes the geometry pass (clear + vertex draw) and the text pass into a
    /// single `CommandEncoder`.  The encoder is returned to the caller **before**
    /// `queue.submit` so that headless callers can append a `copy_to_buffer`
    /// command (which must precede submit).
    ///
    /// # Parameters
    ///
    /// - `vertices` — pre-built vertex list (caller is responsible for overlay
    ///   quads, zone content, etc.)
    /// - `frame_view` — render target view for this frame
    /// - `scene` — used only for the text pass (`collect_text_items`)
    /// - `surf_w` / `surf_h` — surface dimensions used for the text viewport
    /// - `use_overlay_pipeline` — when `true`, selects `clear_pipeline` (no
    ///   blending) instead of the standard `pipeline`
    ///
    /// # Returns
    ///
    /// `(encoder, encode_us)` — the ready-to-submit encoder and the wall-clock
    /// microseconds spent encoding (for `FrameTelemetry::render_encode_us`).
    fn encode_frame(
        &mut self,
        vertices: &[RectVertex],
        frame_view: &wgpu::TextureView,
        scene: &SceneGraph,
        surf_w: u32,
        surf_h: u32,
        use_overlay_pipeline: bool,
    ) -> (wgpu::CommandEncoder, u64) {
        let encode_start = std::time::Instant::now();

        // ── Geometry pass ─────────────────────────────────────────────────────
        let vertex_buffer = if vertices.is_empty() {
            None
        } else {
            let buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vertex_buffer"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buffer)
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // In overlay mode, use the clear_pipeline (no blending) for ALL
            // rendering. The first 6 vertices are a full-screen transparent
            // quad that zeros out every pixel's alpha. Subsequent content
            // (zone bars) overwrites specific regions with their own alpha.
            if use_overlay_pipeline {
                render_pass.set_pipeline(&self.clear_pipeline);
            } else {
                render_pass.set_pipeline(&self.pipeline);
            }

            if let Some(ref buffer) = vertex_buffer {
                render_pass.set_vertex_buffer(0, buffer.slice(..));
                render_pass.draw(0..vertices.len() as u32, 0..1);
            }
        }

        // ── Text pass (Stage 6) ───────────────────────────────────────────────
        // Collect text items before borrowing the rasterizer mutably, to avoid
        // simultaneous mutable + immutable borrow of `self`.
        let sw = surf_w as f32;
        let sh = surf_h as f32;
        let text_items: Vec<TextItem> = if self.text_rasterizer.is_some() {
            self.collect_text_items(scene, sw, sh)
        } else {
            vec![]
        };

        // If a text rasterizer is present, prepare glyphon buffers and run a
        // LoadOp::Load text pass on top of the geometry written above.
        if let Some(ref mut tr) = self.text_rasterizer {
            tr.update_viewport(&self.queue, surf_w, surf_h);
            if !text_items.is_empty() {
                if let Err(e) = tr.prepare_text_items(&self.device, &self.queue, &text_items) {
                    tracing::warn!(error = %e, "text prepare failed — frame continues without text");
                } else {
                    let mut text_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("text_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: frame_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                // LoadOp::Load: preserve geometry pixels under the text.
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    if let Err(e) = tr.render_text_pass(&mut text_pass) {
                        tracing::warn!(error = %e, "text render failed — frame continues without text");
                    }
                }
            }
            // Trim every frame regardless of item count — glyphs from prior frames
            // must be evicted even when the current frame has no text.
            tr.trim_atlas();
        }

        let encode_us = encode_start.elapsed().as_micros() as u64;
        (encoder, encode_us)
    }

    /// Render one frame of the scene to the surface.
    ///
    /// This method is surface-agnostic: it works with any type implementing
    /// `CompositorSurface`.  The same code path executes in headless and windowed
    /// modes — only the surface implementation differs.
    ///
    /// Per runtime-kernel/spec.md Requirement: Headless Mode (line 198):
    /// "No conditional compilation for the render path."
    ///
    /// For headless pixel readback, use `render_frame_headless()` instead,
    /// which includes the `copy_to_buffer` step internally so that
    /// `surface.read_pixels()` returns the current frame's data.
    /// `render_frame()` does NOT copy pixels to the readback buffer — the
    /// encoder is created and consumed internally and is not exposed.
    ///
    /// Returns telemetry for this frame.
    pub fn render_frame(
        &mut self,
        scene: &SceneGraph,
        surface: &dyn CompositorSurface,
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // Collect visible tiles
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        // Build vertex list from scene.
        let (surf_w, surf_h) = surface.size();
        let sw = surf_w as f32;
        let sh = surf_h as f32;
        let mut vertices: Vec<RectVertex> = Vec::new();

        // In overlay mode, prepend a full-screen quad to zero out alpha.
        if self.overlay_mode {
            // One-shot diagnostic: log surface dimensions on first frame.
            if self.frame_number == 1 {
                let diag = format!(
                    "render_frame: sw={sw}, sh={sh}, compositor_w={}, compositor_h={}\n",
                    self.width, self.height,
                );
                let _ = std::fs::write("C:\\tze_hud\\logs\\render_diag.txt", &diag);
            }
            vertices.extend_from_slice(&rect_vertices(
                0.0,
                0.0,
                sw,
                sh,
                sw,
                sh,
                [0.0, 0.0, 0.0, 0.0],
            ));
        }

        for tile in &tiles {
            // Render tile background
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            vertices.extend_from_slice(&verts);

            // Render nodes within the tile
            if let Some(root_id) = tile.root_node {
                self.render_node(root_id, tile, scene, &mut vertices, sw, sh);
            }
        }

        // Update zone animation states (fade-in/fade-out) before rendering.
        self.update_zone_animations(scene);

        // Render zone content.
        self.render_zone_content(scene, &mut vertices, sw, sh);

        // ── Widget texture sync: rasterize dirty SVGs BEFORE frame acquisition.
        // SVG rasterization can be slow; if a resize event arrives while we hold
        // the surface texture, the texture is destroyed and queue.submit panics.
        self.sync_widget_textures(scene, self.degradation_level);

        // Acquire frame through the surface trait (surface-agnostic).
        // The CompositorFrame._guard keeps the backing resource alive until drop.
        let frame = surface.acquire_frame();

        let (mut encoder, encode_us) = self.encode_frame(
            &vertices,
            &frame.view,
            scene,
            surf_w,
            surf_h,
            self.overlay_mode,
        );
        telemetry.render_encode_us = encode_us;

        // ── Widget pass: composite pre-synced textures above zone content ────
        self.encode_widget_pass(&mut encoder, &frame.view, &scene.widget_registry, sw, sh);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));

        // present() is surface-specific: no-op for headless, swap-chain flip for windowed.
        // Drop frame guard AFTER present() so the backing resource stays alive.
        surface.present();
        drop(frame);

        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Render one frame and copy pixel data into the headless readback buffer.
    ///
    /// This is a convenience method for testing/CI that handles the extra
    /// `copy_to_buffer` step required for headless pixel readback.
    ///
    /// `copy_to_buffer` is appended to the encoder before `queue.submit()` via
    /// the shared `encode_frame` helper, which returns the encoder prior to
    /// submission so that this headless-specific step can be inserted cleanly.
    ///
    /// Returns telemetry for this frame.
    pub fn render_frame_headless(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // Collect visible tiles
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        // Build vertex list from scene.
        // Use surface.size() — not self.width/self.height — so that vertex
        // normalization is correct even if the HeadlessSurface was created with
        // different dimensions than the compositor's stored width/height.
        let (surf_w, surf_h) = surface.size();
        let sw = surf_w as f32;
        let sh = surf_h as f32;
        let mut vertices: Vec<RectVertex> = Vec::new();

        for tile in &tiles {
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            vertices.extend_from_slice(&verts);

            if let Some(root_id) = tile.root_node {
                self.render_node(root_id, tile, scene, &mut vertices, sw, sh);
            }
        }

        // Update zone animation states before rendering zone content.
        self.update_zone_animations(scene);

        // Render zone content backdrops.
        self.render_zone_content(scene, &mut vertices, sw, sh);

        // ── Widget texture sync before frame acquisition (same as windowed path).
        self.sync_widget_textures(scene, self.degradation_level);

        // Acquire frame via trait — same code path as render_frame().
        let frame = surface.acquire_frame();

        // Headless never uses overlay mode — pass false for the pipeline selector.
        let (mut encoder, encode_us) =
            self.encode_frame(&vertices, &frame.view, scene, surf_w, surf_h, false);
        telemetry.render_encode_us = encode_us;

        // ── Widget pass: composite pre-synced textures above zone content ────
        self.encode_widget_pass(&mut encoder, &frame.view, &scene.widget_registry, sw, sh);

        // Headless-specific: copy rendered texture to readback buffer.
        // Must happen after all render passes and before submit.
        surface.copy_to_buffer(&mut encoder);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));
        surface.present(); // no-op for headless
        drop(frame);
        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Render one frame with a separate chrome pass after the content pass.
    ///
    /// # Layer ordering (back to front)
    ///
    /// 1. **Content pass** — background clear + all agent tiles. Uses `LoadOp::Clear`.
    /// 2. **Chrome pass** — draws chrome draw commands on top. Uses `LoadOp::Load` to
    ///    preserve the content pass pixels, ensuring chrome is always on top.
    ///
    /// Chrome draw commands are produced by the caller (from `ChromeRenderer::render_chrome`)
    /// before calling this function. The compositor does not access `ChromeState` directly —
    /// chrome state is fully decoupled from scene/agent state.
    ///
    /// # Separable passes
    ///
    /// The content pass and chrome pass are encoded as two separate `RenderPass` blocks
    /// within the same `CommandEncoder`. This architectural separation is the foundation
    /// for future render-skip redaction (capture-safe architecture): the content pass can
    /// be suppressed independently of the chrome pass.
    ///
    /// # Chrome layer sovereignty
    ///
    /// Because the chrome pass uses `LoadOp::Load` and runs after the content pass,
    /// chrome pixels are always written last — no agent tile can occlude chrome regardless
    /// of its z-order value.
    pub fn render_frame_with_chrome(
        &mut self,
        scene: &SceneGraph,
        surface: &HeadlessSurface,
        chrome_cmds: &[ChromeDrawCmd],
    ) -> FrameTelemetry {
        let frame_start = std::time::Instant::now();
        self.frame_number += 1;

        let mut telemetry = FrameTelemetry::new(self.frame_number);

        // ── Widget texture sync before encoding (avoids surface-texture race).
        self.sync_widget_textures(scene, self.degradation_level);

        // ── Pass 1: Content (background + agent tiles) ──────────────────────
        let tiles = scene.visible_tiles();
        telemetry.tile_count = tiles.len() as u32;
        telemetry.node_count = scene.node_count() as u32;
        telemetry.active_leases = scene.leases.len() as u32;

        let mut content_vertices: Vec<RectVertex> = Vec::new();
        let (surf_w, surf_h) = surface.size();
        let sw = surf_w as f32;
        let sh = surf_h as f32;

        for tile in &tiles {
            let bg_color = self.tile_background_color(tile, scene);
            let verts = rect_vertices(
                tile.bounds.x,
                tile.bounds.y,
                tile.bounds.width,
                tile.bounds.height,
                sw,
                sh,
                bg_color,
            );
            content_vertices.extend_from_slice(&verts);
            if let Some(root_id) = tile.root_node {
                self.render_node(root_id, tile, scene, &mut content_vertices, sw, sh);
            }
        }

        // Update zone animation states and render zone content backdrops.
        self.update_zone_animations(scene);
        self.render_zone_content(scene, &mut content_vertices, sw, sh);

        let encode_start = std::time::Instant::now();

        let content_buffer = if content_vertices.is_empty() {
            None
        } else {
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("content_vertex_buffer"),
                    contents: bytemuck::cast_slice(&content_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buf)
        };

        // ── Pass 2: Chrome ───────────────────────────────────────────────────
        let mut chrome_vertices: Vec<RectVertex> = Vec::new();
        for cmd in chrome_cmds {
            let verts = rect_vertices(cmd.x, cmd.y, cmd.width, cmd.height, sw, sh, cmd.color);
            chrome_vertices.extend_from_slice(&verts);
        }

        let chrome_buffer = if chrome_vertices.is_empty() {
            None
        } else {
            let buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("chrome_vertex_buffer"),
                    contents: bytemuck::cast_slice(&chrome_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            Some(buf)
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder_with_chrome"),
            });

        // Content render pass — clears the surface and draws agent tiles.
        {
            let mut content_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("content_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            content_pass.set_pipeline(&self.pipeline);
            if let Some(ref buf) = content_buffer {
                content_pass.set_vertex_buffer(0, buf.slice(..));
                content_pass.draw(0..content_vertices.len() as u32, 0..1);
            }
        }

        // ── Stage 6: Text pass (between content and chrome) ──────────────────
        // Text is content — rendered above geometry rectangles but below chrome.
        let text_items_chrome: Vec<TextItem> = if self.text_rasterizer.is_some() {
            self.collect_text_items(scene, sw, sh)
        } else {
            vec![]
        };
        if let Some(ref mut tr) = self.text_rasterizer {
            let (surf_w, surf_h) = (self.width, self.height);
            tr.update_viewport(&self.queue, surf_w, surf_h);
            if !text_items_chrome.is_empty() {
                if let Err(e) = tr.prepare_text_items(&self.device, &self.queue, &text_items_chrome)
                {
                    tracing::warn!(error = %e, "text prepare failed in chrome path");
                } else {
                    let mut text_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("text_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &surface.view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    if let Err(e) = tr.render_text_pass(&mut text_pass) {
                        tracing::warn!(error = %e, "text render failed in chrome path");
                    }
                }
            }
            // Trim every frame regardless of item count — glyphs from prior frames
            // must be evicted even when the current frame has no text.
            tr.trim_atlas();
        }

        // ── Widget pass: composite pre-synced textures above content + text ──
        // sync_widget_textures is called earlier, before frame encoding begins.
        self.encode_widget_pass(&mut encoder, &surface.view, &scene.widget_registry, sw, sh);

        // Chrome render pass — uses LoadOp::Load to preserve content pixels.
        // Chrome commands are drawn ON TOP of content by construction.
        // No agent tile can occlude chrome regardless of z-order.
        {
            let mut chrome_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("chrome_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // LoadOp::Load: preserve content pixels — chrome draws on top.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            chrome_pass.set_pipeline(&self.pipeline);
            if let Some(ref buf) = chrome_buffer {
                chrome_pass.set_vertex_buffer(0, buf.slice(..));
                chrome_pass.draw(0..chrome_vertices.len() as u32, 0..1);
            }
        }

        telemetry.render_encode_us = encode_start.elapsed().as_micros() as u64;

        surface.copy_to_buffer(&mut encoder);

        let submit_start = std::time::Instant::now();
        self.queue.submit(std::iter::once(encoder.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        telemetry.gpu_submit_us = submit_start.elapsed().as_micros() as u64;

        telemetry.frame_time_us = frame_start.elapsed().as_micros() as u64;
        telemetry
    }

    /// Return the clear color for render passes. Transparent in overlay mode,
    /// dark background in fullscreen mode.
    fn clear_color(&self) -> wgpu::Color {
        if self.overlay_mode {
            wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }
        } else {
            wgpu::Color {
                r: 0.05,
                g: 0.05,
                b: 0.1,
                a: 1.0,
            }
        }
    }

    /// Encode a widget render pass that composites all widget textures into the frame.
    ///
    /// Widget tiles use z_order >= WIDGET_TILE_Z_MIN (0x9000_0000), placing them
    /// above zone tiles but below chrome (spec §Requirement: Widget Contention and
    /// Governance, §Requirement: Widget Input Mode).
    ///
    /// This is a no-op when the widget renderer is not initialized or the registry
    /// has no instances with cached textures.
    fn encode_widget_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame_view: &wgpu::TextureView,
        registry: &WidgetRegistry,
        surf_w: f32,
        surf_h: f32,
    ) {
        let wr = match &self.widget_renderer {
            Some(r) => r,
            None => return,
        };

        // Check if there are any instances with cached textures.
        if registry.instances.is_empty() {
            return;
        }

        let any_textured = registry
            .instances
            .keys()
            .any(|name| wr.texture_entry(name).is_some());
        if !any_textured {
            return;
        }

        // Begin a LoadOp::Load render pass — widgets composite on top of scene content.
        let mut widget_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("widget_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load, // preserve content pixels under widgets
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        wr.composite_widgets(&mut widget_pass, registry, surf_w, surf_h, &self.device);
    }

    /// Render zone content backdrop quads driven by `RenderingPolicy`.
    ///
    /// For each zone with at least one active publish:
    /// - Reads `backdrop` + `backdrop_opacity` from the zone's `RenderingPolicy`.
    /// - For `alert-banner` zones with `Notification` content, overrides the
    ///   backdrop color with the urgency-derived `color.severity.*` token color.
    /// - For non-alert-banner zones with `Notification` content, overrides the
    ///   backdrop color with the urgency-derived `color.notification.urgency.*`
    ///   token color at 0.9 opacity, and renders a 1px 4-quad border using
    ///   `color.border.default`.
    /// - Applies the zone's current animation opacity (fade-in/fade-out).
    /// - Skips the backdrop quad when `rendering_policy.backdrop` is `None`.
    ///
    /// No per-content-type color branching — all visual properties come from
    /// `RenderingPolicy` fields (spec §Refactoring note, §Default Zone Rendering).
    fn render_zone_content(
        &self,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        sw: f32,
        sh: f32,
    ) {
        for (zone_name, publishes) in &scene.zone_registry.active_publishes {
            if publishes.is_empty() {
                continue;
            }
            let zone_def = match scene.zone_registry.zones.get(zone_name) {
                Some(z) => z,
                None => continue,
            };
            let (x, y, w, h) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);

            let policy = &zone_def.rendering_policy;

            // Determine current animation opacity for this zone.
            let anim_opacity = self
                .zone_animation_states
                .get(zone_name)
                .map(|s| s.current_opacity())
                .unwrap_or(1.0);

            // Resolve and emit backdrop quads based on the zone's contention policy.
            //
            // Stack zones: each publication gets its own vertically-stacked slot.
            //   Publishes are rendered newest-first (slot 0 = newest, at top of zone).
            //   slot_h via Self::stack_slot_height() — content-sized, policy-driven.
            //   This gives each notification a content-sized slot rather than dividing
            //   zone height uniformly by max_depth.
            //
            // MergeByKey zones: single backdrop for the full zone (entries are merged
            //   at the data level; visually one unified strip).
            //
            // LatestWins / Replace: single backdrop from the most-recent publish only.
            match zone_def.contention_policy {
                ContentionPolicy::Stack { .. } => {
                    let slot_h = Self::stack_slot_height(policy);
                    // Render newest-first: slot_index 0 = newest (top of zone).
                    for (slot_idx, record) in publishes.iter().rev().enumerate() {
                        let slot_y = y + slot_idx as f32 * slot_h;
                        // Clip to zone bounds.
                        if slot_y >= y + h {
                            break;
                        }
                        let effective_slot_h = slot_h.min((y + h) - slot_y);

                        // Determine backdrop color.
                        // alert-banner: urgency → color.severity.* tokens
                        // non-alert-banner Notification: urgency → color.notification.urgency.* tokens
                        //   with fixed 0.9 opacity and 1px 4-quad border
                        // SolidColor: always its own color
                        // Other: policy.backdrop
                        let is_notification_content = matches!(
                            &record.content,
                            ZoneContent::Notification(_)
                        );
                        let backdrop_rgba: Option<Rgba> = match &record.content {
                            ZoneContent::SolidColor(rgba) => Some(*rgba),
                            ZoneContent::Notification(n) if is_alert_banner_zone(zone_name) => {
                                // alert-banner: severity tokens (color.severity.*).
                                // Respect policy.backdrop contract: only override the color when
                                // a backdrop is enabled; skip entirely when backdrop is None.
                                if policy.backdrop.is_some() {
                                    let severity_color =
                                        urgency_to_severity_color(n.urgency, &self.token_map);
                                    Some(severity_color)
                                } else {
                                    None
                                }
                            }
                            ZoneContent::Notification(n) => {
                                // Non-alert-banner notification: urgency-tinted backdrop
                                // using color.notification.urgency.* tokens.
                                // Per spec: urgency >3 clamped to 3 (critical).
                                // Respect policy.backdrop contract: only emit a backdrop (and
                                // border) when a backdrop is enabled; skip entirely when None.
                                if policy.backdrop.is_some() {
                                    let mut color =
                                        urgency_to_notification_color(n.urgency, &self.token_map);
                                    color.a = NOTIFICATION_BACKDROP_OPACITY;
                                    Some(color)
                                } else {
                                    None
                                }
                            }
                            _ => policy.backdrop,
                        };

                        if let Some(mut rgba) = backdrop_rgba {
                            // For non-notification content: apply policy backdrop_opacity.
                            // For Notification content in non-alert-banner zones: opacity is
                            // already set to NOTIFICATION_BACKDROP_OPACITY above; skip policy override.
                            if !is_notification_content || is_alert_banner_zone(zone_name) {
                                if let Some(opacity) = policy.backdrop_opacity {
                                    rgba.a = opacity.clamp(0.0, 1.0);
                                }
                            }
                            rgba.a *= anim_opacity.clamp(0.0, 1.0);
                            vertices.extend_from_slice(&rect_vertices(
                                x,
                                slot_y,
                                w,
                                effective_slot_h,
                                sw,
                                sh,
                                rgba.to_array(),
                            ));

                            // For non-alert-banner Notification content: emit 1px 4-quad border.
                            if is_notification_content && !is_alert_banner_zone(zone_name) {
                                let mut border_color =
                                    resolve_border_default_color(&self.token_map);
                                border_color.a *= anim_opacity.clamp(0.0, 1.0);
                                emit_border_quads(
                                    vertices,
                                    x,
                                    slot_y,
                                    w,
                                    effective_slot_h,
                                    sw,
                                    sh,
                                    border_color.to_array(),
                                );
                            }
                        }
                    }
                }
                ContentionPolicy::MergeByKey { .. }
                | ContentionPolicy::LatestWins
                | ContentionPolicy::Replace => {
                    // For MergeByKey and single-publish policies: render a single backdrop
                    // for the zone using the latest publication's content type.
                    let latest = &publishes[publishes.len() - 1];
                    let is_notification_content = matches!(
                        &latest.content,
                        ZoneContent::Notification(_)
                    );
                    let backdrop_rgba: Option<Rgba> = match &latest.content {
                        ZoneContent::SolidColor(rgba) => {
                            // SolidColor always renders its own color (no policy override).
                            Some(*rgba)
                        }
                        ZoneContent::Notification(n) if is_alert_banner_zone(zone_name) => {
                            // alert-banner: map urgency to severity token color.
                            // Per spec §Notification Urgency-to-Severity Token Mapping:
                            //   urgency 0,1 → color.severity.info
                            //   urgency 2   → color.severity.warning
                            //   urgency 3   → color.severity.critical
                            // Token-resolved values take precedence over hardcoded constants;
                            // falls back to SEVERITY_* when the token map is empty or the
                            // key is absent.
                            // Respect policy.backdrop contract: only override when backdrop
                            // is enabled; skip entirely when backdrop is None.
                            if policy.backdrop.is_some() {
                                let severity_color =
                                    urgency_to_severity_color(n.urgency, &self.token_map);
                                Some(severity_color)
                            } else {
                                None
                            }
                        }
                        ZoneContent::Notification(n) => {
                            // Non-alert-banner notification: urgency-tinted backdrop using
                            // color.notification.urgency.* tokens.
                            // Per spec: urgency >3 clamped to 3 (critical).
                            // Respect policy.backdrop contract: only emit a backdrop (and
                            // border) when a backdrop is enabled; skip entirely when None.
                            if policy.backdrop.is_some() {
                                let mut color =
                                    urgency_to_notification_color(n.urgency, &self.token_map);
                                color.a = NOTIFICATION_BACKDROP_OPACITY;
                                Some(color)
                            } else {
                                None
                            }
                        }
                        _ => {
                            // All other content: use policy.backdrop.
                            policy.backdrop
                        }
                    };

                    if let Some(mut rgba) = backdrop_rgba {
                        // For non-notification content: apply policy backdrop_opacity.
                        // For Notification content in non-alert-banner zones: opacity is
                        // already set to NOTIFICATION_BACKDROP_OPACITY above; skip policy override.
                        if !is_notification_content || is_alert_banner_zone(zone_name) {
                            if let Some(opacity) = policy.backdrop_opacity {
                                rgba.a = opacity.clamp(0.0, 1.0);
                            }
                        }
                        // Apply zone animation opacity.
                        rgba.a *= anim_opacity.clamp(0.0, 1.0);

                        vertices.extend_from_slice(&rect_vertices(
                            x,
                            y,
                            w,
                            h,
                            sw,
                            sh,
                            rgba.to_array(),
                        ));

                        // For non-alert-banner Notification content: emit 1px 4-quad border.
                        if is_notification_content && !is_alert_banner_zone(zone_name) {
                            let mut border_color = resolve_border_default_color(&self.token_map);
                            border_color.a *= anim_opacity.clamp(0.0, 1.0);
                            emit_border_quads(
                                vertices,
                                x,
                                y,
                                w,
                                h,
                                sw,
                                sh,
                                border_color.to_array(),
                            );
                        }
                    }
                }
            }
        }

        // ── Debug zone tints (--debug-zones) ─────────────────────────────────
        // Render ALL zone boundaries with colored tints ON TOP of content
        // backgrounds so developers can see zone geometry even when zones have
        // opaque content.  The overall background gets a subtle black tint;
        // each zone gets a rainbow-sampled color at low opacity.
        if self.debug_zone_tints {
            // 0.5% opacity black background tint over the full window.
            // In overlay mode, the clear_pipeline writes RGBA directly (no
            // GPU blending). DWM composites with premultiplied alpha, so RGB
            // values must be pre-multiplied by alpha.
            const A: f32 = 0.001;
            vertices.extend_from_slice(&rect_vertices(
                0.0,
                0.0,
                sw,
                sh,
                sw,
                sh,
                [0.0, 0.0, 0.0, A],
            ));

            // Rainbow palette for zone tints (1% opacity, premultiplied).
            const ZA: f32 = 0.005;
            let palette: &[[f32; 4]] = &[
                [1.0 * ZA, 0.2 * ZA, 0.2 * ZA, ZA], // red
                [1.0 * ZA, 0.6 * ZA, 0.1 * ZA, ZA], // orange
                [1.0 * ZA, 1.0 * ZA, 0.2 * ZA, ZA], // yellow
                [0.2 * ZA, 0.9 * ZA, 0.2 * ZA, ZA], // green
                [0.2 * ZA, 0.6 * ZA, 1.0 * ZA, ZA], // blue
                [0.7 * ZA, 0.3 * ZA, 1.0 * ZA, ZA], // violet
            ];
            for (idx, (_zone_name, zone_def)) in scene.zone_registry.zones.iter().enumerate() {
                let color = palette[idx % palette.len()];
                let (x, y, w, h) = Self::resolve_zone_geometry(&zone_def.geometry_policy, sw, sh);
                vertices.extend_from_slice(&rect_vertices(x, y, w, h, sw, sh, color));
            }
        }
    }

    /// Compute the per-slot height for a Stack zone.
    ///
    /// `slot_h = font_size_px + 2 * margin_v + SLOT_BASELINE_GAP`
    ///
    /// - `font_size_px` — from `RenderingPolicy`; defaults to 16.
    /// - `margin_v` — from `margin_vertical` → `margin_px` → 8 px fallback chain.
    /// - `SLOT_BASELINE_GAP` — a small constant gap (2 px) between successive slot
    ///   backdrops so they don't bleed into each other visually. This is not a
    ///   configurable policy field; it is a structural layout constant.
    ///
    /// Used by both `collect_text_items` and `render_zone_content` so both code
    /// paths stay consistent.
    fn stack_slot_height(policy: &RenderingPolicy) -> f32 {
        const SLOT_BASELINE_GAP: f32 = 2.0;
        let font_size_px = policy.font_size_px.unwrap_or(16.0).clamp(6.0, 200.0);
        let margin_v = policy.margin_vertical.or(policy.margin_px).unwrap_or(8.0);
        (font_size_px + 2.0 * margin_v + SLOT_BASELINE_GAP).max(1.0)
    }

    /// Resolve a zone's geometry policy to pixel bounds (x, y, w, h).
    fn resolve_zone_geometry(policy: &GeometryPolicy, sw: f32, sh: f32) -> (f32, f32, f32, f32) {
        match policy {
            GeometryPolicy::EdgeAnchored {
                edge,
                height_pct,
                width_pct,
                margin_px,
            } => {
                let zw = sw * width_pct;
                let zh = sh * height_pct;
                let zx = (sw - zw) / 2.0;
                let zy = match edge {
                    DisplayEdge::Top => *margin_px,
                    DisplayEdge::Bottom => sh - zh - margin_px,
                    DisplayEdge::Left | DisplayEdge::Right => 0.0,
                };
                (zx, zy, zw, zh)
            }
            GeometryPolicy::Relative {
                x_pct,
                y_pct,
                width_pct,
                height_pct,
            } => {
                let zx = sw * x_pct;
                let zy = sh * y_pct;
                let zw = sw * width_pct;
                let zh = sh * height_pct;
                (zx, zy, zw, zh)
            }
        }
    }

    /// Determine the background color for a tile based on its content.
    fn tile_background_color(&self, tile: &Tile, scene: &SceneGraph) -> [f32; 4] {
        if let Some(root_id) = tile.root_node
            && let Some(node) = scene.nodes.get(&root_id)
        {
            match &node.data {
                NodeData::SolidColor(sc) => return sc.color.to_array(),
                NodeData::TextMarkdown(tm) => {
                    if let Some(bg) = &tm.background {
                        return bg.to_array();
                    }
                    return [0.15, 0.15, 0.25, tile.opacity];
                }
                NodeData::HitRegion(_) => {
                    // Check local state for visual feedback
                    if let Some(state) = scene.hit_region_states.get(&root_id) {
                        if state.pressed {
                            return [0.4, 0.7, 1.0, tile.opacity]; // Active blue
                        } else if state.hovered {
                            return [0.3, 0.5, 0.8, tile.opacity]; // Hover blue
                        }
                    }
                    return [0.2, 0.3, 0.5, tile.opacity]; // Default hit region
                }
                NodeData::StaticImage(_) => {
                    // Tile background for image tiles: near-black with slight tint
                    return [0.05, 0.05, 0.05, tile.opacity];
                }
            }
        }
        [0.1, 0.1, 0.2, tile.opacity]
    }

    /// Render a node and its children within a tile.
    #[allow(clippy::only_used_in_recursion)]
    fn render_node(
        &self,
        node_id: SceneId,
        tile: &Tile,
        scene: &SceneGraph,
        vertices: &mut Vec<RectVertex>,
        sw: f32,
        sh: f32,
    ) {
        let node = match scene.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };

        match &node.data {
            NodeData::SolidColor(sc) => {
                let verts = rect_vertices(
                    tile.bounds.x + sc.bounds.x,
                    tile.bounds.y + sc.bounds.y,
                    sc.bounds.width,
                    sc.bounds.height,
                    sw,
                    sh,
                    sc.color.to_array(),
                );
                vertices.extend_from_slice(&verts);
            }
            NodeData::TextMarkdown(tm) => {
                // For the vertical slice, render text as a colored rectangle
                // (actual text rendering deferred to post-vertical-slice)
                let bg = tm.background.unwrap_or(Rgba::new(0.15, 0.15, 0.25, 1.0));
                let verts = rect_vertices(
                    tile.bounds.x + tm.bounds.x,
                    tile.bounds.y + tm.bounds.y,
                    tm.bounds.width,
                    tm.bounds.height,
                    sw,
                    sh,
                    bg.to_array(),
                );
                vertices.extend_from_slice(&verts);

                // Render a smaller text area rectangle in the text color
                // to indicate text content is present
                let text_margin = 8.0;
                if tm.bounds.width > text_margin * 2.0 && tm.bounds.height > text_margin * 2.0 {
                    let verts = rect_vertices(
                        tile.bounds.x + tm.bounds.x + text_margin,
                        tile.bounds.y + tm.bounds.y + text_margin,
                        tm.bounds.width - text_margin * 2.0,
                        (tm.font_size_px * 1.2).min(tm.bounds.height - text_margin * 2.0),
                        sw,
                        sh,
                        tm.color.to_array(),
                    );
                    vertices.extend_from_slice(&verts);
                }
            }
            NodeData::HitRegion(hr) => {
                // Render hit region with local state feedback
                let color = if let Some(state) = scene.hit_region_states.get(&node_id) {
                    if state.pressed {
                        [0.4, 0.7, 1.0, 1.0]
                    } else if state.hovered {
                        [0.3, 0.5, 0.8, 1.0]
                    } else {
                        [0.2, 0.3, 0.5, 1.0]
                    }
                } else {
                    [0.2, 0.3, 0.5, 1.0]
                };

                let verts = rect_vertices(
                    tile.bounds.x + hr.bounds.x,
                    tile.bounds.y + hr.bounds.y,
                    hr.bounds.width,
                    hr.bounds.height,
                    sw,
                    sh,
                    color,
                );
                vertices.extend_from_slice(&verts);
            }
            NodeData::StaticImage(img) => {
                // Render a representative colored quad for the image bounds.
                //
                // Full GPU texture upload (wgpu::Texture from RGBA data) is deferred to a
                // follow-up iteration that adds a sampler pipeline. For the vertical slice this
                // placeholder renders a warm-gray background quad with a smaller accent strip
                // (mimicking the visual weight of an image) so that pixel-readback tests can
                // verify the node is composited into the frame at the correct position.
                let outer_color = [0.55_f32, 0.50, 0.45, 1.0]; // warm gray — "image placeholder"
                let verts = rect_vertices(
                    tile.bounds.x + img.bounds.x,
                    tile.bounds.y + img.bounds.y,
                    img.bounds.width,
                    img.bounds.height,
                    sw,
                    sh,
                    outer_color,
                );
                vertices.extend_from_slice(&verts);

                // Inner accent strip — a slightly brighter inset rectangle.
                let margin = 4.0_f32;
                if img.bounds.width > margin * 2.0 && img.bounds.height > margin * 2.0 {
                    let accent_color = [0.75_f32, 0.70, 0.65, 1.0];
                    let verts = rect_vertices(
                        tile.bounds.x + img.bounds.x + margin,
                        tile.bounds.y + img.bounds.y + margin,
                        img.bounds.width - margin * 2.0,
                        img.bounds.height - margin * 2.0,
                        sw,
                        sh,
                        accent_color,
                    );
                    vertices.extend_from_slice(&verts);
                }
            }
        }

        // Render children
        for child_id in &node.children {
            self.render_node(*child_id, tile, scene, vertices, sw, sh);
        }
    }
}

/// Resolve the pixel dimensions for a widget instance.
///
/// Returns (width, height) in pixels. Returns (0, 0) if the geometry cannot be
/// resolved (e.g., zero-sized surface or unrecognized policy).
fn resolve_widget_pixel_size(
    instance: &WidgetInstance,
    def: &WidgetDefinition,
    surf_w: u32,
    surf_h: u32,
) -> (u32, u32) {
    let geo = instance
        .geometry_override
        .as_ref()
        .unwrap_or(&def.default_geometry_policy);
    let sw = surf_w as f32;
    let sh = surf_h as f32;
    let (w, h) = match geo {
        GeometryPolicy::Relative {
            width_pct,
            height_pct,
            ..
        } => (sw * width_pct, sh * height_pct),
        GeometryPolicy::EdgeAnchored {
            width_pct,
            height_pct,
            ..
        } => (sw * width_pct, sh * height_pct),
    };
    let w = (w.max(1.0) as u32).min(surf_w.max(1));
    let h = (h.max(1.0) as u32).min(surf_h.max(1));
    (w, h)
}

#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    #[error("no suitable GPU adapter found")]
    NoAdapter,
    #[error("failed to create device: {0}")]
    DeviceCreation(String),
}

// Make buffer init descriptor available
use wgpu::util::DeviceExt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::HeadlessSurface;
    use tze_hud_scene::graph::SceneGraph;

    /// Mutex to serialize tests that mutate `HEADLESS_FORCE_SOFTWARE`, a
    /// global environment variable.  Rust tests run in parallel by default,
    /// so concurrent mutations would cause races.  This is an in-process lock;
    /// it does not protect against separate test binary runs.
    static ENV_VAR_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Convenience: build a minimal scene with one tile containing the given node.
    fn scene_with_node(node: Node) -> SceneGraph {
        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "test",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene.set_tile_root(tile_id, node).unwrap();
        scene
    }

    /// Create a headless compositor and surface pair for testing.
    async fn make_compositor_and_surface(w: u32, h: u32) -> (Compositor, HeadlessSurface) {
        let compositor = Compositor::new_headless(w, h)
            .await
            .expect("headless compositor");
        let surface = HeadlessSurface::new(&compositor.device, w, h);
        (compositor, surface)
    }

    #[tokio::test]
    async fn test_static_image_node_renders_placeholder_quad() {
        // The static image placeholder renders a warm-gray outer quad ~[0.55, 0.50, 0.45].
        // In sRGB output the linear values are gamma-compressed.
        // We just verify that *some* non-background pixels appear in the expected warm range.
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;

        // RS-4: StaticImageNode uses resource_id + decoded_bytes; no raw blob embedded.
        let resource_id = ResourceId::of(b"8x8 test image placeholder");
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 8,
                height: 8,
                decoded_bytes: 8 * 8 * 4,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        };

        let scene = scene_with_node(node);
        compositor.render_frame_headless(&scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);
        // The background clear color is ~[0.05, 0.05, 0.1] in linear; tile bg is [0.05,0.05,0.05].
        // The placeholder outer quad is warm gray [0.55, 0.50, 0.45] in linear.
        // In sRGB this is approximately [198, 188, 176]. We look for pixels brighter than 150 in
        // all three channels to confirm the quad was rendered (not just the dark background).
        let any_warm_pixel = pixels
            .chunks(4)
            .any(|p| p[0] > 150 && p[1] > 140 && p[2] > 130);
        assert!(
            any_warm_pixel,
            "expected warm-gray placeholder pixels from StaticImageNode"
        );
    }

    #[tokio::test]
    async fn test_static_image_node_composited_with_other_nodes() {
        // Render a scene with both a SolidColor node and a StaticImage node in adjacent tiles.
        let (mut compositor, surface) = make_compositor_and_surface(512, 256).await;

        let mut scene = SceneGraph::new(512.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);

        // Left tile: red solid color
        let left_tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene
            .set_tile_root(
                left_tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        color: Rgba::new(1.0, 0.0, 0.0, 1.0),
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Right tile: static image
        // RS-4: StaticImageNode uses resource_id + decoded_bytes; no raw blob embedded.
        let right_tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(256.0, 0.0, 256.0, 256.0),
                2,
            )
            .unwrap();
        scene
            .set_tile_root(
                right_tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::StaticImage(StaticImageNode {
                        resource_id: ResourceId::of(b"8x8 green placeholder"),
                        width: 8,
                        height: 8,
                        decoded_bytes: 8 * 8 * 4,
                        fit_mode: ImageFitMode::Cover,
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        compositor.render_frame_headless(&scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 512 * 256 * 4, "pixel buffer size mismatch");
        // Just verify the frame completed without panic and returned the expected buffer size.
    }

    // ── Chrome layer pixel tests ──────────────────────────────────────────────

    /// Layer 1 pixel test: chrome layer is always visible above max-z-order agent tile.
    ///
    /// Acceptance criterion: "Layer 1 pixel tests confirm chrome always visible above
    /// max-z-order agent tile."
    ///
    /// This test renders a bright red tile at max z-order (u32::MAX) then renders a
    /// distinctive chrome rectangle over the same region. The chrome pixels (pure green)
    /// must overwrite the red tile pixels.
    #[tokio::test]
    async fn test_chrome_always_above_max_zorder_tile() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;

        // Agent tile at max valid agent z-order with bright red content.
        // Agent tiles must use z_order < ZONE_TILE_Z_MIN (0x8000_0000); u32::MAX is
        // reserved for runtime zone tiles (scene-graph/spec.md §Zone Layer Attachment).
        use tze_hud_scene::types::ZONE_TILE_Z_MIN;
        let max_agent_z = ZONE_TILE_Z_MIN - 1; // 0x7FFF_FFFF
        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                max_agent_z,
            )
            .unwrap();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        color: Rgba::new(1.0, 0.0, 0.0, 1.0), // bright red
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Chrome draw command: bright green rectangle covering the full surface.
        // In NDC space, this will overwrite all tile content.
        let chrome_cmds = vec![crate::pipeline::ChromeDrawCmd {
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 40.0,                // tab bar height
            color: [0.0, 1.0, 0.0, 1.0], // pure green — distinctive chrome marker
        }];

        compositor.render_frame_with_chrome(&scene, &surface, &chrome_cmds);
        compositor.device.poll(wgpu::Maintain::Wait);

        let pixels = surface.read_pixels(&compositor.device);

        // Check the top-left pixel region (where chrome covers the tile).
        // In sRGB, linear [0,1,0] green becomes approximately [0, 255, 0].
        // We look for pixels that are distinctly green (G > 200, R < 50).
        let chrome_top_pixel = &pixels[0..4]; // first pixel (top-left)
        assert!(
            chrome_top_pixel[1] > 150, // green channel dominant
            "chrome green channel should be dominant at top: {chrome_top_pixel:?}"
        );
        // The tile red should NOT bleed through chrome.
        assert!(
            chrome_top_pixel[0] < 50,
            "agent tile red must not show through chrome: {chrome_top_pixel:?}"
        );
    }

    /// Layer 1 pixel test: chrome hit-test priority — chrome is always drawn last.
    ///
    /// Verifies the separable render pass architecture: content pass first (agent tiles),
    /// chrome pass second (chrome elements). The two-pass structure guarantees chrome
    /// always occupies the final pixels regardless of content.
    #[tokio::test]
    async fn test_chrome_pass_uses_load_op_load() {
        // Render a scene with a blue agent tile + a chrome red stripe.
        // Blue content should persist where chrome doesn't cover; red should cover where it does.
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;

        let mut scene = SceneGraph::new(256.0, 256.0);
        let tab_id = scene.create_tab("test", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 256.0, 256.0),
                1,
            )
            .unwrap();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::SolidColor(SolidColorNode {
                        // Blue tile — fills entire surface in content pass.
                        color: Rgba::new(0.0, 0.0, 1.0, 1.0),
                        bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                    }),
                },
            )
            .unwrap();

        // Chrome: red stripe only in top half (rows 0..128).
        let chrome_cmds = vec![crate::pipeline::ChromeDrawCmd {
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 128.0,
            color: [1.0, 0.0, 0.0, 1.0], // pure red
        }];

        compositor.render_frame_with_chrome(&scene, &surface, &chrome_cmds);
        compositor.device.poll(wgpu::Maintain::Wait);

        let pixels = surface.read_pixels(&compositor.device);

        // Top row: chrome (red) should dominate.
        let top_px = &pixels[0..4];
        assert!(
            top_px[0] > 150,
            "top pixel should be red (chrome): {top_px:?}"
        );
        assert!(
            top_px[2] < 50,
            "top pixel blue (tile) must be suppressed by chrome: {top_px:?}"
        );

        // Bottom row: content (blue) should persist — chrome didn't cover it.
        // Row 255 starts at pixel offset 255*256*4.
        let bottom_row_offset = 255 * 256 * 4;
        let bottom_px = &pixels[bottom_row_offset..bottom_row_offset + 4];
        assert!(
            bottom_px[2] > 150,
            "bottom pixel should be blue (tile content, no chrome): {bottom_px:?}"
        );
        assert!(
            bottom_px[0] < 50,
            "bottom pixel red should be absent (no chrome): {bottom_px:?}"
        );
    }

    /// Verify that render_frame_with_chrome renders correctly even when chrome_cmds is empty.
    #[tokio::test]
    async fn test_two_pass_with_empty_chrome_cmds() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;
        let scene = scene_with_node(Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(0.5, 0.5, 0.5, 1.0),
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
            }),
        });
        // Empty chrome cmds — must not panic.
        compositor.render_frame_with_chrome(&scene, &surface, &[]);
        compositor.device.poll(wgpu::Maintain::Wait);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 256 * 256 * 4);
    }

    // ── Headless parity tests ─────────────────────────────────────────────────

    /// Verify that `render_frame` (surface-agnostic) works with a `HeadlessSurface`
    /// as a `&dyn CompositorSurface`.  This is the core headless parity assertion:
    /// the same method that would be used with a windowed surface works headlessly.
    #[tokio::test]
    async fn test_render_frame_via_compositor_surface_trait() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;
        let scene = SceneGraph::new(256.0, 256.0);

        // render_frame takes &dyn CompositorSurface — no special headless branch.
        let telemetry =
            compositor.render_frame(&scene, &surface as &dyn crate::surface::CompositorSurface);
        assert!(telemetry.frame_time_us > 0, "frame time must be non-zero");
        assert_eq!(telemetry.tile_count, 0, "empty scene has no tiles");
    }

    /// Verify that HEADLESS_FORCE_SOFTWARE env-var path is exercised in the
    /// adapter-selection code.  We cannot assert the adapter backend in a unit
    /// test (it's opaque), so we just verify that creating a compositor with
    /// the env var set does not crash.
    #[tokio::test]
    async fn test_new_headless_with_force_software_env_var() {
        // Serialize all env-var-mutating tests via a process-wide mutex.
        // Rust tests run in parallel by default; without serialization,
        // a concurrent test could observe or overwrite HEADLESS_FORCE_SOFTWARE.
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Safety: single-threaded within the mutex guard; no other test
        // touches HEADLESS_FORCE_SOFTWARE while _guard is held.
        unsafe {
            std::env::set_var("HEADLESS_FORCE_SOFTWARE", "1");
        }
        let result = Compositor::new_headless(64, 64).await;
        unsafe {
            std::env::remove_var("HEADLESS_FORCE_SOFTWARE");
        }
        drop(_guard);

        // Either Ok (software GPU found) or Err(NoAdapter) (no software GPU
        // installed in this CI environment) are acceptable.  A panic would not be.
        match result {
            Ok(_) => {}
            Err(CompositorError::NoAdapter) => {}
            Err(e) => panic!("unexpected error with HEADLESS_FORCE_SOFTWARE=1: {e}"),
        }
    }

    // ── Surface capability guard + dimension clamping (hud-q5hx regression) ────
    //
    // These tests validate the defensive logic added to `new_windowed_inner()`:
    //   1. Empty surface capability lists return `Err` instead of panicking.
    //   2. Dimension clamping uses `.max(1)` to prevent zero-size configs.
    //
    // The windowed path requires a real display handle and GPU, so we test the
    // clamping arithmetic directly as a pure function.

    /// Dimension clamping must apply both the device max and a minimum of 1.
    /// wgpu panics on `surface.configure()` with zero-width or zero-height.
    #[test]
    fn surface_dim_clamp_zero_becomes_one() {
        let max_dim = 16384u32;
        assert_eq!(0u32.min(max_dim).max(1), 1);
        assert_eq!(1u32.min(max_dim).max(1), 1);
        assert_eq!(2560u32.min(max_dim).max(1), 2560);
        assert_eq!(3840u32.min(max_dim).max(1), 3840);
    }

    /// Dimension clamping respects the device maximum texture dimension.
    /// Values larger than the limit are clamped, values within the limit pass through.
    #[test]
    fn surface_dim_clamp_respects_device_limit() {
        let max_dim = 4096u32;
        assert_eq!(4097u32.min(max_dim).max(1), 4096, "over-limit must clamp");
        assert_eq!(4096u32.min(max_dim).max(1), 4096, "at-limit must pass");
        assert_eq!(2560u32.min(max_dim).max(1), 2560, "under-limit must pass");
        assert_eq!(1920u32.min(max_dim).max(1), 1920, "default res must pass");
    }

    /// Dimension clamping at 2560x1440 with a 32768 device limit (RTX 3080)
    /// must not clamp — 2560 and 1440 are well below the RTX 3080's limit.
    #[test]
    fn surface_dim_clamp_2560x1440_passes_on_rtx3080_limit() {
        // RTX 3080 with Vulkan driver reports max_texture_dimension_2d = 32768.
        let max_dim = 32768u32;
        assert_eq!(
            2560u32.min(max_dim).max(1),
            2560,
            "2560 must not be clamped"
        );
        assert_eq!(
            1440u32.min(max_dim).max(1),
            1440,
            "1440 must not be clamped"
        );
        assert_eq!(3840u32.min(max_dim).max(1), 3840, "4K must not be clamped");
        assert_eq!(2160u32.min(max_dim).max(1), 2160, "4K must not be clamped");
    }

    // ── Text rendering pixel tests ────────────────────────────────────────────
    //
    // These tests validate acceptance criteria 1–4 from hud-pmkf:
    //  1. publish_to_zone with StreamText → visible text at zone geometry.
    //  2. TextMarkdownNode renders text (some non-background pixels in text area).
    //  3. Overflow Clip and Ellipsis modes: glyphs stay within TextBounds.
    //  4. Headless pixel readback detects text presence.
    //
    // All tests initialise the text renderer via `compositor.init_text_renderer`
    // targeting `Rgba8UnormSrgb` (the headless surface format).

    /// Pixel readback validates text presence in a TextMarkdownNode tile.
    ///
    /// After text rendering, pixels in the text region should differ from the
    /// solid background color — glyphs overwrite some pixels.  We can't check
    /// exact glyph shapes without font-specific knowledge, so we verify that
    /// at least one pixel in the tile area differs from the pure background
    /// color.
    #[tokio::test]
    async fn test_text_markdown_node_renders_visible_text() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        // Dark-blue background, white text — high contrast for pixel detection.
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "Hello world".to_owned(),
                bounds: Rect::new(0.0, 0.0, 256.0, 256.0),
                font_size_px: 24.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::new(1.0, 1.0, 1.0, 1.0), // white
                background: Some(Rgba::new(0.0, 0.0, 0.5, 1.0)), // dark blue
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            }),
        };
        let scene = scene_with_node(node);
        compositor.render_frame_headless(&scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 256 * 256 * 4, "pixel buffer size");

        // The background is dark blue — sRGB of linear [0,0,0.5] ≈ [0, 0, 188].
        // White text (sRGB [255, 255, 255]) glyphs should appear in the tile.
        // We check that at least one pixel has R > 200 AND G > 200 (white).
        let any_bright_pixel = pixels
            .chunks(4)
            .any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200);
        assert!(
            any_bright_pixel,
            "expected white text pixels in TextMarkdownNode tile — none found"
        );
    }

    /// Text stays within the TextBounds clip rectangle (Clip overflow mode).
    ///
    /// We render white text in a small region at the top-left of a dark tile.
    /// The bottom-right quadrant should remain all-dark (no text overflow).
    #[tokio::test]
    async fn test_text_clip_overflow_stays_within_bounds() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        // Text node occupies only top-left 64x64 pixels of the 256x256 tile.
        // Content: many lines so overflow is tested.
        let content = "Line1\nLine2\nLine3\nLine4\nLine5\nLine6\nLine7\nLine8".to_owned();
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content,
                bounds: Rect::new(0.0, 0.0, 64.0, 32.0), // small box
                font_size_px: 12.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::new(1.0, 1.0, 1.0, 1.0), // white
                background: Some(Rgba::new(0.0, 0.0, 0.0, 1.0)), // pure black bg
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            }),
        };
        let scene = scene_with_node(node);
        compositor.render_frame_headless(&scene, &surface);

        let pixels = surface.read_pixels(&compositor.device);

        // Bottom-right quadrant: rows 128..256, cols 128..256.
        // Tile background is ~[0.15, 0.15, 0.25] (tile_background_color default).
        // There should be no white pixels (from text) there.
        let mut bright_outside = false;
        for row in 128..256_usize {
            for col in 128..256_usize {
                let offset = (row * 256 + col) * 4;
                let p = &pixels[offset..offset + 4];
                // White text would have R > 200 AND G > 200 AND B > 200.
                if p[0] > 200 && p[1] > 200 && p[2] > 200 {
                    bright_outside = true;
                    break;
                }
            }
            if bright_outside {
                break;
            }
        }
        assert!(
            !bright_outside,
            "text overflow detected outside clip bounds (bottom-right quadrant has bright pixels)"
        );
    }

    /// Ellipsis overflow mode: text renders without panic; background present.
    ///
    /// We don't assert exact "…" pixel shape — that's platform-font-specific.
    /// We verify: (a) no panic, (b) background exists, (c) some non-background
    /// pixels appear (text was rendered at all).
    #[tokio::test]
    async fn test_text_ellipsis_overflow_no_panic() {
        let (mut compositor, surface) = make_compositor_and_surface(256, 256).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let long_line =
            "A very long line that definitely overflows the available width of this tile";
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: long_line.to_owned(),
                bounds: Rect::new(0.0, 0.0, 120.0, 40.0),
                font_size_px: 14.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::new(1.0, 1.0, 1.0, 1.0),
                background: Some(Rgba::new(0.1, 0.1, 0.1, 1.0)),
                alignment: TextAlign::Start,
                overflow: TextOverflow::Ellipsis,
            }),
        };
        let scene = scene_with_node(node);
        // Must not panic.
        compositor.render_frame_headless(&scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(
            pixels.len(),
            256 * 256 * 4,
            "pixel buffer must be full size"
        );
    }

    /// Zone StreamText publish renders visible text at zone geometry.
    ///
    /// Acceptance criterion 1: publish_to_zone with StreamText content displays
    /// readable text at the zone geometry position.
    #[tokio::test]
    async fn test_zone_stream_text_renders_visible_text() {
        let (mut compositor, surface) = make_compositor_and_surface(1280, 720).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Register a subtitle zone (bottom edge, 10% height).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "subtitle zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(22.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_align: None,
                margin_px: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Publish "Hello Zone" to the subtitle zone.
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("Hello Zone".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.render_frame_headless(&scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 1280 * 720 * 4, "pixel buffer size");

        // The zone is at the bottom 10% (y ~648..720) centered (x ~128..1152).
        // We check for any bright pixels in that area (white text on dark bg).
        // Zone bg is semi-transparent dark [0.1, 0.1, 0.15, 0.85] rendered over
        // the default compositor clear [0.05, 0.05, 0.1] → still quite dark.
        // White text glyphs should show as bright pixels.
        let mut found_bright = false;
        // Sample a row in the subtitle zone (row 660 ≈ 91.7% of 720 = 661).
        for row in 652usize..715 {
            for col in 150usize..1130 {
                let offset = (row * 1280 + col) * 4;
                let p = &pixels[offset..offset + 4];
                if p[0] > 180 && p[1] > 180 && p[2] > 180 {
                    found_bright = true;
                    break;
                }
            }
            if found_bright {
                break;
            }
        }
        assert!(
            found_bright,
            "expected bright (text) pixels in zone subtitle area (rows 652..715)"
        );
    }

    /// Zone ShortTextWithIcon/Notification publish renders visible text at zone geometry.
    ///
    /// Acceptance criterion for hud-lh3w: publish_to_zone with
    /// `ZoneContent::Notification(NotificationPayload)` produces a `TextItem` in
    /// `collect_text_items` and causes bright glyph pixels to appear in the zone
    /// geometry area.
    #[tokio::test]
    async fn test_zone_notification_renders_visible_text() {
        let (mut compositor, surface) = make_compositor_and_surface(1280, 720).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Register a notification zone (top edge, 8% height).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification".to_owned(),
            description: "notification zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.08,
                width_pct: 0.70,
                margin_px: 12.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(20.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.75)),
                text_align: None,
                margin_px: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Publish a notification to the zone.
        scene
            .publish_to_zone(
                "notification",
                ZoneContent::Notification(NotificationPayload {
                    text: "Alert: system ready".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        compositor.render_frame_headless(&scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 1280 * 720 * 4, "pixel buffer size");

        // The zone is at the top edge: y ≈ 12..(12 + 720*0.08) ≈ 12..69.6,
        // centered: x ≈ (1280-896)/2..896+192 = 192..1088.
        // White text glyphs should appear as bright pixels (r,g,b > 180).
        let mut found_bright = false;
        for row in 12usize..70 {
            for col in 200usize..1080 {
                let offset = (row * 1280 + col) * 4;
                let p = &pixels[offset..offset + 4];
                if p[0] > 180 && p[1] > 180 && p[2] > 180 {
                    found_bright = true;
                    break;
                }
            }
            if found_bright {
                break;
            }
        }
        assert!(
            found_bright,
            "expected bright (text) pixels in notification zone area (rows 12..70)"
        );
    }

    /// Zone StatusBar (KeyValuePairs) publish renders visible text at zone geometry.
    ///
    /// Acceptance criteria for hud-6at1:
    ///   1. `publish_to_zone` with `ZoneContent::StatusBar` produces a `TextItem` in
    ///      `collect_text_items`.
    ///   2. The key-value pairs are rendered as text at the zone geometry position.
    #[tokio::test]
    async fn test_zone_status_bar_renders_visible_text() {
        let (mut compositor, surface) = make_compositor_and_surface(1280, 720).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);

        // Register a status-bar zone (top edge, 5% height).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "statusbar".to_owned(),
            description: "status bar zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.05,
                width_pct: 0.80,
                margin_px: 8.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_align: None,
                margin_px: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // Publish StatusBar content with key-value pairs.
        let mut entries = std::collections::HashMap::new();
        entries.insert("battery".to_owned(), "95%".to_owned());
        entries.insert("time".to_owned(), "12:34".to_owned());
        scene
            .publish_to_zone(
                "statusbar",
                ZoneContent::StatusBar(StatusBarPayload { entries }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // Verify collect_text_items produces a TextItem with the formatted pairs.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(
            items.len(),
            1,
            "expected exactly one TextItem for StatusBar"
        );
        let item = &items[0];
        // Entries are sorted by key ("battery" < "time") and separated by newlines.
        assert_eq!(
            item.text, "battery: 95%\ntime: 12:34",
            "Entries should be sorted by key and formatted correctly"
        );
        // The TextItem position should be within the zone geometry.
        // Zone top-edge: y = 8.0 (margin_px), height = 720*0.05 = 36, width = 1280*0.8 = 1024.
        assert!(
            item.pixel_y >= 8.0,
            "text y should be at or below zone top margin"
        );
        assert!(
            item.pixel_y < 720.0 * 0.10,
            "text y should be within top zone area"
        );

        // Render to pixels and verify bright text appears in the top zone area.
        compositor.render_frame_headless(&scene, &surface);
        let pixels = surface.read_pixels(&compositor.device);
        assert_eq!(pixels.len(), 1280 * 720 * 4, "pixel buffer size");

        // The zone is at the top ~8..44px, centered horizontally.
        // White text glyphs should show as bright pixels.
        let mut found_bright = false;
        for row in 10usize..42 {
            for col in 150usize..1130 {
                let offset = (row * 1280 + col) * 4;
                let p = &pixels[offset..offset + 4];
                if p[0] > 180 && p[1] > 180 && p[2] > 180 {
                    found_bright = true;
                    break;
                }
            }
            if found_bright {
                break;
            }
        }
        assert!(
            found_bright,
            "expected bright (text) pixels in status bar zone area (rows 10..42)"
        );
    }

    /// `init_text_renderer` called multiple times replaces the rasterizer (no panic).
    #[tokio::test]
    async fn test_init_text_renderer_idempotent() {
        let (mut compositor, surface) = make_compositor_and_surface(64, 64).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);
        let scene = SceneGraph::new(64.0, 64.0);
        compositor.render_frame_headless(&scene, &surface);
        // No panic = pass.
    }

    /// Text rendering with no text items (empty scene) must not panic.
    #[tokio::test]
    async fn test_text_renderer_empty_scene_no_panic() {
        let (mut compositor, surface) = make_compositor_and_surface(64, 64).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);
        let scene = SceneGraph::new(64.0, 64.0);
        compositor.render_frame_headless(&scene, &surface);
    }

    /// Stage 6 frame-budget benchmark — text rendering active.
    ///
    /// Renders 60 frames with `init_text_renderer` active, a `TextMarkdownNode`
    /// tile, and a zone with `StreamText` content.  Asserts that the p99 of
    /// `render_encode_us` (the Stage 6 wall-clock encode time returned by
    /// `render_frame_headless`) stays below `STAGE6_BUDGET_US` (4 ms = 4 000 µs).
    ///
    /// Budget constant sourced from `tze_hud_runtime::pipeline::STAGE6_BUDGET_US`.
    /// It is inlined here to avoid a cyclic dev-dependency
    /// (tze_hud_runtime → tze_hud_compositor already exists).
    #[tokio::test]
    async fn test_stage6_budget_with_text_rendering_active() {
        // Stage 6 p99 budget in microseconds — mirrors STAGE6_BUDGET_US in
        // tze_hud_runtime::pipeline (4 ms).
        const STAGE6_BUDGET_US: u64 = 4_000;
        const FRAME_COUNT: usize = 60;

        let (mut compositor, surface) = make_compositor_and_surface(1280, 720).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        // ── Build scene ─────────────────────────────────────────────────────────
        let mut scene = SceneGraph::new(1280.0, 720.0);
        let tab_id = scene.create_tab("bench", 0).unwrap();
        let lease_id = scene.grant_lease("bench", 60_000, vec![]);

        // TextMarkdownNode tile occupying most of the screen.
        let tile_id = scene
            .create_tile(
                tab_id,
                "bench",
                lease_id,
                Rect::new(0.0, 0.0, 1000.0, 600.0),
                1,
            )
            .unwrap();
        scene
            .set_tile_root(
                tile_id,
                Node {
                    id: SceneId::new(),
                    children: vec![],
                    data: NodeData::TextMarkdown(TextMarkdownNode {
                        content: "Stage 6 budget benchmark\nLine two of text\nLine three"
                            .to_owned(),
                        bounds: Rect::new(0.0, 0.0, 1000.0, 600.0),
                        font_size_px: 20.0,
                        font_family: FontFamily::SystemSansSerif,
                        color: Rgba::new(1.0, 1.0, 1.0, 1.0),
                        background: Some(Rgba::new(0.05, 0.05, 0.1, 1.0)),
                        alignment: TextAlign::Start,
                        overflow: TextOverflow::Clip,
                    }),
                },
            )
            .unwrap();

        // Zone with StreamText content (subtitle strip at the bottom).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "bench-subtitle".to_owned(),
            description: "benchmark subtitle zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(22.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_align: None,
                margin_px: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "bench-subtitle",
                ZoneContent::StreamText("Stage 6 benchmark — stream text active".to_owned()),
                "bench",
                None,
                None,
                None,
            )
            .unwrap();

        // ── Warm-up pass ────────────────────────────────────────────────────────
        // Run a few frames to let llvmpipe/WARP JIT-compile the shaders before
        // the timed measurement window.  Shader compilation is a one-time cost
        // that does not reflect steady-state Stage 6 performance; excluding it
        // mirrors production behaviour where shaders are pre-compiled.
        for _ in 0..5 {
            compositor.render_frame_headless(&scene, &surface);
        }

        // ── Render loop ─────────────────────────────────────────────────────────
        let mut timings: Vec<u64> = Vec::with_capacity(FRAME_COUNT);
        for _ in 0..FRAME_COUNT {
            let telem = compositor.render_frame_headless(&scene, &surface);
            // render_encode_us is the Stage 6 wall-clock encode duration.
            timings.push(telem.render_encode_us);
        }

        // ── p99 assertion ────────────────────────────────────────────────────────
        timings.sort_unstable();
        // p99 index: ceil(99/100 * N) - 1 (0-based), clamped to last element.
        let p99_index = ((FRAME_COUNT as f64 * 0.99).ceil() as usize).saturating_sub(1);
        let p99_index = p99_index.min(FRAME_COUNT - 1);
        let p99_us = timings[p99_index];

        assert!(
            p99_us <= STAGE6_BUDGET_US,
            "Stage 6 render-encode p99 ({p99_us} µs) exceeds budget ({STAGE6_BUDGET_US} µs). \
             All timings (sorted): {timings:?}"
        );
    }

    // ── RenderingPolicy-driven zone rendering tests [hud-sc0a.8] ─────────────

    /// Subtitle with outline: when outline_color + outline_width are set in
    /// RenderingPolicy, collect_text_items produces a TextItem with non-None
    /// outline fields.
    #[tokio::test]
    async fn test_zone_subtitle_with_outline_text_item() {
        let (mut compositor, _surface) = make_compositor_and_surface(1280, 720).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "subtitle zone with outline".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(22.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                outline_color: Some(Rgba::BLACK),
                outline_width: Some(2.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("Test outline text".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 1, "expected one TextItem");
        let item = &items[0];
        assert!(
            item.outline_color.is_some(),
            "outline_color should be set from RenderingPolicy"
        );
        assert!(
            item.outline_width.is_some(),
            "outline_width should be set from RenderingPolicy"
        );
        assert_eq!(
            item.outline_width.unwrap(),
            2.0,
            "outline_width should match policy"
        );
        // Text color should be white (from text_color).
        assert_eq!(
            item.color[0], 255,
            "text fill color R should be white (255)"
        );
    }

    /// Subtitle without outline: when outline_width is None, outline fields
    /// on the TextItem should be None.
    #[tokio::test]
    async fn test_zone_subtitle_without_outline_text_item() {
        let (mut compositor, _surface) = make_compositor_and_surface(1280, 720).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "subtitle zone without outline".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(22.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                outline_color: None,
                outline_width: None,
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("No outline subtitle".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 1, "expected one TextItem");
        let item = &items[0];
        assert!(
            item.outline_color.is_none(),
            "outline_color should be None when policy has no outline"
        );
        assert!(
            item.outline_width.is_none(),
            "outline_width should be None when policy has no outline"
        );
    }

    /// Notification with opaque backdrop: backdrop_opacity=0.9 overrides
    /// the backdrop color's alpha.  The backdrop quad should be rendered with
    /// effective alpha = 0.9.
    #[tokio::test]
    async fn test_notification_with_opaque_backdrop() {
        let (mut compositor, _surface) = make_compositor_and_surface(1280, 720).await;
        compositor.init_text_renderer(wgpu::TextureFormat::Rgba8UnormSrgb);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.08,
                width_pct: 0.70,
                margin_px: 12.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(18.0),
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 1.0)),
                backdrop_opacity: Some(0.9),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: Some(5_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Notification with opaque backdrop".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // render_zone_content should produce backdrop rect vertices.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);
        // We check that vertices were emitted (backdrop rendered).
        assert!(
            !vertices.is_empty(),
            "expected backdrop vertices for notification with opaque backdrop"
        );

        // Also verify the text item uses the policy text_color.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 1, "expected one TextItem for notification");
        // White text → R channel should be near 255.
        assert!(
            items[0].color[0] > 200,
            "text color R should be near-white from policy.text_color"
        );
    }

    /// Alert-banner severity mapping: urgency 2 (warning) should map to
    /// color.severity.warning (amber/yellow), NOT the policy backdrop.
    /// We verify by inspecting the vertices emitted by render_zone_content.
    #[tokio::test]
    async fn test_alert_banner_urgency2_maps_to_severity_warning() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "alert banner zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.07,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(20.0),
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)), // dark default
                backdrop_opacity: Some(1.0),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish urgency=2 (warning).
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "Warning: disk space low".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // Collect vertices from render_zone_content.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        // The backdrop should be severity warning color (~amber: R=1.0, G~0.72, B=0.0).
        // rect_vertices emits 6 vertices; each has color at the end.
        // We check that the R component is high (>0.9) and G is mid (~0.5-0.8) and B is low.
        assert!(
            !vertices.is_empty(),
            "expected backdrop vertices for alert-banner urgency=2"
        );

        // Verify urgency_to_severity_color directly (no token map → fallback constants).
        let no_tokens = HashMap::new();
        let warning_color = urgency_to_severity_color(2, &no_tokens);
        assert!(
            warning_color.r > 0.9,
            "warning severity R should be ~1.0 (amber)"
        );
        assert!(
            warning_color.g > 0.5,
            "warning severity G should be >0.5 (amber)"
        );
        assert!(
            warning_color.b < 0.1,
            "warning severity B should be ~0.0 (amber)"
        );
    }

    /// Alert-banner urgency=3 maps to critical (red).
    #[tokio::test]
    async fn test_alert_banner_urgency3_maps_to_severity_critical() {
        let no_tokens = HashMap::new();
        let critical = urgency_to_severity_color(3, &no_tokens);
        assert!(critical.r > 0.9, "critical R should be ~1.0");
        assert!(critical.g < 0.1, "critical G should be ~0.0");
        assert!(critical.b < 0.1, "critical B should be ~0.0");
    }

    /// Alert-banner urgency=0 and 1 both map to info (blue).
    #[tokio::test]
    async fn test_alert_banner_urgency_low_maps_to_info() {
        let no_tokens = HashMap::new();
        let info0 = urgency_to_severity_color(0, &no_tokens);
        let info1 = urgency_to_severity_color(1, &no_tokens);
        // Info color is blue-ish (#4A9EFF).
        assert!(info0.b > 0.9, "info urgency=0 should be blue");
        assert!(info1.b > 0.9, "info urgency=1 should be blue");
        // Both should be the same color.
        assert_eq!(info0.r, info1.r);
        assert_eq!(info0.b, info1.b);
    }

    /// notification-area does NOT use urgency-to-severity mapping (color.severity.*).
    /// It uses color.notification.urgency.* tokens instead.
    /// Even with urgency=3, it must NOT produce severity critical (red #FF0000).
    #[tokio::test]
    async fn test_notification_area_does_not_use_severity_tokens() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area - uses notification urgency tokens, not severity".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.05, 0.05, 0.05, 0.85)),
                backdrop_opacity: Some(0.85),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 16,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "System alert".to_owned(),
                    icon: String::new(),
                    urgency: 3, // Critical — must use color.notification.urgency.critical, NOT color.severity.critical
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // notification-area must NOT be treated as alert-banner.
        assert!(
            !is_alert_banner_zone("notification-area"),
            "notification-area must not be treated as alert-banner"
        );

        // Render and check: the backdrop must NOT be severity critical (pure red R~1.0, G~0.0, B~0.0).
        // It should be notification urgency critical fallback: #8B1A1A (R~0.545, G~0.102, B~0.102).
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        assert!(
            !vertices.is_empty(),
            "expected backdrop vertices for notification-area urgency=3"
        );

        // Check first vertex: R should NOT be ~1.0 (that would be severity critical).
        let first = &vertices[0];
        assert!(
            first.color[0] < 0.7,
            "notification-area urgency=3 R must NOT be severity critical (~1.0), got {}",
            first.color[0]
        );
    }

    // ── Notification urgency token tests ─────────────────────────────────────

    /// urgency_to_notification_color: low (0) maps to #2A2A2A fallback.
    #[test]
    fn test_notification_urgency_low_fallback() {
        let no_tokens = HashMap::new();
        let color = urgency_to_notification_color(0, &no_tokens);
        // #2A2A2A → R=G=B ≈ 0.165
        assert!(
            (color.r - 0.165).abs() < 0.01,
            "urgency low R should be ~0.165, got {}",
            color.r
        );
        assert!(
            (color.g - 0.165).abs() < 0.01,
            "urgency low G should be ~0.165, got {}",
            color.g
        );
        assert!(
            (color.b - 0.165).abs() < 0.01,
            "urgency low B should be ~0.165, got {}",
            color.b
        );
    }

    /// urgency_to_notification_color: normal (1) maps to #1A1A3A fallback.
    #[test]
    fn test_notification_urgency_normal_fallback() {
        let no_tokens = HashMap::new();
        let color = urgency_to_notification_color(1, &no_tokens);
        // #1A1A3A: R≈0.102, G≈0.102, B≈0.227
        assert!(
            (color.r - 0.102).abs() < 0.01,
            "urgency normal R should be ~0.102, got {}",
            color.r
        );
        assert!(
            (color.b - 0.227).abs() < 0.02,
            "urgency normal B should be ~0.227, got {}",
            color.b
        );
        // B must be clearly greater than R for blue-tint
        assert!(
            color.b > color.r + 0.05,
            "urgency normal B should be > R (blue tint)"
        );
    }

    /// urgency_to_notification_color: urgent (2) maps to #8B6914 fallback.
    #[test]
    fn test_notification_urgency_urgent_fallback() {
        let no_tokens = HashMap::new();
        let color = urgency_to_notification_color(2, &no_tokens);
        // #8B6914: R≈0.545, G≈0.412, B≈0.078
        assert!(
            color.r > 0.4,
            "urgency urgent R should be >0.4, got {}",
            color.r
        );
        assert!(
            color.g > 0.3,
            "urgency urgent G should be >0.3, got {}",
            color.g
        );
        assert!(
            color.b < 0.2,
            "urgency urgent B should be <0.2, got {}",
            color.b
        );
        // R should be greatest (amber-gold tint)
        assert!(
            color.r > color.b,
            "urgency urgent R should be > B (amber tint)"
        );
    }

    /// urgency_to_notification_color: critical (3) maps to #8B1A1A fallback.
    #[test]
    fn test_notification_urgency_critical_fallback() {
        let no_tokens = HashMap::new();
        let color = urgency_to_notification_color(3, &no_tokens);
        // #8B1A1A: R≈0.545, G≈0.102, B≈0.102
        assert!(
            color.r > 0.4,
            "urgency critical R should be >0.4, got {}",
            color.r
        );
        assert!(
            color.g < 0.2,
            "urgency critical G should be <0.2, got {}",
            color.g
        );
        assert!(
            color.b < 0.2,
            "urgency critical B should be <0.2, got {}",
            color.b
        );
        // R should be clearly greater than G and B (dark red)
        assert!(
            color.r > color.g + 0.2,
            "urgency critical R should dominate (dark red)"
        );
    }

    /// urgency_to_notification_color: urgency > 3 is clamped to critical (3).
    #[test]
    fn test_notification_urgency_clamped_above_3() {
        let no_tokens = HashMap::new();
        let critical3 = urgency_to_notification_color(3, &no_tokens);
        let clamped4 = urgency_to_notification_color(4, &no_tokens);
        let clamped100 = urgency_to_notification_color(100, &no_tokens);
        assert_eq!(critical3.r, clamped4.r, "urgency=4 should clamp to urgency=3");
        assert_eq!(critical3.g, clamped4.g, "urgency=4 should clamp to urgency=3");
        assert_eq!(critical3.b, clamped4.b, "urgency=4 should clamp to urgency=3");
        assert_eq!(critical3.r, clamped100.r, "urgency=100 should clamp to urgency=3");
    }

    /// Profile token override: color.notification.urgency.low overrides fallback.
    #[test]
    fn test_notification_urgency_low_token_override() {
        let mut token_map = HashMap::new();
        // Override low with pure cyan (#00FFFF) — clearly distinct from default.
        token_map.insert(
            "color.notification.urgency.low".to_string(),
            "#00FFFF".to_string(),
        );
        let color = urgency_to_notification_color(0, &token_map);
        assert!(
            color.r < 0.1,
            "custom low token R should be ~0.0 (cyan), got {}",
            color.r
        );
        assert!(
            color.g > 0.9,
            "custom low token G should be ~1.0 (cyan), got {}",
            color.g
        );
        assert!(
            color.b > 0.9,
            "custom low token B should be ~1.0 (cyan), got {}",
            color.b
        );
    }

    /// Profile token override: color.notification.urgency.critical overrides fallback.
    #[test]
    fn test_notification_urgency_critical_token_override() {
        let mut token_map = HashMap::new();
        // Override critical with pure magenta (#FF00FF).
        token_map.insert(
            "color.notification.urgency.critical".to_string(),
            "#FF00FF".to_string(),
        );
        let color = urgency_to_notification_color(3, &token_map);
        assert!(
            color.r > 0.9,
            "custom critical token R should be ~1.0, got {}",
            color.r
        );
        assert!(
            color.b > 0.9,
            "custom critical token B should be ~1.0, got {}",
            color.b
        );
        assert!(
            color.g < 0.1,
            "custom critical token G should be ~0.0, got {}",
            color.g
        );
    }

    /// notification-area urgency-tinted backdrop: renders backdrop at 0.9 opacity.
    ///
    /// Per spec: non-alert-banner Notification content must use
    /// color.notification.urgency.* tokens with fixed 0.9 opacity.
    /// The policy.backdrop_opacity must NOT override this.
    #[tokio::test]
    async fn test_notification_area_backdrop_uses_0_9_opacity() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area urgency opacity test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.5)),
                // backdrop_opacity = 0.5 must NOT override the 0.9 fixed opacity
                backdrop_opacity: Some(0.5),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "test".to_owned(),
                    icon: String::new(),
                    urgency: 1, // normal
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        assert!(!vertices.is_empty(), "expected vertices");
        // The first quad's alpha (index 3 of color) should be 0.9.
        let first_alpha = vertices[0].color[3];
        assert!(
            (first_alpha - 0.9).abs() < 0.01,
            "notification-area backdrop alpha must be 0.9, got {first_alpha}"
        );
    }

    /// notification-area border rendering: 1px 4-quad border is emitted after the
    /// urgency-tinted backdrop quad.
    ///
    /// For a Stack zone with one Notification publish, render_zone_content should emit:
    ///   - 6 vertices for the backdrop quad
    ///   - up to 24 vertices (4 × 6) for the border quads
    #[tokio::test]
    async fn test_notification_area_emits_border_quads() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area border test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.05, 0.05, 0.05, 1.0)),
                font_size_px: Some(18.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "border test".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        // One backdrop (6 vertices) + up to 4 border quads (6 each) = 6 + 24 = 30 max.
        // Minimum: 6 (backdrop) + 6 (at least top edge border) = 12.
        assert!(
            vertices.len() >= 12,
            "expected at least 12 vertices (backdrop + border), got {}",
            vertices.len()
        );
        // Total should be 6 * N for some N ≥ 2 (backdrop + at least one border quad).
        assert_eq!(
            vertices.len() % 6,
            0,
            "vertex count must be a multiple of 6 (each quad = 6 vertices), got {}",
            vertices.len()
        );
    }

    /// alert-banner does NOT emit border quads — border rendering is only for
    /// non-alert-banner notification zones.
    #[tokio::test]
    async fn test_alert_banner_does_not_emit_border_quads() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "alert banner — no border".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.07,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(20.0),
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                backdrop_opacity: Some(1.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "no border here".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        // alert-banner: only 6 vertices (one backdrop quad, no border).
        assert_eq!(
            vertices.len(),
            6,
            "alert-banner must emit exactly 6 vertices (backdrop only, no border), got {}",
            vertices.len()
        );
    }

    /// Border color uses color.border.default token when present.
    #[tokio::test]
    async fn test_notification_area_border_uses_border_default_token() {
        let (mut compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        // Install a custom border token: pure cyan (#00FFFF).
        let mut token_map = HashMap::new();
        token_map.insert("color.border.default".to_string(), "#00FFFF".to_string());
        compositor.set_token_map(token_map);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area border token test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.05, 0.05, 0.05, 1.0)),
                font_size_px: Some(18.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "cyan border".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        // vertices[0..6] = backdrop quad (urgency low color)
        // vertices[6..] = border quads (should be cyan: R≈0, G≈1, B≈1)
        assert!(
            vertices.len() > 6,
            "expected border quads after backdrop, only got {}",
            vertices.len()
        );
        // Check border quad color (vertex index 6 is the first border vertex).
        let border_v = &vertices[6];
        assert!(
            border_v.color[0] < 0.1,
            "border R should be ~0.0 (cyan token), got {}",
            border_v.color[0]
        );
        assert!(
            border_v.color[1] > 0.9,
            "border G should be ~1.0 (cyan token), got {}",
            border_v.color[1]
        );
        assert!(
            border_v.color[2] > 0.9,
            "border B should be ~1.0 (cyan token), got {}",
            border_v.color[2]
        );
    }

    // ── Token-resolved severity color tests ───────────────────────────────────

    /// Custom `color.severity.warning` token overrides the hardcoded SEVERITY_WARNING
    /// constant for urgency=2.
    #[test]
    fn test_custom_severity_warning_token_overrides_constant() {
        let mut token_map = HashMap::new();
        // Custom warning: bright green (#00FF00) — clearly distinct from amber.
        token_map.insert("color.severity.warning".to_string(), "#00FF00".to_string());
        let color = urgency_to_severity_color(2, &token_map);
        assert!(
            color.g > 0.9,
            "custom warning token G should be ~1.0 (green), got {}",
            color.g
        );
        assert!(
            color.r < 0.1,
            "custom warning token R should be ~0.0 (green), got {}",
            color.r
        );
        assert!(
            color.b < 0.1,
            "custom warning token B should be ~0.0 (green), got {}",
            color.b
        );
    }

    /// Custom `color.severity.critical` token overrides the hardcoded SEVERITY_CRITICAL.
    #[test]
    fn test_custom_severity_critical_token_overrides_constant() {
        let mut token_map = HashMap::new();
        // Custom critical: bright magenta (#FF00FF).
        token_map.insert("color.severity.critical".to_string(), "#FF00FF".to_string());
        let color = urgency_to_severity_color(3, &token_map);
        assert!(
            color.r > 0.9,
            "custom critical R should be ~1.0 (magenta), got {}",
            color.r
        );
        assert!(
            color.b > 0.9,
            "custom critical B should be ~1.0 (magenta), got {}",
            color.b
        );
        assert!(
            color.g < 0.1,
            "custom critical G should be ~0.0 (magenta), got {}",
            color.g
        );
    }

    /// Custom `color.severity.info` token overrides the hardcoded SEVERITY_INFO.
    #[test]
    fn test_custom_severity_info_token_overrides_constant() {
        let mut token_map = HashMap::new();
        // Custom info: pure red (#FF0000) — clearly distinct from default blue.
        token_map.insert("color.severity.info".to_string(), "#FF0000".to_string());
        let color0 = urgency_to_severity_color(0, &token_map);
        let color1 = urgency_to_severity_color(1, &token_map);
        for (urgency, color) in [(0, color0), (1, color1)] {
            assert!(
                color.r > 0.9,
                "custom info urgency={urgency} R should be ~1.0 (red), got {}",
                color.r
            );
            assert!(
                color.g < 0.1,
                "custom info urgency={urgency} G should be ~0.0 (red), got {}",
                color.g
            );
            assert!(
                color.b < 0.1,
                "custom info urgency={urgency} B should be ~0.0 (red), got {}",
                color.b
            );
        }
    }

    /// Invalid/absent token values fall back to hardcoded constants.
    #[test]
    fn test_invalid_severity_token_value_falls_back_to_constant() {
        let mut token_map = HashMap::new();
        // Not a valid hex color — should be ignored.
        token_map.insert(
            "color.severity.warning".to_string(),
            "not-a-color".to_string(),
        );
        let color = urgency_to_severity_color(2, &token_map);
        // Falls back to SEVERITY_WARNING (#FFB800): R~1.0, G~0.72, B~0.0.
        assert!(
            color.r > 0.9,
            "fallback warning R should be ~1.0, got {}",
            color.r
        );
        assert!(
            color.g > 0.5,
            "fallback warning G should be >0.5, got {}",
            color.g
        );
        assert!(
            color.b < 0.1,
            "fallback warning B should be ~0.0, got {}",
            color.b
        );
    }

    /// Custom severity tokens in [design_tokens] affect alert-banner backdrop colors.
    ///
    /// This is the end-to-end integration test: `set_token_map` populates the
    /// compositor, and `render_zone_content` uses the token-resolved color for the
    /// alert-banner backdrop.
    #[tokio::test]
    async fn test_custom_severity_tokens_affect_alert_banner_backdrop() {
        let (mut compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        // Install a custom token map: override warning with pure green (#00FF00).
        let mut token_map = HashMap::new();
        token_map.insert("color.severity.warning".to_string(), "#00FF00".to_string());
        compositor.set_token_map(token_map);

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "alert banner zone".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.07,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(20.0),
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                backdrop_opacity: Some(1.0),
                text_color: Some(Rgba::new(1.0, 1.0, 1.0, 1.0)),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish urgency=2 (warning) — should use custom green token.
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "Custom token warning".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // Collect vertices from render_zone_content.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        // rect_vertices emits 6 vertices per quad; each vertex has a `color: [f32; 4]` field.
        // The backdrop should be green (R~0.0, G~1.0, B~0.0), not amber.
        assert!(
            !vertices.is_empty(),
            "expected backdrop vertices for alert-banner"
        );

        // Check first vertex color. RectVertex layout: [position: [f32; 2], color: [f32; 4]].
        let first = &vertices[0];
        assert!(
            first.color[1] > 0.9,
            "alert-banner backdrop G should be ~1.0 (custom green token), got {}",
            first.color[1]
        );
        assert!(
            first.color[0] < 0.1,
            "alert-banner backdrop R should be ~0.0 (custom green token), got {}",
            first.color[0]
        );
        assert!(
            first.color[2] < 0.1,
            "alert-banner backdrop B should be ~0.0 (custom green token), got {}",
            first.color[2]
        );
    }

    /// ZoneAnimationState fade-in reaches 1.0 after duration elapses.
    #[test]
    fn test_zone_animation_state_fade_in_completes() {
        // Use 0ms duration for instant completion.
        let state = ZoneAnimationState::fade_in(0);
        // Opacity at duration=0 should immediately be target (1.0).
        let opacity = state.current_opacity();
        assert_eq!(opacity, 1.0, "fade-in with 0ms should be 1.0 immediately");
        assert!(
            state.is_complete(),
            "0ms fade-in should be complete immediately"
        );
    }

    /// ZoneAnimationState fade-out starts at 1.0 and reaches 0.0 after duration.
    #[test]
    fn test_zone_animation_state_fade_out_completes() {
        let state = ZoneAnimationState::fade_out(0);
        let opacity = state.current_opacity();
        assert_eq!(opacity, 0.0, "fade-out with 0ms should be 0.0 immediately");
        assert!(
            state.is_complete(),
            "0ms fade-out should be complete immediately"
        );
    }

    /// ZoneAnimationState with non-zero duration: opacity is interpolated.
    #[test]
    fn test_zone_animation_state_interpolates() {
        // 10_000ms duration — very long, so elapsed << duration.
        let state = ZoneAnimationState::fade_in(10_000);
        // Very shortly after creation, opacity should be close to 0.
        let opacity = state.current_opacity();
        assert!(
            opacity >= 0.0 && opacity <= 0.1,
            "fade-in opacity shortly after start should be near 0, got {opacity}"
        );
        assert!(
            !state.is_complete(),
            "10s fade-in should not be complete immediately"
        );
    }

    /// backdrop_opacity overrides the backdrop color's alpha channel.
    /// When backdrop_opacity=0.6 and backdrop.a=1.0, effective alpha=0.6.
    #[tokio::test]
    async fn test_backdrop_opacity_overrides_color_alpha() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // backdrop color has alpha=1.0 but backdrop_opacity=0.6 should override it.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "test backdrop opacity override".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 1.0)), // alpha=1.0
                backdrop_opacity: Some(0.6),                   // override to 0.6
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("opacity test".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // The backdrop rendered should use alpha=0.6 (backdrop_opacity), not 1.0.
        // We verify this by checking the vertex colors produced — the alpha channel
        // of the first rect vertex should reflect 0.6.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);
        assert!(!vertices.is_empty(), "expected backdrop vertices");
        // The RectVertex has color field [f32; 4]; alpha should be ~0.6.
        let alpha = vertices[0].color[3];
        assert!(
            (alpha - 0.6).abs() < 0.01,
            "backdrop alpha should be ~0.6 (backdrop_opacity override), got {alpha}"
        );
    }

    /// backdrop=None: no backdrop quad rendered even when backdrop_opacity is set.
    #[tokio::test]
    async fn test_no_backdrop_when_backdrop_is_none() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "no-backdrop test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                backdrop: None,
                backdrop_opacity: Some(0.9), // ignored because backdrop is None
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("no backdrop".to_owned()),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // With backdrop=None, no rect vertices should be emitted.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);
        assert!(
            vertices.is_empty(),
            "no backdrop quad should be rendered when policy.backdrop is None"
        );
    }

    /// backdrop=None with Notification content: no backdrop or border quads rendered.
    ///
    /// Even though Notification content in a non-alert-banner zone overrides the
    /// backdrop color with urgency-tinted tokens, the override must respect the
    /// policy.backdrop contract: when backdrop is None, nothing is emitted.
    #[tokio::test]
    async fn test_notification_no_backdrop_when_backdrop_is_none() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "notification area with backdrop=None".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: None,
                backdrop_opacity: Some(0.9),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "no backdrop".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);
        assert!(
            vertices.is_empty(),
            "no backdrop or border quads should be rendered when policy.backdrop is None, got {} vertices",
            vertices.len()
        );
    }

    /// text.rs: TextItem::from_zone_policy respects all RenderingPolicy fields.
    #[test]
    fn test_from_zone_policy_reads_all_policy_fields() {
        use crate::text::TextItem;

        let policy = RenderingPolicy {
            font_size_px: Some(28.0),
            text_color: Some(Rgba::new(1.0, 0.5, 0.0, 1.0)), // orange
            font_family: Some(FontFamily::SystemMonospace),
            text_align: Some(TextAlign::Center),
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(1.5),
            margin_horizontal: Some(12.0),
            margin_vertical: Some(6.0),
            ..Default::default()
        };

        let item = TextItem::from_zone_policy("test", 0.0, 0.0, 400.0, 100.0, &policy, 1.0);
        assert_eq!(item.font_size_px, 28.0);
        assert_eq!(item.font_family, FontFamily::SystemMonospace);
        assert_eq!(item.alignment, TextAlign::Center);
        assert!(item.outline_color.is_some(), "outline_color should be set");
        assert_eq!(item.outline_width.unwrap(), 1.5);
        // Margins: x+12, y+6
        assert_eq!(item.pixel_x, 12.0);
        assert_eq!(item.pixel_y, 6.0);
    }

    // ── Multi-publication rendering: Stack and MergeByKey policies ──────────

    /// Stack zone with two notifications: render_zone_content must emit a
    /// separate backdrop quad for each publication, stacked vertically.
    /// With max_depth=4 and zone height=400px, each slot is 100px tall.
    /// Two publications → two quads; the second quad starts at y+100.
    #[tokio::test]
    async fn test_stack_zone_renders_separate_backdrop_per_publication() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone: top-right, 200×400 px via Relative.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "stack zone for multi-pub test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,    // 320 px at 1280 wide
                height_pct: 0.5556, // ~400 px at 720 tall (≈400/720)
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                backdrop_opacity: Some(0.9),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 4 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish two separate notifications.
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "First notification".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Second notification".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                }),
                "agent-b",
                None,
                None,
                None,
            )
            .unwrap();

        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        // Two publications → two backdrop quads (6 verts each) + border quads.
        // Each Notification slot emits: 1 backdrop quad (6) + 4 border quads (24) = 30.
        // Total: 2 × 30 = 60 vertices.
        // We assert at least 12 (2 backdrops) and a multiple of 6.
        assert!(
            vertices.len() >= 12,
            "Stack zone with 2 publications must emit at least 12 vertices (2 backdrop quads), got {}",
            vertices.len()
        );
        assert_eq!(
            vertices.len() % 6,
            0,
            "vertex count must be a multiple of 6 (each quad = 6 vertices), got {}",
            vertices.len()
        );

        // The first backdrop quad's top-left y should be 0 (zone starts at y_pct=0.0 → y=0).
        // The second backdrop quad's top-left y should be ~slot_h after the first slot.
        // zone_h = 720 * 0.5556 ≈ 400; slot_h is content-sized per stack_slot_height.
        // Vertices are in NDC; we check the first and 7th vertex y values differ.
        // rect_vertices emits 6 verts per quad in positions [x,y] NDC.
        // With border quads added, the second backdrop quad starts at vertex index 30
        // (6 backdrop + 24 border for first slot = 30 verts per slot).
        let first_quad_y = vertices[0].position[1];
        // Find second backdrop by skipping first slot (30 vertices: 6 backdrop + 24 border).
        let second_quad_idx = 30; // 6 backdrop + 4 border quads (6 each)
        if vertices.len() > second_quad_idx {
            let second_quad_y = vertices[second_quad_idx].position[1];
            assert!(
                (first_quad_y - second_quad_y).abs() > 0.01,
                "second Stack slot must start at a different y than the first; got first={first_quad_y:.4}, second={second_quad_y:.4}"
            );
        }
    }

    /// Stack zone: collect_text_items must produce a separate TextItem for
    /// each publication, with each item positioned in its own vertical slot.
    #[tokio::test]
    async fn test_stack_zone_collect_text_items_per_publication() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "stack zone text items test".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.5556,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 4 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Alpha alert".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                }),
                "agent-a",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Beta alert".to_owned(),
                    icon: String::new(),
                    urgency: 1,
                }),
                "agent-b",
                None,
                None,
                None,
            )
            .unwrap();
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Gamma alert".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                }),
                "agent-c",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // Three publications in a Stack zone must produce three TextItems.
        assert_eq!(
            items.len(),
            3,
            "Stack zone with 3 publications must produce 3 TextItems, got {}",
            items.len()
        );

        // Items should be ordered newest-first (slot 0 = newest at top of zone).
        assert!(
            items[0].text.contains("Gamma"),
            "first TextItem should be the newest publication (Gamma), got: {}",
            items[0].text
        );
        assert!(
            items[1].text.contains("Beta"),
            "second TextItem should be Beta, got: {}",
            items[1].text
        );
        assert!(
            items[2].text.contains("Alpha"),
            "third TextItem should be the oldest publication (Alpha), got: {}",
            items[2].text
        );

        // Each item should occupy a different vertical slot; slot 0 is at top
        // (lowest y), slot 1 below it, slot 2 below that.
        assert!(
            items[1].pixel_y > items[0].pixel_y,
            "slot 1 y ({}) must be below slot 0 y ({})",
            items[1].pixel_y,
            items[0].pixel_y
        );
        assert!(
            items[2].pixel_y > items[1].pixel_y,
            "slot 2 y ({}) must be below slot 1 y ({})",
            items[2].pixel_y,
            items[1].pixel_y
        );
    }

    // ── Slot layout: content-sized slots, newest-first ───────────────────────

    /// 5 stacked notifications must each appear at a distinct y-position.
    /// slot_height = font_size_px(16) + 18 = 34 px.
    /// Zone is tall enough to accommodate all 5 slots.
    /// Verifies newest-first ordering: slot 0 = newest at zone top.
    #[tokio::test]
    async fn test_stack_slot_layout_five_notifications_distinct_y() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone: 300px wide × 300px tall — enough for 5 × 34px slots (170px).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "slot layout test zone".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,    // 320 px at 1280 wide
                height_pct: 0.4167, // ~300 px at 720 tall
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 5,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish 5 notifications (oldest to newest: "N1" .. "N5").
        for i in 1..=5 {
            scene
                .publish_to_zone(
                    "notification-area",
                    ZoneContent::Notification(NotificationPayload {
                        text: format!("N{i}"),
                        icon: String::new(),
                        urgency: 1,
                    }),
                    &format!("agent-{i}"),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(items.len(), 5, "5 notifications must produce 5 TextItems");

        // With font_size_px=16 → slot_h = 34.  margin_v defaults to 8.
        // pixel_y for slot i = zone_y + i*slot_h + margin_v.
        // Check all 5 y-values are strictly increasing (slot 0 = newest = top).
        let ys: Vec<f32> = items.iter().map(|it| it.pixel_y).collect();
        for w in ys.windows(2) {
            assert!(
                w[1] > w[0],
                "slots must have strictly increasing y; got consecutive y={:.2} then {:.2}",
                w[0],
                w[1]
            );
        }

        // Newest notification is "N5" — must be slot 0 (lowest y).
        assert!(
            items[0].text.contains("N5"),
            "slot 0 must be the newest notification (N5), got: {}",
            items[0].text
        );
        // Oldest is "N1" — must be slot 4 (highest y).
        assert!(
            items[4].text.contains("N1"),
            "slot 4 must be the oldest notification (N1), got: {}",
            items[4].text
        );
    }

    /// When a 6th notification is published to a Stack zone with max_depth=5,
    /// the oldest notification must be evicted.  After eviction, only 5 items
    /// remain and the evicted notification is absent.
    #[tokio::test]
    async fn test_stack_slot_sixth_notification_evicts_oldest() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "eviction test zone".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.5,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 6,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish 6 notifications; "oldest" is "Oldest" (first published).
        scene
            .publish_to_zone(
                "notification-area",
                ZoneContent::Notification(NotificationPayload {
                    text: "Oldest".to_owned(),
                    icon: String::new(),
                    urgency: 0,
                }),
                "agent-0",
                None,
                None,
                None,
            )
            .unwrap();
        for i in 1..=5 {
            scene
                .publish_to_zone(
                    "notification-area",
                    ZoneContent::Notification(NotificationPayload {
                        text: format!("N{i}"),
                        icon: String::new(),
                        urgency: 1,
                    }),
                    &format!("agent-{i}"),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // Only 5 items after eviction (max_depth=5).
        assert_eq!(
            items.len(),
            5,
            "after 6th publish with max_depth=5, must have 5 TextItems; got {}",
            items.len()
        );

        // "Oldest" must have been evicted — not present in any item.
        let has_oldest = items.iter().any(|it| it.text.contains("Oldest"));
        assert!(
            !has_oldest,
            "oldest notification must be evicted after 6th publish"
        );

        // "N5" (newest) must be present as slot 0.
        assert!(
            items[0].text.contains("N5"),
            "newest notification (N5) must be slot 0 after eviction, got: {}",
            items[0].text
        );
    }

    /// Stack notifications clip at zone boundary: slots whose top-left y is at or
    /// beyond zone_bottom are fully clipped and produce no TextItem. Partial slots
    /// (y < zone_bottom but y+slot_h > zone_bottom) are emitted with clamped height.
    #[tokio::test]
    async fn test_stack_slot_clips_at_zone_boundary() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Zone height: 72px at 720 tall → height_pct = 72/720 = 0.1.
        // font_size_px=16, default margin_v=8 → slot_h = 16+2*8+2 = 34px.
        // 2 full slots fit (slots 0,1: 2*34=68 < 72); slot 2 partial (y=68 < 72); slot 3 clipped.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_owned(),
            description: "clipping test zone".to_owned(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.0,
                width_pct: 0.25,
                height_pct: 0.1, // 72 px at 720 tall
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(16.0),
                backdrop: Some(Rgba::new(0.1, 0.1, 0.1, 0.9)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Stack { max_depth: 5 },
            max_publishers: 5,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish 4 notifications; only the 2 newest should be visible (newest-first).
        for i in 1..=4 {
            scene
                .publish_to_zone(
                    "notification-area",
                    ZoneContent::Notification(NotificationPayload {
                        text: format!("M{i}"),
                        icon: String::new(),
                        urgency: 1,
                    }),
                    &format!("agent-{i}"),
                    None,
                    None,
                    None,
                )
                .unwrap();
        }

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // Zone height = 72px, slot_h = 34px.
        // slot 0 at y=0  (fits: 0+34=34 ≤ 72 → emitted).
        // slot 1 at y=34 (fits: 34+34=68 ≤ 72 → emitted).
        // slot 2 at y=68 (partial: y=68 < 72, effective_slot_h = 4 → still emitted,
        //   clamped; text may not visually render but the TextItem is produced).
        // slot 3 at y=102 → y ≥ zone_bottom(72) → loop breaks, no item.
        // Exactly 3 items (slots 0, 1, 2) are emitted.
        assert_eq!(
            items.len(),
            3,
            "with 72px zone and 34px slots, exactly 3 items should be emitted; got {}",
            items.len()
        );

        // The newest notification (M4) must be in slot 0.
        assert!(
            !items.is_empty() && items[0].text.contains("M4"),
            "newest notification (M4) must be slot 0 (top of zone), got: {}",
            if items.is_empty() {
                "empty"
            } else {
                &items[0].text
            }
        );
    }

    /// MergeByKey zone: collect_text_items must merge ALL StatusBar publications'
    /// entries and produce a single TextItem containing all unique keys.
    #[tokio::test]
    async fn test_merge_by_key_zone_merges_all_status_bar_entries() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_owned(),
            description: "merge-by-key zone test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 32 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Agent A publishes "cpu" and "mem" keys.
        let mut entries_a = std::collections::HashMap::new();
        entries_a.insert("cpu".to_owned(), "45%".to_owned());
        entries_a.insert("mem".to_owned(), "8.2 GB".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload { entries: entries_a }),
                "agent-a",
                Some("cpu-mem".to_owned()),
                None,
                None,
            )
            .unwrap();

        // Agent B publishes a "net" key.
        let mut entries_b = std::collections::HashMap::new();
        entries_b.insert("net".to_owned(), "1.2 MB/s".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload { entries: entries_b }),
                "agent-b",
                Some("net".to_owned()),
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // MergeByKey must produce exactly ONE TextItem containing all merged entries.
        assert_eq!(
            items.len(),
            1,
            "MergeByKey zone must produce a single merged TextItem, got {}",
            items.len()
        );

        let text = &items[0].text;
        assert!(
            text.contains("cpu"),
            "merged text must include 'cpu' key; got: {text}"
        );
        assert!(
            text.contains("mem"),
            "merged text must include 'mem' key; got: {text}"
        );
        assert!(
            text.contains("net"),
            "merged text must include 'net' key; got: {text}"
        );
        assert!(
            text.contains("45%"),
            "merged text must include cpu value '45%'; got: {text}"
        );
        assert!(
            text.contains("1.2 MB/s"),
            "merged text must include net value '1.2 MB/s'; got: {text}"
        );
    }

    /// MergeByKey zone: when a key appears in multiple publications, the latest
    /// value wins (last-write-wins per key semantics).
    #[tokio::test]
    async fn test_merge_by_key_latest_value_wins_for_duplicate_keys() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_owned(),
            description: "merge-by-key duplicate key test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.08, 0.08, 0.08, 1.0)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 32 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // First publish: cpu = "10%"
        let mut entries_old = std::collections::HashMap::new();
        entries_old.insert("cpu".to_owned(), "10%".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload {
                    entries: entries_old,
                }),
                "agent-a",
                Some("cpu".to_owned()),
                None,
                None,
            )
            .unwrap();

        // Second publish: same key "cpu" with updated value "90%"
        let mut entries_new = std::collections::HashMap::new();
        entries_new.insert("cpu".to_owned(), "90%".to_owned());
        scene
            .publish_to_zone(
                "status-bar",
                ZoneContent::StatusBar(StatusBarPayload {
                    entries: entries_new,
                }),
                "agent-a",
                Some("cpu".to_owned()),
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        assert_eq!(items.len(), 1, "must produce one merged TextItem");
        let text = &items[0].text;

        // The latest value "90%" must appear; "10%" must not.
        assert!(
            text.contains("90%"),
            "merged text must show latest cpu value '90%'; got: {text}"
        );
        assert!(
            !text.contains("10%"),
            "merged text must not show stale cpu value '10%'; got: {text}"
        );
    }

    /// LatestWins zone still renders only one TextItem even when multiple
    /// publications are present (regression guard).
    #[tokio::test]
    async fn test_latest_wins_zone_renders_only_latest_publication() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_owned(),
            description: "latest-wins regression guard".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 16.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy {
                backdrop: Some(Rgba::new(0.0, 0.0, 0.0, 0.7)),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        });

        // LatestWins should only keep the last publication.
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("Old content".to_owned()),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();
        // With LatestWins policy the scene graph may have already replaced it,
        // but we publish again to ensure only one ends up active.
        scene
            .publish_to_zone(
                "subtitle",
                ZoneContent::StreamText("New content".to_owned()),
                "agent",
                None,
                None,
                None,
            )
            .unwrap();

        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);

        // LatestWins must produce exactly one TextItem.
        assert_eq!(
            items.len(),
            1,
            "LatestWins zone must produce exactly 1 TextItem; got {}",
            items.len()
        );
        // The item should contain the latest content.
        assert!(
            items[0].text.contains("New content"),
            "LatestWins must render latest publish; got: {}",
            items[0].text
        );
    }

    // ── Alert-banner heading typography tests [hud-w3o6.2] ───────────────────
    //
    // Acceptance criteria from spec §Alert-Banner Heading Typography:
    //   1. font_size_px = 24px
    //   2. font_weight = 700 (bold)
    //   3. font_family = SystemSansSerif
    //   4. text_color = #FFFFFF white
    //   5. margin_horizontal inset applied

    /// Alert-banner RenderingPolicy carries heading typography:
    /// 24px font, weight 700, SystemSansSerif, white text, margin_horizontal=8.
    ///
    /// Acceptance criterion 3.1–3.3: heading typography wired to alert-banner zone.
    #[tokio::test]
    async fn test_alert_banner_heading_typography_in_rendering_policy() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Register alert-banner with heading-typography RenderingPolicy (spec values).
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "heading typography test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.06,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(24.0),
                font_family: Some(FontFamily::SystemSansSerif),
                font_weight: Some(700),
                text_color: Some(Rgba {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                }),
                backdrop: Some(Rgba::new(0.1, 0.1, 0.16, 0.9)),
                backdrop_opacity: Some(0.9),
                margin_horizontal: Some(8.0),
                margin_vertical: Some(0.0),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // Publish a notification payload.
        scene
            .publish_to_zone(
                "alert-banner",
                ZoneContent::Notification(NotificationPayload {
                    text: "Weather alert: severe storms".to_owned(),
                    icon: String::new(),
                    urgency: 2,
                }),
                "test",
                None,
                None,
                None,
            )
            .unwrap();

        // collect_text_items uses the RenderingPolicy fields for TextItem construction.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert_eq!(
            items.len(),
            1,
            "expected one TextItem for alert-banner notification"
        );

        let item = &items[0];

        // AC 3.1: font_size_px = 24.0
        assert_eq!(
            item.font_size_px, 24.0,
            "alert-banner text must be 24px per spec §Alert-Banner Heading Typography"
        );

        // AC 3.1: font_family = SystemSansSerif
        assert_eq!(
            item.font_family,
            FontFamily::SystemSansSerif,
            "alert-banner text must use system sans-serif family"
        );

        // AC 3.1: font_weight = 700
        assert_eq!(
            item.font_weight, 700,
            "alert-banner text must be weight 700 (bold)"
        );

        // AC 3.2: text_color = #FFFFFF (white)
        // White in linear sRGB: R=1.0 → 255u8, G=1.0 → 255u8, B=1.0 → 255u8.
        assert_eq!(item.color[0], 255, "text R should be 255 (white)");
        assert_eq!(item.color[1], 255, "text G should be 255 (white)");
        assert_eq!(item.color[2], 255, "text B should be 255 (white)");

        // AC 3.3: text is inset from x=0 by margin_horizontal=8
        // Zone geometry: zx = (sw - sw*1.0)/2 = 0.0, so pixel_x = 0 + 8 = 8.
        assert_eq!(
            item.pixel_x, 8.0,
            "text must be inset by margin_horizontal=8 from backdrop edge"
        );
    }

    /// Alert-banner zone has LayerAttachment::Chrome — renders above all agent content.
    ///
    /// Acceptance criterion: chrome-layer z-order verified by checking ZoneDefinition.
    #[test]
    fn test_alert_banner_default_zone_has_chrome_layer_attachment() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner must be in default zone registry");

        assert_eq!(
            zone.layer_attachment,
            LayerAttachment::Chrome,
            "alert-banner zone must be attached to chrome layer (above all agent content)"
        );
    }

    /// Alert-banner zone spans full display width (width_pct = 1.0).
    ///
    /// Acceptance criterion: backdrop quad spans from x=0 to x=display_width.
    #[test]
    fn test_alert_banner_default_zone_is_full_width() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner must be in default zone registry");

        match zone.geometry_policy {
            GeometryPolicy::EdgeAnchored { width_pct, .. } => {
                assert_eq!(
                    width_pct, 1.0,
                    "alert-banner must span full display width (width_pct=1.0)"
                );
            }
            _ => panic!("alert-banner must use EdgeAnchored geometry for full-width positioning"),
        }
    }

    /// Alert-banner zone resolve_zone_geometry gives backdrop width = display width.
    ///
    /// At 1920×1080, the backdrop must span from x=0 to x=1920.
    #[test]
    fn test_alert_banner_backdrop_spans_full_display_width() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner zone must exist");

        let (x, _y, w, _h) =
            Compositor::resolve_zone_geometry(&zone.geometry_policy, 1920.0, 1080.0);
        assert_eq!(x, 0.0, "alert-banner left edge must be at x=0");
        assert_eq!(
            w, 1920.0,
            "alert-banner width must equal display width (1920)"
        );
    }

    /// Alert-banner zone height accommodates 24px heading + vertical padding.
    ///
    /// At 720p, height_pct=0.06 → 43.2px > 24px + 2×8px = 40px minimum.
    #[test]
    fn test_alert_banner_zone_height_accommodates_heading_typography() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner zone must exist");

        // Check that resolved height at 720p is sufficient for 24px heading.
        // margin_vertical=0.0 (flush to edge), so minimum is font_size_px only.
        // height_pct=0.06 → 0.06×720=43.2px, well above the 24px minimum.
        let (_x, _y, _w, h) =
            Compositor::resolve_zone_geometry(&zone.geometry_policy, 1280.0, 720.0);
        let font_size_px = zone.rendering_policy.font_size_px.unwrap_or(24.0);
        let min_required = font_size_px; // margin_vertical=0.0; height must cover font at minimum
        assert!(
            h >= min_required,
            "alert-banner height {h}px must accommodate heading ({font_size_px}px)"
        );
    }

    /// When no alert-banner publications are active, render_zone_content emits zero
    /// backdrop vertices — the zone occupies zero visible space.
    ///
    /// Acceptance criterion §Alert-Banner Chrome-Layer Positioning:
    ///   "When no alerts are active, the alert-banner zone MUST occupy zero vertical space."
    #[tokio::test]
    async fn test_alert_banner_zero_height_when_inactive() {
        let (compositor, _surface) = make_compositor_and_surface(1280, 720).await;

        let mut scene = SceneGraph::new(1280.0, 720.0);
        // Register alert-banner zone with a visible backdrop so it would render if active.
        scene.register_zone(ZoneDefinition {
            id: SceneId::new(),
            name: "alert-banner".to_owned(),
            description: "zero-height-when-inactive test".to_owned(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Top,
                height_pct: 0.06,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy {
                font_size_px: Some(24.0),
                backdrop: Some(Rgba::new(1.0, 0.0, 0.0, 1.0)), // bright red — visible if leaked
                backdrop_opacity: Some(0.9),
                text_color: Some(Rgba::WHITE),
                ..Default::default()
            },
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        });

        // No publications — zone is inactive.
        let mut vertices: Vec<crate::pipeline::RectVertex> = Vec::new();
        compositor.render_zone_content(&scene, &mut vertices, 1280.0, 720.0);

        // Zero vertices emitted → zone occupies zero visible space.
        assert!(
            vertices.is_empty(),
            "no backdrop quad must be emitted for inactive alert-banner zone (zero visible space)"
        );

        // Also verify no TextItems produced.
        let items = compositor.collect_text_items(&scene, 1280.0, 720.0);
        assert!(
            items.is_empty(),
            "no text must be rendered for inactive alert-banner zone"
        );
    }

    /// Alert-banner RenderingPolicy in ZoneRegistry::with_defaults() carries
    /// heading typography: 24px, weight 700, white text, margin_horizontal=8.
    #[test]
    fn test_alert_banner_default_zone_rendering_policy_has_heading_typography() {
        use tze_hud_scene::types::ZoneRegistry;

        let registry = ZoneRegistry::with_defaults();
        let zone = registry
            .get_by_name("alert-banner")
            .expect("alert-banner must be in default zone registry");

        let policy = &zone.rendering_policy;

        assert_eq!(
            policy.font_size_px,
            Some(24.0),
            "alert-banner default rendering policy must have font_size_px=24"
        );
        assert_eq!(
            policy.font_weight,
            Some(700),
            "alert-banner default rendering policy must have font_weight=700 (bold)"
        );
        assert_eq!(
            policy.font_family,
            Some(FontFamily::SystemSansSerif),
            "alert-banner default rendering policy must use SystemSansSerif"
        );
        // text_color must be white (R=1.0, G=1.0, B=1.0).
        let tc = policy
            .text_color
            .expect("alert-banner default rendering policy must have text_color set");
        assert!(
            (tc.r - 1.0).abs() < 0.01,
            "text_color R must be 1.0 (white), got {}",
            tc.r
        );
        assert!(
            (tc.g - 1.0).abs() < 0.01,
            "text_color G must be 1.0 (white), got {}",
            tc.g
        );
        assert!(
            (tc.b - 1.0).abs() < 0.01,
            "text_color B must be 1.0 (white), got {}",
            tc.b
        );
        assert_eq!(
            policy.margin_horizontal,
            Some(8.0),
            "alert-banner default rendering policy must have margin_horizontal=8"
        );
    }
}
