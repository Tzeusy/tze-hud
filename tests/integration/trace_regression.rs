//! # Trace Regression Tests
//!
//! Integration tests for the record/replay trace harness.
//!
//! These tests validate the spec §"Record/Replay Traces" requirement
//! (validation-framework/spec.md lines 283-295, v1-mandatory):
//!
//! - Trace capture API records mutation batches, input events, zone publishes,
//!   timing data, and agent events into structured traces.
//! - Deterministic replay verifies identical outcomes.
//! - Fuzzing-to-regression pipeline converts minimal reproducers into permanent
//!   regression tests.
//!
//! ## Why integration tests?
//!
//! Unit tests in `tze_hud_scene/src/trace.rs` and `tze_hud_scene/src/replay.rs`
//! cover the format and replay engine in isolation. These integration tests
//! exercise the full stack:
//! - `TraceRecorder` (runtime crate) captures events as they pass through pipeline.
//! - `TraceReplayer` (scene crate) replays them against a fresh scene graph.
//! - The fuzzing-to-regression helper builds traces from raw mutation sequences.
//!
//! ## Spec coverage
//!
//! | Scenario | Test |
//! |---|---|
//! | Trace capture + replay | `trace_capture_and_replay_end_to_end` |
//! | Identical outcome on replay | `replay_produces_same_scene_version` |
//! | Fuzzer reproducer → regression | `build_regression_trace_from_mutations` |
//! | JSON round-trip | `trace_json_round_trip_preserves_all_events` |
//! | Multi-event trace ordering | `multi_event_trace_replays_in_seq_order` |

use tze_hud_runtime::trace_capture::{TraceRecorder, build_regression_trace};
use tze_hud_scene::mutation::{MutationBatch, SceneMutation};
use tze_hud_scene::replay::{TraceReplayer, assert_trace_is_deterministic};
use tze_hud_scene::trace::{
    SceneTrace, TraceEventKind, TracedAgentEvent, TracedZonePublish,
};
use tze_hud_scene::types::{Capability, SceneId};
use tze_hud_scene::graph::SceneGraph;
use tze_hud_runtime::channels::{InputEvent, InputEventKind};

// ── Helpers ────────────────────────────────────────────────────────────────────

fn create_tab_batch(ns: &str, name: &str, order: u32) -> MutationBatch {
    MutationBatch {
        batch_id: SceneId::new(),
        agent_namespace: ns.into(),
        mutations: vec![SceneMutation::CreateTab {
            name: name.into(),
            display_order: order,
        }],
        timing_hints: None,
        lease_id: None,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// Scenario: Trace capture and replay — when a timing-sensitive bug is reproduced,
/// the developer MUST be able to capture a replay trace and replay it to reproduce
/// the exact same outcome.
#[test]
fn trace_capture_and_replay_end_to_end() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let recorder = TraceRecorder::start(&scene, "end-to-end");

    // Apply several batches to the live scene.
    for i in 0..3 {
        let batch = create_tab_batch("agent", &format!("Tab-{i}"), i);
        let result = scene.apply_batch(&batch);
        assert!(result.applied, "batch {i} should succeed");
        recorder.record_mutation_batch(&batch, result.applied, Some(scene.version));
    }

    // Record a frame boundary after each batch.
    for frame in 1..=3 {
        recorder.record_frame_boundary(frame);
    }

    let trace = recorder.finish();
    assert_eq!(trace.event_count(), 6); // 3 batches + 3 frame boundaries

    // Replay: must produce same scene version.
    let result = TraceReplayer::new().replay(&trace);
    assert_trace_is_deterministic(&result);
    assert_eq!(result.replayed, 3);
    assert_eq!(result.skipped, 3);
}

/// Scenario: Identical outcome on replay — when a trace is replayed against the
/// same scene state, the outcome MUST be identical to the original.
#[test]
fn replay_produces_same_scene_version() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let recorder = TraceRecorder::start(&scene, "version-check");

    // Apply 5 batches, each creating one tab.
    for i in 0u32..5 {
        let batch = create_tab_batch("agent", &format!("Tab-{i}"), i);
        let result = scene.apply_batch(&batch);
        recorder.record_mutation_batch(&batch, result.applied, Some(scene.version));
    }

    let expected_version = scene.version;
    let trace = recorder.finish();

    let result = TraceReplayer::new().replay(&trace);
    assert_trace_is_deterministic(&result);
    assert_eq!(
        result.final_version, expected_version,
        "replay final version must match original"
    );
}

/// Scenario: Fuzzing reproducer → regression test — when a fuzzer discovers a
/// crash or invariant violation, the reproducer MUST be convertible to a
/// permanent replay trace regression test.
#[test]
fn build_regression_trace_from_mutations() {
    // Simulate a fuzzer finding that applying duplicate display_order tabs
    // triggers a rejection. Build a trace from the minimal reproducer.
    let batches = vec![
        create_tab_batch("fuzz", "First", 0),
        create_tab_batch("fuzz", "Conflict", 0), // dup display_order — rejected
        create_tab_batch("fuzz", "Third", 1),     // should succeed
    ];

    let trace = build_regression_trace(&batches, "fuzz-dup-display-order");

    // The trace should contain exactly 3 events.
    assert_eq!(trace.event_count(), 3);

    // Verify rejection recording.
    let events: Vec<_> = trace.mutation_events().collect();
    assert_eq!(events.len(), 3);

    match &events[0].kind {
        TraceEventKind::MutationBatch { applied, .. } => assert!(*applied, "first must apply"),
        _ => panic!("expected MutationBatch"),
    }
    match &events[1].kind {
        TraceEventKind::MutationBatch { applied, .. } => assert!(!*applied, "dup must reject"),
        _ => panic!("expected MutationBatch"),
    }
    match &events[2].kind {
        TraceEventKind::MutationBatch { applied, .. } => assert!(*applied, "third must apply"),
        _ => panic!("expected MutationBatch"),
    }

    // The trace must replay deterministically.
    let result = TraceReplayer::new().replay(&trace);
    assert_trace_is_deterministic(&result);
}

