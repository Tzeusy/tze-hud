## Why

The `text-stream-portals` "Portal Window Management" requirement
(`openspec/specs/text-stream-portals/spec.md`) specifies focus-scoped Ctrl+`+`/Ctrl+`-`
resize shortcuts, but is silent on the OS-level key-delivery edge case that live Windows
exhibits: `SendInput` delivers a Ctrl-modified `=`/`-` chord as a key **release** with no
preceding matching key **press** while Ctrl is held. Under the press-only intercept the
shortcut never fired on real hardware — the P1 live defect `hud-v4k1h`.

PR #967 (`hud-vznqm`) shipped the fix — a key-up fallback (`portal_resize_key_code` +
`dispatch_key_up_event_inner` in `crates/tze_hud_runtime/src/windowed/keyboard.rs`) plus a
`consumed_portal_resize_keydowns` dedup set (`crates/tze_hud_runtime/src/windowed/mod.rs`)
so a normal physical down/up pair still resizes exactly once — but no normative requirement
covers it. This is **code ahead of spec**, surfaced by the 2026-06-21 portal spec-vs-code
reconciliation sweep (bead `hud-y3pho`). This change adds the missing requirement so the
behavior is captured and testable.

## What Changes

One MODIFIED requirement on `text-stream-portals`:

- **Portal Window Management** — add that resize-shortcut handling SHALL be robust to
  release-only key delivery (apply the resize step on the key release as a fallback when the
  host delivers the chord as a release with no matching press, as Windows `SendInput` does),
  and SHALL deduplicate consumed press/release pairs so a normal physical key-down/key-up
  cycle resizes exactly once. Adds a scenario covering the release-only key stream. All
  existing clauses (focus-scoping, clamping, local-first feedback, pointer resize, adapter
  non-override, scroll indicator) are unchanged.

## What Does Not Change

- No new transport, RPC, or input plane: this is the existing keyboard input path.
- No change to focus-scoping, clamp bounds, local-first geometry feedback, pointer-driven
  resize, the adapter-cannot-override-gesture rule, or the scroll-position indicator.
- This documents already-shipped behavior; no runtime code change accompanies it.

## Non-Goals

- Live reference-hardware re-verification of the resize fix — that remains `hud-v4k1h`,
  blocked on Windows credentials, and is not a spec concern.
