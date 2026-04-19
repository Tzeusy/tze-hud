# RFC 0008: Lease and Resource Governance — Amendment: C13 Capability Dialog + 7-Day Remember

**Amendment ID:** 0008-amendment-c13-capability-dialog
**Issue:** hud-ora8.1.11
**Date:** 2026-04-19
**Author:** hud-ora8.1.11 (parallel-agents worker)
**Depends on:** RFC 0009 Amendment A1 (C12 role-based operators, hud-ora8.1.12)
**Parent task:** hud-ora8.1 (v2 embodied media presence program preparation)
**Source decision:** v2-embodied-media-presence signoff packet, C13 (F29 mandated: RFC 0008 amendment required before implementation beads on this topic)
**Cross-reference:** `openspec/changes/v2-embodied-media-presence/specs/identity-and-roles/` (capability spec hud-ora8.2.5, to be authored) owns the authoritative identity-and-roles schema; this amendment establishes the per-capability grant dialog and remember behavior within lease governance.

---

## Scope and Purpose

This amendment extends RFC 0008 (Lease and Resource Governance) to specify:

1. A **capability taxonomy** for the eight v2 media-presence capabilities introduced by C13.
2. A **per-session operator dialog flow** triggered the first time an agent requests one of those capabilities within an enabled deployment.
3. A **7-day per-agent-per-capability remember** mechanism: scope, expiry, revocation, and data model.
4. Where the capability dialog **inserts into the existing lease grant flow** (state machine and evaluation sequence).
5. How the dialog interacts with **C12 role-based operators** (RFC 0009 Amendment A1), governing which operator roles may grant, be prompted for, or are permanently excluded from granting capabilities.
6. **Audit events** for capability dialog actions per C17.
7. A **cross-reference** to the identity-and-roles capability spec (hud-ora8.2.5).

This amendment is **documentary only**. No protobuf schema changes are introduced here. Implementation schema and wire changes are owned by downstream tasks as noted.

---

## Background: C13 in the V2 Signoff Packet

The v2-embodied-media-presence signoff packet, decision C13, states:

> **Per-capability grants** with opinionated bundled presets ("household demo", "cloud-collab"). **Runtime config for fundamental on/off (restart), per-session operator dialog for first-use** within enabled capabilities. Dialog remembers per-agent-per-capability for 7 days. Capabilities: media-ingress, microphone-ingress, audio-emit, recording, cloud-relay, external-transcode, federated-send, agent-to-agent-media. **The dialog + 7-day-remember behavior is RFC-level** (RFC 0008 lease-governance amendment); not a spec-only decision.

C13 is explicitly tagged as requiring an RFC 0008 amendment in F29:

> RFC 0008 (lease-governance) for C13's capability dialog + 7-day remember.

The existing lease grant flow (RFC 0008 SS3.3) evaluates capability scope as part of the `REQUESTED -> ACTIVE` transition. This amendment adds a new evaluation gate — the capability dialog — that fires before the standard evaluation when a first-use condition is met.

---

## Amendment Content

### A1. Capability Taxonomy

The following eight capabilities are introduced by C13. Each is a member of the `capability_scope` vocabulary defined in RFC 0005 SS7 and referenced throughout RFC 0008 SS1.3.

| Capability Token | Semantic |
|-----------------|----------|
| `media-ingress` | Allows an agent to open one-way inbound visual streams bound to runtime zones; requires operator admission and viewer privacy ceiling clearance (see `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`). |
| `microphone-ingress` | Allows an agent to receive audio captured from a local microphone device; implies strong privacy and consent requirements above those of `media-ingress`. |
| `audio-emit` | Allows an agent to emit audio output through the runtime's audio routing subsystem; requires explicit operator enablement at the deployment level. |
| `recording` | Allows an agent to initiate recording of media streams; subject to the recording policy in C14 (local disk only, 30-day default TTL, mandatory continuous visible indicator, operator-grants-once consent). |
| `cloud-relay` | Allows an agent to relay media through an external SaaS SFU (LiveKit Cloud or Cloudflare Calls in v2); subject to C15 trust boundary and presence-metadata export restriction. |
| `external-transcode` | Allows an agent to offload media transcoding to an external service; constitutes a data-boundary exit and requires explicit operator approval. |
| `federated-send` | Allows an agent to send media or presence data to another tze_hud installation via a federation channel; deferred to post-v2 federation phase but capability token is defined here to reserve its place in the dialog. |
| `agent-to-agent-media` | Allows an agent to exchange media streams with another agent session within the same runtime via the runtime-relayed relay path (signoff 4e); subject to per-hop policy enforcement and routing rules. |

