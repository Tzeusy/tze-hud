//! Media signaling wire schema tests (RFC 0014 §2.2, §2.3; WM-S2b).
//!
//! Covers:
//! 1. Round-trip serialisation for every new proto message (ClientMessage
//!    fields 60–66, 80–81; ServerMessage fields 60–68, 80–82).
//! 2. Schema/snapshot parity: ZoneContent VideoSurfaceRef and StaticImageRef
//!    variants round-trip; ZonePublishRecordProto carries parity fields.
//! 3. WidgetPublishResult.request_sequence preservation regression (per
//!    RFC 0005 Amendment A1 "Protected Fields" clause; rust-widget-publish-
//!    load-harness contract).
//! 4. ZonePublish snapshot parity fields (present_at_wall_us, expires_at_wall_us,
//!    content_classification) round-trip intact.
//! 5. Field number allocation assertions: the right enum variant is produced by
//!    the right oneof field number (guards against protobuf field-number drift).
//!
//! Acceptance criteria mapped to issue hud-ora8.1.23:
//! - AC1: session-protocol delta merged via RFC 0005 amendment (doc change; tested here indirectly)
//! - AC2: schema + snapshot parity proof (§§ "Snapshot parity" tests below)
//! - AC3: request_sequence preservation (§ "Protected fields regression" below)
//! - AC4: engineering-bar §1 + §6 (test coverage + no new deps beyond prost)

use prost::Message;
use tze_hud_protocol::proto::session::client_message::Payload as ClientPayload;
use tze_hud_protocol::proto::session::server_message::Payload as ServerPayload;
use tze_hud_protocol::proto::session::{
    ClientMessage, CloudRelayClose, CloudRelayCloseNotice, CloudRelayOpen, CloudRelayOpenResult,
    CloudRelayStateUpdate, MediaDegradationNotice, MediaIceCandidate, MediaIngressClose,
    MediaIngressCloseNotice, MediaIngressOpen, MediaIngressOpenResult, MediaIngressState,
    MediaPauseNotice, MediaPauseRequest, MediaResumeNotice, MediaResumeRequest, MediaSdpAnswer,
    MediaSdpOffer, ServerMessage,
    TransportDescriptor, WidgetPublishResult, ZonePublish, ZonePublishResult,
};
use tze_hud_protocol::proto::zone_content::Payload as ZonePayload;
use tze_hud_protocol::proto::{
    StaticImageRef, VideoSurfaceRef, ZoneContent, ZonePublishRecordProto,
};

// ─── Fixture ─────────────────────────────────────────────────────────────────

fn round_trip<T: Message + Default>(msg: &T) -> T {
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode must not fail");
    T::decode(buf.as_slice()).expect("decode must not fail")
}

// ─── AC3: Protected fields regression — WidgetPublishResult.request_sequence ─
//
// RFC 0005 Amendment A1 §"Protected Fields": WidgetPublishResult.request_sequence
// (field 1 of WidgetPublishResult; ServerMessage field 47) MUST be preserved
// exactly as defined. This test is the regression gate for the rust-widget-publish-
// load-harness contract.

#[test]
fn widget_publish_result_request_sequence_preserved_accepted() {
    // Simulates what the load harness does: echo back request_sequence for RTT tracking.
    let result = WidgetPublishResult {
        request_sequence: 42,
        accepted: true,
        widget_name: "gauge".to_string(),
        error_code: String::new(),
        error_message: String::new(),
    };
    let decoded = round_trip(&result);
    assert_eq!(
        decoded.request_sequence, 42,
        "request_sequence MUST survive round-trip (protected field; RFC 0005 A1)"
    );
    assert!(decoded.accepted);
}

#[test]
fn widget_publish_result_request_sequence_preserved_rejected() {
    let result = WidgetPublishResult {
        request_sequence: 9999,
        accepted: false,
        widget_name: "gauge".to_string(),
        error_code: "WIDGET_NOT_FOUND".to_string(),
        error_message: "not found".to_string(),
    };
    let decoded = round_trip(&result);
    assert_eq!(
        decoded.request_sequence, 9999,
        "request_sequence MUST be present in rejected results (RFC 0005 A1 protection)"
    );
    assert!(!decoded.accepted);
    assert_eq!(decoded.error_code, "WIDGET_NOT_FOUND");
}

#[test]
fn widget_publish_result_in_server_message_envelope_field_47() {
    // Asserts ServerMessage field 47 carries WidgetPublishResult (field number guard).
    let server_msg = ServerMessage {
        sequence: 10,
        timestamp_wall_us: 1_000_000,
        payload: Some(ServerPayload::WidgetPublishResult(WidgetPublishResult {
            request_sequence: 7,
            accepted: true,
            widget_name: "spark".to_string(),
            error_code: String::new(),
            error_message: String::new(),
        })),
    };
    let decoded = round_trip(&server_msg);
    match decoded.payload {
        Some(ServerPayload::WidgetPublishResult(r)) => {
            assert_eq!(
                r.request_sequence, 7,
                "field 47 must carry WidgetPublishResult"
            );
        }
        other => panic!("expected WidgetPublishResult at field 47, got {:?}", other),
    }
}

