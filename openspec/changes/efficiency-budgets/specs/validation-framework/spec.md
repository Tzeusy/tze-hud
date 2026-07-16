## ADDED Requirements

### Requirement: Efficiency Measurement Artifact
Layer 3 SHALL emit a deterministic efficiency measurement artifact for both quiescent and single-change scenarios. The artifact MUST identify the scenario and version, runtime build, window mode, pacing mode and requested cadence, renderer/backend, viewport, interval and settling durations, and measurement status. Quiescent records MUST include combined and per-loop runtime-driven event-loop wakeups with source attribution, GPU queue submissions, surface acquisitions, and presents. Change records MUST include typed per-category unique closure-work-item counts and actual-operation counts, layout-resolved nodes, rasterized nodes, texture-upload count and bytes, render-plan items encoded, total render-encoding operations, encoded draw calls, damaged-pixel area, viewport area, full-surface invalidation reason when applicable, and per-category amplification ratios. Missing required counters MUST fail the gate rather than default to zero.
Source: about/heart-and-soul/efficiency.md sections "Compute budget" and "Why this is doctrine and not tuning"; validation.md sections "Layer 3" and "Tests as the engine of recursive self-improvement"
Scope: v1-mandatory

#### Scenario: Quiescent artifact proves idle-zero-work budget
- **WHEN** the static-scene efficiency scenario completes its 5-second settling period and 60-second controlled measurement interval
- **THEN** the artifact MUST fail unless GPU submissions, surface acquisitions, and presents are exactly 0 and the combined runtime-driven main-plus-compositor wakeup count is no greater than 120, and its structured result MUST include actual values, ceilings, combined and per-loop wakeup counts, wakeup sources, pacing mode, and interval duration

#### Scenario: Single-change artifact proves closure-scoped work
- **WHEN** the canonical single-node change scenario completes in a scene containing unrelated tiles
- **THEN** the artifact MUST fail if any unchanged out-of-closure node, tile, region, or render-plan item is laid out, rasterized, uploaded, or encoded, if damage exceeds the closure bounds, or if any actual-operation count exceeds its corresponding unique typed closure-work-item count; a structured full-surface reason MUST instead mark the result diagnostic and MUST NOT satisfy this closure-scoped gate

#### Scenario: Required efficiency field is missing
- **WHEN** an efficiency measurement omits a required counter, identity field, closure field, or full-surface reason
- **THEN** the gate MUST report an invalid artifact and MUST NOT infer a pass from an absent or defaulted value

### Requirement: Canonical LLM Flow Byte and Token Calibration
Layer 3 SHALL measure the request, response, and total UTF-8 byte and token footprints of three versioned canonical LLM flows: (1) one MCP `publish_to_zone` call carrying the canonical zone fixture; (2) one text-stream portal turn consisting of `portal_projection_attach`, `portal_projection_publish`, one long-poll `portal_projection_get_pending_input`, and `portal_projection_acknowledge_input`; and (3) one MCP `publish_to_widget` call carrying the canonical widget fixture. The fixture set MUST pin canonical content, JSON-RPC framing, operation order, IDs, timestamps, and deterministic authority responses. Dynamic credentials or secrets MUST NOT be captured; variable-width dynamic values MUST be replaced before measurement by fixed canonical sentinels of the same schema role. Measurement SHALL include serialized JSON-RPC message bodies and exclude transport headers and bearer credentials. Token counts MUST use a repository-pinned tokenizer name, version, vocabulary fingerprint, and explicit counting policy. Each artifact MUST report per-operation and per-flow request bytes, response bytes, total bytes, request tokens, response tokens, and total tokens, plus a canonical-flow fingerprint.
Source: about/heart-and-soul/efficiency.md section "Token budget"; openspec/specs/cooperative-hud-projection/spec.md; openspec/specs/session-protocol/spec.md requirement "MCP Guest Tool Surface"; openspec/specs/widget-system/spec.md
Scope: v1-mandatory

#### Scenario: Zone publish footprint is deterministic
- **WHEN** the canonical `publish_to_zone` fixture is measured twice with the same runtime build, tokenizer identity, and flow version
- **THEN** the serialized request and response bytes and tokens MUST be identical across both runs and MUST be reported separately and as totals

#### Scenario: Portal conversation turn counts every semantic operation
- **WHEN** the canonical text-stream portal turn is measured
- **THEN** the artifact MUST include exactly one attach, one append-only output publish, one bounded long-poll input retrieval, and one input acknowledgement, with per-operation and aggregate request/response byte and token counts

#### Scenario: Widget publish footprint is deterministic
- **WHEN** the canonical `publish_to_widget` fixture is measured twice with the same runtime build, tokenizer identity, and flow version
- **THEN** the serialized request and response bytes and tokens MUST be identical across both runs and MUST be reported separately and as totals

#### Scenario: Tokenizer or fixture drift invalidates comparison
- **WHEN** the tokenizer name, tokenizer version, vocabulary fingerprint, canonical-flow version, or canonical-flow fingerprint differs from the approved baseline
- **THEN** the result MUST be marked `baseline_incompatible` and MUST NOT be reported as a pass or regression against that baseline

