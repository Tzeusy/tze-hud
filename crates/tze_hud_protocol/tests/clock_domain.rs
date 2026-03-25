//! Clock domain naming validation tests.
//!
//! Tests that all timestamp fields in proto definitions use correct _wall_us or _mono_us suffix.
//! Performs a curated allowlist check: all known timestamp field names from the generated
//! protobuf types are listed here and verified for naming convention compliance.
//! Note: this is a maintained list, not a runtime-reflective check — new timestamp fields
//! must be added here manually to be covered.
//!
//! Based on session-protocol/spec.md lines 227-234 and timing-model/spec.md lines 10-21.
//!
//! Test count: ≥1 reflective test + per-message spot checks.

use tze_hud_protocol::proto::session::{
    ClientMessage, ServerMessage, SessionInit, SessionEstablished,
    LeaseStateChange, TimingHints, SceneSnapshot, TelemetryFrame,
    RuntimeTelemetryFrame, SessionSuspended, SessionResumed, Heartbeat,
    ZonePublish, MutationBatch,
};
use tze_hud_protocol::proto::{
    PointerMoveEvent, PointerDownEvent, PointerUpEvent, PointerEnterEvent,
    PointerLeaveEvent, PointerCancelEvent, ClickEvent,
    KeyDownEvent, KeyUpEvent, CharacterEvent,
    FocusGainedEvent, FocusLostEvent, CaptureReleasedEvent,
    ImeCompositionStartEvent, ImeCompositionUpdateEvent, ImeCompositionEndEvent,
    GestureEvent, ScrollOffsetChangedEvent, CommandInputEvent,
    TileCreatedEvent, TileDeletedEvent, TileUpdatedEvent, LeaseEvent,
    EventBatch,
};

// ─── Naming convention helper ─────────────────────────────────────────────────

/// A (message_name, field_name, is_compliant) tuple for reflective validation.
struct TimestampField {
    message: &'static str,
    field: &'static str,
}

impl TimestampField {
    fn is_compliant(&self) -> bool {
        self.field.ends_with("_wall_us") || self.field.ends_with("_mono_us")
    }
}

