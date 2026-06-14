//! Provider-neutral cooperative HUD projection operation contract.
//!
//! This crate owns the low-token operation schema for the external projection
//! authority described by `openspec/changes/cooperative-hud-projection/`.
//! It deliberately models projection-daemon operations, not runtime v1 MCP
//! tools. If the contract is exposed through MCP, that MCP server belongs to
//! the projection daemon and talks outward to the HUD over the resident control
//! plane.

#[cfg(feature = "resident-grpc")]
pub mod resident_grpc;

/// Portal cadence coalescing with cross-portal fairness.
///
/// Lives here (rather than in `tze_hud_runtime`) so `ProjectionAuthority` can
/// hold a `PortalCadenceCoalescer` without a circular crate dependency.
/// `tze_hud_runtime` re-exports all public items from this module.
pub mod portal_cadence;

mod contract;
pub use self::contract::*;

mod managed_session;
pub use self::managed_session::*;

mod authority;
pub use self::authority::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::{env, fmt};
/// Default maximum bytes accepted by one `publish_output` request.
pub const DEFAULT_MAX_OUTPUT_BYTES_PER_CALL: usize = 16_384;
/// Default maximum bytes accepted by `publish_status.status_text`.
pub const DEFAULT_MAX_STATUS_TEXT_BYTES: usize = 512;
/// Default retained transcript byte budget for a projection.
pub const DEFAULT_MAX_RETAINED_TRANSCRIPT_BYTES: usize = 262_144;
/// Default visible transcript byte budget for portal materialization.
pub const DEFAULT_MAX_VISIBLE_TRANSCRIPT_BYTES: usize = 16_384;
/// Default maximum number of pending HUD input items.
pub const DEFAULT_MAX_PENDING_INPUT_ITEMS: usize = 32;
/// Default maximum bytes in one HUD input item.
pub const DEFAULT_MAX_PENDING_INPUT_BYTES_PER_ITEM: usize = 4_096;
/// Default maximum aggregate pending HUD input bytes.
pub const DEFAULT_MAX_PENDING_INPUT_TOTAL_BYTES: usize = 32_768;
/// Default maximum pending items returned by one poll.
pub const DEFAULT_MAX_POLL_ITEMS: usize = 8;
/// Default maximum bytes returned by one pending-input poll.
pub const DEFAULT_MAX_POLL_RESPONSE_BYTES: usize = 16_384;
/// Default maximum HUD portal updates per second.
pub const DEFAULT_MAX_PORTAL_UPDATES_PER_SECOND: u32 = 10;
/// Default maximum retained publish-output logical-unit IDs per projection.
pub const DEFAULT_MAX_SEEN_LOGICAL_UNITS: usize = 4_096;
/// Default maximum retained audit records for the in-memory authority.
pub const DEFAULT_MAX_AUDIT_RECORDS: usize = 4_096;
/// Owner tokens are 256-bit random values encoded as lowercase hex.
pub const OWNER_TOKEN_ENTROPY_BITS: usize = 256;
/// Default owner-token lifetime in wall-clock microseconds.
pub const DEFAULT_OWNER_TOKEN_TTL_WALL_US: u64 = 24 * 60 * 60 * 1_000_000;
/// One wall-clock second in microseconds, used for portal update-rate windows.
pub const PORTAL_UPDATE_RATE_WINDOW_WALL_US: u64 = 1_000_000;

pub(crate) const MAX_PROJECTION_ID_BYTES: usize = 128;
pub(crate) const MAX_REQUEST_ID_BYTES: usize = 128;
pub(crate) const MAX_CALLER_IDENTITY_BYTES: usize = 256;
pub(crate) const MAX_DISPLAY_NAME_BYTES: usize = 128;
pub(crate) const MAX_HINT_BYTES: usize = 256;
pub(crate) const MAX_STATUS_SUMMARY_BYTES: usize = 512;
pub(crate) const MAX_REASON_BYTES: usize = 512;
pub(crate) const MAX_ACK_MESSAGE_BYTES: usize = 512;
pub(crate) const MAX_PORTAL_ID_BYTES: usize = 192;
pub(crate) const DEFAULT_PORTAL_INPUT_TTL_WALL_US: u64 = 10 * 60 * 1_000_000;

#[derive(Clone, Debug)]
struct ManagedSessionRecord {
    route_plan: ManagedSessionRoutePlan,
}

/// Provider process lifecycle state retained by the external authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProcessState {
    Running,
    Exited { code: Option<i32> },
}

/// Bounded process status for an authority-supervised launched session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProcessStatus {
    pub projection_id: String,
    pub process_id: u32,
    pub state: ProviderProcessState,
}

struct ProviderProcessRecord {
    child: Child,
    last_state: ProviderProcessState,
}

impl fmt::Debug for ProviderProcessRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderProcessRecord")
            .field("process_id", &self.child.id())
            .field("last_state", &self.last_state)
            .finish()
    }
}

impl ProviderProcessRecord {
    fn status(
        &mut self,
        projection_id: &str,
    ) -> Result<ProviderProcessStatus, ProjectionErrorCode> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.last_state = ProviderProcessState::Exited {
                    code: status.code(),
                };
            }
            Ok(None) => {
                self.last_state = ProviderProcessState::Running;
            }
            Err(_) => return Err(ProjectionErrorCode::ProjectionInternalError),
        }
        Ok(ProviderProcessStatus {
            projection_id: projection_id.to_string(),
            process_id: self.child.id(),
            state: self.last_state,
        })
    }

    fn terminate(
        mut self,
        projection_id: &str,
    ) -> Result<ProviderProcessStatus, ProjectionErrorCode> {
        let process_id = self.child.id();
        let state = match self.child.try_wait() {
            Ok(Some(status)) => ProviderProcessState::Exited {
                code: status.code(),
            },
            Ok(None) => {
                self.child
                    .kill()
                    .map_err(|_| ProjectionErrorCode::ProjectionInternalError)?;
                let status = self
                    .child
                    .wait()
                    .map_err(|_| ProjectionErrorCode::ProjectionInternalError)?;
                ProviderProcessState::Exited {
                    code: status.code(),
                }
            }
            Err(_) => return Err(ProjectionErrorCode::ProjectionInternalError),
        };
        Ok(ProviderProcessStatus {
            projection_id: projection_id.to_string(),
            process_id,
            state,
        })
    }
}

/// External authority layer for launched/attached provider-neutral sessions.
#[derive(Debug)]
pub struct ExternalAgentProjectionAuthority {
    projection_authority: ProjectionAuthority,
    targets: HashMap<String, WindowsHudTarget>,
    managed_sessions: HashMap<String, ManagedSessionRecord>,
    provider_processes: HashMap<String, ProviderProcessRecord>,
}

impl ExternalAgentProjectionAuthority {
    pub fn new(bounds: ProjectionBounds) -> Result<Self, ProjectionContractError> {
        Ok(Self {
            projection_authority: ProjectionAuthority::new(bounds)?,
            targets: HashMap::new(),
            managed_sessions: HashMap::new(),
            provider_processes: HashMap::new(),
        })
    }

    pub fn projection_authority(&self) -> &ProjectionAuthority {
        &self.projection_authority
    }

    pub fn projection_authority_mut(&mut self) -> &mut ProjectionAuthority {
        &mut self.projection_authority
    }

    pub fn register_windows_target(
        &mut self,
        target: WindowsHudTarget,
    ) -> Result<(), ProjectionContractError> {
        target.validate()?;
        self.targets.insert(target.target_id.clone(), target);
        Ok(())
    }

    pub fn manage_session(
        &mut self,
        request: ManagedSessionRequest,
        caller_identity: &str,
        server_timestamp_wall_us: u64,
    ) -> Result<ManagedSessionHandle, ProjectionErrorCode> {
        request.validate().map_err(|error| error.code())?;
        let target = self
            .targets
            .get(&request.hud_target_id)
            .ok_or(ProjectionErrorCode::ProjectionInvalidArgument)?
            .clone();

        let attach = AttachRequest {
            envelope: OperationEnvelope {
                operation: ProjectionOperation::Attach,
                projection_id: request.projection_id.clone(),
                request_id: format!("manage-{}", request.projection_id),
                client_timestamp_wall_us: server_timestamp_wall_us,
            },
            provider_kind: request.provider_kind.clone(),
            display_name: request.display_name.clone(),
            workspace_hint: request.workspace_hint.clone(),
            repository_hint: request.repository_hint.clone(),
            icon_profile_hint: request.icon_profile_hint.clone(),
            content_classification: request.content_classification,
            hud_target: Some(target.target_id.clone()),
            idempotency_key: Some(format!("managed-{}", request.projection_id)),
        };
        let response = self.projection_authority.handle_attach(
            attach,
            caller_identity,
            server_timestamp_wall_us,
        );
        if !response.accepted {
            return Err(response
                .error_code
                .unwrap_or(ProjectionErrorCode::ProjectionInternalError));
        }
        let owner_token = response
            .owner_token
            .ok_or(ProjectionErrorCode::ProjectionAlreadyAttached)?;
        let route_plan = route_plan_for_request(&request, &target);
        self.managed_sessions.insert(
            request.projection_id.clone(),
            ManagedSessionRecord {
                route_plan: route_plan.clone(),
            },
        );
        Ok(ManagedSessionHandle {
            route_plan,
            owner_token,
        })
    }

