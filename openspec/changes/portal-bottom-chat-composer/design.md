# Design: portal-bottom-chat-composer

## Context

Owner live-testing direction (2026-07-03, tzehouse-windows rounds 1–2) reverses the earlier "No bottom-chat-style input" decision: the portal should read as a chat — multi-line wrapping composer pinned at the bottom, Enter to send, Ctrl+Enter for newline, and submitted text bubbling into a separator-delimited history instead of vanishing. The reversal was pre-flagged in AGENTS.md as needing this scoped OpenSpec change before implementation. Current state: composer is single-line with horizontal caret-follow (#983) confined to a bottom input strip (#987); Viewer Reply Echo exists only on the projection-authority path, which the raw-tile exemplar pilot does not use.

## Goals / Non-Goals

**Goals**: chat-shaped composer semantics (wrap, bounded growth, submit keys); viewer history visible on every portal path; minimal token-styled turn separation; all local-first.
**Non-Goals**: rich text, IME (v1-reserved), message edit/delete, threading, full turn attribution/multi-node turn model (promotion, hud-g1ena), font behavior on resize (separate delta on portal-whole-unit-resize, hud-ovjxu).

## Decisions

1. **ADDED requirements, not MODIFIED.** §Local-First Composer Draft Editing (pending in portal-composer-interaction-completeness, unsync'd) stays intact; its horizontal caret-follow becomes the defined single-line-profile behavior via this change's Multi-Line requirement. Avoids cross-change MODIFIED coupling between two unsync'd deltas.
2. **Shift+Enter aliases Ctrl+Enter** for newline — matches every mainstream chat app; costs nothing.
3. **Pilot echo: prefer routing the pilot through the projection-authority echo** over duplicating echo logic in the raw-tile path; but the requirement is stated behaviorally so either implementation satisfies it (the exemplar migrating onto `portal_projection_*` is the strategic direction anyway — hud-rpm9s).
4. **Separators are the minimal pilot-visible slice** of the promotion turn model: a token-styled divider only, no attribution chips/alignment — those stay in hud-g1ena scope to avoid double-building.
5. **Composer growth is viewer-local layout**, not adapter-visible geometry: the composer/transcript split moves inside the portal; the portal's outer geometry is untouched, so no interaction with the viewer-geometry lock (#986) or group resize (#984/#989).

## Risks / Trade-offs

- [Composer growth vs bounded transcript window] Growth steals transcript rows; a full-height composer could hide the last agent turn. → Mitigation: token-bounded max lines (default small, e.g. 6) then internal vertical scroll.
- [Enter-to-send muscle-memory conflict with terminal-style adapters] Semantic-inbox submission is already the contract (no raw keystroke passthrough), so Enter-submit is safe; newline reaches the adapter inside the submitted text.
- [Echo duplication if adapter also echoes] Adapters that echo submitted input back as output produce doubles. → Mitigation: pilot echo is runtime-authored and kind-distinct; adapter guidance (hud-projection skill) already tells sessions not to re-publish operator input verbatim.

## Migration Plan

Spec-only change; validate `--strict`, land on main, file implementation beads under hud-nx7yq (composer wrap/growth; submit-key routing; pilot echo; separators + tokens), implement, then sync+archive with the composer-interaction change per hud-hpuzp convention. Rollback = archive without sync; the superseded-decision note in `docs/reports/text-stream-refinement.md` should be annotated when implementation lands, not before.

## Open Questions

- Exact max-line token default (proposal: 6) and whether the bound is lines or pixels — decide with the visual-token compliance epic (hud-2wbco).
- Whether pilot echo entries need a distinct visual accent pre-promotion or separator-only is enough until hud-g1ena lands.
