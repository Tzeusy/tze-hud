# Runs the real windowed compositor in fullscreen and transparent-overlay mode,
# then emits a combined JSON report with the overlay composite delta.

[CmdletBinding()]
param(
    [Parameter(Mandatory=$false)]
    [string]$ExePath = "",

    [Parameter(Mandatory=$false)]
    [string]$OutputDir = "artifacts/windowed-fullscreen-overlay-perf",

    [Parameter(Mandatory=$false)]
    [int]$Width = 1920,

    [Parameter(Mandatory=$false)]
    [int]$Height = 1080,

    [Parameter(Mandatory=$false)]
    [int]$Frames = 600,

    [Parameter(Mandatory=$false)]
    [int]$WarmupFrames = 120,

    [Parameter(Mandatory=$false)]
    [int]$TargetDeltaUs = 500,

    [Parameter(Mandatory=$false)]
    [switch]$FailOnBudget
)

$ErrorActionPreference = "Stop"

if (-not $ExePath) {
    $ExePath = Join-Path (Get-Location) "target\release\tze_hud.exe"
}

if (-not (Test-Path $ExePath)) {
    throw "tze_hud executable not found: $ExePath"
}

if ($Frames -le 0) {
    throw "-Frames must be greater than zero"
}

if ($WarmupFrames -lt 0) {
    throw "-WarmupFrames must be zero or greater"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

$configPath = Join-Path $OutputDir "windowed-benchmark.toml"
$fullscreenPath = Join-Path $OutputDir "fullscreen.json"
$overlayPath = Join-Path $OutputDir "overlay.json"
$reportPath = Join-Path $OutputDir "windowed_fullscreen_vs_overlay_report.json"
$logDir = Join-Path $OutputDir "logs"
$psk = "windowed-benchmark-$([Guid]::NewGuid().ToString('N'))"

$config = @"
[runtime]
profile = "full-display"

[[tabs]]
name = "Benchmark"
"@
Set-Content -Path $configPath -Value $config -Encoding UTF8
New-Item -ItemType Directory -Force -Path $logDir | Out-Null

function Invoke-WindowedBenchmarkMode {
    param(
        [Parameter(Mandatory=$true)]
        [ValidateSet("fullscreen", "overlay")]
        [string]$Mode,

        [Parameter(Mandatory=$true)]
        [string]$EmitPath
    )

    Write-Host "[windowed-perf] Running $Mode benchmark..."
    $stdoutPath = Join-Path $logDir "$Mode.stdout.log"
    $stderrPath = Join-Path $logDir "$Mode.stderr.log"
    $args = @(
        "--config", $configPath,
        "--window-mode", $Mode,
        "--width", "$Width",
        "--height", "$Height",
        "--grpc-port", "0",
        "--mcp-port", "0",
        "--psk", $psk,
        "--benchmark-emit", $EmitPath,
        "--benchmark-frames", "$Frames",
        "--benchmark-warmup-frames", "$WarmupFrames"
    )

    $process = Start-Process `
        -FilePath $ExePath `
        -ArgumentList $args `
        -Wait `
        -PassThru `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath

    if ($null -eq $process.ExitCode) {
        throw "$Mode benchmark exited without an exit code; stdout=$stdoutPath stderr=$stderrPath"
    }

    if ($process.ExitCode -ne 0) {
        throw "$Mode benchmark failed with exit code $($process.ExitCode); stdout=$stdoutPath stderr=$stderrPath"
    }

    if (-not (Test-Path $EmitPath)) {
        throw "$Mode benchmark did not produce expected artifact: $EmitPath; stdout=$stdoutPath stderr=$stderrPath"
    }

    return Get-Content -Path $EmitPath -Raw | ConvertFrom-Json
}

$fullscreen = Invoke-WindowedBenchmarkMode -Mode "fullscreen" -EmitPath $fullscreenPath
$overlay = Invoke-WindowedBenchmarkMode -Mode "overlay" -EmitPath $overlayPath

$fullscreenP50 = [int64]$fullscreen.frame_time.p50_us
$fullscreenP99 = [int64]$fullscreen.frame_time.p99_us
$fullscreenP999 = [int64]$fullscreen.frame_time.p99_9_us
$overlayP50 = [int64]$overlay.frame_time.p50_us
$overlayP99 = [int64]$overlay.frame_time.p99_us
$overlayP999 = [int64]$overlay.frame_time.p99_9_us
$deltaP50 = $overlayP50 - $fullscreenP50
$deltaP99 = $overlayP99 - $fullscreenP99
$deltaP999 = $overlayP999 - $fullscreenP999
$passesTarget = $deltaP99 -le $TargetDeltaUs

$report = [ordered]@{
    schema = "tze_hud.windowed_fullscreen_vs_overlay_report.v1"
    generated_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    target = [ordered]@{
        overlay_composite_delta_p99_us = $TargetDeltaUs
        pass = $passesTarget
    }
    command = [ordered]@{
        exe_path = $ExePath
        width = $Width
        height = $Height
        frames = $Frames
        warmup_frames = $WarmupFrames
    }
    fullscreen = $fullscreen
    overlay = $overlay
    composite_delta = [ordered]@{
        p50_us = $deltaP50
        p99_us = $deltaP99
        p99_9_us = $deltaP999
    }
}

$report | ConvertTo-Json -Depth 32 | Set-Content -Path $reportPath -Encoding UTF8

Write-Host "[windowed-perf] Report: $reportPath"
Write-Host "[windowed-perf] fullscreen p50/p99/p99.9: $fullscreenP50 / $fullscreenP99 / $fullscreenP999 us"
Write-Host "[windowed-perf] overlay    p50/p99/p99.9: $overlayP50 / $overlayP99 / $overlayP999 us"
Write-Host "[windowed-perf] delta      p50/p99/p99.9: $deltaP50 / $deltaP99 / $deltaP999 us"
Write-Host "[windowed-perf] target p99 delta <= $TargetDeltaUs us: $passesTarget"

if ($FailOnBudget -and -not $passesTarget) {
    exit 2
}
