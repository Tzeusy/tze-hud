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
    /// Tile z-order at creation, used as the stable per-member key when
    /// re-homing a durable override onto a portal member tile that was recreated
    /// with a fresh `SceneId` after a runtime restart (hud-08nls).
    ///
    /// A text-stream portal is N tiles that share one namespace, so namespace
    /// alone cannot map a recreated member back to its prior override. The
    /// adapter assigns each member a stable z-order (its role within the group),
    /// so `(namespace, z_order)` identifies the member across restarts. `0` for
    /// zones/widgets (which reconcile by name/instance_name) and for entries
    /// persisted before this field existed.
    #[serde(default)]
    pub z_order: u32,
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

/// A freshly created tile awaiting durable-override adoption (hud-08nls).
///
/// Passed to [`ElementStore::adopt_orphaned_tile_overrides`] so a portal member
/// recreated with a fresh `SceneId` can reclaim the override its predecessor
/// held under its dead id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecreatedTile {
    /// The tile's newly assigned scene id.
    pub id: SceneId,
    /// The tile's namespace (shared across a portal's member tiles).
    pub namespace: String,
    /// The tile's z-order (its stable role within the portal group).
    pub z_order: u32,
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

    /// Re-home durable geometry overrides from orphaned (dead-id) tile entries
    /// onto freshly recreated portal member tiles (hud-08nls).
    ///
    /// # Why this exists
    ///
    /// `set_geometry_override` (hud-8vejp) keys each portal member's durable
    /// override by the member tile's `SceneId`. But portal projection tiles are
    /// recreated with *fresh* `SceneId`s whenever an adapter republishes after a
    /// runtime restart (the adapter's in-memory tile id is gone, so its first
    /// republish issues a new `CreateTile`). `reconcile_scene_ids` only maps
    /// zones/widgets back to their persisted id — never tiles — so without this
    /// step the override loaded from disk stays keyed by the dead id and is
    /// silently dropped, defeating the restart-durability #1012 added.
    ///
    /// # Matching
    ///
    /// A text-stream portal is N tiles that share one namespace, so namespace
    /// alone is ambiguous. Members are matched by `(namespace, z_order)` — the
    /// adapter assigns each member a stable z-order (its role in the group).
    /// Any override-bearing orphans that do not match a recreated member's
    /// z-order (e.g. entries persisted before `z_order` was recorded, defaulting
    /// to `0`) are paired with the remaining recreated members in a stable order
    /// so a legacy store still reconciles.
    ///
    /// An orphan is a stored `Tile` entry that carries an override and whose id
    /// is **not** in `live_ids` (the ids currently present in the scene). A live
    /// sibling's override is therefore never stolen.
    ///
    /// Returns the recreated tile ids that adopted an override; the caller
    /// re-locks their viewer geometry and persists the store. Consumed orphan
    /// entries are removed.
    pub fn adopt_orphaned_tile_overrides(
        &mut self,
        recreated: &[RecreatedTile],
        live_ids: &std::collections::HashSet<SceneId>,
    ) -> Vec<SceneId> {
        // Only namespaces that actually gained a recreated tile can adopt.
        let mut namespaces: Vec<&str> = recreated.iter().map(|t| t.namespace.as_str()).collect();
        namespaces.sort_unstable();
        namespaces.dedup();

        let recreated_ids: std::collections::HashSet<SceneId> =
            recreated.iter().map(|t| t.id).collect();

        let mut adopted = Vec::new();

        for namespace in namespaces {
            // Recreated members for this namespace that do not yet carry an
            // override (a fresh CreateTile always inserts `geometry_override:
            // None`), sorted by z-order then id for a deterministic fallback.
            let mut members: Vec<RecreatedTile> = recreated
                .iter()
                .filter(|t| t.namespace == namespace)
                .filter(|t| {
                    self.entries
                        .get(&t.id)
                        .is_none_or(|e| e.geometry_override.is_none())
                })
                .cloned()
                .collect();
            members.sort_by_key(|t| (t.z_order, t.id.to_bytes_le()));

            // Override-bearing orphans for this namespace: dead ids (not live,
            // not among the recreated set). Sorted by created_at then id so the
            // fallback pairing is stable and matches creation order.
            let mut orphans: Vec<(SceneId, u32)> = self
                .entries
                .iter()
                .filter(|(id, entry)| {
                    entry.element_type == ElementType::Tile
                        && entry.namespace == namespace
                        && entry.geometry_override.is_some()
                        && !live_ids.contains(*id)
                        && !recreated_ids.contains(*id)
                })
                .map(|(id, entry)| (*id, entry.z_order))
                .collect();
            orphans.sort_by_key(|(id, _)| {
                let created_at = self.entries.get(id).map(|e| e.created_at).unwrap_or(0);
                (created_at, id.to_bytes_le())
            });

            // Pass 1: exact (namespace, z_order) matches. Robust when only some
            // members were moved (sparse overrides).
            let mut consumed: std::collections::HashSet<SceneId> = std::collections::HashSet::new();
            let mut unmatched_members: Vec<RecreatedTile> = Vec::new();
            for member in members {
                let orphan = orphans
                    .iter()
                    .find(|(id, z)| *z == member.z_order && !consumed.contains(id))
                    .map(|(id, _)| *id);
                match orphan {
                    Some(orphan_id) => {
                        consumed.insert(orphan_id);
                        if self.migrate_override(orphan_id, member.id) {
                            adopted.push(member.id);
                        }
                    }
                    None => unmatched_members.push(member),
                }
            }

            // Pass 2: pair any members left unmatched (e.g. legacy z_order == 0
            // orphans) with the remaining orphans in stable order.
            let mut remaining_orphans = orphans
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| !consumed.contains(id));
            for member in unmatched_members {
                if let Some(orphan_id) = remaining_orphans.next() {
                    if self.migrate_override(orphan_id, member.id) {
                        adopted.push(member.id);
                    }
                }
            }
        }

        adopted
    }

    /// Move the durable override from `orphan_id` to `target_id`, removing the
    /// orphan entry. Returns `true` if an override was migrated.
    fn migrate_override(&mut self, orphan_id: SceneId, target_id: SceneId) -> bool {
        let Some(override_policy) = self
            .entries
            .get(&orphan_id)
            .and_then(|e| e.geometry_override)
        else {
            return false;
        };
        if let Some(target) = self.entries.get_mut(&target_id) {
            target.geometry_override = Some(override_policy);
            self.entries.remove(&orphan_id);
            true
        } else {
            false
        }
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
                z_order: 0,
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
                z_order: 0,
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
                    z_order: 0,
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

    // ── adopt_orphaned_tile_overrides (hud-08nls) ─────────────────────────

    fn tile_entry(namespace: &str, z_order: u32, created_at: u64) -> ElementStoreEntry {
        ElementStoreEntry {
            element_type: ElementType::Tile,
            namespace: namespace.to_string(),
            created_at,
            last_published_at: created_at,
            z_order,
            geometry_override: None,
        }
    }

    fn rel(x: f32) -> GeometryPolicy {
        GeometryPolicy::Relative {
            x_pct: x,
            y_pct: x,
            width_pct: 0.5,
            height_pct: 0.5,
        }
    }

    #[test]
    fn adopt_rehomes_single_member_override_to_recreated_tile() {
        // A one-tile portal: the old tile id is dead (restart), a fresh tile is
        // created sharing the namespace. The orphaned override must migrate.
        let old_id = SceneId::new();
        let new_id = SceneId::new();
        let mut store = ElementStore::default();
        let mut orphan = tile_entry("agent.portal", 5, 100);
        orphan.geometry_override = Some(rel(0.7));
        store.entries.insert(old_id, orphan);
        store
            .entries
            .insert(new_id, tile_entry("agent.portal", 5, 200));

        let recreated = vec![RecreatedTile {
            id: new_id,
            namespace: "agent.portal".to_string(),
            z_order: 5,
        }];
        let live: std::collections::HashSet<SceneId> = [new_id].into_iter().collect();

        let adopted = store.adopt_orphaned_tile_overrides(&recreated, &live);

        assert_eq!(
            adopted,
            vec![new_id],
            "the recreated tile adopts the override"
        );
        assert_eq!(
            store.entries[&new_id].geometry_override,
            Some(rel(0.7)),
            "override is re-applied to the fresh tile, not orphaned"
        );
        assert!(
            !store.entries.contains_key(&old_id),
            "the consumed orphan entry is removed"
        );
    }

    #[test]
    fn adopt_matches_members_by_z_order_not_position() {
        // Two members share the namespace; each has a distinct override. They
        // must be paired by z_order so the geometries are not swapped.
        let old_frame = SceneId::new();
        let old_composer = SceneId::new();
        let new_frame = SceneId::new();
        let new_composer = SceneId::new();

        let mut store = ElementStore::default();
        let mut frame = tile_entry("agent.portal", 10, 100);
        frame.geometry_override = Some(rel(0.1));
        let mut composer = tile_entry("agent.portal", 20, 101);
        composer.geometry_override = Some(rel(0.2));
        store.entries.insert(old_frame, frame);
        store.entries.insert(old_composer, composer);
        // Fresh tiles inserted with NO override (as a real CreateTile would).
        store
            .entries
            .insert(new_frame, tile_entry("agent.portal", 10, 300));
        store
            .entries
            .insert(new_composer, tile_entry("agent.portal", 20, 300));

        let recreated = vec![
            RecreatedTile {
                id: new_frame,
                namespace: "agent.portal".to_string(),
                z_order: 10,
            },
            RecreatedTile {
                id: new_composer,
                namespace: "agent.portal".to_string(),
                z_order: 20,
            },
        ];
        let live: std::collections::HashSet<SceneId> =
            [new_frame, new_composer].into_iter().collect();

        let adopted = store.adopt_orphaned_tile_overrides(&recreated, &live);

        assert_eq!(adopted.len(), 2, "both members adopt an override");
        assert_eq!(
            store.entries[&new_frame].geometry_override,
            Some(rel(0.1)),
            "frame (z=10) keeps the frame override"
        );
        assert_eq!(
            store.entries[&new_composer].geometry_override,
            Some(rel(0.2)),
            "composer (z=20) keeps the composer override — not swapped"
        );
        assert!(!store.entries.contains_key(&old_frame));
        assert!(!store.entries.contains_key(&old_composer));
    }

    #[test]
    fn adopt_falls_back_to_stable_order_for_legacy_zero_z_order() {
        // Entries persisted before z_order existed default to 0, so the exact
        // (namespace, z_order) match fails against a real z-order. The order
        // fallback still reconciles a single-member portal.
        let old_id = SceneId::new();
        let new_id = SceneId::new();
        let mut store = ElementStore::default();
        let mut orphan = tile_entry("agent.portal", 0, 100); // legacy z_order
        orphan.geometry_override = Some(rel(0.4));
        store.entries.insert(old_id, orphan);
        store
            .entries
            .insert(new_id, tile_entry("agent.portal", 7, 200));

        let recreated = vec![RecreatedTile {
            id: new_id,
            namespace: "agent.portal".to_string(),
            z_order: 7,
        }];
        let live: std::collections::HashSet<SceneId> = [new_id].into_iter().collect();

        let adopted = store.adopt_orphaned_tile_overrides(&recreated, &live);

        assert_eq!(adopted, vec![new_id]);
        assert_eq!(store.entries[&new_id].geometry_override, Some(rel(0.4)));
    }

    #[test]
    fn adopt_never_steals_a_live_siblings_override() {
        // Sibling A is still live and owns an override; member B was recreated.
        // A's override must not be migrated onto B.
        let live_a = SceneId::new();
        let new_b = SceneId::new();
        let mut store = ElementStore::default();
        let mut a = tile_entry("agent.portal", 10, 100);
        a.geometry_override = Some(rel(0.9));
        store.entries.insert(live_a, a);
        store
            .entries
            .insert(new_b, tile_entry("agent.portal", 20, 200));

        let recreated = vec![RecreatedTile {
            id: new_b,
            namespace: "agent.portal".to_string(),
            z_order: 20,
        }];
        // Both A and B are live; only B is "recreated".
        let live: std::collections::HashSet<SceneId> = [live_a, new_b].into_iter().collect();

        let adopted = store.adopt_orphaned_tile_overrides(&recreated, &live);

        assert!(adopted.is_empty(), "no orphan to adopt — A is live");
        assert_eq!(
            store.entries[&live_a].geometry_override,
            Some(rel(0.9)),
            "the live sibling keeps its override"
        );
        assert!(store.entries[&new_b].geometry_override.is_none());
    }

    #[test]
    fn adopt_is_a_noop_without_orphans() {
        let new_id = SceneId::new();
        let mut store = ElementStore::default();
        store
            .entries
            .insert(new_id, tile_entry("agent.portal", 5, 200));
        let recreated = vec![RecreatedTile {
            id: new_id,
            namespace: "agent.portal".to_string(),
            z_order: 5,
        }];
        let live: std::collections::HashSet<SceneId> = [new_id].into_iter().collect();

        let adopted = store.adopt_orphaned_tile_overrides(&recreated, &live);
        assert!(adopted.is_empty());
        assert!(store.entries[&new_id].geometry_override.is_none());
    }
}
