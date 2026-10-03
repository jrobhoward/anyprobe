#!/usr/bin/env bash
# Checks `#[probe]` on macOS: the DOF ld64 built for the anyprobe `attr` and
# `same_name` examples, then what dtrace reads from each encoding.
#
# Usage: spike/scripts/attach-macos-attr.sh [PROFILE...]
#   PROFILE defaults to "release release-lto".
# Environment:
#   SPIKE_TARGET  build for this target (e.g. x86_64-apple-darwin) instead of
#                 the host. The attach step is skipped for a non-host target.
#   SPIKE_ATTACH  set to 0 to skip the dtrace attach.
#
# For each profile it checks that:
#   - every probe site in both examples was rewritten by ld64 and none ends
#     its function (examples/inspect_dof.rs);
#   - `same_name`'s three `new__entry` probes, which have different argument
#     types, each get their own DOF entry;
#   - under `sudo dtrace -c` with 20 iterations, exactly 20 times each:
#     native arguments and a native return value read as written; a `serde`
#     argument reads as its JSON, and as the same text when read up to its
#     NUL; arguments collapsed into one object read as that JSON object;
#     `debug(self)` reads as the receiver's `{:?}` output. A `debug` return
#     value reads as `Ok(5)` 10 times and `Err("odd N")` 10 times;
#   - for `same_name`, dtrace reads each function's arguments as that
#     function declares them, 20 times each, and `new__return` fires 60
#     times.
# Encoding runs only after the enabled check, so any encoded value read
# shows that attaching turned the check on. Works with SIP on. Prints ok/FAIL
# per check and exits non-zero if any check failed.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
[ $# -gt 0 ] || set -- release release-lto

target=${SPIKE_TARGET:-}
attach=${SPIKE_ATTACH:-1}
host="$(uname -m | sed 's/arm64/aarch64/')-apple-darwin"
if [ -n "$target" ] && [ "$target" != "$host" ]; then
  attach=0
fi
iterations=20

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

# dtrace's own notice when SIP is on; the only line dtrace itself may add.
sip='^dtrace: system integrity protection is on, some features will not be available$'

# Number of lines in FILE matching the extended regex PATTERN, compared with
# the number expected.
lines() { test "$(grep -cE "$2" "$3")" -eq "$1"; }

cargo build -q -p anyprobe-spike --example inspect_dof || exit 2
inspect="target/debug/examples/inspect_dof"

for profile in "$@"; do
  echo "== ${target:-$host} $profile (anyprobe examples attr, same_name)"
  profile_failed=0
  build=(cargo build -q -p anyprobe --example attr --example same_name --profile "$profile")
  dir="target/$profile/examples"
  if [ -n "$target" ]; then
    build+=(--target "$target")
    dir="target/$target/$profile/examples"
  fi
  if ! "${build[@]}"; then
    echo "  FAIL  build"
    failed=1
    continue
  fi
  attr="$dir/attr"
  same="$dir/same_name"

  "$inspect" "$attr" >"$work/attr.dof" 2>&1
  expect "attr: every site rewritten and inside its function" test $? -eq 0
  "$inspect" "$same" >"$work/same.dof" 2>&1
  expect "same_name: every site rewritten and inside its function" test $? -eq 0
  expect "same_name: one new-entry probe site per function" \
    test "$(grep -cE ':new-entry sites=1 ' "$work/same.dof")" -eq 3

  if [ "$attach" != 0 ]; then
    out="$work/attr.dtrace"
    sudo dtrace -q -c "$attr $iterations 20" -n '
      attr$target:::lookup-entry { printf("lookup-entry %d %s\n", arg0, copyinstr(arg1, arg2)); }
      attr$target:::lookup-return { printf("lookup-return %d\n", arg0); }
      attr$target:::query-entry {
        printf("query-entry %d %s\n", arg0, copyinstr(arg1, arg2));
        printf("query-entry-nul %s\n", copyinstr(arg1));
      }
      attr$target:::query-return { printf("query-return %s\n", copyinstr(arg0, arg1)); }
      attr$target:::wide-entry { printf("wide-entry %s\n", copyinstr(arg0, arg1)); }
      attr$target:::counter_bump-entry {
        printf("bump-entry %s %d\n", copyinstr(arg0, arg1), arg2);
      }' >"$out" 2>&1
    n=$iterations
    expect "native arguments: id and path" lines "$n" '^lookup-entry [0-9]+ /index$' "$out"
    expect "native return value" lines "$n" '^lookup-return [0-9]*6$' "$out"
    expect "serde argument as JSON" \
      lines "$n" '^query-entry [0-9]+ \{"table":"rows","limit":5\}$' "$out"
    expect "serde argument read up to its NUL" \
      lines "$n" '^query-entry-nul \{"table":"rows","limit":5\}$' "$out"
    expect "debug return value, Ok" lines $((n / 2)) '^query-return Ok\(5\)$' "$out"
    expect "debug return value, Err" lines $((n / 2)) '^query-return Err\("odd [0-9]+"\)$' "$out"
    expect "collapsed arguments as one JSON object" lines "$n" \
      '^wide-entry \{"id":[0-9]+,"a":"x","b":"y","c":"z","tags":"\[\\"t\\"\]"\}$' "$out"
    expect "debug(self) and a native argument" \
      lines "$n" '^bump-entry Counter \{ n: [0-9]+ \} 1$' "$out"
    expect "nothing unexpected" test -z "$(grep -vE \
      "^(lookup-entry|lookup-return|query-entry|query-entry-nul|query-return|wide-entry|bump-entry) |^(pid|backend)=|^done |^\$|$sip" \
      "$out")"

    sout="$work/same.dtrace"
    # The function DTrace reports is the mangled helper, whose path names the
    # impl's type or the module: `3Foo`, `3Bar`, `5other`.
    sudo dtrace -q -c "$same $iterations 20" -n '
      same_name$target:::new-entry /strstr(probefunc, "3Foo") != NULL/ {
        printf("foo-entry %d\n", arg0);
      }
      same_name$target:::new-entry /strstr(probefunc, "3Bar") != NULL/ {
        printf("bar-entry\n");
      }
      same_name$target:::new-entry /strstr(probefunc, "5other") != NULL/ {
        printf("other-entry %s\n", copyinstr(arg0, arg1));
      }
      same_name$target:::new-return { @returns = count(); }
      END { printa("new-return %@d\n", @returns); }' >"$sout" 2>&1
    expect "same_name: Foo::new(x) reads x" \
      test "$(sed -n 's/^foo-entry //p' "$sout" | sort -n | tr '\n' ' ')" = "$(seq 0 $((n - 1)) | tr '\n' ' ')"
    expect "same_name: Bar::new() fires" lines "$n" '^bar-entry$' "$sout"
    expect "same_name: other::new(label) reads label" lines "$n" '^other-entry label$' "$sout"
    expect "same_name: new__return fires for all three" lines 1 "^new-return $((3 * n))$" "$sout"
    expect "same_name: nothing unexpected" test -z "$(grep -vE \
      "^(foo-entry|other-entry|new-return) |^bar-entry\$|^done |^\$|$sip" "$sout")"
  fi

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    for f in attr.dof same.dof attr.dtrace same.dtrace; do
      [ -f "$work/$f" ] && { echo "  --- $f (first 40 lines)"; head -40 "$work/$f"; }
    done
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
