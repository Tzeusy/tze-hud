//! # Level 3 Security — Capability Checks and Canonical Name Validation
//!
//! Implements Level 3 security enforcement per spec §Requirement: Level 3 Security Enforcement
//! and §Requirement: Capability Registry Canonical Names.
//!
//! ## Canonical Names (spec §8.1 as amended by RFC 0005 Round 14)
//!
//! All capability names MUST use `snake_case`. Three names were revised in Round 14:
//!
//! | Pre-Round-14 (superseded) | Canonical (post-Round-14) |
//! |--------------------------|--------------------------|
//! | `read_scene`             | `read_scene_topology`    |
//! | `receive_input`          | `access_input_events`    |
//! | `zone_publish:<zone>`    | `publish_zone:<zone>`    |
//!
//! ## Latency Requirement
//!
//! Capability checks MUST use hash-table lookups; each check MUST complete in < 5us.
//! This module uses `std::collections::HashSet` which provides O(1) average lookup.

use std::collections::HashSet;

// ─── Canonical capability name validation ─────────────────────────────────────

/// Represents the result of validating a capability name's canonicality.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityNameCheck {
    /// The name is valid and canonical.
    Valid,
    /// The name is a superseded pre-Round-14 name. Contains the canonical replacement.
    Superseded { canonical: &'static str },
    /// The name uses an invalid format (not snake_case, unknown prefix, etc.).
    Invalid { reason: &'static str },
}

/// Check whether a capability name is canonical per spec §8.1 as amended by RFC 0005 Round 14.
///
/// Returns `CapabilityNameCheck::Valid` if the name may be used.
/// Returns `CapabilityNameCheck::Superseded` for the three pre-Round-14 names.
/// Returns `CapabilityNameCheck::Invalid` for names with incorrect case conventions.
///
/// # Spec compliance
///
/// WHEN a capability grant uses `read_scene`, `receive_input`, or `zone_publish:<zone>`
/// THEN startup fails with `CONFIG_UNKNOWN_CAPABILITY` with a hint naming the canonical
/// replacement (`read_scene_topology`, `access_input_events`, `publish_zone:<zone>`).
pub fn check_canonical_capability_name(name: &str) -> CapabilityNameCheck {
    // ── Pre-Round-14 superseded names ─────────────────────────────────────────
    if name == "read_scene" {
        return CapabilityNameCheck::Superseded { canonical: "read_scene_topology" };
    }
    if name == "receive_input" {
        return CapabilityNameCheck::Superseded { canonical: "access_input_events" };
    }
    // zone_publish:<zone> → publish_zone:<zone>
    if let Some(zone) = name.strip_prefix("zone_publish:") {
        // Provide a concrete canonical hint with the zone name preserved.
        // We return a static hint for the common form; the runtime should
        // construct the full hint string using `name` if the zone is dynamic.
        let _ = zone; // zone used for context; static hint covers the pattern
        return CapabilityNameCheck::Superseded { canonical: "publish_zone:<zone>" };
    }

    // ── Uppercase / kebab-case rejections ─────────────────────────────────────
    if name.chars().any(|c| c.is_uppercase()) {
        return CapabilityNameCheck::Invalid { reason: "capability names must be snake_case (no uppercase)" };
    }
    if name.contains('-') {
        return CapabilityNameCheck::Invalid { reason: "capability names must be snake_case (no hyphens)" };
    }

    CapabilityNameCheck::Valid
}

/// Returns `Some(canonical)` if `name` is a superseded pre-Round-14 capability name.
///
/// Returns `None` if the name is already canonical or otherwise unknown.
///
/// Use this in startup validation to produce `CONFIG_UNKNOWN_CAPABILITY` errors.
pub fn superseded_canonical(name: &str) -> Option<&'static str> {
    match check_canonical_capability_name(name) {
        CapabilityNameCheck::Superseded { canonical } => Some(canonical),
        _ => None,
    }
}

// ─── Capability registry (hash-set backed, O(1) lookup) ──────────────────────

/// A set of capabilities granted to an agent.
///
/// Backed by `HashSet<String>` for O(1) average lookup.
/// Supports `publish_zone:*` wildcard for zone publishing.
///
/// # Latency
///
/// Each `has_capability` call is a single hash-table lookup (< 5us under nominal load).
#[derive(Clone, Debug, Default)]
pub struct CapabilitySet {
    /// The set of granted capabilities (canonical snake_case names).
    capabilities: HashSet<String>,
    /// Whether the wildcard `publish_zone:*` is granted.
    publish_zone_wildcard: bool,
}

impl CapabilitySet {
    /// Create a new capability set from the given list of capability names.
    pub fn new(caps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut set = CapabilitySet::default();
        for cap in caps {
            set.grant(cap);
        }
        set
    }

