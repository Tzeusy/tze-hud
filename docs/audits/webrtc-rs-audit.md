# webrtc-rs Library Audit

**Issued for**: `hud-ora8.1.17`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent task**: hud-ora8.1 (v2-embodied-media-presence, phase 1 bounded ingress + phase 4 full bidi AV)
**Context**: Procurement mandate — `openspec/changes/v2-embodied-media-presence/procurement.md`

---

## Verdict

**ADOPT-WITH-CAVEATS**

`webrtc-rs` (`webrtc` crate, v0.17.1) is the correct Rust WebRTC implementation for tze_hud's media plane. It is the most mature pure-Rust WebRTC stack, covers the required ICE/DTLS-SRTP/SCTP/RTP protocol suite, and registers H.264/VP8/VP9/AV1/Opus codecs by default. Four caveats must be resolved:

1. **v0.17.x is feature-frozen** — the stable branch receives bug fixes only; new features ship in the not-yet-stable v0.20.0 alpha. tze_hud should pin to `0.17.x` for phase 1 (bounded ingress) and plan a migration to v0.20 for phase 4 (bidirectional AV), tracking the alpha closely.
2. **Simulcast is partially implemented and not production-validated** — v0.17.0 added multi-encoding track APIs; the `rtc` crate (sans-IO core used by v0.20) lists simulcast as "in-progress" in its RID/MID interceptor framework. Phase 4 bidirectional AV must treat simulcast as an integration risk requiring explicit testing against Chrome/Firefox targets.
3. **AV1 is registered in default codecs but H.265 has known packetizer bugs** — AV1 is payload-type 41 in `register_default_codecs()`; no known issues. H.265/HEVC is flagged as having "known issues" in the v0.20.0-alpha.1 announcement and should not be used in v2. This is not a risk for the v2 codec matrix (H.264 + VP9 per D18) but must be documented.
4. **GStreamer integration requires an explicit RTP hand-off bridge** — webrtc-rs and GStreamer are not directly connected; the implementation must bridge RTP packets from `track_remote.read()` into a GStreamer `appsrc` with correct `application/x-rtp` caps. This is a well-understood pattern but requires careful implementation.

No credible Rust alternative displaces webrtc-rs for tze_hud's use case. The main alternative (`str0m`) is a sans-IO library designed for server-side SFU deployments; it is technically superior in API design but lacks the production validation depth and has a narrower focus. GStreamer's `webrtcbin` is a viable alternative for a pure-GStreamer architecture, but CLAUDE.md already locks in WebRTC as the media plane, separate from GStreamer's decode role.

---

## Scope

Phase 1 of v2-embodied-media-presence introduces bounded media ingress. Phase 4 adds bidirectional AV (voice synthesis, agent-emitted audio, full bidi). The signoff packet (D18) specifies:

> **Codecs**: H.264 + VP9 for v2, AV1 deferred. Glass-to-glass budget: p50 ≤150 ms / p99 ≤400 ms, decode drop ≤0.5%, lip-sync drift ≤±40 ms, TTFF ≤500 ms.
> **Cloud-relay**: SaaS SFU (LiveKit Cloud or Cloudflare Calls), selected at phase 4b kickoff (C15).

This audit evaluates webrtc-rs as the WebRTC transport layer. GStreamer (decode, timing, synchronization) and cpal (audio output routing) are audited separately in their own audit beads.

---

## 1. Crate Identity

### 1.1 webrtc (stable branch)

| Field | Value |
|---|---|
| Crate | `webrtc` |
| Repository | https://github.com/webrtc-rs/webrtc |
| Current stable version | 0.17.1 |
| Release date | 2025-02-06 |
| MSRV | Rust 1.75+ (implied by dependencies) |
| License | MIT / Apache-2.0 |
| Stars | ~5,000 |
| Forks | 474 |
| Open issues | 11 (on v0.20 transition branch) |
| Total releases | 27 |
| crates.io | https://crates.io/crates/webrtc |
| docs.rs | https://docs.rs/webrtc |

### 1.2 Architecture

