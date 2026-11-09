# Normalize the test desktop (run in session 1): 1920x1080, solid light-grey background,
# no Spotlight wallpaper. The render test's card heuristic (brightness band 12..95) needs a
# desktop that is neither inside that band nor too small to surround the centered popup.
param([int]$W = 1920, [int]$H = 1080)
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class Disp {
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
  public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmDeviceName;
    public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra; public int dmFields;
    public int dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
    public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmFormName;
    public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
    public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
  }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool EnumDisplaySettings(string d, int i, ref DEVMODE m);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int ChangeDisplaySettings(ref DEVMODE m, int f);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool SystemParametersInfo(int a, int p, string v, int f);
  [DllImport("user32.dll")] public static extern bool SetSysColors(int n, int[] e, int[] c);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
[Disp]::SetProcessDPIAware() | Out-Null
$m = New-Object Disp+DEVMODE; $m.dmSize = [Runtime.InteropServices.Marshal]::SizeOf($m)
[Disp]::EnumDisplaySettings($null, -1, [ref]$m) | Out-Null
"before: $($m.dmPelsWidth)x$($m.dmPelsHeight)"
if ($m.dmPelsWidth -ne $W -or $m.dmPelsHeight -ne $H) {
  $m.dmPelsWidth = $W; $m.dmPelsHeight = $H; $m.dmFields = 0x80000 -bor 0x100000
  "ChangeDisplaySettings -> $([Disp]::ChangeDisplaySettings([ref]$m, 1))  (0 = ok)"
}
# Solid background: Spotlight off, wallpaper none, background colour 200,200,200.
New-Item -Force 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers' | Out-Null
Set-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers' BackgroundType 1 -Type DWord
New-Item -Force 'HKCU:\Software\Policies\Microsoft\Windows\CloudContent' | Out-Null
Set-ItemProperty 'HKCU:\Software\Policies\Microsoft\Windows\CloudContent' DisableSpotlightCollectionOnDesktop 1 -Type DWord
Set-ItemProperty 'HKCU:\Control Panel\Colors' Background '200 200 200'
[Disp]::SetSysColors(1, @(1), @(0x00C8C8C8)) | Out-Null
[Disp]::SystemParametersInfo(0x0014, 0, '', 3) | Out-Null   # SPI_SETDESKWALLPAPER, none, update+broadcast
[Disp]::EnumDisplaySettings($null, -1, [ref]$m) | Out-Null
"after: $($m.dmPelsWidth)x$($m.dmPelsHeight)"
# Hide desktop icons: their pixels fall inside the card brightness band and widen the card bbox.
$adv = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
if ((Get-ItemProperty $adv -Name HideIcons -ErrorAction SilentlyContinue).HideIcons -ne 1) {
  Set-ItemProperty $adv HideIcons 1 -Type DWord
  Stop-Process -Name explorer -Force; Start-Sleep 4
  if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe; Start-Sleep 4 }
  "desktop icons hidden, explorer restarted"
}
