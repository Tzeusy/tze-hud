# Runs the VerticalFlow reference-Windows pixel proof under an exclusive GPU
# controller and restores the production HUD iff it was running before takeover.

[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)]
    [string]$ProofExe,

    [Parameter(Mandatory=$true)]
    [string]$OutputDir,

    [Parameter(Mandatory=$true)]
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string]$SourceCommit,

    [Parameter(Mandatory=$false)]
    [string]$ProductionTaskName = "TzeHudOverlay",

    [Parameter(Mandatory=$false)]
    [string]$ProductionExe = "C:\tze_hud\tze_hud.exe",

    [Parameter(Mandatory=$false)]
    [string]$ReferenceHardwareTag = "TzeHouse",

    [Parameter(Mandatory=$false)]
    [switch]$AllowProductionStop
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$LockDir = "C:\ProgramData\tze_hud"
$LockFile = Join-Path $LockDir "gpu.lock"
$LockTmp = Join-Path $LockDir "gpu.lock.$PID.tmp"
$ControllerReport = Join-Path $OutputDir "vertical-flow-controller.json"
$startedAt = (Get-Date).ToUniversalTime()
$proofExit = 2
$productionStopped = $false
$productionRestored = $false
$lockAcquired = $false
$restorationError = $null
$controllerError = $null
$proofHash = $null
$productionTaskVerified = $false
$productionListenersConfirmed = $false

function Write-ProofLog([string]$Message) {
    Write-Host "[vertical-flow-proof] $Message"
}

function Get-ProductionProcess {
    @(Get-CimInstance Win32_Process -Filter "Name = 'tze_hud.exe'" -ErrorAction SilentlyContinue |
        Where-Object {
            -not [string]::IsNullOrWhiteSpace([string]$_.ExecutablePath) -and
            [string]::Equals(
                [string]$_.ExecutablePath,
                $ProductionExe,
                [System.StringComparison]::OrdinalIgnoreCase
            )
        })
}

function Assert-ProductionTaskAction {
    $task = Get-ScheduledTask -TaskName $ProductionTaskName -ErrorAction Stop
    $actions = @($task.Actions)
    if ($actions.Count -ne 1 -or
        -not [string]::Equals(
            [string]$actions[0].Execute,
            $ProductionExe,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw "scheduled task action does not execute the exact production executable: task=$ProductionTaskName expected=$ProductionExe"
    }
    $script:productionTaskVerified = $true
    return $task
}

function Get-ListeningPortsOwnedBy([int]$ProcessId) {
    @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
        Where-Object { $_.OwningProcess -eq $ProcessId } |
        Select-Object -ExpandProperty LocalPort -Unique)
}

function Read-GpuLock {
    if (-not (Test-Path -LiteralPath $LockFile)) {
        return $null
    }
    $values = @{}
    Get-Content -LiteralPath $LockFile | ForEach-Object {
        $parts = $_ -split "=", 2
        if ($parts.Count -eq 2) {
            $values[$parts[0].Trim()] = $parts[1].Trim()
        }
    }
    if (-not $values.ContainsKey("PID")) {
        throw "GPU lock is unreadable or has no PID: $LockFile"
    }
    $parsedPid = 0
    if (-not [int]::TryParse([string]$values["PID"], [ref]$parsedPid) -or $parsedPid -le 0) {
        throw "GPU lock has an invalid PID: $LockFile"
    }
    [pscustomobject]@{
        Pid = $parsedPid
        SessionType = [string]$values["SESSION_TYPE"]
        Description = [string]$values["DESCRIPTION"]
    }
}

function Wait-ForProcessExit([int[]]$ProcessIds, [int]$TimeoutSeconds = 20) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $live = @($ProcessIds | Where-Object {
            $null -ne (Get-Process -Id $_ -ErrorAction SilentlyContinue)
        })
        if ($live.Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "production HUD PIDs did not exit within ${TimeoutSeconds}s: $($live -join ',')"
}

