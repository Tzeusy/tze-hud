# Text Stream Portal — UX Refinement Handover

Date: 2026-04-18
Author: Claude (session transcript), ready for resumption in a fresh session.

## Prompt for the next session

> Resume iterating UX of the text-stream-portal exemplar against a live Windows
> HUD. Start by reading `docs/text-stream-refinement.md` for full state.
> The blocker to solve first is the "light blue text boxes" — the INPUT and
> OUTPUT pane surfaces still read as light blue on the live HUD even though
> the script writes `(0.0, 0.0, 0.0, 0.96)` to both `INPUT_PANE_BG_RGBA` and
> `OUTPUT_PANE_BG_RGBA` in
> `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py`. See the
> "Open blocker" section below for hypotheses to test. Do NOT reopen hud-dih4
> (click routing) or hud-w5ih (scroll binding) — those are correctly deferred.

## Where the work is

- Exemplar script: `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py`
  - Renders a two-pane portal (INPUT left, OUTPUT right) via the resident
    `HudSession` gRPC stream
  - Uses `agent-alpha` for capability grants (from
    `C:\tze_hud\tze_hud.toml` on Windows)
  - PSK is in `.env` (both `MCP_TEST_PSK` and `TZE_HUD_PSK` = 64-char hex
    matching the `TzeHudOverlay` scheduled task's `--psk` flag)
- Transcript artifacts: `test_results/text-stream-portal-split-v*.json`
- Check-in point: the Skill still needs its "Text Stream Portals Exemplar
  Scenario" section updated with final CLI + human-acceptance table after
  this iteration completes. It currently still says "pending exemplar
  script" even though the script exists.
- `docs/exemplar-manual-review-checklist.md` row 11 needs UX-tweak entries
  recorded once we're happy with the render.

## What's working

1. **Deploy + launch**: `TzeHudOverlay` scheduled task on
   `tzehouse-windows.parrot-hen.ts.net` runs `C:\tze_hud\tze_hud.exe
   --window-mode overlay`. Transparent overlay path is healthy.
2. **Authentication**: Use `agent-id=agent-alpha` (registered in the Windows
   toml with caps `create_tiles, modify_own_tiles, access_input_events`);
   `upload_resource` is NOT granted so avoid resource upload paths.
3. **gRPC mutation pattern**: `set_tile_root` must be submitted as a
   separate batch before `add_node` calls, so the caller can read
   `MutationResult.created_ids[0]` and use that server-assigned id as
   `parent_id` for subsequent children. Batching `set_tile_root +
   add_node` in one call fails with "node not found" even though
   `presence_card_exemplar.py` appears to do this — the server rewrites
   the root id. Fix lives in the `publish_portal` helper.
4. **z-order**: restart the HUD between runs (orphaned tiles hold
   `z_order=220` during grace period; subsequent runs get
   `z-order conflict`). Use `ssh tzeus@... "taskkill /F /IM tze_hud.exe"`
   then `schtasks /Run /TN TzeHudOverlay` + `sleep 6`.

## Open blocker (RESOLVED 2026-04-18)

Root cause: `NodeData::HitRegion` rendered an opaque
`[0.2, 0.3, 0.5, 1.0]` rectangle in its default (non-hover/non-press)
state. The three hit regions in the exemplar (`composer_focus`,
`scroll_hit`, `submit_hit`) land exactly under the three "light blue"
boxes the user reported: "type a reply…", "Exemplar Manual Review
Checklist", "Enter submit…". The opaque blue sat on top of the pane
backgrounds regardless of pane alpha.

Fix: `crates/tze_hud_compositor/src/renderer.rs`
- `tile_background_color` now returns `None` when the tile root is a
  HitRegion (it's a logical affordance, not a visual node).
- `render_node` NodeData::HitRegion only emits a rect when hovered or
  pressed, using the input-model spec's local-feedback tints
  (`spec.md:220`): hovered = `[1.0, 1.0, 1.0, 0.10]` (add 0.1 white),
  pressed = `[0.0, 0.0, 0.0, 0.15]` (multiply by 0.85 approximation).

Tests updated (`crates/tze_hud_runtime/tests/pixel_readback.rs`):
- `test_color_14_overlay_passthrough_regions_near_background` — HitRegion
  content tile is now invisible, so at (400,300) we see the clear
  background darkened by the overlay ≈ [58, 58, 82, 255].
- `test_color_17_chatty_dashboard_touch_transparent_tiles` — 50 HitRegion
  tiles are now invisible, so at (400,300) we see BG_SRGB = [64, 64, 89, 255].

Both tests pass locally.

Also worth noting but not relevant to the fix: `from_text_markdown_node`
at `crates/tze_hud_compositor/src/text.rs:360` never reads
`node.background`. It only affects the `text` glyph-rendering pass. The
background rect IS still drawn in `render_node`'s `NodeData::TextMarkdown`
branch (renderer.rs:6107), which reads `tm.background` correctly. So the
exemplar's `background: [0, 0, 0, 0]` on text nodes does the right thing
(no background rect emitted).

## What was shipped this session (not yet committed)

Modified files (uncommitted, in the main checkout):
- `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py` — new
  ~500-line exemplar script with split-layout portal, submit hints, scroll
  hit region, server-assigned-root-id pattern

Earlier commits in this session (already pushed to `origin/main`):
- `ad3f802` — initial SKILL.md section for text-stream-portals
- `4722126` — initial exemplar-manual-review-checklist row 11
- `79fa541` — correction: docs marked impl-complete (was wrong to call it
  "pre-implementation" — epic hud-t98e shipped the pilot)

Beads filed this session:
- **hud-dih4** P1 feature — "Enable pointer event capture on content-layer
  tiles for transparent overlay." Root cause for no clicks / no drag / no
  scroll-input. Affects presence-card, progress/gauge widgets too.
- **hud-w5ih** P2 feature — "Local-first scroll offset for resident raw-tile
  portals." Blocked-by `hud-dih4`. Wires scroll-offset binding into the
  renderer per the Transcript Interaction Contract.

## Decisions that landed via operator feedback

1. **Layout**: `[INPUT left | OUTPUT right]` split (operator preference).
   No bottom-chat-style input.
2. **Icons**: ASCII only — unicode `↵ ⇧ esc` rendered as `[]` boxes in
   the font. Current hint: `Enter submit · Shift+Enter newline · Esc cancel`.
3. **Backdrop**: "black with 80% opacity instead of gray frosted glass."
   Now at 96% due to bleed-through complaint — revisit per user intent.
4. **Distinct tints requested** earlier (indigo for input, sage for output);
   operator then said both looked light blue and asked for pure black.
   Currently pure black at 96%. User still sees light blue → something
   upstream tints text-node areas.
5. **No drag-to-reposition** for content-layer tiles today. The
   `hud-bs2q` epic (drag handles) was scoped to chrome layer via RFC 0004
   §3.0. Bringing drag to resident portals is out of scope; filed under
   the pointer-capture bead as a dependency.

## Current portal chrome (all tweakable at the top of the script)

- Size: `PORTAL_W=860, PORTAL_H=680`
- Position: right-edge, `x = tab_width - PORTAL_W - 28`, `y = 120`,
  `z_order = 220` (bump if orphan z-conflict)
- Root backdrop: `(0, 0, 0, 0.30)` — light portal frame only
- Header / footer strips: `(0, 0, 0, 0.50)` — mid-density chrome
- Pane backgrounds (both panes, currently under investigation): `(0, 0, 0, 0.96)`
- Composer inset (inside input pane): white `(1, 1, 1, 0.05)` + 1px white
  border quads, green caret
- Typography: title 17px, subtitle 11px, body 13px, meta/hint 10–11px,
  eyebrow 10px uppercase

## How to resume

1. Read this file.
2. Read `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py`
   (~500 lines).
3. Inspect the diagnostic targets in `renderer.rs` / `text.rs` named above.
4. Sanity check the blue-tint with the red-pane diagnostic
   (`INPUT_PANE_BG_RGBA = (0.5, 0, 0, 0.96)` + restart HUD + re-render).
5. Don't commit uncommitted changes yet — land the UX first, then commit
   the script + update `.claude/skills/user-test/SKILL.md` Text Stream
   Portals section with real CLI/phases/HAC and update
   `docs/exemplar-manual-review-checklist.md` row 11 with UX tweaks.

## Repro commands

```bash
# Kill + relaunch HUD (needed between runs to clear orphan tiles)
ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes tzeus@tzehouse-windows.parrot-hen.ts.net \
  "taskkill /F /IM tze_hud.exe"
sleep 3
ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes tzeus@tzehouse-windows.parrot-hen.ts.net \
  "schtasks /Run /TN TzeHudOverlay"
sleep 6

# Run baseline render
cd /home/tze/gt/tze_hud/mayor/rig
set -a && source .env && set +a
cd .claude/skills/user-test/scripts
python3 text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc /home/tze/gt/tze_hud/mayor/rig/docs/exemplar-manual-review-checklist.md \
  --tab-width 1920 \
  --phases baseline \
  --baseline-hold-s 60 \
  --max-lines 80 \
  --transcript-out /home/tze/gt/tze_hud/mayor/rig/test_results/text-stream-portal-split-next.json
```

## Gotchas discovered the hard way

- Default PSK `tze-hud-key` is rejected in strict mode; the
  `TzeHudOverlay` task embeds a 64-char hex PSK in its `--psk` flag.
  `.env` must match.
- `agent-id` must match a `[agents.registered.*]` entry in the Windows
  toml — unregistered agents get `caps=[]` and `request_lease` fails
  with `PERMISSION_DENIED`.
- `set_tile_root` in a single `apply_mutations` batch with
  `add_node` children fails with "node not found" — the server
  rewrites the root id. Split into a dedicated `submit_mutation_batch`
  call and read `MutationResult.created_ids[0]`.
- HUD restart is required between exemplar runs to clear orphan tiles
  at the same z-order. Orphan grace period is longer than operator
  iteration cycle.
- `presence_card_exemplar.py` fails with `upload_resource` capability
  not granted — it's wired to upload avatar PNGs, but agent-alpha
  doesn't have that cap. Not needed for portal testing.
