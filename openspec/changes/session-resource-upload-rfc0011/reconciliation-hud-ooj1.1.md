# hud-ooj1.1 Contract Reconciliation Summary

Date: 2026-04-16  
Issue: `hud-ooj1.1`  
Scope: Reconcile resident scene-resource upload contract across RFC 0011, RFC 0005, and main change-local session/resource specs.

## Doctrine-Aligned (Observed)

1. [Observed] Resident scene-resource ingress stays on the existing `HudSession` stream; no separate upload RPC/service is introduced.
2. [Observed] Scene-resource upload remains distinct from widget SVG asset registration (`WidgetAssetRegister` stays widget-specific).
3. [Observed] Capability authority is explicit: scene-resource upload uses `upload_resource`; widget registration uses `register_widget_asset`.
4. [Observed] Upload transport keeps transactional semantics and remains subject to stream backpressure and rate-limiting.
5. [Observed] Scene-resource decoded texture budget is enforced at node-reference time rather than upload-storage time.

These are reflected in:
- `openspec/changes/session-resource-upload-rfc0011/specs/session-protocol/spec.md`
- `openspec/changes/session-resource-upload-rfc0011/specs/resource-store/spec.md`

## Inferred Decisions (Explicitly Chosen For This Seam)

1. [Inferred] Split envelopes are authoritative for this repair: `ClientMessage`/`ServerMessage` (not RFC 0005's older combined-envelope registry).
2. [Inferred] Chunked uploads require an explicit start acknowledgement: `ResourceUploadAccepted(request_sequence, upload_id)` before chunk transfer begins.
3. [Inferred] Upload-specific failures use `ResourceErrorResponse`, but with shared session error-shape fields (`error_code`, `message`, `context`, `hint`) plus correlation (`request_sequence`, optional `upload_id`).
4. [Inferred] Envelope allocations for this seam use the currently free slots documented in the session-protocol delta (client 36-38; server 41, 42, 49).

These decisions are deliberate contract choices pending downstream schema/runtime implementation beads.

## Open / Follow-On Seams

1. [Open] Law-and-lore sync remains follow-on: RFC 0005 and RFC 0011 prose must be updated to match this reconciled split-envelope handshake.
2. [Open] Schema/runtime implementation is follow-on (`hud-ooj1.2+`): `session.proto`, generated bindings, and server handling are not changed in this bead.
3. [Open] `ResourceQuery`/`ResourceQueryResult` remain out of scope for this seam unless main v1 specs are widened.
4. [Open] First live consumer proof remains image-led; uploaded-font consumer proof is deferred while the font boundary is still specified.

## Signoff Readiness

The `session-protocol` and `resource-store` delta specs in this change are internally aligned on:
- resident upload handshake and correlation semantics,
- upload-specific success/failure response surfaces,
- capability boundary,
- budget enforcement point,
- traffic-class/backpressure expectations,
- separation from widget asset registration.

This makes the delta specs ready for signoff and subsequent sync/implementation work.
