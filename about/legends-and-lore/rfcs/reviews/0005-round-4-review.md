# RFC 0005: Session/Protocol — Round 4 Review
## Final Hardening and Quantitative Verification

**Reviewer:** rig-5vq.30
**Date:** 2026-03-22
**Issue:** rig-5vq.30
**Focus:** Quantitative verification, internal consistency, implementer readiness

---

## Doctrine Files Consulted

- `about/heart-and-soul/architecture.md` — Three-plane protocol, session model, error model, message classes
- `about/heart-and-soul/failure.md` — Reconnection contract, agent crash recovery, degradation axes
- `about/heart-and-soul/security.md` — Trust gradient, capability scopes, agent isolation
- `about/heart-and-soul/validation.md` — Testing doctrine, split latency budgets, structured telemetry

---

## Scores

### Doctrinal Alignment: 5/5

The RFC faithfully implements all doctrine commitments after 12 rounds of refinement:

- **Single stream per agent** (architecture.md §"Session model"): enforced without exception in §2.1.
- **Arrival time ≠ presentation time**: `TimingHints.present_at_wall_us`/`expires_at_wall_us` carry timing semantics; §2.4 "Clock Domains" inventory is complete and consistent.
- **Screen sovereignty**: leases with TTL, capability scopes, and revocation semantics throughout §6–§7 and RFC 0008.
- **Trust gradient**: guest MCP surface restricted to zone-centric operations (§8.3); tile management gated behind `resident_mcp` capability.
- **Local feedback first**: `InputCaptureRelease` confirmed via async `CaptureReleasedEvent`; `SetImePosition` is fire-and-forget; no input path depends on agent round-trip.
- **Human override always available**: `SessionSuspended`/`SessionResumed` (§3.7) implement the safe mode protocol; guaranteed Transactional delivery.
- **Quantitative coverage**: all DR-SP requirements (DR-SP1–8) are traceable to implementation sections; configuration parameters in §10 include units and defaults.

### Technical Robustness: 4/5

The RFC is implementation-ready after prior rounds. Two issues remain that an implementer would hit immediately:

1. **§9 intro / §9.1 import graph inconsistency** (MUST-FIX): The §9 introductory paragraph states `session.proto` imports `RuntimeError` from `scene_service.proto`. This is false — `RuntimeError` is defined in `session.proto` itself (explicitly stated by the comment at the `RuntimeError` definition block: "Defined here (not imported)"). Additionally, §9.1 import graph labels the scene import as `scene.proto` while the actual proto header uses `import "scene_service.proto"`. An implementer reading §9 straight through will be confused about which file to look in for `RuntimeError`.

2. **`SessionResumeResult` missing subscription echo** (SHOULD-FIX): `SessionEstablished` echoes `active_subscriptions` and `denied_subscriptions` so agents know exactly which event categories are active after handshake. `SessionResumeResult` (§6.3) has no equivalent. An agent that resumes a session has no confirmation of which subscriptions were restored — it must assume its previous subscription set is intact, which may not hold if capabilities changed during the grace period.

Strengths: per-session dedup windows are correctly reasoned (§5.2 math is solid), heartbeat threshold at 3× is correct for transient jitter, state machine is complete with all terminal transitions (§1.1), backpressure model covers all traffic classes (§2.5), field number allocation has no conflicts (verified below), all error codes are enumerated.

### Cross-RFC Consistency: 5/5

All cross-RFC gaps identified in Round 3 are closed. The §11 cross-RFC table is complete and accurate. Specific verifications:

- **RFC 0001**: `SceneId` used for `batch_id`, `lease_id`, `created_ids`; `SceneSnapshot` imported correctly; ID type convention documented.
- **RFC 0003**: `TimingHints` inline matches RFC 0003 §7.1; `_wall_us`/`_mono_us` naming consistent; `CLOCK_SKEW_HIGH`/`CLOCK_SKEW_EXCESSIVE` in `ErrorCode` enum.
- **RFC 0004**: Input control fields 26–29 and response fields 43–44 present; RFC 0004 field number conflict (proposed 39–40) resolved; `EventBatch` note at field 34.
- **RFC 0006**: `reconnect_grace_secs` cross-reference in §10; RFC 0006 is authoritative for TOML keys.
- **RFC 0007**: `SessionSuspended`/`SessionResumed` fields 45–46 implement RFC 0007 §5.2/§5.5 gap.
- **RFC 0008**: `SUSPENDED` lease state during safe mode; `reconnect_grace_period_ms` referenced in RFC 0008 §12.
- **RFC 0009**: `PERMISSION_DENIED`/`BUDGET_EXCEEDED` error codes reused; transactional guarantees take precedence over shedding policy.

