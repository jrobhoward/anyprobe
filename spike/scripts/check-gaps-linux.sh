#!/usr/bin/env bash
# Checks two runtime claims in docs/GAPS.md on Linux:
#
#   - A killed tracer leaves nothing behind ("What a tracer writes into the
#     process"). For bpftrace and for `perf record`, killed with SIGKILL
#     while attached to the `work` example: while attached, the semaphore is
#     raised (by one per uprobe, so by the number of sites) and every
#     work__entry site starts with a breakpoint (0xcc); after the kill, the
#     semaphore is back to 0, every site is a `nop` (0x90) again, and the
#     program itself reports the probe off.
#   - A probe in a `cdylib` loaded with `dlopen` can be traced ("Shared
#     libraries"). A scratch library defines `plugonly:tick` and
#     `shared:tick`; the executable that loads it defines `shared:tick` too.
#     The library's `.so` carries the SDT notes and a `.probes` semaphore,
#     and bpftrace attached to the running process reads `plugonly:tick`.
#     Then, as information, each file's `shared:tick` alone and both in one
#     bpftrace program: one probe name in two files of one process.
#
# Usage: spike/scripts/check-gaps-linux.sh
# Needs cargo, bpftrace, perf, and sudo for the tracers and for reading the
# process's memory through /proc/PID/mem. Run as yourself, not under sudo.
# Prints ok/FAIL per check and exits non-zero if any check failed.

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
command -v bpftrace >/dev/null || { echo "bpftrace not found" >&2; exit 2; }
command -v perf >/dev/null || { echo "perf not found" >&2; exit 2; }
sudo -v || exit 2

work=$(mktemp -d)
pids=()
cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done
  sudo perf probe -q -d 'sdt_spike:*' 2>/dev/null
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

# Load address of BIN in process PID: the start of its first mapping, which
# for a position-independent executable or library is where vaddr 0 lands.
load_base() { awk -v f="$(realpath "$2")" '$6 == f { split($1, a, "-"); print "0x" a[1]; exit }' "/proc/$1/maps"; }

# N bytes of process PID's memory at ADDR, as unsigned decimal values.
peek() {
  sudo dd if="/proc/$1/mem" bs=1 skip="$(($2))" count="$3" status=none 2>/dev/null |
    od -An -tu1 | tr -s ' \n' '  ' | sed 's/^ //; s/ $//'
}

# The 16-bit semaphore at ADDR in PID.
semaphore() {
  sudo dd if="/proc/$1/mem" bs=1 skip="$(($2))" count=2 status=none 2>/dev/null | od -An -tu2 | tr -d ' '
}

# "location semaphore" for every SDT note of PROBE in BIN.
notes() {
  readelf -n --wide "$1" | awk -v p="$2" '
    /Name:/ { name = $2 }
    /Location:/ && name == p { gsub(",", ""); print $2, $6 }'
}

# First byte of every site of work__entry in PID, space-separated.
site_bytes() {
  local out=""
  while read -r loc _; do out="$out $(peek "$1" $((base + loc)) 1)"; done <<<"$entry_notes"
  echo "${out# }"
}

# Polls until the program's log shows `entry-enabled=WANT` after line FROM.
wait_enabled() {
  for _ in $(seq 50); do
    tail -n +"$2" "$log" | grep -q "entry-enabled=$1" && return 0
    sleep 0.1
  done
  return 1
}

cargo build -q --release -p anyprobe --example work || exit 2
bin="$PWD/target/release/examples/work"
entry_notes=$(notes "$bin" work__entry)
sema_vaddr=$(head -1 <<<"$entry_notes" | awk '{ print $2 }')
nsites=$(wc -l <<<"$entry_notes")

echo "== killed tracers (anyprobe example work, $nsites work__entry sites)"
log="$work/work.log"
"$bin" 0 20 >"$log" 2>&1 &
pid=$!
pids+=("$pid")
sleep 1
base=$(load_base "$pid" "$bin")
sema=$((base + sema_vaddr))
nops=$(printf '144 %.0s' $(seq "$nsites")); nops=${nops% }
traps=$(printf '204 %.0s' $(seq "$nsites")); traps=${traps% }
expect "before: semaphore 0, every site a nop" \
  test "$(semaphore "$pid" "$sema"):$(site_bytes "$pid")" = "0:$nops"

