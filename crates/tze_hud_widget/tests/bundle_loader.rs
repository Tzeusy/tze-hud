//! Integration tests for the widget asset bundle loader.
//!
//! These tests cover:
//! - Valid bundle loading (acceptance criterion 1)
//! - All structured error codes (acceptance criterion 2)
//! - Binding validation (acceptance criterion 3)
//! - SVG ID resolution
//! - Reference gauge fixture
//!
//! Source: widget-system/spec.md §Requirement: Widget Asset Bundle Format,
//!         §Requirement: SVG Layer Parameter Bindings.

use std::collections::HashMap;
use std::path::PathBuf;

use tze_hud_widget::error::BundleError;
use tze_hud_widget::loader::{
    BundleScanResult, load_bundle_dir, load_bundle_dir_with_tokens, scan_bundle_dirs,
};

// ─── Test helper: path to the gauge fixture ───────────────────────────────────

fn gauge_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("gauge")
}

// ─── Reference gauge fixture ──────────────────────────────────────────────────

/// Acceptance criterion 1: valid widget bundles load and produce WidgetDefinitions.
/// Acceptance criterion 6: Reference gauge bundle included in test fixtures.
#[test]
fn gauge_fixture_loads_successfully() {
    let dir = gauge_fixture_path();
    let result = load_bundle_dir(&dir);
    match result {
        BundleScanResult::Ok(bundle) => {
            let def = &bundle.definition;
            assert_eq!(def.id, "gauge", "widget id should be 'gauge'");
            assert_eq!(def.name, "gauge");
            assert!(
                !def.description.is_empty(),
                "description should not be empty"
            );

            // Parameter schema: level, label, fill_color, severity.
            assert_eq!(def.parameter_schema.len(), 4);
            let param_names: Vec<&str> = def
                .parameter_schema
                .iter()
                .map(|p| p.name.as_str())
                .collect();
            assert!(param_names.contains(&"level"));
            assert!(param_names.contains(&"label"));
            assert!(param_names.contains(&"fill_color"));
            assert!(param_names.contains(&"severity"));

            // Layers: background and fill.
            assert_eq!(def.layers.len(), 2);
            assert_eq!(def.layers[0].svg_file, "background.svg");
            assert_eq!(def.layers[1].svg_file, "fill.svg");

            // background.svg has no bindings.
            assert!(def.layers[0].bindings.is_empty());

            // fill.svg has 4 bindings: level→bar/height (linear),
            // fill_color→bar/fill (direct), label→label-text/text-content (direct),
            // severity→indicator/fill (discrete).
            let fill_bindings = &def.layers[1].bindings;
            assert_eq!(fill_bindings.len(), 4);

            // Check linear binding.
            let level_binding = fill_bindings.iter().find(|b| b.param == "level").unwrap();
            assert_eq!(level_binding.target_element, "bar");
            assert_eq!(level_binding.target_attribute, "height");
            assert!(
                matches!(
                    level_binding.mapping,
                    tze_hud_scene::types::WidgetBindingMapping::Linear {
                        attr_min,
                        attr_max
                    } if (attr_min - 0.0).abs() < 1e-6 && (attr_max - 200.0).abs() < 1e-6
                ),
                "expected linear mapping with attr_min=0, attr_max=200"
            );

            // Check direct binding for color.
            let color_binding = fill_bindings
                .iter()
                .find(|b| b.param == "fill_color")
                .unwrap();
            assert_eq!(color_binding.target_element, "bar");
            assert_eq!(color_binding.target_attribute, "fill");
            assert!(matches!(
                color_binding.mapping,
                tze_hud_scene::types::WidgetBindingMapping::Direct
            ));

            // Check text-content synthetic binding.
            let label_binding = fill_bindings.iter().find(|b| b.param == "label").unwrap();
            assert_eq!(label_binding.target_element, "label-text");
            assert_eq!(label_binding.target_attribute, "text-content");
            assert!(matches!(
                label_binding.mapping,
                tze_hud_scene::types::WidgetBindingMapping::Direct
            ));

            // Check discrete binding.
            let sev_binding = fill_bindings
                .iter()
                .find(|b| b.param == "severity")
                .unwrap();
            assert_eq!(sev_binding.target_element, "indicator");
            assert_eq!(sev_binding.target_attribute, "fill");
            if let tze_hud_scene::types::WidgetBindingMapping::Discrete { value_map } =
                &sev_binding.mapping
            {
                assert_eq!(value_map.get("info").map(String::as_str), Some("#00cc66"));
                assert_eq!(
                    value_map.get("warning").map(String::as_str),
                    Some("#ffcc00")
                );
                assert_eq!(value_map.get("error").map(String::as_str), Some("#ff3300"));
            } else {
                panic!("expected discrete mapping for severity binding");
            }

            // SVG content is captured.
            assert!(bundle.svg_contents.contains_key("background.svg"));
            assert!(bundle.svg_contents.contains_key("fill.svg"));
        }
        BundleScanResult::Err(e) => {
            panic!("gauge fixture failed to load: {e}");
        }
    }
}

