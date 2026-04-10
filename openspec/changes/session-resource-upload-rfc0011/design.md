## Context

The repository doctrine is explicit: resident hot-path behavior rides one primary bidirectional gRPC session stream. The resource-store main spec already says scene-node image/font resources SHALL ingress on that stream via `ResourceUploadStart`/`Chunk`/`Complete`, while widget SVG assets use `WidgetAssetRegister` on the same stream. But the checked-in `session.proto`, the main session-protocol spec, and the server implementation only expose the widget path.

There is also a contract bug inside RFC 0011 itself. The RFC says the runtime allocates `upload_id` when a `ResourceUploadStart` is accepted, and chunk messages require that `upload_id`, but the message set shown in the RFC does not define a server acknowledgement that returns it before chunks are sent. Without repairing that handshake, any implementation would be forced to guess.

RFC 0005 is part of the seam as well. Its older combined-envelope field registry diverges from the split `ClientMessage`/`ServerMessage` model used by the checked-in main session-protocol spec and `session.proto`. This change therefore has to make an explicit authority decision before it can assign upload fields safely.

This change is therefore spec-first and cross-cutting: it touches the resident wire contract, the resource-store contract, runtime/protocol ownership boundaries, error-shape consistency, and exemplar/operator validation surfaces.

Existing main-spec requirements for concurrent-upload limits, rate limiting, and stream backpressure remain in scope for this seam repair. This change does not defer them; it carries them forward while keeping `ResourceQuery`/`ResourceQueryResult` out of scope unless the main v1 specs are expanded.

## Goals / Non-Goals

**Goals:**
- Restore coherence between doctrine, RFC/main specs, protobuf, and implementation for resident scene-resource upload.
- Keep scene-resource upload and widget SVG asset registration as distinct resident flows.
- Define an implementable chunked-upload handshake with explicit correlation semantics for start, success, and failure.
- Choose low-churn envelope field allocations that fit the checked-in schema.
- Reconcile scene-resource budget charging to reference time rather than upload-storage time.

**Non-Goals:**
- Introduce a separate upload RPC or service.
- Redesign widget asset registration.
- Pull `ResourceQuery`/`ResourceQueryResult` into scope unless the main v1 specs are expanded first.
- Expand scene-resource persistence semantics beyond existing v1 scope.
- Build a generalized upload framework for every future asset type before the core resident path works.
- Prove agent-uploaded font consumers in the first live-proof tranche; initial consumer conversion is image-led.

## Decisions

### 1. Add a dedicated resident scene-resource upload message family on `HudSession`

Decision: resident scene-resource upload remains on the existing session stream and does not reuse `WidgetAssetRegister`.

Rationale:
- This preserves the doctrine that resident hot-path traffic stays on one stream.
- Widget SVG assets and scene-node resources have different storage classes, validation rules, and consumers.
- Reusing `WidgetAssetRegister` would blur a boundary that the resource-store spec already treats as distinct.

Alternative considered:
- Reuse `WidgetAssetRegister` for all uploads.
Why rejected:
- It conflates widget-specific metadata and durable store semantics with scene-resource upload.

### 2. Repair the chunked-upload handshake by adding an explicit start acknowledgement

Decision: the repaired contract includes a server acknowledgement message that returns `upload_id` on successful `ResourceUploadStart` acceptance before chunks are sent.

Rationale:
- RFC 0011 currently says `upload_id` is assigned on start acceptance, but without a start-ack message chunked uploads are not implementable.
- Explicit acknowledgement removes reliance on response ordering and enables multiple concurrent uploads per session.

Alternative considered:
- Infer `upload_id` from a later success/failure message or forbid chunked uploads.
Why rejected:
- Later responses are too late for chunk transmission, and forbidding chunked uploads would contradict existing v1 requirements.

### 3. Use explicit resident upload success and upload-specific error messages

Decision: scene-resource uploads use dedicated server messages for acceptance, success, and upload-specific failure rather than overloading `RuntimeError` for normal upload failures.

Rationale:
- Upload failures need correlation (`request_sequence`, optional `upload_id`) and resource-specific error codes.
- `RuntimeError` remains appropriate for generic malformed-envelope or session-level failures, but upload-specific errors deserve a contract that matches the resource-store domain.

Alternative considered:
- Use only `RuntimeError`.
Why rejected:
- It weakens correlation and discards the resource-specific error vocabulary already defined in RFC 0011.

### 4. Use low-churn field allocations from currently free slots

Decision: for this seam, the working envelope authority is the split `ClientMessage`/`ServerMessage` model in the checked-in main session-protocol spec and `crates/tze_hud_protocol/proto/session.proto`. On that basis, recommend the following allocations:
- `ClientMessage.resource_upload_start = 36`
- `ClientMessage.resource_upload_chunk = 37`
- `ClientMessage.resource_upload_complete = 38`
- `ServerMessage.resource_upload_accepted = 41`
- `ServerMessage.resource_stored = 42`
- `ServerMessage.resource_error_response = 49`

