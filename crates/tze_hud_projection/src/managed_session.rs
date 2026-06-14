//! Managed-session orchestration types.
//!
//! Pure data types describing how an external agent session is registered,
//! routed, and tracked by the projection authority. No process-supervision
//! or authority state-machine code lives here — those are P-3 (`portal.rs`)
//! and P-4 (`authority.rs`) respectively.
//!
//! Visibility notes: items that were implicitly private to `lib.rs` gain the
//! minimal visibility required by Rust's module privacy rules (`pub(super)`
//! where a method or helper is called only from `lib.rs`; `pub(crate)` where
//! it is also called from a sibling submodule such as `authority.rs`).

use crate::contract::{validate_non_empty_bounded, validate_non_zero, validate_optional_bounded};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// How a provider-neutral LLM session entered projection authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionOrigin {
    /// Already-running session that opted in through the cooperative contract.
    Attached,
    /// Authority-supervised launch. This records intent and metadata; it is not
    /// terminal capture or PTY ownership.
    Launched(LaunchSessionSpec),
}

/// Redacted, provider-neutral launch metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSessionSpec {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment_keys: Vec<String>,
}

impl LaunchSessionSpec {
    pub(super) fn validate(&self) -> Result<(), crate::ProjectionContractError> {
        validate_non_empty_bounded("launch_command", &self.command, crate::MAX_HINT_BYTES)?;
        for arg in &self.args {
            validate_non_empty_bounded("launch_arg", arg, crate::MAX_HINT_BYTES)?;
        }
        validate_optional_bounded(
            "launch_working_directory",
            &self.working_directory,
            crate::MAX_HINT_BYTES,
        )?;
        for key in &self.environment_keys {
            validate_non_empty_bounded("launch_environment_key", key, crate::MAX_HINT_BYTES)?;
        }
        Ok(())
    }
}

/// Runtime credential source. Values are never stored here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum HudCredentialSource {
    EnvVar(String),
    ProtectedConfigKey(String),
}

impl HudCredentialSource {
    pub(super) fn validate(&self) -> Result<(), crate::ProjectionContractError> {
        match self {
            Self::EnvVar(name) => {
                validate_non_empty_bounded("credential_env_var", name, crate::MAX_HINT_BYTES)
            }
            Self::ProtectedConfigKey(name) => {
                validate_non_empty_bounded("credential_config_key", name, crate::MAX_HINT_BYTES)
            }
        }
    }

    pub(super) fn redacted_marker(&self) -> String {
        match self {
            Self::EnvVar(name) => format!("env:{name}:redacted"),
            Self::ProtectedConfigKey(name) => format!("protected-config:{name}:redacted"),
        }
    }
}

/// Local Windows HUD runtime target metadata retained by the external authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsHudTarget {
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grpc_endpoint: Option<String>,
    pub credential_source: HudCredentialSource,
    pub runtime_audience: String,
}

impl WindowsHudTarget {
    pub(super) fn validate(&self) -> Result<(), crate::ProjectionContractError> {
        validate_non_empty_bounded("hud_target_id", &self.target_id, crate::MAX_HINT_BYTES)?;
        validate_optional_bounded("mcp_url", &self.mcp_url, crate::MAX_HINT_BYTES)?;
        validate_optional_bounded("grpc_endpoint", &self.grpc_endpoint, crate::MAX_HINT_BYTES)?;
        if self.mcp_url.is_none() && self.grpc_endpoint.is_none() {
            return Err(crate::ProjectionContractError::InvalidArgument(
                "Windows HUD target requires mcp_url or grpc_endpoint".to_string(),
            ));
        }
        self.credential_source.validate()?;
        validate_non_empty_bounded(
            "runtime_audience",
            &self.runtime_audience,
            crate::MAX_HINT_BYTES,
        )
    }
}

/// Projection attention intent. V1 defaults to ambient presence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionAttentionIntent {
    #[default]
    Ambient,
    Gentle,
    Interruptive,
}

/// Surface class requested by an external managed session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "surface")]
pub enum PresenceSurfaceRoute {
    Zone {
        zone_name: String,
        content_kind: String,
        ttl_ms: u64,
    },
    Widget {
        widget_name: String,
        #[serde(default)]
        parameters: HashMap<String, WidgetParameterValue>,
        ttl_ms: u64,
    },
    Portal {
        #[serde(default)]
        portal_surface: PortalSurfaceKind,
        #[serde(default)]
        requested_capabilities: Vec<String>,
        lease_ttl_ms: u64,
    },
}

