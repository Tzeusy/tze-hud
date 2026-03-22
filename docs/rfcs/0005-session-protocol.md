# RFC 0005: Session/Protocol

**Status:** Draft
**Issue:** rig-5vq.8
**Date:** 2026-03-22
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0003 (Timing Model), RFC 0004 (Input Model)

---

## Review Changelog

| Round | Date | Reviewer | Focus | Changes |
|-------|------|----------|-------|---------|
| 1 | 2026-03-22 | rig-5vq.27 | Doctrinal alignment deep-dive | `heartbeat_interval_ms` default 10000→5000; split `max_concurrent_sessions` into resident/guest; added `MtlsCredential`; split `InputEvent` traffic class row; added `active_subscriptions`/`denied_subscriptions` to `SessionEstablished`; replaced `SceneEvent` DELTA_COMPLETE sentinel with `StateDeltaComplete {}` message. |
| 2 | 2026-03-22 | rig-5vq.28 | Technical architecture scrutiny | Removed dead resume fields from `SessionInit` (reserved 9–10); completed state machine (Handshaking→Closed, Resuming→Closed paths); added `heartbeat_missed_threshold = 3` config param (3× not 2×); per-session dedup window; `SubscriptionChangeResult` replaces `MutationResult` for subscription acks; `ZonePublishResult` for durable zone acks; `RuntimeError.ErrorCode` typed enum; `CapabilityRequest` rejection semantics; v1 implementation note in §6.4; `InputMessage` variant filter rule in §7.1; embodied presence Q in §12. |

---

## Summary

This RFC defines the wire-level session protocol for agent-to-runtime communication over the gRPC resident control plane. It covers the full session lifecycle (handshake through disconnect), the multiplexed session stream envelope format, all message types and their traffic class assignments, version negotiation, ordering and idempotency guarantees, reconnection and state resumption, subscription management, the MCP bridge, and the protobuf schema for all session messages.

The session protocol is the contract between a resident or embodied agent and the tze_hud runtime. It defines precisely what an agent sends, what the runtime sends back, what ordering guarantees apply, and what happens when connections drop or the runtime restarts.

---

## Motivation

tze_hud gives LLMs governed, performant presence on real screens. That presence requires a durable, bidirectional communication channel that handles the realities of production operation: agents crash, networks blip, runtimes restart. Without a precisely defined session protocol:

- Agents cannot reliably reconnect after a transient disconnect.
- The runtime cannot enforce ordering without a shared sequence model.
- Duplicate mutations (retransmits) can corrupt scene state.
- Version mismatches between agent and runtime produce silent misbehavior.
- Subscription management is ad-hoc and cannot be audited.

This RFC resolves all of these by specifying the session protocol as a first-class contract with defined semantics at every layer.

---

## Design Requirements Satisfied

| ID | Requirement | Source |
|----|-------------|--------|
| DR-SP1 | Single bidirectional stream per resident agent | architecture.md §"Session model" |
| DR-SP2 | Graceful and ungraceful disconnect handling | failure.md §"Agent crashes" |
| DR-SP3 | Reconnection grace period with state resumption | failure.md §"Reconnection contract" |
| DR-SP4 | Version negotiation at handshake | architecture.md §"Versioning and protocol compatibility" |
| DR-SP5 | Structured, machine-readable errors on all planes | architecture.md §"Error model" |
| DR-SP6 | Authentication before any capability grant | security.md §"Authentication" |
| DR-SP7 | MCP bridge for guest/zero-context LLM access | architecture.md §"Compatibility plane: MCP" |
| DR-SP8 | Capability scope filtering on subscriptions | security.md §"Capability scopes" |

---

## 1. Session Lifecycle

### 1.1 Overview

A resident agent session progresses through six states:

```
Connecting → Handshaking ──────────────────────────── Active ⇄ Disconnecting
                  │ (auth/version failure)              │                │
                  ↓                                     │ (ungraceful)   │ (graceful)
                Closed                                  ↓                ↓
                                                   Closed (orphaned leases, grace period)
                                                        │
                                              (within grace period)
                                                        ↓
                                                   Resuming
                                                  ↙        ↘
                                     (accepted)              (token expired/invalid)
                                        ↓                           ↓
                                      Active                      Closed
```

States:

| State | Description |
|-------|-------------|
| `Connecting` | TCP/TLS establishment, HTTP/2 stream setup |
| `Handshaking` | Agent sends `SessionInit`; runtime validates auth and capabilities |
| `Active` | Bidirectional `SessionMessage` stream open; agent can send mutations, receive events |
| `Disconnecting` | Graceful close — agent sends `SessionClose` or server initiates |
| `Closed` | Stream closed; if previously `Active`, leases enter orphan state with grace period. If from `Handshaking` (auth failure, version mismatch), no leases exist. |
| `Resuming` | Reconnecting agent presents session token before grace period expiry. Transitions to `Active` on acceptance; to `Closed` on token expiry or invalid token. |

### 1.2 SessionInit (Client → Server)

The first message an agent sends on a new stream. Must arrive within `handshake_timeout_ms` (default: 5000 ms) or the runtime closes the stream with `DEADLINE_EXCEEDED`.

```protobuf
message SessionInit {
  // Identity
  string agent_id = 1;                  // Stable agent identifier (e.g. "weather-agent")
  string agent_display_name = 2;        // Human-readable label for system shell

  // Protocol
  uint32 min_protocol_version = 3;      // Minimum version the agent can speak
  uint32 max_protocol_version = 4;      // Highest version the agent prefers

  // Authentication
  AuthCredential auth_credential = 5;

  // Capability requests
  repeated string requested_capabilities = 6;   // See §7 and security.md

  // Initial subscriptions — can be modified later via SubscriptionChange
  repeated SubscriptionCategory initial_subscriptions = 7;

  // Presence level hint (guest/resident/embodied)
  PresenceLevel presence_level = 8;

  // Fields 9–10 are reserved. Resume attempts use SessionResume (§6.2), not SessionInit.
  // Encoding resume fields in SessionInit would create a dual handshake path and bypass
  // the SessionResume validation logic. An agent reconnecting within the grace period must
  // send SessionResume as its first message, not SessionInit.
  reserved 9, 10;
  reserved "resume_session_token", "resume_last_seen_server_seq";
}
```

### 1.3 SessionEstablished (Server → Client)

Sent by the runtime after successful authentication and capability negotiation. This is the first server message on a new stream.

