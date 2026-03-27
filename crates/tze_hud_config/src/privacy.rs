//! Privacy configuration validation — rig-mop4.
//!
//! Implements spec `configuration/spec.md` requirements:
//!
//! - **Privacy Configuration Defaults** (lines 215-225, v1-mandatory)
//!   `default_classification`, `default_viewer_class`, `viewer_id_method`,
//!   `redaction_style`, `multi_viewer_policy` with validation.
//! - **Quiet Hours Configuration** (lines 228-239, v1-mandatory)
//!   `[privacy.quiet_hours]` with `pass_through_class` using canonical
//!   `InterruptionClass` enum names (CRITICAL, HIGH, NORMAL, LOW, SILENT).
//! - **Redaction Style Ownership** (lines 254-261, v1-mandatory)
//!   `redaction_style` belongs exclusively to `[privacy]`; absent from `[chrome]`.
//!
//! ## Interruption Class Semantics (quiet hours)
//!
//! `pass_through_class` specifies the minimum class that passes through
//! immediately.  CRITICAL always passes regardless.  The threshold works as:
//!
//! - CRITICAL threshold → only CRITICAL passes; all others queued/discarded
//! - HIGH threshold     → CRITICAL and HIGH pass; NORMAL queued; LOW discarded
//! - NORMAL threshold   → CRITICAL, HIGH, NORMAL pass; LOW discarded
//! - LOW threshold      → CRITICAL, HIGH, NORMAL, LOW pass; SILENT unaffected
//! - SILENT threshold   → all pass (effectively no filtering)
//!
//! SILENT is never queued (invisible by definition).
//!
//! ## v1 Note
//!
//! v1-reserved: Viewer Identification Pipeline (`[[privacy.viewer_detectors]]`) is
//! post-v1.  v1 uses `viewer_id_method` string only.

use tze_hud_scene::config::{ConfigError, ConfigErrorCode};

use crate::raw::RawPrivacy;

// ─── Valid value sets ──────────────────────────────────────────────────────────

/// Valid `default_classification` values (from spec §Privacy Configuration Defaults).
pub const VALID_CLASSIFICATIONS: &[&str] = &["public", "household", "private", "sensitive"];

/// Valid `default_viewer_class` values (from spec §Privacy Configuration Defaults).
pub const VALID_VIEWER_CLASSES: &[&str] = &[
    "owner",
    "household_member",
    "known_guest",
    "unknown",
    "nobody",
];

/// Valid `redaction_style` values (from spec §Privacy Configuration Defaults).
pub const VALID_REDACTION_STYLES: &[&str] = &["pattern", "blank"];

/// Valid `multi_viewer_policy` values (from spec §Privacy Configuration Defaults).
pub const VALID_MULTI_VIEWER_POLICIES: &[&str] = &["most_restrictive", "least_restrictive"];

/// Valid canonical `InterruptionClass` enum names for `pass_through_class`
/// (from spec §Quiet Hours Configuration, RFC 0010 §3.1).
///
/// Note: doctrine names like "urgent" and "gentle" are NOT valid here.
pub const VALID_INTERRUPTION_CLASSES: &[&str] = &["CRITICAL", "HIGH", "NORMAL", "LOW", "SILENT"];

/// Maps doctrine names to their canonical interruption class equivalents.
/// Used to produce helpful hints when a doctrine name is provided.
///
/// Source: RFC 0010 §3.1 — "Urgent" renamed to HIGH for consistency with RFC 0009
/// severity levels; "gentle" renamed to LOW.
const DOCTRINE_TO_CANONICAL: &[(&str, &str)] = &[
    ("urgent", "HIGH"), // RFC 0010 §3.1: "Urgent" → HIGH
    ("high", "HIGH"),
    ("normal", "NORMAL"),
    ("low", "LOW"),
    ("gentle", "LOW"), // RFC 0010 §3.1: "Gentle" → LOW
    ("silent", "SILENT"),
    ("critical", "CRITICAL"),
];

// ─── Validation ───────────────────────────────────────────────────────────────

