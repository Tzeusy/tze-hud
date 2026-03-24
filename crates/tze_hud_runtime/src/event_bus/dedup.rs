//! Deduplication for events that appear in multiple subscription categories
//! (RFC 0010 §1.2, spec line 306-313).
//!
//! Currently, ZoneOccupancyChanged routes to both SCENE_TOPOLOGY and ZONE_EVENTS.
//! An agent subscribing to both MUST receive the event exactly once.
//!
//! Deduplication is per-agent: the delivery layer calls `should_deliver_once`
//! to determine if an event type is a known dual-route event, and if so ensures
//! it is only enqueued to the agent's outbound stream once.

/// Returns `true` if the given event type is subject to dual-routing deduplication.
///
/// Dual-route events are events that map to more than one subscription category
/// prefix. Currently: ZoneOccupancyChanged.
#[inline]
pub fn is_dual_route_event(event_type: &str) -> bool {
    // ZoneOccupancyChanged matches both:
    //   SCENE_TOPOLOGY → "scene.*"
    //   ZONE_EVENTS    → "scene.zone.*"
    event_type == "scene.zone.occupancy_changed"
}

/// Returns the canonical (authoritative) subscription category for a dual-route event.
///
/// When deduplicating, the event is attributed to the canonical category to ensure
/// consistent routing. The canonical category is the more specific one (ZONE_EVENTS
/// for ZoneOccupancyChanged).
#[inline]
pub fn canonical_category_for_dual_route(event_type: &str) -> Option<&'static str> {
    match event_type {
        "scene.zone.occupancy_changed" => Some("ZONE_EVENTS"),
        _ => None,
    }
}

/// Deduplication state for a single delivery pass.
///
/// Created fresh for each event before the fan-out loop over subscribers.
/// Tracks which event types have already been delivered to a particular agent
/// (per-delivery-pass, not persistent across events).
///
/// Usage:
/// ```text
/// let mut dedup = DeliveryDedup::new();
/// for subscriber in subscribed_agents {
///     if dedup.allow(event_type) {
///         deliver(subscriber, event);
///     }
/// }
/// ```
///
/// Note: `DeliveryDedup` is per-agent per-event — callers create a new one for
/// each (agent, event) pair during fan-out, or equivalently use
/// `should_deliver_to_agent` which wraps this logic.
#[derive(Debug, Default)]
pub struct DeliveryDedup {
    already_delivered: std::collections::HashSet<String>,
}

impl DeliveryDedup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if this event type should be delivered (first occurrence).
    /// Returns `false` if it has already been delivered (duplicate).
    pub fn allow(&mut self, event_type: &str) -> bool {
        // Only track dual-route events for dedup; all other events pass through.
        if !is_dual_route_event(event_type) {
            return true;
        }
        self.already_delivered.insert(event_type.to_string())
    }
}

/// Stateless check: given that an agent is subscribed to `matched_categories`,
/// should the event be delivered to the agent?
///
/// For non-dual-route events: always `true` (caller already checked subscription).
/// For dual-route events: `true` only when this is the first delivery decision
/// (deduplication must happen at the caller level using `DeliveryDedup`).
///
/// This function is provided as a helper for tests and documentation.
pub fn should_deliver_once(event_type: &str, matched_category_count: usize) -> bool {
    if !is_dual_route_event(event_type) {
        // Not a dual-route event — deliver regardless of category count.
        return matched_category_count > 0;
    }
    // Dual-route: deliver if at least one category matches; dedup ensures once.
    matched_category_count > 0
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_dual_route_event ───────────────────────────────────────────────────

    #[test]
    fn test_zone_occupancy_is_dual_route() {
        assert!(is_dual_route_event("scene.zone.occupancy_changed"));
    }

    #[test]
    fn test_tile_created_not_dual_route() {
        assert!(!is_dual_route_event("scene.tile.created"));
    }

    #[test]
    fn test_lease_event_not_dual_route() {
        assert!(!is_dual_route_event("system.lease_revoked"));
    }

    #[test]
    fn test_other_zone_events_not_dual_route() {
        // Only the specific ZoneOccupancyChanged event is dual-route;
        // hypothetical future zone events are not.
        assert!(!is_dual_route_event("scene.zone.published"));
    }

    // ── canonical_category_for_dual_route ─────────────────────────────────────

    #[test]
    fn test_canonical_category_zone_occupancy() {
        assert_eq!(
            canonical_category_for_dual_route("scene.zone.occupancy_changed"),
            Some("ZONE_EVENTS")
        );
    }

    #[test]
    fn test_canonical_category_non_dual_route() {
        assert_eq!(
            canonical_category_for_dual_route("scene.tile.created"),
            None
        );
    }

    // ── DeliveryDedup ────────────────────────────────────────────────────────

    #[test]
    fn test_dedup_allows_first_delivery() {
        let mut dedup = DeliveryDedup::new();
        assert!(dedup.allow("scene.zone.occupancy_changed"));
    }

    #[test]
    fn test_dedup_blocks_second_delivery() {
        let mut dedup = DeliveryDedup::new();
        assert!(dedup.allow("scene.zone.occupancy_changed"));
        // Second call should return false (already delivered)
        assert!(!dedup.allow("scene.zone.occupancy_changed"));
    }

    #[test]
    fn test_dedup_non_dual_route_always_passes() {
        let mut dedup = DeliveryDedup::new();
        // Non-dual-route events always pass through
        assert!(dedup.allow("scene.tile.created"));
        assert!(dedup.allow("scene.tile.created")); // second call also passes
        assert!(dedup.allow("system.lease_revoked"));
        assert!(dedup.allow("system.lease_revoked")); // second call also passes
    }

    #[test]
    fn test_dedup_different_event_types_independent() {
        let mut dedup = DeliveryDedup::new();
        assert!(dedup.allow("scene.zone.occupancy_changed"));
        // A different dual-route type (if added in future) would be independent
        // For now just verify non-dual-route types are independent
        assert!(dedup.allow("scene.tile.created"));
        assert!(!dedup.allow("scene.zone.occupancy_changed")); // still blocked
    }

    // ── Integration scenario: agent subscribed to both categories ─────────────

    #[test]
    fn test_agent_receives_zone_occupancy_once_with_both_subscriptions() {
        // Simulate: agent subscribes to SCENE_TOPOLOGY and ZONE_EVENTS.
        // ZoneOccupancyChanged matches both. Agent should receive exactly once.
        let event_type = "scene.zone.occupancy_changed";

        // Simulate the fan-out logic using DeliveryDedup
        let mut dedup = DeliveryDedup::new();
        let mut delivery_count = 0u32;

        // Category 1: SCENE_TOPOLOGY matched
        if dedup.allow(event_type) {
            delivery_count += 1;
        }
        // Category 2: ZONE_EVENTS matched (would be a duplicate delivery)
        if dedup.allow(event_type) {
            delivery_count += 1;
        }

        assert_eq!(delivery_count, 1, "Event must be delivered exactly once");
    }
}