---

## Field Number Verification (Exhaustive)

All `SessionMessage.payload` oneof fields verified for conflicts:

| Range | Fields | Status |
|-------|--------|--------|
| 10–15 | `session_init`, `session_established`, `session_close`, `session_error`, `session_resume`, `session_resume_result` | No conflicts |
| 16–19 | Unallocated | Consistent with §9.2 |
| 20–25 | `mutation_batch`, `lease_request`, `heartbeat_ping`, `capability_request`, `subscription_change`, `zone_publish` | No conflicts |
| 26–29 | `input_focus_request`, `input_capture_request`, `input_capture_release`, `set_ime_position` | No conflicts; RFC 0004 §8.3.1 satisfied |
| 30–37 | `mutation_result`, `lease_response`, `heartbeat_pong`, `scene_event`, `input_event`, `degradation_notice`, `runtime_error`, `capability_notice` | No conflicts |
| 38 | `StateDeltaComplete` — reserved/deferred | Consistent between prose §9 comment and §9.2 table |
| 39–46 | `subscription_change_result`, `zone_publish_result`, `telemetry_frame`, `scene_snapshot`, `input_focus_response`, `input_capture_response`, `session_suspended`, `session_resumed` | No conflicts |
| 47–49 | Available; documented in §9.2 and proto comment | Consistent |
| 50–99 | Reserved for post-v1 | Consistent between §2.1 prose and §9.2 |

**Conclusion:** Zero field number conflicts across 37 allocated fields and all reserved ranges.

---

## Actionable Findings

### MUST-FIX

#### MF-1: §9 intro paragraph incorrectly lists `RuntimeError` as imported from `scene_service.proto`

**Location:** §9 opening paragraph ("The session protocol is defined in a new file `session.proto`...")
**Problem:** The paragraph reads: "It imports the existing `scene_service.proto` for `MutationProto`, `SceneEvent`, `InputEvent`, `RuntimeError`, and zone message types." This is incorrect — `RuntimeError` is defined in `session.proto` itself, not imported. The `RuntimeError` definition block at line ~1128 explicitly states "Defined here (not imported) because RuntimeError is used throughout session.proto." An implementer reading the intro will look for `RuntimeError` in `scene_service.proto`, not find it, and be confused.

**Fix:** Remove `RuntimeError` from the list of types imported from `scene_service.proto` in the §9 intro. The sentence should state that `RuntimeError` is defined in `session.proto` itself.

**Rationale:** Implementation correctness — an implementer writing the proto file will create a broken import if they believe `RuntimeError` comes from `scene_service.proto`.

#### MF-2: §9.1 import graph uses `scene.proto` but proto header imports `scene_service.proto`

**Location:** §9.1 import graph
**Problem:** The import graph shows `session.proto` importing `scene.proto (RFC 0001)`. The actual proto header at §9 shows `import "scene_service.proto"`. These are different file names. `scene.proto` is also separately imported for `SceneId`. The inconsistency leaves implementers unsure which file provides which types.

**Fix:** Update the §9.1 import graph to match the actual proto header: show both `import "scene_service.proto"` (for `MutationProto`, `SceneEvent`, `InputEvent`, `LeaseRequest`, `LeaseResponse`, `SceneSnapshot`, `ZoneContent`) and `import "scene.proto"` (for `SceneId`). This matches the two actual import lines already in the proto header.

**Rationale:** Implementation correctness — the import graph is the first reference an implementer uses when setting up `session.proto`. Mismatched file names produce compiler errors.

### SHOULD-FIX

#### SF-1: `SessionResumeResult` lacks subscription echo fields

