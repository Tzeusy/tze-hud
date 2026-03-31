//! Zone readability validator — hud-sc0a.6.
//!
//! Validates that a zone's effective `RenderingPolicy` satisfies the readability
//! requirements imposed by its component type.
//!
//! Source: `component-shape-language/spec.md §Requirement: Zone Readability Enforcement`.
//!
//! ## Dev mode
//!
//! In development builds, readability violations are downgraded from hard errors to
//! WARN-level log messages so that profile authors can iterate without restarting
//! the full runtime.
//!
//! Dev mode is active when:
//! - `TZE_HUD_DEV=1` is set in the environment, OR
//! - the resolved display profile name is `"headless"`.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tze_hud_config::readability::{check_zone_readability, is_dev_mode};
//! use tze_hud_config::component_types::ReadabilityTechnique;
//! use tze_hud_scene::types::RenderingPolicy;
//!
//! let policy = RenderingPolicy { .. };
//! match check_zone_readability(&policy, ReadabilityTechnique::DualLayer) {
//!     Ok(()) => { /* readability satisfied */ }
//!     Err(v) => {
//!         if is_dev_mode(None) {
//!             tracing::warn!("PROFILE_READABILITY_VIOLATION (dev mode): {v}");
//!         } else {
//!             return Err(v.into());
//!         }
//!     }
//! }
//! ```

use tze_hud_scene::types::RenderingPolicy;

use crate::component_types::ReadabilityTechnique;

// ─── ReadabilityViolation ─────────────────────────────────────────────────────

/// Describes a readability check failure for a zone's effective RenderingPolicy.
///
/// The error code on the wire is `PROFILE_READABILITY_VIOLATION`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadabilityViolation {
    /// The readability technique that was checked.
    pub technique: ReadabilityTechnique,
    /// The specific check that failed (e.g. `"outline_width must be >= 1.0, got None"`).
    pub failing_check: String,
    /// Snapshot of the relevant policy fields for diagnostic purposes.
    pub policy_snapshot: PolicySnapshot,
}

impl std::fmt::Display for ReadabilityViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PROFILE_READABILITY_VIOLATION: {:?}: {}",
            self.technique, self.failing_check
        )
    }
}

/// Relevant field values snapshotted at the time of a readability check.
#[derive(Clone, Debug, PartialEq)]
pub struct PolicySnapshot {
    pub backdrop: Option<String>,
    pub backdrop_opacity: Option<f32>,
    pub outline_color: Option<String>,
    pub outline_width: Option<f32>,
}

impl PolicySnapshot {
    fn from_policy(policy: &RenderingPolicy) -> Self {
        PolicySnapshot {
            backdrop: policy.backdrop.map(|c| format!("{c:?}")),
            backdrop_opacity: policy.backdrop_opacity,
            outline_color: policy.outline_color.map(|c| format!("{c:?}")),
            outline_width: policy.outline_width,
        }
    }
}

// ─── Dev mode detection ───────────────────────────────────────────────────────

/// Returns `true` if the runtime is in development mode.
///
/// Dev mode is active when `TZE_HUD_DEV=1` is in the environment OR when the
/// resolved display profile name is `"headless"`.
///
/// # Arguments
///
/// - `profile_name`: the resolved display profile name (e.g. `"full-display"`,
///   `"headless"`). Pass `None` to rely solely on the environment variable.
pub fn is_dev_mode(profile_name: Option<&str>) -> bool {
    let env_dev = std::env::var("TZE_HUD_DEV")
        .map(|v| v == "1")
        .unwrap_or(false);
    let headless = profile_name.map(|p| p == "headless").unwrap_or(false);
    env_dev || headless
}

// ─── Zone readability validator ───────────────────────────────────────────────

