# Epic 6: Session Protocol

> **Dependencies:** Epic 0 (protocol conformance tests), Epic 1 (scene types), Epic 3 (clock sync), Epic 4 (lease messages)
> **Depended on by:** Epics 7, 8, 9, 10, 11 (config negotiation, policy evaluation, events, resources, shell signaling)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
> **Secondary specs:** `timing-model/spec.md` (clock domains), `lease-governance/spec.md` (lease lifecycle)

## Prompt

Create a `/beads-writer` epic for **session protocol** — the bidirectional gRPC streaming protocol that is the single authoritative resident control path. This is the largest and most complex spec in the package.

### Context

The session protocol defines the HudSession gRPC service with one bidirectional stream per agent. The existing `crates/tze_hud_protocol/` has proto definitions (`types.proto`, `events.proto`, `session.proto`), a partial `session_server.rs`, and tonic-generated code. Epic 0 provides protocol conformance tests (message round-trips, state machine transitions, capability gating). The canonical v1 message vocabulary is: SessionInit/SessionEstablished/SessionError/SessionResumeResult/SessionClose, LeaseRequest/LeaseResponse/LeaseStateChange, MutationBatch/MutationResult, SubscriptionChange/SubscriptionChangeResult, ZonePublish/ZonePublishResult, Heartbeat, EventBatch, and control messages (InputFocusRequest/Response, CapabilityRequest/CapabilityNotice, SessionSuspended/SessionResumed).

### Epic structure

Create an epic with **7 implementation beads**:

#### 1. Session lifecycle state machine (depends on Epic 0 conformance tests)
Implement the session state machine per `session-protocol/spec.md` Requirement: Session Lifecycle State Machine.
- States: Connecting → Handshaking → Active ⇄ Disconnecting → Closed → Resuming
- SessionInit triggers Connecting→Handshaking; valid auth leads to SessionEstablished
- Invalid auth leads to SessionError(code=AUTH_FAILED) and stream close
- Graceful close via SessionClose; ungraceful via heartbeat timeout
- **Acceptance:** All session state machine conformance tests from Epic 0 pass. Every legal transition verified. Every illegal transition rejected.
- **Spec refs:** `session-protocol/spec.md` Requirement: Session Lifecycle State Machine

#### 2. Handshake, auth, and capability negotiation (depends on #1)
Implement the handshake flow per `session-protocol/spec.md` Requirement: SessionEstablished Response, Requirement: Authentication.
- SessionInit carries agent_id, pre_shared_key, requested_capabilities, initial_subscriptions
- Runtime authenticates, negotiates capabilities, returns SessionEstablished with granted_capabilities, session_token, namespace
- Capabilities explicitly gated — not granted unconditionally
- Followed immediately by SceneSnapshot
- **Acceptance:** Auth success/failure scenarios pass. Capability negotiation returns correct grants/denials. SceneSnapshot delivered after SessionEstablished.
- **Spec refs:** `session-protocol/spec.md` Requirement: SessionEstablished Response, Requirement: Authentication, Requirement: SceneSnapshot After SessionEstablished

#### 3. Mutation transport and deduplication (depends on #2, Epic 1 batches)
Implement mutation flow per `session-protocol/spec.md` Requirement: Mutation Transport.
- MutationBatch carries batch_id (UUIDv7), lease_id, ordered mutations
- MutationResult returned with accepted/rejected, created_ids, error details
- Deduplication: duplicate batch_id within 60s returns cached result without re-applying
- Transactional: MutationResult never dropped under backpressure
- **Acceptance:** Mutation round-trip scenarios pass. Deduplication verified. Backpressure doesn't drop results.
- **Spec refs:** `session-protocol/spec.md` Requirement: Mutation Transport, Requirement: Deduplication

#### 4. Lease management over session stream (depends on #2, Epic 4 state machine)
Implement lease RPCs per `session-protocol/spec.md` Requirement: Lease Management.
- LeaseRequest(action=ACQUIRE/RENEW/RELEASE) → LeaseResponse (grant/deny)
- LeaseStateChange notifications for all lease transitions
- All lease_id fields use SceneId (16-byte UUIDv7)
- **Acceptance:** Lease acquire/renew/release scenarios pass. LeaseStateChange notifications delivered. Protocol conformance tests from Epic 0 pass.
- **Spec refs:** `session-protocol/spec.md` Requirement: Lease Management, `lease-governance/spec.md`

#### 5. Subscription and event delivery (depends on #2)
Implement subscription management per `session-protocol/spec.md` Requirement: Subscription Management.
- Categories: SCENE_TOPOLOGY, INPUT_EVENTS, FOCUS_EVENTS, DEGRADATION_NOTICES (mandatory), LEASE_CHANGES (mandatory), ZONE_EVENTS, TELEMETRY_FRAMES, ATTENTION_EVENTS, AGENT_EVENTS
- SubscriptionChange → SubscriptionChangeResult with active/denied sets
- Category filtering by granted capabilities
- **Acceptance:** Subscription grant/deny scenarios pass. Mandatory subscriptions always active. Capability-filtered categories correctly denied.
- **Spec refs:** `session-protocol/spec.md` Requirement: Subscription Management, lines 445-452

#### 6. Reconnection and resume (depends on #1, #2)
Implement session resume per `session-protocol/spec.md` Requirement: Session Resume.
- SessionResume with resume_token within grace period (default 30s)
- Full SceneSnapshot on reconnect (no delta replay in v1)
- SessionResumeResult with new token, restored capabilities
- Token single-use, bound to agent_id, not persisted across restarts
- **Acceptance:** Reconnect within grace succeeds. Expired token rejected with SessionError(SESSION_GRACE_EXPIRED). Full snapshot delivered on resume.
- **Spec refs:** `session-protocol/spec.md` Requirement: Session Resume, Requirement: SessionResumeResult Response

#### 7. MCP bridge: guest and resident tools (depends on #2, #5)
Implement MCP bridge per `session-protocol/spec.md` Requirement: MCP Bridge Guest Tools, Requirement: MCP Bridge Resident Tools.
- Guest tools (unconditional): publish_to_zone, list_zones, list_scene
- Resident tools (require resident_mcp): create_tab, create_tile, set_content, dismiss
- Guest denied resident tools with CAPABILITY_REQUIRED structured error
- Guest sessions not promotable to resident
- **Acceptance:** Guest tool success without capability. Resident tool rejection without capability. Structured error format matches spec. Escalation denied.
- **Spec refs:** `session-protocol/spec.md` Requirement: MCP Bridge Guest Tools, lines 487-510, Requirement: MCP Bridge Resident Tools

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `session-protocol/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference exact spec scenarios
3. **Acceptance criteria** — which Epic 0 conformance tests must pass
4. **Crate/file location** — `crates/tze_hud_protocol/src/`
5. **Message vocabulary** — use canonical v1 names only (SessionEstablished, not SessionAccepted)

### Dependency chain

```
Epics 0+1+3+4 ──→ #1 State Machine ──→ #2 Handshake/Auth ──→ #3 Mutations
                                                           ──→ #4 Leases
                                                           ──→ #5 Subscriptions
                                                           ──→ #6 Reconnection
                                                           ──→ #7 MCP Bridge
```
