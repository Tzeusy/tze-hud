//! Component profile loader and zone rendering override parser — hud-sc0a.5.
//!
//! Implements spec sections:
//! - `component-shape-language/spec.md §Requirement: Component Profile Format`
//! - `component-shape-language/spec.md §Requirement: Zone Rendering Override Schema`
//! - `component-shape-language/spec.md §Requirement: Profile-Scoped Token Resolution`
//! - `component-shape-language/spec.md §Requirement: Profile Widget Scope`
//! - `component-shape-language/spec.md §Requirement: Zone Name Reconciliation`
//! - `component-shape-language/spec.md §Requirement: Profile Validation at Startup`
//!
//! ## Overview
//!
//! A component profile is a directory with:
//! - `profile.toml` — required manifest (name, version, component_type, optional token_overrides)
//! - `widgets/` — optional subdirectory of widget bundle directories
//! - `zones/` — optional subdirectory of `{zone_type_name}.toml` override files
//!
//! Zone override files use the zone registry name (e.g., `notification-area.toml`),
//! not the config constant form (`notification.toml`).
//!
//! ## Profile validation sequence
//!
//! Per `§Requirement: Profile Validation at Startup`, each profile is validated
//! in five ordered phases. A failure in an earlier phase halts that profile
//! before later phases run:
//!
//! 1. **Manifest validation** — `profile.toml` parsed, required fields present,
//!    `component_type` resolved to a known v1 type, name unique across all roots.
//! 2. **Token resolution** — profile-scoped token map built from profile overrides
//!    merged over global config tokens and canonical fallbacks.
//! 3. **Zone override validation** — `zones/` files validated: governed-zone check,
//!    field type/range checks, token reference resolution.
//! 4. **Widget bundle validation** — `widgets/` bundles loaded with scoped tokens;
//!    SVG placeholder resolution and structural validation.
//! 5. **Readability validation** — RenderingPolicy field checks per component type
//!    readability technique. Executed at a higher level after effective zone
//!    policies are assembled; not performed in this module.
//!
//! ## Error codes produced
//!
//! | Error code | Phase | Condition |
//! |---|---|---|
//! | `CONFIG_PROFILE_PATH_NOT_FOUND` | pre-1 | Configured bundle root does not exist |
//! | `PROFILE_UNKNOWN_COMPONENT_TYPE` | 1 | `component_type` field does not match a v1 type |
//! | `CONFIG_PROFILE_DUPLICATE_NAME` | 1 | Two profile directories declare the same name |
//! | `PROFILE_ZONE_OVERRIDE_MISMATCH` | 3 | Zone override file governs a zone not owned by this profile's type |
//! | `PROFILE_INVALID_ZONE_OVERRIDE` | 3 | A zone override field has an invalid value or type |
//! | `PROFILE_UNRESOLVED_TOKEN` | 2–4 | A `{{token.key}}` reference could not be resolved |

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tze_hud_scene::config::{ConfigError, ConfigErrorCode};
use tze_hud_widget::loader::{BundleScanResult, LoadedBundle, scan_bundle_dirs};

use crate::component_types::ComponentType;
use crate::tokens::{
    DesignTokenMap, font_family_from_keyword, parse_color_hex, parse_numeric, resolve_tokens,
};

// ─── ZoneRenderingOverride ────────────────────────────────────────────────────

/// Rendering policy overrides declared in a profile's `zones/{zone_type}.toml`.
///
/// All fields are optional. Omitted fields retain the zone type's token-derived
/// default rendering policy. Fields may contain literal values or `{{token.key}}`
/// references (resolved against the profile's scoped token map at load time).
///
/// Source: `component-shape-language/spec.md §Requirement: Zone Rendering Override Schema`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneRenderingOverride {
    /// Font family: `"system-ui"`, `"sans-serif"`, `"monospace"`, or `"serif"`.
    pub font_family: Option<String>,

    /// Font size in pixels (positive).
    pub font_size_px: Option<f32>,

    /// CSS numeric font weight (100–900).
    pub font_weight: Option<f32>,

    /// Text color as `#RRGGBB` or `#RRGGBBAA`.
    pub text_color: Option<String>,

    /// Text alignment: `"start"`, `"center"`, or `"end"`.
    pub text_align: Option<String>,

    /// Backdrop fill color as `#RRGGBB` or `#RRGGBBAA`.
    pub backdrop_color: Option<String>,

    /// Backdrop opacity in `[0.0, 1.0]`.
    pub backdrop_opacity: Option<f32>,

    /// Outline stroke color as `#RRGGBB` or `#RRGGBBAA`.
    pub outline_color: Option<String>,

    /// Outline stroke width in pixels.
    pub outline_width: Option<f32>,

    /// Horizontal margin in pixels.
    pub margin_horizontal: Option<f32>,

    /// Vertical margin in pixels.
    pub margin_vertical: Option<f32>,

    /// Entry transition duration in milliseconds.
    pub transition_in_ms: Option<u32>,

    /// Exit transition duration in milliseconds.
    pub transition_out_ms: Option<u32>,
}

// ─── ComponentProfile ─────────────────────────────────────────────────────────

/// A fully-loaded component profile.
///
/// Created by scanning a profile directory (containing `profile.toml`),
/// loading its widget bundles from `widgets/`, and parsing zone rendering
/// overrides from `zones/`.
///
/// Note: `Clone` is derived here for use in profile selection — profiles
/// are cloned into the `ProfileSelection` map during startup.
///
/// Source: `component-shape-language/spec.md §Requirement: Component Profile Format`.
#[derive(Clone, Debug)]
pub struct ComponentProfile {
    /// Profile name (kebab-case, unique across all loaded profiles).
    pub name: String,

    /// SemVer version string.
    pub version: String,

    /// Human-readable description (defaults to `""`).
    pub description: String,

    /// The v1 component type this profile implements.
    pub component_type: ComponentType,

    /// Profile-scoped design token overrides.
    ///
    /// Applied as the top layer in three-layer resolution:
    /// profile overrides → global config → canonical fallbacks.
    pub token_overrides: DesignTokenMap,

    /// Widget bundles loaded from the `widgets/` subdirectory.
    ///
    /// Widget names are registered as `"{profile_name}/{widget_name}"` in the
    /// `WidgetRegistry` (namespaced to prevent collision with global bundles).
    pub widget_bundles: Vec<LoadedBundle>,

    /// Zone rendering overrides parsed from `zones/` TOML files.
    ///
    /// Key: zone type registry name (e.g., `"subtitle"`, `"notification-area"`).
    /// Value: the parsed override, with `{{token.key}}` references already resolved.
    pub zone_overrides: HashMap<String, ZoneRenderingOverride>,
}

// ─── Raw deserialization types ────────────────────────────────────────────────

