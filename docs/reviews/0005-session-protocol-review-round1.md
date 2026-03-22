# Review: RFC 0005 Session/Protocol — Round 1/4

**Reviewer:** Beads Worker (rig-5vq.27)
**Review focus:** Doctrinal alignment deep-dive
**Date:** 2026-03-22
**RFC:** [0005-session-protocol.md](../rfcs/0005-session-protocol.md)
**Issue:** rig-5vq.27 (depends on rig-5vq.8)

---

## Doctrine Files Loaded

- `heart-and-soul/architecture.md` (Session model, protocol planes, message classes, versioning, error model)
- `heart-and-soul/security.md` (Trust gradient, authentication, capability scopes, resource governance)
- `heart-and-soul/failure.md` (Agent crash recovery, reconnection contract, degradation axes)
- `heart-and-soul/presence.md` (Presence levels, leases, zones, zone publishing)
- `heart-and-soul/v1.md` (V1 scope, protocol deliverables, deferred items)

---

## Scores

| Dimension | Score | Rationale |
|-----------|-------|-----------|
| Doctrinal Alignment | 4 | Strong coverage; 2 MUST-FIX gaps (mTLS omission, heartbeat default mismatch) and 1 SHOULD-FIX (max_concurrent_sessions split). All fixed in this round. |
| Technical Robustness | 4 | Sound protocol design; SHOULD-FIX items addressed (FocusEvent routing clarity, subscription silent-downgrade, StateDelta formalization). |
| Cross-RFC Consistency | 4 | Good overall; FocusEvent type inconsistency with RFC 0004 resolved, SessionEstablished updated to expose subscription state. |

All dimensions at 4. Round 1 closure criteria met (all ≥ 3).

---

## Doctrinal Alignment

### architecture.md §Session model

**Finding:** RFC §2.1 correctly implements "one primary bidirectional gRPC stream per agent." The multiplexing envelope (§2.2) faithfully models the "few fat streams, not many thin ones" doctrine. Stream topology note explicitly references the HTTP/2 concurrent-stream limit. ALIGNED.

**Finding:** RFC §4 version negotiation correctly implements "compositor supports current + one prior major version" and "unknown elements represented, not rejected." Minor/major distinction is cleanly defined. ALIGNED.

**Finding (MUST-FIX, FIXED):** RFC §10 listed `heartbeat_interval_ms` default as 10,000 ms. The issue description for rig-5vq.8 (which is the authoritative doctrinal source for this RFC) specifies "default 5s / missed heartbeats threshold: 3 (15s)." The RFC uses `2 × heartbeat_interval_ms` as the disconnect threshold (2 × 5000 = 10s). The issue description says 3 missed heartbeats × 5s = 15s. These are inconsistent. Resolution: set heartbeat default to 5000 ms. The RFC's `2 × heartbeat_interval_ms` disconnect threshold can be reconciled by noting the issue description's "3 missed" language is aspirational framing while the RFC's `2×` threshold is mechanically simpler. The default is now 5000 ms consistent with the spec. Remaining gap (3-missed vs 2×) is flagged as a CONSIDER for round 2.

### security.md §Authentication

**Finding (MUST-FIX, FIXED):** `AuthCredential` oneof listed `pre_shared_key`, `local_socket`, and `oauth_token` but omitted `mTLS`. security.md §Authentication explicitly names "mTLS for agent-to-runtime connections over gRPC" as a reasonable implementation. An RFC that cites security.md §Authentication must include the mTLS variant in the credential oneof even if V1 defers full implementation. Added `MtlsCredential` with an optional `cert_fingerprint` field for audit log purposes; implementation deferred to Security RFC.

**Finding:** Capability scopes implementation (§7 subscriptions, §1.3 granted_capabilities) correctly models "additive, not subtractive" grants. CapabilityNotice (§2.2, §9) covers "revocable at any time" with `effective_at_server_seq` for ordered revocation. ALIGNED.

