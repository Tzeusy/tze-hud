# hud-x24po — Live portal output-scroll sign-off (user-test PSK)

- Date: 2026-06-20
- Bead: `hud-x24po`
- Host: `windows-host.example:50051`
- HUD: `TzeHudPortalVal`, deployed `tze_hud.toml` (agent-alpha/beta/gamma registered), overlay
- Auth: dedicated user-test PSK (`MCP_TEST_PSK`) + `--agent-id agent-alpha`

## Result: PASS

The prior blocker — reviewer-worker live scroll evidence could not be collected
because the reachable Windows gRPC endpoint rejected the **local PSK** with
`AUTH_FAILED` — is resolved by launching the validation HUD with the **user-test
PSK** and connecting as a registered agent (`agent-alpha`). Session established
with `caps=[create_tiles, modify_own_tiles, access_input_events]`.

### gRPC-driven scroll contract (`scroll-transcript.json`)

Transcript Interaction Contract checkpoints, in order:

- `scroll:mount` — bounded output mounted, 80 lines total, 14 visible.
- `scroll:offset` ×4 — `scroll_y` stepped 40→80→120→160 px; `viewport_start`
  advanced 1→3→5→7 (output window steps through transcript).
- `scroll:append` ×5 — lines 80–84 appended **while `scroll_y` stayed pinned at
  160.0** → the visible window did NOT jump to the tail (the core contract).
- `scroll` completed — offset returned to tail (`tail_start=71`), newest output
  visible.
- `cleanup:lease-release` — portal tiles removed cleanly on exit.

### Live compositor/input path (`diagnostic-input-transcript.json`)

Real Windows OS input injection over SSH (`returncode=0`, `ok=true`,
`duration_s=4.37`):

- `input:focus-gained` — runtime focus manager focused the composer hit region
  (pointer down at display 2519,267).
- `drag:start` / `drag:end` — portal dragged via header through the normal input
  path; panes stayed grouped (`portal_x=1972, portal_y=192`, panes aligned).
- Injector actions: `focus-composer`, `drag-portal-header`, `scroll-output-pane`
  (OS wheel), `type-composer-text` — all against the live overlay, so this is
  runtime/input-path evidence rather than synthetic transcript success.

## Artifacts

- `scroll-transcript.json` — gRPC scroll contract transcript.
- `diagnostic-input-transcript.json` — real-OS-input compositor path transcript.
