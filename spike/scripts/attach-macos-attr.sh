#!/usr/bin/env bash
# Checks `#[probe]` on macOS: the DOF ld64 built for the anyprobe `attr`,
# `same_name` and `attr_async` examples, then what dtrace reads from each
# encoding, from `async fn`, from `unwind`, and through `symbol`.
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
#   - `cargo anyprobe list` finds every probe of `attr` and `attr_async` in
#     their registry, each with a DOF site, and `cargo anyprobe dtrace`
#     writes one clause per probe;
#   - under `sudo dtrace -c` with 20 iterations, exactly 20 times each:
#     native arguments and a native return value read as written; a `serde`
#     argument reads as its JSON, and as the same text when read up to its
#     NUL; arguments collapsed into one object read as that JSON object;
#     `debug(self)` reads as the receiver's `{:?}` output. A `debug` return
#     value reads as `Ok(5)` 10 times and `Err("odd N")` 10 times;
#   - for `same_name`, dtrace reads each function's arguments as that
#     function declares them, 20 times each, and `new__return` fires 60
#     times;
#   - for `attr_async` (20 iterations, two interleaved `fetch` calls each):
#     every `fetch` return pairs with its entry by invocation id (unique, not
#     0) and carries `id * 10 + path.len()`, and the second call of each pair
#     returns first; the cancelled `slow` fires its unwind probe 20 times
#     with `panicking` 0 and the entry's invocation id, and its return probe
#     never; `may_panic` fires entry 20 times, return 10 and unwind 10; and
#     `exported` is reached both by its USDT probe and, through its
#     `symbol`, by the `pid` provider, 20 times each with ids 0 to 19;
#   - the D scripts `cargo anyprobe dtrace` wrote, run under `sudo dtrace -c`
#     for 20 iterations, print every probe of both examples with each
#     argument decoded, the same number of times as above.
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
cargo build -q -p cargo-anyprobe || exit 2
cli="target/debug/cargo-anyprobe"