**v2 implementation scope note:** `federated-send` is defined in this taxonomy for data-model completeness (the 7-day remember store must reserve a slot for it) but the runtime rejects any `LeaseRequest` that includes `federated-send` in v2 with `DenyReason::CAPABILITY_NOT_IMPLEMENTED`. This matches the treatment of federation fields in RFC 0009 Amendment A1 (A1.2): federated fields ship in the data model but are not active in v2.

**Relationship to existing capabilities:** These eight tokens extend the capability vocabulary defined in RFC 0005 SS7. They do not replace or overlap existing v1 capability tokens (`create_tiles`, `publish_zone:*`, `manage_tabs`, `read_scene_topology`, `lease:priority:*`). Each of the eight C13 capabilities is separately grantable and separately dialog-prompted.

---

### A2. Per-Session Operator Dialog Flow

#### A2.1 When the Dialog Fires

The capability dialog fires **once per agent-per-capability per session** on the first `LeaseRequest` that includes one of the eight C13 capabilities in its `capability_scope`. Specifically:

A dialog is required when ALL of the following conditions hold:

1. The requested `capability_scope` includes at least one C13 capability (from §A1).
2. The capability is **enabled** at the deployment level (runtime config `capabilities.<token>.enabled = true`; if disabled at this level, the request is denied immediately with `DenyReason::CAPABILITY_NOT_ENABLED` — no dialog).
3. The capability does **not** have a valid 7-day remember record for this agent (see §A3).
4. The capability has **not** already been interactively granted in the current session for this agent.

If conditions (1)–(4) all hold, the runtime suspends the `LeaseRequest` processing and presents the operator dialog before completing the `REQUESTED -> ACTIVE` (or `REQUESTED -> DENIED`) transition.

#### A2.2 Dialog State Machine

The capability dialog introduces a new transient sub-state within the `REQUESTED` lease state. The lease remains in `REQUESTED` while awaiting operator response. No new enum value is added to `LeaseState`; this is an implementation-internal gate within the existing `REQUESTED` state.

```
REQUESTED
  │
  ├── [no dialog required] ──────────────────────────────────────► standard evaluation
  │
  └── [dialog required]
        │
        ├── operator grants ──────────► record in session capability cache
        │                               + optionally write 7-day remember record (if "remember" checked)
        │                               + continue standard evaluation
        │
        ├── operator denies ──────────► DENIED (deny_reason = CAPABILITY_DIALOG_DENIED)
        │
        └── dialog timeout ───────────► DENIED (deny_reason = CAPABILITY_DIALOG_TIMEOUT)
             (default 30s)
```

The agent's `LeaseRequest` is held while the dialog is shown. The agent receives a `LeaseResponse` only after operator response (grant or deny) or timeout.

**Latency note:** The dialog introduces human-latency into the lease grant path. This is acceptable for first-use capability grants; it is not acceptable for ordinary lease requests. The gate must be skipped (§A2.1 condition 3 and 4) when a valid remember record or a session-level cached grant exists, preserving the < 1ms grant latency for subsequent requests.

#### A2.3 Dialog Presentation

The dialog is presented through the system shell (RFC 0007) on the runtime display. It MUST include:

- The **agent identity** (namespace) requesting the capability.
- The **capability name and a human-readable description** (one-line semantic from §A1).
- A **"Grant"** action and a **"Deny"** action.
- A **"Remember for 7 days"** checkbox (default: unchecked; operator opt-in only).

