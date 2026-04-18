## MODIFIED Requirements

### Requirement: ClientMessage and ServerMessage Envelopes
Every client-to-server message on the session stream SHALL be wrapped in a ClientMessage envelope, and every server-to-client message SHALL be wrapped in a ServerMessage envelope. Both envelopes SHALL contain: sequence (per-direction monotonically increasing, starting at 1), timestamp_wall_us (sender wall-clock, advisory only), and a oneof payload. ClientMessage oneof payload fields SHALL be allocated as follows: session lifecycle at 10-12 (SessionInit=10, SessionResume=11, SessionClose=12), agent operations at 20-38 (MutationBatch=20, LeaseRequest=21, LeaseRenew=22, LeaseRelease=23, SubscriptionChange=24, ZonePublish=25, TelemetryFrame=26, InputFocusRequest=27, InputCaptureRequest=28, InputCaptureRelease=29, SetImePosition=30, Heartbeat=31, CapabilityRequest=32, EmitSceneEvent=33, WidgetAssetRegister=34, WidgetPublish=35, ResourceUploadStart=36, ResourceUploadChunk=37, ResourceUploadComplete=38). ServerMessage oneof payload fields SHALL be allocated as follows: session lifecycle at 10-15 (SessionEstablished=10, SessionError=11, SessionResumeResult=12, SessionSuspended=13, SessionResumed=14, RuntimeError=15), mutation/lease responses at 20-25 (MutationResult=20, LeaseResponse=21, LeaseStateChange=23, CapabilityNotice=25), scene state at 30-36 (SceneSnapshot=30, SceneDelta=31, Heartbeat=33, EventBatch=34, BackpressureSignal=35, RuntimeTelemetryFrame=36), operational responses at 39-49 (SubscriptionChangeResult=39, ZonePublishResult=40, ResourceUploadAccepted=41, ResourceStored=42, InputFocusResponse=43, InputCaptureResponse=44, EmitSceneEventResult=45, DegradationNotice=46, WidgetPublishResult=47, WidgetAssetRegisterResult=48, ResourceErrorResponse=49). Fields 50-99 in both envelopes SHALL be reserved for post-v1 use.
Source: session-resource-upload-rfc0011 direction/design, reconciling the current split-envelope main spec with RFC 0005 §2.2/§9.2 and RFC 0011 §3.1/§3.4
Scope: v1-mandatory

#### Scenario: Resident upload payloads fit the envelope
- **WHEN** a resident agent starts, chunks, or completes a scene-resource upload
- **THEN** each message SHALL be wrapped in `ClientMessage` using fields 36, 37, and 38 respectively

#### Scenario: Resident upload responses fit the envelope
- **WHEN** the runtime accepts, stores, or rejects a resident scene-resource upload
- **THEN** it SHALL return `ResourceUploadAccepted`, `ResourceStored`, or `ResourceErrorResponse` on `ServerMessage` fields 41, 42, or 49 respectively

### Requirement: Widget Asset Registration via Session Stream
`WidgetAssetRegister` (ClientMessage field 34) SHALL provide a metadata-first register/upload flow for runtime widget SVG assets only. The request SHALL carry `widget_type_id`, `svg_filename`, `content_hash_blake3` (32-byte canonical identity), optional `transport_crc32c` (transport integrity hint only), declared `total_size_bytes`, optional inline payload bytes, and optional `metadata_only_preflight`. `WidgetAssetRegisterResult` (ServerMessage field 48) SHALL carry `accepted`, `widget_type_id`, `svg_filename`, `asset_handle`, `was_deduplicated`, and error details on failure. Scene-node image and font resources SHALL NOT use this widget-specific message pair; they SHALL use the resident scene-resource upload flow.
Source: RFC 0005 §3.10, RFC 0011 §2.2a, §9.1; session-resource-upload-rfc0011 design
Scope: v1-mandatory

#### Scenario: Widget asset path remains widget-specific
- **WHEN** an agent needs to register a runtime widget SVG asset
- **THEN** it SHALL use `WidgetAssetRegister` and receive `WidgetAssetRegisterResult`

