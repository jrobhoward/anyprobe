#!/bin/sh
# Measures what the FreeBSD startup registration costs: generates programs
# with N probes each (one `fire` site and one is-enabled site per probe, in
# modules of 100), and prints the wall time of one run of each, as the best
# of three rounds. The program with 0 probes runs the same constructor,
# which finds no sites and returns, so the difference from it is the cost of
# parsing the site table, building the DOF and registering it, plus
# unregistering at exit. With DTrace loaded it also prints how long the
# ADDDOF and REMOVE ioctls take under ktrace, as the median of 9 runs.
# Prints everything; checks nothing.
#
# Usage: spike/scripts/startup-cost-freebsd.sh
# Environment:
#   STARTUP_COUNTS  probe counts, default "0 10 100 1000 3000".
#   STARTUP_RUNS    runs per round, default 400.
# Needs no root. Run it once with `dtraceall` loaded and once without
# (`kldunload dtraceall`, then every DTrace module) to separate the kernel's
# part from the program's.
set -u
cd "$(dirname "$0")/../.." || exit 2
repo=$PWD
counts=${STARTUP_COUNTS:-0 10 100 1000 3000}
runs=${STARTUP_RUNS:-400}
work=$(mktemp -d)
trap 'rm -rf "${work:?}"' EXIT

echo "=== freebsd-version / uname / CPU / vm_guest"
freebsd-version; uname -mr; sysctl -n hw.model; sysctl -n kern.vm_guest
if [ -e /dev/dtrace/helper ]; then loaded=yes; else loaded=no; fi
echo "dtrace loaded: $loaded"

# gen N DIR: a crate with N probes in modules of 100, each fired once.
gen() {
  mkdir -p "$2/src"
  cat >"$2/Cargo.toml" <<EOF
[package]
name = "startup$1"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
anyprobe = { path = "$repo/crates/anyprobe", default-features = false }
EOF
  awk -v n="$1" 'BEGIN {
    for (m = 0; m * 100 < n; m++) {
      print "mod m" m " {"
      print "    anyprobe::probes! {"
      print "        provider = \"startup\";"
      for (i = m * 100; i < n && i < (m + 1) * 100; i++)
        print "        pub fn p" i "(a: u64, b: u64);"
      print "    }"
      print "    #[inline(never)]"
      print "    pub fn fire(x: u64) {"
      for (i = m * 100; i < n && i < (m + 1) * 100; i++)
        print "        if p" i "::enabled() { p" i "::fire(x, " i "); }"
      print "    }"
      print "}"
    }
    print "fn main() {"
    if (n > 0) print "    let x = std::hint::black_box(1);"
    for (m = 0; m * 100 < n; m++) print "    m" m "::fire(x);"
    print "    std::hint::black_box(anyprobe::registration().is_ok());"
    print "}"
  }' >"$2/src/main.rs"
}

# Seconds for `runs` runs of BIN, best of three rounds.
best() {
  b=""
  for _ in 1 2 3; do
    /usr/bin/time -p -o "$work/time" sh -c "i=0; while [ \$i -lt $runs ]; do '$1' >/dev/null; i=\$((i + 1)); done"
    t=$(awk '/^real/ { print $2 }' "$work/time")
    if [ -z "$b" ] || [ "$(echo "$t < $b" | bc)" -eq 1 ]; then b=$t; fi
  done
  echo "$b"
}

# Median seconds between CALL and RET of the ioctl named $2 in BIN.
ioctl_median() {
  for _ in 1 2 3 4 5 6 7 8 9; do
    ktrace -f "$work/ktrace" -t c "$1" >/dev/null
    kdump -f "$work/ktrace" -R | awk -v name="$2" '
      index($0, name) { want = 1; next }
      want && / RET +ioctl/ { print $3; exit }'
  done | sort -n | sed -n 5p
}

echo "=== probes, ms per run (best of 3 rounds of $runs runs), extra over 0 probes"
base=""
for n in $counts; do
  gen "$n" "$work/s$n"
  if ! (cd "$work/s$n" && CARGO_TARGET_DIR="$work/target" cargo build -q --release); then
    echo "build of $n probes failed" >&2
    exit 2
  fi
  bin="$work/target/release/startup$n"
  s=$(best "$bin")
  ms=$(echo "scale=3; $s * 1000 / $runs" | bc)
  [ -n "$base" ] || base=$ms
  line="$n probes: $ms ms, +$(echo "scale=3; $ms - $base" | bc) ms"
  if [ "$loaded" = yes ] && [ "$n" -gt 0 ]; then
    line="$line; ADDDOF $(ioctl_median "$bin" DTRACEHIOC_ADDDOF) s, REMOVE $(ioctl_median "$bin" DTRACEHIOC_REMOVE) s"
  fi
  echo "$line"
done
