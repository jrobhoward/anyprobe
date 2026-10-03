#!/usr/bin/env bash
# Attaches bpftrace to the running spike and checks what it sees.
#
# Usage: spike/scripts/attach-linux.sh [PROFILE...]
#   PROFILE defaults to "release release-lto".
#
# Needs cargo, bpftrace, and root or sudo for bpftrace. For each profile it
# builds the spike, starts it, attaches bpftrace to the process for three
# seconds, detaches, and checks that:
#   - attaching turned the probe on inside the process (the semaphore), and
#     detaching turned it off;
#   - bpftrace read the label from every work__entry site;
#   - each iteration fired every label exactly once and work__return four
#     times, so a site firing extra times, or not at all, shows up.
#
# Counts are per iteration (arg0 is the iteration number for every call in
# it), not totals. bpftrace attaches one uprobe at a time, so the sites go
# live a few iterations apart and totals skew by that much at both ends of
# the window. The check takes the iterations that every site saw in full,
# requires them to be one unbroken run of at least 100, and allows partial
# iterations only before or after that run.
# Prints ok/FAIL per check and exits non-zero if any check failed.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
[ $# -gt 0 ] || set -- release release-lto

# ATTACH_CRATE=anyprobe checks the anyprobe crate's `work` example, which has
# the spike's provider, probes and command line; the default checks the spike.
if [ "${ATTACH_CRATE:-spike}" = anyprobe ]; then
  pkg_args=(-p anyprobe --example work)
  bin_rel=examples/work
else
  pkg_args=(-p anyprobe-spike)
  bin_rel=anyprobe-spike
fi

command -v bpftrace >/dev/null || { echo "bpftrace not found; install it first" >&2; exit 2; }
if [ "$(id -u)" -eq 0 ]; then sudo=""; else sudo="sudo"; fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failed=0

expect() {
  local what=$1
  shift
  if "$@"; then
    echo "  ok    $what"
  else
    echo "  FAIL  $what"
    profile_failed=1
  fi
}

scripts="$PWD/spike/scripts"

for profile in "$@"; do
  echo "== $profile"
  profile_failed=0
  if ! cargo build -q "${pkg_args[@]}" --profile "$profile"; then
    echo "  FAIL  build"
    failed=1
    continue
  fi
  bin="$PWD/target/$profile/$bin_rel"
  log="$work/$profile.log"
  out="$work/$profile.bpftrace"

  "$bin" 0 20 >"$log" 2>&1 &
  pid=$!
  sleep 1
  $sudo timeout 60 bpftrace -p "$pid" -e "
    usdt:$bin:spike:work__entry { @e[arg0, str(arg1, arg2)] = count(); }
    usdt:$bin:spike:work__return { @r[arg0] = count(); }
    interval:s:3 { exit(); }" >"$out" 2>&1
  sleep 1
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null

  expect "attach turned the probe on in the process" grep -q 'entry-enabled=true' "$log"
  expect "detach turned it off again" grep -q 'entry-enabled=false' "$log"
  for label in first-site second-site u32 u64; do
    expect "read label '$label'" grep -qE "^@e\[[0-9]+, $label\]: " "$out"
  done
  report=$(awk -f "$scripts/per-iteration.awk" "$out")
  problems=$(printf '%s\n' "$report" | grep -v '^full=')
  full=$(printf '%s\n' "$report" | sed -n 's/^full=//p')
  expect "every iteration fired each label once and work__return 4 times" test -z "$problems"
  [ -z "$problems" ] || printf '%s\n' "$problems" | head -20 | sed 's/^/        /'
  expect "at least 100 complete iterations in a row ($full)" test "${full:-0}" -ge 100

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    echo "  --- spike output"
    cat "$log"
    echo "  --- bpftrace output"
    cat "$out"
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
