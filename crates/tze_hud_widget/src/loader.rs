//! Widget asset bundle directory scanner and loader.
//!
//! # Bundle Layout
//!
//! A valid bundle is a directory with:
//! - `widget.toml` — the manifest (required)
//! - One or more `.svg` files referenced by the manifest (required for each layer)
//!
//! # Bundle Scan Algorithm
//!
//! 1. For each configured bundle path, enumerate immediate subdirectories.
//! 2. For each subdirectory, attempt to load a bundle.
//! 3. If loading fails, log the structured error and continue (do not abort).
//! 4. If loading succeeds but the widget type name duplicates an already-loaded
//!    bundle, reject the new bundle with `WIDGET_BUNDLE_DUPLICATE_TYPE`.
//!
//! Source: widget-system/spec.md §Requirement: Widget Asset Bundle Format,
//!         §Requirement: SVG Layer Parameter Bindings.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tze_hud_resource::validation::parse_svg_dimensions;
use tze_hud_scene::types::{
    ContentionPolicy, GeometryPolicy, RenderingPolicy, WidgetBinding, WidgetBindingMapping,
    WidgetDefinition, WidgetParamConstraints, WidgetParamType, WidgetParameterDeclaration,
    WidgetParameterValue, WidgetSvgLayer,
};

use crate::error::BundleError;
use crate::manifest::{RawBinding, RawManifest, RawParameterDeclaration};
use crate::svg_ids::collect_svg_element_ids;

// ─── Bundle loader ─────────────────────────────────────────────────────────────

/// Result of loading a single widget asset bundle.
#[derive(Debug)]
pub struct LoadedBundle {
    /// The widget type definition, ready to register into WidgetRegistry.
    pub definition: WidgetDefinition,
    /// Raw SVG bytes keyed by filename within the bundle directory.
    /// These can be uploaded to the resource store as IMAGE_SVG resources.
    pub svg_contents: HashMap<String, Vec<u8>>,
}

/// Outcome of scanning a bundle directory: either a successful load or a
/// structured error (the error is logged but does not abort scanning).
#[derive(Debug)]
pub enum BundleScanResult {
    Ok(LoadedBundle),
    Err(BundleError),
}

/// Scan one or more bundle root directories and load all valid widget bundles.
///
/// For each root path, every immediate subdirectory is treated as a potential
/// bundle.  Failed bundles are returned as `BundleScanResult::Err` entries and
/// logged at `WARN` level; they do not prevent other bundles from loading.
///
/// Duplicate widget type names across bundles produce a
/// `WIDGET_BUNDLE_DUPLICATE_TYPE` error for the second bundle.
///
/// # Arguments
///
/// - `bundle_roots`: directories to scan; each immediate subdirectory is a
///   potential bundle.
/// - `tokens`: design-token map used to resolve `{{token.key}}` placeholders in
///   SVG files.  Pass an empty map when no token substitution is needed.
///
/// Source: widget-system/spec.md §Requirement: Widget Asset Bundle Format.
pub fn scan_bundle_dirs(
    bundle_roots: &[PathBuf],
    tokens: &HashMap<String, String>,
) -> Vec<BundleScanResult> {
    let mut results: Vec<BundleScanResult> = Vec::new();
    // Track registered names to detect duplicates.
    let mut registered: HashMap<String, PathBuf> = HashMap::new();

    for root in bundle_roots {
        let read_dir = match std::fs::read_dir(root) {
            Ok(rd) => rd,
            Err(e) => {
                tracing::warn!(
                    path = %root.display(),
                    error = %e,
                    "widget bundle root not readable, skipping"
                );
                continue;
            }
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue; // skip non-directory entries
            }

            let result = load_bundle_dir_with_tokens(&path, tokens);
            match &result {
                BundleScanResult::Ok(bundle) => {
                    let name = bundle.definition.id.clone();
                    if let Some(existing) = registered.get(&name) {
                        let err = BundleError::DuplicateType {
                            name: name.clone(),
                            existing_path: existing.display().to_string(),
                            new_path: path.display().to_string(),
                        };
                        tracing::warn!(wire_code = err.wire_code(), "{}", err);
                        results.push(BundleScanResult::Err(err));
                        continue;
                    }
                    registered.insert(name, path.clone());
                    tracing::info!(
                        widget_name = bundle.definition.id,
                        path = %path.display(),
                        "loaded widget bundle"
                    );
                }
                BundleScanResult::Err(err) => {
                    tracing::warn!(
                        wire_code = err.wire_code(),
                        path = %path.display(),
                        "{}",
                        err
                    );
                }
            }
            results.push(result);
        }
    }

    results
}

