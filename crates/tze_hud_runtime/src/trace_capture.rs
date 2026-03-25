//! # Trace Capture Integration
//!
//! Runtime-side integration for recording [`tze_hud_scene::trace::SceneTrace`]
//! traces. Wraps mutation batches, input events, zone publishes, and agent events
//! with wall-clock and monotonic timestamps as they pass through the pipeline.
//!
//! ## Spec alignment
//!
//! Implements the capture side of the `validation-framework/spec.md`
//! §"Record/Replay Traces" requirement (lines 283-295, v1-mandatory):
//!
//! > The runtime SHALL support recording sequences of scene mutations, agent
//! > events, input events, zone publishes, and timing data as structured traces.
//!
//! ## Design
//!
//! [`TraceRecorder`] is a lightweight, opt-in recorder. It only records when
//! the caller explicitly creates one via [`TraceRecorder::start`] and passes
//! it through the pipeline; otherwise no trace data is captured. When active,
//! each `record_*` call appends a [`TraceEvent`] to an in-memory buffer.
//!
//! The caller is responsible for:
//! 1. Creating a [`TraceRecorder`] via [`TraceRecorder::start`].
//! 2. Passing it to the pipeline stages that emit events.
//! 3. Calling [`TraceRecorder::finish`] to obtain the completed [`SceneTrace`].
//! 4. Optionally serializing the trace with [`SceneTrace::to_json`] and saving
//!    it as a regression test.
//!
//! ## Thread safety
//!
//! [`TraceRecorder`] is `Send + Sync`. It uses a `Mutex`-protected event buffer
//! so that multiple pipeline threads (input drain on main thread, mutation intake
//! on compositor thread, network events on tokio threads) can all append events
//! concurrently.
//!
//! ## Fuzzing-to-regression pipeline
//!
//! When a fuzzer finds a crash or invariant violation:
//!
//! 1. The fuzzer's harness holds a `TraceRecorder` (or uses the proptest-based
//!    helper in `tests/regression/`).
//! 2. On crash/violation, call `recorder.finish()` to get the `SceneTrace`.
//! 3. Save the trace with a descriptive label via `trace.to_json_pretty()`.
//! 4. Add a `#[test]` in `tests/regression/traces/` that loads the trace file
//!    and calls `assert_trace_is_deterministic`.
//!
//! See `tests/regression/` for example regression tests built this way.

use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tze_hud_scene::graph::SceneGraph;
use tze_hud_scene::mutation::MutationBatch;
use tze_hud_scene::trace::{
    SceneTrace, TraceEvent, TraceEventKind, TraceHeader, TraceTimestamp,
    TracedAgentEvent, TracedInputEvent, TracedZonePublish,
};
use tze_hud_scene::types::SceneId;

use crate::channels::InputEvent as RuntimeInputEvent;
use crate::channels::InputEventKind;

// ─── TraceRecorder ────────────────────────────────────────────────────────────

/// Records runtime events into an in-memory [`SceneTrace`].
///
/// # Usage
///
/// ```rust,ignore
/// use tze_hud_runtime::trace_capture::TraceRecorder;
///
/// // Start recording against the current scene graph state.
/// let recorder = TraceRecorder::start(&scene, "timing-bug-repro");
///
/// // ... run the pipeline ...
/// recorder.record_mutation_batch(&batch, applied, resulting_version);
/// recorder.record_input_event(&ev);
///
/// // Finish and save.
/// let trace = recorder.finish();
/// std::fs::write("bug.trace.json", trace.to_json_pretty().unwrap()).unwrap();
/// ```
#[derive(Clone)]
pub struct TraceRecorder {
    inner: Arc<TraceRecorderInner>,
}

struct TraceRecorderInner {
    /// Monotonic baseline for computing `mono_us` offsets.
    mono_origin: Instant,
    /// Next sequence number to assign.
    next_seq: Mutex<u64>,
    /// Recorded events (append-only).
    events: Mutex<Vec<TraceEvent>>,
    /// Trace header (fixed at construction time).
    header: TraceHeader,
}

impl TraceRecorder {
    /// Start recording.
    ///
    /// Serializes `initial_scene` to JSON for the trace header. This snapshot is
    /// used by [`tze_hud_scene::replay::TraceReplayer`] to initialize the scene
    /// graph before replaying events.
    ///
    /// # Panics
    ///
    /// Panics if `initial_scene` cannot be serialized (this should never happen
    /// for a valid `SceneGraph`).
    pub fn start(initial_scene: &SceneGraph, label: impl Into<String>) -> Self {
        let initial_json = serde_json::to_string(initial_scene)
            .expect("TraceRecorder::start: failed to serialize initial scene");

        // Bias by | 1 so that 0 (the "not set" sentinel, spec lines 68-70)
        // is never emitted into the trace header even if the system clock
        // pre-dates the Unix epoch.
        let started_at_wall_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
            | 1;

        let header = TraceHeader {
            trace_id: SceneId::new(),
            label: label.into(),
            started_at_wall_us,
            initial_scene_json: initial_json,
            schema_version: TraceHeader::SCHEMA_VERSION,
        };

        Self {
            inner: Arc::new(TraceRecorderInner {
                mono_origin: Instant::now(),
                next_seq: Mutex::new(0),
                events: Mutex::new(Vec::new()),
                header,
            }),
        }
    }

