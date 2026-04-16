# Review: RFC 0005 Session/Protocol — Round 2/4

**Reviewer:** Beads Worker (rig-5vq.28)
**Review focus:** Technical architecture scrutiny
**Date:** 2026-03-22
**RFC:** [0005-session-protocol.md](../rfcs/0005-session-protocol.md)
**Issue:** rig-5vq.28 (depends on rig-5vq.8)

---

## Doctrine Files Loaded

- `about/heart-and-soul/architecture.md` (Session model, protocol planes, message classes, error model, versioning)
- `about/heart-and-soul/v1.md` (V1 scope, protocol deliverables, deferred items)
- `about/heart-and-soul/failure.md` (Reconnection contract, degradation axes)
- `about/heart-and-soul/security.md` (Authentication, capability scopes, resource governance)

Round 1 findings reviewed and confirmed as merged via PR #24.

---

## Scores

| Dimension | Score | Rationale |
|-----------|-------|-----------|
| Doctrinal Alignment | 4 | Maintained from round 1; no regressions found. One MUST-FIX (heartbeat missed-count vs. 2× threshold reconciliation). |
| Technical Robustness | 3 | Several architectural holes: `SessionInit` resume-field conflation, unspecified `SubscriptionChange` ack response type, deduplication window eviction under concurrent sessions, sequence number integer overflow, and an incomplete reconnection state machine. All MUST-FIX and SHOULD-FIX items addressed in this round. |
| Cross-RFC Consistency | 4 | Strong after round 1 fixes. One SHOULD-FIX: subscription variant filtering rule implicit in §7.1 (carried from round 1 CONSIDER). |

---

## Doctrinal Alignment

### architecture.md §"Session model"

**Finding:** Single-stream-per-agent discipline is maintained throughout (§2.1). The HTTP/2 concurrent-stream caveat is properly documented. No new regressions from round 1.

**Finding (MUST-FIX):** §1.6 and §3.6 specify `2 × heartbeat_interval_ms` as the ungraceful disconnect threshold (2 × 5000 = 10 000 ms). The rig-5vq.8 issue description specifies "missed heartbeats threshold: 3 (15s)." Round 1 noted this discrepancy as a CONSIDER but left the 2× threshold in place. For a production protocol spec, the disconnect-detection threshold must be unambiguous. The current text leaves implementers with two contradictory numbers. Resolution: change the disconnect threshold to `3 × heartbeat_interval_ms` to match the authoritative issue spec, and update §3.6, §10, and the relevant prose in §1.6.

### architecture.md §"Error model"

**Finding:** `SessionError` enum uses nested enum `SessionErrorCode` (§1.7). In proto3, nested enums require full qualification at the usage site: `SessionError.SessionErrorCode.AUTH_FAILED`. This is valid protobuf but the RFC code examples in §1.7 reference the enum values as if they are top-level (e.g., `AUTH_FAILED = 1` without the `SESSION_ERROR_CODE_` prefix in the enum values). The enum values actually do have the prefix in the proto definition (`AUTH_FAILED = 1` but the enum is named `SessionErrorCode` — the protobuf naming convention for nested enum values omits the enum name prefix). This is technically correct protobuf but is inconsistent with how `DegradationLevel` uses `DEGRADATION_LEVEL_UNSPECIFIED` as its zero-value prefix. CONSIDER aligning all enum zero-value naming convention.

**Finding:** `RuntimeError.error_code` is typed `string` (§3.5, §9) rather than an enum. The architecture.md §"Error model" requires "A stable, enumerated identifier." Using `string` defeats stability — there is no protobuf guarantee against typos, renaming, or case drift. SHOULD-FIX: At minimum the RFC should define an `ErrorCode` enum for the well-known codes listed in §3.5 (`LEASE_EXPIRED`, `ZONE_TYPE_MISMATCH`, `BUDGET_EXCEEDED`) and use it as the type for `RuntimeError.error_code`. The `string` fallback can remain for extensibility but a typed `error_code_enum` field should be primary.

### failure.md §"Reconnection contract"

