# RFC 0011: Resource Store & Asset Lifecycle

**Status:** Draft
**Issue:** rig-lwj
**Date:** 2026-03-23
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0005 (Session Protocol), RFC 0008 (Lease and Resource Governance)

---

## Summary

This RFC specifies the Resource Store — the subsystem responsible for content-addressed storage, upload, reference counting, garbage collection, and eviction of assets (images and fonts) referenced by scene graph nodes. It gives concrete implementation contracts to concepts introduced in RFC 0001 (content-addressed `ResourceId`, durable blob storage, per-tile texture budget) and architecture.md (reference-counted resource lifecycle, deterministic deallocation). It also resolves a gap in the session protocol: RFC 0001 uses `ResourceId` in `StaticImageNode` and declares it requires upload, but no RFC specifies the upload mechanism.

All other RFCs defer to this document on resource lifecycle questions. Contradictions between this RFC and other RFCs on resource storage, eviction, or upload protocol are resolved here.

---

## Motivation

RFC 0001 §1.1 introduces `ResourceId` as a BLAKE3 content hash and declares resources are "ref-counted globally" (§1.2). Architecture.md §"Resource lifecycle" establishes the governing principle:

> Resources — textures, fonts, images, media handles — are reference-counted and owned by the scene graph. When the last reference is removed, the resource is freed. Resource leaks are treated as correctness bugs, not performance bugs.

V1 ships the `static_image` node type (v1.md §"Scene model"), which requires an image upload mechanism. RFC 0008 §6.1 defines per-lease `ResourceBudget` with `texture_bytes_total` and `texture_bytes_per_tile` fields. Despite this groundwork, the following contracts are missing:

- No upload protocol is specified. `StaticImageNode.resource_id` requires that an agent has already uploaded the resource, but the mechanism is undefined.
- Reference-count rules are stated in one sentence but not formalized. Who holds counts? When do they change? What happens at zero?
- Cross-agent sharing semantics are declared ("shared read-only across namespaces" in RFC 0001 §1.2) but no capability check or dedup protocol is specified.
- Font loading is mentioned in `TextMarkdownNode` (`FontFamily` field) but no loading, caching, or fallback contract exists.
- GC timing constraints relative to the frame loop are unspecified.
- Persistence model for resources is undefined.
- Size limits and rate limiting for uploads are unspecified.

Without these contracts, every implementation must make local decisions that will diverge, and resource leaks will be silent correctness failures.

---

## Design Requirements Satisfied

| ID | Requirement | Source |
|----|-------------|--------|
| DR-RS1 | Content-addressed deduplication: same bytes -> same ResourceId, stored once | RFC 0001 §1.1 |
| DR-RS2 | Upload protocol for agent-sourced assets | v1.md §"Scene model" (static_image node type) |
| DR-RS3 | Formal reference counting with GC trigger rules | architecture.md §"Resource lifecycle" |
| DR-RS4 | Cross-agent sharing policy with capability checks | RFC 0001 §1.2, security.md §"Agent isolation" |
| DR-RS5 | Font asset lifecycle: system fonts, bundled fonts, fallback chains | RFC 0001 §2.3 (`FontFamily`), RFC 0006 (Configuration) |
| DR-RS6 | Per-resource and per-agent size limits tied to lease budgets | RFC 0008 §6.1, security.md §"Resource governance" |
| DR-RS7 | GC must not run during frame render; deterministic deallocation | architecture.md §"Resource lifecycle" |
| DR-RS8 | v1: resources are ephemeral; storage model supports persistence post-v1 | v1.md §"V1 explicitly defers: Persistence" |
| DR-RS9 | Zero post-revocation footprint per agent | RFC 0002 §5.2, RFC 0008 §6.6 |

---

## 1. Content-Addressed Storage Model

### 1.1 Hash Function: BLAKE3

All resources are identified by their BLAKE3 hash. BLAKE3 is chosen for:

- **Speed:** ~3 GB/s on modern hardware. Hashing a 1 MiB resource takes < 1ms.
- **Cryptographic strength:** 256-bit collision resistance. No known practical attacks.
- **No length extension:** Unlike SHA-256, BLAKE3 is immune to length-extension attacks, preventing a class of hash manipulation.
- **Tree hashing:** BLAKE3 supports incremental/parallel hashing, enabling future chunked-upload hash verification without buffering the full resource.

### 1.2 ResourceId

`ResourceId` is the BLAKE3 hash of the raw input bytes (before any decode or transcoding), stored as 32 bytes:

```rust
/// Immutable resource ID — BLAKE3 digest of raw input bytes (32 bytes).
/// Computed before decode. Two uploads of identical bytes produce the same ResourceId.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId([u8; 32]);

impl ResourceId {
    /// Compute the ResourceId for a byte slice.
    pub fn from_bytes(data: &[u8]) -> Self {
        ResourceId(*blake3::hash(data).as_bytes())
    }
}
```

This matches RFC 0001 §1.1 which defines `ResourceId` as "BLAKE3 content hash (32 bytes, hex-encoded)" and the `scene.proto` message:

```protobuf
// BLAKE3 content hash: 32 bytes
message ResourceId {
  bytes bytes = 1;  // Must be exactly 32 bytes
}
```

### 1.3 Immutability