// ─── Error: WIDGET_BUNDLE_NO_MANIFEST ─────────────────────────────────────────

/// Acceptance criterion 2: WIDGET_BUNDLE_NO_MANIFEST.
/// Source: widget-system/spec.md §Scenario: Missing manifest rejected.
#[test]
fn no_manifest_error() {
    let dir = tempfile::tempdir().unwrap();
    // Create an SVG file but no widget.toml.
    std::fs::write(dir.path().join("fill.svg"), b"<svg></svg>").unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::NoManifest { path }) => {
            assert!(path.contains(dir.path().to_str().unwrap()));
        }
        other => panic!("expected NoManifest, got {other:?}"),
    }
}

#[test]
fn no_manifest_wire_code() {
    let err = BundleError::NoManifest {
        path: "/tmp/test".to_string(),
    };
    assert_eq!(err.wire_code(), "WIDGET_BUNDLE_NO_MANIFEST");
}

// ─── Error: WIDGET_BUNDLE_INVALID_MANIFEST ────────────────────────────────────

/// Acceptance criterion 2: WIDGET_BUNDLE_INVALID_MANIFEST.
/// Source: widget-system/spec.md §Scenario: Invalid manifest rejected.
#[test]
fn invalid_manifest_toml_syntax_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        b"this is not [ valid toml >>>",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::InvalidManifest { path, detail }) => {
            assert!(path.contains(dir.path().to_str().unwrap()));
            assert!(!detail.is_empty());
        }
        other => panic!("expected InvalidManifest, got {other:?}"),
    }
}

#[test]
fn invalid_manifest_missing_name() {
    let dir = tempfile::tempdir().unwrap();
    // Valid TOML but missing required 'name' field.
    std::fs::write(
        dir.path().join("widget.toml"),
        b"version = \"1.0.0\"\ndescription = \"test\"\n",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::InvalidManifest { detail, .. }) => {
            assert!(
                detail.contains("name"),
                "error should mention 'name', got: {detail}"
            );
        }
        other => panic!("expected InvalidManifest, got {other:?}"),
    }
}

#[test]
fn invalid_manifest_wire_code() {
    let err = BundleError::InvalidManifest {
        path: "/tmp".to_string(),
        detail: "missing name".to_string(),
    };
    assert_eq!(err.wire_code(), "WIDGET_BUNDLE_INVALID_MANIFEST");
}

// ─── Error: WIDGET_BUNDLE_MISSING_SVG ────────────────────────────────────────

/// Acceptance criterion 2: WIDGET_BUNDLE_MISSING_SVG.
/// Source: widget-system/spec.md §Scenario: Missing SVG file rejected.
#[test]
fn missing_svg_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "test"
version = "1.0.0"
description = "test widget"

[[layers]]
svg_file = "nonexistent.svg"
"#,
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::MissingSvg { svg_file, .. }) => {
            assert_eq!(svg_file, "nonexistent.svg");
        }
        other => panic!("expected MissingSvg, got {other:?}"),
    }
}

#[test]
fn missing_svg_wire_code() {
    let err = BundleError::MissingSvg {
        path: "/tmp".to_string(),
        svg_file: "fill.svg".to_string(),
    };
    assert_eq!(err.wire_code(), "WIDGET_BUNDLE_MISSING_SVG");
}

// ─── Error: WIDGET_BUNDLE_SVG_PARSE_ERROR ────────────────────────────────────

/// Acceptance criterion 2: WIDGET_BUNDLE_SVG_PARSE_ERROR.
/// Source: widget-system/spec.md §Scenario: SVG parse failure rejected.
#[test]
fn svg_parse_error_invalid_xml() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "test"
version = "1.0.0"
description = "test"

