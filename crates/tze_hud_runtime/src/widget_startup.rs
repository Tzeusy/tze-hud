//! Widget registry startup integration.
//!
//! This module handles widget system initialization at runtime startup:
//!
//! 1. **Bundle scanning**: scans configured `[widget_bundles].paths` for valid
//!    widget asset bundles.
//! 2. **Definition registration**: loads `WidgetDefinition` entries into the
//!    scene graph's `WidgetRegistry`.
//! 3. **Instance creation**: creates `WidgetInstance` entries for each
//!    `[[tabs.widgets]]` declaration in the config, bound to the appropriate
//!    tab scene IDs.
//!
//! ## Empty registry
//!
//! If no `[widget_bundles]` section exists, the widget registry is left empty
//! (no definitions, no instances). This is valid — the runtime starts normally.
//!
//! ## Bundle errors
//!
//! Per widget-system/spec.md §Widget Asset Bundle Format: "A rejected bundle
//! MUST NOT prevent other valid bundles from loading; the runtime SHALL log
//! the error and continue."  Bundle load errors are logged at WARN level and
//! do not abort startup.
//!
//! ## Spec references
//!
//! - widget-system/spec.md §Requirement: Widget Registry
//! - widget-system/spec.md §Requirement: Widget Asset Bundle Format
//! - widget-system/spec.md §Requirement: Widget Instance Lifecycle
//! - widget-system/spec.md §Requirement: Widget Contention and Governance
//! - configuration/spec.md §Requirement: Widget Bundle Configuration
//! - configuration/spec.md §Requirement: Widget Instance Configuration

use std::collections::HashMap;
use std::path::Path;

use tze_hud_config::raw::RawConfig;
use tze_hud_config::widgets::{LoadedWidgetType, build_widget_instance, resolve_bundle_path};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::SceneId;
use tze_hud_widget::loader::{BundleScanResult, scan_bundle_dirs};

// ─── Public API ───────────────────────────────────────────────────────────────