```protobuf
message SessionEstablished {
  string session_token = 1;             // Opaque token; present for resume within grace period
  uint32 negotiated_protocol_version = 2;
  repeated string granted_capabilities = 3;
  uint64 heartbeat_interval_ms = 4;     // How often agent must send HeartbeatPing
  string namespace = 5;                 // Agent's namespace in the scene (RFC 0001 §1.2)
  uint64 server_sequence = 6;           // Starting server-side sequence number
  repeated SubscriptionCategory active_subscriptions = 7;   // Confirmed subscriptions
  repeated SubscriptionCategory denied_subscriptions = 8;   // Requested but denied (missing capability)
}
```

`denied_subscriptions` is populated when an agent requests subscription categories for which it lacks the required capability (§7.2). The denied categories are listed here rather than being silently dropped — agents can inspect this field to detect capability gaps and request elevated capabilities if needed.

### 1.4 Authentication

Authentication is evaluated synchronously during handshake before `SessionEstablished` is sent. If authentication fails, the runtime sends `SessionError` and closes the stream.

```protobuf
message AuthCredential {
  oneof mechanism {
    PreSharedKeyCredential pre_shared_key = 1;
    LocalSocketCredential  local_socket   = 2;
    OauthTokenCredential   oauth_token    = 3;
    MtlsCredential         mtls           = 4;
  }
}

message PreSharedKeyCredential {
  string key_id  = 1;
  string api_key = 2;
}

message LocalSocketCredential {
  // Unix socket UID/GID validated by runtime from OS credentials
  // Field present to signal credential type; value ignored
  bool unix_creds = 1;
}

message OauthTokenCredential {
  string access_token = 1;
  string token_type   = 2;  // e.g. "Bearer"
}

message MtlsCredential {
  // mTLS: client certificate identity is validated at the TLS layer.
  // This message signals that the agent is presenting a client cert via the
  // underlying TLS handshake; no additional fields are required here.
  // The runtime extracts and verifies the certificate chain from the TLS
  // context before this message is even read.
  string cert_fingerprint = 1;  // Optional: SHA-256 hex fingerprint for audit log
}
```

The runtime's auth mechanism is pluggable (security.md §"Authentication"). The `AuthCredential` oneof is the wire encoding — the runtime maps each variant to its registered auth handler. V1 ships pre-shared key and local socket implementations; mTLS and OAuth2/OIDC are supported as protocol-level variants but their full implementation is deferred to the Security RFC.

### 1.5 Graceful Disconnect

Agent sends `SessionClose` to initiate graceful shutdown. The runtime acknowledges, tears down subscriptions, and starts the lease grace period.

```protobuf
message SessionClose {
  string reason = 1;    // Optional human-readable reason
  bool   expect_resume = 2;  // Hint: agent intends to reconnect shortly
}
```

If `expect_resume` is true, the runtime holds leases at the full grace period. If false, the runtime may accelerate cleanup. Either way, the grace period starts on stream close.

### 1.6 Ungraceful Disconnect

When the stream drops without a `SessionClose` (crash, network failure, heartbeat timeout), the runtime:

1. Detects disconnection via gRPC stream EOF, RST, or heartbeat timeout (missing `HeartbeatPing` after `heartbeat_missed_threshold × heartbeat_interval_ms`; default: `3 × 5000 ms = 15 000 ms`).
2. Marks agent's leases as "orphaned" — rendered frozen at last known state.
3. Displays a disconnection badge on affected tiles (chrome layer, non-blocking).
4. Starts the reconnection grace period (default: 30 000 ms; configurable per-agent).

### 1.7 Error During Handshake

```protobuf
message SessionError {
  SessionErrorCode code    = 1;
  string           message = 2;     // Human-readable
  string           context = 3;     // Which field or value triggered the error
  string           hint    = 4;     // Machine-readable correction suggestion

  enum SessionErrorCode {
    SESSION_ERROR_CODE_UNSPECIFIED          = 0;
    AUTH_FAILED                             = 1;
    UNSUPPORTED_PROTOCOL_VERSION            = 2;
    CAPABILITY_NOT_GRANTED                  = 3;
    HANDSHAKE_TIMEOUT                       = 4;
    SESSION_NOT_FOUND                       = 5;   // Resume: no such token
    SESSION_GRACE_EXPIRED                   = 6;   // Resume: too late
    DUPLICATE_AGENT_ID                      = 7;   // Another session with same agent_id is active
    INVALID_PRESENCE_LEVEL                  = 8;
    SEQUENCE_GAP_EXCEEDED                   = 9;   // Client sequence gap > max_sequence_gap (§5.4)
    SEQUENCE_REGRESSION                     = 10;  // Client sent a sequence number lower than previously seen (§5.4)
  }
}
```

The error model follows the architecture.md §"Error model" contract: code, human-readable message, context, and correction hint.

---

## 2. Multiplexing Format

### 2.1 Stream Topology

Each resident agent holds exactly one primary bidirectional gRPC stream of type `stream SessionMessage / stream SessionMessage`. All scene mutations, event subscriptions, lease management, heartbeats, and telemetry are multiplexed over this single stream.

Embodied agents (post-v1) may additionally open a media signaling stream for WebRTC negotiation. That stream is separate from the session stream and is out of scope for v1.

**Rule:** Do not proliferate per-concern streams. HTTP/2 has a concurrent-stream limit that becomes a bottleneck under many active streams. One session stream per agent is the v1 target topology.

### 2.2 SessionMessage Envelope

Every message on the session stream — in both directions — is wrapped in a `SessionMessage` envelope. The envelope provides sequence numbering, timestamps, and a `oneof` payload.

```protobuf
message SessionMessage {
  uint64    sequence  = 1;   // Per-direction monotonically increasing, starts at 1
  uint64    timestamp_us = 2; // Sender wall-clock (µs since Unix epoch); advisory only
  oneof payload {
    // Session lifecycle (bidirectional)
    SessionInit         session_init          = 10;
    SessionEstablished  session_established   = 11;
    SessionClose        session_close         = 12;
    SessionError        session_error         = 13;
    SessionResume       session_resume        = 14;
    SessionResumeResult session_resume_result = 15;

    // Agent → Runtime
    MutationBatch       mutation_batch        = 20;
    LeaseRequest        lease_request         = 21;
    HeartbeatPing       heartbeat_ping        = 22;
    CapabilityRequest   capability_request    = 23;
    SubscriptionChange  subscription_change   = 24;
    ZonePublish         zone_publish          = 25;

    // Runtime → Agent
    MutationResult      mutation_result       = 30;
    LeaseResponse       lease_response        = 31;
    HeartbeatPong       heartbeat_pong        = 32;
    SceneEvent          scene_event           = 33;
    InputEvent          input_event           = 34;
    DegradationNotice   degradation_notice    = 35;
    RuntimeError        runtime_error         = 36;
    CapabilityNotice    capability_notice     = 37;  // Mid-session grant/revoke
  }
}
```

### 2.3 Sequence Numbers

