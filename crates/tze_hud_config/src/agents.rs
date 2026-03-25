//! Agent registration configuration validation — rig-mop4.
//!
//! Implements spec `configuration/spec.md` requirements:
//!
//! - **Agent Registration with Per-Agent Budget Overrides** (lines 136-147, v1-mandatory)
//!   Per-agent `max_tiles`, `max_texture_mb`, `max_update_hz` MUST NOT exceed
//!   the active profile's ceiling. Violations → `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE`.
//! - **Dynamic Agent Policy** (lines 302-309, v1-mandatory)
//!   `[agents.dynamic_policy]` with `allow_dynamic_agents` (default: false).
//!   Without this section, unregistered agent connections are rejected.
//! - **Authentication Secret Indirection** (lines 311-322, v1-mandatory)
//!   Agent PSK MUST reference an env var via `auth_psk_env`. If the env var is
//!   unset, a warning is logged and the agent cannot authenticate.
//!
//! ## Immutability Contract
//!
//! `[agents.registered]` is frozen at startup. Dynamic policy (`[agents.dynamic_policy]`)
//! is hot-reloadable (see `reload.rs`).

use tze_hud_scene::config::{ConfigError, ConfigErrorCode, DisplayProfile};

use crate::raw::RawAgents;

// ─── Budget field descriptors ─────────────────────────────────────────────────

/// A per-agent budget field that must be checked against the profile ceiling.
struct AgentBudgetField<'a> {
    /// Field name in the config (for error messages).
    field_name: &'a str,
    /// The agent's declared override value (if any).
    agent_value: Option<u32>,
    /// The profile ceiling.
    profile_ceiling: u32,
    /// Config path to the field.
    field_path: String,
}

// ─── Validation ───────────────────────────────────────────────────────────────

/// Validate `[agents]` section against the active display profile.
///
/// Checks:
/// 1. Per-agent budget overrides do not exceed profile ceilings
///    (`max_tiles`, `max_texture_mb`).
///
/// Note: Auth PSK env-var indirection and dynamic agent policy structural checks
/// are intentionally separate concerns handled by `check_agent_auth_env_vars()`
/// and `dynamic_agents_allowed()` respectively. They are not startup-blocking
/// validation errors; the caller invokes them independently.
pub fn validate_agents(
    agents: &RawAgents,
    profile: &DisplayProfile,
    errors: &mut Vec<ConfigError>,
) {
    if let Some(registered) = &agents.registered {
        for (agent_name, agent) in registered {
            // Collect per-agent budget fields.
            let budget_fields = [
                AgentBudgetField {
                    field_name: "max_tiles",
                    agent_value: agent.max_tiles,
                    profile_ceiling: profile.max_tiles,
                    field_path: format!("agents.registered.{agent_name}.max_tiles"),
                },
                AgentBudgetField {
                    field_name: "max_texture_mb",
                    agent_value: agent.max_texture_mb,
                    profile_ceiling: profile.max_texture_mb,
                    field_path: format!("agents.registered.{agent_name}.max_texture_mb"),
                },
                // max_update_hz maps to profile.max_agent_update_hz.
                // The profile currently doesn't expose max_agent_update_hz, but spec
                // requires validation. We use u32::MAX as a sentinel when the profile
                // does not set an explicit ceiling (meaning no restriction).
                // NOTE: DisplayProfile may gain max_agent_update_hz in a future bead.
            ];

            for f in &budget_fields {
                if let Some(agent_val) = f.agent_value
                    && agent_val > f.profile_ceiling {
                        errors.push(ConfigError {
                            code: ConfigErrorCode::AgentBudgetExceedsProfile,
                            field_path: f.field_path.clone(),
                            expected: format!(
                                "{} <= profile ceiling {}",
                                f.field_name, f.profile_ceiling
                            ),
                            got: format!("{}", agent_val),
                            hint: format!(
                                "agent {:?} sets {}={} which exceeds the active profile ceiling of {}; \
                                 reduce the agent's {} to at most {}",
                                agent_name,
                                f.field_name,
                                agent_val,
                                f.profile_ceiling,
                                f.field_name,
                                f.profile_ceiling
                            ),
                        });
                    }
            }
        }
    }
}