/// Raw `profile.toml` structure.
#[derive(Debug, Deserialize)]
struct RawProfileManifest {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    component_type: Option<String>,
    #[serde(default)]
    token_overrides: HashMap<String, String>,
}

/// Raw `zones/{zone_type}.toml` structure.
///
/// All fields are optional. String fields may hold literal values or
/// `{{token.key}}` references. Numeric fields may be TOML floats, integers,
/// or strings with `{{token.key}}` references.
///
/// This raw form is post-processed by `parse_zone_override` into
/// `ZoneRenderingOverride`.
#[derive(Debug, Deserialize)]
struct RawZoneOverride {
    font_family: Option<toml::Value>,
    font_size_px: Option<toml::Value>,
    font_weight: Option<toml::Value>,
    text_color: Option<toml::Value>,
    text_align: Option<toml::Value>,
    backdrop_color: Option<toml::Value>,
    backdrop_opacity: Option<toml::Value>,
    outline_color: Option<toml::Value>,
    outline_width: Option<toml::Value>,
    margin_horizontal: Option<toml::Value>,
    margin_vertical: Option<toml::Value>,
    transition_in_ms: Option<toml::Value>,
    transition_out_ms: Option<toml::Value>,
}

// ─── Profile directory scanner ────────────────────────────────────────────────

/// Scan one or more profile root directories and load all valid component profiles.
///
/// Each candidate subdirectory is validated by [`load_profile_dir`], which
/// executes the spec's 5-phase profile validation sequence
/// (`§Requirement: Profile Validation at Startup`). This function additionally
/// enforces the **name-uniqueness** sub-check of Phase 1: duplicate profile
/// names across roots produce `CONFIG_PROFILE_DUPLICATE_NAME` and the second
/// occurrence is skipped.
///
/// Failed profiles are logged and skipped; they do not prevent other profiles
/// from loading (per spec: "Invalid profiles MUST be logged and skipped").
///
/// # Arguments
///
/// - `profile_roots`: directories to scan; each immediate subdirectory is a
///   potential profile directory (must contain `profile.toml`).
/// - `config_tokens`: global design token map (from `[design_tokens]`), used
///   as the base layer for per-profile scoped token maps in Phase 2.
/// - `errors`: mutable error accumulator. Root path-not-found errors,
///   duplicate-name errors, and per-profile validation errors are all appended
///   here; callers decide whether to treat accumulated errors as fatal.
///
/// Returns the list of successfully loaded profiles.
pub fn scan_profile_dirs(
    profile_roots: &[PathBuf],
    config_tokens: &DesignTokenMap,
    errors: &mut Vec<ConfigError>,
) -> Vec<ComponentProfile> {
    let mut profiles: Vec<ComponentProfile> = Vec::new();
    let mut registered: HashMap<String, PathBuf> = HashMap::new();

    for root in profile_roots {
        if !root.exists() {
            errors.push(ConfigError {
                code: ConfigErrorCode::ConfigProfilePathNotFound,
                field_path: "component_profile_bundles.paths".into(),
                expected: format!("an existing directory at {:?}", root.display()),
                got: format!("{:?}", root.display()),
                hint: format!(
                    "component profile bundle path {:?} does not exist; \
                     create the directory or remove the path from \
                     [component_profile_bundles].paths",
                    root.display()
                ),
            });
            continue;
        }

        let read_dir = match std::fs::read_dir(root) {
            Ok(rd) => rd,
            Err(e) => {
                tracing::warn!(
                    path = %root.display(),
                    error = %e,
                    "component profile root not readable, skipping"
                );
                continue;
            }
        };

        for entry_result in read_dir {
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        root = %root.display(),
                        error = %e,
                        "error reading profile root directory entry, skipping"
                    );
                    continue;
                }
            };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Try to load the profile from this directory.
            match load_profile_dir(&path, config_tokens) {
                Ok(profile) => {
                    // Check for duplicate name.
                    if let Some(existing_path) = registered.get(&profile.name) {
                        let err = ConfigError {
                            code: ConfigErrorCode::ConfigProfileDuplicateName,
                            field_path: "component_profile_bundles.paths".into(),
                            expected: format!("unique profile name {:?}", profile.name),
                            got: format!(
                                "profile {:?} already loaded from {:?}",
                                profile.name,
                                existing_path.display()
                            ),
                            hint: format!(
                                "profile name {:?} appears in both {:?} and {:?}; \
                                 rename one of the profiles",
                                profile.name,
                                existing_path.display(),
                                path.display()
                            ),
                        };
                        tracing::warn!(
                            profile_name = %profile.name,
                            new_path = %path.display(),
                            existing_path = %existing_path.display(),
                            "CONFIG_PROFILE_DUPLICATE_NAME: duplicate profile name"
                        );
                        errors.push(err);
                        continue;
                    }
                    registered.insert(profile.name.clone(), path.clone());
                    tracing::info!(
                        profile_name = %profile.name,
                        component_type = ?profile.component_type,
                        path = %path.display(),
                        "loaded component profile"
                    );
                    profiles.push(profile);
                }
                Err(profile_errors) => {
                    // Log and skip invalid profile; do not halt startup.
                    for e in &profile_errors {
                        tracing::warn!(
                            path = %path.display(),
                            error_code = ?e.code,
                            "{}: {}",
                            e.field_path,
                            e.got
                        );
                        errors.push(e.clone());
                    }
                }
            }
        }
    }

    profiles
}

// ─── Profile directory loader ─────────────────────────────────────────────────

