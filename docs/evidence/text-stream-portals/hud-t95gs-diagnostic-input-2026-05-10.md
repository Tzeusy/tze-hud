# Text Stream Portal Diagnostic Input Injector - hud-t95gs

Date: 2026-05-10
Branch: `agent/hud-t95gs`

## Scope

Add a durable diagnostic input path for live text-stream portal validation when
manual Windows console input is unavailable. The path must enter the real
runtime/compositor input route and must not synthesize transcript-only success.

## Implementation

`.claude/skills/user-test/scripts/text_stream_portal_exemplar.py` now supports
`--phases diagnostic-input`.

The phase mounts the normal resident raw-tile portal over `HudSession`, then
uses SSH to run a PowerShell OS input script on the Windows desktop user. The
script calls:

- `SetCursorPos` to target live overlay coordinates
- `mouse_event` left down/move/up for header drag
- `mouse_event` wheel for OUTPUT pane scroll
- `SendInput` with Unicode scan codes for composer text

This means focus, drag, and scroll still pass through the windowed runtime's
winit event handling, hit testing, focus manager, capture handling, scroll
processor, and agent input-event dispatch.

## Expected Live Evidence

Run shape:

```bash
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/exemplar-manual-review-checklist.md \
  --phases diagnostic-input \
  --transcript-out test_results/text-stream-portal-diagnostic-input.json
```

A passing transcript should include:

- `diagnostic-input` started/completed
- `input:focus-gained`
- `input:character` or `input:key-down` for composer input
- `drag:start` and `drag:end`
- `scroll:output`
- `cleanup:lease-release`

## Local Verification

```bash
python3 -m py_compile .claude/skills/user-test/scripts/text_stream_portal_exemplar.py
python3 .claude/skills/user-test/tests/test_text_stream_portal_exemplar.py
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py --self-test
```

Local result: all commands passed.

## Residual Live Blocker

This change exposes the diagnostic hook and validates its generated OS-input
script shape locally. Final sign-off for `hud-eq1m4` still requires a reachable
Windows HUD target with `TZE_HUD_PSK` set and the overlay running with gRPC
enabled so the `test_results/text-stream-portal-diagnostic-input.json`
transcript can be captured.

Reachability attempt from this worktree:

```bash
timeout 8s ssh -o ConnectTimeout=5 -o BatchMode=yes -o IdentitiesOnly=yes \
  -o StrictHostKeyChecking=no -i ~/.ssh/ecdsa_home \
  hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"

timeout 8s ssh -o ConnectTimeout=5 -o BatchMode=yes -o IdentitiesOnly=yes \
  -o StrictHostKeyChecking=no -i ~/.ssh/ecdsa_home \
  tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"
```

Both returned:

```text
ssh: connect to host tzehouse-windows.parrot-hen.ts.net port 22: Connection timed out
```
