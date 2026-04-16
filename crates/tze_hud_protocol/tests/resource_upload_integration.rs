//! Resident scene-resource upload/session envelope integration tests.
//!
//! Covers ClientMessage fields 36/37/38 and ServerMessage fields 41/42/49,
//! plus correlation field round-tripping for request_sequence/upload_id.

use prost::Message;
use tze_hud_protocol::proto::ResourceIdProto;
use tze_hud_protocol::proto::session::client_message::Payload as ClientPayload;
use tze_hud_protocol::proto::session::server_message::Payload as ServerPayload;
use tze_hud_protocol::proto::session::{
    ClientMessage, ResourceErrorResponse, ResourceMetadata, ResourceStored, ResourceUploadAccepted,
    ResourceUploadChunk, ResourceUploadComplete, ResourceUploadStart, ServerMessage,
};

fn round_trip<T: Message + Default>(msg: &T) -> T {
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode must succeed");
    T::decode(buf.as_slice()).expect("decode must succeed")
}

#[test]
fn resource_upload_start_roundtrip_preserves_fields() {
    let msg = ResourceUploadStart {
        expected_hash: vec![0xAB; 32],
        resource_type: 2, // IMAGE_PNG
        total_size_bytes: 128,
        metadata: Some(ResourceMetadata {
            width: 32,
            height: 32,
            font_family: String::new(),
            font_style: String::new(),
        }),
        inline_data: vec![1, 2, 3, 4],
    };

    let decoded = round_trip(&msg);
    assert_eq!(decoded.expected_hash.len(), 32);
    assert_eq!(decoded.resource_type, 2);
    assert_eq!(decoded.total_size_bytes, 128);
    let meta = decoded.metadata.expect("metadata must roundtrip");
    assert_eq!(meta.width, 32);
    assert_eq!(meta.height, 32);
    assert_eq!(decoded.inline_data, vec![1, 2, 3, 4]);
}

#[test]
fn client_message_wraps_resource_upload_family() {
    let start = ClientMessage {
        sequence: 10,
        timestamp_wall_us: 111,
        payload: Some(ClientPayload::ResourceUploadStart(ResourceUploadStart {
            expected_hash: vec![0x11; 32],
            resource_type: 2,
            total_size_bytes: 4,
            metadata: Some(ResourceMetadata {
                width: 1,
                height: 1,
                font_family: String::new(),
                font_style: String::new(),
            }),
            inline_data: vec![0, 0, 0, 0],
        })),
    };
    match round_trip(&start).payload {
        Some(ClientPayload::ResourceUploadStart(v)) => {
            assert_eq!(v.total_size_bytes, 4);
            assert_eq!(v.resource_type, 2);
        }
        other => panic!("expected ResourceUploadStart payload, got: {other:?}"),
    }

    let chunk = ClientMessage {
        sequence: 11,
        timestamp_wall_us: 222,
        payload: Some(ClientPayload::ResourceUploadChunk(ResourceUploadChunk {
            upload_id: vec![0x22; 16],
            chunk_index: 7,
            data: vec![9, 8, 7],
        })),
    };
    match round_trip(&chunk).payload {
        Some(ClientPayload::ResourceUploadChunk(v)) => {
            assert_eq!(v.upload_id.len(), 16);
            assert_eq!(v.chunk_index, 7);
            assert_eq!(v.data, vec![9, 8, 7]);
        }
        other => panic!("expected ResourceUploadChunk payload, got: {other:?}"),
    }

    let complete = ClientMessage {
        sequence: 12,
        timestamp_wall_us: 333,
        payload: Some(ClientPayload::ResourceUploadComplete(
            ResourceUploadComplete {
                upload_id: vec![0x33; 16],
            },
        )),
    };
    match round_trip(&complete).payload {
        Some(ClientPayload::ResourceUploadComplete(v)) => {
            assert_eq!(v.upload_id, vec![0x33; 16]);
        }
        other => panic!("expected ResourceUploadComplete payload, got: {other:?}"),
    }
}

#[test]
fn server_message_wraps_resource_upload_responses() {
    let accepted = ServerMessage {
        sequence: 20,
        timestamp_wall_us: 444,
        payload: Some(ServerPayload::ResourceUploadAccepted(
            ResourceUploadAccepted {
                request_sequence: 10,
                upload_id: vec![0x44; 16],
            },
        )),
    };
    match round_trip(&accepted).payload {
        Some(ServerPayload::ResourceUploadAccepted(v)) => {
            assert_eq!(v.request_sequence, 10);
            assert_eq!(v.upload_id, vec![0x44; 16]);
        }
        other => panic!("expected ResourceUploadAccepted payload, got: {other:?}"),
    }

    let stored = ServerMessage {
        sequence: 21,
        timestamp_wall_us: 555,
        payload: Some(ServerPayload::ResourceStored(ResourceStored {
            request_sequence: 11,
            resource_id: Some(ResourceIdProto {
                bytes: vec![0x55; 32],
            }),
            was_deduplicated: false,
            stored_bytes: 4,
            decoded_bytes: 4,
            metadata: Some(ResourceMetadata {
                width: 1,
                height: 1,
                font_family: String::new(),
                font_style: String::new(),
            }),
            upload_id: vec![0x66; 16],
        })),
    };
    match round_trip(&stored).payload {
        Some(ServerPayload::ResourceStored(v)) => {
            assert_eq!(v.request_sequence, 11);
            assert_eq!(v.upload_id, vec![0x66; 16]);
            assert_eq!(
                v.resource_id
                    .expect("resource_id must be present")
                    .bytes
                    .len(),
                32
            );
        }
        other => panic!("expected ResourceStored payload, got: {other:?}"),
    }

    let error = ServerMessage {
        sequence: 22,
        timestamp_wall_us: 666,
        payload: Some(ServerPayload::ResourceErrorResponse(
            ResourceErrorResponse {
                request_sequence: 12,
                error_code: 8, // RESOURCE_TOO_MANY_UPLOADS
                message: "too many uploads".to_string(),
                context: "{\"active_uploads\":4}".to_string(),
                hint: "{\"max\":4}".to_string(),
                upload_id: vec![0x77; 16],
            },
        )),
    };
    match round_trip(&error).payload {
        Some(ServerPayload::ResourceErrorResponse(v)) => {
            assert_eq!(v.request_sequence, 12);
            assert_eq!(v.error_code, 8);
            assert_eq!(v.upload_id, vec![0x77; 16]);
        }
        other => panic!("expected ResourceErrorResponse payload, got: {other:?}"),
    }
}
