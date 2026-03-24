//! # Event Type Naming Convention
//!
//! Implements the dotted namespace hierarchy per scene-events/spec.md §2.2,
//! Requirement: Event Type Naming Convention, lines 35-47.
//!
//! ## Naming grammar
//!
//! | Source  | Pattern                                            | Example                              |
//! |---------|----------------------------------------------------|--------------------------------------|
//! | Scene   | `scene.<object>.<action>`                          | `scene.tile.created`                 |
//! | Agent   | `agent.<namespace>.<category>.<action>`            | `agent.doorbell_agent.doorbell.ring` |
//! | System  | `system.<action>`                                  | `system.degradation_changed`         |
//! | Input   | `input.<device>.<action>` (governed by RFC 0004)   | `input.pointer.down`                 |
//!
//! ## Segment rules
//!
//! All segments (between dots) must consist only of lowercase ASCII letters,
//! digits, and underscores: `[a-z0-9_]+`.  Each segment must be non-empty and
//! must not start with a digit.
//!
//! ## Reserved prefixes
//!
//! The prefixes `system.` and `scene.` are reserved for runtime-generated
//! events.  Agents **must not** emit events with these prefixes.
//!
//! ## Agent bare names
//!
//! Agents supply a *bare name* (e.g., `doorbell.ring`).  The runtime
//! namespace-prefixes it as `agent.<namespace>.<bare_name>` before delivery.
//! Bare names must match: `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+`
//! (at least two dot-separated segments, each starting with a letter).

use std::fmt;

// ─── Errors ──────────────────────────────────────────────────────────────────

/// Errors produced by event type validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamingError {
    /// The event type string is empty.
    Empty,
    /// A segment (between dots) is empty (consecutive dots or leading/trailing dot).
    EmptySegment { position: usize },
    /// A segment contains a character that is not `[a-z0-9_]`.
    InvalidCharacter { segment: String, ch: char },
    /// A segment starts with a digit, which is not allowed.
    SegmentStartsWithDigit { segment: String },
    /// An agent event bare name used a reserved prefix (`system.` or `scene.`).
    ReservedPrefix { prefix: String },
    /// An agent event bare name does not have at least two segments (needs
    /// at least `<category>.<action>`).
    BareTooFewSegments,
    /// A fully-qualified event type does not match any known prefix structure.
    UnknownPrefix,
}

impl fmt::Display for NamingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NamingError::Empty => write!(f, "event type must not be empty"),
            NamingError::EmptySegment { position } => {
                write!(f, "empty segment at position {position} (consecutive dots?)")
            }
            NamingError::InvalidCharacter { segment, ch } => write!(
                f,
                "segment {segment:?} contains invalid character {ch:?} (only [a-z0-9_] allowed)"
            ),
            NamingError::SegmentStartsWithDigit { segment } => {
                write!(f, "segment {segment:?} must not start with a digit")
            }
            NamingError::ReservedPrefix { prefix } => write!(
                f,
                "agent events must not use the reserved prefix {prefix:?}"
            ),
            NamingError::BareTooFewSegments => write!(
                f,
                "agent bare name must have at least two segments (e.g. \"doorbell.ring\")"
            ),
            NamingError::UnknownPrefix => {
                write!(f, "event type must start with scene., agent., system., or input.")
            }
        }
    }
}

impl std::error::Error for NamingError {}

// ─── Segment validation ───────────────────────────────────────────────────────

/// Validate a single dotted-name segment: `[a-z][a-z0-9_]*`.
///
/// Returns `Ok(())` on success or `Err(NamingError)` describing the first
/// problem found.
fn validate_segment(segment: &str, position: usize) -> Result<(), NamingError> {
    if segment.is_empty() {
        return Err(NamingError::EmptySegment { position });
    }
    let first = segment.chars().next().unwrap();
    if first.is_ascii_digit() {
        return Err(NamingError::SegmentStartsWithDigit {
            segment: segment.to_string(),
        });
    }
    for ch in segment.chars() {
        if !matches!(ch, 'a'..='z' | '0'..='9' | '_') {
            return Err(NamingError::InvalidCharacter {
                segment: segment.to_string(),
                ch,
            });
        }
    }
    Ok(())
}

// ─── Full event type validation ───────────────────────────────────────────────

