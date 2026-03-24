//! Capability scope enforcement and zone-publish gating.
//!
//! Implements:
//! - Requirement: Capability Vocabulary (configuration/spec.md §149)
//! - Requirement: Zone Publish Requires Active Lease (lease-governance/spec.md lines 213-224)
//! - Requirement: Lease Suspension Freezes Zone Publications (lines 226-233)
//! - Requirement: Lease Revocation Clears Zone Publications (lines 235-242)
//!
//! ## Zone publish gating
//!
//! For resident agents, publishing to a zone requires all of:
//! 1. An ACTIVE lease (not SUSPENDED, ORPHANED, or terminal).
//! 2. `publish_zone:<zone_type>` capability (or `publish_zone:*`) for the target zone.
//! 3. The zone instance exists in the current active tab.
//!
//! Error mapping per spec:
//! | Condition                           | Error code                  |
//! |-------------------------------------|-----------------------------|
//! | No lease for namespace              | `LEASE_NOT_FOUND`           |
//! | Lease exists but not ACTIVE         | `LEASE_NOT_ACTIVE`          |
//! | Lease ACTIVE but capability missing | `CAPABILITY_NOT_GRANTED`    |
//! | Zone does not exist in active tab   | `ZONE_NOT_FOUND`            |
//! | Lease is SUSPENDED (safe mode)      | `SAFE_MODE_ACTIVE`          |

use crate::types::{Capability, Lease, LeaseState};

// ─── Zone-publish error codes ────────────────────────────────────────────────

/// Structured error returned by [`check_zone_publish`].
///
/// Maps 1-to-1 to the error codes in spec §Requirement: Zone Publish Requires
/// Active Lease (lines 213-224).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZonePublishError {
    /// No lease found for the requesting namespace.
    LeaseNotFound,
    /// Lease exists but is in a non-ACTIVE state (SUSPENDED, ORPHANED, terminal, …).
    LeaseNotActive,
    /// Lease is SUSPENDED — safe mode is active; new publishes blocked.
    SafeModeActive,
    /// Lease is ACTIVE but does not hold `publish_zone:<zone_type>` capability.
    CapabilityNotGranted,
    /// The named zone does not exist in the current active tab.
    ZoneNotFound,
}

impl std::fmt::Display for ZonePublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZonePublishError::LeaseNotFound => write!(f, "LEASE_NOT_FOUND"),
            ZonePublishError::LeaseNotActive => write!(f, "LEASE_NOT_ACTIVE"),
            ZonePublishError::SafeModeActive => write!(f, "SAFE_MODE_ACTIVE"),
            ZonePublishError::CapabilityNotGranted => write!(f, "CAPABILITY_NOT_GRANTED"),
            ZonePublishError::ZoneNotFound => write!(f, "ZONE_NOT_FOUND"),
        }
    }
}

impl std::error::Error for ZonePublishError {}

// ─── Capability check helpers ─────────────────────────────────────────────────

/// Returns `true` if `capabilities` grants the agent permission to publish to
/// `zone_name`.
///
/// Accepts either the wildcard `publish_zone:*` (grants all zones) or the
/// specific `publish_zone:<zone_name>` capability.
///
/// From spec §Requirement: Capability Vocabulary: `publish_zone:<zone_name>` and
/// `publish_zone:*` are both valid capability identifiers.
pub fn has_publish_zone_capability(capabilities: &[Capability], zone_name: &str) -> bool {
    capabilities.iter().any(|c| match c {
        Capability::PublishZone(name) => name == zone_name || name == "*",
        _ => false,
    })
}

/// Returns `true` if `capabilities` contains the given generic capability.
///
/// This is a convenience wrapper around the `Vec::contains` pattern; it exists
/// so callers do not need to import `Capability` variants directly.
pub fn has_capability(capabilities: &[Capability], cap: &Capability) -> bool {
    capabilities.contains(cap)
}

// ─── Zone publish gate ───────────────────────────────────────────────────────