The dialog MUST be rendered in the chrome layer (priority 0) so it cannot be occluded by agent tiles. It MUST NOT be dismissable by the agent whose request triggered it.

**Bundled presets note (C13):** The signoff packet references "household demo" and "cloud-collab" presets as opinionated bundles. Preset application is a configuration concern (which C13 capabilities are pre-enabled in the deployment config) and does not bypass the per-session dialog. A preset only sets the deployment-level `capabilities.<token>.enabled` flag; the per-session dialog still fires on first use within that deployment.

#### A2.4 Session Capability Cache

When an operator grants a capability in the dialog, the runtime records a session-scoped capability cache entry:

```rust
pub struct SessionCapabilityGrant {
    /// The session for which this grant is cached.
    pub session_id: SessionId,
    /// The agent namespace within that session.
    pub agent_namespace: String,
    /// The capability token granted.
    pub capability: String,
    /// When this grant was recorded (UTC microseconds).
    pub granted_at_us: u64,
    /// Whether a 7-day remember record was also written.
    pub remember_written: bool,
}
```

The session capability cache is in-memory and session-scoped. It does not survive session teardown. It is separate from the 7-day remember store (§A3), which is persistent.

---

### A3. 7-Day Per-Agent-Per-Capability Remember

#### A3.1 Scope

The remember record is scoped by:
- **Agent identity:** The `agent_namespace` from the `LeaseRequest` (established at session auth, RFC 0005 SS2.1).
- **Capability token:** One of the eight C13 tokens from §A1.

It is **not** a global grant (does not cover all agents or all capabilities). It is **not** session-scoped (survives session teardown; persists across reconnects).

#### A3.2 Data Model

```rust
pub struct CapabilityRememberRecord {
    /// Agent namespace this record applies to.
    pub agent_namespace: String,

    /// Capability token this record applies to.
    pub capability: String,

    /// The operator principal who granted and chose "remember".
    pub granted_by: PrincipalId,

    /// When the remember record was written (UTC microseconds).
    pub granted_at_us: u64,

    /// When the remember record expires (UTC microseconds).
    /// Always = granted_at_us + 7 * 24 * 60 * 60 * 1_000_000.
    pub expires_at_us: u64,

    /// Whether this record has been explicitly revoked before natural expiry.
    pub revoked: bool,

    /// If revoked: the operator principal who revoked it.
    pub revoked_by: Option<PrincipalId>,

    /// If revoked: when the revocation occurred (UTC microseconds).
    pub revoked_at_us: Option<u64>,
}
```

**`PrincipalId`** is defined in RFC 0009 Amendment A1 (§A1.2): a local UUID in v2, federation-aware DID post-v2.

#### A3.3 Expiry

The remember record expires automatically 7 days after `granted_at_us`. After expiry, the next `LeaseRequest` for the capability from that agent triggers the dialog again. Expiry is evaluated at request time; no background cleanup is required (expired records may linger in the store but are ignored on evaluation).

**No renewal.** An expired remember record cannot be renewed by the agent. The operator must re-grant through the dialog to create a new record.

#### A3.4 Revocation

An operator principal with grant authority (RFC 0009 A1.3: `owner` or `admin` role) may revoke a remember record at any time before its natural expiry. Revocation is immediate:

1. The `CapabilityRememberRecord.revoked` field is set to `true`.
2. The `revoked_by` and `revoked_at_us` fields are populated.
3. Any session-capability cache entries for the same agent+capability in active sessions are also cleared.
4. If the agent currently holds a lease with that capability in its `capability_scope`, the capability is removed from the scope on the next lease evaluation cycle. If the lease scope would be empty after removal, the lease is revoked with `RevokeReason::CAPABILITY_REVOKED` (a new revoke reason value added by this amendment; see §A6).
5. An audit event `capability_revoked` is emitted (§A5).

#### A3.5 Storage

