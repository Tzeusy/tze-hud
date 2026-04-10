## Context

The repository doctrine is explicit: MCP is the compatibility perimeter, while resident agents use one primary bidirectional gRPC session stream on the hot path. Validation is also a first-class concern: machine-readable evidence, tracked across runs, with calibrated interpretation for any formal performance claim. The current Python gRPC harness already proves the basic session/bootstrap/publish loop, but it cannot produce trustworthy per-request RTT for repeated widget publishes because `WidgetPublishResult` does not currently echo the originating client sequence in the runtime-facing contract.

The repository already has the main ingredients needed for a Rust harness: `tze_hud_protocol` owns the session protocol and generated protobuf client types, `examples/benchmark` proves the workspace accepts benchmark binaries as first-class members, and `tze_hud_validation` already owns Layer 4 benchmark artifact directories. What is missing is a dedicated benchmark contract for publish-load runs and one protocol repair so request-level measurement is not ambiguous.

## Goals / Non-Goals

**Goals:**
- Add a narrow, honest Rust benchmark harness for resident gRPC widget publishing on the same target class used by `/user-test`
- Measure throughput, error rate, payload-byte counts, and per-request RTT percentiles on one multiplexed session stream
- Emit canonical JSON artifacts plus stable summary rows for historical comparison
- Preserve doctrinal honesty by distinguishing calibrated verdicts from raw informational numbers
- Reuse existing repo infrastructure where it is authoritative: session protocol client types, Layer 4 benchmark artifact sinks, and the existing target registry / historical-ledger workflow

**Non-Goals:**
- Replacing `examples/benchmark` or folding publish-load metrics into compositor frame-session telemetry
- Building a generic distributed load platform, dashboard, or multi-target orchestration service in v1
- Expanding first delivery to MCP, zones, tiles, or every widget type
- Claiming formal pass/fail validation for remote network-path latency before a defensible normalization model exists

## Decisions

### Use a new workspace example crate
**Decision:** Implement the harness as a new example workspace member rather than extending `examples/benchmark` or adding CLI logic to a core crate.
**Rationale:** `examples/benchmark` is a headless runtime/render benchmark with different telemetry semantics. `tze_hud_protocol` and `tze_hud_validation` should stay reusable libraries, not become benchmark CLIs. A separate example crate keeps dependency growth bounded while staying close to existing benchmark workflows.
**Alternatives considered:**
- Extending `examples/benchmark`: rejected because publish-stream metrics are not the same as frame-session metrics.
- Adding a binary to `crates/tze_hud_protocol`: rejected because it would mix product protocol ownership with benchmark orchestration.

### Require request-sequence correlation for durable widget publish results
**Decision:** Modify the session protocol so `WidgetPublishResult` includes `request_sequence`, echoed from the client envelope sequence.
**Rationale:** Without that field, repeated publishes to the same widget on one stream cannot be paired to the correct acknowledgement, so per-request RTT percentiles are not trustworthy. The RFC already expects this field; the implementation and current OpenSpec are the drifting pieces.
**Alternatives considered:**
- Aggregate-only benchmarking: rejected because it hides queueing behavior and cannot support honest percentile latency claims.
- One widget instance per request: rejected because it distorts the real workload and introduces extra registry/state noise.

### Make canonical output JSON artifacts, with CSV as a derived compatibility layer
**Decision:** The Rust harness writes canonical machine-readable JSON artifacts into the Layer 4 benchmark directory and may also append a stable summary row to the historical CSV used by `/user-test-performance`.
**Rationale:** The repo already has an artifact model for benchmarks; that should stay the canonical source. The CSV remains valuable for cheap trend tracking and version-controlled regressions, but it should derive from the structured result rather than define the contract alone.
**Alternatives considered:**
- CSV-only output: rejected because it loses histogram and provenance detail.
- Artifact-only output with no summary row: rejected because it would break the existing `/user-test-performance` audit workflow.

### Separate raw metrics from formal validation verdicts
**Decision:** Every run reports raw metrics. Formal pass/fail is only allowed when an approved normalization mapping exists for the measured dimension; otherwise the run status is `uncalibrated` and threshold comparisons are informational.
**Rationale:** Current hardware calibration models CPU/GPU/upload factors but not remote network-path variance. The doctrine already requires explicit `uncalibrated` status in that case.
**Alternatives considered:**
- Treating raw network latency thresholds as authoritative: rejected because it violates existing validation doctrine.
- Blocking all remote runs until network normalization exists: rejected because it would prevent useful historical measurement.

### Reuse the existing target registry and audit vocabulary
**Decision:** The harness uses the same target-registry and benchmark-identity concepts already established in `.claude/skills/user-test-performance`.
**Rationale:** The user already asked for consistent-per-primary-key results across targets over time. Reusing the same target ids, network scopes, and traceability fields avoids a second audit vocabulary.
**Alternatives considered:**
- Embedding target config into the binary: rejected because it weakens portability and auditability.

## Risks / Trade-offs

- **[Protocol drift repair touches multiple surfaces]** Adding `request_sequence` requires updates to proto, runtime handling, tests, and delta specs. → Mitigation: scope the protocol change to durable widget publish acknowledgements only and land it before the harness.
- **[Byte-accounting ambiguity]** Payload bytes are easy to measure; full wire bytes may not be available without additional transport instrumentation. → Mitigation: require explicit `byte_accounting_mode` and make payload-byte accounting the minimum guaranteed metric.
- **[Remote normalization is unresolved]** Current calibration factors do not model network-path variance. → Mitigation: mark remote runs `uncalibrated` until a benchmark-specific normalization mapping is approved.
- **[Wrapper integration churn]** The current `/user-test-performance` skill already records results and comparisons. → Mitigation: preserve its CSV schema and target registry, and switch only the gRPC widget path to the Rust binary first.
- **[Scope creep]** Expanding immediately to zones, tiles, MCP, or dashboards would dilute the first honest benchmark. → Mitigation: spec the initial tranche as gRPC `WidgetPublish` only on the existing primary target.

## Migration Plan

1. Land the OpenSpec change and reconcile protocol / validation semantics.
2. Add `request_sequence` to `WidgetPublishResult` and update runtime/tests.
3. Add the Rust example crate with CLI, target resolution, single-stream publish executor, and JSON artifact emission.
4. Add CSV summary compatibility and switch `/user-test-performance` gRPC benchmarking to the Rust binary.
5. Add verification coverage, artifact validation, and reconciliation reporting before widening scope.

Rollback is straightforward: the harness crate and skill integration can be reverted independently. The protocol change is additive at the message-field level and can remain if the harness rollout is postponed.

## Open Questions

- Should wire-byte accounting be added in v1, or is payload-byte accounting plus explicit mode labeling sufficient for first release?
- Does publish-load validation eventually need its own normalization model, or should remote publish runs remain permanently informational while only local resident benchmarks become calibrated?
- Should the historical CSV live solely under `.claude/skills/user-test-performance/reference/results.csv`, or should the Rust harness also support a first-party benchmark-ledger path under `test_results/`?
