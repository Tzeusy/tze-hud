//! Windows UI Automation (UIA) accessibility bridge (stub).
//!
//! UI Automation is the modern Windows accessibility API. It exposes an
//! element tree to AT clients (Narrator, NVDA, JAWS, Inspect.exe) via
//! COM interfaces (`IRawElementProviderSimple`, `IAccessible2`).
//!
//! # Status
//! **STUB** — trait implemented but all operations are no-ops. Full
//! implementation requires:
//! - Windows platform and a COM-capable Rust crate (e.g. `windows` crate)
//! - Implementing `IRawElementProviderSimple` for each scene element
//! - Registering the provider tree with the UIA framework
//! - Raising `UIA_AutomationFocusChangedEventId` and
//!   `UIA_LiveRegionChangedEventId` events as appropriate
//!
//! # References
//! - RFC 0004 §5.8 (Platform A11y API Integration)
//! - https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-providersoverview

#![cfg(target_os = "windows")]

use tze_hud_scene::{SceneId, SceneGraph};

use crate::{AccessibilityTree, LivePoliteness, WarnOnce};

const STUB_MSG: &str = "tze_hud_a11y: Windows UI Automation bridge is a stub — accessibility \
    features are not functional. Implement crates/tze_hud_a11y/src/uia.rs \
    to enable Windows screen reader support (Narrator, NVDA, JAWS).";

/// Windows UI Automation accessibility bridge.
///
/// Stub implementation. Emits a warning on first use.
pub struct UiaAccessibility {
    warner: WarnOnce,
}

impl UiaAccessibility {
    /// Create a new UIA bridge stub.
    pub fn new() -> Self {
        Self { warner: WarnOnce::new() }
    }
}

impl Default for UiaAccessibility {
    fn default() -> Self {
        Self::new()
    }
}

impl AccessibilityTree for UiaAccessibility {
    fn update_from_scene(&mut self, _scene: &SceneGraph) {
        self.warner.call(STUB_MSG);
        // TODO: rebuild COM element tree from scene graph, raise
        //       UIA_StructureChangedEventId as needed
    }

    fn announce(&mut self, _message: &str, _politeness: LivePoliteness) {
        self.warner.call(STUB_MSG);
        // TODO: raise UIA_LiveRegionChangedEventId on the appropriate provider
    }

    fn focus_changed(&mut self, _node_id: SceneId) {
        self.warner.call(STUB_MSG);
        // TODO: raise UIA_AutomationFocusChangedEventId on the focused element's provider
    }
}
