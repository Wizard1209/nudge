#!/usr/bin/env bash
#
# Run the Windows native tiers — tests/native_render.rs, tests/perf_idle.rs and
# tests/windows_native.rs — against a real desktop session inside a Windows 11
# ARM64 VM under UTM, from a Mac.
#
# The Mac counterpart of scripts/native-tests.sh, which runs native_render from
# WSL on a Windows machine's own desktop. Nothing of that path applies here: the
# tests cannot run on the Mac, so this script cross-builds them, ships the
# binaries into the guest and starts them inside the logged-in session.
#
#   scripts/native-tests-windows.sh                  # every suite; leaves the VM up
#   scripts/native-tests-windows.sh --stop           # ... and stop it afterwards
#   scripts/native-tests-windows.sh -- <filter>      # extra args for each test binary
#   SUITE=windows_native scripts/native-tests-windows.sh   # one suite (or a list)
#   REPO=/path/to/other/checkout scripts/native-tests-windows.sh
#
# The VM is built once, by the steps in docs/windows-vm.md. Defaults below
# match that document; every local path is an environment variable.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
VM=${WIN_VM:-nudge-win11}
VM_USER=${WIN_VM_USER:-nudge}
# Outside the repo on purpose, like the macOS guest's key.
KEY=${WIN_VM_KEY:-$HOME/.config/nudge-vm/win_ed25519}
KNOWN_HOSTS=${WIN_VM_KNOWN_HOSTS:-$(dirname "$KEY")/known_hosts}
REPO=${REPO:-$(cd "$HERE/.." && pwd)}
REPO=$(cd "$REPO" && pwd)
SUITE=${SUITE:-native_render perf_idle windows_native}
SUITE_TIMEOUT=${SUITE_TIMEOUT:-1200}
# Prefixed to each suite's command line in the guest, as `set K=V`. For
# falsification runs and for OPENAI_API_KEY, which the vision judge in
# native_render needs and otherwise skips without.
GUEST_ENV=${GUEST_ENV:-}
TARGET=x86_64-pc-windows-gnu
# The judge's key, as scripts/native-tests-macos.sh does it: only the binaries
# are shipped, so the repo's .env never reaches the guest, and without this
# the vision judge no-op-passes on a machine that is in fact configured.
if [[ $GUEST_ENV != *OPENAI_API_KEY=* && -f $REPO/.env ]]; then
  JUDGE_KEY=$(sed -n 's/^OPENAI_API_KEY=//p' "$REPO/.env" | tr -d '"'"'"' ' | head -1)
  [[ -n $JUDGE_KEY ]] && GUEST_ENV="OPENAI_API_KEY=$JUDGE_KEY $GUEST_ENV"
fi
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$REPO/target}
# Absolute: the guest mirrors this path, and cargo bakes it into the tests.
mkdir -p "$CARGO_TARGET_DIR" && CARGO_TARGET_DIR=$(cd "$CARGO_TARGET_DIR" && pwd)
LOG_DIR=${LOG_DIR:-$CARGO_TARGET_DIR/native-tests-windows/$(date +%Y%m%d-%H%M%S)}

STOP_AFTER=0
TEST_ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --stop) STOP_AFTER=1; shift ;;
    --) shift; TEST_ARGS=("$@"); break ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

mkdir -p "$LOG_DIR"
T0=$(date +%s)
say() { printf '\n== [%4ss] %s\n' "$(( $(date +%s) - T0 ))" "$*"; }
die() { say "FATAL: $*"; exit 2; }

# HostKeyAlias pins the guest's host key to the VM's name, so a new DHCP lease
# after a reboot is not a "changed host key".
SSH_OPTS=(-i "$KEY" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new
  -o UserKnownHostsFile="$KNOWN_HOSTS" -o HostKeyAlias="$VM" -o ConnectTimeout=8
  -o ServerAliveInterval=15 -o LogLevel=ERROR)
