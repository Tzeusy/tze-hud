param(
    [string]$TaskName = 'TzeHudGog648Media',
    [int]$GrpcPort = 50052,
    [int]$McpPort = 9092
)

$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'SilentlyContinue'

$result = [ordered]@{
    started_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    task = $TaskName
    production_task = 'TzeHudOverlay'
    grpc_port = $GrpcPort
    mcp_port = $McpPort
    steps = @()
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

$isolatedPids = @(Get-NetTCPConnection -State Listen -LocalPort $GrpcPort, $McpPort -ErrorAction SilentlyContinue |
    Select-Object -ExpandProperty OwningProcess -Unique)
foreach ($pid in $isolatedPids) {
    Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue
}
Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
$isolatedClosed = Wait-Port $GrpcPort $false 10
Add-Step 'stop-isolated-media-hud' 'ok' "grpc_${GrpcPort}_closed=$isolatedClosed"

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