/// Load a single profile from `dir/profile.toml`.
///
/// Executes the 5-phase profile validation sequence defined by the spec
/// (`§Requirement: Profile Validation at Startup`):
///
/// 1. **Manifest validation** — parse `profile.toml`, check required fields,
///    resolve `component_type` to a known v1 type.
/// 2. **Token resolution** — build the profile-scoped token map by layering
///    profile overrides on top of global config tokens and canonical fallbacks.
/// 3. **Zone override validation** — parse `zones/` TOML files, verify each
///    file governs a zone owned by this profile's component type, and resolve
///    all token references in override field values.
/// 4. **Widget bundle validation** — scan `widgets/` with the scoped token
///    map; SVG placeholders are resolved and SVGs are structurally validated.
/// 5. **Readability validation** — RenderingPolicy field checks per the
///    component type's readability technique. This phase runs at a higher level
///    after effective zone policies are fully assembled; it is NOT performed
///    here. This function completes after phase 4.
///
/// Returns `Ok(ComponentProfile)` when all four in-scope phases pass.
/// Returns `Err(Vec<ConfigError>)` on any validation failure; each error
/// carries a specific error code matching the failing phase.
///
/// The spec guarantees that phases execute in order: a manifest failure
/// (phase 1) stops evaluation before token resolution (phase 2), a bad
/// component type stops evaluation before token/zone/widget work, etc.
fn load_profile_dir(
    dir: &Path,
    config_tokens: &DesignTokenMap,
) -> Result<ComponentProfile, Vec<ConfigError>> {
    let manifest_path = dir.join("profile.toml");
    let path_str = dir.display().to_string();

    // ── Validation Phase 1: Manifest validation ───────────────────────────────
    // Spec: "profile.toml has all required fields, component_type references a
    // known v1 type, name is kebab-case and unique."
    //
    // Sub-step 1a: Read and parse profile.toml from disk.
    let toml_str = std::fs::read_to_string(&manifest_path).map_err(|e| {
        vec![ConfigError {
            code: ConfigErrorCode::ConfigProfilePathNotFound,
            field_path: format!("{path_str}/profile.toml"),
            expected: "a readable profile.toml file".into(),
            got: format!("I/O error: {e}"),
            hint: format!(
                "profile directory {path_str:?} must contain a readable profile.toml manifest"
            ),
        }]
    })?;

    let raw: RawProfileManifest = toml::from_str(&toml_str).map_err(|e| {
        vec![ConfigError {
            code: ConfigErrorCode::ParseError,
            field_path: format!("{path_str}/profile.toml"),
            expected: "valid TOML matching the profile.toml schema".into(),
            got: format!("TOML parse error: {e}"),
            hint: format!(
                "profile.toml at {path_str:?} failed to parse; \
                 check for syntax errors"
            ),
        }]
    })?;

    // Sub-step 1b: Validate that all required fields are present and non-empty.
    let mut errors: Vec<ConfigError> = Vec::new();

    let name = raw
        .name
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let version = raw
        .version
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let component_type_str = raw
        .component_type
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    if name.is_none() {
        errors.push(ConfigError {
            code: ConfigErrorCode::ParseError,
            field_path: format!("{path_str}/profile.toml:name"),
            expected: "non-empty kebab-case profile name".into(),
            got: "missing or empty".into(),
            hint: "add `name = \"my-profile\"` to profile.toml".into(),
        });
    }
    if version.is_none() {
        errors.push(ConfigError {
            code: ConfigErrorCode::ParseError,
            field_path: format!("{path_str}/profile.toml:version"),
            expected: "semver version string".into(),
            got: "missing or empty".into(),
            hint: "add `version = \"1.0.0\"` to profile.toml".into(),
        });
    }
    if component_type_str.is_none() {
        errors.push(ConfigError {
            code: ConfigErrorCode::ParseError,
            field_path: format!("{path_str}/profile.toml:component_type"),
            expected: "a v1 component type name (e.g., \"subtitle\")".into(),
            got: "missing or empty".into(),
            hint: "add `component_type = \"subtitle\"` to profile.toml".into(),
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let name = name.unwrap();
    let version = version.unwrap();
    let component_type_str = component_type_str.unwrap();

    // Sub-step 1c: Resolve component_type string to a known v1 ComponentType.
    // Failure here produces PROFILE_UNKNOWN_COMPONENT_TYPE and halts this profile
    // before any token, zone, or widget work begins (spec: "step 1 MUST NOT
    // proceed to token resolution (step 2)").
    let component_type = match ComponentType::from_name(&component_type_str) {
        Some(ct) => ct,
        None => {
            return Err(vec![ConfigError {
                code: ConfigErrorCode::ProfileUnknownComponentType,
                field_path: format!("{path_str}/profile.toml:component_type"),
                expected: format!(
                    "a v1 component type name (one of: {})",
                    ComponentType::ALL
                        .iter()
                        .map(|ct| ct.contract().name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                got: format!("{component_type_str:?}"),
                hint: format!(
                    "profile {:?}: component_type {:?} is not a recognized v1 component type; \
                     use one of: {}",
                    name,
                    component_type_str,
                    ComponentType::ALL
                        .iter()
                        .map(|ct| ct.contract().name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }]);
        }
    };
    // Uniqueness check ("name is kebab-case and unique") is enforced by
    // `scan_profile_dirs`, which accumulates successfully-loaded profiles and
    // rejects duplicates with CONFIG_PROFILE_DUPLICATE_NAME — still part of
    // phase 1 semantics, but structurally outside this function.

    // ── Validation Phase 2: Token resolution ──────────────────────────────────
    // Spec: "Profile-scoped token map is constructed (profile overrides merged
    // with global tokens and canonical fallbacks). All required tokens for the
    // component type are resolvable."
    //
    // Three-layer resolution: profile overrides → global config → canonical fallbacks.
    // Individual token references are resolved lazily in phases 3 and 4 as each
    // field value is parsed; this call builds the merged map used in those phases.
    let scoped_tokens = resolve_tokens(config_tokens, &raw.token_overrides);

    // ── Validation Phase 3: Zone override validation ──────────────────────────
    // Spec: "Zone override files reference only zone types governed by the
    // profile's component type. Override field values are valid types and
    // ranges. Token references in override fields are resolvable."
    //
    // `load_zone_overrides` reads each `zones/{zone_type}.toml`, validates that
    // the zone name is governed by `component_type` (PROFILE_ZONE_OVERRIDE_MISMATCH
    // if not), and resolves {{token.key}} references in field values against
    // `scoped_tokens` (PROFILE_UNRESOLVED_TOKEN / PROFILE_INVALID_ZONE_OVERRIDE
    // on failure).
    let zones_dir = dir.join("zones");
    let zone_overrides = if zones_dir.is_dir() {
        load_zone_overrides(&zones_dir, &name, component_type, &scoped_tokens)?
    } else {
        HashMap::new()
    };

    // ── Validation Phase 4: Widget bundle validation ──────────────────────────
    // Spec: "Profile-scoped widget bundles are loaded with the profile's scoped
    // token map. SVG placeholders are resolved. SVGs parse after resolution."
    //
    // Widget names are namespaced as "{profile_name}/{widget_name}" to prevent
    // collision with global bundles in the WidgetRegistry. Bundle errors are
    // logged and skipped; they do not fail the overall profile load (per spec:
    // invalid bundles are logged, the profile may still register successfully).
    let widgets_dir = dir.join("widgets");
    let widget_bundles = if widgets_dir.is_dir() {
        let results = scan_bundle_dirs(&[widgets_dir.clone()], &scoped_tokens);
        let mut bundles: Vec<LoadedBundle> = Vec::new();
        for result in results {
            match result {
                BundleScanResult::Ok(mut bundle) => {
                    // Namespace widget name as "{profile_name}/{widget_name}".
                    let original_name = bundle.definition.id.clone();
                    bundle.definition.id = format!("{name}/{original_name}");
                    bundles.push(bundle);
                }
                BundleScanResult::Err(e) => {
                    // Bundle errors are logged by scan_bundle_dirs; we skip the bundle.
                    tracing::warn!(
                        profile = %name,
                        error = %e,
                        "skipping invalid widget bundle in profile"
                    );
                }
            }
        }
        bundles
    } else {
        Vec::new()
    };

    // Phases 1–4 complete. Phase 5 (readability validation) is deferred to the
    // caller after effective zone RenderingPolicy is fully assembled.
    Ok(ComponentProfile {
        name,
        version,
        description: raw.description.unwrap_or_default(),
        component_type,
        token_overrides: raw.token_overrides,
        widget_bundles,
        zone_overrides,
    })
}

// ─── Zone override loader ─────────────────────────────────────────────────────

/// Load all zone override files from a profile's `zones/` subdirectory.
///
/// Each `.toml` file must be named `{zone_type_name}.toml` where the zone type
/// is governed by `component_type`. Mismatched zone names produce
/// `PROFILE_ZONE_OVERRIDE_MISMATCH`.
fn load_zone_overrides(
    zones_dir: &Path,
    profile_name: &str,
    component_type: ComponentType,
    scoped_tokens: &DesignTokenMap,
) -> Result<HashMap<String, ZoneRenderingOverride>, Vec<ConfigError>> {
    let governed_zone = component_type.contract().zone_type_name;
    let mut overrides: HashMap<String, ZoneRenderingOverride> = HashMap::new();
    let mut errors: Vec<ConfigError> = Vec::new();

    let read_dir = match std::fs::read_dir(zones_dir) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!(
                path = %zones_dir.display(),
                error = %e,
                "zone override directory not readable, skipping"
            );
            return Ok(HashMap::new());
        }
    };

    for entry_result in read_dir {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    zones_dir = %zones_dir.display(),
                    error = %e,
                    "error reading zone override directory entry, skipping"
                );
                continue;
            }
        };
        let path = entry.path();

        // Only process .toml files.
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !file_name.ends_with(".toml") {
            continue;
        }

        // Extract zone type name from filename (strip .toml extension).
        let zone_type_name = &file_name[..file_name.len() - 5];

        // Validate zone name reconciliation: file must use registry name.
        if zone_type_name != governed_zone {
            errors.push(ConfigError {
                code: ConfigErrorCode::ProfileZoneOverrideMismatch,
                field_path: format!("{}/zones/{}", zones_dir.display(), file_name),
                expected: format!(
                    "zone override file for the governed zone \"{governed_zone}\" \
                     (profile {:?} with component_type {:?} only governs \"{governed_zone}\")",
                    profile_name,
                    component_type.contract().name
                ),
                got: format!(
                    "zone override file for zone \"{zone_type_name}\" \
                     which is not governed by component_type {:?}",
                    component_type.contract().name
                ),
                hint: format!(
                    "profile {:?}: rename \"zones/{}.toml\" to \"zones/{governed_zone}.toml\" \
                     or remove the file; the {} component type only governs the \
                     \"{governed_zone}\" zone",
                    profile_name,
                    zone_type_name,
                    component_type.contract().name
                ),
            });
            continue;
        }

        // Parse the zone override file.
        let toml_str = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                errors.push(ConfigError {
                    code: ConfigErrorCode::ProfileInvalidZoneOverride,
                    field_path: format!("{}", path.display()),
                    expected: "a readable zone override TOML file".into(),
                    got: format!("I/O error: {e}"),
                    hint: format!(
                        "profile {:?}: could not read zone override file {:?}",
                        profile_name,
                        path.display()
                    ),
                });
                continue;
            }
        };

        let raw: RawZoneOverride = match toml::from_str(&toml_str) {
            Ok(r) => r,
            Err(e) => {
                errors.push(ConfigError {
                    code: ConfigErrorCode::ProfileInvalidZoneOverride,
                    field_path: format!("{}", path.display()),
                    expected: "valid TOML matching the zone override schema".into(),
                    got: format!("TOML parse error: {e}"),
                    hint: format!(
                        "profile {:?}: zone override file {:?} failed to parse",
                        profile_name,
                        path.display()
                    ),
                });
                continue;
            }
        };

        // Validate and resolve the raw override.
        match validate_zone_override(&raw, profile_name, zone_type_name, scoped_tokens) {
            Ok(override_val) => {
                overrides.insert(zone_type_name.to_string(), override_val);
            }
            Err(e) => {
                errors.push(e);
            }
        }
    }

    if errors.is_empty() {
        Ok(overrides)
    } else {
        Err(errors)
    }
}

