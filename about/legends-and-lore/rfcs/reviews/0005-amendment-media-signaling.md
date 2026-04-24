# RFC 0005: Session/Protocol — Amendment: Media Signaling in Session Envelope

**Amendment ID:** 0005-amendment-media-signaling
**Issue:** hud-ora8.1.10
**Date:** 2026-04-19
**Author:** hud-ora8.1.10 (parallel-agents worker)
**Depends on:** RFC 0014 (Media Plane — v2-embodied-media-presence program)
**Parent task:** hud-ora8.1 (v2 embodied media presence program preparation)

---

> **ERRATUM (2026-04-24) — Field Allocations in §A1 and §9.2 Superseded**
>
> The `ClientMessage` fields 50–51 (`MediaIngressOpen`, `MediaIngressClose`) and
> `ServerMessage` fields 50–52 (`MediaIngressOpenResult`, `MediaIngressState`,
> `MediaIngressCloseNotice`) allocated in §A1 and recorded in §9.2 of this
> amendment are **invalid and must not be implemented**.
>
> **Root cause:** Fields 50 and 51 on `ServerMessage` were concurrently consumed
> by the persistent-movable-elements work: `list_elements_response` took
> `ServerMessage` field 50 and `element_repositioned` took field 51. The Amendment
> A1 allocations therefore collide with in-use field numbers.
>
> **Authoritative replacement:** RFC 0014 (Media Plane Wire Protocol) §2.2
> relocates all v2 media signaling messages to the **60–79 range** (both
> directions). The 80–99 range is reserved for phase 4 sub-epics; RFC 0018
> §4.3 has since allocated fields 80–81 (client: `CloudRelayOpen`,
> `CloudRelayClose`) and 80–82 (server: `CloudRelayOpenResult`,
> `CloudRelayCloseNotice`, `CloudRelayStateUpdate`) for phase 4b cloud-relay.
>
> Any implementation of the media signaling messages described here **MUST use
> the field numbers from RFC 0014 §2.2**, not the numbers written in this document.
> RFC 0014 §2.2 is the single authoritative field registry for all post-v1 media
> signaling envelope fields.
>
> Task 1.2 of the v2 program (PR #574 / #575, issue hud-ora8.1.23) wires the
> authoritative 60–79 range into `session.proto`. All references to fields 50–52
> (server) and 50–51 (client) for media messages in this document should be read
> as 60–62 (server) and 60–61 (client) respectively.

---

## Scope and Purpose

This amendment extends RFC 0005 (Session/Protocol) to specify how media
signaling messages are carried inside the existing `HudSession.Session`
bidirectional stream envelope. It documents:

1. The confirmed shape decision: session-envelope extension (fields 50–99),
   not a separate `rpc MediaSignaling(...)`.
2. The concrete post-v1 field allocations for bounded media ingress signaling.
3. The traffic class contract for each new message variant.
4. Preservation guarantees for existing v1 fields — in particular
   `WidgetPublishResult.request_sequence` and Layer 3 (protocol adapter)
   extension semantics — which MUST NOT be disturbed by the post-v1 addition.
5. A forward-compatibility contract so v1 agents remain wire-compatible with
   runtimes that implement this amendment.

This amendment is **documentary only**. No `session.proto` field-number
changes are introduced here. The actual protobuf schema changes are owned by
task hud-ora8.1.23 and MUST follow the field allocations documented below
without deviation.

---

## Background: Why Session-Envelope Extension

RFC 0005 §2.1 already reserved fields 50–99 for post-v1 embodied/media
signaling. RFC 0005 §12 (Open Questions, item 5) explicitly named the
session-envelope extension shape as the preferred path. The formal shape
decision is recorded in
`docs/reconciliations/webrtc_media_v1_signaling_shape_decision.md`
(issue `hud-nn9d.7`), which chose session-envelope extension because:

- The architecture doctrine enforces one bidirectional stream per agent
  (`about/heart-and-soul/architecture.md`) — adding a separate media RPC
  would violate this invariant.
- `HudSession.Session` already concentrates all multiplexed session traffic
  (mutations, zone/widget publish, input control, heartbeat, telemetry) and
  any existing extension seams (e.g., `ZonePublish` at field 25) are already
  wired through one path in `crates/tze_hud_protocol/src/session_server.rs`.
- A separate media RPC at this stage would introduce a new auth/version-gating
  surface, new reconnect semantics, and new backpressure policy — all
  avoidable overhead before embodied/bidirectional AV is admitted.

For the deferred embodied/bidirectional media scope (v2 Phase 4 per
`openspec/changes/v2-embodied-media-presence/design.md`), a separate
`rpc MediaSignaling(...)` on `SessionService` remains an open option; that
decision is out of scope here.

---

## Amendment Content

### A1. Session-Envelope Field Allocations for Media Signaling (Post-v1)

Fields 50–99 in `SessionMessage.payload` (or the equivalent `ClientMessage`
/ `ServerMessage` split-envelope layout used in the implementation) are
reserved for post-v1 use. This amendment allocates the first tranche for
**bounded media ingress signaling**, as specified in
`docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md`
(issue `hud-nn9d.8`).

#### ClientMessage — post-v1 allocations

| Field | Message | Traffic Class | Description |
|-------|---------|--------------|-------------|
| 50 | `MediaIngressOpen` | Transactional | Agent requests one-way inbound visual stream bound to a runtime zone. Carries transport descriptor, surface identity, and timing/classification intent. Requires `media_ingress` capability. |
| 51 | `MediaIngressClose` | Transactional | Agent-initiated teardown intent for an active media ingress stream. |

#### ServerMessage — post-v1 allocations

| Field | Message | Traffic Class | Description |
|-------|---------|--------------|-------------|
| 50 | `MediaIngressOpenResult` | Transactional | Deterministic accept/reject. Returns runtime-assigned `stream_epoch` for reconnect consistency. |
| 51 | `MediaIngressState` | State-stream (coalesced) | Coalescible state updates for active stream health and degradation. Latest state wins; intermediate states may be skipped. |
| 52 | `MediaIngressCloseNotice` | Transactional | Runtime-initiated termination notice (policy disable, lease revoke, budget gate, transport failure). |

Fields 53–99 remain unallocated. Do not fill gaps speculatively. Any future
allocation in this range requires an RFC 0005 amendment or a new RFC.

### A2. Traffic Class Contract for Media Signaling Variants

The four-class message model from RFC 0005 §2.5 / CLAUDE.md applies to
media-signaling payloads individually, not to the envelope as a whole:

| Message | Class | Rationale |
|---------|-------|-----------|
| `MediaIngressOpen` | Transactional | Admission decision must be reliable and ordered; failure to ack is a hard blocker |
| `MediaIngressOpenResult` | Transactional | Result must be reliably delivered; used for reconnect epoch matching |
| `MediaIngressState` | State-stream | Health/degradation updates are coalescible; only the latest state matters |
| `MediaIngressClose` | Transactional | Teardown intent must not be dropped |
| `MediaIngressCloseNotice` | Transactional | Runtime termination must be reliably delivered before subsequent session events |

Per `docs/reconciliations/webrtc_media_v1_signaling_shape_decision.md`
§"Performance Trade-Off", the implementation MUST document per-payload
message-size constraints and denial semantics to prevent head-of-line blocking
of existing v1 traffic (mutations, input events) when media and non-media
flows share the stream.

### A3. Capability Gating

Media signaling is gated by two independent conditions:

1. `negotiated_protocol_version` must advertise support for the media-signaling
   extension (a new minor-version bump from the v1 baseline).
2. The session's granted capabilities must include `media_ingress` (for
   inbound media) or the applicable capability for the admitted media type.