/// Load a single bundle directory with no token substitution.
///
/// Returns `BundleScanResult::Ok` on success, or `BundleScanResult::Err` with
/// the first structural error encountered.  A rejected bundle does not prevent
/// other bundles from loading.
pub fn load_bundle_dir(dir: &Path) -> BundleScanResult {
    load_bundle_dir_with_tokens(dir, &HashMap::new())
}

/// Load a single bundle directory, substituting design-token placeholders in
/// SVG files using the supplied `tokens` map.
///
/// Returns `BundleScanResult::Ok` on success, or `BundleScanResult::Err` with
/// the first structural error encountered.  A rejected bundle does not prevent
/// other bundles from loading.
pub fn load_bundle_dir_with_tokens(
    dir: &Path,
    tokens: &HashMap<String, String>,
) -> BundleScanResult {
    let path_str = dir.display().to_string();
    match load_bundle_dir_inner(dir, &path_str, tokens) {
        Ok(bundle) => BundleScanResult::Ok(bundle),
        Err(e) => BundleScanResult::Err(e),
    }
}

fn load_bundle_dir_inner(
    dir: &Path,
    path_str: &str,
    tokens: &HashMap<String, String>,
) -> Result<LoadedBundle, BundleError> {
    // Step 1: Locate widget.toml.
    let manifest_path = dir.join("widget.toml");
    if !manifest_path.exists() {
        return Err(BundleError::NoManifest {
            path: path_str.to_string(),
        });
    }

    // Step 2: Read and parse widget.toml.
    let toml_str =
        std::fs::read_to_string(&manifest_path).map_err(|e| BundleError::InvalidManifest {
            path: path_str.to_string(),
            detail: format!("cannot read widget.toml: {e}"),
        })?;

    let raw: RawManifest = toml::from_str(&toml_str).map_err(|e| BundleError::InvalidManifest {
        path: path_str.to_string(),
        detail: format!("TOML parse error: {e}"),
    })?;

    // Step 3: Validate required manifest fields.
    let name = raw
        .name
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| BundleError::InvalidManifest {
            path: path_str.to_string(),
            detail: "missing required field 'name'".to_string(),
        })?;

    // Step 3a: Validate widget type id format: [a-z][a-z0-9-]*
    if !is_valid_widget_type_id(name) {
        return Err(BundleError::InvalidName {
            path: path_str.to_string(),
            name: name.to_string(),
        });
    }

    let version = raw
        .version
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| BundleError::InvalidManifest {
            path: path_str.to_string(),
            detail: "missing required field 'version'".to_string(),
        })?;

    let description = raw.description.as_deref().unwrap_or("").to_string();

    // Step 4: Parse parameter schema.
    let parameter_schema = parse_parameter_schema(&raw.parameter_schema, path_str)?;

    // Build a set of parameter names for binding validation.
    let param_names: HashSet<&str> = parameter_schema.iter().map(|p| p.name.as_str()).collect();
    // Build a map from param name to type for mapping validation.
    let param_types: HashMap<&str, WidgetParamType> = parameter_schema
        .iter()
        .map(|p| (p.name.as_str(), p.param_type))
        .collect();

    // Step 5: Load SVG files and resolve bindings.
    let mut svg_contents: HashMap<String, Vec<u8>> = HashMap::new();
    let mut layers: Vec<WidgetSvgLayer> = Vec::new();

    for raw_layer in &raw.layers {
        let svg_file = raw_layer
            .svg_file
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| BundleError::InvalidManifest {
                path: path_str.to_string(),
                detail: "a layer entry is missing required field 'svg_file'".to_string(),
            })?;

        // Step 5a: Verify SVG file exists.
        let svg_path = dir.join(svg_file);
        if !svg_path.exists() {
            return Err(BundleError::MissingSvg {
                path: path_str.to_string(),
                svg_file: svg_file.to_string(),
            });
        }

        // Step 5b: Read and validate SVG.
        let svg_bytes = std::fs::read(&svg_path).map_err(|e| BundleError::SvgParseError {
            path: path_str.to_string(),
            svg_file: svg_file.to_string(),
            detail: format!("cannot read file: {e}"),
        })?;

        let svg_text = std::str::from_utf8(&svg_bytes).map_err(|e| BundleError::SvgParseError {
            path: path_str.to_string(),
            svg_file: svg_file.to_string(),
            detail: format!("file is not valid UTF-8: {e}"),
        })?;

        // Step 5b-post: Resolve {{token.key}} placeholders BEFORE SVG parse/scan.
        let svg_text_resolved = resolve_token_placeholders(svg_text, tokens).map_err(|key| {
            BundleError::UnresolvedToken {
                path: path_str.to_string(),
                svg_file: svg_file.to_string(),
                token_key: key,
            }
        })?;
        let svg_text = svg_text_resolved.as_str();

        // Validate SVG (well-formed XML + <svg> root check).
        parse_svg_dimensions(svg_text).map_err(|e| BundleError::SvgParseError {
            path: path_str.to_string(),
            svg_file: svg_file.to_string(),
            detail: e.to_string(),
        })?;

        // Step 5c: Collect element IDs for binding resolution.
        let element_ids =
            collect_svg_element_ids(svg_text).map_err(|e| BundleError::SvgParseError {
                path: path_str.to_string(),
                svg_file: svg_file.to_string(),
                detail: e,
            })?;

        // Step 5d: Resolve bindings.
        let bindings = resolve_bindings(
            &raw_layer.bindings,
            svg_file,
            &element_ids,
            &param_names,
            &param_types,
            path_str,
        )?;

        // Store the resolved SVG text (post-substitution) as bytes.
        svg_contents.insert(svg_file.to_string(), svg_text.as_bytes().to_vec());
        layers.push(WidgetSvgLayer {
            svg_file: svg_file.to_string(),
            bindings,
        });
    }

    // Step 6: Build WidgetDefinition.
    let contention_policy =
        parse_contention_policy(raw.default_contention_policy.as_deref(), path_str)?;
    let rendering_policy =
        parse_rendering_policy(raw.default_rendering_policy.as_deref(), path_str)?;

    // Default geometry: full display area (100% × 100% at origin).
    // Widget instances will override this via config.
    let default_geometry = GeometryPolicy::Relative {
        x_pct: 0.0,
        y_pct: 0.0,
        width_pct: 1.0,
        height_pct: 1.0,
    };

    let definition = WidgetDefinition {
        id: name.to_string(),
        name: name.to_string(),
        description,
        parameter_schema,
        layers,
        default_geometry_policy: default_geometry,
        default_rendering_policy: rendering_policy,
        default_contention_policy: contention_policy,
        ephemeral: false,
    };

    tracing::debug!(
        widget_name = definition.id,
        version = version,
        svg_files = svg_contents.len(),
        "widget bundle loaded successfully"
    );

    Ok(LoadedBundle {
        definition,
        svg_contents,
    })
}