// ─── Zone override validator ──────────────────────────────────────────────────

/// Validate and resolve a raw zone override into a `ZoneRenderingOverride`.
///
/// Resolves `{{token.key}}` references against `scoped_tokens`. Returns
/// `PROFILE_UNRESOLVED_TOKEN` for unknown token keys, and
/// `PROFILE_INVALID_ZONE_OVERRIDE` for bad field values.
fn validate_zone_override(
    raw: &RawZoneOverride,
    profile_name: &str,
    zone_type_name: &str,
    scoped_tokens: &DesignTokenMap,
) -> Result<ZoneRenderingOverride, ConfigError> {
    let mut out = ZoneRenderingOverride::default();

    // Helper: resolve a numeric field (TOML float/integer or {{token.key}} string).
    macro_rules! resolve_numeric_field {
        ($raw_field:expr, $field_name:expr) => {
            if let Some(val) = &$raw_field {
                Some(resolve_numeric_value(
                    val,
                    scoped_tokens,
                    profile_name,
                    zone_type_name,
                    $field_name,
                )?)
            } else {
                None
            }
        };
    }

    // ── font_family ──────────────────────────────────────────────────────────
    // Spec (§Zone Rendering Override Schema): font_family is "parsed per font family format
    // in Token Value Formats" — only the three system keywords are valid in v1.
    if let Some(val) = &raw.font_family {
        let s = extract_string_value(val, "font_family", profile_name, zone_type_name)?;
        let resolved = resolve_token_ref(
            &s,
            scoped_tokens,
            profile_name,
            zone_type_name,
            "font_family",
        )?;
        if font_family_from_keyword(&resolved).is_none() {
            return Err(ConfigError {
                code: ConfigErrorCode::ProfileInvalidZoneOverride,
                field_path: format!("profile:{profile_name}/zones/{zone_type_name}.toml:font_family"),
                expected: "a v1 font family keyword (\"system-ui\", \"sans-serif\", \"monospace\", or \"serif\")".into(),
                got: format!("{resolved:?}"),
                hint: format!(
                    "profile {profile_name:?}: font_family value {resolved:?} is not a recognized font family keyword; \
                     v1 supports only \"system-ui\", \"sans-serif\", \"monospace\", and \"serif\""
                ),
            });
        }
        out.font_family = Some(resolved);
    }

    // ── font_size_px ─────────────────────────────────────────────────────────
    out.font_size_px = resolve_numeric_field!(raw.font_size_px, "font_size_px");

    // ── font_weight ──────────────────────────────────────────────────────────
    out.font_weight = resolve_numeric_field!(raw.font_weight, "font_weight");

    // ── text_color (color hex string) ────────────────────────────────────────
    if let Some(val) = &raw.text_color {
        let s = extract_string_value(val, "text_color", profile_name, zone_type_name)?;
        let resolved = resolve_token_ref(
            &s,
            scoped_tokens,
            profile_name,
            zone_type_name,
            "text_color",
        )?;
        // Validate color format.
        if parse_color_hex(&resolved).is_none() {
            return Err(ConfigError {
                code: ConfigErrorCode::ProfileInvalidZoneOverride,
                field_path: format!(
                    "profile:{profile_name}/zones/{zone_type_name}.toml:text_color"
                ),
                expected: "color hex string (#RRGGBB or #RRGGBBAA)".into(),
                got: format!("{resolved:?}"),
                hint: format!(
                    "profile {profile_name:?}: text_color value {resolved:?} is not a valid color hex; \
                     use a format like #FF0000 or #FF0000FF"
                ),
            });
        }
        out.text_color = Some(resolved);
    }

    // ── text_align (enum) ────────────────────────────────────────────────────
    if let Some(val) = &raw.text_align {
        let s = extract_string_value(val, "text_align", profile_name, zone_type_name)?;
        let resolved = resolve_token_ref(
            &s,
            scoped_tokens,
            profile_name,
            zone_type_name,
            "text_align",
        )?;
        if !matches!(resolved.as_str(), "start" | "center" | "end") {
            return Err(ConfigError {
                code: ConfigErrorCode::ProfileInvalidZoneOverride,
                field_path: format!(
                    "profile:{profile_name}/zones/{zone_type_name}.toml:text_align"
                ),
                expected: "one of \"start\", \"center\", \"end\"".into(),
                got: format!("{resolved:?}"),
                hint: format!(
                    "profile {profile_name:?}: text_align value {resolved:?} is invalid; \
                     use \"start\", \"center\", or \"end\""
                ),
            });
        }
        out.text_align = Some(resolved);
    }

    // ── backdrop_color (color hex string) ────────────────────────────────────
    if let Some(val) = &raw.backdrop_color {
        let s = extract_string_value(val, "backdrop_color", profile_name, zone_type_name)?;
        let resolved = resolve_token_ref(
            &s,
            scoped_tokens,
            profile_name,
            zone_type_name,
            "backdrop_color",
        )?;
        if parse_color_hex(&resolved).is_none() {
            return Err(ConfigError {
                code: ConfigErrorCode::ProfileInvalidZoneOverride,
                field_path: format!(
                    "profile:{profile_name}/zones/{zone_type_name}.toml:backdrop_color"
                ),
                expected: "color hex string (#RRGGBB or #RRGGBBAA)".into(),
                got: format!("{resolved:?}"),
                hint: format!(
                    "profile {profile_name:?}: backdrop_color value {resolved:?} is not a valid color hex"
                ),
            });
        }
        out.backdrop_color = Some(resolved);
    }

    // ── backdrop_opacity ─────────────────────────────────────────────────────
    if let Some(val) = &raw.backdrop_opacity {
        let n = resolve_numeric_value(
            val,
            scoped_tokens,
            profile_name,
            zone_type_name,
            "backdrop_opacity",
        )?;
        if !(0.0..=1.0).contains(&n) {
            return Err(ConfigError {
                code: ConfigErrorCode::ProfileInvalidZoneOverride,
                field_path: format!(
                    "profile:{profile_name}/zones/{zone_type_name}.toml:backdrop_opacity"
                ),
                expected: "a float in [0.0, 1.0]".into(),
                got: format!("{n}"),
                hint: format!(
                    "profile {profile_name:?}: backdrop_opacity {n} is out of range; \
                     use a value between 0.0 and 1.0"
                ),
            });
        }
        out.backdrop_opacity = Some(n);
    }

    // ── outline_color (color hex string) ─────────────────────────────────────
    if let Some(val) = &raw.outline_color {
        let s = extract_string_value(val, "outline_color", profile_name, zone_type_name)?;
        let resolved = resolve_token_ref(
            &s,
            scoped_tokens,
            profile_name,
            zone_type_name,
            "outline_color",
        )?;
        if parse_color_hex(&resolved).is_none() {
            return Err(ConfigError {
                code: ConfigErrorCode::ProfileInvalidZoneOverride,
                field_path: format!(
                    "profile:{profile_name}/zones/{zone_type_name}.toml:outline_color"
                ),
                expected: "color hex string (#RRGGBB or #RRGGBBAA)".into(),
                got: format!("{resolved:?}"),
                hint: format!(
                    "profile {profile_name:?}: outline_color value {resolved:?} is not a valid color hex"
                ),
            });
        }
        out.outline_color = Some(resolved);
    }

    // ── outline_width ────────────────────────────────────────────────────────
    out.outline_width = resolve_numeric_field!(raw.outline_width, "outline_width");

    // ── margin_horizontal ────────────────────────────────────────────────────
    out.margin_horizontal = resolve_numeric_field!(raw.margin_horizontal, "margin_horizontal");

    // ── margin_vertical ──────────────────────────────────────────────────────
    out.margin_vertical = resolve_numeric_field!(raw.margin_vertical, "margin_vertical");

    // ── transition_in_ms ─────────────────────────────────────────────────────
    if let Some(val) = &raw.transition_in_ms {
        out.transition_in_ms = Some(resolve_u32_value(
            val,
            profile_name,
            zone_type_name,
            "transition_in_ms",
        )?);
    }

    // ── transition_out_ms ────────────────────────────────────────────────────
    if let Some(val) = &raw.transition_out_ms {
        out.transition_out_ms = Some(resolve_u32_value(
            val,
            profile_name,
            zone_type_name,
            "transition_out_ms",
        )?);
    }

    Ok(out)
}

