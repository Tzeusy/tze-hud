# Policy Wiring Epic Prompt

Use `/project-direction` end-to-end for this epic. This is not a narrow implementation task. The goal is to decide, with repo evidence, whether `tze_hud_policy` should become a real runtime authority path in v1 or whether claims/spec/docs should shrink to match the current runtime.

## Objective

Produce a spec-grounded work plan for wiring policy arbitration into the runtime in a way that preserves the current strengths of the system:

- pure scene graph
- runtime sovereignty
- explicit authority boundaries
- deterministic validation
- fail-closed governance

The output must be concrete enough to drive a beads epic with implementation children, reconciliation, and a human-readable report bead.

## Why this epic exists

Current evidence shows a split between architecture claims and runtime reality:

- [crates/tze_hud_runtime/src/lib.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime/src/lib.rs) documents `tze_hud_policy` as "not wired in v1".
- [crates/tze_hud_runtime/src/budget.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime/src/budget.rs) treats policy integration as aspirational.
- [openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md) already defines a v1-mandatory arbitration stack.
- [about/heart-and-soul/architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md), [about/heart-and-soul/privacy.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/privacy.md), and [about/heart-and-soul/attention.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/attention.md) treat policy/governance as product-defining, not optional.

This mismatch needs resolution before more scope is added.

## Scope

Focus on:

- runtime authority boundaries
- per-frame / per-event / per-mutation policy evaluation
- integration points with privacy, attention, security, resource, and content resolution
- observability and error surfaces for policy decisions
- test strategy needed to make policy enforcement trustworthy

Do not start implementation in this pass. Produce the direction report and beads decomposition only.

## Required evidence set

Read at minimum:

- [about/heart-and-soul/vision.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/vision.md)
- [about/heart-and-soul/v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md)
- [about/heart-and-soul/architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md)
- [about/heart-and-soul/privacy.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/privacy.md)
- [about/heart-and-soul/attention.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/attention.md)
- [about/heart-and-soul/validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md)
- [openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md)
- [openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md)
- [openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md)
- [crates/tze_hud_runtime/src/lib.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime/src/lib.rs)
- [crates/tze_hud_runtime/src/budget.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime/src/budget.rs)
- [crates/tze_hud_runtime/src/attention_budget/mod.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime/src/attention_budget/mod.rs)
- [crates/tze_hud_runtime/src/quiet_hours/mod.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime/src/quiet_hours/mod.rs)
- [crates/tze_hud_runtime/src/shell](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime/src/shell)
- [crates/tze_hud_policy/src/lib.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_policy/src/lib.rs)
- [crates/tze_hud_protocol/src/session_server.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/src/session_server.rs)

## Questions the direction pass must answer

1. What is the minimum viable way to make `tze_hud_policy` operational without collapsing the current runtime boundaries?
2. Which current logic should move into `tze_hud_policy`, and which should remain runtime-owned for latency, statefulness, or ownership reasons?
3. Where should the arbitration stack execute for:
   - per-frame checks
   - per-event checks
   - per-mutation checks
4. What state must remain local to runtime-owned modules such as safe mode, attention counters, or degradation enforcement?
5. What contract tests and telemetry must exist before policy wiring is credible?
6. If full wiring is not tractable for near-term v1, what claims/spec sections/docs must shrink immediately?

## Output requirements

Produce the full `/project-direction` output, including:

- executive summary
- contradictions and gap analysis
- aligned next steps vs premature work
- chunked work plan with spec references
- explicit “do not do yet” section
- blunt conclusion

Then materialize a beads epic with:

- implementation children
- one reconciliation bead
- one implementation report bead

## Non-negotiable constraints

- Keep the screen sovereign: no agent-controlled policy bypasses.
- Do not put LLM logic in the frame loop.
- Prefer fail-closed behavior over permissive fallback.
- Preserve diagnostic structured errors and measurable latency budgets.
- Do not duplicate stateful enforcement logic in multiple crates unless the duplication is temporary and explicitly retired.

## Recommended decomposition shape

The likely chunk pattern should look roughly like:

1. Authority-boundary decision and spec reconciliation
2. Runtime integration seam design
3. Per-frame/per-event/per-mutation evaluation integration
4. Telemetry + structured error contract
5. Policy-focused validation and regression coverage
6. Documentation/report/reconciliation

## Success condition

At the end of the direction pass, a separate implementer should be able to pick up the resulting beads epic and know:

- whether policy wiring is a real v1 deliverable
- exactly where the seams belong
- which specs are authoritative
- what order to implement the work in
- what not to touch yet
