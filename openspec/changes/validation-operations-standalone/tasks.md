## 1. Canonical Validation Reconciliation

- [ ] 1.1 Compare this delta against `openspec/specs/validation-framework/spec.md` and the archived `v1-mvp-standards/specs/validation-framework/spec.md`.
- [ ] 1.2 Preserve existing canonical requirements where they already cover the v1 backlog; avoid duplicate requirement names during archive.
- [ ] 1.3 Promote only missing standalone validation-operations obligations into canonical `validation-framework`.

## 2. V1 Backlog Closure

- [ ] 2.1 Confirm the standalone Layer 3 benchmark path emits machine-readable JSON with per-frame telemetry and split-latency reporting.
- [ ] 2.2 Confirm the baseline 25-scene registry is represented as canonical validation-framework evidence and remains extensible.
- [ ] 2.3 Confirm record/replay trace infrastructure and soak/leak validation are covered by executable evidence plans.
- [ ] 2.4 Confirm the three-agent integration run and calibrated reference-hardware budget gates are represented independently of v2 media/device release gates.

## 3. Cross-Spec Conformance Audits

- [ ] 3.1 Audit capability vocabulary across configuration, runtime, session protocol, MCP, and spec prose.
- [ ] 3.2 Audit MCP authority-surface enforcement for lease-free guest operations, capability-gated guest publications, and resident-authority-gated operations.
- [ ] 3.3 Audit protobuf/session-envelope field allocation parity against `openspec/specs/session-protocol/spec.md`.
- [ ] 3.4 File follow-up implementation or reconciliation beads for discovered drift; do not broaden this change with fixes.

## 4. Archive Readiness

- [ ] 4.1 Run OpenSpec validation for `validation-operations-standalone`.
- [ ] 4.2 Archive or sync this change only after duplicate canonical language has been resolved.
