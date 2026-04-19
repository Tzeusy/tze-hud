# str0m TURN-over-TCP Validation Spike

**Issued for**: `hud-kjody`
**Date**: 2026-04-19
**Auditor**: agent worker (claude-sonnet-4-6)
**Parent context**: hud-1ee3a (SFU fallback audit, PR #544 open) — phase 4b fallback prep
**Cross-references**: hud-1ee3a (SFU fallback audit), hud-g89zs (webrtc-rs v0.20 spike, PR #543 merged), hud-fpq51 (Phase 4 interop plan, PR #538 merged)

---

## Verdict

**CONDITIONAL-GO**

str0m can be used as the phase 4b webrtc-rs fallback from a corporate-firewall-traversal
perspective, **subject to one blocking integration requirement**: tze_hud must supply an
external TURN client that handles TURN-over-TCP protocol exchanges. str0m's sans-IO design
explicitly externalizes this concern — TURN socket acquisition is the caller's
responsibility, not the library's.

The gap is real but bridgeable. It does not require waiting for str0m to change; it
requires tze_hud to implement a thin TURN client integration layer (e.g., using the
`turn-client` crate or `webrtc-turn`) alongside str0m. This is consistent with str0m's
design philosophy and is not a fundamental architectural incompatibility.

**Summary table:**

| Question | Answer |
|---|---|
| Does str0m support TURN-over-TCP (RFC 6062) natively? | No — TURN is explicitly out of scope in str0m's sans-IO design |
| Does str0m support TURN-over-TLS (RFC 5766) natively? | No — same reason |
| Does str0m support ICE TCP candidates? | Yes — passive/active tcptype ICE candidate handling added in v0.15.0 (PR #797) |
| Does str0m support relayed ICE candidates (relay type)? | Yes — `Candidate::relayed()` API accepts externally-obtained relay addresses |
| Can external TURN clients feed relay addresses into str0m? | Yes — this is the intended integration model per the sans-IO design |
| Is TURN-over-TCP achievable with str0m + external TURN client? | Yes — CONDITIONAL-GO |
| How does this compare to webrtc-rs v0.17 TURN-over-TCP? | webrtc-rs v0.17 also lacks TURN-over-TCP; it is a gap in both stacks |

---

## 1. str0m TURN Architecture

### 1.1 Design philosophy: TURN is explicitly out of scope

str0m's README and internal ICE documentation state this clearly:

> "TURN is a way of obtaining IP addresses that can be used as fallback in case direct
> connections fail. We consider TURN similar to enumerating local network interfaces —
> it's a way of obtaining sockets."
>
> "All discovered candidates, be they local (NIC) or remote sockets (TURN), are added to
> str0m and str0m will perform the task of ICE agent, forming 'candidate pairs' and figuring
> out the best connection while the actual task of sending the network traffic is left to
> the user."

This is an intentional architectural decision, not a deficiency. The feature comparison
table in str0m's README marks TURN as `❌` relative to libWebRTC, alongside NIC
enumeration and encode/decode — all deliberate sans-IO exclusions.

### 1.2 What str0m does provide

str0m handles everything from the relay address inward:

| Capability | Status | Details |
|---|---|---|
| ICE agent (RFC 8445) | Full | Forms candidate pairs, performs connectivity checks |
| Relayed candidate type | Full | `Candidate::relayed(addr, local_interface, proto)` constructor |
| Server-reflexive candidate type | Full | `Candidate::server_reflexive(addr, base, proto)` |
| tcptype ICE candidates | Full since v0.15.0 | `TcpType::Passive`, `TcpType::Active`, `TcpType::SimultaneousOpen` |
| Trickle ICE | Full | Relay addresses can be added incrementally via `add_local_candidate()` |
| ICE-BUNDLE (rtcp-mux-only) | Full | Single ICE component per session |
| DTLS 1.2 | Full | Runs over whatever socket the caller provides |
| SRTP / SRTCP | Full | AES-CM-128 |
| STUN messages | Partial | Parses STUN-encoded TURN messages in v0.8.0; full STUN binding checks |

### 1.3 What str0m does NOT provide

| Missing capability | Impact on TURN-over-TCP |
|---|---|
| TURN protocol client (RFC 8656 / RFC 5766) | Tze_hud must implement or integrate an external TURN client |
| TURN-over-TCP allocation (RFC 6062) | No built-in TCP TURN allocation; external TURN client must handle TCP TURN ALLOCATE/CONNECT exchanges |
| TURN-over-TLS | Same; TLS wrapping of the TURN TCP connection must be external |
| NIC enumeration | Caller enumerates local interfaces and passes socket addresses |

The ICE documentation in `docs/ice.md` (str0m repo) is explicit:

> "Address discovery, STUN and TURN out of scope. Trickle ice means we can add new socket
> addresses as and when we discover them. There is no meaningful difference between adding
> an address of a local NIC, discovering a reflexive address via STUN or creating a new
> tunnel via a TURN server. Therefore, all address discovery, such as enumerating local
> NICs, using a STUN or TURN server, are external concerns to this ICE implementation."

---

## 2. TURN-over-TCP (RFC 6062) Gap Analysis

### 2.1 What RFC 6062 requires

RFC 6062 extends the base TURN protocol (RFC 8656, formerly RFC 5766) to allow
TURN clients to allocate **TCP relay addresses** on a TURN server. The protocol exchange is:

1. Client establishes a TCP (or TLS-over-TCP) connection to the TURN server on port 443
   or 3478.
2. Client sends an ALLOCATE request with `REQUESTED-TRANSPORT: TCP` attribute.
3. Server allocates a TCP port on its relayed address range and responds with the
   relayed address.
4. Client adds the relayed address as an ICE relay candidate and signals it via SDP/ICE.
5. TURN server acts as a TCP relay between the client and the remote peer.

str0m handles steps 4 and 5 (ICE candidate management and data forwarding through the
relayed socket). Steps 1–3 are outside str0m's scope.

### 2.2 Integration path: external TURN client + str0m

The intended integration pattern under str0m's sans-IO model:

```rust
// Step 1: external TURN client obtains a TCP relay allocation
// (e.g., using the `webrtc-turn` crate or a custom STUN/TURN client)
let turn_config = TurnClientConfig {
    server: "turns:turn.example.com:443?transport=tcp".parse()?,
    credentials: TurnCredentials { username, password },
    local_socket: tcp_socket.local_addr()?,
};
let relay_addr = turn_client::allocate_tcp(&turn_config).await?;

// Step 2: create a relay candidate from the TURN-allocated address
let local_interface = tcp_socket.local_addr()?;
let relay_candidate = Candidate::relayed(relay_addr, local_interface, "tcp")?;

// Step 3: add to str0m's ICE agent
rtc.add_local_candidate(relay_candidate);

// Step 4: when str0m emits Output::Transmit to the relay address,
//         forward via the TURN client's TCP connection (not a raw socket)
match rtc.poll_output()? {
    Output::Transmit(send) if send.destination == relay_addr => {
        turn_client.send_data(send.contents, send.destination).await?;
    }
    Output::Transmit(send) => {
        udp_socket.send_to(send.contents, send.destination)?;
    }
    _ => {}
}
```

The TURN client must handle:
- TCP connection lifecycle to the TURN server
- ALLOCATE/REFRESH/CONNECT exchanges
- TURN DATA indication / ChannelData framing
- Relaying outbound packets from str0m via `turn_client.send_data()`
- Injecting incoming TURN-wrapped packets into str0m via `rtc.handle_input(Input::Receive(...))`

### 2.3 Open GitHub issues

**str0m issue #723** (open, Nov 2025): "Add STUN/TURN server support for NIC enumeration"
— proposes incorporating TURN client support via an optional crate feature. References
`rustun` and `librice` as candidate Rust TURN client libraries. No maintainer response
recorded. This issue confirms TURN client integration is a known user request that has
not been merged.

No dedicated TURN-over-TCP issue exists in str0m's tracker. The omission is consistent
with the design philosophy: the library deliberately does not intend to include TURN
client logic.

---

## 3. Comparison with webrtc-rs TURN-over-TCP

### 3.1 webrtc-rs v0.17.x TURN status

The webrtc-rs audit (`docs/audits/webrtc-rs-audit.md`) reports that webrtc-rs v0.17.x
includes a `webrtc-turn` crate with TURN client support (RFC 8656 full). However,
**TURN-over-TCP (RFC 6062) is absent in webrtc-rs as well**.

webrtc-rs issue #539 ("TCP support for TURN (relay)", closed 2026-01-31 as `not_planned`)
documents this gap:

> "TCP relay support is sometimes the only thing that can bypass certain firewalls. When
> I configured coturn to be a TCP-only relay, [a] candidate is not acquired or not used,
> suggesting the absence of this feature."

The issue was closed as not planned, meaning webrtc-rs has no intention of implementing
TURN-over-TCP in the current codebase. Additionally:

- webrtc-rs v0.20 has TCP ICE support open as issue #781 ("TCP ICE support") and PR #789
  ("feat: TCP ICE candidates (RFC 6544) in async wrapper", open) — these cover ICE-TCP
  transport, not the TURN-over-TCP allocation protocol (RFC 6062). These are distinct.
- The `webrtc-turn` crate in v0.17.x handles UDP TURN allocations only.

### 3.2 Side-by-side gap table

| Capability | webrtc-rs v0.17.x | str0m v0.18.0 |
|---|---|---|
| TURN client (UDP, RFC 8656) | Yes — built-in `webrtc-turn` crate | No — external concern |
| TURN-over-TCP (RFC 6062) | No — issue #539 closed not_planned | No — out of scope |
| TURN-over-TLS (RFC 5766 / 8656) | No — no evidence of TLS TURN | No — out of scope |
| ICE-TCP candidates (RFC 6544) | Partial — v0.20 alpha PR #789 open | Yes since v0.15.0 (#797) |
| Relayed ICE candidate type | Yes — via `webrtc-turn` integration | Yes — `Candidate::relayed()` |
| External TURN integration path | No — internal coupling to `webrtc-turn` | Yes — by design (sans-IO) |

**Key finding**: str0m and webrtc-rs are at parity on TURN-over-TCP: **neither implements
it natively**. The difference is that webrtc-rs bundles a UDP-only TURN client internally
(making the UDP case easier), while str0m requires an external TURN client for both UDP
and TCP cases. For TURN-over-TCP specifically, both stacks require external integration
work of roughly equal scope.

---

## 4. Corporate Firewall Traversal Parity Assessment

### 4.1 What corporate firewall traversal requires

In environments where all outbound UDP is blocked (deep-packet-inspection corporate
proxies, strict enterprise firewalls), WebRTC connectivity depends on:

1. TURN-over-TCP: relay via a TURN server on TCP/443 (looks like HTTPS to firewalls).
2. TURN-over-TLS: relay via a TURN server with TLS on TCP/443 (indistinguishable from
   TLS traffic to most firewalls — the highest-fidelity fallback).
3. ICE-TCP candidates (RFC 6544): direct TCP ICE without a relay when the peer is
   accessible via TCP.

The priority order is: UDP direct > ICE-TCP > TURN-UDP > TURN-TCP > TURN-TLS.

### 4.2 Traversal capability parity

| Traversal mechanism | webrtc-rs v0.17.x | str0m v0.18.0 | Integration work required |
|---|---|---|---|
| UDP host candidates | Yes | Yes | None |
| STUN server-reflexive (UDP) | Yes | External STUN client | Trivial; STUN binding request is simple |
| TURN relay (UDP) | Yes — built-in | External TURN client | Moderate — integrate a Rust TURN client crate |
| ICE-TCP (direct TCP, RFC 6544) | Open PR #789 in v0.20 alpha | Yes since v0.15.0 | None for str0m |
| TURN-over-TCP (RFC 6062) | No (issue #539 closed not planned) | External TURN client | Moderate — same as UDP TURN, plus TCP transport layer |
| TURN-over-TLS | No | External TURN client (TLS) | Moderate — TLS adds complexity; use `rustls` or `openssl` |

**Assessment**: For the most restrictive corporate firewall scenario (TURN-over-TLS on
TCP/443 as last fallback), both stacks require external integration. str0m's sans-IO
architecture is actually better suited to this integration because the caller already
owns the network sockets and can wrap them in TLS before feeding packets into str0m.

With webrtc-rs, integrating TURN-over-TCP would require forking or extending `webrtc-turn`
because its internal connection management assumes UDP. With str0m, the integration is
external by design and does not require modifying library internals.

### 4.3 ICE-TCP advantage (str0m specific)

str0m v0.15.0 landed full TCP ICE candidate support (tcptype passive/active/simultaneous-
open per RFC 6544) via PR #797. This provides a non-relay TCP path for environments where
the peer is reachable via TCP directly. webrtc-rs v0.17.x lacks this; it is only partially
present in the v0.20 alpha (PR #789, open). This gives str0m a **traversal advantage**
over webrtc-rs for the ICE-TCP case.

---

## 5. External TURN Client Options (Rust)

For tze_hud's TURN-over-TCP integration with str0m, the following Rust TURN client
options are available:

| Crate | Type | TURN-TCP support | Notes |
|---|---|---|---|
| `webrtc-turn` | Library | UDP only (per issue #539) | Part of webrtc-rs workspace; usable standalone |
| `rustun` | Library | Partial | Lower-level STUN library; TURN support unclear |
| `librice` (Rust bindings) | Library | Investigate | Referenced in str0m issue #723; unverified TCP TURN support |
| Custom implementation | Hand-rolled | Full | RFC 6062 is 13 pages; scope is well-bounded |

**Recommendation**: A minimal custom TURN-over-TCP client that implements only the ALLOCATE
and REFRESH messages (the two required for a relay candidate) is the most controllable
path. RFC 6062 is a small extension. The implementation can reuse any STUN serializer
(`webrtc-stun` or str0m's internal STUN parser).

Alternatively, if tze_hud uses LiveKit Cloud as the C15 SFU vendor (per hud-1ee3a
verdict), LiveKit's managed TURN infrastructure provides TURN-over-TCP and TURN-over-TLS
out of the box — removing this integration requirement entirely for the cloud-relay path.

---

## 6. Implementation Risk Assessment

| Risk | Severity | Likelihood | Mitigation |
|---|---|---|---|
| External TURN client integration adds 2–4 weeks to phase 4b | Medium | High | Scope explicitly at phase 4b kickoff; do not defer discovery |
| Chosen TURN client crate lacks TCP TURN support | Medium | Medium | Validate TURN client crate before commit; fallback to custom impl |
| TLS TURN adds runtime complexity (cert pinning, TLS handshake) | Low | Medium | Use `rustls` (already in tze_hud dependency graph) for TLS wrapping |
| str0m issue #723 (built-in TURN) merged before phase 4b | Low | Low | If it lands, it's additive; integration effort drops |
| TURN server infrastructure gaps (corporate TURN-TCP config) | Low | Low | coturn supports RFC 6062 out of the box; this is a deployment concern |

**Overall risk**: CONDITIONAL-GO at medium confidence. The integration work is bounded,
understood, and external to str0m's core. The risk is scheduling (phase 4b must allocate
time for TURN client integration) rather than feasibility.

---

## 7. Verdict Rationale

**CONDITIONAL-GO** with two conditions:

1. **Phase 4b kickoff must allocate a TURN client integration bead.** The scope is
   moderate (est. 2–3 days for UDP TURN; 1–2 additional days for TCP/TLS wrapping), not
   a one-liner. It must not be discovered as a gap during implementation.

2. **If LiveKit Cloud is selected as the C15 SFU vendor** (per hud-1ee3a
   recommendation), the managed TURN infrastructure eliminates the TCP TURN gap for the
   cloud-relay path. Verify LiveKit Cloud's TURN-TCP/TLS support at phase 4b kickoff.

str0m is **not blocked** by this gap relative to webrtc-rs. Both stacks lack native
TURN-over-TCP. str0m is architecturally better positioned to integrate external TURN
clients cleanly. The CONDITIONAL-GO verdict applies the same condition that would apply
to webrtc-rs — it is not str0m-specific.

---

## 8. Sources

- str0m repository: https://github.com/algesten/str0m
- str0m README (NIC enumeration and TURN section): https://github.com/algesten/str0m/blob/main/README.md
- str0m ICE documentation: https://github.com/algesten/str0m/blob/main/docs/ice.md
- str0m CHANGELOG: https://github.com/algesten/str0m/blob/main/CHANGELOG.md
- str0m issue #723 (STUN/TURN server support): https://github.com/algesten/str0m/issues/723
- str0m PR #797 (tcptype ICE candidates, v0.15.0): https://github.com/algesten/str0m/issues/797
- str0m ICE candidate source (`crates/is/src/candidate.rs`): https://github.com/algesten/str0m/blob/main/crates/is/src/candidate.rs
- str0m ICE agent source (`crates/is/src/agent.rs`): https://github.com/algesten/str0m/blob/main/crates/is/src/agent.rs
- webrtc-rs issue #539 (TCP support for TURN, closed not_planned 2026-01-31): https://github.com/webrtc-rs/webrtc/issues/539
- webrtc-rs issue #781 (TCP ICE support, open): https://github.com/webrtc-rs/webrtc/issues/781
- webrtc-rs PR #789 (feat: TCP ICE candidates, open): https://github.com/webrtc-rs/webrtc/issues/789
- RFC 6062 (TURN extensions for TCP allocations): https://www.rfc-editor.org/rfc/rfc6062.html
- RFC 8656 (TURN, obsoletes RFC 5766): https://datatracker.ietf.org/doc/html/rfc8656
- RFC 6544 (ICE-TCP): https://datatracker.ietf.org/doc/html/rfc6544
- docs/audits/webrtc-rs-audit.md (hud-ora8.1.17 — predecessor audit)
- docs/audits/webrtc-sfu-fallback-audit.md (hud-1ee3a — SFU fallback audit, PR #544 open)
- hud-g89zs verdict: webrtc-rs v0.20 simulcast NO-GO for today; re-evaluate at phase 4 kickoff
- hud-fpq51: Phase 4 simulcast interop plan (PR #538 merged)