// ─── Field resolution helpers ─────────────────────────────────────────────────

/// Extract a string value from a `toml::Value` (must be a TOML string).
fn extract_string_value(
    val: &toml::Value,
    field_name: &str,
    profile_name: &str,
    zone_type_name: &str,
) -> Result<String, ConfigError> {
    match val {
        toml::Value::String(s) => Ok(s.clone()),
        _ => Err(ConfigError {
            code: ConfigErrorCode::ProfileInvalidZoneOverride,
            field_path: format!("profile:{profile_name}/zones/{zone_type_name}.toml:{field_name}"),
            expected: "a TOML string (possibly with {{token.key}} reference)".into(),
            got: format!("{val:?}"),
            hint: format!(
                "profile {profile_name:?}: zone override field \"{field_name}\" must be a string, \
                 e.g., \"{field_name} = \\\"value\\\"\" or \"{field_name} = \\\"{{{{token.key}}}}\\\"\""
            ),
        }),
    }
}

/// Resolve a `{{token.key}}` reference in a string value.
///
/// If the string matches the pattern `{{key}}`, look up `key` in `scoped_tokens`.
/// Otherwise return the string unchanged. Returns `PROFILE_UNRESOLVED_TOKEN` if
/// the token key is not in the map.
fn resolve_token_ref(
    s: &str,
    scoped_tokens: &DesignTokenMap,
    profile_name: &str,
    zone_type_name: &str,
    field_name: &str,
) -> Result<String, ConfigError> {
    // Check if this is a token reference: starts with "{{" and ends with "}}"
    // with no whitespace inside, matching the pattern {{[a-z...][a-z0-9.]*}}.
    if let Some(inner) = extract_token_key(s) {
        match scoped_tokens.get(inner) {
            Some(resolved) => Ok(resolved.clone()),
            None => Err(ConfigError {
                code: ConfigErrorCode::ProfileUnresolvedToken,
                field_path: format!(
                    "profile:{profile_name}/zones/{zone_type_name}.toml:{field_name}"
                ),
                expected: format!("token key {inner:?} to be present in the resolved token map"),
                got: format!("{{{{token.{inner}}}}} not found in profile-scoped token map"),
                hint: format!(
                    "profile {profile_name:?}: zone override field \"{field_name}\" references \
                     token {inner:?} which is not defined; add it to [design_tokens] \
                     or [token_overrides] in the profile"
                ),
            }),
        }
    } else {
        Ok(s.to_string())
    }
}

