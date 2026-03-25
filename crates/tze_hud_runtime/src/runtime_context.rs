//! # RuntimeContext
//!
//! Immutable runtime context built at startup from the validated configuration.
//!
//! ## Purpose
//!
//! `RuntimeContext` is the **single source of truth** for configuration-derived
//! runtime parameters. It is built once from a `ResolvedConfig` and then shared
//! (via `Arc`) across all runtime subsystems.
//!
//! The following configuration dimensions are surfaced:
//!
//! - **Profile budgets** — max tiles, max texture MB, max agents, target/min FPS.
//! - **Agent capability registry** — per-agent capability grants from `[agents.registered]`.
//!
//! ## Configuration-Driven Capability Gating
//!
//! Per configuration/spec.md §Requirement: Agent Registration with Per-Agent Budget
//! Overrides (lines 136-147), each agent entry in `[agents.registered]` carries an
//! explicit capability list. `RuntimeContext::capability_policy_for` returns the
//! appropriate `CapabilityPolicy` for a given agent name:
//!
//! - **Registered agent**: policy built from that agent's listed capabilities.
//! - **Unknown agent**: falls back to the `fallback_policy` (configurable; default `guest`).
//!
//! This replaces the v0 `CapabilityPolicy::unrestricted()` sentinel used for PSK
//! sessions, which granted `"*"` (all capabilities) to any authenticated agent.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tze_hud_runtime::RuntimeContext;
//! use tze_hud_scene::config::ResolvedConfig;
//!
//! let ctx = RuntimeContext::from_config(resolved_config);
//! let policy = ctx.capability_policy_for("my-agent");
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use tze_hud_protocol::auth::CapabilityPolicy;
use tze_hud_scene::config::{DisplayProfile, ResolvedConfig};

// ─── FallbackPolicy ───────────────────────────────────────────────────────────

/// What capability policy to apply to agents not listed in `[agents.registered]`.
///
/// - `Guest` — no capabilities granted (safest default for v1).
/// - `Unrestricted` — all capabilities granted (useful for single-agent dev setups).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackPolicy {
    /// Unknown agents receive no capabilities (guest mode).
    Guest,
    /// Unknown agents receive unrestricted capabilities (dev mode only).
    Unrestricted,
}

impl Default for FallbackPolicy {
    fn default() -> Self {
        Self::Guest
    }
}

// ─── RuntimeContext ───────────────────────────────────────────────────────────

/// Immutable runtime context derived from validated configuration.
///
/// Built once at startup; shared via `Arc<RuntimeContext>` across all subsystems.
///
/// All fields are read-only after construction. Hot-reload is a post-v1 concern;
/// for v1 a restart is required to pick up config changes.
#[derive(Debug)]
pub struct RuntimeContext {
    /// Resolved display profile with budget values.
    pub profile: DisplayProfile,

    /// Per-agent capability grants keyed by agent name.
    /// Populated from `[agents.registered]` in config.
    agent_capabilities: HashMap<String, Vec<String>>,

    /// Policy to apply to agents not listed in `[agents.registered]`.
    pub fallback_policy: FallbackPolicy,
}

impl RuntimeContext {
    // ── Constructors ─────────────────────────────────────────────────────────

    /// Build a `RuntimeContext` from a fully validated `ResolvedConfig`.
    ///
    /// The `fallback_policy` is applied to any agent whose name is not found
    /// in `[agents.registered]`. For v1 production use, pass `FallbackPolicy::Guest`.
    pub fn from_config(config: ResolvedConfig, fallback_policy: FallbackPolicy) -> Self {
        Self {
            profile: config.profile,
            agent_capabilities: config.agent_capabilities,
            fallback_policy,
        }
    }

    /// Build a minimal `RuntimeContext` using the headless profile defaults.
    ///
    /// Used in tests and headless mode when no config file is present.
    /// All unrecognized agents are treated as guests (no capabilities).
    pub fn headless_default() -> Self {
        Self {
            profile: DisplayProfile::headless(),
            agent_capabilities: HashMap::new(),
            fallback_policy: FallbackPolicy::Guest,
        }
    }

    // ── Capability policy lookup ──────────────────────────────────────────────

    /// Return the `CapabilityPolicy` for the given agent name.
    ///
    /// Per configuration/spec.md §Requirement: Agent Registration (lines 136-147):
    /// - Registered agents receive their configured capability set.
    /// - Unregistered agents receive the fallback policy (default: guest).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let policy = ctx.capability_policy_for("weather-agent");
    /// let (granted, denied) = policy.partition_capabilities(&requested);
    /// ```
    pub fn capability_policy_for(&self, agent_name: &str) -> CapabilityPolicy {
        match self.agent_capabilities.get(agent_name) {
            Some(caps) => CapabilityPolicy::new(caps.clone()),
            None => match self.fallback_policy {
                FallbackPolicy::Guest => CapabilityPolicy::guest(),
                FallbackPolicy::Unrestricted => CapabilityPolicy::unrestricted(),
            },
        }
    }

    /// Return the registered capability list for the given agent, if any.
    ///
    /// Returns `None` if the agent is not listed in `[agents.registered]`.
    pub fn agent_capabilities(&self, agent_name: &str) -> Option<&[String]> {
        self.agent_capabilities.get(agent_name).map(Vec::as_slice)
    }

