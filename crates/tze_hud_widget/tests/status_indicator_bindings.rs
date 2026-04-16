//! Status-indicator discrete binding, text-content binding, and token
//! resolution tests.
//!
//! Covers all scenarios required by hud-tjq8.2:
//!
//! **Discrete binding (6 tests):**
//! 1–4. Each status enum value (online/away/busy/offline) → correct fill color.
//!   5. Invalid enum value → WIDGET_PARAMETER_INVALID_VALUE rejection.
//!   6. transition_ms has no effect on enum params — discrete snap only.
//!
//! **Text-content binding (3 tests):**
//!   7. label="Butler" → label-text content is "Butler".
//!   8. label="" → label-text content is empty.
//!   9. label exceeding string_max_bytes=16 → WIDGET_PARAMETER_INVALID_VALUE.
//!
//! **Token placeholder resolution (3 tests):**
//!  10. Default tokens → circle stroke resolved to #333333.
//!  11. Default tokens → label fill resolved to #B0B0B0.
//!  12. Custom token override → propagates to SVG attribute.
//!
//! Source: exemplar-status-indicator/spec.md §Status Indicator Discrete Color
//!         Binding, §Status Indicator Text-Content Binding, §Status Indicator
//!         SVG Template.
//!
//! [hud-tjq8.2]

use std::collections::HashMap;
use std::path::PathBuf;

use tze_hud_compositor::widget::{apply_svg_attribute, resolve_binding_value};
use tze_hud_scene::SceneGraph;
use tze_hud_scene::SceneId;
use tze_hud_scene::types::{
    ContentionPolicy, GeometryPolicy, RenderingPolicy, WidgetBindingMapping, WidgetInstance,
    WidgetParameterValue,
};
use tze_hud_scene::validation::ValidationError;
use tze_hud_widget::loader::{BundleScanResult, load_bundle_dir_with_tokens};

// ─── Fixture helpers ─────────────────────────────────────────────────────────

/// Path to the status-indicator test fixture bundle.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("status-indicator")
}

/// Canonical token map matching the spec's default fallback values.
///
/// `{{token.color.border.default}}` → #333333
/// `{{token.color.text.secondary}}` → #B0B0B0
///
/// These are the fallback values described in spec.md §Status Indicator SVG
/// Template §Scenario: Token placeholders resolve to canonical fallbacks.
fn default_tokens() -> HashMap<String, String> {
    let mut tokens = HashMap::new();
    tokens.insert("color.border.default".to_string(), "#333333".to_string());
    tokens.insert("color.text.secondary".to_string(), "#B0B0B0".to_string());
    tokens
}

/// Load the status-indicator fixture with the given tokens, panicking if the
/// bundle fails to load.  Returns the `LoadedBundle`.
fn load_fixture(tokens: &HashMap<String, String>) -> tze_hud_widget::loader::LoadedBundle {
    let dir = fixture_path();
    match load_bundle_dir_with_tokens(&dir, tokens) {
        BundleScanResult::Ok(bundle) => bundle,
        BundleScanResult::Err(e) => panic!("status-indicator fixture failed to load: {e}"),
    }
}

/// Build a `SceneGraph` with the status-indicator widget type registered and
/// one instance named "status-indicator" bound to the default tab.
///
/// Returns `(scene, tab_id)` ready for `publish_to_widget("status-indicator",
/// ...)` calls.
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
        id: SceneId::new(),
        widget_type_name: "status-indicator".to_string(),
        tab_id,
        geometry_override: None,
        contention_override: None,
        instance_name: "status-indicator".to_string(),
        current_params,
    });

    (scene, tab_id)
}

// ─── Helper: resolve discrete status color from the loaded binding ────────────

