//! # Deterministic Replay Engine
//!
//! Replays a [`crate::trace::SceneTrace`] against a freshly initialized scene
//! graph and verifies that outcomes match the recorded results.
//!
//! ## Spec alignment
//!
//! Implements the `validation-framework/spec.md` §"Record/Replay Traces"
//! requirement (lines 283-295, v1-mandatory):
//!
//! > WHEN a trace is replayed against the same scene state THEN the outcome
//! > is identical to the original.
//!
//! ## Design
//!
//! [`TraceReplayer`] takes a `SceneTrace` and reconstructs the initial scene
//! from `header.initial_scene_json`. It then iterates over events in `seq`
//! order, applying each event to the scene graph and comparing the result to
//! what was recorded.
//!
//! ### What "identical outcome" means
//!
//! For [`TraceEventKind::MutationBatch`]:
//! - `applied` must match: if the original batch was accepted, the replay
//!   must also accept it; if rejected, the replay must also reject it.
//! - `resulting_version` (when `applied == true`) must match the scene
//!   graph's `version` after the replay batch is applied.
//!
//! For other event kinds (input, zone publish, agent events, clock ticks,
//! frame boundaries), the replay currently records them as `Skipped` — they
//! contribute to timeline fidelity but have no independently verifiable
//! outcome in the pure scene graph layer. Higher-level replay harnesses
//! (e.g., full runtime replay) can extend this.
//!
//! ## Fuzzing-to-regression pipeline
//!
//! The [`TraceReplayer`] is the exit point for the fuzzing-to-regression
//! pipeline. After a fuzzer finds a minimal reproducer (a sequence of
//! `SceneMutation`s and timing deltas), the coordinator:
//!
//! 1. Wraps the reproducer in `TraceEvent::MutationBatch` entries.
//! 2. Constructs a `SceneTrace` with an empty initial scene.
//! 3. Verifies that `TraceReplayer::replay` exposes the invariant violation.
//! 4. Saves the trace as `tests/regression/traces/<id>.trace.json`.
//! 5. Adds a `#[test]` that calls [`assert_trace_is_deterministic`].

use crate::graph::SceneGraph;
use crate::trace::{
    ReplayResult, ReplayStepOutcome, SceneTrace, TraceEventKind,
};

// ─── TraceReplayer ────────────────────────────────────────────────────────────

/// Replays a [`SceneTrace`] deterministically.
///
/// # Usage
///
/// ```rust,ignore
/// use tze_hud_scene::replay::{TraceReplayer, assert_trace_is_deterministic};
/// use tze_hud_scene::trace::SceneTrace;
///
/// let trace: SceneTrace = SceneTrace::from_json(json_str).unwrap();
/// let result = TraceReplayer::new().replay(&trace);
/// assert_trace_is_deterministic(&result);
/// ```
pub struct TraceReplayer {
    /// If `true`, the replayer logs each step to `stderr` for debugging.
    pub verbose: bool,
}

impl TraceReplayer {
    /// Create a new replayer with default settings.
    pub fn new() -> Self {
        Self { verbose: false }
    }

    /// Create a verbose replayer that logs each replay step to `stderr`.
    pub fn verbose() -> Self {
        Self { verbose: true }
    }

