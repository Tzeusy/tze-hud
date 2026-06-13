## Context

The active v2 change includes useful validation-operations requirements, but the portable part is not inherently v2 media work. The portable part is the operational validation backlog inherited from the archived `v1-mvp-standards` change plus the cross-spec conformance checks needed before larger capability expansion.

Keeping that material inside the v2 program creates two problems:

1. The canonical `validation-framework` spec cannot receive the carry-forward work independently.
2. Reviewers must separate baseline validation obligations from media/device-specific release gates each time the v2 program changes.

## Design Decision

This change treats validation operations as a modification to the existing `validation-framework` capability, not as a new capability. The delta adds three narrow requirement groups:

1. **Carry-forward closure.** Make the v1 deferred validation backlog an explicit validation-framework obligation rather than a note embedded in v2 planning.
2. **Cross-spec conformance audits.** Gate capability vocabulary, MCP authority surfaces, and session-envelope field allocation parity before future capability expansion relies on those contracts.
3. **Canonical reconciliation.** Require reconciliation against `openspec/specs/validation-framework/spec.md` so archived, canonical, and v2-derived language converge instead of duplicating or drifting.

## Boundaries

This change intentionally does not define:

- D18 real-decode media budgets or runner cadence.
- D19/D20 physical device lane coverage.
- Media admission, teardown, operator override, cloud relay, recording, or bidirectional AV observability.
- Embodied-presence release phases or v2 sequencing gates.

Those remain in the v2 program or later media/device-specific changes.

## Risks

- Some carry-forward requirements already exist in canonical `validation-framework`; archive must merge by intent, not blindly duplicate text.
- Conformance audits may expose active drift in protocol, MCP, or configuration surfaces. Those findings should become implementation/reconciliation beads rather than expanding this spec-only change.
