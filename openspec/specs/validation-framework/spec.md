# validation-framework Specification

## Purpose
Defines the five-layer validation architecture for the tze_hud runtime: scene graph assertions (Layer 0), headless render and pixel readback (Layer 1), visual regression via SSIM (Layer 2), compositor telemetry and performance validation (Layer 3), and developer visibility artifacts (Layer 4). Establishes the test scene registry, design requirements, hardware-normalized calibration harness, latency and performance budgets, protocol conformance tests, record/replay traces, soak and leak tests, and the autonomous LLM development loop that allows models to iterate on implementation without human intervention.
## Requirements
### Requirement: Five Validation Layers
The validation architecture SHALL implement five ordered layers, from cheapest/most deterministic to richest/most human-oriented: Layer 0 (scene graph assertions), Layer 1 (headless render and pixel readback), Layer 2 (visual regression via SSIM), Layer 3 (compositor telemetry and performance validation), and Layer 4 (developer visibility artifacts). Each layer SHALL catch a different class of problem.
Source: validation.md §"Five validation layers"
Scope: v1-mandatory

#### Scenario: Layer ordering
- **WHEN** a developer runs the full validation suite
- **THEN** layers MUST be executable in order from 0 to 4, with each successive layer depending on the previous layers' infrastructure

---

### Requirement: Layer 0 - Scene Graph Assertions
Layer 0 SHALL consist of pure logic tests on the scene data structure with no rendering and no GPU. Tests SHALL be standard `#[test]` functions. Layer 0 SHALL validate: tile CRUD, z-order consistency, lease state machine, tab transitions, sync group invariants, capability negotiation, hit testing, resource budgets, mobile degradation policy, coalescing logic, zone registry operations, zone type validation, zone contention policies, and zone geometry policy resolution. Assertions SHALL be structural. Property-based testing SHALL generate random scene configurations and verify invariants. Layer 0 SHALL run in under 2 seconds, be fully deterministic with no external dependencies, and cover 60%+ of test cases.
Source: validation.md §"Layer 0: Scene graph assertions"
Scope: v1-mandatory

#### Scenario: Layer 0 execution time
- **WHEN** `cargo test` runs all Layer 0 tests
- **THEN** the full Layer 0 suite MUST complete in under 2 seconds

#### Scenario: Property-based invariant verification
- **WHEN** property-based testing generates 10,000 random scene configurations
- **THEN** every configuration MUST satisfy: tiles within bounds, lease state machine valid, z-order total ordering, budgets non-negative

#### Scenario: No GPU dependency
- **WHEN** Layer 0 tests run on a machine with no GPU
- **THEN** all Layer 0 tests MUST pass; they require no rendering or GPU context

---

### Requirement: Layer 1 - Headless Render and Pixel Readback
Layer 1 SHALL render test scenes to offscreen textures, read back pixels, and assert on content. Layer 1 SHALL validate: region color matching (rectangular region matches expected color within tolerance), structural assertions (edges, alpha, resolution), test patterns (solid colors, gradients, checkerboards with exact values), z-order (overlap region matches higher-z tile color), and blending (semi-transparent overlay produces predictable blended color). Media tiles SHALL use synthetic frame sources. Software GPU tolerance SHALL be +/-2 per channel for blending and +/-1 for solid fills.
Source: validation.md §"Layer 1: Headless render and pixel readback"
Scope: v1-mandatory

#### Scenario: Region color assertion
- **WHEN** a test scene renders a solid red tile at known bounds
- **THEN** pixel readback of that region MUST match red (255, 0, 0, 255) within +/-1 per channel on software GPU

#### Scenario: Z-order pixel verification
- **WHEN** two tiles overlap with different z-orders and different colors
- **THEN** the overlap region's pixel color MUST match the higher-z tile's color

#### Scenario: Alpha blending tolerance
- **WHEN** a semi-transparent overlay (50% alpha white) is rendered over a solid blue tile on a software GPU
- **THEN** the blended color MUST match the expected blend value within +/-2 per channel