- Both directions maintain independent monotonically increasing sequence counters, starting at 1.
- The server sends its initial `sequence` value in `SessionEstablished.server_sequence`.
- Client sequence starts at 1 on the first `SessionMessage` after `SessionInit`.
- Sequence gaps indicate lost messages (stream close without `SessionClose`). On reconnect, the client's `SessionResume.last_seen_server_sequence` allows the server to reconstruct missed events.

### 2.4 Backpressure

The session stream uses HTTP/2 flow control as the primary backpressure mechanism. Additionally:

- **State-stream messages** (dashboard patches, scene topology changes): the runtime coalesces updates when the client is not reading fast enough. If the runtime has 10 queued `SceneEvent` messages for a slow client, it applies coalesce-key merging before sending.
- **Ephemeral realtime messages** (cursor trails, interim speech tokens): the runtime drops the oldest message when the send buffer reaches the per-session ephemeral quota (default: 16 messages). Ephemeral messages are `latest-wins` by design.
- **Transactional messages** (mutation results, lease responses): never dropped. If the send buffer is full, the runtime applies HTTP/2 backpressure. The agent must drain its receive buffer.

---

## 3. Message Types

### 3.1 Client → Server Messages

| Message | Traffic Class | Description |
|---------|--------------|-------------|
| `MutationBatch` | Transactional | Atomic set of scene mutations (RFC 0001 §4) |
| `LeaseRequest` | Transactional | Request or renew a surface lease |
| `HeartbeatPing` | Ephemeral | Keepalive; must arrive within `heartbeat_missed_threshold × heartbeat_interval_ms` |
| `CapabilityRequest` | Transactional | Request an additional capability mid-session |
| `SubscriptionChange` | Transactional | Add/remove event subscription categories; acked by `SubscriptionChangeResult` |
| `ZonePublish` | State-stream (ephemeral zones) or Transactional (durable zones) | Push content to a named zone. Durable-zone publishes receive a `ZonePublishResult` ack; ephemeral-zone publishes are fire-and-forget. |

### 3.2 Server → Client Messages

| Message | Traffic Class | Description |
|---------|--------------|-------------|
| `MutationResult` | Transactional | Accept/reject response for a `MutationBatch` |
| `LeaseResponse` | Transactional | Grant/deny/revoke for a lease operation |
| `HeartbeatPong` | Ephemeral | Reply to `HeartbeatPing` with server timestamp (wall-clock; not suitable for RTT measurement) |
| `SceneEvent` | State-stream | Topology change, zone occupancy update, lease change |
| `InputEvent` (pointer/key variants) | Ephemeral realtime | Pointer/touch/key events routed to agent via RFC 0004 `InputEnvelope`. Coalesced under backpressure (RFC 0004 §8.5). |
| `InputEvent` (focus/capture/IME variants) | Transactional | `FocusGainedEvent`, `FocusLostEvent`, `CaptureReleasedEvent`, and IME events carried in the same RFC 0004 `InputEnvelope` oneof. Never dropped or coalesced per RFC 0004 §8.5 — delivery is reliable and ordered. |
| `DegradationNotice` | Transactional | Runtime has changed degradation level; see §3.4 |
| `RuntimeError` | Transactional | Structured error (see §3.5) |
| `CapabilityNotice` | Transactional | Mid-session capability grant or revocation |
| `SubscriptionChangeResult` | Transactional | Ack/nack for a `SubscriptionChange`; echoes full active subscription set |
| `ZonePublishResult` | Transactional | Ack/nack for a durable-zone `ZonePublish`; not sent for ephemeral zones |

### 3.3 MutationBatch

```protobuf
message MutationBatch {
  string         batch_id   = 1;  // UUIDv7 — used for deduplication (§5.2)
  string         lease_id   = 2;  // Lease under which mutations execute
  repeated MutationProto mutations = 3;  // Ordered; applied atomically (RFC 0001 §4)
  TimingHints    timing     = 4;  // Optional present_at / expires_at (RFC 0003)
}

message MutationResult {
  string         batch_id      = 1;
  bool           accepted      = 2;
  repeated string created_ids  = 3;  // UUIDv7 SceneIds (RFC 0001 §1.1) assigned to CreateTile/CreateNode mutations
  RuntimeError   error         = 4;  // Populated if accepted = false
}
```

Mutations map directly to RFC 0001 §4 scene operations. The `batch_id` is used for at-least-once deduplication (§5.2).

### 3.4 DegradationNotice

```protobuf
message DegradationNotice {
  DegradationLevel level   = 1;
  string           reason  = 2;  // Human-readable explanation
  repeated string  affected_capabilities = 3;  // Which capabilities are reduced

  enum DegradationLevel {
    DEGRADATION_LEVEL_UNSPECIFIED    = 0;
    NORMAL                           = 1;
    COALESCING_MORE                  = 2;
    MEDIA_QUALITY_REDUCED            = 3;
    STREAMS_REDUCED                  = 4;
    RENDERING_SIMPLIFIED             = 5;
    SHEDDING_TILES                   = 6;
    AUDIO_ONLY_FALLBACK              = 7;
  }
}
```

The degradation ladder is defined in failure.md §"Degradation axes". Agents must gracefully handle capability reduction; non-compliance within the grace period leads to session throttling.

### 3.5 RuntimeError

The structured error model (architecture.md §"Error model") applies to all error responses:

```protobuf
message RuntimeError {
  string    error_code      = 1;   // String identifier for extensibility (e.g. "LEASE_EXPIRED")
  string    message         = 2;   // Short human-readable sentence
  string    context         = 3;   // Invalid field, value, or operation
  string    hint            = 4;   // Machine-readable correction suggestion (JSON object)
  ErrorCode error_code_enum = 5;   // Typed enum for well-known codes; preferred for programmatic handling

  // Well-known error codes. String `error_code` is the canonical identifier (stable, not renamed).
  // `error_code_enum` mirrors the most common values for type-safe handling in generated clients.
  // Unknown codes not in this enum are represented as ERROR_CODE_UNKNOWN; inspect `error_code` for detail.
  enum ErrorCode {
    ERROR_CODE_UNSPECIFIED    = 0;
    ERROR_CODE_UNKNOWN        = 1;   // Code not in this enum version; see string error_code
    LEASE_EXPIRED             = 2;
    LEASE_NOT_FOUND           = 3;
    ZONE_TYPE_MISMATCH        = 4;
    ZONE_NOT_FOUND            = 5;
    BUDGET_EXCEEDED           = 6;
    MUTATION_REJECTED         = 7;
    PERMISSION_DENIED         = 8;
    RATE_LIMITED              = 9;
    INVALID_ARGUMENT          = 10;
    SESSION_EXPIRED           = 11;
  }
}
```