/// Look up the resolved fill-color string for `status_value` by reading the
/// discrete binding's `value_map` directly from the fixture definition.
///
/// This mirrors what the compositor does when applying bindings to the SVG, but
/// uses the bundle metadata so no wgpu device is required.
fn discrete_color_for_status(tokens: &HashMap<String, String>, status_value: &str) -> String {
    let bundle = load_fixture(tokens);
    let layer = &bundle.definition.layers[0];
    let status_binding = layer
        .bindings
        .iter()
        .find(|b| b.param == "status" && b.target_element == "system-fill")
        .expect("status/system-fill binding must exist in indicator.svg layer");

    match &status_binding.mapping {
        WidgetBindingMapping::Discrete { value_map } => value_map
            .get(status_value)
            .cloned()
            .unwrap_or_else(|| panic!("no discrete entry for status={status_value:?}")),
        other => panic!("expected Discrete mapping for status binding, got {other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// DISCRETE BINDING TESTS (1–6)
// ═══════════════════════════════════════════════════════════════════════════════

// ─── Test 1: status=online → fill #4FB543 ────────────────────────────────────

/// WHEN status=online is published THEN the discrete binding resolves the fill
/// attribute to #4FB543, and publishing succeeds.
///
/// Source: spec.md §Scenario: Online status renders green
/// [hud-tjq8.2]
#[test]
fn status_online_maps_to_green() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    let params = HashMap::from([(
        "status".to_string(),
        WidgetParameterValue::Enum("online".to_string()),
    )]);
    let result = scene.publish_to_widget("status-indicator", params, "agent.test", None, 0, None);
    assert!(
        result.is_ok(),
        "status=online should be accepted, got: {result:?}"
    );

    // Verify the stored publication has the correct enum value.
    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(pubs.len(), 1);
    match pubs[0].params.get("status") {
        Some(WidgetParameterValue::Enum(s)) => {
            assert_eq!(s, "online", "published status should be 'online'")
        }
        other => panic!("expected Enum(\"online\") for status, got {other:?}"),
    }

    // Verify the binding value_map maps "online" to #4FB543.
    let fill_color = discrete_color_for_status(&tokens, "online");
    assert_eq!(
        fill_color, "#4FB543",
        "discrete mapping for online should resolve to #4FB543, got {fill_color:?}"
    );
}

// ─── Test 2: status=away → fill #D97706 ──────────────────────────────────────

/// WHEN status=away is published THEN the discrete binding resolves the fill
/// attribute to #D97706, and publishing succeeds.
///
/// Source: spec.md §Scenario: Away status renders yellow
/// [hud-tjq8.2]
#[test]
fn status_away_maps_to_yellow() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    let params = HashMap::from([(
        "status".to_string(),
        WidgetParameterValue::Enum("away".to_string()),
    )]);
    let result = scene.publish_to_widget("status-indicator", params, "agent.test", None, 0, None);
    assert!(
        result.is_ok(),
        "status=away should be accepted, got: {result:?}"
    );

    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(pubs.len(), 1);
    match pubs[0].params.get("status") {
        Some(WidgetParameterValue::Enum(s)) => {
            assert_eq!(s, "away", "published status should be 'away'")
        }
        other => panic!("expected Enum(\"away\") for status, got {other:?}"),
    }

    let fill_color = discrete_color_for_status(&tokens, "away");
    assert_eq!(
        fill_color, "#D97706",
        "discrete mapping for away should resolve to #D97706, got {fill_color:?}"
    );
}

// ─── Test 3: status=busy → fill #DC2626 ──────────────────────────────────────

/// WHEN status=busy is published THEN the discrete binding resolves the fill
/// attribute to #DC2626, and publishing succeeds.
///
/// Source: spec.md §Scenario: Busy status renders red
/// [hud-tjq8.2]
#[test]
fn status_busy_maps_to_red() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    let params = HashMap::from([(
        "status".to_string(),
        WidgetParameterValue::Enum("busy".to_string()),
    )]);
    let result = scene.publish_to_widget("status-indicator", params, "agent.test", None, 0, None);
    assert!(
        result.is_ok(),
        "status=busy should be accepted, got: {result:?}"
    );

    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(pubs.len(), 1);
    match pubs[0].params.get("status") {
        Some(WidgetParameterValue::Enum(s)) => {
            assert_eq!(s, "busy", "published status should be 'busy'")
        }
        other => panic!("expected Enum(\"busy\") for status, got {other:?}"),
    }

    let fill_color = discrete_color_for_status(&tokens, "busy");
    assert_eq!(
        fill_color, "#DC2626",
        "discrete mapping for busy should resolve to #DC2626, got {fill_color:?}"
    );
}

// ─── Test 4: status=offline → fill #6B7280 ───────────────────────────────────

/// WHEN status=offline is published THEN the discrete binding resolves the fill
/// attribute to #6B7280, and publishing succeeds.
///
/// Source: spec.md §Scenario: Offline status renders gray
/// [hud-tjq8.2]
#[test]
fn status_offline_maps_to_gray() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    let params = HashMap::from([(
        "status".to_string(),
        WidgetParameterValue::Enum("offline".to_string()),
    )]);
    let result = scene.publish_to_widget("status-indicator", params, "agent.test", None, 0, None);
    assert!(
        result.is_ok(),
        "status=offline should be accepted, got: {result:?}"
    );

    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(pubs.len(), 1);
    match pubs[0].params.get("status") {
        Some(WidgetParameterValue::Enum(s)) => {
            assert_eq!(s, "offline", "published status should be 'offline'")
        }
        other => panic!("expected Enum(\"offline\") for status, got {other:?}"),
    }

    let fill_color = discrete_color_for_status(&tokens, "offline");
    assert_eq!(
        fill_color, "#6B7280",
        "discrete mapping for offline should resolve to #6B7280, got {fill_color:?}"
    );
}

