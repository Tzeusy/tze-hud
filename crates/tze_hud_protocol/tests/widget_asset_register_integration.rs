//! Widget asset register/session envelope integration tests.
//!
//! Covers ClientMessage field 34 + ServerMessage field 48 wiring and stable
//! error-code round-tripping for WidgetAssetRegister semantics.

use prost::Message;
use tze_hud_protocol::proto::session::client_message::Payload as ClientPayload;
use tze_hud_protocol::proto::session::server_message::Payload as ServerPayload;
use tze_hud_protocol::proto::session::{
    ClientMessage, ServerMessage, WidgetAssetRegister, WidgetAssetRegisterResult,
};

fn round_trip<T: Message + Default>(msg: &T) -> T {
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode must succeed");
    T::decode(buf.as_slice()).expect("decode must succeed")
}

#[test]
fn widget_asset_register_roundtrip_preserves_fields() {
    let msg = WidgetAssetRegister {
        widget_type_id: "gauge".to_string(),
        svg_filename: "fill.svg".to_string(),
        content_hash_blake3: vec![0xAA; 32],
        transport_crc32c: 0x1122_3344,
        total_size_bytes: 42,
        inline_svg_bytes: b"<svg></svg>".to_vec(),
        metadata_only_preflight: false,
    };

    let decoded = round_trip(&msg);
    assert_eq!(decoded.widget_type_id, "gauge");
    assert_eq!(decoded.svg_filename, "fill.svg");
    assert_eq!(decoded.content_hash_blake3.len(), 32);
    assert_eq!(decoded.transport_crc32c, 0x1122_3344);
    assert_eq!(decoded.total_size_bytes, 42);
    assert_eq!(decoded.inline_svg_bytes, b"<svg></svg>".to_vec());
    assert!(!decoded.metadata_only_preflight);
}

#[test]
fn widget_asset_register_result_roundtrip_preserves_fields() {
    let msg = WidgetAssetRegisterResult {
        request_sequence: 88,
        accepted: true,
        widget_type_id: "gauge".to_string(),
        svg_filename: "fill.svg".to_string(),
        asset_handle: "widget-svg:abc".to_string(),
        was_deduplicated: true,
        error_code: String::new(),
        error_message: String::new(),
    };

    let decoded = round_trip(&msg);
    assert_eq!(decoded.request_sequence, 88);
    assert!(decoded.accepted);
    assert_eq!(decoded.widget_type_id, "gauge");
    assert_eq!(decoded.svg_filename, "fill.svg");
    assert_eq!(decoded.asset_handle, "widget-svg:abc");
    assert!(decoded.was_deduplicated);
}

#[test]
fn client_message_wraps_widget_asset_register_field_34() {
    let msg = ClientMessage {
        sequence: 10,
        timestamp_wall_us: 123,
        payload: Some(ClientPayload::WidgetAssetRegister(WidgetAssetRegister {
            widget_type_id: "gauge".to_string(),
            svg_filename: "fill.svg".to_string(),
            content_hash_blake3: vec![0x11; 32],
            transport_crc32c: 0,
            total_size_bytes: 11,
            inline_svg_bytes: b"<svg></svg>".to_vec(),
            metadata_only_preflight: false,
        })),
    };

    match round_trip(&msg).payload {
        Some(ClientPayload::WidgetAssetRegister(v)) => {
            assert_eq!(v.widget_type_id, "gauge");
            assert_eq!(v.svg_filename, "fill.svg");
        }
        other => panic!("expected WidgetAssetRegister payload, got: {other:?}"),
    }
}

#[test]
fn server_message_wraps_widget_asset_register_result_field_48() {
    let msg = ServerMessage {
        sequence: 11,
        timestamp_wall_us: 456,
        payload: Some(ServerPayload::WidgetAssetRegisterResult(
            WidgetAssetRegisterResult {
                request_sequence: 10,
                accepted: false,
                widget_type_id: "gauge".to_string(),
                svg_filename: "fill.svg".to_string(),
                asset_handle: String::new(),
                was_deduplicated: false,
                error_code: "WIDGET_ASSET_HASH_MISMATCH".to_string(),
                error_message: "mismatch".to_string(),
            },
        )),
    };

    match round_trip(&msg).payload {
        Some(ServerPayload::WidgetAssetRegisterResult(v)) => {
            assert!(!v.accepted);
            assert_eq!(v.error_code, "WIDGET_ASSET_HASH_MISMATCH");
        }
        other => panic!("expected WidgetAssetRegisterResult payload, got: {other:?}"),
    }
}

#[test]
fn widget_asset_register_error_codes_roundtrip() {
    let codes = [
        "WIDGET_ASSET_CAPABILITY_MISSING",
        "WIDGET_ASSET_HASH_MISMATCH",
        "WIDGET_ASSET_CHECKSUM_MISMATCH",
        "WIDGET_ASSET_INVALID_SVG",
        "WIDGET_ASSET_BUDGET_EXCEEDED",
        "WIDGET_ASSET_STORE_IO_ERROR",
        "WIDGET_ASSET_TYPE_INVALID",
    ];

    for code in codes {
        let msg = WidgetAssetRegisterResult {
            request_sequence: 1,
            accepted: false,
            widget_type_id: "gauge".to_string(),
            svg_filename: "fill.svg".to_string(),
            asset_handle: String::new(),
            was_deduplicated: false,
            error_code: code.to_string(),
            error_message: format!("error: {code}"),
        };
        let decoded = round_trip(&msg);
        assert_eq!(decoded.error_code, code);
    }
}
