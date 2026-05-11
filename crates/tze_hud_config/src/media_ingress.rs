//! Windows media-ingress configuration surface.
//!
//! Media ingress is default-off. The only approved first slice is one
//! Windows-local, video-only stream in a config-owned `media-pip` content zone.

use tze_hud_scene::config::{
    APPROVED_MEDIA_ZONE, ConfigError, ConfigErrorCode,
    MediaIngressConfig as ResolvedMediaIngressConfig,
};
use tze_hud_scene::types::{
    ContentionPolicy, GeometryPolicy, LayerAttachment, RenderingPolicy, SceneId,
    TransportConstraint, ZoneDefinition, ZoneMediaType,
};

use crate::profile::resolve_headless_dimensions;
use crate::raw::{RawConfig, RawWidgetGeometry};

pub const REQUIRED_MAX_ACTIVE_STREAMS: u32 = 1;

/// Validate the optional `[media_ingress]` table.
pub fn validate_media_ingress(raw: &RawConfig, errors: &mut Vec<ConfigError>) {
    let Some(media) = &raw.media_ingress else {
        return;
    };

    if !media.enabled {
        return;
    }

    require_eq_str(
        media.approved_zone.as_deref(),
        APPROVED_MEDIA_ZONE,
        "media_ingress.approved_zone",
        "approved media ingress zone must be media-pip",
        errors,
    );

    match media.max_active_streams {
        Some(REQUIRED_MAX_ACTIVE_STREAMS) => {}
        Some(other) => errors.push(invalid_media_ingress(
            "media_ingress.max_active_streams",
            "1",
            other.to_string(),
            "the Windows media exemplar admits exactly one active stream",
        )),
        None => errors.push(invalid_media_ingress(
            "media_ingress.max_active_streams",
            "1",
            "missing",
            "set max_active_streams = 1",
        )),
    }

    if media
        .default_classification
        .as_deref()
        .is_none_or(str::is_empty)
    {
        errors.push(invalid_media_ingress(
            "media_ingress.default_classification",
            "non-empty classification",
            "missing or empty",
            "set a default classification such as \"household\"",
        ));
    }

    if media.operator_disabled.is_none() {
        errors.push(invalid_media_ingress(
            "media_ingress.operator_disabled",
            "explicit boolean",
            "missing",
            "set operator_disabled = true or false so operator state is machine-readable",
        ));
    }

    let (reference_width, reference_height) = resolve_headless_dimensions(raw);
    if reference_width == 0 {
        errors.push(invalid_media_ingress(
            "runtime.headless_width",
            "> 0",
            "0",
            "media-pip geometry normalization requires a positive reference width",
        ));
    }
    if reference_height == 0 {
        errors.push(invalid_media_ingress(
            "runtime.headless_height",
            "> 0",
            "0",
            "media-pip geometry normalization requires a positive reference height",
        ));
    }

    match media.geometry.as_ref() {
        Some(geometry) => validate_fixed_geometry(geometry, errors),
        None => errors.push(invalid_media_ingress(
            "media_ingress.geometry",
            "fixed absolute geometry",
            "missing",
            "add [media_ingress.geometry] with x, y, width, and height",
        )),
    }
}

/// Resolve the frozen media-ingress config, failing closed when validation fails.
pub fn resolve_media_ingress(raw: &RawConfig) -> ResolvedMediaIngressConfig {
    let Some(media) = raw.media_ingress.as_ref() else {
        return ResolvedMediaIngressConfig::default();
    };
    if !media.enabled {
        return ResolvedMediaIngressConfig::default();
    }

    let mut errors = Vec::new();
    validate_media_ingress(raw, &mut errors);
    if !errors.is_empty() {
        return ResolvedMediaIngressConfig::default();
    }

    let (reference_width, reference_height) = resolve_headless_dimensions(raw);
    let reference_width = reference_width as f32;
    let reference_height = reference_height as f32;

    let zone_geometry = match media.geometry.as_ref() {
        Some(RawWidgetGeometry {
            x: Some(x),
            y: Some(y),
            width: Some(width),
            height: Some(height),
            x_pct: None,
            y_pct: None,
            width_pct: None,
            height_pct: None,
        }) if width.is_finite()
            && height.is_finite()
            && x.is_finite()
            && y.is_finite()
            && *width > 0.0
            && *height > 0.0
            && *x >= 0.0
            && *y >= 0.0 =>
        {
            Some(GeometryPolicy::Relative {
                x_pct: *x / reference_width,
                y_pct: *y / reference_height,
                width_pct: *width / reference_width,
                height_pct: *height / reference_height,
            })
        }
        _ => None,
    };

    ResolvedMediaIngressConfig {
        enabled: true,
        approved_zone: Some(APPROVED_MEDIA_ZONE.to_string()),
        zone_geometry,
        max_active_streams: REQUIRED_MAX_ACTIVE_STREAMS,
        default_classification: media.default_classification.clone(),
        operator_disabled: media.operator_disabled.unwrap_or(false),
    }
}