    /// Replay `trace` and return the comparison result.
    ///
    /// ## Process
    ///
    /// 1. Deserialize the initial scene graph from `trace.header.initial_scene_json`.
    /// 2. Sort events by `seq` (they should already be sorted, but enforce it).
    /// 3. For each event, apply it to the scene graph and compare to recorded result.
    /// 4. Return a [`ReplayResult`] summarizing matches, skips, and divergences.
    ///
    /// ## Errors
    ///
    /// If `initial_scene_json` cannot be deserialized, returns a `ReplayResult`
    /// with a single divergence entry describing the parse failure and zero
    /// replayed events.
    pub fn replay(&self, trace: &SceneTrace) -> ReplayResult {
        // ── Step 1: Reconstruct initial scene ─────────────────────────────
        let mut scene: SceneGraph = match serde_json::from_str(&trace.header.initial_scene_json) {
            Ok(g) => g,
            Err(e) => {
                return ReplayResult {
                    replayed: 0,
                    skipped: 0,
                    divergences: vec![ReplayStepOutcome::Diverged {
                        seq: 0,
                        description: format!("failed to deserialize initial scene: {e}"),
                    }],
                    final_version: 0,
                };
            }
        };

        // ── Step 2: Sort events by seq ─────────────────────────────────────
        let mut events = trace.events.clone();
        events.sort_by_key(|e| e.seq);

        // ── Step 3: Replay each event ──────────────────────────────────────
        let mut replayed = 0usize;
        let mut skipped = 0usize;
        let mut divergences = Vec::new();

        for event in &events {
            if self.verbose {
                eprintln!(
                    "[trace-replay] seq={} kind={}",
                    event.seq,
                    event_kind_name(&event.kind)
                );
            }

            match &event.kind {
                TraceEventKind::MutationBatch {
                    batch,
                    applied: recorded_applied,
                    resulting_version: recorded_version,
                } => {
                    let result = scene.apply_batch(batch);
                    let replay_applied = result.applied;

                    if replay_applied != *recorded_applied {
                        divergences.push(ReplayStepOutcome::Diverged {
                            seq: event.seq,
                            description: format!(
                                "mutation batch {} outcome mismatch: recorded applied={}, \
                                 replay applied={}",
                                batch.batch_id, recorded_applied, replay_applied
                            ),
                        });
                    } else if replay_applied {
                        // Both applied — check version consistency
                        if let Some(expected_ver) = recorded_version {
                            let actual_ver = scene.version;
                            if actual_ver != *expected_ver {
                                divergences.push(ReplayStepOutcome::Diverged {
                                    seq: event.seq,
                                    description: format!(
                                        "scene version mismatch after batch {}: \
                                         expected {expected_ver}, got {actual_ver}",
                                        batch.batch_id
                                    ),
                                });
                            }
                        }
                    }
                    // Record as matched (even if we also pushed a divergence above —
                    // we still "ran" the step)
                    replayed += 1;
                }

                // Input events, agent events, zone publishes, clock ticks, and
                // frame boundaries have no verifiable outcome at the pure scene
                // layer. Skip them but count them.
                TraceEventKind::InputEvent { .. }
                | TraceEventKind::ZonePublish { .. }
                | TraceEventKind::AgentEvent { .. }
                | TraceEventKind::ClockTick { .. }
                | TraceEventKind::FrameBoundary { .. } => {
                    skipped += 1;
                }
            }
        }

        ReplayResult {
            replayed,
            skipped,
            divergences,
            final_version: scene.version,
        }
    }
}

impl Default for TraceReplayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Return a human-readable name for the event kind (used in verbose logging).
fn event_kind_name(kind: &TraceEventKind) -> &'static str {
    match kind {
        TraceEventKind::MutationBatch { .. } => "MutationBatch",
        TraceEventKind::InputEvent { .. } => "InputEvent",
        TraceEventKind::ZonePublish { .. } => "ZonePublish",
        TraceEventKind::AgentEvent { .. } => "AgentEvent",
        TraceEventKind::ClockTick { .. } => "ClockTick",
        TraceEventKind::FrameBoundary { .. } => "FrameBoundary",
    }
}

// ─── Regression test helpers ─────────────────────────────────────────────────