Once stored, content at a given `ResourceId` never changes. The resource store never mutates the content of an existing resource. If an upload produces a `ResourceId` that already exists in the store, the upload is a dedup hit — the existing content is returned without modification.

### 1.4 Namespace Independence

Resources are global, not per-agent. A `ResourceId` is a pure function of content bytes — the same bytes uploaded by any agent produce the same `ResourceId`. Ownership is tracked for accounting (§4.3) but does not affect identity or accessibility.

---

## 2. Resource Types (v1)

### 2.1 Type Enumeration

```protobuf
enum ResourceType {
  RESOURCE_TYPE_UNSPECIFIED = 0;
  IMAGE_RGBA8 = 1;        // Raw RGBA8 pixel data (width * height * 4 bytes)
  IMAGE_PNG   = 2;        // PNG-encoded image
  IMAGE_JPEG  = 3;        // JPEG-encoded image
  FONT_TTF    = 4;        // TrueType font
  FONT_OTF    = 5;        // OpenType font
}
```

### 2.2 v1 Scope

V1 supports two asset classes:

| Class | Types | Used By |
|-------|-------|---------|
| **Images** | `IMAGE_RGBA8`, `IMAGE_PNG`, `IMAGE_JPEG` | `StaticImageNode` (RFC 0001 §2.3) |
| **Fonts** | `FONT_TTF`, `FONT_OTF` | `TextMarkdownNode` custom font references |

### 2.3 Post-v1 Types

