# Review: RFC 0003 Timing Model — Round 2 (Technical Architecture Scrutiny)

**Issue:** rig-5vq.20
**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Round:** 2 of 4 — Technical Architecture Scrutiny
**Doctrine files consulted:** architecture.md, validation.md, v1.md

---

## Methodology

Focus: technical rigor — data structure choices, concurrency safety, protocol
correctness, error handling completeness, platform-specific feasibility,
performance budget realism, API ergonomics, state machine edge cases, and
cross-RFC consistency.

The RFC as it exists after Round 1 fixes is the base for this review.

---

## Scores

| Dimension | Round 1 | Round 2 |
|-----------|---------|---------|
| Doctrinal Alignment | 4/5 | 4/5 |
| Technical Robustness | 4/5 | 4/5 |
| Cross-RFC Consistency | 3/5 | **4/5** |

All dimensions ≥ 3. Ready for Round 3.

---

## Doctrinal Alignment: 4/5

No new doctrinal gaps found. Round 1 fixes held. Score unchanged at 4/5.

The RFC correctly embodies:
- "Arrival time is not presentation time" at every layer.
- All four message classes with correct delivery semantics.
- Injectable clock satisfying DR-V4.
- Sync drift budget (500μs) from validation.md.
- Staleness indicators from failure.md.

Score remains 4 (not 5) because `effective_after` (architecture.md §Time) remains
unimplemented, documented in Open Question 1 and §3.2 as an acceptable v1 deferral.

---

## Technical Robustness: 4/5

The clock domain hierarchy, frame deadline model, drift correction arithmetic,
expiry heap, and sync group commit policies are all sound. The following issues
were found and fixed:

### [MUST-FIX → FIXED] T-R1: `ClockSync` RPC missing service definition

**Problem:** §4.5 says agents can request a `ClockSync` RPC. §7.2 says it is "a
unary RPC on the session service." But `timing.proto` defined only the
`ClockSyncRequest`/`ClockSyncResponse` message types — no `service` block
existed. Without a service definition, the RPC cannot be called. Implementors
had no contract for which service exposes it.

**Fix applied:** Added `ClockSyncService` block to `timing.proto` §7.1 with a
normative note that the preferred implementation adds `ClockSync` as a method
on the `SessionService` in RFC 0005 (keeping all agent-runtime communication on
one gRPC endpoint). Updated §7.2 to clarify the service home unambiguously.
Also added `rpc ClockSync` to `SessionService` in RFC 0005 §9 protobuf appendix.

### [MUST-FIX → FIXED] T-R2: Pending queue flush on session close unspecified

**Problem:** §5.3 defines a per-agent pending queue of up to 256 future-scheduled
mutations. RFC 0005 says reconnecting agents perform a full re-handshake. RFC 0003
did not specify what happens to the pending queue on session close. Ambiguities:
- Do queued entries from the old session survive reconnect under the new session?
- Do they count against the new session's budget?
- Can pending mutations from a closed session corrupt the scene graph?

**Fix applied:** Added normative language to §5.3: the pending queue is
session-scoped. On session close (graceful or ungraceful), all entries are
discarded. A reconnecting agent starts with an empty queue and must retransmit
future-scheduled mutations under the new session.

### [SHOULD-FIX → FIXED] T-R3: Expiry non-negotiability under load was a recommendation, not a requirement

**Problem:** Open Question 5 correctly identified that expiry evaluation "must
run even under load" but stated this only as a recommendation in the open
questions section. §5.4 (the normative section) was silent on this. RFC 0002's
degradation machinery could theoretically allow Stage 4 to be deferred, creating
a contradiction.

**Fix applied:** Added normative paragraph to §5.4: expiry evaluation at Stage 4
is never deferred by degradation. Even at ShedTiles/Emergency levels, the expiry
heap must be evaluated and implicit DeleteTile mutations applied. Expiry semantics
are a correctness contract.

### [SHOULD-FIX → FIXED] T-R4: No fast-convergence path after clock jump

