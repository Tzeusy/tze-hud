# RFC 0011 Session Resource Upload Direction Report

Date: 2026-04-10
Scope: `/project-direction` package for the resident scene-resource upload seam
Status: Draft direction artifact plus OpenSpec delta at `openspec/changes/session-resource-upload-rfc0011/`; tracker materialization pending

## Executive summary

[Observed] The real direction here is not "add an upload helper." It is to restore a broken v1 contract chain for resident presence. Doctrine says one resident gRPC session stream owns the hot path, the resource-store spec says scene resources SHALL ingress on that stream, and exemplar specs already depend on that path. But the checked-in session schema and server only expose widget asset registration, not scene-resource upload. See [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L145), [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L50), and [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto#L31).

[Observed] This is not just code-behind-spec drift. It is a three-way contract seam: RFC 0011 describes `ResourceUploadStart`/`Chunk`/`Complete` and `ResourceStored`, but the main session-protocol spec and checked-in `session.proto` only allocate `WidgetAssetRegister`/`WidgetPublish`; the runtime implementation only dispatches widget asset registration; and the resident helper/doc surfaces have already had to paper over the gap. See [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L220), [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L230), [session_server.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/src/session_server.rs#L1893), and [docs/exemplar-presence-card-user-test.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-user-test.md#L39).

[Observed] The highest-priority next work is therefore spec-first contract repair, not implementation heroics. The repo should first reconcile the missing upload handshake, error shape, capability authority, and envelope allocation across RFC 0005, RFC 0011, the v1 main specs, and `session.proto`; only then should it wire the server/runtime path and flip resident exemplar/user-test consumers over to the real upload flow. Anything else keeps v1 pretending that resident raw-tile uploads exist when they do not.

## Project Spirit

**Core problem**: Restore the resident hot-path contract so scene-node images and fonts can actually enter the HUD over the single session stream the project claims to use.
**Primary user**: Internal runtime/protocol developers and operators validating resident raw-tile exemplars.
**Success looks like**: Resident agents can upload scene resources over `HudSession`, the checked-in protobuf/schema/specs agree on that flow, upload failures are correlated and testable, and exemplar/user-test paths stop relying on placeholders.
**Trying to be**: A coherent v1 repair of the resident resource-ingress contract across doctrine, RFCs, main specs, protobuf, runtime, and validation surfaces.
**Not trying to be**: A new upload RPC, a widget-only workaround, or a speculative general asset framework beyond v1 scene resources and existing widget asset registration.

### Requirements

| # | Requirement | Class | Evidence | Status |
|---|------------|-------|---------|--------|
| 1 | Resident hot-path behavior SHALL ride one primary bidirectional session stream | Hard | [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L145), [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L20) | Partial |
| 2 | Scene-node image resources SHALL ingress via `ResourceUploadStart`/`Chunk`/`Complete` on that stream | Hard | [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L50), [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md#L27) | Unmet |
| 3 | Font upload semantics, if retained in this seam, SHALL be limited to scene-node/tile-local text styles and SHALL NOT override runtime-owned zone/component typography | Hard | [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L208), [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L279) | Unmet |
| 4 | Widget SVG asset registration SHALL remain a separate metadata-first path from scene-resource upload | Hard | [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L758), [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md#L38) | Met |
| 5 | Uploading a scene resource SHALL require `upload_resource` capability | Hard | [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L202) | Partial |
| 6 | Chunked uploads SHALL have a coherent correlation/acknowledgement path before clients can send chunks | Hard | [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L191), [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L702) | Unmet |
| 7 | Upload-specific failures SHALL align with the shared session error model rather than becoming an unstructured exception path | Hard | [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L382), [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L257), [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L702) | Unmet |
| 8 | Budget accounting for scene resources SHALL be enforced at node-reference time, not upload-storage time | Hard | [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L734), [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L297) | Unmet |
| 9 | Exemplar and `/user-test` resident flows that claim avatar/icon upload SHALL use the real session-stream contract, not placeholders | Hard | [exemplar-presence-card/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md#L139), [exemplar-dashboard-tile/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md#L58) | Unmet |
| 10 | Spec and implementation claims SHALL not overstate resident upload support | Hard | [validation.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/validation.md#L3) | Unmet |
| 11 | No separate upload RPC/service should be introduced for this v1 repair | Non-goal | [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L50), [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L145) | N/A |
| 12 | `ResourceQuery`/`ResourceQueryResult` SHALL remain explicitly deferred unless the main v1 specs are expanded | Non-goal | [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L528) | N/A |

### Contradictions

[Observed] The resource-store spec says scene resources SHALL use `ResourceUploadStart`/`Chunk`/`Complete` on the session stream, but the session-protocol spec allocates no such messages in the v1 envelope and instead stops at `WidgetAssetRegister` and `WidgetPublish`. See [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L50) versus [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L230).

[Observed] The checked-in protobuf matches the narrower session-protocol spec, not the resource-store spec or RFC 0011. `ClientMessage` exposes fields 34 and 35 for widget flows only, and `ServerMessage` exposes no scene-resource upload responses. See [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto#L31) and [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto#L94).

[Observed] RFC 0011 itself has an unresolved handshake gap: it says the runtime allocates `upload_id` on `ResourceUploadStart` acceptance and later defines `ResourceErrorResponse(upload_id=3)`, but the excerpted upload message set does not define a server acknowledgement that returns `upload_id` before chunks are sent. See [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L191), [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L231), and [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L702). This is a contract bug, not just missing code.

[Observed] RFC 0005 itself is part of the seam, not just background context. Its older combined-envelope field registry diverges from the checked-in split `ClientMessage`/`ServerMessage` model used by the main v1 spec and `session.proto`. Chunk 1 therefore has to declare which registry is authoritative for this repair and then reconcile the stale law-and-lore sections accordingly. See [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0005-session-protocol.md#L281), [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0005-session-protocol.md#L1448), and [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L229).

[Observed] The upload-specific error shape is also inconsistent today. The session error model expects stable code, message, context, and hint semantics, while RFC 0011's `ResourceErrorResponse` carries only `error_code`, `error_detail`, and optional `upload_id`. The seam fix must either align `ResourceErrorResponse` to that structured model or explicitly reject it and use `RuntimeError`; leaving it ambiguous will recreate drift. See [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L382), [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L257), and [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L702).

## Phase 1 Decisions

[Inferred] Phase 2 SHALL treat the split `ClientMessage`/`ServerMessage` model in the main v1 session-protocol spec and the checked-in `session.proto` as the authoritative resident envelope for this seam. RFC 0005 sections that still describe a combined `SessionMessage` registry are treated as stale law-and-lore sections that Chunk 1 must reconcile. See [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L229), [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto#L31), and [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0005-session-protocol.md#L281).

[Inferred] The chunked-upload handshake fix is a dedicated `ResourceUploadAccepted` server response for non-inline, non-deduplicated starts. It carries `request_sequence`, `upload_id`, and acceptance semantics before any chunks are sent. Deduplicated or inline starts may still short-circuit directly to `ResourceStored`.

[Inferred] Upload-specific failures SHALL keep a dedicated `ResourceErrorResponse`, and this direction package explicitly rejects a bare `RuntimeError`-only upload surface. `ResourceErrorResponse` must instead be upgraded to the shared structured session error model: stable error code, message, context, hint, initiating `request_sequence`, and optional `upload_id`. This preserves resource-domain correlation while keeping one doctrine-level error shape.

[Observed] This seam repair restores resident scene-resource ingress only. It does not change the v1 persistence split: scene resources remain ephemeral/in-memory, while widget SVG assets remain the durable registration path. See [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L297) and [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L264).

[Observed] Upload rate limiting and stream backpressure are already in main-spec scope and are therefore part of this seam repair. Only `ResourceQuery`/`ResourceQueryResult` remains explicitly deferred. See [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L331) and [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L270).

[Inferred] Budget enforcement for scene resources should be reconciled to reference time, not upload-storage time. This follows the architecture rule that scene-node resources are owned and accounted by the nodes that reference them and RFC 0011's accounting rules that charge decoded size when a mutation references a `ResourceId`. Chunk 1 must explicitly fix the contradictory upload-time budget language in the current main resource-store spec. See [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L297), [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L734), and [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L91).

[Inferred] Font upload remains in scope only for scene-node/tile-local text styles, and this direction package explicitly rejects any interpretation that would let agent-uploaded fonts override runtime-owned zone/component typography. The first consumer tranche remains image-driven, and live consumer conversion in this tranche does not depend on proving agent-uploaded fonts. See [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md#L27), [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L208), and [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L279).

## Current State

| Dimension | Status | Summary | Key Evidence |
|-----------|--------|---------|-------------|
| Spec adherence | Weak | Main specs and RFCs disagree on resident resource ingress | [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L50) |
| Core workflows | Adequate | Widget asset registration works on the stream, but scene-resource upload does not exist there | [session_server.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/src/session_server.rs#L1899) |
| Test confidence | Weak | There is no session-protocol conformance or resident integration coverage for scene-resource upload because the wire path is absent | [tasks.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/tasks.md#L112) |
| Observability | Adequate | The seam is visible in docs and helper fallbacks, but there is no explicit upload contract artifact tying the layers together | [docs/exemplar-presence-card-user-test.md](/home/tze/gt/tze_hud/mayor/rig/docs/exemplar-presence-card-user-test.md#L39) |
| Delivery readiness | Weak | Resident exemplar/user-test surfaces still cannot exercise the path they claim to validate | [exemplar-presence-card/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md#L139) |
| Architectural fitness | Strong | The architecture wants one resident stream and already has resource-store/runtime primitives; the missing piece is coherent contract wiring | [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md#L145), [font_loader.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_runtime/src/font_loader.rs#L48) |

[Observed] The biggest strength is alignment with doctrine. One resident stream for the hot path is already the project’s stated transport shape, so fixing scene-resource ingress strengthens the core thesis instead of adding scope.

[Observed] The biggest blocker is contract incompleteness. Until the repo agrees on the upload acknowledgement path, error shape, capability authority, and envelope registry, implementation work will either guess or create another seam.

## Alignment Review

### Aligned next steps

- [Observed] Reconcile RFC 0011, RFC 0005, the resource-store spec, and the session-protocol spec around a coherent scene-resource upload handshake. Alignment: Core. User value: High. Leverage: High. Tractability: Needs spec. Timing: Now.
- [Observed] Choose the working envelope authority for this seam before assigning fields. Alignment: Core. User value: High. Leverage: High. Tractability: Needs spec. Timing: Now.
- [Inferred] Add explicit session envelope fields for `ResourceUploadStart`, `ResourceUploadChunk`, `ResourceUploadComplete`, a server acknowledgement carrying `upload_id`, `ResourceStored`, and an upload-specific error response that matches the session error model. Alignment: Core. User value: High. Leverage: High. Tractability: Ready after spec signoff. Timing: Soon.
- [Observed] Implement the server/runtime path and then repair resident exemplar/user-test consumers to use it. Alignment: Supporting. User value: High. Leverage: Medium. Tractability: Needs spec + schema first. Timing: Soon.

### Misaligned directions

- [Observed] A separate upload RPC or side-channel service is misaligned with the doctrine and the existing resource-store spec. It solves the local symptom by violating the system’s stated resident transport shape.

### Premature work

- [Observed] Building polished avatar/icon tooling before the wire contract exists is premature. The seam is protocol authority, not image-generation convenience.
- [Observed] Broad generalization of the resident helper into a reusable upload framework is premature before the core contract is stable and tested.

### Deferred

- [Observed] Durable or remote persistence concerns for scene resources remain deferred. The immediate seam is resident session ingress, not storage-class expansion.

### Rejected

- [Observed] Continuing to document placeholder-color squares as if they satisfy exemplar upload requirements should be rejected. It normalizes spec drift instead of resolving it.

## Gap Analysis

### Blockers

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No resident session-stream message set for scene-resource upload in `session.proto` | Resident agents cannot honestly upload images/fonts on the hot path | Runtime, resident clients | [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto#L31) | Write spec and schema delta first, then implement | M |
| RFC 0011 lacks a coherent start-ack message for `upload_id` | Chunked uploads are under-specified and cannot be implemented without guessing | Protocol designers, implementers | [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L191) | Reconcile RFC/main-spec contract before code | S |
| Upload-specific error shape does not match the shared session error model | Error handling can drift across gRPC/MCP/protocol surfaces again | Protocol designers, client authors | [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md#L382), [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L702) | Align or reject in spec before code | S |
| Upload-time budget language contradicts reference-time accounting | Implementers can reject valid uploads or bypass intended ownership semantics | Protocol and resource-store implementers | [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L252), [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L734), [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md#L91) | Reconcile budget enforcement point in spec before code | S |
| Server implementation has `upload_resource` capability but no matching wire path | Capability exists without a usable resident operation | Resident agents | [session_server.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/src/session_server.rs#L2826) | Implement after schema lands | M |
| Exemplar/user-test specs claim upload behavior that resident tooling cannot perform | Validation claims are overstated and user-visible proofs stay partial | Operators, reviewers | [exemplar-presence-card/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md#L139) | Repair consumers after core path exists | M |

### Important Enhancements

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| No conformance coverage for inline vs chunked vs dedup upload behavior | Future changes can silently re-break the seam | Runtime maintainers | [tasks.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/tasks.md#L112) | Add protocol and integration tests | M |
| No machine-readable seam artifact capturing recommended field allocations and ownership boundaries | Future readers can recreate the same drift | Contributors | [docs/reconciliations](/home/tze/gt/tze_hud/mayor/rig/docs/reconciliations) | Keep this direction artifact and OpenSpec change as source of truth | S |

### Strategic Gaps

| Gap | Why it matters | Who | Evidence | Response | Effort |
|-----|---------------|-----|---------|----------|--------|
| Resource upload consumer surfaces are fragmented across exemplars and helper scripts | Even after core repair, drift can recur in operator tooling | Tooling maintainers | [exemplar-dashboard-tile/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md#L58) | Consolidate after first honest consumer conversions land | M |
| RFC 0011 also defines `ResourceQuery`/`ResourceQueryResult` outside the current main v1 specs | Future workers could accidentally assume this seam fix closes more RFC 0011 surface than it does | Future planners | [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md#L528) | Keep explicit deferral unless main v1 specs are expanded | S |

## Work Plan

### Immediate alignment work

### Chunk 1: Reconcile resident scene-resource upload contract across RFC 0005, RFC 0011, and main specs

**Objective**: Make the resident upload handshake, response semantics, capability authority, and envelope allocation coherent before any code changes.
**Spec reference**: `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md`, `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`, and `about/law-and-lore/rfcs/0005-session-protocol.md`
**Dependencies**: None
**Why ordered here**: The current contract is internally inconsistent. Coding first would cement guesswork.
**Scope**: S
**Parallelizable**: No — this defines the authoritative contract all later work depends on.
**Serialize with**: Chunks 2-5

**Acceptance criteria**:
- [ ] Main specs explicitly distinguish scene-resource upload from widget asset registration
- [ ] The upload handshake includes a server acknowledgement path for `upload_id`
- [ ] Upload-specific failures are aligned to the shared session error model
- [ ] `upload_resource` capability authority is explicit
- [ ] Scene-resource budget enforcement is reconciled to a single point in the lifecycle
- [ ] Upload rate limiting and stream backpressure remain in seam scope because the main specs already require them
- [ ] Scene-resource ingress repair does not change the v1 ephemeral/durable persistence split
- [ ] Envelope field allocations are explicit and internally consistent against the chosen authority registry

**Notes**: The split `ClientMessage`/`ServerMessage` model in the main v1 spec and checked-in `session.proto` is the declared implementation authority for this seam. Chunk 1 reconciles stale RFC 0005 combined-envelope sections to that model rather than re-litigating the envelope shape.

### Chunk 2: Extend protobuf/session bindings for resident scene-resource upload

**Objective**: Add the agreed upload messages to `session.proto` and regenerate the Rust/Python surfaces.
**Spec reference**: Chunk 1 delta spec; `Requirement: ClientMessage and ServerMessage Envelopes`
**Dependencies**: Chunk 1
**Why ordered here**: Runtime and client work should target the real wire contract, not handwritten placeholders.
**Scope**: M
**Parallelizable**: No — shared schema surface.
**Serialize with**: Chunks 3-5

**Acceptance criteria**:
- [ ] `session.proto` carries the new resident upload messages and server responses
- [ ] Generated bindings compile for current consumers
- [ ] Existing widget asset registration fields remain unchanged

**Notes**: Keep widget SVG registration separate. Do not alias one path onto the other.

### Chunk 3: Implement resident session-stream scene-resource upload handling

**Objective**: Wire the session server and resource-store/runtime path for inline, chunked, dedup, and error cases.
**Spec reference**: `resource-store` and `session-protocol` delta specs
**Dependencies**: Chunk 2
**Why ordered here**: This is the core behavioral repair.
**Scope**: M
**Parallelizable**: No — depends on final schema and shared runtime ownership.
**Serialize with**: Chunks 4-5

**Acceptance criteria**:
- [ ] `upload_resource` capability gates the path
- [ ] Dedup, inline, chunked, size-limit, and concurrent-upload behaviors match spec
- [ ] Upload rate limiting and stream backpressure behavior remain compliant with the main v1 specs
- [ ] Scene-resource budget charging occurs at node-reference time rather than upload-storage time
- [ ] Errors use the agreed resource-specific response path rather than ad hoc placeholders

**Notes**: Keep runtime ownership of storage/budget accounting intact; do not move authority out of the runtime.

### Chunk 4: Add protocol, runtime, and integration coverage for resident upload

**Objective**: Make the repaired contract provable across schema, server logic, and representative consumer flows.
**Spec reference**: `Requirement: Chunked Upload Flow`, `Requirement: Upload Validation`, `Requirement: ClientMessage and ServerMessage Envelopes`
**Dependencies**: Chunk 3
**Why ordered here**: This seam has already drifted once; it needs durable coverage.
**Scope**: M
**Parallelizable**: Limited — can split schema/runtime tests from consumer tests after shared fixtures exist.
**Serialize with**: Chunk 5

**Acceptance criteria**:
- [ ] Tests cover inline upload, chunked upload, dedup hit, capability denial, and too-many-uploads rejection
- [ ] At least one resident integration path proves `StaticImageNode` reference after upload
- [ ] Any pre-existing helper/test assumptions that bypass the real upload path are removed or documented as out of scope

**Notes**: Add the missing conformance tests rather than relying on exemplar-only evidence.

### Chunk 5: Repair resident exemplar and `/user-test` consumers to use the real upload path

**Objective**: Replace placeholder/fallback behavior in resident exemplar tooling with real session-stream uploads.
**Spec reference**: [exemplar-presence-card/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md#L139), [exemplar-dashboard-tile/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md#L58)
**Dependencies**: Chunk 4
**Why ordered here**: Consumer repair should follow a proven core contract, not drive it.
**Scope**: M
**Parallelizable**: No — these consumers depend on the stabilized helper/bindings.
**Serialize with**: None

**Acceptance criteria**:
- [ ] Resident helper surfaces can upload a small PNG and obtain a `ResourceId`
- [ ] Presence Card and Dashboard Tile resident flows can use real uploaded resources
- [ ] Operator docs stop disclaiming the missing resident upload transport

**Notes**: Keep the first consumer tranche narrow: fix the known image-based exemplars before generalizing. Font consumer validation is not part of this first live-proof tranche.

### Block Reconciliation: Resident Scene-Resource Upload Seam

Check:
- [ ] Specs, protobuf, runtime, and helper surfaces all describe the same upload contract
- [ ] Upload correlation semantics are explicit for both success and failure
- [ ] The chosen error shape matches the shared session error model
- [ ] Exemplar requirements that depend on upload no longer rely on placeholders
- [ ] Follow-up gaps are captured in beads, not TODO comments

## Bead Graph

Epic:
- `TBD` — Expose RFC 0011 scene-resource upload on resident session stream

Planned children:
- `TBD.1` — Reconcile resident scene-resource upload contract across RFC 0005, RFC 0011, and main specs
- `TBD.2` — Extend session protocol schema with resident scene-resource upload messages
- `TBD.3` — Implement resident session-stream scene-resource upload handling
- `TBD.4` — Add conformance and integration coverage for resident upload flow
- `TBD.5` — Repair resident exemplar and `/user-test` consumers to use real uploads
- `TBD.6` — Reconcile spec-to-code (gen-1) for resident scene-resource upload
- `TBD.7` — Generate epic report for resident scene-resource upload seam

## Do Not Do Yet

| Item | Reason | Revisit when |
|------|--------|-------------|
| Add a separate upload RPC/service | Violates the resident one-stream doctrine and current resource-store spec | Never unless doctrine changes |
| Collapse scene-resource upload into `WidgetAssetRegister` | Conflates two distinct resource classes and breaks current widget path clarity | Never for v1 |
| Pull `ResourceQuery`/`ResourceQueryResult` into this seam fix without main-spec expansion | Broadens scope past the upload seam and muddies closure criteria | After main v1 specs explicitly require it |
| Generalize helper/tooling before the contract is reconciled | Would encode the current seam into more consumers | After Chunk 5 closes |
| Expand into post-v1 persistence or remote asset sync | Not the blocker here | After honest resident upload works end-to-end |

## Appendix

### A. Repository Map
- `crates/tze_hud_protocol/proto/session.proto` — resident session envelope and wire contract
- `crates/tze_hud_protocol/src/session_server.rs` — resident dispatch and protocol handling
- `crates/tze_hud_resource/` and `crates/tze_hud_runtime/src/font_loader.rs` — resource storage/consumption surfaces
- `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md` — current main upload contract
- `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` — current main session envelope contract
- `.claude/skills/user-test/` — operator-facing resident consumer surface affected by the seam

### B. Critical Workflows
1. Resident agent authenticates on `HudSession`
2. Agent uploads a scene resource on that same stream
3. Runtime acknowledges start, accepts chunks or inline data, validates bytes, and stores/dedups content
4. Agent references the returned `ResourceId` from a `StaticImageNode`
5. Resident exemplar/user-test flow proves the path on a real scenario instead of a placeholder

### C. Spec Inventory
- `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md` — requires upload on session stream
- `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` — currently omits the scene-resource upload messages
- `openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md` — depends on avatar upload
- `openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md` — depends on icon upload

### D. Evidence Index
- [vision.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/vision.md)
- [architecture.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/architecture.md)
- [v1.md](/home/tze/gt/tze_hud/mayor/rig/about/heart-and-soul/v1.md)
- [0011-resource-store.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0011-resource-store.md)
- [0005-session-protocol.md](/home/tze/gt/tze_hud/mayor/rig/about/law-and-lore/rfcs/0005-session-protocol.md)
- [resource-store/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/resource-store/spec.md)
- [session-protocol/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md)
- [session.proto](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/proto/session.proto)
- [session_server.rs](/home/tze/gt/tze_hud/mayor/rig/crates/tze_hud_protocol/src/session_server.rs)
- [exemplar-presence-card/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-presence-card/specs/exemplar-presence-card/spec.md)
- [exemplar-dashboard-tile/spec.md](/home/tze/gt/tze_hud/mayor/rig/openspec/changes/exemplar-dashboard-tile/specs/exemplar-dashboard-tile/spec.md)

---

## Conclusion

**Real direction**: This project needs a spec-first repair of the resident scene-resource upload contract so its one-stream resident presence model is true in code, not just in doctrine.

**Work on next**: Reconcile the upload handshake across RFC 0011 and main specs, extend `session.proto`, then implement and test the resident upload path before repairing resident exemplar consumers.

**Stop pretending**: The repo cannot yet honestly claim that resident raw-tile exemplars upload scene resources over the session stream.