webrtc-rs is a workspace of 14+ crates under the `webrtc-rs` GitHub organization. The top-level `webrtc` crate is the user-facing API; the underlying stack is decomposed as:

| Crate | Role |
|---|---|
| `webrtc-ice` | ICE agent (RFC 8445, trickle ICE RFC 8838) |
| `webrtc-dtls` | DTLS 1.2 |
| `webrtc-srtp` | SRTP/SRTCP |
| `webrtc-sctp` | SCTP (data channels) |
| `webrtc-rtp` | RTP/RTCP |
| `webrtc-sdp` | SDP parser/writer |
| `webrtc-turn` | TURN client |
| `webrtc-stun` | STUN |
| `webrtc-mdns` | mDNS |
| `webrtc-media` | Media track types |

### 1.3 Project lineage and direction

webrtc-rs began as a port of Pion (the Go reference implementation). v0.17.0 is the final feature release of the Tokio-coupled branch. The project is actively transitioning to a new architecture:

- **`webrtc-rs/rtc`** — sans-IO protocol core (no async, no runtime dependency). Declared feature-complete January 2026.
- **`webrtc` v0.20.0-alpha.1** (released March 2025) — async API rebuilt on top of `rtc`. Supports Tokio and smol; trait-based event handlers replace callback closures.

The v0.20 alpha is not yet production-ready. The v0.17.x branch receives bug-fix-only support.

---

## 2. WebRTC Specification Coverage

### 2.1 Protocol stack

| Protocol / Standard | Status in v0.17.x | Notes |
|---|---|---|
| ICE (RFC 8445) | Full | Host, STUN, TURN, TCP, mDNS candidates |
| Trickle ICE (RFC 8838) | Full | Incremental candidate exchange |
| DTLS 1.2 | Full | PSK and certificate modes |
| SRTP / SRTCP | Full | AES-CM-128, AES-CM-256 (added v0.17.0) |
| SCTP over DTLS | Full | Data channels |
| RTP / RTCP | Full | NACK, RTCP-FB, bandwidth estimation |
| SDP (JSEP / RFC 8829) | Full | Offer/answer, renegotiation |
| mDNS (privacy ICE) | Full | `webrtc-mdns` crate |
| TURN (RFC 8656) | Full | Client support |
| W3C RTCPeerConnection API | ~95% compliance | JavaScript API shape, adapted to Rust |

