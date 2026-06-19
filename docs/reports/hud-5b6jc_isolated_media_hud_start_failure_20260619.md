# hud-5b6jc Isolated Windows Media HUD Start Failure

Date: 2026-06-19
Host: TzeHouse (tailnet hostname intentionally omitted from committed report)
Worker branch: `agent/hud-5b6jc`

## Result

Root cause found and validated.

The isolated Windows media HUD failed before binding `50052/9092` because the
launch harness force-stopped the currently running production `tze_hud.exe` and
then started the isolated task as soon as port `50051` closed. Closing the port
does not guarantee the process has exited or released
`C:\ProgramData\tze_hud\gpu.lock`. When the next `tze_hud.exe` sees a live or
stale GPU lock for the previous HUD process, it exits with status `1` before
binding either gRPC or MCP.

The same race exists in reverse during restore: force-stopping the isolated HUD
can leave a stale GPU lock until the stopped PID is gone and the lock file is
removed.

## Fix Shape

The safe launch/restore sequence is:

1. Stop the currently running scheduled task.
2. Force-stop the owning `tze_hud.exe` process.
3. Wait for the relevant port to close.
4. Wait for the owning process ID to disappear from `Get-Process`.
5. If `C:\ProgramData\tze_hud\gpu.lock` still names that now-dead PID, remove
   only that stale lock file.
6. Start the next HUD task.

The helper artifact implements this flow without printing PSK values:

```text
docs/reports/artifacts/hud-5b6jc-isolated-start-recovery-20260619T133326Z/start-isolated-media-hud-wait-production-exit.ps1
```

## Verification

Final live verification artifact:

```text
docs/reports/artifacts/hud-5b6jc-isolated-start-recovery-20260619T133326Z/start-isolated-media-hud-wait-production-exit.json
```

Observed in that artifact:

- production baseline: PID `10624` owned `50051/9090`
- production PID exited before isolated launch
- stale production GPU lock for PID `10624` was removed
- isolated HUD bound `50052/9092` as PID `62352`
- isolated PID exited during cleanup
- stale isolated GPU lock for PID `62352` was removed
- production restored successfully as PID `56244` on `50051/9090`

Fresh post-run remote snapshot confirmed only production listeners remained:

```json
[
  {"LocalAddress":"0.0.0.0","LocalPort":50051,"OwningProcess":56244},
  {"LocalAddress":"0.0.0.0","LocalPort":9090,"OwningProcess":56244}
]
```

The temporary `TzeHud5b6jcMedia` task was unregistered after the run.

## Preserved Failed Attempts

The same artifact directory keeps two intermediate diagnostic runs:

- `start-isolated-media-hud-wait-production-exit-script-bug.json`: first helper
  bug; PowerShell `$PID` is read-only, so using `$pid` as a loop/parameter
  variable aborts the script.
- `start-isolated-media-hud-wait-production-exit-stale-lock.json`: isolated
  bind succeeded, but production restore failed until the stale isolated GPU
  lock was removed.

These files are retained because they explain why the final script includes both
PID-wait and stale-lock cleanup in both directions.
