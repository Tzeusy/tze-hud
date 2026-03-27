//! # tze_hud_a11y
//!
//! Accessibility bridge for tze_hud. Converts the scene graph into a
//! platform-native accessibility tree and exposes screen-reader hooks.
//!
//! ## Architecture
//!
//! The a11y bridge subscribes to scene graph changes and maintains a
//! platform-specific tree, updated within 100ms of any scene change
//! (DR-I6). It runs on the main thread, updated during:
//! - Stage 2 (Local Feedback): focus changes
//! - Stage 4 (Scene Commit): content changes
//!
//! ## Platform bridges (all stubs in v1)
//!
//! - **Linux**: AT-SPI2 via D-Bus (`atspi` module)
//! - **Windows**: UI Automation / IAccessible2 (`uia` module)
//! - **macOS**: NSAccessibility protocol (`nsaccessibility` module)
//! - **Headless/CI**: No-op (`noop` module — the default)
//!
//! ## References
//! - RFC 0004 §5 (Accessibility)
//! - [presence.md](../../heart-and-soul/presence.md) for a11y hooks

pub mod noop;

#[cfg(target_os = "linux")]
pub mod atspi;

#[cfg(target_os = "windows")]
pub mod uia;

#[cfg(target_os = "macos")]
pub mod nsaccessibility;

use serde::{Deserialize, Serialize};
use tze_hud_scene::{SceneGraph, SceneId};

// ─── Shared stub helper ───────────────────────────────────────────────────────

/// One-shot warning emitter for stub platform bridges.
///
/// Each platform stub carries one `WarnOnce` instance and calls `call()` with
/// its own message on the first operation. Subsequent calls are no-ops.
/// This removes the duplicated `warned: bool` + `warn_once()` pattern from
/// every platform module.
pub struct WarnOnce {
    warned: bool,
}

impl WarnOnce {
    pub const fn new() -> Self {
        Self { warned: false }
    }

    /// Emit `message` via `tracing::warn!` exactly once. Subsequent calls are
    /// no-ops.
    pub fn call(&mut self, message: &str) {
        if !self.warned {
            tracing::warn!("{}", message);
            self.warned = true;
        }
    }
}

// ─── Accessibility Metadata ───────────────────────────────────────────────────

/// Per-node and per-tile accessibility metadata declared by agents.
///
/// Mirrors `AccessibilityConfig` from RFC 0004 §5.4 (protobuf definition).
/// Agents attach this to tiles and nodes; the runtime bridges it to the
/// platform a11y API without inferring semantics from content.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityConfig {
    /// Human-readable label. Required for interactive elements.
    pub label: String,

    /// Override the default role mapping.
    /// Accepted values mirror ARIA roles: `"button"`, `"link"`, `"menuitem"`,
    /// `"tab"`, `"region"`, `"feed"`, `"article"`, `"image"`, `"staticText"`.
    /// Empty string means "use default mapping from scene element type".
    pub role_hint: String,

    /// Longer description for screen reader detail mode.
    pub description: String,

    /// When `true`, content changes on this node/tile are announced to the
    /// screen reader (equivalent to `aria-live`).
    pub live: bool,

    /// Announcement politeness when `live` is true.
    pub live_politeness: LivePoliteness,
}

/// Screen reader announcement politeness level.
///
/// Matches RFC 0004 §5.4 `LivePoliteness` enum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LivePoliteness {
    /// Append to announcement queue; read after current speech finishes.
    #[default]
    Polite,
    /// Interrupt current speech immediately.
    Assertive,
    /// No announcement (equivalent to `aria-live="off"`).
    Off,
}

// ─── Core Trait ──────────────────────────────────────────────────────────────

/// Platform-independent interface for the a11y bridge.
///
/// Implementors maintain a live platform-specific accessibility tree derived
/// from the scene graph. The trait is intentionally coarse — the bridge owns
/// the mapping details; callers only push scene state and lifecycle events.
///
/// # Threading
/// Implementations are called from the main thread only (Stage 2 and Stage 4
/// of the frame pipeline). No `Send` or `Sync` is required.
pub trait AccessibilityTree {
    /// Rebuild the a11y tree from the current scene state.
    ///
    /// Called during Stage 4 (Scene Commit) after any scene mutation.
    /// Must complete within 100ms (DR-I6).
    fn update_from_scene(&mut self, scene: &SceneGraph);

    /// Queue a screen reader announcement.
    ///
    /// - `Polite` announcements are appended to the queue.
    /// - `Assertive` announcements interrupt current speech.
    /// - Rate-limited: at most one assertive per 500ms (RFC 0004 §5.5).
    fn announce(&mut self, message: &str, politeness: LivePoliteness);

    /// Notify the a11y bridge that focus moved to a different scene node.
    ///
    /// Called during Stage 2 (Local Feedback) so focus is reported in the
    /// same frame as the event that caused the focus transfer.
    fn focus_changed(&mut self, node_id: SceneId);
}

// ─── Re-exports ───────────────────────────────────────────────────────────────

pub use noop::NoopAccessibility;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::SceneGraph;

    fn make_tree() -> impl AccessibilityTree {
        NoopAccessibility::new()
    }

    #[test]
    fn noop_update_from_scene_does_not_panic() {
        let mut tree = make_tree();
        let scene = SceneGraph::new(1920.0, 1080.0);
        tree.update_from_scene(&scene);
    }

    #[test]
    fn noop_announce_does_not_panic() {
        let mut tree = make_tree();
        tree.announce("Hello", LivePoliteness::Polite);
        tree.announce("Urgent", LivePoliteness::Assertive);
        tree.announce("Silent", LivePoliteness::Off);
    }

    #[test]
    fn noop_focus_changed_does_not_panic() {
        let mut tree = make_tree();
        let id = SceneId::new();
        tree.focus_changed(id);
    }

    #[test]
    fn accessibility_config_defaults() {
        let cfg = AccessibilityConfig::default();
        assert!(cfg.label.is_empty());
        assert!(cfg.role_hint.is_empty());
        assert!(!cfg.live);
        assert_eq!(cfg.live_politeness, LivePoliteness::Polite);
    }

    #[test]
    fn noop_accepts_many_updates() {
        let mut tree = make_tree();
        let scene = SceneGraph::new(1920.0, 1080.0);
        for _ in 0..100 {
            tree.update_from_scene(&scene);
        }
    }
}
