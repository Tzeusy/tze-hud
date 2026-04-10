# RFC 0011 Session Resource Upload Backlog Materialization

Date: 2026-04-10
Source artifact: `docs/reconciliations/session_resource_upload_rfc0011_direction_report_20260410.md`
Scope: materialize the smallest honest bead graph that closes the resident scene-resource upload seam
Materialized epic: `hud-ooj1`

## Purpose

Turn the RFC 0011 seam analysis into an execution-ready, low-churn beads graph.

This backlog assumes the current repo reality:

1. The doctrine and resource-store spec already require resident scene-resource ingress on the primary session stream.
2. The checked-in session schema and server do not expose that path.
3. RFC 0011 itself needs a contract repair for upload-start acknowledgement and correlation before implementation should begin.

## Ordering Rules

1. Reconcile the upload handshake and envelope allocation before touching protobuf or runtime code.
2. Land schema changes before runtime/server work.
3. Add conformance/integration coverage before converting resident exemplar consumers.
4. End the epic with both a reconciliation bead and a human-readable report bead.

## Materialized Bead Set

### Phase A: Contract truth

| Bead ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| hud-ooj1.1 | Reconcile resident scene-resource upload contract across RFC 0011 and main specs | task | 1 | none | The current contract is internally inconsistent and cannot be implemented honestly. |

### Phase B: Schema and core implementation

| Bead ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| hud-ooj1.2 | Extend session protocol schema with resident scene-resource upload messages | task | 1 | hud-ooj1.1 | Runtime/client work needs an authoritative wire contract first. |
| hud-ooj1.3 | Implement resident session-stream scene-resource upload handling | feature | 1 | hud-ooj1.2 | This is the core behavioral repair that unblocks resident agents. |
| hud-ooj1.4 | Add conformance and integration coverage for resident upload flow | task | 1 | hud-ooj1.3 | The seam needs durable proof across inline, chunked, dedup, and rejection paths. |

### Phase C: Consumer repair

| Bead ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| hud-ooj1.5 | Repair resident exemplar and `/user-test` consumers to use real uploads | task | 1 | hud-ooj1.4 | Exemplar and operator surfaces should switch only after the core path is stable and tested. |

### Phase D: Terminal quality gates

| Bead ID | Title | Type | Priority | Depends on | Notes |
|---|---|---|---|---|---|
| hud-ooj1.6 | Reconcile spec-to-code (gen-1) for resident scene-resource upload | task | 1 | hud-ooj1.1, hud-ooj1.2, hud-ooj1.3, hud-ooj1.4, hud-ooj1.5 | Mandatory terminal reconciliation bead. |
| hud-ooj1.7 | Generate epic report for resident scene-resource upload seam | task | 1 | hud-ooj1.1, hud-ooj1.2, hud-ooj1.3, hud-ooj1.4, hud-ooj1.5, hud-ooj1.6 | Human-readable closeout/report artifact. |

## Dependency Graph

1. `hud-ooj1.1`
2. `hud-ooj1.2`
3. `hud-ooj1.3`
4. `hud-ooj1.4`
5. `hud-ooj1.5`
6. `hud-ooj1.6`
7. `hud-ooj1.7`

No parallelism is recommended in the initial tranche. The schema, runtime, and consumer surfaces are tightly coupled, and premature splitting would create review churn without reducing risk.

## Coordinator-Ready Summary

- The first ready child bead is the contract reconciliation bead.
- The protobuf/schema and runtime beads are the core implementation tranche.
- Consumer conversion should not start before the coverage bead closes.
- The terminal reconciliation bead is mandatory because this seam spans doctrine, RFCs, main specs, protobuf, runtime, and operator tooling.
- The report bead should summarize both the contract repair and the consumer conversion state.

## Recorded Beads Payload

See `docs/reconciliations/session_resource_upload_rfc0011_backlog_materialization_20260410.proposed_beads.json`.
