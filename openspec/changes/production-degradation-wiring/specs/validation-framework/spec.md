## ADDED Requirements

### Requirement: Production Degradation Telemetry
Every successfully presented frame MUST emit the applied runtime degradation level and `degradation_work_time_us` in the production `FrameTelemetry` schema with backward-compatible defaults. Every transition MUST emit a structured event containing previous level, new level, direction, triggering p95, sample count, elapsed window duration, effective cadence, entry/recovery thresholds, and whether recovery was active-frame or quiescent. Windowed and headless paths MUST populate the same semantic metric from active work before Stage 3 through Stage 7 completion.

#### Scenario: N-to-N+1 telemetry ordering
- **WHEN** frame N causes the controller to select a new level
- **THEN** frame N telemetry MUST report the policy applied to N and frame N+1 telemetry MUST report the newly applied policy

#### Scenario: Backward-compatible telemetry decode
- **WHEN** a telemetry record produced before the degradation fields existed is deserialized
- **THEN** the new metric and applied level MUST default to zero/Normal without error

#### Scenario: Structured quiescent transition
- **WHEN** a full quiescent recovery duration causes a recovery transition
- **THEN** the transition event MUST identify quiescent recovery and MUST NOT be accompanied by a synthetic frame record

### Requirement: Deterministic Degradation Validation
Validation MUST use an injected monotonic clock and deterministic cadence. Tests MUST cover transient spikes, sustained entry, one-level transition, hysteresis, sample reset, quiescent recovery, full Level-5 recovery, stable-SceneId suppression, exact protocol mapping, bounded queue backpressure, snapshot/current-state ordering, and headless/windowed workload-metric parity. The named sustained-load payload MUST also run in release mode with a timing assertion and structured output.

#### Scenario: Sustained-load evidence
- **WHEN** the production-path degradation payload is run in release mode
- **THEN** it MUST prove the expected transition within the cadence-derived deadline and emit machine-readable timing evidence