    /// Spawn the provider command for a managed `Launched` session. The
    /// authority supervises only process lifetime; it deliberately does not
    /// capture stdin, stdout, stderr, PTY state, or transcript bytes.
    pub fn launch_provider_process(
        &mut self,
        projection_id: &str,
    ) -> Result<ProviderProcessStatus, ProjectionErrorCode> {
        validate_non_empty_bounded("projection_id", projection_id, MAX_PROJECTION_ID_BYTES)
            .map_err(|error| error.code())?;
        let route_plan = self
            .managed_sessions
            .get(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?
            .route_plan
            .clone();
        let ManagedSessionOrigin::Launched(spec) = route_plan.origin else {
            return Err(ProjectionErrorCode::ProjectionInvalidArgument);
        };

        if let Some(process) = self.provider_processes.get_mut(projection_id) {
            return process.status(projection_id);
        }

        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(working_directory) = &spec.working_directory {
            command.current_dir(working_directory);
        }
        let child = command
            .spawn()
            .map_err(|_| ProjectionErrorCode::ProjectionInternalError)?;
        let mut record = ProviderProcessRecord {
            child,
            last_state: ProviderProcessState::Running,
        };
        let status = record.status(projection_id)?;
        self.provider_processes
            .insert(projection_id.to_string(), record);
        Ok(status)
    }

    pub fn provider_process_status(
        &mut self,
        projection_id: &str,
    ) -> Result<Option<ProviderProcessStatus>, ProjectionErrorCode> {
        validate_non_empty_bounded("projection_id", projection_id, MAX_PROJECTION_ID_BYTES)
            .map_err(|error| error.code())?;
        self.provider_processes
            .get_mut(projection_id)
            .map(|record| record.status(projection_id))
            .transpose()
    }

    pub fn terminate_provider_process(
        &mut self,
        projection_id: &str,
    ) -> Result<Option<ProviderProcessStatus>, ProjectionErrorCode> {
        validate_non_empty_bounded("projection_id", projection_id, MAX_PROJECTION_ID_BYTES)
            .map_err(|error| error.code())?;
        self.provider_processes
            .remove(projection_id)
            .map(|record| record.terminate(projection_id))
            .transpose()
    }

    pub fn route_plan(&self, projection_id: &str) -> Option<&ManagedSessionRoutePlan> {
        self.managed_sessions
            .get(projection_id)
            .map(|record| &record.route_plan)
    }

    /// Resolve runtime authentication material for the managed session's HUD
    /// target. Credential values are read at the edge, never stored in route
    /// plans or audit-ready structs.
    pub fn resolve_runtime_authentication(
        &self,
        projection_id: &str,
        mut protected_config_lookup: impl FnMut(&str) -> Option<String>,
    ) -> Result<RuntimeAuthenticationMaterial, ProjectionErrorCode> {
        validate_non_empty_bounded("projection_id", projection_id, MAX_PROJECTION_ID_BYTES)
            .map_err(|error| error.code())?;
        let route_plan = self
            .managed_sessions
            .get(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?
            .route_plan
            .clone();
        let target = self
            .targets
            .get(&route_plan.hud_target_id)
            .ok_or(ProjectionErrorCode::ProjectionInvalidArgument)?;
        let credential_secret = match &target.credential_source {
            HudCredentialSource::EnvVar(name) => {
                env::var(name).map_err(|_| ProjectionErrorCode::ProjectionUnauthorized)?
            }
            HudCredentialSource::ProtectedConfigKey(name) => {
                protected_config_lookup(name).ok_or(ProjectionErrorCode::ProjectionUnauthorized)?
            }
        };
        validate_non_empty_bounded("runtime_credential", &credential_secret, MAX_HINT_BYTES)
            .map_err(|error| error.code())?;
        Ok(RuntimeAuthenticationMaterial {
            target_id: target.target_id.clone(),
            mcp_url: target.mcp_url.clone(),
            grpc_endpoint: target.grpc_endpoint.clone(),
            runtime_audience: target.runtime_audience.clone(),
            credential_redacted: target.credential_source.redacted_marker(),
            credential_secret,
        })
    }

    pub fn managed_session_count(&self) -> usize {
        self.managed_sessions.len()
    }

    pub fn revoke_session(&mut self, projection_id: &str) -> Result<(), ProjectionErrorCode> {
        self.managed_sessions
            .remove(projection_id)
            .ok_or(ProjectionErrorCode::ProjectionNotFound)?;
        let _ = self.terminate_provider_process(projection_id);
        self.projection_authority.expire_projection(projection_id);
        Ok(())
    }

    pub fn expire_token_expired_sessions(&mut self, server_timestamp_wall_us: u64) -> usize {
        let expired_count = self
            .projection_authority
            .expire_token_expired_projections(server_timestamp_wall_us);
        self.managed_sessions
            .retain(|projection_id, _| self.projection_authority.has_projection(projection_id));
        let orphaned_processes: Vec<_> = self
            .provider_processes
            .keys()
            .filter(|projection_id| !self.managed_sessions.contains_key(*projection_id))
            .cloned()
            .collect();
        for projection_id in orphaned_processes {
            let _ = self.terminate_provider_process(&projection_id);
        }
        expired_count
    }

    pub fn mark_hud_disconnected(
        &mut self,
        projection_id: &str,
        disconnected_at_wall_us: u64,
    ) -> Result<(), ProjectionErrorCode> {
        self.projection_authority
            .mark_hud_disconnected(projection_id, disconnected_at_wall_us)
    }

    pub fn record_hud_connection(
        &mut self,
        projection_id: &str,
        metadata: HudConnectionMetadata,
    ) -> Result<(), ProjectionErrorCode> {
        self.projection_authority
            .record_hud_connection(projection_id, metadata)
    }

    pub fn three_session_demo_plan(&self) -> Vec<ManagedSessionRoutePlan> {
        let mut plans: Vec<_> = self
            .managed_sessions
            .values()
            .map(|record| record.route_plan.clone())
            .collect();
        plans.sort_by(|left, right| left.projection_id.cmp(&right.projection_id));
        plans
    }
}

impl Default for ExternalAgentProjectionAuthority {
    fn default() -> Self {
        Self::new(ProjectionBounds::default()).expect("default bounds are valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn envelope(
        operation: ProjectionOperation,
        projection_id: &str,
        request_id: &str,
    ) -> OperationEnvelope {
        OperationEnvelope {
            operation,
            projection_id: projection_id.to_string(),
            request_id: request_id.to_string(),
            client_timestamp_wall_us: 1,
        }
    }

    fn attach_request(projection_id: &str, request_id: &str) -> AttachRequest {
        AttachRequest {
            envelope: envelope(ProjectionOperation::Attach, projection_id, request_id),
            provider_kind: ProviderKind::Codex,
            display_name: "Codex Session".to_string(),
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: None,
            content_classification: ContentClassification::Private,
            hud_target: None,
            idempotency_key: Some("attach-once".to_string()),
        }
    }

    fn attach(authority: &mut ProjectionAuthority, projection_id: &str) -> String {
        authority
            .handle_attach(attach_request(projection_id, "attach-1"), "caller-a", 10)
            .owner_token
            .expect("attach must issue owner token")
    }

    fn output_request(
        projection_id: &str,
        owner_token: &str,
        request_id: &str,
    ) -> PublishOutputRequest {
        PublishOutputRequest {
            envelope: envelope(
                ProjectionOperation::PublishOutput,
                projection_id,
                request_id,
            ),
            owner_token: owner_token.to_string(),
            output_text: "hello projection".to_string(),
            output_kind: OutputKind::Assistant,
            content_classification: ContentClassification::Private,
            logical_unit_id: Some("unit-1".to_string()),
            coalesce_key: None,
        }
    }

    fn connection_metadata(grants: &[&str]) -> HudConnectionMetadata {
        HudConnectionMetadata {
            connection_id: "connection-1".to_string(),
            authenticated_session_id: "runtime-session-1".to_string(),
            granted_capabilities: grants.iter().map(|grant| (*grant).to_string()).collect(),
            connected_at_wall_us: 20,
            last_reconnect_wall_us: 20,
        }
    }

    fn advisory_lease(capabilities: &[&str], expires_at_wall_us: u64) -> AdvisoryLeaseIdentity {
        AdvisoryLeaseIdentity {
            lease_id: "lease-1".to_string(),
            capabilities: capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect(),
            acquired_at_wall_us: 21,
            expires_at_wall_us,
        }
    }

    fn portal_submission(input_id: &str, text: &str) -> PortalInputSubmission {
        PortalInputSubmission {
            input_id: input_id.to_string(),
            submission_text: text.to_string(),
            submitted_at_wall_us: 30,
            expires_at_wall_us: Some(1_000),
            content_classification: ContentClassification::Private,
        }
    }

    fn windows_target() -> WindowsHudTarget {
        WindowsHudTarget {
            target_id: "windows-local".to_string(),
            mcp_url: Some("http://tzehouse-windows.parrot-hen.ts.net:9090/mcp".to_string()),
            grpc_endpoint: Some("tzehouse-windows.parrot-hen.ts.net:50051".to_string()),
            credential_source: HudCredentialSource::EnvVar("TZE_HUD_PSK".to_string()),
            runtime_audience: "local-windows-hud".to_string(),
        }
    }

    fn protected_windows_target() -> WindowsHudTarget {
        WindowsHudTarget {
            credential_source: HudCredentialSource::ProtectedConfigKey(
                "windows-runtime-psk".to_string(),
            ),
            ..windows_target()
        }
    }

    fn managed_zone_session(projection_id: &str) -> ManagedSessionRequest {
        ManagedSessionRequest {
            projection_id: projection_id.to_string(),
            provider_kind: ProviderKind::Codex,
            display_name: "Codex Status".to_string(),
            origin: ManagedSessionOrigin::Attached,
            hud_target_id: "windows-local".to_string(),
            surface_route: PresenceSurfaceRoute::Zone {
                zone_name: "status-bar".to_string(),
                content_kind: "status".to_string(),
                ttl_ms: 10_000,
            },
            content_classification: ContentClassification::Household,
            attention_intent: ProjectionAttentionIntent::Ambient,
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: None,
        }
    }

    fn managed_widget_session(projection_id: &str) -> ManagedSessionRequest {
        let mut parameters = HashMap::new();
        parameters.insert("progress".to_string(), WidgetParameterValue::F32Milli(420));
        ManagedSessionRequest {
            projection_id: projection_id.to_string(),
            provider_kind: ProviderKind::Claude,
            display_name: "Claude Progress".to_string(),
            origin: ManagedSessionOrigin::Launched(LaunchSessionSpec {
                command: "claude".to_string(),
                args: vec!["--continue".to_string()],
                working_directory: Some("/home/tze/gt/tze_hud/mayor/rig".to_string()),
                environment_keys: vec!["ANTHROPIC_API_KEY".to_string()],
            }),
            hud_target_id: "windows-local".to_string(),
            surface_route: PresenceSurfaceRoute::Widget {
                widget_name: "main-progress".to_string(),
                parameters,
                ttl_ms: 10_000,
            },
            content_classification: ContentClassification::Private,
            attention_intent: ProjectionAttentionIntent::Ambient,
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: Some("claude".to_string()),
        }
    }

    fn managed_portal_session(projection_id: &str) -> ManagedSessionRequest {
        ManagedSessionRequest {
            projection_id: projection_id.to_string(),
            provider_kind: ProviderKind::Opencode,
            display_name: "Opencode Questions".to_string(),
            origin: ManagedSessionOrigin::Attached,
            hud_target_id: "windows-local".to_string(),
            surface_route: PresenceSurfaceRoute::Portal {
                portal_surface: PortalSurfaceKind::TextStreamRawTile,
                requested_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                lease_ttl_ms: 30_000,
            },
            content_classification: ContentClassification::Private,
            attention_intent: ProjectionAttentionIntent::Gentle,
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: Some("opencode".to_string()),
        }
    }

    #[test]
    fn external_authority_plans_three_provider_neutral_sessions_across_existing_surfaces() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");

        let zone = authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .expect("zone session is managed");
        let widget = authority
            .manage_session(managed_widget_session("agent-progress"), "manager", 11)
            .expect("widget session is managed");
        let portal = authority
            .manage_session(managed_portal_session("agent-question"), "manager", 12)
            .expect("portal session is managed");

        assert_eq!(authority.managed_session_count(), 3);
        assert!(
            authority
                .projection_authority()
                .has_projection("agent-status")
        );
        assert!(
            authority
                .projection_authority()
                .has_projection("agent-progress")
        );
        assert!(
            authority
                .projection_authority()
                .has_projection("agent-question")
        );

        assert!(matches!(
            zone.route_plan.surface_command,
            HudSurfaceCommandPlan::ZonePublish { .. }
        ));
        assert!(matches!(
            widget.route_plan.surface_command,
            HudSurfaceCommandPlan::WidgetPublish { .. }
        ));
        assert!(matches!(
            portal.route_plan.surface_command,
            HudSurfaceCommandPlan::PortalLease { .. }
        ));
        assert_eq!(
            zone.route_plan.attention_intent,
            ProjectionAttentionIntent::Ambient
        );
        assert_eq!(
            widget.route_plan.attention_intent,
            ProjectionAttentionIntent::Ambient
        );
        assert_eq!(
            portal.route_plan.attention_intent,
            ProjectionAttentionIntent::Gentle
        );

        let demo = authority.three_session_demo_plan();
        assert_eq!(demo.len(), 3);
        assert_eq!(demo[0].projection_id, "agent-progress");
        assert_eq!(demo[1].projection_id, "agent-question");
        assert_eq!(demo[2].projection_id, "agent-status");
    }

    #[test]
    fn external_authority_route_plans_redact_credentials_and_expose_no_capture_authority() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");

        let handle = authority
            .manage_session(managed_widget_session("agent-progress"), "manager", 10)
            .expect("widget session is managed");
        let serialized = serde_json::to_string(&handle.route_plan).unwrap();

        assert!(serialized.contains("env:TZE_HUD_PSK:redacted"));
        assert!(!serialized.contains(&handle.owner_token));
        assert!(!serialized.contains("operator-secret"));
        for forbidden in [
            "pty",
            "terminal_capture",
            "stdin",
            "stdout",
            "raw_keystroke",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "route plan must not expose {forbidden} authority"
            );
        }
    }

