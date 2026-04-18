## MODIFIED Requirements

### Requirement: Upload Validation
Before storing, the runtime SHALL validate: (1) capability (`upload_resource` for scene-node image/font resources, `register_widget_asset` for runtime widget SVG assets), (2) BLAKE3 hash integrity matches expected hash, (3) total bytes are within per-resource limits, (4) resource_type is v1-supported for the selected path, and (5) content decodes/parses successfully (images decode, fonts parse, SVG parses as valid SVG). For scene-node image/font resources, decoded texture budget SHALL NOT be rejected at upload-storage time; it SHALL be enforced when a mutation creates or updates a node that references the returned `ResourceId`. For widget registrations, dedicated runtime widget-asset durable budgets SHALL still be enforced at registration time. Upload rate limiting and concurrent-upload limits SHALL apply to the upload transport itself. For widget registrations that provide `transport_crc32c`, the runtime SHALL validate CRC32C as a transport-integrity check and reject mismatches.
Source: RFC 0011 §3.5, §8.4, §9.1, §11.2; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: Capability check failure
- **WHEN** a guest agent (without upload_resource capability) attempts to upload a resource
- **THEN** the upload MUST be rejected with RESOURCE_CAPABILITY_DENIED

#### Scenario: Decode failure
- **WHEN** an agent uploads bytes that claim to be IMAGE_PNG but contain corrupted data that cannot decode
- **THEN** the upload MUST be rejected with RESOURCE_DECODE_ERROR

#### Scenario: Scene-resource upload is not rejected on texture budget alone
- **WHEN** an uploaded image decodes successfully but is not yet referenced by any scene node
- **THEN** the upload MUST NOT be rejected solely because it would exceed the agent's `texture_bytes_total` budget if later referenced

#### Scenario: Widget registration still enforces durable store budget
- **WHEN** a runtime widget SVG registration would exceed the dedicated widget-asset durable budget
- **THEN** the request MUST be rejected at registration time

### Requirement: Upload Protocol via Session Stream
Resource ingress SHALL use the session stream defined in RFC 0005. Scene-node image/font resources SHALL use the resident scene-resource upload flow on that stream: `ResourceUploadStart`, optional `ResourceUploadAccepted`, zero or more `ResourceUploadChunk` messages, `ResourceUploadComplete` when chunked, `ResourceStored` on success, and `ResourceErrorResponse` on upload-specific failure. Runtime widget SVG assets SHALL use `WidgetAssetRegister` metadata-first registration on the same stream. There SHALL be no separate upload RPC or service for either path.
Source: RFC 0011 §3.1, RFC 0005 §3.10; session-resource-upload-rfc0011 design
Scope: v1-mandatory

#### Scenario: Scene-resource upload uses resident upload message family
- **WHEN** an agent needs to upload an image or font for scene-node use
- **THEN** it MUST use the resident scene-resource upload message family on its existing session stream rather than `WidgetAssetRegister` or a separate RPC

#### Scenario: Widget asset registration remains widget-specific
- **WHEN** an agent needs to register a runtime widget SVG asset
- **THEN** it MUST use `WidgetAssetRegister` on the existing session stream

## ADDED Requirements

### Requirement: Upload Start Acknowledgement
When the runtime accepts a non-deduplicated scene-resource `ResourceUploadStart` that requires chunk transfer, it SHALL allocate an opaque `upload_id` and return `ResourceUploadAccepted` before the client sends any chunks. `ResourceUploadAccepted` SHALL include the initiating `request_sequence` and the assigned `upload_id`. Inline or deduplicated uploads MAY bypass this acknowledgement and return `ResourceStored` immediately.
Source: RFC 0011 §3.2, §3.6; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: Large unknown upload receives acknowledgement
- **WHEN** an agent starts a large unknown resource upload without `inline_data`
- **THEN** the runtime MUST return `ResourceUploadAccepted` carrying `request_sequence` and `upload_id`

#### Scenario: Deduplicated upload bypasses acknowledgement
- **WHEN** an agent starts an upload whose declared hash already exists in the relevant scene-resource store
- **THEN** the runtime MUST return `ResourceStored` immediately and MUST NOT require `ResourceUploadAccepted`

### Requirement: Upload Response Correlation
Resident scene-resource upload responses SHALL be correlatable without relying on arrival order. `ResourceStored` SHALL include the initiating `request_sequence` and MAY include `upload_id` when a chunked upload was previously accepted. `ResourceErrorResponse` SHALL include the initiating `request_sequence`, stable resource error code, human-readable message, structured context, structured hint, and optional `upload_id` when the failure applies to an accepted upload.
Source: RFC 0011 §3.5, §3.6, §10; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: Success response identifies originating start
- **WHEN** the runtime stores a chunked resource after `ResourceUploadComplete`
- **THEN** the returned `ResourceStored` MUST identify the originating start request via `request_sequence`

#### Scenario: Error response identifies accepted upload
- **WHEN** the runtime rejects an upload after it has already issued `ResourceUploadAccepted`
- **THEN** `ResourceErrorResponse` MUST carry the relevant `upload_id`