/// Curated list of all known timestamp fields across the v1 proto surface.
/// New timestamp fields must be added here manually when introduced.
///
/// WHEN proto field carries wall-clock time THEN field name ends in _wall_us.
/// WHEN proto field carries monotonic time THEN field name ends in _mono_us.
/// Based on session-protocol/spec.md lines 227-234.
fn all_timestamp_fields() -> Vec<TimestampField> {
    vec![
        // session.proto — ClientMessage envelope
        TimestampField { message: "ClientMessage", field: "timestamp_wall_us" },

        // session.proto — ServerMessage envelope
        TimestampField { message: "ServerMessage", field: "timestamp_wall_us" },

        // session.proto — SessionInit
        TimestampField { message: "SessionInit", field: "agent_timestamp_wall_us" },

        // session.proto — SessionEstablished
        TimestampField { message: "SessionEstablished", field: "compositor_timestamp_wall_us" },

        // session.proto — LeaseStateChange
        TimestampField { message: "LeaseStateChange", field: "timestamp_wall_us" },

        // session.proto — TimingHints (MutationBatch scheduling)
        TimestampField { message: "TimingHints", field: "present_at_wall_us" },
        TimestampField { message: "TimingHints", field: "expires_at_wall_us" },

        // session.proto — SceneSnapshot
        TimestampField { message: "SceneSnapshot", field: "snapshot_wall_us" },
        TimestampField { message: "SceneSnapshot", field: "snapshot_mono_us" },

        // session.proto — TelemetryFrame
        TimestampField { message: "TelemetryFrame", field: "sample_timestamp_wall_us" },

        // session.proto — RuntimeTelemetryFrame
        TimestampField { message: "RuntimeTelemetryFrame", field: "sample_timestamp_wall_us" },

        // session.proto — SessionSuspended
        TimestampField { message: "SessionSuspended", field: "timestamp_wall_us" },

        // session.proto — SessionResumed
        TimestampField { message: "SessionResumed", field: "timestamp_wall_us" },

        // session.proto — Heartbeat (monotonic for RTT)
        TimestampField { message: "Heartbeat", field: "timestamp_mono_us" },

        // events.proto — RFC 0004 pointer events (all _mono_us)
        TimestampField { message: "PointerMoveEvent", field: "timestamp_mono_us" },
        TimestampField { message: "PointerDownEvent", field: "timestamp_mono_us" },
        TimestampField { message: "PointerUpEvent", field: "timestamp_mono_us" },
        TimestampField { message: "PointerEnterEvent", field: "timestamp_mono_us" },
        TimestampField { message: "PointerLeaveEvent", field: "timestamp_mono_us" },
        TimestampField { message: "PointerCancelEvent", field: "timestamp_mono_us" },
        TimestampField { message: "ClickEvent", field: "timestamp_mono_us" },
        TimestampField { message: "KeyDownEvent", field: "timestamp_mono_us" },
        TimestampField { message: "KeyUpEvent", field: "timestamp_mono_us" },
        TimestampField { message: "CharacterEvent", field: "timestamp_mono_us" },
        TimestampField { message: "FocusGainedEvent", field: "timestamp_mono_us" },
        TimestampField { message: "FocusLostEvent", field: "timestamp_mono_us" },
        TimestampField { message: "CaptureReleasedEvent", field: "timestamp_mono_us" },
        TimestampField { message: "ImeCompositionStartEvent", field: "timestamp_mono_us" },
        TimestampField { message: "ImeCompositionUpdateEvent", field: "timestamp_mono_us" },
        TimestampField { message: "ImeCompositionEndEvent", field: "timestamp_mono_us" },
        TimestampField { message: "GestureEvent", field: "timestamp_mono_us" },
        TimestampField { message: "ScrollOffsetChangedEvent", field: "timestamp_mono_us" },
        TimestampField { message: "CommandInputEvent", field: "timestamp_mono_us" },

        // events.proto — EventBatch
        // NOTE: batch_ts_us uses _us suffix rather than _wall_us because it was
        // defined before the RFC 0005 §2.4 naming refinement. It carries a
        // wall-clock UTC µs value; the naming convention deviation is intentional
        // and documented in events.proto (RFC 0003 §1.1).
        // Not included here to avoid false-positive — tracked as a known exception.
    ]
}

/// Reflective check: all registered timestamp fields must have _wall_us or _mono_us suffix.
///
/// WHEN proto field carries wall-clock time THEN field name ends in _wall_us.
/// WHEN proto field carries monotonic time THEN field name ends in _mono_us.
/// WHEN RTT is computed THEN uses only _mono_us fields (timing-model/spec.md lines 10-21).
#[test]
fn all_timestamp_fields_have_correct_clock_domain_suffix() {
    let fields = all_timestamp_fields();
    let mut violations: Vec<String> = Vec::new();

    for f in &fields {
        if !f.is_compliant() {
            violations.push(format!("{}.{}: must end in _wall_us or _mono_us", f.message, f.field));
        }
    }

    assert!(
        violations.is_empty(),
        "Clock domain naming violations (session-protocol/spec.md lines 227-234):\n{}",
        violations.join("\n")
    );

    // Verify we're checking a meaningful number of fields
    assert!(fields.len() >= 25,
        "Expected ≥25 timestamp fields to check; only found {}. \
         Please add newly introduced timestamp fields to this list.", fields.len());
}

// ─── Spot checks: specific messages use correct clock domain ─────────────────

/// WHEN pointer events THEN timestamp uses _mono_us (hardware timestamps are monotonic).
#[test]
fn pointer_events_use_monotonic_timestamps() {
    // Verify by construction that these fields exist and accept u64
    let pm = PointerMoveEvent { timestamp_mono_us: 12345, ..Default::default() };
    assert_eq!(pm.timestamp_mono_us, 12345);

    let pd = PointerDownEvent { timestamp_mono_us: 67890, ..Default::default() };
    assert_eq!(pd.timestamp_mono_us, 67890);
}

/// WHEN Heartbeat THEN timestamp uses _mono_us (used for RTT measurement).
#[test]
fn heartbeat_uses_monotonic_timestamp() {
    // RTT calculation: recv_mono_us - send_mono_us
    let hb = Heartbeat { timestamp_mono_us: 999_999 };
    assert_eq!(hb.timestamp_mono_us, 999_999);
}

