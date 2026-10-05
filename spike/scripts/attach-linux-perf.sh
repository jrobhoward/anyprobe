#!/usr/bin/env bash
# Attaches perf to the running spike and checks what it sees.
#
# Usage: spike/scripts/attach-linux-perf.sh [PROFILE...]
#   PROFILE defaults to "release release-lto".
#
# Needs cargo, perf, and root or sudo for perf. For each profile it builds the
# spike, starts it, adds the SDT probes with `perf probe`, records them for
# three seconds with `perf record -p`, removes the probes, and checks that:
#   - recording turned the probe on inside the process (perf passes the
#     semaphore to the kernel as the uprobe's reference counter), and ending
#     the recording turned it off;
#   - each iteration fired work__entry 4 times and work__return 4 times, with
#     partial iterations only before or after one unbroken run of at least
#     100 (see attach-linux.sh for why counts are per iteration).
# perf reads arguments as integers only, so labels are not checked here;
# attach-linux.sh checks them with bpftrace.
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

command -v perf >/dev/null || { echo "perf not found; install it first" >&2; exit 2; }
if [ "$(id -u)" -eq 0 ]; then sudo=""; else sudo="sudo"; fi

work=$(mktemp -d)
cleanup() {
  $sudo perf probe -q -d 'sdt_spike:*' >/dev/null 2>&1
  $sudo rm -rf "$work"
}
trap cleanup EXIT
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

# Reads `perf script` output on stdin and prints one line per iteration that
# broke the rules, then "full=<count of complete iterations>". Every site of a
# probe gets its own event (work__entry, work__entry_1, ...; perf 7.0 writes
# work_entry, work_entry_1, ...); arg1 is the iteration number.
per_iteration() {
  awk '
    / sdt_spike:work__?(entry|return)(_[0-9]+)?: / {
      kind = ($0 ~ / sdt_spike:work__?entry/) ? "entry" : "return"
      id = ""
      for (f = 1; f <= NF; f++) if ($f ~ /^arg1=/) { id = substr($f, 6); break }
      if (id == "") next
      ids[id] = 1
      if (kind == "entry") e[id]++; else r[id]++
    }
    END {
      lo = -1; hi = -1; full = 0
      for (i in ids) {
        if (e[i] > 4) print "iteration " i ": work__entry fired " e[i] " times"
        if (r[i] > 4) print "iteration " i ": work__return fired " r[i] " times"
        if (e[i] == 4 && r[i] == 4) {
          full++
          if (lo < 0 || i + 0 < lo) lo = i + 0
          if (hi < 0 || i + 0 > hi) hi = i + 0
        }
      }
      if (full > 0 && hi - lo + 1 != full)
        print "complete iterations " lo ".." hi " have gaps (" full " complete)"
      for (i in ids) if (!(e[i] == 4 && r[i] == 4) && i + 0 > lo && i + 0 < hi)
        print "iteration " i " is partial inside the complete run"
      print "full=" full
    }'
}

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
  probe_out="$work/$profile.probe"
  data="$work/$profile.data"
  out="$work/$profile.script"

  $sudo perf probe -q -d 'sdt_spike:*' >/dev/null 2>&1
  $sudo perf buildid-cache --add "$bin" >"$probe_out" 2>&1
  $sudo perf probe -x "$bin" -a '%sdt_spike:work__entry' -a '%sdt_spike:work__return' \
    >>"$probe_out" 2>&1
  expect "perf probe added the SDT events" grep -qE 'sdt_spike:work__?entry' "$probe_out"

  "$bin" 0 20 >"$log" 2>&1 &
  pid=$!
  sleep 1
  $sudo timeout 60 perf record -q -e 'sdt_spike:*' -p "$pid" -o "$data" -- sleep 3 \
    >>"$probe_out" 2>&1
  sleep 1
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  $sudo perf script -i "$data" >"$out" 2>>"$probe_out"
  $sudo perf probe -q -d 'sdt_spike:*' >/dev/null 2>&1

  expect "recording turned the probe on in the process" grep -q 'entry-enabled=true' "$log"
  expect "ending the recording turned it off again" grep -q 'entry-enabled=false' "$log"
  report=$(per_iteration <"$out")
  problems=$(printf '%s\n' "$report" | grep -v '^full=')
  full=$(printf '%s\n' "$report" | sed -n 's/^full=//p')
  expect "every iteration fired work__entry and work__return 4 times each" test -z "$problems"
  [ -z "$problems" ] || printf '%s\n' "$problems" | head -20 | sed 's/^/        /'
  expect "at least 100 complete iterations in a row (${full:-0})" test "${full:-0}" -ge 100

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    echo "  --- spike output"
    cat "$log"
    echo "  --- perf probe/record output"
    cat "$probe_out"
    echo "  --- perf script output (first 10 lines)"
    head -10 "$out"
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