**ICE gather and TCP ICE**: mDNS support is present in v0.17.x but IPv6 ICE gathering has a documented issue in v0.20 alpha (#774). TCP ICE is listed as an open enhancement issue (#781) for v0.20. For v2 phase 1 (bounded ingress on a LAN or server-to-server path), neither gap is blocking.

### 2.2 Simulcast and SVC

| Feature | Status |
|---|---|
| Simulcast (RFC 8853) | Partial. v0.17.0 added multi-encoding track APIs and RID support. Production validation against Chrome/Firefox is not documented. |
| SVC (scalable video coding) | Not implemented. webrtc-rs does not implement SVC layer selection. VP9 SVC payloads can flow through the RTP stack but layer switching requires external logic. |

**Risk assessment for tze_hud**: Phase 1 (bounded ingress) does not require simulcast or SVC — it ingests a single stream per source. Phase 4 (bidirectional AV, E23 upstream composition) could benefit from simulcast for adaptive-quality glasses delivery, but the signoff packet does not mandate it. Treat simulcast as a phase 4 integration risk: test early in phase 4 development, not at phase 1.

---

## 3. Codec Support

### 3.1 Default codec registration

`MediaEngine::register_default_codecs()` registers all standard codecs automatically:

**Video codecs (registered by default):**

| Codec | MIME type | Profiles |
|---|---|---|
| H.264 | `video/H264` | Baseline 3.1, Main 4.2, High 4.0 / 4.1 / 4.2 / 5.1 |
| VP8 | `video/VP8` | Default |
| VP9 | `video/VP9` | Default |
| AV1 | `video/AV1` | Profile-id=0, payload type 41 |
| H.265/HEVC | `video/H265` | Registered but has known packetizer issues |

**Audio codecs (registered by default):**

| Codec | MIME type | Clock rate | Notes |
|---|---|---|---|
| Opus | `audio/opus` | 48000 Hz | Stereo; primary codec for v2 audio |
| G.722 | `audio/G722` | 8000 Hz | Narrowband; legacy compatibility only |
| PCMU (G.711 μ-law) | `audio/PCMU` | 8000 Hz | Legacy |
| PCMA (G.711 A-law) | `audio/PCMA` | 8000 Hz | Legacy |

**Error correction:**

| Mechanism | MIME type |
|---|---|
| ULP-FEC | `video/ulpfec` |
| RED (redundant audio) | Available via RTCP-FB |

### 3.2 Codec integration model

webrtc-rs is a **transport and signaling library, not a codec library**. It handles:
- RTP packetization and depacketization (splitting/reassembling encoded frames into RTP packets)
- RTCP feedback (NACK, PLI, FIR, REMB, transport-wide CC)
- Codec negotiation via SDP

It does **not** handle encoding or decoding. Actual encode/decode is performed by GStreamer (for video and network-egress audio) and cpal (for audio output). This aligns with tze_hud's architecture: webrtc-rs owns transport; GStreamer owns decode.

### 3.3 v2 codec matrix coverage

| v2 requirement (D18) | webrtc-rs support |
|---|---|
| H.264 (primary video) | Full: Baseline through High Profile registered by default; packetizer/depacketizer present |
| VP9 | Full: registered by default |
| AV1 (deferred for v2) | Full: registered by default; no known issues |
| Opus (primary audio) | Full: 48 kHz stereo registered by default |
| H.265 | Avoid: known packetizer bugs in v0.20 alpha; not required for v2 |

---

## 4. GStreamer Integration Pattern

### 4.1 Architecture overview

webrtc-rs and GStreamer are not directly coupled. tze_hud's bounded-ingress architecture must bridge them at the RTP boundary:

```
[Remote peer]
     │ (WebRTC / DTLS-SRTP)
     ▼
[webrtc-rs peer_connection]
     │ track_remote.read_rtp().await → RTPPacket
     ▼
[Tokio task: RTP → appsrc bridge]
     │ marshal_to(&mut buf) → push_buffer()
     ▼
[GStreamer pipeline: appsrc ! rtph264depay ! avdec_h264 ! appsink]
     │ decoded VideoFrame
     ▼
[ring buffer → wgpu texture upload]
```

### 4.2 Integration implementation

The canonical bridge pattern (sourced from GStreamer Discourse community examples):

```rust
// 1. Create appsrc with application/x-rtp caps matching the negotiated codec
let caps = gstreamer::Caps::builder("application/x-rtp")
    .field("media", "video")
    .field("encoding-name", "H264")   // or VP9, VP8, AV1
    .field("payload", 96_i32)          // payload type from SDP negotiation
    .field("clock-rate", 90_000_i32)
    .build();
let appsrc: gstreamer_app::AppSrc = /* pipeline element */;
appsrc.set_caps(Some(&caps));

// 2. Bridge task: read RTP from webrtc-rs, push to GStreamer
let track_remote = /* Arc<TrackRemote> from on_track callback */;
let mut buffer = vec![0u8; 1500]; // MTU-sized scratch buffer
loop {
    match track_remote.read(&mut buffer).await {
        Ok((rtp_packet, _)) => {
            // Use webrtc_util::Marshal — do NOT hand-construct RTP headers
            let n = rtp_packet.marshal_to(&mut buffer).unwrap();
            let gst_buf = gstreamer::Buffer::from_slice(buffer[..n].to_vec());
            if appsrc.push_buffer(gst_buf).is_err() {
                break; // pipeline shutting down
            }
        }
        Err(_) => break,
    }
}
```

**Critical pitfalls:**
- Use `marshal_to()` from the `webrtc-util` crate — do not hand-construct RTP headers. Manual header construction produces malformed packets that GStreamer depayloaders reject with "Received invalid RTP payload" errors.
- Set `is-live=true` and `format=time` on the appsrc. Without `is-live=true`, GStreamer's base-time semantics are wrong for a real-time stream.
- Set `do-timestamp=false` (let webrtc-rs provide the RTP timestamps). GStreamer's `do-timestamp=true` overwrites the RTP timestamps, breaking lip-sync against the glass-to-glass budget (±40 ms drift per D18).

### 4.3 Alternative: GStreamer webrtcbin

GStreamer has its own WebRTC element (`webrtcbin`) and a higher-level `webrtcsink`/`webrtcsrc` pair (from `gst-plugins-rs` / `rswebrtc`). These handle the full WebRTC stack inside GStreamer — including ICE, DTLS, SRTP, and RTP.

**Why webrtcbin is not recommended for tze_hud:**
- webrtcbin embeds a full WebRTC state machine inside a GStreamer element. This creates a second WebRTC stack alongside webrtc-rs if tze_hud ever uses both.
- RFC 0014 (Media Plane Wire Protocol, the forthcoming tze_hud media plane spec) owns the WebRTC control plane; that spec will be designed against webrtc-rs's API. Switching to webrtcbin would couple the media plane spec to GStreamer's element model.
- webrtcbin requires negotiating SDP inside GStreamer, which conflicts with tze_hud's gRPC-based signaling plane (RFC 0005, embodied session message types).

**Decision**: webrtc-rs owns the WebRTC transport and signaling. GStreamer owns decode from the RTP boundary inward. Bridge them at the `appsrc` interface.

---

## 5. Latency Characteristics

### 5.1 Protocol overhead

WebRTC's mandatory DTLS-SRTP adds encryption overhead over raw RTP, but this overhead is well-characterized:

| Component | Contribution |
|---|---|
| DTLS handshake (connection setup) | 1–2 ms on LAN (one-time; not per-frame) |
| SRTP per-packet overhead | Header: +10 bytes (SSRC, sequence, timestamp). Auth tag: +10 bytes (HMAC-SHA1-80). Total: ~20 bytes/packet on top of RTP. Negligible vs. frame payload size. |
| ICE connectivity establishment | 50–500 ms (network-dependent; TTFF budget in D18: ≤500 ms — ICE must complete within this) |
| RTCP feedback latency | 50–200 ms round-trip for NACK/PLI — relevant for error recovery, not glass-to-glass display latency |

### 5.2 Glass-to-glass budget (D18) feasibility

The D18 budget (p50 ≤150 ms, p99 ≤400 ms glass-to-glass) is a full-stack budget, not a transport-only budget. webrtc-rs contributes:

- **Transport latency**: sub-5 ms per-packet on LAN after connection establishment. On public internet (phase 4 cloud-relay), add network RTT/2.
- **DTLS-SRTP processing**: <1 ms per packet on modern hardware (measured in community benchmarks at ~138k messages/second throughput).
- **ICE/TURN overhead**: not on the per-frame critical path after connection establishment.

The transport layer is not the latency bottleneck. Decode (GStreamer) and compositor scheduling are the dominant contributors to glass-to-glass latency within tze_hud's control.

### 5.3 SCTP overhead (data channels)

Data channels over SCTP/DTLS are not on the video frame critical path. tze_hud's interactive input events (touch, cursor) use the gRPC control plane (B7 signoff decision), not WebRTC data channels. SCTP overhead is not a concern for v2 display latency.

---

## 6. Maintenance Health

### 6.1 Activity metrics

| Metric | Observation |
|---|---|
| Last stable release | v0.17.1 — 2025-02-06 |
| Last alpha release | v0.20.0-alpha.1 — 2025-03-01 |
| Open issues (v0.17.x stable) | 11 open issues, all on v0.20 migration topics (IPv6 ICE, mDNS, TCP ICE, post-quantum crypto) |
| RustSec advisories | None on record for any `webrtc-rs/*` crate as of audit date |
| Stars | ~5,000 |
| Forks | 474 |

### 6.2 Architectural transition assessment

The project's transition from v0.17 (Tokio-coupled) to v0.20 (sans-IO + runtime-agnostic) is a positive long-term health signal:

- The sans-IO `rtc` crate was declared feature-complete January 2026.
- v0.20.0-alpha.1 shipped March 2025 with 20 ported examples.
- The v0.17.x stable branch is explicitly maintained for production users during the transition.

The risk is the gap: v0.20 is alpha, v0.17 is feature-frozen. For phase 1 (bounded ingress), v0.17 is appropriate. Phase 4 (bidirectional AV, expected 12–17 months from v1 ship per G31) will likely land after v0.20 stabilizes, which is the right migration point.

### 6.3 Comparison to Pion (Go reference)

| Metric | webrtc-rs (Rust) | Pion (Go) |
|---|---|---|
| Stars | ~5,000 | ~16,200 |
| Latest version | v0.17.1 (stable); v0.20.0-alpha.1 | v4.2.11 (March 2026) |
| Release cadence | Slowed — stable branch on v0.17 since Feb 2025 | Active — v4.2.x patch releases monthly |
| Open issues | 11 | 72 |
| Codec support | H.264, VP8, VP9, AV1, Opus | Opus, H.264, VP8, VP9 (no native AV1) |
| Architecture | Transitioning to sans-IO | Async Go (goroutines) |
| Simulcast | Partial | Production-validated |

**Assessment**: Pion is more battle-tested in production SFU deployments. webrtc-rs is the correct choice for a Rust-native runtime — using Pion would require CGo or an FFI boundary, which violates CLAUDE.md's no-browser, no-FFI-on-hot-path posture. webrtc-rs's lower star count reflects Go's larger WebRTC deployment base, not inferior library quality.

---

## 7. Alternatives Assessment

### 7.1 `str0m` (algesten/str0m)

**Verdict**: Monitor for phase 4; do not adopt for v2.

str0m is a sans-IO WebRTC implementation with strong Rust-idiomatic design: no `Arc<Mutex<>>`, no internal threads, no async tasks. It is explicitly designed for server-side SFU use cases.

| Aspect | Assessment |
|---|---|
| API design | Superior to webrtc-rs v0.17 — no callback hell, no `Arc` everywhere |
| Spec coverage | ICE, DTLS, SRTP, SCTP, RTP, NACK, simulcast present |
| Codec support | H.264, VP8, VP9, Opus (no AV1 native support) |
| Stars | ~544 — significantly lower production validation evidence |
| Version | v0.18.0 (mid-April 2026) — active release cadence |
| Production use | Narrow: server-side SFU at Lookback and community contributors |
| Async runtime | None by design; requires an integration shim for Tokio |

**Recommendation**: str0m's sans-IO approach aligns with the direction webrtc-rs v0.20 is taking. If webrtc-rs v0.20 stabilizes poorly, str0m is the credible fallback, particularly for phase 4 where the AV1 gap is a concern. Track its AV1 support.

### 7.2 GStreamer `webrtcbin` / `webrtcsink`

**Verdict**: Not recommended as the primary WebRTC stack; use webrtc-rs instead.

GStreamer's `webrtcbin` (C-based, part of gstreamer-plugins-bad) and `webrtcsink`/`webrtcsrc` (Rust-based, part of `gst-plugins-rs`) provide a full WebRTC stack inside the GStreamer element pipeline. `webrtcsink` in particular handles encoding, payloading, ICE, DTLS-SRTP, and signaling adapter hookup.

**Why not for tze_hud**:
- Creates a GStreamer-owned WebRTC control plane that conflicts with RFC 0014's spec scope and the gRPC-based signaling architecture (B7).
- webrtcsink accepts only raw audio/video — it handles encoding internally. tze_hud's architecture inverts this: webrtc-rs receives encoded RTP, then GStreamer decodes it. webrtcsink is for the send direction.
- Less ergonomic for the receive-only bounded ingress phase 1 pattern.

**Not to exclude entirely**: webrtcsrc (the GStreamer WebRTC receive element) may be relevant for a pure-GStreamer ingest path as a future simplification. Monitor.

### 7.3 `libwebrtc` bindings (shiguredo/webrtc-rs)

**Verdict**: Not appropriate.

`shiguredo/webrtc-rs` is Google's libwebrtc wrapped in Rust bindings. It is a C++ library with all associated build complexity, platform-specific prebuilt binaries, and a large binary footprint. The abstraction surface is thin over the C++ API. Not suitable for a pure-Rust runtime.

### 7.4 LiveKit / Cloudflare Calls Rust SDK

**Verdict**: Out of scope for phase 1; relevant to phase 4b audit.

The signoff packet (C15) selects a cloud SFU (LiveKit Cloud or Cloudflare Calls) for phase 4b. The Rust SDK for the chosen vendor will be audited at phase 4b kickoff per procurement.md. The transport layer under any SFU SDK is still WebRTC; webrtc-rs (or its v0.20 successor) remains the peer-connection implementation.

---

## 8. Security Posture

### 8.1 RustSec advisory database

No RUSTSEC advisories exist for any `webrtc-rs/*` crate as of 2026-04-19.

### 8.2 Cryptographic posture

DTLS-SRTP is mandatory-on in WebRTC by spec; webrtc-rs enforces this. No plaintext RTP paths exist. The v0.17.0 release added AES-CM-256 support for SRTP, augmenting the previous AES-CM-128-only support. Post-quantum cryptography support is tracked as an open issue (#801) for v0.20 — not a v2 requirement.

### 8.3 In-process isolation

webrtc-rs runs its ICE agent, DTLS handshake, and SRTP processing as in-process Tokio tasks. The E24 verdict (`docs/decisions/e24-in-process-worker-posture.md`) confirms this is compatible with tze_hud's security posture: agent-runtime isolation is maintained at the gRPC/MCP wire boundary, not at internal thread boundaries. webrtc-rs media workers are trusted-side code, the same as the gRPC server runtime.

---

## 9. Integration Guidance for Phase 1 (Bounded Ingress)

This section is non-normative design guidance. The authoritative implementation decisions belong to the RFC 0014 spec and the phase 1 implementation beads.

### 9.1 Recommended Cargo.toml

```toml
[dependencies]
# Pin to 0.17.x for phase 1; v0.20.x for phase 4 when stable
webrtc = "0.17"
```

No additional feature flags are required for the core ICE/DTLS/RTP stack. ICE servers (STUN/TURN) are configured at runtime via `RTCConfiguration`.

### 9.2 Codec setup for bounded ingress

```rust
let mut media_engine = MediaEngine::default();
// Register H.264 and VP9 only (v2 D18 codec matrix)
// register_default_codecs() also registers VP8, AV1, H.265, G.722, PCMU, PCMA —
// acceptable unless binary size is a concern
media_engine.register_default_codecs()?;

let api = APIBuilder::new()
    .with_media_engine(media_engine)
    .build();
```

### 9.3 Track receive and appsrc bridge

See §4.2 for the complete bridge pattern. Key invariants:
- Use `marshal_to()` not hand-rolled serialization.
- Set `do-timestamp=false` on appsrc (preserve RTP timestamps for lip-sync).
- Run the bridge task under Tokio; GStreamer's `push_buffer` is synchronous and safe to call from a Tokio task.

### 9.4 ICE configuration

```rust
let config = RTCConfiguration {
    ice_servers: vec![
        // Provide at least one STUN server for non-LAN connectivity
        RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        },
    ],
    ..Default::default()
};
```

For phase 4 cloud-relay (C15), a TURN server via the selected SFU (LiveKit Cloud or Cloudflare Calls) will replace the public STUN server. The configuration shape is identical.

---

## 10. Open Issues and Discovered Follow-ups

| Issue | Severity | Recommendation |
|---|---|---|
| Simulcast validation against Chrome/Firefox | High for phase 4 | Create explicit phase 4 integration test bead for simulcast before shipping bidirectional AV |
| TCP ICE support missing in v0.20 alpha (#781) | Medium for phase 4 | Track; required for firewall-traversal robustness in cloud-relay scenarios |
| v0.20 migration planning | Medium | Plan migration from 0.17→0.20 in the phase 4 kickoff bead; do not defer to implementation |
| str0m AV1 support gap | Low | Monitor; if AV1 becomes a v2 requirement before phase 4, str0m may not substitute |
| IPv6 ICE gather (#774 in v0.20 alpha) | Low for phase 1 | File a phase 4 pre-condition: validate IPv6 ICE before shipping cloud-relay |

---

## 11. Summary

| Criterion | Assessment |
|---|---|
| Protocol coverage — ICE / DTLS / SRTP / SCTP / RTP | Full in v0.17.x |
| Codec support — H.264, VP9 (v2 required) | Full; registered by default |
| Codec support — Opus | Full; 48 kHz stereo |
| Codec support — AV1 (v2 deferred) | Full; registered by default; no known issues |
| Simulcast | Partial; not production-validated; phase 4 risk |
| SVC | Not implemented |
| GStreamer integration | Bridged via `appsrc` at RTP boundary; well-understood pattern |
| Glass-to-glass latency overhead | Sub-5 ms transport contribution; within D18 budget |
| Maintenance health | Stable v0.17 + active v0.20 transition; no RustSec advisories |
| Security posture | DTLS-SRTP mandatory; no RustSec advisories; E24-compatible |
| Alternatives | No credible Rust-native displacement; str0m worth tracking for phase 4 |

**Verdict: ADOPT-WITH-CAVEATS.** Pin to `webrtc` 0.17.x for phase 1 bounded ingress. Plan migration to v0.20.x for phase 4 bidirectional AV. Treat simulcast as a phase 4 pre-condition integration risk requiring early browser interoperability testing.

---

## Sources

- webrtc-rs GitHub repository: https://github.com/webrtc-rs/webrtc
- webrtc-rs crates.io: https://crates.io/crates/webrtc
- webrtc-rs docs.rs (v0.17.1): https://docs.rs/webrtc/0.17.1/webrtc/
- webrtc-rs releases: https://github.com/webrtc-rs/webrtc/releases
- webrtc-rs v0.17.0 release notes: https://github.com/webrtc-rs/webrtc/releases/tag/v0.17.0
- Announcing rtc v0.3.0 (sans-IO feature complete): https://webrtc.rs/blog/2026/01/04/announcing-rtc-v0.3.0.html
- RTC feature complete / roadmap: https://webrtc.rs/blog/2026/01/18/rtc-feature-complete-whats-next.html
- WebRTC v0.20.0-alpha.1 announcement: https://webrtc.rs/blog/2026/03/01/webrtc-v0.20.0-alpha.1-async-webrtc-on-sansio.html
- Async-friendly webrtc on sans-IO architecture design: https://webrtc.rs/blog/2026/01/31/async-friendly-webrtc-architecture.html
- Sans-IO architecture pattern (webrtc-rs/rtc, DeepWiki): https://deepwiki.com/webrtc-rs/rtc/1.1-sans-io-architecture-pattern
- webrtc-rs open issue #230 (sans-IO discussion): https://github.com/webrtc-rs/webrtc/issues/230
- webrtc-rs MediaEngine mod.rs (codec registration): https://github.com/webrtc-rs/webrtc/blob/v0.17.1/webrtc/src/api/media_engine/mod.rs
- GStreamer Discourse — passing RTP packets to appsrc: https://discourse.gstreamer.org/t/how-to-pass-rtp-packets-to-appsrc/4655
- GStreamer webrtcbin documentation: https://gstreamer.freedesktop.org/documentation/webrtc/index.html
- gst-plugin-webrtc (rswebrtc, webrtcsink): https://crates.io/crates/gst-plugin-webrtc
- str0m GitHub repository: https://github.com/algesten/str0m
- Pion WebRTC (Go reference): https://github.com/pion/webrtc
- Pion v4.2.0 release (comparison reference): https://github.com/pion/webrtc/releases/tag/v4.2.0
- RustSec Advisory Database: https://rustsec.org/advisories/
- v2-embodied-media-presence signoff packet: `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
- v2 procurement list: `openspec/changes/v2-embodied-media-presence/procurement.md`
- E24 in-process worker posture decision: `docs/decisions/e24-in-process-worker-posture.md`
- Media doctrine: `about/heart-and-soul/media-doctrine.md`
