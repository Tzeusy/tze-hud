//! Media ingress handlers — RFC 0014 §2.2.1 (SS-7c).
//!
//! Handles `MediaIngressOpen` and `MediaIngressClose` client messages and the
//! `close_active_media_ingress` helper that the session loop and capability
//! revocation path also call directly.

use crate::proto::session::server_message::Payload as ServerPayload;
use crate::proto::session::*;
use crate::session::{MediaIngressSharedState, SharedState};
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::Status;
use tze_hud_scene::types::ZoneContent;

use super::stream_session::{ActiveMediaIngressStream, StreamSession};
use super::{
    DEFAULT_MAX_FUTURE_SCHEDULE_US, MEDIA_INGRESS_PEAK_KBPS_BUDGET, capability_set_covers,
    now_wall_us, scene_id_to_bytes, validate_timing_hints,
};

// ─── Send helpers ────────────────────────────────────────────────────────────

async fn send_media_open_result(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    result: MediaIngressOpenResult,
) {
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::MediaIngressOpenResult(result)),
        }))
        .await;
}

async fn send_media_state(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    stream_epoch: u64,
    state: i32,
) {
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::MediaIngressState(MediaIngressState {
                stream_epoch,
                state,
                current_step: 0,
                effective_bitrate_kbps: 0,
                effective_fps: 0,
                effective_width_px: 0,
                effective_height_px: 0,
                dropped_frames_since_last: 0,
                watchdog_warnings: 0,
                sample_timestamp_wall_us: now_wall_us(),
            })),
        }))
        .await;
}

async fn send_media_close_notice(
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    stream_epoch: u64,
    reason: i32,
    detail: impl Into<String>,
    retry_after_us: Option<u64>,
) {
    let seq = session.next_server_seq();
    let _ = tx
        .send(Ok(ServerMessage {
            sequence: seq,
            timestamp_wall_us: now_wall_us(),
            payload: Some(ServerPayload::MediaIngressCloseNotice(
                MediaIngressCloseNotice {
                    stream_epoch,
                    reason,
                    detail: detail.into(),
                    retry_after_us,
                },
            )),
        }))
        .await;
}

// ─── Admission helpers ───────────────────────────────────────────────────────

fn media_open_rejected(
    client_stream_id: Vec<u8>,
    code: &str,
    reason: impl Into<String>,
) -> MediaIngressOpenResult {
    MediaIngressOpenResult {
        client_stream_id,
        admitted: false,
        stream_epoch: 0,
        assigned_surface_id: Vec::new(),
        selected_codec: 0,
        runtime_sdp_offer: Vec::new(),
        reject_reason: reason.into(),
        reject_code: code.to_string(),
        runtime_sdp_answer: Vec::new(),
    }
}

fn supported_video_codec(open: &MediaIngressOpen) -> Option<i32> {
    open.codec_preference
        .iter()
        .copied()
        .find(|codec| matches!(*codec, 1..=3))
}

