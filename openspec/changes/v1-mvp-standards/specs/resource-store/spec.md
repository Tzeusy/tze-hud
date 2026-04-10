# Resource Store Specification

## ADDED Requirements

### Requirement: Content-Addressed Resource Identity
All resources SHALL be identified by their BLAKE3 hash of raw input bytes (before decode or transcoding), stored as 32 bytes. Two uploads of identical bytes MUST produce the same ResourceId. The ResourceId SHALL be a pure function of content bytes, independent of the uploading agent's namespace.
Source: RFC 0011 §1.1, §1.2, §1.4
Scope: v1-mandatory

#### Scenario: Identical bytes produce identical ResourceId
- **WHEN** Agent A uploads a 500KB PNG and Agent B uploads the same 500KB PNG
- **THEN** both MUST receive the same ResourceId (BLAKE3 hash of the raw bytes)

#### Scenario: ResourceId is 32 bytes
- **WHEN** a ResourceId is computed for any resource
- **THEN** it MUST be exactly 32 bytes (256-bit BLAKE3 digest)

---

### Requirement: Resource Immutability
Once stored, content at a given ResourceId SHALL never change. The resource store SHALL never mutate the content of an existing resource. If an upload produces a ResourceId that already exists, it SHALL be treated as a dedup hit and the existing content SHALL be returned without modification.
Source: RFC 0011 §1.3
Scope: v1-mandatory

#### Scenario: Immutable content guarantee
- **WHEN** a resource with ResourceId X is stored and a subsequent upload produces ResourceId X
- **THEN** the store MUST return the original content unchanged and report was_deduplicated = true

---

### Requirement: V1 Resource Type Enumeration
V1 SHALL support six resource types: IMAGE_RGBA8 (raw RGBA8 pixel data), IMAGE_PNG (PNG-encoded), IMAGE_JPEG (JPEG-encoded), FONT_TTF (TrueType font), FONT_OTF (OpenType font), and IMAGE_SVG (widget asset graphics only). Uploads of unsupported types SHALL be rejected with RESOURCE_UNSUPPORTED_TYPE error.
Source: RFC 0011 §2.1, §2.2, §2.2a
Scope: v1-mandatory

#### Scenario: Supported type accepted
- **WHEN** an agent uploads a resource with type IMAGE_PNG
- **THEN** the upload MUST be accepted (assuming all other validation passes)

#### Scenario: SVG type accepted for widget asset path
- **WHEN** an agent registers/uploads a widget asset with type IMAGE_SVG through the widget registration flow
- **THEN** the runtime MUST accept IMAGE_SVG type validation for widget assets (subject to capability, hash, and SVG-parse checks)

#### Scenario: Unsupported type rejected
- **WHEN** an agent uploads a resource with type VIDEO_H264
- **THEN** the upload MUST be rejected with RESOURCE_UNSUPPORTED_TYPE error

---

### Requirement: Upload Protocol via Session Stream
Resource ingress SHALL use the session stream defined in RFC 0005. Scene-node image/font resources SHALL use the ResourceUploadStart/Chunk/Complete flow. Runtime widget SVG assets SHALL use WidgetAssetRegister metadata-first registration on the same stream. There SHALL be no separate upload RPC or service for either path.
Source: RFC 0011 §3.1, RFC 0005 §3.10
Scope: v1-mandatory

#### Scenario: Upload on existing stream
- **WHEN** an agent needs to upload a resource
- **THEN** it MUST send ResourceUploadStart on its existing session stream, not open a separate connection

#### Scenario: Widget asset register on existing stream
- **WHEN** an agent needs to register a runtime widget SVG asset
- **THEN** it MUST send WidgetAssetRegister on its existing session stream, not open a separate upload connection

---

### Requirement: Chunked Upload Flow
For resources larger than 64 KiB, the upload SHALL follow a three-phase flow: ResourceUploadStart (with expected hash, type, size, metadata), sequential ResourceUploadChunk messages (max 64 KiB per chunk, 0-based sequential index), and ResourceUploadComplete. The runtime SHALL validate the BLAKE3 hash of received bytes against expected_hash after completion.
Source: RFC 0011 §3.2, §3.4
Scope: v1-mandatory

