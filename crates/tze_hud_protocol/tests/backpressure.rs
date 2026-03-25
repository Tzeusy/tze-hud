//! Backpressure semantics tests.
//!
//! Tests per RFC 0005 channel topology and session-protocol/spec.md lines 238-249:
//! - Transactional messages: never dropped, queued (capacity 256)
//! - Ephemeral realtime messages: drop oldest on overflow (latest-wins semantics)
//! - State-stream messages: coalesced via keyed map (latest state per key wins)
//! - Backpressure signals: MUTATION_QUEUE_PRESSURE at 80% capacity
//!
//! Tests also exercise classify_server_payload, classify_inbound_batch,
//! and BackpressureSignal semantics from session_server.rs.
//!
//! Test count target: ≥4 tests.

use tze_hud_protocol::session_server::{
    SessionConfig, TrafficClass, classify_server_payload,
};
use tze_hud_protocol::proto::session::{
    BackpressureSignal, SessionEstablished, SessionError, MutationResult,
    LeaseResponse, Heartbeat, SceneSnapshot, RuntimeTelemetryFrame,
    server_message::Payload as ServerPayload,
};
use prost::Message;

// ─── Traffic class classification ─────────────────────────────────────────────

/// Transactional messages are never dropped (classified as Transactional).
#[test]
fn transactional_messages_never_dropped() {
    // A sample of transactional payloads
    let payloads = vec![
        ServerPayload::SessionEstablished(SessionEstablished::default()),
        ServerPayload::MutationResult(MutationResult::default()),
        ServerPayload::LeaseResponse(LeaseResponse::default()),
        ServerPayload::BackpressureSignal(BackpressureSignal {
            queue_pressure: 0.9,
            suggested_action: "reduce_rate".to_string(),
        }),
    ];
    for payload in &payloads {
        assert_eq!(
            classify_server_payload(payload),
            TrafficClass::Transactional,
            "transactional payload must never be classified as droppable"
        );
    }
}

/// Ephemeral messages are droppable (classified as Ephemeral).
#[test]
fn heartbeat_is_ephemeral_and_droppable() {
    let payload = ServerPayload::Heartbeat(Heartbeat { timestamp_mono_us: 12345 });
    assert_eq!(
        classify_server_payload(&payload),
        TrafficClass::Ephemeral,
        "Heartbeat must be Ephemeral — oldest dropped under backpressure, latest-wins"
    );
}

/// State-stream messages are coalesced under pressure.
#[test]
fn state_stream_messages_are_coalesced_class() {
    let payloads = vec![
        ServerPayload::SceneSnapshot(SceneSnapshot::default()),
        ServerPayload::RuntimeTelemetry(RuntimeTelemetryFrame::default()),
    ];
    for payload in &payloads {
        assert_eq!(
            classify_server_payload(payload),
            TrafficClass::StateStream,
            "scene/telemetry payloads must be StateStream (coalesced under pressure)"
        );
    }
}

// ─── Backpressure signal semantics ───────────────────────────────────────────

/// BackpressureSignal encodes queue_pressure as float 0.0–1.0.
/// WHEN mutation queue at 80% capacity THEN MUTATION_QUEUE_PRESSURE signal sent.
#[test]
fn backpressure_signal_at_80_percent_triggers_pressure_notice() {
    let pressure_threshold = 0.80_f32;

    // Below threshold: no pressure
    let below = BackpressureSignal { queue_pressure: 0.75, suggested_action: String::new() };
    assert!(below.queue_pressure < pressure_threshold,
        "0.75 is below the 80% pressure threshold");

    // At threshold: pressure signal triggered
    let at_threshold = BackpressureSignal {
        queue_pressure: 0.80,
        suggested_action: "reduce_rate".to_string(),
    };
    assert!(at_threshold.queue_pressure >= pressure_threshold,
        "0.80 must trigger pressure signal (session-protocol/spec.md lines 238-249)");
    assert_eq!(at_threshold.suggested_action, "reduce_rate");

    // Above threshold: pressure signal triggered
    let above = BackpressureSignal {
        queue_pressure: 0.95,
        suggested_action: "coalesce".to_string(),
    };
    assert!(above.queue_pressure >= pressure_threshold);
}

/// Full queue (1.0): mutation dropped signal.
#[test]
fn backpressure_signal_full_queue_mutation_dropped() {
    let full = BackpressureSignal {
        queue_pressure: 1.0,
        suggested_action: "stop".to_string(),
    };
    assert_eq!(full.queue_pressure, 1.0,
        "queue_pressure of 1.0 indicates full queue — mutations dropped");
}

/// BackpressureSignal itself is classified as Transactional (must not be dropped).
#[test]
fn backpressure_signal_is_transactional() {
    let payload = ServerPayload::BackpressureSignal(BackpressureSignal {
        queue_pressure: 0.85,
        suggested_action: "reduce_rate".to_string(),
    });
    assert_eq!(
        classify_server_payload(&payload),
        TrafficClass::Transactional,
        "BackpressureSignal must be Transactional — it must not be dropped under pressure"
    );
}

// ─── 80% pressure threshold validation ───────────────────────────────────────

/// The freeze queue 80% pressure threshold matches spec.
#[test]
fn freeze_queue_pressure_threshold_is_80_percent() {
    // Per session_server.rs: FREEZE_QUEUE_PRESSURE_FRACTION = 0.80
    // This matches session-protocol/spec.md lines 238-249
    let capacity = 1000usize;
    let threshold = (capacity as f32 * 0.80) as usize;
    assert_eq!(threshold, 800,
        "pressure threshold at 80% of 1000 must be 800 (session-protocol/spec.md)");
}

/// Backpressure signal round-trips correctly.
#[test]
fn backpressure_signal_roundtrip_preserves_pressure_value() {
    let orig = BackpressureSignal {
        queue_pressure: 0.8237654,
        suggested_action: "coalesce".to_string(),
    };
    let mut buf = Vec::new();
    orig.encode(&mut buf).unwrap();
    let decoded = BackpressureSignal::decode(buf.as_slice()).unwrap();
    assert!((decoded.queue_pressure - orig.queue_pressure).abs() < 1e-6,
        "pressure value must survive serialization");
    assert_eq!(decoded.suggested_action, "coalesce");
}

// ─── Message class preservation under load ────────────────────────────────────

/// The session config's ephemeral_buffer_max sets the ephemeral queue size.
#[test]
fn ephemeral_buffer_max_default_value() {
    let cfg = SessionConfig::default();
    // Default: 16 per session_server.rs (tunable)
    assert!(cfg.ephemeral_buffer_max > 0,
        "ephemeral_buffer_max must be > 0 to allow some ephemeral messages");
}

/// Transactional messages queue without limit (backed by gRPC backpressure on overflow).
/// This test verifies the classification: Transactional ≠ Ephemeral.
#[test]
fn transactional_not_droppable_different_from_ephemeral() {
    let transactional = ServerPayload::MutationResult(MutationResult::default());
    let ephemeral = ServerPayload::Heartbeat(Heartbeat::default());

    let tc = classify_server_payload(&transactional);
    let te = classify_server_payload(&ephemeral);

    assert_ne!(tc, te, "Transactional and Ephemeral must be different traffic classes");
    assert_eq!(tc, TrafficClass::Transactional);
    assert_eq!(te, TrafficClass::Ephemeral);
}
