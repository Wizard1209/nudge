# Independent check of Nudge's autostart registry value.
#
# This deliberately does NOT call any Nudge code — it reads the registry
# directly so it catches bugs the app's own is_enabled() would share (wrong
# subkey, wrong value name, unquoted path). Run it before and after
# `nudge.exe --autostart-selftest`, or after toggling autostart in Settings,
# to confirm what actually landed in the registry.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\autostart-check.ps1

$ErrorActionPreference = 'Stop'

$key  = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$name = 'Nudge'

$value = (Get-ItemProperty -Path $key -Name $name -ErrorAction SilentlyContinue).$name

if ($null -eq $value) {
    Write-Host "autostart: ABSENT  (no `"$name`" value under $key)"
    exit 1
} else {
    Write-Host "autostart: PRESENT (`"$name`" = $value)"
    exit 0
}
