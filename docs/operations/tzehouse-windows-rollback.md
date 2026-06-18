# TzeHouse Windows HUD Rollback Runbook

Reference host: `windows-host.example`

This runbook restores the production Windows HUD binary at
`C:\tze_hud\tze_hud.exe` to a prior known-good artifact. It does not change
runtime config, scheduled-task arguments, PSKs, firewall rules, or widget/profile
bundles.

If the host is offline, SSH is unavailable, ports `50051`/`9090` are closed, or
`TzeHudOverlay` is missing, first use
[`docs/operations/tzehouse-windows-recovery.md`](tzehouse-windows-recovery.md).

## Required Inputs

- A prior known-good release bundle containing `tze_hud.exe`.
- The matching pipeline-generated `tze_hud.exe.sha256` from the same release
  artifact bundle.
- Non-interactive SSH access to `admin-user@windows-host.example`, or an
  operator PowerShell session on the Windows host.
- The existing production scheduled task: `TzeHudOverlay`.

Do not recover from a binary that lacks a matching pipeline checksum or from a
manually edited checksum. Signing is deferred for v1, but the SHA-256 artifact
is the current provenance gate.

## Preflight

From the repo root, confirm the Windows host and admin SSH path are reachable:

```bash
timeout 12 tailscale ping -c 1 windows-host.example
timeout 12 ssh -i ~/.ssh/hud-ssh-key -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=8 \
  admin-user@windows-host.example "whoami"
```

On the Windows host, choose the known-good release directory. The directory must
contain both files from the same artifact bundle:

```powershell
$KnownGoodDir = "C:\tze_hud\releases\<known-good-release>"
$KnownGoodExe = Join-Path $KnownGoodDir "tze_hud.exe"
$ChecksumFile = Join-Path $KnownGoodDir "tze_hud.exe.sha256"
$ActiveExe = "C:\tze_hud\tze_hud.exe"
$TaskName = "TzeHudOverlay"
```

## Verify The Known-Good Artifact

Stop immediately if the checksum file is missing, malformed, or mismatched.

```powershell
if (-not (Test-Path -LiteralPath $KnownGoodExe)) {
  throw "Known-good executable missing: $KnownGoodExe"
}
if (-not (Test-Path -LiteralPath $ChecksumFile)) {
  throw "Known-good checksum missing: $ChecksumFile"
}

$ExpectedHash = ((Get-Content -LiteralPath $ChecksumFile -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
$ActualHash = (Get-FileHash -LiteralPath $KnownGoodExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($ActualHash -ne $ExpectedHash) {
  throw "Known-good checksum mismatch: expected $ExpectedHash, got $ActualHash"
}
```

## Preserve The Current Binary

Keep a timestamped copy of the current binary before replacing it. This is for
forensics and operator recovery only; do not promote it again unless it has its
own trusted checksum.

```powershell
$Stamp = Get-Date -Format "yyyyMMddTHHmmss"
$RollbackBackupDir = "C:\tze_hud\rollback\pre-rollback-$Stamp"
New-Item -ItemType Directory -Force -Path $RollbackBackupDir | Out-Null

if (Test-Path -LiteralPath $ActiveExe) {
  Copy-Item -LiteralPath $ActiveExe -Destination (Join-Path $RollbackBackupDir "tze_hud.exe") -Force
}
if (Test-Path -LiteralPath "C:\tze_hud\tze_hud.exe.sha256") {
  Copy-Item -LiteralPath "C:\tze_hud\tze_hud.exe.sha256" `
    -Destination (Join-Path $RollbackBackupDir "tze_hud.exe.sha256") -Force
}
```

## Replace And Verify The Active Binary

Stop the scheduled task, kill any stuck process, copy the known-good executable
into place, and verify the active path against the known-good checksum.

```powershell
schtasks /End /TN $TaskName 2>$null | Out-Null
Get-Process tze_hud -ErrorAction SilentlyContinue | Stop-Process -Force

Copy-Item -LiteralPath $KnownGoodExe -Destination $ActiveExe -Force
Copy-Item -LiteralPath $ChecksumFile -Destination "C:\tze_hud\tze_hud.exe.sha256" -Force

$ActiveHash = (Get-FileHash -LiteralPath $ActiveExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($ActiveHash -ne $ExpectedHash) {
  throw "Active executable checksum mismatch after rollback: expected $ExpectedHash, got $ActiveHash"
}
```

## Restart TzeHudOverlay

Restart through Task Scheduler. Do not launch `tze_hud.exe` directly over SSH;
the overlay must run in the interactive desktop task context.

```powershell
schtasks /Run /TN $TaskName
Start-Sleep -Seconds 5
schtasks /Query /TN $TaskName /V /FO LIST
```

Then repeat the port and MCP/gRPC smoke checks from
[`docs/operations/tzehouse-windows-recovery.md`](tzehouse-windows-recovery.md#smoke-before-soak).
If `22` works but `50051` or `9090` stays closed, follow that recovery runbook's
HUD task bring-up section before attempting another rollback.

## Local Smoke Test

The dry-run fixture exercises the same rollback shape without touching the real
Windows host:

```bash
bash scripts/ci/rollback_smoke_test.sh
```

The fixture maps `C:\tze_hud\tze_hud.exe` to a temporary directory, verifies a
known-good `tze_hud.exe.sha256`, replaces the active binary, verifies the active
checksum, and uses a mocked `schtasks` command to assert the documented
`TzeHudOverlay` restart step.
