# Rust Widget Publish Load Harness Direction Report

Date: 2026-04-10
Scope: `/project-direction` planning package for a Rust resident publish-load harness
Status: Planned and locally validated; not pushed

## Executive summary

[Observed] The project is trying to prove that an agent-native HUD can be a real-time, resident, hot-path system rather than a loose collection of remote API calls. The architecture centers the resident gRPC session stream and treats validation as a first-class product surface, not an afterthought. See [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md:31) and [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md:3).

[Observed] The current implementation is directionally ready for a Rust harness, but the truth is narrower than the existing Python tooling suggests. The repo already supports the gRPC widget publish path and already has artifact infrastructure, yet the current contract cannot honestly correlate repeated durable widget publishes because `WidgetPublishResult` does not carry `request_sequence` in the implementation-facing spec and proto. See [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto:563), [session_server.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/src/session_server.rs:4481), and [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0005-session-protocol.md:600).

[Inferred] The highest-priority next work is therefore not “write the Rust harness” in isolation. It is a three-step sequence: repair request correlation in the session contract, define the harness spec and validation semantics, then implement a narrow Rust example crate and route `/user-test-performance` through it. Anything broader before that would be planning theater rather than auditable validation infrastructure.

## Project Spirit

**Core problem**: The project needs honest, machine-readable evidence for the resident publish hot path so performance regressions and transport bottlenecks are measurable over time rather than guessed.
**Primary user**: Internal developers and agents validating tze_hud resident publish performance.
**Success looks like**: A single-stream Rust harness can publish durable widget updates to the existing `user-test` target, emit auditable metrics and artifacts, and support historically comparable runs without violating validation doctrine.
**Trying to be**: A narrow Layer 3 validation instrument for the resident gRPC control plane.
**Not trying to be**: A new control plane, a generic distributed load platform, a dashboard product, or a replacement for compositor/render benchmarks.

### Requirements

| # | Requirement | Class | Evidence | Status |
|---|------------|-------|---------|--------|
| 1 | Resident hot-path benchmarks use one bidirectional gRPC session stream, not many ad hoc connections | Hard | [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md:35), [session-protocol spec](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:20) | Partial |
| 2 | Durable widget publishes must be measurable with request-level correlation | Hard | [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0005-session-protocol.md:600), [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto:563) | Unmet |
| 3 | Performance evidence must be structured, trendable, and machine-readable | Hard | [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md:29), [validation-framework spec](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md:125) | Partial |
| 4 | Formal pass/fail performance claims require calibrated or explicitly `uncalibrated` semantics | Hard | [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md:33), [validation-framework spec](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md:137) | Partial |
| 5 | Initial scope stays on the same primary target as `/user-test-performance`, with future targets deferred | Soft | [.claude skill](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/SKILL.md:61) | Met |
| 6 | The harness should reuse existing validation/artifact infrastructure instead of inventing a second benchmark universe | Soft | [examples/benchmark/src/main.rs](/home/tze/gt/tze_hud/mayor/rig/examples/benchmark/src/main.rs:84), [layer4.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_validation/src/layer4.rs:174) | Partial |
| 7 | MCP is not the primary high-rate path for this benchmark | Non-goal | [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md:31) | N/A |
| 8 | Multi-target orchestration, dashboards, and broad transport coverage are not v1 requirements | Non-goal | [development.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/development.md:153), [.claude skill](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/SKILL.md:83) | N/A |
| 9 | Full wire-byte accounting for gRPC is required in v1 | Unknown | [grpc_widget_publish_perf.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/scripts/grpc_widget_publish_perf.py:338) | Unknown |
| 10 | Remote publish-load benchmarks can be formally normalized with the current calibration model | Unknown | [validation.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_telemetry/src/validation.rs:51) | Unknown |

### Contradictions

[Observed] RFC 0005 already says `WidgetPublishResult` is correlated by `request_sequence`, but the current proto and runtime-facing v1 spec omit that field from the widget publish acknowledgement contract. See [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0005-session-protocol.md:600), [v1 session-protocol spec](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:743), and [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto:563).

[Observed] The Python performance tooling records historical rows and thresholds, but the current validation framework does not yet define publish-load artifacts or calibrated verdict semantics for that benchmark class. See [.claude perf_common.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/scripts/perf_common.py:30) and [validation-framework spec](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md:137).

