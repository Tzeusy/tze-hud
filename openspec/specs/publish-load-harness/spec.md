# publish-load-harness Specification

## Purpose
TBD - created by archiving change rust-widget-publish-load-harness. Update Purpose after archive.
## Requirements
### Requirement: Rust Resident Publish Harness
The repository SHALL provide a Rust benchmark executable for resident-agent publish-load measurement. The executable SHALL connect to a configured target over one bidirectional gRPC session stream, perform the same session bootstrap, authentication, and capability-grant flow used by resident agents, and exercise durable widget publishing using `WidgetPublish` on that stream. The initial v1 scope SHALL support only gRPC widget publishing against the same primary target class used by `/user-test-performance`; MCP, zones, tiles, and multi-target orchestration SHALL remain out of scope.

#### Scenario: Single-stream durable widget publish run
- **WHEN** an operator runs the harness against a valid target and widget instance
- **THEN** the harness SHALL establish exactly one session stream, send the configured series of durable `WidgetPublish` messages on that stream, and collect the corresponding durable acknowledgements on that same stream

#### Scenario: Resident bootstrap path is exercised honestly
- **WHEN** the harness opens a benchmark session
- **THEN** it SHALL use the same session bootstrap, authentication, and capability-grant flow required of resident agents rather than a benchmark-only bypass path

#### Scenario: Future-scope transports are rejected explicitly
- **WHEN** an operator requests MCP, zone, or tile benchmark mode from the initial harness release
- **THEN** the harness SHALL fail fast with a clear unsupported-mode error rather than silently approximating those workloads

### Requirement: Target Registry and Bootstrap Contract
The harness SHALL resolve connection and identity information from a target registry keyed by `target_id`. The recorded run metadata SHALL include at minimum: `target_id`, `target_host`, `network_scope`, `transport`, `mode`, `widget_name`, `payload_profile`, and the benchmark's stable comparison key. No credentials or pre-shared secrets SHALL be hardcoded; authentication inputs SHALL come from environment variables or operator-supplied configuration.

#### Scenario: Target resolved by stable id
- **WHEN** an operator supplies `target_id=user-test-windows-tailnet`
- **THEN** the harness SHALL resolve the target's connection parameters and record that same `target_id` in all emitted artifacts and summary rows

#### Scenario: Missing credential configuration
- **WHEN** a required credential or auth input is absent
- **THEN** the harness SHALL exit with a configuration error before opening the session and SHALL not fall back to a hardcoded secret

### Requirement: Paced and Burst Workload Modes
The harness SHALL support at least two workload modes in its initial release: `burst` and `paced`. `burst` mode SHALL send a fixed publish count as fast as the target path will accept on one session stream. `paced` mode SHALL attempt a configured target rate on one session stream for a bounded duration or publish count. The emitted benchmark identity and result artifact SHALL record which mode was used and the controlling workload parameters for that mode.

#### Scenario: Burst mode records count-limited throughput
- **WHEN** an operator runs the harness in `burst` mode
- **THEN** the harness SHALL send the configured fixed publish count without introducing artificial pacing and SHALL report the achieved throughput for that completed burst

#### Scenario: Paced mode records target-rate context
- **WHEN** an operator runs the harness in `paced` mode with a target rate
- **THEN** the harness SHALL record the requested target rate, the achieved throughput, and the duration or publish-count bound used for the run

### Requirement: Stable Benchmark Identity and Historical Comparison Fields
Every harness run SHALL emit a stable benchmark identity that is suitable for comparison across time. The identity SHALL include enough fields to distinguish materially different workloads while remaining stable for repeated runs of the same workload. At minimum the identity SHALL include: `target_id`, `transport`, `mode`, `widget_name`, `payload_profile`, `publish_count` or `duration_s`, `target_rate_rps` when paced, and `network_scope`. The harness SHALL also record traceability references such as `spec_id`, `rfc_id`, and budget/threshold identifiers when provided.

