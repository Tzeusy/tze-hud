# WebRTC/Media V1 Protocol + Schema/Snapshot Delta Contract (WM-S2b)

Date: 2026-04-09
Issue: `hud-nn9d.8`
Parent epic: `hud-nn9d`
Depends on: `hud-nn9d.6` (WM-S1), `hud-nn9d.7` (WM-S2a)

## Purpose

Define the concrete wire/schema deltas required for the first bounded post-v1
media ingress slice, under the already-chosen signaling shape from WM-S2a:
**session-stream envelope extension**, not a separate media RPC.

This contract is normative for:

1. session envelope field allocation in the post-v1 range,
2. protobuf schema deltas needed for media publish parity,
3. reconnect/snapshot semantics for active media publications,
4. backward-compatible rollout and downgrade behavior.

It does not define zone identity/class constraints (WM-S2c), activation budgets
(WM-S3), privacy/operator policy (WM-S3b), compositor behavior (WM-S3c), or
validation thresholds (WM-S4).

## Inputs And Current Seams

1. Session envelope currently allocates v1 payloads through fields `10-48`,
   keeps `49` open server-side, and reserves `50-99` for post-v1
   (`openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:230`, `:721`).
2. `ZonePublish` is currently v1 field `25` and does not carry media timing or
   classification fields (`crates/tze_hud_protocol/proto/session.proto:54`, `:525`).
3. `ZoneContent` wire payload currently lacks `StaticImage` and
   `VideoSurfaceRef` variants even though scene types include both
   (`crates/tze_hud_protocol/proto/types.proto:198`, `crates/tze_hud_scene/src/types.rs:1717`).
4. `ZonePublishRecordProto` omits `expires_at_wall_us`,
   `content_classification`, and `breakpoints`, causing snapshot parity loss
   (`crates/tze_hud_protocol/proto/types.proto:313`,
   `crates/tze_hud_scene/src/types.rs:1734`).
5. Resume behavior is snapshot-first in v1: `SessionResumeResult` followed by
   full `SceneSnapshot` (`crates/tze_hud_protocol/proto/session.proto:316`,
   `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:450`).

## Contract Decision: Keep Zone Publish; Add Media Signaling Payloads In 50-99

WM-S2a selected the session-envelope extension path. WM-S2b makes that concrete:

1. Keep existing `ZonePublish` (`ClientMessage` field 25) as the **content
   publication path**.
2. Add new media-transport signaling payloads in the reserved post-v1
   `50-99` envelope range.
3. Do not add `rpc MediaSignaling(...)` in this tranche.

## Session Envelope Field Allocation (Post-v1)

### ClientMessage oneof payload allocations

1. `50`: `MediaIngressOpen`
2. `51`: `MediaIngressClose`

### ServerMessage oneof payload allocations

1. `50`: `MediaIngressOpenResult`
2. `51`: `MediaIngressState`
3. `52`: `MediaIngressCloseNotice`

### Message intent and traffic class

1. `MediaIngressOpen` (transactional):
- admission request for one-way inbound visual stream binding to a runtime zone,
- carries transport descriptor + surface identity + timing/classification intent.
2. `MediaIngressOpenResult` (transactional):
- deterministic accept/reject result with machine-readable denial code,
- returns runtime-assigned `stream_epoch` for reconnect consistency checks.
3. `MediaIngressState` (state-stream):
- coalescible state updates for active stream health/degradation,
- latest state wins; intermediate states may be skipped.
4. `MediaIngressClose` (transactional):
- publisher-initiated teardown intent.
5. `MediaIngressCloseNotice` (transactional):
- runtime-initiated termination notice (policy disable, lease revoke, budget gate,
  transport failure).

## Protobuf Schema Deltas

### `types.proto` deltas