/// Initialize the widget registry in `scene` from the raw config.
///
/// Called at runtime startup after zone registry initialization.  This function:
///
/// 1. Resolves `[widget_bundles].paths` relative to `config_parent`.
/// 2. Calls `scan_bundle_dirs` to load all valid bundles.
/// 3. Registers each `WidgetDefinition` in `scene.widget_registry`.
/// 4. Pre-creates tabs from config if they do not exist in the scene graph yet
///    (needed to bind widget instances to tab IDs).
/// 5. Creates `WidgetInstance` records for each `[[tabs.widgets]]` entry.
///
/// Errors are logged at WARN level but never abort startup (spec §Widget
/// Asset Bundle Format).
///
/// # Arguments
///
/// - `scene`: The mutable scene graph to populate.
/// - `raw`: Raw (already-validated) config document.
/// - `config_parent`: Parent directory of the config file (for path resolution).
///   Pass `None` to resolve paths relative to the current working directory.
/// - `tab_name_to_id`: Map from tab name string to its `SceneId` in the scene graph.
///   Used to bind widget instances to tabs. When a tab name from config is not
///   found in this map, the function will attempt to pre-create the tab.
pub fn init_widget_registry(
    scene: &mut SceneGraph,
    raw: &RawConfig,
    config_parent: Option<&Path>,
    tab_name_to_id: &HashMap<String, SceneId>,
) {
    let Some(wb) = &raw.widget_bundles else {
        tracing::debug!("widget_startup: no [widget_bundles] section; widget registry empty");
        return;
    };

    if wb.paths.is_empty() {
        tracing::debug!("widget_startup: [widget_bundles].paths is empty; widget registry empty");
        return;
    }

    let base = config_parent.unwrap_or_else(|| Path::new("."));

    // Step 1: Resolve all bundle root paths relative to `config_parent`
    // (uses the shared helper from tze_hud_config::widgets to avoid duplication).
    let bundle_roots: Vec<std::path::PathBuf> = wb
        .paths
        .iter()
        .map(|p| resolve_bundle_path(p, base))
        .collect();

    // Step 2: Scan all bundle directories.
    // Token map is empty until the runtime wires up design-token resolution
    // (see component-shape-language task 10.2).
    let scan_results = scan_bundle_dirs(&bundle_roots, &HashMap::new());

    // Step 3: Register each valid WidgetDefinition.
    // Track registered names to detect cross-dir duplicates (scan_bundle_dirs
    // already handles within-dir duplicates, but we re-check here for safety).
    let mut registered_names: HashMap<String, ()> = HashMap::new();
    let mut type_map: HashMap<String, LoadedWidgetType> = HashMap::new();

    for result in scan_results {
        match result {
            BundleScanResult::Ok(bundle) => {
                let name = bundle.definition.id.clone();
                if registered_names.contains_key(&name) {
                    tracing::warn!(
                        widget_name = %name,
                        "widget_startup: duplicate widget type name across bundle roots; \
                         skipping second occurrence"
                    );
                    continue;
                }
                registered_names.insert(name.clone(), ());

                // Build the LoadedWidgetType entry for instance creation.
                let loaded = LoadedWidgetType {
                    name: name.clone(),
                    parameter_schema: bundle.definition.parameter_schema.clone(),
                    default_geometry_policy: bundle.definition.default_geometry_policy,
                    default_contention_policy: bundle.definition.default_contention_policy,
                };
                type_map.insert(name.clone(), loaded);

                // Register the WidgetDefinition in the scene graph.
                tracing::info!(
                    widget_name = %name,
                    "widget_startup: registered widget type"
                );
                scene.widget_registry.register_definition(bundle.definition);
            }
            BundleScanResult::Err(err) => {
                // Spec: rejected bundle MUST NOT prevent other bundles from loading.
                tracing::warn!(
                    wire_code = err.wire_code(),
                    error = %err,
                    "widget_startup: bundle load error (skipping)"
                );
            }
        }
    }

    tracing::info!(
        widget_types = scene.widget_registry.definitions.len(),
        "widget_startup: widget type registration complete"
    );

    // Step 4: Create widget instances from [[tabs.widgets]] configuration.
    //
    // We need tab SceneIds to bind instances. Widget instances are declared
    // against named tabs from [[tabs]] in config. If the tab already exists in
    // the scene graph (from the caller's tab_name_to_id map), use its ID.
    // Otherwise, pre-create the tab so widget instances can be bound.
    let mut effective_tab_map: HashMap<String, SceneId> = tab_name_to_id.clone();
    let mut total_instances = 0usize;

    for (tab_idx, tab) in raw.tabs.iter().enumerate() {
        let tab_name = tab.name.as_deref().unwrap_or("<unnamed>");
        let tab_id = if let Some(&id) = effective_tab_map.get(tab_name) {
            id
        } else if tab.widgets.iter().any(|w| {
            w.widget_type
                .as_deref()
                .map(|t| type_map.contains_key(t))
                .unwrap_or(false)
        }) {
            // Pre-create the tab so widget instances can reference it.
            match scene.create_tab(tab_name, tab_idx as u32) {
                Ok(id) => {
                    tracing::debug!(
                        tab_name = tab_name,
                        "widget_startup: pre-created tab for widget instance binding"
                    );
                    effective_tab_map.insert(tab_name.to_string(), id);
                    id
                }
                Err(e) => {
                    tracing::warn!(
                        tab_name = tab_name,
                        error = %e,
                        "widget_startup: could not pre-create tab; skipping widget instances"
                    );
                    continue;
                }
            }
        } else {
            // No widgets need this tab; skip instance creation.
            continue;
        };

        for (widget_idx, raw_widget) in tab.widgets.iter().enumerate() {
            let widget_type = match raw_widget.widget_type.as_deref() {
                Some(t) if !t.is_empty() => t,
                _ => {
                    tracing::warn!(
                        tab_name = tab_name,
                        widget_idx = widget_idx,
                        "widget_startup: widget entry missing widget_type; skipping"
                    );
                    continue;
                }
            };

            // Check that the type is registered.
            if !type_map.contains_key(widget_type) {
                tracing::warn!(
                    tab_name = tab_name,
                    widget_type = widget_type,
                    "widget_startup: widget type not loaded; skipping instance creation"
                );
                continue;
            }

            // Build and register the widget instance.
            if let Some(instance) = build_widget_instance(raw_widget, tab_id, &type_map) {
                tracing::info!(
                    tab_name = tab_name,
                    instance_name = %instance.instance_name,
                    widget_type = %instance.widget_type_name,
                    "widget_startup: created widget instance"
                );
                scene.widget_registry.register_instance(instance);
                total_instances += 1;
            } else {
                tracing::warn!(
                    tab_name = tab_name,
                    widget_type = widget_type,
                    widget_idx = widget_idx,
                    "widget_startup: failed to build widget instance"
                );
            }
        }
    }

    tracing::info!(
        widget_instances = total_instances,
        "widget_startup: widget instance creation complete"
    );
}

