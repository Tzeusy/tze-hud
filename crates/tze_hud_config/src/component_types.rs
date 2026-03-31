//! Component type contracts and readability technique enums — hud-sc0a.4.
//!
//! Defines the six v1 component types and their static contracts as specified in:
//! - `component-shape-language/spec.md §Requirement: Component Type Contract`
//! - `component-shape-language/spec.md §Requirement: V1 Component Type Definitions`
//! - `component-shape-language/spec.md §Requirement: Zone Name Reconciliation`
//!
//! ## Zone Name Reconciliation
//!
//! Zone registry names (used here in contracts) differ from config validation
//! constants (`BUILTIN_ZONE_TYPES`). Profile zone override files MUST use
//! the registry names (e.g. `zones/notification-area.toml`, NOT
//! `zones/notification.toml`).
//!
//! | Zone Registry Name    | Config Constant      |
//! |-----------------------|----------------------|
//! | `"status-bar"`        | `"status_bar"`       |
//! | `"notification-area"` | `"notification"`     |
//! | `"subtitle"`          | `"subtitle"`         |
//! | `"pip"`               | `"pip"`              |
//! | `"ambient-background"`| `"ambient_background"`|
//! | `"alert-banner"`      | `"alert_banner"`     |

// ─── ReadabilityTechnique ─────────────────────────────────────────────────────

/// Readability technique required by a component type.
///
/// Controls how the compositor must ensure text legibility in the zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadabilityTechnique {
    /// Backdrop + outline required.
    ///
    /// `RenderingPolicy` MUST have:
    /// - `backdrop` color set
    /// - `backdrop_opacity` >= 0.3
    /// - `outline_color` set
    /// - `outline_width` >= 1.0
    ///
    /// Widget SVGs MUST have `data-role="backdrop"` and `data-role="text"`
    /// elements with text stroke, with backdrop preceding text in document order.
    DualLayer,

    /// Opaque backdrop required.
    ///
    /// `RenderingPolicy` MUST have:
    /// - `backdrop` color set
    /// - `backdrop_opacity` >= 0.8
    ///
    /// No outline requirement.
    OpaqueBackdrop,

    /// No readability requirement (e.g. video surface, ambient display).
    None,
}

// ─── ComponentTypeContract ────────────────────────────────────────────────────

/// Static contract for a v1 component type.
///
/// Describes the visual-semantic identity requirements for a class of HUD
/// components. These are specification constants — not user-configurable in v1.
#[derive(Clone, Debug)]
pub struct ComponentTypeContract {
    /// Unique kebab-case name for this component type.
    ///
    /// Matches the zone registry name (NOT the config constant).
    pub name: &'static str,

    /// The zone type name this component type governs (registry name).
    ///
    /// Profile zone override files MUST use this name
    /// (e.g. `zones/notification-area.toml`).
    pub zone_type_name: &'static str,

    /// Readability technique required for all active profiles of this type.
    pub readability: ReadabilityTechnique,

    /// Canonical token keys that MUST be resolvable for any active profile.
    ///
    /// Tokens are resolved from: profile overrides → global tokens → canonical
    /// fallbacks. All keys in this list MUST resolve to a value at startup.
    pub required_tokens: &'static [&'static str],

    /// Informal geometry description for profile authors.
    ///
    /// Not validated at startup — geometry is governed by the zone type's
    /// `GeometryPolicy`, not by the component type contract.
    pub geometry_note: &'static str,
}

// ─── ComponentType ────────────────────────────────────────────────────────────

/// The six v1 component types.
///
/// Each variant corresponds to a named visual-semantic role in the HUD.
/// Use [`ComponentType::contract`] to retrieve the full static contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComponentType {
    /// Bottom-strip subtitle overlay, rendered via glyphon.
    Subtitle,

    /// Top-right notification tray, stacked vertically, auto-dismissing.
    Notification,

    /// Full-width status strip, top or bottom edge.
    StatusBar,

    /// Full-width urgent alert bar with severity-tinted backdrop.
    AlertBanner,

    /// Full-screen background ambient display (no text content in v1).
    AmbientBackground,

    /// Corner-anchored picture-in-picture video surface.
    Pip,
}

