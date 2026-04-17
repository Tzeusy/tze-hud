# Resident Scene-Resource Upload Reconciliation (gen-2)

Date: 2026-04-17
Issue: `hud-qmy8`
Epic: `hud-ooj1` (Expose RFC 0011 scene-resource upload on resident session stream)
Prior pass: `hud-ooj1.6` gen-1 (docs/reconciliations/session_resource_upload_rfc0011_reconciliation_gen1_20260417.md)

## Scope of This Pass

Gen-1 left two open gaps:

- **GAP-1** — Resident upload rate-limit/backpressure conformance not implemented or tested.
- **GAP-2** — Session-resource-upload-rfc0011 deltas not yet promoted into authoritative `v1-mvp-standards` specs.

Two follow-on beads have now merged to `main` before this pass:

| Bead | PR | What it delivered |
|------|----|-------------------|
| `hud-r95t` | #442 | Enforced per-session sliding-window upload rate limiter in session transport; applied backpressure to both chunked and inline upload paths; wired `UploadByteRateLimiter` into `StreamSession`; refactored upload worker to decouple backpressure from session event loop |
| `hud-exnu` | #445 | Added protocol/runtime tests for chunked upload backpressure (`test_resource_upload_chunk_transport_backpressure_from_rate_limit`), heartbeat responsiveness under upload backpressure (`test_resource_upload_backpressure_keeps_heartbeat_responsive`), transactional chunk ordering under backpressure (`test_resource_upload_backpressure_preserves_transactional_chunk_order`), and inline upload backpressure (`test_resource_upload_inline_transport_backpressure_from_rate_limit`) |
| `hud-lyl7` | #444 | Promoted upload delta requirements into `v1-mvp-standards/specs/session-protocol/spec.md` and `v1-mvp-standards/specs/resource-store/spec.md`; marked `tasks.md` item 1.6 complete; added HOLB mitigation note to traffic-class spec |

## Inputs Audited