**Finding:** §6.4 state delta replay specifies "replays these missed transactional/state-stream messages as a burst of normal `SessionMessage` envelopes." For large grace periods (30 s) with many scene events, this burst can be arbitrarily large. There is no specified maximum burst size or paging mechanism. An agent reconnecting after 29 s of activity could receive thousands of events before the `StateDeltaComplete` sentinel. SHOULD-FIX: document a maximum burst size or note that v1 implementation may cap replay depth and fall back to a full snapshot if the missed event count exceeds a threshold.

### v1.md §"V1 explicitly defers: No resumable state sync"

**Finding:** v1.md explicitly defers "No resumable state sync (reconnecting agents get a full snapshot, not a diff)." RFC §6.4 specifies incremental delta replay, which is more capable than v1 scope. Round 1 flagged this as a SHOULD-FIX but did not add an implementation note. This needs an explicit v1 implementation note in §6.4 clarifying that V1 ships full snapshot on resume; the delta-replay mechanism is the target API contract for v1.1+. MUST-FIX: the protocol spec must match implementation intent or the vertical slice builder will implement delta replay thinking it is required for v1.

---

## Technical Robustness

### Issue 1: `SessionInit` conflates fresh handshake and resume (MUST-FIX)

`SessionInit` (§1.2, §9) contains both fresh-connection fields (`agent_id`, `auth_credential`, `requested_capabilities`) and resume fields (`resume_session_token`, `resume_last_seen_server_seq`). The RFC also defines a separate `SessionResume` message (§6.2) that is supposed to be sent "instead of `SessionInit`" when reconnecting.

This creates a protocol ambiguity: the RFC says resume uses `SessionResume` (§6.2: "the agent sends `SessionResume` as the first message instead of `SessionInit`"), but `SessionInit` also contains `resume_session_token` and `resume_last_seen_server_seq` fields. These fields in `SessionInit` are dead code — they are never used if the RFC's own reconnection spec is followed. Worse, a confused implementation could accept resume attempts embedded in `SessionInit`, bypassing the proper `SessionResume` validation path.

**Fix:** Remove `resume_session_token` (field 9) and `resume_last_seen_server_seq` (field 10) from `SessionInit`. These fields exist only in `SessionResume`. The RFC comment "Leave empty for a fresh connection" implies these are optional, but having them in both messages is a latent correctness bug.

### Issue 2: `SubscriptionChange` acknowledgment uses wrong message type (MUST-FIX)

§7.3 states: "The runtime acknowledges via a `MutationResult` with an empty `batch_id`, correlated by `sequence` number."

`MutationResult` (§3.3) carries:
- `batch_id` (empty, as stated)
- `accepted: bool`
- `repeated string created_ids` — semantically meaningless for a subscription change
- `RuntimeError error` — the error field if rejected

Using `MutationResult` to acknowledge a `SubscriptionChange` is a type-system abuse that will confuse implementers. A `SubscriptionChange` is not a mutation batch and should not be acknowledged with a mutation result. The agent-side code path that handles `MutationResult` is for scene mutations; mixing subscription acks into that path creates parsing complexity and coupling.

**Fix:** Define a dedicated `SubscriptionChangeResult` message:
```protobuf
message SubscriptionChangeResult {
  repeated SubscriptionCategory active_subscriptions = 1;   // Full set now active
  repeated SubscriptionCategory denied_subscriptions = 2;   // Requested additions denied
  RuntimeError error = 3;                                    // Set if request was malformed
}
```
Add `SubscriptionChangeResult subscription_change_result = 39` to `SessionMessage.oneof`. The ack is correlated by sequence number (same pattern as other non-batch transactionals). This gives agents a clean, self-describing ack type and allows the runtime to confirm the net subscription state after the change.

### Issue 3: Deduplication window under high session concurrency (SHOULD-FIX)

§5.2 specifies a global deduplication window of 1000 `batch_id` values. §10 sets `max_concurrent_resident_sessions = 16`. With 16 resident agents each sending at 60Hz, the system generates up to 960 `batch_id` entries per second. A 1000-entry window with a 60-second TTL means the window rolls over in approximately 1 second under peak load — far short of the 60-second retransmit safety window the spec intends.

