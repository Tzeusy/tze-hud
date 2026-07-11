# Proposal: portal-chat-grade-affordances

## Why

The text stream portal already carries a two-way conversation (agent turns, viewer reply echo, unread and delivery state plumbed end-to-end into `ProjectedPortalState`), but the surface is missing the ambient conversational cues every mature chat application provides: the viewer cannot see whether their reply was delivered, where the unread boundary is, when a turn arrived, whether the agent is still writing, or how to get back to the live tail after scrolling away — and a portal that has never received content greets the viewer with the literal placeholder `<empty projection stream>`. These gaps make the portal feel like a debug console instead of a presence surface, and none of them are covered by requirements in `openspec/specs/text-stream-portals/spec.md`, so implementation cannot proceed spec-first (epic hud-g0c9g: no spec = no plan).

## What Changes

Spec-only change (this change ships requirements and scenarios; rendering work folds into the promotion epic hud-g1ena alongside the multi-node turn model):

- Add **delivery acknowledgement** on the viewer's echoed turn: render the already-tracked `InputDeliveryState` (Pending → Delivered) as an ambient per-turn cue; builds on §Viewer Reply Echo.
- Add **unread divider + ambient unread count**: `unread_output_count` is plumbed but never rendered; present it as an in-transcript divider and a compact ambient count that never self-escalates attention (§Ambient Portal Attention Defaults).
- Add **jump-to-latest / resume-follow-tail affordance**: when the viewer scrolls away from the tail, offer a local-first affordance to return and resume tail-follow, without the adapter being able to force the jump (§Transcript Interaction Contract).
- Add **per-turn timestamps** sourced from `appended_at_wall_us` (typed wall-clock domain), rendered ambiently and token-styled.
- Add **agent activity / streaming cue**: an ambient typing-style indicator and streaming cursor while the agent is actively appending, strictly subordinate to the attention model.
- Add **first-run / empty-portal treatment**: a friendly, token-styled empty state replacing the literal `<empty projection stream>` placeholder.
- Add **connecting-vs-disconnected distinction**: an attached-but-never-connected portal presents a distinct "connecting" treatment rather than reusing the degraded/disconnected treatment.
- Modify **§Viewer Reply Echo** to specify the two-pane INPUT/OUTPUT split (owner live round-6, 2026-07-04, hud-egf39 / PR #1038): the portal tracks two separately-bounded histories — an INPUT history of the viewer's own submissions (rendered beneath a top-anchored composer, stacked with turn dividers via the viewer-echo stack) and an OUTPUT transcript of agent-authored content only. Viewer submissions echo into the INPUT history and MUST NOT be appended to the OUTPUT/agent transcript stream. This reframes the prior single-stream "echo into the retained transcript" wording; all other echo constraints (runtime-authored, kind-distinct, never unread, no attention escalation, redaction parity, transactional delivery, rejected-not-echoed) are preserved.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `text-stream-portals`: adds the seven requirements above as new requirement sections and modifies one existing requirement (§Viewer Reply Echo) to reframe the single-stream echo model as the two-pane INPUT/OUTPUT split. The new requirements anchor to and must stay consistent with §Viewer Reply Echo (as modified), §Ambient Portal Attention Defaults, §Transcript Interaction Contract, §Coherent Transcript Coalescing, and §Governance, Privacy, and Override Compliance.

## Impact

- **Spec**: `openspec/specs/text-stream-portals/spec.md` gains ~7 requirement sections and one modified requirement (§Viewer Reply Echo, two-pane split) via this change's delta.
- **Code (deferred to promotion epic hud-g1ena / hud-s4lrw)**: transcript presentation in the compositor portal render path, `ProjectedPortalState` → render-batch projection (`crates/tze_hud_runtime/src/portal_projection_driver.rs`, `crates/tze_hud_projection/src/authority.rs` state already exists), portal part tokens for the new cues.
- **Beads**: epic hud-g0c9g (this is its STEP 1); rendering beads to be filed under promotion after this change lands.
- **Non-goals**: no new adapter/MCP surface, no notification behavior, no read-receipts back to the adapter (viewer "seen" state is not disclosed to the agent in this change).
