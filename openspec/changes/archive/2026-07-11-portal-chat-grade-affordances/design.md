# Design: portal-chat-grade-affordances

## Context

The projection contract already carries everything these affordances need: `InputDeliveryState` (Pending/Delivered/Deferred/Handled/Rejected/Expired) per submitted input, `unread_output_count`, `appended_at_wall_us` per transcript unit, and `connection_degraded` — all flowing into `ProjectedPortalState`. Nothing renders them. Epic hud-g0c9g mandates spec-first: this change is the STEP 1 spec delta; STEP 2 (rendering) folds into the promotion epic hud-g1ena / hud-s4lrw, which owns the multi-node turn model these cues attach to.

## Goals / Non-Goals

**Goals**
- Specify the seven chat-grade affordances as testable requirements consistent with the existing attention, interaction, privacy, and viewer-echo requirements.
- Keep every cue ambient by construction — no requirement may create a new attention-escalation path.
- Require zero new adapter protocol: all cues derive from state the runtime already tracks.

**Non-Goals**
- No rendering/implementation in this change (promotion owns it).
- No read receipts to the adapter: the viewer's seen state is never disclosed; delivery cues render adapter→runtime acknowledgement state only.
- No new MCP/gRPC surface, no per-message reactions/editing/threading (out of scope for v1 portal doctrine).

## Decisions

1. **Three presentation classes for six delivery states.** Pending/Deferred → in-flight, Delivered/Handled → delivered, Rejected/Expired → failed. Rationale: viewers need "did it land?", not queue mechanics; component profiles may refine within a class but the spec mandates only the three-way distinction. Alternative (render all six) rejected as debug-console UX.
2. **Unread divider counts agent turns only, sits at oldest retained unseen unit.** Follows directly from §Viewer Reply Echo (echo never unread) and §Bounded Transcript Viewport (count may exceed retained units; divider cannot point outside the window).
3. **Local-first unread clearing.** Viewing the tail clears divider/count locally with no adapter round trip — symmetric with local-first scroll, and required because clearing must work while degraded.
4. **Activity cue derives from observed appends, not a typing protocol message.** Alternative (adapter-sent typing events à la Telegram) rejected: adds adapter protocol surface, invites notification-style abuse, and contradicts External Adapter Isolation minimalism. Observed streaming appends are a sufficient, unforgeable signal.
5. **Timestamps: runtime-assigned wall-clock only.** `appended_at_wall_us` is runtime-assigned at append; adapters cannot forge arrival times. Granularity/visibility left to component profile + tokens (pixel-level mandates would violate visual-identity modularity).
6. **Connecting is a presentation distinction, not a new lifecycle state.** The lease/orphan lifecycle is untouched; the runtime distinguishes "never connected since attach" from "connected then dropped" for treatment selection only. Implementation will likely need a `has_ever_connected` bit alongside `connection_degraded` — a contract-level detail deferred to promotion.
7. **Jump-to-latest is the only tail-follow resume affordance an adapter can never trigger.** Reinforces §Transcript Interaction Contract viewport authority; the affordance may carry the unread count so the two cues compose instead of competing.
8. **Two-pane INPUT/OUTPUT split for viewer echo (owner live round-6, 2026-07-04).** §Viewer Reply Echo originally framed the echo as a viewer-authored turn appended into the one retained transcript stream. Live testing (hud-egf39 / PR #1038) established that this double-showed the viewer's text: the raw-tile path already renders the viewer-echo stack beneath the composer (#1020/hud-hsc1t), while #1027/#1031 additionally appended the draft into the OUTPUT transcript (`body_full`), so the same words appeared in both places. Decision: the portal tracks two separately-bounded histories — INPUT (viewer's own submissions, beneath a top-anchored composer, stacked with `---` dividers) and OUTPUT (agent-authored only). Viewer submissions echo into INPUT and never touch the OUTPUT transcript or its scroll. This is a reframe of the echo *destination*, not a relaxation of its guarantees: runtime-authored, kind-distinct, never-unread, redaction parity, transactional delivery, and rejected-not-echoed all carry over unchanged. The alternative (keep a single combined stream and de-dup the double-append) was rejected because it fights the surface's chat mental model and re-entangles viewer intent with the agent's transcript.

## Risks / Trade-offs

- [Spec ahead of turn model] The cues attach to per-turn presentation that only exists after promotion's multi-node turn model. → Mitigation: every requirement is scoped `promotion (rendering under hud-g1ena)`; nothing blocks pre-promotion work.
- [Ambient-by-spec, loud-by-implementation] A profile could still render these cues noisily. → Mitigation: each requirement explicitly subordinates to §Ambient Portal Attention Defaults, making noisy treatments spec violations, not taste disagreements.
- [Divider drift under coalescing] Coalesced windows may drop the exact boundary unit. → Mitigation: spec pins divider to oldest *retained* unseen unit and lets count exceed visible units.

## Migration Plan

Spec-only: validate with `openspec validate --strict`, land on main, then `/opsx:sync` the delta into `openspec/specs/text-stream-portals/spec.md`. File promotion-scoped rendering beads under hud-g1ena and close hud-g0c9g's STEP 1. Rollback = archive the change without sync.

## Open Questions

- Whether `has_ever_connected` lands in `PortalStatusSnapshot` or is derived in the driver — decide during promotion implementation.
- Whether the collapsed-presentation unread count shares a token group with the jump-to-latest count badge — decide in the visual-token compliance epic (hud-2wbco).
