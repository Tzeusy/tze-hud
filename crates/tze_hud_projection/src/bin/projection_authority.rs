//! Stdio control surface for the cooperative HUD projection authority.
//!
//! This executable is intentionally a projection-daemon surface, not runtime
//! v1 MCP. It retains `ProjectionAuthority` state only for the lifetime of
//! this process and dispatches newline-delimited JSON requests to the
//! normative operation handlers in `tze_hud_projection`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use tze_hud_projection::{
    AcknowledgeInputRequest, AttachRequest, CleanupRequest, DetachRequest, GetPendingInputRequest,
    ProjectionAuditRecord, ProjectionAuthority, ProjectionBounds, ProjectionErrorCode,
    ProjectionOperation, ProjectionResponse, PublishOutputRequest, PublishStatusRequest,
};

const DEFAULT_CALLER_IDENTITY: &str = "projection-authority-stdio";
const MAX_CLI_STATUS_SUMMARY_BYTES: usize = 512;
const MAX_STDIN_LINE_BYTES: usize = 65_536;

#[derive(Debug)]
struct CliConfig {
    caller_identity: String,
    operator_authority: Option<String>,
}

#[derive(Debug, Serialize)]
struct CliOperationResult {
    response: ProjectionResponse,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    audit_records: Vec<ProjectionAuditRecord>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = parse_args(env::args().skip(1))?;
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
    let mut caller_identity = DEFAULT_CALLER_IDENTITY.to_string();
    let mut operator_authority = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--stdio" => {}
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
        caller_identity,
        operator_authority,
    })
}

fn print_help() {
    println!(
        "Usage: tze_hud_projection_authority [--stdio] [--caller-identity ID] [--operator-authority-env VAR]\n\
         \n\
         Reads one cooperative HUD projection operation JSON object per stdin line and writes one JSON result per stdout line.\n\
         The process retains projection state in memory only and emits bounded operation responses plus newly written audit records."
    );
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