/// Validate a fully-qualified event type string.
///
/// Accepts well-formed `scene.*`, `agent.*`, `system.*`, or `input.*` strings.
/// Each dot-separated segment must consist of `[a-z0-9_]+` and must not start
/// with a digit.
///
/// # Examples
///
/// ```
/// use tze_hud_scene::events::naming::validate_event_type;
///
/// assert!(validate_event_type("scene.tile.created").is_ok());
/// assert!(validate_event_type("agent.doorbell_agent.doorbell.ring").is_ok());
/// assert!(validate_event_type("system.degradation_changed").is_ok());
/// assert!(validate_event_type("system.lease_revoked").is_ok());
/// assert!(validate_event_type("scene.zone.occupancy_changed").is_ok());
/// assert!(validate_event_type("scene.focus.changed").is_ok());
/// assert!(validate_event_type("BadName").is_err()); // uppercase
/// assert!(validate_event_type("scene.tile").is_err()); // too few segments for scene
/// ```
pub fn validate_event_type(event_type: &str) -> Result<(), NamingError> {
    if event_type.is_empty() {
        return Err(NamingError::Empty);
    }

    let segments: Vec<&str> = event_type.split('.').collect();

    // Validate each segment for character constraints.
    for (i, seg) in segments.iter().enumerate() {
        validate_segment(seg, i)?;
    }

    // Validate structure based on leading prefix.
    match segments[0] {
        "scene" => {
            // scene.<object>.<action> — minimum 3 segments.
            if segments.len() < 3 {
                return Err(NamingError::UnknownPrefix);
            }
        }
        "agent" => {
            // agent.<namespace>.<category>.<action> — minimum 4 segments.
            if segments.len() < 4 {
                return Err(NamingError::UnknownPrefix);
            }
        }
        "system" => {
            // system.<action> — minimum 2 segments.
            if segments.len() < 2 {
                return Err(NamingError::UnknownPrefix);
            }
        }
        "input" => {
            // input.* — minimum 2 segments (governed by RFC 0004).
            if segments.len() < 2 {
                return Err(NamingError::UnknownPrefix);
            }
        }
        _ => {
            return Err(NamingError::UnknownPrefix);
        }
    }

    Ok(())
}

// ─── Agent bare-name validation ───────────────────────────────────────────────

/// Validate an agent-supplied bare event name.
///
/// Bare names are the `<category>.<action>` suffix that agents supply.
/// The runtime prepends `agent.<namespace>.` before delivery.
///
/// Rules (spec §2.2, bead #4 regex: `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+`):
/// - At least two segments separated by a dot.
/// - Each segment: `[a-z][a-z0-9_]*`.
/// - Must not begin with the reserved prefixes `system.` or `scene.`.
///
/// # Examples
///
/// ```
/// use tze_hud_scene::events::naming::validate_bare_name;
///
/// assert!(validate_bare_name("doorbell.ring").is_ok());
/// assert!(validate_bare_name("fire.detected").is_ok());
/// assert!(validate_bare_name("weather.update").is_ok());
/// assert!(validate_bare_name("system.fake").is_err()); // reserved prefix
/// assert!(validate_bare_name("scene.impersonate").is_err()); // reserved prefix
/// assert!(validate_bare_name("doorbell").is_err()); // needs two segments
/// assert!(validate_bare_name("Doorbell.Ring").is_err()); // uppercase
/// assert!(validate_bare_name("9invalid.start").is_err()); // digit start
/// ```
pub fn validate_bare_name(bare_name: &str) -> Result<(), NamingError> {
    if bare_name.is_empty() {
        return Err(NamingError::Empty);
    }

    // Reserved prefix check (spec line 46).
    if bare_name.starts_with("system.") {
        return Err(NamingError::ReservedPrefix {
            prefix: "system.".to_string(),
        });
    }
    if bare_name.starts_with("scene.") {
        return Err(NamingError::ReservedPrefix {
            prefix: "scene.".to_string(),
        });
    }

    let segments: Vec<&str> = bare_name.split('.').collect();
    if segments.len() < 2 {
        return Err(NamingError::BareTooFewSegments);
    }

    for (i, seg) in segments.iter().enumerate() {
        validate_segment(seg, i)?;
    }

    Ok(())
}

// ─── Namespace prefixing ──────────────────────────────────────────────────────

