#!/usr/bin/env bash
# Attaches bpftrace to the anyprobe `attr` and `attr_async` examples
# (`#[probe]`) and checks what it reads from each encoding, from `async fn`,
# from `unwind`, and through `symbol`.
#
# Usage: spike/scripts/attach-linux-attr.sh [PROFILE...]
#   PROFILE defaults to "release release-lto".
# Environment:
#   SPIKE_ATTACH  set to 0 to run only the checks that need no root.
#
# Needs cargo, readelf, objdump, bpftrace, and root or sudo for bpftrace.
# For each profile it builds both examples and checks that:
#   - `attr_async` has one SDT note per probe, each on a `nop`, and exports
#     `attr_async__exported` as a global function;
#   - in both examples, `.probes` starts a page that no other writable
#     segment maps (the kernel raises semaphores by file offset, for
#     `bpftrace -c` and perf, in the first writable mapping of the page);
#   - `cargo anyprobe list` finds every probe of both examples in their
#     registry, each with a site, and `cargo anyprobe bpftrace` writes one
#     clause per probe;
#   - with `attr` running and bpftrace attached for three seconds: native
#     arguments and a native return value read as written; a `serde`
#     argument reads as its JSON, and as the same text when read as a
#     NUL-terminated string (what perf and gdb do); a `debug` return value
#     reads as its `{:?}` output; arguments collapsed into one object read
#     as that JSON object; `debug(self)` on a method reads as the receiver's
#     `{:?}` output;
#   - with `attr_async` run under `bpftrace -c` for 20 iterations (two
#     interleaved `fetch` calls each): every `fetch` return pairs with its
#     entry by invocation id (unique, not 0) and carries
#     `id * 10 + path.len()`, and the second call of each pair returns
#     first; the cancelled `slow` fires its unwind probe 20 times with
#     `panicking` 0 and the entry's invocation id, and its return probe
#     never; `may_panic` fires entry 20 times, return 10 and unwind 10; and
#     `exported` is reached both by its USDT probe and, through its
#     `symbol`, by a uprobe, 20 times each with ids 0 to 19;
#   - the scripts `cargo anyprobe bpftrace` wrote, run under `bpftrace -c`
#     for 20 iterations, print every probe of both examples with each
#     argument decoded, the same number of times as above.
# Encoding runs only after the enabled check, so any encoded value read
# shows that attaching turned the check on.
# Prints ok/FAIL per check and exits non-zero if any check failed.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
[ $# -gt 0 ] || set -- release release-lto

attach=${SPIKE_ATTACH:-1}
if [ "$attach" != 0 ]; then
  command -v bpftrace >/dev/null || { echo "bpftrace not found; install it first" >&2; exit 2; }
fi
iterations=20
if [ "$(id -u)" -eq 0 ]; then sudo=""; else sudo="sudo"; fi

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

# Number of lines in FILE matching the extended regex PATTERN, compared with
# the number expected.
lines() { test "$(grep -cE "$2" "$3")" -eq "$1"; }

# Prints "ok" if every SDT note location in BIN is a `nop`, else the first
# location that is not.
notes_on_nops() {
  local bin=$1 loc insn
  for loc in $(readelf -nW "$bin" | sed -n 's/^ *Location: \(0x[0-9a-f]*\),.*/\1/p'); do
    insn=$(objdump -d --no-show-raw-insn --start-address="$loc" \
      --stop-address=$((loc + 4)) "$bin" | awk -F'\t' '/^ *[0-9a-f]+:/ { print $2; exit }')
    case "$insn" in
      nop*) ;;
      *) echo "$loc: $insn"; return ;;
    esac
  done
  echo ok
}

# Prints "ok" if the file page holding BIN's `.probes` section is mapped by
# exactly one writable LOAD segment, else what maps it.
probes_page() {
  local bin=$1 off
  off=$(readelf -SW "$bin" | awk '$2 == ".probes" { print $5 }')
  [ -n "$off" ] || { echo "no .probes section"; return; }
  readelf -lW "$bin" | awk -v page=$(( 0x$off & ~0xfff )) '
    $1 == "LOAD" && $7 ~ /W/ {
      off = strtonum($2); size = strtonum($5)
      if (off < page + 4096 && off + size > page) { n++; at = at " " $2 }
    }
    END { print (n == 1 ? "ok" : n + 0 " writable segments map it:" at) }'
}