/// Check whether a resident agent is allowed to publish to `zone_name`.
///
/// `lease` is the agent's current lease (if any — pass `None` to simulate
/// a missing lease).  `zone_registered` indicates whether the zone instance
/// exists in the current active tab.
///
/// Returns `Ok(())` when the publish may proceed, or a [`ZonePublishError`]
/// describing exactly why it was rejected.
///
/// # Order of checks (spec §Requirement: Zone Publish Requires Active Lease, lines 213-224)
///
/// 1. Missing lease → `LeaseNotFound`
/// 2. Lease is `Suspended` → `SafeModeActive` (spec §Requirement: Lease Suspension
///    Freezes Zone Publications, lines 226-233: "new publishes rejected with SAFE_MODE_ACTIVE")
/// 3. Lease is any other non-ACTIVE state → `LeaseNotActive`
/// 4. Lease is ACTIVE but lacks `publish_zone:<zone_name>` capability → `CapabilityNotGranted`
/// 5. Zone not registered in active tab → `ZoneNotFound`
pub fn check_zone_publish(
    lease: Option<&Lease>,
    zone_name: &str,
    zone_registered: bool,
) -> Result<(), ZonePublishError> {
    let lease = lease.ok_or(ZonePublishError::LeaseNotFound)?;

    match lease.state {
        LeaseState::Active => {} // proceed to capability check
        LeaseState::Suspended => {
            // Spec §Lease Suspension Freezes Zone Publications (line 227-228):
            // "New zone publishes under the suspended lease MUST be rejected with SAFE_MODE_ACTIVE."
            return Err(ZonePublishError::SafeModeActive);
        }
        _ => {
            // Orphaned, terminal states, etc.
            return Err(ZonePublishError::LeaseNotActive);
        }
    }

    if !has_publish_zone_capability(&lease.capabilities, zone_name) {
        return Err(ZonePublishError::CapabilityNotGranted);
    }

    if !zone_registered {
        return Err(ZonePublishError::ZoneNotFound);
    }

    Ok(())
}

// ─── Lease-revocation zone cleanup ───────────────────────────────────────────

