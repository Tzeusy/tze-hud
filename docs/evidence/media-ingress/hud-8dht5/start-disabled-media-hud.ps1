param(
    [Parameter(Mandatory = $true)][string]$Psk,
    [string]$TaskName   = 'TzeHud8dht5Media',
    [string]$ConfigPath = 'C:\tze_hud\hud-8dht5\windows-media-ingress-operator-disabled.toml',
    [int]$GrpcPort      = 50052,
    [int]$McpPort       = 9092,
    [string]$ProdTask   = 'TzeHudOverlay'
)

$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'SilentlyContinue'

$result = [ordered]@{
    started_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    task           = $TaskName
    config         = $ConfigPath
    grpc_port      = $GrpcPort
    mcp_port       = $McpPort
    steps          = @()
    isolated_bind  = $false
}
function Add-Step($name, $status, $detail) {
    $script:result.steps += [ordered]@{ name = $name; status = $status; detail = $detail }
}
function Listeners($ports) {
    Get-NetTCPConnection -State Listen -LocalPort $ports -ErrorAction SilentlyContinue |
        Select-Object LocalAddress, LocalPort, OwningProcess
}
function Wait-Port($port, $expectOpen, $seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    do {
        $conn = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
        if ($expectOpen -and $conn) { return $true }
        if ((-not $expectOpen) -and (-not $conn)) { return $true }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    return $false
}

try {
    $result.pre_listeners = Listeners @(50051, 9090, $GrpcPort, $McpPort)

    # Stop production overlay to release the GPU lock + ports.
    Stop-ScheduledTask -TaskName $ProdTask -ErrorAction SilentlyContinue
    Get-Process tze_hud -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    $closed = Wait-Port 50051 $false 20
    Add-Step 'stop-production' $(if ($closed) { 'ok' } else { 'timeout' }) "50051_closed=$closed"
    if (-not $closed) { throw 'production gRPC 50051 did not close' }

    # Register + start the isolated operator-disabled media HUD on alternate ports.
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    $mediaArgs = "--config $ConfigPath --window-mode overlay --bind-all-interfaces --grpc-port $GrpcPort --mcp-port $McpPort --psk $Psk"
    $action   = New-ScheduledTaskAction -Execute 'C:\tze_hud\tze_hud.exe' -Argument $mediaArgs -WorkingDirectory 'C:\tze_hud'
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 30)
    Register-ScheduledTask -TaskName $TaskName -Action $action -Settings $settings -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName

    $bound = Wait-Port $GrpcPort $true 30
    $result.isolated_bind = [bool]$bound
    $result.isolated_listeners = Listeners @($GrpcPort, $McpPort)
    Add-Step 'start-isolated-media-hud' $(if ($bound) { 'ok' } else { 'failed' }) "grpc_${GrpcPort}_bound=$bound"
    if (-not $bound) { throw "isolated gRPC $GrpcPort did not bind" }
} catch {
    $result.error = [string]$_
    Add-Step 'exception' 'error' ([string]$_)
} finally {
    $result.finished_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    $result | ConvertTo-Json -Depth 8
}