// ─── Test 5: Invalid enum value → WIDGET_PARAMETER_INVALID_VALUE ─────────────

/// WHEN status=do-not-disturb (not in enum_allowed_values) is published THEN
/// publish_to_widget returns WIDGET_PARAMETER_INVALID_VALUE.
///
/// Source: spec.md §Scenario: Invalid enum value rejected
/// [hud-tjq8.2]
#[test]
fn status_invalid_enum_value_rejected() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    let params = HashMap::from([(
        "status".to_string(),
        WidgetParameterValue::Enum("do-not-disturb".to_string()),
    )]);
    let result = scene.publish_to_widget("status-indicator", params, "agent.test", None, 0, None);
    assert!(
        matches!(
            result,
            Err(ValidationError::WidgetParameterInvalidValue { .. })
        ),
        "status='do-not-disturb' should produce WidgetParameterInvalidValue, got: {result:?}"
    );

    // No publication should have been recorded.
    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(
        pubs.len(),
        0,
        "no publication should be recorded when validation fails"
    );
}

// ─── Test 6: Discrete binding snaps — transition_ms ignored for enums ─────────

/// WHEN status=online is published with transition_ms=500, then status=busy is
/// published, THEN the fill snaps immediately to #DC2626 with no interpolation.
///
/// Enum parameters always snap to new values (spec §Widget Parameter
/// Interpolation: "string / enum: snap to new value at t=0").  transition_ms
/// has no effect on the resolved discrete color.
///
/// Source: spec.md §Scenario: Discrete snap on status change
/// [hud-tjq8.2]
#[test]
fn status_discrete_snap_ignores_transition_ms() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    // First publish: status=online with a non-zero transition_ms.
    let params_online = HashMap::from([(
        "status".to_string(),
        WidgetParameterValue::Enum("online".to_string()),
    )]);
    scene
        .publish_to_widget(
            "status-indicator",
            params_online,
            "agent.test",
            None,
            500, // transition_ms
            None,
        )
        .expect("status=online should be accepted");

    // Second publish: status=busy.
    let params_busy = HashMap::from([(
        "status".to_string(),
        WidgetParameterValue::Enum("busy".to_string()),
    )]);
    scene
        .publish_to_widget("status-indicator", params_busy, "agent.test", None, 0, None)
        .expect("status=busy should be accepted");

    // LatestWins: most recent publication wins.
    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(
        pubs.len(),
        1,
        "LatestWins should leave exactly one publication"
    );

    // The stored value is the final enum value — no interpolated intermediate.
    match pubs[0].params.get("status") {
        Some(WidgetParameterValue::Enum(s)) => {
            assert_eq!(
                s, "busy",
                "final status should be 'busy' (discrete snap), got {s:?}"
            );
        }
        other => panic!("expected Enum(\"busy\") for status after snap, got {other:?}"),
    }

    // Binding resolves to #DC2626 — no interpolation between online and busy.
    let bundle = load_fixture(&tokens);
    let layer = &bundle.definition.layers[0];
    let status_binding = layer
        .bindings
        .iter()
        .find(|b| b.param == "status" && b.target_element == "system-fill")
        .expect("status/system-fill binding must exist");

    // At any t ∈ [0.0, 1.0], resolve_binding_value returns the same result
    // for an enum parameter — it uses the stored Enum value, not interpolation.
    let resolved = resolve_binding_value(
        status_binding,
        &pubs[0].params,
        &HashMap::new(), // no f32 constraints needed for discrete
    )
    .expect("discrete binding should resolve");

    assert_eq!(
        resolved, "#DC2626",
        "discrete binding for busy should resolve to #DC2626 (no interpolation), got {resolved:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEXT-CONTENT BINDING TESTS (7–9)
// ═══════════════════════════════════════════════════════════════════════════════

// ─── Test 7: label="Butler" → label-text content is "Butler" ─────────────────

/// WHEN label="Butler" is published THEN the label-text SVG element's text
/// content is replaced with "Butler".
///
/// Source: spec.md §Scenario: Label text updated via text-content binding
/// [hud-tjq8.2]
#[test]
fn label_butler_sets_text_content() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);
    let bundle = load_fixture(&tokens);

    let params = HashMap::from([(
        "label".to_string(),
        WidgetParameterValue::String("Butler".to_string()),
    )]);
    let result = scene.publish_to_widget("status-indicator", params, "agent.test", None, 0, None);
    assert!(
        result.is_ok(),
        "label=\"Butler\" should be accepted, got: {result:?}"
    );

    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(pubs.len(), 1);
    match pubs[0].params.get("label") {
        Some(WidgetParameterValue::String(s)) => {
            assert_eq!(s, "Butler", "published label should be \"Butler\"")
        }
        other => panic!("expected String(\"Butler\") for label, got {other:?}"),
    }

    // Apply the text-content binding to the resolved SVG and verify the
    // element text was updated.
    let svg_bytes = bundle
        .svg_contents
        .get("indicator.svg")
        .expect("indicator.svg must be in svg_contents");
    let svg_text = std::str::from_utf8(svg_bytes).expect("SVG must be valid UTF-8");

    let modified = apply_svg_attribute(svg_text, "label-text", "text-content", "Butler");
    assert!(
        modified.contains("Butler"),
        "SVG after text-content binding should contain 'Butler', got:\n{modified}"
    );
    // The original empty content should be gone.
    assert!(
        !modified.contains("></ "),
        "SVG should not have an empty span after binding"
    );
}