/// Validate the `[privacy]` section, appending any errors.
///
/// Called from `TzeHudConfig::validate()`.
pub fn validate_privacy(privacy: &RawPrivacy, errors: &mut Vec<ConfigError>) {
    // ── default_classification ────────────────────────────────────────────────
    if let Some(cls) = &privacy.default_classification
        && !VALID_CLASSIFICATIONS.contains(&cls.as_str())
    {
        errors.push(ConfigError {
            code: ConfigErrorCode::UnknownClassification,
            field_path: "privacy.default_classification".into(),
            expected: format!("one of: {}", VALID_CLASSIFICATIONS.join(", ")),
            got: format!("{cls:?}"),
            hint: format!(
                "unknown classification {:?}; valid values: {}",
                cls,
                VALID_CLASSIFICATIONS.join(", ")
            ),
        });
    }

    // ── default_viewer_class ──────────────────────────────────────────────────
    if let Some(vc) = &privacy.default_viewer_class
        && !VALID_VIEWER_CLASSES.contains(&vc.as_str())
    {
        errors.push(ConfigError {
            code: ConfigErrorCode::UnknownViewerClass,
            field_path: "privacy.default_viewer_class".into(),
            expected: format!("one of: {}", VALID_VIEWER_CLASSES.join(", ")),
            got: format!("{vc:?}"),
            hint: format!(
                "unknown viewer class {:?}; valid values: {}",
                vc,
                VALID_VIEWER_CLASSES.join(", ")
            ),
        });
    }

    // ── redaction_style ───────────────────────────────────────────────────────
    if let Some(rs) = &privacy.redaction_style
        && !VALID_REDACTION_STYLES.contains(&rs.as_str())
    {
        errors.push(ConfigError {
            code: ConfigErrorCode::Other("CONFIG_UNKNOWN_REDACTION_STYLE".into()),
            field_path: "privacy.redaction_style".into(),
            expected: format!("one of: {}", VALID_REDACTION_STYLES.join(", ")),
            got: format!("{rs:?}"),
            hint: format!(
                "unknown redaction_style {:?}; valid values: {}",
                rs,
                VALID_REDACTION_STYLES.join(", ")
            ),
        });
    }

    // ── multi_viewer_policy ───────────────────────────────────────────────────
    if let Some(mvp) = &privacy.multi_viewer_policy
        && !VALID_MULTI_VIEWER_POLICIES.contains(&mvp.as_str())
    {
        errors.push(ConfigError {
            code: ConfigErrorCode::Other("CONFIG_UNKNOWN_MULTI_VIEWER_POLICY".into()),
            field_path: "privacy.multi_viewer_policy".into(),
            expected: format!("one of: {}", VALID_MULTI_VIEWER_POLICIES.join(", ")),
            got: format!("{mvp:?}"),
            hint: format!(
                "unknown multi_viewer_policy {:?}; valid values: {}",
                mvp,
                VALID_MULTI_VIEWER_POLICIES.join(", ")
            ),
        });
    }

    // ── quiet_hours ───────────────────────────────────────────────────────────
    if let Some(qh) = &privacy.quiet_hours {
        if let Some(ptc) = &qh.pass_through_class
            && !VALID_INTERRUPTION_CLASSES.contains(&ptc.as_str())
        {
            // Produce a helpful hint if the user used a doctrine name.
            let hint = find_doctrine_hint(ptc);
            errors.push(ConfigError {
                code: ConfigErrorCode::UnknownInterruptionClass,
                field_path: "privacy.quiet_hours.pass_through_class".into(),
                expected: format!(
                    "canonical InterruptionClass enum name: one of {}",
                    VALID_INTERRUPTION_CLASSES.join(", ")
                ),
                got: format!("{ptc:?}"),
                hint,
            });
        }

        // Validate quiet_mode_display values.
        const VALID_QUIET_MODE_DISPLAY: &[&str] = &["dim", "clock_only", "off"];
        if let Some(qmd) = &qh.quiet_mode_display
            && !VALID_QUIET_MODE_DISPLAY.contains(&qmd.as_str())
        {
            errors.push(ConfigError {
                code: ConfigErrorCode::Other("CONFIG_UNKNOWN_QUIET_MODE_DISPLAY".into()),
                field_path: "privacy.quiet_hours.quiet_mode_display".into(),
                expected: format!("one of: {}", VALID_QUIET_MODE_DISPLAY.join(", ")),
                got: format!("{qmd:?}"),
                hint: format!(
                    "unknown quiet_mode_display {:?}; valid values: {}",
                    qmd,
                    VALID_QUIET_MODE_DISPLAY.join(", ")
                ),
            });
        }
    }
}

/// Try to map a value to a canonical interruption class via doctrine name lookup.
/// Returns a helpful hint string.
fn find_doctrine_hint(value: &str) -> String {
    // Try case-insensitive match against doctrine names.
    let lower = value.to_lowercase();
    for (doctrine, canonical) in DOCTRINE_TO_CANONICAL {
        if lower == *doctrine {
            return format!(
                "use the canonical InterruptionClass name {canonical:?} instead of doctrine name {value:?}"
            );
        }
    }
    format!(
        "use a canonical InterruptionClass enum name: {}",
        VALID_INTERRUPTION_CLASSES.join(", ")
    )
}

