#!/usr/bin/env bash
#
# Run tests/macos_native.rs against a real GUI session inside a Tart guest.
#
# The macOS counterpart of scripts/native-tests.sh, and it exists for the same
# reason: the native tier needs a logged-in desktop that CI does not have. It
# also solves a problem the Windows side does not have — a developer Mac whose
# screen has locked cannot run these tests at all. Behind the lock screen
# synthetic input, application activation and `screencapture -R <rect>` all
# fail, while a guest keeps its own unlocked session regardless.
#
#   scripts/native-tests-macos.sh                # full cycle, leaves the VM up
#   scripts/native-tests-macos.sh --stop         # ... and stop it afterwards
#   scripts/native-tests-macos.sh -- <filter>    # extra args for the test binary
#   SUITE=macos_hotkey_native scripts/native-tests-macos.sh   # the recorder tier
#
# SUITE picks which test binary to ship and run. `macos_hotkey_native` is also
# the *recorder* for tests/fixtures/keystreams: it presses real keys, writes
# down every press edge the app's key monitor saw, and step 6 copies those
# streams back to the host, where tests/hotkey_replay.rs replays them on every
# `cargo test`. That is the whole reason the fast tier is allowed to exist —
# its inputs came from here.
#
# First run needs a guest. The one this was built against:
#   tart clone ghcr.io/cirruslabs/macos-tahoe-base:latest nudge-base
# Those images already ship SIP disabled and TCC grants for ScreenCapture,
# Accessibility and PostEvent, so nothing has to be seeded — but step 3 checks
# rather than trusting. Keep a `tart clone` of a known-good guest as a golden
# image; the clone is instant and free on disk (APFS clonefile).
set -euo pipefail

VM=${VM:-nudge-base}
# Which test binary to ship and run. `macos_native` is the port's acceptance
# suite; `macos_hotkey_native` is the recorder tier, and it also writes the key
# streams that tests/hotkey_replay.rs replays on the host.
SUITE=${SUITE:-macos_native}
VM_USER=${VM_USER:-admin}
VM_PASS=${VM_PASS:-admin}
REPO=${REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
TART=${TART:-tart}
# Outside the repo on purpose: this is a private key, and a default inside the
# working tree is one `git add -A` away from being published.
KEY=${VM_KEY:-$HOME/.config/nudge-vm/id_ed25519}

STOP_AFTER=0
TEST_ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --stop) STOP_AFTER=1; shift ;;
    --) shift; TEST_ARGS=("$@"); break ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

say() { printf '\n== %s\n' "$*"; }

# --- 1. start the guest ----------------------------------------------------
# --no-graphics is enough: the guest still gets a virtual display, a
# WindowServer and an auto-logged-in console session. Verified, not assumed —
# step 3 proves it every run.
if ! $TART list | awk -v v="$VM" '$2==v && $NF=="running"{f=1} END{exit !f}'; then
  say "starting $VM"
  nohup $TART run "$VM" --no-graphics >/tmp/tart-$VM.log 2>&1 &
else
  say "$VM already running"
fi
IP=$($TART ip "$VM" --wait 120)
echo "guest ip: $IP"

SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
vs() { ssh -i "$KEY" "${SSH_OPTS[@]}" "$VM_USER@$IP" "$@"; }

