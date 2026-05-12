# hud-i1nrq Exclusive Window Rerun Blocked

Date: 2026-05-12

## Scope

This pass resumed `hud-i1nrq` after `hud-fojgj` removed the path-specific
Windows Firewall blocker for the isolated media-ingress validation executable
paths.

The worker branch `.worktrees/parallel-agents/hud-i1nrq` merged:

- current `origin/main` at `36f2cf8d`
- PR #659 stack head `origin/agent/hud-d82p7` at `1cfec13f`

No PR branch was force-pushed.

## Prepared Validation

Local validation passed before touching the Windows runtime:

```bash
python3 -m py_compile \
  .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py
python3 .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py
python3 .claude/skills/user-test/tests/test_windows_media_ingress_exemplar.py
cargo build --release -p tze_hud_app --target x86_64-pc-windows-gnu
```

Results:

- Python script tests: 16 passed
- user-test media ingress tests: 6 passed
- Windows release build: passed
- built binary SHA-256:
  `0b6a9caf6586c3216f7d5a8cb0142c6c838ceeb26868883089962b1dc4ee8aae`

The binary and media-ingress config were deployed to the already
firewall-allowed path:

```text
C:\tze_hud\hud-s0pit-rerun\tze_hud.exe
C:\tze_hud\hud-s0pit-rerun\windows-media-ingress.toml
```

The copied config rewrote bundle paths to the shared Windows locations:

```toml
[widget_bundles]
paths = ["C:/tze_hud/widget_bundles"]

[component_profile_bundles]
paths = ["C:/tze_hud/profiles"]
```

## Pre-Exclusive State

Immediately before the exclusive window, the Windows host was reachable and the
benchmark HUD was the only observed HUD process:

```text
PID 49856
C:\tze_hud\tze_hud.exe
--config C:\tze_hud\benchmark.toml --window-mode overlay --grpc-port 50051 --mcp-port 9090
```

Listeners were present on `0.0.0.0:50051` and `0.0.0.0:9090`; alternate ports
`50052/9091` were free.

## Blocker

The exclusive-window launch did not reach the isolated HUD start. The remote
PowerShell launch script stopped at cleanup of a missing temp task:

```text
schtasks.exe : ERROR: The system cannot find the file specified.
```

The script was running with `$ErrorActionPreference = "Stop"`, so the native
`schtasks /Delete /TN TzeHudS0pitRerun` miss aborted the launch path before the
new isolated task could be created and run.

After that interruption, the Windows peer dropped off the tailnet:

```text
tailscale ping tzehouse-windows.parrot-hen.ts.net -> timed out/no reply
ssh tzeus@tzehouse-windows.parrot-hen.ts.net -> port 22 timeout
ssh hudbot@tzehouse-windows.parrot-hen.ts.net -> port 22 timeout
tailscale status -> offline, last seen about 2 minutes earlier
```

Because SSH was no longer reachable, the worker could not verify whether
`TzeHudBenchmarkOverlay` was restored or whether any HUD process remained
running. The repository runbook `docs/operations/tzehouse-windows-recovery.md`
still states that there is no supported remote Wake-on-LAN or
Synology-mediated recovery path when the Windows node is offline and SSH times
out.

## Evidence

Artifact directory:

```text
docs/reports/artifacts/hud-i1nrq-youtube-bridge-live-20260512T174854Z/
```

Files:

- `pre-exclusive-state.json`
- `exclusive-window-interrupted.json`
- `isolated-media-hud-launch.json`

No YouTube frame bridge or local-producer MediaIngressOpen command was run in
this pass. The blocker occurred before a live gRPC client could connect to
`50052`.

## Follow-Up

Reopen the existing TzeHouse reachability blocker and make it block
`hud-i1nrq`. When the host returns, first restore or verify
`TzeHudBenchmarkOverlay` on `50051/9090`; then rerun the exclusive
media-ingress validation with missing-temp-task cleanup treated as non-fatal.