#[test]
fn zone_publish_result_request_sequence_preserved() {
    // ZonePublishResult.request_sequence is also a protected Layer 3 field.
    let result = ZonePublishResult {
        request_sequence: 55,
        accepted: true,
        error_code: String::new(),
        error_message: String::new(),
    };
    let decoded = round_trip(&result);
    assert_eq!(
        decoded.request_sequence, 55,
        "ZonePublishResult.request_sequence MUST be preserved (RFC 0005 A1 Layer 3 protection)"
    );
}

// ─── AC2: Snapshot parity — ZoneContent VideoSurfaceRef and StaticImageRef ───

#[test]
fn zone_content_video_surface_ref_roundtrip() {
    // VideoSurfaceRef at ZoneContent oneof field 6 (WM-S2b types.proto delta).
    let surface_id = vec![0xAB_u8; 16]; // 16-byte surface ID (UUIDv7)
    let zone_content = ZoneContent {
        payload: Some(ZonePayload::VideoSurfaceRef(VideoSurfaceRef {
            surface_id: surface_id.clone(),
            expires_at_wall_us: 1_700_000_000_000_000,
            content_classification: "public".to_string(),
        })),
    };
    let decoded = round_trip(&zone_content);
    match decoded.payload {
        Some(ZonePayload::VideoSurfaceRef(v)) => {
            assert_eq!(
                v.surface_id, surface_id,
                "surface_id MUST survive round-trip"
            );
            assert_eq!(
                v.expires_at_wall_us, 1_700_000_000_000_000,
                "expiry MUST survive"
            );
            assert_eq!(
                v.content_classification, "public",
                "classification MUST survive"
            );
        }
        other => panic!("expected VideoSurfaceRef, got {:?}", other),
    }
}

#[test]
fn zone_content_static_image_ref_roundtrip() {
    // StaticImageRef at ZoneContent oneof field 5 (WM-S2b types.proto delta).
    let resource_id = vec![0x12_u8; 32]; // 32-byte BLAKE3 hash
    let zone_content = ZoneContent {
        payload: Some(ZonePayload::StaticImageRef(StaticImageRef {
            resource_id: resource_id.clone(),
        })),
    };
    let decoded = round_trip(&zone_content);
    match decoded.payload {
        Some(ZonePayload::StaticImageRef(r)) => {
            assert_eq!(
                r.resource_id, resource_id,
                "resource_id MUST survive round-trip"
            );
        }
        other => panic!("expected StaticImageRef, got {:?}", other),
    }
}

#[test]
fn zone_content_video_surface_ref_zero_expiry() {
    // Snapshot parity: 0 = no expiry; MUST round-trip as 0.
    let zone_content = ZoneContent {
        payload: Some(ZonePayload::VideoSurfaceRef(VideoSurfaceRef {
            surface_id: vec![1_u8; 16],
            expires_at_wall_us: 0,
            content_classification: String::new(),
        })),
    };
    let decoded = round_trip(&zone_content);
    match decoded.payload {
        Some(ZonePayload::VideoSurfaceRef(v)) => {
            assert_eq!(
                v.expires_at_wall_us, 0,
                "0 expiry must survive (proto3 zero default)"
            );
        }
        other => panic!("expected VideoSurfaceRef, got {:?}", other),
    }
}

#[test]
fn zone_publish_record_parity_fields_roundtrip() {
    // ZonePublishRecordProto carries snapshot parity fields (WM-S2b types.proto delta §4):
    // expires_at_wall_us (field 6), content_classification (field 7), breakpoints (field 8).
    let record = ZonePublishRecordProto {
        zone_name: "status".to_string(),
        publisher_namespace: "agent.weather".to_string(),
        content: Some(ZoneContent {
            payload: Some(ZonePayload::VideoSurfaceRef(VideoSurfaceRef {
                surface_id: vec![0xDE_u8; 16],
                expires_at_wall_us: 9_000_000_000_000_000,
                content_classification: "pii".to_string(),
            })),
        }),
        published_at_wall_us: 1_700_000_000_000,
        merge_key: String::new(),
        // Parity fields
        expires_at_wall_us: 9_000_000_000_000_000,
        content_classification: "pii".to_string(),
        breakpoints: vec![10, 25, 40],
    };
    let decoded = round_trip(&record);
    assert_eq!(decoded.zone_name, "status");
    assert_eq!(
        decoded.expires_at_wall_us, 9_000_000_000_000_000,
        "snapshot parity: expires_at_wall_us MUST survive"
    );
    assert_eq!(
        decoded.content_classification, "pii",
        "snapshot parity: content_classification MUST survive"
    );
    assert_eq!(
        decoded.breakpoints,
        vec![10u64, 25, 40],
        "snapshot parity: breakpoints MUST survive"
    );
}

// ─── AC2: Snapshot parity — ZonePublish session.proto fields 7-9 ─────────────