[[layers]]
svg_file = "bad.svg"
"#,
    )
    .unwrap();
    // Write a file that is not valid SVG.
    std::fs::write(dir.path().join("bad.svg"), b"not xml at all ><<>").unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::SvgParseError { svg_file, .. }) => {
            assert_eq!(svg_file, "bad.svg");
        }
        other => panic!("expected SvgParseError, got {other:?}"),
    }
}

#[test]
fn svg_parse_error_non_svg_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "test"
version = "1.0.0"
description = "test"

[[layers]]
svg_file = "html.svg"
"#,
    )
    .unwrap();
    // Write well-formed XML but with <html> root, not <svg>.
    std::fs::write(
        dir.path().join("html.svg"),
        b"<html><body>not an svg</body></html>",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::SvgParseError { svg_file, .. }) => {
            assert_eq!(svg_file, "html.svg");
        }
        other => panic!("expected SvgParseError, got {other:?}"),
    }
}

#[test]
fn svg_parse_error_wire_code() {
    let err = BundleError::SvgParseError {
        path: "/tmp".to_string(),
        svg_file: "fill.svg".to_string(),
        detail: "bad XML".to_string(),
    };
    assert_eq!(err.wire_code(), "WIDGET_BUNDLE_SVG_PARSE_ERROR");
}

// ─── Error: WIDGET_BUNDLE_DUPLICATE_TYPE ─────────────────────────────────────

/// Acceptance criterion 2: WIDGET_BUNDLE_DUPLICATE_TYPE.
/// Source: widget-system/spec.md §Scenario: Duplicate type name rejected.
#[test]
fn duplicate_type_error_via_scan() {
    // Create two bundle directories with the same widget name.
    let root = tempfile::tempdir().unwrap();
    let bundle_a = root.path().join("bundle_a");
    let bundle_b = root.path().join("bundle_b");
    std::fs::create_dir_all(&bundle_a).unwrap();
    std::fs::create_dir_all(&bundle_b).unwrap();

    let manifest = br#"name = "gauge"
version = "1.0.0"
description = "a gauge"

[[layers]]
svg_file = "fill.svg"
"#;
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"bar\"/></svg>";

    std::fs::write(bundle_a.join("widget.toml"), manifest).unwrap();
    std::fs::write(bundle_a.join("fill.svg"), svg).unwrap();
    std::fs::write(bundle_b.join("widget.toml"), manifest).unwrap();
    std::fs::write(bundle_b.join("fill.svg"), svg).unwrap();

    let results = scan_bundle_dirs(
        &[root.path().to_path_buf()],
        &std::collections::HashMap::new(),
    );

    // Exactly one should succeed and one should fail with DuplicateType.
    let ok_count = results
        .iter()
        .filter(|r| matches!(r, BundleScanResult::Ok(_)))
        .count();
    let dup_count = results
        .iter()
        .filter(|r| matches!(r, BundleScanResult::Err(BundleError::DuplicateType { .. })))
        .count();
    assert_eq!(ok_count, 1, "exactly one bundle should succeed");
    assert_eq!(dup_count, 1, "exactly one duplicate error expected");
}

#[test]
fn duplicate_type_wire_code() {
    let err = BundleError::DuplicateType {
        name: "gauge".to_string(),
        existing_path: "/a".to_string(),
        new_path: "/b".to_string(),
    };
    assert_eq!(err.wire_code(), "WIDGET_BUNDLE_DUPLICATE_TYPE");
}

// ─── Error: WIDGET_BINDING_UNRESOLVABLE ───────────────────────────────────────

/// Acceptance criterion 2 + 3: WIDGET_BINDING_UNRESOLVABLE.
/// Bindings referencing nonexistent params/elements rejected at load time.
/// Source: widget-system/spec.md §Scenario: Unresolvable binding rejected.
#[test]
fn binding_nonexistent_param_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "test"
version = "1.0.0"
description = "test"

# No parameters declared!

[[layers]]
svg_file = "fill.svg"

[[layers.bindings]]
param = "nonexistent"
target_element = "bar"
target_attribute = "height"
mapping = "linear"
attr_min = 0.0
attr_max = 200.0
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"bar\"/></svg>",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::BindingUnresolvable { detail, .. }) => {
            assert!(
                detail.contains("nonexistent"),
                "error should mention the bad param name, got: {detail}"
            );
        }
        other => panic!("expected BindingUnresolvable, got {other:?}"),
    }
}

