//! Status-indicator contention policy, partial-update retention, and
//! performance budget tests.
//!
//! Covers all scenarios required by hud-tjq8.3:
//!
//! **LatestWins contention (1 test):**
//!   1. Agent A publishes status=online label=A, then Agent B publishes
//!      status=busy label=B → assert the sole active publication belongs to B
//!      and its params reflect B's values only (A fully displaced).
//!
//! **Partial update retention (1 test):**
//!   2. Agent A publishes status=online label=Butler, then publishes only
//!      status=away → assert instance.current_params is status=away
//!      label=Butler (partial update retains unpublished parameters).
//!
//! **Performance budget (1 test):**
//!   3. Re-rasterize the 48×48 indicator SVG after a status parameter change
//!      and assert completion under the lenient CI threshold (500ms).  The
//!      strict 2ms spec requirement is enforced by the Criterion benchmark
//!      on reference hardware.
//!
//! Source: exemplar-status-indicator/spec.md §Status Indicator Contention
//!         Policy, §Status Indicator Partial Update, §Status Indicator
//!         Re-Rasterization Budget.
//!
//! [hud-tjq8.3]

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use tze_hud_compositor::widget::rasterize_svg_layers;
use tze_hud_scene::SceneGraph;
use tze_hud_scene::types::{
    ContentionPolicy, GeometryPolicy, RenderingPolicy, WidgetBinding, WidgetBindingMapping,
    WidgetInstance, WidgetParameterValue,
};
use tze_hud_widget::loader::{BundleScanResult, load_bundle_dir_with_tokens};

// ─── CI threshold ─────────────────────────────────────────────────────────────

/// Lenient CI threshold: 500ms allows ample headroom for debug builds and
/// software renderers (llvmpipe).
///
/// The strict 2ms spec requirement is enforced by the Criterion benchmark in
/// the compositor crate:
///   `cargo bench -p tze_hud_compositor --bench widget_rasterize`
const CI_THRESHOLD_MS: u64 = 500;

/// Spec target documented for reference (not enforced in this test).
#[allow(dead_code)]
const SPEC_TARGET_MS: u64 = 2;

// ─── Fixture helpers ─────────────────────────────────────────────────────────

/// Path to the status-indicator test fixture bundle.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("status-indicator")
}

/// Canonical token map matching the spec's default fallback values.
fn default_tokens() -> HashMap<String, String> {
    let mut tokens = HashMap::new();
    tokens.insert("color.border.default".to_string(), "#333333".to_string());
    tokens.insert("color.text.secondary".to_string(), "#B0B0B0".to_string());
    tokens
}

/// Load the status-indicator fixture with the given tokens.  Panics on failure.
fn load_fixture(tokens: &HashMap<String, String>) -> tze_hud_widget::loader::LoadedBundle {
    let dir = fixture_path();
    match load_bundle_dir_with_tokens(&dir, tokens) {
        BundleScanResult::Ok(bundle) => bundle,
        BundleScanResult::Err(e) => panic!("status-indicator fixture failed to load: {e}"),
    }
}

/// Build a `SceneGraph` with the status-indicator widget type registered and
/// one instance named "status-indicator" bound to a default tab.
///
/// Returns `(scene, tab_id)`.
fn scene_with_status_indicator(
    tokens: &HashMap<String, String>,
) -> (SceneGraph, tze_hud_scene::types::SceneId) {
    let bundle = load_fixture(tokens);

    let mut definition = bundle.definition.clone();
    definition.default_contention_policy = ContentionPolicy::LatestWins;
    definition.default_rendering_policy = RenderingPolicy::default();
    definition.default_geometry_policy = GeometryPolicy::Relative {
        x_pct: 0.0,
        y_pct: 0.0,
        width_pct: 1.0,
        height_pct: 1.0,
    };

    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let tab_id = scene.create_tab("Main", 0).unwrap();

    scene.widget_registry.register_definition(definition);

    // Build default current_params from the loaded schema.
    let def = scene
        .widget_registry
        .get_definition("status-indicator")
        .expect("status-indicator definition should be registered");
    let current_params: HashMap<String, WidgetParameterValue> = def
        .parameter_schema
        .iter()
        .map(|p| (p.name.clone(), p.default_value.clone()))
        .collect();

    scene.widget_registry.register_instance(WidgetInstance {
        widget_type_name: "status-indicator".to_string(),
        tab_id,
        geometry_override: None,
        contention_override: None,
        instance_name: "status-indicator".to_string(),
        current_params,
    });

    (scene, tab_id)
}