/// Validate a zone's effective `RenderingPolicy` against a readability technique.
///
/// Returns `Ok(())` if the policy satisfies the technique's requirements.
/// Returns `Err(ReadabilityViolation)` with the failing check description and
/// actual field values on failure.
///
/// Callers should check [`is_dev_mode`] to decide whether to hard-error or WARN.
///
/// ## Rules
///
/// | Technique | Checks |
/// |---|---|
/// | `DualLayer` | `backdrop` is `Some`, `backdrop_opacity >= 0.3`, `outline_color` is `Some`, `outline_width >= 1.0` |
/// | `OpaqueBackdrop` | `backdrop` is `Some`, `backdrop_opacity >= 0.8` |
/// | `None` | No validation; always returns `Ok(())` |
///
/// Source: `component-shape-language/spec.md §Requirement: Zone Readability Enforcement`.
pub fn check_zone_readability(
    policy: &RenderingPolicy,
    technique: ReadabilityTechnique,
) -> Result<(), ReadabilityViolation> {
    match technique {
        ReadabilityTechnique::None => Ok(()),

        ReadabilityTechnique::DualLayer => check_dual_layer(policy),

        ReadabilityTechnique::OpaqueBackdrop => check_opaque_backdrop(policy),
    }
}

// ── DualLayer ─────────────────────────────────────────────────────────────────

fn check_dual_layer(policy: &RenderingPolicy) -> Result<(), ReadabilityViolation> {
    let snapshot = PolicySnapshot::from_policy(policy);

    // backdrop must be Some.
    if policy.backdrop.is_none() {
        return Err(ReadabilityViolation {
            technique: ReadabilityTechnique::DualLayer,
            failing_check: "DualLayer: backdrop must be Some, got None".to_string(),
            policy_snapshot: snapshot,
        });
    }

    // backdrop_opacity must be Some and >= 0.3.
    match policy.backdrop_opacity {
        None => {
            return Err(ReadabilityViolation {
                technique: ReadabilityTechnique::DualLayer,
                failing_check: "DualLayer: backdrop_opacity must be >= 0.3, got None".to_string(),
                policy_snapshot: snapshot,
            });
        }
        Some(v) if v < 0.3 => {
            return Err(ReadabilityViolation {
                technique: ReadabilityTechnique::DualLayer,
                failing_check: format!("DualLayer: backdrop_opacity must be >= 0.3, got {v}"),
                policy_snapshot: snapshot,
            });
        }
        _ => {}
    }

    // outline_color must be Some.
    if policy.outline_color.is_none() {
        return Err(ReadabilityViolation {
            technique: ReadabilityTechnique::DualLayer,
            failing_check: "DualLayer: outline_color must be Some, got None".to_string(),
            policy_snapshot: snapshot,
        });
    }

    // outline_width must be Some and >= 1.0.
    match policy.outline_width {
        None => {
            return Err(ReadabilityViolation {
                technique: ReadabilityTechnique::DualLayer,
                failing_check: "DualLayer: outline_width must be >= 1.0, got None".to_string(),
                policy_snapshot: snapshot,
            });
        }
        Some(v) if v < 1.0 => {
            return Err(ReadabilityViolation {
                technique: ReadabilityTechnique::DualLayer,
                failing_check: format!("DualLayer: outline_width must be >= 1.0, got {v}"),
                policy_snapshot: snapshot,
            });
        }
        _ => {}
    }

    Ok(())
}

// ── OpaqueBackdrop ────────────────────────────────────────────────────────────

