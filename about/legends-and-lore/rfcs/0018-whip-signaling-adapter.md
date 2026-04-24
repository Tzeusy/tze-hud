# RFC 0018: WHIP Signaling Adapter

**Status:** Draft — pending external review (≥1 external reviewer required per signoff packet F29; F29 gate blocks phase 4b bead creation)
**Issue:** hud-amf17
**Date:** 2026-04-19
**Authors:** tze_hud architecture team (drafted by hud-amf17 worker)
**Depends on:**
- RFC 0002 (Runtime Kernel) §2.8 Media Worker Boundary + Amendment A1 media worker lifecycle (hud-ora8.1.9)
- RFC 0005 (Session Protocol) + Amendment A1 media signaling in session envelope (hud-ora8.1.10)
- RFC 0014 (Media Plane Wire Protocol) — WHIP is the signaling bridge for the cloud-relay transport mode defined there
- `about/heart-and-soul/media-doctrine.md` — four governance pillars apply to cloud-relay path
- `about/heart-and-soul/failure.md` §"E25 degradation ladder" — steps 5 and 10 are cloud-relay specific
- `about/heart-and-soul/security.md` §"Cloud-relay trust boundary" and §"In-process media and runtime workers"
- `docs/audits/webrtc-sfu-fallback-audit.md` (hud-1ee3a) — str0m as fallback transport; WHIP integration pattern for LiveKit/Cloudflare
- `docs/reports/webrtc-rs-v0.20-simulcast-readiness.md` (hud-g89zs) — NO-GO verdict for v0.20 alpha; fallback context
- `docs/reports/sfu-vendor-adapter-seam.md` (hud-s2j0l) — dual-adapter design; LiveKit (WHIP) vs Cloudflare Realtime (proprietary JSON REST) seam specification
- `openspec/changes/v2-embodied-media-presence/signoff-packet.md` — C15 vendor decision, F29 gate, E25 ladder
**Parent program:** v2-embodied-media-presence (phase 4b)
**Forward references:**
- RFC 0019 (Audit Log Schema and Retention) — cloud-relay activation events; local-append audit for relay sessions

---

## Summary

This RFC defines the **WHIP (WebRTC-HTTP Ingestion Protocol) signaling adapter** for tze_hud's
cloud-relay path. WHIP (IETF RFC 9725) is the standardized HTTP-based signaling protocol for
pushing a WebRTC stream from a client into a server-side SFU or media relay. When tze_hud
activates the phase 4b cloud-relay transport mode, it uses WHIP to bridge its internal gRPC
session plane (RFC 0014) to an external SFU — eliminating the need for SFU-proprietary
signaling while preserving tze_hud's governance, audit, and degradation semantics.

This RFC resolves the **RFC 0014 §4.2 TBD** on SDP offer/answer field shape when the agent
is the SDP offerer, specifically in the cloud-relay context where the SDP generated internally
must traverse a WHIP HTTP transaction before reaching the peer-connection stack. It pins the
`runtime_sdp_answer` field as a distinct, named field in `MediaIngressOpenResult` (distinct
from the semantically-overloaded `runtime_sdp_offer` field mentioned in the TBD comment).

The adapter is an internal runtime component — agents interact only with the existing
RFC 0014 gRPC wire (specifically `FUTURE_CLOUD_RELAY` transport mode); WHIP is invisible
to agents. Operators configure the cloud-relay endpoint via `media.cloud_relay.*` config
keys.

**F29 gate:** per the signoff packet, RFC 0018 must merge before any phase 4b implementation
bead is created. Requires **≥1 external reviewer sign-off**.

---

## Motivation

RFC 0014 reserves `MediaTransportMode.FUTURE_CLOUD_RELAY` and envelope fields 80–99 for
phase 4b, but it deliberately defers the signaling adapter design:

> "RFC 0018 (Cloud-Relay Trust Boundary) — phase 4b transport mode that plugs into this
> protocol." (RFC 0014 §1.2.3)

The SFU fallback audit (hud-1ee3a) confirmed two things that together make WHIP the correct
signaling choice:

1. **str0m is the recommended fallback transport library.** str0m's protocol matrix shows
   "WHIP: Not built-in — must be implemented by the caller (HTTP layer); WHIP is a signaling
   convention, not a transport." The audit confirmed this is low-effort to implement on top
   of str0m's existing SDP handling. The same holds for webrtc-rs v0.20 if it stabilizes.

2. **LiveKit Server exposes a native WHIP endpoint.** LiveKit Server exposes
   `/rtc/whip/{room}` and is IETF RFC 9725 compliant. Cloudflare Realtime SFU uses a
   **proprietary JSON REST API** (POST `/sessions/new` + POST `/tracks/new` with SDP embedded
   in a JSON body) — it does NOT natively speak WHIP. A WHIP-compatible sidecar
   (`cloudflare/realtime-examples/whip-whep-server`) exists as an operator-deployed optional
   adapter, but the native Cloudflare Realtime API is proprietary and incompatible with RFC
   9725. As a result, the runtime maintains **two adapter paths** (§2.5): a WHIP adapter for
   LiveKit and any WHIP-compliant SFU, and a proprietary JSON REST adapter for Cloudflare
   Realtime. The C15 vendor decision (deferred to phase 4b kickoff) determines which path
   is activated in production, but both are specified here to avoid forked implementations
   at kickoff time.

The simulcast readiness report (hud-g89zs) returned NO-GO for webrtc-rs v0.20 as of April 2026.
If str0m is invoked as the fallback at phase 4b kickoff, the WHIP HTTP layer is tze_hud's
responsibility — str0m supplies the SDP and handles ICE/DTLS/SRTP, but HTTP delivery of the
SDP offer to the SFU endpoint is caller-owned. This RFC defines that caller contract precisely.

Without a specified WHIP adapter:

- Phase 4b implementation beads would each independently design signaling to their SFU,
  creating divergence between the LiveKit path and the Cloudflare path.
- The gRPC/WHIP boundary (where RFC 0014's `MediaTransportMode.FUTURE_CLOUD_RELAY` is
  resolved) has no governance, audit, or failure semantics.
- RFC 0014's §4.2 TBD on `runtime_sdp_answer` would block the proto wiring task
  (hud-ora8.1.23) from completing the cloud-relay fields in the protocol buffer schema.
- The E25 degradation ladder's step 5 ("Drop cloud-relay") has no wire mechanism to execute.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| Phase 4b signaling bridge between gRPC session plane and external SFU | §3 WHIP HTTP Lifecycle |
| Resolve RFC 0014 §4.2 TBD: `runtime_sdp_answer` field shape | §4.1 SDP Offer/Answer Resolution |
| Dual-adapter design (LiveKit WHIP + Cloudflare proprietary JSON REST); C15 vendor-aware | §2.2 Vendor Protocol Profiles, §2.5 Adapter Paths |
| Authentication model for WHIP endpoint (bearer token) | §5 Authentication Model |
| Resource URL lifecycle (Location header, DELETE, PATCH for trickle ICE) | §3.3 Resource Management |
| Error mapping: HTTP status codes → `MediaIngressCloseNotice` close reasons | §6 Error Mapping |
| E25 ladder step 5 "Drop cloud-relay" mechanism | §7.1 Degradation Integration |
| Envelope field allocations (80–99 range reserved by RFC 0014) | §4.3 Wire Field Allocations |
| Security considerations: auth, DoS, replay, CORS | §9 Security Considerations |
| Cross-references: RFC 0002, 0005, 0014; hud-1ee3a, hud-g89zs, hud-kjody | §10 Cross-References |
| F29 gate: ≥1 external reviewer; blocks phase 4b beads | §11 Review Record |

---

## 1. Scope

### 1.1 In-scope

This RFC specifies, normatively:

1. **WHIP protocol handling** — how tze_hud implements the IETF RFC 9725 client role against
   an external SFU endpoint, including POST for session creation, SDP answer processing,
   PATCH for trickle ICE, and DELETE for teardown.
2. **Resolution of RFC 0014 §4.2 TBD** — the `runtime_sdp_answer` field shape in
   `MediaIngressOpenResult`, which is the mechanism by which the runtime delivers a cloud-relay
   SDP answer back to the requesting agent over gRPC.
3. **Wire field allocations** in the 80–99 range (reserved by RFC 0014 §2.2.3) for the
   cloud-relay transport mode messages.
4. **Authentication model** — how bearer tokens are obtained, scoped, and rotated for WHIP
   endpoint access.
5. **SDP lifecycle** in the WHIP context — how the transport descriptor (RFC 0014 §2.6) maps
   to the WHIP POST body and how the answer flows back.
6. **Resource URL lifecycle** — the `Location` header returned by WHIP POST, its use for
   DELETE (teardown) and PATCH (trickle ICE), and how tze_hud stores and expires it.
7. **Error mapping** — the complete table of WHIP HTTP error codes to `MediaIngressCloseNotice`
   close reasons (RFC 0014 §2.3.4) and reject codes (RFC 0014 §2.4).
8. **E25 ladder integration** — how degradation steps 5 and 10 (RFC 0014 §5.2) are wired to
   WHIP DELETE and the cloud-relay transport path.