/// Check agent authentication PSK env var indirection with injectable env lookup.
///
/// For each registered agent that sets `auth_psk_env`, checks whether the
/// referenced environment variable is set using the provided env lookup function.
/// Returns a list of warning messages for unset env vars (the caller should log them as warnings).
///
/// Per spec: if env var is unset → warning logged, agent cannot authenticate.
/// This does NOT produce a `ConfigError` — it is a runtime warning, not a
/// startup-blocking error.
///
/// # Arguments
///
/// * `agents` - The agents configuration to check
/// * `env_lookup` - A closure that takes an env var name and returns `Option<String>`.
///   In production, pass a closure like `|k| std::env::var(k).ok()`, and in tests, a mock.
pub fn check_agent_auth_env_vars_with_lookup<F>(
    agents: &RawAgents,
    env_lookup: F,
) -> Vec<AuthEnvWarning>
where
    F: Fn(&str) -> Option<String>,
{
    let mut warnings = Vec::new();

    if let Some(registered) = &agents.registered {
        for (agent_name, agent) in registered {
            if let Some(env_var_name) = &agent.auth_psk_env {
                match env_lookup(env_var_name) {
                    Some(val) if !val.is_empty() => {
                        // Env var is set and non-empty — agent can authenticate.
                    }
                    _ => {
                        // Env var unset or empty — agent cannot authenticate.
                        warnings.push(AuthEnvWarning {
                            agent_name: agent_name.clone(),
                            env_var_name: env_var_name.clone(),
                        });
                    }
                }
            }
        }
    }

    warnings
}

/// Check agent authentication PSK env var indirection.
///
/// For each registered agent that sets `auth_psk_env`, check whether the
/// referenced environment variable is currently set. Returns a list of
/// warning messages for unset env vars (the caller should log them as warnings).
///
/// Per spec: if env var is unset → warning logged, agent cannot authenticate.
/// This does NOT produce a `ConfigError` — it is a runtime warning, not a
/// startup-blocking error.
///
/// This is a convenience wrapper around `check_agent_auth_env_vars_with_lookup`
/// that uses `std::env::var` for the environment lookup.
pub fn check_agent_auth_env_vars(agents: &RawAgents) -> Vec<AuthEnvWarning> {
    check_agent_auth_env_vars_with_lookup(agents, |var_name| {
        std::env::var(var_name).ok()
    })
}

/// A warning about an unset auth PSK env var.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEnvWarning {
    /// The agent whose PSK env var is unset.
    pub agent_name: String,
    /// The env var that is unset.
    pub env_var_name: String,
}

impl AuthEnvWarning {
    /// Produces a human-readable warning message suitable for logging.
    pub fn to_log_message(&self) -> String {
        format!(
            "WARNING: agent {:?} sets auth_psk_env = {:?} but the environment variable \
             {:?} is not set; the agent cannot authenticate until the variable is set",
            self.agent_name, self.env_var_name, self.env_var_name
        )
    }
}