---

### Requirement: Layer 2 - Visual Regression via SSIM
Layer 2 SHALL compare rendered output against golden reference images using SSIM (Structural Similarity Index), not pixel-exact comparison. Thresholds SHALL be: 0.995 for layout tests and 0.99 for media composition tests. Perceptual hash SHALL be used for fast pre-screening. On failure, the system SHALL produce: per-region SSIM, diff heatmap, and structured JSON. Golden references SHALL be committed to the repository, named by scene and backend. Goldens SHALL be regenerated only when rendering intentionally changes.
Source: validation.md §"Layer 2: Visual regression via perceptual comparison"
Scope: v1-mandatory

#### Scenario: Layout SSIM threshold
- **WHEN** a rendered scene is compared to its golden reference for a layout test
- **THEN** an SSIM score of 0.996 MUST pass and an SSIM score of 0.993 MUST fail

#### Scenario: Structured failure output
- **WHEN** a visual regression test fails (SSIM below threshold)
- **THEN** the output MUST include per-region SSIM scores, a diff heatmap image, and structured JSON failure details

#### Scenario: Golden references in repository
- **WHEN** a new test scene is added to the registry
- **THEN** its golden reference image MUST be committed to the repository, named by scene and backend

---

### Requirement: Layer 3 - Compositor Telemetry and Performance Validation
Layer 3 SHALL emit per-frame structured telemetry including: frame timing (scene update, CPU render, GPU submit, present, total), throughput (tiles, draw calls, texture uploads, coalesced updates, dropped ephemerals), resources (texture memory, GPU buffers, active leases/streams), and correctness (lease violations, z-order conflicts, budget overruns, sync group drift). Per-session aggregates SHALL include: total frames, FPS, frame time at p50/p95/p99, latency breakdown at p50/p95/p99, peaks, and violation totals. A benchmark binary SHALL run scenarios headlessly and emit JSON.
Source: validation.md §"Layer 3: Compositor telemetry and performance validation"
Scope: v1-mandatory

#### Scenario: Per-frame telemetry record
- **WHEN** the compositor renders a frame
- **THEN** a machine-readable telemetry record MUST be emitted containing frame timing, throughput, resource counts, and correctness signals

#### Scenario: Benchmark JSON emission
- **WHEN** `cargo run --bin benchmark --features headless -- --emit telemetry.json` is executed
- **THEN** the benchmark MUST run test scenarios headlessly and produce a JSON file with the telemetry schema

---

### Requirement: Split Latency Budgets
The telemetry SHALL track three distinct latencies, not a single conflated "input-to-pixel" measurement: (1) input_to_local_ack (input event to local visual feedback, budget: p99 < 4ms, purely local), (2) input_to_scene_commit (input event to scene graph reflecting agent response, budget: p99 < 50ms for local agents), (3) input_to_next_present (input event to next rendered frame containing committed change, budget: p99 < 33ms at 60Hz, refresh-rate-dependent). These SHALL be reported separately.
Source: validation.md §"Latency budgets", v1.md §"V1 must prove" item 4
Scope: v1-mandatory

#### Scenario: input_to_local_ack budget
- **WHEN** a touch input event occurs and local visual feedback (press state, hover highlight) is rendered
- **THEN** the latency from input event to local ack MUST be under 4ms at p99

#### Scenario: input_to_next_present budget
- **WHEN** an agent commits a scene mutation in response to input
- **THEN** the latency from input event to the next rendered frame containing the change MUST be under 33ms at p99 (within two frames at 60Hz)

---

### Requirement: Performance Budgets
The following performance budgets SHALL be enforced and measured in normalized units: p99 frame time < 16.6ms (normalized), zero lease violations, zero budget overruns, sync drift < 500 microseconds, texture memory under budget. Performance metrics SHALL be tracked across runs so LLMs can see trends, not just pass/fail.
Source: validation.md §"Layer 3", v1.md §"V1 must prove" item 4
Scope: v1-mandatory

