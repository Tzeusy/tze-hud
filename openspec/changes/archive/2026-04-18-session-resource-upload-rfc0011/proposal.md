## Why

Resident agents are supposed to upload scene-node images and fonts over the existing `HudSession` stream, but the checked-in session schema and server currently expose only widget asset registration on that stream. This leaves v1 doctrine, the resource-store spec, exemplar specs, and implementation out of alignment.

RFC 0011 also has a contract hole for chunked uploads: it says the runtime assigns `upload_id` on `ResourceUploadStart` acceptance, but it does not define a server acknowledgement message that returns that `upload_id` before chunks are sent. The repo needs a spec-first repair before code can land honestly.

## What Changes

- Modify the `session-protocol` capability spec to add a resident scene-resource upload message family on `HudSession`, with explicit envelope field allocations and response/correlation semantics.
- Modify the `resource-store` capability spec to align its resident upload requirements with the repaired handshake and response semantics, including success and upload-specific failure responses that match the session error model.
- Keep `WidgetAssetRegister` as the widget SVG path; do not collapse scene-resource upload into the widget asset flow.
- Explicitly defer `ResourceQuery`/`ResourceQueryResult` from this seam fix unless the main v1 specs are widened.
- Define the spec-first work needed before implementation: RFC/main-spec reconciliation, protobuf/schema changes, runtime wiring, conformance coverage, and resident consumer conversion.

## Capabilities

### New Capabilities

- `resident-scene-resource-upload`: Temporary change-local synthesis slice for resident session-stream upload of scene-node image/font resources, including upload-start acknowledgement, chunked flow, success confirmation, and upload-specific failure correlation. Its requirements sync into `session-protocol` and `resource-store`; it is not a new long-lived root capability after sync.

### Modified Capabilities

- `session-protocol`: Extend the v1 session envelope and resident message inventory to carry scene-resource upload messages separately from widget asset registration.
- `resource-store`: Reconcile the resident upload contract with the repaired session-stream handshake and response semantics.

## Impact

- Affected code: `crates/tze_hud_protocol/proto/session.proto`, generated bindings, `crates/tze_hud_protocol/src/session_server.rs`, resource-store/runtime upload handling, resident helpers and exemplar/user-test consumers.
- Affected specs: `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`, `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md`.
- Affected design contracts: RFC 0011 and the main v1 session/resource-store contract chain.
