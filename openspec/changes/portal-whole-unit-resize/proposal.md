## Why

Live verification on the `tzehouse-windows` reference hardware (2026-06-27),
once the OS keyboard-focus root cause was fixed so keystrokes finally reach the
overlay, surfaced a resize-behavior gap the spec does not currently constrain:

- A text-stream portal is composed of **multiple constituent surfaces** (frame,
  transcript pane, composer/input pane, drag affordances). The runtime has **no
  notion of a portal as a group** — `apply_portal_resize_hotkey` /
  `apply_portal_resize_pointer_event` resolve a single focused tile and mutate
  only its bounds.
- Focusing the composer focuses the *input-pane* surface, so `Ctrl+=` grows only
  that pane (center-anchored) and it detaches from the rest. Operator report:
  *"the input window floats to the top-left … I expect the whole text-stream
  portal to grow."*

"Portal Window Management" already mandates focus-scoped resize and local-first
geometry, but it says "the portal" without stating that a portal is a **coherent
unit** that moves and resizes as a whole even when composed of several surfaces.
That omission is why a conformant-looking implementation can resize a single
constituent surface and still appear to satisfy the spec.

## What Changes

One MODIFIED requirement on `text-stream-portals` (spec ahead of code —
implementation is tracked by `hud-fb3en` and owned by the portal-component
promotion epic `hud-g1ena`):

- **Portal Window Management** — add that move and resize SHALL operate on the
  portal as a single coherent unit: all constituent surfaces move and scale
  together preserving relative layout, and focusing a constituent surface (e.g.
  the composer) resizes the whole portal, never an individual surface in
  isolation. Adds a scenario.

## What Does Not Change

- No change to the focus-scoping rules, the resize step/clamp/lease-budget
  contract, the release-only key-up fallback, local-first geometry, or the
  adapter-cannot-override-gesture rule — only the unit of resize is clarified.
- No new transport or RPC. How the runtime associates constituent surfaces into
  a portal group (reuse the existing `sync_group` member-set vs. a dedicated
  group id) is an implementation choice for `hud-fb3en`, not a spec mandate.
- Tab/Shift+Tab affordance focusability (`hud-02sp5`) is a compliance fix under
  the existing "Transcript Interaction Contract" requirement (focusable
  affordances reachable without a pointer); it needs no delta here.

## Non-Goals

- Per-surface independent resize (explicitly disallowed by this change).
- Free-form aspect-ratio editing of individual panes; pane proportions are
  preserved under a uniform portal scale.
