# First-logon setup for the nudge-win11 VM. Runs elevated in the nudge user's session.
# Log: C:\nudge-setup\firstlogon.log ; completion marker: C:\nudge-setup\done.txt
$ErrorActionPreference = 'Continue'
$here = 'C:\nudge-setup'
Start-Transcript -Path "$here\firstlogon.log" -Append
function Step($m) { Write-Output ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }

Step 'power, lock screen, screensaver'
powercfg /hibernate off
foreach ($k in 'standby-timeout-ac','standby-timeout-dc','monitor-timeout-ac','monitor-timeout-dc','disk-timeout-ac','disk-timeout-dc') { powercfg /change $k 0 }
powercfg /setacvalueindex SCHEME_CURRENT SUB_NONE CONSOLELOCK 0
powercfg /setdcvalueindex SCHEME_CURRENT SUB_NONE CONSOLELOCK 0
reg add "HKCU\Control Panel\Desktop" /v ScreenSaveActive /t REG_SZ /d 0 /f
reg add "HKCU\Control Panel\Desktop" /v ScreenSaverIsSecure /t REG_SZ /d 0 /f
reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\Control Panel\Desktop" /v ScreenSaveActive /t REG_SZ /d 0 /f
reg add HKLM\SOFTWARE\Policies\Microsoft\Windows\Personalization /v NoLockScreen /t REG_DWORD /d 1 /f
reg add HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System /v InactivityTimeoutSecs /t REG_DWORD /d 0 /f
reg add HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System /v ConsentPromptBehaviorAdmin /t REG_DWORD /d 0 /f
reg add HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System /v PromptOnSecureDesktop /t REG_DWORD /d 0 /f
# Permanent autologon (the unattend LogonCount decrements).
$wl = 'HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
reg add $wl /v AutoAdminLogon /t REG_SZ /d 1 /f
reg add $wl /v DefaultUserName /t REG_SZ /d @VM_USER@ /f
reg add $wl /v DefaultPassword /t REG_SZ /d @VM_PASSWORD@ /f
reg add $wl /v DefaultDomainName /t REG_SZ /d . /f
reg delete $wl /v AutoLogonCount /f 2>$null
# No first-run animation / tips on later logons; no Windows Update auto-reboots.
reg add HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System /v EnableFirstLogonAnimation /t REG_DWORD /d 0 /f
reg add HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU /v NoAutoRebootWithLoggedOnUsers /t REG_DWORD /d 1 /f
reg add HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU /v NoAutoUpdate /t REG_DWORD /d 1 /f

Step 'UTM guest tools (silent)'
$gt = Get-ChildItem "$here\utm-guest-tools-*.exe" | Select-Object -First 1
if ($gt) {
  $p = Start-Process -FilePath $gt.FullName -ArgumentList '/S' -Wait -PassThru
  Step "guest tools exit code $($p.ExitCode)"
} else { Step 'guest tools exe not found' }

Step 'wait for network (IPv4 from DHCP)'
$ip = $null
for ($i = 0; $i -lt 30 -and -not $ip; $i++) {
  $ip = (Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue | Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } | Select-Object -First 1).IPAddress
  if (-not $ip) { Start-Sleep 5 }
}
Step "IPv4: $ip"

Step 'OpenSSH Server'
if (-not (Get-Service sshd -ErrorAction SilentlyContinue)) {
  $job = Start-Job { Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0 }
  if (Wait-Job $job -Timeout 600) { Receive-Job $job | Out-String | Write-Output } else { Step 'Add-WindowsCapability timed out'; Stop-Job $job }
}
if (-not (Get-Service sshd -ErrorAction SilentlyContinue)) {
  Step 'capability unavailable -> Win32-OpenSSH MSI from the setup ISO'
  $msi = Get-ChildItem "$here\OpenSSH-ARM64-*.msi" | Select-Object -First 1
  $p = Start-Process msiexec.exe -ArgumentList '/i', "`"$($msi.FullName)`"", '/qn', '/norestart', '/l*v', "$here\openssh-msi.log" -Wait -PassThru
  Step "msiexec exit code $($p.ExitCode)"
}
Set-Service sshd -StartupType Automatic
Start-Service sshd
New-Item -ItemType Directory -Force 'C:\ProgramData\ssh' | Out-Null
Copy-Item "$here\authorized_key.pub" 'C:\ProgramData\ssh\administrators_authorized_keys' -Force
icacls 'C:\ProgramData\ssh\administrators_authorized_keys' /inheritance:r /grant 'Administrators:F' /grant 'SYSTEM:F'
New-Item -Path 'HKLM:\SOFTWARE\OpenSSH' -Force | Out-Null
New-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell -Value 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -PropertyType String -Force | Out-Null
Remove-NetFirewallRule -Name 'nudge-sshd' -ErrorAction SilentlyContinue
New-NetFirewallRule -Name 'nudge-sshd' -DisplayName 'OpenSSH Server (nudge VM)' -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow -Profile Any | Out-Null
Restart-Service sshd
Get-Service sshd | Format-List Name, Status, StartType | Out-String | Write-Output

Step 'done'
Set-Content -Path "$here\done.txt" -Value (Get-Date -Format o)
Stop-Transcript
# Reboot so the guest-tools display/input drivers load cleanly; autologon brings the session back.
shutdown /r /t 15 /c "nudge VM setup finished"