// ─── Test 8: label="" → label-text content is empty ──────────────────────────

/// WHEN label="" is published THEN the label-text SVG element's text content
/// is set to the empty string (no visible text).
///
/// Source: spec.md §Scenario: Empty label clears text content
/// [hud-tjq8.2]
#[test]
fn label_empty_string_clears_text_content() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);
    let bundle = load_fixture(&tokens);

    // First publish a non-empty label so we can verify clearing works.
    let params_set = HashMap::from([(
        "label".to_string(),
        WidgetParameterValue::String("Codex".to_string()),
    )]);
    scene
        .publish_to_widget("status-indicator", params_set, "agent.test", None, 0, None)
        .expect("label=\"Codex\" should be accepted");

    // Now clear the label.
    let params_clear = HashMap::from([(
        "label".to_string(),
        WidgetParameterValue::String("".to_string()),
    )]);
    let result = scene.publish_to_widget(
        "status-indicator",
        params_clear,
        "agent.test",
        None,
        0,
        None,
    );
    assert!(
        result.is_ok(),
        "label=\"\" should be accepted, got: {result:?}"
    );

    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(pubs.len(), 1, "LatestWins: one publication expected");
    match pubs[0].params.get("label") {
        Some(WidgetParameterValue::String(s)) => {
            assert!(
                s.is_empty(),
                "published label should be empty string, got {s:?}"
            )
        }
        other => panic!("expected String(\"\") for label, got {other:?}"),
    }

    // Applying the empty text-content binding to the SVG should produce an
    // element with no character data.
    let svg_bytes = bundle
        .svg_contents
        .get("indicator.svg")
        .expect("indicator.svg must be in svg_contents");
    let svg_text = std::str::from_utf8(svg_bytes).expect("SVG must be valid UTF-8");

    let modified = apply_svg_attribute(svg_text, "label-text", "text-content", "");
    // The label-text opening tag close through the closing tag should contain
    // nothing (empty character data).
    assert!(
        modified.contains("></ ") || {
            // Accept the form "></text>" as empty.
            modified.contains("></text>") || modified.contains("\"></text>")
        },
        "SVG after clearing text-content should have empty label-text element:\n{modified}"
    );
}

// ─── Test 9: label exceeding string_max_bytes=16 → rejection ─────────────────

