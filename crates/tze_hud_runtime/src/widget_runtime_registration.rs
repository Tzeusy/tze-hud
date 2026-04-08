//! Runtime widget SVG registration wiring.
//!
//! Connects runtime-registered SVG assets to the widget registry lifecycle:
//! validate structural compatibility, record asset handles, and enqueue SVG
//! bytes for compositor-side registration.

use std::collections::HashMap;

use thiserror::Error;
use tze_hud_compositor::widget::WidgetRenderer;
use tze_hud_scene::graph::SceneGraph;
use tze_hud_widget::loader::validate_runtime_svg_registration;

/// Errors returned by runtime widget SVG registration.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RuntimeWidgetAssetError {
    /// Widget type or layer filename is invalid for registration.
    #[error("{0}")]
    TypeInvalid(String),
    /// SVG payload failed structural/runtime compatibility validation.
    #[error("{0}")]
    InvalidSvg(String),
}

impl RuntimeWidgetAssetError {
    /// Stable error code for protocol/MCP mappings.
    pub fn wire_code(&self) -> &'static str {
        match self {
            Self::TypeInvalid(_) => "WIDGET_ASSET_TYPE_INVALID",
            Self::InvalidSvg(_) => "WIDGET_ASSET_INVALID_SVG",
        }
    }
}

/// Register a runtime SVG asset into the widget lifecycle.
///
/// This is stage-1 registration plumbing; stage-2 publish remains parameter-only.
pub fn register_runtime_widget_svg_asset(
    scene: &mut SceneGraph,
    widget_type_id: &str,
    svg_filename: &str,
    svg_bytes: &[u8],
    asset_handle: &str,
    tokens: &HashMap<String, String>,
) -> Result<(), RuntimeWidgetAssetError> {
    let definition = scene
        .widget_registry
        .get_definition(widget_type_id)
        .ok_or_else(|| {
            RuntimeWidgetAssetError::TypeInvalid(format!(
                "unknown widget_type_id '{widget_type_id}'"
            ))
        })?;

    if !definition.layers.iter().any(|l| l.svg_file == svg_filename) {
        return Err(RuntimeWidgetAssetError::TypeInvalid(format!(
            "widget type '{widget_type_id}' has no layer '{svg_filename}'"
        )));
    }

    let resolved_svg = validate_runtime_svg_registration(
        definition,
        widget_type_id,
        svg_filename,
        svg_bytes,
        tokens,
    )
    .map_err(|e| RuntimeWidgetAssetError::InvalidSvg(e.to_string()))?;

    scene
        .widget_registry
        .register_runtime_svg_handle(widget_type_id, svg_filename, asset_handle);
    scene.enqueue_widget_svg_asset(widget_type_id, svg_filename, resolved_svg);
    Ok(())
}

/// Register pending widget SVG assets in the compositor widget renderer.
pub fn process_pending_widget_svgs<I>(
    widget_renderer: Option<&mut WidgetRenderer>,
    pending_widget_svgs: I,
) where
    I: IntoIterator<Item = crate::widget_startup::WidgetSvgAsset>,
{
    if let Some(wr) = widget_renderer {
        for (type_id, filename, bytes) in pending_widget_svgs {
            wr.register_svg(&type_id, &filename, bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::types::{
        ContentionPolicy, GeometryPolicy, RenderingPolicy, WidgetBinding, WidgetBindingMapping,
        WidgetDefinition, WidgetParamType, WidgetParameterDeclaration, WidgetParameterValue,
        WidgetSvgLayer,
    };

    fn make_scene() -> SceneGraph {
        let mut scene = SceneGraph::new(800.0, 600.0);
        scene.widget_registry.register_definition(WidgetDefinition {
            id: "gauge".to_string(),
            name: "Gauge".to_string(),
            description: "test".to_string(),
            parameter_schema: vec![WidgetParameterDeclaration {
                name: "level".to_string(),
                param_type: WidgetParamType::F32,
                default_value: WidgetParameterValue::F32(0.0),
                constraints: None,
            }],
            layers: vec![WidgetSvgLayer {
                svg_file: "fill.svg".to_string(),
                bindings: vec![WidgetBinding {
                    param: "level".to_string(),
                    target_element: "bar".to_string(),
                    target_attribute: "height".to_string(),
                    mapping: WidgetBindingMapping::Linear {
                        attr_min: 0.0,
                        attr_max: 100.0,
                    },
                }],
            }],
            default_geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 1.0,
            },
            default_rendering_policy: RenderingPolicy::default(),
            default_contention_policy: ContentionPolicy::LatestWins,
            ephemeral: false,
            hover_behavior: None,
        });
        scene
    }

    #[test]
    fn runtime_registration_records_handle_and_enqueues_svg() {
        let mut scene = make_scene();
        let svg = br#"<svg viewBox="0 0 100 100"><rect id="bar" width="10" height="20"/></svg>"#;
        register_runtime_widget_svg_asset(
            &mut scene,
            "gauge",
            "fill.svg",
            svg,
            "asset:abc123",
            &HashMap::new(),
        )
        .expect("registration should succeed");

        assert_eq!(
            scene
                .widget_registry
                .runtime_svg_handle("gauge", "fill.svg"),
            Some("asset:abc123")
        );
        let drained = scene.drain_pending_widget_svg_assets();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].0, "gauge");
        assert_eq!(drained[0].1, "fill.svg");
    }

    #[test]
    fn runtime_registration_rejects_incompatible_svg_targets() {
        let mut scene = make_scene();
        let bad_svg =
            br#"<svg viewBox="0 0 100 100"><rect id="not-bar" width="10" height="20"/></svg>"#;
        let err = register_runtime_widget_svg_asset(
            &mut scene,
            "gauge",
            "fill.svg",
            bad_svg,
            "asset:oops",
            &HashMap::new(),
        )
        .expect_err("registration must fail when bindings cannot resolve");
        assert_eq!(err.wire_code(), "WIDGET_ASSET_INVALID_SVG");
        assert!(
            scene
                .widget_registry
                .runtime_svg_handle("gauge", "fill.svg")
                .is_none()
        );
    }
}
