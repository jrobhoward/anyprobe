#!/bin/sh
# Checks `#[probe]` on FreeBSD: the site table and registry of the anyprobe
# `attr`, `same_name` and `attr_async` examples, then what dtrace reads from
# each encoding, from `async fn`, from `unwind`, and through `symbol`.
#
# Usage: spike/scripts/attach-freebsd-attr.sh [PROFILE...]
#   PROFILE defaults to "release release-lto".
# Environment:
#   SPIKE_ATTACH  set to 0 to skip the dtrace checks, which need root, sudo
#                 or doas (and `kldload dtraceall`).
#
# For each profile it checks that:
#   - each example has an `anyprobe_sites` section and constructors to
#     register it, and `attr_async__exported` is an exported symbol;
#   - `cargo anyprobe list` finds every probe of `attr` and `attr_async` in
#     their registry, each with a site in the site table, and `cargo anyprobe
#     dtrace` writes one clause per probe;
#   - under dtrace -c with 20 iterations, exactly 20 times each: native
#     arguments and a native return value read as written; a `serde`
#     argument reads as its JSON, and as the same text when read up to its
#     NUL; arguments collapsed into one object read as that JSON object;
#     `debug(self)` reads as the receiver's `{:?}` output. A `debug` return
#     value reads as `Ok(5)` 10 times and `Err("odd N")` 10 times. Five
#     native values, the most a probe passes, read as written, the fifth
#     included;
#   - for `same_name`, whose three `new__entry` probes have different
#     argument types: they are three probes, each fires 20 times, `Foo::new`
#     reads `x` and `other::new` reads `label`, and dtrace refuses typed
#     `args[]` across them, which it does only when their argument types
#     differ; `new__return` fires 60 times. DTrace reports the function of
#     all three as `new`, so they are told apart by probe id;
#   - for `attr_async` (20 iterations, two interleaved `fetch` calls each):
#     every `fetch` return pairs with its entry by invocation id (unique, not
#     0) and carries `id * 10 + path.len()`, and the second call of each pair
#     returns first; the cancelled `slow` fires its unwind probe 20 times
#     with `panicking` 0 and the entry's invocation id, and its return probe
#     never; `may_panic` fires entry 20 times, return 10 and unwind 10; and
#     `exported` is reached both by its USDT probe and, through its
#     `symbol`, by the `pid` provider, 20 times each with ids 0 to 19;
#   - the D scripts `cargo anyprobe dtrace` wrote, run under dtrace -c for
#     20 iterations, print every probe of both examples with each argument
#     decoded, the same number of times as above.
# Encoding runs only after the enabled check, so any encoded value read
# shows that attaching turned the check on. Prints ok/FAIL per check and
# exits non-zero if any check failed.