for profile in "$@"; do
  echo "== $profile (anyprobe examples attr, attr_async)"
  profile_failed=0
  if ! cargo build -q -p anyprobe --example attr --example attr_async --profile "$profile" \
    || ! cargo build -q -p cargo-anyprobe; then
    echo "  FAIL  build"
    failed=1
    continue
  fi
  bin="$PWD/target/$profile/examples/attr"
  log="$work/$profile.log"
  out="$work/$profile.bpftrace"

  async="$PWD/target/$profile/examples/attr_async"
  aout="$work/$profile.async.bpftrace"

  expect "attr_async: one SDT note per probe" test "$(readelf -nW "$async" \
    | sed -n 's/^ *Name: //p' | sort | tr '\n' ' ')" = \
    "exported__entry exported__return fetch__entry fetch__return may_panic__entry may_panic__return may_panic__unwind slow__entry slow__return slow__unwind "
  for b in "$bin" "$async"; do
    page=$(probes_page "$b")
    expect "$(basename "$b"): .probes on a page of its own ($page)" test "$page" = ok
  done
  sites=$(notes_on_nops "$async")
  expect "attr_async: every SDT note on a nop ($sites)" test "$sites" = ok
  expect "attr_async: symbol exported as attr_async__exported" \
    bash -c "readelf -sW '$async' | grep -qE ' FUNC +GLOBAL +DEFAULT +[0-9]+ attr_async__exported\$'"

  cli="$PWD/target/debug/cargo-anyprobe"
  gen="$work/$profile.gen"
  "$cli" list "$bin" >"$gen.attr.list" 2>&1
  expect "cargo anyprobe list: attr has 8 probes, each with a site" \
    bash -c "tail -1 '$gen.attr.list' | grep -qx '8 probes in 1 provider' && ! grep -q 'no site' '$gen.attr.list'"
  "$cli" list "$async" >"$gen.async.list" 2>&1
  expect "cargo anyprobe list: attr_async has 10 probes, each with a site" \
    bash -c "tail -1 '$gen.async.list' | grep -qx '10 probes in 1 provider' && ! grep -q 'no site' '$gen.async.list'"
  "$cli" bpftrace "$bin" >"$gen.attr.bt" 2>"$gen.attr.err"
  expect "cargo anyprobe bpftrace: one clause per attr probe" \
    test "$(grep -c '^usdt:' "$gen.attr.bt")" -eq 8
  "$cli" bpftrace "$async" >"$gen.async.bt" 2>"$gen.async.err"
  expect "cargo anyprobe bpftrace: one clause per attr_async probe" \
    test "$(grep -c '^usdt:' "$gen.async.bt")" -eq 10

  if [ "$attach" = 0 ]; then
    if [ "$profile_failed" -ne 0 ]; then
      failed=1
      for f in "$gen".*; do echo "  --- $(basename "$f")"; head -40 "$f"; done
    fi
    continue
  fi

  "$bin" 0 20 >"$log" 2>&1 &
  pid=$!
  sleep 1
  $sudo timeout 60 bpftrace -p "$pid" -e "
    usdt:$bin:attr:lookup__entry { printf(\"lookup-entry %d %s\n\", arg0, str(arg1, arg2)); }
    usdt:$bin:attr:lookup__return { printf(\"lookup-return %d\n\", arg0); }
    usdt:$bin:attr:query__entry {
      printf(\"query-entry %d %s\n\", arg0, str(arg1, arg2));
      printf(\"query-entry-nul %s\n\", str(arg1));
    }
    usdt:$bin:attr:query__return { printf(\"query-return %s\n\", str(arg0, arg1)); }
    usdt:$bin:attr:wide__entry { printf(\"wide-entry %s\n\", str(arg0, arg1)); }
    usdt:$bin:attr:counter_bump__entry { printf(\"bump-entry %s %d\n\", str(arg0, arg1), arg2); }
    interval:s:3 { exit(); }" >"$out" 2>&1
  sleep 1
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null

  expect "native arguments: id and path" grep -qE '^lookup-entry [0-9]+ /index$' "$out"
  expect "native return value" grep -qE '^lookup-return [0-9]*6$' "$out"
  expect "serde argument as JSON" \
    grep -qE '^query-entry [0-9]+ \{"table":"rows","limit":5\}$' "$out"
  expect "serde argument read up to its NUL" \
    grep -qE '^query-entry-nul \{"table":"rows","limit":5\}$' "$out"
  expect "debug return value, Ok" grep -qE '^query-return Ok\(5\)$' "$out"
  expect "debug return value, Err" grep -qE '^query-return Err\("odd [0-9]+"\)$' "$out"
  expect "collapsed arguments as one JSON object" \
    grep -qE '^wide-entry \{"id":[0-9]+,"a":"x","b":"y","c":"z","tags":"\[\\"t\\"\]"\}$' "$out"
  expect "debug(self) and a native argument" \
    grep -qE '^bump-entry Counter \{ n: [0-9]+ \} 1$' "$out"
  # Every printed line is one of the formats above.
  expect "nothing unexpected" \
    test -z "$(grep -vE '^(lookup-entry|lookup-return|query-entry|query-entry-nul|query-return|wide-entry|bump-entry) |^Attaching |^$' "$out")"

  # Run under bpftrace, so every call is seen and the counts are exact.
  $sudo timeout 120 bpftrace -c "$async $iterations 20" -e "
    usdt:$async:attr_async:fetch__entry {
      printf(\"fetch-entry %d %d %s\n\", arg0, arg1, str(arg2, arg3));
    }
    usdt:$async:attr_async:fetch__return { printf(\"fetch-return %d %d\n\", arg0, arg1); }
    usdt:$async:attr_async:slow__entry { printf(\"slow-entry %d %d\n\", arg0, arg1); }
    usdt:$async:attr_async:slow__return { printf(\"slow-return %d\n\", arg0); }
    usdt:$async:attr_async:slow__unwind { printf(\"slow-unwind %d %d\n\", arg0, arg1); }
    usdt:$async:attr_async:may_panic__entry { printf(\"may_panic-entry %d\n\", arg0); }
    usdt:$async:attr_async:may_panic__return { printf(\"may_panic-return\n\"); }
    usdt:$async:attr_async:may_panic__unwind { printf(\"may_panic-unwind\n\"); }
    usdt:$async:attr_async:exported__entry { printf(\"exported-entry %d\n\", arg0); }
    uprobe:$async:attr_async__exported { printf(\"exported-uprobe %d\n\", arg0); }" \
    >"$aout" 2>&1
  n=$iterations
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
  expect "attr_async: uprobe reaches exported by its symbol" \
    test "$(sed -n 's/^exported-uprobe //p' "$aout" | sort -n | tr '\n' ' ')" = "$ids"
  expect "attr_async: nothing unexpected" test -z "$(grep -vE \
    '^(fetch-entry|fetch-return|slow-entry|slow-unwind|may_panic-entry|exported-entry|exported-uprobe) |^may_panic-(return|unwind)$|^done |^Attaching |^$' \
    "$aout")"

  # The generated scripts, as they are.
  gout="$work/$profile.gen.attr.out"
  $sudo timeout 120 bpftrace -c "$bin $iterations 20" "$gen.attr.bt" >"$gout" 2>&1
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
    '^attr:[a-z_]+__(entry|return)( |$)|^(pid|backend)=|^done |^Attaching |^$' "$gout")"

  gaout="$work/$profile.gen.async.out"
  $sudo timeout 120 bpftrace -c "$async $iterations 20" "$gen.async.bt" >"$gaout" 2>&1
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
    '^attr_async:[a-z_]+__(entry|return|unwind)( |$)|^done |^Attaching |^$' "$gaout")"

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    echo "  --- example output"
    cat "$log"
    echo "  --- bpftrace output (first 40 lines)"
    head -40 "$out"
    echo "  --- bpftrace output, attr_async (first 40 lines)"
    head -40 "$aout"
    for f in "$gout" "$gaout"; do
      [ -f "$f" ] && { echo "  --- $(basename "$f") (first 40 lines)"; head -40 "$f"; }
    done
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
