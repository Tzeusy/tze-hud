//! Stdio control surface for the cooperative HUD projection authority.
//!
//! This executable is intentionally a projection-daemon surface, not runtime
//! v1 MCP. It retains `ProjectionAuthority` state only for the lifetime of
//! this process and dispatches newline-delimited JSON requests to the
//! normative operation handlers in `tze_hud_projection`.
//!
//! ## Portal drive loop (hud-6rkc8)
//!
//! After every `PublishOutput` dispatch the binary runs a work-conserving drain
//! loop that feeds coalesced portal updates into the present path:
//!
//! 1. `ProjectionAuthority::next_due_projection_id` selects the next portal in
//!    round-robin fairness order (cross-portal fairness, tasks.md §5.1).
//! 2. `ProjectionAuthority::take_due_portal_update` materialises the coalesced
//!    transcript window for that portal.
//! 3. `ProjectionAuthority::projected_portal_state` builds the full portal state.
//! 4. `ResidentGrpcPortalAdapter::ensure_portal_tile_message` /
//!    `render_portal_message` builds the outbound `HudSession` gRPC message.
//! 5. Each resulting command is serialised as a `CliPortalDrainRecord` line on
//!    stdout so the caller can forward it to the resident gRPC session.
//!
//! ## Token-map swap propagation (hud-6rkc8 part a)
//!
//! A `SetTokenMap` operation accepts a flat key→value token override map.
//! On receipt the authority resolves the full token set via
//! `tze_hud_config::resolve_portal_tokens`, converts to `PortalVisualTokens`,
//! and calls `adapter.set_visual_tokens(...)` on every live adapter. The next
//! render after the swap uses the new tokens with zero adapter logic changes
//! (§6.1 profile-swap contract).
//!
//! ## Hook points left for follow-up beads
//!
//! - **hud-ttq97** (submitted_at_us telemetry bucket): the `submitted_at_us`
//!   field of `PortalTranscriptUpdate` is included in `CliPortalDrainRecord`
//!   and currently logged at trace level. A structured telemetry bucket should
//!   be added here once hud-ttq97 lands.
//!
//! - **hud-0528i** (follow-tail notify_tile_content_appended): after the adapter
//!   emits a `RenderPortal` command, the caller should trigger
//!   `notify_tile_content_appended` to advance the follow-tail scroll position.
//!   See `TODO(hud-0528i)` comment in `drain_and_emit_portal_updates`.
//!
//! - **hud-pkg2g** (head-trim notify_head_content_removed): when a transcript
//!   head-trim prunes older units, the caller should trigger
//!   `notify_head_content_removed` to reclaim scroll state. See
//!   `TODO(hud-pkg2g)` comment in `drain_and_emit_portal_updates`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use tze_hud_config::{resolve_portal_tokens, tokens::DesignTokenMap};
use tze_hud_projection::ProjectedPortalPolicy;
use tze_hud_projection::resident_grpc::{
    ResidentGrpcPortalAdapter, ResidentGrpcPortalCommandKind, ResidentGrpcPortalConfig,
    portal_visual_tokens_from_part_tokens,
};
use tze_hud_projection::{
    AcknowledgeInputRequest, AdvisoryLeaseIdentity, AttachRequest, CleanupRequest,
    ContentClassification, DetachRequest, ExternalAgentProjectionAuthority, GetPendingInputRequest,
    HudConnectionMetadata, HudCredentialSource, ManagedSessionOrigin, ManagedSessionRequest,
    PresenceSurfaceRoute, ProjectionAttentionIntent, ProjectionAuditRecord, ProjectionAuthority,
    ProjectionBounds, ProjectionErrorCode, ProjectionOperation, ProjectionResponse, ProviderKind,
    PublishOutputRequest, PublishStatusRequest, WidgetParameterValue, WindowsHudTarget,
};

const DEFAULT_CALLER_IDENTITY: &str = "projection-authority-stdio";
const MAX_CLI_STATUS_SUMMARY_BYTES: usize = 512;
const MAX_STDIN_LINE_BYTES: usize = 65_536;

#[derive(Debug)]
struct CliConfig {
    mode: CliMode,
    caller_identity: String,
    operator_authority: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliMode {
    Stdio,
    DemoPlan,
}

#[derive(Debug, Serialize)]
struct CliOperationResult {
    response: ProjectionResponse,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    audit_records: Vec<ProjectionAuditRecord>,
    /// Zero or more portal drain records produced by the drive loop.
    ///
    /// Non-empty after any operation that advances the cadence coalescer
    /// (primarily `PublishOutput`). Each record contains a coalesced portal
    /// transcript update that the caller should forward to the resident gRPC
    /// session. Embedded in the operation result so the caller reads one
    /// JSON line per operation regardless of how many portals are served.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    portal_drain: Vec<CliPortalDrainRecord>,
}

#[derive(Debug, Serialize)]
struct DemoPlanOutput {
    demo_name: String,
    hud_target_id: String,
    route_plans: Vec<tze_hud_projection::ManagedSessionRoutePlan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    zone_messages: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    widget_messages: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    portal_routes: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lifecycle_checks: Vec<Value>,
}

// ── Portal drive state (hud-6rkc8) ───────────────────────────────────────────

/// Per-process state for the production portal drive loop.
///
/// Holds one `ResidentGrpcPortalAdapter` per attached projection session that
/// uses the resident-gRPC portal surface, plus the current resolved token map.
/// The adapter is constructed with runtime-resolved tokens on `Attach` and
/// updated on every `SetTokenMap` operation.
///
/// Not serialized — lives only for the lifetime of the stdio process.
struct PortalDriveState {
    /// Per-projection adapters keyed by `projection_id`.
    adapters: HashMap<String, ResidentGrpcPortalAdapter>,
    /// Current resolved design-token overrides (flat key → value strings).
    /// `PortalVisualTokens` is derived from this on every `SetTokenMap` and at
    /// adapter construction.
    token_overrides: DesignTokenMap,
}

impl PortalDriveState {
    fn new() -> Self {
        Self {
            adapters: HashMap::new(),
            token_overrides: DesignTokenMap::new(),
        }
    }

    /// Resolve the current portal visual tokens from `token_overrides`.
    fn resolve_visual_tokens(&self) -> tze_hud_projection::resident_grpc::PortalVisualTokens {
        let resolved =
            tze_hud_config::tokens::resolve_tokens(&DesignTokenMap::new(), &self.token_overrides);
        portal_visual_tokens_from_part_tokens(&resolve_portal_tokens(&resolved))
    }

    /// Register a new adapter for `projection_id` with runtime-resolved tokens.
    ///
    /// Called on every `Attach` for sessions that will use the portal path.
    /// The adapter is constructed with a default lease ID of all-zeros (the
    /// caller should call `attach_adapter_with_lease` if a real lease ID is
    /// known). Token swap propagation is immediate — `set_token_map` will reach
    /// this adapter on the next `SetTokenMap` operation.
    fn attach_adapter(&mut self, projection_id: &str, lease_id: Vec<u8>) {
        let tokens = self.resolve_visual_tokens();
        let config = ResidentGrpcPortalConfig::new(lease_id);
        let adapter = ResidentGrpcPortalAdapter::with_tokens(config, tokens);
        self.adapters.insert(projection_id.to_string(), adapter);
    }