function Wait-ForRestoredHud([int]$TimeoutSeconds = 30) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $restored = @(Get-ProductionProcess)
        if ($restored.Count -eq 1) {
            $pid = [int]$restored[0].ProcessId
            $ownedPorts = @(Get-ListeningPortsOwnedBy -ProcessId $pid)
            $requiredPorts = @(50051, 9090)
            if (($requiredPorts | Where-Object { $_ -notin $ownedPorts }).Count -eq 0) {
                return $pid
            }
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    throw "restored production HUD did not own listening ports 50051 and 9090 within ${TimeoutSeconds}s"
}

if (-not $AllowProductionStop) {
    throw "-AllowProductionStop is required after the GPU-lane controller authorizes takeover"
}
if ($ReferenceHardwareTag -ne "TzeHouse") {
    throw "reference tag must be exactly TzeHouse"
}
if (-not (Test-Path -LiteralPath $ProofExe -PathType Leaf)) {
    throw "proof executable not found: $ProofExe"
}
if (Test-Path -LiteralPath $OutputDir) {
    if (@(Get-ChildItem -LiteralPath $OutputDir -Force).Count -ne 0) {
        throw "output directory must be new or empty: $OutputDir"
    }
} else {
    New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
}

$proofHash = (Get-FileHash -LiteralPath $ProofExe -Algorithm SHA256).Hash.ToLowerInvariant()
$productionProcesses = @(Get-ProductionProcess)
if ($productionProcesses.Count -gt 1) {
    throw "multiple production HUD processes found; refusing ambiguous takeover"
}
$productionWasRunning = $productionProcesses.Count -eq 1
$productionPids = @($productionProcesses | ForEach-Object { [int]$_.ProcessId })

