# WebRTC SFU Fallback Audit — Phase 4b Kickoff

**Issued for**: `hud-1ee3a`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent context**: hud-ora8.1 — v2-embodied-media-presence; triggered by webrtc-rs audit (hud-ora8.1.17, PR #523)
**Prerequisite audit**: `docs/audits/webrtc-rs-audit.md`

**Re-verification note** (hud-ejhnm, 2026-04-19): Section 3 (Cloudflare Calls / Realtime) has been corrected following PR #544 review. The original version incorrectly stated that Cloudflare Realtime SFU natively supports WHIP/WHEP. The corrected section (3.3 and associated subsections) reflects verified findings: the SFU uses a proprietary JSON-over-REST API; WHIP/WHEP is available only via an official adapter example, not as a native SFU endpoint. See §3.9 for the detailed re-verification summary.

---

## Verdict

**RECOMMENDED FALLBACK: str0m**

If `webrtc-rs` v0.20 fails to reach production-ready status before phase 4b implementation begins, **str0m** is the recommended pure-Rust fallback for tze_hud's peer-connection layer. It is the only stack evaluated here that remains pure-Rust, async-runtime-agnostic, and architecturally aligned with tze_hud's in-process model.

**Stack disposition summary:**

| Stack | Role | Verdict |
|---|---|---|
| **str0m** | Pure-Rust WebRTC peer-connection library | **Recommended fallback** — pure-Rust, sans-IO, simulcast present, active maintainer |
| **LiveKit Rust SDK** | Client SDK for LiveKit SFU (Go server) | **Monitor** — requires LiveKit server; pure-Rust client available; SFU abstraction good for phase 4b cloud-relay track |
| **Cloudflare Calls** | Managed WebRTC SFU via REST/WHIP/WHEP | **Not a transport-layer fallback** — SaaS signaling abstraction, not a Rust library replacement; relevant only to C15 vendor selection |

**Decision boundary**: These three stacks operate at different layers. Only str0m is a direct peer-connection library substitute for webrtc-rs. LiveKit and Cloudflare Calls are SFU/cloud-relay services; they sit above the transport layer and depend on a WebRTC peer-connection implementation underneath. The choice between LiveKit and Cloudflare Calls is the C15 vendor decision (due at phase 4b kickoff), orthogonal to which peer-connection library tze_hud uses internally.

---

## Context

The webrtc-rs audit (hud-ora8.1.17, `docs/audits/webrtc-rs-audit.md`) returned **ADOPT-WITH-CAVEATS** with four caveats:

1. v0.17.x is feature-frozen; v0.20.x alpha must stabilize for phase 4.
2. Simulcast is unvalidated against browsers.
3. SVC is not implemented.
4. GStreamer RTP bridge requires explicit hand-off implementation.

The audit recommended pinning to `webrtc` 0.17.x for phase 1 and migrating to v0.20 for phase 4. Caveats 1 and 2 are the primary phase 4 risks. If v0.20 does not stabilize — or its simulcast interceptors remain incomplete per the hud-g89zs readiness spike — tze_hud needs a contingency transport library identified before phase 4b begins.

This audit evaluates three candidate stacks as that contingency, plus their relationship to the C15 cloud-relay vendor decision.

---

## 1. str0m — Pure-Rust WebRTC Implementation

### 1.1 Identity

| Field | Value |
|---|---|
| Crate | `str0m` |
| Repository | https://github.com/algesten/str0m |
| Current stable version | 0.18.0 (mid-April 2026) |
| License | MIT |
| Maintainer | Martin Algesten (algesten) + contributors |
| Stars | ~550 |
| Open issues | ~35 |
| crates.io | https://crates.io/crates/str0m |
| docs.rs | https://docs.rs/str0m |
| MSRV | Rust 1.75+ |

### 1.2 Architecture

str0m is a **sans-IO WebRTC library**: it exposes a state machine with no internal threads, no async runtime, and no I/O operations. The caller provides input events (network packets, timer ticks) and receives output events (packets to send, decoded media samples). This is the same architectural direction webrtc-rs is pursuing in its v0.20 `rtc` rewrite.

```
// str0m interaction model (sans-IO)
let mut rtc = Rtc::builder().build();

loop {
    // Feed network input
    if let Some(packet) = socket.try_recv() {
        rtc.handle_input(Input::Receive(now, packet))?;
    }
    // Advance timers
    rtc.handle_input(Input::Timeout(now))?;

    // Drain output
    while let Some(output) = rtc.poll_output()? {
        match output {
            Output::Transmit(send) => socket.send_to(send.contents, send.destination)?,
            Output::Timeout(t) => { /* schedule next poll at t */ }
            Output::Event(event) => match event {
                Event::MediaData(data) => handle_rtp(data),
                _ => {}
            }
        }
    }
}
```

This approach eliminates `Arc<Mutex<>>` proliferation, internal callback registrations, and async-runtime coupling — the primary ergonomic complaints about webrtc-rs v0.17.

### 1.3 Protocol Coverage

| Protocol / Standard | Status | Notes |
|---|---|---|
| ICE (RFC 8445) | Full | Host, STUN, TURN candidates |
| Trickle ICE | Full | Incremental exchange |
| DTLS 1.2 | Full | Certificate-based authentication |
| SRTP / SRTCP | Full | AES-CM-128 |
| SCTP over DTLS | Full | Data channels |
| RTP / RTCP | Full | NACK, PLI, FIR, RR/SR |
| SDP (JSEP) | Full | Offer/answer, renegotiation |
| TURN (RFC 8656) | Partial | Client support; TURN-over-TCP not fully validated |
| WHIP (WebRTC-HTTP Ingestion Protocol) | Not built-in | Must be implemented by the caller (HTTP layer); WHIP is a signaling convention, not a transport |

**Gap vs. webrtc-rs v0.17**: TURN-over-TCP is the most relevant omission for cloud-relay scenarios (where firewall traversal may require TCP TURN). webrtc-rs v0.17 supports TCP ICE; str0m's TCP TURN validation is less documented. Verify before phase 4b implementation begins.

### 1.4 Simulcast and SVC

| Feature | Status | Notes |
|---|---|---|
| Simulcast (RFC 8853) | **Present** | RID/MID handling is in the codebase; str0m is explicitly designed for SFU use cases where simulcast forwarding is the primary workload |
| SVC (scalable video coding) | Not implemented | VP9 SVC layer selection requires external logic; same gap as webrtc-rs |

str0m's simulcast support is the strongest pure-Rust implementation available. Its design as an SFU building block (used by SFU implementors, not just browser clients) means simulcast forwarding is a first-class concern — in contrast to webrtc-rs v0.17 where simulcast was bolted on and lacks browser interop validation.

**Caveat**: str0m's simulcast implementation has not been validated against Chrome/Firefox/Safari in the tze_hud context. The hud-fpq51 simulcast interop plan must include str0m if the fallback is invoked.

### 1.5 Codec Support

| Codec | Status | Notes |
|---|---|---|
| H.264 | RTP depacketization present | str0m handles RTP framing; encode/decode is caller's responsibility (GStreamer) |
| VP8 | RTP depacketization present | |
| VP9 | RTP depacketization present | VP9 SVC metadata passthrough; layer selection is external |
| AV1 | **Present** (since v0.15.0) | AV1 RTP packetizer/depacketizer added in v0.15.0 (PR #819, Jan 2026); refined through v0.17.0. Functionally equivalent to webrtc-rs's default AV1 registration. |
| Opus | RTP depacketization present | Standard Opus framing |

**AV1 status**: AV1 RTP packetization is present in str0m 0.18 — it was added in v0.15.0 and stabilised through v0.16.1 and v0.17.0 bug-fix passes. The earlier claim that AV1 was absent was incorrect. AV1 remains deferred per D18 for v2 scope, so this gap does not affect the phase 4b recommendation regardless.

### 1.6 API Ergonomics

str0m's sans-IO API is substantially more ergonomic than webrtc-rs v0.17:

- No `Arc<Mutex<>>` required at the library boundary
- No callback registrations (`on_track`, `on_ice_candidate`, etc.) — replaced by polling `poll_output()`
- No Tokio dependency — the caller owns the I/O loop (integrate with Tokio by wrapping in a `tokio::task`)
- No closure capture issues

For tze_hud, the Tokio integration wrapper is straightforward:

```rust
// Tokio integration pattern for str0m (non-normative sketch)
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_millis(5));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                rtc.handle_input(Input::Timeout(Instant::now())).ok();
            }
            Ok((data, addr)) = socket.recv_from(&mut buf) => {
                rtc.handle_input(Input::Receive(Instant::now(),
                    Receive { source: addr, destination: local_addr, contents: (&buf[..data]).into() }
                )).ok();
            }
        }
        while let Ok(Some(output)) = rtc.poll_output() {
            match output {
                Output::Transmit(send) => { socket.send_to(&send.contents, send.destination).await.ok(); }
                Output::Event(Event::MediaData(d)) => { frame_tx.try_send(d).ok(); }
                _ => {}
            }
        }
    }
});
```

This is significantly cleaner than webrtc-rs v0.17's callback-heavy API.

### 1.7 GStreamer Integration

str0m produces `MediaData` events containing raw RTP payload bytes. The GStreamer bridge pattern from the webrtc-rs audit (§4.2 of `webrtc-rs-audit.md`) applies directly: wrap the payload in an RTP packet header, push to `appsrc` with `application/x-rtp` caps.

Unlike webrtc-rs, str0m exposes raw RTP payloads without the `webrtc-util::marshal_to()` step — the caller constructs the full RTP packet if needed for GStreamer compatibility. This is a slight increase in integration effort but is well-documented in str0m examples.

### 1.8 Maintainer Health

| Metric | Observation |
|---|---|
| Primary maintainer | Martin Algesten (algesten); consistent commit history since 2022 |
| Release cadence | Active: 0.17.0 → 0.18.0 within the audit window; regular minor releases |
| Known production deployments | Lookback (video review SaaS), plus community SFU implementors |
| Stars | ~550 (significantly below webrtc-rs ~5,000, but appropriate for a server-side SFU library with a narrower audience) |
| Open issues | ~35; no critical open bugs blocking core functionality |
| Dependency on commercial maintainer | Single primary maintainer is a bus-factor risk; mitigated by active community |

**Assessment**: Healthy for a specialized library. str0m is not a hobbyist project — it has a paying-customer deployment (Lookback) and is designed for exactly the SFU use case tze_hud needs for phase 4b cloud-relay. The single-maintainer risk is real but acceptable given the fallback posture.

### 1.9 License Compatibility

**MIT** — fully compatible with tze_hud's MIT/Apache-2.0 preferred license posture. No restrictions on use or distribution.

### 1.10 Migration Effort from webrtc-rs

**Medium.** The migration is a transport-layer swap, not an architectural change:

- GStreamer bridge remains identical (RTP → appsrc pattern)
- ICE/DTLS/SRTP semantics are the same; the API shape differs substantially
- SDP generation/parsing must be reimplemented or bridged (webrtc-rs has a `webrtc-sdp` crate; str0m handles SDP internally with less direct exposure)
- Signaling layer (gRPC offers/answers per RFC 0005/0014) remains unchanged — it produces SDP strings that both libraries consume
- Simulcast interceptor setup differs in API but the intent is equivalent

Estimated migration scope: 2–4 person-weeks for the phase 4 transport layer, assuming the phase 1 GStreamer bridge is already written and tested. The primary work is the I/O loop rewrite and SDP negotiation path.

### 1.11 str0m vs. webrtc-rs v0.20 Architecture Comparison

| Criterion | str0m 0.18 | webrtc-rs v0.20 (alpha) |
|---|---|---|
| Architecture | Sans-IO, stable | Sans-IO, alpha / in-flux |
| API stability | Stable (minor breaking between versions, not volatile) | Alpha — API still changing |
| Tokio coupling | None (integrator's choice) | Optional (Tokio or smol) |
| Simulcast | Present, SFU-first | In-progress (RID/MID interceptors per hud-g89zs spike) |
| AV1 | Present (since v0.15.0) | Registered by default |
| TURN-over-TCP | Not fully validated | Open issue (#781 in v0.20 alpha) |
| Stars | ~550 | Inherits webrtc-rs ~5,000 |
| Production validation | Lookback (SFU deployment) | Not yet (alpha) |

**Assessment**: str0m is the more conservative fallback choice today. If webrtc-rs v0.20 stabilizes with complete simulcast interceptors (outcome of hud-g89zs spike), webrtc-rs v0.20 remains the primary path. If the spike returns incomplete or the alpha is not production-ready by phase 4 kickoff, str0m is the safe choice.

---

## 2. LiveKit Rust SDK

### 2.1 Identity

| Field | Value |
|---|---|
| Rust SDK crate | `livekit` (client SDK) |
| Repository | https://github.com/livekit/rust-sdks |
| Current version | 0.3.x (as of April 2026) |
| Server | LiveKit Server (Go, open-source): https://github.com/livekit/livekit |
| SaaS offering | LiveKit Cloud |
| License (Rust SDK) | Apache-2.0 |
| License (LiveKit Server) | Apache-2.0 |
| Stars (rust-sdks) | ~350 |
| Stars (livekit server) | ~11,000 |

### 2.2 What LiveKit Is

**LiveKit is a WebRTC SFU ecosystem, not a transport library.** The architecture is:

```
[tze_hud Rust runtime]
    │ (LiveKit Rust SDK — WebRTC client)
    ▼
[LiveKit Server — Go-based SFU]
    │ (WebRTC to/from remote peers)
    ▼
[Remote participants: browsers, mobile clients, agents]
```

The LiveKit Rust SDK is a **client SDK** that connects to a LiveKit SFU. Under the hood, the Rust SDK wraps `libwebrtc` (Google's C++ WebRTC implementation) via FFI — it is **not pure-Rust**. The SDK exposes a Rust-idiomatic async API, but the transport layer is `libwebrtc` (C++).

**Critical implication**: LiveKit Rust SDK is **not a substitute for webrtc-rs or str0m**. It requires a LiveKit server to function. It is a complete solution (client + server + SFU), not a transport library.

### 2.3 Rust API Surface

The LiveKit Rust SDK provides:

```rust
// LiveKit Rust SDK API pattern (non-normative)
let room = RoomOptions::default().connect(&url, &token).await?;

// Subscribe to tracks from other participants
room.on_track_subscribed(|track, _publication, _participant| {
    tokio::spawn(async move {
        while let Some(frame) = track.rtc_track().on_frame().await {
            // frame is a decoded video frame (not raw RTP)
        }
    });
});

// Publish a local track
let track = LocalVideoTrack::create_video_track("camera", VideoSource::new());
room.local_participant().publish_track(track, TrackPublishOptions::default()).await?;
```

The SDK abstracts away ICE, DTLS, SDP, and RTP entirely. This is a higher-level API than webrtc-rs or str0m. The trade-off: you cannot access raw RTP frames — the SDK delivers decoded frames or encoded `EncodedFrame` depending on the track type.

**GStreamer integration implication**: The LiveKit SDK's decoded-frame delivery bypasses the `appsrc` RTP bridge pattern. tze_hud would need a different GStreamer integration: deliver decoded frames from LiveKit's SDK into GStreamer's `appsrc` as raw video buffers rather than as RTP packets. This is architecturally viable but requires a separate code path from the webrtc-rs bridge in `webrtc-rs-audit.md` §4.2.

### 2.4 Architecture Mismatch for tze_hud

tze_hud's media plane (per RFC 0014, forthcoming) owns ICE/DTLS/SRTP negotiation at the gRPC signaling boundary. The LiveKit SDK owns all of this internally. Using LiveKit means:

1. Replacing the per-RFC-0014 signaling plane with LiveKit's protocol.
2. Accepting LiveKit Server as a mandatory dependency (even in self-host mode).
3. Accepting `libwebrtc` (C++) as a transitive dependency — an FFI boundary in the hot path, which violates CLAUDE.md's posture.

**Verdict on transport layer substitution**: LiveKit is **not an appropriate transport-layer fallback** for webrtc-rs. It is a full SFU system.

### 2.5 Relevant Role: C15 Cloud-Relay Vendor Selection

LiveKit **is** the preferred vendor for the C15 cloud-relay decision (signoff packet C15, procurement.md "Phase 4b: cloud-relay sub-epic"). In that context:

- tze_hud's internal peer-connection library (webrtc-rs or str0m) connects to the LiveKit SFU.
- The LiveKit Rust SDK is **not used** — instead, tze_hud's existing WebRTC stack connects directly to LiveKit's SFU using standard WebRTC (ICE + DTLS + SDP).
- The LiveKit "Rust SDK" would only be used if tze_hud were acting as a LiveKit *client*, routing through LiveKit's participant model.

For tze_hud's ingest and bidirectional AV use case, the relevant LiveKit integration surface is:
- LiveKit's **WHIP endpoint** for push ingest (browser/client → LiveKit Server → tze_hud).
- LiveKit's **gRPC/WebSocket signaling** for room join and track subscription.

Either way, the underlying WebRTC transport remains webrtc-rs (or str0m fallback).

### 2.6 Self-Host vs. Managed

| Mode | Notes |
|---|---|
| LiveKit Cloud (managed) | Per-minute billing; global TURN infrastructure; developer tier is free. Production cost estimate: ≤$50/month during development (per procurement.md). |
| Self-host | Apache-2.0; single binary Go server. Can run on the GPU runner box (D18) for development. TURN infrastructure still required for firewall traversal. |

**Recommendation**: Use LiveKit Cloud for phase 4b development (zero infrastructure burden); self-host post-v2 for cost/control at scale.

### 2.7 Maturity and Maintainer Health

| Metric | Observation |
|---|---|
| LiveKit Server maturity | Production-ready; widely deployed (50,000+ LiveKit rooms/day in public cloud) |
| Rust SDK maturity | Pre-1.0; API surface still evolving. The Go SDK and JS/TS SDKs are substantially more mature. |
| License | Apache-2.0 — compatible with tze_hud |
| Open issues (rust-sdks) | ~80 open issues; active PR velocity |
| Primary concern | FFI boundary (`libwebrtc` C++) in the Rust SDK; not pure-Rust |

**Assessment**: LiveKit Server is production-grade. The Rust SDK is not the recommended integration path for tze_hud (due to libwebrtc FFI and the architectural mismatch above). The right LiveKit integration for phase 4b is tze_hud's WebRTC stack connecting directly to LiveKit Server using standard WebRTC signaling.

---

## 3. Cloudflare Calls / Realtime

### 3.1 Identity

| Field | Value |
|---|---|
| Service | Cloudflare Calls (also branded "Cloudflare Realtime SFU") |
| Type | SaaS WebRTC SFU and TURN infrastructure |
| API | Proprietary JSON-over-HTTPS REST API; WHIP/WHEP available via official example adapter only |
| Rust SDK | **None official** — REST API integration only |
| License | Proprietary SaaS |
| Pricing | $0.05/GB egress; first 1,000 GB/month free (Realtime SFU tier, as of Apr 2026) |
| Protocol | Standard WebRTC (ICE/DTLS/SRTP under the hood); proprietary REST signaling (SDP inside JSON body) |
| TURN | Cloudflare's global Anycast network (shared quota with SFU) |
| Docs | https://developers.cloudflare.com/realtime/ (rebranded from /calls/ in 2026) |

### 3.2 What Cloudflare Calls Is

Cloudflare Calls is a **managed WebRTC SFU service** exposed via a proprietary JSON-over-REST API. Like LiveKit, it is **not a Rust transport library** — it is a cloud service. The integration pattern:

```
[tze_hud Rust runtime]
    │ (webrtc-rs or str0m peer-connection)
    │ (POST /apps/{appId}/sessions/{sessionId}/tracks/new — JSON body with SDP)
    ▼
[Cloudflare Realtime SFU — SaaS]
    │ (WebRTC to/from browsers/clients)
    ▼
[Remote participants]
```

### 3.3 WHIP/WHEP Protocol Compatibility — Corrected (hud-ejhnm)

**Verdict: Cloudflare Realtime SFU does NOT natively support WHIP or WHEP.** The original audit claim was incorrect. The verified findings follow.

#### 3.3.1 Native API: Proprietary JSON-over-REST

The Cloudflare Realtime SFU uses a proprietary HTTPS API where SDP is exchanged inside a JSON body, not as `application/sdp`. The five native endpoints are:

| Endpoint | Method | Purpose |
|---|---|---|
| `/apps/{appId}/sessions/new` | POST | Create a session (maps to one WebRTC PeerConnection) |
| `/apps/{appId}/sessions/{sessionId}/tracks/new` | POST | Push or pull media tracks; body includes `sessionDescription.sdp` (JSON) |
| `/apps/{appId}/sessions/{sessionId}/renegotiate` | PUT | Trigger ICE renegotiation |
| `/apps/{appId}/sessions/{sessionId}/tracks/close` | PUT | Remove tracks |
| `/apps/{appId}/sessions/{sessionId}` | GET | Inspect session state |

The `/tracks/new` request body schema (JSON, `application/json`):
```json
{
  "sessionDescription": { "sdp": "<offer string>", "type": "offer" },
  "tracks": [ { "location": "local", "trackName": "camera", "mid": "0" } ]
}
```

The response embeds the SDP answer inside a JSON object — there is no `Location` header, no `Content-Type: application/sdp`, and no HTTP DELETE to tear down, all of which are required by WHIP (RFC 9725). This is **not WHIP-compliant**.

#### 3.3.2 WHIP/WHEP Availability: Adapter Example Only

Cloudflare ships an official example adapter — `cloudflare/realtime-examples/whip-whep-server` — that implements a WHIP/WHEP HTTP interface on top of the proprietary JSON API. This adapter is:

- Listed in the Cloudflare Realtime SFU demos page
- Described as "WHIP and WHEP server implemented on top of Realtime API"
- A server-side translation layer, not a native SFU protocol

To use WHIP/WHEP with Cloudflare Realtime SFU, the caller must either self-deploy this adapter or implement an equivalent translation layer. The SFU itself does not speak WHIP.

#### 3.3.3 Cloudflare Stream vs. Cloudflare Realtime SFU

There is a product-level distinction that the original audit conflated:

| Product | WHIP/WHEP | Notes |
|---|---|---|
| **Cloudflare Stream** | Native — draft-ietf-wish-whip-06 / draft-murillo-whep-01 | CDN live-streaming product; single-publisher, unlimited viewer broadcast |
| **Cloudflare Realtime SFU** | Adapter only (example server) | Multi-party conferencing SFU; proprietary JSON REST API |

Cloudflare Stream is not an SFU and is not applicable for Phase 4b cloud-relay (it lacks the multi-party track routing capability tze_hud needs). References to WHIP/WHEP in Cloudflare's developer ecosystem primarily document Cloudflare Stream, not Realtime SFU.

#### 3.3.4 tze_hud Integration Implication

The signaling adapter required for Cloudflare Realtime SFU is **not WHIP** — it is a custom JSON REST adapter. The RFC 0018 cloud-relay trust boundary spec must describe this adapter explicitly if Cloudflare Realtime SFU is selected as the C15 fallback. The adapter is implementable (the example reference implementation exists), but it adds a bespoke translation layer not shared with LiveKit integration.

**Protocol summary**: Cloudflare Realtime SFU = SDP-inside-JSON REST, not WHIP. Adapter available. Not natively standards-compliant for WHIP ingest.

### 3.4 API Maturity

| Aspect | Assessment |
|---|---|
| Native WHIP support | **No** — SFU uses proprietary JSON REST API; WHIP is adapter-only |
| Native WHEP support | **No** — same as WHIP; adapter example available |
| WHIP/WHEP via adapter | Available — `cloudflare/realtime-examples/whip-whep-server` |
| SFU features (simulcast forwarding) | Present — Cloudflare handles simulcast forwarding server-side |
| SVC | Not documented; unlikely |
| Recording | Available via Cloudflare Stream integration |
| Data channels | Supported via WebRTC DataChannel (confirmed in API) |
| WebSocket media adapter | Present — ingests audio from WebSocket sources into WebRTC tracks |
| API versioning | v1 stable for core session/track flows |

**Assessment**: Cloudflare Realtime SFU is API-mature for the core ingest + fan-out use case. The proprietary API is well-documented (OpenAPI schema available). The absence of native WHIP/WHEP adds integration surface: either deploy the adapter server or implement the JSON REST protocol directly. The latter is simpler but creates a Cloudflare-specific code path. This increases the RFC 0018 adapter scope compared to what the original audit assumed.

### 3.5 Vendor-Lock Risks

Cloudflare Calls introduces stronger vendor lock-in than LiveKit:

| Risk | Severity | Notes |
|---|---|---|
| No self-host option | High | Cloudflare Calls is SaaS-only. If Cloudflare deprecates or changes pricing, migration requires switching to a different SFU vendor. |
| Fully proprietary REST API | **High** (revised) | The entire signaling surface is Cloudflare-specific JSON REST — no portable WHIP/WHEP layer is natively available. All track management, session routing, and renegotiation flows are non-portable. |
| TURN infrastructure lock-in | Low | Standard WebRTC ICE works with any STUN/TURN; tze_hud could replace Cloudflare TURN with a self-hosted server if needed. |
| Pricing model risk | Medium | Per-GB billing with no self-host escape valve creates cost unpredictability at scale. |

**Verdict on vendor lock-in**: Cloudflare Calls is acceptable for phase 4b development (cost-controlled, no infrastructure burden) but carries higher long-term risk than LiveKit's self-host option. The procurement.md classifies Cloudflare Calls as "less operator control than LiveKit; acceptable fallback" — this audit concurs.

### 3.6 Pricing Model

| Tier | Cost | Notes |
|---|---|---|
| Development | $0.05/GB egress; first 1,000 GB/month free | Development traffic is typically <10 GB/month; well within free tier |
| Production | $0.05/GB beyond 1,000 GB free | Monthly free allowance reduces cost floor substantially vs. initial audit estimate |
| TURN | Included | Cloudflare's Anycast TURN is included in the per-GB rate |

**Comparison to LiveKit Cloud**: LiveKit Cloud has a free developer tier (up to a threshold of concurrent rooms); Cloudflare Realtime SFU also has a free tier (1,000 GB/month). Both are low-cost for development. LiveKit retains the advantage for self-host flexibility.

### 3.7 Integration Pattern for tze_hud (Corrected)

```
Phase 4b cloud-relay integration (Cloudflare Realtime SFU path):

1. tze_hud receives embodied session request (gRPC, RFC 0005)
2. Runtime selects cloud-relay path (C15, RFC 0018)
3. tze_hud initiates Cloudflare Realtime session via proprietary REST API:
   - POST /apps/{appId}/sessions/new  ← create session, get sessionId
   - POST /apps/{appId}/sessions/{sessionId}/tracks/new
       Content-Type: application/json
       Body: { "sessionDescription": { "sdp": "<offer>", "type": "offer" },
               "tracks": [ { "location": "local", ... } ] }
   - Receive JSON response: { "sessionDescription": { "sdp": "<answer>", "type": "answer" }, ... }
4. webrtc-rs (or str0m) completes ICE/DTLS using the SDP answer extracted from JSON
5. Media flows through Cloudflare relay to remote participant
6. GStreamer bridge operates identically to direct peer path (§4.2 of webrtc-rs-audit.md)
```

**Key difference from original audit**: There is no WHIP. The signaling is Cloudflare-proprietary JSON REST. tze_hud must implement a Cloudflare-specific REST adapter, not a reusable WHIP adapter. This adapter is not portable to LiveKit without a separate implementation.

**Alternative**: Deploy `cloudflare/realtime-examples/whip-whep-server` as a sidecar. This provides a WHIP facade and reduces the RFC 0018 adapter surface to standard WHIP, at the cost of an additional service to operate.

The transport library (webrtc-rs or str0m) and GStreamer bridge are unchanged.

### 3.8 Cloudflare Calls as a Transport Fallback

Cloudflare Calls is **not a transport library fallback**. It cannot replace webrtc-rs or str0m. It requires a WebRTC peer-connection implementation underneath. The question "fallback to Cloudflare Calls" is a category error — the correct framing is "use Cloudflare Calls as the C15 SFU, with webrtc-rs or str0m as the peer-connection library."

### 3.9 Re-verification Summary (hud-ejhnm, 2026-04-19)

This subsection documents the WHIP/WHEP verification performed for hud-ejhnm, reconciling claims made in the original PR #544 audit against the current Cloudflare Realtime documentation.

| Claim in original audit | Verified status | Evidence |
|---|---|---|
| "natively supports WHIP ... for push streams" | **INCORRECT** | Native SFU API uses `application/json` with SDP inside; no `application/sdp` endpoint exists |
| "natively supports ... WHEP ... for pull streams" | **INCORRECT** | Same finding; WHEP not a native SFU protocol |
| "Caller sends HTTP PUT with SDP offer body" | **INCORRECT** | Method is POST; body is JSON, not raw SDP |
| "WHIP support — Generally available (GA)" | **INCORRECT** | GA status applies to Cloudflare Stream (separate product); SFU has adapter example only |
| "WHEP support — GA" | **INCORRECT** | Same as above |
| WHIP/WHEP adapter exists | **CORRECT** | `cloudflare/realtime-examples/whip-whep-server` is an official Cloudflare example |
| Cloudflare Stream supports WHIP/WHEP | **CORRECT** | Confirmed; uses draft-ietf-wish-whip-06 and draft-murillo-whep-01 |
| Cloudflare Realtime SFU is a separate product from Stream | **CORRECT** | Confirmed; different API surface, different use case |

**Root cause of original error**: Cloudflare's public marketing conflates Realtime SFU and Cloudflare Stream under "Cloudflare Realtime." WHIP/WHEP coverage in Cloudflare blog posts and webrtchacks analyses primarily documents Cloudflare Stream's 2022 WHIP rollout, not the SFU. The original audit appears to have inherited this conflation.

**Phase 4b implication**: If Cloudflare Realtime SFU is selected as the C15 fallback, RFC 0018 must specify a proprietary REST adapter (or deployment of the whip-whep-server adapter) rather than a standard WHIP/WHEP integration. This makes Cloudflare a **higher adapter-cost** option relative to the original audit's assessment. LiveKit's WHIP endpoint remains a standard-protocol integration option and is unaffected by this finding.

---

## 4. Cross-Stack Comparison

### 4.1 Evaluation Criteria vs. Results

| Criterion | str0m 0.18 | LiveKit Rust SDK | Cloudflare Calls |
|---|---|---|---|
| **Layer** | Transport (peer-connection) | Client SDK (high-level SFU client) | SaaS SFU |
| **Pure-Rust** | Yes | No (`libwebrtc` FFI) | N/A (REST API) |
| **Direct webrtc-rs substitute** | Yes | No | No |
| **Simulcast** | Present (SFU-first) | Handled by LiveKit Server | Handled by CF Server |
| **SVC** | Not implemented | Not documented | Not documented |
| **AV1** | Present (since v0.15.0) | Depends on libwebrtc | Depends on CF server |
| **License** | MIT | Apache-2.0 | Proprietary SaaS |
| **Self-host** | N/A (library) | Apache-2.0 server available | No |
| **Maintainer durability** | Single maintainer + community | Backed by LiveKit Inc. | Backed by Cloudflare |
| **Production maturity** | Lookback (SFU) | LiveKit Server: high; Rust SDK: moderate | GA SaaS; high |
| **Migration effort from webrtc-rs** | Medium (2–4 weeks, transport swap) | High (architectural change required) | N/A (not a library) |
| **GStreamer bridge** | RTP payload events → manual RTP framing → appsrc | Decoded frames → appsrc raw | Unchanged from current path |
| **FFI surface** | None | `libwebrtc` (C++) | None (REST API) |
| **Signaling compatibility** | JSEP SDP (direct or via RFC 0014) | LiveKit protocol; WHIP optional | Proprietary JSON REST (WHIP available via adapter only) |
| **Phase 4b relevance** | Fallback transport library | C15 vendor candidate (server-side) | C15 vendor candidate |

### 4.2 Decision Tree for Phase 4b Kickoff

```
Phase 4b kickoff
    │
    ├─ hud-g89zs spike result: webrtc-rs v0.20 complete + simulcast OK?
    │       YES → Use webrtc-rs v0.20 (primary path)
    │       NO  → Use str0m 0.18 (recommended fallback)
    │
    └─ C15 vendor decision (orthogonal):
            ├─ LiveKit Cloud → preferred (self-host option, free dev tier)
            └─ Cloudflare Calls → fallback (higher vendor lock, no self-host)

        Both C15 options integrate with either transport library.
```

### 4.3 Signaling Layer Compatibility

Both str0m and webrtc-rs produce and consume JSEP SDP. The gRPC signaling plane (RFC 0005/0014) exchanges SDP strings over the gRPC control plane. This is transport-library-agnostic — swapping webrtc-rs for str0m does not change the signaling protocol.

For cloud-relay (C15), the signaling adapter sits between the gRPC plane and the SFU. The adapter type depends on C15 vendor selection:
- **LiveKit**: Standard WHIP/JSEP SDP exchange (portable, protocol-standard)
- **Cloudflare Realtime SFU**: Proprietary JSON REST adapter or self-hosted WHIP adapter sidecar (vendor-specific, higher adapter cost)

RFC 0018 (Cloud-Relay Trust Boundary spec, F29 gate) must specify the adapter contract before phase 4b implementation begins, and must account for this distinction between C15 candidates.

---

## 5. Recommendations

### 5.1 Primary Recommendation: str0m as Fallback Transport

If webrtc-rs v0.20 is not production-ready at phase 4b kickoff:

1. **Adopt str0m 0.18** as the WebRTC peer-connection library.
2. Rewrite the transport integration layer (ICE/DTLS/SRTP I/O loop) to the str0m sans-IO API.
3. Retain the GStreamer RTP bridge pattern; adapt for str0m's `MediaData` event payloads (vs. webrtc-rs's `track_remote.read()`).
4. Retain the gRPC signaling plane (RFC 0005/0014) — SDP is produced the same way.
5. Track str0m's AV1 support roadmap; if AV1 is required before phase 4 closes, evaluate custom RTP packetization or submit an upstream PR.

**What this does NOT require:**
- Changing the GStreamer decode pipeline (GStreamer is transport-library-agnostic).
- Changing the compositor or scene model.
- Changing the gRPC signaling protocol.
- Adding any C++ dependency.

### 5.2 C15 Vendor Recommendation: LiveKit

For the C15 cloud-relay vendor decision (due at phase 4b kickoff):

1. **Select LiveKit Cloud** for phase 4b development.
   - Apache-2.0 server available for self-host post-v2.
   - Free developer tier reduces cost floor.
   - More operator control than Cloudflare Calls.
   - Active Rust ecosystem (though tze_hud will not use the Rust SDK directly).

2. **Do not use the LiveKit Rust SDK** — connect tze_hud's webrtc-rs or str0m peer-connection directly to LiveKit Server using standard JSEP SDP. The LiveKit Rust SDK's `libwebrtc` FFI dependency and high-level abstraction are incompatible with tze_hud's architecture.

3. **Keep Cloudflare Calls as the documented C15 fallback** — it is viable, but carries higher vendor lock and no self-host escape.

### 5.3 Maturity Gate Recommendation for str0m

Before invoking the str0m fallback, verify:

| Gate | Check |
|---|---|
| TURN-over-TCP | Confirm str0m supports TCP TURN for firewall-restricted networks |
| Simulcast interop | Run str0m through the hud-fpq51 simulcast interop plan (Chrome/Firefox at minimum) |
| WHIP signaling compatibility (LiveKit) | Confirm str0m + WHIP with LiveKit Server produces correct ICE/DTLS negotiation |
| CF REST adapter compatibility | If Cloudflare SFU selected: confirm proprietary JSON REST adapter (or whip-whep-server) interoperates with str0m ICE/DTLS (WHIP not native to SFU) |
| AV1 status | Confirm AV1 is not required by phase 4 scope before committing to str0m |

These gates align with the hud-g89zs spike scope — the spike should be expanded to cover str0m validation if webrtc-rs v0.20 shows signs of slipping.

---

## 6. Discovered Follow-Ups

| Item | Priority | Notes |
|---|---|---|
| **str0m TURN-over-TCP validation** | P1 (phase 4 pre-condition) | Confirm TCP TURN works in str0m before invoking as fallback; add to hud-g89zs spike scope or create separate bead |
| **str0m simulcast browser interop test** | P1 (if fallback invoked) | Extend hud-fpq51 simulcast plan to include str0m variant; parallel to webrtc-rs simulcast track |
| **AV1 RTP packetization in str0m** | ~~P3~~ Resolved | AV1 packetizer/depacketizer present since v0.15.0; no gap to track. AV1 is still deferred per D18 regardless. |
| **RFC 0018 cloud-relay adapter spec** | P1 (phase 4b gate per F29) | The cloud-relay signaling adapter must be specced before phase 4b begins. Adapter type differs by C15 vendor: LiveKit uses standard WHIP/JSEP; Cloudflare Realtime SFU uses proprietary JSON REST or a WHIP adapter sidecar. RFC 0018 must specify both code paths if Cloudflare remains a named fallback. |
| **LiveKit Server self-host deployment guide** | P2 (post-v2) | For production cost management; not a v2 gate |

---

## 7. Summary

| Criterion | Assessment |
|---|---|
| **Recommended fallback** | str0m 0.18 — pure-Rust, sans-IO, simulcast-first, MIT license |
| **Trigger condition** | hud-g89zs spike returns webrtc-rs v0.20 simulcast incomplete or alpha unstable at phase 4b kickoff |
| **Migration effort** | Medium — transport layer only; GStreamer and compositor unchanged |
| **C15 vendor** | LiveKit Cloud preferred; Cloudflare Calls acceptable fallback — but carries higher adapter cost than originally assessed (proprietary REST API; no native WHIP) |
| **LiveKit Rust SDK** | Not a transport fallback; not recommended (libwebrtc FFI, architectural mismatch) |
| **Cloudflare Calls** | Not a transport fallback; SaaS SFU for C15 vendor decision only |
| **Key gap in str0m** | No AV1 gap — AV1 RTP packetization present since v0.15.0. Primary gap: TURN-over-TCP validation pending. |
| **Key validation gate** | str0m simulcast interop test (Chrome/Firefox) before phase 4b opens |

**Primary verdict**: Monitor webrtc-rs v0.20 progress via hud-g89zs. If the spike returns complete simulcast interceptors and a stable API, stay on webrtc-rs v0.20. If the spike reveals slippage, adopt str0m 0.18 — the architectural cost is low, the pure-Rust posture is preserved, and the migration path is well-understood.

---

## Sources

- str0m GitHub repository: https://github.com/algesten/str0m
- str0m crates.io: https://crates.io/crates/str0m
- str0m docs.rs: https://docs.rs/str0m
- LiveKit Rust SDKs: https://github.com/livekit/rust-sdks
- LiveKit Server: https://github.com/livekit/livekit
- LiveKit Cloud pricing: https://livekit.io/cloud
- Cloudflare Realtime documentation: https://developers.cloudflare.com/realtime/ (rebranded from Cloudflare Calls in 2026)
- Cloudflare Realtime SFU overview: https://developers.cloudflare.com/realtime/sfu/
- Cloudflare Realtime SFU HTTPS API: https://developers.cloudflare.com/realtime/sfu/https-api/
- Cloudflare Realtime SFU pricing: https://developers.cloudflare.com/realtime/sfu/pricing/
- Cloudflare Realtime SFU demos (includes WHIP/WHEP adapter): https://developers.cloudflare.com/realtime/sfu/demos/
- Cloudflare Realtime vs regular SFUs: https://developers.cloudflare.com/realtime/sfu/calls-vs-sfus/
- Cloudflare Realtime SFU OpenAPI schema: https://developers.cloudflare.com/realtime/static/calls-api-2024-05-21.yaml
- Cloudflare Realtime SFU WHIP/WHEP example adapter: https://github.com/cloudflare/realtime-examples/tree/main/whip-whep-server
- Cloudflare Stream WHIP/WHEP (separate product): https://developers.cloudflare.com/stream/webrtc-beta/
- Cloudflare blog — Calls architecture: https://blog.cloudflare.com/cloudflare-calls-anycast-webrtc/
- webrtcHacks — Cloudflare WHIP/WHEP analysis: https://webrtchacks.com/how-cloudflare-glares-at-webrtc-with-whip-and-whep/
- WHIP RFC 9725 (WebRTC-HTTP Ingestion Protocol — final): https://datatracker.ietf.org/doc/rfc9725/
- WHEP RFC draft (WebRTC-HTTP Egress Protocol): https://datatracker.ietf.org/doc/html/draft-murillo-whep
- webrtc-rs audit (predecessor): `docs/audits/webrtc-rs-audit.md`
- webrtc-rs v0.20 readiness spike: hud-g89zs
- Phase 4 simulcast plan: hud-fpq51 (`docs/testing/simulcast-interop-plan.md`)
- v2 signoff packet (C15, F29): `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
- v2 procurement list (phase 4b SFU): `openspec/changes/v2-embodied-media-presence/procurement.md`
- E24 in-process worker posture: `docs/decisions/e24-in-process-worker-posture.md`
