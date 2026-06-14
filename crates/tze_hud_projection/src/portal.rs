//! Process supervision and external authority for provider-neutral sessions.
//!
//! Moved from `lib.rs` in the P-3 mechanical split (hud-d570a). No logic
//! changes — byte-identical relocation with the minimal visibility adjustments
//! required by Rust's module privacy rules.

use crate::authority::{ProjectionAuthority, route_plan_for_request};
use crate::contract::*;
use crate::managed_session::*;
use crate::{MAX_HINT_BYTES, MAX_PROJECTION_ID_BYTES};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::{env, fmt};

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
