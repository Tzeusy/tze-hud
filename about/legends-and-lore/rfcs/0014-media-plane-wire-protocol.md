# RFC 0014: Media Plane Wire Protocol

**Status:** Draft — pending external review (≥2 external reviewers required per signoff packet F29)
**Issue:** hud-ora8.1.8
**Date:** 2026-04-19
**Authors:** tze_hud architecture team (drafted by hud-ora8.1.8 parallel-agents worker)
**Depends on:**
- RFC 0001 (Scene Contract) — `VideoSurfaceRef`, `SceneId`, namespace model
- RFC 0002 (Runtime Kernel) §2.8 Media Worker Boundary + Amendment A1 media worker lifecycle (hud-ora8.1.9)
- RFC 0005 (Session Protocol) + Amendment A1 media signaling in session envelope (hud-ora8.1.10)
- RFC 0008 (Lease Governance) + Amendment A1 C13 capability dialog (hud-ora8.1.11)
- RFC 0009 (Policy Arbitration) + Amendment A1 C12 role-based operators (hud-ora8.1.12)
- `about/heart-and-soul/media-doctrine.md` (doctrine layer — precedes this RFC)
- `about/heart-and-soul/failure.md` §"E25 degradation ladder" (authoritative ladder)
- `about/heart-and-soul/v2.md` (V2 program structure)
- `about/heart-and-soul/security.md` §"In-process media and runtime workers"
- E24 COMPATIBLE verdict (`docs/decisions/e24-in-process-worker-posture.md`)
- GStreamer pipeline audit (`docs/audits/gstreamer-media-pipeline-audit.md`)
- cpal audio I/O audit (`docs/audits/cpal-audio-io-crate-audit.md`)
**Parent program:** v2-embodied-media-presence (phase 1)
**Forward references:**
- RFC 0015 (Embodied Presence Contract) — presence-level activation that media plane serves
- RFC 0017 (Recording and Audit) — phase 4a capability layered on top of this protocol
- RFC 0018 (Cloud-Relay Trust Boundary) — phase 4b transport mode that plugs into this protocol

---

## Summary

This RFC defines the **wire protocol** for tze_hud's media plane: the set of
protobuf messages, field allocations, state machines, signaling handshakes,
codec negotiation rules, and degradation hooks that let agents request,
operate, and terminate governed media streams against a runtime session.

It is the **mechanism layer** that implements the posture defined in
`about/heart-and-soul/media-doctrine.md` and the lifecycle contract defined in
RFC 0002 Amendment A1 (media worker lifecycle). It is the partner document to
the RFC 0005 Amendment A1 (media signaling) session-envelope allocation: RFC
0005 owns the envelope slot assignments and traffic-class contracts; RFC 0014
owns the payload schemas, state machines, signaling semantics, and governance
hooks those slots carry.

Scope ordering from doctrine to mechanism:

1. **Doctrine** (`media-doctrine.md`) — what the media plane IS, what it
   REFUSES, four governance pillars. Locked in before mechanism.
2. **Amendments** (RFC 0002 A1, RFC 0005 A1, RFC 0008 A1, RFC 0009 A1) —
   lifecycle contract, envelope slots, capability dialog, role authority.
3. **This RFC (0014)** — wire shape, state machine diagrams, SDP handling,
   degradation mechanism, worker-pool protocol API, audio stack.
4. **Capability specs** (`openspec/changes/v2-embodied-media-presence/specs/`)
   — operator-visible contracts authored against this RFC.

**Field number allocation:** This RFC allocates session envelope fields **60–79**
(both directions) for v2 media transport and control messages. Fields 50–59
(both directions) were informally reserved for media by RFC 0005 Amendment A1,
but fields 50–51 were concurrently consumed by the persistent-movable-elements
work (`list_elements_response` / `element_repositioned`). The RFC 0005 Amendment
field allocations for `MediaIngressOpen*` MUST be relocated from 50–52 to the
60–79 range; this RFC carries the authoritative numbering and flags the
erratum (§2.2, §12.1).

**F29 gate:** per the signoff packet, this RFC is the highest-leverage
irreversible decision in v2 and requires **≥2 external reviewer sign-offs**
before merge. Phase 1 implementation beads are blocked on this RFC landing.

---

## Motivation

