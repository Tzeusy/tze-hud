//! # tab_switch_on_event Trigger Logic
//!
//! Implements the `tab_switch_on_event` contract per
//! scene-events/spec.md §9.1–§9.4, lines 235-251:
//!
//! > The runtime SHALL support a `tab_switch_on_event` configuration field per tab
//! > that names a scene-level event triggering automatic tab activation. Agent events
//! > SHALL match against the bare event name (before namespace prefixing) for
//! > agent-independence. System events SHALL NOT trigger tab switches. The triggered
//! > tab switch SHALL be subject to attention filtering (quiet hours, attention budget).
//! > A successful switch SHALL generate an ActiveTabChangedEvent.
//!
//! ## Design
//!
//! This module provides the **matching and switching logic** in isolation from the
//! full event bus. The runtime integrates this by:
//!
//! 1. After an agent event is accepted by `AgentEventHandler::handle`, calling
//!    `TabSwitchTrigger::evaluate`.
//! 2. If a tab match is found, the trigger applies attention filtering via the
//!    injected [`AttentionGate`] and, if allowed, switches the active tab.
//! 3. On a successful switch an `ActiveTabChangedEvent` is emitted.
//!
//! ## Key rules
//!
//! - Match is against the **bare name** (before namespace prefix) for agent-independence.
//! - `system.*` events are **excluded** from triggering tab switches (spec line 250).
//! - The tab switch is **subject to attention filtering** (spec line 246).
//! - A successful switch generates `ActiveTabChangedEvent` (event_type
//!   `"scene.tab.active_changed"`) (spec line 236).

use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::types::SceneId;

// ─── Attention gate trait ─────────────────────────────────────────────────────

/// Controls whether a tab switch is allowed through the attention filter.
///
/// In v1 the gate is wired to the quiet-hours and attention budget enforcer
/// (bead #3). Tests and partial-integration code may inject a pass-through
/// implementation.
pub trait AttentionGate: Send + Sync {
    /// Returns `true` if the tab switch should proceed immediately, `false`
    /// if it should be deferred (e.g. quiet hours active for NORMAL class).
    ///
    /// # Parameters
    ///
    /// - `bare_name`: the bare event name that triggered the tab switch.
    fn allow_tab_switch(&self, bare_name: &str) -> bool;
}

/// A pass-through `AttentionGate` that always allows tab switches.
///
/// Used when no attention enforcement is wired up (tests, headless mode
/// without policy enforcement).
pub struct PermissiveGate;

impl AttentionGate for PermissiveGate {
    fn allow_tab_switch(&self, _bare_name: &str) -> bool {
        true
    }
}

/// An `AttentionGate` that always defers (blocks) tab switches.
///
/// Useful for testing quiet-hours deferral without a full policy engine.
pub struct BlockingGate;

impl AttentionGate for BlockingGate {
    fn allow_tab_switch(&self, _bare_name: &str) -> bool {
        false
    }
}

// ─── Switch outcome ───────────────────────────────────────────────────────────

/// Outcome of a [`TabSwitchTrigger::evaluate`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabSwitchOutcome {
    /// No tab is configured for this bare name — no action taken.
    NoMatchingTab,
    /// A matching tab was found but system events are excluded (spec line 250).
    ExcludedSystemEvent,
    /// A matching tab was found but the attention gate blocked the switch
    /// (e.g. quiet hours active, spec line 246).
    Deferred { tab_id: SceneId },
    /// Tab switch succeeded. Caller must emit `ActiveTabChangedEvent`.
    Switched { tab_id: SceneId, previous_tab_id: Option<SceneId> },
}

// ─── Trigger ─────────────────────────────────────────────────────────────────

/// Evaluates `tab_switch_on_event` trigger conditions and performs the switch.
///
/// Holds a reference to the scene graph (mutable for switching) and the
/// injected attention gate.
///
/// # Thread safety
///
/// This struct is not `Sync`. The caller is responsible for holding the scene
/// graph lock during `evaluate`.
pub struct TabSwitchTrigger<G: AttentionGate> {
    gate: G,
}

impl<G: AttentionGate> TabSwitchTrigger<G> {
    /// Create a new trigger with the given attention gate.
    pub fn new(gate: G) -> Self {
        Self { gate }
    }