/// WHEN a label exceeding string_max_bytes=16 is published THEN
/// publish_to_widget returns WIDGET_PARAMETER_INVALID_VALUE.
///
/// Source: spec.md §Scenario: Label exceeding string_max_bytes rejected
/// [hud-tjq8.2]
#[test]
fn label_exceeding_max_bytes_rejected() {
    let tokens = default_tokens();
    let (mut scene, _tab) = scene_with_status_indicator(&tokens);

    // 17 ASCII bytes — one over the string_max_bytes=16 limit.
    let too_long = "A".repeat(17);
    assert_eq!(
        too_long.len(),
        17,
        "test setup: label must be 17 bytes to exceed limit"
    );

    let params = HashMap::from([("label".to_string(), WidgetParameterValue::String(too_long))]);
    let result = scene.publish_to_widget("status-indicator", params, "agent.test", None, 0, None);
    assert!(
        matches!(
            result,
            Err(ValidationError::WidgetParameterInvalidValue { .. })
        ),
        "label exceeding string_max_bytes=16 should produce WidgetParameterInvalidValue, got: {result:?}"
    );

    // No publication should have been recorded.
    let pubs = scene.widget_registry.active_for_widget("status-indicator");
    assert_eq!(
        pubs.len(),
        0,
        "no publication should be recorded when validation fails"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TOKEN PLACEHOLDER RESOLUTION TESTS (10–12)
// ═══════════════════════════════════════════════════════════════════════════════

// ─── Test 10: Default tokens → circle stroke = #333333 ───────────────────────

/// WHEN the bundle is loaded with default tokens THEN the circle element's
/// stroke attribute is resolved to #333333 (the fallback for
/// {{token.color.border.default}}).
///
/// Source: spec.md §Scenario: Token placeholders resolve to canonical fallbacks
/// [hud-tjq8.2]
#[test]
fn default_token_circle_stroke_is_333333() {
    let tokens = default_tokens();
    let bundle = load_fixture(&tokens);

    let svg_bytes = bundle
        .svg_contents
        .get("indicator.svg")
        .expect("indicator.svg must be in svg_contents");
    let svg_text = std::str::from_utf8(svg_bytes).expect("SVG must be valid UTF-8");

    // After token resolution, the SVG must no longer contain the placeholder.
    assert!(
        !svg_text.contains("{{token.color.border.default}}"),
        "placeholder {{{{token.color.border.default}}}} must be resolved in loaded SVG"
    );

    // The resolved stroke value must be #333333.
    assert!(
        svg_text.contains("stroke=\"#333333\"") || svg_text.contains("stroke='#333333'"),
        "circle stroke must be #333333 after default token resolution; SVG:\n{svg_text}"
    );
}

// ─── Test 11: Default tokens → label fill = #B0B0B0 ──────────────────────────

/// WHEN the bundle is loaded with default tokens THEN the label-text element's
/// fill attribute is resolved to #B0B0B0 (the fallback for
/// {{token.color.text.secondary}}).
///
/// Source: spec.md §Scenario: Token placeholders resolve to canonical fallbacks
/// [hud-tjq8.2]
#[test]
fn default_token_label_fill_is_b0b0b0() {
    let tokens = default_tokens();
    let bundle = load_fixture(&tokens);

    let svg_bytes = bundle
        .svg_contents
        .get("indicator.svg")
        .expect("indicator.svg must be in svg_contents");
    let svg_text = std::str::from_utf8(svg_bytes).expect("SVG must be valid UTF-8");

    // After token resolution, the SVG must no longer contain the placeholder.
    assert!(
        !svg_text.contains("{{token.color.text.secondary}}"),
        "placeholder {{{{token.color.text.secondary}}}} must be resolved in loaded SVG"
    );

    // The resolved fill value must be #B0B0B0.
    assert!(
        svg_text.contains("fill=\"#B0B0B0\"") || svg_text.contains("fill='#B0B0B0'"),
        "label fill must be #B0B0B0 after default token resolution; SVG:\n{svg_text}"
    );
}

// ─── Test 12: Custom token override → propagates to SVG attribute ─────────────

/// WHEN the bundle is loaded with a custom token override for
/// color.border.default = #FF0000 THEN the circle stroke attribute is #FF0000
/// in the resolved SVG.
///
/// Source: spec.md §Scenario: Token placeholders resolve to canonical fallbacks
///         (override sub-scenario implied by "Custom token override → propagates
///         to SVG" from hud-tjq8.2 task spec).
/// [hud-tjq8.2]
#[test]
fn custom_token_override_propagates_to_svg() {
    let mut tokens = default_tokens();
    // Override the border color with a distinctive red.
    tokens.insert("color.border.default".to_string(), "#FF0000".to_string());

    let bundle = load_fixture(&tokens);

    let svg_bytes = bundle
        .svg_contents
        .get("indicator.svg")
        .expect("indicator.svg must be in svg_contents");
    let svg_text = std::str::from_utf8(svg_bytes).expect("SVG must be valid UTF-8");

    // The placeholder must be fully resolved.
    assert!(
        !svg_text.contains("{{token.color.border.default}}"),
        "placeholder must be resolved in loaded SVG"
    );

    // The circle stroke must carry the overridden value, not the default.
    assert!(
        svg_text.contains("stroke=\"#FF0000\"") || svg_text.contains("stroke='#FF0000'"),
        "circle stroke should be #FF0000 after custom token override; SVG:\n{svg_text}"
    );
    assert!(
        !svg_text.contains("stroke=\"#333333\""),
        "default stroke #333333 must NOT appear when overridden with #FF0000; SVG:\n{svg_text}"
    );
}
