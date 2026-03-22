# RFC 0011: Resource Store and Asset Lifecycle

**Status:** Draft
**Issue:** rig-lwj
**Date:** 2026-03-22
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0005 (Session Protocol), RFC 0008 (Lease and Resource Governance)

---

## Summary

This RFC specifies the Resource Store — the subsystem responsible for loading, tracking, deduplicating, and evicting assets (textures, fonts, and raw buffers) referenced by scene graph nodes. It gives concrete implementation contracts to concepts introduced informally in RFC 0001 (content-addressed `ResourceId`, durable blob storage, per-tile texture budget) and architecture.md (reference-counted resource lifecycle, deterministic deallocation). It also resolves a gap in the session protocol: RFC 0001 uses `ResourceId` in `StaticImageNode` and declares it requires upload, but no RFC specifies the upload RPC.

All other RFCs defer to this document on resource lifecycle questions. Contradictions between this RFC and other RFCs on resource storage, eviction, or upload protocol are resolved here.

---

## Motivation

RFC 0001 §1.1 introduces `ResourceId` as a BLAKE3 content hash and declares resources are "ref-counted globally" (§1.2). Architecture.md §"Resource lifecycle" establishes the governing principle: resources are reference-counted, owned by the scene graph, and must have a zero post-deallocation footprint. RFC 0008 §4.1 provides per-agent texture byte budgets at the lease level.

Despite this groundwork, the following contracts are missing:

- No upload RPC is specified. `StaticImageNode.resource_id` requires that an agent has already uploaded the resource, but the mechanism is undefined.
- Reference-count rules are stated in one sentence but not formalized. Who holds counts? When do they change? What happens at zero?
- Cross-agent sharing semantics are declared ("shared read-only across namespaces" in RFC 0001 §1.2) but no capability check or dedup protocol is specified.
- Font loading is mentioned in `TextMarkdownNode` (`FontFamily` field) but no loading, caching, or fallback contract exists.
- GC timing constraints relative to the frame loop are unspecified.
- Persistence format and cleanup policy for the blob store are undefined.
- Cache eviction for zero-refcount resources that haven't been GC'd yet is unspecified.

Without these contracts, every implementation must make local decisions that will diverge, and resource leaks will be silent correctness failures.

---

## Design Requirements Satisfied

| ID | Requirement | Source |
|----|-------------|--------|
| DR-RS1 | Content-addressed deduplication: same bytes → same ResourceId, stored once | RFC 0001 §1.1 |
| DR-RS2 | Upload RPC for agent-sourced assets | v1.md §"Scene model" (static_image node type) |
| DR-RS3 | Formal reference counting with GC trigger rules | architecture.md §"Resource lifecycle" |
| DR-RS4 | Cross-agent sharing policy with capability checks | RFC 0001 §1.2, security.md §"Agent isolation" |
| DR-RS5 | Font asset lifecycle: system fonts, bundled fonts, fallback chains | RFC 0001 §2.3 (`FontFamily`) |
| DR-RS6 | Per-resource and per-agent size limits tied to lease budgets | RFC 0008 §4.1, security.md §"Resource governance" |
| DR-RS7 | GC must not run during frame render; deterministic deallocation | architecture.md §"Resource lifecycle" |
| DR-RS8 | Durable storage: resources survive runtime restart | RFC 0001 §6.1 |
| DR-RS9 | Zero post-revocation footprint per agent | RFC 0002 §5.2, RFC 0008 §4.4 |
| DR-RS10 | Cache eviction for zero-refcount pending-GC resources | architecture.md §"Resource lifecycle" |

---

## 1. Concepts and Definitions

### 1.1 Resource Types

The Resource Store manages three asset classes:

| Type | Description | Content-Addressed | Durable |
|------|-------------|-------------------|---------|
| **Texture** | Decoded pixel data for `StaticImageNode`. Input: PNG/JPEG/WebP/AVIF bytes. Stored as decoded RGBA8 or compressed GPU texture (BC7 on supported platforms). | Yes | Optional — see §7.1 |
| **Font** | Font face loaded into the compositor's text engine. Source: system font paths, bundled font files, or agent-uploaded TTF/OTF bytes. | Yes | Yes — system fonts always present; bundled fonts included in binary |
| **Buffer** | Raw opaque bytes for future node types (post-v1 use). Content-addressed; no MIME validation. | Yes | No — not loaded by v1 node types |

V1 implements `Texture` and `Font` fully. `Buffer` is defined for completeness but no v1 node type references it; upload is accepted but the resource is inert until a post-v1 node type consumes it.

### 1.2 ResourceId

`ResourceId` is a BLAKE3 content hash of the raw input bytes (before any decode or transcoding):

```rust
/// Immutable resource ID — BLAKE3 hex digest of raw input bytes (32 bytes, 64 hex chars).
/// Computed before decode. Two uploads of identical bytes produce the same ResourceId.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(String); // 64 hex chars

impl ResourceId {
    /// Compute the ResourceId for a byte slice.
    pub fn from_bytes(data: &[u8]) -> Self {
        ResourceId(blake3::hash(data).to_hex().to_string())
    }
}
```

**Immutability guarantee:** A `ResourceId` identifies a fixed, immutable byte sequence. The resource store never mutates the content of an existing resource. Uploading different bytes to an existing `ResourceId` is rejected with `RESOURCE_CONFLICT`.

### 1.3 Reference Count

Every resource has a **reference count** (`refcount`) — the number of scene graph nodes currently referencing it. The refcount drives eviction:

- `refcount > 0`: Resource is **live**. It must not be evicted.
- `refcount == 0`: Resource is **pending-GC**. It may be evicted after a grace period (§5.2).

Refcount is an in-memory integer. It is not stored on disk. On restart, the runtime reconstructs refcounts by scanning the durable blob store against agent-uploaded resource registries.

### 1.4 Namespace Ownership

A resource is uploaded by one **uploader namespace** but may be referenced by any namespace with the `READ_RESOURCE` capability (§4.1). The uploader is recorded for accounting and revocation purposes, but content-addressed identity is global — two uploads of the same bytes, regardless of which namespace uploads them, resolve to the same `ResourceId` and the same stored blob.