## Current State

| Dimension | Status | Summary | Key Evidence |
|-----------|--------|---------|-------------|
| Spec adherence | Adequate | Core transport and widget publish semantics exist, but the benchmark contract and request correlation are incomplete | [widget-system spec](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/widget-system/specs/widget-system/spec.md:193) |
| Core workflows | Adequate | Python already proves the bootstrap/publish loop, but honest per-request measurement is missing | [grpc_widget_publish_perf.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/scripts/grpc_widget_publish_perf.py:166) |
| Test confidence | Adequate | Session server already tests widget publish success/failure paths, but not request-sequence correlation | [session_server.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/src/session_server.rs:10091) |
| Observability | Adequate | Layer 4 artifact plumbing exists, but publish-load-specific artifacts are not yet modeled | [layer4.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_validation/src/layer4.rs:446) |
| Delivery readiness | Adequate | The workspace can host a Rust benchmark crate now; the missing work is contract and integration, not foundational infra | [Cargo.toml](/home/tze/gt/tze_hud/mayor/rig/Cargo.toml:18) |
| Architectural fitness | Strong | The repo already has the right hot path, proto tooling, and benchmark precedent for a narrow Rust harness | [build.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/build.rs:1) |

[Observed] Architectural fitness is the biggest strength. The harness does not require inventing new transport infrastructure, a new proto toolchain, or a new benchmark entry pattern. The repo already supports the one-stream resident control path and already has benchmark artifact conventions.

[Observed] Request correlation is the biggest gap. Without it, a sophisticated Rust harness would still be forced into aggregate-only timing for repeated widget publishes, which is below the bar the user asked for and below the bar implied by the RFC.

## Alignment Review

### Aligned next steps

1. [Inferred] Land the `request_sequence` repair for durable widget publish acknowledgements and make the contract explicit in OpenSpec.
2. [Inferred] Define `publish-load-harness` as a dedicated validation capability with benchmark identity, auditable metrics, artifact output, and `uncalibrated` semantics.
3. [Inferred] Implement a new Rust example crate that exercises durable widget publishes over one gRPC session and emits canonical artifacts plus derived CSV summaries.
4. [Inferred] Switch `/user-test-performance` to call the Rust harness for the gRPC widget path while preserving its target registry and historical comparison workflow.

### Misaligned directions

1. [Observed] Treating MCP as the primary performance path for this work is misaligned with doctrine.
2. [Inferred] Broadening the first tranche to tiles, zones, multiple transports, or multiple targets would be overreach before one honest path exists.
3. [Inferred] Reusing compositor frame-session validation structures for publish-stream semantics would blur distinct domains and hide benchmark-specific meaning.

### Premature work

1. [Unknown] Network-path normalization as a formal pass/fail basis is premature until the project decides whether and how remote publish benchmarks should be normalized.
2. [Unknown] Full wire-byte accounting is premature until the transport-instrumentation cost is justified.

### Deferred

1. MCP publish-load benchmarking in Rust.
2. Zone and tile publish harness coverage.
3. Multi-target orchestration and dashboards.

### Rejected

1. [Inferred] A second standalone benchmark universe with its own schema, target vocabulary, and artifact story.
2. [Inferred] Aggregate-only latency reporting presented as a substitute for request-level publish RTT.

## Gap Analysis

### Blockers

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| Durable widget ack lacks request correlation in the implementation-facing contract | Prevents honest per-request RTT on repeated publishes | Protocol/runtime | [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto:563) | Modify `session-protocol` and runtime to add `request_sequence` | M |
| No spec for the harness itself | Any implementation would invent its own contract | Spec | [mcp-stress-testing spec](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/mcp-stress-testing/specs/mcp-stress-testing/spec.md:3) | Add `publish-load-harness` capability spec | M |
| No publish-load artifact / verdict semantics in validation framework | Results cannot be classified honestly | Validation | [validation-framework spec](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md:137) | Add publish-load benchmark evidence requirement | M |

### Important Enhancements

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| Canonical artifact vs CSV history is undefined | Risks two competing sources of truth | Validation/tooling | [.claude perf_common.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/scripts/perf_common.py:30) | Make JSON canonical and CSV derived | S |
| Target/auth contract is underspecified for Rust | Multi-run auditability depends on stable target ids | Tooling | [.claude skill](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/SKILL.md:61) | Reuse target registry and env-based auth contract | S |