// ─── Parameter schema parsing ─────────────────────────────────────────────────

fn parse_parameter_schema(
    raw: &[RawParameterDeclaration],
    path_str: &str,
) -> Result<Vec<WidgetParameterDeclaration>, BundleError> {
    let mut params = Vec::new();
    for raw_param in raw {
        let name = raw_param
            .name
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| BundleError::InvalidManifest {
                path: path_str.to_string(),
                detail: "parameter_schema entry missing required field 'name'".to_string(),
            })?;

        let type_str = raw_param
            .param_type
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| BundleError::InvalidManifest {
                path: path_str.to_string(),
                detail: format!("parameter '{name}' missing required field 'type'"),
            })?;

        let param_type = parse_param_type(type_str).ok_or_else(|| {
            BundleError::InvalidManifest {
                path: path_str.to_string(),
                detail: format!(
                    "parameter '{name}': unknown type '{type_str}' (must be f32, string, color, or enum)"
                ),
            }
        })?;

        let default_value =
            parse_default_value(raw_param.default.as_ref(), param_type, name, path_str)?;

        let constraints = raw_param.constraints.as_ref().map(|c| {
            let mut wc = WidgetParamConstraints::default();
            if let Some(v) = c.f32_min {
                wc.f32_min = Some(v as f32);
            }
            if let Some(v) = c.f32_max {
                wc.f32_max = Some(v as f32);
            }
            if let Some(v) = c.string_max_bytes {
                wc.string_max_bytes = Some(v);
            }
            if !c.enum_allowed_values.is_empty() {
                wc.enum_allowed_values = c.enum_allowed_values.clone();
            }
            wc
        });

        params.push(WidgetParameterDeclaration {
            name: name.to_string(),
            param_type,
            default_value,
            constraints,
        });
    }
    Ok(params)
}

