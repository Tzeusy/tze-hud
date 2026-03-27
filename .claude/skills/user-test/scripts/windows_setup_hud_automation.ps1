param(
    [string]$BaseDir = "C:\tze_hud",
    [string]$TaskName = "TzeHudInteractive",
    [string]$ExeName = "tze_hud.exe",
    [ValidateSet("Limited", "Highest")]
    [string]$RunLevel = "Limited",
    [switch]$InstallOpenSSH,
    [switch]$SkipTaskRegistration
)

$ErrorActionPreference = "Stop"

$RunScriptPath = Join-Path $BaseDir "run_hud.ps1"
$ExePath = Join-Path $BaseDir $ExeName
$LogDir = Join-Path $BaseDir "logs"

Write-Host "[1/5] Creating directories..."
New-Item -ItemType Directory -Path $BaseDir -Force | Out-Null
New-Item -ItemType Directory -Path $LogDir -Force | Out-Null

if ($InstallOpenSSH) {
    Write-Host "[2/5] Installing/enabling OpenSSH server..."
    $cap = Get-WindowsCapability -Online | Where-Object Name -like "OpenSSH.Server*"
    if ($null -eq $cap) {
        throw "OpenSSH.Server capability not found on this Windows image."
    }
    if ($cap.State -ne "Installed") {
        Add-WindowsCapability -Online -Name $cap.Name | Out-Null
    }
    Start-Service sshd
    Set-Service -Name sshd -StartupType Automatic
    if (-not (Get-NetFirewallRule -Name "OpenSSH-Server-In-TCP" -ErrorAction SilentlyContinue)) {
        New-NetFirewallRule -Name "OpenSSH-Server-In-TCP" `
            -DisplayName "OpenSSH Server (sshd)" `
            -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 | Out-Null
    }
} else {
    Write-Host "[2/5] Skipping OpenSSH install (use -InstallOpenSSH to enable)."
}

Write-Host "[3/5] Writing runner script: $RunScriptPath"
$runner = @"
param(
    [string]`$ExePath = "$ExePath",
    [string]`$LogDir = "$LogDir"
)

`$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Path `$LogDir -Force | Out-Null

`$launcher = Join-Path `$LogDir "hud.launcher.log"
`$ProcessName = [System.IO.Path]::GetFileNameWithoutExtension(`$ExePath)

New-Item -ItemType File -Path `$launcher -Force | Out-Null

`$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
Add-Content -Path `$launcher -Value "[`$now] launching `$ExePath"

Get-Process -Name `$ProcessName -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 200

if (-not (Test-Path `$ExePath)) {
    `$ts = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
    Add-Content -Path `$launcher -Value "[`$ts] ERROR: exe missing at `$ExePath"
    throw "Executable not found at `$ExePath"
}

`$workdir = Split-Path `$ExePath -Parent
Push-Location `$workdir
try {
    cmd.exe /c "start \"\" /B \"`$ExePath\""
} finally {
    Pop-Location
}

Start-Sleep -Milliseconds 300
`$proc = Get-Process -Name `$ProcessName -ErrorAction SilentlyContinue |
    Sort-Object StartTime -Descending |
    Select-Object -First 1
`$ts = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
if (`$null -ne `$proc) {
    Add-Content -Path `$launcher -Value "[`$ts] started pid=`$(`$proc.Id)"
} else {
    Add-Content -Path `$launcher -Value "[`$ts] WARN: process not found after launch"
}
"@
Set-Content -Path $RunScriptPath -Value $runner -Encoding UTF8

if ($SkipTaskRegistration) {
    Write-Host "[4/5] Skipping scheduled task registration (-SkipTaskRegistration)."
    Write-Host "[5/5] Done."
    Write-Host ""
    Write-Host "Runner script ready:"
    Write-Host "  $RunScriptPath"
    Write-Host "Launch directly via SSH:"
    Write-Host "  powershell -NoProfile -ExecutionPolicy Bypass -File $RunScriptPath"
    exit 0
}

Write-Host "[4/5] Registering scheduled task: $TaskName (RunLevel=$RunLevel)"
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

Write-Host "Resolved task user: $taskUser"
$action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$RunScriptPath`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $taskUser
$principal = New-ScheduledTaskPrincipal -UserId $taskUser -LogonType Interactive -RunLevel $RunLevel
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null

Write-Host "[5/5] Done."
Write-Host ""
Write-Host "Next steps:"
Write-Host "1) Ensure SSH key auth is configured for this Windows account."
Write-Host "2) From Linux run deploy script:"
Write-Host "   .claude/skills/user-test/scripts/deploy_windows_hud.sh --win-user $env:USERNAME --tail"
Write-Host "3) Trigger manually anytime:"
Write-Host "   schtasks /Run /TN $TaskName"
