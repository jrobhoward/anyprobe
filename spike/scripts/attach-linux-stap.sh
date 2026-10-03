#!/usr/bin/env bash
# Attaches SystemTap to the running spike and checks what it sees.
#
# Usage: [STAP=/path/to/stap] spike/scripts/attach-linux-stap.sh [PROFILE...]
#   PROFILE defaults to "release release-lto". STAP defaults to `stap` on
#   PATH. SystemTap builds a kernel module per script, so it needs the
#   running kernel's headers and a release that supports that kernel
#   (Ubuntu 24.04's 5.0 does not build against 6.8; 5.6 does).
#
# Needs cargo, SystemTap, and root or sudo. For each profile it builds the
# spike, starts it, runs a SystemTap script against the process (`-x PID`)
# for three seconds, and checks the same things as attach-linux.sh:
#   - attaching turned the probe on inside the process (the semaphore), and
#     detaching turned it off;
#   - `user_string_n($arg2, $arg3)` read the label from every work__entry site;
#   - each iteration fired every label exactly once and work__return four
#     times, counted per iteration as described in attach-linux.sh.
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

stap=$(command -v "${STAP:-stap}") || { echo "stap not found; install it or set STAP" >&2; exit 2; }
if [ "$(id -u)" -eq 0 ]; then sudo=""; else sudo="sudo"; fi
scripts="$PWD/spike/scripts"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failed=0

# Prints the same map format as the bpftrace script, so per-iteration.awk
# reads either. @1 is the binary's absolute path.
cat >"$work/check.stp" <<'EOF'
global e, r
probe process(@1).mark("work__entry") {
  if (pid() != target()) next
  e[$arg1, user_string_n($arg2, $arg3)]++
}
probe process(@1).mark("work__return") {
  if (pid() != target()) next
  r[$arg1]++
}
probe timer.s(3) { exit() }
probe end {
  foreach ([i, l] in e) printf("@e[%d, %s]: %d\n", i, l, e[i, l])
  foreach (i in r) printf("@r[%d]: %d\n", i, r[i])
}
EOF

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
  if ! cargo build -q "${pkg_args[@]}" --profile "$profile"; then
    echo "  FAIL  build"
    failed=1
    continue
  fi
  bin="${CARGO_TARGET_DIR:-$PWD/target}/$profile/$bin_rel"
  log="$work/$profile.log"
  out="$work/$profile.stap"
  err="$work/$profile.stap-err"

  "$bin" 0 20 >"$log" 2>&1 &
  pid=$!
  sleep 1
  # Compiling the module takes a while; the spike keeps running meanwhile.
  $sudo timeout 300 "$stap" -x "$pid" "$work/check.stp" "$bin" >"$out" 2>"$err"
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
  expect "at least 100 complete iterations in a row (${full:-0})" test "${full:-0}" -ge 100

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    echo "  --- spike output"
    cat "$log"
    echo "  --- stap stderr"
    cat "$err"
    echo "  --- stap output (first 10 lines)"
    head -10 "$out"
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