// ─── Quiet-hours interruption class semantics ─────────────────────────────────

/// Result of classifying a notification given the current quiet-hours setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuietHoursAction {
    /// Notification passes through immediately.
    PassThrough,
    /// Notification is queued until quiet hours end.
    Queue,
    /// Notification is discarded (too stale to be useful).
    Discard,
    /// Notification is unaffected by quiet hours (e.g., SILENT — invisible).
    Unaffected,
}

/// Compute the quiet-hours action for a given interruption class.
///
/// `pass_through_class` is the configured minimum class that passes through.
/// It must already be validated (i.e., a value from `VALID_INTERRUPTION_CLASSES`).
///
/// Interruption class ordering (highest to lowest):
/// CRITICAL > HIGH > NORMAL > LOW > SILENT
///
/// CRITICAL always passes through.
/// SILENT is always unaffected (invisible by definition; never queued or discarded).
/// For other classes: if their rank >= pass_through rank → PassThrough; else Queue/Discard.
///
/// LOW class is discarded when below the pass-through threshold (too stale).
/// NORMAL class is queued when below the pass-through threshold.
pub fn quiet_hours_action(interruption_class: &str, pass_through_class: &str) -> QuietHoursAction {
    // CRITICAL always passes regardless of threshold.
    if interruption_class == "CRITICAL" {
        return QuietHoursAction::PassThrough;
    }
    // SILENT is always unaffected (invisible by definition).
    if interruption_class == "SILENT" {
        return QuietHoursAction::Unaffected;
    }

    let rank = interruption_rank(interruption_class);
    let threshold_rank = interruption_rank(pass_through_class);

    if rank >= threshold_rank {
        QuietHoursAction::PassThrough
    } else {
        // Below threshold: LOW is discarded, others are queued.
        if interruption_class == "LOW" {
            QuietHoursAction::Discard
        } else {
            QuietHoursAction::Queue
        }
    }
}

