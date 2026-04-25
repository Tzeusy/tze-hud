# Contribution Readiness

After completing this curriculum, the learner should be able to:
- explain why `tze_hud` is a sovereign runtime/compositor rather than a generic UI app
- read scene, runtime, protocol, and integration-test code without losing track of ownership boundaries
- reason about why a change belongs in code, doctrine, RFCs, OpenSpec, or reconciliation artifacts
- predict at least one compatibility or invariant risk before editing protocol, timing, policy, or resource logic
- use validation output, telemetry, and artifacts as the primary debugging surface

Safer first reading targets:
- `about/heart-and-soul/architecture.md`
- `about/lay-and-land/components.md`
- `openspec/specs/scene-graph/spec.md`
- `openspec/specs/runtime-kernel/spec.md`
- `openspec/specs/session-protocol/spec.md`
- `tests/integration/v1_thesis.rs`

Suggested first contribution categories:
- improve or extend docs/spec traceability where authority is already clear
- add or tighten scene-graph or protocol tests for an already-understood invariant
- improve structured diagnostics, validation artifacts, or operator-facing config errors
- make small, localized changes to MCP tool behavior after confirming the protocol and policy surface

Hazard areas where incomplete understanding is risky:
- `.proto` files, generated protocol shapes, and anything that changes wire compatibility
- timing fields, scheduling logic, sync groups, or expiry semantics
- lease, capability, privacy, redaction, attention, and degradation code paths
- queueing/backpressure/freeze behavior in runtime or protocol handlers
- resource upload, dedup, or widget/zone lifecycle semantics

Practical handoff rule:
- If you cannot say which invariant a change might break, you are not yet ready to land that change.