If either gate is absent:
- The agent MUST NOT emit `MediaIngress*` payloads.
- The runtime MUST reject any attempted media-signaling message with a
  structured `RuntimeError` (`error_code = "CAPABILITY_REQUIRED"`) and leave
  all v1 zone/widget publish behavior unchanged.

This preserves the feature-negotiated suppression model used by all other
post-v1 additions (RFC 0005 §4.3).

### A4. Reconnect and Snapshot Semantics

Reconnect behavior follows the existing session rules unchanged:

- On accepted `SessionResume`, ordering is: `SessionResumeResult`, then full
  `SceneSnapshot` (v1 snapshot-first model, RFC 0005 §6.4).
- Any pre-disconnect media transport is treated as non-authoritative after
  resume until a post-resume `MediaIngressOpenResult` confirms it via
  `stream_epoch` matching.
- Matching `stream_epoch` between `MediaIngressOpenResult` and the current
  `MediaIngressState`: stream may continue without a fresh open.
- Mismatched or absent `stream_epoch`: the agent MUST issue a fresh
  `MediaIngressOpen`.
- No WAL/delta replay dependency is introduced; snapshot-first reconnect
  remains valid for this tranche.

`SceneSnapshot.snapshot_json` MUST include declarative media publication
state (zone registry `active_publications` with `VideoSurfaceRef`,
`expires_at_wall_us`, and `content_classification`) when media is active.
Transport internals (SDP/ICE blobs, DTLS/SRTP state, decoder handles)
MUST NOT appear in the snapshot.

