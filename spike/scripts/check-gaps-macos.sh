#!/usr/bin/env bash
# Checks two runtime claims in docs/GAPS.md on macOS:
#
#   - A killed tracer leaves nothing behind ("What a tracer writes into the
#     process"). dtrace enables the `work` example's work-entry probe by its
#     pid-qualified name (`spike<PID>:::work-entry`, no `-p`) and is killed
#     with SIGKILL. While it is attached, the program reports the probe on
#     and the work-entry sites hold the kernel's trap; after the kill, the
#     program reports it off and every site holds the bytes it held before
#     attaching. The trap's encoding depends on the architecture, so the
#     check compares the bytes before, during and after. A second dtrace
#     then attaches and detaches normally.
#     Then the same with `dtrace -p`: killing that dtrace leaves the program
#     running, unlike on FreeBSD, with the probe off and every site restored.
#   - A probe in a `cdylib` loaded with `dlopen` can be traced ("Shared
#     libraries"). A scratch library defines `plugonly:::tick` and
#     `shared:::tick`; the executable that loads it defines `shared:::tick`
#     too. `dtrace -l` lists `plugonly<PID>:::tick` in module
#     `libplug.dylib`, which shows dyld registered the library's DOF, and
#     `dtrace -p` reads `plugonly$target:::tick`. Then `shared$target:::tick`,
#     defined in both files, fires in each, told apart by `probemod`.
#
# Site addresses come from examples/inspect_dof.rs, which lists every probe
# and is-enabled site of the binary on disk; ASLR moves them by the slide,
# which lldb reports. The bytes are read with lldb, which attaches to the
# program for a moment and needs root for that.
#
# Usage: spike/scripts/check-gaps-macos.sh
# Needs cargo, lldb (Xcode command line tools), and sudo for dtrace and
# lldb. Works with SIP on. Run as yourself, not under sudo: cargo run as root
# leaves root-owned files in target/. Prints ok/FAIL per check and exits
# non-zero if any check failed.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
command -v lldb >/dev/null || { echo "lldb not found" >&2; exit 2; }
sudo -v || exit 2

work=$(mktemp -d)
pids=()
cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null && wait "$p" 2>/dev/null; done
  rm -rf "$work"
}
trap cleanup EXIT
failed=0

expect() {
  local what=$1
  shift
  if "$@"; then
    echo "  ok    $what"
  else
    echo "  FAIL  $what"
    failed=1
  fi
}

# Polls until the program's log shows `entry-enabled=WANT` after line FROM.
wait_enabled() {
  for _ in $(seq 100); do
    tail -n +"$2" "$log" | grep -q "entry-enabled=$1" && return 0
    sleep 0.1
  done
  return 1
}

# The pid of the newest dtrace process. `sudo` does not pass SIGKILL on, so
# the checks kill dtrace itself rather than the sudo in front of it.
dtrace_pid() {
  for _ in $(seq 50); do
    pgrep -n -x dtrace && return 0
    sleep 0.1
  done
  return 1
}

# Lines of the program's log after line FROM.
log_since() { tail -n +"$1" "$log"; }

cargo build -q -p anyprobe-spike --example inspect_dof || exit 2
cargo build -q --release -p anyprobe --example work || exit 2
bin="$PWD/target/release/examples/work"
case "$(uname -m)" in
  arm64) insn_back=0 insn_len=4 ;;
  # inspect_dof prints the call's operand, one byte into the instruction.
  *) insn_back=1 insn_len=5 ;;
esac

