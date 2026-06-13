## Why

The v1 validation backlog is currently carried forward only inside the broad `v2-embodied-media-presence` program. That couples baseline validation operations, canonical-spec reconciliation, and cross-spec conformance audits to media/device sequencing even though these obligations are needed independently for v1 closeout and future work.

## What Changes

- Extract the validation-operations carry-forward into a standalone OpenSpec change that can sync into canonical specs without waiting for the v2 media/device program.
- Preserve the v1 deferred validation backlog: standalone Layer 3 benchmark JSON emission, split-latency reporting, baseline 25-scene registry, record/replay traces, soak/leak validation, three-agent cross-spec integration evidence, and calibrated reference-hardware budget gates.
- Add cross-spec conformance audit gates for capability vocabulary, MCP authority-surface enforcement, and protobuf/session-envelope field allocation parity.
- Add explicit reconciliation tasks against `openspec/specs/validation-framework/spec.md` so duplicated or stale backlog language can be resolved before archive.
- Exclude v2-specific media, embodied presence, device-profile execution, real-decode D18/D19/D20 details, cloud-relay/bidirectional observability, phase sequencing, and release gates.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `validation-framework`: adds standalone validation-operations carry-forward requirements and audit/reconciliation gates independent of the v2 media/device program.

## Impact

- Specification-only change under `openspec/changes/validation-operations-standalone/`.
- No runtime, protocol, dependency, or test behavior changes.
- Future implementation beads can target canonical `validation-framework` work without depending on `v2-embodied-media-presence`.