**Fix:** The dedup window must be per-session, not global. Each session independently maintains a window of its own 1000 most-recent `batch_id` values. Cross-session deduplication is neither required nor beneficial (batch IDs are UUIDv7 and globally unique). Per-session windows eliminate the contention between sessions and restore the 60-second TTL guarantee.

Update §5.2 to state: "The runtime maintains a **per-session** deduplication window..."

### Issue 4: Sequence number integer overflow (SHOULD-FIX)

Sequence numbers are `uint64` (§2.3). At 60Hz with 10 messages per frame, a session generates ~600 sequence numbers per second. `uint64` overflows after approximately 975 million years — not a practical concern. However, the RFC specifies sequence numbers "start at 1" but does not specify behavior at overflow. For a protocol spec, overflow behavior should be defined even if it is practically unreachable. CONSIDER adding a note that `uint64` sequence numbers are assumed never to overflow in v1; a future protocol extension can define rollover semantics if needed.

### Issue 5: State machine incomplete — `Resuming → Closed` path missing (MUST-FIX)

Round 1 flagged this as a CONSIDER (item 9); given the Technical Architecture focus of round 2, this is a MUST-FIX.

The state machine diagram (§1.1) shows:
```
Connecting → Handshaking → Active → Disconnecting → Closed
                              ↑             |
                              └── Resuming ←┘ (within grace period)
```

This diagram is incomplete in two ways:
1. It does not show `Resuming → Closed` when `SessionResume` is rejected (expired token, wrong agent_id, etc.).
2. It does not show the `Handshaking → Closed` path when `SessionInit` fails (auth failure, version mismatch).
3. `Resuming` is shown as reached from `Disconnecting`, but the actual transition is: stream drops → `Closed` (orphan) → client reconnects with `SessionResume` → `Resuming`. The current diagram implies resuming is only reachable from `Disconnecting` (i.e., graceful close), not from ungraceful drops where the stream goes directly to `Closed`.

A protocol state machine with missing failure transitions is a correctness risk: implementers fill in the gaps with undefined behavior.

**Fix:** Replace the ASCII diagram in §1.1 with a complete state machine:
```
Connecting → Handshaking → Active ⇄ Disconnecting → Closed (orphaned)
                ↓ (failure)                                 ↓ (grace period)
               Closed                             Resuming ←┘
                                                  ↓ (accepted)    ↓ (rejected)
                                                Active            Closed
```

Also add a row for `Resuming` state to the state table, describing both the accepted and rejected outcomes.

### Issue 6: `SessionInit` — ambiguous behavior when both fresh and resume fields populated (SHOULD-FIX)

Addressed by Issue 1 (remove resume fields from `SessionInit`). If Issue 1 is fixed, this becomes moot. Listed separately to ensure reviewers understand the cascading nature.

### Issue 7: `CapabilityRequest` — no rejection response defined (SHOULD-FIX)

§9 defines `CapabilityRequest` with `capabilities` and `reason`. The RFC says `CapabilityNotice` is sent for mid-session grants/revokes (§2.2, §9). But what happens when a `CapabilityRequest` is **rejected** (the runtime denies the additional capability)?

The only error path for `CapabilityRequest` is if it is treated as a `RuntimeError` response, but `RuntimeError` (§3.5) is a generic error message with no correlation to a specific request (no `request_id` field). An agent that sends a `CapabilityRequest` and gets back a `RuntimeError` has no way to know which request failed if multiple capability requests are in flight.

**Fix:** Either (a) define a `CapabilityRequestResult` message (analogous to `SubscriptionChangeResult`) with a correlation sequence, or (b) document that only one `CapabilityRequest` may be in flight per session at a time (simplest: sequence-correlated `RuntimeError` with the request sequence in the `context` field). The simplest fix is to document the sequence-correlation convention explicitly, matching how `SubscriptionChange` acks work.

### Issue 8: `HeartbeatPong` — server clock conflation (CONSIDER)

