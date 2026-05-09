# Text Stream Portal Windows Rerun - hud-eq1m4

Date: 2026-05-09
Host: `tzehouse-windows.parrot-hen.ts.net`
Branch: `agent/hud-eq1m4-rerun`

## Scope

Rerun the live Windows HUD text-stream portal validation after SSH and HUD
reachability recovered. The target axes were click-to-focus, header drag,
wheel scroll event delivery, and cleanup/lease release.

## Bootstrap And Reachability

- Worker context matched the assigned worktree:
  `/home/tze/gt/tze_hud/mayor/rig/.worktrees/parallel-agents/hud-eq1m4-rerun`.
- Current branch matched the assigned branch: `agent/hud-eq1m4-rerun`.
- The bundled `beads-worker` assertion rejected the rerun branch because it
  expects exactly `agent/<ISSUE_ID>`; an equivalent assertion against the
  explicit assignment passed.
- SSH key auth succeeded for both users:
  - `hudbot@tzehouse-windows.parrot-hen.ts.net`
  - `tzeus@tzehouse-windows.parrot-hen.ts.net`
- HUD ports were listening from the Windows host:
  - gRPC `127.0.0.1:50051`
  - MCP `127.0.0.1:9090`
- The live resident session reported scene display area `3840x2160`.

## Commands

Resident portal runs used the scheduled-task PSK from `TzeHudOverlay`, with
PowerShell carriage returns stripped before exporting `TZE_HUD_PSK`.

```bash
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/exemplar-manual-review-checklist.md \
  --phases baseline,scroll \
  --baseline-hold-s 35 \
  --transcript-out test_results/text-stream-portal-rerun-hitregion-20260509.json
```

```bash
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/exemplar-manual-review-checklist.md \
  --phases baseline,scroll \
  --baseline-hold-s 45 \
  --transcript-out test_results/text-stream-portal-rerun-hitregion-input-20260509.json
```

```bash
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/exemplar-manual-review-checklist.md \
  --phases baseline \
  --baseline-hold-s 70 \
  --transcript-out test_results/text-stream-portal-rerun-live-input-final-20260509.json
```

Input injection was run as interactive scheduled tasks under `tzeus`, because a
direct SSH process could not move the console cursor. The final injector used
`SetCursorPos` for exact placement plus `SendInput` for button and wheel events.
Its local evidence log is:

- `test_results/hud_eq1m4_setcursor_sendinput.log`

## Evidence Summary

Local transcript artifacts:

| Artifact | Scene | Live input events | Scripted scroll | Cleanup |
|---|---:|---:|---:|---:|
| `test_results/text-stream-portal-rerun-hitregion-20260509.json` | `3840x2160` | none | `scroll:offset=4`, `scroll:append=5` | pass, `cleanup_errors=[]` |
| `test_results/text-stream-portal-rerun-hitregion-input-20260509.json` | `3840x2160` | none | `scroll:offset=4`, `scroll:append=5` | pass, `cleanup_errors=[]` |
| `test_results/text-stream-portal-rerun-live-input-final-20260509.json` | `3840x2160` | none | not requested | pass, `cleanup_errors=[]` |

The transcript counters stayed at zero for:

- `input:focus-attempt`
- `input:focus-gained`
- `input:key-down`
- `input:character`
- `drag:start`
- `drag:end`
- `scroll:output`

The final injector did execute during the mounted portal window and landed at
the intended physical cursor coordinates:

```text
after_focus=2111,209
after_drag=2208,127
after_wheel=2402,275
```

Those coordinates are the `2560x1440` Windows input-space equivalent of the
portal's `3840x2160` scene-space coordinates.

## Verdict

Partial pass:

- PASS: SSH reachability remained restored for both `hudbot` and `tzeus`.
- PASS: HUD gRPC and MCP ports remained reachable.
- PASS: resident text-stream portal lease request, tile creation, and baseline
  render completed on the live HUD.
- PASS: scripted scroll phase completed on the live HUD.
- PASS: cleanup/lease release completed every run with `cleanup_errors=[]`.

Failed live sign-off:

- FAIL: click-to-focus did not produce `input:focus-attempt` or
  `input:focus-gained`.
- FAIL: header drag did not produce `drag:start` or `drag:end`.
- FAIL: wheel injection over the output pane did not produce `scroll:output`.

This leaves the original live hit-region/input sign-off unresolved. The host
and HUD are reachable, and cleanup is healthy, but synthetic input delivered
from an interactive scheduled task did not reach the portal's subscribed input
event stream during this rerun.

## Follow-Up

The next pass should use a true operator/manual pointer interaction on the
Windows console, or add a runtime-supported diagnostic input injection path that
enters the same compositor/input pipeline as physical pointer and wheel events.
The current SSH/scheduled-task injector is sufficient to move the interactive
cursor but was not sufficient to validate HUD hit-region event delivery.
