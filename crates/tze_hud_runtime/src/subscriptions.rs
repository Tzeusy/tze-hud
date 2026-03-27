//! Subscription registry — category-based event filtering per agent session.
//!
//! Implements RFC 0010 §7.1–§7.3: the nine subscription categories, their
//! capability requirements, mandatory categories, prefix-match filters,
//! and the 32-subscription limit.

use std::collections::{HashMap, HashSet};

// ─── Category constants ───────────────────────────────────────────────────────

/// All nine subscription category names (SHOUTY_SNAKE_CASE, as used on the wire).
pub const CATEGORY_SCENE_TOPOLOGY: &str = "SCENE_TOPOLOGY";
pub const CATEGORY_INPUT_EVENTS: &str = "INPUT_EVENTS";
pub const CATEGORY_FOCUS_EVENTS: &str = "FOCUS_EVENTS";
pub const CATEGORY_DEGRADATION_NOTICES: &str = "DEGRADATION_NOTICES";
pub const CATEGORY_LEASE_CHANGES: &str = "LEASE_CHANGES";
pub const CATEGORY_ZONE_EVENTS: &str = "ZONE_EVENTS";
pub const CATEGORY_TELEMETRY_FRAMES: &str = "TELEMETRY_FRAMES";
pub const CATEGORY_ATTENTION_EVENTS: &str = "ATTENTION_EVENTS";
pub const CATEGORY_AGENT_EVENTS: &str = "AGENT_EVENTS";

/// Maximum number of subscriptions per agent session (spec line 183).
pub const MAX_SUBSCRIPTIONS_PER_AGENT: usize = 32;

/// The two mandatory categories that cannot be opted out of.
pub const MANDATORY_CATEGORIES: &[&str] = &[CATEGORY_DEGRADATION_NOTICES, CATEGORY_LEASE_CHANGES];

// ─── Category → event type prefix mapping ─────────────────────────────────────

/// Maps a subscription category to its event type prefix pattern.
///
/// Returns `None` for unknown categories. The prefix patterns use trailing
/// wildcards (e.g., `"scene.*"`) and are used for prefix-match filtering.
pub fn category_prefix(category: &str) -> Option<&'static str> {
    match category {
        CATEGORY_SCENE_TOPOLOGY => Some("scene."),
        CATEGORY_LEASE_CHANGES => Some("system.lease_"),
        CATEGORY_DEGRADATION_NOTICES => Some("system.degradation_"),
        CATEGORY_ZONE_EVENTS => Some("scene.zone."),
        CATEGORY_FOCUS_EVENTS => Some("scene.focus."),
        CATEGORY_INPUT_EVENTS => Some("input."),
        CATEGORY_AGENT_EVENTS => Some("agent."),
        CATEGORY_TELEMETRY_FRAMES => Some("system.telemetry_"),
        CATEGORY_ATTENTION_EVENTS => Some("system.attention_"),
        _ => None,
    }
}

/// Returns the capability string required to subscribe to a category.
///
/// Returns `None` for categories that are open to all agents (no capability gate).
/// The mandatory categories (DEGRADATION_NOTICES, LEASE_CHANGES) are always
/// subscribed — capability gating is irrelevant for them at the subscription
/// level, but they are listed as gateless here since we never deny them.
pub fn required_capability(category: &str) -> Option<&'static str> {
    match category {
        CATEGORY_SCENE_TOPOLOGY => None, // open to all
        CATEGORY_INPUT_EVENTS => Some("access_input_events"),
        CATEGORY_FOCUS_EVENTS => Some("access_input_events"),
        CATEGORY_DEGRADATION_NOTICES => None, // mandatory/always granted
        CATEGORY_LEASE_CHANGES => None,       // mandatory/always granted
        CATEGORY_ZONE_EVENTS => None,         // open to all
        CATEGORY_TELEMETRY_FRAMES => Some("read_telemetry"),
        CATEGORY_ATTENTION_EVENTS => None, // open to all
        CATEGORY_AGENT_EVENTS => None,     // open to all
        _ => None,
    }
}

// ─── Subscription entry ───────────────────────────────────────────────────────

/// A single active subscription for an agent.
///
/// Fields are private to enforce the invariant that `filter_prefix`, when
/// present, must start with the category's default prefix (RFC 0010 §7.2).
/// Construct via [`Subscription::new`] or [`Subscription::with_filter`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    /// The subscription category (e.g., "SCENE_TOPOLOGY").
    category: String,
    /// Optional finer-grained event type prefix filter within the category
    /// (e.g., "scene.zone." to receive only zone events within SCENE_TOPOLOGY).
    /// If `None`, the category's default prefix applies.
    filter_prefix: Option<String>,
}

