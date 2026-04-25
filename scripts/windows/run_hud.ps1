# scripts/windows/run_hud.ps1
#
# Lock-aware HUD launcher for interactive /user-test sessions.
#
# PURPOSE
#   Short-term bridge (hud-oooc9) that integrates GPU lock acquisition into the
#   HUD launch sequence so an interactive session can never start without holding
#   the lock.  This prevents races with the nightly real-decode CI job that runs
#   on the same RTX 3080.
#
# SUPERSEDED BY
#   hud-940e4 — native runtime lock support.  Once tze_hud.exe writes and
#   releases the GPU lock itself, this wrapper is no longer needed for lock
#   management.  The scheduled-task launch path (schtasks /Run /TN TzeHudOverlay)
#   remains mandatory for overlay transparency regardless of native lock support.
#
# LAUNCH CONSTRAINT — READ THIS BEFORE MODIFYING
#   tze_hud.exe MUST be started via the Windows Task Scheduler task
#   "TzeHudOverlay" (run as the interactive desktop user "tzeus").  A direct
#   Start-Process or SSH-spawned launch produces a grey/opaque window because the
#   process cannot access the desktop GPU and WS_EX_NOREDIRECTIONBITMAP is not
#   honoured outside an interactive session.  This wrapper triggers the task; it
#   does NOT spawn tze_hud.exe directly.
#
#   See: docs/ci/windows-d18-runner-setup.md §7
#        docs/design/tzehouse-windows-gpu-scheduling.md §3
#        .claude/skills/user-test/SKILL.md ("Behavior Rules")
#
# USAGE
#   run_hud.ps1 [-TaskName <name>] [-PollIntervalSec <n>] [-TimeoutSec <n>]
#               [-Description <text>] [-WhatIf]
#
# PARAMETERS
#   -TaskName        Scheduled task name (default: TzeHudOverlay)
#   -PollIntervalSec How often to poll for tze_hud.exe presence (default: 2)
#   -TimeoutSec      Seconds to wait for tze_hud.exe to appear after task start
#                    before giving up (default: 30)
#   -Description     Human-readable reason written into the GPU lock file
#   -WhatIf          Dry-run: validate locks and task existence without launching
#
# EXIT CODES
#   0   HUD session ran and exited cleanly (tze_hud.exe process gone)
#   1   GPU lock held by a live CI job — refused to launch
#   2   GPU lock or release encountered an unexpected error
#   3   Scheduled task not found or failed to start
#   4   tze_hud.exe did not appear within -TimeoutSec seconds
#   5   Internal error (lock script path missing, etc.)
#
# LOCK SCRIPTS
#   This wrapper delegates acquire/release to the canonical helper scripts at
#   scripts/ci/windows/gpu-lock-start.ps1 and gpu-lock-release.ps1.
#   Those scripts are the single source of truth for lock file format and path.
#
# COMPATIBILITY
#   Windows PowerShell 5.1 and PowerShell 7+.
#   No modules beyond the core Windows-bundled set are required.

[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory=$false)]
    [string]$TaskName = "TzeHudOverlay",

    [Parameter(Mandatory=$false)]
    [int]$PollIntervalSec = 2,

    [Parameter(Mandatory=$false)]
    [int]$TimeoutSec = 30,

    [Parameter(Mandatory=$false)]
    [string]$Description = "tze_hud.exe /user-test overlay session",

    [Parameter(Mandatory=$false)]
    [switch]$WhatIf
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── Helpers ──────────────────────────────────────────────────────────────────

function Write-HudLog([string]$msg) {
    Write-Host "[run_hud] $msg"
}

function Exit-WithCode([int]$code, [string]$reason) {
    Write-HudLog "EXIT $code — $reason"
    exit $code
}

# ── Locate lock helper scripts ────────────────────────────────────────────────
#
# Helper scripts live in scripts/ci/windows/ relative to this script's own
# directory (../ci/windows/).  Resolve at runtime so the wrapper works from
# any working directory.

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$LockStartScript  = Join-Path $ScriptDir "..\ci\windows\gpu-lock-start.ps1"
$LockReleaseScript = Join-Path $ScriptDir "..\ci\windows\gpu-lock-release.ps1"

# Normalise the paths (resolves ".." components)
$LockStartScript   = [System.IO.Path]::GetFullPath($LockStartScript)
$LockReleaseScript = [System.IO.Path]::GetFullPath($LockReleaseScript)

if (-not (Test-Path $LockStartScript)) {
    Write-HudLog "ERROR: Cannot find gpu-lock-start.ps1 at: $LockStartScript"
    Exit-WithCode 5 "lock script missing"
}
if (-not (Test-Path $LockReleaseScript)) {
    Write-HudLog "ERROR: Cannot find gpu-lock-release.ps1 at: $LockReleaseScript"
    Exit-WithCode 5 "lock script missing"
}

# ── Pre-flight: verify the scheduled task exists ──────────────────────────────

