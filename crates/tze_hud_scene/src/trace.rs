//! # Record/Replay Trace Format
//!
//! Structured trace format for recording and replaying scene mutations, input
//! events, zone publishes, and timing data for deterministic reproduction of
//! bugs and regression testing.
//!
//! ## Spec alignment
//!
//! Implements the `validation-framework/spec.md` §"Record/Replay Traces" requirement
//! (lines 283-295, v1-mandatory):
//!
//! - The runtime SHALL support recording sequences of scene mutations, agent
//!   events, input events, zone publishes, and timing data as structured traces.
//! - These traces SHALL be replayable deterministically against the scene graph.
//! - Fuzzing discoveries that produce minimal reproducers SHALL become permanent
//!   regression tests via this mechanism.
//!
//! ## Format overview
//!
//! A [`SceneTrace`] is a sequence of [`TraceEvent`]s with a [`TraceHeader`] that
//! captures the initial scene state. Events carry wall-clock timestamps and
//! monotonic timestamps for replay ordering.
//!
//! The format is intentionally newline-delimited JSON-serializable (each event is
//! a serde-serializable enum variant) so traces can be:
//! - Streamed incrementally (append only during capture)
//! - Stored as `.trace.json` files in the test corpus
//! - Diffed with standard JSON tools
//! - Minimized by fuzzer harnesses

use crate::mutation::MutationBatch;
use crate::types::SceneId;
use serde::{Deserialize, Serialize};

// ─── Timing metadata ──────────────────────────────────────────────────────────

/// Timestamp pair carried on every trace event.
///
/// Both values are in microseconds. `wall_us` is UTC wall-clock time since the
/// Unix epoch (for human display). `mono_us` is a monotonic counter from an
/// arbitrary process-local origin (for replay ordering).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceTimestamp {
    /// UTC wall-clock time in microseconds since the Unix epoch.
    pub wall_us: u64,
    /// Monotonic time in microseconds since process start (arbitrary origin).
    pub mono_us: u64,
}

// ─── Input event snapshot ─────────────────────────────────────────────────────

/// A serializable snapshot of an input event for trace recording.
///
/// This is a simplified, self-contained representation that does not depend on
/// the runtime channel types. It captures enough information to replay the event
/// ordering and content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TracedInputEvent {
    KeyPress { key: u32 },
    KeyRelease { key: u32 },
    PointerMove { x: f32, y: f32 },
    PointerPress { x: f32, y: f32, button: u8 },
    PointerRelease { x: f32, y: f32, button: u8 },
    Resize { width: u32, height: u32 },
    CloseRequested,
}

// ─── Zone publish snapshot ────────────────────────────────────────────────────

/// A serializable snapshot of a zone publish event for trace recording.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TracedZonePublish {
    /// The zone name receiving the publication.
    pub zone_name: String,
    /// The namespace/agent that published.
    pub agent_namespace: String,
    /// Optional expiry timestamp (wall-clock microseconds since epoch).
    pub expires_at_wall_us: Option<u64>,
    /// Optional content classification tag.
    pub content_classification: Option<String>,
    /// Optional merge key (for MergeByKey contention policy).
    pub merge_key: Option<String>,
}

// ─── Agent event snapshot ─────────────────────────────────────────────────────

/// A serializable snapshot of an agent event for trace recording.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TracedAgentEvent {
    /// An agent connected with the given namespace.
    AgentConnected { namespace: String },
    /// An agent disconnected.
    AgentDisconnected { namespace: String },
    /// A lease was granted to an agent.
    LeaseGranted {
        agent_namespace: String,
        lease_id: SceneId,
        duration_ms: u64,
    },
    /// A lease was revoked.
    LeaseRevoked {
        agent_namespace: String,
        lease_id: SceneId,
    },
}

// ─── Trace event ─────────────────────────────────────────────────────────────

/// A single recorded event in a scene trace.
///
/// Every event carries a [`TraceTimestamp`] for ordering and replay. The
/// sequence number is monotonically increasing within a trace and is used
/// as the primary ordering key during replay (in case timestamps have
/// insufficient resolution).
///
/// Note: `TraceEvent` does not implement `PartialEq` because `MutationBatch`
/// does not implement `PartialEq`. Use seq-number comparison for equality checks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Monotonically increasing sequence number within the trace (0-based).
    pub seq: u64,
    /// Timestamp at which this event was recorded.
    pub timestamp: TraceTimestamp,
    /// The event payload.
    pub kind: TraceEventKind,
}

/// The payload of a trace event.
///
/// Note: does not implement `PartialEq` because `MutationBatch` does not.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TraceEventKind {
    /// A mutation batch was applied to the scene graph.
    MutationBatch {
        batch: MutationBatch,
        /// True if the batch was accepted and applied; false if rejected.
        applied: bool,
        /// The scene graph version after this event (only valid when `applied`).
        resulting_version: Option<u64>,
    },
    /// An input event was received from the OS.
    InputEvent { event: TracedInputEvent },
    /// A zone publish occurred (outside of a MutationBatch).
    ZonePublish { publish: TracedZonePublish },
    /// An agent-level event (connect/disconnect/lease).
    AgentEvent { event: TracedAgentEvent },
    /// A clock tick: the simulated clock was advanced to this value.
    /// Only emitted when recording against a `SimulatedClock`.
    ClockTick { now_us: u64 },
    /// A frame boundary: the compositor completed rendering frame `n`.
    FrameBoundary { frame_number: u64 },
}

