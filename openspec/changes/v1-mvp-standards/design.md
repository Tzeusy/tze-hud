## Context

tze_hud is a greenfield agent-native presence engine following a doctrine-first development model. The project has:

- **Doctrine** (`heart-and-soul/`, 11 files) — philosophical and architectural foundations defining what the system is, what it is not, and seven non-negotiable rules
- **Design contracts** (`docs/rfcs/0001–0011`) — wire-level specifications with protobuf schemas, state machines, field allocations, latency budgets, and cross-subsystem integration contracts
- **Existing protocol code** (`crates/tze_hud_protocol/proto/session.proto`) — partial implementation of RFC 0005's bidirectional streaming protocol

What's missing is a **normative specification layer** that bridges doctrine/RFCs to implementation. The RFCs define contracts (HOW subsystems behave at the wire level); specifications define requirements (WHAT the system SHALL do, with testable scenarios). This change creates that layer.

**Stakeholders:** LLM implementers (primary consumers — must load specs into context for implementation tasks), CI validation (specs generate test scenarios), human reviewers (specs trace to doctrine for alignment audits).

## Goals / Non-Goals

**Goals:**

- Create one spec per v1 subsystem, each with testable requirements and scenarios
- Trace every requirement to its source RFC section and doctrine file
- Enforce the v1 scope boundary: requirements marked v1-mandatory are normative; everything else is explicitly deferred
- Extract quantitative budgets into testable requirements (latency targets, capacity limits, resource ceilings)
- Make cross-subsystem integration contracts explicit (which spec depends on which, what crosses boundaries)
- Provide the specification foundation for OpenSpec task generation

**Non-Goals:**

- Not replacing RFCs — specs reference RFCs as authoritative design contracts; specs capture the normative subset that v1 code must implement
- Not implementation guidance — specs say WHAT, not HOW; implementation details belong in tasks and code
- Not post-v1 planning — schema-reserved features are documented as deferred, not specified
- Not API documentation — specs define behavioral requirements, not SDK ergonomics

## Decisions

### D1: One-to-one RFC-to-spec mapping for subsystem specs

Each of the 11 RFCs maps to exactly one capability spec with the same domain boundary. This avoids cross-cutting ambiguity and makes traceability trivial: spec requirement → RFC section → doctrine principle.

**Alternative considered:** Grouping by architectural plane (MCP specs, gRPC specs, rendering specs). Rejected because RFCs already define coherent subsystem boundaries, and splitting would create artificial seams that don't match the dependency graph.

### D2: Validation framework as a 12th cross-cutting spec

`validation.md` from the doctrine defines five validation layers, the test scene registry, hardware-normalized calibration, and developer visibility artifacts. This has no corresponding RFC but is a v1 success criterion. It gets its own spec because it governs testing requirements across all other specs.

**Alternative considered:** Embedding validation requirements into each subsystem spec. Rejected because the validation framework has its own coherent contract (layers, scenes, telemetry schema, artifact pipeline) that would fragment if scattered.

### D3: Requirements use SHALL/MUST (normative) with v1 scope tags

Every requirement explicitly states whether it is v1-mandatory, v1-reserved (schema defined but implementation may be minimal), or post-v1 (documented for forward compatibility, not implemented). Only v1-mandatory requirements generate implementation tasks.

**Alternative considered:** Separate v1 and post-v1 spec files. Rejected because understanding what's deferred requires seeing it alongside what ships — context is lost when split.

### D4: Scenarios as testable contracts

Each requirement includes at least one scenario in WHEN/THEN format. These map directly to test cases across the five validation layers. Scene graph scenarios → Layer 0 tests. Rendering scenarios → Layer 1/2 tests. Performance scenarios → Layer 3 benchmarks.

### D5: Spec dependency graph mirrors RFC dependency graph

```
scene-graph (foundation)
  ├── runtime-kernel
  ├── timing-model
  ├── input-model
  └── session-protocol
        ├── configuration
        ├── system-shell
        ├── lease-governance
        └── policy-arbitration
              └── scene-events
resource-store (orthogonal)
validation-framework (cross-cutting, references all specs)
```

## Risks / Trade-offs

- **[Spec drift from RFCs]** → Mitigation: Every requirement includes `Source: RFC NNNN §X.Y` traceability. Three-pass review validates adherence to law-and-lore contracts.
- **[Scope creep beyond v1]** → Mitigation: Requirements tagged with v1 scope. Post-v1 items documented as deferred but generate no tasks. Review pass validates against `heart-and-soul/v1.md`.
- **[Over-specification]** → Mitigation: Specs define WHAT, not HOW. Implementation flexibility preserved within contract boundaries. If a requirement constrains implementation unnecessarily, it's a spec bug.
- **[Cross-spec inconsistency]** → Mitigation: Cross-RFC integration map (`.claude/skills/law-and-lore/references/cross-rfc-map.md`) used as consistency checklist. Known resolved inconsistencies from RFC review rounds documented.
- **[Context window pressure for LLM implementers]** → Mitigation: Specs are self-contained per subsystem. Implementers load only the spec(s) relevant to their task, same as the selective-loading pattern in heart-and-soul and law-and-lore skills.