    #[test]
    fn external_authority_resolves_runtime_auth_material_without_serializing_secret() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(protected_windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .unwrap();

        let material = authority
            .resolve_runtime_authentication("agent-status", |key| {
                (key == "windows-runtime-psk").then(|| "operator-secret".to_string())
            })
            .expect("runtime auth material resolves");

        assert_eq!(material.target_id, "windows-local");
        assert_eq!(material.runtime_audience, "local-windows-hud");
        assert_eq!(
            material.credential_redacted,
            "protected-config:windows-runtime-psk:redacted"
        );
        assert_eq!(material.credential_secret(), "operator-secret");
        assert!(material.mcp_url.as_deref().unwrap().ends_with(":9090/mcp"));
        assert!(
            material
                .grpc_endpoint
                .as_deref()
                .unwrap()
                .ends_with(":50051")
        );

        let debug = format!("{material:?}");
        assert!(debug.contains("protected-config:windows-runtime-psk:redacted"));
        assert!(!debug.contains("operator-secret"));
    }

    #[test]
    fn external_authority_rejects_missing_runtime_auth_material() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(protected_windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .unwrap();

        assert_eq!(
            authority.resolve_runtime_authentication("agent-status", |_| None),
            Err(ProjectionErrorCode::ProjectionUnauthorized)
        );
        assert_eq!(
            authority.resolve_runtime_authentication("missing-agent", |_| {
                Some("operator-secret".to_string())
            }),
            Err(ProjectionErrorCode::ProjectionNotFound)
        );
    }

    #[cfg(unix)]
    fn managed_process_session(projection_id: &str, args: Vec<String>) -> ManagedSessionRequest {
        let mut request = managed_widget_session(projection_id);
        request.origin = ManagedSessionOrigin::Launched(LaunchSessionSpec {
            command: "sh".to_string(),
            args,
            working_directory: None,
            environment_keys: Vec::new(),
        });
        request
    }

    #[cfg(unix)]
    #[test]
    fn external_authority_launches_provider_process_without_terminal_capture() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(
                managed_process_session(
                    "agent-progress",
                    vec!["-c".to_string(), "exit 0".to_string()],
                ),
                "manager",
                10,
            )
            .unwrap();

        let launched = authority
            .launch_provider_process("agent-progress")
            .expect("provider process launches");
        assert!(launched.process_id > 0);

        let mut final_status = launched;
        for _ in 0..20 {
            final_status = authority
                .provider_process_status("agent-progress")
                .unwrap()
                .expect("process remains tracked");
            if matches!(final_status.state, ProviderProcessState::Exited { .. }) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            final_status.state,
            ProviderProcessState::Exited { code: Some(0) }
        );