#### Scenario: Repeated run produces same comparison key
- **WHEN** two harness runs use the same target, transport, mode, widget, payload profile, and workload shape
- **THEN** both runs SHALL emit the same stable comparison key even though their timestamps differ

#### Scenario: Workload change produces different comparison key
- **WHEN** an operator changes a material workload dimension such as widget name, payload profile, or target rate
- **THEN** the emitted comparison key SHALL also change

### Requirement: Per-Request RTT and Aggregate Publish Metrics
The harness SHALL compute both per-request and aggregate publish metrics. Required metrics SHALL include: request count, success count, error count, wall duration, throughput, per-request RTT percentiles (p50/p95/p99/max), aggregate send time, aggregate acknowledgement-drain time, and byte totals for outbound and inbound publish-related payloads. Per-request RTT SHALL be computed by correlating each durable `WidgetPublishResult` to the originating client envelope sequence.

#### Scenario: Repeated publishes to one widget still yield RTT percentiles
- **WHEN** a run sends multiple durable publishes to the same widget instance on one session stream
- **THEN** the harness SHALL still compute correct per-request RTT percentiles by correlating acknowledgements using `request_sequence`

#### Scenario: Aggregate throughput reported for burst mode
- **WHEN** an operator runs the harness in burst mode for a fixed publish count
- **THEN** the harness SHALL report total wall duration and achieved throughput in publishes per second for the completed burst

### Requirement: Explicit Byte Accounting Semantics
The harness SHALL label byte measurements with an explicit accounting mode. `payload_bytes_out` and `payload_bytes_in` SHALL be mandatory and SHALL refer to serialized protobuf payload bytes. `wire_bytes_out` and `wire_bytes_in` MAY be reported when transport-level accounting is available, but only when the emitted `byte_accounting_mode` makes that distinction explicit.

#### Scenario: Payload-only accounting
- **WHEN** the harness only has protobuf-size information available
- **THEN** it SHALL emit payload byte totals and `byte_accounting_mode="payload_only"`

#### Scenario: Wire-byte accounting advertised only when real
- **WHEN** the harness emits wire-byte totals
- **THEN** the artifact SHALL also state a byte-accounting mode that distinguishes those totals from protobuf payload bytes

### Requirement: Artifact Output and CSV Compatibility
The harness SHALL emit a canonical machine-readable JSON result artifact for each run. The artifact SHALL be suitable for inclusion in Layer 4 benchmark directories and SHALL contain: benchmark identity, raw metrics, byte-accounting mode, threshold fields, traceability fields, calibration status, warnings, and links or paths to any auxiliary histogram data. The harness SHALL also support emitting or appending a summary row compatible with the historical CSV ledger used by `/user-test-performance`.

#### Scenario: Canonical JSON artifact emitted
- **WHEN** a harness run completes
- **THEN** the run SHALL produce a JSON artifact containing all benchmark identity and metric fields needed for later audit and comparison

#### Scenario: Historical ledger compatibility
- **WHEN** CSV output is requested
- **THEN** the harness SHALL append a row whose identity and summary fields are compatible with the established `/user-test-performance` historical ledger

### Requirement: Calibration Status and Verdict Semantics
Every run SHALL report both raw metrics and calibration status. Formal pass/fail verdicts SHALL only be emitted when an approved normalization mapping exists for the measured publish benchmark. Until then, remote publish-load runs SHALL be marked `uncalibrated`, and any threshold comparisons SHALL be informational rather than authoritative validation outcomes.

#### Scenario: Remote run without approved normalization mapping
- **WHEN** a publish-load run measures a remote resident target and no approved normalization mapping exists for that benchmark class
- **THEN** the emitted status SHALL be `uncalibrated`, the raw metrics SHALL still be reported, and threshold comparisons SHALL be labeled informational

#### Scenario: Calibrated publish benchmark verdict
- **WHEN** a future publish benchmark class has an approved normalization mapping and valid calibration inputs
- **THEN** the harness SHALL emit both raw and normalized values and MAY emit a formal pass/fail verdict for that run

