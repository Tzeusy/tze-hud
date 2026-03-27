//! Self-event suppression (RFC 0010 §14.2).
//!
//! The runtime MUST NOT deliver a SceneEvent to the agent whose MutationBatch
//! caused the state change. Lease and degradation events are **exempt** from
//! suppression and are always delivered even if the agent triggered them.
//!
//! The suppression check must complete in < 1 microsecond per event (spec line 255).
//! This is achieved by keeping the implementation to a single string comparison
//! and a prefix check — no heap allocation on the hot path.

/// Returns `true` if the event should be **suppressed** for the given agent.
///
/// An event is suppressed when:
/// 1. The event was caused by `source_namespace` (the agent that submitted
///    the MutationBatch), AND
/// 2. The event is NOT a lease event (`system.lease_*`) or degradation event
///    (`system.degradation_*`) — those are exempt from suppression.
///
/// # Arguments
///
/// * `event_type` — the dotted event type string (e.g., `"scene.tile.created"`)
/// * `source_namespace` — the agent namespace that caused this event (from the
///   MutationBatch; empty string for runtime-internal events with no agent cause)
/// * `recipient_namespace` — the namespace of the agent considering receiving
///   this event
///
/// # Performance
///
/// This function is designed to be < 1 µs. It performs:
/// - Two `starts_with` checks (exempt category detection, O(prefix length))
/// - One `==` comparison (namespace match, O(len))
///
/// No heap allocation, no locking.
#[inline]
pub fn is_suppressed(event_type: &str, source_namespace: &str, recipient_namespace: &str) -> bool {
    // Empty source namespace means the event originated from the runtime
    // itself (no agent cause). Never suppressed.
    if source_namespace.is_empty() {
        return false;
    }

    // Exempt: lease changes — always delivered even to the causer (spec line 265).
    if event_type.starts_with("system.lease_") {
        return false;
    }

    // Exempt: degradation notices — always delivered even to the causer.
    if event_type.starts_with("system.degradation_") {
        return false;
    }

    // Suppress if the recipient is the source of the event.
    source_namespace == recipient_namespace
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic self-suppression ────────────────────────────────────────────────

    #[test]
    fn test_self_event_suppressed() {
        // Agent A creates a tile — Agent A should NOT receive TileCreated
        assert!(is_suppressed("scene.tile.created", "agent_a", "agent_a"));
    }

    #[test]
    fn test_other_agent_not_suppressed() {
        // Agent A creates a tile — Agent B SHOULD receive TileCreated
        assert!(!is_suppressed("scene.tile.created", "agent_a", "agent_b"));
    }

    // ── Lease event exemption ────────────────────────────────────────────────

    #[test]
    fn test_lease_revoked_not_suppressed_for_self() {
        // Agent A's budget violation triggers lease revocation for Agent A —
        // Agent A MUST still receive LeaseRevokedEvent (spec line 265).
        assert!(!is_suppressed("system.lease_revoked", "agent_a", "agent_a"));
    }

    #[test]
    fn test_lease_granted_not_suppressed_for_self() {
        assert!(!is_suppressed("system.lease_granted", "agent_a", "agent_a"));
    }

    #[test]
    fn test_lease_expired_not_suppressed_for_self() {
        assert!(!is_suppressed("system.lease_expired", "agent_a", "agent_a"));
    }

    // ── Degradation event exemption ──────────────────────────────────────────

    #[test]
    fn test_degradation_event_not_suppressed_for_self() {
        assert!(!is_suppressed(
            "system.degradation_changed",
            "agent_a",
            "agent_a"
        ));
    }

    #[test]
    fn test_degradation_level_changed_not_suppressed() {
        assert!(!is_suppressed(
            "system.degradation_level_changed",
            "agent_a",
            "agent_a"
        ));
    }

    // ── Runtime-internal events (no source) ──────────────────────────────────

    #[test]
    fn test_runtime_event_no_source_not_suppressed() {
        // Events with empty source_namespace are runtime-internal;
        // never suppressed for any agent.
        assert!(!is_suppressed("scene.tab.created", "", "agent_a"));
        assert!(!is_suppressed("system.safe_mode_entered", "", "agent_a"));
    }

    // ── Audit events (safe mode, freeze) ─────────────────────────────────────
    // Audit events (system.safe_mode_entered, freeze, etc.) are NOT delivered
    // to agents at all (handled at a higher layer), but if they were to reach
    // the suppression check, they are NOT from an agent cause (empty source),
    // so they pass through correctly.

    #[test]
    fn test_safe_mode_entered_no_source() {
        // Safe mode is a viewer/runtime action — source is empty.
        // suppression check passes (not suppressed); the higher-level audit
        // visibility filter will block delivery.
        assert!(!is_suppressed("system.safe_mode_entered", "", "agent_a"));
    }

    // ── Scene topology events ─────────────────────────────────────────────────

    #[test]
    fn test_scene_topology_self_suppressed() {
        let events = [
            "scene.tile.created",
            "scene.tile.deleted",
            "scene.tab.created",
            "scene.zone.occupancy_changed",
        ];
        for ev in &events {
            assert!(
                is_suppressed(ev, "agent_a", "agent_a"),
                "Expected {ev} to be suppressed for self"
            );
            assert!(
                !is_suppressed(ev, "agent_a", "agent_b"),
                "Expected {ev} NOT to be suppressed for other agent"
            );
        }
    }

    // ── Agent events ──────────────────────────────────────────────────────────

    #[test]
    fn test_agent_event_self_suppressed() {
        // Agent emitting its own agent event should not receive it back
        assert!(is_suppressed(
            "agent.my_agent.doorbell.ring",
            "my_agent",
            "my_agent"
        ));
        // Other agents do receive it
        assert!(!is_suppressed(
            "agent.my_agent.doorbell.ring",
            "my_agent",
            "other_agent"
        ));
    }
}