    /// Return a cloned snapshot of the full agent capability registry.
    ///
    /// Used at server startup to wire the capability map into `HudSessionImpl`.
    /// This is a one-time clone at startup, not on the per-request hot path.
    pub fn snapshot_agent_capabilities(&self) -> HashMap<String, Vec<String>> {
        self.agent_capabilities.clone()
    }
}

// ─── Shared runtime context type alias ───────────────────────────────────────

/// Cheaply-cloneable handle to the shared immutable runtime context.
pub type SharedRuntimeContext = Arc<RuntimeContext>;

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tze_hud_scene::config::ResolvedConfig;

    fn make_config(caps: Vec<(&str, Vec<&str>)>) -> ResolvedConfig {
        let mut agent_capabilities = HashMap::new();
        for (name, agent_caps) in caps {
            agent_capabilities.insert(
                name.to_string(),
                agent_caps.into_iter().map(str::to_string).collect(),
            );
        }
        ResolvedConfig {
            profile: DisplayProfile::headless(),
            tab_names: vec!["main".to_string()],
            agent_capabilities,
            source_path: None,
        }
    }

    // ── from_config ───────────────────────────────────────────────────────────

    #[test]
    fn from_config_populates_profile() {
        let config = make_config(vec![]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        assert_eq!(ctx.profile.name, "headless");
        assert_eq!(ctx.profile.max_tiles, 256);
    }

    #[test]
    fn from_config_populates_agent_capabilities() {
        let config = make_config(vec![
            ("weather-agent", vec!["create_tiles", "modify_own_tiles"]),
        ]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let caps = ctx.agent_capabilities("weather-agent").unwrap();
        assert!(caps.contains(&"create_tiles".to_string()));
        assert!(caps.contains(&"modify_own_tiles".to_string()));
    }

    // ── headless_default ─────────────────────────────────────────────────────

    #[test]
    fn headless_default_has_headless_profile() {
        let ctx = RuntimeContext::headless_default();
        assert_eq!(ctx.profile.name, "headless");
    }

    #[test]
    fn headless_default_returns_guest_for_unknown_agent() {
        let ctx = RuntimeContext::headless_default();
        let policy = ctx.capability_policy_for("any-agent");
        // Guest policy: no capabilities
        let result = policy.evaluate_capability_request(&["create_tiles".to_string()]);
        assert!(result.is_err(), "guest policy should deny all capabilities");
    }

    // ── capability_policy_for ─────────────────────────────────────────────────

    #[test]
    fn registered_agent_gets_configured_capabilities() {
        let config = make_config(vec![
            ("agent-a", vec!["create_tiles", "read_scene_topology"]),
        ]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let policy = ctx.capability_policy_for("agent-a");
        let result = policy.evaluate_capability_request(&["create_tiles".to_string()]);
        assert!(result.is_ok(), "registered agent should be granted create_tiles");
        let result = policy.evaluate_capability_request(&["overlay_privileges".to_string()]);
        assert!(result.is_err(), "registered agent should be denied unconfigured capability");
    }

    #[test]
    fn unregistered_agent_gets_guest_policy_by_default() {
        let config = make_config(vec![]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let policy = ctx.capability_policy_for("unknown-agent");
        assert!(!policy.is_unrestricted());
        let result = policy.evaluate_capability_request(&["create_tiles".to_string()]);
        assert!(result.is_err(), "unregistered agent should be denied under guest fallback");
    }

    #[test]
    fn unregistered_agent_gets_unrestricted_under_dev_fallback() {
        let config = make_config(vec![]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Unrestricted);
        let policy = ctx.capability_policy_for("unknown-agent");
        assert!(policy.is_unrestricted());
    }

    #[test]
    fn multiple_agents_get_independent_policies() {
        let config = make_config(vec![
            ("agent-a", vec!["create_tiles"]),
            ("agent-b", vec!["read_telemetry"]),
        ]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);

        // agent-a can create tiles but not read telemetry
        let policy_a = ctx.capability_policy_for("agent-a");
        assert!(policy_a.evaluate_capability_request(&["create_tiles".to_string()]).is_ok());
        assert!(policy_a.evaluate_capability_request(&["read_telemetry".to_string()]).is_err());

        // agent-b can read telemetry but not create tiles
        let policy_b = ctx.capability_policy_for("agent-b");
        assert!(policy_b.evaluate_capability_request(&["read_telemetry".to_string()]).is_ok());
        assert!(policy_b.evaluate_capability_request(&["create_tiles".to_string()]).is_err());
    }

    // ── Spec: Agent Registration with Per-Agent Budget Overrides ─────────────
    // configuration/spec.md lines 136-147

    #[test]
    fn config_registered_agent_caps_replace_psk_unrestricted_sentinel() {
        // This test encodes the core invariant: after wiring config,
        // PSK auth no longer implies unrestricted "*" — only registered
        // agents get their listed capabilities.
        let config = make_config(vec![
            ("my-agent", vec!["create_tiles", "modify_own_tiles", "access_input_events"]),
        ]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let policy = ctx.capability_policy_for("my-agent");

        // Should NOT be unrestricted
        assert!(!policy.is_unrestricted(), "config-registered agent policy must not be unrestricted");

        // Should allow exactly the listed capabilities
        assert!(policy.evaluate_capability_request(&["create_tiles".to_string()]).is_ok());
        assert!(policy.evaluate_capability_request(&["modify_own_tiles".to_string()]).is_ok());
        assert!(policy.evaluate_capability_request(&["access_input_events".to_string()]).is_ok());
        assert!(policy.evaluate_capability_request(&["overlay_privileges".to_string()]).is_err());
    }
}