#[test]
fn binding_nonexistent_element_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "test"
version = "1.0.0"
description = "test"

[[parameter_schema]]
name = "level"
type = "f32"
default = 0.0

[[layers]]
svg_file = "fill.svg"

[[layers.bindings]]
param = "level"
target_element = "no-such-id"
target_attribute = "height"
mapping = "linear"
attr_min = 0.0
attr_max = 100.0
"#,
    )
    .unwrap();
    // SVG does not have id="no-such-id".
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"bar\"/></svg>",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::BindingUnresolvable { detail, .. }) => {
            assert!(
                detail.contains("no-such-id"),
                "error should mention the missing element id, got: {detail}"
            );
        }
        other => panic!("expected BindingUnresolvable, got {other:?}"),
    }
}

#[test]
fn binding_incompatible_mapping_type_rejected() {
    // 'linear' mapping is only valid for f32 parameters; applying it to a
    // 'string' parameter must be rejected.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "test"
version = "1.0.0"
description = "test"

[[parameter_schema]]
name = "label"
type = "string"
default = ""

[[layers]]
svg_file = "fill.svg"

[[layers.bindings]]
param = "label"
target_element = "bar"
target_attribute = "height"
mapping = "linear"
attr_min = 0.0
attr_max = 100.0
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"bar\"/></svg>",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::BindingUnresolvable { detail, .. }) => {
            assert!(
                detail.contains("linear"),
                "error should mention 'linear', got: {detail}"
            );
        }
        other => panic!("expected BindingUnresolvable, got {other:?}"),
    }
}

#[test]
fn binding_unresolvable_wire_code() {
    let err = BundleError::BindingUnresolvable {
        path: "/tmp".to_string(),
        detail: "param not found".to_string(),
    };
    assert_eq!(err.wire_code(), "WIDGET_BINDING_UNRESOLVABLE");
}

// ─── Three mapping types ──────────────────────────────────────────────────────

/// Acceptance criterion: all three mapping types load correctly.
#[test]
fn linear_mapping_loads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "linear-test"
version = "1.0.0"
description = "linear"

[[parameter_schema]]
name = "level"
type = "f32"
default = 0.0

[parameter_schema.constraints]
f32_min = 0.0
f32_max = 1.0

[[layers]]
svg_file = "fill.svg"

[[layers.bindings]]
param = "level"
target_element = "bar"
target_attribute = "height"
mapping = "linear"
attr_min = 10.0
attr_max = 150.0
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"bar\"/></svg>",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Ok(bundle) => {
            let binding = &bundle.definition.layers[0].bindings[0];
            assert!(matches!(
                binding.mapping,
                tze_hud_scene::types::WidgetBindingMapping::Linear {
                    attr_min,
                    attr_max
                } if (attr_min - 10.0).abs() < 1e-5 && (attr_max - 150.0).abs() < 1e-5
            ));
        }
        BundleScanResult::Err(e) => panic!("unexpected error: {e}"),
    }
}

#[test]
fn direct_string_mapping_loads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "direct-test"
version = "1.0.0"
description = "direct"

[[parameter_schema]]
name = "label"
type = "string"
default = ""

[[layers]]
svg_file = "fill.svg"

[[layers.bindings]]
param = "label"
target_element = "label-el"
target_attribute = "text-content"
mapping = "direct"
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><text id=\"label-el\"/></svg>",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Ok(bundle) => {
            let binding = &bundle.definition.layers[0].bindings[0];
            assert_eq!(binding.target_attribute, "text-content");
            assert!(matches!(
                binding.mapping,
                tze_hud_scene::types::WidgetBindingMapping::Direct
            ));
        }
        BundleScanResult::Err(e) => panic!("unexpected error: {e}"),
    }
}

#[test]
fn discrete_enum_mapping_loads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "discrete-test"
version = "1.0.0"
description = "discrete"

[[parameter_schema]]
name = "severity"
type = "enum"
default = "info"

[parameter_schema.constraints]
enum_allowed_values = ["info", "warning", "error"]

[[layers]]
svg_file = "fill.svg"

[[layers.bindings]]
param = "severity"
target_element = "indicator"
target_attribute = "fill"
mapping = "discrete"

