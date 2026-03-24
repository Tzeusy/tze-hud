//! # window
//!
//! Window mode abstraction per runtime-kernel/spec.md §Window Modes (line 172).
//!
//! ## Two modes, one API
//!
//! - **Fullscreen**: compositor owns the entire display. Opaque background.
//!   All input captured. All platforms.
//! - **Overlay / HUD**: transparent, borderless, always-on-top window.
//!   Per-region input passthrough (pointer events outside active hit-regions
//!   pass through to the desktop). Platform-specific.
//!
//! Runtime mode switching is supported but disruptive — it requires surface
//! recreation.
//!
//! ## Fallback behaviour
//!
//! On GNOME Wayland (no `wlr-layer-shell`), overlay mode silently degrades to
//! fullscreen with a startup warning logged (spec line 186).

use std::fmt;

// ─── Window mode ─────────────────────────────────────────────────────────────

/// The configured window mode for the runtime.
///
/// Modes are set at startup. Switching at runtime is possible but requires
/// surface recreation (spec line 175).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowMode {
    /// Compositor owns the entire display. Opaque. All input captured.
    #[default]
    Fullscreen,
    /// Transparent borderless always-on-top window.
    /// Pointer events outside active hit-regions pass through to the desktop.
    Overlay,
}

impl fmt::Display for WindowMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowMode::Fullscreen => write!(f, "fullscreen"),
            WindowMode::Overlay => write!(f, "overlay"),
        }
    }
}

// ─── Platform overlay support ─────────────────────────────────────────────────

/// Describes platform-specific overlay support detection results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlaySupport {
    /// Full overlay support available (e.g., X11, Windows, macOS).
    Supported,
    /// Overlay requested but unavailable on this platform/compositor.
    /// The runtime falls back to fullscreen with a warning.
    FallbackToFullscreen { reason: FallbackReason },
}

/// Reason why overlay mode fell back to fullscreen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackReason {
    /// GNOME Wayland — no `wlr-layer-shell` extension available.
    GnomeWaylandNoLayerShell,
    /// Generic unsupported platform.
    Unsupported(String),
}

impl fmt::Display for FallbackReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FallbackReason::GnomeWaylandNoLayerShell => {
                write!(f, "GNOME Wayland: wlr-layer-shell not available")
            }
            FallbackReason::Unsupported(msg) => {
                write!(f, "unsupported platform: {msg}")
            }
        }
    }
}

// ─── Input passthrough ────────────────────────────────────────────────────────

/// Represents a rectangular hit-region on screen for overlay input passthrough.
///
/// Pointer events within the union of all `HitRegion` bounds are captured by
/// the runtime. Events outside any hit-region are passed through to the desktop.
#[derive(Debug, Clone, PartialEq)]
pub struct HitRegion {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl HitRegion {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    /// Returns `true` if the point (px, py) is inside this region.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x
            && px < self.x + self.width
            && py >= self.y
            && py < self.y + self.height
    }
}

// ─── WindowConfig ─────────────────────────────────────────────────────────────

/// Complete window configuration for the runtime.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub mode: WindowMode,
    pub width: u32,
    pub height: u32,
    /// Title used in non-fullscreen modes (for debugging, alt-tab, etc.).
    pub title: String,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            mode: WindowMode::Fullscreen,
            width: 1920,
            height: 1080,
            title: "tze_hud".to_string(),
        }
    }
}

// ─── Effective mode resolution ────────────────────────────────────────────────

/// Resolve the effective window mode, performing platform fallback checks.
///
/// Spec line 186: "WHEN runtime starts in overlay mode on GNOME Wayland
/// (no layer-shell) THEN runtime MUST fall back to fullscreen silently
/// with startup warning logged."
pub fn resolve_window_mode(requested: WindowMode) -> (WindowMode, Option<FallbackReason>) {
    if requested == WindowMode::Overlay {
        // Detect overlay availability.
        match check_overlay_support() {
            OverlaySupport::Supported => (WindowMode::Overlay, None),
            OverlaySupport::FallbackToFullscreen { reason } => {
                tracing::warn!(
                    reason = %reason,
                    "overlay mode unavailable; falling back to fullscreen"
                );
                (WindowMode::Fullscreen, Some(reason))
            }
        }
    } else {
        (WindowMode::Fullscreen, None)
    }
}

/// Detect whether overlay mode is available on the current platform.
///
/// This is a best-effort heuristic based on environment variables and
/// compiled platform. Full Wayland protocol negotiation happens inside
/// winit/raw-window-handle integration and is deferred to the windowed
/// runtime (out of scope for this bead). Here we provide the scaffolding
/// that callers can override with real probe results.
pub fn check_overlay_support() -> OverlaySupport {
    #[cfg(target_os = "linux")]
    {
        check_overlay_support_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS and Windows generally support borderless always-on-top windows.
        OverlaySupport::Supported
    }
}

#[cfg(target_os = "linux")]
fn check_overlay_support_linux() -> OverlaySupport {
    // Probe: if WAYLAND_DISPLAY is set and XDG_CURRENT_DESKTOP looks like GNOME,
    // we assume layer-shell is not available (GNOME does not support it as of 2026).
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let is_gnome = desktop.to_lowercase().contains("gnome");

    if is_wayland && is_gnome {
        OverlaySupport::FallbackToFullscreen {
            reason: FallbackReason::GnomeWaylandNoLayerShell,
        }
    } else {
        OverlaySupport::Supported
    }
}

// ─── Input passthrough logic ──────────────────────────────────────────────────

