# The Windows test VM

`scripts/native-tests-windows.sh` runs the Windows native suites —
`tests/native_render.rs`, `tests/perf_idle.rs`, `tests/windows_native.rs` —
from a Mac, inside a UTM virtual machine named `nudge-win11`: Windows 11 ARM64
running the x86_64 `nudge.exe` under the system's x64 emulation. This page is
how that VM is built and why it is built that way.

## What the runner relies on

- **A logged-in desktop.** sshd runs in session 0, where a window is invisible
  and a screen capture is black. `scripts/windows-vm/in-session.ps1` starts each
  test binary in the console session instead, through a scheduled task with an
  interactive principal, and hands its output and exit code back over ssh. The
  task runs under `conhost --headless`, so no console window sits on the desktop
  behind the popup under test. The guest logs its user in automatically and
  never locks.
- **The host path, mirrored.** The test binaries spawn
  `env!("CARGO_BIN_EXE_nudge")`, a path baked in on the Mac at compile time.
  Windows reads `/Users/...` as `C:\Users\...`, so the runner copies `nudge.exe`
  there. The exe imports only system DLLs; no MinGW runtime is needed.
- **OpenGL 2+.** The VM's display adapter (VirtIO GPU DOD) has no 3D, so stock
  Windows offers OpenGL 1.1 and eframe refuses to start. Microsoft's *OpenCL,
  OpenGL and Vulkan Compatibility Pack* (Store id `9NQPSL29BFFF`) maps GL onto
  D3D12 on WARP; the runner installs it with winget when it is missing. Mesa's
  llvmpipe crashes under x64 emulation; mesa-dist-win's softpipe works but is
  not used.
- **A known desktop.** `scripts/windows-vm/desktop-setup.ps1` runs before the
  suites: 1920×1080, a solid grey background, no Spotlight wallpaper, no desktop
  icons. `native_render` classifies pixels by brightness, and wallpaper or icon
  pixels in the card's band corrupt its measurement.
- **The journal is the guest user's.** The app writes to the Documents known
  folder and nothing redirects it, so `windows_native` moves
  `Documents\Nudge\journal-rust.ndjson` aside for each test that reads it and
  puts it back afterwards.

## Configuration

Everything local is an environment variable of the runner:

| variable | default | |
|---|---|---|
| `WIN_VM` | `nudge-win11` | UTM VM name |
| `WIN_VM_USER` | `nudge` | guest account the key logs into |
| `WIN_VM_KEY` | `~/.config/nudge-vm/win_ed25519` | ssh private key, kept outside the repo |
| `WIN_VM_KNOWN_HOSTS` | next to the key | host key, pinned to the VM name rather than its DHCP address |
| `REPO` | the checkout holding the script | the tree that is built and tested |
| `CARGO_TARGET_DIR` | `$REPO/target` | where the cross build goes; logs land in `native-tests-windows/<timestamp>/` under it |
| `SUITE` | `native_render perf_idle windows_native` | which test binaries run |
| `GUEST_ENV` | — | `K=V` pairs set for each suite in the guest, e.g. `OPENAI_API_KEY` for the vision judge |

## Building the VM (about 15 minutes, mostly unattended)

Prerequisites: UTM 5 at `/Applications/UTM.app` (its `utmctl` on `PATH`), the
Rust target `x86_64-pc-windows-gnu` with mingw-w64, `jq`, and about 30 GB free
for the ISO and the VM disk. `WIN_VM_DIR` (default `~/.cache/nudge-win-vm`)
holds the downloads; nothing is written into the repo.

```bash
V=scripts/windows-vm
# 1. the Windows 11 ARM64 ISO, from Microsoft's download page (one link per 24 h;
#    a refused request means an IP block for about a day — do not retry in a loop)
$V/fetch-iso.sh

# 2. the answer ISO: unattended install, guest tools, OpenSSH, the public key.
#    The account password is not in the repo; pick one.
WIN_VM_PASSWORD=... $V/make-setup-iso.sh

# 3. the VM: 4 cores, 8 GB, 64 GB NVMe, TPM, secure boot vars, local-time clock.
#    UTM must be in front with no "What's New" sheet open (see below).
D=${WIN_VM_DIR:-~/.cache/nudge-win-vm}
open -a UTM
osascript $V/create-vm.applescript "$D/win11-arm64.iso" "$D/nudge-setup.iso"
B=~/Library/Containers/com.utmapp.UTM/Data/Documents/nudge-win11.utm
plutil -replace QEMU.TPMDevice -bool YES "$B/config.plist"
plutil -replace QEMU.RTCLocalTime -bool YES "$B/config.plist"
cp /Applications/UTM.app/Contents/Resources/qemu/edk2-arm-secure-vars.fd "$B/Data/efi_vars.fd"
osascript -e 'tell application "UTM" to reload configuration (virtual machine named "nudge-win11")'

# 4. boot. About ten spaces in ten seconds answer "Press any key to boot from
#    CD"; more land on Setup's Cancel button (one more space answers its "No").
utmctl start nudge-win11
for i in $(seq 1 10); do
  osascript -e 'tell application "UTM" to input keystroke (virtual machine named "nudge-win11") text " "'; sleep 1
done

# 5. Setup, first logon and C:\nudge-setup\nudge-setup.ps1 (guest tools, sshd,
#    firewall, key, power and lock settings) run unattended and reboot once.
until ip=$($V/vm-ip.sh) && nc -z -G 3 "$ip" 22; do sleep 15; done

# 6. drop the installer CDs, fix the resolution, keep a snapshot
ssh -i ~/.config/nudge-vm/win_ed25519 nudge@"$ip" 'shutdown /s /t 0'
until [ "$(utmctl status nudge-win11)" = stopped ]; do sleep 5; done
osascript $V/post-install-config.applescript
utmctl snapshot create nudge-win11 --name installed

# 7. first run installs the GL compatibility pack (a few minutes); snapshot again
scripts/native-tests-windows.sh
utmctl stop nudge-win11 && utmctl snapshot create nudge-win11 --name installed+glon12
```

A clean guest is a snapshot restore, not a rebuild (VM stopped):
`utmctl snapshot restore nudge-win11 --snapshot installed+glon12`.

UTM's "What's New" sheet makes every `utmctl start` fail with OSStatus -1712.
To dismiss it from a script:

```bash
osascript -e 'tell application "System Events" to tell process "UTM"
  set bs to {}
  repeat with e in (entire contents of sheet 1 of window "UTM")
    if role of e is "AXButton" then set end of bs to e
  end repeat
  click last item of bs
end tell'
```

## Files

| path | |
|---|---|
| `scripts/native-tests-windows.sh` | the runner |
| `scripts/windows-vm/in-session.ps1` | run a command line in the console session, return its output and exit code |
| `scripts/windows-vm/desktop-setup.ps1` | normalise the desktop |
| `scripts/windows-vm/screenshot.ps1` | capture the guest's screen to a PNG, for looking at a failure by hand |
| `scripts/windows-vm/vm-ip.sh` | the guest's address: guest agent first, then the vmnet DHCP lease |
| `scripts/windows-vm/fetch-iso.sh` | the Windows ISO |
| `scripts/windows-vm/make-setup-iso.sh`, `setup/` | the answer ISO and its templates |
| `scripts/windows-vm/create-vm.applescript`, `post-install-config.applescript` | VM creation and post-install settings through UTM's scripting |