impl Subscription {
    /// Create a subscription for `category` with no finer-grained filter.
    /// The category's default prefix applies to event routing.
    pub fn new(category: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            filter_prefix: None,
        }
    }

    /// Create a subscription with an explicit `filter_prefix`, narrowing delivery
    /// to events whose type starts with that prefix.
    ///
    /// # Panics
    ///
    /// Panics if `filter` does not start with the category's default prefix
    /// (i.e., the filter would escape the category's event namespace).
    /// For an unknown category, the filter is accepted as-is (no prefix to check).
    ///
    /// Use this constructor when the caller has already validated that the filter
    /// is within bounds, or in tests with known-good inputs.
    pub fn with_filter(category: impl Into<String>, filter: impl Into<String>) -> Self {
        let category = category.into();
        let filter = filter.into();
        if let Some(default_prefix) = category_prefix(&category) {
            assert!(
                filter.starts_with(default_prefix),
                "filter_prefix {:?} does not start with category {:?} default prefix {:?}",
                filter,
                category,
                default_prefix,
            );
        }
        Self {
            category,
            filter_prefix: Some(filter),
        }
    }

    /// Returns the subscription category name (e.g., `"SCENE_TOPOLOGY"`).
    pub fn category(&self) -> &str {
        &self.category
    }

    /// Returns the finer-grained filter prefix, if one is active.
    ///
    /// When `None`, the category's default prefix governs event routing.
    pub fn filter_prefix(&self) -> Option<&str> {
        self.filter_prefix.as_deref()
    }

    /// Set or clear the filter prefix.
    ///
    /// # Panics
    ///
    /// Panics if `filter` is `Some` and does not start with the category's
    /// default prefix (same invariant as [`Subscription::with_filter`]).
    pub(crate) fn set_filter_prefix(&mut self, filter: Option<String>) {
        if let (Some(fp), Some(default_prefix)) = (&filter, category_prefix(&self.category)) {
            assert!(
                fp.starts_with(default_prefix),
                "filter_prefix {:?} does not start with category {:?} default prefix {:?}",
                fp,
                self.category,
                default_prefix,
            );
        }
        self.filter_prefix = filter;
    }

    /// Returns true if this subscription matches the given event type.
    ///
    /// Matching rules (RFC 0010 §7.2):
    /// 1. If a finer-grained `filter_prefix` is set, the event type must start
    ///    with that prefix.
    /// 2. Otherwise, the event type must start with the category's default prefix.
    pub fn matches_event_type(&self, event_type: &str) -> bool {
        if let Some(ref filter) = self.filter_prefix {
            event_type.starts_with(filter.as_str())
        } else {
            // Use the category's default prefix
            if let Some(prefix) = category_prefix(&self.category) {
                event_type.starts_with(prefix)
            } else {
                false
            }
        }
    }
}

// ─── Per-agent subscription set ───────────────────────────────────────────────

/// Result of a `SubscriptionChange` operation.
#[derive(Clone, Debug, Default)]
pub struct SubscriptionChangeOutcome {
    /// The full active subscription set after the change (category names).
    pub active: Vec<String>,
    /// Requested additions that were denied (missing capability, limit exceeded,
    /// unknown category, or attempt to unsubscribe from mandatory category).
    pub denied: Vec<String>,
}

/// Manages the subscription set for a single agent session.
///
/// Enforces:
/// - The 32-subscription limit (spec line 183).
/// - Mandatory category pinning (DEGRADATION_NOTICES, LEASE_CHANGES always present).
/// - Capability gating (some categories require specific granted capabilities).
#[derive(Clone, Debug)]
pub struct AgentSubscriptions {
    /// Active subscriptions, indexed by category name.
    entries: HashMap<String, Subscription>,
}

impl AgentSubscriptions {
    /// Create a new subscription set with only the mandatory categories active.
    pub fn new() -> Self {
        let mut entries = HashMap::new();
        for cat in MANDATORY_CATEGORIES {
            entries.insert(cat.to_string(), Subscription::new(*cat));
        }
        Self { entries }
    }