9. **Security considerations** — authentication hardening, DoS surface, replay prevention,
   CORS posture, and the cloud-relay trust boundary extension.
10. **Relationship to RFC 0014** — this RFC does not redefine the media session state machine,
    codec negotiation, or worker pool protocol; it extends RFC 0014 exclusively for the
    cloud-relay transport mode.

### 1.2 Out of scope

This RFC deliberately does not cover:

1. **SFU server implementation.** tze_hud is the signaling client; the SFU (LiveKit via WHIP,
   Cloudflare Realtime via proprietary JSON REST, or another WHIP-compliant server) is the
   remote resource server. SFU configuration, room management, and participant routing are
   operator concerns. For Cloudflare Realtime, the native signaling protocol is proprietary
   JSON REST — not WHIP; see §2.2 and §2.5.
2. **WHEP (WebRTC-HTTP Egress Protocol).** WHEP is the pull/egress counterpart to WHIP.
   Phase 4b scope is push ingest (`FUTURE_CLOUD_RELAY` = pushing tze_hud's local stream to
   a relay SFU). WHEP covers pulling a remote stream *from* the SFU into tze_hud; this is
   phase 4f (bidirectional AV) scope and is deferred.
3. **RFC 0019 audit log schema.** Cloud-relay activation events are enumerated here (§8) but
   the log schema, retention policy, and append mechanism are RFC 0019's concern.
4. **C15 vendor selection.** This RFC is SFU-agnostic. The vendor decision (LiveKit Cloud vs.
   Cloudflare Calls) remains the phase 4b kickoff decision per signoff packet C15. This RFC
   makes that decision transparent at the wire level.
5. **Bidirectional AV egress signaling** (phase 4f). `MediaEgressOpen` and its cloud-relay
   variant are wire-reserved in RFC 0014 (fields 64/66) and deferred.
6. **Federation signaling.** The `federated-send` capability is rejected in v2 per RFC 0014
   §1.2.5. This RFC makes no exception for the cloud-relay path.

### 1.3 Non-goals

1. A new gRPC method for WHIP signaling. WHIP is an HTTP transaction internal to the runtime.
   Agents see only the RFC 0014 `FUTURE_CLOUD_RELAY` transport mode over the existing session
   stream. No agent-facing WHIP surface is exposed.
2. A WHIP server implementation inside tze_hud. tze_hud is a pure WHIP client.
3. SFU-specific management APIs. LiveKit's gRPC room management API is an optional
   operator-configuration path (see §2.5a); for Cloudflare Realtime SFU the proprietary
   JSON REST signaling is a first-class adapter path (§2.5b), not an extension of WHIP.
   The adapter seam (§2.5) isolates these per-vendor differences from the runtime's
   media session state machine.

---

## 2. Background

### 2.1 WHIP (IETF RFC 9725)

WHIP (WebRTC-HTTP Ingestion Protocol) is an IETF-standardized signaling convention for pushing
a WebRTC stream from a client to a server. It uses HTTP as the signaling transport, with the
SDP offer/answer exchange carried in HTTP request/response bodies:

```
Client                              WHIP Resource Server (SFU)
  │                                         │
  │  POST /whip/{resource}                  │
  │  Content-Type: application/sdp          │
  │  Authorization: Bearer <token>          │
  │  (body: SDP offer)                      │
  │ ──────────────────────────────────────► │
  │                                         │
  │  HTTP 201 Created                       │
  │  Location: /whip/{resource}/{id}        │
  │  Content-Type: application/sdp          │
  │  (body: SDP answer)                     │
  │ ◄────────────────────────────────────── │
  │                                         │
  │  [ICE/DTLS/SRTP proceeds on UDP]       │
  │                                         │
  │  PATCH /whip/{resource}/{id}            │
  │  Content-Type: application/trickle-ice-sdpfrag │
  │  (body: trickle ICE candidate fragment) │
  │ ──────────────────────────────────────► │
  │                                         │
  │  HTTP 204 No Content                    │
  │ ◄────────────────────────────────────── │
  │                                         │
  │  DELETE /whip/{resource}/{id}           │
  │ ──────────────────────────────────────► │
  │  HTTP 200 OK                            │
  │ ◄────────────────────────────────────── │
```

Key WHIP properties (per RFC 9725):

- **HTTP-based:** signaling is pure HTTP 1.1 or HTTP/2; no WebSocket required.
- **SDP offer from client:** the WHIP client (tze_hud runtime) generates the SDP offer and
  POSTs it. The SFU responds with the SDP answer. This is the "agent-initiated offer" path
  from RFC 0014 §4.2.
- **`Location` header:** the SFU response includes a `Location` header carrying the resource
  URL for subsequent PATCH (trickle ICE) and DELETE (teardown) operations.
- **Trickle ICE via PATCH:** trickle ICE candidates are exchanged via HTTP PATCH with
  `Content-Type: application/trickle-ice-sdpfrag` (RFC 8840).
- **DELETE for teardown:** the client terminates the WHIP session by sending HTTP DELETE to
  the resource URL.
- **CORS:** the SFU MUST include CORS headers allowing the tze_hud origin; this is a server
  configuration concern, not a client concern.

WHIP is explicitly *not* a media transport — ICE, DTLS, and SRTP continue to operate on UDP
(or TCP TURN relay) as in any WebRTC session. WHIP only carries the SDP signaling.

### 2.2 Vendor Protocol Profiles

The two C15 SFU vendor candidates use **different signaling protocols** at the HTTP level.
This RFC specifies both adapter paths explicitly. See §2.5 for the per-path protocol flows.

| SFU | Signaling protocol | Standards compliance | Notes |
|-----|--------------------|---------------------|-------|
| LiveKit Server (self-host or cloud) | WHIP (IETF RFC 9725) via `/rtc/whip/{room}` | Standards-compliant | Room created via LiveKit API before WHIP POST; token is a LiveKit JWT |
| Cloudflare Realtime SFU | Proprietary JSON REST: POST `/sessions/new` + POST `/tracks/new` (SDP embedded in JSON body) | Proprietary — NOT WHIP | Session created via CF REST API; SDP answer returned in JSON, not `Content-Type: application/sdp`. No `Location` header; teardown via `PUT /tracks/close`, not HTTP DELETE |

**Cloudflare WHIP clarification:** Cloudflare does NOT natively expose a WHIP (RFC 9725)
endpoint for the Realtime SFU. An operator-deployable sidecar
(`cloudflare/realtime-examples/whip-whep-server`) translates WHIP into Cloudflare's
proprietary API, but this sidecar is NOT part of the Cloudflare Realtime SFU product. The
native Cloudflare Realtime SFU signaling is proprietary and requires a dedicated adapter.

This corrects the original §2.2 draft claim that "Cloudflare Calls exposes WHIP for push
ingest" — that claim is false per the CF WHIP correction (hud-ejhnm, PR #550, merged).
Cross-reference: `docs/reports/sfu-vendor-adapter-seam.md` (hud-s2j0l) §1.1.

The runtime config key `media.cloud_relay.vendor` selects the adapter path:
- `livekit` → WHIP adapter (§2.5a)
- `cloudflare_realtime` → Cloudflare proprietary JSON REST adapter (§2.5b)
- `generic_whip` → WHIP adapter for any WHIP-compliant SFU

### 2.3 Transport Library Neutrality

The signaling adapters do not depend on a specific WebRTC transport library. Whether phase 4b
uses webrtc-rs v0.20 or str0m (per the fallback decision matrix in hud-1ee3a §4.2), the
adapter seam is identical from the transport library's perspective:

1. The transport library generates an SDP offer (via its offer-creation API).
2. The appropriate adapter (WHIP or CF JSON REST per §2.5) sends the SDP offer and receives
   the SDP answer from the SFU.
3. The transport library processes the SDP answer (via its answer-processing API).
4. ICE proceeds normally.

The adapter owns only step 2. Steps 1, 3, and 4 are the transport library's API surface.

**ICE mode constraint:** The WHIP adapter (§2.5a) supports trickle ICE via HTTP PATCH.
The Cloudflare proprietary adapter (§2.5b) requires full-gathering SDP — trickle ICE is
not available on the native CF Realtime API. The runtime MUST ensure ICE gathering completes
before issuing the CF `tracks/new` POST.

### 2.4 `whip-whep-server` Sidecar (Out of Scope)

Cloudflare provides an open-source `cloudflare/realtime-examples/whip-whep-server` that
operator-deploys a WHIP-to-CF-REST translation proxy. When this sidecar is present, tze_hud
MAY target it with the WHIP adapter (§2.5a), treating it as a `generic_whip` endpoint.

This RFC does NOT require or specify the sidecar. Operators who deploy it configure
`media.cloud_relay.vendor = generic_whip` and point `endpoint_url` at the sidecar.
The native Cloudflare Realtime adapter (§2.5b) is for operators who use the Cloudflare API
directly without a sidecar.

### 2.5 Adapter Paths

Two adapter paths are defined. Both expose the same internal seam to the runtime's media
session state machine; signaling differences are entirely contained within the adapter.

#### 2.5a WHIP Adapter (LiveKit and WHIP-compliant SFUs)

