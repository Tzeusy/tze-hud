//! # RuntimeContext
//!
//! Runtime context built at startup from the validated configuration.
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
//! - **Hot-reloadable policy** — privacy, degradation, chrome, and dynamic agent policy
//!   sections, which can be updated live without restart.
//!
//! ## Two-Tier Configuration Model
//!
//! Per the configuration spec §Configuration Reload (lines 263-274, v1-mandatory),
//! configuration sections are divided into two tiers:
//!
//! | Section                   | Reload tier           |
//! |---------------------------|-----------------------|
//! | `[runtime]`               | Frozen — restart required |
//! | `[[tabs]]`                | Frozen — restart required |
//! | `[agents.registered]`     | Frozen — restart required |
//! | `[privacy]`               | Hot-reloadable via SIGHUP or `ReloadConfig` RPC |
//! | `[degradation]`           | Hot-reloadable via SIGHUP or `ReloadConfig` RPC |
//! | `[chrome]`                | Hot-reloadable via SIGHUP or `ReloadConfig` RPC |
//! | `[agents.dynamic_policy]` | Hot-reloadable via SIGHUP or `ReloadConfig` RPC |
//!
//! ### Frozen fields
//!
//! `profile`, `agent_capabilities`, and `fallback_policy` are frozen at startup.
//! A restart is required to change them.
//!
//! ### Hot-reloadable fields
//!
//! `hot` holds an `Arc<HotReloadableConfig>` stored inside an `ArcSwap`. Call
//! `reload_hot_config()` with a freshly validated `HotReloadableConfig` to atomically
//! replace the live policy without locking any subsystem:
//!
//! ```rust,ignore
//! // In the SIGHUP or ReloadConfig handler:
//! let hot = tze_hud_config::reload_config(&new_toml)?;
//! ctx.reload_hot_config(hot);
//! // All subsystems reading ctx.hot_config() will see the new values
//! // immediately on their next access.
//! ```
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
//! use tze_hud_runtime::{RuntimeContext, FallbackPolicy};
//! use tze_hud_scene::config::ResolvedConfig;
//!
//! let ctx = RuntimeContext::from_config(resolved_config, FallbackPolicy::Guest);
//! let policy = ctx.capability_policy_for("my-agent");
//!
//! // Hot-reload on SIGHUP:
//! let hot = tze_hud_config::reload_config(&new_toml).expect("valid toml");
//! ctx.reload_hot_config(hot);
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use tze_hud_config::HotReloadableConfig;
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

/// Runtime context derived from validated configuration.
///
/// Built once at startup; shared via `Arc<RuntimeContext>` across all subsystems.
///
/// **Frozen fields** (`profile`, `agent_capabilities`, `fallback_policy`) are
/// immutable after construction. A restart is required to change them.
///
/// **Hot-reloadable fields** are held in `hot` as an `ArcSwap<HotReloadableConfig>`.
/// Call `reload_hot_config()` to atomically swap in a freshly validated config subset
/// (privacy, degradation, chrome, dynamic_policy) with no locks and no restart.
///
/// Per spec §Configuration Reload (lines 263-274, v1-mandatory): SIGHUP and the
/// `RuntimeService.ReloadConfig` gRPC call both trigger a live reload of the
/// hot-reloadable sections. The frozen sections require a full process restart.
#[derive(Debug)]
pub struct RuntimeContext {
    // ── Frozen fields ─────────────────────────────────────────────────────────
    // Immutable after construction. Require restart to change.
    /// Resolved display profile with budget values.
    pub profile: DisplayProfile,

    /// Per-agent capability grants keyed by agent name.
    /// Populated from `[agents.registered]` in config.
    agent_capabilities: HashMap<String, Vec<String>>,

    /// Policy to apply to agents not listed in `[agents.registered]`.
    pub fallback_policy: FallbackPolicy,

    // ── Hot-reloadable fields ─────────────────────────────────────────────────
    // Atomically swappable via SIGHUP or ReloadConfig RPC.
    /// Hot-reloadable policy sections: privacy, degradation, chrome,
    /// and agents.dynamic_policy.
    ///
    /// Access the current snapshot via `self.hot.load()`. Update atomically
    /// via `self.reload_hot_config(new_hot)`.
    hot: ArcSwap<HotReloadableConfig>,
}

impl RuntimeContext {
    // ── Constructors ─────────────────────────────────────────────────────────

