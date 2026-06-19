# Text Stream Portal Resize Hotkeys - hud-v4k1h

Date: 2026-06-19
Issue: `hud-v4k1h`
Branch: `agent/hud-v4k1h`
PR: `#924`
Scope: focused-portal Ctrl resize hotkeys

## Summary

This branch contains a focused runtime fix for the PR #923 input shape:
Ctrl-modified `KeyUp` events for `=`/`+`/`-` now apply the focused portal
resize fallback when no matching resize `KeyDown` was consumed. Normal
key-down/key-up pairs still resize exactly once because matching key-ups are
swallowed after a consumed resize key-down.

Local runtime tests pass, but the corrected live `/user-test` route still does
not produce acceptance evidence. The current blocker is the Windows automation
input route: the focused portal receives pointer focus and modifier keys, but
the automated `=`/`+`/`-` target keys do not reach the runtime, so no
`element_repositioned` geometry events occur.

PR #924 should remain draft/blocked until a real operator keypress or a working
interactive Windows key injector can produce the target OEM key events and the
geometry evidence.

## Correct Live Route

Validated route:

- Windows host: `tzehouse-windows.parrot-hen.ts.net`
- SSH key: `~/.ssh/ecdsa_home`
- SSH flags: `-i ~/.ssh/ecdsa_home -o BatchMode=yes -o IdentitiesOnly=yes`
- Interactive user used for input injection: `tzeus`
- Secondary SSH user checked: `hudbot`
- gRPC target: `tzehouse-windows.parrot-hen.ts.net:50051`
- MCP URL: `http://tzehouse-windows.parrot-hen.ts.net:9090/mcp`
- PSK source: `schtasks /Query /TN TzeHudOverlay /XML`, with CR stripped before use

Authentication check:

```text
ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes -o IdentitiesOnly=yes \
  tzeus@tzehouse-windows.parrot-hen.ts.net whoami
tzehouse\tzeus

ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes -o IdentitiesOnly=yes \
  hudbot@tzehouse-windows.parrot-hen.ts.net whoami
tzehouse\hudbot
```

Deployed binary check:

```text
sha256sum target/x86_64-pc-windows-gnu/release/tze_hud.exe
bd545d13cf87dd5b30670bfdf1c6b9fc074ada1b95f6252a17bfb5b87cf8686f

Get-FileHash C:\tze_hud\tze_hud.exe -Algorithm SHA256
BD545D13CF87DD5B30670BFDF1C6B9FC074ADA1B95F6252A17BFB5B87CF8686F

Get-Process tze_hud
Id=38228, ProcessName=tze_hud

Test-NetConnection 127.0.0.1 -Port 50051
TcpTestSucceeded=true

Test-NetConnection 127.0.0.1 -Port 9090
TcpTestSucceeded=true
```

## Prior Live Evidence Used

Source: PR #923 / `agent/hud-sp8l7`

Observed successful setup:

- resident portal opened against the Windows runtime
- cleanup completed with `cleanup_errors=[]`
- normal typed feedback reached the focused composer
- operator verdict: `ctrl +/- still doesn't work`

Observed failing hotkey stream in PR #923:

- Control key down with `ctrl=true`
- repeated `Equal` key-up events with `ctrl=true`
- no corresponding forwarded `Equal` key-down event
- Control key down with `ctrl=true`
- repeated `Minus` key-up events with `ctrl=true`
- no corresponding forwarded `Minus` key-down event

## Local Runtime Verification

Focused runtime gate:

```text
timeout 180s cargo test -p tze_hud_runtime ctrl_resize --lib
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 765 filtered out
```

Additional local gates previously run on this branch:

```text
cargo fmt --all --check
pass

timeout 180s cargo check -p tze_hud_runtime
pass
```

## Live Control Run

Control transcript:
`docs/evidence/text-stream-portals/hud-v4k1h-diagnostic-input-control-2026-06-19.json`

Command shape:

```bash
PSK=$(ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes -o IdentitiesOnly=yes \
  tzeus@tzehouse-windows.parrot-hen.ts.net \
  "schtasks /Query /TN TzeHudOverlay /XML" \
  | tr -d '\r' \
  | python3 -c 'import re,sys; s=sys.stdin.read(); m=re.search(r"--psk\s+([^\s<]+)", s); sys.exit(2) if not m else print(m.group(1))')

TZE_HUD_PSK="$PSK" python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/reports/exemplar-manual-review-checklist.md \
  --phases diagnostic-input \
  --diagnostic-input-user tzeus \
  --diagnostic-input-ssh-key ~/.ssh/ecdsa_home \
  --diagnostic-input-timeout-s 24 \
  --diagnostic-input-connect-timeout-s 8 \
  --cleanup-timeout-s 10 \
  --transcript-out docs/evidence/text-stream-portals/hud-v4k1h-diagnostic-input-control-2026-06-19.json
```

Observed control result:

- `input:focus-gained`
- `input:focus-attempt` at display coordinates `2519,267`
- `scroll:output` checkpoints at offsets `120`, `240`, and `360`
- injector stdout: `focus-composer`, `drag-portal-header`, `scroll-output-pane`, `type-composer-text`
- `cleanup:lease-release` completed

This proves the corrected route can drive the live HUD through pointer, wheel,
focus, and text-path automation.

## Live Resize-Hotkey Run

Resize transcript:
`docs/evidence/text-stream-portals/hud-v4k1h-portal-resize-hotkeys-2026-06-19.json`

Command shape:

```bash
PSK=$(ssh -i ~/.ssh/ecdsa_home -o BatchMode=yes -o IdentitiesOnly=yes \
  tzeus@tzehouse-windows.parrot-hen.ts.net \
  "schtasks /Query /TN TzeHudOverlay /XML" \
  | tr -d '\r' \
  | python3 -c 'import re,sys; s=sys.stdin.read(); m=re.search(r"--psk\s+([^\s<]+)", s); sys.exit(2) if not m else print(m.group(1))')

TZE_HUD_PSK="$PSK" timeout 180s python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/reports/exemplar-manual-review-checklist.md \
  --phases resize-hotkeys \
  --diagnostic-input-user tzeus \
  --diagnostic-input-ssh-key ~/.ssh/ecdsa_home \
  --diagnostic-input-timeout-s 24 \
  --diagnostic-input-connect-timeout-s 8 \
  --cleanup-timeout-s 10 \
  --transcript-out docs/evidence/text-stream-portals/hud-v4k1h-portal-resize-hotkeys-2026-06-19.json
```

Observed resize result:

- scene display area: `3840x2160`
- initial focused input tile geometry: `x=2092`, `y=195`, `width=854`, `height=1228`
- `input:focus-gained` and `input:focus-attempt` were observed
- injector stdout completed all planned actions: `focus-composer`, `settle-focus`, `ctrl-equals`, `ctrl-plus`, `ctrl-minus`
- runtime observed modifier events (`Control`, and during the plus attempt `Shift`)
- runtime did not observe `=`, `+`, or `-` key events from the automation route
- no `element_repositioned` events were observed for the focused input tile
- failed step recorded: `timed out waiting for focused portal resize geometry event`
- `observed_geometries=[]`
- `cleanup_errors=[]`
- `cleanup:lease-release` completed

## Blocker

The branch is blocked on live acceptance, not on SSH, PSK, process, ports, or
basic input reachability. The specific remaining blocker is:

```json
{
  "symptom": "corrected Windows route can focus the portal, but automated Ctrl resize injection does not deliver =/+/- target key events to the runtime",
  "impact": "Ctrl+=, Ctrl++, and Ctrl+- cannot yet be evidenced as visibly growing/shrinking the focused portal",
  "evidence": [
    "docs/evidence/text-stream-portals/hud-v4k1h-diagnostic-input-control-2026-06-19.json",
    "docs/evidence/text-stream-portals/hud-v4k1h-portal-resize-hotkeys-2026-06-19.json"
  ],
  "next_resume_condition": "Use a real operator keypress on the focused portal or a Windows injector that produces target OEM key events for =, +, and -; rerun resize-hotkeys and require three input-tile geometry events: grow, grow, shrink."
}
```