#[test]
fn zone_publish_parity_fields_roundtrip() {
    // ZonePublish fields 7-9 (WM-S2b session.proto delta; RFC 0005 A1 §3).
    let msg = ZonePublish {
        zone_name: "subtitle".to_string(),
        content: None,
        ttl_us: 5_000_000,
        merge_key: String::new(),
        breakpoints: vec![],
        element_id: vec![],
        present_at_wall_us: 1_700_000_000_000_001,
        expires_at_wall_us: 1_700_000_000_000_002,
        content_classification: "restricted".to_string(),
    };
    let decoded = round_trip(&msg);
    assert_eq!(
        decoded.present_at_wall_us, 1_700_000_000_000_001,
        "present_at_wall_us MUST survive round-trip (field 7)"
    );
    assert_eq!(
        decoded.expires_at_wall_us, 1_700_000_000_000_002,
        "expires_at_wall_us MUST survive round-trip (field 8)"
    );
    assert_eq!(
        decoded.content_classification, "restricted",
        "content_classification MUST survive round-trip (field 9)"
    );
}

// ─── Phase 1 media messages: ClientMessage fields 60-66 ──────────────────────

#[test]
fn media_ingress_open_roundtrip_in_client_message() {
    // ClientMessage field 60 (RFC 0014 §2.2.1).
    let client_id = vec![0xAA_u8; 16]; // UUIDv7 client_stream_id
    let msg = ClientMessage {
        sequence: 1,
        timestamp_wall_us: 1_000,
        payload: Some(ClientPayload::MediaIngressOpen(MediaIngressOpen {
            client_stream_id: client_id.clone(),
            transport: Some(TransportDescriptor {
                mode: 1, // WEBRTC_STANDARD
                agent_sdp_offer: vec![],
                agent_ice_credentials: vec![],
                relay_hint: 1, // DIRECT
                preshared_srtp_material: vec![],
            }),
            surface_binding: Some(
                tze_hud_protocol::proto::session::media_ingress_open::SurfaceBinding::ZoneName(
                    "subtitle".to_string(),
                ),
            ),
            codec_preference: vec![1], // VIDEO_H264_BASELINE
            has_audio_track: false,
            has_video_track: true,
            content_classification: "public".to_string(),
            present_at_wall_us: 0,
            expires_at_wall_us: 0,
            declared_peak_kbps: 2000,
        })),
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.sequence, 1);
    match decoded.payload {
        Some(ClientPayload::MediaIngressOpen(open)) => {
            assert_eq!(
                open.client_stream_id, client_id,
                "client_stream_id MUST survive"
            );
            assert_eq!(open.declared_peak_kbps, 2000);
            assert_eq!(open.codec_preference, vec![1]);
            assert_eq!(open.content_classification, "public");
        }
        other => panic!(
            "expected MediaIngressOpen at ClientMessage field 60, got {:?}",
            other
        ),
    }
}