/// Decide whether a pointer event at (px, py) should be captured by the
/// runtime or passed through to the desktop.
///
/// Spec line 182: "WHEN runtime is in overlay mode and pointer event lands
/// outside any active hit-region THEN event MUST pass through to underlying
/// desktop."
///
/// In fullscreen mode, all events are always captured.
pub fn should_capture_pointer_event(
    mode: WindowMode,
    px: f32,
    py: f32,
    hit_regions: &[HitRegion],
) -> bool {
    match mode {
        WindowMode::Fullscreen => true,
        WindowMode::Overlay => hit_regions.iter().any(|r| r.contains(px, py)),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── WindowMode ──────────────────────────────────────────────────────────

    #[test]
    fn window_mode_default_is_fullscreen() {
        assert_eq!(WindowMode::default(), WindowMode::Fullscreen);
    }

    #[test]
    fn window_mode_display() {
        assert_eq!(format!("{}", WindowMode::Fullscreen), "fullscreen");
        assert_eq!(format!("{}", WindowMode::Overlay), "overlay");
    }

    // ── HitRegion ────────────────────────────────────────────────────────────

    #[test]
    fn hit_region_contains_interior_point() {
        let r = HitRegion::new(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(60.0, 40.0));
    }

    #[test]
    fn hit_region_does_not_contain_exterior_point() {
        let r = HitRegion::new(10.0, 20.0, 100.0, 50.0);
        assert!(!r.contains(5.0, 40.0));   // left of region
        assert!(!r.contains(115.0, 40.0)); // right of region
        assert!(!r.contains(60.0, 15.0));  // above region
        assert!(!r.contains(60.0, 75.0));  // below region
    }

    #[test]
    fn hit_region_top_left_inclusive() {
        let r = HitRegion::new(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(10.0, 20.0), "top-left corner is inclusive");
    }

    #[test]
    fn hit_region_bottom_right_exclusive() {
        let r = HitRegion::new(10.0, 20.0, 100.0, 50.0);
        // x=110, y=70 are exactly at width/height boundary (exclusive).
        assert!(!r.contains(110.0, 70.0), "boundary is exclusive");
    }

    // ── should_capture_pointer_event ─────────────────────────────────────────

    #[test]
    fn fullscreen_mode_captures_all_events() {
        let regions = vec![HitRegion::new(0.0, 0.0, 10.0, 10.0)];
        // Even outside any hit-region, fullscreen captures everything.
        assert!(should_capture_pointer_event(
            WindowMode::Fullscreen,
            9999.0,
            9999.0,
            &regions
        ));
    }

    #[test]
    fn overlay_mode_captures_event_inside_hit_region() {
        let regions = vec![HitRegion::new(50.0, 50.0, 200.0, 100.0)];
        assert!(should_capture_pointer_event(
            WindowMode::Overlay,
            100.0,
            80.0,
            &regions
        ));
    }

    #[test]
    fn overlay_mode_passes_through_event_outside_hit_region() {
        let regions = vec![HitRegion::new(50.0, 50.0, 200.0, 100.0)];
        // Point at (10, 10) is outside the hit-region — must pass through.
        assert!(!should_capture_pointer_event(
            WindowMode::Overlay,
            10.0,
            10.0,
            &regions
        ));
    }

    #[test]
    fn overlay_mode_no_hit_regions_passes_through_all_events() {
        let regions: Vec<HitRegion> = vec![];
        assert!(!should_capture_pointer_event(
            WindowMode::Overlay,
            100.0,
            100.0,
            &regions
        ));
    }

    #[test]
    fn overlay_mode_multiple_hit_regions_union() {
        let regions = vec![
            HitRegion::new(0.0, 0.0, 100.0, 100.0),
            HitRegion::new(200.0, 200.0, 100.0, 100.0),
        ];
        assert!(should_capture_pointer_event(
            WindowMode::Overlay,
            50.0,
            50.0,
            &regions
        ));
        assert!(should_capture_pointer_event(
            WindowMode::Overlay,
            250.0,
            250.0,
            &regions
        ));
        assert!(!should_capture_pointer_event(
            WindowMode::Overlay,
            150.0,
            150.0,
            &regions
        ));
    }

    // ── WindowConfig ─────────────────────────────────────────────────────────

    #[test]
    fn window_config_default() {
        let cfg = WindowConfig::default();
        assert_eq!(cfg.mode, WindowMode::Fullscreen);
        assert_eq!(cfg.width, 1920);
        assert_eq!(cfg.height, 1080);
    }

    // ── resolve_window_mode ───────────────────────────────────────────────────

    #[test]
    fn resolve_fullscreen_stays_fullscreen() {
        let (mode, reason) = resolve_window_mode(WindowMode::Fullscreen);
        assert_eq!(mode, WindowMode::Fullscreen);
        assert!(reason.is_none());
    }

    // Overlay support varies by environment; we just verify no panic occurs.
    #[test]
    fn resolve_overlay_does_not_panic() {
        let (mode, _reason) = resolve_window_mode(WindowMode::Overlay);
        // Result is either Overlay or Fullscreen-fallback — both are valid.
        assert!(mode == WindowMode::Overlay || mode == WindowMode::Fullscreen);
    }

    // ── FallbackReason display ───────────────────────────────────────────────

    #[test]
    fn fallback_reason_display() {
        let r = FallbackReason::GnomeWaylandNoLayerShell;
        let s = format!("{r}");
        assert!(s.contains("GNOME"));

        let r2 = FallbackReason::Unsupported("test".to_string());
        assert!(format!("{r2}").contains("test"));
    }
}
