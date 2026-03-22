# Session Protocol Specification

Source: RFC 0005 (Session/Protocol)
Domain: Hot Path
Depends on: scene-graph, runtime-kernel, timing-model, input-model

---

## ADDED Requirements

### Requirement: Single Bidirectional Stream Per Agent
Each resident agent SHALL hold exactly one primary bidirectional gRPC stream of type `stream SessionMessage / stream SessionMessage`. All scene mutations, event subscriptions, lease management, heartbeats, and telemetry SHALL be multiplexed over this single stream. The runtime SHALL NOT proliferate per-concern streams.
Source: RFC 0005 §2.1, DR-SP1
Scope: v1-mandatory

#### Scenario: All traffic on one stream
- **WHEN** a resident agent establishes a session
- **THEN** mutations, lease requests, heartbeats, input events, scene events, and subscription changes SHALL all be delivered on the single bidirectional SessionMessage stream

#### Scenario: No additional streams per concern
- **WHEN** an agent needs to send both MutationBatch and HeartbeatPing messages
- **THEN** both SHALL be sent as SessionMessage payloads on the same gRPC stream, not on separate streams

---

### Requirement: Legacy unary service exclusion
The v1 runtime MUST NOT host a unary RPC scene service alongside the streaming session protocol. The bidirectional streaming `HudSession` service is the single authoritative resident control path. Any legacy unary service definitions (e.g., `SceneService` with `Connect`/`RequestLease` unary RPCs) MUST be removed from the v1 codebase and protobuf definitions before release.
Source: RFC 0005 §1.1, heart-and-soul/architecture.md (one stream per agent)
Scope: v1-mandatory

#### Scenario: No unary scene service in v1 binary
- **WHEN** the v1 runtime binary starts
- **THEN** it MUST NOT register or serve any unary RPC scene service; only the `HudSession` bidirectional streaming service SHALL be available

#### Scenario: Legacy proto definitions removed
- **WHEN** building the v1 protobuf definitions
- **THEN** the build MUST NOT include legacy unary service definitions such as `SceneService`; only `HudSession` and its message types SHALL be compiled

---

### Requirement: Session Lifecycle State Machine
The session SHALL progress through six states: Connecting, Handshaking, Active, Disconnecting, Closed, and Resuming. Connecting SHALL represent TCP/TLS establishment. Handshaking SHALL represent SessionInit validation. Active SHALL represent the open bidirectional stream. Disconnecting SHALL represent graceful close. Closed SHALL represent stream termination (with orphaned leases if previously Active). Resuming SHALL represent reconnection within the grace period. Valid transitions SHALL include: Connecting to Handshaking, Handshaking to Active or Closed (on auth failure), Active to Disconnecting or Closed (on ungraceful disconnect), Disconnecting to Closed, Closed to Resuming (within grace period), Resuming to Active (accepted) or Closed (token expired/invalid).
Source: RFC 0005 §1.1
Scope: v1-mandatory

#### Scenario: Successful session establishment
- **WHEN** an agent opens a gRPC stream and sends a valid SessionInit within the handshake timeout
- **THEN** the session SHALL transition Connecting to Handshaking to Active, and the runtime SHALL respond with SessionEstablished

#### Scenario: Auth failure during handshake
- **WHEN** an agent sends SessionInit with invalid credentials
- **THEN** the session SHALL transition from Handshaking to Closed, and the runtime SHALL send SessionError with code=AUTH_FAILED

---

### Requirement: SessionInit Handshake
The first message an agent sends on a new stream SHALL be SessionInit. It MUST arrive within handshake_timeout_ms (default: 5000ms) or the runtime SHALL close the stream with DEADLINE_EXCEEDED. SessionInit SHALL carry: agent_id, agent_display_name, min/max_protocol_version, auth_credential, requested_capabilities (from RFC 0006 §6.3), initial_subscriptions, presence_level, and agent_timestamp_wall_us (for clock sync per RFC 0003 §1.3).
Source: RFC 0005 §1.2
Scope: v1-mandatory

#### Scenario: Handshake timeout
- **WHEN** an agent opens a stream but does not send SessionInit within 5000ms
- **THEN** the runtime SHALL close the stream with DEADLINE_EXCEEDED error

#### Scenario: Clock sync on handshake
- **WHEN** an agent includes agent_timestamp_wall_us in SessionInit
- **THEN** the runtime SHALL compute an initial clock-skew estimate and return it as estimated_skew_us in SessionEstablished

---