/// Assert that replaying `trace` produces no divergences.
///
/// Intended for use in `#[test]` functions that load trace files from the
/// regression corpus (`tests/regression/traces/`):
///
/// ```rust,ignore
/// #[test]
/// fn regression_fuzz_oom_in_create_tile() {
///     let json = include_str!("../tests/regression/traces/fuzz-oom-create-tile.trace.json");
///     let trace = SceneTrace::from_json(json).unwrap();
///     assert_trace_is_deterministic(&TraceReplayer::new().replay(&trace));
/// }
/// ```
///
/// # Panics
///
/// Panics with a structured message listing all divergences if any are found.
pub fn assert_trace_is_deterministic(result: &ReplayResult) {
    if result.has_divergences() {
        let msgs: Vec<String> = result
            .divergences
            .iter()
            .map(|d| match d {
                ReplayStepOutcome::Diverged { seq, description } => {
                    format!("  [seq={seq}] {description}")
                }
                ReplayStepOutcome::Skipped { seq, reason } => {
                    format!("  [seq={seq}] skipped: {reason}")
                }
                ReplayStepOutcome::Matched => "  [matched] (unexpected in divergences list)".into(),
            })
            .collect();
        panic!(
            "trace replay produced {} divergence(s):\n{}",
            result.divergences.len(),
            msgs.join("\n")
        );
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;
    use crate::mutation::{MutationBatch, SceneMutation};
    use crate::trace::{SceneTrace, TraceEvent, TraceEventKind, TraceHeader, TraceTimestamp};
    use crate::types::{Capability, SceneId};
    use std::sync::Arc;

    fn make_timestamp(mono_us: u64) -> TraceTimestamp {
        TraceTimestamp {
            wall_us: 1_735_689_600_000_000 + mono_us,
            mono_us,
        }
    }

    /// Construct a minimal trace with the given initial graph and events.
    fn make_trace(
        initial: &SceneGraph,
        events: Vec<TraceEvent>,
        label: &str,
    ) -> SceneTrace {
        let initial_json = serde_json::to_string(initial).unwrap();
        let header = TraceHeader {
            trace_id: SceneId::new(),
            label: label.into(),
            started_at_wall_us: 1_735_689_600_000_000,
            initial_scene_json: initial_json,
            schema_version: TraceHeader::SCHEMA_VERSION,
        };
        SceneTrace { header, events }
    }

    #[test]
    fn empty_trace_replays_deterministically() {
        let graph = SceneGraph::new(1920.0, 1080.0);
        let trace = make_trace(&graph, vec![], "empty");
        let result = TraceReplayer::new().replay(&trace);
        assert!(result.is_deterministic());
        assert_eq!(result.replayed, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.final_version, 0);
    }

    #[test]
    fn trace_with_empty_batch_replays_deterministically() {
        let clock = Arc::new(TestClock::new(1_000));
        let graph = SceneGraph::new_with_clock(1920.0, 1080.0, clock.clone());
        let initial_json = serde_json::to_string(&graph).unwrap();

        // An empty mutation batch with no lease_id always succeeds.
        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".into(),
            mutations: vec![],
            timing_hints: None,
            lease_id: None,
        };

        // Simulate applying and recording what the outcome was.
        let mut sim_graph = serde_json::from_str::<SceneGraph>(&initial_json).unwrap();
        let result = sim_graph.apply_batch(&batch);
        let recorded_applied = result.applied;
        let recorded_version = if result.applied { Some(sim_graph.version) } else { None };

        let trace = SceneTrace {
            header: TraceHeader {
                trace_id: SceneId::new(),
                label: "batch test".into(),
                started_at_wall_us: 1_735_689_600_000_000,
                initial_scene_json: initial_json,
                schema_version: TraceHeader::SCHEMA_VERSION,
            },
            events: vec![TraceEvent {
                seq: 0,
                timestamp: make_timestamp(100),
                kind: TraceEventKind::MutationBatch {
                    batch,
                    applied: recorded_applied,
                    resulting_version: recorded_version,
                },
            }],
        };

        let replay_result = TraceReplayer::new().replay(&trace);
        assert_trace_is_deterministic(&replay_result);
        assert_eq!(replay_result.replayed, 1);
    }

    #[test]
    fn trace_with_create_tab_replays_deterministically() {
        let clock = Arc::new(TestClock::new(1_000));
        let graph = SceneGraph::new_with_clock(1920.0, 1080.0, clock.clone());
        let initial_json = serde_json::to_string(&graph).unwrap();

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent-1".into(),
            mutations: vec![SceneMutation::CreateTab {
                name: "Dashboard".into(),
                display_order: 0,
            }],
            timing_hints: None,
            lease_id: None,
        };

        // Record the outcome.
        let mut sim_graph = serde_json::from_str::<SceneGraph>(&initial_json).unwrap();
        let result = sim_graph.apply_batch(&batch);
        assert!(result.applied, "CreateTab should succeed on empty graph");

        let trace = SceneTrace {
            header: TraceHeader {
                trace_id: SceneId::new(),
                label: "create-tab".into(),
                started_at_wall_us: 0,
                initial_scene_json: initial_json,
                schema_version: TraceHeader::SCHEMA_VERSION,
            },
            events: vec![TraceEvent {
                seq: 0,
                timestamp: make_timestamp(0),
                kind: TraceEventKind::MutationBatch {
                    batch,
                    applied: true,
                    resulting_version: Some(sim_graph.version),
                },
            }],
        };

        let replay_result = TraceReplayer::new().replay(&trace);
        assert_trace_is_deterministic(&replay_result);
    }

    #[test]
    fn divergence_detected_when_version_mismatch() {
        let graph = SceneGraph::new(1920.0, 1080.0);
        let initial_json = serde_json::to_string(&graph).unwrap();

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".into(),
            mutations: vec![SceneMutation::CreateTab {
                name: "Main".into(),
                display_order: 0,
            }],
            timing_hints: None,
            lease_id: None,
        };

        // Lie about the resulting version to inject an artificial divergence.
        let trace = SceneTrace {
            header: TraceHeader {
                trace_id: SceneId::new(),
                label: "divergence-test".into(),
                started_at_wall_us: 0,
                initial_scene_json: initial_json,
                schema_version: TraceHeader::SCHEMA_VERSION,
            },
            events: vec![TraceEvent {
                seq: 0,
                timestamp: make_timestamp(0),
                kind: TraceEventKind::MutationBatch {
                    batch,
                    applied: true,
                    resulting_version: Some(999), // wrong version — should be 1
                },
            }],
        };

        let result = TraceReplayer::new().replay(&trace);
        assert!(
            result.has_divergences(),
            "should detect version mismatch divergence"
        );
        assert_eq!(result.divergences.len(), 1);
        if let ReplayStepOutcome::Diverged { seq, description } = &result.divergences[0] {
            assert_eq!(*seq, 0);
            assert!(description.contains("version mismatch"), "description: {description}");
        } else {
            panic!("expected Diverged outcome");
        }
    }

    #[test]
    fn applied_mismatch_detected() {
        let graph = SceneGraph::new(1920.0, 1080.0);
        let initial_json = serde_json::to_string(&graph).unwrap();

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".into(),
            mutations: vec![SceneMutation::CreateTab {
                name: "X".into(),
                display_order: 0,
            }],
            timing_hints: None,
            lease_id: None,
        };

        // Record "applied=false" but the replay will succeed — deliberate mismatch.
        let trace = SceneTrace {
            header: TraceHeader {
                trace_id: SceneId::new(),
                label: "applied-mismatch".into(),
                started_at_wall_us: 0,
                initial_scene_json: initial_json,
                schema_version: TraceHeader::SCHEMA_VERSION,
            },
            events: vec![TraceEvent {
                seq: 0,
                timestamp: make_timestamp(0),
                kind: TraceEventKind::MutationBatch {
                    batch,
                    applied: false, // lie — replay will return applied=true
                    resulting_version: None,
                },
            }],
        };

        let result = TraceReplayer::new().replay(&trace);
        assert!(result.has_divergences(), "should detect applied mismatch");
        if let ReplayStepOutcome::Diverged { description, .. } = &result.divergences[0] {
            assert!(
                description.contains("outcome mismatch"),
                "description: {description}"
            );
        }
    }

    #[test]
    fn non_mutation_events_are_skipped() {
        let graph = SceneGraph::new(1920.0, 1080.0);
        let initial_json = serde_json::to_string(&graph).unwrap();

        use crate::trace::{TracedAgentEvent, TracedInputEvent, TracedZonePublish};

        let trace = SceneTrace {
            header: TraceHeader {
                trace_id: SceneId::new(),
                label: "non-mutation".into(),
                started_at_wall_us: 0,
                initial_scene_json: initial_json,
                schema_version: TraceHeader::SCHEMA_VERSION,
            },
            events: vec![
                TraceEvent {
                    seq: 0,
                    timestamp: make_timestamp(0),
                    kind: TraceEventKind::InputEvent {
                        event: TracedInputEvent::PointerMove { x: 10.0, y: 20.0 },
                    },
                },
                TraceEvent {
                    seq: 1,
                    timestamp: make_timestamp(100),
                    kind: TraceEventKind::ZonePublish {
                        publish: TracedZonePublish {
                            zone_name: "subtitles".into(),
                            agent_namespace: "llm".into(),
                            expires_at_wall_us: None,
                            content_classification: None,
                            merge_key: None,
                        },
                    },
                },
                TraceEvent {
                    seq: 2,
                    timestamp: make_timestamp(200),
                    kind: TraceEventKind::AgentEvent {
                        event: TracedAgentEvent::AgentConnected {
                            namespace: "bot".into(),
                        },
                    },
                },
                TraceEvent {
                    seq: 3,
                    timestamp: make_timestamp(300),
                    kind: TraceEventKind::ClockTick { now_us: 5_000_000 },
                },
                TraceEvent {
                    seq: 4,
                    timestamp: make_timestamp(400),
                    kind: TraceEventKind::FrameBoundary { frame_number: 42 },
                },
            ],
        };

        let result = TraceReplayer::new().replay(&trace);
        assert!(result.is_deterministic());
        assert_eq!(result.replayed, 0);
        assert_eq!(result.skipped, 5);
    }

    #[test]
    fn events_replayed_in_seq_order_even_if_stored_out_of_order() {
        let graph = SceneGraph::new(1920.0, 1080.0);
        let initial_json = serde_json::to_string(&graph).unwrap();

        // Two CreateTab mutations that must be applied in seq order to avoid
        // duplicate display_order conflicts. If they were swapped the second
        // would collide with the first.
        let batch0 = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".into(),
            mutations: vec![SceneMutation::CreateTab {
                name: "First".into(),
                display_order: 0,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let batch1 = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".into(),
            mutations: vec![SceneMutation::CreateTab {
                name: "Second".into(),
                display_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };

        let mut sim = serde_json::from_str::<SceneGraph>(&initial_json).unwrap();
        let r0 = sim.apply_batch(&batch0);
        let v0 = sim.version;
        let r1 = sim.apply_batch(&batch1);
        let v1 = sim.version;
        assert!(r0.applied && r1.applied);

        // Store events in reverse order to test sorting.
        let trace = SceneTrace {
            header: TraceHeader {
                trace_id: SceneId::new(),
                label: "ordering".into(),
                started_at_wall_us: 0,
                initial_scene_json: initial_json,
                schema_version: TraceHeader::SCHEMA_VERSION,
            },
            events: vec![
                TraceEvent {
                    seq: 1, // out of order
                    timestamp: make_timestamp(100),
                    kind: TraceEventKind::MutationBatch {
                        batch: batch1,
                        applied: true,
                        resulting_version: Some(v1),
                    },
                },
                TraceEvent {
                    seq: 0,
                    timestamp: make_timestamp(0),
                    kind: TraceEventKind::MutationBatch {
                        batch: batch0,
                        applied: true,
                        resulting_version: Some(v0),
                    },
                },
            ],
        };

        let result = TraceReplayer::new().replay(&trace);
        assert_trace_is_deterministic(&result);
        assert_eq!(result.replayed, 2);
    }

    #[test]
    fn invalid_initial_scene_json_produces_divergence() {
        let trace = SceneTrace {
            header: TraceHeader {
                trace_id: SceneId::new(),
                label: "bad-json".into(),
                started_at_wall_us: 0,
                initial_scene_json: "{not valid json!!!}".into(),
                schema_version: TraceHeader::SCHEMA_VERSION,
            },
            events: vec![],
        };

        let result = TraceReplayer::new().replay(&trace);
        assert!(result.has_divergences());
        if let ReplayStepOutcome::Diverged { description, .. } = &result.divergences[0] {
            assert!(description.contains("failed to deserialize initial scene"));
        }
    }

    #[test]
    fn grant_lease_then_create_tile_replays_deterministically() {
        let clock = Arc::new(TestClock::new(1_000));
        let mut graph = SceneGraph::new_with_clock(1920.0, 1080.0, clock.clone());

        // Grant a lease on the live graph before snapshotting, so that
        // the initial_scene_json includes the lease.
        let lease_id = graph.grant_lease("agent-a", 60_000, vec![Capability::CreateTile]);
        let _ = graph.create_tab("Main", 0).unwrap();

        let initial_json = serde_json::to_string(&graph).unwrap();

        // Now build a batch that creates a tile in the existing tab.
        let tab_id = *graph.tabs.keys().next().unwrap();
        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent-a".into(),
            mutations: vec![SceneMutation::CreateTile {
                tab_id,
                namespace: "agent-a".into(),
                lease_id,
                bounds: crate::types::Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: Some(lease_id),
        };

        let mut sim = serde_json::from_str::<SceneGraph>(&initial_json).unwrap();
        let result = sim.apply_batch(&batch);
        assert!(result.applied, "CreateTile should succeed");

        let trace = SceneTrace {
            header: TraceHeader {
                trace_id: SceneId::new(),
                label: "tile-replay".into(),
                started_at_wall_us: 0,
                initial_scene_json: initial_json,
                schema_version: TraceHeader::SCHEMA_VERSION,
            },
            events: vec![TraceEvent {
                seq: 0,
                timestamp: make_timestamp(0),
                kind: TraceEventKind::MutationBatch {
                    batch,
                    applied: true,
                    resulting_version: Some(sim.version),
                },
            }],
        };

        let replay_result = TraceReplayer::new().replay(&trace);
        assert_trace_is_deterministic(&replay_result);
    }
}
