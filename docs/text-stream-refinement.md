# Text Stream Portal — UX Refinement Handover

Date: 2026-04-29 (update; originally opened 2026-04-18)
Status: **live interactive refinement landed; caret/Space polish signed off; broader operator polish still active.**

## Current state (2026-04-29)

The text stream portal exemplar is no longer a static two-pane render. It now
exercises the Phase-0 portal as a live, content-layer interaction surface over
the existing primary `HudSession` stream:

- draggable header repositioning for the portal surface,
- independent transparent capture tiles for composer and transcript panes,
- local-first transcript scrolling on the OUTPUT pane,
- click-to-focus composer input with placeholder/caret rendering,
- bounded text editing, submit/clear behavior, Home/End, arrow movement,
  visual-line Up/Down, word navigation, word delete, and Ctrl+V paste,
- cleanup of stale portal tiles when the user-test session exits normally.

The refinement is still intentionally Phase 0: no portal-specific proto RPCs,
no terminal emulator, no PTY semantics, and no runtime-owned external process.
The exemplar remains a resident raw-tile composition driven by an authenticated
agent session.

**Latest sign-off:** `hud-0ojis` closed the focused composer caret and Space
input polish with deterministic tests plus the portal `--self-test` path. The
sign-off report is
`docs/reports/hud-0ojis-text-stream-portal-caret-space-signoff-20260429.md`.

### What now lives in the repo

One exemplar script under `.claude/skills/user-test/scripts/`:

- **`text_stream_portal_exemplar.py`** — the two-pane live UX demo.
  860×680 content-layer portal surface at the right edge. The static frame
  tile sits at z=220, while transparent capture tiles sit above the header,
  composer, and transcript body so drag, keyboard focus, and mouse wheel input
  route to the intended surface only.
  Header (52px) + footer (30px) chrome strips, INPUT pane on the left with eyebrow
  label, composer text window (black 0.95 backing + 1px border, green caret stub),
  placeholder text, submit-hint strip (`Enter submit · Shift+Enter newline ·
  Esc cancel`). OUTPUT pane on the right with TRANSCRIPT eyebrow and the
  markdown body. **Equal 50/50 split** between panes separated by a **fat
  6px drag divider** with a centred 2×44px grip bar and an 8px-wide
  `portal-pane-resize` HitRegion for future drag. Panes render at **95%
  black opacity** per the operator's 2026-04-25 preference (iterated
  80% → 90% → 95%). All chrome tokens are still at the top
  of the file; iterate there. The former separate hud-w5ih scroll contract
  exemplar is now the `scroll` phase: mount long output + RegisterTileScroll
  on the transcript pane tile, four SetScrollOffset steps, five appends
  mid-scroll with offset preserved, then return-to-tail.

  The composer now uses stable runtime node IDs and mutation updates for the
  text/caret nodes rather than rebuilding the whole input tile for every
  character. Normal printable input is sourced from runtime character events;
  the only key-down fallback that inserts text is `Space`, because Windows
  winit does not send a separate `Key::Character(" ")` event for it in this
  path. Ctrl+V is handled first by runtime clipboard forwarding and then by an
  SSH clipboard fallback if the runtime paste event does not arrive.

  The exemplar also has a `diagnostic-input` phase for live compositor-path
  validation when manual console input is unavailable. It mounts the normal
  portal surface and then uses Windows OS input over SSH (`SetCursorPos`,
  mouse down/move/up, wheel, and `SendInput` Unicode text) against the overlay.
  Expected transcript evidence includes `input:focus-gained`,
  `drag:start`/`drag:end`, and `scroll:output`; this is intentionally not a
  direct `EventBatch` or transcript-only shortcut.

The script uses `HudClient` from `hud_grpc_client.py` and follows the
split-batch `set_tile_root` → `add_node` pattern (see Gotchas).

### Engine work that landed across the refinement

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
5. **Windowed runtime input refinements** (`crates/tze_hud_runtime/src/windowed.rs`):
   keyboard dispatch now uses blocking scene locks rather than opportunistic
   `try_lock()` for active tab/session lookup; Windows pointer capture/release
   is used for drag continuity; Ctrl+V reads `CF_UNICODETEXT` from the Windows
   clipboard and emits a character event; direct character forwarding is
   suppressed while Ctrl/Meta/Alt are held.
6. **Session event burst headroom** (`crates/tze_hud_protocol/src/session_server.rs`):
   `BROADCAST_CHANNEL_CAPACITY` increased from 32 to 1024 because runtime input
   events share the channel with server-push notices. Short typing/pointer bursts
   should not evict focus/input events while mutation responses are in flight.
7. **Literal composer text over proto** (`hud_grpc_client.py` +
   `crates/tze_hud_protocol/src/convert.rs`): the Python test client can emit
   `TextMarkdownNode.color_runs`, and a full-span color run matching the base
   color is interpreted as literal/monospace composer text. This keeps markdown
   markers visible and gives the caret model stable advances until the proto
   grows an explicit `font_family` field.

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