fn media_open_rejection(
    open: &MediaIngressOpen,
    media_config: &tze_hud_scene::config::MediaIngressConfig,
    session: &StreamSession,
) -> Option<(&'static str, String)> {
    if !media_config.enabled || media_config.operator_disabled {
        return Some((
            "MEDIA_DISABLED",
            "media ingress is disabled by runtime configuration".to_string(),
        ));
    }
    if !capability_set_covers(&session.capabilities, "media_ingress") {
        return Some((
            "CAPABILITY_REQUIRED",
            "session does not hold media_ingress capability".to_string(),
        ));
    }
    if session.media_ingress.is_some() {
        return Some((
            "SESSION_STREAM_LIMIT",
            "one active media ingress stream is already admitted for this session".to_string(),
        ));
    }
    if open.has_audio_track {
        return Some((
            "AUDIO_NOT_SUPPORTED",
            "the Windows media ingress exemplar is video-only".to_string(),
        ));
    }
    if !open.has_video_track {
        return Some((
            "INVALID_ARGUMENT",
            "media ingress requires a video track".to_string(),
        ));
    }
    let Some(transport) = open.transport.as_ref() else {
        return Some((
            "INVALID_ARGUMENT",
            "media ingress requires a transport descriptor".to_string(),
        ));
    };
    if transport.mode != MediaTransportMode::WebrtcStandard as i32 {
        return Some((
            "CAPABILITY_NOT_IMPLEMENTED",
            "only WEBRTC_STANDARD transport is active for this slice".to_string(),
        ));
    }
    let zone_name = match open.surface_binding.as_ref() {
        Some(media_ingress_open::SurfaceBinding::ZoneName(zone)) => zone,
        Some(media_ingress_open::SurfaceBinding::TileId(_)) => {
            return Some((
                "SURFACE_NOT_FOUND",
                "tile-bound media ingress is deferred; use approved zone media-pip".to_string(),
            ));
        }
        None => {
            return Some((
                "SURFACE_NOT_FOUND",
                "media ingress requires an approved zone binding".to_string(),
            ));
        }
    };
    let approved_zone = media_config.approved_zone.as_deref().unwrap_or_default();
    if approved_zone != tze_hud_scene::config::APPROVED_MEDIA_ZONE {
        return Some((
            "SURFACE_NOT_FOUND",
            format!("configured media ingress zone {approved_zone:?} is not active in this slice"),
        ));
    }
    if zone_name != approved_zone {
        return Some((
            "SURFACE_NOT_FOUND",
            format!("media ingress is only approved for zone {approved_zone:?}"),
        ));
    }
    let Some(default_classification) = media_config.default_classification.as_deref() else {
        return Some((
            "CONTENT_CLASS_DENIED",
            "media ingress has no default content classification".to_string(),
        ));
    };
    if open.content_classification.is_empty() {
        return Some((
            "CONTENT_CLASS_DENIED",
            "content_classification is required for media ingress".to_string(),
        ));
    }
    if open.content_classification != default_classification {
        return Some((
            "CONTENT_CLASS_DENIED",
            format!(
                "content classification {:?} is not allowed for this media ingress surface",
                open.content_classification
            ),
        ));
    }
    if open.declared_peak_kbps > MEDIA_INGRESS_PEAK_KBPS_BUDGET {
        return Some((
            "BUDGET_EXCEEDED",
            format!(
                "declared_peak_kbps {} exceeds one-stream media ingress budget {}",
                open.declared_peak_kbps, MEDIA_INGRESS_PEAK_KBPS_BUDGET
            ),
        ));
    }
    if supported_video_codec(open).is_none() {
        return Some((
            "CODEC_UNSUPPORTED",
            "no supported video codec preference was provided".to_string(),
        ));
    }
    let timing = TimingHints {
        present_at_wall_us: open.present_at_wall_us,
        expires_at_wall_us: open.expires_at_wall_us,
    };
    if let Err((code, message)) = validate_timing_hints(
        &timing,
        session.session_open_at_wall_us,
        DEFAULT_MAX_FUTURE_SCHEDULE_US,
    ) {
        return Some((code, message));
    }
    None
}

// ─── Handlers ────────────────────────────────────────────────────────────────