fn check_opaque_backdrop(policy: &RenderingPolicy) -> Result<(), ReadabilityViolation> {
    let snapshot = PolicySnapshot::from_policy(policy);

    // backdrop must be Some.
    if policy.backdrop.is_none() {
        return Err(ReadabilityViolation {
            technique: ReadabilityTechnique::OpaqueBackdrop,
            failing_check: "OpaqueBackdrop: backdrop must be Some, got None".to_string(),
            policy_snapshot: snapshot,
        });
    }

    // backdrop_opacity must be Some and >= 0.8.
    match policy.backdrop_opacity {
        None => {
            return Err(ReadabilityViolation {
                technique: ReadabilityTechnique::OpaqueBackdrop,
                failing_check: "OpaqueBackdrop: backdrop_opacity must be >= 0.8, got None"
                    .to_string(),
                policy_snapshot: snapshot,
            });
        }
        Some(v) if v < 0.8 => {
            return Err(ReadabilityViolation {
                technique: ReadabilityTechnique::OpaqueBackdrop,
                failing_check: format!("OpaqueBackdrop: backdrop_opacity must be >= 0.8, got {v}"),
                policy_snapshot: snapshot,
            });
        }
        _ => {}
    }

    Ok(())
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tze_hud_scene::types::{RenderingPolicy, Rgba};

    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// A fully valid DualLayer policy.
    fn dual_layer_policy() -> RenderingPolicy {
        RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.6),
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(2.0),
            ..RenderingPolicy::default()
        }
    }

    /// A fully valid OpaqueBackdrop policy.
    fn opaque_backdrop_policy() -> RenderingPolicy {
        RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.9),
            ..RenderingPolicy::default()
        }
    }

    // ── DualLayer pass ────────────────────────────────────────────────────────

    /// Scenario: Subtitle DualLayer passes.
    #[test]
    fn dual_layer_valid_policy_passes() {
        let policy = dual_layer_policy();
        let result = check_zone_readability(&policy, ReadabilityTechnique::DualLayer);
        assert!(
            result.is_ok(),
            "valid DualLayer policy should pass: {result:?}"
        );
    }

    #[test]
    fn dual_layer_at_minimum_thresholds_passes() {
        let policy = RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.3), // exactly 0.3 is OK
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(1.0), // exactly 1.0 is OK
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::DualLayer);
        assert!(
            result.is_ok(),
            "DualLayer at minimum thresholds should pass: {result:?}"
        );
    }

    // ── DualLayer fail: no backdrop ───────────────────────────────────────────

    #[test]
    fn dual_layer_no_backdrop_fails() {
        let policy = RenderingPolicy {
            backdrop: None,
            backdrop_opacity: Some(0.6),
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(2.0),
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::DualLayer);
        assert!(result.is_err(), "DualLayer with no backdrop must fail");
        let v = result.unwrap_err();
        assert!(
            v.failing_check.contains("backdrop"),
            "failing_check must mention backdrop: {}",
            v.failing_check
        );
    }

    // ── DualLayer fail: no outline_width ─────────────────────────────────────

    /// Scenario: Subtitle DualLayer fails — no outline.
    #[test]
    fn dual_layer_no_outline_width_fails() {
        let policy = RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.6),
            outline_color: Some(Rgba::BLACK),
            outline_width: None, // ← failing field
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::DualLayer);
        assert!(result.is_err(), "DualLayer with no outline_width must fail");
        let v = result.unwrap_err();
        assert!(
            v.failing_check.contains("outline_width"),
            "failing_check must mention outline_width: {}",
            v.failing_check
        );
        assert!(
            v.failing_check.contains("1.0"),
            "failing_check must mention the required threshold: {}",
            v.failing_check
        );
    }

    #[test]
    fn dual_layer_outline_width_below_minimum_fails() {
        let policy = RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.6),
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(0.5), // below 1.0
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::DualLayer);
        assert!(
            result.is_err(),
            "DualLayer with outline_width < 1.0 must fail"
        );
        let v = result.unwrap_err();
        assert!(
            v.failing_check.contains("outline_width"),
            "{}",
            v.failing_check
        );
        assert!(v.failing_check.contains("0.5"), "{}", v.failing_check);
    }

    #[test]
    fn dual_layer_no_outline_color_fails() {
        let policy = RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.6),
            outline_color: None, // ← failing field
            outline_width: Some(2.0),
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::DualLayer);
        assert!(result.is_err(), "DualLayer with no outline_color must fail");
        let v = result.unwrap_err();
        assert!(
            v.failing_check.contains("outline_color"),
            "{}",
            v.failing_check
        );
    }

    // ── DualLayer fail: low backdrop_opacity ─────────────────────────────────

    #[test]
    fn dual_layer_low_backdrop_opacity_fails() {
        let policy = RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.1), // below 0.3
            outline_color: Some(Rgba::BLACK),
            outline_width: Some(2.0),
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::DualLayer);
        assert!(
            result.is_err(),
            "DualLayer with backdrop_opacity < 0.3 must fail"
        );
        let v = result.unwrap_err();
        assert!(
            v.failing_check.contains("backdrop_opacity"),
            "{}",
            v.failing_check
        );
    }

    // ── OpaqueBackdrop pass ───────────────────────────────────────────────────

    #[test]
    fn opaque_backdrop_valid_policy_passes() {
        let policy = opaque_backdrop_policy();
        let result = check_zone_readability(&policy, ReadabilityTechnique::OpaqueBackdrop);
        assert!(
            result.is_ok(),
            "valid OpaqueBackdrop policy should pass: {result:?}"
        );
    }

    #[test]
    fn opaque_backdrop_at_minimum_threshold_passes() {
        let policy = RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.8), // exactly 0.8 is OK
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::OpaqueBackdrop);
        assert!(
            result.is_ok(),
            "OpaqueBackdrop at exactly 0.8 should pass: {result:?}"
        );
    }

    // ── OpaqueBackdrop fail: low opacity ──────────────────────────────────────

    /// Scenario: Notification OpaqueBackdrop fails — low opacity.
    #[test]
    fn opaque_backdrop_low_opacity_fails() {
        let policy = RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: Some(0.5), // below 0.8
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::OpaqueBackdrop);
        assert!(result.is_err(), "OpaqueBackdrop with opacity 0.5 must fail");
        let v = result.unwrap_err();
        assert!(
            v.failing_check.contains("0.8"),
            "failing_check must mention threshold 0.8: {}",
            v.failing_check
        );
        assert!(
            v.failing_check.contains("0.5"),
            "failing_check must mention actual value 0.5: {}",
            v.failing_check
        );
    }

    #[test]
    fn opaque_backdrop_no_backdrop_fails() {
        let policy = RenderingPolicy {
            backdrop: None,
            backdrop_opacity: Some(0.9),
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::OpaqueBackdrop);
        assert!(result.is_err(), "OpaqueBackdrop with no backdrop must fail");
    }

    #[test]
    fn opaque_backdrop_no_opacity_fails() {
        let policy = RenderingPolicy {
            backdrop: Some(Rgba::BLACK),
            backdrop_opacity: None,
            ..RenderingPolicy::default()
        };
        let result = check_zone_readability(&policy, ReadabilityTechnique::OpaqueBackdrop);
        assert!(
            result.is_err(),
            "OpaqueBackdrop with no backdrop_opacity must fail"
        );
        let v = result.unwrap_err();
        assert!(v.failing_check.contains("None"), "{}", v.failing_check);
    }

    // ── None technique always passes ──────────────────────────────────────────

    #[test]
    fn none_technique_always_passes() {
        // ambient-background, pip — no validation.
        let empty_policy = RenderingPolicy::default();
        let result = check_zone_readability(&empty_policy, ReadabilityTechnique::None);
        assert!(result.is_ok(), "None technique must always pass");
    }

    // ── Violation fields ─────────────────────────────────────────────────────

    #[test]
    fn violation_includes_technique() {
        let policy = RenderingPolicy::default();
        let v = check_zone_readability(&policy, ReadabilityTechnique::DualLayer).unwrap_err();
        assert_eq!(v.technique, ReadabilityTechnique::DualLayer);
    }

    #[test]
    fn violation_display_includes_error_code() {
        let policy = RenderingPolicy::default();
        let v = check_zone_readability(&policy, ReadabilityTechnique::DualLayer).unwrap_err();
        let display = v.to_string();
        assert!(
            display.contains("PROFILE_READABILITY_VIOLATION"),
            "Display must include wire code: {display}"
        );
    }

    // ── Dev mode detection ────────────────────────────────────────────────────

    #[test]
    fn is_dev_mode_headless_profile() {
        assert!(
            is_dev_mode(Some("headless")),
            "headless profile should trigger dev mode"
        );
        assert!(
            !is_dev_mode(Some("full-display")),
            "full-display should not trigger dev mode"
        );
        assert!(
            !is_dev_mode(None),
            "no profile name and no env var should not trigger dev mode"
        );
    }

    // NOTE: We cannot reliably test TZE_HUD_DEV=1 in a unit test without
    // environment mutation (which is unsound in parallel tests). The env-var
    // path is covered by integration/smoke tests that set the variable before
    // launching the runtime. The is_dev_mode function is tested here for the
    // profile_name path only.
}