try {
    $null = Assert-ProductionTaskAction
    $existingLock = Read-GpuLock
    if ($null -ne $existingLock) {
        $lockProcess = Get-Process -Id $existingLock.Pid -ErrorAction SilentlyContinue
        if ($null -ne $lockProcess -and $existingLock.Pid -notin $productionPids) {
            throw "GPU lock belongs to live non-production PID $($existingLock.Pid); lane is occupied"
        }
        if ($null -ne $lockProcess -and -not $productionWasRunning) {
            throw "GPU lock is live but no matching production HUD was found"
        }
    }

    if ($productionWasRunning) {
        $productionPid = $productionPids[0]
        $ownedPorts = @(Get-ListeningPortsOwnedBy -ProcessId $productionPid)
        $requiredPorts = @(50051, 9090)
        $missingPorts = @($requiredPorts | Where-Object { $_ -notin $ownedPorts })
        if ($missingPorts.Count -ne 0) {
            throw "production HUD PID $productionPid does not own canonical listeners before takeover: missing=$($missingPorts -join ',')"
        }
        $productionListenersConfirmed = $true
        Write-ProofLog "stopping prior production HUD PID $($productionPids[0])"
        Stop-ScheduledTask -TaskName $ProductionTaskName -ErrorAction SilentlyContinue
        Stop-Process -Id $productionPids -Force
        Wait-ForProcessExit -ProcessIds $productionPids
        $productionStopped = $true
    }

    $staleLock = Read-GpuLock
    if ($null -ne $staleLock) {
        if ($null -ne (Get-Process -Id $staleLock.Pid -ErrorAction SilentlyContinue)) {
            throw "refusing to remove GPU lock while its PID is still alive"
        }
        Write-ProofLog "removing stale GPU lock owned by dead PID $($staleLock.Pid)"
        Remove-Item -LiteralPath $LockFile -Force
    }

    New-Item -ItemType Directory -Force -Path $LockDir | Out-Null
    if (Test-Path -LiteralPath $LockFile) {
        throw "GPU lock appeared during takeover; refusing to race another lane"
    }
    $lockBody = @(
        "SESSION_TYPE=interactive",
        "PID=$PID",
        "STARTED_AT=$($startedAt.ToString('o'))",
        "DESCRIPTION=hud-yglp4 vertical-flow reference-Windows pixel proof"
    )
    $lockBytes = [System.Text.UTF8Encoding]::new($false).GetBytes(
        (($lockBody -join "`r`n") + "`r`n")
    )
    $lockStream = [System.IO.File]::Open(
        $LockTmp,
        [System.IO.FileMode]::CreateNew,
        [System.IO.FileAccess]::Write,
        [System.IO.FileShare]::None
    )
    try {
        $lockStream.Write($lockBytes, 0, $lockBytes.Length)
        $lockStream.Flush($true)
    } finally {
        $lockStream.Dispose()
    }
    Move-Item -LiteralPath $LockTmp -Destination $LockFile
    $lockAcquired = $true

    $gpu = @(Get-CimInstance Win32_VideoController | Where-Object {
        $null -ne $_.CurrentHorizontalResolution -and
        $null -ne $_.CurrentVerticalResolution
    } | Sort-Object AdapterRAM -Descending | Select-Object -First 1)
    if ($gpu.Count -ne 1) {
        throw "unable to identify one active Windows video controller"
    }
    $os = Get-CimInstance Win32_OperatingSystem
    $osLabel = "$($os.Caption) $($os.Version) build $($os.BuildNumber)"
    $proofArgs = @(
        "--output", $OutputDir,
        "--reference-hardware-tag", $ReferenceHardwareTag,
        "--reference-hostname", [System.Net.Dns]::GetHostName(),
        "--reference-gpu", [string]$gpu[0].Name,
        "--reference-gpu-driver", [string]$gpu[0].DriverVersion,
        "--reference-os", $osLabel,
        "--display-width", [string]$gpu[0].CurrentHorizontalResolution,
        "--display-height", [string]$gpu[0].CurrentVerticalResolution
    )
    Write-ProofLog "running proof sha256=$proofHash source=$SourceCommit"
    & $ProofExe @proofArgs
    $proofExit = $LASTEXITCODE
    if ($null -eq $proofExit) {
        $proofExit = 2
    }
} catch {
    $controllerError = $_.Exception.Message
    Write-ProofLog "controller failed: $controllerError"
    $proofExit = 2
} finally {
    try {
        if ($lockAcquired) {
            $ownedLock = Read-GpuLock
            if ($null -eq $ownedLock -or $ownedLock.Pid -ne $PID) {
                throw "proof controller no longer owns the GPU lock; refusing foreign-lock removal"
            }
            Remove-Item -LiteralPath $LockFile -Force
            $lockAcquired = $false
        }
        Remove-Item -LiteralPath $LockTmp -Force -ErrorAction SilentlyContinue

        if ($productionWasRunning) {
            $currentProduction = @(Get-ProductionProcess)
            if ($currentProduction.Count -gt 1) {
                throw "multiple production HUD processes found during restoration"
            }
            if ($currentProduction.Count -eq 0) {
                Write-ProofLog "restoring prior production HUD through $ProductionTaskName"
                Start-ScheduledTask -TaskName $ProductionTaskName
            } else {
                Write-ProofLog "production HUD remained present after the stop attempt; verifying it"
            }
            $restoredPid = Wait-ForRestoredHud
            Write-ProofLog "restored production HUD PID $restoredPid owns ports 50051 and 9090"
            $productionRestored = $true
        } else {
            $productionRestored = $true
        }
    } catch {
        $restorationError = $_.Exception.Message
        Write-ProofLog "restoration failed: $restorationError"
    }
}

$controllerEvidence = [ordered]@{
    schema_version = 1
    artifact = "vertical-flow-reference-windows-controller"
    source_commit = $SourceCommit
    proof_exe_sha256 = $proofHash
    started_at = $startedAt.ToString("o")
    completed_at = (Get-Date).ToUniversalTime().ToString("o")
    production_task = $ProductionTaskName
    production_task_action_verified = $productionTaskVerified
    production_was_running = $productionWasRunning
    production_listener_ownership_confirmed = $productionListenersConfirmed
    production_stop_confirmed = $productionStopped
    original_production_pids = $productionPids
    production_restored = $productionRestored
    proof_exit_code = $proofExit
    controller_error = $controllerError
    restoration_error = $restorationError
}
try {
    $controllerEvidence | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $ControllerReport -Encoding UTF8
} catch {
    Write-ProofLog "unable to write controller evidence: $($_.Exception.Message)"
    exit 3
}

if ($null -ne $restorationError) {
    exit 3
}
exit $proofExit