for tracer in bpftrace perf; do
  from=$(($(wc -l <"$log") + 1))
  if [ "$tracer" = bpftrace ]; then
    sudo bpftrace -p "$pid" -e "usdt:$bin:spike:work__entry { @n = count(); }" \
      >"$work/bpftrace.out" 2>&1 &
  else
    sudo perf probe -q -x "$bin" -a 'sdt_spike:work__entry' >"$work/perf-probe.out" 2>&1
    sudo perf record -q -e 'sdt_spike:work__entry' -p "$pid" -o "$work/perf.data" \
      >"$work/perf.out" 2>&1 &
  fi
  wait_enabled true "$from"
  expect "$tracer attached: the program reports the probe on" test $? -eq 0
  now="$(semaphore "$pid" "$sema"):$(site_bytes "$pid")"
  expect "$tracer attached: semaphore raised, every site a breakpoint ($now)" \
    test "${now%%:*}" -gt 0 -a "${now#*:}" = "$traps"
  # `sudo` does not pass SIGKILL on, so kill the tracer itself.
  tpid=$(pgrep -n -x "$tracer")
  sudo kill -9 "$tpid"
  from=$(($(wc -l <"$log") + 1))
  wait_enabled false "$from"
  expect "$tracer killed: the program reports the probe off" test $? -eq 0
  expect "$tracer killed: semaphore 0, every site a nop again" \
    test "$(semaphore "$pid" "$sema"):$(site_bytes "$pid")" = "0:$nops"
  [ "$tracer" = perf ] && sudo perf probe -q -d 'sdt_spike:*' 2>/dev/null
done
kill "$pid" 2>/dev/null

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
lib="$CARGO_TARGET_DIR/release/libplug.so"
host="$CARGO_TARGET_DIR/release/host"
expect "libplug.so has SDT notes for plugonly:tick and shared:tick" \
  test "$(notes "$lib" tick | wc -l)" -ge 2
expect "libplug.so has a .probes section for its semaphores" \
  bash -c "readelf -SW '$lib' | grep -q ' .probes '"

hlog="$work/host.log"
"$host" "$lib" >"$hlog" 2>&1 &
hpid=$!
pids+=("$hpid")
for _ in $(seq 50); do grep -q loaded "$hlog" && break; sleep 0.1; done

# Runs bpftrace -p on the host for 2 s with CLAUSES; prints the output.
trace() {
  sudo timeout 20 bpftrace -p "$hpid" -e "$1 interval:s:2 { exit(); }" 2>&1
}
# Firings counted under KEY in bpftrace output FILE.
fired() { awk -v k="@[$1]:" '$1 == k { print $2 }' "$2"; }

trace "usdt:$lib:plugonly:tick { @[str(arg0, arg1)] = count(); }" >"$work/dl1.bt"
n=$(fired plugonly "$work/dl1.bt")
expect "bpftrace reads a probe only the library defines (${n:-0} firings)" test "${n:-0}" -gt 20

for what in "$host:host" "$lib:plug"; do
  f="${what%:*}"
  k="${what##*:}"
  trace "usdt:$f:shared:tick { @[str(arg0, arg1)] = count(); }" >"$work/dl-$k.bt"
  n=$(fired "$k" "$work/dl-$k.bt")
  echo "  info  shared:tick in $(basename "$f") alone: ${n:-0} firings"
  [ -n "$n" ] || sed 's/^/        /' "$work/dl-$k.bt"
done
trace "usdt:$host:shared:tick { @[str(arg0, arg1)] = count(); }
       usdt:$lib:shared:tick { @[str(arg0, arg1)] = count(); }" >"$work/dl-both.bt"
echo "  info  shared:tick in both files, one program: host $(fired host "$work/dl-both.bt" || true), plug $(fired plug "$work/dl-both.bt" || true)"
grep -q ERROR "$work/dl-both.bt" && sed 's/^/        /' "$work/dl-both.bt"
kill "$hpid" 2>/dev/null

if [ "$failed" -ne 0 ]; then
  for f in work.log bpftrace.out perf-probe.out perf.out dl1.bt host.log; do
    [ -f "$work/$f" ] && { echo "  --- $f (last 20 lines)"; tail -20 "$work/$f"; }
  done
  echo FAIL
else
  echo PASS
fi
exit "$failed"