    /// Evaluate a bare event name against all tabs' `tab_switch_on_event` config.
    ///
    /// # Arguments
    ///
    /// - `scene`: mutable scene graph (mutated if a tab switch occurs).
    /// - `bare_name`: the raw bare name emitted by the agent (before prefixing).
    ///   Must already be validated by `AgentEventHandler`.
    ///
    /// # Returns
    ///
    /// A [`TabSwitchOutcome`] describing what happened.
    ///
    /// # System event exclusion
    ///
    /// If `bare_name` starts with `"system."`, the method returns
    /// [`TabSwitchOutcome::ExcludedSystemEvent`] immediately without searching
    /// for a matching tab (spec line 250). Note: `bare_name` should have already
    /// been validated and will never start with `"system."` after passing through
    /// `AgentEventHandler` — this is a defence-in-depth check.
    pub fn evaluate(&self, scene: &mut SceneGraph, bare_name: &str) -> TabSwitchOutcome {
        // System events are excluded from tab_switch_on_event matching (spec line 250).
        if bare_name.starts_with("system.") {
            return TabSwitchOutcome::ExcludedSystemEvent;
        }

        // Find the first tab configured with this bare name.
        let target_tab_id = match scene.find_tab_for_event(bare_name) {
            Some(id) => id,
            None => return TabSwitchOutcome::NoMatchingTab,
        };

        // Check attention gate (quiet hours, attention budget) — spec line 246.
        if !self.gate.allow_tab_switch(bare_name) {
            return TabSwitchOutcome::Deferred { tab_id: target_tab_id };
        }

        // Perform the switch.
        let previous_tab_id = scene.active_tab;
        scene
            .switch_active_tab(target_tab_id)
            .expect("tab found but switch_active_tab failed — internal invariant violated");

        TabSwitchOutcome::Switched {
            tab_id: target_tab_id,
            previous_tab_id,
        }
    }
}

// ─── Event type constant ──────────────────────────────────────────────────────

