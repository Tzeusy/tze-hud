//! macOS NSAccessibility bridge (stub).
//!
//! NSAccessibility is the macOS accessibility protocol. Elements adopt the
//! `NSAccessibilityElement` protocol and publish themselves via
//! `accessibilityChildren`, `accessibilityRole`, `accessibilityLabel`, etc.
//! VoiceOver and Switch Control consume this tree.
//!
//! # Status
//! **STUB** — trait implemented but all operations are no-ops. Full
//! implementation requires:
//! - macOS platform
//! - Rust ↔ Objective-C bridge (e.g. `objc2` crate)
//! - Implementing the `NSAccessibilityElement` informal protocol for each node
//! - Posting `NSAccessibilityFocusedUIElementChangedNotification` on focus change
//! - Posting `NSAccessibilityAnnouncementRequestedNotification` for announcements
//!
//! # References
//! - RFC 0004 §5.8 (Platform A11y API Integration)
//! - https://developer.apple.com/documentation/appkit/nsaccessibility

#![cfg(target_os = "macos")]

use tracing::warn;
use tze_hud_scene::{SceneId, SceneGraph};

use crate::{AccessibilityTree, LivePoliteness};

/// macOS NSAccessibility bridge.
///
/// Stub implementation. Emits a warning on first use.
pub struct NsAccessibility {
    warned: bool,
}

impl NsAccessibility {
    /// Create a new NSAccessibility bridge stub.
    pub fn new() -> Self {
        Self { warned: false }
    }

    fn warn_once(&mut self) {
        if !self.warned {
            warn!(
                "tze_hud_a11y: macOS NSAccessibility bridge is a stub — accessibility \
                 features are not functional. Implement \
                 crates/tze_hud_a11y/src/nsaccessibility.rs to enable VoiceOver support."
            );
            self.warned = true;
        }
    }
}

impl Default for NsAccessibility {
    fn default() -> Self {
        Self::new()
    }
}

impl AccessibilityTree for NsAccessibility {
    fn update_from_scene(&mut self, _scene: &SceneGraph) {
        self.warn_once();
        // TODO: rebuild NSAccessibilityElement hierarchy from scene graph;
        //       post NSAccessibilityRowCountChangedNotification if structure changed
    }

    fn announce(&mut self, _message: &str, politeness: LivePoliteness) {
        self.warn_once();
        // TODO: post NSAccessibilityAnnouncementRequestedNotification with
        //       NSAccessibilityPriorityKey set to NSAccessibilityPriorityHigh (assertive)
        //       or NSAccessibilityPriorityMedium (polite)
        let _ = politeness;
    }

    fn focus_changed(&mut self, _node_id: SceneId) {
        self.warn_once();
        // TODO: post NSAccessibilityFocusedUIElementChangedNotification on the
        //       focused element's accessibility object
    }
}
