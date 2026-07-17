# Run the canonical windowed overlay through the bounded WARP quiescent gate.
#
# The runtime itself records the observed GetProcessAffinityMask result and
# selected adapter in its artifact. This harness sets the two-CPU constraint,
# waits for that real runtime to terminate, then makes the Python checker the
# final fail-closed authority.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$ExePath = "target\release\tze_hud.exe",

    [Parameter(Mandatory = $false)]
    [string]$OutputDir = "test_results\windows-performance-budget\quiescent-efficiency"
)

$ErrorActionPreference = "Stop"

if ($env:OS -ne "Windows_NT") {
    throw "quiescent efficiency WARP gate requires Windows"
}
if (-not (Test-Path $ExePath)) {
    throw "canonical tze_hud executable not found: $ExePath"
}

$repoRoot = (git rev-parse --show-toplevel).Trim()
if (-not $repoRoot) {
    throw "unable to resolve repository root"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$artifactPath = Join-Path $OutputDir "quiescent-efficiency.json"
$reportPath = Join-Path $OutputDir "quiescent-efficiency-gate.json"
$stdoutPath = Join-Path $OutputDir "runtime.stdout.log"
$stderrPath = Join-Path $OutputDir "runtime.stderr.log"
$configPath = Join-Path $repoRoot "app\tze_hud_app\config\benchmark.toml"

$psk = "quiescent-efficiency-$([Guid]::NewGuid().ToString('N'))"
$arguments = @(
    "--config", $configPath,
    "--window-mode", "overlay",
    "--width", "640",
    "--height", "360",
    "--grpc-port", "0",
    "--mcp-port", "0",
    "--psk", $psk,
    "--quiescent-efficiency-emit", $artifactPath
)

$process = Start-Process `
    -FilePath $ExePath `
    -ArgumentList $arguments `
    -PassThru `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath

# Apply the limit before the five-second settling window starts. The runtime
# independently observes this process mask via GetProcessAffinityMask and the
# checker rejects any value other than exactly two logical CPUs.
try {
    $process.ProcessorAffinity = [IntPtr]3
    $process.Refresh()
    if ([uint64]$process.ProcessorAffinity -ne 3) {
        throw "observed process affinity mask is $($process.ProcessorAffinity), expected 3"
    }
} catch {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
    }
    throw "failed to enforce two-logical-CPU process affinity: $_"
}

$process.WaitForExit()
$runtimeExit = $process.ExitCode
if (-not (Test-Path $artifactPath)) {
    throw "quiescent runtime did not emit an artifact; exit=$runtimeExit stdout=$stdoutPath stderr=$stderrPath"
}

& python "$repoRoot\scripts\ci\check_idle_efficiency.py" `
    $artifactPath `
    --report $reportPath `
    --require-constrained `
    --require-window-mode overlay
$checkerExit = $LASTEXITCODE

if ($runtimeExit -ne 0) {
    throw "quiescent runtime reported failure ($runtimeExit); stdout=$stdoutPath stderr=$stderrPath"
}
if ($checkerExit -ne 0) {
    exit $checkerExit
}
