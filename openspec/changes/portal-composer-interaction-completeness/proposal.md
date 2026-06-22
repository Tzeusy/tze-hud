## Why

The text-stream-portal composer is the "typing into the window" half of the portal's
Telegram-grade interaction goal. A 2026-06-21 audit found two interaction-completeness gaps
where the shipped composer falls short of what a portal needs to be fully usable — and the
current `text-stream-portals` spec is silent on both, so neither is testable:

1. **No pointer-independent focus path** (`hud-v0cal`). The composer can only be focused via
   a pointer-down hit-test (`crates/tze_hud_runtime/src/windowed/lifecycle.rs`); no keyboard
   focus-traversal is wired into the windowed key path. "Transcript Interaction Contract"
   requires "focusable interaction affordances" and reuse of "runtime-owned focus rules" but
   never states that focus must be reachable **without** a pointer. That is a hard blocker on
   the Mobile Presence Node profile (smart glasses / no-pointer surfaces) — the portal there
   is unusable for input, violating the "one scene model, two profiles" core rule.

2. **No horizontal caret-follow** (`hud-zlfi4`). The single-line composer renders at a fixed
   x with no scroll offset tracking the caret (`crates/tze_hud_runtime/src/compositor/.../tile_render.rs`),
   so the caret vanishes once the draft exceeds the box width. "Local-First Composer Draft
   Editing" mandates local caret rendering, and "Portal Window Management" forbids partially
   clipped glyphs, but neither requires the caret to **stay visible** while editing past the
   visible width — which any usable text field must do.

A third audit finding, pointer caret-placement/range-selection (`hud-hxhnt`), is **already
covered**: "Local-First Composer Draft Editing" already mandates "selection (keyboard and
pointer)", so that is a code-behind-spec compliance fix, not a spec gap — it needs no delta.

## What Changes

Two MODIFIED requirements on `text-stream-portals` (spec ahead of code — implementation
follows via `hud-v0cal` and `hud-zlfi4`):

- **Transcript Interaction Contract** — add that focusable interaction affordances SHALL be
  reachable without a pointer via a runtime-owned keyboard focus-traversal path, so a portal
  is fully operable on pointer-less surfaces. Adds a scenario.
- **Local-First Composer Draft Editing** — add that when draft text exceeds the visible
  composer width, the runtime SHALL maintain a horizontal scroll offset keeping the active
  caret (and the moving edge of an active selection) within the visible composer region.
  Adds a scenario.

## What Does Not Change

- No new transport, RPC, or input plane; this is the existing windowed keyboard / composer
  render path.
- No change to focus *scoping* rules, latency budgets, draft bounds/paste caps, IME status
  (still v1-reserved), or the adapter-cannot-override-draft rule.
- Pointer caret-placement and pointer-drag range-selection (`hud-hxhnt`) are already mandated
  by the existing requirement; their implementation is tracked separately and needs no delta.

## Non-Goals

- Multi-line composer editing, undo/redo, rich text, multi-caret — all remain excluded.
- IME composition and positioning — remains v1-reserved under the input-model spec.
- The renderer/keyboard implementation itself: it lands through `hud-v0cal` / `hud-zlfi4`,
  coordinated with the promotion epic (`hud-g1ena`) which owns the composer render path.
