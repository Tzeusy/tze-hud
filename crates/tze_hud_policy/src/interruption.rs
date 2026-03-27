//! # Interruption Classification
//!
//! Defines the `InterruptionClass` enum (RFC 0010 §3.1) and classification
//! helpers used by Level 4 (Attention) policy evaluation.
//!
//! ## Spec reference
//! - RFC 0010 §3.1 — canonical enum names and semantics
//! - policy-arbitration/spec.md §11.5 — Level 4 Attention Management
//!
//! ## Notes on Ordering
//!
//! The enum is ordered by urgency — lower discriminant = higher urgency:
//!
//! | Class    | Discriminant | Urgency  |
//! |----------|-------------|---------|
//! | Critical | 0           | Highest |
//! | High     | 1           |         |
//! | Normal   | 2           |         |
//! | Low      | 3           |         |
//! | Silent   | 4           | Lowest  |
//!
//! This ordering enables `a < b` to mean "a is more urgent than b", matching
//! the spec's `pass_through_class` comparison semantics.
//!
//! ## Canonical Definition
//!
//! The `InterruptionClass` enum is defined in `types.rs` and re-exported here
//! as the primary public API entry point for this module. This module adds
//! classification helpers as an `impl` block extension.
//!
//! **Note**: RFC 0009 §11.5 uses older names (Gentle/Normal/Urgent) and older
//! defaults (6/12). RFC 0010 §3.1 is authoritative for both enum names and
//! default budgets.

// Re-export the canonical type from types.rs.
pub use crate::types::InterruptionClass;

/// Extension helpers for `InterruptionClass`.
///
/// These functions encode spec rules that are shared across quiet-hours
/// evaluation, budget accounting, and CRITICAL enforcement checks.
impl InterruptionClass {
    /// Returns `true` if this class is runtime-only (agents must not emit it).
    ///
    /// Currently only `Critical` is runtime-only (spec §11.5).
    pub fn is_runtime_only(self) -> bool {
        matches!(self, InterruptionClass::Critical)
    }

    /// Returns `true` if this class bypasses quiet hours unconditionally.
    ///
    /// - `Critical` always bypasses (spec line 160).
    /// - `Silent` always passes (it never interrupts; filtering is irrelevant).
    /// - `High` bypasses only when it meets the `pass_through_class` threshold —
    ///   that check is performed by the attention evaluator, not here.
    pub fn unconditionally_bypasses_quiet_hours(self) -> bool {
        matches!(
            self,
            InterruptionClass::Critical | InterruptionClass::Silent
        )
    }

    /// Returns `true` if this class is counted against the attention budget.
    ///
    /// `Critical` and `Silent` are budget-free (spec §11.5).
    /// `High`, `Normal`, and `Low` consume budget when they pass.
    pub fn counts_against_budget(self) -> bool {
        matches!(
            self,
            InterruptionClass::High | InterruptionClass::Normal | InterruptionClass::Low
        )
    }

    /// Returns `true` if this class bypasses the attention budget unconditionally.
    ///
    /// This is the exact complement of `counts_against_budget()`.
    pub fn bypasses_budget(self) -> bool {
        !self.counts_against_budget()
    }

    /// Returns the human-readable name matching RFC 0010 §3.1 enum name.
    pub fn name(self) -> &'static str {
        match self {
            InterruptionClass::Critical => "CRITICAL",
            InterruptionClass::High => "HIGH",
            InterruptionClass::Normal => "NORMAL",
            InterruptionClass::Low => "LOW",
            InterruptionClass::Silent => "SILENT",
        }
    }
}

#[cfg(test)]
mod interruption_tests {
    use super::*;

    #[test]
    fn test_critical_is_runtime_only() {
        assert!(InterruptionClass::Critical.is_runtime_only());
        assert!(!InterruptionClass::High.is_runtime_only());
        assert!(!InterruptionClass::Normal.is_runtime_only());
        assert!(!InterruptionClass::Low.is_runtime_only());
        assert!(!InterruptionClass::Silent.is_runtime_only());
    }

    #[test]
    fn test_ordering_by_urgency() {
        // Lower discriminant = higher urgency = less than
        assert!(InterruptionClass::Critical < InterruptionClass::High);
        assert!(InterruptionClass::High < InterruptionClass::Normal);
        assert!(InterruptionClass::Normal < InterruptionClass::Low);
        assert!(InterruptionClass::Low < InterruptionClass::Silent);
    }

    #[test]
    fn test_unconditional_bypass_quiet_hours() {
        assert!(InterruptionClass::Critical.unconditionally_bypasses_quiet_hours());
        assert!(InterruptionClass::Silent.unconditionally_bypasses_quiet_hours());
        // These do NOT unconditionally bypass:
        assert!(!InterruptionClass::High.unconditionally_bypasses_quiet_hours());
        assert!(!InterruptionClass::Normal.unconditionally_bypasses_quiet_hours());
        assert!(!InterruptionClass::Low.unconditionally_bypasses_quiet_hours());
    }

    #[test]
    fn test_budget_counting() {
        // Critical and Silent are budget-free
        assert!(InterruptionClass::Critical.bypasses_budget());
        assert!(InterruptionClass::Silent.bypasses_budget());
        // High, Normal, Low count against budget
        assert!(InterruptionClass::High.counts_against_budget());
        assert!(InterruptionClass::Normal.counts_against_budget());
        assert!(InterruptionClass::Low.counts_against_budget());
        // Consistency: counts_against_budget and bypasses_budget are complements
        assert!(!InterruptionClass::Critical.counts_against_budget());
        assert!(!InterruptionClass::Silent.counts_against_budget());
    }

    #[test]
    fn test_default_is_normal() {
        let default = InterruptionClass::default();
        assert_eq!(default, InterruptionClass::Normal);
    }

    #[test]
    fn test_all_names_are_uppercase() {
        for class in [
            InterruptionClass::Critical,
            InterruptionClass::High,
            InterruptionClass::Normal,
            InterruptionClass::Low,
            InterruptionClass::Silent,
        ] {
            let name = class.name();
            assert_eq!(name, name.to_uppercase(), "Name {name} must be uppercase");
        }
    }
}