### A5. Backward Compatibility

All additions under this amendment are additive-only in protobuf:
- No existing field numbers are renumbered.
- No existing `oneof` arms are reused or semantically reassigned.
- No existing message definitions are altered.

V1 agents that do not know the media-signaling fields will ignore them per
protobuf forward-compatibility rules. Runtimes implementing this amendment
suppress media-signaling delivery to v1 agents (capability/version gate
absent), so v1 agents never see `MediaIngress*` messages.

### A6. Cross-Pillar Reference to RFC 0014

The media plane doctrine, WebRTC signaling lifecycle, transport descriptor
format, `VideoSurfaceRef` compositor behavior, and operator/privacy governance
for media are defined in **RFC 0014 (Media Plane)**. RFC 0014 is the
authoritative cross-pillar reference for:

- the v2 Phase 1–4 media program structure
  (`openspec/changes/v2-embodied-media-presence/design.md`),
- the governance model that media MUST obey (capability, lease, privacy,
  operator-policy, and budget gates),
- the `MediaIngressOpen` transport descriptor wire format and field semantics.

RFC 0005 governs the session-envelope allocation and traffic class contract.
RFC 0014 governs the media-plane semantics, lifecycle, and governance. The
two RFCs MUST be read together for a complete picture of media signaling.

---

## Protected Fields — Preservation Guarantee

The following fields and semantics introduced by prior work on RFC 0005 are
explicitly protected by this amendment. No change introduced by hud-ora8.1.23
(or any subsequent media-plane implementation task) may alter, remove, or
renumber them.

### WidgetPublishResult.request_sequence (PROTECTED)

`WidgetPublishResult.request_sequence` (field 1 of `WidgetPublishResult`,
`ServerMessage` field 47) was specified by the rust-widget-publish-load-harness
work (openspec change `2026-04-18-rust-widget-publish-load-harness`, issue
`hud-nn9d`; see
`openspec/changes/archive/2026-04-18-rust-widget-publish-load-harness/specs/session-protocol/spec.md`).

It is the `uint64` echoing the originating `ClientMessage.sequence` of the
durable `WidgetPublish` that triggered this result. The
`examples/widget_publish_load_harness/src/main.rs` harness uses it as the
in-flight correlation key for latency measurement and RTT tracking.

**This field MUST be preserved exactly as defined:**
- field number: 1 within `WidgetPublishResult`
- type: `uint64`
- semantic: echo of the `ClientMessage.sequence` value from the correlating
  `WidgetPublish` message
- presence: present in EVERY `WidgetPublishResult` (accepted and rejected)

Any media-plane implementation work MUST NOT:
- add a field numbered 1 to `WidgetPublishResult` with different semantics,
- change the type of `request_sequence`,
- make `request_sequence` optional where it was formerly always populated,
- reuse `ServerMessage` field 47 for any purpose other than `WidgetPublishResult`.

### Layer 3 Extension Semantics (PROTECTED)

Layer 3 in the tze_hud component topology (`about/lay-and-land/components.md`)
is the protocol-adapter layer: `tze_hud_protocol` provides the session
transport bridge between external agents and the runtime scene/telemetry
surfaces. The bounded-ingress and mcp-stress-testing work layers added
extension semantics to this layer that must be preserved:

1. **`ZonePublishResult.request_sequence`** — analogous to
   `WidgetPublishResult.request_sequence`; field 1 of `ZonePublishResult`,
   correlating durable `ZonePublish` acks. MUST NOT be altered.

2. **`WidgetAssetRegisterResult.request_sequence`** — field 1 of
   `WidgetAssetRegisterResult` (`ServerMessage` field 48); correlates widget
   SVG asset registration acks. MUST NOT be altered.

3. **`ClientMessage` field 35 / `ServerMessage` field 47** —
   `WidgetPublish` / `WidgetPublishResult` field assignments established by the
   widget-system and rust-widget-publish-load-harness openspec changes. MUST
   NOT be renumbered or reused.