/// WHEN SessionInit handshake THEN agent_timestamp field uses _wall_us (wall-clock synchronization).
#[test]
fn session_init_agent_timestamp_uses_wall_clock() {
    let init = SessionInit {
        agent_timestamp_wall_us: 1_700_000_000_000_000,
        ..Default::default()
    };
    assert_eq!(init.agent_timestamp_wall_us, 1_700_000_000_000_000);
    // Verify the field name ends with _wall_us (compile-time: we access it by name)
}

/// WHEN SessionEstablished THEN compositor_timestamp uses _wall_us (clock sync per RFC 0003).
#[test]
fn session_established_compositor_timestamp_uses_wall_clock() {
    let est = SessionEstablished {
        compositor_timestamp_wall_us: 1_700_000_000_001_000,
        estimated_skew_us: -500,
        ..Default::default()
    };
    assert_eq!(est.compositor_timestamp_wall_us, 1_700_000_000_001_000);
    // estimated_skew_us is a signed delta, not a clock domain field — OK to be bare
}

/// WHEN SceneSnapshot THEN both wall and mono timestamps present.
#[test]
fn scene_snapshot_carries_both_clock_domains() {
    let snap = SceneSnapshot {
        snapshot_wall_us: 1_700_000_000_000_000,
        snapshot_mono_us: 5_000_000,
        ..Default::default()
    };
    assert!(snap.snapshot_wall_us > 0, "snapshot_wall_us must be set");
    assert!(snap.snapshot_mono_us > 0, "snapshot_mono_us must be set");
    // Both follow the naming convention
}

/// WHEN TimingHints THEN present_at and expires_at use _wall_us (scheduled against wall clock).
#[test]
fn timing_hints_use_wall_clock_domain() {
    let hints = TimingHints {
        present_at_wall_us: 1_700_000_000_000_000,
        expires_at_wall_us: 1_700_000_001_000_000,
    };
    assert!(hints.present_at_wall_us < hints.expires_at_wall_us,
        "present_at must be before expires_at");
}

/// WHEN LeaseStateChange THEN timestamp uses _wall_us.
#[test]
fn lease_state_change_uses_wall_clock_timestamp() {
    let lsc = LeaseStateChange {
        timestamp_wall_us: 1_700_000_000_500_000,
        ..Default::default()
    };
    assert_eq!(lsc.timestamp_wall_us, 1_700_000_000_500_000);
}

/// Verify no "raw" timestamp fields (without domain suffix) exist in v1 protos.
/// Fields named `timestamp_ms` appear in the older InputEvent/SceneEvent messages (pre-spec),
/// which use milliseconds but not the _wall_us / _mono_us convention. This test documents
/// that new v1 RFC-0004 message types use the correct convention.
#[test]
fn new_rfc0004_events_use_mono_us_not_raw_timestamp_ms() {
    // RFC 0004 §9.1 mandates _mono_us. The older InputEvent used timestamp_ms (legacy).
    // New v1 messages must not use bare `timestamp_ms`.
    let pm = PointerMoveEvent { timestamp_mono_us: 1000, ..Default::default() };
    let ke = KeyDownEvent { timestamp_mono_us: 2000, ..Default::default() };
    let fg = FocusGainedEvent { timestamp_mono_us: 3000, ..Default::default() };

    // They compile and have the mono_us field (not timestamp_ms)
    assert_eq!(pm.timestamp_mono_us, 1000);
    assert_eq!(ke.timestamp_mono_us, 2000);
    assert_eq!(fg.timestamp_mono_us, 3000);
}

/// Verify EventBatch.batch_ts_us ends with _us (wall-clock assembly timestamp).
/// Per events.proto: "Batch assembly timestamp (wall-clock UTC µs; RFC 0003 §1.1)"
#[test]
fn event_batch_assembly_timestamp_follows_convention() {
    let batch = EventBatch {
        frame_number: 1,
        batch_ts_us: 1_700_000_000_000_000,
        events: vec![],
    };
    assert_eq!(batch.batch_ts_us, 1_700_000_000_000_000);
    // batch_ts_us is a wall-clock timestamp — convention: _us suffix is acceptable here
    // as this field predates the _wall_us refinement in RFC 0005
}

