#!/usr/bin/env bash
# Attaches bpftrace to the anyprobe `attr`, `attr_async` and `same_name`
# examples (`#[probe]`) and checks what it reads from each encoding, from
# `async fn`, from `unwind`, through `symbol`, and from three functions that
# share probe names.
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
#   - `anyprobe-check`'s `gc_sections` example, which never calls most of
#     the library's probed functions, has every SDT note on a `nop`: a
#     function `--gc-sections` collected must not leave a note naming an
#     address outside the code; its `foreign:site` note, written with the
#     flags of `sys/sdt.h` and the `usdt` crate in the same object as
#     anyprobe's sites, is in the binary too;
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
#     `{:?}` output; five native values, the most a probe passes, read as
#     written, the fifth included;
#   - with `attr_async` run under `bpftrace -c` for 20 iterations (two
#     interleaved `fetch` calls each): every `fetch` return pairs with its
#     entry by invocation id (unique, not 0) and carries
#     `id * 10 + path.len()`, and the second call of each pair returns
#     first; the cancelled `slow` fires its unwind probe 20 times with
#     `panicking` 0 and the entry's invocation id, and its return probe
#     never; `may_panic` fires entry 20 times, return 10 and unwind 10; and
#     `exported` is reached both by its USDT probe and, through its
#     `symbol`, by a uprobe, 20 times each with ids 0 to 19;
#   - with `same_name` run under `bpftrace -c` for 20 iterations: bpftrace
#     attaches to all three `new__entry` sites, and either one of the three
#     functions fires 20 times (bpftrace 0.20 raises one function's
#     semaphore) or all three do (0.25 raises them all); docs/GAPS.md,
#     "Methods with the same name";
#   - the scripts `cargo anyprobe bpftrace` wrote, run as their header says
#     (`bpftrace -p PID FILE`) for a few seconds against each running
#     example, print every probe of both examples with each argument
#     decoded, and no firing twice: a probe attached twice prints each id
#     twice.
# Encoding runs only after the enabled check, so any encoded value read
# shows that attaching turned the check on.
# bpftrace runs with --no-warnings. Lines bpftrace prints on its own are left
# out of the "nothing unexpected" checks (see `noise` below).
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
# At least N lines in FILE match PATTERN.
atleast() { test "$(grep -cE "$2" "$3")" -ge "$1"; }

# Lines bpftrace prints besides a script's output: "Attaching N probes..."
# (0.20) or "Attached N probes" (0.24 and later), and the C compiler
# warnings Ubuntu's bpftrace 0.25 prints about the kernel's vmlinux.h even
# under --no-warnings.
noise='^Attach(ing [0-9]+ probes\.\.\.|ed [0-9]+ probes?)$|^HINT: |^[^ ]+:[0-9]+:[0-9]+: warning: |^ *[0-9]* \| |^[0-9]+ warnings? generated\.$|^$'