// ─── Trace header ─────────────────────────────────────────────────────────────

/// Header information recorded at the start of a trace.
///
/// The header captures the initial scene state (as a serialized snapshot) and
/// metadata about the recording session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceHeader {
    /// Unique ID for this trace (UUIDv7, assigned at recording start).
    pub trace_id: SceneId,
    /// Human-readable label (e.g., "fuzz reproducer for oom-in-create-tile").
    pub label: String,
    /// Wall-clock time at which recording started (microseconds since epoch).
    pub started_at_wall_us: u64,
    /// Initial scene graph state, serialized as JSON.
    ///
    /// During replay, the scene graph is initialized from this snapshot before
    /// the first event is applied. This ensures deterministic replay even if the
    /// scene graph had non-default state before recording began.
    pub initial_scene_json: String,
    /// Schema version of this trace format. Currently "1".
    pub schema_version: u32,
}

impl TraceHeader {
    /// Current schema version.
    pub const SCHEMA_VERSION: u32 = 1;
}

// ─── Complete trace ───────────────────────────────────────────────────────────

/// A complete recorded trace: header + ordered sequence of events.
///
/// ## Serialization
///
/// A `SceneTrace` serializes to compact JSON. For large traces, consider
/// streaming each event as a newline-delimited JSON line during capture rather
/// than collecting everything in memory.
///
/// ## Replay
///
/// Use [`crate::replay::TraceReplayer`] to replay a `SceneTrace` against a
/// freshly constructed scene graph and verify that outcomes match.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneTrace {
    pub header: TraceHeader,
    pub events: Vec<TraceEvent>,
}

impl SceneTrace {
    /// Create a new empty trace with the given header.
    pub fn new(header: TraceHeader) -> Self {
        Self {
            header,
            events: Vec::new(),
        }
    }

    /// Number of events recorded in this trace.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Serialize the trace to a JSON string.
    ///
    /// Returns an error if serialization fails (e.g., a `MutationBatch` contains
    /// non-serializable data — in practice this should never happen as all fields
    /// implement `Serialize`).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serialize the trace to a pretty-printed JSON string (for human readability).
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a trace from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Returns an iterator over mutation batch events in the trace, in order.
    pub fn mutation_events(&self) -> impl Iterator<Item = &TraceEvent> {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, TraceEventKind::MutationBatch { .. }))
    }

    /// Returns an iterator over input events in the trace, in order.
    pub fn input_events(&self) -> impl Iterator<Item = &TraceEvent> {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, TraceEventKind::InputEvent { .. }))
    }
}

// ─── Replay outcome ───────────────────────────────────────────────────────────

/// The result of a single event replay step.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplayStepOutcome {
    /// Event replayed and outcome matches the recorded result.
    Matched,
    /// Event replayed but the outcome differs from the recorded result.
    ///
    /// This indicates a non-determinism bug: the same inputs produced a
    /// different output on the second run.
    Diverged { seq: u64, description: String },
    /// Event could not be replayed (e.g., a MutationBatch with a lease_id
    /// that does not exist in the replayed scene graph, unrelated to the bug).
    Skipped { seq: u64, reason: String },
}

/// The result of replaying an entire trace.
#[derive(Clone, Debug)]
pub struct ReplayResult {
    /// Number of events successfully replayed.
    pub replayed: usize,
    /// Number of events skipped (non-fatal, e.g., agent-level events with no
    /// replay handler).
    pub skipped: usize,
    /// Divergences detected: events whose replay outcome differs from the
    /// recorded outcome.
    pub divergences: Vec<ReplayStepOutcome>,
    /// Final scene graph version after replay.
    pub final_version: u64,
}

impl ReplayResult {
    /// Returns `true` if the replay completed with no divergences.
    pub fn is_deterministic(&self) -> bool {
        self.divergences.is_empty()
    }

