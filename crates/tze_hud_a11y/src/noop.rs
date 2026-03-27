//! No-op accessibility implementation.
//!
//! `NoopAccessibility` silently discards all a11y operations. It is the
//! default implementation used in headless environments and CI where there is
//! no platform accessibility API available.
//!
//! This is the correct choice when:
//! - Running tests that only care about scene graph semantics.
//! - Running in a headless compositor (no display server).
//! - The host OS has no supported a11y API.

use tracing::trace;
use tze_hud_scene::{SceneGraph, SceneId};

use crate::{AccessibilityTree, LivePoliteness};

/// No-op accessibility bridge. All operations are accepted and silently
/// discarded. Useful for headless CI and test environments.
pub struct NoopAccessibility {
    /// Count of `update_from_scene` calls (for diagnostics / tracing only).
    update_count: u64,
}

impl NoopAccessibility {
    /// Create a new no-op bridge.
    pub fn new() -> Self {
        Self { update_count: 0 }
    }

    /// Number of scene updates received (informational; not observable via the
    /// trait interface).
    pub fn update_count(&self) -> u64 {
        self.update_count
    }
}

impl Default for NoopAccessibility {
    fn default() -> Self {
        Self::new()
    }
}

impl AccessibilityTree for NoopAccessibility {
    fn update_from_scene(&mut self, _scene: &SceneGraph) {
        self.update_count += 1;
        trace!(
            update_count = self.update_count,
            "noop a11y: update_from_scene"
        );
    }

    fn announce(&mut self, message: &str, politeness: LivePoliteness) {
        trace!(?politeness, message, "noop a11y: announce (discarded)");
    }

    fn focus_changed(&mut self, node_id: SceneId) {
        trace!(%node_id, "noop a11y: focus_changed (discarded)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::SceneGraph;

    #[test]
    fn update_count_increments() {
        let mut noop = NoopAccessibility::new();
        assert_eq!(noop.update_count(), 0);
        noop.update_from_scene(&SceneGraph::new(1920.0, 1080.0));
        assert_eq!(noop.update_count(), 1);
        noop.update_from_scene(&SceneGraph::new(1920.0, 1080.0));
        assert_eq!(noop.update_count(), 2);
    }

    #[test]
    fn default_is_equivalent_to_new() {
        let a = NoopAccessibility::new();
        let b = NoopAccessibility::default();
        assert_eq!(a.update_count(), b.update_count());
    }
}