/// Numeric rank for interruption class ordering (higher = more urgent).
///
/// CRITICAL=4, HIGH=3, NORMAL=2, LOW=1, SILENT=0
fn interruption_rank(class: &str) -> u8 {
    match class {
        "CRITICAL" => 4,
        "HIGH" => 3,
        "NORMAL" => 2,
        "LOW" => 1,
        _ => 0,
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{RawPrivacy, RawQuietHours};

    // ── Privacy defaults ──────────────────────────────────────────────────────

    #[test]
    fn test_unknown_classification_produces_error() {
        let privacy = RawPrivacy {
            default_classification: Some("top_secret".into()),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_privacy(&privacy, &mut errors);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::UnknownClassification)),
            "top_secret classification should produce CONFIG_UNKNOWN_CLASSIFICATION"
        );
    }

    #[test]
    fn test_valid_classifications_accepted() {
        for cls in VALID_CLASSIFICATIONS {
            let privacy = RawPrivacy {
                default_classification: Some((*cls).into()),
                ..Default::default()
            };
            let mut errors = Vec::new();
            validate_privacy(&privacy, &mut errors);
            let cls_errors: Vec<_> = errors
                .iter()
                .filter(|e| matches!(e.code, ConfigErrorCode::UnknownClassification))
                .collect();
            assert!(
                cls_errors.is_empty(),
                "classification {cls:?} should be valid"
            );
        }
    }

    #[test]
    fn test_unknown_viewer_class_produces_error() {
        let privacy = RawPrivacy {
            default_viewer_class: Some("admin".into()),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_privacy(&privacy, &mut errors);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.code, ConfigErrorCode::UnknownViewerClass)),
            "admin viewer class should produce CONFIG_UNKNOWN_VIEWER_CLASS"
        );
    }

    #[test]
    fn test_valid_viewer_classes_accepted() {
        for vc in VALID_VIEWER_CLASSES {
            let privacy = RawPrivacy {
                default_viewer_class: Some((*vc).into()),
                ..Default::default()
            };
            let mut errors = Vec::new();
            validate_privacy(&privacy, &mut errors);
            let vc_errors: Vec<_> = errors
                .iter()
                .filter(|e| matches!(e.code, ConfigErrorCode::UnknownViewerClass))
                .collect();
            assert!(vc_errors.is_empty(), "viewer class {vc:?} should be valid");
        }
    }

    // ── Quiet hours ───────────────────────────────────────────────────────────

    #[test]
    fn test_invalid_pass_through_class_doctrine_name_produces_error_with_hint() {
        // Spec scenario: pass_through_class = "urgent" → CONFIG_UNKNOWN_INTERRUPTION_CLASS
        // with hint suggesting "HIGH".
        let privacy = RawPrivacy {
            quiet_hours: Some(RawQuietHours {
                enabled: true,
                pass_through_class: Some("urgent".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_privacy(&privacy, &mut errors);
        let ptc_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e.code, ConfigErrorCode::UnknownInterruptionClass))
            .collect();
        assert!(
            !ptc_errors.is_empty(),
            "doctrine name 'urgent' should produce CONFIG_UNKNOWN_INTERRUPTION_CLASS"
        );
        // Per spec line 239 and RFC 0010 §3.1: "urgent" maps to canonical "HIGH".
        // The hint must suggest "HIGH" specifically.
        let hint = &ptc_errors[0].hint;
        assert!(
            hint.contains("HIGH"),
            "hint for 'urgent' must suggest canonical name 'HIGH' (RFC 0010 §3.1), got: {hint:?}"
        );
    }

    #[test]
    fn test_valid_pass_through_classes_accepted() {
        for ptc in VALID_INTERRUPTION_CLASSES {
            let privacy = RawPrivacy {
                quiet_hours: Some(RawQuietHours {
                    pass_through_class: Some((*ptc).into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let mut errors = Vec::new();
            validate_privacy(&privacy, &mut errors);
            let ptc_errors: Vec<_> = errors
                .iter()
                .filter(|e| matches!(e.code, ConfigErrorCode::UnknownInterruptionClass))
                .collect();
            assert!(
                ptc_errors.is_empty(),
                "pass_through_class {ptc:?} should be valid"
            );
        }
    }

    // ── Quiet hours action semantics ──────────────────────────────────────────

    #[test]
    fn test_quiet_hours_high_pass_through_class() {
        // Spec scenario: pass_through_class = "HIGH" → CRITICAL and HIGH pass; NORMAL queued;
        // LOW discarded; SILENT unaffected.
        assert_eq!(
            quiet_hours_action("CRITICAL", "HIGH"),
            QuietHoursAction::PassThrough
        );
        assert_eq!(
            quiet_hours_action("HIGH", "HIGH"),
            QuietHoursAction::PassThrough
        );
        assert_eq!(
            quiet_hours_action("NORMAL", "HIGH"),
            QuietHoursAction::Queue
        );
        assert_eq!(quiet_hours_action("LOW", "HIGH"), QuietHoursAction::Discard);
        assert_eq!(
            quiet_hours_action("SILENT", "HIGH"),
            QuietHoursAction::Unaffected
        );
    }

    #[test]
    fn test_quiet_hours_critical_pass_through_class() {
        // Only CRITICAL passes; SILENT unaffected.
        assert_eq!(
            quiet_hours_action("CRITICAL", "CRITICAL"),
            QuietHoursAction::PassThrough
        );
        assert_eq!(
            quiet_hours_action("HIGH", "CRITICAL"),
            QuietHoursAction::Queue
        );
        assert_eq!(
            quiet_hours_action("NORMAL", "CRITICAL"),
            QuietHoursAction::Queue
        );
        assert_eq!(
            quiet_hours_action("LOW", "CRITICAL"),
            QuietHoursAction::Discard
        );
        assert_eq!(
            quiet_hours_action("SILENT", "CRITICAL"),
            QuietHoursAction::Unaffected
        );
    }

    #[test]
    fn test_quiet_hours_normal_pass_through_class() {
        assert_eq!(
            quiet_hours_action("CRITICAL", "NORMAL"),
            QuietHoursAction::PassThrough
        );
        assert_eq!(
            quiet_hours_action("HIGH", "NORMAL"),
            QuietHoursAction::PassThrough
        );
        assert_eq!(
            quiet_hours_action("NORMAL", "NORMAL"),
            QuietHoursAction::PassThrough
        );
        assert_eq!(
            quiet_hours_action("LOW", "NORMAL"),
            QuietHoursAction::Discard
        );
        assert_eq!(
            quiet_hours_action("SILENT", "NORMAL"),
            QuietHoursAction::Unaffected
        );
    }

    #[test]
    fn test_privacy_no_section_no_errors() {
        // When [privacy] absent, no errors should be emitted.
        // (This is tested via the loader — here we test the struct directly.)
        let privacy = RawPrivacy::default();
        let mut errors = Vec::new();
        validate_privacy(&privacy, &mut errors);
        assert!(
            errors.is_empty(),
            "absent privacy fields should not produce errors"
        );
    }
}