Write-HudLog "Checking scheduled task '$TaskName' ..."
$task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
if (-not $task) {
    Write-HudLog "ERROR: Scheduled task '$TaskName' not found."
    Write-HudLog "       Register it first (see docs/ci/windows-d18-runner-setup.md §7.7):"
    Write-HudLog "       Register-ScheduledTask -TaskName '$TaskName' ..."
    Exit-WithCode 3 "scheduled task not found"
}
Write-HudLog "Task '$TaskName' found (State: $($task.State))."

# ── WhatIf / dry-run mode ─────────────────────────────────────────────────────

if ($WhatIf) {
    Write-HudLog "WhatIf: would acquire GPU lock (interactive) then run '$TaskName'."
    Write-HudLog "WhatIf: dry-run complete — no lock acquired, no task started."
    exit 0
}

# ── Phase 1: Acquire GPU lock ─────────────────────────────────────────────────

Write-HudLog "Acquiring GPU lock (interactive session) ..."
$LockAcquired = $false

try {
    & $LockStartScript -SessionType interactive -Description $Description
    $lockExitCode = $LASTEXITCODE

    switch ($lockExitCode) {
        0 {
            Write-HudLog "GPU lock acquired."
            $LockAcquired = $true
        }
        1 {
            Write-HudLog "GPU lock is held by a live CI session."
            Write-HudLog "The nightly real-decode job is running on this box."
            Write-HudLog "Wait for it to finish (check GitHub Actions) or cancel it, then retry."
            Exit-WithCode 1 "GPU lock held by CI — launch refused"
        }
        default {
            Write-HudLog "ERROR: gpu-lock-start.ps1 exited with unexpected code $lockExitCode."
            Exit-WithCode 2 "lock acquire error (exit $lockExitCode)"
        }
    }
} catch {
    Write-HudLog "ERROR: Failed to run gpu-lock-start.ps1: $_"
    Exit-WithCode 2 "lock script execution failed"
}

# ── Phase 2: Launch + monitor + release (try/finally guarantees cleanup) ──────

try {
    # Step 2a: Start the scheduled task
    Write-HudLog "Starting scheduled task '$TaskName' ..."
    try {
        Start-ScheduledTask -TaskName $TaskName
    } catch {
        Write-HudLog "ERROR: Failed to start scheduled task '$TaskName': $_"
        Exit-WithCode 3 "task start failed"
    }
    Write-HudLog "Task started. Waiting for tze_hud.exe to appear ..."

    # Step 2b: Wait for tze_hud.exe to appear in the process list
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $hudProcess = $null
    while ((Get-Date) -lt $deadline) {
        $hudProcess = Get-Process -Name "tze_hud" -ErrorAction SilentlyContinue
        if ($hudProcess) {
            Write-HudLog "tze_hud.exe is running (PID $($hudProcess.Id))."
            break
        }
        Start-Sleep -Seconds $PollIntervalSec
    }

    if (-not $hudProcess) {
        Write-HudLog "ERROR: tze_hud.exe did not appear within $TimeoutSec seconds."
        Write-HudLog "       Check the scheduled task log and ensure the task action path is correct."
        Exit-WithCode 4 "tze_hud.exe did not start in time"
    }

    # Step 2c: Monitor until the process exits
    Write-HudLog "Monitoring tze_hud.exe (PID $($hudProcess.Id)). Press Ctrl-C to stop."
    Write-HudLog "(GPU lock will be released automatically when the process exits.)"

    # Re-fetch by Id to get a stable handle; process name can be ambiguous
    $trackedPid = $hudProcess.Id
    while ($true) {
        $running = Get-Process -Id $trackedPid -ErrorAction SilentlyContinue
        if (-not $running) {
            Write-HudLog "tze_hud.exe (PID $trackedPid) has exited."
            break
        }
        Start-Sleep -Seconds $PollIntervalSec
    }

} finally {
    # Always release the GPU lock — even on Ctrl-C or error.
    # The finally block runs even when the script is interrupted by Ctrl-C
    # in PowerShell 5.1 and 7+.
    if ($LockAcquired) {
        Write-HudLog "Releasing GPU lock ..."
        try {
            & $LockReleaseScript
            $releaseCode = $LASTEXITCODE
            if ($releaseCode -eq 0) {
                Write-HudLog "GPU lock released."
            } elseif ($releaseCode -eq 1) {
                # PID mismatch — our lock was stolen or replaced (should not happen
                # in normal operation; use -Force to override).
                Write-HudLog "WARNING: Lock PID mismatch on release (exit 1). Forcing release ..."
                & $LockReleaseScript -Force
                Write-HudLog "GPU lock force-released."
            } else {
                Write-HudLog "WARNING: gpu-lock-release.ps1 exited with code $releaseCode. Lock may still be held."
            }
        } catch {
            Write-HudLog "WARNING: Exception during lock release: $_"
            Write-HudLog "         You may need to remove C:\ProgramData\tze_hud\gpu.lock manually."
        }
    }
}

exit 0