### Requirement: SessionEstablished Response
The runtime SHALL respond to a valid SessionInit with SessionEstablished containing: session_token (opaque, for resume), negotiated_protocol_version, granted_capabilities, heartbeat_interval_ms, namespace (agent's scene namespace per RFC 0001 §1.2), server_sequence (starting server-side sequence number), active_subscriptions (confirmed), denied_subscriptions (requested but denied due to missing capability), compositor_timestamp_wall_us, and estimated_skew_us.
Source: RFC 0005 §1.3
Scope: v1-mandatory

#### Scenario: Denied subscriptions reported
- **WHEN** an agent requests input_events subscription but lacks the access_input_events capability
- **THEN** SessionEstablished SHALL include input_events in denied_subscriptions and SHALL NOT include it in active_subscriptions

---

### Requirement: SceneSnapshot After SessionEstablished
Immediately after SessionEstablished, the runtime SHALL send a SceneSnapshot message containing the current scene topology. Agents MUST wait for the SceneSnapshot before acting on scene state or issuing mutations.
Source: RFC 0005 §1.3, §6.4, §6.5
Scope: v1-mandatory

#### Scenario: Agent receives scene state on connect
- **WHEN** a new agent session is established
- **THEN** the runtime SHALL send SceneSnapshot (imported from RFC 0001 §7.1) immediately after SessionEstablished, before any incremental SceneEvent updates

---

### Requirement: Authentication
Authentication SHALL be evaluated synchronously during handshake before SessionEstablished is sent. The AuthCredential oneof SHALL support: PreSharedKeyCredential, LocalSocketCredential, OauthTokenCredential, and MtlsCredential. V1 SHALL ship pre-shared key and local socket implementations. If authentication fails, the runtime SHALL send SessionError and close the stream.
Source: RFC 0005 §1.4, DR-SP6
Scope: v1-mandatory

#### Scenario: Pre-shared key authentication
- **WHEN** an agent sends SessionInit with a valid PreSharedKeyCredential
- **THEN** the runtime SHALL authenticate the agent and proceed to SessionEstablished

#### Scenario: Authentication failure closes stream
- **WHEN** an agent sends SessionInit with an invalid auth credential
- **THEN** the runtime SHALL send SessionError(code=AUTH_FAILED) and close the stream without sending SessionEstablished

---

### Requirement: Graceful Disconnect
An agent SHALL be able to initiate graceful shutdown by sending SessionClose with an optional reason and expect_resume hint. If expect_resume is true, the runtime SHALL hold leases at the full grace period. If false, the runtime MAY accelerate cleanup. The grace period SHALL start on stream close.
Source: RFC 0005 §1.5
Scope: v1-mandatory

#### Scenario: Graceful close with resume hint
- **WHEN** an agent sends SessionClose(expect_resume=true, reason="updating")
- **THEN** the runtime SHALL hold the agent's leases for the full reconnect_grace_period_ms (default: 30000ms)

---

### Requirement: Ungraceful Disconnect Detection
When the stream drops without a SessionClose, the runtime SHALL detect disconnection via gRPC stream EOF, RST, or heartbeat timeout (heartbeat_missed_threshold x heartbeat_interval_ms; default: 3 x 5000ms = 15000ms). The runtime SHALL mark the agent's leases as orphaned (rendered frozen at last known state), display a disconnection badge on affected tiles, and start the reconnection grace period (default: 30000ms).
Source: RFC 0005 §1.6, DR-SP2
Scope: v1-mandatory

#### Scenario: Heartbeat timeout triggers disconnect
- **WHEN** the runtime does not receive a HeartbeatPing for 15 seconds (3 x 5000ms)
- **THEN** the session SHALL be marked as ungracefully disconnected, leases SHALL enter orphaned state, and a disconnection badge SHALL appear on affected tiles

---

### Requirement: SessionMessage Envelope
Every message on the session stream SHALL be wrapped in a SessionMessage envelope containing: sequence (per-direction monotonically increasing, starting at 1), timestamp_wall_us (sender wall-clock, advisory only), and a oneof payload. Client-to-server fields SHALL be allocated at 10-29 (lifecycle at 10-15, agent operations at 20-29). Server-to-client fields SHALL be allocated at 30-46 (responses and events). Fields 47-49 SHALL be reserved for future server-to-client messages. Fields 50-99 SHALL be reserved for post-v1 use.
Source: RFC 0005 §2.2, §9.2
Scope: v1-mandatory

#### Scenario: Sequence numbers monotonically increase
- **WHEN** an agent sends three SessionMessages
- **THEN** the sequence numbers SHALL be 1, 2, 3 respectively

#### Scenario: All payloads fit the envelope
- **WHEN** a MutationBatch needs to be sent
- **THEN** it SHALL be wrapped in a SessionMessage with a sequence number, timestamp_wall_us, and the mutation_batch field (20) set

---

### Requirement: Sequence Number Validation
Both directions SHALL maintain independent monotonically increasing sequence counters starting at 1. The runtime SHALL validate that client-side sequence numbers are monotonically increasing. A gap larger than max_sequence_gap (default: 100) SHALL cause the runtime to close the stream with SEQUENCE_GAP_EXCEEDED. A sequence regression (lower number than previously seen) SHALL be rejected with SEQUENCE_REGRESSION.
Source: RFC 0005 §2.3, §5.4
Scope: v1-mandatory

#### Scenario: Sequence gap exceeds threshold
- **WHEN** the client sends sequence 5 followed by sequence 150 (gap of 145, exceeding max_sequence_gap=100)
- **THEN** the runtime SHALL close the stream with SessionError(code=SEQUENCE_GAP_EXCEEDED)

#### Scenario: Sequence regression rejected
- **WHEN** the client sends sequence 10 followed by sequence 8
- **THEN** the runtime SHALL close the stream with SessionError(code=SEQUENCE_REGRESSION)

---

### Requirement: Clock Domain Naming Convention
All timestamp fields in the session protocol SHALL encode their clock domain explicitly via suffix: _wall_us for wall-clock (UTC microseconds since Unix epoch) and _mono_us for monotonic system clock (microseconds since arbitrary epoch). This convention SHALL be consistent across all session messages including SessionMessage.timestamp_wall_us, SessionInit.agent_timestamp_wall_us, HeartbeatPing.client_timestamp_mono_us, HeartbeatPong.server_timestamp_wall_us, and TimingHints.present_at_wall_us/expires_at_wall_us.
Source: RFC 0005 §2.4
Scope: v1-mandatory

#### Scenario: RTT measurement uses monotonic clock
- **WHEN** the agent sends HeartbeatPing with client_timestamp_mono_us
- **THEN** the HeartbeatPong SHALL echo the same client_timestamp_mono_us value (monotonic), and the agent SHALL compute RTT as current_monotonic - echoed_value, not using wall-clock fields

---

### Requirement: Backpressure Handling
The session stream SHALL use HTTP/2 flow control as the primary backpressure mechanism. State-stream messages SHALL be coalesced when the client is not reading fast enough (coalesce-key merging). Ephemeral realtime messages SHALL be dropped (oldest first, latest-wins) when the send buffer reaches the per-session ephemeral quota (default: 16 messages). Transactional messages SHALL NEVER be dropped; if the send buffer is full, HTTP/2 backpressure SHALL be applied and the agent MUST drain its receive buffer.
Source: RFC 0005 §2.5
Scope: v1-mandatory

#### Scenario: Ephemeral messages dropped under pressure
- **WHEN** the agent is slow to consume and 20 ephemeral cursor-trail messages queue up (exceeding the 16-message quota)
- **THEN** the oldest ephemeral messages SHALL be dropped, retaining only the latest 16

#### Scenario: Transactional messages never dropped
- **WHEN** the send buffer is full and a MutationResult needs to be sent
- **THEN** the MutationResult SHALL NOT be dropped; HTTP/2 backpressure SHALL be applied instead

---

### Requirement: Traffic Class Routing
Messages SHALL be classified into three traffic classes with distinct delivery guarantees. Transactional messages (MutationBatch, LeaseRequest, CapabilityRequest, SubscriptionChange, InputFocusRequest, InputCaptureRequest) SHALL use at-least-once delivery with ack and retransmit, per-direction sequence order, and SHALL never be dropped. State-stream messages (SceneEvent, TelemetryFrame, ephemeral ZonePublish) SHALL use at-least-once delivery with coalescing and sequence order, but intermediate states MAY be skipped. Ephemeral realtime messages (HeartbeatPing, pointer/key input events, SetImePosition) SHALL use at-most-once delivery with best-effort ordering and MAY be dropped under backpressure.
Source: RFC 0005 §3.1, §3.2, §5.1
Scope: v1-mandatory

#### Scenario: MutationBatch acknowledged reliably
- **WHEN** an agent sends a MutationBatch
- **THEN** the runtime SHALL respond with a MutationResult (accepted or rejected); if no response arrives within retransmit_timeout_ms, the agent SHALL retransmit

#### Scenario: Input events droppable under backpressure
- **WHEN** pointer/key input events are classified as ephemeral realtime and the agent's buffer is full
- **THEN** the oldest non-transactional input events MAY be dropped

---

### Requirement: Heartbeat Protocol
The agent SHALL send HeartbeatPing at the interval specified in SessionEstablished.heartbeat_interval_ms (default: 5000ms). The runtime SHALL respond with HeartbeatPong echoing the client's monotonic timestamp and including the server's wall-clock timestamp. The runtime SHALL treat the session as ungracefully disconnected when heartbeat_missed_threshold (default: 3) consecutive pings are missed, resulting in a 15000ms grace window.
Source: RFC 0005 §3.6
Scope: v1-mandatory

#### Scenario: Heartbeat exchange
- **WHEN** the agent sends HeartbeatPing(client_timestamp_mono_us=12345)
- **THEN** the runtime SHALL respond with HeartbeatPong(client_timestamp_mono_us=12345, server_timestamp_wall_us=<current_wall_time>)

#### Scenario: Missed heartbeats trigger disconnect
- **WHEN** 3 consecutive heartbeat intervals pass without a HeartbeatPing from the agent
- **THEN** the runtime SHALL declare the session ungracefully disconnected and begin the reconnection grace period

---

### Requirement: Lease Management RPCs
LeaseRequest (ACQUIRE/RENEW/RELEASE) SHALL be a transactional message on the session stream. The runtime SHALL respond with LeaseResponse (grant/deny/revoke). LeaseStateChange notifications SHALL be delivered to the agent when lease state changes occur. All lease_id fields SHALL use SceneId (16-byte UUIDv7).
Source: RFC 0005 §3.1, §3.2
Scope: v1-mandatory

#### Scenario: Lease acquisition via session stream
- **WHEN** an agent sends LeaseRequest(action=ACQUIRE) on the session stream
- **THEN** the runtime SHALL respond with LeaseResponse on the same stream indicating grant or denial

---

### Requirement: MutationBatch Processing
MutationBatch SHALL carry: batch_id (SceneId UUIDv7, for deduplication), lease_id (SceneId, governing lease), repeated MutationProto (ordered, atomic per RFC 0001 §4), and optional TimingHints (present_at_wall_us, expires_at_wall_us). The runtime SHALL respond with MutationResult containing batch_id, accepted flag, created_ids (SceneIds for new tiles/nodes), and error details if rejected.
Source: RFC 0005 §3.3
Scope: v1-mandatory

#### Scenario: Atomic mutation batch
- **WHEN** an agent sends a MutationBatch with 3 mutations
- **THEN** all 3 mutations SHALL be applied atomically (all succeed or all fail) and a single MutationResult SHALL be returned

#### Scenario: Created IDs returned
- **WHEN** a MutationBatch includes CreateTile and CreateNode mutations
- **THEN** MutationResult.created_ids SHALL contain the SceneIds assigned to the newly created scene objects

---

### Requirement: Batch Idempotency
The runtime SHALL maintain a per-session deduplication window for MutationBatch: 1000 unique batch_id values or 60 seconds, whichever expires first. On duplicate batch_id within the window, the runtime SHALL return the original MutationResult without re-applying mutations. After window expiry, a reappearing batch_id SHALL be treated as a new batch.
Source: RFC 0005 §5.2
Scope: v1-mandatory

#### Scenario: Duplicate batch deduplicated
- **WHEN** an agent retransmits a MutationBatch with the same batch_id within 60 seconds
- **THEN** the runtime SHALL return the cached MutationResult without re-applying the mutations

#### Scenario: Window expiry treats as new
- **WHEN** a batch_id reappears after 60 seconds
- **THEN** the runtime SHALL treat it as a new batch and apply the mutations

---

### Requirement: Retransmission Policy
Agents SHALL be responsible for retransmitting unacknowledged transactional messages. If no acknowledgement arrives within retransmit_timeout_ms (default: 5000ms), the agent SHALL resend with the same batch_id but a new sequence number. After 3 retransmits with no acknowledgement, the agent SHOULD treat the session as degraded and attempt reconnection. Lease operations, SubscriptionChange, and CapabilityRequest SHALL follow the same at-least-once retransmit pattern using sequence as the correlation key.
Source: RFC 0005 §5.3
Scope: v1-mandatory

#### Scenario: Retransmit after timeout
- **WHEN** an agent sends a MutationBatch and receives no MutationResult within 5000ms
- **THEN** the agent SHALL retransmit the same batch with the same batch_id and a new sequence number

#### Scenario: Degraded after 3 retransmits
- **WHEN** an agent retransmits 3 times with no acknowledgement
- **THEN** the agent SHOULD treat the session as degraded and initiate reconnection

---

### Requirement: DegradationNotice Delivery
The runtime SHALL send DegradationNotice to all active sessions when the degradation level changes. DegradationNotice SHALL include a DegradationLevel enum (NORMAL, COALESCING_MORE, MEDIA_QUALITY_REDUCED, STREAMS_REDUCED, RENDERING_SIMPLIFIED, SHEDDING_TILES, AUDIO_ONLY_FALLBACK), a human-readable reason, and a list of affected_capabilities. DegradationNotice SHALL be transactional (never dropped). degradation_notices subscriptions SHALL be delivered unconditionally and SHALL NOT be filterable.
Source: RFC 0005 §3.4, §7.1
Scope: v1-mandatory

#### Scenario: Degradation notice always delivered
- **WHEN** the runtime enters COALESCING_MORE degradation level
- **THEN** all active sessions SHALL receive DegradationNotice(level=COALESCING_MORE) regardless of their subscription configuration

---

### Requirement: RuntimeError Structure
All error responses SHALL follow the structured error model: error_code (string, canonical and stable), message (human-readable), context (invalid field/value), hint (machine-readable correction suggestion as JSON), and error_code_enum (typed enum for well-known codes). The well-known ErrorCode enum SHALL include: LEASE_EXPIRED, LEASE_NOT_FOUND, ZONE_TYPE_MISMATCH, ZONE_NOT_FOUND, BUDGET_EXCEEDED, MUTATION_REJECTED, PERMISSION_DENIED, RATE_LIMITED, INVALID_ARGUMENT, SESSION_EXPIRED, CLOCK_SKEW_HIGH, CLOCK_SKEW_EXCESSIVE, SAFE_MODE_ACTIVE, TIMESTAMP_TOO_OLD, TIMESTAMP_TOO_FUTURE, TIMESTAMP_EXPIRY_BEFORE_PRESENT.
Source: RFC 0005 §3.5, DR-SP5
Scope: v1-mandatory

#### Scenario: Structured error on lease expiry
- **WHEN** an agent sends a MutationBatch referencing an expired lease
- **THEN** the runtime SHALL respond with RuntimeError(error_code="LEASE_EXPIRED", error_code_enum=LEASE_EXPIRED, context="lease_id=<id>", hint=<JSON correction>)

---

### Requirement: Version Negotiation
Protocol versions SHALL follow a major.minor scheme encoded as uint32 (version = major * 1000 + minor). The agent SHALL declare min_protocol_version and max_protocol_version in SessionInit. The runtime SHALL pick the highest mutually supported version and return it in SessionEstablished.negotiated_protocol_version. If no mutual version exists, the runtime SHALL send SessionError(UNSUPPORTED_PROTOCOL_VERSION). Minor versions SHALL be additive-only (new optional fields, new oneof variants, new enum values). The runtime SHALL support the current and one prior major version simultaneously.
Source: RFC 0005 §4.1, §4.2, §4.3, DR-SP4
Scope: v1-mandatory

#### Scenario: Version negotiated successfully
- **WHEN** an agent declares min=1000, max=1001 and the runtime supports 1000-1001
- **THEN** SessionEstablished SHALL contain negotiated_protocol_version=1001

#### Scenario: No mutual version
- **WHEN** an agent declares min=2000, max=2001 and the runtime only supports 1000-1001
- **THEN** the runtime SHALL send SessionError(code=UNSUPPORTED_PROTOCOL_VERSION) and close the stream

---

### Requirement: Session Token and Reconnection Grace Period
On SessionEstablished, the runtime SHALL issue a session_token that is opaque, cryptographically random, single-use for resumption, bound to agent_id and namespace, and valid for the grace period duration (default: 30000ms from stream close). Tokens SHALL NOT be persisted across process restarts.
Source: RFC 0005 §6.1
Scope: v1-mandatory

#### Scenario: Token validity within grace period
- **WHEN** an agent disconnects and reconnects within 30 seconds with a valid session_token
- **THEN** the token SHALL be accepted for session resumption

#### Scenario: Token expired after grace period
- **WHEN** an agent disconnects and attempts to reconnect after 30 seconds
- **THEN** the token SHALL be rejected with SessionError(code=SESSION_GRACE_EXPIRED)

---

### Requirement: SessionResume Protocol
When reconnecting within the grace period, the agent SHALL send SessionResume (not SessionInit) as the first message, carrying: agent_id, session_token, last_seen_server_sequence, and auth_credential (re-authentication required even on resume). SessionResume fields 9-10 in SessionInit are reserved and SHALL NOT be used for resume.
Source: RFC 0005 §6.2
Scope: v1-mandatory

#### Scenario: Resume with SessionResume message
- **WHEN** an agent reconnects within the grace period
- **THEN** it SHALL send SessionResume as its first message, NOT SessionInit

#### Scenario: Re-authentication on resume
- **WHEN** an agent sends SessionResume
- **THEN** auth_credential SHALL be validated; invalid credentials SHALL result in SessionError(code=AUTH_FAILED)

---

### Requirement: SessionResumeResult Response
The runtime SHALL respond to a valid SessionResume with SessionResumeResult containing: accepted flag, new_session_token (new token for the resumed session), new_server_sequence, negotiated_protocol_version, granted_capabilities, active_subscriptions, denied_subscriptions, and error (if rejected). Agents MUST use the confirmed subscription state from SessionResumeResult rather than assuming their pre-disconnect set is intact.
Source: RFC 0005 §6.3
Scope: v1-mandatory

#### Scenario: Successful resume
- **WHEN** an agent sends SessionResume with a valid token within the grace period
- **THEN** the runtime SHALL respond with SessionResumeResult(accepted=true) including a new session_token and the current subscription state

---

### Requirement: Full Snapshot on Resume (V1 Reconnect)
When a resume is accepted within the grace period, the runtime SHALL send a single SceneSnapshot message carrying the current scene topology (the same mechanism used for new connections). The agent's orphaned leases SHALL be automatically reclaimed. V1 SHALL NOT implement incremental delta replay; last_seen_server_sequence is used for identity binding and lease reclaim only.
Source: RFC 0005 §6.4, DR-SP3
Scope: v1-mandatory

#### Scenario: Snapshot delivered on resume
- **WHEN** a session resume is accepted
- **THEN** the runtime SHALL send a SceneSnapshot after SessionResumeResult, and orphaned leases SHALL be restored

---

### Requirement: Post-Grace-Period Reconnect
If the grace period expires before the agent reconnects, the runtime SHALL have evicted the agent's leases and cleared its tiles. The session_token SHALL be invalid. The agent MUST perform a full re-handshake via SessionInit. After SessionEstablished, the runtime SHALL send a SceneSnapshot.
Source: RFC 0005 §6.5
Scope: v1-mandatory

#### Scenario: Post-grace reconnect requires full handshake
- **WHEN** an agent attempts to resume after the grace period has expired
- **THEN** SessionResume SHALL be rejected, and the agent MUST send a fresh SessionInit

---

### Requirement: Runtime Restart Recovery
After display node process restart, all session tokens SHALL be invalid (token store is in-memory only), all leases SHALL be gone (scene is ephemeral), and agents SHALL reconnect with SessionInit. Tab and layout configuration SHALL persist (loaded from config at startup). Agent registration and capability profiles SHALL persist (config-driven).
Source: RFC 0005 §6.6
Scope: v1-mandatory

#### Scenario: Agent reconnects after runtime restart
- **WHEN** the runtime process restarts and an agent attempts to resume with an old session_token
- **THEN** the token SHALL be rejected and the agent MUST perform a full SessionInit handshake

---

### Requirement: Subscription Management
Agents SHALL declare initial subscriptions in SessionInit.initial_subscriptions. The runtime SHALL filter each category by the agent's granted capabilities. The available SubscriptionCategory values SHALL be: SCENE_TOPOLOGY (requires read_scene_topology), INPUT_EVENTS (requires access_input_events), FOCUS_EVENTS (requires access_input_events), DEGRADATION_NOTICES (always subscribed, not filterable), LEASE_CHANGES (always subscribed, not filterable), ZONE_EVENTS (requires publish_zone:<zone>), and TELEMETRY_FRAMES (requires read_telemetry). Emitting events to unsubscribed agents SHALL be a protocol violation.
Source: RFC 0005 §7.1, §7.2, DR-SP8
Scope: v1-mandatory

#### Scenario: Subscription denied for missing capability
- **WHEN** an agent requests INPUT_EVENTS subscription but lacks the access_input_events capability
- **THEN** the subscription SHALL be denied and listed in SessionEstablished.denied_subscriptions

#### Scenario: Lease changes always delivered
- **WHEN** a lease is revoked for an agent
- **THEN** the agent SHALL receive a LeaseResponse notification regardless of its subscription configuration

---

### Requirement: Mid-Session Subscription Change
Agents SHALL be able to add or remove subscriptions mid-session via SubscriptionChange (add/remove lists of SubscriptionCategory). The runtime SHALL acknowledge with SubscriptionChangeResult (echoing full active set and denied additions). The new subscription set SHALL take effect immediately after the ack is sent. SubscriptionChangeResult SHALL NOT reuse MutationResult.
Source: RFC 0005 §7.3
Scope: v1-mandatory

#### Scenario: Add subscription mid-session
- **WHEN** an agent sends SubscriptionChange(add=[SCENE_TOPOLOGY]) and has the required capability
- **THEN** the runtime SHALL respond with SubscriptionChangeResult listing SCENE_TOPOLOGY in active_subscriptions, and the agent SHALL begin receiving SceneEvent messages

---

### Requirement: InputEvent Variant Filtering
InputEvent messages (field 34) SHALL be filtered by subscription category at the variant level. Focus variants (FocusGainedEvent, FocusLostEvent, CaptureReleasedEvent, IME events) SHALL be filtered by the focus_events subscription. All other variants (pointer, touch, key, gesture) SHALL be filtered by the input_events subscription. An agent subscribed to input_events but not focus_events SHALL receive pointer/key events but not focus/IME events.
Source: RFC 0005 §7.1
Scope: v1-mandatory

#### Scenario: Focus events filtered separately
- **WHEN** an agent is subscribed to input_events but not focus_events
- **THEN** the agent SHALL receive PointerDownEvent and KeyDownEvent but SHALL NOT receive FocusGainedEvent or FocusLostEvent

---

### Requirement: MCP Bridge Guest Tools (V1)
The v1 MCP guest tool surface SHALL be restricted to zone-centric operations: publish_to_zone (zone_name, content, ttl_us, merge_key), list_zones (returns zone registry), and list_scene (returns tab names and zone registry only, not full tile topology). Guest tools (publish_to_zone, list_zones, list_scene) MUST be available to any authenticated MCP caller without any lease grant or capability negotiation — these are unconditionally accessible. Resident tools (create_tab, create_tile, set_content, dismiss) MUST be rejected unless the calling agent has been granted the `resident_mcp` capability through the session handshake. Invoking a resident tool without the `resident_mcp` capability SHALL produce a structured JSON-RPC error with error_code CAPABILITY_REQUIRED, a context field identifying the tool, and a hint field containing `{"required_capability": "resident_mcp"}`.
Source: RFC 0005 §8.1, §8.3, DR-SP7
Scope: v1-mandatory

#### Scenario: Guest publishes to zone without capability grant
- **WHEN** a guest agent with no granted capabilities calls publish_to_zone via MCP with valid zone_name and content
- **THEN** the content SHALL be published to the zone and a success response SHALL be returned; no lease or capability grant is required

#### Scenario: Guest lists zones without capability grant
- **WHEN** a guest agent with no granted capabilities calls list_zones via MCP
- **THEN** the zone registry SHALL be returned; no lease or capability grant is required

#### Scenario: Guest lists scene without capability grant
- **WHEN** a guest agent with no granted capabilities calls list_scene via MCP
- **THEN** tab names and zone registry SHALL be returned (not full tile topology); no lease or capability grant is required

#### Scenario: Guest denied tile management
- **WHEN** a guest agent calls create_tile via MCP without resident_mcp capability
- **THEN** the runtime SHALL return PERMISSION_DENIED with hint {"required_capability": "resident_mcp"}

#### Scenario: Guest calling create_tile receives structured error
- **WHEN** a guest agent without `resident_mcp` capability calls create_tile via MCP
- **THEN** the runtime SHALL return a JSON-RPC 2.0 error response with code -32603, data.error_code="CAPABILITY_REQUIRED", data.context="tool=create_tile", and data.hint={"required_capability": "resident_mcp", "resolution": "obtain resident_mcp capability via session handshake"}

---

### Requirement: MCP Authentication
MCP tool calls SHALL carry authentication via header or initial JSON-RPC parameter. Pre-shared key SHALL be the primary MCP auth mechanism. Each tool call SHALL be authenticated independently (no persistent session).
Source: RFC 0005 §8.4
Scope: v1-mandatory

#### Scenario: Each MCP call authenticated
- **WHEN** a guest agent makes two consecutive MCP tool calls
- **THEN** each call SHALL be independently authenticated via the provided credential

---

### Requirement: MCP Error Model
MCP errors SHALL use JSON-RPC 2.0 error objects with a structured data field matching the RuntimeError proto (error_code, message, context, hint). Error codes SHALL be the same stable codes used in gRPC RuntimeError responses.
Source: RFC 0005 §8.5
Scope: v1-mandatory

#### Scenario: MCP error matches gRPC codes
- **WHEN** a MCP tool call fails with a lease expiry
- **THEN** the JSON-RPC error.data SHALL include error_code="LEASE_EXPIRED" matching the gRPC RuntimeError code

---

### Requirement: Session Configuration Parameters
The runtime SHALL expose the following configurable parameters: handshake_timeout_ms (default: 5000), heartbeat_interval_ms (default: 5000), heartbeat_missed_threshold (default: 3), reconnect_grace_period_ms (default: 30000; config key: reconnect_grace_secs in seconds), retransmit_timeout_ms (default: 5000), dedup_window_size (default: 1000 per session), dedup_window_ttl_s (default: 60), max_sequence_gap (default: 100), ephemeral_buffer_max (default: 16), max_concurrent_resident_sessions (default: 16), max_concurrent_guest_sessions (default: 64).
Source: RFC 0005 §10
Scope: v1-mandatory

#### Scenario: Max concurrent sessions enforced
- **WHEN** 16 resident sessions are active and a 17th agent attempts to connect
- **THEN** the runtime SHALL deny the 17th session based on max_concurrent_resident_sessions

---

### Requirement: SessionSuspended and SessionResumed Messages
The runtime SHALL send SessionSuspended to all active sessions when safe mode is entered (RFC 0007 §5.2). After delivery, all MutationBatch messages from suspended sessions SHALL be rejected with RuntimeError error_code="SAFE_MODE_ACTIVE". The agent's session SHALL remain open and HeartbeatPing/HeartbeatPong SHALL continue. The runtime SHALL send SessionResumed to all suspended sessions when safe mode exits. After delivery, mutation submission SHALL be permitted again. Both messages SHALL be transactional (never dropped).
Source: RFC 0005 §3.7
Scope: v1-mandatory

#### Scenario: Mutations rejected during safe mode
- **WHEN** the runtime sends SessionSuspended and an agent subsequently sends a MutationBatch
- **THEN** the runtime SHALL reject the mutation with RuntimeError(error_code="SAFE_MODE_ACTIVE")

#### Scenario: Mutations accepted after resume
- **WHEN** the runtime sends SessionResumed after a safe mode period
- **THEN** subsequent MutationBatch messages SHALL be accepted normally

---

### Requirement: Input Control Request Transport
FocusRequest, CaptureRequest, CaptureReleaseRequest, and SetImePositionRequest from RFC 0004 SHALL travel agent-to-runtime on the session stream as SessionMessage payload variants (fields 26-29). FocusResponse and CaptureResponse SHALL travel runtime-to-agent (fields 43-44). FocusRequest and CaptureRequest SHALL use synchronous request/response semantics correlated by sequence number. CaptureReleaseRequest SHALL be confirmed asynchronously by CaptureReleasedEvent in InputEvent. SetImePosition SHALL be fire-and-forget.
Source: RFC 0005 §3.8
Scope: v1-mandatory

#### Scenario: Focus request correlated by sequence
- **WHEN** an agent sends InputFocusRequest at sequence N
- **THEN** the runtime SHALL respond with InputFocusResponse correlated to the request by server response sequence

---

### Requirement: TimingHints on Mutations
MutationBatch SHALL support optional TimingHints containing present_at_wall_us (wall-clock; 0 = present immediately) and expires_at_wall_us (wall-clock; 0 = no expiry). These fields SHALL follow RFC 0003 timing semantics.
Source: RFC 0005 §3.3, RFC 0003
Scope: v1-mandatory

#### Scenario: Scheduled presentation
- **WHEN** an agent sends MutationBatch with TimingHints.present_at_wall_us set to 500ms in the future
- **THEN** the runtime SHALL defer applying the mutations until the specified wall-clock time

---

### Requirement: Capability Request Mid-Session
An agent SHALL be able to request additional capabilities mid-session via CapabilityRequest (capabilities list, reason). The runtime SHALL respond with CapabilityNotice (granted/revoked lists, reason, effective_at_server_seq) on success, or RuntimeError(PERMISSION_DENIED) on denial. At most one CapabilityRequest SHOULD be in flight per session at a time. The runtime MUST NOT grant all requested capabilities unconditionally; every capability request MUST be validated against the agent's authorization policy (pre-registered grants in configuration, dynamic agent policy, or operator approval). Capabilities not explicitly authorized for the requesting agent MUST be denied.
Source: RFC 0005 §5.3
Scope: v1-mandatory

#### Scenario: Capability granted mid-session
- **WHEN** an agent sends CapabilityRequest(capabilities=["read_telemetry"], reason="monitoring") and the agent's configuration authorizes read_telemetry
- **THEN** the runtime SHALL respond with CapabilityNotice(granted=["read_telemetry"]) and the agent MAY then subscribe to TELEMETRY_FRAMES

#### Scenario: Unauthorized capability denied
- **WHEN** an agent requests capabilities it is not authorized for (e.g., a guest agent requests overlay_privileges or high_priority_z_order)
- **THEN** the runtime MUST deny the request with RuntimeError(error_code="PERMISSION_DENIED", context="<denied capabilities>", hint={"unauthorized_capabilities": ["overlay_privileges", "high_priority_z_order"]})

#### Scenario: Guest agent denied resident tools via capability escalation
- **WHEN** a guest-level agent sends CapabilityRequest(capabilities=["create_tiles", "modify_own_tiles"]) attempting to escalate to resident-level operations
- **THEN** the runtime MUST deny the request with RuntimeError(error_code="PERMISSION_DENIED") and the agent MUST remain unable to create or modify tiles

#### Scenario: Partial grant of mixed capabilities
- **WHEN** an agent requests capabilities=["read_telemetry", "overlay_privileges"] and is authorized for read_telemetry but not overlay_privileges
- **THEN** the runtime MUST deny the entire request with RuntimeError(error_code="PERMISSION_DENIED", context="overlay_privileges") rather than silently granting only the authorized subset

---

### Requirement: Zone Publishing via Session Stream
ZonePublish SHALL carry zone_name, content (ZoneContent from scene_service.proto), ttl_us (0 = zone default), and merge_key. Durable-zone publishes SHALL be transactional and receive ZonePublishResult acknowledgement. Ephemeral-zone publishes SHALL be fire-and-forget (no ZonePublishResult sent).
Source: RFC 0005 §3.1, §8.6
Scope: v1-mandatory

#### Scenario: Durable zone publish acknowledged
- **WHEN** an agent publishes to a durable zone
- **THEN** the runtime SHALL respond with ZonePublishResult(accepted=true/false)

#### Scenario: Ephemeral zone publish fire-and-forget
- **WHEN** an agent publishes to an ephemeral zone
- **THEN** the runtime SHALL NOT send a ZonePublishResult

---

### Requirement: TelemetryFrame Delivery
The runtime SHALL send TelemetryFrame messages to sessions subscribed to TELEMETRY_FRAMES (requires read_telemetry capability). TelemetryFrame SHALL include: sample_timestamp_wall_us, compositor_frame_rate, compositor_frame_budget_us, compositor_frame_time_us, active_sessions, active_leases, heap_used_bytes, and gpu_utilization_pct. TelemetryFrame SHALL be state-stream traffic class (coalesced under backpressure, latest-wins).
Source: RFC 0005 §9, §3.2
Scope: v1-mandatory

#### Scenario: Telemetry delivered to subscribed agent
- **WHEN** an agent has the read_telemetry capability and is subscribed to TELEMETRY_FRAMES
- **THEN** the runtime SHALL periodically deliver TelemetryFrame messages with compositor performance data

---

### Requirement: SceneId for Scene-Object Identifiers
All scene-object identifiers (batch_id, lease_id, created_ids in MutationBatch/MutationResult) SHALL use SceneId (16-byte little-endian UUIDv7, imported from scene.proto). Session-level identifiers (agent_id, session_token, namespace) SHALL remain string.
Source: RFC 0005 §3.3, §9.1
Scope: v1-mandatory

#### Scenario: batch_id uses SceneId
- **WHEN** an agent creates a MutationBatch
- **THEN** batch_id SHALL be a SceneId (binary UUIDv7), not a string

---

### Requirement: Protobuf Schema in session.proto
The session protocol SHALL be defined in session.proto (package tze_hud.protocol.v1). It SHALL import scene_service.proto for MutationProto, SceneEvent, InputEvent, LeaseRequest, LeaseResponse, SceneSnapshot, and zone types. It SHALL import scene.proto for SceneId. RuntimeError SHALL be defined in session.proto itself. The gRPC service SHALL define: rpc Session(stream SessionMessage) returns (stream SessionMessage) for the primary bidirectional stream, and rpc ClockSync(ClockSyncRequest) returns (ClockSyncResponse) for ongoing clock re-synchronization.
Source: RFC 0005 §9, §9.1
Scope: v1-mandatory

#### Scenario: Single gRPC service definition
- **WHEN** an agent connects to the runtime
- **THEN** it SHALL use the SessionService with the Session RPC for all bidirectional communication

---

### Requirement: Incremental Delta Replay (Post-v1)
Incremental delta replay on reconnect (WAL-based event replay using last_seen_server_sequence) is explicitly deferred to post-v1. V1 SHALL use full SceneSnapshot on all reconnects. Field 38 (StateDeltaComplete) in SessionMessage SHALL be reserved for future delta replay.
Source: RFC 0005 §6.4
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** v1 ships
- **THEN** all reconnects SHALL use full SceneSnapshot, and StateDeltaComplete (field 38) SHALL be reserved but unused

---

### Requirement: Embodied Presence Stream (Post-v1)
Embodied agents (EMBODIED presence level) and their separate WebRTC media signaling stream are explicitly deferred to post-v1. EMBODIED=3 in PresenceLevel SHALL be reserved. Fields 50-99 in SessionMessage SHALL be reserved for post-v1 embodied presence and media signaling.
Source: RFC 0005 §12.5
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** v1 ships
- **THEN** EMBODIED presence level SHALL be reserved but not implemented, and no WebRTC media signaling stream SHALL be available

---

### Requirement: Session Migration (Post-v1)
Session migration (transferring session tokens during hot runtime reload) is explicitly deferred to post-v1. Runtime restart SHALL invalidate all tokens.
Source: RFC 0005 §12.2
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** v1 ships
- **THEN** runtime restart SHALL invalidate all session tokens; no hot-reload session handoff SHALL be supported

---

### Requirement: Promoted Guest Pattern (Post-v1)
Promoting an MCP guest session to resident-level presence by pairing it with a backing gRPC session is explicitly deferred to post-v1.
Source: RFC 0005 §8.3
Scope: post-v1

#### Scenario: Deferred marker
- **WHEN** v1 ships
- **THEN** MCP guest sessions SHALL NOT be promotable to resident-level; guests requiring tile management MUST use a full gRPC agent session