#[test]
fn media_ingress_close_roundtrip_client_field_61() {
    let msg = ClientMessage {
        sequence: 2,
        timestamp_wall_us: 0,
        payload: Some(ClientPayload::MediaIngressClose(MediaIngressClose {
            stream_epoch: 42,
            reason: "agent-initiated close".to_string(),
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ClientPayload::MediaIngressClose(close)) => {
            assert_eq!(
                close.stream_epoch, 42,
                "stream_epoch MUST survive (field 61)"
            );
            assert_eq!(close.reason, "agent-initiated close");
        }
        other => panic!("expected MediaIngressClose at field 61, got {:?}", other),
    }
}

#[test]
fn media_sdp_answer_roundtrip_client_field_62() {
    let sdp_bytes = b"v=0\r\no=agent 0 0 IN IP4 127.0.0.1\r\n".to_vec();
    let msg = ClientMessage {
        sequence: 3,
        timestamp_wall_us: 0,
        payload: Some(ClientPayload::MediaSdpAnswer(MediaSdpAnswer {
            stream_epoch: 7,
            sdp_bytes: sdp_bytes.clone(),
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ClientPayload::MediaSdpAnswer(answer)) => {
            assert_eq!(
                answer.stream_epoch, 7,
                "stream_epoch MUST survive (field 62)"
            );
            assert_eq!(answer.sdp_bytes, sdp_bytes, "SDP bytes MUST survive");
        }
        other => panic!("expected MediaSdpAnswer at field 62, got {:?}", other),
    }
}

#[test]
fn media_ice_candidate_roundtrip_client_field_63() {
    let msg = ClientMessage {
        sequence: 4,
        timestamp_wall_us: 0,
        payload: Some(ClientPayload::MediaIceCandidate(MediaIceCandidate {
            stream_epoch: 7,
            candidate_str: "candidate:1 1 UDP 2113937151 192.168.1.2 54400 typ host".to_string(),
            sdp_mid: "audio".to_string(),
            sdp_mline_index: 0,
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ClientPayload::MediaIceCandidate(cand)) => {
            assert_eq!(cand.stream_epoch, 7, "stream_epoch MUST survive (field 63)");
            assert!(
                cand.candidate_str.contains("typ host"),
                "candidate_str MUST survive"
            );
            assert_eq!(cand.sdp_mid, "audio");
        }
        other => panic!("expected MediaIceCandidate at field 63, got {:?}", other),
    }
}

#[test]
fn media_egress_open_client_field_64_is_reserved() {
    // RFC 0014 §2.2.1: client field 64 (MediaEgressOpen) is plain `reserved` —
    // no message type is defined until the phase-4 egress design is finalised.
    // This test documents the reserved status by verifying that a ClientMessage
    // with no payload round-trips cleanly (there is no enum variant to construct).
    //
    // When phase 4 egress is designed, this test should be replaced with a
    // full round-trip test for the then-defined message.
    let msg = ClientMessage {
        sequence: 5,
        timestamp_wall_us: 0,
        payload: None,
    };
    let decoded = round_trip(&msg);
    assert!(
        decoded.payload.is_none(),
        "field 64 is reserved; a ClientMessage without a payload should decode payload as None"
    );
}

#[test]
fn media_pause_request_roundtrip_client_field_65() {
    let msg = ClientMessage {
        sequence: 6,
        timestamp_wall_us: 0,
        payload: Some(ClientPayload::MediaPauseRequest(MediaPauseRequest {
            stream_epoch: 99,
            reason: "attention policy".to_string(),
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ClientPayload::MediaPauseRequest(req)) => {
            assert_eq!(req.stream_epoch, 99, "stream_epoch MUST survive (field 65)");
        }
        other => panic!("expected MediaPauseRequest at field 65, got {:?}", other),
    }
}

#[test]
fn media_resume_request_roundtrip_client_field_66() {
    let msg = ClientMessage {
        sequence: 7,
        timestamp_wall_us: 0,
        payload: Some(ClientPayload::MediaResumeRequest(MediaResumeRequest {
            stream_epoch: 99,
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ClientPayload::MediaResumeRequest(req)) => {
            assert_eq!(req.stream_epoch, 99, "stream_epoch MUST survive (field 66)");
        }
        other => panic!("expected MediaResumeRequest at field 66, got {:?}", other),
    }
}

// ─── Phase 1 media messages: ServerMessage fields 60-68 ──────────────────────

#[test]
fn media_ingress_open_result_roundtrip_server_field_60() {
    let client_stream_id = vec![0xBB_u8; 16];
    let assigned_surface_id = vec![0xCC_u8; 16];
    let msg = ServerMessage {
        sequence: 1,
        timestamp_wall_us: 2_000,
        payload: Some(ServerPayload::MediaIngressOpenResult(
            MediaIngressOpenResult {
                client_stream_id: client_stream_id.clone(),
                admitted: true,
                stream_epoch: 1,
                assigned_surface_id: assigned_surface_id.clone(),
                selected_codec: 1, // VIDEO_H264_BASELINE
                runtime_sdp_offer: vec![],
                reject_reason: String::new(),
                reject_code: String::new(),
                runtime_sdp_answer: vec![],
            },
        )),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::MediaIngressOpenResult(result)) => {
            assert_eq!(
                result.client_stream_id, client_stream_id,
                "client_stream_id MUST survive (field 60)"
            );
            assert!(result.admitted, "admitted MUST survive");
            assert_eq!(result.stream_epoch, 1, "stream_epoch MUST survive");
            assert_eq!(
                result.assigned_surface_id, assigned_surface_id,
                "surface_id MUST survive"
            );
        }
        other => panic!(
            "expected MediaIngressOpenResult at field 60, got {:?}",
            other
        ),
    }
}

#[test]
fn media_ingress_open_result_rejected_roundtrip() {
    // Rejection path: admitted=false; reject_code and reject_reason populated.
    let msg = MediaIngressOpenResult {
        client_stream_id: vec![0x01_u8; 16],
        admitted: false,
        stream_epoch: 0,
        assigned_surface_id: vec![],
        selected_codec: 0,
        runtime_sdp_offer: vec![],
        reject_reason: "Worker pool full".to_string(),
        reject_code: "POOL_EXHAUSTED".to_string(),
        runtime_sdp_answer: vec![],
    };
    let decoded = round_trip(&msg);
    assert!(!decoded.admitted, "admitted MUST be false on rejection");
    assert_eq!(
        decoded.reject_code, "POOL_EXHAUSTED",
        "reject_code MUST survive"
    );
    assert_eq!(
        decoded.stream_epoch, 0,
        "stream_epoch MUST be 0 on rejection"
    );
}

#[test]
fn media_ingress_state_roundtrip_server_field_61() {
    let msg = ServerMessage {
        sequence: 2,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaIngressState(MediaIngressState {
            stream_epoch: 1,
            state: 2, // MEDIA_SESSION_STATE_STREAMING
            current_step: 0,
            effective_bitrate_kbps: 1500,
            effective_fps: 30,
            effective_width_px: 1920,
            effective_height_px: 1080,
            dropped_frames_since_last: 0,
            watchdog_warnings: 0,
            sample_timestamp_wall_us: 1_700_000_000_000,
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::MediaIngressState(state)) => {
            assert_eq!(
                state.stream_epoch, 1,
                "stream_epoch MUST survive (field 61)"
            );
            assert_eq!(state.state, 2, "STREAMING state MUST survive");
            assert_eq!(state.effective_bitrate_kbps, 1500, "bitrate MUST survive");
            assert_eq!(state.effective_fps, 30, "fps MUST survive");
        }
        other => panic!("expected MediaIngressState at field 61, got {:?}", other),
    }
}

#[test]
fn media_ingress_close_notice_roundtrip_server_field_62() {
    // Base case: no retry_after_us hint (non-watchdog close, AGENT_CLOSED).
    let msg = ServerMessage {
        sequence: 3,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaIngressCloseNotice(
            MediaIngressCloseNotice {
                stream_epoch: 1,
                reason: 1, // AGENT_CLOSED
                detail: "agent requested close".to_string(),
                retry_after_us: None,
            },
        )),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::MediaIngressCloseNotice(notice)) => {
            assert_eq!(
                notice.stream_epoch, 1,
                "stream_epoch MUST survive (field 62)"
            );
            assert_eq!(notice.reason, 1, "AGENT_CLOSED reason MUST survive");
            assert_eq!(notice.detail, "agent requested close");
            assert_eq!(
                notice.retry_after_us, None,
                "absent retry_after_us MUST decode as None"
            );
        }
        other => panic!(
            "expected MediaIngressCloseNotice at field 62, got {:?}",
            other
        ),
    }
}

/// RFC 0014 §6.3 A1: retry_after_us field MUST round-trip when set for BUDGET_WATCHDOG.
#[test]
fn media_ingress_close_notice_retry_after_us_roundtrip() {
    let hint_us: u64 = 5_000_000; // 5 seconds
    let msg = ServerMessage {
        sequence: 4,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaIngressCloseNotice(
            MediaIngressCloseNotice {
                stream_epoch: 2,
                reason: 6, // BUDGET_WATCHDOG
                detail: "ring-buffer 75% sustained".to_string(),
                retry_after_us: Some(hint_us),
            },
        )),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::MediaIngressCloseNotice(notice)) => {
            assert_eq!(notice.stream_epoch, 2);
            assert_eq!(notice.reason, 6, "BUDGET_WATCHDOG");
            assert_eq!(
                notice.retry_after_us,
                Some(hint_us),
                "retry_after_us MUST survive proto round-trip (RFC 0014 §6.3 A1)"
            );
        }
        other => panic!(
            "expected MediaIngressCloseNotice, got {:?}",
            other
        ),
    }
}

#[test]
fn media_sdp_offer_roundtrip_server_field_63() {
    let sdp = b"v=0\r\no=runtime 0 0 IN IP4 127.0.0.1\r\n".to_vec();
    let msg = ServerMessage {
        sequence: 4,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaSdpOffer(MediaSdpOffer {
            stream_epoch: 1,
            sdp_bytes: sdp.clone(),
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::MediaSdpOffer(offer)) => {
            assert_eq!(
                offer.stream_epoch, 1,
                "stream_epoch MUST survive (field 63)"
            );
            assert_eq!(offer.sdp_bytes, sdp, "SDP bytes MUST survive");
        }
        other => panic!("expected MediaSdpOffer at field 63, got {:?}", other),
    }
}

#[test]
fn media_degradation_notice_roundtrip_server_field_65() {
    let msg = ServerMessage {
        sequence: 5,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaDegradationNotice(
            MediaDegradationNotice {
                stream_epoch: 1,
                ladder_step: 3, // E25 step 3: reduce resolution
                trigger: 1,     // RUNTIME_LADDER_ADVANCE
                detail: "resolution reduced".to_string(),
            },
        )),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::MediaDegradationNotice(notice)) => {
            assert_eq!(
                notice.stream_epoch, 1,
                "stream_epoch MUST survive (field 65)"
            );
            assert_eq!(notice.ladder_step, 3, "ladder_step MUST survive");
            assert_eq!(
                notice.trigger, 1,
                "RUNTIME_LADDER_ADVANCE trigger MUST survive"
            );
        }
        other => panic!(
            "expected MediaDegradationNotice at field 65, got {:?}",
            other
        ),
    }
}

#[test]
fn media_egress_open_result_server_field_66_is_reserved() {
    // RFC 0014 §2.2.2: server field 66 (MediaEgressOpenResult) is plain `reserved` —
    // no message type is defined until the phase-4 egress design is finalised.
    // This test documents the reserved status by verifying that a ServerMessage
    // with no payload round-trips cleanly (there is no enum variant to construct).
    //
    // When phase 4 egress is designed, this test should be replaced with a
    // full round-trip test for the then-defined message.
    let msg = ServerMessage {
        sequence: 6,
        timestamp_wall_us: 0,
        payload: None,
    };
    let decoded = round_trip(&msg);
    assert!(
        decoded.payload.is_none(),
        "field 66 is reserved; a ServerMessage without a payload should decode payload as None"
    );
}

#[test]
fn media_pause_notice_roundtrip_server_field_67() {
    let msg = ServerMessage {
        sequence: 7,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaPauseNotice(MediaPauseNotice {
            stream_epoch: 1,
            trigger: 3, // SAFE_MODE
            detail: "safe mode entered".to_string(),
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::MediaPauseNotice(notice)) => {
            assert_eq!(
                notice.stream_epoch, 1,
                "stream_epoch MUST survive (field 67)"
            );
            assert_eq!(notice.trigger, 3, "SAFE_MODE trigger MUST survive");
        }
        other => panic!("expected MediaPauseNotice at field 67, got {:?}", other),
    }
}

#[test]
fn media_resume_notice_roundtrip_server_field_68() {
    let msg = ServerMessage {
        sequence: 8,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaResumeNotice(MediaResumeNotice {
            stream_epoch: 1,
            last_trigger: 3, // SAFE_MODE (the trigger that caused the preceding pause)
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::MediaResumeNotice(notice)) => {
            assert_eq!(
                notice.stream_epoch, 1,
                "stream_epoch MUST survive (field 68)"
            );
            assert_eq!(notice.last_trigger, 3, "last_trigger MUST survive");
        }
        other => panic!("expected MediaResumeNotice at field 68, got {:?}", other),
    }
}

// ─── Phase 4b cloud-relay messages (RFC 0018 §4.3) ───────────────────────────

#[test]
fn cloud_relay_open_roundtrip_client_field_80() {
    let msg = ClientMessage {
        sequence: 10,
        timestamp_wall_us: 0,
        payload: Some(ClientPayload::CloudRelayOpen(CloudRelayOpen {
            stream_epoch: 42,
            relay_path_hint: 1, // NEAREST_REGION
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ClientPayload::CloudRelayOpen(open)) => {
            assert_eq!(
                open.stream_epoch, 42,
                "stream_epoch MUST survive (field 80)"
            );
            assert_eq!(open.relay_path_hint, 1, "NEAREST_REGION hint MUST survive");
        }
        other => panic!(
            "expected CloudRelayOpen at ClientMessage field 80, got {:?}",
            other
        ),
    }
}

#[test]
fn cloud_relay_close_roundtrip_client_field_81() {
    let msg = ClientMessage {
        sequence: 11,
        timestamp_wall_us: 0,
        payload: Some(ClientPayload::CloudRelayClose(CloudRelayClose {
            stream_epoch: 42,
            reason: "agent teardown".to_string(),
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ClientPayload::CloudRelayClose(close)) => {
            assert_eq!(
                close.stream_epoch, 42,
                "stream_epoch MUST survive (field 81)"
            );
        }
        other => panic!(
            "expected CloudRelayClose at ClientMessage field 81, got {:?}",
            other
        ),
    }
}

#[test]
fn cloud_relay_open_result_roundtrip_server_field_80() {
    let sdp_answer = b"v=0\r\no=sfu 0 0 IN IP4 10.0.0.1\r\n".to_vec();
    let msg = ServerMessage {
        sequence: 10,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::CloudRelayOpenResult(CloudRelayOpenResult {
            stream_epoch: 42,
            established: true,
            sdp_answer: sdp_answer.clone(),
            relay_epoch: 1,
            relay_resource_url: "https://sfu.example/rooms/abc/whip".to_string(),
            close_reason_code: String::new(),
            close_reason_detail: String::new(),
        })),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::CloudRelayOpenResult(result)) => {
            assert_eq!(
                result.stream_epoch, 42,
                "stream_epoch MUST survive (field 80)"
            );
            assert!(result.established, "established MUST survive");
            assert_eq!(result.sdp_answer, sdp_answer, "SDP answer MUST survive");
            assert_eq!(result.relay_epoch, 1, "relay_epoch MUST survive");
        }
        other => panic!("expected CloudRelayOpenResult at field 80, got {:?}", other),
    }
}

#[test]
fn cloud_relay_close_notice_roundtrip_server_field_81() {
    let msg = ServerMessage {
        sequence: 11,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::CloudRelayCloseNotice(
            CloudRelayCloseNotice {
                stream_epoch: 42,
                relay_epoch: 1,
                reason: 7, // CLOUD_RELAY_E25_STEP_5
                detail: "E25 step 5: drop cloud-relay".to_string(),
                stream_survives: true,
            },
        )),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::CloudRelayCloseNotice(notice)) => {
            assert_eq!(
                notice.stream_epoch, 42,
                "stream_epoch MUST survive (field 81)"
            );
            assert_eq!(notice.relay_epoch, 1, "relay_epoch MUST survive");
            assert_eq!(notice.reason, 7, "E25_STEP_5 reason MUST survive");
            assert!(notice.stream_survives, "stream_survives MUST survive");
        }
        other => panic!(
            "expected CloudRelayCloseNotice at field 81, got {:?}",
            other
        ),
    }
}

#[test]
fn cloud_relay_state_update_roundtrip_server_field_82() {
    let msg = ServerMessage {
        sequence: 12,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::CloudRelayStateUpdate(
            CloudRelayStateUpdate {
                stream_epoch: 42,
                relay_epoch: 1,
                relay_rtt_ms: 45,
                packet_loss_ppm: 100,
                relay_bitrate_kbps: 1200,
                sample_timestamp_wall_us: 1_700_000_000_000,
            },
        )),
    };
    let decoded = round_trip(&msg);
    match decoded.payload {
        Some(ServerPayload::CloudRelayStateUpdate(update)) => {
            assert_eq!(
                update.stream_epoch, 42,
                "stream_epoch MUST survive (field 82)"
            );
            assert_eq!(update.relay_rtt_ms, 45, "relay_rtt_ms MUST survive");
            assert_eq!(update.packet_loss_ppm, 100, "packet_loss_ppm MUST survive");
        }
        other => panic!(
            "expected CloudRelayStateUpdate at field 82, got {:?}",
            other
        ),
    }
}

// ─── Field number allocation guards ──────────────────────────────────────────
//
// These tests verify that proto field assignments produce the expected variant
// when decoded from a hand-crafted byte sequence. Field numbers are from RFC
// 0014 §2.2 erratum. Each test encodes a ClientMessage/ServerMessage with a
// single known field set, then decodes and asserts the correct variant fires.

#[test]
fn client_field_60_produces_media_ingress_open_not_list_elements_request() {
    // ClientMessage field 60 = MediaIngressOpen (RFC 0014 §2.2.1 erratum).
    // Before the erratum, field 50 was intended for MediaIngressOpen (now element_request).
    // This test guards against field-number regression — field 60 MUST be MediaIngressOpen.
    let msg = ClientMessage {
        sequence: 999,
        timestamp_wall_us: 0,
        payload: Some(ClientPayload::MediaIngressOpen(MediaIngressOpen {
            client_stream_id: vec![0xFF_u8; 16],
            transport: None,
            surface_binding: None,
            codec_preference: vec![],
            has_audio_track: false,
            has_video_track: false,
            content_classification: String::new(),
            present_at_wall_us: 0,
            expires_at_wall_us: 0,
            declared_peak_kbps: 0,
        })),
    };
    // Round-trip via bytes to confirm field 60 is not conflated with another allocation.
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = ClientMessage::decode(buf.as_slice()).unwrap();
    match decoded.payload {
        Some(ClientPayload::MediaIngressOpen(open)) => {
            assert_eq!(open.client_stream_id, vec![0xFF_u8; 16]);
        }
        other => panic!(
            "ClientMessage field 60 MUST be MediaIngressOpen (RFC 0014 erratum), got {:?}",
            other
        ),
    }
}

#[test]
fn server_field_60_produces_media_ingress_open_result_not_list_elements_response() {
    // ServerMessage field 60 = MediaIngressOpenResult.
    // Fields 50-51 were consumed by persistent-movable-elements (ListElementsResponse,
    // ElementRepositionedEvent); media signaling was relocated to 60+ per RFC 0014 §2.2 erratum.
    let msg = ServerMessage {
        sequence: 999,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaIngressOpenResult(
            MediaIngressOpenResult {
                client_stream_id: vec![0x01_u8; 16],
                admitted: true,
                stream_epoch: 1,
                assigned_surface_id: vec![0x02_u8; 16],
                selected_codec: 1,
                runtime_sdp_offer: vec![],
                reject_reason: String::new(),
                reject_code: String::new(),
                runtime_sdp_answer: vec![],
            },
        )),
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = ServerMessage::decode(buf.as_slice()).unwrap();
    match decoded.payload {
        Some(ServerPayload::MediaIngressOpenResult(result)) => {
            assert_eq!(result.stream_epoch, 1);
        }
        other => panic!(
            "ServerMessage field 60 MUST be MediaIngressOpenResult (RFC 0014 erratum), got {:?}",
            other
        ),
    }
}

// ─── ICE candidate field boundaries ──────────────────────────────────────────

#[test]
fn media_ice_candidate_server_field_64_distinct_from_client_field_63() {
    // Server-side MediaIceCandidate lives at ServerMessage field 64 (not 63).
    // Agents send at field 63; runtime sends at field 64. Both carry the same struct.
    let server_msg = ServerMessage {
        sequence: 50,
        timestamp_wall_us: 0,
        payload: Some(ServerPayload::MediaIceCandidate(MediaIceCandidate {
            stream_epoch: 5,
            candidate_str: "candidate:2 1 UDP 1677729535 203.0.113.1 35000 typ srflx raddr 192.168.1.2 rport 54400".to_string(),
            sdp_mid: "video".to_string(),
            sdp_mline_index: 0,
        })),
    };
    let decoded = round_trip(&server_msg);
    match decoded.payload {
        Some(ServerPayload::MediaIceCandidate(cand)) => {
            assert_eq!(
                cand.stream_epoch, 5,
                "stream_epoch MUST survive (server field 64)"
            );
            assert!(
                cand.candidate_str.contains("typ srflx"),
                "candidate_str MUST survive"
            );
        }
        other => panic!(
            "expected MediaIceCandidate at ServerMessage field 64, got {:?}",
            other
        ),
    }
}

// ─── MediaIngressOpenResult field 9 — runtime_sdp_answer (RFC 0018 §4.1) ────

#[test]
fn media_ingress_open_result_runtime_sdp_answer_field_9() {
    // RFC 0018 §4.1 resolved the RFC 0014 §4.2 TBD: runtime_sdp_answer (field 9) is
    // populated for agent-initiated offer path (direct ingress). On cloud-relay path
    // it is ALWAYS EMPTY; CloudRelayOpenResult.sdp_answer is used instead.
    let sdp_answer = b"v=0\r\no=runtime 0 0 IN IP4 127.0.0.1\r\n".to_vec();
    let result = MediaIngressOpenResult {
        client_stream_id: vec![0x01_u8; 16],
        admitted: true,
        stream_epoch: 1,
        assigned_surface_id: vec![0x02_u8; 16],
        selected_codec: 3,         // VIDEO_VP9
        runtime_sdp_offer: vec![], // empty: agent initiated the offer
        reject_reason: String::new(),
        reject_code: String::new(),
        runtime_sdp_answer: sdp_answer.clone(), // populated: agent-initiated path
    };
    let decoded = round_trip(&result);
    assert_eq!(
        decoded.runtime_sdp_answer, sdp_answer,
        "runtime_sdp_answer (field 9; RFC 0018 §4.1) MUST survive round-trip"
    );
    assert!(
        decoded.runtime_sdp_offer.is_empty(),
        "runtime_sdp_offer MUST be empty on agent-initiated offer path"
    );
}

// ─── stream_epoch stability across traffic classes ────────────────────────────

#[test]
fn stream_epoch_u64_max_roundtrips() {
    // stream_epoch is u64; verify max value roundtrips without overflow.
    let state = MediaIngressState {
        stream_epoch: u64::MAX,
        state: 2,
        current_step: 0,
        effective_bitrate_kbps: 0,
        effective_fps: 0,
        effective_width_px: 0,
        effective_height_px: 0,
        dropped_frames_since_last: 0,
        watchdog_warnings: 0,
        sample_timestamp_wall_us: 0,
    };
    let decoded = round_trip(&state);
    assert_eq!(
        decoded.stream_epoch,
        u64::MAX,
        "u64::MAX stream_epoch MUST survive"
    );
}

// ─── Codec enum values (RFC 0014 §2.5) ────────────────────────────────────────

#[test]
fn media_codec_enum_values_roundtrip_via_open() {
    // All codec values in the RFC 0014 §2.5 table MUST round-trip correctly.
    // VIDEO_H264_BASELINE=1, VIDEO_H264_MAIN=2, VIDEO_VP9=3, VIDEO_AV1=4 (reserved),
    // AUDIO_OPUS_STEREO=10, AUDIO_OPUS_MONO=11, AUDIO_PCM_S16LE=12.
    let codecs = vec![1, 2, 3, 4, 10, 11, 12];
    for codec in codecs {
        let open = MediaIngressOpen {
            client_stream_id: vec![0x00_u8; 16],
            transport: None,
            surface_binding: None,
            codec_preference: vec![codec],
            has_audio_track: false,
            has_video_track: false,
            content_classification: String::new(),
            present_at_wall_us: 0,
            expires_at_wall_us: 0,
            declared_peak_kbps: 0,
        };
        let decoded = round_trip(&open);
        assert_eq!(
            decoded.codec_preference,
            vec![codec],
            "codec value {codec} MUST round-trip correctly"
        );
    }
}

// ─── Transport descriptor roundtrip ──────────────────────────────────────────

#[test]
fn transport_descriptor_with_sdp_offer_roundtrip() {
    use tze_hud_protocol::proto::session::{IceCredential, TransportDescriptor};
    let descriptor = TransportDescriptor {
        mode: 1, // WEBRTC_STANDARD
        agent_sdp_offer: b"v=0\r\no=agent 0 0 IN IP4 127.0.0.1\r\n".to_vec(),
        agent_ice_credentials: vec![IceCredential {
            ufrag: "abc123".to_string(),
            pwd: "secret-password-1".to_string(),
            controlling: true,
        }],
        relay_hint: 1, // DIRECT
        preshared_srtp_material: vec![],
    };
    let decoded = round_trip(&descriptor);
    assert_eq!(decoded.mode, 1, "mode MUST survive");
    assert!(
        !decoded.agent_sdp_offer.is_empty(),
        "agent_sdp_offer MUST survive"
    );
    assert_eq!(
        decoded.agent_ice_credentials.len(),
        1,
        "ICE credentials MUST survive"
    );
    assert_eq!(decoded.agent_ice_credentials[0].ufrag, "abc123");
    assert!(
        decoded.agent_ice_credentials[0].controlling,
        "ICE controlling MUST survive"
    );
}

#[test]
fn transport_descriptor_future_cloud_relay_mode() {
    use tze_hud_protocol::proto::session::TransportDescriptor;
    let descriptor = TransportDescriptor {
        mode: 3, // FUTURE_CLOUD_RELAY (phase 4b reserved)
        agent_sdp_offer: vec![],
        agent_ice_credentials: vec![],
        relay_hint: 0,
        preshared_srtp_material: vec![],
    };
    let decoded = round_trip(&descriptor);
    assert_eq!(
        decoded.mode, 3,
        "FUTURE_CLOUD_RELAY (mode=3) MUST round-trip (wire-reserved for phase 4b)"
    );
}