IP=
vs() { ssh "${SSH_OPTS[@]}" "$VM_USER@$IP" "$@"; }        # remote shell is PowerShell
vcp() { scp -q "${SSH_OPTS[@]}" "$@"; }                     # guest paths as $VM_USER@$IP:C:/...
# Run a cmd.exe command line in the console session, where windows exist.
insession() { # <job name> <timeout s> <command line>
  vs "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\nudge\\in-session.ps1 -Name $1 -TimeoutSec $2 -Cmd '$3'"
}

# --- 1. start the guest ----------------------------------------------------
pgrep -qf 'UTM.app/Contents/MacOS/UTM' || { open -a UTM; sleep 5; }
st=$(utmctl status "$VM" 2>&1)
if [[ $st != started ]]; then
  say "starting $VM ($st)"
  utmctl start "$VM" || die "utmctl start failed — a sheet or dialog open in UTM blocks it (docs/windows-vm.md)"
else
  say "$VM already running"
fi

# --- 2. ssh, and a logged-in console session ------------------------------
# sshd lives in session 0, where windows are invisible; the tests need the
# autologon session to be Active on the console.
deadline=$(( $(date +%s) + 600 ))
until IP=$("$HERE/windows-vm/vm-ip.sh" "$VM") && nc -z -G 3 "$IP" 22 2>/dev/null; do
  [[ $(date +%s) -lt $deadline ]] || die "no ssh on the guest within 600 s"; sleep 10
done
# `query session` exits non-zero even when it succeeds, so its text is what is tested.
until qs=$(vs 'query session' 2>/dev/null; true) && grep -Eq "console +$VM_USER +[0-9]+ +Active" <<<"$qs"; do
  [[ $(date +%s) -lt $deadline ]] || die "console session of $VM_USER not Active within 600 s"; sleep 10
done
say "guest $IP: $(vs 'cmd /c ver' | tr -d '\r' | grep -m1 Version)"

# --- 3. build on the host ---------------------------------------------------
say "building $REPO (target dir $CARGO_TARGET_DIR)"
SUITE_FLAGS=()
for s in $SUITE; do SUITE_FLAGS+=(--test "$s"); done
( cd "$REPO" && cargo build --release --bin nudge --target $TARGET ) >"$LOG_DIR/build.log" 2>&1 \
  || { tail -30 "$LOG_DIR/build.log"; die "cargo build failed, see $LOG_DIR/build.log"; }
( cd "$REPO" && cargo test --release --target $TARGET "${SUITE_FLAGS[@]}" --no-run --message-format=json ) \
  >"$LOG_DIR/build-tests.json" 2>"$LOG_DIR/build-tests.log" \
  || { tail -30 "$LOG_DIR/build-tests.log"; die "building the test binaries failed, see $LOG_DIR/build-tests.log"; }
/usr/bin/python3 -I -c '
import json, sys
want = set(sys.argv[2].split())
for line in open(sys.argv[1]):
    try: m = json.loads(line)
    except ValueError: continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] in want:
        print(m["target"]["name"], m["executable"])' "$LOG_DIR/build-tests.json" "$SUITE" >"$LOG_DIR/test-exes.txt"
test_exe() { awk -v s="$1" '$1==s{print $2}' "$LOG_DIR/test-exes.txt" | tail -1; }
NUDGE="$CARGO_TARGET_DIR/$TARGET/release/nudge.exe"
[[ -f $NUDGE ]] || die "no $NUDGE"
for s in $SUITE; do [[ -f $(test_exe "$s") ]] || die "no test binary for $s"; done

# --- 4. ship ---------------------------------------------------------------
# The tests spawn env!("CARGO_BIN_EXE_nudge"), the HOST path baked in at
# compile time. Windows resolves "/Users/..." against drive C:, so the same
# path under C:\ is where nudge.exe has to be.
BIN_DIR=$(dirname "$NUDGE")
GBIN="C:$(tr / '\\' <<<"$BIN_DIR")"
vs "New-Item -ItemType Directory -Force '$GBIN\\deps','C:\\nudge\\shots','C:\\nudge\\jobs' | Out-Null; Stop-Process -Name nudge -Force -ErrorAction SilentlyContinue; exit 0" \
  || die "guest mkdir failed"
