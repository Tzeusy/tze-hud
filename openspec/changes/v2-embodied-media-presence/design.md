## Context

V1 proves a bounded thesis: governed tiles/zones/widgets, deterministic timing, lease authority, and local-first interaction. The doctrine also makes clear that the long-horizon target includes media, synchronization, richer interaction, and broader device forms. A credible v2 program must extend the existing architecture rather than replace it.

The non-negotiables remain:

1. The screen is sovereign.
2. LLMs do not sit in the frame loop.
3. Arrival time is not presentation time.
4. Human/operator override always wins.

## Design Decision

V2 is a **phased capability expansion**, not a monolithic release:

1. **Phase 1: Bounded media activation**
   - promote post-v1 bounded ingress from contract-only to implemented capability
   - keep default-off and operator-governed activation
   - prove reconnect, privacy, and degradation behavior under validation
2. **Phase 2: Embodied presence**
   - introduce embodied session semantics and a device-aware presence model
   - bind media lifecycle to governed embodied sessions rather than ad hoc streams
3. **Phase 3: Device execution**
   - make mobile/glasses profiles exercised and measurable
   - define upstream-composition and degraded render paths explicitly
4. **Phase 4: Broader AV and orchestration**
   - only after the earlier phases are stable, admit bidirectional AV, multi-feed coordination, and richer orchestration

## Key Boundaries

### Media stays governed

Media is not a shortcut around leases, capability grants, privacy ceilings, or degradation ladders. Every admitted stream must remain tied to explicit authority and operator policy.

### Embodiment is stronger than residency

Embodied presence is not "resident plus video." It requires stronger session identity, richer failure handling, explicit operator visibility, and device-aware routing.

### Device profiles are execution targets

The mobile/glasses story only becomes real when the repo can validate and operate those profiles directly. V2 therefore treats device-profile execution as a first-class workstream, not a documentation footnote.

## Risks

1. Scope inflation if bounded ingress and bidirectional AV are collapsed into one tranche.
2. Validation debt if real-decode/device paths are planned without runner strategy and artifact contracts.
3. Governance drift if media/device implementation bypasses existing operator/privacy/failure doctrine.

## Reconciled Constraints

The following constraints are normative for this v2 design and come directly from existing doctrine/contracts:

1. **Do not widen v1 promises.** No v1 capability claim changes are implied by this change.
   Source: `about/heart-and-soul/v1.md`
2. **Phase 1 stays bounded.** No audio, no bidirectional AV session semantics, no multi-feed orchestration in bounded-ingress activation.
   Source: `openspec/specs/media-webrtc-bounded-ingress/spec.md`
3. **Admission precedence is preserved.** Explicit enablement and operator disable checks remain ahead of viewer/privacy and transport checks.
   Source: `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`
4. **Runtime remains sovereign.** Agents never bypass leases/capabilities/policy gates to control presentation directly.
   Source: `about/heart-and-soul/architecture.md`, `about/heart-and-soul/presence.md`
5. **Validation is release-gating, not aspirational.** Later phases remain blocked unless phase-specific deterministic and higher-fidelity evidence both exist.
   Source: `about/heart-and-soul/validation.md`