```
tze_hud runtime
    │
    │  POST {endpoint_url}
    │  Content-Type: application/sdp
    │  Authorization: Bearer {livekit-jwt}
    │  body: SDP offer
    │ ──────────────────────────────────────► LiveKit Server (WHIP resource server)
    │
    │  HTTP 201 Created
    │  Location: {resource_url}
    │  Content-Type: application/sdp
    │  body: SDP answer
    │ ◄──────────────────────────────────────
    │
    │  [trickle ICE via PATCH to {resource_url}]
    │  [teardown via DELETE to {resource_url}]
```

- Follows IETF RFC 9725 exactly.
- SDP answer is returned directly in the HTTP 201 body (`Content-Type: application/sdp`).
- `Location` header carries the resource URL for PATCH/DELETE.
- Trickle ICE via `PATCH {resource_url}` with `Content-Type: application/trickle-ice-sdpfrag`.
- Teardown via `DELETE {resource_url}`.
- Used for `media.cloud_relay.vendor = livekit` or `generic_whip`.

#### 2.5b Cloudflare Realtime Proprietary JSON REST Adapter

```
tze_hud runtime
    │
    │  POST /apps/{appId}/sessions/new
    │  Content-Type: application/json   {}
    │ ──────────────────────────────────────► Cloudflare Realtime SFU
    │  { "sessionId": "<uuid>" }
    │ ◄──────────────────────────────────────
    │
    │  POST /apps/{appId}/sessions/{sessionId}/tracks/new
    │  Content-Type: application/json
    │  body: { "sessionDescription": { "type": "offer", "sdp": "..." }, "tracks": [...] }
    │ ──────────────────────────────────────►
    │  { "sessionDescription": { "type": "answer", "sdp": "..." }, "tracks": [...] }
    │ ◄──────────────────────────────────────
    │
    │  [ICE renegotiate via PUT /sessions/{sessionId}/renegotiate — NOT trickle PATCH]
    │  [teardown via PUT /sessions/{sessionId}/tracks/close — NOT DELETE]
```

- Does NOT follow RFC 9725. No `Content-Type: application/sdp`. No `Location` header.
- SDP answer is embedded in a JSON body at `sessionDescription.sdp`.
- Session identified by `sessionId` (not a resource URL).
- Trickle ICE is NOT supported; full-gathering SDP is required.
- Teardown via `PUT /tracks/close` (not HTTP DELETE).
- Used for `media.cloud_relay.vendor = cloudflare_realtime`.
- Full adapter design: `docs/reports/sfu-vendor-adapter-seam.md` §3.2 (hud-s2j0l).

**Error implications:** The CF adapter error taxonomy partially diverges from the WHIP error
table in §6. Section §6.1 maps both paths. Four `CloudRelayCloseReason` codes are defined in
§4.3 for errors that apply to both adapter paths: `WHIP_RATE_LIMITED` (HTTP 429),
`WHIP_SERVER_ERROR` (HTTP 5xx non-503), `WHIP_PROTOCOL_VIOLATION` (caller-side protocol
misuse, e.g., missing Location header), `WHIP_BAD_REQUEST` (HTTP 400). Added by Amendment A2
(hud-6t5hj) to fulfil the cross-reference in `docs/reports/sfu-vendor-adapter-seam.md` §4.1.

---

## 3. WHIP HTTP Lifecycle

### 3.1 Session Creation (POST)

When the runtime activates a cloud-relay stream (`MediaTransportMode.FUTURE_CLOUD_RELAY`):

1. The transport library (webrtc-rs or str0m) generates an SDP offer for a peer connection
   to the SFU. The offer is produced from the agent's codec preferences and the local ICE
   candidates gathered so far. ICE gathering may be partial at POST time (trickle ICE).

2. The WHIP adapter sends HTTP POST:

   ```
   POST {media.cloud_relay.endpoint_url}
   Content-Type: application/sdp
   Authorization: Bearer {token}
   Content-Length: {len(sdp_offer)}

   {sdp_offer_body}
   ```

   - `Content-Type: application/sdp` is mandatory per RFC 9725 §4.
   - `Authorization` carries the bearer token from §5.
   - The SDP offer body MUST NOT exceed 16 KiB (RFC 0014 §4.6 size bound).

3. The SFU returns **HTTP 201 Created** with:
   - `Location: {resource_url}` — the resource URL for PATCH/DELETE.
   - `Content-Type: application/sdp` — the SDP answer body.
   - The SDP answer body MUST NOT exceed 16 KiB (same size bound enforced by adapter).

4. The adapter extracts the `Location` header value and stores it as the **WHIP resource URL**
   for this stream (keyed by `stream_epoch`). Retention: until stream teardown or session end.

5. The adapter delivers the SDP answer to the transport library for processing. The transport
   library completes the offer/answer exchange and begins ICE establishment.

