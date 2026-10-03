#!/bin/sh
# Attaches dtrace to the running spike and checks what it sees.
#
# Usage: spike/scripts/attach-freebsd.sh [PROFILE...]
#   PROFILE defaults to "release release-lto".
#
# Needs cargo, and root, sudo or doas for dtrace and `kldload dtraceall`.
# The spike runs as the invoking user, so a failure to register its probes
# also shows whether an unprivileged process may open /dev/dtrace/helper.
#
# For each profile it builds the spike, starts it, attaches dtrace to the
# process for three seconds, detaches, and checks that:
#   - the spike registered its DOF with the kernel;
#   - attaching turned the probe on inside the process (the is-enabled site),
#     and detaching turned it off;
#   - dtrace read the label from every work-entry site;
#   - every label fired as often as the others, and work-return fired once per
#     work-entry, so a site firing extra times shows up.
# It also reports, without failing, whether `dtrace -Z -c` picks the probes up
# when they register after the process starts.
# Prints ok/FAIL per check and exits non-zero if any check failed.

set -u
cd "$(dirname "$0")/../.." || exit 2
[ $# -gt 0 ] || set -- release release-lto

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
trap 'rm -rf "$work"' EXIT
failed=0

expect() {
  what=$1
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

# Count for KEY in dtrace's aggregation output.
count() { awk -v k="$1" '$1 == k { print $2 }' "$2"; }

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
  out="$work/$profile.dtrace"

  "$bin" 0 20 >"$log" 2>&1 &
  pid=$!
  sleep 2
  $sudo timeout 60 dtrace -q -p "$pid" -n '
    spike$target:::work-entry { @[copyinstr(arg1, arg2)] = count(); }
    spike$target:::work-return { @["work-return"] = count(); }
    tick-3s { exit(0); }' >"$out" 2>&1
  sleep 1
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null

  expect "spike registered its probes" sh -c "! grep -q register-error '$log'"
  expect "attach turned the probe on in the process" grep -q 'entry-enabled=true' "$log"
  expect "detach turned it off again" grep -q 'entry-enabled=false' "$log"
  entries=""
  for label in first-site second-site u32 u64; do
    n=$(count "$label" "$out")
    expect "read label '$label'" test -n "$n"
    entries="$entries ${n:-0}"
  done
  ret=$(count work-return "$out")
  ret=${ret:-0}
  total=$(printf '%s\n' $entries | awk '{ s += $1 } END { print s + 0 }')
  if [ "$ret" -gt "$total" ]; then diff=$((ret - total)); else diff=$((total - ret)); fi
  expect "every label fired equally often (within 1)" test "$(printf '%s\n' $entries | spread)" -le 1
  expect "one work-return per work-entry ($ret vs $total, within 4)" test "$diff" -le 4

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    echo "  --- spike output"
    cat "$log"
    echo "  --- dtrace output"
    cat "$out"
  fi

  # Informational: the provider appears only when main() registers it, after
  # dtrace has started the process. -Z keeps the enabling until it matches.
  zout="$work/$profile.dtrace-z"
  $sudo timeout 60 dtrace -q -Z -c "$bin 40 50" -n '
    spike$target:::work-entry { @[copyinstr(arg1, arg2)] = count(); }' >"$zout" 2>&1
  if [ "$(count first-site "$zout")" = 40 ]; then
    echo "  info  dtrace -Z -c saw all 40 iterations"
  else
    echo "  info  dtrace -Z -c did not see all 40 iterations:"
    sed 's/^/        /' "$zout"
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