fn parse_param_type(s: &str) -> Option<WidgetParamType> {
    match s {
        "f32" => Some(WidgetParamType::F32),
        "string" => Some(WidgetParamType::String),
        "color" => Some(WidgetParamType::Color),
        "enum" => Some(WidgetParamType::Enum),
        _ => None,
    }
}

fn parse_default_value(
    raw: Option<&toml::Value>,
    param_type: WidgetParamType,
    name: &str,
    path_str: &str,
) -> Result<WidgetParameterValue, BundleError> {
    let raw = raw.ok_or_else(|| BundleError::InvalidManifest {
        path: path_str.to_string(),
        detail: format!("parameter '{name}' missing required field 'default'"),
    })?;

    let type_err = || BundleError::InvalidManifest {
        path: path_str.to_string(),
        detail: format!(
            "parameter '{name}': 'default' value type mismatch for type {param_type:?}"
        ),
    };

    match param_type {
        WidgetParamType::F32 => {
            let v = match raw {
                toml::Value::Float(f) => *f as f32,
                toml::Value::Integer(i) => *i as f32,
                _ => return Err(type_err()),
            };
            Ok(WidgetParameterValue::F32(v))
        }
        WidgetParamType::String => {
            let s = raw.as_str().ok_or_else(type_err)?;
            Ok(WidgetParameterValue::String(s.to_string()))
        }
        WidgetParamType::Color => {
            // Expect array of 4 integers [r, g, b, a].
            let arr = raw.as_array().ok_or_else(type_err)?;
            if arr.len() != 4 {
                return Err(BundleError::InvalidManifest {
                    path: path_str.to_string(),
                    detail: format!(
                        "parameter '{name}': color default must be [r, g, b, a] (4 integers)"
                    ),
                });
            }
            let mut components = [0u8; 4];
            for (i, v) in arr.iter().enumerate() {
                let int = v.as_integer().ok_or_else(|| BundleError::InvalidManifest {
                    path: path_str.to_string(),
                    detail: format!("parameter '{name}': color component {i} must be an integer"),
                })?;
                if !(0..=255).contains(&int) {
                    return Err(BundleError::InvalidManifest {
                        path: path_str.to_string(),
                        detail: format!(
                            "parameter '{name}': color component {i} value {int} out of range [0, 255]"
                        ),
                    });
                }
                components[i] = int as u8;
            }
            // WidgetParameterValue::Color uses Rgba with f32 [0.0, 1.0] components.
            use tze_hud_scene::types::Rgba;
            Ok(WidgetParameterValue::Color(Rgba::new(
                components[0] as f32 / 255.0,
                components[1] as f32 / 255.0,
                components[2] as f32 / 255.0,
                components[3] as f32 / 255.0,
            )))
        }
        WidgetParamType::Enum => {
            let s = raw.as_str().ok_or_else(type_err)?;
            Ok(WidgetParameterValue::Enum(s.to_string()))
        }
    }
}

// ─── Binding resolution ────────────────────────────────────────────────────────