/// Build the fully-qualified agent event type from namespace and bare name.
///
/// Spec: scene-events/spec.md line 42 — "the delivered event_type MUST be
/// `agent.doorbell_agent.doorbell.ring`".
///
/// This function does **not** validate the bare name.  Call
/// `validate_bare_name` first if the bare name comes from an untrusted source.
///
/// # Examples
///
/// ```
/// use tze_hud_scene::events::naming::build_agent_event_type;
///
/// let t = build_agent_event_type("doorbell_agent", "doorbell.ring");
/// assert_eq!(t, "agent.doorbell_agent.doorbell.ring");
///
/// let t2 = build_agent_event_type("alarm_agent", "fire.detected");
/// assert_eq!(t2, "agent.alarm_agent.fire.detected");
/// ```
pub fn build_agent_event_type(namespace: &str, bare_name: &str) -> String {
    format!("agent.{namespace}.{bare_name}")
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_event_type ───────────────────────────────────────────────────

    /// Spec-mandated examples that MUST be valid (spec lines 35-47).
    #[test]
    fn valid_scene_event_types() {
        assert!(validate_event_type("scene.tile.created").is_ok());
        assert!(validate_event_type("scene.zone.occupancy_changed").is_ok());
        assert!(validate_event_type("scene.focus.changed").is_ok());
        assert!(validate_event_type("scene.tab.active_changed").is_ok());
    }

    #[test]
    fn valid_agent_event_types() {
        assert!(validate_event_type("agent.doorbell_agent.doorbell.ring").is_ok());
        assert!(validate_event_type("agent.alarm_agent.fire.detected").is_ok());
    }

    #[test]
    fn valid_system_event_types() {
        assert!(validate_event_type("system.degradation_changed").is_ok());
        assert!(validate_event_type("system.lease_revoked").is_ok());
    }

    #[test]
    fn invalid_event_type_uppercase() {
        assert!(validate_event_type("Scene.tile.created").is_err());
        assert!(validate_event_type("scene.Tile.created").is_err());
    }

    #[test]
    fn invalid_event_type_unknown_prefix() {
        assert!(validate_event_type("unknown.event.type").is_err());
    }

    #[test]
    fn invalid_event_type_empty() {
        assert!(validate_event_type("").is_err());
    }

    #[test]
    fn invalid_event_type_too_few_scene_segments() {
        // scene must have at least 3 segments
        assert!(validate_event_type("scene.tile").is_err());
    }

    #[test]
    fn invalid_event_type_too_few_agent_segments() {
        // agent must have at least 4 segments
        assert!(validate_event_type("agent.ns.action").is_err());
    }

    #[test]
    fn invalid_event_type_consecutive_dots() {
        assert!(validate_event_type("scene..tile.created").is_err());
    }

    #[test]
    fn invalid_event_type_special_characters() {
        assert!(validate_event_type("scene.tile-created").is_err());
        assert!(validate_event_type("scene.tile.created!").is_err());
    }

    // ── validate_bare_name ────────────────────────────────────────────────────

    /// WHEN an agent attempts to emit an event with name starting with "system."
    /// or "scene." THEN the runtime MUST reject the emission (spec line 46).
    #[test]
    fn reserved_prefix_system_rejected() {
        let err = validate_bare_name("system.fake").unwrap_err();
        assert!(
            matches!(err, NamingError::ReservedPrefix { ref prefix } if prefix == "system."),
            "expected ReservedPrefix(system.), got {err:?}"
        );
    }

    #[test]
    fn reserved_prefix_scene_rejected() {
        let err = validate_bare_name("scene.impersonate").unwrap_err();
        assert!(
            matches!(err, NamingError::ReservedPrefix { ref prefix } if prefix == "scene."),
            "expected ReservedPrefix(scene.), got {err:?}"
        );
    }

    #[test]
    fn valid_bare_names() {
        assert!(validate_bare_name("doorbell.ring").is_ok());
        assert!(validate_bare_name("fire.detected").is_ok());
        assert!(validate_bare_name("weather.update").is_ok());
        assert!(validate_bare_name("status.heartbeat.alive").is_ok());
    }

    #[test]
    fn bare_name_too_few_segments() {
        assert!(matches!(
            validate_bare_name("doorbell"),
            Err(NamingError::BareTooFewSegments)
        ));
    }

    #[test]
    fn bare_name_uppercase_rejected() {
        assert!(validate_bare_name("Doorbell.Ring").is_err());
    }

    #[test]
    fn bare_name_digit_start_rejected() {
        assert!(validate_bare_name("9invalid.start").is_err());
    }

    #[test]
    fn bare_name_leading_dot_rejected() {
        assert!(validate_bare_name(".ring").is_err());
    }

    // ── build_agent_event_type ────────────────────────────────────────────────

    /// WHEN an agent with namespace "doorbell_agent" emits event "doorbell.ring"
    /// THEN the delivered event_type MUST be "agent.doorbell_agent.doorbell.ring"
    /// (spec line 42).
    #[test]
    fn agent_event_type_prefixing() {
        assert_eq!(
            build_agent_event_type("doorbell_agent", "doorbell.ring"),
            "agent.doorbell_agent.doorbell.ring"
        );
    }

    #[test]
    fn agent_event_type_alarm_agent() {
        assert_eq!(
            build_agent_event_type("alarm_agent", "fire.detected"),
            "agent.alarm_agent.fire.detected"
        );
    }

    #[test]
    fn agent_event_type_is_valid() {
        let et = build_agent_event_type("doorbell_agent", "doorbell.ring");
        assert!(
            validate_event_type(&et).is_ok(),
            "built agent event type should pass validation: {et}"
        );
    }
}
