//! # Interruption Classification — Enforcement Logic
//!
//! Agent-submitted interruption class enforcement per
//! scene-events/spec.md §Requirement: Interruption Classification, lines 50-66.
//!
//! ## What this module adds (bead 3 of Epic 9)
//!
//! The five `InterruptionClass` variants and ordering are defined in
//! [`super::envelope`].  This module adds the **enforcement rules** that the
//! runtime gate applies to every event before it enters the delivery pipeline:
//!
//! 1. **Agent CRITICAL downgrade** — only the runtime may emit CRITICAL; any
//!    agent-declared CRITICAL is silently downgraded to HIGH (spec line 61).
//! 2. **Zone ceiling application** — already implemented in
//!    [`InterruptionClass::ceiling`]; re-exported here for visibility.
//!
//! ## Usage
//!
//! ```rust
//! use tze_hud_scene::events::interruption::{apply_agent_class, apply_zone_ceiling};
//! use tze_hud_scene::events::InterruptionClass;
//!
//! // Agent-requested CRITICAL is downgraded to HIGH.
//! assert_eq!(apply_agent_class(InterruptionClass::Critical), InterruptionClass::High);
//!
//! // Zone ceiling: agent HIGH on NORMAL-ceiling zone → effective NORMAL.
//! assert_eq!(
//!     apply_zone_ceiling(InterruptionClass::High, InterruptionClass::Normal),
//!     InterruptionClass::Normal,
//! );
//! ```

pub use super::envelope::InterruptionClass;

// ─── Agent CRITICAL downgrade ────────────────────────────────────────────────

/// Apply the agent-submitted interruption class gate.
///
/// **CRITICAL agent requests are downgraded to HIGH** because only the runtime
/// is authorised to emit CRITICAL-class events (spec line 61).
///
/// All other classes pass through unchanged.
///
/// # Spec
/// scene-events/spec.md line 61:
/// > WHEN an agent requests CRITICAL interruption class on an emitted event
/// > THEN the runtime MUST downgrade the effective class to HIGH,
/// > since only the runtime can emit CRITICAL events.
#[must_use]
pub fn apply_agent_class(agent_declared: InterruptionClass) -> InterruptionClass {
    if agent_declared == InterruptionClass::Critical {
        InterruptionClass::High
    } else {
        agent_declared
    }
}

// ─── Zone ceiling ─────────────────────────────────────────────────────────────

/// Apply the zone ceiling to an event's effective class.
///
/// The effective class is the **more restrictive** (higher numeric value,
/// lower urgency) of `agent_effective` and `zone_ceiling`.
///
/// This is a thin forward to [`InterruptionClass::ceiling`], exposed here as
/// a free function so it can be called without method syntax in pipeline code.
///
/// # Spec
/// scene-events/spec.md line 57:
/// > WHEN an agent declares HIGH on a zone with ceiling NORMAL
/// > THEN the effective class MUST be NORMAL.
#[must_use]
pub fn apply_zone_ceiling(
    agent_effective: InterruptionClass,
    zone_ceiling: InterruptionClass,
) -> InterruptionClass {
    agent_effective.ceiling(zone_ceiling)
}

// ─── Combined agent + zone gate ───────────────────────────────────────────────

/// Apply both the agent-CRITICAL downgrade and zone ceiling in one call.
///
/// Equivalent to `apply_zone_ceiling(apply_agent_class(agent_declared), zone_ceiling)`.
///
/// This is the function the event classification stage (Stage 2) should call
/// for every agent-sourced event.
///
/// Spec: scene-events/spec.md lines 57, 61.
#[must_use]
pub fn classify_agent_event(
    agent_declared: InterruptionClass,
    zone_ceiling: InterruptionClass,
) -> InterruptionClass {
    apply_zone_ceiling(apply_agent_class(agent_declared), zone_ceiling)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── apply_agent_class ─────────────────────────────────────────────────────

    /// WHEN an agent requests CRITICAL THEN it is downgraded to HIGH (spec line 61).
    #[test]
    fn agent_critical_downgraded_to_high() {
        assert_eq!(apply_agent_class(InterruptionClass::Critical), InterruptionClass::High);
    }

    /// HIGH, NORMAL, LOW, SILENT pass through unchanged.
    #[test]
    fn non_critical_classes_pass_through() {
        for cls in [
            InterruptionClass::High,
            InterruptionClass::Normal,
            InterruptionClass::Low,
            InterruptionClass::Silent,
        ] {
            assert_eq!(apply_agent_class(cls), cls, "class {cls:?} should pass unchanged");
        }
    }

    // ── apply_zone_ceiling ────────────────────────────────────────────────────

    /// WHEN agent declares HIGH on NORMAL-ceiling zone THEN effective = NORMAL (spec line 57).
    #[test]
    fn zone_ceiling_downgrades_high_to_normal() {
        assert_eq!(
            apply_zone_ceiling(InterruptionClass::High, InterruptionClass::Normal),
            InterruptionClass::Normal,
        );
    }

    /// WHEN agent declares LOW and zone ceiling is HIGH THEN effective = LOW (no upgrade).
    #[test]
    fn zone_ceiling_does_not_upgrade_class() {
        assert_eq!(
            apply_zone_ceiling(InterruptionClass::Low, InterruptionClass::High),
            InterruptionClass::Low,
        );
    }

    /// Same class as ceiling — no change.
    #[test]
    fn zone_ceiling_same_as_agent_class_unchanged() {
        assert_eq!(
            apply_zone_ceiling(InterruptionClass::Normal, InterruptionClass::Normal),
            InterruptionClass::Normal,
        );
    }

    // ── classify_agent_event ──────────────────────────────────────────────────

    /// CRITICAL agent request + CRITICAL zone ceiling → effective HIGH (downgrade first).
    #[test]
    fn classify_critical_agent_with_critical_ceiling() {
        // CRITICAL downgraded to HIGH, ceiling is CRITICAL (=0), HIGH(=1) > CRITICAL(=0),
        // so the more restrictive (higher numeric) wins → HIGH.
        let effective = classify_agent_event(InterruptionClass::Critical, InterruptionClass::Critical);
        assert_eq!(effective, InterruptionClass::High);
    }

    /// HIGH agent + NORMAL ceiling → NORMAL (ceiling more restrictive).
    #[test]
    fn classify_high_agent_normal_ceiling() {
        assert_eq!(
            classify_agent_event(InterruptionClass::High, InterruptionClass::Normal),
            InterruptionClass::Normal,
        );
    }

    /// NORMAL agent + CRITICAL ceiling → NORMAL (agent already more restrictive).
    #[test]
    fn classify_normal_agent_critical_ceiling() {
        // agent NORMAL=2, ceiling CRITICAL=0; max(2,0)=2=NORMAL.
        assert_eq!(
            classify_agent_event(InterruptionClass::Normal, InterruptionClass::Critical),
            InterruptionClass::Normal,
        );
    }

    /// SILENT agent → always SILENT (lowest urgency, no ceiling can increase urgency).
    #[test]
    fn classify_silent_agent_any_ceiling() {
        for ceiling in [
            InterruptionClass::Critical,
            InterruptionClass::High,
            InterruptionClass::Normal,
            InterruptionClass::Low,
            InterruptionClass::Silent,
        ] {
            assert_eq!(
                classify_agent_event(InterruptionClass::Silent, ceiling),
                InterruptionClass::Silent,
                "SILENT should always produce SILENT regardless of ceiling {ceiling:?}",
            );
        }
    }
}