4. **`ClientMessage` field 34 / `ServerMessage` field 48** —
   `WidgetAssetRegister` / `WidgetAssetRegisterResult` field assignments.
   MUST NOT be renumbered or reused.

5. **`ServerMessage` fields 50–52** — `MediaIngressOpenResult`,
   `MediaIngressState`, and `MediaIngressCloseNotice` as allocated in §A1 of
   this amendment. Once allocated by hud-ora8.1.23, these numbers MUST NOT be
   reused for a different purpose.

6. **`ClientMessage` fields 50–51** — `MediaIngressOpen` and
   `MediaIngressClose` as allocated in §A1. Same protection applies.

---

## §9.2 Field Registry Update (Addendum)

The following rows are addended to the §9.2 field registry defined in
RFC 0005 §9.2 (authoritative as of Round 14):

**ClientMessage post-v1 additions (50–51):**

| Field | Message | Status | Notes |
|-------|---------|--------|-------|
| 50 | `MediaIngressOpen` | Post-v1; not v1 | Allocated by this amendment; schema in RFC 0014 |
| 51 | `MediaIngressClose` | Post-v1; not v1 | Allocated by this amendment; schema in RFC 0014 |
| 52–99 | (unallocated) | Reserved | Reserved for future post-v1 use; do not fill speculatively |

**ServerMessage post-v1 additions (50–52):**

| Field | Message | Status | Notes |
|-------|---------|--------|-------|
| 50 | `MediaIngressOpenResult` | Post-v1; not v1 | Allocated by this amendment; schema in RFC 0014 |
| 51 | `MediaIngressState` | Post-v1; not v1 | Allocated by this amendment; schema in RFC 0014 |
| 52 | `MediaIngressCloseNotice` | Post-v1; not v1 | Allocated by this amendment; schema in RFC 0014 |
| 53–99 | (unallocated) | Reserved | Reserved for future post-v1 use; do not fill speculatively |

---

## §11 Cross-RFC Table Update (Addendum)

The following row is addended to the §11 cross-RFC table in RFC 0005:

| RFC | Relationship |
|-----|-------------|
| RFC 0014 (Media Plane) | `MediaIngressOpen`, `MediaIngressOpenResult`, `MediaIngressState`, `MediaIngressClose`, and `MediaIngressCloseNotice` (session envelope fields 50–51 client-side, 50–52 server-side) carry media transport semantics defined in RFC 0014. RFC 0005 governs field allocation and traffic class; RFC 0014 governs media lifecycle, governance, and transport descriptor format. The capability `media_ingress` is defined in RFC 0014 and gated here via the standard capability model (§5.3). |

---

## Implementation Notes for hud-ora8.1.23

The proto schema changes are owned by task hud-ora8.1.23 (Task 1.2 of the
v2 program). That task MUST:

1. Use the field numbers specified in §A1 above exactly as documented — no
   deviation from the allocated ranges.
2. Add `reserved` declarations for any field numbers in the 50–99 range that
   are not yet allocated, to prevent accidental wire-format reuse.
3. Extend `ZonePublish` at field 25 additively with
   `present_at_wall_us` (field 7), `expires_at_wall_us` (field 8), and
   `content_classification` (field 9) per
   `docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md`
   §"session.proto deltas". Note: the delta doc specifies fields 6/7/8, but
   `element_id` was added at field 6 after the delta doc was written
   (hud-bs2q.3, commit 7427da6). The correct safe starting field is 7; the
   delta doc is superseded by this amendment for field numbering.
4. Extend `ZoneContent` oneof with `StaticImageRef` (field 5) and
   `VideoSurfaceRef` (field 6) per the `types.proto` deltas in the same doc.
5. Confirm that `WidgetPublishResult.request_sequence` (field 1) is NOT
   touched — verify by diffing the message definition against this amendment's
   protection clause before opening the PR.

---

## Open Questions Closed by This Amendment

This amendment closes RFC 0005 §12 Open Question 5 ("Embodied session
stream"): the decision for the bounded media ingress tranche is
session-envelope extension (fields 50–99), not a separate gRPC method.
A separate `rpc MediaSignaling(...)` remains deferred and may be reconsidered
only when the embodied/bidirectional media scope is admitted (v2 Phase 4).

RFC 0005 §12 Open Questions 1–4 are not affected by this amendment.