    // ── Recording methods ──────────────────────────────────────────────────

    /// Record a mutation batch and its outcome.
    ///
    /// Call this after `SceneGraph::apply_batch` returns so you have the
    /// actual `applied` status and `resulting_version`.
    pub fn record_mutation_batch(
        &self,
        batch: &MutationBatch,
        applied: bool,
        resulting_version: Option<u64>,
    ) {
        self.append(TraceEventKind::MutationBatch {
            batch: batch.clone(),
            applied,
            resulting_version,
        });
    }

    /// Record an OS input event (from the winit event loop drain stage).
    pub fn record_input_event(&self, event: &RuntimeInputEvent) {
        let traced = convert_input_event(&event.kind);
        self.append(TraceEventKind::InputEvent { event: traced });
    }

    /// Record a zone publish event.
    ///
    /// Call this when a zone publish is processed (either from a
    /// `MutationBatch::PublishToZone` mutation or a direct API call).
    pub fn record_zone_publish(&self, publish: TracedZonePublish) {
        self.append(TraceEventKind::ZonePublish { publish });
    }

    /// Record an agent event (connect/disconnect/lease grant/revoke).
    pub fn record_agent_event(&self, event: TracedAgentEvent) {
        self.append(TraceEventKind::AgentEvent { event });
    }

    /// Record a clock tick (simulated time advancement, useful in tests).
    pub fn record_clock_tick(&self, now_us: u64) {
        self.append(TraceEventKind::ClockTick { now_us });
    }

    /// Record a frame boundary (called after each compositor frame completes).
    pub fn record_frame_boundary(&self, frame_number: u64) {
        self.append(TraceEventKind::FrameBoundary { frame_number });
    }

    // ── Finalization ───────────────────────────────────────────────────────

    /// Finish recording and return the completed [`SceneTrace`].
    ///
    /// The recorder remains usable after calling `finish` — it returns a clone
    /// of the current state. Subsequent `record_*` calls will continue appending
    /// to the same buffer.
    pub fn finish(&self) -> SceneTrace {
        let events = self
            .inner
            .events
            .lock()
            .expect("TraceRecorder: events lock poisoned")
            .clone();

        SceneTrace {
            header: self.inner.header.clone(),
            events,
        }
    }

    /// Returns the number of events recorded so far.
    pub fn event_count(&self) -> usize {
        self.inner
            .events
            .lock()
            .expect("TraceRecorder: events lock poisoned")
            .len()
    }

    /// Returns `true` if the recorder has recorded at least one event.
    pub fn has_events(&self) -> bool {
        self.event_count() > 0
    }

    // ── Internal helpers ───────────────────────────────────────────────────

    fn append(&self, kind: TraceEventKind) {
        let timestamp = self.now();
        let seq = {
            let mut guard = self
                .inner
                .next_seq
                .lock()
                .expect("TraceRecorder: next_seq lock poisoned");
            let s = *guard;
            *guard = s + 1;
            s
        };
        let event = TraceEvent { seq, timestamp, kind };
        self.inner
            .events
            .lock()
            .expect("TraceRecorder: events lock poisoned")
            .push(event);
    }

    fn now(&self) -> TraceTimestamp {
        // Bias both timestamps by | 1 / + 1 so that 0 (the "not set" sentinel,
        // spec lines 68-70) is never emitted into trace events.  Mirrors the
        // bias applied in `SystemClock::now_us` and `SystemClock::monotonic_us`
        // (crates/tze_hud_scene/src/clock.rs).
        let wall_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
            | 1;
        let mono_us = self.inner.mono_origin.elapsed().as_micros() as u64 + 1;
        TraceTimestamp { wall_us, mono_us }
    }
}

// ─── Input event conversion ───────────────────────────────────────────────────

/// Convert a runtime `InputEventKind` into the serializable `TracedInputEvent`.
fn convert_input_event(kind: &InputEventKind) -> TracedInputEvent {
    match kind {
        InputEventKind::KeyPress { key } => TracedInputEvent::KeyPress { key: *key },
        InputEventKind::KeyRelease { key } => TracedInputEvent::KeyRelease { key: *key },
        InputEventKind::PointerMove { x, y } => TracedInputEvent::PointerMove { x: *x, y: *y },
        InputEventKind::PointerPress { x, y, button } => TracedInputEvent::PointerPress {
            x: *x,
            y: *y,
            button: *button,
        },
        InputEventKind::PointerRelease { x, y, button } => TracedInputEvent::PointerRelease {
            x: *x,
            y: *y,
            button: *button,
        },
        InputEventKind::Resize { width, height } => TracedInputEvent::Resize {
            width: *width,
            height: *height,
        },
        InputEventKind::CloseRequested => TracedInputEvent::CloseRequested,
    }
}