`error_code` (string) is the canonical, stable identifier across all protocol versions. `error_code_enum` is provided for type-safe handling in generated clients — set it to `ERROR_CODE_UNKNOWN` for any code not yet in the enum. The two fields always carry the same logical value.

Common error codes are defined per-subsystem: scene operation errors in RFC 0001, lease errors in RFC 0002, input routing errors in RFC 0004, and session errors in §1.7 above.

### 3.6 HeartbeatPing / HeartbeatPong

```protobuf
message HeartbeatPing {
  uint64 client_timestamp_us = 1;   // Client monotonic clock
}

message HeartbeatPong {
  uint64 client_timestamp_us = 1;   // Echo of ping value
  uint64 server_timestamp_us = 2;   // Server wall-clock at receipt
}
```

Heartbeat interval is negotiated at handshake (`SessionEstablished.heartbeat_interval_ms`). The runtime treats the session as ungracefully disconnected when `heartbeat_missed_threshold` consecutive pings are missed (default: `heartbeat_missed_threshold = 3`, so `3 × 5000 ms = 15 000 ms`).

Note: `HeartbeatPong.server_timestamp_us` is a wall-clock value and is not suitable for round-trip latency estimation (wall clocks can jump). Agents that need RTT measurement should compute it from `HeartbeatPing.client_timestamp_us` using their own monotonic clock.

---

## 4. Version Negotiation

### 4.1 Protocol Version Numbers

Protocol versions follow a `major.minor` scheme encoded as a single `uint32`:

```
version = major * 1000 + minor
```

Examples: `1000` = v1.0, `1001` = v1.1, `2000` = v2.0.

### 4.2 Negotiation at Handshake

The agent declares its supported range in `SessionInit`:

```
min_protocol_version: 1000   // Lowest version the agent can speak
max_protocol_version: 1001   // Highest version the agent prefers
```

The runtime picks the highest version within `[min, max]` that it supports and returns it in `SessionEstablished.negotiated_protocol_version`. If no mutual version exists, the runtime sends `SessionError` with `UNSUPPORTED_PROTOCOL_VERSION`.

### 4.3 Compatibility Guarantees

**Minor versions** (e.g., v1.0 → v1.1): additive changes only. New optional fields, new `oneof` variants, new enum values. Agents that do not know a new field ignore it (protobuf forward compatibility). New enum values that the agent does not know about are treated as `UNSPECIFIED`. The runtime does not send minor-version features to agents that declared `max_protocol_version` below the feature version.

**Major versions** (e.g., v1.x → v2.0): may change wire format, remove deprecated fields, or alter fundamental semantics. The runtime supports the current major version and one prior major version simultaneously. An agent from two major versions ago cannot connect.

**MCP tools** are versioned alongside the gRPC protocol. A new zone type or mutation operation that ships with compositor v1.2 also ships as a new MCP tool in the same release.

---

## 5. Ordering and Idempotency

### 5.1 Delivery Guarantees by Traffic Class

| Traffic Class | Delivery | Ordering | Dropped? |
|---------------|----------|----------|---------|
| Transactional | At-least-once (ack + retransmit) | Per-direction sequence order | Never |
| State-stream | At-least-once, coalesced | Sequence order; intermediate states may be skipped | Never (coalesced, not dropped) |
| Ephemeral realtime | At-most-once (fire and forget) | Best-effort | Yes, under backpressure |

### 5.2 Batch Idempotency

Every `MutationBatch` carries a `batch_id` (UUIDv7). The runtime maintains a **per-session** deduplication window (not global — cross-session deduplication is neither required nor beneficial since UUIDv7 batch IDs are globally unique):

- **Window size:** 1000 unique `batch_id` values per session, or 60 seconds, whichever expires first.
- **Behavior on duplicate:** The runtime returns the original `MutationResult` without re-applying the mutations. This is transparent to the agent — retransmit produces the same result as the original send.
- **Behavior after window expiry:** A `batch_id` that reappears after 60 seconds is treated as a new batch. Agents must not retransmit a batch after 60 seconds; they should treat the original as lost and issue a new batch with a new ID if needed.

Per-session windows are required for correctness: with `max_concurrent_resident_sessions = 16` sessions each sending mutations at 60Hz, a global 1000-entry window would roll over in approximately 1 second — far short of the 60-second retransmit safety guarantee. Per-session windows eliminate this contention entirely.

### 5.3 Retransmission Policy

Agents are responsible for retransmitting unacknowledged transactional messages:

1. The agent sends a `MutationBatch` with sequence N.
2. If no `MutationResult` arrives within `retransmit_timeout_ms` (default: 5000 ms), the agent resends the same message with the same `batch_id` but a new `sequence` number.
3. The runtime deduplicates via `batch_id` and returns the cached result.
4. After 3 retransmits with no acknowledgement, the agent should treat the session as degraded and attempt reconnection.

Lease operations and `CapabilityRequest` follow the same at-least-once + retransmit pattern, using the `sequence` field as the correlation key (no separate `batch_id`; sequence numbers are per-direction unique).

**`CapabilityRequest` rejection:** When the runtime denies a capability request (insufficient trust level, capability not available, or policy restriction), it sends a `RuntimeError` on the session stream with `error_code = "PERMISSION_DENIED"` and `context` set to the names of the denied capabilities (comma-separated). The `RuntimeError` is correlated to the request by matching its position in the server's response sequence against the client's `CapabilityRequest` sequence number. At most one `CapabilityRequest` should be in flight per session at a time; concurrent requests will be processed in arrival order but correlation becomes ambiguous if multiple requests are denied simultaneously.

### 5.4 Sequence Monotonicity

The runtime validates that client-side sequence numbers are monotonically increasing. A gap of 1 is expected (missed message). A gap larger than `max_sequence_gap` (default: 100) causes the runtime to close the stream with a `SEQUENCE_GAP_EXCEEDED` error, forcing a fresh reconnect. Sequence resets (client sending a lower number than previously seen) are rejected with `SEQUENCE_REGRESSION`.

---

## 6. Reconnection and Resumption

### 6.1 Session Token

On `SessionEstablished`, the runtime issues a `session_token` — an opaque, cryptographically random token bound to the session. Tokens are:

- Single-use for resumption (a successful resume issues a new token).
- Bound to the `agent_id` and `namespace` (a token cannot be used to resume a different agent's session).
- Valid for the grace period duration (default: 30 000 ms from stream close).

Agents must store the token in memory for the duration of their session. Tokens are not persisted across process restarts.

### 6.2 SessionResume (Client → Server)

When reconnecting within the grace period, the agent sends `SessionResume` as the first message instead of `SessionInit`:

```protobuf
message SessionResume {
  string agent_id                  = 1;
  string session_token             = 2;
  uint64 last_seen_server_sequence = 3;  // Last `sequence` the agent received before disconnect
  AuthCredential auth_credential   = 4;  // Re-authenticate even on resume
}
```

The `last_seen_server_sequence` allows the runtime to reconstruct the set of server messages the agent missed during the gap.

### 6.3 SessionResumeResult (Server → Client)

```protobuf
message SessionResumeResult {
  bool   accepted                = 1;
  string new_session_token       = 2;   // New token for this resumed session
  uint64 new_server_sequence     = 3;   // Server sequence to use going forward
  uint32 negotiated_protocol_version = 4;
  repeated string granted_capabilities = 5;
  RuntimeError error             = 6;   // Populated if accepted = false
}
```

### 6.4 State Delta on Resume

When a resume is accepted within the grace period:

1. The runtime identifies all server-side `SceneEvent` and `LeaseResponse` messages with `sequence > last_seen_server_sequence`.
2. It replays these missed transactional/state-stream messages as a burst of normal `SessionMessage` envelopes (carrying `SceneEvent`, `LeaseResponse`, or `CapabilityNotice` payloads). This replayed burst is informally called the "state delta." A dedicated `StateDeltaComplete` sentinel message (see §9) is sent as the final message in the burst to signal the end of the catch-up phase.
3. Ephemeral events (cursor moves, interim speech tokens) are not replayed — they are inherently transient.
4. Once the delta burst is complete (the agent receives `StateDeltaComplete`), the session transitions to `Active` state normally.

If the agent's leases are still orphaned (not yet evicted — within grace period), they are automatically reclaimed as part of the state delta. The disconnection badges clear.

> **V1 implementation note:** v1.md §"V1 explicitly defers" states "No resumable state sync (reconnecting agents get a full snapshot, not a diff)." The delta-replay mechanism specified above is the target API contract for v1.1+. The v1 implementation ships a full scene snapshot on resume instead of incremental delta replay: rather than replaying individual missed `SceneEvent` messages, the runtime sends a single `SceneEvent` carrying a full scene topology snapshot, followed by `StateDeltaComplete`. The `last_seen_server_sequence` field in `SessionResume` is accepted but may be ignored by the v1 implementation. Agents must handle both full-snapshot and delta-replay resume responses correctly (both terminate with `StateDeltaComplete`).

### 6.5 Post-Grace-Period Reconnect

If the grace period expires before the agent reconnects:

1. The runtime has evicted the agent's leases and cleared its tiles.
2. The `session_token` is no longer valid.
3. The agent must perform a full re-handshake via `SessionInit` (no resume token).
4. The runtime sends the current scene topology snapshot so the agent can make informed lease requests.
5. Capabilities are re-granted based on the agent's registered profile (capability grants are durable from config; security.md §"Authentication").

### 6.6 Runtime Restart

After the display node process restarts:

1. All session tokens are invalid — the token store is in-memory only.
2. All leases are gone — the scene is ephemeral by default (failure.md §"Display node restart").
3. Agents receive connection-refused or gRPC stream error; they must reconnect with `SessionInit`.
4. Tab and layout configuration persists (loaded from config file at startup).
5. Agent registration and capability profiles persist (config-driven), so agents re-authenticate quickly without re-requesting all capabilities.

---

## 7. Subscription Management

### 7.1 Subscription Categories

Agents declare which event categories they want to receive. Receiving events for unsubscribed categories wastes bandwidth and CPU; emitting events to unsubscribed agents is a protocol violation.

| Category | Description | Minimum Capability |
|----------|-------------|-------------------|
| `scene_topology` | Tile created/deleted/updated, tab switched | `read_scene` |
| `input_events` | Pointer, touch, key events routed to agent's tiles | `receive_input` |
| `focus_events` | Focus gained/lost on agent's tiles | `receive_input` |
| `degradation_notices` | Runtime degradation level changes | *(always subscribed)* |
| `lease_changes` | Lease granted/renewed/revoked/expired for agent's leases | *(always subscribed)* |
| `zone_events` | Zone occupancy changes in zones the agent has publish access to | `zone_publish:<zone>` |

`degradation_notices` and `lease_changes` are delivered unconditionally to all active sessions — they are not filterable because agents must react to them.

**InputMessage variant routing:** `InputEvent` messages (field 34 in `SessionMessage`) carry an RFC 0004 `InputMessage` envelope. The runtime inspects the `InputMessage.event` oneof variant to determine which subscription filter applies:
- Focus variants (`FocusGainedEvent`, `FocusLostEvent`, `CaptureReleasedEvent`, IME events) are filtered by the `focus_events` subscription.
- All other variants (pointer, touch, key, gesture) are filtered by the `input_events` subscription.

An agent subscribed to `input_events` but not `focus_events` will receive pointer/key events but not focus/IME events, even though both are delivered as `InputEvent` messages on the same wire channel. This is consistent with RFC 0004 §8.5, which classifies focus/IME variants as Transactional (never dropped) and pointer-move variants as Ephemeral (coalesced under backpressure).

### 7.2 Initial Subscriptions

Declared in `SessionInit.initial_subscriptions`. Each category is filtered by the agent's granted capabilities: requesting a category for which the agent lacks the required capability is downgraded — the category is omitted from active delivery. The `SessionEstablished` response explicitly lists `active_subscriptions` (confirmed) and `denied_subscriptions` (omitted due to missing capability), so agents can detect and react to capability gaps rather than silently receiving no events (see §1.3).

### 7.3 SubscriptionChange (Mid-Session)

```protobuf
message SubscriptionChange {
  repeated SubscriptionCategory add    = 1;
  repeated SubscriptionCategory remove = 2;
}

enum SubscriptionCategory {
  SUBSCRIPTION_CATEGORY_UNSPECIFIED = 0;
  SCENE_TOPOLOGY                    = 1;
  INPUT_EVENTS                      = 2;
  FOCUS_EVENTS                      = 3;
  DEGRADATION_NOTICES               = 4;
  LEASE_CHANGES                     = 5;
  ZONE_EVENTS                       = 6;
}
```

The runtime acknowledges via a `SubscriptionChangeResult` (see §9), correlated by `sequence` number (the server's response sequence maps to the `SubscriptionChange` message's client sequence). Using `MutationResult` for this purpose would be a type-system abuse — subscription changes are not scene mutations. The new subscription set takes effect immediately after the ack is sent — events generated after that point use the new filter.

`SubscriptionChangeResult` echoes the full active subscription set after the change, so agents always have a current view of which categories are active. Denied additions (due to missing capability) appear in `denied_subscriptions`.

### 7.4 Mobile Reduced Granularity

Mobile Presence Nodes (post-v1) may negotiate reduced-granularity event delivery to conserve bandwidth. For `scene_topology`, the runtime can omit node-level detail (only tile-level changes are delivered). For `input_events`, high-frequency `POINTER_MOVE` events are decimated to `POINTER_MOVE_COALESCED` at 30Hz instead of raw event rate. Reduced granularity is negotiated at handshake via the `presence_level` field and capability profile; it is not controllable per-category by the agent.

---

## 8. MCP Bridge

### 8.1 Purpose

The MCP bridge provides a compatibility surface for guest-level LLM agents that use JSON-RPC tool calls (stdio or Streamable HTTP) rather than holding a persistent gRPC session. Guest agents cannot subscribe to events, hold surface leases, or access media streams. They perform one-off operations and disconnect.

The MCP bridge is not a simplified version of the gRPC API — it is a separate transport adapter that maps specific, LLM-optimized tool calls to the same scene mutation and zone publish operations that resident agents use. The scene model is shared; only the transport differs.

### 8.2 Transport

| Mode | Description |
|------|-------------|
| `stdio` | Runtime spawns agent as child process; JSON-RPC over stdin/stdout |
| `Streamable HTTP` | Agent connects over HTTP POST/SSE; session is per-request |

Both modes use JSON-RPC 2.0 as the message format. Neither mode supports persistent sessions, event subscriptions, or server-initiated messages (beyond SSE notifications in Streamable HTTP mode).

### 8.3 MCP Tool Set

| Tool | Parameters | Effect |
|------|-----------|--------|
| `create_tab` | `name: string` | Create a new tab |
| `create_tile` | `tab_id, bounds, z_order` | Create a tile; returns `tile_id` |
| `set_content` | `tile_id, node: NodeSpec` | Set or replace the root node of a tile |
| `dismiss` | `tile_id` | Delete a tile and release its lease |
| `list_scene` | *(none)* | Returns current scene topology as JSON |
| `publish_to_zone` | `zone_name, content, ttl_ms?, merge_key?` | Publish content to a zone |
| `list_zones` | *(none)* | Returns zone registry (names, types, current occupancy) |

Tools `publish_to_zone` and `list_zones` are the primary LLM-first surface. They require no scene context and zero tile management. See v1.md §"V1 success criteria" for the design intent.

### 8.4 Authentication Over MCP

MCP tool calls carry authentication via a header or initial JSON-RPC parameter. Pre-shared key is the primary MCP auth mechanism; OAuth2 tokens are also supported. Each tool call is authenticated independently (no persistent session).

### 8.5 Error Model Over MCP

MCP errors use the JSON-RPC 2.0 error object with structured `data`:

```json
{
  "jsonrpc": "2.0",
  "id": 42,
  "error": {
    "code": -32000,
    "message": "LEASE_EXPIRED",
    "data": {
      "error_code": "LEASE_EXPIRED",
      "message": "The tile lease has expired and must be renewed before mutation.",
      "context": "tile_id=tile-abc123",
      "hint": "{\"action\": \"renew_lease\", \"tile_id\": \"tile-abc123\"}"
    }
  }
}
```

The `data` object is the `RuntimeError` proto (§3.5) serialized as JSON. Error codes are stable and documented — the same codes used in gRPC `RuntimeError` responses are reused verbatim in MCP `data.error_code`.

### 8.6 Zone Publishing via MCP

Zone publishing is available via both protocol planes (gRPC `ZonePublish` and MCP `publish_to_zone`). When an MCP guest publishes to a zone:

- The guest does not acquire a lease. The zone's internal tile is runtime-owned (presence.md §"Guest agents and zone leases").
- Content persists until the zone's `auto_clear_ms` timeout, or until another publish replaces/extends it.
- The guest receives a success/failure response for the tool call. No events are sent to the guest (it has no subscription stream).

---

## 9. Protobuf Schema

The session protocol is defined in a new file `session.proto` in the `tze_hud.protocol.v1` package. It imports the existing `scene_service.proto` for `MutationProto`, `SceneEvent`, `InputEvent`, `RuntimeError`, and zone message types.

```protobuf
syntax = "proto3";

package tze_hud.protocol.v1;

import "scene_service.proto";

// ─── Presence ────────────────────────────────────────────────────────────────

enum PresenceLevel {
  PRESENCE_LEVEL_UNSPECIFIED = 0;
  GUEST                      = 1;
  RESIDENT                   = 2;
  EMBODIED                   = 3;  // Post-v1; reserved
}

// ─── Authentication ───────────────────────────────────────────────────────────

message PreSharedKeyCredential {
  string key_id  = 1;
  string api_key = 2;
}

message LocalSocketCredential {
  bool unix_creds = 1;
}

message OauthTokenCredential {
  string access_token = 1;
  string token_type   = 2;
}

message MtlsCredential {
  string cert_fingerprint = 1;  // SHA-256 hex fingerprint; optional, for audit log
}

message AuthCredential {
  oneof mechanism {
    PreSharedKeyCredential pre_shared_key = 1;
    LocalSocketCredential  local_socket   = 2;
    OauthTokenCredential   oauth_token    = 3;
    MtlsCredential         mtls           = 4;
  }
}

// ─── Subscriptions ───────────────────────────────────────────────────────────

enum SubscriptionCategory {
  SUBSCRIPTION_CATEGORY_UNSPECIFIED = 0;
  SCENE_TOPOLOGY                    = 1;
  INPUT_EVENTS                      = 2;
  FOCUS_EVENTS                      = 3;
  DEGRADATION_NOTICES               = 4;
  LEASE_CHANGES                     = 5;
  ZONE_EVENTS                       = 6;
}

// ─── Handshake ───────────────────────────────────────────────────────────────

message SessionInit {
  string         agent_id               = 1;
  string         agent_display_name     = 2;
  uint32         min_protocol_version   = 3;
  uint32         max_protocol_version   = 4;
  AuthCredential auth_credential        = 5;
  repeated string requested_capabilities = 6;
  repeated SubscriptionCategory initial_subscriptions = 7;
  PresenceLevel  presence_level         = 8;
  // Fields 9–10 are reserved. Resume uses SessionResume (§6.2), never SessionInit.
  reserved 9, 10;
  reserved "resume_session_token", "resume_last_seen_server_seq";
}

message SessionEstablished {
  string  session_token                  = 1;
  uint32  negotiated_protocol_version    = 2;
  repeated string granted_capabilities   = 3;
  uint64  heartbeat_interval_ms          = 4;
  string  namespace                      = 5;
  uint64  server_sequence                = 6;
  repeated SubscriptionCategory active_subscriptions = 7;
  repeated SubscriptionCategory denied_subscriptions = 8;
}

message SessionClose {
  string reason        = 1;
  bool   expect_resume = 2;
}

message SessionError {
  SessionErrorCode code    = 1;
  string           message = 2;
  string           context = 3;
  string           hint    = 4;

  enum SessionErrorCode {
    SESSION_ERROR_CODE_UNSPECIFIED = 0;
    AUTH_FAILED                    = 1;
    UNSUPPORTED_PROTOCOL_VERSION   = 2;
    CAPABILITY_NOT_GRANTED         = 3;
    HANDSHAKE_TIMEOUT              = 4;
    SESSION_NOT_FOUND              = 5;
    SESSION_GRACE_EXPIRED          = 6;
    DUPLICATE_AGENT_ID             = 7;
    INVALID_PRESENCE_LEVEL         = 8;
    SEQUENCE_GAP_EXCEEDED          = 9;
    SEQUENCE_REGRESSION            = 10;
  }
}

// ─── Resumption ──────────────────────────────────────────────────────────────

message SessionResume {
  string         agent_id                  = 1;
  string         session_token             = 2;
  uint64         last_seen_server_sequence = 3;
  AuthCredential auth_credential           = 4;
}

message SessionResumeResult {
  bool    accepted                       = 1;
  string  new_session_token              = 2;
  uint64  new_server_sequence            = 3;
  uint32  negotiated_protocol_version    = 4;
  repeated string granted_capabilities   = 5;
  RuntimeError error                     = 6;
}

// ─── State delta sentinel (server → client) ──────────────────────────────────

// Sent as the final message of the state-delta burst after session resumption
// (§6.4). Signals that all missed transactional/state-stream messages have been
// replayed. No payload fields are needed; receipt of this message is the signal.
message StateDeltaComplete {}

// ─── Heartbeat ───────────────────────────────────────────────────────────────

message HeartbeatPing {
  uint64 client_timestamp_us = 1;
}

message HeartbeatPong {
  uint64 client_timestamp_us = 1;
  uint64 server_timestamp_us = 2;
}

// ─── Capability mid-session ───────────────────────────────────────────────────

message CapabilityRequest {
  repeated string capabilities = 1;
  string          reason       = 2;   // Human-readable justification
}

message CapabilityNotice {
  repeated string granted = 1;
  repeated string revoked = 2;
  string          reason  = 3;
  uint64          effective_at_server_seq = 4;  // Change is effective after this server sequence
}

// ─── Subscription change ─────────────────────────────────────────────────────

message SubscriptionChange {
  repeated SubscriptionCategory add    = 1;
  repeated SubscriptionCategory remove = 2;
}

// ─── Subscription change result (server → client) ────────────────────────────
// Acks a SubscriptionChange. Correlated by sequence number. Replaces the
// prior practice of reusing MutationResult for this purpose (§7.3).

message SubscriptionChangeResult {
  repeated SubscriptionCategory active_subscriptions = 1;  // Full active set after the change
  repeated SubscriptionCategory denied_subscriptions = 2;  // Additions denied (missing capability)
  RuntimeError                  error                = 3;  // Set if the request was malformed
}

// ─── Mutation batch (client → server) ────────────────────────────────────────
// TimingHints imported from timing.proto (RFC 0003) in the full implementation.
// Defined inline here for completeness.

message TimingHints {
  uint64 present_at_us  = 1;   // Wall-clock (µs since epoch); 0 = present immediately
  uint64 expires_at_us  = 2;   // Wall-clock; 0 = no expiry
  string sync_group_id  = 3;   // Scene sync group (RFC 0001 §3)
}

message MutationBatch {
  string         batch_id   = 1;   // UUIDv7; deduplication key
  string         lease_id   = 2;
  repeated MutationProto mutations = 3;
  TimingHints    timing     = 4;
}

message MutationResult {
  string         batch_id     = 1;
  bool           accepted     = 2;
  repeated string created_ids = 3;  // UUIDv7 strings; type SceneId per RFC 0001 §1.1
  RuntimeError   error        = 4;
}

// ─── Zone publish (client → server) ──────────────────────────────────────────

message ZonePublish {
  string      zone_name  = 1;
  ZoneContent content    = 2;    // Imported from scene_service.proto
  uint64      ttl_ms     = 3;    // 0 = zone default; use zone's auto_clear_ms
  string      merge_key  = 4;    // For MergeByKey contention policy; empty otherwise
}

// ─── Zone publish result (server → client) ───────────────────────────────────
// Sent only for durable-zone publishes (Transactional traffic class).
// Ephemeral-zone publishes are fire-and-forget; no ZonePublishResult is sent.
// Correlated by request_sequence matching the ZonePublish envelope's sequence.

message ZonePublishResult {
  uint64       request_sequence = 1;  // Sequence of the ZonePublish that triggered this
  bool         accepted         = 2;
  RuntimeError error            = 3;  // Populated if accepted = false
}

// ─── Runtime error ───────────────────────────────────────────────────────────
// Defined here (not imported) because RuntimeError is used throughout session.proto.
// See §3.5 for the full specification.

message RuntimeError {
  string    error_code      = 1;  // String identifier (canonical, stable); e.g. "LEASE_EXPIRED"
  string    message         = 2;  // Short human-readable sentence
  string    context         = 3;  // Invalid field, value, or operation
  string    hint            = 4;  // Machine-readable correction suggestion (JSON object)
  ErrorCode error_code_enum = 5;  // Typed enum for well-known codes; preferred for programmatic use

  // Well-known error codes. String error_code is canonical; error_code_enum mirrors it.
  // Codes not in this enum are represented as ERROR_CODE_UNKNOWN; inspect error_code for detail.
  enum ErrorCode {
    ERROR_CODE_UNSPECIFIED    = 0;
    ERROR_CODE_UNKNOWN        = 1;  // Code not yet in this enum; see string error_code
    LEASE_EXPIRED             = 2;
    LEASE_NOT_FOUND           = 3;
    ZONE_TYPE_MISMATCH        = 4;
    ZONE_NOT_FOUND            = 5;
    BUDGET_EXCEEDED           = 6;
    MUTATION_REJECTED         = 7;
    PERMISSION_DENIED         = 8;
    RATE_LIMITED              = 9;
    INVALID_ARGUMENT          = 10;
    SESSION_EXPIRED           = 11;
  }
}

// ─── Degradation notice (server → client) ────────────────────────────────────

message DegradationNotice {
  DegradationLevel level                   = 1;
  string           reason                  = 2;
  repeated string  affected_capabilities   = 3;

  enum DegradationLevel {
    DEGRADATION_LEVEL_UNSPECIFIED = 0;
    NORMAL                        = 1;
    COALESCING_MORE               = 2;
    MEDIA_QUALITY_REDUCED         = 3;
    STREAMS_REDUCED               = 4;
    RENDERING_SIMPLIFIED          = 5;
    SHEDDING_TILES                = 6;
    AUDIO_ONLY_FALLBACK           = 7;
  }
}

// ─── Envelope ────────────────────────────────────────────────────────────────

message SessionMessage {
  uint64    sequence     = 1;
  uint64    timestamp_us = 2;
  oneof payload {
    // Lifecycle
    SessionInit          session_init          = 10;
    SessionEstablished   session_established   = 11;
    SessionClose         session_close         = 12;
    SessionError         session_error         = 13;
    SessionResume        session_resume        = 14;
    SessionResumeResult  session_resume_result = 15;

    // Agent → Runtime
    MutationBatch        mutation_batch        = 20;
    LeaseRequest         lease_request         = 21;  // Reuse from scene_service.proto
    HeartbeatPing        heartbeat_ping        = 22;
    CapabilityRequest    capability_request    = 23;
    SubscriptionChange   subscription_change   = 24;
    ZonePublish          zone_publish          = 25;

    // Runtime → Agent
    MutationResult          mutation_result          = 30;
    LeaseResponse           lease_response           = 31;  // Reuse from scene_service.proto
    HeartbeatPong           heartbeat_pong           = 32;
    SceneEvent              scene_event              = 33;  // Reuse from scene_service.proto
    InputEvent              input_event              = 34;  // Reuse from scene_service.proto
    DegradationNotice       degradation_notice       = 35;
    RuntimeError            runtime_error            = 36;  // Defined in session.proto (§3.5)
    CapabilityNotice        capability_notice        = 37;
    StateDeltaComplete      state_delta_complete     = 38;  // Sentinel: end of resume delta burst (§6.4)
    SubscriptionChangeResult subscription_change_result = 39;  // Ack for SubscriptionChange (§7.3)
    ZonePublishResult       zone_publish_result      = 40;  // Ack for durable ZonePublish (§3.1)
    // Fields 41–49 reserved for future server→client messages.
    // Fields 50–99 reserved for future use.
  }
}

// ─── gRPC service ────────────────────────────────────────────────────────────

service SessionService {
  // Primary bidirectional session stream.
  // All session traffic (handshake, mutations, events, heartbeats) flows here.
  rpc Session(stream SessionMessage) returns (stream SessionMessage);
}
```

### 9.1 Import Graph

```
session.proto
  ├── defines: RuntimeError (§3.5), SessionMessage envelope, all session lifecycle messages
  └── imports scene_service.proto
        └── defines: MutationProto, ZoneContent, SceneEvent, InputEvent,
                     LeaseRequest, LeaseResponse
```

`timing.proto` (RFC 0003) is imported for `TimingHints` in the full implementation; the inline definition above is provided for completeness during the pre-code draft phase. **Normative note:** if the inline `TimingHints` definition and the `timing.proto` definition in RFC 0003 ever diverge, RFC 0003 is authoritative. Implementers should flag any divergence for correction before the pre-code phase ends.

### 9.2 Field Number Reservation

Field numbers 10–29 in `SessionMessage.payload` are reserved for lifecycle and client→server messages; 30–49 for server→client messages. Numbers 50–99 are reserved for future use (including post-v1 embodied presence/media signaling). Do not fill gaps speculatively.

Currently allocated server→client fields: 30–38 (original), 39 (`SubscriptionChangeResult`), 40 (`ZonePublishResult`). Fields 41–49 are available for future server→client additions.

---

## 10. Configuration Parameters

The session protocol exposes the following configurable parameters in the runtime's config file:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `handshake_timeout_ms` | 5000 | Timeout for `SessionInit` arrival after stream open |
| `heartbeat_interval_ms` | 5000 | How often agents must send `HeartbeatPing` |
| `heartbeat_missed_threshold` | 3 | Number of consecutive missed heartbeats before ungraceful disconnect is declared. Disconnect timeout = `heartbeat_missed_threshold × heartbeat_interval_ms` = 15 000 ms by default. |
| `reconnect_grace_period_ms` | 30 000 | How long orphaned leases are held after disconnect |
| `retransmit_timeout_ms` | 5000 | Agent-side timeout before retransmitting unacked transactional message |
| `dedup_window_size` | 1000 | Max unique `batch_id` values held **per-session** in the deduplication window |
| `dedup_window_ttl_s` | 60 | Time-to-live for deduplication window entries |
| `max_sequence_gap` | 100 | Sequence gap that triggers stream close |
| `ephemeral_buffer_max` | 16 | Per-session max queued ephemeral messages before drop |
| `max_concurrent_resident_sessions` | 16 | Hard limit on simultaneous active resident/embodied sessions |
| `max_concurrent_guest_sessions` | 64 | Hard limit on simultaneous active MCP guest sessions |

---

## 11. Interaction with Other RFCs

| RFC | Relationship |
|-----|-------------|
| RFC 0001 (Scene Contract) | `MutationBatch` payloads are `MutationProto` lists defined in RFC 0001. Scene topology events reference `SceneId` types from RFC 0001. |
| RFC 0002 (Runtime Kernel) | The session service is a component of the runtime kernel. Lease lifecycle (grace period, revocation) is governed by RFC 0002. |
| RFC 0003 (Timing Model) | `TimingHints` in `MutationBatch` use the timestamp semantics and clock domains defined in RFC 0003. |
| RFC 0004 (Input Model) | `InputEvent` messages delivered over the session stream follow the routing and dispatch rules of RFC 0004. |

---

## 12. Open Questions

1. **Embodied session stream**: post-v1, embodied agents need a separate media signaling stream for WebRTC. Should it share the `SessionMessage` envelope with new payload variants, or be an entirely separate gRPC method? The current design reserves space (field numbers 50+) for future expansion.

2. **Session migration**: if the runtime moves to a new process (hot reload), can session tokens be transferred? Currently, runtime restart invalidates all tokens. A future "session handoff" mechanism could allow graceful runtime upgrades without agent disconnects.

3. **Multi-runtime federation**: the current model is one runtime per display node. If multiple runtime instances coordinate (e.g., a wall + a phone in the same session), session tokens would need to be federated. This is out of scope for v1.

4. **Audit log**: capability grants, revocations, and session lifecycle events are auditable per security.md. The audit log format and delivery mechanism (local file, structured syslog, telemetry stream) is deferred to the Security/Audit RFC.

5. **Embodied presence (post-v1)**: `EMBODIED = 3` in `PresenceLevel` is reserved per v1.md §"V1 explicitly defers: No embodied presence level." Embodied agents need a separate WebRTC media signaling stream. Whether to add this as new `SessionMessage.oneof` variants (fields 50+) or as a separate `rpc MediaSignaling(...)` on `SessionService` is an open design question deferred to the post-v1 Embodied Presence RFC.
