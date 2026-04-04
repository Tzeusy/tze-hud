//! Binding validation tests for the production exemplar progress-bar widget.
//!
//! Loads the canonical production progress-bar bundle from
//! `assets/widget_bundles/progress-bar/` and verifies all three binding types
//! resolve correctly at registration time, that default parameter values match
//! the spec, and that the widget appears in the scene with correct geometry.
//!
//! Tests (6 total):
//!
//! 1. `progress_bar_bundle_loads_with_correct_structure`:
//!    Bundle loads with 3 parameters, 2 layers, 3 bindings on the fill layer.
//!
//! 2. `progress_bar_linear_binding_resolves_at_registration`:
//!    progress → fill-bar.width linear binding (attr_min=0, attr_max=296).
//!
//! 3. `progress_bar_direct_color_binding_resolves_at_registration`:
//!    fill_color → fill-bar.fill direct binding.
//!
//! 4. `progress_bar_text_content_binding_resolves_at_registration`:
//!    label → label-text.text-content direct binding.
//!
//! 5. `progress_bar_instance_has_correct_parameter_defaults`:
//!    Default values: progress=0.0, label="", fill_color=[74,158,255,255].
//!
//! 6. `progress_bar_instance_appears_in_scene_with_correct_geometry`:
//!    Widget instance registered with geometry x=10/1920, y=400/1080,
//!    w=300/1920, h=20/1080 (fractional-relative to 1920x1080).
//!
//! Source: exemplar-progress-bar spec §Progress Bar Parameter Bindings,
//!         §Progress Bar Parameter Schema, §Widget Instance Configuration.
//!
//! [hud-jqc3]

use std::collections::HashMap;
use std::path::PathBuf;

use tze_hud_scene::SceneGraph;
use tze_hud_scene::types::{
    ContentionPolicy, GeometryPolicy, WidgetBindingMapping, WidgetInstance, WidgetParameterValue,
};
use tze_hud_widget::loader::{BundleScanResult, load_bundle_dir_with_tokens};

// ─── Fixture helpers ──────────────────────────────────────────────────────────

/// Minimal token map that resolves all `{{token.*}}` placeholders in the
/// production progress-bar SVGs without requiring a live design-token store.
///
/// Tokens used by the bundle:
/// - `{{token.color.backdrop.default}}` — track-bg fill (track.svg)
/// - `{{token.color.text.accent}}`      — fill-bar initial fill (fill.svg)
/// - `{{token.color.text.primary}}`     — label-text fill (fill.svg)
fn progress_bar_tokens() -> HashMap<String, String> {
    HashMap::from([
        (
            "color.backdrop.default".to_string(),
            "#1a1a2e".to_string(),
        ),
        ("color.text.accent".to_string(), "#4a9eff".to_string()),
        ("color.text.primary".to_string(), "#cccccc".to_string()),
    ])
}

/// Path to the production progress-bar bundle.
///
/// `CARGO_MANIFEST_DIR` points to `crates/tze_hud_widget/`, so we go up two
/// levels to the workspace root and then into `assets/widget_bundles/`.
fn production_progress_bar_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..") // crates/
        .join("..") // workspace root
        .join("assets")
        .join("widget_bundles")
        .join("progress-bar")
}

/// Load the production progress-bar bundle with test tokens.
///
/// Panics if the bundle fails to load — a load failure is a hard test error,
/// not a scenario under test here.
fn load_progress_bar_bundle() -> tze_hud_widget::loader::LoadedBundle {
    let tokens = progress_bar_tokens();
    match load_bundle_dir_with_tokens(&production_progress_bar_path(), &tokens) {
        BundleScanResult::Ok(bundle) => bundle,
        BundleScanResult::Err(e) => panic!("production progress-bar bundle failed to load: {e}"),
    }
}