/// Event type generated by a successful tab switch triggered by tab_switch_on_event.
///
/// Spec: scene-events/spec.md line 236 — "A successful switch SHALL generate an
/// ActiveTabChangedEvent (event_type 'scene.tab.active_changed')."
pub const ACTIVE_TAB_CHANGED_EVENT_TYPE: &str = "scene.tab.active_changed";

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::graph::SceneGraph;

    fn make_graph() -> SceneGraph {
        SceneGraph::new(1920.0, 1080.0)
    }

    // ── Spec scenario: Agent-independent tab switch (spec lines 240-242) ─────

    /// WHEN a tab is configured with `tab_switch_on_event = "doorbell.ring"` and
    /// any agent with the capability emits "doorbell.ring" THEN the runtime MUST
    /// switch to that tab (subject to attention filtering) regardless of which
    /// agent emitted the event (spec line 242).
    #[test]
    fn tab_switch_fires_on_matching_bare_name() {
        let mut graph = make_graph();
        let tab_a = graph.create_tab("TabA", 0).unwrap();
        let tab_b = graph.create_tab("TabB", 1).unwrap();

        graph
            .set_tab_switch_on_event(tab_b, Some("doorbell.ring".to_string()))
            .unwrap();

        // Active tab starts as tab_a (first created).
        assert_eq!(graph.active_tab, Some(tab_a));

        let trigger = TabSwitchTrigger::new(PermissiveGate);
        let outcome = trigger.evaluate(&mut graph, "doorbell.ring");

        assert_eq!(
            outcome,
            TabSwitchOutcome::Switched {
                tab_id: tab_b,
                previous_tab_id: Some(tab_a),
            }
        );
        assert_eq!(graph.active_tab, Some(tab_b), "active tab must switch to tab_b");
    }

    /// Agent-independence: two different bare-name namespaces map to the same
    /// bare name — both trigger the same tab switch.
    #[test]
    fn tab_switch_is_agent_independent() {
        let mut graph = make_graph();
        let tab_a = graph.create_tab("TabA", 0).unwrap();
        let tab_b = graph.create_tab("TabB", 1).unwrap();
        graph
            .set_tab_switch_on_event(tab_b, Some("doorbell.ring".to_string()))
            .unwrap();

        // Reset to tab_a as active.
        graph.switch_active_tab(tab_a).unwrap();

        let trigger = TabSwitchTrigger::new(PermissiveGate);

        // "doorbell_agent_1" emits "doorbell.ring" — switch to tab_b.
        let r1 = trigger.evaluate(&mut graph, "doorbell.ring");
        assert!(matches!(r1, TabSwitchOutcome::Switched { tab_id, .. } if tab_id == tab_b));

        // Reset.
        graph.switch_active_tab(tab_a).unwrap();

        // "doorbell_agent_2" also emits "doorbell.ring" — also switches tab_b.
        let r2 = trigger.evaluate(&mut graph, "doorbell.ring");
        assert!(matches!(r2, TabSwitchOutcome::Switched { tab_id, .. } if tab_id == tab_b));
    }

    // ── No matching tab ──────────────────────────────────────────────────────

    #[test]
    fn no_matching_tab_returns_no_match() {
        let mut graph = make_graph();
        let _ = graph.create_tab("TabA", 0).unwrap();

        let trigger = TabSwitchTrigger::new(PermissiveGate);
        assert_eq!(
            trigger.evaluate(&mut graph, "unknown.event"),
            TabSwitchOutcome::NoMatchingTab
        );
    }

    // ── Spec scenario: Quiet hours deferral (spec lines 244-246) ─────────────

    /// WHEN quiet hours are active and a tab_switch_on_event fires with NORMAL
    /// interruption class THEN the tab switch MUST be deferred until quiet hours
    /// end (spec line 246).
    #[test]
    fn tab_switch_deferred_when_gate_blocks() {
        let mut graph = make_graph();
        let tab_a = graph.create_tab("TabA", 0).unwrap();
        let tab_b = graph.create_tab("TabB", 1).unwrap();
        graph
            .set_tab_switch_on_event(tab_b, Some("doorbell.ring".to_string()))
            .unwrap();
        graph.switch_active_tab(tab_a).unwrap();

        let trigger = TabSwitchTrigger::new(BlockingGate);
        let outcome = trigger.evaluate(&mut graph, "doorbell.ring");

        assert_eq!(
            outcome,
            TabSwitchOutcome::Deferred { tab_id: tab_b },
            "switch must be deferred when gate blocks"
        );
        // Active tab must NOT have changed.
        assert_eq!(graph.active_tab, Some(tab_a));
    }

    // ── Spec scenario: System events excluded (spec lines 248-250) ───────────

    /// WHEN a tab is configured with `tab_switch_on_event = "system.degradation_changed"`
    /// THEN the runtime MUST NOT trigger a tab switch from system events; the
    /// system.* prefix is excluded from tab_switch_on_event matching (spec line 250).
    #[test]
    fn system_event_bare_name_excluded() {
        let mut graph = make_graph();
        let tab_a = graph.create_tab("TabA", 0).unwrap();
        let tab_b = graph.create_tab("TabB", 1).unwrap();
        // Although the runtime should reject system.* tab_switch_on_event values at
        // config time, this verifies the defence-in-depth exclusion in evaluate().
        graph.tabs.get_mut(&tab_b).unwrap().tab_switch_on_event =
            Some("system.degradation_changed".to_string());
        graph.switch_active_tab(tab_a).unwrap();

        let trigger = TabSwitchTrigger::new(PermissiveGate);
        let outcome = trigger.evaluate(&mut graph, "system.degradation_changed");

        assert_eq!(
            outcome,
            TabSwitchOutcome::ExcludedSystemEvent,
            "system events must not trigger tab switches"
        );
        assert_eq!(graph.active_tab, Some(tab_a));
    }

    // ── ActiveTabChangedEvent type constant ──────────────────────────────────

    #[test]
    fn active_tab_changed_event_type_is_correct() {
        assert_eq!(ACTIVE_TAB_CHANGED_EVENT_TYPE, "scene.tab.active_changed");
    }

    // ── find_tab_for_event on SceneGraph ──────────────────────────────────────

    /// Verify `SceneGraph::find_tab_for_event` excludes system.* tab_switch_on_event
    /// values at the graph level too.
    #[test]
    fn find_tab_for_event_excludes_system_prefix() {
        let mut graph = make_graph();
        let _ = graph.create_tab("TabA", 0).unwrap();
        let tab_b = graph.create_tab("TabB", 1).unwrap();

        // Directly set an illegal system.* value on the tab to test the graph's
        // defensive exclusion.
        graph.tabs.get_mut(&tab_b).unwrap().tab_switch_on_event =
            Some("system.degradation_changed".to_string());

        // find_tab_for_event must NOT return tab_b for a system.* event name.
        assert_eq!(
            graph.find_tab_for_event("system.degradation_changed"),
            None
        );
    }

    /// When tab_switch_on_event is None, the tab is not matched.
    #[test]
    fn tab_with_no_event_not_matched() {
        let mut graph = make_graph();
        let _ = graph.create_tab("TabA", 0).unwrap();
        // No tab has tab_switch_on_event set.
        assert_eq!(graph.find_tab_for_event("doorbell.ring"), None);
    }

    // ── set_tab_switch_on_event graph API ─────────────────────────────────────

    #[test]
    fn set_tab_switch_on_event_persists() {
        let mut graph = make_graph();
        let tab_id = graph.create_tab("Main", 0).unwrap();

        graph
            .set_tab_switch_on_event(tab_id, Some("doorbell.ring".to_string()))
            .unwrap();
        assert_eq!(
            graph.tabs[&tab_id].tab_switch_on_event,
            Some("doorbell.ring".to_string())
        );

        // Clear it.
        graph.set_tab_switch_on_event(tab_id, None).unwrap();
        assert_eq!(graph.tabs[&tab_id].tab_switch_on_event, None);
    }

    #[test]
    fn set_tab_switch_on_event_increments_version() {
        let mut graph = make_graph();
        let tab_id = graph.create_tab("Main", 0).unwrap();
        let v_before = graph.version;
        graph
            .set_tab_switch_on_event(tab_id, Some("x.y".to_string()))
            .unwrap();
        assert!(graph.version > v_before);
    }

    #[test]
    fn set_tab_switch_on_event_tab_not_found_returns_error() {
        let mut graph = make_graph();
        let fake_id = tze_hud_scene::types::SceneId::new();
        let result = graph.set_tab_switch_on_event(fake_id, Some("x.y".to_string()));
        assert!(result.is_err());
    }
}