pub(super) async fn handle_media_ingress_open(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    media_config: &tze_hud_scene::config::MediaIngressConfig,
    open: MediaIngressOpen,
) {
    if let Some((code, reason)) = media_open_rejection(&open, media_config, session) {
        tracing::warn!(
            subsystem = "media_ingress",
            agent = %session.agent_name,
            reject_code = code,
            reject_reason = %reason,
            "media ingress admission rejected"
        );
        send_media_open_result(
            session,
            tx,
            media_open_rejected(open.client_stream_id, code, reason),
        )
        .await;
        return;
    }

    let zone_name = media_config
        .approved_zone
        .as_deref()
        .unwrap_or(tze_hud_scene::config::APPROVED_MEDIA_ZONE)
        .to_string();
    let selected_codec = supported_video_codec(&open).unwrap_or(1);
    let stream_epoch = session.next_media_epoch();
    let surface_id = tze_hud_scene::SceneId::new();

    enum MediaPublishAdmission {
        Published,
        GlobalLimit,
        PublishFailed(String),
    }

    let publish_result = {
        let mut st = state.lock().await;
        if st.media_ingress_active.is_some() {
            MediaPublishAdmission::GlobalLimit
        } else {
            let mut scene = st.scene.lock().await;
            let result = scene.publish_to_zone(
                &zone_name,
                ZoneContent::VideoSurfaceRef(surface_id),
                &session.namespace,
                None,
                if open.expires_at_wall_us == 0 {
                    None
                } else {
                    Some(open.expires_at_wall_us)
                },
                Some(open.content_classification.clone()),
            );
            drop(scene);
            if let Err(err) = result {
                MediaPublishAdmission::PublishFailed(err.to_string())
            } else {
                st.media_ingress_active = Some(MediaIngressSharedState {
                    publisher_namespace: session.namespace.clone(),
                    stream_epoch,
                    zone_name: zone_name.clone(),
                    surface_id,
                });
                MediaPublishAdmission::Published
            }
        }
    };
    match publish_result {
        MediaPublishAdmission::Published => {}
        MediaPublishAdmission::GlobalLimit => {
            let reason = "one active media ingress stream is already admitted globally".to_string();
            tracing::warn!(
                subsystem = "media_ingress",
                agent = %session.agent_name,
                reject_code = "SESSION_STREAM_LIMIT",
                reject_reason = %reason,
                "media ingress admission rejected"
            );
            send_media_open_result(
                session,
                tx,
                media_open_rejected(open.client_stream_id, "SESSION_STREAM_LIMIT", reason),
            )
            .await;
            return;
        }
        MediaPublishAdmission::PublishFailed(err) => {
            let reason = format!("approved media surface could not be published: {err}");
            tracing::warn!(
                subsystem = "media_ingress",
                agent = %session.agent_name,
                reject_code = "SURFACE_NOT_FOUND",
                reject_reason = %reason,
                "media ingress surface publish failed"
            );
            send_media_open_result(
                session,
                tx,
                media_open_rejected(open.client_stream_id, "SURFACE_NOT_FOUND", reason),
            )
            .await;
            return;
        }
    }

    session.media_ingress = Some(ActiveMediaIngressStream {
        stream_epoch,
        zone_name,
        surface_id,
    });
    tracing::info!(
        subsystem = "media_ingress",
        agent = %session.agent_name,
        stream_epoch,
        "media ingress admission granted"
    );
    send_media_open_result(
        session,
        tx,
        MediaIngressOpenResult {
            client_stream_id: open.client_stream_id,
            admitted: true,
            stream_epoch,
            assigned_surface_id: scene_id_to_bytes(surface_id),
            selected_codec,
            runtime_sdp_offer: Vec::new(),
            reject_reason: String::new(),
            reject_code: String::new(),
            runtime_sdp_answer: Vec::new(),
        },
    )
    .await;
    send_media_state(
        session,
        tx,
        stream_epoch,
        MediaSessionState::Admitted as i32,
    )
    .await;
}

pub(super) async fn close_active_media_ingress(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    reason: i32,
    detail: impl Into<String>,
    final_state: i32,
    retry_after_us: Option<u64>,
) -> bool {
    let Some(active) = session.media_ingress.take() else {
        return false;
    };
    let detail = detail.into();
    {
        let mut st = state.lock().await;
        if st
            .media_ingress_active
            .as_ref()
            .map(|global| {
                global.publisher_namespace == session.namespace
                    && global.stream_epoch == active.stream_epoch
                    && global.surface_id == active.surface_id
            })
            .unwrap_or(false)
        {
            st.media_ingress_active = None;
            let _ = st
                .scene
                .lock()
                .await
                .clear_zone_for_publisher(&active.zone_name, &session.namespace);
        }
    }
    tracing::info!(
        subsystem = "media_ingress",
        agent = %session.agent_name,
        stream_epoch = active.stream_epoch,
        surface_id = %active.surface_id,
        close_reason = reason,
        detail = %detail,
        "media ingress stream closed"
    );
    send_media_state(session, tx, active.stream_epoch, final_state).await;
    send_media_close_notice(
        session,
        tx,
        active.stream_epoch,
        reason,
        detail,
        retry_after_us,
    )
    .await;
    true
}

pub(super) async fn handle_media_ingress_close(
    state: &Arc<Mutex<SharedState>>,
    session: &mut StreamSession,
    tx: &tokio::sync::mpsc::Sender<Result<ServerMessage, Status>>,
    close: MediaIngressClose,
) {
    match session.media_ingress.as_ref() {
        Some(active) if active.stream_epoch == close.stream_epoch => {
            let detail = if close.reason.is_empty() {
                "agent closed media ingress stream".to_string()
            } else {
                close.reason
            };
            close_active_media_ingress(
                state,
                session,
                tx,
                MediaCloseReason::AgentClosed as i32,
                detail,
                MediaSessionState::Closed as i32,
                None,
            )
            .await;
        }
        _ => {
            super::send_runtime_error(
                session,
                tx,
                "MEDIA_STREAM_NOT_FOUND",
                "no active media ingress stream matches the requested stream_epoch",
                &format!("stream_epoch={}", close.stream_epoch),
                ErrorCode::InvalidArgument,
            )
            .await;
        }
    }
}