/// Resolve and validate all bindings for a single layer.
///
/// Source: widget-system/spec.md §Requirement: SVG Layer Parameter Bindings.
fn resolve_bindings(
    raw_bindings: &[RawBinding],
    svg_file: &str,
    element_ids: &HashSet<String>,
    param_names: &HashSet<&str>,
    param_types: &HashMap<&str, WidgetParamType>,
    path_str: &str,
) -> Result<Vec<WidgetBinding>, BundleError> {
    let mut bindings = Vec::new();

    for raw_b in raw_bindings {
        let param = raw_b
            .param
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| BundleError::InvalidManifest {
                path: path_str.to_string(),
                detail: format!("layer '{svg_file}': binding missing required field 'param'"),
            })?;

        let target_element = raw_b
            .target_element
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| BundleError::InvalidManifest {
                path: path_str.to_string(),
                detail: format!(
                    "layer '{svg_file}': binding for param '{param}' missing 'target_element'"
                ),
            })?;

        let target_attribute = raw_b
            .target_attribute
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| BundleError::InvalidManifest {
                path: path_str.to_string(),
                detail: format!(
                    "layer '{svg_file}': binding for param '{param}' missing 'target_attribute'"
                ),
            })?;

        let mapping_str = raw_b
            .mapping
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| BundleError::InvalidManifest {
                path: path_str.to_string(),
                detail: format!(
                    "layer '{svg_file}': binding for param '{param}' missing 'mapping'"
                ),
            })?;

        // Validate: param name must exist in the parameter schema.
        if !param_names.contains(param) {
            return Err(BundleError::BindingUnresolvable {
                path: path_str.to_string(),
                detail: format!(
                    "layer '{svg_file}': binding references nonexistent parameter '{param}'"
                ),
            });
        }

        // Validate: target_element must exist in the SVG (except for text-content,
        // where any element with an id is valid — we still require the element exists).
        if !element_ids.contains(target_element) {
            return Err(BundleError::BindingUnresolvable {
                path: path_str.to_string(),
                detail: format!(
                    "layer '{svg_file}': binding references nonexistent SVG element id '{target_element}'"
                ),
            });
        }

        let param_type = *param_types.get(param).unwrap(); // checked above

        // Validate and parse the mapping.
        let mapping =
            parse_binding_mapping(mapping_str, raw_b, param, param_type, svg_file, path_str)?;

        bindings.push(WidgetBinding {
            param: param.to_string(),
            target_element: target_element.to_string(),
            target_attribute: target_attribute.to_string(),
            mapping,
        });
    }

    Ok(bindings)
}

/// Parse and validate a binding mapping.
///
/// Validates that the mapping type is compatible with the parameter type:
/// - `linear` is only valid for f32 parameters.
/// - `direct` is valid for string and color parameters.
/// - `discrete` is only valid for enum parameters.
fn parse_binding_mapping(
    mapping_str: &str,
    raw_b: &RawBinding,
    param: &str,
    param_type: WidgetParamType,
    svg_file: &str,
    path_str: &str,
) -> Result<WidgetBindingMapping, BundleError> {
    match mapping_str {
        "linear" => {
            if param_type != WidgetParamType::F32 {
                return Err(BundleError::BindingUnresolvable {
                    path: path_str.to_string(),
                    detail: format!(
                        "layer '{svg_file}': binding param '{param}' uses 'linear' mapping but type is {param_type:?} (linear is only valid for f32)"
                    ),
                });
            }
            let attr_min = raw_b.attr_min.unwrap_or(0.0) as f32;
            let attr_max = raw_b.attr_max.unwrap_or(1.0) as f32;
            Ok(WidgetBindingMapping::Linear { attr_min, attr_max })
        }
        "direct" => {
            if param_type != WidgetParamType::String && param_type != WidgetParamType::Color {
                return Err(BundleError::BindingUnresolvable {
                    path: path_str.to_string(),
                    detail: format!(
                        "layer '{svg_file}': binding param '{param}' uses 'direct' mapping but type is {param_type:?} (direct is only valid for string and color)"
                    ),
                });
            }
            Ok(WidgetBindingMapping::Direct)
        }
        "discrete" => {
            if param_type != WidgetParamType::Enum {
                return Err(BundleError::BindingUnresolvable {
                    path: path_str.to_string(),
                    detail: format!(
                        "layer '{svg_file}': binding param '{param}' uses 'discrete' mapping but type is {param_type:?} (discrete is only valid for enum)"
                    ),
                });
            }
            Ok(WidgetBindingMapping::Discrete {
                value_map: raw_b.value_map.clone(),
            })
        }
        other => Err(BundleError::BindingUnresolvable {
            path: path_str.to_string(),
            detail: format!(
                "layer '{svg_file}': binding param '{param}' has unknown mapping type '{other}'"
            ),
        }),
    }
}

// ─── Widget type id validation ────────────────────────────────────────────────

/// Returns `true` if `id` conforms to the widget type id format: `[a-z][a-z0-9-]*`.
///
/// The id must:
/// - start with a lowercase ASCII letter (`a`–`z`),
/// - contain only lowercase ASCII letters, ASCII digits, or hyphens (`-`).
///
/// Source: scene-graph/spec.md §Widget Type Identifier.
pub(crate) fn is_valid_widget_type_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        // Must start with a lowercase letter.
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    // Remaining characters must be lowercase letters, digits, or hyphens.
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ─── Policy helpers ────────────────────────────────────────────────────────────

