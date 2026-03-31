# RFC 0005: Session/Protocol — Round 3 Review
## Cross-RFC Consistency and Integration

**Reviewer:** rig-5vq.29
**Date:** 2026-03-22
**Issue:** rig-5vq.29
**Focus:** Cross-RFC coherence with RFC 0001, 0003, 0004, 0006, 0007, 0008, 0009

---

## Doctrine Files Consulted

- `heart-and-soul/architecture.md` — Three-plane protocol, session model, one stream per agent
- `heart-and-soul/security.md` — Trust gradient, capability scopes
- `heart-and-soul/failure.md` — Reconnection contract, degradation
- `heart-and-soul/v1.md` — V1 scope boundary, deferred items

---

## Scores

### Doctrinal Alignment: 5/5

The RFC faithfully implements all doctrine commitments:

- **One stream per agent** (architecture.md §"Resident control plane"): enforced by §2.1; HTTP/2 stream proliferation anti-pattern explicitly prohibited.
- **Arrival time ≠ presentation time** (CLAUDE.md core rule): `TimingHints.present_at_wall_us`/`expires_at_wall_us` carry timing semantics per RFC 0003.
- **Screen sovereignty** (architecture.md §"The screen is sovereign"): leases with TTL, capability scopes, and revocation semantics per §7, RFC 0008.
- **Trust gradient** (security.md §"Trust gradient"): guest MCP surface restricted to zone-centric operations (§8.3); tile management gated behind `resident_mcp` capability.
- **Local feedback first** (CLAUDE.md core rule): `InputCaptureRelease` confirmation via async `CaptureReleasedEvent` preserves local-first input semantics without blocking on round-trip.
- **Human override** (architecture.md): `SessionSuspended`/`SessionResumed` (added this round) give the viewer a clean emergency stop path with guaranteed delivery.

All quantitative requirements are traceable to doctrine passages via the DR-SP table (§Design Requirements).

### Technical Robustness: 4/5

The RFC is technically sound after prior rounds. One residual gap:

- **EventBatch delivery (§3.2 note, §3.8)**: RFC 0004 §8.3 specifies that field 34 must carry `EventBatch` not a single `InputEnvelope`. This is documented now but not yet a wire change in the proto — the proto still shows `InputEvent` (reuse from `scene_service.proto`). Implementors need to reconcile this before coding.

Strengths: per-session dedup windows correctly sized (§5.2 reasoning is solid), heartbeat missed threshold at 3× (not 2×) is correct for transient jitter, state machine is complete with all terminal transitions, backpressure model covers all traffic classes.

### Cross-RFC Consistency: 4/5

**Before this round:** 2/5 — multiple protocol gaps identified.
**After this round:** 4/5 — all MUST-FIX gaps closed; one residual SHOULD-FIX remains.

**Closed in this round:**
1. `SessionSuspended`/`SessionResumed` — RFC 0007 §8 and RFC 0008 §11 protocol gap resolved.
2. Input control request fields 26–29 — RFC 0004 §8.3.1 dependency resolved.
3. `TelemetryFrame.sample_timestamp_us` naming — aligned with §2.4 `_wall_us` convention.
4. `RuntimeError.ErrorCode` — `CLOCK_SKEW_HIGH`, `CLOCK_SKEW_EXCESSIVE`, `SAFE_MODE_ACTIVE`, and RFC 0003 timestamp error codes added.
5. `reconnect_grace_period_ms` / `reconnect_grace_secs` naming drift — §10 now cross-references RFC 0006 config key.
6. §11 cross-RFC table — RFC 0006, 0007, 0008, 0009 rows added.

**Residual (SHOULD-FIX, not blocking):**
- RFC 0004 `EventBatch.batch_ts_us` and `ClockSyncRequest.agent_timestamp_us` still use plain `_us` suffix (not `_wall_us`); those are issues in their respective RFCs, not RFC 0005.

---

## Actionable Findings

### MUST-FIX (all applied in this round)

