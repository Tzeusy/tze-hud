# Text Stream Portal Resize Hotkeys Live Evidence - hud-sp8l7

Date: 2026-06-19
Issue: `hud-sp8l7`
Related fix: PR #917 / `hud-maq82`
Result: FAIL

## Scope

Ran the live Windows `/user-test` resident text-stream portal path against a
fresh `tze_hud.exe` built from worktree HEAD `763326db0ab8c84b3dd047b8c202a68be2d8a396`,
which contains PR #917 (`2241be23 fix: route portal resize hotkeys with composer focus (#917)`).

The committed JSON transcript is sanitized:
`docs/evidence/text-stream-portals/hud-sp8l7-portal-resize-hotkeys-2026-06-19.json`.
Private host/user/key values are replaced with the public placeholders used by
the repository docs. The runtime PSK was recovered only into process environment
and is not stored here.

## Deployment Facts

- Build command: `cargo build -p tze_hud_app --bin tze_hud --release --target x86_64-pc-windows-gnu`
- Build result: pass
- Local binary: `target/x86_64-pc-windows-gnu/release/tze_hud.exe`
- Local and remote SHA-256: `23e35485365ed0430c26183a4ecb6da5f820995b41a1e80a58bcf5e5f43f60f2`
- Remote size: `23024606` bytes
- Remote task: `TzeHudOverlay`
- Remote launch shape: `C:\tze_hud\tze_hud.exe --window-mode overlay --psk <redacted> --bind-all-interfaces`
- Restart verification: fresh `tze_hud.exe` PID owned ports `50051` and `9090`.

## Live Run

First attempt used `agent-hud-sp8l7` and failed before creating a lease:

```text
Lease denied [PERMISSION_DENIED]: requested lease scope exceeds session-granted capabilities: create_tiles, modify_own_tiles, access_input_events
```

The successful live run used the already authorized `agent-alpha` resident
session:

```bash
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target windows-host.example:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/reports/exemplar-manual-review-checklist.md \
  --phases baseline \
  --baseline-hold-s 120 \
  --cleanup-timeout-s 10 \
  --transcript-out test_results/hud-sp8l7-portal-resize-hotkeys-raw.json
```

The portal opened at live display size `3840x2160`; the resolved portal size was
`1720x1360`. The session established with `create_tiles`, `modify_own_tiles`,
and `access_input_events`, rendered the baseline portal, and released the lease
cleanly on exit.

## Hotkey Exercise

During the baseline hold, an interactive Windows scheduled task sent this
sequence to the live desktop:

1. Click the focused portal composer area.
2. Send `Ctrl+plus` via `Ctrl+Shift+OEM_PLUS`.
3. Send `Ctrl+equals` via `Ctrl+OEM_PLUS`.
4. Send `Ctrl+minus` via `Ctrl+OEM_MINUS`.

The injector completed successfully:

```text
hotkey:ctrl-plus:start
hotkey:ctrl-plus:done
hotkey:ctrl-equals:start
hotkey:ctrl-equals:done
hotkey:ctrl-minus:start
hotkey:ctrl-minus:done
```

The portal transcript also captured keyboard delivery around the same live
interaction window. The resize key-downs did not produce a visible accepted
state. The operator submitted this direct portal feedback:

```text
ctrl +/- still doesn't work
```

## Verdict

This run does not satisfy `hud-sp8l7` acceptance criteria. The portal was live,
focused, and running a PR #917-containing Windows build, but the operator did not
observe visible growth from `Ctrl+plus` / `Ctrl+equals` or visible shrink from
`Ctrl+minus`.

## Follow-Up Candidate

Create or continue a blocking input/runtime bug for the remaining live failure:

- Title: `Live focused portal Ctrl resize hotkeys still have no visible effect after PR #917`
- Type: `bug`
- Priority: `1`
- Depends on: `hud-sp8l7`
- Rationale: live Windows evidence on the PR #917 build still fails the visible
  resize contract; transcript shows focused portal input and explicit operator
  failure feedback.
- Unblock condition: focused Windows portal visibly grows on `Ctrl+plus` /
  `Ctrl+equals` and visibly shrinks on `Ctrl+minus`, with transcript or evidence
  recorded under `docs/evidence/text-stream-portals/`.