/// Build a `SceneGraph` with the progress-bar widget type registered and one
/// instance named "progress-bar" with absolute pixel geometry (x=10, y=400,
/// width=300, height=20) stored as a `GeometryPolicy::Relative` fraction
/// against the 1920x1080 display.
///
/// Returns `(scene, tab_id)`.
fn scene_with_progress_bar() -> (SceneGraph, tze_hud_scene::types::SceneId) {
    let bundle = load_progress_bar_bundle();

    let mut definition = bundle.definition.clone();
    definition.default_contention_policy = ContentionPolicy::LatestWins;

    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Main", 0).unwrap();

    scene.widget_registry.register_definition(definition);

    // Build default current_params from the loaded schema.
    let def = scene
        .widget_registry
        .get_definition("progress-bar")
        .expect("progress-bar definition should be registered after register_definition");
    let current_params: HashMap<String, WidgetParameterValue> = def
        .parameter_schema
        .iter()
        .map(|p| (p.name.clone(), p.default_value.clone()))
        .collect();

    // Geometry: x=10, y=400, width=300, height=20 expressed as fractional
    // percentages against the 1920x1080 virtual display.
    let geometry_override = Some(GeometryPolicy::Relative {
        x_pct: 10.0 / 1920.0,
        y_pct: 400.0 / 1080.0,
        width_pct: 300.0 / 1920.0,
        height_pct: 20.0 / 1080.0,
    });

    scene.widget_registry.register_instance(WidgetInstance {
        widget_type_name: "progress-bar".to_string(),
        tab_id,
        geometry_override,
        contention_override: None,
        instance_name: "progress-bar".to_string(),
        current_params,
    });

    (scene, tab_id)
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 1: Bundle loads with correct structure
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN the production progress-bar bundle is loaded THEN it produces a
/// `WidgetDefinition` with 3 parameters, 2 layers, and 3 bindings on the
/// fill layer (fill.svg).
///
/// Source: spec §Progress Bar Asset Bundle Structure, §Progress Bar Parameter Schema.
/// [hud-jqc3]
#[test]
fn progress_bar_bundle_loads_with_correct_structure() {
    let bundle = load_progress_bar_bundle();
    let def = &bundle.definition;

    // Widget type name must match the manifest.
    assert_eq!(def.id, "progress-bar", "widget id should be 'progress-bar'");
    assert_eq!(def.name, "progress-bar", "widget name should be 'progress-bar'");

    // Parameter schema: 3 parameters (progress, label, fill_color).
    assert_eq!(
        def.parameter_schema.len(),
        3,
        "progress-bar must have exactly 3 parameters, got: {:?}",
        def.parameter_schema.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    let param_names: Vec<&str> = def.parameter_schema.iter().map(|p| p.name.as_str()).collect();
    assert!(param_names.contains(&"progress"), "parameter schema must contain 'progress'");
    assert!(param_names.contains(&"label"), "parameter schema must contain 'label'");
    assert!(param_names.contains(&"fill_color"), "parameter schema must contain 'fill_color'");

    // Layers: 2 total — track (index 0) and fill (index 1).
    assert_eq!(
        def.layers.len(),
        2,
        "progress-bar must have exactly 2 layers, got {}",
        def.layers.len()
    );
    assert_eq!(
        def.layers[0].svg_file, "track.svg",
        "layer 0 must be track.svg (background track)"
    );
    assert_eq!(
        def.layers[1].svg_file, "fill.svg",
        "layer 1 must be fill.svg (dynamic fill)"
    );

    // Track layer has no bindings (static after token resolution).
    assert!(
        def.layers[0].bindings.is_empty(),
        "track.svg layer must have no bindings (static), got: {:?}",
        def.layers[0].bindings
    );

    // Fill layer has exactly 3 bindings.
    let fill_bindings = &def.layers[1].bindings;
    assert_eq!(
        fill_bindings.len(),
        3,
        "fill.svg layer must have exactly 3 bindings, got: {:?}",
        fill_bindings.iter().map(|b| (&b.param, &b.target_attribute)).collect::<Vec<_>>()
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 2: Linear binding resolves at registration
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN the progress-bar bundle is loaded THEN the fill.svg layer contains a
/// Linear binding from `progress` to `fill-bar.width` with attr_min=0,
/// attr_max=296.
///
/// Source: spec §Progress Bar Parameter Bindings §Scenario: Linear binding.
/// [hud-jqc3]
#[test]
fn progress_bar_linear_binding_resolves_at_registration() {
    let bundle = load_progress_bar_bundle();
    let fill_bindings = &bundle.definition.layers[1].bindings;

    let linear_binding = fill_bindings
        .iter()
        .find(|b| b.param == "progress")
        .expect("fill.svg must have a binding for param 'progress'");

    assert_eq!(
        linear_binding.target_element, "fill-bar",
        "progress binding must target element 'fill-bar', got '{}'",
        linear_binding.target_element
    );
    assert_eq!(
        linear_binding.target_attribute, "width",
        "progress binding must target attribute 'width', got '{}'",
        linear_binding.target_attribute
    );

    match &linear_binding.mapping {
        WidgetBindingMapping::Linear { attr_min, attr_max } => {
            assert!(
                (attr_min - 0.0).abs() < 1e-4,
                "linear binding attr_min must be 0, got {attr_min}"
            );
            assert!(
                (attr_max - 296.0).abs() < 1e-4,
                "linear binding attr_max must be 296, got {attr_max}"
            );
        }
        other => panic!("progress binding must use Linear mapping, got {other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 3: Direct color binding resolves at registration
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN the progress-bar bundle is loaded THEN the fill.svg layer contains a
/// Direct binding from `fill_color` to `fill-bar.fill`.
///
/// Source: spec §Progress Bar Parameter Bindings §Scenario: Direct color binding.
/// [hud-jqc3]
#[test]
fn progress_bar_direct_color_binding_resolves_at_registration() {
    let bundle = load_progress_bar_bundle();
    let fill_bindings = &bundle.definition.layers[1].bindings;

    let color_binding = fill_bindings
        .iter()
        .find(|b| b.param == "fill_color")
        .expect("fill.svg must have a binding for param 'fill_color'");

    assert_eq!(
        color_binding.target_element, "fill-bar",
        "fill_color binding must target element 'fill-bar', got '{}'",
        color_binding.target_element
    );
    assert_eq!(
        color_binding.target_attribute, "fill",
        "fill_color binding must target attribute 'fill', got '{}'",
        color_binding.target_attribute
    );
    assert!(
        matches!(color_binding.mapping, WidgetBindingMapping::Direct),
        "fill_color binding must use Direct mapping, got {:?}",
        color_binding.mapping
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 4: Text-content binding resolves at registration
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN the progress-bar bundle is loaded THEN the fill.svg layer contains a
/// Direct binding from `label` to `label-text.text-content`.
///
/// Source: spec §Progress Bar Parameter Bindings §Scenario: Text-content binding.
/// [hud-jqc3]
#[test]
fn progress_bar_text_content_binding_resolves_at_registration() {
    let bundle = load_progress_bar_bundle();
    let fill_bindings = &bundle.definition.layers[1].bindings;

    let text_binding = fill_bindings
        .iter()
        .find(|b| b.param == "label")
        .expect("fill.svg must have a binding for param 'label'");

    assert_eq!(
        text_binding.target_element, "label-text",
        "label binding must target element 'label-text', got '{}'",
        text_binding.target_element
    );
    assert_eq!(
        text_binding.target_attribute, "text-content",
        "label binding must target attribute 'text-content', got '{}'",
        text_binding.target_attribute
    );
    assert!(
        matches!(text_binding.mapping, WidgetBindingMapping::Direct),
        "label binding must use Direct mapping, got {:?}",
        text_binding.mapping
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 5: Instance has correct parameter defaults
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN the progress-bar widget instance is created THEN its default parameter
/// values are: progress=0.0, label="", fill_color=[74,158,255,255].
///
/// Source: spec §Progress Bar Parameter Schema §Scenario: Default values.
/// [hud-jqc3]
#[test]
fn progress_bar_instance_has_correct_parameter_defaults() {
    let (scene, _tab) = scene_with_progress_bar();

    let instance = scene
        .widget_registry
        .get_instance("progress-bar")
        .expect("progress-bar instance must be registered");

    // progress default = 0.0
    match instance.current_params.get("progress") {
        Some(WidgetParameterValue::F32(v)) => {
            assert!(
                v.abs() < 1e-6,
                "default progress must be 0.0, got {v}"
            );
        }
        other => panic!("expected F32(0.0) for default progress, got {other:?}"),
    }

    // label default = ""
    match instance.current_params.get("label") {
        Some(WidgetParameterValue::String(s)) => {
            assert!(
                s.is_empty(),
                "default label must be empty string, got {s:?}"
            );
        }
        other => panic!("expected String(\"\") for default label, got {other:?}"),
    }

    // fill_color default = [74, 158, 255, 255]
    // Stored as Rgba with f32 components normalised from [0, 255].
    match instance.current_params.get("fill_color") {
        Some(WidgetParameterValue::Color(rgba)) => {
            let expected_r = 74.0_f32 / 255.0;
            let expected_g = 158.0_f32 / 255.0;
            let expected_b = 255.0_f32 / 255.0;
            let expected_a = 255.0_f32 / 255.0;

            assert!(
                (rgba.r - expected_r).abs() < 1e-3,
                "fill_color.r must be ~{expected_r:.4} (74/255), got {}",
                rgba.r
            );
            assert!(
                (rgba.g - expected_g).abs() < 1e-3,
                "fill_color.g must be ~{expected_g:.4} (158/255), got {}",
                rgba.g
            );
            assert!(
                (rgba.b - expected_b).abs() < 1e-3,
                "fill_color.b must be ~{expected_b:.4} (255/255), got {}",
                rgba.b
            );
            assert!(
                (rgba.a - expected_a).abs() < 1e-3,
                "fill_color.a must be ~{expected_a:.4} (255/255), got {}",
                rgba.a
            );
        }
        other => panic!(
            "expected Color([74,158,255,255]) for default fill_color, got {other:?}"
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 6: Instance appears in scene with correct geometry
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN the progress-bar widget instance is registered with geometry
/// x=10, y=400, width=300, height=20 (absolute pixels on a 1920x1080 display)
/// THEN the instance is retrievable from the registry with a
/// `GeometryPolicy::Relative` geometry override matching those fractions.
///
/// Source: spec §Widget Instance Configuration §Scenario: Instance geometry.
/// [hud-jqc3]
#[test]
fn progress_bar_instance_appears_in_scene_with_correct_geometry() {
    let (scene, _tab) = scene_with_progress_bar();

    let instance = scene
        .widget_registry
        .get_instance("progress-bar")
        .expect("progress-bar instance must be in the scene");

    // The instance should carry the geometry override we set up.
    let geometry = instance
        .geometry_override
        .expect("progress-bar instance must have a geometry_override set");

    match geometry {
        GeometryPolicy::Relative {
            x_pct,
            y_pct,
            width_pct,
            height_pct,
        } => {
            let expected_x = 10.0_f32 / 1920.0;
            let expected_y = 400.0_f32 / 1080.0;
            let expected_w = 300.0_f32 / 1920.0;
            let expected_h = 20.0_f32 / 1080.0;
            let tol = 1e-5_f32;

            assert!(
                (x_pct - expected_x).abs() < tol,
                "geometry x_pct must be {expected_x:.6} (10/1920), got {x_pct:.6}"
            );
            assert!(
                (y_pct - expected_y).abs() < tol,
                "geometry y_pct must be {expected_y:.6} (400/1080), got {y_pct:.6}"
            );
            assert!(
                (width_pct - expected_w).abs() < tol,
                "geometry width_pct must be {expected_w:.6} (300/1920), got {width_pct:.6}"
            );
            assert!(
                (height_pct - expected_h).abs() < tol,
                "geometry height_pct must be {expected_h:.6} (20/1080), got {height_pct:.6}"
            );
        }
        other => panic!(
            "expected GeometryPolicy::Relative for progress-bar, got {other:?}"
        ),
    }

    // Verify the instance is also present in a full scene snapshot.
    let snapshot = scene.take_snapshot(0, 0);
    let widget_in_snapshot = snapshot
        .widget_registry
        .widget_instances
        .iter()
        .find(|inst| inst.instance_name == "progress-bar");
    assert!(
        widget_in_snapshot.is_some(),
        "progress-bar must appear in SceneGraphSnapshot.widget_registry.widget_instances"
    );
}