# --- 2. key-based ssh, once ------------------------------------------------
# The guest's prompt is capital-P "Password:"; a lowercase expect pattern hangs
# until the timeout with no clue as to why.
mkdir -p "$(dirname "$KEY")"
[[ -f $KEY ]] || ssh-keygen -t ed25519 -N '' -f "$KEY" -C nudge-vm >/dev/null
if ! ssh -i "$KEY" "${SSH_OPTS[@]}" -o BatchMode=yes "$VM_USER@$IP" true 2>/dev/null; then
  say "installing ssh key"
  PUB=$(cat "$KEY.pub")
  expect -c "
    set timeout 60
    log_user 0
    spawn ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null $VM_USER@$IP \
      \"mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '$PUB' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys\"
    expect { -re {assword:} { send \"$VM_PASS\r\"; exp_continue } eof }
  "
  ssh -i "$KEY" "${SSH_OPTS[@]}" -o BatchMode=yes "$VM_USER@$IP" true
fi

# --- 3. wait for the desktop, then prove it is usable ----------------------
# sshd answers while the guest is still showing the boot progress bar, so a
# probe gated on ssh reachability screencaptures the Apple logo and reports a
# healthy session. Dock and Finder running is the signal that auto-login
# finished. A row in TCC.db is not evidence either; a capture that returns
# window pixels is.
say "waiting for the guest desktop"
vs '
  for i in $(seq 1 60); do
    pgrep -qx Dock && pgrep -qx Finder && exit 0
    sleep 2
  done
  echo "Dock/Finder never appeared — no Aqua session"; exit 1
'
say "checking the guest GUI session"
vs '
  set -e
  console=$(stat -f%Su /dev/console)
  [ "$console" = "'"$VM_USER"'" ] || { echo "no console session (owner=$console)"; exit 1; }
  pgrep -q WindowServer || { echo "no WindowServer"; exit 1; }
  ioreg -n Root -d1 -a | grep -q CGSSessionScreenIsLocked && { echo "guest screen is LOCKED"; exit 1; }
  rm -f /tmp/_gui_probe.png
  screencapture -x /tmp/_gui_probe.png
  size=$(stat -f%z /tmp/_gui_probe.png)
  [ "$size" -gt 500000 ] || { echo "screencapture produced only $size bytes — still booting?"; exit 1; }
  osascript -e "tell application \"System Events\" to key code 53" >/dev/null
  echo "console=$console  screencapture=${size}B  System Events keystroke: ok"
'

# --- 4. build on the host, ship the binaries -------------------------------
# The test resolves the app through env!("CARGO_MANIFEST_DIR"), i.e. an
# absolute HOST path. Recreating that exact path in the guest is simpler than
# sharing a directory: `tart run --dir` needs the VM restarted and lands the
# share under /Volumes/My Shared Files, where the hardcoded lookup would not
# find it. Both binaries link only system frameworks, so the guest needs no
# Rust toolchain and none is installed there.
say "building on the host"
( cd "$REPO" && cargo build --bin nudge --example control_backdrop && cargo test --no-run --test "$SUITE" )
TEST_BIN=$(cd "$REPO" && cargo test --no-run --test "$SUITE" --message-format=json 2>/dev/null \
  | SUITE="$SUITE" /usr/bin/python3 -c 'import sys, json, os
want = os.environ["SUITE"]
for l in sys.stdin:
    try: m = json.loads(l)
    except Exception: continue
    if m.get("target", {}).get("name") == want and m.get("executable"):
        print(m["executable"])' | tail -1)
[[ -n $TEST_BIN ]] || { echo "could not locate the $SUITE test binary" >&2; exit 1; }
echo "test binary: $TEST_BIN"

say "shipping binaries to $REPO inside the guest"
# The guest user differs from the host user, so the mirrored path lives under
# somebody else's /Users tree and has to be created with sudo, then handed over.
REPO_ROOT=$(printf '%s\n' "$REPO" | awk -F/ '{print "/"$2"/"$3}')
vs "sudo mkdir -p '$REPO/target/debug/deps' '$REPO/target/debug/examples' && sudo chown -R $VM_USER:staff '$REPO_ROOT'"
scp -q -i "$KEY" "${SSH_OPTS[@]}" "$REPO/target/debug/nudge" "$VM_USER@$IP:$REPO/target/debug/nudge"
# The appearance tests photograph the card over a known pattern rather than
# over whatever happens to be on the desktop; this is what puts it there.
scp -q -i "$KEY" "${SSH_OPTS[@]}" "$REPO/target/debug/examples/control_backdrop" \
  "$VM_USER@$IP:$REPO/target/debug/examples/control_backdrop"
scp -q -i "$KEY" "${SSH_OPTS[@]}" "$TEST_BIN" "$VM_USER@$IP:$REPO/target/debug/deps/"
GUEST_TEST="$REPO/target/debug/deps/$(basename "$TEST_BIN")"
vs "chmod +x '$REPO/target/debug/nudge' '$REPO/target/debug/examples/control_backdrop' '$GUEST_TEST'"

# --- 5. run the suite ------------------------------------------------------
# --test-threads=1 is load-bearing: the synthetic Esc is addressed by process
# name, so two live `nudge` instances make the keystroke land on the wrong one
# and the failure reads like a port regression rather than a harness collision.
# GUEST_ENV prefixes the run with environment for the guest. It exists for
# falsification passes — checking a new assertion goes red against a
# deliberately broken build, rather than trusting that it is green for the
# reason its name claims:
#   GUEST_ENV="NUDGE_X=1" scripts/native-tests-macos.sh -- some_test
GUEST_ENV=${GUEST_ENV:-}

# Carry the judge's key across by hand. The test binary falls back to the
# repo's .env when the variable is unset, but that file stays on the host —
# only the binaries are shipped — so without this the vision judge silently
# no-op-passes in the guest on a machine that is in fact configured. Passed as
# environment rather than copied in, so the key leaves nothing behind in the VM.
JUDGE_KEY=${OPENAI_API_KEY:-}
if [[ -z $JUDGE_KEY && -f $REPO/.env ]]; then
  JUDGE_KEY=$(sed -n 's/^OPENAI_API_KEY=//p' "$REPO/.env" | tr -d '"'\'' ' | head -1)
fi
if [[ -n $JUDGE_KEY ]]; then
  GUEST_ENV="OPENAI_API_KEY=$JUDGE_KEY $GUEST_ENV"
  echo "judge key: found (forwarding to the guest)"
else
  echo "judge key: none — the vision judge will skip"
fi

say "running the suite"
set +e
vs "$GUEST_ENV '$GUEST_TEST' --ignored --nocapture --test-threads=1 ${TEST_ARGS[*]:-} 2>&1"
RC=$?
set -e

# --- 6. bring the recorded key streams home --------------------------------
# The recorder tier writes one NDJSON stream per acceptance bullet, inside the
# guest, from keys the guest's own session delivered. Those streams are the
# input of the host-side replay tier, so they have to come back — without them
# the fast tier stays empty and its coverage is a promise rather than a fact.
# Collected even on failure: a stream recorded before the assertion that failed
# is still a real recording, and often the thing that explains the failure.
if vs "test -d '$REPO/tests/fixtures/keystreams'"; then
  say "collecting recorded key streams"
  mkdir -p "$REPO/tests/fixtures/keystreams"
  scp -q -i "$KEY" "${SSH_OPTS[@]}" \
    "$VM_USER@$IP:$REPO/tests/fixtures/keystreams/*.ndjson" \
    "$REPO/tests/fixtures/keystreams/" || echo "no streams to collect"
  ls -1 "$REPO/tests/fixtures/keystreams" | sed 's/^/  /'
fi

[[ $STOP_AFTER -eq 1 ]] && { say "stopping $VM"; $TART stop "$VM"; }
exit $RC
