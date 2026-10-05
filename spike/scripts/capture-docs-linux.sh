#!/usr/bin/env bash
# Captures the output shown in docs/usage/linux.md and the attached-cost
# numbers in docs/PERFORMANCE.md: the `demo` example under bpftrace (probe
# list, a one-liner, the script `cargo anyprobe bpftrace` writes) and under
# perf, and the `overhead` example with no tracer, with bpftrace counting
# every firing, with bpftrace printing every argument, and with perf
# recording every firing. Prints everything; checks nothing.
#
# Usage: spike/scripts/capture-docs-linux.sh [CALLS]
#   CALLS is the call count for the traced `overhead` runs, default 1000000.
#   The untraced run always uses 1000000, as the macOS capture does.
# Run as yourself, not under sudo: it calls sudo for bpftrace and perf only.
set -u
cd "$(dirname "$0")/../.." || exit 2
calls=${1:-1000000}
cargo build -q --release -p anyprobe --example demo --example overhead || exit 2
cargo build -q --release -p cargo-anyprobe || exit 2
demo=$PWD/target/release/examples/demo
over=$PWD/target/release/examples/overhead
cli=$PWD/target/release/cargo-anyprobe
out=$(mktemp -d)
# `sudo true` rather than `sudo -v`, which sudo-rs asks for a password even
# under NOPASSWD.
sudo true || exit 2
cleanup() {
  sudo perf probe -q -d 'sdt_demo:*' >/dev/null 2>&1
  sudo perf probe -q -d 'sdt_overhead:*' >/dev/null 2>&1
  sudo rm -rf "$out"
}
trap cleanup EXIT

echo "=== uname / os-release / cpu / governor / tools"
uname -srm
grep PRETTY_NAME /etc/os-release
grep -m1 'model name' /proc/cpuinfo
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null
bpftrace --version; perf --version; rustc -V

echo "=== demo started"
"$demo" >"$out/demo.out" 2>&1 &
pid=$!
sleep 1.5
head -2 "$out/demo.out"

echo "=== cargo anyprobe list"
"$cli" list "$demo"

echo "=== bpftrace -l"
sudo bpftrace -l "usdt:$demo:*" 2>&1

echo "=== one-liner, 3 s"
sudo bpftrace -p "$pid" -e "
  usdt:*:demo:tick { printf(\"tick %d\n\", arg0); }
  usdt:*:demo:checkout__entry {
    printf(\"checkout id=%d customer=%r order=%s\n\",
           arg0, buf(arg1, arg2), str(arg3, arg4 + 1));
  }
  usdt:*:demo:checkout__return { printf(\"checkout returned %d\n\", arg0); }
  interval:s:3 { exit(); }" 2>&1

echo "=== cargo anyprobe bpftrace"
"$cli" bpftrace "$demo" >"$out/demo.bt"
cat "$out/demo.bt"
echo "=== generated script, 3 s"
# The exit clause goes in a second file: bpftrace takes one program, so it is
# appended to a copy.
cp "$out/demo.bt" "$out/demo-exit.bt"
printf '\ninterval:s:3 { exit(); }\n' >>"$out/demo-exit.bt"
sudo bpftrace -p "$pid" "$out/demo-exit.bt" 2>&1

echo "=== perf probe / record / script, 3 s"
sudo perf probe -q -d 'sdt_demo:*' >/dev/null 2>&1
sudo perf buildid-cache --add "$demo" 2>&1
sudo perf probe -x "$demo" -a '%sdt_demo:tick' -a '%sdt_demo:checkout__return' 2>&1
sudo perf record -e 'sdt_demo:*' -p "$pid" -o "$out/demo.data" -- sleep 3 2>&1
sudo perf script -i "$out/demo.data" 2>&1
sudo perf probe -d 'sdt_demo:*' 2>&1

kill "$pid"
wait "$pid" 2>/dev/null
echo "=== demo's own output"
cat "$out/demo.out"

echo "=== overhead, no tracer"
"$over" 1000000
echo "=== overhead, bpftrace counting every probe, $calls calls"
sudo bpftrace -c "$over $calls" \
  -e "usdt:$over:overhead:* { @[probe] = count(); }" 2>&1
echo "=== overhead, bpftrace with printf of every argument, $calls calls (output discarded)"
sudo bpftrace -c "$over $calls" -e "
  usdt:$over:overhead:native__entry { printf(\"%d %r\n\", arg0, buf(arg1, arg2)); }
  usdt:$over:overhead:encoded__entry { printf(\"%d %s\n\", arg0, str(arg1, arg2 + 1)); }
  usdt:$over:overhead:native__return, usdt:$over:overhead:encoded__return { printf(\"%d\n\", arg0); }" \
  2>&1 | grep -E '^(Attach|pid=|baseline|native |encoded |Lost|WARNING|ERROR)'
echo "=== overhead, perf record of every probe, $calls calls"
sudo perf probe -q -d 'sdt_overhead:*' >/dev/null 2>&1
sudo perf buildid-cache --add "$over" 2>&1
sudo perf probe -q -x "$over" -a '%sdt_overhead:native__entry' -a '%sdt_overhead:native__return' \
  -a '%sdt_overhead:encoded__entry' -a '%sdt_overhead:encoded__return' 2>&1
sudo perf record -e 'sdt_overhead:*' -o "$out/overhead.data" -- "$over" "$calls" 2>&1
ls -l "$out/overhead.data"
echo "events recorded (expected $((8 * calls))):"
sudo perf script -i "$out/overhead.data" -F event 2>/dev/null | sort | uniq -c
sudo perf probe -d 'sdt_overhead:*' 2>&1