#### Scenario: Frame time budget
- **WHEN** the compositor renders frames under test load
- **THEN** p99 frame time MUST be under 16.6ms (hardware-normalized)

#### Scenario: Zero lease violations
- **WHEN** a benchmark or test scenario runs to completion
- **THEN** the telemetry MUST report zero lease violations

#### Scenario: Sync drift budget
- **WHEN** a sync group runs across two or more streams
- **THEN** sync group drift MUST be under 500 microseconds (measured as wall-clock delta between earliest and latest frame presentation within a sync group, in the compositor clock domain)

---

### Requirement: Layer 4 - Developer Visibility Artifacts
Layer 4 SHALL render timestamped visual artifacts to a results folder structured as: `test_results/{YYYYMMDD-HHmmss}-{branch}/`. The output SHALL include: `index.html` (self-contained browsable gallery with scene thumbnails, pass/fail badges, rendered vs golden with diff overlay, benchmark charts, status filter), `manifest.json` (machine-readable index with status, metrics, artifact paths), `scenes/{name}/` (rendered.png, golden.png, diff.png, telemetry.json, explanation.md), and `benchmarks/{name}/` (session telemetry, histograms). Artifacts SHALL be generated on every PR CI run, on any visual regression failure, on LLM request, and as nightly baselines.
Source: validation.md §"Layer 4: Developer visibility artifacts"
Scope: v1-mandatory

#### Scenario: PR CI artifact generation
- **WHEN** a CI run executes for a pull request
- **THEN** Layer 4 MUST generate the full artifact set including index.html, manifest.json, per-scene directories with rendered/golden/diff/telemetry, and benchmarks

#### Scenario: Machine-readable manifest
- **WHEN** Layer 4 artifacts are generated
- **THEN** manifest.json MUST contain an entry for every test scene with status (pass/fail), metrics, and paths to all artifact files

#### Scenario: Per-scene explanation
- **WHEN** a test scene's artifacts are generated
- **THEN** explanation.md MUST be auto-generated from scene registry metadata describing what the scene tests, what to look for, automated results, and changes since previous golden

---

### Requirement: Hardware-Normalized Calibration Harness
All performance metrics SHALL be normalized to hardware capability using a calibration vector, not a single scalar. All quantitative performance budgets across all specs (session-protocol, scene-graph, validation-framework, and any future specs) are expressed in hardware-normalized units. Raw timings alone are NOT sufficient for pass/fail determination of any performance budget. At the start of every benchmark run, three fixed calibration workloads SHALL execute: (1) Scene-graph CPU calibration (rapid tile mutations, no rendering, measures pure CPU throughput), (2) Fill/composition GPU calibration (fixed multi-tile scene with overlapping alpha-blended regions, measures GPU composition throughput), (3) Upload-heavy resource calibration (rapid texture-backed tile creation/update, measures texture upload throughput). Each SHALL produce a hardware factor. All subsequent measurements SHALL be reported both raw and normalized against the relevant factor. The calibration harness MUST be implemented and producing valid normalization factors BEFORE any performance budgets can be validated as pass/fail. Until calibration is operational, performance test results MUST be marked as "uncalibrated" and treated as informational warnings, not pass/fail determinations.
Source: validation.md §"Hardware-normalized performance"
Scope: v1-mandatory

#### Scenario: Three-workload calibration
- **WHEN** a benchmark run begins
- **THEN** all three calibration workloads MUST execute first, producing factors (e.g., {cpu: 0.8, gpu: 0.12, upload: 0.15} on a CI runner with llvmpipe)

#### Scenario: Normalized reporting
- **WHEN** a benchmark measurement is recorded
- **THEN** it MUST be reported both as a raw value and as a normalized value against the relevant calibration dimension

#### Scenario: Calibration stability
- **WHEN** calibration workloads are run on the same hardware across different code versions
- **THEN** calibration factors MUST be stable (they exercise renderer infrastructure, not application logic)