    /// Grant a capability.
    pub fn grant(&mut self, cap: impl Into<String>) {
        let s: String = cap.into();
        if s == "publish_zone:*" {
            self.publish_zone_wildcard = true;
        }
        self.capabilities.insert(s);
    }

    /// Revoke a capability.
    pub fn revoke(&mut self, cap: &str) {
        if cap == "publish_zone:*" {
            self.publish_zone_wildcard = false;
        }
        self.capabilities.remove(cap);
    }

    /// Returns `true` if the agent holds the named capability.
    ///
    /// Supports `publish_zone:*` wildcard: if the wildcard is granted,
    /// any `publish_zone:<zone>` query returns `true`.
    ///
    /// This is a hash-table lookup; each call completes in < 5us under nominal load.
    #[inline]
    pub fn has(&self, required: &str) -> bool {
        if self.capabilities.contains(required) {
            return true;
        }
        // Wildcard: publish_zone:* covers any publish_zone:<name>
        if self.publish_zone_wildcard && required.starts_with("publish_zone:") {
            return true;
        }
        false
    }

    /// Returns the first missing capability from `required`, or `None` if all pass.
    ///
    /// Security is conjunctive: ALL required capabilities must be present.
    pub fn first_missing<'a>(&self, required: &[&'a str]) -> Option<&'a str> {
        required.iter().copied().find(|&cap| !self.has(cap))
    }

    /// Returns `true` if the set is empty (no capabilities granted).
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }
}

impl From<Vec<String>> for CapabilitySet {
    fn from(caps: Vec<String>) -> Self {
        Self::new(caps)
    }
}

impl From<&[&str]> for CapabilitySet {
    fn from(caps: &[&str]) -> Self {
        Self::new(caps.iter().copied())
    }
}

// ─── Startup capability config validation ─────────────────────────────────────

/// Error returned when a capability config uses a superseded or invalid name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigUnknownCapability {
    /// The name that was used in the config.
    pub used: String,
    /// The canonical replacement hint.
    pub hint: String,
}

impl std::fmt::Display for ConfigUnknownCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CONFIG_UNKNOWN_CAPABILITY: '{}' is superseded; use '{}' instead",
            self.used, self.hint
        )
    }
}