    /// Build a `RuntimeContext` from a fully validated `ResolvedConfig`.
    ///
    /// The `fallback_policy` is applied to any agent whose name is not found
    /// in `[agents.registered]`. For v1 production use, pass `FallbackPolicy::Guest`.
    ///
    /// Hot-reloadable sections are initialized to defaults (all `None` / empty).
    /// They will be populated on the first SIGHUP or `ReloadConfig` RPC call.
    pub fn from_config(config: ResolvedConfig, fallback_policy: FallbackPolicy) -> Self {
        Self {
            profile: config.profile,
            agent_capabilities: config.agent_capabilities,
            fallback_policy,
            hot: ArcSwap::from_pointee(HotReloadableConfig::default()),
        }
    }

    /// Build a `RuntimeContext` from a `ResolvedConfig` and an initial
    /// `HotReloadableConfig`.
    ///
    /// Use this constructor when a config file is available at startup and you
    /// want the hot-reloadable sections to reflect the initial file contents
    /// immediately, rather than waiting for the first SIGHUP.
    pub fn from_config_with_hot(
        config: ResolvedConfig,
        fallback_policy: FallbackPolicy,
        hot: HotReloadableConfig,
    ) -> Self {
        Self {
            profile: config.profile,
            agent_capabilities: config.agent_capabilities,
            fallback_policy,
            hot: ArcSwap::from_pointee(hot),
        }
    }

    /// Build a minimal `RuntimeContext` using the headless profile defaults.
    ///
    /// Used in tests and headless mode when no config file is present.
    /// All unrecognized agents are treated as guests (no capabilities).
    /// Hot-reloadable sections are initialized to defaults.
    pub fn headless_default() -> Self {
        Self {
            profile: DisplayProfile::headless(),
            agent_capabilities: HashMap::new(),
            fallback_policy: FallbackPolicy::Guest,
            hot: ArcSwap::from_pointee(HotReloadableConfig::default()),
        }
    }

    // ── Hot-reload ────────────────────────────────────────────────────────────

    /// Atomically replace the hot-reloadable configuration sections.
    ///
    /// This is the integration point for SIGHUP and `RuntimeService.ReloadConfig`.
    /// The caller is responsible for calling `tze_hud_config::reload_config()` first
    /// to parse and validate the new TOML; this method only stores the result.
    ///
    /// Subsystems that hold a loaded snapshot (via `ctx.hot.load()`) will see stale
    /// values until their next `load()` call. This is intentional — the swap is
    /// atomic and lock-free; subsystems do not need to coordinate.
    ///
    /// ## What is reloaded
    ///
    /// - `[privacy]` — privacy classification, redaction style, quiet hours.
    /// - `[degradation]` — frame-time and GPU thresholds for degradation steps.
    /// - `[chrome]` — chrome rendering policy.
    /// - `[agents.dynamic_policy]` — whether dynamic agents are allowed and their
    ///   default capabilities.
    ///
    /// ## What is NOT reloaded (frozen, restart required)
    ///
    /// - `[runtime]` / `profile` — display profile and budget values.
    /// - `[[tabs]]` — tab layout and zone configuration.
    /// - `[agents.registered]` — pre-registered agent capability grants.
    pub fn reload_hot_config(&self, new_hot: HotReloadableConfig) {
        self.hot.store(Arc::new(new_hot));
    }

