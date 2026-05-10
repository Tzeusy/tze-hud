# resident-scene-resource-upload Specification

## Purpose
Defines resident scene-resource upload over the existing HudSession stream, including resource start/chunk/complete correlation, deduplication, error surfaces, and separation from widget asset registration.

## Requirements
### Requirement: Resident Scene-Resource Upload via Session Stream
Resident agents SHALL upload scene-node image and font resources on the existing `HudSession` bidirectional stream. This flow SHALL remain distinct from widget SVG asset registration. `ClientMessage` SHALL carry `ResourceUploadStart`, `ResourceUploadChunk`, and `ResourceUploadComplete` as dedicated payload variants, and `ServerMessage` SHALL carry dedicated resident upload acknowledgement, success, and upload-specific error payloads. There SHALL be no separate upload RPC or side-channel service for this v1 path.
Source: RFC 0011 §3.1, §3.2, §3.3; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: Inline scene-resource upload on the primary stream
- **WHEN** a resident agent needs to upload a small PNG or font resource
- **THEN** it MUST send `ResourceUploadStart` on its existing `HudSession` stream and MUST NOT open a separate upload RPC

#### Scenario: Widget asset registration remains separate
- **WHEN** a resident agent registers a widget SVG asset
- **THEN** it MUST continue using `WidgetAssetRegister` rather than the scene-resource upload message family

### Requirement: Upload Start Acknowledgement and Correlation
When the runtime accepts a non-deduplicated `ResourceUploadStart` that requires chunk transfer, it SHALL return a dedicated acknowledgement carrying the originating `request_sequence` and an `upload_id` before any chunks are sent. All resident upload success and upload-specific error responses SHALL carry enough correlation data to map back to the initiating start request, using `request_sequence` and, when applicable, `upload_id`.
Source: RFC 0011 §3.2, §3.6; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: Chunked upload receives start acknowledgement
- **WHEN** a resident agent sends `ResourceUploadStart` for a large unknown resource
- **THEN** the runtime MUST respond with a start acknowledgement containing `request_sequence` and `upload_id` before the agent sends `ResourceUploadChunk`

#### Scenario: Dedup hit skips upload acknowledgement
- **WHEN** a resident agent sends `ResourceUploadStart` for a hash already present in the scene-resource store
- **THEN** the runtime MUST return `ResourceStored` immediately and MUST NOT require a start acknowledgement or chunk transfer

### Requirement: Resident Upload Success and Error Surfaces
Successful resident scene-resource uploads SHALL return `ResourceStored` on the session stream. Upload-specific resident failures SHALL return `ResourceErrorResponse` on the session stream with a stable resource error code, message, context, correction hint, and correlation fields. `RuntimeError` MAY still be used for malformed envelopes or generic protocol violations, but SHALL NOT replace the resource-specific response surface for semantically valid upload requests.
Source: RFC 0011 §3.5, §3.6, §10; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: Successful chunked upload returns ResourceStored
- **WHEN** a resident agent completes a valid chunked upload
- **THEN** the runtime MUST return `ResourceStored` carrying the confirmed `ResourceId` and the initiating `request_sequence`

#### Scenario: Upload-specific failure returns ResourceErrorResponse
- **WHEN** a resident agent exceeds the concurrent-upload limit or sends bytes that fail hash validation
- **THEN** the runtime MUST return `ResourceErrorResponse` with the appropriate stable resource error code and the relevant `request_sequence` and `upload_id` when applicable

### Requirement: Uploaded Font Boundary
If resident font uploads remain enabled in v1, they SHALL be limited to scene-node/tile-local text styling and SHALL NOT override runtime-owned zone or component-profile typography. This seam's first consumer tranche SHALL NOT require proving agent-uploaded font consumers.
Source: architecture.md §Text rendering; resource-store main spec §Font Asset Management; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: Uploaded font does not alter zone typography
- **WHEN** an agent uploads a custom font resource
- **THEN** subtitle, notification, and other runtime-owned zone/component typography MUST remain controlled by runtime rendering policy rather than that uploaded font

#### Scenario: First live-proof tranche remains image-led
- **WHEN** the first resident exemplar/user-test consumers are converted to the repaired upload contract
- **THEN** successful image-based consumer proof MUST be sufficient even if no agent-uploaded font consumer is exercised yet