#### Scenario: Uncalibrated performance test produces warning
- **WHEN** a performance test runs without valid calibration data (calibration harness not yet implemented or calibration workloads failed to produce factors)
- **THEN** the test result MUST be marked as "uncalibrated" with a warning status, NOT reported as pass or fail; the structured output MUST include `{"status": "uncalibrated", "reason": "no valid calibration factors available", "raw_value": <measured>}` so that the result is visible but cannot be mistaken for a validated pass

---

### Requirement: Test Scene Registry
A test scene registry SHALL define named, versioned scene configurations shared across all five layers. Each scene SHALL define: scene graph, synthetic content, Layer 0 invariants, Layer 1 pixel expectations, Layer 2 golden, Layer 3 budgets, and Layer 4 explanation. The initial corpus SHALL include at least 25 named scenes covering: empty_scene, single_tile_solid, three_tiles_no_overlap, overlapping_tiles_zorder, overlay_transparency, tab_switch, lease_expiry, mobile_degraded, sync_group_media, input_highlight, coalesced_dashboard, max_tiles_stress, three_agents_contention, overlay_passthrough_regions, disconnect_reclaim_multiagent, privacy_redaction_mode, chatty_dashboard_touch, zone_publish_subtitle, zone_reject_wrong_type, zone_conflict_two_publishers, zone_orchestrate_then_publish, zone_geometry_adapts_profile, zone_disconnect_cleanup, policy_matrix_basic, and policy_arbitration_collision.
Source: validation.md §"Test scene registry"
Scope: v1-mandatory

#### Scenario: Scene determinism
- **WHEN** a registered test scene is rendered twice on the same hardware
- **THEN** both renders MUST produce identical output (all randomness seeded, time sources injectable, media inputs synthetic)

#### Scenario: Scene versioning
- **WHEN** a test scene definition is intentionally modified
- **THEN** the scene version MUST be incremented and its golden reference regenerated

---

### Requirement: DR-V1 - Scene Model Separable from Renderer
The scene graph MUST be a pure data structure: constructable, mutable, queryable, serializable, and assertable without any GPU context.
Source: validation.md §"Design requirements" DR-V1
Scope: v1-mandatory

#### Scenario: Scene graph without GPU
- **WHEN** Layer 0 tests construct, mutate, and query the scene graph
- **THEN** no GPU context, display server, or rendering infrastructure MUST be required

---

### Requirement: DR-V2 - Headless Rendering
The compositor MUST be capable of rendering a complete frame to an offscreen texture with no window, no display server, and no user interaction. Headless mode SHALL be feature-equivalent to windowed for scene composition. Activation SHALL be via feature flag or runtime config, never by forking the render pipeline.
Source: validation.md §"Design requirements" DR-V2
Scope: v1-mandatory

#### Scenario: Headless parity
- **WHEN** the same scene is rendered in headless mode and windowed mode
- **THEN** the composed output MUST be feature-equivalent (same layout, z-order, blending, content)

#### Scenario: Single render pipeline
- **WHEN** headless mode is activated
- **THEN** it MUST use the same render pipeline as windowed mode, toggled by configuration, not a separate code path

---

### Requirement: DR-V3 - Structured Telemetry
Every frame SHALL emit a machine-readable telemetry record. This SHALL serve as both the CI validation surface and the production observability surface. The same telemetry schema SHALL be used in CI (full sampling) and production (configurable sampling rate).
Source: validation.md §"Design requirements" DR-V3
Scope: v1-mandatory

#### Scenario: Every frame emits telemetry
- **WHEN** the compositor completes a frame
- **THEN** a structured telemetry record MUST be emitted (in CI at full rate, in production at configurable rate)

---

### Requirement: DR-V4 - Deterministic Test Scenes
A scene registry SHALL provide named, versioned configurations that produce reproducible output. All randomness SHALL be seeded. All time sources SHALL be injectable. All media inputs SHALL be synthetic.
Source: validation.md §"Design requirements" DR-V4
Scope: v1-mandatory

