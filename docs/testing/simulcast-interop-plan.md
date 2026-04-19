# Phase 4 Simulcast Interop Test Plan

**Issue:** `hud-fpq51`
**Parent audit:** `hud-ora8.1.17` (webrtc-rs library audit, PR #523)
**Date:** 2026-04-19
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

The test crate is guarded by a Cargo feature flag:

```toml
[features]
simulcast-interop = ["dep:playwright", "dep:webrtc"]
```

This prevents the browser automation dependency from entering the main build
graph before Phase 4.

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

### 6.3 Waiver process

A soft-gate failure may be waived by the Phase 4 tech lead with:
1. A recorded bead note on `hud-fpq51` (or its successor) explaining the waiver.
2. A follow-up bead created for the waived failure with P1 priority, blocking
   the next phase that makes the gap observable to operators.

### 6.4 Phase 4b exit criteria relationship

The Phase 4b closeout report (per F33 convention, filed under `docs/reports/`)
must include a simulcast interop matrix section recording the final state of each
cell in §3.2. Cells that were CONDITIONAL and waived must be listed with the
waiver bead ID.

---

## 7. Cross-Links

### 7.1 RFC 0014 — Media Plane Wire Protocol

RFC 0014 (not yet authored as of this document; see signoff packet F29) owns the
authoritative WebRTC signaling and simulcast negotiation wire shape. When RFC 0014
is drafted, it must include:

- The simulcast SDP offer structure (JSEP simulcast extensions, RFC 8853 rid= lines).
- The RID-to-layer mapping semantics.
- How the tze_hud runtime signals layer preference to the SFU (subscribe API,
  RTCP feedback, or SFU-proprietary SDK call).
- State machine transitions for simulcast layer degradation (must reference the
  E25 degradation ladder: framerate → resolution, in that order).

This test plan will need to be revisited when RFC 0014 lands to align the
assertion set with the wire-level contract.

### 7.2 Signoff packet decisions

| Decision | Relevance |
|---|---|
| D18 | Codec matrix (H.264 + VP9); glass-to-glass latency budget (p50 ≤150 ms / p99 ≤400 ms) |
| D19 | Real device coverage: 1× Mac, 1× Windows, 1× Linux primary; cloud device farm for breadth |
| D20 | CI cadence: label-gated on PRs, nightly on GPU runner |
| D21 | Release gate tiers: latency regression >20% is Major (blocks unless waived) |
| E25 | Degradation ladder order: framerate first, then resolution — simulcast layer selection must respect this |
| C15 | SFU vendor (LiveKit Cloud or Cloudflare Calls) selected at phase 4b kickoff; harness must be SFU-vendor-agnostic or have a vendor adapter seam |
| G31 | Phase 4 is 6–9 months from v1 ship; harness implementation begins at phase 4 kickoff bead |

### 7.3 webrtc-rs audit

See `docs/audits/webrtc-rs-audit.md` §2.2 (Simulcast and SVC) and §10 (Open
Issues) for the full audit finding that motivated this plan.

---

## 8. Implementation Sequencing

This document is a **design deliverable only**. The test harness is not
implemented until Phase 4 kickoff.

| Milestone | Action | Trigger |
|---|---|---|
| Phase 4 kickoff bead | Re-verify webrtc-rs simulcast matrix (§2) against then-current v0.20 release | Phase 3 closeout report merges |
| RFC 0014 draft | Align §7.1 cross-link items; update harness sketch if wire shape changes | RFC 0014 PR opens |
| Phase 4 implementation bead | Implement `crates/tze_hud_webrtc_interop/`; fill each matrix cell | Phase 4 kickoff done |
| Phase 4b pre-ship | Run full interop matrix; record results in closeout report | All phase 4b implementation beads closed |
| Phase 4b closeout | Apply go/no-go gate (§6); file waivers if needed | Interop run complete |

---

## 9. Discovered Gaps and Follow-Ups

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
   `hud-ora8.1.17` audit identified `str0m` as a credible fallback. If the phase
   4 kickoff spike finds v0.20 simulcast incomplete, a str0m evaluation bead
   should be created.

4. **Safari macOS runner in CI** — Safari interop requires a `macos-latest` GitHub
   Actions runner. CI configuration does not currently have this lane. Add as a
   phase 4 chore.

5. **IPv6 ICE validation** — `hud-ora8.1.17` audit issue #774 (IPv6 ICE gather
   in v0.20 alpha). IPv6 browser paths must be validated before phase 4b cloud-
   relay ships, particularly for Safari on macOS (which defaults to IPv6-preferred
   ICE candidate ordering). File as a phase 4 pre-condition bead parallel to this
   interop plan.

---

*End of document. Re-visit at phase 4 kickoff.*
