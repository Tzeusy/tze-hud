//! Agent scene event emission handler (scene-events/spec.md §5.1–§5.4).
//!
//! Extracted from `session_server/mod.rs` in SS-7h.
//! Contains `handle_emit_scene_event` and its private validation helper.

use super::*;

// ─── Agent Scene Event Emission handler ──────────────────────────────────────

/// Handle an `EmitSceneEvent` request from an agent.
///
/// Implements the server-side of the agent event emission protocol per
/// scene-events/spec.md §5.1–§5.4:
///
/// 1. Validate the bare name (format + reserved prefix).
/// 2. Check the `emit_scene_event:<bare_name>` capability.
/// 3. Enforce the 4 KB payload size limit.
/// 4. Apply the per-session sliding-window rate limit.
/// 5. On success, dispatch the event to subscribers and respond with the
///    fully-prefixed event type.
///
/// Per-session rate limiting is enforced via the
/// `StreamSession::agent_event_rate_limiter: AgentEventRateLimiter` field.
///
/// Note: Full event bus delivery to subscribers (step 5) is wired in by bead #2.
/// This handler performs all gating checks and returns a result; actual fan-out
/// to subscription channels is not implemented in this bead.
pub(super) async fn handle_emit_scene_event(
    _state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    request_sequence: u64,
    emit: EmitSceneEvent,
) {
    // Run all validation checks. On rejection, the Err variant carries the
    // (error_code, error_message) pair for the wire response; on success, the
    // Ok variant carries the fully-prefixed delivered_event_type.
    let outcome = validate_emission(session, &emit);

    let seq = session.next_server_seq();
    let (accepted, delivered_event_type, error_code, error_message) = match outcome {
        Ok(delivered) => (true, delivered, String::new(), String::new()),
        Err((code, msg)) => (false, String::new(), code, msg),
    };

    // TODO (bead #2): on accepted, dispatch delivered_event_type to subscribers
    // via the event bus.

    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::EmitSceneEventResult(EmitSceneEventResult {
                request_sequence,
                accepted,
                delivered_event_type,
                error_code,
                error_message,
            })),
        }))
        .await;
}

/// Validate all emission gates for `EmitSceneEvent`, mutating session state
/// (rate-limiter) only on acceptance.
///
/// Returns `Ok(delivered_event_type)` on success, or
/// `Err((error_code, error_message))` on the first failing gate.
///
/// Validation order (spec §5.1–§5.4):
/// 1. Bare name format and reserved-prefix check.
/// 2. Capability check (`emit_scene_event:<bare_name>`).
/// 3. Payload size limit (≤ [`MAX_PAYLOAD_BYTES`]).
/// 4. Sliding-window rate limit ([`DEFAULT_MAX_EVENTS_PER_SECOND`] events/s).
fn validate_emission(
    session: &mut StreamSession,
    emit: &EmitSceneEvent,
) -> Result<String, (String, String)> {
    use tze_hud_scene::events::naming::{NamingError, build_agent_event_type, validate_bare_name};

    // ── Step 1: Validate bare name (format + reserved prefix) ────────────
    if let Err(naming_err) = validate_bare_name(&emit.bare_name) {
        let (code, msg) = match &naming_err {
            NamingError::ReservedPrefix { prefix } => (
                "AGENT_EVENT_RESERVED_PREFIX".to_string(),
                format!("bare name must not start with reserved prefix {prefix:?}"),
            ),
            _ => (
                "AGENT_EVENT_INVALID_NAME".to_string(),
                format!("invalid bare name: {naming_err}"),
            ),
        };
        return Err((code, msg));
    }

    // ── Step 2: Capability check ──────────────────────────────────────────
    let required_cap = format!("emit_scene_event:{}", emit.bare_name);
    if !capability_set_covers(&session.capabilities, &required_cap) {
        return Err((
            "AGENT_EVENT_CAPABILITY_MISSING".to_string(),
            format!("missing capability: {required_cap}"),
        ));
    }

    // ── Step 3: Payload size limit ────────────────────────────────────────
    if emit.payload.len() > MAX_PAYLOAD_BYTES {
        return Err((
            "AGENT_EVENT_PAYLOAD_TOO_LARGE".to_string(),
            format!(
                "payload {} bytes exceeds {MAX_PAYLOAD_BYTES}-byte limit",
                emit.payload.len()
            ),
        ));
    }

    // ── Step 4: Rate limit ────────────────────────────────────────────────
    if session
        .agent_event_rate_limiter
        .check_and_record(std::time::Instant::now())
        .is_err()
    {
        return Err((
            "AGENT_EVENT_RATE_EXCEEDED".to_string(),
            format!(
                "agent event rate limit exceeded ({DEFAULT_MAX_EVENTS_PER_SECOND}/s sliding window)"
            ),
        ));
    }

    // ── Accepted: build fully-prefixed event type ─────────────────────────
    Ok(build_agent_event_type(&session.namespace, &emit.bare_name))
}
