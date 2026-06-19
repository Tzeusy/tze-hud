# Text Stream Portal Resize Hotkeys - hud-v4k1h

Date: 2026-06-19
Issue: `hud-v4k1h`
Branch: `agent/hud-v4k1h`
Scope: focused-portal Ctrl resize hotkeys

## Summary

The remaining runtime failure after PR #917 was a live Windows input-shape
mismatch. The PR #923 run showed that the focused portal accepted ordinary
typing and cleanup completed, but the injected resize keys arrived as
Ctrl-modified `KeyUp` events without matching `KeyDown` events for `Equal` or
`Minus`. The runtime only applied focused-portal resize hotkeys on `KeyDown`, so
those release-only live events had no visible effect.

This branch adds a focused runtime fallback that applies the same resize action
on Ctrl-modified `KeyUp` when no matching resize `KeyDown` was consumed. Normal
physical key-down/key-up pairs still resize exactly once.

## Prior Live Evidence Used

Source: PR #923 / `agent/hud-sp8l7` transcript from
`test_results/hud-sp8l7-portal-resize-hotkeys-raw.json`.

Observed successful setup:

- resident portal opened against the Windows runtime
- `cleanup_errors=[]`
- normal typed feedback reached the focused composer
- operator verdict: `ctrl +/- still doesn't work`

Observed failing hotkey stream:

- Control key down with `ctrl=true`
- repeated `Equal` key-up events with `ctrl=true`
- no corresponding `Equal` key-down event with `ctrl=true`
- Control key down with `ctrl=true`
- repeated `Minus` key-up events with `ctrl=true`
- no corresponding `Minus` key-down event with `ctrl=true`

## Local Runtime Verification

The fix was driven by a focused failing regression test before implementation:

```text
timeout 180s cargo test -p tze_hud_runtime ctrl_resize_keyup_fallback_resizes_when_live_windows_omits_keydown --lib
```

Initial result before the fix:

```text
FAILED
Ctrl+= release fallback must grow the focused portal when the live OS stream omitted Equal KeyDown
```

Post-fix focused verification:

```text
timeout 180s cargo test -p tze_hud_runtime ctrl_resize --lib
```

Result:

```text
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 765 filtered out
```

The covered cases are:

- key-up fallback grows on `Ctrl+=` when the live stream omits resize `KeyDown`
- key-up fallback shrinks on `Ctrl+-` when the live stream omits resize `KeyDown`
- a matching key-up after an already-consumed resize key-down is swallowed
- resize hotkeys do not leak into the focused composer/agent input path
- unfocused portals and safe mode preserve the existing guard behavior

Additional local gates:

```text
cargo fmt --all --check
pass

timeout 180s cargo check -p tze_hud_runtime
pass
```

## Live Windows Rerun Status

The required visual Windows rerun could not be completed from this worker
environment.

Evidence:

```text
pwd -P
/home/tze/gt/tze_hud/mayor/rig/.worktrees/parallel-agents/hud-v4k1h

git branch --show-current
agent/hud-v4k1h

getent hosts windows-host.example
<no result>

tailscale ping -c 1 windows-host.example
error looking up IP of "windows-host.example": lookup windows-host.example on 127.0.0.53:53: no such host

test -f docs/operations/private/tzehouse-windows.local.md
missing

test -f ~/.ssh/hud-ssh-key
hud_key_missing

test -n "${TZE_HUD_PSK:-}"
TZE_HUD_PSK_missing

test -n "${MCP_TEST_PSK:-}"
MCP_TEST_PSK_missing
```

The underlying Windows peer itself is present on the tailnet. The real DNS name
and IP came from `tailscale status --json` and are intentionally represented
below with the repository's public placeholder.

```text
tailscale status --json | jq -r '.Peer[]? | select(.HostName=="TzeHouse" and .OS=="windows") | [.HostName, .DNSName, (.TailscaleIPs[0] // ""), (.Online|tostring), (.OS // "")] | @tsv'
TzeHouse    windows-host.example.    <tailnet-ip>    true    windows

tailscale ping -c 1 <real-windows-peer-dns>
pong from <real-windows-peer>

TCP probes:
22 open
50051 open
9090 open
```

But the deploy and OS-input path is blocked by missing local auth material:

```text
ssh-add -l
The agent has no identities.

ssh -i ~/.ssh/id_rsa -o IdentitiesOnly=yes -o BatchMode=yes admin-user@<real-windows-peer-dns> whoami
Permission denied (publickey,password,keyboard-interactive).

ssh -i ~/.ssh/id_rsa -o IdentitiesOnly=yes -o BatchMode=yes hud-user@<real-windows-peer-dns> whoami
Permission denied (publickey,password,keyboard-interactive).

ssh -i ~/.ssh/tzec2.pem -o IdentitiesOnly=yes -o BatchMode=yes admin-user@<real-windows-peer-dns> whoami
Permission denied (publickey,password,keyboard-interactive).

ssh -i ~/.ssh/tzec2.pem -o IdentitiesOnly=yes -o BatchMode=yes hud-user@<real-windows-peer-dns> whoami
Permission denied (publickey,password,keyboard-interactive).

python3 scripts/mcp_reachability_check.py --url http://<real-windows-peer-dns>:9090/mcp --psk-env TZE_HUD_PSK
ERROR: env var 'TZE_HUD_PSK' is unset or empty. Set it to the MCP pre-shared key.
```

## Resume Condition

To complete the expected visual acceptance evidence, resume from this branch
with the repo-private Windows mapping, accepted `~/.ssh/hud-ssh-key`, and live
HUD PSK available. Rebuild/deploy this branch's `tze_hud.exe`, run the focused
resident text-stream portal path, inject Ctrl+plus/Ctrl+equals/Ctrl+minus
through the interactive scheduled-task input injector, and record that the
focused portal visibly grows then shrinks.