---

## 2. Upload Protocol

### 2.1 Overview

Agents upload resources via a dedicated gRPC streaming RPC on the `ResourceService`. Uploads are separate from scene mutations (the `SceneMutationService`). A resource must be fully uploaded and acknowledged before any scene mutation references its `ResourceId`.

The upload flow:

```
Agent                                  Runtime
  │                                       │
  ├──UploadResource (header) ────────────►│
  │                                       │  1. Authenticate + capability check
  │                                       │  2. Compute ResourceId from bytes
  │                                       │  3. Check dedup (already stored?)
  ├──UploadResource (chunks...) ─────────►│  4. Receive chunks
  │                                       │  5. Validate (MIME, size, integrity)
  ├──UploadResource (final) ─────────────►│  6. Decode to internal format
  │                                       │  7. Store blob + register in index
  │◄──UploadResourceResponse ─────────────┤  8. Return ResourceId + storage metadata
```

**Fast path:** If the runtime detects a dedup hit — only possible if `declared_hash` is provided in the `ResourceUploadFinalizer` and matches an already-stored resource — it returns a success `ResourceUploadResponse` with `was_deduplicated = true` after the finalizer arrives. The server still receives all chunks before responding; gRPC client-streaming does not support server-side stream interruption. Received bytes are discarded after the hash match is confirmed.

### 2.2 Upload RPC (Protobuf)

```protobuf
syntax = "proto3";
package tze_hud.resource.v1;

// Agent uploads a resource in chunks. First message must be a ResourceUploadHeader.
// Subsequent messages are data chunks. Final message is a ResourceUploadFinalizer.
// The server returns a single ResourceUploadResponse once the upload completes.
rpc UploadResource(stream ResourceUploadRequest)
    returns (ResourceUploadResponse);

// Query resource metadata without downloading content.
rpc StatResource(StatResourceRequest)
    returns (StatResourceResponse);

// Download resource bytes (used by admin tools and debugging surfaces; not for agents).
// Requires READ_RESOURCE_CONTENT capability (not granted to standard resident agents).
rpc DownloadResource(DownloadResourceRequest)
    returns (stream DownloadResourceResponse);

// Acquire an explicit hold on an already-uploaded resource (see §3.4).
// Prevents GC between upload completion and node creation.
rpc AcquireResourceHold(AcquireResourceHoldRequest)
    returns (AcquireResourceHoldResponse);

// Release an explicit hold on a resource (see §3.4).
rpc ReleaseResourceHold(ReleaseResourceHoldRequest)
    returns (ReleaseResourceHoldResponse);
```

### 2.3 Upload Request Messages

```protobuf
message ResourceUploadHeader {
  ResourceType   resource_type  = 1;   // TEXTURE, FONT, or BUFFER
  string         declared_mime  = 2;   // e.g., "image/png", "font/ttf"; validated server-side
  uint64         declared_size  = 3;   // Total byte size; must match received bytes; 0 = unknown (allowed)
  string         display_name   = 4;   // Human-readable label for debug tooling; not used for identity
  bool           skip_if_exists = 5;   // If true and the ResourceId (hash) already exists, server responds
                                       // with UPLOAD_ERROR_ALREADY_EXISTS after receiving the finalizer
                                       // (hash must be provided in ResourceUploadFinalizer.declared_hash).
                                       // All bytes are still transmitted; the server checks identity only
                                       // after finalizer arrives. Use when the agent does not want an implicit
                                       // dedup-success response — it prefers an explicit error to detect
                                       // inadvertent double-upload.
}

message ResourceUploadChunk {
  bytes data = 1;   // Chunk of resource bytes; max chunk size: 1 MiB
}

message ResourceUploadFinalizer {
  bytes declared_hash = 1;   // BLAKE3 hash of all bytes, 32 bytes; server verifies
}

message ResourceUploadRequest {
  oneof payload {
    ResourceUploadHeader    header    = 1;
    ResourceUploadChunk     chunk     = 2;
    ResourceUploadFinalizer finalizer = 3;
  }
}

enum ResourceType {
  RESOURCE_TYPE_UNSPECIFIED = 0;
  RESOURCE_TYPE_TEXTURE     = 1;
  RESOURCE_TYPE_FONT        = 2;
  RESOURCE_TYPE_BUFFER      = 3;
}
```

### 2.4 Upload Response Messages

```protobuf
message ResourceUploadResponse {
  ResourceId resource_id  = 1;   // Computed BLAKE3 hash; matches declared_hash if provided
  bool       was_deduplicated = 2;   // True if resource already existed; no new storage consumed
  uint64     stored_bytes    = 3;   // Bytes actually stored (0 if deduplicated)
  uint64     decoded_bytes   = 4;   // In-memory size after decode (GPU texture memory for textures)
}

message StatResourceRequest {
  ResourceId resource_id = 1;
}

message StatResourceResponse {
  ResourceId   resource_id    = 1;
  ResourceType resource_type  = 2;
  uint64       stored_bytes   = 3;    // Raw bytes on disk
  uint64       decoded_bytes  = 4;    // In-memory decoded bytes (0 if not currently decoded)
  uint32       refcount       = 5;    // Current reference count
  string       uploader_namespace = 6;
  bool         is_durable     = 7;    // Survives restart
  bool         is_gpu_resident = 8;   // Currently loaded in GPU texture memory
}
```

### 2.5 Upload Validation

Before storing, the runtime validates:

1. **MIME type**: must be an allowed type for the declared `ResourceType` (see §2.6).
2. **Size**: total bytes must not exceed the per-resource size limit (see §6.1).
3. **Session budget**: the upload must not exceed the agent's `texture_bytes_total` budget (RFC 0008 §4.1). This check happens before decode.
4. **Hash integrity**: if `declared_hash` is provided in the finalizer, computed BLAKE3 must match. Mismatch → `RESOURCE_HASH_MISMATCH` error.
5. **Decode validation**: for textures, the image must decode successfully. For fonts, the font face must parse and load without error. Corrupt data → `RESOURCE_DECODE_ERROR`.