`HeartbeatPong.server_timestamp_us` is described as "Server wall-clock at receipt" (§3.6). The field is typed `uint64` representing µs since Unix epoch. However, the RFC elsewhere notes that `timestamp_us` in `SessionMessage` is "Sender wall-clock (µs since Unix epoch); advisory only" (§2.2). Mixing wall-clock semantics into a round-trip latency measurement is fragile — agents that use `HeartbeatPong` for latency estimation need monotonic clocks, not wall-clocks (which can jump). CONSIDER documenting that `server_timestamp_us` is wall-clock only (not suitable for RTT measurement) and that RTT should be computed from `HeartbeatPing.client_timestamp_us` using the agent's own monotonic clock instead.

### Issue 9: `ZonePublish` — missing `zone_publish_result` in `SessionMessage` (MUST-FIX)

`ZonePublish` is listed as a client→server message (`zone_publish = 25` in `SessionMessage.oneof`). Its traffic class is "State-stream or Transactional" (§3.1).

However, there is no server→client `ZonePublishResult` message. If `ZonePublish` is Transactional (for durable zone content), it requires an ack. The current spec leaves agents with no way to know whether their zone publish succeeded or failed (e.g., zone not found, content type mismatch, TTL out of range).

For ephemeral zone content, fire-and-forget is acceptable. For durable zones, agents must know if the publish was rejected. The `ZonePublish` message has no `batch_id` field for deduplication or a mechanism to correlate a result.

**Fix:** Define a `ZonePublishResult` message and add it to `SessionMessage.oneof`:
```protobuf
message ZonePublishResult {
  uint64       request_sequence = 1;  // Sequence of the ZonePublish that triggered this
  bool         accepted         = 2;
  RuntimeError error            = 3;  // Populated if accepted = false
}
```
Add `ZonePublishResult zone_publish_result = 40` to `SessionMessage.oneof` (within the server→client range, 30–49). This uses the sequence-correlation pattern rather than a `batch_id` since zone publishes are not scene-graph mutations.

Update §3.1 table: `ZonePublish` is Transactional when the zone is durable (requires ack), State-stream when ephemeral (fire-and-forget). The `ZonePublishResult` is only sent for durable-zone publishes; for ephemeral zones the publish is acknowledged by the zone's auto-clear behavior.

---

## Cross-RFC Consistency

### RFC 0004 §"Subscription filter rule for InputMessage variants" (SHOULD-FIX)

Round 1 carried this forward as a CONSIDER (item 8). For round 2 (technical architecture), the gap is a correctness risk for implementers.

RFC 0004 §8.5 defines that `FocusGainedEvent` and `FocusLostEvent` are Transactional (never dropped), while `PointerMoveEvent` is Ephemeral (coalesced). Both are variants of the RFC 0004 `InputMessage` `oneof`. RFC 0005 §7.1 defines two separate subscription categories: `input_events` (pointer/key) and `focus_events` (focus gained/lost). But the runtime dispatches them both as `InputEvent` messages on the session stream.

The filtering rule is: inspect the `InputMessage.event` oneof variant before applying the subscription filter. Focus variants route via `focus_events` subscription; pointer/key/gesture variants route via `input_events`. This rule is not stated anywhere in RFC 0005.

**Fix:** Add a note to §7.1 under the subscription table:
> Note: `InputEvent` messages (field 34) carry an RFC 0004 `InputMessage` envelope. The runtime inspects the `InputMessage.event` oneof variant to determine which subscription filter applies: focus variants (`FocusGainedEvent`, `FocusLostEvent`, `CaptureReleasedEvent`, IME events) are filtered by `focus_events`; all other variants (pointer, touch, key, gesture) are filtered by `input_events`. An agent subscribed to `input_events` but not `focus_events` will receive pointer events but not focus events even though they share the same wire message type.

### RFC 0001 (Scene Contract) — `created_ids` type consistency

`MutationResult.created_ids` is `repeated string` (§3.3). RFC 0001 §1 uses `SceneId` (UUIDv7) for all scene object identifiers. In protobuf, `SceneId` is likely a `string` alias (UUIDv7 is text-encoded), so this is probably consistent. CONSIDER adding a comment to the `created_ids` field: `// UUIDv7 strings; type SceneId per RFC 0001 §1.1` to make the relationship explicit.

