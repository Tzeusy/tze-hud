# Text Stream Portal — UX Refinement Handover

Date: 2026-04-19 (update; originally opened 2026-04-18)
Status: **in progress — not complete.**

## Current state (2026-04-19 pm)

The previous revision of this doc claimed the HitRegion render fix had
landed; on inspection it had never been committed. Operator feedback on the
live HUD confirmed the symptom ("background is still pale blue") — the
default HitRegion branch in `renderer.rs` was still painting opaque
`[0.2, 0.3, 0.5, 1.0]` over the pane backgrounds.

This update actually lands the fix, updates pixel-readback tests 14 + 17 to
match the new behaviour, and drops pane opacity from 0.95 → 0.90 per the
operator's latest preference.

**Interactivity status** — `hud-dih4` (pointer capture on content-layer
tiles, PR #490) and `hud-6bbe` (wheel + PgUp/PgDn wiring, PR #497) are both
**closed and merged on main**.  If the live HUD still shows no click / drag
/ scroll, the deployed Windows binary is almost certainly older than those
PRs — redeploy from a fresh `cargo build --target x86_64-pc-windows-gnu
--release` (or the Butler flow) before re-testing. Composer *text input*
(typing into the input box) is a separate unplumbed path — the composer is
placeholder-only today; there is no HitRegion→keyboard→text-buffer pipe in
v1 for agent-authored tiles. That needs its own bead; see "What's NOT
complete".

### What now lives in the repo

Two exemplar scripts under `.claude/skills/user-test/scripts/`:

- **`text_stream_portal_exemplar.py`** — the two-pane UX demo.
  860×680 content-layer tile at the right edge, rounded 14px, z=220. Header
  (52px) + footer (30px) chrome strips, INPUT pane on the left with eyebrow
  label, composer box (white 0.05 inset + 1px border, green caret stub),
  placeholder text, submit-hint strip (`Enter submit · Shift+Enter newline ·
  Esc cancel`). OUTPUT pane on the right with TRANSCRIPT eyebrow and the
  markdown body. **Equal 50/50 split** between panes separated by a **fat
  6px drag divider** with a centred 2×44px grip bar and an 8px-wide
  `portal-pane-resize` HitRegion for future drag. Panes render at **90%
  black opacity** per the operator's 2026-04-19 pm preference (iterated
  80% → 90% → 95% → back to 90%). All chrome tokens are still at the top
  of the file; iterate there.

- **`text_stream_scroll_exemplar.py`** — hud-w5ih Transcript Interaction
  Contract test. Four phases: (1) mount long transcript + RegisterTileScroll,
  (2) four SetScrollOffset steps, (3) five appends mid-scroll with offset
  preserved, (4) return-to-tail.

Both scripts use `HudClient` from `hud_grpc_client.py` and follow the
split-batch `set_tile_root` → `add_node` pattern (see Gotchas).

### Engine work that landed this session

1. **HitRegion rendering fix** (`crates/tze_hud_compositor/src/renderer.rs`)
   — actually committed this time.
   - `tile_background_color` returns `None` when the tile root is a
     HitRegion, so the whole tile renders nothing.
   - `render_node` emits no rect in the default state; hover uses
     `local_style.hover_tint` if set, else `(1, 1, 1, 0.1)`; pressed uses
     `local_style.pressed_tint` if set, else `(0, 0, 0, 0.15)`. Matches RFC
     0004 §6.5 (+0.1 white overlay on hover, ×0.85 darken on press) within
     the sibling-rect model.
   - Unblocks the "pale blue boxes" symptom; pane backgrounds now show
     through the four HitRegion children (composer_focus, submit_hit,
     pane_resize_hit, scroll_hit) in their default state.
