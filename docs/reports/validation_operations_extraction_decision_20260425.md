# Validation Operations Extraction Decision

Date: 2026-04-25
Bead: hud-8pmjy
Decision: **extract**

## Context

`v2-embodied-media-presence` currently carries the deferred v1 validation backlog in `openspec/changes/v2-embodied-media-presence/specs/validation-operations/spec.md`, added by commit `9a75214` (`Carry v1 standards deferred work into v2 specs`). The carried-forward work includes the standalone Layer 3 benchmark path, the baseline 25-scene registry, record/replay trace infrastructure, soak/leak validation, three-agent cross-spec integration, and calibrated reference-hardware budget gates.

Those items came from the archived `v1-mvp-standards` validation framework and are not intrinsically dependent on v2 media, embodied presence, device-profile execution, or broader AV orchestration.

## Decision

Extract `validation-operations` into a standalone OpenSpec change, provisionally named `validation-operations-standalone`, before relying on it as the permanent home for the v1 carry-forward backlog.

The standalone change should own the v1 carry-forward validation requirements and sync them independently into `openspec/specs/`. The v2 `validation-operations` delta should then narrow to v2-specific extensions: media/device dual-lane validation, operator/failure observability for media and embodied flows, phase release gates, and any v2-specific conformance audits.

## Rationale

1. The v1 carry-forward backlog has independent release value. Keeping it only inside the v2 change makes canonical spec sync wait for unrelated v2 phase gates.
2. The v2 signoff packet treats v2 as a multi-phase program with soft gate on v1 ship, RFC prerequisites, doctrine/RFC sequencing, procurement, and per-phase closeout gates. Any stall or rebaseline in that program would strand the v1 validation backlog in a non-canonical delta.
3. Extracting reduces review and planning risk. A smaller validation-operations change can close around the archived v1 standards scope without inheriting media plane, embodied presence, device-profile, recording, cloud relay, or bidirectional AV decisions.
4. The split preserves v2 intent. V2 can still depend on and extend the canonical validation-operations spec once the standalone change lands.

## Required Follow-Up

Coordinator should file a child bead to create the standalone OpenSpec change.

Suggested bead:

```json
{
  "title": "Create standalone OpenSpec change for validation-operations carry-forward",
  "type": "task",
  "priority": 2,
  "depends_on": "hud-8pmjy",
  "rationale": "Extract v1 validation backlog currently carried only by v2-embodied-media-presence into a smaller OpenSpec change that can sync independently into canonical specs."
}
```

## Standalone Change Scope

Include:

- v1 deferred validation backlog from archived `openspec/changes/archive/2026-04-18-v1-mvp-standards/specs/validation-framework/spec.md`.
- `V1 Validation Backlog Carries Forward` requirements currently in v2 `validation-operations`.
- Cross-spec conformance audits that are inherited v1 convergence work rather than v2-media-specific evidence.
- Tasks to reconcile the new standalone delta against `openspec/specs/validation-framework` and any already-landed validation harness artifacts.

Exclude:

- V2 media/device real-decode lane details that depend on D18/D19/D20.
- V2 operator and failure observability tied specifically to media admission, embodied presence, recording, cloud relay, or bidirectional AV.
- V2 phase sequencing and release gates that belong to `v2-embodied-media-presence`.

## Interim Handling

Until the standalone change lands, treat the current v2 `validation-operations` delta as a temporary staging location only. Do not archive or close v2 on the assumption that it is the sole canonical path for v1 carry-forward validation requirements.