**Location:** §6.3, §9 proto `SessionResumeResult`
**Problem:** `SessionEstablished` (§1.3, §9) includes `active_subscriptions` and `denied_subscriptions` fields — essential for agents to know which event categories are confirmed after handshake. `SessionResumeResult` has no equivalent. An agent resuming a session must assume its previous subscription set was preserved intact. This assumption can fail if: (a) the agent's capabilities were changed during the grace period, (b) a new subscription category was added in a newer protocol version, or (c) the runtime's resource governor applied subscription restrictions during the suspension.

**Fix:** Add `repeated SubscriptionCategory active_subscriptions = 7` and `repeated SubscriptionCategory denied_subscriptions = 8` to `SessionResumeResult`, matching the `SessionEstablished` field pattern. Update §6.3 prose to note that these echo the confirmed post-resume subscription state.

**Rationale:** An implementer writing a resume handler without explicit subscription confirmation will silently have wrong subscription state after resume. The `SessionEstablished` pattern explicitly exists to solve this problem — `SessionResumeResult` should be symmetric.

### CONSIDER

#### C-1: `ZonePublish.ttl_us` comment says "UTC µs duration" — duration is not UTC

**Location:** §9 proto `ZonePublish.ttl_us` field comment
**Problem:** The comment reads `// UTC µs duration; 0 = zone default`. A TTL is a duration (relative), not a UTC timestamp (absolute). The `UTC` prefix is misleading — it implies a wall-clock reference point when the field is a simple duration value.

**Fix:** Change comment to `// Duration in µs; 0 = zone default (use zone's auto_clear_us). RFC 0003 §3.1: _us is authoritative.` (remove "UTC").

**Rationale:** Documentation clarity — "UTC µs duration" is an oxymoron that could confuse an implementer into treating this as a wall-clock deadline rather than a relative duration.

#### C-2: `SessionEstablished.heartbeat_interval_ms` typed `uint64` instead of `uint32`

**Location:** §1.3 prose proto and §9 proto `SessionEstablished.heartbeat_interval_ms`
**Problem:** `heartbeat_interval_ms` is typed `uint64`. The default value is 5000 ms (5 seconds); the maximum sensible value is on the order of minutes. `uint64` wastes 4 bytes on the wire per message for a value that fits easily in `uint32` (max ~4.29 billion ms ≈ 49 days). All similar duration fields in `TelemetryFrame` (`compositor_frame_budget_us`, `compositor_frame_time_us`) use `uint32`. The inconsistency creates minor confusion.

**Fix:** Change `uint64 heartbeat_interval_ms = 4` to `uint32 heartbeat_interval_ms = 4` in both the §1.3 prose proto and §9 proto. This is a wire-compatible type narrowing in proto3 (uint32 and uint64 use the same varint encoding; receiving the other type truncates/extends without data loss for values in range).

**Rationale:** Consistency with `TelemetryFrame` duration fields; proto3 style guide prefers `uint32` for values that fit.

---

## Implementer Readiness Assessment

An implementer reading this RFC from scratch would be able to implement the full session protocol without ambiguity, with two exceptions (MF-1 and MF-2) that would produce immediate build errors. After fixing those, the following are fully specified:

- Complete state machine with all transitions (§1.1)
- All message types with field numbers and types (§2.2, §9)
- Traffic class for every message (§3.1, §3.2)
- Retransmission policy with timeouts (§5.3)
- Deduplication window sizing with correctness reasoning (§5.2)
- Reconnection behavior for all three cases (§6.4, §6.5, §6.6)
- Subscription filtering rule for `InputEvent` variants (§7.1)
- MCP tool surface with capability gates (§8.3)
- All configuration parameters with defaults and units (§10)
- Cross-RFC dependencies with specific section references (§11)

---

## Changes Applied to RFC 0005

All MUST-FIX items (MF-1, MF-2) and SHOULD-FIX item (SF-1) are applied directly in this round. The CONSIDER items are documented above for future rounds.

### Summary of Changes

1. **§9 intro paragraph**: Removed `RuntimeError` from the list of types imported from `scene_service.proto`; added clarifying note that `RuntimeError` is defined in `session.proto`.

2. **§9.1 import graph**: Updated to show both `scene_service.proto` and `scene.proto` imports with accurate type lists matching the actual proto header.

3. **§6.3 `SessionResumeResult`**: Added `active_subscriptions` (field 7) and `denied_subscriptions` (field 8) to both the §6.3 prose proto block and §9 proto definition; updated §6.3 prose to explain the subscription echo semantics.
