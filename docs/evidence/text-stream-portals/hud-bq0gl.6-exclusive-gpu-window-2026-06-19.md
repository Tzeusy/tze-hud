# Text Stream Portal Exclusive GPU Window Check - hud-bq0gl.6

Date: 2026-06-19
Issue: `hud-bq0gl.6`
Scope: read-only TzeHouse Windows GPU-window readiness probe

## Summary

The exclusive benchmark-overlay window is not currently available for portal
live evidence runs. The live Windows host is reachable and the production
`TzeHudOverlay` runtime is healthy, but that production process owns both the
runtime ports and the GPU lock.

No process was stopped, no scheduled task was started, no Beads lifecycle state
was mutated, and no runtime credential was recovered or written during this
probe. Host, user, and SSH key names below use the repository's public
placeholders; the operator supplied the real values at dispatch time.

## Bootstrap

- Worktree: `.worktrees/parallel-agents/hud-bq0gl.6`
- Branch: `agent/hud-bq0gl.6`
- `git status --short --branch`: `## agent/hud-bq0gl.6...origin/main`
- Worker context helper: `status=ok`

## Read-Only Live Evidence

SSH and TCP reachability succeeded:

- `ssh ... hud-user@windows-host.example whoami` returned the expected Windows
  account.
- `ssh ... admin-user@windows-host.example whoami` returned the expected
  Windows account.
- TCP ports `22`, `50051`, and `9090` were open on the Windows host.
- `GET http://windows-host.example:9090/mcp` returned HTTP `200` with a
  JSON-RPC parse error for an empty request, proving the MCP HTTP endpoint is
  responsive without exposing a credential.

Windows runtime state, collected via sanitized PowerShell:

```text
C:\ProgramData\tze_hud\gpu.lock
SESSION_TYPE=interactive
PID=42280
STARTED_AT=2026-06-18T14:22:48Z
DESCRIPTION=tze_hud.exe interactive session
```

The only observed `tze_hud.exe` process was PID `42280`:

```text
"C:\tze_hud\tze_hud.exe" --window-mode overlay --psk <redacted> --bind-all-interfaces
```

Runtime port owners:

| Local address | Port | Owning PID |
|---|---:|---:|
| `0.0.0.0` | `50051` | `42280` |
| `0.0.0.0` | `9090` | `42280` |

Scheduled task state:

| Task | State | Last result | Action |
|---|---|---:|---|
| `TzeHudOverlay` | `Running` | `267009` | `C:\tze_hud\tze_hud.exe --window-mode overlay --psk <redacted> --bind-all-interfaces` |
| `TzeHudBenchmarkOverlay` | `Ready` | `0` | `powershell.exe -NoProfile -ExecutionPolicy Bypass -File "C:\tze_hud\run_benchmark_hud.ps1"` |

## Interpretation

The current host state is a production-overlay session, not an exclusive
benchmark-overlay validation window. The lock file PID, process list, port
owners, and `TzeHudOverlay` task all agree on the same live production process.
`TzeHudBenchmarkOverlay` is registered and ready, but it is not running and
cannot be used on `50051/9090` while production owns the GPU lock and ports.

This means the June 15 note that an exclusive GPU window was available is no
longer current as of this probe.

## Recovery Condition

Coordinator/operator action is required before a portal live evidence run can
claim an exclusive benchmark-overlay window:

1. Schedule a short maintenance window where interrupting the visible
   production HUD is acceptable.
2. Record the current `TzeHudOverlay` state.
3. Stop the production `TzeHudOverlay` process.
4. Start `TzeHudBenchmarkOverlay`.
5. Verify the new `tze_hud.exe` process command line references
   `C:\tze_hud\benchmark.toml`, owns `C:\ProgramData\tze_hud\gpu.lock`, and owns
   ports `50051` and `9090`.
6. Recover the runtime credential only into the caller process environment, run
   the requested `text_stream_portal_exemplar.py` live phases, then release the
   portal lease and restore `TzeHudOverlay`.

Until that window is scheduled, this bead should remain blocked rather than
rerunning portal evidence against the production overlay or starting the
benchmark task in a way that disrupts the current HUD unexpectedly.
