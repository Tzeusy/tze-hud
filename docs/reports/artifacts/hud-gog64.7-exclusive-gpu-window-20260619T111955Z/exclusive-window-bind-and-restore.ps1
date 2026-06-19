$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'SilentlyContinue'

$result = [ordered]@{
    started_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    task = 'TzeHudGog647Media'
    config = 'C:\tze_hud\hud-gog64_7\windows-media-ingress.toml'
    grpc_port = 50052
    mcp_port = 9092
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

try {
    $overlayXml = (& schtasks /Query /TN TzeHudOverlay /XML 2>$null) -join "`n"
    $pskMatch = [regex]::Match($overlayXml, '--psk\s+([^\s"<]+)')
    if (-not $pskMatch.Success) {
        throw 'Could not recover non-default PSK from TzeHudOverlay task XML'
    }
    Add-Step 'recover-psk' 'ok' 'Recovered from task XML; value intentionally omitted.'

    $preListeners = Get-Listeners @(50051, 9090, 50052, 9091, 9092)
    $result.pre_listeners = $preListeners
    $prodPid = ($preListeners | Where-Object { $_.LocalPort -eq 50051 } | Select-Object -First 1).OwningProcess
    if (-not $prodPid) {
        throw 'Production gRPC listener 50051 was not present before interruption'
    }
    if ($preListeners | Where-Object { $_.LocalPort -eq 50052 }) {
        throw 'Alternate gRPC port 50052 was already occupied before interruption'
    }
    if ($preListeners | Where-Object { $_.LocalPort -eq 9092 }) {
        throw 'Alternate MCP port 9092 was already occupied before interruption'
    }
    $result.production_pid_before = $prodPid
    $result.gpu_lock_before = Get-LockLines
    Add-Step 'baseline-check' 'ok' "production_pid=$prodPid; alternate ports 50052/9092 available"

    Stop-ScheduledTask -TaskName TzeHudOverlay -ErrorAction SilentlyContinue
    Stop-Process -Id $prodPid -Force -ErrorAction SilentlyContinue
    $closed = Wait-Port 50051 $false 15
    if ($closed) {
        Add-Step 'stop-production' 'ok' '50051_closed=True'
    } else {
        Add-Step 'stop-production' 'timeout' '50051_closed=False'
        throw 'Timed out waiting for production gRPC port 50051 to close'
    }

    Unregister-ScheduledTask -TaskName TzeHudGog647Media -Confirm:$false -ErrorAction SilentlyContinue
    $psk = $pskMatch.Groups[1].Value
    $mediaArgs = '--config C:\tze_hud\hud-gog64_7\windows-media-ingress.toml --window-mode overlay --bind-all-interfaces --grpc-port 50052 --mcp-port 9092 --psk ' + $psk
    $action = New-ScheduledTaskAction -Execute 'C:\tze_hud\tze_hud.exe' -Argument $mediaArgs -WorkingDirectory 'C:\tze_hud'
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 5)
    Register-ScheduledTask -TaskName TzeHudGog647Media -Action $action -Settings $settings -Force | Out-Null
    Start-ScheduledTask -TaskName TzeHudGog647Media
    $bound = Wait-Port 50052 $true 20
    $result.isolated_bind = [bool]$bound
    $result.isolated_listeners = Get-Listeners @(50052, 9092)
    $result.isolated_process = Get-Process tze_hud -ErrorAction SilentlyContinue |
        Where-Object { $_.Id -in @($result.isolated_listeners.OwningProcess) } |
        Select-Object Id, ProcessName, Path
    $result.gpu_lock_during_isolated = Get-LockLines
    if ($bound) {
        Add-Step 'start-isolated-media-hud' 'ok' 'grpc_50052_bound=True'
    } else {
        Add-Step 'start-isolated-media-hud' 'failed' 'grpc_50052_bound=False'
    }
} catch {
    $result.error = [string]$_
    Add-Step 'exception' 'error' ([string]$_)
} finally {
    $isolatedPids = @(Get-NetTCPConnection -State Listen -LocalPort 50052, 9092 -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty OwningProcess -Unique)
    foreach ($pid in $isolatedPids) {
        Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue
    }
    Stop-ScheduledTask -TaskName TzeHudGog647Media -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName TzeHudGog647Media -Confirm:$false -ErrorAction SilentlyContinue
    Wait-Port 50052 $false 10 | Out-Null

    Start-ScheduledTask -TaskName TzeHudOverlay -ErrorAction SilentlyContinue
    $restoredGrpc = Wait-Port 50051 $true 25
    $restoredMcp = Wait-Port 9090 $true 10
    $result.restored = [bool]($restoredGrpc -and $restoredMcp)
    $result.restored_listeners = Get-Listeners @(50051, 9090, 50052, 9092)
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