**Timeout:** WHIP POST must complete within `media.cloud_relay.whip_post_timeout_secs`
(default: 10s, same as RFC 0014's `transport_timeout`). On timeout → `WHIP_TIMEOUT` error
→ `MediaIngressCloseNotice(TRANSPORT_FAILURE)` (see §6).

**Retry:** WHIP POST is NOT retried automatically. If POST fails, the stream transitions to
`CLOSING` with the mapped close reason. The agent may issue a fresh `MediaIngressOpen`.

### 3.2 Trickle ICE (PATCH)

For each trickle ICE candidate generated by the transport library after the WHIP POST:

```
PATCH {resource_url}
Content-Type: application/trickle-ice-sdpfrag
Authorization: Bearer {token}
Content-Length: {len(candidate_fragment)}

{sdpfrag_body}
```

Per RFC 8840, the `sdpfrag_body` carries the ICE candidate in SDP fragment format:
```
a=ice-ufrag:{ufrag}
a=ice-pwd:{pwd}
m=audio 9 RTP/AVP 0
a=candidate:{candidate_line}
```

- PATCH may be sent in parallel with the ICE establishment; the SFU processes candidates
  as they arrive.
- PATCH failures are non-fatal during candidate gathering (the peer connection may still
  succeed with other candidates). On PATCH `4xx` errors (other than 404 for expired resource),
  the adapter logs the failure and continues ICE with the remaining gathered candidates.
- On `404 Not Found` for PATCH: the resource URL has expired; the adapter transitions the
  stream to `CLOSING` with `TRANSPORT_FAILURE`.
- ICE candidate count per WHIP session: MAX 50 per RFC 0014 §4.6 candidate-count limit
  (capped at the same per-stream limit as the direct path).

#### 3.2.1 Cloudflare Realtime adapter: PUT /renegotiate

The Cloudflare Realtime adapter does **not** use WHIP trickle PATCH (§3.2 above). Instead,
it uses a proprietary `PUT /renegotiate` endpoint for ICE candidate delivery and simulcast
layer changes. The tze_hud adapter branches on `SfuVendorConfig` to select the right method.

**Why the divergence:** Cloudflare Realtime SFU does not accept
`Content-Type: application/trickle-ice-sdpfrag` PATCH requests. ICE renegotiation is
performed by sending a full revised SDP offer as a JSON body to:

```
PUT https://rtc.live.cloudflare.com/v1/apps/{appId}/sessions/{sessionId}/renegotiate
Content-Type: application/json
Authorization: Bearer {cf-api-token}

{
  "sessionDescription": {
    "type": "offer",
    "sdp": "..."
  }
}
```

Response: `200 OK` with `{ "sessionDescription": { "type": "answer", "sdp": "..." } }`.

**Implications for the runtime adapter:**

1. **Full ICE gathering required before `tracks/new`.** Because trickle ICE is not
   supported on the native CF Realtime path, the runtime MUST wait for ICE gathering to
   complete before issuing the initial `tracks/new` POST (§2.5b). This is in contrast to
   the WHIP adapter (§2.5a), which permits partial-ICE offers with trickle PATCH.

2. **`SfuVendorConfig` branch.** The adapter harness checks `media.cloud_relay.vendor`:
   - `livekit` or `generic_whip` → use §3.2 trickle PATCH flow.
   - `cloudflare_realtime` → use `PUT /renegotiate` flow; MUST suppress trickle ICE delivery.
   `CloudflareRealtimeAdapter::trickle_ice` returns `AdapterError::UnsupportedOperation`
   to signal this branch explicitly rather than silently dropping candidates.

3. **Simulcast layer control.** `PUT /renegotiate` also drives simulcast layer changes
   (enable/disable spatial layers) on the CF adapter. The harness calls
   `update_simulcast_layer` which issues a `PUT /renegotiate` with a revised SDP offer
   reflecting the new layer configuration.

4. **Timeout.** The renegotiate PUT MUST be bounded by
   `media.cloud_relay.cf_renegotiate_timeout_secs` (default: 5s, matching the field
   `renegotiate_timeout` in `CloudflareRealtimeAdapterConfig`). On timeout →
   `WHIP_TIMEOUT` close reason.

Cross-reference: `docs/reports/sfu-vendor-adapter-seam.md` §3.2
"CloudflareRealtimeAdapter" (hud-s2j0l, merged PR #552) contains the full
`CloudflareRealtimeAdapter` struct, HTTP lifecycle, and test matrix for this path.

### 3.3 Stream Teardown (DELETE)

On stream teardown (any trigger: agent `MediaIngressClose`, E25 step 5, step 8, step 10,
lease revocation, session disconnect, operator mute, or watchdog threshold):

```
DELETE {resource_url}
Authorization: Bearer {token}
```

Expected response: **HTTP 200 OK** or **HTTP 204 No Content**.

- DELETE is sent in parallel with the RFC 0014 teardown flow (ring buffer drain, GStreamer
  EOS injection per RFC 0002 A1 §A1).
- DELETE failure (non-2xx, timeout) is logged but does not block the internal teardown
  sequence. The stream still transitions to `CLOSED` or `REVOKED` per RFC 0014 §3.3.
- If the resource URL is absent (e.g., POST never succeeded): DELETE is skipped silently.
- **DELETE timeout:** `media.cloud_relay.whip_delete_timeout_secs` (default: 5s). On timeout,
  the adapter abandons the DELETE and completes internal teardown.

### 3.4 End-of-Session Cleanup

On session disconnect (RFC 0005 §6 grace expired):

1. For each active cloud-relay stream, the adapter fires DELETE (§3.3) with best-effort
   semantics (no wait for response).
2. Resource URLs are discarded.
3. No new WHIP sessions are opened during the grace period after disconnect.

---

## 4. SDP Handling in the WHIP Context

### 4.1 Resolution of RFC 0014 §4.2 TBD

RFC 0014 §4.2 contains the following TBD:

> "Agent-initiated offer: agent puts an SDP offer in `TransportDescriptor.agent_sdp_offer`
> on `MediaIngressOpen`. Runtime validates and, on admission, emits `MediaIngressOpenResult`
> with an SDP answer carried in the result (TBD: extend §2.3.2 with `runtime_sdp_answer`;
> phase-1 implementation may choose to return the answer in
> `MediaIngressOpenResult.runtime_sdp_offer` semantically as an 'answer' when the agent
> offered — clarity improvement owned by hud-ora8.1.23's proto wiring task)."

**This RFC resolves that TBD.** The resolution is:

- `MediaIngressOpenResult` receives a **distinct `runtime_sdp_answer` field** (field 9,
  see §4.3).
- `runtime_sdp_offer` (field 6 of `MediaIngressOpenResult`) is used **only** when the
  runtime is the SDP offerer (runtime-initiated offer path, §4.2 second bullet).
- `runtime_sdp_answer` (field 9, this RFC) is used **only on the direct-ingress path**
  (transport modes other than `FUTURE_CLOUD_RELAY`) when the agent provided an SDP offer and
  the runtime obtained an answer from the local peer-connection stack without a cloud-relay
  hop. See delivery path clarification below.
- The two fields are mutually exclusive per stream; the proto wiring task (hud-ora8.1.23)
  MUST ensure this constraint is documented in the proto comment.

**Cloud-relay SDP answer delivery path (Amendment 2 — hud-4mdir):**
For `transport.mode = FUTURE_CLOUD_RELAY`, the `MediaIngressOpenResult` does NOT carry the
SFU's SDP answer in `runtime_sdp_answer` (field 9). The two-step cloud-relay open flow
means `MediaIngressOpenResult` is emitted when the stream is admitted — before the WHIP
POST completes. The SFU's SDP answer arrives asynchronously and is delivered to the agent
via `CloudRelayOpenResult.sdp_answer` (field 3 of `CloudRelayOpenResult`, ServerMessage
field 80, §4.3).

Therefore:
- `runtime_sdp_answer` (field 9 of `MediaIngressOpenResult`) is **ALWAYS EMPTY** for
  `transport.mode = FUTURE_CLOUD_RELAY`. Agents MUST NOT wait for it on that path.
- The actual SDP answer for cloud-relay is in `CloudRelayOpenResult.sdp_answer` (§4.3,
  ServerMessage field 80, field 3 of the message).
- Phase 4b agents MUST branch on `transport.mode`: if `FUTURE_CLOUD_RELAY`, await
  `CloudRelayOpenResult` for the SDP answer; otherwise use `runtime_sdp_answer` from
  `MediaIngressOpenResult`.

**Rationale for distinct field:** Semantic overloading (`runtime_sdp_offer` used as an
"answer") would confuse agents implementing the offer/answer state machine and make protocol
analysis ambiguous in audit logs. A named `runtime_sdp_answer` field is explicit and
self-documenting. The cloud-relay delivery split (via `CloudRelayOpenResult`) reflects the
asynchronous nature of the WHIP HTTP round-trip.

**Impact on phase 1 (direct WebRTC path):** The direct WebRTC path in phase 1 uses
runtime-initiated offer exclusively (RFC 0014 §4.2 default). `runtime_sdp_answer` is
populated only on direct-ingress paths where the agent offered. Phase 1 agents do not need
to handle it. Phase 4b agents MUST handle the split described above.

### 4.2 SDP Generation in the WHIP Context

For cloud-relay streams, the SDP offer generation differs from the direct path:

1. **SDP must include SFU-targeting codec parameters.** SFUs typically require specific
   payload type numbers and RTP header extension URIs. The WHIP adapter requests from the
   transport library an SDP offer that includes the full codec suite from the agent's
   `codec_preference` (RFC 0014 §2.5), plus the standard WebRTC RTP extension headers
   (MID, RID, SSRC) required for simulcast forwarding.

2. **Simulcast attributes (RFC 8853).** If the SFU supports simulcast (LiveKit does;
   Cloudflare Calls does), the SDP offer MAY include `a=simulcast` and `a=rid` attributes
   to enable multi-layer ingestion. Whether simulcast is offered is controlled by
   `media.cloud_relay.simulcast_enabled` (default: false in phase 4b; re-evaluate when
   simulcast interop plan (hud-fpq51) is complete).

3. **BUNDLE grouping.** The SDP offer MUST include `a=group:BUNDLE` for audio+video streams.
   This is required by RFC 9725 §4 for WHIP SDP.

4. **ICE trickle attribute.** The SDP offer MUST include `a=ice-options:trickle` to signal
   support for trickle ICE via PATCH.

### 4.3 Wire Field Allocations (Phase 4b, Envelope Range 80–99)

This RFC allocates the following fields from the 80–99 range reserved by RFC 0014 §2.2.3:

#### ClientMessage additions (phase 4b cloud-relay)

| Field | Message | Traffic Class | Description |
|-------|---------|--------------|-------------|
| 80 | `CloudRelayOpen` | Transactional | Agent signals intent to activate cloud-relay transport on an admitted stream. Sent after `MediaIngressOpenResult(admitted=true)` when `transport.mode = FUTURE_CLOUD_RELAY`. Triggers WHIP POST (§3.1). |
| 81 | `CloudRelayClose` | Transactional | Agent-initiated cloud-relay teardown. Triggers WHIP DELETE (§3.3). Idempotent. |

Fields 82–89 (client) are unallocated in phase 4b.

#### ServerMessage additions (phase 4b cloud-relay)

| Field | Message | Traffic Class | Description |
|-------|---------|--------------|-------------|
| 80 | `CloudRelayOpenResult` | Transactional | Result of WHIP POST + ICE/DTLS establishment. Carries `runtime_sdp_answer` (§4.1), WHIP resource URL (for operator audit only; agents do not send PATCH/DELETE directly), and the stream's cloud-relay `relay_epoch` for reconnect. |
| 81 | `CloudRelayCloseNotice` | Transactional | Runtime-initiated cloud-relay path teardown. Carries `CloudRelayCloseReason` (§6). Distinct from `MediaIngressCloseNotice`: cloud-relay teardown (step 5) leaves the stream alive on direct path if the runtime can fall back; `MediaIngressCloseNotice` terminates the stream. |
| 82 | `CloudRelayStateUpdate` | State-stream | Coalescible update: relay-path RTT, packet loss to SFU, relay epoch health. Latest-wins. |

Fields 83–99 (server) are unallocated in phase 4b. Fields 83–89 and 90–99 are reserved for
phase 4f (bidirectional AV egress) and future extension.

#### `MediaIngressOpenResult` field 9 (resolution of RFC 0014 §4.2 TBD)

```protobuf
// Extension to MediaIngressOpenResult (RFC 0014 §2.3.2):
// Added by RFC 0018 to resolve the §4.2 TBD on SDP answer field shape.
message MediaIngressOpenResult {
  // ... existing fields 1–8 from RFC 0014 §2.3.2 ...

  // SDP answer from the runtime when the agent supplied an SDP offer in
  // TransportDescriptor.agent_sdp_offer on the DIRECT-INGRESS path only.
  //
  // DELIVERY SPLIT — populated vs. empty:
  //   Direct-ingress paths (transport.mode != FUTURE_CLOUD_RELAY):
  //     Populated when admitted = true and agent provided an SDP offer.
  //   Cloud-relay path (transport.mode = FUTURE_CLOUD_RELAY):
  //     ALWAYS EMPTY in MediaIngressOpenResult. The WHIP POST to the SFU
  //     completes asynchronously after admission. The SFU's SDP answer is
  //     delivered via CloudRelayOpenResult.sdp_answer (ServerMessage field 80,
  //     field 3 of CloudRelayOpenResult). Agents MUST NOT wait for this field
  //     on the cloud-relay path; await CloudRelayOpenResult instead.
  //
  // MUST NOT be populated alongside runtime_sdp_offer (fields are mutually
  // exclusive per stream — runtime is either offerer or answerer, not both).
  // Subject to §9 SDP security scrutiny identically to runtime_sdp_offer.
  bytes runtime_sdp_answer = 9;
}
```

#### New messages

```protobuf
// Cloud-relay session activation (ClientMessage field 80).
// Sent by agent after receiving MediaIngressOpenResult(admitted=true)
// with transport.mode = FUTURE_CLOUD_RELAY.
message CloudRelayOpen {
  // stream_epoch from MediaIngressOpenResult, for correlation.
  uint64 stream_epoch = 1;

  // Relay-path preference hint. The runtime uses this to select the WHIP
  // endpoint if multiple are configured (e.g., regional endpoints).
  RelayPathHint relay_path_hint = 2;
}

enum RelayPathHint {
  RELAY_PATH_HINT_UNSPECIFIED = 0;
  NEAREST_REGION              = 1;  // Runtime picks the nearest configured endpoint
  EXPLICIT_ENDPOINT           = 2;  // Use media.cloud_relay.explicit_endpoint_url if set
}

// Cloud-relay path close (ClientMessage field 81).
message CloudRelayClose {
  uint64 stream_epoch = 1;
  string reason = 2;  // Audit-only
}

// Cloud-relay activation result (ServerMessage field 80).
message CloudRelayOpenResult {
  uint64 stream_epoch = 1;

  // true = relay path established; false = relay path failed.
  bool established = 2;

  // SDP answer from the SFU (via WHIP POST or CF /tracks/new JSON response),
  // delivered to the agent so it can complete the offer/answer exchange on
  // its local peer connection. Populated when established = true.
  //
  // This is the authoritative SDP answer delivery vehicle for the
  // FUTURE_CLOUD_RELAY path. MediaIngressOpenResult.runtime_sdp_answer
  // (field 9) is ALWAYS EMPTY for cloud-relay; this field is the answer.
  // See RFC 0018 §4.1 (Amendment 2, hud-4mdir) for the delivery split.
  bytes sdp_answer = 3;

  // Relay epoch: stable identifier for this specific relay path instance.
  // Changes on each WHIP reconnect (distinct from stream_epoch, which
  // is stable across relay path reconnects for the same stream).
  uint64 relay_epoch = 4;

  // WHIP resource URL as reported by the SFU (Location header).
  // Included for operator-level audit and chrome display only.
  // Agents MUST NOT send HTTP PATCH or DELETE to this URL directly —
  // the runtime owns the relay path lifecycle.
  string relay_resource_url = 5;

  // Failure code when established = false.
  string close_reason_code = 6;  // §6 code table
  string close_reason_detail = 7;  // Human-readable
}

// Cloud-relay path teardown notice (ServerMessage field 81).
message CloudRelayCloseNotice {
  uint64 stream_epoch = 1;
  uint64 relay_epoch = 2;
  CloudRelayCloseReason reason = 3;
  string detail = 4;
  // true if the stream remains alive on direct WebRTC path after relay teardown.
  // false if the stream is also terminated (steps 8–10 of E25).
  bool stream_survives = 5;
}

enum CloudRelayCloseReason {
  CLOUD_RELAY_CLOSE_REASON_UNSPECIFIED = 0;
  WHIP_POST_FAILED        = 1;  // SFU rejected the WHIP POST (see §6 HTTP error table)
  WHIP_TIMEOUT            = 2;  // WHIP POST/PATCH/DELETE or CF renegotiate PUT timed out; also covers ICE establishment timeout
  WHIP_RESOURCE_EXPIRED   = 3;  // SFU returned 404 on PATCH/DELETE
  ICE_FAILURE             = 4;  // ICE failed on the relay path specifically
  DTLS_FAILURE            = 5;  // DTLS handshake failure on relay path
  SFU_DISCONNECTED        = 6;  // Transport-level disconnect from SFU
  E25_STEP_5              = 7;  // Degradation ladder step 5: "Drop cloud-relay"
  E25_STEP_10             = 8;  // Degradation ladder step 10: "Disconnect" (relay path termination)
  OPERATOR_DISABLED       = 9;  // Operator or policy disabled cloud-relay mid-session
  CAPABILITY_REVOKED      = 10; // cloud-relay capability revoked
  SESSION_DISCONNECTED    = 11; // Agent session disconnected; relay path cleaned up
  // CF-specific and general HTTP error codes (Amendment A2, hud-6t5hj):
  WHIP_RATE_LIMITED       = 12; // HTTP 429 Too Many Requests — SFU rate limit hit; maps AdapterError::RateLimited
  WHIP_SERVER_ERROR       = 13; // HTTP 5xx (non-503) — unexpected SFU-side server failure; maps AdapterError::ServerError
  WHIP_PROTOCOL_VIOLATION = 14; // Caller-side protocol misuse (e.g., missing Location header, wrong Content-Type); maps AdapterError::Protocol
  WHIP_BAD_REQUEST        = 15; // HTTP 400 Bad Request — malformed SDP or missing required attributes; maps AdapterError::BadRequest
}

// Coalescible relay path health update (ServerMessage field 82).
message CloudRelayStateUpdate {
  uint64 stream_epoch = 1;
  uint64 relay_epoch = 2;
  uint32 relay_rtt_ms = 3;     // RTT to SFU (RTCP-derived or ICE consent check)
  uint32 packet_loss_ppm = 4;  // Parts-per-million packet loss on relay path
  uint32 relay_bitrate_kbps = 5;
  uint64 sample_timestamp_wall_us = 6;
}
```

---

## 5. Authentication Model

### 5.1 Bearer Token Delivery

The WHIP spec (RFC 9725 §4.1) requires bearer token authentication. tze_hud delivers the
bearer token to the WHIP adapter through the following chain:

1. **Operator configuration**: `media.cloud_relay.bearer_token_source` specifies how the
   token is obtained:

   | Source | Behavior |
   |--------|----------|
   | `static` | Token is read from `media.cloud_relay.static_bearer_token` at startup. Suitable for development. NOT recommended for production. |
   | `env` | Token is read from the environment variable named in `media.cloud_relay.bearer_token_env`. |
   | `hook` | Token is obtained by invoking the executable at `media.cloud_relay.bearer_token_hook`. The hook is called with the target WHIP endpoint URL as `$1` and MUST write the token to stdout. |
   | `oidc` | Token is obtained from an OIDC-compatible token endpoint (client credentials flow) configured via `media.cloud_relay.oidc.*`. |

2. **Token scope**: the token MUST be scoped to the specific SFU endpoint and room/session.
   For LiveKit JWT tokens, the standard LiveKit `video.roomJoin` grant is used. For Cloudflare
   Calls tokens, the Cloudflare API token must have `calls:write` permission.

3. **Token delivery to WHIP POST/PATCH/DELETE**: the adapter includes the token as:
   `Authorization: Bearer {token}`

### 5.2 Token Lifetime and Rotation

- **Short-lived tokens** (recommended): LiveKit JWTs default to 1 hour; Cloudflare tokens
  should be scoped to the session lifetime. The adapter does not refresh tokens mid-session.
  If a PATCH or DELETE fails with `401 Unauthorized`, the adapter logs the event, transitions
  the stream to `CLOSING` with `CLOUD_RELAY_CLOSE_REASON_CAPABILITY_REVOKED`, and does not
  retry with a refreshed token. Token refresh is a pre-session concern.

- **Hook-mode rotation**: if `bearer_token_source = hook`, the hook is called once per
  `CloudRelayOpen` (per stream admission). The hook is NOT called for PATCH or DELETE —
  those reuse the token from the initial POST. This ensures the token lifetime covers the
  full stream lifecycle.

- **Token never logged**: the bearer token MUST NOT appear in audit log entries, debug logs,
  or the `relay_resource_url` field in `CloudRelayOpenResult`. Audit entries record the WHIP
  event, endpoint URL, and result code — not the token value.

### 5.3 Token Validation by the Runtime (Defense-in-Depth)

The runtime does not validate the SFU's token acceptance before delivering it. The SFU's
`401 Unauthorized` response (§6) is the authoritative signal. This is correct for a WHIP
client — the SFU is the token authority.

However, the `hook` and `oidc` token sources SHOULD include:

- A pre-session connectivity check: the adapter MAY call the WHIP endpoint's `OPTIONS`
  method (RFC 9725 §4.4) to verify the endpoint is reachable and check `Allow: POST, PATCH,
  DELETE` before issuing the first POST.
- Token expiry pre-check: if the token source provides an expiry field (OIDC `exp` claim),
  the adapter SHOULD reject admission with `TRANSPORT_NEGOTIATION_FAILED` if the token will
  expire within the session lifetime estimate (`expires_at_wall_us`).

---

## 6. Error Mapping

### 6.1 WHIP HTTP Status Codes → Close Reasons

| HTTP Status | Context | `CloudRelayCloseReason` | `MediaCloseReason` (if stream terminates) | Notes |
|------------|---------|------------------------|------------------------------------------|-------|
| 201 Created | POST success | — (success path) | — | Normal continuation |
| 400 Bad Request | POST: malformed SDP or missing required attributes | `WHIP_BAD_REQUEST` | `TRANSPORT_FAILURE` | Adapter logs the response body; does not retry. Maps `AdapterError::BadRequest`. |
| 401 Unauthorized | POST/PATCH/DELETE: invalid or expired token | `WHIP_POST_FAILED` | `TRANSPORT_FAILURE` | See §5.2 |
| 403 Forbidden | POST: token valid but room/capability denied | `WHIP_POST_FAILED` | `TRANSPORT_FAILURE` | SFU policy rejection; operator must fix config |
| 404 Not Found | PATCH/DELETE: resource URL expired | `WHIP_RESOURCE_EXPIRED` | `TRANSPORT_FAILURE` | Resource expired; stream terminated |
| 405 Method Not Allowed | Any: WHIP endpoint does not support the method | `WHIP_PROTOCOL_VIOLATION` | `TRANSPORT_FAILURE` | Config error; endpoint URL wrong. Also raised when SFU 201 missing Location header. Maps `AdapterError::Protocol`. |
| 409 Conflict | POST: session state conflict (CF adapter only) | `WHIP_POST_FAILED` | `TRANSPORT_FAILURE` | CF Realtime: session already exists or state machine conflict; operator must retry with a new session |
| 422 Unprocessable Entity | POST: SDP negotiation rejected by SFU | `WHIP_POST_FAILED` | `TRANSPORT_FAILURE` | Adapter logs the response body; does not retry |
| 429 Too Many Requests | POST: SFU rate limit (both adapter paths) | `WHIP_RATE_LIMITED` | `TRANSPORT_FAILURE` | Backoff is NOT implemented (see §9.4 DoS). Maps `AdapterError::RateLimited`. |
| 503 Service Unavailable | POST: SFU unavailable | `WHIP_POST_FAILED` | `TRANSPORT_FAILURE` | Operator should configure a fallback endpoint |
| 5xx (non-503) | POST/PATCH/DELETE: unexpected server error (both adapter paths) | `WHIP_SERVER_ERROR` | `TRANSPORT_FAILURE` | Catch-all for server-side failures not covered by specific rows; adapter logs status and body. Maps `AdapterError::ServerError`. |
| Timeout (no response) | POST/PATCH/DELETE: network timeout | `WHIP_TIMEOUT` | `TRANSPORT_FAILURE` | See §3.1 timeout policy |
| ICE failure (post-WHIP) | ICE gathering or consent check failure on relay path | `ICE_FAILURE` | `TRANSPORT_FAILURE` | After SDP exchange, before media flows |
| DTLS failure (post-WHIP) | DTLS handshake failure on relay path | `DTLS_FAILURE` | `TRANSPORT_FAILURE` | |
| SFU disconnect (post-DTLS) | TCP/UDP session to SFU drops | `SFU_DISCONNECTED` | `TRANSPORT_FAILURE` if no recovery | Adapter attempts ICE restart once (§6.2) |

### 6.2 ICE Restart on Relay Path Disconnect

If the relay transport drops after the stream reaches `STREAMING` state (SFU disconnect or
sustained packet loss threshold):

1. Adapter attempts one ICE restart: generates a new SDP offer and sends a WHIP PATCH with
   `Content-Type: application/sdp` (RFC 9725 §5, ICE restart via PATCH).
2. If the ICE restart succeeds within `media.cloud_relay.ice_restart_timeout_secs` (default
   5s): relay epoch increments, `CloudRelayStateUpdate` is emitted with updated `relay_epoch`.
3. If the ICE restart fails: adapter transitions to `SFU_DISCONNECTED`, fires WHIP DELETE,
   and emits `CloudRelayCloseNotice(SFU_DISCONNECTED, stream_survives=false)`. The stream
   transitions to `CLOSING` via `MediaIngressCloseNotice(TRANSPORT_FAILURE)`.

ICE restart is attempted **at most once** per relay session to avoid thrashing. Subsequent
reconnect requires a fresh `CloudRelayOpen` from the agent.

### 6.3 Reject Codes for `MediaIngressOpenResult`

When the WHIP transport mode is requested but admission fails before WHIP POST:

| Condition | `reject_code` (RFC 0014 §2.4) |
|-----------|-------------------------------|
| `cloud-relay` capability not granted | `CAPABILITY_REQUIRED` |
| `media.cloud_relay.enabled = false` at deployment | `CAPABILITY_NOT_ENABLED` |
| No WHIP endpoint configured | `TRANSPORT_NEGOTIATION_FAILED` |
| Codec not supported by configured SFU | `CODEC_UNSUPPORTED` (informed by `media.cloud_relay.supported_codecs` config) |

---

## 7. E25 Degradation Ladder Integration

### 7.1 Step 5: "Drop Cloud-Relay"

E25 ladder step 5 is "Drop cloud-relay" — the runtime sheds the relay path without tearing
down the media stream. The mechanism:

1. Runtime determines the E25 step-5 condition is met (budget breach at global level, per
   RFC 0014 §5.2 and `failure.md` §"E25 degradation ladder").
2. For each stream on the cloud-relay path:
   a. Adapter sends WHIP DELETE (§3.3) — best-effort, non-blocking teardown.
   b. Runtime emits `CloudRelayCloseNotice(E25_STEP_5, stream_survives=true)` — indicating
      the stream itself is not terminated; only the relay path is dropped.
   c. Runtime emits `MediaDegradationNotice(ladder_step=5, trigger=RUNTIME_LADDER_ADVANCE)`
      per RFC 0014 §5.2.
   d. Stream remains in `STREAMING` or `DEGRADED` state on the direct WebRTC path if one
      is available. If no direct path is available (stream was relay-only), the stream
      transitions to `CLOSING` via `MediaIngressCloseNotice(DEGRADATION_TEARDOWN)`.

3. Step 5 is reported through the same `MediaDegradationNotice` infrastructure as steps 1–4
   (RFC 0014 §2.3.6), providing correlated observability across all ladder steps.

**Agent behavior after step 5:** The agent MAY send a fresh `CloudRelayOpen` after recovery
(when the runtime signals recovery via `MediaDegradationNotice(ladder_step=0)`). The runtime
admits the re-request subject to admission gate re-evaluation.

### 7.2 Step 10: "Disconnect"

E25 ladder step 10 triggers full session teardown (RFC 0014 §5.2 step 10,
`SESSION_DISCONNECTED`). For cloud-relay streams, the WHIP adapter fires DELETE for each
active relay resource as part of the teardown sequence. This is best-effort (no wait for
response); the stream transitions to `REVOKED` per RFC 0014 §3.3 regardless of DELETE
outcome.

### 7.3 Audit Events for Degradation

The adapter emits the following events at steps 5 and 10 (in addition to the RFC 0014
`MediaDegradationNotice` events) for cloud-relay-specific audit:

| Event | Step | Data |
|-------|------|------|
| `cloud_relay_drop` | 5 | `stream_epoch`, `relay_epoch`, `relay_resource_url` (hashed), `sfu_endpoint` (domain only), timestamp |
| `cloud_relay_session_end` | 10 | Same as above |

Full audit schema and retention are RFC 0019's responsibility.

---

## 8. Cross-Plane Relationships

### 8.1 Relationship to RFC 0014 (Media Plane Wire Protocol)

This RFC is an additive extension of RFC 0014. It does not modify:

- The media session state machine (§3 of RFC 0014). Cloud-relay is a transport-path concern
  within the `STREAMING` state; the state machine remains unchanged.
- Codec negotiation (§2.5 of RFC 0014). The codec envelope applies identically; the WHIP
  SDP offer carries the same codec preferences.
- The degradation mechanism (§5 of RFC 0014). E25 steps 1–4 and 6–10 are unchanged; this
  RFC adds the step-5 mechanism only.
- Worker pool protocol API (§6 of RFC 0014). The cloud-relay path runs in the same media
  worker pool; no new spawn category is introduced.

RFC 0014 §1.2.3 states: "Recording wire protocol (RFC 0017 phase 4a) and cloud-relay trust
boundary (RFC 0018 phase 4b). Both extend this RFC additively; their wire fields will land in
the 80–99 envelope range reserved here for phase 4 additions." This RFC fulfills that
forward reference.

### 8.2 Relationship to RFC 0005 (Session Protocol)

The cloud-relay transport path is opaque to the session envelope layer. Agents still send
`MediaIngressOpen` (field 60) and receive `MediaIngressOpenResult` (field 60) exactly as in
phase 1. The only session-envelope change is the new field 9 (`runtime_sdp_answer`) in
`MediaIngressOpenResult` (§4.1, §4.3) and the new fields 80–81 (client) and 80–82 (server).

All RFC 0005 protected-field invariants remain unchanged:

- `WidgetPublishResult.request_sequence` (ServerMessage field 47, field 1 of
  `WidgetPublishResult`): untouched.
- Layer 3 extension semantics from `mcp-stress-testing`: untouched.
- RFC 0005 Amendment A1 Protected Fields list: untouched.

### 8.3 Relationship to RFC 0002 (Runtime Kernel)

The WHIP adapter runs within the media worker boundary defined in RFC 0002 §2.8 and
Amendment A1 §A1:

- The WHIP HTTP client (POST/PATCH/DELETE) is an async Tokio task co-located with the
  media worker for the stream. No new thread is introduced.
- WHIP HTTP I/O does not touch the compositor thread or the gRPC control-plane thread.
- The WHIP resource URL is stored in the media worker's state; it is released in the
  DRAINING phase (RFC 0002 A1 §A1).
- WHIP HTTP I/O is subject to the per-worker CPU watchdog (RFC 0002 A1 §A4.1). If the
  WHIP task exceeds its time budget, the watchdog fires `BUDGET_WATCHDOG` → stream teardown.

### 8.4 Relationship to RFC 0008 (Lease Governance)

The `cloud-relay` capability (RFC 0008 A1 §A1 capability taxonomy) gates the cloud-relay
path. Admission of `FUTURE_CLOUD_RELAY` transport mode requires:

1. `cloud-relay` capability granted and dialog-passed per RFC 0008 A1 §A2.
2. `media-ingress` capability also required (cloud-relay is a transport mode on top of
   media ingress, not a replacement).
3. Capability revocation mid-session: emits
   `CloudRelayCloseNotice(CAPABILITY_REVOKED, stream_survives=false)` +
   `MediaIngressCloseNotice(CAPABILITY_REVOKED)`.

### 8.5 Relationship to hud-1ee3a (SFU Fallback Audit) and hud-g89zs (Simulcast Readiness)

The SFU fallback audit (hud-1ee3a) established:

- str0m is the recommended fallback transport library if webrtc-rs v0.20 is not ready.
- WHIP integration for str0m: str0m generates the SDP offer; the WHIP adapter sends it via
  HTTP; str0m processes the answer. This is explicitly the caller's responsibility per the
  str0m protocol coverage table (§1.3: "WHIP: Not built-in — must be implemented by the
  caller (HTTP layer)").

The simulcast readiness report (hud-g89zs) returned NO-GO for webrtc-rs v0.20 alpha as of
April 2026. This RFC's design is transport-library-neutral (§2.3) so the fallback decision
does not affect the WHIP adapter specification.

**hud-kjody** (referenced in the issue description alongside hud-1ee3a and hud-g89zs) is
part of the audit trail for phase 4b pre-conditions. This RFC cross-references the discovered
follow-up chain: hud-1ee3a → hud-amf17 (this RFC) → phase 4b implementation beads.

---

## 9. Security Considerations

### 9.1 Authentication Hardening

Defense-in-depth for WHIP bearer token security:

1. **Token in memory only.** The bearer token MUST NOT be written to disk, included in audit
   logs, or appear in the `relay_resource_url` field. The `hook` and `oidc` sources are
   preferred over `static` for production deployments.

2. **Token scope.** SFU tokens MUST be scoped to the minimum required permissions (single
   room + write). Broad tokens (all-rooms write, admin) are explicitly rejected by the adapter
   at the configuration validation step.

3. **TLS required.** All WHIP HTTP requests (POST/PATCH/DELETE) MUST use HTTPS. The adapter
   MUST reject `http://` endpoint URLs in production mode (`media.cloud_relay.allow_insecure`
   defaults to false; only settable to true in `media.mode = development`).

4. **CORS.** WHIP resource servers SHOULD include appropriate CORS headers. tze_hud is not
   a browser; it does not enforce CORS as a client. Operator responsibility.

5. **TOFU extended to relay path.** RFC 0014 §4.5 establishes DTLS fingerprint TOFU per
   session for the direct path. For the relay path, the DTLS endpoint is the SFU, not the
   remote peer. The SFU's DTLS fingerprint is pinned on first connection per relay session
   and stored in the media worker's state. Per-SFU fingerprint pinning (stronger than TOFU)
   is deferred to a post-v2 hardening item.

### 9.2 Denial-of-Service Surface

Cloud-relay–specific DoS vectors:

1. **WHIP POST flood.** Bounded by the same per-session signaling rate limit as RFC 0014 §9.5
   (suggested 10 opens/s per session). Each `CloudRelayOpen` from the agent is rate-limited
   before WHIP POST is issued.

2. **WHIP timeout as resource exhaustion.** An attacker-controlled SFU endpoint that accepts
   the POST but never responds could hold the media worker's Tokio task for the duration of
   `whip_post_timeout_secs`. Mitigated by the timeout (§3.1) and the per-worker CPU watchdog.

3. **Malformed SDP answer from SFU.** The SFU's SDP answer is processed by the transport
   library's SDP parser. The same §4.6 size bounds and §4.5 parser hardening requirements
   from RFC 0014 apply to the SDP answer received over WHIP. The adapter enforces the 16 KiB
   size limit before delivering the answer to the transport library.

4. **Bogus `Location` header.** If the SFU returns a malformed or off-domain `Location`
   header, the adapter validates the URL (HTTPS scheme, same registered domain as the
   configured endpoint URL, no auth credentials in URL) before storing it. An invalid
   `Location` causes `TRANSPORT_NEGOTIATION_FAILED`.

5. **ICE candidate storm via WHIP PATCH.** Trickle ICE candidate volume is bounded by
   RFC 0014 §4.6 (MAX 50 candidates per stream), enforced before PATCH is issued.

### 9.3 Replay Prevention

WHIP sessions are protected against replay by:

- The WHIP resource URL (returned in `Location`) is UUIDv4 or equivalent random token
  generated by the SFU. Each session gets a fresh URL; old resource URLs return 404 after
  teardown, preventing replay of recorded DELETE requests.
- The bearer token is short-lived (§5.2). A replayed POST with an expired token returns
  `401 Unauthorized`.
- The `relay_epoch` (§4.3) is monotonically incrementing within a stream's lifetime. The
  agent can detect relay-path reconnects by comparing `relay_epoch` values.

### 9.4 DoS from Excessive Retries

The adapter does NOT implement automatic retry on WHIP POST failure (§3.1). This is
intentional: automatic retry against a rate-limited (`429`) or unavailable (`503`) SFU
could amplify traffic. If the agent's application layer wants retry, it may issue a new
`CloudRelayOpen` after receiving `CloudRelayCloseNotice`. The runtime applies the same
per-session signaling rate limit before accepting the new `CloudRelayOpen`.

### 9.5 Trust Boundary Extension

RFC 0014 §1.2.3 names this RFC as the "cloud-relay trust boundary" definition. The trust
model extension:

- **Direct path (phase 1):** tze_hud is the DTLS endpoint; no SFU is involved. The trust
  boundary is the local runtime.
- **Cloud-relay path (phase 4b):** the SFU is an intermediate relay. Media bytes traverse
  the SFU. The SFU operator (LiveKit/Cloudflare) MUST be treated as a *trusted relay*, not
  a *trusted peer*:
  - The SFU sees RTP media but not the gRPC control plane signaling.
  - SRTP encrypts the media between tze_hud and the remote peer end-to-end (SRTP is not
    decrypted at the SFU in standard relay mode; it IS decrypted in SFU-terminated modes
    like LiveKit's selective forwarding with decryption).
  - For SFUs that decrypt SRTP (selective forwarding with server-side media access),
    tze_hud's security posture degrades from E2E encrypted to operator-trust-required.
    This must be documented in the operator's deployment guide and the capability dialog
    for `cloud-relay` must inform the operator of the decryption posture.
  - tze_hud does NOT expose the gRPC session key, session token, or agent identity to the
    SFU. The WHIP bearer token is SFU-scoped and grants only media relay access.

### 9.6 Audit Events

Cloud-relay-specific audit events (schema owned by RFC 0019):

| Event | Trigger |
|-------|---------|
| `cloud_relay_activation` | `CloudRelayOpenResult(established=true)` — WHIP POST succeeded, ICE/DTLS established |
| `cloud_relay_activation_denied` | `CloudRelayOpenResult(established=false)` — includes `close_reason_code` |
| `cloud_relay_drop` | E25 step 5: relay path dropped, stream survives |
| `cloud_relay_teardown` | WHIP DELETE sent (any reason) |
| `cloud_relay_ice_restart` | ICE restart attempted on relay path |
| `cloud_relay_operator_disable` | Operator or policy disabled cloud-relay mid-session |
| `cloud_relay_capability_revoke` | `cloud-relay` capability revoked mid-session |

All events include: `session_id`, `agent_namespace`, `stream_epoch`, `relay_epoch`,
`sfu_endpoint` (domain only — not including auth components of the URL), timestamp, reason
code. Bearer token is never logged (§9.1).

---

## 10. Open Questions

1. **ICE restart via PATCH (RFC 9725 §5).** IETF RFC 9725 specifies that ICE restart uses
   a PATCH with `Content-Type: application/sdp` (full new offer). However, the spec notes
   this is optional server support. LiveKit's WHIP implementation supports ICE restart via
   PATCH; Cloudflare Calls' PATCH support for ICE restart is not confirmed at doc time.
   Gate the `ice_restart_attempt` code path on `media.cloud_relay.supports_ice_restart_patch`
   (default: true for LiveKit, false for Cloudflare Calls until confirmed).

2. **Simulcast in WHIP SDP.** §4.2 defers simulcast in WHIP offers to post-hud-fpq51
   completion. Once the simulcast interop plan is executed, this RFC should be amended to
   mandate simulcast SDP structure for cloud-relay WHIP offers.

3. **SRTP decryption posture at SFU.** §9.5 notes that SFUs that decrypt SRTP change the
   security posture. The operator-facing capability dialog for `cloud-relay` should include
   a disclosure of the SFU's SRTP handling. The exact dialog wording is a UX concern for
   RFC 0007 (System Shell) and is deferred.

4. **Pre-session room creation.** LiveKit requires a room to exist before WHIP POST. The
   `media.cloud_relay.session_init_hook` is the escape valve, but the runtime does not
   natively call LiveKit's Room API. Should tze_hud support a native room-creation step
   (using LiveKit's gRPC admin API) as an alternative to the hook? Deferred: hook is
   sufficient for phase 4b; native integration is post-v2.

5. **`relay_resource_url` confidentiality.** The WHIP resource URL is a capability token —
   sending it to the agent (`CloudRelayOpenResult.relay_resource_url`) for chrome display is
   a minor information exposure (the URL alone does not grant access without the bearer token,
   but it reveals the SFU endpoint structure). Consider omitting from the agent-facing message
   and retaining only in operator-facing telemetry. Deferred: include for now, revisit at
   security review.

---

## 11. Review Record

Per signoff packet F29, RFC 0018 requires **≥1 external reviewer sign-off** before phase 4b
implementation beads may be created. The table below is empty at draft time.

| Round | Date | Reviewer | Role | Focus | Verdict | Notes |
|-------|------|----------|------|-------|---------|-------|
| A0 | 2026-04-19 | hud-amf17 | author (agent worker) | Draft authored from F29 signoff packet + hud-1ee3a SFU fallback audit + hud-g89zs simulcast readiness + RFC 0014 (open PR #530). Resolved RFC 0014 §4.2 TBD on `runtime_sdp_answer` field shape. WHIP adapter specified as vendor-neutral (LiveKit + Cloudflare Calls). str0m fallback transport is compatible with no changes. | AUTHOR | Open questions flagged: ICE restart via PATCH (LiveKit only), simulcast in WHIP SDP (post-hud-fpq51), SRTP decryption posture disclosure. Note: §2.2 contained an error (Cloudflare WHIP claim) corrected in A1. |
| A1 | 2026-04-19 | hud-ojxka | amendment worker | Three amendments applied: (1) Dual-adapter correction per PR #550 (hud-ejhnm) — §1.2, §2.2, §2.3, §2.4, §2.5 added with explicit WHIP (LiveKit) vs Cloudflare proprietary JSON REST protocol flows; cross-reference to `docs/reports/sfu-vendor-adapter-seam.md` added. (2) `runtime_sdp_answer` delivery path clarification (hud-4mdir) — §4.1 text and §4.3 proto comment updated to make explicit that `runtime_sdp_answer` is ALWAYS EMPTY for `FUTURE_CLOUD_RELAY`; `CloudRelayOpenResult.sdp_answer` is the authoritative delivery vehicle for cloud-relay SDP answers. (3) §6.1 error table catch-all rows added for 409 Conflict, 422 Unprocessable Entity, and 5xx (non-503). | AMENDMENT | F29 gate (≥1 external reviewer) still required and still blocks phase 4b bead creation. |
| A2 | 2026-04-19 | hud-6t5hj | amendment worker | Two amendments applied: (1) [hud-6t5hj] Added 4 `CloudRelayCloseReason` enum variants referenced in §2.5b but missing from §4.3 proto: `WHIP_RATE_LIMITED` (HTTP 429, field 12), `WHIP_SERVER_ERROR` (HTTP 5xx non-503, field 13), `WHIP_PROTOCOL_VIOLATION` (caller-side protocol misuse, field 14), `WHIP_BAD_REQUEST` (HTTP 400, field 15). Updated §6.1 error table to map 400, 429, 5xx (non-503), and 405/protocol errors to the specific new codes rather than the generic `WHIP_POST_FAILED`. Variants match the `AdapterError` taxonomy in `docs/reports/sfu-vendor-adapter-seam.md` §4.1. (2) [hud-7lehd] Added §3.2.1 "Cloudflare Realtime adapter: PUT /renegotiate" documenting the proprietary `PUT /sessions/{sessionId}/renegotiate` path, the `SfuVendorConfig` branch point, full-ICE gathering requirement, simulcast layer control via renegotiate, and cross-reference to `docs/reports/sfu-vendor-adapter-seam.md` §3.2. Refs: hud-6t5hj + hud-7lehd. | AMENDMENT | F29 gate (≥1 external reviewer) still required and still blocks phase 4b bead creation. |
| R1 | — | (external reviewer 1) | external | (to be assigned) | — | — |
| (as needed) | — | — | — | — | — | — |

Sign-off criteria for reviewers:

- WHIP HTTP lifecycle (§3) correctly implements IETF RFC 9725 client semantics: POST with
  SDP offer, 201 + Location + SDP answer, PATCH for trickle ICE, DELETE for teardown.
- RFC 0014 §4.2 TBD resolution (§4.1): `runtime_sdp_answer` as a distinct field 9 in
  `MediaIngressOpenResult` is unambiguous and backward-compatible (field 9 is unallocated
  in RFC 0014; all existing implementations see empty bytes, which is a valid zero-value
  for the bytes type).
- Dual-adapter design (§2.2, §2.5): the two adapter paths (WHIP for LiveKit; proprietary
  JSON REST for Cloudflare Realtime) are correctly specified. The Cloudflare WHIP claim
  from the A0 draft has been corrected (§2.2). The adapter seam isolates both paths from
  the media session state machine.
- Authentication model (§5): bearer token delivery and rotation are sufficient for production
  deployment; token is never logged.
- Error mapping (§6): every relevant WHIP HTTP status code is mapped to a defined
  `CloudRelayCloseReason` and, where the stream terminates, a `MediaCloseReason`.
- E25 ladder step 5 mechanism (§7.1): relay path teardown (step 5) correctly leaves the
  stream alive on direct path when available; `stream_survives` flag is used correctly.
- Security posture (§9): trust boundary extension is correctly documented; SRTP decryption
  posture note is accurate; no bearer token logging.
- Cross-references (§8): relationships to RFC 0002, 0005, 0014 are consistent with those
  RFCs' text and do not introduce contradictions.

---

## Cross-References

- `about/heart-and-soul/media-doctrine.md` — four governance pillars; cloud-relay is a
  governed surface, not a bypass
- `about/heart-and-soul/failure.md` §"E25 degradation ladder" — steps 5 ("Drop cloud-relay")
  and 10 ("Disconnect") are this RFC's primary degradation hooks
- `about/heart-and-soul/security.md` §"Cloud-relay trust boundary" — trust model for SFU
  relay path; SRTP posture
- `about/legends-and-lore/rfcs/0002-runtime-kernel.md` §2.8 + Amendment A1 — media worker
  lifecycle; WHIP HTTP task runs within worker boundary
- `about/legends-and-lore/rfcs/0005-session-protocol.md` + Amendment A1 — session envelope;
  protected fields; reconnect semantics
- `about/legends-and-lore/rfcs/0014-media-plane-wire-protocol.md` — primary dependency;
  this RFC extends RFC 0014 for the cloud-relay transport mode; resolves §4.2 TBD
- `about/legends-and-lore/rfcs/0008-lease-governance.md` + Amendment A1 — `cloud-relay`
  capability gating; revocation path
- `docs/audits/webrtc-sfu-fallback-audit.md` (hud-1ee3a) — str0m fallback audit; WHIP
  integration pattern; LiveKit/Cloudflare Calls C15 vendor assessment
- `docs/reports/webrtc-rs-v0.20-simulcast-readiness.md` (hud-g89zs) — NO-GO verdict for
  v0.20 alpha; context for str0m fallback invocation at phase 4b
- `docs/reports/sfu-vendor-adapter-seam.md` (hud-s2j0l) — full dual-adapter design spec;
  `SfuVendorAdapter` trait, `WhipAdapter` (LiveKit), `CloudflareRealtimeAdapter` (proprietary
  JSON REST), error taxonomy, harness selection mechanism, test matrix
- `openspec/changes/v2-embodied-media-presence/signoff-packet.md` — C15 vendor decision,
  F29 gate (≥1 reviewer for RFC 0018), E25 ladder order, D18 budgets
- `openspec/changes/v2-embodied-media-presence/procurement.md` — GPU runner, SFU vendor
  cost estimates; LiveKit Cloud free dev tier
- RFC 0015 (forthcoming) — Embodied Presence Contract; cloud-relay may carry embodied media
- RFC 0017 (forthcoming) — Recording and Audit; E25 step 4 "Suspend recording" precedes
  step 5 "Drop cloud-relay" in the ladder
- RFC 0019 (forthcoming) — Audit Log Schema and Retention; owns the schema for events
  enumerated in §9.6
- IETF RFC 9725 — WHIP (WebRTC-HTTP Ingestion Protocol), the external standard this RFC
  implements
- IETF RFC 8840 — Trickle ICE SDP fragment format, used in WHIP PATCH requests