impl ComponentType {
    /// Returns the name of all six v1 component types in canonical order.
    pub const ALL: &'static [ComponentType] = &[
        ComponentType::Subtitle,
        ComponentType::Notification,
        ComponentType::StatusBar,
        ComponentType::AlertBanner,
        ComponentType::AmbientBackground,
        ComponentType::Pip,
    ];

    /// Returns the static contract for this component type.
    ///
    /// Contracts are specification constants defined by
    /// `component-shape-language/spec.md §Requirement: V1 Component Type Definitions`.
    pub fn contract(self) -> ComponentTypeContract {
        match self {
            ComponentType::Subtitle => ComponentTypeContract {
                name: "subtitle",
                zone_type_name: "subtitle",
                readability: ReadabilityTechnique::DualLayer,
                required_tokens: &[
                    "color.text.primary",
                    "color.backdrop.default",
                    "opacity.backdrop.default",
                    "color.outline.default",
                    "typography.subtitle.family",
                    "typography.subtitle.size",
                    "typography.subtitle.weight",
                    "stroke.outline.width",
                ],
                geometry_note: "bottom-center, 5–15% screen height, centered text",
            },

            ComponentType::Notification => ComponentTypeContract {
                name: "notification",
                // IMPORTANT: uses registry name "notification-area", NOT config
                // constant "notification". Zone override files MUST use
                // zones/notification-area.toml.
                zone_type_name: "notification-area",
                readability: ReadabilityTechnique::OpaqueBackdrop,
                required_tokens: &[
                    "color.text.primary",
                    "color.backdrop.default",
                    "opacity.backdrop.opaque",
                    "color.border.default",
                    "typography.body.family",
                    "typography.body.size",
                    "typography.body.weight",
                    "spacing.padding.medium",
                    "stroke.border.width",
                ],
                geometry_note: "top-right corner, stacked vertically, auto-dismisses",
            },

            ComponentType::StatusBar => ComponentTypeContract {
                name: "status-bar",
                // Registry name is "status-bar"; config constant is "status_bar".
                zone_type_name: "status-bar",
                readability: ReadabilityTechnique::OpaqueBackdrop,
                required_tokens: &[
                    "color.text.secondary",
                    "color.backdrop.default",
                    "opacity.backdrop.opaque",
                    "typography.body.family",
                    "typography.body.size",
                ],
                geometry_note: "full-width strip, top or bottom edge",
            },

            ComponentType::AlertBanner => ComponentTypeContract {
                name: "alert-banner",
                // Registry name is "alert-banner"; config constant is "alert_banner".
                zone_type_name: "alert-banner",
                readability: ReadabilityTechnique::OpaqueBackdrop,
                required_tokens: &[
                    "color.text.primary",
                    "color.backdrop.default",
                    "opacity.backdrop.opaque",
                    "color.severity.info",
                    "color.severity.warning",
                    "color.severity.error",
                    "color.severity.critical",
                    "typography.heading.family",
                    "typography.heading.size",
                    "typography.heading.weight",
                ],
                geometry_note: "full-width horizontal bar; backdrop tinted by urgency-to-severity token mapping",
            },

            ComponentType::AmbientBackground => ComponentTypeContract {
                name: "ambient-background",
                // Registry name is "ambient-background"; config constant is "ambient_background".
                zone_type_name: "ambient-background",
                readability: ReadabilityTechnique::None,
                required_tokens: &[],
                geometry_note: "full-screen background layer",
            },

            ComponentType::Pip => ComponentTypeContract {
                name: "pip",
                zone_type_name: "pip",
                readability: ReadabilityTechnique::None,
                required_tokens: &["color.border.default", "stroke.border.width"],
                geometry_note: "corner-anchored, resizable within bounds; border tokens reserved for post-v1 border rendering",
            },
        }
    }

    /// Parses a kebab-case component type name into a [`ComponentType`].
    ///
    /// Returns `None` if the name is not a recognized v1 component type.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "subtitle" => Some(ComponentType::Subtitle),
            "notification" => Some(ComponentType::Notification),
            "status-bar" => Some(ComponentType::StatusBar),
            "alert-banner" => Some(ComponentType::AlertBanner),
            "ambient-background" => Some(ComponentType::AmbientBackground),
            "pip" => Some(ComponentType::Pip),
            _ => None,
        }
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Subtitle ─────────────────────────────────────────────────────────────

    #[test]
    fn subtitle_contract_name() {
        let c = ComponentType::Subtitle.contract();
        assert_eq!(c.name, "subtitle");
    }

    #[test]
    fn subtitle_contract_zone_type_name() {
        let c = ComponentType::Subtitle.contract();
        assert_eq!(c.zone_type_name, "subtitle");
    }

    #[test]
    fn subtitle_contract_readability_dual_layer() {
        let c = ComponentType::Subtitle.contract();
        assert_eq!(c.readability, ReadabilityTechnique::DualLayer);
    }

    #[test]
    fn subtitle_contract_required_tokens() {
        let c = ComponentType::Subtitle.contract();
        let expected: &[&str] = &[
            "color.text.primary",
            "color.backdrop.default",
            "opacity.backdrop.default",
            "color.outline.default",
            "typography.subtitle.family",
            "typography.subtitle.size",
            "typography.subtitle.weight",
            "stroke.outline.width",
        ];
        assert_eq!(c.required_tokens, expected);
    }

    // ── Notification ──────────────────────────────────────────────────────────

    #[test]
    fn notification_contract_name() {
        let c = ComponentType::Notification.contract();
        assert_eq!(c.name, "notification");
    }

    /// Zone Name Reconciliation: notification governs "notification-area" in the
    /// zone registry, NOT "notification" (which is the config constant name).
    #[test]
    fn notification_contract_zone_type_name_is_registry_name() {
        let c = ComponentType::Notification.contract();
        assert_eq!(
            c.zone_type_name, "notification-area",
            "notification zone_type_name must be the registry name 'notification-area', \
             not the config constant 'notification'"
        );
    }

    #[test]
    fn notification_contract_readability_opaque_backdrop() {
        let c = ComponentType::Notification.contract();
        assert_eq!(c.readability, ReadabilityTechnique::OpaqueBackdrop);
    }

    #[test]
    fn notification_contract_required_tokens() {
        let c = ComponentType::Notification.contract();
        let expected: &[&str] = &[
            "color.text.primary",
            "color.backdrop.default",
            "opacity.backdrop.opaque",
            "color.border.default",
            "typography.body.family",
            "typography.body.size",
            "typography.body.weight",
            "spacing.padding.medium",
            "stroke.border.width",
        ];
        assert_eq!(c.required_tokens, expected);
    }

    // ── StatusBar ─────────────────────────────────────────────────────────────

    #[test]
    fn status_bar_contract_name() {
        let c = ComponentType::StatusBar.contract();
        assert_eq!(c.name, "status-bar");
    }

    /// Zone Name Reconciliation: status-bar registry name is "status-bar",
    /// config constant is "status_bar".
    #[test]
    fn status_bar_contract_zone_type_name_is_registry_name() {
        let c = ComponentType::StatusBar.contract();
        assert_eq!(
            c.zone_type_name, "status-bar",
            "status-bar zone_type_name must be the registry name 'status-bar', \
             not the config constant 'status_bar'"
        );
    }

    #[test]
    fn status_bar_contract_readability_opaque_backdrop() {
        let c = ComponentType::StatusBar.contract();
        assert_eq!(c.readability, ReadabilityTechnique::OpaqueBackdrop);
    }

    #[test]
    fn status_bar_contract_required_tokens() {
        let c = ComponentType::StatusBar.contract();
        let expected: &[&str] = &[
            "color.text.secondary",
            "color.backdrop.default",
            "opacity.backdrop.opaque",
            "typography.body.family",
            "typography.body.size",
        ];
        assert_eq!(c.required_tokens, expected);
    }

    // ── AlertBanner ───────────────────────────────────────────────────────────

    #[test]
    fn alert_banner_contract_name() {
        let c = ComponentType::AlertBanner.contract();
        assert_eq!(c.name, "alert-banner");
    }

    /// Zone Name Reconciliation: alert-banner registry name is "alert-banner",
    /// config constant is "alert_banner".
    #[test]
    fn alert_banner_contract_zone_type_name_is_registry_name() {
        let c = ComponentType::AlertBanner.contract();
        assert_eq!(
            c.zone_type_name, "alert-banner",
            "alert-banner zone_type_name must be the registry name 'alert-banner', \
             not the config constant 'alert_banner'"
        );
    }

    #[test]
    fn alert_banner_contract_readability_opaque_backdrop() {
        let c = ComponentType::AlertBanner.contract();
        assert_eq!(c.readability, ReadabilityTechnique::OpaqueBackdrop);
    }

    #[test]
    fn alert_banner_contract_required_tokens() {
        let c = ComponentType::AlertBanner.contract();
        let expected: &[&str] = &[
            "color.text.primary",
            "color.backdrop.default",
            "opacity.backdrop.opaque",
            "color.severity.info",
            "color.severity.warning",
            "color.severity.error",
            "color.severity.critical",
            "typography.heading.family",
            "typography.heading.size",
            "typography.heading.weight",
        ];
        assert_eq!(c.required_tokens, expected);
    }

    /// Alert banner must include all four severity tokens for urgency-to-severity mapping.
    #[test]
    fn alert_banner_has_all_severity_tokens() {
        let c = ComponentType::AlertBanner.contract();
        assert!(
            c.required_tokens.contains(&"color.severity.info"),
            "alert-banner must require color.severity.info"
        );
        assert!(
            c.required_tokens.contains(&"color.severity.warning"),
            "alert-banner must require color.severity.warning"
        );
        assert!(
            c.required_tokens.contains(&"color.severity.error"),
            "alert-banner must require color.severity.error"
        );
        assert!(
            c.required_tokens.contains(&"color.severity.critical"),
            "alert-banner must require color.severity.critical"
        );
    }

    // ── AmbientBackground ─────────────────────────────────────────────────────

    #[test]
    fn ambient_background_contract_name() {
        let c = ComponentType::AmbientBackground.contract();
        assert_eq!(c.name, "ambient-background");
    }

    /// Zone Name Reconciliation: ambient-background registry name is "ambient-background",
    /// config constant is "ambient_background".
    #[test]
    fn ambient_background_contract_zone_type_name_is_registry_name() {
        let c = ComponentType::AmbientBackground.contract();
        assert_eq!(
            c.zone_type_name, "ambient-background",
            "ambient-background zone_type_name must be the registry name 'ambient-background', \
             not the config constant 'ambient_background'"
        );
    }

    #[test]
    fn ambient_background_contract_readability_none() {
        let c = ComponentType::AmbientBackground.contract();
        assert_eq!(c.readability, ReadabilityTechnique::None);
    }

    #[test]
    fn ambient_background_contract_required_tokens_empty() {
        let c = ComponentType::AmbientBackground.contract();
        assert!(
            c.required_tokens.is_empty(),
            "ambient-background has no required tokens (no text content in v1)"
        );
    }

    // ── Pip ───────────────────────────────────────────────────────────────────

    #[test]
    fn pip_contract_name() {
        let c = ComponentType::Pip.contract();
        assert_eq!(c.name, "pip");
    }

    #[test]
    fn pip_contract_zone_type_name() {
        let c = ComponentType::Pip.contract();
        assert_eq!(c.zone_type_name, "pip");
    }

    #[test]
    fn pip_contract_readability_none() {
        let c = ComponentType::Pip.contract();
        assert_eq!(c.readability, ReadabilityTechnique::None);
    }

    #[test]
    fn pip_contract_required_tokens() {
        let c = ComponentType::Pip.contract();
        let expected: &[&str] = &["color.border.default", "stroke.border.width"];
        assert_eq!(c.required_tokens, expected);
    }

    // ── All six types ─────────────────────────────────────────────────────────

    #[test]
    fn all_six_v1_component_types_defined() {
        assert_eq!(
            ComponentType::ALL.len(),
            6,
            "spec requires exactly 6 v1 component types"
        );
        let names: Vec<&str> = ComponentType::ALL
            .iter()
            .map(|ct| ct.contract().name)
            .collect();
        assert!(names.contains(&"subtitle"));
        assert!(names.contains(&"notification"));
        assert!(names.contains(&"status-bar"));
        assert!(names.contains(&"alert-banner"));
        assert!(names.contains(&"ambient-background"));
        assert!(names.contains(&"pip"));
    }

    #[test]
    fn from_name_round_trips_all_types() {
        for ct in ComponentType::ALL {
            let name = ct.contract().name;
            let parsed = ComponentType::from_name(name);
            assert_eq!(
                parsed,
                Some(*ct),
                "from_name({name:?}) should round-trip to {ct:?}"
            );
        }
    }

    #[test]
    fn from_name_returns_none_for_unknown() {
        assert_eq!(ComponentType::from_name("unknown-type"), None);
        assert_eq!(ComponentType::from_name(""), None);
        assert_eq!(ComponentType::from_name("Subtitle"), None); // case-sensitive
    }

    #[test]
    fn from_name_rejects_config_constant_forms() {
        // Config constants use underscores; component type names use hyphens.
        assert_eq!(
            ComponentType::from_name("status_bar"),
            None,
            "config constant 'status_bar' should not match; use 'status-bar'"
        );
        assert_eq!(
            ComponentType::from_name("alert_banner"),
            None,
            "config constant 'alert_banner' should not match; use 'alert-banner'"
        );
        assert_eq!(
            ComponentType::from_name("ambient_background"),
            None,
            "config constant 'ambient_background' should not match; use 'ambient-background'"
        );
    }

    #[test]
    fn all_zone_type_names_are_registry_names() {
        // Verify zone_type_name values match the zone registry (not config constants).
        // Registry names are defined in ZoneRegistry::with_defaults().
        let expected_registry_names = [
            ("subtitle", "subtitle"),
            ("notification", "notification-area"), // NOT "notification"
            ("status-bar", "status-bar"),          // NOT "status_bar"
            ("alert-banner", "alert-banner"),      // NOT "alert_banner"
            ("ambient-background", "ambient-background"), // NOT "ambient_background"
            ("pip", "pip"),
        ];
        for (ct_name, expected_zone) in &expected_registry_names {
            let ct = ComponentType::from_name(ct_name).unwrap();
            assert_eq!(
                ct.contract().zone_type_name,
                *expected_zone,
                "component type '{ct_name}' should govern zone '{expected_zone}'"
            );
        }
    }

    #[test]
    fn geometry_note_is_non_empty_for_types_with_text() {
        // Types with text content should have non-empty geometry notes.
        for ct in &[
            ComponentType::Subtitle,
            ComponentType::Notification,
            ComponentType::StatusBar,
            ComponentType::AlertBanner,
        ] {
            let c = ct.contract();
            assert!(
                !c.geometry_note.is_empty(),
                "{} contract should have a non-empty geometry_note",
                c.name
            );
        }
    }
}