    /// Apply a subscription change request.
    ///
    /// `subscribe` — `(category, filter_prefix)` pairs to add.  Pass
    ///   `filter_prefix = None` to use the category's default prefix.
    ///   A non-`None` `filter_prefix` narrows delivery to events whose type
    ///   starts with that prefix (RFC 0010 §7.2, spec line 179).
    /// `unsubscribe` — categories to remove.
    /// `granted_capabilities` — capabilities held by this agent session.
    ///
    /// Returns the outcome with `active` (full set after change) and `denied`
    /// (entries that were rejected).
    pub fn apply_change(
        &mut self,
        subscribe: &[(String, Option<String>)],
        unsubscribe: &[String],
        granted_capabilities: &[String],
    ) -> SubscriptionChangeOutcome {
        let mut denied = Vec::new();

        // Process unsubscriptions first
        for cat in unsubscribe {
            if MANDATORY_CATEGORIES.contains(&cat.as_str()) {
                // Cannot unsubscribe from mandatory categories (spec line 174).
                denied.push(cat.clone());
            } else {
                self.entries.remove(cat.as_str());
            }
        }

        // Process subscriptions
        for (cat, filter_prefix) in subscribe {
            // Reject unknown categories
            if category_prefix(cat.as_str()).is_none() {
                denied.push(cat.clone());
                continue;
            }

            // Check capability gate
            if let Some(req_cap) = required_capability(cat.as_str())
                && !granted_capabilities.iter().any(|c| c == req_cap)
            {
                denied.push(cat.clone());
                continue;
            }

            // Validate filter_prefix: if provided, it must start with the
            // category's default prefix (RFC 0010 §7.2). A filter that escapes
            // the category boundary could route events outside the agent's
            // granted capability scope.
            if let Some(fp) = filter_prefix {
                let default_prefix = category_prefix(cat.as_str()).unwrap_or("");
                if !fp.starts_with(default_prefix) {
                    denied.push(cat.clone());
                    continue;
                }
                // If already subscribed, update the filter_prefix.
                // This allows an agent to refine its filter without re-subscribing.
                if let Some(existing) = self.entries.get_mut(cat.as_str()) {
                    existing.set_filter_prefix(Some(fp.clone()));
                    continue;
                }
                // Enforce the 32-subscription limit
                if self.entries.len() >= MAX_SUBSCRIPTIONS_PER_AGENT {
                    denied.push(cat.clone());
                    continue;
                }
                self.entries.insert(
                    cat.clone(),
                    Subscription::with_filter(cat.clone(), fp.clone()),
                );
            } else {
                // If already subscribed, clear any stored filter (reset to default).
                if let Some(existing) = self.entries.get_mut(cat.as_str()) {
                    existing.set_filter_prefix(None);
                    continue;
                }
                // Enforce the 32-subscription limit
                if self.entries.len() >= MAX_SUBSCRIPTIONS_PER_AGENT {
                    denied.push(cat.clone());
                    continue;
                }
                self.entries
                    .insert(cat.clone(), Subscription::new(cat.clone()));
            }
        }

        SubscriptionChangeOutcome {
            active: self.active_categories(),
            denied,
        }
    }

    /// Returns all active subscription category names.
    pub fn active_categories(&self) -> Vec<String> {
        let mut cats: Vec<String> = self.entries.keys().cloned().collect();
        cats.sort();
        cats
    }

    /// Returns true if the agent should receive an event of the given type.
    ///
    /// Checks all active subscriptions for a prefix match. Returns true if any
    /// subscription matches. For ZoneOccupancyChanged deduplication, callers
    /// must use `matched_categories` to detect multi-match and deduplicate.
    pub fn should_receive(&self, event_type: &str) -> bool {
        self.entries
            .values()
            .any(|s| s.matches_event_type(event_type))
    }

    /// Returns the set of category names that match the given event type.
    ///
    /// Used by the deduplication layer to detect when the same event matches
    /// multiple categories (e.g., ZoneOccupancyChanged → both SCENE_TOPOLOGY
    /// and ZONE_EVENTS). The caller uses this to deliver the event exactly once.
    pub fn matched_categories(&self, event_type: &str) -> Vec<String> {
        self.entries
            .values()
            .filter(|s| s.matches_event_type(event_type))
            .map(|s| s.category().to_string())
            .collect()
    }

    /// Returns the number of active subscriptions.
    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

impl Default for AgentSubscriptions {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Global subscription registry ─────────────────────────────────────────────

/// Registry of all per-agent subscription sets keyed by agent namespace.
#[derive(Debug, Default)]
pub struct SubscriptionRegistry {
    agents: HashMap<String, AgentSubscriptions>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new agent (initializes mandatory subscriptions).
    pub fn register(&mut self, namespace: &str) {
        self.agents.entry(namespace.to_string()).or_default();
    }

    /// Remove an agent from the registry (on disconnect).
    pub fn remove(&mut self, namespace: &str) {
        self.agents.remove(namespace);
    }

    /// Apply a subscription change for `namespace`.
    ///
    /// Creates the agent entry if it does not exist (lenient: lazy registration).
    pub fn apply_change(
        &mut self,
        namespace: &str,
        subscribe: &[(String, Option<String>)],
        unsubscribe: &[String],
        granted_capabilities: &[String],
    ) -> SubscriptionChangeOutcome {
        let subs = self.agents.entry(namespace.to_string()).or_default();
        subs.apply_change(subscribe, unsubscribe, granted_capabilities)
    }