/// Extract the token key from a `{{key}}` reference string.
///
/// Returns `Some(key)` if the string is exactly `{{key}}` with no whitespace
/// inside the braces. Returns `None` otherwise.
fn extract_token_key(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.starts_with("{{") && s.ends_with("}}") && s.len() > 4 {
        let inner = &s[2..s.len() - 2];
        // No whitespace allowed inside braces (per spec).
        if inner.contains(' ') || inner.contains('\t') {
            return None;
        }
        // Inner must be non-empty.
        if inner.is_empty() {
            return None;
        }
        Some(inner)
    } else {
        None
    }
}

/// Resolve a numeric value from a `toml::Value` (float, integer, or `{{token}}` string).
fn resolve_numeric_value(
    val: &toml::Value,
    scoped_tokens: &DesignTokenMap,
    profile_name: &str,
    zone_type_name: &str,
    field_name: &str,
) -> Result<f32, ConfigError> {
    match val {
        toml::Value::Float(f) => {
            let n = *f as f32;
            if !n.is_finite() {
                return Err(ConfigError {
                    code: ConfigErrorCode::ProfileInvalidZoneOverride,
                    field_path: format!(
                        "profile:{profile_name}/zones/{zone_type_name}.toml:{field_name}"
                    ),
                    expected: "a finite numeric value".into(),
                    got: format!("{n}"),
                    hint: format!(
                        "profile {profile_name:?}: field \"{field_name}\" must be a finite number"
                    ),
                });
            }
            Ok(n)
        }
        toml::Value::Integer(i) => Ok(*i as f32),
        toml::Value::String(s) => {
            // May be a {{token.key}} reference.
            let resolved =
                resolve_token_ref(s, scoped_tokens, profile_name, zone_type_name, field_name)?;
            parse_numeric(&resolved).ok_or_else(|| ConfigError {
                code: ConfigErrorCode::ProfileInvalidZoneOverride,
                field_path: format!(
                    "profile:{profile_name}/zones/{zone_type_name}.toml:{field_name}"
                ),
                expected: "a numeric value or {{token.key}} reference resolving to a number".into(),
                got: format!("{resolved:?}"),
                hint: format!(
                    "profile {profile_name:?}: field \"{field_name}\" value {resolved:?} (resolved from token) \
                     is not a valid number"
                ),
            })
        }
        _ => Err(ConfigError {
            code: ConfigErrorCode::ProfileInvalidZoneOverride,
            field_path: format!("profile:{profile_name}/zones/{zone_type_name}.toml:{field_name}"),
            expected: "a TOML float, integer, or string with {{token.key}} reference".into(),
            got: format!("{val:?}"),
            hint: format!(
                "profile {profile_name:?}: field \"{field_name}\" must be numeric or a token reference string"
            ),
        }),
    }
}