The remember store is a persistent, operator-local key-value store keyed by `(agent_namespace, capability)`. It is written to durable storage on the same commit path as the audit log (C17: local append-only, daily rotation). In v2, it is local-only; no federation synchronization of remember records is performed.

---

### A4. Interaction with the Lease Grant Flow

#### A4.1 Insertion Point in the Evaluation Sequence

RFC 0008 SS3.3 (`REQUESTED -> ACTIVE`) defines a six-step evaluation sequence. This amendment inserts a new step **0** before the existing steps:

```
Step 0 (NEW — C13 capability dialog gate):
  For each C13 capability in the requested capability_scope:
    a. If deployment-level config has the capability disabled:
       → DENY immediately (DenyReason::CAPABILITY_NOT_ENABLED).
    b. If a valid (non-expired, non-revoked) remember record exists for
       (agent_namespace, capability):
       → Gate passes for this capability; continue.
    c. If a session capability cache entry exists for (session_id,
       agent_namespace, capability):
       → Gate passes for this capability; continue.
    d. Otherwise:
       → Present operator dialog (§A2).
       → If operator grants: record session cache entry; optionally write
         remember record; continue.
       → If operator denies or timeout:
         → DENY (DenyReason::CAPABILITY_DIALOG_DENIED or
                 DenyReason::CAPABILITY_DIALOG_TIMEOUT).

Step 1 (existing): Agent session is Active.
Step 2 (existing): Agent holds required base capability (create_tiles).
Step 3 (existing): active_leases.len() < max_active_leases.
Step 4 (existing): Requested capability scope is a subset of session-granted capabilities.
Step 5 (existing): Requested lease_priority within capability ceiling.
Step 6 (existing): Requested resource_budget within session maximum.
```

**The dialog gate (step 0) runs before all existing checks.** A `LeaseRequest` that fails the dialog gate (operator denies) receives `DENIED` without proceeding to the existing checks. The existing checks are not visible to the operator during the dialog.

#### A4.2 Capability Scope Subset Check (Step 4)

The existing step 4 check — "requested capability scope is a subset of the agent's session-granted capabilities" — continues to apply after the dialog gate passes. Dialog approval establishes that the operator consents to the agent using the capability for this session; it does not bypass the session-granted capability ceiling. The session must also have the capability in its session-granted set (established at session establishment, RFC 0005 SS7).

This means capability grants flow from two independent sources:

1. **Session-level grant** (established at `SessionInit`, RFC 0005 SS7): the capability must be in the session's granted set.
2. **Per-session operator dialog** (this amendment): the operator must interactively consent for first-use.

Both must be satisfied. Neither alone is sufficient for C13 capabilities.

#### A4.3 Impact on < 1ms Latency Requirement

The lease grant latency requirement (RFC 0008 SS3.3: < 1ms) applies to the steady-state path where the dialog gate is skipped (remember record or session cache hit). The dialog gate itself introduces human-scale latency only on the first-use path; that latency is acceptable and expected.

Implementations MUST ensure that the remember-record and session-cache lookups in step 0 complete in < 100us under normal operating conditions to preserve the overall < 1ms budget for the common case.

---

### A5. Audit Events

Per C17 (signoff packet: mandatory audit events including capability grants/denials), the following three event types are added to the capability grant audit log (RFC 0009 SS13.3).

#### A5.1 `capability_granted`

Emitted when an operator grants a C13 capability through the dialog (whether or not "remember" is checked).

```json
{
  "event": "capability_granted",
  "agent_namespace": "agent-id-string",
  "capability": "media-ingress",
  "granted_by": "principal-uuid",
  "session_id": "session-uuid",
  "granted_at_us": 1234567890000000,
  "remember_written": false
}
```

If `remember_written` is `true`, the `CapabilityRememberRecord.expires_at_us` is also included:

```json
{
  "event": "capability_granted",
  "agent_namespace": "agent-id-string",
  "capability": "media-ingress",
  "granted_by": "principal-uuid",
  "session_id": "session-uuid",
  "granted_at_us": 1234567890000000,
  "remember_written": true,
  "remember_expires_at_us": 1235172690000000
}
```

