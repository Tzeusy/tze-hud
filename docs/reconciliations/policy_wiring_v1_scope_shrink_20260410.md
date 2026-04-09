# Policy Wiring v1 Scope Shrink (Closeout)

Date: 2026-04-10
Issue: `hud-s98v.4`

## Decision

Adopt `shrink-v1-claims` for policy wiring closeout.

V1 truth is runtime/session/scene-owned enforcement. Unified `tze_hud_policy` hot-path wiring is explicitly deferred to post-v1 (v2) and is not a shipped v1 requirement.

## Spec Updates Applied

1. `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
   - Reframed unified policy wiring as deferred beyond v1.
   - Converted residual `v1-reserved` unified-stack requirements to `post-v1`.
   - Replaced stale bead-specific handoff notes with a release-train closeout note.
2. `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
   - Replaced stale handoff-note section with explicit v1 deferral note for centralized policy hot-path execution.
3. `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
   - Replaced stale handoff-note section with explicit v1 authority/deferral note aligned to shipped session semantics.

## Outcome

Residual policy language now matches shipped v1 behavior and no longer implies unified policy hot-path wiring as an in-scope v1 deliverable.