/// Build the approved media-pip zone when media ingress is explicitly enabled.
pub fn approved_media_zone(raw: &RawConfig) -> Option<ZoneDefinition> {
    let resolved = resolve_media_ingress(raw);
    if !resolved.enabled {
        return None;
    }
    Some(ZoneDefinition {
        id: SceneId::new(),
        name: APPROVED_MEDIA_ZONE.to_string(),
        description: "Approved Windows media ingress picture-in-picture zone".to_string(),
        geometry_policy: resolved.zone_geometry?,
        accepted_media_types: vec![ZoneMediaType::VideoSurfaceRef],
        rendering_policy: RenderingPolicy::default(),
        contention_policy: ContentionPolicy::Replace,
        max_publishers: 1,
        transport_constraint: Some(TransportConstraint::WebRtcRequired),
        auto_clear_ms: None,
        ephemeral: false,
        layer_attachment: LayerAttachment::Content,
    })
}

fn require_eq_str(
    got: Option<&str>,
    expected: &str,
    field_path: &str,
    hint: &str,
    errors: &mut Vec<ConfigError>,
) {
    match got {
        Some(actual) if actual == expected => {}
        Some(actual) => errors.push(invalid_media_ingress(
            field_path,
            format!("{expected:?}"),
            format!("{actual:?}"),
            hint,
        )),
        None => errors.push(invalid_media_ingress(
            field_path,
            format!("{expected:?}"),
            "missing",
            hint,
        )),
    }
}

fn validate_fixed_geometry(geometry: &RawWidgetGeometry, errors: &mut Vec<ConfigError>) {
    for (field, value) in [
        ("x", geometry.x),
        ("y", geometry.y),
        ("width", geometry.width),
        ("height", geometry.height),
    ] {
        match value {
            Some(v) if v.is_finite() && v >= 0.0 => {}
            Some(v) => errors.push(invalid_media_ingress(
                format!("media_ingress.geometry.{field}"),
                "finite value >= 0",
                v.to_string(),
                "media-pip geometry must be fixed and non-negative",
            )),
            None => errors.push(invalid_media_ingress(
                format!("media_ingress.geometry.{field}"),
                "required absolute pixel value",
                "missing",
                "media-pip geometry must use x, y, width, and height",
            )),
        }
    }

    if matches!(geometry.width, Some(width) if width <= 0.0) {
        errors.push(invalid_media_ingress(
            "media_ingress.geometry.width",
            "> 0",
            geometry.width.unwrap().to_string(),
            "media-pip width must be positive",
        ));
    }
    if matches!(geometry.height, Some(height) if height <= 0.0) {
        errors.push(invalid_media_ingress(
            "media_ingress.geometry.height",
            "> 0",
            geometry.height.unwrap().to_string(),
            "media-pip height must be positive",
        ));
    }

    for (field, value) in [
        ("x_pct", geometry.x_pct),
        ("y_pct", geometry.y_pct),
        ("width_pct", geometry.width_pct),
        ("height_pct", geometry.height_pct),
    ] {
        if value.is_some() {
            errors.push(invalid_media_ingress(
                format!("media_ingress.geometry.{field}"),
                "absent",
                "present",
                "media-pip geometry must be fixed absolute pixels, not percentages",
            ));
        }
    }
}

fn invalid_media_ingress(
    field_path: impl Into<String>,
    expected: impl Into<String>,
    got: impl Into<String>,
    hint: impl Into<String>,
) -> ConfigError {
    ConfigError {
        code: ConfigErrorCode::ConfigInvalidMediaIngress,
        field_path: field_path.into(),
        expected: expected.into(),
        got: got.into(),
        hint: hint.into(),
    }
}