Rationale:
- In the checked-in main session spec and `session.proto`, client fields 36-38 are free after widget flows, and server fields 41, 42, and 49 are free.
- This avoids perturbing existing widget allocations and keeps the new family grouped in predictable free space.
- RFC 0005's older combined-envelope registry is treated as stale law-and-lore that must be reconciled to this split model before implementation begins.

Alternative considered:
- Repack or renumber existing widget/session messages.
Why rejected:
- Unnecessary churn for a seam that can be closed using free slots.

### 5. Align upload-specific errors with the shared session error model

Decision: retain a dedicated `ResourceErrorResponse`, but extend it so it matches the shared session error model by carrying stable error code, message, context, correction hint, initiating `request_sequence`, and optional `upload_id`.

Rationale:
- Upload failures need domain-specific codes and correlation, but they should not become a bespoke error contract that drifts from the rest of the session surface.
- This preserves doctrine-level consistency across resident gRPC and any future bridges.

Alternative considered:
- Use `RuntimeError` only.
Why rejected:
- It loses a clean resource-domain response surface and weakens upload-specific correlation semantics.

### 6. Charge scene-resource budgets at reference time, not upload-storage time

Decision: resident scene-resource upload validates capability, hash, type, size, decodeability, concurrency, and rate-limit/backpressure at upload time, but decoded texture budget is charged when a scene node references the `ResourceId`, not when bytes are first stored.

Rationale:
- Architecture says scene resources are owned by the nodes that reference them.
- RFC 0011 accounting rules charge decoded size at reference time.
- Upload-time storage without a scene reference should not by itself consume per-agent scene texture budget.

Alternative considered:
- Reject uploads immediately if storing the decoded asset would exceed `texture_bytes_total`.
Why rejected:
- It conflicts with reference-owned accounting and would make uploaded-but-unreferenced resources count against the wrong lifecycle stage.

### 7. Keep font wire support separate from first-tranche consumer proof

Decision: the resident wire contract remains capable of carrying font uploads, but the first consumer tranche for this seam is image-driven and does not attempt to prove agent-uploaded font consumers. Any retained font semantics are limited to scene-node/tile-local text styles and do not override runtime-owned zone/component typography.

Rationale:
- Static image nodes are explicitly in v1 scope and already drive the blocked exemplar/user-test consumers.
- Doctrine places runtime-owned typography and zone/component styling above agent concern.
- Keeping the initial consumer tranche image-led minimizes churn while preserving the broader wire contract.

## Risks / Trade-offs

- [Risk] RFC 0011 itself needs follow-on repair in `about/law-and-lore/` to match the corrected handshake.
  Mitigation: Make contract reconciliation the first bead and do not start implementation before signoff.

- [Risk] Adding multiple new session messages can break generated bindings and helper clients.
  Mitigation: Land protobuf/schema work in its own tranche and add conformance coverage before consumer conversion.

- [Risk] RFC 0005 and RFC 0011 may need law-and-lore updates in addition to main-spec sync.
  Mitigation: Make contract reconciliation the first execution bead and do not present field allocations as final until that closes.

- [Risk] Consumer conversion work could sprawl across many exemplar/helper surfaces.
  Mitigation: Limit the first tranche to known resident consumers that already claim upload behavior.

- [Risk] Runtime/upload ownership could drift between protocol, resource-store, and runtime crates.
  Mitigation: Keep runtime ownership explicit in the delta specs and enforce it via the reconciliation bead.

## Migration Plan

1. Reconcile the resident upload handshake and field allocations across RFC/main specs.
2. Extend `session.proto` and regenerate bindings.
3. Implement server/runtime handling for acceptance, inline/chunked upload, dedup, and upload-specific rejection.
4. Add conformance and integration coverage for the repaired contract.
5. Convert resident exemplar/helper surfaces that currently depend on placeholder upload behavior.
6. Run a terminal spec-to-code reconciliation and publish a human-readable report.

## Open Questions

- Should malformed upload payloads use `ResourceErrorResponse` exclusively, or fall back to `RuntimeError` when the message cannot be parsed at all? The direction recommendation is resource-specific errors for semantically valid upload requests, `RuntimeError` for envelope/protocol malformation.
- Should a follow-on change pull `ResourceQuery`/`ResourceQueryResult` into the main v1 specs, or keep them explicitly deferred? This seam repair keeps only `ResourceQuery`/`ResourceQueryResult` deferred; upload rate limiting and backpressure remain in scope in the main delta.