[layers.bindings.value_map]
info = "green"
warning = "yellow"
error = "red"
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle id=\"indicator\"/></svg>",
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Ok(bundle) => {
            let binding = &bundle.definition.layers[0].bindings[0];
            if let tze_hud_scene::types::WidgetBindingMapping::Discrete { value_map } =
                &binding.mapping
            {
                assert_eq!(value_map.get("info").map(String::as_str), Some("green"));
                assert_eq!(value_map.get("warning").map(String::as_str), Some("yellow"));
                assert_eq!(value_map.get("error").map(String::as_str), Some("red"));
            } else {
                panic!("expected discrete mapping");
            }
        }
        BundleScanResult::Err(e) => panic!("unexpected error: {e}"),
    }
}

// ─── Rejected bundle does not prevent others from loading ─────────────────────

/// Spec: A rejected bundle MUST NOT prevent other valid bundles from loading.
/// Source: widget-system/spec.md §Requirement: Widget Asset Bundle Format.
#[test]
fn invalid_bundle_does_not_prevent_valid_bundles() {
    let root = tempfile::tempdir().unwrap();

    // Bundle 1: valid.
    let valid_dir = root.path().join("valid-gauge");
    std::fs::create_dir_all(&valid_dir).unwrap();
    std::fs::write(
        valid_dir.join("widget.toml"),
        br#"name = "progress"
version = "1.0.0"
description = "progress bar"

[[layers]]
svg_file = "fill.svg"
"#,
    )
    .unwrap();
    std::fs::write(
        valid_dir.join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
    )
    .unwrap();

    // Bundle 2: invalid (no manifest).
    let bad_dir = root.path().join("no-manifest-bundle");
    std::fs::create_dir_all(&bad_dir).unwrap();
    std::fs::write(bad_dir.join("fill.svg"), b"<svg></svg>").unwrap();

    let results = scan_bundle_dirs(
        &[root.path().to_path_buf()],
        &std::collections::HashMap::new(),
    );

    let ok_count = results
        .iter()
        .filter(|r| matches!(r, BundleScanResult::Ok(_)))
        .count();
    assert_eq!(ok_count, 1, "valid bundle should still load");

    let err_count = results
        .iter()
        .filter(|r| matches!(r, BundleScanResult::Err(_)))
        .count();
    assert_eq!(err_count, 1, "invalid bundle should produce one error");
}

// ─── Empty bundle directory ──────────────────────────────────────────────────

#[test]
fn empty_bundle_root_returns_no_results() {
    let root = tempfile::tempdir().unwrap();
    let results = scan_bundle_dirs(
        &[root.path().to_path_buf()],
        &std::collections::HashMap::new(),
    );
    assert!(
        results.is_empty(),
        "empty root should produce no results, got {results:?}"
    );
}

// ─── Error: WIDGET_BUNDLE_INVALID_NAME ───────────────────────────────────────

/// Acceptance criterion: widget type name must match [a-z][a-z0-9-]*.
/// Names with uppercase letters, spaces, or special characters are rejected.
/// Source: scene-graph/spec.md §Widget Type Identifier.
#[test]
fn invalid_name_uppercase_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "Gauge"
version = "1.0.0"
description = "uppercase name not allowed"
"#,
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::InvalidName { name, .. }) => {
            assert_eq!(name, "Gauge");
        }
        other => panic!("expected InvalidName, got {other:?}"),
    }
}

#[test]
fn invalid_name_starts_with_digit_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "1gauge"
version = "1.0.0"
description = "digit-leading name not allowed"
"#,
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::InvalidName { name, .. }) => {
            assert_eq!(name, "1gauge");
        }
        other => panic!("expected InvalidName, got {other:?}"),
    }
}

#[test]
fn invalid_name_underscore_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "my_gauge"
version = "1.0.0"
description = "underscore not allowed"
"#,
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::InvalidName { name, .. }) => {
            assert_eq!(name, "my_gauge");
        }
        other => panic!("expected InvalidName, got {other:?}"),
    }
}

#[test]
fn invalid_name_space_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "my gauge"
version = "1.0.0"
description = "space not allowed"
"#,
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    match result {
        BundleScanResult::Err(BundleError::InvalidName { name, .. }) => {
            assert_eq!(name, "my gauge");
        }
        other => panic!("expected InvalidName, got {other:?}"),
    }
}