1. Extend `ZoneContent` oneof:
- `5`: `StaticImageRef`
- `6`: `VideoSurfaceRef`
2. Add message `VideoSurfaceRef`:
- `bytes surface_id = 1` (16-byte scene/runtime surface identity)
3. Add message `StaticImageRef`:
- `bytes resource_id = 1` (content-addressed resource identity)
4. Extend `ZonePublishRecordProto` with parity fields:
- `6`: `uint64 expires_at_wall_us` (`0` = none)
- `7`: `string content_classification` (empty = none)
- `8`: `repeated uint64 breakpoints`

### `session.proto` deltas

1. Keep `ZonePublish` at field `25`; extend message fields additively:
- `6`: `uint64 present_at_wall_us` (`0` = immediate)
- `7`: `uint64 expires_at_wall_us` (`0` = no expiry)
- `8`: `string content_classification` (empty = unset)
2. Define the new `MediaIngress*` messages listed above.
3. Add explicit oneof comments + `reserved` declarations for any removed
   experimental tags/names during rollout to prevent wire reuse.

## Snapshot + Reconnect Contract For Bounded Media Ingress

### Snapshot inclusion requirements

1. `SceneSnapshot.snapshot_json` MUST include media publications in
   `zone_registry.active_publications` with:
- `ZoneContent::VideoSurfaceRef`,
- `expires_at_wall_us`,
- `content_classification`,
- `breakpoints` when relevant.
2. Snapshot MUST remain deterministic and checksum-stable under the existing
   RFC 0001 serialization rules.

### Snapshot exclusion requirements

1. Snapshot MUST NOT embed transport-session internals (SDP/ICE blobs,
   DTLS/SRTP state, decoder handles, frame buffers).
2. Snapshot MUST carry the declarative publication state only; transport
   re-establishment remains a signaling concern on resume.

### Resume behavior

1. On accepted `SessionResume`, runtime sends `SessionResumeResult`, then
   `SceneSnapshot` (existing ordering contract unchanged).
2. Any pre-disconnect media transport is treated as non-authoritative after
   resume until a post-resume signaling confirmation occurs.
3. Runtime and publisher reconcile using `stream_epoch` from
   `MediaIngressOpenResult`/`MediaIngressState`:
- matching epoch: stream may continue without re-open,
- mismatched/missing epoch: publisher MUST issue a fresh `MediaIngressOpen`.
4. No WAL/delta replay dependency is introduced for this tranche; snapshot-first
   reconnect remains valid.

## Backward-Compatibility And Downgrade Rules

1. All changes are additive-only in protobuf:
- no renumbering of existing tags,
- no oneof arm reuse,
- no semantic reassignment of existing fields.
2. Media ingress path is gated by both:
- negotiated protocol version advertising WM-S2b support,
- granted media-ingress capability scope.
3. If either gate is missing:
- client MUST NOT emit `MediaIngress*` payloads,
- runtime MUST reject attempted media signaling with deterministic structured
  errors and leave v1 zone behavior unchanged.
4. Unknown-field tolerance MUST preserve interop:
- old peers ignore new post-v1 fields/messages,
- new peers interoperate with old peers by feature-negotiated suppression.
5. Existing v1 clients using non-media `ZonePublish` remain wire-compatible.

## Implementation Guardrails (For WM-I1)

1. Do not merge schema and conversion changes separately from reconnect
   semantics; partial parity recreates silent metadata loss.
2. Update conversion paths in both directions for every newly-added field.
3. Add integration tests that prove:
- media publication metadata survives snapshot roundtrip,
- resume ordering remains `SessionResumeResult` then `SceneSnapshot`,
- downgrade suppression path emits deterministic denial/error behavior.

## Acceptance Traceability (`hud-nn9d.8`)

1. Required protocol/schema deltas are specified:
- fulfilled by explicit field allocations and message/schema delta tables above.
2. Snapshot and reconnect semantics are explicit:
- fulfilled by snapshot inclusion/exclusion and resume-epoch rules above.
3. Backward-compatibility constraints are documented:
- fulfilled by additive-only, gating, and downgrade rules above.