// ─── Inline SVG for the performance test ─────────────────────────────────────

/// Resolved indicator SVG (tokens substituted, 48×48 canvas).
///
/// Mirrors the fixture's `indicator.svg` with token placeholders already
/// replaced, so the performance test does not depend on the loader path.
const INDICATOR_SVG_48: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" width="48" height="48">
  <circle id="indicator-fill" cx="24" cy="20" r="10"
          fill="#00cc66"
          stroke="#333333" stroke-width="2"/>
  <text id="label-text" x="24" y="42" text-anchor="middle"
        font-family="sans-serif" font-size="8"
        fill="#B0B0B0"></text>
</svg>"##;

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 1: LatestWins contention — Agent B fully displaces Agent A
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN Agent A publishes status=online label=A, THEN Agent B publishes
/// status=busy label=B, THEN the sole active publication is Agent B's record
/// and its params contain status=busy and label=B (Agent A is fully displaced).
///
/// LatestWins policy: each call to `publish_to_widget` replaces the single
/// stored publication record regardless of the publishing namespace.
///
/// Source: spec.md §Scenario: LatestWins — agent B displaces agent A
/// [hud-tjq8.3]
#[test]
fn latest_wins_agent_b_displaces_agent_a() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    // Agent A publishes status=online, label=A.
    let params_a = HashMap::from([
        (
            "status".to_string(),
            WidgetParameterValue::Enum("online".to_string()),
        ),
        (
            "label".to_string(),
            WidgetParameterValue::String("A".to_string()),
        ),
    ]);
    scene
        .publish_to_widget(
            "status-indicator",
            params_a,
            "agent.A",
            None,
            0,
            None,
        )
        .expect("Agent A publish should succeed");

    // Sanity: exactly one active publication from A.
    let pubs_after_a = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(pubs_after_a.len(), 1, "one publication after A");
    assert_eq!(
        pubs_after_a[0].publisher_namespace, "agent.A",
        "publication should be from agent.A"
    );

    // Agent B publishes status=busy, label=B.
    let params_b = HashMap::from([
        (
            "status".to_string(),
            WidgetParameterValue::Enum("busy".to_string()),
        ),
        (
            "label".to_string(),
            WidgetParameterValue::String("B".to_string()),
        ),
    ]);
    scene
        .publish_to_widget(
            "status-indicator",
            params_b,
            "agent.B",
            None,
            0,
            None,
        )
        .expect("Agent B publish should succeed");

    // LatestWins: exactly one active publication, belonging to Agent B.
    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(
        pubs.len(),
        1,
        "LatestWins must retain exactly one publication after two publishes; got {}",
        pubs.len()
    );

    // Agent A is fully displaced.
    assert_eq!(
        pubs[0].publisher_namespace, "agent.B",
        "surviving publication must be from agent.B, got {:?}",
        pubs[0].publisher_namespace
    );

    // Verify status reflects B's value only.
    match pubs[0].params.get("status") {
        Some(WidgetParameterValue::Enum(s)) => assert_eq!(
            s, "busy",
            "status should be 'busy' (B's value), got {s:?}"
        ),
        other => panic!("expected Enum(\"busy\") for status, got {other:?}"),
    }

    // Verify label reflects B's value only.
    match pubs[0].params.get("label") {
        Some(WidgetParameterValue::String(s)) => assert_eq!(
            s, "B",
            "label should be \"B\" (B's value), got {s:?}"
        ),
        other => panic!("expected String(\"B\") for label, got {other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 2: Partial update retention — unpublished parameters are retained
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN Agent A publishes status=online label=Butler THEN publishes only
/// status=away (omitting label), THEN the widget instance's current_params
/// is status=away label=Butler — the partial update retains the previously
/// published label value.
///
/// This verifies the incremental merge in `publish_to_widget` Step 4: new
/// params are merged *over* existing `current_params`, so unmentioned
/// parameters carry forward.
///
/// Source: spec.md §Scenario: Partial update retains unpublished parameters
/// [hud-tjq8.3]
#[test]
fn partial_update_retains_unpublished_parameters() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    // Step 1: full publish — status=online, label=Butler.
    let params_full = HashMap::from([
        (
            "status".to_string(),
            WidgetParameterValue::Enum("online".to_string()),
        ),
        (
            "label".to_string(),
            WidgetParameterValue::String("Butler".to_string()),
        ),
    ]);
    scene
        .publish_to_widget(
            "status-indicator",
            params_full,
            "agent.test",
            None,
            0,
            None,
        )
        .expect("full publish should succeed");

    // Sanity: current_params should reflect both params.
    {
        let inst = scene
            .widget_registry
            .instances
            .get("status-indicator")
            .expect("instance must exist");
        match inst.current_params.get("label") {
            Some(WidgetParameterValue::String(s)) => {
                assert_eq!(s, "Butler", "label should be 'Butler' after full publish")
            }
            other => panic!("expected String(\"Butler\") for label after full publish, got {other:?}"),
        }
    }

    // Step 2: partial publish — status=away only (label is NOT included).
    let params_partial = HashMap::from([(
        "status".to_string(),
        WidgetParameterValue::Enum("away".to_string()),
    )]);
    scene
        .publish_to_widget(
            "status-indicator",
            params_partial,
            "agent.test",
            None,
            0,
            None,
        )
        .expect("partial publish should succeed");

    // The active publication record only has the one submitted param.
    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(
        pubs.len(),
        1,
        "LatestWins: one active publication after partial update"
    );
    assert!(
        pubs[0].params.contains_key("status"),
        "partial publication record must contain 'status'"
    );
    assert!(
        !pubs[0].params.contains_key("label"),
        "partial publication record must NOT contain 'label' (it was not submitted)"
    );

    // current_params on the instance must carry the merged state:
    // status=away (new) AND label=Butler (retained from previous publish).
    let inst = scene
        .widget_registry
        .instances
        .get("status-indicator")
        .expect("instance must exist");

    match inst.current_params.get("status") {
        Some(WidgetParameterValue::Enum(s)) => assert_eq!(
            s, "away",
            "current_params.status should be 'away' after partial update, got {s:?}"
        ),
        other => panic!("expected Enum(\"away\") for status in current_params, got {other:?}"),
    }

    match inst.current_params.get("label") {
        Some(WidgetParameterValue::String(s)) => assert_eq!(
            s, "Butler",
            "current_params.label should still be 'Butler' after partial update (retained), got {s:?}"
        ),
        other => panic!(
            "expected String(\"Butler\") for label in current_params after partial update, got {other:?}"
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEST 3: Performance — 48×48 indicator SVG re-rasterization within CI budget
// ═══════════════════════════════════════════════════════════════════════════════

/// WHEN the 48×48 status-indicator SVG is re-rasterized after a status
/// parameter change THEN rasterization completes within the lenient CI
/// threshold of 500ms.
///
/// ## Thresholds
///
/// | Context            | Threshold | Rationale                                    |
/// |--------------------|-----------|----------------------------------------------|
/// | Reference hardware | < 2ms     | Spec target (status-indicator/spec.md §3)    |
/// | CI (any renderer)  | < 500ms   | Headroom for debug builds + llvmpipe VMs     |
///
/// The 2ms spec requirement is measured by the Criterion benchmark
/// (`benches/widget_rasterize.rs`) on release-optimised reference hardware.
/// This test uses the lenient threshold to avoid false failures on CI.
///
/// Source: spec.md §Scenario: 48×48 re-rasterization within 2ms
/// [hud-tjq8.3]
#[test]
fn indicator_48x48_rasterize_within_ci_budget() {
    // Build bindings matching the fixture's indicator.svg layer.
    let bindings: Vec<WidgetBinding> = vec![
        WidgetBinding {
            param: "status".to_string(),
            target_element: "indicator-fill".to_string(),
            target_attribute: "fill".to_string(),
            mapping: WidgetBindingMapping::Discrete {
                value_map: [
                    ("online".to_string(), "#00CC66".to_string()),
                    ("away".to_string(), "#FFB800".to_string()),
                    ("busy".to_string(), "#FF4444".to_string()),
                    ("offline".to_string(), "#666666".to_string()),
                ]
                .into_iter()
                .collect(),
            },
        },
        WidgetBinding {
            param: "label".to_string(),
            target_element: "label-text".to_string(),
            target_attribute: "text-content".to_string(),
            mapping: WidgetBindingMapping::Direct,
        },
    ];

    // Parameters simulating a status change to "busy" with a label.
    let params: HashMap<String, WidgetParameterValue> = [
        (
            "status".to_string(),
            WidgetParameterValue::Enum("busy".to_string()),
        ),
        (
            "label".to_string(),
            WidgetParameterValue::String("Butler".to_string()),
        ),
    ]
    .into_iter()
    .collect();

    let constraints: HashMap<String, (f32, f32)> = HashMap::new();

    let layers: Vec<(&str, &[WidgetBinding])> = vec![(INDICATOR_SVG_48, &bindings)];

    // Warmup: one iteration to allow any lazy initialization.
    let _ = rasterize_svg_layers(&layers, &constraints, &params, 48, 48);

    // Timed iteration: re-rasterize after a status parameter change.
    let start = Instant::now();
    let pixmap = rasterize_svg_layers(&layers, &constraints, &params, 48, 48);
    let elapsed_us = start.elapsed().as_micros() as u64;
    let elapsed_ms = elapsed_us / 1000;

    // Correctness: must produce a non-empty 48×48 pixmap.
    let pixmap =
        pixmap.expect("rasterize_svg_layers must produce a pixmap for the indicator fixture");
    assert_eq!(pixmap.width(), 48, "pixmap width must be 48");
    assert_eq!(pixmap.height(), 48, "pixmap height must be 48");
    assert_eq!(
        pixmap.data().len(),
        48 * 48 * 4,
        "pixmap must contain 48×48×4 bytes (RGBA)"
    );

    // Verify the discrete `status` binding was applied by sampling the center
    // pixel of the circle (cx=24, cy=20) and checking it is close to the
    // expected busy color (#FF4444).  A small per-channel tolerance accounts
    // for antialiasing differences between renderers.
    {
        let width = pixmap.width() as usize;
        // Circle center at (24, 20) in the 48×48 canvas.
        let cx = 24_usize;
        let cy = 20_usize;
        let idx = (cy * width + cx) * 4;
        let data = pixmap.data();
        let r = data[idx];
        let g = data[idx + 1];
        let b = data[idx + 2];
        let a = data[idx + 3];

        // Expected busy color: #FF4444 (red=255, green=68, blue=68).
        let expected_r: u8 = 0xFF;
        let expected_g: u8 = 0x44;
        let expected_b: u8 = 0x44;
        let tolerance: u8 = 16;

        assert!(
            a > 0,
            "center pixel alpha must be non-zero: discrete status binding for 'busy' must render"
        );
        assert!(
            r.abs_diff(expected_r) <= tolerance
                && g.abs_diff(expected_g) <= tolerance
                && b.abs_diff(expected_b) <= tolerance,
            "center pixel color ({r}, {g}, {b}, {a}) must be close to busy status color \
             #{expected_r:02X}{expected_g:02X}{expected_b:02X} (±{tolerance} per channel); \
             discrete binding may not have been applied"
        );
    }

    eprintln!(
        "[status_indicator_perf] 48×48 re-rasterize: {}µs ({} ms) \
         — CI threshold: {}ms, spec target (ref hw): {}ms",
        elapsed_us, elapsed_ms, CI_THRESHOLD_MS, SPEC_TARGET_MS,
    );

    // Budget assertion: lenient CI threshold catches catastrophic regressions.
    // For the strict 2ms spec target, run:
    //   cargo bench -p tze_hud_compositor --bench widget_rasterize
    assert!(
        elapsed_ms < CI_THRESHOLD_MS,
        "48×48 indicator SVG re-rasterization took {}ms, exceeds lenient CI threshold of {}ms \
         (spec target for reference hardware is {}ms). \
         This likely indicates a catastrophic regression. \
         Run `cargo bench -p tze_hud_compositor --bench widget_rasterize` on optimised reference \
         hardware to verify the 2ms spec requirement.",
        elapsed_ms,
        CI_THRESHOLD_MS,
        SPEC_TARGET_MS,
    );
}
