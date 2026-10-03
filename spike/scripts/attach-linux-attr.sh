#!/usr/bin/env bash
# Attaches bpftrace to the anyprobe `attr` example (`#[probe]`) and checks
# what it reads from each encoding.
#
# Usage: spike/scripts/attach-linux-attr.sh [PROFILE...]
#   PROFILE defaults to "release release-lto".
#
# Needs cargo, bpftrace, and root or sudo for bpftrace. For each profile it
# builds the example, starts it, attaches bpftrace for three seconds, and
# checks that:
#   - native arguments and a native return value read as written;
#   - a `serde` argument reads as its JSON, and as the same text when read
#     as a NUL-terminated string (what perf and gdb do);
#   - a `debug` return value reads as its `{:?}` output;
#   - arguments collapsed into one object read as that JSON object;
#   - `debug(self)` on a method reads as the receiver's `{:?}` output.
# Encoding runs only after the enabled check, so any encoded value read
# shows that attaching turned the check on.
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

for profile in "$@"; do
  echo "== $profile"
  profile_failed=0
  if ! cargo build -q -p anyprobe --example attr --profile "$profile"; then
    echo "  FAIL  build"
    failed=1
    continue
  fi
  bin="$PWD/target/$profile/examples/attr"
  log="$work/$profile.log"
  out="$work/$profile.bpftrace"

  "$bin" 0 20 >"$log" 2>&1 &
  pid=$!
  sleep 1
  $sudo timeout 60 bpftrace -p "$pid" -e "
    usdt:$bin:attr:lookup__entry { printf(\"lookup-entry %d %s\n\", arg0, str(arg1, arg2)); }
    usdt:$bin:attr:lookup__return { printf(\"lookup-return %d\n\", arg0); }
    usdt:$bin:attr:query__entry {
      printf(\"query-entry %d %s\n\", arg0, str(arg1, arg2));
      printf(\"query-entry-nul %s\n\", str(arg1));
    }
    usdt:$bin:attr:query__return { printf(\"query-return %s\n\", str(arg0, arg1)); }
    usdt:$bin:attr:wide__entry { printf(\"wide-entry %s\n\", str(arg0, arg1)); }
    usdt:$bin:attr:counter_bump__entry { printf(\"bump-entry %s %d\n\", str(arg0, arg1), arg2); }
    interval:s:3 { exit(); }" >"$out" 2>&1
  sleep 1
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null

  expect "native arguments: id and path" grep -qE '^lookup-entry [0-9]+ /index$' "$out"
  expect "native return value" grep -qE '^lookup-return [0-9]*6$' "$out"
  expect "serde argument as JSON" \
    grep -qE '^query-entry [0-9]+ \{"table":"rows","limit":5\}$' "$out"
  expect "serde argument read up to its NUL" \
    grep -qE '^query-entry-nul \{"table":"rows","limit":5\}$' "$out"
  expect "debug return value, Ok" grep -qE '^query-return Ok\(5\)$' "$out"
  expect "debug return value, Err" grep -qE '^query-return Err\("odd [0-9]+"\)$' "$out"
  expect "collapsed arguments as one JSON object" \
    grep -qE '^wide-entry \{"id":[0-9]+,"a":"x","b":"y","c":"z","tags":"\[\\"t\\"\]"\}$' "$out"
  expect "debug(self) and a native argument" \
    grep -qE '^bump-entry Counter \{ n: [0-9]+ \} 1$' "$out"
  # Every printed line is one of the formats above.
  expect "nothing unexpected" \
    test -z "$(grep -vE '^(lookup-entry|lookup-return|query-entry|query-entry-nul|query-return|wide-entry|bump-entry) |^Attaching |^$' "$out")"

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    echo "  --- example output"
    cat "$log"
    echo "  --- bpftrace output (first 40 lines)"
    head -40 "$out"
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
