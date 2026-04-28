# Text Stream Portal Windows Validation - hud-eq1m4

Date: 2026-04-27
Host: `tzehouse-windows.parrot-hen.ts.net`
Branch: `agent/hud-eq1m4`

## Scope

Validate text stream portal hit-region input on a fresh Windows HUD process after
SSH recovered. The residual watch item from the prior review was a first drag
ending by watchdog before a later `pointer_up` drag succeeded.

## Environment

- SSH key auth succeeded for both `hudbot` and `tzeus`.
- Fresh HUD processes were launched via scheduled task `TzeHudOverlay`.
- gRPC `50051` and MCP `9090` were reachable after each restart.
- The scheduled task carries a non-default PSK; when extracting it over SSH,
  PowerShell output must be normalized by stripping `\r` before setting
  `TZE_HUD_PSK`, or resident gRPC authentication fails with PSK mismatch.

## Script Fix

The first fresh `baseline,scroll` run exposed a deterministic script defect:
the scroll phase remounted the input tile with static composer node IDs and the
runtime rejected the mutation with `duplicate id`.

Fix applied:

- `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py` now creates
  fresh composer root/hit/text/caret node IDs for each full input-tile remount.
- The script still records server-assigned runtime text/caret node IDs after
  remount so live composer updates continue to target the active nodes.

## Evidence

Transcript artifacts were written under local `test_results/`:

- `test_results/text-stream-portal-fresh-hitregion-20260427.json`
- `test_results/text-stream-portal-input-fresh-20260427.json`
- `test_results/text-stream-portal-sendinput-fresh-20260427.json`

`test_results/` is local runtime evidence and is not tracked in git.

### Fresh Baseline + Scroll Run

Command shape:

```bash
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/exemplar-manual-review-checklist.md \
  --phases baseline,scroll \
  --baseline-hold-s 25 \
  --transcript-out test_results/text-stream-portal-fresh-hitregion-20260427.json
```

Observed transcript counts:

- `drag:start`: 4
- `drag:end`: 4
- `scroll:mount`: 1
- `scroll:offset`: 4
- `scroll:append`: 5
- `scroll`: 2 (`started`, `completed`)
- `cleanup:lease-release`: 1

Important drag result:

- At least one drag ended with `reason: "pointer_up"` on the fresh process.
- Earlier drag attempts also ended by `superseded:pointer_down` and
  `idle_release`, so the original "watchdog first, pointer_up later" symptom did
  not recur exactly, but drag termination still has sensitivity to remote input
  timing.

### Fresh Input Run

Command shape:

```bash
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/exemplar-manual-review-checklist.md \
  --phases baseline \
  --baseline-hold-s 30 \
  --transcript-out test_results/text-stream-portal-input-fresh-20260427.json
```

Observed transcript counts:

- `input:focus-gained`: 1
- `input:focus-attempt`: 1
- `input:key-down`: 3
- `input:shortcut-ignored`: 2
- `cleanup:lease-release`: 1

This validates live composer hit-region focus and keyboard event delivery on a
fresh process. The captured key events had `meta=true`, consistent with the
remote key injection path leaving Windows modifier state active.

### Character Event Status

`input:character` was not reproduced in this session. Virtual-key injection over
SSH produced focus and modified key-downs but no printable character events.
A lower-level `SendInput` Unicode attempt failed before input dispatch because
the ad hoc PowerShell `Add-Type` wrapper was malformed.

## Verdict

Fresh-process live hit-region input is partially validated:

- PASS: pointer hit regions reached the resident portal script.
- PASS: a drag ended by `pointer_up` on a fresh process.
- PASS: composer hit region gained focus.
- PASS: key-down events reached the focused composer tile.
- PASS: the scripted scroll phase completes after fixing duplicate composer IDs.
- WATCH: printable `Character` events were not reproduced by SSH-driven
  injection in this session.
- WATCH: drag endings still vary under remote injection (`superseded`,
  `idle_release`, `pointer_up`), although the prior watchdog termination did not
  recur.

## Verification

```bash
python3 -m py_compile .claude/skills/user-test/scripts/text_stream_portal_exemplar.py
```