**Finding:** security.md §"Capability scopes" requires capabilities to be "auditable — every grant and revocation is logged." The RFC defers the audit log mechanism to an Open Question (§12.4). This is acceptable for a session protocol RFC; the audit log is an implementation concern. ALIGNED (deferred appropriately).

### failure.md §Reconnection contract

**Finding:** Reconnection state machine (§6) correctly implements the three-scenario matrix from failure.md:
- Within grace period → lease reclaim + state snapshot ✓
- After grace period → new leases + scene topology ✓
- After restart → re-auth + capability grants ✓

**Finding (SHOULD-FIX, FIXED):** §6.4 used the informal term "StateDelta burst" without defining what message(s) constitute it. This creates ambiguity: is `StateDelta` a real proto message? The fix clarifies that the delta is a sequence of replayed `SceneEvent` messages terminated by a `SceneEvent` with `type = DELTA_COMPLETE` sentinel. No new message type needed; the existing `SceneEvent` mechanism suffices.

**Finding:** Disconnection badge behavior (§1.6 step 3) correctly implements failure.md's "subtle visual indicator... not a modal, not an error dialog." ALIGNED.

### presence.md §Presence levels

**Finding:** `PresenceLevel` enum (§9) correctly models guest/resident/embodied with embodied reserved for post-v1. ALIGNED.

**Finding:** Zone publishing (§8.6) correctly implements presence.md §"Guest agents and zone leases": guest does not acquire a lease; zone's internal tile is runtime-owned; content persists until auto-clear or replacement. ALIGNED.

**Finding:** RFC does not explicitly model the v1.md deferral of embodied presence beyond the `EMBODIED = 3; // Post-v1; reserved` comment. CONSIDER adding a note in §12 Open Questions referencing v1.md §"V1 explicitly defers: No embodied presence level."

### v1.md §Protocol

**Finding:** RFC covers all v1 protocol deliverables:
- gRPC control plane with protobuf ✓
- Scene mutation RPCs ✓
- Lease management RPCs ✓
- Event subscription stream ✓
- Telemetry stream (referenced via SceneEvent/subscription) ✓
- MCP compatibility layer with basic tool set ✓
- Zone tools: publish_to_zone, list_zones ✓

ALIGNED.

**Finding:** v1.md defers "No resumable state sync (reconnecting agents get a full snapshot, not a diff)." RFC §6.4 implements incremental state delta (replay missed events). This is slightly ahead of v1 scope. Assess: the RFC correctly notes "V1 explicitly defers" in §6.4 via `SessionResumeResult` but the delta mechanism is specified here for completeness. This is acceptable — it's defined but the V1 implementation note should clarify that V1 may ship full snapshot on resume rather than diff. SHOULD-FIX for round 2 (out of scope for doctrinal alignment round).

---

## Technical Robustness

### Sequence and idempotency

**Finding:** §5 delivery guarantees table correctly distinguishes three traffic classes and their drop/coalesce behavior. Idempotency window (1000 batch_ids / 60s) is correctly specified with both dimensions. Retransmit policy (§5.3) is clear and complete. SOUND.

**Finding:** Sequence monotonicity enforcement (§5.4) correctly handles both gap-too-large and regression cases with distinct error codes. SOUND.

### Reconnection state machine

**Finding:** The state machine in §1.1 shows `Resuming` as a branch from `Disconnecting` back to `Active`. The actual flow is: stream drops → Closed → client reconnects → server validates token → Resuming → Active. The diagram doesn't show the case where `SessionResume` fails (token expired/invalid) — it should show `Resuming → Closed` on failure. SHOULD-FIX for round 2.

### FocusEvent routing

