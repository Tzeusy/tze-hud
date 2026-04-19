# webrtc-rs v0.20 Simulcast Readiness Audit

**Issue:** `hud-g89zs`
**Parent plan:** `hud-fpq51` (Phase 4 simulcast interop test harness)
**Unblocks:** `hud-fpq51` Phase 4b implementation bead (once harness is authored)
**Date:** 2026-04-19
**Auditor:** agent worker (claude-sonnet-4-6)
**Prior art:** `hud-ora8.1.17` webrtc-rs audit (PR #523); `hud-fpq51` interop plan (PR #538)

---

## Verdict

**NO-GO for webrtc-rs v0.20 as the Phase 4b harness implementation base today.**

v0.20.0-alpha.1 (released 2026-03-01) is the only pre-release of the v0.20 line to date.
The simulcast-critical RID/MID interceptor work is shipped in the upstream `rtc` sans-IO
crate (v0.5.0+) but the async `webrtc` wrapper layer — which tze_hud would actually depend
on — has not promoted these fixes from `rtc` into a second alpha or a stable release.
Nineteen open PRs targeting the `rtc` crate are awaiting review as of 2026-04-19. A stable
v0.20.0 has no announced ship date.

**Recommended path:** keep Phase 4b harness implementation gated. Monitor the `webrtc-rs/rtc`
PR backlog (issue #87 tracks the current batch) and the `webrtc-rs/webrtc` release cadence.
Re-evaluate at Phase 4 kickoff. If v0.20 stable has not shipped by then, the harness should
be authored against webrtc-rs `0.17.x` with partial simulcast, or the project should pivot to
`str0m v0.18.0` (see §5 for the str0m evaluation).

---

## 1. Release Status: webrtc-rs v0.20 as of April 2026

### 1.1 Crates and version lines

| Crate | Latest stable | Latest alpha | Notes |
|---|---|---|---|
| `webrtc` (async wrapper) | **v0.17.1** (2026-02-06) | **v0.20.0-alpha.1** (2026-03-01) | Only one alpha released since March; no alpha.2 |
| `rtc` (sans-IO core) | **v0.9.0** (2026-02-08) | — (stable cadence) | 19 unmerged PRs queued as of 2026-04-19 |

The `webrtc` v0.20.0-alpha.1 wraps the `rtc` crate and exposes an async API.
The `rtc` crate itself has had numerous releases (v0.3.0 through v0.9.0 in January–February 2026)
during the feature-build phase. However, after v0.9.0 and the v0.20.0-alpha.1 wrapper, the
public cadence of tagged releases has paused while a large queue of fixes and features accumulates
in open PRs.

**Bottom line**: webrtc-rs v0.20 is alpha-only. No stable release has been announced. The
maintainers have signalled that substantial additional work remains before v0.20.0 final.

### 1.2 What changed in v0.20.0-alpha.1 relative to the hud-ora8.1.17 audit baseline

The original audit (`hud-ora8.1.17`, 2026-04-18) assessed `webrtc` at v0.17.1 and characterized
v0.20 as "v0.20.0-alpha.1 — a ground-up rewrite on a sans-IO core — pin 0.17 for phase 1,
migrate at phase 4 kickoff." The alpha shipped with:

- Async API rebuilt on top of `rtc` (no more Tokio-coupled callback model)
- 20 working examples including `simulcast` and `simulcast_bidirection`
- Trait-based event handlers replacing callback closures
- Tokio and smol runtime support
- Known open issues: IPv6 ICE gather (#774), socket recv error handling (#777), localhost STUN
  timeout (#778), H.265 codec bugs (#779)

The simulcast examples in the alpha run. They are not accompanied by browser interop test evidence.

---

## 2. Simulcast Support Matrix: Current Status

### 2.1 RID header extension (RFC 8852)

| Component | Status | Evidence |
|---|---|---|
| `RtpStreamId` type alias (RFC 8852) | **Implemented** — defined in `rtc` v0.5.0 | rtc v0.5.0 release blog; `rtp_transceiver/rtp_sender/rtp_coding_parameters.rs` |
| RID registered in `configure_simulcast_extension_headers()` | **Implemented** | `rtc` source: `urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id` registered |
| RID extraction from incoming packets | **Implemented** | simulcast.rs example uses `rtp_receiver.track().rid(ssrc)` |
| `rrid` (Repaired RTP Stream ID, RFC 8852 §3) | **Open PR — not yet merged** | rtc PR #72 (open as of 2026-04-19): "fix(rtp): associate repair SSRC with base stream RTX parameters (closes #12)" |

The `rrid` extension handler was a stub (`TODO` in `endpoint.rs:450`) through v0.9.0. PR #72
implements the full rrid→RTX SSRC association and fixes simulcast RTX routing. It is open as
of the audit date, passing all tests on its branch, but not yet merged to `rtc` master.

### 2.2 MID header extension (RFC 9143 / BUNDLE)

| Component | Status | Evidence |
|---|---|---|
| MID parsed from SDP | **Implemented** | `peer_connection/sdp/mod.rs` uses `get_mid_value()` throughout |
| `urn:ietf:params:rtp-hdrext:sdes:mid` extension registered | **Implemented** | simulcast.rs registers via `register_header_extension` |
| Duplicate MID filtering | **Merged** in rtc | PR #55 "Remove duplicate mid" merged 2026-03-01 |
| MID in interceptor `StreamInfo` | **Not present** | `rtc-interceptor/src/stream_info.rs`: struct has SSRC, payload type, extensions but no MID field |

MID is handled at the SDP and peer-connection layer. Interceptors receive SSRC-keyed
`StreamInfo` structs with no MID field, meaning interceptors cannot filter or route by MID.
For the tze_hud use case (outbound simulcast from a webrtc-rs sender to a browser via SFU),
this is a transport-level concern and not a blocking gap for the basic harness.

### 2.3 Simulcast SDP negotiation (RFC 8853)

| Feature | Status | Evidence |
|---|---|---|
| `a=simulcast` attribute generation | **Implemented** | `peer_connection/sdp/mod.rs`: `add_sender_sdp()` detects multiple encodings with non-empty RIDs and emits RFC 8853 simulcast attributes |
| `a=rid` attribute parsing | **Implemented** | `get_rids()` in SDP module; direction flipping in answers |
| SSRC omission for simulcast m-sections | **Implemented** | `write_ssrc_attributes_for_simulcast` flag |
| Bidirectional simulcast over a single m-line | **Fixed** | rtc issue #20 closed: "Fix simulcast issue where both peers send simulcast streams over a single m= line" |
| Simulcast SDP answer when peer is offerer | **Fixed** | rtc issue #14 closed: "fix simulcast integration tests for RTC as Offerer" |

RFC 8853 SDP negotiation appears substantially complete in `rtc` master. This is the strongest
area of the simulcast implementation.

### 2.4 Layer selection / RID-based routing

| Feature | Status | Evidence |
|---|---|---|
| RID-keyed sender lookup | **Implemented** | simulcast.rs: SSRC→RID map, forward packets to matching output senders |
| Dynamic layer switching (RTCP-driven) | **Not implemented** | Noted in simulcast.rs: PLI is sent every 3 s as a temporary workaround; "This is a temporary fix until we implement incoming RTCP events" |
| SFU-directed layer selection | **Not implemented** | tze_hud would need to implement this in the media worker layer |

Layer switching is the most significant remaining gap for production simulcast. The current
implementation forwards all layers at all times; the SFU sends REMB/transport-wide CC to
signal bandwidth constraints, but the webrtc-rs side does not respond to incoming RTCP to
drop layers. This is a known TODO in the example code.

### 2.5 RTCP interceptors for simulcast feedback

The v0.6.0 release of `rtc` was announced as "Interceptor Framework Complete" with NACK, RR/SR,
and TWCC. Two advanced interceptors remain open as of 2026-04-19:

| Interceptor | Status | PR |
|---|---|---|
| JitterBuffer (receiver-side) | **Open PR #84** in `rtc` | feat(interceptor): add JitterBuffer receiver-side interceptor |
| GCC sender-side bandwidth estimator | **Open PR #85** in `rtc` | feat(interceptor): add GCC sender-side bandwidth estimator |

GCC (Google Congestion Control) is the algorithm used by Chrome to compute bandwidth estimates
that drive simulcast layer selection. Without a GCC interceptor, webrtc-rs cannot generate
accurate REMB/transport-wide CC feedback that SFUs rely on for layer decisions. This is not
a blocker for a basic harness that tests negotiation and reception, but it is a gap for
production adaptive simulcast.

### 2.6 Production browser interoperability evidence

No documented browser interop test results exist for webrtc-rs simulcast (v0.17.x or v0.20
alpha). The maintainer roadmap ("RTC Feature Complete: What's Next", 2026-01-18) identifies
browser interop testing (Chrome, Firefox, Safari, Edge via Selenium/Playwright) as a major
next goal, not a current achievement.

---

## 3. Comparison with hud-ora8.1.17 Audit Findings

| Caveat (hud-ora8.1.17) | Status at audit (2026-04-18) | Status today (2026-04-19) | Delta |
|---|---|---|---|
| v0.17.x feature-frozen; v0.20 alpha | v0.20.0-alpha.1 released March 2026 | Confirmed — no second alpha or stable | No change |
| Simulcast partial / unvalidated | "in-progress" in rtc RID/MID framework | RID: substantially implemented; rrid: open PR #72; layer switching: TODO; no browser interop evidence | Partial progress |
| SVC not implemented | Not targeted for v2 | Confirmed not implemented or planned | No change |
| GStreamer RTP bridge requires careful handling | Implementation guidance provided | No change to guidance | No change |

The delta between the original audit and this spike is:
- Positive: RFC 8853 SDP negotiation is substantially more complete in `rtc`; RID extraction
  from incoming packets is implemented; the bidirectional simulcast m-line bug is fixed.
- Gaps remain: `rrid` RTX association is an open PR; layer switching via incoming RTCP is
  unimplemented; GCC interceptor is an open PR; no browser interop test evidence.

---

## 4. Gap Analysis: Phase 4b Harness Readiness

The `hud-fpq51` test plan (PR #538) defines the following must-pass gates for Phase 4b:

| Phase 4b gate requirement | webrtc-rs v0.20 status | Blocking? |
|---|---|---|
| H.264 + VP9 simulcast SDP negotiation with Chrome/Firefox | SDP layer is implemented; no browser interop tested | Gap (test evidence missing) |
| Three simulcast layers (h/m/l) negotiated via RID | RID generation is implemented | Likely OK |
| `rrid` RTX demux | Open PR #72 — not in any published release | Gap (missing in alpha.1) |
| Layer switch assertion (PLI/REMB response) | Not implemented; PLI is periodic only | Gap |
| GCC bandwidth estimation for adaptive layer selection | Open PR #85 | Gap |
| webrtc-rs v0.20 stable release | No stable release; alpha.1 only | Structural blocker |

A Phase 4b harness written against v0.20.0-alpha.1 today would:
1. Be pinned to an alpha crate with breaking API changes expected before stable.
2. Lack `rrid` RTX support (open PR, not merged).
3. Lack RTCP-driven layer switching.
4. Have no browser interop evidence to verify its assertions.

Verdict: **implementation of the Phase 4b harness should remain gated.** The gate condition
from `hud-fpq51` stands: "re-verify webrtc-rs simulcast matrix at phase 4 kickoff."

---

## 5. str0m Fallback Evaluation

Per the issue scope, `hud-1ee3a` identifies `str0m` as the leading pure-Rust fallback.

### 5.1 Current str0m status

| Metric | Value |
|---|---|
| Latest version | `0.18.0` (tagged 2026; latest commit 2026-04-18) |
| Simulcast | Supported (checkmark in feature matrix); send-side simulcast API added v0.7.0, revised v0.14.0 |
| RID | Implemented (simulcast layer identification) |
| AV1 | Not natively supported (v2 deferred anyway per D18) |
| Architecture | Sans-IO; requires integration shim for Tokio |
| Production use | Server-side SFU at Lookback; BitWHIP client |
| Client-side features absent | Audio/video capture, encode, decode, adaptive jitter buffer |

### 5.2 str0m fit for tze_hud Phase 4

str0m is designed for server-side SFU deployments. Its missing client-side decode/capture
is not a blocker for tze_hud (GStreamer handles decode), but its sans-IO architecture requires
a Tokio integration shim. The existing webrtc-rs GStreamer bridge pattern (`track_remote.read()`
→ `marshal_to()` → appsrc) would need to be rewritten around str0m's poll-based input/output
model.

str0m has a confirmed simulcast send path with RID. However:
- No browser interop test evidence for str0m simulcast send against Chrome/Firefox/Safari exists
  in public documentation, for the same reason it does not exist for webrtc-rs.
- str0m explicitly does not target client-side AV rendering (no jitter buffer, no decode), which
  means tze_hud's Phase 4 latency testing would still be entirely GStreamer-dependent.
- The Phase 4b signoff packet (C15) selects a SaaS SFU; the SFU SDK client would still need to
  be wrapped around a WebRTC stack. str0m is the SFU-internal stack at Lookback, not the client
  SDK facing a cloud SFU.

**Verdict on str0m**: str0m is a credible fallback if webrtc-rs v0.20 fails to stabilize by
Phase 4 kickoff, but it carries its own integration cost and does not resolve the browser interop
evidence gap. Adopting str0m does not avoid the Phase 4 interop testing; it changes which Rust
crate is under test. The `hud-1ee3a` full audit (LiveKit SDK + Cloudflare Calls SDK + str0m
comparison) should be conducted at Phase 4 kickoff, not now.

---

## 6. Open Issues and Follow-ups

### Tracked upstream (webrtc-rs/rtc)

| Issue / PR | Status | Relevance to tze_hud |
|---|---|---|
| PR #72: rrid RTX association | **Open** | Needed for RTX in simulcast; blocks complete rrid demux |
| PR #75: route RTX/FEC repair packets | **Open** | Depends on PR #72; routing correctness for repair streams |
| PR #84: JitterBuffer interceptor | **Open** | Nice-to-have for latency measurement accuracy |
| PR #85: GCC bandwidth estimator | **Open** | Required for adaptive simulcast layer selection |
| Issue #12: rrid support | **Open** (closed by PR #72, which is itself open) | Foundational gap |
| Issue #774: IPv6 ICE gather | **Open** | Phase 4b cloud-relay Safari risk (tracked as hud-0bqk8) |
| Issue #781: TCP ICE | **Open** | Phase 4b firewall traversal; open PR #789 |

### Discovered follow-ups for tze_hud coordinator

1. **Monitor rtc PR #72 merge** — when PR #72 merges to `rtc` master and the `webrtc` wrapper
   cuts a second alpha (alpha.2), reassess rrid readiness. The PR passes all tests and is
   ready for review; merge is a near-term event.
2. **Monitor webrtc-rs/webrtc alpha.2 release** — v0.20 stable requires: alpha.2 at minimum
   (incorporating current open PRs), browser interop testing by the maintainers, and API
   stabilization. No timeline announced; watch the `webrtc-rs/webrtc` releases page.
3. **Phase 4 kickoff gate** — before implementing the `hud-fpq51` harness, the Phase 4
   kickoff bead must execute this checklist:
   - Is webrtc-rs v0.20 stable (not alpha) available?
   - Is `rrid` RTX (PR #72) merged and promoted to the async wrapper?
   - Is GCC or equivalent bandwidth estimation interceptor available?
   - If any answer is "no" after a one-week survey, proceed with webrtc-rs 0.17 limited scope
     or escalate to str0m fallback evaluation (hud-1ee3a).
4. **No action needed on str0m today** — hud-1ee3a (full fallback audit) remains open at P2.
   The spike finding is that str0m does not resolve the core gap (browser interop evidence) and
   carries its own integration cost. Defer the full audit to Phase 4 kickoff.

---

## 7. Summary

| Criterion | Status (April 2026) |
|---|---|
| webrtc-rs v0.20 stable release | **Not available** — alpha.1 only |
| RID header extension (send + receive) | **Implemented** in rtc v0.5.0+ |
| rrid (Repaired RTP Stream ID) | **Open PR #72** — not in any release |
| MID negotiation / BUNDLE | **Implemented** in SDP layer |
| Simulcast SDP (RFC 8853) | **Substantially implemented** in rtc master |
| Layer switching via RTCP | **Not implemented** (periodic PLI workaround only) |
| GCC / REMB bandwidth estimation | **Open PR #85** |
| Browser interop evidence | **None documented** for any webrtc-rs version |
| str0m as fallback | **Feasible but not faster** — same evidence gap, different integration cost |

**Go/no-go for Phase 4b harness**: **NO-GO today.** Re-evaluate at Phase 4 kickoff when webrtc-rs
v0.20 stable is expected. Key signals to watch: webrtc-rs/webrtc alpha.2, rtc PR #72 merge,
GCC interceptor merge. If these land before Phase 4 kickoff, upgrade to **CONDITIONAL GO**
(harness implementation can begin, still subject to hud-fpq51 gate criteria).

---

## Monitoring Update (2026-04-19)

**Trigger:** hud-af6g1 phase 4 kickoff evaluation watch. Re-verified rtc PR #72 and webrtc-rs alpha.2 release status.

### rtc PR #72 Status (rrid RTX association)

**Status as of 2026-04-19:** **Open, awaiting review**

The PR remains unmerged. Last activity was **April 10, 2026**, when the author rebased onto upstream master to resolve merge conflicts and address maintainer review feedback from rainliu. The PR passes all tests on its branch but requires maintainer sign-off before merging.

**Delta from audit date:** No substantive change. The PR was open on 2026-04-18 (audit date); still open 2026-04-19 (monitoring date). No indication of imminent merge.

### webrtc-rs v0.20.0-alpha.2 Release Status

**Status as of 2026-04-19:** **Not released**

The latest alpha release is still v0.20.0-alpha.1 (released 2026-03-01). No alpha.2 has been announced or published.

The latest stable remains v0.17.1 (released 2026-02-06).

The rtc crate shows stable at v0.9.0 (2026-02-08) with no subsequent releases.

**Delta from audit date:** No change. The audit baseline was "only one alpha released since March"; this monitoring confirms alpha.2 remains absent through 2026-04-19.

### Phase 4 Kickoff Gate Signal

| Gate signal | Required | Status | Blocker? |
|---|---|---|---|
| rtc PR #72 (rrid RTX) merged | Yes | Open; awaiting review | **YES** |
| webrtc-rs v0.20.0-alpha.2 released | Yes (soft signal) | Not released | **YES** |
| webrtc-rs v0.20.0 stable released | Yes (required) | Not released | **YES** |

**Conclusion:** Phase 4 kickoff gate remains **NO-GO**. All three signals are still absent as of monitoring date. The rrid RTX signal (PR #72 merge) is the most tractable near-term blocker; its merge would unblock alpha.2. Neither event has materialized in the 10 days since the original audit.

**Recommendation:** Maintain existing gate. No Phase 4b harness implementation should begin until rtc PR #72 is merged AND webrtc-rs/webrtc cuts alpha.2 with that PR incorporated.

---

## Sources

- webrtc-rs GitHub releases: https://github.com/webrtc-rs/webrtc/releases
- webrtc-rs/rtc GitHub releases: https://github.com/webrtc-rs/rtc/releases
- webrtc-rs blog: https://webrtc.rs/blog/
- Announcing rtc 0.5.0 (simulcast support): https://webrtc.rs/blog/2026/01/05/announcing-rtc-v0.5.0.html
- Announcing rtc 0.6.0 (interceptor framework): https://webrtc.rs/blog/2026/01/09/announcing-rtc-v0.6.0.html
- RTC feature complete — what's next: https://webrtc.rs/blog/2026/01/18/rtc-feature-complete-whats-next.html
- WebRTC v0.20.0-alpha.1 announcement: https://webrtc.rs/blog/2026/03/01/webrtc-v0.20.0-alpha.1-async-webrtc-on-sansio.html
- webrtc-rs/rtc issue #12 (rrid support): https://github.com/webrtc-rs/rtc/issues/12
- webrtc-rs/rtc PR #72 (rrid RTX association): https://github.com/webrtc-rs/rtc/pull/72
- webrtc-rs/rtc PR #84 (JitterBuffer): https://github.com/webrtc-rs/rtc/pull/84
- webrtc-rs/rtc PR #85 (GCC bandwidth estimator): https://github.com/webrtc-rs/rtc/pull/85
- webrtc-rs/webrtc issue #774 (IPv6 ICE): https://github.com/webrtc-rs/webrtc/issues/774
- webrtc-rs/webrtc issue #781 (TCP ICE): https://github.com/webrtc-rs/webrtc/issues/781
- str0m GitHub repository: https://github.com/algesten/str0m
- str0m CHANGELOG.md: https://github.com/algesten/str0m/blob/main/CHANGELOG.md
- Prior audit: docs/audits/webrtc-rs-audit.md (hud-ora8.1.17, PR #523)
- Phase 4 simulcast interop plan: docs/testing/simulcast-interop-plan.md (hud-fpq51, PR #538)
