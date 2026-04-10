## Why

The current `/user-test-performance` workflow records useful historical numbers, but its Python gRPC path cannot measure repeated widget publishes honestly because durable `WidgetPublishResult` acknowledgements are not correlated to the originating client envelope sequence. The repo also lacks a dedicated spec for a resident-agent publish-load harness, so any Rust implementation would otherwise be inventing its own contract for metrics, artifacts, and verdict semantics.

## What Changes

- Add a new `publish-load-harness` capability that defines a Rust, gRPC-first benchmark harness for resident-agent widget publishing on a single bidirectional session stream
- Define the target registry, workload modes, benchmark identity, auditable metrics, byte-accounting semantics, artifact outputs, and historical comparison contract for the harness
- Modify `session-protocol` so durable `WidgetPublishResult` acknowledgements carry `request_sequence`, enabling honest per-request RTT measurement on a multiplexed stream
- Modify `validation-framework` so publish-load benchmark runs have explicit raw-vs-normalized reporting rules, `uncalibrated` status semantics, and Layer 4 artifact expectations
- Keep initial scope narrow: one target class, durable widget publishes, gRPC only; MCP, zones, tiles, and multi-target orchestration remain future work

## Capabilities

### New Capabilities
- `publish-load-harness`: Rust benchmark harness for resident-agent widget publish throughput and latency measurement, including target resolution, auditable metrics, artifact emission, and historical comparison fields

### Modified Capabilities
- `session-protocol`: durable `WidgetPublishResult` acknowledgements gain explicit request-sequence correlation so repeated publishes can be measured per request
- `validation-framework`: publish-load benchmark runs gain explicit artifact, calibration-status, and verdict semantics for remote resident-agent performance evidence

## Impact

- New Rust workspace example crate for the harness, reusing `tze_hud_protocol` generated client types and `tze_hud_validation` artifact sinks
- Protocol/runtime/test changes in `crates/tze_hud_protocol` to carry and assert `WidgetPublishResult.request_sequence`
- New benchmark telemetry and artifact schema for publish-load runs, plus a derived CSV compatibility path for `.claude/skills/user-test-performance`
- Skill and documentation updates so `/user-test-performance` invokes the Rust harness for gRPC widget publish benchmarking instead of the current Python gRPC path