Upload errors:

```protobuf
enum UploadError {
  UPLOAD_ERROR_UNSPECIFIED          = 0;
  UPLOAD_ERROR_CAPABILITY_DENIED    = 1;   // Agent lacks UPLOAD_RESOURCE capability
  UPLOAD_ERROR_BUDGET_EXCEEDED      = 2;   // Would exceed texture_bytes_total budget
  UPLOAD_ERROR_SIZE_EXCEEDED        = 3;   // Single resource exceeds per-resource size limit
  UPLOAD_ERROR_UNSUPPORTED_MIME     = 4;   // MIME type not accepted for this ResourceType
  UPLOAD_ERROR_DECODE_ERROR         = 5;   // Image/font decode failed
  UPLOAD_ERROR_HASH_MISMATCH        = 6;   // Declared hash does not match received bytes
  UPLOAD_ERROR_RESOURCE_CONFLICT    = 7;   // ResourceId exists with different content (impossible
                                           //   by construction; defensive error for corruption)
  UPLOAD_ERROR_SESSION_NOT_FOUND    = 8;   // Session expired between upload start and completion
  UPLOAD_ERROR_DURABLE_QUOTA_EXCEEDED = 9;   // durable=true but max_blob_store_mib reached
  UPLOAD_ERROR_OVERSIZED_TEXTURE_PIXELS = 10;  // Exceeds 8192×8192 pixel cap
  UPLOAD_ERROR_ALREADY_EXISTS       = 11;  // skip_if_exists=true and resource already stored
}
```

### 2.6 Allowed MIME Types

| ResourceType | Accepted MIME Types |
|--------------|---------------------|
| `TEXTURE` | `image/png`, `image/jpeg`, `image/webp`, `image/avif` |
| `FONT` | `font/ttf`, `font/otf`, `font/woff2`, `application/font-sfnt` |
| `BUFFER` | Any; no MIME validation for buffer type |

**AVIF in v1:** AVIF is accepted but decode is best-effort; if the platform's image decoder does not support AVIF, the upload returns `UPLOAD_ERROR_DECODE_ERROR`. Agents should prefer PNG or JPEG for maximum v1 compatibility.

### 2.7 Upload Latency Budget

| Stage | Budget |
|-------|--------|
| Header acknowledgement | < 5ms from first byte received |
| Full dedup-hit response (after finalizer) | < 10ms |
| Full upload+store (after finalizer, non-dedup) | < 200ms for resources ≤ 1 MiB; proportionally larger for bigger resources |
| Decode validation (included in store) | ≤ 50ms for textures ≤ 4096×4096; ≤ 20ms for fonts ≤ 1 MiB |

These budgets are on the network thread, not the compositor thread. Uploads never block the compositor. The `UploadResource` RPC runs on a dedicated upload thread pool sized to at least `min(num_cpus, 4)` threads.

---

## 3. Reference Counting

### 3.1 Formal Refcount Rules

The resource store maintains an integer `refcount` for each stored resource. The rules:

| Event | Refcount Change | Notes |
|-------|-----------------|-------|
| `StaticImageNode` added to scene | +1 | Applies to any node type that embeds a `ResourceId` |
| `StaticImageNode` removed from scene | -1 | Node deletion, tile deletion, tab deletion, lease revocation |
| `StaticImageNode` replaced by mutation | old -1, new +1 | Atomic within the mutation batch |
| Upload completes (resource newly stored) | 0 | Newly uploaded resource starts at refcount 0 unless a node references it |
| `ReleaseResourceHold` called by agent | -1 | Explicit hold (see §3.4); only valid if agent holds a hold |
| Runtime restart | Reconstructed | See §3.5 |