#### Scenario: Chunked upload success
- **WHEN** an agent uploads a 200 KiB image via ResourceUploadStart followed by 4 chunks and ResourceUploadComplete
- **THEN** the runtime MUST compute BLAKE3 of the received bytes, verify it matches expected_hash, decode the content, and return ResourceStored with confirmed ResourceId

#### Scenario: Hash mismatch rejection
- **WHEN** the computed BLAKE3 hash of received bytes does not match the expected_hash
- **THEN** the runtime MUST reject the upload with RESOURCE_HASH_MISMATCH error

---

### Requirement: Small Resource Fast Path
Resources <= 64 KiB MAY be uploaded in a single ResourceUploadStart message by including the content bytes in the inline_data field. The chunked upload sequence (ResourceUploadChunk/ResourceUploadComplete) SHALL NOT be required for inline uploads.
Source: RFC 0011 §3.3
Scope: v1-mandatory

#### Scenario: Inline upload for small resource
- **WHEN** an agent uploads a 32 KiB icon with inline_data populated in ResourceUploadStart
- **THEN** the runtime MUST accept the upload without requiring chunk or complete messages

---

### Requirement: Upload Validation
Before storing, the runtime SHALL validate: (1) capability (`upload_resource` for scene-node image/font resources, `register_widget_asset` for runtime widget SVG assets), (2) BLAKE3 hash integrity matches expected hash, (3) total bytes are within per-resource limits, (4) budget limits are not exceeded (including dedicated runtime widget-asset durable budgets), (5) resource_type is v1-supported for the selected path, and (6) content decodes/parses successfully (images decode, fonts parse, SVG parses as valid SVG). For widget registrations that provide `transport_crc32c`, the runtime SHALL validate CRC32C as a transport-integrity check and reject mismatches.
Source: RFC 0011 §3.5, §2.2a, §9.1
Scope: v1-mandatory

#### Scenario: Capability check failure
- **WHEN** a guest agent (without upload_resource capability) attempts to upload a resource
- **THEN** the upload MUST be rejected with RESOURCE_CAPABILITY_DENIED

#### Scenario: Widget registration capability check failure
- **WHEN** an agent without register_widget_asset capability attempts runtime widget SVG registration
- **THEN** the request MUST be rejected with WIDGET_ASSET_CAPABILITY_MISSING

#### Scenario: Decode failure
- **WHEN** an agent uploads bytes that claim to be IMAGE_PNG but contain corrupted data that cannot decode
- **THEN** the upload MUST be rejected with RESOURCE_DECODE_ERROR

#### Scenario: Budget exceeded
- **WHEN** an agent whose texture_bytes_total consumption is at the lease limit attempts to upload another texture
- **THEN** the upload MUST be rejected with RESOURCE_BUDGET_EXCEEDED

#### Scenario: Widget transport checksum mismatch
- **WHEN** WidgetAssetRegister provides transport_crc32c and payload CRC does not match
- **THEN** the runtime MUST reject the request with WIDGET_ASSET_CHECKSUM_MISMATCH

---

### Requirement: Content-Addressed Deduplication
When the runtime receives a ResourceUploadStart or WidgetAssetRegister metadata preflight, it SHALL check whether the declared BLAKE3 hash already exists in the relevant store/index. If found, it SHALL return success immediately with was_deduplicated = true and skip payload transfer. No additional storage SHALL be consumed. Dedup check latency SHALL be less than 100 microseconds.
Source: RFC 0011 §3.6, §9.1
Scope: v1-mandatory

#### Scenario: Dedup hit skips upload
- **WHEN** an agent starts an upload with an expected_hash that matches an already-stored resource
- **THEN** the runtime MUST return ResourceStored immediately with was_deduplicated = true; no chunks are needed