### Strategic Gaps

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| Remote publish normalization model is unresolved | Formal pass/fail status cannot be honest yet | Validation | [validation.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_telemetry/src/validation.rs:51) | Keep remote results `uncalibrated` until approved mapping exists | M |
| Wire-byte accounting remains ambiguous | “bytes in/out” could drift in meaning across runs | Tooling | [grpc_widget_publish_perf.py](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/scripts/grpc_widget_publish_perf.py:338) | Require explicit `byte_accounting_mode` | S |

## Work Plan

### Immediate alignment work

### Chunk 1: Repair Widget Publish Result Correlation

**Objective**: Make durable widget publish acknowledgements correlate to their originating request sequence.
**Spec reference**: `openspec/changes/rust-widget-publish-load-harness/specs/session-protocol/spec.md`
**Dependencies**: None
**Why ordered here**: The harness cannot measure repeated publishes honestly without this.
**Scope**: M
**Parallelizable**: No — it changes the protocol contract and tests that the harness depends on.
**Serialize with**: Chunks 2-4

**Acceptance criteria**:
- [ ] `WidgetPublishResult` carries `request_sequence` in proto, runtime, and tests
- [ ] Durable widget publish tests assert request-sequence echo behavior

**Notes**: This is an RFC/OpenSpec drift repair, not a speculative feature.

### Chunk 2: Define Publish-Load Artifact and Verdict Semantics

**Objective**: Make publish-load benchmark outputs auditable and validation-honest.
**Spec reference**: `openspec/changes/rust-widget-publish-load-harness/specs/publish-load-harness/spec.md`, `openspec/changes/rust-widget-publish-load-harness/specs/validation-framework/spec.md`
**Dependencies**: Chunk 1
**Why ordered here**: Artifact and verdict semantics determine what the Rust binary must emit.
**Scope**: M
**Parallelizable**: No — it defines shared schemas consumed by the harness and skill wrapper.
**Serialize with**: Chunks 1, 3, 4

**Acceptance criteria**:
- [ ] Canonical JSON artifact schema exists and is validated
- [ ] CSV summary compatibility fields are defined
- [ ] `uncalibrated` status behavior is explicit for remote runs

**Notes**: Keep JSON canonical and CSV derived.

### Chunk 3: Build the Rust Harness Crate

**Objective**: Add a Rust example crate that runs single-stream durable widget publish benchmarks.
**Spec reference**: `openspec/changes/rust-widget-publish-load-harness/specs/publish-load-harness/spec.md`
**Dependencies**: Chunks 1-2
**Why ordered here**: The contract must exist before the binary is written.
**Scope**: L
**Parallelizable**: Partial — CLI shell and target resolution can be staged, but the executor depends on protocol and artifact schemas.
**Serialize with**: Chunk 4

**Acceptance criteria**:
- [ ] Harness connects over one gRPC session stream and runs paced and burst publish modes
- [ ] Per-request RTT and throughput metrics are emitted
- [ ] Canonical JSON artifact output works

**Notes**: Prefer a new workspace example crate over extending `examples/benchmark`.

### Chunk 4: Integrate `/user-test-performance` and Historical Comparison

**Objective**: Preserve the existing audit workflow while switching the gRPC widget path to the Rust harness.
**Spec reference**: `openspec/changes/rust-widget-publish-load-harness/specs/publish-load-harness/spec.md`
**Dependencies**: Chunk 3
**Why ordered here**: The wrapper should not change until the binary contract is stable.
**Scope**: M
**Parallelizable**: No — it changes the workflow surface that operators already use.
**Serialize with**: Chunk 3

**Acceptance criteria**:
- [ ] `/user-test-performance` invokes the Rust harness for gRPC widget publish runs
- [ ] Existing target registry and results ledger stay compatible
- [ ] Comparison tooling still works on the derived summary rows

**Notes**: MCP benchmarking can remain on the Python path for now.

### Block Reconciliation: Rust Publish-Load Harness

Check:
- [ ] All chunks' acceptance criteria are met
- [ ] End-to-end benchmark flow works as the specs describe
- [ ] No regression in adjacent performance tooling or protocol behavior
- [ ] Spec sections referenced by this block are current
- [ ] Deferred work is captured in beads instead of TODO comments