        let status_json = serde_json::to_string(&final_status).unwrap();
        for forbidden in ["pty", "terminal", "stdin", "stdout", "stderr"] {
            assert!(
                !status_json.contains(forbidden),
                "process status must not expose {forbidden} capture authority"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn external_authority_rejects_process_launch_for_attached_session() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .unwrap();

        assert_eq!(
            authority.launch_provider_process("agent-status"),
            Err(ProjectionErrorCode::ProjectionInvalidArgument)
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_authority_revoke_terminates_tracked_provider_process() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(
                managed_process_session(
                    "agent-progress",
                    vec!["-c".to_string(), "sleep 30".to_string()],
                ),
                "manager",
                10,
            )
            .unwrap();

        let launched = authority
            .launch_provider_process("agent-progress")
            .expect("provider process launches");
        assert_eq!(launched.state, ProviderProcessState::Running);

        authority.revoke_session("agent-progress").unwrap();
        assert!(
            authority
                .provider_process_status("agent-progress")
                .unwrap()
                .is_none()
        );
        assert_eq!(authority.managed_session_count(), 0);
    }

    #[test]
    fn external_authority_revokes_one_session_without_mutating_others() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .unwrap();
        authority
            .manage_session(managed_widget_session("agent-progress"), "manager", 11)
            .unwrap();
        authority
            .manage_session(managed_portal_session("agent-question"), "manager", 12)
            .unwrap();

        authority.revoke_session("agent-progress").unwrap();

        assert_eq!(authority.managed_session_count(), 2);
        assert!(authority.route_plan("agent-progress").is_none());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-progress")
                .is_none()
        );
        assert!(authority.route_plan("agent-status").is_some());
        assert!(authority.route_plan("agent-question").is_some());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-status")
                .is_some()
        );
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-question")
                .is_some()
        );
    }

    #[test]
    fn external_authority_expiry_purges_managed_session_and_preserves_unexpired_sessions() {
        let mut authority = ExternalAgentProjectionAuthority::new(ProjectionBounds {
            owner_token_ttl_wall_us: 20,
            ..ProjectionBounds::default()
        })
        .unwrap();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_zone_session("agent-status"), "manager", 10)
            .unwrap();
        authority
            .manage_session(managed_portal_session("agent-question"), "manager", 11)
            .unwrap();

        assert_eq!(authority.expire_token_expired_sessions(29), 0);
        assert_eq!(authority.managed_session_count(), 2);

        assert_eq!(authority.expire_token_expired_sessions(30), 1);
        assert_eq!(authority.managed_session_count(), 1);
        assert!(authority.route_plan("agent-status").is_none());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-status")
                .is_none()
        );
        assert!(authority.route_plan("agent-question").is_some());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-question")
                .is_some()
        );

        assert_eq!(authority.expire_token_expired_sessions(31), 1);
        assert_eq!(authority.managed_session_count(), 0);
        assert!(authority.route_plan("agent-question").is_none());
        assert!(
            authority
                .projection_authority()
                .state_summary("agent-question")
                .is_none()
        );
    }

    #[test]
    fn external_authority_reconnect_requires_fresh_runtime_lease_authority() {
        let mut authority = ExternalAgentProjectionAuthority::default();
        authority
            .register_windows_target(windows_target())
            .expect("target is valid");
        authority
            .manage_session(managed_portal_session("agent-question"), "manager", 10)
            .unwrap();
        authority
            .record_hud_connection(
                "agent-question",
                HudConnectionMetadata {
                    connection_id: "connection-1".to_string(),
                    authenticated_session_id: "runtime-session-1".to_string(),
                    granted_capabilities: vec![
                        "create_tiles".to_string(),
                        "modify_own_tiles".to_string(),
                    ],
                    connected_at_wall_us: 20,
                    last_reconnect_wall_us: 20,
                },
            )
            .unwrap();
        authority
            .projection_authority_mut()
            .record_advisory_lease(
                "agent-question",
                AdvisoryLeaseIdentity {
                    lease_id: "lease-1".to_string(),
                    capabilities: vec!["create_tiles".to_string()],
                    acquired_at_wall_us: 21,
                    expires_at_wall_us: 100,
                },
                22,
            )
            .unwrap();

        authority
            .mark_hud_disconnected("agent-question", 30)
            .unwrap();
        let disconnected = authority
            .projection_authority()
            .state_summary("agent-question")
            .unwrap();
        assert_eq!(
            disconnected.lifecycle_state,
            ProjectionLifecycleState::HudUnavailable
        );
        assert!(!disconnected.has_advisory_lease);

        authority
            .record_hud_connection(
                "agent-question",
                HudConnectionMetadata {
                    connection_id: "connection-2".to_string(),
                    authenticated_session_id: "runtime-session-2".to_string(),
                    granted_capabilities: vec![
                        "create_tiles".to_string(),
                        "modify_own_tiles".to_string(),
                    ],
                    connected_at_wall_us: 40,
                    last_reconnect_wall_us: 40,
                },
            )
            .unwrap();
        let stale = authority
            .projection_authority_mut()
            .authorize_portal_republish(
                "agent-question",
                "lease-1",
                &["create_tiles".to_string()],
                41,
            );
        assert_eq!(stale, Err(ProjectionErrorCode::ProjectionUnauthorized));

        authority
            .projection_authority_mut()
            .record_advisory_lease(
                "agent-question",
                AdvisoryLeaseIdentity {
                    lease_id: "lease-2".to_string(),
                    capabilities: vec!["create_tiles".to_string()],
                    acquired_at_wall_us: 42,
                    expires_at_wall_us: 100,
                },
                43,
            )
            .unwrap();
        assert_eq!(
            authority
                .projection_authority_mut()
                .authorize_portal_republish(
                    "agent-question",
                    "lease-2",
                    &["create_tiles".to_string()],
                    44,
                ),
            Ok(())
        );
    }

    #[test]
    fn schema_uses_required_wall_clock_and_owner_token_fields() {
        let attach_json = serde_json::to_value(attach_request("projection-a", "req-a")).unwrap();
        assert_eq!(attach_json["operation"], "attach");
        assert_eq!(attach_json["client_timestamp_wall_us"], 1);
        assert!(attach_json.get("owner_token").is_none());

        let publish_json =
            serde_json::to_value(output_request("projection-a", "owner-token", "req-b")).unwrap();
        assert_eq!(publish_json["operation"], "publish_output");
        assert_eq!(publish_json["owner_token"], "owner-token");
        assert_eq!(publish_json["content_classification"], "private");
    }

    #[test]
    fn stable_error_code_set_is_append_only_wire_shape() {
        let codes: Vec<&str> = INITIAL_ERROR_CODES
            .iter()
            .map(|code| code.as_str())
            .collect();
        assert_eq!(
            codes,
            vec![
                "PROJECTION_NOT_FOUND",
                "PROJECTION_ALREADY_ATTACHED",
                "PROJECTION_UNAUTHORIZED",
                "PROJECTION_TOKEN_EXPIRED",
                "PROJECTION_INVALID_ARGUMENT",
                "PROJECTION_OUTPUT_TOO_LARGE",
                "PROJECTION_INPUT_TOO_LARGE",
                "PROJECTION_INPUT_QUEUE_FULL",
                "PROJECTION_RATE_LIMITED",
                "PROJECTION_STATE_CONFLICT",
                "PROJECTION_HUD_UNAVAILABLE",
                "PROJECTION_INTERNAL_ERROR",
            ]
        );
        assert_eq!(
            serde_json::to_string(&ProjectionErrorCode::ProjectionUnauthorized).unwrap(),
            "\"PROJECTION_UNAUTHORIZED\""
        );
    }

    #[test]
    fn attach_materializes_content_layer_projected_portal_and_reuses_idempotently() {
        let mut authority = ProjectionAuthority::default();
        let first =
            authority.handle_attach(attach_request("projection-a", "req-a"), "caller-a", 10);
        assert!(first.accepted);

        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .expect("attach creates portal state");
        assert_eq!(
            state.adapter_family,
            ProjectedPortalAdapterFamily::CooperativeProjection
        );
        assert_eq!(
            state.runtime_authority,
            ProjectedPortalRuntimeAuthority::ResidentSessionLease
        );
        assert_eq!(state.layer, ProjectedPortalLayer::Content);
        assert_eq!(state.presentation, ProjectedPortalPresentation::Expanded);
        assert_eq!(state.display_name.as_deref(), Some("Codex Session"));
        assert_eq!(state.workspace_hint.as_deref(), Some("mayor/rig"));
        assert!(state.interaction_enabled);

        authority.collapse_projected_portal("projection-a").unwrap();
        let collapsed = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .expect("collapsed portal state remains materializable");
        assert_eq!(collapsed.portal_id, state.portal_id);
        assert_eq!(
            collapsed.presentation,
            ProjectedPortalPresentation::Collapsed
        );
        assert!(collapsed.visible_transcript.is_empty());
        assert!(!collapsed.interaction_enabled);

        let replay =
            authority.handle_attach(attach_request("projection-a", "req-b"), "caller-a", 11);
        assert!(replay.accepted);
        assert!(replay.owner_token.is_none());
        assert_eq!(
            authority
                .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
                .unwrap()
                .portal_id,
            state.portal_id
        );
    }

    #[test]
    fn successful_attach_issues_high_entropy_token_and_stores_only_verifier() {
        let mut authority = ProjectionAuthority::default();
        let response =
            authority.handle_attach(attach_request("projection-a", "req-a"), "caller-a", 10);
        assert!(response.accepted);
        let owner_token = response.owner_token.expect("attach must return token once");
        assert_eq!(owner_token.len(), OWNER_TOKEN_ENTROPY_BITS / 4);
        assert!(owner_token.chars().all(|ch| ch.is_ascii_hexdigit()));

        let verifier = authority
            .owner_token_verifier_for_test("projection-a")
            .expect("session stores verifier");
        assert_ne!(verifier, owner_token);
        assert_eq!(verifier.len(), 64);
    }

    #[test]
    fn attach_conflict_is_deterministic_and_idempotent_replay_does_not_expose_token() {
        let mut authority = ProjectionAuthority::default();
        let first =
            authority.handle_attach(attach_request("projection-a", "req-a"), "caller-a", 10);
        assert!(first.accepted);
        assert!(first.owner_token.is_some());

        let replay =
            authority.handle_attach(attach_request("projection-a", "req-b"), "caller-a", 11);
        assert!(replay.accepted);
        assert!(replay.owner_token.is_none());

        let mut conflicting = attach_request("projection-a", "req-c");
        conflicting.idempotency_key = Some("different-key".to_string());
        let conflict = authority.handle_attach(conflicting, "caller-b", 12);
        assert!(!conflict.accepted);
        assert_eq!(
            conflict.error_code,
            Some(ProjectionErrorCode::ProjectionAlreadyAttached)
        );
    }

    #[test]
    fn cross_projection_read_fails_closed_and_audits_without_payload_text() {
        let mut authority = ProjectionAuthority::default();
        let _token_a = attach(&mut authority, "projection-a");
        let token_b = attach(&mut authority, "projection-b");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "private operator text".to_string(),
                20,
                1_000,
                None,
            )
            .unwrap();

        let denied = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-read",
                ),
                owner_token: token_b,
                max_items: None,
                max_bytes: None,
            },
            "caller-b",
            30,
        );
        assert!(!denied.accepted);
        assert_eq!(
            denied.error_code,
            Some(ProjectionErrorCode::ProjectionUnauthorized)
        );
        assert!(denied.pending_input.is_empty());

        let audit = authority.audit_log().last().expect("denial audit exists");
        assert_eq!(audit.category, ProjectionAuditCategory::AuthDenied);
        assert_eq!(
            audit.error_code,
            Some(ProjectionErrorCode::ProjectionUnauthorized)
        );
        assert!(!audit.reason.contains("private operator text"));
    }

    #[test]
    fn oversized_output_is_rejected_with_stable_code() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_output_bytes_per_call: 4,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        let mut request = output_request("projection-a", &owner_token, "req-output");
        request.output_text = "too large".to_string();

        let response = authority.handle_publish_output(request, "caller-a", 20);
        assert!(!response.accepted);
        assert_eq!(
            response.error_code,
            Some(ProjectionErrorCode::ProjectionOutputTooLarge)
        );
    }

    #[test]
    fn logical_unit_id_replay_is_idempotent() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let first = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output-1"),
            "caller-a",
            20,
        );
        let replay = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output-2"),
            "caller-a",
            21,
        );
        assert!(first.accepted);
        assert!(replay.accepted);
        assert!(replay.status_summary.contains("idempotently"));
    }

    #[test]
    fn logical_unit_id_cache_is_bounded() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_seen_logical_units: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        let first = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output-1"),
            "caller-a",
            20,
        );
        let mut second_request = output_request("projection-a", &owner_token, "req-output-2");
        second_request.logical_unit_id = Some("unit-2".to_string());
        let second = authority.handle_publish_output(second_request, "caller-a", 21);
        let first_again = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output-3"),
            "caller-a",
            22,
        );

        assert!(first.accepted);
        assert!(second.accepted);
        assert!(first_again.accepted);
        assert!(!first_again.status_summary.contains("idempotently"));
    }

    #[test]
    fn audit_log_is_bounded_without_payload_text() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_audit_records: 2,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        for index in 0..3 {
            let mut request =
                output_request("projection-a", &owner_token, &format!("req-output-{index}"));
            request.output_text = format!("private transcript {index}");
            let response = authority.handle_publish_output(request, "caller-a", 20 + index);
            assert!(response.accepted);
        }

        assert_eq!(authority.audit_log().len(), 2);
        assert!(
            authority
                .audit_log()
                .iter()
                .all(|audit| !audit.reason.contains("private transcript"))
        );
    }

    #[test]
    fn portal_composer_submission_is_transactional_bounded_inbox_feedback() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_pending_input_items: 1,
            max_pending_input_bytes_per_item: 4,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        let oversized = authority.submit_portal_input(
            "projection-a",
            portal_submission("input-too-large", "12345"),
        );
        assert_eq!(oversized.feedback_state, PortalInputFeedbackState::Rejected);
        assert_eq!(
            oversized.error_code,
            Some(ProjectionErrorCode::ProjectionInputTooLarge)
        );
        assert_eq!(oversized.pending_input_count, 0);

        let accepted =
            authority.submit_portal_input("projection-a", portal_submission("input-1", "ok"));
        assert_eq!(accepted.feedback_state, PortalInputFeedbackState::Accepted);
        assert_eq!(accepted.pending_input_count, 1);
        assert_eq!(accepted.pending_input_bytes, 2);

        let full =
            authority.submit_portal_input("projection-a", portal_submission("input-2", "yo"));
        assert_eq!(full.feedback_state, PortalInputFeedbackState::Rejected);
        assert_eq!(
            full.error_code,
            Some(ProjectionErrorCode::ProjectionInputQueueFull)
        );
        assert_eq!(full.pending_input_count, 1);

        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .expect("portal state includes pending feedback");
        assert_eq!(state.pending_input_count, Some(1));
        assert_eq!(state.pending_input_bytes, Some(2));
        assert_eq!(
            state.last_input_feedback.as_ref().map(|f| f.feedback_state),
            Some(PortalInputFeedbackState::Rejected)
        );
        assert_eq!(
            state
                .last_input_feedback
                .as_ref()
                .map(|f| f.input_id.as_str()),
            Some("input-2")
        );

        let poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token,
                max_items: None,
                max_bytes: None,
            },
            "caller-a",
            40,
        );
        assert!(poll.accepted);
        assert_eq!(poll.pending_input.len(), 1);
        assert_eq!(poll.pending_input[0].input_id, "input-1");
        assert_eq!(poll.pending_input[0].submission_text, "ok");
    }

    #[test]
    fn default_bounds_match_projection_spec_values() {
        let bounds = ProjectionBounds::default();
        assert_eq!(
            bounds.max_output_bytes_per_call,
            DEFAULT_MAX_OUTPUT_BYTES_PER_CALL
        );
        assert_eq!(bounds.max_status_text_bytes, DEFAULT_MAX_STATUS_TEXT_BYTES);
        assert_eq!(
            bounds.max_retained_transcript_bytes,
            DEFAULT_MAX_RETAINED_TRANSCRIPT_BYTES
        );
        assert_eq!(
            bounds.max_visible_transcript_bytes,
            DEFAULT_MAX_VISIBLE_TRANSCRIPT_BYTES
        );
        assert_eq!(
            bounds.max_pending_input_items,
            DEFAULT_MAX_PENDING_INPUT_ITEMS
        );
        assert_eq!(
            bounds.max_pending_input_bytes_per_item,
            DEFAULT_MAX_PENDING_INPUT_BYTES_PER_ITEM
        );
        assert_eq!(
            bounds.max_pending_input_total_bytes,
            DEFAULT_MAX_PENDING_INPUT_TOTAL_BYTES
        );
        assert_eq!(bounds.max_poll_items, DEFAULT_MAX_POLL_ITEMS);
        assert_eq!(
            bounds.max_poll_response_bytes,
            DEFAULT_MAX_POLL_RESPONSE_BYTES
        );
        assert_eq!(
            bounds.max_portal_updates_per_second,
            DEFAULT_MAX_PORTAL_UPDATES_PER_SECOND
        );
    }

    #[test]
    fn collapsed_redacted_projection_preserves_geometry_and_suppresses_private_affordances() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let mut output = output_request("projection-a", &owner_token, "req-output");
        output.output_text = "private projected transcript".to_string();
        assert!(
            authority
                .handle_publish_output(output, "caller-a", 20)
                .accepted
        );
        let feedback =
            authority.submit_portal_input("projection-a", portal_submission("input-1", "help"));
        assert_eq!(feedback.feedback_state, PortalInputFeedbackState::Accepted);
        authority.collapse_projected_portal("projection-a").unwrap();

        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::default())
            .expect("redacted portal still materializes");
        assert_eq!(state.presentation, ProjectedPortalPresentation::Collapsed);
        assert!(state.preserve_geometry);
        assert!(state.redacted);
        assert!(!state.interaction_enabled);
        assert_eq!(state.layer, ProjectedPortalLayer::Content);
        assert!(state.provider_kind.is_none());
        assert!(state.display_name.is_none());
        assert!(state.workspace_hint.is_none());
        assert!(state.lifecycle_state.is_none());
        assert!(state.visible_transcript.is_empty());
        assert_eq!(state.unread_output_count, None);
        assert_eq!(state.pending_input_count, None);
        assert!(
            !serde_json::to_string(&state)
                .unwrap()
                .contains("private projected transcript")
        );
    }

    #[test]
    fn portal_submission_default_ttl_overflow_is_rejected_not_panicked() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");

        let feedback = authority.submit_portal_input(
            "projection-a",
            PortalInputSubmission {
                input_id: "input-overflow".to_string(),
                submission_text: "help".to_string(),
                submitted_at_wall_us: u64::MAX,
                expires_at_wall_us: None,
                content_classification: ContentClassification::Private,
            },
        );

        assert_eq!(feedback.feedback_state, PortalInputFeedbackState::Rejected);
        assert_eq!(
            feedback.error_code,
            Some(ProjectionErrorCode::ProjectionInvalidArgument)
        );
        assert_eq!(feedback.pending_input_count, 0);
    }

    #[test]
    fn projection_private_state_is_memory_only_and_purged_on_detach_cleanup_and_expiry() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            owner_token_ttl_wall_us: 30,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .record_hud_connection(
                "projection-a",
                connection_metadata(&["create_tiles", "modify_own_tiles"]),
            )
            .unwrap();
        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 22)
            .unwrap();
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "operator private text".to_string(),
                23,
                100,
                None,
            )
            .unwrap();
        let mut output = output_request("projection-a", &owner_token, "req-output");
        output.output_text = "private transcript text".to_string();
        assert!(
            authority
                .handle_publish_output(output, "caller-a", 24)
                .accepted
        );

        let summary = authority.state_summary("projection-a").unwrap();
        assert!(summary.has_hud_connection);
        assert!(summary.has_advisory_lease);
        assert_eq!(summary.pending_input_count, 1);
        assert!(summary.retained_transcript_bytes > 0);
        assert_eq!(
            authority.visible_transcript_window("projection-a").unwrap()[0].output_text,
            "private transcript text"
        );

        let detached = authority.handle_detach(
            DetachRequest {
                envelope: envelope(ProjectionOperation::Detach, "projection-a", "req-detach"),
                owner_token: owner_token.clone(),
                reason: "done".to_string(),
            },
            "caller-a",
            25,
        );
        assert!(detached.accepted);
        assert!(!authority.has_projection("projection-a"));
        assert!(
            authority
                .audit_log()
                .iter()
                .all(|audit| !audit.reason.contains("private transcript")
                    && !audit.reason.contains("operator private text"))
        );

        let mut restarted = ProjectionAuthority::default();
        assert!(!restarted.has_projection("projection-a"));
        let fresh = attach(&mut restarted, "projection-a");
        assert_ne!(fresh, owner_token);

        let expired_token = attach(&mut restarted, "projection-expiring");
        assert!(restarted.has_projection("projection-expiring"));
        let expired = restarted.handle_publish_status(
            PublishStatusRequest {
                envelope: envelope(
                    ProjectionOperation::PublishStatus,
                    "projection-expiring",
                    "req-expired",
                ),
                owner_token: expired_token,
                lifecycle_state: ProjectionLifecycleState::Active,
                status_text: None,
            },
            "caller-a",
            DEFAULT_OWNER_TOKEN_TTL_WALL_US + 20,
        );
        assert!(!expired.accepted);
        assert_eq!(
            expired.error_code,
            Some(ProjectionErrorCode::ProjectionTokenExpired)
        );
        assert!(!restarted.has_projection("projection-expiring"));
    }

    #[test]
    fn stale_or_overbroad_lease_identity_cannot_authorize_republish() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");
        authority
            .record_hud_connection(
                "projection-a",
                connection_metadata(&["create_tiles", "modify_own_tiles"]),
            )
            .unwrap();
        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 22)
            .unwrap();

        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                30
            ),
            Ok(())
        );
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("upload_resource")],
                31
            ),
            Err(ProjectionErrorCode::ProjectionUnauthorized)
        );
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                101
            ),
            Err(ProjectionErrorCode::ProjectionTokenExpired)
        );
        assert!(
            !authority
                .state_summary("projection-a")
                .unwrap()
                .has_advisory_lease
        );

        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 200), 120)
            .unwrap();
        authority
            .mark_hud_disconnected("projection-a", 130)
            .unwrap();
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                131
            ),
            Err(ProjectionErrorCode::ProjectionHudUnavailable)
        );
    }

    #[test]
    fn reconnect_updates_bookkeeping_and_requires_fresh_lease() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");
        authority
            .record_hud_connection(
                "projection-a",
                connection_metadata(&["create_tiles", "modify_own_tiles"]),
            )
            .unwrap();
        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 22)
            .unwrap();

        let mut reconnected = connection_metadata(&["create_tiles", "modify_own_tiles"]);
        reconnected.connection_id = "connection-2".to_string();
        reconnected.authenticated_session_id = "runtime-session-2".to_string();
        reconnected.connected_at_wall_us = 40;
        reconnected.last_reconnect_wall_us = 40;
        authority
            .record_hud_connection("projection-a", reconnected)
            .unwrap();

        let summary = authority.state_summary("projection-a").unwrap();
        assert_eq!(summary.reconnect.reconnect_count, 1);
        assert_eq!(summary.reconnect.last_reconnect_wall_us, Some(40));
        assert!(!summary.has_advisory_lease);
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                41
            ),
            Err(ProjectionErrorCode::ProjectionUnauthorized)
        );

        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 42)
            .unwrap();
        authority.mark_hud_disconnected("projection-a", 50).unwrap();
        let mut after_disconnect = connection_metadata(&["create_tiles"]);
        after_disconnect.connection_id = "connection-3".to_string();
        after_disconnect.authenticated_session_id = "runtime-session-3".to_string();
        after_disconnect.connected_at_wall_us = 60;
        after_disconnect.last_reconnect_wall_us = 60;
        authority
            .record_hud_connection("projection-a", after_disconnect)
            .unwrap();

        let summary = authority.state_summary("projection-a").unwrap();
        assert_eq!(summary.reconnect.reconnect_count, 2);
        assert_eq!(summary.reconnect.last_disconnect_wall_us, Some(50));
        assert!(!summary.has_advisory_lease);
    }

    #[test]
    fn heartbeat_requires_live_connection_and_is_monotonic() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");

        assert_eq!(
            authority.record_heartbeat("projection-a", 25),
            Err(ProjectionErrorCode::ProjectionHudUnavailable)
        );

        authority
            .record_hud_connection("projection-a", connection_metadata(&["create_tiles"]))
            .unwrap();
        authority.record_heartbeat("projection-a", 30).unwrap();
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .reconnect
                .last_heartbeat_wall_us,
            Some(30)
        );
        assert_eq!(
            authority.record_heartbeat("projection-a", 29),
            Err(ProjectionErrorCode::ProjectionStateConflict)
        );

        authority.mark_hud_disconnected("projection-a", 40).unwrap();
        assert_eq!(
            authority.record_heartbeat("projection-a", 41),
            Err(ProjectionErrorCode::ProjectionHudUnavailable)
        );
    }

    #[test]
    fn reconnect_preserves_transcript_inbox_ack_state_and_requires_new_lease() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let mut output = output_request("projection-a", &owner_token, "req-output");
        output.output_text = "retained across HUD reconnect".to_string();
        assert!(
            authority
                .handle_publish_output(output, "caller-a", 20)
                .accepted
        );
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "operator input survives reconnect".to_string(),
                21,
                1_000,
                None,
            )
            .unwrap();
        let delivered = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(1),
                max_bytes: None,
            },
            "caller-a",
            22,
        );
        assert!(delivered.accepted);
        authority
            .record_hud_connection("projection-a", connection_metadata(&["create_tiles"]))
            .unwrap();
        authority
            .record_advisory_lease("projection-a", advisory_lease(&["create_tiles"], 100), 23)
            .unwrap();

        authority.mark_hud_disconnected("projection-a", 30).unwrap();
        let mut reconnected = connection_metadata(&["create_tiles"]);
        reconnected.connection_id = "connection-after-drop".to_string();
        reconnected.authenticated_session_id = "runtime-session-after-drop".to_string();
        reconnected.connected_at_wall_us = 40;
        reconnected.last_reconnect_wall_us = 40;
        authority
            .record_hud_connection("projection-a", reconnected)
            .unwrap();

        let summary = authority.state_summary("projection-a").unwrap();
        assert_eq!(summary.retained_transcript_units, 1);
        assert_eq!(summary.pending_input_count, 1);
        assert!(!summary.has_advisory_lease);
        assert_eq!(
            authority.visible_transcript_window("projection-a").unwrap()[0].output_text,
            "retained across HUD reconnect"
        );
        assert_eq!(
            authority.authorize_portal_republish(
                "projection-a",
                "lease-1",
                &[String::from("create_tiles")],
                41
            ),
            Err(ProjectionErrorCode::ProjectionUnauthorized)
        );

        let handled_after_reconnect = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-after-reconnect",
                ),
                owner_token,
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            42,
        );
        assert!(handled_after_reconnect.accepted);
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .pending_input_count,
            0
        );
    }

    #[test]
    fn owner_degraded_lifecycle_is_not_overwritten_by_connection_or_output() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let degraded = authority.handle_publish_status(
            PublishStatusRequest {
                envelope: envelope(
                    ProjectionOperation::PublishStatus,
                    "projection-a",
                    "req-status",
                ),
                owner_token: owner_token.clone(),
                lifecycle_state: ProjectionLifecycleState::Degraded,
                status_text: Some("HUD projection is degraded".to_string()),
            },
            "caller-a",
            20,
        );
        assert!(degraded.accepted);

        authority
            .record_hud_connection("projection-a", connection_metadata(&["create_tiles"]))
            .unwrap();
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .lifecycle_state,
            ProjectionLifecycleState::Degraded
        );

        let published = authority.handle_publish_output(
            output_request("projection-a", &owner_token, "req-output"),
            "caller-a",
            21,
        );
        assert!(published.accepted);
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .lifecycle_state,
            ProjectionLifecycleState::Degraded
        );
    }

    #[test]
    fn acknowledgement_and_detach_cleanup_update_projected_portal_state() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        let accepted =
            authority.submit_portal_input("projection-a", portal_submission("input-1", "ok"));
        assert_eq!(accepted.feedback_state, PortalInputFeedbackState::Accepted);
        assert_eq!(
            authority
                .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
                .unwrap()
                .pending_input_count,
            Some(1)
        );

        let poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token: owner_token.clone(),
                max_items: None,
                max_bytes: None,
            },
            "caller-a",
            40,
        );
        assert!(poll.accepted);
        let handled = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            41,
        );
        assert!(handled.accepted);
        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .unwrap();
        assert_eq!(state.pending_input_count, Some(0));
        assert_eq!(state.pending_input_bytes, Some(0));

        let detached = authority.handle_detach(
            DetachRequest {
                envelope: envelope(ProjectionOperation::Detach, "projection-a", "req-detach"),
                owner_token,
                reason: "session complete".to_string(),
            },
            "caller-a",
            42,
        );
        assert!(detached.accepted);
        assert!(
            authority
                .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
                .is_none()
        );
    }

    #[test]
    fn projected_portal_contract_has_no_terminal_or_process_authority() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "projection-a");
        let state = authority
            .projected_portal_state("projection-a", &ProjectedPortalPolicy::permit_all())
            .expect("portal state exists");

        assert_eq!(
            state.adapter_family,
            ProjectedPortalAdapterFamily::CooperativeProjection
        );
        assert_eq!(
            state.runtime_authority,
            ProjectedPortalRuntimeAuthority::ResidentSessionLease
        );
        let wire = serde_json::to_string(&state).unwrap();
        for forbidden in ["pty", "tmux", "terminal", "stdin", "process"] {
            assert!(
                !wire.contains(forbidden),
                "projected portal state must not expose {forbidden} authority"
            );
        }
    }

    #[test]
    fn provider_kind_does_not_change_projection_semantics() {
        for (index, provider_kind) in [
            ProviderKind::Codex,
            ProviderKind::Claude,
            ProviderKind::Opencode,
            ProviderKind::Other,
        ]
        .into_iter()
        .enumerate()
        {
            let projection_id = format!("projection-provider-{index}");
            let mut authority = ProjectionAuthority::default();
            let attach = authority.handle_attach(
                AttachRequest {
                    provider_kind,
                    display_name: format!("Provider {index}"),
                    ..attach_request(&projection_id, "req-attach")
                },
                "caller-a",
                10,
            );
            assert!(attach.accepted);
            let owner_token = attach.owner_token.expect("attach returns owner token");

            assert!(
                authority
                    .handle_publish_output(
                        output_request(&projection_id, &owner_token, "req-output"),
                        "caller-a",
                        20,
                    )
                    .accepted
            );
            let feedback =
                authority.submit_portal_input(&projection_id, portal_submission("input-1", "ok"));
            assert_eq!(feedback.feedback_state, PortalInputFeedbackState::Accepted);
            let poll = authority.handle_get_pending_input(
                GetPendingInputRequest {
                    envelope: envelope(
                        ProjectionOperation::GetPendingInput,
                        &projection_id,
                        "req-poll",
                    ),
                    owner_token: owner_token.clone(),
                    max_items: None,
                    max_bytes: None,
                },
                "caller-a",
                30,
            );
            assert!(poll.accepted);
            assert_eq!(poll.pending_input.len(), 1);
            let detached = authority.handle_detach(
                DetachRequest {
                    envelope: envelope(ProjectionOperation::Detach, &projection_id, "req-detach"),
                    owner_token,
                    reason: "done".to_string(),
                },
                "caller-a",
                40,
            );
            assert!(detached.accepted);
            assert!(!authority.has_projection(&projection_id));
        }
    }

    #[test]
    fn ready_portal_update_can_be_taken_without_spending_another_rate_slot() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        let mut first = output_request("projection-a", &owner_token, "req-output-1");
        first.output_text = "first".to_string();
        first.logical_unit_id = Some("unit-first".to_string());
        let first_response = authority.handle_publish_output(first, "caller-a", 20);
        assert!(first_response.accepted);
        assert!(first_response.portal_update_ready);

        let immediate = authority
            .take_due_portal_update("projection-a", 20)
            .unwrap()
            .expect("ready publish should be immediately materializable");
        assert_eq!(immediate.unread_output_count, 1);
        assert_eq!(immediate.coalesced_output_count, 0);
        assert!(
            authority
                .take_due_portal_update("projection-a", 20)
                .unwrap()
                .is_none()
        );

        let mut second = output_request("projection-a", &owner_token, "req-output-2");
        second.output_text = "second".to_string();
        second.logical_unit_id = Some("unit-second".to_string());
        let second_response = authority.handle_publish_output(second, "caller-a", 20);
        assert!(second_response.accepted);
        assert!(!second_response.portal_update_ready);
        assert!(
            authority
                .take_due_portal_update("projection-a", 20)
                .unwrap()
                .is_none()
        );

        let coalesced = authority
            .take_due_portal_update("projection-a", PORTAL_UPDATE_RATE_WINDOW_WALL_US + 20)
            .unwrap()
            .expect("coalesced publish should become due in the next rate window");
        assert_eq!(coalesced.unread_output_count, 1);
        assert_eq!(coalesced.coalesced_output_count, 1);
        assert_eq!(
            coalesced
                .visible_transcript
                .last()
                .expect("visible transcript includes second publish")
                .output_text,
            "second"
        );
    }

    #[test]
    fn transcript_pruning_and_portal_update_rate_coalescing_are_bounded() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_retained_transcript_bytes: 12,
            max_visible_transcript_bytes: 8,
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");

        let mut first = output_request("projection-a", &owner_token, "req-output-1");
        first.output_text = "aaaa".to_string();
        first.logical_unit_id = Some("unit-a".to_string());
        let first_response = authority.handle_publish_output(first, "caller-a", 20);
        assert!(first_response.accepted);
        assert!(first_response.portal_update_ready);

        let mut second = output_request("projection-a", &owner_token, "req-output-2");
        second.output_text = "bbbb".to_string();
        second.logical_unit_id = Some("unit-b".to_string());
        second.coalesce_key = Some("status-line".to_string());
        let second_response = authority.handle_publish_output(second, "caller-a", 20);
        assert!(second_response.accepted);
        assert!(!second_response.portal_update_ready);
        assert_eq!(second_response.coalesced_output_count, 1);

        let mut third = output_request("projection-a", &owner_token, "req-output-3");
        third.output_text = "cccc".to_string();
        third.logical_unit_id = Some("unit-c".to_string());
        third.coalesce_key = Some("status-line".to_string());
        let third_response = authority.handle_publish_output(third, "caller-a", 20);
        assert!(third_response.accepted);
        assert!(!third_response.portal_update_ready);
        assert_eq!(third_response.coalesced_output_count, 2);

        let retained = authority.visible_transcript_window("projection-a").unwrap();
        assert_eq!(
            retained
                .iter()
                .filter(|unit| unit.coalesce_key.as_deref() == Some("status-line"))
                .count(),
            1
        );
        assert_eq!(
            retained
                .iter()
                .find(|unit| unit.coalesce_key.as_deref() == Some("status-line"))
                .unwrap()
                .output_text,
            "cccc"
        );

        let mut fourth = output_request("projection-a", &owner_token, "req-output-4");
        fourth.output_text = "dddd".to_string();
        fourth.logical_unit_id = Some("unit-d".to_string());
        assert!(
            authority
                .handle_publish_output(fourth, "caller-a", 20)
                .accepted
        );

        let mut fifth = output_request("projection-a", &owner_token, "req-output-5");
        fifth.output_text = "eeee".to_string();
        fifth.logical_unit_id = Some("unit-e".to_string());
        let fifth_response = authority.handle_publish_output(
            fifth,
            "caller-a",
            PORTAL_UPDATE_RATE_WINDOW_WALL_US + 21,
        );
        assert!(fifth_response.accepted);
        assert!(fifth_response.portal_update_ready);
        assert_eq!(fifth_response.coalesced_output_count, 3);

        let summary = authority.state_summary("projection-a").unwrap();
        assert!(summary.retained_transcript_bytes <= 12);
        assert!(summary.visible_transcript_bytes <= 8);
        assert_eq!(summary.retained_transcript_units, 3);

        let update = authority
            .take_due_portal_update("projection-a", (PORTAL_UPDATE_RATE_WINDOW_WALL_US * 2) + 22)
            .unwrap()
            .expect("update should be due in the next rate window");
        assert!(update.visible_transcript_bytes <= 8);
        assert_eq!(update.coalesced_output_count, 3);
        assert_eq!(update.unread_output_count, 5);
        assert_eq!(
            authority
                .state_summary("projection-a")
                .unwrap()
                .unread_output_count,
            0
        );
    }

    #[test]
    fn pending_input_bounds_and_acknowledgement_state_conflicts_are_enforced() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_pending_input_items: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "first".to_string(),
                20,
                1_000,
                None,
            )
            .unwrap();
        assert_eq!(
            authority.enqueue_input(
                "projection-a",
                "input-2",
                "second".to_string(),
                21,
                1_000,
                None
            ),
            Err(ProjectionErrorCode::ProjectionInputQueueFull)
        );

        let poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(8),
                max_bytes: None,
            },
            "caller-a",
            30,
        );
        assert!(poll.accepted);
        assert_eq!(
            poll.pending_input[0].delivery_state,
            InputDeliveryState::Delivered
        );

        let handled = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-1",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            31,
        );
        assert!(handled.accepted);

        let replay = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-2",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            32,
        );
        assert!(replay.accepted);

        let conflict = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-3",
                ),
                owner_token,
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Rejected,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            33,
        );
        assert!(!conflict.accepted);
        assert_eq!(
            conflict.error_code,
            Some(ProjectionErrorCode::ProjectionStateConflict)
        );
    }

    #[test]
    fn deferred_input_redelivers_after_not_before_and_expires_before_delivery() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "defer me".to_string(),
                20,
                100,
                None,
            )
            .unwrap();
        authority
            .enqueue_input(
                "projection-a",
                "input-2",
                "expire me".to_string(),
                21,
                45,
                None,
            )
            .unwrap();

        let first_poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll-1",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(1),
                max_bytes: None,
            },
            "caller-a",
            30,
        );
        assert_eq!(first_poll.pending_input[0].input_id, "input-1");

        let deferred = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-defer",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Deferred,
                ack_message: None,
                not_before_wall_us: Some(60),
            },
            "caller-a",
            31,
        );
        assert!(deferred.accepted);

        let hidden = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll-2",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(8),
                max_bytes: None,
            },
            "caller-a",
            50,
        );
        assert!(hidden.pending_input.is_empty());

        let redelivered = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll-3",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(8),
                max_bytes: None,
            },
            "caller-a",
            61,
        );
        assert_eq!(redelivered.pending_input.len(), 1);
        assert_eq!(redelivered.pending_input[0].input_id, "input-1");
        assert_eq!(
            redelivered.pending_input[0].delivery_state,
            InputDeliveryState::Delivered
        );

        let expired_ack = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-expired-ack",
                ),
                owner_token,
                input_id: "input-2".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            62,
        );
        assert!(!expired_ack.accepted);
        assert_eq!(
            expired_ack.error_code,
            Some(ProjectionErrorCode::ProjectionStateConflict)
        );
    }

    #[test]
    fn terminal_pending_input_is_pruned_without_losing_ack_replay() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_pending_input_items: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "first".to_string(),
                20,
                1_000,
                None,
            )
            .unwrap();

        let poll = authority.handle_get_pending_input(
            GetPendingInputRequest {
                envelope: envelope(
                    ProjectionOperation::GetPendingInput,
                    "projection-a",
                    "req-poll",
                ),
                owner_token: owner_token.clone(),
                max_items: Some(8),
                max_bytes: None,
            },
            "caller-a",
            30,
        );
        assert!(poll.accepted);

        let handled = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-1",
                ),
                owner_token: owner_token.clone(),
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            31,
        );
        assert!(handled.accepted);

        authority
            .enqueue_input(
                "projection-a",
                "input-2",
                "second".to_string(),
                32,
                1_000,
                None,
            )
            .unwrap();

        let replay = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack-2",
                ),
                owner_token,
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: None,
            },
            "caller-a",
            33,
        );
        assert!(replay.accepted);
        assert!(replay.status_summary.contains("idempotently"));
    }

    #[test]
    fn not_before_is_rejected_for_terminal_acknowledgements() {
        let mut authority = ProjectionAuthority::default();
        let owner_token = attach(&mut authority, "projection-a");
        authority
            .enqueue_input(
                "projection-a",
                "input-1",
                "first".to_string(),
                20,
                1_000,
                None,
            )
            .unwrap();

        let response = authority.handle_acknowledge_input(
            AcknowledgeInputRequest {
                envelope: envelope(
                    ProjectionOperation::AcknowledgeInput,
                    "projection-a",
                    "req-ack",
                ),
                owner_token,
                input_id: "input-1".to_string(),
                ack_state: InputAckState::Handled,
                ack_message: None,
                not_before_wall_us: Some(50),
            },
            "caller-a",
            30,
        );

        assert!(!response.accepted);
        assert_eq!(
            response.error_code,
            Some(ProjectionErrorCode::ProjectionInvalidArgument)
        );
    }

    #[test]
    fn bounded_copy_preserves_utf8_boundaries() {
        assert_eq!(bounded_copy("hello".to_string(), 10), "hello");
        assert_eq!(bounded_copy("éclair".to_string(), 1), "");
        assert_eq!(bounded_copy("aéclair".to_string(), 2), "a");
    }

    #[test]
    fn owner_cleanup_and_operator_cleanup_use_distinct_authority_paths() {
        let mut authority = ProjectionAuthority::default();
        authority.set_operator_authority("operator-secret").unwrap();
        let owner_token = attach(&mut authority, "projection-owner");
        attach(&mut authority, "projection-operator");

        let owner_cleanup = authority.handle_cleanup(
            CleanupRequest {
                envelope: envelope(
                    ProjectionOperation::Cleanup,
                    "projection-owner",
                    "req-owner-cleanup",
                ),
                cleanup_authority: CleanupAuthority::Owner,
                owner_token: Some(owner_token),
                operator_authority: None,
                reason: "owner requested detach".to_string(),
            },
            "caller-a",
            40,
        );
        assert!(owner_cleanup.accepted);

        let operator_cleanup = authority.handle_cleanup(
            CleanupRequest {
                envelope: envelope(
                    ProjectionOperation::Cleanup,
                    "projection-operator",
                    "req-operator-cleanup",
                ),
                cleanup_authority: CleanupAuthority::Operator,
                owner_token: None,
                operator_authority: Some("operator-secret".to_string()),
                reason: "operator override".to_string(),
            },
            "operator",
            41,
        );
        assert!(operator_cleanup.accepted);

        assert!(
            authority
                .audit_log()
                .iter()
                .any(|audit| audit.category == ProjectionAuditCategory::OwnerCleanup)
        );
        assert!(
            authority
                .audit_log()
                .iter()
                .any(|audit| audit.category == ProjectionAuditCategory::OperatorCleanup)
        );
    }

    #[test]
    fn token_expiry_fails_with_stable_code() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            owner_token_ttl_wall_us: 5,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "projection-a");
        let response = authority.handle_publish_status(
            PublishStatusRequest {
                envelope: envelope(
                    ProjectionOperation::PublishStatus,
                    "projection-a",
                    "req-status",
                ),
                owner_token,
                lifecycle_state: ProjectionLifecycleState::Active,
                status_text: None,
            },
            "caller-a",
            20,
        );
        assert!(!response.accepted);
        assert_eq!(
            response.error_code,
            Some(ProjectionErrorCode::ProjectionTokenExpired)
        );
        assert!(!authority.has_projection("projection-a"));
    }

    proptest! {
        #[test]
        fn pending_input_polling_is_fifo_and_bounded(
            item_count in 1usize..24,
            requested_items in 1usize..12,
            requested_bytes in 1usize..96,
        ) {
            let mut authority = ProjectionAuthority::new(ProjectionBounds {
                max_pending_input_items: 32,
                max_pending_input_total_bytes: 4096,
                max_poll_items: 16,
                max_poll_response_bytes: 128,
                ..ProjectionBounds::default()
            })
            .unwrap();
            let owner_token = attach(&mut authority, "projection-a");
            for index in 0..item_count {
                authority
                    .enqueue_input(
                        "projection-a",
                        &format!("input-{index}"),
                        format!("msg-{index:02}"),
                        20 + index as u64,
                        10_000,
                        None,
                    )
                    .unwrap();
            }

            let poll = authority.handle_get_pending_input(
                GetPendingInputRequest {
                    envelope: envelope(
                        ProjectionOperation::GetPendingInput,
                        "projection-a",
                        "req-poll",
                    ),
                    owner_token,
                    max_items: Some(requested_items),
                    max_bytes: Some(requested_bytes),
                },
                "caller-a",
                100,
            );

            prop_assert!(poll.accepted);
            prop_assert!(poll.pending_input.len() <= requested_items.min(16));
            let returned_bytes: usize = poll
                .pending_input
                .iter()
                .map(|item| item.submission_text.len())
                .sum();
            prop_assert!(returned_bytes <= requested_bytes.min(128));
            for (index, item) in poll.pending_input.iter().enumerate() {
                prop_assert_eq!(&item.input_id, &format!("input-{index}"));
                prop_assert_eq!(item.delivery_state, InputDeliveryState::Delivered);
                prop_assert_eq!(item.delivered_at_wall_us, Some(100));
            }
            prop_assert_eq!(
                poll.pending_remaining_count + poll.pending_input.len(),
                item_count
            );
        }

        #[test]
        fn lifecycle_state_machine_never_reuses_stale_connection_or_lease(
            actions in prop::collection::vec(0u8..6, 1..32),
        ) {
            let mut authority = ProjectionAuthority::default();
            let owner_token = attach(&mut authority, "projection-a");
            let mut projection_exists = true;
            let mut has_connection = false;
            let mut lease_expires_at = None;
            let mut now = 20u64;

            for action in actions {
                now += 10;
                match action {
                    0 => {
                        let mut metadata = connection_metadata(&["create_tiles"]);
                        metadata.connection_id = format!("connection-{now}");
                        metadata.authenticated_session_id = format!("runtime-session-{now}");
                        metadata.connected_at_wall_us = now;
                        metadata.last_reconnect_wall_us = now;
                        prop_assert_eq!(
                            authority.record_hud_connection("projection-a", metadata),
                            Ok(())
                        );
                        has_connection = true;
                        lease_expires_at = None;
                    }
                    1 => {
                        let result = authority.record_heartbeat("projection-a", now);
                        if has_connection {
                            prop_assert_eq!(result, Ok(()));
                        } else {
                            prop_assert_eq!(result, Err(ProjectionErrorCode::ProjectionHudUnavailable));
                        }
                    }
                    2 => {
                        let result = authority.record_advisory_lease(
                            "projection-a",
                            advisory_lease(&["create_tiles"], now + 100),
                            now,
                        );
                        if has_connection {
                            prop_assert_eq!(result, Ok(()));
                            lease_expires_at = Some(now + 100);
                        } else {
                            prop_assert_eq!(result, Err(ProjectionErrorCode::ProjectionHudUnavailable));
                        }
                    }
                    3 => {
                        prop_assert_eq!(
                            authority.mark_hud_disconnected("projection-a", now),
                            Ok(())
                        );
                        has_connection = false;
                        lease_expires_at = None;
                    }
                    4 => {
                        let result = authority.authorize_portal_republish(
                            "projection-a",
                            "lease-1",
                            &[String::from("create_tiles")],
                            now,
                        );
                        if has_connection && lease_expires_at.is_some_and(|expires_at| now < expires_at) {
                            prop_assert_eq!(result, Ok(()));
                        } else {
                            prop_assert!(result.is_err());
                            if has_connection && lease_expires_at.is_some_and(|expires_at| now >= expires_at) {
                                prop_assert_eq!(result, Err(ProjectionErrorCode::ProjectionTokenExpired));
                                lease_expires_at = None;
                            }
                        }
                    }
                    _ => {
                        let response = authority.handle_detach(
                            DetachRequest {
                                envelope: envelope(
                                    ProjectionOperation::Detach,
                                    "projection-a",
                                    "req-detach",
                                ),
                                owner_token: owner_token.clone(),
                                reason: "property lifecycle detach".to_string(),
                            },
                            "caller-a",
                            now,
                        );
                        prop_assert!(response.accepted);
                        projection_exists = false;
                    }
                }

                if !projection_exists {
                    prop_assert!(authority.state_summary("projection-a").is_none());
                    break;
                }

                let summary = authority.state_summary("projection-a").unwrap();
                prop_assert_eq!(summary.has_hud_connection, has_connection);
                if !has_connection {
                    prop_assert!(!summary.has_advisory_lease);
                }
            }
        }
    }

    // ── AdapterGeometryBatch::coalesce (§6b.4) ───────────────────────────────

    fn make_snapshot(sequence: u64, gesture_active: bool) -> AdapterGeometrySnapshot {
        AdapterGeometrySnapshot {
            rect: AdapterPortalRect {
                x_px: 10,
                y_px: 20,
                width_px: 400,
                height_px: 300,
            },
            gesture_active,
            sequence,
        }
    }

    #[test]
    fn coalesce_first_snapshot_is_stored() {
        let mut batch = AdapterGeometryBatch::default();
        assert!(batch.is_empty(), "batch must start empty");

        batch.coalesce(make_snapshot(1, true));

        assert!(!batch.is_empty(), "batch must not be empty after coalesce");
        let latest = batch.latest.expect("latest must be Some after coalesce");
        assert_eq!(latest.sequence, 1, "latest sequence must be 1");
        assert!(latest.gesture_active, "gesture_active must match snapshot");
    }

    #[test]
    fn coalesce_newer_snapshot_replaces_older() {
        let mut batch = AdapterGeometryBatch::default();
        batch.coalesce(make_snapshot(1, true));
        batch.coalesce(make_snapshot(2, false)); // newer

        let latest = batch.latest.expect("latest must be Some");
        assert_eq!(
            latest.sequence, 2,
            "newer snapshot (seq=2) must replace older (seq=1)"
        );
        assert!(
            !latest.gesture_active,
            "gesture_active from newer snapshot must win"
        );
    }

    #[test]
    fn coalesce_older_snapshot_does_not_replace_newer() {
        let mut batch = AdapterGeometryBatch::default();
        batch.coalesce(make_snapshot(5, false));
        batch.coalesce(make_snapshot(3, true)); // older — must be dropped

        let latest = batch.latest.expect("latest must be Some");
        assert_eq!(
            latest.sequence, 5,
            "older snapshot (seq=3) must NOT displace newer (seq=5)"
        );
        assert!(
            !latest.gesture_active,
            "gesture_active from older snapshot must be ignored"
        );
    }

    #[test]
    fn coalesce_equal_sequence_does_not_replace() {
        let mut batch = AdapterGeometryBatch::default();
        batch.coalesce(make_snapshot(7, false));

        // Same sequence with different geometry — first write wins (not strictly
        // required by spec, but the coalescer MUST NOT drop the existing entry
        // for an equal sequence in a way that loses a confirmed snapshot).
        let mut snap_same_seq = make_snapshot(7, true); // gesture_active differs
        snap_same_seq.rect = AdapterPortalRect {
            x_px: 99,
            y_px: 99,
            width_px: 100,
            height_px: 100,
        };
        batch.coalesce(snap_same_seq);

        let latest = batch.latest.expect("latest must be Some");
        assert_eq!(
            latest.sequence, 7,
            "sequence must remain 7 after equal-sequence coalesce attempt"
        );
        assert!(
            !latest.gesture_active,
            "first write (gesture_active=false) must be retained for equal sequence"
        );
    }

    proptest! {
        /// For any sequence of (sequence, gesture_active) pairs,
        /// `latest.sequence` must equal the maximum sequence seen.
        #[test]
        fn coalesce_latest_wins_monotone(pairs in proptest::collection::vec((0u64..100u64, proptest::bool::ANY), 1..20usize)) {
            let mut batch = AdapterGeometryBatch::default();
            let mut max_seq = 0u64;
            for (seq, gesture_active) in &pairs {
                if *seq > max_seq { max_seq = *seq; }
                batch.coalesce(make_snapshot(*seq, *gesture_active));
            }
            let latest = batch.latest.expect("batch must be non-empty after coalescing snapshots");
            prop_assert_eq!(
                latest.sequence, max_seq,
                "latest sequence must equal maximum sequence seen"
            );
        }
    }

    // ── ProjectionAuthority geometry batch flow (§6b.4) ─────────────────────
    //
    // Covers push_geometry_snapshot → projected_portal_state → consume_geometry_batch
    // so that callers cannot accidentally re-deliver geometry indefinitely or
    // fail to surface it in ProjectedPortalState.

    #[test]
    fn geometry_batch_not_surfaced_before_first_push() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "p-geo");
        // No snapshot has been pushed — geometry_batch must be None.
        let state = authority
            .projected_portal_state("p-geo", &ProjectedPortalPolicy::permit_all())
            .expect("session must exist");
        assert!(
            state.geometry_batch.is_none(),
            "geometry_batch must be None before any push"
        );
    }

    #[test]
    fn geometry_batch_surfaced_after_push_and_cleared_after_consume() {
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "p-geo");

        let snap = make_snapshot(1, false);
        let accepted = authority.push_geometry_snapshot("p-geo", snap);
        assert!(accepted, "push must return true for a new snapshot");

        // After push: projected_portal_state must include the batch.
        let state = authority
            .projected_portal_state("p-geo", &ProjectedPortalPolicy::permit_all())
            .expect("session must exist");
        let batch = state
            .geometry_batch
            .expect("geometry_batch must be Some after push");
        let latest = batch.latest.expect("batch.latest must be Some");
        assert_eq!(
            latest.sequence, 1,
            "surfaced sequence must match pushed snapshot"
        );

        // After consume: projected_portal_state must return None for geometry_batch.
        authority.consume_geometry_batch("p-geo");
        let state2 = authority
            .projected_portal_state("p-geo", &ProjectedPortalPolicy::permit_all())
            .expect("session must still exist");
        assert!(
            state2.geometry_batch.is_none(),
            "geometry_batch must be None after consume"
        );
    }

    #[test]
    fn geometry_batch_not_re_delivered_without_new_push() {
        // Verifies that a caller that calls projected_portal_state twice after one
        // consume does NOT receive stale geometry on the second call.
        let mut authority = ProjectionAuthority::default();
        attach(&mut authority, "p-geo");

        authority.push_geometry_snapshot("p-geo", make_snapshot(3, true));
        // First read + consume.
        let _ = authority.projected_portal_state("p-geo", &ProjectedPortalPolicy::permit_all());
        authority.consume_geometry_batch("p-geo");

        // Second read without a new push — must be empty.
        let state = authority
            .projected_portal_state("p-geo", &ProjectedPortalPolicy::permit_all())
            .expect("session must exist");
        assert!(
            state.geometry_batch.is_none(),
            "geometry_batch must remain None after consume with no new push"
        );
    }

    #[test]
    fn push_geometry_snapshot_rejects_unknown_session() {
        let mut authority = ProjectionAuthority::default();
        let accepted = authority.push_geometry_snapshot("does-not-exist", make_snapshot(1, false));
        assert!(
            !accepted,
            "push must return false for an unknown projection_id"
        );
    }

    // ── Regression tests for hud-endkj defect fixes ──────────────────────────
    //
    // Defect 1: coalesce-key in-place update path passed a stale sequence to
    // record_append, causing the coalescer to drop it after the first drain.
    // Final-state for a coalesce-key would never be presented until an unrelated
    // fresh append arrived.
    //
    // Defect 2: next_due_projection_id returns a rate-limited portal whose
    // take_due_portal_update returns Ok(None) without consuming the coalescer
    // entry, allowing a naive driver to busy-spin.  portal_next_due_at_us
    // provides the wait hint that breaks the spin.

    /// Regression: coalesce-key final-state must appear via next_due_projection_id
    /// after the first drain, even when no new append follows.
    ///
    /// Scenario:
    /// 1. First append (portal_update_ready = true) → coalescer entry seq=0 accepted.
    /// 2. take_due_portal_update → drains coalescer (last_drained_sequence=0).
    /// 3. Second append (same coalesce_key, rate-limited) → previously bugged
    ///    code passed seq=0 → stale guard dropped it → portal vanished from
    ///    next_due_projection_id forever.
    ///
    /// After fix: append_transcript_unit bumps next_transcript_sequence on the
    /// in-place path, returning seq=1, which clears the stale guard.
    #[test]
    fn coalesce_key_final_state_reappears_after_drain_without_fresh_append() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "proj-coalesce");
        let portal_id = "proj-coalesce";

        // First append — rate window is open.
        let mut first = output_request(portal_id, &owner_token, "req-1");
        first.output_text = "initial".to_string();
        first.logical_unit_id = Some("unit-1".to_string());
        first.coalesce_key = Some("status".to_string());
        let r1 = authority.handle_publish_output(first, "caller", 10);
        assert!(r1.accepted, "first append must be accepted");
        assert!(
            r1.portal_update_ready,
            "first append must be immediately ready"
        );

        // Oracle confirms a pending update.
        assert_eq!(
            authority.next_due_projection_id().as_deref(),
            Some(portal_id),
            "portal must be ready after first append"
        );

        // Drain the coalescer — last_drained_sequence is now recorded.
        let update = authority
            .take_due_portal_update(portal_id, 10)
            .expect("no error")
            .expect("first update must be present");
        assert_eq!(update.unread_output_count, 1);

        // Oracle must return None after draining (nothing pending).
        assert!(
            authority.next_due_projection_id().is_none(),
            "oracle must be idle after drain"
        );

        // Second append — rate window is closed (same timestamp 10).
        // Uses the same coalesce_key → in-place update path.
        let mut second = output_request(portal_id, &owner_token, "req-2");
        second.output_text = "final-value".to_string();
        second.logical_unit_id = Some("unit-2".to_string());
        second.coalesce_key = Some("status".to_string());
        let r2 = authority.handle_publish_output(second, "caller", 10);
        assert!(r2.accepted, "second append must be accepted");
        assert!(
            !r2.portal_update_ready,
            "second append must be rate-limited (same window)"
        );

        // The coalescer must have a pending entry for this portal.
        // Previously this was dropped as stale — verify the fix.
        assert_eq!(
            authority.next_due_projection_id().as_deref(),
            Some(portal_id),
            "portal must re-appear in oracle after coalesce-key in-place update (hud-endkj defect 1)"
        );

        // And drain at the next window to confirm final value is delivered.
        let update2 = authority
            .take_due_portal_update(portal_id, 10 + PORTAL_UPDATE_RATE_WINDOW_WALL_US)
            .expect("no error")
            .expect("second update must be due in next window");
        let last_text = update2
            .visible_transcript
            .iter()
            .find(|u| u.coalesce_key.as_deref() == Some("status"))
            .map(|u| u.output_text.as_str())
            .unwrap_or("");
        assert_eq!(
            last_text, "final-value",
            "coalesced final value must be delivered (hud-endkj defect 1)"
        );
    }

    /// Regression: multiple coalesce-key in-place updates within the same rate
    /// window must ALL produce strictly-increasing coalescer sequences, so each
    /// successive in-place update remains visible via next_due_projection_id.
    #[test]
    fn coalesce_key_successive_in_place_updates_stay_in_oracle() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "proj-multi");
        let portal_id = "proj-multi";

        // Seed with a first ready append.
        let mut seed = output_request(portal_id, &owner_token, "req-seed");
        seed.output_text = "seed".to_string();
        seed.logical_unit_id = Some("unit-seed".to_string());
        seed.coalesce_key = Some("progress".to_string());
        authority.handle_publish_output(seed, "caller", 5);
        authority
            .take_due_portal_update(portal_id, 5)
            .expect("no error")
            .expect("seed must be drained");

        // Three successive in-place updates within the same rate window.
        for i in 1u64..=3u64 {
            let mut req = output_request(portal_id, &owner_token, &format!("req-{i}"));
            req.output_text = format!("progress-{i}");
            req.logical_unit_id = None;
            req.coalesce_key = Some("progress".to_string());
            let r = authority.handle_publish_output(req, "caller", 5);
            assert!(r.accepted);
            // After each in-place update the oracle must show this portal.
            assert_eq!(
                authority.next_due_projection_id().as_deref(),
                Some(portal_id),
                "oracle must show portal after in-place update {i} (hud-endkj defect 1)"
            );
            // Peek back into the oracle without consuming (internal rotation).
            // Re-insert by querying once — this is fine; the VecDeque rotates.
        }

        // Final drain should deliver the last in-place value.
        let final_update = authority
            .take_due_portal_update(portal_id, 5 + PORTAL_UPDATE_RATE_WINDOW_WALL_US)
            .expect("no error")
            .expect("final update must be present");
        let last_text = final_update
            .visible_transcript
            .iter()
            .find(|u| u.coalesce_key.as_deref() == Some("progress"))
            .map(|u| u.output_text.as_str())
            .unwrap_or("");
        assert_eq!(
            last_text, "progress-3",
            "latest coalesced value must be delivered"
        );
    }

    /// Regression (defect 2): portal_next_due_at_us returns Some when the portal
    /// is rate-limited, giving a wait hint to avoid busy-spinning.
    #[test]
    fn portal_next_due_at_us_returns_wait_hint_when_rate_limited() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "proj-rate");
        let portal_id = "proj-rate";

        // First append (rate window open at t=100).
        let mut first = output_request(portal_id, &owner_token, "req-1");
        first.output_text = "first".to_string();
        first.logical_unit_id = Some("unit-1".to_string());
        authority.handle_publish_output(first, "caller", 100);

        // Drain the first update.
        authority
            .take_due_portal_update(portal_id, 100)
            .expect("no error")
            .expect("first update must be present");

        // Second append at the same timestamp → rate-limited, coalescer entry pending.
        let mut second = output_request(portal_id, &owner_token, "req-2");
        second.output_text = "second".to_string();
        second.logical_unit_id = Some("unit-2".to_string());
        authority.handle_publish_output(second, "caller", 100);

        // next_due_projection_id returns the portal (coalescer has a pending entry).
        assert_eq!(
            authority.next_due_projection_id().as_deref(),
            Some(portal_id),
            "oracle must show portal when coalescer entry is pending"
        );

        // take_due_portal_update returns Ok(None) (rate window not elapsed).
        let result = authority.take_due_portal_update(portal_id, 100);
        assert!(
            matches!(result, Ok(None)),
            "take_due must return Ok(None) when rate window has not elapsed"
        );

        // portal_next_due_at_us must return Some to prevent busy-spin.
        let next_due = authority.portal_next_due_at_us(portal_id, 100);
        assert!(
            next_due.is_some(),
            "portal_next_due_at_us must return Some when portal is rate-limited (hud-endkj defect 2)"
        );
        let next_due_us = next_due.unwrap();
        assert!(
            next_due_us > 100,
            "next_due must be in the future; got {next_due_us}"
        );
        assert_eq!(
            next_due_us,
            100 + PORTAL_UPDATE_RATE_WINDOW_WALL_US,
            "next_due must equal rate window start + window duration"
        );

        // After the rate window elapses, portal_next_due_at_us returns None.
        let after_window = authority.portal_next_due_at_us(portal_id, next_due_us);
        assert!(
            after_window.is_none(),
            "portal_next_due_at_us must return None after rate window elapses"
        );

        // And take_due_portal_update now succeeds.
        let update = authority
            .take_due_portal_update(portal_id, next_due_us)
            .expect("no error")
            .expect("update must be present after rate window elapses");
        assert_eq!(update.unread_output_count, 1);
    }

    /// portal_next_due_at_us returns None for an unknown projection.
    #[test]
    fn portal_next_due_at_us_returns_none_for_unknown_projection() {
        let authority = ProjectionAuthority::default();
        assert!(
            authority
                .portal_next_due_at_us("does-not-exist", 1_000_000)
                .is_none(),
            "must return None for unknown projection"
        );
    }

    /// portal_next_due_at_us returns None immediately after a successful drain
    /// (no pending coalescer entry).
    #[test]
    fn portal_next_due_at_us_returns_none_when_no_pending_entry() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let owner_token = attach(&mut authority, "proj-idle");
        let portal_id = "proj-idle";

        let mut req = output_request(portal_id, &owner_token, "req-1");
        req.logical_unit_id = Some("unit-1".to_string());
        authority.handle_publish_output(req, "caller", 10);
        authority
            .take_due_portal_update(portal_id, 10)
            .expect("no error")
            .expect("must drain");

        // No pending coalescer entry → returns None.
        assert!(
            authority.portal_next_due_at_us(portal_id, 10).is_none(),
            "portal_next_due_at_us must return None when no coalescer entry is pending"
        );
    }
}