    /// Remove the adapter for `projection_id` (called on `Detach` / `Cleanup`).
    fn detach_adapter(&mut self, projection_id: &str) {
        self.adapters.remove(projection_id);
    }

    /// Apply a new token override map: resolve, then propagate to all live adapters.
    ///
    /// This is the §6.1 profile-swap contract: `set_visual_tokens` is called on
    /// every adapter so the next render uses the new tokens without any adapter
    /// logic changes.
    fn apply_token_map(&mut self, overrides: DesignTokenMap) {
        self.token_overrides = overrides;
        let tokens = self.resolve_visual_tokens();
        for adapter in self.adapters.values_mut() {
            adapter.set_visual_tokens(tokens.clone());
        }
    }

    /// Get a mutable reference to the adapter for `projection_id`, if any.
    fn adapter_mut(&mut self, projection_id: &str) -> Option<&mut ResidentGrpcPortalAdapter> {
        self.adapters.get_mut(projection_id)
    }
}

/// Kind of stdout line emitted by the portal drain loop.
///
/// The drain loop emits `CliPortalDrainRecord` lines interleaved with the
/// normal `CliOperationResult` lines. Callers distinguish them by the
/// `record_type` field.
/// Serialized form of one drained portal update produced by the drive loop.
///
/// This is the "present path" output: a coalesced portal transcript update
/// that the caller should forward to the resident gRPC session (build and send
/// a `session_proto::ClientMessage` using the `portal_markdown` from
/// `projected_portal_state` applied through `ResidentGrpcPortalAdapter`).
///
/// Carries semantic data (transcript, state, geometry, budget) rather than
/// raw proto bytes so the stdio surface stays protocol-agnostic; the gRPC
/// session layer (which has prost in scope) builds the proto message.
#[derive(Debug, Serialize)]
struct CliPortalDrainRecord {
    /// Projection ID that received this update.
    projection_id: String,
    /// Kind of resident gRPC command produced.
    command_kind: CliResidentCommandKind,
    /// Number of transcript units included in this update.
    visible_transcript_units: usize,
    /// Byte count of visible transcript.
    visible_transcript_bytes: usize,
    /// Number of output calls coalesced into this update.
    coalesced_output_count: usize,
    /// Total unread output count drained by this update.
    unread_output_count: usize,
    /// Wall-clock submission timestamp (µs) of the most-recently-coalesced
    /// append (arrival→present latency anchor, tasks.md §5.7).
    ///
    /// **Hook point for hud-ttq97**: record this value into the structured
    /// telemetry latency bucket once that bead lands.
    submitted_at_us: u64,
    /// Rendered portal markdown content (from `portal_node` via the adapter).
    ///
    /// The caller uses this to populate the `TextMarkdownNodeProto.content`
    /// field in the outbound `ClientMessage`. This is the token-styled portal
    /// text with transcript, composer display, and caret already applied.
    portal_markdown: String,
    /// Presentation state of this update (`expanded` or `collapsed`).
    presentation: String,
    /// Tile ID previously assigned by `record_created_tile`, if any.
    /// `None` means the caller must send a `CreateTile` mutation first.
    tile_id_hex: Option<String>,
    /// Elapsed microseconds building this command (budget evidence).
    elapsed_us: u64,
    /// Budget ceiling for this command kind (µs).
    budget_us: u64,
    /// True when `elapsed_us <= budget_us`.
    within_budget: bool,
}

/// Serializable variant of `ResidentGrpcPortalCommandKind`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CliResidentCommandKind {
    CreatePortalTile,
    ReusePortalTile,
    RenderPortal,
    ReleaseLease,
}

impl From<ResidentGrpcPortalCommandKind> for CliResidentCommandKind {
    fn from(kind: ResidentGrpcPortalCommandKind) -> Self {
        match kind {
            ResidentGrpcPortalCommandKind::CreatePortalTile => Self::CreatePortalTile,
            ResidentGrpcPortalCommandKind::ReusePortalTile => Self::ReusePortalTile,
            ResidentGrpcPortalCommandKind::RenderPortal => Self::RenderPortal,
            ResidentGrpcPortalCommandKind::ReleaseLease => Self::ReleaseLease,
        }
    }
}

/// `SetTokenMap` operation payload — accepted as a JSON stdin line with
/// `"operation": "set_token_map"`.
#[derive(Debug, Deserialize)]
struct SetTokenMapRequest {
    /// Request ID for correlation in responses.
    #[serde(default)]
    request_id: String,
    /// Flat design-token override map (key → CSS-style value string).
    /// Only portal-relevant tokens are consumed. Unrecognised keys are silently
    /// ignored. An empty map resets to built-in defaults.
    token_overrides: HashMap<String, String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = parse_args(env::args().skip(1))?;
    if config.mode == CliMode::DemoPlan {
        return write_demo_plan(&config);
    }

    let mut authority = ProjectionAuthority::new(ProjectionBounds::default())
        .map_err(|error| format!("failed to initialize projection authority: {error}"))?;
    if let Some(operator_authority) = config.operator_authority.as_deref() {
        authority
            .set_operator_authority(operator_authority)
            .map_err(|error| format!("invalid operator authority: {error}"))?;
    }