# "PROBE ADDRESS" per site, probe and is-enabled sites alike, as file
# addresses.
target/debug/examples/inspect_dof "$bin" >"$work/dof" || exit 2
sed -E 's/^[^:]+:[^:]+:([^ ]+) .* at=\[([^]]*)\] enabled_at=\[([^]]*)\]$/\1 \2, \3/' "$work/dof" |
  while read -r probe rest; do
    for a in ${rest//,/ }; do printf '%s 0x%x\n' "$probe" $((0x$a - insn_back)); done
  done >"$work/sites"
nentry=$(grep -c '^work-entry ' "$work/sites")
nreturn=$(grep -c '^work-return ' "$work/sites")
text_vmaddr=$(otool -l "$bin" | awk '/segname __TEXT$/ { found = 1 } found && $1 == "vmaddr" { print $2; exit }')

# "PROBE ADDRESS BYTES" per site of the running program PID. The first call
# also finds the slide: where lldb says the image header is loaded, minus
# where the file puts it.
slide=""
snapshot() {
  if [ -z "$slide" ]; then
    local header
    header=$(sudo lldb -p "$pid" --batch -o "image list -h -f $(basename "$bin")" </dev/null 2>/dev/null |
      awk '/^\[ *0\]/ { print $3; exit }')
    [ -n "$header" ] || { echo "lldb could not attach to $pid" >&2; return 1; }
    slide=$((header - text_vmaddr))
  fi
  local args=() a
  while read -r _ a; do
    args+=(-o "memory read -s1 -fx -c$insn_len $(printf '0x%x' $((a + slide)))")
  done <"$work/sites"
  sudo lldb -p "$pid" --batch "${args[@]}" </dev/null 2>/dev/null |
    awk '/^0x[0-9a-f]+: 0x/ { $1 = ""; sub(/^ /, ""); print }' |
    paste -d' ' "$work/sites" -
}

# Number of sites of PROBE whose bytes differ between snapshots A and B.
changed() {
  paste -d'|' "$2" "$3" | awk -F'|' -v p="$1" '
    { split($1, a, " ") }
    a[1] == p && $1 != $2 { n++ }
    END { print n + 0 }'
}

echo "== killed tracer (anyprobe example work, $nentry work-entry sites, $nreturn work-return sites)"
log="$work/work.log"
"$bin" 0 20 >"$log" 2>&1 &
pid=$!
pids+=("$pid")
sleep 1
snapshot >"$work/before"
expect "before: read the bytes of all $((nentry + nreturn)) sites" \
  test "$(awk 'NF > 2' "$work/before" | wc -l)" -eq $((nentry + nreturn))

from=$(($(wc -l <"$log") + 1))
sudo dtrace -q -n "spike$pid:::work-entry { @n = count(); }" >"$work/dtrace.out" 2>&1 &
disown
wait_enabled true "$from"
expect "dtrace attached: the program reports the probe on" test $? -eq 0
snapshot >"$work/during"
n=$(changed work-entry "$work/before" "$work/during")
expect "dtrace attached: $n of $nentry work-entry sites hold other bytes" test "$n" -gt 0
expect "dtrace attached: no work-return site changed" \
  test "$(changed work-return "$work/before" "$work/during")" -eq 0
tpid=$(dtrace_pid)
sudo kill -9 "$tpid"
from=$(($(wc -l <"$log") + 1))
wait_enabled false "$from"
expect "dtrace killed: the program reports the probe off" test $? -eq 0
sleep 1
snapshot >"$work/after"
expect "dtrace killed: every site holds its bytes from before" cmp -s "$work/before" "$work/after"

from=$(($(wc -l <"$log") + 1))
sudo dtrace -q -n "spike$pid:::work-entry { @n = count(); } tick-1s { exit(0); }" \
  >"$work/dtrace2.out" 2>&1
expect "a second dtrace attaches and detaches normally" bash -c "
  tail -n +$from '$log' | grep -q 'entry-enabled=true' &&
  tail -n +$from '$log' | grep -q 'entry-enabled=false' &&
  grep -qE '^ +[0-9]+\$' '$work/dtrace2.out'"
snapshot >"$work/after2"
expect "after it: every site holds its bytes from before" cmp -s "$work/before" "$work/after2"

echo "== killed dtrace -p"
from=$(($(wc -l <"$log") + 1))
sudo dtrace -q -p "$pid" -n 'spike$target:::work-entry { @n = count(); }' >"$work/dtrace3.out" 2>&1 &
disown
wait_enabled true "$from"
expect "dtrace -p attached: the program reports the probe on" test $? -eq 0
tpid=$(dtrace_pid)
sudo kill -9 "$tpid"
sleep 2
if kill -0 "$pid" 2>/dev/null; then
  wait_enabled false "$from"
  expect "dtrace -p killed: the program keeps running and reports the probe off" test $? -eq 0
  snapshot >"$work/after3"
  expect "dtrace -p killed: every site holds its bytes from before" cmp -s "$work/before" "$work/after3"
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
else
  wait "$pid" 2>/dev/null
  echo "  FAIL  dtrace -p killed: the program died with it (status $?)"
  failed=1
fi

echo "== cdylib loaded with dlopen"
crate="$work/dl"
mkdir -p "$crate/plug/src" "$crate/host/src"
cat >"$crate/plug/Cargo.toml" <<EOF
[package]
name = "plug"
version = "0.0.0"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[dependencies]
anyprobe = { path = "$PWD/crates/anyprobe", default-features = false }
EOF
cat >"$crate/plug/src/lib.rs" <<'EOF'
mod shared {
    anyprobe::probes! {
        provider = "shared";
        pub fn tick(from: &str, n: u64);
    }
}
mod plugonly {
    anyprobe::probes! {
        provider = "plugonly";
        pub fn tick(from: &str, n: u64);
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn plug_tick(n: u64) {
    if shared::tick::enabled() {
        shared::tick::fire("plug", n);
    }
    if plugonly::tick::enabled() {
        plugonly::tick::fire("plugonly", n);
    }
}
EOF
cat >"$crate/host/Cargo.toml" <<EOF
[package]
name = "host"
version = "0.0.0"
edition = "2024"
[dependencies]
anyprobe = { path = "$PWD/crates/anyprobe", default-features = false }
EOF
cat >"$crate/host/src/main.rs" <<'EOF'
use std::ffi::{c_char, c_int, c_void, CString};
use std::io::Write;
unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}
anyprobe::probes! {
    provider = "shared";
    fn tick(from: &str, n: u64);
}
fn main() {
    let path = CString::new(std::env::args().nth(1).unwrap()).unwrap();
    let name = CString::new("plug_tick").unwrap();
    // SAFETY: a test program; the library and symbol exist.
    let f: extern "C" fn(u64) = unsafe {
        let h = dlopen(path.as_ptr(), 2);
        assert!(!h.is_null(), "dlopen failed");
        std::mem::transmute(dlsym(h, name.as_ptr()))
    };
    println!("loaded pid={}", std::process::id());
    let _ = std::io::stdout().flush();
    for n in 0..4000 {
        if tick::enabled() {
            tick::fire("host", n);
        }
        f(n);
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
EOF
export CARGO_TARGET_DIR="$PWD/target/check-gaps"
if ! (cd "$crate/plug" && cargo build -q --release) || ! (cd "$crate/host" && cargo build -q --release); then
  echo "  FAIL  build the scratch crates"
  exit 1
fi
lib="$CARGO_TARGET_DIR/release/libplug.dylib"
host="$CARGO_TARGET_DIR/release/host"
expect "libplug.dylib has DOF for plugonly:::tick and shared:::tick" bash -c "
  '$PWD/target/debug/examples/inspect_dof' '$lib' >'$work/plug.dof' &&
  grep -q '^plugonly:.*:tick ' '$work/plug.dof' && grep -q '^shared:.*:tick ' '$work/plug.dof'"

hlog="$work/host.log"
"$host" "$lib" >"$hlog" 2>&1 &
hpid=$!
pids+=("$hpid")
for _ in $(seq 50); do grep -q loaded "$hlog" && break; sleep 0.1; done

sudo dtrace -l -n "plugonly$hpid:::" >"$work/dl-list" 2>&1
expect "dtrace -l lists plugonly$hpid:::tick in module libplug.dylib" \
  bash -c "awk '\$2 == \"plugonly$hpid\" && \$3 == \"libplug.dylib\" && \$NF == \"tick\"' '$work/dl-list' | grep -q ."
sudo dtrace -l -n "shared$hpid:::" >"$work/dl-list-shared" 2>&1
echo "  info  dtrace -l lists shared$hpid:::tick in: $(awk -v p="shared$hpid" '$2 == p { print $3 }' "$work/dl-list-shared" | sort -u | tr '\n' ' ')"

# Runs dtrace -p on the host for 2 s with CLAUSES; prints the output.
trace() {
  sudo dtrace -q -p "$hpid" -n "$1 tick-2s { exit(0); }" 2>&1
}
# Firings counted under KEY in dtrace's aggregation output FILE.
fired() { awk -v k="$1" '$1 == k { print $NF }' "$2"; }

trace 'plugonly$target:::tick { @[copyinstr(arg0, arg1)] = count(); }' >"$work/dl1.out"
n=$(fired plugonly "$work/dl1.out")
expect "dtrace -p reads a probe only the library defines (${n:-0} firings)" test "${n:-0}" -gt 20

trace 'shared$target:::tick { @[probemod, copyinstr(arg0, arg1)] = count(); }' >"$work/dl2.out"
for k in "$(basename "$host") host" "libplug.dylib plug"; do
  n=$(awk -v m="${k% *}" -v f="${k#* }" '$1 == m && $2 == f { print $3 }' "$work/dl2.out")
  expect "shared\$target:::tick fires in ${k% *} (${n:-0} firings)" test "${n:-0}" -gt 20
done
kill "$hpid" 2>/dev/null
wait "$hpid" 2>/dev/null

if [ "$failed" -ne 0 ]; then
  for f in work.log dtrace.out dtrace2.out dtrace3.out dof before during after after2 \
    after3 host.log plug.dof dl-list dl-list-shared dl1.out dl2.out; do
    [ -f "$work/$f" ] && { echo "  --- $f (last 30 lines)"; tail -30 "$work/$f"; }
  done
  echo FAIL
else
  echo PASS
fi
exit "$failed"