### Requirement: Canonical LLM Flow Regression Gate
Each canonical LLM flow SHALL have a checked-in owner-approved baseline containing the flow version and fingerprint, tokenizer identity, and every per-operation and per-flow request/response/total byte and token value emitted by the calibration artifact. On a compatible measurement, the gate MUST compare every emitted byte and token value with its matching baseline value. An increase greater than 5 percent in any compared value MUST fail the gate. An increase greater than 0 and no greater than 5 percent MUST pass only with a structured regression warning showing the absolute and percentage delta. A decrease SHALL pass and be reported as an improvement. An intentional schema, fixture, or tokenizer change requires a newly versioned candidate baseline and explicit owner approval before it can become the comparison authority; a missing or unapproved baseline MUST fail closed.
Source: about/heart-and-soul/efficiency.md section "Token budget"; validation.md section "Tests as the engine of recursive self-improvement"; about/craft-and-care/engineering-bar.md section 2
Scope: v1-mandatory

#### Scenario: More than five-percent token regression fails
- **WHEN** any compatible canonical flow request, response, or total token count is more than 5 percent above its approved baseline
- **THEN** the gate MUST fail and report the flow, operation or total, baseline count, measured count, absolute delta, and percentage delta

#### Scenario: More than five-percent byte regression fails
- **WHEN** any compatible canonical flow request, response, or total byte count is more than 5 percent above its approved baseline
- **THEN** the gate MUST fail and report the flow, operation or total, baseline count, measured count, absolute delta, and percentage delta

#### Scenario: Small regression remains visible
- **WHEN** a compatible byte or token count increases by more than 0 and no more than 5 percent
- **THEN** the gate SHALL pass with a structured regression warning containing the absolute and percentage delta rather than hiding the movement behind a silent green result

#### Scenario: Baseline update requires owner approval
- **WHEN** a flow schema, fixture, or tokenizer change produces a candidate baseline
- **THEN** the candidate MUST NOT replace the checked-in comparison authority or allow the gate to pass until its new version, rationale, and measured counts receive explicit owner approval

## MODIFIED Requirements

### Requirement: Hardware-Normalized Calibration Harness
All performance metrics SHALL be normalized to hardware capability using a calibration vector, not a single scalar. All quantitative performance budgets across all specs (session-protocol, scene-graph, validation-framework, and any future specs) are expressed in hardware-normalized units. Raw timings alone are NOT sufficient for pass/fail determination of any performance budget. At the start of every benchmark run, three fixed calibration workloads SHALL execute: (1) Scene-graph CPU calibration (rapid tile mutations, no rendering, measures pure CPU throughput), (2) Fill/composition GPU calibration (fixed multi-tile scene with overlapping alpha-blended regions, measures GPU composition throughput), (3) Upload-heavy resource calibration (rapid texture-backed tile creation/update, measures texture upload throughput). Each SHALL produce a hardware factor. All subsequent measurements SHALL be reported both raw and normalized against the relevant factor. The calibration harness MUST be implemented and producing valid normalization factors BEFORE any performance budgets can be validated as pass/fail. Until calibration is operational, performance test results MUST be marked as "uncalibrated" and treated as informational warnings, not pass/fail determinations.

In addition to the ordinary reference lane, the validation system MUST provide at least one gating constrained-envelope lane that runs the same versioned calibration vector and benchmark scenarios with a software renderer (WARP on Windows or llvmpipe on Linux) and an enforced limit of two logical CPUs available to the benchmark process. The constrained artifact MUST record operating system, CPU model, logical-CPU limit and enforcement mechanism, memory limit when one is imposed, renderer/backend, adapter identity, resolution, calibration-vector version, raw factors, and normalized results. The constrained lane SHALL enforce the same normalized performance ceilings as the reference lane; it MUST NOT introduce wider normalized ceilings merely because raw execution is slower. Missing profile identity, failure to enforce the CPU/renderer constraints, or invalid calibration MUST fail the constrained lane rather than silently falling back to an unconstrained run. This lane is a low-power proxy and MUST NOT be represented as a smart-glasses or VR device qualification.
Source: validation.md section "Hardware-normalized performance"; about/heart-and-soul/efficiency.md sections "Deployment trajectory" and "Compute budget"
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

#### Scenario: Constrained-envelope lane uses enforced low-power proxy
- **WHEN** the gating constrained-envelope benchmark lane starts
- **THEN** it MUST use WARP or llvmpipe, enforce a two-logical-CPU process limit, run the same versioned CPU/GPU/upload calibration vector as the reference lane, and record the complete constrained-profile identity and enforcement mechanism

#### Scenario: Constrained normalized ceiling matches reference
- **WHEN** the constrained-envelope lane produces valid calibration factors and benchmark measurements
- **THEN** each normalized result MUST be evaluated against the same normalized ceiling as the reference lane even though its raw timing is expected to be slower

#### Scenario: Constraint fallback fails closed
- **WHEN** the requested software renderer is unavailable, the two-logical-CPU limit is not enforced, required profile identity is missing, or constrained calibration is invalid
- **THEN** the constrained-envelope lane MUST fail with a structured reason and MUST NOT substitute an unconstrained or uncalibrated pass