# Runs the generated script SCRIPT as its header says, `bpftrace -p PID
# SCRIPT`, for five seconds against COMMAND..., and writes what bpftrace
# printed to OUT. Ubuntu's bpftrace 0.25.0 aborts on any script file with a
# USDT probe (docs/GAPS.md, "bpftrace versions"); the same text is then
# passed with -e, and the run says so.
generated() {
  local script=$1 out=$2 pid
  shift 2
  "$@" >/dev/null 2>&1 &
  pid=$!
  sleep 1
  $sudo timeout -s INT 5 bpftrace --no-warnings -p "$pid" "$script" >"$out" 2>&1
  if grep -q "source_context.*Assertion.*failed" "$out"; then
    echo "  info  bpftrace aborted on $(basename "$script") (a bpftrace bug); passing it with -e"
    $sudo timeout -s INT 5 bpftrace --no-warnings -p "$pid" -e "$(cat "$script")" >"$out" 2>&1
  fi
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}

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
  echo "== $profile (anyprobe examples attr, attr_async, same_name)"
  profile_failed=0
  if ! cargo build -q -p anyprobe --example attr --example attr_async --example same_name \
    --profile "$profile" \
    || ! cargo build -q -p anyprobe-check --example gc_sections --profile "$profile" \
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
  sites=$(notes_on_nops "$PWD/target/$profile/examples/gc_sections")
  expect "gc_sections: every SDT note on a nop ($sites)" test "$sites" = ok
  expect "gc_sections: sys/sdt.h-style note beside anyprobe's" \
    bash -c "readelf -nW '$PWD/target/$profile/examples/gc_sections' | grep -qE 'Provider: foreign\$'"
  expect "attr_async: symbol exported as attr_async__exported" \
    bash -c "readelf -sW '$async' | grep -qE ' FUNC +GLOBAL +DEFAULT +[0-9]+ attr_async__exported\$'"

  cli="$PWD/target/debug/cargo-anyprobe"
  gen="$work/$profile.gen"
  "$cli" list "$bin" >"$gen.attr.list" 2>&1
  expect "cargo anyprobe list: attr has 12 probes, each with a site" \
    bash -c "tail -1 '$gen.attr.list' | grep -qx '12 probes in 1 provider' && ! grep -q 'no site' '$gen.attr.list'"
  "$cli" list "$async" >"$gen.async.list" 2>&1
  expect "cargo anyprobe list: attr_async has 10 probes, each with a site" \
    bash -c "tail -1 '$gen.async.list' | grep -qx '10 probes in 1 provider' && ! grep -q 'no site' '$gen.async.list'"
  "$cli" bpftrace "$bin" >"$gen.attr.bt" 2>"$gen.attr.err"
  expect "cargo anyprobe bpftrace: one clause per attr probe" \
    test "$(grep -c '^usdt:' "$gen.attr.bt")" -eq 12
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
  # `usdt:*:` rather than the binary's path: under -p, bpftrace 0.25 attaches
  # a probe named by path twice (the path and /proc/PID/root). A native &str
  # has no NUL after it and is read with `buf`, which reads its length in
  # every bpftrace version; `str` reads one byte less from 0.23 on. Encoded
  # text has a NUL after it, so `str` of its length plus one reads all of it
  # in every version.
  $sudo timeout 60 bpftrace --no-warnings -p "$pid" -e "
    usdt:*:attr:lookup__entry { printf(\"lookup-entry %d %r\n\", arg0, buf(arg1, arg2)); }
    usdt:*:attr:lookup__return { printf(\"lookup-return %d\n\", arg0); }
    usdt:*:attr:query__entry {
      printf(\"query-entry %d %s\n\", arg0, str(arg1, arg2 + 1));
      printf(\"query-entry-nul %s\n\", str(arg1));
    }
    usdt:*:attr:query__return { printf(\"query-return %s\n\", str(arg0, arg1 + 1)); }
    usdt:*:attr:wide__entry { printf(\"wide-entry %s\n\", str(arg0, arg1 + 1)); }
    usdt:*:attr:counter_bump__entry { printf(\"bump-entry %s %d\n\", str(arg0, arg1 + 1), arg2); }
    usdt:*:attr:five__entry {
      printf(\"five-entry %d %d %d %d %d\n\", arg0, (int64)arg1, arg2, arg3, arg4);
    }
    usdt:*:attr:optional__entry {
      printf(\"optional-entry %r|%d|%s\n\", buf(arg0, arg1), arg3, str(arg4));
    }
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
  expect "five native values, the fifth on its own" \
    grep -qE '^five-entry [0-9]+ -12 13 1 16$' "$out"
  expect "Option<&str> Some, Option<&[u8]> None, &CStr" grep -qE '^optional-entry opt\|0\|cee$' "$out"
  expect "Option<&str> None, Option<&[u8]> Some, &CStr" grep -qE '^optional-entry \|1\|cee$' "$out"
  # Every printed line is one of the formats above.
  expect "nothing unexpected" \
    test -z "$(grep -vE '^(lookup-entry|lookup-return|query-entry|query-entry-nul|query-return|wide-entry|bump-entry|five-entry|optional-entry) ' "$out" | grep -vE "$noise")"

  # Run under bpftrace, so every call is seen and the counts are exact.
  $sudo timeout 120 bpftrace --no-warnings -c "$async $iterations 20" -e "
    usdt:$async:attr_async:fetch__entry {
      printf(\"fetch-entry %d %d %r\n\", arg0, arg1, buf(arg2, arg3));
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
    '^(fetch-entry|fetch-return|slow-entry|slow-unwind|may_panic-entry|exported-entry|exported-uprobe) |^may_panic-(return|unwind)$|^done ' \
    "$aout" | grep -vE "$noise")"

  # Prints the site of every firing; the symbol names the function. bpftrace
  # names the program counter `ip` on x86-64 and `pc` on aarch64.
  same="$PWD/target/$profile/examples/same_name"
  sout="$work/$profile.same.bpftrace"
  case $(uname -m) in
    aarch64 | arm64) pc=pc ;;
    *) pc=ip ;;
  esac
  $sudo timeout 60 bpftrace --no-warnings -c "$same $iterations 20" -e "
    usdt:$same:same_name:new__entry { printf(\"entry %s\n\", usym(reg(\"$pc\"))); }" \
    >"$sout" 2>&1
  fired=$(grep -oE '3Foo3new|3Bar3new|5other3new' "$sout" | sort | uniq -c \
    | awk '{ printf "%s%s=%d", sep, $2, $1; sep = " " }')
  # bpftrace 0.20 counts the three sites ("Attaching 3 probes..."); 0.25
  # counts the one clause ("Attached 1 probe"), and its sites show up by all
  # three functions firing.
  expect "same_name: bpftrace attaches to all three new__entry sites" \
    bash -c "grep -qx 'Attaching 3 probes...' '$sout' ||
      [ '$fired' = '3Bar3new=$n 3Foo3new=$n 5other3new=$n' ]"
  expect "same_name: one function fires $n times, or all three do ($fired)" \
    bash -c "[[ '$fired' =~ ^[0-9a-zA-Z]+=$n\$ || '$fired' == '3Bar3new=$n 3Foo3new=$n 5other3new=$n' ]]"

  # The generated scripts, as they are. Counts depend on how long bpftrace
  # was attached, so the checks ask for every line format, at least 50
  # firings of the busiest probe, and no line printed twice: each of these
  # lines carries an id or counter that changes with every call.
  gout="$work/$profile.gen.attr.out"
  generated "$gen.attr.bt" "$gout" "$bin" 0 20
  expect "generated: lookup entry and return" bash -c "
    atleast() { test \$(grep -cE \"\$2\" \"\$3\") -ge \$1; }
    atleast 50 '^attr:lookup__entry id=[0-9]+ path=/index\$' '$gout' &&
    atleast 1 '^attr:lookup__return ret=[0-9]*6\$' '$gout'"
  expect "generated: query serde argument and debug return" bash -c "
    grep -qE '^attr:query__entry id=[0-9]+ q=\{\"table\":\"rows\",\"limit\":5\}\$' '$gout' &&
    grep -qE '^attr:query__return ret=Ok\(5\)\$' '$gout' &&
    grep -qE '^attr:query__return ret=Err\(\"odd [0-9]+\"\)\$' '$gout'"
  expect "generated: collapsed arguments" bash -c "
    grep -qE '^attr:wide__entry args=\{\"id\":[0-9]+,' '$gout' &&
    grep -qE '^attr:wide__return\$' '$gout'"
  expect "generated: debug(self)" bash -c "
    grep -qE '^attr:counter_bump__entry self=Counter \{ n: [0-9]+ \} by=1\$' '$gout' &&
    grep -qE '^attr:counter_bump__return\$' '$gout'"
  expect "generated: five native values" bash -c "
    grep -qE '^attr:five__entry id=[0-9]+ neg=-12 c=13 on=1 last=16\$' '$gout' &&
    grep -qE '^attr:five__return\$' '$gout'"
  expect "generated: optional strings and bytes, C string" bash -c "
    grep -qE '^attr:optional__entry name=opt key= label=cee\$' '$gout' &&
    grep -qE '^attr:optional__entry name= key=.+ label=cee\$' '$gout' &&
    grep -qE '^attr:optional__return\$' '$gout'"
  expect "generated: attr, no firing printed twice" \
    test -z "$(grep -E 'id=[0-9]+|n: [0-9]+' "$gout" | sort | uniq -d)"
  expect "generated: attr, nothing unexpected" test -z "$(grep -vE \
    '^attr:[a-z_]+__(entry|return)( |$)' "$gout" | grep -vE "$noise")"

  gaout="$work/$profile.gen.async.out"
  generated "$gen.async.bt" "$gaout" "$async" 100000 20
  expect "generated: fetch entry and return with invocation ids" bash -c "
    atleast() { test \$(grep -cE \"\$2\" \"\$3\") -ge \$1; }
    atleast 50 '^attr_async:fetch__entry invocation=[1-9][0-9]* id=[0-9]+ path=/(a|bb)\$' '$gaout' &&
    atleast 1 '^attr_async:fetch__return invocation=[1-9][0-9]* ret=[0-9]+\$' '$gaout'"
  expect "generated: cancelled slow unwinds, never returns" bash -c "
    grep -qE '^attr_async:slow__unwind invocation=[1-9][0-9]* panicking=0\$' '$gaout' &&
    ! grep -qE '^attr_async:slow__return ' '$gaout'"
  expect "generated: may_panic entry, return and unwind" bash -c "
    grep -qE '^attr_async:may_panic__entry id=[0-9]+\$' '$gaout' &&
    grep -qE '^attr_async:may_panic__return\$' '$gaout' &&
    grep -qE '^attr_async:may_panic__unwind\$' '$gaout'"
  expect "generated: exported entry and return" bash -c "
    grep -qE '^attr_async:exported__entry id=[0-9]+\$' '$gaout' &&
    grep -qE '^attr_async:exported__return\$' '$gaout'"
  expect "generated: attr_async, no firing printed twice" \
    test -z "$(grep -E 'invocation=[0-9]+|id=[0-9]+' "$gaout" | sort | uniq -d)"
  expect "generated: attr_async, nothing unexpected" test -z "$(grep -vE \
    '^attr_async:[a-z_]+__(entry|return|unwind)( |$)' "$gaout" | grep -vE "$noise")"

  if [ "$profile_failed" -ne 0 ]; then
    failed=1
    echo "  --- example output"
    cat "$log"
    echo "  --- bpftrace output (first 40 lines)"
    head -40 "$out"
    echo "  --- bpftrace output, attr_async (first 40 lines)"
    head -40 "$aout"
    echo "  --- bpftrace output, same_name (first 40 lines)"
    head -40 "$sout"
    for f in "$gout" "$gaout"; do
      [ -f "$f" ] && { echo "  --- $(basename "$f") (first 40 lines)"; head -40 "$f"; }
    done
  fi
done

if [ "$failed" -eq 0 ]; then echo "PASS"; else echo "FAIL"; fi
exit "$failed"