2. **Scroll mutations in the session protocol**. `RegisterTileScrollMutation`
   + `SetScrollOffsetMutation` added to `types.proto` (tags 11, 12);
   `SceneMutation::RegisterTileScroll` + `SetScrollOffset` variants added in
   `tze_hud_scene/src/mutation.rs` (namespace-checked); both paths
   translated in `session_server.rs`; Python stubs regenerated. Renderer /
   coalescer plumbing was already shipped by hud-w5ih (PR #489); this is the
   agent-facing exposure that was missing.
3. **Windows build regression fix** (`element_store.rs`). Removed stray
   `.ok()` before `map_err` that broke the Windows cross-compile target.
4. **Bead `hud-r52v`** (TextMarkdownNode inline color runs) filed + specs /
   RFCs cross-referenced. Appears to have shipped in the intervening day —
   recent commits (`hud-vwggh`, `hud-qu8k4`, `hud-9pmd`, `hud-rxfc`) all
   touch `color_run_spans`. Verify separately before building on it.

### Pixel-readback tests updated (2026-04-19 pm)

`crates/tze_hud_runtime/tests/pixel_readback.rs`:
- `test_color_14_overlay_passthrough_regions_near_background` — expected
  pixel now `[58, 58, 82, 255]` (clear bg linear-blended with the
  `[0, 0, 0, 0.15]` passthrough overlay, since the HitRegion content tile
  no longer paints anything). Tolerance dropped back to `CI_BLEND_TOLERANCE`
  (no more llvmpipe quirk to cover).
- `test_color_17_chatty_dashboard_touch_transparent_tiles` — expected pixel
  now `BG_SRGB = [64, 64, 89, 255]` (50 HitRegion tiles are invisible), at
  `CI_SOLID_TOLERANCE`.

Both verified locally with `cargo test -p tze_hud_runtime --test
pixel_readback --features "headless dev-mode" -- test_color_14 test_color_17`
→ 2 passed.

## What's NOT complete

- **Windows HUD redeploy** — operator still sees "no click / drag / scroll"
  as of 2026-04-19 pm. `hud-dih4` (PR #490, content-tile pointer capture)
  and `hud-6bbe` (PR #497, wheel + PgUp/PgDn) are both merged on main.
  Most likely the `TzeHudOverlay` scheduled task on Windows is still running
  a binary from before those PRs. Rebuild + redeploy before re-testing any
  of the interaction items below.
- **Drag-to-resize the pane divider**: the visual affordance +
  `portal-pane-resize` HitRegion are there. The divider resize logic itself
  is not wired — moving the handle still needs code that consumes the
  captured drag and adjusts `INPUT_PANE_W` at runtime. File a bead when the
  engine redeploy confirms pointer capture is landing.
- **Composer typing / submit**: the composer is placeholder-only.  No text
  input path exists for agent-authored content-layer tiles in v1 — key
  events currently reach the runtime but there is no plumbing from a
  focused HitRegion to a mutable text buffer that can be re-rendered via
  MutationBatch. This is a new piece of work, not blocked on any existing
  bead; needs a dedicated epic (propose: "composer-input pipeline —
  HitRegion.focused → CommandInputEvent → agent mutation batch").
- **Wheel / keyboard scroll exercised through the exemplar**: the engine
  path is in place (PR #497). What's missing is an exemplar phase that
  actually drives the wheel / PgUp / PgDn to prove the path; today the
  scroll exemplar drives `SetScrollOffset` via the adapter.
- **`docs/exemplar-manual-review-checklist.md` row 11** — still needs the
  final UX-tweak sign-off row entered once a human confirms the two-pane
  render on the live HUD.
- **`.claude/skills/user-test/SKILL.md`** — the "Text Stream Portals
  Exemplar Scenario" section still reads "pending exemplar script" even
  though both scripts now exist. Needs: CLI, phase table, and Human
  Acceptance Criteria (matching the shape used for presence-card /
  subtitle / status-bar exemplars).
- **User-visible confirmation of the latest render**. The last on-screen
  check by the operator happened just before the mid-session file
  replacement; after restoring the two-pane design the script did publish
  successfully, but the operator hasn't reconfirmed that it matches their
  mental model of the design.

## Renderer fix detail (resolved)

Root cause of the "light blue boxes": `NodeData::HitRegion` rendered an
opaque `[0.2, 0.3, 0.5, 1.0]` rectangle in its default state. The three hit
regions in the exemplar (`composer_focus`, `scroll_hit`, `submit_hit`)
landed exactly under the three boxes the operator reported ("type a
reply…", "Exemplar Manual Review Checklist", "Enter submit…"). Opaque blue
sat on top of the pane backgrounds regardless of pane alpha.

Fix in `crates/tze_hud_compositor/src/renderer.rs`:
- `tile_background_color` returns `None` for HitRegion root nodes.
- `render_node` HitRegion branch only emits a rect on hover/pressed state,
  using the spec-compliant tints.

Secondary note, not the root cause: `from_text_markdown_node` at
`crates/tze_hud_compositor/src/text.rs:360` never reads `node.background` —
only the `render_node` `NodeData::TextMarkdown` branch does (renderer.rs
~6107). The exemplar's `background: [0, 0, 0, 0]` on text nodes does the
right thing (no background rect emitted).

## Decisions that landed via operator feedback

1. **Layout**: `[INPUT left | OUTPUT right]` 50/50 split with a fat 6px drag
   divider between them. No bottom-chat-style input.
2. **Icons**: ASCII only — unicode `↵ ⇧ esc` rendered as `[]` boxes in the
   font. Current hint: `Enter submit · Shift+Enter newline · Esc cancel`.
3. **Backdrop opacity**: iterated 80% → 90% → 95%. Currently 95%. User has
   not asked for further adjustment.
4. **Pane tinting**: earlier indigo/sage tints were discarded — operator
   wanted pure black panes once the blue-tint bug was fixed.
5. **No drag-to-reposition** for content-layer tiles today (same as
   `hud-bs2q` drag-handle epic scoping).

## Current portal chrome tokens (all tweakable at the top of the script)

- `PORTAL_W=860, PORTAL_H=680, PORTAL_RADIUS=14`
- Position: right-edge, `x = tab_width - PORTAL_W - 28`, `y = 120`,
  `z_order = 220` (bump if orphan z-conflict)
- Root backdrop: `(0, 0, 0, 0.30)` — light portal frame only
- Header / footer strips: `(0, 0, 0, 0.50)`
- Pane backgrounds (INPUT + OUTPUT): `(0, 0, 0, 0.90)`
- Composer inset (inside input pane): white `(1, 1, 1, 0.05)` + 1px white
  border quads, green caret
- Pane divider: `PANE_DIVIDER_W=6.0`, `INPUT_PANE_W = (PORTAL_W -
  PANE_DIVIDER_W) / 2.0 = 427.0`, divider fill `(1,1,1,0.14)`, grip
  `(1,1,1,0.40)` at 2×44px
- Typography: title 17, subtitle 11, body 13, meta/hint 10–11, eyebrow 10

## How to resume

1. Read this file.
2. Read `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py`
   (~675 lines) and `text_stream_scroll_exemplar.py` (~349 lines).
3. If iterating the visual UX, tweak the constants at the top of
   `text_stream_portal_exemplar.py` and re-run (repro commands below). No
   rebuild needed — only the Python client changes.
4. If iterating the scroll contract, the `scroll_exemplar` covers the four
   Transcript Interaction phases.
5. Remaining housekeeping: update `docs/exemplar-manual-review-checklist.md`
   row 11 and `.claude/skills/user-test/SKILL.md` Text Stream Portals
   section once you're happy with the render.

## Repro commands

```bash
# Kill + relaunch HUD (clears orphan tiles at same z-order)
ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes tzeus@tzehouse-windows.parrot-hen.ts.net \
  "taskkill /F /IM tze_hud.exe"
ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes tzeus@tzehouse-windows.parrot-hen.ts.net \
  "schtasks /Run /TN TzeHudOverlay"
# Wait until MCP HTTP responds, then render.
set -a && source /home/tze/gt/tze_hud/mayor/rig/.env && set +a

# Two-pane UX render
python3 /home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc /home/tze/gt/tze_hud/mayor/rig/docs/exemplar-manual-review-checklist.md \
  --tab-width 1920 \
  --phases baseline \
  --baseline-hold-s 30 \
  --max-lines 80

# hud-w5ih scroll contract test
python3 /home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/scripts/text_stream_scroll_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha
```

## Gotchas discovered the hard way

- Default PSK `tze-hud-key` is rejected in strict mode; the `TzeHudOverlay`
  task embeds a 64-char hex PSK in its `--psk` flag. `.env` must match.
- `agent-id` must match a `[agents.registered.*]` entry in the Windows toml
  — unregistered agents get `caps=[]` and `request_lease` fails with
  `PERMISSION_DENIED`.
- `set_tile_root` in a single batch with `add_node` children fails — the
  server rewrites the root id, so `parent_id` ends up pointing to a
  nonexistent node and the atomic-batch rejection drops the `set_tile_root`
  too. Split into a dedicated `submit_mutation_batch`, read
  `MutationResult.created_ids[0]`, and use it as `parent_id` for subsequent
  `add_node` calls.
- The `ServerMessage` oneof field is `mutation_result`, not `batch_result`;
  the `MutationResult` field is `accepted`, not `applied`. Reading the wrong
  names silently yields default-False and hides real rejection reasons.
- HUD restart is required between exemplar runs to clear orphan tiles at
  the same z-order.
- `presence_card_exemplar.py` fails with `upload_resource` capability not
  granted on `agent-alpha`. Not relevant for portal testing.
- `MoveFileExW` in `element_store.rs` returns `windows::core::Result<()>`
  directly in the 0.58 crate; `.ok()` turns Result into Option and breaks
  the subsequent `.map_err`. Drop the `.ok()`. Has regressed more than once
  this session — watch for it on the Windows cross-compile.

## Related beads

- `hud-r52v` — TextMarkdownNode inline color runs. Filed this epic, docs
  cross-referenced. Recent commits (`hud-vwggh`, `hud-qu8k4`, `hud-9pmd`,
  `hud-rxfc`) all look like the implementation landed; verify before
  building atop.
- `hud-dih4` — **CLOSED** (PR #490). Pointer capture on content-layer tiles
  is live on main. If the live HUD still isn't responding to clicks, the
  deployed Windows binary is stale.
- `hud-w5ih` — **CLOSED** (PR #489). Renderer/coalescer plumbing for
  scroll-offset. Session-protocol mutation exposure landed in the 2026-04-19
  am session (see "Engine work" above).
- `hud-6bbe` — **CLOSED** (PR #497). Wheel + PgUp/PgDn wired through
  `process_keyboard_scroll`.
- Needed new bead — composer text input pipeline (no existing bead tracks
  this).