#### Scenario: Scene resource upload does not reuse WidgetAssetRegister
- **WHEN** an agent needs to upload a PNG or font for later `StaticImageNode` or font use
- **THEN** it SHALL use `ResourceUploadStart` rather than `WidgetAssetRegister`

## ADDED Requirements

### Requirement: Resident Scene-Resource Upload Handshake
The resident session protocol SHALL expose a dedicated scene-resource upload handshake. `ResourceUploadStart` SHALL initiate the request with declared hash, type, size, and metadata. If the request is accepted and additional payload transfer is required, the runtime SHALL respond with `ResourceUploadAccepted` carrying the initiating `request_sequence` and `upload_id`. The client SHALL send `ResourceUploadChunk` messages using that `upload_id`, followed by `ResourceUploadComplete`. If the start request is deduplicated or fully satisfied inline, the runtime MAY skip `ResourceUploadAccepted` and return `ResourceStored` immediately.
Source: RFC 0011 §3.1, §3.2, §3.3, §3.6; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: Accepted chunked upload returns upload_id
- **WHEN** the runtime accepts a large unknown resident upload
- **THEN** it MUST send `ResourceUploadAccepted(request_sequence=<start-sequence>, upload_id=<opaque id>)` before any chunks are expected

#### Scenario: Inline upload skips chunk phase
- **WHEN** a resident agent provides `inline_data` in `ResourceUploadStart` for a resource within the fast-path limit
- **THEN** the runtime MUST NOT require `ResourceUploadChunk` or `ResourceUploadComplete`

### Requirement: Resident Scene-Resource Upload Responses
`ResourceStored` SHALL be the resident upload success response for scene-resource uploads. `ResourceErrorResponse` SHALL be the resident upload-specific failure response for semantically valid upload requests and SHALL include `request_sequence`, stable `error_code`, human-readable `message`, structured `context`, structured `hint`, and optional `upload_id`. `RuntimeError` SHALL remain reserved for malformed session envelopes or generic protocol violations outside the upload-specific response surface.
Source: RFC 0011 §3.5, §3.6, §10; session-resource-upload-rfc0011 direction/design
Scope: v1-mandatory

#### Scenario: ResourceStored correlates to initiating request
- **WHEN** the runtime successfully stores or deduplicates a resident upload
- **THEN** `ResourceStored` MUST include the initiating `request_sequence`

#### Scenario: ResourceErrorResponse carries upload correlation
- **WHEN** the runtime rejects a chunked resident upload after start acceptance
- **THEN** `ResourceErrorResponse` MUST include both the initiating `request_sequence` and the relevant `upload_id`

### Requirement: Resident Upload Traffic Classes and Backpressure
Resident upload control messages (`ResourceUploadStart`, `ResourceUploadComplete`, `ResourceUploadAccepted`, `ResourceStored`, and `ResourceErrorResponse`) SHALL use the transactional traffic class. `ResourceUploadChunk` SHALL also be transactional: upload bytes MUST remain ordered, reliable, and never silently dropped. Upload throughput shaping SHALL rely on the existing session backpressure model and per-session upload rate limiting; the runtime MAY delay reading or acknowledging chunk progress under backpressure, but it SHALL NOT downgrade upload chunks to a droppable class.
Source: session-resource-upload-rfc0011 direction/design, reconciling main session traffic-class rules with RFC 0011 §8.4
Scope: v1-mandatory

#### Scenario: Upload chunks are not droppable
- **WHEN** a resident client sends chunked upload data under backpressure
- **THEN** the runtime MUST preserve ordered, reliable delivery for `ResourceUploadChunk` rather than treating chunk bytes as a droppable realtime class

#### Scenario: Upload transport backpressure shapes throughput
- **WHEN** an agent exceeds the configured per-session upload rate
- **THEN** the runtime MAY back-pressure the session transport and delay chunk intake, but it MUST NOT reclassify upload control or chunk messages to a droppable traffic class