## Bead Graph

Epic: `hud-bm9i` — `Build Rust widget publish load harness`

Children:
- `hud-bm9i.1` — `Add WidgetPublishResult request correlation`
- `hud-bm9i.2` — `Implement publish-load artifact and verdict schema`
- `hud-bm9i.3` — `Create Rust gRPC widget publish harness crate`
- `hud-bm9i.4` — `Route /user-test-performance gRPC runs to Rust harness`
- `hud-bm9i.5` — `Reconcile spec-to-code (gen-1) for Rust publish harness`
- `hud-bm9i.6` — `Generate epic report for: Rust publish harness`

Dependencies:
- `hud-bm9i.2` depends on `hud-bm9i.1`
- `hud-bm9i.3` depends on `hud-bm9i.1` and `hud-bm9i.2`
- `hud-bm9i.4` depends on `hud-bm9i.3`
- `hud-bm9i.5` depends on `hud-bm9i.1`, `hud-bm9i.2`, `hud-bm9i.3`, and `hud-bm9i.4`
- `hud-bm9i.6` depends on `hud-bm9i.1`, `hud-bm9i.2`, `hud-bm9i.3`, `hud-bm9i.4`, and `hud-bm9i.5`

Execution note:
- `hud-bm9i.1` is the first ready implementation bead.
- `hud-bm9i.5` is the mandatory terminal reconciliation bead for implementation coverage.
- `hud-bm9i.6` is the terminal human-review/report bead and depends on reconciliation.

## Do Not Do Yet

| Item | Reason | Revisit when |
|------|--------|-------------|
| MCP transport support in the Rust harness | Not the hot path under doctrine | After gRPC widget path is stable and benchmarked |
| Zone/tile harness coverage | Expands scope before one honest path exists | After widget publish coverage is complete |
| Multi-target orchestration | Adds coordination and infra complexity too early | After one target has stable artifacts and regressions over time |
| Dashboard/reporting UI | Presentation is lower value than auditable numbers | After artifacts and comparison workflows are trusted |
| Formal normalized verdicts for remote publish latency | Normalization model is unresolved | After publish-benchmark normalization is specified |

## Appendix

### A. Repository Map
- Rust workspace with protocol, runtime, validation, telemetry, compositor, widget, and example crates
- `.claude/skills/user-test-performance/` owns current benchmarking workflow, target registry, and history ledger
- `openspec/changes/` contains in-flight capability deltas

### B. Critical Workflows
1. Operator invokes `/user-test-performance` for gRPC widget publish benchmarking
2. Tool resolves target config and credentials
3. Resident agent opens one gRPC session and publishes durable widget updates
4. Harness emits JSON artifacts plus derived historical summary row
5. Comparison tooling evaluates drift or regression over time

### C. Spec Inventory
- `openspec/changes/widget-system/specs/widget-system/spec.md`: widget publish semantics
- `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`: resident session stream contract
- `openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md`: validation layers and calibration doctrine
- `openspec/changes/rust-widget-publish-load-harness/`: new planning change for this work

### D. Evidence Index
- [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md)
- [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md)
- [development.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/development.md)
- [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/legends-and-lore/rfcs/0005-session-protocol.md)
- [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto)
- [session_server.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/src/session_server.rs)
- [examples/benchmark/src/main.rs](/home/tze/gt/tze_hud/mayor/rig/examples/benchmark/src/main.rs)
- [layer4.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_validation/src/layer4.rs)
- [.claude user-test-performance skill](/home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test-performance/SKILL.md)
- [rust-widget-publish-load-harness proposal](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/rust-widget-publish-load-harness/proposal.md)
- [rust-widget-publish-load-harness design](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/rust-widget-publish-load-harness/design.md)

---

## Conclusion

The real direction is to build a narrow, Rust, gRPC-session-faithful publish-load harness for the existing `user-test` target, and to refuse any broader story until the protocol correlation seam and validation semantics are fixed. The next work is explicit: repair `WidgetPublishResult` correlation, land the publish-load OpenSpec change, implement the Rust example crate, and route `/user-test-performance` through it. The project should stop pretending that aggregate-only Python timing on the current gRPC path is good enough for historically auditable hot-path performance claims.