#### Scenario: Seeded randomness
- **WHEN** a test scene uses any random values
- **THEN** the random seed MUST be fixed and documented, producing identical output across runs

#### Scenario: Injectable time sources
- **WHEN** a test scene depends on timing (e.g., lease_expiry)
- **THEN** time sources MUST be injectable so the test can control time deterministically

---

### Requirement: DR-V5 - Trivial Headless Invocation
`cargo test --features headless` SHALL run the full test suite (Layers 0-2). No manual setup, no environment hacks, no external services required.
Source: validation.md §"Design requirements" DR-V5
Scope: v1-mandatory

#### Scenario: One-command test invocation
- **WHEN** a developer or LLM runs `cargo test --features headless`
- **THEN** all Layer 0, Layer 1, and Layer 2 tests MUST execute without any manual setup steps

---

### Requirement: DR-V6 - No Physical GPU Required for CI
The headless rendering path SHALL work on software GPU implementations: mesa llvmpipe on Linux, WARP on Windows, and hardware-backed macOS CI runners. Perceptual thresholds (SSIM, pixel tolerance) SHALL account for software/hardware rendering differences.
Source: validation.md §"Design requirements" DR-V6, v1.md §"V1 success criteria" item 5
Scope: v1-mandatory

#### Scenario: Linux CI on llvmpipe
- **WHEN** the test suite runs on a Linux CI runner with mesa llvmpipe (no physical GPU)
- **THEN** all headless tests MUST pass with appropriate software GPU tolerances

#### Scenario: Windows CI on WARP
- **WHEN** the test suite runs on a Windows CI runner using WARP
- **THEN** all headless tests MUST pass with appropriate software GPU tolerances

---

### Requirement: LLM Development Loop
The validation architecture SHALL support the autonomous LLM development loop: (1) cargo test for Layer 0 (< 2s, fix logic errors), (2) cargo test --features headless for Layers 1+2 (fix rendering, regenerate goldens), (3) benchmark binary with --emit for Layer 3 (parse JSON, fix regressions), (4) render-artifacts binary for Layer 4 (include summary.md in PR). Steps 1-3 SHALL need no human. Test failures SHALL produce structured, machine-readable output that an LLM can parse, reason about, and act on.
Source: validation.md §"The LLM development loop", v1.md §"V1 success criteria" item 2
Scope: v1-mandatory

#### Scenario: Diagnostic failure output
- **WHEN** a Layer 3 benchmark fails (e.g., p99 frame time exceeds budget)
- **THEN** the output MUST include: the metric name, actual value, budget value, regression percentage, the scene name, and the frame at which the budget was first exceeded

#### Scenario: LLM self-diagnosis
- **WHEN** a test fails with structured output
- **THEN** the output MUST be sufficient for an LLM to diagnose the root cause and fix it in 1-2 iterations for common failure modes, without looking at rendered frames

---

### Requirement: Protocol Conformance Tests
Protocol conformance tests SHALL cover the wire surface: gRPC schema validation, MCP JSON-RPC error structure (code + context + correction hint), version negotiation, unknown-field handling, and graceful rejection of oversized payloads. These tests SHALL operate at the protocol boundary, not the scene graph level.
Source: validation.md §"Protocol conformance tests"
Scope: v1-mandatory

#### Scenario: Invalid protobuf rejection
- **WHEN** a malformed protobuf message is sent to the gRPC scene mutation endpoint
- **THEN** the runtime MUST reject it with a structured error response, not crash

#### Scenario: Oversized payload rejection
- **WHEN** an oversized gRPC payload exceeding limits is received
- **THEN** it MUST be gracefully rejected with a diagnostic error, not cause a hang or crash

---

### Requirement: Record/Replay Traces
The runtime SHALL support recording sequences of scene mutations, agent events, input events, zone publishes, and timing data as structured traces. These traces SHALL be replayable deterministically against the scene graph for debugging and regression testing. Fuzzing discoveries that produce minimal reproducers SHALL become permanent regression tests via this mechanism.
Source: validation.md §"Record/replay traces"
Scope: v1-mandatory

