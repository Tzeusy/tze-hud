# Resident Scene-Resource Upload Reconciliation (gen-1)

Date: 2026-04-17
Issue: `hud-ooj1.6`
Epic: `hud-ooj1` (Expose RFC 0011 scene-resource upload on resident session stream)

## Inputs Audited

- `bd show hud-ooj1.6 --json`
- `docs/reconciliations/session_resource_upload_rfc0011_direction_report_20260410.md`
- `docs/reconciliations/session_resource_upload_rfc0011_backlog_materialization_20260410.md`
- `openspec/changes/session-resource-upload-rfc0011/reconciliation-hud-ooj1.1.md`
- `openspec/changes/session-resource-upload-rfc0011/tasks.md`
- `openspec/changes/session-resource-upload-rfc0011/specs/resident-scene-resource-upload/spec.md`
- `openspec/changes/session-resource-upload-rfc0011/specs/session-protocol/spec.md`
- `openspec/changes/session-resource-upload-rfc0011/specs/resource-store/spec.md`
- `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
- `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md`
- `crates/tze_hud_protocol/proto/session.proto`
- `crates/tze_hud_protocol/src/session_server.rs`
- `crates/tze_hud_protocol/tests/resource_upload_integration.rs`
- `.claude/skills/user-test/scripts/hud_grpc_client.py`
- `.claude/skills/user-test/scripts/presence_card_exemplar.py`
- `docs/exemplar-presence-card-user-test.md`
- `crates/tze_hud_resource/src/types.rs`
- `crates/tze_hud_resource/src/upload.rs`

## Requirement-to-Bead Coverage Matrix

| Requirement | Primary implementing bead(s) | Status | Evidence |
|---|---|---|---|
| `resident-scene-resource-upload` :: Resident Scene-Resource Upload via Session Stream | `hud-ooj1.1`, `hud-ooj1.2`, `hud-ooj1.3` | Covered | Delta contract and split-envelope allocations: `openspec/changes/session-resource-upload-rfc0011/specs/resident-scene-resource-upload/spec.md:3`; wire fields on stream: `crates/tze_hud_protocol/proto/session.proto:92`; runtime dispatch on primary session stream: `crates/tze_hud_protocol/src/session_server.rs:2043`. |
| `resident-scene-resource-upload` :: Upload Start Acknowledgement and Correlation | `hud-ooj1.2`, `hud-ooj1.3`, `hud-ooj1.4` | Covered | Ack contract in proto: `crates/tze_hud_protocol/proto/session.proto:641`; runtime emits `ResourceUploadAccepted`: `crates/tze_hud_protocol/src/session_server.rs:4355`; correlation tested with out-of-order completion: `crates/tze_hud_protocol/src/session_server.rs:11955`. |
| `resident-scene-resource-upload` :: Resident Upload Success and Error Surfaces | `hud-ooj1.2`, `hud-ooj1.3`, `hud-ooj1.4` | Covered | Dedicated success/error payloads defined: `crates/tze_hud_protocol/proto/session.proto:659`; runtime emits structured `ResourceErrorResponse` (`message/context/hint`): `crates/tze_hud_protocol/src/session_server.rs:4212`; envelope round-trip coverage: `crates/tze_hud_protocol/tests/resource_upload_integration.rs:108`. |
| `resident-scene-resource-upload` :: Uploaded Font Boundary (zone/component typography remains runtime-owned) | `hud-ooj1.1` | Covered (contract-boundary) | Boundary captured in reconciliation signoff and image-led tranche decision: `openspec/changes/session-resource-upload-rfc0011/reconciliation-hud-ooj1.1.md:13`; explicit requirement text: `openspec/changes/session-resource-upload-rfc0011/specs/resident-scene-resource-upload/spec.md:42`. |
| `session-protocol` delta :: Client/Server envelope allocation includes resident upload family | `hud-ooj1.2`, `hud-ooj1.4` | Covered | Client fields 36-38 and server fields 41/42/49: `crates/tze_hud_protocol/proto/session.proto:92`; integration tests verify wrapping and round-trip: `crates/tze_hud_protocol/tests/resource_upload_integration.rs:47`. |
| `session-protocol` delta :: Widget asset registration remains separate from scene-resource upload | `hud-ooj1.1`, `hud-ooj1.2`, `hud-ooj1.3` | Covered | Widget path retained on field 34 and scene-resource path split to 36-38: `crates/tze_hud_protocol/proto/session.proto:83`; separate runtime handlers: `crates/tze_hud_protocol/src/session_server.rs:2040`. |
| `session-protocol` delta :: Resident upload traffic classes and backpressure | `hud-ooj1.1`, `hud-ooj1.3`, `hud-ooj1.4` | **Partially covered (GAP-1)** | Transactional classification is implemented for upload responses: `crates/tze_hud_protocol/src/session_server.rs:201`; classification test coverage exists: `crates/tze_hud_protocol/src/session_server.rs:7379`. However, per-session upload rate-limit enforcement remains unimplemented in runtime/resource store (`TODO`): `crates/tze_hud_resource/src/types.rs:230`; upload state keeps `started_at` but does not enforce a limit: `crates/tze_hud_resource/src/upload.rs:79`. |
| `resource-store` delta :: Upload validation, capability checks, and concurrent-upload rejection | `hud-ooj1.3`, `hud-ooj1.4` | Covered | Capability denial and error code assertion: `crates/tze_hud_protocol/src/session_server.rs:11723`; concurrent upload limit rejection coverage: `crates/tze_hud_protocol/src/session_server.rs:11898`. |
| `resource-store` delta :: Scene-resource budget charging at reference time (not upload-storage admission) | `hud-ooj1.1`, `hud-ooj1.3`, `hud-ooj1.4` | Covered | Session upload path bypasses texture budget admission checks intentionally: `crates/tze_hud_protocol/src/session_server.rs:4308`; mutation-time budget ownership model remains authoritative: `crates/tze_hud_resource/src/budget.rs:33`; integration proves upload + later scene reference path: `crates/tze_hud_protocol/src/session_server.rs:12231`. |
| `resource-store` delta :: Upload start acknowledgement + response correlation | `hud-ooj1.2`, `hud-ooj1.3`, `hud-ooj1.4` | Covered | Ack payload schema: `crates/tze_hud_protocol/proto/session.proto:641`; runtime ties chunk/complete results back to originating start: `crates/tze_hud_protocol/src/session_server.rs:4344`; correlation tests validate `request_sequence`/`upload_id`: `crates/tze_hud_protocol/src/session_server.rs:11955`. |
| Consumer repair (`/user-test`, Presence Card) uses real resident upload flow | `hud-ooj1.5` | Covered | Helper uploads PNG via `ResourceUploadStart` and waits for correlated `ResourceStored`: `.claude/skills/user-test/scripts/hud_grpc_client.py:874`; Presence Card scenario uses uploaded `avatar_resource_id` in `StaticImageNode`: `.claude/skills/user-test/scripts/presence_card_exemplar.py:539`; operator doc now states real upload path: `docs/exemplar-presence-card-user-test.md:67`. |
| Authoritative main-spec sync after tranche signoff (`v1-mvp-standards`) | `hud-ooj1.1` follow-on | **Partially covered (GAP-2)** | Delta spec exists, but authoritative `v1-mvp-standards` envelope text still reflects pre-upload field map (ops 20-35 only): `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:230`; task checklist still leaves sync step pending: `openspec/changes/session-resource-upload-rfc0011/tasks.md:8`. |

## Gaps Requiring Follow-On Beads

- **GAP-1 (implementation + validation):** resident upload rate limiting/backpressure conformance is not fully closed. Transactional classification is present, but explicit upload rate-limit enforcement and tests for `RESOURCE_RATE_LIMITED`/backpressure shaping are not present.
- **GAP-2 (spec sync):** the session/resource delta for `session-resource-upload-rfc0011` has not yet been synced into the authoritative `v1-mvp-standards` specs.

## Coverage Verdict

1. Core resident upload wire contract, runtime handling, correlation semantics, and `/user-test` consumer conversion are materially implemented by `hud-ooj1.2` through `hud-ooj1.5`.
2. Full closeout is not yet justified because GAP-1 and GAP-2 remain.
3. `hud-ooj1.6` should remain open until those follow-ons land and a gen-2 pass confirms full closure.
4. Epic report bead `hud-ooj1.7` should consume the gen-2 output, not this interim snapshot.

## Coordinator Follow-On Proposals

The worker cannot mutate Beads lifecycle state in this lane. Materialize the following child beads under epic `hud-ooj1`:

1. `title`: `Implement resident upload rate limiting and backpressure conformance`
   `type`: `feature`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-ooj1.6`
   `rationale`: `Close GAP-1 by enforcing configured per-session upload rate limits on resident upload flow (including explicit `RESOURCE_RATE_LIMITED` behavior, currently not implemented) and by documenting/validating transport shaping traffic classes and head-of-line blocking risk per payload variant.`

2. `title`: `Add resident upload rate-limit/backpressure protocol-runtime coverage`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-ooj1.6`
   `rationale`: `Close GAP-1 verification seam by adding tests for transactional chunk semantics under backpressure and explicit rate-limit denial or throttling outcomes.`

3. `title`: `Sync session-resource-upload-rfc0011 deltas into v1-mvp-standards specs`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-ooj1.6`
   `rationale`: `Close GAP-2 by promoting the reconciled upload requirements into authoritative main specs so spec/code truth is unified.`

4. `title`: `Reconcile spec-to-code (gen-2) for resident scene-resource upload`
   `type`: `task`
   `priority`: `1`
   `depends_on`: `discovered-from:hud-ooj1.6`
   `rationale`: `Required terminal pass after the GAP-1/GAP-2 beads land; confirms complete requirement coverage and determines epic closeout readiness.`