/// JSON round-trip: all events survive serialization → deserialization unchanged.
#[test]
fn trace_json_round_trip_preserves_all_events() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let recorder = TraceRecorder::start(&scene, "round-trip");

    // Mix of event types.
    let batch = create_tab_batch("agent", "Main", 0);
    let result = scene.apply_batch(&batch);
    recorder.record_mutation_batch(&batch, result.applied, Some(scene.version));

    recorder.record_input_event(&InputEvent {
        timestamp_ns: 1_000_000,
        kind: InputEventKind::PointerMove { x: 50.0, y: 75.0 },
    });

    recorder.record_zone_publish(TracedZonePublish {
        zone_name: "subtitles".into(),
        agent_namespace: "llm".into(),
        expires_at_wall_us: Some(9_999_999),
        content_classification: Some("public".into()),
        merge_key: None,
    });

    recorder.record_agent_event(TracedAgentEvent::LeaseGranted {
        agent_namespace: "agent".into(),
        lease_id: SceneId::new(),
        duration_ms: 60_000,
    });

    recorder.record_clock_tick(1_735_689_600_000_000 + 1000);
    recorder.record_frame_boundary(1);

    let trace = recorder.finish();
    assert_eq!(trace.event_count(), 6);

    // Round-trip through JSON.
    let json = trace.to_json().expect("serialize trace");
    let restored = SceneTrace::from_json(&json).expect("deserialize trace");

    assert_eq!(restored.event_count(), 6);
    assert_eq!(restored.header.label, "round-trip");
    assert_eq!(restored.header.schema_version, 1);

    // Replay the restored trace — should be deterministic.
    let result = TraceReplayer::new().replay(&restored);
    assert_trace_is_deterministic(&result);
}

/// Multi-event trace: events stored in any order are replayed in seq order.
#[test]
fn multi_event_trace_replays_in_seq_order() {
    // Build a trace manually with events out of order.
    let initial_scene = SceneGraph::new(1920.0, 1080.0);
    let initial_json = serde_json::to_string(&initial_scene).unwrap();

    // Two CreateTab batches: display_order 0 then 1.
    let b0 = create_tab_batch("a", "Alpha", 0);
    let b1 = create_tab_batch("a", "Beta", 1);

    // Simulate applying b0 then b1.
    let mut sim = serde_json::from_str::<SceneGraph>(&initial_json).unwrap();
    let r0 = sim.apply_batch(&b0);
    let v0 = sim.version;
    let r1 = sim.apply_batch(&b1);
    let v1 = sim.version;
    assert!(r0.applied && r1.applied);

    // Build trace with events in reverse seq order.
    use tze_hud_scene::trace::{SceneTrace, TraceEvent, TraceHeader, TraceTimestamp};

    let trace = SceneTrace {
        header: TraceHeader {
            trace_id: SceneId::new(),
            label: "ordering-test".into(),
            started_at_wall_us: 0,
            initial_scene_json: initial_json,
            schema_version: 1,
        },
        events: vec![
            TraceEvent {
                seq: 1, // out of order
                timestamp: TraceTimestamp { wall_us: 200, mono_us: 200 },
                kind: TraceEventKind::MutationBatch {
                    batch: b1,
                    applied: true,
                    resulting_version: Some(v1),
                },
            },
            TraceEvent {
                seq: 0,
                timestamp: TraceTimestamp { wall_us: 100, mono_us: 100 },
                kind: TraceEventKind::MutationBatch {
                    batch: b0,
                    applied: true,
                    resulting_version: Some(v0),
                },
            },
        ],
    };

    // Replay must sort by seq and succeed.
    let result = TraceReplayer::new().replay(&trace);
    assert_trace_is_deterministic(&result);
    assert_eq!(result.replayed, 2);
}

/// Records a trace with a lease-bearing CreateTile batch and replays it.
#[test]
fn trace_with_lease_and_create_tile_replays() {
    let mut scene = SceneGraph::new(1920.0, 1080.0);

    // Grant a lease and create a tab before starting the trace.
    let lease_id = scene.grant_lease("agent-a", 60_000, vec![Capability::CreateTile]);
    let tab_id = scene.create_tab("Main", 0).unwrap();

    // Start recording with the current (non-empty) scene state.
    let recorder = TraceRecorder::start(&scene, "lease-tile");

    let tile_batch = MutationBatch {
        batch_id: SceneId::new(),
        agent_namespace: "agent-a".into(),
        mutations: vec![SceneMutation::CreateTile {
            tab_id,
            namespace: "agent-a".into(),
            lease_id,
            bounds: tze_hud_scene::types::Rect::new(0.0, 0.0, 100.0, 100.0),
            z_order: 1,
        }],
        timing_hints: None,
        lease_id: Some(lease_id),
    };

    let result = scene.apply_batch(&tile_batch);
    assert!(result.applied, "CreateTile should succeed");
    recorder.record_mutation_batch(&tile_batch, result.applied, Some(scene.version));

    let trace = recorder.finish();
    let replay_result = TraceReplayer::new().replay(&trace);
    assert_trace_is_deterministic(&replay_result);
    assert_eq!(replay_result.replayed, 1);
}