#### Scenario: Trace capture and replay
- **WHEN** a timing-sensitive bug is reproduced
- **THEN** the developer or LLM MUST be able to capture a replay trace, iterate on a fix while replaying the same trace, until the bug is resolved

#### Scenario: Fuzzing reproducer to regression test
- **WHEN** a fuzzer discovers a crash or invariant violation and produces a minimal reproducer
- **THEN** the reproducer MUST be convertible to a permanent replay trace regression test

---

### Requirement: Soak and Leak Tests
Soak tests SHALL run hours-long sessions with repeated agent connects, disconnects, reconnects, lease grants, revocations, zone publishes, and content updates. Pass criteria: resource utilization at hour N SHALL be within 5% of resource utilization at hour 1 for the same steady-state workload. Any monotonic growth SHALL be a bug. After an agent disconnects and leases expire, its resource footprint MUST be zero.
Source: validation.md §"Soak and leak tests"
Scope: v1-mandatory

#### Scenario: No memory growth after hours
- **WHEN** a soak test runs for N hours with steady-state workload
- **THEN** memory usage, file descriptors, and scene graph size at hour N MUST be within 5% of hour 1

#### Scenario: Zero post-disconnect footprint
- **WHEN** an agent disconnects and its leases expire during a soak test
- **THEN** the agent's resource footprint (memory, textures, scene graph nodes) MUST reach exactly zero

---

### Requirement: V1 Success Criterion - Live Multi-Agent Presence
V1 MUST demonstrate 3 LLM agents simultaneously holding tiles on a screen, updating content in real time, governed by leases, at 60fps.
Source: v1.md §"V1 success criteria" item 1
Scope: v1-mandatory

#### Scenario: Three-agent 60fps demonstration
- **WHEN** 3 concurrent resident agents hold tiles with active leases and update content
- **THEN** the compositor MUST render at 60fps with correct layout, z-order, and lease governance

---

### Requirement: V1 Success Criterion - Autonomous LLM Development Workflow
An LLM MUST be able to autonomously: write a new tile type, test it across all five validation layers, open a PR with developer visibility artifacts, and have the PR be mergeable. When a test fails, the LLM MUST diagnose the root cause from structured failure output alone and fix it in 1-2 iterations for common failure modes.
Source: v1.md §"V1 success criteria" item 2
Scope: v1-mandatory

#### Scenario: End-to-end LLM development cycle
- **WHEN** an LLM writes a new tile type and runs the validation suite
- **THEN** the LLM MUST be able to iterate from structured test output without looking at rendered frames, fixing failures in 1-2 iterations

---

### Requirement: V1 Success Criterion - Security Isolation
The security model MUST prevent a rogue agent from affecting other agents or the runtime. Agent isolation SHALL be validated.
Source: v1.md §"V1 success criteria" item 3
Scope: v1-mandatory

#### Scenario: Rogue agent containment
- **WHEN** a rogue agent attempts to modify, read, or interfere with another agent's tiles or leases
- **THEN** the runtime MUST prevent the interference completely; the rogue agent's actions MUST NOT affect other agents

---

### Requirement: V1 Success Criterion - MCP-Only Zone Publishing
An LLM with only MCP access MUST be able to publish a subtitle to a zone with one tool call and see it render correctly, with zero scene context and zero tile management.
Source: v1.md §"V1 success criteria" item 4
Scope: v1-mandatory

#### Scenario: Single-call zone publish
- **WHEN** an LLM calls publish_to_zone via MCP with subtitle content
- **THEN** the subtitle MUST render correctly in the zone with proper geometry and rendering policy, requiring no prior scene context

---