    /// Returns the subscription set for `namespace`, if registered.
    pub fn get(&self, namespace: &str) -> Option<&AgentSubscriptions> {
        self.agents.get(namespace)
    }

    /// Returns the set of namespaces that should receive the given event type,
    /// after subscription filtering.
    ///
    /// Each namespace appears at most once (deduplication happens per-agent
    /// inside `AgentSubscriptions::should_receive`).
    pub fn subscribers_for_event(&self, event_type: &str) -> HashSet<String> {
        self.agents
            .iter()
            .filter_map(|(ns, subs)| {
                if subs.should_receive(event_type) {
                    Some(ns.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(c: &[&str]) -> Vec<String> {
        c.iter().map(|s| s.to_string()).collect()
    }

    /// Build a plain subscribe list (no filter prefix) from string slices.
    fn subscribe(cats: &[&str]) -> Vec<(String, Option<String>)> {
        cats.iter().map(|s| (s.to_string(), None)).collect()
    }

    /// Build a single-entry subscribe list with an explicit filter prefix.
    fn subscribe_with_filter(cat: &str, prefix: &str) -> Vec<(String, Option<String>)> {
        vec![(cat.to_string(), Some(prefix.to_string()))]
    }

    // ── Category prefix mapping ───────────────────────────────────────────────

    #[test]
    fn test_nine_categories_mapped() {
        let categories = [
            CATEGORY_SCENE_TOPOLOGY,
            CATEGORY_INPUT_EVENTS,
            CATEGORY_FOCUS_EVENTS,
            CATEGORY_DEGRADATION_NOTICES,
            CATEGORY_LEASE_CHANGES,
            CATEGORY_ZONE_EVENTS,
            CATEGORY_TELEMETRY_FRAMES,
            CATEGORY_ATTENTION_EVENTS,
            CATEGORY_AGENT_EVENTS,
        ];
        for cat in &categories {
            assert!(
                category_prefix(cat).is_some(),
                "Category {cat} must have a prefix"
            );
        }
    }

    #[test]
    fn test_prefix_values() {
        assert_eq!(category_prefix(CATEGORY_SCENE_TOPOLOGY), Some("scene."));
        assert_eq!(
            category_prefix(CATEGORY_LEASE_CHANGES),
            Some("system.lease_")
        );
        assert_eq!(
            category_prefix(CATEGORY_DEGRADATION_NOTICES),
            Some("system.degradation_")
        );
        assert_eq!(category_prefix(CATEGORY_ZONE_EVENTS), Some("scene.zone."));
        assert_eq!(category_prefix(CATEGORY_FOCUS_EVENTS), Some("scene.focus."));
        assert_eq!(category_prefix(CATEGORY_INPUT_EVENTS), Some("input."));
        assert_eq!(category_prefix(CATEGORY_AGENT_EVENTS), Some("agent."));
        assert_eq!(
            category_prefix(CATEGORY_TELEMETRY_FRAMES),
            Some("system.telemetry_")
        );
        assert_eq!(
            category_prefix(CATEGORY_ATTENTION_EVENTS),
            Some("system.attention_")
        );
    }

    // ── Mandatory category subscription ──────────────────────────────────────

    #[test]
    fn test_new_agent_has_mandatory_categories() {
        let subs = AgentSubscriptions::new();
        let active = subs.active_categories();
        assert!(active.contains(&CATEGORY_DEGRADATION_NOTICES.to_string()));
        assert!(active.contains(&CATEGORY_LEASE_CHANGES.to_string()));
    }

    #[test]
    fn test_cannot_unsubscribe_from_mandatory_categories() {
        let mut subs = AgentSubscriptions::new();
        let outcome = subs.apply_change(
            &subscribe(&[]),
            &[
                CATEGORY_DEGRADATION_NOTICES.to_string(),
                CATEGORY_LEASE_CHANGES.to_string(),
            ],
            &caps(&[]),
        );
        assert!(
            outcome
                .denied
                .contains(&CATEGORY_DEGRADATION_NOTICES.to_string())
        );
        assert!(outcome.denied.contains(&CATEGORY_LEASE_CHANGES.to_string()));
        // Still in active set
        let active = subs.active_categories();
        assert!(active.contains(&CATEGORY_DEGRADATION_NOTICES.to_string()));
        assert!(active.contains(&CATEGORY_LEASE_CHANGES.to_string()));
    }

    // ── Normal subscribe/unsubscribe ──────────────────────────────────────────

    #[test]
    fn test_subscribe_and_unsubscribe() {
        let mut subs = AgentSubscriptions::new();
        let outcome = subs.apply_change(&subscribe(&[CATEGORY_SCENE_TOPOLOGY]), &[], &caps(&[]));
        assert!(outcome.denied.is_empty());
        assert!(
            outcome
                .active
                .contains(&CATEGORY_SCENE_TOPOLOGY.to_string())
        );

        let outcome2 = subs.apply_change(
            &subscribe(&[]),
            &[CATEGORY_SCENE_TOPOLOGY.to_string()],
            &caps(&[]),
        );
        assert!(outcome2.denied.is_empty());
        assert!(
            !outcome2
                .active
                .contains(&CATEGORY_SCENE_TOPOLOGY.to_string())
        );
    }

    // ── Capability gating ─────────────────────────────────────────────────────

    #[test]
    fn test_input_events_requires_capability() {
        let mut subs = AgentSubscriptions::new();
        // Without capability: denied
        let outcome = subs.apply_change(&subscribe(&[CATEGORY_INPUT_EVENTS]), &[], &caps(&[]));
        assert!(outcome.denied.contains(&CATEGORY_INPUT_EVENTS.to_string()));
        assert!(!outcome.active.contains(&CATEGORY_INPUT_EVENTS.to_string()));

        // With capability: granted
        let outcome2 = subs.apply_change(
            &subscribe(&[CATEGORY_INPUT_EVENTS]),
            &[],
            &caps(&["access_input_events"]),
        );
        assert!(outcome2.denied.is_empty());
        assert!(outcome2.active.contains(&CATEGORY_INPUT_EVENTS.to_string()));
    }

    #[test]
    fn test_focus_events_requires_access_input_events_capability() {
        let mut subs = AgentSubscriptions::new();
        // Without capability: denied
        let outcome = subs.apply_change(&subscribe(&[CATEGORY_FOCUS_EVENTS]), &[], &caps(&[]));
        assert!(outcome.denied.contains(&CATEGORY_FOCUS_EVENTS.to_string()));
        assert!(!outcome.active.contains(&CATEGORY_FOCUS_EVENTS.to_string()));

        // With canonical access_input_events capability: granted
        let outcome2 = subs.apply_change(
            &subscribe(&[CATEGORY_FOCUS_EVENTS]),
            &[],
            &caps(&["access_input_events"]),
        );
        assert!(outcome2.denied.is_empty());
        assert!(outcome2.active.contains(&CATEGORY_FOCUS_EVENTS.to_string()));
    }

    #[test]
    fn test_telemetry_frames_requires_read_telemetry_capability() {
        let mut subs = AgentSubscriptions::new();
        // Without capability: denied
        let outcome = subs.apply_change(&subscribe(&[CATEGORY_TELEMETRY_FRAMES]), &[], &caps(&[]));
        assert!(
            outcome
                .denied
                .contains(&CATEGORY_TELEMETRY_FRAMES.to_string())
        );

        // With canonical read_telemetry capability: granted
        let outcome2 = subs.apply_change(
            &subscribe(&[CATEGORY_TELEMETRY_FRAMES]),
            &[],
            &caps(&["read_telemetry"]),
        );
        assert!(outcome2.denied.is_empty());
        assert!(
            outcome2
                .active
                .contains(&CATEGORY_TELEMETRY_FRAMES.to_string())
        );
    }

    // ── 32-subscription limit ──────────────────────────────────────────────────

    #[test]
    fn test_subscription_limit_enforced() {
        let mut subs = AgentSubscriptions::new();
        // Fill up to MAX - 2 (mandatory already take 2 slots)
        for i in 0..(MAX_SUBSCRIPTIONS_PER_AGENT - 2) {
            let fake_cat = format!("FAKE_CAT_{i}");
            // Directly insert to bypass category validation for this test
            subs.entries
                .insert(fake_cat.clone(), Subscription::new(fake_cat));
        }
        assert_eq!(subs.count(), MAX_SUBSCRIPTIONS_PER_AGENT);

        // Attempt to subscribe to one more — should be denied
        let outcome = subs.apply_change(&subscribe(&[CATEGORY_SCENE_TOPOLOGY]), &[], &caps(&[]));
        assert!(
            outcome
                .denied
                .contains(&CATEGORY_SCENE_TOPOLOGY.to_string())
        );
    }

    #[test]
    fn test_33rd_subscription_denied() {
        let mut subs = AgentSubscriptions::new();
        // Add 30 more (2 mandatory already present) = 32 total
        for i in 0..30 {
            let fake_cat = format!("FAKE_{i}");
            subs.entries
                .insert(fake_cat.clone(), Subscription::new(fake_cat));
        }
        assert_eq!(subs.count(), 32);

        // 33rd should be denied
        let outcome = subs.apply_change(&subscribe(&[CATEGORY_SCENE_TOPOLOGY]), &[], &caps(&[]));
        assert_eq!(outcome.denied, vec![CATEGORY_SCENE_TOPOLOGY.to_string()]);
    }

    // ── should_receive / event routing ────────────────────────────────────────

    #[test]
    fn test_unsubscribed_events_not_delivered() {
        let subs = AgentSubscriptions::new();
        // Only mandatory categories subscribed; scene.* events should not be delivered
        assert!(!subs.should_receive("scene.tile.created"));
        assert!(!subs.should_receive("scene.tab.created"));
        assert!(!subs.should_receive("agent.my_agent.something"));
        assert!(!subs.should_receive("input.pointer_down"));
    }

    #[test]
    fn test_subscribed_events_delivered() {
        let mut subs = AgentSubscriptions::new();
        subs.apply_change(&subscribe(&[CATEGORY_SCENE_TOPOLOGY]), &[], &caps(&[]));
        assert!(subs.should_receive("scene.tile.created"));
        assert!(subs.should_receive("scene.tab.created"));
        assert!(subs.should_receive("scene.zone.occupancy_changed")); // scene.* matches
    }

    #[test]
    fn test_mandatory_events_always_received() {
        let subs = AgentSubscriptions::new();
        assert!(subs.should_receive("system.lease_revoked"));
        assert!(subs.should_receive("system.lease_granted"));
        assert!(subs.should_receive("system.degradation_changed"));
        assert!(subs.should_receive("system.degradation_level_changed"));
    }

    // ── Fine-grained prefix filter (spec line 179) ────────────────────────────

    #[test]
    fn test_zone_filter_within_scene_topology() {
        let subs = AgentSubscriptions {
            entries: {
                let mut map = HashMap::new();
                // Add mandatory categories
                for cat in MANDATORY_CATEGORIES {
                    map.insert(cat.to_string(), Subscription::new(*cat));
                }
                // Add SCENE_TOPOLOGY with finer filter
                map.insert(
                    CATEGORY_SCENE_TOPOLOGY.to_string(),
                    Subscription::with_filter(CATEGORY_SCENE_TOPOLOGY, "scene.zone."),
                );
                map
            },
        };

        // Must receive ZoneOccupancyChanged (matches "scene.zone.")
        assert!(subs.should_receive("scene.zone.occupancy_changed"));
        // Must NOT receive TileCreated (does not match "scene.zone.")
        assert!(!subs.should_receive("scene.tile.created"));
        // Must NOT receive ActiveTabChanged
        assert!(!subs.should_receive("scene.tab.active_changed"));
    }

    // ── filter_prefix persisted via apply_change (spec line 179) ─────────────

    #[test]
    fn test_apply_change_persists_filter_prefix() {
        // Agents MUST be able to subscribe to a category with a finer-grained
        // filter_prefix via the change API, not just via direct struct construction.
        let mut subs = AgentSubscriptions::new();
        let outcome = subs.apply_change(
            &subscribe_with_filter(CATEGORY_SCENE_TOPOLOGY, "scene.zone."),
            &[],
            &caps(&[]),
        );
        assert!(outcome.denied.is_empty(), "subscription should be granted");
        assert!(
            outcome
                .active
                .contains(&CATEGORY_SCENE_TOPOLOGY.to_string())
        );

        // Filter must be persisted: only zone events should be delivered
        assert!(
            subs.should_receive("scene.zone.occupancy_changed"),
            "zone event must match scene.zone. filter"
        );
        assert!(
            !subs.should_receive("scene.tile.created"),
            "tile event must NOT match scene.zone. filter"
        );
        assert!(
            !subs.should_receive("scene.tab.active_changed"),
            "tab event must NOT match scene.zone. filter"
        );
    }

    #[test]
    fn test_apply_change_filter_prefix_update_in_place() {
        // Subscribing to a category that is already active MUST update its filter.
        let mut subs = AgentSubscriptions::new();
        // First: subscribe without filter (all scene.* events)
        subs.apply_change(&subscribe(&[CATEGORY_SCENE_TOPOLOGY]), &[], &caps(&[]));
        assert!(
            subs.should_receive("scene.tile.created"),
            "no filter: tile events expected"
        );

        // Second: re-subscribe with a narrower filter
        subs.apply_change(
            &subscribe_with_filter(CATEGORY_SCENE_TOPOLOGY, "scene.zone."),
            &[],
            &caps(&[]),
        );
        assert!(
            !subs.should_receive("scene.tile.created"),
            "after filter update: tile events must be excluded"
        );
        assert!(
            subs.should_receive("scene.zone.occupancy_changed"),
            "after filter update: zone events must still be delivered"
        );
    }

    #[test]
    fn test_apply_change_no_filter_prefix_uses_default() {
        // A subscription with no filter_prefix must use the category default.
        let mut subs = AgentSubscriptions::new();
        subs.apply_change(&subscribe(&[CATEGORY_SCENE_TOPOLOGY]), &[], &caps(&[]));
        // With no filter, all scene.* events arrive
        assert!(subs.should_receive("scene.tile.created"));
        assert!(subs.should_receive("scene.tab.active_changed"));
        assert!(subs.should_receive("scene.zone.occupancy_changed"));
    }

    #[test]
    fn test_apply_change_filter_prefix_capability_denied() {
        // filter_prefix on a category that requires capability must still be denied
        // without the capability.
        let mut subs = AgentSubscriptions::new();
        let outcome = subs.apply_change(
            &subscribe_with_filter(CATEGORY_INPUT_EVENTS, "input.pointer"),
            &[],
            &caps(&[]),
        );
        assert!(
            outcome.denied.contains(&CATEGORY_INPUT_EVENTS.to_string()),
            "capability check must still apply when filter_prefix is set"
        );
    }

    #[test]
    fn test_registry_apply_change_persists_filter_prefix() {
        // SubscriptionRegistry::apply_change must also thread filter_prefix through.
        let mut registry = SubscriptionRegistry::new();
        registry.register("agent_x");
        registry.apply_change(
            "agent_x",
            &subscribe_with_filter(CATEGORY_SCENE_TOPOLOGY, "scene.zone."),
            &[],
            &caps(&[]),
        );
        let agent_subs = registry.get("agent_x").unwrap();
        // Only zone events should reach agent_x
        assert!(agent_subs.should_receive("scene.zone.occupancy_changed"));
        assert!(!agent_subs.should_receive("scene.tile.created"));
    }

    #[test]
    fn test_filter_prefix_must_stay_within_category_default_prefix() {
        // A filter_prefix that escapes the category's allowed prefix boundary must be denied.
        // SCENE_TOPOLOGY has default prefix "scene."; "system." is outside that boundary.
        let mut subs = AgentSubscriptions::new();
        let outcome = subs.apply_change(
            &subscribe_with_filter(CATEGORY_SCENE_TOPOLOGY, "system."),
            &[],
            &caps(&[]),
        );
        assert!(
            outcome
                .denied
                .contains(&CATEGORY_SCENE_TOPOLOGY.to_string()),
            "filter_prefix outside category boundary must be denied"
        );
        // Category must not be subscribed
        assert!(
            !outcome
                .active
                .contains(&CATEGORY_SCENE_TOPOLOGY.to_string()),
            "subscription must not be active after denied filter_prefix"
        );
    }

    #[test]
    fn test_filter_prefix_reset_via_none_clears_stored_filter() {
        // Sending a None filter_prefix for an already-subscribed category must reset
        // the filter to the category default (i.e., clear any stored filter_prefix).
        let mut subs = AgentSubscriptions::new();
        // Subscribe with a narrow filter first
        subs.apply_change(
            &subscribe_with_filter(CATEGORY_SCENE_TOPOLOGY, "scene.zone."),
            &[],
            &caps(&[]),
        );
        assert!(
            !subs.should_receive("scene.tile.created"),
            "narrow filter: tile events must be excluded"
        );

        // Re-subscribe with no filter (None) — should reset to default
        subs.apply_change(&subscribe(&[CATEGORY_SCENE_TOPOLOGY]), &[], &caps(&[]));
        assert!(
            subs.should_receive("scene.tile.created"),
            "after filter reset: tile events must be delivered"
        );
        assert!(
            subs.should_receive("scene.zone.occupancy_changed"),
            "after filter reset: zone events must also be delivered"
        );
    }

    // ── Dual-routing deduplication ────────────────────────────────────────────

    #[test]
    fn test_zone_occupancy_dual_routing_returns_multiple_categories() {
        let mut subs = AgentSubscriptions::new();
        subs.apply_change(
            &subscribe(&[CATEGORY_SCENE_TOPOLOGY, CATEGORY_ZONE_EVENTS]),
            &[],
            &caps(&[]),
        );

        // Both categories match "scene.zone.occupancy_changed"
        let matched = subs.matched_categories("scene.zone.occupancy_changed");
        assert!(matched.contains(&CATEGORY_SCENE_TOPOLOGY.to_string()));
        assert!(matched.contains(&CATEGORY_ZONE_EVENTS.to_string()));
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_should_receive_deduplicates_to_single_delivery() {
        let mut subs = AgentSubscriptions::new();
        subs.apply_change(
            &subscribe(&[CATEGORY_SCENE_TOPOLOGY, CATEGORY_ZONE_EVENTS]),
            &[],
            &caps(&[]),
        );

        // should_receive returns bool (not count) — agent gets the event exactly once
        assert!(subs.should_receive("scene.zone.occupancy_changed"));
        // Verify the caller is responsible for dedup by checking matched_categories length
        // (The event_bus dedup layer uses this to ensure single delivery)
        let matched = subs.matched_categories("scene.zone.occupancy_changed");
        assert_eq!(
            matched.len(),
            2,
            "two categories match — dedup layer must collapse to one delivery"
        );
    }

    // ── Registry ──────────────────────────────────────────────────────────────

    #[test]
    fn test_registry_register_and_remove() {
        let mut registry = SubscriptionRegistry::new();
        registry.register("agent_a");
        registry.register("agent_b");

        let subs_a = registry.get("agent_a").unwrap();
        assert!(
            subs_a
                .active_categories()
                .contains(&CATEGORY_LEASE_CHANGES.to_string())
        );

        registry.remove("agent_a");
        assert!(registry.get("agent_a").is_none());
    }

    #[test]
    fn test_registry_subscribers_for_event() {
        let mut registry = SubscriptionRegistry::new();
        registry.register("agent_a");
        registry.register("agent_b");

        registry.apply_change(
            "agent_a",
            &subscribe(&[CATEGORY_SCENE_TOPOLOGY]),
            &[],
            &caps(&[]),
        );
        // agent_b has no SCENE_TOPOLOGY subscription

        let subs = registry.subscribers_for_event("scene.tile.created");
        assert!(subs.contains("agent_a"));
        assert!(!subs.contains("agent_b"));

        // Both agents should receive lease events (mandatory)
        let lease_subs = registry.subscribers_for_event("system.lease_revoked");
        assert!(lease_subs.contains("agent_a"));
        assert!(lease_subs.contains("agent_b"));
    }

    // ── Unknown category ──────────────────────────────────────────────────────

    #[test]
    fn test_unknown_category_denied() {
        let mut subs = AgentSubscriptions::new();
        let outcome = subs.apply_change(&subscribe(&["TOTALLY_UNKNOWN_CATEGORY"]), &[], &caps(&[]));
        assert!(
            outcome
                .denied
                .contains(&"TOTALLY_UNKNOWN_CATEGORY".to_string())
        );
    }

    // ── Subscription accessor methods ─────────────────────────────────────────

    #[test]
    fn test_subscription_accessors_no_filter() {
        let sub = Subscription::new(CATEGORY_SCENE_TOPOLOGY);
        assert_eq!(sub.category(), CATEGORY_SCENE_TOPOLOGY);
        assert_eq!(sub.filter_prefix(), None);
    }

    #[test]
    fn test_subscription_accessors_with_filter() {
        let sub = Subscription::with_filter(CATEGORY_SCENE_TOPOLOGY, "scene.zone.");
        assert_eq!(sub.category(), CATEGORY_SCENE_TOPOLOGY);
        assert_eq!(sub.filter_prefix(), Some("scene.zone."));
    }

    // ── with_filter invariant enforcement ────────────────────────────────────

    #[test]
    #[should_panic(expected = "does not start with category")]
    fn test_with_filter_panics_on_out_of_bounds_prefix() {
        // "system." does not start with the SCENE_TOPOLOGY default prefix "scene."
        let _ = Subscription::with_filter(CATEGORY_SCENE_TOPOLOGY, "system.");
    }

    #[test]
    #[should_panic(expected = "does not start with category")]
    fn test_with_filter_panics_on_completely_unrelated_prefix() {
        // "input." is entirely outside "agent." namespace
        let _ = Subscription::with_filter(CATEGORY_AGENT_EVENTS, "input.");
    }

    #[test]
    fn test_with_filter_accepts_valid_narrowing_prefix() {
        // "scene.zone." starts with "scene." (SCENE_TOPOLOGY default) — valid
        let sub = Subscription::with_filter(CATEGORY_SCENE_TOPOLOGY, "scene.zone.");
        assert_eq!(sub.filter_prefix(), Some("scene.zone."));
    }

    #[test]
    fn test_with_filter_accepts_exact_category_prefix() {
        // Using the exact category prefix as filter is valid (no narrowing, but allowed)
        let sub = Subscription::with_filter(CATEGORY_SCENE_TOPOLOGY, "scene.");
        assert_eq!(sub.filter_prefix(), Some("scene."));
    }

    #[test]
    fn test_with_filter_unknown_category_allows_any_prefix() {
        // Unknown categories have no enforced prefix (no category_prefix entry)
        // so any filter is accepted without panic.
        let sub = Subscription::with_filter("UNKNOWN_CAT", "anything.");
        assert_eq!(sub.category(), "UNKNOWN_CAT");
        assert_eq!(sub.filter_prefix(), Some("anything."));
    }
}