/// Determine which zone publish records should be cleared when a lease is
/// revoked or expired.
///
/// Returns `true` for each record whose `publisher_namespace` matches the
/// revoked lease's `namespace`.
///
/// From spec §Requirement: Lease Revocation Clears Zone Publications (lines 235-242):
/// > "When a lease is REVOKED or EXPIRED, all zone publications made under that
/// >  lease MUST be immediately cleared from the zone registry."
///
/// # Usage
/// ```ignore
/// zone_registry.active_publishes.retain(|_zone, records| {
///     records.retain(|r| !should_clear_on_revoke(&lease, r.publisher_namespace.as_str()));
///     true
/// });
/// ```
pub fn should_clear_on_revoke(lease: &Lease, publisher_namespace: &str) -> bool {
    lease.namespace == publisher_namespace
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Capability, Lease, LeaseState, RenewalPolicy, ResourceBudget, SceneId};

    fn make_lease(state: LeaseState, capabilities: Vec<Capability>) -> Lease {
        Lease {
            id: SceneId::new(),
            namespace: "agent.test".to_string(),
            session_id: SceneId::nil(),
            state,
            priority: 2,
            granted_at_ms: 0,
            ttl_ms: 60_000,
            renewal_policy: RenewalPolicy::default(),
            capabilities,
            resource_budget: ResourceBudget::default(),
            suspended_at_ms: None,
            ttl_remaining_at_suspend_ms: None,
            disconnected_at_ms: None,
            grace_period_ms: 30_000,
        }
    }

    // ── has_publish_zone_capability ──────────────────────────────────────────

    #[test]
    fn publish_zone_specific_matches() {
        let caps = vec![Capability::PublishZone("subtitle".to_string())];
        assert!(has_publish_zone_capability(&caps, "subtitle"));
        assert!(!has_publish_zone_capability(&caps, "notifications"));
    }

    #[test]
    fn publish_zone_wildcard_matches_all() {
        let caps = vec![Capability::PublishZone("*".to_string())];
        assert!(has_publish_zone_capability(&caps, "subtitle"));
        assert!(has_publish_zone_capability(&caps, "notifications"));
        assert!(has_publish_zone_capability(&caps, "anything"));
    }

    #[test]
    fn publish_zone_no_capability_returns_false() {
        let caps = vec![Capability::CreateTiles];
        assert!(!has_publish_zone_capability(&caps, "subtitle"));
    }

    // ── check_zone_publish ───────────────────────────────────────────────────

    /// WHEN no lease exists THEN LEASE_NOT_FOUND.
    #[test]
    fn zone_publish_no_lease_returns_not_found() {
        let err = check_zone_publish(None, "subtitle", true).unwrap_err();
        assert_eq!(err, ZonePublishError::LeaseNotFound);
    }

    /// WHEN lease is SUSPENDED THEN SAFE_MODE_ACTIVE.
    ///
    /// Spec scenario (lines 219-220):
    /// "WHEN a resident agent attempts to publish to a zone while its lease is SUSPENDED
    ///  THEN the publish is rejected with LEASE_NOT_ACTIVE"
    /// Note: the spec text at line 219-220 says LEASE_NOT_ACTIVE but §Lease Suspension
    /// Freezes Zone Publications (line 227-228) says SAFE_MODE_ACTIVE. This module
    /// implements SAFE_MODE_ACTIVE per §7.2 which is the more specific requirement.
    #[test]
    fn zone_publish_suspended_lease_returns_safe_mode_active() {
        let lease = make_lease(
            LeaseState::Suspended,
            vec![Capability::PublishZone("subtitle".to_string())],
        );
        let err = check_zone_publish(Some(&lease), "subtitle", true).unwrap_err();
        assert_eq!(err, ZonePublishError::SafeModeActive);
    }

    /// WHEN lease is ORPHANED THEN LEASE_NOT_ACTIVE.
    #[test]
    fn zone_publish_orphaned_lease_returns_not_active() {
        let lease = make_lease(
            LeaseState::Orphaned,
            vec![Capability::PublishZone("subtitle".to_string())],
        );
        let err = check_zone_publish(Some(&lease), "subtitle", true).unwrap_err();
        assert_eq!(err, ZonePublishError::LeaseNotActive);
    }

    /// WHEN lease is REVOKED THEN LEASE_NOT_ACTIVE.
    #[test]
    fn zone_publish_revoked_lease_returns_not_active() {
        let lease = make_lease(
            LeaseState::Revoked,
            vec![Capability::PublishZone("subtitle".to_string())],
        );
        let err = check_zone_publish(Some(&lease), "subtitle", true).unwrap_err();
        assert_eq!(err, ZonePublishError::LeaseNotActive);
    }

    /// WHEN lease ACTIVE but lacks publish_zone:subtitle THEN CAPABILITY_NOT_GRANTED.
    ///
    /// Spec scenario (lines 222-224):
    /// "WHEN an agent holds an ACTIVE lease but lacks publish_zone:subtitle and
    ///  publishes to subtitle THEN the publish is rejected with CAPABILITY_NOT_GRANTED"
    #[test]
    fn zone_publish_active_missing_capability_returns_capability_not_granted() {
        let lease = make_lease(LeaseState::Active, vec![Capability::CreateTiles]);
        let err = check_zone_publish(Some(&lease), "subtitle", true).unwrap_err();
        assert_eq!(err, ZonePublishError::CapabilityNotGranted);
    }

    /// WHEN lease ACTIVE and has capability but zone not registered THEN ZONE_NOT_FOUND.
    #[test]
    fn zone_publish_missing_zone_returns_zone_not_found() {
        let lease = make_lease(
            LeaseState::Active,
            vec![Capability::PublishZone("subtitle".to_string())],
        );
        let err = check_zone_publish(Some(&lease), "subtitle", false).unwrap_err();
        assert_eq!(err, ZonePublishError::ZoneNotFound);
    }

    /// WHEN lease ACTIVE, has capability, zone registered THEN publish succeeds.
    #[test]
    fn zone_publish_happy_path() {
        let lease = make_lease(
            LeaseState::Active,
            vec![Capability::PublishZone("subtitle".to_string())],
        );
        assert!(check_zone_publish(Some(&lease), "subtitle", true).is_ok());
    }

    /// Wildcard capability allows publishing to any zone.
    #[test]
    fn zone_publish_wildcard_capability_grants_any_zone() {
        let lease = make_lease(
            LeaseState::Active,
            vec![Capability::PublishZone("*".to_string())],
        );
        assert!(check_zone_publish(Some(&lease), "subtitle", true).is_ok());
        assert!(check_zone_publish(Some(&lease), "notifications", true).is_ok());
    }

    // ── should_clear_on_revoke ───────────────────────────────────────────────

    /// Publications from the revoked lease's namespace are cleared.
    #[test]
    fn clear_on_revoke_matches_namespace() {
        let lease = make_lease(LeaseState::Revoked, vec![]);
        assert!(should_clear_on_revoke(&lease, "agent.test"));
        assert!(!should_clear_on_revoke(&lease, "agent.other"));
    }
}