The tze_hud v1 baseline is deliberately mute: agents publish scene mutations,
text, and widget parameters over the session stream; no live media moves over
the wire. V1 ships without a media plane by design (see `v1.md`, "V1
explicitly defers").

V2 activates the media plane. It does so as a *governed surface*, not as a
free pass — every byte of video and every sample of audio crosses through
capability gates, privacy policy, attention budgets, and the E25 degradation
ladder, on the same arbitration stack as every other agent-visible surface.
Without a precise wire protocol:

- Agents have no stable contract for requesting or operating media streams.
- The runtime cannot deterministically accept, reject, degrade, or revoke
  streams across sessions, operators, and device profiles.
- Signaling (SDP exchange, ICE, DTLS-SRTP) has no documented security
  envelope — raw SDP bodies would traverse the control plane without audit.
- Phase 4 sub-epics (bidirectional AV, cloud-relay, recording, agent-to-agent
  media) have no common substrate to build on, multiplying design cost.
- Validation cannot assert that a given stream obeys its budget, its
  capability scope, or the degradation ladder because it has no normative
  state machine to assert against.

This RFC resolves those by giving the media plane a single authoritative
wire definition that every later phase and sub-epic extends additively.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| Wire protocol for bounded media ingress and v2 phase 1 activation | §2 Wire Protocol, §3 Session Lifecycle |
| Media signaling envelope (SDP handling + security analysis) | §4 Media Signaling |
| Session lifecycle state machine rendered as `statig`-compatible diagram (E26) | §3 Session Lifecycle State Machine |
| Codec negotiation (H.264 + VP9 for v2; AV1 deferred per D18) | §2.5 Codec Negotiation |
| Degradation ladder mechanism per E25 (10 steps) | §5 Degradation Mechanism |
| Worker pool protocol API (E24) | §6 Worker Pool Interface |
| Audio stack contract (E22 Opus + stereo) | §7 Audio Stack |
| Relationship to RFC 0005 envelope + RFC 0002 A1 worker lifecycle preserved | §8 Relationships |
| Security analysis (signaling, codec CVE surface, capability gates) | §9 Security Considerations |
| Open questions and deferred work flagged | §10 Open Questions / Future Extensions |
| Review record surface for external reviewers (≥2 required) | §11 Review Record |

---

## 1. Scope

### 1.1 In-scope

This RFC specifies, normatively:

1. **Session envelope field allocations** (fields 60–79, both directions) and
   the protobuf message shapes that occupy them.
2. **Media session lifecycle state machine** covering `ADMITTED`, `STREAMING`,
   `DEGRADED`, `PAUSED`, `CLOSING`, `CLOSED`, and terminal `REVOKED` states
   plus all transitions between them.
3. **Media signaling handshake** — transport descriptor payload, SDP
   offer/answer handling posture, ICE/DTLS/SRTP lifecycle hooks, `stream_epoch`
   reconnect semantics.
4. **Codec negotiation** — the v2 codec envelope (H.264 baseline + VP9),
   hardware-decode fallback posture, and AV1 deferral.
5. **Degradation mechanism** — how the E25 ladder from
   `about/heart-and-soul/failure.md` translates into protocol-visible state
   transitions, which actor may trigger each step, and how the runtime reports
   advances.
6. **Worker pool protocol API** — the protocol-level envelope for E24's shared
   worker pool: spawn gate, preemption messages, watchdog-driven termination
   notices.
7. **Audio stack wire shape** — how Opus stereo traffic (E22) is modeled at
   the signaling layer; audio-only stream variants.
8. **Security considerations** — capability gate, codec CVE surface,
   SDP/signaling security review, denial-of-service surface, observability.
9. **Relationship to prior documents** — RFC 0005 envelope, RFC 0002 A1 worker
   lifecycle, RFC 0008 A1 dialog, RFC 0009 A1 roles, E24/E25 doctrine.

### 1.2 Out of scope

This RFC deliberately does not cover:

1. **Implementation schema in protobuf source.** Field numbers and message
   shapes are specified here; edits to `crates/tze_hud_protocol/proto/session.proto`
   and `types.proto` are owned by task hud-ora8.1.23 (proto wiring task) and
   MUST follow this RFC exactly.
2. **RFC 0015 embodied-presence state machine.** Embodied presence has its
   own state machine (`REQUESTING → ACTIVE → ORPHANED → (ACTIVE | EXPIRED)`
   plus `DEGRADED`). Media streams may be owned by resident or embodied
   sessions; the session presence level is RFC 0015's concern. This RFC
   treats the presence level as an input, not a variable.
3. **Recording wire protocol** (RFC 0017 phase 4a) and **cloud-relay trust
   boundary** (RFC 0018 phase 4b). Both extend this RFC additively; their
   wire fields will land in the 80–99 envelope range reserved here for phase
   4 additions.
4. **Device profile composition** (RFC 0016 phase 3). Glasses upstream
   composition carries pre-composited WebRTC frames over this protocol's
   media plane, but the per-profile negotiation lives in RFC 0016.
5. **Federation** (`federated-send` capability). The capability is defined in
   the RFC 0008 A1 taxonomy but rejected at runtime in v2
   (`CAPABILITY_NOT_IMPLEMENTED`). This RFC reserves no federation-specific
   fields and assumes no federation wire semantics.
6. **Audio decoder implementation details.** Opus decoder choice, sample-rate
   conversion strategy, and backend selection (cpal / WASAPI / CoreAudio / etc.)
   are implementation concerns under the `cpal-audio-io-crate-audit.md`
   mandate, not wire protocol.

### 1.3 Non-goals

Independently of in/out-of-scope lines, this RFC refuses to define:

1. A separate `rpc MediaSignaling(...)` gRPC method. RFC 0005 A1's
   session-envelope extension decision is normative for v2; separate transport
   remains a deferred option for v2 phase 4 (embodied bidirectional AV) and
   may be reconsidered under a future amendment. Until then, media signaling
   rides the single `HudSession.Session` bidirectional stream.
2. An agent-initiated degradation demand. Per E25 doctrine, agents may request
   teardown of *their own* stream only (`MediaIngressClose` equivalent). They
   MAY NOT request degradation of other agents' streams, request the runtime
   to advance its degradation level, or announce their own resource pressure
   as a shedding signal. The degradation decision remains the runtime's.
3. Raw SDP bodies flowing unscrutinized through the control plane. §4 defines
   the transport-descriptor wrapper, the SDP handling constraints, and the
   security boundary that SDP crosses at the runtime. The raw-SDP-on-wire path
   is explicitly rejected.
4. A media session lifetime independent of session presence level. A media
   session is always owned by exactly one agent session; teardown of the
   owning session tears down its media session (§3.4). There is no
   session-less, agent-less media path in v2.

---

## 2. Wire Protocol

### 2.1 Envelope Host

All v2 media plane traffic rides the existing `HudSession.Session` bidirectional
gRPC stream (RFC 0005 §1). No new RPC method is introduced. `ClientMessage` and
`ServerMessage` envelopes carry media payloads inside their `oneof payload`
fields at the allocations in §2.2.

The single-envelope choice is normative per:

- RFC 0005 Amendment A1 §"Background: Why Session-Envelope Extension"
- `about/heart-and-soul/architecture.md` ("one bidirectional stream per
  agent" invariant)

### 2.2 Field Number Allocations

This RFC allocates **fields 60–79** (both directions) for v2 phase 1 media
plane payloads. Fields 80–99 (both directions) are reserved for phase 4
additions (recording, cloud-relay, agent-to-agent media, bidirectional AV).

#### Erratum to RFC 0005 Amendment A1

RFC 0005 Amendment A1 §A1 allocated `MediaIngress*` messages to
`ServerMessage` fields 50–52 and `ClientMessage` fields 50–51. Those field
numbers were concurrently consumed by persistent-movable-elements work
(`list_elements_response` at ServerMessage field 50, `element_repositioned`
at ServerMessage field 51). The Amendment A1 allocations MUST be relocated
to the 60–79 range defined here. This RFC is the authoritative numbering;
any implementation of Amendment A1's media signaling messages MUST use the
allocations below, not the 50–52 numbers written in Amendment A1. RFC 0005
Amendment A1 SHOULD be updated with an erratum pointer to this RFC the next
time it is touched (tracked as discovered work for task hud-ora8.1.23).

#### 2.2.1 ClientMessage post-v1 allocations (phase 1 activation)

| Field | Message | Traffic Class | Description |
|-------|---------|--------------|-------------|
| 60 | `MediaIngressOpen` | Transactional | Agent requests inbound media stream bound to a runtime zone or tile. Carries transport descriptor, codec intent, surface identity, capability claim. |
| 61 | `MediaIngressClose` | Transactional | Agent-initiated teardown intent. Idempotent w.r.t. already-terminated streams. |
| 62 | `MediaSdpAnswer` | Transactional | Agent-side SDP answer in response to a runtime-initiated `MediaSdpOffer` (§4.2). Subject to §9 SDP security scrutiny. |
| 63 | `MediaIceCandidate` | Ephemeral realtime | ICE candidate exchange during transport establishment. Latest-wins coalescing permitted per candidate family. |
| 64 | `MediaEgressOpen` | Transactional | **Reserved for phase 4** (bidirectional AV). Agent requests an outbound media stream (voice synthesis, agent-emitted audio). Wire-reserved in v2; runtime rejects with `CAPABILITY_NOT_IMPLEMENTED`. |
| 65 | `MediaPauseRequest` | Transactional | Agent requests its own stream transition from `STREAMING` to `PAUSED` (§3.3) without teardown. |
| 66 | `MediaResumeRequest` | Transactional | Agent requests its own stream transition from `PAUSED` back to `STREAMING`. |

Fields 67–79 (client) are unallocated. Do not fill gaps speculatively.

#### 2.2.2 ServerMessage post-v1 allocations (phase 1 activation)

| Field | Message | Traffic Class | Description |
|-------|---------|--------------|-------------|
| 60 | `MediaIngressOpenResult` | Transactional | Deterministic accept/reject of `MediaIngressOpen`. Carries `stream_epoch` (reconnect key), chosen codec, assigned `surface_id`. |
| 61 | `MediaIngressState` | State-stream | Coalescible health/degradation updates for active stream (frame rate, bitrate, dropped frames, current degradation step). Latest state wins. |
| 62 | `MediaIngressCloseNotice` | Transactional | Runtime-initiated termination notice. Carries structured `close_reason` (policy revoke, budget gate, preemption, operator mute, transport failure, watchdog threshold exceeded). |
| 63 | `MediaSdpOffer` | Transactional | Runtime-side SDP offer presented to the agent during transport establishment (see §4.2). |
| 64 | `MediaIceCandidate` | Ephemeral realtime | ICE candidate from runtime. Same semantics as client field 63. |
| 65 | `MediaDegradationNotice` | Transactional | Per-stream notice that the runtime has advanced this stream's degradation step (E25 ladder step, current resolution/framerate, recovery conditions). Delivered in addition to the global `DegradationNotice` (RFC 0005 field 46), which remains unchanged. |
| 66 | `MediaEgressOpenResult` | Transactional | **Reserved for phase 4** (paired with client field 64). Wire-reserved in v2. |
| 67 | `MediaPauseNotice` | Transactional | Runtime-initiated pause (operator request, policy trigger, or agent's `MediaPauseRequest` ack). |
| 68 | `MediaResumeNotice` | Transactional | Counterpart to `MediaPauseNotice`. |

Fields 69–79 (server) are unallocated. Do not fill gaps speculatively.

#### 2.2.3 Fields 80–99 reservation (phase 4)

Fields 80–99 (both directions) are reserved for phase 4 sub-epic additions:

- Recording start/stop/access (RFC 0017, phase 4a).
- Cloud-relay activation / SFU attach (RFC 0018, phase 4b).
- Agent-to-agent media routing (phase 4e).
- Bidirectional AV / voice synthesis egress signaling (phase 4f).

Each sub-epic RFC is responsible for allocating within this range additively;
no gap-filling from the 80–99 range is permitted in phase 1. The allocation
registry for 80–99 is empty at v2 phase 1 activation.

### 2.3 Core Message Shapes

Definitions below are normative for the protobuf schema. Actual `.proto` edits
are owned by task hud-ora8.1.23.

#### 2.3.1 `MediaIngressOpen` (ClientMessage field 60)

```protobuf
// Agent requests admission of an inbound media stream.
// Transactional: exactly one MediaIngressOpenResult is emitted in response
// (accept or reject). Requires `media-ingress` capability grant (RFC 0008 A1)
// AND a matching capability dialog passage or 7-day remember record.
message MediaIngressOpen {
  // Client-generated stream identity for correlation.
  // UUIDv7; echoed in MediaIngressOpenResult.
  bytes  client_stream_id           = 1;

  // Transport descriptor (§4.3). Carries the transport mode
  // (WEBRTC_STANDARD | WEBRTC_PRECOMPOSITED_GLASSES | FUTURE_CLOUD_RELAY),
  // agent-side ICE/DTLS fingerprints (if offer-in-advance), and SDP offer
  // if the agent is initiating.
  TransportDescriptor  transport    = 2;

  // Surface binding: the runtime scene surface into which decoded frames
  // will be composited. Exactly one MUST be set.
  oneof surface_binding {
    // Zone-bound media (§3.1 scene-zone-owned publication path).
    // Follows RFC 0005 A1 VideoSurfaceRef extension in ZoneContent.
    string zone_name                 = 3;

    // Tile-bound media (embodied/resident tile-owned stream).
    // Tile must be owned by the requesting session's active lease.
    bytes  tile_id                   = 4;
  }

  // Codec intent (§2.5). Ordered preference list. Runtime picks highest
  // preference it can serve; rejects if none match.
  repeated MediaCodec codec_preference = 5;

  // Audio track present? (E22 Opus stereo). Informational; actual codec
  // carried in codec_preference.
  bool   has_audio_track              = 6;

  // Video track present? Most streams are video-only or video+audio; audio-
  // only is valid (e.g., microphone-ingress capability path).
  bool   has_video_track              = 7;

  // Content classification intent (privacy.md viewer-class filter input).
  // Empty = unclassified (defaults to most-restrictive per privacy policy).
  string content_classification       = 8;

  // Optional wall-clock scheduling of stream activation (RFC 0003 §3.5).
  // 0 = admit and begin STREAMING as soon as transport is established.
  uint64 present_at_wall_us           = 9;

  // Optional wall-clock expiry (RFC 0003 §3.5).
  // 0 = no expiry; stream runs until explicit close or revocation.
  uint64 expires_at_wall_us           = 10;

  // Agent's declared peak bitrate (informational; runtime caps against
  // the session's resource budget and per-stream watchdog threshold).
  uint32 declared_peak_kbps           = 11;
}
```

#### 2.3.2 `MediaIngressOpenResult` (ServerMessage field 60)

```protobuf
// Transactional admission result. Carries stream_epoch (reconnect key),
// chosen codec, and runtime-assigned surface identity.
message MediaIngressOpenResult {
  // Echo of MediaIngressOpen.client_stream_id for correlation.
  bytes  client_stream_id             = 1;

  // true = admitted → transitions to STREAMING once transport is established;
  // false = rejected (inspect reject_reason and reject_code).
  bool   admitted                     = 2;

  // Runtime-assigned stream epoch. Stable across transport reconnects within
  // the same session; used by MediaIngressState and reconnect reconciliation
  // (§3.6). 0 if rejected.
  uint64 stream_epoch                 = 3;

  // Runtime-assigned surface identity. For zone-bound streams, this is the
  // VideoSurfaceRef surface_id materialized into the zone (RFC 0005 A1).
  bytes  assigned_surface_id          = 4;

  // Codec selected from MediaIngressOpen.codec_preference. Must be one of
  // the agent's declared preferences. Populated when admitted=true.
  MediaCodec  selected_codec          = 5;

  // SDP offer from runtime, if the runtime is initiating SDP. Empty when
  // the agent provided the offer in MediaIngressOpen.transport (§4.2).
  // Transport carrier — inspect with §4.3 semantics; MUST NOT be interpreted
  // as raw SDP bytes without §9 SDP-security scrutiny.
  bytes  runtime_sdp_offer            = 6;

  // Populated when admitted=false.
  string reject_reason                = 7;   // Human-readable
  string reject_code                  = 8;   // Machine-readable; see §2.4
}
```

#### 2.3.3 `MediaIngressState` (ServerMessage field 61)

```protobuf
// State-stream class (coalescible). Carries per-stream health and degradation
// state. Latest state wins; intermediate states may be skipped under load.
message MediaIngressState {
  uint64 stream_epoch                 = 1;  // Correlating MediaIngressOpenResult
  MediaSessionState state             = 2;  // Current state machine state (§3)
  uint32 current_step                 = 3;  // Current E25 degradation step (0=none, 1–10=ladder, §5)
  uint32 effective_bitrate_kbps       = 4;
  uint32 effective_fps                = 5;  // 0 if video not present
  uint32 effective_width_px           = 6;  // 0 if video not present
  uint32 effective_height_px          = 7;  // 0 if video not present
  uint32 dropped_frames_since_last    = 8;
  uint32 watchdog_warnings            = 9;  // Monotonic; incremented on each watchdog warning
  uint64 sample_timestamp_wall_us     = 10;
}

enum MediaSessionState {
  MEDIA_SESSION_STATE_UNSPECIFIED = 0;
  ADMITTED                        = 1;
  STREAMING                       = 2;
  DEGRADED                        = 3;
  PAUSED                          = 4;
  CLOSING                         = 5;
  CLOSED                          = 6;  // Terminal
  REVOKED                         = 7;  // Terminal
}
```

#### 2.3.4 `MediaIngressCloseNotice` (ServerMessage field 62)

```protobuf
// Runtime-initiated termination notice. Delivered before stream teardown
// so the agent knows why the stream is ending. Always paired with a final
// MediaIngressState carrying state=CLOSED or REVOKED.
message MediaIngressCloseNotice {
  uint64 stream_epoch                 = 1;
  MediaCloseReason reason             = 2;
  string detail                       = 3;  // Human-readable context
}

enum MediaCloseReason {
  MEDIA_CLOSE_REASON_UNSPECIFIED = 0;
  AGENT_CLOSED              = 1;  // Echo of MediaIngressClose
  LEASE_REVOKED             = 2;  // Owning lease was revoked (RFC 0008 §3)
  CAPABILITY_REVOKED        = 3;  // media-ingress capability revoked (RFC 0008 A1 §A3.4)
  OPERATOR_MUTE             = 4;  // Human override (chrome mute, §5.5)
  POLICY_DISABLED           = 5;  // Runtime config disabled the capability at deployment level
  BUDGET_WATCHDOG           = 6;  // Per-stream watchdog threshold crossed (RFC 0002 A1 §A4.1)
  PREEMPTED                 = 7;  // Higher-priority stream preempted this one (RFC 0002 A1 §A3.2)
  DEGRADATION_TEARDOWN      = 8;  // E25 step 8 "Tear down media, keep session" reached
  EMBODIMENT_REVOKED        = 9;  // E25 step 9 reached; paired with RFC 0015 presence demote
  SESSION_DISCONNECTED      = 10; // E25 step 10 / session teardown
  TRANSPORT_FAILURE         = 11; // ICE / DTLS / SRTP fatal
  DECODER_FAILURE           = 12; // GStreamer pipeline unrecoverable
  SCHEDULE_EXPIRED          = 13; // expires_at_wall_us passed
}
```

#### 2.3.5 `MediaIngressClose` (ClientMessage field 61)

```protobuf
// Agent-initiated teardown. Idempotent w.r.t. already-terminated streams
// (runtime responds with a no-op close notice). Transactional.
message MediaIngressClose {
  uint64 stream_epoch                 = 1;
  string reason                       = 2;  // Optional, audit-only
}
```

#### 2.3.6 `MediaDegradationNotice` (ServerMessage field 65)

```protobuf
// Per-stream degradation notice. Delivered whenever the runtime advances or
// recedes this stream's position on the E25 ladder. Distinct from the
// global DegradationNotice (RFC 0005 field 46), which describes the
// runtime-level degradation level; this message describes the per-stream
// step applied to a specific media session.
message MediaDegradationNotice {
  uint64 stream_epoch                 = 1;
  uint32 ladder_step                  = 2;  // 0 = recovery/no-step, 1–10 = E25 step reached
  MediaDegradationTrigger trigger     = 3;  // Who/what triggered this step
  string detail                       = 4;
}

enum MediaDegradationTrigger {
  MEDIA_DEGRADATION_TRIGGER_UNSPECIFIED = 0;
  RUNTIME_LADDER_ADVANCE      = 1;  // Global runtime degradation level advanced (E25 automatic)
  WATCHDOG_PER_STREAM         = 2;  // Per-stream watchdog threshold (§5.3)
  OPERATOR_MANUAL             = 3;  // Human override at chrome
  CAPABILITY_POLICY           = 4;  // Capability/policy revocation forced a step
}
```

#### 2.3.7 Pause/Resume (fields 65/66 client, 67/68 server)

```protobuf
message MediaPauseRequest {
  uint64 stream_epoch                 = 1;
  string reason                       = 2;  // Audit-only
}

message MediaResumeRequest {
  uint64 stream_epoch                 = 1;
}

message MediaPauseNotice {
  uint64 stream_epoch                 = 1;
  MediaPauseTrigger trigger           = 2;
  string detail                       = 3;
}

message MediaResumeNotice {
  uint64 stream_epoch                 = 1;
  MediaPauseTrigger last_trigger      = 2;  // Which trigger caused the preceding pause
}

enum MediaPauseTrigger {
  MEDIA_PAUSE_TRIGGER_UNSPECIFIED = 0;
  AGENT_REQUEST               = 1;
  OPERATOR_REQUEST            = 2;  // Chrome pause affordance
  SAFE_MODE                   = 3;  // RFC 0005 §3.7 safe mode entry (all streams pause)
  POLICY_QUIET_HOURS          = 4;  // Attention policy (RFC 0009 level 4)
}
```

### 2.4 Reject / Close Code Registry

Machine-readable codes carried in `MediaIngressOpenResult.reject_code` and
auxiliary close reason strings. These align with existing RFC 0005 and RFC
0008 A1 code conventions (SHOUTY_SNAKE_CASE strings; matched to a typed enum
at the ErrorCode layer where possible).

| Code | Origin | Meaning |
|------|--------|---------|
| `CAPABILITY_REQUIRED` | RFC 0008 A1 §A2 | Session does not hold `media-ingress` |
| `CAPABILITY_DIALOG_DENIED` | RFC 0008 A1 §A6 | Operator denied capability dialog |
| `CAPABILITY_DIALOG_TIMEOUT` | RFC 0008 A1 §A6 | Dialog timed out; no operator present |
| `CAPABILITY_NOT_ENABLED` | RFC 0008 A1 §A6 | Capability disabled at deployment level |
| `CAPABILITY_NOT_IMPLEMENTED` | RFC 0008 A1 §A6 | e.g., `federated-send` in v2; `MediaEgressOpen` in v2 |
| `CODEC_UNSUPPORTED` | §2.5 | None of the declared codec preferences intersect the runtime set |
| `SURFACE_NOT_FOUND` | §2.3.1 | Zone or tile binding does not resolve |
| `SURFACE_OCCUPIED` | §2.3.1 | Surface already bound to another stream with incompatible policy |
| `POOL_EXHAUSTED` | RFC 0002 A1 §A2.2 | Media worker pool full; preemption not applicable |
| `SESSION_STREAM_LIMIT` | RFC 0002 A1 §A2.2 | Per-session `max_concurrent_media_streams` exceeded |
| `TEXTURE_HEADROOM_LOW` | RFC 0002 A1 §A2.2 | Global GPU texture budget below admission threshold; spawn deferred or rejected |
| `TRANSPORT_NEGOTIATION_FAILED` | §4 | SDP/ICE could not complete within transport timeout |
| `CONTENT_CLASS_DENIED` | RFC 0009 (privacy) | Viewer-class floor above declared `content_classification` |
| `SCHEDULE_INVALID` | RFC 0003 §3.5 | `present_at_wall_us` / `expires_at_wall_us` out of bounds |
| `INVALID_ARGUMENT` | RFC 0005 §3.5 | Malformed request |

### 2.5 Codec Negotiation

Per signoff packet D18: **H.264 (baseline/constrained profile) and VP9 for
v2**. AV1 is deferred to post-v2.

```protobuf
enum MediaCodec {
  MEDIA_CODEC_UNSPECIFIED = 0;

  // Video
  VIDEO_H264_BASELINE         = 1;  // H.264 constrained-baseline profile
  VIDEO_H264_MAIN             = 2;  // H.264 main profile (higher CPU; optional)
  VIDEO_VP9                   = 3;  // VP9 (software or hw via va/nvcodec/d3d11)
  VIDEO_AV1                   = 4;  // **Reserved for post-v2.** Runtime rejects.

  // Audio (E22)
  AUDIO_OPUS_STEREO           = 10; // Opus, stereo, 48kHz
  AUDIO_OPUS_MONO             = 11; // Opus, mono, 48kHz (fallback / microphone-ingress)
  AUDIO_PCM_S16LE             = 12; // Uncompressed PCM — test/debug path only, gated
}
```

**Negotiation algorithm (runtime side):**

1. Runtime maintains a per-deployment supported-codec set (populated from
   GStreamer pipeline capability probe at startup; see `gstreamer-media-pipeline-audit.md`
   §3 for the plugin license matrix and §5.3 for hardware-decode fallback).
2. On `MediaIngressOpen`, iterate `codec_preference` in declared order.
3. Pick the first codec that is in the supported set AND allowed by the
   session's capability scope AND allowed by the deployment config.
4. If no match: reject with `CODEC_UNSUPPORTED`, include supported-codec list
   in `reject_reason` for agent debugging.
5. `AUDIO_PCM_S16LE` is gated behind a `media.debug.allow_pcm` config flag;
   rejected by default in production.

**Hardware-decode fallback:** the runtime SHOULD probe for `va`/`nvcodec`/
`d3d11` hardware-decode elements at pipeline construction and transparently
fall back to software decode (`avdec_h264`, `vp9dec`) without exposing the
decision to the agent. The chosen backend is recorded in the
`MediaIngressState.effective_bitrate_kbps` accompanying telemetry and in
audit logs (§9.6), not in the protocol response.

**Plugin license matrix**: per `gstreamer-media-pipeline-audit.md` §3, the
LGPL plugin set covers H.264 + VP9; `plugins-ugly` carries patent-exposure
risk in some jurisdictions. The `media.codecs.allow_patent_risky` config
flag gates any use of plugins-ugly codecs. This flag is false by default.

**AV1 deferral**: `VIDEO_AV1` is wire-reserved. Runtime responds with
`CAPABILITY_NOT_IMPLEMENTED` if it appears in `codec_preference`. Removing
this restriction is a post-v2 decision.

### 2.6 TransportDescriptor

```protobuf
message TransportDescriptor {
  MediaTransportMode mode             = 1;

  // Agent-provided SDP offer, if the agent initiates (§4.2).
  // MUST pass §9 SDP security checks before the runtime parses it.
  bytes agent_sdp_offer               = 2;

  // Agent-provided ICE credentials for agent-initiated offer.
  repeated IceCredential agent_ice_credentials = 3;

  // Agent-requested relay mode hint (§2.7).
  RelayModeHint relay_hint            = 4;

  // Optional pre-shared SRTP key material. ONLY valid for transport modes
  // that require it (none in v2 phase 1). Rejected otherwise.
  bytes preshared_srtp_material        = 5;
}

enum MediaTransportMode {
  MEDIA_TRANSPORT_MODE_UNSPECIFIED = 0;
  WEBRTC_STANDARD                  = 1;  // Default: full WebRTC stack
  WEBRTC_PRECOMPOSITED_GLASSES     = 2;  // Phase 3 glasses; pre-composited frames upstream
  FUTURE_CLOUD_RELAY               = 3;  // **Reserved**: phase 4b cloud SFU. Rejected in v2 phase 1.
}

enum RelayModeHint {
  RELAY_MODE_HINT_UNSPECIFIED = 0;
  DIRECT                       = 1;  // ICE host / srflx; no relay
  RELAYED                      = 2;  // TURN relay allowed
  RUNTIME_RELAY_ONLY           = 3;  // Cloud-relay / SFU only (post-v2 enforcement path)
}
```

### 2.7 ZoneContent / VideoSurfaceRef relationship

Zone-bound media streams materialize as a `VideoSurfaceRef` in the target
zone's `active_publications`, per RFC 0005 Amendment A1. The `surface_id`
assigned in `MediaIngressOpenResult.assigned_surface_id` is the same
`surface_id` that appears in the zone's snapshot `VideoSurfaceRef` entry.
This is the materialization contract: snapshot carries **declarative
publication state**; wire signaling carries **transport state**.

---

## 3. Session Lifecycle State Machine

Per signoff packet E26, v2 state machines are implemented via the `statig`
crate plus a protobuf mirror carried in session traffic. This section
defines both.

### 3.1 States

| State | Description | Terminal? | Wire state (§2.3.3) |
|-------|-------------|-----------|---------------------|
| `ADMITTED` | `MediaIngressOpenResult.admitted = true` sent; SDP/ICE transport being established | No | `ADMITTED` |
| `STREAMING` | Transport established; decoded frames flowing through ring buffer to compositor | No | `STREAMING` |
| `DEGRADED` | Stream active but running below nominal quality due to E25 ladder step | No | `DEGRADED` |
| `PAUSED` | Stream admitted and previously streaming; frames suspended (by agent, operator, or safe mode) | No | `PAUSED` |
| `CLOSING` | Teardown initiated; DRAINING in worker lifecycle (RFC 0002 A1 §A1); final frames being consumed from ring buffer | No | `CLOSING` |
| `CLOSED` | Worker TERMINATED; all resources freed; stream cannot resume; agent may request fresh admission | Yes | `CLOSED` |
| `REVOKED` | Terminal failure path: capability revoked, lease revoked, or embodiment revoked. Distinct from `CLOSED` for audit clarity | Yes | `REVOKED` |

### 3.2 State Machine Diagram

```
                      ┌──────────────────────────────────┐
                      │  Admission gate (§2.3.1, §6.1)   │
                      │  - capability (RFC 0008 A1)      │
                      │  - pool slot (RFC 0002 A1 §A2.2) │
                      │  - codec match (§2.5)            │
                      │  - surface resolve (§2.3.1)      │
                      │  - content_class viewer-gate     │
                      └────────────┬─────────────────────┘
                                   │ all pass
                                   ▼
                          ┌──────────────────┐
               ┌─────────►│    ADMITTED      │ ◄── initial state on
               │          │                  │     MediaIngressOpenResult(admitted=true)
               │          └────────┬─────────┘
               │                   │ transport established
               │                   │ (SDP + ICE + DTLS/SRTP complete)
               │                   ▼
               │          ┌──────────────────┐                 ┌───────────────┐
               │          │    STREAMING     │◄──ladder step 0─┤    DEGRADED   │
               │          │                  │                 │               │
               │          │  (normal nominal │   E25 advance   │ (step N, 1–7) │
               │          │   quality)       │────────────────►│               │
               │          └────┬─────────────┘                 └───┬───────────┘
               │               │                                   │
               │               │ pause trigger        pause        │
               │               │                      trigger      │
               │               ▼                                   ▼
               │          ┌──────────────────────────────────────────┐
               │          │               PAUSED                     │
               │          │  (MediaPauseNotice reason: AGENT_REQUEST │
               │          │   | OPERATOR_REQUEST | SAFE_MODE |       │
               │          │   POLICY_QUIET_HOURS)                    │
               │          └────────────┬─────────────────────────────┘
               │                       │ resume trigger
               │                       │ (valid source for that pause's trigger)
               │                       ▼
               │                       (back to STREAMING or DEGRADED)
               │
               │  ┌────────────────────────────────────────────────┐
               │  │ any state except CLOSED/REVOKED:               │
               │  │   - agent: MediaIngressClose            ───────┼──► CLOSING
               │  │   - runtime: E25 step 8 DEGRADATION_TEARDOWN  │
               │  │   - runtime: schedule expired                 │
               │  │   - runtime: transport failure                │
               │  │   - runtime: decoder failure (watchdog-driven) │
               │  │   - runtime: budget watchdog threshold        │
               │  │   - runtime: pool preemption                  │
               │  └────────────────────────────────────────────────┘
               │                       │
               │                       ▼
               │            ┌──────────────────────┐
               │            │      CLOSING         │
               │            │                      │
               │            │ ring buffer drains;  │
               │            │ GStreamer EOS        │
               │            │ injected; worker in  │
               │            │ DRAINING (RFC 0002 A1) │
               │            └────────────┬─────────┘
               │                         │ ring buffer empty
               │                         │ AND EOS confirmed
               │                         ▼
               │                ┌────────────────────┐
               │                │      CLOSED        │
               │                │  (terminal)        │
               │                └────────────────────┘
               │
               │  ┌────────────────────────────────────────────────┐
               │  │ revocation paths (any state → REVOKED):        │
               │  │   - capability revoked mid-session (§A3.4 C13) │
               │  │   - lease revoked (RFC 0008 §3)                │
               │  │   - embodiment revoked (E25 step 9)             │
               │  │   - session disconnected with no grace reclaim  │
               │  │   - policy disabled at deployment level        │
               │  └────────────────────────────────────────────────┘
               │                         │
               │                         ▼
               │                ┌────────────────────┐
               │                │      REVOKED       │
               │                │  (terminal)        │
               │                └────────────────────┘
               │
               │  (agent may request a fresh MediaIngressOpen if terminal;
               │   a new stream_epoch is issued; prior epoch is never reused)
               │
               └── (reconnect path; §3.6)
```

### 3.3 Transitions

Formal transition table. The "Actor" column identifies the party that causes
the transition: `R` = runtime-automatic, `W` = watchdog (runtime-initiated via
threshold crossing), `O` = operator (human override), `A` = agent request.

| From | To | Actor | Trigger | Wire signal | Notes |
|------|----|-------|---------|-------------|-------|
| (start) | `ADMITTED` | R | `MediaIngressOpenResult.admitted=true` | Field 60 | §6.1 admission gate passed |
| (start) | (rejected; no state) | R | `admitted=false` | Field 60 | Reject codes in §2.4 |
| `ADMITTED` | `STREAMING` | R | transport established (SDP + ICE + DTLS/SRTP) | `MediaIngressState(STREAMING)` | §4 signaling complete |
| `ADMITTED` | `CLOSING` | R | transport negotiation failed within transport_timeout (default 10s) | `MediaIngressCloseNotice(TRANSPORT_FAILURE)` | §4.4 |
| `STREAMING` | `DEGRADED` | R | E25 ladder step 1–7 advanced on this stream | `MediaDegradationNotice` | §5 |
| `DEGRADED` | `STREAMING` | R | E25 recovery (frame-time guardian under threshold; budget recovered) | `MediaDegradationNotice(ladder_step=0)` | §5.4 hysteresis |
| `STREAMING` / `DEGRADED` | `PAUSED` | A | `MediaPauseRequest` | `MediaPauseNotice(AGENT_REQUEST)` | |
| `STREAMING` / `DEGRADED` | `PAUSED` | O | operator chrome pause | `MediaPauseNotice(OPERATOR_REQUEST)` | |
| `STREAMING` / `DEGRADED` | `PAUSED` | R | RFC 0005 §3.7 safe mode entry | `MediaPauseNotice(SAFE_MODE)` | all active media streams pause |
| `STREAMING` / `DEGRADED` | `PAUSED` | R | attention policy (RFC 0009 level 4) quiet hours | `MediaPauseNotice(POLICY_QUIET_HOURS)` | applies per-stream by content_classification |
| `PAUSED` | `STREAMING` | A | `MediaResumeRequest` | `MediaResumeNotice(AGENT_REQUEST)` | allowed only if original pause trigger was `AGENT_REQUEST` |
| `PAUSED` | `STREAMING` | O | operator chrome resume | `MediaResumeNotice(OPERATOR_REQUEST)` | clears any pause trigger |
| `PAUSED` | `STREAMING` | R | safe mode exit | `MediaResumeNotice(SAFE_MODE)` | paired with RFC 0005 `SessionResumed` |
| `PAUSED` | `STREAMING` | R | quiet-hours window closed | `MediaResumeNotice(POLICY_QUIET_HOURS)` | |
| (any non-terminal) | `CLOSING` | A | `MediaIngressClose` | `MediaIngressCloseNotice(AGENT_CLOSED)` | |
| (any non-terminal) | `CLOSING` | R | schedule expired (`expires_at_wall_us`) | `MediaIngressCloseNotice(SCHEDULE_EXPIRED)` | |
| (any non-terminal) | `CLOSING` | R | transport failure | `MediaIngressCloseNotice(TRANSPORT_FAILURE)` | |
| (any non-terminal) | `CLOSING` | W | watchdog threshold crossed (RFC 0002 A1 §A4.1) | `MediaIngressCloseNotice(BUDGET_WATCHDOG)` | |
| (any non-terminal) | `CLOSING` | W | decoder pipeline failure | `MediaIngressCloseNotice(DECODER_FAILURE)` | |
| (any non-terminal) | `CLOSING` | R | pool preemption (RFC 0002 A1 §A3.2) | `MediaIngressCloseNotice(PREEMPTED)` | |
| (any non-terminal) | `CLOSING` | R | E25 step 8 DEGRADATION_TEARDOWN | `MediaIngressCloseNotice(DEGRADATION_TEARDOWN)` | |
| `CLOSING` | `CLOSED` | R | ring buffer drained AND pipeline EOS confirmed | `MediaIngressState(CLOSED)` | drain_timeout default 500ms; force-clear on timeout |
| (any non-terminal) | `REVOKED` | O | capability revoked mid-session (RFC 0008 A1 §A3.4) | `MediaIngressCloseNotice(CAPABILITY_REVOKED)` → `MediaIngressState(REVOKED)` | |
| (any non-terminal) | `REVOKED` | R | owning lease revoked (RFC 0008 §3) | `MediaIngressCloseNotice(LEASE_REVOKED)` | |
| (any non-terminal) | `REVOKED` | R | embodiment revoked (E25 step 9) | `MediaIngressCloseNotice(EMBODIMENT_REVOKED)` | RFC 0015 presence demote |
| (any non-terminal) | `REVOKED` | R | session disconnected, grace period expired | `MediaIngressCloseNotice(SESSION_DISCONNECTED)` | RFC 0005 §6 grace |
| (any non-terminal) | `REVOKED` | O | policy disabled at deployment level during session | `MediaIngressCloseNotice(POLICY_DISABLED)` | |

### 3.4 Composition with session-level state

The media session lifetime is strictly subordinate to the owning agent
session:

- If the owning session transitions to `Closed` (RFC 0005 §1.1), all its
  media sessions transition to `REVOKED` after the session reconnect grace
  period expires (RFC 0005 §6), unless the session was suspended (safe
  mode), in which case media sessions transition to `PAUSED` and remain
  paused until session resumption or max suspension timeout.
- A safe-mode-triggered `PAUSED` → `STREAMING` transition MUST NOT require
  the agent to re-admit; the stream_epoch is preserved and the transport
  descriptor may be reused if still valid. If the transport is stale, the
  runtime sends a fresh `MediaSdpOffer` as part of resume.
- On agent `SessionResume` within grace period (RFC 0005 §6.3), media
  sessions are delivered in the snapshot's `active_publications` (zone-bound
  streams) and the agent MUST reconcile stream_epochs per §3.6.

### 3.5 `statig` implementation guidance

Per E26, state machines in v2 use the `statig` crate. The state machine in
§3.2 maps to a `statig` hierarchical state machine with:

- top-level states: `ADMITTED`, `STREAMING`, `DEGRADED`, `PAUSED`, `CLOSING`,
  `CLOSED`, `REVOKED`.
- `STREAMING` and `DEGRADED` share a parent superstate `ACTIVE` whose guard
  ensures transport is healthy; pause triggers fire on the parent.
- `CLOSED` and `REVOKED` are terminal; the `statig` machine rejects any
  post-terminal transition.
- transitions carry a typed event enum mirroring the §3.3 "Trigger" column.

The implementation crate MUST serialize the state onto the wire using
`MediaSessionState` (§2.3.3) on every `MediaIngressState` emission, so the
agent's local mirror converges with the runtime's authoritative state.

### 3.6 Reconnect Semantics

On accepted `SessionResume` (RFC 0005 §6.3):

1. Runtime sends `SessionResumeResult`, then `SceneSnapshot` (unchanged).
2. `SceneSnapshot.snapshot_json` MUST include media publications in
   `zone_registry.active_publications` with `VideoSurfaceRef`,
   `expires_at_wall_us`, and `content_classification` (per RFC 0005 A1 §A4).
3. For each active media stream, the runtime emits the latest
   `MediaIngressState` carrying the current `stream_epoch`.
4. The agent reconciles:
   - If its stored `stream_epoch` matches the runtime's: stream continues;
     no re-admit required. Transport may be stale — if ICE/DTLS/SRTP state
     is lost, the runtime sends a fresh `MediaSdpOffer` and the agent
     replies with `MediaSdpAnswer` to re-establish the transport plane
     without re-admitting the stream.
   - If its stored `stream_epoch` differs or is absent: the agent MUST
     issue a fresh `MediaIngressOpen`. The runtime assigns a new
     `stream_epoch`; the old one is never reused.
5. No WAL/delta replay is introduced for media. Snapshot-first reconnect is
   sufficient for v2 phase 1.

---

## 4. Media Signaling

### 4.1 Posture

Media signaling rides the session envelope, not a separate transport. The
`MediaSdpOffer` / `MediaSdpAnswer` / `MediaIceCandidate` messages are
protobuf-wrapped carriers for the underlying WebRTC handshake artifacts —
but the wrapper is mandatory. Raw SDP blobs MUST NOT be emitted or consumed
outside the wrapper, and the wrapper is subject to §9 security scrutiny.

### 4.2 Offer/Answer Direction

Both offer-in-client-open and offer-from-runtime patterns are supported:

- **Agent-initiated offer:** agent puts an SDP offer in
  `TransportDescriptor.agent_sdp_offer` on `MediaIngressOpen`. Runtime
  validates and, on admission, emits `MediaIngressOpenResult` with an SDP
  answer carried in the result (TBD: extend §2.3.2 with
  `runtime_sdp_answer`; phase-1 implementation may choose to return the
  answer in `MediaIngressOpenResult.runtime_sdp_offer` semantically as an
  "answer" when the agent offered — clarity improvement owned by
  hud-ora8.1.23's proto wiring task).
- **Runtime-initiated offer:** agent omits `agent_sdp_offer`. Runtime emits
  `MediaSdpOffer` (field 63) after admission. Agent replies with
  `MediaSdpAnswer` (field 62).

Exactly one pattern is used per stream. Mixing is rejected with
`TRANSPORT_NEGOTIATION_FAILED`.

Default for v2 phase 1: **runtime-initiated offer** for embodied/tile-bound
streams; **agent-initiated offer** for zone-bound media where the agent
has an existing decoder pipeline and supplies the transport. The choice is
driven by the session's presence level and the surface binding.

### 4.3 Transport Descriptor Semantics

`TransportDescriptor` (§2.6) is the structured envelope for all transport
plumbing. It carries:

- `mode`: `WEBRTC_STANDARD` (default), `WEBRTC_PRECOMPOSITED_GLASSES`
  (phase 3; glasses pipeline expects pre-composited frames upstream),
  `FUTURE_CLOUD_RELAY` (phase 4b; rejected in phase 1).
- `agent_sdp_offer`: structured SDP bytes, carried inside protobuf so the
  transport is auditable and subject to the §9 security envelope.
- `agent_ice_credentials`: repeated ICE credential (ufrag/pwd) and role
  assignment; structured, not free-form SDP attributes.
- `relay_hint`: `DIRECT`, `RELAYED`, or `RUNTIME_RELAY_ONLY` (post-v2).
- `preshared_srtp_material`: rejected in phase 1; reserved for future
  closed-transport modes (phase 4 cloud-relay trust boundary may use it).

### 4.4 Transport Timeouts and Failure

- `transport_timeout` (default 10s): the wall-clock duration the runtime
  waits for transport to complete from `ADMITTED` to `STREAMING`. Expiry
  triggers `ADMITTED → CLOSING` with `TRANSPORT_FAILURE`.
- ICE trickle is supported; `MediaIceCandidate` (fields 63/64) messages
  flow both directions during transport establishment.
- DTLS/SRTP key material is negotiated inside the WebRTC stack; tze_hud
  does not log or expose SRTP master keys.
- On fatal transport failure (DTLS handshake fail, ICE gathering timeout,
  SRTP auth fail), runtime transitions stream to `CLOSING`
  (`TRANSPORT_FAILURE`). The agent may retry by issuing a fresh
  `MediaIngressOpen`, subject to the admission gate.

### 4.5 Security Analysis of Signaling

(Also referenced from §9.)

SDP and signaling artifacts carry material that MUST be scrutinized before
the runtime trusts them:

1. **SDP parser exposure.** Raw SDP parsing is a known CVE surface
   historically (see §9.3 for CVE posture). The runtime's SDP parser runs on
   the trusted side of the gRPC wire; it still receives attacker-controlled
   bytes. The parser used MUST be hardened (fuzzed) and bounded in size and
   complexity (max SDP body bytes, max media descriptions, max attribute
   lines, max ICE candidates). These limits are enforced at the wrapper
   layer before the SDP bytes reach the WebRTC stack.
2. **ICE candidate address lists.** Agent-provided ICE candidates can enumerate
   address lists. The runtime MUST filter candidates against a runtime
   allow-list (by default: no explicit disallow; operator config may add
   a block-list) and bound the total candidate count per stream.
3. **DTLS fingerprint trust model.** In v2 phase 1 (bounded ingress from
   trusted agents only), DTLS fingerprints are trusted on first use (TOFU)
   per session. Per-agent fingerprint pinning is deferred to phase 4b
   (cloud-relay trust boundary).
4. **SRTP master keys.** Never cross the gRPC wire. Derived inside the
   WebRTC stack after DTLS handshake. Not logged.
5. **Out-of-band control.** No bandwidth or pause negotiation happens via
   SDP; all transitions go through the `MediaPause*`/`MediaResume*` wire.
   SDP `b=` lines are advisory and capped by the session's bitrate budget.

### 4.6 Signaling Size Bounds

Per RFC 0005 Amendment A1 §A2 (traffic-class contract), media signaling
payloads share the session stream with ordinary v1 traffic. To prevent
head-of-line blocking:

- `MediaIngressOpen.TransportDescriptor.agent_sdp_offer` MAX 16 KiB.
- `MediaSdpOffer.sdp_bytes` MAX 16 KiB.
- `MediaSdpAnswer.sdp_bytes` MAX 16 KiB.
- `MediaIceCandidate.candidate_str` MAX 512 bytes.
- `MediaIngressOpen.codec_preference` MAX 16 entries.
- `TransportDescriptor.agent_ice_credentials` MAX 8 entries.

Oversize payloads are rejected with `INVALID_ARGUMENT`.

---

## 5. Degradation Mechanism

### 5.1 Relationship to E25

The E25 degradation ladder is defined in `about/heart-and-soul/failure.md`
§"E25 degradation ladder" as the authoritative ordered list:

1. Degrade spatial audio
2. Reduce framerate
3. Reduce resolution
4. Suspend recording
5. Drop cloud-relay
6. Drop second stream
7. Freeze and block input
8. Tear down media, keep session
9. Revoke embodied presence
10. Disconnect

This RFC specifies the **mechanism** that translates those ordered steps into
per-stream wire signaling. The doctrine order and the "never agent-initiated"
rule are invariants; this RFC must not relax them.

### 5.2 Step-to-mechanism mapping

| E25 step | Triggered by | Per-stream wire signal | Per-stream wire state |
|----------|--------------|------------------------|-----------------------|
| 1. Degrade spatial audio | R | `MediaDegradationNotice(step=1, trigger=RUNTIME_LADDER_ADVANCE)` | `STREAMING → DEGRADED` if not already |
| 2. Reduce framerate | R | `MediaDegradationNotice(step=2, …)` + `MediaIngressState.effective_fps` updated | `STREAMING → DEGRADED` |
| 3. Reduce resolution | R | `MediaDegradationNotice(step=3, …)` + `MediaIngressState.effective_width_px/height_px` updated | `STREAMING → DEGRADED` |
| 4. Suspend recording | R | (recording-plane concern; RFC 0017). Media plane emits `MediaDegradationNotice(step=4, …)` for correlated observability | (no media state change) |
| 5. Drop cloud-relay | R | (cloud-relay concern; RFC 0018). Media plane emits `MediaDegradationNotice(step=5, …)` | cloud-relayed streams transition `STREAMING → CLOSING` |
| 6. Drop second stream | R | `MediaIngressCloseNotice(DEGRADATION_TEARDOWN)` on the lowest-priority non-primary stream per session | stream → `CLOSING` |
| 7. Freeze and block input | R | Streams do not teardown; compositor freezes presentation. `MediaDegradationNotice(step=7)` on all active streams | `STREAMING → DEGRADED` |
| 8. Tear down media, keep session | R | `MediaIngressCloseNotice(DEGRADATION_TEARDOWN)` for all remaining streams | streams → `CLOSING` |
| 9. Revoke embodied presence | R | `MediaIngressCloseNotice(EMBODIMENT_REVOKED)` on all embodied-owned streams + RFC 0015 presence demote | streams → `REVOKED` |
| 10. Disconnect | R | `MediaIngressCloseNotice(SESSION_DISCONNECTED)` on all streams + RFC 0005 session close | streams → `REVOKED` |

Steps 1–3 modify stream quality without teardown. Steps 4–5 are noted
(observability) but owned by their respective planes. Step 6 sheds
non-primary streams. Step 7 freezes presentation. Step 8 tears down all
media while preserving the session. Steps 9–10 are terminal for
embodied/session.

### 5.3 Trigger Authority

Exactly aligned with E25 trigger semantics and RFC 0002 A1 §A4:

| Trigger kind | Who | What | Per-stream notice |
|-------------|-----|------|-------------------|
| Runtime-automatic | R | Global degradation level advances (RFC 0002 §6.2 frame-time guardian or budget breach) | `MediaDegradationNotice(RUNTIME_LADDER_ADVANCE)` |
| Watchdog-automatic | W | Per-stream watchdog threshold (RFC 0002 A1 §A4.1: CPU / GPU texture / ring buffer / decoder lifetime) crossed | `MediaDegradationNotice(WATCHDOG_PER_STREAM)`; may advance this stream's step without advancing the runtime ladder |
| Operator-manual | O | Operator chrome affordance (mute, pause, revoke) | `MediaDegradationNotice(OPERATOR_MANUAL)` |
| Capability/policy | R (policy) | Capability revoked mid-session (RFC 0008 A1 §A3.4); quiet-hours policy fires | `MediaDegradationNotice(CAPABILITY_POLICY)` |

**Agent-initiated degradation is refused.** If an agent emits a
`MediaDegradationNotice`-shaped message client→server (there is no such
ClientMessage field in §2.2), the runtime rejects with `INVALID_ARGUMENT`.
If an agent requests teardown, it may use `MediaIngressClose` (its own
stream only). It may not request any step on any other stream.

### 5.4 Recovery / Hysteresis

Per RFC 0002 §6.3 hysteresis:

- Recovery to `step=0` (nominal) is driven by the same frame-time guardian
  and budget observer that drove the advance.
- Recovery is subject to the same hysteresis thresholds as the runtime
  degradation level — the runtime does not immediately recede on transient
  improvement.
- On recovery, the runtime emits `MediaDegradationNotice(ladder_step=0, …)`
  and transitions `DEGRADED → STREAMING`.
- Per-stream watchdog-triggered DRAINING is NOT a candidate for recovery;
  the stream stays in `CLOSING → CLOSED` once watchdog fires. The agent
  may request a fresh stream.

### 5.5 Operator Override (Human Override Path)

Per security.md §"Human override" (Level 0 arbitration), the operator may at
any time:

- **Mute a stream.** Wire: `MediaIngressCloseNotice(OPERATOR_MUTE)` →
  stream → `CLOSING`. The "mute" affordance is logically a teardown; the
  operator's intent is "make this stream stop now". Differs from `PAUSE`
  (reversible) by the chrome affordance used.
- **Pause a stream.** Wire: `MediaPauseNotice(OPERATOR_REQUEST)` →
  stream → `PAUSED`.
- **Resume a paused stream.** Wire: `MediaResumeNotice(OPERATOR_REQUEST)`.
- **Revoke the capability.** Wire: `MediaIngressCloseNotice(CAPABILITY_REVOKED)`
  on all affected streams; triggers RFC 0008 A1 §A3.4 revocation.

All operator overrides land at Level 0 in the arbitration stack (RFC 0009
§1) and cannot be intercepted, delayed, or vetoed by any agent or policy.

---

## 6. Worker Pool Interface

Per E24 (in-process tokio task shared pool) and RFC 0002 Amendment A1
(media worker lifecycle), the runtime maintains a shared worker pool with
priority-based preemption. This RFC specifies the protocol-visible surface
of that pool.

### 6.1 Admission Gate (wire-observable)

Admission is evaluated in the order mandated by RFC 0002 A1 §A2:

1. **Capability gate** (§A2.1): `media-ingress` granted with dialog / 7-day
   remember passage per RFC 0008 A1 §A2.
2. **Budget headroom** (§A2.2): pool slot available; per-session stream
   limit (`max_concurrent_media_streams`, default 1); global GPU texture
   headroom ≥ 128 MiB.
3. **Role authority** (§A2.3): capability grant authorized by `owner` or
   `admin` role per RFC 0009 A1.

Any failure short-circuits with the corresponding `reject_code` from §2.4.

### 6.2 Preemption Protocol

When a higher-priority stream requests admission and the pool is full, the
runtime may preempt per RFC 0002 A1 §A3.2:

1. Preempted stream receives `MediaIngressCloseNotice(PREEMPTED, detail=<priority comparison>)`.
2. Preempted stream transitions `STREAMING/DEGRADED → CLOSING`.
3. Incoming stream is admitted immediately (does not wait for preempted's
   `CLOSING → CLOSED` to complete).

Preemption eligibility per RFC 0002 A1 §A3.2: incoming lease priority
strictly higher than existing; no same-priority preemption.

### 6.3 Watchdog-Driven Termination

Per RFC 0002 A1 §A4.1, the budget watchdog observes per-worker resources:

- CPU time (rolling 10s window)
- GPU texture occupancy
- Ring-buffer occupancy (75% sustained for 30 frames)
- Decoder lifetime (24h force-recycle)
- Leases held (per-agent envelope)

On threshold crossing, the worker transitions to DRAINING (equivalent to
wire state `CLOSING` for its stream). Wire signal:
`MediaIngressCloseNotice(BUDGET_WATCHDOG, detail=<which threshold>)`.

A watchdog-triggered close does NOT automatically advance the global
degradation level. It is per-stream.

### 6.4 Pool Contraction Under Budget Pressure

Per RFC 0002 A1 §A3.3, at runtime degradation Level 2+ the effective pool
size contracts to `media.worker_pool_size_max_budget_pressure` (default 1).
Wire-observable:

- No new admissions above contracted limit; reject with `POOL_EXHAUSTED`.
- Existing streams run to natural close; on close, slot is NOT re-issued
  while pressure persists.
- Pool expansion on pressure clearance is not announced over the wire;
  agents that were rejected may retry.

---

## 7. Audio Stack

### 7.1 Contract

Per signoff packet E22:

- **Codec: Opus.**
- **Channels: stereo (or mono for microphone-ingress).**
- **Sample rate: 48 kHz canonical.**
- **Default output device: operator-selected at first run, sticky, changeable via config.**
- **Runtime-owned routing.**
- **Spatial audio: phase 4 refinement (not in phase 1).**

### 7.2 Wire representation

Audio is carried as a track inside the same `MediaIngressOpen` / WebRTC
transport as video. The codec enum (§2.5) includes `AUDIO_OPUS_STEREO` and
`AUDIO_OPUS_MONO`. The `MediaIngressOpen.has_audio_track` boolean is
informational; actual per-track codec selection happens via the codec
preference list and SDP negotiation.

Audio-only streams (e.g., microphone-ingress path under the
`microphone-ingress` capability) open with `has_video_track = false` and a
codec preference containing only `AUDIO_OPUS_*` entries.

### 7.3 Audio device binding

Runtime-owned per cpal audit (`cpal-audio-io-crate-audit.md`):

- On startup, runtime picks the operator-selected sticky default output
  device (or first-run selection UI on fresh installs).
- WASAPI default-device-change tracking (cpal audit §4): runtime registers
  an `IMMNotificationClient` listener and rebuilds the stream on
  `OnDefaultDeviceChanged`. This is a transport-layer concern not visible
  on the wire, but it is the mechanism that honors E22's "sticky" contract.
- Audio device selection is not an agent-negotiable parameter on the wire.
  Agents request an audio track; the runtime routes it.

### 7.4 Audio latency budget

Glass-to-glass latency budgets from D18 are video-oriented (p50 ≤150ms /
p99 ≤400ms). Audio-specific targets:

- **Audio latency target (phase 1):** ≤ 50ms mouth-to-ear under shared-mode
  WASAPI / ALSA / PulseAudio / CoreAudio. Exclusive-mode paths (WASAPI
  exclusive via ASIO) are post-v2.
- **Lip-sync drift:** ±40 ms (D18).

### 7.5 Microphone ingress (post-v1 capability)

The `microphone-ingress` capability (RFC 0008 A1 §A1) permits an agent to
receive captured audio from a local microphone device. It uses the same wire
shape as video ingress, with `has_video_track = false` and an audio-only
codec preference. Privacy, operator-visible indicator, and capability
dialog all apply per RFC 0008 A1.

### 7.6 Audio emit (phase 4)

`audio-emit` (agent-authored audio output) is phase 4 scope. The
`MediaEgressOpen` / `MediaEgressOpenResult` wire fields are reserved
(§2.2.1 field 64, §2.2.2 field 66) but runtime rejects with
`CAPABILITY_NOT_IMPLEMENTED` in v2 phase 1. Full egress wire is owned by a
forthcoming phase-4 RFC (likely RFC 0017 covers audit; phase-4f bidi AV
owns its own RFC, TBD).

---

## 8. Relationships to Prior Documents

### 8.1 RFC 0005 Session Protocol + Amendment A1

RFC 0005 §2.1 reserves envelope fields 50–99 for post-v1 use. Amendment
A1 allocated `MediaIngress*` to fields 50–52 (server) and 50–51 (client).
This RFC relocates those allocations to fields 60–79 (see §2.2 erratum) to
avoid the collision with persistent-movable-elements' use of fields 50–51.

Preserved from RFC 0005 and its amendments (per signoff packet F29):

- `WidgetPublishResult.request_sequence` (field 1 of `WidgetPublishResult`;
  ServerMessage field 47). This RFC does not touch `WidgetPublishResult`
  in any way.
- `ZonePublishResult.request_sequence` and other Layer 3 extension
  semantics. Unchanged by this RFC.
- RFC 0005 A1 Protected Fields list (§"Protected Fields — Preservation
  Guarantee"). Unchanged by this RFC.

Traffic class contract preserved:

- `MediaIngressOpen`, `MediaIngressOpenResult`, `MediaIngressCloseNotice`,
  `MediaIngressClose`, `MediaSdpOffer`, `MediaSdpAnswer`, `MediaPause*`,
  `MediaResume*`, `MediaDegradationNotice`: **Transactional.**
- `MediaIngressState`: **State-stream** (coalescible, latest-wins).
- `MediaIceCandidate`: **Ephemeral realtime** (latest-wins per candidate
  family; drop-tolerant; high-rate).

### 8.2 RFC 0002 Runtime Kernel + Amendment A1

RFC 0002 A1 §A1 defines the worker state machine (SPAWNING → RUNNING →
DRAINING → TERMINATED; FAILED terminal). This RFC's wire state machine
(§3) runs at a higher abstraction level: it describes the **agent-observable
media session** state, not the per-worker internal state. Mapping:

| Worker state (RFC 0002 A1) | Media session state (this RFC) |
|-----------------------------|----------------------------------|
| (pre-spawn gate) | (pre-`ADMITTED`; admission gate evaluation) |
| SPAWNING | `ADMITTED` (transport being established) |
| RUNNING (transport healthy) | `STREAMING` or `DEGRADED` |
| RUNNING (paused) | `PAUSED` |
| DRAINING | `CLOSING` |
| TERMINATED | `CLOSED` |
| FAILED | `REVOKED` or `CLOSED` with `TRANSPORT_FAILURE`/`DECODER_FAILURE` |

This RFC MUST NOT introduce a worker state skip (e.g., jumping from
SPAWNING to TERMINATED without DRAINING). RFC 0002 A1 §A1 state invariants
hold.

GPU device ownership (RFC 0002 §2.8 + A1 §A5.2) is unchanged: compositor
thread is sole wgpu owner; media workers never access GPU directly.

`DecodedFrameReady` channel (RFC 0002 §2.6): 4-slot ring buffer per stream,
drop-oldest. Unchanged by this RFC.

Cross-agent isolation via `session_id` tagging on `DecodedFrameReady`:
unchanged by this RFC.

### 8.3 RFC 0008 Lease Governance + Amendment A1

RFC 0008 A1 introduces the C13 capability dialog + 7-day remember for eight
v2 capabilities including `media-ingress`, `microphone-ingress`, and
`audio-emit`. This RFC's admission gate (§6.1) defers to the dialog gate
defined in RFC 0008 A1 §A4.

When a lease's `media-ingress` capability is revoked mid-session (RFC 0008
A1 §A3.4), all media streams owned by that lease transition to `REVOKED`
with `MediaIngressCloseNotice(CAPABILITY_REVOKED)`. If the capability was
the only grant in a lease's scope, the lease itself is revoked per RFC 0008
A1 §A3.4.

### 8.4 RFC 0009 Policy Arbitration + Amendment A1

RFC 0009 A1 (C12 role-based operators) governs who may grant or revoke
media capabilities. The admission gate (§6.1 step 3) re-checks role
authority as defense-in-depth.

Arbitration stack levels relevant to media:

- Level 0 (Human override): operator mute, pause, revoke. Immediate.
- Level 1 (Safety): safe mode pauses all streams; GPU failure triggers
  step 8 tear-down.
- Level 2 (Privacy): `content_classification` vs viewer class filter
  (`CONTENT_CLASS_DENIED` reject on admission; redaction/mute at runtime).
- Level 3 (Security): capability / lease / session identity enforcement.
- Level 4 (Attention): quiet-hours pause (`POLICY_QUIET_HOURS`).
- Level 5 (Resource): budget watchdog, E25 ladder trigger.
- Level 6 (Content): zone contention for zone-bound media (runs under
  the zone contention policy unchanged from RFC 0005 §3.1).

### 8.5 Cross-pillar reference

Per signoff packet §"Cross-pillar impact matrix":

- **heart-and-soul:** `media-doctrine.md` is the doctrine precedent; this
  RFC is its mechanism layer. `failure.md` E25 amendment is the
  authoritative ladder; §5 is its wire surface.
- **legends-and-lore:** RFC 0002 A1, RFC 0005 A1, RFC 0008 A1, RFC 0009 A1
  as listed. Forthcoming RFC 0015, 0017, 0018 extend additively.
- **lay-and-land:** `components.md` entries for media-worker-pool (E24),
  audio-routing subsystem (E22), recording store (phase 4a), cloud-relay
  (phase 4b). Owned by F31 task.
- **craft-and-care:** D18 performance budgets (glass-to-glass p50 ≤150 ms,
  decode drop ≤0.5%, lip-sync drift ≤±40 ms, TTFF ≤500 ms) flow into
  `engineering-bar.md`; D21 tier gates promoted as release gate. Owned
  by F32 task.
- **openspec:** `media-plane/spec.md` is the phase-1 capability spec;
  it references this RFC normatively.

---

## 9. Security Considerations

### 9.1 Capability Gate Integrity

The admission gate (§6.1) is the primary enforcement point. Defense-in-depth
layering:

1. RFC 0008 A1 §A2: capability must be session-granted AND dialog-passed
   OR 7-day remembered.
2. RFC 0009 A1 §A1.3: grant authorized by `owner` or `admin` role.
3. RFC 0002 A1 §A2.3: role authority re-checked at worker spawn.
4. Runtime config: `capabilities.<token>.enabled = true` at deployment level.

Bypass of any layer must fail closed (capability absent → `CAPABILITY_REQUIRED`).

### 9.2 SDP/Signaling Security Envelope

Per §4.5:

- **Parser hardening.** The SDP parser used by the runtime (within the
  WebRTC stack, typically `webrtc-rs`) MUST be fuzzed. Phase-1 CI lane
  should include an SDP fuzz corpus checked into LFS with the reference
  streams.
- **Size / complexity limits.** §4.6 bounds each signaling payload at a
  small multiple of the expected production size.
- **ICE candidate filtering.** Runtime rejects candidates pointing at
  reserved / loopback / link-local ranges unless the operator has
  opted into a LAN-device allow-list (phase 3 glasses pairing concern).
- **DTLS fingerprint posture.** TOFU per session in v2 phase 1. Per-agent
  fingerprint pinning is a phase 4b cloud-relay hardening.
- **SRTP keys never on wire.** Derived inside WebRTC stack post-DTLS.
- **No raw SDP in audit logs.** Audit entries record the signaling event
  plus structured metadata (codec chosen, `stream_epoch`, size) but not
  the raw SDP body. Debug logs MAY include SDP under a dedicated
  `media.log.debug_sdp` flag, disabled by default.

### 9.3 Codec CVE Surface

Codec implementations historically accumulate CVEs (H.264 stack, VP9 stack,
Opus occasional). Mitigations layered here:

- Plugin license matrix governed at deployment (`media.codecs.allow_patent_risky`
  for plugins-ugly; disabled by default).
- CVE surface tracked under the defense-in-depth backlog item `hud-lezjj`
  (codec CVE tracking / subprocess isolation option). **This RFC does not
  introduce subprocess isolation**; that is a post-v2 hardening item per
  security.md §"In-process media and runtime workers".
- Hardware-decode fallback path (`va`/`nvcodec`/`d3d11` → software
  `avdec_h264`/`vp9dec`) reduces reliance on any single decoder plugin.
- Upstream GStreamer + gstreamer-rs version pinning and update cadence
  governed by the engineering bar F32 item (dependency hygiene).

### 9.4 Cross-Agent Isolation

Per security.md §"Agent isolation" and RFC 0002 A1 §A5.1:

- `DecodedFrameReady` is tagged with owning `session_id`.
- Compositor thread refuses to blit a frame tagged with session A's
  `session_id` into session B's tile.
- Zone-bound media materialize into a zone's `active_publications`
  namespaced by the owning `session_id` for snapshot / reconnect purposes.
- Pool slot sharing does NOT weaken isolation; it is no different from
  sharing the gRPC server's tokio runtime.

### 9.5 Denial-of-service Surface

Sources and mitigations:

- **Signaling flood.** Bounded by traffic-class contract + per-session
  signaling rate limit (implementation-defined; suggested 10 opens/s per
  session).
- **SDP parser DoS.** Size/complexity bounds in §4.6.
- **ICE candidate storm.** Candidate-count limit in §4.6.
- **Pool exhaustion by bogus admissions.** Pool has admission gate; per-session
  stream cap; budget watchdog. Repeated failed admissions subject to the
  same resource governance (warning → throttle → revocation cascade per
  security.md §"Resource governance").
- **Per-stream watchdog.** CPU/GPU texture/ring buffer thresholds per RFC
  0002 A1 §A4.1; sustained threshold crossing terminates the offending
  stream without cascading.
- **Decoder lifetime cap.** 24h force-recycle per RFC 0002 A1 §A4.1
  prevents long-running GStreamer resource leaks.

### 9.6 Audit Events

Per signoff packet C17 (mandatory audit events) and RFC 0009 §13.3 audit
log infrastructure, media plane emits:

| Event | Trigger |
|-------|---------|
| `media_admission_grant` | `MediaIngressOpenResult(admitted=true)` emitted |
| `media_admission_deny` | `MediaIngressOpenResult(admitted=false)`; includes `reject_code` |
| `media_stream_close` | `MediaIngressCloseNotice` emitted (any reason) |
| `media_stream_revoke` | Stream transitions to `REVOKED` state |
| `media_degradation_step` | `MediaDegradationNotice` with non-zero `ladder_step` |
| `media_capability_revoke` | `media-ingress` (or related) capability revoked mid-session |
| `media_preempt` | Pool preemption (RFC 0002 A1 §A3.2) |
| `media_operator_override` | Operator chrome-level mute/pause/revoke |

All events include: `session_id`, `agent_namespace`, `stream_epoch`,
`capability` (where relevant), timestamp, structured reason. Retention per
C17: 90-day default, operator-configurable, local append-only with daily
rotation, schema-versioned. Full retention and schema in forthcoming RFC
0019 (Audit Log Schema and Retention).

### 9.7 Threat Model Limits

This RFC explicitly does NOT defend against:

- Malicious / untrusted agent-supplied decoder bytecode (no such path
  exists; agents don't load code).
- Kernel-level compromise of the runtime process (out of scope; relies on
  host OS).
- Codec plugin supply chain attacks beyond version-pinning and checksum
  verification.
- Upstream WebRTC implementation CVEs beyond version-pinning, fuzzing of
  the SDP parser, and keeping runtime on a supported release.

These live under the defense-in-depth backlog (`hud-lezjj`) and may be
addressed by a post-v2 subprocess-isolation hardening.

---

## 10. Open Questions / Future Extensions

### 10.1 Open Questions

1. **Media signaling answer field shape.** §4.2 notes that
   `MediaIngressOpenResult.runtime_sdp_offer` may be semantically used as
   an "answer" when the agent offered. Clarity: should
   `MediaIngressOpenResult` carry a distinct `runtime_sdp_answer` field
   or should the two be conflated by convention? Tracking for the proto
   wiring task (hud-ora8.1.23) to resolve before shipping.
2. **Operator-visible indicator wire shape.** The C14 recording indicator
   and C13 capability dialog require chrome-layer UX; their wire surface
   to chrome is owned by RFC 0007 and future RFC 0017 (recording). This
   RFC does not define the operator-visible indicator wire; media plane
   only supplies the data (`MediaIngressState.state`) that chrome
   displays.
3. **`stream_epoch` durability across device-reboot-persistent identity
   (B9).** Identity-portability work (hud-ora8.2.5 spec, RFC 0015) defines
   durable agent identity. Does `stream_epoch` persist across device
   reboots or only within a session? Phase 1 position: `stream_epoch` is
   session-scoped and NOT durable across a full session teardown. Revisit
   in RFC 0015.
4. **Audio-only latency budget validation.** §7.4 asserts a 50ms target
   without a dedicated measurement harness. Phase 1 validation
   (validation-operations spec) needs a mouth-to-ear harness; this is
   tracked as discovered work for the validation-operations bead graph.
5. **Single-embodied-agent invariant and media pool.** With the
   single-embodied-agent rule (A4), can a resident agent and the single
   embodied agent each hold concurrent media streams? Yes, subject to the
   pool size N=2–4 and per-session `max_concurrent_media_streams`. The
   admission gate treats presence-level as a priority input, not a
   mutually-exclusive gate.
6. **PCM fallback for test harness.** §2.5 gates `AUDIO_PCM_S16LE` behind
   a debug flag. Confirm at phase-1 validation time whether PCM is
   actually needed for the reference stream library (D18) or whether
   Opus-only suffices.

### 10.2 Deferred (post-v2)

1. **AV1 codec.** Wire-reserved at `VIDEO_AV1 = 4` but rejected in v2.
2. **Federated media (`federated-send`).** Wire-rejected in v2
   (`CAPABILITY_NOT_IMPLEMENTED`).
3. **Subprocess codec isolation.** Security hardening if threat model
   admits untrusted-codec cases. Tracked as `hud-lezjj`.
4. **Per-agent DTLS fingerprint pinning.** Beyond TOFU. Phase 4b cloud-relay
   hardening.
5. **Runtime-relay-only transport mode.** `RELAY_MODE_HINT` has the
   `RUNTIME_RELAY_ONLY` enum reserved; enforcement wire is phase 4b
   (RFC 0018).
6. **Exclusive-mode low-latency audio (ASIO WASAPI).** Phase 1 uses
   shared-mode backends; exclusive-mode is post-v2.
7. **Spatial audio (E22 phase 4 refinement).** Degradation step 1 already
   places it at the top of the E25 ladder; phase 1 ships without spatial
   audio.
8. **Voice synthesis / agent audio egress (`audio-emit`, `MediaEgressOpen`).**
   Wire-reserved at fields 64/66; rejected in v2 phase 1.
9. **Recording wire** (RFC 0017, phase 4a). Extends this RFC at fields
   80–99.
10. **Cloud-relay SFU attach** (RFC 0018, phase 4b). Extends this RFC at
    fields 80–99.
11. **Bidirectional AV / voice synthesis wire** (phase 4f). Extends this
    RFC at fields 80–99.

---

## 11. Review Record

Per signoff packet F29, this RFC requires **≥2 external reviewer sign-offs**
given its fan-out across all later v2 phases. The table below is empty at
draft time; reviewers add rows at sign-off.

| Round | Date | Reviewer | Role | Focus | Verdict | Notes |
|-------|------|----------|------|-------|---------|-------|
| A0 | 2026-04-19 | hud-ora8.1.8 | author (agent worker) | Draft authored from signoff packet + amendments (RFC 0002 A1, RFC 0005 A1, RFC 0008 A1, RFC 0009 A1) + doctrine (media-doctrine, failure, security, v2) + audits (gstreamer, cpal). Field allocations 60–79 selected to avoid persistent-movable-elements collision at 50–51. | AUTHOR | §2.2 erratum flagged for RFC 0005 A1 relocation. Discovered work: proto wiring task hud-ora8.1.23 must use 60–79, not 50–52; hud-lezjj codec CVE defense-in-depth remains tracked. |
| R1 | — | (external reviewer 1) | external | (to be assigned) | — | — |
| R2 | — | (external reviewer 2) | external | (to be assigned) | — | — |
| (as needed) | — | — | — | — | — | — |

Sign-off criteria for reviewers:

- Field allocations (§2.2) land in a non-colliding range and do not break
  v1 wire compatibility. Erratum to RFC 0005 A1 is accepted or contested
  with a counter-proposal.
- State machine (§3) covers the seven states and all transitions from the
  signoff packet (ADMITTED / STREAMING / DEGRADED / PAUSED / CLOSING /
  CLOSED / REVOKED) and cleanly composes with RFC 0002 A1 worker lifecycle.
- SDP handling posture (§4) is defensible: no raw SDP on wire without the
  wrapper, size bounds enforced, CVE surface addressed by §9.
- E25 ladder mapping (§5) matches `failure.md` doctrine order and trigger
  authority; no agent-initiated degradation path exists.
- Codec envelope (§2.5) ships only H.264 + VP9 for v2 phase 1; AV1 deferred;
  plugin license matrix gated.
- Worker pool protocol API (§6) is consistent with RFC 0002 A1 and does not
  introduce a new isolation surface beyond E24 COMPATIBLE.
- Audio stack (§7) matches E22 (Opus stereo, runtime-owned, operator-sticky
  default).
- Relationship to RFC 0005 envelope (§8.1) preserves all protected fields
  (WidgetPublishResult.request_sequence, Layer 3 extension semantics).
- Security considerations (§9) cover capability gate, SDP handling, codec
  CVE surface, DoS, audit events, threat-model limits.

---

## Cross-references

- `about/heart-and-soul/media-doctrine.md` — doctrine layer; precedes this RFC
- `about/heart-and-soul/failure.md` §"E25 degradation ladder" — authoritative
  ladder
- `about/heart-and-soul/security.md` §"In-process media and runtime workers" —
  E24 posture
- `about/heart-and-soul/v2.md` — V2 program structure; phase 1 media activation
- `about/legends-and-lore/rfcs/0002-runtime-kernel.md` §2.8 + Amendment A1
- `about/legends-and-lore/rfcs/0005-session-protocol.md` + Amendment A1
- `about/legends-and-lore/rfcs/0008-lease-governance.md` + Amendment A1
- `about/legends-and-lore/rfcs/0009-policy-arbitration.md` + Amendment A1
- `about/legends-and-lore/rfcs/reviews/0002-amendment-media-worker-lifecycle.md`
- `about/legends-and-lore/rfcs/reviews/0005-amendment-media-signaling.md`
- `about/legends-and-lore/rfcs/reviews/0008-amendment-c13-capability-dialog.md`
- `docs/decisions/e24-in-process-worker-posture.md` — E24 COMPATIBLE verdict
- `docs/audits/gstreamer-media-pipeline-audit.md` — GStreamer audit ADOPT-WITH-CAVEATS
- `docs/audits/cpal-audio-io-crate-audit.md` — cpal audio I/O audit ADOPT-WITH-CAVEATS
- `docs/reconciliations/webrtc_media_v1_signaling_shape_decision.md` —
  session-envelope extension decision
- `docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md` —
  original delta document (superseded by this RFC for field numbering)
- `openspec/changes/v2-embodied-media-presence/signoff-packet.md` — F29 gate,
  E22, E24, E25, E26, D18, C13, C17
- `openspec/changes/v2-embodied-media-presence/procurement.md` — GPU runner
  and reference streams (D18)
- `openspec/changes/v2-embodied-media-presence/specs/media-plane/` — capability
  spec (authored against this RFC)
- RFC 0015 (forthcoming) — Embodied Presence Contract
- RFC 0016 (forthcoming) — Device Profile Execution
- RFC 0017 (forthcoming) — Recording and Audit
- RFC 0018 (forthcoming) — Cloud-Relay Trust Boundary
- RFC 0019 (forthcoming) — Audit Log Schema and Retention