**Problem:** The 32-sample median window for skew estimation converges slowly
after a sudden NTP step correction on the agent host. During the convergence
period, the compositor applies the wrong correction. No mechanism existed to
detect a clock jump and reset the window.

**Fix applied:** Added clock jump detection to §4.3: if consecutive samples show
a skew change > `timing.clock_jump_detection_ms` (default 50ms), the estimation
window is reset to the latest single sample. Added `clock_jump_detection_ms` to
`TimingConfig` (field 10). Added corresponding Layer 0 test requirement to §8.2.

### [SHOULD-FIX → FIXED] T-R5 (SM-1): Explicit tile deletion mid-deferral unspecified

**Problem:** §5.4 specified the sync-group interaction for tile *expiry* during
deferral, but not for explicit `DeleteTile` mutations. The two cases have the
same semantics and should be stated explicitly for both.

**Fix applied:** Added paragraph to §2.3: explicit `DeleteTile` causes the tile
to leave its sync group before deletion, identical to expiry behavior. If the
deleted tile was the sole missing member of an `AllOrDefer` group, removing it
unblocks the group in the same frame.

### [CONSIDER] T-R6: Frame quantization formula boundary condition

The formula `T <= frame_F_vsync_us + frame_budget_us / 2` handles all cases
correctly. The `TIMESTAMP_TOO_OLD` check (§3.5) is the safety valve for the
past-timestamp case. The two checks work together and no change is needed.
Noted in the RFC's §3.3 without modification.

---

## Cross-RFC Consistency: 4/5 (up from 3/5)

### [MUST-FIX → FIXED] C-R1: `TimingHints.sync_group_id` type mismatch between RFC 0005 and RFC 0003

**Problem:** RFC 0005's inline `TimingHints` definition used `string sync_group_id`.
RFC 0003 §2.2 and §7.1 establish `SyncGroupId` as a 16-byte UUIDv7 binary value
(`bytes`), consistent with RFC 0001's `SceneId` type. Using `string` for a binary
UUID is incorrect — it does not round-trip through proto serialization for binary
UUIDs and would mislead implementors until the actual `timing.proto` import lands.

**Fix applied:** Updated RFC 0005's inline `TimingHints` to `bytes sync_group_id`
with a comment consistent with RFC 0003 §2.2. Added a note clarifying this is a
documentation aid that must match `timing.proto` exactly.

### [MUST-FIX → FIXED] C-R2: `ClockSync` RPC absent from RFC 0005 session service

**Problem:** RFC 0003 §7.2 stated `ClockSync` is "a unary RPC on the session
service" but RFC 0005's `SessionService` had no `ClockSync` method. An implementor
reading both RFCs could not determine where the RPC lives.

**Fix applied:** Added `rpc ClockSync(ClockSyncRequest) returns (ClockSyncResponse)`
to `SessionService` in RFC 0005 §9. Updated the import graph (§9.1) to show that
`session.proto` imports `timing.proto`. Updated the cross-reference table (§11).

### [SHOULD-FIX → FIXED] C-R3: `session_open_at_us` sync point not exposed in `SessionEstablished`

**Problem:** RFC 0003 §1.3 defines the "per-handshake sync point" where the
compositor records `session_open_wallclock_us` to initialize the clock-skew
estimate. Agents cannot produce valid timestamps for their first mutation batch
without knowing the compositor's clock reference — yet `SessionEstablished` in
RFC 0005 provided no clock information. Agents had to make a separate `ClockSync`
RPC call just to get a reference, adding an unnecessary round trip.

**Fix applied:**
- Added `agent_timestamp_us` (field 11) to `SessionInit` so agents can supply
  their clock at handshake time, enabling the compositor to compute an initial
  skew estimate.
- Added `compositor_wallclock_us` (field 9) and `estimated_skew_us` (field 10)
  to `SessionEstablished` so agents receive a clock reference at handshake time
  without a separate RPC. The `ClockSync` RPC remains available for ongoing
  re-synchronization.

