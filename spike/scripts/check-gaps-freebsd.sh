#!/bin/sh
# Checks a runtime claim in docs/GAPS.md on FreeBSD: a killed tracer leaves
# nothing behind ("What a tracer writes into the process").
#
# dtrace enables the `work` example's work-entry probe by its pid-qualified
# name (`spike<PID>:::work-entry`, no `-p`) and is killed with SIGKILL.
# While it is attached, the program reports the probe on and some sites
# start with a breakpoint (0xcc); after the kill, the program reports it off
# and no site starts with 0xcc. A second dtrace then attaches and detaches
# normally, so nothing stale is left in the kernel either.
#
# Then the same with `dtrace -p`, which holds the process with ptrace for
# the whole session: with `kern.kill_on_debugger_exit=1`, the default, the
# kernel kills a traced process whose tracer exits without detaching, so
# killing that dtrace kills the program as well. The check expects that
# whenever the sysctl is 1.
#
# The site addresses come from the `R_X86_64_RELATIVE` relocations inside
# `anyprobe_sites`: the only pointer in each record is its site. The bytes
# are read with lldb, which attaches to the program for a moment.
#
# Usage: spike/scripts/check-gaps-freebsd.sh
# Needs cargo, lldb (base system), and root, sudo or doas for dtrace. Run as
# yourself. Prints ok/FAIL per check and exits non-zero if any check failed.

set -u
cd "$(dirname "$0")/../.." || exit 2
if [ "$(id -u)" -eq 0 ]; then
  sudo=""
elif command -v sudo >/dev/null; then
  sudo="sudo"
elif command -v doas >/dev/null; then
  sudo="doas"
else
  echo "run as root, or install sudo or doas: dtrace needs root" >&2
  exit 2
fi
$sudo kldload -n dtraceall || exit 2

work=$(mktemp -d)
pid=""
trap 'kill $pid 2>/dev/null; rm -rf "$work"' EXIT
failed=0

expect() {
  check=$1
  shift
  if "$@"; then
    echo "  ok    $check"
  else
    echo "  FAIL  $check"
    failed=1
  fi
}

# Polls until the program's log shows `entry-enabled=WANT` after line FROM.
wait_enabled() {
  for _ in $(seq 50); do
    tail -n +"$2" "$log" | grep -q "entry-enabled=$1" && return 0
    sleep 0.1
  done
  return 1
}

# How many sites currently start with 0xcc.
traps() {
  set --
  for a in $sites; do set -- "$@" -o "memory read -s1 -fx -c1 $(printf '0x%x' $((base + a)))"; done
  lldb -p "$pid" --batch "$@" 2>/dev/null | grep -cE '^0x[0-9a-f]+: 0xcc$'
}

cargo build -q --release -p anyprobe --example work || exit 2
bin="$PWD/target/release/examples/work"
set -- $(readelf -SW "$bin" | awk '/ anyprobe_sites / { print $4, $6 }')
start=$((0x$1))
end=$((0x$1 + 0x$2))
sites=$(readelf -rW "$bin" | awk '$3 == "R_X86_64_RELATIVE" { print $1, $4 }' |
  while read -r off add; do
    [ $((0x$off)) -ge "$start" ] && [ $((0x$off)) -lt "$end" ] && echo "0x$add"
  done)
nsites=$(echo "$sites" | wc -w | tr -d ' ')

echo "== killed tracer (anyprobe example work, $nsites sites)"
log="$work/work.log"
"$bin" 0 20 >"$log" 2>&1 &
pid=$!
sleep 1
# The start of the first mapping, already written with its 0x.
base=$(procstat -v "$pid" | awk -v f="$(realpath "$bin")" '$NF == f { print $2; exit }')
expect "before: no site starts with a breakpoint" test "$(traps)" -eq 0

from=$(($(wc -l <"$log") + 1))
$sudo dtrace -q -n "spike$pid:::work-entry { @n = count(); }" >"$work/dtrace.out" 2>&1 &
wait_enabled true "$from"
expect "dtrace attached: the program reports the probe on" test $? -eq 0
attached=$(traps)
expect "dtrace attached: $attached sites start with a breakpoint" test "$attached" -gt 0
# `sudo`/`doas` does not pass SIGKILL on, so kill dtrace itself.
$sudo kill -9 "$(pgrep -n -x dtrace)"
from=$(($(wc -l <"$log") + 1))
wait_enabled false "$from"
expect "dtrace killed: the program reports the probe off" test $? -eq 0
sleep 1
expect "dtrace killed: no site starts with a breakpoint" test "$(traps)" -eq 0

from=$(($(wc -l <"$log") + 1))
$sudo timeout 20 dtrace -q -n "spike$pid:::work-entry { @n = count(); } tick-1s { exit(0); }" \
  >"$work/dtrace2.out" 2>&1
expect "a second dtrace attaches and detaches normally" sh -c "
  tail -n +$from '$log' | grep -q 'entry-enabled=true' &&
  tail -n +$from '$log' | grep -q 'entry-enabled=false' &&
  grep -qE '^ +[0-9]+\$' '$work/dtrace2.out'"
expect "after it: no site starts with a breakpoint" test "$(traps)" -eq 0

echo "== killed dtrace -p"
if [ "$(sysctl -n kern.kill_on_debugger_exit)" = 1 ]; then
  from=$(($(wc -l <"$log") + 1))
  $sudo dtrace -q -p "$pid" -n 'spike$target:::work-entry { @n = count(); }' >"$work/dtrace3.out" 2>&1 &
  wait_enabled true "$from"
  expect "dtrace -p attached: the program reports the probe on" test $? -eq 0
  $sudo kill -9 "$(pgrep -n -x dtrace)"
  sleep 1
  wait "$pid" 2>/dev/null
  status=$?
  expect "dtrace -p killed: the kernel killed the program too (status $status, kern.kill_on_debugger_exit=1)" \
    test "$status" -eq 137
  pid=""
else
  echo "  skip  kern.kill_on_debugger_exit is not 1"
fi

if [ "$failed" -ne 0 ]; then
  for f in work.log dtrace.out dtrace2.out dtrace3.out; do
    [ -f "$work/$f" ] && { echo "  --- $f (last 20 lines)"; tail -20 "$work/$f"; }
  done
  echo FAIL
else
  echo PASS
fi
exit "$failed"
