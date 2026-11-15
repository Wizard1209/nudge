# Run a command line in the interactive desktop session of user nudge (session 1),
# not in sshd's session 0. Waits for it to finish and prints its captured output.
# usage (over ssh): powershell -File C:\nudge\in-session.ps1 -Cmd '<cmd.exe command line>' [-TimeoutSec 600] [-Name job]
# The wrapper .cmd is deleted when the job ends: a command line may carry environment
# (scripts/native-tests-windows.sh GUEST_ENV) that should not stay on the guest's disk.
param(
  [Parameter(Mandatory=$true)][string]$Cmd,
  [int]$TimeoutSec = 600,
  [string]$Name = 'nudge-job'
)
$ErrorActionPreference = 'Stop'
$dir = 'C:\nudge\jobs'
New-Item -ItemType Directory -Force $dir | Out-Null
$out = Join-Path $dir "$Name.out.txt"
$rcf = Join-Path $dir "$Name.rc.txt"
$bat = Join-Path $dir "$Name.cmd"
Remove-Item $out, $rcf -ErrorAction SilentlyContinue
# The .cmd wrapper captures stdout+stderr and the exit code into files the ssh side can read.
Set-Content -Encoding ASCII -Path $bat -Value @(
  '@echo off',
  "($Cmd) > `"$out`" 2>&1",
  "echo %ERRORLEVEL% > `"$rcf`""
)
# conhost --headless: the job gets a console but no console WINDOW, so nothing dark sits
# on the desktop behind the window under test (the render test compares against the desktop).
$action = New-ScheduledTaskAction -Execute 'conhost.exe' -Argument "--headless cmd.exe /c `"$bat`""
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Seconds ($TimeoutSec + 60))
Register-ScheduledTask -TaskName $Name -Action $action -Principal $principal -Settings $settings -Force | Out-Null
Start-ScheduledTask -TaskName $Name
$t0 = Get-Date
while (-not (Test-Path $rcf) -and ((Get-Date) - $t0).TotalSeconds -lt $TimeoutSec) { Start-Sleep -Milliseconds 500 }
Start-Sleep -Milliseconds 300
$timedOut = -not (Test-Path $rcf)
if ($timedOut) { Stop-ScheduledTask -TaskName $Name -ErrorAction SilentlyContinue }
Unregister-ScheduledTask -TaskName $Name -Confirm:$false -ErrorAction SilentlyContinue
Remove-Item $bat -ErrorAction SilentlyContinue
if (Test-Path $out) { Get-Content $out }
if ($timedOut) {
  Write-Output "[in-session] TIMEOUT after $TimeoutSec s"
  exit 124
}
$rc = [int]((Get-Content $rcf | Select-Object -First 1).Trim())
Write-Output "[in-session] exit code $rc"
exit $rc