- **Drag-to-resize the pane divider**: the visual affordance +
  `portal-pane-resize` HitRegion are there. The divider resize logic itself
  is not wired — moving the handle still needs code that consumes the
  captured drag and adjusts `INPUT_PANE_W` at runtime. File a bead when the
  engine redeploy confirms pointer capture is landing.
- **Full operator input pass**: `hud-0ojis` signed off deterministic caret
  advance and Space fallback behavior. A future operator pass can still use the
  `composer-smoke` phase to visually confirm the latest render on the Windows
  HUD alongside the remaining drag/resize/minimize polish.
- **Drag persistence**: header drag repositions the live portal, but position
  persistence across sessions is not implemented.
- **`docs/exemplar-manual-review-checklist.md` row 11** — records the focused
  caret/Space sign-off, but still tracks the broader Text Stream Portal visual
  review as `live-refinement`.
- **`.claude/skills/user-test/SKILL.md`** — now documents the unified
  `text_stream_portal_exemplar.py` CLI and phase table. Keep it in sync as
  the portal phases evolve.
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
5. **Drag-to-reposition is in scope** for the text stream portal. The portal
   should gain a header/title-bar move affordance that updates tile bounds and
   preserves scroll/composer hit behaviour after moving. Tracked as `hud-9yfce`.

## Current portal chrome tokens (all tweakable at the top of the script)

- `PORTAL_W=860, PORTAL_H=680, PORTAL_RADIUS=14`
- Position: right-edge, `x = tab_width - PORTAL_W - 28`, `y = 120`,
  `z_order = 220` (bump if orphan z-conflict)
- Root backdrop: `(0, 0, 0, 0.30)` — light portal frame only
- Header / footer strips: `(0, 0, 0, 0.50)`
- Pane backgrounds (INPUT + OUTPUT): `(0, 0, 0, 0.95)`
- Scroll routing: the frame tile is non-scrollable; composer and transcript
  body are separate transparent capture tiles at `PORTAL_Z + 1`
- Text windows (composer + transcript body): black `(0, 0, 0, 0.95)`.
  Composer keeps 1px white border quads and green caret.
- Pane divider: `PANE_DIVIDER_W=6.0`, `INPUT_PANE_W = (PORTAL_W -
  PANE_DIVIDER_W) / 2.0 = 427.0`, divider fill `(1,1,1,0.14)`, grip
  `(1,1,1,0.40)` at 2×44px
- Typography: title 17, subtitle 11, body 13, meta/hint 10–11, eyebrow 10

## How to resume

1. Read this file.
2. Read `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py`.
3. If iterating the visual UX, tweak the constants at the top of
   `text_stream_portal_exemplar.py` and re-run (repro commands below). No
   rebuild needed — only the Python client changes.
4. If iterating the scroll contract, use the portal exemplar's `scroll`
   phase; it covers the four Transcript Interaction phases.
5. Remaining housekeeping: update `docs/exemplar-manual-review-checklist.md`
   row 11 and `.claude/skills/user-test/SKILL.md` Text Stream Portals
   section once you're happy with the render.

## Repro commands

```bash
# Optional: kill + relaunch HUD if a previous run used --leave-lease-on-exit
# or crashed before cleanup.
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
  --phases baseline,scroll \
  --baseline-hold-s 30 \
  --max-lines 80

# Automated compositor-path input pass
python3 /home/tze/gt/tze_hud/mayor/rig/.claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc /home/tze/gt/tze_hud/mayor/rig/docs/exemplar-manual-review-checklist.md \
  --tab-width 1920 \
  --phases diagnostic-input \
  --transcript-out test_results/text-stream-portal-diagnostic-input.json
```

## Gotchas discovered the hard way

- Default PSK `tze-hud-key` is rejected in strict mode; the `TzeHudOverlay`
  task embeds a 64-char hex PSK in its `--psk` flag. `.env` must match.
- The portal exemplar now releases its lease on exit by default, so ordinary
  Ctrl-C should remove the portal immediately. Use `--leave-lease-on-exit`
  only when intentionally testing the orphan/grace cleanup path.
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
- `hud-opkvq` — click-focus and keyboard input for the portal composer.
  Implementation has effectively landed in the user-test exemplar path; keep
  the bead open only if there is remaining acceptance bookkeeping or engine
  generalization to do.
- `hud-9yfce` — drag-to-move support for text stream portal tiles. Header
  drag is implemented in the exemplar via content-tile pointer capture and
  tile geometry mutation; persistence and final sign-off remain polish items.
- `hud-0ojis` — remaining caret x-position and Space/focus sign-off after
  the 2026-04-27 live input-path refinement.