fn parse_contention_policy(
    s: Option<&str>,
    _path_str: &str,
) -> Result<ContentionPolicy, BundleError> {
    Ok(match s {
        None | Some("LatestWins") | Some("latest_wins") => ContentionPolicy::LatestWins,
        Some("Stack") | Some("stack") => ContentionPolicy::Stack { max_depth: 8 },
        Some("Replace") | Some("replace") => ContentionPolicy::Replace,
        _ => ContentionPolicy::LatestWins,
    })
}

fn parse_rendering_policy(
    s: Option<&str>,
    _path_str: &str,
) -> Result<RenderingPolicy, BundleError> {
    // Default rendering policy when not specified.
    let _ = s;
    Ok(RenderingPolicy::default())
}

// ─── Token placeholder resolution ────────────────────────────────────────────

/// Resolve `{{token.key}}` mustache-style placeholders in SVG text.
///
/// # Syntax
///
/// - A placeholder has the form `{{token.key}}` where `key` matches the pattern
///   `[a-z][a-z0-9]*(?:\.[a-z][a-z0-9_]*)*` — no whitespace inside the braces.
/// - The token lookup key is the full dotted path after `token.` (e.g. the
///   placeholder `{{token.color.primary}}` looks up the key `color.primary`).
/// - The function performs a single left-to-right pass with no recursive
///   re-scanning of substituted values.
///
/// # Escape sequences
///
/// Literal `{{` and `}}` can be written as `\{\{` / `\}\}` in the SVG source
/// (each brace individually backslash-escaped, per the spec).  These are
/// replaced with sentinels before scanning and restored afterwards, ensuring
/// they are never treated as placeholder delimiters.
///
/// # Errors
///
/// Returns `Err(token_key)` if a valid-syntax placeholder references a key not
/// present in `tokens`.  Unknown-syntax sequences (e.g. whitespace inside
/// braces) are passed through unchanged and never produce an error.
///
/// # Guarantees
///
/// - Single left-to-right pass: resolved substitution values are never
///   re-scanned for further placeholders.
/// - Placeholders in `<style>` blocks are resolved identically to any other
///   text content.
/// - UTF-8 safe: uses string-level `find` for all scanning; no raw byte casts.
pub(crate) fn resolve_token_placeholders(
    svg_text: &str,
    tokens: &HashMap<String, String>,
) -> Result<String, String> {
    // Sentinel strings that cannot appear in valid SVG/XML.
    const ESC_OPEN: &str = "\x00LBRACE\x00";
    const ESC_CLOSE: &str = "\x00RBRACE\x00";

    // Step 1: Replace escape sequences with sentinels.
    // The spec escape format is \{\{ / \}\} (each brace individually escaped).
    let work = svg_text
        .replace("\\{\\{", ESC_OPEN)
        .replace("\\}\\}", ESC_CLOSE);

    // Step 2: Single left-to-right scan using string-level find().
    // This is UTF-8 safe: `find` returns byte positions at character
    // boundaries; we only slice at those positions.
    let mut result = String::with_capacity(work.len());
    let mut remaining = work.as_str();

    while let Some(open_pos) = remaining.find("{{") {
        // Append everything before the `{{`.
        result.push_str(&remaining[..open_pos]);
        let after_open = &remaining[open_pos + 2..];

        // Find the matching `}}`.
        if let Some(close_offset) = after_open.find("}}") {
            let inner = &after_open[..close_offset];

            // Validate: must be `token.<key>` with no whitespace.
            if let Some(key_part) = inner.strip_prefix("token.") {
                if is_valid_token_key(key_part) {
                    // Resolve against the token map.
                    match tokens.get(key_part) {
                        Some(value) => {
                            result.push_str(value);
                            remaining = &after_open[close_offset + 2..]; // skip past `}}`
                            continue;
                        }
                        None => {
                            // Unresolved token — return the key as the error.
                            return Err(key_part.to_string());
                        }
                    }
                }
            }
            // Inner text didn't match token syntax — pass `{{` through literally
            // and advance past just the `{{` so the inner text is re-scanned.
            result.push_str("{{");
            remaining = after_open;
        } else {
            // No closing `}}` found — pass `{{` through and stop scanning.
            result.push_str("{{");
            remaining = after_open;
        }
    }

    // Append any remaining text after the last `{{` (or the whole string if
    // no `{{` was found).
    result.push_str(remaining);

    // Step 3: Restore sentinels to their literal form.
    let result = result.replace(ESC_OPEN, "{{").replace(ESC_CLOSE, "}}");

    Ok(result)
}