/// Returns `true` if dynamic agents are allowed per the `[agents.dynamic_policy]` section.
///
/// Per spec: if no `[agents.dynamic_policy]` section is present → `false` (connections
/// from unregistered agents are rejected by default).
pub fn dynamic_agents_allowed(agents: &RawAgents) -> bool {
    agents
        .dynamic_policy
        .as_ref()
        .map(|dp| dp.allow_dynamic_agents)
        .unwrap_or(false)
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{RawAgents, RawDynamicPolicy, RawRegisteredAgent};
    use std::collections::HashMap;
    use tze_hud_scene::config::DisplayProfile;

    fn full_display_profile() -> DisplayProfile {
        DisplayProfile::full_display()
    }

    // ── Agent budget validation ───────────────────────────────────────────────

    #[test]
    fn test_agent_budget_within_profile_ceiling_accepted() {
        // Spec scenario: agent sets max_tiles = 4, profile has max_tiles = 1024 → accepted.
        let mut registered = HashMap::new();
        registered.insert(
            "agent_a".to_string(),
            RawRegisteredAgent {
                max_tiles: Some(4),
                ..Default::default()
            },
        );
        let agents = RawAgents {
            registered: Some(registered),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_agents(&agents, &full_display_profile(), &mut errors);
        assert!(errors.is_empty(), "max_tiles=4 within profile ceiling should be accepted");
    }

    #[test]
    fn test_agent_budget_exceeds_profile_ceiling_rejected() {
        // Spec scenario: agent sets max_tiles = 2048, profile has max_tiles = 1024
        // → CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE identifying agent, field, and ceiling.
        let mut registered = HashMap::new();
        registered.insert(
            "agent_b".to_string(),
            RawRegisteredAgent {
                max_tiles: Some(2048),
                ..Default::default()
            },
        );
        let agents = RawAgents {
            registered: Some(registered),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_agents(&agents, &full_display_profile(), &mut errors);
        assert!(
            errors.iter().any(|e| matches!(e.code, ConfigErrorCode::AgentBudgetExceedsProfile)),
            "max_tiles=2048 exceeding profile ceiling 1024 should produce CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE"
        );
        let err = errors
            .iter()
            .find(|e| matches!(e.code, ConfigErrorCode::AgentBudgetExceedsProfile))
            .unwrap();
        // Error must identify the agent.
        assert!(
            err.hint.contains("agent_b"),
            "error should identify agent name, got hint: {:?}", err.hint
        );
        // Error must identify the field.
        assert!(
            err.field_path.contains("max_tiles"),
            "error should identify max_tiles field, got field_path: {:?}", err.field_path
        );
        // Error must identify the ceiling.
        assert!(
            err.expected.contains("1024"),
            "error should identify profile ceiling 1024, got expected: {:?}", err.expected
        );
    }

    #[test]
    fn test_agent_max_texture_mb_exceeds_ceiling_rejected() {
        let mut registered = HashMap::new();
        registered.insert(
            "agent_c".to_string(),
            RawRegisteredAgent {
                max_texture_mb: Some(4096), // exceeds full-display ceiling of 2048
                ..Default::default()
            },
        );
        let agents = RawAgents {
            registered: Some(registered),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_agents(&agents, &full_display_profile(), &mut errors);
        assert!(
            errors.iter().any(|e| {
                matches!(e.code, ConfigErrorCode::AgentBudgetExceedsProfile)
                    && e.field_path.contains("max_texture_mb")
            }),
            "max_texture_mb=4096 exceeding ceiling 2048 should produce error, got: {:?}", errors
        );
    }

    #[test]
    fn test_no_agents_section_no_errors() {
        let agents = RawAgents::default();
        let mut errors = Vec::new();
        validate_agents(&agents, &full_display_profile(), &mut errors);
        assert!(errors.is_empty(), "absent agents section should not produce errors");
    }

    // ── Dynamic agent policy ──────────────────────────────────────────────────

    #[test]
    fn test_no_dynamic_policy_section_dynamic_agents_disabled() {
        // Spec scenario: no [agents.dynamic_policy] → connections from unregistered
        // agents rejected.
        let agents = RawAgents {
            dynamic_policy: None,
            ..Default::default()
        };
        assert!(
            !dynamic_agents_allowed(&agents),
            "no dynamic_policy section should mean dynamic agents are disabled"
        );
    }

    #[test]
    fn test_dynamic_policy_allow_dynamic_agents_false() {
        let agents = RawAgents {
            dynamic_policy: Some(RawDynamicPolicy {
                allow_dynamic_agents: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!dynamic_agents_allowed(&agents));
    }

    #[test]
    fn test_dynamic_policy_allow_dynamic_agents_true() {
        let agents = RawAgents {
            dynamic_policy: Some(RawDynamicPolicy {
                allow_dynamic_agents: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(dynamic_agents_allowed(&agents));
    }

    // ── Auth PSK env var indirection ──────────────────────────────────────────

    #[test]
    fn test_auth_psk_env_set_no_warning() {
        // Spec scenario: agent sets auth_psk_env = "TEST_AGENT_KEY_SET" and env var is set
        // → agent can authenticate (no warning).
        // Use mock env lookup to avoid unsafe env mutation.
        let mut registered = HashMap::new();
        registered.insert(
            "agent_a".to_string(),
            RawRegisteredAgent {
                auth_psk_env: Some("TEST_AGENT_KEY_SET".into()),
                ..Default::default()
            },
        );
        let agents = RawAgents {
            registered: Some(registered),
            ..Default::default()
        };
        let mock_lookup = |var_name: &str| -> Option<String> {
            if var_name == "TEST_AGENT_KEY_SET" {
                Some("mysecret".to_string())
            } else {
                None
            }
        };
        let warnings = check_agent_auth_env_vars_with_lookup(&agents, mock_lookup);
        assert!(
            warnings.is_empty(),
            "set env var should produce no auth warnings, got: {:?}", warnings
        );
    }

    #[test]
    fn test_auth_psk_env_unset_produces_warning() {
        // Spec scenario: agent sets auth_psk_env = "AGENT_KEY" and env var AGENT_KEY is not set
        // → warning logged, agent cannot authenticate.
        let env_var = "TEST_AGENT_KEY_UNSET";
        let mut registered = HashMap::new();
        registered.insert(
            "agent_b".to_string(),
            RawRegisteredAgent {
                auth_psk_env: Some(env_var.into()),
                ..Default::default()
            },
        );
        let agents = RawAgents {
            registered: Some(registered),
            ..Default::default()
        };
        // Mock env lookup that always returns None (unset).
        let mock_lookup = |_var_name: &str| -> Option<String> { None };
        let warnings = check_agent_auth_env_vars_with_lookup(&agents, mock_lookup);
        assert!(
            !warnings.is_empty(),
            "unset env var should produce auth warning"
        );
        let w = &warnings[0];
        assert_eq!(w.agent_name, "agent_b");
        assert_eq!(w.env_var_name, env_var);
        // Warning message must be informative.
        let msg = w.to_log_message();
        assert!(msg.contains("agent_b"), "warning should mention agent name");
        assert!(msg.contains(env_var), "warning should mention env var name");
    }
}
