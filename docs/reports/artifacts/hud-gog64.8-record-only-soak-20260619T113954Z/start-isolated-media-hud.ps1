param(
    [string]$TaskName = 'TzeHudGog648Media',
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
    $result.finished_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    $result | ConvertTo-Json -Depth 8
}