for profile in "$@"; do
  echo "== ${target:-$host} $profile (anyprobe examples attr, same_name, attr_async)"
  profile_failed=0
  build=(cargo build -q -p anyprobe --example attr --example same_name --example attr_async \
    --profile "$profile")
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
  async="$dir/attr_async"

  "$inspect" "$attr" >"$work/attr.dof" 2>&1
  expect "attr: every site rewritten and inside its function" test $? -eq 0
  "$inspect" "$same" >"$work/same.dof" 2>&1
  expect "same_name: every site rewritten and inside its function" test $? -eq 0
  expect "same_name: one new-entry probe site per function" \
    test "$(grep -cE ':new-entry sites=1 ' "$work/same.dof")" -eq 3
  "$inspect" "$async" >"$work/async.dof" 2>&1
  expect "attr_async: every site rewritten and inside its function" test $? -eq 0
  expect "attr_async: symbol exported as attr_async__exported" \
    bash -c "nm '$async' | grep -qE ' T _attr_async__exported\$'"

  gen="$work/gen"
  "$cli" list "$attr" >"$gen.attr.list" 2>&1
  expect "cargo anyprobe list: attr has 8 probes, each with a site" \
    bash -c "tail -1 '$gen.attr.list' | grep -qx '8 probes in 1 provider' && ! grep -q 'no site' '$gen.attr.list'"
  "$cli" list "$async" >"$gen.async.list" 2>&1
  expect "cargo anyprobe list: attr_async has 10 probes, each with a site" \
    bash -c "tail -1 '$gen.async.list' | grep -qx '10 probes in 1 provider' && ! grep -q 'no site' '$gen.async.list'"
  "$cli" dtrace "$attr" >"$gen.attr.d" 2>"$gen.attr.err"
  expect "cargo anyprobe dtrace: one clause per attr probe" \
    test "$(grep -c '^attr\$target:::' "$gen.attr.d")" -eq 8
  "$cli" dtrace "$async" >"$gen.async.d" 2>"$gen.async.err"
  expect "cargo anyprobe dtrace: one clause per attr_async probe" \
    test "$(grep -c '^attr_async\$target:::' "$gen.async.d")" -eq 10

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

    aout="$work/async.dtrace"
    sudo dtrace -q -c "$async $iterations 20" -n '
      attr_async$target:::fetch-entry {
        printf("fetch-entry %d %d %s\n", arg0, arg1, copyinstr(arg2, arg3));
      }
      attr_async$target:::fetch-return { printf("fetch-return %d %d\n", arg0, arg1); }
      attr_async$target:::slow-entry { printf("slow-entry %d %d\n", arg0, arg1); }
      attr_async$target:::slow-return { printf("slow-return %d\n", arg0); }
      attr_async$target:::slow-unwind { printf("slow-unwind %d %d\n", arg0, arg1); }
      attr_async$target:::may_panic-entry { printf("may_panic-entry %d\n", arg0); }
      attr_async$target:::may_panic-return { printf("may_panic-return\n"); }
      attr_async$target:::may_panic-unwind { printf("may_panic-unwind\n"); }
      attr_async$target:::exported-entry { printf("exported-entry %d\n", arg0); }
      pid$target::attr_async__exported:entry { printf("exported-pid %d\n", arg0); }' \
      >"$aout" 2>&1
    ids="$(seq 0 $((n - 1)) | tr '\n' ' ')"
    expect "attr_async: fetch entry fires twice per iteration" lines $((2 * n)) '^fetch-entry ' "$aout"
    expect "attr_async: fetch return fires twice per iteration" lines $((2 * n)) '^fetch-return ' "$aout"
    # Pairs each return with its entry by invocation id, checks the value,
    # and checks that the second call of each pair (odd id) returned first.
    pairing=$(awk '
      /^fetch-entry / {
        if ($2 == 0 || ($2 in id)) bad = "invocation id 0 or reused"
        id[$2] = $3; len[$2] = length($4)
      }
      /^fetch-return / {
        k = returns++
        if (!($2 in id)) { bad = "return with no matching entry"; next }
        if ($3 != id[$2] * 10 + len[$2]) bad = "return value does not match its entry"
        if (id[$2] != (k % 2 == 0 ? k + 1 : k - 1)) bad = "returns not in the interleaved order"
      }
      END { print (bad == "" ? "ok" : bad) }' "$aout")
    expect "attr_async: fetch returns pair with their entries ($pairing)" test "$pairing" = ok
    expect "attr_async: cancelled slow never returns" lines 0 '^slow-return ' "$aout"
    unwound=$(awk '
      /^slow-entry / { started[$2] = 1 }
      /^slow-unwind / { if (($2 in started) && $3 == 0) n++ }
      END { print n + 0 }' "$aout")
    expect "attr_async: cancelled slow unwinds, not panicking, with its invocation id" \
      test "$unwound" -eq "$n"
    expect "attr_async: may_panic entry" lines "$n" '^may_panic-entry ' "$aout"
    expect "attr_async: may_panic returns for even ids" lines $((n / 2)) '^may_panic-return$' "$aout"
    expect "attr_async: may_panic unwinds for odd ids" lines $((n / 2)) '^may_panic-unwind$' "$aout"
    expect "attr_async: exported's USDT probe reads its id" \
      test "$(sed -n 's/^exported-entry //p' "$aout" | sort -n | tr '\n' ' ')" = "$ids"
    expect "attr_async: pid provider reaches exported by its symbol" \
      test "$(sed -n 's/^exported-pid //p' "$aout" | sort -n | tr '\n' ' ')" = "$ids"
    expect "attr_async: nothing unexpected" test -z "$(grep -vE \
      "^(fetch-entry|fetch-return|slow-entry|slow-unwind|may_panic-entry|exported-entry|exported-pid) |^may_panic-(return|unwind)\$|^done |^\$|$sip" \
      "$aout")"

    # The generated D scripts, as they are.
    gout="$work/gen.attr.out"
    sudo dtrace -c "$attr $iterations 20" -s "$gen.attr.d" >"$gout" 2>&1
    expect "generated: lookup entry and return" bash -c "
      [ \$(grep -cE '^attr:lookup__entry id=[0-9]+ path=/index\$' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:lookup__return ret=[0-9]*6\$' '$gout') -eq $n ]"
    expect "generated: query serde argument and debug return" bash -c "
      [ \$(grep -cE '^attr:query__entry id=[0-9]+ q=\{\"table\":\"rows\",\"limit\":5\}\$' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:query__return ret=Ok\(5\)\$' '$gout') -eq $((n / 2)) ] &&
      [ \$(grep -cE '^attr:query__return ret=Err\(\"odd [0-9]+\"\)\$' '$gout') -eq $((n / 2)) ]"
    expect "generated: collapsed arguments" bash -c "
      [ \$(grep -cE '^attr:wide__entry args=\{\"id\":[0-9]+,' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:wide__return\$' '$gout') -eq $n ]"
    expect "generated: debug(self)" bash -c "
      [ \$(grep -cE '^attr:counter_bump__entry self=Counter \{ n: [0-9]+ \} by=1\$' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:counter_bump__return\$' '$gout') -eq $n ]"
    expect "generated: attr, nothing unexpected" test -z "$(grep -vE \
      "^attr:[a-z_]+__(entry|return)( |\$)|^(pid|backend)=|^done |^\$|$sip" "$gout")"

    gaout="$work/gen.async.out"
    sudo dtrace -c "$async $iterations 20" -s "$gen.async.d" >"$gaout" 2>&1
    expect "generated: fetch entry and return with invocation ids" bash -c "
      [ \$(grep -cE '^attr_async:fetch__entry invocation=[1-9][0-9]* id=[0-9]+ path=/(a|bb)\$' '$gaout') -eq $((2 * n)) ] &&
      [ \$(grep -cE '^attr_async:fetch__return invocation=[1-9][0-9]* ret=[0-9]+\$' '$gaout') -eq $((2 * n)) ]"
    expect "generated: cancelled slow unwinds, never returns" bash -c "
      [ \$(grep -cE '^attr_async:slow__unwind invocation=[1-9][0-9]* panicking=0\$' '$gaout') -eq $n ] &&
      [ \$(grep -cE '^attr_async:slow__return ' '$gaout') -eq 0 ]"
    expect "generated: may_panic entry, return and unwind" bash -c "
      [ \$(grep -cE '^attr_async:may_panic__entry id=[0-9]+\$' '$gaout') -eq $n ] &&
      [ \$(grep -cE '^attr_async:may_panic__return\$' '$gaout') -eq $((n / 2)) ] &&
      [ \$(grep -cE '^attr_async:may_panic__unwind\$' '$gaout') -eq $((n / 2)) ]"
    expect "generated: exported entry and return" bash -c "
      [ \$(grep -cE '^attr_async:exported__entry id=[0-9]+\$' '$gaout') -eq $n ] &&
      [ \$(grep -cE '^attr_async:exported__return\$' '$gaout') -eq $n ]"
    expect "generated: attr_async, nothing unexpected" test -z "$(grep -vE \
      "^attr_async:[a-z_]+__(entry|return|unwind)( |\$)|^done |^\$|$sip" "$gaout")"
  fi

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    for f in attr.dof same.dof async.dof attr.dtrace same.dtrace async.dtrace \
      gen.attr.list gen.async.list gen.attr.d gen.async.d gen.attr.out gen.async.out; do
      [ -f "$work/$f" ] && { echo "  --- $f (first 40 lines)"; head -40 "$work/$f"; }
    done
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
