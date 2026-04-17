# Resident Scene-Resource Upload Epic Report

**Epic:** `hud-ooj1` — Expose RFC 0011 scene-resource upload on resident session stream
**Date:** 2026-04-17
**Status:** Closed

---

## 1. Overview

This epic repaired a broken v1 contract chain: doctrine and RFC 0011 described a
resident gRPC session stream as the upload path for scene-node images and fonts,
but the checked-in session schema, runtime, and exemplar consumers had none of
it. The uploaded field range was unallocated, the start-acknowledgement message
did not exist, rate-limit enforcement was a `TODO`, and resident user-test flows
relied on placeholders. Over eight beads and two reconciliation passes the epic
delivered the complete resident upload seam — protocol schema, runtime handler,
conformance coverage, consumer repair, rate-limit enforcement with backpressure,
and authoritative spec sync — making it possible for a resident agent to upload
PNG images over `HudSession`, receive a correlated `ResourceId`, and immediately
reference that id from a live scene node.

---

## 2. Implementation Chronicle

### hud-ooj1.1 — Contract reconciliation (PR: `6e8ef33`)

Spec-first pass reconciling RFC 0011, RFC 0005, the v1 session-protocol spec,
and the resource-store spec. Identified three concrete contract bugs: the missing
`ResourceUploadAccepted` start-acknowledgement, the misaligned error shape in
`ResourceErrorResponse`, and the stale combined-envelope registry in RFC 0005.
Produced the reconciliation signoff document
(`openspec/changes/session-resource-upload-rfc0011/reconciliation-hud-ooj1.1.md`),
established the split `ClientMessage`/`ServerMessage` model as authoritative, and
defined the v1 font-boundary decision (scene-node scope only; zone/component
typography remains runtime-owned).

### hud-ooj1.2 — Schema extension (PR #428)

Added the resident upload message family to `crates/tze_hud_protocol/proto/session.proto`:
client fields `ResourceUploadStart=36`, `ResourceUploadChunk=37`,
`ResourceUploadComplete=38`; server fields `ResourceUploadAccepted=41`,
`ResourceStored=42`, `ResourceErrorResponse=49`. Regenerated bindings and
updated compile surfaces. Envelope round-trip tests established the correlation
fields (`request_sequence`, `upload_id`).

### hud-ooj1.3 — Runtime handler (PR #430)

Wired the full upload dispatch inside the primary session stream handler in
`crates/tze_hud_protocol/src/session_server.rs`. Covered:
- Capability check (`upload_resource`) before any state is allocated.
- Inline fast-path (≤ 64 KiB `inline_data`): hash, decode, store, emit
  `ResourceStored` without an ack/chunk cycle.
- Dedup short-circuit: immediate `ResourceStored` if the hash is already known.
- Chunked path: emit `ResourceUploadAccepted` (with `request_sequence` and
  `upload_id`) before chunks, collect chunks in upload worker, verify hash on
  `ResourceUploadComplete`, emit `ResourceStored` or `ResourceErrorResponse`.
- Structured `ResourceErrorResponse` with stable error codes, message, context,
  hint, and correlation fields.
- Upload traffic classes routed through `classify_server_payload` as
  `TrafficClass::Transactional`.
- Budget charging deferred to mutation-reference time (not upload-storage time),
  matching RFC 0011 §11.

### hud-ooj1.4 — Conformance coverage (PR #434)

Added protocol and runtime tests in
`crates/tze_hud_protocol/tests/resource_upload_integration.rs` and inline in
`session_server.rs`:
- Envelope allocation and round-trip for all upload message variants.
- Inline fast-path, dedup short-circuit, capability denial.
- Chunked ack-then-complete flow, concurrent-upload limit rejection (max 4
  in-flight), correlation by `request_sequence`.
- Integration test uploading a resource and referencing it from a `StaticImageNode`.
- Traffic-class assignment unit test.

### hud-ooj1.5 — Consumer repair (PR #436)

Converted the `/user-test` gRPC client and Presence Card exemplar from
placeholder behavior to the real session-stream upload path:
- `upload_avatar_png` in `.claude/skills/user-test/scripts/hud_grpc_client.py`
  now sends `ResourceUploadStart` with `inline_data` and awaits a correlated
  `ResourceStored` response.
- `presence_card_exemplar.py` passes the resulting `avatar_resource_id` to
  `StaticImageNodeProto`, completing the live resident upload proof.

### hud-ooj1.6 — Gen-1 reconciliation (PR #440 / `13548c2`)

First spec-to-code pass after the implementation beads merged. Confirmed core
upload wire contract, runtime correlation, and consumer repair were solid.
Identified two open gaps requiring follow-ons:
- **GAP-1:** per-session upload rate-limit enforcement and backpressure tests
  were missing (only a `TODO` in `types.rs`).