vcp "$HERE"/windows-vm/*.ps1 "$VM_USER@$IP:C:/nudge/" || die "copying the guest helpers failed"
vcp "$NUDGE" "$VM_USER@$IP:C:$BIN_DIR/" || die "copying nudge.exe failed"
for s in $SUITE; do vcp "$(test_exe "$s")" "$VM_USER@$IP:C:$BIN_DIR/deps/" || die "copying $s failed"; done
say "shipped to $GBIN"

# --- 5. OpenGL for an x64 process on ARM64 ---------------------------------
# The VM's display adapter has no 3D, so stock Windows offers OpenGL 1.1 and
# eframe refuses to start. Microsoft's compatibility pack maps GL onto D3D12.
if ! vs 'if (Get-AppxPackage -AllUsers Microsoft.D3DMappingLayers) { exit 0 } else { exit 1 }' >/dev/null 2>&1; then
  say "installing the OpenCL, OpenGL and Vulkan Compatibility Pack"
  insession winget 900 'winget install --id 9NQPSL29BFFF -s msstore --accept-package-agreements --accept-source-agreements --disable-interactivity' \
    >"$LOG_DIR/winget.txt" 2>&1 || die "winget failed, see $LOG_DIR/winget.txt"
fi

# --- 6. a known desktop ------------------------------------------------------
# 1920x1080, solid grey, no icons: native_render classifies pixels by
# brightness, and wallpaper or icon pixels in the card's band corrupt it.
insession desk 120 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\nudge\desktop-setup.ps1' \
  >"$LOG_DIR/desktop-setup.txt" 2>&1 || die "desktop setup failed, see $LOG_DIR/desktop-setup.txt"

# --- 7. the suites, one at a time --------------------------------------------
# --test-threads=1: the tests drive one keyboard and one foreground window.
ENV_PREFIX=
for kv in $GUEST_ENV; do ENV_PREFIX+="set \"$kv\"&& "; done
RCS=
for s in $SUITE; do
  say "running $s"
  extra=
  [[ $s == native_render ]] && extra='set NUDGE_DUMP_PNG=C:\nudge\shots\native_render-capture.png&& '
  vs "Stop-Process -Name nudge -Force -ErrorAction SilentlyContinue; exit 0" >/dev/null
  insession "$s" "$SUITE_TIMEOUT" \
    "$ENV_PREFIX${extra}cd /d $GBIN\\deps && $(basename "$(test_exe "$s")") --ignored --nocapture --test-threads=1 ${TEST_ARGS[*]:-}" \
    2>&1 | LC_ALL=C tr -d '\r' | tee "$LOG_DIR/$s.txt"
  RCS+="$s=${PIPESTATUS[0]} "
done
vs "Stop-Process -Name nudge -Force -ErrorAction SilentlyContinue; exit 0" >/dev/null
vcp "$VM_USER@$IP:C:/nudge/shots/native_render-capture.png" "$LOG_DIR/" 2>/dev/null

# --- 8. verdict ----------------------------------------------------------------
say "results (logs in $LOG_DIR)"
fail=0
for s in $SUITE; do
  rc=$(tr ' ' '\n' <<<"$RCS" | awk -F= -v s="$s" '$1==s{print $2}')
  printf '  %-16s exit %-4s %s\n' "$s" "$rc" "$(grep -a '^test result' "$LOG_DIR/$s.txt" | tail -1)"
  [[ $rc == 0 ]] || fail=1
done
echo "  total $(( $(date +%s) - T0 )) s"
[[ $STOP_AFTER -eq 1 ]] && { say "stopping $VM"; utmctl stop "$VM"; }
exit $fail
