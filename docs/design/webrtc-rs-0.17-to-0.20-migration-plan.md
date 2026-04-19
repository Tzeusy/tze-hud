# webrtc-rs 0.17 → 0.20 Migration Plan

**Issue**: `hud-j1msi`  
**Date**: 2026-04-19  
**Author**: agent worker (claude-sonnet-4-6)  
**Status**: PLANNING — Phase 4 kickoff gate not yet open  
**Cross-references**:
- `hud-ora8.1.17` — webrtc-rs library audit, ADOPT-WITH-CAVEATS verdict (PR #523)
- `hud-g89zs` — v0.20 simulcast readiness audit, NO-GO verdict (PR #543)
- `hud-1ee3a` — SFU fallback audit, str0m recommended fallback (PR #544)
- `hud-kjody` — str0m TURN-over-TCP validation (PR #547)
- `hud-rw3je` — TURN client integration design (PR #546 or see `docs/design/turn-client-integration.md`)

---

## Executive Summary

tze_hud is pinned to `webrtc` v0.17.x for phase 1 (bounded ingress) per the hud-ora8.1.17
audit verdict. This document plans the migration to v0.20 at Phase 4 (bidirectional AV).

**Current gate status: NO-GO.** Three upstream signals must land before migration begins.
Until all three are satisfied, tze_hud remains on v0.17.x. If v0.20 stable is not ready at
Phase 4 kickoff, the fallback path is `str0m` v0.18 (see §7 and hud-1ee3a).

---

## Migration Gate Table

| Gate ID | Signal | Status (2026-04-19) | Owner | Next Check Date |
|---|---|---|---|---|
| GATE-1 | `rtc` PR #72 (rrid RTX association) merged to `rtc` master | **OPEN — not merged** (last activity 2026-04-10) | webrtc-rs/rtc maintainers | 2026-05-19 |
| GATE-2 | `rtc` PR #85 (GCC sender-side bandwidth estimator) merged | **OPEN — not merged** (blocks on PR #84) | webrtc-rs/rtc maintainers | 2026-05-19 |
| GATE-3 | `webrtc` v0.20.0-alpha.2 released (incorporating GATE-1 + GATE-2) | **NOT RELEASED** (latest: alpha.1, 2026-03-01) | webrtc-rs/webrtc maintainers | 2026-05-19 |

**Threshold to unblock migration**: All three gates satisfied AND a stable v0.20.0 release
(not alpha) published. Re-evaluate at Phase 4 kickoff.

---

## 1. Current webrtc-rs Usage in tze_hud

### 1.1 Adoption status

As of 2026-04-19, `webrtc-rs` (the `webrtc` crate) is **not yet declared as a Cargo
dependency** in any tze_hud crate. The workspace `Cargo.toml` and all 15 crate-level
manifests under `crates/` contain no `webrtc` dependency entry.

**Context**: The hud-ora8.1.17 audit established the selection decision and integration
guidance. Actual wiring of the `webrtc` crate into the relevant crates (expected:
`tze_hud_media_apple`, a forthcoming `tze_hud_media` crate, or a new `tze_hud_webrtc`
crate) is deferred to the Phase 1 bounded-ingress implementation beads. This migration
plan covers the version bump that will need to occur at Phase 4.

### 1.2 Anticipated API surface at Phase 1 (0.17.x)

Based on the hud-ora8.1.17 audit and integration guidance, Phase 1 bounded ingress will
consume the following API surface from `webrtc` v0.17.x:

| API component | Purpose | Location |
|---|---|---|
| `APIBuilder` + `MediaEngine` | Peer connection factory, codec registration | Session setup |
| `RTCPeerConnection` | WebRTC state machine (offer/answer, ICE, DTLS) | Session lifecycle |
| `RTCConfiguration` (ICE servers) | STUN/TURN server configuration | Session setup |
| `on_track` callback | RTP track arrival, `Arc<TrackRemote>` | Media ingest |
| `TrackRemote::read()` | RTP packet read from received track | RTP bridge loop |
| `webrtc_util::Marshal::marshal_to()` | Serialize RTP packet to bytes for GStreamer bridge | RTP bridge loop |
| `RTCSessionDescription` | SDP offer/answer exchange over gRPC signaling plane | Signaling |
| `RTCIceCandidate` | Trickle ICE candidate exchange | Signaling |

Phase 4 (bidirectional AV) will additionally require:

| API component | Purpose |
|---|---|
| `TrackLocalStaticRTP` / outbound RTP tracks | Agent audio synthesis egress |
| Simulcast encoding setup (RID parameters) | Adaptive-quality video to cloud SFU |
| `RTCRtpSender` + encoding parameter control | Outbound track management |

### 1.3 Crate structure in v0.17.x (sub-crates)

webrtc-rs is a workspace of sub-crates. Phase 1 will pull in `webrtc` (umbrella crate),
which re-exports the sub-crates. Explicit sub-crate dependencies are not needed unless
tze_hud adds direct `webrtc-ice`, `webrtc-sdp`, or `webrtc-dtls` usage.

---

## 2. Breaking Changes: 0.17 → 0.20

### 2.1 Architectural shift (the fundamental change)

v0.17.x is **Tokio-coupled**: the library starts its own Tokio tasks, owns async callbacks,
and exposes `Arc<Mutex<>>` throughout. v0.20 is built on the `rtc` sans-IO core: the
library is a pure state machine; the caller owns all I/O, sockets, and timer scheduling.

This is the largest migration surface. It is not a rename but a model change.

### 2.2 API-level breaking changes

| Area | v0.17.x | v0.20.x | Migration action |
|---|---|---|---|
| **Peer connection construction** | `APIBuilder::new().build().new_peer_connection(config)` | `Rtc::builder()` + runtime adapter | Rewrite session init |
| **Event handlers** | Callback closures (`on_track(Box<dyn Fn...>)`, `on_ice_candidate(...)`) | `poll_output()` loop + `Event` enum variants | Replace all callbacks with event poll |
| **Track receive** | `TrackRemote::read(&mut buf).await` on `Arc<TrackRemote>` | `Event::MediaData(data)` emitted from `poll_output()` | Rewrite RTP bridge ingest |
| **Track send** | `Arc<TrackLocalStaticRTP>` + `write_sample()` | Output packets via `poll_output()` → `Output::Transmit` | Rewrite egress path |
| **ICE candidate handling** | `RTCPeerConnection::add_ice_candidate()` + callback | `Input::Receive` (network packet) or dedicated trickle candidate method | Rewrite ICE input path |
| **SDP exchange** | `RTCSessionDescription::unmarshal()` + `set_remote_description()` | `rtc.sdp().offer(sdp_str)` / `rtc.sdp().answer()` | Rewrite offer/answer flow |
| **RTP serialization** | `webrtc_util::Marshal::marshal_to()` | Raw bytes in `MediaData` event; manual RTP header construction if GStreamer requires full packet | Update bridge |
| **Codec registration** | `MediaEngine::register_default_codecs()` | Codec configuration in `Rtc::builder()` with explicit MIME types | Update codec setup |
| **RTCP** | `RTCPeerConnection::write_rtcp()` | Output via `poll_output()` → `Output::Transmit` | Remove direct RTCP calls |
| **Tokio integration** | Library manages Tokio tasks internally | Caller wraps `poll_output()` in a `tokio::spawn` loop | Add integration shim |
| **H.265** | Registered with known packetizer bugs | Same known issues; do not use | No change needed |

### 2.3 What does NOT change

| Item | Notes |
|---|---|
| GStreamer appsrc bridge pattern | RTP bytes → `push_buffer()` path is unchanged; only the source of the bytes changes |
| gRPC signaling plane (RFC 0005/0014) | SDP strings are produced/consumed identically; library is agnostic |
| Codec negotiation intent | H.264 + VP9 + Opus remain the v2 codec matrix |
| ICE/DTLS/SRTP protocol semantics | Same protocol; different API shape |
| STUN/TURN server configuration | URLs still passed at construction; syntax may differ |
| tze_hud architecture | Transport layer only; scene model, compositor, leases unchanged |

### 2.4 Known open issues in v0.20 alpha

These must be resolved or verified-fixed before migration proceeds:

| Issue | Severity for tze_hud | Status |
|---|---|---|
| IPv6 ICE gather (#774) | Medium (cloud-relay Safari risk; tracked hud-0bqk8) | Open |
| Socket recv error handling (#777) | Medium (reliability) | Open |
| localhost STUN timeout (#778) | Low (development only) | Open |
| H.265 packetizer (#779) | Not applicable (H.265 not in v2 codec matrix) | Open (irrelevant) |
| TCP ICE (#781) | Medium (firewall traversal; TURN-TCP is separate mitigation) | Open |
| rrid RTX association (rtc PR #72) | High (simulcast; GATE-1) | Open |
| GCC bandwidth estimator (rtc PR #85) | High (adaptive simulcast; GATE-2) | Open |

---

## 3. The Three Signal Gates

The hud-g89zs NO-GO verdict identified three specific signals that must land before Phase 4b
can proceed with webrtc-rs v0.20. These are the GATE-1, GATE-2, and GATE-3 entries above.

### GATE-1: rtc PR #72 — rrid RTX association

**What it is**: Implements full `rrid` (Repaired RTP Stream ID, RFC 8852 §3) → RTX SSRC
association. Through v0.9.0, the rrid handler was a `TODO` stub in `endpoint.rs:450`.
Without this, simulcast with RTX repair streams cannot correctly demux repair packets to
their base stream.

**Why it blocks**: Phase 4 bidirectional AV uses simulcast (multi-layer video to cloud SFU).
RTX (retransmission) is mandatory for video quality over lossy links. Without rrid→RTX
association, repair packets are misrouted and simulcast degrades to a single layer.

**Current status**: PR #72 passes all tests; awaiting maintainer sign-off from `rainliu`.
Last rebase 2026-04-10. No announced merge timeline.

**Cascade**: When PR #72 merges to `rtc` master, it unblocks `rtc` v0.10.0 release, which
in turn unblocks the `webrtc` v0.20.0-alpha.2 wrapper cut (GATE-3).

### GATE-2: rtc PR #85 — GCC sender-side bandwidth estimator

**What it is**: Implements Google Congestion Control (GCC) as a sender-side interceptor,
consuming TWCC (Transport-Wide Congestion Control) feedback from the receiver to adjust
outbound bitrate. Also includes a receiver-side jitter buffer interceptor (PR #84,
prerequisite) and a rate controller.

**Why it blocks**: SFUs use REMB/transport-wide CC to signal bandwidth constraints to the
sender. Without GCC, the webrtc-rs sender ignores these signals and transmits at fixed
bitrate. For adaptive simulcast (sending h/m/l layers and allowing the SFU to select),
the sender must respond to congestion signals by dropping layers. This is the production
adaptive-quality mechanism.

**Current status**: PR #85 opens after PR #84 merges. Tests pass (154 interceptor tests,
87.7% patch coverage). One review concern: missing `min_bitrate_bps > max_bitrate_bps`
validation in the rate controller builder. Requires maintainer resolution.

**Without this gate**: tze_hud can still transmit simulcast layers, but at fixed bitrate.
The SFU will drop layers it doesn't want, but tze_hud's sender will not reduce bandwidth
proactively. This degrades quality under congestion and wastes relay bandwidth.

### GATE-3: webrtc v0.20.0-alpha.2 released (and subsequently stable)

**What it is**: The `webrtc` async wrapper crate (what tze_hud actually depends on) must
cut a release that incorporates GATE-1 and GATE-2 from the `rtc` sub-crate. The alpha.2
is the intermediate validation milestone; a stable v0.20.0 is required before tze_hud
can ship against it.

**Why alpha is insufficient**: Alpha releases carry API breakage risk between versions.
The hud-ora8.1.17 audit posture is to avoid pinning to alpha crates in shipping code.
An alpha would be acceptable for a local development spike but not for a production gate.

**Current status**: Latest alpha is v0.20.0-alpha.1 (2026-03-01). No alpha.2 announced.
The `webrtc` wrapper is blocked on the `rtc` backlog (19 open PRs as of audit date).

---

## 4. Migration Steps

These steps are ordered and incremental. Each step should be a separate commit or PR to
isolate compile errors and regressions.

### Pre-migration (before any code change)

1. **Confirm all three gates are satisfied** — check GATE-1, GATE-2, GATE-3 against current
   upstream state. Do not begin code migration if any gate is OPEN.

2. **Run the simulcast interop plan** (hud-fpq51) against v0.20 stable to verify Phase 4b
   gate criteria pass: H.264 + VP9 simulcast SDP negotiation with Chrome and Firefox, three
   RID layers (h/m/l), rrid RTX demux, and GCC layer response.

3. **Audit open issues in v0.20 stable** — verify #774, #777, #778, #781 are resolved or
   have documented mitigations. File any unresolved items as Phase 4 pre-conditions.

### Step 1: Cargo.toml bump

```toml
# In the crate that contains the WebRTC transport layer (e.g., tze_hud_media or tze_hud_webrtc)
[dependencies]
# OLD (Phase 1):
webrtc = "0.17"

# NEW (Phase 4):
webrtc = "0.20"
```

Run `cargo build` to surface all compile errors. **Do not fix errors yet** — capture the
full error list as the migration inventory. Commit `Cargo.toml` alone on a branch so the
diff is reviewable.

### Step 2: Fix peer connection construction

Replace the `APIBuilder` + `MediaEngine` factory with the v0.20 `Rtc::builder()` pattern.
Codec registration changes from `MediaEngine::register_default_codecs()` to explicit
builder configuration.

```rust
// v0.17.x (reference — remove)
let mut media_engine = MediaEngine::default();
media_engine.register_default_codecs()?;
let api = APIBuilder::new().with_media_engine(media_engine).build();
let peer_connection = api.new_peer_connection(config).await?;

// v0.20.x (replacement — sketch; consult actual v0.20 docs)
let rtc = Rtc::builder()
    .set_ice_servers(vec!["stun:stun.l.google.com:19302"])
    .build();
```

Commit: `refactor(media): replace APIBuilder with Rtc::builder [hud-j1msi]`

### Step 3: Replace callback event handlers with poll loop

This is the largest change. All `on_track`, `on_ice_candidate`, `on_ice_connection_state_change`,
`on_peer_connection_state_change` closures are removed. Replace with a `tokio::spawn` loop
that calls `rtc.poll_output()` and dispatches on `Event` variants.

```rust
// v0.17.x pattern (reference — remove)
peer_connection.on_track(Box::new(|track, _, _| {
    tokio::spawn(async move { /* rtp bridge */ });
    Box::pin(async {})
}));
peer_connection.on_ice_candidate(Box::new(|candidate| {
    // send candidate over gRPC
    Box::pin(async {})
}));

// v0.20.x pattern (replacement — sketch)
tokio::spawn(async move {
    loop {
        tokio::select! {
            _ = timer.tick() => {
                rtc.handle_input(Input::Timeout(Instant::now())).ok();
            }
            Some((data, addr)) = socket.recv() => {
                rtc.handle_input(Input::Receive(Instant::now(), Receive {
                    source: addr, destination: local_addr, contents: data.into()
                })).ok();
            }
        }
        while let Ok(Some(output)) = rtc.poll_output() {
            match output {
                Output::Transmit(send) => { socket.send_to(&send.contents, send.destination).await.ok(); }
                Output::Event(Event::IceCandidate(c)) => { /* forward over gRPC */ }
                Output::Event(Event::MediaData(data)) => { rtp_bridge_tx.try_send(data).ok(); }
                Output::Timeout(t) => { timer.reset_at(t); }
                _ => {}
            }
        }
    }
});
```

Commit: `refactor(media): replace webrtc-rs v0.17 callbacks with v0.20 poll loop [hud-j1msi]`

### Step 4: Update RTP bridge

The GStreamer appsrc bridge receives RTP data differently in v0.20:

- v0.17.x: `track_remote.read(&mut buf).await` → `RtpPacket` → `marshal_to(&mut buf)` → `push_buffer`
- v0.20.x: `Event::MediaData(data)` → `data.data` (raw RTP payload bytes) → reconstruct RTP header → `push_buffer`

The GStreamer side (appsrc caps, `do-timestamp=false`, `is-live=true`) is unchanged.

```rust
// v0.20.x bridge (sketch — adapt to actual MediaData fields)
Output::Event(Event::MediaData(data)) => {
    // data.payload is the RTP payload; wrap in RTP header for GStreamer
    let rtp_packet = build_rtp_packet(&data);  // helper: assemble header + payload
    let n = rtp_packet.marshal_to(&mut buf)?;
    let gst_buf = gstreamer::Buffer::from_slice(buf[..n].to_vec());
    appsrc.push_buffer(gst_buf)?;
}
```

Consult actual v0.20 `MediaData` struct fields when v0.20 stable is available. The key
invariant: do NOT re-timestamp; preserve RTP timestamps from the packet for lip-sync.

Commit: `refactor(media): update RTP→appsrc bridge for v0.20 MediaData events [hud-j1msi]`

### Step 5: Update SDP offer/answer flow

The gRPC signaling plane (RFC 0005/0014) exchanges SDP strings. In v0.17.x, SDP goes
through `RTCSessionDescription`. In v0.20, use the `rtc.sdp()` accessor. The SDP strings
themselves are protocol-identical; only the API to set/get them changes.

Commit: `refactor(media): update SDP offer/answer API for v0.20 [hud-j1msi]`

### Step 6: Wire simulcast encoding (Phase 4 specific)

Add outbound simulcast tracks with RID encoding parameters. This is new Phase 4 code,
not a refactor of Phase 1 ingest. The v0.20 API for simulcast encoding setup uses the
GCC interceptor (GATE-2) and explicit RID layer configuration.

Commit: `feat(media): add simulcast outbound encoding with RID layers [hud-j1msi]`

### Step 7: Run quality gates

```bash
# Compile check
cargo build

# Lint
cargo clippy -- -D warnings

# Tests
cargo test

# Integration: simulcast interop (hud-fpq51 test plan)
# Run Chrome + Firefox browser interop tests against the updated stack
```

---

## 5. Rollback Plan

If v0.20 introduces a blocking regression, the rollback path back to v0.17 must be
executable within one day.

### Fast rollback procedure

1. **Revert the Cargo.toml version bump**:
   ```bash
   # In the relevant Cargo.toml
   webrtc = "0.17"   # restore
   cargo build       # verify it compiles
   ```

2. **Revert the API refactor commits** — the poll-loop refactor (Step 3) is the largest
   change. If a feature branch was used, revert to the Phase 1 commit on that branch.
   If the migration was merged to main, create a revert PR.

3. **Verify the Phase 1 test suite passes** before closing the rollback.

### Rollback triggers

Invoke rollback immediately (without waiting for upstream fixes) if:
- Glass-to-glass p99 latency exceeds 400 ms under D18 reference streams
- Decode-drop rate exceeds 0.5% under D18 reference streams
- Simulcast negotiation fails with Chrome or Firefox in the hud-fpq51 test harness
- Any critical-tier issue (compositor hang, session state-machine violation) introduced by
  the migration

### Fallback to str0m

If rollback to v0.17 is triggered AND v0.17 cannot satisfy Phase 4 bidirectional AV
requirements (e.g., outbound track API insufficient), invoke the str0m fallback:

- **Fallback library**: `str0m` v0.18.0 (hud-1ee3a verdict: recommended fallback)
- **Migration scope**: Transport I/O loop rewrite; GStreamer bridge adapts for `MediaData`
  events; gRPC signaling plane unchanged
- **Estimated effort**: 2–4 person-weeks (per hud-1ee3a §1.10)
- **Pre-conditions for str0m**: TURN-over-TCP validation (hud-kjody), simulcast browser
  interop test (hud-fpq51 str0m variant), WHIP signaling compatibility with LiveKit

---

## 6. Risks and Mitigations

### R1: Simulcast breakage under v0.20

**Risk**: v0.20 simulcast interceptors are incomplete at migration time (rrid stub, no
layer switching, no GCC). Phase 4 bidirectional AV requires adaptive simulcast to the
cloud SFU (C15 vendor).

**Likelihood**: High if gates are not checked. Low if all three gates are satisfied before
migration begins.

**Mitigation**: The three-gate check is mandatory before any migration code change. The
hud-fpq51 simulcast interop plan must pass with Chrome and Firefox before the migration
PR is merged. If simulcast fails post-migration, roll back immediately (§5).

**Residual risk**: Browser interop evidence for webrtc-rs simulcast does not exist as of
2026-04-19 for any version. Even with all three gates satisfied, the first browser interop
run may reveal protocol-level issues. Reserve a Phase 4 sprint for browser interop
debugging.

### R2: Bandwidth estimation changes (GCC)

**Risk**: The GCC interceptor (GATE-2) is new code. Its behavior in production (bitrate
ramp-up curves, congestion reaction latency, min/max validation) may differ from
expectations. Misconfigured GCC could cause excessive layer drops (degraded video) or
bandwidth overuse (congestion in shared networks).

**Likelihood**: Medium — new interceptor code with a known validation gap in the builder.

**Mitigation**: Test GCC under controlled network conditions (netem/tc qdisc) before
cloud-relay deployment. Verify builder validation fix (min/max bitrate ordering) is in the
merged PR. Monitor bandwidth utilization in the Phase 4 QA environment against D18 budgets.
Add a circuit-breaker in the tze_hud media worker: if decode-drop rate exceeds 0.5%,
emit a diagnostic event (do not auto-degrade silently).

### R3: Trickle ICE semantics change

**Risk**: The sans-IO event model in v0.20 changes how ICE candidates are fed. If
tze_hud's gRPC signaling plane (RFC 0005/0014) sends candidates at a different timing than
the poll loop expects, connection establishment could regress.

**Likelihood**: Low — trickle ICE protocol is unchanged; only the API wrapper differs.

**Mitigation**: During integration testing (Step 7), verify ICE establishment latency is
within D18 TTFF budget (≤500 ms) for both LAN and STUN-only paths. Add a telemetry
probe: emit an ICE establishment event with elapsed time to the structured trace log so
regressions are visible in CI.

### R4: Phase 4 timeline slip from upstream dependency

**Risk**: webrtc-rs maintainers do not merge GATE-1/GATE-2 or cut a stable v0.20.0 before
Phase 4 kickoff. Migration is blocked indefinitely.

**Likelihood**: Medium — 19 open PRs queued as of audit date; no announced stable timeline.

**Mitigation**: Monthly monitoring check on GATE-1, GATE-2, GATE-3 status (calendar date
for next check: 2026-05-19). If gates remain unsatisfied six weeks before Phase 4 kickoff,
invoke str0m fallback planning (hud-1ee3a) — do not wait for the kickoff date to make the
decision. The fallback decision must be made early enough to absorb the 2–4 week migration
effort.

### R5: API instability between alpha.2 and stable

**Risk**: The v0.20 API changes between alpha.2 and stable, requiring rework after
migration is complete.

**Likelihood**: Low if migration is deferred to stable. High if migration begins on alpha.

**Mitigation**: Do not migrate on alpha. The migration plan gates on stable v0.20.0.
Track release notes between alpha.2 and stable for any breaking API changes. If breaking
changes affect the core `Rtc` / `poll_output()` / `Event` surface, update the refactor
PR before merging.

### R6: IPv6 ICE gather regression (webrtc-rs #774)

**Risk**: webrtc-rs v0.20 has a known open bug (#774) in IPv6 ICE candidate gathering. On
Safari over cloud relay (Cloudflare NAT64 path, gate G5a in hud-0bqk8), IPv6 gather may
fail or time out, causing session establishment failure for IPv6-only Safari clients. This
is distinct from TURN-over-TCP coverage (hud-kjody) and is not mitigated by TURN alone.

**Likelihood**: Medium — the issue is open as of 2026-04-19 with no committed fix timeline.
Cloud-relay deployment (C15 gate) is the primary exposure path.

**Mitigation**: Before Phase 4b cloud-relay deployment, verify webrtc-rs #774 is resolved
or has a documented mitigation in the v0.20 stable release notes. If unresolved at that
point: (a) restrict ICE candidate gathering to IPv4 + TURN as a short-term workaround, or
(b) invoke the str0m fallback path, which does not share the same gather logic. Track via
hud-0bqk8 IPv6 ICE audit. Do not block Phase 4a (LAN / STUN-only) on this risk; it is
specific to the NAT64 cloud-relay path.

---

## 7. Effort Estimate

| Phase | Scope | Estimate |
|---|---|---|
| Pre-migration validation (gate check + interop) | Gate verification, browser interop run per hud-fpq51 | 3–5 days |
| Step 1: Cargo.toml bump + compile audit | Map all compile errors | 0.5 day |
| Step 2: Peer connection construction | `APIBuilder` → `Rtc::builder()` | 0.5 day |
| Step 3: Callback → poll loop refactor | Largest single change; all event handlers | 2–3 days |
| Step 4: RTP bridge update | `read()` → `Event::MediaData` bridge | 1 day |
| Step 5: SDP offer/answer update | API call site changes | 0.5 day |
| Step 6: Simulcast outbound (Phase 4 new feature) | RID layers + GCC config | 2–3 days |
| Step 7: Quality gates + regression testing | CI, browser interop, D18 latency | 2–3 days |
| Buffer for unexpected regressions | Diagnosing alpha-era API issues | 2 days |
| **Total** | | **11–16 days (2.2–3.2 person-weeks)** |

If the str0m fallback is invoked instead, add 2–4 weeks for the transport I/O loop rewrite
(hud-1ee3a §1.10 estimate). The GStreamer bridge and gRPC signaling plane efforts are the
same in both paths.

The estimate assumes Phase 1 bounded-ingress implementation is complete and the GStreamer
appsrc bridge is already written and tested. If Phase 1 is incomplete, the migration effort
is higher because there is no baseline v0.17 integration to refactor from.

---

## 8. Cross-Reference Index

| Issue | Title | Relationship |
|---|---|---|
| `hud-ora8.1.17` | webrtc-rs library audit (PR #523) | Origin audit: ADOPT-WITH-CAVEATS; set v0.17 pin + phase 4 migration mandate |
| `hud-g89zs` | v0.20 simulcast readiness audit (PR #543) | Produced the NO-GO verdict; identified the three gate signals |
| `hud-1ee3a` | SFU fallback audit (PR #544) | str0m recommended fallback; effort and integration details |
| `hud-kjody` | str0m TURN-over-TCP validation (PR #547) | Pre-condition for str0m fallback: TCP TURN coverage |
| `hud-rw3je` | TURN client integration design | Applies to both webrtc-rs and str0m paths; hud-rw3je is transport-agnostic |
| `hud-fpq51` | Phase 4 simulcast interop test harness | Browser interop test plan; must pass before migration or fallback merge |
| `hud-0bqk8` | IPv6 ICE audit | Phase 4b cloud-relay Safari ICE gather risk (webrtc-rs #774) |

---

## 9. Monitoring Schedule

| Date | Action |
|---|---|
| 2026-05-19 | Check GATE-1 (rtc PR #72), GATE-2 (rtc PR #85), GATE-3 (webrtc alpha.2) |
| 2026-06-19 | Second check; if no progress on GATE-2, escalate str0m fallback decision |
| Phase 4 kickoff (date TBD) | Final gate evaluation; commit to webrtc-rs v0.20 or str0m |
| Phase 4b kickoff (C15 gate) | C15 SFU vendor selection; orthogonal to transport library decision |

---

## Sources

- webrtc-rs library audit: `docs/audits/webrtc-rs-audit.md` (hud-ora8.1.17, PR #523)
- v0.20 simulcast readiness report: `docs/reports/webrtc-rs-v0.20-simulcast-readiness.md` (hud-g89zs, PR #543)
- SFU fallback audit: `docs/audits/webrtc-sfu-fallback-audit.md` (hud-1ee3a, PR #544)
- PR #85 monitoring report: `docs/reports/rtc-pr-85-monitor-2026-04-19.md` (hud-fzeb9)
- TURN client integration design: `docs/design/turn-client-integration.md` (hud-rw3je)
- webrtc-rs GitHub: https://github.com/webrtc-rs/webrtc
- webrtc-rs/rtc GitHub: https://github.com/webrtc-rs/rtc
- rtc PR #72 (rrid RTX): https://github.com/webrtc-rs/rtc/pull/72
- rtc PR #85 (GCC): https://github.com/webrtc-rs/rtc/pull/85
- v0.20.0-alpha.1 announcement: https://webrtc.rs/blog/2026/03/01/webrtc-v0.20.0-alpha.1-async-webrtc-on-sansio.html
- str0m GitHub: https://github.com/algesten/str0m
- v2 signoff packet: `openspec/changes/v2-embodied-media-presence/signoff-packet.md`
