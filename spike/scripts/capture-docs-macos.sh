#!/usr/bin/env bash
# Captures the output shown in docs/usage/macos.md and the attached-cost
# numbers in docs/PERFORMANCE.md: the `demo` example under dtrace (probe
# list, a one-liner, the script `cargo anyprobe dtrace` writes) and the
# `overhead` example with no tracer, with dtrace counting every firing, and
# with dtrace printing every argument. Prints everything; checks nothing.
#
# Usage: spike/scripts/capture-docs-macos.sh
# Run as yourself, not under sudo: it calls sudo for dtrace only.
set -u
cd "$(dirname "$0")/../.." || exit 2
cargo build -q --release -p anyprobe --example demo --example overhead || exit 2
cargo build -q --release -p cargo-anyprobe || exit 2
demo=target/release/examples/demo
over=target/release/examples/overhead
cli=target/release/cargo-anyprobe
out=$(mktemp -d)
sudo -v || exit 2

echo "=== sw_vers / dtrace -V"
sw_vers; sysctl -n machdep.cpu.brand_string; dtrace -V 2>&1

echo "=== demo started"
"$demo" >"$out/demo.out" 2>&1 &
pid=$!
sleep 1.5

echo "=== dtrace -l (probes in the running demo)"
sudo dtrace -l -p "$pid" -n 'demo$target:::' 2>&1

echo "=== one-liner, -q, 3 s"
sudo dtrace -q -p "$pid" -n '
  demo$target:::tick { printf("tick %d\n", arg0); }
  demo$target:::checkout-entry {
    printf("checkout id=%d customer=%s order=%s\n", arg0, copyinstr(arg1, arg2), copyinstr(arg3, arg4));
  }
  demo$target:::checkout-return { printf("checkout returned %d\n", arg0); }
  tick-3s { exit(0); }' 2>&1

echo "=== same, without -q, 2 s"
sudo dtrace -p "$pid" -n 'demo$target:::checkout-return { printf("%d", arg0); }' -n 'tick-2s { exit(0); }' 2>&1

echo "=== cargo anyprobe dtrace"
"$cli" dtrace "$demo" >"$out/demo.d"
cat "$out/demo.d"
echo "=== generated script, 3 s"
sudo dtrace -p "$pid" -s "$out/demo.d" -n 'tick-3s { exit(0); }' 2>&1

kill "$pid"
echo "=== demo's own output"
cat "$out/demo.out"

echo "=== overhead, no tracer"
"$over" 1000000
echo "=== overhead, dtrace counting every probe"
sudo dtrace -q -c "$over 1000000" -n 'overhead$target:::* { @[probename] = count(); }' 2>&1
echo "=== overhead, dtrace with printf of every argument (output discarded)"
sudo dtrace -q -c "$over 1000000" -n '
  overhead$target:::native-entry { printf("%d %s\n", arg0, copyinstr(arg1, arg2)); }
  overhead$target:::encoded-entry { printf("%d %s\n", arg0, copyinstr(arg1, arg2)); }
  overhead$target:::native-return, overhead$target:::encoded-return { printf("%d\n", arg0); }' \
  2>&1 | grep -E '^(pid=|baseline|native |encoded |dtrace)'
rm -rf "$out"
