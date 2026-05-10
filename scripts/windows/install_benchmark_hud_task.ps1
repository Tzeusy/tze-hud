# Register a dedicated Windows scheduled task for benchmark HUD runs.
#
# This script does not modify the production TzeHudOverlay task. It creates or
# replaces a separate task that launches tze_hud.exe with benchmark.toml and
# explicit ports, so live performance runs can use benchmark-only agent grants
# without broadening the production config.

[CmdletBinding()]
param(
    [Parameter(Mandatory=$false)]
    [string]$BaseDir = "C:\tze_hud",

    [Parameter(Mandatory=$false)]
    [string]$TaskName = "TzeHudBenchmarkOverlay",

    [Parameter(Mandatory=$false)]
    [string]$ExeName = "tze_hud.exe",

    [Parameter(Mandatory=$false)]
    [string]$ConfigName = "benchmark.toml",

    [Parameter(Mandatory=$false)]
    [int]$GrpcPort = 50051,

    [Parameter(Mandatory=$false)]
    [int]$McpPort = 9090,

    [Parameter(Mandatory=$false)]
    [ValidateSet("Limited", "Highest")]
    [string]$RunLevel = "Highest",

    [Parameter(Mandatory=$false)]
    [string]$Psk = $env:TZE_HUD_PSK,

    [Parameter(Mandatory=$false)]
    [switch]$WhatIf
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Write-BenchmarkTaskLog([string]$Message) {
    Write-Host "[benchmark-task] $Message"
}

$ExePath = Join-Path $BaseDir $ExeName
$ConfigPath = Join-Path $BaseDir $ConfigName
$LogDir = Join-Path $BaseDir "logs"
$RunScriptPath = Join-Path $BaseDir "run_benchmark_hud.ps1"
$PskPath = Join-Path $BaseDir "benchmark_hud.psk.dpapi"

$EscapedExePath = $ExePath.Replace('`', '``').Replace('"', '`"')
$EscapedConfigPath = $ConfigPath.Replace('`', '``').Replace('"', '`"')
$EscapedLogDir = $LogDir.Replace('`', '``').Replace('"', '`"')
$EscapedPskPath = $PskPath.Replace('`', '``').Replace('"', '`"')

if ($WhatIf) {
    Write-BenchmarkTaskLog "WhatIf: would write $RunScriptPath"
    Write-BenchmarkTaskLog "WhatIf: would write DPAPI-protected PSK to $PskPath"
    Write-BenchmarkTaskLog "WhatIf: would register task $TaskName for config $ConfigPath"
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Psk)) {
    throw "PSK is required. Pass -Psk or set TZE_HUD_PSK before registering the benchmark task."
}

if ($Psk -eq "tze-hud-key") {
    throw "Refusing to register benchmark task with the default development PSK."
}

if (-not (Test-Path $ExePath)) {
    throw "Executable not found: $ExePath"
}
if (-not (Test-Path $ConfigPath)) {
    throw "Benchmark config not found: $ConfigPath"
}

New-Item -ItemType Directory -Path $LogDir -Force | Out-Null

$runner = @"
`$ErrorActionPreference = "Stop"
`$ExePath = "$EscapedExePath"
`$ConfigPath = "$EscapedConfigPath"
`$LogDir = "$EscapedLogDir"
`$PskPath = "$EscapedPskPath"
`$GrpcPort = $GrpcPort
`$McpPort = $McpPort

New-Item -ItemType Directory -Path `$LogDir -Force | Out-Null
`$launcher = Join-Path `$LogDir "benchmark-hud.launcher.log"
`$stdout = Join-Path `$LogDir "benchmark-hud.stdout.log"
`$stderr = Join-Path `$LogDir "benchmark-hud.stderr.log"
`$processFileName = [System.IO.Path]::GetFileName(`$ExePath)

