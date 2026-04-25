# scripts/ci/windows/gpu-lock-release.ps1
#
# Releases the tzehouse-windows GPU lock, verifying PID ownership.
#
# Usage:
#   gpu-lock-release.ps1
#   gpu-lock-release.ps1 -Force   # release even if PID does not match (use with caution)
#
# Exit codes:
#   0  — lock released (or was already absent)
#   1  — lock is held by a different PID; use -Force to override
#   2  — unexpected error
#
# See docs/design/tzehouse-windows-gpu-scheduling.md for full policy.

[CmdletBinding()]
param(
    [Parameter(Mandatory=$false)]
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$LockDir  = "C:\ProgramData\tze_hud"
$LockFile = "$LockDir\gpu.lock"
$MyPid    = $PID

function Write-LockLog([string]$msg) {
    Write-Host "[gpu-lock] $msg"
}

Write-LockLog "Releasing $LockFile ..."

if (-not (Test-Path $LockFile)) {
    Write-LockLog "Lock file does not exist. Nothing to release."
    exit 0
}

# Read the lock file
$lines = @{}
try {
    Get-Content $LockFile | ForEach-Object {
        $parts = $_ -split "=", 2
        if ($parts.Count -eq 2) {
            $lines[$parts[0].Trim()] = $parts[1].Trim()
        }
    }
} catch {
    Write-LockLog "WARNING: Lock file unreadable. Removing anyway."
    Remove-Item -Force $LockFile -ErrorAction SilentlyContinue
    exit 0
}

$lockPid = [int]($lines["PID"] ?? 0)
$lockType = $lines["SESSION_TYPE"] ?? "unknown"

Write-LockLog "Lock held by: SESSION_TYPE=$lockType PID=$lockPid"

if ($lockPid -ne $MyPid -and $lockPid -gt 0) {
    if (-not $Force) {
        Write-LockLog "WARNING: Lock is held by PID $lockPid, not by this process ($MyPid)."
        Write-LockLog "To release anyway, use: gpu-lock-release.ps1 -Force"
        exit 1
    } else {
        Write-LockLog "WARNING: -Force specified. Releasing lock held by PID $lockPid."
    }
}

try {
    Remove-Item -Force $LockFile
    Write-LockLog "Lock released."
    exit 0
} catch {
    Write-LockLog "ERROR: Failed to remove lock file: $_"
    exit 2
}