- **GAP-2:** the reconciled upload delta had not been promoted into the
  authoritative `v1-mvp-standards` specs.

Produced: `docs/reconciliations/session_resource_upload_rfc0011_reconciliation_gen1_20260417.md`

### Follow-ons: hud-r95t, hud-exnu, hud-lyl7 (PRs #442, #445, #444)

Three gap-closure beads, all landed on `main` before the gen-2 pass:

| Bead | What it delivered |
|------|-------------------|
| `hud-r95t` (#442) | `UploadByteRateLimiter` (1-second sliding window per session) in `session_server.rs:819`; wired into both chunked and inline paths; backpressure decoupled from session event loop; `TODO` in `types.rs` replaced with accurate doc comment. |
| `hud-exnu` (#445) | Protocol/runtime tests for chunked backpressure, heartbeat liveness under backpressure, transactional chunk ordering under backpressure, inline backpressure, sliding-window unit test, zero-limit escape hatch. |
| `hud-lyl7` (#444) | Promoted upload delta requirements into `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` (fields 36–38/41/42/49, upload handshake, traffic class, backpressure) and `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md` (upload protocol, validation, dedup, rate-limit). Marked `tasks.md` item 1.6 complete. |

### hud-qmy8 — Gen-2 reconciliation (PR #447 / `1511c42`)

Second spec-to-code pass confirmed full requirement coverage across seven
groups (wire protocol, upload flow, validation, traffic classes, spec sync,
consumer repair, out-of-scope boundaries). Both gaps closed. Verdict:
`ready-to-close-epic`.

Produced: `docs/reconciliations/session_resource_upload_rfc0011_reconciliation_gen2_20260417.md`

---

## 3. Final Capabilities

After this epic, a resident agent connected to `HudSession` can:

- **Upload images over the primary session stream.** Send
  `ResourceUploadStart` + optional `ResourceUploadChunk` sequence +
  `ResourceUploadComplete` (or use the inline fast-path for small resources) and
  receive a correlated `ResourceStored` containing a stable `ResourceId`.

- **Use content-addressed deduplication.** Uploading a resource whose BLAKE3
  hash already exists in the store returns an immediate `ResourceStored` with no
  ack/chunk cycle needed.

- **Reference the returned `ResourceId` in a scene node.** A
  `StaticImageNode` (or any other resource-referencing node) passed in a
  subsequent `ApplyMutation` can use the `ResourceId` directly; the compositor
  will resolve it at render time.

- **Receive correlated, structured upload errors.** `ResourceErrorResponse`
  carries a stable numeric error code, human-readable `message`, `context`,
  `hint`, the initiating `request_sequence`, and (where applicable) `upload_id`.
  Error codes cover capability denial, size exceeded, unsupported type, hash
  mismatch, decode failure, too-many-concurrent-uploads, and rate-limited.

- **Stay within per-session upload budgets.** The runtime enforces a 1 MiB/s
  sliding-window rate limit per session. Excess bandwidth causes transport-level
  backpressure (gRPC flow control delay), not dropped chunks. The session event
  loop (including heartbeats) is not blocked by upload backpressure.

Widget SVG asset registration (`WidgetAssetRegister`, field 34) remains a
separate path from scene-resource upload, unaffected by this epic.

---

## 4. Verification Summary

### Protocol conformance (`crates/tze_hud_protocol/tests/resource_upload_integration.rs`)

- `ResourceUploadStart`/`ResourceUploadChunk`/`ResourceUploadComplete` envelope
  allocation and round-trip (fields 36/37/38).
- `ResourceUploadAccepted`/`ResourceStored`/`ResourceErrorResponse` envelope
  allocation and round-trip (fields 41/42/49).

### Runtime tests (inline in `crates/tze_hud_protocol/src/session_server.rs`)

| Test | Coverage |
|------|----------|
| `test_resource_upload_start_requires_upload_resource_capability` | Capability denial |
| `test_resource_upload_inline_and_dedup_short_circuit` | Inline fast-path; dedup hit |
| `test_resource_upload_chunked_ack_then_complete` | Chunked flow, ack before first chunk |
| `test_resource_upload_chunked_concurrent_limit_rejected` | Max-4-in-flight enforcement |
| `test_resource_upload_chunked_success_correlates_by_request_sequence` | Response correlation |
| `test_resource_upload_chunk_transport_backpressure_from_rate_limit` | Chunked path backpressure |
| `test_resource_upload_backpressure_keeps_heartbeat_responsive` | Session liveness under backpressure |
| `test_resource_upload_backpressure_preserves_transactional_chunk_order` | Chunk ordering under backpressure |
| `test_resource_upload_inline_transport_backpressure_from_rate_limit` | Inline path backpressure |
| `upload_byte_rate_limiter_enforces_sliding_window` | Sliding-window unit |
| `upload_byte_rate_limiter_zero_limit_is_unbounded` | Zero-limit escape hatch |
| `test_traffic_class_routing` | Upload messages classified as Transactional |
| `test_resident_upload_then_static_image_references_uploaded_resource_id` | End-to-end: upload → scene reference |

---

## 5. Residual Risk / Deferred Seams

The following items are intentionally outside this epic's scope, documented in
RFC 0011:

| Deferred item | RFC 0011 §§ | Notes |
|---|---|---|
| Post-v1 resource types: `VIDEO_H264`, `VIDEO_VP9`, `AUDIO_OPUS`, `AUDIO_AAC`, `WASM_MODULE` | §2.3, §18 | Future RFC when corresponding node types ship |
| WebP / AVIF image format support | §18 (open question 4) | Candidate for v1.1 |
| GPU texture compression (BC7/ASTC transcoding on upload) | §18 (open question 1) | Deferred to post-v1 profiling |
| Persistent resource store (flat-file or embedded-DB format, cleanup policy) | §9.2 | Scene-node resources remain in-memory / ephemeral in v1 |
| `ResourceQuery` / `ResourceQueryResult` RPC handler | §10 | Proto fields exist; no runtime handler required for v1 |
| Agent-uploaded fonts in scene-node/tile scope | §7.1 | Contract boundary established; image-led first tranche; font consumer not required for v1 proof |
| Reference counting / GC arc and grace-period tuning | §4, §6 | Implemented in `crates/tze_hud_resource`; not modified by this epic |
| `RESOURCE_RATE_LIMITED` explicit rejection (error code 11) | §8.4 | Error code defined in proto; implementation uses transport-delay backpressure per §8.4 MAY language; explicit rejection is a post-v1 policy option |

---

## 6. Updated Artifacts

### Spec files updated

| File | What changed |
|------|--------------|
| `crates/tze_hud_protocol/proto/session.proto` | Added client fields 36–38, server fields 41/42/49, `ResourceUploadStart`, `ResourceUploadChunk`, `ResourceUploadComplete`, `ResourceUploadAccepted`, `ResourceStored` (upload variant), `ResourceErrorResponse` message definitions |
| `openspec/changes/session-resource-upload-rfc0011/specs/resident-scene-resource-upload/spec.md` | New delta spec; session-stream upload path, ack/correlation contract, inline fast-path, font boundary |
| `openspec/changes/session-resource-upload-rfc0011/specs/session-protocol/spec.md` | Delta: upload envelope allocation, traffic class, upload handshake requirements |
| `openspec/changes/session-resource-upload-rfc0011/specs/resource-store/spec.md` | Delta: upload protocol, validation, dedup, rate-limit, concurrent-limit requirements |
| `openspec/changes/session-resource-upload-rfc0011/tasks.md` | Section 1 all checked; sections 2–5 reflect tasks that were completed by implementation beads (not retroactively ticked in the file) |
| `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` | Updated `ClientMessage`/`ServerMessage` envelope to include upload fields; added upload handshake, traffic class, and backpressure requirements |
| `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md` | Added Upload Protocol section with all sub-requirements (validation, dedup, rate-limit, concurrent limits) |
| `.claude/skills/user-test/scripts/hud_grpc_client.py` | `upload_avatar_png` now uses real `ResourceUploadStart` + `ResourceStored` flow |
| `.claude/skills/user-test/scripts/presence_card_exemplar.py` | Uses `avatar_resource_id` from upload in `StaticImageNode` |

### RFC status

RFC 0011 (`about/legends-and-lore/rfcs/0011-resource-store.md`) was the
canonical design source for this epic. No changes were made to the RFC text;
the implementation follows the RFC, with the
`ResourceUploadAccepted` start-ack gap resolved in the delta spec at
`openspec/changes/session-resource-upload-rfc0011/reconciliation-hud-ooj1.1.md`.

### Reconciliation documents

- **Gen-1 reconciliation** (hud-ooj1.6):
  `docs/reconciliations/session_resource_upload_rfc0011_reconciliation_gen1_20260417.md`
- **Gen-2 reconciliation** (hud-qmy8):
  `docs/reconciliations/session_resource_upload_rfc0011_reconciliation_gen2_20260417.md`
- **Direction report** (pre-epic):
  `docs/reconciliations/session_resource_upload_rfc0011_direction_report_20260410.md`
- **Backlog materialization** (pre-epic):
  `docs/reconciliations/session_resource_upload_rfc0011_backlog_materialization_20260410.md`
