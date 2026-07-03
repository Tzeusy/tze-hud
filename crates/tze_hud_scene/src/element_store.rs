//! Persistent element identity store.
//!
//! Pure data model for stable Scene IDs for zones, widgets, and tiles.
//!
//! # Crate-boundary note
//!
//! This module is intentionally I/O-free. TOML serialization, file persistence,
//! atomic writes, and platform-specific file-replace APIs live in
//! `tze_hud_runtime::element_store`, which is the correct layer for that I/O.
//! Callers that need to load or persist the store should use the free functions
//! exposed there:
//!
//! - `tze_hud_runtime::element_store::load_element_store`
//! - `tze_hud_runtime::element_store::persist_element_store_to_path`

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{GeometryPolicy, SceneId};

/// Element category for persistent identity records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ElementType {
    Zone,
    Widget,
    Tile,
}

/// A persisted element identity entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementStoreEntry {
    /// Kind of element this entry represents.
    pub element_type: ElementType,
    /// Stable namespace key (zone_name, widget instance_name, or tile namespace).
    pub namespace: String,
    /// Wall-clock creation time (milliseconds since Unix epoch).
    pub created_at: u64,
    /// Last publish/update timestamp (milliseconds since Unix epoch).
    pub last_published_at: u64,
    /// Optional user geometry override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_override: Option<GeometryPolicy>,
}

/// Container for all persisted element identity entries.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ElementStore {
    /// Entries keyed by stable scene ID.
    #[serde(default)]
    pub entries: HashMap<SceneId, ElementStoreEntry>,
}

impl ElementStore {
    /// Clear the user geometry override for an element, returning the override
    /// that was removed.
    ///
    /// Returns `Some(previous_override)` if an override was present and has been
    /// cleared. Returns `None` if the element has no override or is unknown
    /// (no-op in either case).
    ///
    /// The caller MUST:
    /// 1. Persist the store after a successful clear.
    /// 2. Re-resolve the effective geometry via the fallback chain to populate
    ///    the `new_geometry` field of any outbound `ElementRepositionedEvent`.
    pub fn reset_geometry_override(&mut self, element_id: SceneId) -> Option<GeometryPolicy> {
        let entry = self.entries.get_mut(&element_id)?;
        entry.geometry_override.take()
    }

    /// Set the user geometry override for the entry keyed by `element_id`.
    ///
    /// Unlike [`crate::graph`]-adjacent `persist_geometry_override` (in
    /// `tze_hud_input::drag`), which matches by `(element_type, namespace)` and
    /// writes a single entry, this targets one exact entry by its stable
    /// `SceneId`. Portal members share a namespace but have distinct scene ids,
    /// so a whole-portal resize/move must key each member's durable override by
    /// id — a namespace match would only reach one arbitrary member (hud-8vejp).
    ///
    /// Returns `true` if an entry existed and was updated; `false` if no entry
    /// is keyed by `element_id` (no-op — the element is not yet registered).
    pub fn set_geometry_override(&mut self, element_id: SceneId, geometry: GeometryPolicy) -> bool {
        match self.entries.get_mut(&element_id) {
            Some(entry) => {
                entry.geometry_override = Some(geometry);
                true
            }
            None => false,
        }
    }

    /// Find an entry by `(element_type, namespace)`.
    ///
    /// If duplicates exist, returns the oldest (then lexicographically smallest ID).
    pub fn find_id_by_type_namespace(
        &self,
        element_type: ElementType,
        namespace: &str,
    ) -> Option<SceneId> {
        let mut matches: Vec<(SceneId, &ElementStoreEntry)> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                (entry.element_type == element_type && entry.namespace == namespace)
                    .then_some((*id, entry))
            })
            .collect();

        matches.sort_by_key(|(id, entry)| (entry.created_at, id.to_bytes_le()));
        matches.first().map(|(id, _)| *id)
    }
}

/// Zero-area relative geometry policy used as the final fallback when no
/// configured or agent-supplied geometry can be resolved.
pub const ZERO_GEOMETRY_POLICY: GeometryPolicy = GeometryPolicy::Relative {
    x_pct: 0.0,
    y_pct: 0.0,
    width_pct: 0.0,
    height_pct: 0.0,
};