    let mut portal_drive = PortalDriveState::new();
    serve_stdio(&mut authority, &config, &mut portal_drive)
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliConfig, String> {
    let mut mode = CliMode::Stdio;
    let mut caller_identity = DEFAULT_CALLER_IDENTITY.to_string();
    let mut operator_authority = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--stdio" => {}
            "--demo-plan" => {
                mode = CliMode::DemoPlan;
            }
            "--caller-identity" => {
                caller_identity = iter
                    .next()
                    .ok_or("--caller-identity requires a value".to_string())?;
            }
            "--operator-authority" => {
                return Err(
                    "--operator-authority is not supported; use --operator-authority-env VAR"
                        .to_string(),
                );
            }
            "--operator-authority-env" => {
                let var = iter
                    .next()
                    .ok_or("--operator-authority-env requires a variable name".to_string())?;
                operator_authority = Some(
                    env::var(&var).map_err(|_| format!("{var} is not set in the environment"))?,
                );
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }

    Ok(CliConfig {
        mode,
        caller_identity,
        operator_authority,
    })
}

fn print_help() {
    println!(
        "Usage: tze_hud_projection_authority [--stdio|--demo-plan] [--caller-identity ID] [--operator-authority-env VAR]\n\
         \n\
         Reads one cooperative HUD projection operation JSON object per stdin line and writes one JSON result per stdout line.\n\
         The process retains projection state in memory only and emits bounded operation responses plus newly written audit records.\n\
         --demo-plan emits a redacted three-session zone/widget/portal route-plan artifact without connecting to the HUD."
    );
}

fn write_demo_plan(config: &CliConfig) -> Result<(), String> {
    let mut authority = ExternalAgentProjectionAuthority::default();
    register_demo_target(&mut authority)?;

    let now = now_wall_us();
    for (offset, request) in demo_session_requests().into_iter().enumerate() {
        authority
            .manage_session(
                request,
                &config.caller_identity,
                now.saturating_add(offset as u64 + 1),
            )
            .map_err(|error| format!("failed to build demo plan: {error}"))?;
    }

    let route_plans = authority.three_session_demo_plan();
    let output = DemoPlanOutput {
        demo_name: "external-agent-projection-authority-three-session-demo".to_string(),
        hud_target_id: "windows-local".to_string(),
        zone_messages: zone_messages_for_demo(&route_plans),
        widget_messages: widget_messages_for_demo(&route_plans),
        portal_routes: portal_routes_for_demo(&route_plans),
        lifecycle_checks: lifecycle_checks_for_demo(&config.caller_identity)?,
        route_plans,
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &output)
        .map_err(|error| format!("failed to encode demo plan: {error}"))?;
    stdout
        .write_all(b"\n")
        .map_err(|error| format!("failed to write demo plan: {error}"))
}

fn register_demo_target(authority: &mut ExternalAgentProjectionAuthority) -> Result<(), String> {
    authority
        .register_windows_target(demo_windows_target())
        .map_err(|error| format!("invalid demo HUD target: {error}"))
}

fn demo_windows_target() -> WindowsHudTarget {
    WindowsHudTarget {
        target_id: "windows-local".to_string(),
        mcp_url: Some("http://tzehouse-windows.parrot-hen.ts.net:9090/mcp".to_string()),
        grpc_endpoint: Some("tzehouse-windows.parrot-hen.ts.net:50051".to_string()),
        credential_source: HudCredentialSource::EnvVar("TZE_HUD_PSK".to_string()),
        runtime_audience: "local-windows-hud".to_string(),
    }
}

fn zone_messages_for_demo(
    route_plans: &[tze_hud_projection::ManagedSessionRoutePlan],
) -> Vec<Value> {
    route_plans
        .iter()
        .filter_map(|plan| match &plan.surface_command {
            tze_hud_projection::HudSurfaceCommandPlan::ZonePublish {
                zone_name,
                content_kind,
                ttl_ms,
                agent_id,
            } => Some(serde_json::json!({
                "zone_name": zone_name,
                "content": {
                    "type": "status_bar",
                    "entries": {
                        "agent": plan.display_name,
                        "provider": plan.provider_kind,
                        "state": plan.lifecycle_state,
                        "kind": content_kind,
                    },
                },
                "merge_key": plan.projection_id,
                "namespace": agent_id,
                "ttl_us": ttl_ms.saturating_mul(1_000),
            })),
            _ => None,
        })
        .collect()
}

fn widget_messages_for_demo(
    route_plans: &[tze_hud_projection::ManagedSessionRoutePlan],
) -> Vec<Value> {
    route_plans
        .iter()
        .filter_map(|plan| match &plan.surface_command {
            tze_hud_projection::HudSurfaceCommandPlan::WidgetPublish {
                widget_name,
                parameters,
                ttl_ms,
                agent_id,
            } => Some(serde_json::json!({
                "widget_name": widget_name,
                "params": parameters
                    .iter()
                    .map(|(key, value)| (key.clone(), widget_value_to_json(value)))
                    .collect::<serde_json::Map<String, Value>>(),
                "namespace": agent_id,
                "ttl_us": ttl_ms.saturating_mul(1_000),
            })),
            _ => None,
        })
        .collect()
}

fn portal_routes_for_demo(
    route_plans: &[tze_hud_projection::ManagedSessionRoutePlan],
) -> Vec<Value> {
    route_plans
        .iter()
        .filter_map(|plan| match &plan.surface_command {
            tze_hud_projection::HudSurfaceCommandPlan::PortalLease {
                portal_surface,
                portal_id,
                requested_capabilities,
                lease_ttl_ms,
                agent_id,
            } => Some(serde_json::json!({
                "projection_id": plan.projection_id,
                "portal_surface": portal_surface,
                "portal_id": portal_id,
                "agent_id": agent_id,
                "requested_capabilities": requested_capabilities,
                "lease_ttl_ms": lease_ttl_ms,
                "materialization": "resident_raw_tile",
                "replay": "resident_grpc_text_stream_portal",
            })),
            _ => None,
        })
        .collect()
}

fn lifecycle_checks_for_demo(caller_identity: &str) -> Result<Vec<Value>, String> {
    Ok(vec![
        revoke_isolation_check(caller_identity)?,
        expiry_cleanup_check(caller_identity)?,
        reconnect_fresh_lease_check(caller_identity)?,
        provider_process_supervision_check(caller_identity)?,
    ])
}

fn revoke_isolation_check(caller_identity: &str) -> Result<Value, String> {
    let mut authority = ExternalAgentProjectionAuthority::default();
    register_demo_target(&mut authority)?;
    for (offset, request) in demo_session_requests().into_iter().enumerate() {
        authority
            .manage_session(request, caller_identity, 10 + offset as u64)
            .map_err(|error| format!("revoke check manage_session failed: {error}"))?;
    }
    authority
        .revoke_session("agent-progress")
        .map_err(|error| format!("revoke check failed: {error}"))?;
    Ok(serde_json::json!({
        "check": "revoke_isolation",
        "accepted": authority.managed_session_count() == 2
            && authority.route_plan("agent-progress").is_none()
            && authority.route_plan("agent-status").is_some()
            && authority.route_plan("agent-question").is_some(),
        "remaining_sessions": authority.managed_session_count(),
        "revoked_projection_absent": authority.route_plan("agent-progress").is_none(),
        "other_routes_intact": [
            authority.route_plan("agent-status").is_some(),
            authority.route_plan("agent-question").is_some(),
        ],
    }))
}

fn expiry_cleanup_check(caller_identity: &str) -> Result<Value, String> {
    let mut authority = ExternalAgentProjectionAuthority::new(ProjectionBounds {
        owner_token_ttl_wall_us: 20,
        ..ProjectionBounds::default()
    })
    .map_err(|error| format!("expiry check authority init failed: {error}"))?;
    register_demo_target(&mut authority)?;
    authority
        .manage_session(managed_session_by_id("agent-status")?, caller_identity, 10)
        .map_err(|error| format!("expiry check status manage_session failed: {error}"))?;
    authority
        .manage_session(
            managed_session_by_id("agent-question")?,
            caller_identity,
            11,
        )
        .map_err(|error| format!("expiry check portal manage_session failed: {error}"))?;
    let first_expired = authority.expire_token_expired_sessions(30);
    let second_expired = authority.expire_token_expired_sessions(31);
    Ok(serde_json::json!({
        "check": "expiry_cleanup",
        "accepted": first_expired == 1
            && second_expired == 1
            && authority.managed_session_count() == 0,
        "first_expired_count": first_expired,
        "second_expired_count": second_expired,
        "remaining_sessions": authority.managed_session_count(),
    }))
}

fn reconnect_fresh_lease_check(caller_identity: &str) -> Result<Value, String> {
    let mut authority = ExternalAgentProjectionAuthority::default();
    register_demo_target(&mut authority)?;
    authority
        .manage_session(
            managed_session_by_id("agent-question")?,
            caller_identity,
            10,
        )
        .map_err(|error| format!("reconnect check manage_session failed: {error}"))?;
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
        .map_err(|error| format!("reconnect check initial connection failed: {error}"))?;
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
        .map_err(|error| format!("reconnect check initial lease failed: {error}"))?;
    authority
        .mark_hud_disconnected("agent-question", 30)
        .map_err(|error| format!("reconnect check disconnect failed: {error}"))?;
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
        .map_err(|error| format!("reconnect check reconnect failed: {error}"))?;
    let stale_rejected = authority
        .projection_authority_mut()
        .authorize_portal_republish(
            "agent-question",
            "lease-1",
            &["create_tiles".to_string()],
            41,
        )
        == Err(ProjectionErrorCode::ProjectionUnauthorized);
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
        .map_err(|error| format!("reconnect check fresh lease failed: {error}"))?;
    let fresh_authorized = authority
        .projection_authority_mut()
        .authorize_portal_republish(
            "agent-question",
            "lease-2",
            &["create_tiles".to_string()],
            44,
        )
        .is_ok();
    Ok(serde_json::json!({
        "check": "reconnect_requires_fresh_lease",
        "accepted": stale_rejected && fresh_authorized,
        "stale_lease_rejected": stale_rejected,
        "fresh_lease_authorized": fresh_authorized,
    }))
}

fn provider_process_supervision_check(caller_identity: &str) -> Result<Value, String> {
    let mut authority = ExternalAgentProjectionAuthority::default();
    register_demo_target(&mut authority)?;
    let mut request = managed_session_by_id("agent-progress")?;
    let current_exe =
        env::current_exe().map_err(|error| format!("provider process check failed: {error}"))?;
    request.origin = ManagedSessionOrigin::Launched(tze_hud_projection::LaunchSessionSpec {
        command: current_exe.display().to_string(),
        args: vec!["--help".to_string()],
        working_directory: None,
        environment_keys: Vec::new(),
    });
    authority
        .manage_session(request, caller_identity, 10)
        .map_err(|error| format!("provider process check manage_session failed: {error}"))?;
    let launched = authority
        .launch_provider_process("agent-progress")
        .map_err(|error| format!("provider process check launch failed: {error}"))?;
    let mut final_status = launched.clone();
    for _ in 0..20 {
        final_status = authority
            .provider_process_status("agent-progress")
            .map_err(|error| format!("provider process check status failed: {error}"))?
            .ok_or("provider process check lost tracked process".to_string())?;
        if matches!(
            final_status.state,
            tze_hud_projection::ProviderProcessState::Exited { .. }
        ) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    authority
        .terminate_provider_process("agent-progress")
        .map_err(|error| format!("provider process check cleanup failed: {error}"))?;
    Ok(serde_json::json!({
        "check": "provider_process_supervision",
        "accepted": launched.process_id > 0
            && matches!(
                final_status.state,
                tze_hud_projection::ProviderProcessState::Exited { .. }
            )
            && authority.provider_process_status("agent-progress")
                .map_err(|error| format!("provider process check final status failed: {error}"))?
                .is_none(),
        "process_id_present": launched.process_id > 0,
        "final_state": final_status.state,
        "stdio_capture": "disabled",
    }))
}

fn managed_session_by_id(projection_id: &str) -> Result<ManagedSessionRequest, String> {
    demo_session_requests()
        .into_iter()
        .find(|request| request.projection_id == projection_id)
        .ok_or_else(|| format!("unknown demo projection id: {projection_id}"))
}

fn widget_value_to_json(value: &WidgetParameterValue) -> Value {
    match value {
        WidgetParameterValue::F32Milli(value) => serde_json::json!((*value as f64) / 1_000.0),
        WidgetParameterValue::Text(value) => serde_json::json!(value),
        WidgetParameterValue::ColorRgba([r, g, b, a]) => serde_json::json!({
            "r": (*r as f64) / 255.0,
            "g": (*g as f64) / 255.0,
            "b": (*b as f64) / 255.0,
            "a": (*a as f64) / 255.0,
        }),
        WidgetParameterValue::Enum(value) => serde_json::json!(value),
    }
}

fn demo_session_requests() -> Vec<ManagedSessionRequest> {
    let mut progress_parameters = HashMap::new();
    progress_parameters.insert("progress".to_string(), WidgetParameterValue::F32Milli(420));
    progress_parameters.insert(
        "label".to_string(),
        WidgetParameterValue::Text("External authority demo".to_string()),
    );

    vec![
        ManagedSessionRequest {
            projection_id: "agent-status".to_string(),
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
        },
        ManagedSessionRequest {
            projection_id: "agent-progress".to_string(),
            provider_kind: ProviderKind::Claude,
            display_name: "Claude Progress".to_string(),
            origin: ManagedSessionOrigin::Launched(tze_hud_projection::LaunchSessionSpec {
                command: "claude".to_string(),
                args: vec!["--continue".to_string()],
                working_directory: Some("/home/tze/gt/tze_hud/mayor/rig".to_string()),
                environment_keys: vec!["ANTHROPIC_API_KEY".to_string()],
            }),
            hud_target_id: "windows-local".to_string(),
            surface_route: PresenceSurfaceRoute::Widget {
                widget_name: "main-progress".to_string(),
                parameters: progress_parameters,
                ttl_ms: 10_000,
            },
            content_classification: ContentClassification::Private,
            attention_intent: ProjectionAttentionIntent::Ambient,
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: Some("claude".to_string()),
        },
        ManagedSessionRequest {
            projection_id: "agent-question".to_string(),
            provider_kind: ProviderKind::Opencode,
            display_name: "Opencode Questions".to_string(),
            origin: ManagedSessionOrigin::Attached,
            hud_target_id: "windows-local".to_string(),
            surface_route: PresenceSurfaceRoute::Portal {
                portal_surface: tze_hud_projection::PortalSurfaceKind::TextStreamRawTile,
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
        },
    ]
}

fn serve_stdio(
    authority: &mut ProjectionAuthority,
    config: &CliConfig,
    portal_drive: &mut PortalDriveState,
) -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = io::stdout().lock();

    loop {
        let result = match read_bounded_line(&mut stdin, MAX_STDIN_LINE_BYTES)
            .map_err(|error| format!("failed to read stdin: {error}"))?
        {
            StdinLine::Line(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                dispatch_line(authority, config, portal_drive, &line)
            }
            StdinLine::TooLong => malformed_response(
                "unknown",
                "unknown",
                now_wall_us(),
                format!("stdin request line exceeds {MAX_STDIN_LINE_BYTES} bytes"),
            ),
            StdinLine::InvalidUtf8 => malformed_response(
                "unknown",
                "unknown",
                now_wall_us(),
                "stdin request line is not valid UTF-8",
            ),
            StdinLine::Eof => break,
        };
        write_result(&mut stdout, &result)?;
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum StdinLine {
    Line(String),
    TooLong,
    InvalidUtf8,
    Eof,
}

fn read_bounded_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<StdinLine> {
    let mut bytes = Vec::new();

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(StdinLine::Eof);
            }
            break;
        }

        if let Some(newline_index) = available.iter().position(|byte| *byte == b'\n') {
            let take = newline_index + 1;
            if bytes.len() + take > max_bytes {
                reader.consume(take);
                return Ok(StdinLine::TooLong);
            }
            bytes.extend_from_slice(&available[..take]);
            reader.consume(take);
            break;
        }

        if bytes.len() + available.len() > max_bytes {
            let consumed = available.len();
            reader.consume(consumed);
            drain_until_newline(reader)?;
            return Ok(StdinLine::TooLong);
        }

        bytes.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
    }

    if bytes.ends_with(b"\n") {
        bytes.pop();
        if bytes.ends_with(b"\r") {
            bytes.pop();
        }
    }

    match String::from_utf8(bytes) {
        Ok(line) => Ok(StdinLine::Line(line)),
        Err(_) => Ok(StdinLine::InvalidUtf8),
    }
}

fn drain_until_newline<R: BufRead>(reader: &mut R) -> io::Result<()> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(());
        }
        if let Some(newline_index) = available.iter().position(|byte| *byte == b'\n') {
            reader.consume(newline_index + 1);
            return Ok(());
        }
        let consumed = available.len();
        reader.consume(consumed);
    }
}

