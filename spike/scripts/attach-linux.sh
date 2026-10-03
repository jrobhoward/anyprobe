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
#   - every label fired as often as the others, and work__return fired once
#     per work__entry, so a site firing extra times shows up.
# Prints ok/FAIL per check and exits non-zero if any check failed.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
[ $# -gt 0 ] || set -- release release-lto

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

# Largest minus smallest of the numbers on stdin.
spread() { sort -n | awk 'NR == 1 { min = $1 } { max = $1 } END { print max - min }'; }

for profile in "$@"; do
  echo "== $profile"
  profile_failed=0
  if ! cargo build -q -p anyprobe-spike --profile "$profile"; then
    echo "  FAIL  build"
    failed=1
    continue
  fi
  bin="$PWD/target/$profile/anyprobe-spike"
  log="$work/$profile.log"
  out="$work/$profile.bpftrace"

  "$bin" 0 20 >"$log" 2>&1 &
  pid=$!
  sleep 1
  $sudo timeout 60 bpftrace -p "$pid" -e "
    usdt:$bin:spike:work__entry { @entry[str(arg1, arg2)] = count(); }
    usdt:$bin:spike:work__return { @ret = count(); }
    interval:s:3 { exit(); }" >"$out" 2>&1
  sleep 1
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null

  expect "attach turned the probe on in the process" grep -q 'entry-enabled=true' "$log"
  expect "detach turned it off again" grep -q 'entry-enabled=false' "$log"
  for label in first-site second-site u32 u64; do
    expect "read label '$label'" grep -q "^@entry\[$label\]: " "$out"
  done
  entries=$(sed -nE 's/^@entry\[.*\]: ([0-9]+)$/\1/p' "$out")
  ret=$(sed -nE 's/^@ret: ([0-9]+)$/\1/p' "$out")
  ret=${ret:-0}
  total=$(printf '%s\n' $entries | awk '{ s += $1 } END { print s + 0 }')
  diff=$((ret > total ? ret - total : total - ret))
  expect "every label fired equally often (within 1)" test "$(printf '%s\n' $entries | spread)" -le 1
  expect "one work__return per work__entry ($ret vs $total, within 4)" test "$diff" -le 4

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