#### Scenario: Widget metadata-only dedup preflight
- **WHEN** an agent sends WidgetAssetRegister(metadata_only_preflight=true) for an already-indexed hash
- **THEN** the runtime MUST return WidgetAssetRegisterResult with was_deduplicated=true and MUST NOT request payload bytes

#### Scenario: Dedup check performance
- **WHEN** the runtime performs a dedup lookup against the resource store index
- **THEN** the lookup MUST complete within 100 microseconds

---

### Requirement: Concurrent Upload Limits
An agent SHALL have at most 4 concurrent uploads in flight. A 5th ResourceUploadStart before a previous upload completes SHALL be rejected with RESOURCE_TOO_MANY_UPLOADS. This prevents a single agent from monopolizing upload bandwidth.
Source: RFC 0011 §3.7
Scope: v1-mandatory

#### Scenario: Fifth concurrent upload rejected
- **WHEN** an agent has 4 uploads in flight and sends a 5th ResourceUploadStart
- **THEN** the runtime MUST reject it with RESOURCE_TOO_MANY_UPLOADS

---

### Requirement: Scene-Graph-Level Reference Counting
The resource store SHALL maintain an integer refcount for each stored resource. Refcount changes SHALL be atomic and occur on the compositor thread during mutation commit. Refcount SHALL track how many scene graph nodes reference the resource, regardless of which agent owns those nodes. Newly uploaded resources SHALL start at refcount 0. Refcount SHALL never go below zero (underflow is a bug).
Source: RFC 0011 §4.1, §4.2, §4.4
Scope: v1-mandatory

#### Scenario: Refcount increment on node creation
- **WHEN** an agent creates a StaticImageNode referencing ResourceId X
- **THEN** the refcount for X MUST increment by 1

#### Scenario: Refcount decrement on node deletion
- **WHEN** a tile is deleted, cascading to all its nodes that reference resources
- **THEN** the refcount for each referenced resource MUST decrement by 1 per deleted node