// ─── Helper: collect tab_name → SceneId from scene graph ──────────────────────

/// Build a map from tab name to its `SceneId` from the current scene graph.
///
/// Useful at runtime startup after tabs have been created, before calling
/// `init_widget_registry`.
pub fn collect_tab_name_to_id(scene: &SceneGraph) -> HashMap<String, SceneId> {
    scene
        .tabs
        .iter()
        .map(|(id, tab)| (tab.name.clone(), *id))
        .collect()
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_config::raw::{RawConfig, RawTab, RawWidgetBundles};
    use tze_hud_scene::graph::SceneGraph;

    /// WHEN [widget_bundles] is absent THEN widget registry stays empty and startup succeeds.
    #[test]
    fn absent_widget_bundles_empty_registry() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let raw = RawConfig::default();
        let tab_map = HashMap::new();
        init_widget_registry(&mut scene, &raw, None, &tab_map);
        assert!(
            scene.widget_registry.definitions.is_empty(),
            "widget registry should be empty when no bundles configured"
        );
        assert!(
            scene.widget_registry.instances.is_empty(),
            "no instances should be created when registry is empty"
        );
    }

    /// WHEN [widget_bundles].paths is empty THEN widget registry stays empty.
    #[test]
    fn empty_paths_list_empty_registry() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut raw = RawConfig::default();
        raw.widget_bundles = Some(RawWidgetBundles { paths: vec![] });
        let tab_map = HashMap::new();
        init_widget_registry(&mut scene, &raw, None, &tab_map);
        assert!(scene.widget_registry.definitions.is_empty());
    }

    /// WHEN [widget_bundles].paths contains a non-existent path THEN
    /// scan_bundle_dirs handles it gracefully (skips non-readable dirs).
    #[test]
    fn nonexistent_path_does_not_panic() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut raw = RawConfig::default();
        raw.widget_bundles = Some(RawWidgetBundles {
            paths: vec!["/tmp/tze_hud_nonexistent_widget_dir_99999_a1b2c3".into()],
        });
        let tab_map = HashMap::new();
        // Should not panic; the bundle scanner handles missing dirs gracefully.
        init_widget_registry(&mut scene, &raw, None, &tab_map);
        assert!(scene.widget_registry.definitions.is_empty());
    }

    /// WHEN tab_name_to_id is empty THEN no instances created (tab not found warning).
    #[test]
    fn missing_tab_in_scene_skips_instance_creation() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let mut raw = RawConfig::default();
        raw.widget_bundles = Some(RawWidgetBundles { paths: vec![] });
        raw.tabs.push(RawTab {
            name: Some("Main".into()),
            widgets: vec![tze_hud_config::raw::RawTabWidget {
                widget_type: Some("gauge".into()),
                ..Default::default()
            }],
            ..Default::default()
        });
        // Empty tab map — tab "Main" not in scene yet.
        let tab_map = HashMap::new();
        init_widget_registry(&mut scene, &raw, None, &tab_map);
        assert!(scene.widget_registry.instances.is_empty());
    }
}
