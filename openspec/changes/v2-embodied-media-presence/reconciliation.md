# V2 Embodied/Media Reconciliation Matrix

Date: 2026-04-09
Issue: `hud-8cy3.1`
Change: `openspec/changes/v2-embodied-media-presence`

## Purpose

Reconcile the v2 embodied/media draft against:

1. project doctrine (`about/heart-and-soul/*`),
2. existing post-v1 bounded ingress and privacy/operator contracts,
3. relevant RFC-level ownership/governance constraints.

This artifact defines what the v2 change is allowed to claim now, and what remains deferred until later phases.

## Reconciliation Decisions

| Area | Existing source of truth | Reconciled v2 position | Status |
|---|---|---|---|
| V1 scope boundary | `about/heart-and-soul/v1.md` | v2 change does not widen v1 promises; media/embodied/mobile execution stay post-v1 workstreams | Aligned |
| Screen sovereignty | `about/heart-and-soul/architecture.md` | Runtime remains authority owner for timing, composition, admission, and revocation; agents do not bypass governance | Aligned |
| First admissible media slice | `openspec/specs/media-webrtc-bounded-ingress/spec.md` | Phase 1 remains bounded ingress only; no implicit jump to bidirectional AV/audio/multi-feed | Aligned |
| Privacy/operator precedence | `openspec/specs/media-webrtc-privacy-operator-policy/spec.md` | v2 phases preserve explicit enablement + operator override + viewer/privacy short-circuit behavior | Aligned |
| Presence model | `about/heart-and-soul/presence.md`, RFC 0008/0009 | Embodied presence is treated as a stronger governed state, not a transport shortcut | Aligned |
| Device profile claims | `about/heart-and-soul/mobile.md`, `about/heart-and-soul/v1.md` | v2 makes device profiles execution targets in later phases; no claim that v1 already exercises them | Aligned |
| Validation doctrine | `about/heart-and-soul/validation.md` | each phase requires deterministic + higher-fidelity evidence before activation claims | Aligned |

## Explicit Non-Goals (for this bead)

1. Defining final wire-level bidirectional AV session machines in this bead.
2. Claiming audio routing/mixing policy is complete.
3. Claiming mobile/glasses runtime execution already ships.
4. Changing v1 doctrine or v1 capability specs.

## Delta Applied in This Bead

1. Added doctrine/contract reconciliation sections to:
   - `proposal.md`
   - `design.md`
2. Added explicit `Source` and `Scope: post-v1` traceability to all v2 delta requirements in:
   - `specs/media-plane/spec.md`
   - `specs/presence-orchestration/spec.md`
   - `specs/device-profiles/spec.md`
   - `specs/validation-operations/spec.md`

## Follow-On Boundary

This bead reconciles the v2 draft contract surfaces. It does not produce the phased execution plan (`hud-8cy3.2`), program bead graph (`hud-8cy3.3`), or final package reconciliation (`hud-8cy3.4`).