impl PresenceSurfaceRoute {
    pub(super) fn validate(&self) -> Result<(), crate::ProjectionContractError> {
        match self {
            Self::Zone {
                zone_name,
                content_kind,
                ttl_ms,
            } => {
                validate_non_empty_bounded("zone_name", zone_name, crate::MAX_HINT_BYTES)?;
                validate_non_empty_bounded(
                    "zone_content_kind",
                    content_kind,
                    crate::MAX_HINT_BYTES,
                )?;
                validate_non_zero("zone_ttl_ms", *ttl_ms)
            }
            Self::Widget {
                widget_name,
                parameters,
                ttl_ms,
            } => {
                validate_non_empty_bounded("widget_name", widget_name, crate::MAX_HINT_BYTES)?;
                if parameters.is_empty() {
                    return Err(crate::ProjectionContractError::InvalidArgument(
                        "widget route requires at least one parameter".to_string(),
                    ));
                }
                for key in parameters.keys() {
                    validate_non_empty_bounded(
                        "widget_parameter_name",
                        key,
                        crate::MAX_HINT_BYTES,
                    )?;
                }
                validate_non_zero("widget_ttl_ms", *ttl_ms)
            }
            Self::Portal {
                portal_surface: _,
                requested_capabilities,
                lease_ttl_ms,
            } => {
                for capability in requested_capabilities {
                    validate_non_empty_bounded(
                        "portal_capability",
                        capability,
                        crate::MAX_HINT_BYTES,
                    )?;
                }
                validate_non_zero("portal_lease_ttl_ms", *lease_ttl_ms)
            }
        }
    }
}

/// Existing v1 portal materialization requested by a managed session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalSurfaceKind {
    #[default]
    TextStreamRawTile,
}

/// Bounded widget parameter value used by route plans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum WidgetParameterValue {
    F32Milli(i64),
    Text(String),
    ColorRgba([u8; 4]),
    Enum(String),
}

/// Request to register or update one managed external session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedSessionRequest {
    pub projection_id: String,
    pub provider_kind: crate::ProviderKind,
    pub display_name: String,
    pub origin: ManagedSessionOrigin,
    pub hud_target_id: String,
    pub surface_route: PresenceSurfaceRoute,
    #[serde(default)]
    pub content_classification: crate::ContentClassification,
    #[serde(default)]
    pub attention_intent: ProjectionAttentionIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_profile_hint: Option<String>,
}

impl ManagedSessionRequest {
    pub(super) fn validate(&self) -> Result<(), crate::ProjectionContractError> {
        validate_non_empty_bounded(
            "projection_id",
            &self.projection_id,
            crate::MAX_PROJECTION_ID_BYTES,
        )?;
        validate_non_empty_bounded(
            "display_name",
            &self.display_name,
            crate::MAX_DISPLAY_NAME_BYTES,
        )?;
        validate_non_empty_bounded("hud_target_id", &self.hud_target_id, crate::MAX_HINT_BYTES)?;
        validate_optional_bounded(
            "workspace_hint",
            &self.workspace_hint,
            crate::MAX_HINT_BYTES,
        )?;
        validate_optional_bounded(
            "repository_hint",
            &self.repository_hint,
            crate::MAX_HINT_BYTES,
        )?;
        validate_optional_bounded(
            "icon_profile_hint",
            &self.icon_profile_hint,
            crate::MAX_HINT_BYTES,
        )?;
        if let ManagedSessionOrigin::Launched(spec) = &self.origin {
            spec.validate()?;
        }
        self.surface_route.validate()
    }
}

/// Runtime-facing command plan for a managed session. This is advisory: the
/// runtime remains the final policy and capability authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum HudSurfaceCommandPlan {
    ZonePublish {
        zone_name: String,
        content_kind: String,
        ttl_ms: u64,
        agent_id: String,
    },
    WidgetPublish {
        widget_name: String,
        parameters: HashMap<String, WidgetParameterValue>,
        ttl_ms: u64,
        agent_id: String,
    },
    PortalLease {
        portal_surface: PortalSurfaceKind,
        portal_id: String,
        requested_capabilities: Vec<String>,
        lease_ttl_ms: u64,
        agent_id: String,
    },
}

/// Bounded, redacted route plan suitable for audit and demo evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedSessionRoutePlan {
    pub projection_id: String,
    pub provider_kind: crate::ProviderKind,
    pub display_name: String,
    pub origin: ManagedSessionOrigin,
    pub hud_target_id: String,
    pub runtime_audience: String,
    pub credential_redacted: String,
    pub lifecycle_state: crate::ProjectionLifecycleState,
    pub content_classification: crate::ContentClassification,
    pub attention_intent: ProjectionAttentionIntent,
    pub surface_command: HudSurfaceCommandPlan,
    pub cleanup_on_detach: bool,
}

/// Handle returned after registering a managed session. The owner token is
/// returned only to the caller and is intentionally absent from route plans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedSessionHandle {
    pub route_plan: ManagedSessionRoutePlan,
    pub owner_token: String,
}

/// Secret-bearing runtime authentication material resolved just-in-time by
/// the external authority. This intentionally has no `Serialize` impl.
#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeAuthenticationMaterial {
    pub target_id: String,
    pub mcp_url: Option<String>,
    pub grpc_endpoint: Option<String>,
    pub runtime_audience: String,
    pub credential_redacted: String,
    pub(super) credential_secret: String,
}

impl RuntimeAuthenticationMaterial {
    /// Return the credential value for the runtime client that will perform the
    /// MCP/gRPC authentication attempt. Callers must not log this value.
    pub fn credential_secret(&self) -> &str {
        &self.credential_secret
    }
}

impl fmt::Debug for RuntimeAuthenticationMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeAuthenticationMaterial")
            .field("target_id", &self.target_id)
            .field("mcp_url", &self.mcp_url)
            .field("grpc_endpoint", &self.grpc_endpoint)
            .field("runtime_audience", &self.runtime_audience)
            .field("credential_redacted", &self.credential_redacted)
            .finish_non_exhaustive()
    }
}
