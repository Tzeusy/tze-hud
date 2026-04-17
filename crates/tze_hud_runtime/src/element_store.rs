//! Runtime bootstrap and reconciliation for persistent element IDs.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tze_hud_scene::element_store::{ElementStore, ElementStoreEntry, ElementType};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::SceneId;

/// Result of element-store startup bootstrap.
#[derive(Clone, Debug)]
pub struct ElementStoreBootstrap {
    pub path: PathBuf,
    pub store: ElementStore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorePlatform {
    Windows,
    Linux,
    MacOs,
}

/// Resolve the runtime path for `element_store.toml`.
///
/// Environment override:
/// - `TZE_HUD_ELEMENT_STORE_PATH`
pub fn resolve_element_store_path() -> PathBuf {
    if let Some(override_path) = env_var("TZE_HUD_ELEMENT_STORE_PATH") {
        return PathBuf::from(override_path);
    }

    let platform = if cfg!(target_os = "windows") {
        StorePlatform::Windows
    } else if cfg!(target_os = "macos") {
        StorePlatform::MacOs
    } else {
        StorePlatform::Linux
    };

    resolve_element_store_path_for(platform, &|key| env_var(key))
}

/// Load, reconcile, and persist the element store for the given scene.
pub fn bootstrap_scene_element_store(scene: &mut SceneGraph) -> ElementStoreBootstrap {
    bootstrap_scene_element_store_with_path(scene, resolve_element_store_path())
}

/// Same as [`bootstrap_scene_element_store`], but with an explicit path.
pub fn bootstrap_scene_element_store_with_path(
    scene: &mut SceneGraph,
    path: PathBuf,
) -> ElementStoreBootstrap {
    let existed = path.exists();
    let mut store = match ElementStore::load_or_default(&path) {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "element_store: failed to load store, continuing with empty store"
            );
            ElementStore::default()
        }
    };

    let now_ms = now_wall_ms();
    let changed = reconcile_scene_ids(scene, &mut store, now_ms);
    if changed || !existed {
        if let Err(err) = store.persist_to_path_atomic(&path) {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "element_store: failed to persist reconciled store"
            );
        }
    }

    ElementStoreBootstrap { path, store }
}

/// Reconcile persisted IDs with startup-registered zones and widgets.
///
/// Matching keys:
/// - zone: `zone.name`
/// - widget: `instance.instance_name`
pub fn reconcile_scene_ids(scene: &mut SceneGraph, store: &mut ElementStore, now_ms: u64) -> bool {
    let mut changed = false;

    for zone in scene.zone_registry.zones.values_mut() {
        let resolved_id = match store.find_id_by_type_namespace(ElementType::Zone, &zone.name) {
            Some(id) => id,
            None => {
                if zone.id.is_null() {
                    zone.id = SceneId::new();
                    changed = true;
                }
                zone.id
            }
        };

        if zone.id != resolved_id {
            zone.id = resolved_id;
            changed = true;
        }

        if ensure_entry(
            store,
            resolved_id,
            ElementType::Zone,
            zone.name.clone(),
            now_ms,
        ) {
            changed = true;
        }
    }

    for instance in scene.widget_registry.instances.values_mut() {
        let resolved_id =
            match store.find_id_by_type_namespace(ElementType::Widget, &instance.instance_name) {
                Some(id) => id,
                None => {
                    if instance.id.is_null() {
                        instance.id = SceneId::new();
                        changed = true;
                    }
                    instance.id
                }
            };

        if instance.id != resolved_id {
            instance.id = resolved_id;
            changed = true;
        }

        if ensure_entry(
            store,
            resolved_id,
            ElementType::Widget,
            instance.instance_name.clone(),
            now_ms,
        ) {
            changed = true;
        }
    }

    changed
}

fn ensure_entry(
    store: &mut ElementStore,
    id: SceneId,
    element_type: ElementType,
    namespace: String,
    now_ms: u64,
) -> bool {
    match store.entries.get_mut(&id) {
        Some(entry) => {
            let mut changed = false;
            if entry.element_type != element_type {
                entry.element_type = element_type;
                changed = true;
            }
            if entry.namespace != namespace {
                entry.namespace = namespace;
                changed = true;
            }
            if entry.created_at == 0 {
                entry.created_at = now_ms;
                changed = true;
            }
            if entry.last_published_at == 0 {
                entry.last_published_at = now_ms;
                changed = true;
            }
            changed
        }
        None => {
            store.entries.insert(
                id,
                ElementStoreEntry {
                    element_type,
                    namespace,
                    created_at: now_ms,
                    last_published_at: now_ms,
                    geometry_override: None,
                },
            );
            true
        }
    }
}