    /// Returns `true` if there were any divergences.
    pub fn has_divergences(&self) -> bool {
        !self.divergences.is_empty()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::SceneGraph;
    use crate::mutation::{BatchTimingHints, MutationBatch};
    use crate::timing::domains::WallUs;

    fn make_timestamp(mono_us: u64) -> TraceTimestamp {
        TraceTimestamp {
            wall_us: 1_735_689_600_000_000 + mono_us,
            mono_us,
        }
    }

    #[test]
    fn trace_serializes_and_deserializes_round_trip() {
        let graph = SceneGraph::new(1920.0, 1080.0);
        let initial_json = serde_json::to_string(&graph).unwrap();

        let header = TraceHeader {
            trace_id: SceneId::new(),
            label: "round-trip test".into(),
            started_at_wall_us: 1_735_689_600_000_000,
            initial_scene_json: initial_json,
            schema_version: TraceHeader::SCHEMA_VERSION,
        };

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test-agent".into(),
            mutations: vec![],
            timing_hints: Some(BatchTimingHints {
                present_at_wall_us: Some(WallUs(1_735_689_600_001_000)),
                expires_at_wall_us: None,
            }),
            lease_id: None,
        };

        let mut trace = SceneTrace::new(header);
        trace.events.push(TraceEvent {
            seq: 0,
            timestamp: make_timestamp(100),
            kind: TraceEventKind::MutationBatch {
                batch,
                applied: true,
                resulting_version: Some(1),
            },
        });
        trace.events.push(TraceEvent {
            seq: 1,
            timestamp: make_timestamp(200),
            kind: TraceEventKind::InputEvent {
                event: TracedInputEvent::PointerMove { x: 100.0, y: 200.0 },
            },
        });
        trace.events.push(TraceEvent {
            seq: 2,
            timestamp: make_timestamp(300),
            kind: TraceEventKind::ClockTick {
                now_us: 1_735_689_600_000_300,
            },
        });

        let json = trace.to_json().expect("serialize trace");
        let restored = SceneTrace::from_json(&json).expect("deserialize trace");

        assert_eq!(restored.header.label, trace.header.label);
        assert_eq!(restored.header.schema_version, TraceHeader::SCHEMA_VERSION);
        assert_eq!(restored.events.len(), 3);
        assert_eq!(restored.events[0].seq, 0);
        assert_eq!(restored.events[1].seq, 1);
    }

    #[test]
    fn trace_mutation_events_filter() {
        let graph = SceneGraph::new(1920.0, 1080.0);
        let initial_json = serde_json::to_string(&graph).unwrap();

        let header = TraceHeader {
            trace_id: SceneId::new(),
            label: "filter test".into(),
            started_at_wall_us: 0,
            initial_scene_json: initial_json,
            schema_version: TraceHeader::SCHEMA_VERSION,
        };

        let mut trace = SceneTrace::new(header);

        // 3 mutation events + 2 input events
        for i in 0..3 {
            let batch = MutationBatch {
                batch_id: SceneId::new(),
                agent_namespace: "agent".into(),
                mutations: vec![],
                timing_hints: None,
                lease_id: None,
            };
            trace.events.push(TraceEvent {
                seq: i as u64 * 2,
                timestamp: make_timestamp(i as u64 * 100),
                kind: TraceEventKind::MutationBatch {
                    batch,
                    applied: true,
                    resulting_version: Some(i as u64),
                },
            });
            trace.events.push(TraceEvent {
                seq: i as u64 * 2 + 1,
                timestamp: make_timestamp(i as u64 * 100 + 50),
                kind: TraceEventKind::InputEvent {
                    event: TracedInputEvent::KeyPress { key: i as u32 },
                },
            });
        }

        assert_eq!(trace.mutation_events().count(), 3);
        assert_eq!(trace.input_events().count(), 3);
        assert_eq!(trace.event_count(), 6);
    }

    #[test]
    fn traced_input_event_serializes() {
        let events = vec![
            TracedInputEvent::KeyPress { key: 65 },
            TracedInputEvent::PointerMove { x: 1.5, y: 2.5 },
            TracedInputEvent::PointerPress {
                x: 10.0,
                y: 20.0,
                button: 0,
            },
            TracedInputEvent::Resize {
                width: 1920,
                height: 1080,
            },
            TracedInputEvent::CloseRequested,
        ];
        for ev in &events {
            let json = serde_json::to_string(ev).unwrap();
            let restored: TracedInputEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(ev, &restored);
        }
    }

    #[test]
    fn agent_events_serialize() {
        let ev = TracedAgentEvent::LeaseGranted {
            agent_namespace: "bot".into(),
            lease_id: SceneId::new(),
            duration_ms: 5000,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let restored: TracedAgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, restored);
    }

    #[test]
    fn zone_publish_serializes() {
        let zp = TracedZonePublish {
            zone_name: "subtitles".into(),
            agent_namespace: "llm-1".into(),
            expires_at_wall_us: Some(9_999_999),
            content_classification: Some("public".into()),
            merge_key: None,
        };
        let json = serde_json::to_string(&zp).unwrap();
        let restored: TracedZonePublish = serde_json::from_str(&json).unwrap();
        assert_eq!(zp, restored);
    }

    #[test]
    fn replay_result_determinism_check() {
        let result_clean = ReplayResult {
            replayed: 10,
            skipped: 0,
            divergences: vec![],
            final_version: 10,
        };
        assert!(result_clean.is_deterministic());
        assert!(!result_clean.has_divergences());

        let result_diverged = ReplayResult {
            replayed: 10,
            skipped: 0,
            divergences: vec![ReplayStepOutcome::Diverged {
                seq: 3,
                description: "version mismatch".into(),
            }],
            final_version: 10,
        };
        assert!(!result_diverged.is_deterministic());
        assert!(result_diverged.has_divergences());
    }
}
