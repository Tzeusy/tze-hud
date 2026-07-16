## 1. Contract Amendment

- [x] 1.1 Amend RFC 0002 with the cadence envelope, workload metric, quiescent recovery, sole authority, and atomic policy contract
- [x] 1.2 Amend RFC 0005 with append-only exact level mapping, bounded never-drop delivery, and snapshot/current-state ordering
- [x] 1.3 Strict-validate the OpenSpec change before behavior code

## 2. Deterministic Controller and Telemetry

- [x] 2.1 Add failing tests for cadence derivation, elapsed-window p95, reset, hysteresis, and quiescent recovery
- [x] 2.2 Implement the immutable envelope and injected-monotonic controller API
- [x] 2.3 Add backward-compatible per-frame degradation workload/applied-level fields and structured transition context

## 3. Compositor Policy

- [x] 3.1 Add failing pure tests for exhaustive level mapping and stable-SceneId suppression snapshots
- [x] 3.2 Implement the explicit compositor policy and Level 2-5 render consumers without scene or lease mutation
- [x] 3.3 Prove the scene-side stateful tracker has no production call site

## 4. Transactional Protocol

- [x] 4.1 Append exact protobuf enum values and add mapping/default-block tests
- [x] 4.2 Replace lossy degradation broadcast with a bounded per-session never-drop hub and queue-full test
- [x] 4.3 Implement and test SessionEstablished/resume snapshot-current-transition ordering

## 5. Production Wiring

- [x] 5.1 Wire windowed active-frame telemetry, N-to-N+1 policy application, quiescent recovery, tracing, and session notices
- [x] 5.2 Wire the same semantic controller/policy/telemetry path into headless rendering
- [x] 5.3 Wire Level 1 outbound state-stream coalescing without changing inbound batch atomicity

## 6. Verification

- [x] 6.1 Run focused deterministic runtime, compositor, telemetry, and protocol tests
- [x] 6.2 Run the named sustained-load production-path payload in release mode with timing assertions and structured output
- [x] 6.3 Run production call-site searches, workspace check, fmt, clippy all-targets, workspace tests, and integration tests
- [x] 6.4 Verify the OpenSpec implementation for completeness, correctness, and coherence