#### MF-1: `SessionSuspended` / `SessionResumed` absent from RFC 0005
**Location:** §3.2, §9 proto, §9.2
**Problem:** RFC 0007 §4.2, §5.2, §5.5, §8 and RFC 0008 §6.2, §11 reference these messages as required. They were not defined in RFC 0005's `SessionMessage` oneof or §3.2 table. Without them, the safe mode protocol (RFC 0007) cannot be implemented — agents have no way to know mutations are being rejected or when to resume.
**Fix applied:** Added `SessionSuspended` (field 45) and `SessionResumed` (field 46) to `SessionMessage` oneof in §2.2 and §9 proto; added §3.7 with full semantics; added rows to §3.2 table; updated §9.2 field registry; added `SAFE_MODE_ACTIVE` to `RuntimeError.ErrorCode`; updated §11.
**Rationale:** Human override doctrine — "always available, never interceptable" — requires the session protocol to carry safe mode signals.

#### MF-2: Input control request fields 26–29 missing from `SessionMessage`
**Location:** §3.1, §9 proto, §9.2
**Problem:** RFC 0004 §8.3.1 explicitly requires `FocusRequest`, `CaptureRequest`, `CaptureReleaseRequest`, and `SetImePositionRequest` to be added at agent→runtime fields 26–29. Without these, agents cannot request focus or capture on the session stream, and the local-feedback-first doctrine (CLAUDE.md) cannot be implemented.
**Fix applied:** Added fields 26–29 (`InputFocusRequest`, `InputCaptureRequest`, `InputCaptureRelease`, `SetImePosition`) to `SessionMessage` oneof in §2.2 and §9 proto; added response fields 43–44 (`InputFocusResponse`, `InputCaptureResponse`) with corrected field numbers (RFC 0004 proposed 39–40 which are occupied); added §3.8 with semantics and dependency note; updated §3.1 table; updated §9.2.
**Rationale:** RFC 0004 §8.3.1 identifies this as a blocking dependency: "Both RFCs must be updated together before implementation."

#### MF-3: RFC 0004 proposed response field numbers 39–40 conflict with allocated fields
**Location:** §9 proto, §3.8
**Problem:** RFC 0004 §8.3.1 proposed `InputFocusResponse` at field 39 and `InputCaptureResponse` at field 40. Both are already allocated: 39 = `SubscriptionChangeResult`, 40 = `ZonePublishResult`. Using those field numbers would silently corrupt the protobuf wire encoding.
**Fix applied:** Assigned `InputFocusResponse` = 43 and `InputCaptureResponse` = 44 (next available in the server→client range). §3.8 explicitly notes the supersession of RFC 0004's draft field numbers.
**Rationale:** Protobuf field number conflicts produce silent wire encoding corruption; must be resolved before any code is written.

#### MF-4: `TelemetryFrame.sample_timestamp_us` violates §2.4 naming convention
**Location:** §9 proto `TelemetryFrame` message
**Problem:** RFC 0005 §2.4 mandates `_wall_us` suffix for wall-clock timestamps and `_mono_us` for monotonic. `sample_timestamp_us` uses neither suffix, making it ambiguous and inconsistent with the six other timestamp fields in the RFC (all of which were renamed in Round 6).
**Fix applied:** Renamed to `sample_timestamp_wall_us` in both `TelemetryFrame` proto block occurrences.
**Rationale:** The §2.4 field inventory exists precisely to prevent clock-domain ambiguity. An inconsistent field in the same RFC undercuts that guarantee.

### SHOULD-FIX (all applied in this round)

#### SF-1: `CLOCK_SKEW_HIGH` / `CLOCK_SKEW_EXCESSIVE` not in `RuntimeError.ErrorCode`
**Location:** §3.5 `RuntimeError.ErrorCode` enum
**Problem:** RFC 0003 §4.5 says the compositor emits a `CLOCK_SKEW_HIGH` warning "in the session event stream" and agents should call `ClockSync` after receiving it. RFC 0005 §9 also says agents should call `ClockSync` "after receiving CLOCK_SKEW_HIGH events." But `CLOCK_SKEW_HIGH` was not in the `ErrorCode` enum — agents receiving it would see `ERROR_CODE_UNKNOWN` and have to fall back to string matching.
**Fix applied:** Added `CLOCK_SKEW_HIGH = 12`, `CLOCK_SKEW_EXCESSIVE = 13`, `SAFE_MODE_ACTIVE = 14`, `TIMESTAMP_TOO_OLD = 15`, `TIMESTAMP_TOO_FUTURE = 16`, `TIMESTAMP_EXPIRY_BEFORE_PRESENT = 17` to both the §3.5 prose enum and the §9 proto RuntimeError. Updated prose to note that RFC 0003 §3.5/§4.5 are the sources.
**Rationale:** The `ErrorCode` enum exists for type-safe programmatic handling. Any code that agents are expected to react to programmatically (re-sync clock, stop sending mutations) must be enumerated.