/// Returns `true` if `key` matches `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*`.
///
/// Pattern breakdown:
/// - First segment: `[a-z][a-z0-9]*` — lowercase letter followed by lowercase
///   letters and digits only (no underscores).
/// - Each additional segment: `[a-z][a-z0-9_]*` — lowercase letter followed by
///   lowercase letters, digits, and underscores.
///
/// This is the allowed token key syntax within `{{token.<key>}}` placeholders.
fn is_valid_token_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    for (segment_index, segment) in key.split('.').enumerate() {
        let mut chars = segment.chars();
        match chars.next() {
            Some(first) if first.is_ascii_lowercase() => {}
            _ => return false,
        }
        if segment_index == 0 {
            // First segment: only lowercase letters and digits (no underscores).
            if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
                return false;
            }
        } else {
            // Subsequent segments: lowercase letters, digits, and underscores.
            if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                return false;
            }
        }
    }
    true
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::is_valid_widget_type_id;

    // Valid ids.
    #[test]
    fn valid_single_letter() {
        assert!(is_valid_widget_type_id("a"));
    }

    #[test]
    fn valid_simple_name() {
        assert!(is_valid_widget_type_id("gauge"));
    }

    #[test]
    fn valid_with_digits() {
        assert!(is_valid_widget_type_id("widget123"));
    }

    #[test]
    fn valid_with_hyphens() {
        assert!(is_valid_widget_type_id("my-widget"));
    }

    #[test]
    fn valid_complex_name() {
        assert!(is_valid_widget_type_id("a1b2-c3d4"));
    }

    #[test]
    fn valid_trailing_digit() {
        assert!(is_valid_widget_type_id("progress-bar2"));
    }

    // Invalid ids.
    #[test]
    fn invalid_empty() {
        assert!(!is_valid_widget_type_id(""));
    }

    #[test]
    fn invalid_starts_with_digit() {
        assert!(!is_valid_widget_type_id("1gauge"));
    }

    #[test]
    fn invalid_starts_with_hyphen() {
        assert!(!is_valid_widget_type_id("-gauge"));
    }

    #[test]
    fn invalid_uppercase() {
        assert!(!is_valid_widget_type_id("Gauge"));
    }

    #[test]
    fn invalid_all_uppercase() {
        assert!(!is_valid_widget_type_id("GAUGE"));
    }

    #[test]
    fn invalid_contains_uppercase() {
        assert!(!is_valid_widget_type_id("my-Gauge"));
    }

    #[test]
    fn invalid_space() {
        assert!(!is_valid_widget_type_id("my gauge"));
    }

    #[test]
    fn invalid_underscore() {
        assert!(!is_valid_widget_type_id("my_gauge"));
    }

    #[test]
    fn invalid_dot() {
        assert!(!is_valid_widget_type_id("my.gauge"));
    }

    #[test]
    fn invalid_slash() {
        assert!(!is_valid_widget_type_id("my/gauge"));
    }

    // ─── resolve_token_placeholders ──────────────────────────────────────────────

    use super::resolve_token_placeholders;
    use std::collections::HashMap;

    fn token_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Single placeholder is substituted with the token value.
    #[test]
    fn single_placeholder_substituted() {
        let tokens = token_map(&[("color.primary", "#ff0000")]);
        let input = r##"<rect fill="{{token.color.primary}}"/>"##;
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, r##"<rect fill="#ff0000"/>"##);
    }

    /// Multiple placeholders in one attribute value are all substituted.
    #[test]
    fn multiple_placeholders_in_one_attribute() {
        let tokens = token_map(&[("fg", "white"), ("bg", "black")]);
        let input = r#"<text fill="{{token.fg}}" stroke="{{token.bg}}">x</text>"#;
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, r#"<text fill="white" stroke="black">x</text>"#);
    }

    /// Escaped braces `\{\{` / `\}\}` are preserved as literal `{{` / `}}`.
    ///
    /// The spec escape format requires each brace to be individually escaped:
    /// `\{\{` (not `\{{`) and `\}\}` (not `\}}`).
    #[test]
    fn escaped_braces_preserved_as_literals() {
        let tokens = token_map(&[]);
        // Each brace is individually escaped: \{ \{ and \} \}
        let input = r"no placeholder \{\{ here \}\} either";
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, "no placeholder {{ here }} either");
    }

    /// An unresolved token (valid syntax, key absent from map) produces an error.
    #[test]
    fn unresolved_token_yields_error() {
        let tokens = token_map(&[]);
        let input = r#"<rect fill="{{token.missing.key}}"/>"#;
        let err = resolve_token_placeholders(input, &tokens).unwrap_err();
        assert_eq!(err, "missing.key");
    }

    /// Resolved values are never re-scanned (no recursive substitution).
    #[test]
    fn no_recursive_substitution() {
        // The value itself looks like a placeholder; it must NOT be re-resolved.
        let tokens = token_map(&[("a", "{{token.b}}"), ("b", "SHOULD_NOT_APPEAR")]);
        let input = r#"<text>{{token.a}}</text>"#;
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        // The value "{{token.b}}" is inserted verbatim; it should NOT be expanded.
        assert_eq!(result, r#"<text>{{token.b}}</text>"#);
    }

    /// Placeholder inside a `<style>` block is resolved identically to any attribute.
    #[test]
    fn placeholder_inside_style_block() {
        let tokens = token_map(&[("color.accent", "blue")]);
        let input = "<style>.cls { fill: {{token.color.accent}}; }</style>";
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, "<style>.cls { fill: blue; }</style>");
    }

    /// `{{ token.key }}` with whitespace inside braces is NOT treated as a placeholder.
    #[test]
    fn whitespace_inside_braces_not_a_placeholder() {
        let tokens = token_map(&[("color.primary", "SHOULD_NOT_APPEAR")]);
        // The spec requires no whitespace inside braces.
        let input = r#"<rect fill="{{ token.color.primary }}"/>"#;
        // Should pass through unchanged (no match).
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, input);
    }

    /// A placeholder whose key is not under `token.` namespace is passed through unchanged.
    #[test]
    fn non_token_namespace_passed_through() {
        let tokens = token_map(&[]);
        let input = "{{other.key}} stays put";
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, "{{other.key}} stays put");
    }

    /// A bare `{{}}` (empty inner) is passed through unchanged.
    #[test]
    fn empty_braces_passed_through() {
        let tokens = token_map(&[]);
        let result = resolve_token_placeholders("{{}}", &tokens).unwrap();
        assert_eq!(result, "{{}}");
    }

    /// Unclosed `{{` is passed through unchanged without panicking.
    #[test]
    fn unclosed_braces_passed_through() {
        let tokens = token_map(&[]);
        let result = resolve_token_placeholders("{{ no close", &tokens).unwrap();
        assert_eq!(result, "{{ no close");
    }

    /// A key whose first segment contains an underscore is NOT a valid placeholder.
    ///
    /// The spec pattern `[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)*)` disallows
    /// underscores in the first segment; only subsequent segments permit them.
    #[test]
    fn underscore_in_first_segment_not_a_placeholder() {
        // `my_key` starts with a valid letter but the first segment contains `_`.
        let tokens = token_map(&[("my_key", "SHOULD_NOT_APPEAR")]);
        let input = "{{token.my_key}} stays put";
        // Should pass through unchanged — `my_key` fails the first-segment rule.
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, "{{token.my_key}} stays put");
    }

    /// Underscores in a subsequent (non-first) segment are valid.
    #[test]
    fn underscore_in_subsequent_segment_is_valid() {
        let tokens = token_map(&[("color.text_primary", "#00ff00")]);
        let input = r##"<rect fill="{{token.color.text_primary}}"/>"##;
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, r##"<rect fill="#00ff00"/>"##);
    }

    /// Non-ASCII (multi-byte UTF-8) content outside placeholders is preserved intact.
    #[test]
    fn non_ascii_content_preserved() {
        let tokens = token_map(&[("color.primary", "red")]);
        // U+00E9 (é) is a 2-byte UTF-8 sequence.
        let input = "<!-- caf\u{00e9} -->{{token.color.primary}}";
        let result = resolve_token_placeholders(input, &tokens).unwrap();
        assert_eq!(result, "<!-- caf\u{00e9} -->red");
    }
}
