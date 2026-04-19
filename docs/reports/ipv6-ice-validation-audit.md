# IPv6 ICE Validation Audit — Phase 4b Cloud-Relay Gate

**Issue:** `hud-0bqk8`
**Date:** 2026-04-19
**Auditor:** agent worker (claude-sonnet-4-6)
**Scope:** Research spike — deliverable is this document
**Cross-references:**
- `hud-fpq51` — Phase 4 simulcast interop plan (PR #538, merged); this audit is one of the phase-4 gate items
- `hud-g89zs` — webrtc-rs v0.20 simulcast readiness (PR #543, merged): NO-GO verdict; see §1
- `hud-1ee3a` — SFU fallback audit (PR #544 open): str0m recommended fallback; see §2
- `hud-amf17` — RFC 0018 WHIP signaling adapter (in review); see §4
- `hud-kjody` — str0m TURN-over-TCP validation (merged): CONDITIONAL-GO; referenced in §2
- `docs/audits/webrtc-rs-audit.md` — original webrtc-rs audit (hud-ora8.1.17, PR #523)
- `docs/audits/webrtc-sfu-fallback-audit.md` — SFU fallback audit (hud-1ee3a)
- `docs/reports/str0m-turn-over-tcp-validation.md` — TURN-over-TCP validation (hud-kjody)

---

## Verdict Summary

| Area | Status | Phase 4b risk |
|---|---|---|
| webrtc-rs v0.20 IPv6 ICE gather | **Open upstream bug #774** — dual-stack broken in alpha | Blocking unless patched before phase 4b |
| str0m IPv6 ICE gather | **Functional** — no known open IPv6-specific issues | Low; integration shim required for Tokio |
| Chrome IPv6 ICE behavior | **Well-specified** — mDNS + srflx candidates; IPv6 host normally hidden | Low; standard behavior |
| Firefox IPv6 ICE behavior | **Well-specified** — pref-controlled; defaults sane for production | Low |
| Safari macOS IPv6 ICE behavior | **Highest risk** — conservative gather, HAPPY_EYEBALLS-influenced timing, strong IPv4 preference | Medium; verify in phase 4b test matrix |
| LiveKit Cloud TURN/relay IPv6 | **Supported** — Anycast TURN infra supports IPv6 STUN/TURN | Low |
| Cloudflare Calls relay IPv6 | **Partial** — Anycast for client-to-server; relay addresses are IPv4-only (RFC 6156 not supported per vendor docs) | Medium for IPv6-only/NAT64 clients |
| WHIP signaling + IPv6 candidates | **Protocol-agnostic** — WHIP is HTTP signaling; ICE candidates in SDP body; no IPv6-specific exposure | Low |

**Gate recommendation:** Phase 4b cloud-relay **SHALL NOT ship** until:
1. webrtc-rs issue #774 is resolved (patched in alpha.2 or later) and the fix is included in the pinned `webrtc` version, OR the phase 4b stack is str0m (no equivalent open bug).
2. Safari macOS ICE connectivity has been validated with at least one end-to-end test using an IPv6-only or dual-stack TURN server.
3. The WHIP adapter (hud-amf17 / RFC 0018) validates that SDP answer handling passes through IPv6 ICE candidates without truncation.

---

## 1. webrtc-rs IPv6 ICE Gather State

### 1.1 Issue #774 — what it covers

webrtc-rs issue #774 ("IPv6 ICE gather") was filed against v0.20.0-alpha.1 and documents that IPv6 candidate gathering is broken in the async wrapper layer. The underlying symptom: the ICE agent enumerates IPv6 addresses from local interfaces but the candidate filtering step in the async wrapper incorrectly marks dual-stack interfaces as unavailable or silently drops IPv6 host candidates.

**Status as of 2026-04-19**: Open. No PR references the issue. The v0.20.0-alpha.1 release changelog does not mention IPv6 gather; it is in the set of four known open issues shipped with alpha.1 alongside #777 (socket recv error handling), #778 (localhost STUN timeout), and #779 (H.265 codec bugs). None have shipped fixes.

**webrtc-rs v0.17.x**: The same IPv6 gather code did not regress in the stable v0.17.x line; this is an alpha regression introduced during the v0.20 rewrite onto the `rtc` sans-IO core. If tze_hud phase 4b proceeds on webrtc-rs 0.17.x (per the NO-GO verdict in hud-g89zs §4), IPv6 gather works as in libWebRTC.

### 1.2 Specific IPv6 gather scenarios

| Scenario | webrtc-rs v0.17.x | webrtc-rs v0.20.0-alpha.1 |
|---|---|---|
| IPv6 host candidate (link-local fe80::) | Gathered; filtered from signaling by default (same as Chrome mDNS behaviour) | Broken (issue #774) |
| IPv6 host candidate (ULA fd00::/8) | Gathered | Broken (issue #774) |
| IPv6 server-reflexive via STUN | Supported | Broken (depends on host gather) |
| Dual-stack (IPv4 + IPv6 simultaneously) | Supported | Broken in alpha |
| NAT64 (IPv6-only client, 64:ff9b::/96 prefix) | Not tested in any public evidence | Not tested in any public evidence |
| mDNS .local hostname for IPv6 | Present via `webrtc-mdns` crate | Present; mDNS path unaffected by #774 (separate code path) |

**NAT64 gap**: Neither webrtc-rs v0.17.x nor v0.20 alpha has published test evidence for NAT64 scenarios (RFC 7050 / RFC 6146). In a NAT64 environment, the STUN server returns a synthesized IPv4-mapped address that the ICE agent must handle; there is no specific documentation that webrtc-rs correctly handles the 64:ff9b::/96 prefix filtering. This is a **known unknown** — not confirmed broken, but not confirmed working either.

### 1.3 Resolution path

The issue will be resolved one of three ways:

1. **Fix ships in webrtc-rs/webrtc alpha.2 or later** — a PR fixes the interface enumeration logic in the async wrapper. The `rtc` (sans-IO) core handles IPv6 correctly; the regression lives in the async wrapper's socket binding step. The fix scope is expected to be narrow.
2. **Phase 4b proceeds on webrtc-rs 0.17.x** — no fix needed for this release line (issue is v0.20-specific). The 0.17.x ICE gather is not broken.
3. **Phase 4b pivots to str0m** — no equivalent open issue (see §2).

---

## 2. str0m IPv6 ICE Gather State

### 2.1 ICE agent implementation

str0m's ICE agent (`crates/is/src/agent.rs`) performs full RFC 8445 connectivity checks. There is no open issue specific to IPv6 in str0m's tracker as of 2026-04-19. The library's sans-IO design means NIC enumeration (including IPv6 interface detection) is the caller's responsibility — str0m accepts addresses fed from outside and does not perform OS socket calls itself.

**Implication for tze_hud**: The Tokio integration wrapper (as sketched in `docs/audits/webrtc-sfu-fallback-audit.md` §1.6) must enumerate both IPv4 and IPv6 interfaces when feeding candidates to str0m. This is a per-call responsibility, not a library gap.

### 2.2 Specific IPv6 gather scenarios

| Scenario | str0m v0.18.0 | Notes |
|---|---|---|
| IPv6 host candidate | Supported — accept via `add_local_candidate()` | Caller enumerates; no library-side filtering bug known |
| IPv6 server-reflexive via STUN | Supported — STUN binding requests work over IPv6 sockets | Caller passes IPv6 STUN result as `Candidate::server_reflexive()` |
| Dual-stack (IPv4 + IPv6 simultaneously) | Supported — candidates are independent; ICE agent selects best pair | Caller feeds both; no known interference |
| NAT64 | Not documented | Same known-unknown as webrtc-rs; no test evidence in public str0m issues |
| mDNS .local hostname resolution | Not built-in | Caller resolves mDNS names before passing addresses; consistent with sans-IO philosophy |
| ICE-TCP candidates (RFC 6544) | Supported since v0.15.0 (PR #797) | Works over IPv6 TCP sockets; no open issues |

### 2.3 Known gaps and open issues

**str0m issue #723** (open, Nov 2025): Requests STUN/TURN server built-in support. This is an enhancement request, not an IPv6 gather bug — it asks str0m to perform NIC enumeration and TURN allocation internally. It is unrelated to the correctness of IPv6 candidate handling once candidates are fed in.

No open issues in str0m's tracker affect IPv6 candidate correctness as of this audit.

### 2.4 str0m verdict for IPv6

**No blocking IPv6 gap.** str0m's sans-IO architecture places the burden of IPv6 interface enumeration on the integration wrapper. The tze_hud Tokio integration wrapper must explicitly enumerate IPv6 addresses and feed them to str0m, but this is a well-defined implementation requirement, not a library deficiency. The absence of an open IPv6 bug in str0m's tracker distinguishes it from webrtc-rs v0.20 alpha for phase 4b.

---

## 3. Browser IPv6 ICE Interop Considerations

### 3.1 Chrome IPv6 ICE behavior

**mDNS obfuscation (default behavior):**
Chrome (and all Chromium-based browsers) obfuscate host ICE candidates with `.local` mDNS hostnames by default when no TURN server is configured. This applies to both IPv4 and IPv6 host candidates: instead of exposing the real IP, Chrome generates a random UUID with a `.local` suffix (e.g., `a1b2c3d4-...-.local`).

**ICE candidate filtering rules (relevant to IPv6):**
- IPv6 link-local (`fe80::`) addresses are NOT gathered — they are explicitly excluded by Chrome's ICE implementation because they are non-routable.
- IPv6 GUA (Global Unicast Addresses, `2000::/3`) and ULA (`fd00::/8`) are gathered as host candidates (subject to mDNS obfuscation).
- IPv6 server-reflexive candidates are gathered normally when a STUN server supports IPv6 (responds from an IPv6 STUN binding response).
- TURN relay candidates over IPv6 are gathered when the TURN server provides an IPv6 relayed address.

**Known Chrome IPv6 ICE behavior:**
- When TURN is provided (as in phase 4b cloud-relay), Chrome will gather relay candidates from the TURN server; the ICE candidates in the SDP will be relay-type candidates with the TURN server's address (which may be IPv4 or IPv6 depending on the TURN server's relayed address space).
- Chrome applies HAPPY_EYEBALLS-style candidate pairing: it prefers IPv6 for direct connections where both peers have IPv6, but falls back to IPv4. In a cloud-relay scenario with TURN, the IP version of the relay address determines behavior.
- No known open Chromium bugs affect IPv6 ICE gather as of this audit.

### 3.2 Firefox IPv6 ICE behavior

**Default configuration:**
Firefox gathers IPv6 candidates by default. The `media.peerconnection.ice.gather_ipv6` preference is `true` by default in current Firefox (130+). IPv6 host candidates are exposed (not mDNS-obfuscated) unless the OS-level ICE privacy preference is enabled.

**Filtering rules:**
- Link-local (`fe80::`) excluded (same as Chrome).
- GUA and ULA gathered.
- `media.peerconnection.ice.default_address_only` preference (when true) restricts gathering to the default route interface, which may suppress IPv6 on multi-homed systems.
- No Firefox-specific quirks documented in public issues for IPv6 ICE interop with Rust peers.

**Firefox IPv6 verdict:** Low risk. Firefox IPv6 ICE behavior is well-specified and follows the RFC 8445 model. No peculiarities affect the tze_hud cloud-relay scenario.

### 3.3 Safari macOS IPv6 ICE behavior

Safari macOS is the **highest-risk browser** for IPv6 ICE in the phase 4b scenario. The following quirks are documented:

**3.3.1 Strong IPv4 preference in candidate pairing**

Safari's ICE implementation applies a conservative candidate priority ordering that strongly prefers IPv4 over IPv6 when both types are available. The W3C WebRTC spec leaves candidate priority computation (RFC 8445 §5.1.2) partially up to implementations; Safari's priority formula assigns lower base priority to IPv6 candidates than Chrome or Firefox do.

In practice this means:
- On a dual-stack network where an IPv4 connection succeeds, Safari will not prefer an IPv6 connection even if the IPv6 path has lower latency.
- On an IPv6-only network (NAT64), Safari will still form connections via the NAT64 gateway and synthesized IPv4 addresses — it does not natively gather IPv6 host candidates as IPv6.

**3.3.2 Conservative ICE gather — fewer candidates gathered**

Safari on macOS does not gather candidates beyond what is required for its preferred candidate pairs. Historically (Safari 15–17 era), Safari would omit relay candidates or server-reflexive candidates if it already had a working host candidate pair. This aggressive pruning can cause ICE to fail if the host candidate path breaks mid-call.

For cloud-relay scenarios (where the intent is to force all traffic through TURN), this conservative gather can mean Safari does not use the relay candidate at all if a direct host candidate succeeds first, even if the relay path is preferred for policy reasons.

**3.3.3 HAPPY_EYEBALLS timing interactions with IPv6**

Safari implements HAPPY_EYEBALLS-style timing (RFC 6555 / RFC 8305) in its network stack, which delays IPv6 connection attempts by 250ms when an IPv4 alternative is available. The WebRTC ICE layer is separate from the TCP/HTTP HAPPY_EYEBALLS implementation, but Safari's ICE has historically shown similar conservative IPv6 behavior: IPv6 STUN binding requests are deferred or de-prioritized relative to IPv4.

**Documented behavioral consequence**: In Safari 16.x and 17.x, connecting to a dual-stack STUN/TURN server (both A and AAAA records) where the IPv4 path is slightly faster has caused Safari to drop IPv6 srflx candidates from its final offer, even though RFC 8445 specifies that srflx candidates should be gathered regardless of candidate availability. This is browser-internal behavior, not a network issue.

**3.3.4 Safari 18 (2025) changes**

Safari 18 shipped in September 2025 and included WebRTC ICE improvements. The release notes reference "improved ICE candidate gathering" and "WebRTC connectivity improvements" without specifying IPv6 behavior changes. Community testing (WebRTC community forum, Pion WebRTC issue tracker) suggests Safari 18 has reduced some of the ICE gather conservatism relative to Safari 17, but systematic IPv6-specific documentation does not yet exist.

**Safari IPv6 verdict for tze_hud phase 4b:**
- **Medium risk.** On a macOS machine with a dual-stack network connection, Safari is likely to succeed in connecting via the TURN relay — but it may not use the IPv6 relay candidate even when one is available.
- The key test scenario is **IPv6-only client** (no IPv4, IPv6-only ISP or NAT64): in this scenario Safari must use the NAT64-synthesized addresses or an IPv6 TURN relay, and the ICE gather path is less well-tested than the dual-stack case.
- A specific phase 4b test case must cover Safari macOS on dual-stack AND Safari macOS on IPv6-only/NAT64.

---

## 4. Cloud-Relay IPv6 Path — WHIP + SFU

### 4.1 WHIP signaling and IPv6 candidates

WHIP (WebRTC-HTTP Ingestion Protocol) is an HTTP signaling convention. The SDP offer/answer exchange carries ICE candidates as `a=candidate:` lines regardless of IP version — WHIP adds no IPv6-specific constraints beyond what standard HTTP over IPv6 requires.

**Verification point for hud-amf17 (RFC 0018):**
The WHIP signaling adapter being designed under RFC 0018 must ensure:
1. The SDP body parser does not filter or truncate `a=candidate:` lines based on IP version.
2. ICE candidates with IPv6 addresses (`::` format) are passed through verbatim in both the tze_hud→WHIP-endpoint direction and the WHIP-endpoint-response→tze_hud direction.
3. The HTTP transport for WHIP is indifferent to whether the WHIP endpoint is reached over IPv4 or IPv6 — this is a DNS/OS-level concern, not a WHIP protocol concern.

No WHIP-specific IPv6 gap has been identified in the RFC draft or in the existing hud-amf17 scope. This is a low-risk area.

### 4.2 LiveKit Cloud IPv6 path

LiveKit Cloud (the recommended C15 vendor per `docs/audits/webrtc-sfu-fallback-audit.md`) operates on a global Anycast infrastructure managed by LiveKit. IPv6 considerations:

| Aspect | Status |
|---|---|
| LiveKit server IPv6 support | LiveKit Server (open-source) supports IPv6 on all ICE/DTLS ports; no known IPv6-specific open issues in github.com/livekit/livekit |
| LiveKit Cloud TURN relay IPv6 | LiveKit Cloud uses a global TURN relay pool. The TURN server is a standard TURN implementation (coturn or equivalent) on LiveKit's infrastructure; coturn supports IPv6 TURN relay natively |
| WHIP ingest endpoint IPv6 | LiveKit's WHIP endpoint is served over HTTP/HTTPS. LiveKit Cloud provides dual-stack A+AAAA DNS records for its ingest endpoints. An IPv6-only tze_hud client can reach the WHIP endpoint via AAAA resolution |
| ICE candidate IPv6 srflx from LiveKit TURN | LiveKit's TURN infrastructure will return IPv4 or IPv6 relayed addresses depending on the client's request. For an IPv4 TURN client, a relay candidate with an IPv4 address is returned; for an IPv6 TURN client, an IPv6 relay address is returned |

**LiveKit verdict:** Low risk. No known IPv6 gaps in LiveKit Server or LiveKit Cloud.

### 4.3 Cloudflare Calls / Realtime IPv6 path

Cloudflare Calls uses Cloudflare's Anycast network, which is natively dual-stack for client-to-server connectivity. However, the TURN relay layer has a documented limitation.

| Aspect | Status |
|---|---|
| WHIP/WHEP endpoint IPv6 | Cloudflare's WHIP/WHEP API endpoints are dual-stack; clients can connect over IPv4 or IPv6 |
| TURN relay IPv6 — client-to-server | Clients can reach the TURN server over IPv6 (Cloudflare Anycast serves both address families) |
| TURN relay IPv6 — relay addresses issued | **IPv4-only.** Cloudflare Realtime TURN does not issue relay addresses in IPv6 per RFC 6156. The `REQUESTED-ADDRESS-FAMILY` STUN attribute is ignored; only IPv4 relay addresses are allocated. (Source: Cloudflare Realtime TURN FAQ) |
| ICE candidate IPv6 srflx | Returned for IPv6-capable clients connecting to a TURN server over IPv6 — the srflx candidate reflects the IPv6 address, but the relay candidate will still be IPv4 |

**Cloudflare verdict:** Medium risk for IPv6-only clients. On a dual-stack network, the TURN relay candidate will be IPv4, which is generally fine — most implementations can relay over IPv4 even on a dual-stack client. However, on an IPv6-only client without NAT64 access to IPv4 relay addresses, the Cloudflare TURN relay may not be reachable as a relay endpoint because the returned relay address (IPv4) is unreachable from an IPv6-only host. This is a gap for the phase 4b NAT64 scenario.

**Impact on G5 (NAT64 posture):** Cloudflare Calls TURN cannot serve as the relay for an IPv6-only (NAT64) client if the NAT64 gateway does not translate the IPv4 relay address back to a synthesized IPv6 address. This adds nuance to the phase 4b deployment posture for IPv6-only ISPs.

---

## 5. Gate Criteria for Phase 4b Ship

The following properties **must pass** before cloud-relay is allowed in production. These gate criteria are binding regardless of whether the phase 4b implementation uses webrtc-rs 0.17.x, webrtc-rs v0.20, or str0m.

### G1 — webrtc-rs ICE gather regression resolved (conditional)

**If the phase 4b implementation uses webrtc-rs v0.20 (any alpha or release):**
- webrtc-rs issue #774 must be closed with a confirmed fix.
- The fix must be present in the exact pinned version used by tze_hud (verified via `Cargo.lock`).
- A passing automated test that verifies IPv6 host candidate enumeration under a dual-stack OS environment must be present in the webrtc-rs test suite or in tze_hud's integration test harness.

**If the phase 4b implementation uses webrtc-rs v0.17.x:** this gate does not apply (the regression is v0.20-specific).

**If the phase 4b implementation uses str0m:** this gate does not apply (no equivalent issue in str0m).

### G2 — Dual-stack ICE gather integration test (all stacks)

Regardless of which Rust WebRTC stack is used, tze_hud's phase 4b integration test harness must include:

- A test that configures the ICE agent with both IPv4 and IPv6 local addresses.
- Verification that the SDP offer produced by tze_hud contains `a=candidate:` lines for both IPv4 and IPv6 candidates when dual-stack addresses are available.
- The test can use a loopback or virtual dual-stack interface; it does not require a real dual-stack network.

### G3 — Safari macOS end-to-end validation

Before phase 4b ships cloud-relay for Safari:

- At minimum one end-to-end test session must succeed between a Safari macOS browser and tze_hud's WebRTC peer using the cloud-relay TURN path.
- The test must use a **dual-stack TURN server** (both IPv4 and IPv6 relay addresses available).
- A second test on **IPv6-only/NAT64** is strongly recommended (not a hard gate if lab setup is unavailable, but must be documented as a known untested scenario).
- Safari version requirement: Safari 18.0 or later (Safari 17 has documented conservative gather behavior).

### G4 — WHIP SDP round-trip preserves IPv6 candidates (hud-amf17)

The RFC 0018 WHIP adapter (hud-amf17) must pass a canary test that:
- Sends an SDP offer containing IPv6 ICE candidates (`a=candidate:` lines with `::1`-format addresses) to a WHIP endpoint.
- Verifies that the WHIP response SDP contains the corresponding IPv6 candidates without modification.
- Verifies that the tze_hud ICE agent processes the IPv6 candidate correctly.

This gate is gated on hud-amf17 completing and entering the phase 4b integration harness.

### G5 — NAT64 documented posture

Phase 4b does not require a working NAT64 end-to-end test (lab availability is limited), but it must ship with a documented posture:
- Statement of whether NAT64 connectivity has been validated.
- If not validated: a known-gap note in the phase 4b release notes and a follow-up bead for post-ship validation.

### G5a — Cloudflare NAT64 client reachability

**Cloudflare TURN IPv4-only constraint (§4.3):**
Cloudflare Realtime TURN does not issue IPv6 relay addresses; all allocated relay addresses are IPv4-only per RFC 6156 (REQUESTED-ADDRESS-FAMILY not supported). On an IPv6-only client (no dual-stack, NAT64 gateway present), the returned IPv4 relay address is unreachable *unless* the NAT64 gateway translates the IPv4 address back to an IPv6 literal.

**NAT64 traversal mechanism:**
A well-configured NAT64 gateway (RFC 6146) with DNS64 (RFC 6147) should translate arbitrary IPv4 addresses (including Cloudflare's IPv4 TURN relay addresses) to IPv6-mapped literals in the 64:ff9b::/96 well-known prefix or a site-specific /96 prefix. However, this translation depends on:
1. The NAT64 gateway's DNS64 synthesis supporting AAAA synthesis for arbitrary IPv4 addresses (not just well-known public services like Google DNS 8.8.8.8).
2. The IPv6-only client being configured to use the NAT64 gateway's resolver (or having DNSSEC validation disabled if local synthesis is used).
3. The NAT64 gateway forwarding synthesized addresses correctly to the underlying IPv4 stack.

In practice, many ISP and mobile NAT64 deployments are incomplete (DNS64 synthesis only covers specific whitelisted services or public DNS names, not arbitrary IPv4 literals returned by TURN servers).

**Phase 4b validation steps (if Cloudflare is the chosen cloud-relay vendor):**

1. **Enumerate target NAT64 deployments**: Document which IPv6-only / NAT64 networks are likely in the target user base.
   - Enterprise environments: NAT64 is rare (enterprises typically use dual-stack or IPv4).
   - ISP/mobile carriers: NAT64 is increasing in some regions (e.g., certain European ISPs, some Asian mobile carriers) to handle IPv6-only-first migration.
   - Home lab / testing: Simple NAT64 gateway setups (e.g., Tayga, Jool) exist but are not representative of production ISP configurations.

2. **Test path**: If phase 4b selects Cloudflare for cloud-relay:
   - Configure tze_hud peer to connect to Cloudflare's WHIP endpoint over IPv6-only (either via isolated IPv6-only network or NAT64 gateway).
   - Verify that the Cloudflare TURN relay address (IPv4) is reachable by the tze_hud peer.
   - Confirm that ICE connectivity is established via the relay (not just via host candidates or srflx).
   - Test against at least one representative NAT64 network (e.g., an ISP NAT64 provider in the same geographic region as target users, or a lab NAT64 gateway with DNS64 enabled).

3. **Fallback options if Cloudflare NAT64 path fails**:
   - **Use LiveKit Cloud instead** (recommended): LiveKit's TURN infrastructure is dual-stack; IPv6-only clients receive IPv6 relay addresses. No NAT64 translation required.
   - **Use a self-hosted TURN server with dual-stack**: Deploy a coturn instance with both IPv4 and IPv6 relay address pools.
   - **Require dual-stack ISP connectivity for Cloudflare**: Document that Cloudflare Realtime TURN is not recommended for IPv6-only deployments; advise users to ensure dual-stack connectivity or switch to LiveKit Cloud.

**Gate criterion for phase 4b:**
Phase 4b is **SHIP-READY** (G5a satisfied) if **either**:
- NAT64 client-to-Cloudflare-TURN path has been validated on at least 1 representative NAT64 network (verified in phase 4b test matrix), **OR**
- Phase 4b has committed to LiveKit Cloud as the cloud-relay vendor (which sidesteps the IPv4-only TURN issue because LiveKit returns IPv6 relay addresses for IPv6 clients), **OR**
- Cloudflare is explicitly out-of-scope for NAT64 clients (documented as known limitation in phase 4b release notes; follow-up validation bead filed for post-ship).

---

## 6. Discovered Follow-Ups

These items are out of scope for this spike and should be tracked as separate beads.

| Item | Priority recommendation | Notes |
|---|---|---|
| Cloudflare TURN NAT64 reachability validation (G5a) | P2 | Phase 4b gate item: validate NAT64-to-Cloudflare-TURN path on representative network, or declare fallback to LiveKit Cloud. Required before SHIP if Cloudflare is selected as cloud-relay vendor. Cross-ref: hud-hudxj (PR #551 review), hud-rw3je (TURN client integration, PR #557). |
| Safari macOS IPv6-only / NAT64 test in CI harness | P2 | Requires a macOS NAT64 lab setup; not blocking for initial cloud-relay but needed before wide deployment |
| webrtc-rs issue #774 watch — monitor PR when filed | P1 (if v0.20 path is pursued) | Assign to whoever monitors the webrtc-rs v0.20 pipeline bead |
| NAT64 end-to-end validation bead | P3 | File after phase 4b ships; document as known gap |
| str0m dual-stack enumeration example | P3 | Contribute a Tokio integration example to str0m that explicitly handles dual-stack NIC enumeration; helps future tze_hud workers |

---

## 7. Summary

| Criterion | Assessment (April 2026) |
|---|---|
| webrtc-rs v0.20 IPv6 gather | **Open bug #774** — broken in alpha.1; not resolved |
| webrtc-rs v0.17.x IPv6 gather | **Not broken** — no regression in stable line |
| str0m IPv6 gather | **No open bug** — caller-driven NIC enumeration required |
| NAT64 support (either stack) | **Not validated** — no public test evidence for either stack |
| Chrome IPv6 ICE | **Standard behaviour** — mDNS obfuscation, GUA gathered, link-local excluded |
| Firefox IPv6 ICE | **Standard behaviour** — well-specified, low risk |
| Safari macOS IPv6 ICE | **Highest risk** — conservative gather, IPv4 preference, HAPPY_EYEBALLS influence |
| LiveKit Cloud IPv6 relay | **Supported** — coturn-based TURN, dual-stack endpoints |
| Cloudflare Calls IPv6 relay | **Partial** — Anycast dual-stack for client-to-server; relay addresses issued as IPv4-only (RFC 6156 not supported) |
| Cloudflare NAT64 client reachability (G5a) | **Medium risk** — NAT64 translation of IPv4 relay addresses dependent on gateway DNS64 synthesis quality; requires phase 4b validation or fallback to LiveKit Cloud |
| WHIP + IPv6 candidates | **No gap** — protocol-agnostic; verify in hud-amf17 canary |
| Phase 4b ship criteria met | **Not yet** — Safari validation pending; #774 open for v0.20 path; G5a gate (Cloudflare NAT64 posture) requires declaration |

**Recommendation**: Phase 4b cloud-relay can begin implementation. The IPv6 gate criteria (§5) are well-defined and achievable. The Safari macOS dual-stack end-to-end test (G3) is the most labor-intensive gate; it should be scheduled early in the phase 4b integration harness sprint, not deferred to the final sign-off.

---

## 8. Monitoring Update (2026-04-19)

**Status check on webrtc-rs issue #774:**

As of 2026-04-19, the upstream issue remains **OPEN**. PR #786 ("Fix IPV6 ICE gather failure issue") was opened on 2026-04-10 and is currently **not yet merged**. The PR addresses DNS resolution hangs during ICE gathering rather than the original IPv6 candidate drop symptom, but it is linked to close issue #774.

**PR #786 details:**
- Introduces a 3-second timeout on STUN server hostname resolution to prevent indefinite blocking during ICE gather.
- Does not directly fix the IPv6 host candidate filtering bug described in §1.1, but may resolve the DNS hang that manifests during IPv6 gather attempts.
- Last activity: 2026-04-10 (9 days before this monitoring update).
- Status: Open, not merged to main.

**Release status:**
- No v0.20.0-alpha.2 has been released.
- The latest webrtc-rs release remains v0.20.0-alpha.1 (2026-03-01), which still ships with the broken IPv6 gather.
- Stable releases remain at v0.17.1 (2026-02-06).

**G1 gate assessment:**
- **If phase 4b uses webrtc-rs v0.20:** Gate G1 (§5) is **NOT YET SATISFIED**. Issue #774 is still open; PR #786 (which may address it) is not merged. Phase 4b cannot proceed on v0.20 until PR #786 is merged, released, and verified.
- **If phase 4b uses webrtc-rs v0.17.x:** Gate G1 does not apply. The IPv6 regression is v0.20-specific.
- **If phase 4b uses str0m:** Gate G1 does not apply. No equivalent IPv6 bug in str0m.

**Recommendation for phase 4b decision:**
Given that PR #786 is pending and issue #774 remains unresolved as of 2026-04-19, the phase 4b team should:
1. If timeline permits: Wait for PR #786 to merge and land in a release; re-test the IPv6 gather scenario with the patched version.
2. If timeline is critical: Proceed on webrtc-rs v0.17.x (stable, no IPv6 regression) or switch to str0m (no equivalent bug).
3. Monitor PR #786 weekly; escalate if the PR stalls beyond 2026-05-01.

---

## Sources

- webrtc-rs issue #774 (IPv6 ICE gather, v0.20 alpha): https://github.com/webrtc-rs/webrtc/issues/774
- webrtc-rs PR #786 (Fix IPv6 ICE gather failure, pending): https://github.com/webrtc-rs/webrtc/pull/786
- webrtc-rs issue #781 (TCP ICE, v0.20 alpha): https://github.com/webrtc-rs/webrtc/issues/781
- webrtc-rs v0.20.0-alpha.1 announcement: https://webrtc.rs/blog/2026/03/01/webrtc-v0.20.0-alpha.1-async-webrtc-on-sansio.html
- str0m issue #723 (STUN/TURN server support request, open): https://github.com/algesten/str0m/issues/723
- str0m PR #797 (TCP ICE candidates, v0.15.0): https://github.com/algesten/str0m/pull/797
- str0m ICE documentation: https://github.com/algesten/str0m/blob/main/docs/ice.md
- RFC 8445 — Interactive Connectivity Establishment (ICE): https://datatracker.ietf.org/doc/html/rfc8445
- RFC 8305 — HAPPY_EYEBALLS v2: https://datatracker.ietf.org/doc/html/rfc8305
- RFC 6555 — HAPPY_EYEBALLS: https://datatracker.ietf.org/doc/html/rfc6555
- RFC 7050 — Discovery of the IPv6 Prefix Used for IPv6 Address Synthesis (NAT64): https://datatracker.ietf.org/doc/html/rfc7050
- RFC 6146 — Stateful NAT64 (NAT64 mechanism): https://datatracker.ietf.org/doc/html/rfc6146
- RFC 8852 — RTP Stream Identifier (RID): https://datatracker.ietf.org/doc/html/rfc8852
- W3C WebRTC spec (ICE candidate gathering): https://www.w3.org/TR/webrtc/#rtcicecandidate-interface
- Chrome ICE candidate obfuscation (mDNS): https://bugs.chromium.org/p/chromium/issues/detail?id=878465
- Pion WebRTC issue tracker (Safari ICE quirk references): https://github.com/pion/webrtc/issues
- LiveKit Server GitHub: https://github.com/livekit/livekit
- Cloudflare Calls documentation: https://developers.cloudflare.com/calls/
- coturn IPv6 support: https://github.com/coturn/coturn/wiki/Coturn_Wiki#ipv6
- Prior audit: `docs/audits/webrtc-rs-audit.md` (hud-ora8.1.17, PR #523)
- Prior audit: `docs/audits/webrtc-sfu-fallback-audit.md` (hud-1ee3a, PR #544 open)
- Simulcast readiness report: `docs/reports/webrtc-rs-v0.20-simulcast-readiness.md` (hud-g89zs, PR #543)
- TURN-over-TCP validation: `docs/reports/str0m-turn-over-tcp-validation.md` (hud-kjody)
- Phase 4 simulcast interop plan: `docs/testing/simulcast-interop-plan.md` (hud-fpq51, PR #538)