#### A5.2 `capability_remembered`

Emitted when a 7-day remember record is successfully written (i.e., the operator checked "Remember for 7 days" and clicked "Grant").

```json
{
  "event": "capability_remembered",
  "agent_namespace": "agent-id-string",
  "capability": "microphone-ingress",
  "granted_by": "principal-uuid",
  "granted_at_us": 1234567890000000,
  "expires_at_us": 1235172690000000
}
```

#### A5.3 `capability_revoked`

Emitted when an operator manually revokes a remember record or when a capability is removed from a lease's scope as a result of revocation (§A3.4).

```json
{
  "event": "capability_revoked",
  "agent_namespace": "agent-id-string",
  "capability": "cloud-relay",
  "revoked_by": "principal-uuid",
  "revoked_at_us": 1234567890000000,
  "reason": "operator_manual_revoke"
}
```

When a lease's capability scope is narrowed as a result of revocation (§A3.4 step 4), an additional `LeaseStateChange` event is emitted on the `lease_changes` subscription category (RFC 0008 SS7.3) identifying the affected lease.

**Retention policy:** All three event types are subject to the C17 retention policy: 90-day default, operator-configurable, local append-only log, daily rotation, schema versioned.

---

### A6. Wire Protocol Additions

#### A6.1 New `DenyReason` Values

Three new `DenyReason` values are added to `LeaseResponse.DenyReason` (RFC 0008 SS10.2):

```protobuf
enum DenyReason {
  // ... existing values 0–8 unchanged ...
  CAPABILITY_NOT_ENABLED    = 9;   // Capability is disabled at deployment config level
  CAPABILITY_DIALOG_DENIED  = 10;  // Operator denied capability in the per-session dialog
  CAPABILITY_DIALOG_TIMEOUT = 11;  // Operator did not respond to dialog within timeout
  CAPABILITY_NOT_IMPLEMENTED = 12; // Capability is defined but not active in this runtime version (e.g., federated-send in v2)
}
```

#### A6.2 New `RevokeReason` Value

One new `RevokeReason` value is added to `LeaseResponse.RevokeReason` (RFC 0008 SS10.2):

```protobuf
enum RevokeReason {
  // ... existing values 0–6 unchanged ...
  CAPABILITY_REVOKED = 7;  // Operator revoked a capability that was in this lease's scope
}
```

#### A6.3 Dialog Timeout Configuration

A new configuration parameter is added to the parameter table (RFC 0008 SS11):

| Parameter | Default | Description |
|-----------|---------|-------------|
| `capability_dialog_timeout_ms` | 30,000 (30 s) | How long the runtime waits for an operator response to the capability dialog before denying the request with `CAPABILITY_DIALOG_TIMEOUT`. |

---

### A7. Interaction with C12 Role-Based Operators (RFC 0009 Amendment A1)

The capability dialog requires an operator action. Not all operator roles have authority to grant C13 capabilities. The following table governs dialog behavior by role:

| Operator Role | Dialog Behavior |
|--------------|-----------------|
| `owner` | May grant any C13 capability in the dialog. "Remember for 7 days" option is available. |
| `admin` | May grant any C13 capability in the dialog. "Remember for 7 days" option is available. |
| `member` | The dialog is **not shown to a `member` operator** for C13 capabilities. A `member` cannot grant new capabilities (RFC 0009 A1.3). If the only operator present at the screen is a `member`, the dialog times out and the request is denied with `CAPABILITY_DIALOG_TIMEOUT`. The timeout message MUST indicate that a `member`-role operator cannot grant this capability and that an `owner` or `admin` is required. |
| `guest` | A `guest` cannot grant new capabilities. Same behavior as `member`: dialog is not shown; request times out with `CAPABILITY_DIALOG_TIMEOUT`. |

**Escalation path:** When the runtime would show the dialog but no operator with `owner` or `admin` role is present, the runtime SHOULD show a notification that an owner/admin authorization is needed (using the system notification mechanism, RFC 0007 SS6) rather than silently timing out. This notification is informational only; the `LeaseRequest` is still denied on timeout.

