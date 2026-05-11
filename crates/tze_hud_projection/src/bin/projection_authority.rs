//! Stdio control surface for the cooperative HUD projection authority.
//!
//! This executable is intentionally a projection-daemon surface, not runtime
//! v1 MCP. It retains `ProjectionAuthority` state only for the lifetime of
//! this process and dispatches newline-delimited JSON requests to the
//! normative operation handlers in `tze_hud_projection`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};
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

    serve_stdio(&mut authority, &config)
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

fn serve_stdio(authority: &mut ProjectionAuthority, config: &CliConfig) -> Result<(), String> {
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
                dispatch_line(authority, config, &line)
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
            authority.handle_attach(request, caller_identity, server_timestamp_wall_us)
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
            authority.handle_detach(request, caller_identity, server_timestamp_wall_us)
        }),
        ProjectionOperation::Cleanup => deserialize_then(value, |request: CleanupRequest| {
            authority.handle_cleanup(request, caller_identity, server_timestamp_wall_us)
        }),
    };

    let audit_records = new_audit_records(authority, audit_start, &response);
    CliOperationResult {
        response,
        audit_records,
    }
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
