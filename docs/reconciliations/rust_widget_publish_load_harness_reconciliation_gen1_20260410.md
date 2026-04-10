# Rust Widget Publish Load Harness Reconciliation (gen-1)

Date: 2026-04-10
Issue: `hud-bm9i.5`
Inputs:
- `openspec/changes/rust-widget-publish-load-harness`
- `docs/reconciliations/rust_widget_publish_load_harness_direction_20260410.md`
- merged implementation beads `hud-bm9i.1`..`hud-bm9i.4` (`#409`, `#411`, `#415`, `#420`)

## Reconciliation Goal

Verify that every retained requirement in the Rust publish-load OpenSpec change
is mapped to concrete implementation evidence (or an explicit justified gap),
and identify any follow-on work needed before closeout.

## Requirement-to-Implementation Checklist

| Req ID | Source | Requirement (short) | Implementing bead(s) | Evidence | Status |
|---|---|---|---|---|---|
| SP-1 | `specs/session-protocol/spec.md` | Durable `WidgetPublishResult` carries `request_sequence`, `accepted`, `widget_name`, error fields; no result for ephemeral publishes | `hud-bm9i.1` | `crates/tze_hud_protocol/proto/session.proto`, `crates/tze_hud_protocol/src/session_server.rs`, `crates/tze_hud_protocol/tests/widget_publish_integration.rs` | Covered |
| SP-2 | `specs/session-protocol/spec.md` | Accepted durable publish echoes client envelope sequence | `hud-bm9i.1` | `handle_widget_publish(... request_sequence ...)` success path in `session_server.rs` | Covered |
| SP-3 | `specs/session-protocol/spec.md` | Rejected durable publish still echoes sequence | `hud-bm9i.1` | Rejection paths in `session_server.rs` return `WidgetPublishResult { request_sequence, accepted: false, ... }` | Covered |
| SP-4 | `specs/session-protocol/spec.md` | Repeated publishes to same widget remain distinguishable by sequence | `hud-bm9i.1` | `test_durable_widget_publish_repeated_requests_are_correlated` in `session_server.rs` | Covered |
| PH-1 | `specs/publish-load-harness/spec.md` | Rust harness exists; one stream; real bootstrap/auth/capability flow; durable widget publish hot path | `hud-bm9i.3` | `examples/widget_publish_load_harness/src/main.rs` (single `session()` stream + `SessionInit` + `WidgetPublish`) | Covered |
| PH-2 | `specs/publish-load-harness/spec.md` | Future transport requests (MCP/zone/tile) fail fast with explicit unsupported-mode error | `hud-bm9i.3` | Harness CLI parser accepts unknown `--<key>`/flags without explicit rejection; no transport selector validation in `main.rs` | **Gap** |
| PH-3 | `specs/publish-load-harness/spec.md` | Target registry keyed by `target_id`; metadata includes identity fields; env/config auth only | `hud-bm9i.3` | `resolve_target(...)`, `TargetConfig`, env-based `psk_env` lookup in `main.rs`; `targets/publish_load_targets.toml` | Partial |
| PH-4 | `specs/publish-load-harness/spec.md` | Burst and paced modes with controlling parameters recorded | `hud-bm9i.3` | `WorkloadMode::{Burst,Paced}` and run-mode handling in `main.rs`; identity/metrics fields in artifact | Covered |
| PH-5 | `specs/publish-load-harness/spec.md` | Stable benchmark identity/comparison key and traceability refs | `hud-bm9i.2`, `hud-bm9i.3` | `PublishLoadIdentity::stable_comparison_key` and `PublishLoadTraceability` in `crates/tze_hud_telemetry/src/publish_load.rs`; artifact creation in harness | Covered |
| PH-6 | `specs/publish-load-harness/spec.md` | Per-request RTT and aggregate metrics via `request_sequence` correlation | `hud-bm9i.1`, `hud-bm9i.3` | `drain_widget_publish_results(...)` maps ack `request_sequence` to inflight send timestamps in harness | Covered |
| PH-7 | `specs/publish-load-harness/spec.md` | Explicit byte-accounting semantics (`payload_*` mandatory, `wire_*` optional + labeled) | `hud-bm9i.2`, `hud-bm9i.3` | `ByteAccountingMode` + validator rules in `publish_load.rs`; harness emits `payload_only` | Covered |
| PH-8 | `specs/publish-load-harness/spec.md` | Canonical JSON artifact + CSV compatibility with `/user-test` historical ledger | `hud-bm9i.2`, `hud-bm9i.3`, `hud-bm9i.4` | harness `write_artifact(...)`; `.claude/.../grpc_widget_publish_perf.py`; `.claude/.../perf_common.py` + tests | Covered |
| PH-9 | `specs/publish-load-harness/spec.md` | `uncalibrated` semantics for runs without approved normalization | `hud-bm9i.2`, `hud-bm9i.3` | `PublishLoadArtifact::validate_uncalibrated_semantics` + harness verdict/calibration wiring | Covered |
| VF-1 | `specs/validation-framework/spec.md` | Publish-load evidence modeled as distinct benchmark class with audit fields | `hud-bm9i.2` | `PublishLoadArtifact` schema and validation tests in `tze_hud_telemetry` | Covered |
| VF-2 | `specs/validation-framework/spec.md` | Layer 4 artifact set + manifest references publish-load outputs | none completed in epic | No `publish_load` wiring in `crates/tze_hud_validation/src/layer4.rs` or harness-to-L4 integration path | **Gap** |
| VF-3 | `specs/validation-framework/spec.md` | Calibration status semantics distinguish raw evidence from formal verdicts | `hud-bm9i.2`, `hud-bm9i.3` | `PublishLoadCalibrationStatus`/`PublishLoadVerdict` validation and harness flags | Covered |

## Coverage Verdict

1. Core protocol correlation, artifact schema, Rust harness execution path, and
   `/user-test-performance` routing are implemented and test-backed.
2. Two retained requirements are not fully met in current merged state:
   explicit unsupported transport rejection, and Layer 4 manifest inclusion for
   publish-load artifact sets.
3. Target-registry behavior is implemented, but default registry payload does
   not currently encode the `user-test-windows-tailnet` scenario used in the
   OpenSpec narrative and skill example.

## Gap Notes

1. **Unsupported transport rejection is implicit/absent**
   - Current CLI only supports burst/paced workload mode and does not model a
     transport selector.
   - Unknown keys/flags are currently accepted silently, so invalid transport
     intent can be ignored instead of failing fast.
2. **Layer 4 publish-load manifest wiring is missing**
   - Publish-load artifacts are emitted directly to JSON/CSV via harness+skill
     scripts.
   - There is no explicit bridge that registers publish-load outputs into a
     Layer 4 manifest entry set for this benchmark class.
3. **Default target registry example drift**
   - `targets/publish_load_targets.toml` currently includes only `local-dev`,
     while spec/skill examples use `user-test-windows-tailnet`.