**Implementation note:** The runtime identity store (RFC 0009 A1.2) is the source of truth for the present operator's role. The capability dialog subsystem MUST query the identity store at dialog-show time to determine which roles are available in the current session context.

---

### A8. Cross-Reference to Identity-and-Roles Capability Spec (hud-ora8.2.5)

The `identity-and-roles` capability spec (to be authored as `openspec/changes/v2-embodied-media-presence/specs/identity-and-roles/spec.md`, issue hud-ora8.2.5) is the **authoritative source** for:

- The complete role definitions and user-directory schema (extended from RFC 0009 A1.1).
- The role-to-capability binding tables (which capabilities each role may grant, revoke, or be prompted for).
- The principal identity wire format and `PrincipalId` type.
- The operator identity store persistence model.

This amendment is the **lease-governance surface** of that spec. The full role model lives in hud-ora8.2.5. The two documents MUST be read together for a complete picture of capability grant authority.

**Forward reference:** Until hud-ora8.2.5 is authored, the role-to-authority table in RFC 0009 Amendment A1 (§A1.3) is the operative source for role-grant authority. This amendment's §A7 table is consistent with and derived from that table.

---

## §15 Open Questions Interaction (Addendum)

This amendment partially addresses RFC 0008 §15 Open Question 1 (budget renegotiation mid-lease):

> V1 does not support changing a lease's `capability_scope` after grant.

C13 introduces a case where a lease's `capability_scope` is **narrowed** mid-lease (§A3.4 revocation). This is a one-way operation (scope can be reduced but not expanded mid-lease). The general case of scope expansion via mid-lease renegotiation remains a post-v1 open question. This amendment does not close the question but establishes the narrowing path as a defined operation.

---

## §18 Related RFCs Update (Addendum)

The following row is addended to the §18 related RFCs table in RFC 0008:

| RFC / Spec | Relationship |
|-----------|-------------|
| RFC 0009 Amendment A1 (C12 role-based operators) | The per-session capability dialog (§A2) requires operator role authority to grant C13 capabilities. `owner` and `admin` roles may grant; `member` and `guest` may not (§A7). Role definitions and user-directory schema are authoritative in RFC 0009 A1 and hud-ora8.2.5. |
| `openspec/changes/v2-embodied-media-presence/specs/identity-and-roles/` (hud-ora8.2.5) | Authoritative source for role definitions, principal identity store, and role-to-capability binding. This amendment is the lease-governance surface of that spec. |
| RFC 0007 (System Shell) | The capability dialog is presented through the system shell chrome layer (RFC 0007 SS6). The dialog MUST be rendered at chrome-layer priority (priority 0) so it cannot be occluded by agent tiles. |
| RFC 0009 (Policy Arbitration) | Capability grant audit events from §A5 use the audit log infrastructure defined in RFC 0009 SS13.3. |

---

## §19 Review Record Update (Addendum)

| Round | Date | Reviewer | Focus | Changes |
|-------|------|----------|-------|---------|
| A1 | 2026-04-19 | hud-ora8.1.11 | Amendment: C13 capability dialog + 7-day remember | Added §A1 capability taxonomy (8 C13 capabilities). Added §A2 per-session operator dialog flow (dialog state machine, session capability cache). Added §A3 7-day remember data model (scope, expiry, revocation, storage). Added §A4 insertion into lease grant flow (new evaluation step 0). Added §A5 audit events (`capability_granted`, `capability_remembered`, `capability_revoked`). Added §A6 wire protocol additions (4 new `DenyReason` values, 1 new `RevokeReason` value, `capability_dialog_timeout_ms` config). Added §A7 role-based operator interaction (owner/admin grant; member/guest cannot grant). Added §A8 cross-reference to hud-ora8.2.5. Full amendment document: `about/legends-and-lore/rfcs/reviews/0008-amendment-c13-capability-dialog.md` (issue hud-ora8.1.11). |