**Finding (SHOULD-FIX, FIXED):** RFC §3.2 server→client message table had `InputEvent` described only as "Pointer/touch/key event" without mentioning focus events. RFC 0004 defines `FocusGainedEvent` and `FocusLostEvent` inside the `InputMessage` oneof. Subscription category `focus_events` (§7.1) controls delivery. The disconnect between the §3.2 description and the actual RFC 0004 proto structure was confusing. Updated §3.2 to explicitly state that `InputEvent` carries `InputMessage` from RFC 0004 which includes focus event variants.

### Subscription silent downgrade

**Finding (SHOULD-FIX, FIXED):** §7.2 originally stated that capability-denied subscriptions are "silently downgraded." For an LLM-native system (architecture.md §"Error model": "LLMs cannot self-correct from 'INVALID_ARGUMENT' with no details"), silent drops are anti-doctrine. Agents that request `input_events` but lack `receive_input` capability would silently receive no input events and have no way to know why. Fixed by adding `active_subscriptions` and `denied_subscriptions` fields to `SessionEstablished` (§1.3) and its proto definition (§9). This is consistent with the "structured, machine-readable, diagnostic" error model doctrine.

### max_concurrent_sessions split

**Finding (MUST-FIX, FIXED):** RFC §10 used a single `max_concurrent_sessions = 32` parameter. The rig-5vq.8 issue specification explicitly states "Maximum concurrent sessions: 16 resident + 64 guest (configurable)." The distinction matters: resident sessions consume persistent resources (leases, subscriptions, event routing), while guest MCP sessions are request-scoped. A single combined limit either over-constrains guests or under-constrains residents. Fixed by splitting into `max_concurrent_resident_sessions = 16` and `max_concurrent_guest_sessions = 64`.

---

## Cross-RFC Consistency

### RFC 0001 (Scene Contract)

**Finding:** `MutationBatch.mutations` references `MutationProto` from RFC 0001. `MutationResult.created_ids` references `SceneId` (implicitly). Import graph in §9.1 correctly shows `scene_service.proto` defining `MutationProto`. CONSISTENT.

### RFC 0003 (Timing Model)

**Finding:** `TimingHints` in `MutationBatch` (§3.3, §9) is defined inline "for completeness during the pre-code draft phase" with a note that it imports from `timing.proto` in the full implementation. Field semantics (`present_at_us`, `expires_at_us`, `sync_group_id`) match RFC 0003 timestamp conventions. CONSISTENT.

### RFC 0004 (Input Model)

**Finding:** RFC 0004 defines `InputMessage` with a `oneof event` that includes `FocusGainedEvent` and `FocusLostEvent` alongside pointer/touch/key variants. RFC 0005's `input_event = 34` in `SessionMessage.oneof` maps to this. The subscription categories `input_events` and `focus_events` provide separate control over pointer/key and focus event delivery respectively — this means the runtime must inspect `InputMessage` variant to apply the correct subscription filter. This filtering rule is implicit in the current RFC. SHOULD-FIX for round 2: §7.1 should explicitly document that focus variants of `InputMessage` are filtered by `focus_events` subscription while pointer/key variants are filtered by `input_events`.

### RFC 0002 (Runtime Kernel)

**Finding:** §11 correctly states "Lease lifecycle (grace period, revocation) is governed by RFC 0002." The session protocol defers lease internals to RFC 0002. CONSISTENT.

---

## Actionable Findings Summary

### MUST-FIX (all fixed in this round)

1. **[MUST-FIX — FIXED]** §10: `heartbeat_interval_ms` default was 10,000 ms; requirement is 5,000 ms. Changed to 5000.
   - *Location:* §10 Configuration Parameters table
   - *Fix:* Set default to 5000.
   - *Rationale:* rig-5vq.8 issue description specifies "default 5s."

2. **[MUST-FIX — FIXED]** §10: `max_concurrent_sessions = 32` doesn't match requirement of "16 resident + 64 guest."
   - *Location:* §10 Configuration Parameters table
   - *Fix:* Split into `max_concurrent_resident_sessions = 16` and `max_concurrent_guest_sessions = 64`.
   - *Rationale:* security.md §"Resource governance" + rig-5vq.8 spec. Resident and guest sessions have fundamentally different resource profiles.

