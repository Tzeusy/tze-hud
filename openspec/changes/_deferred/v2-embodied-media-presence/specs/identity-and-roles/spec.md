# identity-and-roles Capability Spec

**Change:** v2-embodied-media-presence
**Phase:** 2 (Embodied Presence)
**Status:** scaffold — phase 1 only; requirements to be fleshed out in phase 2
**Source decision:** C12 (signoff-packet.md)

---

## Purpose

This spec owns the operator identity model for tze_hud installations: role
taxonomy, role-to-capability binding, the user directory schema, and the
federation-aware data model that makes cross-operator federation possible in
a future phase without wire-breaking changes.

An operator installation has exactly one owner, zero or more admins, zero or more
members, and any number of transient guests. Roles govern the authority of human
principals to issue and modify capability grants for agent sessions; they are
distinct from the capability grants themselves. Agents operate under capability
grants established at session handshake (RFC 0005, RFC 0008); roles govern which
human operators may authorize those grants.

The policy-arbitration surface of this spec (how roles intersect Level 3 Security
and Level 0 Human Override) is already established by RFC 0009 Amendment 1.
Phase 2 will flesh out the full role model, the user-directory wire format, and
the mutation and lookup flows that back the arbitration layer.

Phase 2 will flesh out each requirement below. The scaffold records decision intent
from C12 so that RFC authorship, bead scoping, and cross-spec dependencies can
proceed.

---

## Requirements

### Requirement: Role Taxonomy

Will specify: the authoritative semantic definition of each of the four operator
roles (owner / admin / member / guest) in spec form, complementing the policy-level
role table in RFC 0009 Amendment 1 (A1.1). This includes: exactly-one-owner
invariant for v2, the admin authority boundary (no ownership promotion), member
personal-preference scope, and guest ephemerality constraints. The role promotion
and demotion rules, role conflict semantics, and the owner succession path in the
event of the sole owner principal being removed are TBD — phase 2.

Source: C12 (signoff-packet.md), RFC 0009 Amendment 1 §A1.1
Scope: v2-phase-2

#### Scenario: exactly one owner per installation (placeholder)

- **WHEN** an attempt is made to promote a second principal to `owner`
- **THEN** the runtime rejects the promotion with a configuration error (details TBD — phase 2)

---

### Requirement: Role-to-Capability Binding

Will specify: the authoritative table of which roles may grant or revoke which
capabilities from the RFC 0008 Amendment 1 capability set (media-ingress,
microphone-ingress, audio-emit, recording, cloud-relay, external-transcode,
federated-send, agent-to-agent-media) and from the RFC 0009 §8.1 canonical
capability registry. The binding table will complement RFC 0009 A1.3 which
establishes the Level 3 policy-arbitration impact; this spec owns the complete
binding matrix. Default capability profiles per role (i.e., what an owner
sees by default vs. what a member sees) are TBD — phase 2.

Source: C12 (signoff-packet.md), RFC 0009 Amendment 1 §A1.3, RFC 0008 Amendment 1
Scope: v2-phase-2

#### Scenario: member cannot grant media capabilities (placeholder)

- **WHEN** a `member` principal attempts to grant `stream_media` to an agent session
- **THEN** the runtime rejects the grant at Level 3 with `CapabilityDenied` (details TBD — phase 2)

---

### Requirement: User Directory Schema

Will specify: the full wire format and storage schema for `OperatorPrincipal`
records (id, display_name, role, origin, devices) as introduced in RFC 0009 A1.2,
the lookup and mutation API (create, update, deactivate), and the runtime
lifecycle of those records (initialization on first boot, migration path from
the single-owner default, persistence guarantees). The `PrincipalId` format
(local UUID in v2), the index structure for fast lookup at Level 3 arbitration,
and the persistence layer are TBD — phase 2.

Source: C12 (signoff-packet.md), RFC 0009 Amendment 1 §A1.2
Scope: v2-phase-2

#### Scenario: principal lookup at capability grant time (placeholder)

- **WHEN** an operator-initiated capability grant arrives for a session
- **THEN** the runtime resolves the authorizing principal from the identity store
  and validates their role before the grant is applied (details TBD — phase 2)

---

### Requirement: Federation-Aware Data Model

Will specify: the behavior of `PrincipalOrigin::Federated` entries in the data
model — specifically the load-time rejection path (v2 enforces: any principal
record with `origin = Federated` is rejected with a configuration error), the
reserved field layout that prevents wire-breaking changes when federation is
activated in a future phase, and the migration contract from v2 local-only to
post-v2 federated. The cross-operator role-merge policy, federated capability
delegation rules, and the DID wire format for federated `PrincipalId` are
deferred to post-v2 and are TBD — phase 2.