/// Resolve the fallback geometry for an element after its user override has
/// been cleared (or when no override is present).
///
/// Implements the shared resolution chain used by both the sync Winit-thread
/// path (`perform_reset_element_geometry` in `tze_hud_runtime::windowed`) and
/// the async gRPC path (`HudSessionImpl::reset_element_geometry` in
/// `tze_hud_protocol::session_server`):
///
/// - **Tile**: agent-requested bounds → (no config override) → zero policy.
/// - **Zone**: config policy from `zone_registry` → zero policy.
/// - **Widget**: config policy from `widget_registry` → zero policy.
///
/// Returns [`ZERO_GEOMETRY_POLICY`] when the element is unknown or no registry
/// entry can be found, matching the pre-existing caller behaviour.
pub fn fallback_geometry_for_element(
    element_id: SceneId,
    entry: &ElementStoreEntry,
    scene: &crate::graph::SceneGraph,
) -> GeometryPolicy {
    use crate::types::{rect_to_relative_geometry_policy, resolve_geometry_override_chain};

    match entry.element_type {
        ElementType::Tile => {
            let agent_policy = scene.tiles.get(&element_id).map(|tile| {
                rect_to_relative_geometry_policy(
                    tile.bounds,
                    scene.display_area.width,
                    scene.display_area.height,
                )
            });
            resolve_geometry_override_chain(None, agent_policy, None, None)
                .unwrap_or(ZERO_GEOMETRY_POLICY)
        }
        ElementType::Zone => scene
            .zone_registry
            .resolve_geometry_policy_for_zone(&entry.namespace, None, None)
            .unwrap_or(ZERO_GEOMETRY_POLICY),
        ElementType::Widget => scene
            .widget_registry
            .resolve_geometry_policy_for_instance(&entry.namespace, None)
            .unwrap_or(ZERO_GEOMETRY_POLICY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_store() -> ElementStore {
        let now = 1_710_000_000_000u64;
        let mut entries = HashMap::new();
        entries.insert(
            SceneId::new(),
            ElementStoreEntry {
                element_type: ElementType::Zone,
                namespace: "subtitle".to_string(),
                created_at: now,
                last_published_at: now,
                geometry_override: None,
            },
        );
        entries.insert(
            SceneId::new(),
            ElementStoreEntry {
                element_type: ElementType::Widget,
                namespace: "gauge-main".to_string(),
                created_at: now + 1,
                last_published_at: now + 1,
                geometry_override: Some(GeometryPolicy::Relative {
                    x_pct: 0.1,
                    y_pct: 0.2,
                    width_pct: 0.3,
                    height_pct: 0.4,
                }),
            },
        );
        ElementStore { entries }
    }

    #[test]
    fn find_id_by_type_and_namespace() {
        let store = sample_store();
        let zone_id = store.find_id_by_type_namespace(ElementType::Zone, "subtitle");
        assert!(zone_id.is_some(), "should find zone entry by namespace");

        let widget_id = store.find_id_by_type_namespace(ElementType::Widget, "gauge-main");
        assert!(widget_id.is_some(), "should find widget entry by namespace");

        let missing = store.find_id_by_type_namespace(ElementType::Tile, "not-present");
        assert!(
            missing.is_none(),
            "should return None for missing namespace"
        );
    }

    #[test]
    fn reset_geometry_override_clears_override() {
        let mut store = sample_store();
        let (id, _) = store
            .entries
            .iter()
            .find(|(_, e)| e.geometry_override.is_some())
            .map(|(id, e)| (*id, e.clone()))
            .expect("sample store has a geometry override");

        let removed = store.reset_geometry_override(id);
        assert!(removed.is_some(), "should return the removed override");
        assert!(
            store.entries[&id].geometry_override.is_none(),
            "override should be cleared"
        );
    }

    #[test]
    fn set_geometry_override_updates_only_the_keyed_entry() {
        // Two Tile entries that SHARE a namespace — exactly the portal-member
        // shape: a namespace match would be ambiguous, so the override must be
        // keyed by id (hud-8vejp).
        let now = 1_710_000_000_000u64;
        let a = SceneId::new();
        let b = SceneId::new();
        let mut entries = HashMap::new();
        for id in [a, b] {
            entries.insert(
                id,
                ElementStoreEntry {
                    element_type: ElementType::Tile,
                    namespace: "portal".to_string(),
                    created_at: now,
                    last_published_at: now,
                    geometry_override: None,
                },
            );
        }
        let mut store = ElementStore { entries };

        let policy = GeometryPolicy::Relative {
            x_pct: 0.1,
            y_pct: 0.2,
            width_pct: 0.3,
            height_pct: 0.4,
        };
        assert!(
            store.set_geometry_override(a, policy),
            "setting an override on a known id returns true"
        );
        assert_eq!(
            store.entries[&a].geometry_override,
            Some(policy),
            "the keyed entry gets the override"
        );
        assert!(
            store.entries[&b].geometry_override.is_none(),
            "the sibling sharing the namespace is untouched"
        );

        // Unknown id is a no-op.
        assert!(
            !store.set_geometry_override(SceneId::new(), policy),
            "setting an override on an unknown id returns false"
        );
    }
}
