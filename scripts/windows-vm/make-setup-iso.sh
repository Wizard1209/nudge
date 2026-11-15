#!/usr/bin/env bash
#
# Build the answer ISO for an unattended Windows 11 ARM64 install of the test
# VM: autounattend.xml + the first-logon script from setup/, UTM's guest tools,
# the Win32-OpenSSH MSI and the public half of the ssh key. Everything is
# staged and written under $WIN_VM_DIR, never inside the repo.
#
#   WIN_VM_PASSWORD=... scripts/windows-vm/make-setup-iso.sh
#
# The account's password is not in the repo: it comes from WIN_VM_PASSWORD or
# from the file $WIN_VM_PASSWORD_FILE. It only matters for autologon and the
# console; the runner logs in with the key.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DIR=${WIN_VM_DIR:-$HOME/.cache/nudge-win-vm}
VM_USER=${WIN_VM_USER:-nudge}
KEY=${WIN_VM_KEY:-$HOME/.config/nudge-vm/win_ed25519}
PASS_FILE=${WIN_VM_PASSWORD_FILE:-$HOME/.config/nudge-vm/win_password}
GUEST_TOOLS_ISO=${GUEST_TOOLS_ISO:-$DIR/utm-guest-tools.iso}
OPENSSH_MSI=${OPENSSH_MSI:-$DIR/OpenSSH-ARM64-v10.4.0.0.msi}
OUT=${SETUP_ISO:-$DIR/nudge-setup.iso}

PASS=${WIN_VM_PASSWORD:-}
if [[ -z $PASS && -f $PASS_FILE ]]; then PASS=$(head -1 "$PASS_FILE"); fi
[[ -n $PASS ]] || { echo "set WIN_VM_PASSWORD or write the password to $PASS_FILE" >&2; exit 2; }

mkdir -p "$DIR" "$(dirname "$KEY")"
[[ -f $GUEST_TOOLS_ISO ]] || curl -fsSL -o "$GUEST_TOOLS_ISO" https://getutm.app/downloads/utm-guest-tools-latest.iso
[[ -f $OPENSSH_MSI ]] || curl -fsSL -o "$OPENSSH_MSI" \
  https://github.com/PowerShell/Win32-OpenSSH/releases/download/10.4.0.0p1-Preview/OpenSSH-ARM64-v10.4.0.0.msi
[[ -f $KEY ]] || ssh-keygen -t ed25519 -N '' -C nudge-win-vm -f "$KEY" >/dev/null

STAGE="$DIR/setup-iso"
rm -rf "$STAGE"; mkdir -p "$STAGE"
MNT=$(mktemp -d)
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$GUEST_TOOLS_ISO" >/dev/null
trap 'hdiutil detach -quiet "$MNT" 2>/dev/null || true; rmdir "$MNT" 2>/dev/null || true' EXIT
cp "$MNT"/utm-guest-tools-*.exe "$STAGE/"
cp -R "$MNT/Drivers" "$STAGE/"
cp "$MNT"/*.txt "$STAGE/" 2>/dev/null || true
cp "$OPENSSH_MSI" "$STAGE/"
cp "$KEY.pub" "$STAGE/authorized_key.pub"
for f in autounattend.xml nudge-setup.ps1; do
  # perl interpolates the variables into the replacement as plain text, so no
  # character of the password can act as a metacharacter.
  NUDGE_U="$VM_USER" NUDGE_P="$PASS" perl -pe 's/\@VM_USER\@/$ENV{NUDGE_U}/g; s/\@VM_PASSWORD\@/$ENV{NUDGE_P}/g' \
    "$HERE/setup/$f" > "$STAGE/$f"
done
rm -f "$OUT"
hdiutil makehybrid -quiet -iso -joliet -default-volume-name NUDGESETUP -o "$OUT" "$STAGE"
echo "$OUT"
