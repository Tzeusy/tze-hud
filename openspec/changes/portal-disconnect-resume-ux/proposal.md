## Why

The text-stream portal is the project's exemplar end-to-end flow (epic `hud-bq0gl`),
but it has a design hole on its least-happy path: what the **viewer** sees when the
driving session's gRPC stream or LLM session drops mid-stream, and what the portal
restores when a session re-attaches after death.

The *authority-side* of this is already specified. `cooperative-hud-projection`
(`openspec/specs/cooperative-hud-projection/spec.md` §External Projection State
Authority) requires the projection authority to preserve "pending input,
acknowledgement state, lifecycle state, and the latest coherent visible transcript
window" across a reconnect, and `external-agent-projection-authority`
(`openspec/specs/external-agent-projection-authority/spec.md` §Multi-Session
Lifecycle Management) tracks `last connection state` + `reconnect bookkeeping`. The
runtime already models a degraded connection: `ProjectionLifecycleState` carries
`Degraded`, `HudUnavailable`, and `Detached` variants
(`crates/tze_hud_projection/src/contract.rs` §ProjectionLifecycleState), and
`ProjectionAuthority::mark_hud_disconnected` transitions a session to
`HudUnavailable` and records `last_disconnect_wall_us`
(`crates/tze_hud_projection/src/authority.rs`). The hud-projection skill documents
detach/re-attach mechanics and owner-token loss recovery
(`.claude/skills/hud-projection/SKILL.md`).

What is **unspecified** is the viewer-facing contract. The only existing
`text-stream-portals` (`openspec/specs/text-stream-portals/spec.md`) coverage is one
scenario under §Governance, Privacy, and Override Compliance ("disconnected portal
follows orphan path"), which says the surface "SHALL freeze at its last coherent
state or runtime placeholder policy" and is removed on grace expiry. That is a
governance/lease statement, not a UX contract. It does not say:

- what the portal *renders* during the disconnect window (stale-content indicator,
  dimming, disconnect affordance) before grace expiry,
- when content is considered *stale* vs. live (the degradation contract / timeouts),
- what a re-attach *restores* vs. clears on the viewer surface, including
  `logical_unit_id` continuity and coalesce semantics.

Doctrine is explicit that this matters: "Treating graceful degradation as a bug" is
an anti-pattern (`CLAUDE.md`, `about/heart-and-soul`). A portal that simply blanks,
or silently shows stale text as if live, violates that doctrine. This change closes
the design-coverage gap with a precise viewer-facing specification.

## What Changes

Three ADDED requirements on `text-stream-portals`:

- **Portal Disconnect Presentation** — what the portal renders when the driving
  stream/session drops mid-stream: the last coherent transcript window is retained
  but visibly marked stale via a token-resolved degraded treatment (no hardcoded
  styling), a geometry-only disconnect indicator that survives redaction, and no
  loss of already-committed transcript units. Activity/typing indicators clear; the
  surface does not fabricate liveness.

- **Portal Stale-Content Degradation Contract** — the timeout/staleness contract:
  the connection is degraded after a bounded liveness gap, content becomes "stale"
  on entering the degraded window, the degraded window is bounded by the existing
  lease grace, and presentation timing of the degraded transition is runtime-owned
  (arrival time ≠ presentation time). Defines the relationship to lease orphan/grace
  expiry rather than inventing a second lifecycle.

- **Portal Reconnect and Resume Presentation** — what a re-attach restores vs.
  clears: on reconnect before grace expiry the portal resumes from the retained
  coherent window and clears the stale treatment; identity continuity uses the
  authority's existing keys — `logical_unit_id` stays idempotency-only (a replayed id
  is a no-op) while an in-place continuation reuses the unit's `coalesce_key`, so a
  resumed/continued logical unit updates in place rather than duplicating without
  redefining `logical_unit_id` semantics; resume
  appends coalesce under the existing state-stream rules; after grace expiry (session
  death) the surface is gone and a fresh attach starts a new portal rather than
  silently reviving stale content.

One MODIFIED requirement:

- **Governance, Privacy, and Override Compliance** — the existing "disconnected
  portal follows orphan path" scenario is extended to cross-reference the new
  viewer-facing disconnect/resume contract, so the lease orphan lifecycle and the UX
  contract stay coherent.

## What Does Not Change

- No new transport, no second portal stream, no portal-specific disconnect RPC: the
  disconnect signal is the existing primary session stream dropping, surfaced through
  the existing `ProjectionLifecycleState`/orphan-lease path.
- No scene-graph transcript history: resume restores only the bounded retained
  visible window the authority already keeps, materialized per the existing Bounded
  Transcript Viewport rule.
- No change to lease grace/orphan governance: staleness is bounded *by* the existing
  lease grace, not by a new timer authority.
- No hardcoded styling: every degraded/stale/disconnect treatment resolves from
  design tokens via the existing component-profile path.

## Non-Goals

- Terminal-style reconnection (PTY re-attach, byte-stream replay) — out of scope per
  RFC 0013 §1.2 and the standing portal non-goals.
- Owner-token recovery — that is the skill/authority concern already covered by
  `cooperative-hud-projection`; this change is purely the viewer surface.
- Cross-host transcript persistence — v1 projection state is memory-only per
  `cooperative-hud-projection` §External Projection State Authority; resume after a
  *daemon/host restart* (not just a stream drop) legitimately starts fresh.
