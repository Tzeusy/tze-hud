# Validation Framework — Delta: Publish Load Benchmarks

## ADDED Requirements

### Requirement: Layer 3 Publish-Load Benchmark Evidence
Layer 3 SHALL support resident publish-load benchmark evidence as a distinct benchmark class alongside compositor frame/session telemetry. A publish-load benchmark artifact SHALL record benchmark identity, raw latency and throughput metrics, byte-accounting mode, success/error counts, threshold fields, traceability references, calibration status, and any benchmark-specific warnings. Publish-load evidence SHALL NOT be forced into compositor frame-session summaries when its semantics differ.

#### Scenario: Publish-load artifact captured separately from frame-session telemetry
- **WHEN** a resident publish-load benchmark run completes
- **THEN** the emitted artifact SHALL be stored and evaluated as a publish-load benchmark artifact rather than masquerading as a compositor frame-session summary

#### Scenario: Benchmark artifact includes audit fields
- **WHEN** a publish-load benchmark artifact is generated
- **THEN** it SHALL include benchmark identity, threshold fields, traceability references, and calibration status in addition to its raw metrics

### Requirement: Layer 4 Artifact Inclusion for Publish-Load Benchmarks
Layer 4 SHALL support benchmark artifact directories for publish-load runs. Each generated publish-load artifact set SHALL include a canonical result file and any auxiliary histogram or hardware/calibration files that the benchmark produced. The Layer 4 manifest SHALL reference the publish-load artifact set just as it does other benchmark outputs.

#### Scenario: Layer 4 stores publish-load benchmark outputs
- **WHEN** Layer 4 artifacts are generated for a publish-load benchmark run
- **THEN** the result directory SHALL contain the canonical publish-load JSON artifact and any companion histogram or calibration files emitted by the benchmark

#### Scenario: Manifest references publish-load artifact set
- **WHEN** the Layer 4 manifest is written
- **THEN** it SHALL include an entry for each publish-load benchmark artifact set with paths to the generated files

### Requirement: Publish-Load Calibration Status Semantics
Publish-load benchmarks SHALL distinguish raw evidence from calibrated validation verdicts. If a publish-load benchmark lacks an approved normalization mapping or valid calibration inputs for the measured dimension, the benchmark SHALL be marked `uncalibrated` and its threshold comparisons SHALL be informational only. If a publish-load benchmark has an approved normalization mapping and valid calibration inputs, it SHALL report both raw and normalized values and MAY emit a formal pass/fail verdict.

#### Scenario: Uncalibrated remote publish benchmark
- **WHEN** a remote publish-load benchmark runs without an approved normalization mapping for that benchmark class
- **THEN** the result SHALL be marked `uncalibrated`, raw metrics SHALL still be emitted, and any threshold comparisons SHALL be labeled informational

#### Scenario: Calibrated publish benchmark result
- **WHEN** a publish-load benchmark has valid calibration inputs and an approved normalization mapping
- **THEN** the result SHALL include both raw and normalized values and MAY report a formal pass/fail outcome
