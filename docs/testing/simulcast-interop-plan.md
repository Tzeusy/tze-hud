# Phase 4 Simulcast Interop Test Plan

**Issue:** `hud-fpq51`
**Parent audit:** `hud-ora8.1.17` (webrtc-rs library audit, PR #523)
**Date:** 2026-04-19
**Amended:** 2026-04-19 by `hud-khx6u` — added §7 str0m fallback path; renumbered §7–9 to §8–10
**Status:** Design document — Phase 4 pre-flight, test harness not yet implemented
**Blocks:** Phase 4b bidirectional AV ship

---

## 1. Background and Motivation

The webrtc-rs library audit (`hud-ora8.1.17`, PR #523) returned a verdict of
**ADOPT-WITH-CAVEATS**. Caveat 2 reads:

> **Simulcast is partially implemented and not production-validated** — v0.17.0
> added multi-encoding track APIs and RID support. The `rtc` crate (sans-IO core
> used by v0.20) lists simulcast as "in-progress" in its RID/MID interceptor
> framework. Phase 4 bidirectional AV must treat simulcast as an integration
> risk requiring explicit testing against Chrome/Firefox targets.

Phase 1 (bounded ingress) does not require simulcast — it ingests a single
stream per source. Phase 4 bidirectional AV (sub-epic 4f, voice synthesis +
agent-emitted audio + full bidi) introduces agent-emitted WebRTC streams,
potentially targetted to heterogeneous endpoints (desktop, mobile, glasses). At
that point, simulcast becomes relevant for adaptive quality delivery: a single
agent-origin can broadcast at multiple spatial/temporal resolutions, and the SFU
(LiveKit Cloud or Cloudflare Calls, selected at phase 4b kickoff per C15) or the
endpoint can select the appropriate layer.

This document captures:

- What the webrtc-rs simulcast support matrix looked like at audit time
- The required browser × codec interop matrix
- Test methodology for an agent-driven harness
- Where such tests would live in the codebase
- The go/no-go gate definition for Phase 4b ship

---

## 2. webrtc-rs Simulcast Support Matrix (Audit Snapshot)

The following table reflects webrtc-rs status as of the 2026-04-19 audit
(`hud-ora8.1.17`). It must be re-verified at phase 4 kickoff before implementation
begins.

| Feature | webrtc v0.17.x (pin target for phase 1) | webrtc v0.20.x (migration target for phase 4) | Status |
|---|---|---|---|
| Multi-encoding track API | Partial — `TrackLocalSimulcast` wrapper present | Ported from v0.17 to v0.20 via `rtc` sans-IO core | PARTIAL |
| RID (RTP Stream ID, RFC 8851) | Present in v0.17.0 release | `rtc` crate: RID interceptor listed as in-progress | IN-PROGRESS |
| MID (BUNDLE extension, RFC 9143) | Present | `rtc` crate: MID interceptor listed as in-progress | IN-PROGRESS |
| Simulcast SDP negotiation (RFC 8853) | Partial | Not complete in v0.20 alpha as of March 2026 | PARTIAL |
| Layer switching / rid selection | Not present in v0.17 | Under development in `rtc` | NOT IMPLEMENTED |
| RTCP REMB / transport-wide CC for layer selection | Present (RTCP feedback handled) | Present | PRESENT |
| SVC (scalable video coding) | Not implemented | Not targeted for v0.20 | NOT PLANNED |
| Production browser interop (Chrome/Firefox/Safari) | Not documented | Not documented | UNVALIDATED |

**Key gaps requiring phase 4 work:**

1. RID/MID interceptors not complete in v0.20 alpha — the migration plan must
   account for this landing before phase 4 implementation begins.
2. Layer switching must be implemented in tze_hud's media worker layer, using
   RTCP feedback from the SFU.
3. Browser interop is undocumented in webrtc-rs for simulcast paths — this is
   the principal validation gap.

---

## 3. Browser × Codec Interop Matrix

Phase 4b (cloud-relay) delivers media through a SaaS SFU (LiveKit Cloud or
Cloudflare Calls). The interop matrix covers the browsers that operator
households or end-user companion apps might use to view agent-emitted streams.

### 3.1 Target browser versions

| Browser | Minimum version | Notes |
|---|---|---|
| Chrome (desktop) | 110+ | Simulcast + VP9 SVC stable from Chrome 91; prefer 110+ for Plan-B → Unified Plan migration complete |
| Chrome (Android) | 110+ | Same codebase as desktop for WebRTC; verify hardware encode paths |
| Firefox (desktop) | 113+ | VP9 simulcast support landed in FF 113; H.264 simulcast requires explicit config |
| Firefox (Android) | 113+ | Same as desktop |
| Safari (macOS) | 16.4+ | VP9 support added in Safari 16.4; simulcast support is limited — see §3.3 |
| Safari (iOS) | 16.4+ | Same codebase as macOS WebKit; verify WebRTC restrictions in WKWebView |

### 3.2 Required codec matrix

Per D18 and RFC 0014 (planned), the v2 codec matrix is H.264 + VP9. AV1 is
deferred. The interop test must cover all cells in this matrix:

| Browser | H.264 Simulcast | VP9 Simulcast | Notes |
|---|---|---|---|
| Chrome | MUST PASS | MUST PASS | Chrome's simulcast is most mature; use as baseline |
| Firefox | MUST PASS | MUST PASS | VP9 simulcast requires `vp9_simulcast` config flag until FF ~120 |
| Safari | MUST PASS | CONDITIONAL | Safari VP9 simulcast is experimental; pass required only if Safari is in the phase 4b device matrix |

**CONDITIONAL cells**: A CONDITIONAL result means the test run is informational
but not blocking. Failure produces a phase 4b ship risk note, not a hard gate.
The operator must explicitly waive the CONDITIONAL failure before shipping.

### 3.3 Safari simulcast constraints

Safari's WebRTC stack (WebKit libwebrtc fork) has historically lagged behind
Chrome and Firefox for simulcast. Known constraints as of 2026:

- VP9 simulcast: not guaranteed in all Safari versions; test separately.
- H.264 simulcast: should work via hardware encoder; verify on macOS and iOS.
- SVC: not applicable (tze_hud does not ship SVC in v2).
- Safari WebRTC uses Plan-B SDP in older versions; Unified Plan required for
  simulcast with RID. Safari 15+ uses Unified Plan by default.

---

## 4. Test Methodology

### 4.1 What "simulcast interop" means for tze_hud

tze_hud's Phase 4f/4b scenario is **agent-originated simulcast send**: the
tze_hud runtime (webrtc-rs) publishes multiple simulcast layers to the SFU,
and browser subscribers receive the appropriate layer. The interop test validates
that:

1. tze_hud's webrtc-rs peer connection can successfully negotiate simulcast SDP
   with the target browser's WebRTC implementation via the SFU.
2. Each simulcast layer (low/mid/high) is received by the browser at the
   negotiated resolution and frame rate.
3. The SFU can switch layers in response to bandwidth constraints (REMB/transport-
   wide CC), and the browser receives the downgraded stream without artefacting or
   stalling.
4. The RTP RID extension is correctly parsed by each browser's depayloader.
5. Glass-to-glass latency remains within the D18 budget (p50 ≤150 ms, p99 ≤400 ms)
   for at least the baseline (high-quality) simulcast layer on LAN.

### 4.2 Simulcast layer configuration

The test harness publishes three simulcast layers for each codec:

| Layer | RID | Resolution | Target bitrate | Frame rate |
|---|---|---|---|---|
| High | `h` | Full (e.g. 1280×720) | 1200 kbps | 30 fps |
| Medium | `m` | Half (640×360) | 400 kbps | 15 fps |
| Low | `l` | Quarter (320×180) | 100 kbps | 7.5 fps |

These are starting values; the actual layer configuration is an RFC 0014 decision.
The test harness must be parameterizable.

### 4.3 Test flow (per browser × codec cell)

```
1. Harness starts: launch webrtc-rs peer connection in a Tokio test task.
2. Browser automation (Playwright or WebDriver BiDi) launches the target browser
   and opens a test page that creates an RTCPeerConnection.
3. Signaling: harness ↔ browser exchange SDP offer/answer through a local
   test signaling server (WebSocket, localhost).
4. ICE: candidates exchanged via the same signaling channel.
5. Media: harness sends three simulcast layers (encoded by GStreamer or a test
   video source) via webrtc-rs.
6. Assertions (browser side, via JS injected by Playwright):
   a. RTCInboundRtpStreamStats shows three SSRCs or three RIDs decoded.
   b. Each layer's packetsLost / packetsReceived ratio < 1%.
   c. Each layer's framesDecoded > 0 after 5 seconds.
7. Layer switch assertion: harness sends a PLI + simulated bandwidth constraint
   (throttle via tc netem); observe browser stats transition from high → low layer.
8. Latency probe: harness timestamps each RTP frame (pts in RTP header); browser
   captures frame arrival via RTCVideoSink; delta reported.
9. Browser signals PASS/FAIL via postMessage to Playwright.
10. Harness records result per (browser, version, codec, layer) tuple.
```

### 4.4 Test infrastructure dependencies

| Dependency | Role | Notes |
|---|---|---|
| `webrtc` 0.20.x | tze_hud side peer connection | Must be stable before harness implements; v0.17 as fallback but simulcast incomplete |
| `gst-plugins-good` videotestsrc | Synthetic video source | No camera required for CI; smpte pattern gives known RTP timestamps |
| GStreamer → appsrc bridge | Encodes and pushes RTP to webrtc-rs | Per §4.2 of `docs/audits/webrtc-rs-audit.md` |
| `playwright` (Node.js or Python) | Browser automation | Manages browser lifecycle, injects JS, reads RTCPeerConnection stats |
| WebDriver BiDi | Alternative to Playwright | Browser-native automation protocol; may provide lower-overhead stat collection |
| Local signaling server | SDP/ICE exchange | A minimal WebSocket server; ~50 LOC; can be a Tokio task in the test binary |
| `tc netem` | Bandwidth simulation | Linux traffic control; simulate constrained uplink for layer-switch assertion |
| Docker + `browserless/chrome`, `firefox:latest` | CI browser runners | Headless browsers in containers; Safari requires macOS runner |

---

## 5. Harness Location in the Repo

### 5.1 Recommended crate

The test harness belongs in a new integration-test crate:

```
crates/
  tze_hud_webrtc_interop/
    Cargo.toml
    src/
      lib.rs          # Shared test utilities (signaling server, stats collector)
      simulcast.rs    # Simulcast layer publisher (wraps webrtc-rs)
    tests/
      simulcast_chrome.rs
      simulcast_firefox.rs
      simulcast_safari.rs
```

This follows the precedent of dedicated test crates in the workspace (e.g.
`tze_hud_protocol/tests/`). The crate is test-only (no `lib` export to
consumers); `dev-dependencies` only in the workspace `Cargo.toml`.

### 5.2 CI placement

Per D20, browser interop tests are a **label-gated** lane, not a per-PR gate.
These tests:

- Run nightly on the dedicated GPU runner (D18).
- Run label-gated on PRs touching `crates/tze_hud_webrtc_interop/`,
  `crates/tze_hud_media_worker/` (forthcoming), or `Cargo.toml` pinning for
  `webrtc`.
- Block the Phase 4b ship gate (see §6).

Safari requires a macOS CI runner (GitHub Actions `macos-latest`) and cannot
run in the Linux container fleet.

### 5.3 Feature flag

The test crate is guarded by a Cargo feature flag. Since Playwright is a Node.js/Python
library with no native Rust crate, the Cargo feature uses marker-based composition:

```toml
[features]
simulcast-interop = ["dep:webrtc"]
simulcast-interop-playwright = ["simulcast-interop"]  # no crate dep; signals CI to enable Node.js Playwright bridge
```

The `simulcast-interop-playwright` flag (with no dependency) signals the CI harness
that the Node.js Playwright bridge subprocess (`playwright-bridge.js`) should be
installed in the pre-step. This approach is documented in detail in
`docs/ci/safari-simulcast-interop-runner.md` §4.3.1 (Approach A — Playwright as
external subprocess).

For the str0m fallback path (if invoked, §7), add a parallel feature:

```toml
simulcast-interop-str0m = ["simulcast-interop"]  # str0m transport layer
simulcast-interop-str0m-playwright = ["simulcast-interop-str0m"]  # str0m + Playwright subprocess
```

Only one transport is active per CI lane (webrtc-rs or str0m); mixed-transport runs
are not permitted. The feature flag presence signals the CI system which dependencies
to configure.

---

## 6. Go / No-Go Gate for Phase 4b Bidirectional AV Ship

Phase 4b (cloud-relay, sub-epic `4b` from the signoff packet §A1) is the first
sub-epic that ships a publicly reachable WebRTC path. Simulcast is not mandatory
for 4b ship, but the interop test must demonstrate that tze_hud's WebRTC stack
does not regress browser compatibility when simulcast is disabled.

### 6.1 Hard gate (blocks 4b ship)

All of the following must be green:

| Check | Threshold |
|---|---|
| H.264 simulcast — Chrome MUST PASS cells | 100% pass, all three layers received |
| H.264 simulcast — Firefox MUST PASS cells | 100% pass, all three layers received |
| VP9 simulcast — Chrome MUST PASS cells | 100% pass, all three layers received |
| VP9 simulcast — Firefox MUST PASS cells | 100% pass, all three layers received |
| Layer switch assertion (Chrome + H.264) | High → low layer transition observed within 10 s of bandwidth constraint |
| Packet loss on any MUST PASS cell | < 1% during 30 s test window |
| webrtc-rs RID negotiation | No SDP negotiation failure on any MUST PASS cell |
| Glass-to-glass latency (high layer, LAN) | p50 ≤ 150 ms, p99 ≤ 400 ms (D18 budget) |

### 6.2 Soft gate (informational, waivable)

| Check | Threshold | Waiver authority |
|---|---|---|
| VP9 simulcast — Safari CONDITIONAL cell | Pass preferred | Tech lead (Tzeusy) named waiver |
| H.264 simulcast — Safari CONDITIONAL cell | Pass preferred | Tech lead (Tzeusy) named waiver |
| Layer switch assertion (Firefox + VP9) | Pass preferred | Tech lead waiver |

### 6.3 str0m fallback branch in the gate

If the Phase 4b kickoff spike (per §8 sequencing) triggers the str0m fallback path
(i.e., webrtc-rs v0.20 simulcast is declared NO-GO per hud-g89zs verdict — see §7):

1. All hard-gate cells in §6.1 remain in force but are evaluated against the
   **str0m transport layer** instead of webrtc-rs. No gate threshold changes.
2. An additional str0m-specific gate is added: **TURN client integration verified**
   (per hud-kjody CONDITIONAL-GO, PR #547) — the external TURN client must exercise
   at least one successful relay candidate acquisition over TCP before 4b ships.
3. The closeout report must record which transport library (webrtc-rs or str0m) was
   under test; mixed runs (e.g., webrtc-rs for some cells, str0m for others) are
   not permitted — pick one and run the full matrix against it.

### 6.4 Waiver process

A soft-gate failure may be waived by the Phase 4 tech lead with:
1. A recorded bead note on `hud-fpq51` (or its successor) explaining the waiver.
2. A follow-up bead created for the waived failure with P1 priority, blocking
   the next phase that makes the gap observable to operators.

### 6.5 Phase 4b exit criteria relationship

The Phase 4b closeout report (per F33 convention, filed under `docs/reports/`)
must include a simulcast interop matrix section recording the final state of each
cell in §3.2. Cells that were CONDITIONAL and waived must be listed with the
waiver bead ID. If str0m was invoked, the closeout report must additionally record
the TURN client integration verdict.

---

## 7. str0m Fallback Path (Contingency)

This section extends the original plan to cover the str0m transport layer, which
is invoked if webrtc-rs v0.20 simulcast is declared NO-GO at Phase 4b kickoff.

**Cross-references:**
- `hud-1ee3a` — SFU fallback audit (PR #544): str0m recommended as the fallback; verdict RECOMMENDED FALLBACK
- `hud-g89zs` — webrtc-rs v0.20 simulcast spike (PR #543): NO-GO verdict; trigger signal chain for this section
- `hud-kjody` — str0m TURN-over-TCP validation (PR #547): CONDITIONAL-GO verdict; defines TURN integration requirement
- `hud-amf17` — RFC 0018 WHIP signaling adapter spec (in flight): defines signaling bridge required for cloud-relay

### 7.1 When the fallback is invoked

The fallback from webrtc-rs to str0m is triggered by the Phase 4b kickoff decision
process. Per the hud-g89zs NO-GO verdict and the hud-1ee3a fallback audit, the
decision tree is:

```
Phase 4b kickoff spike (one-week survey):
    │
    ├─ Is webrtc-rs v0.20 stable (non-alpha) available?
    │       YES ──┐
    │             │
    ├─ Is rtc PR #72 (rrid RTX) merged and promoted to the async wrapper?
    │       YES ──┤
    │             │
    ├─ Is GCC / REMB bandwidth estimation interceptor available (PR #85)?
    │       YES ──┴─→ UPGRADE to CONDITIONAL-GO: use webrtc-rs v0.20 (primary path)
    │
    └─ Any answer is NO after the survey:
            → Invoke str0m 0.18 (this section applies)
```

hud-g89zs returned NO-GO as of April 2026 (alpha.1 only; no rrid RTX; no GCC
interceptor). The Phase 4b kickoff spike must re-evaluate. If it returns the same
gaps, the str0m path is taken and the remainder of this section governs.

### 7.2 str0m simulcast implementation differences vs. webrtc-rs

The primary implementation difference for the harness is the **I/O model**. str0m
is a sans-IO library: it exposes a state machine and emits output events rather than
managing its own sockets or async tasks. The harness must own the event loop.

#### 7.2.1 Sans-IO signaling plane implications

Under the webrtc-rs design (§4.3), the signaling server exchanges SDP offer/answer
over a WebSocket signaling channel; webrtc-rs internally manages ICE gathering and
DTLS. With str0m, the harness explicitly drives:

1. SDP production — via `rtc.sdp()` / `rtc.set_remote_sdp()` after negotiation.
2. ICE candidate gathering — by enumerating local network interfaces and calling
   `rtc.add_local_candidate()`. No automatic NIC enumeration; the harness provides
   the candidate list.
3. Socket I/O — by polling `rtc.poll_output()` for `Output::Transmit` events and
   feeding received packets via `rtc.handle_input(Input::Receive(...))`.
4. Timer ticks — by calling `rtc.handle_input(Input::Timeout(Instant::now()))` on
   the interval returned by `Output::Timeout`.

The local WebSocket signaling server in the harness (see §4.4) is unchanged; only
the peer-connection driver behind it changes.

#### 7.2.2 RID/MID handling in str0m 0.18

str0m 0.18 implements RID (RFC 8852) natively in its simulcast send path:

| Feature | str0m 0.18 status | webrtc-rs v0.20 alpha status |
|---|---|---|
| RID generation (send side) | Present — simulcast layer API identifies layers by RID | Implemented in rtc v0.5.0+ |
| RID parsing (receive side) | Present — demuxes incoming streams by RID | Implemented; rrid RTX: open PR #72 |
| MID / BUNDLE | Present — handled at SDP layer | Present; interceptor-level MID field absent |
| Simulcast SDP (RFC 8853) | Present — str0m emits RFC 8853 `a=simulcast` and `a=rid` | Substantially implemented in rtc master |
| rrid (Repaired RTP Stream ID) | Present — RTX demux by rid is part of the simulcast send path | Open PR #72, not yet merged |
| Layer selection API | `SendStream` per RID, caller controls which layer to populate | Not implemented; PLI-only workaround |

For the harness, str0m's simulcast send API looks like:

```rust
// str0m simulcast send sketch (non-normative)
let mid = rtc.add_media(MediaKind::Video, Direction::SendOnly)?;
// Add simulcast layers by RID
let stream_h = rtc.stream_tx(&mid, "h")?;
let stream_m = rtc.stream_tx(&mid, "m")?;
let stream_l = rtc.stream_tx(&mid, "l")?;

// Push RTP payloads per layer (from GStreamer appsink)
stream_h.write_rtp(pt, seq, ts, payload)?;
stream_m.write_rtp(pt, seq, ts, payload)?;
stream_l.write_rtp(pt, seq, ts, payload)?;
```

The harness `simulcast.rs` module (§5.1) must provide a transport-layer abstraction
so that the webrtc-rs and str0m paths share the same test assertion harness.

#### 7.2.3 Layer selection knobs

str0m does not implement RTCP-driven layer switching internally — the caller controls
which `SendStream` receives payloads. For the layer-switch assertion in §4.3 step 7,
the harness must:

1. Observe incoming RTCP REMB or transport-wide CC from the SFU via
   `Output::Event(Event::Rtcp(...))`.
2. Map the bandwidth estimate to the E25 degradation ladder (framerate first, then
   resolution).
3. Stop populating the high-layer `SendStream` and continue populating the
   low-layer stream.

This is architecturally equivalent to what tze_hud's media worker would do in
production. The harness implements a minimal version sufficient to trigger the
browser's layer-switch observation.

### 7.3 Signaling bridge: WHIP vs. current webrtc-rs bridge

The current plan assumes a local WebSocket signaling server for direct SDP/ICE
exchange (§4.4). For the cloud-relay path (C15), the signaling adapter changes:

| Mode | Signaling path | Notes |
|---|---|---|
| Direct / local | WebSocket signaling server (§4.4) | No change for str0m; sans-IO peer-connection is transport-agnostic |
| Cloud-relay (C15, LiveKit) | gRPC session plane → WHIP adapter → LiveKit Server | RFC 0018 (hud-amf17) specifies this adapter |
| Cloud-relay (C15, Cloudflare Calls) | gRPC session plane → WHIP PUT → Cloudflare Calls REST | Same adapter shape; vendor endpoint differs |

The WHIP protocol (WebRTC-HTTP Ingestion Protocol, RFC draft) is a signaling
convention, not a transport. Both str0m and webrtc-rs produce JSEP SDP that WHIP
carries as an HTTP body. From str0m's perspective:

1. tze_hud's WHIP adapter (to be specced in RFC 0018 / hud-amf17) sends
   `PUT /whip` with the SDP offer generated by `rtc.sdp()`.
2. The SFU returns an SDP answer.
3. The answer is fed back to str0m via `rtc.set_remote_sdp(answer)`.
4. ICE candidates from the SFU trickle in via the WHIP resource `PATCH` (trickle
   ICE over WHIP, RFC draft §4.4.2) and are added to str0m via `rtc.add_remote_candidate()`.

**Dependency on hud-amf17**: The WHIP adapter is not a harness concern for the
direct/local test path. For the cloud-relay path, the harness can skip WHIP and
connect directly to the SFU via its test API (LiveKit server has a `create_room` +
`create_token` API that bypasses WHIP for test participants). WHIP correctness is
validated separately by hud-amf17.

### 7.4 External TURN client integration requirement

Per the hud-kjody CONDITIONAL-GO verdict (PR #547), str0m requires tze_hud to supply
an external TURN client for any relay candidate path. This is a **phase 4b pre-condition**,
not a harness implementation detail.

Key findings from hud-kjody:

- str0m explicitly excludes TURN client logic as an out-of-scope concern (by design).
- str0m's `Candidate::relayed(addr, local_interface, proto)` API accepts
  externally-obtained relay addresses; the harness adds them via `rtc.add_local_candidate()`.
- TURN-over-TCP (RFC 6062) is absent from **both** webrtc-rs and str0m; the gap is
  not str0m-specific.
- str0m has an ICE-TCP advantage: full tcptype candidate support since v0.15.0 (PR #797),
  while webrtc-rs v0.17 lacks this entirely.
- Recommended TURN integration: a thin TURN-over-TCP client (est. 2–3 days) using
  `webrtc-stun` for STUN serialization, or relying on LiveKit Cloud's managed TURN
  infrastructure if it is the C15 vendor (which removes the TCP TURN requirement
  for the cloud-relay path).

**Harness impact**: The local-path simulcast harness tests (§4.3) do not require TURN
— they use host ICE candidates over loopback. TURN integration is only exercised by
the cloud-relay path. The TURN client bead must be scheduled at Phase 4b kickoff and
must complete before the cloud-relay harness cells are run.

### 7.5 AV1 support in str0m

The hud-1ee3a SFU fallback audit (PR #544, §1.5) contains a correction to an earlier
claim:

> **AV1 RTP packetization is present in str0m since v0.15.0** (PR #819, Jan 2026),
> refined through v0.16.1 and v0.17.0 bug-fix passes. The earlier claim that AV1 was
> absent was incorrect.

This plan previously deferred AV1 entirely (§3.2 codec matrix: "AV1 is deferred per
D18"). That deferral stands — D18 specifies H.264 + VP9 for v2, and AV1 is not in
scope for Phase 4b. The str0m AV1 status is a non-issue for this plan.

**If D18 is revised to include AV1 before Phase 4b closes**: str0m v0.18 supports
AV1 RTP packetization/depacketization natively; no additional gap exists relative to
webrtc-rs. The codec matrix in §3.2 would need to add an AV1 row.

### 7.6 Harness crate delta: swapping str0m into `tze_hud_webrtc_interop`

The harness crate sketch in §5.1 assumes webrtc-rs as the transport. To support the
str0m fallback path, the following structural changes are required:

#### 7.6.1 New source files

```
crates/
  tze_hud_webrtc_interop/
    src/
      lib.rs           # Unchanged: shared utilities (signaling server, stats collector)
      simulcast.rs     # NOW: transport-agnostic trait + webrtc-rs impl
      simulcast_str0m.rs  # NEW: str0m-backed simulcast layer publisher
      turn_client.rs      # NEW: external TURN client integration shim (Phase 4b pre-condition)
    tests/
      simulcast_chrome.rs    # Unchanged: browser assertion logic is transport-agnostic
      simulcast_firefox.rs   # Unchanged
      simulcast_safari.rs    # Unchanged
```

#### 7.6.2 Transport abstraction trait

```rust
// Proposed abstraction in src/simulcast.rs (non-normative)
pub trait SimulcastPublisher: Send + 'static {
    /// Negotiate SDP and begin ICE for the given test session.
    async fn negotiate(&mut self, signaling: &mut SignalingChannel) -> anyhow::Result<()>;

    /// Push one RTP payload for the given simulcast layer RID.
    async fn push_rtp(&mut self, rid: &str, payload: Bytes, timestamp: u32) -> anyhow::Result<()>;

    /// Observe the next RTCP event (for layer-switch assertion, §4.3 step 7).
    async fn next_rtcp_event(&mut self) -> anyhow::Result<RtcpEvent>;
}
```

The webrtc-rs implementation (`simulcast.rs`) and the str0m implementation
(`simulcast_str0m.rs`) both implement this trait. Test files (`simulcast_chrome.rs`,
etc.) are written against the trait and are transport-agnostic.

#### 7.6.3 Feature flags

Extend the existing feature flag scheme from §5.3. The webrtc-rs feature flags are
defined in §5.3; for the str0m fallback path, add:

```toml
[features]
# str0m transport layer (mutually exclusive with webrtc-rs flag in any given CI lane)
simulcast-interop-str0m = ["dep:str0m"]

# str0m + Playwright subprocess for browser automation
simulcast-interop-str0m-playwright = ["simulcast-interop-str0m"]
```

The CI matrix (§5.2) adds a second lane when the str0m fallback is invoked:
run with `--features simulcast-interop-str0m-playwright` in place of
`--features simulcast-interop-playwright`. Only one transport is tested per lane;
mixed runs are not permitted (per §6.3). The Playwright subprocess bridge remains
unchanged between transport layers.

#### 7.6.4 GStreamer bridge delta

str0m produces `MediaData` events containing raw RTP payload bytes. The GStreamer bridge
in §4.4 (using `appsrc` with `application/x-rtp` caps) remains structurally identical.
The delta is:

- webrtc-rs path: `track_local.write_sample()` → internal RTP framing → SRTP → socket
- str0m path: `stream_tx.write_rtp(pt, seq, ts, payload)` → SRTP (internal) → `Output::Transmit` → harness socket

The GStreamer pipeline (videotestsrc → encoder → appsink) is unchanged. Only the
appsink callback changes: instead of calling webrtc-rs's track write API, it calls
`stream_tx.write_rtp()` on the appropriate `SendStream`.

---

## 8. Cross-Links

### 8.1 RFC 0014 — Media Plane Wire Protocol

RFC 0014 (not yet authored as of this document; see signoff packet F29) owns the
authoritative WebRTC signaling and simulcast negotiation wire shape. When RFC 0014
is drafted, it must include:

- The simulcast SDP offer structure (JSEP simulcast extensions, RFC 8853 rid= lines).
- The RID-to-layer mapping semantics.
- How the tze_hud runtime signals layer preference to the SFU (subscribe API,
  RTCP feedback, or SFU-proprietary SDK call).
- State machine transitions for simulcast layer degradation (must reference the
  E25 degradation ladder: framerate → resolution, in that order).
- If str0m is the fallback transport: the transport-abstraction trait (§7.6.2) must
  be aligned with the RFC 0014 wire shape.

This test plan will need to be revisited when RFC 0014 lands to align the
assertion set with the wire-level contract.

### 8.2 Signoff packet decisions

| Decision | Relevance |
|---|---|
| D18 | Codec matrix (H.264 + VP9); glass-to-glass latency budget (p50 ≤150 ms / p99 ≤400 ms) |
| D19 | Real device coverage: 1× Mac, 1× Windows, 1× Linux primary; cloud device farm for breadth |
| D20 | CI cadence: label-gated on PRs, nightly on GPU runner |
| D21 | Release gate tiers: latency regression >20% is Major (blocks unless waived) |
| E25 | Degradation ladder order: framerate first, then resolution — simulcast layer selection must respect this |
| C15 | SFU vendor (LiveKit Cloud or Cloudflare Calls) selected at phase 4b kickoff; harness must be SFU-vendor-agnostic or have a vendor adapter seam |
| G31 | Phase 4 is 6–9 months from v1 ship; harness implementation begins at phase 4 kickoff bead |

### 8.3 webrtc-rs audit

See `docs/audits/webrtc-rs-audit.md` §2.2 (Simulcast and SVC) and §10 (Open
Issues) for the full audit finding that motivated this plan.

### 8.4 str0m fallback and TURN validation

- `docs/audits/webrtc-sfu-fallback-audit.md` (hud-1ee3a, PR #544) — str0m verdict,
  AV1 correction, WHIP/WHEP compatibility, migration effort estimate.
- `docs/reports/str0m-turn-over-tcp-validation.md` (hud-kjody, PR #547) — TURN
  CONDITIONAL-GO verdict, external TURN client integration pattern, RFC 6062 gap
  analysis (applies equally to webrtc-rs — neither stack has native TURN-over-TCP).
- `docs/reports/webrtc-rs-v0.20-simulcast-readiness.md` (hud-g89zs, PR #543) — NO-GO
  verdict; signal chain that triggers this section.

### 8.5 RFC 0018 WHIP signaling adapter

`hud-amf17` (in flight) specifies the WHIP adapter that bridges tze_hud's gRPC session
plane to the SFU's WHIP ingest endpoint. This adapter is required for the cloud-relay
path regardless of whether str0m or webrtc-rs is the transport layer. The adapter spec
must be in a merged RFC before Phase 4b harness implementation begins (per F29 gate).

---

## 9. Implementation Sequencing

This document is a **design deliverable only**. The test harness is not
implemented until Phase 4 kickoff.

| Milestone | Action | Trigger |
|---|---|---|
| Phase 4 kickoff bead | Re-verify webrtc-rs simulcast matrix (§2) against then-current v0.20 release | Phase 3 closeout report merges |
| Phase 4 kickoff bead | If NO-GO: invoke str0m path — read §7 and schedule TURN client bead | webrtc-rs v0.20 survey returns incomplete |
| RFC 0014 draft | Align §8.1 cross-link items; update harness sketch if wire shape changes | RFC 0014 PR opens |
| RFC 0018 (hud-amf17) draft | Align §7.3 WHIP adapter; required before cloud-relay harness cells | hud-amf17 PR opens |
| Phase 4 implementation bead | Implement `crates/tze_hud_webrtc_interop/`; fill each matrix cell | Phase 4 kickoff done |
| Phase 4b pre-ship | Run full interop matrix; record results in closeout report | All phase 4b implementation beads closed |
| Phase 4b closeout | Apply go/no-go gate (§6 including §6.3 str0m branch); file waivers if needed | Interop run complete |

---

## 10. Discovered Gaps and Follow-Ups

The following items surfaced during the design of this plan and are out of scope
for this document but should be tracked as phase 4 beads:

1. **SFU vendor adapter seam in the test harness** — The harness must be able to
   work with both LiveKit and Cloudflare Calls until C15 vendor selection is made.
   This requires an abstract signaling adapter. File as a sub-bead of the phase 4
   harness implementation bead.

2. **v0.20 simulcast RID/MID interceptor readiness** — The phase 4 kickoff bead
   must include a one-day spike to verify that webrtc-rs v0.20's simulcast
   implementation is sufficiently complete for the harness. If not, the bead
   must escalate with a decision on whether to patch webrtc-rs, contribute
   upstream, or fall back to v0.17 with a limited simulcast scope.

3. **str0m as fallback if webrtc-rs v0.20 simulcast is incomplete** — The
   `hud-ora8.1.17` audit identified `str0m` as a credible fallback. The decision
   tree (§7.1) now governs invocation. When invoked, §7 applies in full.

4. **Safari macOS runner in CI** — Safari interop requires a `macos-latest` GitHub
   Actions runner. CI configuration does not currently have this lane. Add as a
   phase 4 chore.

5. **IPv6 ICE validation** — `hud-ora8.1.17` audit issue #774 (IPv6 ICE gather
   in v0.20 alpha). IPv6 browser paths must be validated before phase 4b cloud-
   relay ships, particularly for Safari on macOS (which defaults to IPv6-preferred
   ICE candidate ordering). File as a phase 4 pre-condition bead parallel to this
   interop plan.

6. **TURN client integration bead** — Per hud-kjody CONDITIONAL-GO, tze_hud must
   schedule an explicit bead for the external TURN client integration before Phase 4b
   cloud-relay harness cells run. Estimated scope: 3–5 days (2–3 days for UDP TURN;
   1–2 additional days for TCP/TLS wrapping). If LiveKit Cloud is the C15 vendor,
   verify at kickoff whether LiveKit's managed TURN infrastructure covers TCP/TLS
   relay — if yes, the bead scope reduces to UDP-only validation.

---

*End of document. Re-visit at phase 4 kickoff.*