// ─── Fuzzing-to-regression helpers ───────────────────────────────────────────

/// Build a minimal `SceneTrace` from a sequence of `MutationBatch`es applied
/// to an empty scene graph.
///
/// This is the core of the fuzzing-to-regression pipeline:
///
/// 1. A fuzzer generates a sequence of `MutationBatch` inputs.
/// 2. When a crash/violation is found, the fuzzer calls this function to
///    package the reproducer as a `SceneTrace`.
/// 3. The trace is saved as a `.trace.json` file in the regression corpus.
/// 4. A `#[test]` is added that loads and replays the trace.
///
/// The scene graph starts empty (1920x1080 display area, no tabs, no tiles).
/// Each batch is applied in order; the `applied` status and `resulting_version`
/// are recorded faithfully (rejections are recorded as `applied=false`).
pub fn build_regression_trace(
    batches: &[MutationBatch],
    label: impl Into<String>,
) -> SceneTrace {
    let mut scene = SceneGraph::new(1920.0, 1080.0);
    let recorder = TraceRecorder::start(&scene, label);

    for batch in batches {
        let result = scene.apply_batch(batch);
        let resulting_version = if result.applied { Some(scene.version) } else { None };
        recorder.record_mutation_batch(batch, result.applied, resulting_version);
    }

    recorder.finish()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tze_hud_scene::mutation::{MutationBatch, SceneMutation, TimingHints};
    use tze_hud_scene::replay::{TraceReplayer, assert_trace_is_deterministic};
    use tze_hud_scene::trace::TraceEventKind;

    fn empty_batch(namespace: &str) -> MutationBatch {
        MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: namespace.into(),
            mutations: vec![],
            timing_hints: None,
            lease_id: None,
        }
    }

    #[test]
    fn recorder_starts_with_no_events() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "test");
        assert!(!recorder.has_events());
        assert_eq!(recorder.event_count(), 0);
    }

    #[test]
    fn recorder_captures_mutation_batch() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "batch-capture");

        let batch = empty_batch("agent");
        recorder.record_mutation_batch(&batch, true, Some(0));

        assert_eq!(recorder.event_count(), 1);
        let trace = recorder.finish();
        assert_eq!(trace.events.len(), 1);
        assert!(matches!(
            trace.events[0].kind,
            TraceEventKind::MutationBatch { applied: true, .. }
        ));
    }

    #[test]
    fn recorder_captures_input_event() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "input-capture");

        let ev = RuntimeInputEvent {
            timestamp_ns: 1_000_000,
            kind: InputEventKind::PointerMove { x: 50.0, y: 75.0 },
        };
        recorder.record_input_event(&ev);

        assert_eq!(recorder.event_count(), 1);
        let trace = recorder.finish();
        assert!(matches!(
            trace.events[0].kind,
            TraceEventKind::InputEvent { .. }
        ));
    }

    #[test]
    fn recorder_captures_frame_boundary() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "frame-capture");
        recorder.record_frame_boundary(42);

        let trace = recorder.finish();
        assert_eq!(trace.events.len(), 1);
        assert!(matches!(
            trace.events[0].kind,
            TraceEventKind::FrameBoundary { frame_number: 42 }
        ));
    }

    #[test]
    fn recorder_captures_agent_event() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "agent-event");

        recorder.record_agent_event(TracedAgentEvent::AgentConnected {
            namespace: "llm-1".into(),
        });
        recorder.record_agent_event(TracedAgentEvent::AgentDisconnected {
            namespace: "llm-1".into(),
        });

        let trace = recorder.finish();
        assert_eq!(trace.events.len(), 2);
        assert_eq!(trace.events[0].seq, 0);
        assert_eq!(trace.events[1].seq, 1);
    }

    #[test]
    fn recorder_seq_monotonically_increases() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "seq-test");

        for i in 0..10u64 {
            let batch = empty_batch("agent");
            recorder.record_mutation_batch(&batch, true, Some(i));
        }

        let trace = recorder.finish();
        assert_eq!(trace.events.len(), 10);
        for (i, event) in trace.events.iter().enumerate() {
            assert_eq!(event.seq, i as u64, "seq must be monotonic at index {i}");
        }
    }

    #[test]
    fn finish_returns_snapshot_not_consuming_recorder() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "snapshot-test");

        let batch = empty_batch("a");
        recorder.record_mutation_batch(&batch, true, Some(0));

        let trace1 = recorder.finish();
        assert_eq!(trace1.events.len(), 1);

        // Record another event after finish.
        recorder.record_frame_boundary(1);
        let trace2 = recorder.finish();
        assert_eq!(trace2.events.len(), 2, "second finish should include both events");
    }

    #[test]
    fn build_regression_trace_and_replay_deterministically() {
        // Simulate a fuzzer reproducer: a sequence of empty batches.
        let batches: Vec<MutationBatch> = (0..5)
            .map(|i| MutationBatch {
                batch_id: SceneId::new(),
                agent_namespace: format!("agent-{i}"),
                mutations: vec![SceneMutation::CreateTab {
                    name: format!("Tab-{i}"),
                    display_order: i as u32,
                }],
                timing_hints: None,
                lease_id: None,
            })
            .collect();

        let trace = build_regression_trace(&batches, "fuzz-5-tabs");

        // All batches should be recorded.
        assert_eq!(trace.event_count(), 5);

        // Replay must be deterministic.
        let result = TraceReplayer::new().replay(&trace);
        assert_trace_is_deterministic(&result);
        assert_eq!(result.replayed, 5);
    }

    #[test]
    fn build_regression_trace_records_rejections() {
        // Duplicate display_order should cause rejection.
        let batch0 = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".into(),
            mutations: vec![SceneMutation::CreateTab {
                name: "Main".into(),
                display_order: 0,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let batch1 = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".into(),
            mutations: vec![SceneMutation::CreateTab {
                name: "Conflict".into(),
                display_order: 0, // duplicate — should be rejected
            }],
            timing_hints: None,
            lease_id: None,
        };

        let trace = build_regression_trace(&[batch0, batch1], "fuzz-dup-display-order");
        assert_eq!(trace.event_count(), 2);

        // The first event should be applied=true, second applied=false.
        if let TraceEventKind::MutationBatch { applied, .. } = &trace.events[0].kind {
            assert!(*applied, "first batch should be accepted");
        }
        if let TraceEventKind::MutationBatch { applied, .. } = &trace.events[1].kind {
            assert!(!*applied, "second batch should be rejected (dup display_order)");
        }

        // Replay must still be deterministic (rejections are deterministic too).
        let result = TraceReplayer::new().replay(&trace);
        assert_trace_is_deterministic(&result);
    }

    #[test]
    fn convert_input_event_covers_all_kinds() {
        let cases = vec![
            InputEventKind::KeyPress { key: 65 },
            InputEventKind::KeyRelease { key: 65 },
            InputEventKind::PointerMove { x: 1.0, y: 2.0 },
            InputEventKind::PointerPress { x: 1.0, y: 2.0, button: 0 },
            InputEventKind::PointerRelease { x: 1.0, y: 2.0, button: 1 },
            InputEventKind::Resize { width: 800, height: 600 },
            InputEventKind::CloseRequested,
        ];

        for kind in cases {
            // Should not panic.
            let _ = convert_input_event(&kind);
        }
    }

    #[test]
    fn trace_header_has_correct_schema_version() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "schema-ver");
        let trace = recorder.finish();
        assert_eq!(trace.header.schema_version, TraceHeader::SCHEMA_VERSION);
        assert_eq!(trace.header.schema_version, 1);
    }

    #[test]
    fn recorder_records_zone_publish() {
        let scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "zone-publish");

        recorder.record_zone_publish(TracedZonePublish {
            zone_name: "subtitles".into(),
            agent_namespace: "llm".into(),
            expires_at_wall_us: Some(9_999_999),
            content_classification: Some("public".into()),
            merge_key: None,
        });

        let trace = recorder.finish();
        assert_eq!(trace.events.len(), 1);
        assert!(matches!(trace.events[0].kind, TraceEventKind::ZonePublish { .. }));
    }

    #[test]
    fn serialized_trace_from_recorder_round_trips() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let recorder = TraceRecorder::start(&scene, "round-trip");

        // Apply a real mutation so the trace has meaningful content.
        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".into(),
            mutations: vec![SceneMutation::CreateTab {
                name: "Home".into(),
                display_order: 0,
            }],
            timing_hints: Some(TimingHints {
                present_at_wall_us: Some(1_000_000),
                expires_at_wall_us: None,
            }),
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        recorder.record_mutation_batch(&batch, result.applied, Some(scene.version));
        recorder.record_frame_boundary(1);

        let trace = recorder.finish();
        let json = trace.to_json().expect("serialize");
        let restored = SceneTrace::from_json(&json).expect("deserialize");

        assert_eq!(restored.events.len(), 2);
        assert_eq!(restored.header.label, "round-trip");

        // Replay the restored trace.
        let replay = TraceReplayer::new().replay(&restored);
        assert_trace_is_deterministic(&replay);
    }
}