**All refcount mutations are atomic.** They occur on the compositor thread during mutation commit (stage 4 of RFC 0002's frame pipeline). No refcount change happens on the upload thread or the network thread.

### 3.2 Zero-Refcount Resources

When a refcount reaches zero, the resource transitions to **pending-GC** state. Pending-GC resources:

- Remain accessible in the blob store (disk-backed durable resources remain on disk).
- Are eligible for eviction from memory (the GPU texture is unloaded; the blob may be removed after the grace period).
- Are **not** visible in `StatResource` responses' `is_gpu_resident` field (it returns false).
- Can still be re-uploaded or re-referenced before eviction, which increments refcount back above zero (resurrection, §5.3).

### 3.3 Refcount Consistency Invariants

The runtime enforces these invariants as correctness assertions (violation → compositor panic in debug builds, structured error log in release builds):

1. `refcount >= 0` at all times. Underflow is a bug.
2. A resource referenced by a live scene node always has `refcount >= 1`.
3. After a tile is deleted and its nodes freed, no node from that tile retains a reference.
4. After a session is revoked, all resources uploaded by that session with `refcount == 0` must be scheduled for GC within one GC cycle.

These invariants are verified by the `resource_lifecycle_consistency` test scene (see §9.1).

### 3.4 Explicit Holds

Agents may request an **explicit hold** on a resource via `ReleaseResourceHold`'s counterpart `AcquireResourceHold` (implicitly granted during upload — the uploader gets one hold). An explicit hold increments the refcount and prevents GC even if no scene nodes reference the resource.

Use case: an agent pre-uploads a resource it intends to use soon but has not yet created a node for. Without a hold, the resource could be GC'd between upload completion and node creation.

```protobuf
// An explicit hold is granted during upload (implicit) and can be released
// when the agent no longer needs pre-GC protection.
// Session identity is inferred from the authenticated gRPC connection; no session_id field needed.
message ReleaseResourceHoldRequest {
  ResourceId resource_id = 1;
}

message ReleaseResourceHoldResponse {
  uint32 refcount_after_release = 1;   // Informational; may be 0 (resource now pending-GC)
}
```

**Hold accounting:** Holds count against the agent's `texture_bytes_total` budget (RFC 0008 §4.1) for the duration they are held. When the session is revoked, all holds owned by that session are released atomically on the same frame tick as lease teardown.

**Hold limit:** An agent may hold at most `max_tiles * 4` explicit holds simultaneously (defaults to 32). This prevents hold accumulation as a budget bypass.

### 3.5 Refcount Reconstruction on Restart

On startup, the runtime does not persist refcounts. It reconstructs them by:

1. Loading the durable blob store index (§7.1).
2. Loading the scene graph from durable config (tabs, zone registry — RFC 0001 §6.1).
3. Walking all node definitions in the restored scene graph and incrementing refcounts for every referenced `ResourceId`.
4. Setting all other resources in the blob store to `refcount = 0` (pending-GC).

Resources with `refcount == 0` after reconstruction are eligible for eviction on the first GC pass (§5.2). This is correct behavior: resources that were only referenced by the ephemeral scene graph (which is lost on restart) are no longer needed.

---

## 4. Cross-Agent Sharing

### 4.1 Capability Requirements

| Operation | Required Capability |
|-----------|---------------------|
| Upload a resource | `UPLOAD_RESOURCE` (included in resident capability set by default) |
| Reference a self-uploaded resource | Implicit; no extra capability needed |
| Reference a resource uploaded by another agent | `READ_RESOURCE` (included in resident capability set by default) |
| Download resource bytes | `READ_RESOURCE_CONTENT` (not granted by default; debug/admin use only) |
| View resource metadata (stat) | `READ_RESOURCE` |

**Default behavior:** Resident agents may reference any stored resource by `ResourceId` regardless of which namespace uploaded it. This enables natural deduplication without coordination: two agents uploading the same image get the same `ResourceId` and can reference it freely.

**Guest agents** have no access to `ResourceService`. Guest MCP paths reference named content (zone content strings, inline image URIs) — they do not upload or reference `ResourceId` values directly.

### 4.2 Sharing Semantics

Cross-namespace resource sharing is **read-only and content-addressed**:

- Agent A uploads a PNG → `ResourceId(abc123)`.
- Agent B uploads the same PNG → server returns `ResourceId(abc123)` (dedup hit, `was_deduplicated = true`).
- Agent B creates a `StaticImageNode` with `resource_id = abc123` → refcount +1.
- Agent A's session is revoked → Agent A's hold on `abc123` is released → refcount -1. But Agent B's node still holds a reference → `refcount >= 1` → resource is **not** evicted.

**Security model:** Content-addressed sharing does not leak agent identity. Agent B knows the `ResourceId` of a resource but learns nothing about who uploaded it (the uploader namespace is not exposed via the scene graph, only via `StatResource` which requires `READ_RESOURCE`). The content itself is identical by construction — no agent-specific information is embedded in the resource by the runtime.

### 4.3 Accounting Attribution

Texture bytes in use count against the session that holds the **reference** (via a live node or an explicit hold), not the session that uploaded the resource. This prevents a denial-of-service where Agent A uploads a huge resource and Agent B (unknowingly) references it, pushing Agent A over budget.

Formally: when a node is committed to the scene, the scene mutation pipeline (RFC 0001 §4) checks the `texture_bytes_total` budget of the **mutating agent's lease** (RFC 0008 §4.2). The texture bytes of the referenced resource are added to that agent's consumption counter, not the uploader's.

---

## 5. Garbage Collection

### 5.1 GC Runs on the Compositor Thread

All GC operations run on the compositor thread during a dedicated **resource-GC phase** that runs as a compositor-thread epilogue after stage 7 (GPU Submit + Present) completes, before the compositor thread begins stage 3 of the next frame (RFC 0002 §3.2). This places GC in the inter-frame idle window on the compositor thread. The GC phase has a time budget of **2ms per frame**. If the GC backlog requires more than 2ms, it is deferred to the next frame.

**GC must not run during frame render** (RFC 0002 stages 4–7). It must not acquire any lock held by the render pipeline. It must not touch GPU device state. Memory deallocation (freeing decoded textures, removing blob store entries) runs in the GC phase only.

### 5.2 GC Grace Period

A resource transitions to **pending-GC** when `refcount == 0`. It is not immediately freed. A configurable grace period (default: **5 seconds**) must elapse before the resource is evicted from memory.

**Rationale:** Agents frequently delete and recreate nodes as they update content. Immediate GC on refcount-0 would cause repeated upload-decode-free-upload cycles for resources the agent will reference again in the next few frames. The grace period acts as a soft LRU cache for recently-dereferenced resources.

Grace period timing uses wall clock time (`Instant::now()`), not frame count, so it is not affected by compositor pause states.

### 5.3 Resurrection

A resource that is pending-GC (refcount == 0, grace period not yet elapsed) can be **resurrected** by a new reference:

- A scene mutation creates a node referencing a pending-GC resource.
- The mutation pipeline increments the refcount (+1).
- The resource transitions from pending-GC back to live.
- Its decoded in-memory form (if still resident) does not need to be reloaded.

If the resource's grace period has already elapsed and its decoded form has been evicted from memory (but the blob store entry is intact), resurrection requires a re-decode from the stored blob. The mutation pipeline **must not** perform this re-decode synchronously on the compositor thread — blob reads and image decoding can take up to 50ms (§2.7), which would shatter the 1ms Stage 4 budget (RFC 0002 §3.2).

Instead, the mutation is rejected with `RESOURCE_NOT_RESIDENT` (a retriable error). The runtime simultaneously enqueues a re-decode job on the upload thread pool. Once the resource is GPU-resident again, the runtime emits a `ResourceResidentEvent` notification on the agent's event stream. The agent may then resubmit the mutation. The re-decode is idempotent and the blob store entry remains intact throughout.

**Resurrection does not apply to resources whose blob store entry has been fully removed.** A fully-removed resource requires a new upload.

### 5.4 Eviction Tiers

Resources are evicted in two tiers:

| Tier | What Is Freed | Trigger |
|------|---------------|---------|
| **GPU eviction** | Decoded in-memory / GPU texture data; blob store entry retained | refcount == 0 for ≥ grace period (5s default), OR compositor memory pressure (§5.5) |
| **Blob eviction** | Disk-backed blob store entry removed | GPU-evicted AND not durable (§7.1), OR explicit purge (admin only) |

Durable resources (§7.1) are never blob-evicted automatically. They survive indefinitely on disk until explicitly purged by an admin operation.

Non-durable resources (raw uploads not marked durable) are blob-evicted after GPU eviction completes and the resource remains at refcount == 0.

### 5.5 Memory Pressure Eviction

When total GPU texture memory across all resources exceeds a configurable **memory pressure threshold** (default: 80% of `max_texture_bytes` system-wide, see §6.2), the GC phase performs emergency eviction:

1. Sort pending-GC resources by `time_since_last_dereferenced` descending (oldest first).
2. GPU-evict until total drops below 60% of the memory pressure threshold (20% hysteresis band).
3. If pending-GC resources are exhausted before reaching the target, emit `RESOURCE_MEMORY_PRESSURE` telemetry event. No live resources are evicted.
4. If the system cannot reduce below the threshold after 3 consecutive frames of emergency eviction, emit a structured warning log and increase eviction aggressiveness (reduce grace period to 0).

Memory pressure eviction never removes live resources (refcount > 0). If live resources alone fill GPU memory beyond the hard max, budget enforcement at the mutation pipeline level (RFC 0008 §4.2) should have prevented this — a breach here indicates a bug in budget accounting and is logged as a correctness error.

### 5.6 GC Latency Budget

| Operation | Budget |
|-----------|--------|
| Refcount decrement (per node, on mutation commit) | < 1μs |
| Pending-GC marking (transition to pending-GC) | < 1μs |
| GPU eviction per resource (decoded bytes freed) | < 500μs |
| Blob eviction per resource (disk removal) | < 5ms (deferred to async IO thread if exceeded) |
| Emergency eviction pass | ≤ 2ms total (per-frame GC phase budget) |

Blob eviction is performed on an async IO thread if the individual removal would exceed 5ms (large blobs on slow storage). The resource is marked `blob_eviction_pending` atomically before the async IO starts; any resurrection attempt during this window waits on the eviction completion channel.

---

## 6. Size Limits and Budgets

### 6.1 Per-Resource Limits

| Resource Type | Max Input Size | Max Decoded In-Memory Size |
|---------------|---------------|---------------------------|
| Texture (PNG/JPEG/WebP) | 16 MiB | 64 MiB (e.g., 4096×4096×4 bytes RGBA8) |
| Texture (AVIF) | 16 MiB | 64 MiB |
| Font (TTF/OTF/WOFF2) | 4 MiB | 16 MiB (includes hinted glyph cache at default sizes) |
| Buffer | 64 MiB | 64 MiB (no decode; stored as-is) |

The decoded in-memory limit exists to prevent a small compressed input from expanding into an unreasonably large texture (e.g., a heavily compressed PNG of a solid color could trivially produce a 4K+ texture). Input that would exceed the decoded limit is rejected at validation time with `UPLOAD_ERROR_SIZE_EXCEEDED`, even if the raw bytes are within limits.

**Texture resolution cap:** The compositor rejects textures with either dimension exceeding 8192 pixels. Textures with either dimension exceeding 4096 pixels emit a `RESOURCE_OVERSIZED_TEXTURE` warning event but are accepted.

### 6.2 System-Wide Memory Budget

The Resource Store observes a system-wide texture memory budget, configured in `[resources]` (§8.1):

```toml
[resources]
max_texture_memory_mib = 1024   # Total GPU texture memory for all resources; default: 1024 MiB
max_font_memory_mib    = 64     # Font cache memory limit; separate from texture budget
gc_grace_period_ms     = 5000   # Pending-GC grace period before GPU eviction; default: 5000ms
blob_store_path        = "~/.local/share/tze_hud/resources"  # Durable blob store location
max_blob_store_mib     = 4096   # Maximum disk usage for durable blob store; default: 4096 MiB
```

Per-agent texture budgets (RFC 0008 §4.1 `texture_bytes_total`) are enforced within the system-wide budget. The system-wide budget is the hard ceiling; individual agent budgets are sub-allocations within it.

### 6.3 Budget Interaction with RFC 0008

RFC 0008 §4.1 defines `ResourceBudget.texture_bytes_total` as "aggregate texture bytes across all lease tiles." This RFC clarifies how that budget accounts for resources:

- A resource's **decoded in-memory size** (not raw upload size) counts against `texture_bytes_total`.
- A resource with `refcount > 0` counts for each agent that holds a reference, using the decoded size. This means two agents sharing a resource each bear its full decoded cost against their own budgets.
- A resource that has been GPU-evicted (decoded bytes freed) but still has `refcount > 0` from pending nodes counts as 0 bytes against budgets until re-decoded (resurrection, §5.3). Re-decode adds the bytes back to the referencing agent's budget counter.
- Explicit holds (§3.4) count at the full decoded size.

**Rationale for per-agent double-counting:** Shared resources are cheaper to store (stored once on disk, decoded once in GPU memory) but must still be accounted against each agent's budget to prevent a coordinated multi-agent attack where agents collectively consume more texture memory than any one of them is permitted.

---

## 7. Persistence

### 7.1 Durable Resources

Resources can be marked **durable** at upload time (not by default). Durable resources:

- Are written to the blob store (filesystem) at `blob_store_path` (§8.1).
- Survive runtime restarts.
- Are never blob-evicted automatically (only by explicit admin purge).
- Restore their `refcount` to the correct value via scan at startup (§3.5).

Non-durable resources are in-memory only. If the runtime restarts, non-durable resources must be re-uploaded by agents. This is consistent with the scene graph being ephemeral (RFC 0001 §6.2) — agents re-establish scene state on reconnect, which includes re-uploading resources referenced by their tiles.

**Marking a resource durable:** Set `durable = true` in the `ResourceUploadHeader` (see §2.3, `durable` field added in extended proto below).

```protobuf
// Extended ResourceUploadHeader with durability flag.
// Field 6 added to ResourceUploadHeader message (§2.3).
message ResourceUploadHeader {
  // ... existing fields 1-5 ...
  bool durable = 6;   // If true, resource is written to blob store and survives restart. Default: false.
}
```

**Durable resource quota:** Durable resources count against `max_blob_store_mib` (§8.1). When this limit is reached, upload requests with `durable = true` are rejected with `UPLOAD_ERROR_DURABLE_QUOTA_EXCEEDED`.

### 7.2 Blob Store Format

The blob store is a flat-file store rooted at `blob_store_path`:

```
blob_store/
├── index.json           — JSON manifest: ResourceId → metadata (type, size, MIME, uploader, timestamp)
├── data/
│   ├── ab/              — First two hex chars of ResourceId (shard directory)
│   │   └── abcdef...    — Full ResourceId hex string as filename; raw input bytes
│   └── ...
└── tmp/                 — In-progress upload staging area; files here are not in index
```

The index is written atomically (write to `index.json.tmp`, then `rename`) on every resource add or remove. The `tmp/` directory is cleaned on startup to remove any aborted uploads.

**File format:** Raw input bytes. No wrapping envelope. The `ResourceId` is the filename and the ground truth of content identity. A corrupt blob (BLAKE3 mismatch on verify) is treated as missing.

**On-disk verification:** The runtime runs a background integrity check on startup that computes BLAKE3 hashes of all blobs and cross-references with `index.json`. Mismatches are logged as errors and the corrupt resource is removed from the index. Verification runs asynchronously and does not delay compositor startup.

### 7.3 Cleanup Policy

On startup, after reconstruction (§3.5):

1. Any resource in the blob store with `refcount == 0` that is **not durable** is removed immediately (these are stale non-durable leftovers from an abnormal shutdown).
2. Durable resources with `refcount == 0` enter pending-GC state (grace period applies; they are not removed automatically).
3. Orphaned files in `tmp/` are deleted.

---

## 8. Font Asset Lifecycle

### 8.1 Font Sources

Fonts enter the resource store from three sources:

| Source | ResourceId | Durable | Example |
|--------|------------|---------|---------|
| **System fonts** | Computed from file bytes at load time | Implicit (system managed) | `/usr/share/fonts/...` |
| **Bundled fonts** | Compiled into binary; computed from bytes | Implicit (always present) | Noto Sans, monospace fallback |
| **Agent-uploaded fonts** | Computed from upload bytes | Configurable (§7.1) | Custom display fonts |

System and bundled fonts are available to all agents without upload. Their `ResourceId` values are stable across sessions (same binary + same system font installation).

### 8.2 FontFamily Resolution

RFC 0001 §2.3 defines `FontFamily` with named variants (`Monospace`, `SansSerif`, `Serif`, `Display`, `Custom`). The `Custom` variant includes a `ResourceId` referencing an uploaded or system font.

Resolution order for `TextMarkdownNode.font_family`:

1. If `Custom(resource_id)`: look up `resource_id` in the resource store. If found and decoded: use it. If not found: fall back to `SansSerif`.
2. Named variants (`Monospace`, `SansSerif`, `Serif`, `Display`): resolved against the font resolution table (§8.3).
3. If resolution fails: use bundled default (Noto Sans for SansSerif/Serif; bundled monospace for Monospace).

**Fallback is transparent.** Agents are not notified when a font fallback occurs. Telemetry tracks font fallback events per frame for debugging.

### 8.3 Font Resolution Table

Configured in `[resources.fonts]` (see §8.5). The resolution table maps named `FontFamily` variants to font face `ResourceId` values, allowing the display node's administrator to override default fonts:

```toml
[resources.fonts]
sans_serif    = "system:Noto Sans"        # "system:<name>" = system font lookup by name
monospace     = "system:JetBrains Mono"
serif         = "system:Noto Serif"
display       = "bundled:default_display" # "bundled:<name>" = bundled font
```

If a configured font is not found, the bundled default is used without error.

### 8.4 Font Cache

Fonts are loaded into the compositor's text engine (fontdue or similar) and cached in a per-font-face glyph cache. The cache is bounded by `max_font_memory_mib` (§6.2).

Font cache eviction is separate from texture GC:

- Font glyph caches are evicted LRU when the font memory budget is exceeded.
- A font face with `refcount == 0` is eligible for full eviction (glyph cache + face object).
- System and bundled fonts are never fully evicted (they have implicit permanent holds).

**Font rendering is always on the compositor thread.** Font layout and rasterization happen in stage 5 (Layout Resolve) of the frame pipeline (RFC 0002). Font cache mutations are not thread-safe; all font cache access is serialized by the compositor thread.

### 8.5 Font Configuration

```toml
[resources.fonts]
sans_serif    = "system:Noto Sans"
monospace     = "system:monospace"
serif         = "system:serif"
display       = "bundled:default_display"
min_size_px   = 8.0    # Minimum rendered font size (security: prevents invisible text attacks)
max_size_px   = 256.0  # Maximum rendered font size
```

---

## 9. Protobuf Schema Summary

All messages in package `tze_hud.resource.v1`.

```protobuf
syntax = "proto3";
package tze_hud.resource.v1;

import "scene.proto";  // For ResourceId

// ─── Service ───────────────────────────────────────────────────────────────

service ResourceService {
  rpc UploadResource(stream ResourceUploadRequest)
      returns (ResourceUploadResponse);
  rpc StatResource(StatResourceRequest)
      returns (StatResourceResponse);
  rpc DownloadResource(DownloadResourceRequest)
      returns (stream DownloadResourceResponse);
  rpc AcquireResourceHold(AcquireResourceHoldRequest)
      returns (AcquireResourceHoldResponse);
  rpc ReleaseResourceHold(ReleaseResourceHoldRequest)
      returns (ReleaseResourceHoldResponse);
}

// ─── Upload ─────────────────────────────────────────────────────────────────

enum ResourceType {
  RESOURCE_TYPE_UNSPECIFIED = 0;
  RESOURCE_TYPE_TEXTURE     = 1;
  RESOURCE_TYPE_FONT        = 2;
  RESOURCE_TYPE_BUFFER      = 3;
}

enum UploadError {
  UPLOAD_ERROR_UNSPECIFIED               = 0;
  UPLOAD_ERROR_CAPABILITY_DENIED         = 1;
  UPLOAD_ERROR_BUDGET_EXCEEDED           = 2;
  UPLOAD_ERROR_SIZE_EXCEEDED             = 3;
  UPLOAD_ERROR_UNSUPPORTED_MIME          = 4;
  UPLOAD_ERROR_DECODE_ERROR              = 5;
  UPLOAD_ERROR_HASH_MISMATCH             = 6;
  UPLOAD_ERROR_RESOURCE_CONFLICT         = 7;
  UPLOAD_ERROR_SESSION_NOT_FOUND         = 8;
  UPLOAD_ERROR_DURABLE_QUOTA_EXCEEDED    = 9;
  UPLOAD_ERROR_OVERSIZED_TEXTURE_PIXELS  = 10;  // Exceeds 8192×8192 pixel cap
  UPLOAD_ERROR_ALREADY_EXISTS            = 11;  // skip_if_exists=true and resource already stored
}

message ResourceUploadHeader {
  ResourceType resource_type  = 1;
  string       declared_mime  = 2;
  uint64       declared_size  = 3;
  string       display_name   = 4;
  bool         skip_if_exists = 5;
  bool         durable        = 6;
}

message ResourceUploadChunk {
  bytes data = 1;
}

message ResourceUploadFinalizer {
  bytes declared_hash = 1;  // 32 bytes; optional but recommended
}

message ResourceUploadRequest {
  oneof payload {
    ResourceUploadHeader    header    = 1;
    ResourceUploadChunk     chunk     = 2;
    ResourceUploadFinalizer finalizer = 3;
  }
}

message ResourceUploadResponse {
  // On success: resource_id is set; error is UNSPECIFIED.
  // On failure: error is set; resource_id may be empty.
  tze_hud.scene.v1.ResourceId resource_id       = 1;
  bool                         was_deduplicated  = 2;
  uint64                       stored_bytes      = 3;
  uint64                       decoded_bytes     = 4;
  UploadError                  error             = 5;
  string                       error_detail      = 6;  // Human-readable; not stable
}

// ─── Stat ───────────────────────────────────────────────────────────────────

message StatResourceRequest {
  tze_hud.scene.v1.ResourceId resource_id = 1;
}

message StatResourceResponse {
  tze_hud.scene.v1.ResourceId resource_id          = 1;
  ResourceType                 resource_type        = 2;
  uint64                       stored_bytes         = 3;
  uint64                       decoded_bytes        = 4;
  uint32                       refcount             = 5;
  string                       uploader_namespace   = 6;
  bool                         is_durable           = 7;
  bool                         is_gpu_resident      = 8;
  bool                         is_pending_gc        = 9;
  uint64                       uploaded_at_us       = 10;  // UTC μs
}

// ─── Download (admin / debug only) ──────────────────────────────────────────

message DownloadResourceRequest {
  tze_hud.scene.v1.ResourceId resource_id = 1;
}

message DownloadResourceResponse {
  bytes  data         = 1;
  bool   is_last      = 2;
  uint64 total_bytes  = 3;  // Set on first chunk only; 0 on subsequent chunks
}

// ─── Holds ──────────────────────────────────────────────────────────────────

message AcquireResourceHoldRequest {
  tze_hud.scene.v1.ResourceId resource_id = 1;
}

message AcquireResourceHoldResponse {
  uint32 refcount_after = 1;
  bool   hold_granted   = 2;   // False if agent is at hold limit
}

message ReleaseResourceHoldRequest {
  tze_hud.scene.v1.ResourceId resource_id = 1;
}

message ReleaseResourceHoldResponse {
  uint32 refcount_after = 1;
}
```

---

## 10. Rust Module Overview

```
tze_resource/
├── lib.rs                — ResourceStore trait, ResourceId, ResourceType
├── store.rs              — ResourceStore implementation: in-memory index, refcount table
├── upload.rs             — Upload pipeline: receive, hash, validate, decode, store
├── gc.rs                 — GC phase: pending-GC tracking, eviction logic, grace period
├── font.rs               — FontRegistry: resolution table, system/bundled/uploaded fonts
├── blob/
│   ├── mod.rs            — BlobStore trait
│   └── filesystem.rs     — FilesystemBlobStore: shard-directory layout, index.json
├── budget.rs             — Budget integration with RFC 0008 AgentResourceState
└── proto/
    └── resource.proto    — (canonical schema from §9)
```

The `ResourceStore` is owned by the compositor thread. The `BlobStore` backend is accessed from both the compositor thread (GC phase) and the upload thread pool, guarded by an `Arc<Mutex<BlobStore>>` with a non-blocking try-lock in the GC phase (GC defers blob eviction if the lock is contended).

---

## 11. Telemetry

The `TelemetryRecord` (RFC 0002, Appendix A) is extended with a `ResourceStoreTelemetry` field:

```rust
pub struct ResourceStoreTelemetry {
    pub texture_bytes_live: u64,          // Total decoded GPU texture bytes (refcount > 0)
    pub texture_bytes_pending_gc: u64,    // Decoded bytes of pending-GC resources
    pub font_cache_bytes: u64,            // Total font glyph cache bytes
    pub blob_store_bytes: u64,            // Total durable blob store bytes on disk
    pub refcount_ops_this_frame: u32,     // Refcount increments + decrements this frame
    pub gc_resources_freed_this_frame: u32,  // Resources GC'd this frame
    pub upload_in_progress: u32,          // Active upload RPCs
    pub resurrection_count_total: u64,    // Cumulative resurrection count since startup
    pub font_fallback_count_this_frame: u32,  // Font resolution fallbacks this frame
}
```

---

## 12. Security Considerations

### 12.1 Content-Address Collision Resistance

BLAKE3 is collision-resistant for any practical input. Two different byte sequences producing the same `ResourceId` is computationally infeasible. The `RESOURCE_CONFLICT` error is a defensive assertion — it should never occur in practice; its presence in the error enum exists to handle storage corruption, not hash collisions.

### 12.2 Zip-Bomb / Decompression Attack

A small compressed PNG could expand to a very large decoded texture. The decoded-size limit (§6.1) and the pixel dimension cap (§6.1) defend against this. Decode is performed in the upload thread pool with a memory-capped arena allocator; a decompression that exceeds the decoded limit is aborted mid-decode with `UPLOAD_ERROR_SIZE_EXCEEDED`.

### 12.3 Cross-Agent Resource Exfiltration

Agent B can reference resources by `ResourceId` (a hash) without being able to enumerate stored resources. There is no "list all resources" RPC available to resident agents. `StatResource` requires knowing the `ResourceId` in advance; it does not enable discovery. The `DownloadResource` RPC requires `READ_RESOURCE_CONTENT` capability, which is not granted by default and is not accessible to standard resident agents.

### 12.4 Budget Bypass via Sharing

Two agents each holding a reference to the same resource are each charged the full decoded size against their budgets (§4.3). This prevents a coordinated pair of agents from collectively consuming more texture memory than permitted by using a shared resource.

---

## 13. Interaction with Other RFCs

| RFC | Interaction |
|-----|-------------|
| **RFC 0001 §1.1** | `ResourceId` definition (BLAKE3 hex) and `StaticImageNode.resource_id` — this RFC provides the upload and lifecycle contract |
| **RFC 0001 §6.1** | Durable state includes "uploaded resources" in blob store — this RFC specifies the blob store format and persistence rules |
| **RFC 0002 §5.2** | Resource cleanup on revocation ("free all agent-owned textures and node data") — this RFC specifies what "free" means: release all holds, decrement refcounts for all nodes, schedule pending-GC |
| **RFC 0008 §4.1** | `ResourceBudget.texture_bytes_total` — this RFC clarifies that decoded in-memory size (not raw bytes) counts against this budget, and that sharing causes per-agent double-counting |
| **RFC 0009** | Policy arbitration order — resource GC runs after the policy arbitration stack; resource eviction does not bypass any policy check |

---

## 14. Open Questions

1. **GPU texture compression.** Should the resource store transcode PNG/JPEG inputs to BC7/ASTC GPU-native compressed formats on upload? This reduces VRAM usage by 4–8x but adds transcoding latency and requires platform capability detection. Deferred to post-v1 profiling.

2. **Resource deduplication across restarts.** On restart, non-durable resources are lost. If an agent re-uploads the same resource after restart, the `ResourceId` is the same but it's a fresh upload. There is no risk of incorrect deduplication, but agents cannot assume a resource survives restart unless they mark it durable. This is the intended behavior; no change needed.

3. **Blob store replication.** For deployments with multiple display nodes, should durable resources be synced across nodes? This is a post-v1 concern; v1 is single-node.

4. **Font preloading.** Should the runtime preload all configured system fonts at startup, or lazy-load on first use? Lazy-load keeps startup time low but causes a first-use latency spike. Recommended: lazy-load with a configurable preload list in `[resources.fonts.preload]`.

5. **Buffer node types.** V1 accepts `BUFFER` uploads but no node type references them. The `BUFFER` ResourceType should be removed from the v1 upload RPC and added back in the post-v1 release that introduces the first buffer-consuming node type. Decision deferred to the implementors; the current spec includes it for forward compatibility but it is inert in v1.

---

## Appendix A: Performance Budget Summary

| Operation | Budget | Notes |
|-----------|--------|-------|
| Upload header ack | < 5ms | |
| Dedup-hit response | < 10ms after finalizer | |
| Full upload + store (≤ 1 MiB) | < 200ms | On upload thread pool |
| Texture decode (≤ 4096×4096) | ≤ 50ms | Included in upload store |
| Font decode (≤ 1 MiB) | ≤ 20ms | Included in upload store |
| Refcount change (per node) | < 1μs | Compositor thread, frame pipeline |
| GC phase per frame | ≤ 2ms | Appended to frame tick |
| Resurrection (blob-evicted) re-decode enqueue | < 1ms | Compositor thread rejects with RESOURCE_NOT_RESIDENT, enqueues re-decode on upload pool |
| Re-decode on upload pool (≤ 1 MiB blob) | ≤ 200ms | Same budget as fresh upload; agent retries mutation after ResourceResidentEvent |
| Blob eviction per resource | < 5ms synchronous; async if larger | Async IO thread for slow paths |

---

## Appendix B: Validation Scenes

These test scenes verify the resource lifecycle contract (referenced by architecture.md §"Resource lifecycle"):

| Scene Name | What It Tests |
|------------|---------------|
| `resource_upload_dedup` | Two agents upload identical bytes; verify `was_deduplicated = true`, one storage entry, two refcounts |
| `resource_lifecycle_consistency` | Upload → node create → node delete → verify refcount → pending-GC → wait grace → verify GPU eviction |
| `resource_resurrection` | Upload → create node → delete node (pending-GC) → create new node before grace period → verify resurrection, no re-upload needed |
| `resource_revocation_cleanup` | Agent uploads + references resource; agent revoked; verify post-revocation footprint == 0 within 100ms + 1 frame |
| `resource_budget_enforcement` | Agent uploads resources up to budget limit; next upload rejected with `UPLOAD_ERROR_BUDGET_EXCEEDED` |
| `resource_cross_agent_sharing` | Agent A uploads; Agent B references by ResourceId; Agent A revoked; verify resource stays live (Agent B still holds refcount) |
| `resource_memory_pressure` | Fill GPU texture budget to 85%; verify LRU eviction of pending-GC resources; verify live resources not evicted |
| `font_fallback_chain` | Configure invalid `sans_serif` font; verify fallback to bundled default; telemetry records `font_fallback_count` |
| `blob_store_integrity` | Corrupt a blob on disk; restart; verify corrupt resource removed from index and re-uploadable |
| `resource_leak_soak` | Repeated agent connect/upload/disconnect/reconnect over 1 hour; verify `texture_bytes_live` does not grow monotonically |