fn env_var(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn now_wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn resolve_element_store_path_for<F>(platform: StorePlatform, env_lookup: &F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    let dir = match platform {
        StorePlatform::Windows => env_lookup("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("tze_hud"),
        StorePlatform::MacOs => env_lookup("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Library")
            .join("Application Support")
            .join("tze_hud"),
        StorePlatform::Linux => {
            if let Some(xdg_data_home) = env_lookup("XDG_DATA_HOME") {
                PathBuf::from(xdg_data_home).join("tze_hud")
            } else if let Some(home) = env_lookup("HOME") {
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("tze_hud")
            } else {
                std::env::temp_dir().join("tze_hud")
            }
        }
    };
    dir.join("element_store.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;
    use tze_hud_scene::types::{
        ContentionPolicy, GeometryPolicy, RenderingPolicy, WidgetInstance, WidgetParameterValue,
        ZoneDefinition, ZoneMediaType,
    };

    fn make_scene_with_zone_and_widget() -> SceneGraph {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        let tab_id = scene.create_tab("main", 0).expect("tab");
        scene.zone_registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_string(),
            description: "subtitle zone".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.9,
                width_pct: 1.0,
                height_pct: 0.1,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: Default::default(),
        });

        scene.widget_registry.register_instance(WidgetInstance {
            id: SceneId::new(),
            widget_type_name: "gauge".to_string(),
            tab_id,
            geometry_override: None,
            contention_override: None,
            instance_name: "gauge-main".to_string(),
            current_params: HashMap::<String, WidgetParameterValue>::new(),
        });

        scene
    }

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tze_hud_runtime_element_store_{name}_{}",
            SceneId::new()
        ))
    }

    #[test]
    fn windows_path_uses_localappdata() {
        let path = resolve_element_store_path_for(StorePlatform::Windows, &|key| {
            (key == "LOCALAPPDATA").then_some("C:\\Users\\Alice\\AppData\\Local".to_string())
        });
        assert_eq!(
            path,
            Path::new("C:\\Users\\Alice\\AppData\\Local")
                .join("tze_hud")
                .join("element_store.toml")
        );
    }

    #[test]
    fn linux_path_prefers_xdg_data_home() {
        let path = resolve_element_store_path_for(StorePlatform::Linux, &|key| {
            (key == "XDG_DATA_HOME").then_some("/home/alice/.data".to_string())
        });
        assert_eq!(
            path,
            Path::new("/home/alice/.data")
                .join("tze_hud")
                .join("element_store.toml")
        );
    }

    #[test]
    fn linux_path_falls_back_to_home_local_share() {
        let path = resolve_element_store_path_for(StorePlatform::Linux, &|key| {
            (key == "HOME").then_some("/home/alice".to_string())
        });
        assert_eq!(
            path,
            Path::new("/home/alice")
                .join(".local")
                .join("share")
                .join("tze_hud")
                .join("element_store.toml")
        );
    }

    #[test]
    fn macos_path_uses_application_support() {
        let path = resolve_element_store_path_for(StorePlatform::MacOs, &|key| {
            (key == "HOME").then_some("/Users/alice".to_string())
        });
        assert_eq!(
            path,
            Path::new("/Users/alice")
                .join("Library")
                .join("Application Support")
                .join("tze_hud")
                .join("element_store.toml")
        );
    }

    #[test]
    fn reconcile_reuses_existing_zone_and_widget_ids() {
        let mut scene = make_scene_with_zone_and_widget();
        let desired_zone_id = SceneId::new();
        let desired_widget_id = SceneId::new();
        let mut store = ElementStore::default();
        store.entries.insert(
            desired_zone_id,
            ElementStoreEntry {
                element_type: ElementType::Zone,
                namespace: "subtitle".to_string(),
                created_at: 100,
                last_published_at: 200,
                geometry_override: None,
            },
        );
        store.entries.insert(
            desired_widget_id,
            ElementStoreEntry {
                element_type: ElementType::Widget,
                namespace: "gauge-main".to_string(),
                created_at: 100,
                last_published_at: 200,
                geometry_override: None,
            },
        );

        let changed = reconcile_scene_ids(&mut scene, &mut store, 300);
        assert!(changed, "scene IDs should be rewritten to persisted IDs");
        assert_eq!(scene.zone_registry.zones["subtitle"].id, desired_zone_id);
        assert_eq!(
            scene.widget_registry.instances["gauge-main"].id,
            desired_widget_id
        );
    }

    #[test]
    fn bootstrap_round_trip_reuses_ids_across_restart() {
        let path = test_path("restart");
        let _ = std::fs::remove_file(&path);

        let mut first_scene = make_scene_with_zone_and_widget();
        let first = bootstrap_scene_element_store_with_path(&mut first_scene, path.clone());
        assert!(
            !first.store.entries.is_empty(),
            "startup must persist zone/widget identities"
        );
        let first_zone_id = first_scene.zone_registry.zones["subtitle"].id;
        let first_widget_id = first_scene.widget_registry.instances["gauge-main"].id;

        let mut second_scene = make_scene_with_zone_and_widget();
        let _second = bootstrap_scene_element_store_with_path(&mut second_scene, path.clone());
        assert_eq!(
            second_scene.zone_registry.zones["subtitle"].id,
            first_zone_id
        );
        assert_eq!(
            second_scene.widget_registry.instances["gauge-main"].id,
            first_widget_id
        );

        let _ = std::fs::remove_file(path);
    }
}
