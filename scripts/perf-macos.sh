#!/usr/bin/env bash
#
# Run tests/perf_macos.rs against the release build and print its
# PERF_MACOS_JSON line(s): hidden/visible footprint, hidden CPU and wakeups,
# footprint after timer re-shows, show latency, IOSurface total.
#
#   scripts/perf-macos.sh            # one run
#   RUNS=3 scripts/perf-macos.sh     # three runs, one JSON line each
#
# Needs an unlocked, logged-in desktop: the test shows the popup and takes the
# keyboard away from it repeatedly for about three minutes per run. No TCC
# grant is needed. Each run's full output (including the `footprint` table) is
# kept in target/perf-macos/run-N.log.
set -euo pipefail

cd "$(dirname "$0")/.."

RUNS=${RUNS:-1}
LOG_DIR=target/perf-macos
mkdir -p "$LOG_DIR"

cargo build --release --bin nudge
cargo test --release --test perf_macos --no-run

RC=0
for i in $(seq 1 "$RUNS"); do
  printf '== run %s/%s\n' "$i" "$RUNS" >&2
  if ! cargo test --release --test perf_macos -- --ignored --nocapture --test-threads=1 \
      >"$LOG_DIR/run-$i.log" 2>&1; then
    RC=1
    grep -E 'panicked|assertion' -A3 "$LOG_DIR/run-$i.log" >&2 || tail -20 "$LOG_DIR/run-$i.log" >&2
  fi
done

grep -h '^PERF_MACOS_JSON ' $(seq -f "$LOG_DIR/run-%g.log" 1 "$RUNS") || {
  echo "no PERF_MACOS_JSON line in any run — see $LOG_DIR" >&2
  exit 1
}
exit $RC