### [CONSIDER] C-R4: `ZonePublish.ttl_ms` unit inconsistency

`ZonePublish.ttl_ms` in RFC 0005 uses milliseconds while RFC 0003 mandates
microseconds for all timing values. Zone auto-clear timeouts are not
sub-millisecond operations, so this is functionally harmless. Deferred to the
RFC 0001 `_ms → _us` amendment sweep, which should also cover `ZonePublish.ttl_ms`
in RFC 0005 at the same time.

---

## Performance Budget Realism

| Budget | Value | Assessment |
|--------|-------|------------|
| p99 frame time | < 16.6ms | Achievable on reference hardware for v1 node types (no media decode). |
| input_to_local_ack | p99 < 4ms | Conservative — stages 1+2 combined are ≤ 1ms per RFC 0002 §3.2. Substantial headroom. |
| input_to_scene_commit | p99 < 50ms | Achievable for local agents. Cannot be validated in headless CI for remote agents — document the limitation. |
| input_to_next_present | p99 < 33ms | Achievable at 60Hz (two frame windows = 33.3ms). |
| Sync drift | < 500μs | **Achievable but tight on loaded hardware.** Two localhost gRPC agents typically see 100–500μs arrival spread. Treating this as a telemetry budget (not hard reject) is the correct call. Test infra should explicitly suppress sync-drift staleness indicators in headless CI to avoid spurious alerts. |
| Agent clock drift | 100ms warning / 1s reject | Appropriate for LAN/localhost. |

Overall: budgets are achievable on declared v1 hardware targets for v1 scope.

---

## State Machine Analysis: Sync Group Lifecycle

Edge cases examined:

1. **Tile expires while sync group is deferring.** §5.4 specifies: tile leaves group first, then is deleted. The updated member set is evaluated by the commit policy. If the expired tile was the sole missing member, the group can now commit. Correct.

2. **Agent disconnects while sync group has pending deferred mutations.** The T-R2 fix (pending queue flush on session close) resolves this: queued mutations from the closed session are discarded, releasing any sync group deferred state associated with them. The sync group will proceed with available members on next evaluation.

3. **Sync group with only one member.** Always satisfies `AllOrDefer` trivially. Correct.

4. **Explicit `DeleteTile` mid-deferral.** Fixed (T-R5/SM-1 above): now explicitly specified to behave identically to expiry.

---

## API Ergonomics Assessment

`TimestampedPayload` is well-structured. Clock trait design is clean. The `SimulatedClock`
advance method design note (added to §8.1) prevents a common implementor error of adding
mutation methods to the `Clock` trait. `ClockSync` being on `SessionService` (not a
separate endpoint) is the ergonomically correct choice — agents manage one connection.

---

## Changes Applied

### RFC 0003 (docs/rfcs/0003-timing.md)
- §Review History: Added Round 2 summary.
- §2.3: Added tile deletion mid-deferral behavior.
- §4.3: Added clock jump detection and window reset.
- §5.3: Added session close flush normative requirement.
- §5.4: Added expiry non-negotiability under load as normative requirement.
- §7.1 `timing.proto`: Added `clock_jump_detection_ms` to `TimingConfig`; added `ClockSyncService` block.
- §7.2: Clarified ClockSync RPC service home.
- §8.1: Added Clock trait design constraint note.
- §8.2: Added three new test coverage requirements.

### RFC 0005 (docs/rfcs/0005-session-protocol.md)
- `SessionInit`: Added `agent_timestamp_us` (field 11).
- `SessionEstablished`: Added `compositor_wallclock_us` (field 9) and `estimated_skew_us` (field 10).
- Inline `TimingHints`: Fixed `sync_group_id` from `string` to `bytes`.
- `SessionService`: Added `rpc ClockSync(ClockSyncRequest) returns (ClockSyncResponse)`.
- §9.1 Import graph: Added `timing.proto` import entry.
- §11 Cross-reference table: Updated RFC 0003 row with new relationships.