function Get-BenchmarkHudProcess {
    Get-CimInstance Win32_Process -Filter "Name = '`$processFileName'" -ErrorAction SilentlyContinue |
        Where-Object {
            `$cmd = [string]`$_.CommandLine
            `$cmd.IndexOf(`$ConfigPath, [System.StringComparison]::OrdinalIgnoreCase) -ge 0 -and
                `$cmd.IndexOf("--grpc-port `$GrpcPort", [System.StringComparison]::OrdinalIgnoreCase) -ge 0 -and
                `$cmd.IndexOf("--mcp-port `$McpPort", [System.StringComparison]::OrdinalIgnoreCase) -ge 0
        }
}

if (-not (Test-Path `$PskPath)) {
    throw "DPAPI-protected benchmark PSK not found: `$PskPath"
}
`$securePsk = (Get-Content -Path `$PskPath -Raw).Trim() | ConvertTo-SecureString
`$pskPtr = [System.IntPtr]::Zero
try {
    `$pskPtr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR(`$securePsk)
    `$env:TZE_HUD_PSK = [Runtime.InteropServices.Marshal]::PtrToStringBSTR(`$pskPtr)
} finally {
    if (`$pskPtr -ne [System.IntPtr]::Zero) {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR(`$pskPtr)
    }
}

`$ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
Add-Content -Path `$launcher -Value "[`$ts] launching benchmark HUD: `$ExePath --config `$ConfigPath"

Get-BenchmarkHudProcess | ForEach-Object {
    Stop-Process -Id `$_.ProcessId -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Milliseconds 300

`$args = @(
    "--config", `$ConfigPath,
    "--window-mode", "overlay",
    "--grpc-port", [string]`$GrpcPort,
    "--mcp-port", [string]`$McpPort
)

`$workdir = Split-Path `$ExePath -Parent
`$startArgs = @{
    FilePath = `$ExePath
    ArgumentList = `$args
    WorkingDirectory = `$workdir
    RedirectStandardOutput = `$stdout
    RedirectStandardError = `$stderr
}
Start-Process @startArgs

Start-Sleep -Milliseconds 500
`$proc = Get-BenchmarkHudProcess |
    Select-Object -First 1
`$ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
if (`$null -ne `$proc) {
    Add-Content -Path `$launcher -Value "[`$ts] started pid=`$(`$proc.ProcessId) grpc=`$GrpcPort mcp=`$McpPort"
} else {
    Add-Content -Path `$launcher -Value "[`$ts] ERROR: process not found after launch"
    throw "benchmark HUD process did not appear after launch"
}
"@

Set-Content -Path $RunScriptPath -Value $runner -Encoding UTF8
$securePsk = ConvertTo-SecureString -String $Psk -AsPlainText -Force
$securePsk | ConvertFrom-SecureString | Set-Content -Path $PskPath -Encoding UTF8

$candidateAccounts = @(
    "$env:USERDOMAIN\$env:USERNAME",
    "$env:COMPUTERNAME\$env:USERNAME",
    "$env:USERNAME"
) | Select-Object -Unique

$taskUser = $null
foreach ($candidate in $candidateAccounts) {
    try {
        $null = (New-Object System.Security.Principal.NTAccount($candidate)).Translate([System.Security.Principal.SecurityIdentifier])
        $taskUser = $candidate
        break
    } catch {
        continue
    }
}

if ($null -eq $taskUser) {
    throw "Could not resolve a valid task user from: $($candidateAccounts -join ', ')"
}

icacls $RunScriptPath /inheritance:r /grant:r "${taskUser}:F" "Administrators:F" "SYSTEM:F" | Out-Null
icacls $PskPath /inheritance:r /grant:r "${taskUser}:F" "Administrators:F" "SYSTEM:F" | Out-Null

Write-BenchmarkTaskLog "Registering $TaskName as $taskUser with config $ConfigPath"
$action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$RunScriptPath`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $taskUser
$principal = New-ScheduledTaskPrincipal -UserId $taskUser -LogonType Interactive -RunLevel $RunLevel
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null

Write-BenchmarkTaskLog "Registered. Launch with: schtasks /Run /TN $TaskName"
Write-BenchmarkTaskLog "Logs: $LogDir\benchmark-hud.*.log"