- RFC 0011 (`about/legends-and-lore/rfcs/0011-resource-store.md`)
- Gen-1 report (`docs/reconciliations/session_resource_upload_rfc0011_reconciliation_gen1_20260417.md`)
- `openspec/changes/session-resource-upload-rfc0011/specs/resident-scene-resource-upload/spec.md`
- `openspec/changes/session-resource-upload-rfc0011/specs/session-protocol/spec.md`
- `openspec/changes/session-resource-upload-rfc0011/specs/resource-store/spec.md`
- `openspec/changes/session-resource-upload-rfc0011/tasks.md`
- `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
- `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md`
- `crates/tze_hud_protocol/proto/session.proto`
- `crates/tze_hud_protocol/src/session_server.rs`
- `crates/tze_hud_protocol/tests/resource_upload_integration.rs`
- `crates/tze_hud_resource/src/types.rs`
- `crates/tze_hud_resource/src/upload.rs`
- `crates/tze_hud_resource/src/budget.rs`
- `.claude/skills/user-test/scripts/hud_grpc_client.py`
- `.claude/skills/user-test/scripts/presence_card_exemplar.py`

---

## Requirement-to-Code Coverage Matrix (gen-2)

### Group A: Wire Protocol and Envelope Allocation

| Requirement | RFC 0011 § | Implementing beads | Status | Evidence |
|---|---|---|---|---|
| Session-stream multiplexing: no separate upload RPC | §3.1 | `hud-ooj1.2`, `hud-ooj1.3` | **Covered** | Upload worker dispatched inside primary session stream handler: `crates/tze_hud_protocol/src/session_server.rs:2658`. No secondary RPC exists in `proto/session.proto`. |
| ClientMessage field allocation (ResourceUploadStart=36, ResourceUploadChunk=37, ResourceUploadComplete=38) | §3.4, §10 | `hud-ooj1.2` | **Covered** | Proto fields confirmed: `crates/tze_hud_protocol/proto/session.proto:92–94`. Round-trip test: `crates/tze_hud_protocol/tests/resource_upload_integration.rs:47`. |
| ServerMessage field allocation (ResourceUploadAccepted=41, ResourceStored=42, ResourceErrorResponse=49) | §3.4, §10 | `hud-ooj1.2` | **Covered** | Proto fields confirmed: `crates/tze_hud_protocol/proto/session.proto:161–186`. Round-trip test: `crates/tze_hud_protocol/tests/resource_upload_integration.rs:108`. |
| Widget asset registration path separate from scene-resource upload (WidgetAssetRegister=34) | §2.2a, §9.1 | `hud-ooj1.1`, `hud-ooj1.2`, `hud-ooj1.3` | **Covered** | Separate proto fields (34 vs 36–38); separate runtime dispatch branches: `crates/tze_hud_protocol/src/session_server.rs:2040` (widget), `2658` (scene resource). |

### Group B: Upload Flow and Correlation

| Requirement | RFC 0011 § | Implementing beads | Status | Evidence |
|---|---|---|---|---|
| Upload start acknowledgement: `ResourceUploadAccepted` with `request_sequence` and `upload_id` before first chunk | §3.2, §3.6 | `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | Runtime emits `ResourceUploadAccepted`: `crates/tze_hud_protocol/src/session_server.rs:1998–2002`. Test: `test_resource_upload_chunked_ack_then_complete` at line 13940. |
| Dedup hit: immediate `ResourceStored` without `ResourceUploadAccepted` or chunks | §3.2, §3.6 | `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | Dedup short-circuit path: `crates/tze_hud_protocol/src/session_server.rs`. Test: `test_resource_upload_inline_and_dedup_short_circuit` at line 13875. |
| Inline fast-path (≤64 KiB inline_data, no chunk/complete sequence) | §3.3 | `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | Inline path processed at `crates/tze_hud_protocol/src/session_server.rs:1041`. Test: `test_resource_upload_inline_and_dedup_short_circuit` at line 13875. |
| Chunked upload flow (start → ack → chunks → complete) | §3.2 | `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | Full chunked path with in-flight state tracking in upload worker: `crates/tze_hud_protocol/src/session_server.rs:1142`. Test: `test_resource_upload_chunked_ack_then_complete` at line 13940. |
| `ResourceStored` correlation: carries initiating `request_sequence` and optional `upload_id` | §3.5, §3.6 | `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | `send_resource_stored` helper attaches correlation fields: `crates/tze_hud_protocol/src/session_server.rs:5322`. Test: `test_resource_upload_chunked_success_correlates_by_request_sequence` at line 14072. |
| `ResourceErrorResponse` carries `request_sequence`, stable `error_code`, `message`, `context`, `hint`, and `upload_id` when applicable | §3.5, §3.6 | `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | Error surface emits structured `ResourceErrorResponse` at `crates/tze_hud_protocol/src/session_server.rs:5386`. `RuntimeError` reserved for malformed envelope violations. |

### Group C: Upload Validation

| Requirement | RFC 0011 § | Implementing beads | Status | Evidence |
|---|---|---|---|---|
| Capability check: `upload_resource` capability required | §3.5 (1), §5.2 | `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | Capability mapped at `crates/tze_hud_protocol/src/session_server.rs:3915`; denial tested: `test_resource_upload_start_requires_upload_resource_capability` at line 13840. |
| Hash integrity: computed BLAKE3 must match `expected_hash` | §3.5 (2) | `hud-ooj1.3` | **Covered** | Hash mismatch maps to `StoreResourceError::HashMismatch` → error code 6: `crates/tze_hud_protocol/src/session_server.rs:5273`. |
| Per-resource size limit enforcement | §3.5 (3), §8.1 | `hud-ooj1.3` | **Covered** | `StoreResourceError::SizeExceeded` → error code 3: `crates/tze_hud_protocol/src/session_server.rs:5279`. |
| v1 resource type acceptance/rejection | §3.5 (5), §2.1 | `hud-ooj1.3` | **Covered** | `StoreResourceError::UnsupportedType` → error code 4: `crates/tze_hud_protocol/src/session_server.rs:5280`. |
| Decode validation (images decode, fonts parse) | §3.5 (6) | `hud-ooj1.3` | **Covered** | `StoreResourceError::DecodeError` → error code 5: `crates/tze_hud_protocol/src/session_server.rs:5281`. |
| Concurrent upload limit: max 4 in-flight, 5th rejected with `RESOURCE_TOO_MANY_UPLOADS` | §3.7 | `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | `StoreResourceError::TooManyUploads` → error code 8: `crates/tze_hud_protocol/src/session_server.rs:5289`. Test: `test_resource_upload_chunked_concurrent_limit_rejected` at line 14015. |
| Decoded texture budget NOT rejected at upload time; only at mutation reference time | §11.2 | `hud-ooj1.1`, `hud-ooj1.3`, `hud-ooj1.4` | **Covered** | Upload-time budget bypass documented at `crates/tze_hud_protocol/src/session_server.rs`; budget enforcer operates in mutation pipeline: `crates/tze_hud_resource/src/budget.rs:33`. Integration test: `test_resident_upload_then_static_image_references_uploaded_resource_id` at line 14348. |

### Group D: Traffic Classes and Backpressure (GAP-1 closure)

| Requirement | RFC 0011 § | Implementing beads | Status | Evidence |
|---|---|---|---|---|
| Upload control and response messages use transactional traffic class | §3.4 | `hud-ooj1.3`, `hud-ooj1.4`, `hud-exnu` | **Covered** | `classify_server_payload` maps `ResourceUploadAccepted`, `ResourceStored`, `ResourceErrorResponse` to `TrafficClass::Transactional`: `crates/tze_hud_protocol/src/session_server.rs:204–208`. Unit test: `test_traffic_class_routing` at line 8922. |
| `ResourceUploadChunk` transactional (ordered, reliable, never silently dropped) | §3.4 | `hud-ooj1.3`, `hud-exnu` | **Covered** | Chunk ordering preserved via sequential in-flight tracking in upload worker. Test: `test_resource_upload_backpressure_preserves_transactional_chunk_order` at line 13566. |
| Upload rate-limit enforcement (1 MiB/s sliding window per session, back-pressure via gRPC flow control) | §8.4 | `hud-r95t`, `hud-exnu` | **Covered** | `UploadByteRateLimiter` with 1-second sliding window implemented at `crates/tze_hud_protocol/src/session_server.rs:819–879`. Applied to both chunk path (line 1142) and inline path (line 1041) via `apply_upload_transport_backpressure`. Configured from `resource_store.upload_rate_limit_bytes_per_sec()`. Tests: `test_resource_upload_chunk_transport_backpressure_from_rate_limit` at line 13338; `test_resource_upload_inline_transport_backpressure_from_rate_limit` at line 13729. |
| Heartbeat responsiveness preserved under upload backpressure | §3.1 (implicit) | `hud-r95t`, `hud-exnu` | **Covered** | Upload backpressure decoupled from session event loop. Test: `test_resource_upload_backpressure_keeps_heartbeat_responsive` at line 13446. |
| Note: RFC 0011 §8.4 specifies `RESOURCE_RATE_LIMITED` error code (11) | §8.4 | `hud-ooj1.2`, `hud-r95t` | **Covered (transport-delay approach)** | Error code 11 (`RESOURCE_RATE_LIMITED`) defined in `proto/session.proto:689`. Implementation uses transport-level delay (sleep/yield) rather than emitting the error code explicitly, per the spec's "MAY back-pressure" language. This is compliant with RFC 0011 §8.4 and the v1-mvp-standards traffic class spec: "the runtime MAY delay reading or acknowledging chunk progress under backpressure." |

### Group E: Authoritative Spec Sync (GAP-2 closure)

| Requirement | Source spec | Implementing bead | Status | Evidence |
|---|---|---|---|---|
| Upload field allocation (36–38 client, 41/42/49 server) in v1-mvp-standards session-protocol spec | session-resource-upload-rfc0011 delta | `hud-lyl7` | **Covered** | Upload fields now in `ClientMessage`/`ServerMessage` envelope requirement: `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:230`. |
| Resident upload handshake requirements in v1-mvp-standards session-protocol spec | session-resource-upload-rfc0011 delta | `hud-lyl7` | **Covered** | "Resident Scene-Resource Upload Handshake" requirement added at `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:793`. |
| Upload traffic class (transactional) and backpressure in v1-mvp-standards session-protocol spec | session-resource-upload-rfc0011 delta | `hud-lyl7` | **Covered** | "Traffic Class Routing" requirement updated to include all upload message types: `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:294`. "Resident Upload Traffic Classes and Backpressure" requirement added at line 823. |
| Upload validation, acknowledgement, correlation, dedup, rate-limit in v1-mvp-standards resource-store spec | session-resource-upload-rfc0011 delta | `hud-lyl7` | **Covered** | All upload requirements present in `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md` including: Upload Protocol, Start Acknowledgement, Response Correlation, Chunked Flow, Small Resource Fast Path, Upload Validation, Deduplication, Concurrent Upload Limits, and Upload Rate Limiting. |
| tasks.md item 1.6 marked complete | session-resource-upload-rfc0011 tasks | `hud-lyl7` | **Covered** | `openspec/changes/session-resource-upload-rfc0011/tasks.md:8` changed from `[ ]` to `[x]`. |

### Group F: Consumer Repair

| Requirement | RFC 0011 § | Implementing bead | Status | Evidence |
|---|---|---|---|---|
| `/user-test` grpc client uses real resident upload flow | §3.1, §3.2 | `hud-ooj1.5` | **Covered** | `upload_avatar_png` method uses `ResourceUploadStart` with `inline_data` and awaits correlated `ResourceStored`: `.claude/skills/user-test/scripts/hud_grpc_client.py:884–903`. |
| Presence card exemplar uses uploaded `ResourceId` in `StaticImageNode` | §3.1 | `hud-ooj1.5` | **Covered** | `avatar_resource_id = await client.upload_avatar_png(avatar_png)` then passed to `StaticImageNodeProto`: `.claude/skills/user-test/scripts/presence_card_exemplar.py:488`. |
| Uploaded font boundary: scene-node scope only; zone/component typography remains runtime-owned | §7.1 | `hud-ooj1.1` | **Covered (contract-boundary)** | Requirement text in resident-scene-resource-upload spec: `openspec/changes/session-resource-upload-rfc0011/specs/resident-scene-resource-upload/spec.md:42`. First tranche is image-led; no agent-uploaded font consumer required for v1 proof. |

### Group G: Remaining RFC 0011 Requirements (scope boundary check)

The following RFC 0011 requirements are outside the hud-ooj1 epic's v1 scope or are deferred post-v1 as documented:

| Requirement | RFC 0011 § | Scope | Notes |
|---|---|---|---|
| Reference counting, GC candidacy/grace period, GC cycle | §4, §6 | Pre-existing / separate crate | Implemented in `crates/tze_hud_resource/src/upload.rs` and related; not in scope for this upload-seam epic |
| Font asset management (system/bundled/agent fonts, fallback chain, font cache) | §7 | Pre-existing / separate concern | Font subsystem in resource crate; contract documented in spec but not part of resident-upload-seam beads |
| Budget accounting at mutation reference time | §11 | Pre-existing / separate crate | `crates/tze_hud_resource/src/budget.rs`; confirmed correct behavior in gen-1 and integration test at line 14348 |
| Persistence split (scene-node ephemeral, widget SVG durable) | §9.1 | Separate widget crate path | Widget asset durability via `WidgetAssetRegister` path (separate from scene-resource upload); scene-node ephemerality by design |
| ResourceQuery RPC | §10 | Not in v1 epic scope | Proto defines `ResourceQuery`/`ResourceQueryResult` but no runtime handler required for hud-ooj1 |
| Post-v1: persistence, video/audio types, WebP/AVIF, GPU texture compression | §9.2, §2.3, §18 | Explicitly post-v1 | Deferred per RFC 0011 §18 open questions |

---

## GAP Resolution Assessment

### GAP-1 (Implementation + Validation): CLOSED

The gen-1 report identified that per-session upload rate-limit enforcement and tests for rate-limit/backpressure outcomes were missing. Status as of this pass:

**Implementation (hud-r95t / PR #442):**
- `UploadByteRateLimiter` struct with 1-second sliding window: `crates/tze_hud_protocol/src/session_server.rs:819`
- `apply_upload_transport_backpressure` async function: line 5300
- Both chunked upload path (line 1142) and inline path (line 1041) pass through the limiter
- `StreamSession.resource_upload_rate_limiter` field per-session: line 1338
- Limit value wired from `resource_store.upload_rate_limit_bytes_per_sec()`: lines 2351–2384, 2501–2546
- Upload backpressure decoupled from session event loop (no HOL blocking of heartbeats)
- `types.rs` `TODO` for rate limiting replaced with implementation-accurate doc comment: `crates/tze_hud_resource/src/types.rs:228`

**Test coverage (hud-exnu / PR #445):**
- `test_resource_upload_chunk_transport_backpressure_from_rate_limit`: chunked path backpressure verified
- `test_resource_upload_backpressure_keeps_heartbeat_responsive`: session liveness verified under backpressure
- `test_resource_upload_backpressure_preserves_transactional_chunk_order`: chunk ordering preserved
- `test_resource_upload_inline_transport_backpressure_from_rate_limit`: inline path backpressure verified
- `upload_byte_rate_limiter_enforces_sliding_window` unit test: limiter mechanics verified
- `upload_byte_rate_limiter_zero_limit_is_unbounded`: zero-limit escape hatch verified

**Design note:** The implementation uses transport-level delay (sleep/yield) rather than emitting a `RESOURCE_RATE_LIMITED` error response. RFC 0011 §8.4 specifies the error code exists but uses permissive language ("MAY back-pressure"). The v1-mvp-standards spec says "the runtime MAY delay reading or acknowledging chunk progress under backpressure, but it SHALL NOT downgrade upload chunks to a droppable class." The delay approach is compliant and aligns with gRPC flow control semantics. The `RESOURCE_RATE_LIMITED` error code (11) remains defined in the proto for future use or for explicit rejection policies.

### GAP-2 (Spec Sync): CLOSED

The gen-1 report identified that the `v1-mvp-standards` session-protocol and resource-store specs had not been updated with the upload delta requirements. Status as of this pass:

**Spec sync (hud-lyl7 / PR #444):**
- `v1-mvp-standards/specs/session-protocol/spec.md`: Updated `ClientMessage`/`ServerMessage` envelope requirement to include fields 36–38 and 41/42/49; added "Resident Scene-Resource Upload Handshake" requirement; updated "Traffic Class Routing" to include all upload message types; added "Resident Upload Traffic Classes and Backpressure" requirement with HOLB mitigation note.
- `v1-mvp-standards/specs/resource-store/spec.md`: Added Upload Protocol, Start Acknowledgement, Response Correlation, Chunked Upload Flow, Small Resource Fast Path, Upload Validation, Content-Addressed Deduplication, Concurrent Upload Limits, and Upload Rate Limiting requirements (all with scenario tests).
- `openspec/changes/session-resource-upload-rfc0011/tasks.md`: Item 1.6 marked complete.

---

## Coverage Verdict

**All requirements in the hud-ooj1 epic scope are now fully covered.**

| Category | Count | Status |
|---|---|---|
| Wire protocol / envelope | 4 | All covered |
| Upload flow and correlation | 6 | All covered |
| Upload validation | 7 | All covered |
| Traffic classes and backpressure (GAP-1) | 5 | All covered |
| Authoritative spec sync (GAP-2) | 5 | All covered |
| Consumer repair | 3 | All covered |
| Out-of-scope / post-v1 | 6 | Deferred by design |

No remaining gaps. The two gaps identified in gen-1 are closed:
- GAP-1 fully closed by `hud-r95t` (implementation) + `hud-exnu` (test coverage)
- GAP-2 fully closed by `hud-lyl7` (spec sync)

---

## Closeout Readiness

**Verdict: ready-to-close-epic**

All `hud-ooj1` child tasks are closed (`hud-ooj1.1` through `hud-ooj1.6`). All follow-on gap beads are closed (`hud-r95t`, `hud-exnu`, `hud-lyl7`). This gen-2 pass confirms no remaining coverage gaps. The coordinator may close `hud-ooj1` (the epic) and `hud-ooj1.7` (the epic report bead) after consuming this report.
