param(
    [int]$GrpcPort = 50052,
    [int]$Samples = 21,
    [int]$IntervalSeconds = 30
)

$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'SilentlyContinue'

$result = [ordered]@{
    started_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    grpc_port = $GrpcPort
    samples_requested = $Samples
    interval_seconds = $IntervalSeconds
    logical_processors = (Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors
    samples = @()
    errors = @()
}

function Read-Gpu3dUtilization {
    try {
        $gpuSamples = (Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples |
            Where-Object { $_.InstanceName -like '*engtype_3D*' }
        if ($gpuSamples) {
            return ($gpuSamples | Measure-Object CookedValue -Sum).Sum
        }
    } catch {
        $script:result.errors += "gpu counter unavailable: $($_.Exception.Message)"
    }
    return $null
}

function Read-GpuLockLines {
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

$start = Get-Date
for ($i = 0; $i -lt $Samples; $i++) {
    $listener = Get-NetTCPConnection -State Listen -LocalPort $GrpcPort -ErrorAction SilentlyContinue |
        Select-Object -First 1
    $owner = if ($listener) { $listener.OwningProcess } else { $null }
    $proc = if ($owner) { Get-Process -Id $owner -ErrorAction SilentlyContinue } else { $null }
    $result.samples += [ordered]@{
        at_utc = (Get-Date).ToUniversalTime().ToString('o')
        elapsed_s = [math]::Round(((Get-Date) - $start).TotalSeconds, 3)
        listener_pid = $owner
        cpu_seconds = if ($proc) { $proc.CPU } else { $null }
        working_set_bytes = if ($proc) { $proc.WorkingSet64 } else { $null }
        private_memory_bytes = if ($proc) { $proc.PrivateMemorySize64 } else { $null }
        gpu_3d_utilization_pct_sum = Read-Gpu3dUtilization
        gpu_lock = Read-GpuLockLines
    }
    if ($i -lt ($Samples - 1)) {
        Start-Sleep -Seconds $IntervalSeconds
    }
}

$result.finished_at_utc = (Get-Date).ToUniversalTime().ToString('o')
$result | ConvertTo-Json -Depth 8