#### SF-2: `reconnect_grace_period_ms` / `reconnect_grace_secs` naming drift
**Location:** §10 configuration table
**Problem:** RFC 0005 §10 uses `reconnect_grace_period_ms` (milliseconds), RFC 0006 exposes `reconnect_grace_secs` (seconds) in the TOML config, and RFC 0008 §12 also references `reconnect_grace_period_ms`. Without a cross-reference, implementors configuring this parameter would need to correlate across three documents.
**Fix applied:** Added **Config file key:** note to the §10 `reconnect_grace_period_ms` row pointing to RFC 0006 §2.2 and noting the seconds/ms unit difference.
**Rationale:** Runtime configuration is the operational interface; unit mismatches cause off-by-1000x bugs.

### CONSIDER (documented, not applied)

#### C-1: RFC 0004 `EventBatch` vs single `InputEnvelope` for field 34
**Location:** §3.2 `InputEvent` row, §9 proto field 34 comment
**Problem:** RFC 0004 §8.3 specifies field 34 must carry `EventBatch` (per-frame batching). The current §9 proto shows `InputEvent` reused from `scene_service.proto`. A note was added to §3.2 and the field comment was updated to say "EventBatch per RFC 0004 §8.3" but the actual proto type change requires a RFC 0004 alignment pass. This is a v1 implementation dependency, not a wire-format bug today (since no code exists yet).
**Recommendation:** Create a linked RFC 0004 amendment task to formally declare `EventBatch` as the field 34 type before coding begins.

#### C-2: `SessionEstablished` missing in §3.2 server→client table
**Location:** §3.2
**Problem:** `SessionEstablished` is a server→client message but not listed in the §3.2 table (it is covered in §1.3). For completeness the table could include lifecycle messages.
**Recommendation:** Low priority — §1.3 documents it thoroughly. Acceptable as-is.

#### C-3: `ClockSyncRequest.agent_timestamp_us` naming inconsistency (in RFC 0003)
**Location:** RFC 0003 §7.1 (not this RFC)
**Problem:** RFC 0003's `ClockSyncRequest.agent_timestamp_us` uses the plain `_us` suffix while RFC 0005's convention mandates `_wall_us` for wall-clock values. This is a RFC 0003 issue, not RFC 0005's.
**Recommendation:** File as RFC 0003 amendment to rename to `agent_timestamp_wall_us`, consistent with RFC 0005's `SessionInit.agent_timestamp_wall_us` which serves a similar purpose.

---

## Summary

Round 3 resolves all cross-RFC consistency gaps identified in the issue description. The key wins are:

1. **Protocol completeness**: `SessionSuspended`/`SessionResumed` closes the safe mode protocol gap that blocked RFC 0007 and RFC 0008 from being fully implementable against RFC 0005.
2. **Input protocol completeness**: Fields 26–29 and 43–44 close the RFC 0004 §8.3.1 dependency, making the input subsystem implementable without further RFC changes.
3. **Field number safety**: The RFC 0004 field number conflict (proposed 39–40, both occupied) is caught and corrected before any code could embed the wrong numbers.
4. **Clock domain consistency**: `sample_timestamp_wall_us` renaming and `CLOCK_SKEW_HIGH` enum entries bring timing and error handling into alignment with RFC 0003.
5. **Config cross-reference**: Operators now have a clear path from `reconnect_grace_period_ms` (internal name) to `reconnect_grace_secs` (TOML key).

No dimension scores below 3. Round 4 can proceed.