Source: C12 (signoff-packet.md, "Federation-aware roles modeled in the data model
but not fully enforced in v2"), RFC 0009 Amendment 1 §A1.2 and §A1.5
Scope: v2-phase-2

#### Scenario: federated principal rejected at load time (placeholder)

- **WHEN** the runtime loads a principal record with `origin = Federated`
- **THEN** the runtime emits a configuration error and does not admit the record
  (federation enforcement is not active in v2 — details TBD — phase 2)

---

## Cross-References

- **RFC 0009 Amendment 1 (policy arbitration)** — the authoritative policy-
  arbitration surface of the role model. Defines role taxonomy (A1.1), the
  `OperatorPrincipal` / `OperatorRole` / `PrincipalOrigin` data model (A1.2),
  Level 3 and Level 0 arbitration impact (A1.3), audit event types (A1.4), and
  the v2 non-enforcement note for federation (A1.5). This spec extends RFC 0009 A1
  with the full user-directory schema and role-to-capability binding matrix.
- **RFC 0008 Amendment 1 (capability dialog, C13)** — defines the eight
  v2-gated capabilities (media-ingress, microphone-ingress, audio-emit, recording,
  cloud-relay, external-transcode, federated-send, agent-to-agent-media) and the
  role-based interaction at the capability dialog: only `owner` / `admin` may
  grant; `member` / `guest` cannot. This spec's role-to-capability binding table
  will subsume and extend that interaction.
- **`identity-portability/spec.md`** — sibling capability spec that owns
  device-reboot-persistent embodied identity and the cryptographic device-migration
  ritual (B9). Identity-portability keys must be resolvable against the
  identity-and-roles user directory: portability export/import is an operator-
  authorized action and the authorizing principal must satisfy the role check
  defined in this spec.
- **`about/heart-and-soul/security.md`** — operator agent-isolation posture;
  the operator identity store must satisfy the isolation guarantees stated there.
  Specifically: agent sessions must not be able to read or mutate principal records
  directly — all principal mutations flow through operator-initiated actions
  (Level 0 / Level 3 gated).

---

## Open Questions

All items below are TBD and deferred to phase 2 elaboration:

1. **Role transition flow** — what is the exact sequence for promoting a `member`
   to `admin`, or for the owner demoting an admin? Is there a confirmation ritual,
   an audit event, a cooldown period? The transition must be atomic from the
   operator's perspective but the exact wire and UX contract are undefined.
2. **Role expiration** — should `guest` (and optionally `member`) roles have an
   expiry field? A guest with a standing permanent record is operationally different
   from a transient guest. Expiry semantics, the cleanup trigger, and the behavior
   of in-flight sessions when a guest record expires are TBD.
3. **Audit semantics for role mutations** — RFC 0009 A1.4 defines `role_grant` and
   `role_revoke` events, but it does not specify whether role mutations are
   synchronous audit (blocked until the write is durable) or async audit
   (written best-effort). Given C17's 90-day append-only retention requirement,
   the durability contract for role-mutation events needs explicit spec language.
4. **Multi-owner support** — signoff-packet C12 says "exactly one owner per
   installation in v2." The post-v2 path to multi-owner must not require a breaking
   schema change. The scaffold for a multi-owner data model (e.g., a list field
   reserved for future use) should be weighed against data-model simplicity.
5. **Role inheritance or delegation** — can an `admin` create a scoped sub-admin
   role that can only manage a subset of capabilities? The v2 role set is flat
   (owner / admin / member / guest), but installations with larger user pools
   may need intermediate roles. Whether this is a v2 concern or post-v2 is TBD.
6. **Default role for new principals** — when the runtime performs first-boot
   initialization and creates the initial owner, what is the path for subsequent
   principal creation? Is there a default role for newly-invited principals before
   the owner assigns one? The bootstrap ceremony and the default-role policy are
   undefined.
7. **Session identity vs. principal identity** — RFC 0005 session handshake
   does not carry a role field (RFC 0009 A1.5: "roles are a runtime identity store
   concern, not a per-session wire field"). The mechanism by which a session is
   associated with a principal at runtime — for the purpose of the Level 3
   role-check — is unspecified. Whether this is a config-time binding, an
   operator-action-at-session-start, or a cryptographic challenge is TBD.
