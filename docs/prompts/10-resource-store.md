# Epic 10: Resource Store

> **Dependencies:** Epic 0 (ResourceStore trait contract), Epic 1 (ResourceId), Epic 6 (session upload stream)
> **Depended on by:** Epic 12 (integration tests with real assets)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md`
> **Secondary specs:** `scene-graph/spec.md` (ResourceId, StaticImageNode), `session-protocol/spec.md` (upload transport)

## Prompt

Create a `/beads-writer` epic for **resource store** — the content-addressed, immutable blob store for images, fonts, and other assets.

### Context

The resource store manages immutable resources identified by BLAKE3 content hash (raw 32-byte ResourceId). In v1, all resources are ephemeral — stored in memory, lost on restart. The existing crate structure doesn't have a dedicated resource store module. Epic 0 provides the `ResourceStore` trait contract.

### Epic structure

Create an epic with **4 implementation beads**:

#### 1. Content-addressed upload and deduplication (depends on Epic 0 ResourceStore contract, Epic 1 ResourceId)
Implement upload and dedup per `resource-store/spec.md` Requirement: Upload Protocol, Requirement: Content-Addressed Deduplication.
- Upload: agent sends raw bytes over session stream; runtime computes BLAKE3 hash → ResourceId
- If ResourceId already exists: return existing reference (no re-upload)
- Upload validation: size limits, v1 type whitelist (IMAGE_RGBA8, IMAGE_PNG, IMAGE_JPEG, FONT_TTF, FONT_OTF)
- ResourceId is the 32-byte raw BLAKE3 digest, NOT hex-encoded
- **Acceptance:** ResourceStore trait tests from Epic 0 pass. Duplicate uploads return same ResourceId. Invalid types rejected. Size limits enforced.
- **Spec refs:** `resource-store/spec.md` Requirement: Upload Protocol, Requirement: Content-Addressed Deduplication, lines 5-16

#### 2. Reference counting and GC (depends on #1)
Implement lifecycle management per `resource-store/spec.md` Requirement: Reference Counting, Requirement: Garbage Collection.
- Refcount incremented when resource referenced by a scene node; decremented when node removed
- GC: resources with refcount 0 after grace period are freed
- Resurrection: if resource is re-referenced during grace period, GC cancelled
- Per-agent budget: decoded size (not compressed) — shared resources double-counted
- **Acceptance:** Refcount tracks correctly across add/remove. GC fires after grace period. Resurrection prevents premature collection. Double-counting enforced for shared resources.
- **Spec refs:** `resource-store/spec.md` Requirement: Reference Counting, Requirement: Garbage Collection

#### 3. Cross-agent sharing (depends on #1, #2)
Implement sharing semantics per `resource-store/spec.md` Requirement: Cross-Agent Sharing.
- Resources are namespace-agnostic: any agent can reference any ResourceId
- Read-access is default; no capability required to use a shared resource
- Each referencing agent's budget is charged independently (double-counting)
- **Acceptance:** Agent A uploads resource; Agent B references same ResourceId successfully. Both agents charged independently. Removal by one agent doesn't affect other's reference.
- **Spec refs:** `resource-store/spec.md` Requirement: Cross-Agent Sharing

#### 4. V1 ephemerality contract (depends on #1)
Enforce v1 storage semantics per `resource-store/spec.md` Requirement: V1 Ephemerality.
- All resources stored in memory only — lost on runtime restart
- No persistence, no disk caching, no restore-from-disk in v1
- Scene snapshots reference ResourceIds but don't guarantee backing data survives
- Operator/debug representation: hex-encoded string for logs/CLI (not the wire format)
- **Acceptance:** Resources lost after simulated restart. No filesystem artifacts created. Snapshot contains ResourceId references but not blob data. Debug repr is lowercase hex.
- **Spec refs:** `resource-store/spec.md` Requirement: V1 Ephemerality, `scene-graph/spec.md` (v1 ephemerality alignment)

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `resource-store/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference exact spec scenarios
3. **Acceptance criteria** — which Epic 0 ResourceStore tests must pass
4. **Crate/file location** — new `crates/tze_hud_resource/` or module in scene crate
5. **Wire vs debug format** — ResourceId is raw 32 bytes on wire, hex for logging

### Dependency chain

```
Epics 0+1+6 ──→ #1 Upload/Dedup ──→ #2 Refcount/GC ──→ #3 Cross-Agent Sharing
                                 ──→ #4 V1 Ephemerality
```
