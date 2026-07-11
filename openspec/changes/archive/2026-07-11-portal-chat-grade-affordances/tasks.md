# Tasks: portal-chat-grade-affordances

This change is SPEC-FIRST (epic hud-g0c9g STEP 1). Sections 1–2 are this change's deliverable; section 3 records the implementation split that folds into the promotion epic hud-g1ena and is NOT required to archive this change once the beads exist.

## 1. Spec delta

- [x] 1.1 Author delta spec `specs/text-stream-portals/spec.md` with the seven ADDED requirements (delivery ack, unread divider+count, jump-to-latest, timestamps, activity/streaming cue, empty-state, connecting distinction)
- [x] 1.2 `openspec validate portal-chat-grade-affordances --strict` passes
- [x] 1.3 Cross-check delta consistency against §Viewer Reply Echo, §Ambient Portal Attention Defaults, §Transcript Interaction Contract, §Bounded Transcript Viewport, §Governance/Privacy (no contradictions introduced)
- [x] 1.4 Add MODIFIED §Viewer Reply Echo reframing single-stream echo as the two-pane INPUT/OUTPUT split (owner round-6, hud-egf39/#1038); INPUT history under a top-anchored composer with turn dividers, OUTPUT transcript agent-authored only, viewer submissions never appended to OUTPUT (hud-til02)

## 2. Land + sync

- [x] 2.1 Commit + push the change directory to main (30bdfc62)
- [x] 2.2 `/opsx:sync` the delta into `openspec/specs/text-stream-portals/spec.md` — all 7 rendering children (hud-g1ena.1..7) merged, all 8 requirements (7 ADDED + 1 MODIFIED) re-verified against current main and synced verbatim (with a minimal A2 wording note on drain-cadence unread clearing, hud-0j1ju), and this change archived [hud-djnrv]
- [x] 2.3 Update epic hud-g0c9g: STEP 1 done, note the change name and requirement list

## 3. Implementation handoff (promotion epic hud-g1ena — file beads, do not implement here)

- [x] 3.1 File rendering bead: delivery-ack cue on viewer echo turns → `hud-g1ena.1` (blocked-by hud-s4lrw multi-node turn model)
- [x] 3.2 File rendering bead: unread divider + ambient count + clear-on-tail-view → `hud-g1ena.2`
- [x] 3.3 File rendering bead: jump-to-latest affordance (compose with unread count) → `hud-g1ena.3`
- [x] 3.4 File rendering bead: per-turn timestamps from appended_at_wall_us → `hud-g1ena.4`
- [x] 3.5 File rendering bead: streaming cursor + header activity cue (derive from observed appends; quiesce; redaction suppression) → `hud-g1ena.5`
- [x] 3.6 File rendering bead: token-styled empty/first-run state replacing `<empty projection stream>` → `hud-g1ena.6`
- [x] 3.7 File rendering bead: connecting-vs-degraded treatment (`has_ever_connected` contract decision recorded in design.md open questions) → `hud-g1ena.7`
- [x] 3.8 Wire all new beads as children of hud-g1ena; mark hud-g0c9g STEP 2 delegated