    /// Return a snapshot of the hot-reloadable configuration.
    ///
    /// The returned `Arc` keeps the current `HotReloadableConfig` alive for as long
    /// as there are strong references to it. Use this to access privacy,
    /// degradation, chrome, and dynamic policy settings without exposing the
    /// internal hot-reload mechanism.
    ///
    /// ```rust,ignore
    /// let hot = ctx.hot_config();
    /// let privacy = &hot.privacy;
    /// ```
    pub fn hot_config(&self) -> Arc<HotReloadableConfig> {
        self.hot.load_full()
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

/// Cheaply-cloneable handle to the shared runtime context.
pub type SharedRuntimeContext = Arc<RuntimeContext>;

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tze_hud_config::HotReloadableConfig;
    use tze_hud_config::raw::{RawChrome, RawDegradation, RawDynamicPolicy, RawPrivacy};
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
        let config = make_config(vec![(
            "weather-agent",
            vec!["create_tiles", "modify_own_tiles"],
        )]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let caps = ctx.agent_capabilities("weather-agent").unwrap();
        assert!(caps.contains(&"create_tiles".to_string()));
        assert!(caps.contains(&"modify_own_tiles".to_string()));
    }

    #[test]
    fn from_config_hot_defaults_are_all_none() {
        let config = make_config(vec![]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let hot = ctx.hot_config();
        assert!(hot.privacy.default_classification.is_none());
        assert!(hot.privacy.redaction_style.is_none());
        assert!(hot.dynamic_policy.is_none());
    }

    // ── from_config_with_hot ──────────────────────────────────────────────────

    #[test]
    fn from_config_with_hot_stores_initial_hot_config() {
        let config = make_config(vec![]);
        let hot = HotReloadableConfig {
            privacy: RawPrivacy {
                redaction_style: Some("blank".to_string()),
                ..Default::default()
            },
            degradation: RawDegradation::default(),
            chrome: RawChrome::default(),
            dynamic_policy: None,
        };
        let ctx = RuntimeContext::from_config_with_hot(config, FallbackPolicy::Guest, hot);
        let loaded = ctx.hot_config();
        assert_eq!(loaded.privacy.redaction_style, Some("blank".to_string()));
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

    #[test]
    fn headless_default_hot_config_all_defaults() {
        let ctx = RuntimeContext::headless_default();
        let hot = ctx.hot_config();
        assert!(hot.privacy.redaction_style.is_none());
        assert!(hot.dynamic_policy.is_none());
    }

    // ── reload_hot_config ─────────────────────────────────────────────────────

    /// Spec §Configuration Reload (lines 263-274): SIGHUP or ReloadConfig RPC
    /// atomically replaces the hot-reloadable sections without restart.
    #[test]
    fn reload_hot_config_atomically_replaces_privacy() {
        let ctx = RuntimeContext::headless_default();

        // Before reload: defaults (all None).
        assert!(ctx.hot_config().privacy.redaction_style.is_none());

        // Reload with updated privacy.
        let new_hot = HotReloadableConfig {
            privacy: RawPrivacy {
                redaction_style: Some("pattern".to_string()),
                ..Default::default()
            },
            degradation: RawDegradation::default(),
            chrome: RawChrome::default(),
            dynamic_policy: None,
        };
        ctx.reload_hot_config(new_hot);

        // After reload: new value is visible.
        assert_eq!(
            ctx.hot_config().privacy.redaction_style,
            Some("pattern".to_string()),
            "reload_hot_config must atomically replace privacy settings"
        );
    }

    #[test]
    fn reload_hot_config_replaces_degradation_thresholds() {
        let ctx = RuntimeContext::headless_default();

        let new_hot = HotReloadableConfig {
            privacy: RawPrivacy::default(),
            degradation: RawDegradation {
                coalesce_frame_ms: Some(16.0),
                simplify_rendering_frame_ms: Some(33.0),
                ..Default::default()
            },
            chrome: RawChrome::default(),
            dynamic_policy: None,
        };
        ctx.reload_hot_config(new_hot);

        let hot = ctx.hot_config();
        assert_eq!(hot.degradation.coalesce_frame_ms, Some(16.0));
        assert_eq!(hot.degradation.simplify_rendering_frame_ms, Some(33.0));
    }

    #[test]
    fn reload_hot_config_enables_dynamic_agents() {
        let ctx = RuntimeContext::headless_default();
        assert!(
            ctx.hot_config().dynamic_policy.is_none(),
            "dynamic_policy absent by default"
        );

        let new_hot = HotReloadableConfig {
            privacy: RawPrivacy::default(),
            degradation: RawDegradation::default(),
            chrome: RawChrome::default(),
            dynamic_policy: Some(RawDynamicPolicy {
                allow_dynamic_agents: true,
                default_capabilities: Some(vec!["create_tiles".to_string()]),
                prompt_for_elevated_capabilities: true,
                dynamic_presence_ceiling: None,
            }),
        };
        ctx.reload_hot_config(new_hot);

        let hot = ctx.hot_config();
        let dp = hot
            .dynamic_policy
            .as_ref()
            .expect("dynamic_policy should be set");
        assert!(dp.allow_dynamic_agents);
    }

    #[test]
    fn reload_hot_config_multiple_reloads_always_returns_latest() {
        let ctx = RuntimeContext::headless_default();

        for i in 1u32..=5 {
            let style = format!("style-{i}");
            ctx.reload_hot_config(HotReloadableConfig {
                privacy: RawPrivacy {
                    redaction_style: Some(style.clone()),
                    ..Default::default()
                },
                degradation: RawDegradation::default(),
                chrome: RawChrome::default(),
                dynamic_policy: None,
            });
            assert_eq!(
                ctx.hot_config().privacy.redaction_style,
                Some(style),
                "after reload {i}, hot_config must return the latest value"
            );
        }
    }

    /// Frozen sections must not change after a reload.
    #[test]
    fn reload_hot_config_does_not_touch_frozen_fields() {
        let config = make_config(vec![("my-agent", vec!["create_tiles"])]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);

        // Snapshot frozen state before reload.
        let profile_name_before = ctx.profile.name.clone();
        let max_tiles_before = ctx.profile.max_tiles;

        // Reload hot config.
        ctx.reload_hot_config(HotReloadableConfig {
            privacy: RawPrivacy {
                redaction_style: Some("blank".to_string()),
                ..Default::default()
            },
            degradation: RawDegradation::default(),
            chrome: RawChrome::default(),
            dynamic_policy: None,
        });

        // Frozen fields unchanged.
        assert_eq!(ctx.profile.name, profile_name_before);
        assert_eq!(ctx.profile.max_tiles, max_tiles_before);
        // Frozen agent registry unchanged.
        let policy = ctx.capability_policy_for("my-agent");
        assert!(
            policy
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_ok()
        );
    }

    // ── capability_policy_for ─────────────────────────────────────────────────

    #[test]
    fn registered_agent_gets_configured_capabilities() {
        let config = make_config(vec![(
            "agent-a",
            vec!["create_tiles", "read_scene_topology"],
        )]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let policy = ctx.capability_policy_for("agent-a");
        let result = policy.evaluate_capability_request(&["create_tiles".to_string()]);
        assert!(
            result.is_ok(),
            "registered agent should be granted create_tiles"
        );
        let result = policy.evaluate_capability_request(&["overlay_privileges".to_string()]);
        assert!(
            result.is_err(),
            "registered agent should be denied unconfigured capability"
        );
    }

    #[test]
    fn unregistered_agent_gets_guest_policy_by_default() {
        let config = make_config(vec![]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let policy = ctx.capability_policy_for("unknown-agent");
        assert!(!policy.is_unrestricted());
        let result = policy.evaluate_capability_request(&["create_tiles".to_string()]);
        assert!(
            result.is_err(),
            "unregistered agent should be denied under guest fallback"
        );
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
        assert!(
            policy_a
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_ok()
        );
        assert!(
            policy_a
                .evaluate_capability_request(&["read_telemetry".to_string()])
                .is_err()
        );

        // agent-b can read telemetry but not create tiles
        let policy_b = ctx.capability_policy_for("agent-b");
        assert!(
            policy_b
                .evaluate_capability_request(&["read_telemetry".to_string()])
                .is_ok()
        );
        assert!(
            policy_b
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_err()
        );
    }

    // ── Spec: Agent Registration with Per-Agent Budget Overrides ─────────────
    // configuration/spec.md lines 136-147

    #[test]
    fn config_registered_agent_caps_replace_psk_unrestricted_sentinel() {
        // This test encodes the core invariant: after wiring config,
        // PSK auth no longer implies unrestricted "*" — only registered
        // agents get their listed capabilities.
        let config = make_config(vec![(
            "my-agent",
            vec!["create_tiles", "modify_own_tiles", "access_input_events"],
        )]);
        let ctx = RuntimeContext::from_config(config, FallbackPolicy::Guest);
        let policy = ctx.capability_policy_for("my-agent");

        // Should NOT be unrestricted
        assert!(
            !policy.is_unrestricted(),
            "config-registered agent policy must not be unrestricted"
        );

        // Should allow exactly the listed capabilities
        assert!(
            policy
                .evaluate_capability_request(&["create_tiles".to_string()])
                .is_ok()
        );
        assert!(
            policy
                .evaluate_capability_request(&["modify_own_tiles".to_string()])
                .is_ok()
        );
        assert!(
            policy
                .evaluate_capability_request(&["access_input_events".to_string()])
                .is_ok()
        );
        assert!(
            policy
                .evaluate_capability_request(&["overlay_privileges".to_string()])
                .is_err()
        );
    }
}