#### Scenario: Cross-agent sharing via refcount
- **WHEN** Agent A and Agent B each create a node referencing the same ResourceId X, then Agent A deletes its node
- **THEN** refcount MUST be 1 (Agent B's reference), and the resource MUST NOT be GC-eligible

#### Scenario: Refcount underflow detection
- **WHEN** a bug would cause refcount to drop below zero
- **THEN** the runtime MUST panic in debug builds and log a structured error in release builds

---

### Requirement: Per-Agent Budget Accounting for Shared Resources
Texture bytes SHALL count against the agent whose node references the resource, not the uploader. If multiple agents reference the same resource, each agent SHALL be charged the full decoded size against their respective budgets (per-agent double-counting). This prevents coordinated multi-agent budget bypass.
Source: RFC 0011 §4.3
Scope: v1-mandatory

#### Scenario: Double-counting for shared resource
- **WHEN** Agent A (budget: 10 MiB) and Agent B (budget: 10 MiB) both reference a 4 MiB decoded texture
- **THEN** Agent A MUST be charged 4 MiB and Agent B MUST be charged 4 MiB against their respective budgets

#### Scenario: Budget measured as decoded size
- **WHEN** an agent references a 500 KiB compressed PNG that decodes to 4 MiB RGBA8
- **THEN** 4 MiB (decoded in-memory size) MUST be charged against the agent's texture budget, not 500 KiB

---

### Requirement: Cross-Agent Resource Sharing
Resources SHALL be global, not per-agent. Any agent SHALL be able to reference any ResourceId if they know the hash (content-addressed identity is the capability). There SHALL be no access control list or ownership gate on reading. There SHALL be no "list all resources" enumeration operation to prevent resource discovery attacks.
Source: RFC 0011 §5.1, §5.2, §5.4
Scope: v1-mandatory

#### Scenario: Cross-namespace read access
- **WHEN** Agent A uploads an image producing ResourceId X, and Agent B knows X
- **THEN** Agent B MUST be able to create a node referencing ResourceId X without additional capability

#### Scenario: No resource enumeration
- **WHEN** an agent attempts to list or enumerate all stored resources
- **THEN** no such operation SHALL exist; agents can only query resources they know the ResourceId of

---

### Requirement: Upload Capability Gate
Uploading a resource SHALL require the `upload_resource` capability (included in the default resident capability set). Guest agents SHALL NOT be able to upload resources directly; they interact through MCP zone tools which handle resource management internally.
Source: RFC 0011 §5.2
Scope: v1-mandatory

#### Scenario: Guest agent upload denied
- **WHEN** a guest agent attempts to upload a resource directly
- **THEN** the upload MUST be rejected with RESOURCE_CAPABILITY_DENIED

---

### Requirement: Garbage Collection Candidacy and Grace Period
Resources with refcount == 0 SHALL enter GC candidacy. Candidate resources SHALL remain in the store and SHALL NOT be immediately deleted. A configurable grace period timer (default: 60 seconds, min: 1 second, max: 3600 seconds) SHALL start when refcount reaches zero. The resource SHALL be eligible for deletion only after the full grace period elapses at refcount == 0.
Source: RFC 0011 §6.1, §6.2
Scope: v1-mandatory

#### Scenario: Grace period prevents premature deletion
- **WHEN** a resource's refcount reaches 0 and 30 seconds pass (less than the 60s default grace period)
- **THEN** the resource MUST still be in the store and available for resurrection

#### Scenario: Eviction after grace period
- **WHEN** a resource's refcount has been 0 for at least 60 seconds (default grace period) and a GC cycle runs
- **THEN** the resource MUST be evicted: decoded data freed, entry removed from store index

---

### Requirement: GC Cycle Timing
The GC SHALL run on a configurable timer (default: 30 seconds, min: 5 seconds, max: 300 seconds). Each GC cycle SHALL: scan all candidates, evict those whose grace period has elapsed, free decoded in-memory data, and remove from store index. Each GC cycle SHALL have a time budget of 5ms; excess work SHALL be deferred to the next cycle.
Source: RFC 0011 §6.3, §6.4
Scope: v1-mandatory

#### Scenario: GC cycle budget enforcement
- **WHEN** a GC cycle has more eviction work than can be completed in 5ms
- **THEN** remaining evictions MUST be deferred to the next GC cycle to prevent frame drops

---

### Requirement: GC Frame Render Isolation
GC SHALL never run during frame render. The GC phase SHALL run as a compositor-thread epilogue after GPU Submit + Present completes, in the inter-frame idle window. GC MUST NOT acquire any lock held by the render pipeline during stages 4-7. GC MUST NOT touch GPU device state during frame render. GC MUST NOT deallocate GPU textures bound to the current frame's draw calls.
Source: RFC 0011 §6.4, §6.6
Scope: v1-mandatory

#### Scenario: GC in inter-frame window
- **WHEN** the compositor completes frame N's GPU submit and present
- **THEN** GC MUST run in the inter-frame epilogue before stage 3 of frame N+1 begins

#### Scenario: No mid-render GPU texture deallocation
- **WHEN** a texture is still bound to the current frame's draw calls
- **THEN** GC MUST NOT deallocate that texture; eviction MUST be deferred to the next GC cycle

---

### Requirement: Resource Resurrection
A GC candidate resource (refcount == 0, grace period not yet elapsed) SHALL be resurrectable. When a scene mutation creates a node referencing a candidate resource, the refcount SHALL increment, the resource SHALL transition back to live, and its decoded in-memory form (if still resident) SHALL NOT need to be reloaded.
Source: RFC 0011 §6.5
Scope: v1-mandatory

#### Scenario: Resurrection before eviction
- **WHEN** a resource has refcount 0 for 20 seconds (within the 60s grace period) and a new node references it
- **THEN** the resource MUST be resurrected: refcount incremented to 1, no re-upload or re-decode needed

---

### Requirement: V1 Persistence Split
V1 persistence SHALL be split by resource class. Scene-node resources (images/fonts referenced by scene graph nodes) SHALL remain ephemeral in memory and SHALL be lost on restart. Runtime widget SVG assets registered through WidgetAssetRegister SHALL be durable and SHALL survive restart through a local content-addressed asset store plus startup re-index.
Source: RFC 0011 §9.1
Scope: v1-mandatory

#### Scenario: Scene-node resources lost on restart
- **WHEN** the runtime restarts
- **THEN** previously uploaded scene-node images/fonts MUST be gone; agents MUST re-upload after reconnection

#### Scenario: Runtime widget SVG assets survive restart
- **WHEN** the runtime restarts after successful runtime widget asset registrations
- **THEN** startup MUST re-index the durable widget asset store and previously registered hashes MUST remain dedup hits

---

### Requirement: Font Asset Management
The runtime SHALL discover fonts from three sources: system fonts (platform font directories), bundled fonts (compiled into the binary), and agent-uploaded fonts. System and bundled fonts SHALL have permanent implicit holds and SHALL never be GC'd. Font family resolution SHALL follow: (1) named variant from display profile, (2) custom ResourceId lookup with fallback to SystemSansSerif, (3) bundled default. Fallback SHALL be transparent to agents.
Source: RFC 0011 §7.1, §7.2, §7.3
Scope: v1-mandatory

#### Scenario: System font never GC'd
- **WHEN** no scene graph nodes reference a system font
- **THEN** the font MUST remain available; system and bundled fonts have permanent implicit holds

#### Scenario: Font fallback on missing custom font
- **WHEN** a TextMarkdownNode references a custom font ResourceId that is not in the store
- **THEN** the runtime MUST fall back to SystemSansSerif (bundled default) without notifying the agent

---

### Requirement: Font Cache
Fonts SHALL be cached in an LRU cache bounded by configurable maximum size (default: 64 MiB). The cache SHALL include loaded font faces, shaped glyph caches, and rasterized glyph atlases. Font glyph caches SHALL be evicted LRU when the font memory budget is exceeded. System and bundled fonts SHALL never be evicted from the cache.
Source: RFC 0011 §7.5
Scope: v1-mandatory

#### Scenario: Font cache LRU eviction
- **WHEN** the font cache exceeds 64 MiB
- **THEN** the least recently used agent-uploaded font data MUST be evicted first; system/bundled fonts MUST NOT be evicted

---

### Requirement: Per-Resource Size Limits
Maximum input size per resource SHALL be 16 MiB (configurable). Maximum decoded texture size SHALL be 64 MiB (configurable). Maximum texture dimension SHALL be 8192 pixels in either width or height (not configurable). Resources exceeding these limits SHALL be rejected at upload time with RESOURCE_SIZE_EXCEEDED.
Source: RFC 0011 §8.1
Scope: v1-mandatory

#### Scenario: Oversized resource rejected
- **WHEN** an agent uploads a resource of 20 MiB (exceeding the default 16 MiB limit)
- **THEN** the upload MUST be rejected with RESOURCE_SIZE_EXCEEDED

#### Scenario: Decompression bomb defense
- **WHEN** a 1 MiB PNG decodes to a texture exceeding 64 MiB (e.g., 16384x16384 RGBA8)
- **THEN** the decode MUST be aborted with RESOURCE_SIZE_EXCEEDED

---

### Requirement: Per-Runtime Total Limits
Maximum total texture memory across all resources SHALL default to 512 MiB (configurable). Maximum total font cache memory SHALL default to 64 MiB (configurable). Maximum concurrent resources SHALL default to 4096 (configurable). When the runtime-wide texture memory limit is reached, new uploads SHALL be rejected with RESOURCE_BUDGET_EXCEEDED even if the individual agent has budget remaining.
Source: RFC 0011 §8.3
Scope: v1-mandatory

#### Scenario: Runtime-wide limit reached
- **WHEN** total texture memory across all agents reaches 512 MiB and a new upload arrives
- **THEN** the upload MUST be rejected with RESOURCE_BUDGET_EXCEEDED regardless of the individual agent's remaining budget

---

### Requirement: Upload Rate Limiting
Each agent SHALL be rate-limited to 1 MiB/s upload throughput (configurable), enforced per-session as a sliding window over the last 1 second. Upload chunks exceeding the rate limit SHALL be back-pressured via gRPC flow control.
Source: RFC 0011 §8.4
Scope: v1-mandatory

#### Scenario: Rate limit back-pressure
- **WHEN** an agent attempts to upload at 2 MiB/s
- **THEN** the runtime MUST back-pressure the stream via gRPC flow control, stopping reads until the sliding window allows more data

---

### Requirement: Refcount Update Performance
Refcount update latency SHALL be less than 1 microsecond per operation. Refcount SHALL be an AtomicU32 in the resource store index, updated with a single atomic increment/decrement during mutation commit with no lock contention.
Source: RFC 0011 §4.5
Scope: v1-mandatory

#### Scenario: Atomic refcount performance
- **WHEN** a mutation commit increments or decrements a resource refcount
- **THEN** the operation MUST complete within 1 microsecond

---

### Requirement: BLAKE3 Hash Performance
BLAKE3 hash computation SHALL complete in less than 1 millisecond for a 1 MiB resource (BLAKE3 achieves approximately 3 GB/s on modern hardware).
Source: RFC 0011 §12
Scope: v1-mandatory

#### Scenario: Hash computation speed
- **WHEN** a 1 MiB resource is uploaded and its BLAKE3 hash is computed
- **THEN** the hash computation MUST complete within 1 millisecond

---

### Requirement: Zero Post-Revocation Resource Footprint
After an agent is revoked and its leases cleaned up, all resources with refcount == 0 SHALL be scheduled for GC within one GC cycle. After the grace period elapses, the agent's resource footprint MUST be zero.
Source: RFC 0011 §4.4, DR-RS9
Scope: v1-mandatory

#### Scenario: Complete cleanup after revocation
- **WHEN** Agent A is revoked, its leases cleaned up, and all its referenced resources reach refcount 0
- **THEN** after the grace period plus one GC cycle, Agent A's resource footprint MUST be exactly zero

---

### Requirement: Budget Enforcement at Mutation Time
Budget checks for texture_bytes_per_tile and texture_bytes_total SHALL occur in the mutation pipeline at per-mutation validation. Budget checks SHALL be all-or-nothing within a mutation batch (atomic pipeline). Budget SHALL be measured as decoded in-memory size, not raw upload size.
Source: RFC 0011 §11.2, §11.3
Scope: v1-mandatory

#### Scenario: Per-tile budget exceeded
- **WHEN** a mutation adds nodes to a single tile whose total decoded texture size would exceed texture_bytes_per_tile
- **THEN** the entire mutation batch MUST be rejected with BUDGET_EXCEEDED_TEXTURE_BYTES

---

### Requirement: GPU Texture Compression (BC7/ASTC)
GPU-native texture compression (BC7/ASTC) to reduce VRAM usage is deferred to post-v1 profiling.
Source: RFC 0011 §18.1
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** GPU texture compression is needed to reduce VRAM
- **THEN** this optimization is not available in v1; images are stored in decoded format

---

### Requirement: Persistent Resource Store
A durable persistent store for scene-node image/font resources is deferred to post-v1. The v1 durable-storage exception applies only to runtime widget SVG assets.
Source: RFC 0011 §9.2, §9.1
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** scene-node image/font resources need to survive runtime restarts
- **THEN** persistence is not implemented in v1 for those resource classes

---

### Requirement: Post-V1 Resource Types
VIDEO_H264, VIDEO_VP9, AUDIO_OPUS, AUDIO_AAC, and WASM_MODULE resource types are deferred to future RFCs.
Source: RFC 0011 §2.3
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** video, audio, or WASM resources are needed
- **THEN** these types are not available in v1 and will be added when corresponding node types ship
