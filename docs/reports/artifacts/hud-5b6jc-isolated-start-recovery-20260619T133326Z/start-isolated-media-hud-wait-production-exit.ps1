param(
    [string]$TaskName = 'TzeHud5b6jcMedia',
    [string]$ConfigPath = 'C:\tze_hud\hud-gog64_8\windows-media-ingress-enabled.toml',
    [int]$GrpcPort = 50052,
    [int]$McpPort = 9092
)

$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'SilentlyContinue'

$result = [ordered]@{
    started_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    task = $TaskName
    config = $ConfigPath
    grpc_port = $GrpcPort
    mcp_port = $McpPort
    production_task = 'TzeHudOverlay'
    steps = @()
    isolated_bind = $false
    restored = $false
}

function Add-Step($name, $status, $detail) {
    $script:result.steps += [ordered]@{
        at_utc = (Get-Date).ToUniversalTime().ToString('o')
        name = $name
        status = $status
        detail = $detail
    }
}

function Get-Listeners($ports) {
    Get-NetTCPConnection -State Listen -LocalPort $ports -ErrorAction SilentlyContinue |
        Select-Object LocalAddress, LocalPort, OwningProcess
}

function Get-LockLines() {
    $lockPath = 'C:\ProgramData\tze_hud\gpu.lock'
    if (Test-Path $lockPath) {
        return @(Get-Content -Path $lockPath | ForEach-Object {
            $line = [string]$_
            if ($line -match '^DESCRIPTION=') {
                'DESCRIPTION=<redacted-description>'
            } else {
                $line
            }
        })
    }
    return @('gpu_lock=absent')
}

function Clear-StaleGpuLockForPid($processId, $label) {
    $lockPath = 'C:\ProgramData\tze_hud\gpu.lock'
    if (-not $processId) {
        Add-Step $label 'skipped' 'no_pid'
        return $false
    }
    if (Get-Process -Id $processId -ErrorAction SilentlyContinue) {
        Add-Step $label 'skipped' "pid_${processId}_still_live=True"
        return $false
    }
    if (-not (Test-Path $lockPath)) {
        Add-Step $label 'ok' 'gpu_lock=absent'
        return $false
    }
    $lockLines = @(Get-Content -Path $lockPath -ErrorAction SilentlyContinue)
    $lockPidLine = $lockLines | Where-Object { $_ -match '^PID=' } | Select-Object -First 1
    $lockPid = $null
    if ($lockPidLine -match '^PID=(\d+)$') {
        $lockPid = [int]$Matches[1]
    }
    if ($lockPid -eq $processId) {
        Remove-Item -Path $lockPath -Force -ErrorAction SilentlyContinue
        Add-Step $label 'ok' "removed_stale_gpu_lock_pid=$processId"
        return $true
    }
    Add-Step $label 'skipped' "gpu_lock_pid_mismatch=$lockPid"
    return $false
}