### Requirement: V1 Success Criterion - Headless CI on All Platforms
The entire compositor MUST run headlessly in CI on Linux (llvmpipe), Windows (WARP), and macOS (Metal) with no manual intervention.
Source: v1.md §"V1 success criteria" item 5
Scope: v1-mandatory

#### Scenario: Cross-platform headless CI
- **WHEN** CI runs on Linux, Windows, and macOS
- **THEN** all validation layers MUST execute headlessly on each platform without manual intervention or physical GPU requirements (except macOS which uses Metal on hardware-backed runners)

---

### Requirement: Test Honesty and Stability
Tests MUST be honest (not too easy to pass, not gameable by overfitting), stable (deterministic, no flakiness, time-dependent tests use injectable clocks, order-dependent tests are bugs), and diagnostic (failures produce actionable structured output including metric name, actual value, budget, regression percentage, scene name, and frame number).
Source: validation.md §"Tests as the engine of recursive self-improvement"
Scope: v1-mandatory

#### Scenario: No flaky tests
- **WHEN** any test is run 100 times on the same hardware and code
- **THEN** it MUST produce the same result every time; any non-determinism is a bug

#### Scenario: Actionable failure output
- **WHEN** a performance test fails
- **THEN** the output MUST NOT be a bare "assertion failed"; it MUST include the metric, actual value, budget, regression amount, scene name, and identification of where the failure occurred

---

### Requirement: Fuzzing Scene Graph and Protocol Boundaries
Scene graph fuzzing SHALL feed the mutation API random operation sequences and verify invariants hold after every operation (no crash, no inconsistent state, no leaked resources). Protocol boundary fuzzing SHALL feed gRPC and MCP entry points malformed, oversized, out-of-order, and adversarial messages. The system MUST reject invalid input without crashing, hanging, or corrupting state.
Source: validation.md §"Fuzzing and chaos testing"
Scope: v1-mandatory

#### Scenario: Scene graph fuzz invariants
- **WHEN** a fuzzer generates 100,000 random scene mutation sequences (create, delete, resize, reparent, z-order change, lease grant/revoke)
- **THEN** the scene graph MUST never crash, never reach an internally inconsistent state, and never leak resources

#### Scenario: Protocol boundary fuzz safety
- **WHEN** malformed or adversarial messages are sent to gRPC or MCP endpoints
- **THEN** the runtime MUST reject them without crashing, hanging, or corrupting state

---

### Requirement: API Quality as Tested Property
Tests SHALL read like usage examples. If a test is awkward to write (excessive setup, deep knowledge of internals, fragile ordering, opaque types), that is a signal the API is poorly designed. When testing requires ugly workarounds, the API SHOULD be improved rather than writing a more complex test.
Source: validation.md §"API quality as a tested property"
Scope: v1-mandatory

#### Scenario: Test as API quality signal
- **WHEN** an LLM struggles to write a correct test for a public API
- **THEN** the API design MUST be reviewed and improved; the test difficulty is a signal of poor API ergonomics, not a test problem

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

---

### Requirement: Layer 3 — External Endpoint Validation
In addition to internal compositor telemetry benchmarks, Layer 3 SHALL include external MCP HTTP endpoint stress testing as a complementary validation dimension. The MCP stress test tool (see `mcp-stress-testing` capability spec) provides network-facing latency and throughput characterization that cannot be observed from internal telemetry alone.

The MCP stress test results SHALL be includable in Layer 4 artifacts (`benchmarks/mcp-stress/`) alongside internal compositor benchmarks, using the same JSON report format conventions.

**Note:** Layer 4 artifact generation is not yet implemented. The integration below is aspirational — the stress test JSON report is self-contained and useful standalone. Layer 4 integration will be implemented when the artifact pipeline is built.

#### Scenario: MCP stress results included in Layer 4 artifacts (aspirational)
- **WHEN** Layer 4 artifacts are generated after an MCP stress test run
- **THEN** the stress test JSON report SHALL be copied to `benchmarks/mcp-stress/` in the artifact output directory
- **AND** the `manifest.json` SHALL reference the MCP stress report