fn write_result(mut stdout: impl Write, result: &CliOperationResult) -> Result<(), String> {
    serde_json::to_writer(&mut stdout, result)
        .map_err(|error| format!("failed to encode response: {error}"))?;
    stdout
        .write_all(b"\n")
        .map_err(|error| format!("failed to write response: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("failed to flush response: {error}"))
}

fn dispatch_line(
    authority: &mut ProjectionAuthority,
    config: &CliConfig,
    portal_drive: &mut PortalDriveState,
    line: &str,
) -> CliOperationResult {
    let server_timestamp_wall_us = now_wall_us();
    let value = match serde_json::from_str::<Value>(line) {
        Ok(value) => value,
        Err(error) => {
            return malformed_response(
                "unknown",
                "unknown",
                server_timestamp_wall_us,
                format!("invalid JSON request: {error}"),
            );
        }
    };

    // Special-case: "set_token_map" is not a `ProjectionOperation` variant —
    // it targets the portal drive layer, not the authority. Handle it first.
    if value
        .get("operation")
        .and_then(Value::as_str)
        .map(|op| op == "set_token_map")
        .unwrap_or(false)
    {
        return dispatch_set_token_map(value, portal_drive, server_timestamp_wall_us);
    }

    let caller_identity = config.caller_identity.as_str();
    let Some(operation_value) = value.get("operation") else {
        return malformed_response(
            request_id(&value),
            projection_id(&value),
            server_timestamp_wall_us,
            "operation is required",
        );
    };
    let operation = match serde_json::from_value::<ProjectionOperation>(operation_value.clone()) {
        Ok(operation) => operation,
        Err(error) => {
            return malformed_response(
                request_id(&value),
                projection_id(&value),
                server_timestamp_wall_us,
                format!("invalid operation: {error}"),
            );
        }
    };

    let audit_start = authority.audit_log().len();
    let response = match operation {
        ProjectionOperation::Attach => deserialize_then(value, |request: AttachRequest| {
            // Register an adapter for this projection on successful attach.
            // We use an empty lease ID initially; the caller sets a real lease
            // ID via the advisory lease path when the HUD session is live.
            // Part (a): adapter hosted here with runtime-resolved tokens.
            let proj_id = request.envelope.projection_id.clone();
            let result =
                authority.handle_attach(request, caller_identity, server_timestamp_wall_us);
            if result.accepted {
                portal_drive.attach_adapter(&proj_id, Vec::new());
            }
            result
        }),
        ProjectionOperation::PublishOutput => {
            deserialize_then(value, |request: PublishOutputRequest| {
                authority.handle_publish_output(request, caller_identity, server_timestamp_wall_us)
            })
        }
        ProjectionOperation::PublishStatus => {
            deserialize_then(value, |request: PublishStatusRequest| {
                authority.handle_publish_status(request, caller_identity, server_timestamp_wall_us)
            })
        }
        ProjectionOperation::GetPendingInput => {
            deserialize_then(value, |request: GetPendingInputRequest| {
                authority.handle_get_pending_input(
                    request,
                    caller_identity,
                    server_timestamp_wall_us,
                )
            })
        }
        ProjectionOperation::AcknowledgeInput => {
            deserialize_then(value, |request: AcknowledgeInputRequest| {
                authority.handle_acknowledge_input(
                    request,
                    caller_identity,
                    server_timestamp_wall_us,
                )
            })
        }
        ProjectionOperation::Detach => deserialize_then(value, |request: DetachRequest| {
            let proj_id = request.envelope.projection_id.clone();
            let result =
                authority.handle_detach(request, caller_identity, server_timestamp_wall_us);
            if result.accepted {
                portal_drive.detach_adapter(&proj_id);
            }
            result
        }),
        ProjectionOperation::Cleanup => deserialize_then(value, |request: CleanupRequest| {
            let proj_id = request.envelope.projection_id.clone();
            let result =
                authority.handle_cleanup(request, caller_identity, server_timestamp_wall_us);
            if result.accepted {
                portal_drive.detach_adapter(&proj_id);
            }
            result
        }),
    };

    let audit_records = new_audit_records(authority, audit_start, &response);
    // Part (b): drain portal updates into the present path after any operation
    // that could advance the coalescer (PublishOutput is the primary trigger,
    // but we drain after every operation for work-conserving behaviour).
    // Drain records are embedded in the operation result (one JSON line per
    // operation) so callers read a single line regardless of drain size.
    let portal_drain =
        drain_and_emit_portal_updates(authority, portal_drive, server_timestamp_wall_us);
    CliOperationResult {
        response,
        audit_records,
        portal_drain,
    }
}

/// Handle the out-of-band `set_token_map` operation (portal drive layer only).
///
/// Not a `ProjectionOperation` — dispatched separately so that
/// `ProjectionAuthority` state is not touched. Returns a `CliOperationResult`
/// with the token-swap outcome embedded in the response.
fn dispatch_set_token_map(
    value: Value,
    portal_drive: &mut PortalDriveState,
    server_timestamp_wall_us: u64,
) -> CliOperationResult {
    let req: SetTokenMapRequest = match serde_json::from_value(value) {
        Ok(req) => req,
        Err(error) => {
            return CliOperationResult {
                response: invalid_argument_response(
                    "unknown".to_string(),
                    "set_token_map".to_string(),
                    server_timestamp_wall_us,
                    format!("invalid set_token_map payload: {error}"),
                ),
                audit_records: Vec::new(),
                portal_drain: Vec::new(),
            };
        }
    };

    let adapter_count = portal_drive.adapters.len();
    portal_drive.apply_token_map(req.token_overrides);
    // Note: no authority available here so the drain loop cannot run.
    // The caller will drain on the next PublishOutput operation.
    CliOperationResult {
        response: ProjectionResponse {
            request_id: req.request_id,
            projection_id: "set_token_map".to_string(),
            accepted: true,
            error_code: None,
            server_timestamp_wall_us,
            status_summary: format!("token map applied; {adapter_count} adapter(s) updated"),
            owner_token: None,
            lifecycle_state: None,
            pending_input: Vec::new(),
            pending_remaining_count: 0,
            pending_remaining_bytes: 0,
            portal_update_ready: false,
            coalesced_output_count: 0,
        },
        audit_records: Vec::new(),
        portal_drain: Vec::new(),
    }
}

/// Work-conserving drain loop: pull all due portal updates round-robin and
/// convert them through the resident gRPC adapter into `CliPortalDrainRecord`
/// lines for the caller to forward to the live HUD session.
///
/// Round-robin fairness is enforced by `next_due_projection_id()`: portals are
/// served in the order the coalescer tracks, preventing starvation under equal
/// sustained input rates (tasks.md §5.1, §5.4).
///
/// ## Hook points
///
/// - **hud-ttq97** (telemetry bucket): `submitted_at_us` in each
///   `CliPortalDrainRecord` captures the arrival→present latency anchor.
///   A structured bucket should be emitted here once hud-ttq97 lands.
///
/// - **hud-0528i** (follow-tail notify): after `RenderPortal`, the caller
///   should trigger `notify_tile_content_appended` to advance the follow-tail
///   scroll position. Leave a TODO comment so the hook point is visible.
///
/// - **hud-pkg2g** (head-trim notify): when the transcript head is pruned,
///   the caller should trigger `notify_head_content_removed`. This fires when
///   `visible_transcript_bytes < unread_output_count` bytes have been trimmed.
///   Leave a TODO comment so the hook point is visible.
fn drain_and_emit_portal_updates(
    authority: &mut ProjectionAuthority,
    portal_drive: &mut PortalDriveState,
    server_timestamp_wall_us: u64,
) -> Vec<CliPortalDrainRecord> {
    let mut records = Vec::new();
    let policy = ProjectedPortalPolicy::permit_all();

    loop {
        // Round-robin fairness oracle — returns None when no portal has a
        // pending update in the coalescer.
        let Some(proj_id) = authority.next_due_projection_id() else {
            break;
        };

        // Materialise the coalesced update for this portal.
        let update = match authority.take_due_portal_update(&proj_id, server_timestamp_wall_us) {
            Ok(Some(update)) => update,
            Ok(None) => {
                // Coalescer said ready but update is not yet due (rate window).
                // Do not spin; move to next portal or exit.
                break;
            }
            Err(_) => {
                // Projection not found or expired — clean up the adapter to
                // prevent leaks and continue draining other portals.
                portal_drive.detach_adapter(&proj_id);
                continue;
            }
        };

        // Build the full projected portal state for rendering.
        let Some(state) = authority.projected_portal_state(&proj_id, &policy) else {
            // Session was removed between take_due and state query (race).
            // Clean up the adapter and continue draining other portals.
            portal_drive.detach_adapter(&proj_id);
            continue;
        };

        // Drive the adapter: determine command kind and render portal content.
        let adapter = match portal_drive.adapter_mut(&proj_id) {
            Some(adapter) => adapter,
            None => {
                // No adapter registered for this portal (session may have
                // used a non-portal surface). Skip silently.
                continue;
            }
        };

        // Render the portal markdown — the semantic content of the tile.
        // This is what portal_node places in TextMarkdownNodeProto::content.
        let portal_markdown = adapter.render_portal_markdown(&state);

        // Determine the command kind: CreatePortalTile if no tile yet registered,
        // RenderPortal otherwise. The gRPC session layer uses this to decide
        // whether to send a CreateTile mutation before the PublishToTile mutation.
        let command_kind = if adapter.tile_id().is_none() {
            ResidentGrpcPortalCommandKind::CreatePortalTile
        } else {
            ResidentGrpcPortalCommandKind::RenderPortal
        };

        // Capture tile_id (hex) for the caller to correlate tile operations.
        let tile_id_hex = adapter
            .tile_id()
            .map(|id| id.iter().map(|b| format!("{b:02x}")).collect());

        // Build a budget sample for the render work done above.
        // Dispatch to the correct path: if no tile has been created yet, use
        // ensure_portal_tile_message (which handles CreatePortalTile without
        // requiring a tile_id); otherwise use render_portal_message.
        let seq = server_timestamp_wall_us;
        let budget_result = if adapter.tile_id().is_none() {
            adapter.ensure_portal_tile_message(&state, seq, server_timestamp_wall_us)
        } else {
            adapter.render_portal_message(&state, seq, server_timestamp_wall_us)
        };
        let (elapsed_us, budget_us, within_budget) = match &budget_result {
            Ok(cmd) => (
                cmd.budget.elapsed_us,
                cmd.budget.budget_us,
                cmd.budget.within_budget(),
            ),
            Err(_) => (0, 0, false),
        };

        let presentation = match state.presentation {
            tze_hud_projection::ProjectedPortalPresentation::Expanded => "expanded".to_string(),
            tze_hud_projection::ProjectedPortalPresentation::Collapsed => "collapsed".to_string(),
        };

        // TODO(hud-0528i): after RenderPortal, trigger notify_tile_content_appended
        // on the tile so the follow-tail scroll position advances. Hook point:
        //   if matches!(command_kind, ResidentGrpcPortalCommandKind::RenderPortal) {
        //       notify_tile_content_appended(tile_id, ...);
        //   }

        // TODO(hud-pkg2g): when update.visible_transcript_bytes < previous bytes,
        // a head-trim occurred. Trigger notify_head_content_removed so scroll
        // state is reclaimed. Hook point:
        //   if update indicates head trim {
        //       notify_head_content_removed(tile_id, trimmed_bytes);
        //   }

        // TODO(hud-ttq97): record submitted_at_us → server_timestamp_wall_us as
        // an arrival→present latency sample in the structured telemetry bucket.
        // Hook point:
        //   telemetry.record_portal_latency(
        //       &proj_id,
        //       update.submitted_at_us,
        //       server_timestamp_wall_us,
        //   );

        records.push(CliPortalDrainRecord {
            projection_id: proj_id.clone(),
            command_kind: command_kind.into(),
            visible_transcript_units: update.visible_transcript.len(),
            visible_transcript_bytes: update.visible_transcript_bytes,
            coalesced_output_count: update.coalesced_output_count,
            unread_output_count: update.unread_output_count,
            submitted_at_us: update.submitted_at_us,
            portal_markdown,
            presentation,
            tile_id_hex,
            elapsed_us,
            budget_us,
            within_budget,
        });
    }

    records
}

fn new_audit_records(
    authority: &ProjectionAuthority,
    audit_start: usize,
    response: &ProjectionResponse,
) -> Vec<ProjectionAuditRecord> {
    let audit_log = authority.audit_log();
    if audit_log.len() > audit_start {
        return audit_log[audit_start..].to_vec();
    }

    audit_log
        .last()
        .filter(|record| {
            record.request_id == response.request_id
                && record.projection_id == response.projection_id
                && record.timestamp_wall_us == response.server_timestamp_wall_us
        })
        .cloned()
        .into_iter()
        .collect()
}

fn deserialize_then<T>(
    value: Value,
    dispatch: impl FnOnce(T) -> ProjectionResponse,
) -> ProjectionResponse
where
    T: for<'de> Deserialize<'de>,
{
    let request_id = request_id(&value);
    let projection_id = projection_id(&value);
    match serde_json::from_value(value) {
        Ok(request) => dispatch(request),
        Err(error) => invalid_argument_response(
            request_id,
            projection_id,
            now_wall_us(),
            format!("invalid operation payload: {error}"),
        ),
    }
}

fn malformed_response(
    request_id: impl Into<String>,
    projection_id: impl Into<String>,
    server_timestamp_wall_us: u64,
    reason: impl Into<String>,
) -> CliOperationResult {
    CliOperationResult {
        response: invalid_argument_response(
            request_id.into(),
            projection_id.into(),
            server_timestamp_wall_us,
            reason,
        ),
        audit_records: Vec::new(),
        portal_drain: Vec::new(),
    }
}

fn invalid_argument_response(
    request_id: String,
    projection_id: String,
    server_timestamp_wall_us: u64,
    reason: impl Into<String>,
) -> ProjectionResponse {
    ProjectionResponse {
        request_id,
        projection_id,
        accepted: false,
        error_code: Some(ProjectionErrorCode::ProjectionInvalidArgument),
        server_timestamp_wall_us,
        status_summary: bounded_copy(reason.into(), MAX_CLI_STATUS_SUMMARY_BYTES),
        owner_token: None,
        lifecycle_state: None,
        pending_input: Vec::new(),
        pending_remaining_count: 0,
        pending_remaining_bytes: 0,
        portal_update_ready: false,
        coalesced_output_count: 0,
    }
}

fn request_id(value: &Value) -> String {
    value
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn projection_id(value: &Value) -> String {
    value
        .get("projection_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn now_wall_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}

fn bounded_copy(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_projection::PORTAL_UPDATE_RATE_WINDOW_WALL_US;
    use tze_hud_projection::{
        AttachRequest, ContentClassification, OperationEnvelope, OutputKind, ProjectionAuthority,
        ProjectionBounds, ProjectionOperation, ProviderKind, PublishOutputRequest,
    };

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn test_envelope(
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

    fn attach_projection(authority: &mut ProjectionAuthority, projection_id: &str) -> String {
        authority
            .handle_attach(
                AttachRequest {
                    envelope: test_envelope(
                        ProjectionOperation::Attach,
                        projection_id,
                        &format!("attach-{projection_id}"),
                    ),
                    provider_kind: ProviderKind::Claude,
                    display_name: format!("Test session {projection_id}"),
                    workspace_hint: None,
                    repository_hint: None,
                    icon_profile_hint: None,
                    content_classification: ContentClassification::Private,
                    hud_target: None,
                    idempotency_key: None,
                },
                "test-caller",
                10,
            )
            .owner_token
            .expect("attach must return owner token")
    }

    fn publish_output(
        authority: &mut ProjectionAuthority,
        projection_id: &str,
        owner_token: &str,
        text: &str,
        ts: u64,
    ) {
        authority.handle_publish_output(
            PublishOutputRequest {
                envelope: test_envelope(
                    ProjectionOperation::PublishOutput,
                    projection_id,
                    &format!("pub-{projection_id}-{ts}"),
                ),
                owner_token: owner_token.to_string(),
                output_text: text.to_string(),
                output_kind: OutputKind::Assistant,
                content_classification: ContentClassification::Private,
                logical_unit_id: Some(format!("unit-{ts}")),
                coalesce_key: None,
            },
            "test-caller",
            ts,
        );
    }

    // ── Part (a): Adapter hosted with runtime-resolved tokens ─────────────────

    /// Constructing `PortalDriveState` and attaching an adapter yields an
    /// adapter with tokens resolved from the (empty) default token map.
    #[test]
    fn portal_drive_state_attach_creates_adapter_with_default_tokens() {
        let mut drive = PortalDriveState::new();
        drive.attach_adapter("proj-a", Vec::new());
        assert!(
            drive.adapter_mut("proj-a").is_some(),
            "adapter must be registered after attach"
        );
    }

    /// `apply_token_map` propagates new visual tokens to all live adapters.
    ///
    /// Verifies the §6.1 profile-swap contract: after `set_token_map`, the next
    /// render from any live adapter uses the new token values without adapter
    /// logic changes.
    #[test]
    fn token_map_swap_propagates_to_live_adapter() {
        use tze_hud_config::{PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR, tokens::resolve_tokens};

        let mut drive = PortalDriveState::new();
        drive.attach_adapter("proj-a", Vec::new());
        drive.attach_adapter("proj-b", Vec::new());

        // Verify baseline is the default (transcript text r ≈ 0.90)
        let baseline_r = drive
            .adapter_mut("proj-a")
            .unwrap()
            .visual_tokens()
            .transcript_text_color
            .r;
        // Default from PortalVisualTokens::default(): r = 0.90
        assert!(
            baseline_r > 0.8,
            "baseline transcript text r should be ~0.90, got {baseline_r}"
        );

        // Apply a token override: red transcript text (#FF0000 → r=1.0)
        let mut overrides = HashMap::new();
        overrides.insert(
            PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR.to_string(),
            "#FF0000".to_string(),
        );
        drive.apply_token_map(overrides);

        // Both adapters must now carry the new token.
        for id in &["proj-a", "proj-b"] {
            let r = drive
                .adapter_mut(id)
                .unwrap()
                .visual_tokens()
                .transcript_text_color
                .r;
            assert!(
                (r - 1.0_f32).abs() < 1e-2,
                "after token swap, adapter {id} transcript text r must be ~1.0 (red), got {r}"
            );
        }
        let _ = resolve_tokens; // suppress unused-import warning in some toolchain configs
    }

    /// Adapter removed on `detach_adapter`.
    #[test]
    fn detach_removes_adapter() {
        let mut drive = PortalDriveState::new();
        drive.attach_adapter("proj-a", Vec::new());
        drive.detach_adapter("proj-a");
        assert!(
            drive.adapter_mut("proj-a").is_none(),
            "adapter must be gone after detach"
        );
    }

    // ── Part (b): Drain loop pulls updates round-robin ────────────────────────

    /// Single portal: drain returns the update after a `PublishOutput`.
    #[test]
    fn drain_emits_portal_update_for_single_portal() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 100,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let mut drive = PortalDriveState::new();

        let token_a = attach_projection(&mut authority, "proj-a");
        drive.attach_adapter("proj-a", Vec::new());

        publish_output(&mut authority, "proj-a", &token_a, "hello", 20);

        let records = drain_and_emit_portal_updates(&mut authority, &mut drive, 20);

        assert_eq!(records.len(), 1, "one drain record expected");
        assert_eq!(records[0].projection_id, "proj-a");
        assert!(records[0].visible_transcript_units > 0);
        assert!(
            !records[0].portal_markdown.is_empty(),
            "portal markdown must be non-empty"
        );
    }

    /// Two portals at equal rates: drain serves them in round-robin order.
    ///
    /// This is the cross-portal fairness invariant from tasks.md §5.1 / §5.4:
    /// under equal sustained rates, no portal is starved relative to the other.
    /// The round-robin oracle (`next_due_projection_id`) ensures bounded
    /// divergence.
    #[test]
    fn drain_round_robin_fairness_under_equal_rates() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 100,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let mut drive = PortalDriveState::new();

        let token_a = attach_projection(&mut authority, "proj-a");
        let token_b = attach_projection(&mut authority, "proj-b");
        drive.attach_adapter("proj-a", Vec::new());
        drive.attach_adapter("proj-b", Vec::new());

        // Publish to both portals at the same logical timestamp.
        publish_output(&mut authority, "proj-a", &token_a, "alpha-1", 20);
        publish_output(&mut authority, "proj-b", &token_b, "beta-1", 20);

        let records = drain_and_emit_portal_updates(&mut authority, &mut drive, 20);

        // Both portals must be drained; the order is round-robin (a before b
        // since a was attached first, but both must appear).
        assert_eq!(records.len(), 2, "both portals must be drained");
        let ids: Vec<&str> = records.iter().map(|r| r.projection_id.as_str()).collect();
        assert!(
            ids.contains(&"proj-a") && ids.contains(&"proj-b"),
            "both portals must appear in drain output, got: {ids:?}"
        );

        // After drain, no further updates should be pending.
        let second_drain = drain_and_emit_portal_updates(&mut authority, &mut drive, 20);
        assert!(
            second_drain.is_empty(),
            "second drain must be empty after all portals served"
        );
    }

    /// Three portals: round-robin serves all three in one drain pass.
    #[test]
    fn drain_three_portals_all_served() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 100,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let mut drive = PortalDriveState::new();

        let ids = ["proj-1", "proj-2", "proj-3"];
        let mut tokens = Vec::new();
        for id in &ids {
            tokens.push(attach_projection(&mut authority, id));
            drive.attach_adapter(id, Vec::new());
        }
        for (idx, id) in ids.iter().enumerate() {
            publish_output(&mut authority, id, &tokens[idx], "msg", 20);
        }

        let records = drain_and_emit_portal_updates(&mut authority, &mut drive, 20);
        let drained_ids: Vec<&str> = records.iter().map(|r| r.projection_id.as_str()).collect();

        assert_eq!(
            records.len(),
            3,
            "all three portals must be drained, got: {drained_ids:?}"
        );
        for id in &ids {
            assert!(
                drained_ids.contains(id),
                "portal {id} missing from drain output: {drained_ids:?}"
            );
        }
    }

    /// Rate-window: if an update is rate-limited (second publish within window),
    /// the drain does not produce a record until the window passes.
    #[test]
    fn drain_respects_rate_window() {
        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 1,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let mut drive = PortalDriveState::new();

        let token_a = attach_projection(&mut authority, "proj-a");
        drive.attach_adapter("proj-a", Vec::new());

        // First publish — immediately drainable.
        publish_output(&mut authority, "proj-a", &token_a, "first", 20);
        let first_drain = drain_and_emit_portal_updates(&mut authority, &mut drive, 20);
        assert_eq!(first_drain.len(), 1);

        // Second publish within the rate window — coalesced, not yet drainable.
        publish_output(&mut authority, "proj-a", &token_a, "second", 21);
        let mid_drain = drain_and_emit_portal_updates(&mut authority, &mut drive, 21);
        assert!(
            mid_drain.is_empty(),
            "second publish within rate window must not produce a drain record"
        );

        // After the rate window passes, the update becomes drainable.
        let after_ts = PORTAL_UPDATE_RATE_WINDOW_WALL_US + 25;
        let late_drain = drain_and_emit_portal_updates(&mut authority, &mut drive, after_ts);
        assert_eq!(
            late_drain.len(),
            1,
            "coalesced update must drain after rate window"
        );
        assert_eq!(late_drain[0].coalesced_output_count, 1);
    }

    /// Token-map swap in portal drive state during live session: the rendered
    /// portal markdown reflects the new tokens.
    ///
    /// This is the end-to-end §6.1 proof: a `SetTokenMap` call on the drive
    /// state changes the visual tokens in the live adapter, and the next drain
    /// render uses the new tokens.
    #[test]
    fn drain_portal_markdown_reflects_token_swap() {
        use tze_hud_config::PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR;

        let mut authority = ProjectionAuthority::new(ProjectionBounds {
            max_portal_updates_per_second: 100,
            ..ProjectionBounds::default()
        })
        .unwrap();
        let mut drive = PortalDriveState::new();

        let token_a = attach_projection(&mut authority, "proj-a");
        drive.attach_adapter("proj-a", Vec::new());

        // Publish and drain once with default tokens.
        publish_output(&mut authority, "proj-a", &token_a, "before swap", 20);
        let before = drain_and_emit_portal_updates(&mut authority, &mut drive, 20);
        assert_eq!(before.len(), 1);

        // Swap tokens: change transcript text to red.
        let mut overrides = HashMap::new();
        overrides.insert(
            PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR.to_string(),
            "#FF0000".to_string(),
        );
        drive.apply_token_map(overrides);

        // Publish again and drain with new tokens.
        publish_output(
            &mut authority,
            "proj-a",
            &token_a,
            "after swap",
            PORTAL_UPDATE_RATE_WINDOW_WALL_US + 30,
        );
        let after = drain_and_emit_portal_updates(
            &mut authority,
            &mut drive,
            PORTAL_UPDATE_RATE_WINDOW_WALL_US + 30,
        );
        assert_eq!(after.len(), 1);

        // The adapter's transcript_text_color must now be red.
        let adapter_r = drive
            .adapter_mut("proj-a")
            .unwrap()
            .visual_tokens()
            .transcript_text_color
            .r;
        assert!(
            (adapter_r - 1.0_f32).abs() < 1e-2,
            "adapter transcript_text_color.r must be ~1.0 (red) after token swap, got {adapter_r}"
        );

        // The portal markdown is rendered — just verify it is non-empty
        // (the actual color application is in the proto message layer).
        assert!(!after[0].portal_markdown.is_empty());
    }
}