### RFC 0003 (Timing Model) — `TimingHints` inline definition

`TimingHints` is defined inline in `session.proto` (§9) "for completeness during the pre-code draft phase" with the comment that the production implementation imports from `timing.proto`. The inline and imported definitions must stay in sync. CONSIDER adding a normative note that the inline definition is authoritative only during the pre-code phase; any divergence between the inline definition and `timing.proto` (RFC 0003) must resolve in favor of RFC 0003.

---

## Actionable Findings Summary

### MUST-FIX (addressed in this round)

1. **[MUST-FIX — FIXED]** §1.2, §9: `SessionInit` contains dead resume fields (`resume_session_token`, `resume_last_seen_server_seq`) that duplicate `SessionResume` and create a protocol ambiguity.
   - *Location:* §1.2 proto block; §9 `SessionInit` proto definition
   - *Fix:* Remove fields 9 and 10 from `SessionInit`. Remove the resume-related comment.
   - *Rationale:* Protocol correctness — resume uses `SessionResume`, not `SessionInit`. Having both creates an exploitable or confusing dual-path.

2. **[MUST-FIX — FIXED]** §7.3, §9: `SubscriptionChange` acknowledged with `MutationResult` — type-system abuse.
   - *Location:* §7.3 prose; §9 `SessionMessage` oneof
   - *Fix:* Define `SubscriptionChangeResult` message; add to `SessionMessage.oneof` at field 39.
   - *Rationale:* A subscription change is not a mutation. Reusing `MutationResult` pollutes the mutation result handler with subscription semantics.

3. **[MUST-FIX — FIXED]** §9: `ZonePublish` has no result/ack message.
   - *Location:* §3.1 table; §9 `SessionMessage` oneof
   - *Fix:* Define `ZonePublishResult`; add to `SessionMessage.oneof` at field 40.
   - *Rationale:* Durable zone publishes are Transactional; without an ack, agents cannot detect publish failures.

4. **[MUST-FIX — FIXED]** §1.1: State machine missing `Resuming → Closed` failure path and `Handshaking → Closed` path.
   - *Location:* §1.1 state diagram and state table
   - *Fix:* Update the state diagram to include all failure transitions; add `Resuming` row to the state table with accepted/rejected outcomes.
   - *Rationale:* Incomplete state machines lead to undefined behavior in implementations.

5. **[MUST-FIX — FIXED]** §1.6, §3.6, §10: Heartbeat disconnect threshold is `2 × heartbeat_interval_ms` but the authoritative spec (rig-5vq.8) says 3 missed heartbeats.
   - *Location:* §1.6 step 1, §3.6 prose, §10 table (no parameter for the multiplier)
   - *Fix:* Change to `3 × heartbeat_interval_ms`; add `heartbeat_missed_threshold = 3` as a configurable parameter in §10.
   - *Rationale:* 2× (10 s) vs. 3× (15 s) is a visible operational difference. The authoritative issue description wins.

6. **[MUST-FIX — FIXED]** §6.4: No v1 implementation note; delta replay contradicts v1.md deferral.
   - *Location:* §6.4 State Delta on Resume
   - *Fix:* Add explicit note: "V1 implementation ships full scene snapshot on resume rather than incremental delta replay. The delta-replay mechanism specified here is the target API contract for v1.1+."
   - *Rationale:* Prevents the vertical slice builder from implementing delta replay unnecessarily, contradicting v1.md.

### SHOULD-FIX (addressed in this round)

7. **[SHOULD-FIX — FIXED]** §5.2: Deduplication window is described globally but must be per-session to survive high-concurrency load.
   - *Location:* §5.2 prose
   - *Fix:* Specify "per-session deduplication window."
   - *Rationale:* 16 concurrent sessions × 60Hz = ~960 entries/sec; a 1000-entry global window expires in ~1 second, breaking the 60-second retransmit safety guarantee.

