//! Runtime widget SVG registration wiring shared across runtime/session paths.
//!
//! Connects runtime-registered SVG assets to the widget registry lifecycle:
//! validate structural compatibility, record asset handles, and enqueue SVG
//! bytes for compositor-side registration.

use std::collections::HashMap;

use thiserror::Error;
use tze_hud_scene::graph::SceneGraph;

use crate::loader::validate_runtime_svg_registration;

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
