#!/usr/bin/env bash
# Checks the macOS DTrace backend: the DOF ld64 built, then a real dtrace
# attach.
#
# Usage: spike/scripts/attach-macos.sh [PROFILE...]
#   PROFILE defaults to "release release-lto".
# Environment:
#   SPIKE_TARGET  build for this target (e.g. x86_64-apple-darwin) instead of
#                 the host. The attach step is skipped for a non-host target.
#   SPIKE_ATTACH  set to 0 to skip the dtrace attach (GitHub's runners have
#                 SIP on and may refuse it).
#
# For each profile it checks that:
#   - every probe site was rewritten by ld64 and none ends its function
#     (examples/inspect_dof.rs; a tail call there falls through once
#     rewritten);
#   - work__entry has 3 probe sites and work__return 1;
#   - no call to a __dtrace_ symbol survived linking;
#   - under `sudo dtrace -c`, 40 iterations fire each label exactly 40 times
#     and work__return exactly 160 times.
# Works with SIP on for this binary. Prints ok/FAIL per check and exits
# non-zero if any check failed.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
[ $# -gt 0 ] || set -- release release-lto

target=${SPIKE_TARGET:-}
attach=${SPIKE_ATTACH:-1}
host="$(uname -m | sed 's/arm64/aarch64/')-apple-darwin"
if [ -n "$target" ] && [ "$target" != "$host" ]; then
  attach=0
fi

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

# Sum of the `sites=N` fields for one probe name in inspect_dof output.
sites() { grep ":$1 " "$2" | sed -E 's/.* sites=([0-9]+) .*/\1/' | awk '{ s += $1 } END { print s + 0 }'; }

# Count for KEY in dtrace's aggregation output.
count() { awk -v k="$1" '$1 == k { print $2 }' "$2"; }

cargo build -q -p anyprobe-spike --example inspect_dof || exit 2
inspect="target/debug/examples/inspect_dof"

for profile in "$@"; do
  echo "== ${target:-$host} $profile"
  profile_failed=0
  if [ -n "$target" ]; then
    build=(cargo build -q -p anyprobe-spike --profile "$profile" --target "$target")
    bin="target/$target/$profile/anyprobe-spike"
  else
    build=(cargo build -q -p anyprobe-spike --profile "$profile")
    bin="target/$profile/anyprobe-spike"
  fi
  if ! "${build[@]}"; then
    echo "  FAIL  build"
    failed=1
    continue
  fi
  dof="$work/$profile.dof"
  out="$work/$profile.dtrace"

  "$inspect" "$bin" >"$dof" 2>&1
  inspected=$?
  expect "every site rewritten and inside its function" test "$inspected" -eq 0
  expect "3 work__entry probe sites" test "$(sites work-entry "$dof")" -eq 3
  expect "1 work__return probe site" test "$(sites work-return "$dof")" -eq 1
  expect "no __dtrace_ call survived linking" bash -c "! otool -tV '$bin' | grep -q ___dtrace_"

  if [ "$attach" != 0 ]; then
    sudo dtrace -q -c "$bin 40 50" -n '
      spike$target:::work-entry { @[copyinstr(arg1, arg2)] = count(); }
      spike$target:::work-return { @["work-return"] = count(); }' >"$out" 2>&1
    for label in first-site second-site u32 u64; do
      expect "dtrace: '$label' fired 40 times" test "$(count "$label" "$out")" = 40
    done
    expect "dtrace: work-return fired 160 times" test "$(count work-return "$out")" = 160
  fi

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    echo "  --- inspect_dof output"
    cat "$dof"
    [ -f "$out" ] && { echo "  --- dtrace output"; cat "$out"; }
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