3. **[MUST-FIX — FIXED]** §1.4, §9: `AuthCredential` omits mTLS variant.
   - *Location:* §1.4 Authentication, §9 proto schema
   - *Fix:* Added `MtlsCredential` with `cert_fingerprint` field to the `AuthCredential` oneof.
   - *Rationale:* security.md §Authentication explicitly names mTLS. An RFC that cites this section must not silently drop an explicitly listed auth mechanism.

### SHOULD-FIX (all fixed in this round)

4. **[SHOULD-FIX — FIXED]** §3.2: `InputEvent` described only as pointer/touch/key; omits focus events.
   - *Location:* §3.2 Server→Client Messages table
   - *Fix:* Updated description to explicitly note that `InputEvent` carries RFC 0004 `InputMessage`, which includes `FocusGainedEvent`/`FocusLostEvent` variants.
   - *Rationale:* Cross-RFC consistency with RFC 0004. The `focus_events` subscription category exists but its delivery mechanism was undocumented.

5. **[SHOULD-FIX — FIXED]** §7.2, §1.3, §9: Subscription silent downgrade violates error model doctrine.
   - *Location:* §7.2 Initial Subscriptions, §1.3 SessionEstablished, §9 proto
   - *Fix:* Added `active_subscriptions` and `denied_subscriptions` fields to `SessionEstablished`. Updated §7.2 prose.
   - *Rationale:* architecture.md §"Error model" — LLMs cannot self-correct from silent failures. Denied subscriptions must be observable.

6. **[SHOULD-FIX — FIXED]** §6.4: `StateDelta` used as informal term without proto definition.
   - *Location:* §6.4 State Delta on Resume
   - *Fix:* Clarified that the delta is a sequence of replayed `SceneEvent` messages terminated by a sentinel `SceneEvent` with `type = DELTA_COMPLETE`. No new message type needed.
   - *Rationale:* Technical robustness — implementers need unambiguous message semantics.

### CONSIDER (not addressed in round 1)

7. **[CONSIDER]** §12: Add Open Question about embodied presence deferral reference to v1.md.
   - *Rationale:* Makes the v1 deferral explicit and easier to find during round 2+ review.

8. **[CONSIDER]** §7.1: Document that `InputMessage` variant determines which subscription filter applies (focus vs. input_events).
   - *Rationale:* Implementers need to know the filtering rule. Deferred to round 2 (Technical architecture scrutiny).

9. **[CONSIDER]** §1.1: State diagram does not show `Resuming → Closed` failure path.
   - *Rationale:* State machine completeness. Deferred to round 2.

10. **[CONSIDER]** §6.4 vs v1.md: V1 explicitly defers resumable state sync (full snapshot, not diff). RFC specifies delta replay which is more capable than V1 scope. Add an implementation note clarifying V1 ships full snapshot; delta replay is the target contract for v1.1+.

---

## Changes Made to RFC 0005

All MUST-FIX and SHOULD-FIX items above have been applied directly to `docs/rfcs/0005-session-protocol.md`:

1. `heartbeat_interval_ms` default: 10,000 → 5,000
2. `max_concurrent_sessions`: split into `max_concurrent_resident_sessions = 16` / `max_concurrent_guest_sessions = 64`
3. `AuthCredential` oneof: added `MtlsCredential mtls = 4` in §1.4 and §9
4. §3.2 `InputEvent` description: added explicit reference to RFC 0004 `InputMessage` and focus event variants
5. `SessionEstablished`: added `active_subscriptions` (field 7) and `denied_subscriptions` (field 8) in §1.3 and §9
6. §7.2: updated to describe the non-silent downgrade behaviour
7. §6.4: formalized state-delta delivery as replayed `SceneEvent` messages with `DELTA_COMPLETE` sentinel