set -u
cd "$(dirname "$0")/../.." || exit 2
[ $# -gt 0 ] || set -- release release-lto

attach=${SPIKE_ATTACH:-1}
iterations=20

if [ "$attach" != 0 ]; then
  if [ "$(id -u)" -eq 0 ]; then
    sudo=""
  elif command -v sudo >/dev/null; then
    sudo="sudo"
  elif command -v doas >/dev/null; then
    sudo="doas"
  else
    echo "run as root, or install sudo or doas: dtrace needs root (or SPIKE_ATTACH=0)" >&2
    exit 2
  fi
  $sudo kldload -n dtraceall || exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failed=0

expect() {
  check=$1
  shift
  if "$@"; then
    echo "  ok    $check"
  else
    echo "  FAIL  $check"
    profile_failed=1
  fi
}

# Number of lines in FILE matching the extended regex PATTERN, compared with
# the number expected.
lines() { test "$(grep -cE "$2" "$3")" -eq "$1"; }

# Space-separated, numerically sorted values after PREFIX in FILE.
values() { sed -n "s/^$1 //p" "$2" | sort -n | tr '\n' ' '; }

cargo build -q -p cargo-anyprobe || exit 2
cli="target/debug/cargo-anyprobe"

for profile in "$@"; do
  echo "== $profile (anyprobe examples attr, same_name, attr_async)"
  profile_failed=0
  if ! cargo build -q -p anyprobe --example attr --example same_name --example attr_async \
    --profile "$profile"; then
    echo "  FAIL  build"
    failed=1
    continue
  fi
  dir="target/$profile/examples"
  attr="$dir/attr"
  same="$dir/same_name"
  async="$dir/attr_async"

  for bin in "$attr" "$same" "$async"; do
    expect "$(basename "$bin"): site table and constructors present" sh -c "
      readelf -SW '$bin' | grep -q ' anyprobe_sites ' &&
      readelf -SW '$bin' | grep -q ' .init_array ' &&
      readelf -SW '$bin' | grep -q ' .fini_array '"
  done
  expect "attr_async: symbol exported as attr_async__exported" \
    sh -c "nm '$async' | grep -qE ' T attr_async__exported\$'"

  gen="$work/gen"
  "$cli" list "$attr" >"$gen.attr.list" 2>&1
  expect "cargo anyprobe list: attr has 12 probes, each with a site" \
    sh -c "tail -1 '$gen.attr.list' | grep -qx '12 probes in 1 provider' && ! grep -q 'no site' '$gen.attr.list'"
  "$cli" list "$async" >"$gen.async.list" 2>&1
  expect "cargo anyprobe list: attr_async has 10 probes, each with a site" \
    sh -c "tail -1 '$gen.async.list' | grep -qx '10 probes in 1 provider' && ! grep -q 'no site' '$gen.async.list'"
  "$cli" dtrace "$attr" >"$gen.attr.d" 2>"$gen.attr.err"
  expect "cargo anyprobe dtrace: one clause per attr probe" \
    test "$(grep -c '^attr\$target:::' "$gen.attr.d")" -eq 12
  "$cli" dtrace "$async" >"$gen.async.d" 2>"$gen.async.err"
  expect "cargo anyprobe dtrace: one clause per attr_async probe" \
    test "$(grep -c '^attr_async\$target:::' "$gen.async.d")" -eq 10

  if [ "$attach" != 0 ]; then
    n=$iterations
    ids="$(seq 0 $((n - 1)) | tr '\n' ' ')"

    out="$work/attr.dtrace"
    $sudo dtrace -q -c "$attr $n 20" -n '
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
      }
      attr$target:::five-entry {
        printf("five-entry %d %d %d %d %d\n", arg0, arg1, arg2, arg3, arg4);
      }
      attr$target:::optional-entry {
        printf("optional-entry %s|%d|%s\n",
          arg0 ? copyinstr(arg0, arg1) : "(none)", arg3, copyinstr(arg4));
      }' >"$out" 2>&1
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
    expect "five native values, the fifth on its own" \
      lines "$n" '^five-entry [0-9]+ -12 13 1 16$' "$out"
    expect "Option<&str> Some, Option<&[u8]> None, &CStr" \
      lines $((n / 2)) '^optional-entry opt\|0\|cee$' "$out"
    expect "Option<&str> None, Option<&[u8]> Some, &CStr" \
      lines $((n / 2)) '^optional-entry \(none\)\|1\|cee$' "$out"
    expect "nothing unexpected" test -z "$(grep -vE \
      "^(lookup-entry|lookup-return|query-entry|query-entry-nul|query-return|wide-entry|bump-entry|five-entry|optional-entry) |^(pid|backend)=|^done |^\$" \
      "$out")"

    sout="$work/same.dtrace"
    # Every firing prints the probe id and the first two argument registers;
    # the label clause reads the string only where the length is right.
    $sudo dtrace -q -c "$same $n 20" -n '
      same_name$target:::new-entry { printf("entry %d %d %d\n", id, arg0, arg1); }
      same_name$target:::new-entry /arg1 == 5/ { printf("label %d %s\n", id, copyinstr(arg0, arg1)); }
      same_name$target:::new-return { @returns = count(); }
      END { printa("new-return %@d\n", @returns); }' >"$sout" 2>&1
    expect "same_name: new-entry is three probes, each firing $n times" test "$(
      awk '/^entry / { c[$2]++ } END { for (i in c) print c[i] }' "$sout" | sort | tr '\n' ' ')" \
      = "$n $n $n "
    other=$(awk '/^label / && $3 == "label" { c[$2]++ } END { for (p in c) print p }' "$sout")
    expect "same_name: other::new(label) reads label" \
      test -n "$other" -a "$(grep -c "^label $other label\$" "$sout")" -eq "$n"
    # `Bar::new()` passes nothing, so its argument registers hold whatever
    # the code before it left there, often `x` from `Foo::new`: Foo is the
    # first remaining probe that reads 0 to n-1, and Bar the other one.
    foo=$(awk -v n="$n" -v o="${other:-x}" '
      /^entry / && $2 != o { s[$2] = s[$2] " " $3 }
      END { want = ""; for (i = 0; i < n; i++) want = want " " i
            for (p in s) if (s[p] == want) { print p; exit } }' "$sout")
    expect "same_name: Foo::new(x) reads x, 0 to $((n - 1))" test -n "$foo"
    expect "same_name: Bar::new() is the third probe" test "$(
      awk -v a="${foo:-x}" -v b="${other:-x}" \
        '/^entry / && $2 != a && $2 != b { c[$2]++ } END { for (p in c) print c[p] }' "$sout")" = "$n"
    # DTrace refuses typed `args[]` across probes whose argument types
    # differ, so the refusal shows each function kept its own types.
    $sudo dtrace -q -c "$same 1 1" -n \
      'same_name$target:::new-entry { trace(args[0]); }' >"$work/same.args" 2>&1
    expect "same_name: the three probes have different argument types" \
      grep -q 'matches an unstable set of probes' "$work/same.args"
    expect "same_name: new__return fires for all three" lines 1 "^new-return $((3 * n))$" "$sout"
    expect "same_name: nothing unexpected" test -z "$(grep -vE \
      "^(entry|label|new-return) |^done |^\$" "$sout")"

    aout="$work/async.dtrace"
    $sudo dtrace -q -c "$async $n 20" -n '
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
      test "$(values exported-entry "$aout")" = "$ids"
    expect "attr_async: pid provider reaches exported by its symbol" \
      test "$(values exported-pid "$aout")" = "$ids"
    expect "attr_async: nothing unexpected" test -z "$(grep -vE \
      "^(fetch-entry|fetch-return|slow-entry|slow-unwind|may_panic-entry|exported-entry|exported-pid) |^may_panic-(return|unwind)\$|^done |^\$" \
      "$aout")"

    # The generated D scripts, as they are.
    gout="$work/gen.attr.out"
    $sudo dtrace -c "$attr $n 20" -s "$gen.attr.d" >"$gout" 2>&1
    expect "generated: lookup entry and return" sh -c "
      [ \$(grep -cE '^attr:lookup__entry id=[0-9]+ path=/index\$' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:lookup__return ret=[0-9]*6\$' '$gout') -eq $n ]"
    expect "generated: query serde argument and debug return" sh -c "
      [ \$(grep -cE '^attr:query__entry id=[0-9]+ q=\{\"table\":\"rows\",\"limit\":5\}\$' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:query__return ret=Ok\(5\)\$' '$gout') -eq $((n / 2)) ] &&
      [ \$(grep -cE '^attr:query__return ret=Err\(\"odd [0-9]+\"\)\$' '$gout') -eq $((n / 2)) ]"
    expect "generated: collapsed arguments" sh -c "
      [ \$(grep -cE '^attr:wide__entry args=\{\"id\":[0-9]+,' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:wide__return\$' '$gout') -eq $n ]"
    expect "generated: debug(self)" sh -c "
      [ \$(grep -cE '^attr:counter_bump__entry self=Counter \{ n: [0-9]+ \} by=1\$' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:counter_bump__return\$' '$gout') -eq $n ]"
    expect "generated: five native values" sh -c "
      [ \$(grep -cE '^attr:five__entry id=[0-9]+ neg=-12 c=13 on=1 last=16\$' '$gout') -eq $n ] &&
      [ \$(grep -cE '^attr:five__return\$' '$gout') -eq $n ]"
    expect "generated: optional strings and bytes, C string" sh -c "
      [ \$(grep -cE '^attr:optional__entry name=opt key=<0 bytes> label=cee\$' '$gout') -eq $((n / 2)) ] &&
      [ \$(grep -cE '^attr:optional__entry name=\(none\) key=<1 bytes> label=cee\$' '$gout') -eq $((n / 2)) ] &&
      [ \$(grep -cE '^attr:optional__return\$' '$gout') -eq $n ]"
    expect "generated: attr, nothing unexpected" test -z "$(grep -vE \
      "^attr:[a-z_]+__(entry|return)( |\$)|^(pid|backend)=|^done |^\$" "$gout")"

    gaout="$work/gen.async.out"
    $sudo dtrace -c "$async $n 20" -s "$gen.async.d" >"$gaout" 2>&1
    expect "generated: fetch entry and return with invocation ids" sh -c "
      [ \$(grep -cE '^attr_async:fetch__entry invocation=[1-9][0-9]* id=[0-9]+ path=/(a|bb)\$' '$gaout') -eq $((2 * n)) ] &&
      [ \$(grep -cE '^attr_async:fetch__return invocation=[1-9][0-9]* ret=[0-9]+\$' '$gaout') -eq $((2 * n)) ]"
    expect "generated: cancelled slow unwinds, never returns" sh -c "
      [ \$(grep -cE '^attr_async:slow__unwind invocation=[1-9][0-9]* panicking=0\$' '$gaout') -eq $n ] &&
      [ \$(grep -cE '^attr_async:slow__return ' '$gaout') -eq 0 ]"
    expect "generated: may_panic entry, return and unwind" sh -c "
      [ \$(grep -cE '^attr_async:may_panic__entry id=[0-9]+\$' '$gaout') -eq $n ] &&
      [ \$(grep -cE '^attr_async:may_panic__return\$' '$gaout') -eq $((n / 2)) ] &&
      [ \$(grep -cE '^attr_async:may_panic__unwind\$' '$gaout') -eq $((n / 2)) ]"
    expect "generated: exported entry and return" sh -c "
      [ \$(grep -cE '^attr_async:exported__entry id=[0-9]+\$' '$gaout') -eq $n ] &&
      [ \$(grep -cE '^attr_async:exported__return\$' '$gaout') -eq $n ]"
    expect "generated: attr_async, nothing unexpected" test -z "$(grep -vE \
      "^attr_async:[a-z_]+__(entry|return|unwind)( |\$)|^done |^\$" "$gaout")"
  fi

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    for f in attr.dtrace same.dtrace async.dtrace \
      gen.attr.list gen.async.list gen.attr.d gen.async.d gen.attr.out gen.async.out; do
      [ -f "$work/$f" ] && { echo "  --- $f (first 40 lines)"; head -40 "$work/$f"; }
    done
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