/// Resolve a `u32` value from a `toml::Value` (integer only; spec mandates TOML integer).
fn resolve_u32_value(
    val: &toml::Value,
    profile_name: &str,
    zone_type_name: &str,
    field_name: &str,
) -> Result<u32, ConfigError> {
    match val {
        toml::Value::Integer(i) => {
            if *i < 0 || *i > u32::MAX as i64 {
                return Err(ConfigError {
                    code: ConfigErrorCode::ProfileInvalidZoneOverride,
                    field_path: format!(
                        "profile:{profile_name}/zones/{zone_type_name}.toml:{field_name}"
                    ),
                    expected: "a non-negative integer (u32)".into(),
                    got: format!("{i}"),
                    hint: format!(
                        "profile {profile_name:?}: field \"{field_name}\" must be a non-negative integer"
                    ),
                });
            }
            Ok(*i as u32)
        }
        _ => Err(ConfigError {
            code: ConfigErrorCode::ProfileInvalidZoneOverride,
            field_path: format!("profile:{profile_name}/zones/{zone_type_name}.toml:{field_name}"),
            expected: "a TOML integer (e.g., transition_in_ms = 200)".into(),
            got: format!("{val:?}"),
            hint: format!(
                "profile {profile_name:?}: field \"{field_name}\" must be a TOML integer, not a float; \
                 write e.g., `{field_name} = 200` (no decimal point)"
            ),
        }),
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn empty_tokens() -> DesignTokenMap {
        DesignTokenMap::new()
    }

    /// Create a temporary profile directory with given profile.toml content.
    fn make_profile_dir(toml_content: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let manifest_path = dir.path().join("profile.toml");
        std::fs::write(&manifest_path, toml_content).unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    /// Create a profile dir with zones/ subdirectory and given zone override content.
    fn make_profile_dir_with_zone(
        profile_toml: &str,
        zone_name: &str,
        zone_toml: &str,
    ) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let manifest_path = dir.path().join("profile.toml");
        std::fs::write(&manifest_path, profile_toml).unwrap();
        let zones_dir = dir.path().join("zones");
        std::fs::create_dir(&zones_dir).unwrap();
        let zone_path = zones_dir.join(format!("{zone_name}.toml"));
        std::fs::write(&zone_path, zone_toml).unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    // ── Valid profile round-trip ───────────────────────────────────────────────

    #[test]
    fn valid_profile_round_trip() {
        let (_dir, path) = make_profile_dir(
            r##"
name = "test-subtitle"
version = "1.0.0"
description = "Test subtitle profile"
component_type = "subtitle"

[token_overrides]
"color.text.primary" = "#FFFF00"
"##,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let profile = result.expect("valid profile should load");

        assert_eq!(profile.name, "test-subtitle");
        assert_eq!(profile.version, "1.0.0");
        assert_eq!(profile.description, "Test subtitle profile");
        assert_eq!(profile.component_type, ComponentType::Subtitle);
        assert_eq!(
            profile.token_overrides.get("color.text.primary"),
            Some(&"#FFFF00".to_string())
        );
        assert!(profile.widget_bundles.is_empty());
        assert!(profile.zone_overrides.is_empty());
    }

    #[test]
    fn valid_profile_minimal_fields() {
        let (_dir, path) = make_profile_dir(
            r#"
name = "minimal"
version = "0.1.0"
component_type = "notification"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let profile = result.expect("minimal profile should load");

        assert_eq!(profile.name, "minimal");
        assert_eq!(profile.version, "0.1.0");
        assert_eq!(profile.description, ""); // defaults to empty
        assert_eq!(profile.component_type, ComponentType::Notification);
        assert!(profile.token_overrides.is_empty());
    }

    // ── Missing required fields ───────────────────────────────────────────────

    #[test]
    fn missing_name_field_produces_error() {
        let (_dir, path) = make_profile_dir(
            r#"
version = "1.0.0"
component_type = "subtitle"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("missing name should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ParseError)),
            "missing name should produce ParseError, got: {errors:?}"
        );
    }

    #[test]
    fn missing_version_field_produces_error() {
        let (_dir, path) = make_profile_dir(
            r#"
name = "test"
component_type = "subtitle"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("missing version should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ParseError)),
            "missing version should produce ParseError"
        );
    }

    #[test]
    fn missing_component_type_field_produces_error() {
        let (_dir, path) = make_profile_dir(
            r#"
name = "test"
version = "1.0.0"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("missing component_type should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ParseError)),
            "missing component_type should produce ParseError"
        );
    }

    // ── Unknown component type ────────────────────────────────────────────────

    #[test]
    fn unknown_component_type_produces_error() {
        let (_dir, path) = make_profile_dir(
            r#"
name = "hologram-profile"
version = "1.0.0"
component_type = "hologram"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("unknown component type should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ProfileUnknownComponentType)),
            "unknown component_type should produce PROFILE_UNKNOWN_COMPONENT_TYPE, got: {errors:?}"
        );
    }

    #[test]
    fn all_six_v1_component_types_accepted() {
        for ct in ComponentType::ALL {
            let type_name = ct.contract().name;
            let (_dir, path) = make_profile_dir(&format!(
                r#"name = "test-{type_name}"
version = "1.0.0"
component_type = "{type_name}"
"#
            ));
            let result = load_profile_dir(&path, &empty_tokens());
            assert!(
                result.is_ok(),
                "component_type {:?} should be accepted, got: {:?}",
                type_name,
                result.err()
            );
        }
    }

    // ── Duplicate profile names ───────────────────────────────────────────────

    #[test]
    fn duplicate_profile_names_produce_error() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();

        // Create two profile root dirs, each containing a profile named "same".
        let root1 = dir1.path().to_path_buf();
        let root2 = dir2.path().to_path_buf();

        let p1 = root1.join("same");
        let p2 = root2.join("same");
        std::fs::create_dir(&p1).unwrap();
        std::fs::create_dir(&p2).unwrap();

        let profile_toml = r#"name = "same"
version = "1.0.0"
component_type = "subtitle"
"#;
        std::fs::write(p1.join("profile.toml"), profile_toml).unwrap();
        std::fs::write(p2.join("profile.toml"), profile_toml).unwrap();

        let mut errors: Vec<ConfigError> = Vec::new();
        let profiles = scan_profile_dirs(&[root1, root2], &empty_tokens(), &mut errors);

        // Exactly one profile loaded (the second is rejected as duplicate).
        assert_eq!(profiles.len(), 1, "only one profile should be loaded");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ConfigProfileDuplicateName)),
            "duplicate names should produce CONFIG_PROFILE_DUPLICATE_NAME, got: {errors:?}"
        );
    }

    // ── Zone override mismatch ────────────────────────────────────────────────

    #[test]
    fn zone_override_mismatch_produces_error() {
        // subtitle profile MUST NOT contain zones/notification-area.toml
        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "test-subtitle"
version = "1.0.0"
component_type = "subtitle"
"#,
            "notification-area", // wrong zone for subtitle
            "backdrop_opacity = 0.8",
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("zone override mismatch should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ProfileZoneOverrideMismatch)),
            "wrong zone name should produce PROFILE_ZONE_OVERRIDE_MISMATCH, got: {errors:?}"
        );
    }

    #[test]
    fn correct_zone_override_accepted() {
        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "cinematic"
version = "1.0.0"
component_type = "subtitle"
"#,
            "subtitle", // correct zone for subtitle
            r#"backdrop_opacity = 0.8
text_align = "center"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let profile = result.expect("correct zone override should load");
        let zone_override = profile
            .zone_overrides
            .get("subtitle")
            .expect("subtitle override should exist");
        assert_eq!(zone_override.backdrop_opacity, Some(0.8));
        assert_eq!(zone_override.text_align, Some("center".to_string()));
    }

    // ── Token references in overrides ─────────────────────────────────────────

    #[test]
    fn token_reference_in_zone_override_resolved() {
        let mut config_tokens = DesignTokenMap::new();
        config_tokens.insert("color.text.accent".to_string(), "#4A9EFF".to_string());

        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "token-test"
version = "1.0.0"
component_type = "subtitle"
"#,
            "subtitle",
            r#"text_color = "{{color.text.accent}}"
"#,
        );

        let result = load_profile_dir(&path, &config_tokens);
        let profile = result.expect("token reference should be resolved");
        let zone_override = profile.zone_overrides.get("subtitle").unwrap();
        assert_eq!(zone_override.text_color, Some("#4A9EFF".to_string()));
    }

    #[test]
    fn unresolved_token_reference_produces_error() {
        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "broken-profile"
version = "1.0.0"
component_type = "subtitle"
"#,
            "subtitle",
            r#"text_color = "{{color.nonexistent.token}}"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("unresolved token should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ProfileUnresolvedToken)),
            "unresolved token should produce PROFILE_UNRESOLVED_TOKEN, got: {errors:?}"
        );
    }

    #[test]
    fn profile_token_overrides_scoped_to_profile() {
        // Profile declares a token override; it MUST be used in zone override resolution.
        let (_dir, path) = make_profile_dir_with_zone(
            r##"name = "scoped"
version = "1.0.0"
component_type = "subtitle"

[token_overrides]
"color.text.primary" = "#00FF00"
"##,
            "subtitle",
            r#"text_color = "{{color.text.primary}}"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let profile = result.expect("profile-scoped token reference should resolve");
        let zone_override = profile.zone_overrides.get("subtitle").unwrap();
        // Must use the profile's override (#00FF00), not the canonical fallback (#FFFFFF).
        assert_eq!(zone_override.text_color, Some("#00FF00".to_string()));
    }

    // ── text_align validation ─────────────────────────────────────────────────

    #[test]
    fn invalid_text_align_produces_error() {
        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "bad-align"
version = "1.0.0"
component_type = "subtitle"
"#,
            "subtitle",
            r#"text_align = "middle"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("invalid text_align should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ProfileInvalidZoneOverride)),
            "invalid text_align should produce PROFILE_INVALID_ZONE_OVERRIDE, got: {errors:?}"
        );
    }

    #[test]
    fn valid_text_align_values_accepted() {
        for align in &["start", "center", "end"] {
            let (_dir, path) = make_profile_dir_with_zone(
                r#"name = "align-test"
version = "1.0.0"
component_type = "subtitle"
"#,
                "subtitle",
                &format!(r#"text_align = "{align}""#),
            );
            let result = load_profile_dir(&path, &empty_tokens());
            assert!(
                result.is_ok(),
                "text_align {:?} should be accepted, got: {:?}",
                align,
                result.err()
            );
        }
    }

    // ── backdrop_opacity range ────────────────────────────────────────────────

    #[test]
    fn backdrop_opacity_out_of_range_produces_error() {
        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "opacity-test"
version = "1.0.0"
component_type = "subtitle"
"#,
            "subtitle",
            "backdrop_opacity = 1.5",
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("out-of-range opacity should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ProfileInvalidZoneOverride)),
            "out-of-range backdrop_opacity should produce PROFILE_INVALID_ZONE_OVERRIDE"
        );
    }

    // ── scan_profile_dirs path not found ─────────────────────────────────────

    #[test]
    fn nonexistent_profile_root_produces_error() {
        let mut errors: Vec<ConfigError> = Vec::new();
        let profiles = scan_profile_dirs(
            &[PathBuf::from("/tmp/tze_hud_nonexistent_profile_dir_99999")],
            &empty_tokens(),
            &mut errors,
        );
        assert!(profiles.is_empty());
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ConfigProfilePathNotFound)),
            "nonexistent path should produce CONFIG_PROFILE_PATH_NOT_FOUND"
        );
    }

    // ── Zone Name Reconciliation ──────────────────────────────────────────────

    /// Per spec §Zone Name Reconciliation: notification-area (registry) vs notification (config).
    #[test]
    fn notification_zone_override_uses_registry_name() {
        // notification profile must use zones/notification-area.toml (registry name).
        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "notif"
version = "1.0.0"
component_type = "notification"
"#,
            "notification-area", // registry name — CORRECT
            "backdrop_opacity = 0.9",
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let profile = result.expect("notification-area override should be accepted");
        assert!(
            profile.zone_overrides.contains_key("notification-area"),
            "zone override should be keyed by registry name 'notification-area'"
        );
    }

    /// Using the config constant name (notification) for notification component type must fail.
    #[test]
    fn notification_config_constant_name_rejected_as_zone_override() {
        // notification profile MUST NOT use zones/notification.toml (config constant).
        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "notif"
version = "1.0.0"
component_type = "notification"
"#,
            "notification", // config constant — WRONG for zone override matching
            "backdrop_opacity = 0.9",
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors =
            result.expect_err("notification.toml for notification type should be rejected");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ProfileZoneOverrideMismatch)),
            "notification.toml should fail with PROFILE_ZONE_OVERRIDE_MISMATCH \
             because the governed zone is 'notification-area', got: {errors:?}"
        );
    }

    // ── extract_token_key ─────────────────────────────────────────────────────

    #[test]
    fn extract_token_key_valid() {
        assert_eq!(
            extract_token_key("{{color.text.primary}}"),
            Some("color.text.primary")
        );
        assert_eq!(extract_token_key("{{spacing.unit}}"), Some("spacing.unit"));
        assert_eq!(extract_token_key("{{a}}"), Some("a"));
    }

    #[test]
    fn extract_token_key_invalid() {
        // Not a token reference.
        assert_eq!(extract_token_key("#FF0000"), None);
        assert_eq!(extract_token_key("center"), None);
        // Whitespace inside braces — per spec not treated as placeholder.
        assert_eq!(extract_token_key("{{ color.text.primary }}"), None);
        // Empty braces.
        assert_eq!(extract_token_key("{{}}"), None);
        // Literal escaped braces (treated as non-token since they are {{{}}}}).
        assert_eq!(extract_token_key("{{}}"), None);
    }

    // ── font_family validation ────────────────────────────────────────────────

    #[test]
    fn invalid_font_family_keyword_produces_error() {
        // v1 only supports system-ui, sans-serif, monospace, serif.
        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "bad-font"
version = "1.0.0"
component_type = "subtitle"
"#,
            "subtitle",
            r#"font_family = "Fira Code"
"#,
        );

        let result = load_profile_dir(&path, &empty_tokens());
        let errors = result.expect_err("invalid font_family keyword should produce errors");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::ProfileInvalidZoneOverride)),
            "invalid font_family should produce PROFILE_INVALID_ZONE_OVERRIDE, got: {errors:?}"
        );
    }

    #[test]
    fn valid_font_family_keywords_accepted() {
        for kw in &["system-ui", "sans-serif", "monospace", "serif"] {
            let (_dir, path) = make_profile_dir_with_zone(
                r#"name = "font-kw-test"
version = "1.0.0"
component_type = "subtitle"
"#,
                "subtitle",
                &format!(r#"font_family = "{kw}""#),
            );
            let result = load_profile_dir(&path, &empty_tokens());
            assert!(
                result.is_ok(),
                "font_family {:?} should be accepted, got: {:?}",
                kw,
                result.err()
            );
        }
    }

    // ── Profile with numeric token reference ──────────────────────────────────

    #[test]
    fn numeric_token_reference_in_zone_override_resolved() {
        let mut config_tokens = DesignTokenMap::new();
        config_tokens.insert("typography.subtitle.size".to_string(), "32".to_string());

        let (_dir, path) = make_profile_dir_with_zone(
            r#"name = "font-test"
version = "1.0.0"
component_type = "subtitle"
"#,
            "subtitle",
            r#"font_size_px = "{{typography.subtitle.size}}"
"#,
        );

        let result = load_profile_dir(&path, &config_tokens);
        let profile = result.expect("numeric token reference should resolve");
        let zone_override = profile.zone_overrides.get("subtitle").unwrap();
        assert_eq!(zone_override.font_size_px, Some(32.0));
    }
}
