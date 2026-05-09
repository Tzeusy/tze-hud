# identity-portability Capability Spec

**Change:** v2-embodied-media-presence
**Phase:** 2 (Embodied Presence)
**Status:** scaffold — phase 1 only; requirements to be fleshed out in phase 2
**Source decision:** B9 (signoff-packet.md)

---

## Purpose

This spec governs device-reboot-persistent identity for embodied agents, and the
operator-initiated cryptographic ritual by which that identity can be safely migrated
to a new device without relying on a cloud identity anchor.

An embodied agent's session identity MUST survive device reboots and MUST remain
portable across device replacements through a user-initiated export/import mechanism.
This spec owns the key material format, the pairing ritual UX, and the operator-facing
device-migration flow. It is a companion to `presence-orchestration` (which owns the
embodied session contract) and to `identity-and-roles` (which owns role definitions and
user directory schema).

Phase 2 will flesh out each requirement below. The scaffold records decision intent
from B9 so that RFC authorship, bead scoping, and cross-spec dependencies can proceed.

---

## Requirements

### Requirement: Key Material Format

Will specify: what cryptographic material constitutes an embodied identity, how it is
stored locally (key store path, format, and access controls), and what guarantees are
required for reboot persistence. Key material MUST survive a device reboot without
cloud storage. Specific algorithm choices, encoding, and rotation policy are TBD —
phase 2.

Source: B9 (signoff-packet.md)
Scope: v2-phase-2

#### Scenario: identity survives reboot (placeholder)

- **WHEN** a device running an embodied session is rebooted
- **THEN** the embodied identity key material is present and valid after restart (details TBD — phase 2)

---

### Requirement: Pairing Ritual UX

Will specify: the user-visible flow for the "pair new device" ritual, including what the
operator must confirm, what is shown to the household, and what invariants must hold
during the pairing window (e.g., only one outstanding pairing ritual at a time). The
ritual MUST be operator-initiated and MUST NOT be silently triggered by agent activity.
Full UX contract and error states are TBD — phase 2.

Source: B9 (signoff-packet.md)
Scope: v2-phase-2

#### Scenario: pairing is operator-initiated (placeholder)

- **WHEN** a device migration is required
- **THEN** the operator must explicitly trigger the pairing ritual before any key export begins (details TBD — phase 2)

---

### Requirement: Operator Device-Migration Flow

Will specify: the end-to-end operator flow for migrating an embodied identity from one
device to another — export package format, import procedure on the new device, and
revocation of the old device after a successful import. The old device MUST be revoked
as part of the migration flow. Migration MUST be atomic from the operator's perspective:
the old identity is not revoked until the new device confirms successful import. Full
protocol and rollback conditions are TBD — phase 2.

Source: B9 (signoff-packet.md)
Scope: v2-phase-2

#### Scenario: old device revoked after migration (placeholder)

- **WHEN** the operator completes a device migration and the new device confirms import
- **THEN** the old device's identity is revoked and can no longer establish embodied sessions (details TBD — phase 2)

---

### Requirement: Device-Reboot Persistence

Will specify: runtime guarantees that embodied identity is available after a reboot
without user intervention (i.e., no re-pairing required for normal reboots). The key
store MUST be durable across normal OS-level restarts. Conditions under which identity
is intentionally cleared (factory reset, operator wipe) are TBD — phase 2.

Source: B9 (signoff-packet.md)
Scope: v2-phase-2

#### Scenario: no re-pairing on normal reboot (placeholder)

- **WHEN** a device undergoes a normal reboot
- **THEN** the runtime recovers the embodied identity automatically without requiring a new pairing ritual (details TBD — phase 2)

---

## Cross-References

- **RFC TBD (crypto / key material)** — the RFC governing the specific cryptographic
  algorithms, key derivation, and storage guarantees will be authored in phase 2.
  Candidate: a new RFC 0020 or an amendment to RFC 0015 (Embodied Presence Contract).
- **`identity-and-roles/spec.md`** — companion capability spec that owns role
  definitions, user directory schema, and role-to-capability binding. Identity-portability
  keys must be resolvable against the identity-and-roles user model.
- **`presence-orchestration/spec.md`** — owns the embodied session contract and the
  session-level semantics of identity (B6, B7, B8, B10, B16). Identity-portability keys
  are issued and recognized in the context of embodied sessions defined there.
- **RFC 0015 (Embodied Presence Contract)** — will contain the wire-level export/import
  message types and the revocation event for device migration.
- **`about/heart-and-soul/security.md`** — operator agent-isolation posture must be
  consulted before finalizing key-store access controls (per open item 3 in
  signoff-packet.md).
- **`about/heart-and-soul/embodied.md`** — doctrine file for embodied presence (to be
  authored in phase 2); will record the portability ethic and the "no cloud anchor" rule
  as a prime directive.

---

## Open Questions

All items below are TBD and deferred to phase 2 elaboration:

1. **Crypto algorithm choice** — which key type (e.g., Ed25519, P-256) and what
   derivation / wrapping scheme? Depends on RFC 0015 / RFC TBD authorship.
2. **Export package format** — encrypted blob vs. QR code vs. NFC tap? UX and
   transport medium for the pairing ritual are unspecified.
3. **Partial revocation / multi-device** — signoff-packet (B9) is explicit that multi-device
   embodied presence is post-v2 (A4); this spec must not inadvertently leave hooks that
   encourage multi-device operation before the governance model supports it.
4. **OS key store integration** — should the runtime use OS-provided secure enclaves
   (Keychain, Android Keystore, TPM) or a runtime-owned local key file? Security/threat
   model to be assessed in phase 2 against `security.md`.
5. **Rollback and failure modes** — what happens if the new device crashes mid-import
   before revocation of the old device? Recovery protocol is TBD.
6. **Audit log entries** — pairing rituals, migration completions, and old-device
   revocations are candidate mandatory audit events (per C17); exact event schema TBD
   pending RFC 0019 (Audit Log Schema).
