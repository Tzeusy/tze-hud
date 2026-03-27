//! Linux AT-SPI2 accessibility bridge (stub).
//!
//! AT-SPI2 (Assistive Technology Service Provider Interface 2) is the
//! standard Linux accessibility API, used by Orca screen reader, GNOME
//! accessibility tools, and others. It communicates over D-Bus.
//!
//! # Status
//! **STUB** — trait implemented but all operations are no-ops. Full
//! implementation requires:
//! - `atspi` crate (or `zbus` for raw D-Bus)
//! - Registration as an AT-SPI2 application
//! - Exposing `org.a11y.atspi.Accessible` and related interfaces
//! - Emitting `StateChanged`, `ObjectPropertyChange`, `ChildrenChanged` signals
//!
//! # References
//! - RFC 0004 §5.8 (Platform A11y API Integration)
//! - https://gitlab.gnome.org/GNOME/at-spi2-core

#![cfg(target_os = "linux")]

use tze_hud_scene::{SceneGraph, SceneId};

use crate::{AccessibilityTree, LivePoliteness, WarnOnce};

const STUB_MSG: &str = "tze_hud_a11y: AT-SPI2 bridge is a stub — accessibility features are \
    not functional. Implement crates/tze_hud_a11y/src/atspi.rs to enable \
    Linux screen reader support.";

/// AT-SPI2 accessibility bridge for Linux.
///
/// Stub implementation. Emits a warning on first use to remind implementors
/// that the bridge is not yet wired to the actual D-Bus AT-SPI2 stack.
pub struct AtspiAccessibility {
    warner: WarnOnce,
}

impl AtspiAccessibility {
    /// Create a new AT-SPI2 bridge stub.
    pub fn new() -> Self {
        Self {
            warner: WarnOnce::new(),
        }
    }
}

impl Default for AtspiAccessibility {
    fn default() -> Self {
        Self::new()
    }
}

impl AccessibilityTree for AtspiAccessibility {
    fn update_from_scene(&mut self, _scene: &SceneGraph) {
        self.warner.call(STUB_MSG);
        // TODO: diff scene graph → emit AT-SPI2 ChildrenChanged / ObjectPropertyChange signals
    }

    fn announce(&mut self, _message: &str, _politeness: LivePoliteness) {
        self.warner.call(STUB_MSG);
        // TODO: call org.a11y.atspi.Event.Object:Announcement (AT-SPI2 live region)
    }

    fn focus_changed(&mut self, _node_id: SceneId) {
        self.warner.call(STUB_MSG);
        // TODO: emit AT-SPI2 StateChanged:focused signal for the corresponding accessible object
    }
}
