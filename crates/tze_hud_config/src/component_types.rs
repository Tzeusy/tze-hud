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

use tze_hud_scene::types::PortalPartKind;

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

/// The six v1 component types plus the promotion-era `text-portal` type.
///
/// Each variant corresponds to a named visual-semantic role in the HUD.
/// Use [`ComponentType::contract`] to retrieve the full static contract.
///
/// The first six variants are the v1 zone-governing component types. The
/// seventh, [`ComponentType::TextPortal`], is a **promotion-era** type whose
/// first-class surface exists only after the RFC 0013 §7.2 promotion gate
/// passes; it governs a multi-part portal surface rather than a single zone
/// type (see `component-shape-language/spec.md §Requirement: Text-Portal
/// Component Type`). It is defined **in addition to** the six v1 types and does
/// not alter them (hud-m4xay / reconciliation finding F4).
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

    /// Promotion-era text-stream portal surface (RFC 0013 §7.2). Governs a
    /// first-class portal surface composed of named parts (frame, header,
    /// composer, transcript, divider, collapsed-card, capture-backstop,
    /// gesture-shield) rather than a single zone type. Readability is declared
    /// **per part** (see [`ComponentType::text_portal_part_readability`]).
    TextPortal,
}

impl ComponentType {
    /// All defined component types in canonical order: the six v1 types followed
    /// by the promotion-era `text-portal` type.
    pub const ALL: &'static [ComponentType] = &[
        ComponentType::Subtitle,
        ComponentType::Notification,
        ComponentType::StatusBar,
        ComponentType::AlertBanner,
        ComponentType::AmbientBackground,
        ComponentType::Pip,
        ComponentType::TextPortal,
    ];

    /// The six v1 zone-governing component types, in canonical order. Excludes
    /// the promotion-era [`ComponentType::TextPortal`], which governs a portal
    /// surface rather than a v1 zone.
    pub const V1: &'static [ComponentType] = &[
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

            ComponentType::TextPortal => ComponentTypeContract {
                name: "text-portal",
                // text-portal governs a first-class portal SURFACE (a set of
                // named parts), not a single zone type. This nominal name
                // matches nothing in the zone registry, so the zone-oriented
                // effective-policy builder (`build_all_effective_policies`)
                // simply skips it; readability is enforced per-part instead
                // (see `run_component_startup` step 7 and
                // `text_portal_part_readability`).
                zone_type_name: "text-portal",
                // Surface-level default technique for text-bearing parts; the
                // authoritative per-part mapping is `text_portal_part_readability`.
                readability: ReadabilityTechnique::OpaqueBackdrop,
                // Reuses existing canonical keys only — no new canonical key
                // (component-shape-language/spec.md §Text-Portal Component Type).
                required_tokens: &[
                    "color.text.primary",
                    "color.text.secondary",
                    "color.backdrop.default",
                    "color.border.default",
                    "color.outline.default",
                    "opacity.backdrop.opaque",
                    "typography.heading.family",
                    "typography.heading.size",
                    "typography.heading.weight",
                    "typography.body.family",
                    "typography.body.size",
                    "typography.body.weight",
                    "spacing.padding.medium",
                    "stroke.border.width",
                    "stroke.outline.width",
                ],
                geometry_note: "content-layer, lease-governed, movable/resizable two-pane surface (transcript + composer) with a header band and a collapsed-card state; governed by the surface's own bounds/lease, not the component type",
            },
        }
    }

    /// Readability technique required for a given `text-portal` surface part.
    ///
    /// Per `component-shape-language/spec.md §Requirement: Text-Portal
    /// Readability Enforcement`, the text-bearing parts (`frame`, `header`,
    /// `composer`, `transcript`, `collapsed-card`) require `OpaqueBackdrop`
    /// (`backdrop` set and `backdrop_opacity >= 0.8`), and the geometry-only
    /// parts (`divider`, `capture-backstop`, `gesture-shield`) require `None`.
    ///
    /// The text-bearing / geometry-only split is the single source of truth
    /// [`PortalPartKind::is_text_bearing`], so this mapping stays in lockstep
    /// with the scene model.
    pub fn text_portal_part_readability(part: PortalPartKind) -> ReadabilityTechnique {
        if part.is_text_bearing() {
            ReadabilityTechnique::OpaqueBackdrop
        } else {
            ReadabilityTechnique::None
        }
    }

    /// Parses a kebab-case component type name into a [`ComponentType`].
    ///
    /// Recognizes the six v1 component types and the promotion-era
    /// `text-portal` type. Returns `None` for any other name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "subtitle" => Some(ComponentType::Subtitle),
            "notification" => Some(ComponentType::Notification),
            "status-bar" => Some(ComponentType::StatusBar),
            "alert-banner" => Some(ComponentType::AlertBanner),
            "ambient-background" => Some(ComponentType::AmbientBackground),
            "pip" => Some(ComponentType::Pip),
            "text-portal" => Some(ComponentType::TextPortal),
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
    fn six_v1_component_types_defined() {
        assert_eq!(
            ComponentType::V1.len(),
            6,
            "spec requires exactly 6 v1 component types"
        );
        let names: Vec<&str> = ComponentType::V1
            .iter()
            .map(|ct| ct.contract().name)
            .collect();
        assert!(names.contains(&"subtitle"));
        assert!(names.contains(&"notification"));
        assert!(names.contains(&"status-bar"));
        assert!(names.contains(&"alert-banner"));
        assert!(names.contains(&"ambient-background"));
        assert!(names.contains(&"pip"));
        // TextPortal is a promotion-era type, NOT a v1 zone-governing type.
        assert!(
            !names.contains(&"text-portal"),
            "text-portal must not be one of the six v1 types"
        );
    }

    #[test]
    fn all_includes_six_v1_plus_text_portal() {
        assert_eq!(
            ComponentType::ALL.len(),
            7,
            "ALL is the six v1 types plus the promotion-era text-portal type"
        );
        let names: Vec<&str> = ComponentType::ALL
            .iter()
            .map(|ct| ct.contract().name)
            .collect();
        for v1 in ComponentType::V1 {
            assert!(
                names.contains(&v1.contract().name),
                "ALL must contain v1 type {v1:?}"
            );
        }
        assert!(
            names.contains(&"text-portal"),
            "ALL must contain the promotion-era text-portal type (reconciliation F4)"
        );
    }

    // ── TextPortal (promotion-era) ────────────────────────────────────────────

    #[test]
    fn text_portal_contract_name() {
        assert_eq!(ComponentType::TextPortal.contract().name, "text-portal");
    }

    #[test]
    fn text_portal_from_name_round_trips() {
        assert_eq!(
            ComponentType::from_name("text-portal"),
            Some(ComponentType::TextPortal)
        );
    }

    /// Surface-level default technique is OpaqueBackdrop (spec §Text-Portal
    /// Component Type: "surface-level default for text-bearing parts is
    /// OpaqueBackdrop").
    #[test]
    fn text_portal_contract_readability_opaque_backdrop() {
        assert_eq!(
            ComponentType::TextPortal.contract().readability,
            ReadabilityTechnique::OpaqueBackdrop
        );
    }

    /// Per-part readability mirrors `PortalPartKind::is_text_bearing`:
    /// text-bearing parts require OpaqueBackdrop; geometry-only parts require None.
    #[test]
    fn text_portal_part_readability_matches_spec_table() {
        use tze_hud_scene::types::PortalPartKind;
        // Text-bearing → OpaqueBackdrop.
        for part in [
            PortalPartKind::Frame,
            PortalPartKind::Header,
            PortalPartKind::Composer,
            PortalPartKind::Transcript,
            PortalPartKind::CollapsedCard,
        ] {
            assert_eq!(
                ComponentType::text_portal_part_readability(part),
                ReadabilityTechnique::OpaqueBackdrop,
                "{part:?} is text-bearing and must require OpaqueBackdrop"
            );
        }
        // Geometry-only → None.
        for part in [
            PortalPartKind::Divider,
            PortalPartKind::CaptureBackstop,
            PortalPartKind::GestureShield,
        ] {
            assert_eq!(
                ComponentType::text_portal_part_readability(part),
                ReadabilityTechnique::None,
                "{part:?} is geometry-only and must require None"
            );
        }
    }

    /// The text-portal required-tokens list introduces no NEW canonical key: every
    /// key must already be resolvable in the canonical token schema.
    #[test]
    fn text_portal_required_tokens_are_all_canonical() {
        use crate::tokens::{DesignTokenMap, resolve_tokens};
        let canonical = resolve_tokens(&DesignTokenMap::new(), &DesignTokenMap::new());
        for key in ComponentType::TextPortal.contract().required_tokens {
            assert!(
                canonical.contains_key(*key),
                "text-portal required token '{key}' must be a canonical key (no new key introduced)"
            );
        }
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
