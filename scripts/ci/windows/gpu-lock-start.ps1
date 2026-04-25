# scripts/ci/windows/gpu-lock-start.ps1
#
# Acquires the tzehouse-windows GPU lock for a session (CI or interactive).
#
# Usage:
#   gpu-lock-start.ps1 -SessionType ci       -Description "real-decode nightly run"
#   gpu-lock-start.ps1 -SessionType interactive -Description "user-test overlay session"
#
# Exit codes:
#   0  — lock acquired successfully
#   1  — lock is held by a live process (conflict); caller decides how to proceed
#   2  — unexpected error (lock directory not writable, etc.)
#
# See docs/design/tzehouse-windows-gpu-scheduling.md for full policy.

[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)]
    [ValidateSet("ci", "interactive")]
    [string]$SessionType,

    [Parameter(Mandatory=$false)]
    [string]$Description = ""
)

$ErrorActionPreference = "Stop"
$LockDir  = "C:\ProgramData\tze_hud"
$LockFile = "$LockDir\gpu.lock"
$TmpFile  = "$LockDir\gpu.lock.tmp"
$MyPid    = $PID

function Write-LockLog([string]$msg) {
    Write-Host "[gpu-lock] $msg"
}

# Ensure the lock directory exists
if (-not (Test-Path $LockDir)) {
    Write-LockLog "Creating lock directory: $LockDir"
    try {
        New-Item -ItemType Directory -Force -Path $LockDir | Out-Null
    } catch {
        Write-LockLog "ERROR: Cannot create lock directory: $_"
        exit 2
    }
}

Write-LockLog "Checking $LockFile ..."

# Check for an existing lock
if (Test-Path $LockFile) {
    $lines = @{}
    try {
        Get-Content $LockFile | ForEach-Object {
            $parts = $_ -split "=", 2
            if ($parts.Count -eq 2) {
                $lines[$parts[0].Trim()] = $parts[1].Trim()
            }
        }
    } catch {
        Write-LockLog "WARNING: Lock file unreadable or corrupt. Treating as absent."
        Remove-Item -Force $LockFile -ErrorAction SilentlyContinue
        $lines = @{}
    }

    if ($lines.Count -gt 0) {
        $existingPid = [int]($lines["PID"] ?? 0)
        $existingType = $lines["SESSION_TYPE"] ?? "unknown"
        $existingDesc = $lines["DESCRIPTION"] ?? ""
        $existingTime = $lines["STARTED_AT"] ?? "unknown"

        Write-LockLog "Lock file found: SESSION_TYPE=$existingType PID=$existingPid STARTED_AT=$existingTime"
        if ($existingDesc) {
            Write-LockLog "Description: $existingDesc"
        }

        # Check if the holding process is still alive
        if ($existingPid -gt 0) {
            $proc = Get-Process -Id $existingPid -ErrorAction SilentlyContinue
            if ($proc) {
                Write-LockLog "Process $existingPid is alive ($($proc.ProcessName)). GPU is in use."
                Write-LockLog "Cannot acquire lock. Exiting with code 1."
                exit 1
            } else {
                Write-LockLog "Process $existingPid is no longer running. Lock is stale. Removing."
                Remove-Item -Force $LockFile -ErrorAction SilentlyContinue
            }
        } else {
            Write-LockLog "WARNING: Lock file has no PID. Treating as stale. Removing."
            Remove-Item -Force $LockFile -ErrorAction SilentlyContinue
        }
    }
}

# Acquire: write to tmp then rename (best-effort atomic on NTFS)
$startedAt = (Get-Date).ToUniversalTime().ToString("o")
$content = @"
SESSION_TYPE=$SessionType
PID=$MyPid
STARTED_AT=$startedAt
DESCRIPTION=$Description
"@

try {
    Set-Content -Path $TmpFile -Value $content -Encoding UTF8
    Move-Item -Path $TmpFile -Destination $LockFile -Force
} catch {
    Write-LockLog "ERROR: Failed to write lock file: $_"
    Remove-Item -Force $TmpFile -ErrorAction SilentlyContinue
    exit 2
}

Write-LockLog "Lock acquired (SESSION_TYPE=$SessionType PID=$MyPid STARTED_AT=$startedAt)."
exit 0