#[test]
fn valid_name_with_hyphens_and_digits_accepted() {
    // Names like "my-widget2" must be accepted.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "my-widget2"
version = "1.0.0"
description = "hyphen and digit are valid"
"#,
    )
    .unwrap();

    let result = load_bundle_dir(dir.path());
    // No layers in this minimal bundle, but the name is valid so we should NOT get InvalidName.
    // We expect Ok (no layers = valid but empty layer list).
    match result {
        BundleScanResult::Ok(bundle) => {
            assert_eq!(bundle.definition.id, "my-widget2");
        }
        BundleScanResult::Err(BundleError::InvalidName { name, .. }) => {
            panic!("valid name '{name}' was incorrectly rejected");
        }
        BundleScanResult::Err(e) => {
            // Any other error (e.g. MissingSvg) is acceptable — the name was valid.
            let _ = e;
        }
    }
}

#[test]
fn invalid_name_wire_code() {
    let err = BundleError::InvalidName {
        path: "/tmp/test".to_string(),
        name: "Gauge".to_string(),
    };
    assert_eq!(err.wire_code(), "WIDGET_BUNDLE_INVALID_NAME");
}

// ─── Token placeholder resolution (bundle-level integration) ─────────────────

/// SVG containing a resolved token placeholder loads successfully; resolved
/// text is stored in svg_contents.
#[test]
fn bundle_with_resolved_token_loads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "token-test"
version = "1.0.0"
description = "token substitution test"

[[layers]]
svg_file = "fill.svg"
"#,
    )
    .unwrap();
    // SVG uses a token placeholder for the fill colour.
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"bar\" fill=\"{{token.color.primary}}\"/></svg>",
    )
    .unwrap();

    let mut tokens = HashMap::new();
    tokens.insert("color.primary".to_string(), "red".to_string());

    let result = load_bundle_dir_with_tokens(dir.path(), &tokens);
    match result {
        BundleScanResult::Ok(bundle) => {
            // The stored SVG bytes should contain the resolved value, not the placeholder.
            let svg_bytes = bundle.svg_contents.get("fill.svg").unwrap();
            let svg_str = std::str::from_utf8(svg_bytes).unwrap();
            assert!(
                svg_str.contains("red"),
                "resolved token value should be present: {svg_str}"
            );
            assert!(
                !svg_str.contains("{{token.color.primary}}"),
                "placeholder should have been replaced: {svg_str}"
            );
        }
        BundleScanResult::Err(e) => panic!("unexpected error: {e}"),
    }
}

/// SVG with an unresolved token produces `BundleError::UnresolvedToken`.
#[test]
fn bundle_with_unresolved_token_fails() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "unresolved-test"
version = "1.0.0"
description = "unresolved token test"

[[layers]]
svg_file = "fill.svg"
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect fill=\"{{token.missing.key}}\"/></svg>",
    )
    .unwrap();

    // Empty token map — placeholder cannot be resolved.
    let result = load_bundle_dir_with_tokens(dir.path(), &HashMap::new());
    match result {
        BundleScanResult::Err(BundleError::UnresolvedToken {
            svg_file,
            token_key,
            ..
        }) => {
            assert_eq!(svg_file, "fill.svg");
            assert_eq!(token_key, "missing.key");
        }
        other => panic!("expected UnresolvedToken, got {other:?}"),
    }
}

/// Wire code for `UnresolvedToken` is `WIDGET_BUNDLE_UNRESOLVED_TOKEN`.
#[test]
fn unresolved_token_wire_code() {
    let err = BundleError::UnresolvedToken {
        path: "/tmp".to_string(),
        svg_file: "fill.svg".to_string(),
        token_key: "color.primary".to_string(),
    };
    assert_eq!(err.wire_code(), "WIDGET_BUNDLE_UNRESOLVED_TOKEN");
}

/// `load_bundle_dir` (no-token variant) still works when SVG contains no placeholders.
#[test]
fn load_bundle_dir_no_tokens_no_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("widget.toml"),
        br#"name = "plain-test"
version = "1.0.0"
description = "no placeholders"

[[layers]]
svg_file = "fill.svg"
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("fill.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"bar\"/></svg>",
    )
    .unwrap();

    // load_bundle_dir uses an empty token map internally.
    let result = load_bundle_dir(dir.path());
    assert!(
        matches!(result, BundleScanResult::Ok(_)),
        "plain SVG without placeholders should load fine: {result:?}"
    );
}