Future resource types (not part of this RFC's v1 scope):

- `VIDEO_H264`, `VIDEO_VP9` — video frames for media surface nodes
- `AUDIO_OPUS`, `AUDIO_AAC` — audio clips
- `IMAGE_SVG` — scalable vector graphics
- `WASM_MODULE` — sandboxed compute modules

These will be added as new `ResourceType` enum values in a future RFC when the corresponding node types ship.

---

## 3. Upload Protocol

### 3.1 Transport: Session Stream (RFC 0005)

Resources are uploaded via the session stream defined in RFC 0005. Upload messages are multiplexed over the agent's existing bidirectional gRPC session stream — there is no separate upload RPC or service. This follows the session protocol's "few fat streams" principle (architecture.md §"Session model").

### 3.2 Upload Flow

```
Agent                                       Runtime
  │                                            │
  ├──ResourceUploadStart ─────────────────────►│
  │  (expected_hash, resource_type,            │  1. Validate capability (upload_resource)
  │   total_size_bytes, metadata)              │  2. Check budget (texture_bytes_total)
  │                                            │  3. Check dedup (expected_hash already stored?)
  │                                            │  4. If dedup hit → immediate ResourceStored
  │  ◄────────────────── ResourceStored ───────┤     (skip chunks, no storage consumed)
  │                                            │
  │  OR (if not a dedup hit):                  │
  │                                            │  5. Allocate upload_id
  ├──ResourceUploadChunk (chunk 0) ───────────►│  6. Receive and hash chunks
  ├──ResourceUploadChunk (chunk 1) ───────────►│
  ├──  ... (up to total_size_bytes)            │
  ├──ResourceUploadComplete ──────────────────►│  7. Validate hash matches
  │                                            │  8. Decode and validate content
  │                                            │  9. Store in resource store
  │  ◄────────────────── ResourceStored ───────┤  10. Return confirmed ResourceId
```

### 3.3 Small Resource Fast Path

Resources <= 64 KiB may be uploaded in a single `ResourceUploadStart` message by including the content bytes in the `inline_data` field. This avoids the chunked upload overhead for small images and fonts. The `ResourceUploadChunk`/`ResourceUploadComplete` sequence is not used for inline uploads.

### 3.4 Upload Messages (Protobuf)

```protobuf
// Metadata about the resource, provided with the upload start or as part of
// ResourceStored confirmation.
message ResourceMetadata {
  // For images:
  uint32 width  = 1;     // Image width in pixels (0 if not an image)
  uint32 height = 2;     // Image height in pixels (0 if not an image)

  // For fonts:
  string font_family = 3;  // Font family name (empty if not a font)
  string font_style  = 4;  // e.g., "Regular", "Bold", "Italic" (empty if not a font)
}

// Initiates a resource upload. For resources <= 64 KiB, include inline_data
// and omit the chunk/complete sequence.
message ResourceUploadStart {
  bytes          expected_hash   = 1;  // BLAKE3 of full content (32 bytes)
  ResourceType   resource_type   = 2;
  uint64         total_size_bytes = 3;
  ResourceMetadata metadata      = 4;
  bytes          inline_data     = 5;  // If non-empty: full resource content (≤ 64 KiB)
}

// A chunk of resource data for uploads > 64 KiB.
message ResourceUploadChunk {
  bytes  upload_id   = 1;   // Assigned by runtime in response to ResourceUploadStart
  uint32 chunk_index = 2;   // 0-based, sequential
  bytes  data        = 3;   // Chunk payload; max chunk size: 64 KiB
}

// Signals the end of a chunked upload.
message ResourceUploadComplete {
  bytes upload_id = 1;
}

// Server acknowledgement of a successful upload (or dedup hit).
message ResourceStored {
  ResourceId       resource_id      = 1;  // Confirmed BLAKE3 hash
  bool             was_deduplicated = 2;  // True if resource already existed
  uint64           stored_bytes     = 3;  // Bytes consumed (0 if deduplicated)
  uint64           decoded_bytes    = 4;  // In-memory decoded size (GPU texture bytes for images)
  ResourceMetadata metadata         = 5;  // Confirmed metadata (width/height populated after decode)
}
```

### 3.5 Upload Validation

Before storing, the runtime validates:

1. **Capability:** Agent must hold the `upload_resource` capability (included in the default resident capability set, RFC 0005 §7).
2. **Hash integrity:** Computed BLAKE3 of received bytes must match `expected_hash`. Mismatch -> `RESOURCE_HASH_MISMATCH` error.
3. **Size within limits:** Total bytes must not exceed the per-resource size limit (§8.1). Violation -> `RESOURCE_SIZE_EXCEEDED` error.
4. **Budget check:** Upload must not exceed the agent's `texture_bytes_total` budget (RFC 0008 §6.1). Violation -> `RESOURCE_BUDGET_EXCEEDED` error.
5. **Type supported:** `resource_type` must be a v1-supported type (§2.1). Violation -> `RESOURCE_UNSUPPORTED_TYPE` error.
6. **Decode validation:** For images, the data must decode successfully. For fonts, the font face must parse. Corruption -> `RESOURCE_DECODE_ERROR`.

### 3.6 Deduplication

When the runtime receives a `ResourceUploadStart`:

1. Compute `ResourceId = BLAKE3(expected_hash)` — the agent provides the expected hash.
2. Look up `expected_hash` in the resource store index.
3. If found: return `ResourceStored` immediately with `was_deduplicated = true`. No chunks are needed; the agent skips the chunk/complete sequence.
4. If not found: proceed with chunked upload.

Dedup check latency: < 100us (hash table lookup).

### 3.7 Concurrent Upload Limits

An agent may have at most **4 concurrent uploads** in flight. A 5th `ResourceUploadStart` before a previous upload completes is rejected with `RESOURCE_TOO_MANY_UPLOADS`. This prevents a single agent from monopolizing upload bandwidth.

### 3.8 Upload Error Codes

```protobuf
enum ResourceError {
  RESOURCE_ERROR_UNSPECIFIED       = 0;
  RESOURCE_CAPABILITY_DENIED       = 1;   // Agent lacks upload_resource capability
  RESOURCE_BUDGET_EXCEEDED         = 2;   // Would exceed texture_bytes_total budget
  RESOURCE_SIZE_EXCEEDED           = 3;   // Single resource exceeds per-resource size limit
  RESOURCE_UNSUPPORTED_TYPE        = 4;   // ResourceType not accepted in v1
  RESOURCE_DECODE_ERROR            = 5;   // Image/font decode failed
  RESOURCE_HASH_MISMATCH           = 6;   // expected_hash does not match received bytes
  RESOURCE_CONFLICT                = 7;   // ResourceId exists with different content (defensive)
  RESOURCE_TOO_MANY_UPLOADS        = 8;   // Max concurrent uploads (4) exceeded
  RESOURCE_NOT_FOUND               = 9;   // Query for a ResourceId that does not exist
}
```

---

## 4. Reference Counting

### 4.1 Refcount Rules

The resource store maintains an integer `refcount` for each stored resource. Refcount changes are atomic and occur on the compositor thread during mutation commit (stage 4 of RFC 0002's frame pipeline).

| Event | Refcount Change | Notes |
|-------|-----------------|-------|
| `CreateNode` with a `ResourceId` (e.g., `StaticImageNode`) | +1 | Any node type that embeds a `ResourceId` |
| `UpdateNode` replacing a `ResourceId` | old -1, new +1 | Atomic within the mutation batch |
| `DeleteNode` referencing a `ResourceId` | -1 | Explicit node deletion |
| Tile deletion (cascades to all nodes) | -1 per node | Tile delete, tab delete, or lease revocation |
| Lease revocation (cascades to all tiles) | -1 per node per tile | RFC 0008 §6.6 cleanup |
| Upload completes (new resource stored) | 0 | Newly uploaded resource starts at refcount 0 |
| Runtime restart | Reconstructed | See §9.2 |

### 4.2 Scene-Graph-Level Refcount

Refcount is scene-graph-level, not per-agent. The refcount tracks how many scene graph nodes reference the resource, regardless of which agent owns those nodes. This enables natural cross-agent sharing:

- Agent A uploads image X -> refcount = 0
- Agent A creates a `StaticImageNode` referencing X -> refcount = 1
- Agent B creates a `StaticImageNode` referencing X -> refcount = 2
- Agent A deletes its node -> refcount = 1 (resource stays live because Agent B still references it)
- Agent B deletes its node -> refcount = 0 (resource enters GC candidacy)

### 4.3 Budget Accounting

Texture bytes count against the agent whose **node** references the resource, not the uploader. When a mutation commits a node referencing a resource, the referenced resource's decoded size is added to the mutating agent's lease `texture_bytes_total` consumption (RFC 0008 §6.2).

**Per-agent double-counting for shared resources:** If Agent A and Agent B both reference the same resource, both agents are charged the full decoded size against their respective budgets. This prevents a coordinated multi-agent budget bypass.

### 4.4 Refcount Consistency Invariants

The runtime enforces these invariants (violation -> panic in debug, structured error log in release):

1. `refcount >= 0` at all times. Underflow is a bug.
2. A resource referenced by a live scene node always has `refcount >= 1`.
3. After a tile is deleted, no node from that tile retains a reference.
4. After a session is revoked and its leases cleaned up, all resources with `refcount == 0` are scheduled for GC within one GC cycle.

### 4.5 Refcount Update Performance

Refcount update latency: **< 1us per operation**. Refcount is an `AtomicU32` in the resource store index, updated with a single atomic increment/decrement during mutation commit. No lock contention.

---

## 5. Cross-Agent Sharing

### 5.1 Content-Addressed = Capability-less Read

Any agent can reference any `ResourceId` if they know the hash. Content-addressed identity is the capability: knowing the hash is sufficient to reference the resource. There is no access control list or ownership gate on reading.

This follows RFC 0001 §1.2: "`ResourceId` is namespace-agnostic: resources are shared read-only across namespaces."

### 5.2 Upload Requires Capability

Uploading a resource requires the `upload_resource` capability (included in the default resident capability set per RFC 0005 §7). Guest agents cannot upload resources — they interact through MCP zone tools which handle resource management internally.

### 5.3 Sharing Semantics

Cross-namespace resource sharing is read-only and content-addressed:

- Agent A uploads a PNG -> `ResourceId(abc123...)`.
- Agent B uploads the same PNG -> server returns `ResourceId(abc123...)` (dedup hit, `was_deduplicated = true`).
- Agent B creates a `StaticImageNode` with `resource_id = abc123...` -> refcount +1.
- Agent A's lease is revoked -> Agent A's nodes are deleted -> refcount -1. But Agent B's node still holds a reference -> `refcount >= 1` -> resource is **not** evicted.

### 5.4 No Resource Enumeration

There is no "list all resources" operation. Agents can only query resources they know the `ResourceId` of. This prevents resource discovery attacks — Agent B cannot enumerate what Agent A has uploaded.

---

## 6. Garbage Collection

### 6.1 GC Candidacy

Resources with `refcount == 0` enter **GC candidacy**. A candidate resource:

- Remains in the resource store.
- Is not immediately deleted.
- Has a **grace period** timer started at the moment refcount reaches zero.
- Can be resurrected if re-referenced before eviction (§6.5).

### 6.2 Grace Period

A GC candidate resource must remain at `refcount == 0` for the full **grace period** before it is eligible for deletion. The grace period is configurable:

- **Default:** 60 seconds
- **Minimum:** 1 second
- **Maximum:** 3600 seconds (1 hour)

**Rationale:** Agents frequently delete and recreate nodes as they update content. Immediate GC on refcount-0 would cause repeated upload-decode-free-upload cycles for resources the agent will reference again shortly. The grace period acts as a soft cache for recently-dereferenced resources.

Grace period timing uses monotonic clock (`Instant::now()`), not frame count.

### 6.3 GC Cycle

The GC runs on a configurable timer:

- **Default interval:** 30 seconds
- **Minimum interval:** 5 seconds
- **Maximum interval:** 300 seconds

Each GC cycle:

1. Scans all GC candidate resources.
2. Evicts candidates whose grace period has fully elapsed.
3. Frees decoded in-memory data (GPU textures, font face objects).
4. Removes the resource from the store index.
5. For persistent stores (post-v1): removes the blob from disk.

### 6.4 GC Timing Constraints

**GC never runs during frame render.** The GC phase runs as a compositor-thread epilogue after stage 7 (GPU Submit + Present) completes, before the compositor thread begins stage 3 of the next frame (RFC 0002 §3.2). This places GC in the inter-frame idle window.

**GC budget:** Each GC cycle has a time budget of **5ms**. If the eviction backlog requires more than 5ms, remaining work is deferred to the next GC cycle. This prevents GC from causing frame drops.

### 6.5 Resurrection

A GC candidate resource (refcount == 0, grace period not yet elapsed) can be **resurrected**:

- A scene mutation creates a node referencing the candidate resource.
- The mutation pipeline increments the refcount (+1).
- The resource transitions from GC candidacy back to live.
- Its decoded in-memory form (if still resident) does not need to be reloaded.

If the resource has already been evicted from memory but was not yet deleted from disk (post-v1 persistent mode), resurrection requires a re-decode. The mutation is rejected with `RESOURCE_NOT_RESIDENT` (retriable). The runtime enqueues a re-decode job, and the agent retries after a `ResourceResidentEvent` notification.

### 6.6 GC and Frame Render Isolation

GC must:

- Not acquire any lock held by the render pipeline during stages 4-7.
- Not touch GPU device state during frame render.
- Not deallocate GPU textures that are bound to the current frame's draw calls.

Memory deallocation (freeing decoded textures) runs exclusively in the GC phase. If a texture is still needed for the current frame's render, its eviction is deferred to the next GC cycle.

---

## 7. Font Asset Management

### 7.1 Font Sources

Fonts enter the resource store from three sources:

| Source | ResourceId | Lifecycle |
|--------|------------|-----------|
| **System fonts** | Computed from file bytes at startup | Always available; synthetic `ResourceId` from system font bytes |
| **Bundled fonts** | Compiled into binary; `ResourceId` computed from embedded bytes | Always available; included in compositor binary |
| **Agent-uploaded fonts** | Computed from upload bytes (same as image upload) | Same lifecycle as images: reference-counted, GC-eligible |

System and bundled fonts are available to all agents without upload. Their `ResourceId` values are stable across sessions (same binary + same system font installation = same IDs).

### 7.2 Font Discovery at Startup

On startup, the runtime:

1. Scans platform font directories (e.g., `/usr/share/fonts/` on Linux, `~/Library/Fonts/` on macOS, `C:\Windows\Fonts\` on Windows).
2. For each font file: computes `ResourceId = BLAKE3(file_bytes)` and registers it with the resource store.
3. Registers bundled fonts (compiled into the binary) the same way.
4. System and bundled fonts have permanent implicit holds — they are never GC'd.

### 7.3 FontFamily Resolution

RFC 0001 §2.3 defines `FontFamily` with named variants (`SystemSansSerif`, `SystemMonospace`, `SystemSerif`). Resolution:

1. Named variants are resolved against the font resolution table configured in the display profile (RFC 0006).
2. If a custom font `ResourceId` is specified: look up in the resource store. If found: use it. If not found: fall back to `SystemSansSerif`.
3. If resolution fails: use bundled default (Noto Sans for SansSerif/Serif; bundled monospace for Monospace).

**Fallback is transparent.** Agents are not notified when a font fallback occurs. Telemetry tracks font fallback events per frame for debugging.

### 7.4 Font Fallback Chain

The font fallback chain is configurable per display profile (RFC 0006):

```toml
[resources.fonts]
sans_serif    = "system:Noto Sans"
monospace     = "system:JetBrains Mono"
serif         = "system:Noto Serif"
```

If a configured font is not found, the bundled default is used without error.

### 7.5 Font Cache

Fonts are cached in an LRU cache bounded by a configurable maximum size:

- **Default:** 64 MiB
- Includes: loaded font faces, shaped glyph caches, rasterized glyph atlases.

Font cache eviction is separate from texture GC:

- Font glyph caches are evicted LRU when the font memory budget is exceeded.
- A font face with `refcount == 0` (agent-uploaded, no nodes referencing it) is eligible for full eviction.
- System and bundled fonts are never evicted (permanent implicit holds).

**Font rendering is always on the compositor thread.** Font layout and rasterization happen in stage 5 (Layout Resolve) of the frame pipeline (RFC 0002). Font cache access is serialized by the compositor thread — no thread-safety concerns.

---

## 8. Size Limits

### 8.1 Per-Resource Limits

| Dimension | Limit | Configurable |
|-----------|-------|-------------|
| Maximum input size per resource | 16 MiB | Yes |
| Maximum decoded texture size | 64 MiB (e.g., 4096x4096 RGBA8) | Yes |
| Maximum texture dimension | 8192 pixels (either width or height) | No |

Resources exceeding the input size limit are rejected at upload time with `RESOURCE_SIZE_EXCEEDED`.

### 8.2 Per-Lease Storage Budget

Per-lease texture budgets are defined by `ResourceBudget.texture_bytes_total` (RFC 0008 §6.1). The decoded in-memory size of each referenced resource counts against this budget, not the raw upload size.

### 8.3 Per-Runtime Total

| Dimension | Default | Configurable |
|-----------|---------|-------------|
| Maximum total texture memory (all resources) | 512 MiB | Yes |
| Maximum total font cache memory | 64 MiB | Yes |
| Maximum concurrent resources | 4096 | Yes |

When the runtime-wide texture memory limit is reached, new uploads are rejected with `RESOURCE_BUDGET_EXCEEDED` even if the individual agent has budget remaining.

### 8.4 Upload Rate Limit

Each agent is rate-limited to **1 MiB/s** upload throughput (configurable). This prevents a single agent from saturating network/disk bandwidth with uploads, ensuring other agents can upload concurrently.

Rate limiting is enforced per-session, measured as a sliding window over the last 1 second. Upload chunks that would exceed the rate limit are back-pressured via gRPC flow control — the runtime stops reading from the stream until the window allows more data.

---

## 9. Persistence

### 9.1 v1: Resources Are Ephemeral

In v1, all resources are ephemeral. They are stored in memory and **lost on runtime restart**. This is consistent with the scene graph being ephemeral (RFC 0001 §6.2, v1.md §"V1 explicitly defers: Persistence"):

- Agents re-establish sessions on reconnect.
- Agents re-create scene state, including re-uploading images and fonts.
- The content-addressed model makes re-upload idempotent: the same bytes produce the same `ResourceId`.

### 9.2 Storage Model Supports Persistence (Post-v1)

The storage model is designed to support persistence in a future version:

- **Content-addressed identity** means a persistent store can be verified on startup by recomputing hashes.
- **Refcount reconstruction:** On startup, the runtime would scan the persistent blob store and reconstruct refcounts by walking the restored scene graph.
- **Cache directory:** The natural persistence location is `$XDG_CACHE_HOME/tze_hud/resources/` (Linux), `~/Library/Caches/tze_hud/resources/` (macOS), `%LOCALAPPDATA%\tze_hud\resources\` (Windows).

Post-v1, a `durable` flag on upload would mark resources for persistence. Durable resources would survive restarts and be loaded from the cache directory at startup. This RFC defines the storage model; the persistence implementation is deferred.

### 9.3 Refcount Reconstruction on Restart

When persistence is implemented (post-v1), restart reconstruction will:

1. Load the persistent blob store index.
2. Load the scene graph from durable config (tabs, zone registry — RFC 0001 §6.1).
3. Walk all node definitions and increment refcounts for every referenced `ResourceId`.
4. Set all other resources to `refcount = 0` (GC candidates).

Resources with `refcount == 0` after reconstruction are eligible for eviction on the first GC pass.

---

## 10. Wire Protocol

### 10.1 Session Stream Integration

Resource upload messages are carried on the session stream (RFC 0005). The following message types are added to the `SessionMessage` oneof:

```protobuf
// Added to SessionMessage oneof (client → server):
//   resource_upload_start    = <field_number>;
//   resource_upload_chunk    = <field_number>;
//   resource_upload_complete = <field_number>;
//   resource_query           = <field_number>;

// Added to SessionMessage oneof (server → client):
//   resource_stored          = <field_number>;
//   resource_query_result    = <field_number>;
//   resource_error           = <field_number>;
```

Field numbers are allocated from the RFC 0005 §9.2 field registry, coordinated with the session protocol maintainers.

### 10.2 Complete Protobuf Definitions

```protobuf
syntax = "proto3";
package tze_hud.resource.v1;

import "scene.proto";  // For ResourceId

// ─── Resource Types ─────────────────────────────────────────────────────────

enum ResourceType {
  RESOURCE_TYPE_UNSPECIFIED = 0;
  IMAGE_RGBA8 = 1;        // Raw RGBA8 pixel data (width * height * 4 bytes)
  IMAGE_PNG   = 2;        // PNG-encoded image
  IMAGE_JPEG  = 3;        // JPEG-encoded image
  FONT_TTF    = 4;        // TrueType font
  FONT_OTF    = 5;        // OpenType font
}

// ─── Metadata ───────────────────────────────────────────────────────────────

message ResourceMetadata {
  // For images:
  uint32 width  = 1;     // Image width in pixels (0 if not an image)
  uint32 height = 2;     // Image height in pixels (0 if not an image)

  // For fonts:
  string font_family = 3;  // Font family name (empty if not a font)
  string font_style  = 4;  // e.g., "Regular", "Bold", "Italic" (empty if not a font)
}

// ─── Upload Messages ────────────────────────────────────────────────────────

// Initiates a resource upload. For resources ≤ 64 KiB, include inline_data
// and omit the chunk/complete sequence.
message ResourceUploadStart {
  bytes            expected_hash    = 1;  // BLAKE3 of full content (32 bytes)
  ResourceType     resource_type    = 2;
  uint64           total_size_bytes = 3;
  ResourceMetadata metadata         = 4;
  bytes            inline_data      = 5;  // Full content for small resources (≤ 64 KiB)
}

// A chunk of resource data for uploads > 64 KiB.
message ResourceUploadChunk {
  bytes  upload_id   = 1;   // Assigned by runtime on ResourceUploadStart acceptance
  uint32 chunk_index = 2;   // 0-based, sequential
  bytes  data        = 3;   // Max chunk size: 64 KiB
}

// Signals completion of a chunked upload.
message ResourceUploadComplete {
  bytes upload_id = 1;
}

// ─── Server Responses ───────────────────────────────────────────────────────

// Confirms a successful upload or dedup hit.
message ResourceStored {
  tze_hud.scene.v1.ResourceId resource_id = 1;  // Confirmed BLAKE3 hash
  bool             was_deduplicated       = 2;   // True if resource already existed
  uint64           stored_bytes           = 3;   // Bytes consumed (0 if deduplicated)
  uint64           decoded_bytes          = 4;   // In-memory decoded size
  ResourceMetadata metadata               = 5;   // Confirmed metadata (populated after decode)
}

// ─── Query Messages ─────────────────────────────────────────────────────────

// Check if a resource exists and retrieve its metadata.
message ResourceQuery {
  tze_hud.scene.v1.ResourceId resource_id = 1;
}

message ResourceQueryResult {
  bool             exists         = 1;
  ResourceType     resource_type  = 2;   // UNSPECIFIED if !exists
  uint64           stored_bytes   = 3;
  uint64           decoded_bytes  = 4;
  uint32           refcount       = 5;
  ResourceMetadata metadata       = 6;
  bool             is_gpu_resident = 7;  // Currently loaded in GPU memory
}

// ─── Error Messages ─────────────────────────────────────────────────────────

enum ResourceErrorCode {
  RESOURCE_ERROR_UNSPECIFIED       = 0;
  RESOURCE_CAPABILITY_DENIED       = 1;
  RESOURCE_BUDGET_EXCEEDED         = 2;
  RESOURCE_SIZE_EXCEEDED           = 3;
  RESOURCE_UNSUPPORTED_TYPE        = 4;
  RESOURCE_DECODE_ERROR            = 5;
  RESOURCE_HASH_MISMATCH           = 6;
  RESOURCE_CONFLICT                = 7;   // Defensive: hash collision (should never happen)
  RESOURCE_TOO_MANY_UPLOADS        = 8;
  RESOURCE_NOT_FOUND               = 9;
  RESOURCE_NOT_RESIDENT            = 10;  // Retriable: resource evicted, re-decode enqueued
  RESOURCE_RATE_LIMITED             = 11;  // Upload rate limit exceeded
}

message ResourceErrorResponse {
  ResourceErrorCode error_code   = 1;
  string            error_detail = 2;   // Human-readable; not stable
  bytes             upload_id    = 3;   // If applicable
}
```

---

## 11. Relationship to RFC 0008

RFC 0008 (Lease and Resource Governance) defines the per-lease `ResourceBudget` that governs agent resource consumption. This section clarifies the interaction between the Resource Store and lease-level budget enforcement.

### 11.1 Budget Dimensions Affected by Resources

From RFC 0008 §6.1:

```rust
pub struct ResourceBudget {
    pub texture_bytes_per_tile: u64,    // Max texture memory for a single tile's nodes
    pub texture_bytes_total: u64,       // Aggregate texture bytes across all lease tiles
    // ... other fields
}
```

The Resource Store contributes to two budget dimensions:

| Budget Dimension | How Resources Contribute |
|-----------------|------------------------|
| `texture_bytes_per_tile` | Sum of decoded sizes of all resources referenced by nodes in that tile |
| `texture_bytes_total` | Sum of decoded sizes of all resources referenced by nodes across all tiles under the lease |

### 11.2 Accounting Rules

1. **Decoded size, not raw size:** A 500 KiB compressed PNG that decodes to 4 MiB RGBA8 counts as 4 MiB against the budget.
2. **Counted at reference time:** Budget is checked when a mutation creates or updates a node that references a `ResourceId`. The upload itself does not count against the texture budget — only scene graph references do.
3. **Per-agent double-counting for shared resources:** If two agents reference the same resource, each agent bears the full decoded cost. This prevents coordinated budget bypass.
4. **Released on dereference:** When a node is deleted and its `ResourceId` reference is removed, the decoded size is subtracted from the agent's budget consumption.

### 11.3 Enforcement Points

Budget checks occur in the mutation pipeline (RFC 0001 §4, RFC 0008 §6.4):

| Stage | Check | Failure Action |
|-------|-------|---------------|
| Per-mutation validation | `texture_bytes_per_tile` for node with texture | `BUDGET_EXCEEDED_TEXTURE_BYTES` |
| Per-mutation validation | `texture_bytes_total` aggregate | `BUDGET_EXCEEDED_TEXTURE_TOTAL` |

Budget checks are all-or-nothing within a mutation batch (RFC 0001 §4 atomic pipeline).

---

## 12. Quantitative Requirements Summary

| Metric | Requirement | Notes |
|--------|-------------|-------|
| Upload throughput per agent | Saturate 1 MiB/s rate limit | Agent should be able to sustain max upload rate |
| BLAKE3 hash computation | < 1ms for 1 MiB resource | BLAKE3 achieves ~3 GB/s on modern hardware |
| Dedup check | < 100us | Hash table lookup in resource store index |
| GC cycle duration | < 5ms | Per-cycle budget; excess deferred to next cycle |
| Refcount update | < 1us per operation | Atomic integer operation |
| Max concurrent uploads per agent | 4 | Prevents monopolization |
| Max resources per runtime | 4096 | Configurable |
| Max total texture memory | 512 MiB | Configurable |
| Max per-resource input size | 16 MiB | Configurable |
| Upload rate limit | 1 MiB/s per agent | Sliding window; configurable |
| GC grace period | 60 seconds | Configurable |
| GC cycle interval | 30 seconds | Configurable |
| Font cache size | 64 MiB | Configurable, LRU eviction |

---

## 13. Configuration

All resource store parameters are configurable in the runtime configuration file (RFC 0006):

```toml
[resources]
max_resource_size_mib      = 16     # Per-resource input size limit
max_texture_memory_mib     = 512    # Total GPU texture memory for all resources
max_resources              = 4096   # Maximum number of stored resources
gc_grace_period_secs       = 60     # Pending-GC grace period before eviction
gc_interval_secs           = 30     # How often the GC cycle runs
upload_rate_limit_mib_s    = 1      # Per-agent upload throughput limit
max_concurrent_uploads     = 4      # Per-agent concurrent upload limit
max_texture_dimension      = 8192   # Maximum pixel dimension (width or height)

[resources.fonts]
max_font_cache_mib         = 64     # Font cache memory limit (LRU)
sans_serif                 = "system:Noto Sans"
monospace                  = "system:monospace"
serif                      = "system:serif"
min_size_px                = 8.0    # Minimum rendered font size
max_size_px                = 256.0  # Maximum rendered font size
```

---

## 14. Security Considerations

### 14.1 Content-Address Collision Resistance

BLAKE3 is collision-resistant for any practical input. Two different byte sequences producing the same `ResourceId` is computationally infeasible. The `RESOURCE_CONFLICT` error is a defensive assertion for storage corruption detection, not hash collision handling.

### 14.2 Decompression Attack

A small compressed PNG could expand to a very large decoded texture. Defenses:

- Per-resource decoded size limit (64 MiB for textures, §8.1).
- Pixel dimension cap (8192 x 8192, §8.1).
- Decode runs in a memory-capped arena on the upload thread pool. A decode that exceeds the limit is aborted mid-decode with `RESOURCE_SIZE_EXCEEDED`.

### 14.3 Resource Exfiltration

Agents can only query resources they know the `ResourceId` of. There is no enumeration RPC. The `ResourceQuery` RPC returns metadata but not content bytes. Content download requires a separate `READ_RESOURCE_CONTENT` capability not granted to standard resident agents.

### 14.4 Budget Bypass via Sharing

Two agents each referencing the same resource are each charged the full decoded size (§4.3, §11.2). This prevents coordinated budget bypass.

---

## 15. Interaction with Other RFCs

| RFC | Interaction |
|-----|-------------|
| **RFC 0001 §1.1** | `ResourceId` definition (BLAKE3, 32 bytes) and `StaticImageNode.resource_id` — this RFC provides the upload and lifecycle contract |
| **RFC 0001 §6.1** | Durable state includes "uploaded resources" — this RFC specifies that v1 resources are ephemeral; persistence deferred to post-v1 |
| **RFC 0002 §5.2** | Resource cleanup on revocation — this RFC specifies: release all references, decrement refcounts, schedule pending-GC |
| **RFC 0005** | Session stream carries upload messages — this RFC specifies the `ResourceUploadStart`, `ResourceUploadChunk`, `ResourceUploadComplete`, `ResourceStored` message types |
| **RFC 0006** | Font fallback chain configured per display profile — this RFC specifies font resolution order and fallback semantics |
| **RFC 0008 §6.1** | `ResourceBudget.texture_bytes_total` and `texture_bytes_per_tile` — this RFC clarifies that decoded in-memory size counts against budgets, and sharing causes per-agent double-counting |

---

## 16. Rust Module Overview

```
tze_resource/
├── lib.rs                — ResourceStore trait, ResourceId, ResourceType
├── store.rs              — In-memory resource index, refcount table, dedup lookup
├── upload.rs             — Upload pipeline: receive, hash, validate, decode, store
├── gc.rs                 — GC phase: candidacy tracking, eviction logic, grace period
├── font.rs               — FontRegistry: resolution table, system/bundled/uploaded fonts
├── budget.rs             — Budget integration with RFC 0008 AgentResourceState
└── proto/
    └── resource.proto    — Canonical proto schema from §10
```

The `ResourceStore` is owned by the compositor thread. Upload processing runs on a dedicated thread pool (sized `min(num_cpus, 4)`). The upload thread pool communicates with the compositor thread via a channel — completed uploads are enqueued and picked up by the compositor thread in the next frame's intake phase.

---

## 17. Validation Scenes

These test scenes verify the resource lifecycle contract:

| Scene Name | What It Tests |
|------------|---------------|
| `resource_upload_dedup` | Two agents upload identical bytes; verify `was_deduplicated = true`, one storage entry, shared refcount |
| `resource_lifecycle_consistency` | Upload -> node create -> node delete -> verify refcount -> GC candidacy -> wait grace -> verify eviction |
| `resource_resurrection` | Upload -> create node -> delete node (GC candidate) -> create new node before grace period -> verify resurrection, no re-upload needed |
| `resource_revocation_cleanup` | Agent uploads + references resource; agent revoked; verify post-revocation footprint == 0 within grace period + 1 GC cycle |
| `resource_budget_enforcement` | Agent uploads resources up to budget limit; next mutation rejected with `BUDGET_EXCEEDED_TEXTURE_TOTAL` |
| `resource_cross_agent_sharing` | Agent A uploads; Agent B references by ResourceId; Agent A revoked; verify resource stays live (Agent B holds refcount) |
| `resource_rate_limit` | Agent uploads at > 1 MiB/s; verify back-pressure / rate limit enforcement |
| `font_fallback_chain` | Configure invalid `sans_serif` font; verify fallback to bundled default; telemetry records fallback event |
| `resource_gc_frame_safety` | Trigger GC during active rendering; verify no frame drop and no GPU texture freed mid-render |
| `resource_leak_soak` | Repeated agent connect/upload/disconnect/reconnect over 1 hour; verify texture memory does not grow monotonically |

---

## 18. Open Questions

1. **GPU texture compression.** Should the resource store transcode PNG/JPEG inputs to BC7/ASTC GPU-native compressed formats on upload? This reduces VRAM usage by 4-8x but adds transcoding latency. Deferred to post-v1 profiling.

2. **Persistent resource store.** The storage model supports persistence (§9.2) but v1 does not implement it. The persistence format (flat-file shard directory vs. embedded database), cleanup policy, and cache directory structure are deferred to the post-v1 persistence RFC.

3. **Font preloading.** Should the runtime preload all configured system fonts at startup, or lazy-load on first use? Lazy-load keeps startup time low but causes a first-use latency spike. Recommended: lazy-load with a configurable preload list.

4. **WebP/AVIF support.** V1 supports PNG and JPEG. WebP and AVIF are valuable for smaller file sizes but add decode dependencies. Candidates for a v1.1 addition.

---

## Appendix A: Performance Budget Summary

| Operation | Budget | Notes |
|-----------|--------|-------|
| BLAKE3 hash (1 MiB) | < 1ms | ~3 GB/s on modern hardware |
| Dedup check | < 100us | Hash table lookup |
| Upload header validation | < 5ms | Capability + budget + dedup check |
| Full upload + store (≤ 1 MiB) | < 200ms | On upload thread pool |
| Texture decode (≤ 4096x4096) | ≤ 50ms | Included in upload store |
| Font decode (≤ 1 MiB) | ≤ 20ms | Included in upload store |
| Refcount change (per node) | < 1us | Compositor thread, frame pipeline |
| GC cycle | ≤ 5ms | Inter-frame; excess deferred |
| Resurrection re-decode enqueue | < 1ms | Compositor thread rejects with RESOURCE_NOT_RESIDENT |