/// Validate a list of capability names for use in config.
///
/// Returns `Err(Vec<ConfigUnknownCapability>)` if any name is superseded or invalid.
/// Returns `Ok(())` if all names are canonical.
///
/// # Spec compliance
///
/// WHEN a capability grant uses `read_scene`, `receive_input`, or `zone_publish:<zone>`
/// THEN startup fails with `CONFIG_UNKNOWN_CAPABILITY` with a hint naming the canonical
/// replacement.
pub fn validate_capability_names(names: &[&str]) -> Result<(), Vec<ConfigUnknownCapability>> {
    let errors: Vec<_> = names
        .iter()
        .filter_map(|&name| match check_canonical_capability_name(name) {
            CapabilityNameCheck::Superseded { canonical } => {
                // For zone_publish:<zone>, build a concrete hint with the zone part.
                let hint = if let Some(zone) = name.strip_prefix("zone_publish:") {
                    format!("publish_zone:{zone}")
                } else {
                    canonical.to_string()
                };
                Some(ConfigUnknownCapability { used: name.to_string(), hint })
            }
            CapabilityNameCheck::Invalid { reason } => Some(ConfigUnknownCapability {
                used: name.to_string(),
                hint: reason.to_string(),
            }),
            CapabilityNameCheck::Valid => None,
        })
        .collect();

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    // ─── Canonical name validation ────────────────────────────────────────────

    /// WHEN capability check uses snake_case canonical names
    /// THEN they are accepted (spec lines 286-288)
    #[test]
    fn test_canonical_names_are_valid() {
        assert_eq!(check_canonical_capability_name("create_tiles"), CapabilityNameCheck::Valid);
        assert_eq!(check_canonical_capability_name("modify_own_tiles"), CapabilityNameCheck::Valid);
        assert_eq!(
            check_canonical_capability_name("read_scene_topology"),
            CapabilityNameCheck::Valid
        );
        assert_eq!(
            check_canonical_capability_name("access_input_events"),
            CapabilityNameCheck::Valid
        );
        assert_eq!(
            check_canonical_capability_name("publish_zone:notification"),
            CapabilityNameCheck::Valid
        );
        assert_eq!(
            check_canonical_capability_name("publish_zone:subtitle"),
            CapabilityNameCheck::Valid
        );
        assert_eq!(check_canonical_capability_name("publish_zone:*"), CapabilityNameCheck::Valid);
    }

    /// WHEN capability grant uses read_scene THEN superseded with hint read_scene_topology
    #[test]
    fn test_read_scene_is_superseded() {
        let result = check_canonical_capability_name("read_scene");
        assert_eq!(
            result,
            CapabilityNameCheck::Superseded { canonical: "read_scene_topology" }
        );
    }

    /// WHEN capability grant uses receive_input THEN superseded with hint access_input_events
    #[test]
    fn test_receive_input_is_superseded() {
        let result = check_canonical_capability_name("receive_input");
        assert_eq!(
            result,
            CapabilityNameCheck::Superseded { canonical: "access_input_events" }
        );
    }

    /// WHEN capability grant uses zone_publish:<zone> THEN superseded (spec lines 290-292)
    #[test]
    fn test_zone_publish_prefix_is_superseded() {
        let result = check_canonical_capability_name("zone_publish:subtitle");
        assert_eq!(
            result,
            CapabilityNameCheck::Superseded { canonical: "publish_zone:<zone>" }
        );
        let result2 = check_canonical_capability_name("zone_publish:notification");
        assert_eq!(
            result2,
            CapabilityNameCheck::Superseded { canonical: "publish_zone:<zone>" }
        );
    }

    /// WHEN capability name uses uppercase THEN invalid
    #[test]
    fn test_uppercase_names_are_invalid() {
        let result = check_canonical_capability_name("CREATE_TILE");
        assert!(matches!(result, CapabilityNameCheck::Invalid { .. }));
        let result2 = check_canonical_capability_name("CreateTile");
        assert!(matches!(result2, CapabilityNameCheck::Invalid { .. }));
    }

    /// WHEN capability name uses kebab-case THEN invalid
    #[test]
    fn test_kebab_case_names_are_invalid() {
        let result = check_canonical_capability_name("create-tiles");
        assert!(matches!(result, CapabilityNameCheck::Invalid { .. }));
    }

    // ─── Config validation ────────────────────────────────────────────────────

    /// WHEN startup config uses zone_publish:subtitle THEN CONFIG_UNKNOWN_CAPABILITY
    /// with hint publish_zone:subtitle (spec lines 290-292)
    #[test]
    fn test_validate_capability_names_rejects_superseded() {
        let result = validate_capability_names(&[
            "create_tiles",
            "zone_publish:subtitle",
            "read_scene",
        ]);
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 2);
        let hints: Vec<_> = errors.iter().map(|e| e.hint.as_str()).collect();
        assert!(hints.contains(&"publish_zone:subtitle"));
        assert!(hints.contains(&"read_scene_topology"));
    }

    #[test]
    fn test_validate_capability_names_accepts_canonical() {
        let result = validate_capability_names(&[
            "create_tiles",
            "modify_own_tiles",
            "publish_zone:notification",
            "read_scene_topology",
            "access_input_events",
        ]);
        assert!(result.is_ok());
    }

    // ─── CapabilitySet hash lookup ────────────────────────────────────────────

    #[test]
    fn test_capability_set_has_exact_match() {
        let set = CapabilitySet::new(vec!["create_tiles", "modify_own_tiles"]);
        assert!(set.has("create_tiles"));
        assert!(set.has("modify_own_tiles"));
        assert!(!set.has("publish_zone:subtitle"));
    }

    #[test]
    fn test_capability_set_wildcard_covers_any_zone() {
        let set = CapabilitySet::new(vec!["publish_zone:*"]);
        assert!(set.has("publish_zone:notification"));
        assert!(set.has("publish_zone:subtitle"));
        assert!(set.has("publish_zone:weather"));
        assert!(!set.has("create_tiles"));
    }

    #[test]
    fn test_capability_set_first_missing_conjunctive() {
        let set = CapabilitySet::new(vec!["create_tiles"]);
        let required = &["create_tiles", "publish_zone:subtitle"];
        let missing = set.first_missing(required);
        assert_eq!(missing, Some("publish_zone:subtitle"));
    }

    #[test]
    fn test_capability_set_first_missing_all_present() {
        let set = CapabilitySet::new(vec!["create_tiles", "publish_zone:subtitle"]);
        let required = &["create_tiles", "publish_zone:subtitle"];
        assert!(set.first_missing(required).is_none());
    }

    /// WHEN agent attempts to create a tile without create_tiles capability
    /// THEN missing capability named (spec lines 131-133)
    #[test]
    fn test_capability_set_names_missing_capability() {
        let set = CapabilitySet::new(vec!["modify_own_tiles"]);
        let missing = set.first_missing(&["create_tiles"]);
        assert_eq!(missing, Some("create_tiles"));
    }

    /// Verify grant / revoke lifecycle
    #[test]
    fn test_capability_set_grant_revoke() {
        let mut set = CapabilitySet::default();
        set.grant("create_tiles");
        assert!(set.has("create_tiles"));
        set.revoke("create_tiles");
        assert!(!set.has("create_tiles"));
    }

    #[test]
    fn test_capability_set_wildcard_grant_revoke() {
        let mut set = CapabilitySet::default();
        set.grant("publish_zone:*");
        assert!(set.has("publish_zone:foo"));
        set.revoke("publish_zone:*");
        assert!(!set.has("publish_zone:foo"));
    }
}