8. **[SHOULD-FIX — FIXED]** §7.1: Missing InputMessage variant → subscription filter rule.
   - *Location:* §7.1 subscription table
   - *Fix:* Add note explaining that focus variants route via `focus_events` subscription and pointer/key variants via `input_events`.
   - *Rationale:* Cross-RFC consistency with RFC 0004 §8.5; implementers need this rule to correctly filter `InputEvent` messages.

9. **[SHOULD-FIX — FIXED]** §3.5: `RuntimeError.error_code` is `string`; should be an enum for stability.
   - *Location:* §3.5 prose and §9 `RuntimeError` proto definition
   - *Fix:* Add `ErrorCode error_code_enum = 5` field to `RuntimeError` with a well-known enum (at minimum covering `LEASE_EXPIRED`, `ZONE_TYPE_MISMATCH`, `BUDGET_EXCEEDED`, `MUTATION_REJECTED`, `PERMISSION_DENIED`). Retain `string error_code` for extension.
   - *Rationale:* architecture.md §"Error model" requires "stable, enumerated identifier." String types are not stable against typos or renaming.

10. **[SHOULD-FIX — FIXED]** §9: `CapabilityRequest` rejection has no clear response path.
    - *Location:* §5.3, §9
    - *Fix:* Add prose to §5.3 documenting that `CapabilityRequest` rejection is signaled by a `RuntimeError` with `context` set to the rejected capability names, correlated by the `sequence` of the `CapabilityRequest` envelope.
    - *Rationale:* Without documented correlation semantics, agents cannot distinguish a `RuntimeError` caused by a capability denial from one caused by a concurrent mutation.

### CONSIDER (not addressed in this round)

11. **[CONSIDER]** §2.3: `uint64` sequence numbers — add note that overflow is not specified (practically unreachable but worth noting for completeness).

12. **[CONSIDER]** §3.6: `HeartbeatPong.server_timestamp_us` is wall-clock; document that it is unsuitable for RTT measurement and agents should use their own monotonic clock.

13. **[CONSIDER]** §9: `MutationResult.created_ids` — add comment linking to RFC 0001 §1.1 `SceneId` type.

14. **[CONSIDER]** §9: `TimingHints` inline definition — add normative note that RFC 0003 is authoritative in case of divergence.

15. **[CONSIDER]** §12: Add note explicitly referencing v1.md §"V1 explicitly defers: No embodied presence level" for the `EMBODIED = 3` enum value (from round 1 carry-forward).

---

## Changes Made to RFC 0005

All MUST-FIX and SHOULD-FIX items above have been applied directly to `about/legends-and-lore/rfcs/0005-session-protocol.md`:

1. **`SessionInit` dead resume fields removed:** fields 9 (`resume_session_token`) and 10 (`resume_last_seen_server_seq`) removed from `SessionInit` proto definition and prose in §1.2. The resume path is fully served by `SessionResume` (§6.2).

2. **`SubscriptionChangeResult` added:** new message defined; `subscription_change_result = 39` added to `SessionMessage.oneof`. §7.3 prose updated to reference `SubscriptionChangeResult` instead of `MutationResult`.

3. **`ZonePublishResult` added:** new message defined; `zone_publish_result = 40` added to `SessionMessage.oneof`. §3.1 table updated to reflect durable vs. ephemeral ack semantics.

4. **State machine completed:** §1.1 diagram updated to show `Handshaking → Closed` (auth failure) and `Resuming → Closed` (token expired/rejected). `Resuming` added to the state table.

5. **Heartbeat threshold changed to 3×:** §1.6 step 1 updated from "2 ×" to "3 ×". §3.6 updated. `heartbeat_missed_threshold = 3` added to §10 configuration table.

6. **V1 implementation note added to §6.4:** explicit note that v1 ships full snapshot on resume; delta replay is the v1.1+ target.

7. **Deduplication window changed to per-session:** §5.2 prose updated.

8. **InputMessage filter rule documented:** §7.1 note added.

9. **`RuntimeError` enum added:** `ErrorCode` enum defined in §9; `error_code_enum` field added to `RuntimeError`. `string error_code` retained for extension.

10. **`CapabilityRequest` rejection documented:** §5.3 paragraph added.