function Wait-Port($port, $expectOpen, $seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    do {
        $conn = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
        if ($expectOpen -and $conn) {
            return $true
        }
        if ((-not $expectOpen) -and (-not $conn)) {
            return $true
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Wait-ProcessGone($processId, $seconds) {
    if (-not $processId) {
        return $true
    }
    $deadline = (Get-Date).AddSeconds($seconds)
    do {
        $proc = Get-Process -Id $processId -ErrorAction SilentlyContinue
        if (-not $proc) {
            return $true
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    return $false
}

try {
    $overlayXml = (& schtasks /Query /TN TzeHudOverlay /XML 2>$null) -join "`n"
    $pskMatch = [regex]::Match($overlayXml, '--psk\s+([^\s"<]+)')
    if (-not $pskMatch.Success) {
        throw 'Could not recover non-default PSK from TzeHudOverlay task XML'
    }
    Add-Step 'recover-psk' 'ok' 'Recovered from task XML; value intentionally omitted.'

    $preListeners = Get-Listeners @(50051, 9090, $GrpcPort, $McpPort)
    $result.pre_listeners = $preListeners
    $prodPid = ($preListeners | Where-Object { $_.LocalPort -eq 50051 } | Select-Object -First 1).OwningProcess
    if (-not $prodPid) {
        throw 'Production gRPC listener 50051 was not present before interruption'
    }
    if ($preListeners | Where-Object { $_.LocalPort -eq $GrpcPort }) {
        throw "Alternate gRPC port $GrpcPort was already occupied before interruption"
    }
    if ($preListeners | Where-Object { $_.LocalPort -eq $McpPort }) {
        throw "Alternate MCP port $McpPort was already occupied before interruption"
    }
    $result.production_pid_before = $prodPid
    $result.gpu_lock_before = Get-LockLines
    Add-Step 'baseline-check' 'ok' "production_pid=$prodPid; alternate ports $GrpcPort/$McpPort available"

    Stop-ScheduledTask -TaskName TzeHudOverlay -ErrorAction SilentlyContinue
    Stop-Process -Id $prodPid -Force -ErrorAction SilentlyContinue
    $closed = Wait-Port 50051 $false 15
    if (-not $closed) {
        Add-Step 'stop-production' 'timeout' '50051_closed=False'
        throw 'Timed out waiting for production gRPC port 50051 to close'
    }
    Add-Step 'stop-production' 'ok' '50051_closed=True'

    $prodGone = Wait-ProcessGone $prodPid 20
    $result.production_process_gone = [bool]$prodGone
    $result.gpu_lock_after_production_stop = Get-LockLines
    if (-not $prodGone) {
        Add-Step 'wait-production-exit' 'timeout' "pid_${prodPid}_gone=False"
        throw "Timed out waiting for production PID $prodPid to exit"
    }
    Add-Step 'wait-production-exit' 'ok' "pid_${prodPid}_gone=True"
    $result.removed_stale_production_gpu_lock = Clear-StaleGpuLockForPid $prodPid 'clear-stale-production-gpu-lock'
    $result.gpu_lock_after_production_lock_cleanup = Get-LockLines

    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    $psk = $pskMatch.Groups[1].Value
    $mediaArgs = "--config $ConfigPath --window-mode overlay --bind-all-interfaces --grpc-port $GrpcPort --mcp-port $McpPort --psk " + $psk
    $action = New-ScheduledTaskAction -Execute 'C:\tze_hud\tze_hud.exe' -Argument $mediaArgs -WorkingDirectory 'C:\tze_hud'
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 45)
    Register-ScheduledTask -TaskName $TaskName -Action $action -Settings $settings -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName
    $bound = Wait-Port $GrpcPort $true 25
    $result.isolated_bind = [bool]$bound
    $result.isolated_listeners = Get-Listeners @($GrpcPort, $McpPort)
    $result.isolated_process = Get-Process tze_hud -ErrorAction SilentlyContinue |
        Where-Object { $_.Id -in @($result.isolated_listeners.OwningProcess) } |
        Select-Object Id, ProcessName, Path
    $result.gpu_lock_during_isolated = Get-LockLines
    if ($bound) {
        Add-Step 'start-isolated-media-hud' 'ok' "grpc_${GrpcPort}_bound=True"
    } else {
        Add-Step 'start-isolated-media-hud' 'failed' "grpc_${GrpcPort}_bound=False"
        throw "Timed out waiting for isolated gRPC port $GrpcPort"
    }
} catch {
    $result.error = [string]$_
    Add-Step 'exception' 'error' ([string]$_)
} finally {
    $isolatedPids = @(Get-NetTCPConnection -State Listen -LocalPort $GrpcPort, $McpPort -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty OwningProcess -Unique)
    foreach ($isolatedPid in $isolatedPids) {
        Stop-Process -Id $isolatedPid -Force -ErrorAction SilentlyContinue
    }
    Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    $isolatedClosed = Wait-Port $GrpcPort $false 10
    Add-Step 'stop-isolated-media-hud' 'ok' "grpc_${GrpcPort}_closed=$isolatedClosed"
    $isolatedGone = $true
    foreach ($isolatedPid in $isolatedPids) {
        if (-not (Wait-ProcessGone $isolatedPid 20)) {
            $isolatedGone = $false
        }
    }
    $result.isolated_processes_gone = [bool]$isolatedGone
    $result.gpu_lock_after_isolated_stop = Get-LockLines
    if ($isolatedGone) {
        Add-Step 'wait-isolated-exit' 'ok' 'isolated_pids_gone=True'
    } else {
        Add-Step 'wait-isolated-exit' 'timeout' 'isolated_pids_gone=False'
    }
    $removedIsolatedLock = $false
    foreach ($isolatedPid in $isolatedPids) {
        if (Clear-StaleGpuLockForPid $isolatedPid 'clear-stale-isolated-gpu-lock') {
            $removedIsolatedLock = $true
        }
    }
    $result.removed_stale_isolated_gpu_lock = [bool]$removedIsolatedLock
    $result.gpu_lock_after_isolated_lock_cleanup = Get-LockLines

    Start-ScheduledTask -TaskName TzeHudOverlay -ErrorAction SilentlyContinue
    $restoredGrpc = Wait-Port 50051 $true 25
    $restoredMcp = Wait-Port 9090 $true 15
    $result.restored = [bool]($restoredGrpc -and $restoredMcp)
    $result.restored_listeners = Get-Listeners @(50051, 9090, $GrpcPort, $McpPort)
    $result.restored_processes = Get-Process tze_hud -ErrorAction SilentlyContinue |
        Select-Object Id, ProcessName, Path
    $result.gpu_lock_after_restore = Get-LockLines
    if ($result.restored) {
        Add-Step 'restore-production' 'ok' "grpc_50051=$restoredGrpc; mcp_9090=$restoredMcp"
    } else {
        Add-Step 'restore-production' 'failed' "grpc_50051=$restoredGrpc; mcp_9090=$restoredMcp"
    }
    $result.finished_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    $result | ConvertTo-Json -Depth 8
}
