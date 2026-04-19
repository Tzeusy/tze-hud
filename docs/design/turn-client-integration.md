# TURN Client Integration Design

**Bead**: hud-rw3je  
**Date**: 2026-04-19  
**Author**: agent worker (claude-sonnet-4-6)  
**Status**: READY FOR IMPLEMENTATION  
**Cross-references**: hud-kjody (str0m TURN validation, PR #547), hud-1ee3a (SFU fallback audit, PR #544), hud-s2j0l (SFU adapter seam, PR #552), hud-amf17 (RFC 0018 WHIP signaling, PR #548), hud-0bqk8 (IPv6 ICE audit, PR #551)

---

## 1. Why tze_hud Needs a TURN Client

### 1.1 The Corporate Firewall Traversal Problem

WebRTC connectivity in corporate networks frequently fails because:

- Outbound UDP is blocked at the perimeter (deep-packet-inspection proxies, strict enterprise firewalls)
- Only TCP/443 and TCP/80 egress is permitted
- HTTPS inspection may break TLS-fingerprint-based protocols but passes plain TCP/443 traffic

In these environments, the ICE candidate priority order degrades to:
1. UDP host/srflx — **blocked**
2. ICE-TCP direct — possible if peer is directly reachable, often blocked
3. TURN-UDP relay — **blocked** (UDP is blocked at the perimeter)
4. TURN-TCP relay — **first viable fallback** (TCP/443 looks like HTTPS)
5. TURN-TLS relay — **most reliable fallback** (TLS/443 is indistinguishable from HTTPS traffic)

Without TURN-TCP or TURN-TLS, tze_hud sessions fail to establish in these environments.

### 1.2 The str0m + webrtc-rs Gap

The `docs/reports/str0m-turn-over-tcp-validation.md` audit (hud-kjody, CONDITIONAL-GO verdict) established that:

- **str0m does not implement TURN natively.** TURN socket acquisition is explicitly an external caller concern under str0m's sans-IO design philosophy. str0m's ICE documentation states: "Address discovery, STUN and TURN out of scope."
- **webrtc-rs v0.17 also lacks TURN-over-TCP.** Issue #539 was closed `not_planned`. The `webrtc-turn` crate supports UDP TURN only.
- **This gap is symmetric.** Neither stack is disadvantaged relative to the other for TURN-over-TCP. The difference is that webrtc-rs bundles a UDP-only TURN client; str0m requires an external client for all TURN variants.

str0m's sans-IO architecture is actually better suited for external TURN integration: the caller already owns all network sockets and can wrap any socket in TLS before passing packets to str0m. With webrtc-rs, TURN-over-TCP would require forking or extending the internal `webrtc-turn` crate.

### 1.3 What str0m Provides (the integration boundary)

str0m handles everything from the relay address inward:

| Capability | Status |
|---|---|
| ICE agent (RFC 8445) full | Full |
| `Candidate::relayed(addr, base, proto)` | Full |
| tcptype ICE candidates (RFC 6544) | Full since v0.15.0 |
| Trickle ICE — `add_local_candidate()` | Full |
| STUN binding checks | Full |
| DTLS 1.2 over caller-provided socket | Full |

str0m's contract: **tze_hud supplies relay addresses; str0m forms ICE candidate pairs and manages the session.** The TURN allocation protocol (RFC 8656 / RFC 6062) lives entirely in tze_hud's caller layer.

---

## 2. Library Selection

### 2.1 Candidate Survey

| Crate | Version | TURN-UDP | TURN-TCP | Maintained | Notes |
|---|---|---|---|---|---|
| `webrtc-turn` (webrtc-rs workspace) | 0.10.x | Yes | No | Moderate | Part of webrtc-rs; usable standalone. Issue #539 closed not_planned for TCP. |
| `rustun` | 0.5.x | Partial | Unclear | Low | Low-level STUN framing only; no TURN state machine. |
| `turn-rs` | 0.8.x | Yes | Partial | Active (2025) | Server-focused crate; client module present but undertested. |
| `librice` (Rust) | 0.3.x | Investigative | Unclear | Low | Referenced in str0m issue #723; ICE-focused, TURN support unverified. |
| `stun-rs` | 0.1.x | No | No | Low | STUN message codec only; no TURN state machine. |
| Custom minimal impl | N/A | Full | Full | N/A | RFC 6062 is 13 pages; RFC 8656 §4-8 is the allocate/refresh/data path. |

### 2.2 Recommendation: Custom Minimal TURN Client

**Pick: custom `tze_hud_turn` crate (new crate in this workspace) using `webrtc-stun` for message framing.**

Rationale:

1. **Scope control.** tze_hud needs only three TURN operations: ALLOCATE, REFRESH, and SEND/DATA indication for packet relay. No crate provides this subset cleanly for both UDP and TCP transports with Tokio integration.

2. **RFC 6062 is small.** The TCP extension adds CONNECT + ConnectionBind messages on top of the base ALLOCATE protocol. Total implementation scope: ~600-900 lines of Rust including tests.

3. **Avoid coupling to webrtc-rs internals.** `webrtc-turn` is tightly coupled to webrtc-rs's async I/O model; importing it while tze_hud uses str0m creates a dual-dependency that complicates upgrades and cargo feature trees.

4. **TLS integration is cleaner.** A custom client can use `tokio-rustls` (rustls is already in the dependency graph) and pin server certificates cleanly, without fighting another crate's TLS model.

5. **Minimal maintenance burden.** TURN is a stable protocol (RFC 8656 obsoletes RFC 5766 from 2008). The spec is frozen; the implementation does not need to track upstream development.

**Build-vs-buy summary:**

| | `webrtc-turn` | `turn-rs` | Custom |
|---|---|---|---|
| TURN-TCP (RFC 6062) | No | Partial | Yes |
| TURN-TLS | No | Unclear | Yes |
| Tokio integration | Yes | Yes | Yes (custom) |
| Coupling risk | High (webrtc-rs) | Low | None |
| Implementation cost | ~1 day integration | ~2 days integration+verify | ~2-3 days |

The cost delta is one day; the control benefit justifies it.

**If tze_hud selects LiveKit Cloud as the C15 SFU vendor**: LiveKit Cloud provides managed TURN infrastructure with TURN-TCP and TURN-TLS out of the box. In that scenario, the custom crate scope reduces to UDP TURN only (or may be skipped for the cloud-relay path). The crate design should remain agnostic to this and implement all three transport variants regardless — it will be needed for self-host deployments.

---

## 3. Integration Architecture

### 3.1 Component Layering

```
┌─────────────────────────────────────────────────────────┐
│ tze_hud_webrtc (str0m session manager)                  │
│                                                          │
│  SessionState { rtc: Rtc, turn_client: TurnClient? }    │
│                                                          │
│  poll loop:                                              │
│    rtc.poll_output() → Output::Transmit → route_transmit│
│    turn_client.poll_event() → TurnEvent → handle_turn   │
└─────────────────────────────────────────────────────────┘
         ↕ relay candidates         ↕ relay packets
┌─────────────────────────────────────────────────────────┐
│ tze_hud_turn (new crate)                                │
│                                                          │
│  TurnClient { config, state, transport: TurnTransport } │
│                                                          │
│  TurnTransport enum:                                     │
│    Udp(UdpSocket)                                        │
│    Tcp(TcpStream)                                        │
│    Tls(TlsStream<TcpStream>)                            │
└─────────────────────────────────────────────────────────┘
         ↕ TURN protocol (RFC 8656 / RFC 6062)
┌─────────────────────────────────────────────────────────┐
│ TURN Server (coturn / LiveKit Cloud / Cloudflare)        │
│ Port 3478 (UDP+TCP) or 443 (TLS+TCP)                    │
└─────────────────────────────────────────────────────────┘
```

### 3.2 TURN Client API

```rust
/// Crate: tze_hud_turn
/// File: src/lib.rs

pub struct TurnConfig {
    pub server_addr: SocketAddr,
    pub server_url: String,          // "turns:turn.example.com:443?transport=tcp"
    pub credentials: TurnCredentials,
    pub transport: TurnTransportKind,
    pub tls: Option<TlsConfig>,      // for TURN-TLS only
}

pub struct TurnCredentials {
    pub username: String,
    pub password: String,  // long-term credential (RFC 8656 §9.2)
}

pub enum TurnTransportKind {
    Udp,
    Tcp,
    Tls,
}

pub struct TlsConfig {
    pub server_name: String,
    pub pinned_cert_der: Option<Vec<u8>>,  // cert pinning; None = system roots
    pub ca_roots: TrustRoots,
}

pub enum TrustRoots {
    SystemRoots,                   // use webpki-roots
    Pinned(Vec<rustls::Certificate>),
    Custom(rustls::RootCertStore),
}

pub struct TurnClient {
    config: TurnConfig,
    state: TurnState,
    transport: TurnTransport,
    allocation: Option<TurnAllocation>,
}

pub struct TurnAllocation {
    pub relay_addr: SocketAddr,      // the externally-visible relay address
    pub lifetime: Duration,
    pub refresh_at: Instant,
}

pub enum TurnEvent {
    AllocationReady { relay_addr: SocketAddr },
    AllocationRefreshed { lifetime: Duration },
    DataReceived { from: SocketAddr, data: Bytes },
    AllocationExpired,
    Error(TurnError),
}

impl TurnClient {
    pub async fn allocate(config: TurnConfig) -> Result<Self, TurnError>;

    /// Poll for pending TURN events. Call this in the session poll loop.
    pub async fn poll_event(&mut self) -> TurnEvent;

    /// Send data to a remote address via the TURN relay.
    /// For UDP TURN: wraps in SEND indication or ChannelData.
    /// For TCP TURN (RFC 6062): wraps in TURN DATA indication on the TCP stream.
    pub async fn send_to(&mut self, data: &[u8], remote: SocketAddr) -> Result<(), TurnError>;

    /// Refresh the allocation before it expires (called by poll loop).
    pub async fn refresh(&mut self) -> Result<(), TurnError>;
}
```

### 3.3 Integration Sequence Diagram

```
tze_hud_webrtc        tze_hud_turn          TURN server         str0m Rtc
    │                     │                     │                   │
    │ TurnClient::allocate()                    │                   │
    │─────────────────────►                     │                   │
    │                     │──ALLOCATE req──────►│                   │
    │                     │  (REQUESTED-TRANSPORT: UDP|TCP)         │
    │                     │◄──ALLOCATE 401──────│ (auth challenge)  │
    │                     │──ALLOCATE req──────►│ (with HMAC-SHA1 creds)
    │                     │◄──ALLOCATE 200──────│                   │
    │                     │  (XOR-RELAYED-ADDRESS, LIFETIME)        │
    │◄─TurnEvent::AllocationReady(relay_addr)──│                   │
    │                     │                     │                   │
    │ rtc.add_local_candidate(Candidate::relayed(relay_addr, ...))  │
    │─────────────────────────────────────────────────────────────►│
    │                     │                     │                   │
    │ [ICE proceeds; str0m emits Output::Transmit to relay_addr]   │
    │◄─────────────────────────────────────────────────────────────│
    │                     │                     │                   │
    │ route_transmit(send):                     │                   │
    │  if send.destination == relay_addr:       │                   │
    │    turn_client.send_to(data, peer)        │                   │
    │─────────────────────►                     │                   │
    │                     │──SEND indication───►│                   │
    │                     │  (or ChannelData)   │──────TCP relay───►peer
    │                     │                     │                   │
    │ [inbound: TURN server wraps peer data in DATA indication]     │
    │                     │◄──DATA indication───│                   │
    │◄─TurnEvent::DataReceived(from, data)      │                   │
    │                     │                     │                   │
    │ rtc.handle_input(Input::Receive(Receive { source: peer, destination: relay_addr, contents: data }))
    │─────────────────────────────────────────────────────────────►│
    │                     │                     │                   │
    │ [Refresh cycle — every (LIFETIME * 0.8) seconds]             │
    │ turn_client.refresh()                     │                   │
    │─────────────────────►                     │                   │
    │                     │──REFRESH req───────►│                   │
    │                     │◄──REFRESH 200───────│                   │
```

### 3.4 Session Poll Loop Integration

The existing str0m session poll loop in `crates/tze_hud_webrtc/src/session.rs` (or equivalent) must be extended:

```rust
// Sketch — non-normative; adapt to actual session state structure
loop {
    tokio::select! {
        // str0m output processing
        output = poll_str0m(&mut rtc) => {
            match output {
                Output::Transmit(send) => {
                    route_transmit(&mut turn_client, &udp_socket, send).await?;
                }
                Output::Timeout(deadline) => {
                    // schedule wakeup at deadline
                }
                Output::Event(event) => {
                    handle_rtc_event(event).await?;
                }
            }
        }

        // TURN event processing
        turn_event = turn_client.poll_event(), if turn_client.is_some() => {
            match turn_event {
                TurnEvent::AllocationReady { relay_addr } => {
                    let candidate = Candidate::relayed(
                        relay_addr,
                        turn_client_local_addr,
                        "udp", // or "tcp"
                    )?;
                    rtc.add_local_candidate(candidate);
                }
                TurnEvent::DataReceived { from, data } => {
                    rtc.handle_input(Input::Receive(Receive {
                        source: from,
                        destination: relay_addr,
                        contents: (&data).into(),
                    }))?;
                }
                TurnEvent::AllocationExpired => {
                    log::warn!("TURN allocation expired; ICE connectivity may be lost");
                }
                TurnEvent::Error(e) => {
                    log::error!("TURN error: {e}");
                }
                _ => {}
            }
        }

        // Inbound UDP from non-relay paths
        result = udp_socket.recv_from(&mut buf) => {
            let (n, source) = result?;
            rtc.handle_input(Input::Receive(Receive {
                source,
                destination: local_udp_addr,
                contents: (&buf[..n]).into(),
            }))?;
        }
    }
}

async fn route_transmit(
    turn_client: &mut Option<TurnClient>,
    udp_socket: &UdpSocket,
    send: Transmit,
) -> Result<(), SessionError> {
    if let Some(tc) = turn_client {
        if send.destination == tc.relay_addr() {
            tc.send_to(&send.contents, send.destination).await?;
            return Ok(());
        }
    }
    udp_socket.send_to(&send.contents, send.destination).await?;
    Ok(())
}
```

---

## 4. UDP TURN (RFC 5766 / RFC 8656) — Baseline Path

RFC 8656 (current, obsoletes RFC 5766) defines the core TURN protocol over UDP:

1. Client sends `ALLOCATE` request with `REQUESTED-TRANSPORT: UDP` to TURN server.
2. Server responds with `XOR-RELAYED-ADDRESS` (the relay address) and `LIFETIME`.
3. Client periodically sends `REFRESH` requests to keep the allocation alive.
4. Client sends data using `SEND` indications (for peer addresses not yet in a channel) or `ChannelData` (after a `CHANNEL-BIND` for efficiency).
5. Server delivers data from peers to the client using `DATA` indications or `ChannelData`.

**Implementation notes:**
- The UDP TURN path is the simplest: one `UdpSocket`, no additional framing.
- `ChannelData` reduces overhead for high-frequency streams (4-byte header vs. 36-byte TURN header); bind channels for any peer after initial handshake.
- HMAC-SHA1 long-term credentials (RFC 8656 §9.2) are the standard auth mechanism.
- `MESSAGE-INTEGRITY` attribute on all requests; `MESSAGE-INTEGRITY-SHA256` preferred if server supports.
- Use `nonce` from the server's 401 challenge; re-fetch nonce when server returns 438 (Stale Nonce).

**This path should work out of the box** with str0m after the `TurnClient::allocate()` call completes and the relay candidate is added via `rtc.add_local_candidate()`.

---

## 5. TURN-TCP (RFC 6062) — Tunnel Path

### 5.1 Protocol Extension

RFC 6062 extends RFC 8656 to allocate TCP relay addresses. The additional messages:

| Message | Purpose |
|---|---|
| `ALLOCATE (REQUESTED-TRANSPORT: TCP)` | Request a TCP relay port |
| `CONNECT` | Initiate a TCP connection from the server to a remote peer |
| `CONNECTION-BIND` | Bind a data connection to a previously established CONNECT |
| `CONNECTION-ATTEMPT` (indication) | Server notifies client of an inbound connection from a peer |

### 5.2 TCP TURN Data Flow

RFC 6062 uses two separate TCP connections to the TURN server:
1. **Control connection**: carries ALLOCATE/REFRESH/CONNECT messages.
2. **Data connection(s)**: one per peer; carries raw application data (no TURN framing).

```
tze_hud ──TCP control conn──► TURN server
tze_hud ──TCP data conn #1──► TURN server ──TCP relay──► Peer A
tze_hud ──TCP data conn #2──► TURN server ──TCP relay──► Peer B
```

### 5.3 Integration with str0m

The data flowing over TCP TURN connections is the same DTLS/SRTP data that str0m would send over UDP. From str0m's perspective, the relay address is a normal ICE relay candidate with `proto: "tcp"`. tze_hud is responsible for:

1. Holding the TCP control connection alive (REFRESH on timer).
2. On `Output::Transmit` for the relay address: write to the correct TCP data connection.
3. On inbound data from a TCP data connection: `rtc.handle_input(Input::Receive(...))`.

**Key implementation detail**: RFC 6062 data connections carry raw bytes, not TURN-framed messages. The TURN server strips the TURN framing on the server side. tze_hud sends raw bytes on the data TCP connection; str0m's DTLS/SRTP processing handles the content.

### 5.4 `TurnTransport::Tcp` Implementation

```rust
// Non-normative sketch
impl TurnClient {
    async fn allocate_tcp(config: &TurnConfig) -> Result<Self, TurnError> {
        // 1. Connect TCP control stream to TURN server
        let control_stream = TcpStream::connect(config.server_addr).await?;
        
        // 2. Send ALLOCATE with REQUESTED-TRANSPORT: TCP
        let alloc_request = build_allocate_tcp(config);
        send_stun_on_tcp(&control_stream, &alloc_request).await?;
        
        // 3. Handle 401 challenge, re-send with credentials
        let response = recv_stun_on_tcp(&control_stream).await?;
        // ... (standard STUN auth exchange)
        
        // 4. Parse XOR-RELAYED-ADDRESS from 200 OK
        let relay_addr = parse_relayed_address(&response)?;
        
        Ok(TurnClient {
            control: TcpControl { stream: control_stream },
            data_conns: HashMap::new(),   // peer_addr → TcpStream
            relay_addr,
            // ...
        })
    }
    
    async fn connect_peer(&mut self, peer: SocketAddr) -> Result<(), TurnError> {
        // 1. Send CONNECT request on control connection (RFC 6062 §4.3)
        let connect_req = build_connect_request(peer);
        self.control.send(connect_req).await?;
        
        // 2. Receive 200 OK with CONNECTION-ID attribute
        let resp = self.control.recv().await?;
        let connection_id = parse_connection_id(&resp)?;
        
        // 3. Open a new TCP data connection to TURN server
        let data_stream = TcpStream::connect(self.config.server_addr).await?;
        
        // 4. Send CONNECTION-BIND on the data connection
        let bind_req = build_connection_bind(connection_id);
        send_stun_on_tcp(&data_stream, &bind_req).await?;
        let _ = recv_stun_on_tcp(&data_stream).await?; // expect 200 OK
        
        // 5. Register data connection; all subsequent sends/recvs are raw bytes
        self.data_conns.insert(peer, data_stream);
        Ok(())
    }
}
```

---

## 6. TURN-TLS (RFC 8656 + RFC 7443)

### 6.1 Protocol

TURN-TLS wraps either UDP TURN or TCP TURN inside a TLS session:

- **TURN-TCP + TLS** (most common, port 443 or 5349): TLS over TCP, then RFC 6062 on top.
- **TURN-UDP + DTLS** (RFC 7350, less common): DTLS over UDP, then RFC 8656 on top.

For corporate firewall traversal, **TURN-TCP+TLS on port 443** is the target: it is indistinguishable from HTTPS to perimeter firewalls.

### 6.2 TLS Implementation with rustls

```rust
// TlsStream construction for TURN-TLS
use tokio_rustls::{TlsConnector, rustls};

async fn connect_tls(config: &TlsConfig, server_addr: SocketAddr) -> Result<TlsStream<TcpStream>, TurnError> {
    let tcp_stream = TcpStream::connect(server_addr).await?;
    
    let tls_config = build_rustls_config(config)?;
    let connector = TlsConnector::from(Arc::new(tls_config));
    let server_name = config.server_name.as_str().try_into()?;
    
    Ok(connector.connect(server_name, tcp_stream).await?)
}

fn build_rustls_config(config: &TlsConfig) -> Result<rustls::ClientConfig, TurnError> {
    let mut root_store = rustls::RootCertStore::empty();
    
    match &config.ca_roots {
        TrustRoots::SystemRoots => {
            root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| {
                rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
                    ta.subject, ta.spki, ta.name_constraints,
                )
            }));
        }
        TrustRoots::Pinned(certs) => {
            for cert in certs {
                root_store.add(cert)?;
            }
        }
        TrustRoots::Custom(store) => {
            root_store = store.clone();
        }
    }
    
    let builder = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store);
    
    Ok(builder.with_no_client_auth())
}
```

### 6.3 Certificate Pinning

The `TlsConfig::pinned_cert_der` field supports pinning the TURN server's leaf certificate:

```rust
// During TLS handshake verification, compare server cert DER bytes
// against the pinned DER (if set). Reject if mismatch.
// Use a custom rustls::ServerCertVerifier implementation.
```

Pinning is optional; it is relevant for:
- Self-hosted coturn deployments where the TLS cert is under operator control.
- High-security environments where TURN traffic must be authenticated end-to-end.

For LiveKit Cloud or Cloudflare TURN, **system roots are correct**: their certs are signed by public CAs. Pinning should not be mandatory by default; it should be an opt-in configuration for self-hosted operators.

### 6.4 Trust Root Guidance

| Deployment | Recommended trust roots |
|---|---|
| LiveKit Cloud managed TURN | `TrustRoots::SystemRoots` |
| Cloudflare TURN (Anycast) | `TrustRoots::SystemRoots` |
| Self-hosted coturn (public cert) | `TrustRoots::SystemRoots` |
| Self-hosted coturn (self-signed) | `TrustRoots::Custom(operator cert)` |
| High-security isolated deployment | `TrustRoots::Pinned(leaf_cert)` |

---

## 7. Relationship to LiveKit Cloud Managed TURN

**If LiveKit Cloud is selected as the C15 SFU vendor**, its managed TURN infrastructure provides TURN-TCP and TURN-TLS out of the box via LiveKit's TURN endpoints.

Impact on this bead's scope:

| Scenario | TURN client scope |
|---|---|
| LiveKit Cloud as C15 vendor | UDP TURN may still be needed for direct peer-connection path; TCP/TLS TURN provided by LiveKit's managed TURN via ICE candidate injection into SDP offers. Verify at C15 vendor selection. |
| Cloudflare Calls as C15 vendor | Cloudflare's Anycast TURN is included; TURN-TCP/TLS handled by Cloudflare. Same as LiveKit: verify exact ICE candidate flow. |
| Self-hosted LiveKit / coturn | tze_hud must implement full TURN client (UDP + TCP + TLS). This bead is fully in scope. |
| No SFU (pure P2P) | Full TURN client required for firewall traversal. |

**Verification action at phase 4b kickoff**: Confirm LiveKit Cloud TURN endpoint configuration, supported transports (UDP/TCP/TLS), and whether TURN relay candidates appear in ICE offers from the LiveKit signaling server. If yes, the TCP/TLS TURN implementation in this crate becomes secondary (still needed for self-hosted), and the phase 4b gate criteria shift accordingly.

**This bead must not be deferred pending that verification.** The custom TURN client crate delivers value regardless: it provides self-hosted capability and serves as a verified reference implementation even if managed TURN covers the cloud-relay path.

---

## 8. Relationship to WHIP Signaling (hud-amf17 / RFC 0018)

### 8.1 ICE Candidate Gathering Must Complete Before WHIP SDP POST

RFC 9725 (WHIP) defines a simple HTTP ingest flow:

```
POST /whip/endpoint  ← SDP offer (from tze_hud)
201 Created          ← SDP answer (from server)
```

This is a **single-shot SDP exchange**. WHIP does not support trickle ICE in the standard form. Critically, the hud-s2j0l audit (PR #552) established:

> **Cloudflare Calls does not support trickle ICE per hud-s2j0l finding in PR #552.** The WHIP SDP offer sent to Cloudflare must include all ICE candidates in the initial POST body; candidates cannot be added via PATCH after the fact.

This creates a hard ordering dependency:

```
TURN client allocate()   ←── must complete FIRST
    ↓
relay_addr obtained
    ↓
Candidate::relayed() added to ICE candidate list
    ↓
SDP offer constructed (includes relay candidate)
    ↓
WHIP POST /endpoint      ←── sent with full candidate list
```

### 8.2 Gathering Timeout

tze_hud must define a TURN allocation timeout that bounds the WHIP connection setup latency. Recommended values:

| Phase | Timeout |
|---|---|
| TURN ALLOCATE request (single attempt) | 3 seconds |
| Total TURN gathering (with retries) | 8 seconds |
| WHIP connection setup hard deadline | 15 seconds |

If TURN allocation fails within the gathering timeout, tze_hud should fall through to the next ICE candidate type (ICE-TCP direct or UDP host candidates), rather than blocking WHIP POST indefinitely.

### 8.3 LiveKit Trickle ICE Exception

LiveKit's WHIP implementation supports trickle ICE via RFC 8840 (SDP fragment via PATCH). For LiveKit Cloud, relay candidates can be added after the initial SDP offer. The gathering timeout therefore only applies to the Cloudflare path.

This difference must be encoded in the `SfuVendorAdapter` trait (hud-s2j0l) via a capability flag:

```rust
pub trait SfuVendorAdapter {
    /// Whether this vendor supports trickle ICE via WHIP PATCH.
    /// False for Cloudflare Calls; true for LiveKit.
    fn supports_trickle_ice(&self) -> bool;
}
```

If `supports_trickle_ice()` is false, the session manager must wait for TURN gathering to complete (within the timeout) before calling `adapter.send_offer(sdp)`.

---

## 9. Test Plan

### 9.1 Unit Tests (`crates/tze_hud_turn/tests/`)

| Test | Description |
|---|---|
| `allocate_udp_mock` | Mock TURN server; verify ALLOCATE → 401 challenge → credential re-send → 200 OK → relay addr extraction |
| `allocate_tcp_mock` | Same flow over TCP; verify RFC 6062 REQUESTED-TRANSPORT attribute |
| `refresh_lifecycle` | Verify REFRESH is sent before LIFETIME expires (at 80% of lifetime) |
| `channel_bind_and_send` | CHANNEL-BIND request; ChannelData send; DATA indication receive |
| `nonce_rotation` | Server sends 438 (Stale Nonce); client fetches new nonce and retries |
| `connection_bind_tcp` | RFC 6062 CONNECT → CONNECTION-ID → second TCP conn → CONNECTION-BIND |
| `tls_trust_roots` | TLS handshake with system roots (using test cert); cert pinning rejection |

### 9.2 Integration Tests — TURN Stack

| Test | Description |
|---|---|
| `integration_udp_coturn` | Real coturn server in Docker; allocate UDP relay; send echo packet; verify round-trip |
| `integration_tcp_coturn` | Real coturn server; allocate TCP relay; CONNECT to echo server; data round-trip |
| `integration_tls_coturn` | coturn with TLS (test cert); full TLS TURN round-trip |
| `integration_str0m_relay_udp` | Full str0m session via UDP TURN relay; verify DTLS handshake + media packet |
| `integration_str0m_relay_tcp` | Full str0m session via TCP TURN relay |

### 9.3 Corporate Firewall Simulator

Use a Docker network policy to simulate a firewall that blocks all UDP and restricts TCP to port 443 only:

```dockerfile
# docker-compose.yml (test harness)
services:
  coturn:
    image: coturn/coturn:4.6
    ports:
      - "443:443"   # TURN-TLS only
    command: >
      --realm=test.local
      --lt-cred-mech
      --user=testuser:testpass
      --cert=/certs/server.pem
      --pkey=/certs/server.key
      --no-udp           # UDP disabled — simulates corporate firewall
      --tcp-port=443
      --tls-listening-port=443

  firewall:
    image: alpine
    cap_add: [NET_ADMIN]
    command: |
      iptables -A FORWARD -p udp -j DROP
      iptables -A FORWARD -p tcp --dport 443 -j ACCEPT
      iptables -A FORWARD -j DROP
```

```bash
# Run the firewall simulator test suite
cargo test --test integration_corporate_firewall -- --nocapture
```

The test verifies that:
1. UDP host candidates fail (blocked)
2. UDP TURN fails (blocked)
3. TURN-TCP on 443 succeeds
4. A full str0m WebRTC session establishes through the TURN-TCP relay

### 9.4 Test Coverage Targets

| Layer | Target coverage |
|---|---|
| TURN message codec | 90% line coverage |
| TurnClient state machine | 85% line coverage |
| Integration (coturn) | All 5 tests pass |
| Firewall simulator | TCP-only path verified end-to-end |

---

## 10. Effort Estimate

Per hud-kjody estimate (scope-validated in section 6 of the str0m TURN validation report):

| Work item | Estimate |
|---|---|
| `tze_hud_turn` crate skeleton + message codec (reusing `webrtc-stun` for framing) | 0.5 day |
| UDP TURN (RFC 8656) client: ALLOCATE, REFRESH, SEND/DATA, ChannelData | 1.5 days |
| TCP TURN (RFC 6062): CONNECT, CONNECTION-BIND, data connection management | 1 day |
| TLS wrapping (tokio-rustls, cert pinning, trust root config) | 0.5 day |
| str0m session loop integration (route_transmit, poll_event, trickle ICE timing) | 0.5 day |
| Unit tests (mock TURN server) | 0.5 day |
| Integration tests (coturn Docker) + firewall simulator | 0.5 day |
| **Total** | **5 days** |

**Breakdown by phase:**
- **UDP TURN (baseline)**: 2–3 days (crate skeleton + codec + UDP + session integration + unit tests)
- **TCP/TLS TURN (firewall traversal)**: 1.5–2 days (RFC 6062 + TLS wrapping + integration tests)

This is consistent with the hud-kjody estimate of "2–3 days UDP + 1–2 days TCP/TLS wrapping" and the bead scope label of "3–5 days."

---

## 11. Phase 4b Gate Criteria

TURN-over-TCP is considered compliant when ALL of the following gates pass:

| ID | Gate | Verification method |
|---|---|---|
| G1 | `TurnClient::allocate(TurnTransportKind::Udp)` succeeds against a real coturn server | `integration_udp_coturn` test passes |
| G2 | `TurnClient::allocate(TurnTransportKind::Tcp)` succeeds against a real coturn server with TCP TURN enabled | `integration_tcp_coturn` test passes |
| G3 | `TurnClient::allocate(TurnTransportKind::Tls)` succeeds against a coturn server with TLS on port 443 | `integration_tls_coturn` test passes |
| G4 | Full str0m WebRTC session (DTLS handshake + at least one media packet) establishes through a TURN-TCP relay | `integration_str0m_relay_tcp` test passes |
| G5 | Corporate firewall simulator (UDP blocked, TCP/443 only): tze_hud session establishes end-to-end | `integration_corporate_firewall` test passes |
| G6 | TURN allocation completes within 8 seconds under normal network conditions | Measured in integration tests |
| G7 | REFRESH is sent before allocation expires (no silent expiry) | Verified in `refresh_lifecycle` unit test |
| G8 | LiveKit Cloud TURN-TCP/TLS support verified (if LiveKit Cloud selected as C15 vendor) | Manual verification step at C15 decision; documents the out-clause from hud-kjody §7 |

Gates G1–G7 must pass before TURN-over-TCP is declared compliant. Gate G8 is an informational step that may reduce the required scope if LiveKit Cloud's managed TURN covers the corporate-firewall case.

---

## 12. Crate Structure

```
crates/tze_hud_turn/
├── Cargo.toml
├── src/
│   ├── lib.rs           # Public API: TurnClient, TurnConfig, TurnEvent
│   ├── codec.rs         # STUN/TURN message encoding/decoding
│   ├── auth.rs          # HMAC-SHA1 long-term credential generation
│   ├── transport/
│   │   ├── mod.rs
│   │   ├── udp.rs       # UDP TURN transport
│   │   ├── tcp.rs       # TCP TURN transport (RFC 6062)
│   │   └── tls.rs       # TLS wrapping (tokio-rustls)
│   ├── allocate.rs      # ALLOCATE / REFRESH state machine
│   ├── channel.rs       # CHANNEL-BIND / ChannelData
│   └── error.rs         # TurnError enum
└── tests/
    ├── unit/
    │   ├── codec_tests.rs
    │   ├── auth_tests.rs
    │   └── state_machine_tests.rs
    └── integration/
        ├── coturn_udp.rs
        ├── coturn_tcp.rs
        ├── coturn_tls.rs
        ├── str0m_relay.rs
        └── corporate_firewall.rs
```

**Cargo.toml dependencies:**

```toml
[dependencies]
tokio = { version = "1", features = ["net", "time"] }
tokio-rustls = "0.26"
rustls = "0.23"
webpki-roots = "0.26"
bytes = "1"
hmac = "0.12"
sha1 = "0.10"
rand = "0.8"
tracing = "0.1"
thiserror = "1"

[dev-dependencies]
tokio = { version = "1", features = ["test-util", "macros"] }
```

---

## 13. Open Questions and Follow-Ups

| ID | Question | Resolution path |
|---|---|---|
| FU-1 | LiveKit Cloud: does its TURN service support TURN-TCP on port 443 for clients? What are the credentials API surface? | Verify at C15 vendor selection kickoff; if yes, G8 is satisfied and TCP TURN bead scope reduces to self-hosted |
| FU-2 | Cloudflare TURN: same as FU-1, but for CF's Anycast TURN | Same as FU-1 |
| FU-3 | Should `tze_hud_turn` expose a STUN-only client for server-reflexive candidate gathering? | Separate concern; out of scope here but could share the codec module |
| FU-4 | coturn Docker image for CI: should this live in `ci/docker/` or a test-fixtures directory? | Coordinator to decide; no blocker |
| FU-5 | str0m issue #723 (built-in TURN) — if this merges before phase 4b, this crate may be redundant for the str0m path | Monitor; will not block implementation |

---

## References

- `docs/reports/str0m-turn-over-tcp-validation.md` (hud-kjody, CONDITIONAL-GO verdict)
- `docs/audits/webrtc-sfu-fallback-audit.md` (hud-1ee3a, str0m recommended fallback)
- `docs/reports/sfu-vendor-adapter-seam.md` (hud-s2j0l, PR #552, SfuVendorAdapter trait)
- RFC 8656: TURN (obsoletes RFC 5766) — https://datatracker.ietf.org/doc/html/rfc8656
- RFC 6062: TURN extensions for TCP allocations — https://www.rfc-editor.org/rfc/rfc6062.html
- RFC 6544: ICE-TCP — https://datatracker.ietf.org/doc/html/rfc6544
- RFC 7443: STUN over TLS/DTLS — https://datatracker.ietf.org/doc/html/rfc7443
- RFC 9725: WHIP — https://www.rfc-editor.org/rfc/rfc9725.html
- RFC 8840: Trickle ICE for SDP — https://datatracker.ietf.org/doc/html/rfc8840
- str0m repository: https://github.com/algesten/str0m
- str0m ICE docs: https://github.com/algesten/str0m/blob/main/docs/ice.md
- webrtc-rs issue #539 (TCP TURN, closed not_planned): https://github.com/webrtc-rs/webrtc/issues/539
- hud-amf17: RFC 0018 WHIP Signaling Adapter, PR #548
- hud-0bqk8: IPv6 ICE audit, PR #551
